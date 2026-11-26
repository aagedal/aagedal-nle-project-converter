use crate::binary::{
    decode_keyed_fields, decode_protobuf, encode_keyed_fields, encode_utf16be, hex_decode,
    hex_encode, ProtoValue,
};
use uuid::Uuid;

pub const STILL_SOURCE_IN: &str = "86400";
const KEYED_DOUBLE: u32 = 0x06;
const KEYED_STRING: u32 = 0x0A;
const KEYED_BYTES: u32 = 0x0C;

/// Encode DaVinci Resolve's 60,000-second still-image hold map at source zero.
pub fn build_still_timemap() -> String {
    let extent = 60_000.0_f64;
    let mut kf = vec![0x80, 0x0A, 0x00, 0x0A, 0x09, 0x09];
    kf.extend_from_slice(&extent.to_le_bytes());

    let entries = vec![
        ("YMin".into(), KEYED_DOUBLE, (-1.0_f64).to_be_bytes().to_vec()),
        ("YMax".into(), KEYED_DOUBLE, (-1.0_f64).to_be_bytes().to_vec()),
        ("XMax".into(), KEYED_DOUBLE, extent.to_be_bytes().to_vec()),
        ("UniqueId".into(), KEYED_STRING, encode_utf16be(&Uuid::new_v4().to_string())),
        ("KeyframesBA".into(), KEYED_BYTES, kf),
        ("DbType".into(), KEYED_STRING, encode_utf16be("Sm2TimeMap")),
    ];
    hex_encode(&encode_keyed_fields(&entries))
}

/// Build `MediaTimemapBA` hex for identity (100%), constant forward speed, reverse speed (`< 0`),
/// or freeze frame (`== 0`).
pub fn build_media_timemap(
    source_frames: i64,
    source_fps: f64,
    speed_percent: f64,
    audio: bool,
) -> String {
    let frames = source_frames.max(1);
    let fps = if source_fps.is_finite() && source_fps > 0.0 {
        source_fps
    } else {
        25.0
    };
    let end_seconds = (frames - 1) as f64 / fps;
    let speed = if speed_percent.is_finite() {
        speed_percent / 100.0
    } else {
        1.0
    };

    if (speed - 1.0).abs() < 1e-6 || frames < 2 {
        let mut buf = vec![0x02];
        if audio {
            buf.extend_from_slice(&end_seconds.to_be_bytes());
        } else {
            buf.extend_from_slice(&end_seconds.to_be_bytes());
            buf.extend_from_slice(&0.0_f64.to_be_bytes());
            buf.extend_from_slice(&end_seconds.to_be_bytes());
            buf.extend_from_slice(&0.0_f64.to_be_bytes());
            buf.extend_from_slice(&end_seconds.to_be_bytes());
        }
        return hex_encode(&buf);
    }

    // Freeze frame (0% speed): hold at frame 0 across 60,000s record span.
    if speed.abs() < 1e-6 {
        let extent = 60_000.0_f64;
        let hold_sec = 0.0_f64;
        let mut kf = vec![0x80, 0x0A, 0x09, 0x11];
        kf.extend_from_slice(&hold_sec.to_le_bytes());
        kf.extend_from_slice(&[0x0A, 0x12, 0x09]);
        kf.extend_from_slice(&extent.to_le_bytes());
        kf.push(0x11);
        kf.extend_from_slice(&hold_sec.to_le_bytes());

        let entries = vec![
            ("YMin".into(), KEYED_DOUBLE, (-1.0_f64).to_be_bytes().to_vec()),
            ("YMax".into(), KEYED_DOUBLE, (-1.0_f64).to_be_bytes().to_vec()),
            ("XMax".into(), KEYED_DOUBLE, extent.to_be_bytes().to_vec()),
            ("UniqueId".into(), KEYED_STRING, encode_utf16be(&Uuid::new_v4().to_string())),
            ("LastValidYOffset".into(), KEYED_DOUBLE, hold_sec.to_be_bytes().to_vec()),
            ("KeyframesBA".into(), KEYED_BYTES, kf),
            ("DbType".into(), KEYED_STRING, encode_utf16be("Sm2TimeMap")),
        ];
        return hex_encode(&encode_keyed_fields(&entries));
    }

    // Reverse constant speed (< 0): (x=0, y=end_seconds) -> (x=record_end, y=0).
    if speed < 0.0 {
        let record_end = end_seconds / speed.abs();
        let mut kf = vec![0x80, 0x0A, 0x09, 0x11];
        kf.extend_from_slice(&end_seconds.to_le_bytes());
        kf.extend_from_slice(&[0x0A, 0x09, 0x09]);
        kf.extend_from_slice(&record_end.to_le_bytes());

        let entries = vec![
            ("YMin".into(), KEYED_DOUBLE, (-1.0_f64).to_be_bytes().to_vec()),
            ("YMax".into(), KEYED_DOUBLE, (-1.0_f64).to_be_bytes().to_vec()),
            ("XMax".into(), KEYED_DOUBLE, record_end.to_be_bytes().to_vec()),
            ("UniqueId".into(), KEYED_STRING, encode_utf16be(&Uuid::new_v4().to_string())),
            ("KeyframesBA".into(), KEYED_BYTES, kf),
            ("DbType".into(), KEYED_STRING, encode_utf16be("Sm2TimeMap")),
        ];
        return hex_encode(&encode_keyed_fields(&entries));
    }

    let record_end = end_seconds / speed;
    let mut kf = vec![0x80, 0x0A, 0x00, 0x0A, 0x12, 0x09];
    kf.extend_from_slice(&record_end.to_le_bytes());
    kf.push(0x11);
    kf.extend_from_slice(&end_seconds.to_le_bytes());

    let entries = vec![
        ("YMin".into(), KEYED_DOUBLE, (-1.0_f64).to_be_bytes().to_vec()),
        ("YMax".into(), KEYED_DOUBLE, (-1.0_f64).to_be_bytes().to_vec()),
        ("XMax".into(), KEYED_DOUBLE, record_end.to_be_bytes().to_vec()),
        ("UniqueId".into(), KEYED_STRING, encode_utf16be(&Uuid::new_v4().to_string())),
        ("LastValidYOffset".into(), KEYED_DOUBLE, end_seconds.to_be_bytes().to_vec()),
        ("KeyframesBA".into(), KEYED_BYTES, kf),
        ("DbType".into(), KEYED_STRING, encode_utf16be("Sm2TimeMap")),
    ];
    hex_encode(&encode_keyed_fields(&entries))
}

/// Decode Resolve's `MediaTimemapBA` hex into `(speed_percent, reversed, freeze_hold_sec)`.
/// Returns `Some((100.0, false, None))` for identity `0x02...` maps, negative speed + `reversed = true`
/// for reverse playback, and `Some((0.0, false, Some(hold_sec)))` for freeze frames / still holds.
pub fn decode_media_timemap_detail(hex: &str) -> Option<(f64, bool, Option<f64>)> {
    let raw = hex_decode(hex).ok()?;
    if raw.is_empty() {
        return None;
    }
    if raw[0] == 0x02 {
        return Some((100.0, false, None));
    }
    let fields = decode_keyed_fields(&raw).ok()?;
    let kf_raw = fields
        .into_iter()
        .find(|(k, _, _)| k == "KeyframesBA")
        .map(|(_, _, v)| v)?;
    // KeyframesBA begins with a 1-byte flags prefix (0x80) followed by repeated field 1 Keyframe messages.
    let pb_slice = if kf_raw.first() == Some(&0x80) {
        &kf_raw[1..]
    } else {
        &kf_raw[..]
    };
    let pb = decode_protobuf(pb_slice).ok()?;
    let mut points: Vec<(f64, f64)> = Vec::new();
    for f in pb {
        if f.number == 1 {
            if let ProtoValue::Bytes(bytes) = f.value {
                let mut x = 0.0_f64;
                let mut y = 0.0_f64;
                if let Ok(sub) = decode_protobuf(&bytes) {
                    for sf in sub {
                        if let ProtoValue::Fixed64(arr) = sf.value {
                            match sf.number {
                                1 => x = f64::from_le_bytes(arr),
                                2 => y = f64::from_le_bytes(arr),
                                _ => {}
                            }
                        }
                    }
                }
                points.push((x, y));
            }
        }
    }
    if points.len() < 2 {
        return None;
    }
    let (x0, y0) = points[0];
    let (x1, y1) = points[points.len() - 1];
    let dx = x1 - x0;
    if !dx.is_finite() || dx.abs() < 1e-9 {
        return None;
    }
    let dy = y1 - y0;
    if dy.abs() < 1e-9 {
        return Some((0.0, false, Some(y0.max(0.0))));
    }
    let speed = (dy / dx) * 100.0;
    if !speed.is_finite() {
        return None;
    }
    Some((speed, speed < 0.0, None))
}

/// Decode Resolve's `MediaTimemapBA` hex into `(speed_percent, reversed)`.
pub fn decode_media_timemap(hex: &str) -> Option<(f64, bool)> {
    let (spd, rev, _) = decode_media_timemap_detail(hex)?;
    Some((spd, rev))
}

/// Encode source in-point as Resolve's record-domain frame position (integer or `whole|<f64_le_hex>`).
/// For reversed clips (`reversed = true`), Resolve measures `<In>` backwards from the end of the media
/// (`(asset_frames - source_out) / source_fps / |speed| * timeline_fps`).
pub fn retimed_source_in(
    source_in: i64,
    source_out: i64,
    asset_frames: i64,
    source_fps: f64,
    timeline_fps: f64,
    speed_percent: f64,
    reversed: bool,
) -> String {
    let src_rate = if source_fps > 0.0 { source_fps } else { 25.0 };
    let tl_rate = if timeline_fps > 0.0 { timeline_fps } else { 25.0 };
    let speed = if speed_percent.abs() > 1e-6 {
        speed_percent.abs() / 100.0
    } else {
        1.0
    };

    let effective_src_frames = if reversed {
        (asset_frames - source_out).max(0) as f64
    } else {
        source_in.max(0) as f64
    };

    let record_frames = effective_src_frames / src_rate / speed * tl_rate;
    let whole = record_frames.floor();
    let frac = record_frames - whole;
    if frac.abs() < 1e-12 {
        format!("{}", whole as i64)
    } else if (1.0 - frac).abs() < 1e-12 {
        format!("{}", whole as i64 + 1)
    } else {
        format!("{}|{}", whole as i64, hex_encode(&frac.to_le_bytes()))
    }
}
