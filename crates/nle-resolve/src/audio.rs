use crate::binary::{decode_keyed_fields, encode_keyed_fields, hex_encode};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use nle_core::{NleError, NleResult};
use std::collections::HashMap;
use std::io::{Read, Write};

/// Encode a 1-based audio source channel in Resolve 21's version-2 `VirtualAudioTrackBA`.
pub fn mono_channel_mapping(source_channel: u32) -> String {
    let ch = source_channel.clamp(1, 0x3FFF);
    let mut channels_ba = Vec::with_capacity(12);
    channels_ba.extend_from_slice(&2u32.to_be_bytes());
    channels_ba.extend_from_slice(&1u32.to_be_bytes());
    channels_ba.extend_from_slice(&(0x4000 + ch).to_be_bytes());

    let entries = vec![
        ("ChannelsBA".into(), 12, channels_ba),
        ("AudioType".into(), 2, 1u32.to_be_bytes().to_vec()),
    ];
    hex_encode(&encode_keyed_fields(&entries))
}

#[derive(Debug, Clone)]
pub struct FairlightTrackSpec {
    pub is_mono: bool,
    pub name: String,
    pub enabled: bool,
    pub locked: bool,
}

const FIXED_MODEL_COUNTS: [u32; 3] = [0x18, 0x1C, 0x2F6];

fn mono_default(field: u32, occurrence: usize) -> Option<u32> {
    match (field, occurrence) {
        (0x4, 0) | (0x5, 1) | (0xB4, 0) | (0xB8, 0) | (0xB9, 0) | (0x4B5, 0) => Some(0),
        (0xBA, 0) => Some(1),
        (0xF2, 0) => Some(119),
        (0x334, 0) => Some(85),
        (0x335, 0) => Some(137),
        (0x50D, 0) => Some(5000),
        _ => None,
    }
}

enum RowValues {
    Ints(Vec<u32>),
    Names,
}

/// Rebuild Fairlight `FLStudioModelBA` channel strips inside `Sm2Sequence.FieldsBlob`
/// so audio track names, mute, lock, and count stay valid in DaVinci Resolve 21.
pub fn rebuild_fairlight_track_model(
    fields_blob: &[u8],
    tracks: &[FairlightTrackSpec],
) -> NleResult<Vec<u8>> {
    if tracks.is_empty() {
        return Ok(fields_blob.to_vec());
    }
    let mut fields = decode_keyed_fields(fields_blob)?;
    let Some((_, kind, wrapped)) = fields.iter_mut().find(|(k, _, _)| k == "FLStudioModelBA") else {
        return Err(NleError::InvalidFormat("Sequence FieldsBlob missing FLStudioModelBA".into()));
    };
    if *kind != 12 || wrapped.len() < 4 {
        return Err(NleError::InvalidFormat("Invalid FLStudioModelBA envelope".into()));
    }
    let expected_len = u32::from_be_bytes(wrapped[0..4].try_into().unwrap()) as usize;
    let mut decoder = ZlibDecoder::new(&wrapped[4..]);
    let mut payload = Vec::new();
    decoder
        .read_to_end(&mut payload)
        .map_err(|e| NleError::Archive(format!("Failed to inflate FLStudioModelBA: {e}")))?;
    if payload.len() != expected_len {
        return Err(NleError::InvalidFormat("FLStudioModelBA decompressed size mismatch".into()));
    }

    let rebuilt = rebuild_model_payload(&payload, tracks)?;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&rebuilt)
        .map_err(|e| NleError::Archive(format!("Failed to deflate FLStudioModelBA: {e}")))?;
    let compressed = encoder
        .finish()
        .map_err(|e| NleError::Archive(format!("Failed to finish FLStudioModelBA zlib: {e}")))?;

    let mut new_wrapped = Vec::with_capacity(4 + compressed.len());
    new_wrapped.extend_from_slice(&(rebuilt.len() as u32).to_be_bytes());
    new_wrapped.extend_from_slice(&compressed);
    *wrapped = new_wrapped;

    Ok(encode_keyed_fields(&fields))
}

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from + needle.len() > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn rebuild_model_payload(data: &[u8], tracks: &[FairlightTrackSpec]) -> NleResult<Vec<u8>> {
    let last_client = find_bytes(data, b"LastClient", 0)
        .ok_or_else(|| NleError::InvalidFormat("Fairlight payload missing LastClient".into()))?;
    if last_client < 16 {
        return Err(NleError::InvalidFormat("Fairlight LastClient offset invalid".into()));
    }
    let section = last_client - 16;
    let start_marker: Vec<u8> = [1u32, 1, 2]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let start = find_bytes(data, &start_marker, section + 100)
        .ok_or_else(|| NleError::InvalidFormat("Fairlight template missing channel records".into()))?;

    let mut offset = start;
    let mut rows: Vec<(u32, u32, RowValues)> = Vec::new();
    while offset + 12 <= data.len() {
        let flag = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        let field = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
        let count = u32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap());
        if flag != 1 || field == 0 || field >= 0x2000 || count > 8192 {
            break;
        }
        let mut pos = offset + 12;
        let values = if field == 0xBC {
            for _ in 0..count {
                if pos + 4 > data.len() {
                    return Err(NleError::InvalidFormat("Truncated Fairlight channel name".into()));
                }
                let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4;
                if pos + len > data.len() {
                    return Err(NleError::InvalidFormat("Invalid Fairlight channel name bounds".into()));
                }
                pos += len;
            }
            RowValues::Names
        } else {
            let byte_len = (count as usize) * 4;
            if pos + byte_len > data.len() {
                return Err(NleError::InvalidFormat("Invalid Fairlight channel value bounds".into()));
            }
            let mut ints = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                let p = pos + i * 4;
                ints.push(u32::from_le_bytes(data[p..p + 4].try_into().unwrap()));
            }
            pos += byte_len;
            RowValues::Ints(ints)
        };
        rows.push((field, count, values));
        offset = pos;
    }

    let end = offset;
    let total_channels: u32 = tracks.iter().map(|t| if t.is_mono { 1 } else { 2 }).sum();
    let mut parts: Vec<u8> = Vec::new();
    let mut occurrences: HashMap<u32, usize> = HashMap::new();

    for (field, count, values) in &rows {
        let occ = *occurrences.get(field).unwrap_or(&0);
        occurrences.insert(*field, occ + 1);

        if FIXED_MODEL_COUNTS.contains(field) {
            parts.extend_from_slice(&1u32.to_le_bytes());
            parts.extend_from_slice(&field.to_le_bytes());
            parts.extend_from_slice(&count.to_le_bytes());
            if let RowValues::Ints(ints) = values {
                for v in ints {
                    parts.extend_from_slice(&v.to_le_bytes());
                }
            }
            continue;
        }

        parts.extend_from_slice(&1u32.to_le_bytes());
        parts.extend_from_slice(&field.to_le_bytes());
        parts.extend_from_slice(&total_channels.to_le_bytes());

        for (track_idx, spec) in tracks.iter().enumerate() {
            let width = if spec.is_mono { 1 } else { 2 };
            if *field == 0xBC {
                let nb = spec.name.as_bytes();
                for _ in 0..width {
                    parts.extend_from_slice(&(nb.len() as u32).to_le_bytes());
                    parts.extend_from_slice(nb);
                }
                continue;
            }
            let ints = match values {
                RowValues::Ints(v) => v,
                RowValues::Names => continue,
            };
            for ch in 0..width {
                let mut val = ints.get(ch).copied().unwrap_or(0);
                if spec.is_mono {
                    if let Some(d) = mono_default(*field, occ) {
                        val = d;
                    }
                    if (0x153..=0x16A).contains(field) {
                        val = 0;
                    }
                }
                if *field == 0x458 {
                    val = if ch == 0 { track_idx as u32 } else { 0xFFFF_FFFF };
                } else if *field == 0x2 {
                    val = if spec.enabled { 0 } else { 1 };
                } else if *field == 0xDB {
                    val = if spec.locked { 1 } else { 0 };
                }
                parts.extend_from_slice(&val.to_le_bytes());
            }
        }
    }

    let mut suffix = data[end..].to_vec();
    let track_len = tracks.len() as u32;
    let strip_updates: &[(u32, &[u32], Vec<u32>)] = &[
        (0x458, &[2, 1], vec![track_len + 1, track_len]),
        (0x324, &[2], vec![track_len.max(2)]),
        (0x32F, &[1], vec![track_len]),
    ];

    for (field, expected, updates) in strip_updates {
        let mut pat = Vec::with_capacity(12);
        pat.extend_from_slice(&1u32.to_le_bytes());
        pat.extend_from_slice(&field.to_le_bytes());
        pat.extend_from_slice(&1u32.to_le_bytes());

        let mut offsets = Vec::new();
        let mut cursor = 0;
        while let Some(pos) = find_bytes(&suffix, &pat, cursor) {
            if pos + 16 <= suffix.len() {
                offsets.push(pos + 12);
            }
            cursor = pos + 1;
        }
        let actual: Vec<u32> = offsets
            .iter()
            .map(|&o| u32::from_le_bytes(suffix[o..o + 4].try_into().unwrap()))
            .collect();
        if actual == *expected {
            for (&o, &new_v) in offsets.iter().zip(updates.iter()) {
                suffix[o..o + 4].copy_from_slice(&new_v.to_le_bytes());
            }
        }
    }

    let mut result = Vec::with_capacity(start + parts.len() + suffix.len());
    result.extend_from_slice(&data[..start]);
    result.extend_from_slice(&parts);
    result.extend_from_slice(&suffix);
    if result.len() >= 12 {
        let body_len = (result.len() - 12) as u32;
        result[8..12].copy_from_slice(&body_len.to_le_bytes());
    }
    Ok(result)
}
