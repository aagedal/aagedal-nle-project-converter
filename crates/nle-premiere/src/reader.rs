use crate::lumetri::{child_text, find_descendant, nested_text, parse_keyframe_scalar, parse_lumetri_component};
use flate2::read::GzDecoder;
use nle_core::{
    frames_to_timecode, Clip, ClipSourceKind, ColorConfig, EffectCategory, EffectInstance,
    EffectParameter, LumetriData, Marker, MarkerScope, MediaAsset, MediaKind, MotionTransform,
    NleError, NleFormat, NleResult, Project, ProjectBin, ProjectReader, Rational, Sequence,
    SequenceKind, Track, TrackKind, Transition,
};
use roxmltree::{Document, Node};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub struct PremiereReader;

impl ProjectReader for PremiereReader {
    fn format(&self) -> NleFormat {
        NleFormat::PremierePro
    }

    fn read_file(&self, path: &Path) -> NleResult<Project> {
        read_prproj_file(path)
    }
}

/// Read an Adobe Premiere Pro `.prproj` file (gzip-compressed or plain XML) directly into the canonical `Project` IR.
pub fn read_prproj_file(path: &Path) -> NleResult<Project> {
    let mut file = File::open(path)?;
    let mut raw = Vec::new();
    file.read_to_end(&mut raw)?;

    let xml_bytes = if raw.len() >= 2 && raw[0] == 0x1F && raw[1] == 0x8B {
        let mut decoder = GzDecoder::new(&raw[..]);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| NleError::Archive(format!("Failed to gunzip .prproj: {e}")))?;
        out
    } else {
        raw
    };

    let xml_text = String::from_utf8(xml_bytes)
        .map_err(|e| NleError::InvalidFormat(format!(".prproj is not valid UTF-8 XML: {e}")))?;

    let project_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Premiere Project")
        .to_string();

    let mut project = parse_prproj_xml(&xml_text, &project_name)?;
    project.source_path = Some(path.to_string_lossy().into_owned());
    Ok(project)
}

/// Parse decompressed `<PremiereData>` XML text directly into a `Project`.
pub fn parse_prproj_xml(xml_text: &str, project_name: &str) -> NleResult<Project> {
    let doc = Document::parse(xml_text).map_err(|e| NleError::Xml(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "PremiereData" {
        return Err(NleError::InvalidFormat(format!(
            "Expected <PremiereData> root element, found <{}>",
            root.tag_name().name()
        )));
    }

    // Build fast ObjectID and ObjectUID lookup tables.
    // Top-level children of <PremiereData> MUST be indexed first because nested UI nodes
    // (such as <ColumnList>, <StringColumn>, <ProjectViewState>) reuse low ObjectIDs like
    // "1", "2", "3", ... and would otherwise shadow top-level <ProjectSettings> or <VideoMediaSource> nodes.
    let mut by_id: HashMap<&str, Node<'_, '_>> = HashMap::new();
    let mut by_uid: HashMap<&str, Node<'_, '_>> = HashMap::new();
    for node in root.children().filter(|n| n.is_element()) {
        if let Some(id) = node.attribute("ObjectID") {
            by_id.entry(id).or_insert(node);
        }
        if let Some(uid) = node.attribute("ObjectUID") {
            by_uid.entry(uid).or_insert(node);
        }
    }
    for node in root.descendants().filter(|n| n.is_element()) {
        if let Some(id) = node.attribute("ObjectID") {
            by_id.entry(id).or_insert(node);
        }
        if let Some(uid) = node.attribute("ObjectUID") {
            by_uid.entry(uid).or_insert(node);
        }
    }

    let scale_to_frame_size = root
        .children()
        .find(|n| n.has_tag_name("ProjectSettings"))
        .and_then(|ps| child_text(ps, "ShouldScaleMedia"))
        .map(|v| !v.eq_ignore_ascii_case("false"))
        .unwrap_or(true);

    let color_config = extract_color_config(root);
    let bins = extract_bins(root, &by_id, &by_uid);
    let link_map = extract_link_groups(root, &by_id);

    let mut media_map: BTreeMap<String, MediaAsset> = BTreeMap::new();
    let mut sequences = Vec::new();

    for seq_node in root.children().filter(|n| n.has_tag_name("Sequence")) {
        let seq = parse_sequence(seq_node, &by_id, &by_uid, &link_map, &mut media_map);
        sequences.push(seq);
    }

    // Tag multicam source sequences and upgrade linked audio clips referencing them.
    let mut multicam_seq_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for seq in &sequences {
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            for clip in &track.clips {
                if clip.source_kind == ClipSourceKind::MulticamClip && !clip.media_id.is_empty() {
                    multicam_seq_ids.insert(clip.media_id.clone());
                }
            }
        }
    }
    if !multicam_seq_ids.is_empty() {
        for seq in &mut sequences {
            if multicam_seq_ids.contains(&seq.id) {
                seq.kind = SequenceKind::MulticamClip;
            }
            for track in seq.video_tracks.iter_mut().chain(seq.audio_tracks.iter_mut()) {
                for clip in &mut track.clips {
                    if clip.source_kind == ClipSourceKind::NestedSequence
                        && multicam_seq_ids.contains(&clip.media_id)
                    {
                        clip.source_kind = ClipSourceKind::MulticamClip;
                    }
                }
            }
        }
    }

    // Also collect any project-level Media elements not yet referenced in sequences
    for media_node in root.children().filter(|n| n.has_tag_name("Media")) {
        if let Some(asset) = extract_media_asset(media_node, &by_id) {
            media_map.entry(asset.id.clone()).or_insert(asset);
        }
    }

    Ok(Project {
        name: project_name.to_string(),
        source_format: NleFormat::PremierePro,
        source_path: None,
        color_config,
        scale_to_frame_size,
        bins,
        media_assets: media_map.into_values().collect(),
        sequences,
        metadata: BTreeMap::new(),
    })
}

fn extract_color_config(root: Node<'_, '_>) -> ColorConfig {
    let mut working = None;
    let mut output = None;

    for node in root.descendants().filter(|n| n.is_element()) {
        let tag = node.tag_name().name();
        if tag == "WorkingColorSpace" || tag == "WorkingColorSpaceConfiguration" {
            if working.is_none() {
                if let Some(text) = node.text().filter(|t| !t.trim().is_empty()) {
                    working = Some(decode_color_profile_name(
                        text,
                        tag == "WorkingColorSpaceConfiguration",
                    ));
                }
            }
        } else if tag == "OutputColorSpace" && output.is_none() {
            if let Some(text) = node.text().filter(|t| !t.trim().is_empty()) {
                output = Some(decode_color_profile_name(text, false));
            }
        }
    }

    ColorConfig {
        working_space: working,
        output_space: output,
    }
}

fn decode_color_profile_name(raw: &str, is_config: bool) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
        if is_config {
            if let Some(s) = val.get("workingSpaceID").and_then(|v| v.as_str()) {
                return s.to_string();
            }
        }
        if let Some(s) = val
            .get("baseColorProfile")
            .and_then(|b| b.get("colorProfileName"))
            .and_then(|v| v.as_str())
        {
            return s.to_string();
        }
    }
    raw.trim().to_string()
}

fn resolve_node<'a, 'input>(
    ref_el: Node<'a, 'input>,
    by_id: &HashMap<&str, Node<'a, 'input>>,
    by_uid: &HashMap<&str, Node<'a, 'input>>,
) -> Option<Node<'a, 'input>> {
    if let Some(uref) = ref_el.attribute("ObjectURef") {
        if let Some(n) = by_uid.get(uref) {
            return Some(*n);
        }
    }
    if let Some(oref) = ref_el.attribute("ObjectRef") {
        if let Some(n) = by_id.get(oref) {
            return Some(*n);
        }
    }
    None
}

fn extract_bins(
    root: Node<'_, '_>,
    by_id: &HashMap<&str, Node<'_, '_>>,
    by_uid: &HashMap<&str, Node<'_, '_>>,
) -> Vec<ProjectBin> {
    let bin_nodes: Vec<_> = root
        .children()
        .filter(|n| n.has_tag_name("BinProjectItem"))
        .collect();

    let mut parent_by_uid: HashMap<String, String> = HashMap::new();
    for parent in &bin_nodes {
        let Some(puid) = parent.attribute("ObjectUID") else {
            continue;
        };
        if let Some(items) = find_descendant(*parent, "Items") {
            for item in items.children().filter(|n| n.has_tag_name("Item")) {
                if let Some(child) = resolve_node(item, by_id, by_uid) {
                    if child.has_tag_name("BinProjectItem") {
                        if let Some(cuid) = child.attribute("ObjectUID") {
                            parent_by_uid.insert(cuid.to_string(), puid.to_string());
                        }
                    }
                }
            }
        }
    }

    let mut result = Vec::new();
    for bin_el in bin_nodes {
        let Some(uid) = bin_el.attribute("ObjectUID") else {
            continue;
        };
        let name = nested_text(bin_el, &["ProjectItem", "Name"])
            .or_else(|| child_text(bin_el, "Name"))
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let mut media_paths = Vec::new();
        let mut sequence_ids = Vec::new();

        if let Some(items) = find_descendant(bin_el, "Items") {
            for item in items.children().filter(|n| n.has_tag_name("Item")) {
                let Some(target) = resolve_node(item, by_id, by_uid) else {
                    continue;
                };
                if !target.has_tag_name("ClipProjectItem") {
                    continue;
                }
                let Some(mc_ref) = target.children().find(|n| n.has_tag_name("MasterClip")) else {
                    continue;
                };
                let Some(master) = resolve_node(mc_ref, by_id, by_uid) else {
                    continue;
                };
                let Some(clips_el) = master.children().find(|n| n.has_tag_name("Clips")) else {
                    continue;
                };
                for clip_ref in clips_el.children().filter(|n| n.has_tag_name("Clip")) {
                    let Some(clip_el) = resolve_node(clip_ref, by_id, by_uid) else {
                        continue;
                    };
                    if let Some(seq_uid) = find_sequence_uid_for_clip(clip_el, by_id, by_uid) {
                        if !sequence_ids.contains(&seq_uid) {
                            sequence_ids.push(seq_uid);
                        }
                    } else if let Some((path, _, _)) = find_media_for_clip(clip_el, by_id, by_uid) {
                        if !path.is_empty() && !media_paths.contains(&path) {
                            media_paths.push(path);
                        }
                    }
                }
            }
        }

        result.push(ProjectBin {
            id: uid.to_string(),
            name,
            parent_id: parent_by_uid.get(uid).cloned(),
            media_paths,
            sequence_ids,
        });
    }
    result
}

fn find_sequence_uid_for_clip(
    clip_el: Node<'_, '_>,
    by_id: &HashMap<&str, Node<'_, '_>>,
    by_uid: &HashMap<&str, Node<'_, '_>>,
) -> Option<String> {
    let inner = clip_el.children().find(|n| n.has_tag_name("Clip"))?;
    let src_ref = inner.children().find(|n| n.has_tag_name("Source"))?;
    let source = resolve_node(src_ref, by_id, by_uid)?;
    let seq_src = source.children().find(|n| n.has_tag_name("SequenceSource"))?;
    let seq_ref = seq_src.children().find(|n| n.has_tag_name("Sequence"))?;
    let seq = resolve_node(seq_ref, by_id, by_uid)?;
    seq.attribute("ObjectUID").map(|s| s.to_string())
}

fn find_media_for_clip<'a, 'input>(
    clip_el: Node<'a, 'input>,
    by_id: &HashMap<&str, Node<'a, 'input>>,
    by_uid: &HashMap<&str, Node<'a, 'input>>,
) -> Option<(String, String, Node<'a, 'input>)> {
    let inner = clip_el.children().find(|n| n.has_tag_name("Clip"))?;
    let src_ref = inner.children().find(|n| n.has_tag_name("Source"))?;
    let source = resolve_node(src_ref, by_id, by_uid)?;
    let media_ref = source
        .children()
        .find(|n| n.has_tag_name("MediaSource") || n.has_tag_name("Media"))
        .and_then(|ms| ms.children().find(|n| n.has_tag_name("Media")))?;
    let media = resolve_node(media_ref, by_id, by_uid)?;
    let path = child_text(media, "FilePath")
        .filter(|s| !s.is_empty())
        .or_else(|| child_text(media, "ActualMediaFilePath").filter(|s| !s.is_empty()))
        .or_else(|| child_text(media, "RelativePath"))
        .unwrap_or_default();
    let media_id = media
        .attribute("ObjectUID")
        .or_else(|| media.attribute("ObjectID"))
        .unwrap_or("")
        .to_string();
    Some((path, media_id, media))
}

fn extract_link_groups(
    root: Node<'_, '_>,
    by_id: &HashMap<&str, Node<'_, '_>>,
) -> HashMap<String, String> {
    let mut track_item_to_group = HashMap::new();
    for (idx, link_el) in root.children().filter(|n| n.has_tag_name("Link")).enumerate() {
        let group_id = format!("link-{idx}");
        if let Some(items) = find_descendant(link_el, "TrackItems") {
            for ti in items.children().filter(|n| n.has_tag_name("TrackItem")) {
                if let Some(oref) = ti.attribute("ObjectRef") {
                    if by_id.contains_key(oref) {
                        track_item_to_group.insert(oref.to_string(), group_id.clone());
                    }
                }
            }
        }
    }
    track_item_to_group
}

fn extract_media_asset(
    media: Node<'_, '_>,
    by_id: &HashMap<&str, Node<'_, '_>>,
) -> Option<MediaAsset> {
    let path = child_text(media, "FilePath")
        .filter(|s| !s.is_empty())
        .or_else(|| child_text(media, "ActualMediaFilePath").filter(|s| !s.is_empty()))
        .or_else(|| child_text(media, "RelativePath").filter(|s| !s.is_empty()))?;

    let id = media
        .attribute("ObjectUID")
        .or_else(|| media.attribute("ObjectID"))
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.clone());

    let name = Path::new(&path.replace('\\', "/"))
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .or_else(|| child_text(media, "Title"))
        .unwrap_or_else(|| "media".to_string());

    let video_stream_node = media
        .children()
        .find(|n| n.has_tag_name("VideoStream"))
        .and_then(|vs_ref| vs_ref.attribute("ObjectRef"))
        .and_then(|oref| by_id.get(oref).copied());

    let (width, height) = video_stream_node
        .and_then(|vs| parse_frame_rect(child_text(vs, "FrameRect").as_deref()))
        .unwrap_or((0, 0));

    let asset_fps = video_stream_node
        .and_then(extract_video_stream_fps)
        .unwrap_or(Rational::FPS_25);

    let ext = Path::new(&name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let has_video = media.children().any(|n| n.has_tag_name("VideoStream"));
    let media_kind = if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "tif" | "tiff" | "psd") {
        MediaKind::Image
    } else if has_video || width > 0 {
        MediaKind::Video
    } else {
        MediaKind::Audio
    };

    Some(MediaAsset {
        id,
        name,
        path,
        media_kind,
        width,
        height,
        frame_rate: asset_fps,
        duration_frames: 0,
        source_tc_frame: 0,
        source_tc_string: "00:00:00:00".to_string(),
        audio_channels: 2,
        audio_sample_rate: 48_000,
        color_space: None,
    })
}

fn extract_video_stream_fps(vs: Node<'_, '_>) -> Option<Rational> {
    let overridden = child_text(vs, "IsFrameRateOverridden")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    // Note: Premiere Pro's XML schema spells this tag "OveriddenFrameRate" (single 'r', double 'd').
    let ticks_opt = if overridden {
        child_text(vs, "OveriddenFrameRate")
            .or_else(|| child_text(vs, "OverriddenFrameRate"))
            .or_else(|| child_text(vs, "FrameRate"))
    } else {
        child_text(vs, "FrameRate")
    };
    let ticks = ticks_opt?.trim().parse::<i64>().ok()?;
    if ticks > 0 && ticks < 9_000_000_000_000_000_000 {
        Some(Rational::from_adobe_frame_ticks(ticks))
    } else {
        None
    }
}

fn parse_frame_rect(rect: Option<&str>) -> Option<(u32, u32)> {
    let parts: Vec<&str> = rect?.split(',').collect();
    if parts.len() != 4 {
        return None;
    }
    let w = parts[2].trim().parse::<u32>().ok()?;
    let h = parts[3].trim().parse::<u32>().ok()?;
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

fn parse_sequence(
    seq_node: Node<'_, '_>,
    by_id: &HashMap<&str, Node<'_, '_>>,
    by_uid: &HashMap<&str, Node<'_, '_>>,
    link_map: &HashMap<String, String>,
    media_map: &mut BTreeMap<String, MediaAsset>,
) -> Sequence {
    let id = seq_node
        .attribute("ObjectUID")
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let name = child_text(seq_node, "Name").unwrap_or_else(|| "Untitled Sequence".to_string());

    let mut width = 1920u32;
    let mut height = 1080u32;
    if let Some(props) = find_descendant(seq_node, "Properties") {
        for p in props.children().filter(|n| n.is_element()) {
            let tag = p.tag_name().name();
            if tag.contains("PreviewFrameSizeWidth") {
                if let Some(w) = p.text().and_then(|t| t.trim().parse::<u32>().ok()).filter(|&v| v > 0) {
                    width = w;
                }
            } else if tag.contains("PreviewFrameSizeHeight") {
                if let Some(h) = p.text().and_then(|t| t.trim().parse::<u32>().ok()).filter(|&v| v > 0) {
                    height = h;
                }
            }
        }
    }

    let mut frame_rate = Rational::FPS_25;
    let mut video_tg: Option<Node<'_, '_>> = None;
    let mut audio_tg: Option<Node<'_, '_>> = None;

    if let Some(tg_sec) = seq_node.children().find(|n| n.has_tag_name("TrackGroups")) {
        for tg_pair in tg_sec.children().filter(|n| n.has_tag_name("TrackGroup")) {
            let Some(second) = tg_pair.children().find(|n| n.has_tag_name("Second")) else {
                continue;
            };
            let Some(tg) = resolve_node(second, by_id, by_uid) else {
                continue;
            };
            if tg.has_tag_name("VideoTrackGroup") {
                video_tg = Some(tg);
                if let Some((rw, rh)) = parse_frame_rect(child_text(tg, "FrameRect").as_deref()) {
                    width = rw;
                    height = rh;
                }
                if let Some(fr_ticks) = find_descendant(tg, "FrameRate")
                    .and_then(|n| n.text())
                    .and_then(|t| t.trim().parse::<i64>().ok())
                {
                    frame_rate = Rational::from_adobe_frame_ticks(fr_ticks);
                }
            } else if tg.has_tag_name("AudioTrackGroup") {
                audio_tg = Some(tg);
            }
        }
    }

    let mark_in = nested_text(seq_node, &["Node", "Properties", "MZ.InPoint"])
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&t| t >= 0)
        .map(|t| frame_rate.adobe_ticks_to_frames(t));
    let mark_out = nested_text(seq_node, &["Node", "Properties", "MZ.OutPoint"])
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&t| t >= 0)
        .map(|t| frame_rate.adobe_ticks_to_frames(t));

    let markers = parse_marker_owner(
        seq_node.children().find(|n| n.has_tag_name("MarkerOwner")),
        by_id,
        frame_rate,
        MarkerScope::Sequence,
    );

    let mut video_tracks = Vec::new();
    let mut audio_tracks = Vec::new();
    let mut max_end = 0i64;

    if let Some(vtg) = video_tg {
        if let Some(tracks_el) = find_descendant(vtg, "Tracks") {
            for (idx, track_ref) in tracks_el
                .children()
                .filter(|n| n.has_tag_name("Track"))
                .enumerate()
            {
                let Some(track_el) = resolve_node(track_ref, by_id, by_uid) else {
                    continue;
                };
                let track = parse_track(
                    track_el,
                    idx + 1,
                    TrackKind::Video,
                    width,
                    height,
                    frame_rate,
                    by_id,
                    by_uid,
                    link_map,
                    media_map,
                );
                for c in &track.clips {
                    max_end = max_end.max(c.end_frame);
                }
                video_tracks.push(track);
            }
        }
    }

    if let Some(atg) = audio_tg {
        if let Some(tracks_el) = find_descendant(atg, "Tracks") {
            for (idx, track_ref) in tracks_el
                .children()
                .filter(|n| n.has_tag_name("Track"))
                .enumerate()
            {
                let Some(track_el) = resolve_node(track_ref, by_id, by_uid) else {
                    continue;
                };
                let track = parse_track(
                    track_el,
                    idx + 1,
                    TrackKind::Audio,
                    width,
                    height,
                    frame_rate,
                    by_id,
                    by_uid,
                    link_map,
                    media_map,
                );
                for c in &track.clips {
                    max_end = max_end.max(c.end_frame);
                }
                audio_tracks.push(track);
            }
        }
    }

    let start_tc_frame = child_text(seq_node, "ZeroPoint")
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&t| t > 0)
        .map(|t| frame_rate.adobe_ticks_to_frames(t))
        .unwrap_or(0);

    Sequence {
        id,
        name,
        kind: nle_core::SequenceKind::Timeline,
        width,
        height,
        frame_rate,
        duration_frames: max_end,
        start_tc_frame,
        mark_in,
        mark_out,
        video_tracks,
        audio_tracks,
        markers,
        metadata: BTreeMap::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_track(
    track_el: Node<'_, '_>,
    index: usize,
    kind: TrackKind,
    seq_w: u32,
    seq_h: u32,
    seq_fps: Rational,
    by_id: &HashMap<&str, Node<'_, '_>>,
    by_uid: &HashMap<&str, Node<'_, '_>>,
    link_map: &HashMap<String, String>,
    media_map: &mut BTreeMap<String, MediaAsset>,
) -> Track {
    let ct = track_el.children().find(|n| n.has_tag_name("ClipTrack"));
    let track_node = ct.and_then(|c| c.children().find(|n| n.has_tag_name("Track")));

    let muted = track_node
        .and_then(|t| child_text(t, "IsMuted"))
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let locked = track_node
        .and_then(|t| child_text(t, "IsLocked"))
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let solo = track_node
        .and_then(|t| child_text(t, "Solo"))
        .map(|v| !matches!(v.trim(), "" | "0" | "false" | "FALSE"))
        .unwrap_or(false);
    let name = track_node
        .and_then(|t| nested_text(t, &["Node", "Properties", "MZ.TrackName"]))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if kind == TrackKind::Video {
                format!("V{index}")
            } else {
                format!("A{index}")
            }
        });

    let track_markers = parse_marker_owner(
        track_el.children().find(|n| n.has_tag_name("MarkerOwner")),
        by_id,
        seq_fps,
        MarkerScope::Track,
    );

    let mut clips = Vec::new();
    let mut transitions = Vec::new();
    if let Some(clip_track) = ct {
        // Premiere stores clips in <ClipItems><TrackItems> and transitions in <TransitionItems><TrackItems>.
        for items_sec in clip_track
            .descendants()
            .filter(|n| n.has_tag_name("TrackItems"))
        {
            for ti_ref in items_sec.children().filter(|n| n.has_tag_name("TrackItem")) {
                let Some(ti_el) = resolve_node(ti_ref, by_id, by_uid) else {
                    continue;
                };
                if let Some(tti) = ti_el.children().find(|n| n.has_tag_name("TransitionTrackItem")) {
                    if let Some(tr) = parse_transition(tti, seq_fps) {
                        transitions.push(tr);
                    }
                    continue;
                }
                let Some(cti) = ti_el.children().find(|n| n.has_tag_name("ClipTrackItem")) else {
                    continue;
                };
                let ti_obj_id = ti_el.attribute("ObjectID").unwrap_or("");
                let clip = parse_clip(
                    cti,
                    ti_obj_id,
                    kind,
                    seq_w,
                    seq_h,
                    seq_fps,
                    by_id,
                    by_uid,
                    link_map,
                    media_map,
                );
                clips.push(clip);
            }
        }
    }

    Track {
        index,
        kind,
        name,
        enabled: !muted,
        locked,
        muted,
        solo,
        clips,
        transitions,
        markers: track_markers,
    }
}

fn parse_transition(tti: Node<'_, '_>, seq_fps: Rational) -> Option<Transition> {
    let ti_inner = tti.children().find(|n| n.has_tag_name("TrackItem"));
    // Premiere omits <Start> when the transition begins at tick 0.
    let start_frame = ti_inner
        .and_then(|n| child_text(n, "Start"))
        .and_then(|s| s.trim().parse::<i64>().ok())
        .map(|t| seq_fps.adobe_ticks_to_frames(t))
        .unwrap_or(0);
    let end_frame = ti_inner
        .and_then(|n| child_text(n, "End"))
        .and_then(|s| s.trim().parse::<i64>().ok())
        .map(|t| seq_fps.adobe_ticks_to_frames(t))
        .unwrap_or(start_frame);
    if end_frame <= start_frame {
        return None;
    }

    let match_name = child_text(tti, "MatchName").unwrap_or_default();
    let display_name = child_text(tti, "DisplayName")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if match_name.is_empty() {
                "Transition".to_string()
            } else {
                match_name.clone()
            }
        });
    let has_out = child_text(tti, "HasOutgoingClip")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let has_in = child_text(tti, "HasIncomingClip")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let alignment = match (has_out, has_in) {
        (false, true) => "start".to_string(),
        (true, false) => "end".to_string(),
        _ => "center".to_string(),
    };

    Some(Transition {
        name: display_name,
        effect_id: if match_name.is_empty() {
            "Transition".to_string()
        } else {
            match_name
        },
        start_frame,
        end_frame,
        alignment,
    })
}

#[allow(clippy::too_many_arguments)]
fn parse_clip(
    cti: Node<'_, '_>,
    ti_obj_id: &str,
    kind: TrackKind,
    seq_w: u32,
    seq_h: u32,
    seq_fps: Rational,
    by_id: &HashMap<&str, Node<'_, '_>>,
    by_uid: &HashMap<&str, Node<'_, '_>>,
    link_map: &HashMap<String, String>,
    media_map: &mut BTreeMap<String, MediaAsset>,
) -> Clip {
    let ti_inner = cti.children().find(|n| n.has_tag_name("TrackItem"));
    let start_frame = ti_inner
        .and_then(|n| child_text(n, "Start"))
        .and_then(|s| s.trim().parse::<i64>().ok())
        .map(|t| seq_fps.adobe_ticks_to_frames(t))
        .unwrap_or(0);
    let end_frame = ti_inner
        .and_then(|n| child_text(n, "End"))
        .and_then(|s| s.trim().parse::<i64>().ok())
        .map(|t| seq_fps.adobe_ticks_to_frames(t))
        .unwrap_or(start_frame);
    let enabled = !ti_inner
        .and_then(|n| child_text(n, "Deactivated"))
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let mut name = "(unnamed clip)".to_string();
    let mut media_path = String::new();
    let mut media_id = String::new();
    let mut media_kind = if kind == TrackKind::Audio {
        MediaKind::Audio
    } else {
        MediaKind::Video
    };
    let mut source_kind = ClipSourceKind::Media;
    let mut active_angle: Option<String> = None;
    let mut source_fps = seq_fps;
    let mut source_w = seq_w;
    let mut source_h = seq_h;
    let mut source_in_ticks: Option<i64> = None;
    let mut source_out_ticks: Option<i64> = None;
    let mut source_in_frame = 0i64;
    let mut source_out_frame = (end_frame - start_frame).max(0);
    let mut source_full_duration = 0i64;
    let mut source_tc_frame = 0i64;
    let mut sample_rate = 48_000u32;
    let mut speed_percent = 100.0f64;
    let mut reversed = false;
    let mut audio_channel = 1u32;
    let mut label_name: Option<String> = None;
    let mut label_preset_index: Option<u8> = None;

    if let Some(sc_el) = cti
        .children()
        .find(|n| n.has_tag_name("SubClip"))
        .and_then(|r| resolve_node(r, by_id, by_uid))
    {
        if let Some(n) = child_text(sc_el, "Name").filter(|s| !s.is_empty()) {
            name = n;
        }
        // MasterClip -> LoggingInfo
        if let Some(mc_el) = sc_el
            .children()
            .find(|n| n.has_tag_name("MasterClip"))
            .and_then(|r| resolve_node(r, by_id, by_uid))
        {
            if let Some(cli) = mc_el
                .children()
                .find(|n| n.has_tag_name("LoggingInfo"))
                .and_then(|r| resolve_node(r, by_id, by_uid))
            {
                if let Some(mfr_ticks) = child_text(cli, "MediaFrameRate")
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .filter(|&v| v > 0 && v < 9_000_000_000_000_000_000)
                {
                    let rate = Rational::from_adobe_frame_ticks(mfr_ticks);
                    if rate.as_f64() > 1000.0 {
                        sample_rate = rate.as_f64().round() as u32;
                    } else {
                        source_fps = rate;
                    }
                }
                let mip = child_text(cli, "MediaInPoint").and_then(|s| s.trim().parse::<i64>().ok());
                let mop = child_text(cli, "MediaOutPoint").and_then(|s| s.trim().parse::<i64>().ok());
                if let Some(mip_ticks) = mip.filter(|&v| v >= 0) {
                    source_tc_frame = source_fps.adobe_ticks_to_frames(mip_ticks);
                }
                if let (Some(mip_t), Some(mop_t)) = (mip, mop) {
                    if mop_t > mip_t {
                        source_full_duration = source_fps.adobe_ticks_to_frames(mop_t - mip_t);
                    }
                }
            }
        }

        // SubClip -> Clip
        if let Some(clip_el) = sc_el
            .children()
            .find(|n| n.has_tag_name("Clip"))
            .and_then(|r| resolve_node(r, by_id, by_uid))
        {
            if let Some(lbl) = find_descendant(clip_el, "asl.clip.label.name")
                .and_then(|n| n.text())
                .filter(|s| !s.is_empty())
            {
                label_name = Some(lbl.to_string());
                label_preset_index = lbl
                    .rsplit('.')
                    .next()
                    .and_then(|num| num.parse::<u8>().ok());
            }

            if let Some(seq_uid) = find_sequence_uid_for_clip(clip_el, by_id, by_uid) {
                media_id = seq_uid;
                let inner_clip = clip_el.children().find(|n| n.has_tag_name("Clip"));
                let is_mc = inner_clip
                    .and_then(|c| child_text(c, "IsMulticam"))
                    .map(|v| v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if is_mc {
                    source_kind = ClipSourceKind::MulticamClip;
                    active_angle = inner_clip
                        .and_then(|c| child_text(c, "SelectedTrackIndex"))
                        .and_then(|s| s.trim().parse::<u32>().ok())
                        .map(|idx| format!("Camera {}", idx + 1));
                } else {
                    source_kind = ClipSourceKind::NestedSequence;
                }
            } else if let Some((path, mid, media_node)) = find_media_for_clip(clip_el, by_id, by_uid) {
                media_path = path;
                media_id = mid;
                let ext = Path::new(&media_path.replace('\\', "/"))
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let has_vs = media_node.children().any(|n| n.has_tag_name("VideoStream"));
                if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "tif" | "tiff" | "psd") {
                    media_kind = MediaKind::Image;
                } else if has_vs {
                    media_kind = MediaKind::Video;
                } else if kind == TrackKind::Audio {
                    media_kind = MediaKind::Audio;
                }

                if let Some(vs) = media_node
                    .children()
                    .find(|n| n.has_tag_name("VideoStream"))
                    .and_then(|r| resolve_node(r, by_id, by_uid))
                {
                    if let Some((rw, rh)) = parse_frame_rect(child_text(vs, "FrameRect").as_deref()) {
                        source_w = rw;
                        source_h = rh;
                    }
                    if let Some(overridden_fps) = extract_video_stream_fps(vs) {
                        source_fps = overridden_fps;
                    }
                }
            }

            if let Some(inner) = clip_el.children().find(|n| n.has_tag_name("Clip")) {
                if let Some(ip) = child_text(inner, "InPoint").and_then(|s| s.trim().parse::<i64>().ok()) {
                    source_in_ticks = Some(ip);
                    source_in_frame = seq_fps.adobe_ticks_to_frames(ip);
                }
                if let Some(op) = child_text(inner, "OutPoint").and_then(|s| s.trim().parse::<i64>().ok()) {
                    source_out_ticks = Some(op);
                    source_out_frame = seq_fps.adobe_ticks_to_frames(op);
                }
                if let Some(ps) = child_text(inner, "PlaybackSpeed")
                    .or_else(|| child_text(clip_el, "PlaybackSpeed"))
                    .and_then(|s| s.trim().parse::<f64>().ok())
                {
                    speed_percent = if ps.abs() <= 10.0 { ps * 100.0 } else { ps };
                }
                if child_text(inner, "PlayBackwards")
                    .or_else(|| child_text(clip_el, "PlayBackwards"))
                    .map(|v| v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false)
                {
                    reversed = true;
                    if speed_percent > 0.0 {
                        speed_percent = -speed_percent;
                    }
                } else if speed_percent < 0.0 {
                    reversed = true;
                }
            }

            // Extract 1-based source audio channel from SecondaryContents/SecondaryContentItem -> ChannelIndex
            if kind == TrackKind::Audio {
                if let Some(sec_contents) = clip_el.children().find(|n| n.has_tag_name("SecondaryContents")) {
                    if let Some(first_item) = sec_contents
                        .children()
                        .find(|n| n.has_tag_name("SecondaryContentItem"))
                    {
                        if let Some(sec_node) = resolve_node(first_item, by_id, by_uid) {
                            if let Some(ch_idx) = child_text(sec_node, "ChannelIndex")
                                .and_then(|s| s.trim().parse::<u32>().ok())
                            {
                                audio_channel = ch_idx + 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // Register or update media asset record
    let resolved_media_id = if !media_id.is_empty() {
        media_id.clone()
    } else if !media_path.is_empty() {
        media_path.clone()
    } else {
        name.clone()
    };

    let dur_est = source_full_duration
        .max(source_out_frame)
        .max((end_frame - start_frame).max(1));

    // Register or update media asset record only for actual media-backed clips
    // (never pollute media_assets with nested sequence / multicam containers).
    if source_kind == ClipSourceKind::Media {
        media_map
            .entry(resolved_media_id.clone())
            .and_modify(|existing| {
                if existing.duration_frames < dur_est {
                    existing.duration_frames = dur_est;
                }
                if existing.width == 0 && source_w > 0 && kind == TrackKind::Video {
                    existing.width = source_w;
                    existing.height = source_h;
                }
                if existing.media_kind == MediaKind::Audio && media_kind == MediaKind::Video {
                    existing.media_kind = MediaKind::Video;
                }
            })
            .or_insert_with(|| MediaAsset {
                id: resolved_media_id.clone(),
                name: Path::new(&media_path.replace('\\', "/"))
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| name.clone()),
                path: media_path.clone(),
                media_kind,
                width: if kind == TrackKind::Video { source_w } else { 0 },
                height: if kind == TrackKind::Video { source_h } else { 0 },
                frame_rate: source_fps,
                duration_frames: dur_est,
                source_tc_frame,
                source_tc_string: frames_to_timecode(source_tc_frame, source_fps),
                audio_channels: 2,
                audio_sample_rate: sample_rate,
                color_space: None,
            });
    }

    // Extract components (Motion, Opacity, Lumetri, and ALL other video/audio effects!)
    let mut motion = MotionTransform::default();
    let mut opacity = 100.0f64;
    let mut lumetri_effects: Vec<LumetriData> = Vec::new();
    let mut effects: Vec<EffectInstance> = Vec::new();

    if let Some(chain) = cti
        .children()
        .find(|n| n.has_tag_name("ComponentOwner"))
        .and_then(|co| co.children().find(|n| n.has_tag_name("Components")))
        .and_then(|r| resolve_node(r, by_id, by_uid))
    {
        let comp_list = chain
            .children()
            .find(|n| n.has_tag_name("Components"))
            .or_else(|| {
                chain
                    .children()
                    .find(|n| n.has_tag_name("ComponentChain"))
                    .and_then(|cc| cc.children().find(|n| n.has_tag_name("Components")))
            });

        if let Some(comp_list) = comp_list {
            for comp_ref in comp_list.children().filter(|n| n.has_tag_name("Component")) {
                let Some(component) = resolve_node(comp_ref, by_id, by_uid) else {
                    continue;
                };
                let match_name = child_text(component, "MatchName")
                    .or_else(|| child_text(component, "FilterMatchName"))
                    .unwrap_or_default();
                let lower = match_name.to_ascii_lowercase();
                let is_intrinsic = find_descendant(component, "Intrinsic")
                    .and_then(|n| n.text())
                    .map(|v| v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                let display_name = child_text(component, "InstanceName")
                    .or_else(|| nested_text(component, &["Component", "DisplayName"]))
                    .or_else(|| find_descendant(component, "DisplayName").and_then(|n| n.text()).map(|s| s.to_string()))
                    .or_else(|| child_text(component, "Name"))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| match_name.clone());

                if lower.contains("adbe motion") {
                    parse_motion_component(component, by_id, &mut motion);
                    continue;
                }
                if lower.contains("lumetri") {
                    let order = lumetri_effects.len();
                    lumetri_effects.push(parse_lumetri_component(component, order, by_id));
                    continue;
                }
                if lower.contains("adbe opacity") {
                    let params = extract_effect_params(component, by_id);
                    let mut is_nondefault = false;
                    for p in &params {
                        if p.id == "1" {
                            if let Ok(val) = p.value.parse::<f64>() {
                                opacity = val;
                                if (val - 100.0).abs() > 1e-3 {
                                    is_nondefault = true;
                                }
                            }
                        }
                        if p.is_animated {
                            is_nondefault = true;
                        }
                    }
                    if is_nondefault {
                        effects.push(EffectInstance {
                            id: component.attribute("ObjectID").unwrap_or("").to_string(),
                            match_name,
                            display_name: "Opacity".to_string(),
                            category: EffectCategory::Transform,
                            media_kind: kind,
                            enabled: true,
                            is_intrinsic: false,
                            is_animated: params.iter().any(|p| p.is_animated),
                            parameters: params,
                        });
                    }
                    continue;
                }
                // Skip intrinsic audio volume/panner/channel volume components
                if is_intrinsic
                    || lower.contains("adbe volume")
                    || lower.contains("adbe panner")
                    || lower.contains("adbe channel volume")
                    || lower.starts_with("internal volume")
                    || lower.starts_with("internal channel volume")
                {
                    continue;
                }

                if !match_name.is_empty() {
                    let params = extract_effect_params(component, by_id);
                    let is_animated = params.iter().any(|p| p.is_animated);
                    let category = classify_effect_category(&lower, kind);
                    effects.push(EffectInstance {
                        id: component.attribute("ObjectID").unwrap_or("").to_string(),
                        match_name,
                        display_name,
                        category,
                        media_kind: kind,
                        enabled: true,
                        is_intrinsic: false,
                        is_animated,
                        parameters: params,
                    });
                }
            }
        }
    }

    // Detect Adobe Essential Sound panel processing stored in TrackItem Properties (ESP.*)
    if kind == TrackKind::Audio {
        if let Some(props) = ti_inner.and_then(|n| find_descendant(n, "Properties")) {
            if let Some(mode) = child_text(props, "ESP.AdjustmentsModeID").filter(|s| !s.is_empty()) {
                let mut esp_params = Vec::new();
                for p in props.children().filter(|n| n.is_element()) {
                    let tag = p.tag_name().name();
                    if tag.starts_with("ESP.RestorationSection.")
                        || tag.starts_with("ESP.ClaritySection.")
                        || tag.starts_with("ESP.Loudness")
                        || tag.starts_with("ESP.CreativeSection.")
                    {
                        if let Some(val) = p.text().map(str::trim).filter(|v| !v.is_empty()) {
                            esp_params.push(EffectParameter {
                                id: tag.to_string(),
                                name: tag.trim_start_matches("ESP.").to_string(),
                                value: val.to_string(),
                                is_animated: false,
                            });
                        }
                    }
                }
                if !esp_params.is_empty() {
                    let mode_label = match mode.as_str() {
                        "dialog" => "Dialogue",
                        "music" => "Music",
                        "sfx" => "SFX",
                        "ambience" => "Ambience",
                        other => other,
                    };
                    effects.push(EffectInstance {
                        id: format!("esp-{ti_obj_id}"),
                        match_name: format!("Premiere.EssentialSound.{mode}"),
                        display_name: format!("Essential Sound ({mode_label})"),
                        category: EffectCategory::Audio,
                        media_kind: TrackKind::Audio,
                        enabled: true,
                        is_intrinsic: false,
                        is_animated: false,
                        parameters: esp_params,
                    });
                }
            }
        }
    }

    Clip {
        id: if ti_obj_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            ti_obj_id.to_string()
        },
        name,
        media_id: resolved_media_id,
        media_path,
        media_kind,
        source_kind,
        active_angle,
        start_frame,
        end_frame,
        source_in_frame,
        source_out_frame,
        source_in_ticks,
        source_out_ticks,
        source_fps,
        source_width: source_w,
        source_height: source_h,
        speed_percent,
        reversed,
        enabled,
        audio_channel,
        link_group_id: link_map.get(ti_obj_id).cloned(),
        label_name,
        label_preset_index,
        motion,
        opacity,
        lumetri_effects,
        effects,
        markers: Vec::new(),
    }
}

fn parse_motion_component(
    component: Node<'_, '_>,
    by_id: &HashMap<&str, Node<'_, '_>>,
    motion: &mut MotionTransform,
) {
    let Some(params_sec) = find_descendant(component, "Params") else {
        return;
    };
    for p_ref in params_sec.children().filter(|n| n.has_tag_name("Param")) {
        let Some(oid) = p_ref.attribute("ObjectRef") else {
            continue;
        };
        let Some(p) = by_id.get(oid).copied() else {
            continue;
        };
        let pid = child_text(p, "ParameterID").unwrap_or_default();
        let raw_kf = child_text(p, "Keyframes").unwrap_or_default();
        let is_tv = child_text(p, "IsTimeVarying")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if is_tv || !raw_kf.trim().is_empty() {
            motion.has_keyframes = true;
        }
        let Some(start_raw) = child_text(p, "StartKeyframe") else {
            continue;
        };
        let val_part = start_raw.split(',').nth(1).unwrap_or("").trim();
        match pid.as_str() {
            "1" => {
                // Position: "0.5:0.5" normalized center
                let coords: Vec<f64> = val_part
                    .split(':')
                    .filter_map(|s| s.parse::<f64>().ok())
                    .collect();
                if coords.len() >= 2 {
                    motion.position_x = coords[0] - 0.5;
                    motion.position_y = -(coords[1] - 0.5);
                }
            }
            "2" => {
                if let Some(s) = parse_keyframe_scalar(&start_raw) {
                    motion.scale = s;
                }
            }
            "5" => {
                if let Some(r) = parse_keyframe_scalar(&start_raw) {
                    motion.rotation = r;
                }
            }
            "6" => {
                let coords: Vec<f64> = val_part
                    .split(':')
                    .filter_map(|s| s.parse::<f64>().ok())
                    .collect();
                if coords.len() >= 2 {
                    motion.anchor_x = coords[0] - 0.5;
                    motion.anchor_y = coords[1] - 0.5;
                }
            }
            _ => {}
        }
    }
}

fn extract_effect_params(
    component: Node<'_, '_>,
    by_id: &HashMap<&str, Node<'_, '_>>,
) -> Vec<EffectParameter> {
    let mut out = Vec::new();
    let Some(params_sec) = find_descendant(component, "Params") else {
        return out;
    };
    for p_ref in params_sec.children().filter(|n| n.has_tag_name("Param")) {
        let Some(oid) = p_ref.attribute("ObjectRef") else {
            continue;
        };
        let Some(p) = by_id.get(oid).copied() else {
            continue;
        };
        let id = child_text(p, "ParameterID").unwrap_or_default();
        let name = child_text(p, "Name").unwrap_or_else(|| format!("Param {id}"));
        let value = child_text(p, "StartKeyframe")
            .and_then(|raw| raw.split(',').nth(1).map(|v| v.trim().to_string()))
            .unwrap_or_default();
        let is_animated = child_text(p, "IsTimeVarying")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
            || child_text(p, "Keyframes")
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
        out.push(EffectParameter {
            id,
            name,
            value,
            is_animated,
        });
    }
    out
}

fn classify_effect_category(lower_match: &str, kind: TrackKind) -> EffectCategory {
    if kind == TrackKind::Audio {
        return EffectCategory::Audio;
    }
    if lower_match.contains("blur") {
        EffectCategory::Blur
    } else if lower_match.contains("stabiliz") {
        EffectCategory::Stabilization
    } else if lower_match.contains("key") {
        EffectCategory::Keying
    } else if lower_match.contains("crop") || lower_match.contains("transform") || lower_match.contains("flip") {
        EffectCategory::Transform
    } else if lower_match.contains("color") || lower_match.contains("tint") || lower_match.contains("lumetri") {
        EffectCategory::Color
    } else if lower_match.contains("sharpen") || lower_match.contains("noise") || lower_match.contains("grain") {
        EffectCategory::Stylize
    } else if lower_match.contains("pin") || lower_match.contains("warp") || lower_match.contains("distort") {
        EffectCategory::Distort
    } else if lower_match.contains("text") || lower_match.contains("graphic") || lower_match.contains("mogrt") {
        EffectCategory::Title
    } else if lower_match.contains("dissolve") || lower_match.contains("morph") {
        EffectCategory::Transition
    } else {
        EffectCategory::Plugin
    }
}

fn parse_marker_owner(
    owner: Option<Node<'_, '_>>,
    by_id: &HashMap<&str, Node<'_, '_>>,
    fps: Rational,
    scope: MarkerScope,
) -> Vec<Marker> {
    let Some(owner) = owner else {
        return Vec::new();
    };
    let Some(markers_obj) = owner
        .children()
        .find(|n| n.has_tag_name("Markers"))
        .and_then(|r| r.attribute("ObjectRef"))
        .and_then(|oid| by_id.get(oid).copied())
    else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for second in markers_obj
        .descendants()
        .filter(|n| n.has_tag_name("Second"))
    {
        let Some(oid) = second.attribute("ObjectRef") else {
            continue;
        };
        let Some(marker_node) = by_id.get(oid).copied() else {
            continue;
        };
        let Some(payload) = child_text(marker_node, "DVAMarker") else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let details = json.get("DVAMarker").unwrap_or(&json);
        let start_ticks = details
            .pointer("/mStartTime/ticks")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0);
        let dur_ticks = details
            .pointer("/mDuration/ticks")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0);
        let name = details
            .get("mName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let comment = details
            .get("mComments")
            .or_else(|| details.get("mComment"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        out.push(Marker {
            frame: fps.adobe_ticks_to_frames(start_ticks),
            duration_frames: fps.adobe_ticks_to_frames(dur_ticks).max(1),
            name,
            comment,
            color: "Blue".to_string(),
            scope,
        });
    }
    out
}
