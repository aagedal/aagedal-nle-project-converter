use crate::actions::{CloseWindow, DismissOverlay, MinimizeWindow, ZoomWindow};
use crate::assets;
use crate::theme::{self, PillTone};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants as _},
        scroll::ScrollableElement as _,
        ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex,
    },
    *,
};
use nle_effects::InstalledPluginRegistry;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Tracks the single-instance About window handle across the application.
#[derive(Default)]
struct AboutWindowSlot(Option<AnyWindowHandle>);

impl Global for AboutWindowSlot {}

pub struct AboutWindow {
    focus_handle: FocusHandle,
    copied_info: bool,
    cross_plugin_count: usize,
}

impl AboutWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let cross_plugin_count = InstalledPluginRegistry::system().grouped_plugins(2).len();
        Self {
            focus_handle,
            copied_info: false,
            cross_plugin_count,
        }
    }

    fn system_info_summary(&self) -> String {
        format!(
            "Aagedal NLE Project Converter v{APP_VERSION}\nOS: {} ({})\nStack: Rust + GPUI\nLicense: GPL-3.0-or-later\nDetected Cross-NLE Plugins: {}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.cross_plugin_count,
        )
    }

    fn on_copy_system_info(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.system_info_summary()));
        self.copied_info = true;
        cx.notify();
    }
}

/// Open the About window, or bring the existing About window to the foreground if already open.
pub fn open_about_window(cx: &mut App) {
    if !cx.has_global::<AboutWindowSlot>() {
        cx.set_global(AboutWindowSlot::default());
    }

    if let Some(existing) = cx.global::<AboutWindowSlot>().0 {
        let activated = cx
            .update_window(existing, |_, window, _| {
                window.activate_window();
            })
            .is_ok();
        if activated {
            return;
        }
    }

    let bounds = Bounds::centered(None, size(px(500.), px(590.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(460.), px(520.))),
        is_resizable: false,
        titlebar: Some(TitlebarOptions {
            title: Some("About Aagedal NLE Project Converter".into()),
            ..Default::default()
        }),
        ..Default::default()
    };

    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| AboutWindow::new(window, cx))
    }) {
        cx.global_mut::<AboutWindowSlot>().0 = Some(handle);
    }
}

impl Render for AboutWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = cx.theme().mono_font_family.clone();

        v_flex()
            .id("about-window-root")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(theme::BG))
            .text_color(rgb(theme::TEXT))
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_action(|_: &DismissOverlay, window, _| window.remove_window())
            .on_action(|_: &MinimizeWindow, window, _| window.minimize_window())
            .on_action(|_: &ZoomWindow, window, _| window.zoom_window())
            .child(
                div()
                    .id("about-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .px(px(28.))
                            .pt(px(26.))
                            .pb(px(22.))
                            .items_center()
                            .gap(px(16.))
                            .child(
                                div()
                                    .p(px(4.))
                                    .rounded(px(22.))
                                    .bg(theme::color_alpha(theme::ACCENT, 0.14))
                                    .border_1()
                                    .border_color(theme::color_alpha(theme::ACCENT, 0.35))
                                    .child(
                                        img(assets::APP_ICON_ASSET)
                                            .w(px(84.))
                                            .h(px(84.))
                                            .rounded(px(18.)),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(20.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(rgb(theme::TEXT))
                                            .text_center()
                                            .child("Aagedal NLE Project Converter"),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .justify_center()
                                            .flex_wrap()
                                            .gap(px(6.))
                                            .child(theme::pill(
                                                format!("Version {APP_VERSION}"),
                                                PillTone::Purple,
                                            ))
                                            .child(theme::pill("Rust + GPUI", PillTone::Cyan))
                                            .child(theme::pill(
                                                "GPL-3.0-or-later",
                                                PillTone::Green,
                                            )),
                                    )
                                    .child(
                                        div()
                                            .max_w(px(400.))
                                            .pt(px(4.))
                                            .text_xs()
                                            .text_color(rgb(theme::MUTED))
                                            .text_center()
                                            .child(
                                                "Direct native project conversion between Adobe Premiere Pro, DaVinci Resolve, Final Cut Pro, OpenTimelineIO, and FCP7 XML — 100% local, zero cloud dependencies.",
                                            ),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .bg(rgb(theme::SURFACE))
                                    .border_1()
                                    .border_color(rgb(theme::BORDER))
                                    .rounded(px(12.))
                                    .px(px(16.))
                                    .py(px(14.))
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(theme::TEXT))
                                            .child("Supported Formats"),
                                    )
                                    .child(
                                        h_flex()
                                            .flex_wrap()
                                            .gap(px(6.))
                                            .child(theme::pill(
                                                "Premiere Pro (.prproj)",
                                                PillTone::Purple,
                                            ))
                                            .child(theme::pill(
                                                "DaVinci Resolve (.drp / .drt)",
                                                PillTone::Cyan,
                                            ))
                                            .child(theme::pill(
                                                "Final Cut Pro (.fcpbundle / .fcpxml)",
                                                PillTone::Green,
                                            ))
                                            .child(theme::pill(
                                                "OpenTimelineIO (.otio)",
                                                PillTone::Neutral,
                                            ))
                                            .child(theme::pill(
                                                "FCP7 XML (.xml)",
                                                PillTone::Neutral,
                                            )),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .bg(rgb(theme::SURFACE))
                                    .border_1()
                                    .border_color(rgb(theme::BORDER))
                                    .rounded(px(12.))
                                    .px(px(16.))
                                    .py(px(14.))
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(theme::TEXT))
                                            .child("License & Third-Party Attributions"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(theme::MUTED))
                                            .child(
                                                "Licensed under the GNU General Public License v3.0 (GPL-3.0-or-later).",
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(theme::MUTED))
                                            .child(
                                                "• Ersiter/pr2resolve (MIT) — DaVinci Resolve 21 native template & schema research",
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(theme::MUTED))
                                            .child(
                                                "• ChatOctopus/timeline (MIT) — Exact rational-time canonical IR inspiration",
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .justify_between()
                                    .px(px(14.))
                                    .py(px(10.))
                                    .rounded(px(10.))
                                    .bg(rgb(theme::SURFACE_2))
                                    .border_1()
                                    .border_color(rgb(theme::BORDER))
                                    .child(
                                        div()
                                            .font_family(mono.clone())
                                            .text_xs()
                                            .text_color(rgb(theme::MUTED))
                                            .child(format!(
                                                "{} / {} · {} cross-NLE plugins",
                                                std::env::consts::OS,
                                                std::env::consts::ARCH,
                                                self.cross_plugin_count,
                                            )),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .px(px(22.))
                    .py(px(14.))
                    .bg(rgb(theme::SURFACE))
                    .border_t_1()
                    .border_color(rgb(theme::BORDER))
                    .child(
                        Button::new("about-copy-info")
                            .small()
                            .icon(if self.copied_info {
                                IconName::Check
                            } else {
                                IconName::Copy
                            })
                            .label(if self.copied_info {
                                "Copied System Info"
                            } else {
                                "Copy System Info"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.on_copy_system_info(cx);
                            })),
                    )
                    .child(
                        Button::new("about-close")
                            .primary()
                            .small()
                            .label("Close")
                            .on_click(|_, window, _| {
                                window.remove_window();
                            }),
                    ),
            )
    }
}
