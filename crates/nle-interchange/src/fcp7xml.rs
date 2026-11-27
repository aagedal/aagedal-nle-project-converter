use nle_core::{
    flatten_nested_and_multicam_clips, ClipSourceKind, ConversionMessage, ConversionOptions,
    ConversionReport, NleError, NleFormat, NleResult, Project, ProjectWriter, Sequence,
    SequenceKind, Severity,
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

    let tb = seq.frame_rate.timebase();
    let ntsc = if seq.frame_rate.is_ntsc() { "TRUE" } else { "FALSE" };
    let dur = seq.computed_duration_frames();

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n");
    xml.push_str("<xmeml version=\"5\">\n  <sequence>\n");
    xml.push_str(&format!("    <name>{}</name>\n", esc(&seq.name)));
    xml.push_str(&format!("    <duration>{dur}</duration>\n"));
    xml.push_str(&format!("    <rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate>\n"));
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
    let mut id_counter = 1usize;
    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;

    for track in &seq.video_tracks {
        xml.push_str("        <track>\n");
        for clip in &track.clips {
            let ci_id = format!("clipitem-{id_counter}");
            id_counter += 1;
            let file_xml = render_file_ref(&clip.name, &clip.media_path, clip.source_out_frame.max(clip.duration_frames()), tb, ntsc, &mut file_ids, &mut id_counter);
            let enabled_str = if clip.enabled { "TRUE" } else { "FALSE" };
            let speed_xml = render_speed_filter(clip);
            let markers_xml = if options.include_markers {
                total_markers += clip.markers.len();
                render_markers(&clip.markers, "            ")
            } else {
                String::new()
            };
            xml.push_str(&format!(
                "          <clipitem id=\"{ci_id}\"><name>{}</name><duration>{}</duration><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><start>{}</start><end>{}</end><in>{}</in><out>{}</out><enabled>{enabled_str}</enabled>{file_xml}{speed_xml}{markers_xml}</clipitem>\n",
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

    for track in &seq.audio_tracks {
        xml.push_str("        <track>\n");
        for clip in &track.clips {
            let ci_id = format!("clipitem-{id_counter}");
            id_counter += 1;
            let file_xml = render_file_ref(&clip.name, &clip.media_path, clip.source_out_frame.max(clip.duration_frames()), tb, ntsc, &mut file_ids, &mut id_counter);
            let enabled_str = if clip.enabled { "TRUE" } else { "FALSE" };
            let speed_xml = render_speed_filter(clip);
            let markers_xml = if options.include_markers {
                total_markers += clip.markers.len();
                render_markers(&clip.markers, "            ")
            } else {
                String::new()
            };
            xml.push_str(&format!(
                "          <clipitem id=\"{ci_id}\"><name>{}</name><duration>{}</duration><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><start>{}</start><end>{}</end><in>{}</in><out>{}</out><enabled>{enabled_str}</enabled>{file_xml}<sourcetrack><mediatype>audio</mediatype><trackindex>{}</trackindex></sourcetrack>{speed_xml}{markers_xml}</clipitem>\n",
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
        && !parent.as_os_str().is_empty() {
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

fn render_markers(markers: &[nle_core::Marker], indent: &str) -> String {
    let mut out = String::new();
    for m in markers {
        out.push_str(&format!(
            "{indent}<marker><name>{}</name><comment>{}</comment><in>{}</in><out>{}</out></marker>\n",
            esc(&m.name),
            esc(&m.comment),
            m.frame.max(0),
            m.frame.max(0) + m.duration_frames.max(1)
        ));
    }
    out
}

fn render_speed_filter(clip: &nle_core::Clip) -> String {
    let is_reversed = clip.reversed || clip.speed_percent < 0.0;
    let abs_speed = clip.speed_percent.abs();
    if !is_reversed && (abs_speed - 100.0).abs() < 0.05 {
        return String::new();
    }
    let speed_val = if abs_speed < 0.01 { 0.5 } else { abs_speed };
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
        ("Cross Fade (+3dB)", "KGAudioTransCrossFade3dB", "Audio Transitions", "audio")
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

fn render_file_ref(
    name: &str,
    path: &str,
    duration: i64,
    tb: i64,
    ntsc: &str,
    file_ids: &mut HashMap<String, String>,
    counter: &mut usize,
) -> String {
    let key = if path.is_empty() { name.to_string() } else { path.to_string() };
    if let Some(existing) = file_ids.get(&key) {
        return format!("<file id=\"{existing}\"/>");
    }
    let fid = format!("file-{}", *counter);
    *counter += 1;
    file_ids.insert(key, fid.clone());
    let url = format!("file://localhost/{}", path.trim_start_matches('/').replace(' ', "%20"));
    format!(
        "<file id=\"{fid}\"><name>{}</name><pathurl>{}</pathurl><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><duration>{duration}</duration></file>",
        esc(name),
        esc(&url)
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::Rational;

    #[test]
    fn warns_when_multiple_timelines_are_selected() {
        let dir = std::env::temp_dir().join("aagedal_fcp7_multi_seq_test");
        let _ = fs::remove_dir_all(&dir);
        let out = dir.join("multi.xml");

        let mk_seq = |id: &str, name: &str| Sequence {
            id: id.into(),
            name: name.into(),
            kind: SequenceKind::Timeline,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 0,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: Vec::new(),
            audio_tracks: Vec::new(),
            audio_buses: Vec::new(),
            markers: Vec::new(),
            metadata: std::collections::BTreeMap::new(),
        };
        let proj = Project {
            name: "Multi".into(),
            source_format: NleFormat::DaVinciResolve,
            sequences: vec![mk_seq("s1", "Reel_1"), mk_seq("s2", "Reel_2")],
            ..Default::default()
        };

        let report = write_fcp7_xml_file(&proj, &out, &ConversionOptions::default()).unwrap();
        assert_eq!(report.sequences_converted, 1);
        assert!(
            report
                .messages
                .iter()
                .any(|m| m.code == "FCP7_SINGLE_SEQUENCE" && m.severity == Severity::Warning),
            "expected FCP7_SINGLE_SEQUENCE warning, got: {:?}",
            report.messages
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
