#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod settings;
mod settings_view;
mod startup;
mod tunnel;
mod tunnel_view;
mod window_state;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use settings::Settings;
use window_state::WindowState;

gpui_kit::assets::icon_assets!(
    SettingsIcons,
    [
        Waypoints,
        ShieldCheck,
        Plus,
        Terminal,
        Server,
        Laptop,
        ArrowRight,
        Pencil,
        Play,
        Square,
        Copy,
        Check,
        Trash,
        Info,
        Settings
    ]
);

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if path == "app-icon.png" {
            return Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../resources/app-icon.png"
            ))));
        }
        if let Some(bytes) = SettingsIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(SettingsIcons.list(path)?);
        if "app-icon.png".starts_with(path) {
            paths.push("app-icon.png".into());
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

impl Global for Settings {}

#[cfg(test)]
mod tests {
    use super::{AssetSource, Assets};

    #[test]
    fn app_icon_is_embedded_and_discoverable() -> gpui_kit::Result<()> {
        let bytes = Assets.load("app-icon.png")?.expect("app icon is embedded");
        assert!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "app icon must be a PNG"
        );
        assert!(Assets.list("")?.contains(&"app-icon.png".into()));
        assert!(
            Assets
                .list("icons/")?
                .iter()
                .all(|path| path.starts_with("icons/"))
        );
        Ok(())
    }
}

actions!(base_app, [Quit, OpenSettings]);

struct AppView {
    window_path: Option<std::path::PathBuf>,
    window_state: WindowState,
    tunnels: Entity<tunnel_view::TunnelView>,
}

impl AppView {
    fn new(
        window_path: Option<std::path::PathBuf>,
        restored: Option<WindowState>,
        settings_path: Option<std::path::PathBuf>,
        startup_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut window_state = restored.unwrap_or_else(|| WindowState::capture(window));
        window_state.update(window);
        cx.observe_window_bounds(window, |view, window, _| view.window_state.update(window))
            .detach();
        cx.on_app_quit(|view, cx| {
            view.save_window_state();
            view.tunnels.update(cx, |tunnels, _| tunnels.stop_all());
            async {}
        })
        .detach();
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = weak.update(cx, |view, cx| {
                view.window_state.update(window);
                view.save_window_state();
                view.tunnels.update(cx, |tunnels, _| tunnels.stop_all());
            });
            true
        });
        let tunnels = cx.new(|cx| tunnel_view::TunnelView::new(settings_path, startup_error, cx));
        Self {
            window_path,
            window_state,
            tunnels,
        }
    }

    fn save_window_state(&self) {
        if let Some(path) = &self.window_path
            && let Err(error) = settings::write_json(path, &self.window_state)
        {
            eprintln!(
                "Couldn’t save window geometry to {}: {error}",
                path.display()
            );
        }
    }
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.tunnels.clone())
    }
}

fn main() {
    let loaded = Settings::path().and_then(|path| {
        Settings::load(&path)
            .map(|settings| (path.clone(), settings))
            .map_err(|error| {
                std::io::Error::new(error.kind(), format!("{}: {error}", path.display()))
            })
    });
    let (settings_path, settings, mut startup_error) = match loaded {
        Ok((path, settings)) => (Some(path), settings, None),
        Err(error) => (
            None,
            Settings::default(),
            Some(format!(
                "Couldn’t load settings: {error}. Existing files are preserved and saving is disabled. Fix the file or folder permissions, then restart the app."
            )),
        ),
    };
    if let Some(path) = &settings_path
        && let Err(error) = startup::register(path, settings.start_at_login)
    {
        startup_error = Some(format!(
            "Couldn’t update launch at login: {error}. Check Settings to retry."
        ));
    }
    let (window_path, restored) = match WindowState::load() {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!(
                "Couldn’t load window geometry: {error}. Existing file is preserved; geometry saving is disabled."
            );
            (None, None)
        }
    };
    let window_handle = std::rc::Rc::new(std::cell::Cell::new(None::<AnyWindowHandle>));
    let application = gpui_kit::application().with_assets(Assets);
    application.on_reopen({
        let window_handle = window_handle.clone();
        move |cx| {
            if let Some(handle) = window_handle.get() {
                cx.defer(move |cx| {
                    if let Err(error) = cx.update_window(handle, |_, window, cx| {
                        window.activate_window();
                        cx.activate(true);
                    }) {
                        eprintln!("Couldn’t reactivate the app window: {error}");
                    }
                });
            }
        }
    });
    application.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(settings);
        let preferences_path = settings_path.clone();
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let options = WindowOptions {
            window_bounds: Some(WindowState::window_bounds(restored.as_ref(), cx)),
            window_min_size: Some(size(px(1100.), px(760.))),
            titlebar: Some(TitlebarOptions {
                title: Some("SSH Tunnel Plus — Reverse SSH tunnels".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
            Theme::change(ThemeMode::Dark, Some(window), cx);
            Theme::update(cx, |theme| {
                theme.colors.primary = rgb(0x127657).into();
                theme.colors.button_primary = rgb(0x127657).into();
                theme.colors.button_primary_hover = rgb(0x0c6147).into();
                theme.colors.button_primary_active = rgb(0x094d38).into();
                theme.colors.button_primary_foreground = rgb(0xffffff).into();
                theme.colors.ring = rgb(0x8edbb2).into();
            });
            cx.new(|cx| {
                AppView::new(
                    window_path,
                    restored,
                    settings_path,
                    startup_error,
                    window,
                    cx,
                )
            })
        })
        .expect("Failed to open app window");
        window_handle.set(Some(handle));
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            #[cfg(not(target_os = "macos"))]
            KeyBinding::new("ctrl-q", Quit, None),
            #[cfg(not(target_os = "macos"))]
            KeyBinding::new("ctrl-,", OpenSettings, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(move |_: &OpenSettings, cx| {
            let path = preferences_path.clone();
            cx.defer(move |cx| {
                let _ = cx.update_window(handle, |_, window, cx| {
                    settings_view::preferences(path, window, cx)
                });
            });
        });
        cx.set_menus(vec![Menu {
            name: "SSH Tunnel Plus".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit", Quit),
            ],
        }]);
        cx.activate(true);
    });
}
