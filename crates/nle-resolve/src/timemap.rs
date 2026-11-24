use crate::binary::{encode_keyed_fields, encode_utf16be, hex_encode};
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

/// Build `MediaTimemapBA` hex for identity (100%) or constant-speed forward playback.
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
    let speed = if speed_percent.is_finite() && speed_percent > 0.0 {
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

/// Encode source in-point as Resolve's record-domain frame position (integer or `whole|<f64_le_hex>`).
pub fn retimed_source_in(
    source_in: i64,
    source_fps: f64,
    timeline_fps: f64,
    speed_percent: f64,
) -> String {
    let src_in = source_in.max(0) as f64;
    let src_rate = if source_fps > 0.0 { source_fps } else { 25.0 };
    let tl_rate = if timeline_fps > 0.0 { timeline_fps } else { 25.0 };
    let speed = if speed_percent > 0.0 { speed_percent / 100.0 } else { 1.0 };

    let record_frames = src_in / src_rate / speed * tl_rate;
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
