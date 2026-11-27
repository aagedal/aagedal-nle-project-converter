use crate::catalog::{
    builtin_effect_catalog, find_catalog_rule_for_source, MappingFidelity, TargetEffectOption,
};
use crate::plugins::{InstalledPlugin, InstalledPluginRegistry};
use nle_core::{
    ConversionMessage, EffectCategory, Marker, MarkerScope, NleFormat, Project, Severity, TrackKind,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// User decision for one detected source effect type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MappingAction {
    /// Convert directly using the built-in native encoder (e.g. Motion, Lumetri DRX).
    NativeConvert,
    /// Map to a target-NLE equivalent (and optionally place a color-coded review marker on the timeline).
    MapToEquivalent {
        target_effect_id: String,
        target_display_name: String,
        add_review_marker: bool,
    },
    /// Drop the effect, but place a timeline marker with the original parameters so the editor can rebuild it.
    PlaceMarkerOnly,
    /// Skip this effect completely.
    Skip,
}

/// Summary of one distinct effect detected across the selected sequences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedEffectGroup {
    pub key: String,
    pub match_name: String,
    pub display_name: String,
    pub category: EffectCategory,
    pub media_kind: TrackKind,
    pub clip_count: usize,
    pub sample_clips: Vec<String>,
    pub has_keyframes: bool,
    pub fidelity: MappingFidelity,
    pub notes: String,
    pub available_targets: Vec<TargetEffectOption>,
    pub default_action: MappingAction,
}

/// Full effect analysis plan presented to the user before conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectMappingPlan {
    pub source_format: NleFormat,
    pub target_format: NleFormat,
    pub effects: Vec<DetectedEffectGroup>,
    pub native_auto_count: usize,
    pub needs_user_review_count: usize,
    pub unmapped_count: usize,
}

/// User choices keyed by `DetectedEffectGroup.key` (normalized match name).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserEffectDecisions {
    pub actions: BTreeMap<String, MappingAction>,
}

impl UserEffectDecisions {
    /// Parse either a full GUI-saved `EffectMappingPreset` JSON or a bare `UserEffectDecisions` JSON.
    pub fn from_json_str(raw: &str) -> Result<Self, serde_json::Error> {
        if let Ok(preset) = serde_json::from_str::<EffectMappingPreset>(raw) {
            return Ok(preset.decisions);
        }
        serde_json::from_str::<Self>(raw)
    }
}

/// Serializable studio preset for saving/loading user effect-mapping preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectMappingPreset {
    pub name: String,
    pub source_format: NleFormat,
    pub target_format: NleFormat,
    pub decisions: UserEffectDecisions,
}

#[derive(Debug, Clone, Default)]
pub struct EffectApplicationSummary {
    pub markers_injected: usize,
    pub messages: Vec<ConversionMessage>,
    pub disable_lumetri_native: bool,
    pub disable_motion_native: bool,
}

/// Inspect the selected sequences of a `Project` and produce an interactive `EffectMappingPlan`,
/// automatically cross-matching third-party plugins installed locally for the target NLE.
pub fn analyze_project_effects(
    project: &Project,
    selected_sequence_ids: &[String],
    target_format: NleFormat,
) -> EffectMappingPlan {
    analyze_project_effects_with_plugins(
        project,
        selected_sequence_ids,
        target_format,
        InstalledPluginRegistry::system(),
    )
}

/// Inspect the selected sequences of a `Project` using an explicit `InstalledPluginRegistry`.
pub fn analyze_project_effects_with_plugins(
    project: &Project,
    selected_sequence_ids: &[String],
    target_format: NleFormat,
    plugin_registry: &InstalledPluginRegistry,
) -> EffectMappingPlan {
    let catalog = builtin_effect_catalog();

    struct Accum {
        match_name: String,
        display_name: String,
        category: EffectCategory,
        media_kind: TrackKind,
        clip_count: usize,
        sample_clips: Vec<String>,
        has_keyframes: bool,
    }

    let mut groups: BTreeMap<String, Accum> = BTreeMap::new();

    for seq in &project.sequences {
        if !selected_sequence_ids.is_empty() && !selected_sequence_ids.contains(&seq.id) {
            continue;
        }
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            for clip in &track.clips {
                // Non-identity Motion
                if !clip.motion.is_identity() {
                    let key = "ae.adbe motion".to_string();
                    let acc = groups.entry(key).or_insert_with(|| Accum {
                        match_name: "AE.ADBE Motion".to_string(),
                        display_name: "Motion (Transform)".to_string(),
                        category: EffectCategory::Transform,
                        media_kind: TrackKind::Video,
                        clip_count: 0,
                        sample_clips: Vec::new(),
                        has_keyframes: false,
                    });
                    acc.clip_count += 1;
                    acc.has_keyframes |= clip.motion.has_keyframes;
                    if acc.sample_clips.len() < 4 && !acc.sample_clips.contains(&clip.name) {
                        acc.sample_clips.push(clip.name.clone());
                    }
                }

                // Lumetri Color
                if !clip.lumetri_effects.is_empty() {
                    let key = "ae.adbe lumetri".to_string();
                    let acc = groups.entry(key).or_insert_with(|| Accum {
                        match_name: "AE.ADBE Lumetri".to_string(),
                        display_name: "Lumetri Color".to_string(),
                        category: EffectCategory::Color,
                        media_kind: TrackKind::Video,
                        clip_count: 0,
                        sample_clips: Vec::new(),
                        has_keyframes: false,
                    });
                    acc.clip_count += clip.lumetri_effects.len();
                    acc.has_keyframes |= clip.lumetri_effects.iter().any(|l| l.is_animated);
                    if acc.sample_clips.len() < 4 && !acc.sample_clips.contains(&clip.name) {
                        acc.sample_clips.push(clip.name.clone());
                    }
                }

                // All other effects (app-exclusive, third-party, audio effects, non-default opacity/crop)
                for eff in &clip.effects {
                    if eff.is_intrinsic {
                        continue;
                    }
                    let key = eff.match_name.to_ascii_lowercase();
                    let acc = groups.entry(key).or_insert_with(|| Accum {
                        match_name: eff.match_name.clone(),
                        display_name: eff.display_name.clone(),
                        category: eff.category,
                        media_kind: eff.media_kind,
                        clip_count: 0,
                        sample_clips: Vec::new(),
                        has_keyframes: false,
                    });
                    acc.clip_count += 1;
                    acc.has_keyframes |= eff.is_animated;
                    if acc.sample_clips.len() < 4 && !acc.sample_clips.contains(&clip.name) {
                        acc.sample_clips.push(clip.name.clone());
                    }
                }
            }

            // Non-standard / third-party / app-exclusive transitions
            for tr in &track.transitions {
                if is_standard_native_transition(&tr.effect_id, &tr.name) {
                    continue;
                }
                let match_name = if tr.effect_id.is_empty() {
                    tr.name.clone()
                } else {
                    tr.effect_id.clone()
                };
                let key = match_name.to_ascii_lowercase();
                let acc = groups.entry(key).or_insert_with(|| Accum {
                    match_name,
                    display_name: tr.name.clone(),
                    category: EffectCategory::Transition,
                    media_kind: track.kind,
                    clip_count: 0,
                    sample_clips: Vec::new(),
                    has_keyframes: false,
                });
                acc.clip_count += 1;
                let label = format!("{} @ {}", track.name, tr.start_frame);
                if acc.sample_clips.len() < 4 && !acc.sample_clips.contains(&label) {
                    acc.sample_clips.push(label);
                }
            }
        }
    }

    let mut effects = Vec::new();
    let mut native_auto_count = 0usize;
    let mut needs_user_review_count = 0usize;
    let mut unmapped_count = 0usize;

    for (key, acc) in groups {
        let rule = find_catalog_rule_for_source(
            &catalog,
            project.source_format,
            &acc.match_name,
            &acc.display_name,
        );
        let (fidelity, notes, available_targets, default_action, category, display_name) =
            if let Some(rule) = rule {
                let targets: Vec<TargetEffectOption> = rule
                    .targets
                    .iter()
                    .filter(|t| t.target_nle == target_format)
                    .cloned()
                    .collect();

                let native_supported = matches!(
                    target_format,
                    NleFormat::DaVinciResolve | NleFormat::PremierePro
                ) || (target_format == NleFormat::FinalCutPro
                    && rule.category == EffectCategory::Transform);

                let default_action = match rule.fidelity {
                    MappingFidelity::NativeAutomatic if native_supported => {
                        MappingAction::NativeConvert
                    }
                    _ => {
                        if let Some(first) = targets.first() {
                            MappingAction::MapToEquivalent {
                                target_effect_id: first.id.clone(),
                                target_display_name: first.display_name.clone(),
                                add_review_marker: true,
                            }
                        } else {
                            MappingAction::PlaceMarkerOnly
                        }
                    }
                };

                let effective_fidelity = if rule.fidelity == MappingFidelity::NativeAutomatic
                    && !native_supported
                {
                    if targets.is_empty() {
                        MappingFidelity::Unmapped
                    } else {
                        MappingFidelity::AskUserSuggested
                    }
                } else {
                    rule.fidelity
                };

                (
                    effective_fidelity,
                    rule.notes.clone(),
                    targets,
                    default_action,
                    rule.category,
                    if acc.display_name.is_empty() {
                        rule.source_display_name.clone()
                    } else {
                        acc.display_name
                    },
                )
            } else {
                let installed_matches = plugin_registry.find_target_matches(
                    project.source_format,
                    target_format,
                    &acc.match_name,
                    &acc.display_name,
                    acc.media_kind,
                );
                if !installed_matches.is_empty() {
                    let targets: Vec<TargetEffectOption> = installed_matches
                        .iter()
                        .map(|p| installed_plugin_to_target_option(p, acc.category))
                        .collect();
                    let first = &targets[0];
                    let direct_note = if installed_matches[0]
                        .format
                        .supports_direct_attachment(target_format)
                    {
                        format!(
                            "Same third-party plugin detected locally for {} ({}) — will be attached directly onto matching clips with an annotated review marker for source parameters.",
                            target_format.short_name(),
                            installed_matches[0].native_id
                        )
                    } else {
                        format!(
                            "Same third-party plugin detected locally for {} ({}, {}) — mapped with target plugin ID and source parameters on timeline review markers.",
                            target_format.short_name(),
                            installed_matches[0].format.short_label(),
                            installed_matches[0].native_id
                        )
                    };
                    let default_action = MappingAction::MapToEquivalent {
                        target_effect_id: first.id.clone(),
                        target_display_name: first.display_name.clone(),
                        add_review_marker: true,
                    };
                    (
                        MappingFidelity::AskUserSuggested,
                        direct_note,
                        targets,
                        default_action,
                        acc.category,
                        acc.display_name,
                    )
                } else {
                    (
                        MappingFidelity::Unmapped,
                        "No built-in equivalent or locally installed matching third-party plugin found. You can place a review marker with parameters or skip it.".to_string(),
                        Vec::new(),
                        MappingAction::PlaceMarkerOnly,
                        acc.category,
                        acc.display_name,
                    )
                }
            };

        match fidelity {
            MappingFidelity::NativeAutomatic => native_auto_count += 1,
            MappingFidelity::AskUserSuggested => needs_user_review_count += 1,
            MappingFidelity::Unmapped => unmapped_count += 1,
        }

        effects.push(DetectedEffectGroup {
            key,
            match_name: acc.match_name,
            display_name,
            category,
            media_kind: acc.media_kind,
            clip_count: acc.clip_count,
            sample_clips: acc.sample_clips,
            has_keyframes: acc.has_keyframes,
            fidelity,
            notes,
            available_targets,
            default_action,
        });
    }

    // Sort: AskUserSuggested first, then Unmapped, then NativeAutomatic so the user sees actionable items at the top.
    effects.sort_by_key(|e| match e.fidelity {
        MappingFidelity::AskUserSuggested => 0,
        MappingFidelity::Unmapped => 1,
        MappingFidelity::NativeAutomatic => 2,
    });

    EffectMappingPlan {
        source_format: project.source_format,
        target_format,
        effects,
        native_auto_count,
        needs_user_review_count,
        unmapped_count,
    }
}

/// Apply user effect decisions onto a mutable `Project`, injecting review markers & recording audit messages.
pub fn apply_effect_decisions(
    project: &mut Project,
    plan: &EffectMappingPlan,
    decisions: &UserEffectDecisions,
    inject_markers: bool,
) -> EffectApplicationSummary {
    let mut summary = EffectApplicationSummary::default();

    let resolved: HashMap<String, MappingAction> = plan
        .effects
        .iter()
        .map(|g| {
            let action = decisions
                .actions
                .get(&g.key)
                .cloned()
                .unwrap_or_else(|| g.default_action.clone());
            (g.key.clone(), action)
        })
        .collect();

    let lumetri_action = resolved.get("ae.adbe lumetri").cloned();
    let motion_action = resolved.get("ae.adbe motion").cloned();
    if matches!(lumetri_action, Some(MappingAction::Skip | MappingAction::PlaceMarkerOnly)) {
        summary.disable_lumetri_native = true;
    }
    if matches!(motion_action, Some(MappingAction::Skip | MappingAction::PlaceMarkerOnly)) {
        summary.disable_motion_native = true;
    }

    for seq in &mut project.sequences {
        let mut new_markers: Vec<Marker> = Vec::new();
        for track in seq.video_tracks.iter_mut().chain(seq.audio_tracks.iter_mut()) {
            let track_prefix = if track.kind == TrackKind::Video {
                format!("V{}", track.index)
            } else {
                format!("A{}", track.index)
            };
            for clip in &mut track.clips {
                let clip_name = clip.name.clone();
                let clip_start = clip.start_frame;
                let clip_dur = clip.duration_frames().max(1);

                // Honor user decision to disable / marker-only native Motion transform
                if summary.disable_motion_native && !clip.motion.is_identity() {
                    if inject_markers && matches!(motion_action, Some(MappingAction::PlaceMarkerOnly))
                    {
                        let m = &clip.motion;
                        new_markers.push(Marker {
                            frame: clip_start,
                            duration_frames: clip_dur,
                            name: "Skipped FX: Motion (Transform)".to_string(),
                            comment: format!(
                                "[{track_prefix} · {clip_name}] Motion skipped per user choice | Scale={:.1}%, Pos=({:.3},{:.3}), Rot={:.1}°, Crop=({:.1}/{:.1}/{:.1}/{:.1}%)",
                                m.scale, m.position_x, m.position_y, m.rotation,
                                m.crop_left, m.crop_right, m.crop_top, m.crop_bottom
                            ),
                            color: "Yellow".to_string(),
                            scope: MarkerScope::Sequence,
                        });
                        summary.markers_injected += 1;
                    }
                    clip.motion = nle_core::MotionTransform::default();
                }

                // Honor user decision to place marker for skipped native Lumetri grades
                if summary.disable_lumetri_native
                    && !clip.lumetri_effects.is_empty()
                    && inject_markers
                    && matches!(lumetri_action, Some(MappingAction::PlaceMarkerOnly))
                {
                    new_markers.push(Marker {
                        frame: clip_start,
                        duration_frames: clip_dur,
                        name: "Skipped FX: Lumetri Color".to_string(),
                        comment: format!(
                            "[{track_prefix} · {clip_name}] {} Lumetri color grade(s) replaced with marker per user choice",
                            clip.lumetri_effects.len()
                        ),
                        color: "Yellow".to_string(),
                        scope: MarkerScope::Sequence,
                    });
                    summary.markers_injected += 1;
                }
                for eff in &mut clip.effects {
                    if eff.is_intrinsic {
                        continue;
                    }
                    let key = eff.match_name.to_ascii_lowercase();
                    let action = resolved
                        .get(&key)
                        .cloned()
                        .unwrap_or(MappingAction::PlaceMarkerOnly);

                    if matches!(action, MappingAction::Skip | MappingAction::PlaceMarkerOnly) {
                        eff.enabled = false;
                    }

                    let param_preview: String = eff
                        .parameters
                        .iter()
                        .take(5)
                        .filter(|p| !p.value.is_empty())
                        .map(|p| format!("{}={}", p.name, p.value))
                        .collect::<Vec<_>>()
                        .join(", ");

                    match action {
                        MappingAction::NativeConvert => {
                            summary.messages.push(ConversionMessage {
                                severity: Severity::Info,
                                code: "FX_NATIVE".into(),
                                sequence: Some(seq.name.clone()),
                                clip: Some(clip_name.clone()),
                                message: format!("Natively converted {}", eff.display_name),
                            });
                        }
                        MappingAction::MapToEquivalent {
                            target_effect_id,
                            target_display_name,
                            add_review_marker,
                        } => {
                            let resolved_plugin =
                                extract_target_plugin_id(&target_effect_id);
                            if let Some((plugin_kind, native_id)) = resolved_plugin {
                                // Rewrite effect match_name to the matched target plugin's native ID
                                // so Premiere / FCPXML writers attach it directly onto the clip!
                                eff.match_name = native_id.to_string();
                                eff.enabled = true;
                                if inject_markers && add_review_marker {
                                    let comment = if param_preview.is_empty() {
                                        format!(
                                            "[{track_prefix} · {}] Matched installed third-party plugin '{}' → {} ({})",
                                            clip_name, eff.display_name, target_display_name, native_id
                                        )
                                    } else {
                                        format!(
                                            "[{track_prefix} · {}] Matched installed third-party plugin '{}' → {} ({}) | Source params: {}",
                                            clip_name, eff.display_name, target_display_name, native_id, param_preview
                                        )
                                    };
                                    new_markers.push(Marker {
                                        frame: clip_start,
                                        duration_frames: clip_dur,
                                        name: format!("Plugin → {}", eff.display_name),
                                        comment,
                                        color: "Cyan".to_string(),
                                        scope: MarkerScope::Sequence,
                                    });
                                    summary.markers_injected += 1;
                                }
                                let attached_hint = match plugin_kind {
                                    "plugin.premiere" | "plugin.fcp" => "attached natively",
                                    _ => "matched locally",
                                };
                                summary.messages.push(ConversionMessage {
                                    severity: Severity::Info,
                                    code: "FX_PLUGIN_MATCHED".into(),
                                    sequence: Some(seq.name.clone()),
                                    clip: Some(clip_name.clone()),
                                    message: format!(
                                        "Third-party plugin '{}' → {} [{} — {attached_hint}]",
                                        eff.display_name, target_display_name, native_id
                                    ),
                                });
                            } else {
                                if inject_markers && add_review_marker {
                                    let comment = if param_preview.is_empty() {
                                        format!(
                                            "[{track_prefix} · {}] Mapped '{}' → {}",
                                            clip_name, eff.display_name, target_display_name
                                        )
                                    } else {
                                        format!(
                                            "[{track_prefix} · {}] Mapped '{}' → {} | Source params: {}",
                                            clip_name, eff.display_name, target_display_name, param_preview
                                        )
                                    };
                                    new_markers.push(Marker {
                                        frame: clip_start,
                                        duration_frames: clip_dur,
                                        name: format!("FX → {}", target_display_name),
                                        comment,
                                        color: "Purple".to_string(),
                                        scope: MarkerScope::Sequence,
                                    });
                                    summary.markers_injected += 1;
                                }
                                summary.messages.push(ConversionMessage {
                                    severity: Severity::Info,
                                    code: "FX_MAPPED".into(),
                                    sequence: Some(seq.name.clone()),
                                    clip: Some(clip_name.clone()),
                                    message: format!(
                                        "{} → {} (review marker {})",
                                        eff.display_name,
                                        target_display_name,
                                        if inject_markers && add_review_marker { "placed" } else { "disabled" }
                                    ),
                                });
                            }
                        }
                        MappingAction::PlaceMarkerOnly => {
                            if inject_markers {
                                let comment = if param_preview.is_empty() {
                                    format!(
                                        "[{track_prefix} · {}] Unmapped effect '{}' ({})",
                                        clip_name, eff.display_name, eff.match_name
                                    )
                                } else {
                                    format!(
                                        "[{track_prefix} · {}] Unmapped effect '{}' ({}) | Params: {}",
                                        clip_name, eff.display_name, eff.match_name, param_preview
                                    )
                                };
                                new_markers.push(Marker {
                                    frame: clip_start,
                                    duration_frames: clip_dur,
                                    name: format!("Unmapped FX: {}", eff.display_name),
                                    comment,
                                    color: "Red".to_string(),
                                    scope: MarkerScope::Sequence,
                                });
                                summary.markers_injected += 1;
                            }
                            summary.messages.push(ConversionMessage {
                                severity: Severity::Warning,
                                code: "FX_UNMAPPED".into(),
                                sequence: Some(seq.name.clone()),
                                clip: Some(clip_name.clone()),
                                message: format!(
                                    "Unmapped effect '{}' ({}) flagged via timeline marker",
                                    eff.display_name, eff.match_name
                                ),
                            });
                        }
                        MappingAction::Skip => {
                            summary.messages.push(ConversionMessage {
                                severity: Severity::Info,
                                code: "FX_SKIPPED".into(),
                                sequence: Some(seq.name.clone()),
                                clip: Some(clip_name.clone()),
                                message: format!("Skipped effect '{}' per user choice", eff.display_name),
                            });
                        }
                    }
                }
            }

            // Apply decisions to non-standard / third-party / app-exclusive transitions
            for tr in &track.transitions {
                if is_standard_native_transition(&tr.effect_id, &tr.name) {
                    continue;
                }
                let match_name = if tr.effect_id.is_empty() {
                    tr.name.as_str()
                } else {
                    tr.effect_id.as_str()
                };
                let key = match_name.to_ascii_lowercase();
                let action = resolved
                    .get(&key)
                    .cloned()
                    .unwrap_or(MappingAction::PlaceMarkerOnly);
                let dur = (tr.end_frame - tr.start_frame).max(1);

                match action {
                    MappingAction::NativeConvert => {}
                    MappingAction::MapToEquivalent {
                        target_display_name,
                        add_review_marker,
                        ..
                    } => {
                        if inject_markers && add_review_marker {
                            new_markers.push(Marker {
                                frame: tr.start_frame,
                                duration_frames: dur,
                                name: format!("Transition → {}", target_display_name),
                                comment: format!(
                                    "[{track_prefix}] Mapped transition '{}' ({}) → {}",
                                    tr.name, match_name, target_display_name
                                ),
                                color: "Purple".to_string(),
                                scope: MarkerScope::Sequence,
                            });
                            summary.markers_injected += 1;
                        }
                        summary.messages.push(ConversionMessage {
                            severity: Severity::Info,
                            code: "TR_MAPPED".into(),
                            sequence: Some(seq.name.clone()),
                            clip: None,
                            message: format!(
                                "Transition '{}' → {} (review marker {})",
                                tr.name,
                                target_display_name,
                                if inject_markers && add_review_marker { "placed" } else { "disabled" }
                            ),
                        });
                    }
                    MappingAction::PlaceMarkerOnly => {
                        if inject_markers {
                            new_markers.push(Marker {
                                frame: tr.start_frame,
                                duration_frames: dur,
                                name: format!("Unmapped Transition: {}", tr.name),
                                comment: format!(
                                    "[{track_prefix}] Unmapped transition '{}' ({})",
                                    tr.name, match_name
                                ),
                                color: "Red".to_string(),
                                scope: MarkerScope::Sequence,
                            });
                            summary.markers_injected += 1;
                        }
                        summary.messages.push(ConversionMessage {
                            severity: Severity::Warning,
                            code: "TR_UNMAPPED".into(),
                            sequence: Some(seq.name.clone()),
                            clip: None,
                            message: format!(
                                "Unmapped transition '{}' ({}) flagged via timeline marker",
                                tr.name, match_name
                            ),
                        });
                    }
                    MappingAction::Skip => {
                        summary.messages.push(ConversionMessage {
                            severity: Severity::Info,
                            code: "TR_SKIPPED".into(),
                            sequence: Some(seq.name.clone()),
                            clip: None,
                            message: format!("Skipped transition '{}' per user choice", tr.name),
                        });
                    }
                }
            }
        }
        seq.markers.extend(new_markers);
    }

    summary
}

fn installed_plugin_to_target_option(
    plugin: &InstalledPlugin,
    category: EffectCategory,
) -> TargetEffectOption {
    let notes = if plugin.format.supports_direct_attachment(plugin.target_nle) {
        format!(
            "Installed at {} (ID: {}). Attached directly onto the clip in the exported project.",
            plugin.path, plugin.native_id
        )
    } else {
        format!(
            "Installed at {} (ID: {}). Flagged with exact target plugin ID and source parameters.",
            plugin.path, plugin.native_id
        )
    };
    TargetEffectOption {
        id: plugin.target_option_id(),
        display_name: plugin.target_display_label(),
        target_nle: plugin.target_nle,
        category,
        studio_only: false,
        notes,
    }
}

fn extract_target_plugin_id(target_effect_id: &str) -> Option<(&str, &str)> {
    for prefix in [
        "plugin.premiere",
        "plugin.fcp",
        "plugin.resolve.ofx",
        "plugin.audio.au",
        "plugin.audio.vst3",
        "plugin.other",
    ] {
        if let Some(rest) = target_effect_id.strip_prefix(&format!("{prefix}:")) {
            if !rest.is_empty() {
                return Some((prefix, rest));
            }
        }
    }
    None
}

/// Returns true for standard cross-NLE video dissolves and audio crossfades
/// that translate natively without needing user effect review.
pub fn is_standard_native_transition(effect_id: &str, name: &str) -> bool {
    let id = effect_id.to_ascii_lowercase();
    let nm = name.to_ascii_lowercase();
    const STANDARD: &[&str] = &[
        "cross dissolve",
        "film dissolve",
        "additive dissolve",
        "non-additive dissolve",
        "non additive dissolve",
        "dip to black",
        "dip to white",
        "dip to color",
        "fade to color",
        "fade in",
        "fade out",
        "audio fade",
        "constant power",
        "constant gain",
        "exponential fade",
        "custom fade",
        "cross fade",
    ];
    STANDARD.iter().any(|s| id.contains(s) || nm.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::*;

    #[test]
    fn analyzes_and_maps_app_exclusive_effects() {
        let mut project = Project::default();
        project.sequences.push(Sequence {
            id: "seq-1".into(),
            name: "Main Edit".into(),
            kind: SequenceKind::Timeline,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 250,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: vec![Track {
                index: 1,
                kind: TrackKind::Video,
                name: "V1".into(),
                enabled: true,
                locked: false,
                muted: false,
                solo: false,
                audio_channels: 0,
                output_bus_id: None,
                clips: vec![Clip {
                    id: "c1".into(),
                    name: "A001.mov".into(),
                    media_id: "m1".into(),
                    media_path: "/media/A001.mov".into(),
                    media_kind: MediaKind::Video,
                    source_kind: ClipSourceKind::Media,
                    active_angle: None,
                    start_frame: 0,
                    end_frame: 100,
                    source_in_frame: 0,
                    source_out_frame: 100,
                    source_in_ticks: None,
                    source_out_ticks: None,
                    source_fps: Rational::FPS_25,
                    source_width: 1920,
                    source_height: 1080,
                    speed_percent: 100.0,
                    reversed: false,
                    speed_keyframes: vec![],
                    retime_interpolation: nle_core::RetimeInterpolation::Default,
                    enabled: true,
                    audio_channel: 1,
                    link_group_id: None,
                    label_name: None,
                    label_preset_index: None,
                    motion: MotionTransform::default(),
                    opacity: 100.0,
                    blend_mode: BlendMode::Normal,
                    lumetri_effects: vec![LumetriData {
                        order: 0,
                        display_name: "Lumetri Color".into(),
                        exposure: Some(0.5),
                        ..Default::default()
                    }],
                    effects: vec![EffectInstance {
                        id: "fx1".into(),
                        match_name: "AE.ADBE SubspaceStabilizer".into(),
                        display_name: "Warp Stabilizer".into(),
                        category: EffectCategory::Stabilization,
                        media_kind: TrackKind::Video,
                        enabled: true,
                        is_intrinsic: false,
                        is_animated: false,
                        parameters: vec![EffectParameter {
                            id: "1".into(),
                            name: "Smoothness".into(),
                            value: "50".into(),
                            is_animated: false,
                        }],
                    }],
                    markers: vec![],
                }],
                transitions: vec![],
                markers: vec![],
            }],
            audio_tracks: vec![],
            audio_buses: vec![],
            markers: vec![],
            metadata: BTreeMap::new(),
        });

        let plan = analyze_project_effects(&project, &[], NleFormat::DaVinciResolve);
        assert_eq!(plan.native_auto_count, 1);
        assert_eq!(plan.needs_user_review_count, 1);
        assert_eq!(plan.effects[0].display_name, "Warp Stabilizer");

        let summary = apply_effect_decisions(&mut project, &plan, &UserEffectDecisions::default(), true);
        assert_eq!(summary.markers_injected, 1);
        assert_eq!(project.sequences[0].markers.len(), 1);
        assert!(project.sequences[0].markers[0].name.contains("Stabilization"));
    }

    #[test]
    fn parses_both_preset_and_raw_decisions_and_honors_motion_disable() {
        let preset = EffectMappingPreset {
            name: "Studio".into(),
            source_format: NleFormat::PremierePro,
            target_format: NleFormat::DaVinciResolve,
            decisions: UserEffectDecisions {
                actions: BTreeMap::from([(
                    "ae.adbe motion".into(),
                    MappingAction::PlaceMarkerOnly,
                )]),
            },
        };
        let preset_json = serde_json::to_string(&preset).unwrap();
        let parsed_from_preset = UserEffectDecisions::from_json_str(&preset_json).unwrap();
        assert_eq!(
            parsed_from_preset.actions.get("ae.adbe motion"),
            Some(&MappingAction::PlaceMarkerOnly)
        );

        let raw_json = serde_json::to_string(&preset.decisions).unwrap();
        let parsed_from_raw = UserEffectDecisions::from_json_str(&raw_json).unwrap();
        assert_eq!(
            parsed_from_raw.actions.get("ae.adbe motion"),
            Some(&MappingAction::PlaceMarkerOnly)
        );

        let mut project = Project::default();
        project.sequences.push(Sequence {
            id: "s1".into(),
            name: "Seq".into(),
            kind: SequenceKind::Timeline,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 100,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: vec![Track {
                index: 1,
                kind: TrackKind::Video,
                name: "V1".into(),
                enabled: true,
                locked: false,
                muted: false,
                solo: false,
                audio_channels: 0,
                output_bus_id: None,
                clips: vec![Clip {
                    id: "c1".into(),
                    name: "Shot.mov".into(),
                    media_id: "m1".into(),
                    media_path: "/Media/Shot.mov".into(),
                    media_kind: MediaKind::Video,
                    source_kind: ClipSourceKind::Media,
                    active_angle: None,
                    start_frame: 0,
                    end_frame: 50,
                    source_in_frame: 0,
                    source_out_frame: 50,
                    source_in_ticks: None,
                    source_out_ticks: None,
                    source_fps: Rational::FPS_25,
                    source_width: 1920,
                    source_height: 1080,
                    speed_percent: 100.0,
                    reversed: false,
                    speed_keyframes: vec![],
                    retime_interpolation: RetimeInterpolation::Default,
                    enabled: true,
                    audio_channel: 0,
                    link_group_id: None,
                    label_name: None,
                    label_preset_index: None,
                    motion: MotionTransform {
                        scale: 120.0,
                        ..Default::default()
                    },
                    opacity: 100.0,
                    blend_mode: BlendMode::Normal,
                    lumetri_effects: vec![],
                    effects: vec![],
                    markers: vec![],
                }],
                transitions: vec![],
                markers: vec![],
            }],
            audio_tracks: vec![],
            audio_buses: vec![],
            markers: vec![],
            metadata: BTreeMap::new(),
        });

        let plan = analyze_project_effects(&project, &[], NleFormat::DaVinciResolve);
        let summary = apply_effect_decisions(&mut project, &plan, &parsed_from_preset, true);
        assert!(summary.disable_motion_native);
        assert!(project.sequences[0].video_tracks[0].clips[0].motion.is_identity());
        assert_eq!(summary.markers_injected, 1);
        assert!(project.sequences[0].markers[0].name.contains("Motion"));
    }
}
