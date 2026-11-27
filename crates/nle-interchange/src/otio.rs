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

    for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
        tracks_json.push(serialize_track(track, rate));
        if track.kind == TrackKind::Video {
            total_v += track.clips.len();
        } else {
            total_a += track.clips.len();
        }
    }

    let markers_json: Vec<Value> = seq
        .markers
        .iter()
        .map(|m| {
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
        })
        .collect();

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
        markers_written: seq.markers.len(),
        grades_attached: 0,
        effect_markers_injected: 0,
        messages,
    })
}

fn serialize_track(track: &Track, rate: f64) -> Value {
    let mut sorted = track.clips.clone();
    sorted.sort_by_key(|c| c.start_frame);
    let mut children = Vec::new();
    let mut cursor = 0i64;

    for clip in &sorted {
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
                "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": clip.duration_frames().max(1) as f64 }
            }
        }));
        cursor = clip.end_frame.max(cursor);
    }

    json!({
        "OTIO_SCHEMA": "Track.1",
        "name": track.name,
        "kind": if track.kind == TrackKind::Video { "Video" } else { "Audio" },
        "enabled": track.enabled,
        "children": children
    })
}
