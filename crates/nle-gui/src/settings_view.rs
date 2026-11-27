use crate::about::open_about_window;
use crate::actions::SHORTCUT_CATALOG;
use crate::assets;
use crate::backend::SUPPORTED_TARGETS;
use crate::settings::{AppSettings, AppSettingsGlobal, DefaultEffectPolicy};
use crate::theme::{self, PillTone};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState},
        kbd::Kbd,
        select::{Select, SelectEvent, SelectItem, SelectState},
        switch::Switch,
        ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _, h_flex, v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};
use nle_core::NleFormat;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSection {
    General,
    ConversionDefaults,
    MediaRelinking,
    Shortcuts,
}

impl SettingsSection {
    const ALL: &'static [(Self, &'static str, &'static str, IconName)] = &[
        (
            Self::General,
            "General",
            "Target format, workflow behavior, and UI preferences",
            IconName::Settings2,
        ),
        (
            Self::ConversionDefaults,
            "Conversion Defaults",
            "Default checkboxes applied when opening projects",
            IconName::LayoutDashboard,
        ),
        (
            Self::MediaRelinking,
            "Media Relinking",
            "Default drive or folder prefix rewrites across workstations",
            IconName::HardDrive,
        ),
        (
            Self::Shortcuts,
            "Keyboard Shortcuts",
            "All keyboard bindings and application menu commands",
            IconName::SquareTerminal,
        ),
    ];

    fn title(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(s, _, _, _)| *s == self)
            .map(|(_, t, _, _)| *t)
            .unwrap_or("Settings")
    }

    fn subtitle(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(s, _, _, _)| *s == self)
            .map(|(_, _, d, _)| *d)
            .unwrap_or("")
    }
}

#[derive(Clone, Debug)]
pub enum SettingsViewEvent {
    Changed,
    ApplyDefaultsToProject,
    BackToConverter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TargetOption {
    format: NleFormat,
    label: &'static str,
}

impl SelectItem for TargetOption {
    type Value = NleFormat;

    fn title(&self) -> SharedString {
        SharedString::from(self.label)
    }

    fn value(&self) -> &Self::Value {
        &self.format
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PolicyOption {
    policy: DefaultEffectPolicy,
    label: &'static str,
}

impl SelectItem for PolicyOption {
    type Value = DefaultEffectPolicy;

    fn title(&self) -> SharedString {
        SharedString::from(self.label)
    }

    fn value(&self) -> &Self::Value {
        &self.policy
    }
}

fn target_options() -> Vec<TargetOption> {
    SUPPORTED_TARGETS
        .iter()
        .map(|&(format, label)| TargetOption { format, label })
        .collect()
}

fn policy_options() -> Vec<PolicyOption> {
    DefaultEffectPolicy::ALL
        .iter()
        .map(|&(policy, label)| PolicyOption { policy, label })
        .collect()
}

pub struct SettingsView {
    active_section: SettingsSection,
    has_loaded_project: bool,
    save_feedback: Option<String>,
    save_error: Option<String>,

    default_target_select: Entity<SelectState<Vec<TargetOption>>>,
    default_policy_select: Entity<SelectState<Vec<PolicyOption>>>,
    relink_from_input: Entity<InputState>,
    relink_to_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsViewEvent> for SettingsView {}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = cx.global::<AppSettingsGlobal>().0.clone();

        let target_items = target_options();
        let target_row = target_items
            .iter()
            .position(|o| o.format == initial.default_target_format)
            .unwrap_or(0);
        let default_target_select = cx.new(|cx| {
            SelectState::new(
                target_items,
                Some(IndexPath::default().row(target_row)),
                window,
                cx,
            )
        });

        let policy_items = policy_options();
        let policy_row = policy_items
            .iter()
            .position(|o| o.policy == initial.default_effect_policy)
            .unwrap_or(0);
        let default_policy_select = cx.new(|cx| {
            SelectState::new(
                policy_items,
                Some(IndexPath::default().row(policy_row)),
                window,
                cx,
            )
        });

        let relink_from_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("From prefix (e.g. D:\\Media or /Media)")
                .default_value(initial.default_relink_from.clone())
        });
        let relink_to_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("To prefix (e.g. /Volumes/RAID/Media)")
                .default_value(initial.default_relink_to.clone())
        });

        let target_sub = cx.subscribe_in(
            &default_target_select,
            window,
            |this, _, event: &SelectEvent<Vec<TargetOption>>, _, cx| {
                if let SelectEvent::Confirm(Some(fmt)) = event {
                    this.mutate_settings(cx, |s| s.default_target_format = *fmt);
                }
            },
        );

        let policy_sub = cx.subscribe_in(
            &default_policy_select,
            window,
            |this, _, event: &SelectEvent<Vec<PolicyOption>>, _, cx| {
                if let SelectEvent::Confirm(Some(policy)) = event {
                    this.mutate_settings(cx, |s| s.default_effect_policy = *policy);
                }
            },
        );

        let from_sub = cx.subscribe_in(
            &relink_from_input,
            window,
            |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change | InputEvent::Blur) {
                    let val = input.read(cx).value().to_string();
                    this.mutate_settings(cx, |s| s.default_relink_from = val);
                }
            },
        );

        let to_sub = cx.subscribe_in(
            &relink_to_input,
            window,
            |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change | InputEvent::Blur) {
                    let val = input.read(cx).value().to_string();
                    this.mutate_settings(cx, |s| s.default_relink_to = val);
                }
            },
        );

        Self {
            active_section: SettingsSection::General,
            has_loaded_project: false,
            save_feedback: None,
            save_error: None,
            default_target_select,
            default_policy_select,
            relink_from_input,
            relink_to_input,
            _subscriptions: vec![target_sub, policy_sub, from_sub, to_sub],
        }
    }

    pub fn set_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.active_section = section;
        self.save_feedback = None;
        cx.notify();
    }

    pub fn set_has_loaded_project(&mut self, has_loaded: bool, cx: &mut Context<Self>) {
        if self.has_loaded_project != has_loaded {
            self.has_loaded_project = has_loaded;
            cx.notify();
        }
    }

    fn mutate_settings(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut AppSettings)) {
        let updated = {
            let global = cx.global_mut::<AppSettingsGlobal>();
            f(&mut global.0);
            global.0.clone()
        };
        match updated.save() {
            Ok(_) => {
                self.save_error = None;
            }
            Err(err) => {
                self.save_error = Some(format!("Failed to save settings: {err}"));
            }
        }
        cx.emit(SettingsViewEvent::Changed);
        cx.notify();
    }

    fn reset_all_to_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = AppSettings::default();
        self.default_target_select.update(cx, |state, cx| {
            state.set_selected_value(&defaults.default_target_format, window, cx);
        });
        self.default_policy_select.update(cx, |state, cx| {
            state.set_selected_value(&defaults.default_effect_policy, window, cx);
        });
        self.relink_from_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.relink_to_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });

        self.mutate_settings(cx, |s| *s = defaults);
        self.save_feedback = Some("Restored all settings to their defaults.".to_string());
        cx.notify();
    }

    fn render_setting_row(
        &self,
        title: &'static str,
        description: &'static str,
        control: impl IntoElement,
    ) -> Div {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .py(px(12.))
            .border_b_1()
            .border_color(theme::color_alpha(0xffffff, 0.05))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::TEXT))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::MUTED))
                            .child(description),
                    ),
            )
            .child(div().flex_shrink_0().child(control))
    }

    fn render_card(&self, title: &'static str, subtitle: Option<&'static str>) -> Div {
        v_flex()
            .w_full()
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(12.))
            .px(px(20.))
            .py(px(16.))
            .gap(px(4.))
            .child(
                v_flex()
                    .pb(px(6.))
                    .gap(px(2.))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT))
                            .child(title),
                    )
                    .when_some(subtitle, |col, sub| {
                        col.child(div().text_xs().text_color(rgb(theme::MUTED)).child(sub))
                    }),
            )
    }

    fn render_general_section(&self, settings: &AppSettings, cx: &mut Context<Self>) -> Div {
        let mono = cx.theme().mono_font_family.clone();
        let config_path_display = AppSettings::settings_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unavailable".to_string());
        let config_path_reveal = AppSettings::settings_path();

        v_flex()
            .w_full()
            .gap(px(16.))
            .child(
                self.render_card(
                    "Workflow & Output",
                    Some("Control default conversion targets and post-export behavior."),
                )
                .child(self.render_setting_row(
                    "Default Target Format",
                    "Selected automatically on launch and when loading new projects (if different from source).",
                    div()
                        .w(px(290.))
                        .child(Select::new(&self.default_target_select)),
                ))
                .child(self.render_setting_row(
                    "Default Effect Mapping Policy",
                    "Initial action assigned to detected effects whenever a new project is opened.",
                    div()
                        .w(px(290.))
                        .child(Select::new(&self.default_policy_select)),
                ))
                .child(self.render_setting_row(
                    "Auto-Reveal Output After Conversion",
                    "Automatically reveal the exported project file in Finder / File Explorer when conversion finishes.",
                    Switch::new("set-auto-reveal")
                        .checked(settings.auto_reveal_output)
                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                            let val = *checked;
                            this.mutate_settings(cx, |s| s.auto_reveal_output = val);
                        })),
                ))
                .child(self.render_setting_row(
                    "Remember Last Used Folder",
                    "Open file and save dialogs in your most recently used project directory.",
                    Switch::new("set-remember-dir")
                        .checked(settings.remember_last_directory)
                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                            let val = *checked;
                            this.mutate_settings(cx, |s| s.remember_last_directory = val);
                        })),
                )),
            )
            .child(
                self.render_card(
                    "Interface",
                    Some("Visual cues and keyboard discoverability."),
                )
                .child(self.render_setting_row(
                    "Show Keyboard Shortcut Hints",
                    "Display compact shortcut keycaps next to primary toolbar and workspace actions.",
                    Switch::new("set-shortcut-hints")
                        .checked(settings.show_shortcut_hints)
                        .on_change(cx.listener(|this, checked: &bool, _, cx| {
                            let val = *checked;
                            this.mutate_settings(cx, |s| s.show_shortcut_hints = val);
                        })),
                )),
            )
            .child(
                self.render_card(
                    "Configuration File",
                    Some("Settings are saved automatically in JSON format on your machine."),
                )
                .child(
                    h_flex()
                        .w_full()
                        .pt(px(8.))
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .px(px(12.))
                                .py(px(8.))
                                .rounded(px(8.))
                                .bg(rgb(theme::BG))
                                .border_1()
                                .border_color(rgb(theme::BORDER))
                                .font_family(mono)
                                .text_xs()
                                .text_color(rgb(theme::MUTED))
                                .child(config_path_display),
                        )
                        .when_some(config_path_reveal, |row, path| {
                            row.child(
                                Button::new("settings-reveal-config")
                                    .small()
                                    .icon(IconName::FolderOpen)
                                    .label("Reveal")
                                    .on_click(move |_, _, cx| {
                                        if path.exists() {
                                            cx.reveal_path(&path);
                                        } else if let Some(parent) = path.parent() {
                                            let _ = std::fs::create_dir_all(parent);
                                            cx.reveal_path(parent);
                                        }
                                    }),
                            )
                        }),
                ),
            )
    }

    fn render_conversion_defaults_section(
        &self,
        settings: &AppSettings,
        cx: &mut Context<Self>,
    ) -> Div {
        v_flex().w_full().gap(px(16.)).child(
            self.render_card(
                "Default Conversion Options",
                Some(
                    "These options pre-configure the Conversion Options panel at startup. You can still toggle them per project.",
                ),
            )
            .child(self.render_setting_row(
                "Preserve Nested Project Bins",
                "Recreate Media Pool folders and bin hierarchy in the target project.",
                Switch::new("set-def-bins")
                    .checked(settings.default_include_bins)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let val = *checked;
                        this.mutate_settings(cx, |s| s.default_include_bins = val);
                    })),
            ))
            .child(self.render_setting_row(
                "Preserve Unplaced Project Bin Media",
                "Keep media pool assets that are not currently placed on a sequence timeline.",
                Switch::new("set-def-unused-media")
                    .checked(settings.default_include_unused_media)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let val = *checked;
                        this.mutate_settings(cx, |s| s.default_include_unused_media = val);
                    })),
            ))
            .child(self.render_setting_row(
                "Translate Color Grades",
                "Convert Lumetri Basic + Color Wheels grades to/from DaVinci Resolve Primaries.",
                Switch::new("set-def-lumetri")
                    .checked(settings.default_include_lumetri)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let val = *checked;
                        this.mutate_settings(cx, |s| s.default_include_lumetri = val);
                    })),
            ))
            .child(self.render_setting_row(
                "Preserve Sequence Timeline Markers",
                "Carry over sequence and clip markers with color, name, and comment metadata.",
                Switch::new("set-def-markers")
                    .checked(settings.default_include_markers)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let val = *checked;
                        this.mutate_settings(cx, |s| s.default_include_markers = val);
                    })),
            ))
            .child(self.render_setting_row(
                "Preserve Clip Label Colors",
                "Map clip label colors between Premiere Pro, DaVinci Resolve, and Final Cut Pro.",
                Switch::new("set-def-colors")
                    .checked(settings.default_include_clip_colors)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let val = *checked;
                        this.mutate_settings(cx, |s| s.default_include_clip_colors = val);
                    })),
            ))
            .child(self.render_setting_row(
                "Inject Effect Review Markers",
                "Add color-coded timeline markers noting original parameters for mapped or unmapped effects.",
                Switch::new("set-def-fx-markers")
                    .checked(settings.default_inject_fx_markers)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let val = *checked;
                        this.mutate_settings(cx, |s| s.default_inject_fx_markers = val);
                    })),
            ))
            .child(self.render_setting_row(
                "Flatten Nested, Compound & Multicam Clips",
                "Resolve nested sequences and multicam cuts down to underlying source media clips.",
                Switch::new("set-def-flatten")
                    .checked(settings.default_flatten_nested)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let val = *checked;
                        this.mutate_settings(cx, |s| s.default_flatten_nested = val);
                    })),
            )),
        )
    }

    fn render_relinking_section(&self, settings: &AppSettings, cx: &mut Context<Self>) -> Div {
        v_flex().w_full().gap(px(16.)).child(
            self.render_card(
                "Default Media Path Prefix Rewrite",
                Some(
                    "Useful when regularly migrating projects between a Windows edit bay and a macOS grading suite (e.g. D:\\Media → /Volumes/RAID/Media).",
                ),
            )
            .child(self.render_setting_row(
                "Auto-Fill Relinker on Startup & Project Load",
                "Automatically populate the Media Path Relinker fields when empty.",
                Switch::new("set-auto-fill-relink")
                    .checked(settings.auto_fill_relink)
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let val = *checked;
                        this.mutate_settings(cx, |s| s.auto_fill_relink = val);
                    })),
            ))
            .child(
                v_flex()
                    .w_full()
                    .pt(px(12.))
                    .gap(px(10.))
                    .child(
                        v_flex()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(theme::TEXT))
                                    .child("Default Source Prefix (From)"),
                            )
                            .child(Input::new(&self.relink_from_input)),
                    )
                    .child(
                        v_flex()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(theme::TEXT))
                                    .child("Default Destination Prefix (To)"),
                            )
                            .child(Input::new(&self.relink_to_input)),
                    )
                    .when(
                        !settings.default_relink_from.is_empty()
                            || !settings.default_relink_to.is_empty(),
                        |col| {
                            col.child(
                                h_flex().justify_end().pt(px(4.)).child(
                                    Button::new("set-relink-clear")
                                        .small()
                                        .label("Clear Default Prefixes")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.relink_from_input
                                                .update(cx, |s, cx| s.set_value("", window, cx));
                                            this.relink_to_input
                                                .update(cx, |s, cx| s.set_value("", window, cx));
                                            this.mutate_settings(cx, |s| {
                                                s.default_relink_from.clear();
                                                s.default_relink_to.clear();
                                            });
                                        })),
                                ),
                            )
                        },
                    ),
            ),
        )
    }

    fn render_shortcuts_section(&self) -> Div {
        let categories = [
            "File & Project",
            "Sequences & Effects",
            "Application & Window",
        ];

        let mut container = v_flex().w_full().gap(px(16.));
        for cat in categories {
            let mut card = self.render_card(cat, None);
            for entry in SHORTCUT_CATALOG.iter().filter(|e| e.category == cat) {
                let badge: AnyElement = match Keystroke::parse(entry.keystroke) {
                    Ok(stroke) => Kbd::new(stroke).outline().into_any_element(),
                    Err(_) => theme::pill(entry.keystroke, PillTone::Neutral).into_any_element(),
                };
                card = card.child(self.render_setting_row(entry.label, entry.description, badge));
            }
            container = container.child(card);
        }
        container
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<AppSettingsGlobal>().0.clone();

        let sidebar_items = SettingsSection::ALL.iter().enumerate().map(
            |(idx, &(section, title, _, ref icon))| {
                let is_active = self.active_section == section;
                h_flex()
                    .id(("settings-nav", idx))
                    .w_full()
                    .items_center()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(9.))
                    .cursor_pointer()
                    .when(is_active, |el| {
                        el.bg(theme::color_alpha(theme::ACCENT, 0.22))
                            .border_1()
                            .border_color(theme::color_alpha(theme::ACCENT, 0.48))
                            .text_color(rgb(theme::TEXT))
                    })
                    .when(!is_active, |el| {
                        el.border_1()
                            .border_color(theme::color_alpha(0xffffff, 0.0))
                            .text_color(rgb(theme::MUTED))
                            .hover(|s| {
                                s.bg(rgb(theme::SURFACE_2)).text_color(rgb(theme::TEXT))
                            })
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_section(section, cx);
                    }))
                    .child(Icon::new(icon.clone()).small())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(if is_active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .child(title),
                    )
            },
        );

        let sidebar = v_flex()
            .w(px(240.))
            .flex_shrink_0()
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(12.))
            .p(px(14.))
            .gap(px(12.))
            .child(
                v_flex()
                    .px(px(6.))
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT))
                            .child("Settings"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::MUTED))
                            .child("Changes save automatically"),
                    ),
            )
            .child(v_flex().gap(px(4.)).children(sidebar_items))
            .child(
                v_flex()
                    .mt(px(8.))
                    .pt(px(12.))
                    .border_t_1()
                    .border_color(rgb(theme::BORDER))
                    .gap(px(10.))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(4.))
                            .child(
                                img(assets::APP_ICON_ASSET)
                                    .w(px(28.))
                                    .h(px(28.))
                                    .rounded(px(7.)),
                            )
                            .child(
                                v_flex()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(theme::TEXT))
                                            .child(format!("v{APP_VERSION}")),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(theme::MUTED))
                                            .child("GPL-3.0-or-later"),
                                    ),
                            ),
                    )
                    .child(
                        Button::new("settings-open-about")
                            .small()
                            .icon(IconName::Info)
                            .label("About App...")
                            .on_click(|_, _, cx| {
                                open_about_window(cx);
                            }),
                    ),
            );

        let section_body = match self.active_section {
            SettingsSection::General => self.render_general_section(&settings, cx),
            SettingsSection::ConversionDefaults => {
                self.render_conversion_defaults_section(&settings, cx)
            }
            SettingsSection::MediaRelinking => self.render_relinking_section(&settings, cx),
            SettingsSection::Shortcuts => self.render_shortcuts_section(),
        };

        let show_apply_to_project = self.has_loaded_project
            && matches!(
                self.active_section,
                SettingsSection::ConversionDefaults | SettingsSection::MediaRelinking
            );

        let content_header = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap(px(12.))
            .child(
                v_flex()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(19.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::TEXT))
                            .child(self.active_section.title()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::MUTED))
                            .child(self.active_section.subtitle()),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .when(show_apply_to_project, |row| {
                        row.child(
                            Button::new("settings-apply-current")
                                .primary()
                                .small()
                                .label("Apply to Current Project")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.emit(SettingsViewEvent::ApplyDefaultsToProject);
                                    this.save_feedback = Some(
                                        "Applied defaults to the currently loaded project."
                                            .to_string(),
                                    );
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.active_section != SettingsSection::Shortcuts, |row| {
                        row.child(
                            Button::new("settings-reset-defaults")
                                .small()
                                .icon(IconName::RotateCw)
                                .label("Reset Defaults")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reset_all_to_defaults(window, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("settings-back-btn")
                            .small()
                            .icon(IconName::ArrowLeft)
                            .label("Back")
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SettingsViewEvent::BackToConverter);
                            })),
                    ),
            );

        h_flex()
            .w_full()
            .items_start()
            .gap(px(20.))
            .child(sidebar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(16.))
                    .child(content_header)
                    .when_some(self.save_error.as_ref(), |col, err| {
                        col.child(
                            h_flex()
                                .w_full()
                                .px(px(14.))
                                .py(px(10.))
                                .rounded(px(9.))
                                .bg(theme::color_alpha(theme::RED, 0.16))
                                .border_1()
                                .border_color(theme::color_alpha(theme::RED, 0.45))
                                .text_xs()
                                .text_color(rgb(theme::RED_BANNER_TEXT))
                                .child(err.clone()),
                        )
                    })
                    .when_some(self.save_feedback.as_ref(), |col, msg| {
                        col.child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .px(px(14.))
                                .py(px(10.))
                                .rounded(px(9.))
                                .bg(theme::color_alpha(theme::GREEN, 0.16))
                                .border_1()
                                .border_color(theme::color_alpha(theme::GREEN, 0.45))
                                .text_xs()
                                .text_color(rgb(theme::GREEN_BANNER_TEXT))
                                .child(msg.clone())
                                .child(
                                    Button::new("settings-dismiss-feedback")
                                        .small()
                                        .label("OK")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.save_feedback = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .child(section_body),
            )
    }
}
