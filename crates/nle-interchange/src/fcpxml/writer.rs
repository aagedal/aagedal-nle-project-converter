use super::time::{path_to_file_url, xml_escape, FcpTime};
use nle_core::{
    Clip, ClipSourceKind, ConversionOptions, ConversionReport, Marker, MediaAsset, MediaKind,
    NleError, NleFormat, NleResult, Project, ProjectWriter, Rational, Sequence, SequenceKind,
    Transition,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

const FCP_CROSS_DISSOLVE_UID: &str = "FxPlug:4731E73A-8DAC-4113-9A30-AE85B1761265";
const FCP_FADE_TO_COLOR_UID: &str = "FxPlug:4731E73A-8DAC-4113-9A30-AE85B1761265";

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
///
/// Supports multiple sequences, per-asset/per-sequence `<format>` resources, accurate `tcStart`
/// and source timecode `start`, connected video lanes (`V2+`), connected audio lanes (`A1+`) with
/// linked-audio deduplication, primary spine transitions, transforms, crop, opacity, retiming
/// (`<timeMap>`), compound clips (`<ref-clip>`), multicam clips (`<mc-clip>`), and markers.
pub fn write_fcpxml_file(
    project: &Project,
    output_path: &Path,
    options: &ConversionOptions,
) -> NleResult<ConversionReport> {
    let selected = collect_target_sequences(project, options);
    if selected.is_empty() {
        return Err(NleError::InvalidFormat(
            "No sequences selected for FCPXML export".into(),
        ));
    }

    let mut res = ResourceRegistry::new(project);

    // 1. Pre-register formats for all selected sequences
    for seq in &selected {
        res.ensure_format(seq.width, seq.height, seq.frame_rate);
    }

    // 2. Pre-scan which asset keys appear on audio tracks or video tracks
    for seq in &selected {
        for track in &seq.video_tracks {
            for clip in &track.clips {
                if clip.source_kind == ClipSourceKind::Media {
                    res.note_clip_usage(clip, true);
                }
            }
        }
        for track in &seq.audio_tracks {
            for clip in &track.clips {
                if clip.source_kind == ClipSourceKind::Media {
                    res.note_clip_usage(clip, false);
                }
            }
        }
    }

    // 3. Register all media assets referenced by selected sequences (plus unused project assets if requested)
    for seq in &selected {
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            for clip in &track.clips {
                if clip.source_kind == ClipSourceKind::Media {
                    res.ensure_asset_for_clip(clip);
                }
            }
            for tr in &track.transitions {
                res.ensure_transition_effect(tr);
            }
        }
    }
    if options.include_unused_media {
        for asset in &project.media_assets {
            res.ensure_asset_from_media_asset(asset);
        }
    }

    // 4. Pre-assign <media id="..."> resource IDs for compound & multicam sequences
    for seq in &selected {
        if matches!(
            seq.kind,
            SequenceKind::CompoundClip | SequenceKind::MulticamClip
        ) {
            res.register_media_sequence(seq);
        }
    }

    // 5. Render <resources>
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<!DOCTYPE fcpxml>\n");
    xml.push_str("<fcpxml version=\"1.11\">\n");
    xml.push_str("  <resources>\n");

    res.write_formats_xml(&mut xml);
    res.write_assets_xml(&mut xml);

    // Emit compound & multicam <media> definitions inside <resources>
    let mut dummy_stats = WriteStats::default();
    for seq in &selected {
        match seq.kind {
            SequenceKind::CompoundClip => {
                if let Some(mid) = res.media_seq_ids.get(&seq.id).cloned() {
                    xml.push_str(&format!(
                        "    <media id=\"{mid}\" name=\"{}\">\n",
                        xml_escape(&seq.name)
                    ));
                    write_sequence_element(seq, &res, options, 6, &mut xml, &mut dummy_stats);
                    xml.push_str("    </media>\n");
                }
            }
            SequenceKind::MulticamClip => {
                if let Some(mid) = res.media_seq_ids.get(&seq.id).cloned() {
                    write_multicam_media_resource(seq, &mid, &res, options, &mut xml);
                }
            }
            SequenceKind::Timeline => {}
        }
    }

    res.write_effects_xml(&mut xml);
    xml.push_str("  </resources>\n");

    // 6. Render <library><event><project>...</project></event></library>
    let proj_name = options
        .project_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&project.name);

    xml.push_str("  <library>\n");
    xml.push_str(&format!(
        "    <event name=\"{}\">\n",
        xml_escape(proj_name)
    ));

    let mut stats = WriteStats::default();
    let timeline_seqs: Vec<&Sequence> = selected
        .iter()
        .copied()
        .filter(|s| s.kind == SequenceKind::Timeline)
        .collect();
    let export_timelines = if timeline_seqs.is_empty() {
        selected.clone()
    } else {
        timeline_seqs
    };

    for seq in &export_timelines {
        xml.push_str(&format!(
            "      <project name=\"{}\">\n",
            xml_escape(&seq.name)
        ));
        write_sequence_element(seq, &res, options, 8, &mut xml, &mut stats);
        xml.push_str("      </project>\n");
        stats.sequences_converted += 1;
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
        sequences_converted: stats.sequences_converted,
        video_clips_converted: stats.video_clips_converted,
        audio_clips_converted: stats.audio_clips_converted,
        media_assets_linked: res.assets.len(),
        bins_created: 1,
        markers_written: stats.markers_written,
        grades_attached: 0,
        effect_markers_injected: 0,
        messages: Vec::new(),
    })
}

#[derive(Default)]
struct WriteStats {
    sequences_converted: usize,
    video_clips_converted: usize,
    audio_clips_converted: usize,
    markers_written: usize,
}

fn collect_target_sequences<'a>(
    project: &'a Project,
    options: &ConversionOptions,
) -> Vec<&'a Sequence> {
    let by_id: HashMap<&str, &'a Sequence> = project
        .sequences
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();

    let mut chosen: HashSet<&str> = if options.selected_sequence_ids.is_empty() {
        project.sequences.iter().map(|s| s.id.as_str()).collect()
    } else {
        options
            .selected_sequence_ids
            .iter()
            .map(|s| s.as_str())
            .filter(|id| by_id.contains_key(id))
            .collect()
    };

    // Transitive closure for compound/multicam/nested sequences
    let mut queue: Vec<&str> = chosen.iter().copied().collect();
    while let Some(sid) = queue.pop() {
        if let Some(seq) = by_id.get(sid) {
            for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
                for clip in &track.clips {
                    if clip.source_kind != ClipSourceKind::Media
                        && !clip.media_id.is_empty()
                        && by_id.contains_key(clip.media_id.as_str())
                        && chosen.insert(clip.media_id.as_str())
                    {
                        queue.push(clip.media_id.as_str());
                    }
                }
            }
        }
    }

    project
        .sequences
        .iter()
        .filter(|s| chosen.contains(s.id.as_str()))
        .collect()
}

// ── Resource Registry ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FormatKey {
    width: u32,
    height: u32,
    fps_num: i64,
    fps_den: i64,
}

struct RegisteredAsset {
    res_id: String,
    name: String,
    path: String,
    media_kind: MediaKind,
    format_id: Option<String>,
    fps: Rational,
    start_tc_frame: i64,
    duration_frames: i64,
    has_video: bool,
    has_audio: bool,
    audio_channels: u32,
    audio_sample_rate: u32,
    standard_metadata: BTreeMap<String, String>,
}

struct RegisteredEffect {
    res_id: String,
    name: String,
    uid: String,
}

struct ResourceRegistry<'a> {
    next_res: usize,
    formats: BTreeMap<FormatKey, (String, Rational)>,
    format_order: Vec<(FormatKey, String, Rational)>,
    asset_by_id: HashMap<&'a str, &'a MediaAsset>,
    asset_by_path: HashMap<String, &'a MediaAsset>,
    assets: BTreeMap<String, RegisteredAsset>,
    asset_order: Vec<String>,
    used_as_video: HashSet<String>,
    used_as_audio: HashSet<String>,
    effects: BTreeMap<String, RegisteredEffect>,
    media_seq_ids: HashMap<String, String>,
    /// `seq_id -> Vec<(angle_name, angle_id)>`
    multicam_angles: HashMap<String, Vec<(String, String)>>,
}

impl<'a> ResourceRegistry<'a> {
    fn new(project: &'a Project) -> Self {
        let asset_by_id = project
            .media_assets
            .iter()
            .map(|a| (a.id.as_str(), a))
            .collect();
        let asset_by_path = project
            .media_assets
            .iter()
            .filter(|a| !a.path.is_empty())
            .map(|a| (asset_key(&a.path, &a.name), a))
            .collect();
        Self {
            next_res: 1,
            formats: BTreeMap::new(),
            format_order: Vec::new(),
            asset_by_id,
            asset_by_path,
            assets: BTreeMap::new(),
            asset_order: Vec::new(),
            used_as_video: HashSet::new(),
            used_as_audio: HashSet::new(),
            effects: BTreeMap::new(),
            media_seq_ids: HashMap::new(),
            multicam_angles: HashMap::new(),
        }
    }

    fn alloc_res_id(&mut self) -> String {
        let id = format!("r{}", self.next_res);
        self.next_res += 1;
        id
    }

    fn ensure_format(&mut self, width: u32, height: u32, fps: Rational) -> String {
        let w = if width == 0 { 1920 } else { width };
        let h = if height == 0 { 1080 } else { height };
        let key = FormatKey {
            width: w,
            height: h,
            fps_num: fps.num,
            fps_den: fps.den,
        };
        if let Some((id, _)) = self.formats.get(&key) {
            return id.clone();
        }
        let id = self.alloc_res_id();
        self.formats.insert(key.clone(), (id.clone(), fps));
        self.format_order.push((key, id.clone(), fps));
        id
    }

    fn note_clip_usage(&mut self, clip: &Clip, is_video: bool) {
        let key = asset_key(&clip.media_path, &clip.name);
        if is_video {
            self.used_as_video.insert(key);
        } else {
            self.used_as_audio.insert(key);
        }
    }

    fn ensure_asset_for_clip(&mut self, clip: &Clip) -> String {
        let key = asset_key(&clip.media_path, &clip.name);
        if let Some(existing) = self.assets.get_mut(&key) {
            let needed_dur = clip.source_out_frame.max(clip.duration_frames()).max(1);
            if existing.media_kind != MediaKind::Image && needed_dur > existing.duration_frames {
                existing.duration_frames = needed_dur;
            }
            return existing.res_id.clone();
        }

        let matched = self
            .asset_by_id
            .get(clip.media_id.as_str())
            .copied()
            .or_else(|| self.asset_by_path.get(&key).copied());

        if let Some(ma) = matched {
            let rid = self.ensure_asset_from_media_asset(ma);
            if let Some(existing) = self.assets.get_mut(&key) {
                let needed_dur = clip.source_out_frame.max(clip.duration_frames()).max(1);
                if existing.media_kind != MediaKind::Image && needed_dur > existing.duration_frames {
                    existing.duration_frames = needed_dur;
                }
            }
            return rid;
        }

        let is_video = clip.media_kind != MediaKind::Audio || self.used_as_video.contains(&key);
        let is_audio =
            clip.media_kind != MediaKind::Image && (clip.media_kind == MediaKind::Audio || self.used_as_audio.contains(&key));
        let format_id = if is_video {
            Some(self.ensure_format(clip.source_width, clip.source_height, clip.source_fps))
        } else {
            None
        };
        let rid = self.alloc_res_id();
        let dur_frames = if clip.media_kind == MediaKind::Image {
            0
        } else {
            clip.source_out_frame.max(clip.duration_frames()).max(1)
        };
        let reg = RegisteredAsset {
            res_id: rid.clone(),
            name: clip.name.clone(),
            path: if clip.media_path.is_empty() {
                clip.name.clone()
            } else {
                clip.media_path.clone()
            },
            media_kind: clip.media_kind,
            format_id,
            fps: clip.source_fps,
            start_tc_frame: 0,
            duration_frames: dur_frames,
            has_video: is_video,
            has_audio: is_audio,
            audio_channels: if is_audio { 2 } else { 0 },
            audio_sample_rate: if is_audio { 48_000 } else { 0 },
            standard_metadata: BTreeMap::new(),
        };
        self.asset_order.push(key.clone());
        self.assets.insert(key, reg);
        rid
    }

    fn ensure_asset_from_media_asset(&mut self, ma: &MediaAsset) -> String {
        let key = asset_key(&ma.path, &ma.name);
        if let Some(existing) = self.assets.get(&key) {
            return existing.res_id.clone();
        }
        let has_video = ma.media_kind != MediaKind::Audio
            && (ma.width > 0 || ma.height > 0 || self.used_as_video.contains(&key) || ma.media_kind == MediaKind::Video || ma.media_kind == MediaKind::Image);
        let has_audio = ma.media_kind != MediaKind::Image
            && (ma.media_kind == MediaKind::Audio
                || ma.audio_channels > 0
                || self.used_as_audio.contains(&key));
        let format_id = if has_video {
            Some(self.ensure_format(ma.width, ma.height, ma.frame_rate))
        } else {
            None
        };
        let rid = self.alloc_res_id();
        let dur_frames = if ma.media_kind == MediaKind::Image {
            0
        } else {
            ma.duration_frames.max(1)
        };
        let reg = RegisteredAsset {
            res_id: rid.clone(),
            name: ma.name.clone(),
            path: if ma.path.is_empty() {
                ma.name.clone()
            } else {
                ma.path.clone()
            },
            media_kind: ma.media_kind,
            format_id,
            fps: ma.frame_rate,
            start_tc_frame: ma.source_tc_frame.max(0),
            duration_frames: dur_frames,
            has_video,
            has_audio,
            audio_channels: if has_audio {
                ma.audio_channels.max(2)
            } else {
                0
            },
            audio_sample_rate: if has_audio {
                if ma.audio_sample_rate > 0 {
                    ma.audio_sample_rate
                } else {
                    48_000
                }
            } else {
                0
            },
            // Only standard editorial metadata — never custom/third-party MAM metadata!
            standard_metadata: ma.metadata.clone(),
        };
        self.asset_order.push(key.clone());
        self.assets.insert(key, reg);
        rid
    }

    fn ensure_transition_effect(&mut self, tr: &Transition) -> String {
        let lower = tr.name.to_ascii_lowercase();
        let (canonical_name, uid) = if lower.contains("dip") || lower.contains("fade") {
            (tr.name.clone(), FCP_FADE_TO_COLOR_UID.to_string())
        } else {
            ("Cross Dissolve".to_string(), FCP_CROSS_DISSOLVE_UID.to_string())
        };
        let key = format!("{canonical_name}|{uid}");
        if let Some(e) = self.effects.get(&key) {
            return e.res_id.clone();
        }
        let rid = self.alloc_res_id();
        self.effects.insert(
            key,
            RegisteredEffect {
                res_id: rid.clone(),
                name: canonical_name,
                uid,
            },
        );
        rid
    }

    fn transition_effect_id(&self, tr: &Transition) -> Option<(&str, &str)> {
        let lower = tr.name.to_ascii_lowercase();
        let (canonical_name, uid) = if lower.contains("dip") || lower.contains("fade") {
            (tr.name.as_str(), FCP_FADE_TO_COLOR_UID)
        } else {
            ("Cross Dissolve", FCP_CROSS_DISSOLVE_UID)
        };
        let key = format!("{canonical_name}|{uid}");
        self.effects
            .get(&key)
            .map(|e| (e.res_id.as_str(), e.name.as_str()))
    }

    fn register_media_sequence(&mut self, seq: &Sequence) {
        if self.media_seq_ids.contains_key(&seq.id) {
            return;
        }
        let rid = self.alloc_res_id();
        self.media_seq_ids.insert(seq.id.clone(), rid);
        if seq.kind == SequenceKind::MulticamClip {
            let angles: Vec<(String, String)> = seq
                .video_tracks
                .iter()
                .enumerate()
                .map(|(i, t)| (t.name.clone(), format!("angle-{}", i + 1)))
                .collect();
            self.multicam_angles.insert(seq.id.clone(), angles);
        }
    }

    fn format_id_for(&self, width: u32, height: u32, fps: Rational) -> &str {
        let w = if width == 0 { 1920 } else { width };
        let h = if height == 0 { 1080 } else { height };
        let key = FormatKey {
            width: w,
            height: h,
            fps_num: fps.num,
            fps_den: fps.den,
        };
        self.formats
            .get(&key)
            .map(|(id, _)| id.as_str())
            .unwrap_or("r1")
    }

    fn write_formats_xml(&self, xml: &mut String) {
        for (key, id, fps) in &self.format_order {
            let frame_dur = FcpTime::from_frames(1, *fps).to_fcpxml_string();
            let fps_tag = fps.label().replace('.', "");
            xml.push_str(&format!(
                "    <format id=\"{id}\" name=\"FFVideoFormat{}p{fps_tag}\" frameDuration=\"{frame_dur}\" width=\"{}\" height=\"{}\"/>\n",
                key.height, key.width, key.height
            ));
        }
    }

    fn write_assets_xml(&self, xml: &mut String) {
        for key in &self.asset_order {
            let a = &self.assets[key];
            let start_str = FcpTime::from_frames(a.start_tc_frame, a.fps).to_fcpxml_string();
            let dur_str = if a.media_kind == MediaKind::Image {
                "0s".to_string()
            } else {
                FcpTime::from_frames(a.duration_frames.max(1), a.fps).to_fcpxml_string()
            };
            let has_v = if a.has_video { "1" } else { "0" };
            let has_a = if a.has_audio { "1" } else { "0" };

            xml.push_str(&format!(
                "    <asset id=\"{}\" name=\"{}\" start=\"{start_str}\" duration=\"{dur_str}\" hasVideo=\"{has_v}\"",
                a.res_id,
                xml_escape(&a.name)
            ));
            if let Some(fid) = &a.format_id {
                xml.push_str(&format!(" format=\"{fid}\""));
            }
            xml.push_str(&format!(" hasAudio=\"{has_a}\""));
            if a.has_audio {
                xml.push_str(&format!(
                    " audioSources=\"1\" audioChannels=\"{}\" audioRate=\"{}\"",
                    a.audio_channels.max(1),
                    a.audio_sample_rate.max(48_000)
                ));
            }
            xml.push_str(">\n");
            let src_url = path_to_file_url(&a.path);
            xml.push_str(&format!(
                "      <media-rep kind=\"original-media\" src=\"{}\"/>\n",
                xml_escape(&src_url)
            ));
            write_standard_asset_metadata_xml(&a.standard_metadata, xml);
            xml.push_str("    </asset>\n");
        }
    }

    fn write_effects_xml(&self, xml: &mut String) {
        for fx in self.effects.values() {
            xml.push_str(&format!(
                "    <effect id=\"{}\" name=\"{}\" uid=\"{}\"/>\n",
                fx.res_id,
                xml_escape(&fx.name),
                xml_escape(&fx.uid)
            ));
        }
    }
}

fn asset_key(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        path.to_string()
    }
}

fn write_standard_asset_metadata_xml(meta: &BTreeMap<String, String>, xml: &mut String) {
    let mut entries = Vec::new();
    for (k, v) in meta {
        if v.trim().is_empty() {
            continue;
        }
        let fcp_key = match k.as_str() {
            "Tape Name" => Some("com.apple.proapps.studio.reel"),
            "Scene" => Some("com.apple.proapps.studio.scene"),
            "Shot" => Some("com.apple.proapps.studio.shot"),
            "Take" => Some("com.apple.proapps.studio.take"),
            "Camera #" => Some("com.apple.proapps.mio.cameraName"),
            "Angle" => Some("com.apple.proapps.mio.cameraAngle"),
            "Comments" => Some("com.apple.proapps.spotlight.kMDItemFinderComment"),
            "Description" => Some("com.apple.proapps.share.description"),
            _ => None,
        };
        if let Some(fk) = fcp_key {
            entries.push((fk, v.as_str()));
        }
    }
    if entries.is_empty() {
        return;
    }
    xml.push_str("      <metadata>\n");
    for (fk, val) in entries {
        xml.push_str(&format!(
            "        <md key=\"{fk}\" value=\"{}\"/>\n",
            xml_escape(val)
        ));
    }
    xml.push_str("      </metadata>\n");
}

// ── Sequence & Spine Serialization ──────────────────────────────────────

struct SpineSlot<'a> {
    start_frame: i64,
    end_frame: i64,
    /// Anchor item's local `start` time (used to compute connected clip & marker offsets).
    local_start: FcpTime,
    kind: SpineSlotKind<'a>,
    connected: Vec<ConnectedItem<'a>>,
    markers: Vec<&'a Marker>,
}

enum SpineSlotKind<'a> {
    Gap,
    Clip {
        clip: &'a Clip,
        has_embedded_audio: bool,
    },
}

struct ConnectedItem<'a> {
    lane: i32,
    clip: &'a Clip,
    is_audio_only: bool,
    has_embedded_audio: bool,
}

fn write_multicam_media_resource(
    seq: &Sequence,
    mid: &str,
    res: &ResourceRegistry<'_>,
    options: &ConversionOptions,
    xml: &mut String,
) {
    let fmt_id = res.format_id_for(seq.width, seq.height, seq.frame_rate);
    let tc_start = FcpTime::from_frames(seq.start_tc_frame.max(0), seq.frame_rate);
    let tc_format = fcpxml_tc_format(seq.frame_rate);
    xml.push_str(&format!(
        "    <media id=\"{mid}\" name=\"{}\">\n",
        xml_escape(&seq.name)
    ));
    xml.push_str(&format!(
        "      <multicam format=\"{fmt_id}\" tcStart=\"{}\" tcFormat=\"{tc_format}\">\n",
        tc_start.to_fcpxml_string()
    ));

    let angles = res.multicam_angles.get(&seq.id);
    for (idx, vtrack) in seq.video_tracks.iter().enumerate() {
        let (angle_name, angle_id) = angles
            .and_then(|a| a.get(idx))
            .map(|(n, id)| (n.as_str(), id.as_str()))
            .unwrap_or((vtrack.name.as_str(), "angle-1"));
        xml.push_str(&format!(
            "        <mc-angle name=\"{}\" angleID=\"{angle_id}\">\n",
            xml_escape(angle_name)
        ));
        let mut sorted: Vec<&Clip> = vtrack.clips.iter().collect();
        sorted.sort_by_key(|c| c.start_frame);
        let mut cursor = 0i64;
        for clip in sorted {
            if clip.start_frame > cursor {
                let gap_off = tc_start + FcpTime::from_frames(cursor, seq.frame_rate);
                let gap_dur = FcpTime::from_frames(clip.start_frame - cursor, seq.frame_rate);
                xml.push_str(&format!(
                    "          <gap name=\"Gap\" offset=\"{}\" duration=\"{}\"/>\n",
                    gap_off.to_fcpxml_string(),
                    gap_dur.to_fcpxml_string()
                ));
            }
            let off = tc_start + FcpTime::from_frames(clip.start_frame.max(0), seq.frame_rate);
            let mut stats = WriteStats::default();
            write_clip_element(
                clip,
                off,
                None,
                false,
                true,
                seq,
                res,
                options,
                &[],
                &[],
                10,
                xml,
                &mut stats,
            );
            cursor = clip.end_frame.max(cursor);
        }
        xml.push_str("        </mc-angle>\n");
    }

    xml.push_str("      </multicam>\n");
    xml.push_str("    </media>\n");
}

fn write_sequence_element(
    seq: &Sequence,
    res: &ResourceRegistry<'_>,
    options: &ConversionOptions,
    indent: usize,
    xml: &mut String,
    stats: &mut WriteStats,
) {
    let pad = " ".repeat(indent);
    let fmt_id = res.format_id_for(seq.width, seq.height, seq.frame_rate);
    let total_dur = seq.computed_duration_frames().max(1);
    let dur_str = FcpTime::from_frames(total_dur, seq.frame_rate).to_fcpxml_string();
    let tc_start = FcpTime::from_frames(seq.start_tc_frame.max(0), seq.frame_rate);
    let tc_start_str = tc_start.to_fcpxml_string();
    let tc_format = fcpxml_tc_format(seq.frame_rate);

    xml.push_str(&format!(
        "{pad}<sequence format=\"{fmt_id}\" duration=\"{dur_str}\" tcStart=\"{tc_start_str}\" tcFormat=\"{tc_format}\" audioLayout=\"stereo\" audioRate=\"48k\">\n"
    ));
    xml.push_str(&format!("{pad}  <spine>\n"));

    // Match video clips to paired audio clips so embedded audio isn't duplicated as detached lanes
    let mut embedded_audio_ids: HashSet<(usize, usize)> = HashSet::new();
    let mut video_has_audio: HashSet<(usize, usize)> = HashSet::new();

    for (vt_idx, vtrack) in seq.video_tracks.iter().enumerate() {
        for (vc_idx, vclip) in vtrack.clips.iter().enumerate() {
            for (at_idx, atrack) in seq.audio_tracks.iter().enumerate() {
                for (ac_idx, aclip) in atrack.clips.iter().enumerate() {
                    if embedded_audio_ids.contains(&(at_idx, ac_idx)) {
                        continue;
                    }
                    let same_link = vclip.link_group_id.is_some()
                        && vclip.link_group_id == aclip.link_group_id;
                    let same_media = vclip.source_kind == aclip.source_kind
                        && ((vclip.source_kind == ClipSourceKind::Media
                            && asset_key(&vclip.media_path, &vclip.name)
                                == asset_key(&aclip.media_path, &aclip.name))
                            || (vclip.source_kind != ClipSourceKind::Media
                                && vclip.media_id == aclip.media_id));
                    let same_timing = vclip.start_frame == aclip.start_frame
                        && vclip.end_frame == aclip.end_frame
                        && vclip.source_in_frame == aclip.source_in_frame
                        && (vclip.speed_percent - aclip.speed_percent).abs() < 0.01
                        && vclip.reversed == aclip.reversed;
                    if same_media && same_timing && (same_link || vclip.link_group_id.is_none()) {
                        embedded_audio_ids.insert((at_idx, ac_idx));
                        video_has_audio.insert((vt_idx, vc_idx));
                        stats.audio_clips_converted += 1;
                        break;
                    }
                }
            }
        }
    }

    // Build primary spine slots from V1 + gaps covering [0, total_dur]
    let mut slots: Vec<SpineSlot<'_>> = Vec::new();
    let mut cursor = 0i64;
    if let Some(v1) = seq.video_tracks.first() {
        let mut indexed: Vec<(usize, &Clip)> = v1.clips.iter().enumerate().collect();
        indexed.sort_by_key(|(_, c)| c.start_frame);

        for (vc_idx, clip) in indexed {
            let clip_start = clip.start_frame.max(0);
            let clip_end = clip.end_frame.max(clip_start + 1);
            if clip_start > cursor {
                slots.push(SpineSlot {
                    start_frame: cursor,
                    end_frame: clip_start,
                    local_start: FcpTime::ZERO,
                    kind: SpineSlotKind::Gap,
                    connected: Vec::new(),
                    markers: Vec::new(),
                });
            }
            let local_start = clip_local_start_time(clip, res);
            slots.push(SpineSlot {
                start_frame: clip_start,
                end_frame: clip_end,
                local_start,
                kind: SpineSlotKind::Clip {
                    clip,
                    has_embedded_audio: video_has_audio.contains(&(0, vc_idx)),
                },
                connected: Vec::new(),
                markers: Vec::new(),
            });
            cursor = clip_end.max(cursor);
            stats.video_clips_converted += 1;
        }
    }
    if cursor < total_dur || slots.is_empty() {
        slots.push(SpineSlot {
            start_frame: cursor,
            end_frame: total_dur.max(cursor + 1),
            local_start: FcpTime::ZERO,
            kind: SpineSlotKind::Gap,
            connected: Vec::new(),
            markers: Vec::new(),
        });
    }

    // Anchor V2+ clips onto positive lanes (lane = vt_idx)
    for (vt_idx, vtrack) in seq.video_tracks.iter().enumerate().skip(1) {
        let lane = vt_idx as i32;
        for (vc_idx, clip) in vtrack.clips.iter().enumerate() {
            let slot_idx = find_anchor_slot(&slots, clip.start_frame.max(0));
            slots[slot_idx].connected.push(ConnectedItem {
                lane,
                clip,
                is_audio_only: false,
                has_embedded_audio: video_has_audio.contains(&(vt_idx, vc_idx)),
            });
            stats.video_clips_converted += 1;
        }
    }

    // Anchor remaining (non-embedded) audio clips onto negative lanes (lane = -(at_idx + 1))
    for (at_idx, atrack) in seq.audio_tracks.iter().enumerate() {
        let lane = -((at_idx as i32) + 1);
        for (ac_idx, clip) in atrack.clips.iter().enumerate() {
            if embedded_audio_ids.contains(&(at_idx, ac_idx)) {
                continue;
            }
            let slot_idx = find_anchor_slot(&slots, clip.start_frame.max(0));
            slots[slot_idx].connected.push(ConnectedItem {
                lane,
                clip,
                is_audio_only: true,
                has_embedded_audio: true,
            });
            stats.audio_clips_converted += 1;
        }
    }

    // Anchor sequence markers onto overlapping primary spine slots
    if options.include_markers {
        for m in &seq.markers {
            let slot_idx = find_anchor_slot(&slots, m.frame.max(0));
            slots[slot_idx].markers.push(m);
        }
    }

    // Index V1 transitions by the cut point between adjacent spine slots
    let v1_transitions: &[Transition] = seq
        .video_tracks
        .first()
        .map(|t| t.transitions.as_slice())
        .unwrap_or(&[]);
    let mut emitted_tr: HashSet<usize> = HashSet::new();

    for (s_idx, slot) in slots.iter().enumerate() {
        // Check if a V1 transition straddles the cut right before this slot
        if s_idx > 0 {
            let cut_frame = slot.start_frame;
            for (tr_idx, tr) in v1_transitions.iter().enumerate() {
                if !emitted_tr.contains(&tr_idx)
                    && tr.start_frame <= cut_frame
                    && tr.end_frame >= cut_frame
                {
                    write_transition_element(tr, tc_start, seq.frame_rate, res, indent + 4, xml);
                    emitted_tr.insert(tr_idx);
                }
            }
        }

        let slot_offset = tc_start + FcpTime::from_frames(slot.start_frame, seq.frame_rate);
        match &slot.kind {
            SpineSlotKind::Gap => {
                let dur = FcpTime::from_frames((slot.end_frame - slot.start_frame).max(1), seq.frame_rate);
                if slot.connected.is_empty() && slot.markers.is_empty() {
                    xml.push_str(&format!(
                        "{pad}    <gap name=\"Gap\" offset=\"{}\" start=\"0s\" duration=\"{}\"/>\n",
                        slot_offset.to_fcpxml_string(),
                        dur.to_fcpxml_string()
                    ));
                } else {
                    xml.push_str(&format!(
                        "{pad}    <gap name=\"Gap\" offset=\"{}\" start=\"0s\" duration=\"{}\">\n",
                        slot_offset.to_fcpxml_string(),
                        dur.to_fcpxml_string()
                    ));
                    for conn in &slot.connected {
                        let rel_off = slot.local_start
                            + FcpTime::from_frames(conn.clip.start_frame - slot.start_frame, seq.frame_rate);
                        write_clip_element(
                            conn.clip,
                            rel_off,
                            Some(conn.lane),
                            conn.is_audio_only,
                            conn.has_embedded_audio,
                            seq,
                            res,
                            options,
                            &[],
                            &[],
                            indent + 6,
                            xml,
                            stats,
                        );
                    }
                    for m in &slot.markers {
                        let m_start = slot.local_start
                            + FcpTime::from_frames(m.frame - slot.start_frame, seq.frame_rate);
                        write_marker_element(m, m_start, seq.frame_rate, indent + 6, xml);
                        stats.markers_written += 1;
                    }
                    xml.push_str(&format!("{pad}    </gap>\n"));
                }
            }
            SpineSlotKind::Clip {
                clip,
                has_embedded_audio,
            } => {
                write_clip_element(
                    clip,
                    slot_offset,
                    None,
                    false,
                    *has_embedded_audio,
                    seq,
                    res,
                    options,
                    &slot.connected,
                    &slot.markers,
                    indent + 4,
                    xml,
                    stats,
                );
            }
        }
    }

    xml.push_str(&format!("{pad}  </spine>\n"));
    xml.push_str(&format!("{pad}</sequence>\n"));
}

#[allow(clippy::too_many_arguments)]
fn write_clip_element(
    clip: &Clip,
    offset: FcpTime,
    lane: Option<i32>,
    is_audio_only: bool,
    has_embedded_audio: bool,
    seq: &Sequence,
    res: &ResourceRegistry<'_>,
    options: &ConversionOptions,
    connected: &[ConnectedItem<'_>],
    seq_markers: &[&Marker],
    indent: usize,
    xml: &mut String,
    stats: &mut WriteStats,
) {
    let pad = " ".repeat(indent);
    let dur = FcpTime::from_frames(clip.duration_frames().max(1), seq.frame_rate);
    let local_start = clip_local_start_time(clip, res);
    let lane_attr = lane.map(|l| format!(" lane=\"{l}\"")).unwrap_or_default();
    let enabled_attr = if clip.enabled { "" } else { " enabled=\"0\"" };

    let (tag, ref_id, mc_angle_id, disable_audio_on_av_asset) = match clip.source_kind {
        ClipSourceKind::CompoundClip | ClipSourceKind::NestedSequence => {
            if let Some(mid) = res.media_seq_ids.get(&clip.media_id) {
                ("ref-clip", mid.as_str(), None, false)
            } else {
                let key = asset_key(&clip.media_path, &clip.name);
                let rid = res
                    .assets
                    .get(&key)
                    .map(|a| a.res_id.as_str())
                    .unwrap_or("r2");
                ("asset-clip", rid, None, false)
            }
        }
        ClipSourceKind::MulticamClip => {
            if let Some(mid) = res.media_seq_ids.get(&clip.media_id) {
                let angle_id = res.multicam_angles.get(&clip.media_id).and_then(|angles| {
                    clip.active_angle
                        .as_deref()
                        .and_then(|wanted| {
                            angles
                                .iter()
                                .find(|(name, id)| name == wanted || id == wanted)
                                .map(|(_, id)| id.as_str())
                        })
                        .or_else(|| angles.first().map(|(_, id)| id.as_str()))
                });
                ("mc-clip", mid.as_str(), angle_id, false)
            } else {
                let key = asset_key(&clip.media_path, &clip.name);
                let rid = res
                    .assets
                    .get(&key)
                    .map(|a| a.res_id.as_str())
                    .unwrap_or("r2");
                ("asset-clip", rid, None, false)
            }
        }
        ClipSourceKind::Media => {
            let key = asset_key(&clip.media_path, &clip.name);
            let reg = res.assets.get(&key);
            let rid = reg.map(|a| a.res_id.as_str()).unwrap_or("r2");
            let mute_av_audio = !is_audio_only
                && !has_embedded_audio
                && reg.map(|a| a.has_video && a.has_audio).unwrap_or(false);
            ("asset-clip", rid, None, mute_av_audio)
        }
    };

    let src_enable_attr = if tag == "asset-clip" && is_audio_only {
        let key = asset_key(&clip.media_path, &clip.name);
        if res.assets.get(&key).map(|a| a.has_video).unwrap_or(false) {
            " srcEnable=\"audio\""
        } else {
            ""
        }
    } else if tag == "asset-clip" && disable_audio_on_av_asset {
        " srcEnable=\"video\""
    } else {
        ""
    };

    xml.push_str(&format!(
        "{pad}<{tag} ref=\"{ref_id}\"{lane_attr} offset=\"{}\" name=\"{}\" start=\"{}\" duration=\"{}\"{src_enable_attr}{enabled_attr}>\n",
        offset.to_fcpxml_string(),
        xml_escape(&clip.name),
        local_start.to_fcpxml_string(),
        dur.to_fcpxml_string(),
    ));

    let cpad = " ".repeat(indent + 2);

    // 1. Retiming (<timeMap>)
    let is_retimed = (clip.speed_percent - 100.0).abs() > 0.05 || clip.reversed;
    if is_retimed {
        write_timemap_element(clip, local_start, dur, res, &cpad, xml);
    }

    // 2. Crop (<adjust-crop>)
    if !is_audio_only {
        let m = &clip.motion;
        let aspect = if seq.height > 0 {
            seq.width as f64 / seq.height as f64
        } else {
            16.0 / 9.0
        };
        if m.crop_left.abs() > 1e-4
            || m.crop_right.abs() > 1e-4
            || m.crop_top.abs() > 1e-4
            || m.crop_bottom.abs() > 1e-4
        {
            xml.push_str(&format!("{cpad}<adjust-crop mode=\"trim\">\n"));
            xml.push_str(&format!(
                "{cpad}  <trim-rect left=\"{:.4}\" right=\"{:.4}\" top=\"{:.4}\" bottom=\"{:.4}\"/>\n",
                m.crop_left * aspect,
                m.crop_right * aspect,
                m.crop_top,
                m.crop_bottom
            ));
            xml.push_str(&format!("{cpad}</adjust-crop>\n"));
        }

        // 3. Transform (<adjust-transform>)
        let has_transform = (m.scale - 100.0).abs() > 1e-4
            || m.scale_y.is_some()
            || m.position_x.abs() > 1e-4
            || m.position_y.abs() > 1e-4
            || m.rotation.abs() > 1e-4
            || m.anchor_x.abs() > 1e-4
            || m.anchor_y.abs() > 1e-4;
        if has_transform {
            let sx = m.scale / 100.0;
            let sy = m.scale_y.unwrap_or(m.scale) / 100.0;
            let px = m.position_x * aspect * 100.0;
            let py = m.position_y * 100.0;
            let rot = -m.rotation; // IR CW-positive -> FCPXML CCW-positive
            let mut attrs = format!("position=\"{px:.4} {py:.4}\" scale=\"{sx:.4} {sy:.4}\"");
            if rot.abs() > 1e-4 {
                attrs.push_str(&format!(" rotation=\"{rot:.3}\""));
            }
            if m.anchor_x.abs() > 1e-4 || m.anchor_y.abs() > 1e-4 {
                let ax = m.anchor_x * aspect * 100.0;
                let ay = -m.anchor_y * 100.0;
                attrs.push_str(&format!(" anchor=\"{ax:.4} {ay:.4}\""));
            }
            xml.push_str(&format!("{cpad}<adjust-transform {attrs}/>\n"));
        }

        // 4. Opacity (<adjust-blend>)
        if (clip.opacity - 100.0).abs() > 1e-3 {
            let amt = (clip.opacity / 100.0).clamp(0.0, 1.0);
            xml.push_str(&format!("{cpad}<adjust-blend amount=\"{amt:.4}\"/>\n"));
        }
    }

    // 5. Multicam <mc-source>
    if let Some(aid) = mc_angle_id {
        let src_enable = if is_audio_only {
            "audio"
        } else if has_embedded_audio {
            "all"
        } else {
            "video"
        };
        xml.push_str(&format!(
            "{cpad}<mc-source angleID=\"{}\" srcEnable=\"{src_enable}\"/>\n",
            xml_escape(aid)
        ));
    }

    // 6. Connected clips anchored to this clip
    for conn in connected {
        let rel_off = local_start
            + FcpTime::from_frames(conn.clip.start_frame - clip.start_frame, seq.frame_rate);
        write_clip_element(
            conn.clip,
            rel_off,
            Some(conn.lane),
            conn.is_audio_only,
            conn.has_embedded_audio,
            seq,
            res,
            options,
            &[],
            &[],
            indent + 2,
            xml,
            stats,
        );
    }

    // 7. Sequence markers falling within this clip + clip-level markers
    if options.include_markers {
        for m in seq_markers {
            let m_start =
                local_start + FcpTime::from_frames(m.frame - clip.start_frame, seq.frame_rate);
            write_marker_element(m, m_start, seq.frame_rate, indent + 2, xml);
            stats.markers_written += 1;
        }
        for m in &clip.markers {
            let m_start = local_start + FcpTime::from_frames(m.frame, seq.frame_rate);
            write_marker_element(m, m_start, seq.frame_rate, indent + 2, xml);
            stats.markers_written += 1;
        }
    }

    xml.push_str(&format!("{pad}</{tag}>\n"));
}

fn write_timemap_element(
    clip: &Clip,
    local_start: FcpTime,
    dur: FcpTime,
    res: &ResourceRegistry<'_>,
    pad: &str,
    xml: &mut String,
) {
    let key = asset_key(&clip.media_path, &clip.name);
    let asset_tc = res
        .assets
        .get(&key)
        .map(|a| FcpTime::from_frames(a.start_tc_frame, a.fps))
        .unwrap_or(FcpTime::ZERO);

    let src_in = asset_tc + FcpTime::from_frames(clip.source_in_frame.max(0), clip.source_fps);
    let src_out = asset_tc
        + FcpTime::from_frames(
            clip.source_out_frame
                .max(clip.source_in_frame + 1),
            clip.source_fps,
        );

    let (v0, v1) = if clip.speed_percent.abs() < 1e-4 {
        // Freeze frame
        (src_in, src_in)
    } else if clip.reversed {
        // Reversed: play from src_out down to src_in
        (src_out, src_in)
    } else {
        // Forward speed change
        let span_frames = ((clip.duration_frames().max(1) as f64) * (clip.speed_percent / 100.0))
            .round()
            .max(1.0) as i64;
        let end_v = local_start + FcpTime::from_frames(span_frames, clip.source_fps);
        (local_start, end_v)
    };

    xml.push_str(&format!("{pad}<timeMap>\n"));
    xml.push_str(&format!(
        "{pad}  <timept time=\"0s\" value=\"{}\" interp=\"smooth2\"/>\n",
        v0.to_fcpxml_string()
    ));
    xml.push_str(&format!(
        "{pad}  <timept time=\"{}\" value=\"{}\" interp=\"smooth2\"/>\n",
        dur.to_fcpxml_string(),
        v1.to_fcpxml_string()
    ));
    xml.push_str(&format!("{pad}</timeMap>\n"));
}

fn write_transition_element(
    tr: &Transition,
    tc_start: FcpTime,
    fps: Rational,
    res: &ResourceRegistry<'_>,
    indent: usize,
    xml: &mut String,
) {
    let pad = " ".repeat(indent);
    let off = tc_start + FcpTime::from_frames(tr.start_frame.max(0), fps);
    let dur = FcpTime::from_frames((tr.end_frame - tr.start_frame).max(1), fps);
    xml.push_str(&format!(
        "{pad}<transition name=\"{}\" offset=\"{}\" duration=\"{}\">\n",
        xml_escape(&tr.name),
        off.to_fcpxml_string(),
        dur.to_fcpxml_string()
    ));
    if let Some((fx_rid, fx_name)) = res.transition_effect_id(tr) {
        xml.push_str(&format!(
            "{pad}  <filter-video ref=\"{fx_rid}\" name=\"{}\"/>\n",
            xml_escape(fx_name)
        ));
    }
    xml.push_str(&format!("{pad}</transition>\n"));
}

fn write_marker_element(
    m: &Marker,
    start: FcpTime,
    fps: Rational,
    indent: usize,
    xml: &mut String,
) {
    let pad = " ".repeat(indent);
    let dur = FcpTime::from_frames(m.duration_frames.max(1), fps);
    let note_attr = if m.comment.is_empty() {
        String::new()
    } else {
        format!(" note=\"{}\"", xml_escape(&m.comment))
    };
    let completed_attr = match m.color.to_ascii_lowercase().as_str() {
        "red" => " completed=\"0\"",
        "green" => " completed=\"1\"",
        _ => "",
    };
    xml.push_str(&format!(
        "{pad}<marker start=\"{}\" duration=\"{}\" value=\"{}\"{completed_attr}{note_attr}/>\n",
        start.to_fcpxml_string(),
        dur.to_fcpxml_string(),
        xml_escape(&m.name)
    ));
}

fn clip_local_start_time(clip: &Clip, res: &ResourceRegistry<'_>) -> FcpTime {
    let base_tc = if clip.source_kind == ClipSourceKind::Media {
        let key = asset_key(&clip.media_path, &clip.name);
        res.assets
            .get(&key)
            .map(|a| FcpTime::from_frames(a.start_tc_frame, a.fps))
            .unwrap_or(FcpTime::ZERO)
    } else {
        FcpTime::ZERO
    };
    base_tc + FcpTime::from_frames(clip.source_in_frame.max(0), clip.source_fps)
}

fn find_anchor_slot(slots: &[SpineSlot<'_>], frame: i64) -> usize {
    for (idx, slot) in slots.iter().enumerate() {
        if frame >= slot.start_frame && frame < slot.end_frame {
            return idx;
        }
    }
    slots.len().saturating_sub(1)
}

fn fcpxml_tc_format(fps: Rational) -> &'static str {
    if fps.is_ntsc() && matches!(fps.timebase(), 30 | 60) {
        "DF"
    } else {
        "NDF"
    }
}
