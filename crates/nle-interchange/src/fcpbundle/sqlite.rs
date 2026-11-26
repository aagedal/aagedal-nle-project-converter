use nle_core::{NleError, NleResult};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Column value decoded from a SQLite 3 table B-tree record.
#[derive(Debug, Clone)]
pub enum SqlValue {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqlValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            SqlValue::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            SqlValue::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            SqlValue::Blob(b) => Some(b.as_slice()),
            _ => None,
        }
    }
}

/// Minimal zero-dependency pure-Rust read-only SQLite 3 database reader
/// (supports standard table B-trees, overflow pages, and `-wal` overlay).
pub struct SqliteDb {
    bytes: Vec<u8>,
    page_size: usize,
    usable_size: usize,
    wal_pages: HashMap<u32, Vec<u8>>,
    table_roots: HashMap<String, u32>,
}

impl SqliteDb {
    pub fn open(path: &Path) -> NleResult<Self> {
        let bytes = fs::read(path)?;
        if bytes.len() < 100 || &bytes[0..16] != b"SQLite format 3\0" {
            return Err(NleError::InvalidFormat(format!(
                "'{}' is not a valid SQLite 3 database",
                path.display()
            )));
        }

        let raw_ps = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
        let page_size = if raw_ps == 1 { 65536 } else { raw_ps };
        if page_size < 512 || (page_size & (page_size - 1)) != 0 {
            return Err(NleError::InvalidFormat(format!(
                "Invalid SQLite page size: {page_size}"
            )));
        }
        let reserved = bytes[20] as usize;
        let usable_size = page_size.saturating_sub(reserved);

        // Load committed WAL frames if a sibling `-wal` file exists
        let mut wal_path = path.as_os_str().to_os_string();
        wal_path.push("-wal");
        let wal_pages = load_wal_pages(Path::new(&wal_path), page_size);

        let mut db = Self {
            bytes,
            page_size,
            usable_size,
            wal_pages,
            table_roots: HashMap::new(),
        };

        // Read sqlite_master (page 1): columns = [type, name, tbl_name, rootpage, sql]
        let master_rows = db.read_table_by_root(1)?;
        for (_rowid, cols) in master_rows {
            let kind = cols.first().and_then(SqlValue::as_str).unwrap_or("");
            let name = cols.get(1).and_then(SqlValue::as_str).unwrap_or("");
            let root = cols.get(3).and_then(SqlValue::as_i64).unwrap_or(0);
            if kind == "table" && !name.is_empty() && root > 0 {
                db.table_roots.insert(name.to_string(), root as u32);
            }
        }

        Ok(db)
    }

    pub fn has_table(&self, name: &str) -> bool {
        self.table_roots.contains_key(name)
    }

    /// Read all rows `(rowid, columns)` from the named table.
    pub fn read_table(&self, name: &str) -> NleResult<Vec<(i64, Vec<SqlValue>)>> {
        let Some(&root) = self.table_roots.get(name) else {
            return Ok(Vec::new());
        };
        self.read_table_by_root(root)
    }

    fn get_page(&self, page_no: u32) -> NleResult<&[u8]> {
        if page_no == 0 {
            return Err(NleError::InvalidFormat("SQLite page 0 is invalid".into()));
        }
        if let Some(wal_page) = self.wal_pages.get(&page_no) {
            return Ok(wal_page.as_slice());
        }
        let start = ((page_no as usize) - 1) * self.page_size;
        let end = start + self.page_size;
        if end > self.bytes.len() {
            return Err(NleError::InvalidFormat(format!(
                "SQLite page {page_no} out of range (file size {})",
                self.bytes.len()
            )));
        }
        Ok(&self.bytes[start..end])
    }

    fn read_table_by_root(&self, root_page: u32) -> NleResult<Vec<(i64, Vec<SqlValue>)>> {
        let mut out = Vec::new();
        let mut visited = std::collections::HashSet::new();
        self.walk_table_btree(root_page, &mut visited, &mut out)?;
        Ok(out)
    }

    fn walk_table_btree(
        &self,
        page_no: u32,
        visited: &mut std::collections::HashSet<u32>,
        out: &mut Vec<(i64, Vec<SqlValue>)>,
    ) -> NleResult<()> {
        if !visited.insert(page_no) {
            return Ok(());
        }
        let page = self.get_page(page_no)?;
        let hdr_off = if page_no == 1 { 100 } else { 0 };
        if hdr_off + 8 > page.len() {
            return Ok(());
        }

        let page_type = page[hdr_off];
        let cell_count = u16::from_be_bytes([page[hdr_off + 3], page[hdr_off + 4]]) as usize;

        match page_type {
            // Interior table B-tree page
            0x05 => {
                if hdr_off + 12 > page.len() {
                    return Ok(());
                }
                let right_child = u32::from_be_bytes([
                    page[hdr_off + 8],
                    page[hdr_off + 9],
                    page[hdr_off + 10],
                    page[hdr_off + 11],
                ]);
                let ptrs_start = hdr_off + 12;
                for i in 0..cell_count {
                    let p_off = ptrs_start + i * 2;
                    if p_off + 2 > page.len() {
                        break;
                    }
                    let cell_off = u16::from_be_bytes([page[p_off], page[p_off + 1]]) as usize;
                    if cell_off + 4 > page.len() {
                        continue;
                    }
                    let left_child = u32::from_be_bytes([
                        page[cell_off],
                        page[cell_off + 1],
                        page[cell_off + 2],
                        page[cell_off + 3],
                    ]);
                    self.walk_table_btree(left_child, visited, out)?;
                }
                self.walk_table_btree(right_child, visited, out)?;
            }
            // Leaf table B-tree page
            0x0d => {
                let ptrs_start = hdr_off + 8;
                let max_local = self.usable_size.saturating_sub(35);
                let min_local = ((self.usable_size.saturating_sub(12)) * 32 / 255).saturating_sub(23);

                for i in 0..cell_count {
                    let p_off = ptrs_start + i * 2;
                    if p_off + 2 > page.len() {
                        break;
                    }
                    let cell_off = u16::from_be_bytes([page[p_off], page[p_off + 1]]) as usize;
                    if cell_off >= page.len() {
                        continue;
                    }
                    let (payload_len_u64, n1) = read_varint(&page[cell_off..]);
                    let (rowid_u64, n2) = read_varint(&page[cell_off + n1..]);
                    let payload_len = payload_len_u64 as usize;
                    let rowid = rowid_u64 as i64;
                    let payload_start = cell_off + n1 + n2;

                    let payload = if payload_len <= max_local {
                        let end = (payload_start + payload_len).min(page.len());
                        page[payload_start..end].to_vec()
                    } else {
                        let k = min_local
                            + ((payload_len.saturating_sub(min_local))
                                % self.usable_size.saturating_sub(4).max(1));
                        let local_len = if k <= max_local { k } else { min_local };
                        let mut buf = Vec::with_capacity(payload_len);
                        let local_end = (payload_start + local_len).min(page.len());
                        buf.extend_from_slice(&page[payload_start..local_end]);

                        if local_end + 4 <= page.len() {
                            let mut ovf = u32::from_be_bytes([
                                page[local_end],
                                page[local_end + 1],
                                page[local_end + 2],
                                page[local_end + 3],
                            ]);
                            let mut guard = 0usize;
                            while ovf != 0 && buf.len() < payload_len && guard < 10_000 {
                                guard += 1;
                                let Ok(opage) = self.get_page(ovf) else {
                                    break;
                                };
                                if opage.len() < 4 {
                                    break;
                                }
                                let next_ovf =
                                    u32::from_be_bytes([opage[0], opage[1], opage[2], opage[3]]);
                                let chunk_max = self.usable_size.saturating_sub(4).min(opage.len() - 4);
                                let take = (payload_len - buf.len()).min(chunk_max);
                                buf.extend_from_slice(&opage[4..4 + take]);
                                ovf = next_ovf;
                            }
                        }
                        buf
                    };

                    if let Some(cols) = decode_record(&payload) {
                        out.push((rowid, cols));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn load_wal_pages(wal_path: &Path, expected_page_size: usize) -> HashMap<u32, Vec<u8>> {
    let mut committed = HashMap::new();
    let Ok(data) = fs::read(wal_path) else {
        return committed;
    };
    if data.len() < 32 {
        return committed;
    }
    let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    if magic != 0x377f_0682 && magic != 0x377f_0683 {
        return committed;
    }
    let wal_ps = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;
    if wal_ps != expected_page_size {
        return committed;
    }
    let hdr_salt1 = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let hdr_salt2 = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);

    let frame_size = 24 + expected_page_size;
    let mut off = 32;
    let mut pending: Vec<(u32, Vec<u8>)> = Vec::new();
    while off + frame_size <= data.len() {
        let fh = &data[off..off + 24];
        let page_no = u32::from_be_bytes([fh[0], fh[1], fh[2], fh[3]]);
        let db_size = u32::from_be_bytes([fh[4], fh[5], fh[6], fh[7]]);
        let s1 = u32::from_be_bytes([fh[8], fh[9], fh[10], fh[11]]);
        let s2 = u32::from_be_bytes([fh[12], fh[13], fh[14], fh[15]]);
        if s1 != hdr_salt1 || s2 != hdr_salt2 || page_no == 0 {
            break;
        }
        let page_bytes = data[off + 24..off + frame_size].to_vec();
        pending.push((page_no, page_bytes));
        if db_size > 0 {
            for (pn, pb) in pending.drain(..) {
                committed.insert(pn, pb);
            }
        }
        off += frame_size;
    }
    committed
}

fn read_varint(buf: &[u8]) -> (u64, usize) {
    let mut val: u64 = 0;
    for (i, &b) in buf.iter().take(9).enumerate() {
        if i == 8 {
            val = (val << 8) | (b as u64);
            return (val, 9);
        }
        val = (val << 7) | ((b & 0x7f) as u64);
        if (b & 0x80) == 0 {
            return (val, i + 1);
        }
    }
    (val, buf.len().min(9))
}

fn decode_record(payload: &[u8]) -> Option<Vec<SqlValue>> {
    if payload.is_empty() {
        return None;
    }
    let (hdr_len_u64, mut pos) = read_varint(payload);
    let hdr_len = (hdr_len_u64 as usize).min(payload.len());
    let mut serial_types = Vec::new();
    while pos < hdr_len {
        let (st, n) = read_varint(&payload[pos..hdr_len]);
        if n == 0 {
            break;
        }
        serial_types.push(st);
        pos += n;
    }

    let mut data_pos = hdr_len;
    let mut cols = Vec::with_capacity(serial_types.len());
    for st in serial_types {
        let val = match st {
            0 => SqlValue::Null,
            1 => {
                let b = *payload.get(data_pos)?;
                data_pos += 1;
                SqlValue::Int(b as i8 as i64)
            }
            2 => {
                let s = payload.get(data_pos..data_pos + 2)?;
                data_pos += 2;
                SqlValue::Int(i16::from_be_bytes([s[0], s[1]]) as i64)
            }
            3 => {
                let s = payload.get(data_pos..data_pos + 3)?;
                data_pos += 3;
                let sign = if (s[0] & 0x80) != 0 { 0xff } else { 0x00 };
                SqlValue::Int(i32::from_be_bytes([sign, s[0], s[1], s[2]]) as i64)
            }
            4 => {
                let s = payload.get(data_pos..data_pos + 4)?;
                data_pos += 4;
                SqlValue::Int(i32::from_be_bytes([s[0], s[1], s[2], s[3]]) as i64)
            }
            5 => {
                let s = payload.get(data_pos..data_pos + 6)?;
                data_pos += 6;
                let sign = if (s[0] & 0x80) != 0 { 0xff } else { 0x00 };
                SqlValue::Int(i64::from_be_bytes([
                    sign, sign, s[0], s[1], s[2], s[3], s[4], s[5],
                ]))
            }
            6 => {
                let s = payload.get(data_pos..data_pos + 8)?;
                data_pos += 8;
                SqlValue::Int(i64::from_be_bytes([
                    s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
                ]))
            }
            7 => {
                let s = payload.get(data_pos..data_pos + 8)?;
                data_pos += 8;
                SqlValue::Float(f64::from_be_bytes([
                    s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
                ]))
            }
            8 => SqlValue::Int(0),
            9 => SqlValue::Int(1),
            10 | 11 => SqlValue::Null,
            n if n >= 12 && n % 2 == 0 => {
                let len = ((n - 12) / 2) as usize;
                let end = (data_pos + len).min(payload.len());
                let b = payload[data_pos..end].to_vec();
                data_pos = end;
                SqlValue::Blob(b)
            }
            n => {
                let len = ((n - 13) / 2) as usize;
                let end = (data_pos + len).min(payload.len());
                let s = String::from_utf8_lossy(&payload[data_pos..end]).into_owned();
                data_pos = end;
                SqlValue::Text(s)
            }
        };
        cols.push(val);
    }

    Some(cols)
}
