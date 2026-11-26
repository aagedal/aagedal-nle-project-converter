use crate::audio::{
    mono_channel_mapping, rebuild_fairlight_track_model, stereo_channel_mapping, FairlightBusSpec,
    FairlightTrackSpec,
};
use crate::binary::{
    decode_keyed_fields, decode_protobuf, decode_wrapper_hex, encode_keyed_fields, encode_protobuf,
    encode_utf16be, encode_utf16le, encode_wrapper_hex, hex_decode, hex_encode, replace_subslice,
    ProtoField, ProtoValue,
};
use crate::grade::{build_grade_and_stabilization_body, build_version_table_xml};
use crate::state::{
    append_lockable_blob, build_sequence_setup_blob, clip_color_lockable_blob_xml,
    configure_project_color_blob, media_metadata_lockable_blob_xml, patch_project_setup_defaults,
    premiere_label_to_resolve_color, sequence_lockable_blob_xml,
};
use crate::timemap::{build_media_timemap, build_still_timemap, retimed_source_in, STILL_SOURCE_IN};
use nle_core::{
    Clip, ClipSourceKind, ConversionMessage, ConversionOptions, ConversionReport, MediaKind,
    MotionTransform, NleError, NleFormat, NleResult, Project, ProjectWriter, Sequence,
    SequenceKind, Severity,
};
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const RESOLVE_TEMPLATE_BYTES: &[u8] = include_bytes!("../resources/resolve21_native_template.drp");

pub struct ResolveWriter;

impl ProjectWriter for ResolveWriter {
    fn format(&self) -> NleFormat {
        NleFormat::DaVinciResolve
    }

    fn write_file(
        &self,
        project: &Project,
        output_path: &Path,
        options: &ConversionOptions,
    ) -> NleResult<ConversionReport> {
        write_drp_file(project, output_path, options)
    }
}

/// Write a native DaVinci Resolve `.drp` ZIP project archive directly from our canonical `Project` IR.
pub fn write_drp_file(
    project: &Project,
    output_path: &Path,
    options: &ConversionOptions,
) -> NleResult<ConversionReport> {
    let mut selected_sequences: Vec<&Sequence> = if options.selected_sequence_ids.is_empty() {
        project.sequences.iter().collect()
    } else {
        project
            .sequences
            .iter()
            .filter(|s| options.selected_sequence_ids.contains(&s.id))
            .collect()
    };

    if selected_sequences.is_empty() {
        return Err(NleError::InvalidFormat(
            "No sequences selected to convert into DaVinci Resolve .drp".into(),
        ));
    }

    // Automatically include any nested, compound, or multicam sequences referenced by selected sequences
    // so MediaRef links are never dangling.
    let mut selected_ids: std::collections::HashSet<&str> =
        selected_sequences.iter().map(|s| s.id.as_str()).collect();
    let seq_by_id: HashMap<&str, &Sequence> =
        project.sequences.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut scan_idx = 0usize;
    while scan_idx < selected_sequences.len() {
        let seq = selected_sequences[scan_idx];
        scan_idx += 1;
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            for clip in &track.clips {
                if clip.source_kind != ClipSourceKind::Media && !clip.media_id.is_empty() {
                    if let Some(&dep_seq) = seq_by_id.get(clip.media_id.as_str()) {
                        if selected_ids.insert(dep_seq.id.as_str()) {
                            selected_sequences.push(dep_seq);
                        }
                    }
                }
            }
        }
    }

    let mut members = load_template_members()?;
    let pool_key = "MediaPool/Master/MpFolder.xml".to_string();
    let orig_seq_key = members
        .keys()
        .find(|k| k.starts_with("SeqContainer/") && k.ends_with(".xml"))
        .cloned()
        .ok_or_else(|| NleError::InvalidFormat("Template DRP missing SeqContainer XML".into()))?;

    let orig_pool_xml = String::from_utf8(
        members
            .get(&pool_key)
            .cloned()
            .ok_or_else(|| NleError::InvalidFormat("Template DRP missing MpFolder.xml".into()))?,
    )
    .map_err(|e| NleError::InvalidFormat(e.to_string()))?;

    let orig_seq_xml = String::from_utf8(members.remove(&orig_seq_key).unwrap())
        .map_err(|e| NleError::InvalidFormat(e.to_string()))?;

    // Extract reusable node templates from the Resolve 21 scaffold
    let mp_video_templates = extract_elements_by_tag(&orig_pool_xml, "Sm2MpVideoClip");
    let mp_audio_templates = extract_elements_by_tag(&orig_pool_xml, "Sm2MpAudioClip");
    let mp_timeline_template = extract_elements_by_tag(&orig_pool_xml, "Sm2MpTimelineClip")
        .into_iter()
        .next()
        .ok_or_else(|| NleError::InvalidFormat("Template missing Sm2MpTimelineClip".into()))?;

    let mp_video_tpl = mp_video_templates
        .iter()
        .find(|s| s.contains("neutral-source.mp4"))
        .or_else(|| mp_video_templates.first())
        .ok_or_else(|| NleError::InvalidFormat("Template missing video media pool node".into()))?
        .clone();
    let mp_still_tpl = mp_video_templates
        .iter()
        .find(|s| s.contains("neutral-still.png"))
        .unwrap_or(&mp_video_tpl)
        .clone();
    let mp_audio_tpl = mp_audio_templates
        .iter()
        .find(|s| s.contains("neutral-audio.wav"))
        .or_else(|| mp_audio_templates.first())
        .ok_or_else(|| NleError::InvalidFormat("Template missing audio media pool node".into()))?
        .clone();

    let ti_video_templates = extract_elements_by_tag(&orig_seq_xml, "Sm2TiVideoClip");
    let ti_audio_templates = extract_elements_by_tag(&orig_seq_xml, "Sm2TiAudioClip");
    let ti_video_tpl = ti_video_templates
        .iter()
        .find(|s| s.contains("neutral-source.mp4"))
        .or_else(|| ti_video_templates.first())
        .ok_or_else(|| NleError::InvalidFormat("Template missing Sm2TiVideoClip".into()))?
        .clone();
    let ti_still_tpl = ti_video_templates
        .iter()
        .find(|s| s.contains("neutral-still.png"))
        .unwrap_or(&ti_video_tpl)
        .clone();
    let ti_audio_tpl = ti_audio_templates
        .first()
        .ok_or_else(|| NleError::InvalidFormat("Template missing Sm2TiAudioClip".into()))?
        .clone();
    let ti_audio_wav_tpl = ti_audio_templates
        .iter()
        .find(|s| s.contains("neutral-audio.wav"))
        .unwrap_or(&ti_audio_tpl)
        .clone();

    let vtrack_vec_inner = extract_tag_inner(&orig_seq_xml, "VideoTrackVec")
        .ok_or_else(|| NleError::InvalidFormat("Template missing VideoTrackVec".into()))?;
    let atrack_vec_inner = extract_tag_inner(&orig_seq_xml, "AudioTrackVec")
        .ok_or_else(|| NleError::InvalidFormat("Template missing AudioTrackVec".into()))?;
    let vtrack_tpl = extract_elements_by_tag(&vtrack_vec_inner, "Sm2TiTrack")
        .into_iter()
        .next()
        .ok_or_else(|| NleError::InvalidFormat("Template missing video Sm2TiTrack".into()))?;
    let atrack_tpl = extract_elements_by_tag(&atrack_vec_inner, "Sm2TiTrack")
        .into_iter()
        .next()
        .ok_or_else(|| NleError::InvalidFormat("Template missing audio Sm2TiTrack".into()))?;

    let orig_container_id = extract_attr(&orig_seq_xml, "Sm2SequenceContainer", "DbId")
        .ok_or_else(|| NleError::InvalidFormat("Template missing Sm2SequenceContainer DbId".into()))?;
    let orig_sequence_ref = extract_tag_inner(&orig_seq_xml, "Sequence")
        .ok_or_else(|| NleError::InvalidFormat("Template missing <Sequence> ref".into()))?;
    let template_master_folder_id = extract_attr(&orig_pool_xml, "Sm2MpFolder", "DbId")
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Index project MediaAssets by id & normalized path
    let asset_by_id: HashMap<&str, &nle_core::MediaAsset> = project
        .media_assets
        .iter()
        .map(|a| (a.id.as_str(), a))
        .collect();
    let asset_by_path: HashMap<String, &nle_core::MediaAsset> = project
        .media_assets
        .iter()
        .filter(|a| !a.path.is_empty())
        .map(|a| (path_key(&a.path, &a.name), a))
        .collect();

    // Canonical deduplicated media pool entries: path_key -> (db_id, xml_block, is_video)
    let mut media_pool_ids: HashMap<String, String> = HashMap::new();
    let mut media_pool_entries: Vec<(String, String)> = Vec::new(); // (path_key, <Element>...</Element>)
    let mut video_clip_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Pre-scan which media paths appear on video tracks (skip sequence-backed nested/multicam clips)
    for seq in &selected_sequences {
        for track in &seq.video_tracks {
            for clip in &track.clips {
                if clip.source_kind == ClipSourceKind::Media {
                    video_clip_paths.insert(path_key(&clip.media_path, &clip.name));
                }
            }
        }
    }

    // Pre-allocate Resolve DbIds for every sequence so nested/compound/multicam clips
    // can reference the target Sm2MpTimelineClip / Sm2MpCompoundClip / Sm2MpMulticamClip DbId.
    struct SeqResolveIds {
        container_id: String,
        seq_db_id: String,
        timeline_id: String,
        mp_clip_id: String,
        unique_seq_id: String,
        unique_mp_id: String,
        duration_frames: i64,
        fps: f64,
    }
    let mut seq_resolve_ids: HashMap<&str, SeqResolveIds> = HashMap::new();
    for seq in &selected_sequences {
        seq_resolve_ids.insert(
            seq.id.as_str(),
            SeqResolveIds {
                container_id: Uuid::new_v4().to_string(),
                seq_db_id: Uuid::new_v4().to_string(),
                timeline_id: Uuid::new_v4().to_string(),
                mp_clip_id: Uuid::new_v4().to_string(),
                unique_seq_id: Uuid::new_v4().to_string(),
                unique_mp_id: Uuid::new_v4().to_string(),
                duration_frames: seq.computed_duration_frames().max(2),
                fps: seq.frame_rate.as_f64(),
            },
        );
    }

    let mut timeline_clip_blocks: Vec<(String, String)> = Vec::new(); // (source_seq_id, <Element>...</Element>)
    let mut timeline_handles: Vec<String> = Vec::new();
    let mut lockable_blobs: Vec<String> = Vec::new();
    let mut messages: Vec<ConversionMessage> = Vec::new();

    let mut total_video_clips = 0usize;
    let mut total_audio_clips = 0usize;
    let mut total_markers = 0usize;
    let mut total_grades = 0usize;

    // Choose project default resolution/fps from the first non-empty regular timeline
    let primary_seq = selected_sequences
        .iter()
        .copied()
        .find(|s| s.kind == SequenceKind::Timeline && s.total_clip_count() > 0)
        .or_else(|| selected_sequences.iter().copied().find(|s| s.total_clip_count() > 0))
        .unwrap_or(selected_sequences[0]);
    let default_w = primary_seq.width;
    let default_h = primary_seq.height;
    let default_fps_int = primary_seq.frame_rate.timebase().max(1) as u32;

    for seq in &selected_sequences {
        let sids = &seq_resolve_ids[seq.id.as_str()];
        let new_container_id = sids.container_id.clone();
        let new_seq_db_id = sids.seq_db_id.clone();
        let new_timeline_id = sids.timeline_id.clone();
        let new_timeline_clip_id = sids.mp_clip_id.clone();
        let new_unique_seq_id = sids.unique_seq_id.clone();
        let new_unique_mp_id = sids.unique_mp_id.clone();
        let seq_fps = seq.frame_rate.as_f64();
        let tc_offset = seq.start_tc_frame.max(0);

        // Pre-assign Resolve DbIds to every clip in this sequence so linked clips (Premiere <Link> groups)
        // can reference each other's DbIds inside FieldsBlob f1.f1.f1.
        let mut video_clip_db_ids: Vec<Vec<String>> = Vec::with_capacity(seq.video_tracks.len());
        let mut audio_clip_db_ids: Vec<Vec<String>> = Vec::with_capacity(seq.audio_tracks.len());
        let mut link_groups: HashMap<String, Vec<String>> = HashMap::new();
        for track in &seq.video_tracks {
            let mut ids = Vec::with_capacity(track.clips.len());
            for clip in &track.clips {
                let db_id = Uuid::new_v4().to_string();
                if let Some(lg) = &clip.link_group_id {
                    link_groups.entry(lg.clone()).or_default().push(db_id.clone());
                }
                ids.push(db_id);
            }
            video_clip_db_ids.push(ids);
        }
        for track in &seq.audio_tracks {
            let mut ids = Vec::with_capacity(track.clips.len());
            for clip in &track.clips {
                let db_id = Uuid::new_v4().to_string();
                if let Some(lg) = &clip.link_group_id {
                    link_groups.entry(lg.clone()).or_default().push(db_id.clone());
                }
                ids.push(db_id);
            }
            audio_clip_db_ids.push(ids);
        }

        // Build video tracks XML
        let mut video_tracks_xml = String::new();
        for (t_idx, track) in seq.video_tracks.iter().enumerate() {
            let mut timed_items: Vec<(i64, u8, String)> = Vec::new();
            for (c_idx, clip) in track.clips.iter().enumerate() {
                let container_target = if clip.source_kind != ClipSourceKind::Media {
                    seq_resolve_ids.get(clip.media_id.as_str())
                } else {
                    None
                };

                let (media_ref_id, effective_media_path, src_fps, asset_dur) =
                    if let Some(target) = container_target {
                        (
                            target.mp_clip_id.clone(),
                            String::new(),
                            target.fps,
                            target
                                .duration_frames
                                .max(clip.source_out_frame)
                                .max(clip.duration_frames())
                                .max(2),
                        )
                    } else {
                        let pkey = path_key(&clip.media_path, &clip.name);
                        let matched_asset = asset_by_id
                            .get(clip.media_id.as_str())
                            .copied()
                            .or_else(|| asset_by_path.get(&pkey).copied());
                        let mref = ensure_media_pool_item(
                            &pkey,
                            clip,
                            matched_asset,
                            true,
                            &mp_video_tpl,
                            &mp_still_tpl,
                            &mp_audio_tpl,
                            &mut media_pool_ids,
                            &mut media_pool_entries,
                            &mut lockable_blobs,
                        )?;
                        let dur = matched_asset
                            .map(|a| a.duration_frames)
                            .unwrap_or(0)
                            .max(clip.source_out_frame)
                            .max(clip.duration_frames())
                            .max(2);
                        (mref, clip.media_path.clone(), clip.source_fps.as_f64(), dur)
                    };

                let chosen_tpl = if clip.media_kind == MediaKind::Image && container_target.is_none() {
                    &ti_still_tpl
                } else {
                    &ti_video_tpl
                };
                let clip_db_id = video_clip_db_ids[t_idx][c_idx].clone();
                let mut node = replace_root_db_id(chosen_tpl, &clip_db_id);
                let abs_start = tc_offset + clip.start_frame.max(0);
                node = set_child_text(&node, "Name", &xml_escape(&clip.name));
                node = set_child_text(&node, "Start", &abs_start.to_string());
                node = set_child_text(&node, "Duration", &clip.duration_frames().max(1).to_string());

                let signed_speed = if clip.reversed && clip.speed_percent > 0.0 {
                    -clip.speed_percent
                } else {
                    clip.speed_percent
                };
                let source_in = if clip.media_kind == MediaKind::Image && container_target.is_none() {
                    STILL_SOURCE_IN.to_string()
                } else {
                    retimed_source_in(
                        clip.source_in_frame,
                        clip.source_out_frame,
                        asset_dur,
                        src_fps,
                        seq_fps,
                        signed_speed,
                        clip.reversed,
                    )
                };
                node = set_child_text(&node, "In", &source_in);
                node = set_child_text(&node, "MediaRef", &media_ref_id);
                node = set_child_text(&node, "MediaFilePath", &xml_escape(&effective_media_path));
                node = set_child_text(&node, "CurrentSelectorIdx", "0");
                if container_target.is_some() {
                    node = set_child_text(&node, "IsForceConformed", "true");
                }
                let mut fps_bytes = Vec::with_capacity(16);
                fps_bytes.extend_from_slice(&src_fps.to_le_bytes());
                fps_bytes.extend_from_slice(&[0u8; 8]);
                node = set_child_text(&node, "MediaFrameRate", &hex_encode(&fps_bytes));

                let timemap = if clip.media_kind == MediaKind::Image && container_target.is_none() {
                    build_still_timemap()
                } else {
                    build_media_timemap(asset_dur, src_fps, signed_speed, false)
                };
                node = set_child_text(&node, "MediaTimemapBA", &timemap);

                // Clip FieldsBlob: linked sibling UUIDs (Premiere <Link>) + active multicam angle
                let sibling_ids: Vec<String> = clip
                    .link_group_id
                    .as_deref()
                    .and_then(|lg| link_groups.get(lg))
                    .map(|g| g.iter().filter(|id| **id != clip_db_id).cloned().collect())
                    .unwrap_or_default();
                if let Some(fb_hex) =
                    build_clip_fields_blob(&sibling_ids, clip.active_angle.as_deref())
                {
                    node = set_child_text(&node, "FieldsBlob", &fb_hex);
                }

                // Motion static transform + Opacity
                let motion_hex = build_static_video_effects_payload(
                    &clip.motion,
                    clip.opacity,
                    (clip.source_width, clip.source_height),
                    (seq.width, seq.height),
                );
                if clip.motion.has_keyframes {
                    messages.push(ConversionMessage {
                        severity: Severity::Warning,
                        code: "MOTION_KF".into(),
                        sequence: Some(seq.name.clone()),
                        clip: Some(clip.name.clone()),
                        message: "Animated Motion keyframes converted using initial keyframe values; review animation curve in Resolve.".into(),
                    });
                }
                node = set_child_text(&node, "EffectFiltersBA", motion_hex.as_deref().unwrap_or(""));

                // Lumetri Color -> native DRX grade attachment, and/or activated Inspector Stabilization
                let lumetri_slice: &[nle_core::LumetriData] = if options.include_lumetri_grades {
                    &clip.lumetri_effects
                } else {
                    &[]
                };
                let enable_stabilization = clip.effects.iter().any(|e| {
                    e.enabled
                        && (e.category == nle_core::EffectCategory::Stabilization
                            || e.match_name
                                .to_ascii_lowercase()
                                .contains("subspacestabilizer")
                            || e.match_name.to_ascii_lowercase().contains("stabiliz"))
                });
                if let Some(body) = build_grade_and_stabilization_body(
                    lumetri_slice,
                    enable_stabilization,
                    seq.width,
                    seq.height,
                ) {
                    let ver_table = build_version_table_xml(&body);
                    node = replace_or_insert_block(&node, "pLmVerTable", &ver_table, "</Sm2TiVideoClip>");
                    if !lumetri_slice.is_empty() {
                        total_grades += 1;
                    }
                }

                // Clip color label
                if options.include_clip_colors {
                    if let Some(preset) = clip.label_preset_index {
                        if let Some(resolve_color) = premiere_label_to_resolve_color(preset) {
                            lockable_blobs.push(clip_color_lockable_blob_xml(&clip_db_id, resolve_color));
                        }
                    }
                }

                timed_items.push((abs_start, 0, format!("<Element>{node}</Element>")));
                total_video_clips += 1;
            }

            for tr in &track.transitions {
                if let Some(tr_xml) = build_resolve_transition_xml(tr, tc_offset, false) {
                    let abs_start = tc_offset + tr.start_frame.max(0);
                    timed_items.push((abs_start, 1, format!("<Element>{tr_xml}</Element>")));
                }
            }
            timed_items.sort_by_key(|(s, ord, _)| (*s, *ord));
            let items_xml: String = timed_items.into_iter().map(|(_, _, x)| x).collect();

            let track_xml = build_track_xml(&vtrack_tpl, track, &items_xml, false);
            video_tracks_xml.push_str("<Element>");
            video_tracks_xml.push_str(&track_xml);
            video_tracks_xml.push_str("</Element>");
        }

        // Build audio tracks XML
        let mut audio_tracks_xml = String::new();
        for (t_idx, track) in seq.audio_tracks.iter().enumerate() {
            let mut timed_items: Vec<(i64, u8, String)> = Vec::new();
            for (c_idx, clip) in track.clips.iter().enumerate() {
                let container_target = if clip.source_kind != ClipSourceKind::Media {
                    seq_resolve_ids.get(clip.media_id.as_str())
                } else {
                    None
                };

                let (media_ref_id, effective_media_path, is_video_backed, src_fps, asset_dur) =
                    if let Some(target) = container_target {
                        (
                            target.mp_clip_id.clone(),
                            String::new(),
                            true,
                            target.fps,
                            target
                                .duration_frames
                                .max(clip.source_out_frame)
                                .max(clip.duration_frames())
                                .max(2),
                        )
                    } else {
                        let pkey = path_key(&clip.media_path, &clip.name);
                        let video_backed = video_clip_paths.contains(&pkey);
                        let matched_asset = asset_by_id
                            .get(clip.media_id.as_str())
                            .copied()
                            .or_else(|| asset_by_path.get(&pkey).copied());
                        let mref = ensure_media_pool_item(
                            &pkey,
                            clip,
                            matched_asset,
                            video_backed,
                            &mp_video_tpl,
                            &mp_still_tpl,
                            &mp_audio_tpl,
                            &mut media_pool_ids,
                            &mut media_pool_entries,
                            &mut lockable_blobs,
                        )?;
                        let dur = matched_asset
                            .map(|a| a.duration_frames)
                            .unwrap_or(0)
                            .max(clip.source_out_frame)
                            .max(clip.duration_frames())
                            .max(2);
                        (mref, clip.media_path.clone(), video_backed, clip.source_fps.as_f64(), dur)
                    };

                let chosen_tpl = if is_video_backed {
                    &ti_audio_tpl
                } else {
                    &ti_audio_wav_tpl
                };
                let clip_db_id = audio_clip_db_ids[t_idx][c_idx].clone();
                let mut node = replace_root_db_id(chosen_tpl, &clip_db_id);
                let abs_start = tc_offset + clip.start_frame.max(0);
                node = set_child_text(&node, "Name", &xml_escape(&clip.name));
                node = set_child_text(&node, "Start", &abs_start.to_string());
                node = set_child_text(&node, "Duration", &clip.duration_frames().max(1).to_string());
                let is_stereo_track = track.audio_channels >= 2;
                let vat_hex = if is_stereo_track {
                    stereo_channel_mapping(clip.audio_channel.max(1))
                } else {
                    mono_channel_mapping(clip.audio_channel.max(1))
                };
                node = set_child_text(&node, "VirtualAudioTrackBA", &vat_hex);

                let signed_speed = if clip.reversed && clip.speed_percent > 0.0 {
                    -clip.speed_percent
                } else {
                    clip.speed_percent
                };
                let source_in = retimed_source_in(
                    clip.source_in_frame,
                    clip.source_out_frame,
                    asset_dur,
                    src_fps,
                    seq_fps,
                    signed_speed,
                    clip.reversed,
                );
                node = set_child_text(&node, "In", &source_in);
                node = set_child_text(&node, "MediaRef", &media_ref_id);
                node = set_child_text(&node, "MediaFilePath", &xml_escape(&effective_media_path));
                if container_target.is_some() {
                    node = set_child_text(&node, "IsForceConformed", "true");
                }
                let mut fps_bytes = Vec::with_capacity(16);
                fps_bytes.extend_from_slice(&src_fps.to_le_bytes());
                fps_bytes.extend_from_slice(&[0u8; 8]);
                node = set_child_text(&node, "MediaFrameRate", &hex_encode(&fps_bytes));

                let timemap = build_media_timemap(asset_dur, src_fps, signed_speed, true);
                node = set_child_text(&node, "MediaTimemapBA", &timemap);
                node = set_child_text(&node, "EffectFiltersBA", "");

                let sibling_ids: Vec<String> = clip
                    .link_group_id
                    .as_deref()
                    .and_then(|lg| link_groups.get(lg))
                    .map(|g| g.iter().filter(|id| **id != clip_db_id).cloned().collect())
                    .unwrap_or_default();
                if let Some(fb_hex) =
                    build_clip_fields_blob(&sibling_ids, clip.active_angle.as_deref())
                {
                    node = set_child_text(&node, "FieldsBlob", &fb_hex);
                }

                if options.include_clip_colors {
                    if let Some(preset) = clip.label_preset_index {
                        if let Some(resolve_color) = premiere_label_to_resolve_color(preset) {
                            lockable_blobs.push(clip_color_lockable_blob_xml(&clip_db_id, resolve_color));
                        }
                    }
                }

                timed_items.push((abs_start, 0, format!("<Element>{node}</Element>")));
                total_audio_clips += 1;
            }

            for tr in &track.transitions {
                if let Some(tr_xml) = build_resolve_transition_xml(tr, tc_offset, true) {
                    let abs_start = tc_offset + tr.start_frame.max(0);
                    timed_items.push((abs_start, 1, format!("<Element>{tr_xml}</Element>")));
                }
            }
            timed_items.sort_by_key(|(s, ord, _)| (*s, *ord));
            let items_xml: String = timed_items.into_iter().map(|(_, _, x)| x).collect();

            let track_xml = build_track_xml(&atrack_tpl, track, &items_xml, true);
            audio_tracks_xml.push_str("<Element>");
            audio_tracks_xml.push_str(&track_xml);
            audio_tracks_xml.push_str("</Element>");
        }

        // Assemble SeqContainer/<new_container_id>.xml
        let mut seq_container_xml = orig_seq_xml.replacen(&orig_container_id, &new_container_id, 1);
        seq_container_xml = set_tag_inner(&seq_container_xml, "VideoTrackVec", &video_tracks_xml);
        seq_container_xml = set_tag_inner(&seq_container_xml, "AudioTrackVec", &audio_tracks_xml);
        seq_container_xml = seq_container_xml.replace(&orig_sequence_ref, &new_seq_db_id);
        members.insert(
            format!("SeqContainer/{new_container_id}.xml"),
            seq_container_xml.into_bytes(),
        );

        // Assemble MediaPool container:
        // - SequenceKind::CompoundClip -> <Sm2MpCompoundClip> (SequenceType=1)
        // - SequenceKind::MulticamClip -> <Sm2MpMulticamClip> (SequenceType=2)
        // - SequenceKind::Timeline     -> <Sm2MpTimelineClip> (added to TimelineHandles)
        let tl_block = if matches!(seq.kind, SequenceKind::CompoundClip | SequenceKind::MulticamClip)
        {
            let tag = if seq.kind == SequenceKind::CompoundClip {
                "Sm2MpCompoundClip"
            } else {
                "Sm2MpMulticamClip"
            };
            let seq_type_code: i32 = if seq.kind == SequenceKind::CompoundClip {
                1
            } else {
                2
            };
            let escaped_seq_name = xml_escape(&seq.name);
            let mut fr_hex_bytes = Vec::with_capacity(16);
            fr_hex_bytes.extend_from_slice(&seq_fps.to_le_bytes());
            fr_hex_bytes.extend_from_slice(&[0u8; 8]);
            let fr_hex = hex_encode(&fr_hex_bytes);

            let mut res_bytes = Vec::with_capacity(16);
            res_bytes.extend_from_slice(&(seq.width as u64).to_be_bytes());
            res_bytes.extend_from_slice(&(seq.height as u64).to_be_bytes());
            let res_hex = hex_encode(&res_bytes);

            let start_sec = tc_offset as f64 / seq_fps.max(1.0);
            let dur_sec = seq.computed_duration_frames().max(0) as f64 / seq_fps.max(1.0);
            let mut ext_bytes = Vec::with_capacity(16);
            ext_bytes.extend_from_slice(&start_sec.to_le_bytes());
            ext_bytes.extend_from_slice(&dur_sec.to_le_bytes());
            let ext_hex = hex_encode(&ext_bytes);

            let seq_blob_entries = vec![
                ("SequenceType".into(), 2u32, seq_type_code.to_be_bytes().to_vec()),
                ("SeqRef".into(), 10u32, encode_utf16be(&new_container_id)),
                ("IsRenderCacheBAVersioned".into(), 1u32, vec![1u8]),
            ];
            let seq_blob_hex = hex_encode(&encode_keyed_fields(&seq_blob_entries));

            let vm_entries = vec![
                ("UniqueId".into(), 10u32, encode_utf16be(&Uuid::new_v4().to_string())),
                ("DbType".into(), 10u32, encode_utf16be("BtVideoMetadata")),
            ];
            let vm_hex = hex_encode(&encode_keyed_fields(&vm_entries));
            let ptzr_id = Uuid::new_v4().to_string();
            let num_audio_ch = (seq.audio_tracks.len().max(2) as u32).min(16);

            format!(
                "<{tag} DbId=\"{new_timeline_clip_id}\">\
                 <FieldsBlob/>\
                 <Name>{escaped_seq_name}</Name>\
                 <MpFolder>{template_master_folder_id}</MpFolder>\
                 <UniqueMediaPoolItemId>{new_unique_mp_id}</UniqueMediaPoolItemId>\
                 <MarkIn/><MarkInVideo/><MarkInAudio/><MarkOut/><MarkOutVideo/><MarkOutAudio/>\
                 <CurPlayheadPosition/><PinsBA/><VirtualAudioTracksBA/><MatteVec/>\
                 <AudioSource>AUDIO_SOURCE_EMBEDDED</AudioSource>\
                 <PTZRPreset><SmPTZRPreset DbId=\"{ptzr_id}\"><FieldsBlob/><Name/><ID>100</ID><IsCustom>true</IsCustom><Producer>Producer@Resolve</Producer><Presets>7b7d</Presets></SmPTZRPreset></PTZRPreset>\
                 <PTZRPresetType>0</PTZRPresetType>\
                 <SlateTC>-68719476737</SlateTC>\
                 <Sequence>\
                  <Sm2Sequence DbId=\"{new_seq_db_id}\">\
                   <FieldsBlob>{seq_blob_hex}</FieldsBlob>\
                   <UniqueSequenceId>{new_unique_seq_id}</UniqueSequenceId>\
                   <MediaExtents>{ext_hex}</MediaExtents>\
                   <FrameRate>{fr_hex}</FrameRate>\
                   <Resolution>{res_hex}</Resolution>\
                   <VideoTrackVec/><AudioTrackVec/>\
                   <Parent>{new_timeline_clip_id}</Parent>\
                   <LastChangedTime>0</LastChangedTime>\
                   <RenderCacheBA>00000024789c6366606420040000b20005</RenderCacheBA>\
                   <AuxRenderCacheBA>00000024789c6366606420040000b20005</AuxRenderCacheBA>\
                   <UIElementsState/>\
                   <NumOutputAudioChannels>{num_audio_ch}</NumOutputAudioChannels>\
                   <OutputAudioGain>0</OutputAudioGain>\
                   <PinsBA/><ImportExportMetadataBA/><LockSysId/><DbSavedTime>0</DbSavedTime>\
                  </Sm2Sequence>\
                 </Sequence>\
                 <VideoMetadata>{vm_hex}</VideoMetadata>\
                 <MediaMetadata/>\
                </{tag}>"
            )
        } else {
            let mut tl_block = mp_timeline_template.clone();
            tl_block = replace_attr_first(&tl_block, "Sm2MpTimelineClip", "DbId", &new_timeline_clip_id);
            tl_block = set_child_text(&tl_block, "UniqueMediaPoolItemId", &new_unique_mp_id);
            tl_block = replace_attr_first(&tl_block, "Sm2Timeline", "DbId", &new_timeline_id);
            tl_block = set_child_text(&tl_block, "MpTimelineClip", &new_timeline_clip_id);
            tl_block = replace_attr_first(&tl_block, "Sm2Sequence", "DbId", &new_seq_db_id);
            tl_block = set_child_text(&tl_block, "UniqueSequenceId", &new_unique_seq_id);
            tl_block = set_child_text(&tl_block, "Parent", &new_timeline_id);

            // Update sequence names
            let escaped_seq_name = xml_escape(&seq.name);
            let re_tl_name = Regex::new(r"(?s)(<Sm2Timeline\b[^>]*>.*?<Name>).*?(</Name>)").unwrap();
            tl_block = re_tl_name
                .replace(&tl_block, format!("${{1}}{escaped_seq_name}${{2}}"))
                .into_owned();
            let re_mp_name = Regex::new(r"(?s)(<Sm2MpTimelineClip\b[^>]*>.*?<Name>).*?(</Name>)").unwrap();
            tl_block = re_mp_name
                .replace(&tl_block, format!("${{1}}{escaped_seq_name}${{2}}"))
                .into_owned();

            // Update UTF-16LE hex container UUID inside Sm2Sequence FieldsBlob / SequenceContainer ref
            let old_hex = hex_encode(&encode_utf16le(&orig_container_id));
            let new_hex = hex_encode(&encode_utf16le(&new_container_id));
            tl_block = tl_block.replace(&old_hex, &new_hex);

            // Update FrameRate, Resolution, MediaExtents, Marks
            let mut fr_hex_bytes = Vec::with_capacity(16);
            fr_hex_bytes.extend_from_slice(&seq_fps.to_le_bytes());
            fr_hex_bytes.extend_from_slice(&[0u8; 8]);
            tl_block = set_child_text(&tl_block, "FrameRate", &hex_encode(&fr_hex_bytes));

            let mut res_bytes = Vec::with_capacity(16);
            res_bytes.extend_from_slice(&(seq.width as u64).to_be_bytes());
            res_bytes.extend_from_slice(&(seq.height as u64).to_be_bytes());
            tl_block = set_child_text(&tl_block, "Resolution", &hex_encode(&res_bytes));

            let start_sec = tc_offset as f64 / seq_fps.max(1.0);
            let dur_sec = seq.computed_duration_frames().max(0) as f64 / seq_fps.max(1.0);
            let mut ext_bytes = Vec::with_capacity(16);
            ext_bytes.extend_from_slice(&start_sec.to_le_bytes());
            ext_bytes.extend_from_slice(&dur_sec.to_le_bytes());
            tl_block = set_child_text(&tl_block, "MediaExtents", &hex_encode(&ext_bytes));

            let mark_in_str = seq.mark_in.map(|v| v.to_string()).unwrap_or_default();
            let mark_out_str = seq
                .mark_out
                .map(|v| (v - 1).max(0).to_string())
                .unwrap_or_default();
            for kind in ["Audio", "Video"] {
                tl_block = set_child_text(&tl_block, &format!("MarkIn{kind}"), &mark_in_str);
                tl_block = set_child_text(&tl_block, &format!("MarkOut{kind}"), &mark_out_str);
            }

            // Rebuild Fairlight audio model and inject custom SequenceSetup (when timeline res/fps differs from project default)
            let seq_fps_int = seq.frame_rate.timebase().max(1) as u32;
            let needs_custom_setup =
                seq.width != default_w || seq.height != default_h || seq_fps_int != default_fps_int;
            if !seq.audio_tracks.is_empty() || !seq.audio_buses.is_empty() || needs_custom_setup {
                let re_seq_blob = Regex::new(r"(?s)(<Sm2Sequence\b[^>]*>.*?<FieldsBlob>)([0-9a-fA-F]+)(</FieldsBlob>)").unwrap();
                if let Some(caps) = re_seq_blob.captures(&tl_block) {
                    let mut blob_bytes = hex_decode(&caps[2])?;
                    if !seq.audio_tracks.is_empty() || !seq.audio_buses.is_empty() {
                        // Order buses so main/root bus is index 0, followed by submixes
                        let mut ordered_buses: Vec<&nle_core::AudioBus> = Vec::new();
                        for b in &seq.audio_buses {
                            if b.is_main || b.parent_bus_id.is_none() {
                                ordered_buses.push(b);
                            }
                        }
                        for b in &seq.audio_buses {
                            if !(b.is_main || b.parent_bus_id.is_none()) {
                                ordered_buses.push(b);
                            }
                        }
                        let mut bus_idx_by_id: HashMap<&str, usize> = HashMap::new();
                        for (i, b) in ordered_buses.iter().enumerate() {
                            bus_idx_by_id.insert(b.id.as_str(), i);
                        }
                        let bus_specs: Vec<FairlightBusSpec> = ordered_buses
                            .iter()
                            .enumerate()
                            .map(|(i, b)| {
                                let parent_idx = b
                                    .parent_bus_id
                                    .as_deref()
                                    .and_then(|pid| bus_idx_by_id.get(pid).copied())
                                    .filter(|&p| p != i)
                                    .or_else(|| {
                                        if i > 0 && !b.is_main {
                                            Some(0)
                                        } else {
                                            None
                                        }
                                    });
                                FairlightBusSpec {
                                    name: b.name.clone(),
                                    is_main: b.is_main || parent_idx.is_none(),
                                    parent_bus_index: parent_idx,
                                }
                            })
                            .collect();

                        let specs: Vec<FairlightTrackSpec> = seq
                            .audio_tracks
                            .iter()
                            .map(|t| {
                                let bus_index = t
                                    .output_bus_id
                                    .as_deref()
                                    .and_then(|bid| bus_idx_by_id.get(bid).copied())
                                    .unwrap_or(0);
                                FairlightTrackSpec {
                                    is_mono: t.audio_channels < 2,
                                    name: t.name.clone(),
                                    enabled: t.enabled,
                                    locked: t.locked,
                                    bus_index,
                                }
                            })
                            .collect();
                        match rebuild_fairlight_track_model(&blob_bytes, &specs, &bus_specs) {
                            Ok(rebuilt) => blob_bytes = rebuilt,
                            Err(err) => {
                                messages.push(ConversionMessage {
                                    severity: Severity::Warning,
                                    code: "FAIRLIGHT_MODEL".into(),
                                    sequence: Some(seq.name.clone()),
                                    clip: None,
                                    message: format!("Could not rebuild Fairlight strip model: {err}"),
                                });
                            }
                        }
                    }
                    if needs_custom_setup {
                        if let Ok(mut kfields) = decode_keyed_fields(&blob_bytes) {
                            kfields.retain(|(k, _, _)| k != "SequenceSetup");
                            let setup_blob = build_sequence_setup_blob(seq.width, seq.height, seq_fps_int);
                            let insert_pos = usize::from(!kfields.is_empty());
                            kfields.insert(insert_pos, ("SequenceSetup".into(), 12, setup_blob));
                            blob_bytes = encode_keyed_fields(&kfields);
                        }
                    }
                    let m = caps.get(2).unwrap();
                    let mut updated = String::with_capacity(tl_block.len());
                    updated.push_str(&tl_block[..m.start()]);
                    updated.push_str(&hex_encode(&blob_bytes));
                    updated.push_str(&tl_block[m.end()..]);
                    tl_block = updated;
                }
            }

            timeline_handles.push(new_timeline_id);
            tl_block
        };

        // Sequence markers
        if options.include_markers && !seq.markers.is_empty() {
            lockable_blobs.push(sequence_lockable_blob_xml(&new_seq_db_id, &seq.markers));
            total_markers += seq.markers.len();
        }

        timeline_clip_blocks.push((
            seq.id.clone(),
            format!("   <Element>\n{tl_block}\n   </Element>"),
        ));
    }

    // Optionally include unplaced project bin media items (preserving their file & custom metadata)
    if options.include_unused_media {
        for asset in &project.media_assets {
            if asset.path.is_empty() && asset.name.is_empty() {
                continue;
            }
            let pkey = path_key(&asset.path, &asset.name);
            if media_pool_ids.contains_key(&pkey) {
                continue;
            }
            let is_video = asset.media_kind != MediaKind::Audio;
            ensure_media_pool_asset(
                &pkey,
                asset,
                is_video,
                &mp_video_tpl,
                &mp_still_tpl,
                &mp_audio_tpl,
                &mut media_pool_ids,
                &mut media_pool_entries,
                &mut lockable_blobs,
            )?;
        }
    }

    // Build Master MpFolder.xml + optional nested bins
    let mut bins_created = 0usize;
    let re_strip_items = Regex::new(
        r"(?s)<Element>\s*<(?:Sm2MpVideoClip|Sm2MpAudioClip|Sm2MpTimelineClip)\b.*?</(?:Sm2MpVideoClip|Sm2MpAudioClip|Sm2MpTimelineClip)>\s*</Element>",
    )
    .unwrap();
    let mut base_pool = re_strip_items.replace_all(&orig_pool_xml, "").into_owned();

    let mut master_items: Vec<String> = Vec::new();
    if options.include_bins && !project.bins.is_empty() {
        let master_id = extract_attr(&base_pool, "Sm2MpFolder", "DbId")
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let media_pool_id = extract_tag_inner(&base_pool, "MediaPool")
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let folder_blob = "0000000200000027800a061090d395a40d1890d395a40d2090d395a40d5a100a0c3030303030302030303030301200";

        let folder_db_ids: HashMap<&str, String> = project
            .bins
            .iter()
            .map(|b| (b.id.as_str(), Uuid::new_v4().to_string()))
            .collect();
        let bins_by_id: HashMap<&str, &nle_core::ProjectBin> =
            project.bins.iter().map(|b| (b.id.as_str(), b)).collect();

        // Map each parent Sm2MpFolder DbId -> direct child folder DbIds (required in parent FieldsBlob field 2 for Resolve 21!)
        let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
        for b in &project.bins {
            let folder_id = folder_db_ids[b.id.as_str()].clone();
            let parent_folder_id = b
                .parent_id
                .as_deref()
                .and_then(|pid| folder_db_ids.get(pid))
                .cloned()
                .unwrap_or_else(|| master_id.clone());
            children_by_parent
                .entry(parent_folder_id)
                .or_default()
                .push(folder_id);
        }

        if let Some(master_children) = children_by_parent.get(&master_id) {
            if let Some(orig_master_blob) = extract_tag_inner(&base_pool, "FieldsBlob") {
                let updated_master_blob =
                    inject_child_folders_into_blob(&orig_master_blob, master_children);
                base_pool = set_child_text(&base_pool, "FieldsBlob", &updated_master_blob);
            }
        }

        let mut media_to_bin: HashMap<String, String> = HashMap::new();
        let mut seq_to_bin: HashMap<String, String> = HashMap::new();
        for b in &project.bins {
            for p in &b.media_paths {
                let k = path_key(p, "");
                media_to_bin.entry(k).or_insert_with(|| b.id.clone());
            }
            for sid in &b.sequence_ids {
                seq_to_bin.entry(sid.clone()).or_insert_with(|| b.id.clone());
            }
        }

        let mut bin_items: HashMap<&str, Vec<String>> = project
            .bins
            .iter()
            .map(|b| (b.id.as_str(), Vec::new()))
            .collect();

        for (pkey, block) in &media_pool_entries {
            if let Some(bin_id) = media_to_bin.get(pkey) {
                let target_folder_id = &folder_db_ids[bin_id.as_str()];
                let moved = set_child_text(block, "MpFolder", target_folder_id);
                bin_items.get_mut(bin_id.as_str()).unwrap().push(moved);
            } else {
                master_items.push(block.clone());
            }
        }
        for (seq_id, block) in &timeline_clip_blocks {
            if let Some(bin_id) = seq_to_bin.get(seq_id) {
                let target_folder_id = &folder_db_ids[bin_id.as_str()];
                let moved = set_child_text(block, "MpFolder", target_folder_id);
                bin_items.get_mut(bin_id.as_str()).unwrap().push(moved);
            } else {
                master_items.push(block.clone());
            }
        }

        let mut folder_paths: HashMap<String, String> = HashMap::new();
        for b in &project.bins {
            let fpath = resolve_bin_zip_path(&b.id, &bins_by_id, &mut folder_paths);
            let folder_id = &folder_db_ids[b.id.as_str()];
            let parent_folder_id = b
                .parent_id
                .as_deref()
                .and_then(|pid| folder_db_ids.get(pid))
                .map(|s| s.as_str())
                .unwrap_or(&master_id);
            let moved_xml = bin_items
                .get(b.id.as_str())
                .map(|v| v.join("\n"))
                .unwrap_or_default();
            let this_blob = children_by_parent
                .get(folder_id)
                .map(|kids| inject_child_folders_into_blob(folder_blob, kids))
                .unwrap_or_else(|| folder_blob.to_string());

            let bin_xml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!--DbAppVer=\"21.0.2.0004\" DbPrjVer=\"17\"-->\n<Sm2MpFolder DbId=\"{folder_id}\"><FieldsBlob>{this_blob}</FieldsBlob><Name>{}</Name><MpFolder>{parent_folder_id}</MpFolder><UniqueMediaPoolItemId>{}</UniqueMediaPoolItemId><MediaVec>{moved_xml}</MediaVec><MediaPool>{media_pool_id}</MediaPool><Folded>true</Folded><ColorTag>FOLDER_COLOR_NONE</ColorTag><LockSysId/><DbSavedTime>0</DbSavedTime></Sm2MpFolder>\n",
                xml_escape(&b.name),
                Uuid::new_v4(),
            );
            members.insert(format!("{fpath}/MpFolder.xml"), bin_xml.into_bytes());
            bins_created += 1;
        }
    } else {
        for (_, block) in &media_pool_entries {
            master_items.push(block.clone());
        }
        for (_, block) in &timeline_clip_blocks {
            master_items.push(block.clone());
        }
    }

    let final_pool = set_tag_inner(&base_pool, "MediaVec", &format!("\n{}\n", master_items.join("\n")));
    members.insert(pool_key, final_pool.into_bytes());

    // Configure project.xml (color science, SetupBA resolution/fps, TimelineHandleVec, lockable blobs, ProjectName)
    configure_project_color_blob(&mut members, &project.color_config)?;
    let mut project_xml = String::from_utf8(members.get("project.xml").cloned().unwrap())
        .map_err(|e| NleError::InvalidFormat(e.to_string()))?;

    let handles_inner: String = timeline_handles
        .iter()
        .map(|h| format!("<Element>{h}</Element>"))
        .collect();
    project_xml = set_tag_inner(&project_xml, "TimelineHandleVec", &handles_inner);

    for blob in &lockable_blobs {
        project_xml = append_lockable_blob(&project_xml, blob)?;
    }

    project_xml = patch_project_setup_defaults(
        &project_xml,
        default_w,
        default_h,
        default_fps_int,
    )?;

    let out_name = options
        .project_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&project.name);
    let escaped_proj_name = xml_escape(out_name);
    project_xml = project_xml.replace("_mcp_pr2resolve_codec_template_20260915", &escaped_proj_name);
    let re_proj_name = Regex::new(r"<ProjectName>.*?</ProjectName>").unwrap();
    project_xml = re_proj_name
        .replace(&project_xml, format!("<ProjectName>{escaped_proj_name}</ProjectName>"))
        .into_owned();
    members.insert("project.xml".into(), project_xml.into_bytes());

    // Write final .drp ZIP archive
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let file = File::create(output_path)?;
    let mut zip = ZipWriter::new(file);
    let zip_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in &members {
        zip.start_file(name, zip_opts)
            .map_err(|e| NleError::Archive(e.to_string()))?;
        zip.write_all(data)?;
    }
    zip.finish().map_err(|e| NleError::Archive(e.to_string()))?;

    Ok(ConversionReport {
        output_path: output_path.to_string_lossy().into_owned(),
        source_format: project.source_format,
        target_format: NleFormat::DaVinciResolve,
        sequences_converted: selected_sequences.len(),
        video_clips_converted: total_video_clips,
        audio_clips_converted: total_audio_clips,
        media_assets_linked: media_pool_entries.len(),
        bins_created,
        markers_written: total_markers,
        grades_attached: total_grades,
        effect_markers_injected: 0,
        messages,
    })
}

fn load_template_members() -> NleResult<BTreeMap<String, Vec<u8>>> {
    let cursor = Cursor::new(RESOLVE_TEMPLATE_BYTES);
    let mut archive = ZipArchive::new(cursor).map_err(|e| NleError::Archive(e.to_string()))?;
    let mut map = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| NleError::Archive(e.to_string()))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        map.insert(entry.name().to_string(), buf);
    }
    Ok(map)
}

#[allow(clippy::too_many_arguments)]
fn ensure_media_pool_item(
    pkey: &str,
    clip: &Clip,
    asset: Option<&nle_core::MediaAsset>,
    is_video: bool,
    mp_video_tpl: &str,
    mp_still_tpl: &str,
    mp_audio_tpl: &str,
    media_pool_ids: &mut HashMap<String, String>,
    media_pool_entries: &mut Vec<(String, String)>,
    lockable_blobs: &mut Vec<String>,
) -> NleResult<String> {
    if let Some(existing_id) = media_pool_ids.get(pkey) {
        return Ok(existing_id.clone());
    }
    let item_id = Uuid::new_v4().to_string();
    media_pool_ids.insert(pkey.to_string(), item_id.clone());

    let tpl = if is_video {
        if clip.media_kind == MediaKind::Image {
            mp_still_tpl
        } else {
            mp_video_tpl
        }
    } else {
        mp_audio_tpl
    };

    let full_path = if !clip.media_path.is_empty() {
        clip.media_path.clone()
    } else {
        format!("/{}", clip.name)
    };
    let display_name = Path::new(&full_path.replace('\\', "/"))
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| clip.name.clone());

    let duration = asset
        .map(|a| a.duration_frames)
        .unwrap_or(0)
        .max(clip.source_out_frame)
        .max(clip.duration_frames())
        .max(1);
    let fps = asset
        .map(|a| a.frame_rate.as_f64())
        .unwrap_or_else(|| clip.source_fps.as_f64());
    let width = asset.map(|a| a.width).unwrap_or(clip.source_width);
    let height = asset.map(|a| a.height).unwrap_or(clip.source_height);
    let source_start = asset.map(|a| a.source_tc_frame).unwrap_or(0);
    let sample_rate = asset.map(|a| a.audio_sample_rate).unwrap_or(48_000);

    let mut node = replace_root_db_id(tpl, &item_id);
    node = set_child_text(&node, "UniqueMediaPoolItemId", &Uuid::new_v4().to_string());
    node = set_child_text(&node, "Name", &xml_escape(&display_name));
    let (rewritten, primary_info_id) = rewrite_media_pool_av_blobs(
        &node,
        &full_path,
        &display_name,
        duration,
        fps,
        width,
        height,
        source_start,
        sample_rate,
        !is_video,
    )?;
    node = rewritten;

    if let (Some(a), Some(info_id)) = (asset, primary_info_id.as_deref()) {
        if let Some(blob) = media_metadata_lockable_blob_xml(info_id, &a.metadata, &a.custom_metadata)
        {
            lockable_blobs.push(blob);
        }
    }

    media_pool_entries.push((
        pkey.to_string(),
        format!("   <Element>\n{node}\n   </Element>"),
    ));
    Ok(item_id)
}

#[allow(clippy::too_many_arguments)]
fn ensure_media_pool_asset(
    pkey: &str,
    asset: &nle_core::MediaAsset,
    is_video: bool,
    mp_video_tpl: &str,
    mp_still_tpl: &str,
    mp_audio_tpl: &str,
    media_pool_ids: &mut HashMap<String, String>,
    media_pool_entries: &mut Vec<(String, String)>,
    lockable_blobs: &mut Vec<String>,
) -> NleResult<String> {
    if let Some(existing_id) = media_pool_ids.get(pkey) {
        return Ok(existing_id.clone());
    }
    let item_id = Uuid::new_v4().to_string();
    media_pool_ids.insert(pkey.to_string(), item_id.clone());

    let tpl = if is_video {
        if asset.media_kind == MediaKind::Image {
            mp_still_tpl
        } else {
            mp_video_tpl
        }
    } else {
        mp_audio_tpl
    };

    let full_path = if !asset.path.is_empty() {
        asset.path.clone()
    } else {
        format!("/{}", asset.name)
    };
    let display_name = Path::new(&full_path.replace('\\', "/"))
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| asset.name.clone());

    let duration = asset.duration_frames.max(25);
    let fps = asset.frame_rate.as_f64();
    let width = asset.width;
    let height = asset.height;
    let source_start = asset.source_tc_frame;
    let sample_rate = asset.audio_sample_rate;

    let mut node = replace_root_db_id(tpl, &item_id);
    node = set_child_text(&node, "UniqueMediaPoolItemId", &Uuid::new_v4().to_string());
    node = set_child_text(&node, "Name", &xml_escape(&display_name));
    let (rewritten, primary_info_id) = rewrite_media_pool_av_blobs(
        &node,
        &full_path,
        &display_name,
        duration,
        fps,
        width,
        height,
        source_start,
        sample_rate,
        !is_video,
    )?;
    node = rewritten;

    if let Some(info_id) = primary_info_id.as_deref() {
        if let Some(blob) =
            media_metadata_lockable_blob_xml(info_id, &asset.metadata, &asset.custom_metadata)
        {
            lockable_blobs.push(blob);
        }
    }

    media_pool_entries.push((
        pkey.to_string(),
        format!("   <Element>\n{node}\n   </Element>"),
    ));
    Ok(item_id)
}

#[allow(clippy::too_many_arguments)]
fn rewrite_media_pool_av_blobs(
    xml: &str,
    full_path: &str,
    file_name: &str,
    duration: i64,
    fps: f64,
    width: u32,
    height: u32,
    source_start: i64,
    sample_rate: u32,
    is_standalone_audio: bool,
) -> NleResult<(String, Option<String>)> {
    let mut out = xml.to_string();
    let mut primary_info_id: Option<String> = None;
    let norm = full_path.replace('\\', "/");
    let parent_dir = Path::new(&norm)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/".to_string());
    let video_uuid = Uuid::new_v4().to_string();

    for info_tag in ["BtVideoInfo", "BtAudioInfo"] {
        let re_info = Regex::new(&format!(r"(?s)<{info_tag}\b[^>]*>.*?</{info_tag}>")).unwrap();
        let blocks: Vec<String> = re_info
            .find_iter(&out)
            .map(|m| m.as_str().to_string())
            .collect();

        for orig_block in blocks {
            let mut block = orig_block.clone();
            let old_info_id = extract_attr(&block, info_tag, "DbId").unwrap_or_default();
            let new_info_id = Uuid::new_v4().to_string();
            if primary_info_id.is_none()
                && ((info_tag == "BtVideoInfo" && !is_standalone_audio)
                    || (info_tag == "BtAudioInfo" && is_standalone_audio))
            {
                primary_info_id = Some(new_info_id.clone());
            }
            block = replace_attr_first(&block, info_tag, "DbId", &new_info_id);

            if let Some(clip_hex) = extract_tag_inner(&block, "Clip") {
                if let Ok(raw) = decode_wrapper_hex(&clip_hex) {
                    if let Ok(mut fields) = decode_protobuf(&raw) {
                        for f in &mut fields {
                            if let ProtoValue::Bytes(ref mut bytes) = f.value {
                                match f.number {
                                    1 => *bytes = parent_dir.as_bytes().to_vec(),
                                    2 => *bytes = file_name.as_bytes().to_vec(),
                                    6 if info_tag == "BtVideoInfo" => {
                                        *bytes = file_name.as_bytes().to_vec()
                                    }
                                    7 if info_tag == "BtVideoInfo" => {
                                        *bytes = video_uuid.as_bytes().to_vec()
                                    }
                                    _ => {}
                                }
                            }
                        }
                        let new_hex = encode_wrapper_hex(&encode_protobuf(&fields));
                        block = set_child_text(&block, "Clip", &new_hex);
                    }
                }
            }

            if info_tag == "BtVideoInfo" {
                if let Some(time_hex) = extract_tag_inner(&block, "Time") {
                    if let Ok(mut raw) = hex_decode(&time_hex) {
                        patch_utf16_keyed_bytes(
                            &mut raw,
                            "StartFrame",
                            2,
                            &(source_start as i32).to_be_bytes(),
                        );
                        if duration > 0 {
                            patch_utf16_keyed_bytes(
                                &mut raw,
                                "NumFrames",
                                2,
                                &(duration as i32).to_be_bytes(),
                            );
                        }
                        if fps > 0.0 {
                            let mut fr = Vec::with_capacity(20);
                            fr.extend_from_slice(&16u32.to_be_bytes());
                            fr.extend_from_slice(&fps.to_le_bytes());
                            fr.extend_from_slice(&[0u8; 8]);
                            patch_utf16_keyed_bytes(&mut raw, "FrameRate", 12, &fr);
                        }
                        block = set_child_text(&block, "Time", &hex_encode(&raw));
                    }
                }
                if width > 0 && height > 0 {
                    if let Some(geo_hex) = extract_tag_inner(&block, "Geometry") {
                        if let Ok(mut raw) = hex_decode(&geo_hex) {
                            let mut res = Vec::with_capacity(20);
                            res.extend_from_slice(&16u32.to_be_bytes());
                            res.extend_from_slice(&(width as u64).to_be_bytes());
                            res.extend_from_slice(&(height as u64).to_be_bytes());
                            patch_utf16_keyed_bytes(&mut raw, "Resolution", 12, &res);
                            block = set_child_text(&block, "Geometry", &hex_encode(&raw));
                        }
                    }
                }
            } else if info_tag == "BtAudioInfo" && duration > 0 && fps > 0.0 {
                if let Some(tracks_hex) = extract_tag_inner(&block, "TracksBA") {
                    if let Ok(raw) = hex_decode(&tracks_hex) {
                        if let Ok(mut phys) = decode_keyed_fields(&raw) {
                            for (_, _, track_raw) in &mut phys {
                                if let Ok(mut tfields) = decode_keyed_fields(track_raw) {
                                    let rate = if sample_rate > 0 { sample_rate } else { 48_000 };
                                    let sample_dur = ((duration as f64 / fps) * rate as f64).round() as i64;
                                    for (k, _, v) in &mut tfields {
                                        if k == "SampleRate" {
                                            *v = rate.to_be_bytes().to_vec();
                                        } else if k == "Duration" {
                                            *v = sample_dur.to_be_bytes().to_vec();
                                        } else if k == "NumChannels" && is_standalone_audio {
                                            *v = 2u32.to_be_bytes().to_vec();
                                        }
                                    }
                                    *track_raw = encode_keyed_fields(&tfields);
                                }
                            }
                            block = set_child_text(&block, "TracksBA", &hex_encode(&encode_keyed_fields(&phys)));
                        }
                    }
                }
            }

            out = out.replacen(&orig_block, &block, 1);

            // Rewrite UTF-16LE MediaRef DbId inside the root clip's <FieldsBlob>
            if !old_info_id.is_empty() {
                let re_root_blob = Regex::new(r"(?s)(<Sm2Mp(?:Video|Audio)Clip\b[^>]*>\s*<FieldsBlob>)([0-9a-fA-F]+)(</FieldsBlob>)").unwrap();
                if let Some(caps) = re_root_blob.captures(&out) {
                    if let Ok(raw) = decode_wrapper_hex(&caps[2]) {
                        let old_u16 = encode_utf16le(&old_info_id);
                        let new_u16 = encode_utf16le(&new_info_id);
                        let updated_raw = replace_subslice(&raw, &old_u16, &new_u16);
                        let m = caps.get(2).unwrap();
                        let mut next = String::with_capacity(out.len());
                        next.push_str(&out[..m.start()]);
                        next.push_str(&encode_wrapper_hex(&updated_raw));
                        next.push_str(&out[m.end()..]);
                        out = next;
                    }
                }
            }
        }
    }

    Ok((out, primary_info_id))
}

fn patch_utf16_keyed_bytes(raw: &mut [u8], key: &str, value_type: u32, new_val: &[u8]) {
    let mut marker = encode_utf16be(key);
    marker.extend_from_slice(&value_type.to_be_bytes());
    marker.push(0);
    if let Some(pos) = raw.windows(marker.len()).position(|w| w == marker.as_slice()) {
        let start = pos + marker.len();
        if start + new_val.len() <= raw.len() {
            raw[start..start + new_val.len()].copy_from_slice(new_val);
        }
    }
}

/// Build Resolve 21.2 clip `<FieldsBlob>` encoding linked sibling DbIds (`f1.f1.f1`)
/// and/or active multicam angle name (`f1.f3`, `f1.f4 = 1`).
fn build_clip_fields_blob(sibling_ids: &[String], active_angle: Option<&str>) -> Option<String> {
    let angle = active_angle.map(str::trim).filter(|s| !s.is_empty());
    if sibling_ids.is_empty() && angle.is_none() {
        return None;
    }
    let mut f1_fields: Vec<ProtoField> = Vec::new();
    if !sibling_ids.is_empty() {
        let entries: Vec<(String, u32, Vec<u8>)> = sibling_ids
            .iter()
            .enumerate()
            .map(|(idx, id)| (idx.to_string(), 10u32, encode_utf16be(id)))
            .collect();
        let dict_bytes = encode_keyed_fields(&entries);
        let f11 = encode_protobuf(&[ProtoField::bytes(1, dict_bytes)]);
        f1_fields.push(ProtoField::bytes(1, f11));
    }
    if let Some(a) = angle {
        f1_fields.push(ProtoField::bytes(3, a.as_bytes().to_vec()));
        f1_fields.push(ProtoField::varint(4, 1));
    }
    let outer = encode_protobuf(&[ProtoField::bytes(1, encode_protobuf(&f1_fields))]);
    Some(encode_wrapper_hex(&outer))
}

fn build_static_video_effects_payload(
    motion: &MotionTransform,
    opacity: f64,
    source_geom: (u32, u32),
    timeline_geom: (u32, u32),
) -> Option<String> {
    let has_motion = !motion.is_identity();
    let has_opacity = opacity.is_finite() && (opacity - 100.0).abs() > 1e-3;
    if !has_motion && !has_opacity {
        return None;
    }

    let mut outer_fields: Vec<ProtoField> = Vec::new();

    if has_motion {
        let (sw, sh) = source_geom;
        let (tw, th) = timeline_geom;
        let fit = if sw > 0 && sh > 0 && tw > 0 && th > 0 {
            (tw as f64 / sw as f64).min(th as f64 / sh as f64)
        } else {
            1.0
        };
        let zoom = (motion.scale / 100.0) / fit.max(1e-6);
        let zoom_y = motion
            .scale_y
            .map(|sy| (sy / 100.0) / fit.max(1e-6))
            .unwrap_or(zoom);

        let mut params: HashMap<usize, (u64, f64)> = HashMap::new();
        params.insert(0, (42, zoom));
        params.insert(1, (43, zoom_y));
        params.insert(2, (40, motion.position_x));
        params.insert(3, (41, motion.position_y));
        params.insert(5, (47, -motion.rotation));

        let mut fields = vec![ProtoField::varint(1, 4), ProtoField::varint(7, 20)];
        for slot in 0..12 {
            if let Some(&(pid, val)) = params.get(&slot) {
                let kf = encode_protobuf(&[ProtoField::f64_le(2, val)]);
                let curve = encode_protobuf(&[ProtoField::bytes(1, kf)]);
                let param =
                    encode_protobuf(&[ProtoField::varint(1, pid), ProtoField::bytes(3, curve)]);
                fields.push(ProtoField::bytes(9, param));
            } else {
                fields.push(ProtoField::bytes(9, Vec::new()));
            }
        }
        outer_fields.push(ProtoField::bytes(1, encode_protobuf(&fields)));
    }

    if has_opacity {
        // Resolve Filter 2 (Composite): slot 0 = CompositeMode (empty/default), slot 1 = param 1 Opacity (f64le)
        let kf = encode_protobuf(&[ProtoField::f64_le(2, opacity.clamp(0.0, 100.0))]);
        let curve = encode_protobuf(&[ProtoField::bytes(1, kf)]);
        let param = encode_protobuf(&[ProtoField::varint(1, 1), ProtoField::bytes(3, curve)]);
        let comp_fields = vec![
            ProtoField::varint(1, 2),
            ProtoField::bytes(9, Vec::new()),
            ProtoField::bytes(9, param),
        ];
        outer_fields.push(ProtoField::bytes(1, encode_protobuf(&comp_fields)));
    }

    let outer = encode_protobuf(&outer_fields);
    Some(encode_wrapper_hex(&outer))
}

fn build_resolve_transition_xml(
    tr: &nle_core::Transition,
    tc_offset: i64,
    is_audio: bool,
) -> Option<String> {
    let dur = tr.end_frame - tr.start_frame;
    if dur <= 0 {
        return None;
    }
    let id_lower = tr.effect_id.to_ascii_lowercase();
    let nm_lower = tr.name.to_ascii_lowercase();

    let pretty_type = if is_audio {
        if id_lower.contains("constant power") || nm_lower.contains("constant power") {
            "Cross Fade +3DB"
        } else if id_lower.contains("constant gain")
            || nm_lower.contains("constant gain")
            || id_lower.contains("exponential")
            || nm_lower.contains("exponential")
            || id_lower.contains("custom fade")
            || nm_lower.contains("custom fade")
            || id_lower.contains("cross fade")
            || nm_lower.contains("cross fade")
        {
            "Cross Fade 0DB"
        } else {
            return None;
        }
    } else if id_lower.contains("cross dissolve")
        || nm_lower.contains("cross dissolve")
        || id_lower.contains("film dissolve")
        || nm_lower.contains("film dissolve")
        || id_lower.contains("additive dissolve")
        || nm_lower.contains("additive dissolve")
        || id_lower.contains("dip to black")
        || nm_lower.contains("dip to black")
        || id_lower.contains("dip to white")
        || nm_lower.contains("dip to white")
    {
        "Cross Dissolve"
    } else {
        return None;
    };

    let align_code = match tr.alignment.as_str() {
        "start" => 1,
        "end" => 3,
        _ => 2,
    };
    let abs_start = tc_offset + tr.start_frame.max(0);
    let db_id = Uuid::new_v4();

    Some(format!(
        "<Sm2TiTransition DbId=\"{db_id}\"><FieldsBlob/><PrettyType>{pretty_type}</PrettyType><Name>Transition</Name><Start>{abs_start}</Start><Duration>{dur}</Duration><LinkedItemSync/><WasDisbanded>false</WasDisbanded><MarkersBA/><UiMemento>0</UiMemento><Flags>0</Flags><PriorityIndex>0</PriorityIndex><EffectFiltersBA/><ImportExportMetadataBA/><RenderTextEnabled>true</RenderTextEnabled><RenderTextGanged>true</RenderTextGanged><RenderTextPrefixed>true</RenderTextPrefixed><AlignmentType>{align_code}</AlignmentType><Position>{align_code}</Position></Sm2TiTransition>"
    ))
}

fn build_track_xml(
    track_tpl: &str,
    track: &nle_core::Track,
    items_xml: &str,
    is_audio: bool,
) -> String {
    let mut node = replace_root_db_id(track_tpl, &Uuid::new_v4().to_string());
    node = set_tag_inner(&node, "Items", items_xml);
    node = set_child_text(&node, "FusionCompHolderItems", "");
    node = set_child_text(&node, "LayersVec", "");
    if is_audio {
        let sub_type = if track.audio_channels >= 2 { "0" } else { "1" };
        node = set_child_text(&node, "SubType", sub_type);
    }
    node = set_child_text(&node, "UserDefinedName", &xml_escape(&track.name));
    let base_flags: u32 = extract_tag_inner(&node, "Flags")
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let flags = (base_flags & !3)
        | if track.locked { 1 } else { 0 }
        | if !track.enabled || track.muted { 2 } else { 0 };
    node = set_child_text(&node, "Flags", &flags.to_string());
    node
}

/// Inject Resolve 21's child subfolder registry (protobuf field 2 keyed-dict `"0" -> childDbId`, ...)
/// into a parent `Sm2MpFolder`'s `<FieldsBlob>` so DaVinci Resolve discovers all child bins on import.
fn inject_child_folders_into_blob(blob_hex: &str, child_folder_ids: &[String]) -> String {
    if child_folder_ids.is_empty() {
        return blob_hex.to_string();
    }
    let Ok(raw) = decode_wrapper_hex(blob_hex) else {
        return blob_hex.to_string();
    };
    let Ok(mut fields) = decode_protobuf(&raw) else {
        return blob_hex.to_string();
    };
    fields.retain(|f| f.number != 2);
    let entries: Vec<(String, u32, Vec<u8>)> = child_folder_ids
        .iter()
        .enumerate()
        .map(|(i, cid)| (i.to_string(), 10, encode_utf16be(cid)))
        .collect();
    fields.insert(0, ProtoField::bytes(2, encode_keyed_fields(&entries)));
    encode_wrapper_hex(&encode_protobuf(&fields))
}

fn resolve_bin_zip_path(
    bin_id: &str,
    bins_by_id: &HashMap<&str, &nle_core::ProjectBin>,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(p) = cache.get(bin_id) {
        return p.clone();
    }
    let Some(bin) = bins_by_id.get(bin_id) else {
        return "MediaPool/Master".to_string();
    };
    let parent_path = match bin.parent_id.as_deref() {
        Some(pid) if pid != bin_id && bins_by_id.contains_key(pid) => {
            resolve_bin_zip_path(pid, bins_by_id, cache)
        }
        _ => "MediaPool/Master".to_string(),
    };
    let safe_name: String = bin
        .name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let idx = cache.len();
    let path = format!("{parent_path}/{idx:03}_{safe_name}");
    cache.insert(bin_id.to_string(), path.clone());
    path
}

fn path_key(path: &str, fallback_name: &str) -> String {
    let raw = if path.is_empty() { fallback_name } else { path };
    raw.replace('\\', "/").to_ascii_lowercase()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn extract_elements_by_tag(xml: &str, tag: &str) -> Vec<String> {
    let re = Regex::new(&format!(r"(?s)<{tag}\b[^>]*>.*?</{tag}>")).unwrap();
    re.find_iter(xml).map(|m| m.as_str().to_string()).collect()
}

fn extract_tag_inner(xml: &str, tag: &str) -> Option<String> {
    let re = Regex::new(&format!(r"(?s)<{tag}\b[^>]*>(.*?)</{tag}>")).unwrap();
    re.captures(xml).map(|c| c[1].to_string())
}

fn extract_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"<{tag}\b[^>]*\b{attr}="([^"]+)""#)).unwrap();
    re.captures(xml).map(|c| c[1].to_string())
}

fn replace_root_db_id(xml: &str, new_id: &str) -> String {
    let re = Regex::new(r#"(?s)(<[A-Za-z0-9_:]+\b[^>]*\bDbId=")[^"]+(")"#).unwrap();
    re.replace(xml, format!("${{1}}{new_id}${{2}}")).into_owned()
}

fn replace_attr_first(xml: &str, tag: &str, attr: &str, new_val: &str) -> String {
    let re = Regex::new(&format!(r#"(<{tag}\b[^>]*\b{attr}=")[^"]+(")"#)).unwrap();
    re.replace(xml, format!("${{1}}{new_val}${{2}}")).into_owned()
}

fn set_child_text(xml: &str, tag: &str, value: &str) -> String {
    let re_pair = Regex::new(&format!(r"(?s)<{tag}\b[^>]*>.*?</{tag}>")).unwrap();
    if re_pair.is_match(xml) {
        return re_pair
            .replace(xml, format!("<{tag}>{value}</{tag}>"))
            .into_owned();
    }
    let re_self = Regex::new(&format!(r"<{tag}\b[^>]*/\s*>")).unwrap();
    if re_self.is_match(xml) {
        return re_self
            .replace(xml, format!("<{tag}>{value}</{tag}>"))
            .into_owned();
    }
    xml.to_string()
}

fn set_tag_inner(xml: &str, tag: &str, inner: &str) -> String {
    let re = Regex::new(&format!(r"(?s)(<{tag}\b[^>]*>).*?(</{tag}>)")).unwrap();
    if re.is_match(xml) {
        return re
            .replace(xml, format!("${{1}}{inner}${{2}}"))
            .into_owned();
    }
    let re_self = Regex::new(&format!(r"<{tag}\b[^>]*/\s*>")).unwrap();
    if re_self.is_match(xml) {
        return re_self
            .replace(xml, format!("<{tag}>{inner}</{tag}>"))
            .into_owned();
    }
    xml.to_string()
}

fn replace_or_insert_block(xml: &str, tag: &str, replacement: &str, closing_parent: &str) -> String {
    let re_pair = Regex::new(&format!(r"(?s)<{tag}\b[^>]*>.*?</{tag}>")).unwrap();
    if re_pair.is_match(xml) {
        return re_pair.replace(xml, replacement).into_owned();
    }
    let re_self = Regex::new(&format!(r"<{tag}\b[^>]*/\s*>")).unwrap();
    if re_self.is_match(xml) {
        return re_self.replace(xml, replacement).into_owned();
    }
    if let Some(pos) = xml.rfind(closing_parent) {
        let mut out = String::with_capacity(xml.len() + replacement.len());
        out.push_str(&xml[..pos]);
        out.push_str(replacement);
        out.push_str(&xml[pos..]);
        out
    } else {
        xml.to_string()
    }
}
