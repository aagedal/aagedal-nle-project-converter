use super::time::{file_url_to_path, parse_frame_duration_to_fps, FcpTime};
use nle_core::{
    frames_to_timecode, AlphaMode, BlendMode, Clip, ClipSourceKind, ColorConfig, DataLevels,
    EffectCategory, EffectInstance, EffectParameter, Marker, MarkerScope, MediaAsset, MediaKind,
    MotionTransform, NleError, NleFormat, NleResult, Project, ProjectBin, ProjectReader, Rational,
    RetimeInterpolation, Sequence, SequenceKind, SpeedKeyframe, Track, TrackKind, Transition,
};
use roxmltree::{Document, Node, ParsingOptions};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct FcpXmlReader;

impl ProjectReader for FcpXmlReader {
    fn format(&self) -> NleFormat {
        NleFormat::FinalCutPro
    }

    fn read_file(&self, path: &Path) -> NleResult<Project> {
        read_fcpxml_file(path)
    }
}

/// Read a Final Cut Pro `.fcpxml` file or `.fcpxmld` bundle directory into the canonical `Project` IR.
pub fn read_fcpxml_file(path: &Path) -> NleResult<Project> {
    let (xml_path, default_name) = resolve_fcpxml_input(path)?;
    let xml = fs::read_to_string(&xml_path)?;
    let mut project = parse_fcpxml_str(&xml, &default_name)?;
    project.source_path = Some(path.to_string_lossy().into_owned());
    Ok(project)
}

fn resolve_fcpxml_input(path: &Path) -> NleResult<(PathBuf, String)> {
    if path.is_dir() {
        let default_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Final Cut Pro Project")
            .to_string();

        let info = path.join("Info.fcpxml");
        if info.is_file() {
            return Ok((info, default_name));
        }
        // Fallback: first *.fcpxml inside the bundle directory
        if let Ok(entries) = fs::read_dir(path) {
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|de| de.path()))
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("fcpxml"))
                            .unwrap_or(false)
                })
                .collect();
            candidates.sort();
            if let Some(first) = candidates.into_iter().next() {
                return Ok((first, default_name));
            }
        }
        return Err(NleError::InvalidFormat(format!(
            "No Info.fcpxml found inside bundle '{}'",
            path.display()
        )));
    }

    // If pointing directly at `SomeBundle.fcpxmld/Info.fcpxml`, name after the bundle.
    let default_name = if path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|n| n.eq_ignore_ascii_case("Info.fcpxml"))
        .unwrap_or(false)
    {
        path.parent()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Final Cut Pro Project")
            .to_string()
    } else {
        path.file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Final Cut Pro Project")
            .to_string()
    };

    Ok((path.to_path_buf(), default_name))
}

/// Parse an FCPXML (1.8–1.13+) document string into the canonical `Project` IR.
pub fn parse_fcpxml_str(xml: &str, default_name: &str) -> NleResult<Project> {
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = Document::parse_with_options(xml, opts)
        .map_err(|e| NleError::Xml(format!("Invalid FCPXML document: {e}")))?;

    let root = doc.root_element();
    if root.tag_name().name() != "fcpxml" {
        return Err(NleError::InvalidFormat(format!(
            "Expected root <fcpxml> element, found <{}>",
            root.tag_name().name()
        )));
    }

    let version = root.attribute("version").unwrap_or("1.11").to_string();

    let mut ctx = ParseContext::new();

    // 1. Parse <resources> (formats, assets, effects, and compound/multicam <media> definitions)
    if let Some(res_node) = root.children().find(|n| n.is_element() && n.has_tag_name("resources"))
    {
        ctx.parse_resources(res_node);
    }

    // 2. Parse <library>, <event>, and/or standalone <project>
    let mut project_name = default_name.to_string();
    let mut bins: Vec<ProjectBin> = Vec::new();
    let mut sequences: Vec<Sequence> = Vec::new();

    if let Some(lib) = root.children().find(|n| n.is_element() && n.has_tag_name("library")) {
        if let Some(loc) = lib.attribute("location") {
            let p = file_url_to_path(loc);
            if let Some(stem) = Path::new(&p)
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                && (project_name.is_empty() || project_name == "Info") {
                    project_name = stem.to_string();
                }
        }
        for ev in lib.children().filter(|n| n.is_element() && n.has_tag_name("event")) {
            ctx.parse_event(ev, &mut bins, &mut sequences);
        }
    }

    // Standalone <event> elements at root
    for ev in root.children().filter(|n| n.is_element() && n.has_tag_name("event")) {
        ctx.parse_event(ev, &mut bins, &mut sequences);
    }

    // Standalone <project> elements at root
    for proj in root.children().filter(|n| n.is_element() && n.has_tag_name("project")) {
        if let Some(seq) = ctx.parse_project_node(proj) {
            if project_name.is_empty() || project_name == "Final Cut Pro Project" {
                project_name = seq.name.clone();
            }
            sequences.push(seq);
        }
    }

    // Append compound & multicam sequences from <resources><media> so Resolve/Premiere writers
    // can resolve every CompoundClip / MulticamClip reference.
    let mut existing_seq_ids: HashSet<String> = sequences.iter().map(|s| s.id.clone()).collect();
    for mdef in ctx.media_defs.values() {
        let seq = match mdef {
            MediaDef::Compound(s) => s,
            MediaDef::Multicam { seq, .. } => seq,
        };
        if existing_seq_ids.insert(seq.id.clone()) {
            sequences.push(seq.clone());
        }
    }

    // Build Project.media_assets from parsed <asset> resources (preserving insertion order via sorted key or order vec)
    let mut media_assets: Vec<MediaAsset> = Vec::new();
    for aid in &ctx.asset_order {
        if let Some(a) = ctx.assets.get(aid) {
            media_assets.push(a.to_media_asset(&ctx.formats));
        }
    }

    // Derive ColorConfig from the primary sequence's format (deterministic), falling back to format order
    let mut color_config = ColorConfig::default();
    if let Some(cs) = ctx.primary_color_space.clone() {
        color_config.working_space = Some(cs.clone());
        color_config.output_space = Some(cs);
    } else {
        for fid in &ctx.format_order {
            if let Some(cs) = ctx.formats.get(fid).and_then(|f| f.color_space.clone()) {
                color_config.working_space = Some(cs.clone());
                color_config.output_space = Some(cs);
                break;
            }
        }
    }

    let mut metadata = BTreeMap::new();
    metadata.insert("fcpxml_version".to_string(), version);

    Ok(Project {
        name: project_name,
        source_format: NleFormat::FinalCutPro,
        source_path: None,
        color_config,
        scale_to_frame_size: true,
        bins,
        media_assets,
        sequences,
        metadata,
        custom_metadata_schema: BTreeMap::new(),
    })
}

// ── Internal Resource & Context Structures ──────────────────────────────

#[derive(Debug, Clone)]
struct FormatInfo {
    width: u32,
    height: u32,
    fps: Rational,
    color_space: Option<String>,
}

impl Default for FormatInfo {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: Rational::FPS_25,
            color_space: None,
        }
    }
}

#[derive(Debug, Clone)]
struct AssetInfo {
    id: String,
    name: String,
    path: String,
    media_kind: MediaKind,
    has_video: bool,
    has_audio: bool,
    format_id: Option<String>,
    start: FcpTime,
    duration: FcpTime,
    audio_channels: u32,
    audio_sample_rate: u32,
    metadata: BTreeMap<String, String>,
}

impl AssetInfo {
    fn effective_format(&self, formats: &HashMap<String, FormatInfo>) -> FormatInfo {
        self.format_id
            .as_deref()
            .and_then(|fid| formats.get(fid))
            .cloned()
            .unwrap_or_default()
    }

    fn to_media_asset(&self, formats: &HashMap<String, FormatInfo>) -> MediaAsset {
        let fmt = self.effective_format(formats);
        let fps = fmt.fps;
        let source_tc_frame = self.start.to_frames(fps).max(0);
        let duration_frames = if self.media_kind == MediaKind::Image {
            0
        } else {
            self.duration.to_frames(fps).max(0)
        };
        MediaAsset {
            id: self.id.clone(),
            name: self.name.clone(),
            path: self.path.clone(),
            media_kind: self.media_kind,
            width: if self.has_video { fmt.width } else { 0 },
            height: if self.has_video { fmt.height } else { 0 },
            frame_rate: fps,
            duration_frames,
            source_tc_frame,
            source_tc_string: frames_to_timecode(source_tc_frame, fps),
            audio_channels: self.audio_channels,
            audio_sample_rate: self.audio_sample_rate,
            color_space: fmt.color_space,
            video_codec: None,
            audio_codec: None,
            alpha_mode: AlphaMode::Default,
            data_levels: DataLevels::Auto,
            pixel_aspect_ratio: None,
            metadata: self.metadata.clone(),
            custom_metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct EffectInfo {
    name: String,
    uid: String,
}

#[derive(Debug, Clone)]
enum MediaDef {
    Compound(Sequence),
    Multicam {
        seq: Sequence,
        /// `angleID -> angle name` (e.g. `"angle-1" -> "Camera A"`)
        angle_names: HashMap<String, String>,
    },
}

struct ParseContext {
    formats: HashMap<String, FormatInfo>,
    format_order: Vec<String>,
    primary_color_space: Option<String>,
    assets: HashMap<String, AssetInfo>,
    asset_order: Vec<String>,
    effects: HashMap<String, EffectInfo>,
    media_defs: HashMap<String, MediaDef>,
    next_id: usize,
}

impl ParseContext {
    fn new() -> Self {
        Self {
            formats: HashMap::new(),
            format_order: Vec::new(),
            primary_color_space: None,
            assets: HashMap::new(),
            asset_order: Vec::new(),
            effects: HashMap::new(),
            media_defs: HashMap::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}-{}", self.next_id);
        self.next_id += 1;
        id
    }

    fn parse_resources(&mut self, res_node: Node<'_, '_>) {
        // Pass 1: formats, assets, effects (so <media> compound/multicam can reference them)
        for child in res_node.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "format" => self.parse_format(child),
                "asset" => self.parse_asset(child),
                "effect" => self.parse_effect(child),
                _ => {}
            }
        }

        // Pass 2: <media> compound clips & multicam clips
        for child in res_node.children().filter(|n| n.is_element() && n.has_tag_name("media")) {
            self.parse_media_resource(child);
        }
    }

    fn parse_format(&mut self, node: Node<'_, '_>) {
        let Some(id) = node.attribute("id") else {
            return;
        };
        let width = node
            .attribute("width")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1920);
        let height = node
            .attribute("height")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1080);
        let fps = node
            .attribute("frameDuration")
            .and_then(parse_frame_duration_to_fps)
            .unwrap_or(Rational::FPS_25);
        let color_space = node
            .attribute("colorSpace")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        self.format_order.push(id.to_string());
        self.formats.insert(
            id.to_string(),
            FormatInfo {
                width,
                height,
                fps,
                color_space,
            },
        );
    }

    fn parse_asset(&mut self, node: Node<'_, '_>) {
        let Some(res_id) = node.attribute("id") else {
            return;
        };
        let uid = node
            .attribute("uid")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("fcp-asset-{res_id}"));

        // Locate media URL: prefer <media-rep kind="original-media" src="..."/>, then any <media-rep src="..."/>, then legacy asset@src
        let src_url = node
            .children()
            .find(|n| {
                n.is_element()
                    && n.has_tag_name("media-rep")
                    && n.attribute("kind") == Some("original-media")
                    && n.attribute("src").is_some()
            })
            .and_then(|n| n.attribute("src"))
            .or_else(|| {
                node.children()
                    .find(|n| {
                        n.is_element()
                            && n.has_tag_name("media-rep")
                            && n.attribute("src").is_some()
                    })
                    .and_then(|n| n.attribute("src"))
            })
            .or_else(|| node.attribute("src"))
            .unwrap_or("");

        let path = file_url_to_path(src_url);
        let name = node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                Path::new(&path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| res_id.to_string());

        let has_video = node.attribute("hasVideo").map(|s| s == "1").unwrap_or(false);
        let has_audio = node.attribute("hasAudio").map(|s| s == "1").unwrap_or(false);
        let start = node
            .attribute("start")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);
        let duration = node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);

        let is_image_ext = Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "tif" | "tiff" | "psd" | "bmp" | "webp" | "heic" | "exr" | "dpx"
                )
            })
            .unwrap_or(false);

        let media_kind = if (has_video && duration == FcpTime::ZERO && !has_audio) || is_image_ext {
            MediaKind::Image
        } else if has_video {
            MediaKind::Video
        } else if has_audio {
            MediaKind::Audio
        } else {
            MediaKind::Unknown
        };

        let audio_sources = node
            .attribute("audioSources")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1)
            .max(1);
        let per_source_channels = node
            .attribute("audioChannels")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(if has_audio { 2 } else { 0 });
        let audio_channels = if has_audio {
            (per_source_channels.max(1) * audio_sources).min(32)
        } else {
            0
        };
        let audio_sample_rate = node
            .attribute("audioRate")
            .and_then(parse_audio_rate)
            .unwrap_or(if has_audio { 48_000 } else { 0 });

        let metadata = parse_fcp_metadata(node);

        let info = AssetInfo {
            id: uid,
            name,
            path,
            media_kind,
            has_video: has_video || media_kind == MediaKind::Image,
            has_audio,
            format_id: node.attribute("format").map(|s| s.to_string()),
            start,
            duration,
            audio_channels,
            audio_sample_rate,
            metadata,
        };
        self.asset_order.push(res_id.to_string());
        self.assets.insert(res_id.to_string(), info);
    }

    fn parse_effect(&mut self, node: Node<'_, '_>) {
        let Some(id) = node.attribute("id") else {
            return;
        };
        let name = node.attribute("name").unwrap_or("Effect").to_string();
        let uid = node.attribute("uid").unwrap_or("").to_string();
        self.effects.insert(id.to_string(), EffectInfo { name, uid });
    }

    fn parse_media_resource(&mut self, node: Node<'_, '_>) {
        let Some(res_id) = node.attribute("id") else {
            return;
        };
        let name = node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Compound Clip")
            .to_string();
        let seq_id = node
            .attribute("uid")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        if let Some(seq_node) = node
            .children()
            .find(|n| n.is_element() && n.has_tag_name("sequence"))
        {
            let seq = self.parse_sequence_node(seq_node, &seq_id, &name, SequenceKind::CompoundClip);
            self.media_defs
                .insert(res_id.to_string(), MediaDef::Compound(seq));
        } else if let Some(mc_node) = node
            .children()
            .find(|n| n.is_element() && n.has_tag_name("multicam"))
        {
            let (seq, angle_names) = self.parse_multicam_node(mc_node, &seq_id, &name);
            self.media_defs
                .insert(res_id.to_string(), MediaDef::Multicam { seq, angle_names });
        }
    }

    fn parse_multicam_node(
        &mut self,
        mc_node: Node<'_, '_>,
        seq_id: &str,
        name: &str,
    ) -> (Sequence, HashMap<String, String>) {
        let fmt = mc_node
            .attribute("format")
            .and_then(|fid| self.formats.get(fid))
            .cloned()
            .unwrap_or_default();
        let tc_start = mc_node
            .attribute("tcStart")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);
        let start_tc_frame = tc_start.to_frames(fmt.fps).max(0);

        let mut video_tracks = Vec::new();
        let mut audio_tracks = Vec::new();
        let mut angle_names = HashMap::new();
        let mut markers = Vec::new();

        for (idx, angle_node) in mc_node
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("mc-angle"))
            .enumerate()
        {
            let angle_name = angle_node
                .attribute("name")
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Camera {}", idx + 1));
            if let Some(aid) = angle_node.attribute("angleID") {
                angle_names.insert(aid.to_string(), angle_name.clone());
            }

            let mut tb = TrackBuilder::new(fmt.width, fmt.height, fmt.fps, tc_start);
            self.walk_spine_children(angle_node, tc_start, 0, &mut tb, &mut markers);
            let (mut v_tracks, mut a_tracks) = tb.finish();

            let v_clips = if !v_tracks.is_empty() {
                v_tracks.remove(0).clips
            } else {
                Vec::new()
            };
            video_tracks.push(Track {
                index: idx + 1,
                kind: TrackKind::Video,
                name: angle_name.clone(),
                enabled: true,
                locked: false,
                muted: false,
                solo: false,
                audio_channels: 0,
                output_bus_id: None,
                clips: v_clips,
                transitions: Vec::new(),
                markers: Vec::new(),
            });

            if !a_tracks.is_empty() {
                let mut at = a_tracks.remove(0);
                at.index = audio_tracks.len() + 1;
                at.name = format!("A{}", at.index);
                audio_tracks.push(at);
            }
        }

        let mut seq = Sequence {
            id: seq_id.to_string(),
            name: name.to_string(),
            kind: SequenceKind::MulticamClip,
            width: fmt.width,
            height: fmt.height,
            frame_rate: fmt.fps,
            duration_frames: 0,
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
        (seq, angle_names)
    }

    fn parse_event(
        &mut self,
        event_node: Node<'_, '_>,
        bins: &mut Vec<ProjectBin>,
        sequences: &mut Vec<Sequence>,
    ) {
        let bin_name = event_node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Event")
            .to_string();
        let bin_id = event_node
            .attribute("uid")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.alloc_id("event"));

        let mut media_paths: Vec<String> = Vec::new();
        let mut seen_paths: HashSet<String> = HashSet::new();
        let mut sequence_ids: Vec<String> = Vec::new();

        for child in event_node.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "project" => {
                    if let Some(seq) = self.parse_project_node(child) {
                        sequence_ids.push(seq.id.clone());
                        sequences.push(seq);
                    }
                }
                "asset-clip" | "clip" | "sync-clip" => {
                    // Browser clip inside the event: attach keywords/metadata onto asset and record in bin
                    let ref_id = child.attribute("ref").or_else(|| {
                        child
                            .children()
                            .find(|n| n.is_element() && (n.has_tag_name("video") || n.has_tag_name("audio")))
                            .and_then(|n| n.attribute("ref"))
                    });
                    if let Some(rid) = ref_id {
                        let keywords: Vec<String> = child
                            .children()
                            .filter(|n| n.is_element() && n.has_tag_name("keyword"))
                            .filter_map(|n| n.attribute("value"))
                            .map(|s| s.to_string())
                            .collect();
                        if let Some(asset) = self.assets.get_mut(rid) {
                            if !keywords.is_empty() {
                                let existing = asset.metadata.entry("Keywords".to_string()).or_default();
                                for kw in keywords {
                                    if !existing.split(',').any(|k| k.trim() == kw) {
                                        if !existing.is_empty() {
                                            existing.push_str(", ");
                                        }
                                        existing.push_str(&kw);
                                    }
                                }
                            }
                            if !asset.path.is_empty() && seen_paths.insert(asset.path.clone()) {
                                media_paths.push(asset.path.clone());
                            }
                        }
                    }
                }
                "ref-clip" | "mc-clip" => {
                    if let Some(rid) = child.attribute("ref")
                        && let Some(mdef) = self.media_defs.get(rid) {
                            let sid = match mdef {
                                MediaDef::Compound(s) => s.id.clone(),
                                MediaDef::Multicam { seq, .. } => seq.id.clone(),
                            };
                            if !sequence_ids.contains(&sid) {
                                sequence_ids.push(sid);
                            }
                        }
                }
                _ => {}
            }
        }

        bins.push(ProjectBin {
            id: bin_id,
            name: bin_name,
            parent_id: None,
            media_paths,
            sequence_ids,
            custom_metadata: BTreeMap::new(),
        });
    }

    fn parse_project_node(&mut self, proj_node: Node<'_, '_>) -> Option<Sequence> {
        let seq_node = proj_node
            .children()
            .find(|n| n.is_element() && n.has_tag_name("sequence"))?;
        let name = proj_node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Untitled Timeline")
            .to_string();
        let id = proj_node
            .attribute("uid")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        Some(self.parse_sequence_node(seq_node, &id, &name, SequenceKind::Timeline))
    }

    fn parse_sequence_node(
        &mut self,
        seq_node: Node<'_, '_>,
        id: &str,
        name: &str,
        kind: SequenceKind,
    ) -> Sequence {
        let fmt = seq_node
            .attribute("format")
            .and_then(|fid| self.formats.get(fid))
            .cloned()
            .unwrap_or_default();
        if kind == SequenceKind::Timeline && self.primary_color_space.is_none()
            && let Some(cs) = &fmt.color_space {
                self.primary_color_space = Some(cs.clone());
            }

        let tc_start = seq_node
            .attribute("tcStart")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);
        let start_tc_frame = tc_start.to_frames(fmt.fps).max(0);
        let declared_dur = seq_node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .map(|t| t.to_frames(fmt.fps).max(0))
            .unwrap_or(0);

        let mut tb = TrackBuilder::new(fmt.width, fmt.height, fmt.fps, tc_start);
        let mut markers: Vec<Marker> = Vec::new();

        if let Some(spine) = seq_node
            .children()
            .find(|n| n.is_element() && n.has_tag_name("spine"))
        {
            self.walk_spine_children(spine, tc_start, 0, &mut tb, &mut markers);
        }

        let (video_tracks, audio_tracks) = tb.finish();
        let mut seq = Sequence {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            width: fmt.width,
            height: fmt.height,
            frame_rate: fmt.fps,
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
        seq
    }

    fn walk_spine_children(
        &mut self,
        spine_node: Node<'_, '_>,
        spine_origin: FcpTime,
        spine_lane: i32,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let children: Vec<Node<'_, '_>> = spine_node.children().filter(|n| n.is_element()).collect();
        let mut cursor = spine_origin;
        for (idx, child) in children.iter().copied().enumerate() {
            let tag = child.tag_name().name();
            if tag == "transition" {
                let has_prev = idx > 0
                    && !matches!(
                        children[idx - 1].tag_name().name(),
                        "gap" | "transition"
                    );
                let has_next = idx + 1 < children.len()
                    && !matches!(
                        children[idx + 1].tag_name().name(),
                        "gap" | "transition"
                    );
                let alignment = match (has_prev, has_next) {
                    (false, true) => "start",
                    (true, false) => "end",
                    _ => "center",
                };
                self.parse_transition(child, cursor, spine_lane, alignment, tb, markers);
                continue;
            }
            let elem_offset = child
                .attribute("offset")
                .and_then(FcpTime::parse)
                .unwrap_or(cursor);
            let elem_dur = child
                .attribute("duration")
                .and_then(FcpTime::parse)
                .unwrap_or(FcpTime::ZERO);

            self.parse_timeline_item(child, elem_offset, spine_lane, tb, markers);
            if elem_dur.num > 0 {
                cursor = elem_offset + elem_dur;
            }
        }
    }

    fn parse_timeline_item(
        &mut self,
        node: Node<'_, '_>,
        timeline_offset: FcpTime,
        lane: i32,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let tag = node.tag_name().name();
        match tag {
            "audition" => {
                // Active pick is the first element child of <audition>
                if let Some(active) = node.children().find(|n| n.is_element()) {
                    self.parse_timeline_item(active, timeline_offset, lane, tb, markers);
                }
            }
            "gap" => {
                let local_start = node
                    .attribute("start")
                    .and_then(FcpTime::parse)
                    .unwrap_or(FcpTime::ZERO);
                self.collect_item_markers(node, timeline_offset, local_start, tb, markers);
                self.walk_connected_children(node, timeline_offset, local_start, tb, markers);
            }
            "asset-clip" => {
                self.parse_asset_clip(node, timeline_offset, lane, tb, markers);
            }
            "clip" | "sync-clip" => {
                self.parse_compound_wrapper_clip(node, timeline_offset, lane, tb, markers);
            }
            "video" => {
                self.parse_bare_media_component(node, timeline_offset, lane, true, None, tb, markers);
            }
            "audio" => {
                self.parse_bare_media_component(node, timeline_offset, lane, false, None, tb, markers);
            }
            "ref-clip" => {
                self.parse_ref_clip(node, timeline_offset, lane, tb, markers);
            }
            "mc-clip" => {
                self.parse_mc_clip(node, timeline_offset, lane, tb, markers);
            }
            "title" => {
                self.parse_title_item(node, timeline_offset, tb, markers);
            }
            _ => {}
        }
    }

    fn walk_connected_children(
        &mut self,
        parent: Node<'_, '_>,
        parent_timeline_offset: FcpTime,
        parent_local_start: FcpTime,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        for child in parent.children().filter(|n| n.is_element()) {
            let Some(lane_str) = child.attribute("lane") else {
                continue;
            };
            let lane = lane_str.parse::<i32>().unwrap_or(0);
            let child_offset = child
                .attribute("offset")
                .and_then(FcpTime::parse)
                .unwrap_or(parent_local_start);
            let child_timeline_offset = parent_timeline_offset + (child_offset - parent_local_start);

            if child.has_tag_name("spine") {
                self.walk_spine_children(child, child_timeline_offset, lane, tb, markers);
            } else {
                self.parse_timeline_item(child, child_timeline_offset, lane, tb, markers);
            }
        }
    }

    fn parse_asset_clip(
        &mut self,
        node: Node<'_, '_>,
        timeline_offset: FcpTime,
        lane: i32,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let Some(ref_id) = node.attribute("ref") else {
            return;
        };
        let Some(asset) = self.assets.get(ref_id).cloned() else {
            return;
        };

        let asset_fmt = asset.effective_format(&self.formats);
        let dur_time = node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .unwrap_or(asset.duration);
        let local_start = node
            .attribute("start")
            .and_then(FcpTime::parse)
            .unwrap_or(asset.start);

        let start_frame = (timeline_offset - tb.tc_start).to_frames(tb.seq_fps).max(0);
        let end_frame = (timeline_offset + dur_time - tb.tc_start)
            .to_frames(tb.seq_fps)
            .max(start_frame + 1);

        let clip_name = node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&asset.name)
            .to_string();
        let enabled = node.attribute("enabled").map(|s| s != "0").unwrap_or(true);

        let (speed_percent, reversed, src_in_frame, src_out_frame, speed_keyframes) =
            parse_retiming_and_source_range(node, local_start, dur_time, asset.start, asset_fmt.fps);

        let (motion, opacity, blend_mode, effects) = self.parse_clip_adjustments_and_effects(
            node,
            tb.seq_width,
            tb.seq_height,
            TrackKind::Video,
        );

        // Determine whether video and/or audio should be emitted for this <asset-clip>
        let src_enable = node.attribute("srcEnable").unwrap_or("all");
        let audio_channel_sources: Vec<Node<'_, '_>> = node
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("audio-channel-source"))
            .collect();
        let all_audio_disabled = !audio_channel_sources.is_empty()
            && audio_channel_sources
                .iter()
                .all(|n| n.attribute("active") == Some("0"));

        let emit_video = asset.has_video && src_enable != "audio";
        let emit_audio = asset.has_audio && src_enable != "video" && !all_audio_disabled;

        let link_group_id = if emit_video && emit_audio {
            Some(self.alloc_id("fcp-link"))
        } else {
            None
        };

        // Split audio filters onto the audio clip if both video & audio exist
        let (video_effects, audio_effects) = if emit_video && emit_audio {
            let mut v_fx = Vec::new();
            let mut a_fx = Vec::new();
            for fx in effects {
                if fx.media_kind == TrackKind::Audio {
                    a_fx.push(fx);
                } else {
                    v_fx.push(fx);
                }
            }
            (v_fx, a_fx)
        } else if emit_video {
            (effects, Vec::new())
        } else {
            (Vec::new(), effects)
        };

        if emit_video {
            let v_clip = Clip {
                id: self.alloc_id("clip"),
                name: clip_name.clone(),
                media_id: asset.id.clone(),
                media_path: asset.path.clone(),
                media_kind: asset.media_kind,
                source_kind: ClipSourceKind::Media,
                active_angle: None,
                start_frame,
                end_frame,
                source_in_frame: src_in_frame,
                source_out_frame: src_out_frame,
                source_in_ticks: None,
                source_out_ticks: None,
                source_fps: asset_fmt.fps,
                source_width: asset_fmt.width,
                source_height: asset_fmt.height,
                speed_percent,
                reversed,
                speed_keyframes: speed_keyframes.clone(),
                retime_interpolation: RetimeInterpolation::Default,
                enabled,
                audio_channel: 0,
                link_group_id: link_group_id.clone(),
                label_name: None,
                label_preset_index: None,
                motion,
                opacity,
                blend_mode,
                lumetri_effects: Vec::new(),
                effects: video_effects,
                markers: Vec::new(),
            };
            let pref_v = lane.max(0) as usize;
            tb.place_video_clip(pref_v, v_clip);
        }

        if emit_audio {
            let a_clip = Clip {
                id: self.alloc_id("clip"),
                name: clip_name,
                media_id: asset.id.clone(),
                media_path: asset.path.clone(),
                media_kind: if emit_video {
                    asset.media_kind
                } else {
                    MediaKind::Audio
                },
                source_kind: ClipSourceKind::Media,
                active_angle: None,
                start_frame,
                end_frame,
                source_in_frame: src_in_frame,
                source_out_frame: src_out_frame,
                source_in_ticks: None,
                source_out_ticks: None,
                source_fps: asset_fmt.fps,
                source_width: asset_fmt.width,
                source_height: asset_fmt.height,
                speed_percent,
                reversed,
                speed_keyframes: speed_keyframes.clone(),
                retime_interpolation: RetimeInterpolation::Default,
                enabled,
                audio_channel: 1,
                link_group_id,
                label_name: None,
                label_preset_index: None,
                motion: MotionTransform::default(),
                opacity: 100.0,
                blend_mode: BlendMode::Normal,
                lumetri_effects: Vec::new(),
                effects: audio_effects,
                markers: Vec::new(),
            };
            let pref_a = if lane < 0 {
                ((-lane) - 1) as usize
            } else {
                0
            };
            tb.place_audio_clip(pref_a, a_clip, asset.audio_channels.max(1));
        }

        self.collect_item_markers(node, timeline_offset, local_start, tb, markers);
        self.walk_connected_children(node, timeline_offset, local_start, tb, markers);
    }

    fn parse_compound_wrapper_clip(
        &mut self,
        node: Node<'_, '_>,
        timeline_offset: FcpTime,
        lane: i32,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let local_start = node
            .attribute("start")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);
        let wrapper_dur = node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);
        let wrapper_window = if wrapper_dur.num > 0 {
            Some((timeline_offset, timeline_offset + wrapper_dur))
        } else {
            None
        };
        let wrapper_name = node.attribute("name").map(|s| s.to_string());
        let wrapper_enabled = node.attribute("enabled").map(|s| s != "0").unwrap_or(true);

        let (wrapper_motion, wrapper_opacity, wrapper_blend, wrapper_effects) = self
            .parse_clip_adjustments_and_effects(node, tb.seq_width, tb.seq_height, TrackKind::Video);

        let link_group_id = self.alloc_id("fcp-link");

        // Direct non-lane <video> / <audio> children inside <clip> / <sync-clip>
        let mut has_direct_media = false;
        for child in node.children().filter(|n| n.is_element()) {
            if child.attribute("lane").is_some() {
                continue;
            }
            match child.tag_name().name() {
                "video" => {
                    has_direct_media = true;
                    let child_off = child
                        .attribute("offset")
                        .and_then(FcpTime::parse)
                        .unwrap_or(local_start);
                    let child_tl_off = timeline_offset + (child_off - local_start);
                    self.parse_bare_media_component_with_overrides(
                        child,
                        child_tl_off,
                        lane,
                        true,
                        Some(link_group_id.clone()),
                        wrapper_name.as_deref(),
                        wrapper_window,
                        wrapper_enabled,
                        Some(&wrapper_motion),
                        Some(wrapper_opacity),
                        Some(wrapper_blend),
                        &wrapper_effects,
                        tb,
                        markers,
                    );
                }
                "audio" => {
                    has_direct_media = true;
                    let child_off = child
                        .attribute("offset")
                        .and_then(FcpTime::parse)
                        .unwrap_or(local_start);
                    let child_tl_off = timeline_offset + (child_off - local_start);
                    self.parse_bare_media_component_with_overrides(
                        child,
                        child_tl_off,
                        lane,
                        false,
                        Some(link_group_id.clone()),
                        wrapper_name.as_deref(),
                        wrapper_window,
                        wrapper_enabled,
                        None,
                        None,
                        None,
                        &[],
                        tb,
                        markers,
                    );
                }
                _ => {}
            }
        }

        // Also handle connected audio/video inside <sync-clip> or <clip>
        self.collect_item_markers(node, timeline_offset, local_start, tb, markers);
        if has_direct_media || node.children().any(|c| c.is_element() && c.attribute("lane").is_some()) {
            self.walk_connected_children(node, timeline_offset, local_start, tb, markers);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_bare_media_component(
        &mut self,
        node: Node<'_, '_>,
        timeline_offset: FcpTime,
        lane: i32,
        is_video: bool,
        link_group_id: Option<String>,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        self.parse_bare_media_component_with_overrides(
            node,
            timeline_offset,
            lane,
            is_video,
            link_group_id,
            None,
            None,
            true,
            None,
            None,
            None,
            &[],
            tb,
            markers,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_bare_media_component_with_overrides(
        &mut self,
        node: Node<'_, '_>,
        timeline_offset: FcpTime,
        lane: i32,
        is_video: bool,
        link_group_id: Option<String>,
        fallback_name: Option<&str>,
        clamp_window: Option<(FcpTime, FcpTime)>,
        parent_enabled: bool,
        parent_motion: Option<&MotionTransform>,
        parent_opacity: Option<f64>,
        parent_blend: Option<BlendMode>,
        parent_effects: &[EffectInstance],
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let Some(ref_id) = node.attribute("ref") else {
            return;
        };
        let Some(asset) = self.assets.get(ref_id).cloned() else {
            // Could be a generator <video ref="r_effect"> — walk its connected children/markers anyway
            let local_start = node
                .attribute("start")
                .and_then(FcpTime::parse)
                .unwrap_or(FcpTime::ZERO);
            self.collect_item_markers(node, timeline_offset, local_start, tb, markers);
            self.walk_connected_children(node, timeline_offset, local_start, tb, markers);
            return;
        };

        let asset_fmt = asset.effective_format(&self.formats);
        let raw_dur = node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .unwrap_or(asset.duration);
        let raw_local_start = node
            .attribute("start")
            .and_then(FcpTime::parse)
            .unwrap_or(asset.start);

        // Clamp component to parent <clip> trimming window when nested inside a <clip>
        let (eff_tl_offset, eff_local_start, eff_dur) = if let Some((win_start, win_end)) = clamp_window {
            let raw_end = timeline_offset + raw_dur;
            let clamped_start = if timeline_offset < win_start {
                win_start
            } else {
                timeline_offset
            };
            let clamped_end = if raw_end > win_end { win_end } else { raw_end };
            let trim_in = clamped_start - timeline_offset;
            let dur = clamped_end - clamped_start;
            (clamped_start, raw_local_start + trim_in, dur)
        } else {
            (timeline_offset, raw_local_start, raw_dur)
        };

        let start_frame = (eff_tl_offset - tb.tc_start).to_frames(tb.seq_fps).max(0);
        let end_frame = (eff_tl_offset + eff_dur - tb.tc_start)
            .to_frames(tb.seq_fps)
            .max(start_frame + 1);

        let clip_name = node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .or(fallback_name)
            .unwrap_or(&asset.name)
            .to_string();
        let enabled = parent_enabled && node.attribute("enabled").map(|s| s != "0").unwrap_or(true);

        let (speed_percent, reversed, src_in_frame, src_out_frame, speed_keyframes) =
            parse_retiming_and_source_range(node, eff_local_start, eff_dur, asset.start, asset_fmt.fps);

        let (own_motion, own_opacity, own_blend, mut effects) = self.parse_clip_adjustments_and_effects(
            node,
            tb.seq_width,
            tb.seq_height,
            if is_video { TrackKind::Video } else { TrackKind::Audio },
        );
        let motion = if !own_motion.is_identity() {
            own_motion
        } else {
            parent_motion.cloned().unwrap_or_default()
        };
        let opacity = if (own_opacity - 100.0).abs() > 1e-3 {
            own_opacity
        } else {
            parent_opacity.unwrap_or(100.0)
        };
        let blend_mode = if !own_blend.is_normal() {
            own_blend
        } else {
            parent_blend.unwrap_or(BlendMode::Normal)
        };
        effects.extend(parent_effects.iter().cloned());

        let clip = Clip {
            id: self.alloc_id("clip"),
            name: clip_name,
            media_id: asset.id.clone(),
            media_path: asset.path.clone(),
            media_kind: if is_video {
                asset.media_kind
            } else {
                MediaKind::Audio
            },
            source_kind: ClipSourceKind::Media,
            active_angle: None,
            start_frame,
            end_frame,
            source_in_frame: src_in_frame,
            source_out_frame: src_out_frame,
            source_in_ticks: None,
            source_out_ticks: None,
            source_fps: asset_fmt.fps,
            source_width: asset_fmt.width,
            source_height: asset_fmt.height,
            speed_percent,
            reversed,
            speed_keyframes: speed_keyframes.clone(),
            retime_interpolation: RetimeInterpolation::Default,
            enabled,
            audio_channel: if is_video { 0 } else { 1 },
            link_group_id: link_group_id.clone(),
            label_name: None,
            label_preset_index: None,
            motion,
            opacity,
            blend_mode,
            lumetri_effects: Vec::new(),
            effects,
            markers: Vec::new(),
        };

        if is_video {
            tb.place_video_clip(lane.max(0) as usize, clip);
        } else {
            let pref_a = if lane < 0 { ((-lane) - 1) as usize } else { 0 };
            tb.place_audio_clip(pref_a, clip, asset.audio_channels.max(1));
        }

        // Handle nested <audio> inside <video> (both un-laned and lane="-1" embedded audio in <clip><video><audio .../></video></clip>)
        for child in node.children().filter(|n| n.is_element() && n.has_tag_name("audio")) {
            let child_lane = child
                .attribute("lane")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(-1);
            let child_off = child
                .attribute("offset")
                .and_then(FcpTime::parse)
                .unwrap_or(raw_local_start);
            let child_tl_off = timeline_offset + (child_off - raw_local_start);
            self.parse_bare_media_component_with_overrides(
                child,
                child_tl_off,
                child_lane,
                false,
                link_group_id.clone(),
                fallback_name,
                clamp_window,
                parent_enabled,
                None,
                None,
                None,
                &[],
                tb,
                markers,
            );
        }

        self.collect_item_markers(node, timeline_offset, raw_local_start, tb, markers);
        // Walk any OTHER connected children (excluding <audio> already handled above)
        for child in node.children().filter(|n| n.is_element() && !n.has_tag_name("audio")) {
            let Some(lane_str) = child.attribute("lane") else {
                continue;
            };
            let child_lane = lane_str.parse::<i32>().unwrap_or(0);
            let child_offset = child
                .attribute("offset")
                .and_then(FcpTime::parse)
                .unwrap_or(raw_local_start);
            let child_timeline_offset = timeline_offset + (child_offset - raw_local_start);
            if child.has_tag_name("spine") {
                self.walk_spine_children(child, child_timeline_offset, child_lane, tb, markers);
            } else {
                self.parse_timeline_item(child, child_timeline_offset, child_lane, tb, markers);
            }
        }
    }

    fn parse_ref_clip(
        &mut self,
        node: Node<'_, '_>,
        timeline_offset: FcpTime,
        lane: i32,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let Some(ref_id) = node.attribute("ref") else {
            return;
        };
        let Some(MediaDef::Compound(comp_seq)) = self.media_defs.get(ref_id).cloned() else {
            return;
        };

        let dur_time = node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .unwrap_or_else(|| FcpTime::from_frames(comp_seq.duration_frames, comp_seq.frame_rate));
        let comp_start = FcpTime::from_frames(comp_seq.start_tc_frame, comp_seq.frame_rate);
        let local_start = node
            .attribute("start")
            .and_then(FcpTime::parse)
            .unwrap_or(comp_start);

        let start_frame = (timeline_offset - tb.tc_start).to_frames(tb.seq_fps).max(0);
        let end_frame = (timeline_offset + dur_time - tb.tc_start)
            .to_frames(tb.seq_fps)
            .max(start_frame + 1);
        let clip_name = node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&comp_seq.name)
            .to_string();
        let enabled = node.attribute("enabled").map(|s| s != "0").unwrap_or(true);

        let (speed_percent, reversed, src_in_frame, src_out_frame, speed_keyframes) =
            parse_retiming_and_source_range(node, local_start, dur_time, comp_start, comp_seq.frame_rate);
        let (motion, opacity, blend_mode, effects) = self.parse_clip_adjustments_and_effects(
            node,
            tb.seq_width,
            tb.seq_height,
            TrackKind::Video,
        );

        let has_audio = !comp_seq.audio_tracks.is_empty();
        let link_group_id = if has_audio {
            Some(self.alloc_id("fcp-link"))
        } else {
            None
        };

        let v_clip = Clip {
            id: self.alloc_id("clip"),
            name: clip_name.clone(),
            media_id: comp_seq.id.clone(),
            media_path: String::new(),
            media_kind: MediaKind::Video,
            source_kind: ClipSourceKind::CompoundClip,
            active_angle: None,
            start_frame,
            end_frame,
            source_in_frame: src_in_frame,
            source_out_frame: src_out_frame,
            source_in_ticks: None,
            source_out_ticks: None,
            source_fps: comp_seq.frame_rate,
            source_width: comp_seq.width,
            source_height: comp_seq.height,
            speed_percent,
            reversed,
            speed_keyframes: speed_keyframes.clone(),
            retime_interpolation: RetimeInterpolation::Default,
            enabled,
            audio_channel: 0,
            link_group_id: link_group_id.clone(),
            label_name: None,
            label_preset_index: None,
            motion,
            opacity,
            blend_mode,
            lumetri_effects: Vec::new(),
            effects,
            markers: Vec::new(),
        };
        tb.place_video_clip(lane.max(0) as usize, v_clip);

        if has_audio {
            let a_clip = Clip {
                id: self.alloc_id("clip"),
                name: clip_name,
                media_id: comp_seq.id.clone(),
                media_path: String::new(),
                media_kind: MediaKind::Audio,
                source_kind: ClipSourceKind::CompoundClip,
                active_angle: None,
                start_frame,
                end_frame,
                source_in_frame: src_in_frame,
                source_out_frame: src_out_frame,
                source_in_ticks: None,
                source_out_ticks: None,
                source_fps: comp_seq.frame_rate,
                source_width: comp_seq.width,
                source_height: comp_seq.height,
                speed_percent,
                reversed,
                speed_keyframes: speed_keyframes.clone(),
                retime_interpolation: RetimeInterpolation::Default,
                enabled,
                audio_channel: 1,
                link_group_id,
                label_name: None,
                label_preset_index: None,
                motion: MotionTransform::default(),
                opacity: 100.0,
                blend_mode: BlendMode::Normal,
                lumetri_effects: Vec::new(),
                effects: Vec::new(),
                markers: Vec::new(),
            };
            tb.place_audio_clip(0, a_clip, 2);
        }

        self.collect_item_markers(node, timeline_offset, local_start, tb, markers);
        self.walk_connected_children(node, timeline_offset, local_start, tb, markers);
    }

    fn parse_mc_clip(
        &mut self,
        node: Node<'_, '_>,
        timeline_offset: FcpTime,
        lane: i32,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let Some(ref_id) = node.attribute("ref") else {
            return;
        };
        let Some(MediaDef::Multicam { seq: mc_seq, angle_names }) =
            self.media_defs.get(ref_id).cloned()
        else {
            return;
        };

        let dur_time = node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .unwrap_or_else(|| FcpTime::from_frames(mc_seq.duration_frames, mc_seq.frame_rate));
        let mc_start = FcpTime::from_frames(mc_seq.start_tc_frame, mc_seq.frame_rate);
        let local_start = node
            .attribute("start")
            .and_then(FcpTime::parse)
            .unwrap_or(mc_start);

        let start_frame = (timeline_offset - tb.tc_start).to_frames(tb.seq_fps).max(0);
        let end_frame = (timeline_offset + dur_time - tb.tc_start)
            .to_frames(tb.seq_fps)
            .max(start_frame + 1);
        let clip_name = node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&mc_seq.name)
            .to_string();
        let enabled = node.attribute("enabled").map(|s| s != "0").unwrap_or(true);

        // Determine active video & audio angles from <mc-source angleID="..." srcEnable="..."/>
        let mut video_angle: Option<String> = None;
        let mut audio_angle: Option<String> = None;
        let mut has_audio_source = false;
        for src in node.children().filter(|n| n.is_element() && n.has_tag_name("mc-source")) {
            let aid = src.attribute("angleID").unwrap_or("");
            let resolved_angle = angle_names
                .get(aid)
                .cloned()
                .or_else(|| if !aid.is_empty() { Some(aid.to_string()) } else { None });
            match src.attribute("srcEnable").unwrap_or("all") {
                "video" => video_angle = resolved_angle,
                "audio" => {
                    has_audio_source = true;
                    audio_angle = resolved_angle;
                }
                _ => {
                    video_angle = resolved_angle.clone();
                    audio_angle = resolved_angle;
                    has_audio_source = true;
                }
            }
        }
        if video_angle.is_none() {
            video_angle = mc_seq.video_tracks.first().map(|t| t.name.clone());
        }

        let (speed_percent, reversed, src_in_frame, src_out_frame, speed_keyframes) =
            parse_retiming_and_source_range(node, local_start, dur_time, mc_start, mc_seq.frame_rate);
        let (motion, opacity, blend_mode, effects) = self.parse_clip_adjustments_and_effects(
            node,
            tb.seq_width,
            tb.seq_height,
            TrackKind::Video,
        );

        let emit_audio = has_audio_source && !mc_seq.audio_tracks.is_empty();
        let link_group_id = if emit_audio {
            Some(self.alloc_id("fcp-link"))
        } else {
            None
        };

        let v_clip = Clip {
            id: self.alloc_id("clip"),
            name: clip_name.clone(),
            media_id: mc_seq.id.clone(),
            media_path: String::new(),
            media_kind: MediaKind::Video,
            source_kind: ClipSourceKind::MulticamClip,
            active_angle: video_angle,
            start_frame,
            end_frame,
            source_in_frame: src_in_frame,
            source_out_frame: src_out_frame,
            source_in_ticks: None,
            source_out_ticks: None,
            source_fps: mc_seq.frame_rate,
            source_width: mc_seq.width,
            source_height: mc_seq.height,
            speed_percent,
            reversed,
            speed_keyframes: speed_keyframes.clone(),
            retime_interpolation: RetimeInterpolation::Default,
            enabled,
            audio_channel: 0,
            link_group_id: link_group_id.clone(),
            label_name: None,
            label_preset_index: None,
            motion,
            opacity,
            blend_mode,
            lumetri_effects: Vec::new(),
            effects,
            markers: Vec::new(),
        };
        tb.place_video_clip(lane.max(0) as usize, v_clip);

        if emit_audio {
            let a_clip = Clip {
                id: self.alloc_id("clip"),
                name: clip_name,
                media_id: mc_seq.id.clone(),
                media_path: String::new(),
                media_kind: MediaKind::Audio,
                source_kind: ClipSourceKind::MulticamClip,
                active_angle: audio_angle,
                start_frame,
                end_frame,
                source_in_frame: src_in_frame,
                source_out_frame: src_out_frame,
                source_in_ticks: None,
                source_out_ticks: None,
                source_fps: mc_seq.frame_rate,
                source_width: mc_seq.width,
                source_height: mc_seq.height,
                speed_percent,
                reversed,
                speed_keyframes: speed_keyframes.clone(),
                retime_interpolation: RetimeInterpolation::Default,
                enabled,
                audio_channel: 1,
                link_group_id,
                label_name: None,
                label_preset_index: None,
                motion: MotionTransform::default(),
                opacity: 100.0,
                blend_mode: BlendMode::Normal,
                lumetri_effects: Vec::new(),
                effects: Vec::new(),
                markers: Vec::new(),
            };
            tb.place_audio_clip(0, a_clip, 2);
        }

        self.collect_item_markers(node, timeline_offset, local_start, tb, markers);
        self.walk_connected_children(node, timeline_offset, local_start, tb, markers);
    }

    fn parse_title_item(
        &mut self,
        node: Node<'_, '_>,
        timeline_offset: FcpTime,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let local_start = node
            .attribute("start")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);
        let dur_time = node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);
        let start_frame = (timeline_offset - tb.tc_start).to_frames(tb.seq_fps).max(0);
        let dur_frames = dur_time.to_frames(tb.seq_fps).max(1);

        let title_name = node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                node.attribute("ref")
                    .and_then(|rid| self.effects.get(rid))
                    .map(|e| e.name.as_str())
            })
            .unwrap_or("Title");

        // Extract visible text inside <text><text-style>...</text-style></text>
        let text_body: String = node
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("text-style") && n.parent().is_some_and(|p| p.has_tag_name("text")))
            .filter_map(|n| n.text())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        let comment = if text_body.is_empty() {
            format!("Final Cut Pro Title: {title_name}")
        } else {
            format!("Final Cut Pro Title ({title_name}): \"{text_body}\"")
        };

        markers.push(Marker {
            frame: start_frame,
            duration_frames: dur_frames,
            name: format!("Title: {title_name}"),
            comment,
            color: "Cyan".to_string(),
            scope: MarkerScope::Sequence,
        });

        self.collect_item_markers(node, timeline_offset, local_start, tb, markers);
        self.walk_connected_children(node, timeline_offset, local_start, tb, markers);
    }

    fn parse_transition(
        &mut self,
        node: Node<'_, '_>,
        fallback_offset: FcpTime,
        lane: i32,
        alignment: &str,
        tb: &mut TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        let offset = node
            .attribute("offset")
            .and_then(FcpTime::parse)
            .unwrap_or(fallback_offset);
        let dur = node
            .attribute("duration")
            .and_then(FcpTime::parse)
            .unwrap_or(FcpTime::ZERO);
        let start_frame = (offset - tb.tc_start).to_frames(tb.seq_fps).max(0);
        let end_frame = (offset + dur - tb.tc_start)
            .to_frames(tb.seq_fps)
            .max(start_frame + 1);

        let tr_name = node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Cross Dissolve")
            .to_string();

        let video_filter = node
            .children()
            .find(|n| n.is_element() && n.has_tag_name("filter-video"));
        let audio_filter = node
            .children()
            .find(|n| n.is_element() && n.has_tag_name("filter-audio"));

        if let Some(vf) = video_filter {
            let (fx_name, fx_id) = self.resolve_effect_ref(vf, &tr_name);
            let track_idx = lane.max(0) as usize;
            tb.add_video_transition(
                track_idx,
                Transition {
                    name: fx_name,
                    effect_id: fx_id,
                    start_frame,
                    end_frame,
                    alignment: alignment.to_string(),
                },
            );
        } else if audio_filter.is_none() {
            // Bare <transition name="Cross Dissolve" .../> without explicit filter child
            if lane >= 0 {
                tb.add_video_transition(
                    lane as usize,
                    Transition {
                        name: tr_name.clone(),
                        effect_id: tr_name.clone(),
                        start_frame,
                        end_frame,
                        alignment: alignment.to_string(),
                    },
                );
            }
        }

        if let Some(af) = audio_filter {
            let (fx_name, fx_id) = self.resolve_effect_ref(af, "Constant Power");
            let track_idx = if lane < 0 { ((-lane) - 1) as usize } else { 0 };
            tb.add_audio_transition(
                track_idx,
                Transition {
                    name: fx_name,
                    effect_id: fx_id,
                    start_frame,
                    end_frame,
                    alignment: alignment.to_string(),
                },
            );
        }

        self.collect_item_markers(node, offset, FcpTime::ZERO, tb, markers);
    }

    fn resolve_effect_ref(&self, filter_node: Node<'_, '_>, fallback_name: &str) -> (String, String) {
        let res_info = filter_node
            .attribute("ref")
            .and_then(|rid| self.effects.get(rid));
        let name = filter_node
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .or_else(|| res_info.map(|e| e.name.clone()))
            .unwrap_or_else(|| fallback_name.to_string());
        let effect_id = res_info
            .map(|e| {
                if e.uid.is_empty() {
                    e.name.clone()
                } else {
                    format!("{} ({})", e.name, e.uid)
                }
            })
            .unwrap_or_else(|| name.clone());
        (name, effect_id)
    }

    fn parse_clip_adjustments_and_effects(
        &mut self,
        node: Node<'_, '_>,
        seq_width: u32,
        seq_height: u32,
        default_kind: TrackKind,
    ) -> (MotionTransform, f64, BlendMode, Vec<EffectInstance>) {
        let mut motion = MotionTransform::default();
        let mut opacity = 100.0_f64;
        let mut blend_mode = BlendMode::Normal;
        let mut effects = Vec::new();
        let aspect = if seq_height > 0 {
            seq_width as f64 / seq_height as f64
        } else {
            16.0 / 9.0
        };

        for child in node.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "adjust-transform" => {
                    if let Some(pos) = child.attribute("position") {
                        let vals = parse_f64_pair(pos);
                        if let Some((fx, fy)) = vals {
                            motion.position_x = (fx / 100.0) / aspect;
                            motion.position_y = fy / 100.0;
                        }
                    }
                    if let Some(sc) = child.attribute("scale")
                        && let Some((sx, sy)) = parse_f64_pair(sc) {
                            motion.scale = sx * 100.0;
                            if (sx - sy).abs() > 1e-4 {
                                motion.scale_y = Some(sy * 100.0);
                            }
                        }
                    if let Some(rot) = child.attribute("rotation").and_then(|s| s.parse::<f64>().ok())
                    {
                        // FCPXML rotation is CCW-positive; IR rotation is CW-positive (Premiere convention)
                        motion.rotation = -rot;
                    }
                    if let Some(anc) = child.attribute("anchor")
                        && let Some((ax, ay)) = parse_f64_pair(anc) {
                            motion.anchor_x = (ax / 100.0) / aspect;
                            motion.anchor_y = -(ay / 100.0);
                        }
                    if has_keyframe_animation(child) {
                        motion.has_keyframes = true;
                    }
                }
                "adjust-crop" => {
                    if let Some(trim) = child
                        .children()
                        .find(|n| n.is_element() && n.has_tag_name("trim-rect"))
                    {
                        if let Some(v) = trim.attribute("left").and_then(|s| s.parse::<f64>().ok()) {
                            motion.crop_left = (v / aspect).clamp(0.0, 100.0);
                        }
                        if let Some(v) = trim.attribute("right").and_then(|s| s.parse::<f64>().ok()) {
                            motion.crop_right = (v / aspect).clamp(0.0, 100.0);
                        }
                        if let Some(v) = trim.attribute("top").and_then(|s| s.parse::<f64>().ok()) {
                            motion.crop_top = v.clamp(0.0, 100.0);
                        }
                        if let Some(v) = trim.attribute("bottom").and_then(|s| s.parse::<f64>().ok())
                        {
                            motion.crop_bottom = v.clamp(0.0, 100.0);
                        }
                    }
                    if has_keyframe_animation(child) {
                        motion.has_keyframes = true;
                    }
                }
                "adjust-blend" => {
                    if let Some(amt) = child
                        .attribute("amount")
                        .and_then(|s| s.parse::<f64>().ok())
                    {
                        opacity = (amt * 100.0).clamp(0.0, 100.0);
                    }
                    if let Some(mode) = child.attribute("mode").filter(|m| {
                        let low = m.trim().to_ascii_lowercase();
                        !low.is_empty() && low != "normal" && !low.ends_with("(normal)")
                    }) {
                        if let Some(bm) = BlendMode::parse_name(mode) {
                            blend_mode = bm;
                        } else {
                            effects.push(EffectInstance {
                                id: self.alloc_id("fx"),
                                match_name: "fcp.compositing".to_string(),
                                display_name: format!("Blend Mode ({mode})"),
                                category: EffectCategory::Transform,
                                media_kind: TrackKind::Video,
                                enabled: true,
                                is_intrinsic: false,
                                is_animated: has_keyframe_animation(child),
                                parameters: vec![EffectParameter {
                                    id: "mode".to_string(),
                                    name: "Blend Mode".to_string(),
                                    value: mode.to_string(),
                                    is_animated: false,
                                }],
                            });
                        }
                    }
                }
                "adjust-stabilization" => {
                    let stab_type = child.attribute("type").unwrap_or("automatic");
                    let mut params = vec![EffectParameter {
                        id: "type".to_string(),
                        name: "Method".to_string(),
                        value: stab_type.to_string(),
                        is_animated: false,
                    }];
                    for p in child.children().filter(|n| n.is_element() && n.has_tag_name("param")) {
                        if let Some(pname) = p.attribute("name") {
                            params.push(EffectParameter {
                                id: pname.to_string(),
                                name: pname.to_string(),
                                value: p.attribute("value").unwrap_or("").to_string(),
                                is_animated: has_keyframe_animation(p),
                            });
                        }
                    }
                    effects.push(EffectInstance {
                        id: self.alloc_id("fx"),
                        match_name: "fcp.stabilization".to_string(),
                        display_name: "Stabilization".to_string(),
                        category: EffectCategory::Stabilization,
                        media_kind: TrackKind::Video,
                        enabled: true,
                        is_intrinsic: false,
                        is_animated: false,
                        parameters: params,
                    });
                }
                "adjust-voiceIsolation" => {
                    let amt = child.attribute("amount").unwrap_or("50");
                    effects.push(EffectInstance {
                        id: self.alloc_id("fx"),
                        match_name: "fcp.voice_isolation".to_string(),
                        display_name: "Voice Isolation".to_string(),
                        category: EffectCategory::Audio,
                        media_kind: TrackKind::Audio,
                        enabled: child.attribute("enabled").map(|s| s != "0").unwrap_or(true),
                        is_intrinsic: false,
                        is_animated: false,
                        parameters: vec![EffectParameter {
                            id: "amount".to_string(),
                            name: "Amount".to_string(),
                            value: amt.to_string(),
                            is_animated: false,
                        }],
                    });
                }
                "filter-video-mask" => {
                    let mask_names: Vec<String> = child
                        .children()
                        .filter(|n| n.is_element() && n.tag_name().name().starts_with("mask-"))
                        .map(|n| {
                            n.attribute("name")
                                .unwrap_or(n.tag_name().name())
                                .to_string()
                        })
                        .collect();
                    for inner in child
                        .children()
                        .filter(|n| n.is_element() && n.has_tag_name("filter-video"))
                    {
                        let mut fx = self.parse_filter_node(inner, TrackKind::Video);
                        if !mask_names.is_empty() {
                            fx.parameters.push(EffectParameter {
                                id: "mask".to_string(),
                                name: "Mask".to_string(),
                                value: mask_names.join(", "),
                                is_animated: false,
                            });
                        }
                        effects.push(fx);
                    }
                }
                "filter-video" | "filter-audio" => {
                    let kind = if child.has_tag_name("filter-audio") {
                        TrackKind::Audio
                    } else {
                        default_kind
                    };
                    effects.push(self.parse_filter_node(child, kind));
                }
                _ => {}
            }
        }

        (motion, opacity, blend_mode, effects)
    }

    fn parse_filter_node(&mut self, child: Node<'_, '_>, kind: TrackKind) -> EffectInstance {
        let res_info = child
            .attribute("ref")
            .and_then(|rid| self.effects.get(rid));
        let display_name = child
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .or_else(|| res_info.map(|e| e.name.clone()))
            .unwrap_or_else(|| "Effect".to_string());
        let match_name = res_info
            .filter(|e| !e.uid.is_empty())
            .map(|e| format!("fcp.{}:{}", display_name, e.uid))
            .unwrap_or_else(|| format!("fcp.{display_name}"));
        let enabled = child.attribute("enabled").map(|s| s != "0").unwrap_or(true);
        let mut parameters = Vec::new();
        let mut is_animated = false;
        for p in child
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("param"))
        {
            let anim = has_keyframe_animation(p);
            is_animated |= anim;
            if let Some(pname) = p.attribute("name") {
                parameters.push(EffectParameter {
                    id: p.attribute("key").unwrap_or(pname).to_string(),
                    name: pname.to_string(),
                    value: p.attribute("value").unwrap_or("").to_string(),
                    is_animated: anim,
                });
            }
        }
        let category = infer_fcp_effect_category(&display_name, &match_name, kind);
        EffectInstance {
            id: self.alloc_id("fx"),
            match_name,
            display_name,
            category,
            media_kind: kind,
            enabled,
            is_intrinsic: false,
            is_animated,
            parameters,
        }
    }

    fn collect_item_markers(
        &self,
        node: Node<'_, '_>,
        item_timeline_offset: FcpTime,
        item_local_start: FcpTime,
        tb: &TrackBuilder,
        markers: &mut Vec<Marker>,
    ) {
        for child in node.children().filter(|n| n.is_element()) {
            let tag = child.tag_name().name();
            if !matches!(tag, "marker" | "chapter-marker") {
                continue;
            }
            let m_start = child
                .attribute("start")
                .and_then(FcpTime::parse)
                .unwrap_or(item_local_start);
            let m_dur = child
                .attribute("duration")
                .and_then(FcpTime::parse)
                .unwrap_or(FcpTime::ZERO);

            let timeline_pos = item_timeline_offset + (m_start - item_local_start);
            let frame = (timeline_pos - tb.tc_start).to_frames(tb.seq_fps).max(0);
            let duration_frames = m_dur.to_frames(tb.seq_fps).max(1);

            let name = child
                .attribute("value")
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(if tag == "chapter-marker" {
                    "Chapter"
                } else {
                    "Marker"
                })
                .to_string();
            let comment = child.attribute("note").unwrap_or("").to_string();
            let color = if tag == "chapter-marker" {
                "Orange"
            } else {
                match child.attribute("completed") {
                    Some("0") => "Red",
                    Some("1") => "Green",
                    _ => "Blue",
                }
            }
            .to_string();

            markers.push(Marker {
                frame,
                duration_frames,
                name,
                comment,
                color,
                scope: MarkerScope::Sequence,
            });
        }
    }
}

// ── Track Placement Builder ─────────────────────────────────────────────

struct TrackBuilder {
    seq_width: u32,
    seq_height: u32,
    seq_fps: Rational,
    tc_start: FcpTime,
    video_tracks: Vec<Track>,
    audio_tracks: Vec<Track>,
}

impl TrackBuilder {
    fn new(seq_width: u32, seq_height: u32, seq_fps: Rational, tc_start: FcpTime) -> Self {
        Self {
            seq_width,
            seq_height,
            seq_fps,
            tc_start,
            video_tracks: Vec::new(),
            audio_tracks: Vec::new(),
        }
    }

    fn ensure_video_track(&mut self, idx: usize) {
        while self.video_tracks.len() <= idx {
            let n = self.video_tracks.len() + 1;
            self.video_tracks.push(Track {
                index: n,
                kind: TrackKind::Video,
                name: format!("V{n}"),
                enabled: true,
                locked: false,
                muted: false,
                solo: false,
                audio_channels: 0,
                output_bus_id: None,
                clips: Vec::new(),
                transitions: Vec::new(),
                markers: Vec::new(),
            });
        }
    }

    fn ensure_audio_track(&mut self, idx: usize, channels: u32) {
        while self.audio_tracks.len() <= idx {
            let n = self.audio_tracks.len() + 1;
            self.audio_tracks.push(Track {
                index: n,
                kind: TrackKind::Audio,
                name: format!("A{n}"),
                enabled: true,
                locked: false,
                muted: false,
                solo: false,
                audio_channels: channels.max(1),
                output_bus_id: None,
                clips: Vec::new(),
                transitions: Vec::new(),
                markers: Vec::new(),
            });
        }
    }

    fn place_video_clip(&mut self, preferred_idx: usize, clip: Clip) {
        let mut idx = preferred_idx;
        loop {
            self.ensure_video_track(idx);
            let overlaps = self.video_tracks[idx]
                .clips
                .iter()
                .any(|c| c.start_frame < clip.end_frame && clip.start_frame < c.end_frame);
            if !overlaps {
                self.video_tracks[idx].clips.push(clip);
                break;
            }
            idx += 1;
        }
    }

    fn place_audio_clip(&mut self, preferred_idx: usize, clip: Clip, channels: u32) {
        let mut idx = preferred_idx;
        loop {
            self.ensure_audio_track(idx, channels);
            let overlaps = self.audio_tracks[idx]
                .clips
                .iter()
                .any(|c| c.start_frame < clip.end_frame && clip.start_frame < c.end_frame);
            if !overlaps {
                if self.audio_tracks[idx].clips.is_empty() {
                    self.audio_tracks[idx].audio_channels = channels.max(1);
                }
                self.audio_tracks[idx].clips.push(clip);
                break;
            }
            idx += 1;
        }
    }

    fn add_video_transition(&mut self, track_idx: usize, tr: Transition) {
        self.ensure_video_track(track_idx);
        self.video_tracks[track_idx].transitions.push(tr);
    }

    fn add_audio_transition(&mut self, track_idx: usize, tr: Transition) {
        self.ensure_audio_track(track_idx, 2);
        self.audio_tracks[track_idx].transitions.push(tr);
    }

    fn finish(mut self) -> (Vec<Track>, Vec<Track>) {
        if self.video_tracks.is_empty() {
            self.ensure_video_track(0);
        }
        for (i, t) in self.video_tracks.iter_mut().enumerate() {
            t.index = i + 1;
            t.name = format!("V{}", i + 1);
            t.clips.sort_by_key(|c| c.start_frame);
            t.transitions.sort_by_key(|tr| tr.start_frame);
        }
        for (i, t) in self.audio_tracks.iter_mut().enumerate() {
            t.index = i + 1;
            t.name = format!("A{}", i + 1);
            t.clips.sort_by_key(|c| c.start_frame);
            t.transitions.sort_by_key(|tr| tr.start_frame);
        }
        (self.video_tracks, self.audio_tracks)
    }
}

// ── Helper Parsers ──────────────────────────────────────────────────────

fn parse_retiming_and_source_range(
    node: Node<'_, '_>,
    local_start: FcpTime,
    dur_time: FcpTime,
    asset_start: FcpTime,
    src_fps: Rational,
) -> (f64, bool, i64, i64, Vec<SpeedKeyframe>) {
    let default_in = (local_start - asset_start).to_frames(src_fps).max(0);
    let default_out = ((local_start + dur_time) - asset_start)
        .to_frames(src_fps)
        .max(default_in + 1);

    let Some(tmap) = node
        .children()
        .find(|n| n.is_element() && n.has_tag_name("timeMap"))
    else {
        return (100.0, false, default_in, default_out, Vec::new());
    };

    let pts: Vec<(FcpTime, FcpTime)> = tmap
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("timept"))
        .filter_map(|p| {
            let t = p.attribute("time").and_then(FcpTime::parse)?;
            let v = p.attribute("value").and_then(FcpTime::parse)?;
            Some((t, v))
        })
        .collect();

    if pts.len() < 2 {
        return (100.0, false, default_in, default_out, Vec::new());
    }

    let speed_keyframes = if pts.len() > 2 {
        let t0 = pts[0].0;
        pts.iter()
            .map(|&(t, v)| SpeedKeyframe {
                timeline_sec: (t - t0).as_f64().max(0.0),
                source_sec: (v - asset_start).as_f64().max(0.0),
            })
            .collect()
    } else {
        Vec::new()
    };

    let (t0, v0) = pts[0];
    let (t1, v1) = *pts.last().unwrap();
    let dt = (t1 - t0).as_f64();
    let dv = (v1 - v0).as_f64();

    if dt.abs() < 1e-9 {
        return (100.0, false, default_in, default_out, Vec::new());
    }

    if dv.abs() < 1e-6 && speed_keyframes.is_empty() {
        // Freeze frame
        let hold_frame = (v0 - asset_start).to_frames(src_fps).max(0);
        return (0.0, false, hold_frame, hold_frame + 1, Vec::new());
    }

    let reversed = dv < 0.0;
    let speed = ((dv.abs() / dt.abs()) * 100.0).clamp(0.1, 10_000.0);
    let v_min = if v0 <= v1 { v0 } else { v1 };
    let v_max = if v0 <= v1 { v1 } else { v0 };
    let src_in = (v_min - asset_start).to_frames(src_fps).max(0);
    let src_out = (v_max - asset_start).to_frames(src_fps).max(src_in + 1);

    (speed, reversed, src_in, src_out, speed_keyframes)
}

fn parse_audio_rate(raw: &str) -> Option<u32> {
    let s = raw.trim().to_ascii_lowercase();
    if let Some(k) = s.strip_suffix('k') {
        let v = k.parse::<f64>().ok()?;
        Some((v * 1000.0).round() as u32)
    } else {
        s.parse::<u32>().ok()
    }
}

fn parse_f64_pair(raw: &str) -> Option<(f64, f64)> {
    let mut parts = raw.split_whitespace();
    let a = parts.next()?.parse::<f64>().ok()?;
    let b = parts.next()?.parse::<f64>().ok()?;
    Some((a, b))
}

fn has_keyframe_animation(node: Node<'_, '_>) -> bool {
    node.descendants()
        .any(|n| n.is_element() && n.has_tag_name("keyframeAnimation"))
}

fn parse_fcp_metadata(node: Node<'_, '_>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(md_container) = node
        .children()
        .find(|n| n.is_element() && n.has_tag_name("metadata"))
    else {
        return out;
    };
    for md in md_container
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("md"))
    {
        let Some(key) = md.attribute("key") else {
            continue;
        };
        let Some(val) = md.attribute("value").filter(|v| !v.trim().is_empty()) else {
            continue;
        };
        let canonical = match key {
            "com.apple.proapps.studio.reel" => Some("Tape Name"),
            "com.apple.proapps.studio.scene" => Some("Scene"),
            "com.apple.proapps.studio.shot" => Some("Shot"),
            "com.apple.proapps.studio.take" => Some("Take"),
            "com.apple.proapps.mio.cameraName" => Some("Camera #"),
            "com.apple.proapps.mio.cameraAngle" => Some("Angle"),
            "com.apple.proapps.spotlight.kMDItemFinderComment" => Some("Comments"),
            "com.apple.proapps.share.description" | "com.apple.proapps.studio.notes" => {
                Some("Description")
            }
            _ => None,
        };
        if let Some(c) = canonical {
            out.insert(c.to_string(), val.to_string());
        }
    }
    out
}

fn infer_fcp_effect_category(name: &str, match_name: &str, kind: TrackKind) -> EffectCategory {
    if kind == TrackKind::Audio {
        return EffectCategory::Audio;
    }
    let lower = format!("{name} {match_name}").to_ascii_lowercase();
    if lower.contains("color") || lower.contains("lut") || lower.contains("hue") || lower.contains("curves") {
        EffectCategory::Color
    } else if lower.contains("blur") || lower.contains("defocus") {
        EffectCategory::Blur
    } else if lower.contains("keyer") || lower.contains("key") || lower.contains("matte") {
        EffectCategory::Keying
    } else if lower.contains("stabiliz") {
        EffectCategory::Stabilization
    } else if lower.contains("sharpen") || lower.contains("vignette") || lower.contains("glow") {
        EffectCategory::Stylize
    } else if lower.contains("distort") || lower.contains("corner") || lower.contains("fisheye") {
        EffectCategory::Distort
    } else if lower.contains("title") || lower.contains("text") {
        EffectCategory::Title
    } else {
        EffectCategory::Plugin
    }
}
