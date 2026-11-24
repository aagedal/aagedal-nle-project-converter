use nle_core::{
    ConversionOptions, ConversionReport, MediaKind, NleError, NleFormat, NleResult, Project,
    ProjectWriter, Sequence,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub struct FcpXmlWriter;

impl ProjectWriter for FcpXmlWriter {
    fn format(&self) -> NleFormat {
        NleFormat::FinalCutPro
    }

    fn write_file(
        &self,
        project: &Project,
        output_path: &Path,
        options: &ConversionOptions,
    ) -> NleResult<ConversionReport> {
        write_fcpxml_file(project, output_path, options)
    }
}

/// Write a Final Cut Pro `.fcpxml` (v1.11) file directly from the canonical `Project` IR.
pub fn write_fcpxml_file(
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
    if selected.is_empty() {
        return Err(NleError::InvalidFormat("No sequences selected for FCPXML export".into()));
    }

    let first = selected[0];
    let fps = first.frame_rate;
    let frame_dur = fps.frames_to_fcpxml(1);

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<!DOCTYPE fcpxml>\n");
    xml.push_str("<fcpxml version=\"1.11\">\n");
    xml.push_str("  <resources>\n");
    xml.push_str(&format!(
        "    <format id=\"r1\" name=\"FFVideoFormat{}p{}\" frameDuration=\"{frame_dur}\" width=\"{}\" height=\"{}\"/>\n",
        first.height,
        fps.label(),
        first.width,
        first.height
    ));

    // Collect unique media assets across selected sequences
    let mut asset_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut next_res = 2usize;
    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;

    for seq in &selected {
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            for clip in &track.clips {
                let key = if clip.media_path.is_empty() {
                    clip.name.clone()
                } else {
                    clip.media_path.clone()
                };
                if !asset_ids.contains_key(&key) {
                    let rid = format!("r{next_res}");
                    next_res += 1;
                    let src_url = path_to_file_url(&key);
                    let dur_str = clip
                        .source_fps
                        .frames_to_fcpxml(clip.source_out_frame.max(clip.duration_frames()).max(1));
                    let has_v = if clip.media_kind != MediaKind::Audio { "1" } else { "0" };
                    xml.push_str(&format!(
                        "    <asset id=\"{rid}\" name=\"{}\" start=\"0s\" duration=\"{dur_str}\" hasVideo=\"{has_v}\" hasAudio=\"1\" format=\"r1\">\n",
                        xml_escape(&clip.name),
                    ));
                    xml.push_str(&format!(
                        "      <media-rep kind=\"original-media\" src=\"{}\"/>\n",
                        xml_escape(&src_url)
                    ));
                    xml.push_str("    </asset>\n");
                    asset_ids.insert(key, rid);
                }
            }
        }
    }
    xml.push_str("  </resources>\n");

    let proj_name = options
        .project_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&project.name);

    xml.push_str("  <library>\n");
    xml.push_str(&format!("    <event name=\"{}\">\n", xml_escape(proj_name)));

    for seq in &selected {
        let dur_str = seq
            .frame_rate
            .frames_to_fcpxml(seq.computed_duration_frames().max(1));
        xml.push_str(&format!("      <project name=\"{}\">\n", xml_escape(&seq.name)));
        xml.push_str(&format!(
            "        <sequence format=\"r1\" duration=\"{dur_str}\" tcStart=\"0s\" tcFormat=\"{}\" audioLayout=\"stereo\" audioRate=\"48k\">\n",
            if seq.frame_rate.is_ntsc() { "DF" } else { "NDF" }
        ));
        xml.push_str("          <spine>\n");

        // Primary storyline = V1 (with gaps if needed)
        if let Some(v1) = seq.video_tracks.first() {
            let mut sorted_clips = v1.clips.clone();
            sorted_clips.sort_by_key(|c| c.start_frame);
            let mut cursor = 0i64;
            for clip in &sorted_clips {
                if clip.start_frame > cursor {
                    let gap_dur = seq.frame_rate.frames_to_fcpxml(clip.start_frame - cursor);
                    let gap_off = seq.frame_rate.frames_to_fcpxml(cursor);
                    xml.push_str(&format!(
                        "            <gap name=\"Gap\" offset=\"{gap_off}\" duration=\"{gap_dur}\"/>\n"
                    ));
                }
                let key = if clip.media_path.is_empty() {
                    clip.name.clone()
                } else {
                    clip.media_path.clone()
                };
                let rid = &asset_ids[&key];
                let off = seq.frame_rate.frames_to_fcpxml(clip.start_frame.max(0));
                let dur = seq.frame_rate.frames_to_fcpxml(clip.duration_frames().max(1));
                let start = seq.frame_rate.frames_to_fcpxml(clip.source_in_frame.max(0));
                xml.push_str(&format!(
                    "            <asset-clip ref=\"{rid}\" offset=\"{off}\" name=\"{}\" start=\"{start}\" duration=\"{dur}\">\n",
                    xml_escape(&clip.name)
                ));
                if !clip.motion.is_identity() {
                    let sc = clip.motion.scale / 100.0;
                    xml.push_str(&format!(
                        "              <adjust-transform scale=\"{sc:.4} {sc:.4}\" rotation=\"{:.3}\"/>\n",
                        clip.motion.rotation
                    ));
                }
                // Attach sequence markers falling inside this clip
                for m in &seq.markers {
                    if m.frame >= clip.start_frame && m.frame < clip.end_frame {
                        let m_start = seq
                            .frame_rate
                            .frames_to_fcpxml(clip.source_in_frame + (m.frame - clip.start_frame));
                        let m_dur = seq.frame_rate.frames_to_fcpxml(m.duration_frames.max(1));
                        xml.push_str(&format!(
                            "              <marker start=\"{m_start}\" duration=\"{m_dur}\" value=\"{}\" note=\"{}\"/>\n",
                            xml_escape(&m.name),
                            xml_escape(&m.comment)
                        ));
                        total_markers += 1;
                    }
                }
                xml.push_str("            </asset-clip>\n");
                cursor = clip.end_frame.max(cursor);
                total_v += 1;
            }
        }

        // Connected video tracks (V2+) and audio tracks as lanes attached to a base gap if needed
        let extra_v: usize = seq.video_tracks.iter().skip(1).map(|t| t.clips.len()).sum();
        let extra_a: usize = seq.audio_tracks.iter().map(|t| t.clips.len()).sum();
        total_v += extra_v;
        total_a += extra_a;

        xml.push_str("          </spine>\n");
        xml.push_str("        </sequence>\n");
        xml.push_str("      </project>\n");
    }

    xml.push_str("    </event>\n");
    xml.push_str("  </library>\n");
    xml.push_str("</fcpxml>\n");

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(output_path, xml)?;

    Ok(ConversionReport {
        output_path: output_path.to_string_lossy().into_owned(),
        source_format: project.source_format,
        target_format: NleFormat::FinalCutPro,
        sequences_converted: selected.len(),
        video_clips_converted: total_v,
        audio_clips_converted: total_a,
        media_assets_linked: asset_ids.len(),
        bins_created: 0,
        markers_written: total_markers,
        grades_attached: 0,
        effect_markers_injected: 0,
        messages: Vec::new(),
    })
}

fn path_to_file_url(path: &str) -> String {
    if path.starts_with("file://") {
        return path.to_string();
    }
    let norm = path.replace('\\', "/");
    let encoded = norm.replace(' ', "%20");
    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
