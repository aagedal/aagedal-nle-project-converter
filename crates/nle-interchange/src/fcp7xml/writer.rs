use crate::fcpxml::time::{path_to_file_url, xml_escape};
use nle_core::{
    flatten_nested_and_multicam_clips, BlendMode, Clip, ClipSourceKind, ConversionMessage,
    ConversionOptions, ConversionReport, MediaAsset, NleError, NleFormat, NleResult, Project,
    ProjectWriter, Sequence, SequenceKind, Severity,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct Fcp7XmlWriter;

impl ProjectWriter for Fcp7XmlWriter {
    fn format(&self) -> NleFormat {
        NleFormat::Fcp7Xml
    }

    fn write_file(
        &self,
        project: &Project,
        output_path: &Path,
        options: &ConversionOptions,
    ) -> NleResult<ConversionReport> {
        write_fcp7_xml_file(project, output_path, options)
    }
}

/// Write an FCP7 XMEML v5 `.xml` file (importable directly into Adobe Premiere Pro & DaVinci Resolve).
/// Automatically flattens nested/compound/multicam clips into source media clips so every `<file>`
/// reference points to real footage.
pub fn write_fcp7_xml_file(
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
        .ok_or_else(|| NleError::InvalidFormat("No sequences selected for FCP7 XML export".into()))?;

    let skipped_timelines: Vec<&str> = selected
        .iter()
        .filter(|s| s.id != seq.id && s.kind == SequenceKind::Timeline)
        .map(|s| s.name.as_str())
        .collect();
    if !skipped_timelines.is_empty() {
        let preview = skipped_timelines
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        let extra = if skipped_timelines.len() > 3 {
            format!(" (+{} more)", skipped_timelines.len() - 3)
        } else {
            String::new()
        };
        messages.push(ConversionMessage {
            severity: Severity::Warning,
            code: "FCP7_SINGLE_SEQUENCE".into(),
            message: format!(
                "FCP7 XML (.xml) export writes one timeline per file: exported '{}'; skipped {} other selected timeline(s): {preview}{extra}.",
                seq.name,
                skipped_timelines.len()
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

    // Pre-assign deterministic clipitem IDs and build link groups for <link> emission
    let mut id_counter = 1usize;
    let mut video_clip_ids: Vec<Vec<String>> = Vec::with_capacity(seq.video_tracks.len());
    let mut audio_clip_ids: Vec<Vec<String>> = Vec::with_capacity(seq.audio_tracks.len());
    let mut link_groups: HashMap<String, Vec<(String, &'static str, usize, usize)>> =
        HashMap::new();

    for (ti, track) in seq.video_tracks.iter().enumerate() {
        let mut ids = Vec::with_capacity(track.clips.len());
        for (ci, clip) in track.clips.iter().enumerate() {
            let cid = format!("clipitem-{id_counter}");
            id_counter += 1;
            if let Some(lg) = clip.link_group_id.as_deref().filter(|s| !s.is_empty()) {
                link_groups
                    .entry(lg.to_string())
                    .or_default()
                    .push((cid.clone(), "video", ti + 1, ci + 1));
            }
            ids.push(cid);
        }
        video_clip_ids.push(ids);
    }
    for (ti, track) in seq.audio_tracks.iter().enumerate() {
        let mut ids = Vec::with_capacity(track.clips.len());
        for (ci, clip) in track.clips.iter().enumerate() {
            let cid = format!("clipitem-{id_counter}");
            id_counter += 1;
            if let Some(lg) = clip.link_group_id.as_deref().filter(|s| !s.is_empty()) {
                link_groups
                    .entry(lg.to_string())
                    .or_default()
                    .push((cid.clone(), "audio", ti + 1, ci + 1));
            }
            ids.push(cid);
        }
        audio_clip_ids.push(ids);
    }

    let tb = seq.frame_rate.timebase();
    let ntsc = if seq.frame_rate.is_ntsc() { "TRUE" } else { "FALSE" };
    let dur = seq.computed_duration_frames();

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n");
    xml.push_str("<xmeml version=\"5\">\n  <sequence>\n");
    xml.push_str(&format!("    <name>{}</name>\n", esc(&seq.name)));
    xml.push_str(&format!("    <duration>{dur}</duration>\n"));
    xml.push_str(&format!(
        "    <rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate>\n"
    ));
    if seq.start_tc_frame > 0 {
        let tc_str = nle_core::frames_to_timecode(seq.start_tc_frame, seq.frame_rate);
        xml.push_str(&format!(
            "    <timecode><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><string>{tc_str}</string><frame>{}</frame><displayformat>{}</displayformat></timecode>\n",
            seq.start_tc_frame,
            if seq.frame_rate.is_ntsc() { "DF" } else { "NDF" }
        ));
    }
    xml.push_str("    <media>\n      <video>\n");
    xml.push_str(&format!(
        "        <format><samplecharacteristics><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><width>{}</width><height>{}</height><pixelaspectratio>square</pixelaspectratio></samplecharacteristics></format>\n",
        seq.width, seq.height
    ));

    let mut file_ids: HashMap<String, String> = HashMap::new();
    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;

    for (ti, track) in seq.video_tracks.iter().enumerate() {
        xml.push_str("        <track>\n");
        for (ci, clip) in track.clips.iter().enumerate() {
            let ci_id = &video_clip_ids[ti][ci];
            let asset = assets_by_id
                .get(clip.media_id.as_str())
                .or_else(|| assets_by_path.get(clip.media_path.as_str()))
                .copied();
            let file_xml = render_file_ref(
                &clip.name,
                &clip.media_path,
                clip.source_out_frame.max(clip.duration_frames()),
                tb,
                ntsc,
                asset,
                &mut file_ids,
                &mut id_counter,
            );
            let enabled_str = if clip.enabled { "TRUE" } else { "FALSE" };
            let composite_xml = if clip.blend_mode != BlendMode::Normal {
                format!(
                    "<compositemode>{}</compositemode>",
                    esc(&clip.blend_mode.display_name().to_ascii_lowercase())
                )
            } else {
                String::new()
            };
            let label_xml = if options.include_clip_colors {
                clip.label_name
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|lbl| format!("<labels><label2>{}</label2></labels>", esc(lbl)))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let links_xml = render_links(clip, &link_groups);
            let filters_xml = format!(
                "{}{}{}{}",
                render_motion_filter(clip),
                render_crop_filter(clip),
                render_opacity_filter(clip),
                render_speed_filter(clip),
            );
            let markers_xml = if options.include_markers {
                total_markers += clip.markers.len();
                render_markers(&clip.markers, "            ")
            } else {
                String::new()
            };
            xml.push_str(&format!(
                "          <clipitem id=\"{ci_id}\"><name>{}</name><duration>{}</duration><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><start>{}</start><end>{}</end><in>{}</in><out>{}</out><enabled>{enabled_str}</enabled>{composite_xml}{file_xml}{links_xml}{filters_xml}{label_xml}{markers_xml}</clipitem>\n",
                esc(&clip.name),
                clip.duration_frames().max(1),
                clip.start_frame,
                clip.end_frame,
                clip.source_in_frame,
                clip.source_out_frame,
            ));
            total_v += 1;
        }
        for tr in &track.transitions {
            xml.push_str(&render_transition_item(tr, tb, ntsc, false));
        }
        xml.push_str(&format!(
            "          <enabled>{}</enabled><locked>{}</locked>\n        </track>\n",
            if track.enabled { "TRUE" } else { "FALSE" },
            if track.locked { "TRUE" } else { "FALSE" }
        ));
    }
    xml.push_str("      </video>\n      <audio>\n");

    for (ti, track) in seq.audio_tracks.iter().enumerate() {
        xml.push_str("        <track>\n");
        for (ci, clip) in track.clips.iter().enumerate() {
            let ci_id = &audio_clip_ids[ti][ci];
            let asset = assets_by_id
                .get(clip.media_id.as_str())
                .or_else(|| assets_by_path.get(clip.media_path.as_str()))
                .copied();
            let file_xml = render_file_ref(
                &clip.name,
                &clip.media_path,
                clip.source_out_frame.max(clip.duration_frames()),
                tb,
                ntsc,
                asset,
                &mut file_ids,
                &mut id_counter,
            );
            let enabled_str = if clip.enabled { "TRUE" } else { "FALSE" };
            let links_xml = render_links(clip, &link_groups);
            let speed_xml = render_speed_filter(clip);
            let label_xml = if options.include_clip_colors {
                clip.label_name
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|lbl| format!("<labels><label2>{}</label2></labels>", esc(lbl)))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let markers_xml = if options.include_markers {
                total_markers += clip.markers.len();
                render_markers(&clip.markers, "            ")
            } else {
                String::new()
            };
            xml.push_str(&format!(
                "          <clipitem id=\"{ci_id}\"><name>{}</name><duration>{}</duration><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><start>{}</start><end>{}</end><in>{}</in><out>{}</out><enabled>{enabled_str}</enabled>{file_xml}<sourcetrack><mediatype>audio</mediatype><trackindex>{}</trackindex></sourcetrack>{links_xml}{speed_xml}{label_xml}{markers_xml}</clipitem>\n",
                esc(&clip.name),
                clip.duration_frames().max(1),
                clip.start_frame,
                clip.end_frame,
                clip.source_in_frame,
                clip.source_out_frame,
                clip.audio_channel.max(1)
            ));
            total_a += 1;
        }
        for tr in &track.transitions {
            xml.push_str(&render_transition_item(tr, tb, ntsc, true));
        }
        xml.push_str(&format!(
            "          <enabled>{}</enabled><locked>{}</locked>\n        </track>\n",
            if track.enabled { "TRUE" } else { "FALSE" },
            if track.locked { "TRUE" } else { "FALSE" }
        ));
    }

    xml.push_str("      </audio>\n    </media>\n");
    if options.include_markers {
        total_markers += seq.markers.len();
        xml.push_str(&render_markers(&seq.markers, "    "));
    }
    xml.push_str("  </sequence>\n</xmeml>\n");

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, xml)?;

    Ok(ConversionReport {
        output_path: output_path.to_string_lossy().into_owned(),
        source_format: project.source_format,
        target_format: NleFormat::Fcp7Xml,
        sequences_converted: 1,
        video_clips_converted: total_v,
        audio_clips_converted: total_a,
        media_assets_linked: file_ids.len(),
        bins_created: 0,
        markers_written: total_markers,
        grades_attached: 0,
        effect_markers_injected: 0,
        messages,
    })
}

fn render_links(
    clip: &Clip,
    link_groups: &HashMap<String, Vec<(String, &'static str, usize, usize)>>,
) -> String {
    let Some(lg) = clip.link_group_id.as_deref() else {
        return String::new();
    };
    let Some(members) = link_groups.get(lg) else {
        return String::new();
    };
    if members.len() < 2 {
        return String::new();
    }
    let mut out = String::new();
    for (cid, mtype, track_idx, clip_idx) in members {
        out.push_str(&format!(
            "<link><linkclipref>{cid}</linkclipref><mediatype>{mtype}</mediatype><trackindex>{track_idx}</trackindex><clipindex>{clip_idx}</clipindex></link>"
        ));
    }
    out
}

fn render_markers(markers: &[nle_core::Marker], indent: &str) -> String {
    let mut out = String::new();
    for m in markers {
        let (r, g, b) = marker_color_to_rgb(&m.color);
        out.push_str(&format!(
            "{indent}<marker><name>{}</name><comment>{}</comment><in>{}</in><out>{}</out><color><alpha>0</alpha><red>{r}</red><green>{g}</green><blue>{b}</blue></color></marker>\n",
            esc(&m.name),
            esc(&m.comment),
            m.frame.max(0),
            m.frame.max(0) + m.duration_frames.max(1)
        ));
    }
    out
}

fn marker_color_to_rgb(color: &str) -> (u8, u8, u8) {
    match color.trim().to_ascii_lowercase().as_str() {
        "red" | "rose" => (255, 45, 45),
        "green" | "mint" => (45, 200, 75),
        "cyan" | "sky" | "teal" => (45, 205, 235),
        "yellow" | "lemon" => (250, 215, 40),
        "orange" | "sand" => (255, 145, 35),
        "purple" | "lavender" | "fuchsia" => (170, 80, 235),
        "pink" | "magenta" => (240, 85, 180),
        "white" | "cream" => (240, 240, 240),
        _ => (55, 130, 245), // Blue default
    }
}

fn render_motion_filter(clip: &Clip) -> String {
    let m = &clip.motion;
    let has_basic = (m.scale - 100.0).abs() > 1e-4
        || m.position_x.abs() > 1e-4
        || m.position_y.abs() > 1e-4
        || m.rotation.abs() > 1e-4
        || m.anchor_x.abs() > 1e-4
        || m.anchor_y.abs() > 1e-4;
    if !has_basic {
        return String::new();
    }
    let mut params = String::new();
    if (m.scale - 100.0).abs() > 1e-4 {
        params.push_str(&format!(
            "<parameter><name>Scale</name><parameterid>scale</parameterid><value>{:.4}</value></parameter>",
            m.scale
        ));
    }
    if m.rotation.abs() > 1e-4 {
        params.push_str(&format!(
            "<parameter><name>Rotation</name><parameterid>rotation</parameterid><value>{:.4}</value></parameter>",
            m.rotation
        ));
    }
    if m.position_x.abs() > 1e-4 || m.position_y.abs() > 1e-4 {
        // FCP7 XML center: +horiz right, +vert down (IR position_y is +up)
        params.push_str(&format!(
            "<parameter><name>Center</name><parameterid>center</parameterid><value><horiz>{:.6}</horiz><vert>{:.6}</vert></value></parameter>",
            m.position_x,
            -m.position_y
        ));
    }
    if m.anchor_x.abs() > 1e-4 || m.anchor_y.abs() > 1e-4 {
        params.push_str(&format!(
            "<parameter><name>Anchor Point</name><parameterid>anchorpoint</parameterid><value><horiz>{:.6}</horiz><vert>{:.6}</vert></value></parameter>",
            m.anchor_x, m.anchor_y
        ));
    }
    format!(
        "<filter><effect><name>Basic Motion</name><effectid>basic</effectid><effectcategory>motion</effectcategory><effecttype>motion</effecttype><mediatype>video</mediatype>{params}</effect></filter>"
    )
}

fn render_crop_filter(clip: &Clip) -> String {
    let m = &clip.motion;
    if m.crop_left.abs() < 1e-4
        && m.crop_right.abs() < 1e-4
        && m.crop_top.abs() < 1e-4
        && m.crop_bottom.abs() < 1e-4
    {
        return String::new();
    }
    format!(
        "<filter><effect><name>Crop</name><effectid>crop</effectid><effectcategory>motion</effectcategory><effecttype>motion</effecttype><mediatype>video</mediatype><parameter><name>left</name><parameterid>left</parameterid><value>{:.4}</value></parameter><parameter><name>right</name><parameterid>right</parameterid><value>{:.4}</value></parameter><parameter><name>top</name><parameterid>top</parameterid><value>{:.4}</value></parameter><parameter><name>bottom</name><parameterid>bottom</parameterid><value>{:.4}</value></parameter></effect></filter>",
        m.crop_left, m.crop_right, m.crop_top, m.crop_bottom
    )
}

fn render_opacity_filter(clip: &Clip) -> String {
    if (clip.opacity - 100.0).abs() < 1e-4 {
        return String::new();
    }
    format!(
        "<filter><effect><name>Opacity</name><effectid>opacity</effectid><effectcategory>motion</effectcategory><effecttype>motion</effecttype><mediatype>video</mediatype><parameter><name>opacity</name><parameterid>opacity</parameterid><value>{:.4}</value></parameter></effect></filter>",
        clip.opacity.clamp(0.0, 100.0)
    )
}

fn render_speed_filter(clip: &Clip) -> String {
    let is_reversed = clip.reversed || clip.speed_percent < 0.0;
    let abs_speed = clip.speed_percent.abs();
    if !is_reversed && (abs_speed - 100.0).abs() < 0.05 {
        return String::new();
    }
    let speed_val = if abs_speed < 0.01 { 0.0 } else { abs_speed };
    let rev_str = if is_reversed { "TRUE" } else { "FALSE" };
    format!(
        "<filter><effect><name>Time Remap</name><effectid>timeremap</effectid><effectcategory>motion</effectcategory><effecttype>motion</effecttype><mediatype>video</mediatype><parameter><name>speed</name><parameterid>speed</parameterid><value>{speed_val:.2}</value></parameter><parameter><name>reverse</name><parameterid>reverse</parameterid><value>{rev_str}</value></parameter></effect></filter>"
    )
}

fn render_transition_item(tr: &nle_core::Transition, tb: i64, ntsc: &str, is_audio: bool) -> String {
    let dur = tr.end_frame - tr.start_frame;
    if dur <= 0 {
        return String::new();
    }
    let align = match tr.alignment.as_str() {
        "start" => "start",
        "end" => "end",
        _ => "center",
    };
    let (eff_name, eff_id, eff_cat, media_type) = if is_audio {
        (
            "Cross Fade (+3dB)",
            "KGAudioTransCrossFade3dB",
            "Audio Transitions",
            "audio",
        )
    } else {
        ("Cross Dissolve", "Cross Dissolve", "Dissolve", "video")
    };
    format!(
        "          <transitionitem><start>{}</start><end>{}</end><alignment>{align}</alignment><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><effect><name>{}</name><effectid>{eff_id}</effectid><effectcategory>{eff_cat}</effectcategory><effecttype>transition</effecttype><mediatype>{media_type}</mediatype></effect></transitionitem>\n",
        tr.start_frame.max(0),
        tr.end_frame.max(tr.start_frame + 1),
        esc(if tr.name.is_empty() { eff_name } else { &tr.name }),
    )
}

#[allow(clippy::too_many_arguments)]
fn render_file_ref(
    name: &str,
    path: &str,
    duration: i64,
    tb: i64,
    ntsc: &str,
    asset: Option<&MediaAsset>,
    file_ids: &mut HashMap<String, String>,
    counter: &mut usize,
) -> String {
    let key = if path.is_empty() {
        name.to_string()
    } else {
        path.to_string()
    };
    if let Some(existing) = file_ids.get(&key) {
        return format!("<file id=\"{existing}\"/>");
    }
    let fid = format!("file-{}", *counter);
    *counter += 1;
    file_ids.insert(key, fid.clone());

    let url = if path.is_empty() {
        String::new()
    } else {
        let std_url = path_to_file_url(path);
        if let Some(rest) = std_url.strip_prefix("file://") {
            format!("file://localhost{rest}")
        } else {
            std_url
        }
    };

    let file_tb = asset.map(|a| a.frame_rate.timebase()).unwrap_or(tb);
    let file_ntsc = asset
        .map(|a| if a.frame_rate.is_ntsc() { "TRUE" } else { "FALSE" })
        .unwrap_or(ntsc);
    let file_dur = asset
        .map(|a| a.duration_frames.max(duration))
        .unwrap_or(duration)
        .max(1);

    let tc_xml = asset
        .filter(|a| a.source_tc_frame > 0)
        .map(|a| {
            let tc_str = nle_core::frames_to_timecode(a.source_tc_frame, a.frame_rate);
            let df = if a.frame_rate.is_ntsc() { "DF" } else { "NDF" };
            format!(
                "<timecode><rate><timebase>{file_tb}</timebase><ntsc>{file_ntsc}</ntsc></rate><string>{tc_str}</string><frame>{}</frame><displayformat>{df}</displayformat></timecode>",
                a.source_tc_frame
            )
        })
        .unwrap_or_default();

    let media_xml = asset
        .filter(|a| a.width > 0 || a.audio_channels > 0)
        .map(|a| {
            let mut m = String::from("<media>");
            if a.width > 0 && a.height > 0 {
                m.push_str(&format!(
                    "<video><samplecharacteristics><rate><timebase>{file_tb}</timebase><ntsc>{file_ntsc}</ntsc></rate><width>{}</width><height>{}</height></samplecharacteristics></video>",
                    a.width, a.height
                ));
            }
            if a.audio_channels > 0 {
                let sr = if a.audio_sample_rate > 0 {
                    a.audio_sample_rate
                } else {
                    48_000
                };
                m.push_str(&format!(
                    "<audio><samplecharacteristics><samplerate>{sr}</samplerate></samplecharacteristics><channelcount>{}</channelcount></audio>",
                    a.audio_channels
                ));
            }
            m.push_str("</media>");
            m
        })
        .unwrap_or_default();

    let pathurl_xml = if url.is_empty() {
        String::new()
    } else {
        format!("<pathurl>{}</pathurl>", esc(&url))
    };

    format!(
        "<file id=\"{fid}\"><name>{}</name>{pathurl_xml}<rate><timebase>{file_tb}</timebase><ntsc>{file_ntsc}</ntsc></rate><duration>{file_dur}</duration>{tc_xml}{media_xml}</file>",
        esc(name),
    )
}

fn esc(s: &str) -> String {
    xml_escape(s)
}
