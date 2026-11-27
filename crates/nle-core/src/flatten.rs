use crate::model::{
    Clip, ClipSourceKind, Project, Sequence, SequenceKind, Track, TrackKind,
};
use crate::report::{ConversionMessage, Severity};
use crate::time::Rational;
use std::collections::{HashMap, HashSet};

const MAX_FLATTEN_DEPTH: usize = 8;

#[derive(Debug, Clone, Default)]
pub struct FlattenSummary {
    pub clips_flattened: usize,
    pub multicam_clips_flattened: usize,
    pub compound_clips_flattened: usize,
    pub nested_clips_flattened: usize,
    pub helper_sequences_removed: usize,
    pub messages: Vec<ConversionMessage>,
}

/// Flatten nested sequences, compound clips, and multicam cuts across all sequences in `project`
/// into their underlying source media clips.
///
/// - **Multicam clips**: selects the active camera/angle track matching `clip.active_angle`
///   (by track name or 1-based `"Camera N"` index, falling back to track 0) and slices inner
///   clips covering `[clip.source_in_frame, clip.source_out_frame)`.
/// - **Compound clips & nested sequences**: expands all inner tracks of the matching `TrackKind`.
///   Inner track 0 replaces the clip on the parent track; higher layers (`V2+` / `A2+`) are placed
///   onto non-overlapping parent tracks (creating additional tracks as needed).
/// - Nested containers inside containers are expanded recursively up to `MAX_FLATTEN_DEPTH`.
/// - Helper `CompoundClip` / `MulticamClip` sequences that are no longer referenced after
///   flattening are removed (unless the project contains only helper sequences).
pub fn flatten_nested_and_multicam_clips(project: &mut Project) -> FlattenSummary {
    let mut summary = FlattenSummary::default();
    if project.sequences.is_empty() {
        return summary;
    }

    // Snapshot original sequences for immutable lookup while mutating target sequences.
    let seq_snapshot: HashMap<String, Sequence> = project
        .sequences
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();

    let mut flattened_target_ids: HashSet<String> = HashSet::new();
    let mut next_id_counter = 1usize;

    for seq in &mut project.sequences {
        // Only flatten regular timelines (or helper sequences if no regular timelines exist).
        let seq_name = seq.name.clone();
        let seq_fps = seq.frame_rate;

        flatten_track_set(
            &mut seq.video_tracks,
            TrackKind::Video,
            &seq.id,
            &seq_name,
            seq_fps,
            &seq_snapshot,
            &mut flattened_target_ids,
            &mut next_id_counter,
            &mut summary,
        );
        flatten_track_set(
            &mut seq.audio_tracks,
            TrackKind::Audio,
            &seq.id,
            &seq_name,
            seq_fps,
            &seq_snapshot,
            &mut flattened_target_ids,
            &mut next_id_counter,
            &mut summary,
        );
        seq.duration_frames = seq.computed_duration_frames();
    }

    // Drop helper CompoundClip / MulticamClip sequences that were flattened away,
    // provided at least one regular Timeline remains.
    let has_timeline = project
        .sequences
        .iter()
        .any(|s| s.kind == SequenceKind::Timeline);
    if has_timeline && !flattened_target_ids.is_empty() {
        // Check if anything still references a helper sequence.
        let mut still_referenced: HashSet<String> = HashSet::new();
        for seq in &project.sequences {
            if seq.kind == SequenceKind::Timeline {
                for tr in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
                    for clip in &tr.clips {
                        if clip.source_kind != ClipSourceKind::Media && !clip.media_id.is_empty() {
                            still_referenced.insert(clip.media_id.clone());
                        }
                    }
                }
            }
        }
        let before = project.sequences.len();
        project.sequences.retain(|s| {
            !(matches!(s.kind, SequenceKind::CompoundClip | SequenceKind::MulticamClip)
                && flattened_target_ids.contains(&s.id)
                && !still_referenced.contains(&s.id))
        });
        summary.helper_sequences_removed = before.saturating_sub(project.sequences.len());
    }

    if summary.clips_flattened > 0 {
        summary.messages.push(ConversionMessage {
            severity: Severity::Info,
            code: "FLATTENED_NESTED".into(),
            sequence: None,
            clip: None,
            message: format!(
                "Flattened {} nested/compound/multicam clip(s) ({} multicam, {} compound, {} nested) to source media.",
                summary.clips_flattened,
                summary.multicam_clips_flattened,
                summary.compound_clips_flattened,
                summary.nested_clips_flattened
            ),
        });
    }

    summary
}

#[allow(clippy::too_many_arguments)]
fn flatten_track_set(
    tracks: &mut Vec<Track>,
    kind: TrackKind,
    parent_seq_id: &str,
    parent_seq_name: &str,
    parent_fps: Rational,
    seq_snapshot: &HashMap<String, Sequence>,
    flattened_target_ids: &mut HashSet<String>,
    next_id_counter: &mut usize,
    summary: &mut FlattenSummary,
) {
    // Collect overflow clips destined for upper tracks: (preferred_track_idx, clip, preferred_channels)
    let mut overflow: Vec<(usize, Clip, u32)> = Vec::new();

    for (t_idx, track) in tracks.iter_mut().enumerate() {
        let orig_clips = std::mem::take(&mut track.clips);
        let mut new_clips = Vec::with_capacity(orig_clips.len());

        for clip in orig_clips {
            if clip.source_kind == ClipSourceKind::Media {
                new_clips.push(clip);
                continue;
            }

            let mut visited = HashSet::new();
            visited.insert(parent_seq_id.to_string());

            match expand_container_clip(
                &clip,
                kind,
                parent_fps,
                seq_snapshot,
                &mut visited,
                0,
                next_id_counter,
            ) {
                Ok(layers) => {
                    flattened_target_ids.insert(clip.media_id.clone());
                    summary.clips_flattened += 1;
                    match clip.source_kind {
                        ClipSourceKind::MulticamClip => summary.multicam_clips_flattened += 1,
                        ClipSourceKind::CompoundClip => summary.compound_clips_flattened += 1,
                        ClipSourceKind::NestedSequence => summary.nested_clips_flattened += 1,
                        ClipSourceKind::Media => {}
                    }
                    for (layer_offset, expanded_clip, ch) in layers {
                        if layer_offset == 0 {
                            new_clips.push(expanded_clip);
                        } else {
                            overflow.push((t_idx + layer_offset, expanded_clip, ch));
                        }
                    }
                }
                Err(reason) => {
                    summary.messages.push(ConversionMessage {
                        severity: Severity::Warning,
                        code: "FLATTEN_UNRESOLVED".into(),
                        sequence: Some(parent_seq_name.to_string()),
                        clip: Some(clip.name.clone()),
                        message: format!(
                            "Could not flatten {} '{}': {reason}",
                            source_kind_label(clip.source_kind),
                            clip.name
                        ),
                    });
                    new_clips.push(clip);
                }
            }
        }

        new_clips.sort_by_key(|c| c.start_frame);
        track.clips = new_clips;
    }

    // Place overflow clips onto non-overlapping tracks, adding tracks if needed.
    for (pref_idx, clip, pref_ch) in overflow {
        let mut idx = pref_idx;
        loop {
            while tracks.len() <= idx {
                let n = tracks.len() + 1;
                tracks.push(Track {
                    index: n,
                    kind,
                    name: if kind == TrackKind::Video {
                        format!("V{n}")
                    } else {
                        format!("A{n}")
                    },
                    enabled: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    audio_channels: if kind == TrackKind::Audio {
                        pref_ch.max(1)
                    } else {
                        0
                    },
                    output_bus_id: None,
                    clips: Vec::new(),
                    transitions: Vec::new(),
                    markers: Vec::new(),
                });
            }
            let overlaps = tracks[idx]
                .clips
                .iter()
                .any(|c| c.start_frame < clip.end_frame && clip.start_frame < c.end_frame);
            if !overlaps {
                tracks[idx].clips.push(clip);
                tracks[idx].clips.sort_by_key(|c| c.start_frame);
                break;
            }
            idx += 1;
        }
    }
}

/// Expand a single nested/compound/multicam clip into `(layer_offset, flat_clip, track_channels)`.
fn expand_container_clip(
    outer: &Clip,
    kind: TrackKind,
    parent_fps: Rational,
    seq_snapshot: &HashMap<String, Sequence>,
    visited: &mut HashSet<String>,
    depth: usize,
    next_id: &mut usize,
) -> Result<Vec<(usize, Clip, u32)>, String> {
    if depth >= MAX_FLATTEN_DEPTH {
        return Err(format!("exceeded maximum nesting depth ({MAX_FLATTEN_DEPTH})"));
    }
    if outer.media_id.is_empty() {
        return Err("missing target sequence reference".into());
    }
    let Some(target_seq) = seq_snapshot.get(&outer.media_id) else {
        return Err(format!("referenced sequence '{}' not found", outer.media_id));
    };
    if !visited.insert(target_seq.id.clone()) {
        return Err(format!("cyclic sequence reference '{}'", target_seq.name));
    }

    let inner_tracks: &[Track] = if kind == TrackKind::Video {
        &target_seq.video_tracks
    } else {
        &target_seq.audio_tracks
    };

    if inner_tracks.is_empty() {
        visited.remove(&target_seq.id);
        return Ok(Vec::new());
    }

    // Select which inner tracks to expand:
    // - Multicam: only the active angle track.
    // - Compound / Nested: all inner tracks of this kind.
    let selected_tracks: Vec<(usize, &Track)> =
        if outer.source_kind == ClipSourceKind::MulticamClip
            || target_seq.kind == SequenceKind::MulticamClip
        {
            let angle_idx = resolve_multicam_angle_index(outer.active_angle.as_deref(), inner_tracks);
            vec![(0, &inner_tracks[angle_idx])]
        } else {
            inner_tracks.iter().enumerate().collect()
        };

    let parent_fps_f = parent_fps.as_f64().max(1.0);
    let inner_fps_f = target_seq.frame_rate.as_f64().max(1.0);
    let fps_ratio = inner_fps_f / parent_fps_f;

    let outer_dur_parent = outer.duration_frames().max(1);
    let outer_abs_speed = if outer.speed_percent.is_finite() && outer.speed_percent.abs() > 1e-4 {
        outer.speed_percent.abs() / 100.0
    } else {
        1.0
    };
    let is_freeze = outer.speed_percent.is_finite() && outer.speed_percent.abs() < 1e-4;

    // Visible window inside `target_seq` in target_seq's frame coordinates (0-based relative to target_seq start).
    // `outer.source_in_frame` is stored in parent/source frame units.
    let win_start_inner = (outer.source_in_frame.max(0) as f64) * fps_ratio;
    let win_span_inner = if is_freeze {
        1.0
    } else if outer.source_out_frame > outer.source_in_frame {
        ((outer.source_out_frame - outer.source_in_frame) as f64) * fps_ratio
    } else {
        (outer_dur_parent as f64) * outer_abs_speed * fps_ratio
    };
    let win_end_inner = win_start_inner + win_span_inner.max(1.0);

    let mut out_layers: Vec<(usize, Clip, u32)> = Vec::new();

    for (layer_idx, inner_track) in selected_tracks {
        if !inner_track.enabled || inner_track.muted {
            continue;
        }
        let ch = inner_track.audio_channels;

        for inner in &inner_track.clips {
            if !inner.enabled {
                continue;
            }
            let ic_start = inner.start_frame as f64;
            let ic_end = inner.end_frame as f64;
            if ic_end <= win_start_inner || ic_start >= win_end_inner {
                continue;
            }

            let overlap_start = ic_start.max(win_start_inner);
            let overlap_end = ic_end.min(win_end_inner);
            if overlap_end - overlap_start < 0.5 {
                continue;
            }

            // Map overlap back to parent timeline frame coordinates.
            let (parent_clip_start, parent_clip_end) = if is_freeze {
                (outer.start_frame, outer.end_frame)
            } else if outer.reversed {
                let rel_from_end_start = (win_end_inner - overlap_end) / (outer_abs_speed * fps_ratio);
                let rel_from_end_end = (win_end_inner - overlap_start) / (outer_abs_speed * fps_ratio);
                (
                    outer.start_frame + rel_from_end_start.round() as i64,
                    outer.start_frame + rel_from_end_end.round() as i64,
                )
            } else {
                let rel_start = (overlap_start - win_start_inner) / (outer_abs_speed * fps_ratio);
                let rel_end = (overlap_end - win_start_inner) / (outer_abs_speed * fps_ratio);
                (
                    outer.start_frame + rel_start.round() as i64,
                    outer.start_frame + rel_end.round() as i64,
                )
            };

            let clipped_start = parent_clip_start.clamp(outer.start_frame, outer.end_frame);
            let clipped_end = parent_clip_end.clamp(clipped_start, outer.end_frame);
            if clipped_end <= clipped_start {
                continue;
            }

            // Trim inner clip's source_in / source_out for the sliced portion.
            let trim_head_inner = (overlap_start - ic_start).max(0.0);
            let trim_tail_inner = (ic_end - overlap_end).max(0.0);
            let inner_speed_abs = if inner.speed_percent.is_finite() && inner.speed_percent.abs() > 1e-4 {
                inner.speed_percent.abs() / 100.0
            } else {
                1.0
            };
            let src_fps_ratio = inner.source_fps.as_f64().max(1.0) / inner_fps_f;

            let (new_src_in, new_src_out) = if inner.reversed {
                let trim_src_from_out = (trim_head_inner * inner_speed_abs * src_fps_ratio).round() as i64;
                let trim_src_from_in = (trim_tail_inner * inner_speed_abs * src_fps_ratio).round() as i64;
                let s_in = (inner.source_in_frame + trim_src_from_in).max(0);
                let s_out = (inner.source_out_frame - trim_src_from_out).max(s_in + 1);
                (s_in, s_out)
            } else {
                let trim_src_head = (trim_head_inner * inner_speed_abs * src_fps_ratio).round() as i64;
                let span_src = ((overlap_end - overlap_start) * inner_speed_abs * src_fps_ratio)
                    .round()
                    .max(1.0) as i64;
                let s_in = (inner.source_in_frame + trim_src_head).max(0);
                (s_in, s_in + span_src)
            };

            let composed_reversed = inner.reversed ^ outer.reversed;
            let composed_speed_mag = if is_freeze || inner.speed_percent.abs() < 1e-4 {
                0.0
            } else {
                (inner.speed_percent.abs() * outer_abs_speed).clamp(0.1, 10_000.0)
            };
            let composed_speed = if composed_reversed && composed_speed_mag > 0.0 {
                -composed_speed_mag
            } else {
                composed_speed_mag
            };

            let composed_opacity = ((inner.opacity / 100.0) * (outer.opacity / 100.0) * 100.0)
                .clamp(0.0, 100.0);

            // Preserve inner motion if non-identity, otherwise inherit outer motion.
            let composed_motion = if !inner.motion.is_identity() {
                inner.motion.clone()
            } else {
                outer.motion.clone()
            };

            let mut composed_effects = inner.effects.clone();
            composed_effects.extend(outer.effects.iter().cloned());

            let mut composed_lumetri = inner.lumetri_effects.clone();
            let base_order = composed_lumetri.len();
            for (i, lum) in outer.lumetri_effects.iter().enumerate() {
                let mut l = lum.clone();
                l.order = base_order + i;
                composed_lumetri.push(l);
            }

            let mut composed_markers = inner.markers.clone();
            composed_markers.extend(outer.markers.iter().cloned());

            let cid = format!("flat-{}-{}", inner.id, *next_id);
            *next_id += 1;

            let mut flat = inner.clone();
            flat.id = cid;
            flat.start_frame = clipped_start;
            flat.end_frame = clipped_end;
            flat.source_in_frame = new_src_in;
            flat.source_out_frame = new_src_out;
            flat.source_in_ticks = None;
            flat.source_out_ticks = None;
            flat.speed_percent = composed_speed;
            flat.reversed = composed_reversed;
            flat.enabled = outer.enabled && inner.enabled;
            flat.opacity = composed_opacity;
            flat.motion = composed_motion;
            flat.effects = composed_effects;
            flat.lumetri_effects = composed_lumetri;
            flat.markers = composed_markers;
            if outer.label_preset_index.is_some() && flat.label_preset_index.is_none() {
                flat.label_preset_index = outer.label_preset_index;
                flat.label_name = outer.label_name.clone();
            }
            // Carry outer link group for single-layer expansions (e.g. multicam V+A pairs).
            if layer_idx == 0 && outer.link_group_id.is_some() {
                flat.link_group_id = outer.link_group_id.clone();
            }

            // Recursively expand if the inner clip is itself nested/compound/multicam.
            if flat.source_kind != ClipSourceKind::Media {
                match expand_container_clip(
                    &flat,
                    kind,
                    parent_fps,
                    seq_snapshot,
                    visited,
                    depth + 1,
                    next_id,
                ) {
                    Ok(nested_layers) => {
                        for (sub_layer, sub_clip, sub_ch) in nested_layers {
                            out_layers.push((layer_idx + sub_layer, sub_clip, sub_ch));
                        }
                    }
                    Err(_) => {
                        out_layers.push((layer_idx, flat, ch));
                    }
                }
            } else {
                out_layers.push((layer_idx, flat, ch));
            }
        }
    }

    visited.remove(&target_seq.id);
    Ok(out_layers)
}

fn resolve_multicam_angle_index(active_angle: Option<&str>, tracks: &[Track]) -> usize {
    let Some(angle) = active_angle.map(str::trim).filter(|s| !s.is_empty()) else {
        return 0;
    };
    let lower = angle.to_ascii_lowercase();

    // 1. Exact track name match (case-insensitive)
    if let Some(pos) = tracks
        .iter()
        .position(|t| t.name.trim().to_ascii_lowercase() == lower)
    {
        return pos;
    }

    // 2. Numeric suffix ("Camera 2", "Cam 3", "Angle 2", "V2", "2")
    let digits: String = angle
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if let Ok(num) = digits.parse::<usize>()
        && num >= 1
        && num <= tracks.len()
    {
        return num - 1;
    }

    0
}

fn source_kind_label(kind: ClipSourceKind) -> &'static str {
    match kind {
        ClipSourceKind::Media => "media clip",
        ClipSourceKind::NestedSequence => "nested sequence",
        ClipSourceKind::CompoundClip => "compound clip",
        ClipSourceKind::MulticamClip => "multicam clip",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MediaKind, MotionTransform, RetimeInterpolation};
    use std::collections::BTreeMap;

    fn make_media_clip(
        id: &str,
        name: &str,
        path: &str,
        start: i64,
        end: i64,
        src_in: i64,
        src_out: i64,
    ) -> Clip {
        Clip {
            id: id.into(),
            name: name.into(),
            media_id: id.into(),
            media_path: path.into(),
            media_kind: MediaKind::Video,
            source_kind: ClipSourceKind::Media,
            active_angle: None,
            start_frame: start,
            end_frame: end,
            source_in_frame: src_in,
            source_out_frame: src_out,
            source_in_ticks: None,
            source_out_ticks: None,
            source_fps: Rational::FPS_25,
            source_width: 1920,
            source_height: 1080,
            speed_percent: 100.0,
            reversed: false,
            speed_keyframes: Vec::new(),
            retime_interpolation: RetimeInterpolation::Default,
            enabled: true,
            audio_channel: 0,
            link_group_id: None,
            label_name: None,
            label_preset_index: None,
            motion: MotionTransform::default(),
            opacity: 100.0,
            lumetri_effects: Vec::new(),
            effects: Vec::new(),
            markers: Vec::new(),
        }
    }

    fn make_track(index: usize, name: &str, kind: TrackKind, clips: Vec<Clip>) -> Track {
        Track {
            index,
            kind,
            name: name.into(),
            enabled: true,
            locked: false,
            muted: false,
            solo: false,
            audio_channels: if kind == TrackKind::Audio { 2 } else { 0 },
            output_bus_id: None,
            clips,
            transitions: Vec::new(),
            markers: Vec::new(),
        }
    }

    #[test]
    fn flattens_multicam_angle_cuts_and_removes_helper_sequence() {
        let mc_seq = Sequence {
            id: "mc-1".into(),
            name: "Multicam_3Cam".into(),
            kind: SequenceKind::MulticamClip,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 200,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: vec![
                make_track(
                    1,
                    "Camera 1",
                    TrackKind::Video,
                    vec![make_media_clip("cam1", "CamA.mov", "/media/CamA.mov", 0, 200, 10, 210)],
                ),
                make_track(
                    2,
                    "Camera 2",
                    TrackKind::Video,
                    vec![make_media_clip("cam2", "CamB.mov", "/media/CamB.mov", 0, 200, 50, 250)],
                ),
            ],
            audio_tracks: Vec::new(),
            audio_buses: Vec::new(),
            markers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let mut cut1 = make_media_clip("cut1", "MC Cut 1", "", 0, 75, 0, 75);
        cut1.source_kind = ClipSourceKind::MulticamClip;
        cut1.media_id = "mc-1".into();
        cut1.active_angle = Some("Camera 1".into());

        let mut cut2 = make_media_clip("cut2", "MC Cut 2", "", 75, 150, 75, 150);
        cut2.source_kind = ClipSourceKind::MulticamClip;
        cut2.media_id = "mc-1".into();
        cut2.active_angle = Some("Camera 2".into());

        let main_seq = Sequence {
            id: "main".into(),
            name: "Main Edit".into(),
            kind: SequenceKind::Timeline,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 150,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: vec![make_track(1, "V1", TrackKind::Video, vec![cut1, cut2])],
            audio_tracks: Vec::new(),
            audio_buses: Vec::new(),
            markers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let mut project = Project {
            sequences: vec![mc_seq, main_seq],
            ..Default::default()
        };

        let summary = flatten_nested_and_multicam_clips(&mut project);
        assert_eq!(summary.clips_flattened, 2);
        assert_eq!(summary.multicam_clips_flattened, 2);
        assert_eq!(summary.helper_sequences_removed, 1);
        assert_eq!(project.sequences.len(), 1);

        let clips = &project.sequences[0].video_tracks[0].clips;
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].source_kind, ClipSourceKind::Media);
        assert_eq!(clips[0].media_path, "/media/CamA.mov");
        assert_eq!((clips[0].start_frame, clips[0].end_frame), (0, 75));
        assert_eq!((clips[0].source_in_frame, clips[0].source_out_frame), (10, 85));

        assert_eq!(clips[1].source_kind, ClipSourceKind::Media);
        assert_eq!(clips[1].media_path, "/media/CamB.mov");
        assert_eq!((clips[1].start_frame, clips[1].end_frame), (75, 150));
        assert_eq!((clips[1].source_in_frame, clips[1].source_out_frame), (125, 200));
    }

    #[test]
    fn flattens_multilayer_compound_clip_into_multiple_tracks() {
        let comp_seq = Sequence {
            id: "comp-1".into(),
            name: "Logo_Compound".into(),
            kind: SequenceKind::CompoundClip,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 100,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: vec![
                make_track(
                    1,
                    "V1",
                    TrackKind::Video,
                    vec![make_media_clip("bg", "BG.mov", "/media/BG.mov", 0, 100, 0, 100)],
                ),
                make_track(
                    2,
                    "V2",
                    TrackKind::Video,
                    vec![make_media_clip("fg", "Logo.png", "/media/Logo.png", 20, 80, 0, 60)],
                ),
            ],
            audio_tracks: Vec::new(),
            audio_buses: Vec::new(),
            markers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let mut comp_clip = make_media_clip("c1", "Logo_Compound", "", 50, 120, 10, 80);
        comp_clip.source_kind = ClipSourceKind::CompoundClip;
        comp_clip.media_id = "comp-1".into();
        comp_clip.opacity = 50.0;

        let main_seq = Sequence {
            id: "main".into(),
            name: "Main".into(),
            kind: SequenceKind::Timeline,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 120,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: vec![make_track(1, "V1", TrackKind::Video, vec![comp_clip])],
            audio_tracks: Vec::new(),
            audio_buses: Vec::new(),
            markers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let mut project = Project {
            sequences: vec![comp_seq, main_seq],
            ..Default::default()
        };

        let summary = flatten_nested_and_multicam_clips(&mut project);
        assert_eq!(summary.compound_clips_flattened, 1);
        assert_eq!(project.sequences.len(), 1);
        assert_eq!(project.sequences[0].video_tracks.len(), 2);

        let v1 = &project.sequences[0].video_tracks[0].clips[0];
        assert_eq!(v1.media_path, "/media/BG.mov");
        assert_eq!((v1.start_frame, v1.end_frame), (50, 120));
        assert_eq!((v1.source_in_frame, v1.source_out_frame), (10, 80));
        assert!((v1.opacity - 50.0).abs() < 1e-6);

        let v2 = &project.sequences[0].video_tracks[1].clips[0];
        assert_eq!(v2.media_path, "/media/Logo.png");
        // inner 20..80 sliced against window 10..80 -> offset +10..+70 -> parent 60..120
        assert_eq!((v2.start_frame, v2.end_frame), (60, 120));
        assert!((v2.opacity - 50.0).abs() < 1e-6);
    }

    #[test]
    fn handles_missing_nested_sequence_gracefully() {
        let mut missing = make_media_clip("m1", "MissingNest", "", 0, 50, 0, 50);
        missing.source_kind = ClipSourceKind::NestedSequence;
        missing.media_id = "non-existent".into();

        let main_seq = Sequence {
            id: "main".into(),
            name: "Main".into(),
            kind: SequenceKind::Timeline,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 50,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: vec![make_track(1, "V1", TrackKind::Video, vec![missing])],
            audio_tracks: Vec::new(),
            audio_buses: Vec::new(),
            markers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let mut project = Project {
            sequences: vec![main_seq],
            ..Default::default()
        };

        let summary = flatten_nested_and_multicam_clips(&mut project);
        assert_eq!(summary.clips_flattened, 0);
        assert_eq!(summary.messages.len(), 1);
        assert_eq!(summary.messages[0].code, "FLATTEN_UNRESOLVED");
        assert_eq!(project.sequences[0].video_tracks[0].clips.len(), 1);
    }
}
