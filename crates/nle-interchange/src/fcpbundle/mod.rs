pub mod bplist;
pub mod sqlite;

use crate::fcpxml::time::FcpTime;
use bplist::{parse_bplist_nskeyed, PlistValue};
use nle_core::{
    frames_to_timecode, AlphaMode, BlendMode, Clip, ClipSourceKind, ColorConfig, DataLevels,
    EffectCategory, EffectInstance, EffectParameter, Marker, MarkerScope, MediaAsset, MediaKind,
    MotionTransform, NleError, NleFormat, NleResult, Project, ProjectBin, ProjectReader, Rational,
    RetimeInterpolation, Sequence, SequenceKind, SpeedKeyframe, Track, TrackKind, Transition,
};
use roxmltree::{Document, ParsingOptions};
use sqlite::{SqlValue, SqliteDb};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct FcpBundleReader;

impl ProjectReader for FcpBundleReader {
    fn format(&self) -> NleFormat {
        NleFormat::FinalCutPro
    }

    fn read_file(&self, path: &Path) -> NleResult<Project> {
        read_fcpbundle_file(path)
    }
}

/// Read a native Final Cut Pro library package (`.fcpbundle`) directly in pure Rust
/// by parsing its CoreData SQLite `CurrentVersion.fcpevent` databases and embedded
/// `NSKeyedArchiver` binary plists — without needing Final Cut Pro or FCPXML export.
pub fn read_fcpbundle_file(path: &Path) -> NleResult<Project> {
    if !path.is_dir() {
        return Err(NleError::InvalidFormat(format!(
            "Expected .fcpbundle package directory at '{}'",
            path.display()
        )));
    }

    let library_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Final Cut Pro Library")
        .to_string();

    let mut assets_by_id: HashMap<String, BundleAsset> = HashMap::new();
    let mut asset_order: Vec<String> = Vec::new();
    let mut bins: Vec<ProjectBin> = Vec::new();
    let mut sequences: Vec<Sequence> = Vec::new();
    let mut primary_color_space: Option<String> = None;

    // Discover Event folders inside the .fcpbundle (directories containing `CurrentVersion.fcpevent`,
    // skipping internal `__Trash`, `__Temp`, `.fcpcache`, `Motion Templates.localized`, etc.)
    let mut event_dirs: Vec<PathBuf> = fs::read_dir(path)?
        .filter_map(|e| e.ok().map(|de| de.path()))
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|n| n.starts_with('.') || n.starts_with("__") || n.ends_with(".localized"))
                    .unwrap_or(true)
                && p.join("CurrentVersion.fcpevent").is_file()
        })
        .collect();
    event_dirs.sort();

    if event_dirs.is_empty() {
        return Err(NleError::InvalidFormat(format!(
            "No Final Cut Pro events (CurrentVersion.fcpevent) found in '{}'",
            path.display()
        )));
    }

    for event_dir in &event_dirs {
        let event_name = event_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Event")
            .to_string();

        let event_db_path = event_dir.join("CurrentVersion.fcpevent");
        let event_assets = read_event_assets(&event_db_path, event_dir)?;
        let mut bin_media_paths = Vec::new();
        for a in event_assets {
            if !a.path.is_empty() && !bin_media_paths.contains(&a.path) {
                bin_media_paths.push(a.path.clone());
            }
            if !assets_by_id.contains_key(&a.id) {
                asset_order.push(a.id.clone());
            }
            assets_by_id.insert(a.id.clone(), a);
        }

        // Discover Project subdirectories inside this Event
        let mut project_dirs: Vec<PathBuf> = fs::read_dir(event_dir)?
            .filter_map(|e| e.ok().map(|de| de.path()))
            .filter(|p| {
                p.is_dir()
                    && !p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .map(|n| {
                            n.starts_with('.')
                                || n.starts_with("__")
                                || matches!(
                                    n,
                                    "Original Media"
                                        | "Transcoded Media"
                                        | "Render Files"
                                        | "Analysis Files"
                                        | "Shared Items"
                                )
                        })
                        .unwrap_or(true)
                    && p.join("CurrentVersion.fcpevent").is_file()
            })
            .collect();
        project_dirs.sort();

        let mut bin_seq_ids = Vec::new();
        for proj_dir in &project_dirs {
            let proj_fallback_name = proj_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled Project");
            let proj_db_path = proj_dir.join("CurrentVersion.fcpevent");
            if let Ok(Some((seq, color_space))) =
                read_project_sequence(&proj_db_path, proj_fallback_name, &assets_by_id)
            {
                if primary_color_space.is_none() {
                    primary_color_space = color_space;
                }
                bin_seq_ids.push(seq.id.clone());
                sequences.push(seq);
            }
        }

        bins.push(ProjectBin {
            id: Uuid::new_v4().to_string(),
            name: event_name,
            parent_id: None,
            media_paths: bin_media_paths,
            sequence_ids: bin_seq_ids,
            custom_metadata: BTreeMap::new(),
        });
    }

    let media_assets: Vec<MediaAsset> = asset_order
        .iter()
        .filter_map(|id| assets_by_id.get(id))
        .map(BundleAsset::to_media_asset)
        .collect();

    let color_config = ColorConfig {
        working_space: primary_color_space.clone(),
        output_space: primary_color_space,
    };

    let mut metadata = BTreeMap::new();
    metadata.insert("fcp_source".to_string(), "fcpbundle".to_string());

    Ok(Project {
        name: library_name,
        source_format: NleFormat::FinalCutPro,
        source_path: Some(path.to_string_lossy().into_owned()),
        color_config,
        scale_to_frame_size: true,
        bins,
        media_assets,
        sequences,
        metadata,
        custom_metadata_schema: BTreeMap::new(),
    })
}

// ── Event Graph Loader (FFAsset + FFMediaRep) ───────────────────────────

#[derive(Debug, Clone)]
struct BundleAsset {
    id: String,
    name: String,
    path: String,
    media_kind: MediaKind,
    has_video: bool,
    has_audio: bool,
    width: u32,
    height: u32,
    fps: Rational,
    start: FcpTime,
    duration: FcpTime,
    audio_channels: u32,
    audio_sample_rate: u32,
    color_space: Option<String>,
}

impl BundleAsset {
    fn to_media_asset(&self) -> MediaAsset {
        let source_tc_frame = self.start.to_frames(self.fps).max(0);
        let duration_frames = if self.media_kind == MediaKind::Image {
            0
        } else {
            self.duration.to_frames(self.fps).max(0)
        };
        MediaAsset {
            id: self.id.clone(),
            name: self.name.clone(),
            path: self.path.clone(),
            media_kind: self.media_kind,
            width: if self.has_video { self.width } else { 0 },
            height: if self.has_video { self.height } else { 0 },
            frame_rate: self.fps,
            duration_frames,
            source_tc_frame,
            source_tc_string: frames_to_timecode(source_tc_frame, self.fps),
            audio_channels: self.audio_channels,
            audio_sample_rate: self.audio_sample_rate,
            color_space: self.color_space.clone(),
            video_codec: None,
            audio_codec: None,
            alpha_mode: AlphaMode::Default,
            data_levels: DataLevels::Auto,
            pixel_aspect_ratio: None,
            metadata: BTreeMap::new(),
            custom_metadata: BTreeMap::new(),
        }
    }
}

struct EventGraph {
    nodes: HashMap<i64, GraphNode>,
    children: HashMap<i64, Vec<i64>>,
}

struct GraphNode {
    pk: i64,
    ztype: String,
    zname: String,
    zident: String,
    plist: Option<PlistValue>,
}

impl EventGraph {
    fn load(db_path: &Path) -> NleResult<Self> {
        let db = SqliteDb::open(db_path)?;

        // ZCOLLECTIONMD: Z_PK(0), Z_ENT(1), Z_OPT(2), ZCOLLECTION(3), ZIDENTIFIER(4), ZDICTIONARYDATA(5)
        let mut md_by_pk: HashMap<i64, Vec<u8>> = HashMap::new();
        for (rowid, cols) in db.read_table("ZCOLLECTIONMD")? {
            let pk = cols.first().and_then(SqlValue::as_i64).unwrap_or(rowid);
            if let Some(blob) = cols.get(5).and_then(SqlValue::as_bytes) {
                md_by_pk.insert(pk, blob.to_vec());
            }
        }

        // ZCOLLECTION: Z_PK(0), Z_ENT(1), Z_OPT(2), ZFLAGS(3), ZCATALOG(4), ZMETADATA(5), ZIDENTIFIER(6), ZNAME(7), ZTYPE(8)
        let mut nodes = HashMap::new();
        for (rowid, cols) in db.read_table("ZCOLLECTION")? {
            let pk = cols.first().and_then(SqlValue::as_i64).unwrap_or(rowid);
            let md_pk = cols.get(5).and_then(SqlValue::as_i64);
            let zident = cols
                .get(6)
                .and_then(SqlValue::as_str)
                .unwrap_or("")
                .to_string();
            let zname = cols
                .get(7)
                .and_then(SqlValue::as_str)
                .unwrap_or("")
                .to_string();
            let ztype = cols
                .get(8)
                .and_then(SqlValue::as_str)
                .unwrap_or("")
                .to_string();
            let plist = md_pk
                .and_then(|mpk| md_by_pk.get(&mpk))
                .and_then(|blob| parse_bplist_nskeyed(blob));

            nodes.insert(
                pk,
                GraphNode {
                    pk,
                    ztype,
                    zname,
                    zident,
                    plist,
                },
            );
        }

        // Z_3CHILDCOLLECTIONS: Z_3PARENTCOLLECTIONS(0), Z_3CHILDCOLLECTIONS(1)
        let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
        for (_rowid, cols) in db.read_table("Z_3CHILDCOLLECTIONS")? {
            if let (Some(p), Some(c)) = (
                cols.first().and_then(SqlValue::as_i64),
                cols.get(1).and_then(SqlValue::as_i64),
            ) {
                children.entry(p).or_default().push(c);
            }
        }

        Ok(Self { nodes, children })
    }

    fn named_child_items(&self, parent_pk: i64, relation_name: &str) -> Vec<i64> {
        let Some(direct) = self.children.get(&parent_pk) else {
            return Vec::new();
        };
        for &cid in direct {
            if let Some(node) = self.nodes.get(&cid)
                && node.zname == relation_name {
                    let raw_children = self.children.get(&cid).cloned().unwrap_or_default();
                    // If this NSArray carries an explicit `$order` UUID array, sort children by it!
                    if let Some(order_ids) = node
                        .plist
                        .as_ref()
                        .and_then(|p| p.get("$order"))
                        .and_then(PlistValue::as_array)
                    {
                        let mut by_ident: HashMap<&str, i64> = HashMap::new();
                        for &item_pk in &raw_children {
                            if let Some(item) = self.nodes.get(&item_pk)
                                && !item.zident.is_empty() {
                                    by_ident.insert(item.zident.as_str(), item_pk);
                                }
                        }
                        let mut ordered = Vec::with_capacity(raw_children.len());
                        let mut used = HashSet::new();
                        for oid in order_ids {
                            if let Some(s) = oid.as_str()
                                && let Some(&pk) = by_ident.get(s)
                                    && used.insert(pk) {
                                        ordered.push(pk);
                                    }
                        }
                        for pk in raw_children {
                            if used.insert(pk) {
                                ordered.push(pk);
                            }
                        }
                        return ordered;
                    }
                    return raw_children;
                }
        }
        Vec::new()
    }
}

fn read_event_assets(event_db_path: &Path, event_dir: &Path) -> NleResult<Vec<BundleAsset>> {
    let graph = EventGraph::load(event_db_path)?;

    // Map md5Seed -> resolved path from FFMediaRep (prefer originalMediaRep)
    let mut rep_paths: HashMap<String, String> = HashMap::new();
    for node in graph.nodes.values() {
        if node.ztype != "FFMediaRep" {
            continue;
        }
        let Some(pl) = &node.plist else {
            continue;
        };
        let Some(md5) = pl.get("md5Seed").and_then(PlistValue::as_str) else {
            continue;
        };
        let rep_type = pl
            .get("repType")
            .and_then(PlistValue::as_str)
            .unwrap_or("");
        let is_original = rep_type == "originalMediaRep" || rep_type.is_empty();

        if let Some(rel) = pl
            .get("projectRelativePath")
            .and_then(PlistValue::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            let full = event_dir.join(rel);
            let full_str = full.to_string_lossy().into_owned();
            if is_original || !rep_paths.contains_key(md5) {
                rep_paths.insert(md5.to_string(), full_str);
            }
        }
    }

    // Collect videoProps from FFAnchoredCollection nodes in the event (for assets whose videoFormatName is empty, e.g. vertical iPhone clips!)
    let mut props_by_name: HashMap<String, (u32, u32, Rational, Option<String>)> = HashMap::new();
    for node in graph.nodes.values() {
        if node.ztype == "FFAnchoredCollection"
            && let Some(pl) = &node.plist
                && let (Some(name), Some(vp)) = (
                    pl.get("displayName").and_then(PlistValue::as_str),
                    pl.get("videoProps"),
                )
                    && let Some(parsed) = parse_video_props(vp) {
                        props_by_name.insert(name.to_string(), parsed);
                    }
    }

    let mut out = Vec::new();
    let mut asset_pks: Vec<i64> = graph
        .nodes
        .values()
        .filter(|n| n.ztype == "FFAsset")
        .map(|n| n.pk)
        .collect();
    asset_pks.sort();

    for pk in asset_pks {
        let node = &graph.nodes[&pk];
        let Some(pl) = &node.plist else {
            continue;
        };
        let id = pl
            .get("mediaIdentifier")
            .or_else(|| pl.get("md5String"))
            .and_then(PlistValue::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&node.zident)
            .to_string();
        if id.is_empty() {
            continue;
        }
        let name = pl
            .get("displayName")
            .and_then(PlistValue::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&id)
            .to_string();

        let path = rep_paths
            .get(&id)
            .cloned()
            .or_else(|| {
                // Fallback: look in Original Media/ for a file whose stem matches displayName
                find_original_media_by_stem(event_dir, &name)
            })
            .unwrap_or_default();

        let (start, duration) = pl
            .get("mediaRange")
            .and_then(PlistValue::as_str)
            .and_then(parse_fcp_range)
            .unwrap_or((FcpTime::ZERO, FcpTime::ZERO));

        let has_audio = pl
            .get("hasAudio")
            .and_then(PlistValue::as_bool)
            .unwrap_or(false);
        let audio_sources = pl
            .get("audioSourceCount")
            .and_then(PlistValue::as_i64)
            .unwrap_or(if has_audio { 1 } else { 0 })
            .max(0) as u32;
        let audio_sample_rate = pl
            .get("audioSampleRate")
            .and_then(PlistValue::as_f64)
            .map(|r| r.round() as u32)
            .unwrap_or(if has_audio { 48_000 } else { 0 });

        // Count channels across audioSourceDict if present
        let mut total_audio_ch = 0u32;
        if let Some(asd) = pl.get("audioSourceDict").and_then(PlistValue::as_dict) {
            for src_val in asd.values() {
                let ch = src_val
                    .get("FFMediaAudioSourceChannelCountKey")
                    .and_then(PlistValue::as_i64)
                    .unwrap_or(1)
                    .max(1) as u32;
                total_audio_ch += ch;
            }
        }
        let audio_channels = if !has_audio {
            0
        } else if total_audio_ch > 0 {
            total_audio_ch
        } else {
            audio_sources.max(2)
        };

        let fmt_name = pl
            .get("videoFormatName")
            .and_then(PlistValue::as_str)
            .unwrap_or("");
        let (width, height, fps, color_space) = parse_video_format_name(fmt_name)
            .or_else(|| props_by_name.get(&name).cloned())
            .unwrap_or((1920, 1080, Rational::FPS_25, None));

        let is_image_ext = Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "tif" | "tiff" | "psd" | "bmp" | "webp" | "heic" | "exr"
                )
            })
            .unwrap_or(false);
        let is_audio_only_ext = Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "wav" | "aif" | "aiff" | "mp3" | "m4a" | "aac" | "flac" | "ogg"
                )
            })
            .unwrap_or(false);

        let has_video = !is_audio_only_ext;
        let media_kind = if is_image_ext || (has_video && duration == FcpTime::ZERO && !has_audio) {
            MediaKind::Image
        } else if has_video {
            MediaKind::Video
        } else if has_audio {
            MediaKind::Audio
        } else {
            MediaKind::Unknown
        };

        out.push(BundleAsset {
            id,
            name,
            path,
            media_kind,
            has_video,
            has_audio,
            width,
            height,
            fps,
            start,
            duration,
            audio_channels,
            audio_sample_rate,
            color_space,
        });
    }

    Ok(out)
}

fn find_original_media_by_stem(event_dir: &Path, stem: &str) -> Option<String> {
    let om = event_dir.join("Original Media");
    let entries = fs::read_dir(om).ok()?;
    for e in entries.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.file_stem().and_then(|s| s.to_str()) == Some(stem) {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

// ── Project Sequence Loader ─────────────────────────────────────────────

fn read_project_sequence(
    proj_db_path: &Path,
    fallback_name: &str,
    assets_by_id: &HashMap<String, BundleAsset>,
) -> NleResult<Option<(Sequence, Option<String>)>> {
    let graph = EventGraph::load(proj_db_path)?;

    // Locate root project FFAnchoredCollection (`isProject == true`)
    let Some(root_col) = graph.nodes.values().find(|n| {
        n.ztype == "FFAnchoredCollection"
            && n.plist
                .as_ref()
                .and_then(|p| p.get("isProject"))
                .and_then(PlistValue::as_bool)
                .unwrap_or(false)
    }) else {
        return Ok(None);
    };

    let root_pl = root_col.plist.as_ref();
    let seq_name = root_pl
        .and_then(|p| p.get("displayName"))
        .and_then(PlistValue::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            graph
                .nodes
                .values()
                .find(|n| n.ztype == "FFAnchoredSequence")
                .and_then(|n| n.plist.as_ref())
                .and_then(|p| p.get("displayName"))
                .and_then(PlistValue::as_str)
        })
        .unwrap_or(fallback_name)
        .to_string();

    let seq_id = graph
        .nodes
        .values()
        .find(|n| n.ztype == "FFAnchoredSequence")
        .map(|n| {
            if !n.zident.is_empty() {
                n.zident.clone()
            } else {
                Uuid::new_v4().to_string()
            }
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let (width, height, seq_fps, color_space) = root_pl
        .and_then(|p| p.get("videoProps"))
        .and_then(parse_video_props)
        .unwrap_or((1920, 1080, Rational::FPS_25, None));

    let tc_start = graph
        .nodes
        .values()
        .find(|n| n.ztype == "FFSequenceInfo")
        .and_then(|n| n.plist.as_ref())
        .and_then(|p| p.get("mediaRange"))
        .and_then(PlistValue::as_str)
        .and_then(parse_fcp_range)
        .map(|(s, _)| s)
        .or_else(|| {
            root_pl
                .and_then(|p| p.get("unclippedStart"))
                .and_then(PlistValue::as_str)
                .and_then(FcpTime::parse)
        })
        .unwrap_or(FcpTime::ZERO);

    let mut builder = BundleTrackBuilder::new(width, height, seq_fps, tc_start);
    let mut markers = Vec::new();
    let mut next_id = 1usize;

    // Extract sequence-level markers attached to the root collection
    extract_bundle_markers(
        &graph,
        root_col.pk,
        tc_start,
        tc_start,
        seq_fps,
        MarkerScope::Sequence,
        &mut markers,
    );

    // Walk primary spine `containedItems` in `$order`
    let spine_items = graph.named_child_items(root_col.pk, "containedItems");
    let mut cursor = tc_start;
    for item_pk in spine_items {
        let Some(item) = graph.nodes.get(&item_pk) else {
            continue;
        };
        let item_dur = item
            .plist
            .as_ref()
            .and_then(|p| p.get("clippedRange"))
            .and_then(PlistValue::as_str)
            .and_then(parse_fcp_range)
            .map(|(_, d)| d)
            .unwrap_or(FcpTime::ZERO);

        if item.ztype == "FFAnchoredTransition" {
            // Transitions overlap the edit point at `cursor`; do not advance the spine cursor.
            if item_dur.num > 0 {
                let dur_frames = item_dur.to_frames(seq_fps).max(1);
                let cut_frame = (cursor - tc_start).to_frames(seq_fps).max(0);
                let half = dur_frames / 2;
                let tr_start = (cut_frame - half).max(0);
                let tr_name = item
                    .plist
                    .as_ref()
                    .and_then(|p| p.get("displayName"))
                    .and_then(PlistValue::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Cross Dissolve")
                    .to_string();
                let tr = Transition {
                    name: tr_name.clone(),
                    effect_id: tr_name,
                    start_frame: tr_start,
                    end_frame: tr_start + dur_frames,
                    alignment: "center".to_string(),
                };
                builder.add_video_transition(0, tr);
            }
            continue;
        }

        parse_bundle_item(
            &graph,
            item_pk,
            cursor,
            0,
            assets_by_id,
            &mut builder,
            &mut markers,
            &mut next_id,
        );
        if item_dur.num > 0 {
            cursor = cursor + item_dur;
        }
    }

    let (video_tracks, audio_tracks) = builder.finish();
    let mut seq = Sequence {
        id: seq_id,
        name: seq_name,
        kind: SequenceKind::Timeline,
        width,
        height,
        frame_rate: seq_fps,
        duration_frames: 0,
        start_tc_frame: tc_start.to_frames(seq_fps).max(0),
        mark_in: None,
        mark_out: None,
        video_tracks,
        audio_tracks,
        audio_buses: Vec::new(),
        markers,
        metadata: BTreeMap::new(),
    };
    seq.duration_frames = seq.computed_duration_frames();
    Ok(Some((seq, color_space)))
}

#[allow(clippy::too_many_arguments)]
fn parse_bundle_item(
    graph: &EventGraph,
    pk: i64,
    timeline_offset: FcpTime,
    lane: i32,
    assets_by_id: &HashMap<String, BundleAsset>,
    tb: &mut BundleTrackBuilder,
    markers: &mut Vec<Marker>,
    next_id: &mut usize,
) {
    let Some(node) = graph.nodes.get(&pk) else {
        return;
    };
    let Some(pl) = &node.plist else {
        return;
    };

    let (clipped_start, clipped_dur) = pl
        .get("clippedRange")
        .and_then(PlistValue::as_str)
        .and_then(parse_fcp_range)
        .unwrap_or((FcpTime::ZERO, FcpTime::ZERO));

    match node.ztype.as_str() {
        "FFAnchoredGapGeneratorComponent" => {
            walk_bundle_anchored_items(
                graph,
                pk,
                timeline_offset,
                clipped_start,
                assets_by_id,
                tb,
                markers,
                next_id,
            );
        }
        "FFAnchoredGeneratorComponent" => {
            let start_frame = (timeline_offset - tb.tc_start).to_frames(tb.seq_fps).max(0);
            let dur_frames = clipped_dur.to_frames(tb.seq_fps).max(1);
            let name = pl
                .get("displayName")
                .and_then(PlistValue::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("Generator / Title");
            markers.push(Marker {
                frame: start_frame,
                duration_frames: dur_frames,
                name: format!("Title: {name}"),
                comment: format!("Final Cut Pro Title / Generator: {name}"),
                color: "Cyan".to_string(),
                scope: MarkerScope::Sequence,
            });
            walk_bundle_anchored_items(
                graph,
                pk,
                timeline_offset,
                clipped_start,
                assets_by_id,
                tb,
                markers,
                next_id,
            );
        }
        "FFAnchoredCollection" => {
            let clip_name = pl
                .get("displayName")
                .and_then(PlistValue::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("Clip")
                .to_string();

            let start_frame = (timeline_offset - tb.tc_start).to_frames(tb.seq_fps).max(0);
            let end_frame = (timeline_offset + clipped_dur - tb.tc_start)
                .to_frames(tb.seq_fps)
                .max(start_frame + 1);

            let (mut vp_w, mut vp_h, mut vp_fps) = pl
                .get("videoProps")
                .and_then(parse_video_props)
                .map(|(w, h, f, _)| (w, h, f))
                .unwrap_or((tb.seq_width, tb.seq_height, tb.seq_fps));

            // Find contained FFAnchoredMediaComponent(s) and their FFAssetRef(s)
            let contained = graph.named_child_items(pk, "containedItems");
            let mut primary_asset_id: Option<String> = None;
            let mut has_video_comp = false;
            let mut has_audio_comp = false;
            let mut extra_effect_stacks: Vec<i64> = Vec::new();

            for &comp_pk in &contained {
                let Some(comp) = graph.nodes.get(&comp_pk) else {
                    continue;
                };
                if comp.ztype != "FFAnchoredMediaComponent" {
                    continue;
                }
                let src_key = comp
                    .plist
                    .as_ref()
                    .and_then(|p| p.get("providerSourceKey"))
                    .and_then(PlistValue::as_str)
                    .unwrap_or("");
                if src_key.starts_with('v') {
                    has_video_comp = true;
                } else if src_key.starts_with('a') {
                    has_audio_comp = true;
                }
                for &ref_pk in &graph.named_child_items(comp_pk, "media") {
                    if let Some(ref_node) = graph.nodes.get(&ref_pk)
                        && let Some(mid) = ref_node
                            .plist
                            .as_ref()
                            .and_then(|p| p.get("mediaIdentifier"))
                            .and_then(PlistValue::as_str)
                            && primary_asset_id.is_none() {
                                primary_asset_id = Some(mid.to_string());
                            }
                }
                // Component-level effectStack (e.g., FFStabilizationEffect lives on v1 component!)
                extra_effect_stacks.extend(graph.named_child_items(comp_pk, "effectStack"));

                // Embedded audio components anchored inside v1
                for &anc_pk in &graph.named_child_items(comp_pk, "anchoredItems") {
                    if let Some(anc) = graph.nodes.get(&anc_pk)
                        && anc.ztype == "FFAnchoredMediaComponent" {
                            let ak = anc
                                .plist
                                .as_ref()
                                .and_then(|p| p.get("providerSourceKey"))
                                .and_then(PlistValue::as_str)
                                .unwrap_or("");
                            if ak.starts_with('a') {
                                has_audio_comp = true;
                            }
                        }
                }
            }

            // Collect clip-level videoEffects & audioEffects + component effectStacks
            let mut stacks = graph.named_child_items(pk, "videoEffects");
            stacks.extend(graph.named_child_items(pk, "audioEffects"));
            stacks.extend(extra_effect_stacks);

            let (motion, opacity, blend_mode, effects) =
                extract_bundle_effects(graph, &stacks, tb.seq_width, tb.seq_height, next_id);

            let asset = primary_asset_id
                .as_deref()
                .and_then(|id| assets_by_id.get(id));

            let (media_id, media_path, media_kind, asset_start, audio_ch) = if let Some(a) = asset {
                if vp_w == 0 || vp_h == 0 {
                    vp_w = a.width;
                    vp_h = a.height;
                    vp_fps = a.fps;
                }
                (
                    a.id.clone(),
                    a.path.clone(),
                    a.media_kind,
                    a.start,
                    a.audio_channels.max(1),
                )
            } else {
                (
                    primary_asset_id.unwrap_or_default(),
                    String::new(),
                    if has_video_comp {
                        MediaKind::Video
                    } else {
                        MediaKind::Audio
                    },
                    FcpTime::ZERO,
                    2,
                )
            };

            let clip_enabled = !pl
                .get("disabled")
                .and_then(PlistValue::as_bool)
                .unwrap_or(false);

            let (speed_percent, reversed, speed_keyframes) =
                extract_bundle_retiming(graph, pk, pl, clipped_dur);

            let mut clip_markers = Vec::new();
            extract_bundle_markers(
                graph,
                pk,
                clipped_start,
                FcpTime::ZERO,
                vp_fps,
                MarkerScope::Clip,
                &mut clip_markers,
            );

            let src_in_frame = (clipped_start - asset_start).to_frames(vp_fps).max(0);
            let src_out_frame = ((clipped_start + clipped_dur) - asset_start)
                .to_frames(vp_fps)
                .max(src_in_frame + 1);

            let emit_video = has_video_comp || (!has_audio_comp && media_kind != MediaKind::Audio);
            let emit_audio = has_audio_comp || asset.map(|a| a.has_audio).unwrap_or(false);

            let link_group_id = if emit_video && emit_audio {
                let lg = format!("fcpb-link-{}", *next_id);
                *next_id += 1;
                Some(lg)
            } else {
                None
            };

            let (v_fx, a_fx) = if emit_video && emit_audio {
                let mut vf = Vec::new();
                let mut af = Vec::new();
                for fx in effects {
                    if fx.media_kind == TrackKind::Audio {
                        af.push(fx);
                    } else {
                        vf.push(fx);
                    }
                }
                (vf, af)
            } else if emit_video {
                (effects, Vec::new())
            } else {
                (Vec::new(), effects)
            };

            if emit_video {
                let cid = format!("fcpb-clip-{}", *next_id);
                *next_id += 1;
                tb.place_video_clip(
                    lane.max(0) as usize,
                    Clip {
                        id: cid,
                        name: clip_name.clone(),
                        media_id: media_id.clone(),
                        media_path: media_path.clone(),
                        media_kind,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame,
                        end_frame,
                        source_in_frame: src_in_frame,
                        source_out_frame: src_out_frame,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: vp_fps,
                        source_width: vp_w,
                        source_height: vp_h,
                        speed_percent,
                        reversed,
                        speed_keyframes: speed_keyframes.clone(),
                        retime_interpolation: RetimeInterpolation::Default,
                        enabled: clip_enabled,
                        audio_channel: 0,
                        link_group_id: link_group_id.clone(),
                        label_name: None,
                        label_preset_index: None,
                        motion,
                        opacity,
                        blend_mode,
                        lumetri_effects: Vec::new(),
                        effects: v_fx,
                        markers: clip_markers.clone(),
                    },
                );
            }

            if emit_audio {
                let cid = format!("fcpb-clip-{}", *next_id);
                *next_id += 1;
                let pref_a = if lane < 0 { ((-lane) - 1) as usize } else { 0 };
                tb.place_audio_clip(
                    pref_a,
                    Clip {
                        id: cid,
                        name: clip_name,
                        media_id,
                        media_path,
                        media_kind: if emit_video { media_kind } else { MediaKind::Audio },
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame,
                        end_frame,
                        source_in_frame: src_in_frame,
                        source_out_frame: src_out_frame,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: vp_fps,
                        source_width: vp_w,
                        source_height: vp_h,
                        speed_percent,
                        reversed,
                        speed_keyframes,
                        retime_interpolation: RetimeInterpolation::Default,
                        enabled: clip_enabled,
                        audio_channel: 1,
                        link_group_id,
                        label_name: None,
                        label_preset_index: None,
                        motion: MotionTransform::default(),
                        opacity: 100.0,
                        blend_mode: BlendMode::Normal,
                        lumetri_effects: Vec::new(),
                        effects: a_fx,
                        markers: if emit_video { Vec::new() } else { clip_markers },
                    },
                    audio_ch,
                );
            }

            // Walk connected clips/titles anchored to this clip
            walk_bundle_anchored_items(
                graph,
                pk,
                timeline_offset,
                clipped_start,
                assets_by_id,
                tb,
                markers,
                next_id,
            );
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_bundle_anchored_items(
    graph: &EventGraph,
    parent_pk: i64,
    parent_timeline_offset: FcpTime,
    parent_clipped_start: FcpTime,
    assets_by_id: &HashMap<String, BundleAsset>,
    tb: &mut BundleTrackBuilder,
    markers: &mut Vec<Marker>,
    next_id: &mut usize,
) {
    let anchored = graph.named_child_items(parent_pk, "anchoredItems");
    for child_pk in anchored {
        let Some(child) = graph.nodes.get(&child_pk) else {
            continue;
        };
        let Some(pl) = &child.plist else {
            continue;
        };
        let lane = pl
            .get("anchoredLane")
            .and_then(PlistValue::as_i64)
            .unwrap_or(1) as i32;
        if lane == 999999 {
            continue;
        }
        let child_clipped_start = pl
            .get("clippedRange")
            .and_then(PlistValue::as_str)
            .and_then(parse_fcp_range)
            .map(|(s, _)| s)
            .unwrap_or(FcpTime::ZERO);

        let child_tl_offset = if let Some((child_anchor, parent_anchor)) = pl
            .get("anchorPair")
            .and_then(PlistValue::as_str)
            .and_then(parse_fcp_range)
        {
            parent_timeline_offset
                + (parent_anchor - parent_clipped_start)
                - (child_anchor - child_clipped_start)
        } else {
            parent_timeline_offset
        };

        parse_bundle_item(
            graph,
            child_pk,
            child_tl_offset,
            lane,
            assets_by_id,
            tb,
            markers,
            next_id,
        );
    }
}

// ── Bundle Markers & Retiming Decoders ──────────────────────────────────

fn extract_bundle_markers(
    graph: &EventGraph,
    parent_pk: i64,
    origin: FcpTime,
    tc_offset: FcpTime,
    fps: Rational,
    scope: MarkerScope,
    out: &mut Vec<Marker>,
) {
    let mut marker_pks = graph.named_child_items(parent_pk, "markers");
    marker_pks.extend(graph.named_child_items(parent_pk, "metadata"));

    for mpk in marker_pks {
        let Some(node) = graph.nodes.get(&mpk) else {
            continue;
        };
        if !matches!(
            node.ztype.as_str(),
            "FFMarker" | "FFChapterMarker" | "FFToDoMarker" | "FFAnchoredMarker"
        ) {
            continue;
        }
        let Some(pl) = &node.plist else {
            continue;
        };
        let (m_start, m_dur) = pl
            .get("range")
            .or_else(|| pl.get("clippedRange"))
            .and_then(PlistValue::as_str)
            .and_then(parse_fcp_range)
            .unwrap_or_else(|| {
                let s = pl
                    .get("start")
                    .and_then(PlistValue::as_str)
                    .and_then(FcpTime::parse)
                    .unwrap_or(origin);
                let d = pl
                    .get("duration")
                    .and_then(PlistValue::as_str)
                    .and_then(FcpTime::parse)
                    .unwrap_or(FcpTime::ZERO);
                (s, d)
            });

        let rel_time = if scope == MarkerScope::Sequence {
            m_start - tc_offset
        } else {
            m_start - origin
        };
        let frame = rel_time.to_frames(fps).max(0);
        let duration_frames = m_dur.to_frames(fps).max(1);
        let name = pl
            .get("name")
            .or_else(|| pl.get("displayName"))
            .or_else(|| pl.get("value"))
            .and_then(PlistValue::as_str)
            .unwrap_or("Marker")
            .to_string();
        let comment = pl
            .get("note")
            .or_else(|| pl.get("comment"))
            .and_then(PlistValue::as_str)
            .unwrap_or("")
            .to_string();
        let color = match node.ztype.as_str() {
            "FFChapterMarker" => "Orange",
            "FFToDoMarker" => {
                if pl
                    .get("completed")
                    .and_then(PlistValue::as_bool)
                    .unwrap_or(false)
                {
                    "Green"
                } else {
                    "Red"
                }
            }
            _ => "Blue",
        }
        .to_string();

        out.push(Marker {
            frame,
            duration_frames,
            name,
            comment,
            color,
            scope,
        });
    }
}

fn extract_bundle_retiming(
    graph: &EventGraph,
    clip_pk: i64,
    clip_pl: &PlistValue,
    clipped_dur: FcpTime,
) -> (f64, bool, Vec<SpeedKeyframe>) {
    // 1. Direct speed / reverse properties on the collection plist (if present)
    if let Some(rate) = clip_pl
        .get("rate")
        .or_else(|| clip_pl.get("speed"))
        .and_then(PlistValue::as_f64)
        && rate.is_finite() && (rate - 1.0).abs() > 1e-4 {
            let pct = if rate.abs() <= 20.0 { rate * 100.0 } else { rate };
            return (pct, pct < 0.0, Vec::new());
        }

    // 2. Inspect child timeMap / FFTimeMap nodes (if present)
    let mut tm_pks = graph.named_child_items(clip_pk, "timeMap");
    tm_pks.extend(graph.named_child_items(clip_pk, "timeRemap"));
    for comp_pk in graph.named_child_items(clip_pk, "containedItems") {
        tm_pks.extend(graph.named_child_items(comp_pk, "timeMap"));
    }

    for tpk in tm_pks {
        let Some(tnode) = graph.nodes.get(&tpk) else {
            continue;
        };
        let Some(tpl) = &tnode.plist else {
            continue;
        };
        if let Some(pts_arr) = tpl
            .get("timePoints")
            .or_else(|| tpl.get("points"))
            .and_then(PlistValue::as_array)
        {
            let mut pts: Vec<SpeedKeyframe> = Vec::new();
            for pt in pts_arr {
                let t = pt
                    .get("time")
                    .and_then(PlistValue::as_str)
                    .and_then(FcpTime::parse)
                    .map(|ft| ft.num as f64 / ft.den.max(1) as f64)
                    .or_else(|| pt.get("time").and_then(PlistValue::as_f64));
                let v = pt
                    .get("value")
                    .and_then(PlistValue::as_str)
                    .and_then(FcpTime::parse)
                    .map(|ft| ft.num as f64 / ft.den.max(1) as f64)
                    .or_else(|| pt.get("value").and_then(PlistValue::as_f64));
                if let (Some(timeline_sec), Some(source_sec)) = (t, v)
                    && timeline_sec.is_finite() && source_sec.is_finite() {
                        pts.push(SpeedKeyframe {
                            timeline_sec,
                            source_sec,
                        });
                    }
            }
            if pts.len() >= 2 {
                let dt = pts.last().unwrap().timeline_sec - pts[0].timeline_sec;
                let dv = pts.last().unwrap().source_sec - pts[0].source_sec;
                if dt.abs() > 1e-6 {
                    let speed = (dv / dt) * 100.0;
                    let kfs = if pts.len() > 2 { pts } else { Vec::new() };
                    return (speed, speed < 0.0, kfs);
                }
            }
        }
        if let Some(rate) = tpl.get("rate").and_then(PlistValue::as_f64)
            && rate.is_finite() && (rate - 1.0).abs() > 1e-4 {
                let pct = if rate.abs() <= 20.0 { rate * 100.0 } else { rate };
                return (pct, pct < 0.0, Vec::new());
            }
    }

    let _ = clipped_dur;
    (100.0, false, Vec::new())
}

// ── Effect & OZML Decoding ──────────────────────────────────────────────

fn extract_bundle_effects(
    graph: &EventGraph,
    stack_pks: &[i64],
    seq_width: u32,
    seq_height: u32,
    next_id: &mut usize,
) -> (MotionTransform, f64, BlendMode, Vec<EffectInstance>) {
    let mut motion = MotionTransform::default();
    let mut opacity = 100.0_f64;
    let mut blend_mode = BlendMode::Normal;
    let mut effects = Vec::new();
    let mut visited = HashSet::new();

    for &spk in stack_pks {
        let mut fx_pks = graph.named_child_items(spk, "intrinsicEffects");
        fx_pks.extend(graph.named_child_items(spk, "effects"));

        for fx_pk in fx_pks {
            if !visited.insert(fx_pk) {
                continue;
            }
            process_bundle_effect_node(
                graph,
                fx_pk,
                seq_width,
                seq_height,
                &mut motion,
                &mut opacity,
                &mut blend_mode,
                &mut effects,
                next_id,
                &mut visited,
            );
        }
    }

    (motion, opacity, blend_mode, effects)
}

#[allow(clippy::too_many_arguments)]
fn process_bundle_effect_node(
    graph: &EventGraph,
    fx_pk: i64,
    seq_width: u32,
    seq_height: u32,
    motion: &mut MotionTransform,
    opacity: &mut f64,
    blend_mode: &mut BlendMode,
    effects: &mut Vec<EffectInstance>,
    next_id: &mut usize,
    visited: &mut HashSet<i64>,
) {
    let Some(node) = graph.nodes.get(&fx_pk) else {
        return;
    };
    let Some(pl) = &node.plist else {
        return;
    };

    match node.ztype.as_str() {
        "FFHeXForm3DEffect" => {
            if let Some(xml_bytes) = pl.get("channelData").and_then(PlistValue::as_bytes) {
                parse_ozml_transform(xml_bytes, seq_width, seq_height, motion);
            }
        }
        "FFHeBlendEffect" => {
            if let Some(xml_bytes) = pl.get("channelData").and_then(PlistValue::as_bytes) {
                parse_ozml_blend(xml_bytes, opacity, blend_mode, effects, next_id);
            }
        }
        "FFStabilizationEffect" => {
            let id = format!("fcpb-fx-{}", *next_id);
            *next_id += 1;
            effects.push(EffectInstance {
                id,
                match_name: "fcp.stabilization".to_string(),
                display_name: "Stabilization".to_string(),
                category: EffectCategory::Stabilization,
                media_kind: TrackKind::Video,
                enabled: true,
                is_intrinsic: false,
                is_animated: false,
                parameters: vec![EffectParameter {
                    id: "type".to_string(),
                    name: "Method".to_string(),
                    value: "automatic".to_string(),
                    is_animated: false,
                }],
            });
        }
        "FFSingleMaskedEffect" => {
            for wrapped_pk in graph.named_child_items(fx_pk, "wrappedEffect") {
                if visited.insert(wrapped_pk) {
                    process_bundle_effect_node(
                        graph,
                        wrapped_pk,
                        seq_width,
                        seq_height,
                        motion,
                        opacity,
                        blend_mode,
                        effects,
                        next_id,
                        visited,
                    );
                }
            }
        }
        "FFMotionEffect" | "FFFxPlugEffect" | "FFAudioUnitEffect" => {
            let display_name = pl
                .get("displayName")
                .and_then(PlistValue::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("Effect")
                .to_string();
            let effect_id = pl
                .get("effectID")
                .and_then(PlistValue::as_str)
                .unwrap_or("");
            let is_audio = node.ztype == "FFAudioUnitEffect"
                || pl
                    .get("effectType")
                    .and_then(PlistValue::as_str)
                    .map(|t| t.contains("audio"))
                    .unwrap_or(false);
            let match_name = if effect_id.is_empty() {
                format!("fcp.{display_name}")
            } else {
                format!("fcp.{display_name}:{effect_id}")
            };
            let id = format!("fcpb-fx-{}", *next_id);
            *next_id += 1;
            effects.push(EffectInstance {
                id,
                match_name,
                display_name,
                category: if is_audio {
                    EffectCategory::Audio
                } else {
                    EffectCategory::Plugin
                },
                media_kind: if is_audio {
                    TrackKind::Audio
                } else {
                    TrackKind::Video
                },
                enabled: true,
                is_intrinsic: false,
                is_animated: false,
                parameters: Vec::new(),
            });
        }
        _ => {}
    }
}

fn parse_ozml_transform(
    xml_bytes: &[u8],
    seq_width: u32,
    seq_height: u32,
    motion: &mut MotionTransform,
) {
    let xml = String::from_utf8_lossy(xml_bytes);
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let Ok(doc) = Document::parse_with_options(&xml, opts) else {
        return;
    };
    let aspect = if seq_height > 0 {
        seq_width as f64 / seq_height as f64
    } else {
        16.0 / 9.0
    };

    let mut scale_x: Option<f64> = None;
    let mut scale_y: Option<f64> = None;

    for param in doc
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("parameter"))
    {
        let pname = param.attribute("name").unwrap_or("");
        let parent_name = param
            .parent()
            .and_then(|p| p.attribute("name"))
            .unwrap_or("");
        let curve = param
            .children()
            .find(|n| n.is_element() && n.has_tag_name("curve"));
        let Some(val) = curve
            .and_then(|c| c.attribute("value"))
            .and_then(|s| s.parse::<f64>().ok())
        else {
            continue;
        };

        match (parent_name, pname) {
            ("Rotation", "Z") => {
                // OZML Rotation Z is stored in radians (CCW-positive) -> IR degrees (CW-positive)
                motion.rotation = -val.to_degrees();
            }
            ("Scale (All)", "Scale X") => scale_x = Some(val),
            ("Scale (All)", "Scale Y") => scale_y = Some(val),
            ("Position", "X") => motion.position_x = (val / 100.0) / aspect,
            ("Position", "Y") => motion.position_y = val / 100.0,
            _ => {}
        }
    }

    if let Some(sx) = scale_x {
        motion.scale = sx * 100.0;
        if let Some(sy) = scale_y
            && (sx - sy).abs() > 1e-4 {
                motion.scale_y = Some(sy * 100.0);
            }
    }
}

fn parse_ozml_blend(
    xml_bytes: &[u8],
    opacity: &mut f64,
    blend_mode: &mut BlendMode,
    effects: &mut Vec<EffectInstance>,
    next_id: &mut usize,
) {
    let xml = String::from_utf8_lossy(xml_bytes);
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let Ok(doc) = Document::parse_with_options(&xml, opts) else {
        return;
    };

    for param in doc
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("parameter"))
    {
        let pname = param.attribute("name").unwrap_or("");
        let curve = param
            .children()
            .find(|n| n.is_element() && n.has_tag_name("curve"));
        let Some(val) = curve
            .and_then(|c| c.attribute("value"))
            .and_then(|s| s.parse::<f64>().ok())
        else {
            continue;
        };
        if pname == "Opacity" {
            let pct = if val <= 1.0 { val * 100.0 } else { val };
            *opacity = pct.clamp(0.0, 100.0);
        } else if pname == "Blend Mode" && val.round() as i64 != 0 {
            let code = val.round() as i64;
            if let Some(bm) = BlendMode::from_fcp_code(code) {
                *blend_mode = bm;
            } else {
                let mode_label = fcp_blend_mode_name(code);
                let id = format!("fcpb-fx-{}", *next_id);
                *next_id += 1;
                effects.push(EffectInstance {
                    id,
                    match_name: "fcp.compositing".to_string(),
                    display_name: format!("Blend Mode ({mode_label})"),
                    category: EffectCategory::Transform,
                    media_kind: TrackKind::Video,
                    enabled: true,
                    is_intrinsic: false,
                    is_animated: false,
                    parameters: vec![EffectParameter {
                        id: "mode".to_string(),
                        name: "Blend Mode".to_string(),
                        value: mode_label,
                        is_animated: false,
                    }],
                });
            }
        }
    }
}

fn fcp_blend_mode_name(code: i64) -> String {
    match code {
        0 => "0 (Normal)".to_string(),
        2 => "2 (Subtract)".to_string(),
        3 => "3 (Multiply)".to_string(),
        4 => "4 (Screen)".to_string(),
        5 => "5 (Darken)".to_string(),
        6 => "6 (Lighten)".to_string(),
        7 => "7 (Add)".to_string(),
        14 => "14 (Overlay)".to_string(),
        15 => "15 (Soft Light)".to_string(),
        16 => "16 (Hard Light)".to_string(),
        17 => "17 (Color Dodge)".to_string(),
        18 => "18 (Color Burn)".to_string(),
        21 => "21 (Difference)".to_string(),
        22 => "22 (Exclusion)".to_string(),
        other => format!("{other}"),
    }
}

// ── String & VideoProps Parsers ─────────────────────────────────────────

/// Parse FCP range/pair string `"{(num1/den1),(num2/den2)}"` into `(FcpTime, FcpTime)`.
fn parse_fcp_range(s: &str) -> Option<(FcpTime, FcpTime)> {
    let inner = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (left, right) = inner.split_once(',')?;
    let t1 = FcpTime::parse(left.trim().trim_matches(['(', ')']))?;
    let t2 = FcpTime::parse(right.trim().trim_matches(['(', ')']))?;
    Some((t1, t2))
}

fn parse_video_props(val: &PlistValue) -> Option<(u32, u32, Rational, Option<String>)> {
    let pb = val.get("PB").and_then(PlistValue::as_str)?;
    // Format: "{{0, 0}, {1920, 1080}}"
    let (width, height) = parse_nsrect_size(pb)?;

    let fps = val
        .get("FRD")
        .and_then(|frd| {
            let v = frd.get("value").and_then(PlistValue::as_i64)?;
            let ts = frd.get("timescale").and_then(PlistValue::as_i64)?;
            if v > 0 && ts > 0 {
                Some(Rational::new(ts, v))
            } else {
                None
            }
        })
        .unwrap_or(Rational::FPS_25);

    let color_space = val
        .get("FFVideoFormat.name")
        .and_then(PlistValue::as_str)
        .and_then(color_space_from_format_name)
        .or_else(|| {
            val.get("CSE")
                .and_then(PlistValue::as_i64)
                .and_then(color_space_from_cse)
        });

    Some((width, height, fps, color_space))
}

fn parse_nsrect_size(s: &str) -> Option<(u32, u32)> {
    // "{{0, 0}, {1920, 1080}}" -> extract last "{w, h}"
    let last_open = s.rfind('{')?;
    let rest = &s[last_open + 1..];
    let close = rest.find('}')?;
    let (w_s, h_s) = rest[..close].split_once(',')?;
    let w = w_s.trim().parse::<f64>().ok()?.round() as u32;
    let h = h_s.trim().parse::<f64>().ok()?.round() as u32;
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

fn parse_video_format_name(name: &str) -> Option<(u32, u32, Rational, Option<String>)> {
    if !name.starts_with("FFVideoFormat") {
        return None;
    }
    let body = name.strip_prefix("FFVideoFormat")?;
    let core = body.split('_').next().unwrap_or(body);
    let (res_part, fps_part) = core.split_once('p').or_else(|| core.split_once('i'))?;

    let (w, h) = if let Some((ws, hs)) = res_part.split_once('x') {
        (ws.parse::<u32>().ok()?, hs.parse::<u32>().ok()?)
    } else {
        let h = res_part.parse::<u32>().ok()?;
        let w = match h {
            1080 => 1920,
            720 => 1280,
            2160 => 3840,
            _ => (h * 16) / 9,
        };
        (w, h)
    };

    let fps = match fps_part {
        "2398" => Rational::FPS_23_976,
        "24" => Rational::FPS_24,
        "25" => Rational::FPS_25,
        "2997" => Rational::FPS_29_97,
        "30" => Rational::FPS_30,
        "50" => Rational::FPS_50,
        "5994" => Rational::FPS_59_94,
        "60" => Rational::FPS_60,
        other => other
            .parse::<f64>()
            .ok()
            .map(Rational::from_fps_approx)
            .unwrap_or(Rational::FPS_25),
    };

    Some((w, h, fps, color_space_from_format_name(name)))
}

fn color_space_from_format_name(name: &str) -> Option<String> {
    if name.contains("Rec2020_PQ") {
        Some("9-16-9 (Rec. 2020 PQ)".to_string())
    } else if name.contains("Rec2020_HLG") {
        Some("9-18-9 (Rec. 2020 HLG)".to_string())
    } else if name.contains("Rec2020") {
        Some("9-1-9 (Rec. 2020)".to_string())
    } else {
        None
    }
}

fn color_space_from_cse(cse: i64) -> Option<String> {
    // 24-bit packed: (primaries << 16) | (transfer << 8) | matrix
    // 593929 = 0x091009 = 9-16-9 (Rec. 2020 PQ)
    // 594441 = 0x091209 = 9-18-9 (Rec. 2020 HLG)
    // 65793  = 0x010101 = 1-1-1  (Rec. 709)
    match cse {
        0x09_10_09 => Some("9-16-9 (Rec. 2020 PQ)".to_string()),
        0x09_12_09 => Some("9-18-9 (Rec. 2020 HLG)".to_string()),
        0x01_01_01 => Some("1-1-1 (Rec. 709)".to_string()),
        _ => None,
    }
}

// ── Track Builder ───────────────────────────────────────────────────────

struct BundleTrackBuilder {
    seq_width: u32,
    seq_height: u32,
    seq_fps: Rational,
    tc_start: FcpTime,
    video_tracks: Vec<Track>,
    audio_tracks: Vec<Track>,
}

impl BundleTrackBuilder {
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

    fn add_video_transition(&mut self, track_idx: usize, tr: Transition) {
        self.ensure_video_track(track_idx);
        self.video_tracks[track_idx].transitions.push(tr);
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

    fn finish(mut self) -> (Vec<Track>, Vec<Track>) {
        if self.video_tracks.is_empty() {
            self.ensure_video_track(0);
        }
        for (i, t) in self.video_tracks.iter_mut().enumerate() {
            t.index = i + 1;
            t.name = format!("V{}", i + 1);
            t.clips.sort_by_key(|c| c.start_frame);
        }
        for (i, t) in self.audio_tracks.iter_mut().enumerate() {
            t.index = i + 1;
            t.name = format!("A{}", i + 1);
            t.clips.sort_by_key(|c| c.start_frame);
        }
        (self.video_tracks, self.audio_tracks)
    }
}
