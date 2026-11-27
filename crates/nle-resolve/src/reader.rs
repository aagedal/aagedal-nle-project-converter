use crate::audio::{decode_fairlight_mixer, decode_virtual_audio_track, FairlightMixerData};
use crate::binary::{
    decode_keyed_fields, decode_protobuf, decode_utf16be, decode_wrapper_hex, hex_decode,
    ProtoValue,
};
use crate::grade::{has_resolve_stabilization, parse_resolve_grade_nodes};
use crate::state::{
    decode_lockable_blob, decode_media_metadata_blob, resolve_color_to_premiere_label,
};
use crate::timemap::decode_media_timemap_full;
use nle_core::{
    frames_to_timecode, AlphaMode, Clip, ClipSourceKind, ColorConfig, DataLevels, EffectCategory,
    EffectInstance, Marker, MarkerScope, MediaAsset, MediaKind, MotionTransform, NleError,
    NleFormat, NleResult, Project, ProjectBin, ProjectReader, Rational, RetimeInterpolation,
    Sequence, SequenceKind, Track, TrackKind, Transition,
};
use roxmltree::{Document, Node};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

pub struct ResolveReader;

impl ProjectReader for ResolveReader {
    fn format(&self) -> NleFormat {
        NleFormat::DaVinciResolve
    }

    fn read_file(&self, path: &Path) -> NleResult<Project> {
        read_drp_file(path)
    }
}

/// Read a DaVinci Resolve `.drp` ZIP project archive directly into our canonical `Project` IR.
/// Enables inspecting Resolve projects and exporting Resolve → FCPXML / OTIO / FCP7 XML (and future `.prproj`).
pub fn read_drp_file(path: &Path) -> NleResult<Project> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).map_err(|e| NleError::Archive(e.to_string()))?;

    let mut members: BTreeMap<String, String> = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| NleError::Archive(e.to_string()))?;
        if entry.name().ends_with(".xml") {
            let mut text = String::new();
            let _ = entry.read_to_string(&mut text);
            members.insert(entry.name().to_string(), text);
        }
    }

    let fallback_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Resolve Project")
        .to_string();

    let project_name = members
        .get("project.xml")
        .and_then(|xml| extract_simple_tag(xml, "ProjectName"))
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_name);

    // Decode sequence markers, per-clip markers/colors, and MediaPool clip metadata from project.xml LockableBlobs.
    let mut seq_markers_by_id: HashMap<String, Vec<Marker>> = HashMap::new();
    let mut clip_markers_by_id: HashMap<String, Vec<Marker>> = HashMap::new();
    let mut clip_color_by_id: HashMap<String, u32> = HashMap::new();
    let mut media_meta_by_info_id: HashMap<String, (BTreeMap<String, String>, BTreeMap<String, String>)> =
        HashMap::new();
    if let Some(proj_xml) = members.get("project.xml") {
        let sanitized = sanitize_resolve_xml(proj_xml);
        if let Ok(doc) = Document::parse(&sanitized) {
            for node in doc.root_element().descendants().filter(|n| n.is_element()) {
                match node.tag_name().name() {
                    "Sm2SequenceLockableBlob" => {
                        if let (Some(owner), Some(hex)) =
                            (child_text(node, "BlobOwner"), child_text(node, "FieldsBlob"))
                        {
                            let (markers, _) = decode_lockable_blob(&hex, MarkerScope::Sequence);
                            if !markers.is_empty() {
                                seq_markers_by_id.entry(owner).or_default().extend(markers);
                            }
                        }
                    }
                    "Sm2TiItemLockableBlob" => {
                        if let (Some(owner), Some(hex)) =
                            (child_text(node, "BlobOwner"), child_text(node, "FieldsBlob"))
                        {
                            let (markers, color) = decode_lockable_blob(&hex, MarkerScope::Clip);
                            if !markers.is_empty() {
                                clip_markers_by_id
                                    .entry(owner.clone())
                                    .or_default()
                                    .extend(markers);
                            }
                            if let Some(c) = color {
                                clip_color_by_id.insert(owner, c);
                            }
                        }
                    }
                    "BtMetadataLockableBlob" => {
                        if let (Some(owner), Some(hex)) =
                            (child_text(node, "BlobOwner"), child_text(node, "FieldsBlob"))
                        {
                            let (std_m, custom_m) = decode_media_metadata_blob(&hex);
                            if !std_m.is_empty() || !custom_m.is_empty() {
                                let entry = media_meta_by_info_id.entry(owner).or_default();
                                entry.0.extend(std_m);
                                entry.1.extend(custom_m);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    struct SeqMeta {
        name: String,
        kind: SequenceKind,
        width: u32,
        height: u32,
        fps: Rational,
        start_tc_frame: i64,
        fairlight: Option<FairlightMixerData>,
    }

    let mut seq_meta_by_id: HashMap<String, SeqMeta> = HashMap::new();
    // Maps MediaPoolItem DbId -> (ClipSourceKind, inner Sm2Sequence DbId) for nested timelines,
    // compound clips, and multicam clips.
    let mut mp_container_by_id: HashMap<String, (ClipSourceKind, String)> = HashMap::new();
    let mut bins: Vec<ProjectBin> = Vec::new();
    let mut media_assets: BTreeMap<String, MediaAsset> = BTreeMap::new();

    for (member_name, xml) in &members {
        if !member_name.starts_with("MediaPool/") || !member_name.ends_with("MpFolder.xml") {
            continue;
        }
        let sanitized = sanitize_resolve_xml(xml);
        let Ok(doc) = Document::parse(&sanitized) else {
            continue;
        };
        let root = doc.root_element();
        let bin_id = root.attribute("DbId").unwrap_or("").to_string();
        let bin_name = child_text(root, "Name").unwrap_or_default();
        let parent_id = child_text(root, "MpFolder").filter(|s| !s.is_empty());

        let mut bin_media_paths = Vec::new();
        let mut bin_seq_ids = Vec::new();

        for node in root.descendants().filter(|n| n.is_element()) {
            match node.tag_name().name() {
                "Sm2MpTimelineClip" | "Sm2MpCompoundClip" | "Sm2MpMulticamClip" => {
                    let mp_db_id = node.attribute("DbId").unwrap_or("").to_string();
                    let (seq_kind, clip_source_kind, default_name) = match node.tag_name().name() {
                        "Sm2MpCompoundClip" => (
                            SequenceKind::CompoundClip,
                            ClipSourceKind::CompoundClip,
                            "Compound Clip",
                        ),
                        "Sm2MpMulticamClip" => (
                            SequenceKind::MulticamClip,
                            ClipSourceKind::MulticamClip,
                            "Multicam Clip",
                        ),
                        _ => (
                            SequenceKind::Timeline,
                            ClipSourceKind::NestedSequence,
                            "Timeline",
                        ),
                    };
                    let tl_name =
                        child_text(node, "Name").unwrap_or_else(|| default_name.to_string());
                    if let Some(seq_el) = node.descendants().find(|n| n.has_tag_name("Sm2Sequence")) {
                        let seq_id = seq_el.attribute("DbId").unwrap_or("").to_string();
                        let fps = child_text(seq_el, "FrameRate")
                            .and_then(|h| decode_resolve_fps(&h))
                            .unwrap_or(Rational::FPS_25);
                        let (width, height) = child_text(seq_el, "Resolution")
                            .and_then(|h| decode_resolve_resolution(&h))
                            .unwrap_or((1920, 1080));
                        let start_tc_frame = child_text(seq_el, "MediaExtents")
                            .and_then(|h| decode_resolve_media_extents(&h))
                            .map(|(start_sec, _)| (start_sec * fps.as_f64()).round() as i64)
                            .unwrap_or(0)
                            .max(0);
                        let fairlight = child_text(seq_el, "FieldsBlob")
                            .and_then(|h| hex_decode(&h).ok())
                            .and_then(|b| decode_fairlight_mixer(&b));
                        if !seq_id.is_empty() {
                            if !mp_db_id.is_empty() {
                                mp_container_by_id
                                    .insert(mp_db_id, (clip_source_kind, seq_id.clone()));
                            }
                            bin_seq_ids.push(seq_id.clone());
                            seq_meta_by_id.insert(
                                seq_id,
                                SeqMeta {
                                    name: tl_name,
                                    kind: seq_kind,
                                    width,
                                    height,
                                    fps,
                                    start_tc_frame,
                                    fairlight,
                                },
                            );
                        }
                    }
                }
                "Sm2MpVideoClip" | "Sm2MpAudioClip" => {
                    let db_id = node.attribute("DbId").unwrap_or("").to_string();
                    let name = child_text(node, "Name").unwrap_or_default();
                    let full_path = node
                        .descendants()
                        .find(|n| n.has_tag_name("Clip"))
                        .and_then(|c| c.text())
                        .and_then(decode_bt_clip_path)
                        .unwrap_or_else(|| name.clone());
                    if !full_path.is_empty() {
                        bin_media_paths.push(full_path.clone());
                    }
                    let kind = if node.has_tag_name("Sm2MpAudioClip") {
                        MediaKind::Audio
                    } else {
                        MediaKind::Video
                    };
                    let (mut width, mut height) = (0u32, 0u32);
                    let mut asset_fps = Rational::FPS_25;
                    let mut duration_frames = 0i64;
                    let mut source_tc_frame = 0i64;
                    let mut video_codec: Option<String> = None;
                    let mut audio_codec: Option<String> = None;
                    let mut audio_channels = 2u32;
                    let mut audio_sample_rate = 48_000u32;

                    if let Some(bvi) = node.descendants().find(|n| n.has_tag_name("BtVideoInfo")) {
                        if let Some(clip_hex) = child_text(bvi, "Clip") {
                            video_codec = decode_bt_clip_codec(&clip_hex);
                        }
                        if let Some(geo_hex) = child_text(bvi, "Geometry") {
                            if let Some((w, h)) = decode_bt_geometry(&geo_hex) {
                                width = w;
                                height = h;
                            }
                        }
                        if let Some(time_hex) = child_text(bvi, "Time") {
                            let (sf, nf, fr) = decode_bt_time(&time_hex);
                            if let Some(s) = sf {
                                source_tc_frame = s;
                            }
                            if let Some(n) = nf {
                                duration_frames = n;
                            }
                            if let Some(f) = fr {
                                asset_fps = f;
                            }
                        }
                    }

                    if let Some(bai) = node.descendants().find(|n| n.has_tag_name("BtAudioInfo")) {
                        if let Some(clip_hex) = child_text(bai, "Clip") {
                            audio_codec = decode_bt_clip_codec(&clip_hex);
                        }
                        if let Some(tracks_hex) = child_text(bai, "TracksBA") {
                            let (ch, sr, t_codec) = decode_bt_audio_tracks(&tracks_hex);
                            if let Some(c) = ch {
                                audio_channels = c;
                            }
                            if let Some(r) = sr {
                                audio_sample_rate = r;
                            }
                            if audio_codec.is_none() {
                                audio_codec = t_codec;
                            }
                        }
                    }

                    let mut item_std_meta = BTreeMap::new();
                    let mut item_custom_meta = BTreeMap::new();
                    for info_node in node
                        .descendants()
                        .filter(|n| n.has_tag_name("BtVideoInfo") || n.has_tag_name("BtAudioInfo"))
                    {
                        if let Some(info_id) = info_node.attribute("DbId") {
                            if let Some((sm, cm)) = media_meta_by_info_id.get(info_id) {
                                item_std_meta.extend(sm.clone());
                                item_custom_meta.extend(cm.clone());
                            }
                        }
                    }

                    if !db_id.is_empty() {
                        media_assets.insert(
                            db_id.clone(),
                            MediaAsset {
                                id: db_id,
                                name,
                                path: full_path,
                                media_kind: kind,
                                width,
                                height,
                                frame_rate: asset_fps,
                                duration_frames,
                                source_tc_frame,
                                source_tc_string: frames_to_timecode(source_tc_frame, asset_fps),
                                audio_channels,
                                audio_sample_rate,
                                color_space: None,
                                video_codec,
                                audio_codec,
                                alpha_mode: AlphaMode::Default,
                                data_levels: DataLevels::Auto,
                                pixel_aspect_ratio: None,
                                metadata: item_std_meta,
                                custom_metadata: item_custom_meta,
                            },
                        );
                    }
                }
                _ => {}
            }
        }

        if member_name != "MediaPool/Master/MpFolder.xml" && !bin_id.is_empty() && !bin_name.is_empty() {
            bins.push(ProjectBin {
                id: bin_id,
                name: bin_name,
                parent_id,
                media_paths: bin_media_paths,
                sequence_ids: bin_seq_ids,
                custom_metadata: BTreeMap::new(),
            });
        }
    }

    let mut sequences = Vec::new();
    for (member_name, xml) in &members {
        if !member_name.starts_with("SeqContainer/") {
            continue;
        }
        let sanitized = sanitize_resolve_xml(xml);
        let Ok(doc) = Document::parse(&sanitized) else {
            continue;
        };
        let root = doc.root_element();
        let seq_id = root
            .descendants()
            .find(|n| n.has_tag_name("Sequence"))
            .and_then(|n| n.text())
            .map(|s| s.to_string())
            .unwrap_or_else(|| root.attribute("DbId").unwrap_or("seq").to_string());
        let meta = seq_meta_by_id.get(&seq_id);
        let seq_name = meta
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("Timeline {}", sequences.len() + 1));
        let seq_kind = meta.map(|m| m.kind).unwrap_or(SequenceKind::Timeline);
        let width = meta.map(|m| m.width).unwrap_or(1920);
        let height = meta.map(|m| m.height).unwrap_or(1080);
        let fps = meta.map(|m| m.fps).unwrap_or(Rational::FPS_25);
        let start_tc_frame = meta.map(|m| m.start_tc_frame).unwrap_or(0);

        let video_tracks = parse_resolve_tracks(
            root,
            "VideoTrackVec",
            TrackKind::Video,
            width,
            height,
            fps,
            start_tc_frame,
            &mp_container_by_id,
            &clip_markers_by_id,
            &clip_color_by_id,
            &mut media_assets,
        );
        let mut audio_tracks = parse_resolve_tracks(
            root,
            "AudioTrackVec",
            TrackKind::Audio,
            width,
            height,
            fps,
            start_tc_frame,
            &mp_container_by_id,
            &clip_markers_by_id,
            &clip_color_by_id,
            &mut media_assets,
        );

        let mut audio_buses = Vec::new();
        if let Some(fl) = meta.and_then(|m| m.fairlight.as_ref()) {
            audio_buses = fl.buses.clone();
            for (idx, tr) in audio_tracks.iter_mut().enumerate() {
                if let Some(&ch) = fl.track_channels.get(&idx) {
                    tr.audio_channels = ch;
                }
                if let Some(bus_id) = fl.track_bus_ids.get(&idx) {
                    tr.output_bus_id = Some(bus_id.clone());
                }
            }
        }

        let dur = video_tracks
            .iter()
            .chain(audio_tracks.iter())
            .flat_map(|t| t.clips.iter())
            .map(|c| c.end_frame)
            .max()
            .unwrap_or(0);

        let markers = seq_markers_by_id.remove(&seq_id).unwrap_or_default();

        sequences.push(Sequence {
            id: seq_id,
            name: seq_name,
            kind: seq_kind,
            width,
            height,
            frame_rate: fps,
            duration_frames: dur,
            start_tc_frame,
            mark_in: None,
            mark_out: None,
            video_tracks,
            audio_tracks,
            audio_buses,
            markers,
            metadata: BTreeMap::new(),
        });
    }

    let mut custom_metadata_schema = BTreeMap::new();
    for asset in media_assets.values() {
        for key in asset.custom_metadata.keys() {
            custom_metadata_schema
                .entry(key.clone())
                .or_insert_with(|| key.replace('.', " "));
        }
    }

    Ok(Project {
        name: project_name,
        source_format: NleFormat::DaVinciResolve,
        source_path: Some(path.to_string_lossy().into_owned()),
        color_config: ColorConfig::default(),
        scale_to_frame_size: true,
        bins,
        media_assets: media_assets.into_values().collect(),
        sequences,
        metadata: BTreeMap::new(),
        custom_metadata_schema,
    })
}

#[allow(clippy::too_many_arguments)]
fn parse_resolve_tracks(
    root: Node<'_, '_>,
    vec_tag: &str,
    kind: TrackKind,
    width: u32,
    height: u32,
    seq_fps: Rational,
    start_tc_frame: i64,
    mp_container_by_id: &HashMap<String, (ClipSourceKind, String)>,
    clip_markers_by_id: &HashMap<String, Vec<Marker>>,
    clip_color_by_id: &HashMap<String, u32>,
    media_assets: &mut BTreeMap<String, MediaAsset>,
) -> Vec<Track> {
    let Some(vec_el) = root.children().find(|n| n.has_tag_name(vec_tag)) else {
        return Vec::new();
    };
    let mut tracks = Vec::new();
    for (idx, elem) in vec_el.children().filter(|n| n.has_tag_name("Element")).enumerate() {
        let Some(track_el) = elem.children().find(|n| n.has_tag_name("Sm2TiTrack")) else {
            continue;
        };
        let flags: u32 = child_text(track_el, "Flags")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let locked = (flags & 1) != 0;
        let disabled = (flags & 2) != 0;
        let name = child_text(track_el, "UserDefinedName")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if kind == TrackKind::Video {
                    format!("V{}", idx + 1)
                } else {
                    format!("A{}", idx + 1)
                }
            });

        let clip_tag = if kind == TrackKind::Video {
            "Sm2TiVideoClip"
        } else {
            "Sm2TiAudioClip"
        };
        let mut clips = Vec::new();
        let mut transitions = Vec::new();
        if let Some(items_el) = track_el.children().find(|n| n.has_tag_name("Items")) {
            for item_el in items_el.children().filter(|n| n.has_tag_name("Element")) {
                if let Some(tnode) = item_el.children().find(|n| n.has_tag_name("Sm2TiTransition")) {
                    let raw_start = child_text(tnode, "Start")
                        .and_then(|s| s.trim().parse::<i64>().ok())
                        .unwrap_or(0);
                    let dur = child_text(tnode, "Duration")
                        .and_then(|s| s.trim().parse::<i64>().ok())
                        .unwrap_or(0);
                    if dur > 0 {
                        let start = (raw_start - start_tc_frame).max(0);
                        let ptype = child_text(tnode, "PrettyType")
                            .filter(|s| !s.is_empty())
                            .or_else(|| child_text(tnode, "Name").filter(|s| !s.is_empty()))
                            .unwrap_or_else(|| "Transition".to_string());
                        let align_code = child_text(tnode, "AlignmentType")
                            .and_then(|s| s.trim().parse::<u32>().ok())
                            .unwrap_or(2);
                        let alignment = match align_code {
                            1 => "start".to_string(),
                            3 => "end".to_string(),
                            _ => "center".to_string(),
                        };
                        transitions.push(Transition {
                            name: ptype.clone(),
                            effect_id: ptype,
                            start_frame: start,
                            end_frame: start + dur,
                            alignment,
                        });
                    }
                    continue;
                }

                let Some(cnode) = item_el.children().find(|n| n.has_tag_name(clip_tag)) else {
                    continue;
                };
                let raw_start = child_text(cnode, "Start")
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .unwrap_or(0);
                let start = (raw_start - start_tc_frame).max(0);
                let dur = child_text(cnode, "Duration")
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .unwrap_or(0);
                let in_frame = child_text(cnode, "In")
                    .and_then(|s| s.split('|').next().and_then(|n| n.trim().parse::<i64>().ok()))
                    .unwrap_or(0);
                let clip_fps = child_text(cnode, "MediaFrameRate")
                    .and_then(|h| decode_resolve_fps(&h))
                    .unwrap_or(seq_fps);
                let media_path = child_text(cnode, "MediaFilePath").unwrap_or_default();
                let media_id = child_text(cnode, "MediaRef").unwrap_or_else(|| media_path.clone());
                let cname = child_text(cnode, "Name").unwrap_or_else(|| "Clip".into());
                let audio_channel = child_text(cnode, "VirtualAudioTrackBA")
                    .and_then(|h| decode_virtual_audio_track(&h))
                    .map(|(_, ch)| ch)
                    .or_else(|| {
                        child_text(cnode, "MediaTrackIdx")
                            .and_then(|s| s.trim().parse::<u32>().ok())
                            .filter(|&c| c > 0)
                    })
                    .unwrap_or(1);

                let clip_db_id = cnode.attribute("DbId").unwrap_or("").to_string();
                let (source_kind, resolved_media_id) = match mp_container_by_id.get(&media_id) {
                    Some((sk, inner_seq_id)) => (*sk, inner_seq_id.clone()),
                    None => (ClipSourceKind::Media, media_id.clone()),
                };

                let (speed_percent, reversed, freeze_hold_sec, speed_keyframes) =
                    child_text(cnode, "MediaTimemapBA")
                        .and_then(|h| decode_media_timemap_full(&h))
                        .unwrap_or((100.0, false, None, Vec::new()));
                let effective_in_frame = if let Some(hold_sec) = freeze_hold_sec {
                    (hold_sec * seq_fps.as_f64()).round() as i64
                } else {
                    in_frame
                };

                let (active_angle, link_group_id) = child_text(cnode, "FieldsBlob")
                    .map(|h| decode_clip_fields_blob(&h, &clip_db_id))
                    .unwrap_or((None, None));

                let (mut motion, opacity, retime_interpolation) =
                    child_text(cnode, "EffectFiltersBA")
                        .map(|h| decode_effect_filters(&h))
                        .unwrap_or_else(|| {
                            (MotionTransform::default(), 100.0, RetimeInterpolation::Default)
                        });

                // Backfill MediaAsset full path when Resolve 21.2 zstd-compressed BtVideoInfo/Clip
                // could not be unpacked directly from MpFolder.xml.
                let mut clip_w = width;
                let mut clip_h = height;
                if let Some(asset) = media_assets.get_mut(&media_id) {
                    if !media_path.is_empty()
                        && (asset.path.is_empty() || asset.path == asset.name)
                    {
                        asset.path = media_path.clone();
                    }
                    if asset.width > 0 && asset.height > 0 {
                        clip_w = asset.width;
                        clip_h = asset.height;
                    }
                }

                // Resolve's ZoomX/ZoomY are relative to the fitted frame ("Scale entire image to fit"),
                // whereas our canonical MotionTransform.scale (and Premiere's Motion Scale) is relative
                // to the clip's native source pixel dimensions. Convert fitted zoom -> native pixel scale.
                if kind == TrackKind::Video
                    && source_kind == ClipSourceKind::Media
                    && clip_w > 0
                    && clip_h > 0
                    && width > 0
                    && height > 0
                    && (clip_w != width || clip_h != height)
                {
                    let fit = (width as f64 / clip_w as f64).min(height as f64 / clip_h as f64);
                    if fit.is_finite() && fit > 1e-6 {
                        motion.scale *= fit;
                        if let Some(sy) = motion.scale_y.as_mut() {
                            *sy *= fit;
                        }
                    }
                }

                let label_preset_index = clip_color_by_id
                    .get(&clip_db_id)
                    .and_then(|&c| resolve_color_to_premiere_label(c));
                let label_name =
                    label_preset_index.map(|idx| format!("BE.Prefs.LabelColors.{idx}"));
                let clip_markers = clip_markers_by_id
                    .get(&clip_db_id)
                    .cloned()
                    .unwrap_or_default();

                let mut clip_effects = Vec::new();
                let mut lumetri_effects = Vec::new();
                if kind == TrackKind::Video {
                    if let Some(vt) = cnode.children().find(|n| n.has_tag_name("pLmVerTable")) {
                        let active_id = vt
                            .descendants()
                            .find(|n| n.has_tag_name("pActive"))
                            .and_then(|n| n.text())
                            .map(|s| s.trim().to_string());
                        let versions: Vec<_> = vt
                            .descendants()
                            .filter(|n| n.tag_name().name().ends_with("LmVersion"))
                            .collect();
                        let active_body = active_id
                            .as_deref()
                            .and_then(|aid| {
                                versions
                                    .iter()
                                    .find(|v| v.attribute("DbId") == Some(aid))
                                    .and_then(|v| {
                                        v.children()
                                            .find(|c| c.has_tag_name("Body"))
                                            .and_then(|b| b.text())
                                    })
                            })
                            .or_else(|| {
                                vt.descendants()
                                    .filter(|n| n.has_tag_name("Body"))
                                    .filter_map(|b| b.text())
                                    .next()
                            });

                        if let Some(body) = active_body {
                            lumetri_effects = parse_resolve_grade_nodes(body);
                        }

                        let has_stab = vt
                            .descendants()
                            .filter(|n| n.has_tag_name("Body"))
                            .filter_map(|b| b.text())
                            .any(has_resolve_stabilization);
                        if has_stab {
                            clip_effects.push(EffectInstance {
                                id: "resolve-stab".to_string(),
                                match_name: "resolve.inspector.stabilization".to_string(),
                                display_name: "Stabilization".to_string(),
                                category: EffectCategory::Stabilization,
                                media_kind: TrackKind::Video,
                                enabled: true,
                                is_intrinsic: false,
                                is_animated: false,
                                parameters: Vec::new(),
                            });
                        }
                    }
                }

                clips.push(Clip {
                    id: clip_db_id,
                    name: cname,
                    media_id: resolved_media_id,
                    media_path,
                    media_kind: if kind == TrackKind::Video {
                        MediaKind::Video
                    } else {
                        MediaKind::Audio
                    },
                    source_kind,
                    active_angle,
                    start_frame: start,
                    end_frame: start + dur,
                    source_in_frame: effective_in_frame,
                    source_out_frame: effective_in_frame + dur,
                    source_in_ticks: None,
                    source_out_ticks: None,
                    source_fps: clip_fps,
                    source_width: clip_w,
                    source_height: clip_h,
                    speed_percent,
                    reversed,
                    speed_keyframes,
                    retime_interpolation,
                    enabled: true,
                    audio_channel,
                    link_group_id,
                    label_name,
                    label_preset_index,
                    motion,
                    opacity,
                    lumetri_effects,
                    effects: clip_effects,
                    markers: clip_markers,
                });
            }
        }

        let audio_channels = if kind == TrackKind::Audio {
            match child_text(track_el, "SubType")
                .and_then(|s| s.trim().parse::<u32>().ok())
            {
                Some(1) => 1,
                Some(0) => 2,
                _ => 2,
            }
        } else {
            0
        };

        tracks.push(Track {
            index: idx + 1,
            kind,
            name,
            enabled: !disabled,
            locked,
            muted: disabled,
            solo: false,
            audio_channels,
            output_bus_id: None,
            clips,
            transitions,
            markers: Vec::new(),
        });
    }
    tracks
}

fn sanitize_resolve_xml(xml: &str) -> String {
    xml.replace("ListMgt::", "ListMgt__")
        .replace("Gallery::", "Gallery__")
}

fn extract_simple_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = xml.find(&open)? + open.len();
    let e = xml[s..].find(&close)?;
    Some(xml[s..s + e].to_string())
}

fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(|s| s.to_string())
}

fn decode_resolve_fps(hex: &str) -> Option<Rational> {
    let bytes = hex_decode(hex).ok()?;
    if bytes.len() < 8 {
        return None;
    }
    let fps = f64::from_le_bytes(bytes[0..8].try_into().ok()?);
    if fps.is_finite() && fps > 0.0 {
        Some(Rational::from_fps_approx(fps))
    } else {
        None
    }
}

fn decode_resolve_resolution(hex: &str) -> Option<(u32, u32)> {
    let bytes = hex_decode(hex).ok()?;
    if bytes.len() < 16 {
        return None;
    }
    let w = u64::from_be_bytes(bytes[0..8].try_into().ok()?) as u32;
    let h = u64::from_be_bytes(bytes[8..16].try_into().ok()?) as u32;
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

fn decode_resolve_media_extents(hex: &str) -> Option<(f64, f64)> {
    let bytes = hex_decode(hex).ok()?;
    if bytes.len() < 16 {
        return None;
    }
    let start_sec = f64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let dur_sec = f64::from_le_bytes(bytes[8..16].try_into().ok()?);
    if start_sec.is_finite() && dur_sec.is_finite() {
        Some((start_sec, dur_sec))
    } else {
        None
    }
}

fn decode_bt_geometry(hex: &str) -> Option<(u32, u32)> {
    let raw = hex_decode(hex).ok()?;
    let fields = decode_keyed_fields(&raw).ok()?;
    for (k, _, v) in fields {
        if k == "Resolution" && v.len() >= 16 {
            let w = u64::from_be_bytes(v[0..8].try_into().ok()?) as u32;
            let h = u64::from_be_bytes(v[8..16].try_into().ok()?) as u32;
            if w > 0 && h > 0 {
                return Some((w, h));
            }
        }
    }
    None
}

fn decode_bt_time(hex: &str) -> (Option<i64>, Option<i64>, Option<Rational>) {
    let mut start_frame = None;
    let mut num_frames = None;
    let mut fps = None;
    let Ok(raw) = hex_decode(hex) else {
        return (None, None, None);
    };
    let Ok(fields) = decode_keyed_fields(&raw) else {
        return (None, None, None);
    };
    for (k, _, v) in fields {
        match k.as_str() {
            "StartFrame" if v.len() >= 4 => {
                start_frame = Some(i32::from_be_bytes(v[0..4].try_into().unwrap()) as i64);
            }
            "NumFrames" if v.len() >= 4 => {
                num_frames = Some(i32::from_be_bytes(v[0..4].try_into().unwrap()) as i64);
            }
            "FrameRate" if v.len() >= 8 => {
                let f = f64::from_le_bytes(v[0..8].try_into().unwrap());
                if f.is_finite() && f > 0.0 {
                    fps = Some(Rational::from_fps_approx(f));
                }
            }
            _ => {}
        }
    }
    (start_frame, num_frames, fps)
}

fn decode_bt_clip_codec(hex: &str) -> Option<String> {
    let raw = decode_wrapper_hex(hex).ok()?;
    let fields = decode_protobuf(&raw).ok()?;
    for f in fields {
        if f.number == 5 {
            if let ProtoValue::Bytes(b) = f.value {
                let s = String::from_utf8_lossy(&b).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn decode_bt_audio_tracks(hex: &str) -> (Option<u32>, Option<u32>, Option<String>) {
    let Ok(raw) = hex_decode(hex) else {
        return (None, None, None);
    };
    let Ok(phys) = decode_keyed_fields(&raw) else {
        return (None, None, None);
    };
    let mut channels = None;
    let mut sample_rate = None;
    let mut codec_name = None;
    for (_, _, track_raw) in phys {
        if let Ok(tfields) = decode_keyed_fields(&track_raw) {
            for (k, kind, v) in tfields {
                match k.as_str() {
                    "NumChannels" if v.len() >= 4 => {
                        let c = u32::from_be_bytes(v[0..4].try_into().unwrap());
                        if c > 0 {
                            channels = Some(c);
                        }
                    }
                    "SampleRate" if v.len() >= 4 => {
                        let r = u32::from_be_bytes(v[0..4].try_into().unwrap());
                        if r > 0 {
                            sample_rate = Some(r);
                        }
                    }
                    "CodecName" if kind == 10 => {
                        if let Ok(s) = decode_utf16be(&v) {
                            let s = s.trim().to_string();
                            if !s.is_empty() {
                                codec_name = Some(s);
                            }
                        }
                    }
                    _ => {}
                }
            }
            break;
        }
    }
    (channels, sample_rate, codec_name)
}

fn decode_bt_clip_path(hex: &str) -> Option<String> {
    let raw = decode_wrapper_hex(hex).ok()?;
    let fields = decode_protobuf(&raw).ok()?;
    let mut dir = String::new();
    let mut file = String::new();
    for f in fields {
        if let ProtoValue::Bytes(b) = f.value {
            match f.number {
                1 => dir = String::from_utf8_lossy(&b).into_owned(),
                2 => file = String::from_utf8_lossy(&b).into_owned(),
                _ => {}
            }
        }
    }
    if file.is_empty() {
        None
    } else if dir.is_empty() {
        Some(file)
    } else {
        Some(format!("{}/{}", dir.trim_end_matches(['/', '\\']), file))
    }
}

/// Decode Resolve 21.2 clip `<FieldsBlob>` (supports `0x80` and `0x81` zstd) for:
/// - `active_angle` (UTF-8 camera/angle name at `f1.f3`, e.g. `"Camera 3"`)
/// - `link_group_id` (canonical group ID computed from `f1.f1.f1` keyed-dict of linked sibling DbIds)
fn decode_clip_fields_blob(hex: &str, self_db_id: &str) -> (Option<String>, Option<String>) {
    let Ok(raw) = decode_wrapper_hex(hex) else {
        return (None, None);
    };
    let Ok(outer) = decode_protobuf(&raw) else {
        return (None, None);
    };
    let mut active_angle: Option<String> = None;
    let mut linked_ids: Vec<String> = Vec::new();

    for f in outer {
        if f.number == 1 {
            if let ProtoValue::Bytes(f1_bytes) = f.value {
                if let Ok(inner) = decode_protobuf(&f1_bytes) {
                    for sf in inner {
                        match (sf.number, sf.value) {
                            (1, ProtoValue::Bytes(f11_bytes)) => {
                                if let Ok(sub) = decode_protobuf(&f11_bytes) {
                                    for ssf in sub {
                                        if ssf.number == 1 {
                                            if let ProtoValue::Bytes(dict_bytes) = ssf.value {
                                                if let Ok(entries) = decode_keyed_fields(&dict_bytes) {
                                                    for (_, kind, val) in entries {
                                                        if kind == 10 {
                                                            if let Ok(id) = decode_utf16be(&val) {
                                                                let id = id.trim().to_string();
                                                                if !id.is_empty() {
                                                                    linked_ids.push(id);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            (3, ProtoValue::Bytes(angle_bytes)) => {
                                if let Ok(s) = String::from_utf8(angle_bytes) {
                                    let trimmed = s.trim();
                                    if !trimmed.is_empty() {
                                        active_angle = Some(trimmed.to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let link_group_id = if !linked_ids.is_empty() {
        if !self_db_id.is_empty() {
            linked_ids.push(self_db_id.to_string());
        }
        linked_ids.sort();
        linked_ids.dedup();
        linked_ids.first().map(|min_id| format!("resolve-link-{min_id}"))
    } else {
        None
    };

    (active_angle, link_group_id)
}

/// Decode Resolve `<EffectFiltersBA>` (`0x80` or `0x81` zstd) into `(MotionTransform, opacity, RetimeInterpolation)`.
fn decode_effect_filters(hex: &str) -> (MotionTransform, f64, RetimeInterpolation) {
    let mut motion = MotionTransform::default();
    let mut opacity = 100.0_f64;
    let mut retime_interp = RetimeInterpolation::Default;

    let Ok(raw) = decode_wrapper_hex(hex) else {
        return (motion, opacity, retime_interp);
    };
    let Ok(outer) = decode_protobuf(&raw) else {
        return (motion, opacity, retime_interp);
    };

    let mut zoom_x: Option<f64> = None;
    let mut zoom_y: Option<f64> = None;

    for f in outer {
        if f.number != 1 {
            continue;
        }
        let ProtoValue::Bytes(filter_bytes) = f.value else {
            continue;
        };
        let Ok(filter_fields) = decode_protobuf(&filter_bytes) else {
            continue;
        };
        let filter_id = filter_fields.iter().find_map(|ff| match (ff.number, &ff.value) {
            (1, ProtoValue::Varint(v)) => Some(*v),
            _ => None,
        });
        let Some(filter_id) = filter_id else {
            continue;
        };

        for ff in &filter_fields {
            if ff.number != 9 {
                continue;
            }
            let ProtoValue::Bytes(param_bytes) = &ff.value else {
                continue;
            };
            if param_bytes.is_empty() {
                continue;
            }
            let Ok(pfields) = decode_protobuf(param_bytes) else {
                continue;
            };
            let pid = pfields.iter().find_map(|pf| match (pf.number, &pf.value) {
                (1, ProtoValue::Varint(v)) => Some(*v),
                _ => None,
            });
            let Some(pid) = pid else {
                continue;
            };
            let curve_bytes = pfields.iter().find_map(|pf| match (pf.number, &pf.value) {
                (3, ProtoValue::Bytes(b)) => Some(b.as_slice()),
                _ => None,
            });
            let Some(curve_bytes) = curve_bytes else {
                continue;
            };
            let Ok(curve_fields) = decode_protobuf(curve_bytes) else {
                continue;
            };
            let kf_count = curve_fields.iter().filter(|cf| cf.number == 1).count();
            if kf_count > 1 && filter_id == 4 {
                motion.has_keyframes = true;
            }

            // Handle enum-valued parameters (e.g. Filter 44 RetimeProcess param 91)
            if filter_id == 44 && pid == 91 {
                let first_enum = curve_fields.iter().find_map(|cf| {
                    if cf.number != 1 {
                        return None;
                    }
                    let ProtoValue::Bytes(kf_bytes) = &cf.value else {
                        return None;
                    };
                    let kf_sub = decode_protobuf(kf_bytes).ok()?;
                    kf_sub.iter().find_map(|k| match (k.number, &k.value) {
                        (4, ProtoValue::Varint(v)) => Some(*v),
                        _ => None,
                    })
                });
                if let Some(ev) = first_enum {
                    retime_interp = match ev {
                        1 => RetimeInterpolation::FrameSampling,
                        2 => RetimeInterpolation::FrameBlending,
                        3 => RetimeInterpolation::OpticalFlow,
                        _ => RetimeInterpolation::Default,
                    };
                }
                continue;
            }

            let first_val = curve_fields.iter().find_map(|cf| {
                if cf.number != 1 {
                    return None;
                }
                let ProtoValue::Bytes(kf_bytes) = &cf.value else {
                    return None;
                };
                let kf_sub = decode_protobuf(kf_bytes).ok()?;
                kf_sub.iter().find_map(|k| match (k.number, &k.value) {
                    (2, ProtoValue::Fixed64(arr)) => Some(f64::from_le_bytes(*arr)),
                    _ => None,
                })
            });
            let Some(val) = first_val.filter(|v| v.is_finite()) else {
                continue;
            };
            match (filter_id, pid) {
                (4, 42) => zoom_x = Some(val),
                (4, 43) => zoom_y = Some(val),
                (4, 40) => motion.position_x = val,
                (4, 41) => motion.position_y = val,
                (4, 47) => motion.rotation = -val,
                (6, 54) => motion.crop_left = val * 100.0,
                (6, 55) => motion.crop_right = val * 100.0,
                (6, 56) => motion.crop_top = val * 100.0,
                (6, 57) => motion.crop_bottom = val * 100.0,
                (2, 1) => opacity = val,
                _ => {}
            }
        }
    }

    if let Some(zx) = zoom_x {
        motion.scale = zx * 100.0;
        if let Some(zy) = zoom_y {
            if (zx - zy).abs() > 1e-4 {
                motion.scale_y = Some(zy * 100.0);
            }
        }
    }

    (motion, opacity, retime_interp)
}
