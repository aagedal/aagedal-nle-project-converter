use crate::binary::{decode_keyed_fields, encode_keyed_fields, hex_decode, hex_encode};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use nle_core::{AudioBus, NleError, NleResult};
use std::collections::HashMap;
use std::io::{Read, Write};

/// Decode Resolve 21's `VirtualAudioTrackBA` hex into `(channel_count, first_source_channel_1based)`.
pub fn decode_virtual_audio_track(hex: &str) -> Option<(u32, u32)> {
    let raw = hex_decode(hex).ok()?;
    let fields = decode_keyed_fields(&raw).ok()?;
    let mut audio_type: Option<u32> = None;
    let mut first_channel: Option<u32> = None;
    let mut channel_count: Option<u32> = None;

    for (k, _, v) in fields {
        match k.as_str() {
            "AudioType" if v.len() >= 4 => {
                audio_type = Some(u32::from_be_bytes(v[0..4].try_into().ok()?));
            }
            "ChannelsBA" if v.len() >= 12 => {
                let count = u32::from_be_bytes(v[4..8].try_into().ok()?);
                let ch0_raw = u32::from_be_bytes(v[8..12].try_into().ok()?);
                let ch0 = if ch0_raw > 0x4000 {
                    ch0_raw - 0x4000
                } else {
                    ch0_raw.max(1)
                };
                if count > 0 {
                    channel_count = Some(count);
                }
                if ch0 > 0 {
                    first_channel = Some(ch0);
                }
            }
            _ => {}
        }
    }

    let ch_count = channel_count.or_else(|| match audio_type {
        Some(1) => Some(1),
        Some(0) => Some(2),
        _ => None,
    })?;
    Some((ch_count, first_channel.unwrap_or(1)))
}

/// Encode a 1-based audio source channel in Resolve 21's version-2 `VirtualAudioTrackBA` (Mono).
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

/// Encode a 1-based stereo pair (`left_channel`, `left_channel + 1`) in Resolve 21's `VirtualAudioTrackBA`.
pub fn stereo_channel_mapping(left_channel: u32) -> String {
    let ch_l = left_channel.clamp(1, 0x3FFE);
    let ch_r = ch_l + 1;
    let mut channels_ba = Vec::with_capacity(16);
    channels_ba.extend_from_slice(&2u32.to_be_bytes());
    channels_ba.extend_from_slice(&2u32.to_be_bytes());
    channels_ba.extend_from_slice(&(0x4000 + ch_l).to_be_bytes());
    channels_ba.extend_from_slice(&(0x4000 + ch_r).to_be_bytes());

    let entries = vec![
        ("ChannelsBA".into(), 12, channels_ba),
        ("AudioType".into(), 2, 0u32.to_be_bytes().to_vec()),
    ];
    hex_encode(&encode_keyed_fields(&entries))
}

#[derive(Debug, Clone)]
pub struct FairlightTrackSpec {
    pub is_mono: bool,
    pub name: String,
    pub enabled: bool,
    pub locked: bool,
    /// 0-based index into the sequence's Fairlight bus table (`0` = primary/main bus).
    pub bus_index: usize,
}

#[derive(Debug, Clone)]
pub struct FairlightBusSpec {
    pub name: String,
    pub is_main: bool,
    /// Optional 0-based index of the parent bus (`None` = main output bus).
    pub parent_bus_index: Option<usize>,
}

/// Decoded Fairlight FlexBus mixer state from `Sm2Sequence.FieldsBlob` (`FLStudioModelBA`).
#[derive(Debug, Clone, Default)]
pub struct FairlightMixerData {
    pub buses: Vec<AudioBus>,
    /// 0-based timeline audio track index -> assigned `AudioBus.id`.
    pub track_bus_ids: HashMap<usize, String>,
    /// 0-based timeline audio track index -> channel count (1 = mono, 2 = stereo).
    pub track_channels: HashMap<usize, u32>,
}

const FIXED_TRACK_COUNTS: [u32; 3] = [0x18, 0x1C, 0x2F6];
const FIXED_BUS_COUNTS: [u32; 1] = [0x1C];

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
    Names(Vec<String>),
}

/// Decode Fairlight FlexBus buses and track-to-bus routing from `Sm2Sequence.FieldsBlob` raw bytes.
pub fn decode_fairlight_mixer(fields_blob: &[u8]) -> Option<FairlightMixerData> {
    let fields = decode_keyed_fields(fields_blob).ok()?;
    let (_, kind, wrapped) = fields.iter().find(|(k, _, _)| k == "FLStudioModelBA")?;
    if *kind != 12 || wrapped.len() < 4 {
        return None;
    }
    let mut decoder = ZlibDecoder::new(&wrapped[4..]);
    let mut data = Vec::new();
    decoder.read_to_end(&mut data).ok()?;

    let last_client = find_bytes(&data, b"LastClient", 0)?;
    if last_client < 16 {
        return None;
    }
    let section = last_client - 16;

    // 1. Locate Track table (Section 0): [u32(0), u32(1), u32(1), u32(track_ch_count)]
    let track_start = find_table_start(&data, section + 100, 0)?;
    let (_, track_rows) = parse_columnar_table(&data, track_start + 4).ok()?;

    // 2. Locate Bus table (Section 7): [u32(7), u32(1), u32(1), u32(bus_count)]
    let bus_start = find_table_start(&data, track_start + 16, 7)?;
    let (_, bus_rows) = parse_columnar_table(&data, bus_start + 4).ok()?;

    let bus_names: Vec<String> = bus_rows
        .iter()
        .find_map(|(f, _, v)| {
            if *f == 0xBC {
                if let RowValues::Names(names) = v {
                    return Some(names.clone());
                }
            }
            None
        })
        .unwrap_or_default();
    if bus_names.is_empty() {
        return None;
    }

    let bus_is_main: Vec<u32> = bus_rows
        .iter()
        .find_map(|(f, _, v)| {
            if *f == 0xAA {
                if let RowValues::Ints(ints) = v {
                    return Some(ints.clone());
                }
            }
            None
        })
        .unwrap_or_default();

    let bus_parent_idx: Vec<u32> = bus_rows
        .iter()
        .find_map(|(f, _, v)| {
            if *f == 0x40B {
                if let RowValues::Ints(ints) = v {
                    return Some(ints.clone());
                }
            }
            None
        })
        .unwrap_or_default();

    let mut buses = Vec::with_capacity(bus_names.len());
    for (idx, name) in bus_names.iter().enumerate() {
        let parent_raw = bus_parent_idx.get(idx).copied().unwrap_or(0xFFFF_FFFF);
        let parent_bus_id = if parent_raw != 0xFFFF_FFFF && (parent_raw as usize) < bus_names.len() {
            Some(format!("bus-{parent_raw}"))
        } else {
            None
        };
        let is_main = bus_is_main.get(idx).copied().unwrap_or(0) != 0 || parent_bus_id.is_none();
        buses.push(AudioBus {
            id: format!("bus-{idx}"),
            name: if name.trim().is_empty() {
                format!("Bus {}", idx + 1)
            } else {
                name.clone()
            },
            channels: 2,
            is_main,
            parent_bus_id,
        });
    }

    let tr_ids = track_rows.iter().find_map(|(f, _, v)| {
        if *f == 0x458 {
            if let RowValues::Ints(ints) = v {
                return Some(ints.clone());
            }
        }
        None
    });
    let tr_bus = track_rows.iter().find_map(|(f, _, v)| {
        if *f == 0x40B {
            if let RowValues::Ints(ints) = v {
                return Some(ints.clone());
            }
        }
        None
    });
    let tr_width = track_rows.iter().find_map(|(f, _, v)| {
        if *f == 0xBA {
            if let RowValues::Ints(ints) = v {
                return Some(ints.clone());
            }
        }
        None
    });

    let mut track_bus_ids = HashMap::new();
    let mut track_channels = HashMap::new();
    if let Some(ids) = tr_ids {
        for (ch_idx, &tid) in ids.iter().enumerate() {
            if tid == 0xFFFF_FFFF {
                continue;
            }
            let tid_usize = tid as usize;
            if let Some(ref widths) = tr_width {
                if let Some(&w) = widths.get(ch_idx) {
                    if w > 0 {
                        track_channels.insert(tid_usize, w);
                    }
                }
            }
            if let Some(ref b_assign) = tr_bus {
                if let Some(&bidx) = b_assign.get(ch_idx) {
                    if bidx != 0xFFFF_FFFF && (bidx as usize) < buses.len() {
                        track_bus_ids.insert(tid_usize, format!("bus-{bidx}"));
                    }
                }
            }
        }
    }

    Some(FairlightMixerData {
        buses,
        track_bus_ids,
        track_channels,
    })
}

/// Rebuild Fairlight `FLStudioModelBA` channel strips and FlexBus mixer buses inside
/// `Sm2Sequence.FieldsBlob` so track formats, names, mute, lock, buses, and routing stay valid in DaVinci Resolve 21.
pub fn rebuild_fairlight_track_model(
    fields_blob: &[u8],
    tracks: &[FairlightTrackSpec],
    buses: &[FairlightBusSpec],
) -> NleResult<Vec<u8>> {
    if tracks.is_empty() && buses.is_empty() {
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

    let default_buses = vec![FairlightBusSpec {
        name: "Bus 1".to_string(),
        is_main: true,
        parent_bus_index: None,
    }];
    let eff_buses = if buses.is_empty() {
        &default_buses[..]
    } else {
        buses
    };

    let rebuilt = rebuild_model_payload(&payload, tracks, eff_buses)?;

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

/// Find `[u32(section_id), u32(1), u32(1), u32(count)]` where `1 <= count <= 512`
/// and the immediately following row also starts with `flag == 1, field == 2`.
fn find_table_start(data: &[u8], from: usize, section_id: u32) -> Option<usize> {
    let prefix: [u8; 12] = [
        section_id.to_le_bytes(),
        1u32.to_le_bytes(),
        1u32.to_le_bytes(),
    ]
    .concat()
    .try_into()
    .unwrap();
    let mut cursor = from;
    while let Some(pos) = find_bytes(data, &prefix, cursor) {
        if pos + 16 <= data.len() {
            let cnt = u32::from_le_bytes(data[pos + 12..pos + 16].try_into().unwrap()) as usize;
            if (1..=512).contains(&cnt) {
                let next_off = pos + 16 + cnt * 4;
                if next_off + 12 <= data.len() {
                    let nf = u32::from_le_bytes(data[next_off..next_off + 4].try_into().unwrap());
                    let npid =
                        u32::from_le_bytes(data[next_off + 4..next_off + 8].try_into().unwrap());
                    if nf == 1 && (2..=0x40).contains(&npid) {
                        return Some(pos);
                    }
                }
            }
        }
        cursor = pos + 1;
    }
    None
}

fn parse_columnar_table(data: &[u8], start: usize) -> NleResult<(usize, Vec<(u32, u32, RowValues)>)> {
    let mut offset = start;
    let mut rows: Vec<(u32, u32, RowValues)> = Vec::new();
    while offset + 12 <= data.len() {
        let flag = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        if flag == 0 {
            break;
        }
        let field = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
        let count = u32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap());
        if flag != 1 || field == 0 || field >= 0x2000 || count > 32768 {
            break;
        }
        let mut pos = offset + 12;
        let values = if field == 0xBC {
            let mut names = Vec::with_capacity(count as usize);
            for _ in 0..count {
                if pos + 4 > data.len() {
                    return Err(NleError::InvalidFormat("Truncated Fairlight name".into()));
                }
                let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4;
                if pos + len > data.len() {
                    return Err(NleError::InvalidFormat("Invalid Fairlight name bounds".into()));
                }
                names.push(String::from_utf8_lossy(&data[pos..pos + len]).into_owned());
                pos += len;
            }
            RowValues::Names(names)
        } else {
            let byte_len = (count as usize) * 4;
            if pos + byte_len > data.len() {
                return Err(NleError::InvalidFormat("Invalid Fairlight value bounds".into()));
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
    Ok((offset, rows))
}

fn rebuild_model_payload(
    data: &[u8],
    tracks: &[FairlightTrackSpec],
    buses: &[FairlightBusSpec],
) -> NleResult<Vec<u8>> {
    let last_client = find_bytes(data, b"LastClient", 0)
        .ok_or_else(|| NleError::InvalidFormat("Fairlight payload missing LastClient".into()))?;
    if last_client < 16 {
        return Err(NleError::InvalidFormat("Fairlight LastClient offset invalid".into()));
    }
    let section = last_client - 16;

    // 1. Rebuild Section 0 (Track Channels)
    let start_marker: Vec<u8> = [1u32, 1, 2]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let track_start = find_bytes(data, &start_marker, section + 100)
        .ok_or_else(|| NleError::InvalidFormat("Fairlight template missing channel records".into()))?;
    let (track_end, track_rows) = parse_columnar_table(data, track_start)?;

    let mut track_parts: Vec<u8> = Vec::new();
    if !tracks.is_empty() {
        let total_channels: u32 = tracks.iter().map(|t| if t.is_mono { 1 } else { 2 }).sum();
        let mut occurrences: HashMap<u32, usize> = HashMap::new();

        for (field, count, values) in &track_rows {
            let occ = *occurrences.get(field).unwrap_or(&0);
            occurrences.insert(*field, occ + 1);

            if FIXED_TRACK_COUNTS.contains(field) {
                track_parts.extend_from_slice(&1u32.to_le_bytes());
                track_parts.extend_from_slice(&field.to_le_bytes());
                track_parts.extend_from_slice(&count.to_le_bytes());
                if let RowValues::Ints(ints) = values {
                    for v in ints {
                        track_parts.extend_from_slice(&v.to_le_bytes());
                    }
                }
                continue;
            }

            track_parts.extend_from_slice(&1u32.to_le_bytes());
            track_parts.extend_from_slice(&field.to_le_bytes());
            track_parts.extend_from_slice(&total_channels.to_le_bytes());

            for (track_idx, spec) in tracks.iter().enumerate() {
                let width = if spec.is_mono { 1 } else { 2 };
                if *field == 0xBC {
                    let nb = spec.name.as_bytes();
                    for _ in 0..width {
                        track_parts.extend_from_slice(&(nb.len() as u32).to_le_bytes());
                        track_parts.extend_from_slice(nb);
                    }
                    continue;
                }
                let ints = match values {
                    RowValues::Ints(v) => v,
                    RowValues::Names(_) => continue,
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
                    } else if *field == 0x400 {
                        val = 1;
                    } else if *field == 0x40B {
                        val = spec.bus_index.min(buses.len().saturating_sub(1)) as u32;
                    }
                    track_parts.extend_from_slice(&val.to_le_bytes());
                }
            }
        }
    } else {
        track_parts.extend_from_slice(&data[track_start..track_end]);
    }

    // 2. Rebuild Section 7 (Buses) inside the post-track suffix
    let suffix_raw = &data[track_end..];
    let track_len = if tracks.is_empty() { 1 } else { tracks.len() as u32 };
    let bus_len = buses.len() as u32;

    let mut suffix = if let Some(bus_sec_pos) = find_table_start(suffix_raw, 0, 7) {
        let bus_start = bus_sec_pos + 4;
        let (bus_end, bus_rows) = parse_columnar_table(suffix_raw, bus_start)?;
        let mut bus_parts: Vec<u8> = Vec::new();
        let multi_bus = buses.len() > 1;

        for (field, count, values) in &bus_rows {
            if FIXED_BUS_COUNTS.contains(field) {
                bus_parts.extend_from_slice(&1u32.to_le_bytes());
                bus_parts.extend_from_slice(&field.to_le_bytes());
                bus_parts.extend_from_slice(&count.to_le_bytes());
                if let RowValues::Ints(ints) = values {
                    for v in ints {
                        bus_parts.extend_from_slice(&v.to_le_bytes());
                    }
                }
                continue;
            }

            bus_parts.extend_from_slice(&1u32.to_le_bytes());
            bus_parts.extend_from_slice(&field.to_le_bytes());
            bus_parts.extend_from_slice(&bus_len.to_le_bytes());

            if *field == 0xBC {
                for b in buses {
                    let nb = b.name.as_bytes();
                    bus_parts.extend_from_slice(&(nb.len() as u32).to_le_bytes());
                    bus_parts.extend_from_slice(nb);
                }
                continue;
            }

            let base_val = match values {
                RowValues::Ints(v) => v.first().copied().unwrap_or(0),
                RowValues::Names(_) => 0,
            };

            for (bus_idx, b) in buses.iter().enumerate() {
                let is_root = b.parent_bus_index.is_none();
                let val = match *field {
                    0xAA => {
                        if multi_bus && (b.is_main || is_root) {
                            1
                        } else {
                            0
                        }
                    }
                    0x10A => {
                        if is_root {
                            0xFFFF_0000
                        } else {
                            0
                        }
                    }
                    0x145 => {
                        if is_root {
                            1
                        } else {
                            0
                        }
                    }
                    0x1A5 => {
                        if is_root {
                            0
                        } else {
                            8
                        }
                    }
                    0x400 => {
                        if is_root {
                            0
                        } else {
                            1
                        }
                    }
                    0x40B => b
                        .parent_bus_index
                        .filter(|&p| p < buses.len() && p != bus_idx)
                        .map(|p| p as u32)
                        .unwrap_or(0xFFFF_FFFF),
                    0x458 => track_len + 1 + (bus_idx as u32),
                    _ => base_val,
                };
                bus_parts.extend_from_slice(&val.to_le_bytes());
            }
        }

        let mut tail = suffix_raw[bus_end..].to_vec();
        // Update Section 9 Preview/Monitor strip ID (0x458), total strip count (0x324), and track count (0x32F)
        let tail_updates: &[(u32, &[u32], Vec<u32>)] = &[
            (0x458, &[1], vec![track_len]),
            (0x324, &[2], vec![(track_len + bus_len).max(2)]),
            (0x32F, &[1], vec![track_len]),
        ];
        apply_single_field_updates(&mut tail, tail_updates);

        let mut out = Vec::with_capacity(bus_start + bus_parts.len() + tail.len());
        out.extend_from_slice(&suffix_raw[..bus_start]);
        out.extend_from_slice(&bus_parts);
        out.extend_from_slice(&tail);
        out
    } else {
        let mut out = suffix_raw.to_vec();
        let strip_updates: &[(u32, &[u32], Vec<u32>)] = &[
            (0x458, &[2, 1], vec![track_len + 1, track_len]),
            (0x324, &[2], vec![track_len.max(2)]),
            (0x32F, &[1], vec![track_len]),
        ];
        apply_single_field_updates(&mut out, strip_updates);
        out
    };

    // Keep unused mutable binding warning away
    let _ = &mut suffix;

    let mut result = Vec::with_capacity(track_start + track_parts.len() + suffix.len());
    result.extend_from_slice(&data[..track_start]);
    result.extend_from_slice(&track_parts);
    result.extend_from_slice(&suffix);
    if result.len() >= 12 {
        let body_len = (result.len() - 12) as u32;
        result[8..12].copy_from_slice(&body_len.to_le_bytes());
    }
    Ok(result)
}

fn apply_single_field_updates(buf: &mut [u8], updates: &[(u32, &[u32], Vec<u32>)]) {
    for (field, expected, new_vals) in updates {
        let mut pat = Vec::with_capacity(12);
        pat.extend_from_slice(&1u32.to_le_bytes());
        pat.extend_from_slice(&field.to_le_bytes());
        pat.extend_from_slice(&1u32.to_le_bytes());

        let mut offsets = Vec::new();
        let mut cursor = 0;
        while let Some(pos) = find_bytes(buf, &pat, cursor) {
            if pos + 16 <= buf.len() {
                offsets.push(pos + 12);
            }
            cursor = pos + 1;
        }
        let actual: Vec<u32> = offsets
            .iter()
            .map(|&o| u32::from_le_bytes(buf[o..o + 4].try_into().unwrap()))
            .collect();
        if actual == *expected {
            for (&o, &new_v) in offsets.iter().zip(new_vals.iter()) {
                buf[o..o + 4].copy_from_slice(&new_v.to_le_bytes());
            }
        }
    }
}
