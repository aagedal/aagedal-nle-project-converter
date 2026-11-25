use nle_core::{
    ConversionOptions, ConversionReport, NleError, NleFormat, NleResult, Project, ProjectWriter,
    Sequence,
};
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
pub fn write_fcp7_xml_file(
    project: &Project,
    output_path: &Path,
    options: &ConversionOptions,
) -> NleResult<ConversionReport> {
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
        .first()
        .ok_or_else(|| NleError::InvalidFormat("No sequences selected for FCP7 XML export".into()))?;

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

    for track in &seq.video_tracks {
        xml.push_str("        <track>\n");
        for clip in &track.clips {
            let ci_id = format!("clipitem-{id_counter}");
            id_counter += 1;
            let file_xml = render_file_ref(&clip.name, &clip.media_path, clip.source_out_frame.max(clip.duration_frames()), tb, ntsc, &mut file_ids, &mut id_counter);
            xml.push_str(&format!(
                "          <clipitem id=\"{ci_id}\"><name>{}</name><duration>{}</duration><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><start>{}</start><end>{}</end><in>{}</in><out>{}</out><enabled>TRUE</enabled>{file_xml}</clipitem>\n",
                esc(&clip.name),
                clip.duration_frames().max(1),
                clip.start_frame,
                clip.end_frame,
                clip.source_in_frame,
                clip.source_out_frame,
            ));
            total_v += 1;
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
            xml.push_str(&format!(
                "          <clipitem id=\"{ci_id}\"><name>{}</name><duration>{}</duration><rate><timebase>{tb}</timebase><ntsc>{ntsc}</ntsc></rate><start>{}</start><end>{}</end><in>{}</in><out>{}</out><enabled>TRUE</enabled>{file_xml}<sourcetrack><mediatype>audio</mediatype><trackindex>{}</trackindex></sourcetrack></clipitem>\n",
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
        xml.push_str(&format!(
            "          <enabled>{}</enabled><locked>{}</locked>\n        </track>\n",
            if track.enabled { "TRUE" } else { "FALSE" },
            if track.locked { "TRUE" } else { "FALSE" }
        ));
    }

    xml.push_str("      </audio>\n    </media>\n");
    for m in &seq.markers {
        xml.push_str(&format!(
            "    <marker><name>{}</name><comment>{}</comment><in>{}</in><out>{}</out></marker>\n",
            esc(&m.name),
            esc(&m.comment),
            m.frame,
            m.frame + m.duration_frames.max(1)
        ));
    }
    xml.push_str("  </sequence>\n</xmeml>\n");

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
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
        markers_written: seq.markers.len(),
        grades_attached: 0,
        effect_markers_injected: 0,
        messages: Vec::new(),
    })
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
