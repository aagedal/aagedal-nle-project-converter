use crate::catalog::{builtin_effect_catalog, find_catalog_rule, MappingFidelity, TargetEffectOption};
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

/// Inspect the selected sequences of a `Project` and produce an interactive `EffectMappingPlan`.
pub fn analyze_project_effects(
    project: &Project,
    selected_sequence_ids: &[String],
    target_format: NleFormat,
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
        }
    }

    let mut effects = Vec::new();
    let mut native_auto_count = 0usize;
    let mut needs_user_review_count = 0usize;
    let mut unmapped_count = 0usize;

    for (key, acc) in groups {
        let rule = find_catalog_rule(&catalog, &acc.match_name, &acc.display_name);
        let (fidelity, notes, available_targets, default_action, category, display_name) =
            if let Some(rule) = rule {
                let targets: Vec<TargetEffectOption> = rule
                    .targets
                    .iter()
                    .filter(|t| t.target_nle == target_format)
                    .cloned()
                    .collect();

                let default_action = match rule.fidelity {
                    MappingFidelity::NativeAutomatic
                        if target_format == NleFormat::DaVinciResolve =>
                    {
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
                    && target_format != NleFormat::DaVinciResolve
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
                (
                    MappingFidelity::Unmapped,
                    "No built-in equivalent found (possibly a third-party OFX/VST/AU plugin or app-exclusive effect). You can place a review marker with parameters or skip it.".to_string(),
                    Vec::new(),
                    MappingAction::PlaceMarkerOnly,
                    acc.category,
                    acc.display_name,
                )
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

    if matches!(resolved.get("ae.adbe lumetri"), Some(MappingAction::Skip | MappingAction::PlaceMarkerOnly)) {
        summary.disable_lumetri_native = true;
    }
    if matches!(resolved.get("ae.adbe motion"), Some(MappingAction::Skip | MappingAction::PlaceMarkerOnly)) {
        summary.disable_motion_native = true;
    }

    for seq in &mut project.sequences {
        let mut new_markers: Vec<Marker> = Vec::new();
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            let track_prefix = if track.kind == TrackKind::Video {
                format!("V{}", track.index)
            } else {
                format!("A{}", track.index)
            };
            for clip in &track.clips {
                for eff in &clip.effects {
                    if eff.is_intrinsic {
                        continue;
                    }
                    let key = eff.match_name.to_ascii_lowercase();
                    let action = resolved
                        .get(&key)
                        .cloned()
                        .unwrap_or(MappingAction::PlaceMarkerOnly);

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
                                clip: Some(clip.name.clone()),
                                message: format!("Natively converted {}", eff.display_name),
                            });
                        }
                        MappingAction::MapToEquivalent {
                            target_display_name,
                            add_review_marker,
                            ..
                        } => {
                            if inject_markers && add_review_marker {
                                let comment = if param_preview.is_empty() {
                                    format!(
                                        "[{track_prefix} · {}] Mapped '{}' → {}",
                                        clip.name, eff.display_name, target_display_name
                                    )
                                } else {
                                    format!(
                                        "[{track_prefix} · {}] Mapped '{}' → {} | Source params: {}",
                                        clip.name, eff.display_name, target_display_name, param_preview
                                    )
                                };
                                new_markers.push(Marker {
                                    frame: clip.start_frame,
                                    duration_frames: clip.duration_frames().max(1),
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
                                clip: Some(clip.name.clone()),
                                message: format!(
                                    "{} → {} (review marker {})",
                                    eff.display_name,
                                    target_display_name,
                                    if inject_markers && add_review_marker { "placed" } else { "disabled" }
                                ),
                            });
                        }
                        MappingAction::PlaceMarkerOnly => {
                            if inject_markers {
                                let comment = if param_preview.is_empty() {
                                    format!(
                                        "[{track_prefix} · {}] Unmapped effect '{}' ({})",
                                        clip.name, eff.display_name, eff.match_name
                                    )
                                } else {
                                    format!(
                                        "[{track_prefix} · {}] Unmapped effect '{}' ({}) | Params: {}",
                                        clip.name, eff.display_name, eff.match_name, param_preview
                                    )
                                };
                                new_markers.push(Marker {
                                    frame: clip.start_frame,
                                    duration_frames: clip.duration_frames().max(1),
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
                                clip: Some(clip.name.clone()),
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
                                clip: Some(clip.name.clone()),
                                message: format!("Skipped effect '{}' per user choice", eff.display_name),
                            });
                        }
                    }
                }
            }
        }
        seq.markers.extend(new_markers);
    }

    summary
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
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 250,
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
                clips: vec![Clip {
                    id: "c1".into(),
                    name: "A001.mov".into(),
                    media_id: "m1".into(),
                    media_path: "/media/A001.mov".into(),
                    media_kind: MediaKind::Video,
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
                    enabled: true,
                    audio_channel: 1,
                    link_group_id: None,
                    label_name: None,
                    label_preset_index: None,
                    motion: MotionTransform::default(),
                    opacity: 100.0,
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
}
