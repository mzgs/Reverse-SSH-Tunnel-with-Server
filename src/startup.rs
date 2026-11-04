use std::{io, path::Path};

pub(crate) fn register(settings_path: &Path, enabled: bool) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    apply_once(
        &settings_path.with_file_name("startup.json"),
        enabled,
        || {
            #[cfg(target_os = "macos")]
            if !executable
                .ancestors()
                .any(|path| path.extension().is_some_and(|extension| extension == "app"))
            {
                return Err(io::Error::other(
                    "Open the built macOS .app to change launch at login",
                ));
            }
            let path = executable
                .to_str()
                .ok_or_else(|| io::Error::other("Application path is not valid UTF-8"))?;
            #[cfg(target_os = "macos")]
            {
                let mut builder = auto_launch::AutoLaunchBuilder::new();
                builder.set_app_name("SSH Tunnel Plus");
                builder
                    .set_app_path(path)
                    .set_macos_launch_mode(auto_launch::MacOSLaunchMode::SMAppService);
                builder
                    .build()
                    .and_then(|startup| {
                        if enabled {
                            if !startup.is_enabled()? {
                                startup.enable()?;
                            }
                            Ok(())
                        } else {
                            startup.disable()
                        }
                    })
                    .map_err(io::Error::other)
            }
            #[cfg(target_os = "windows")]
            {
                // Keep the registry value name stable across app renames.
                let key = windows_registry::CURRENT_USER
                    .create(r"Software\Microsoft\Windows\CurrentVersion\Run")
                    .map_err(io::Error::from)?;
                if enabled {
                    key.set_string("Portway", format!("\"{path}\""))
                        .map_err(io::Error::from)?;
                    // Re-enable the entry if Task Manager previously disabled it.
                    match windows_registry::CURRENT_USER.options().write().open(
                        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
                    ).map_err(io::Error::from) {
                        Ok(key) => key.set_bytes("Portway", windows_registry::Type::Bytes,
                            &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).map_err(io::Error::from),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                        Err(error) => Err(error),
                    }
                } else {
                    match key.remove_value("Portway").map_err(io::Error::from) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                        result => result,
                    }
                }
            }
            #[cfg(target_os = "linux")]
            {
                let directory = dirs::config_dir()
                    .ok_or_else(|| io::Error::other("Could not find the configuration directory"))?
                    .join("autostart");
                // Keep the filename stable so upgrades reuse the existing startup entry.
                let entry = directory.join("Portway.desktop");
                if enabled {
                    std::fs::create_dir_all(directory)?;
                    std::fs::write(
                        entry,
                        format!(
                            "[Desktop Entry]\nType=Application\nName=SSH Tunnel Plus\nExec={}\nTerminal=false\n",
                            desktop_exec_path(path)
                        ),
                    )
                } else {
                    match std::fs::remove_file(entry) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                        result => result,
                    }
                }
            }
        },
    )
}

fn apply_once(
    marker: &Path,
    enabled: bool,
    apply: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    match crate::settings::read_json::<bool>(marker) {
        // Leave later changes in the OS startup settings alone.
        Ok(previous) if previous == enabled => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    apply()?;
    crate::settings::write_json(marker, &enabled)
}

#[cfg(any(target_os = "linux", test))]
fn desktop_exec_path(path: &str) -> String {
    // Desktop entries unescape string values before parsing quoted Exec arguments.
    let escaped = path
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('`', "\\\\`")
        .replace('$', "\\\\$")
        .replace('%', "%%")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_runs_once_retries_failures_and_preserves_invalid_state() -> io::Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "ssh-tunnel-plus-startup-test-{}",
            std::process::id()
        ));
        std::fs::create_dir(&directory)?;
        let marker = directory.join("startup.json");
        let result = (|| {
            assert!(apply_once(&marker, true, || Err(io::Error::other("denied"))).is_err());
            assert!(!marker.exists(), "failed registration must be retried");
            apply_once(&marker, true, || Ok(()))?;
            apply_once(&marker, true, || {
                panic!("must not override OS startup preferences")
            })?;
            assert!(crate::settings::read_json::<bool>(&marker)?);
            assert!(apply_once(&marker, false, || Err(io::Error::other("denied"))).is_err());
            assert!(
                crate::settings::read_json::<bool>(&marker)?,
                "failed changes must be retried"
            );
            apply_once(&marker, false, || Ok(()))?;
            assert!(!crate::settings::read_json::<bool>(&marker)?);
            apply_once(&marker, false, || panic!("must keep startup disabled"))?;
            std::fs::write(&marker, "invalid JSON")?;
            assert!(
                apply_once(&marker, true, || panic!("invalid state must be preserved")).is_err()
            );
            assert_eq!(std::fs::read_to_string(&marker)?, "invalid JSON");
            Ok(())
        })();
        std::fs::remove_dir_all(directory)?;
        result
    }

    #[test]
    fn desktop_paths_keep_spaces_and_escape_exec_metacharacters() {
        assert_eq!(
            desktop_exec_path("/home/me/My Apps/SSH Tunnel Plus"),
            "\"/home/me/My Apps/SSH Tunnel Plus\""
        );
        assert_eq!(
            desktop_exec_path("/tmp/\\\"`$%\n\r\t"),
            concat!(
                "\"/tmp/",
                "\\\\\\\\",
                "\\\\\"",
                "\\\\`",
                "\\\\$",
                "%%",
                "\\n\\r\\t\""
            )
        );
    }
}
