use nle_core::{NleError, NleResult};
use std::collections::BTreeMap;
use std::io::Read;

/// Encode bytes as lowercase hex string.
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

/// Decode a hex string into bytes.
pub fn hex_decode(s: &str) -> NleResult<Vec<u8>> {
    let trimmed = s.trim();
    if trimmed.len() % 2 != 0 {
        return Err(NleError::InvalidFormat("Odd-length hex string in Resolve blob".into()));
    }
    let mut out = Vec::with_capacity(trimmed.len() / 2);
    let bytes = trimmed.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = from_hex_digit(bytes[i])?;
        let lo = from_hex_digit(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn from_hex_digit(b: u8) -> NleResult<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(NleError::InvalidFormat(format!("Invalid hex char: {}", b as char))),
    }
}

pub fn encode_utf16be(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out
}

pub fn encode_utf16le(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

pub fn decode_utf16be(bytes: &[u8]) -> NleResult<String> {
    if bytes.len() % 2 != 0 {
        return Err(NleError::InvalidFormat("Odd-length UTF-16BE slice".into()));
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units)
        .map_err(|e| NleError::InvalidFormat(format!("Invalid UTF-16BE string: {e}")))
}

// ── Protobuf wire-level helpers ────────────────────────────────────────

pub fn read_varint(data: &[u8], pos: &mut usize) -> NleResult<u64> {
    let mut val = 0u64;
    let mut shift = 0u32;
    loop {
        if *pos >= data.len() {
            return Err(NleError::InvalidFormat("Truncated protobuf varint".into()));
        }
        let b = data[*pos];
        *pos += 1;
        val |= ((b & 0x7F) as u64) << shift;
        if b < 0x80 {
            return Ok(val);
        }
        shift += 7;
        if shift >= 70 {
            return Err(NleError::InvalidFormat("Oversized protobuf varint".into()));
        }
    }
}

pub fn write_varint(mut val: u64, out: &mut Vec<u8>) {
    while val >= 0x80 {
        out.push(((val as u8) & 0x7F) | 0x80);
        val >>= 7;
    }
    out.push(val as u8);
}

#[derive(Debug, Clone)]
pub enum ProtoValue {
    Varint(u64),
    Fixed64([u8; 8]),
    Bytes(Vec<u8>),
    Fixed32([u8; 4]),
}

#[derive(Debug, Clone)]
pub struct ProtoField {
    pub number: u32,
    pub value: ProtoValue,
}

impl ProtoField {
    pub fn varint(number: u32, v: u64) -> Self {
        Self { number, value: ProtoValue::Varint(v) }
    }
    pub fn bytes(number: u32, b: Vec<u8>) -> Self {
        Self { number, value: ProtoValue::Bytes(b) }
    }
    pub fn f64_le(number: u32, v: f64) -> Self {
        Self { number, value: ProtoValue::Fixed64(v.to_le_bytes()) }
    }
    pub fn f32_le(number: u32, v: f32) -> Self {
        Self { number, value: ProtoValue::Fixed32(v.to_le_bytes()) }
    }
}

pub fn decode_protobuf(data: &[u8]) -> NleResult<Vec<ProtoField>> {
    let mut fields = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let key = read_varint(data, &mut pos)?;
        let number = (key >> 3) as u32;
        let wire = (key & 7) as u8;
        let value = match wire {
            0 => ProtoValue::Varint(read_varint(data, &mut pos)?),
            1 => {
                if pos + 8 > data.len() {
                    return Err(NleError::InvalidFormat("Truncated protobuf fixed64".into()));
                }
                let mut arr = [0u8; 8];
                arr.copy_from_slice(&data[pos..pos + 8]);
                pos += 8;
                ProtoValue::Fixed64(arr)
            }
            2 => {
                let len = read_varint(data, &mut pos)? as usize;
                if pos + len > data.len() {
                    return Err(NleError::InvalidFormat("Truncated protobuf length-delimited field".into()));
                }
                let bytes = data[pos..pos + len].to_vec();
                pos += len;
                ProtoValue::Bytes(bytes)
            }
            5 => {
                if pos + 4 > data.len() {
                    return Err(NleError::InvalidFormat("Truncated protobuf fixed32".into()));
                }
                let mut arr = [0u8; 4];
                arr.copy_from_slice(&data[pos..pos + 4]);
                pos += 4;
                ProtoValue::Fixed32(arr)
            }
            _ => return Err(NleError::InvalidFormat(format!("Unsupported protobuf wire type {wire}"))),
        };
        fields.push(ProtoField { number, value });
    }
    Ok(fields)
}

pub fn encode_protobuf(fields: &[ProtoField]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in fields {
        let wire: u64 = match &f.value {
            ProtoValue::Varint(_) => 0,
            ProtoValue::Fixed64(_) => 1,
            ProtoValue::Bytes(_) => 2,
            ProtoValue::Fixed32(_) => 5,
        };
        write_varint(((f.number as u64) << 3) | wire, &mut out);
        match &f.value {
            ProtoValue::Varint(v) => write_varint(*v, &mut out),
            ProtoValue::Fixed64(arr) => out.extend_from_slice(arr),
            ProtoValue::Bytes(b) => {
                write_varint(b.len() as u64, &mut out);
                out.extend_from_slice(b);
            }
            ProtoValue::Fixed32(arr) => out.extend_from_slice(arr),
        }
    }
    out
}

/// Unwrap Resolve's 9-byte envelope (`[version: u32be][len: u32be][flag][payload...]`).
/// Supports both `0x80` (uncompressed protobuf) and `0x81` (Resolve 21.2 zstd-compressed protobuf).
pub fn decode_wrapper_hex(hex: &str) -> NleResult<Vec<u8>> {
    let raw = hex_decode(hex)?;
    if raw.len() < 9 {
        return Err(NleError::InvalidFormat("Resolve blob envelope too short".into()));
    }
    match raw[8] {
        0x80 => Ok(raw[9..].to_vec()),
        0x81 => {
            let mut decoder = ruzstd::StreamingDecoder::new(&raw[9..])
                .map_err(|e| NleError::Archive(format!("Resolve 0x81 zstd init failed: {e}")))?;
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|e| NleError::Archive(format!("Resolve 0x81 zstd decode failed: {e}")))?;
            Ok(out)
        }
        flag => Err(NleError::InvalidFormat(format!(
            "Unsupported Resolve blob envelope flag 0x{flag:02x}"
        ))),
    }
}

/// Wrap raw protobuf bytes into Resolve's version-2 `0x80` hex envelope.
pub fn encode_wrapper_hex(payload: &[u8]) -> String {
    let mut buf = Vec::with_capacity(9 + payload.len());
    buf.extend_from_slice(&2u32.to_be_bytes());
    buf.extend_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
    buf.push(0x80);
    buf.extend_from_slice(payload);
    hex_encode(&buf)
}

// ── Resolve Keyed-Dict (FieldsBlob / TracksBA / Sm2TimeMap) ────────────

pub type KeyedFields = BTreeMap<String, (u32, Vec<u8>)>;

/// Decode Resolve's version-1 keyed-dict binary format (`[1: u32be][count: u32be] ...`).
pub fn decode_keyed_fields(data: &[u8]) -> NleResult<Vec<(String, u32, Vec<u8>)>> {
    if data.len() < 8 {
        return Err(NleError::InvalidFormat("Keyed fields blob too short".into()));
    }
    let version = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    if version != 1 {
        return Err(NleError::InvalidFormat(format!("Unsupported keyed fields version {version}")));
    }
    let count = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let mut offset = 8usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if offset + 4 > data.len() {
            return Err(NleError::InvalidFormat("Truncated keyed field key length".into()));
        }
        let key_len = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if key_len % 2 != 0 || offset + key_len + 5 > data.len() {
            return Err(NleError::InvalidFormat("Invalid keyed field key bounds".into()));
        }
        let key = decode_utf16be(&data[offset..offset + key_len])?;
        offset += key_len;
        let kind = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
        let is_null = data[offset + 4];
        offset += 5;
        if is_null != 0 {
            return Err(NleError::InvalidFormat("Unsupported null keyed field".into()));
        }
        let val_len = match kind {
            1 => 1,
            2 | 3 => 4,
            4 | 6 => 8,
            10 | 12 => {
                if offset + 4 > data.len() {
                    return Err(NleError::InvalidFormat("Truncated keyed field value length".into()));
                }
                let l = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
                offset += 4;
                l
            }
            _ => return Err(NleError::InvalidFormat(format!("Unsupported keyed field type {kind}"))),
        };
        if offset + val_len > data.len() {
            return Err(NleError::InvalidFormat("Invalid keyed field value bounds".into()));
        }
        entries.push((key, kind, data[offset..offset + val_len].to_vec()));
        offset += val_len;
    }
    Ok(entries)
}

/// Encode Resolve's version-1 keyed-dict binary format preserving insertion order.
pub fn encode_keyed_fields(entries: &[(String, u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (key, kind, value) in entries {
        let key_bytes = encode_utf16be(key);
        out.extend_from_slice(&(key_bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&key_bytes);
        out.extend_from_slice(&kind.to_be_bytes());
        out.push(0u8);
        if *kind == 10 || *kind == 12 {
            out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        }
        out.extend_from_slice(value);
    }
    out
}

pub fn replace_subslice(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    if needle.is_empty() {
        return haystack.to_vec();
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if i + needle.len() <= haystack.len() && &haystack[i..i + needle.len()] == needle {
            out.extend_from_slice(replacement);
            i += needle.len();
        } else {
            out.push(haystack[i]);
            i += 1;
        }
    }
    out
}
