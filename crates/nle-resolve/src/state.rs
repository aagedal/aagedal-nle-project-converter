use crate::binary::{
    encode_protobuf, encode_utf16be, hex_decode, hex_encode, read_varint, write_varint, ProtoField,
};
use nle_core::{ColorConfig, Marker, NleError, NleResult};
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

/// Patch Resolve 21 `SM_Config` `SetupBA` protobuf fields for default resolution and frame rate.
pub fn patch_project_setup_defaults(
    project_xml: &str,
    width: u32,
    height: u32,
    fps_int: u32,
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
                pos += len;
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
