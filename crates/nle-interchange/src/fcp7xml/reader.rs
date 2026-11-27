use crate::fcpxml::time::file_url_to_path;
use nle_core::{
    frames_to_timecode, timecode_to_frames, AlphaMode, BlendMode, Clip, ClipSourceKind,
    ColorConfig, DataLevels, EffectCategory, EffectInstance, EffectParameter, Marker, MarkerScope,
    MediaAsset, MediaKind, MotionTransform, NleError, NleFormat, NleResult, Project, ProjectBin,
    ProjectReader, Rational, RetimeInterpolation, Sequence, SequenceKind, SpeedKeyframe, Track,
    TrackKind, Transition,
};
use roxmltree::{Document, Node, ParsingOptions};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use uuid::Uuid;

pub struct Fcp7XmlReader;

impl ProjectReader for Fcp7XmlReader {
    fn format(&self) -> NleFormat {
        NleFormat::Fcp7Xml
    }

    fn read_file(&self, path: &Path) -> NleResult<Project> {
        read_fcp7_xml_file(path)
    }
}

/// Read an FCP7 XML (XMEML v4/v5 `.xml`) file into the canonical `Project` IR.
pub fn read_fcp7_xml_file(path: &Path) -> NleResult<Project> {
    let xml = fs::read_to_string(path)?;
    let default_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("FCP7 XML Project");
    let mut project = parse_fcp7_xml_str(&xml, default_name)?;
    project.source_path = Some(path.to_string_lossy().into_owned());
    Ok(project)
}

/// Parse an FCP7 XMEML v4/v5 document string into the canonical `Project` IR.
pub fn parse_fcp7_xml_str(xml: &str, default_name: &str) -> NleResult<Project> {
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = Document::parse_with_options(xml, opts)
        .map_err(|e| NleError::Xml(format!("Invalid FCP7 XML document: {e}")))?;

    let root = doc.root_element();
    if root.tag_name().name() != "xmeml" {
        return Err(NleError::InvalidFormat(format!(
            "Expected root <xmeml> element for FCP7 XML, found <{}>",
            root.tag_name().name()
        )));
    }

    let version = root.attribute("version").unwrap_or("5").to_string();
    let mut ctx = Fcp7ParseCtx::new();

    // Pass 1: Index all <file id="..."> definitions across the entire XML document
    ctx.index_files(root);

    let mut project_name = default_name.to_string();
    let mut bins: Vec<ProjectBin> = Vec::new();
    let mut sequences: Vec<Sequence> = Vec::new();
    let mut seen_seq_ids: HashSet<String> = HashSet::new();

    // Pass 2: Walk <project>, top-level <bin>, and top-level <sequence> elements
    for child in root.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "project" => {
                if let Some(pname) = child_text(child, "name").filter(|s| !s.is_empty()) {
                    project_name = pname;
                }
                if let Some(children_node) = child_elem(child, "children") {
                    ctx.walk_container(
                        children_node,
                        None,
                        &mut bins,
                        &mut sequences,
                        &mut seen_seq_ids,
                        &mut Vec::new(),
                        &mut Vec::new(),
                    );
                }
            }
            "bin" => {
                ctx.parse_bin(child, None, &mut bins, &mut sequences, &mut seen_seq_ids);
            }
            "sequence" => {
                if let Some(seq) = ctx.parse_sequence(child, &mut seen_seq_ids) {
                    if project_name.is_empty() || project_name == "FCP7 XML Project" {
                        project_name = seq.name.clone();
                    }
                    sequences.push(seq);
                }
            }
            "clip" => {
                ctx.register_master_clip(child, &mut Vec::new());
            }
            _ => {}
        }
    }

    // Also include any referenced nested sequences that were defined inline inside clipitems
    sequences.extend(ctx.extra_sequences);

    let mut metadata = BTreeMap::new();
    metadata.insert("xmeml_version".into(), version);

    Ok(Project {
        name: project_name,
        source_format: NleFormat::Fcp7Xml,
        source_path: None,
        color_config: ColorConfig::default(),
        scale_to_frame_size: true,
        bins,
        media_assets: ctx.media_assets,
        sequences,
        metadata,
        custom_metadata_schema: BTreeMap::new(),
    })
}

// ── File Registry & Parse Context ───────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct Fcp7FileInfo {
    id: String,
    name: String,
    path: String,
    frame_rate: Option<Rational>,
    duration_frames: i64,
    source_tc_frame: i64,
    width: u32,
    height: u32,
    pixel_aspect_ratio: Option<f64>,
    has_video: bool,
    has_audio: bool,
    audio_channels: u32,
    audio_sample_rate: u32,
    reel_name: Option<String>,
    metadata: BTreeMap<String, String>,
}

struct Fcp7ParseCtx {
    files_by_id: HashMap<String, Fcp7FileInfo>,
    media_assets: Vec<MediaAsset>,
    asset_idx_by_key: HashMap<String, usize>,
    extra_sequences: Vec<Sequence>,
    next_clip_id: usize,
}

impl Fcp7ParseCtx {
    fn new() -> Self {
        Self {
            files_by_id: HashMap::new(),
            media_assets: Vec::new(),
            asset_idx_by_key: HashMap::new(),
            extra_sequences: Vec::new(),
            next_clip_id: 1,
        }
    }

    fn alloc_clip_id(&mut self) -> String {
        let id = format!("fcp7-clip-{}", self.next_clip_id);
        self.next_clip_id += 1;
        id
    }

    fn index_files(&mut self, root: Node<'_, '_>) {
        for node in root.descendants().filter(|n| n.is_element() && n.has_tag_name("file")) {
            let Some(fid) = node.attribute("id").filter(|s| !s.is_empty()) else {
                continue;
            };
            let entry = self
                .files_by_id
                .entry(fid.to_string())
                .or_insert_with(|| Fcp7FileInfo {
                    id: fid.to_string(),
                    ..Default::default()
                });
            merge_file_node(entry, node);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_container(
        &mut self,
        container: Node<'_, '_>,
        parent_bin_id: Option<&str>,
        bins: &mut Vec<ProjectBin>,
        sequences: &mut Vec<Sequence>,
        seen_seq_ids: &mut HashSet<String>,
        bin_media_paths: &mut Vec<String>,
        bin_seq_ids: &mut Vec<String>,
    ) {
        for child in container.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "bin" => {
                    self.parse_bin(child, parent_bin_id, bins, sequences, seen_seq_ids);
                }
                "sequence" => {
                    if let Some(seq) = self.parse_sequence(child, seen_seq_ids) {
                        bin_seq_ids.push(seq.id.clone());
                        sequences.push(seq);
                    }
                }
                "clip" => {
                    self.register_master_clip(child, bin_media_paths);
                }
                _ => {}
            }
        }
    }

    fn parse_bin(
        &mut self,
        bin_node: Node<'_, '_>,
        parent_bin_id: Option<&str>,
        bins: &mut Vec<ProjectBin>,
        sequences: &mut Vec<Sequence>,
        seen_seq_ids: &mut HashSet<String>,
    ) {
        let bin_id = bin_node
            .attribute("id")
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let name = child_text(bin_node, "name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Bin".to_string());

        let mut media_paths = Vec::new();
        let mut sequence_ids = Vec::new();
        let insert_pos = bins.len();
        bins.push(ProjectBin {
            id: bin_id.clone(),
            name,
            parent_id: parent_bin_id.map(ToString::to_string),
            media_paths: Vec::new(),
            sequence_ids: Vec::new(),
            custom_metadata: BTreeMap::new(),
        });

        if let Some(children_node) = child_elem(bin_node, "children") {
            self.walk_container(
                children_node,
                Some(&bin_id),
                bins,
                sequences,
                seen_seq_ids,
                &mut media_paths,
                &mut sequence_ids,
            );
        }

        bins[insert_pos].media_paths = media_paths;
        bins[insert_pos].sequence_ids = sequence_ids;
    }

    fn register_master_clip(&mut self, clip_node: Node<'_, '_>, bin_media_paths: &mut Vec<String>) {
        let mut logging_meta = parse_logging_info(clip_node);
        let clip_name = child_text(clip_node, "name").unwrap_or_default();

        // Find first <file> descendant inside the master <clip>
        let file_node = clip_node
            .descendants()
            .find(|n| n.is_element() && n.has_tag_name("file"));
        if let Some(fn_node) = file_node {
            let mut info = if let Some(fid) = fn_node.attribute("id") {
                self.files_by_id.get(fid).cloned().unwrap_or_default()
            } else {
                Fcp7FileInfo::default()
            };
            merge_file_node(&mut info, fn_node);
            if info.name.is_empty() {
                info.name = clip_name;
            }
            for (k, v) in logging_meta.drain(..) {
                info.metadata.entry(k).or_insert(v);
            }
            if !info.path.is_empty() && !bin_media_paths.contains(&info.path) {
                bin_media_paths.push(info.path.clone());
            }
            self.ensure_asset_from_file(&info, TrackKind::Video, Rational::FPS_25, 1920, 1080);
        }
    }

    fn parse_sequence(
        &mut self,
        seq_node: Node<'_, '_>,
        seen_seq_ids: &mut HashSet<String>,
    ) -> Option<Sequence> {
        // Ignore empty reference-only <sequence id="..."/> shells
        if child_elem(seq_node, "media").is_none() && child_text(seq_node, "name").is_none() {
            return None;
        }
        let seq_id = seq_node
            .attribute("id")
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if !seen_seq_ids.insert(seq_id.clone()) {
            return None;
        }

        let name = child_text(seq_node, "name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Untitled Sequence".to_string());

        let media_node = child_elem(seq_node, "media");
        let video_node = media_node.and_then(|m| child_elem(m, "video"));
        let audio_node = media_node.and_then(|m| child_elem(m, "audio"));

        let vid_sample = video_node
            .and_then(|v| child_elem(v, "format"))
            .and_then(|f| child_elem(f, "samplecharacteristics"));

        let seq_fps = child_elem(seq_node, "rate")
            .and_then(parse_rate)
            .or_else(|| vid_sample.and_then(|s| child_elem(s, "rate")).and_then(parse_rate))
            .unwrap_or(Rational::FPS_25);

        let width = vid_sample
            .and_then(|s| child_i64(s, "width"))
            .filter(|&w| w > 0)
            .map(|w| w as u32)
            .unwrap_or(1920);
        let height = vid_sample
            .and_then(|s| child_i64(s, "height"))
            .filter(|&h| h > 0)
            .map(|h| h as u32)
            .unwrap_or(1080);

        let start_tc_frame = child_elem(seq_node, "timecode")
            .and_then(|tc| parse_timecode_frame(tc, seq_fps))
            .unwrap_or(0);

        let declared_dur = child_i64(seq_node, "duration").unwrap_or(0).max(0);

        // Collect link pairs across all clipitems in this sequence for Union-Find grouping
        let mut link_pairs: Vec<(String, String)> = Vec::new();
        let mut video_tracks = Vec::new();
        let mut audio_tracks = Vec::new();

        if let Some(vnode) = video_node {
            for (idx, tr_node) in vnode
                .children()
                .filter(|n| n.is_element() && n.has_tag_name("track"))
                .enumerate()
            {
                let track = self.parse_track(
                    tr_node,
                    TrackKind::Video,
                    idx + 1,
                    seq_fps,
                    width,
                    height,
                    seen_seq_ids,
                    &mut link_pairs,
                );
                video_tracks.push(track);
            }
        }

        if let Some(anode) = audio_node {
            for (idx, tr_node) in anode
                .children()
                .filter(|n| n.is_element() && n.has_tag_name("track"))
                .enumerate()
            {
                let track = self.parse_track(
                    tr_node,
                    TrackKind::Audio,
                    idx + 1,
                    seq_fps,
                    width,
                    height,
                    seen_seq_ids,
                    &mut link_pairs,
                );
                audio_tracks.push(track);
            }
        }

        // Resolve linked clip groups via Union-Find
        let clip_to_group = build_link_groups(&link_pairs);
        if !clip_to_group.is_empty() {
            for track in video_tracks.iter_mut().chain(audio_tracks.iter_mut()) {
                for clip in &mut track.clips {
                    if let Some(group_id) = clip_to_group.get(&clip.id) {
                        clip.link_group_id = Some(group_id.clone());
                    }
                }
            }
        }

        let markers: Vec<Marker> = seq_node
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("marker"))
            .filter_map(|m| parse_marker(m, MarkerScope::Sequence))
            .collect();

        let mut seq = Sequence {
            id: seq_id,
            name,
            kind: SequenceKind::Timeline,
            width,
            height,
            frame_rate: seq_fps,
            duration_frames: declared_dur,
            start_tc_frame,
            mark_in: None,
            mark_out: None,
            video_tracks,
            audio_tracks,
            audio_buses: Vec::new(),
            markers,
            metadata: BTreeMap::new(),
        };
        seq.duration_frames = seq.computed_duration_frames();
        Some(seq)
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_track(
        &mut self,
        track_node: Node<'_, '_>,
        kind: TrackKind,
        index: usize,
        seq_fps: Rational,
        seq_w: u32,
        seq_h: u32,
        seen_seq_ids: &mut HashSet<String>,
        link_pairs: &mut Vec<(String, String)>,
    ) -> Track {
        let enabled = child_bool(track_node, "enabled").unwrap_or(true);
        let locked = child_bool(track_node, "locked").unwrap_or(false);
        let muted = child_bool(track_node, "muted").unwrap_or(false);
        let name = child_text(track_node, "MZ.TrackName")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| match kind {
                TrackKind::Video => format!("V{index}"),
                TrackKind::Audio => format!("A{index}"),
            });

        // Walk direct <clipitem> and <transitionitem> children in DOM order so -1 boundaries
        // on clips adjacent to transitions resolve accurately.
        enum TrackElem<'a, 'b> {
            Clip(Node<'a, 'b>),
            Transition(i64), // cut_frame
        }

        let mut elems: Vec<TrackElem<'_, '_>> = Vec::new();
        let mut transitions: Vec<Transition> = Vec::new();

        for ch in track_node.children().filter(|n| n.is_element()) {
            match ch.tag_name().name() {
                "clipitem" => elems.push(TrackElem::Clip(ch)),
                "transitionitem" => {
                    if let Some((tr, cut_frame)) = parse_transition_item(ch, seq_fps, kind) {
                        transitions.push(tr);
                        elems.push(TrackElem::Transition(cut_frame));
                    }
                }
                _ => {}
            }
        }

        let mut clips = Vec::new();
        for (i, elem) in elems.iter().enumerate() {
            let TrackElem::Clip(ci_node) = elem else {
                continue;
            };
            let prev_cut = if i > 0 {
                match &elems[i - 1] {
                    TrackElem::Transition(cut) => Some(*cut),
                    _ => None,
                }
            } else {
                None
            };
            let next_cut = if i + 1 < elems.len() {
                match &elems[i + 1] {
                    TrackElem::Transition(cut) => Some(*cut),
                    _ => None,
                }
            } else {
                None
            };

            if let Some(clip) = self.parse_clipitem(
                *ci_node,
                kind,
                seq_fps,
                seq_w,
                seq_h,
                prev_cut,
                next_cut,
                seen_seq_ids,
                link_pairs,
            ) {
                clips.push(clip);
            }
        }

        let audio_channels = if kind == TrackKind::Audio {
            if track_node
                .attribute("PremiereTrackType")
                .map(|t| t.eq_ignore_ascii_case("Mono"))
                .unwrap_or(false)
            {
                1
            } else {
                2
            }
        } else {
            0
        };

        Track {
            index,
            kind,
            name,
            enabled,
            locked,
            muted,
            solo: false,
            audio_channels,
            output_bus_id: None,
            clips,
            transitions,
            markers: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_clipitem(
        &mut self,
        node: Node<'_, '_>,
        track_kind: TrackKind,
        seq_fps: Rational,
        seq_w: u32,
        seq_h: u32,
        prev_transition_cut: Option<i64>,
        next_transition_cut: Option<i64>,
        seen_seq_ids: &mut HashSet<String>,
        link_pairs: &mut Vec<(String, String)>,
    ) -> Option<Clip> {
        let clip_id = node
            .attribute("id")
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| self.alloc_clip_id());

        let clip_fps = child_elem(node, "rate")
            .and_then(parse_rate)
            .unwrap_or(seq_fps);

        let raw_start = child_i64(node, "start").unwrap_or(-1);
        let raw_end = child_i64(node, "end").unwrap_or(-1);
        let raw_dur = child_i64(node, "duration").unwrap_or(0);

        let start_frame = if raw_start >= 0 {
            convert_frames(raw_start, clip_fps, seq_fps)
        } else {
            prev_transition_cut.unwrap_or(0)
        };
        let end_frame = if raw_end >= 0 {
            convert_frames(raw_end, clip_fps, seq_fps)
        } else if let Some(cut) = next_transition_cut {
            cut
        } else {
            start_frame + convert_frames(raw_dur, clip_fps, seq_fps).max(1)
        };
        if end_frame <= start_frame {
            return None;
        }

        // Collect <link><linkclipref> pairs for this clipitem
        for link in node
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("link"))
        {
            if let Some(target_ref) = child_text(link, "linkclipref").filter(|s| !s.is_empty())
                && target_ref != clip_id
            {
                link_pairs.push((clip_id.clone(), target_ref));
            }
        }

        let name = child_text(node, "name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Clip".to_string());
        let enabled = child_bool(node, "enabled").unwrap_or(true);
        let audio_channel = child_elem(node, "sourcetrack")
            .and_then(|st| child_i64(st, "trackindex"))
            .filter(|&i| i > 0)
            .map(|i| i as u32)
            .unwrap_or(if track_kind == TrackKind::Audio { 1 } else { 0 });

        let blend_mode = child_text(node, "compositemode")
            .and_then(|s| BlendMode::parse_name(&s))
            .unwrap_or(BlendMode::Normal);

        let label_name = child_elem(node, "labels").and_then(|lbl| {
            child_text(lbl, "label2")
                .or_else(|| child_text(lbl, "label"))
                .filter(|s| !s.is_empty())
        });

        // Resolve source: either nested <sequence> or <file>
        let (source_kind, media_id, media_path, media_kind, source_fps, source_w, source_h) =
            if let Some(nested_seq_node) = child_elem(node, "sequence") {
                let nested_id = nested_seq_node
                    .attribute("id")
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                if let Some(nested_seq) = self.parse_sequence(nested_seq_node, seen_seq_ids) {
                    self.extra_sequences.push(nested_seq);
                }
                (
                    ClipSourceKind::NestedSequence,
                    nested_id,
                    String::new(),
                    if track_kind == TrackKind::Audio {
                        MediaKind::Audio
                    } else {
                        MediaKind::Video
                    },
                    clip_fps,
                    if track_kind == TrackKind::Video { seq_w } else { 0 },
                    if track_kind == TrackKind::Video { seq_h } else { 0 },
                )
            } else if let Some(file_node) = child_elem(node, "file") {
                let mut finfo = file_node
                    .attribute("id")
                    .and_then(|fid| self.files_by_id.get(fid))
                    .cloned()
                    .unwrap_or_default();
                merge_file_node(&mut finfo, file_node);
                if finfo.name.is_empty() {
                    finfo.name = name.clone();
                }
                for (k, v) in parse_logging_info(node) {
                    finfo.metadata.entry(k).or_insert(v);
                }
                let src_fps = finfo.frame_rate.unwrap_or(clip_fps);
                let mid = self.ensure_asset_from_file(&finfo, track_kind, src_fps, seq_w, seq_h);
                let mkind = infer_file_media_kind(&finfo, track_kind);
                let (sw, sh) = if mkind == MediaKind::Audio {
                    (0, 0)
                } else {
                    (
                        if finfo.width > 0 { finfo.width } else { seq_w },
                        if finfo.height > 0 { finfo.height } else { seq_h },
                    )
                };
                (
                    ClipSourceKind::Media,
                    mid,
                    finfo.path,
                    mkind,
                    src_fps,
                    sw,
                    sh,
                )
            } else {
                // Generator / slug / offline clip without <file>
                let mkind = if track_kind == TrackKind::Audio {
                    MediaKind::Audio
                } else {
                    MediaKind::Video
                };
                (
                    ClipSourceKind::Media,
                    String::new(),
                    String::new(),
                    mkind,
                    clip_fps,
                    if track_kind == TrackKind::Video { seq_w } else { 0 },
                    if track_kind == TrackKind::Video { seq_h } else { 0 },
                )
            };

        let raw_in = child_i64(node, "in").unwrap_or(-1);
        let raw_out = child_i64(node, "out").unwrap_or(-1);
        let tl_dur = (end_frame - start_frame).max(1);
        let source_in_frame = if raw_in >= 0 {
            convert_frames(raw_in, clip_fps, source_fps)
        } else {
            0
        };
        let mut source_out_frame = if raw_out >= 0 {
            convert_frames(raw_out, clip_fps, source_fps)
        } else {
            source_in_frame + convert_frames(tl_dur, seq_fps, source_fps)
        };

        // Parse <filter> children (Basic Motion, Crop, Opacity, Time Remap, and generic effects)
        let mut motion = MotionTransform::default();
        let mut opacity = 100.0;
        let mut speed_percent = 100.0;
        let mut reversed = false;
        let mut speed_keyframes: Vec<SpeedKeyframe> = Vec::new();
        let mut retime_interpolation = RetimeInterpolation::Default;
        let mut effects: Vec<EffectInstance> = Vec::new();

        for (fi, filter_node) in node
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("filter"))
            .enumerate()
        {
            let filter_enabled = child_bool(filter_node, "enabled").unwrap_or(true);
            let Some(eff_node) = child_elem(filter_node, "effect") else {
                continue;
            };
            let eff_id = child_text(eff_node, "effectid").unwrap_or_default();
            let eff_name = child_text(eff_node, "name")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| eff_id.clone());
            let eff_cat = child_text(eff_node, "effectcategory").unwrap_or_default();
            let eff_media = child_text(eff_node, "mediatype").unwrap_or_default();

            match eff_id.to_ascii_lowercase().as_str() {
                "timeremap" | "time remap" => {
                    if filter_enabled {
                        parse_timeremap_filter(
                            eff_node,
                            source_fps,
                            &mut speed_percent,
                            &mut reversed,
                            &mut retime_interpolation,
                            &mut speed_keyframes,
                        );
                        if speed_percent.abs() < 0.01 {
                            source_out_frame = source_in_frame;
                        }
                    }
                }
                "basic" => {
                    if filter_enabled {
                        parse_basic_motion_filter(eff_node, &mut motion);
                    }
                }
                "crop" => {
                    if filter_enabled {
                        parse_crop_filter(eff_node, &mut motion);
                    }
                }
                "opacity" => {
                    if filter_enabled {
                        let animated = parse_opacity_filter(eff_node, &mut opacity);
                        if animated {
                            effects.push(build_effect_instance(
                                fi,
                                "AE.ADBE Opacity",
                                "Opacity",
                                EffectCategory::Transform,
                                TrackKind::Video,
                                true,
                                true,
                                eff_node,
                            ));
                        }
                    }
                }
                "audiolevels" | "audiopan" => {
                    // Standard intrinsic audio level / pan filters
                }
                _ => {
                    let media_kind = if eff_media.eq_ignore_ascii_case("audio")
                        || track_kind == TrackKind::Audio
                    {
                        TrackKind::Audio
                    } else {
                        TrackKind::Video
                    };
                    let match_name = if eff_id.is_empty() {
                        eff_name.clone()
                    } else {
                        eff_id.clone()
                    };
                    let has_kf = eff_node
                        .descendants()
                        .any(|n| n.is_element() && n.has_tag_name("keyframe"));
                    effects.push(build_effect_instance(
                        fi,
                        &match_name,
                        &eff_name,
                        guess_fcp7_effect_category(&eff_cat, &match_name, &eff_name, media_kind),
                        media_kind,
                        filter_enabled,
                        has_kf,
                        eff_node,
                    ));
                }
            }
        }

        let clip_markers: Vec<Marker> = node
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("marker"))
            .filter_map(|m| parse_marker(m, MarkerScope::Clip))
            .collect();

        Some(Clip {
            id: clip_id,
            name,
            media_id,
            media_path,
            media_kind,
            source_kind,
            active_angle: None,
            start_frame,
            end_frame,
            source_in_frame,
            source_out_frame,
            source_in_ticks: None,
            source_out_ticks: None,
            source_fps,
            source_width: source_w,
            source_height: source_h,
            speed_percent,
            reversed,
            speed_keyframes,
            retime_interpolation,
            enabled,
            audio_channel,
            link_group_id: None,
            label_name,
            label_preset_index: None,
            motion,
            opacity,
            blend_mode,
            lumetri_effects: Vec::new(),
            effects,
            markers: clip_markers,
        })
    }

    fn ensure_asset_from_file(
        &mut self,
        finfo: &Fcp7FileInfo,
        track_kind: TrackKind,
        fallback_fps: Rational,
        fallback_w: u32,
        fallback_h: u32,
    ) -> String {
        let key = if !finfo.path.is_empty() {
            finfo.path.clone()
        } else if !finfo.id.is_empty() {
            format!("id:{}", finfo.id)
        } else {
            format!("name:{}", finfo.name)
        };
        if let Some(&idx) = self.asset_idx_by_key.get(&key) {
            let asset = &mut self.media_assets[idx];
            if finfo.duration_frames > asset.duration_frames {
                asset.duration_frames = finfo.duration_frames;
            }
            if asset.source_tc_frame == 0 && finfo.source_tc_frame > 0 {
                asset.source_tc_frame = finfo.source_tc_frame;
                asset.source_tc_string =
                    frames_to_timecode(finfo.source_tc_frame, asset.frame_rate);
            }
            if asset.width == 0 && finfo.width > 0 {
                asset.width = finfo.width;
                asset.height = finfo.height;
            }
            if asset.audio_channels == 0 && finfo.audio_channels > 0 {
                asset.audio_channels = finfo.audio_channels;
                asset.audio_sample_rate = finfo.audio_sample_rate.max(48_000);
            }
            for (k, v) in &finfo.metadata {
                asset.metadata.entry(k.clone()).or_insert_with(|| v.clone());
            }
            return asset.id.clone();
        }

        let fps = finfo.frame_rate.unwrap_or(fallback_fps);
        let mkind = infer_file_media_kind(finfo, track_kind);
        let id = if !finfo.id.is_empty() {
            finfo.id.clone()
        } else {
            format!("fcp7-asset-{}", self.media_assets.len() + 1)
        };
        let mut metadata = finfo.metadata.clone();
        if let Some(reel) = finfo.reel_name.as_deref().filter(|s| !s.is_empty()) {
            metadata
                .entry("Tape Name".to_string())
                .or_insert_with(|| reel.to_string());
        }
        let asset_name = if !finfo.name.is_empty() {
            finfo.name.clone()
        } else {
            Path::new(&finfo.path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Media")
                .to_string()
        };
        let audio_ch = if finfo.audio_channels > 0 {
            finfo.audio_channels
        } else if finfo.has_audio || mkind == MediaKind::Audio {
            2
        } else {
            0
        };

        let asset = MediaAsset {
            id: id.clone(),
            name: asset_name,
            path: finfo.path.clone(),
            media_kind: mkind,
            width: if mkind == MediaKind::Audio {
                0
            } else if finfo.width > 0 {
                finfo.width
            } else {
                fallback_w
            },
            height: if mkind == MediaKind::Audio {
                0
            } else if finfo.height > 0 {
                finfo.height
            } else {
                fallback_h
            },
            frame_rate: fps,
            duration_frames: finfo.duration_frames.max(0),
            source_tc_frame: finfo.source_tc_frame.max(0),
            source_tc_string: frames_to_timecode(finfo.source_tc_frame.max(0), fps),
            audio_channels: audio_ch,
            audio_sample_rate: if audio_ch > 0 {
                finfo.audio_sample_rate.max(48_000)
            } else {
                0
            },
            color_space: None,
            video_codec: None,
            audio_codec: None,
            alpha_mode: AlphaMode::Default,
            data_levels: DataLevels::Auto,
            pixel_aspect_ratio: finfo.pixel_aspect_ratio,
            metadata,
            custom_metadata: BTreeMap::new(),
        };
        let idx = self.media_assets.len();
        self.media_assets.push(asset);
        self.asset_idx_by_key.insert(key, idx);
        id
    }
}

// ── Element Parsers ─────────────────────────────────────────────────────

fn merge_file_node(info: &mut Fcp7FileInfo, node: Node<'_, '_>) {
    if let Some(n) = child_text(node, "name").filter(|s| !s.is_empty())
        && info.name.is_empty()
    {
        info.name = n;
    }
    if let Some(url) = child_text(node, "pathurl").filter(|s| !s.is_empty()) {
        let decoded = file_url_to_path(&url);
        if !decoded.is_empty() {
            info.path = decoded;
        }
    }
    if let Some(r) = child_elem(node, "rate").and_then(parse_rate) {
        info.frame_rate = Some(r);
    }
    if let Some(d) = child_i64(node, "duration").filter(|&d| d > 0) {
        info.duration_frames = info.duration_frames.max(d);
    }
    if let Some(tc_node) = child_elem(node, "timecode") {
        let fps = info.frame_rate.unwrap_or(Rational::FPS_25);
        if let Some(tc_f) = parse_timecode_frame(tc_node, fps) {
            info.source_tc_frame = tc_f;
        }
        if let Some(reel) = child_elem(tc_node, "reel")
            .and_then(|r| child_text(r, "name"))
            .filter(|s| !s.is_empty())
        {
            info.reel_name = Some(reel);
        }
    }
    if let Some(media) = child_elem(node, "media") {
        if let Some(vid) = child_elem(media, "video") {
            info.has_video = true;
            if let Some(sc) = child_elem(vid, "samplecharacteristics") {
                if let Some(w) = child_i64(sc, "width").filter(|&w| w > 0) {
                    info.width = w as u32;
                }
                if let Some(h) = child_i64(sc, "height").filter(|&h| h > 0) {
                    info.height = h as u32;
                }
                if info.frame_rate.is_none()
                    && let Some(r) = child_elem(sc, "rate").and_then(parse_rate)
                {
                    info.frame_rate = Some(r);
                }
                if let Some(par_str) = child_text(sc, "pixelaspectratio") {
                    info.pixel_aspect_ratio = parse_pixel_aspect_ratio(&par_str);
                }
            }
        }
        for aud in media
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("audio"))
        {
            info.has_audio = true;
            if let Some(ch) = child_i64(aud, "channelcount").filter(|&c| c > 0) {
                info.audio_channels = info.audio_channels.max(ch as u32);
            }
            if let Some(sr) = child_elem(aud, "samplecharacteristics")
                .and_then(|sc| child_i64(sc, "samplerate"))
                .filter(|&r| r > 0)
            {
                info.audio_sample_rate = sr as u32;
            }
        }
    }
}

fn infer_file_media_kind(info: &Fcp7FileInfo, track_kind: TrackKind) -> MediaKind {
    if let Some(ext) = Path::new(&info.path).extension().and_then(|s| s.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "tif" | "tiff" | "psd" | "bmp" | "webp" | "heic" | "exr"
            | "dpx" | "tga" => return MediaKind::Image,
            "wav" | "aif" | "aiff" | "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "bwf" => {
                return MediaKind::Audio;
            }
            _ => {}
        }
    }
    if info.has_video {
        MediaKind::Video
    } else if info.has_audio || track_kind == TrackKind::Audio {
        MediaKind::Audio
    } else {
        MediaKind::Video
    }
}

fn parse_rate(rate_node: Node<'_, '_>) -> Option<Rational> {
    let tb = child_i64(rate_node, "timebase").filter(|&t| t > 0)?;
    let ntsc = child_bool(rate_node, "ntsc").unwrap_or(false);
    if ntsc {
        match tb {
            24 => Some(Rational::FPS_23_976),
            30 => Some(Rational::FPS_29_97),
            60 => Some(Rational::FPS_59_94),
            _ => Some(Rational::new(tb * 1000, 1001)),
        }
    } else {
        Some(Rational::new(tb, 1))
    }
}

fn parse_timecode_frame(tc_node: Node<'_, '_>, fallback_fps: Rational) -> Option<i64> {
    let fps = child_elem(tc_node, "rate")
        .and_then(parse_rate)
        .unwrap_or(fallback_fps);
    if let Some(f) = child_i64(tc_node, "frame").filter(|&f| f >= 0) {
        return Some(f);
    }
    child_text(tc_node, "string").and_then(|s| timecode_to_frames(&s, fps))
}

fn parse_pixel_aspect_ratio(raw: &str) -> Option<f64> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "square" => None,
        "ntsc-601" => Some(0.9091),
        "pal-601" => Some(1.0940),
        "hd-(960x720)" | "hd-(1280x1080)" | "hd-(1440x1080)" | "dvcpprohd-1080" => Some(1.3333),
        other => other.parse::<f64>().ok().filter(|&v| v > 0.0 && (v - 1.0).abs() > 1e-4),
    }
}

fn parse_transition_item(
    node: Node<'_, '_>,
    seq_fps: Rational,
    track_kind: TrackKind,
) -> Option<(Transition, i64)> {
    let tr_fps = child_elem(node, "rate")
        .and_then(parse_rate)
        .unwrap_or(seq_fps);
    let raw_start = child_i64(node, "start")?;
    let raw_end = child_i64(node, "end")?;
    let start_frame = convert_frames(raw_start, tr_fps, seq_fps).max(0);
    let end_frame = convert_frames(raw_end, tr_fps, seq_fps);
    if end_frame <= start_frame {
        return None;
    }

    let raw_align = child_text(node, "alignment").unwrap_or_else(|| "center".into());
    let (alignment, cut_frame) = match raw_align.trim().to_ascii_lowercase().as_str() {
        "start" | "start-black" => ("start".to_string(), start_frame),
        "end" | "end-black" => ("end".to_string(), end_frame),
        _ => ("center".to_string(), (start_frame + end_frame) / 2),
    };

    let eff = child_elem(node, "effect");
    let raw_name = eff
        .and_then(|e| child_text(e, "name"))
        .unwrap_or_default();
    let raw_id = eff
        .and_then(|e| child_text(e, "effectid"))
        .unwrap_or_default();

    let default_name = if track_kind == TrackKind::Audio {
        "Constant Power"
    } else {
        "Cross Dissolve"
    };
    let name = if raw_name.is_empty() {
        if raw_id.is_empty() {
            default_name.to_string()
        } else {
            raw_id.clone()
        }
    } else {
        raw_name
    };
    let effect_id = if raw_id.is_empty() {
        name.clone()
    } else {
        raw_id
    };

    Some((
        Transition {
            name,
            effect_id,
            start_frame,
            end_frame,
            alignment,
        },
        cut_frame,
    ))
}

fn parse_marker(node: Node<'_, '_>, scope: MarkerScope) -> Option<Marker> {
    let in_f = child_i64(node, "in")?;
    let out_f = child_i64(node, "out").unwrap_or(-1);
    let duration_frames = if out_f > in_f { out_f - in_f } else { 1 };
    let name = child_text(node, "name").unwrap_or_default();
    let comment = child_text(node, "comment").unwrap_or_default();
    let color = child_elem(node, "color")
        .and_then(parse_marker_rgb_color)
        .unwrap_or_else(|| "Blue".to_string());

    Some(Marker {
        frame: in_f.max(0),
        duration_frames,
        name,
        comment,
        color,
        scope,
    })
}

fn parse_marker_rgb_color(color_node: Node<'_, '_>) -> Option<String> {
    let r = child_i64(color_node, "red")? as i32;
    let g = child_i64(color_node, "green")? as i32;
    let b = child_i64(color_node, "blue")? as i32;

    const PALETTE: &[(&str, (i32, i32, i32))] = &[
        ("Red", (255, 45, 45)),
        ("Green", (45, 200, 75)),
        ("Blue", (55, 130, 245)),
        ("Cyan", (45, 205, 235)),
        ("Yellow", (250, 215, 40)),
        ("Orange", (255, 145, 35)),
        ("Purple", (170, 80, 235)),
        ("Pink", (240, 85, 180)),
        ("White", (240, 240, 240)),
    ];

    PALETTE
        .iter()
        .min_by_key(|(_, (pr, pg, pb))| {
            let dr = r - pr;
            let dg = g - pg;
            let db = b - pb;
            dr * dr + dg * dg + db * db
        })
        .map(|(name, _)| (*name).to_string())
}

fn parse_timeremap_filter(
    eff_node: Node<'_, '_>,
    source_fps: Rational,
    speed_percent: &mut f64,
    reversed: &mut bool,
    retime_interp: &mut RetimeInterpolation,
    speed_keyframes: &mut Vec<SpeedKeyframe>,
) {
    let mut is_variable = false;
    for param in eff_node
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("parameter"))
    {
        let pid = child_text(param, "parameterid")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let val = child_text(param, "value").unwrap_or_default();
        match pid.as_str() {
            "speed" => {
                if let Ok(s) = val.parse::<f64>() {
                    if s < 0.0 {
                        *reversed = true;
                        *speed_percent = s.abs();
                    } else {
                        *speed_percent = s;
                    }
                }
            }
            "reverse" => {
                if val.eq_ignore_ascii_case("true") {
                    *reversed = true;
                }
            }
            "frameblending" => {
                if val.eq_ignore_ascii_case("true") {
                    *retime_interp = RetimeInterpolation::FrameBlending;
                }
            }
            "variablespeed" => {
                is_variable = val.eq_ignore_ascii_case("true") || val == "1";
            }
            "graphdict" => {
                let fps_f = source_fps.as_f64().max(1.0);
                for kf in param
                    .children()
                    .filter(|n| n.is_element() && n.has_tag_name("keyframe"))
                {
                    if let (Some(when), Some(v)) =
                        (child_f64(kf, "when"), child_f64(kf, "value"))
                    {
                        speed_keyframes.push(SpeedKeyframe {
                            timeline_sec: when / fps_f,
                            source_sec: v / fps_f,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    if !is_variable && speed_keyframes.len() <= 2 {
        speed_keyframes.clear();
    }
}

fn parse_basic_motion_filter(eff_node: Node<'_, '_>, motion: &mut MotionTransform) {
    for param in eff_node
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("parameter"))
    {
        let pid = child_text(param, "parameterid")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if param
            .children()
            .any(|n| n.is_element() && n.has_tag_name("keyframe"))
        {
            motion.has_keyframes = true;
        }
        match pid.as_str() {
            "scale" => {
                if let Some(v) = param_scalar(param) {
                    motion.scale = v;
                }
            }
            "rotation" => {
                if let Some(v) = param_scalar(param) {
                    motion.rotation = v;
                }
            }
            "center" => {
                if let Some((h, v)) = param_horiz_vert(param) {
                    motion.position_x = h;
                    motion.position_y = -v;
                }
            }
            "anchorpoint" => {
                if let Some((h, v)) = param_horiz_vert(param) {
                    motion.anchor_x = h;
                    motion.anchor_y = v;
                }
            }
            _ => {}
        }
    }
}

fn parse_crop_filter(eff_node: Node<'_, '_>, motion: &mut MotionTransform) {
    for param in eff_node
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("parameter"))
    {
        let pid = child_text(param, "parameterid")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let Some(v) = param_scalar(param) else {
            continue;
        };
        match pid.as_str() {
            "left" => motion.crop_left = v.clamp(0.0, 100.0),
            "right" => motion.crop_right = v.clamp(0.0, 100.0),
            "top" => motion.crop_top = v.clamp(0.0, 100.0),
            "bottom" => motion.crop_bottom = v.clamp(0.0, 100.0),
            _ => {}
        }
    }
}

fn parse_opacity_filter(eff_node: Node<'_, '_>, opacity: &mut f64) -> bool {
    let mut animated = false;
    for param in eff_node
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("parameter"))
    {
        let pid = child_text(param, "parameterid")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if pid == "opacity" {
            if param
                .children()
                .any(|n| n.is_element() && n.has_tag_name("keyframe"))
            {
                animated = true;
            }
            if let Some(v) = param_scalar(param) {
                *opacity = v.clamp(0.0, 100.0);
            }
        }
    }
    animated
}

#[allow(clippy::too_many_arguments)]
fn build_effect_instance(
    idx: usize,
    match_name: &str,
    display_name: &str,
    category: EffectCategory,
    media_kind: TrackKind,
    enabled: bool,
    is_animated: bool,
    eff_node: Node<'_, '_>,
) -> EffectInstance {
    let mut parameters = Vec::new();
    for param in eff_node
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("parameter"))
    {
        let pid = child_text(param, "parameterid").unwrap_or_default();
        let pname = child_text(param, "name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| pid.clone());
        let kf = param
            .children()
            .any(|n| n.is_element() && n.has_tag_name("keyframe"));
        let val = child_text(param, "value").unwrap_or_default();
        if !pname.is_empty() {
            parameters.push(EffectParameter {
                id: pid,
                name: pname,
                value: val,
                is_animated: kf,
            });
        }
    }
    EffectInstance {
        id: format!("fcp7-fx-{}", idx + 1),
        match_name: match_name.to_string(),
        display_name: display_name.to_string(),
        category,
        media_kind,
        enabled,
        is_intrinsic: false,
        is_animated,
        parameters,
    }
}

fn parse_logging_info(node: Node<'_, '_>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(li) = child_elem(node, "logginginfo") else {
        return out;
    };
    for (tag, label) in [
        ("description", "Description"),
        ("scene", "Scene"),
        ("shottake", "Shot"),
        ("lognote", "Log Note"),
        ("angle", "Angle"),
    ] {
        if let Some(v) = child_text(li, tag).filter(|s| !s.is_empty()) {
            out.push((label.to_string(), v));
        }
    }
    out
}

fn build_link_groups(pairs: &[(String, String)]) -> HashMap<String, String> {
    if pairs.is_empty() {
        return HashMap::new();
    }
    let mut parent: HashMap<String, String> = HashMap::new();

    fn find(parent: &mut HashMap<String, String>, x: &str) -> String {
        let mut cur = x.to_string();
        while let Some(p) = parent.get(&cur) {
            if p == &cur {
                break;
            }
            cur = p.clone();
        }
        parent.insert(x.to_string(), cur.clone());
        cur
    }

    for (a, b) in pairs {
        parent.entry(a.clone()).or_insert_with(|| a.clone());
        parent.entry(b.clone()).or_insert_with(|| b.clone());
        let ra = find(&mut parent, a);
        let rb = find(&mut parent, b);
        if ra != rb {
            // Deterministic union by lexicographic order
            if ra < rb {
                parent.insert(rb, ra);
            } else {
                parent.insert(ra, rb);
            }
        }
    }

    let keys: Vec<String> = parent.keys().cloned().collect();
    let mut root_to_group: HashMap<String, String> = HashMap::new();
    let mut sorted_roots: Vec<String> = keys
        .iter()
        .map(|k| find(&mut parent, k))
        .collect();
    sorted_roots.sort();
    sorted_roots.dedup();
    for (idx, r) in sorted_roots.into_iter().enumerate() {
        root_to_group.insert(r, format!("fcp7-link-{}", idx + 1));
    }

    let mut out = HashMap::new();
    for k in keys {
        let r = find(&mut parent, &k);
        if let Some(g) = root_to_group.get(&r) {
            out.insert(k, g.clone());
        }
    }
    out
}

fn param_scalar(param: Node<'_, '_>) -> Option<f64> {
    child_f64(param, "value").or_else(|| {
        param
            .children()
            .find(|n| n.is_element() && n.has_tag_name("keyframe"))
            .and_then(|kf| child_f64(kf, "value"))
    })
}

fn param_horiz_vert(param: Node<'_, '_>) -> Option<(f64, f64)> {
    let val_node = child_elem(param, "value").or_else(|| {
        param
            .children()
            .find(|n| n.is_element() && n.has_tag_name("keyframe"))
            .and_then(|kf| child_elem(kf, "value"))
    })?;
    let h = child_f64(val_node, "horiz")?;
    let v = child_f64(val_node, "vert")?;
    Some((h, v))
}

fn child_elem<'a, 'b>(node: Node<'a, 'b>, tag: &str) -> Option<Node<'a, 'b>> {
    node.children()
        .find(|n| n.is_element() && n.has_tag_name(tag))
}

fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    child_elem(node, tag).and_then(|n| n.text()).map(|s| s.trim().to_string())
}

fn child_i64(node: Node<'_, '_>, tag: &str) -> Option<i64> {
    child_text(node, tag)?.parse::<i64>().ok()
}

fn child_f64(node: Node<'_, '_>, tag: &str) -> Option<f64> {
    child_text(node, tag)?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

fn child_bool(node: Node<'_, '_>, tag: &str) -> Option<bool> {
    let txt = child_text(node, tag)?;
    if txt.eq_ignore_ascii_case("true") || txt == "1" {
        Some(true)
    } else if txt.eq_ignore_ascii_case("false") || txt == "0" {
        Some(false)
    } else {
        None
    }
}

fn convert_frames(frames: i64, from_fps: Rational, to_fps: Rational) -> i64 {
    if from_fps == to_fps || frames == 0 {
        return frames;
    }
    let num = (frames as i128) * (to_fps.num as i128) * (from_fps.den as i128);
    let den = (from_fps.num as i128) * (to_fps.den as i128);
    if den == 0 {
        return frames;
    }
    if num >= 0 {
        ((num + den / 2) / den) as i64
    } else {
        -(((-num) + den / 2) / den) as i64
    }
}

fn guess_fcp7_effect_category(
    cat: &str,
    match_name: &str,
    display_name: &str,
    kind: TrackKind,
) -> EffectCategory {
    if kind == TrackKind::Audio {
        return EffectCategory::Audio;
    }
    let s = format!(
        "{} {} {}",
        cat.to_ascii_lowercase(),
        match_name.to_ascii_lowercase(),
        display_name.to_ascii_lowercase()
    );
    if s.contains("blur") {
        EffectCategory::Blur
    } else if s.contains("key") {
        EffectCategory::Keying
    } else if s.contains("stabiliz") {
        EffectCategory::Stabilization
    } else if s.contains("color") || s.contains("lumetri") || s.contains("tint") {
        EffectCategory::Color
    } else if s.contains("distort") || s.contains("pin") {
        EffectCategory::Distort
    } else if s.contains("stylize") || s.contains("sharpen") {
        EffectCategory::Stylize
    } else if s.contains("title") || s.contains("text") {
        EffectCategory::Title
    } else if s.contains("motion") || s.contains("transform") {
        EffectCategory::Transform
    } else {
        EffectCategory::Other
    }
}
