use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_SETTINGS_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) profiles: Vec<crate::tunnel::Profile>,
    pub(crate) start_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            profiles: Vec::new(),
            start_at_login: true,
        }
    }
}

impl Settings {
    pub(crate) fn add_profile(
        &mut self,
        mut profile: crate::tunnel::Profile,
    ) -> Result<crate::tunnel::Profile, String> {
        if self.profiles.len() >= 100 {
            return Err("You can save up to 100 tunnels. Remove one before adding another.".into());
        }
        profile.validate()?;
        profile.id = self
            .profiles
            .iter()
            .map(|p| p.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Tunnel ID limit reached")?;
        self.profiles.push(profile.clone());
        Ok(profile)
    }

    pub(crate) fn path() -> io::Result<PathBuf> {
        let directory = dirs::data_local_dir()
            .ok_or_else(|| io::Error::other("Could not find the application data directory"))?;
        Ok(directory.join(env!("CARGO_PKG_NAME")).join("settings.json"))
    }

    pub(crate) fn load(path: &Path) -> io::Result<Self> {
        match Self::read(path) {
            Ok(settings) => Ok(settings),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let settings = Self::default();
                settings.save(path)?;
                Ok(settings)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn read(path: &Path) -> io::Result<Self> {
        let settings: Self = read_json(path)?;
        if settings.profiles.len() > 100 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "At most 100 tunnel profiles are supported",
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for profile in &settings.profiles {
            profile
                .validate()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if profile.id == 0 || !ids.insert(profile.id) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid or duplicate tunnel profile ID",
                ));
            }
        }
        Ok(settings)
    }

    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        write_json(path, self)
    }
}

pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_SETTINGS_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Settings JSON exceeds 1 MiB",
        ));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Settings JSON exceeds 1 MiB",
        ));
    }
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid settings path"))?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_profiles_validates_limits_without_changing_existing_profiles() {
        let profile = crate::tunnel::Profile {
            name: "Development".into(),
            server: "example.com".into(),
            user: "deploy".into(),
            ..crate::tunnel::Profile::default()
        };
        let mut settings = Settings::default();
        assert!(
            settings
                .add_profile(crate::tunnel::Profile::default())
                .is_err()
        );
        assert!(settings.profiles.is_empty());
        assert_eq!(settings.add_profile(profile.clone()).unwrap().id, 1);
        settings.profiles[0].id = u64::MAX;
        let previous = settings.clone();
        assert!(settings.add_profile(profile.clone()).is_err());
        assert_eq!(settings, previous);
        settings.profiles = (1..=100)
            .map(|id| crate::tunnel::Profile {
                id,
                ..profile.clone()
            })
            .collect();
        let previous = settings.clone();
        assert!(settings.add_profile(profile).is_err());
        assert_eq!(settings, previous);
    }

    #[test]
    fn settings_persist_defaults_changes_and_preserve_invalid_files() -> io::Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "{}-settings-test-{}",
            env!("CARGO_PKG_NAME"),
            std::process::id()
        ));
        fs::create_dir(&directory)?;
        let path = directory.join("app/settings.json");
        let result = (|| -> io::Result<()> {
            let mut settings = Settings::load(&path)?;
            assert_eq!(settings, Settings::default());
            assert!(settings.start_at_login);
            assert!(path.exists());
            let missing = directory.join("missing.json");
            assert!(Settings::read(&missing).is_err());
            assert!(!missing.exists());
            settings.profiles.push(crate::tunnel::Profile {
                id: 1,
                name: "Development".into(),
                server: "example.com".into(),
                user: "deploy".into(),
                local_command: "cd ~/my-app && npm start".into(),
                autostart: true,
                ..crate::tunnel::Profile::default()
            });
            settings.start_at_login = false;
            settings.save(&path)?;
            assert_eq!(Settings::load(&path)?, settings);
            let mut old_settings = serde_json::to_value(&settings)?;
            old_settings
                .as_object_mut()
                .unwrap()
                .remove("start_at_login");
            old_settings["profiles"][0]
                .as_object_mut()
                .unwrap()
                .remove("local_command");
            old_settings["profiles"][0]
                .as_object_mut()
                .unwrap()
                .remove("autostart");
            old_settings["profiles"][0]
                .as_object_mut()
                .unwrap()
                .remove("local_autostart");
            fs::write(&path, serde_json::to_vec(&old_settings)?)?;
            assert!(Settings::load(&path)?.profiles[0].local_command.is_empty());
            assert!(!Settings::load(&path)?.profiles[0].autostart);
            assert!(Settings::load(&path)?.profiles[0].local_autostart);
            assert!(
                Settings::load(&path)?.start_at_login,
                "existing users default to enabled"
            );
            // Old scaffold preferences still load without discarding the file on error.
            fs::write(&path, r#"{"display_name":"Lin"}"#)?;
            assert_eq!(Settings::load(&path)?, Settings::default());
            let invalid = r#"{"profiles":[{"id":1}]}"#;
            for invalid in ["invalid JSON", invalid] {
                fs::write(&path, invalid)?;
                assert!(Settings::load(&path).is_err());
                assert_eq!(fs::read_to_string(&path)?, invalid);
            }
            let oversized = directory.join("oversized.json");
            fs::write(&oversized, vec![b' '; MAX_SETTINGS_BYTES + 1])?;
            assert_eq!(
                Settings::read(&oversized).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert!(write_json(&path, &"x".repeat(MAX_SETTINGS_BYTES)).is_err());
            assert_eq!(fs::read_to_string(&path)?, invalid);
            let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
            fs::write(&temporary, "existing temporary file")?;
            assert!(settings.save(&path).is_err());
            assert_eq!(fs::read_to_string(&path)?, invalid);
            assert_eq!(fs::read_to_string(&temporary)?, "existing temporary file");
            assert!(
                Settings::path()?
                    .ends_with(Path::new(env!("CARGO_PKG_NAME")).join("settings.json"))
            );
            Ok(())
        })();
        fs::remove_dir_all(directory)?;
        result
    }
}
