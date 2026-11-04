use crate::{
    settings::Settings,
    settings_view,
    tunnel::{Profile, Session, Status},
};
use gpui_kit::{
    assets::IconName,
    base::SelectableText,
    component::{
        ActiveTheme, Disableable, Icon, Selectable, Sizable, WindowExt,
        button::{Button, ButtonCustomVariant, ButtonVariants},
        clipboard::Clipboard,
    },
    prelude::FluentBuilder,
    *,
};
use std::{collections::HashMap, path::PathBuf, time::Duration};

const INK: u32 = 0xf0f0f2;
const MUTED: u32 = 0xa7a7b1;
const LINE: u32 = 0x35353d;
const ACCENT: u32 = 0xa4b5ff;

pub(crate) struct TunnelView {
    path: Option<PathBuf>,
    selected: Option<u64>,
    sessions: HashMap<u64, Session>,
    error: Option<String>,
    log_scroll: ScrollHandle,
}

impl TunnelView {
    pub fn new(path: Option<PathBuf>, error: Option<String>, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let mut changed = this.sessions.values().any(Session::active);
                        let selected = this
                            .selected
                            .or_else(|| cx.global::<Settings>().profiles.last().map(|p| p.id));
                        // The stderr reader can deliver final diagnostics after the process exits.
                        for (id, session) in &mut this.sessions {
                            let previous = session.logs.back().cloned();
                            session.poll();
                            if session.logs.back() != previous.as_ref() {
                                changed = true;
                                if Some(*id) == selected {
                                    this.log_scroll.scroll_to_bottom();
                                }
                            }
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            path,
            selected: None,
            sessions: HashMap::new(),
            error,
            log_scroll: ScrollHandle::new(),
        }
    }

    pub fn stop_all(&mut self) {
        for session in self.sessions.values_mut() {
            session.stop();
        }
    }

    fn edit(&self, profile: Option<Profile>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = &self.path {
            settings_view::open(path.clone(), profile, window, cx);
        }
    }

    fn toggle(&mut self, profile: &Profile, cx: &mut Context<Self>) {
        self.error = None;
        if let Some(session) = self.sessions.get_mut(&profile.id).filter(|s| s.active()) {
            session.stop();
        } else {
            match Session::start(profile) {
                Ok(session) => {
                    self.sessions.insert(profile.id, session);
                }
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }

    fn delete(&self, profile: Profile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let profile = profile.clone();
            let path = path.clone();
            let weak = weak.clone();
            dialog
                .title("Delete tunnel?")
                .description(format!(
                    "Remove “{}” from your saved tunnels? Your SSH keys and server are kept.",
                    profile.name
                ))
                .on_ok(move |_, _, cx| {
                    let mut settings = cx.global::<Settings>().clone();
                    settings.profiles.retain(|p| p.id != profile.id);
                    match settings.save(&path) {
                        Ok(()) => {
                            cx.set_global(settings);
                            let _ = weak.update(cx, |this, cx| {
                                this.sessions.remove(&profile.id);
                                this.selected = None;
                                cx.notify();
                            });
                            cx.refresh_windows();
                            true
                        }
                        Err(error) => {
                            let _ = weak.update(cx, |this, cx| {
                                this.error = Some(format!("Couldn’t delete tunnel: {error}"));
                                cx.notify();
                            });
                            false
                        }
                    }
                })
        });
    }

    fn status(&self, id: u64) -> Status {
        self.sessions
            .get(&id)
            .map(|s| s.status)
            .unwrap_or(Status::Stopped)
    }

    fn sidebar(
        &self,
        profiles: &[Profile],
        selected: Option<u64>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("connections-sidebar")
            .test_support()
            .w_72()
            .min_h_0()
            .h_full()
            .flex_shrink_0()
            .bg(rgb(0x101013))
            .text_color(rgb(0xf0f0f2))
            .flex()
            .flex_col()
            .p_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_3()
                    .pb_8()
                    .child(
                        div()
                            .size(px(34.))
                            .rounded_lg()
                            .bg(rgb(0xdce4ff))
                            .text_color(rgb(0x263b80))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::Waypoints).size_5()),
                    )
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .child("Portway"),
                    ),
            )
            .child(
                Button::new("new-tunnel")
                    .w_full()
                    .mb_5()
                    .primary()
                    .icon(IconName::Plus)
                    .label("New tunnel")
                    .disabled(self.path.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.edit(None, window, cx))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x93939f))
                    .mb_3()
                    .child("WORKSPACE"),
            )
            .child(
                Button::new("tunnels-nav")
                    .w_full()
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(rgb(0x2d3040).into())
                            .foreground(rgb(0xe0e6ff).into())
                            .hover(rgb(0x383d50).into()),
                    )
                    .icon(IconName::Waypoints)
                    .label("Reverse tunnels")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected = None;
                        cx.notify();
                    })),
            )
            .child(
                div().mt_2().child(
                    Button::new("connection-guide")
                        .w_full()
                        .ghost()
                        .text_color(rgb(0xb1b1bc))
                        .icon(IconName::BookOpen)
                        .label("Connection guide")
                        .on_click(|_, window, cx| settings_view::guide(window, cx)),
                ),
            )
            .child(self.profile_list(profiles, selected, cx))
            .child(
                div()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0x35353d))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .child(
                                Icon::new(IconName::ShieldCheck)
                                    .size_4()
                                    .text_color(rgb(0xdce4ff)),
                            )
                            .child("Your keys stay yours"),
                    )
                    .child(div().text_xs().text_color(rgb(0xa7a7b1)).child(
                        "Powered by OpenSSH. No passwords or private keys are stored in Portway.",
                    )),
            )
            .child(
                div()
                    .mt_5()
                    .text_xs()
                    .text_color(rgb(0x93939f))
                    .child("LOCAL WORKSPACE   /   v0.1.0"),
            )
    }

    fn empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        card().flex_1().flex().flex_col().items_center().justify_center().gap_5().p_8()
            .child(div().size(px(70.)).rounded_2xl().bg(rgb(0x292e43)).flex().items_center().justify_center().text_color(rgb(ACCENT)).child(Icon::new(IconName::Waypoints).size(px(32.))))
            .child(div().text_2xl().font_weight(FontWeight::SEMIBOLD).child("Your local world, within reach."))
            .child(div().max_w(px(470.)).text_center().text_color(rgb(MUTED)).child("Bring a local app, development server, or device to your SSH server through an encrypted reverse tunnel."))
            .child(Button::new("create-first").primary().icon(IconName::Plus).label("Create your first tunnel").disabled(self.path.is_none()).on_click(cx.listener(|this, _, window, cx| this.edit(None, window, cx))))
            .child(div().mt_6().flex().gap_8()
                .child(step("01", "Add your SSH server"))
                .child(step("02", "Choose your ports"))
                .child(step("03", "Connect securely")))
            .child(div().text_xs().text_color(rgb(MUTED)).child("macOS  ·  Windows  ·  Linux"))
    }

    fn profile_list(
        &self,
        profiles: &[Profile],
        selected: Option<u64>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .my_5()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .mb_3()
                    .child("Connections"),
            )
            .child(
                div()
                    .id("profile-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(profiles.is_empty(), |list| {
                        list.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No saved connections"),
                        )
                    })
                    .children(profiles.iter().map(|p| {
                        let id = p.id;
                        Button::new(SharedString::from(format!("profile-{id}")))
                            .w_full()
                            .h_16()
                            .ghost()
                            .selected(Some(id) == selected)
                            .accessibility_label(p.name.clone())
                            .tooltip(format!(
                                "{} · {}@{}:{}",
                                p.name, p.user, p.server, p.ssh_port
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .text_left()
                                    .child(div().truncate().child(p.name.clone()))
                                    .child(
                                        div()
                                            .flex()
                                            .justify_between()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!("{} → {}", p.remote_port, p.local_port))
                                            .child(status_badge(self.status(id), cx)),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected = Some(id);
                                this.log_scroll.scroll_to_bottom();
                                this.error = None;
                                cx.notify();
                            }))
                    })),
            )
    }

    fn details(&self, profile: &Profile, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status(profile.id);
        let session = self.sessions.get(&profile.id);
        let log_text = session
            .map(|s| s.logs.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|| "Ready when you are. Connect to see OpenSSH output here.".into());
        let active = session.is_some_and(Session::active);
        let edit = profile.clone();
        let connect = profile.clone();
        let delete = profile.clone();
        let uptime = session
            .and_then(|s| s.connected_at)
            .filter(|_| status == Status::Connected)
            .map(|t| {
                let seconds = t.elapsed().as_secs();
                format!("{}m {:02}s", seconds / 60, seconds % 60)
            })
            .unwrap_or_else(|| "—".into());
        div()
            .id("connection-details")
            .test_support()
            .aria_label(profile.name.clone())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                card()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(profile.name.clone()),
                                    )
                                    .child(
                                        div().text_xs().text_color(rgb(MUTED)).truncate().child(
                                            format!(
                                                "{}@{}:{} · {}",
                                                profile.user,
                                                profile.server,
                                                profile.ssh_port,
                                                if profile.identity.is_empty() {
                                                    "SSH agent / default keys"
                                                } else {
                                                    "Private key"
                                                }
                                            ),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new("edit-tunnel")
                                            .ghost()
                                            .icon(IconName::Pencil)
                                            .label("Edit")
                                            .tooltip("Edit tunnel")
                                            .disabled(active || self.path.is_none())
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.edit(Some(edit.clone()), window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("connect-tunnel")
                                            .primary()
                                            .icon(if active {
                                                IconName::Square
                                            } else {
                                                IconName::Play
                                            })
                                            .label(if active { "Disconnect" } else { "Connect" })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.toggle(&connect, cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(LINE))
                            .bg(rgb(0x17171c))
                            .p_3()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(endpoint(
                                IconName::Server,
                                "REMOTE SERVER",
                                format!("{}:{}", profile.remote_bind(), profile.remote_port),
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(div().min_w_0().truncate().child(profile.server.clone()))
                                    .child(
                                        Clipboard::new(("copy-server", profile.id))
                                            .value(profile.server.clone())
                                            .tooltip("Copy server address")
                                            .accessibility_label("Copy server address"),
                                    ),
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(34.))
                                    .text_center()
                                    .text_color(rgb(ACCENT))
                                    .child(Icon::new(IconName::ArrowRight).size_5()),
                            )
                            .child(endpoint(
                                IconName::Laptop,
                                "LOCAL SERVICE",
                                format!("{}:{}", profile.local_host, profile.local_port),
                                "This device",
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .border_t_1()
                            .border_color(rgb(LINE))
                            .pt_3()
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .child(status_badge(status, cx))
                                    .child(format!("Uptime {uptime}")),
                            )
                            .child(div().text_color(rgb(MUTED)).child(if profile.public {
                                "Public access requested · 0.0.0.0"
                            } else {
                                "Private access requested · 127.0.0.1"
                            })),
                    ),
            )
            .child(
                card()
                    .flex_1()
                    .min_h(px(145.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .px_4()
                            .py_2()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(rgb(LINE))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(Icon::new(IconName::Terminal).size_4())
                                    .child("Connection log"),
                            )
                            .child(
                                Button::new("copy-log")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Copy)
                                    .label("Copy")
                                    .disabled(session.is_none())
                                    .on_click({
                                        let text = log_text.clone();
                                        move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                text.clone(),
                                            ))
                                        }
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id("connection-log")
                            .track_scroll(&self.log_scroll)
                            .test_support()
                            .focusable()
                            .role(Role::Log)
                            .aria_label(format!("SSH connection log. {log_text}"))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_4()
                            .bg(rgb(0x121216))
                            .text_color(rgb(0xc9c9d2))
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .when(session.is_none(), |element| {
                                        element.text_color(rgb(0x9d9daa))
                                    })
                                    .child(SelectableText::new(
                                        ("connection-log-text", profile.id),
                                        log_text,
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Logs stay in memory · Last 200 lines")
                    .child(
                        Button::new("delete-tunnel")
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .label("Delete tunnel")
                            .disabled(active || self.path.is_none())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete(delete.clone(), window, cx)
                            })),
                    ),
            )
    }
}

impl Render for TunnelView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let profiles = cx.global::<Settings>().profiles.clone();
        let selected = self
            .selected
            .and_then(|id| profiles.iter().find(|p| p.id == id))
            .or_else(|| profiles.last());
        div().size_full().flex().bg(rgb(0x131316)).text_color(rgb(INK))
            .child(self.sidebar(&profiles, selected.map(|profile| profile.id), cx))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col()
                .child(div().h(px(55.)).flex_shrink_0().px_7().border_b_1().border_color(rgb(LINE)).flex().items_center().justify_between().bg(rgb(0x1c1c21))
                    .child(div().text_sm().text_color(rgb(MUTED)).child("Workspace   /   Reverse tunnels"))
                    .child(div().flex().items_center().gap_2().text_xs().text_color(rgb(MUTED)).child(div().size(px(6.)).rounded_full().bg(rgb(ACCENT))).child("ON THIS DEVICE")))
                .child(div().id("workspace").flex_1().min_h_0().overflow_y_scroll().p_7().flex().flex_col().gap_5()
                    .children(self.error.clone().map(|error| div().id("tunnel-error").test_support().role(Role::Alert).aria_label(error.clone()).rounded_md().border_1().border_color(rgb(0x76533e)).bg(rgb(0x33251e)).p_3().text_sm().text_color(rgb(0xf1b99b)).child(error)))
                    .child(div().flex_1().min_h(px(410.)).flex().gap_4().map(|element| {
                        if let Some(profile) = selected {
                            element.child(self.details(profile, cx))
                        } else { element.child(self.empty(cx)) }
                    }))
                    .child(div().text_xs().text_color(rgb(MUTED)).flex().items_center().gap_2().child(Icon::new(IconName::Info).size_4()).child("Tunnels run while Portway is open. Closing the app disconnects every tunnel."))))
    }
}

fn card() -> Div {
    div()
        .bg(rgb(0x1c1c21))
        .rounded_xl()
        .border_1()
        .border_color(rgb(LINE))
}

fn step(n: &'static str, text: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(rgb(MUTED))
        .child(
            div()
                .text_color(rgb(ACCENT))
                .font_weight(FontWeight::SEMIBOLD)
                .child(n),
        )
        .child(text)
}
fn endpoint(
    icon: IconName,
    label: &'static str,
    address: String,
    caption: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(Icon::new(icon).size_4())
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(address),
        )
        .child(div().text_xs().text_color(rgb(MUTED)).child(caption))
}
fn status_badge(status: Status, cx: &App) -> impl IntoElement {
    let color = match status {
        Status::Connected => cx.theme().success,
        Status::Connecting => rgb(0xf1cc83).into(),
        Status::Stopped => rgb(MUTED).into(),
        Status::Failed => rgb(0xffa99a).into(),
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(color)
        .child(div().size(px(6.)).rounded_full().bg(color))
        .child(status.label())
}

#[cfg(test)]
mod tests {
    use super::TunnelView;
    use crate::{settings::Settings, tunnel::Profile};
    use gpui_kit::{
        AppContext, ClipboardItem, TestAppContext, base::TextSelection, component::Root, point, px,
        size, test::TestWindowExt,
    };

    #[gpui_kit::test]
    async fn sidebar_selection_keeps_connection_log_visible_and_selectable(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            cx.set_global(Settings {
                profiles: vec![
                    Profile {
                        id: 1,
                        name: "Local development".into(),
                        server: "example.com".into(),
                        user: "deploy".into(),
                        ..Profile::default()
                    },
                    Profile {
                        id: 2,
                        name: "Staging service".into(),
                        server: "staging.example.com".into(),
                        user: "deploy".into(),
                        ..Profile::default()
                    },
                ],
            });
        });
        let handle = cx.open_window(size(px(1200.), px(800.)), |window, cx| {
            let view = cx.new(|cx| TunnelView::new(None, None, cx));
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let sidebar = window.find("connections-sidebar").bounds();
            let row = window.find("profile-1").bounds();
            assert!(
                row.left() >= sidebar.left() && row.right() <= sidebar.right(),
                "Connection row must be inside the sidebar: {row:?}, {sidebar:?}"
            );
            assert_eq!(
                window.find("connection-details").label(),
                Some("Staging service")
            );
            window.click("profile-1", cx);
            assert_eq!(
                window.find("connection-details").label(),
                Some("Local development")
            );
            window.click("profile-2", cx);
            assert_eq!(
                window.find("connection-details").label(),
                Some("Staging service")
            );
            window.focus_next(cx);
            window.render_frame(cx);
            for _ in 0..8 {
                if window.find("profile-1").focused() == Some(true) {
                    break;
                }
                window.press("tab", cx);
            }
            assert_eq!(window.find("profile-1").focused(), Some(true));
            window.press("enter", cx);
            assert_eq!(
                window.find("connection-details").label(),
                Some("Local development")
            );
            assert!(
                window.find("connection-details").bounds().left() >= sidebar.right(),
                "Details must appear beside the sidebar"
            );
            let log = window.find("connection-log").bounds();
            assert!(
                log.size.height >= px(90.),
                "Connection log collapsed: {log:?}"
            );
            assert!(
                log.bottom() < px(800.),
                "Connection log extends past the window: {log:?}"
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.blur(cx);
            cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
            window.render_frame(cx);
            let log = window.find("connection-log").bounds();
            window.drag(
                log.origin + point(px(16.), px(22.)),
                point(log.right() - px(16.), log.top() + px(22.)),
                cx,
            );
            let expected = "Ready when you are. Connect to see OpenSSH output here.";
            assert_eq!(TextSelection::selected_text(window, cx), expected);
            window.press("secondary-c", cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(expected.into())
            );
            for (id, server) in [(1_u64, "example.com"), (2, "staging.example.com")] {
                window.click(format!("profile-{id}"), cx);
                let copy_id = gpui_kit::ElementId::from(("copy-server", id));
                assert_eq!(
                    window.find(copy_id.clone()).label(),
                    Some("Copy server address")
                );
                window.click(copy_id, cx);
                assert_eq!(
                    cx.read_from_clipboard().and_then(|item| item.text()),
                    Some(server.into())
                );
            }
        })
        .unwrap();
    }
}
