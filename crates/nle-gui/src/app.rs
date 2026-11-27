use crate::about::open_about_window;
use crate::actions::{
    ApplyRecommendedEffects, CloseProject, CloseWindow, ConvertProject, CopyConversionLog,
    DeselectAllSequences, DismissOverlay, LoadDemoProject, LoadEffectPreset, MinimizeWindow,
    OpenProject, OpenSettings, OpenShortcuts, RevealOutput, SaveEffectPreset, SelectAllSequences,
    ZoomWindow, CONVERT_KEYSTROKE, OPEN_PROJECT_KEYSTROKE, SETTINGS_KEYSTROKE,
};
use crate::assets;
use crate::backend::{self, LoadedProject, SUPPORTED_TARGETS};
use crate::settings::{AppSettings, AppSettingsGlobal, DefaultEffectPolicy};
use crate::settings_view::{SettingsSection, SettingsView, SettingsViewEvent};
use crate::theme::{self, PillTone};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        input::{Input, InputState},
        kbd::Kbd,
        scroll::ScrollableElement as _,
        select::{Select, SelectEvent, SelectItem, SelectState},
        ActiveTheme as _, Disableable as _, IconName, IndexPath, Selectable as _, Sizable as _,
        h_flex, v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};
use nle_core::{
    ConversionOptions, ConversionReport, NleFormat, PathRelinkRule, SequenceKind, Severity,
};
use nle_effects::{
    analyze_project_effects, DetectedEffectGroup, EffectMappingPreset, InstalledPluginRegistry,
    MappingAction, MappingFidelity, UserEffectDecisions,
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivePage {
    Converter,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BulkEffectMode {
    Recommended,
    MarkerOnly,
    SkipAll,
}

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

fn format_conversion_log(rep: &ConversionReport) -> String {
    rep.messages
        .iter()
        .map(|m| {
            let prefix = match (&m.sequence, &m.clip) {
                (Some(s), Some(c)) => format!("[{s} · {c}] "),
                (Some(s), None) => format!("[{s}] "),
                _ => String::new(),
            };
            format!("[{:?}] {}{} ({})", m.severity, prefix, m.message, m.code)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn shortcut_hint_badge(keystroke: &'static str) -> Option<Kbd> {
    Keystroke::parse(keystroke).ok().map(|s| Kbd::new(s).outline())
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
    focus_handle: FocusHandle,
    active_page: ActivePage,
    settings_view: Entity<SettingsView>,

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
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let initial_settings = cx
            .try_global::<AppSettingsGlobal>()
            .map(|g| g.0.clone())
            .unwrap_or_default();

        let targets = target_items();
        let initial_target_row = targets
            .iter()
            .position(|item| item.format == initial_settings.default_target_format)
            .unwrap_or(0);
        let initial_target = targets
            .get(initial_target_row)
            .map(|item| item.format)
            .unwrap_or(NleFormat::DaVinciResolve);

        let target_select = cx.new(|cx| {
            SelectState::new(
                targets,
                Some(IndexPath::default().row(initial_target_row)),
                window,
                cx,
            )
        });

        let initial_relink_from = if initial_settings.auto_fill_relink {
            initial_settings.default_relink_from.clone()
        } else {
            String::new()
        };
        let initial_relink_to = if initial_settings.auto_fill_relink {
            initial_settings.default_relink_to.clone()
        } else {
            String::new()
        };

        let relink_from = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("From prefix (e.g. /Media)")
                .default_value(initial_relink_from)
        });
        let relink_to = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("To prefix (e.g. /Volumes/Edit)")
                .default_value(initial_relink_to)
        });

        let settings_view = cx.new(|cx| SettingsView::new(window, cx));

        let relink_from_obs = cx.observe(&relink_from, |_, _, cx| {
            cx.notify();
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

        let settings_sub = cx.subscribe_in(
            &settings_view,
            window,
            |this, _, event: &SettingsViewEvent, window, cx| match event {
                SettingsViewEvent::Changed => {
                    if this.loaded.is_none() {
                        let s = this.current_settings(cx);
                        this.apply_defaults_from_settings(&s, window, cx);
                    }
                    cx.notify();
                }
                SettingsViewEvent::ApplyDefaultsToProject => {
                    let s = this.current_settings(cx);
                    this.apply_defaults_from_settings(&s, window, cx);
                    this.status_toast =
                        Some("Applied default settings to current project.".to_string());
                    cx.notify();
                }
                SettingsViewEvent::BackToConverter => {
                    this.active_page = ActivePage::Converter;
                    window.focus(&this.focus_handle, cx);
                    cx.notify();
                }
            },
        );

        Self {
            focus_handle,
            active_page: ActivePage::Converter,
            settings_view,
            loaded: None,
            target_format: initial_target,
            selected_seq_ids: HashSet::new(),
            decisions: BTreeMap::new(),
            include_bins: initial_settings.default_include_bins,
            include_unused_media: initial_settings.default_include_unused_media,
            include_markers: initial_settings.default_include_markers,
            include_clip_colors: initial_settings.default_include_clip_colors,
            include_lumetri: initial_settings.default_include_lumetri,
            inject_fx_markers: initial_settings.default_inject_fx_markers,
            flatten_nested_and_multicam: initial_settings.default_flatten_nested,
            target_select,
            relink_from,
            relink_to,
            effect_selects: BTreeMap::new(),
            _base_subscriptions: vec![target_sub, relink_from_obs, settings_sub],
            effect_subscriptions: Vec::new(),
            busy: false,
            busy_label: None,
            error_msg: None,
            status_toast: None,
            report: None,
        }
    }

    fn current_settings(&self, cx: &App) -> AppSettings {
        cx.try_global::<AppSettingsGlobal>()
            .map(|g| g.0.clone())
            .unwrap_or_default()
    }

    fn remember_directory(&self, path: &Path, cx: &mut Context<Self>) {
        let Some(parent) = path.parent() else {
            return;
        };
        if !parent.is_dir() {
            return;
        }
        let dir_str = parent.to_string_lossy().into_owned();
        if let Some(global) = cx.try_global::<AppSettingsGlobal>() {
            if !global.0.remember_last_directory
                || global.0.last_directory.as_deref() == Some(dir_str.as_str())
            {
                return;
            }
        } else {
            return;
        }
        let updated = {
            let g = cx.global_mut::<AppSettingsGlobal>();
            g.0.last_directory = Some(dir_str);
            g.0.clone()
        };
        let _ = updated.save();
    }

    fn apply_defaults_from_settings(
        &mut self,
        s: &AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.include_bins = s.default_include_bins;
        self.include_unused_media = s.default_include_unused_media;
        self.include_markers = s.default_include_markers;
        self.include_clip_colors = s.default_include_clip_colors;
        self.include_lumetri = s.default_include_lumetri;
        self.inject_fx_markers = s.default_inject_fx_markers;
        self.flatten_nested_and_multicam = s.default_flatten_nested;

        if self.loaded.is_none() {
            self.target_format = s.default_target_format;
            self.target_select.update(cx, |state, cx| {
                state.set_selected_value(&s.default_target_format, window, cx);
            });
        }

        if s.auto_fill_relink {
            let from_val = s.default_relink_from.clone();
            let to_val = s.default_relink_to.clone();
            self.relink_from.update(cx, |input, cx| {
                input.set_value(from_val, window, cx);
            });
            self.relink_to.update(cx, |input, cx| {
                input.set_value(to_val, window, cx);
            });
        }
    }

    pub fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.active_page = match self.active_page {
            ActivePage::Converter => ActivePage::Settings,
            ActivePage::Settings => ActivePage::Converter,
        };
        cx.notify();
    }

    pub fn open_settings_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.active_page = ActivePage::Settings;
        self.settings_view.update(cx, |view, cx| {
            view.set_section(section, cx);
        });
        cx.notify();
    }

    pub fn trigger_open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_open_file(window, cx);
    }

    pub fn trigger_load_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_load_demo(window, cx);
    }

    pub fn trigger_convert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_convert(window, cx);
    }

    pub fn trigger_close_project(&mut self, cx: &mut Context<Self>) {
        if self.loaded.is_none() || self.busy {
            return;
        }
        self.loaded = None;
        self.selected_seq_ids.clear();
        self.decisions.clear();
        self.effect_selects.clear();
        self.effect_subscriptions.clear();
        self.report = None;
        self.error_msg = None;
        self.status_toast = None;
        self.active_page = ActivePage::Converter;
        self.settings_view
            .update(cx, |v, cx| v.set_has_loaded_project(false, cx));
        cx.notify();
    }

    pub fn trigger_load_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded.is_some() {
            self.on_load_preset(window, cx);
        }
    }

    pub fn trigger_save_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded.is_some() {
            self.on_save_preset(window, cx);
        }
    }

    pub fn trigger_select_all_sequences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(loaded) = &self.loaded else {
            return;
        };
        self.selected_seq_ids = loaded
            .inspection
            .sequences
            .iter()
            .map(|s| s.id.clone())
            .collect();
        self.reanalyze_effects(window, cx);
    }

    pub fn trigger_deselect_all_sequences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded.is_none() {
            return;
        }
        self.selected_seq_ids.clear();
        self.reanalyze_effects(window, cx);
    }

    pub fn trigger_apply_recommended_effects(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loaded.is_some() {
            self.apply_bulk_effect_action(BulkEffectMode::Recommended, window, cx);
        }
    }

    pub fn trigger_reveal_output(&mut self, cx: &mut Context<Self>) {
        if let Some(rep) = &self.report {
            cx.reveal_path(Path::new(&rep.output_path));
        }
    }

    pub fn trigger_copy_conversion_log(&mut self, cx: &mut Context<Self>) {
        let Some(rep) = &self.report else {
            return;
        };
        if rep.messages.is_empty() {
            return;
        }
        let log_text = format_conversion_log(rep);
        cx.write_to_clipboard(ClipboardItem::new_string(log_text));
        self.status_toast = Some("Copied conversion log to clipboard".to_string());
        cx.notify();
    }

    fn apply_loaded_project(
        &mut self,
        payload: LoadedProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.current_settings(cx);
        let seq_set: HashSet<String> = payload
            .inspection
            .sequences
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let target_fmt = payload.effect_plan.target_format;

        let mut default_actions = BTreeMap::new();
        for fx in &payload.effect_plan.effects {
            let action = match settings.default_effect_policy {
                DefaultEffectPolicy::Recommended => fx.default_action.clone(),
                DefaultEffectPolicy::MarkerOnly => MappingAction::PlaceMarkerOnly,
                DefaultEffectPolicy::SkipAll => MappingAction::Skip,
            };
            default_actions.insert(fx.key.clone(), action);
        }

        self.target_format = target_fmt;
        self.selected_seq_ids = seq_set;
        self.decisions = default_actions;
        self.report = None;
        self.error_msg = None;
        self.status_toast = None;
        self.active_page = ActivePage::Converter;
        self.loaded = Some(payload);

        if settings.auto_fill_relink
            && self.relink_from.read(cx).value().trim().is_empty()
            && !settings.default_relink_from.trim().is_empty()
        {
            let rf = settings.default_relink_from.clone();
            let rt = settings.default_relink_to.clone();
            self.relink_from
                .update(cx, |input, cx| input.set_value(rf, window, cx));
            self.relink_to
                .update(cx, |input, cx| input.set_value(rt, window, cx));
        }

        self.settings_view
            .update(cx, |v, cx| v.set_has_loaded_project(true, cx));

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

        // Preserve existing user decisions whenever the saved action is still valid for the new plan
        // (e.g. toggling a sequence checkbox no longer wipes customized mappings).
        let mut next_actions = BTreeMap::new();
        for fx in &new_plan.effects {
            let valid_items = effect_action_items(fx);
            let chosen = self
                .decisions
                .get(&fx.key)
                .filter(|prev| valid_items.iter().any(|item| &item.action == *prev))
                .cloned()
                .unwrap_or_else(|| fx.default_action.clone());
            next_actions.insert(fx.key.clone(), chosen);
        }

        self.decisions = next_actions;
        loaded.effect_plan = new_plan;
        self.rebuild_effect_selects(window, cx);
        cx.notify();
    }

    fn apply_bulk_effect_action(
        &mut self,
        mode: BulkEffectMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(loaded) = self.loaded.as_ref() else {
            return;
        };
        for fx in &loaded.effect_plan.effects {
            let action = match mode {
                BulkEffectMode::Recommended => fx.default_action.clone(),
                BulkEffectMode::MarkerOnly => MappingAction::PlaceMarkerOnly,
                BulkEffectMode::SkipAll => MappingAction::Skip,
            };
            if let Some(sel) = self.effect_selects.get(&fx.key) {
                sel.update(cx, |state, cx| {
                    state.set_selected_value(&action, window, cx);
                });
            }
            self.decisions.insert(fx.key.clone(), action);
        }
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
        let remembered_path = path.clone();
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend::load_and_inspect_project(&path, requested_target) })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.busy_label = None;
                this.remember_directory(&remembered_path, cx);
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
        let last_dir = self.current_settings(cx).last_dir_path();
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Open NLE Project File")
                .add_filter(
                    "NLE Projects",
                    &["prproj", "drp", "drt", "fcpbundle", "fcpxml", "fcpxmld"],
                )
                .add_filter("Premiere Pro Project", &["prproj"])
                .add_filter("DaVinci Resolve Project", &["drp", "drt"])
                .add_filter(
                    "Final Cut Pro Library / XML",
                    &["fcpbundle", "fcpxml", "fcpxmld"],
                );
            if let Some(dir) = last_dir {
                dialog = dialog.set_directory(dir);
            }
            let picked = dialog.pick_file().await;

            let Some(handle) = picked else {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    this.busy_label = None;
                    cx.notify();
                });
                return;
            };

            let path = handle.path().to_path_buf();
            let remembered_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { backend::load_and_inspect_project(&path, requested_target) })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.busy_label = None;
                this.remember_directory(&remembered_path, cx);
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
        let settings = self.current_settings(cx);
        let last_dir = settings.last_dir_path();
        let auto_reveal = settings.auto_reveal_output;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Save Converted Project")
                .set_file_name(&default_name)
                .add_filter(filter_name, &[ext]);
            if let Some(dir) = last_dir {
                dialog = dialog.set_directory(dir);
            }
            let saved = dialog.save_file().await;

            let Some(handle) = saved else {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    this.busy_label = None;
                    cx.notify();
                });
                return;
            };

            let output_path = handle.path().to_path_buf();
            let remembered_path = output_path.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    backend::run_conversion(&source_project, &output_path, options, &decisions)
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.busy_label = None;
                this.remember_directory(&remembered_path, cx);
                match result {
                    Ok(report) => {
                        if auto_reveal {
                            cx.reveal_path(Path::new(&report.output_path));
                        }
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

        let last_dir = self.current_settings(cx).last_dir_path();
        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Save Effect Mapping Preset")
                .set_file_name("nle-effect-preset.json")
                .add_filter("Effect Preset JSON", &["json"]);
            if let Some(dir) = last_dir {
                dialog = dialog.set_directory(dir);
            }
            let saved = dialog.save_file().await;

            let Some(handle) = saved else {
                return;
            };
            let path = handle.path().to_path_buf();
            let remembered_path = path.clone();
            let path_display = path.to_string_lossy().into_owned();
            let result = cx
                .background_executor()
                .spawn(async move { backend::save_effect_preset(&path, &preset) })
                .await;

            let _ = this.update(cx, |this, cx| {
                this.remember_directory(&remembered_path, cx);
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
        let last_dir = self.current_settings(cx).last_dir_path();
        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Load Effect Mapping Preset")
                .add_filter("Effect Preset JSON", &["json"]);
            if let Some(dir) = last_dir {
                dialog = dialog.set_directory(dir);
            }
            let picked = dialog.pick_file().await;

            let Some(handle) = picked else {
                return;
            };
            let path = handle.path().to_path_buf();
            let remembered_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { backend::load_effect_preset(&path) })
                .await;

            let _ = this.update_in(cx, |this, window, cx| {
                this.remember_directory(&remembered_path, cx);
                match result {
                    Ok(preset) => {
                        let total_detected = this.effect_selects.len();
                        let mut matched = 0usize;
                        for (k, v) in preset.decisions.actions {
                            if let Some(sel) = this.effect_selects.get(&k) {
                                matched += 1;
                                sel.update(cx, |state, cx| {
                                    state.set_selected_value(&v, window, cx);
                                });
                            }
                            this.decisions.insert(k, v);
                        }
                        this.status_toast = Some(format!(
                            "Loaded preset '{}' ({matched} of {total_detected} detected effects matched)",
                            preset.name
                        ));
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
        let show_hints = self.current_settings(cx).show_shortcut_hints;
        let in_settings = self.active_page == ActivePage::Settings;

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
                    .id("topbar-brand")
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.active_page == ActivePage::Settings {
                            this.active_page = ActivePage::Converter;
                            window.focus(&this.focus_handle, cx);
                            cx.notify();
                        } else {
                            open_about_window(cx);
                        }
                    }))
                    .child(
                        img(assets::APP_ICON_ASSET)
                            .w(px(36.))
                            .h(px(36.))
                            .rounded(px(8.)),
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
                    .gap(px(8.))
                    .when(!in_settings, |row| {
                        row.child(
                            Button::new("topbar-demo")
                                .label("Load Demo .prproj")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.on_load_demo(window, cx);
                                })),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    Button::new("topbar-open")
                                        .primary()
                                        .icon(IconName::FolderOpen)
                                        .label("Open Project...")
                                        .disabled(self.busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.on_open_file(window, cx);
                                        })),
                                )
                                .when(show_hints, |el| {
                                    el.when_some(
                                        shortcut_hint_badge(OPEN_PROJECT_KEYSTROKE),
                                        |el, kbd| el.child(kbd),
                                    )
                                }),
                        )
                    })
                    .when(in_settings, |row| {
                        row.child(
                            Button::new("topbar-back-converter")
                                .primary()
                                .icon(IconName::ArrowLeft)
                                .label("Back to Converter")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.active_page = ActivePage::Converter;
                                    window.focus(&this.focus_handle, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                Button::new("topbar-settings")
                                    .icon(IconName::Settings)
                                    .label("Settings")
                                    .selected(in_settings)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_settings(cx);
                                        if this.active_page == ActivePage::Converter {
                                            window.focus(&this.focus_handle, cx);
                                        }
                                    })),
                            )
                            .when(show_hints && !in_settings, |el| {
                                el.when_some(
                                    shortcut_hint_badge(SETTINGS_KEYSTROKE),
                                    |el, kbd| el.child(kbd),
                                )
                            }),
                    )
                    .child(
                        Button::new("topbar-about")
                            .icon(IconName::Info)
                            .tooltip("About Aagedal NLE Project Converter")
                            .on_click(|_, _, cx| {
                                open_about_window(cx);
                            }),
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
        let cross_plugins = InstalledPluginRegistry::system().grouped_plugins(2);
        let cross_plugin_pill = if cross_plugins.is_empty() {
            None
        } else {
            let names: Vec<&str> = cross_plugins.iter().take(3).map(|p| p.name.as_str()).collect();
            let suffix = if cross_plugins.len() > 3 {
                format!(" +{} more", cross_plugins.len() - 3)
            } else {
                String::new()
            };
            Some(theme::pill(
                format!(
                    "{} Cross-NLE Plugins Installed ({}{suffix})",
                    cross_plugins.len(),
                    names.join(", ")
                ),
                PillTone::Green,
            ))
        };

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
                            ))
                            .when_some(cross_plugin_pill, |row, p| row.child(p)),
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
                            .when(!rep.messages.is_empty(), |row| {
                                let log_text = format_conversion_log(rep);
                                row.child(
                                    Button::new("report-copy-log")
                                        .small()
                                        .label("Copy Log")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                log_text.clone(),
                                            ));
                                            this.status_toast = Some(
                                                "Copied conversion log to clipboard".to_string(),
                                            );
                                            cx.notify();
                                        })),
                                )
                            })
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
                        h_flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                Button::new("convert-btn")
                                    .primary()
                                    .label(convert_label)
                                    .disabled(self.busy || self.selected_seq_ids.is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.on_convert(window, cx);
                                    })),
                            )
                            .when(self.current_settings(cx).show_shortcut_hints, |el| {
                                el.when_some(
                                    shortcut_hint_badge(CONVERT_KEYSTROKE),
                                    |el, kbd| el.child(kbd),
                                )
                            }),
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

        let relink_from_val = self.relink_from.read(cx).value().to_string();
        let relink_has_input = !relink_from_val.trim().is_empty();
        let (relink_matched, relink_total) =
            backend::count_matching_relink_paths(&loaded.project, &relink_from_val);

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
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT))
                            .child("Media Path Relinker (Optional)"),
                    )
                    .when(relink_has_input, |hdr| {
                        hdr.child(
                            Button::new("relink-clear")
                                .small()
                                .label("Clear")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.relink_from.update(cx, |s, cx| s.set_value("", window, cx));
                                    this.relink_to.update(cx, |s, cx| s.set_value("", window, cx));
                                    cx.notify();
                                })),
                        )
                    }),
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
            )
            .when(relink_has_input, |panel| {
                let (color, label) = if relink_matched > 0 {
                    (
                        rgb(theme::GREEN_TEXT),
                        format!("{relink_matched} of {relink_total} media paths match this prefix"),
                    )
                } else {
                    (
                        rgb(theme::AMBER_TEXT),
                        format!("0 of {relink_total} media paths match this prefix"),
                    )
                };
                panel.child(div().text_xs().text_color(color).child(label))
            });

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

        let cross_plugins = InstalledPluginRegistry::system().grouped_plugins(2);
        let installed_plugin_banner = if cross_plugins.is_empty() {
            None
        } else {
            let names: Vec<&str> = cross_plugins.iter().take(4).map(|p| p.name.as_str()).collect();
            let more = if cross_plugins.len() > 4 {
                format!(" +{} more", cross_plugins.len() - 4)
            } else {
                String::new()
            };
            Some(format!(
                "Cross-NLE third-party plugins detected on this machine ({}): {}{more}",
                cross_plugins.len(),
                names.join(", ")
            ))
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
                                    .child("App-exclusive effects and third-party plugins are detected automatically — matching plugins installed for the target NLE are matched and attached directly where supported."),
                            )
                            .when_some(installed_plugin_banner, |col, txt| {
                                col.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme::CYAN_TEXT))
                                        .child(txt),
                                )
                            }),
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
                    .items_center()
                    .justify_between()
                    .flex_wrap()
                    .gap(px(8.))
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
                    .when(!plan.effects.is_empty(), |row| {
                        row.child(
                            h_flex()
                                .gap(px(6.))
                                .child(
                                    Button::new("fx-bulk-recommended")
                                        .small()
                                        .label("Recommended")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.apply_bulk_effect_action(
                                                BulkEffectMode::Recommended,
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                                .child(
                                    Button::new("fx-bulk-markers")
                                        .small()
                                        .label("All → Markers")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.apply_bulk_effect_action(
                                                BulkEffectMode::MarkerOnly,
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                                .child(
                                    Button::new("fx-bulk-skip")
                                        .small()
                                        .label("Skip All")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.apply_bulk_effect_action(
                                                BulkEffectMode::SkipAll,
                                                window,
                                                cx,
                                            );
                                        })),
                                ),
                        )
                    }),
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

        let main_body: AnyElement = match self.active_page {
            ActivePage::Settings => self.settings_view.clone().into_any_element(),
            ActivePage::Converter => match &self.loaded {
                None => self.render_hero(cx).into_any_element(),
                Some(loaded) => self.render_workspace(loaded, cx).into_any_element(),
            },
        };

        v_flex()
            .id("app-shell")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(theme::BG))
            .text_color(rgb(theme::TEXT))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.toggle_settings(cx);
                if this.active_page == ActivePage::Converter {
                    window.focus(&this.focus_handle, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenShortcuts, _, cx| {
                this.open_settings_section(SettingsSection::Shortcuts, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenProject, window, cx| {
                this.on_open_file(window, cx);
            }))
            .on_action(cx.listener(|this, _: &LoadDemoProject, window, cx| {
                this.on_load_demo(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ConvertProject, window, cx| {
                this.on_convert(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseProject, _, cx| {
                this.trigger_close_project(cx);
            }))
            .on_action(cx.listener(|this, _: &LoadEffectPreset, window, cx| {
                this.trigger_load_preset(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SaveEffectPreset, window, cx| {
                this.trigger_save_preset(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAllSequences, window, cx| {
                this.trigger_select_all_sequences(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DeselectAllSequences, window, cx| {
                this.trigger_deselect_all_sequences(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ApplyRecommendedEffects, window, cx| {
                this.trigger_apply_recommended_effects(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RevealOutput, _, cx| {
                this.trigger_reveal_output(cx);
            }))
            .on_action(cx.listener(|this, _: &CopyConversionLog, _, cx| {
                this.trigger_copy_conversion_log(cx);
            }))
            .on_action(cx.listener(|this, _: &DismissOverlay, window, cx| {
                if this.active_page == ActivePage::Settings {
                    this.active_page = ActivePage::Converter;
                    window.focus(&this.focus_handle, cx);
                    cx.notify();
                } else if this.error_msg.is_some() || this.status_toast.is_some() {
                    this.error_msg = None;
                    this.status_toast = None;
                    cx.notify();
                }
            }))
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_action(|_: &MinimizeWindow, window, _| window.minimize_window())
            .on_action(|_: &ZoomWindow, window, _| window.zoom_window())
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
                            .when(self.active_page == ActivePage::Converter, |col| {
                                col.children(banners)
                            })
                            .child(main_body),
                    ),
            )
    }
}
