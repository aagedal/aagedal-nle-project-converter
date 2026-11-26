use std::collections::{BTreeMap, HashSet};

/// Decoded Apple binary plist (`bplist00`) / `NSKeyedArchiver` value.
#[derive(Debug, Clone)]
pub enum PlistValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Data(Vec<u8>),
    String(String),
    Uid(usize),
    Array(Vec<PlistValue>),
    Dict(BTreeMap<String, PlistValue>),
}

impl PlistValue {
    pub fn as_dict(&self) -> Option<&BTreeMap<String, PlistValue>> {
        match self {
            PlistValue::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[PlistValue]> {
        match self {
            PlistValue::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            PlistValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            PlistValue::Int(i) => Some(*i),
            PlistValue::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PlistValue::Float(f) => Some(*f),
            PlistValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            PlistValue::Bool(b) => Some(*b),
            PlistValue::Int(i) => Some(*i != 0),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            PlistValue::Data(d) => Some(d.as_slice()),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&PlistValue> {
        self.as_dict()?.get(key)
    }
}

/// Parse an Apple binary plist (`bplist00`) and automatically dereference `NSKeyedArchiver`
/// object tables (`$archiver == "NSKeyedArchiver"`) into plain `PlistValue` trees.
pub fn parse_bplist_nskeyed(data: &[u8]) -> Option<PlistValue> {
    let raw = parse_bplist(data)?;
    if let Some(dict) = raw.as_dict() {
        let is_nska = dict
            .get("$archiver")
            .and_then(PlistValue::as_str)
            .map(|s| s == "NSKeyedArchiver")
            .unwrap_or(false);
        if is_nska {
            let objects = dict.get("$objects")?.as_array()?;
            let top = dict.get("$top")?.as_dict()?;
            let root_ref = top.get("root").or_else(|| top.values().next())?;
            let mut seen = HashSet::new();
            return Some(resolve_nska(root_ref, objects, &mut seen, 0));
        }
    }
    Some(raw)
}

fn resolve_nska(
    val: &PlistValue,
    objects: &[PlistValue],
    seen: &mut HashSet<usize>,
    depth: usize,
) -> PlistValue {
    if depth > 24 {
        return PlistValue::Null;
    }
    match val {
        PlistValue::Uid(idx) => {
            if *idx >= objects.len() || !seen.insert(*idx) {
                return PlistValue::Null;
            }
            let resolved = resolve_nska(&objects[*idx], objects, seen, depth + 1);
            seen.remove(idx);
            resolved
        }
        PlistValue::String(s) if s == "$null" => PlistValue::Null,
        PlistValue::Array(items) => PlistValue::Array(
            items
                .iter()
                .map(|it| resolve_nska(it, objects, seen, depth + 1))
                .collect(),
        ),
        PlistValue::Dict(d) => {
            // NSDictionary / NSMutableDictionary encoded as NS.keys + NS.objects
            if let (Some(keys), Some(vals)) = (
                d.get("NS.keys").and_then(PlistValue::as_array),
                d.get("NS.objects").and_then(PlistValue::as_array),
            ) {
                let mut out = BTreeMap::new();
                for (k, v) in keys.iter().zip(vals.iter()) {
                    let rk = resolve_nska(k, objects, seen, depth + 1);
                    let rv = resolve_nska(v, objects, seen, depth + 1);
                    if let Some(ks) = rk.as_str() {
                        out.insert(ks.to_string(), rv);
                    }
                }
                return PlistValue::Dict(out);
            }
            // NSArray / NSSet encoded as NS.objects
            if let Some(objs) = d.get("NS.objects").and_then(PlistValue::as_array) {
                return PlistValue::Array(
                    objs.iter()
                        .map(|it| resolve_nska(it, objects, seen, depth + 1))
                        .collect(),
                );
            }
            if let Some(s) = d.get("NS.string") {
                return resolve_nska(s, objects, seen, depth + 1);
            }
            if let Some(b) = d.get("NS.data") {
                return resolve_nska(b, objects, seen, depth + 1);
            }
            // NSNull
            if let Some(cls) = d.get("$class") {
                let rcls = resolve_nska(cls, objects, seen, depth + 1);
                if rcls.get("$classname").and_then(PlistValue::as_str) == Some("NSNull") {
                    return PlistValue::Null;
                }
            }
            let mut out = BTreeMap::new();
            for (k, v) in d {
                if k == "$class" {
                    continue;
                }
                out.insert(k.clone(), resolve_nska(v, objects, seen, depth + 1));
            }
            PlistValue::Dict(out)
        }
        other => other.clone(),
    }
}

/// Parse a raw `bplist00` buffer without NSKeyedArchiver resolution.
pub fn parse_bplist(data: &[u8]) -> Option<PlistValue> {
    if data.len() < 40 || &data[0..8] != b"bplist00" {
        return None;
    }
    let trailer = &data[data.len() - 32..];
    let offset_int_size = trailer[6] as usize;
    let object_ref_size = trailer[7] as usize;
    let num_objects = u64::from_be_bytes(trailer[8..16].try_into().ok()?) as usize;
    let top_object = u64::from_be_bytes(trailer[16..24].try_into().ok()?) as usize;
    let offset_table_offset = u64::from_be_bytes(trailer[24..32].try_into().ok()?) as usize;

    if offset_int_size == 0
        || offset_int_size > 8
        || object_ref_size == 0
        || object_ref_size > 8
        || top_object >= num_objects
    {
        return None;
    }
    let table_end = offset_table_offset.checked_add(num_objects.checked_mul(offset_int_size)?)?;
    if table_end > data.len() - 32 {
        return None;
    }

    let mut offsets = Vec::with_capacity(num_objects);
    for i in 0..num_objects {
        let start = offset_table_offset + i * offset_int_size;
        let off = read_be_uint(&data[start..start + offset_int_size]) as usize;
        offsets.push(off);
    }

    let mut visiting = HashSet::new();
    parse_object(data, &offsets, object_ref_size, top_object, &mut visiting)
}

fn parse_object(
    data: &[u8],
    offsets: &[usize],
    ref_size: usize,
    idx: usize,
    visiting: &mut HashSet<usize>,
) -> Option<PlistValue> {
    let off = *offsets.get(idx)?;
    if off >= data.len() || !visiting.insert(idx) {
        return None;
    }
    let res = parse_object_at(data, offsets, ref_size, off, visiting);
    visiting.remove(&idx);
    res
}

fn parse_object_at(
    data: &[u8],
    offsets: &[usize],
    ref_size: usize,
    off: usize,
    visiting: &mut HashSet<usize>,
) -> Option<PlistValue> {
    let marker = *data.get(off)?;
    let kind = marker >> 4;
    let info = marker & 0x0f;

    match kind {
        0x0 => match info {
            0x0 | 0xf => Some(PlistValue::Null),
            0x8 => Some(PlistValue::Bool(false)),
            0x9 => Some(PlistValue::Bool(true)),
            _ => Some(PlistValue::Null),
        },
        // Integer
        0x1 => {
            let nbytes = 1usize.checked_shl(info as u32)?;
            let slice = data.get(off + 1..off + 1 + nbytes)?;
            let val = match nbytes {
                1 => slice[0] as i64,
                2 => u16::from_be_bytes([slice[0], slice[1]]) as i64,
                4 => u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64,
                8 => i64::from_be_bytes(slice.try_into().ok()?),
                16 => i128::from_be_bytes(slice.try_into().ok()?) as i64,
                _ => read_be_uint(slice) as i64,
            };
            Some(PlistValue::Int(val))
        }
        // Real
        0x2 => {
            let nbytes = 1usize.checked_shl(info as u32)?;
            let slice = data.get(off + 1..off + 1 + nbytes)?;
            let val = match nbytes {
                4 => f32::from_be_bytes(slice.try_into().ok()?) as f64,
                8 => f64::from_be_bytes(slice.try_into().ok()?),
                _ => return None,
            };
            Some(PlistValue::Float(val))
        }
        // Date
        0x3 => {
            let slice = data.get(off + 1..off + 9)?;
            let secs = f64::from_be_bytes(slice.try_into().ok()?);
            Some(PlistValue::Float(secs))
        }
        // Data
        0x4 => {
            let (len, start) = read_length(data, off, info)?;
            let slice = data.get(start..start.checked_add(len)?)?;
            Some(PlistValue::Data(slice.to_vec()))
        }
        // ASCII string
        0x5 => {
            let (len, start) = read_length(data, off, info)?;
            let slice = data.get(start..start.checked_add(len)?)?;
            Some(PlistValue::String(
                String::from_utf8_lossy(slice).into_owned(),
            ))
        }
        // UTF-16BE string
        0x6 => {
            let (units, start) = read_length(data, off, info)?;
            let byte_len = units.checked_mul(2)?;
            let slice = data.get(start..start.checked_add(byte_len)?)?;
            let code_units: Vec<u16> = slice
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(PlistValue::String(String::from_utf16_lossy(&code_units)))
        }
        // UID
        0x8 => {
            let nbytes = (info as usize) + 1;
            let slice = data.get(off + 1..off + 1 + nbytes)?;
            Some(PlistValue::Uid(read_be_uint(slice) as usize))
        }
        // Array or Set
        0xA | 0xC => {
            let (count, start) = read_length(data, off, info)?;
            let mut items = Vec::with_capacity(count.min(4096));
            for i in 0..count {
                let r_off = start + i * ref_size;
                let r_idx = read_be_uint(data.get(r_off..r_off + ref_size)?) as usize;
                items.push(
                    parse_object(data, offsets, ref_size, r_idx, visiting)
                        .unwrap_or(PlistValue::Null),
                );
            }
            Some(PlistValue::Array(items))
        }
        // Dict
        0xD => {
            let (count, start) = read_length(data, off, info)?;
            let vals_start = start.checked_add(count.checked_mul(ref_size)?)?;
            let mut map = BTreeMap::new();
            for i in 0..count {
                let k_off = start + i * ref_size;
                let v_off = vals_start + i * ref_size;
                let k_idx = read_be_uint(data.get(k_off..k_off + ref_size)?) as usize;
                let v_idx = read_be_uint(data.get(v_off..v_off + ref_size)?) as usize;
                let k_val = parse_object(data, offsets, ref_size, k_idx, visiting)
                    .unwrap_or(PlistValue::Null);
                let v_val = parse_object(data, offsets, ref_size, v_idx, visiting)
                    .unwrap_or(PlistValue::Null);
                if let Some(ks) = k_val.as_str() {
                    map.insert(ks.to_string(), v_val);
                }
            }
            Some(PlistValue::Dict(map))
        }
        _ => Some(PlistValue::Null),
    }
}

fn read_length(data: &[u8], off: usize, info: u8) -> Option<(usize, usize)> {
    if info < 0x0f {
        return Some((info as usize, off + 1));
    }
    let int_marker = *data.get(off + 1)?;
    if (int_marker >> 4) != 0x1 {
        return None;
    }
    let nbytes = 1usize.checked_shl((int_marker & 0x0f) as u32)?;
    let slice = data.get(off + 2..off + 2 + nbytes)?;
    let len = read_be_uint(slice) as usize;
    Some((len, off + 2 + nbytes))
}

fn read_be_uint(bytes: &[u8]) -> u64 {
    let mut v = 0u64;
    for &b in bytes {
        v = (v << 8) | (b as u64);
    }
    v
}
