use crate::fcpxml::time::path_to_file_url;
use nle_core::{
    flatten_nested_and_multicam_clips, BlendMode, Clip, ClipSourceKind, ConversionMessage,
    ConversionOptions, ConversionReport, MediaAsset, NleError, NleFormat, NleResult, Project,
    ProjectWriter, Sequence, SequenceKind, Severity, Track, TrackKind,
};
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct OtioWriter;

impl ProjectWriter for OtioWriter {
    fn format(&self) -> NleFormat {
        NleFormat::OpenTimelineIo
    }

    fn write_file(
        &self,
        project: &Project,
        output_path: &Path,
        options: &ConversionOptions,
    ) -> NleResult<ConversionReport> {
        write_otio_file(project, output_path, options)
    }
}

/// Write an OpenTimelineIO (`.otio`) JSON file from the canonical `Project` IR.
/// Automatically flattens nested/compound/multicam clips into source media clips so
/// every OTIO `ExternalReference` resolves to a real media file.
pub fn write_otio_file(
    project: &Project,
    output_path: &Path,
    options: &ConversionOptions,
) -> NleResult<ConversionReport> {
    let has_nested = project.sequences.iter().any(|s| {
        s.video_tracks
            .iter()
            .chain(s.audio_tracks.iter())
            .any(|t| t.clips.iter().any(|c| c.source_kind != ClipSourceKind::Media))
    });
    let mut messages: Vec<ConversionMessage> = Vec::new();
    let project_cow: Cow<'_, Project> = if options.flatten_nested_and_multicam || has_nested {
        let mut cloned = project.clone();
        let flat = flatten_nested_and_multicam_clips(&mut cloned);
        messages.extend(flat.messages);
        Cow::Owned(cloned)
    } else {
        Cow::Borrowed(project)
    };
    let project = project_cow.as_ref();

    let selected: Vec<&Sequence> = if options.selected_sequence_ids.is_empty() {
        project.sequences.iter().collect()
    } else {
        project
            .sequences
            .iter()
            .filter(|s| options.selected_sequence_ids.contains(&s.id))
            .collect()
    };

    let timeline_seqs: Vec<&Sequence> = selected
        .iter()
        .copied()
        .filter(|s| s.kind == SequenceKind::Timeline)
        .collect();
    let export_timelines: Vec<&Sequence> = if timeline_seqs.is_empty() {
        selected
    } else {
        timeline_seqs
    };
    if export_timelines.is_empty() {
        return Err(NleError::InvalidFormat(
            "No sequences selected for OTIO export".into(),
        ));
    }

    let assets_by_id: HashMap<&str, &MediaAsset> = project
        .media_assets
        .iter()
        .map(|a| (a.id.as_str(), a))
        .collect();
    let assets_by_path: HashMap<&str, &MediaAsset> = project
        .media_assets
        .iter()
        .filter(|a| !a.path.is_empty())
        .map(|a| (a.path.as_str(), a))
        .collect();

    let proj_name = options
        .project_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&project.name);

    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;
    let mut timelines_json = Vec::with_capacity(export_timelines.len());

    for seq in &export_timelines {
        let (tl_val, v_cnt, a_cnt, m_cnt) = serialize_timeline(
            seq,
            proj_name,
            options,
            &assets_by_id,
            &assets_by_path,
        );
        timelines_json.push(tl_val);
        total_v += v_cnt;
        total_a += a_cnt;
        total_markers += m_cnt;
    }

    let root_doc = if timelines_json.len() == 1 {
        timelines_json.into_iter().next().unwrap()
    } else {
        messages.push(ConversionMessage {
            severity: Severity::Info,
            code: "OTIO_SERIALIZABLE_COLLECTION".into(),
            message: format!(
                "Exported {} selected timelines into an OpenTimelineIO SerializableCollection.1 container (note: some single-timeline NLE importers only accept a single Timeline.1).",
                export_timelines.len()
            ),
            sequence: None,
            clip: None,
        });
        json!({
            "OTIO_SCHEMA": "SerializableCollection.1",
            "name": proj_name,
            "metadata": {
                "aagedal_nle_converter": {
                    "project_name": proj_name
                }
            },
            "children": timelines_json
        })
    };

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, serde_json::to_string_pretty(&root_doc)?)?;

    Ok(ConversionReport {
        output_path: output_path.to_string_lossy().into_owned(),
        source_format: project.source_format,
        target_format: NleFormat::OpenTimelineIo,
        sequences_converted: export_timelines.len(),
        video_clips_converted: total_v,
        audio_clips_converted: total_a,
        media_assets_linked: project.media_assets.len(),
        bins_created: 0,
        markers_written: total_markers,
        grades_attached: 0,
        effect_markers_injected: 0,
        messages,
    })
}

fn serialize_timeline(
    seq: &Sequence,
    proj_name: &str,
    options: &ConversionOptions,
    assets_by_id: &HashMap<&str, &MediaAsset>,
    assets_by_path: &HashMap<&str, &MediaAsset>,
) -> (Value, usize, usize, usize) {
    let rate = seq.frame_rate.as_f64();
    let mut tracks_json = Vec::new();
    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;

    for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
        let (track_val, track_markers_written) = serialize_track(
            track,
            rate,
            options,
            assets_by_id,
            assets_by_path,
        );
        tracks_json.push(track_val);
        total_markers += track_markers_written;
        if track.kind == TrackKind::Video {
            total_v += track.clips.len();
        } else {
            total_a += track.clips.len();
        }
    }

    let markers_json: Vec<Value> = if options.include_markers {
        total_markers += seq.markers.len();
        seq.markers
            .iter()
            .map(|m| serialize_marker(m, rate))
            .collect()
    } else {
        Vec::new()
    };

    let timeline = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": seq.name,
        "global_start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": seq.start_tc_frame.max(0) as f64 },
        "metadata": {
            "aagedal_nle_converter": {
                "project_name": proj_name,
                "width": seq.width,
                "height": seq.height
            }
        },
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "name": "tracks",
            "children": tracks_json,
            "markers": markers_json
        }
    });

    (timeline, total_v, total_a, total_markers)
}

fn serialize_marker(m: &nle_core::Marker, rate: f64) -> Value {
    json!({
        "OTIO_SCHEMA": "Marker.2",
        "name": m.name,
        "color": m.color.to_ascii_uppercase(),
        "comment": m.comment,
        "marked_range": {
            "OTIO_SCHEMA": "TimeRange.1",
            "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": m.frame as f64 },
            "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": m.duration_frames.max(1) as f64 }
        }
    })
}

fn serialize_transition(tr: &nle_core::Transition, rate: f64) -> Value {
    let dur = (tr.end_frame - tr.start_frame).max(1) as f64;
    let (in_off, out_off) = match tr.alignment.as_str() {
        "start" => (0.0, dur),
        "end" => (dur, 0.0),
        _ => {
            let half = (dur / 2.0).floor();
            (half, (dur - half).max(0.0))
        }
    };
    json!({
        "OTIO_SCHEMA": "Transition.1",
        "name": tr.name,
        "transition_type": "SMPTE_Dissolve",
        "in_offset": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": in_off },
        "out_offset": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": out_off }
    })
}

fn serialize_clip_metadata(clip: &Clip, include_clip_colors: bool) -> Value {
    let mut meta = Map::new();
    if !clip.motion.is_identity() {
        let m = &clip.motion;
        let mut motion_map = Map::new();
        motion_map.insert("scale".into(), json!(m.scale));
        if let Some(sy) = m.scale_y {
            motion_map.insert("scale_y".into(), json!(sy));
        }
        if m.position_x.abs() > 1e-6 {
            motion_map.insert("position_x".into(), json!(m.position_x));
        }
        if m.position_y.abs() > 1e-6 {
            motion_map.insert("position_y".into(), json!(m.position_y));
        }
        if m.rotation.abs() > 1e-6 {
            motion_map.insert("rotation".into(), json!(m.rotation));
        }
        if m.anchor_x.abs() > 1e-6 {
            motion_map.insert("anchor_x".into(), json!(m.anchor_x));
        }
        if m.anchor_y.abs() > 1e-6 {
            motion_map.insert("anchor_y".into(), json!(m.anchor_y));
        }
        if m.crop_left.abs() > 1e-6 {
            motion_map.insert("crop_left".into(), json!(m.crop_left));
        }
        if m.crop_right.abs() > 1e-6 {
            motion_map.insert("crop_right".into(), json!(m.crop_right));
        }
        if m.crop_top.abs() > 1e-6 {
            motion_map.insert("crop_top".into(), json!(m.crop_top));
        }
        if m.crop_bottom.abs() > 1e-6 {
            motion_map.insert("crop_bottom".into(), json!(m.crop_bottom));
        }
        meta.insert("motion".into(), Value::Object(motion_map));
    }
    if (clip.opacity - 100.0).abs() > 1e-4 {
        meta.insert("opacity".into(), json!(clip.opacity));
    }
    if clip.blend_mode != BlendMode::Normal {
        meta.insert("blend_mode".into(), json!(clip.blend_mode.display_name()));
    }
    if include_clip_colors
        && let Some(label) = clip.label_name.as_deref().filter(|s| !s.is_empty())
    {
        meta.insert("label_name".into(), json!(label));
    }
    if let Some(lg) = clip.link_group_id.as_deref().filter(|s| !s.is_empty()) {
        meta.insert("link_group_id".into(), json!(lg));
    }
    if meta.is_empty() {
        json!({})
    } else {
        json!({ "aagedal_nle_converter": Value::Object(meta) })
    }
}

fn serialize_track(
    track: &Track,
    rate: f64,
    options: &ConversionOptions,
    assets_by_id: &HashMap<&str, &MediaAsset>,
    assets_by_path: &HashMap<&str, &MediaAsset>,
) -> (Value, usize) {
    let mut sorted = track.clips.clone();
    sorted.sort_by_key(|c| c.start_frame);
    let mut transitions = track.transitions.clone();
    transitions.sort_by_key(|t| t.start_frame);
    let mut used_tr = vec![false; transitions.len()];

    let mut children = Vec::new();
    let mut cursor = 0i64;
    let mut markers_written = 0usize;

    for clip in &sorted {
        // Interleave any transition whose cut boundary lands at or near clip.start_frame
        for (ti, tr) in transitions.iter().enumerate() {
            if !used_tr[ti]
                && tr.start_frame <= clip.start_frame
                && tr.end_frame >= clip.start_frame
            {
                children.push(serialize_transition(tr, rate));
                used_tr[ti] = true;
            }
        }

        if clip.start_frame > cursor {
            children.push(json!({
                "OTIO_SCHEMA": "Gap.1",
                "name": "",
                "source_range": {
                    "OTIO_SCHEMA": "TimeRange.1",
                    "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": 0.0 },
                    "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": (clip.start_frame - cursor) as f64 }
                }
            }));
        }

        let asset = assets_by_id
            .get(clip.media_id.as_str())
            .or_else(|| assets_by_path.get(clip.media_path.as_str()))
            .copied();

        let src_rate = asset
            .map(|a| a.frame_rate.as_f64())
            .filter(|&r| r > 0.0)
            .unwrap_or_else(|| {
                let r = clip.source_fps.as_f64();
                if r > 0.0 { r } else { rate }
            });
        let asset_tc = asset.map(|a| a.source_tc_frame.max(0)).unwrap_or(0);
        let asset_dur = asset.map(|a| a.duration_frames.max(0)).unwrap_or(0);

        let media_ref = if clip.media_path.is_empty() {
            json!({
                "OTIO_SCHEMA": "MissingReference.1",
                "name": clip.name
            })
        } else {
            let target_url = path_to_file_url(&clip.media_path);
            let mut ext_ref = Map::new();
            ext_ref.insert("OTIO_SCHEMA".into(), json!("ExternalReference.1"));
            ext_ref.insert("name".into(), json!(clip.name));
            ext_ref.insert("target_url".into(), json!(target_url));
            if asset_tc > 0 || asset_dur > 0 {
                let avail_dur = asset_dur.max(clip.source_out_frame).max(clip.duration_frames()).max(1);
                ext_ref.insert(
                    "available_range".into(),
                    json!({
                        "OTIO_SCHEMA": "TimeRange.1",
                        "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": src_rate, "value": asset_tc as f64 },
                        "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": src_rate, "value": avail_dur as f64 }
                    }),
                );
            }
            Value::Object(ext_ref)
        };

        let mut effects = Vec::new();
        let abs_speed = clip.speed_percent.abs();
        let signed_scalar = if clip.reversed && clip.speed_percent > 0.0 {
            -clip.speed_percent / 100.0
        } else {
            clip.speed_percent / 100.0
        };
        let is_freeze = abs_speed < 0.01;
        let is_retimed = !is_freeze && ((signed_scalar - 1.0).abs() > 1e-4 || clip.reversed);
        if is_freeze {
            effects.push(json!({
                "OTIO_SCHEMA": "FreezeFrame.1",
                "name": "FreezeFrame",
                "effect_name": "FreezeFrame"
            }));
        } else if is_retimed {
            effects.push(json!({
                "OTIO_SCHEMA": "LinearTimeWarp.1",
                "name": "LinearTimeWarp",
                "effect_name": "LinearTimeWarp",
                "time_scalar": signed_scalar
            }));
        }

        let clip_markers: Vec<Value> = if options.include_markers {
            markers_written += clip.markers.len();
            clip.markers
                .iter()
                .map(|m| {
                    let mut m_shifted = m.clone();
                    m_shifted.frame = asset_tc + m.frame.max(0);
                    serialize_marker(&m_shifted, src_rate)
                })
                .collect()
        } else {
            Vec::new()
        };

        // Per OTIO spec, Clip.source_range.duration is the clip's duration on the parent Track
        // (independent of LinearTimeWarp.time_scalar / FreezeFrame).
        let tl_dur = clip.duration_frames().max(1) as f64;
        let src_start_val = (asset_tc + clip.source_in_frame.max(0)) as f64;
        let clip_metadata = serialize_clip_metadata(clip, options.include_clip_colors);

        children.push(json!({
            "OTIO_SCHEMA": "Clip.2",
            "name": clip.name,
            "enabled": clip.enabled,
            "media_reference": media_ref,
            "source_range": {
                "OTIO_SCHEMA": "TimeRange.1",
                "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": src_rate, "value": src_start_val },
                "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": tl_dur }
            },
            "effects": effects,
            "markers": clip_markers,
            "metadata": clip_metadata
        }));
        cursor = clip.end_frame.max(cursor);
    }

    // Emit any remaining trailing transitions (e.g. fade-out at end of track)
    for (ti, tr) in transitions.iter().enumerate() {
        if !used_tr[ti] {
            children.push(serialize_transition(tr, rate));
        }
    }

    let track_markers: Vec<Value> = if options.include_markers {
        markers_written += track.markers.len();
        track
            .markers
            .iter()
            .map(|m| serialize_marker(m, rate))
            .collect()
    } else {
        Vec::new()
    };

    let mut track_obj = Map::new();
    track_obj.insert("OTIO_SCHEMA".into(), json!("Track.1"));
    track_obj.insert("name".into(), json!(track.name));
    track_obj.insert(
        "kind".into(),
        json!(if track.kind == TrackKind::Video {
            "Video"
        } else {
            "Audio"
        }),
    );
    track_obj.insert("enabled".into(), json!(track.enabled));
    track_obj.insert("children".into(), Value::Array(children));
    if !track_markers.is_empty() {
        track_obj.insert("markers".into(), Value::Array(track_markers));
    }
    if track.kind == TrackKind::Audio && track.audio_channels > 0 {
        track_obj.insert(
            "metadata".into(),
            json!({ "aagedal_nle_converter": { "audio_channels": track.audio_channels } }),
        );
    }

    (Value::Object(track_obj), markers_written)
}
