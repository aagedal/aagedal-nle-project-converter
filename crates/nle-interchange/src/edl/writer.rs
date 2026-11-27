use nle_core::{
    flatten_nested_and_multicam_clips, frames_to_timecode, Clip, ClipSourceKind, ConversionMessage,
    ConversionOptions, ConversionReport, MediaAsset, NleError, NleFormat, NleResult, Project,
    ProjectWriter, Rational, Sequence, SequenceKind, Severity, Transition,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct EdlWriter;

impl ProjectWriter for EdlWriter {
    fn format(&self) -> NleFormat {
        NleFormat::Edl
    }

    fn write_file(
        &self,
        project: &Project,
        output_path: &Path,
        options: &ConversionOptions,
    ) -> NleResult<ConversionReport> {
        write_edl_file(project, output_path, options)
    }
}

/// Write a CMX 3600 `.edl` file from the canonical `Project` IR.
/// Automatically flattens nested/compound/multicam clips into source media clips and exports the
/// primary video track (`V1`) plus audio tracks (`A1`–`A4`), dissolves, `M2` retiming, and markers.
pub fn write_edl_file(
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
        .ok_or_else(|| NleError::InvalidFormat("No sequences selected for EDL export".into()))?;

    let skipped_timelines: Vec<&str> = selected
        .iter()
        .filter(|s| s.id != seq.id && s.kind == SequenceKind::Timeline)
        .map(|s| s.name.as_str())
        .collect();
    if !skipped_timelines.is_empty() {
        messages.push(ConversionMessage {
            severity: Severity::Warning,
            code: "EDL_SINGLE_SEQUENCE".into(),
            message: format!(
                "CMX 3600 EDL (.edl) stores one timeline per file: exported '{}'; skipped {} other selected timeline(s).",
                seq.name,
                skipped_timelines.len()
            ),
            sequence: Some(seq.name.clone()),
            clip: None,
        });
    }

    let extra_video_tracks = seq
        .video_tracks
        .iter()
        .skip(1)
        .filter(|t| !t.clips.is_empty())
        .count();
    if extra_video_tracks > 0 {
        messages.push(ConversionMessage {
            severity: Severity::Warning,
            code: "EDL_SINGLE_VIDEO_TRACK".into(),
            message: format!(
                "CMX 3600 EDL supports a single primary video track (V1): exported V1; {} upper video overlay track(s) (V2+) were omitted.",
                extra_video_tracks
            ),
            sequence: Some(seq.name.clone()),
            clip: None,
        });
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

    let fps = seq.frame_rate;
    let is_df = fps.is_ntsc() && matches!(fps.timebase(), 30 | 60);
    let rec_offset = seq.start_tc_frame.max(0);

    let mut out = String::new();
    out.push_str(&format!("TITLE: {}\n", seq.name));
    out.push_str(&format!(
        "FCM: {}\n",
        if is_df {
            "DROP FRAME"
        } else {
            "NON-DROP FRAME"
        }
    ));
    out.push_str(&format!("* FPS: {}\n\n", fps.label()));

    let mut event_num = 1usize;
    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;

    // Emit V1 clips + dissolves
    if let Some(v1) = seq.video_tracks.first() {
        let mut clips: Vec<&Clip> = v1.clips.iter().filter(|c| c.enabled).collect();
        clips.sort_by_key(|c| c.start_frame);

        let mut transitions: Vec<&Transition> = v1.transitions.iter().collect();
        transitions.sort_by_key(|t| t.start_frame);

        let mut cursor = 0i64;
        for (ci, clip) in clips.iter().enumerate() {
            let asset = assets_by_id
                .get(clip.media_id.as_str())
                .or_else(|| assets_by_path.get(clip.media_path.as_str()))
                .copied();

            // Check if a dissolve bridges the previous clip and this clip at clip.start_frame
            let incoming_tr = if ci > 0 {
                transitions.iter().copied().find(|tr| {
                    tr.start_frame <= clip.start_frame
                        && tr.end_frame >= clip.start_frame
                        && tr.end_frame > tr.start_frame
                })
            } else {
                None
            };

            // Check if a dissolve bridges this clip and the NEXT clip at clip.end_frame
            let outgoing_tr = if ci + 1 < clips.len() {
                transitions.iter().copied().find(|tr| {
                    tr.start_frame <= clip.end_frame
                        && tr.end_frame >= clip.end_frame
                        && tr.end_frame > tr.start_frame
                })
            } else {
                None
            };

            // Emit BL gap if there is an empty space before this clip
            if incoming_tr.is_none() && clip.start_frame > cursor {
                let gap_dur = clip.start_frame - cursor;
                out.push_str(&format!(
                    "{:03}  BL       V     C        {} {} {} {}\n\n",
                    event_num,
                    frames_to_timecode(0, fps),
                    frames_to_timecode(gap_dur, fps),
                    frames_to_timecode(rec_offset + cursor, fps),
                    frames_to_timecode(rec_offset + clip.start_frame, fps),
                ));
                event_num += 1;
            }

            let reel = derive_reel_name(clip, asset);
            let asset_tc = asset.map(|a| a.source_tc_frame.max(0)).unwrap_or(0);
            let mut rec_in = clip.start_frame;
            let mut rec_out = clip.end_frame;
            let mut src_in = asset_tc + clip.source_in_frame.max(0);
            let mut src_out = asset_tc + clip.source_out_frame.max(clip.source_in_frame);

            if let Some(tr) = incoming_tr {
                // CMX 3600 dissolve incoming event starts at tr.start_frame (half transition earlier)
                let shift = (rec_in - tr.start_frame).max(0);
                rec_in = tr.start_frame;
                src_in = (src_in - shift).max(0);
            }
            if let Some(tr) = outgoing_tr {
                // Outgoing clip stops at tr.start_frame; dissolve carries through tr.end_frame
                let trim = (rec_out - tr.start_frame).max(0);
                rec_out = tr.start_frame.max(rec_in);
                src_out = (src_out - trim).max(src_in);
            }

            let edit_col = if let Some(tr) = incoming_tr {
                let dur = (tr.end_frame - tr.start_frame).clamp(1, 999);
                format!("D    {dur:03}")
            } else {
                "C       ".to_string()
            };

            out.push_str(&format!(
                "{:03}  {:<8} V     {} {} {} {} {}\n",
                event_num,
                reel,
                edit_col,
                frames_to_timecode(src_in, fps),
                frames_to_timecode(src_out, fps),
                frames_to_timecode(rec_offset + rec_in, fps),
                frames_to_timecode(rec_offset + rec_out, fps),
            ));

            emit_m2_and_comments(&mut out, clip, &reel, fps, src_in, options.include_clip_colors);

            // Attach sequence markers falling inside this clip's record span using * LOC: with exact TC
            if options.include_markers {
                for m in &seq.markers {
                    if m.frame >= clip.start_frame && m.frame < clip.end_frame {
                        let color_up = m.color.to_ascii_uppercase();
                        let comment = if m.comment.is_empty() {
                            m.name.as_str()
                        } else {
                            m.comment.as_str()
                        };
                        out.push_str(&format!(
                            "* LOC: {} {:<7} {} |C:ResolveColor{} |M:{} |D:{}\n",
                            frames_to_timecode(rec_offset + m.frame.max(0), fps),
                            color_up,
                            comment,
                            capitalize_color(&m.color),
                            m.name,
                            m.duration_frames.max(1)
                        ));
                        total_markers += 1;
                    }
                }
            }

            out.push('\n');
            event_num += 1;
            total_v += 1;
            cursor = clip.end_frame.max(cursor);
        }
    }

    // Emit audio tracks (A1..A4)
    for (ti, atrack) in seq.audio_tracks.iter().take(4).enumerate() {
        let mut clips: Vec<&Clip> = atrack.clips.iter().filter(|c| c.enabled).collect();
        clips.sort_by_key(|c| c.start_frame);
        let track_code = if ti == 0 && atrack.audio_channels >= 2 {
            "AA".to_string()
        } else {
            format!("A{}", ti + 1)
        };
        for clip in clips {
            let asset = assets_by_id
                .get(clip.media_id.as_str())
                .or_else(|| assets_by_path.get(clip.media_path.as_str()))
                .copied();
            let reel = derive_reel_name(clip, asset);
            let asset_tc = asset.map(|a| a.source_tc_frame.max(0)).unwrap_or(0);
            let src_in = asset_tc + clip.source_in_frame.max(0);
            let src_out = asset_tc + clip.source_out_frame.max(clip.source_in_frame);
            out.push_str(&format!(
                "{:03}  {:<8} {:<5} C        {} {} {} {}\n",
                event_num,
                reel,
                track_code,
                frames_to_timecode(src_in, fps),
                frames_to_timecode(src_out, fps),
                frames_to_timecode(rec_offset + clip.start_frame, fps),
                frames_to_timecode(rec_offset + clip.end_frame, fps),
            ));
            emit_m2_and_comments(&mut out, clip, &reel, fps, src_in, options.include_clip_colors);
            out.push('\n');
            event_num += 1;
            total_a += 1;
        }
    }

    // Emit any sequence markers that didn't fall inside a V1 clip (using Avid * LOC: syntax)
    if options.include_markers && total_markers < seq.markers.len() {
        let v1_clips: Vec<(i64, i64)> = seq
            .video_tracks
            .first()
            .map(|t| {
                t.clips
                    .iter()
                    .filter(|c| c.enabled)
                    .map(|c| (c.start_frame, c.end_frame))
                    .collect()
            })
            .unwrap_or_default();
        for m in &seq.markers {
            let covered = v1_clips.iter().any(|&(s, e)| m.frame >= s && m.frame < e);
            if !covered {
                let text = if m.comment.is_empty() {
                    m.name.as_str()
                } else {
                    m.comment.as_str()
                };
                out.push_str(&format!(
                    "* LOC: {} {:<7} {}\n",
                    frames_to_timecode(rec_offset + m.frame.max(0), fps),
                    m.color.to_ascii_uppercase(),
                    text
                ));
                total_markers += 1;
            }
        }
    }

    if event_num > 1000 {
        messages.push(ConversionMessage {
            severity: Severity::Warning,
            code: "EDL_EVENT_LIMIT".into(),
            message: format!(
                "EDL contains {} events, exceeding the classic CMX 3600 999-event limit; some legacy tools may require splitting.",
                event_num - 1
            ),
            sequence: Some(seq.name.clone()),
            clip: None,
        });
    }

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, out)?;

    Ok(ConversionReport {
        output_path: output_path.to_string_lossy().into_owned(),
        source_format: project.source_format,
        target_format: NleFormat::Edl,
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

fn emit_m2_and_comments(
    out: &mut String,
    clip: &Clip,
    reel: &str,
    fps: Rational,
    src_in: i64,
    include_clip_colors: bool,
) {
    let is_reversed = clip.reversed || clip.speed_percent < 0.0;
    let abs_speed = clip.speed_percent.abs();
    if is_reversed || (abs_speed - 100.0).abs() > 0.05 {
        let mut eff_fps = fps.as_f64() * (abs_speed / 100.0);
        if is_reversed {
            eff_fps = -eff_fps;
        }
        out.push_str(&format!(
            "M2   {:<8} {:>05.1}           {}\n",
            reel,
            eff_fps,
            frames_to_timecode(src_in, fps)
        ));
    }
    out.push_str(&format!("* FROM CLIP NAME: {}\n", clip.name));
    if !clip.media_path.is_empty() && clip.media_path != clip.name {
        out.push_str(&format!("* SOURCE FILE: {}\n", clip.media_path));
    }
    if include_clip_colors
        && let Some(lbl) = clip.label_name.as_deref().filter(|s| !s.is_empty())
    {
        out.push_str(&format!("* RESOLVE COLOR: {lbl}\n"));
    }
}

fn derive_reel_name(clip: &Clip, asset: Option<&MediaAsset>) -> String {
    if let Some(tape) = asset
        .and_then(|a| a.metadata.get("Tape Name"))
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        let clean: String = tape
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(8)
            .collect();
        if !clean.is_empty() {
            return clean;
        }
    }
    let stem = Path::new(&clip.name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&clip.name);
    let clean: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(8)
        .collect();
    if clean.is_empty() {
        "AX".to_string()
    } else {
        clean
    }
}

fn capitalize_color(raw: &str) -> String {
    let t = raw.trim();
    let mut chars = t.chars();
    match chars.next() {
        Some(first) => format!(
            "{}{}",
            first.to_ascii_uppercase(),
            chars.as_str().to_ascii_lowercase()
        ),
        None => "Blue".to_string(),
    }
}
