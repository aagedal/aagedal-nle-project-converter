use nle_core::{
    flatten_nested_and_multicam_clips, ClipSourceKind, ConversionMessage, ConversionOptions,
    ConversionReport, NleError, NleFormat, NleResult, Project, ProjectWriter, Sequence,
    SequenceKind, Track, TrackKind,
};
use serde_json::{json, Value};
use std::borrow::Cow;
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
    let seq = selected
        .iter()
        .copied()
        .find(|s| s.kind == SequenceKind::Timeline)
        .or_else(|| selected.first().copied())
        .ok_or_else(|| NleError::InvalidFormat("No sequences selected for OTIO export".into()))?;

    let rate = seq.frame_rate.as_f64();
    let mut tracks_json = Vec::new();
    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;

    for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
        let (track_val, clip_markers_written) =
            serialize_track(track, rate, options.include_markers);
        tracks_json.push(track_val);
        total_markers += clip_markers_written;
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
                "project_name": project.name,
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

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(output_path, serde_json::to_string_pretty(&timeline)?)?;

    Ok(ConversionReport {
        output_path: output_path.to_string_lossy().into_owned(),
        source_format: project.source_format,
        target_format: NleFormat::OpenTimelineIo,
        sequences_converted: 1,
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

fn serialize_track(track: &Track, rate: f64, include_markers: bool) -> (Value, usize) {
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
            if !used_tr[ti] && tr.start_frame <= clip.start_frame && tr.end_frame >= clip.start_frame {
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
        let target_url = if clip.media_path.starts_with("file://") {
            clip.media_path.clone()
        } else {
            format!("file://{}", clip.media_path.replace('\\', "/"))
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

        let clip_markers: Vec<Value> = if include_markers {
            markers_written += clip.markers.len();
            clip.markers
                .iter()
                .map(|m| serialize_marker(m, rate))
                .collect()
        } else {
            Vec::new()
        };

        let tl_dur = clip.duration_frames().max(1) as f64;
        let src_dur = if is_freeze {
            tl_dur
        } else if is_retimed {
            (tl_dur * signed_scalar.abs()).max(1.0)
        } else {
            tl_dur
        };

        children.push(json!({
            "OTIO_SCHEMA": "Clip.2",
            "name": clip.name,
            "enabled": clip.enabled,
            "media_reference": {
                "OTIO_SCHEMA": "ExternalReference.1",
                "name": clip.name,
                "target_url": target_url
            },
            "source_range": {
                "OTIO_SCHEMA": "TimeRange.1",
                "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": clip.source_in_frame as f64 },
                "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": src_dur }
            },
            "effects": effects,
            "markers": clip_markers
        }));
        cursor = clip.end_frame.max(cursor);
    }

    // Emit any remaining trailing transitions (e.g. fade-out at end of track)
    for (ti, tr) in transitions.iter().enumerate() {
        if !used_tr[ti] {
            children.push(serialize_transition(tr, rate));
        }
    }

    (
        json!({
            "OTIO_SCHEMA": "Track.1",
            "name": track.name,
            "kind": if track.kind == TrackKind::Video { "Video" } else { "Audio" },
            "enabled": track.enabled,
            "children": children
        }),
        markers_written,
    )
}
