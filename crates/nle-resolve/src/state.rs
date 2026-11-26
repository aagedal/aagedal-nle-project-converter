use crate::binary::{
    decode_keyed_fields, decode_protobuf, decode_wrapper_bytes, encode_protobuf, encode_utf16be,
    hex_decode, hex_encode, read_varint, write_varint, ProtoField, ProtoValue,
};
use nle_core::{ColorConfig, Marker, MarkerScope, NleError, NleResult};
use regex::Regex;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Premiere Pro label preset index (0..=15) -> Resolve clip color enum.
pub fn premiere_label_to_resolve_color(preset_index: u8) -> Option<u32> {
    match preset_index {
        0 => Some(15),
        1 => Some(13),
        2 => Some(11),
        3 => Some(5),
        4 => Some(12),
        5 => Some(9),
        6 => Some(8),
        7 => Some(10),
        8 => Some(4),
        9 => Some(3),
        10 => Some(2),
        11 => Some(14),
        12 => Some(6),
        13 => Some(1),
        14 => Some(7),
        15 => Some(16),
        _ => None,
    }
}

/// Resolve clip color enum (1..=16) -> Premiere Pro label preset index (0..=15).
pub fn resolve_color_to_premiere_label(resolve_color: u32) -> Option<u8> {
    match resolve_color {
        1 => Some(13),
        2 => Some(10),
        3 => Some(9),
        4 => Some(8),
        5 => Some(3),
        6 => Some(12),
        7 => Some(14),
        8 => Some(6),
        9 => Some(5),
        10 => Some(7),
        11 => Some(2),
        12 => Some(4),
        13 => Some(1),
        14 => Some(11),
        15 => Some(0),
        16 => Some(15),
        _ => None,
    }
}

fn marker_color_bits(color: &str) -> u64 {
    match color.to_ascii_lowercase().as_str() {
        "cyan" => 4,
        "green" => 8,
        "yellow" => 16,
        "red" => 32,
        "pink" => 64,
        "purple" => 128,
        "fuchsia" => 512,
        "rose" => 1024,
        "lavender" => 2048,
        "sky" => 4096,
        "mint" => 8192,
        "lemon" => 16384,
        "sand" => 32768,
        "cocoa" => 65536,
        "cream" => 131072,
        _ => 2, // Blue default
    }
}

fn marker_color_from_bits(bits: u64) -> String {
    match bits {
        4 => "Cyan",
        8 => "Green",
        16 => "Yellow",
        32 => "Red",
        64 => "Pink",
        128 => "Purple",
        512 => "Fuchsia",
        1024 => "Rose",
        2048 => "Lavender",
        4096 => "Sky",
        8192 => "Mint",
        16384 => "Lemon",
        32768 => "Sand",
        65536 => "Cocoa",
        131072 => "Cream",
        _ => "Blue",
    }
    .to_string()
}

/// Decode Resolve `Sm2SequenceLockableBlob` / `Sm2TiItemLockableBlob` `FieldsBlob` hex
/// (supporting both `0x80` raw and `0x81` zstd) into `(markers, clip_color_enum)`.
pub fn decode_lockable_blob(fields_blob_hex: &str, scope: MarkerScope) -> (Vec<Marker>, Option<u32>) {
    let Ok(raw) = hex_decode(fields_blob_hex) else {
        return (Vec::new(), None);
    };
    let Ok(entries) = decode_keyed_fields(&raw) else {
        return (Vec::new(), None);
    };
    let Some(blob_data) = entries
        .into_iter()
        .find(|(k, _, _)| k == "BlobData")
        .map(|(_, _, v)| v)
    else {
        return (Vec::new(), None);
    };
    let Ok(proto_bytes) = decode_wrapper_bytes(&blob_data) else {
        return (Vec::new(), None);
    };
    let Ok(top_fields) = decode_protobuf(&proto_bytes) else {
        return (Vec::new(), None);
    };

    let mut markers = Vec::new();
    let mut clip_color: Option<u32> = None;

    for f in top_fields {
        match (f.number, f.value) {
            (2, ProtoValue::Bytes(container_bytes)) => {
                let Ok(c_fields) = decode_protobuf(&container_bytes) else {
                    continue;
                };
                for cf in c_fields {
                    if cf.number != 2 {
                        continue;
                    }
                    let ProtoValue::Bytes(entry_bytes) = cf.value else {
                        continue;
                    };
                    if let Some(m) = decode_marker_entry(&entry_bytes, scope) {
                        markers.push(m);
                    }
                }
            }
            (5, ProtoValue::Varint(v)) => {
                let c = (v / 2) as u32;
                if c > 0 {
                    clip_color = Some(c);
                }
            }
            _ => {}
        }
    }

    markers.sort_by_key(|m| m.frame);
    (markers, clip_color)
}

fn decode_marker_entry(entry_bytes: &[u8], scope: MarkerScope) -> Option<Marker> {
    let fields = decode_protobuf(entry_bytes).ok()?;
    let mut frame: Option<i64> = None;
    let mut wrapped: Option<Vec<u8>> = None;

    for f in fields {
        match (f.number, f.value) {
            (1, ProtoValue::Bytes(key_bytes)) => {
                if let Ok(kfields) = decode_protobuf(&key_bytes) {
                    for kf in kfields {
                        if kf.number == 1 {
                            if let ProtoValue::Varint(v) = kf.value {
                                frame = Some((v / 2) as i64);
                            }
                        }
                    }
                }
            }
            (2, ProtoValue::Bytes(w)) => wrapped = Some(w),
            _ => {}
        }
    }

    let frame = frame?;
    let wrapped = wrapped?;
    if wrapped.len() < 8 {
        return None;
    }
    // 8-byte header [ver: u32be][len: u32be] followed by protobuf field 1 marker body
    let body_msg = decode_protobuf(&wrapped[8..]).ok()?;
    let body_bytes = body_msg.into_iter().find_map(|f| match (f.number, f.value) {
        (1, ProtoValue::Bytes(b)) => Some(b),
        _ => None,
    })?;
    let body_fields = decode_protobuf(&body_bytes).ok()?;

    let mut color = "Blue".to_string();
    let mut strings: Vec<String> = Vec::new();
    for bf in body_fields {
        match (bf.number, bf.value) {
            (1, ProtoValue::Varint(bits)) => color = marker_color_from_bits(bits),
            (3, ProtoValue::Bytes(b)) => {
                strings.push(String::from_utf8_lossy(&b).into_owned());
            }
            _ => {}
        }
    }

    let comment = strings.first().cloned().unwrap_or_default();
    let duration_frames = strings
        .get(1)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    let name = strings.get(2).cloned().unwrap_or_default();

    Some(Marker {
        frame,
        duration_frames,
        name,
        comment,
        color,
        scope,
    })
}

fn length_field(field_num: u32, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(((field_num as u64) << 3) | 2, &mut out);
    write_varint(bytes.len() as u64, &mut out);
    out.extend_from_slice(bytes);
    out
}

/// Encode Resolve 21 sequence markers into `Sm2SequenceLockableBlob.FieldsBlob` hex.
/// DaVinci Resolve keys timeline markers by frame, so multiple markers landing on the
/// same frame are merged into one combined marker rather than silently overwriting each other.
pub fn encode_timeline_markers(markers: &[Marker]) -> String {
    let mut by_frame: BTreeMap<i64, Marker> = BTreeMap::new();
    for m in markers {
        let frame = m.frame.max(0);
        by_frame
            .entry(frame)
            .and_modify(|existing| {
                if !m.name.is_empty() && !existing.name.contains(&m.name) {
                    if existing.name.is_empty() {
                        existing.name = m.name.clone();
                    } else {
                        existing.name = format!("{} + {}", existing.name, m.name);
                    }
                }
                if !m.comment.is_empty() {
                    if existing.comment.is_empty() {
                        existing.comment = m.comment.clone();
                    } else {
                        existing.comment = format!("{}\n{}", existing.comment, m.comment);
                    }
                }
                existing.duration_frames = existing.duration_frames.max(m.duration_frames);
            })
            .or_insert_with(|| Marker {
                frame,
                ..m.clone()
            });
    }

    let mut entries = Vec::new();
    for (frame, m) in by_frame {
        let frame = frame as u64;
        let dur = m.duration_frames.max(1);
        let color_bits = marker_color_bits(&m.color);

        let mut body = Vec::new();
        write_varint(8, &mut body);
        write_varint(color_bits, &mut body);
        body.extend_from_slice(&length_field(3, m.comment.as_bytes()));
        body.extend_from_slice(&length_field(3, dur.to_string().as_bytes()));
        body.extend_from_slice(&length_field(3, m.name.as_bytes()));

        let marker_msg = length_field(1, &body);
        let mut wrapped = Vec::with_capacity(8 + marker_msg.len());
        wrapped.extend_from_slice(&2u32.to_be_bytes());
        wrapped.extend_from_slice(&(marker_msg.len() as u32).to_be_bytes());
        wrapped.extend_from_slice(&marker_msg);

        let mut key_inner = Vec::new();
        write_varint(8, &mut key_inner);
        write_varint(frame * 2, &mut key_inner);
        let key = length_field(1, &key_inner);

        let mut entry = key;
        entry.extend_from_slice(&length_field(2, &wrapped));
        entries.extend_from_slice(&length_field(2, &entry));
    }

    let proto = length_field(2, &entries);
    let mut blob_data = Vec::with_capacity(9 + proto.len());
    blob_data.extend_from_slice(&10001u32.to_be_bytes());
    blob_data.extend_from_slice(&((proto.len() + 1) as u32).to_be_bytes());
    blob_data.push(0x80);
    blob_data.extend_from_slice(&proto);

    let key = encode_utf16be("BlobData");
    let mut fields_blob = Vec::new();
    fields_blob.extend_from_slice(&1u32.to_be_bytes());
    fields_blob.extend_from_slice(&1u32.to_be_bytes());
    fields_blob.extend_from_slice(&(key.len() as u32).to_be_bytes());
    fields_blob.extend_from_slice(&key);
    fields_blob.extend_from_slice(&12u32.to_be_bytes());
    fields_blob.push(0);
    fields_blob.extend_from_slice(&(blob_data.len() as u32).to_be_bytes());
    fields_blob.extend_from_slice(&blob_data);
    hex_encode(&fields_blob)
}

pub fn sequence_lockable_blob_xml(sequence_db_id: &str, markers: &[Marker]) -> String {
    let fields_blob = encode_timeline_markers(markers);
    let blob_id = Uuid::new_v4();
    format!(
        "<Element><Sm2SequenceLockableBlob DbId=\"{blob_id}\"><FieldsBlob>{fields_blob}</FieldsBlob><BlobOwner>{sequence_db_id}</BlobOwner><DbSavedTime>0</DbSavedTime></Sm2SequenceLockableBlob></Element>"
    )
}

pub fn clip_color_lockable_blob_xml(clip_db_id: &str, resolve_color: u32) -> String {
    let proto = encode_protobuf(&[ProtoField::varint(5, (resolve_color as u64) * 2)]);
    let mut blob_data = Vec::with_capacity(9 + proto.len());
    blob_data.extend_from_slice(&10001u32.to_be_bytes());
    blob_data.extend_from_slice(&((proto.len() + 1) as u32).to_be_bytes());
    blob_data.push(0x80);
    blob_data.extend_from_slice(&proto);

    let key = encode_utf16be("BlobData");
    let mut fields_blob = Vec::new();
    fields_blob.extend_from_slice(&1u32.to_be_bytes());
    fields_blob.extend_from_slice(&1u32.to_be_bytes());
    fields_blob.extend_from_slice(&(key.len() as u32).to_be_bytes());
    fields_blob.extend_from_slice(&key);
    fields_blob.extend_from_slice(&12u32.to_be_bytes());
    fields_blob.push(0);
    fields_blob.extend_from_slice(&(blob_data.len() as u32).to_be_bytes());
    fields_blob.extend_from_slice(&blob_data);

    let hex = hex_encode(&fields_blob);
    let blob_id = Uuid::new_v4();
    format!(
        "<Element><Sm2TiItemLockableBlob DbId=\"{blob_id}\"><FieldsBlob>{hex}</FieldsBlob><BlobOwner>{clip_db_id}</BlobOwner><DbSavedTime>0</DbSavedTime></Sm2TiItemLockableBlob></Element>"
    )
}

/// Resolve standard MediaPoolItem metadata enum IDs (verified in Resolve Studio 21.2).
pub fn resolve_meta_enum_to_name(enum_id: u64) -> Option<&'static str> {
    match enum_id {
        64 => Some("Description"),
        65 => Some("Comments"),
        66 => Some("Scene"),
        67 => Some("Shot"),
        68 => Some("Take"),
        69 => Some("Angle"),
        74 => Some("Keywords"),
        78 => Some("Shoot Day"),
        80 => Some("Camera #"),
        88 => Some("Location"),
        96 => Some("Camera Manufacturer"),
        97 => Some("Camera Type"),
        100 => Some("Camera Notes"),
        107 => Some("ISO"),
        115 => Some("LUT Used"),
        116 => Some("Lens Type"),
        211 => Some("Audio Notes"),
        304 => Some("Production Name"),
        336 => Some("Director"),
        _ => None,
    }
}

/// Map canonical clip metadata field name to Resolve's `BtMetadataLockableBlob` enum ID.
pub fn name_to_resolve_meta_enum(name: &str) -> Option<u64> {
    match name.trim().to_ascii_lowercase().as_str() {
        "description" => Some(64),
        "comments" | "comment" => Some(65),
        "scene" => Some(66),
        "shot" => Some(67),
        "take" => Some(68),
        "angle" => Some(69),
        "keywords" | "keyword" => Some(74),
        "shoot day" => Some(78),
        "camera #" | "camera" => Some(80),
        "location" => Some(88),
        "camera manufacturer" => Some(96),
        "camera type" => Some(97),
        "camera notes" => Some(100),
        "iso" => Some(107),
        "lut used" => Some(115),
        "lens type" => Some(116),
        "audio notes" => Some(211),
        "production name" => Some(304),
        "director" => Some(336),
        _ => None,
    }
}

/// Encode Resolve 21.2 `BtMetadataLockableBlob` XML for a Media Pool clip's standard metadata
/// (`f2` enum entries) and third-party/custom metadata (`f4` string key-value entries).
/// `info_db_id` must be the `DbId` of the clip's `<BtVideoInfo>` (or `<BtAudioInfo>` for audio-only).
pub fn media_metadata_lockable_blob_xml(
    info_db_id: &str,
    std_meta: &BTreeMap<String, String>,
    custom_meta: &BTreeMap<String, String>,
) -> Option<String> {
    if info_db_id.is_empty() || (std_meta.is_empty() && custom_meta.is_empty()) {
        return None;
    }

    let mut std_by_enum: BTreeMap<u64, String> = BTreeMap::new();
    let mut extra_third_party: BTreeMap<String, String> = custom_meta.clone();

    for (k, v) in std_meta {
        if v.is_empty() {
            continue;
        }
        if let Some(eid) = name_to_resolve_meta_enum(k) {
            std_by_enum.entry(eid).or_insert_with(|| v.clone());
        } else if k.eq_ignore_ascii_case("log note") && !std_by_enum.contains_key(&65) {
            std_by_enum.insert(65, v.clone());
        } else if k.eq_ignore_ascii_case("tape name") {
            // Native Resolve MAM/Premiere interop convention for tape/reel name in ThirdPartyMetadata.
            extra_third_party
                .entry("Column.Intrinsic.TapeName".to_string())
                .or_insert_with(|| v.clone());
        } else {
            // Preserve unmapped file metadata fields in ThirdPartyMetadata so nothing is lost.
            extra_third_party.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    if std_by_enum.is_empty() && extra_third_party.is_empty() {
        return None;
    }

    let mut inner = Vec::new();
    for (eid, val) in &std_by_enum {
        let entry = encode_protobuf(&[
            ProtoField::varint(1, *eid),
            ProtoField::bytes(2, val.as_bytes().to_vec()),
        ]);
        inner.extend_from_slice(&length_field(2, &entry));
    }
    for (k, v) in &extra_third_party {
        if k.is_empty() {
            continue;
        }
        let entry = encode_protobuf(&[
            ProtoField::bytes(1, k.as_bytes().to_vec()),
            ProtoField::bytes(2, v.as_bytes().to_vec()),
        ]);
        inner.extend_from_slice(&length_field(4, &entry));
    }

    // 8-byte BtMetadata header: [1_000_001 (0x000f4241): u32be][inner_len: u32be]
    let mut wrapped_inner = Vec::with_capacity(8 + inner.len());
    wrapped_inner.extend_from_slice(&1_000_001u32.to_be_bytes());
    wrapped_inner.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    wrapped_inner.extend_from_slice(&inner);

    let proto = length_field(2, &wrapped_inner);
    let mut blob_data = Vec::with_capacity(9 + proto.len());
    blob_data.extend_from_slice(&10001u32.to_be_bytes());
    blob_data.extend_from_slice(&((proto.len() + 1) as u32).to_be_bytes());
    blob_data.push(0x80);
    blob_data.extend_from_slice(&proto);

    let key = encode_utf16be("BlobData");
    let mut fields_blob = Vec::new();
    fields_blob.extend_from_slice(&1u32.to_be_bytes());
    fields_blob.extend_from_slice(&1u32.to_be_bytes());
    fields_blob.extend_from_slice(&(key.len() as u32).to_be_bytes());
    fields_blob.extend_from_slice(&key);
    fields_blob.extend_from_slice(&12u32.to_be_bytes());
    fields_blob.push(0);
    fields_blob.extend_from_slice(&(blob_data.len() as u32).to_be_bytes());
    fields_blob.extend_from_slice(&blob_data);

    let hex = hex_encode(&fields_blob);
    let blob_id = Uuid::new_v4();
    Some(format!(
        "<Element><BtMetadataLockableBlob DbId=\"{blob_id}\"><FieldsBlob>{hex}</FieldsBlob><BlobOwner>{info_db_id}</BlobOwner><DbSavedTime>0</DbSavedTime></BtMetadataLockableBlob></Element>"
    ))
}

/// Decode Resolve `BtMetadataLockableBlob` `FieldsBlob` hex (`0x80` raw or `0x81` zstd)
/// into `(standard_metadata, third_party_custom_metadata)`.
pub fn decode_media_metadata_blob(
    fields_blob_hex: &str,
) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut std_meta = BTreeMap::new();
    let mut custom_meta = BTreeMap::new();

    let Ok(raw) = hex_decode(fields_blob_hex) else {
        return (std_meta, custom_meta);
    };
    let Ok(entries) = decode_keyed_fields(&raw) else {
        return (std_meta, custom_meta);
    };
    let Some(blob_data) = entries
        .into_iter()
        .find(|(k, _, _)| k == "BlobData")
        .map(|(_, _, v)| v)
    else {
        return (std_meta, custom_meta);
    };
    let Ok(proto_bytes) = decode_wrapper_bytes(&blob_data) else {
        return (std_meta, custom_meta);
    };
    let Ok(top_fields) = decode_protobuf(&proto_bytes) else {
        return (std_meta, custom_meta);
    };

    for f in top_fields {
        if f.number != 2 {
            continue;
        }
        let ProtoValue::Bytes(wrapped) = f.value else {
            continue;
        };
        if wrapped.len() < 8 {
            continue;
        }
        let Ok(inner_fields) = decode_protobuf(&wrapped[8..]) else {
            continue;
        };
        for inf in inner_fields {
            match (inf.number, inf.value) {
                (2, ProtoValue::Bytes(entry_bytes)) => {
                    if let Ok(sub) = decode_protobuf(&entry_bytes) {
                        let mut eid: Option<u64> = None;
                        let mut val: Option<String> = None;
                        for sf in sub {
                            match (sf.number, sf.value) {
                                (1, ProtoValue::Varint(v)) => eid = Some(v),
                                (2, ProtoValue::Bytes(b)) => {
                                    val = Some(String::from_utf8_lossy(&b).into_owned());
                                }
                                _ => {}
                            }
                        }
                        if let (Some(id), Some(v)) = (eid, val) {
                            if !v.is_empty() {
                                if let Some(name) = resolve_meta_enum_to_name(id) {
                                    std_meta.insert(name.to_string(), v);
                                }
                            }
                        }
                    }
                }
                (4, ProtoValue::Bytes(entry_bytes)) => {
                    if let Ok(sub) = decode_protobuf(&entry_bytes) {
                        let mut k: Option<String> = None;
                        let mut v: Option<String> = None;
                        for sf in sub {
                            if let ProtoValue::Bytes(b) = sf.value {
                                match sf.number {
                                    1 => k = Some(String::from_utf8_lossy(&b).into_owned()),
                                    2 => v = Some(String::from_utf8_lossy(&b).into_owned()),
                                    _ => {}
                                }
                            }
                        }
                        if let (Some(key), Some(val)) = (k, v) {
                            if !key.is_empty() {
                                if key == "Column.Intrinsic.TapeName" {
                                    if !val.is_empty() {
                                        std_meta.entry("Tape Name".to_string()).or_insert(val);
                                    }
                                } else {
                                    custom_meta.insert(key, val);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    (std_meta, custom_meta)
}

pub fn append_lockable_blob(project_xml: &str, blob_xml: &str) -> NleResult<String> {
    let Some(pos) = project_xml.find("</LocableBlobSet>") else {
        return Err(NleError::InvalidFormat("project.xml missing </LocableBlobSet>".into()));
    };
    let mut out = String::with_capacity(project_xml.len() + blob_xml.len());
    out.push_str(&project_xml[..pos]);
    out.push_str(blob_xml);
    out.push_str(&project_xml[pos..]);
    Ok(out)
}

/// Apply Resolve 21 color science preset blob (`default`, `acescct`, `hlg`) into `project.xml`.
pub fn configure_project_color_blob(
    members: &mut BTreeMap<String, Vec<u8>>,
    color_config: &ColorConfig,
) -> NleResult<()> {
    let default_blob = members.remove("pr2/setup_default.blob");
    let acescct_blob = members.remove("pr2/setup_acescct.blob");
    let hlg_blob = members.remove("pr2/setup_hlg.blob");

    let working = color_config
        .working_space
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase()
        .replace("rec. ", "rec.");
    let output = color_config
        .output_space
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase()
        .replace("rec. ", "rec.");

    let chosen = if working.contains("acescct") {
        acescct_blob
    } else if working.contains("2100 hlg") || output.contains("2100 hlg") {
        hlg_blob
    } else {
        default_blob
    };

    if let Some(blob) = chosen {
        let project_xml = String::from_utf8(
            members
                .get("project.xml")
                .cloned()
                .ok_or_else(|| NleError::InvalidFormat("Template missing project.xml".into()))?,
        )
        .map_err(|e| NleError::InvalidFormat(e.to_string()))?;

        let re = Regex::new(r"(?s)(<SM_Config\b.*?<FieldsBlob>)[0-9a-fA-F]*(</FieldsBlob>)").unwrap();
        let hex = hex_encode(&blob);
        let updated = re.replace(&project_xml, format!("${{1}}{hex}${{2}}"));
        members.insert("project.xml".into(), updated.into_owned().into_bytes());
    }
    Ok(())
}

/// Build Resolve 21's 422-byte `SequenceSetup` blob for custom per-timeline resolution & frame rate.
pub fn build_sequence_setup_blob(width: u32, height: u32, fps_int: u32) -> Vec<u8> {
    const BASE_HEX: &str = "00000000000000030000000000216e6200000000000000000000000000000064000000000000000600000000000004380000000000000780000000000000000000000000000000000000803f00000000000000190000000000000000ffffffffffffffff0000000000000001fffffffffffffffe0000000000000001000000000000000000000000000000c40000000000000005000000000000006d00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000004380000000000000780000000000000000000000000000000000000803fffffffffffffffff00000000000000010000000000000221000000000000000000000000000000000000000000000000000000000000000000000000000000000000003f0000003f000000000000004e080a100e1800200840a00172134e6f204f7574707574205472616e73666f726d880144900101c00108e80101880208a80201b00202b8029003c80208e0020ee80202800304a00301a8030eb00301";
    let mut buf = hex_decode(BASE_HEX).expect("valid SequenceSetup template");
    let w = (width.max(16) as u64).to_be_bytes();
    let h = (height.max(16) as u64).to_be_bytes();
    let f = (fps_int.max(1) as u64).to_be_bytes();
    buf[40..48].copy_from_slice(&w);
    buf[48..56].copy_from_slice(&h);
    buf[76..84].copy_from_slice(&f);
    buf[244..252].copy_from_slice(&w);
    buf[252..260].copy_from_slice(&h);
    buf
}

/// Patch Resolve 21 `SM_Config` `SetupBA` protobuf fields for default resolution, frame rate,
/// Input Scaling (Fit), and DaVinci YRGB Color Managed mode when using the default Rec.709 workflow.
pub fn patch_project_setup_defaults(
    project_xml: &str,
    width: u32,
    height: u32,
    fps_int: u32,
    color_config: &ColorConfig,
) -> NleResult<String> {
    let re = Regex::new(r"(?s)(<SM_Config\b.*?<FieldsBlob>)([0-9a-fA-F]+)(</FieldsBlob>)").unwrap();
    let Some(caps) = re.captures(project_xml) else {
        return Ok(project_xml.to_string());
    };
    let fields = hex_decode(&caps[2])?;
    let key = encode_utf16be("SetupBA");
    let mut first_field = Vec::new();
    first_field.extend_from_slice(&(key.len() as u32).to_be_bytes());
    first_field.extend_from_slice(&key);
    first_field.extend_from_slice(&12u32.to_be_bytes());
    first_field.push(0);

    let prefix_len = 8 + first_field.len();
    if fields.len() < prefix_len + 13 || &fields[8..prefix_len] != first_field.as_slice() {
        return Ok(project_xml.to_string());
    }
    let wrapped_len = u32::from_be_bytes(fields[prefix_len..prefix_len + 4].try_into().unwrap()) as usize;
    let value_end = prefix_len + 4 + wrapped_len;
    if wrapped_len < 9 || value_end > fields.len() || fields[prefix_len + 4 + 8] != 0x80 {
        return Ok(project_xml.to_string());
    }

    let raw_payload = &fields[prefix_len + 4 + 9..value_end];
    let mut updates = BTreeMap::new();
    if width > 0 {
        updates.insert(2u32, (width as u64) * 2);
    }
    if height > 0 {
        updates.insert(3u32, (height as u64) * 2);
    }
    if fps_int > 0 {
        updates.insert(15u32, (fps_int as u64) * 2);
    }

    // Enable DaVinci YRGB Color Managed (Automatic Color Management -> SDR Rec.709) by default
    // unless ACEScct or explicit HLG output was requested, so HDR/Log clips (e.g. BT.2100 PQ)
    // automatically tone-map into Rec.709 just like Premiere Pro's Auto Tone Map.
    let working = color_config
        .working_space
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase()
        .replace("rec. ", "rec.");
    let output = color_config
        .output_space
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase()
        .replace("rec. ", "rec.");
    let is_special_preset =
        working.contains("acescct") || working.contains("2100 hlg") || output.contains("2100 hlg");
    if !is_special_preset {
        // Verified DaVinci Resolve 21.2 YRGB Color Managed (Automatic, SDR Rec.709) SetupBA fields:
        updates.insert(7, 10);
        updates.insert(11, 2);
        updates.insert(183, 160);
        updates.insert(184, 0);
        updates.insert(190, 68);
        updates.insert(191, 1);
        updates.insert(195, 1);
        updates.insert(202, 8);
        updates.insert(203, 8);
        updates.insert(209, 1);
        updates.insert(210, 2);
        updates.insert(211, 400);
        updates.insert(212, 8);
        updates.insert(215, 8);
        updates.insert(225, 14);
        updates.insert(276, 26);
        updates.insert(307, 2);
    }

    let patched_payload = patch_proto_varints(raw_payload, &updates)?;
    let mut wrapped = Vec::with_capacity(9 + patched_payload.len());
    wrapped.extend_from_slice(&1u32.to_be_bytes());
    wrapped.extend_from_slice(&((patched_payload.len() + 1) as u32).to_be_bytes());
    wrapped.push(0x80);
    wrapped.extend_from_slice(&patched_payload);

    let mut new_fields = Vec::new();
    new_fields.extend_from_slice(&fields[..prefix_len]);
    new_fields.extend_from_slice(&(wrapped.len() as u32).to_be_bytes());
    new_fields.extend_from_slice(&wrapped);
    new_fields.extend_from_slice(&fields[value_end..]);

    let m = caps.get(2).unwrap();
    let mut out = String::with_capacity(project_xml.len());
    out.push_str(&project_xml[..m.start()]);
    out.push_str(&hex_encode(&new_fields));
    out.push_str(&project_xml[m.end()..]);
    Ok(out)
}

fn patch_proto_varints(data: &[u8], updates: &BTreeMap<u32, u64>) -> NleResult<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    let mut pos = 0usize;
    let mut seen = std::collections::HashSet::new();
    while pos < data.len() {
        let start = pos;
        let key = read_varint(data, &mut pos)?;
        let number = (key >> 3) as u32;
        let wire = (key & 7) as u8;
        let val_start = pos;
        match wire {
            0 => {
                let _ = read_varint(data, &mut pos)?;
            }
            1 => pos += 8,
            2 => {
                let len = read_varint(data, &mut pos)? as usize;
                let payload_start = pos;
                pos += len;
                if pos > data.len() {
                    return Err(NleError::InvalidFormat("Truncated SetupBA field".into()));
                }
                // Patch Resolve Input Scaling (f235) from Center Crop (0x2B) to Scale Entire Image to Fit (0x6D)
                // so ZoomX=1.0 matches our motion transform math (zoom = (scale / 100) / fit) and Resolve's default.
                if number == 235
                    && len >= 16
                    && data[payload_start..payload_start + 16]
                        == [0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0x2B]
                {
                    out.extend_from_slice(&data[start..payload_start + 15]);
                    out.push(0x6D);
                    out.extend_from_slice(&data[payload_start + 16..pos]);
                    continue;
                }
                out.extend_from_slice(&data[start..pos]);
                continue;
            }
            5 => pos += 4,
            _ => return Err(NleError::InvalidFormat("Invalid SetupBA wire type".into())),
        }
        if pos > data.len() {
            return Err(NleError::InvalidFormat("Truncated SetupBA field".into()));
        }
        if let Some(&new_val) = updates.get(&number) {
            if wire == 0 && seen.insert(number) {
                out.extend_from_slice(&data[start..val_start]);
                write_varint(new_val, &mut out);
                continue;
            }
        }
        out.extend_from_slice(&data[start..pos]);
    }
    for (&number, &new_val) in updates {
        if !seen.contains(&number) {
            write_varint((number as u64) << 3, &mut out);
            write_varint(new_val, &mut out);
        }
    }
    Ok(out)
}
