use nle_core::{NleError, NleResult, Rational};
use std::collections::BTreeMap;

// ── AAF Stored Form Codes (AMWA AAF Stored Format v1.1) ─────────────────
pub const SF_DATA: u16 = 0x0082;
pub const SF_STRONG_OBJECT_REFERENCE: u16 = 0x0022;
pub const SF_STRONG_OBJECT_REFERENCE_VECTOR: u16 = 0x0032;
pub const SF_STRONG_OBJECT_REFERENCE_SET: u16 = 0x003A;
pub const SF_WEAK_OBJECT_REFERENCE: u16 = 0x0002;

// ── SMPTE ST 377-1 / AAF Static Property IDs (PIDs) ─────────────────────
pub const PID_ROOT_META_DICT: u16 = 0x0001;
pub const PID_ROOT_HEADER: u16 = 0x0002;

pub const PID_HEADER_BYTE_ORDER: u16 = 0x3b01;
pub const PID_HEADER_LAST_MODIFIED: u16 = 0x3b02;
pub const PID_HEADER_CONTENT: u16 = 0x3b03;
pub const PID_HEADER_DICTIONARY: u16 = 0x3b04;
pub const PID_HEADER_VERSION: u16 = 0x3b05;
pub const PID_HEADER_IDENT_LIST: u16 = 0x3b06;
pub const PID_HEADER_OBJ_MODEL_VER: u16 = 0x3b07;

pub const PID_IDENT_COMPANY_NAME: u16 = 0x3c01;
pub const PID_IDENT_PRODUCT_NAME: u16 = 0x3c02;
pub const PID_IDENT_VERSION_STR: u16 = 0x3c04;
pub const PID_IDENT_PRODUCT_ID: u16 = 0x3c05;
pub const PID_IDENT_DATE: u16 = 0x3c06;
pub const PID_IDENT_GENERATION_AUID: u16 = 0x3c09;

pub const PID_CONTENT_MOBS: u16 = 0x1901;

pub const PID_DICT_OPERATION_DEFS: u16 = 0x2603;
pub const PID_DICT_DATA_DEFS: u16 = 0x2605;

pub const PID_DEF_IDENTIFICATION: u16 = 0x1b01;
pub const PID_DEF_NAME: u16 = 0x1b02;
pub const PID_DEF_DESCRIPTION: u16 = 0x1b03;

pub const PID_MOB_ID: u16 = 0x4401;
pub const PID_MOB_NAME: u16 = 0x4402;
pub const PID_MOB_SLOTS: u16 = 0x4403;
pub const PID_MOB_LAST_MODIFIED: u16 = 0x4404;
pub const PID_MOB_CREATION_TIME: u16 = 0x4405;
pub const PID_MOB_USER_COMMENTS: u16 = 0x4406;
pub const PID_MOB_USAGE_CODE: u16 = 0x4408;

pub const PID_SOURCE_MOB_ESSENCE_DESC: u16 = 0x4701;

pub const PID_SLOT_ID: u16 = 0x4801;
pub const PID_SLOT_NAME: u16 = 0x4802;
pub const PID_SLOT_SEGMENT: u16 = 0x4803;
pub const PID_SLOT_PHYSICAL_TRACK: u16 = 0x4804;
pub const PID_TIMELINE_SLOT_EDIT_RATE: u16 = 0x4b01;
pub const PID_TIMELINE_SLOT_ORIGIN: u16 = 0x4b02;
pub const PID_EVENT_SLOT_EDIT_RATE: u16 = 0x4901;

pub const PID_COMP_DATA_DEF: u16 = 0x0201;
pub const PID_COMP_LENGTH: u16 = 0x0202;

pub const PID_SEQ_COMPONENTS: u16 = 0x1001;

pub const PID_SRC_REF_SOURCE_ID: u16 = 0x1101;
pub const PID_SRC_REF_SOURCE_SLOT_ID: u16 = 0x1102;
pub const PID_SRC_CLIP_START_TIME: u16 = 0x1201;

pub const PID_TIMECODE_START: u16 = 0x1501;
pub const PID_TIMECODE_FPS: u16 = 0x1502;
pub const PID_TIMECODE_DROP: u16 = 0x1503;

pub const PID_TRANSITION_CUT_POINT: u16 = 0x1701;
pub const PID_TRANSITION_OP_GROUP: u16 = 0x1702;

pub const PID_OP_GROUP_OPERATION: u16 = 0x0b01;
pub const PID_OP_GROUP_INPUT_SEGMENTS: u16 = 0x0b02;
pub const PID_OP_GROUP_PARAMETERS: u16 = 0x0b03;

pub const PID_NESTED_SCOPE_SLOTS: u16 = 0x0c01;
pub const PID_SELECTOR_SELECTED: u16 = 0x0e01;

pub const PID_EVENT_POSITION: u16 = 0x0601;
pub const PID_EVENT_COMMENT: u16 = 0x0602;
pub const PID_COMMENT_MARKER_COLOR: u16 = 0x0902;

pub const PID_PARAM_DEF: u16 = 0x4c01;
pub const PID_CONST_VAL_VALUE: u16 = 0x4d01;

pub const PID_DESC_LOCATOR: u16 = 0x2f01;
pub const PID_FILE_DESC_SAMPLE_RATE: u16 = 0x3001;
pub const PID_FILE_DESC_LENGTH: u16 = 0x3002;
pub const PID_IMAGE_DESC_STORED_HEIGHT: u16 = 0x3202;
pub const PID_IMAGE_DESC_STORED_WIDTH: u16 = 0x3203;
pub const PID_SOUND_DESC_AUDIO_SAMPLING_RATE: u16 = 0x3d03;
pub const PID_SOUND_DESC_CHANNEL_COUNT: u16 = 0x3d07;
pub const PID_MULTI_DESC_FILE_DESCS: u16 = 0x3f01;
pub const PID_NET_LOCATOR_URL: u16 = 0x4001;

pub const PID_TAGGED_VAL_NAME: u16 = 0x5001;
pub const PID_TAGGED_VAL_VALUE: u16 = 0x5003;

// ── SMPTE / AAF AUID / CLSID Builders ───────────────────────────────────

/// Encode a standard AAF AUID (Microsoft mixed-endian GUID layout used in CFB CLSIDs & AAF streams).
pub const fn auid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
    let b1 = d1.to_le_bytes();
    let b2 = d2.to_le_bytes();
    let b3 = d3.to_le_bytes();
    [
        b1[0], b1[1], b1[2], b1[3], b2[0], b2[1], b3[0], b3[1], d4[0], d4[1], d4[2], d4[3], d4[4],
        d4[5], d4[6], d4[7],
    ]
}

const SMPTE_CLASS_TAIL: [u8; 8] = [0x06, 0x0e, 0x2b, 0x34, 0x02, 0x06, 0x01, 0x01];

pub const fn smpte_class_clsid(code: u16) -> [u8; 16] {
    auid(0x0d01_0101, 0x0101, code, SMPTE_CLASS_TAIL)
}

/// Extract the 16-bit SMPTE class code (`0x3500`, `0x3600`, `0x1100`, etc.) from a standard AAF CLSID.
pub fn smpte_class_code(clsid: &[u8; 16]) -> Option<u16> {
    let d1 = u32::from_le_bytes([clsid[0], clsid[1], clsid[2], clsid[3]]);
    let d2 = u16::from_le_bytes([clsid[4], clsid[5]]);
    let d3 = u16::from_le_bytes([clsid[6], clsid[7]]);
    if d1 == 0x0d01_0101 && d2 == 0x0101 && clsid[8..16] == SMPTE_CLASS_TAIL {
        Some(d3)
    } else {
        None
    }
}

pub const CLSID_ROOT: [u8; 16] = auid(0xb3b3_98a5, 0x1c90, 0x11d4, [0x80, 0x53, 0x08, 0x00, 0x36, 0x21, 0x08, 0x04]);
pub const CLSID_HEADER: [u8; 16] = smpte_class_clsid(0x2f00);
pub const CLSID_IDENTIFICATION: [u8; 16] = smpte_class_clsid(0x3000);
pub const CLSID_CONTENT_STORAGE: [u8; 16] = smpte_class_clsid(0x1800);
pub const CLSID_DICTIONARY: [u8; 16] = smpte_class_clsid(0x2200);
pub const CLSID_DATA_DEFINITION: [u8; 16] = smpte_class_clsid(0x1b00);
pub const CLSID_OPERATION_DEFINITION: [u8; 16] = smpte_class_clsid(0x1c00);

pub const CLASS_COMPOSITION_MOB: u16 = 0x3500;
pub const CLASS_MASTER_MOB: u16 = 0x3600;
pub const CLASS_SOURCE_MOB: u16 = 0x3700;
pub const CLASS_EVENT_MOB_SLOT: u16 = 0x3900;
pub const CLASS_STATIC_MOB_SLOT: u16 = 0x3a00;
pub const CLASS_TIMELINE_MOB_SLOT: u16 = 0x3b00;
pub const CLASS_SEQUENCE: u16 = 0x0f00;
pub const CLASS_SOURCE_CLIP: u16 = 0x1100;
pub const CLASS_FILLER: u16 = 0x0900;
pub const CLASS_OPERATION_GROUP: u16 = 0x0a00;
pub const CLASS_NESTED_SCOPE: u16 = 0x0b00;
pub const CLASS_SELECTOR: u16 = 0x0e00;
pub const CLASS_TIMECODE: u16 = 0x1400;
pub const CLASS_TRANSITION: u16 = 0x1700;
pub const CLASS_COMMENT_MARKER: u16 = 0x0800;
pub const CLASS_DESCRIPTIVE_MARKER: u16 = 0x4100;
pub const CLASS_NETWORK_LOCATOR: u16 = 0x3200;
pub const CLASS_CDCI_DESCRIPTOR: u16 = 0x2800;
pub const CLASS_RGBA_DESCRIPTOR: u16 = 0x2900;
pub const CLASS_PCM_DESCRIPTOR: u16 = 0x4800;
pub const CLASS_WAVE_DESCRIPTOR: u16 = 0x2c00;
pub const CLASS_TAPE_DESCRIPTOR: u16 = 0x2e00;
pub const CLASS_IMPORT_DESCRIPTOR: u16 = 0x4a00;
pub const CLASS_MULTIPLE_DESCRIPTOR: u16 = 0x4400;
pub const CLASS_CONSTANT_VALUE: u16 = 0x3c00;
pub const CLASS_VARYING_VALUE: u16 = 0x3d00;
pub const CLASS_TAGGED_VALUE: u16 = 0x3f00;

// SMPTE DataDefinition AUIDs
pub const DATADEF_PICTURE: [u8; 16] =
    auid(0x0103_0202, 0x0100, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);
pub const DATADEF_SOUND: [u8; 16] =
    auid(0x0103_0202, 0x0200, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);
pub const DATADEF_TIMECODE: [u8; 16] =
    auid(0x0103_0201, 0x0100, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);
// Legacy AAF v1.0 DataDef AUIDs (exported by older Avid / Pro Tools)
pub const DATADEF_LEGACY_PICTURE: [u8; 16] =
    auid(0x80ef_0001, 0x0000, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);
pub const DATADEF_LEGACY_SOUND: [u8; 16] =
    auid(0x80ef_0002, 0x0000, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);
pub const DATADEF_LEGACY_TIMECODE: [u8; 16] =
    auid(0x80ef_0003, 0x0000, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);

// OperationDefinition AUIDs
pub const OPDEF_VIDEO_DISSOLVE: [u8; 16] =
    auid(0x0d01_0101, 0x0101, 0x0100, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);
pub const OPDEF_AUDIO_DISSOLVE: [u8; 16] =
    auid(0x0d01_0101, 0x0101, 0x0200, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);
pub const OPDEF_VIDEO_SPEED_CONTROL: [u8; 16] =
    auid(0x9d2e_a890, 0x0968, 0x11d3, [0x8a, 0x38, 0x00, 0x50, 0x04, 0x0e, 0xf7, 0xd2]);

// ParameterDef & TypeDef AUIDs
pub const PARAMDEF_SPEED_RATIO: [u8; 16] =
    auid(0x725a_06c1, 0x0968, 0x11d3, [0x8a, 0x38, 0x00, 0x50, 0x04, 0x0e, 0xf7, 0xd2]);
pub const TYPEDEF_RATIONAL: [u8; 16] =
    auid(0x0301_0100, 0x0000, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x01, 0x04, 0x01, 0x01]);
pub const TYPEDEF_STRING: [u8; 16] =
    auid(0x0110_0200, 0x0000, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x01, 0x04, 0x01, 0x01]);
pub const TYPEDEF_INT32: [u8; 16] =
    auid(0x0101_0700, 0x0000, 0x0000, [0x06, 0x0e, 0x2b, 0x34, 0x01, 0x04, 0x01, 0x01]);
pub const USAGE_TOP_LEVEL: [u8; 16] =
    auid(0x0d01_0201, 0x0101, 0x0100, [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01]);

// ── Property Stream Reader / Writer ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AafProperty {
    pub pid: u16,
    pub stored_form: u16,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PropertyMap {
    pub little_endian: bool,
    pub props: BTreeMap<u16, AafProperty>,
}

impl PropertyMap {
    pub fn parse(bytes: &[u8]) -> NleResult<Self> {
        if bytes.len() < 4 {
            return Err(NleError::InvalidFormat(
                "AAF properties stream truncated".into(),
            ));
        }
        let little_endian = match bytes[0] {
            0x4C => true,  // 'L'
            0x42 => false, // 'B'
            other => {
                return Err(NleError::InvalidFormat(format!(
                    "Invalid AAF property byte order marker 0x{other:02X}"
                )));
            }
        };
        let entry_count = read_u16(bytes, 2, little_endian) as usize;
        let header_end = 4 + entry_count * 6;
        if header_end > bytes.len() {
            return Err(NleError::InvalidFormat(
                "AAF properties index exceeds stream length".into(),
            ));
        }

        let mut props = BTreeMap::new();
        let mut val_off = header_end;
        for i in 0..entry_count {
            let e_off = 4 + i * 6;
            let pid = read_u16(bytes, e_off, little_endian);
            let stored_form = read_u16(bytes, e_off + 2, little_endian);
            let len = read_u16(bytes, e_off + 4, little_endian) as usize;
            let end = (val_off + len).min(bytes.len());
            let data = bytes[val_off..end].to_vec();
            val_off = end;
            props.insert(
                pid,
                AafProperty {
                    pid,
                    stored_form,
                    data,
                },
            );
        }

        Ok(Self {
            little_endian,
            props,
        })
    }

    pub fn get(&self, pid: u16) -> Option<&AafProperty> {
        self.props.get(&pid)
    }

    pub fn get_string(&self, pid: u16) -> Option<String> {
        let p = self.props.get(&pid)?;
        Some(decode_utf16(&p.data, self.little_endian))
    }

    pub fn get_u8(&self, pid: u16) -> Option<u8> {
        self.props.get(&pid)?.data.first().copied()
    }

    pub fn get_u16(&self, pid: u16) -> Option<u16> {
        let d = &self.props.get(&pid)?.data;
        (d.len() >= 2).then(|| read_u16(d, 0, self.little_endian))
    }

    pub fn get_u32(&self, pid: u16) -> Option<u32> {
        let d = &self.props.get(&pid)?.data;
        (d.len() >= 4).then(|| read_u32(d, 0, self.little_endian))
    }

    pub fn get_i64(&self, pid: u16) -> Option<i64> {
        let d = &self.props.get(&pid)?.data;
        (d.len() >= 8).then(|| read_i64(d, 0, self.little_endian))
    }

    pub fn get_rational(&self, pid: u16) -> Option<Rational> {
        let d = &self.props.get(&pid)?.data;
        if d.len() < 8 {
            return None;
        }
        let num = read_i32(d, 0, self.little_endian) as i64;
        let den = read_i32(d, 4, self.little_endian) as i64;
        if den == 0 {
            return None;
        }
        Some(Rational::new(num, den))
    }

    pub fn get_auid(&self, pid: u16) -> Option<[u8; 16]> {
        let p = self.props.get(&pid)?;
        if p.stored_form == SF_WEAK_OBJECT_REFERENCE {
            // WeakObjectReference layout: table_idx (u16), key_pid (u16), key_size (u8), key_bytes
            if p.data.len() >= 5 {
                let key_size = p.data[4] as usize;
                if key_size == 16 && p.data.len() >= 21 {
                    let mut out = [0u8; 16];
                    out.copy_from_slice(&p.data[5..21]);
                    return Some(out);
                }
            }
            return None;
        }
        if p.data.len() >= 16 {
            let mut out = [0u8; 16];
            out.copy_from_slice(&p.data[0..16]);
            return Some(out);
        }
        None
    }

    pub fn get_mob_id(&self, pid: u16) -> Option<[u8; 32]> {
        let d = &self.props.get(&pid)?.data;
        if d.len() < 32 {
            return None;
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&d[0..32]);
        Some(out)
    }

    pub fn get_ref_name(&self, pid: u16) -> Option<String> {
        let p = self.props.get(&pid)?;
        if matches!(
            p.stored_form,
            SF_STRONG_OBJECT_REFERENCE
                | SF_STRONG_OBJECT_REFERENCE_VECTOR
                | SF_STRONG_OBJECT_REFERENCE_SET
        ) {
            let s = decode_utf16(&p.data, self.little_endian);
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }
}

/// Builder for little-endian AAF v1.1 `properties` stream.
#[derive(Default)]
pub struct PropertyBuilder {
    entries: Vec<(u16, u16, Vec<u8>)>,
}

impl PropertyBuilder {
    pub fn add_raw(&mut self, pid: u16, stored_form: u16, data: Vec<u8>) {
        self.entries.push((pid, stored_form, data));
    }

    pub fn add_data(&mut self, pid: u16, data: Vec<u8>) {
        self.add_raw(pid, SF_DATA, data);
    }

    pub fn add_u8(&mut self, pid: u16, v: u8) {
        self.add_data(pid, vec![v]);
    }

    pub fn add_u16(&mut self, pid: u16, v: u16) {
        self.add_data(pid, v.to_le_bytes().to_vec());
    }

    pub fn add_u32(&mut self, pid: u16, v: u32) {
        self.add_data(pid, v.to_le_bytes().to_vec());
    }

    pub fn add_i64(&mut self, pid: u16, v: i64) {
        self.add_data(pid, v.to_le_bytes().to_vec());
    }

    pub fn add_rational(&mut self, pid: u16, r: Rational) {
        let mut buf = Vec::with_capacity(8);
        buf.extend_from_slice(&(r.num as i32).to_le_bytes());
        buf.extend_from_slice(&(r.den as i32).to_le_bytes());
        self.add_data(pid, buf);
    }

    pub fn add_string(&mut self, pid: u16, s: &str) {
        self.add_data(pid, encode_utf16le_nul(s));
    }

    pub fn add_strong_ref(&mut self, pid: u16, child_name: &str) {
        self.add_raw(pid, SF_STRONG_OBJECT_REFERENCE, encode_utf16le_nul(child_name));
    }

    pub fn add_strong_ref_vector(&mut self, pid: u16, base_name: &str) {
        self.add_raw(
            pid,
            SF_STRONG_OBJECT_REFERENCE_VECTOR,
            encode_utf16le_nul(base_name),
        );
    }

    pub fn add_strong_ref_set(&mut self, pid: u16, base_name: &str) {
        self.add_raw(
            pid,
            SF_STRONG_OBJECT_REFERENCE_SET,
            encode_utf16le_nul(base_name),
        );
    }

    pub fn add_weak_ref_auid(&mut self, pid: u16, table_idx: u16, key_pid: u16, target: &[u8; 16]) {
        let mut buf = Vec::with_capacity(21);
        buf.extend_from_slice(&table_idx.to_le_bytes());
        buf.extend_from_slice(&key_pid.to_le_bytes());
        buf.push(16u8);
        buf.extend_from_slice(target);
        self.add_raw(pid, SF_WEAK_OBJECT_REFERENCE, buf);
    }

    pub fn finish(self) -> Vec<u8> {
        let count = self.entries.len() as u16;
        let payload_size: usize = self.entries.iter().map(|(_, _, d)| d.len()).sum();
        let mut out = Vec::with_capacity(4 + self.entries.len() * 6 + payload_size);
        out.push(0x4C); // 'L' little-endian
        out.push(0x20); // Format version 32 (AAF v1.1)
        out.extend_from_slice(&count.to_le_bytes());
        for (pid, sf, data) in &self.entries {
            out.extend_from_slice(&pid.to_le_bytes());
            out.extend_from_slice(&sf.to_le_bytes());
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        }
        for (_, _, data) in &self.entries {
            out.extend_from_slice(data);
        }
        out
    }
}

// ── Vector & Set Index Stream Codecs ────────────────────────────────────

pub fn parse_strong_ref_vector_index(bytes: &[u8], le: bool) -> Vec<u32> {
    if bytes.len() < 12 {
        return Vec::new();
    }
    let count = read_u32(bytes, 0, le) as usize;
    let mut keys = Vec::with_capacity(count.min(bytes.len() / 4));
    for i in 0..count {
        let off = 12 + i * 4;
        if off + 4 > bytes.len() {
            break;
        }
        keys.push(read_u32(bytes, off, le));
    }
    keys
}

pub fn encode_strong_ref_vector_index(count: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + (count as usize) * 4);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes()); // first_free_key
    out.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // last_free_key
    for k in 0..count {
        out.extend_from_slice(&k.to_le_bytes());
    }
    out
}

pub fn parse_strong_ref_set_index(bytes: &[u8], le: bool) -> Vec<u32> {
    if bytes.len() < 11 {
        return Vec::new();
    }
    let count = read_u32(bytes, 0, le) as usize;
    let key_size = bytes[10] as usize;
    let stride = 8 + key_size;
    let mut keys = Vec::with_capacity(count.min(1024));
    for i in 0..count {
        let off = 11 + i * stride;
        if off + 4 > bytes.len() {
            break;
        }
        keys.push(read_u32(bytes, off, le));
    }
    keys
}

pub fn encode_strong_ref_set_index(key_pid: u16, keys: &[Vec<u8>]) -> Vec<u8> {
    let key_size = keys.first().map(|k| k.len() as u8).unwrap_or(16);
    let count = keys.len() as u32;
    let mut out = Vec::with_capacity(11 + keys.len() * (8 + key_size as usize));
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes()); // next_free_key
    out.extend_from_slice(&key_pid.to_le_bytes());
    out.push(key_size);
    for (idx, kbytes) in keys.iter().enumerate() {
        out.extend_from_slice(&(idx as u32).to_le_bytes()); // local_key
        out.extend_from_slice(&1u32.to_le_bytes()); // ref_count
        out.extend_from_slice(kbytes);
    }
    out
}

/// Encode `/referenced properties` root stream describing weak-ref target property paths.
pub fn encode_referenced_properties(paths: &[&[u16]]) -> Vec<u8> {
    let entry_count = paths.len() as u32;
    let total_pids: u32 = paths.iter().map(|p| (p.len() + 1) as u32).sum();
    let mut out = Vec::with_capacity(10 + (total_pids as usize) * 2);
    out.push(0x4C); // 'L' little-endian
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&total_pids.to_le_bytes());
    for path in paths {
        for &pid in *path {
            out.extend_from_slice(&pid.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

// ── IndirectValue, UTF-16, and Scalar Helpers ───────────────────────────

pub fn encode_indirect_rational(r: Rational) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 16 + 8);
    buf.push(0x4C); // 'L'
    buf.extend_from_slice(&TYPEDEF_RATIONAL);
    buf.extend_from_slice(&(r.num as i32).to_le_bytes());
    buf.extend_from_slice(&(r.den as i32).to_le_bytes());
    buf
}

pub fn decode_indirect_rational(bytes: &[u8]) -> Option<Rational> {
    if bytes.len() < 17 {
        return None;
    }
    let le = bytes[0] != 0x42;
    let payload = &bytes[17..];
    if payload.len() >= 8 {
        let num = read_i32(payload, 0, le) as i64;
        let den = read_i32(payload, 4, le) as i64;
        if den != 0 {
            return Some(Rational::new(num, den));
        }
    }
    None
}

pub fn decode_indirect_string(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 17 {
        return None;
    }
    let le = bytes[0] != 0x42;
    if bytes[1..17] == TYPEDEF_STRING {
        return Some(decode_utf16(&bytes[17..], le));
    }
    None
}

pub fn decode_indirect_f64_or_rational(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 17 {
        return None;
    }
    let le = bytes[0] != 0x42;
    let type_auid = &bytes[1..17];
    let payload = &bytes[17..];
    if type_auid == TYPEDEF_RATIONAL && payload.len() >= 8 {
        let num = read_i32(payload, 0, le) as f64;
        let den = read_i32(payload, 4, le) as f64;
        if den.abs() > 1e-9 {
            return Some(num / den);
        }
    } else if type_auid == TYPEDEF_INT32 && payload.len() >= 4 {
        return Some(read_i32(payload, 0, le) as f64);
    } else if payload.len() == 8 {
        let num = read_i32(payload, 0, le) as f64;
        let den = read_i32(payload, 4, le) as f64;
        if den.abs() > 1e-9 && num.abs() < 1_000_000.0 && den.abs() < 1_000_000.0 {
            return Some(num / den);
        }
    }
    None
}

pub fn encode_utf16le_nul(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity((s.len() + 1) * 2);
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

pub fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let mut u16s = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.as_chunks::<2>().0 {
        let v = if little_endian {
            u16::from_le_bytes(*chunk)
        } else {
            u16::from_be_bytes(*chunk)
        };
        if v == 0 {
            break;
        }
        u16s.push(v);
    }
    String::from_utf16_lossy(&u16s)
}

pub fn generate_mob_id() -> [u8; 32] {
    let mut mob_id = [0u8; 32];
    // Standard SMPTE 330M Basic UMID prefix
    mob_id[0..12].copy_from_slice(&[
        0x06, 0x0a, 0x2b, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0f, 0x20,
    ]);
    mob_id[12] = 0x13;
    let u = uuid::Uuid::new_v4();
    mob_id[16..32].copy_from_slice(u.as_bytes());
    mob_id
}

pub fn is_zero_mob_id(id: &[u8; 32]) -> bool {
    id.iter().all(|&b| b == 0)
}

pub fn read_u16(buf: &[u8], off: usize, le: bool) -> u16 {
    if le {
        u16::from_le_bytes([buf[off], buf[off + 1]])
    } else {
        u16::from_be_bytes([buf[off], buf[off + 1]])
    }
}

pub fn read_u32(buf: &[u8], off: usize, le: bool) -> u32 {
    if le {
        u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
    } else {
        u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
    }
}

pub fn read_i32(buf: &[u8], off: usize, le: bool) -> i32 {
    read_u32(buf, off, le) as i32
}

pub fn read_i64(buf: &[u8], off: usize, le: bool) -> i64 {
    if le {
        i64::from_le_bytes([
            buf[off],
            buf[off + 1],
            buf[off + 2],
            buf[off + 3],
            buf[off + 4],
            buf[off + 5],
            buf[off + 6],
            buf[off + 7],
        ])
    } else {
        i64::from_be_bytes([
            buf[off],
            buf[off + 1],
            buf[off + 2],
            buf[off + 3],
            buf[off + 4],
            buf[off + 5],
            buf[off + 6],
            buf[off + 7],
        ])
    }
}
