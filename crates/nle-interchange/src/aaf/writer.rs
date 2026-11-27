use super::cfb::{write_cfb_v3, CfbStorageNode};
use super::codec::*;
use crate::fcpxml::time::path_to_file_url;
use nle_core::{
    flatten_nested_and_multicam_clips, Clip, ClipSourceKind, ConversionMessage, ConversionOptions,
    ConversionReport, MediaKind, NleError, NleFormat, NleResult, Project,
    ProjectWriter, Rational, Sequence, SequenceKind, Track, TrackKind, Transition,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct AafWriter;

impl ProjectWriter for AafWriter {
    fn format(&self) -> NleFormat {
        NleFormat::Aaf
    }

    fn write_file(
        &self,
        project: &Project,
        output_path: &Path,
        options: &ConversionOptions,
    ) -> NleResult<ConversionReport> {
        write_aaf_file(project, output_path, options)
    }
}

/// Write an Advanced Authoring Format (`.aaf`, MS-CFB v3 Structured Storage) file
/// from the canonical `Project` IR. Supports multiple sequences, nested sequences,
/// video/audio tracks, transitions, retiming (`VideoSpeedControl`), and sequence markers.
pub fn write_aaf_file(
    project: &Project,
    output_path: &Path,
    options: &ConversionOptions,
) -> NleResult<ConversionReport> {
    let mut messages: Vec<ConversionMessage> = Vec::new();
    let project_cow: Cow<'_, Project> = if options.flatten_nested_and_multicam {
        let mut cloned = project.clone();
        let flat = flatten_nested_and_multicam_clips(&mut cloned);
        messages.extend(flat.messages);
        Cow::Owned(cloned)
    } else {
        Cow::Borrowed(project)
    };
    let project = project_cow.as_ref();

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
        return Err(NleError::InvalidFormat(
            "No sequences selected for AAF export".into(),
        ));
    }

    // Include any nested/compound sequences referenced by selected sequences
    let mut export_seqs: Vec<&Sequence> = selected.clone();
    let mut seq_ids_present: std::collections::HashSet<&str> =
        export_seqs.iter().map(|s| s.id.as_str()).collect();
    for seq in &selected {
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            for clip in &track.clips {
                if clip.source_kind != ClipSourceKind::Media
                    && !seq_ids_present.contains(clip.media_id.as_str())
                    && let Some(helper) = project.sequences.iter().find(|s| s.id == clip.media_id)
                {
                    seq_ids_present.insert(helper.id.as_str());
                    export_seqs.push(helper);
                }
            }
        }
    }

    // Assign MobIDs to every Sequence (CompositionMob)
    let mut seq_mob_ids: HashMap<&str, [u8; 32]> = HashMap::new();
    for seq in &export_seqs {
        seq_mob_ids.insert(seq.id.as_str(), generate_mob_id());
    }

    // Collect & deduplicate assets referenced by clips (plus project.media_assets)
    let mut asset_entries: Vec<AssetMobEntry> = Vec::new();
    let mut asset_mob_by_key: HashMap<String, usize> = HashMap::new();

    for a in &project.media_assets {
        let key = if !a.path.is_empty() {
            a.path.clone()
        } else {
            format!("id:{}", a.id)
        };
        asset_mob_by_key.entry(key).or_insert_with(|| {
            let idx = asset_entries.len();
            asset_entries.push(AssetMobEntry {
                id: a.id.clone(),
                name: a.name.clone(),
                path: a.path.clone(),
                media_kind: a.media_kind,
                width: a.width,
                height: a.height,
                fps: a.frame_rate,
                duration_frames: a.duration_frames.max(1),
                source_tc_frame: a.source_tc_frame.max(0),
                audio_channels: a.audio_channels,
                audio_sample_rate: a.audio_sample_rate,
                source_mob_id: generate_mob_id(),
                master_mob_id: generate_mob_id(),
            });
            idx
        });
    }

    // Ensure every clip's media has an AssetMobEntry
    for seq in &export_seqs {
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            for clip in &track.clips {
                if clip.source_kind != ClipSourceKind::Media {
                    continue;
                }
                let key = if !clip.media_path.is_empty() {
                    clip.media_path.clone()
                } else if !clip.media_id.is_empty() {
                    format!("id:{}", clip.media_id)
                } else {
                    format!("name:{}", clip.name)
                };
                asset_mob_by_key.entry(key).or_insert_with(|| {
                    let idx = asset_entries.len();
                    asset_entries.push(AssetMobEntry {
                        id: clip.media_id.clone(),
                        name: clip.name.clone(),
                        path: clip.media_path.clone(),
                        media_kind: clip.media_kind,
                        width: clip.source_width,
                        height: clip.source_height,
                        fps: clip.source_fps,
                        duration_frames: clip.source_out_frame.max(clip.duration_frames()).max(1),
                        source_tc_frame: 0,
                        audio_channels: if track.kind == TrackKind::Audio { 2 } else { 0 },
                        audio_sample_rate: if track.kind == TrackKind::Audio {
                            48_000
                        } else {
                            0
                        },
                        source_mob_id: generate_mob_id(),
                        master_mob_id: generate_mob_id(),
                    });
                    idx
                });
            }
        }
    }

    let asset_idx_by_id: HashMap<String, usize> = asset_entries
        .iter()
        .enumerate()
        .filter(|(_, a)| !a.id.is_empty())
        .map(|(i, a)| (a.id.clone(), i))
        .collect();

    // Build all Mob storages (`SourceMob`, `MasterMob`, `CompositionMob`)
    let mut mob_storages: Vec<([u8; 32], CfbStorageNode)> = Vec::new();
    for a in &asset_entries {
        mob_storages.push((a.source_mob_id, build_source_mob(a)));
        mob_storages.push((a.master_mob_id, build_master_mob(a)));
    }

    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;

    for seq in &export_seqs {
        let comp_mob_id = seq_mob_ids[seq.id.as_str()];
        let (comp_node, vc, ac, mk) = build_composition_mob(
            seq,
            comp_mob_id,
            &seq_mob_ids,
            &asset_entries,
            &asset_mob_by_key,
            &asset_idx_by_id,
            options,
        );
        mob_storages.push((comp_mob_id, comp_node));
        total_v += vc;
        total_a += ac;
        total_markers += mk;
    }

    // Build ContentStorage (`Mobs-1901` strong ref set)
    let mut content = CfbStorageNode::new("Content-3b03", CLSID_CONTENT_STORAGE);
    let mut content_props = PropertyBuilder::default();
    content_props.add_strong_ref_set(PID_CONTENT_MOBS, "Mobs-1901");
    content.add_stream("properties", content_props.finish());

    let mob_keys: Vec<Vec<u8>> = mob_storages.iter().map(|(id, _)| id.to_vec()).collect();
    content.add_stream(
        "Mobs-1901 index",
        encode_strong_ref_set_index(PID_MOB_ID, &mob_keys),
    );
    for (idx, (_, mut node)) in mob_storages.into_iter().enumerate() {
        node.name = format!("Mobs-1901{{{idx:x}}}");
        content.add_storage(node);
    }

    // Build Dictionary (`DataDefinitions-2605` & `OperationDefinitions-2603`)
    let dictionary = build_dictionary();

    // Build IdentificationList (`IdentificationList-3b06`)
    let ident = build_identification();

    // Build Header
    let mut header = CfbStorageNode::new("Header-2", CLSID_HEADER);
    let mut hdr_props = PropertyBuilder::default();
    hdr_props.add_u16(PID_HEADER_BYTE_ORDER, 0x4949); // Little-endian
    hdr_props.add_data(PID_HEADER_LAST_MODIFIED, vec![0u8; 8]);
    hdr_props.add_strong_ref(PID_HEADER_CONTENT, "Content-3b03");
    hdr_props.add_strong_ref(PID_HEADER_DICTIONARY, "Dictionary-3b04");
    hdr_props.add_data(PID_HEADER_VERSION, vec![1, 1]);
    hdr_props.add_strong_ref_vector(PID_HEADER_IDENT_LIST, "IdentificationList-3b06");
    hdr_props.add_u32(PID_HEADER_OBJ_MODEL_VER, 1);
    header.add_stream("properties", hdr_props.finish());
    header.add_storage(content);
    header.add_storage(dictionary);
    header.add_stream(
        "IdentificationList-3b06 index",
        encode_strong_ref_vector_index(1),
    );
    header.add_storage(ident);

    // Build Root Storage
    let mut root = CfbStorageNode::new("Root Entry", CLSID_ROOT);
    let mut root_props = PropertyBuilder::default();
    root_props.add_strong_ref(PID_ROOT_HEADER, "Header-2");
    root.add_stream("properties", root_props.finish());
    // Weak-reference table paths:
    //   index 0 -> Header -> Dictionary -> DataDefinitions
    //   index 1 -> Header -> Dictionary -> OperationDefinitions
    root.add_stream(
        "referenced properties",
        encode_referenced_properties(&[
            &[PID_ROOT_HEADER, PID_HEADER_DICTIONARY, PID_DICT_DATA_DEFS],
            &[
                PID_ROOT_HEADER,
                PID_HEADER_DICTIONARY,
                PID_DICT_OPERATION_DEFS,
            ],
        ]),
    );
    root.add_storage(header);

    let cfb_bytes = write_cfb_v3(&root);
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, cfb_bytes)?;

    Ok(ConversionReport {
        output_path: output_path.to_string_lossy().into_owned(),
        source_format: project.source_format,
        target_format: NleFormat::Aaf,
        sequences_converted: export_seqs.len(),
        video_clips_converted: total_v,
        audio_clips_converted: total_a,
        media_assets_linked: asset_entries.len(),
        bins_created: 0,
        markers_written: total_markers,
        grades_attached: 0,
        effect_markers_injected: 0,
        messages,
    })
}

struct AssetMobEntry {
    id: String,
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
    source_mob_id: [u8; 32],
    master_mob_id: [u8; 32],
}

fn build_dictionary() -> CfbStorageNode {
    let mut dict = CfbStorageNode::new("Dictionary-3b04", CLSID_DICTIONARY);
    let mut dp = PropertyBuilder::default();
    dp.add_strong_ref_set(PID_DICT_DATA_DEFS, "DataDefinitions-2605");
    dp.add_strong_ref_set(PID_DICT_OPERATION_DEFS, "OperationDefinitions-2603");
    dict.add_stream("properties", dp.finish());

    let data_defs: &[([u8; 16], &str)] = &[
        (DATADEF_PICTURE, "Picture"),
        (DATADEF_SOUND, "Sound"),
        (DATADEF_TIMECODE, "Timecode"),
    ];
    let dd_keys: Vec<Vec<u8>> = data_defs.iter().map(|(a, _)| a.to_vec()).collect();
    dict.add_stream(
        "DataDefinitions-2605 index",
        encode_strong_ref_set_index(PID_DEF_IDENTIFICATION, &dd_keys),
    );
    for (i, (auid, name)) in data_defs.iter().enumerate() {
        let mut node =
            CfbStorageNode::new(format!("DataDefinitions-2605{{{i:x}}}"), CLSID_DATA_DEFINITION);
        let mut p = PropertyBuilder::default();
        p.add_data(PID_DEF_IDENTIFICATION, auid.to_vec());
        p.add_string(PID_DEF_NAME, name);
        p.add_string(PID_DEF_DESCRIPTION, name);
        node.add_stream("properties", p.finish());
        dict.add_storage(node);
    }

    let op_defs: &[([u8; 16], &str)] = &[
        (OPDEF_VIDEO_DISSOLVE, "VideoDissolve"),
        (OPDEF_AUDIO_DISSOLVE, "AudioDissolve"),
        (OPDEF_VIDEO_SPEED_CONTROL, "VideoSpeedControl"),
    ];
    let od_keys: Vec<Vec<u8>> = op_defs.iter().map(|(a, _)| a.to_vec()).collect();
    dict.add_stream(
        "OperationDefinitions-2603 index",
        encode_strong_ref_set_index(PID_DEF_IDENTIFICATION, &od_keys),
    );
    for (i, (auid, name)) in op_defs.iter().enumerate() {
        let mut node = CfbStorageNode::new(
            format!("OperationDefinitions-2603{{{i:x}}}"),
            CLSID_OPERATION_DEFINITION,
        );
        let mut p = PropertyBuilder::default();
        p.add_data(PID_DEF_IDENTIFICATION, auid.to_vec());
        p.add_string(PID_DEF_NAME, name);
        p.add_string(PID_DEF_DESCRIPTION, name);
        node.add_stream("properties", p.finish());
        dict.add_storage(node);
    }

    dict
}

fn build_identification() -> CfbStorageNode {
    let mut node = CfbStorageNode::new("IdentificationList-3b06{0}", CLSID_IDENTIFICATION);
    let mut p = PropertyBuilder::default();
    p.add_string(PID_IDENT_COMPANY_NAME, "Aagedal");
    p.add_string(PID_IDENT_PRODUCT_NAME, "Aagedal NLE Project Converter");
    p.add_string(PID_IDENT_VERSION_STR, env!("CARGO_PKG_VERSION"));
    p.add_data(PID_IDENT_PRODUCT_ID, uuid::Uuid::new_v4().as_bytes().to_vec());
    p.add_data(PID_IDENT_DATE, vec![0u8; 8]);
    p.add_data(
        PID_IDENT_GENERATION_AUID,
        uuid::Uuid::new_v4().as_bytes().to_vec(),
    );
    node.add_stream("properties", p.finish());
    node
}

fn build_source_mob(a: &AssetMobEntry) -> CfbStorageNode {
    let mut mob = CfbStorageNode::new("SourceMob", smpte_class_clsid(CLASS_SOURCE_MOB));
    let mut mp = PropertyBuilder::default();
    mp.add_data(PID_MOB_ID, a.source_mob_id.to_vec());
    mp.add_string(PID_MOB_NAME, &a.name);
    mp.add_data(PID_MOB_LAST_MODIFIED, vec![0u8; 8]);
    mp.add_data(PID_MOB_CREATION_TIME, vec![0u8; 8]);
    mp.add_strong_ref_vector(PID_MOB_SLOTS, "Slots-4403");
    mp.add_strong_ref(PID_SOURCE_MOB_ESSENCE_DESC, "EssenceDescription-4701");
    mob.add_stream("properties", mp.finish());

    let mut slots: Vec<CfbStorageNode> = Vec::new();
    // Slot 1: Video or Audio essence slot
    let is_audio_only = a.media_kind == MediaKind::Audio;
    let ddef = if is_audio_only {
        &DATADEF_SOUND
    } else {
        &DATADEF_PICTURE
    };
    slots.push(build_timeline_slot_single_clip(
        1,
        if is_audio_only { "A1" } else { "V1" },
        a.fps,
        ddef,
        [0u8; 32],
        0,
        0,
        a.duration_frames,
    ));
    // Slot 2: Timecode slot (when source_tc_frame > 0)
    if a.source_tc_frame > 0 {
        slots.push(build_timecode_slot(
            2,
            a.fps,
            a.source_tc_frame,
            a.duration_frames,
        ));
    }

    mob.add_stream(
        "Slots-4403 index",
        encode_strong_ref_vector_index(slots.len() as u32),
    );
    for (i, mut s) in slots.into_iter().enumerate() {
        s.name = format!("Slots-4403{{{i:x}}}");
        mob.add_storage(s);
    }

    // EssenceDescriptor with NetworkLocator
    let desc_clsid = if is_audio_only {
        smpte_class_clsid(CLASS_PCM_DESCRIPTOR)
    } else {
        smpte_class_clsid(CLASS_CDCI_DESCRIPTOR)
    };
    let mut desc = CfbStorageNode::new("EssenceDescription-4701", desc_clsid);
    let mut dp = PropertyBuilder::default();
    dp.add_rational(PID_FILE_DESC_SAMPLE_RATE, a.fps);
    dp.add_i64(PID_FILE_DESC_LENGTH, a.duration_frames);
    if !is_audio_only {
        dp.add_u32(PID_IMAGE_DESC_STORED_WIDTH, a.width.max(1));
        dp.add_u32(PID_IMAGE_DESC_STORED_HEIGHT, a.height.max(1));
    }
    if a.audio_channels > 0 {
        dp.add_u32(PID_SOUND_DESC_CHANNEL_COUNT, a.audio_channels);
        let sr = if a.audio_sample_rate > 0 {
            a.audio_sample_rate
        } else {
            48_000
        };
        dp.add_rational(PID_SOUND_DESC_AUDIO_SAMPLING_RATE, Rational::new(sr as i64, 1));
    }
    if !a.path.is_empty() {
        dp.add_strong_ref_vector(PID_DESC_LOCATOR, "Locator-2f01");
        desc.add_stream("Locator-2f01 index", encode_strong_ref_vector_index(1));
        let mut loc =
            CfbStorageNode::new("Locator-2f01{0}", smpte_class_clsid(CLASS_NETWORK_LOCATOR));
        let mut lp = PropertyBuilder::default();
        lp.add_string(PID_NET_LOCATOR_URL, &path_to_file_url(&a.path));
        loc.add_stream("properties", lp.finish());
        desc.add_storage(loc);
    }
    desc.add_stream("properties", dp.finish());
    mob.add_storage(desc);

    mob
}

fn build_master_mob(a: &AssetMobEntry) -> CfbStorageNode {
    let mut mob = CfbStorageNode::new("MasterMob", smpte_class_clsid(CLASS_MASTER_MOB));
    let mut mp = PropertyBuilder::default();
    mp.add_data(PID_MOB_ID, a.master_mob_id.to_vec());
    mp.add_string(PID_MOB_NAME, &a.name);
    mp.add_data(PID_MOB_LAST_MODIFIED, vec![0u8; 8]);
    mp.add_data(PID_MOB_CREATION_TIME, vec![0u8; 8]);
    mp.add_strong_ref_vector(PID_MOB_SLOTS, "Slots-4403");
    mob.add_stream("properties", mp.finish());

    let is_audio_only = a.media_kind == MediaKind::Audio;
    let mut slots = Vec::new();
    if !is_audio_only {
        slots.push(build_timeline_slot_single_clip(
            1,
            "V1",
            a.fps,
            &DATADEF_PICTURE,
            a.source_mob_id,
            1,
            0,
            a.duration_frames,
        ));
    }
    if is_audio_only || a.audio_channels > 0 {
        let slot_id = if is_audio_only { 1 } else { 2 };
        slots.push(build_timeline_slot_single_clip(
            slot_id,
            "A1",
            a.fps,
            &DATADEF_SOUND,
            a.source_mob_id,
            1,
            0,
            a.duration_frames,
        ));
    }

    mob.add_stream(
        "Slots-4403 index",
        encode_strong_ref_vector_index(slots.len() as u32),
    );
    for (i, mut s) in slots.into_iter().enumerate() {
        s.name = format!("Slots-4403{{{i:x}}}");
        mob.add_storage(s);
    }
    mob
}

#[allow(clippy::too_many_arguments)]
fn build_timeline_slot_single_clip(
    slot_id: u32,
    name: &str,
    fps: Rational,
    data_def: &[u8; 16],
    source_mob_id: [u8; 32],
    source_slot_id: u32,
    start_time: i64,
    length: i64,
) -> CfbStorageNode {
    let mut slot = CfbStorageNode::new("Slot", smpte_class_clsid(CLASS_TIMELINE_MOB_SLOT));
    let mut sp = PropertyBuilder::default();
    sp.add_u32(PID_SLOT_ID, slot_id);
    sp.add_string(PID_SLOT_NAME, name);
    sp.add_rational(PID_TIMELINE_SLOT_EDIT_RATE, fps);
    sp.add_i64(PID_TIMELINE_SLOT_ORIGIN, 0);
    sp.add_strong_ref(PID_SLOT_SEGMENT, "Segment-4803");
    slot.add_stream("properties", sp.finish());

    let clip = build_source_clip_node(
        "Segment-4803",
        data_def,
        length,
        source_mob_id,
        source_slot_id,
        start_time,
    );
    slot.add_storage(clip);
    slot
}

fn build_timecode_slot(
    slot_id: u32,
    fps: Rational,
    start_tc_frame: i64,
    length: i64,
) -> CfbStorageNode {
    let mut slot = CfbStorageNode::new("Slot", smpte_class_clsid(CLASS_TIMELINE_MOB_SLOT));
    let mut sp = PropertyBuilder::default();
    sp.add_u32(PID_SLOT_ID, slot_id);
    sp.add_string(PID_SLOT_NAME, "TC1");
    sp.add_rational(PID_TIMELINE_SLOT_EDIT_RATE, fps);
    sp.add_i64(PID_TIMELINE_SLOT_ORIGIN, 0);
    sp.add_strong_ref(PID_SLOT_SEGMENT, "Segment-4803");
    slot.add_stream("properties", sp.finish());

    let mut tc = CfbStorageNode::new("Segment-4803", smpte_class_clsid(CLASS_TIMECODE));
    let mut tp = PropertyBuilder::default();
    tp.add_weak_ref_auid(PID_COMP_DATA_DEF, 0, PID_DEF_IDENTIFICATION, &DATADEF_TIMECODE);
    tp.add_i64(PID_COMP_LENGTH, length.max(1));
    tp.add_i64(PID_TIMECODE_START, start_tc_frame.max(0));
    tp.add_u16(PID_TIMECODE_FPS, fps.timebase().max(1) as u16);
    tp.add_u8(
        PID_TIMECODE_DROP,
        if fps.is_ntsc() && matches!(fps.timebase(), 30 | 60) {
            1
        } else {
            0
        },
    );
    tc.add_stream("properties", tp.finish());
    slot.add_storage(tc);
    slot
}

#[allow(clippy::too_many_arguments)]
fn build_composition_mob(
    seq: &Sequence,
    comp_mob_id: [u8; 32],
    seq_mob_ids: &HashMap<&str, [u8; 32]>,
    assets: &[AssetMobEntry],
    asset_by_key: &HashMap<String, usize>,
    asset_by_id: &HashMap<String, usize>,
    options: &ConversionOptions,
) -> (CfbStorageNode, usize, usize, usize) {
    let mut mob = CfbStorageNode::new("CompositionMob", smpte_class_clsid(CLASS_COMPOSITION_MOB));
    let mut mp = PropertyBuilder::default();
    mp.add_data(PID_MOB_ID, comp_mob_id.to_vec());
    mp.add_string(PID_MOB_NAME, &seq.name);
    mp.add_data(PID_MOB_LAST_MODIFIED, vec![0u8; 8]);
    mp.add_data(PID_MOB_CREATION_TIME, vec![0u8; 8]);
    if seq.kind == SequenceKind::Timeline {
        mp.add_data(PID_MOB_USAGE_CODE, USAGE_TOP_LEVEL.to_vec());
    }
    mp.add_strong_ref_vector(PID_MOB_SLOTS, "Slots-4403");
    mob.add_stream("properties", mp.finish());

    let mut slots: Vec<CfbStorageNode> = Vec::new();
    let mut next_slot_id = 1u32;
    let total_dur = seq.computed_duration_frames().max(1);

    // Timecode slot
    slots.push(build_timecode_slot(
        next_slot_id,
        seq.frame_rate,
        seq.start_tc_frame,
        total_dur,
    ));
    next_slot_id += 1;

    let mut total_v = 0usize;
    let mut total_a = 0usize;
    let mut total_markers = 0usize;

    for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
        let (slot_node, clip_count) = build_track_slot(
            next_slot_id,
            track,
            seq.frame_rate,
            seq_mob_ids,
            assets,
            asset_by_key,
            asset_by_id,
        );
        slots.push(slot_node);
        next_slot_id += 1;
        if track.kind == TrackKind::Video {
            total_v += clip_count;
        } else {
            total_a += clip_count;
        }
    }

    if options.include_markers && !seq.markers.is_empty() {
        let marker_slot = build_marker_event_slot(next_slot_id, seq.frame_rate, &seq.markers);
        total_markers += seq.markers.len();
        slots.push(marker_slot);
    }

    mob.add_stream(
        "Slots-4403 index",
        encode_strong_ref_vector_index(slots.len() as u32),
    );
    for (i, mut s) in slots.into_iter().enumerate() {
        s.name = format!("Slots-4403{{{i:x}}}");
        mob.add_storage(s);
    }

    (mob, total_v, total_a, total_markers)
}

#[allow(clippy::too_many_arguments)]
fn build_track_slot(
    slot_id: u32,
    track: &Track,
    seq_fps: Rational,
    seq_mob_ids: &HashMap<&str, [u8; 32]>,
    assets: &[AssetMobEntry],
    asset_by_key: &HashMap<String, usize>,
    asset_by_id: &HashMap<String, usize>,
) -> (CfbStorageNode, usize) {
    let is_video = track.kind == TrackKind::Video;
    let ddef = if is_video {
        &DATADEF_PICTURE
    } else {
        &DATADEF_SOUND
    };

    let mut sorted_clips: Vec<&Clip> = track.clips.iter().filter(|c| c.enabled).collect();
    sorted_clips.sort_by_key(|c| c.start_frame);
    let mut sorted_tr: Vec<&Transition> = track.transitions.iter().collect();
    sorted_tr.sort_by_key(|t| t.start_frame);
    let mut used_tr = vec![false; sorted_tr.len()];

    let mut components: Vec<CfbStorageNode> = Vec::new();
    let mut cursor = 0i64;

    for (ci, clip) in sorted_clips.iter().enumerate() {
        // Check if a transition bridges the previous clip and this clip at clip.start_frame
        let incoming_tr = if ci > 0 {
            sorted_tr
                .iter()
                .enumerate()
                .find(|(ti, tr)| {
                    !used_tr[*ti]
                        && tr.start_frame <= clip.start_frame
                        && tr.end_frame >= clip.start_frame
                        && tr.end_frame > tr.start_frame
                })
                .map(|(ti, tr)| (ti, *tr))
        } else {
            None
        };

        if let Some((ti, tr)) = incoming_tr {
            used_tr[ti] = true;
            let tr_len = (tr.end_frame - tr.start_frame).max(1);
            let cut_point = match tr.alignment.as_str() {
                "start" => 0,
                "end" => tr_len,
                _ => (clip.start_frame - tr.start_frame).clamp(0, tr_len),
            };
            // Extend previous component in AAF Sequence by (tr_len - cut_point) and extend incoming head by cut_point
            // so the AAF Sequence's overlap subtraction yields exact timeline positions!
            let tail_ext = tr_len - cut_point;
            if let Some(prev_node) = components.last_mut() {
                extend_component_length(prev_node, tail_ext);
            }
            components.push(build_transition_node(ddef, tr_len, cut_point, is_video));

            let comp_node = build_clip_component(
                clip,
                ddef,
                is_video,
                cut_point, // extra head overlap for AAF transition
                seq_mob_ids,
                assets,
                asset_by_key,
                asset_by_id,
            );
            components.push(comp_node);
            cursor = clip.end_frame.max(cursor);
            continue;
        }

        if clip.start_frame > cursor {
            components.push(build_filler_node(ddef, clip.start_frame - cursor));
        }

        components.push(build_clip_component(
            clip,
            ddef,
            is_video,
            0,
            seq_mob_ids,
            assets,
            asset_by_key,
            asset_by_id,
        ));
        cursor = clip.end_frame.max(cursor);
    }

    let mut seq_node = CfbStorageNode::new("Segment-4803", smpte_class_clsid(CLASS_SEQUENCE));
    let mut seq_p = PropertyBuilder::default();
    seq_p.add_weak_ref_auid(PID_COMP_DATA_DEF, 0, PID_DEF_IDENTIFICATION, ddef);
    seq_p.add_i64(PID_COMP_LENGTH, cursor.max(0));
    seq_p.add_strong_ref_vector(PID_SEQ_COMPONENTS, "Components-1001");
    seq_node.add_stream("properties", seq_p.finish());
    seq_node.add_stream(
        "Components-1001 index",
        encode_strong_ref_vector_index(components.len() as u32),
    );
    for (i, mut c) in components.into_iter().enumerate() {
        c.name = format!("Components-1001{{{i:x}}}");
        seq_node.add_storage(c);
    }

    let mut slot = CfbStorageNode::new("Slot", smpte_class_clsid(CLASS_TIMELINE_MOB_SLOT));
    let mut sp = PropertyBuilder::default();
    sp.add_u32(PID_SLOT_ID, slot_id);
    sp.add_string(PID_SLOT_NAME, &track.name);
    sp.add_u32(PID_SLOT_PHYSICAL_TRACK, track.index as u32);
    sp.add_rational(PID_TIMELINE_SLOT_EDIT_RATE, seq_fps);
    sp.add_i64(PID_TIMELINE_SLOT_ORIGIN, 0);
    sp.add_strong_ref(PID_SLOT_SEGMENT, "Segment-4803");
    slot.add_stream("properties", sp.finish());
    slot.add_storage(seq_node);

    (slot, sorted_clips.len())
}

#[allow(clippy::too_many_arguments)]
fn build_clip_component(
    clip: &Clip,
    ddef: &[u8; 16],
    is_video: bool,
    head_overlap: i64,
    seq_mob_ids: &HashMap<&str, [u8; 32]>,
    assets: &[AssetMobEntry],
    asset_by_key: &HashMap<String, usize>,
    asset_by_id: &HashMap<String, usize>,
) -> CfbStorageNode {
    let dur = (clip.duration_frames().max(1)) + head_overlap;
    let src_in = (clip.source_in_frame - head_overlap).max(0);

    let (target_mob_id, target_slot_id) = if clip.source_kind != ClipSourceKind::Media
        && let Some(&nested_mob_id) = seq_mob_ids.get(clip.media_id.as_str())
    {
        (nested_mob_id, 2u32)
    } else {
        let key = if !clip.media_path.is_empty() {
            clip.media_path.clone()
        } else if !clip.media_id.is_empty() {
            format!("id:{}", clip.media_id)
        } else {
            format!("name:{}", clip.name)
        };
        let asset = asset_by_key
            .get(&key)
            .or_else(|| asset_by_id.get(&clip.media_id))
            .and_then(|&i| assets.get(i));
        match asset {
            Some(a) => {
                let slot_id = if !is_video && a.media_kind != MediaKind::Audio {
                    2u32
                } else {
                    1u32
                };
                (a.master_mob_id, slot_id)
            }
            None => ([0u8; 32], 1u32),
        }
    };

    let is_reversed = clip.reversed || clip.speed_percent < 0.0;
    let abs_speed = clip.speed_percent.abs();
    let is_retimed = is_reversed || (abs_speed - 100.0).abs() > 0.05;

    let src_clip = build_source_clip_node(
        if is_retimed {
            "InputSegments-b02{0}"
        } else {
            "Component"
        },
        ddef,
        dur,
        target_mob_id,
        target_slot_id,
        src_in,
    );

    if !is_retimed {
        return src_clip;
    }

    // Wrap retimed/reversed/freeze clip in a VideoSpeedControl OperationGroup
    let signed_pct = if is_reversed {
        -abs_speed
    } else {
        abs_speed
    };
    let speed_rat = if abs_speed < 0.01 {
        Rational::ZERO
    } else {
        Rational::new((signed_pct * 100.0).round() as i64, 10_000)
    };

    let mut param =
        CfbStorageNode::new("Parameters-b03{0}", smpte_class_clsid(CLASS_CONSTANT_VALUE));
    let mut pp = PropertyBuilder::default();
    pp.add_data(PID_PARAM_DEF, PARAMDEF_SPEED_RATIO.to_vec());
    pp.add_data(PID_CONST_VAL_VALUE, encode_indirect_rational(speed_rat));
    param.add_stream("properties", pp.finish());

    let mut og = CfbStorageNode::new("Component", smpte_class_clsid(CLASS_OPERATION_GROUP));
    let mut ogp = PropertyBuilder::default();
    ogp.add_weak_ref_auid(PID_COMP_DATA_DEF, 0, PID_DEF_IDENTIFICATION, ddef);
    ogp.add_i64(PID_COMP_LENGTH, dur);
    ogp.add_weak_ref_auid(
        PID_OP_GROUP_OPERATION,
        1,
        PID_DEF_IDENTIFICATION,
        &OPDEF_VIDEO_SPEED_CONTROL,
    );
    ogp.add_strong_ref_vector(PID_OP_GROUP_INPUT_SEGMENTS, "InputSegments-b02");
    ogp.add_strong_ref_vector(PID_OP_GROUP_PARAMETERS, "Parameters-b03");
    og.add_stream("properties", ogp.finish());
    og.add_stream("InputSegments-b02 index", encode_strong_ref_vector_index(1));
    og.add_storage(src_clip);
    og.add_stream("Parameters-b03 index", encode_strong_ref_vector_index(1));
    og.add_storage(param);
    og
}

fn build_source_clip_node(
    name: &str,
    ddef: &[u8; 16],
    length: i64,
    source_mob_id: [u8; 32],
    source_slot_id: u32,
    start_time: i64,
) -> CfbStorageNode {
    let mut node = CfbStorageNode::new(name, smpte_class_clsid(CLASS_SOURCE_CLIP));
    let mut p = PropertyBuilder::default();
    p.add_weak_ref_auid(PID_COMP_DATA_DEF, 0, PID_DEF_IDENTIFICATION, ddef);
    p.add_i64(PID_COMP_LENGTH, length.max(1));
    p.add_data(PID_SRC_REF_SOURCE_ID, source_mob_id.to_vec());
    p.add_u32(PID_SRC_REF_SOURCE_SLOT_ID, source_slot_id);
    p.add_i64(PID_SRC_CLIP_START_TIME, start_time.max(0));
    node.add_stream("properties", p.finish());
    node
}

fn build_filler_node(ddef: &[u8; 16], length: i64) -> CfbStorageNode {
    let mut node = CfbStorageNode::new("Filler", smpte_class_clsid(CLASS_FILLER));
    let mut p = PropertyBuilder::default();
    p.add_weak_ref_auid(PID_COMP_DATA_DEF, 0, PID_DEF_IDENTIFICATION, ddef);
    p.add_i64(PID_COMP_LENGTH, length.max(1));
    node.add_stream("properties", p.finish());
    node
}

fn build_transition_node(
    ddef: &[u8; 16],
    length: i64,
    cut_point: i64,
    is_video: bool,
) -> CfbStorageNode {
    let op_auid = if is_video {
        &OPDEF_VIDEO_DISSOLVE
    } else {
        &OPDEF_AUDIO_DISSOLVE
    };

    let mut og = CfbStorageNode::new("OperationGroup-1702", smpte_class_clsid(CLASS_OPERATION_GROUP));
    let mut ogp = PropertyBuilder::default();
    ogp.add_weak_ref_auid(PID_COMP_DATA_DEF, 0, PID_DEF_IDENTIFICATION, ddef);
    ogp.add_i64(PID_COMP_LENGTH, length);
    ogp.add_weak_ref_auid(PID_OP_GROUP_OPERATION, 1, PID_DEF_IDENTIFICATION, op_auid);
    og.add_stream("properties", ogp.finish());

    let mut tr = CfbStorageNode::new("Transition", smpte_class_clsid(CLASS_TRANSITION));
    let mut tp = PropertyBuilder::default();
    tp.add_weak_ref_auid(PID_COMP_DATA_DEF, 0, PID_DEF_IDENTIFICATION, ddef);
    tp.add_i64(PID_COMP_LENGTH, length);
    tp.add_i64(PID_TRANSITION_CUT_POINT, cut_point);
    tp.add_strong_ref(PID_TRANSITION_OP_GROUP, "OperationGroup-1702");
    tr.add_stream("properties", tp.finish());
    tr.add_storage(og);
    tr
}

fn build_marker_event_slot(
    slot_id: u32,
    seq_fps: Rational,
    markers: &[nle_core::Marker],
) -> CfbStorageNode {
    let mut seq = CfbStorageNode::new("Segment-4803", smpte_class_clsid(CLASS_SEQUENCE));
    let mut sp = PropertyBuilder::default();
    sp.add_strong_ref_vector(PID_SEQ_COMPONENTS, "Components-1001");
    seq.add_stream("properties", sp.finish());
    seq.add_stream(
        "Components-1001 index",
        encode_strong_ref_vector_index(markers.len() as u32),
    );

    for (i, m) in markers.iter().enumerate() {
        let mut mk = CfbStorageNode::new(
            format!("Components-1001{{{i:x}}}"),
            smpte_class_clsid(CLASS_COMMENT_MARKER),
        );
        let mut mp = PropertyBuilder::default();
        mp.add_i64(PID_EVENT_POSITION, m.frame.max(0));
        mp.add_i64(PID_COMP_LENGTH, m.duration_frames.max(1));
        let text = if m.comment.is_empty() {
            m.name.clone()
        } else if m.name.is_empty() {
            m.comment.clone()
        } else {
            format!("{}: {}", m.name, m.comment)
        };
        mp.add_string(PID_EVENT_COMMENT, &text);
        if !m.color.is_empty() {
            mp.add_string(PID_COMMENT_MARKER_COLOR, &m.color);
        }
        mk.add_stream("properties", mp.finish());
        seq.add_storage(mk);
    }

    let mut slot = CfbStorageNode::new("Slot", smpte_class_clsid(CLASS_EVENT_MOB_SLOT));
    let mut slp = PropertyBuilder::default();
    slp.add_u32(PID_SLOT_ID, slot_id);
    slp.add_string(PID_SLOT_NAME, "Markers");
    slp.add_rational(PID_EVENT_SLOT_EDIT_RATE, seq_fps);
    slp.add_strong_ref(PID_SLOT_SEGMENT, "Segment-4803");
    slot.add_stream("properties", slp.finish());
    slot.add_storage(seq);
    slot
}

fn extend_component_length(node: &mut CfbStorageNode, extra_frames: i64) {
    if extra_frames <= 0 {
        return;
    }
    if let Some((_, prop_bytes)) = node.streams.iter_mut().find(|(n, _)| n == "properties")
        && let Ok(mut pmap) = PropertyMap::parse(prop_bytes)
        && let Some(len_prop) = pmap.props.get_mut(&PID_COMP_LENGTH)
        && len_prop.data.len() == 8
    {
        let cur = read_i64(&len_prop.data, 0, true);
        len_prop.data = (cur + extra_frames).to_le_bytes().to_vec();
        let mut pb = PropertyBuilder::default();
        for p in pmap.props.into_values() {
            pb.add_raw(p.pid, p.stored_form, p.data);
        }
        *prop_bytes = pb.finish();
    }
}
