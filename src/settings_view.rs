use crate::{settings::Settings, tunnel::Profile};
use gpui_kit::{
    component::{
        ActiveTheme, WindowExt,
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        dialog::{DialogAction, DialogFooter},
        input::{Input, InputState},
    },
    *,
};
use std::path::{Path, PathBuf};

fn private_key_directory(home: &Path) -> PathBuf {
    let ssh = home.join(".ssh");
    if ssh.is_dir() {
        ssh
    } else {
        home.to_path_buf()
    }
}

const IDS: [&str; 8] = [
    "tunnel-name",
    "ssh-server",
    "ssh-user",
    "ssh-port",
    "identity-file",
    "local-host",
    "local-port",
    "remote-port",
];

pub(crate) fn open(path: PathBuf, profile: Option<Profile>, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let editing = profile.is_some();
    let form = cx.new(|cx| ProfileForm::new(path, profile.unwrap_or_default(), window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let confirm = form.clone();
        dialog
            .title(if editing {
                "Edit tunnel"
            } else {
                "New reverse tunnel"
            })
            .width(px(620.))
            .child(form.clone())
            .on_ok(move |_, _, cx| confirm.update(cx, |form, cx| form.save(cx)))
    });
}

pub(crate) fn guide(window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    window.open_dialog(cx, |dialog, _, cx| {
        dialog.title("A small guide to going backward").width(px(650.))
            .child(div().flex().flex_col().gap_4().text_sm()
                .child(guide_step("01   Prepare your SSH server", "Use an SSH server you control. Enable AllowTcpForwarding yes (or remote) in sshd_config, then reload sshd. Public access also requires GatewayPorts clientspecified and an open firewall port."))
                .child(guide_step("02   Trust the server and load your key", "In a terminal, connect with ssh -F /dev/null -p 22 user@server (use -F NUL on Windows). Verify its fingerprint with your server provider before accepting. Add an encrypted key with ssh-add, or choose a private key in the tunnel form. Password prompts are not supported."))
                .child(guide_step("03   Map a remote port to your local service", "For a local app on 127.0.0.1:3000, choose remote port 8080. Connect the tunnel, then reach the app at 127.0.0.1:8080 on your SSH server. Enable public access only when you intend to expose the service."))
                .child(div().p_3().rounded_md().bg(cx.theme().muted).child("OpenSSH 8.0+ must be on PATH. macOS includes it; install openssh-client on Linux or the OpenSSH Client optional feature on Windows. SSH config aliases and jump hosts are not used."))
                .child(div().text_color(cx.theme().muted_foreground).child("Closing the app stops every tunnel. A connected tunnel confirms the remote listener; your local service must also be running.")))
            .child(DialogFooter::new().child(Button::new("close-guide").primary().label("Got it").on_click(|_, window, cx| window.close_dialog(cx))))
    });
}

fn guide_step(title: &'static str, description: &'static str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .child(description)
}

struct ProfileForm {
    path: PathBuf,
    profile: Profile,
    inputs: Vec<Entity<InputState>>,
    error: Option<String>,
}

impl ProfileForm {
    fn new(path: PathBuf, profile: Profile, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let values = [
            profile.name.clone(),
            profile.server.clone(),
            profile.user.clone(),
            profile.ssh_port.to_string(),
            profile.identity.clone(),
            profile.local_host.clone(),
            profile.local_port.to_string(),
            profile.remote_port.to_string(),
        ];
        let placeholders = [
            "e.g. Development app",
            "e.g. 203.0.113.10",
            "e.g. deploy",
            "22",
            "Default keys / SSH agent",
            "127.0.0.1",
            "3000",
            "8080",
        ];
        let inputs = values
            .into_iter()
            .zip(placeholders)
            .map(|(value, placeholder)| {
                cx.new(|cx| {
                    let mut state = InputState::new(window, cx).placeholder(placeholder);
                    state.set_value(value, window, cx);
                    state
                })
            })
            .collect();
        Self {
            path,
            profile,
            inputs,
            error: None,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let result = (|| -> Result<Settings, String> {
            let values: Vec<String> = self
                .inputs
                .iter()
                .map(|input| input.read(cx).value().trim().to_string())
                .collect();
            let port = |i: usize| {
                values[i]
                    .parse::<u16>()
                    .ok()
                    .filter(|p| *p > 0)
                    .ok_or_else(|| "Ports must be whole numbers from 1 to 65535.".to_string())
            };
            let mut profile = Profile {
                id: self.profile.id,
                name: values[0].clone(),
                server: values[1].clone(),
                user: values[2].clone(),
                ssh_port: port(3)?,
                identity: values[4].clone(),
                local_host: values[5].clone(),
                local_port: port(6)?,
                remote_port: port(7)?,
                public: self.profile.public,
            };
            profile.validate()?;
            let mut settings = cx.global::<Settings>().clone();
            if profile.id == 0 {
                if settings.profiles.len() >= 100 {
                    return Err(
                        "You can save up to 100 tunnels. Remove one before adding another.".into(),
                    );
                }
                profile.id = settings
                    .profiles
                    .iter()
                    .map(|p| p.id)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or("Tunnel ID limit reached")?;
                settings.profiles.push(profile);
            } else {
                let saved = settings
                    .profiles
                    .iter_mut()
                    .find(|p| p.id == profile.id)
                    .ok_or("This tunnel no longer exists")?;
                *saved = profile;
            }
            settings.save(&self.path).map_err(|e| {
                format!("Couldn’t save the tunnel: {e}. Your previous profiles are preserved.")
            })?;
            Ok(settings)
        })();
        match result {
            Ok(settings) => {
                cx.set_global(settings);
                cx.refresh_windows();
                true
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                false
            }
        }
    }

    fn field(&self, i: usize, label: &'static str) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .text_sm()
            .child(label)
            .child(Input::new(&self.inputs[i]).id(IDS[i]))
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_title("Choose SSH private key")
            .set_parent(window);
        if let Some(home) = dirs::home_dir() {
            dialog = dialog.set_directory(private_key_directory(&home));
        }
        let picker = dialog.pick_file();
        cx.spawn_in(window, async move |this, cx| {
            let result = picker.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(file) = result {
                    this.inputs[4].update(cx, |input, cx| {
                        input.set_value(file.path().to_string_lossy().to_string(), window, cx)
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ProfileForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().gap_3()
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Forward a port on your SSH server to a service on this device."))
            .child(self.field(0, "Tunnel name"))
            .child(div().flex().gap_3().child(self.field(1, "SSH server")).child(self.field(2, "Username")).child(div().w(px(90.)).child(self.field(3, "SSH port"))))
            .child(div().flex().items_end().gap_2().child(self.field(4, "Private key (optional)")).child(Button::new("browse-key").outline().label("Browse…").on_click(cx.listener(|this, _, window, cx| this.browse(window, cx)))))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Leave blank to use default keys or your SSH agent. Load encrypted keys with ssh-add."))
            .child(div().border_t_1().border_color(cx.theme().border).pt_3().flex().gap_3().child(self.field(5, "Local host")).child(self.field(6, "Local port")).child(self.field(7, "Remote port")))
            .child(Checkbox::new("public-access").label("Allow public access on the SSH server").checked(self.profile.public).on_change(cx.listener(|this, value, _, cx| { this.profile.public = *value; cx.notify(); })))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(if self.profile.public { "Exposes your local service to the server’s network. Requires GatewayPorts clientspecified and a firewall rule. Protect the service with authentication." } else { "Private by default: the remote port is accessible only from the SSH server, subject to the server’s GatewayPorts policy." }))
            .child(div().id("profile-error").test_support().role(Role::Status).aria_label(self.error.clone().unwrap_or_default()).text_sm().text_color(cx.theme().danger).children(self.error.clone()))
            .child(DialogFooter::new()
                .child(Button::new("cancel-profile").label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                .child(DialogAction::new().child(Button::new("save-profile").primary().label("Save tunnel"))))
    }
}

#[cfg(test)]
mod tests {
    use super::{Settings, open, private_key_directory};
    use gpui_kit::{
        App, AppContext, TestAppContext, Window, component::Root, div, px, size,
        test::TestWindowExt,
    };

    fn fill(window: &mut Window, id: &'static str, value: &str, cx: &mut App) {
        window.click(id, cx);
        window.press("secondary-a", cx);
        window.input(value, cx);
    }

    #[test]
    fn private_key_picker_prefers_ssh_directory_and_falls_back_to_home() {
        let home = std::env::temp_dir().join(format!("portway-key-picker-{}", std::process::id()));
        std::fs::create_dir(&home).unwrap();
        assert_eq!(private_key_directory(&home), home);
        let ssh = home.join(".ssh");
        std::fs::write(&ssh, "not a directory").unwrap();
        assert_eq!(private_key_directory(&home), home);
        std::fs::remove_file(&ssh).unwrap();
        std::fs::create_dir(&ssh).unwrap();
        assert_eq!(private_key_directory(&home), ssh);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[gpui_kit::test]
    async fn profile_form_validates_saves_and_preserves_on_cancel(cx: &mut TestAppContext) {
        let directory = std::env::temp_dir().join(format!("portway-ui-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("settings.json");
        Settings::default().save(&path).unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            cx.set_global(Settings::default());
        });
        let handle = cx.open_window(size(px(1000.), px(900.)), |window, cx| {
            let content = cx.new(|_| Empty);
            Root::new(content, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            open(path.clone(), None, window, cx);
            window.render_frame(cx);
            window.click("save-profile", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find("profile-error")
                    .label()
                    .unwrap()
                    .contains("name")
            );
            fill(window, "tunnel-name", "Development", cx);
            fill(window, "ssh-server", "example.com", cx);
            fill(window, "ssh-user", "deploy", cx);
            fill(window, "remote-port", "70000", cx);
            window.click("save-profile", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find("profile-error")
                    .label()
                    .unwrap()
                    .contains("65535")
            );
            fill(window, "remote-port", "8080", cx);
            window.click("save-profile", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let saved = Settings::read(&path).unwrap();
        assert_eq!(saved.profiles.len(), 1);
        assert_eq!(saved.profiles[0].name, "Development");
        cx.update_window(handle.into(), |_, window, cx| {
            open(path.clone(), Some(saved.profiles[0].clone()), window, cx);
            window.render_frame(cx);
            fill(window, "tunnel-name", "Discarded", cx);
            window.click("cancel-profile", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(Settings::read(&path).unwrap(), saved);
        std::fs::remove_dir_all(directory).unwrap();
    }

    struct Empty;
    impl gpui_kit::Render for Empty {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            div()
        }
    }
}
