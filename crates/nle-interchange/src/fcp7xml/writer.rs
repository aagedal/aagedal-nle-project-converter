use crate::fcpxml::time::{path_to_file_url, xml_escape};
use nle_core::{
    flatten_nested_and_multicam_clips, BlendMode, Clip, ClipSourceKind, ConversionMessage,
    ConversionOptions, ConversionReport, MediaAsset, NleError, NleFormat, NleResult, Project,
    ProjectBin, ProjectWriter, RetimeInterpolation, Sequence, SequenceKind,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
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
            "No sequences selected for FCP7 XML export".into(),
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

    let mut id_counter = 1usize;
    let mut file_ids: HashMap<String, String> = HashMap::new();
    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;
    let mut bins_created = 0usize;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n");
    xml.push_str("<xmeml version=\"5\">\n");

    if export_timelines.len() == 1 {
        let seq = export_timelines[0];
        let (v, a, m) = render_sequence_xml(
            seq,
            "  ",
            options,
            &assets_by_id,
            &assets_by_path,
            &mut file_ids,
            &mut id_counter,
            &mut xml,
        );
        total_v += v;
        total_a += a;
        total_markers += m;
    } else {
        let proj_name = options
            .project_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&project.name);
        xml.push_str("  <project>\n");
        xml.push_str(&format!("    <name>{}</name>\n", esc(proj_name)));
        xml.push_str("    <children>\n");

        let seq_by_id: HashMap<&str, &Sequence> = export_timelines
            .iter()
            .copied()
            .map(|s| (s.id.as_str(), s))
            .collect();
        let mut emitted_seq_ids: HashSet<&str> = HashSet::new();

        if options.include_bins && !project.bins.is_empty() {
            let mut children_by_parent: HashMap<Option<&str>, Vec<&ProjectBin>> = HashMap::new();
            let bin_ids: HashSet<&str> = project.bins.iter().map(|b| b.id.as_str()).collect();
            for bin in &project.bins {
                let parent_key = bin
                    .parent_id
                    .as_deref()
                    .filter(|pid| bin_ids.contains(pid));
                children_by_parent.entry(parent_key).or_default().push(bin);
            }
            if let Some(roots) = children_by_parent.get(&None) {
                for bin in roots {
                    render_bin_xml(
                        bin,
                        "      ",
                        &children_by_parent,
                        &seq_by_id,
                        &mut emitted_seq_ids,
                        options,
                        &assets_by_id,
                        &assets_by_path,
                        &mut file_ids,
                        &mut id_counter,
                        &mut total_v,
                        &mut total_a,
                        &mut total_markers,
                        &mut bins_created,
                        &mut xml,
                    );
                }
            }
        }

        for seq in &export_timelines {
            if emitted_seq_ids.insert(seq.id.as_str()) {
                let (v, a, m) = render_sequence_xml(
                    seq,
                    "      ",
                    options,
                    &assets_by_id,
                    &assets_by_path,
                    &mut file_ids,
                    &mut id_counter,
                    &mut xml,
                );
                total_v += v;
                total_a += a;
                total_markers += m;
            }
        }

        xml.push_str("    </children>\n  </project>\n");
    }

    xml.push_str("</xmeml>\n");

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
        sequences_converted: export_timelines.len(),
        video_clips_converted: total_v,
        audio_clips_converted: total_a,
        media_assets_linked: file_ids.len(),
        bins_created,
        markers_written: total_markers,
        grades_attached: 0,
        effect_markers_injected: 0,
        messages,
    })
}

fn bin_has_exportable_sequences<'a>(
    bin: &'a ProjectBin,
    children_by_parent: &HashMap<Option<&'a str>, Vec<&'a ProjectBin>>,
    seq_by_id: &HashMap<&str, &Sequence>,
) -> bool {
    if bin
        .sequence_ids
        .iter()
        .any(|sid| seq_by_id.contains_key(sid.as_str()))
    {
        return true;
    }
    if let Some(sub_bins) = children_by_parent.get(&Some(bin.id.as_str())) {
        return sub_bins
            .iter()
            .any(|sb| bin_has_exportable_sequences(sb, children_by_parent, seq_by_id));
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn render_bin_xml<'a>(
    bin: &'a ProjectBin,
    indent: &str,
    children_by_parent: &HashMap<Option<&'a str>, Vec<&'a ProjectBin>>,
    seq_by_id: &HashMap<&'a str, &'a Sequence>,
    emitted_seq_ids: &mut HashSet<&'a str>,
    options: &ConversionOptions,
    assets_by_id: &HashMap<&str, &MediaAsset>,
    assets_by_path: &HashMap<&str, &MediaAsset>,
    file_ids: &mut HashMap<String, String>,
    id_counter: &mut usize,
    total_v: &mut usize,
    total_a: &mut usize,
    total_markers: &mut usize,
    bins_created: &mut usize,
    xml: &mut String,
) {
    if !bin_has_exportable_sequences(bin, children_by_parent, seq_by_id) {
        return;
    }
    let bin_xml_id = format!("bin-{}", *id_counter);
    *id_counter += 1;
    *bins_created += 1;
    let child_indent = format!("{indent}    ");
    xml.push_str(&format!("{indent}<bin id=\"{bin_xml_id}\">\n"));
    xml.push_str(&format!("{indent}  <name>{}</name>\n", esc(&bin.name)));
    xml.push_str(&format!("{indent}  <children>\n"));

    if let Some(sub_bins) = children_by_parent.get(&Some(bin.id.as_str())) {
        for sb in sub_bins {
            render_bin_xml(
                sb,
                &child_indent,
                children_by_parent,
                seq_by_id,
                emitted_seq_ids,
                options,
                assets_by_id,
                assets_by_path,
                file_ids,
                id_counter,
                total_v,
                total_a,
                total_markers,
                bins_created,
                xml,
            );
        }
    }

    for sid in &bin.sequence_ids {
        if let Some(&seq) = seq_by_id.get(sid.as_str())
            && emitted_seq_ids.insert(seq.id.as_str())
        {
            let (v, a, m) = render_sequence_xml(
                seq,
                &child_indent,
                options,
                assets_by_id,
                assets_by_path,
                file_ids,
                id_counter,
                xml,
            );
            *total_v += v;
            *total_a += a;
            *total_markers += m;
        }
    }

    xml.push_str(&format!("{indent}  </children>\n{indent}</bin>\n"));
}

#[allow(clippy::too_many_arguments)]
fn render_sequence_xml(
    seq: &Sequence,
    indent: &str,
    options: &ConversionOptions,
    assets_by_id: &HashMap<&str, &MediaAsset>,
    assets_by_path: &HashMap<&str, &MediaAsset>,
    file_ids: &mut HashMap<String, String>,
    id_counter: &mut usize,
    xml: &mut String,
) -> (usize, usize, usize) {
    // Pre-assign deterministic clipitem IDs and build per-sequence link groups for <link> emission
    let seq_xml_id = format!("sequence-{}", *id_counter);
    *id_counter += 1;
    let mut video_clip_ids: Vec<Vec<String>> = Vec::with_capacity(seq.video_tracks.len());
    let mut audio_clip_ids: Vec<Vec<String>> = Vec::with_capacity(seq.audio_tracks.len());
    let mut link_groups: HashMap<String, Vec<(String, &'static str, usize, usize)>> =
        HashMap::new();

    for (ti, track) in seq.video_tracks.iter().enumerate() {
        let mut ids = Vec::with_capacity(track.clips.len());
        for (ci, clip) in track.clips.iter().enumerate() {
            let cid = format!("clipitem-{}", *id_counter);
            *id_counter += 1;
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
            let cid = format!("clipitem-{}", *id_counter);
            *id_counter += 1;
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

    xml.push_str(&format!("{indent}<sequence id=\"{seq_xml_id}\">\n"));
    xml.push_str(&format!("{indent}  <name>{}</name>\n", esc(&seq.name)));
    xml.push_str(&format!("{indent}  <duration>{dur}</duration>\n"));
    xml.push_str(&format!(
        "{indent}  <rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate>\n"
    ));
    if seq.start_tc_frame > 0 {
        let tc_str = nle_core::frames_to_timecode(seq.start_tc_frame, seq.frame_rate);
        xml.push_str(&format!(
            "{indent}  <timecode><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><string>{tc_str}</string><frame>{}</frame><displayformat>{}</displayformat></timecode>\n",
            seq.start_tc_frame,
            if seq.frame_rate.is_ntsc() { "DF" } else { "NDF" }
        ));
    }
    xml.push_str(&format!("{indent}  <media>\n{indent}    <video>\n"));
    xml.push_str(&format!(
        "{indent}      <format><samplecharacteristics><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><width>{}</width><height>{}</height><pixelaspectratio>square</pixelaspectratio></samplecharacteristics></format>\n",
        seq.width, seq.height
    ));

    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;
    let clip_marker_indent = format!("{indent}          ");

    for (ti, track) in seq.video_tracks.iter().enumerate() {
        xml.push_str(&format!("{indent}      <track>\n"));
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
                file_ids,
                id_counter,
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
                render_markers(&clip.markers, &clip_marker_indent)
            } else {
                String::new()
            };
            xml.push_str(&format!(
                "{indent}        <clipitem id=\"{ci_id}\"><name>{}</name><duration>{}</duration><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><start>{}</start><end>{}</end><in>{}</in><out>{}</out><enabled>{enabled_str}</enabled>{composite_xml}{file_xml}{links_xml}{filters_xml}{label_xml}{markers_xml}</clipitem>\n",
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
            "{indent}        <enabled>{}</enabled><locked>{}</locked>\n{indent}      </track>\n",
            if track.enabled { "TRUE" } else { "FALSE" },
            if track.locked { "TRUE" } else { "FALSE" }
        ));
    }
    xml.push_str(&format!("{indent}    </video>\n{indent}    <audio>\n"));

    for (ti, track) in seq.audio_tracks.iter().enumerate() {
        xml.push_str(&format!("{indent}      <track>\n"));
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
                file_ids,
                id_counter,
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
                render_markers(&clip.markers, &clip_marker_indent)
            } else {
                String::new()
            };
            xml.push_str(&format!(
                "{indent}        <clipitem id=\"{ci_id}\"><name>{}</name><duration>{}</duration><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><start>{}</start><end>{}</end><in>{}</in><out>{}</out><enabled>{enabled_str}</enabled>{file_xml}<sourcetrack><mediatype>audio</mediatype><trackindex>{}</trackindex></sourcetrack>{links_xml}{speed_xml}{label_xml}{markers_xml}</clipitem>\n",
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
            "{indent}        <enabled>{}</enabled><locked>{}</locked>\n{indent}      </track>\n",
            if track.enabled { "TRUE" } else { "FALSE" },
            if track.locked { "TRUE" } else { "FALSE" }
        ));
    }

    xml.push_str(&format!("{indent}    </audio>\n{indent}  </media>\n"));
    if options.include_markers {
        total_markers += seq.markers.len();
        let seq_marker_indent = format!("{indent}  ");
        xml.push_str(&render_markers(&seq.markers, &seq_marker_indent));
    }
    xml.push_str(&format!("{indent}</sequence>\n"));

    (total_v, total_a, total_markers)
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
    let has_ramp = clip.speed_keyframes.len() > 2;
    let has_blend = clip.retime_interpolation == RetimeInterpolation::FrameBlending;
    if !has_ramp && !is_reversed && (abs_speed - 100.0).abs() < 0.05 && !has_blend {
        return String::new();
    }
    let speed_val = if abs_speed < 0.01 { 0.0 } else { abs_speed };
    let rev_str = if is_reversed { "TRUE" } else { "FALSE" };
    let mut params = String::new();
    if has_ramp {
        params.push_str(
            "<parameter><name>variablespeed</name><parameterid>variablespeed</parameterid><value>1</value></parameter>",
        );
    }
    params.push_str(&format!(
        "<parameter><name>speed</name><parameterid>speed</parameterid><value>{speed_val:.2}</value></parameter><parameter><name>reverse</name><parameterid>reverse</parameterid><value>{rev_str}</value></parameter>"
    ));
    if has_blend {
        params.push_str(
            "<parameter><name>frameblending</name><parameterid>frameblending</parameterid><value>TRUE</value></parameter>",
        );
    }
    if has_ramp {
        let fps_f = clip.source_fps.as_f64().max(1.0);
        params.push_str("<parameter><name>graphdict</name><parameterid>graphdict</parameterid>");
        for kf in &clip.speed_keyframes {
            params.push_str(&format!(
                "<keyframe><when>{:.4}</when><value>{:.4}</value></keyframe>",
                kf.timeline_sec * fps_f,
                kf.source_sec * fps_f
            ));
        }
        params.push_str("</parameter>");
    }
    format!(
        "<filter><effect><name>Time Remap</name><effectid>timeremap</effectid><effectcategory>motion</effectcategory><effecttype>motion</effecttype><mediatype>video</mediatype>{params}</effect></filter>"
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
