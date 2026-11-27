use super::cfb::{CfbFile, CfbObjType};
use super::codec::*;
use crate::fcpxml::time::file_url_to_path;
use nle_core::{
    frames_to_timecode, AlphaMode, BlendMode, Clip, ClipSourceKind, ColorConfig, DataLevels,
    EffectCategory, EffectInstance, EffectParameter, Marker, MarkerScope, MediaAsset, MediaKind,
    MotionTransform, NleError, NleFormat, NleResult, Project, ProjectReader, Rational,
    RetimeInterpolation, Sequence, SequenceKind, Track, TrackKind, Transition,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use uuid::Uuid;

pub struct AafReader;

impl ProjectReader for AafReader {
    fn format(&self) -> NleFormat {
        NleFormat::Aaf
    }

    fn read_file(&self, path: &Path) -> NleResult<Project> {
        read_aaf_file(path)
    }
}

/// Read an Advanced Authoring Format (`.aaf` Structured Storage) file into the canonical `Project` IR.
pub fn read_aaf_file(path: &Path) -> NleResult<Project> {
    let bytes = fs::read(path)?;
    let default_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("AAF Project");
    let mut project = parse_aaf_bytes(&bytes, default_name)?;
    project.source_path = Some(path.to_string_lossy().into_owned());
    Ok(project)
}

/// Parse raw `.aaf` (OLE2/CFB Structured Storage) bytes into the canonical `Project` IR.
pub fn parse_aaf_bytes(bytes: &[u8], default_name: &str) -> NleResult<Project> {
    let cfb = CfbFile::parse(bytes)?;
    let root_id = cfb.root_id();
    let root_props = read_obj_properties(&cfb, root_id)?;

    let header_id = resolve_strong_ref(&cfb, root_id, &root_props, PID_ROOT_HEADER)
        .or_else(|| cfb.find_child(root_id, "Header-2"))
        .ok_or_else(|| {
            NleError::InvalidFormat("AAF root storage is missing Header object".into())
        })?;
    let header_props = read_obj_properties(&cfb, header_id)?;

    // Parse Dictionary definitions (DataDefinitions & OperationDefinitions AUID -> Name)
    let mut data_def_names: HashMap<[u8; 16], String> = HashMap::new();
    let mut op_def_names: HashMap<[u8; 16], String> = HashMap::new();
    if let Some(dict_id) = resolve_strong_ref(&cfb, header_id, &header_props, PID_HEADER_DICTIONARY)
        && let Ok(dict_props) = read_obj_properties(&cfb, dict_id)
    {
        for dd_id in resolve_strong_ref_collection(&cfb, dict_id, &dict_props, PID_DICT_DATA_DEFS) {
            if let Ok(p) = read_obj_properties(&cfb, dd_id)
                && let (Some(auid), Some(name)) =
                    (p.get_auid(PID_DEF_IDENTIFICATION), p.get_string(PID_DEF_NAME))
            {
                data_def_names.insert(auid, name);
            }
        }
        for od_id in
            resolve_strong_ref_collection(&cfb, dict_id, &dict_props, PID_DICT_OPERATION_DEFS)
        {
            if let Ok(p) = read_obj_properties(&cfb, od_id)
                && let (Some(auid), Some(name)) =
                    (p.get_auid(PID_DEF_IDENTIFICATION), p.get_string(PID_DEF_NAME))
            {
                op_def_names.insert(auid, name);
            }
        }
    }

    let content_id = resolve_strong_ref(&cfb, header_id, &header_props, PID_HEADER_CONTENT)
        .or_else(|| cfb.find_child(header_id, "Content-3b03"))
        .ok_or_else(|| {
            NleError::InvalidFormat("AAF Header is missing ContentStorage object".into())
        })?;
    let content_props = read_obj_properties(&cfb, content_id)?;

    // Collect all Mobs indexed by 32-byte MobID
    let mob_ids_storage =
        resolve_strong_ref_collection(&cfb, content_id, &content_props, PID_CONTENT_MOBS);
    let mut mobs_by_id: HashMap<[u8; 32], ParsedMob> = HashMap::new();
    let mut comp_mob_ids: Vec<[u8; 32]> = Vec::new();

    for sid in mob_ids_storage {
        let Some(entry) = cfb.entry(sid) else {
            continue;
        };
        let Ok(props) = read_obj_properties(&cfb, sid) else {
            continue;
        };
        let Some(mob_id) = props.get_mob_id(PID_MOB_ID) else {
            continue;
        };
        let class_code = smpte_class_code(&entry.clsid).unwrap_or(0);
        let mob_kind = match class_code {
            CLASS_COMPOSITION_MOB => MobKind::Composition,
            CLASS_MASTER_MOB => MobKind::Master,
            CLASS_SOURCE_MOB => MobKind::Source,
            _ => {
                // Fallback heuristic if custom subclass
                if props.get(PID_SOURCE_MOB_ESSENCE_DESC).is_some() {
                    MobKind::Source
                } else {
                    MobKind::Composition
                }
            }
        };
        let name = props.get_string(PID_MOB_NAME).unwrap_or_default();
        let is_top_level = props
            .get_auid(PID_MOB_USAGE_CODE)
            .map(|u| u == USAGE_TOP_LEVEL)
            .unwrap_or(false);

        let descriptor = if mob_kind == MobKind::Source {
            resolve_strong_ref(&cfb, sid, &props, PID_SOURCE_MOB_ESSENCE_DESC)
                .map(|did| parse_essence_descriptor(&cfb, did))
        } else {
            None
        };

        let slot_storages = resolve_strong_ref_collection(&cfb, sid, &props, PID_MOB_SLOTS);
        let mut slots = Vec::new();
        for slot_sid in slot_storages {
            if let Some(slot) = parse_mob_slot(&cfb, slot_sid, &data_def_names) {
                slots.push(slot);
            }
        }

        if mob_kind == MobKind::Composition {
            comp_mob_ids.push(mob_id);
        }
        mobs_by_id.insert(
            mob_id,
            ParsedMob {
                storage_id: sid,
                kind: mob_kind,
                name,
                is_top_level,
                slots,
                descriptor,
            },
        );
    }

    // Identify which CompositionMobs are referenced as nested clips inside other CompositionMobs
    let mut nested_comp_ids: HashSet<[u8; 32]> = HashSet::new();
    for &cid in &comp_mob_ids {
        if let Some(mob) = mobs_by_id.get(&cid) {
            collect_referenced_comp_mobs(&cfb, mob.storage_id, &mobs_by_id, &mut nested_comp_ids);
        }
    }

    // Assign stable sequence IDs to every CompositionMob
    let mut comp_seq_uuid: HashMap<[u8; 32], String> = HashMap::new();
    for &cid in &comp_mob_ids {
        comp_seq_uuid.insert(cid, Uuid::new_v4().to_string());
    }

    let mut ctx = AafBuildCtx {
        cfb: &cfb,
        mobs_by_id: &mobs_by_id,
        comp_seq_uuid: &comp_seq_uuid,
        op_def_names: &op_def_names,
        media_assets: Vec::new(),
        asset_idx_by_key: HashMap::new(),
        next_clip_id: 1,
    };

    // Order top-level CompositionMobs first, then nested ones
    let mut ordered_comps = comp_mob_ids.clone();
    ordered_comps.sort_by_key(|id| {
        let m = &mobs_by_id[id];
        if m.is_top_level && !nested_comp_ids.contains(id) {
            0
        } else if !nested_comp_ids.contains(id) {
            1
        } else {
            2
        }
    });

    let mut sequences: Vec<Sequence> = Vec::new();
    for cid in ordered_comps {
        let is_nested = nested_comp_ids.contains(&cid) && !mobs_by_id[&cid].is_top_level;
        let kind = if is_nested {
            SequenceKind::CompoundClip
        } else {
            SequenceKind::Timeline
        };
        if let Some(seq) = ctx.build_sequence(cid, kind) {
            sequences.push(seq);
        }
    }

    let project_name = if default_name.is_empty() || default_name == "AAF Project" {
        sequences
            .first()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "AAF Project".to_string())
    } else {
        default_name.to_string()
    };

    let mut metadata = BTreeMap::new();
    metadata.insert("aaf_format".into(), "StructuredStorage".into());

    Ok(Project {
        name: project_name,
        source_format: NleFormat::Aaf,
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

// ── Internal Parsed AAF Structures ──────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MobKind {
    Composition,
    Master,
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotMediaKind {
    Video,
    Audio,
    Timecode,
    Event,
    Other,
}

#[derive(Debug, Clone)]
struct ParsedSlot {
    slot_id: u32,
    name: String,
    edit_rate: Rational,
    origin: i64,
    media_kind: SlotMediaKind,
    segment_storage_id: Option<usize>,
    is_event_slot: bool,
}

#[derive(Debug, Clone, Default)]
struct ParsedDescriptor {
    urls: Vec<String>,
    width: u32,
    height: u32,
    sample_rate: Option<Rational>,
    length: i64,
    audio_channels: u32,
    audio_sample_rate: u32,
    is_tape: bool,
    has_video: bool,
    has_audio: bool,
}

struct ParsedMob {
    storage_id: usize,
    kind: MobKind,
    name: String,
    is_top_level: bool,
    slots: Vec<ParsedSlot>,
    descriptor: Option<ParsedDescriptor>,
}

struct ResolvedSource {
    name: String,
    path: String,
    media_kind: MediaKind,
    width: u32,
    height: u32,
    fps: Rational,
    duration_frames: i64,
    source_tc_frame: i64,
    audio_channels: u32,
    audio_sample_rate: u32,
    tape_name: Option<String>,
    source_in_offset: i64,
}

struct AafBuildCtx<'a> {
    cfb: &'a CfbFile<'a>,
    mobs_by_id: &'a HashMap<[u8; 32], ParsedMob>,
    comp_seq_uuid: &'a HashMap<[u8; 32], String>,
    op_def_names: &'a HashMap<[u8; 16], String>,
    media_assets: Vec<MediaAsset>,
    asset_idx_by_key: HashMap<String, usize>,
    next_clip_id: usize,
}

impl<'a> AafBuildCtx<'a> {
    fn alloc_clip_id(&mut self) -> String {
        let id = format!("aaf-clip-{}", self.next_clip_id);
        self.next_clip_id += 1;
        id
    }

    fn build_sequence(&mut self, comp_mob_id: [u8; 32], kind: SequenceKind) -> Option<Sequence> {
        let mob = self.mobs_by_id.get(&comp_mob_id)?;
        let seq_id = self
            .comp_seq_uuid
            .get(&comp_mob_id)
            .cloned()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let name = if mob.name.is_empty() {
            "Untitled Sequence".to_string()
        } else {
            mob.name.clone()
        };

        // Determine sequence FPS and start_tc_frame from Timecode or first Video TimelineMobSlot
        let mut seq_fps: Option<Rational> = None;
        let mut start_tc_frame: i64 = 0;

        for slot in &mob.slots {
            if slot.media_kind == SlotMediaKind::Video && seq_fps.is_none() {
                seq_fps = Some(slot.edit_rate);
            }
            if slot.media_kind == SlotMediaKind::Timecode
                && let Some(seg_sid) = slot.segment_storage_id
                && let Some((tc_start, tc_fps)) = extract_timecode_segment(self.cfb, seg_sid)
            {
                start_tc_frame = tc_start.max(0);
                if seq_fps.is_none() {
                    seq_fps = Some(slot.edit_rate);
                    if seq_fps == Some(Rational::ZERO) {
                        seq_fps = Some(tc_fps);
                    }
                }
            }
        }
        let seq_fps = seq_fps
            .or_else(|| mob.slots.first().map(|s| s.edit_rate))
            .filter(|r| r.num > 0 && r.den > 0)
            .unwrap_or(Rational::FPS_25);

        let mut seq_width = 1920u32;
        let mut seq_height = 1080u32;
        let mut found_res = false;
        let mut video_tracks = Vec::new();
        let mut audio_tracks = Vec::new();
        let mut markers = Vec::new();

        for slot in &mob.slots {
            let Some(seg_sid) = slot.segment_storage_id else {
                continue;
            };
            if slot.is_event_slot || slot.media_kind == SlotMediaKind::Event {
                self.collect_event_markers(seg_sid, slot.edit_rate, seq_fps, &mut markers);
                continue;
            }
            match slot.media_kind {
                SlotMediaKind::Video | SlotMediaKind::Audio => {
                    let track_kind = if slot.media_kind == SlotMediaKind::Video {
                        TrackKind::Video
                    } else {
                        TrackKind::Audio
                    };
                    let idx = if track_kind == TrackKind::Video {
                        video_tracks.len() + 1
                    } else {
                        audio_tracks.len() + 1
                    };
                    let track_name = if !slot.name.is_empty() {
                        slot.name.clone()
                    } else if track_kind == TrackKind::Video {
                        format!("V{idx}")
                    } else {
                        format!("A{idx}")
                    };

                    let mut clips = Vec::new();
                    let mut transitions = Vec::new();
                    let mut cursor = slot.origin.max(0);
                    self.walk_segment(
                        seg_sid,
                        track_kind,
                        slot.edit_rate,
                        seq_fps,
                        &mut cursor,
                        &mut clips,
                        &mut transitions,
                        None,
                        &[],
                    );

                    if track_kind == TrackKind::Video && !found_res {
                        for c in &clips {
                            if c.source_width >= 64 && c.source_height >= 64 {
                                seq_width = c.source_width;
                                seq_height = c.source_height;
                                found_res = true;
                                break;
                            }
                        }
                    }

                    let track = Track {
                        index: idx,
                        kind: track_kind,
                        name: track_name,
                        enabled: true,
                        locked: false,
                        muted: false,
                        solo: false,
                        audio_channels: if track_kind == TrackKind::Audio { 2 } else { 0 },
                        output_bus_id: None,
                        clips,
                        transitions,
                        markers: Vec::new(),
                    };
                    if track_kind == TrackKind::Video {
                        video_tracks.push(track);
                    } else {
                        audio_tracks.push(track);
                    }
                }
                _ => {}
            }
        }

        let mut seq = Sequence {
            id: seq_id,
            name,
            kind,
            width: seq_width,
            height: seq_height,
            frame_rate: seq_fps,
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
        Some(seq)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_segment(
        &mut self,
        seg_sid: usize,
        track_kind: TrackKind,
        slot_rate: Rational,
        seq_fps: Rational,
        cursor: &mut i64,
        clips: &mut Vec<Clip>,
        transitions: &mut Vec<Transition>,
        inherited_speed: Option<(f64, bool)>,
        inherited_effects: &[EffectInstance],
    ) {
        let Some(entry) = self.cfb.entry(seg_sid) else {
            return;
        };
        let Ok(props) = read_obj_properties(self.cfb, seg_sid) else {
            return;
        };
        let class_code = smpte_class_code(&entry.clsid).unwrap_or(0);

        match class_code {
            CLASS_SEQUENCE => {
                let comps =
                    resolve_strong_ref_collection(self.cfb, seg_sid, &props, PID_SEQ_COMPONENTS);
                // In AAF Sequences, a Transition sits between adjacent outgoing & incoming clips
                // and overlaps them by `length` frames (with cut point at `cut_point` from transition start).
                let mut prev_was_transition_len: i64 = 0;
                for comp_sid in comps {
                    let Some(ce) = self.cfb.entry(comp_sid) else {
                        continue;
                    };
                    let c_code = smpte_class_code(&ce.clsid).unwrap_or(0);
                    if c_code == CLASS_TRANSITION {
                        if let Ok(tp) = read_obj_properties(self.cfb, comp_sid) {
                            let raw_len = tp.get_i64(PID_COMP_LENGTH).unwrap_or(0);
                            let raw_cut = tp.get_i64(PID_TRANSITION_CUT_POINT).unwrap_or(raw_len / 2);
                            let len = convert_frames(raw_len, slot_rate, seq_fps).max(1);
                            let cut = convert_frames(raw_cut, slot_rate, seq_fps).clamp(0, len);
                            // In AAF Sequence timing, Transition reduces total duration by `len`:
                            // outgoing clip overlaps the first `len` frames and incoming starts `len` frames earlier.
                            let tr_start = (*cursor - len).max(0);
                            let tr_end = tr_start + len;
                            let alignment = if cut == 0 {
                                "start"
                            } else if cut >= len {
                                "end"
                            } else {
                                "center"
                            };
                            let (tr_name, tr_id) =
                                resolve_transition_name(self.cfb, comp_sid, &tp, self.op_def_names, track_kind);
                            transitions.push(Transition {
                                name: tr_name,
                                effect_id: tr_id,
                                start_frame: tr_start,
                                end_frame: tr_end,
                                alignment: alignment.to_string(),
                            });
                            // Trim previous clip to cut point and set cursor back to cut point
                            let cut_pos = tr_start + cut;
                            if let Some(prev) = clips.last_mut()
                                && prev.end_frame > cut_pos
                            {
                                let trim = prev.end_frame - cut_pos;
                                prev.end_frame = cut_pos;
                                prev.source_out_frame =
                                    (prev.source_out_frame - trim).max(prev.source_in_frame);
                            }
                            *cursor = cut_pos;
                            prev_was_transition_len = cut;
                        }
                        continue;
                    }

                    let before_len = clips.len();
                    self.walk_segment(
                        comp_sid,
                        track_kind,
                        slot_rate,
                        seq_fps,
                        cursor,
                        clips,
                        transitions,
                        inherited_speed,
                        inherited_effects,
                    );
                    // If immediately after a transition, trim incoming clip head by (len - cut) so it meets cut_pos cleanly
                    if prev_was_transition_len > 0 && clips.len() > before_len {
                        let incoming = &mut clips[before_len];
                        let trim = prev_was_transition_len.min((incoming.duration_frames() - 1).max(0));
                        incoming.end_frame -= trim;
                        incoming.source_in_frame += trim;
                        *cursor = incoming.end_frame;
                    }
                    prev_was_transition_len = 0;
                }
            }
            CLASS_FILLER => {
                let raw_len = props.get_i64(PID_COMP_LENGTH).unwrap_or(0).max(0);
                *cursor += convert_frames(raw_len, slot_rate, seq_fps);
            }
            CLASS_SOURCE_CLIP => {
                let raw_len = props.get_i64(PID_COMP_LENGTH).unwrap_or(0).max(0);
                let dur_frames = convert_frames(raw_len, slot_rate, seq_fps);
                if dur_frames <= 0 {
                    return;
                }
                let raw_start = props.get_i64(PID_SRC_CLIP_START_TIME).unwrap_or(0).max(0);
                let source_mob_id = props.get_mob_id(PID_SRC_REF_SOURCE_ID);
                let source_slot_id = props.get_u32(PID_SRC_REF_SOURCE_SLOT_ID).unwrap_or(1);

                let (speed_percent, reversed) = inherited_speed.unwrap_or((100.0, false));
                let start_frame = *cursor;
                let end_frame = start_frame + dur_frames;
                *cursor = end_frame;

                // Check if this SourceClip points to a nested CompositionMob
                if let Some(mid) = source_mob_id
                    && !is_zero_mob_id(&mid)
                    && let Some(target_mob) = self.mobs_by_id.get(&mid)
                    && target_mob.kind == MobKind::Composition
                {
                    let nested_seq_id = self
                        .comp_seq_uuid
                        .get(&mid)
                        .cloned()
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    let src_in = convert_frames(raw_start, slot_rate, seq_fps).max(0);
                    clips.push(Clip {
                        id: self.alloc_clip_id(),
                        name: if target_mob.name.is_empty() {
                            "Nested Composition".into()
                        } else {
                            target_mob.name.clone()
                        },
                        media_id: nested_seq_id,
                        media_path: String::new(),
                        media_kind: if track_kind == TrackKind::Audio {
                            MediaKind::Audio
                        } else {
                            MediaKind::Video
                        },
                        source_kind: ClipSourceKind::NestedSequence,
                        active_angle: None,
                        start_frame,
                        end_frame,
                        source_in_frame: src_in,
                        source_out_frame: src_in + dur_frames,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: seq_fps,
                        source_width: if track_kind == TrackKind::Video { 1920 } else { 0 },
                        source_height: if track_kind == TrackKind::Video { 1080 } else { 0 },
                        speed_percent,
                        reversed,
                        speed_keyframes: Vec::new(),
                        retime_interpolation: RetimeInterpolation::Default,
                        enabled: true,
                        audio_channel: if track_kind == TrackKind::Audio { 1 } else { 0 },
                        link_group_id: None,
                        label_name: None,
                        label_preset_index: None,
                        motion: MotionTransform::default(),
                        opacity: 100.0,
                        blend_mode: BlendMode::Normal,
                        lumetri_effects: Vec::new(),
                        effects: inherited_effects.to_vec(),
                        markers: Vec::new(),
                    });
                    return;
                }

                // Follow MasterMob -> SourceMob -> EssenceDescriptor chain
                let resolved = source_mob_id
                    .filter(|id| !is_zero_mob_id(id))
                    .map(|id| self.resolve_source_chain(id, source_slot_id, track_kind, slot_rate))
                    .unwrap_or_else(|| ResolvedSource {
                        name: "Offline Clip".into(),
                        path: String::new(),
                        media_kind: if track_kind == TrackKind::Audio {
                            MediaKind::Audio
                        } else {
                            MediaKind::Video
                        },
                        width: if track_kind == TrackKind::Video { 1920 } else { 0 },
                        height: if track_kind == TrackKind::Video { 1080 } else { 0 },
                        fps: slot_rate,
                        duration_frames: raw_len,
                        source_tc_frame: 0,
                        audio_channels: if track_kind == TrackKind::Audio { 2 } else { 0 },
                        audio_sample_rate: if track_kind == TrackKind::Audio { 48_000 } else { 0 },
                        tape_name: None,
                        source_in_offset: 0,
                    });

                let source_in_frame = (convert_frames(raw_start, slot_rate, resolved.fps)
                    + resolved.source_in_offset)
                    .max(0);
                let dur_in_src = convert_frames(dur_frames, seq_fps, resolved.fps).max(1);
                let source_out_frame = if speed_percent.abs() < 0.01 {
                    source_in_frame
                } else {
                    source_in_frame
                        + ((dur_in_src as f64) * (speed_percent / 100.0))
                            .round()
                            .max(1.0) as i64
                };

                let media_id = self.register_asset(&resolved, source_out_frame);

                clips.push(Clip {
                    id: self.alloc_clip_id(),
                    name: resolved.name,
                    media_id,
                    media_path: resolved.path,
                    media_kind: resolved.media_kind,
                    source_kind: ClipSourceKind::Media,
                    active_angle: None,
                    start_frame,
                    end_frame,
                    source_in_frame,
                    source_out_frame,
                    source_in_ticks: None,
                    source_out_ticks: None,
                    source_fps: resolved.fps,
                    source_width: resolved.width,
                    source_height: resolved.height,
                    speed_percent,
                    reversed,
                    speed_keyframes: Vec::new(),
                    retime_interpolation: RetimeInterpolation::Default,
                    enabled: true,
                    audio_channel: if track_kind == TrackKind::Audio { 1 } else { 0 },
                    link_group_id: None,
                    label_name: None,
                    label_preset_index: None,
                    motion: MotionTransform::default(),
                    opacity: 100.0,
                    blend_mode: BlendMode::Normal,
                    lumetri_effects: Vec::new(),
                    effects: inherited_effects.to_vec(),
                    markers: Vec::new(),
                });
            }
            CLASS_OPERATION_GROUP => {
                let op_auid = props.get_auid(PID_OP_GROUP_OPERATION);
                let op_name = op_auid
                    .and_then(|a| self.op_def_names.get(&a).cloned())
                    .or_else(|| {
                        op_auid.and_then(|a| {
                            if a == OPDEF_VIDEO_SPEED_CONTROL {
                                Some("VideoSpeedControl".to_string())
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or_else(|| "AAF Effect".to_string());

                let mut next_speed = inherited_speed;
                let mut next_effects = inherited_effects.to_vec();

                let is_speed_op = op_auid == Some(OPDEF_VIDEO_SPEED_CONTROL)
                    || op_name.to_ascii_lowercase().contains("speed")
                    || op_name.to_ascii_lowercase().contains("motion control")
                    || op_name.to_ascii_lowercase().contains("timewarp");

                let param_sids =
                    resolve_strong_ref_collection(self.cfb, seg_sid, &props, PID_OP_GROUP_PARAMETERS);
                let mut fx_params = Vec::new();
                for psid in param_sids {
                    if let Ok(pp) = read_obj_properties(self.cfb, psid) {
                        let pdef = pp.get_auid(PID_PARAM_DEF);
                        if is_speed_op
                            && let Some(val_prop) = pp.get(PID_CONST_VAL_VALUE)
                            && let Some(ratio) = decode_indirect_f64_or_rational(&val_prop.data)
                            && (pdef == Some(PARAMDEF_SPEED_RATIO) || next_speed.is_none())
                        {
                            if ratio.abs() < 1e-4 {
                                next_speed = Some((0.0, false));
                            } else {
                                next_speed = Some((ratio.abs() * 100.0, ratio < 0.0));
                            }
                        } else if let Some(val_prop) = pp.get(PID_CONST_VAL_VALUE)
                            && let Some(v) = decode_indirect_f64_or_rational(&val_prop.data)
                        {
                            fx_params.push(EffectParameter {
                                id: format!("param-{}", fx_params.len() + 1),
                                name: format!("Param {}", fx_params.len() + 1),
                                value: format!("{v:.3}"),
                                is_animated: false,
                            });
                        }
                    }
                }

                if !is_speed_op
                    && !op_name.to_ascii_lowercase().contains("audio gain")
                    && !op_name.to_ascii_lowercase().contains("audio pan")
                {
                    next_effects.push(EffectInstance {
                        id: format!("aaf-fx-{}", next_effects.len() + 1),
                        match_name: op_name.clone(),
                        display_name: op_name,
                        category: if track_kind == TrackKind::Audio {
                            EffectCategory::Audio
                        } else {
                            EffectCategory::Other
                        },
                        media_kind: track_kind,
                        enabled: true,
                        is_intrinsic: false,
                        is_animated: false,
                        parameters: fx_params,
                    });
                }

                let inputs = resolve_strong_ref_collection(
                    self.cfb,
                    seg_sid,
                    &props,
                    PID_OP_GROUP_INPUT_SEGMENTS,
                );
                if inputs.is_empty() {
                    let raw_len = props.get_i64(PID_COMP_LENGTH).unwrap_or(0).max(0);
                    *cursor += convert_frames(raw_len, slot_rate, seq_fps);
                } else {
                    // Use the OperationGroup's own length to advance cursor accurately even under retiming
                    let group_start = *cursor;
                    let raw_group_len = props.get_i64(PID_COMP_LENGTH).unwrap_or(0).max(0);
                    let group_len = convert_frames(raw_group_len, slot_rate, seq_fps);
                    let before = clips.len();
                    self.walk_segment(
                        inputs[0],
                        track_kind,
                        slot_rate,
                        seq_fps,
                        cursor,
                        clips,
                        transitions,
                        next_speed,
                        &next_effects,
                    );
                    if group_len > 0 && clips.len() == before + 1 {
                        let c = &mut clips[before];
                        c.start_frame = group_start;
                        c.end_frame = group_start + group_len;
                        *cursor = c.end_frame;
                    }
                }
            }
            CLASS_NESTED_SCOPE => {
                let slots =
                    resolve_strong_ref_collection(self.cfb, seg_sid, &props, PID_NESTED_SCOPE_SLOTS);
                if let Some(&first) = slots.first() {
                    self.walk_segment(
                        first,
                        track_kind,
                        slot_rate,
                        seq_fps,
                        cursor,
                        clips,
                        transitions,
                        inherited_speed,
                        inherited_effects,
                    );
                }
            }
            CLASS_SELECTOR => {
                if let Some(sel_sid) =
                    resolve_strong_ref(self.cfb, seg_sid, &props, PID_SELECTOR_SELECTED)
                {
                    self.walk_segment(
                        sel_sid,
                        track_kind,
                        slot_rate,
                        seq_fps,
                        cursor,
                        clips,
                        transitions,
                        inherited_speed,
                        inherited_effects,
                    );
                }
            }
            _ => {
                let raw_len = props.get_i64(PID_COMP_LENGTH).unwrap_or(0).max(0);
                *cursor += convert_frames(raw_len, slot_rate, seq_fps);
            }
        }
    }

    fn resolve_source_chain(
        &self,
        start_mob_id: [u8; 32],
        start_slot_id: u32,
        track_kind: TrackKind,
        fallback_fps: Rational,
    ) -> ResolvedSource {
        let mut cur_mob_id = start_mob_id;
        let mut cur_slot_id = start_slot_id;
        let mut visited = HashSet::new();

        let mut best_name = String::new();
        let mut best_path = String::new();
        let mut width = 0u32;
        let mut height = 0u32;
        let mut fps = fallback_fps;
        let mut duration_frames = 0i64;
        let mut source_tc_frame = 0i64;
        let mut audio_channels = 0u32;
        let mut audio_sample_rate = 0u32;
        let mut tape_name: Option<String> = None;
        let mut source_in_offset = 0i64;

        while visited.insert(cur_mob_id) {
            let Some(mob) = self.mobs_by_id.get(&cur_mob_id) else {
                break;
            };
            if best_name.is_empty() && !mob.name.is_empty() {
                best_name = mob.name.clone();
            }

            if let Some(desc) = &mob.descriptor {
                if best_path.is_empty()
                    && let Some(url) = desc.urls.first()
                {
                    best_path = file_url_to_path(url);
                }
                if width == 0 && desc.width > 0 {
                    width = desc.width;
                    height = desc.height;
                }
                if let Some(sr) = desc.sample_rate
                    && sr.num > 0
                    && sr.den > 0
                    && sr.as_f64() <= 240.0
                {
                    fps = sr;
                }
                if desc.length > duration_frames {
                    duration_frames = desc.length;
                }
                if desc.audio_channels > 0 {
                    audio_channels = desc.audio_channels;
                }
                if desc.audio_sample_rate > 0 {
                    audio_sample_rate = desc.audio_sample_rate;
                }
                if desc.is_tape && tape_name.is_none() && !mob.name.is_empty() {
                    tape_name = Some(mob.name.clone());
                }
            }

            // Extract Timecode slot on this Mob if present
            for s in &mob.slots {
                if s.media_kind == SlotMediaKind::Timecode
                    && source_tc_frame == 0
                    && let Some(seg_sid) = s.segment_storage_id
                    && let Some((tc_start, _)) = extract_timecode_segment(self.cfb, seg_sid)
                {
                    source_tc_frame = tc_start.max(0);
                }
            }

            // Follow SourceClip inside matching slot to next Mob in the chain
            let matching_slot = mob
                .slots
                .iter()
                .find(|s| s.slot_id == cur_slot_id)
                .or_else(|| {
                    mob.slots.iter().find(|s| {
                        (track_kind == TrackKind::Video && s.media_kind == SlotMediaKind::Video)
                            || (track_kind == TrackKind::Audio
                                && s.media_kind == SlotMediaKind::Audio)
                    })
                });

            let Some(slot) = matching_slot else {
                break;
            };
            if slot.edit_rate.num > 0 && slot.edit_rate.den > 0 && slot.edit_rate.as_f64() <= 240.0 {
                fps = slot.edit_rate;
            }
            let Some(seg_sid) = slot.segment_storage_id else {
                break;
            };
            let Some((next_mob, next_slot, start_pos, seg_len)) =
                find_first_source_clip(self.cfb, seg_sid)
            else {
                break;
            };
            if seg_len > duration_frames {
                duration_frames = seg_len;
            }
            if is_zero_mob_id(&next_mob) {
                break;
            }
            source_in_offset += start_pos.max(0);
            cur_mob_id = next_mob;
            cur_slot_id = next_slot;
        }

        if best_name.is_empty() && !best_path.is_empty() {
            best_name = Path::new(&best_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Media")
                .to_string();
        }
        if best_name.is_empty() {
            best_name = "Media".to_string();
        }

        let media_kind = if track_kind == TrackKind::Audio && width == 0 {
            MediaKind::Audio
        } else {
            MediaKind::Video
        };

        ResolvedSource {
            name: best_name,
            path: best_path,
            media_kind,
            width: if media_kind == MediaKind::Audio {
                0
            } else if width > 0 {
                width
            } else {
                1920
            },
            height: if media_kind == MediaKind::Audio {
                0
            } else if height > 0 {
                height
            } else {
                1080
            },
            fps,
            duration_frames,
            source_tc_frame,
            audio_channels: if audio_channels > 0 {
                audio_channels
            } else if track_kind == TrackKind::Audio {
                2
            } else {
                0
            },
            audio_sample_rate: if audio_sample_rate > 0 {
                audio_sample_rate
            } else if track_kind == TrackKind::Audio {
                48_000
            } else {
                0
            },
            tape_name,
            source_in_offset,
        }
    }

    fn register_asset(&mut self, src: &ResolvedSource, min_dur: i64) -> String {
        let key = if !src.path.is_empty() {
            src.path.clone()
        } else {
            format!("name:{}", src.name)
        };
        if let Some(&idx) = self.asset_idx_by_key.get(&key) {
            let a = &mut self.media_assets[idx];
            if min_dur > a.duration_frames {
                a.duration_frames = min_dur;
            }
            if a.width == 0 && src.width > 0 {
                a.width = src.width;
                a.height = src.height;
                a.media_kind = MediaKind::Video;
            }
            if a.audio_channels == 0 && src.audio_channels > 0 {
                a.audio_channels = src.audio_channels;
                a.audio_sample_rate = src.audio_sample_rate.max(48_000);
            }
            return a.id.clone();
        }

        let id = format!("aaf-asset-{}", self.media_assets.len() + 1);
        let mut metadata = BTreeMap::new();
        if let Some(tape) = &src.tape_name {
            metadata.insert("Tape Name".into(), tape.clone());
        }
        let asset = MediaAsset {
            id: id.clone(),
            name: src.name.clone(),
            path: src.path.clone(),
            media_kind: src.media_kind,
            width: src.width,
            height: src.height,
            frame_rate: src.fps,
            duration_frames: src.duration_frames.max(min_dur).max(0),
            source_tc_frame: src.source_tc_frame.max(0),
            source_tc_string: frames_to_timecode(src.source_tc_frame.max(0), src.fps),
            audio_channels: src.audio_channels,
            audio_sample_rate: src.audio_sample_rate,
            color_space: None,
            video_codec: None,
            audio_codec: None,
            alpha_mode: AlphaMode::Default,
            data_levels: DataLevels::Auto,
            pixel_aspect_ratio: None,
            metadata,
            custom_metadata: BTreeMap::new(),
        };
        let idx = self.media_assets.len();
        self.media_assets.push(asset);
        self.asset_idx_by_key.insert(key, idx);
        id
    }

    fn collect_event_markers(
        &self,
        seg_sid: usize,
        slot_rate: Rational,
        seq_fps: Rational,
        markers: &mut Vec<Marker>,
    ) {
        let Some(entry) = self.cfb.entry(seg_sid) else {
            return;
        };
        let Ok(props) = read_obj_properties(self.cfb, seg_sid) else {
            return;
        };
        let class_code = smpte_class_code(&entry.clsid).unwrap_or(0);
        if class_code == CLASS_SEQUENCE {
            for comp_sid in
                resolve_strong_ref_collection(self.cfb, seg_sid, &props, PID_SEQ_COMPONENTS)
            {
                self.collect_event_markers(comp_sid, slot_rate, seq_fps, markers);
            }
            return;
        }
        if class_code == CLASS_COMMENT_MARKER || class_code == CLASS_DESCRIPTIVE_MARKER {
            let pos = props.get_i64(PID_EVENT_POSITION).unwrap_or(0);
            let len = props.get_i64(PID_COMP_LENGTH).unwrap_or(1).max(1);
            let comment = props.get_string(PID_EVENT_COMMENT).unwrap_or_default();
            let color = props
                .get_string(PID_COMMENT_MARKER_COLOR)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "Blue".to_string());
            let (name, body) = if let Some((n, b)) = comment.split_once(": ") {
                (n.to_string(), b.to_string())
            } else {
                (comment.clone(), comment)
            };
            markers.push(Marker {
                frame: convert_frames(pos, slot_rate, seq_fps).max(0),
                duration_frames: convert_frames(len, slot_rate, seq_fps).max(1),
                name,
                comment: body,
                color,
                scope: MarkerScope::Sequence,
            });
        }
    }
}

// ── Low-level AAF Storage Traversal Helpers ─────────────────────────────

fn read_obj_properties(cfb: &CfbFile<'_>, storage_id: usize) -> NleResult<PropertyMap> {
    let prop_stream_id = cfb
        .find_child(storage_id, "properties")
        .ok_or_else(|| NleError::InvalidFormat("AAF object missing properties stream".into()))?;
    let bytes = cfb.read_stream(prop_stream_id)?;
    PropertyMap::parse(&bytes)
}

fn resolve_strong_ref(
    cfb: &CfbFile<'_>,
    parent_sid: usize,
    props: &PropertyMap,
    pid: u16,
) -> Option<usize> {
    let name = props.get_ref_name(pid)?;
    cfb.find_child(parent_sid, &name)
}

fn resolve_strong_ref_collection(
    cfb: &CfbFile<'_>,
    parent_sid: usize,
    props: &PropertyMap,
    pid: u16,
) -> Vec<usize> {
    let Some(prop) = props.get(pid) else {
        return Vec::new();
    };
    let Some(base_name) = props.get_ref_name(pid) else {
        return Vec::new();
    };
    let idx_name = format!("{base_name} index");
    if let Some(idx_sid) = cfb.find_child(parent_sid, &idx_name)
        && let Ok(idx_bytes) = cfb.read_stream(idx_sid)
    {
        let keys = match prop.stored_form {
            SF_STRONG_OBJECT_REFERENCE_SET => {
                parse_strong_ref_set_index(&idx_bytes, props.little_endian)
            }
            _ => parse_strong_ref_vector_index(&idx_bytes, props.little_endian),
        };
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let child_name = format!("{base_name}{{{k:x}}}");
            if let Some(cid) = cfb.find_child(parent_sid, &child_name) {
                out.push(cid);
            }
        }
        return out;
    }

    // Fallback: scan child storages matching `{base_name}{...}`
    let prefix = format!("{base_name}{{");
    cfb.children(parent_sid)
        .into_iter()
        .filter(|&cid| {
            cfb.entry(cid)
                .map(|e| e.obj_type == CfbObjType::Storage && e.name.starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect()
}

fn parse_mob_slot(
    cfb: &CfbFile<'_>,
    slot_sid: usize,
    data_def_names: &HashMap<[u8; 16], String>,
) -> Option<ParsedSlot> {
    let entry = cfb.entry(slot_sid)?;
    let props = read_obj_properties(cfb, slot_sid).ok()?;
    let class_code = smpte_class_code(&entry.clsid).unwrap_or(0);
    let is_event_slot = class_code == CLASS_EVENT_MOB_SLOT;
    let slot_id = props.get_u32(PID_SLOT_ID).unwrap_or(1);
    let name = props.get_string(PID_SLOT_NAME).unwrap_or_default();
    let edit_rate = props
        .get_rational(PID_TIMELINE_SLOT_EDIT_RATE)
        .or_else(|| props.get_rational(PID_EVENT_SLOT_EDIT_RATE))
        .unwrap_or(Rational::FPS_25);
    let origin = props.get_i64(PID_TIMELINE_SLOT_ORIGIN).unwrap_or(0);
    let segment_storage_id = resolve_strong_ref(cfb, slot_sid, &props, PID_SLOT_SEGMENT);

    let media_kind = if is_event_slot {
        SlotMediaKind::Event
    } else if let Some(seg_sid) = segment_storage_id {
        classify_segment_media_kind(cfb, seg_sid, data_def_names)
    } else {
        SlotMediaKind::Other
    };

    Some(ParsedSlot {
        slot_id,
        name,
        edit_rate,
        origin,
        media_kind,
        segment_storage_id,
        is_event_slot,
    })
}

fn classify_segment_media_kind(
    cfb: &CfbFile<'_>,
    seg_sid: usize,
    data_def_names: &HashMap<[u8; 16], String>,
) -> SlotMediaKind {
    if let Some(entry) = cfb.entry(seg_sid) {
        let code = smpte_class_code(&entry.clsid).unwrap_or(0);
        if code == CLASS_TIMECODE {
            return SlotMediaKind::Timecode;
        }
        if code == CLASS_COMMENT_MARKER || code == CLASS_DESCRIPTIVE_MARKER {
            return SlotMediaKind::Event;
        }
    }
    let Ok(props) = read_obj_properties(cfb, seg_sid) else {
        return SlotMediaKind::Other;
    };
    if let Some(dd) = props.get_auid(PID_COMP_DATA_DEF) {
        if dd == DATADEF_PICTURE || dd == DATADEF_LEGACY_PICTURE {
            return SlotMediaKind::Video;
        }
        if dd == DATADEF_SOUND || dd == DATADEF_LEGACY_SOUND {
            return SlotMediaKind::Audio;
        }
        if dd == DATADEF_TIMECODE || dd == DATADEF_LEGACY_TIMECODE {
            return SlotMediaKind::Timecode;
        }
        if let Some(name) = data_def_names.get(&dd) {
            let lower = name.to_ascii_lowercase();
            if lower.contains("picture") || lower.contains("video") {
                return SlotMediaKind::Video;
            }
            if lower.contains("sound") || lower.contains("audio") {
                return SlotMediaKind::Audio;
            }
            if lower.contains("timecode") {
                return SlotMediaKind::Timecode;
            }
            if lower.contains("descriptive") || lower.contains("marker") {
                return SlotMediaKind::Event;
            }
        }
    }
    SlotMediaKind::Other
}

fn parse_essence_descriptor(cfb: &CfbFile<'_>, desc_sid: usize) -> ParsedDescriptor {
    let mut out = ParsedDescriptor::default();
    collect_descriptor_into(cfb, desc_sid, &mut out);
    out
}

fn collect_descriptor_into(cfb: &CfbFile<'_>, desc_sid: usize, out: &mut ParsedDescriptor) {
    let Some(entry) = cfb.entry(desc_sid) else {
        return;
    };
    let Ok(props) = read_obj_properties(cfb, desc_sid) else {
        return;
    };
    let code = smpte_class_code(&entry.clsid).unwrap_or(0);
    if code == CLASS_TAPE_DESCRIPTOR {
        out.is_tape = true;
    }
    if code == CLASS_CDCI_DESCRIPTOR || code == CLASS_RGBA_DESCRIPTOR {
        out.has_video = true;
    }
    if code == CLASS_PCM_DESCRIPTOR || code == CLASS_WAVE_DESCRIPTOR {
        out.has_audio = true;
    }

    if let Some(w) = props.get_u32(PID_IMAGE_DESC_STORED_WIDTH).filter(|&w| w > 0) {
        out.width = w;
        out.has_video = true;
    }
    if let Some(h) = props
        .get_u32(PID_IMAGE_DESC_STORED_HEIGHT)
        .filter(|&h| h > 0)
    {
        out.height = h;
    }
    if let Some(sr) = props.get_rational(PID_FILE_DESC_SAMPLE_RATE) {
        out.sample_rate = Some(sr);
    }
    if let Some(len) = props.get_i64(PID_FILE_DESC_LENGTH).filter(|&l| l > 0) {
        out.length = out.length.max(len);
    }
    if let Some(ch) = props
        .get_u32(PID_SOUND_DESC_CHANNEL_COUNT)
        .filter(|&c| c > 0)
    {
        out.audio_channels = ch;
        out.has_audio = true;
    }
    if let Some(asr) = props.get_rational(PID_SOUND_DESC_AUDIO_SAMPLING_RATE) {
        let rate = asr.as_f64().round() as u32;
        if rate > 0 {
            out.audio_sample_rate = rate;
        }
    }

    for loc_sid in resolve_strong_ref_collection(cfb, desc_sid, &props, PID_DESC_LOCATOR) {
        if let Ok(lp) = read_obj_properties(cfb, loc_sid)
            && let Some(url) = lp.get_string(PID_NET_LOCATOR_URL).filter(|s| !s.trim().is_empty())
        {
            out.urls.push(url);
        }
    }

    // Recurse into MultipleDescriptor sub-descriptors
    for sub_sid in resolve_strong_ref_collection(cfb, desc_sid, &props, PID_MULTI_DESC_FILE_DESCS) {
        collect_descriptor_into(cfb, sub_sid, out);
    }
}

fn extract_timecode_segment(cfb: &CfbFile<'_>, seg_sid: usize) -> Option<(i64, Rational)> {
    let entry = cfb.entry(seg_sid)?;
    let props = read_obj_properties(cfb, seg_sid).ok()?;
    let code = smpte_class_code(&entry.clsid).unwrap_or(0);
    if code == CLASS_TIMECODE {
        let start = props.get_i64(PID_TIMECODE_START).unwrap_or(0);
        let fps_base = props.get_u16(PID_TIMECODE_FPS).unwrap_or(25) as i64;
        let drop = props.get_u8(PID_TIMECODE_DROP).unwrap_or(0) != 0;
        let fps = if drop {
            Rational::new(fps_base * 1000, 1001)
        } else {
            Rational::new(fps_base.max(1), 1)
        };
        return Some((start, fps));
    }
    if code == CLASS_SEQUENCE {
        for comp_sid in resolve_strong_ref_collection(cfb, seg_sid, &props, PID_SEQ_COMPONENTS) {
            if let Some(res) = extract_timecode_segment(cfb, comp_sid) {
                return Some(res);
            }
        }
    }
    None
}

fn find_first_source_clip(cfb: &CfbFile<'_>, seg_sid: usize) -> Option<([u8; 32], u32, i64, i64)> {
    let entry = cfb.entry(seg_sid)?;
    let props = read_obj_properties(cfb, seg_sid).ok()?;
    let code = smpte_class_code(&entry.clsid).unwrap_or(0);
    if code == CLASS_SOURCE_CLIP {
        let mob_id = props.get_mob_id(PID_SRC_REF_SOURCE_ID).unwrap_or([0u8; 32]);
        let slot_id = props.get_u32(PID_SRC_REF_SOURCE_SLOT_ID).unwrap_or(1);
        let start = props.get_i64(PID_SRC_CLIP_START_TIME).unwrap_or(0);
        let len = props.get_i64(PID_COMP_LENGTH).unwrap_or(0);
        return Some((mob_id, slot_id, start, len));
    }
    if code == CLASS_SEQUENCE {
        for comp_sid in resolve_strong_ref_collection(cfb, seg_sid, &props, PID_SEQ_COMPONENTS) {
            if let Some(res) = find_first_source_clip(cfb, comp_sid) {
                return Some(res);
            }
        }
    }
    None
}

fn collect_referenced_comp_mobs(
    cfb: &CfbFile<'_>,
    storage_id: usize,
    mobs_by_id: &HashMap<[u8; 32], ParsedMob>,
    nested_out: &mut HashSet<[u8; 32]>,
) {
    let mut stack = vec![storage_id];
    let mut seen = HashSet::new();
    while let Some(sid) = stack.pop() {
        if !seen.insert(sid) {
            continue;
        }
        if let Some(entry) = cfb.entry(sid)
            && smpte_class_code(&entry.clsid) == Some(CLASS_SOURCE_CLIP)
            && let Ok(props) = read_obj_properties(cfb, sid)
            && let Some(ref_mob) = props.get_mob_id(PID_SRC_REF_SOURCE_ID)
            && let Some(m) = mobs_by_id.get(&ref_mob)
            && m.kind == MobKind::Composition
        {
            nested_out.insert(ref_mob);
        }
        for child in cfb.children(sid) {
            if let Some(e) = cfb.entry(child)
                && e.obj_type == CfbObjType::Storage
            {
                stack.push(child);
            }
        }
    }
}

fn resolve_transition_name(
    cfb: &CfbFile<'_>,
    tr_sid: usize,
    tr_props: &PropertyMap,
    op_def_names: &HashMap<[u8; 16], String>,
    track_kind: TrackKind,
) -> (String, String) {
    if let Some(og_sid) = resolve_strong_ref(cfb, tr_sid, tr_props, PID_TRANSITION_OP_GROUP)
        && let Ok(og_props) = read_obj_properties(cfb, og_sid)
        && let Some(op_auid) = og_props.get_auid(PID_OP_GROUP_OPERATION)
    {
        if op_auid == OPDEF_VIDEO_DISSOLVE {
            return ("Cross Dissolve".into(), "AE.ADBE Cross Dissolve".into());
        }
        if op_auid == OPDEF_AUDIO_DISSOLVE {
            return ("Constant Power".into(), "Constant Power".into());
        }
        if let Some(name) = op_def_names.get(&op_auid) {
            let lower = name.to_ascii_lowercase();
            if lower.contains("dissolve") {
                return if track_kind == TrackKind::Audio {
                    ("Constant Power".into(), "Constant Power".into())
                } else {
                    ("Cross Dissolve".into(), "AE.ADBE Cross Dissolve".into())
                };
            }
            return (name.clone(), name.clone());
        }
    }
    if track_kind == TrackKind::Audio {
        ("Constant Power".into(), "Constant Power".into())
    } else {
        ("Cross Dissolve".into(), "AE.ADBE Cross Dissolve".into())
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
