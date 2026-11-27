use crate::backend::{self, LoadedProject, SUPPORTED_TARGETS};
use crate::theme::{self, PillTone};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        input::{Input, InputState},
        scroll::ScrollableElement as _,
        select::{Select, SelectEvent, SelectItem, SelectState},
        ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, h_flex, v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};
use nle_core::{
    ConversionOptions, ConversionReport, NleFormat, PathRelinkRule, SequenceKind, Severity,
};
use nle_effects::{
    analyze_project_effects, DetectedEffectGroup, EffectMappingPreset, MappingAction,
    MappingFidelity, UserEffectDecisions,
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
struct TargetFormatItem {
    format: NleFormat,
    label: &'static str,
}

impl SelectItem for TargetFormatItem {
    type Value = NleFormat;

    fn title(&self) -> SharedString {
        SharedString::from(self.label)
    }

    fn value(&self) -> &Self::Value {
        &self.format
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EffectActionItem {
    action: MappingAction,
    label: SharedString,
}

impl SelectItem for EffectActionItem {
    type Value = MappingAction;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.action
    }
}

fn target_items() -> Vec<TargetFormatItem> {
    SUPPORTED_TARGETS
        .iter()
        .map(|&(format, label)| TargetFormatItem { format, label })
        .collect()
}

fn effect_action_items(fx: &DetectedEffectGroup) -> Vec<EffectActionItem> {
    let mut items = Vec::new();
    if fx.fidelity == MappingFidelity::NativeAutomatic {
        items.push(EffectActionItem {
            action: MappingAction::NativeConvert,
            label: "Convert Natively into Output Project (Recommended)".into(),
        });
    }
    for target in &fx.available_targets {
        let studio = if target.studio_only { " [Studio]" } else { "" };
        items.push(EffectActionItem {
            action: MappingAction::MapToEquivalent {
                target_effect_id: target.id.clone(),
                target_display_name: target.display_name.clone(),
                add_review_marker: true,
            },
            label: format!("Map → {}{studio}", target.display_name).into(),
        });
    }
    items.push(EffectActionItem {
        action: MappingAction::PlaceMarkerOnly,
        label: "Drop Effect + Place Timeline Marker with Parameters".into(),
    });
    items.push(EffectActionItem {
        action: MappingAction::Skip,
        label: "Skip Effect Completely".into(),
    });
    items
}

pub struct ConverterApp {
    loaded: Option<LoadedProject>,
    target_format: NleFormat,
    selected_seq_ids: HashSet<String>,
    decisions: BTreeMap<String, MappingAction>,

    // Conversion options
    include_bins: bool,
    include_unused_media: bool,
    include_markers: bool,
    include_clip_colors: bool,
    include_lumetri: bool,
    inject_fx_markers: bool,
    flatten_nested_and_multicam: bool,

    // Inputs & Selects
    target_select: Entity<SelectState<Vec<TargetFormatItem>>>,
    relink_from: Entity<InputState>,
    relink_to: Entity<InputState>,
    effect_selects: BTreeMap<String, Entity<SelectState<Vec<EffectActionItem>>>>,

    // Subscriptions
    _base_subscriptions: Vec<Subscription>,
    effect_subscriptions: Vec<Subscription>,

    // Status & Report
    busy: bool,
    busy_label: Option<SharedString>,
    error_msg: Option<String>,
    status_toast: Option<String>,
    report: Option<ConversionReport>,
}

impl ConverterApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let target_select = cx.new(|cx| {
            SelectState::new(
                target_items(),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });

        let relink_from = cx.new(|cx| {
            InputState::new(window, cx).placeholder("From prefix (e.g. /Media)")
        });
        let relink_to = cx.new(|cx| {
            InputState::new(window, cx).placeholder("To prefix (e.g. /Volumes/Edit)")
        });

        let target_sub = cx.subscribe_in(
            &target_select,
            window,
            |this, _select, event: &SelectEvent<Vec<TargetFormatItem>>, window, cx| {
                if let SelectEvent::Confirm(Some(new_fmt)) = event {
                    if *new_fmt != this.target_format {
                        this.target_format = *new_fmt;
                        this.reanalyze_effects(window, cx);
                    }
                }
            },
        );

        Self {
            loaded: None,
            target_format: NleFormat::DaVinciResolve,
            selected_seq_ids: HashSet::new(),
            decisions: BTreeMap::new(),
            include_bins: true,
            include_unused_media: true,
            include_markers: true,
            include_clip_colors: true,
            include_lumetri: true,
            inject_fx_markers: true,
            flatten_nested_and_multicam: false,
            target_select,
            relink_from,
            relink_to,
            effect_selects: BTreeMap::new(),
            _base_subscriptions: vec![target_sub],
            effect_subscriptions: Vec::new(),
            busy: false,
            busy_label: None,
            error_msg: None,
            status_toast: None,
            report: None,
        }
    }

    fn apply_loaded_project(
        &mut self,
        payload: LoadedProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let seq_set: HashSet<String> = payload
            .inspection
            .sequences
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let target_fmt = payload.effect_plan.target_format;

        let mut default_actions = BTreeMap::new();
        for fx in &payload.effect_plan.effects {
            default_actions.insert(fx.key.clone(), fx.default_action.clone());
        }

        self.target_format = target_fmt;
        self.selected_seq_ids = seq_set;
        self.decisions = default_actions;
        self.report = None;
        self.error_msg = None;
        self.status_toast = None;
        self.loaded = Some(payload);

        self.target_select.update(cx, |state, cx| {
            state.set_selected_value(&target_fmt, window, cx);
        });
        self.rebuild_effect_selects(window, cx);
        cx.notify();
    }

    fn reanalyze_effects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(loaded) = self.loaded.as_mut() else {
            return;
        };
        let seqs: Vec<String> = self.selected_seq_ids.iter().cloned().collect();
        let new_plan = analyze_project_effects(&loaded.project, &seqs, self.target_format);

        let mut next_actions = BTreeMap::new();
        for fx in &new_plan.effects {
            next_actions.insert(fx.key.clone(), fx.default_action.clone());
        }

        self.decisions = next_actions;
        loaded.effect_plan = new_plan;
        self.rebuild_effect_selects(window, cx);
        cx.notify();
    }

    fn rebuild_effect_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.effect_selects.clear();
        self.effect_subscriptions.clear();

        let Some(loaded) = self.loaded.as_ref() else {
            return;
        };

        for fx in &loaded.effect_plan.effects {
            let items = effect_action_items(fx);
            let current = self
                .decisions
                .get(&fx.key)
                .unwrap_or(&fx.default_action);
            let selected_row = items
                .iter()
                .position(|item| &item.action == current)
                .unwrap_or(0);

            let select_entity = cx.new(|cx| {
                SelectState::new(
                    items,
                    Some(IndexPath::default().row(selected_row)),
                    window,
                    cx,
                )
            });

            let key = fx.key.clone();
            let sub = cx.subscribe_in(
                &select_entity,
                window,
                move |this, _select, event: &SelectEvent<Vec<EffectActionItem>>, _window, cx| {
                    if let SelectEvent::Confirm(Some(action)) = event {
                        this.decisions.insert(key.clone(), action.clone());
                        cx.notify();
                    }
                },
            );

            self.effect_selects.insert(fx.key.clone(), select_entity);
            self.effect_subscriptions.push(sub);
        }
    }

    fn open_project_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.busy_label = Some("Inspecting project...".into());
        self.error_msg = None;
        let requested_target = self.target_format;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend::load_and_inspect_project(&path, requested_target) })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.busy_label = None;
                match result {
                    Ok(payload) => this.apply_loaded_project(payload, window, cx),
                    Err(err) => {
                        this.error_msg = Some(err);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn on_open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.busy_label = Some("Opening project...".into());
        self.error_msg = None;
        let requested_target = self.target_format;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Open NLE Project File")
                .add_filter("NLE Projects", &["prproj", "drp", "drt", "fcpbundle", "fcpxml", "fcpxmld"])
                .add_filter("Premiere Pro Project", &["prproj"])
                .add_filter("DaVinci Resolve Project", &["drp", "drt"])
                .add_filter("Final Cut Pro Library / XML", &["fcpbundle", "fcpxml", "fcpxmld"])
                .pick_file()
                .await;

            let Some(handle) = picked else {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    this.busy_label = None;
                    cx.notify();
                });
                return;
            };

            let path = handle.path().to_path_buf();
            let result = cx
                .background_executor()
                .spawn(async move { backend::load_and_inspect_project(&path, requested_target) })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.busy_label = None;
                match result {
                    Ok(payload) => this.apply_loaded_project(payload, window, cx),
                    Err(err) => {
                        this.error_msg = Some(err);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn on_load_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.busy_label = Some("Loading demo project...".into());
        self.error_msg = None;
        let requested_target = self.target_format;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend::load_demo_project(requested_target) })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.busy_label = None;
                match result {
                    Ok(payload) => this.apply_loaded_project(payload, window, cx),
                    Err(err) => {
                        this.error_msg = Some(err);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn on_convert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.selected_seq_ids.is_empty() {
            return;
        }
        let Some(loaded) = self.loaded.as_ref() else {
            return;
        };

        let source_project = loaded.project.clone();
        let target_format = self.target_format;
        let ext = target_format.default_extension();
        let default_name = format!("{}.{}", loaded.inspection.name, ext);
        let filter_name = target_format.display_name();

        let mut relink_rules = Vec::new();
        let rf = self.relink_from.read(cx).value().to_string();
        let rt = self.relink_to.read(cx).value().to_string();
        if !rf.trim().is_empty() {
            relink_rules.push(PathRelinkRule {
                from_prefix: rf,
                to_prefix: rt,
            });
        }

        let options = ConversionOptions {
            target_format,
            project_name: Some(loaded.inspection.name.clone()),
            selected_sequence_ids: self.selected_seq_ids.iter().cloned().collect(),
            include_bins: self.include_bins,
            include_unused_media: self.include_unused_media,
            include_markers: self.include_markers,
            include_clip_colors: self.include_clip_colors,
            include_lumetri_grades: self.include_lumetri,
            inject_effect_review_markers: self.inject_fx_markers,
            flatten_nested_and_multicam: self.flatten_nested_and_multicam,
            relink_rules,
        };
        let decisions = UserEffectDecisions {
            actions: self.decisions.clone(),
        };

        self.busy = true;
        self.busy_label = Some("Converting...".into());
        self.error_msg = None;
        self.status_toast = None;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let saved = rfd::AsyncFileDialog::new()
                .set_title("Save Converted Project")
                .set_file_name(&default_name)
                .add_filter(filter_name, &[ext])
                .save_file()
                .await;

            let Some(handle) = saved else {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    this.busy_label = None;
                    cx.notify();
                });
                return;
            };

            let output_path = handle.path().to_path_buf();
            let result = cx
                .background_executor()
                .spawn(async move {
                    backend::run_conversion(&source_project, &output_path, options, &decisions)
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.busy_label = None;
                match result {
                    Ok(report) => {
                        this.report = Some(report);
                    }
                    Err(err) => {
                        this.error_msg = Some(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_save_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(loaded) = self.loaded.as_ref() else {
            return;
        };
        let preset = EffectMappingPreset {
            name: format!("{} Mapping Preset", loaded.inspection.name),
            source_format: loaded.inspection.source_format,
            target_format: self.target_format,
            decisions: UserEffectDecisions {
                actions: self.decisions.clone(),
            },
        };

        cx.spawn_in(window, async move |this, cx| {
            let saved = rfd::AsyncFileDialog::new()
                .set_title("Save Effect Mapping Preset")
                .set_file_name("nle-effect-preset.json")
                .add_filter("Effect Preset JSON", &["json"])
                .save_file()
                .await;

            let Some(handle) = saved else {
                return;
            };
            let path = handle.path().to_path_buf();
            let path_display = path.to_string_lossy().into_owned();
            let result = cx
                .background_executor()
                .spawn(async move { backend::save_effect_preset(&path, &preset) })
                .await;

            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.status_toast =
                            Some(format!("Saved effect mapping preset to {path_display}"));
                    }
                    Err(err) => {
                        this.error_msg = Some(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_load_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Load Effect Mapping Preset")
                .add_filter("Effect Preset JSON", &["json"])
                .pick_file()
                .await;

            let Some(handle) = picked else {
                return;
            };
            let path = handle.path().to_path_buf();
            let result = cx
                .background_executor()
                .spawn(async move { backend::load_effect_preset(&path) })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(preset) => {
                        for (k, v) in preset.decisions.actions {
                            if let Some(sel) = this.effect_selects.get(&k) {
                                sel.update(cx, |state, cx| {
                                    state.set_selected_value(&v, window, cx);
                                });
                            }
                            this.decisions.insert(k, v);
                        }
                        this.status_toast = Some(format!("Loaded preset: {}", preset.name));
                    }
                    Err(err) => {
                        this.error_msg = Some(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .px(px(22.))
            .py(px(14.))
            .bg(rgb(theme::SURFACE))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .gap(px(16.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .w(px(34.))
                            .h(px(34.))
                            .rounded(px(9.))
                            .bg(linear_gradient(
                                135.,
                                linear_color_stop(rgb(theme::ACCENT), 0.),
                                linear_color_stop(rgb(0x6366f1), 1.),
                            ))
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_weight(FontWeight::BOLD)
                            .text_sm()
                            .text_color(rgb(0xffffff))
                            .child("AN"),
                    )
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::TEXT))
                                    .child("Aagedal NLE Project Converter"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme::MUTED))
                                    .child("Direct Native Project Conversion · Rust + GPUI · GPL-3.0"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Button::new("topbar-demo")
                            .label("Load Demo .prproj")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_load_demo(window, cx);
                            })),
                    )
                    .child(
                        Button::new("topbar-open")
                            .primary()
                            .label("Open Project...")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_open_file(window, cx);
                            })),
                    ),
            )
    }

    fn render_banners(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();

        if let Some(err) = &self.error_msg {
            out.push(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .px(px(16.))
                    .py(px(12.))
                    .rounded(px(10.))
                    .bg(theme::color_alpha(theme::RED, 0.16))
                    .border_1()
                    .border_color(theme::color_alpha(theme::RED, 0.45))
                    .text_color(rgb(theme::RED_BANNER_TEXT))
                    .text_sm()
                    .child(div().flex_1().child(err.clone()))
                    .child(
                        Button::new("dismiss-err")
                            .small()
                            .label("Dismiss")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.error_msg = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }

        if let Some(msg) = &self.status_toast {
            out.push(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .px(px(16.))
                    .py(px(12.))
                    .rounded(px(10.))
                    .bg(theme::color_alpha(theme::GREEN, 0.16))
                    .border_1()
                    .border_color(theme::color_alpha(theme::GREEN, 0.45))
                    .text_color(rgb(theme::GREEN_BANNER_TEXT))
                    .text_sm()
                    .child(div().flex_1().child(msg.clone()))
                    .child(
                        Button::new("dismiss-toast")
                            .small()
                            .label("OK")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.status_toast = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }

        out
    }

    fn render_hero(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .justify_center()
            .pt(px(36.))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(720.))
                    .bg(rgb(theme::SURFACE))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .rounded(px(16.))
                    .px(px(36.))
                    .py(px(40.))
                    .items_center()
                    .gap(px(16.))
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::TEXT))
                            .text_center()
                            .child("Convert NLE Projects Directly — No XML Required"),
                    )
                    .child(
                        div()
                            .max_w(px(560.))
                            .text_sm()
                            .text_color(rgb(theme::MUTED))
                            .text_center()
                            .child(
                                "Read native Adobe Premiere Pro (.prproj), DaVinci Resolve (.drp), and Final Cut Pro (.fcpbundle / .fcpxml / .fcpxmld) projects directly in Rust, review app-exclusive effects interactively, and export ready-to-open projects. Drop a file here or open one below.",
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_center()
                            .flex_wrap()
                            .gap(px(12.))
                            .pt(px(8.))
                            .pb(px(12.))
                            .child(
                                Button::new("hero-open")
                                    .primary()
                                    .label("Choose .prproj, .drp, or FCP File")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.on_open_file(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("hero-demo")
                                    .label("Try Built-in Demo Project")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.on_load_demo(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_center()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(theme::pill(
                                "Premiere .prproj ↔ Resolve .drp (Native)",
                                PillTone::Purple,
                            ))
                            .child(theme::pill(
                                "Final Cut Pro .fcpbundle / .fcpxml ↔ Resolve / Premiere",
                                PillTone::Cyan,
                            ))
                            .child(theme::pill(
                                "Interactive Effect Mapping & Review Markers",
                                PillTone::Amber,
                            ))
                            .child(theme::pill(
                                "100% Local · Free & Open Source",
                                PillTone::Green,
                            )),
                    ),
            )
    }

    fn stat_box(&self, value: impl Into<SharedString>, label: &'static str, mono: &SharedString) -> Div {
        v_flex()
            .flex_1()
            .min_w(px(96.))
            .bg(rgb(theme::SURFACE_2))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(8.))
            .p(px(10.))
            .gap(px(2.))
            .child(
                div()
                    .font_family(mono.clone())
                    .text_size(px(18.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme::TEXT))
                    .child(value.into()),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(theme::MUTED))
                    .child(label.to_uppercase()),
            )
    }

    fn render_report_panel(&self, rep: &ConversionReport, cx: &mut Context<Self>) -> Div {
        let out_path_reveal = rep.output_path.clone();
        let mono = cx.theme().mono_font_family.clone();

        v_flex()
            .w_full()
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(theme::color_alpha(theme::GREEN, 0.5))
            .rounded(px(12.))
            .px(px(18.))
            .py(px(16.))
            .gap(px(12.))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::GREEN_TEXT))
                                    .child("Conversion Complete"),
                            )
                            .child(
                                div()
                                    .font_family(mono.clone())
                                    .text_xs()
                                    .text_color(rgb(theme::MUTED))
                                    .child(rep.output_path.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                Button::new("report-reveal")
                                    .primary()
                                    .small()
                                    .label("Reveal in Finder")
                                    .on_click(move |_, _, cx| {
                                        cx.reveal_path(Path::new(&out_path_reveal));
                                    }),
                            )
                            .child(
                                Button::new("report-hide")
                                    .small()
                                    .label("Hide")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.report = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(self.stat_box(rep.sequences_converted.to_string(), "Sequences", &mono))
                    .child(self.stat_box(rep.video_clips_converted.to_string(), "Video Clips", &mono))
                    .child(self.stat_box(rep.audio_clips_converted.to_string(), "Audio Clips", &mono))
                    .child(self.stat_box(rep.bins_created.to_string(), "Bins", &mono))
                    .child(self.stat_box(
                        rep.grades_attached.to_string(),
                        if rep.target_format == NleFormat::DaVinciResolve {
                            "DRX Grades"
                        } else {
                            "Color Grades"
                        },
                        &mono,
                    ))
                    .child(self.stat_box(rep.markers_written.to_string(), "Markers", &mono))
                    .child(self.stat_box(
                        rep.effect_markers_injected.to_string(),
                        "FX Markers",
                        &mono,
                    )),
            )
            .when(!rep.messages.is_empty(), |panel| {
                let entries = rep.messages.iter().enumerate().map(|(idx, m)| {
                    let color = match m.severity {
                        Severity::Info => rgb(theme::INFO_LOG),
                        Severity::Warning => rgb(theme::AMBER_TEXT),
                        Severity::Error => rgb(theme::RED_TEXT),
                    };
                    let prefix = match (&m.sequence, &m.clip) {
                        (Some(s), Some(c)) => format!("[{s} · {c}] "),
                        (Some(s), None) => format!("[{s}] "),
                        _ => String::new(),
                    };
                    div()
                        .id(("log-entry", idx))
                        .py(px(3.))
                        .border_b_1()
                        .border_color(theme::color_alpha(0xffffff, 0.04))
                        .font_family(mono.clone())
                        .text_xs()
                        .text_color(color)
                        .child(format!("{}{} ({})", prefix, m.message, m.code))
                });

                panel.child(
                    div()
                        .w_full()
                        .max_h(px(220.))
                        .bg(rgb(theme::BG))
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .rounded(px(8.))
                        .px(px(12.))
                        .py(px(10.))
                        .child(
                            v_flex()
                                .id("report-log-scroll")
                                .max_h(px(198.))
                                .overflow_y_scrollbar()
                                .children(entries),
                        ),
                )
            })
    }

    fn render_workspace(&self, loaded: &LoadedProject, cx: &mut Context<Self>) -> impl IntoElement {
        let insp = &loaded.inspection;
        let plan = &loaded.effect_plan;
        let mono = cx.theme().mono_font_family.clone();
        let color_label = insp
            .working_color_space
            .clone()
            .unwrap_or_else(|| "Rec. 709 / Default".into());

        let convert_label: SharedString = if self.busy {
            self.busy_label
                .clone()
                .unwrap_or_else(|| "Converting...".into())
        } else {
            "Convert & Save...".into()
        };

        let convert_bar = h_flex()
            .w_full()
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(12.))
            .px(px(20.))
            .py(px(16.))
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap(px(16.))
            .child(
                v_flex()
                    .gap(px(4.))
                    .child(
                        h_flex()
                            .items_center()
                            .flex_wrap()
                            .gap(px(10.))
                            .child(
                                div()
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::TEXT))
                                    .child(insp.name.clone()),
                            )
                            .child(theme::pill(
                                insp.source_format.display_name(),
                                PillTone::Purple,
                            ))
                            .child(theme::pill(
                                format!("Color: {color_label}"),
                                PillTone::Neutral,
                            )),
                    )
                    .when_some(insp.source_path.clone(), |el, path| {
                        el.child(
                            div()
                                .font_family(mono.clone())
                                .text_xs()
                                .text_color(rgb(theme::MUTED))
                                .child(path),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::MUTED))
                            .child("Convert to:"),
                    )
                    .child(div().w(px(270.)).child(Select::new(&self.target_select)))
                    .child(
                        Button::new("convert-btn")
                            .primary()
                            .label(convert_label)
                            .disabled(self.busy || self.selected_seq_ids.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_convert(window, cx);
                            })),
                    ),
            );

        // LEFT COLUMN
        let sequence_rows = insp.sequences.iter().enumerate().map(|(idx, seq)| {
            let sid = seq.id.clone();
            let is_selected = self.selected_seq_ids.contains(&sid);
            let specs_line1 = format!(
                "{}×{} · {} fps · {}",
                seq.width, seq.height, seq.fps_label, seq.duration_tc
            );
            let mut specs_line2 = format!(
                "V:{} ({} clips) · A:{} ({} clips) · {} mk · {} fx",
                seq.video_track_count,
                seq.video_clip_count,
                seq.audio_track_count,
                seq.audio_clip_count,
                seq.marker_count,
                seq.effect_count
            );
            if seq.transition_count > 0 {
                specs_line2.push_str(&format!(" · {} tr", seq.transition_count));
            }
            if seq.retimed_clip_count > 0 {
                specs_line2.push_str(&format!(" · {} retimed", seq.retimed_clip_count));
            }
            if seq.nested_clip_count > 0 {
                specs_line2.push_str(&format!(" · {} nested", seq.nested_clip_count));
            }
            let kind_badge = match seq.kind {
                SequenceKind::CompoundClip => Some(theme::pill("Compound", PillTone::Cyan)),
                SequenceKind::MulticamClip => Some(theme::pill("Multicam", PillTone::Purple)),
                SequenceKind::Timeline => None,
            };

            let sid_text = sid.clone();
            h_flex()
                .id(("seq-row", idx))
                .w_full()
                .items_start()
                .gap(px(10.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(9.))
                .bg(rgb(theme::SURFACE_2))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .hover(|s| s.border_color(rgb(theme::BORDER_STRONG)))
                .child(
                    Checkbox::new(("seq-chk", idx))
                        .checked(is_selected)
                        .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                            if *checked {
                                this.selected_seq_ids.insert(sid.clone());
                            } else {
                                this.selected_seq_ids.remove(&sid);
                            }
                            this.reanalyze_effects(window, cx);
                        })),
                )
                .child(
                    v_flex()
                        .id(("seq-info", idx))
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.selected_seq_ids.contains(&sid_text) {
                                this.selected_seq_ids.remove(&sid_text);
                            } else {
                                this.selected_seq_ids.insert(sid_text.clone());
                            }
                            this.reanalyze_effects(window, cx);
                        }))
                        .child(
                            h_flex()
                                .items_center()
                                .flex_wrap()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(theme::TEXT))
                                        .child(seq.name.clone()),
                                )
                                .when_some(kind_badge, |row, badge| row.child(badge)),
                        )
                        .child(
                            div()
                                .font_family(mono.clone())
                                .text_xs()
                                .text_color(rgb(theme::MUTED))
                                .child(specs_line1),
                        )
                        .child(
                            div()
                                .font_family(mono.clone())
                                .text_xs()
                                .text_color(rgb(theme::MUTED))
                                .child(specs_line2),
                        ),
                )
        });

        let overview_panel = v_flex()
            .w_full()
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(12.))
            .px(px(18.))
            .py(px(16.))
            .gap(px(12.))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child("Project Overview"),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(self.stat_box(insp.sequences.len().to_string(), "Sequences", &mono))
                    .child(self.stat_box(insp.bin_count.to_string(), "Bins", &mono))
                    .child(self.stat_box(insp.media_count.to_string(), "Media", &mono)),
            )
            .when(insp.missing_media_count > 0, |panel| {
                panel.child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::AMBER_TEXT))
                        .child(format!(
                            "{} media files not found at original paths — use Path Relinker below if drives moved (offline conversion still works).",
                            insp.missing_media_count
                        )),
                )
            })
            .child(
                h_flex()
                    .pt(px(4.))
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT))
                            .child("Sequences to Convert"),
                    )
                    .when(insp.sequences.len() > 1, |hdr| {
                        hdr.child(
                            h_flex()
                                .gap(px(6.))
                                .child(
                                    Button::new("seq-select-all")
                                        .small()
                                        .label("All")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if let Some(loaded) = &this.loaded {
                                                this.selected_seq_ids = loaded
                                                    .inspection
                                                    .sequences
                                                    .iter()
                                                    .map(|s| s.id.clone())
                                                    .collect();
                                            }
                                            this.reanalyze_effects(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("seq-select-none")
                                        .small()
                                        .label("None")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.selected_seq_ids.clear();
                                            this.reanalyze_effects(window, cx);
                                        })),
                                ),
                        )
                    }),
            )
            .child(v_flex().gap(px(8.)).children(sequence_rows));

        let options_panel = v_flex()
            .w_full()
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(12.))
            .px(px(18.))
            .py(px(16.))
            .gap(px(10.))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child("Conversion Options"),
            )
            .child(
                Checkbox::new("opt-bins")
                    .checked(self.include_bins)
                    .label("Preserve nested project bins (Media Pool folders)")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.include_bins = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("opt-unused-media")
                    .checked(self.include_unused_media)
                    .label("Preserve unplaced project bin media")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.include_unused_media = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("opt-lumetri")
                    .checked(self.include_lumetri)
                    .label("Translate color grades (Lumetri Basic + Color Wheels ↔ Resolve Primaries)")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.include_lumetri = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("opt-markers")
                    .checked(self.include_markers)
                    .label("Preserve sequence timeline markers")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.include_markers = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("opt-colors")
                    .checked(self.include_clip_colors)
                    .label("Preserve clip label colors across NLEs")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.include_clip_colors = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("opt-fx-markers")
                    .checked(self.inject_fx_markers)
                    .label("Inject color-coded timeline markers for mapped/unmapped effects")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.inject_fx_markers = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("opt-flatten-nested")
                    .checked(self.flatten_nested_and_multicam)
                    .label("Flatten nested sequences, compound & multicam clips to source media")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.flatten_nested_and_multicam = *checked;
                        cx.notify();
                    })),
            );

        let relink_panel = v_flex()
            .w_full()
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(12.))
            .px(px(18.))
            .py(px(16.))
            .gap(px(10.))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child("Media Path Relinker (Optional)"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child("Rewrite drive/folder prefixes when migrating across workstations (e.g. D:\\Media → /Volumes/RAID/Media)."),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap(px(8.))
                    .child(div().flex_1().child(Input::new(&self.relink_from)))
                    .child(div().flex_1().child(Input::new(&self.relink_to))),
            );

        let left_column = v_flex()
            .w(px(380.))
            .flex_shrink_0()
            .gap(px(16.))
            .child(overview_panel)
            .child(options_panel)
            .child(relink_panel);

        // RIGHT COLUMN: Interactive Effect Mapping Review
        let fx_body: AnyElement = if plan.effects.is_empty() {
            div()
                .w_full()
                .p(px(24.))
                .rounded(px(10.))
                .bg(rgb(theme::SURFACE_2))
                .text_center()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child("No non-default or app-exclusive effects detected in the selected sequences.")
                .into_any_element()
        } else {
            let cards = plan.effects.iter().enumerate().map(|(idx, fx)| {
                let (accent_border, badge_tone, badge_txt) = match fx.fidelity {
                    MappingFidelity::AskUserSuggested => (
                        rgb(theme::AMBER),
                        PillTone::Amber,
                        "Ask User · Equivalent Available",
                    ),
                    MappingFidelity::Unmapped => {
                        (rgb(theme::RED), PillTone::Red, "Unmapped / Plugin")
                    }
                    MappingFidelity::NativeAutomatic => (
                        rgb(theme::GREEN),
                        PillTone::Green,
                        "Native Auto-Convert",
                    ),
                };

                let match_line = if fx.has_keyframes {
                    format!("{} · Animated (keyframes)", fx.match_name)
                } else {
                    fx.match_name.clone()
                };

                let sample_line = if fx.sample_clips.is_empty() {
                    None
                } else {
                    Some(format!("Clips: {}", fx.sample_clips.join(", ")))
                };

                let select_el = self
                    .effect_selects
                    .get(&fx.key)
                    .map(|state| Select::new(state).into_any_element());

                v_flex()
                    .id(("fx-card", idx))
                    .w_full()
                    .bg(rgb(theme::SURFACE_2))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .rounded(px(10.))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .w_full()
                            .child(div().w(px(3.)).flex_shrink_0().bg(accent_border))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(15.))
                                    .py(px(14.))
                                    .gap(px(8.))
                                    .child(
                                        h_flex()
                                            .items_start()
                                            .justify_between()
                                            .flex_wrap()
                                            .gap(px(10.))
                                            .child(
                                                v_flex()
                                                    .gap(px(2.))
                                                    .child(
                                                        h_flex()
                                                            .items_baseline()
                                                            .gap(px(8.))
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .font_weight(
                                                                        FontWeight::SEMIBOLD,
                                                                    )
                                                                    .text_color(rgb(theme::TEXT))
                                                                    .child(fx.display_name.clone()),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(rgb(theme::MUTED))
                                                                    .child(format!(
                                                                        "×{} clips",
                                                                        fx.clip_count
                                                                    )),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .font_family(mono.clone())
                                                            .text_xs()
                                                            .text_color(rgb(theme::MUTED))
                                                            .child(match_line),
                                                    ),
                                            )
                                            .child(theme::pill(badge_txt, badge_tone)),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(theme::MUTED))
                                            .child(fx.notes.clone()),
                                    )
                                    .when_some(select_el, |card, el| {
                                        card.child(div().w_full().child(el))
                                    })
                                    .when_some(sample_line, |card, line| {
                                        card.child(
                                            div()
                                                .font_family(mono.clone())
                                                .text_xs()
                                                .text_color(rgb(theme::MUTED))
                                                .child(line),
                                        )
                                    }),
                            ),
                    )
            });

            v_flex().w_full().gap(px(10.)).children(cards).into_any_element()
        };

        let right_column = v_flex()
            .flex_1()
            .min_w_0()
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(12.))
            .px(px(18.))
            .py(px(16.))
            .gap(px(14.))
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::TEXT))
                                    .child("Interactive Effect Translation & Mapping"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme::MUTED))
                                    .child("App-exclusive effects are detected automatically so you can choose corresponding target NLE equivalents or place annotated review markers."),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .flex_shrink_0()
                            .child(
                                Button::new("load-preset")
                                    .small()
                                    .label("Load Preset")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.on_load_preset(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("save-preset")
                                    .small()
                                    .label("Save Preset")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.on_save_preset(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(theme::pill(
                        format!("{} Ask User / Suggested", plan.needs_user_review_count),
                        PillTone::Amber,
                    ))
                    .child(theme::pill(
                        format!("{} Unmapped / 3rd Party", plan.unmapped_count),
                        PillTone::Red,
                    ))
                    .child(theme::pill(
                        format!("{} Native Auto-Convert", plan.native_auto_count),
                        PillTone::Green,
                    )),
            )
            .child(fx_body);

        v_flex()
            .w_full()
            .gap(px(18.))
            .when_some(self.report.as_ref(), |el, rep| {
                el.child(self.render_report_panel(rep, cx))
            })
            .child(convert_bar)
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap(px(18.))
                    .child(left_column)
                    .child(right_column),
            )
    }
}

impl Render for ConverterApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let banners = self.render_banners(cx);

        let main_body: AnyElement = match &self.loaded {
            None => self.render_hero(cx).into_any_element(),
            Some(loaded) => self.render_workspace(loaded, cx).into_any_element(),
        };

        v_flex()
            .id("app-shell")
            .size_full()
            .bg(rgb(theme::BG))
            .text_color(rgb(theme::TEXT))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if let Some(first) = paths.paths().first() {
                    this.open_project_path(first.clone(), window, cx);
                }
            }))
            .child(self.render_topbar(cx))
            .child(
                div()
                    .id("main-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(1320.))
                            .mx_auto()
                            .px(px(22.))
                            .pt(px(20.))
                            .pb(px(32.))
                            .gap(px(16.))
                            .children(banners)
                            .child(main_body),
                    ),
            )
    }
}
