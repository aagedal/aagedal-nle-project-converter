use crate::binary::{decode_protobuf, decode_wrapper_hex, hex_decode, ProtoValue};
use nle_core::{
    Clip, ColorConfig, MediaAsset, MediaKind, MotionTransform, NleError, NleFormat, NleResult,
    Project, ProjectBin, ProjectReader, Rational, Sequence, Track, TrackKind,
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

    struct SeqMeta {
        name: String,
        width: u32,
        height: u32,
        fps: Rational,
    }

    let mut seq_meta_by_id: HashMap<String, SeqMeta> = HashMap::new();
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
                "Sm2MpTimelineClip" => {
                    let tl_name = child_text(node, "Name").unwrap_or_else(|| "Timeline".to_string());
                    if let Some(seq_el) = node.descendants().find(|n| n.has_tag_name("Sm2Sequence")) {
                        let seq_id = seq_el.attribute("DbId").unwrap_or("").to_string();
                        let fps = child_text(seq_el, "FrameRate")
                            .and_then(|h| decode_resolve_fps(&h))
                            .unwrap_or(Rational::FPS_25);
                        let (width, height) = child_text(seq_el, "Resolution")
                            .and_then(|h| decode_resolve_resolution(&h))
                            .unwrap_or((1920, 1080));
                        if !seq_id.is_empty() {
                            bin_seq_ids.push(seq_id.clone());
                            seq_meta_by_id.insert(
                                seq_id,
                                SeqMeta {
                                    name: tl_name,
                                    width,
                                    height,
                                    fps,
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
                    if !db_id.is_empty() {
                        media_assets.insert(
                            db_id.clone(),
                            MediaAsset {
                                id: db_id,
                                name,
                                path: full_path,
                                media_kind: kind,
                                width: 1920,
                                height: 1080,
                                frame_rate: Rational::FPS_25,
                                duration_frames: 0,
                                source_tc_frame: 0,
                                source_tc_string: "00:00:00:00".into(),
                                audio_channels: 2,
                                audio_sample_rate: 48_000,
                                color_space: None,
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
        let width = meta.map(|m| m.width).unwrap_or(1920);
        let height = meta.map(|m| m.height).unwrap_or(1080);
        let fps = meta.map(|m| m.fps).unwrap_or(Rational::FPS_25);

        let video_tracks = parse_resolve_tracks(root, "VideoTrackVec", TrackKind::Video, width, height, fps);
        let audio_tracks = parse_resolve_tracks(root, "AudioTrackVec", TrackKind::Audio, width, height, fps);

        let dur = video_tracks
            .iter()
            .chain(audio_tracks.iter())
            .flat_map(|t| t.clips.iter())
            .map(|c| c.end_frame)
            .max()
            .unwrap_or(0);

        sequences.push(Sequence {
            id: seq_id,
            name: seq_name,
            width,
            height,
            frame_rate: fps,
            duration_frames: dur,
            mark_in: None,
            mark_out: None,
            video_tracks,
            audio_tracks,
            markers: Vec::new(),
            metadata: BTreeMap::new(),
        });
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
    })
}

fn parse_resolve_tracks(
    root: Node<'_, '_>,
    vec_tag: &str,
    kind: TrackKind,
    width: u32,
    height: u32,
    seq_fps: Rational,
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
        if let Some(items_el) = track_el.children().find(|n| n.has_tag_name("Items")) {
            for item_el in items_el.children().filter(|n| n.has_tag_name("Element")) {
                let Some(cnode) = item_el.children().find(|n| n.has_tag_name(clip_tag)) else {
                    continue;
                };
                let start = child_text(cnode, "Start")
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .unwrap_or(0);
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

                clips.push(Clip {
                    id: cnode.attribute("DbId").unwrap_or("").to_string(),
                    name: cname,
                    media_id,
                    media_path,
                    media_kind: if kind == TrackKind::Video {
                        MediaKind::Video
                    } else {
                        MediaKind::Audio
                    },
                    start_frame: start,
                    end_frame: start + dur,
                    source_in_frame: in_frame,
                    source_out_frame: in_frame + dur,
                    source_in_ticks: None,
                    source_out_ticks: None,
                    source_fps: clip_fps,
                    source_width: width,
                    source_height: height,
                    speed_percent: 100.0,
                    reversed: false,
                    enabled: true,
                    audio_channel: 1,
                    link_group_id: None,
                    label_name: None,
                    label_preset_index: None,
                    motion: MotionTransform::default(),
                    opacity: 100.0,
                    lumetri_effects: Vec::new(),
                    effects: Vec::new(),
                    markers: Vec::new(),
                });
            }
        }

        tracks.push(Track {
            index: idx + 1,
            kind,
            name,
            enabled: !disabled,
            locked,
            muted: disabled,
            solo: false,
            clips,
            transitions: Vec::new(),
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
