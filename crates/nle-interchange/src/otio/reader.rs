use crate::fcpxml::time::file_url_to_path;
use nle_core::{
    frames_to_timecode, AlphaMode, BlendMode, Clip, ClipSourceKind, ColorConfig, DataLevels,
    EffectCategory, EffectInstance, EffectParameter, Marker, MarkerScope, MediaAsset, MediaKind,
    MotionTransform, NleError, NleFormat, NleResult, Project, ProjectReader, Rational,
    RetimeInterpolation, Sequence, SequenceKind, Track, TrackKind, Transition,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct OtioReader;

impl ProjectReader for OtioReader {
    fn format(&self) -> NleFormat {
        NleFormat::OpenTimelineIo
    }

    fn read_file(&self, path: &Path) -> NleResult<Project> {
        read_otio_file(path)
    }
}

/// Read an OpenTimelineIO file (`.otio`), bundle directory (`.otiod`), or zip archive (`.otioz`)
/// into the canonical `Project` IR.
pub fn read_otio_file(path: &Path) -> NleResult<Project> {
    let default_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("OpenTimelineIO Project")
        .to_string();

    if path.is_dir() {
        let content_path = resolve_otiod_content(path)?;
        let json_str = fs::read_to_string(&content_path)?;
        let mut project = parse_otio_str_with_base(&json_str, &default_name, Some(path))?;
        project.source_path = Some(path.to_string_lossy().into_owned());
        return Ok(project);
    }

    let is_otioz = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| e.eq_ignore_ascii_case("otioz"))
        .unwrap_or(false);

    if is_otioz {
        let json_str = read_otioz_content(path)?;
        let mut project = parse_otio_str_with_base(&json_str, &default_name, path.parent())?;
        project.source_path = Some(path.to_string_lossy().into_owned());
        return Ok(project);
    }

    let json_str = fs::read_to_string(path)?;
    let mut project = parse_otio_str_with_base(&json_str, &default_name, path.parent())?;
    project.source_path = Some(path.to_string_lossy().into_owned());
    Ok(project)
}

/// Parse an OpenTimelineIO JSON string (`Timeline.1`, `SerializableCollection.1`, or `Stack.1`)
/// into the canonical `Project` IR.
pub fn parse_otio_str(json_str: &str, default_name: &str) -> NleResult<Project> {
    parse_otio_str_with_base(json_str, default_name, None)
}

fn resolve_otiod_content(bundle_dir: &Path) -> NleResult<PathBuf> {
    let content = bundle_dir.join("content.otio");
    if content.is_file() {
        return Ok(content);
    }
    if let Ok(entries) = fs::read_dir(bundle_dir) {
        let mut candidates: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|de| de.path()))
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("otio"))
                        .unwrap_or(false)
            })
            .collect();
        candidates.sort();
        if let Some(first) = candidates.into_iter().next() {
            return Ok(first);
        }
    }
    Err(NleError::InvalidFormat(format!(
        "No content.otio found inside OTIO bundle '{}'",
        bundle_dir.display()
    )))
}

fn read_otioz_content(path: &Path) -> NleResult<String> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| NleError::Archive(format!("Invalid .otioz archive: {e}")))?;

    let mut target_idx: Option<usize> = None;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| NleError::Archive(format!("Failed reading .otioz entry: {e}")))?;
        let name = entry.name();
        if name == "content.otio" || name.ends_with("/content.otio") {
            target_idx = Some(i);
            break;
        }
        if target_idx.is_none() && name.to_ascii_lowercase().ends_with(".otio") {
            target_idx = Some(i);
        }
    }

    let idx = target_idx.ok_or_else(|| {
        NleError::InvalidFormat(format!(
            "No content.otio found inside '{}'",
            path.display()
        ))
    })?;
    let mut entry = archive
        .by_index(idx)
        .map_err(|e| NleError::Archive(format!("Failed opening content.otio: {e}")))?;
    let mut out = String::new();
    entry.read_to_string(&mut out)?;
    Ok(out)
}

fn parse_otio_str_with_base(
    json_str: &str,
    default_name: &str,
    base_dir: Option<&Path>,
) -> NleResult<Project> {
    let root: Value = serde_json::from_str(json_str)?;
    let schema = schema_base(&root).ok_or_else(|| {
        NleError::InvalidFormat("Missing OTIO_SCHEMA field in OpenTimelineIO JSON".into())
    })?;

    let mut ctx = OtioParseCtx::new(base_dir);
    let mut project_name = default_name.to_string();
    let mut sequences: Vec<Sequence> = Vec::new();

    match schema {
        "Timeline" => {
            if let Some(pname) = root
                .pointer("/metadata/aagedal_nle_converter/project_name")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                project_name = pname.to_string();
            } else if let Some(tname) = root
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                && (project_name.is_empty() || project_name == "content")
            {
                project_name = tname.to_string();
            }
            let seq = ctx.parse_timeline(&root, SequenceKind::Timeline, &project_name);
            sequences.push(seq);
        }
        "SerializableCollection" => {
            if let Some(cname) = root
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                project_name = cname.to_string();
            }
            if let Some(children) = root.get("children").and_then(Value::as_array) {
                for (idx, child) in children.iter().enumerate() {
                    let fallback = format!("Timeline_{}", idx + 1);
                    match schema_base(child) {
                        Some("Timeline") => {
                            let seq = ctx.parse_timeline(child, SequenceKind::Timeline, &fallback);
                            sequences.push(seq);
                        }
                        Some("Stack") => {
                            let seq = ctx.parse_stack_as_sequence(
                                child,
                                SequenceKind::Timeline,
                                &fallback,
                                Rational::FPS_25,
                                1920,
                                1080,
                            );
                            sequences.push(seq);
                        }
                        _ => {}
                    }
                }
            }
        }
        "Stack" => {
            let seq = ctx.parse_stack_as_sequence(
                &root,
                SequenceKind::Timeline,
                &project_name,
                infer_fps_from_node(&root).unwrap_or(Rational::FPS_25),
                1920,
                1080,
            );
            sequences.push(seq);
        }
        other => {
            return Err(NleError::InvalidFormat(format!(
                "Unsupported OpenTimelineIO root schema '{other}'. Expected Timeline, SerializableCollection, or Stack."
            )));
        }
    }

    // Append any nested compound sequences discovered inside tracks
    sequences.extend(ctx.extra_sequences);

    let mut metadata = BTreeMap::new();
    if let Some(raw_schema) = root.get("OTIO_SCHEMA").and_then(Value::as_str) {
        metadata.insert("otio_schema".into(), raw_schema.to_string());
    }

    Ok(Project {
        name: project_name,
        source_format: NleFormat::OpenTimelineIo,
        source_path: None,
        color_config: ColorConfig::default(),
        scale_to_frame_size: true,
        bins: Vec::new(),
        media_assets: ctx.media_assets,
        sequences,
        metadata,
        custom_metadata_schema: BTreeMap::new(),
    })
}

// ── Internal Parsing Context ────────────────────────────────────────────

struct OtioParseCtx<'a> {
    base_dir: Option<&'a Path>,
    media_assets: Vec<MediaAsset>,
    asset_index_by_key: HashMap<(String, MediaKind), usize>,
    extra_sequences: Vec<Sequence>,
    next_clip_id: usize,
}

impl<'a> OtioParseCtx<'a> {
    fn new(base_dir: Option<&'a Path>) -> Self {
        Self {
            base_dir,
            media_assets: Vec::new(),
            asset_index_by_key: HashMap::new(),
            extra_sequences: Vec::new(),
            next_clip_id: 1,
        }
    }

    fn alloc_clip_id(&mut self) -> String {
        let id = format!("otio-clip-{}", self.next_clip_id);
        self.next_clip_id += 1;
        id
    }

    fn parse_timeline(
        &mut self,
        node: &Value,
        kind: SequenceKind,
        fallback_name: &str,
    ) -> Sequence {
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(fallback_name)
            .to_string();

        let global_start = node
            .get("global_start_time")
            .and_then(OtioTime::from_json);
        let seq_fps = global_start
            .filter(|t| t.rate > 0.0)
            .map(|t| Rational::from_fps_approx(t.rate))
            .or_else(|| {
                node.pointer("/metadata/Resolve_OTIO/TimelineFrameRate")
                    .and_then(|v| {
                        v.as_f64()
                            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                    })
                    .filter(|&r| r > 0.0)
                    .map(Rational::from_fps_approx)
            })
            .or_else(|| infer_fps_from_node(node))
            .unwrap_or(Rational::FPS_25);

        let start_tc_frame = global_start
            .map(|t| t.to_frames(seq_fps).max(0))
            .unwrap_or(0);

        let (width, height) = infer_resolution_from_timeline(node);

        let mut seq = if let Some(tracks_stack) = node.get("tracks") {
            self.parse_stack_as_sequence(tracks_stack, kind, &name, seq_fps, width, height)
        } else {
            Sequence {
                id: Uuid::new_v4().to_string(),
                name: name.clone(),
                kind,
                width,
                height,
                frame_rate: seq_fps,
                duration_frames: 0,
                start_tc_frame,
                mark_in: None,
                mark_out: None,
                video_tracks: Vec::new(),
                audio_tracks: Vec::new(),
                audio_buses: Vec::new(),
                markers: Vec::new(),
                metadata: BTreeMap::new(),
            }
        };

        seq.name = name;
        seq.start_tc_frame = start_tc_frame;
        // Also collect markers placed directly on Timeline (if any)
        if let Some(mk_arr) = node.get("markers").and_then(Value::as_array) {
            for m in mk_arr {
                if let Some(parsed) = parse_marker(m, seq_fps, MarkerScope::Sequence, 0) {
                    seq.markers.push(parsed);
                }
            }
        }
        seq.duration_frames = seq.computed_duration_frames();
        seq
    }

    fn parse_stack_as_sequence(
        &mut self,
        stack: &Value,
        kind: SequenceKind,
        fallback_name: &str,
        seq_fps: Rational,
        width: u32,
        height: u32,
    ) -> Sequence {
        let name = stack
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty() && !s.eq_ignore_ascii_case("tracks"))
            .unwrap_or(fallback_name)
            .to_string();

        let mut video_tracks = Vec::new();
        let mut audio_tracks = Vec::new();

        if let Some(children) = stack.get("children").and_then(Value::as_array) {
            for child in children {
                if schema_base(child) == Some("Track") {
                    let is_audio = child
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(|k| k.eq_ignore_ascii_case("audio"))
                        .unwrap_or(false);
                    let track_kind = if is_audio {
                        TrackKind::Audio
                    } else {
                        TrackKind::Video
                    };
                    let idx = if is_audio {
                        audio_tracks.len() + 1
                    } else {
                        video_tracks.len() + 1
                    };
                    let track = self.parse_track(child, track_kind, idx, seq_fps, width, height);
                    if is_audio {
                        audio_tracks.push(track);
                    } else {
                        video_tracks.push(track);
                    }
                }
            }
        }

        let mut markers = Vec::new();
        if let Some(mk_arr) = stack.get("markers").and_then(Value::as_array) {
            for m in mk_arr {
                if let Some(parsed) = parse_marker(m, seq_fps, MarkerScope::Sequence, 0) {
                    markers.push(parsed);
                }
            }
        }

        let mut seq = Sequence {
            id: Uuid::new_v4().to_string(),
            name,
            kind,
            width,
            height,
            frame_rate: seq_fps,
            duration_frames: 0,
            start_tc_frame: 0,
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

    fn parse_track(
        &mut self,
        node: &Value,
        kind: TrackKind,
        index: usize,
        seq_fps: Rational,
        seq_w: u32,
        seq_h: u32,
    ) -> Track {
        let default_name = match kind {
            TrackKind::Video => format!("V{index}"),
            TrackKind::Audio => format!("A{index}"),
        };
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&default_name)
            .to_string();
        let enabled = node
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let audio_channels = if kind == TrackKind::Audio {
            node.pointer("/metadata/aagedal_nle_converter/audio_channels")
                .and_then(Value::as_u64)
                .map(|c| c as u32)
                .unwrap_or(2)
        } else {
            0
        };

        let mut clips = Vec::new();
        let mut transitions = Vec::new();
        let mut cursor: i64 = 0;

        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                match schema_base(child) {
                    Some("Gap") => {
                        let dur = child
                            .pointer("/source_range/duration")
                            .and_then(OtioTime::from_json)
                            .map(|t| t.to_frames(seq_fps).max(0))
                            .unwrap_or(0);
                        cursor += dur;
                    }
                    Some("Transition") => {
                        let in_frames = child
                            .get("in_offset")
                            .and_then(OtioTime::from_json)
                            .map(|t| t.to_frames(seq_fps).max(0))
                            .unwrap_or(0);
                        let out_frames = child
                            .get("out_offset")
                            .and_then(OtioTime::from_json)
                            .map(|t| t.to_frames(seq_fps).max(0))
                            .unwrap_or(0);
                        let total = in_frames + out_frames;
                        if total > 0 {
                            let start_frame = (cursor - in_frames).max(0);
                            let end_frame = (cursor + out_frames).max(start_frame + 1);
                            let alignment = if in_frames == 0 && out_frames > 0 {
                                "start"
                            } else if out_frames == 0 && in_frames > 0 {
                                "end"
                            } else {
                                "center"
                            };
                            let tr_type = child
                                .get("transition_type")
                                .and_then(Value::as_str)
                                .unwrap_or("SMPTE_Dissolve");
                            let raw_name = child
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .trim();
                            let (tr_name, effect_id) =
                                map_otio_transition(raw_name, tr_type, kind);
                            transitions.push(Transition {
                                name: tr_name,
                                effect_id,
                                start_frame,
                                end_frame,
                                alignment: alignment.to_string(),
                            });
                        }
                    }
                    Some("Clip") => {
                        if let Some((clip, dur_frames)) =
                            self.parse_clip(child, kind, cursor, seq_fps, seq_w, seq_h)
                        {
                            clips.push(clip);
                            cursor += dur_frames;
                        }
                    }
                    Some("Stack") | Some("Timeline") => {
                        let nested_fallback = format!("Compound_{}", self.extra_sequences.len() + 1);
                        let nested_seq = if schema_base(child) == Some("Timeline") {
                            self.parse_timeline(child, SequenceKind::CompoundClip, &nested_fallback)
                        } else {
                            self.parse_stack_as_sequence(
                                child,
                                SequenceKind::CompoundClip,
                                &nested_fallback,
                                seq_fps,
                                seq_w,
                                seq_h,
                            )
                        };
                        let src_start = child
                            .pointer("/source_range/start_time")
                            .and_then(OtioTime::from_json)
                            .map(|t| t.to_frames(seq_fps).max(0))
                            .unwrap_or(0);
                        let dur_frames = child
                            .pointer("/source_range/duration")
                            .and_then(OtioTime::from_json)
                            .map(|t| t.to_frames(seq_fps).max(1))
                            .unwrap_or_else(|| nested_seq.computed_duration_frames().max(1));
                        let nested_id = nested_seq.id.clone();
                        let nested_name = nested_seq.name.clone();
                        self.extra_sequences.push(nested_seq);

                        let clip = Clip {
                            id: self.alloc_clip_id(),
                            name: nested_name,
                            media_id: nested_id,
                            media_path: String::new(),
                            media_kind: if kind == TrackKind::Audio {
                                MediaKind::Audio
                            } else {
                                MediaKind::Video
                            },
                            source_kind: ClipSourceKind::CompoundClip,
                            active_angle: None,
                            start_frame: cursor,
                            end_frame: cursor + dur_frames,
                            source_in_frame: src_start,
                            source_out_frame: src_start + dur_frames,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: seq_fps,
                            source_width: if kind == TrackKind::Video { seq_w } else { 0 },
                            source_height: if kind == TrackKind::Video { seq_h } else { 0 },
                            speed_percent: 100.0,
                            reversed: false,
                            speed_keyframes: Vec::new(),
                            retime_interpolation: RetimeInterpolation::Default,
                            enabled: child.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                            audio_channel: if kind == TrackKind::Audio { 1 } else { 0 },
                            link_group_id: None,
                            label_name: None,
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            blend_mode: BlendMode::Normal,
                            lumetri_effects: Vec::new(),
                            effects: Vec::new(),
                            markers: Vec::new(),
                        };
                        clips.push(clip);
                        cursor += dur_frames;
                    }
                    _ => {}
                }
            }
        }

        let mut track_markers = Vec::new();
        if let Some(mk_arr) = node.get("markers").and_then(Value::as_array) {
            for m in mk_arr {
                if let Some(parsed) = parse_marker(m, seq_fps, MarkerScope::Track, 0) {
                    track_markers.push(parsed);
                }
            }
        }

        Track {
            index,
            kind,
            name,
            enabled,
            locked: false,
            muted: false,
            solo: false,
            audio_channels,
            output_bus_id: None,
            clips,
            transitions,
            markers: track_markers,
        }
    }

    fn parse_clip(
        &mut self,
        node: &Value,
        track_kind: TrackKind,
        cursor: i64,
        seq_fps: Rational,
        seq_w: u32,
        seq_h: u32,
    ) -> Option<(Clip, i64)> {
        let media_ref = active_media_reference(node);
        let ref_info = parse_media_reference(media_ref, track_kind, self.base_dir, seq_fps, seq_w, seq_h);

        let name = node
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .or_else(|| (!ref_info.name.is_empty()).then(|| ref_info.name.clone()))
            .or_else(|| {
                Path::new(&ref_info.path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| "Clip".to_string());

        let src_start_otio = node
            .pointer("/source_range/start_time")
            .and_then(OtioTime::from_json);
        let src_dur_otio = node
            .pointer("/source_range/duration")
            .and_then(OtioTime::from_json);

        let tl_dur_frames = src_dur_otio
            .map(|t| t.to_frames(seq_fps))
            .or_else(|| {
                ref_info
                    .avail_dur_frames
                    .filter(|&d| d > 0)
                    .map(|d| convert_frames(d, ref_info.source_fps, seq_fps))
            })
            .unwrap_or(0);
        if tl_dur_frames <= 0 {
            return None;
        }

        let source_fps = if ref_info.has_explicit_fps {
            ref_info.source_fps
        } else if let Some(st) = src_start_otio.filter(|t| t.rate > 0.0) {
            Rational::from_fps_approx(st.rate)
        } else {
            seq_fps
        };

        let asset_tc = ref_info.source_tc_frame;
        let raw_src_start = src_start_otio
            .map(|t| t.to_frames(source_fps))
            .unwrap_or(asset_tc);
        let source_in_frame = if ref_info.has_available_range && raw_src_start >= asset_tc {
            (raw_src_start - asset_tc).max(0)
        } else {
            raw_src_start.max(0)
        };

        // Parse effects: FreezeFrame, LinearTimeWarp, and any custom Effects
        let mut speed_percent = 100.0f64;
        let mut reversed = false;
        let mut is_freeze = false;
        let mut effects = Vec::new();

        if let Some(eff_arr) = node.get("effects").and_then(Value::as_array) {
            for (ei, eff) in eff_arr.iter().enumerate() {
                match schema_base(eff) {
                    Some("FreezeFrame") => {
                        is_freeze = true;
                        speed_percent = 0.0;
                    }
                    Some("LinearTimeWarp") => {
                        if let Some(scalar) = eff.get("time_scalar").and_then(Value::as_f64) {
                            if scalar.abs() < 1e-4 {
                                is_freeze = true;
                                speed_percent = 0.0;
                            } else {
                                reversed = scalar < 0.0;
                                speed_percent = scalar.abs() * 100.0;
                            }
                        }
                    }
                    Some("TimeEffect") => {}
                    Some(_) => {
                        let eff_name = eff
                            .get("effect_name")
                            .and_then(Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| eff.get("name").and_then(Value::as_str))
                            .unwrap_or("OTIO Effect");
                        let display_name = eff
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or(eff_name);
                        let mut params = Vec::new();
                        if let Some(meta_obj) = eff.get("metadata").and_then(Value::as_object) {
                            for (k, v) in meta_obj {
                                let val_str = match v {
                                    Value::String(s) => s.clone(),
                                    Value::Number(n) => n.to_string(),
                                    Value::Bool(b) => b.to_string(),
                                    _ => continue,
                                };
                                params.push(EffectParameter {
                                    id: k.clone(),
                                    name: k.clone(),
                                    value: val_str,
                                    is_animated: false,
                                });
                            }
                        }
                        effects.push(EffectInstance {
                            id: format!("otio-fx-{}", ei + 1),
                            match_name: eff_name.to_string(),
                            display_name: display_name.to_string(),
                            category: guess_effect_category(eff_name, display_name, track_kind),
                            media_kind: track_kind,
                            enabled: true,
                            is_intrinsic: false,
                            is_animated: false,
                            parameters: params,
                        });
                    }
                    None => {}
                }
            }
        }

        let tl_dur_in_src = convert_frames(tl_dur_frames, seq_fps, source_fps).max(1);
        let source_out_frame = if is_freeze {
            source_in_frame
        } else {
            let consumed = ((tl_dur_in_src as f64) * (speed_percent / 100.0))
                .round()
                .max(1.0) as i64;
            source_in_frame + consumed
        };

        let media_id = self.register_asset(
            &name,
            &ref_info.path,
            ref_info.media_kind,
            ref_info.width,
            ref_info.height,
            source_fps,
            ref_info
                .avail_dur_frames
                .unwrap_or(source_out_frame)
                .max(source_out_frame),
            asset_tc,
            ref_info.audio_channels,
        );

        let (motion, opacity, blend_mode, label_name, link_group_id) =
            parse_clip_metadata(node);

        let mut clip_markers = Vec::new();
        if let Some(mk_arr) = node.get("markers").and_then(Value::as_array) {
            let subtract_tc = if ref_info.has_available_range {
                asset_tc
            } else {
                0
            };
            for m in mk_arr {
                if let Some(parsed) = parse_marker(m, source_fps, MarkerScope::Clip, subtract_tc) {
                    clip_markers.push(parsed);
                }
            }
        }

        let clip = Clip {
            id: self.alloc_clip_id(),
            name,
            media_id,
            media_path: ref_info.path,
            media_kind: ref_info.media_kind,
            source_kind: ClipSourceKind::Media,
            active_angle: None,
            start_frame: cursor,
            end_frame: cursor + tl_dur_frames,
            source_in_frame,
            source_out_frame,
            source_in_ticks: None,
            source_out_ticks: None,
            source_fps,
            source_width: ref_info.width,
            source_height: ref_info.height,
            speed_percent,
            reversed,
            speed_keyframes: Vec::new(),
            retime_interpolation: RetimeInterpolation::Default,
            enabled: node.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            audio_channel: if track_kind == TrackKind::Audio { 1 } else { 0 },
            link_group_id,
            label_name,
            label_preset_index: None,
            motion,
            opacity,
            blend_mode,
            lumetri_effects: Vec::new(),
            effects,
            markers: clip_markers,
        };

        Some((clip, tl_dur_frames))
    }

    #[allow(clippy::too_many_arguments)]
    fn register_asset(
        &mut self,
        name: &str,
        path: &str,
        media_kind: MediaKind,
        width: u32,
        height: u32,
        fps: Rational,
        duration_frames: i64,
        source_tc_frame: i64,
        audio_channels: u32,
    ) -> String {
        let key_str = if path.is_empty() {
            format!("offline:{name}")
        } else {
            path.to_string()
        };
        if let Some(&idx) = self.asset_index_by_key.get(&(key_str.clone(), media_kind)) {
            let asset = &mut self.media_assets[idx];
            if duration_frames > asset.duration_frames {
                asset.duration_frames = duration_frames;
            }
            if asset.source_tc_frame == 0 && source_tc_frame > 0 {
                asset.source_tc_frame = source_tc_frame;
                asset.source_tc_string = frames_to_timecode(source_tc_frame, asset.frame_rate);
            }
            return asset.id.clone();
        }
        // Also share asset when a video file is referenced on both video and audio tracks
        if media_kind == MediaKind::Audio
            && !path.is_empty()
            && let Some(&idx) = self
                .asset_index_by_key
                .get(&(key_str.clone(), MediaKind::Video))
        {
            let asset = &mut self.media_assets[idx];
            if asset.audio_channels == 0 {
                asset.audio_channels = audio_channels.max(2);
                asset.audio_sample_rate = 48_000;
            }
            return asset.id.clone();
        }

        let id = format!("otio-asset-{}", self.media_assets.len() + 1);
        let asset_name = if !path.is_empty() {
            Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(name)
                .to_string()
        } else {
            name.to_string()
        };
        let asset = MediaAsset {
            id: id.clone(),
            name: asset_name,
            path: path.to_string(),
            media_kind,
            width,
            height,
            frame_rate: fps,
            duration_frames: duration_frames.max(0),
            source_tc_frame: source_tc_frame.max(0),
            source_tc_string: frames_to_timecode(source_tc_frame.max(0), fps),
            audio_channels,
            audio_sample_rate: if audio_channels > 0 { 48_000 } else { 0 },
            color_space: None,
            video_codec: None,
            audio_codec: None,
            alpha_mode: AlphaMode::Default,
            data_levels: DataLevels::Auto,
            pixel_aspect_ratio: None,
            metadata: BTreeMap::new(),
            custom_metadata: BTreeMap::new(),
        };
        let idx = self.media_assets.len();
        self.media_assets.push(asset);
        self.asset_index_by_key.insert((key_str, media_kind), idx);
        id
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct OtioTime {
    value: f64,
    rate: f64,
}

impl OtioTime {
    fn from_json(v: &Value) -> Option<Self> {
        let value = v.get("value").and_then(Value::as_f64)?;
        let rate = v.get("rate").and_then(Value::as_f64)?;
        if !value.is_finite() || !rate.is_finite() || rate <= 0.0 {
            return None;
        }
        Some(Self { value, rate })
    }

    fn to_frames(self, target_fps: Rational) -> i64 {
        let target = target_fps.as_f64();
        if (self.rate - target).abs() < 0.015 {
            self.value.round() as i64
        } else {
            (self.value * target / self.rate).round() as i64
        }
    }
}

struct ParsedMediaRef {
    name: String,
    path: String,
    media_kind: MediaKind,
    width: u32,
    height: u32,
    source_fps: Rational,
    has_explicit_fps: bool,
    source_tc_frame: i64,
    avail_dur_frames: Option<i64>,
    has_available_range: bool,
    audio_channels: u32,
}

fn schema_base(v: &Value) -> Option<&str> {
    let raw = v.get("OTIO_SCHEMA")?.as_str()?;
    Some(raw.split('.').next().unwrap_or(raw))
}

fn active_media_reference(clip: &Value) -> Option<&Value> {
    if let Some(refs) = clip.get("media_references").and_then(Value::as_object) {
        let active_key = clip
            .get("active_media_reference_key")
            .and_then(Value::as_str)
            .unwrap_or("DEFAULT_MEDIA");
        if let Some(found) = refs.get(active_key) {
            return Some(found);
        }
        if let Some(first) = refs.values().next() {
            return Some(first);
        }
    }
    clip.get("media_reference")
}

fn parse_media_reference(
    media_ref: Option<&Value>,
    track_kind: TrackKind,
    base_dir: Option<&Path>,
    seq_fps: Rational,
    seq_w: u32,
    seq_h: u32,
) -> ParsedMediaRef {
    let default_kind = if track_kind == TrackKind::Audio {
        MediaKind::Audio
    } else {
        MediaKind::Video
    };
    let mut out = ParsedMediaRef {
        name: String::new(),
        path: String::new(),
        media_kind: default_kind,
        width: if track_kind == TrackKind::Video { seq_w } else { 0 },
        height: if track_kind == TrackKind::Video { seq_h } else { 0 },
        source_fps: seq_fps,
        has_explicit_fps: false,
        source_tc_frame: 0,
        avail_dur_frames: None,
        has_available_range: false,
        audio_channels: if track_kind == TrackKind::Audio { 2 } else { 0 },
    };

    let Some(mref) = media_ref else {
        return out;
    };

    if let Some(n) = mref.get("name").and_then(Value::as_str) {
        out.name = n.to_string();
    }

    if let Some(avail) = mref.get("available_range") {
        let start = avail.get("start_time").and_then(OtioTime::from_json);
        let dur = avail.get("duration").and_then(OtioTime::from_json);
        if let Some(rate) = dur.or(start).map(|t| t.rate).filter(|&r| r > 0.0) {
            out.source_fps = Rational::from_fps_approx(rate);
            out.has_explicit_fps = true;
        }
        if let Some(st) = start {
            out.source_tc_frame = st.to_frames(out.source_fps).max(0);
            out.has_available_range = true;
        }
        if let Some(du) = dur {
            out.avail_dur_frames = Some(du.to_frames(out.source_fps).max(0));
            out.has_available_range = true;
        }
    }

    match schema_base(mref) {
        Some("ExternalReference") => {
            if let Some(raw_url) = mref
                .get("target_url")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                let decoded = file_url_to_path(raw_url);
                out.path = resolve_possibly_relative_path(&decoded, base_dir);
                out.media_kind = infer_media_kind_from_path(&out.path, track_kind);
            }
        }
        Some("ImageSequenceReference") => {
            let base = mref
                .get("target_url_base")
                .and_then(Value::as_str)
                .unwrap_or("");
            let prefix = mref
                .get("name_prefix")
                .and_then(Value::as_str)
                .unwrap_or("");
            let suffix = mref
                .get("name_suffix")
                .and_then(Value::as_str)
                .unwrap_or("");
            let start_frame = mref
                .get("start_frame")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let pad = mref
                .get("frame_zero_padding")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let frame_str = if pad > 0 {
                format!("{start_frame:0pad$}")
            } else {
                start_frame.to_string()
            };
            let decoded_base = file_url_to_path(base);
            let joined = if decoded_base.is_empty() {
                format!("{prefix}{frame_str}{suffix}")
            } else {
                format!(
                    "{}/{prefix}{frame_str}{suffix}",
                    decoded_base.trim_end_matches('/')
                )
            };
            out.path = resolve_possibly_relative_path(&joined, base_dir);
            out.media_kind = MediaKind::Image;
        }
        _ => {}
    }

    // Check available_image_bounds for pixel dimensions if sensible
    if let Some((w, h)) = parse_image_bounds(mref.get("available_image_bounds")) {
        out.width = w;
        out.height = h;
    }

    out
}

fn resolve_possibly_relative_path(path: &str, base_dir: Option<&Path>) -> String {
    let p = path.trim();
    if p.is_empty() {
        return String::new();
    }
    let bytes = p.as_bytes();
    let is_windows_abs =
        bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if p.starts_with('/') || p.starts_with("\\\\") || is_windows_abs {
        return p.to_string();
    }
    if let Some(dir) = base_dir {
        return dir.join(p).to_string_lossy().into_owned();
    }
    p.to_string()
}

fn infer_media_kind_from_path(path: &str, track_kind: TrackKind) -> MediaKind {
    if let Some(ext) = Path::new(path).extension().and_then(|s| s.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "tif" | "tiff" | "psd" | "bmp" | "webp" | "heic" | "exr"
            | "dpx" | "tga" => return MediaKind::Image,
            "wav" | "aif" | "aiff" | "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "bwf" => {
                return MediaKind::Audio;
            }
            _ => {}
        }
    }
    if track_kind == TrackKind::Audio {
        MediaKind::Audio
    } else {
        MediaKind::Video
    }
}

fn parse_image_bounds(bounds: Option<&Value>) -> Option<(u32, u32)> {
    let b = bounds?;
    let min_x = b.pointer("/min/x").and_then(Value::as_f64)?;
    let min_y = b.pointer("/min/y").and_then(Value::as_f64)?;
    let max_x = b.pointer("/max/x").and_then(Value::as_f64)?;
    let max_y = b.pointer("/max/y").and_then(Value::as_f64)?;
    let w = (max_x - min_x).abs().round() as u32;
    let h = (max_y - min_y).abs().round() as u32;
    if w >= 64 && h >= 64 {
        Some((w, h))
    } else {
        None
    }
}

fn infer_resolution_from_timeline(timeline: &Value) -> (u32, u32) {
    if let (Some(w), Some(h)) = (
        timeline
            .pointer("/metadata/aagedal_nle_converter/width")
            .and_then(Value::as_u64),
        timeline
            .pointer("/metadata/aagedal_nle_converter/height")
            .and_then(Value::as_u64),
    ) && w >= 64
        && h >= 64
    {
        return (w as u32, h as u32);
    }

    // Scan tracks for first clip with available_image_bounds
    if let Some(tracks) = timeline.pointer("/tracks/children").and_then(Value::as_array) {
        for tr in tracks {
            if let Some(items) = tr.get("children").and_then(Value::as_array) {
                for item in items {
                    if let Some(mref) = active_media_reference(item)
                        && let Some(dims) =
                            parse_image_bounds(mref.get("available_image_bounds"))
                    {
                        return dims;
                    }
                }
            }
        }
    }
    (1920, 1080)
}

fn infer_fps_from_node(node: &Value) -> Option<Rational> {
    if let Some(rt) = node
        .pointer("/source_range/duration")
        .or_else(|| node.pointer("/source_range/start_time"))
        .or_else(|| node.get("in_offset"))
        .and_then(OtioTime::from_json)
    {
        return Some(Rational::from_fps_approx(rt.rate));
    }
    if let Some(mref) = active_media_reference(node)
        && let Some(rt) = mref
            .pointer("/available_range/duration")
            .or_else(|| mref.pointer("/available_range/start_time"))
            .and_then(OtioTime::from_json)
    {
        return Some(Rational::from_fps_approx(rt.rate));
    }
    if let Some(tracks) = node.get("tracks")
        && let Some(fps) = infer_fps_from_node(tracks)
    {
        return Some(fps);
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for ch in children {
            if let Some(fps) = infer_fps_from_node(ch) {
                return Some(fps);
            }
        }
    }
    None
}

fn parse_marker(
    node: &Value,
    fps: Rational,
    scope: MarkerScope,
    subtract_tc: i64,
) -> Option<Marker> {
    let start = node
        .pointer("/marked_range/start_time")
        .and_then(OtioTime::from_json)?;
    let dur = node
        .pointer("/marked_range/duration")
        .and_then(OtioTime::from_json);

    let raw_frame = start.to_frames(fps);
    let frame = if subtract_tc > 0 && raw_frame >= subtract_tc {
        (raw_frame - subtract_tc).max(0)
    } else {
        raw_frame.max(0)
    };
    let duration_frames = dur.map(|d| d.to_frames(fps).max(1)).unwrap_or(1);
    let name = node
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let comment = node
        .get("comment")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let color = normalize_otio_color(
        node.get("color")
            .and_then(Value::as_str)
            .unwrap_or("BLUE"),
    );

    Some(Marker {
        frame,
        duration_frames,
        name,
        comment,
        color,
        scope,
    })
}

fn normalize_otio_color(raw: &str) -> String {
    match raw.trim().to_ascii_uppercase().as_str() {
        "RED" => "Red".into(),
        "GREEN" => "Green".into(),
        "BLUE" => "Blue".into(),
        "CYAN" => "Cyan".into(),
        "YELLOW" => "Yellow".into(),
        "ORANGE" => "Orange".into(),
        "PURPLE" => "Purple".into(),
        "MAGENTA" | "PINK" => "Pink".into(),
        "WHITE" => "White".into(),
        "BLACK" => "Black".into(),
        other if !other.is_empty() => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => {
                    format!("{}{}", first, chars.as_str().to_ascii_lowercase())
                }
                None => "Blue".into(),
            }
        }
        _ => "Blue".into(),
    }
}

fn parse_clip_metadata(
    node: &Value,
) -> (
    MotionTransform,
    f64,
    BlendMode,
    Option<String>,
    Option<String>,
) {
    let mut motion = MotionTransform::default();
    let mut opacity = 100.0;
    let mut blend_mode = BlendMode::Normal;
    let mut label_name: Option<String> = None;
    let mut link_group_id: Option<String> = None;

    if let Some(meta) = node.pointer("/metadata/aagedal_nle_converter") {
        if let Some(m) = meta.get("motion") {
            if let Some(v) = m.get("scale").and_then(Value::as_f64) {
                motion.scale = v;
            }
            if let Some(v) = m.get("scale_y").and_then(Value::as_f64) {
                motion.scale_y = Some(v);
            }
            if let Some(v) = m.get("position_x").and_then(Value::as_f64) {
                motion.position_x = v;
            }
            if let Some(v) = m.get("position_y").and_then(Value::as_f64) {
                motion.position_y = v;
            }
            if let Some(v) = m.get("rotation").and_then(Value::as_f64) {
                motion.rotation = v;
            }
            if let Some(v) = m.get("anchor_x").and_then(Value::as_f64) {
                motion.anchor_x = v;
            }
            if let Some(v) = m.get("anchor_y").and_then(Value::as_f64) {
                motion.anchor_y = v;
            }
            if let Some(v) = m.get("crop_left").and_then(Value::as_f64) {
                motion.crop_left = v;
            }
            if let Some(v) = m.get("crop_right").and_then(Value::as_f64) {
                motion.crop_right = v;
            }
            if let Some(v) = m.get("crop_top").and_then(Value::as_f64) {
                motion.crop_top = v;
            }
            if let Some(v) = m.get("crop_bottom").and_then(Value::as_f64) {
                motion.crop_bottom = v;
            }
        }
        if let Some(v) = meta.get("opacity").and_then(Value::as_f64) {
            opacity = v.clamp(0.0, 100.0);
        }
        if let Some(bm) = meta
            .get("blend_mode")
            .and_then(Value::as_str)
            .and_then(BlendMode::parse_name)
        {
            blend_mode = bm;
        }
        if let Some(lbl) = meta
            .get("label_name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            label_name = Some(lbl.to_string());
        }
        if let Some(lg) = meta
            .get("link_group_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            link_group_id = Some(lg.to_string());
        }
    }

    // Fallback: Resolve OTIO clip color metadata if present
    if label_name.is_none()
        && let Some(col) = node
            .pointer("/metadata/Resolve_OTIO/Color")
            .or_else(|| node.pointer("/metadata/Resolve_OTIO/ClipColor"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    {
        label_name = Some(col.to_string());
    }

    (motion, opacity, blend_mode, label_name, link_group_id)
}

fn map_otio_transition(raw_name: &str, tr_type: &str, track_kind: TrackKind) -> (String, String) {
    let lower_type = tr_type.to_ascii_lowercase();
    if lower_type == "smpte_dissolve" || lower_type.is_empty() {
        if track_kind == TrackKind::Audio {
            let name = if raw_name.is_empty() {
                "Constant Power".to_string()
            } else {
                raw_name.to_string()
            };
            (name, "Constant Power".to_string())
        } else {
            let name = if raw_name.is_empty() {
                "Cross Dissolve".to_string()
            } else {
                raw_name.to_string()
            };
            (name, "AE.ADBE Cross Dissolve".to_string())
        }
    } else {
        let name = if raw_name.is_empty() {
            tr_type.to_string()
        } else {
            raw_name.to_string()
        };
        (name, tr_type.to_string())
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

fn guess_effect_category(match_name: &str, display_name: &str, kind: TrackKind) -> EffectCategory {
    if kind == TrackKind::Audio {
        return EffectCategory::Audio;
    }
    let s = format!(
        "{} {}",
        match_name.to_ascii_lowercase(),
        display_name.to_ascii_lowercase()
    );
    if s.contains("blur") {
        EffectCategory::Blur
    } else if s.contains("key") {
        EffectCategory::Keying
    } else if s.contains("stabiliz") {
        EffectCategory::Stabilization
    } else if s.contains("color") || s.contains("lumetri") || s.contains("grade") || s.contains("lut") {
        EffectCategory::Color
    } else if s.contains("transform") || s.contains("crop") || s.contains("motion") {
        EffectCategory::Transform
    } else if s.contains("distort") || s.contains("warp") || s.contains("pin") {
        EffectCategory::Distort
    } else if s.contains("title") || s.contains("text") {
        EffectCategory::Title
    } else {
        EffectCategory::Other
    }
}
