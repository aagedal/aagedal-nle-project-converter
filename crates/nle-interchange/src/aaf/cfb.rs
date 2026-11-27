use nle_core::{NleError, NleResult};
use std::cmp::Ordering;
use std::collections::HashSet;

pub const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const FREESECT: u32 = 0xFFFF_FFFF;
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FATSECT: u32 = 0xFFFF_FFFD;
const DIFSECT: u32 = 0xFFFF_FFFC;
const NOSTREAM: u32 = 0xFFFF_FFFF;
const MINI_STREAM_CUTOFF: u32 = 4096;
const MINI_SECTOR_SIZE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfbObjType {
    Unknown,
    Storage,
    Stream,
    Root,
}

#[derive(Debug, Clone)]
pub struct CfbEntry {
    pub id: usize,
    pub name: String,
    pub obj_type: CfbObjType,
    pub clsid: [u8; 16],
    pub left_sibling: Option<usize>,
    pub right_sibling: Option<usize>,
    pub child: Option<usize>,
    pub start_sector: u32,
    pub stream_size: u64,
}

/// Pure-Rust Microsoft Compound File Binary Format (OLE2 / Structured Storage) reader
/// supporting both Version 3 (512-byte sectors) and Version 4 (4096-byte sectors).
pub struct CfbFile<'a> {
    data: &'a [u8],
    sector_size: usize,
    mini_sector_size: usize,
    mini_stream_cutoff: u64,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    mini_stream: Vec<u8>,
    entries: Vec<CfbEntry>,
}

impl<'a> CfbFile<'a> {
    pub fn parse(data: &'a [u8]) -> NleResult<Self> {
        if data.len() < 512 || data[0..8] != CFB_MAGIC {
            return Err(NleError::InvalidFormat(
                "Not a valid Compound File Binary (OLE2 / AAF) header".into(),
            ));
        }

        let major_ver = u16::from_le_bytes([data[26], data[27]]);
        let sector_shift = u16::from_le_bytes([data[30], data[31]]) as u32;
        let mini_shift = u16::from_le_bytes([data[32], data[33]]) as u32;
        if !(9..=12).contains(&sector_shift) || !(3..=9).contains(&mini_shift) {
            return Err(NleError::InvalidFormat(format!(
                "Unsupported CFB sector shift ({sector_shift}) or mini-sector shift ({mini_shift})"
            )));
        }
        let sector_size = 1usize << sector_shift;
        let mini_sector_size = 1usize << mini_shift;

        let num_fat_sectors = read_u32_le(data, 44) as usize;
        let first_dir_sector = read_u32_le(data, 48);
        let mini_stream_cutoff = read_u32_le(data, 56) as u64;
        let first_mini_fat_sector = read_u32_le(data, 60);
        let num_mini_fat_sectors = read_u32_le(data, 64) as usize;
        let first_difat_sector = read_u32_le(data, 68);
        let num_difat_sectors = read_u32_le(data, 72) as usize;

        // 1. Collect FAT sector locations from header DIFAT (109 entries) + extended DIFAT chain
        let mut fat_sectors: Vec<u32> = Vec::with_capacity(num_fat_sectors.min(4096));
        for i in 0..109 {
            let sid = read_u32_le(data, 76 + i * 4);
            if sid != FREESECT && sid != ENDOFCHAIN {
                fat_sectors.push(sid);
            }
        }
        let mut difat_sid = first_difat_sector;
        let mut difat_seen = HashSet::new();
        for _ in 0..num_difat_sectors {
            if difat_sid >= DIFSECT || !difat_seen.insert(difat_sid) {
                break;
            }
            let Some(sec) = sector_slice(data, sector_size, difat_sid) else {
                break;
            };
            let count = (sector_size / 4).saturating_sub(1);
            for i in 0..count {
                let sid = read_u32_le(sec, i * 4);
                if sid != FREESECT && sid != ENDOFCHAIN {
                    fat_sectors.push(sid);
                }
            }
            difat_sid = read_u32_le(sec, count * 4);
        }
        if num_fat_sectors > 0 && fat_sectors.len() > num_fat_sectors {
            fat_sectors.truncate(num_fat_sectors);
        }

        // 2. Build FAT table
        let entries_per_sector = sector_size / 4;
        let mut fat: Vec<u32> = Vec::with_capacity(fat_sectors.len() * entries_per_sector);
        for &fsid in &fat_sectors {
            let Some(sec) = sector_slice(data, sector_size, fsid) else {
                break;
            };
            for i in 0..entries_per_sector {
                fat.push(read_u32_le(sec, i * 4));
            }
        }

        // 3. Read Directory entries
        let dir_bytes = read_fat_chain(data, sector_size, &fat, first_dir_sector, None)?;
        let num_entries = dir_bytes.len() / 128;
        let mut entries = Vec::with_capacity(num_entries);
        for id in 0..num_entries {
            let off = id * 128;
            let raw = &dir_bytes[off..off + 128];
            let obj_type = match raw[66] {
                1 => CfbObjType::Storage,
                2 => CfbObjType::Stream,
                5 => CfbObjType::Root,
                _ => CfbObjType::Unknown,
            };
            let name_len = (u16::from_le_bytes([raw[64], raw[65]]) as usize).min(64);
            let name = decode_utf16le_name(&raw[0..name_len]);
            let left = read_u32_le(raw, 68);
            let right = read_u32_le(raw, 72);
            let child = read_u32_le(raw, 76);
            let mut clsid = [0u8; 16];
            clsid.copy_from_slice(&raw[80..96]);
            let start_sector = read_u32_le(raw, 116);
            let mut stream_size = u64::from_le_bytes([
                raw[120], raw[121], raw[122], raw[123], raw[124], raw[125], raw[126], raw[127],
            ]);
            if major_ver == 3 {
                stream_size &= 0xFFFF_FFFF;
            }
            entries.push(CfbEntry {
                id,
                name,
                obj_type,
                clsid,
                left_sibling: (left != NOSTREAM).then_some(left as usize),
                right_sibling: (right != NOSTREAM).then_some(right as usize),
                child: (child != NOSTREAM).then_some(child as usize),
                start_sector,
                stream_size,
            });
        }

        // 4. Read MiniFAT & MiniStream (rooted at Directory entry 0)
        let mut mini_fat = Vec::new();
        if first_mini_fat_sector < DIFSECT && num_mini_fat_sectors > 0 {
            let mf_bytes = read_fat_chain(
                data,
                sector_size,
                &fat,
                first_mini_fat_sector,
                Some(num_mini_fat_sectors * sector_size),
            )?;
            for chunk in mf_bytes.as_chunks::<4>().0 {
                mini_fat.push(u32::from_le_bytes(*chunk));
            }
        }

        let mini_stream = if let Some(root) = entries.first()
            && root.start_sector < DIFSECT
            && root.stream_size > 0
        {
            read_fat_chain(
                data,
                sector_size,
                &fat,
                root.start_sector,
                Some(root.stream_size as usize),
            )?
        } else {
            Vec::new()
        };

        Ok(Self {
            data,
            sector_size,
            mini_sector_size,
            mini_stream_cutoff,
            fat,
            mini_fat,
            mini_stream,
            entries,
        })
    }

    pub fn root_id(&self) -> usize {
        0
    }

    pub fn entry(&self, id: usize) -> Option<&CfbEntry> {
        self.entries.get(id)
    }

    pub fn children(&self, storage_id: usize) -> Vec<usize> {
        let Some(entry) = self.entries.get(storage_id) else {
            return Vec::new();
        };
        let Some(root_child) = entry.child else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        self.collect_bst(root_child, &mut visited, &mut out);
        out
    }

    fn collect_bst(&self, id: usize, visited: &mut HashSet<usize>, out: &mut Vec<usize>) {
        if !visited.insert(id) {
            return;
        }
        let Some(entry) = self.entries.get(id) else {
            return;
        };
        if let Some(l) = entry.left_sibling {
            self.collect_bst(l, visited, out);
        }
        if entry.obj_type != CfbObjType::Unknown {
            out.push(id);
        }
        if let Some(r) = entry.right_sibling {
            self.collect_bst(r, visited, out);
        }
    }

    pub fn find_child(&self, storage_id: usize, name: &str) -> Option<usize> {
        let target_utf16: Vec<u16> = name.encode_utf16().collect();
        for cid in self.children(storage_id) {
            if let Some(e) = self.entries.get(cid) {
                let cand: Vec<u16> = e.name.encode_utf16().collect();
                if cfb_name_cmp(&cand, &target_utf16) == Ordering::Equal {
                    return Some(cid);
                }
            }
        }
        None
    }

    pub fn read_stream(&self, stream_id: usize) -> NleResult<Vec<u8>> {
        let entry = self
            .entries
            .get(stream_id)
            .ok_or_else(|| NleError::InvalidFormat(format!("Missing CFB stream {stream_id}")))?;
        if entry.obj_type != CfbObjType::Stream {
            return Err(NleError::InvalidFormat(format!(
                "CFB entry '{}' is not a stream",
                entry.name
            )));
        }
        let size = entry.stream_size as usize;
        if size == 0 || entry.start_sector >= DIFSECT {
            return Ok(Vec::new());
        }
        if entry.stream_size < self.mini_stream_cutoff {
            read_mini_chain(
                &self.mini_stream,
                self.mini_sector_size,
                &self.mini_fat,
                entry.start_sector,
                size,
            )
        } else {
            read_fat_chain(
                self.data,
                self.sector_size,
                &self.fat,
                entry.start_sector,
                Some(size),
            )
        }
    }
}

fn sector_slice(data: &[u8], sector_size: usize, sid: u32) -> Option<&[u8]> {
    let off = (sid as usize + 1).checked_mul(sector_size)?;
    let end = off.checked_add(sector_size)?;
    if end <= data.len() {
        Some(&data[off..end])
    } else if off < data.len() {
        Some(&data[off..])
    } else {
        None
    }
}

fn read_fat_chain(
    data: &[u8],
    sector_size: usize,
    fat: &[u32],
    start_sid: u32,
    expected_len: Option<usize>,
) -> NleResult<Vec<u8>> {
    let mut out = Vec::with_capacity(expected_len.unwrap_or(sector_size));
    let mut sid = start_sid;
    let mut seen = HashSet::new();
    while sid < DIFSECT {
        if !seen.insert(sid) {
            break;
        }
        let Some(sec) = sector_slice(data, sector_size, sid) else {
            break;
        };
        out.extend_from_slice(sec);
        if let Some(limit) = expected_len
            && out.len() >= limit
        {
            out.truncate(limit);
            return Ok(out);
        }
        sid = *fat.get(sid as usize).unwrap_or(&ENDOFCHAIN);
    }
    if let Some(limit) = expected_len
        && out.len() > limit
    {
        out.truncate(limit);
    }
    Ok(out)
}

fn read_mini_chain(
    mini_stream: &[u8],
    mini_sector_size: usize,
    mini_fat: &[u32],
    start_sid: u32,
    expected_len: usize,
) -> NleResult<Vec<u8>> {
    let mut out = Vec::with_capacity(expected_len);
    let mut sid = start_sid;
    let mut seen = HashSet::new();
    while sid < DIFSECT && out.len() < expected_len {
        if !seen.insert(sid) {
            break;
        }
        let off = (sid as usize).saturating_mul(mini_sector_size);
        let end = (off + mini_sector_size).min(mini_stream.len());
        if off >= mini_stream.len() {
            break;
        }
        out.extend_from_slice(&mini_stream[off..end]);
        sid = *mini_fat.get(sid as usize).unwrap_or(&ENDOFCHAIN);
    }
    if out.len() > expected_len {
        out.truncate(expected_len);
    }
    Ok(out)
}

fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn decode_utf16le_name(bytes: &[u8]) -> String {
    let mut u16s = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.as_chunks::<2>().0 {
        let v = u16::from_le_bytes(*chunk);
        if v == 0 {
            break;
        }
        u16s.push(v);
    }
    String::from_utf16_lossy(&u16s)
}

fn cfb_name_cmp(a: &[u16], b: &[u16]) -> Ordering {
    match a.len().cmp(&b.len()) {
        Ordering::Equal => {
            for (&ca, &cb) in a.iter().zip(b.iter()) {
                let ua = ascii_upper_u16(ca);
                let ub = ascii_upper_u16(cb);
                match ua.cmp(&ub) {
                    Ordering::Equal => {}
                    ord => return ord,
                }
            }
            Ordering::Equal
        }
        ord => ord,
    }
}

fn ascii_upper_u16(c: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&c) {
        c - 32
    } else {
        c
    }
}

// ── Pure-Rust MS-CFB v3 Writer ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CfbStorageNode {
    pub name: String,
    pub clsid: [u8; 16],
    pub storages: Vec<CfbStorageNode>,
    pub streams: Vec<(String, Vec<u8>)>,
}

impl CfbStorageNode {
    pub fn new(name: impl Into<String>, clsid: [u8; 16]) -> Self {
        Self {
            name: name.into(),
            clsid,
            storages: Vec::new(),
            streams: Vec::new(),
        }
    }

    pub fn add_stream(&mut self, name: impl Into<String>, data: Vec<u8>) {
        self.streams.push((name.into(), data));
    }

    pub fn add_storage(&mut self, child: CfbStorageNode) {
        self.storages.push(child);
    }
}

struct FlatDirEntry {
    name_u16: Vec<u16>,
    obj_type: u8,
    color: u8,
    left: u32,
    right: u32,
    child: u32,
    clsid: [u8; 16],
    start_sector: u32,
    stream_size: u64,
    data: Vec<u8>,
}

/// Serialize a tree of storages & streams into a spec-compliant MS-CFB v3 (512-byte sector) binary.
pub fn write_cfb_v3(root: &CfbStorageNode) -> Vec<u8> {
    const SEC: usize = 512;

    let mut entries: Vec<FlatDirEntry> = Vec::new();
    entries.push(FlatDirEntry {
        name_u16: "Root Entry".encode_utf16().collect(),
        obj_type: 5,
        color: 1, // Black
        left: NOSTREAM,
        right: NOSTREAM,
        child: NOSTREAM,
        clsid: root.clsid,
        start_sector: ENDOFCHAIN,
        stream_size: 0,
        data: Vec::new(),
    });

    fn flatten_children(node: &CfbStorageNode, parent_id: usize, entries: &mut Vec<FlatDirEntry>) {
        let mut child_ids: Vec<usize> = Vec::new();

        for (sname, sdata) in &node.streams {
            let id = entries.len();
            entries.push(FlatDirEntry {
                name_u16: sname.encode_utf16().take(31).collect(),
                obj_type: 2,
                color: 1,
                left: NOSTREAM,
                right: NOSTREAM,
                child: NOSTREAM,
                clsid: [0u8; 16],
                start_sector: ENDOFCHAIN,
                stream_size: sdata.len() as u64,
                data: sdata.clone(),
            });
            child_ids.push(id);
        }

        for sub in &node.storages {
            let id = entries.len();
            entries.push(FlatDirEntry {
                name_u16: sub.name.encode_utf16().take(31).collect(),
                obj_type: 1,
                color: 1,
                left: NOSTREAM,
                right: NOSTREAM,
                child: NOSTREAM,
                clsid: sub.clsid,
                start_sector: 0,
                stream_size: 0,
                data: Vec::new(),
            });
            flatten_children(sub, id, entries);
            child_ids.push(id);
        }

        if child_ids.is_empty() {
            return;
        }
        // Sort siblings by MS-CFB canonical order
        child_ids.sort_by(|&a, &b| cfb_name_cmp(&entries[a].name_u16, &entries[b].name_u16));

        fn build_bst(ids: &[usize], entries: &mut [FlatDirEntry], depth: usize) -> u32 {
            if ids.is_empty() {
                return NOSTREAM;
            }
            let mid = ids.len() / 2;
            let root = ids[mid];
            let left = build_bst(&ids[..mid], entries, depth + 1);
            let right = build_bst(&ids[mid + 1..], entries, depth + 1);
            entries[root].left = left;
            entries[root].right = right;
            // Color deepest level red when uneven or keep black balanced tree (valid RB tree for balanced split)
            entries[root].color = if depth > 0 && left == NOSTREAM && right == NOSTREAM {
                0
            } else {
                1
            };
            root as u32
        }

        entries[parent_id].child = build_bst(&child_ids, entries, 0);
    }

    flatten_children(root, 0, &mut entries);

    // Build MiniStream (< 4096 bytes) and MiniFAT
    let mut mini_stream: Vec<u8> = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();

    for e in entries.iter_mut().skip(1) {
        if e.obj_type == 2 && !e.data.is_empty() && e.data.len() < MINI_STREAM_CUTOFF as usize {
            let start_ms = mini_fat.len() as u32;
            let num_ms = e.data.len().div_ceil(MINI_SECTOR_SIZE);
            for i in 0..num_ms {
                let next = if i + 1 < num_ms {
                    start_ms + (i as u32) + 1
                } else {
                    ENDOFCHAIN
                };
                mini_fat.push(next);
            }
            mini_stream.extend_from_slice(&e.data);
            let pad = num_ms * MINI_SECTOR_SIZE - e.data.len();
            mini_stream.resize(mini_stream.len() + pad, 0);
            e.start_sector = start_ms;
        }
    }

    // Allocate regular 512-byte sectors
    let mut sectors: Vec<[u8; SEC]> = Vec::new();
    let mut fat: Vec<u32> = Vec::new();

    let alloc_chain = |bytes: &[u8], sectors: &mut Vec<[u8; SEC]>, fat: &mut Vec<u32>| -> u32 {
        if bytes.is_empty() {
            return ENDOFCHAIN;
        }
        let start = sectors.len() as u32;
        let count = bytes.len().div_ceil(SEC);
        for i in 0..count {
            let mut block = [0u8; SEC];
            let src_start = i * SEC;
            let src_end = (src_start + SEC).min(bytes.len());
            block[..(src_end - src_start)].copy_from_slice(&bytes[src_start..src_end]);
            sectors.push(block);
            let next = if i + 1 < count {
                start + (i as u32) + 1
            } else {
                ENDOFCHAIN
            };
            fat.push(next);
        }
        start
    };

    // 1. MiniStream sectors
    if !mini_stream.is_empty() {
        let ms_start = alloc_chain(&mini_stream, &mut sectors, &mut fat);
        entries[0].start_sector = ms_start;
        entries[0].stream_size = mini_stream.len() as u64;
    }

    // 2. MiniFAT sectors
    let mut first_mini_fat_sector = ENDOFCHAIN;
    let mut num_mini_fat_sectors = 0u32;
    if !mini_fat.is_empty() {
        let mut mf_bytes = Vec::with_capacity(mini_fat.len() * 4);
        for &v in &mini_fat {
            mf_bytes.extend_from_slice(&v.to_le_bytes());
        }
        let padded_entries = mf_bytes.len().div_ceil(SEC) * (SEC / 4);
        while mf_bytes.len() < padded_entries * 4 {
            mf_bytes.extend_from_slice(&FREESECT.to_le_bytes());
        }
        first_mini_fat_sector = alloc_chain(&mf_bytes, &mut sectors, &mut fat);
        num_mini_fat_sectors = (mf_bytes.len() / SEC) as u32;
    }

    // 3. Regular streams (>= 4096 bytes)
    for e in entries.iter_mut().skip(1) {
        if e.obj_type == 2 && e.data.len() >= MINI_STREAM_CUTOFF as usize {
            e.start_sector = alloc_chain(&e.data, &mut sectors, &mut fat);
        }
    }

    // 4. Directory sectors (4 entries per 512B sector)
    let dir_sec_count = entries.len().div_ceil(4);
    let mut dir_bytes = vec![0u8; dir_sec_count * SEC];
    for (idx, e) in entries.iter().enumerate() {
        let off = idx * 128;
        let dst = &mut dir_bytes[off..off + 128];
        for (i, &ch) in e.name_u16.iter().take(31).enumerate() {
            dst[i * 2..i * 2 + 2].copy_from_slice(&ch.to_le_bytes());
        }
        let name_byte_len = ((e.name_u16.len().min(31) + 1) * 2) as u16;
        dst[64..66].copy_from_slice(&name_byte_len.to_le_bytes());
        dst[66] = e.obj_type;
        dst[67] = e.color;
        dst[68..72].copy_from_slice(&e.left.to_le_bytes());
        dst[72..76].copy_from_slice(&e.right.to_le_bytes());
        dst[76..80].copy_from_slice(&e.child.to_le_bytes());
        dst[80..96].copy_from_slice(&e.clsid);
        dst[116..120].copy_from_slice(&e.start_sector.to_le_bytes());
        dst[120..128].copy_from_slice(&e.stream_size.to_le_bytes());
    }
    let first_dir_sector = alloc_chain(&dir_bytes, &mut sectors, &mut fat);

    // 5. Compute required FAT + DIFAT sectors iteratively
    let base_sectors = fat.len();
    let mut num_fat_sectors = 1usize;
    let mut num_difat_sectors = 0usize;
    loop {
        let total = base_sectors + num_fat_sectors + num_difat_sectors;
        let needed_fat = total.div_ceil(128).max(1);
        let needed_difat = if needed_fat > 109 {
            (needed_fat - 109).div_ceil(127)
        } else {
            0
        };
        if needed_fat == num_fat_sectors && needed_difat == num_difat_sectors {
            break;
        }
        num_fat_sectors = needed_fat;
        num_difat_sectors = needed_difat;
    }

    let first_fat_sector = sectors.len() as u32;
    let mut fat_sector_ids = Vec::with_capacity(num_fat_sectors);
    for i in 0..num_fat_sectors {
        fat_sector_ids.push(first_fat_sector + i as u32);
        fat.push(FATSECT);
    }

    let first_difat_sector = if num_difat_sectors > 0 {
        (first_fat_sector as usize + num_fat_sectors) as u32
    } else {
        ENDOFCHAIN
    };
    fat.extend(std::iter::repeat_n(DIFSECT, num_difat_sectors));

    // Pad FAT to multiple of 128 u32 entries
    while !fat.len().is_multiple_of(128) {
        fat.push(FREESECT);
    }

    // Append FAT sectors
    for chunk in fat.as_chunks::<128>().0 {
        let mut block = [0u8; SEC];
        for (i, &val) in chunk.iter().enumerate() {
            block[i * 4..i * 4 + 4].copy_from_slice(&val.to_le_bytes());
        }
        sectors.push(block);
    }

    // Append DIFAT sectors if needed (> 109 FAT sectors)
    if num_difat_sectors > 0 {
        let extra = &fat_sector_ids[109..];
        for d in 0..num_difat_sectors {
            let mut block = [0xFFu8; SEC];
            let slice_start = d * 127;
            let slice_end = (slice_start + 127).min(extra.len());
            for (i, &sid) in extra[slice_start..slice_end].iter().enumerate() {
                block[i * 4..i * 4 + 4].copy_from_slice(&sid.to_le_bytes());
            }
            let next_difat = if d + 1 < num_difat_sectors {
                first_difat_sector + (d as u32) + 1
            } else {
                ENDOFCHAIN
            };
            block[127 * 4..128 * 4].copy_from_slice(&next_difat.to_le_bytes());
            sectors.push(block);
        }
    }

    // Build 512-byte CFB v3 header
    let mut header = [0u8; SEC];
    header[0..8].copy_from_slice(&CFB_MAGIC);
    header[24..26].copy_from_slice(&0x003Eu16.to_le_bytes()); // Minor version
    header[26..28].copy_from_slice(&3u16.to_le_bytes()); // Major version 3
    header[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes()); // Little-endian
    header[30..32].copy_from_slice(&9u16.to_le_bytes()); // Sector shift (512B)
    header[32..34].copy_from_slice(&6u16.to_le_bytes()); // Mini sector shift (64B)
    header[44..48].copy_from_slice(&(num_fat_sectors as u32).to_le_bytes());
    header[48..52].copy_from_slice(&first_dir_sector.to_le_bytes());
    header[56..60].copy_from_slice(&MINI_STREAM_CUTOFF.to_le_bytes());
    header[60..64].copy_from_slice(&first_mini_fat_sector.to_le_bytes());
    header[64..68].copy_from_slice(&num_mini_fat_sectors.to_le_bytes());
    header[68..72].copy_from_slice(&first_difat_sector.to_le_bytes());
    header[72..76].copy_from_slice(&(num_difat_sectors as u32).to_le_bytes());

    for i in 0..109 {
        let sid = fat_sector_ids.get(i).copied().unwrap_or(FREESECT);
        header[76 + i * 4..76 + i * 4 + 4].copy_from_slice(&sid.to_le_bytes());
    }

    let mut out = Vec::with_capacity((sectors.len() + 1) * SEC);
    out.extend_from_slice(&header);
    for s in &sectors {
        out.extend_from_slice(s);
    }
    out
}
