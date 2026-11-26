use crate::binary::{decode_protobuf, encode_protobuf, hex_decode, hex_encode, ProtoField, ProtoValue};
use nle_core::{ColorWheel, LumetriData};
use std::collections::BTreeMap;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ── Resolve 21.2 Primary Corrector (Corrector 1) DRX Parameter IDs ────
// Verified directly against live DaVinci Resolve Studio 21.2 `.drp` exports.
#[allow(dead_code)]
const DRX_HUE_ID: u64 = 0x0600_0004;         // 100_663_300: (ui - 50.0) / 50.0, neutral 0.0
const DRX_SATURATION_ID: u64 = 0x0600_0005;  // 100_663_301: ui / 50.0, neutral 1.0
const DRX_LIFT_R_ID: u64 = 0x0600_0018;      // 100_663_320: ui * 2.0, neutral 0.0
const DRX_LIFT_G_ID: u64 = 0x0600_0019;      // 100_663_321: ui * 2.0, neutral 0.0
const DRX_LIFT_B_ID: u64 = 0x0600_001A;      // 100_663_322: ui * 2.0, neutral 0.0
const DRX_LIFT_Y_ID: u64 = 0x0600_001B;      // 100_663_323: ui * 2.0, neutral 0.0
const DRX_GAIN_R_ID: u64 = 0x0600_001D;      // 100_663_325: ui * 1.0, neutral 1.0
const DRX_GAIN_G_ID: u64 = 0x0600_001E;      // 100_663_326: ui * 1.0, neutral 1.0
const DRX_GAIN_B_ID: u64 = 0x0600_001F;      // 100_663_327: ui * 1.0, neutral 1.0
const DRX_GAIN_Y_ID: u64 = 0x0600_0020;      // 100_663_328: ui * 1.0, neutral 1.0
const DRX_GAMMA_R_ID: u64 = 0x0600_0022;     // 100_663_330: ui * 4.0, neutral 0.0
const DRX_GAMMA_G_ID: u64 = 0x0600_0023;     // 100_663_331: ui * 4.0, neutral 0.0
const DRX_GAMMA_B_ID: u64 = 0x0600_0024;     // 100_663_332: ui * 4.0, neutral 0.0
const DRX_GAMMA_Y_ID: u64 = 0x0600_0025;     // 100_663_333: ui * 4.0, neutral 0.0
const DRX_OFFSET_R_ID: u64 = 0x0600_007D;    // 100_663_421: (ui - 25.0) / 25.0, neutral 0.0
const DRX_OFFSET_G_ID: u64 = 0x0600_007E;    // 100_663_422: (ui - 25.0) / 25.0, neutral 0.0
const DRX_OFFSET_B_ID: u64 = 0x0600_007F;    // 100_663_423: (ui - 25.0) / 25.0, neutral 0.0
#[allow(dead_code)]
const DRX_LUM_MIX_ID: u64 = 0x8600_000B;     // 2_248_146_955: ui / 100.0, neutral 1.0
const DRX_CONTRAST_ID: u64 = 0x8600_00C1;    // 2_248_147_137: ui * 1.0, neutral 1.0
const DRX_HIGHLIGHTS_ID: u64 = 0x8600_0110;  // 2_248_147_216: ui * 1.0, neutral 0.0
const DRX_SHADOWS_ID: u64 = 0x8600_0111;     // 2_248_147_217: ui * 1.0, neutral 0.0
const DRX_COLOR_BOOST_ID: u64 = 0x8600_0112; // 2_248_147_218: ui * 1.0, neutral 0.0
const DRX_MID_DETAIL_ID: u64 = 0x8600_0113;  // 2_248_147_219: ui * 1.0, neutral 0.0
const DRX_TEMP_ID: u64 = 0x8600_0115;        // 2_248_147_221: ui * 1.0, neutral 0.0
const DRX_TINT_ID: u64 = 0x8600_0116;        // 2_248_147_222: ui * 1.0, neutral 0.0

// ── Inspector Stabilization (Corrector 10) ────────────────────────────
const DRX_STAB_CORRECTOR_ID: u64 = 10;
const DRX_STAB_MODE_PARAM_ID: u64 = 0x1030_0016; // 271_581_206
const DRX_STAB_OPTS_PARAM_ID: u64 = 0x1030_001D; // 271_581_213

const VERSION_TABLE_FIELDS_BLOB: &str = "00000001000000010000002200410063007400690076006500560065007200730069006f006e0054007900700065000000020000";

// Rec.709 luminance weights matching Resolve's Primaries Y channel and Premiere's BasicCorrection3.
const LUMA_R: f64 = 0.2126;
const LUMA_G: f64 = 0.7152;
const LUMA_B: f64 = 0.0722;

// Premiere Lumetri <-> Resolve Primaries calibration scales.
// Premiere BasicCorrection3 applies Exposure (`pow(2, exp)`) in linearized gamma-2.2 space,
// whereas Resolve Primary Gain operates in gamma-encoded display space (`gain = 2^(exp / 2.2)`).
// Premiere's Color Wheels have a soft shadow/highlight roll-off, whereas Resolve Lift/Gamma
// directly shift the black floor and midtone exponent, so Lift/Gamma need finer scales.
const EXPOSURE_GAMMA: f64 = 2.2;
const LIFT_CHROMA_SCALE: f64 = 0.06;
const LIFT_LUMA_SCALE: f64 = 0.12;
const GAMMA_CHROMA_SCALE: f64 = 0.08;
const GAMMA_LUMA_SCALE: f64 = 0.15;
const GAIN_CHROMA_SCALE: f64 = 0.25;
const GAIN_LUMA_SCALE: f64 = 0.25;
const BLACKS_TO_LIFT_SCALE: f64 = 0.05;
const WHITES_TO_GAIN_SCALE: f64 = 0.10;
const CONTRAST_PREMIERE_TO_RESOLVE: f64 = 0.005;
const SHADOWS_HIGHLIGHTS_SCALE: f64 = 0.5;
const TEMP_PREMIERE_TO_RESOLVE: f64 = 10.0;

fn grade_parameter(param_id: u64, value: f32) -> ProtoField {
    let val_msg = encode_protobuf(&[ProtoField::f32_le(1, value)]);
    let entry = encode_protobuf(&[ProtoField::varint(1, param_id), ProtoField::bytes(2, val_msg)]);
    ProtoField::bytes(3, entry)
}

/// Pure-hue RGB triplet in `[0.0, 1.0]^3` for standard HSL hue angle (`0°=R, 120°=G, 240°=B`).
fn hsl_pure_hue_rgb(hue_deg: f64) -> (f64, f64, f64) {
    let h = hue_deg.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h.floor() as i32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    }
}

/// Convert a Premiere Lumetri `ColorWheel` `(hue_deg, luma, saturation)` into a zero-centered
/// Resolve Primary wheel `(dR, dG, dB, dY)` where `LUMA_R*dR + LUMA_G*dG + LUMA_B*dB == dY`.
pub fn wheel_hsl_to_yrgb_delta(
    wheel: ColorWheel,
    chroma_scale: f64,
    luma_scale: f64,
) -> (f64, f64, f64, f64) {
    let dy = (wheel.luma.clamp(0.0, 1.0) - 0.5) * luma_scale;
    let sat = wheel.saturation.clamp(0.0, 1.0);
    if sat < 1e-9 {
        return (dy, dy, dy, dy);
    }
    let (r0, g0, b0) = hsl_pure_hue_rgb(wheel.hue_deg);
    let y0 = LUMA_R * r0 + LUMA_G * g0 + LUMA_B * b0;
    let scale = sat * chroma_scale;
    (
        dy + (r0 - y0) * scale,
        dy + (g0 - y0) * scale,
        dy + (b0 - y0) * scale,
        dy,
    )
}

/// Exact inverse of `wheel_hsl_to_yrgb_delta`: recover Premiere `ColorWheel` from Resolve
/// zero-centered `(dR, dG, dB, dY)`.
pub fn yrgb_delta_to_wheel(
    dr: f64,
    dg: f64,
    db: f64,
    dy: f64,
    chroma_scale: f64,
    luma_scale: f64,
) -> ColorWheel {
    let luma = (0.5 + dy / luma_scale).clamp(0.0, 1.0);
    let cr = dr - dy;
    let cg = dg - dy;
    let cb = db - dy;
    let cmax = cr.max(cg).max(cb);
    let cmin = cr.min(cg).min(cb);
    let delta = cmax - cmin;
    if delta < 1e-6 {
        return ColorWheel {
            hue_deg: 0.0,
            luma,
            saturation: 0.0,
        };
    }
    let hue_deg = if (cmax - cr).abs() < 1e-9 {
        60.0 * (((cg - cb) / delta).rem_euclid(6.0))
    } else if (cmax - cg).abs() < 1e-9 {
        60.0 * (((cb - cr) / delta) + 2.0)
    } else {
        60.0 * (((cr - cg) / delta) + 4.0)
    };
    ColorWheel {
        hue_deg: hue_deg.rem_euclid(360.0),
        luma,
        saturation: (delta / chroma_scale).clamp(0.0, 1.0),
    }
}

fn primary_corrector(lumetri: &LumetriData) -> Vec<u8> {
    // Store in BTreeMap so parameters are emitted in canonical ascending Resolve param_id order.
    let mut params: BTreeMap<u64, f32> = BTreeMap::new();

    // 1. Saturation (Premiere 0..200 neutral 100 -> Resolve UI 0..100 neutral 50 -> DRX 0.0..2.0 neutral 1.0)
    if let Some(sat) = lumetri.saturation {
        if sat.is_finite() && (0.0..=200.0).contains(&sat) && (sat - 100.0).abs() > 1e-6 {
            params.insert(DRX_SATURATION_ID, (sat / 100.0) as f32);
        }
    }

    // 2. Lift wheel = Shadows Color Wheel + Basic Correction Blacks (DRX = ui * 2.0, neutral 0.0)
    let mut lift = [0.0f64; 4]; // R, G, B, Y in Resolve UI units
    let mut has_lift = false;
    if let Some(w) = lumetri.shadows_wheel.filter(|w| !w.is_neutral()) {
        let (dr, dg, db, dy) = wheel_hsl_to_yrgb_delta(w, LIFT_CHROMA_SCALE, LIFT_LUMA_SCALE);
        lift[0] += dr;
        lift[1] += dg;
        lift[2] += db;
        lift[3] += dy;
        has_lift = true;
    }
    if let Some(blacks) = lumetri.blacks {
        if blacks.is_finite() && (-100.0..=100.0).contains(&blacks) && blacks.abs() > 1e-6 {
            let d = (blacks / 100.0) * BLACKS_TO_LIFT_SCALE;
            for ch in &mut lift {
                *ch += d;
            }
            has_lift = true;
        }
    }
    if has_lift && lift.iter().any(|v| v.abs() > 1e-6) {
        params.insert(DRX_LIFT_R_ID, (lift[0] * 2.0) as f32);
        params.insert(DRX_LIFT_G_ID, (lift[1] * 2.0) as f32);
        params.insert(DRX_LIFT_B_ID, (lift[2] * 2.0) as f32);
        params.insert(DRX_LIFT_Y_ID, (lift[3] * 2.0) as f32);
    }

    // 3. Gain wheel = Highlights Color Wheel * Exposure (2^stops) * Whites (DRX = ui * 1.0, neutral 1.0)
    let mut gain = [1.0f64; 4]; // R, G, B, Y in Resolve UI units
    let mut has_gain = false;
    if let Some(w) = lumetri.highlights_wheel.filter(|w| !w.is_neutral()) {
        let (dr, dg, db, dy) = wheel_hsl_to_yrgb_delta(w, GAIN_CHROMA_SCALE, GAIN_LUMA_SCALE);
        gain[0] += dr;
        gain[1] += dg;
        gain[2] += db;
        gain[3] += dy;
        has_gain = true;
    }
    if let Some(exp) = lumetri.exposure {
        if exp.is_finite() && (-5.0..=5.0).contains(&exp) && exp.abs() > 1e-6 {
            let mult = 2.0_f64.powf(exp / EXPOSURE_GAMMA);
            for ch in &mut gain {
                *ch *= mult;
            }
            has_gain = true;
        }
    }
    if let Some(whites) = lumetri.whites {
        if whites.is_finite() && (-100.0..=100.0).contains(&whites) && whites.abs() > 1e-6 {
            let mult = (1.0 + (whites / 100.0) * WHITES_TO_GAIN_SCALE).max(0.05);
            for ch in &mut gain {
                *ch *= mult;
            }
            has_gain = true;
        }
    }
    if has_gain && gain.iter().any(|v| (v - 1.0).abs() > 1e-6) {
        params.insert(DRX_GAIN_R_ID, gain[0].clamp(0.01, 16.0) as f32);
        params.insert(DRX_GAIN_G_ID, gain[1].clamp(0.01, 16.0) as f32);
        params.insert(DRX_GAIN_B_ID, gain[2].clamp(0.01, 16.0) as f32);
        params.insert(DRX_GAIN_Y_ID, gain[3].clamp(0.01, 16.0) as f32);
    }

    // 4. Gamma wheel = Midtones Color Wheel (DRX = ui * 4.0, neutral 0.0)
    if let Some(w) = lumetri.midtones_wheel.filter(|w| !w.is_neutral()) {
        let (dr, dg, db, dy) = wheel_hsl_to_yrgb_delta(w, GAMMA_CHROMA_SCALE, GAMMA_LUMA_SCALE);
        if [dr, dg, db, dy].iter().any(|v| v.abs() > 1e-6) {
            params.insert(DRX_GAMMA_R_ID, (dr * 4.0) as f32);
            params.insert(DRX_GAMMA_G_ID, (dg * 4.0) as f32);
            params.insert(DRX_GAMMA_B_ID, (db * 4.0) as f32);
            params.insert(DRX_GAMMA_Y_ID, (dy * 4.0) as f32);
        }
    }

    // 5. Offset RGB (preserved for Resolve round-trips, DRX = (ui - 25.0) / 25.0)
    if let Some([or, og, ob]) = lumetri.offset_rgb {
        if [or, og, ob].iter().all(|v| v.is_finite())
            && [or, og, ob].iter().any(|v| v.abs() > 1e-6)
        {
            params.insert(DRX_OFFSET_R_ID, or.clamp(-4.0, 4.0) as f32);
            params.insert(DRX_OFFSET_G_ID, og.clamp(-4.0, 4.0) as f32);
            params.insert(DRX_OFFSET_B_ID, ob.clamp(-4.0, 4.0) as f32);
        }
    }

    // 6. Contrast (Premiere -100..100 -> Resolve 0.5..1.5 neutral 1.0)
    if let Some(con) = lumetri.contrast {
        if con.is_finite() && (-100.0..=100.0).contains(&con) && con.abs() > 1e-6 {
            params.insert(
                DRX_CONTRAST_ID,
                (1.0 + con * CONTRAST_PREMIERE_TO_RESOLVE).clamp(0.0, 2.0) as f32,
            );
        }
    }

    // 7. Primary adjustment bars: Highlights, Shadows, Color Boost (Vibrance), Mid/Detail (Sharpen), Temp, Tint
    if let Some(hi) = lumetri.highlights {
        if hi.is_finite() && hi.abs() > 1e-6 {
            params.insert(
                DRX_HIGHLIGHTS_ID,
                (hi * SHADOWS_HIGHLIGHTS_SCALE).clamp(-100.0, 100.0) as f32,
            );
        }
    }
    if let Some(sh) = lumetri.shadows {
        if sh.is_finite() && sh.abs() > 1e-6 {
            params.insert(
                DRX_SHADOWS_ID,
                (sh * SHADOWS_HIGHLIGHTS_SCALE).clamp(-100.0, 100.0) as f32,
            );
        }
    }
    if let Some(vib) = lumetri.vibrance {
        if vib.is_finite() && vib.abs() > 1e-6 {
            params.insert(DRX_COLOR_BOOST_ID, vib.clamp(-100.0, 100.0) as f32);
        }
    }
    if let Some(sharp) = lumetri.sharpen {
        if sharp.is_finite() && sharp.abs() > 1e-6 {
            params.insert(DRX_MID_DETAIL_ID, sharp.clamp(-100.0, 100.0) as f32);
        }
    }
    if let Some(temp) = lumetri.temperature {
        if temp.is_finite() && temp.abs() > 1e-6 {
            params.insert(
                DRX_TEMP_ID,
                (temp * TEMP_PREMIERE_TO_RESOLVE).clamp(-4000.0, 4000.0) as f32,
            );
        }
    }
    if let Some(tint) = lumetri.tint {
        if tint.is_finite() && tint.abs() > 1e-6 {
            params.insert(DRX_TINT_ID, tint.clamp(-100.0, 100.0) as f32);
        }
    }

    if params.is_empty() {
        return Vec::new();
    }

    let mut container_fields = Vec::with_capacity(1 + params.len());
    container_fields.push(ProtoField::varint(1, 1));
    for (pid, val) in params {
        container_fields.push(grade_parameter(pid, val));
    }
    let param_container = encode_protobuf(&container_fields);
    let corr_params = encode_protobuf(&[ProtoField::bytes(2, param_container)]);
    let corrector = encode_protobuf(&[
        ProtoField::varint(1, 1),
        ProtoField::varint(3, 1),
        ProtoField::bytes(6, corr_params),
    ]);
    encode_protobuf(&[ProtoField::bytes(1, corrector)])
}

fn drx_node(
    node_id: u64,
    index: u64,
    total_nodes: usize,
    lumetri: &LumetriData,
    ts: u64,
) -> ProtoField {
    let label = if lumetri.display_name.is_empty() {
        format!("Lumetri {index}")
    } else if total_nodes > 1 && lumetri.display_name.eq_ignore_ascii_case("Lumetri Color") {
        format!("Lumetri Color {index}")
    } else {
        lumetri.display_name.clone()
    };
    let mut fields = vec![
        ProtoField::varint(1, node_id),
        ProtoField::varint(2, index),
        ProtoField::varint(4, 190 + (index - 1) * 270),
        ProtoField::varint(5, 180),
        ProtoField::bytes(6, label.into_bytes()),
        ProtoField::varint(7, 1),
        ProtoField::varint(8, 44),
    ];

    let primary = primary_corrector(lumetri);
    if !primary.is_empty() {
        // Exact companion construction matching Resolve DRX wire layout:
        let cp_inner = encode_protobuf(&[
            ProtoField::varint(1, 2_284_847_105),
            ProtoField::bytes(2, encode_protobuf(&[ProtoField::varint(2, 0)])),
        ]);
        let c_cont = encode_protobuf(&[
            ProtoField::varint(1, 1),
            ProtoField::bytes(3, cp_inner),
        ]);
        let companion = encode_protobuf(&[ProtoField::bytes(
            1,
            encode_protobuf(&[
                ProtoField::varint(1, 2),
                ProtoField::varint(3, 1),
                ProtoField::bytes(6, encode_protobuf(&[ProtoField::bytes(2, c_cont)])),
            ]),
        )]);
        let mut node_settings = primary;
        node_settings.extend_from_slice(&companion);
        node_settings.extend_from_slice(&encode_protobuf(&[ProtoField::bytes(
            2,
            vec![3, 4, 5, 6, 18],
        )]));
        fields.push(ProtoField::bytes(9, node_settings));
    }

    let tool = encode_protobuf(&[ProtoField::bytes(
        1,
        encode_protobuf(&[
            ProtoField::varint(1, 0xC000_0001),
            ProtoField::bytes(2, encode_protobuf(&[ProtoField::varint(2, 2)])),
        ]),
    )]);
    fields.push(ProtoField::bytes(10, tool));
    fields.push(ProtoField::varint(12, ts));
    ProtoField::bytes(7, encode_protobuf(&fields))
}

/// Build DaVinci Resolve native DRX grade `Body` bytes (`0x80` + protobuf) from ordered Lumetri effects.
pub fn build_lumetri_grade_body(effects: &[LumetriData], width: u32, height: u32) -> Option<Vec<u8>> {
    build_grade_and_stabilization_body(effects, false, width, height)
}

/// Build DaVinci Resolve native DRX `Body` bytes (`0x80` + protobuf) combining optional
/// Lumetri Color serial nodes (`f1`) and/or activated Inspector Stabilization (`f3`, corrector `10`).
pub fn build_grade_and_stabilization_body(
    effects: &[LumetriData],
    enable_stabilization: bool,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    if effects.is_empty() && !enable_stabilization {
        return None;
    }
    let w = width.max(1) as u64;
    let h = height.max(1) as u64;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1_760_000_000);

    let res = ProtoField::bytes(
        3,
        encode_protobuf(&[
            ProtoField::varint(1, w),
            ProtoField::varint(2, h),
            ProtoField::f32_le(3, 1.0),
            ProtoField::varint(4, w),
            ProtoField::varint(5, h),
            ProtoField::f32_le(6, 1.0),
            ProtoField::varint(7, w),
            ProtoField::varint(8, h),
            ProtoField::varint(9, 0xFFFF_FFFF),
        ]),
    );

    let f1_bytes = if effects.is_empty() {
        // Minimal grade graph header matching Resolve 21.2 when only Inspector Stabilization is active
        encode_protobuf(&[
            ProtoField::varint(2, 1),
            res,
            ProtoField::varint(12, ts),
        ])
    } else {
        // Reserve marker IDs to avoid collisions
        let mut ids: Vec<u64> = Vec::with_capacity(effects.len());
        let mut candidate = 2u64;
        while ids.len() < effects.len() {
            if !matches!(candidate, 8 | 17 | 24) {
                ids.push(candidate);
            }
            candidate += 1;
        }

        let mut fields = vec![
            ProtoField::varint(1, *ids.last().unwrap()),
            ProtoField::varint(2, 1),
            res,
        ];
        for (idx, (&nid, eff)) in ids.iter().zip(effects.iter()).enumerate() {
            fields.push(drx_node(nid, (idx + 1) as u64, effects.len(), eff, ts));
        }
        for i in 0..ids.len().saturating_sub(1) {
            let edge_body = encode_protobuf(&[
                ProtoField::varint(1, ids[i]),
                ProtoField::varint(3, ids[i + 1]),
                ProtoField::varint(5, 64),
                ProtoField::varint(6, 64),
                ProtoField::varint(7, (3 + i) as u64),
            ]);
            fields.push(ProtoField::bytes(8, edge_body));
        }

        let first_id = ids[0];
        let last_id = *ids.last().unwrap();
        let input_id = first_id + 6;
        let input_inner = encode_protobuf(&[
            ProtoField::varint(1, first_id + 22),
            ProtoField::varint(2, 64),
            ProtoField::varint(3, input_id),
            ProtoField::varint(4, first_id),
        ]);
        fields.push(ProtoField::bytes(
            9,
            encode_protobuf(&[
                ProtoField::varint(1, input_id),
                ProtoField::varint(2, 80),
                ProtoField::bytes(3, input_inner),
            ]),
        ));

        let output_id = last_id + 7;
        let output_inner = encode_protobuf(&[
            ProtoField::varint(1, last_id + 23),
            ProtoField::varint(2, 64),
            ProtoField::varint(3, output_id),
            ProtoField::varint(4, last_id),
        ]);
        fields.push(ProtoField::bytes(
            10,
            encode_protobuf(&[
                ProtoField::varint(1, output_id),
                ProtoField::varint(2, 64),
                ProtoField::bytes(3, output_inner),
            ]),
        ));

        fields.push(ProtoField::varint(11, 1));
        fields.push(ProtoField::varint(12, ts));
        encode_protobuf(&fields)
    };

    let stab_settings = if enable_stabilization {
        // Resolve 21.2 Inspector Stabilization (Corrector 10): Perspective mode enabled, ready for Stabilize analysis.
        let mode_param = encode_protobuf(&[
            ProtoField::varint(1, DRX_STAB_MODE_PARAM_ID),
            ProtoField::bytes(2, encode_protobuf(&[ProtoField::varint(2, 8)])),
        ]);
        let opts_param = encode_protobuf(&[
            ProtoField::varint(1, DRX_STAB_OPTS_PARAM_ID),
            ProtoField::bytes(2, encode_protobuf(&[ProtoField::varint(4, 0)])),
        ]);
        let param_container = encode_protobuf(&[
            ProtoField::varint(1, 1),
            ProtoField::bytes(3, mode_param),
            ProtoField::bytes(3, opts_param),
        ]);
        let corr_params = encode_protobuf(&[ProtoField::bytes(2, param_container)]);
        let corrector = encode_protobuf(&[
            ProtoField::varint(1, DRX_STAB_CORRECTOR_ID),
            ProtoField::varint(3, 1),
            ProtoField::bytes(6, corr_params),
        ]);
        encode_protobuf(&[
            ProtoField::bytes(1, corrector),
            ProtoField::bytes(2, vec![0x06]),
        ])
    } else {
        Vec::new()
    };

    let root_meta = encode_protobuf(&[
        ProtoField::varint(1, 0xC000_0001),
        ProtoField::bytes(2, encode_protobuf(&[ProtoField::varint(2, 2)])),
    ]);
    let root_pos = encode_protobuf(&[
        ProtoField::varint(4, 190),
        ProtoField::varint(5, 180),
        ProtoField::varint(7, 1),
        ProtoField::varint(8, 84),
        ProtoField::bytes(9, stab_settings),
        ProtoField::bytes(10, encode_protobuf(&[ProtoField::bytes(1, root_meta)])),
        ProtoField::varint(12, ts),
    ]);

    let mut top = vec![ProtoField::bytes(1, f1_bytes)];
    if enable_stabilization {
        top.push(ProtoField::varint(2, 1));
    }
    top.push(ProtoField::bytes(3, root_pos));
    let proto = encode_protobuf(&top);

    let mut body = Vec::with_capacity(1 + proto.len());
    body.push(0x80);
    body.extend_from_slice(&proto);
    Some(body)
}

fn unwrap_drx_body(body_hex: &str) -> Option<Vec<u8>> {
    let raw = hex_decode(body_hex).ok()?;
    if raw.len() < 2 {
        return None;
    }
    match raw[0] {
        0x80 => Some(raw[1..].to_vec()),
        0x81 => {
            let mut dec = ruzstd::StreamingDecoder::new(&raw[1..]).ok()?;
            let mut out = Vec::new();
            dec.read_to_end(&mut out).ok()?;
            Some(out)
        }
        _ => None,
    }
}

/// Decode Resolve `<pLmVerTable><Body>` (`0x80` raw or `0x81` zstd) Primary Corrector serial nodes
/// into ordered `LumetriData` structs (Primaries – Color Wheels ↔ Lumetri Basic + Color Wheels).
pub fn parse_resolve_grade_nodes(body_hex: &str) -> Vec<LumetriData> {
    let Some(proto_bytes) = unwrap_drx_body(body_hex) else {
        return Vec::new();
    };
    let Ok(top) = decode_protobuf(&proto_bytes) else {
        return Vec::new();
    };
    let Some(f1_bytes) = top.iter().find_map(|f| {
        if f.number == 1 {
            if let ProtoValue::Bytes(b) = &f.value {
                return Some(b.as_slice());
            }
        }
        None
    }) else {
        return Vec::new();
    };
    let Ok(graph_fields) = decode_protobuf(f1_bytes) else {
        return Vec::new();
    };

    let mut raw_nodes: Vec<(u64, LumetriData)> = Vec::new();
    for gf in graph_fields {
        if gf.number != 7 {
            continue;
        }
        let ProtoValue::Bytes(node_bytes) = gf.value else {
            continue;
        };
        if let Some((idx, data)) = parse_drx_primary_node(&node_bytes) {
            raw_nodes.push((idx, data));
        }
    }

    raw_nodes.sort_by_key(|(idx, _)| *idx);
    raw_nodes
        .into_iter()
        .enumerate()
        .map(|(order, (_, mut d))| {
            d.order = order;
            if d.display_name.is_empty() {
                d.display_name = "Lumetri Color".to_string();
            }
            d
        })
        .collect()
}

fn parse_drx_primary_node(node_bytes: &[u8]) -> Option<(u64, LumetriData)> {
    let node_fields = decode_protobuf(node_bytes).ok()?;
    let mut index = 0u64;
    let mut label = String::new();
    let mut f9_bytes: Option<&[u8]> = None;

    for nf in &node_fields {
        match (nf.number, &nf.value) {
            (2, ProtoValue::Varint(v)) => index = *v,
            (6, ProtoValue::Bytes(b)) => {
                if let Ok(s) = std::str::from_utf8(b) {
                    label = s.trim().to_string();
                }
            }
            (9, ProtoValue::Bytes(b)) => f9_bytes = Some(b.as_slice()),
            _ => {}
        }
    }

    let settings = f9_bytes?;
    let params = extract_primary_corrector_params(settings);
    if params.is_empty() {
        return None;
    }

    let mut data = LumetriData {
        display_name: if label.is_empty() {
            "Lumetri Color".to_string()
        } else {
            label
        },
        ..Default::default()
    };

    if let Some(&sat) = params.get(&DRX_SATURATION_ID) {
        let pr_sat = (sat * 100.0).clamp(0.0, 200.0);
        if (pr_sat - 100.0).abs() > 1e-4 {
            data.saturation = Some(pr_sat);
        }
    }
    if let Some(&con) = params.get(&DRX_CONTRAST_ID) {
        let pr_con = ((con - 1.0) / CONTRAST_PREMIERE_TO_RESOLVE).clamp(-100.0, 100.0);
        if pr_con.abs() > 1e-4 {
            data.contrast = Some(pr_con);
        }
    }
    if let Some(&temp) = params.get(&DRX_TEMP_ID) {
        let pr_temp = (temp / TEMP_PREMIERE_TO_RESOLVE).clamp(-100.0, 100.0);
        if pr_temp.abs() > 1e-4 {
            data.temperature = Some(pr_temp);
        }
    }
    if let Some(&tint) = params.get(&DRX_TINT_ID) {
        let pr_tint = tint.clamp(-100.0, 100.0);
        if pr_tint.abs() > 1e-4 {
            data.tint = Some(pr_tint);
        }
    }
    if let Some(&hi) = params.get(&DRX_HIGHLIGHTS_ID) {
        let pr_hi = (hi / SHADOWS_HIGHLIGHTS_SCALE).clamp(-100.0, 100.0);
        if pr_hi.abs() > 1e-4 {
            data.highlights = Some(pr_hi);
        }
    }
    if let Some(&sh) = params.get(&DRX_SHADOWS_ID) {
        let pr_sh = (sh / SHADOWS_HIGHLIGHTS_SCALE).clamp(-100.0, 100.0);
        if pr_sh.abs() > 1e-4 {
            data.shadows = Some(pr_sh);
        }
    }
    if let Some(&cb) = params.get(&DRX_COLOR_BOOST_ID) {
        let pr_vib = cb.clamp(-100.0, 100.0);
        if pr_vib.abs() > 1e-4 {
            data.vibrance = Some(pr_vib);
        }
    }
    if let Some(&md) = params.get(&DRX_MID_DETAIL_ID) {
        let pr_sharp = md.clamp(-100.0, 100.0);
        if pr_sharp.abs() > 1e-4 {
            data.sharpen = Some(pr_sharp);
        }
    }

    // Lift (DRX = ui * 2.0, neutral 0.0) -> Shadows Color Wheel
    if [DRX_LIFT_R_ID, DRX_LIFT_G_ID, DRX_LIFT_B_ID, DRX_LIFT_Y_ID]
        .iter()
        .any(|id| params.contains_key(id))
    {
        let r = params.get(&DRX_LIFT_R_ID).copied().unwrap_or(0.0) / 2.0;
        let g = params.get(&DRX_LIFT_G_ID).copied().unwrap_or(0.0) / 2.0;
        let b = params.get(&DRX_LIFT_B_ID).copied().unwrap_or(0.0) / 2.0;
        let y = params
            .get(&DRX_LIFT_Y_ID)
            .copied()
            .map(|v| v / 2.0)
            .unwrap_or_else(|| LUMA_R * r + LUMA_G * g + LUMA_B * b);
        let w = yrgb_delta_to_wheel(r, g, b, y, LIFT_CHROMA_SCALE, LIFT_LUMA_SCALE);
        if !w.is_neutral() {
            data.shadows_wheel = Some(w);
        }
    }

    // Gamma (DRX = ui * 4.0, neutral 0.0) -> Midtones Color Wheel
    if [DRX_GAMMA_R_ID, DRX_GAMMA_G_ID, DRX_GAMMA_B_ID, DRX_GAMMA_Y_ID]
        .iter()
        .any(|id| params.contains_key(id))
    {
        let r = params.get(&DRX_GAMMA_R_ID).copied().unwrap_or(0.0) / 4.0;
        let g = params.get(&DRX_GAMMA_G_ID).copied().unwrap_or(0.0) / 4.0;
        let b = params.get(&DRX_GAMMA_B_ID).copied().unwrap_or(0.0) / 4.0;
        let y = params
            .get(&DRX_GAMMA_Y_ID)
            .copied()
            .map(|v| v / 4.0)
            .unwrap_or_else(|| LUMA_R * r + LUMA_G * g + LUMA_B * b);
        let w = yrgb_delta_to_wheel(r, g, b, y, GAMMA_CHROMA_SCALE, GAMMA_LUMA_SCALE);
        if !w.is_neutral() {
            data.midtones_wheel = Some(w);
        }
    }

    // Gain (DRX = ui * 1.0, neutral 1.0) -> Exposure (if achromatic) and/or Highlights Color Wheel
    let has_gain_rgb = [DRX_GAIN_R_ID, DRX_GAIN_G_ID, DRX_GAIN_B_ID]
        .iter()
        .any(|id| params.contains_key(id));
    let has_gain_y = params.contains_key(&DRX_GAIN_Y_ID);
    if has_gain_rgb || has_gain_y {
        let y_raw = params.get(&DRX_GAIN_Y_ID).copied();
        let r = params.get(&DRX_GAIN_R_ID).copied().unwrap_or_else(|| y_raw.unwrap_or(1.0));
        let g = params.get(&DRX_GAIN_G_ID).copied().unwrap_or_else(|| y_raw.unwrap_or(1.0));
        let b = params.get(&DRX_GAIN_B_ID).copied().unwrap_or_else(|| y_raw.unwrap_or(1.0));
        let y = y_raw.unwrap_or_else(|| LUMA_R * r + LUMA_G * g + LUMA_B * b);

        let chroma_span = r.max(g).max(b) - r.min(g).min(b);
        if chroma_span < 1e-4 {
            // Pure master exposure adjustment (inverse of 2^(exp / EXPOSURE_GAMMA))
            if (y - 1.0).abs() > 1e-4 && y > 0.0 {
                data.exposure = Some((y.log2() * EXPOSURE_GAMMA).clamp(-5.0, 5.0));
            }
        } else if (0.875..=1.125).contains(&y) {
            let w = yrgb_delta_to_wheel(
                r - 1.0,
                g - 1.0,
                b - 1.0,
                y - 1.0,
                GAIN_CHROMA_SCALE,
                GAIN_LUMA_SCALE,
            );
            if !w.is_neutral() {
                data.highlights_wheel = Some(w);
            }
        } else {
            // Large overall gain shift combined with highlight chroma tint: split exposure & wheel
            let y_safe = y.max(0.01);
            data.exposure = Some((y_safe.log2() * EXPOSURE_GAMMA).clamp(-5.0, 5.0));
            let w = yrgb_delta_to_wheel(
                r / y_safe - 1.0,
                g / y_safe - 1.0,
                b / y_safe - 1.0,
                0.0,
                GAIN_CHROMA_SCALE,
                GAIN_LUMA_SCALE,
            );
            if !w.is_neutral() {
                data.highlights_wheel = Some(w);
            }
        }
    }

    // Offset (DRX = (ui - 25.0) / 25.0, neutral 0.0)
    if [DRX_OFFSET_R_ID, DRX_OFFSET_G_ID, DRX_OFFSET_B_ID]
        .iter()
        .any(|id| params.contains_key(id))
    {
        let or = params.get(&DRX_OFFSET_R_ID).copied().unwrap_or(0.0);
        let og = params.get(&DRX_OFFSET_G_ID).copied().unwrap_or(0.0);
        let ob = params.get(&DRX_OFFSET_B_ID).copied().unwrap_or(0.0);
        if or.abs() > 1e-6 || og.abs() > 1e-6 || ob.abs() > 1e-6 {
            data.offset_rgb = Some([or, og, ob]);
        }
    }

    data.has_primary_adjustments().then_some((index, data))
}

fn extract_primary_corrector_params(f9_bytes: &[u8]) -> BTreeMap<u64, f64> {
    let mut out = BTreeMap::new();
    let Ok(f9_fields) = decode_protobuf(f9_bytes) else {
        return out;
    };
    for sf in f9_fields {
        if sf.number != 1 {
            continue;
        }
        let ProtoValue::Bytes(corr_bytes) = sf.value else {
            continue;
        };
        let Ok(corr_fields) = decode_protobuf(&corr_bytes) else {
            continue;
        };
        let is_primary = corr_fields
            .iter()
            .any(|cf| cf.number == 1 && matches!(cf.value, ProtoValue::Varint(1)));
        if !is_primary {
            continue;
        }
        let is_disabled = corr_fields
            .iter()
            .any(|cf| cf.number == 3 && matches!(cf.value, ProtoValue::Varint(0)));
        if is_disabled {
            continue;
        }
        for cf in corr_fields {
            if cf.number != 6 {
                continue;
            }
            let ProtoValue::Bytes(f6_bytes) = cf.value else {
                continue;
            };
            let Ok(f6_fields) = decode_protobuf(&f6_bytes) else {
                continue;
            };
            for pcf in f6_fields {
                if pcf.number != 2 {
                    continue;
                }
                let ProtoValue::Bytes(cont_bytes) = pcf.value else {
                    continue;
                };
                let Ok(cont_fields) = decode_protobuf(&cont_bytes) else {
                    continue;
                };
                for entry in cont_fields {
                    if entry.number != 3 {
                        continue;
                    }
                    let ProtoValue::Bytes(eb) = entry.value else {
                        continue;
                    };
                    let Ok(ef) = decode_protobuf(&eb) else {
                        continue;
                    };
                    let mut pid = None;
                    let mut val = None;
                    for item in ef {
                        match (item.number, item.value) {
                            (1, ProtoValue::Varint(id)) => pid = Some(id),
                            (2, ProtoValue::Bytes(vb)) => {
                                if let Ok(vf) = decode_protobuf(&vb) {
                                    for vitem in vf {
                                        if vitem.number == 1 {
                                            if let ProtoValue::Fixed32(arr) = vitem.value {
                                                let f = f32::from_le_bytes(arr) as f64;
                                                if f.is_finite() {
                                                    val = Some(f);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if let (Some(id), Some(v)) = (pid, val) {
                        out.insert(id, v);
                    }
                }
            }
        }
    }
    out
}

/// Inspect a Resolve `<pLmVerTable><Body>` hex string (`0x80` raw or `0x81` zstd)
/// and return `true` if Inspector Stabilization (node `f3`, corrector `f9.f1.f1 == 10`) is present.
pub fn has_resolve_stabilization(body_hex: &str) -> bool {
    let Some(proto_bytes) = unwrap_drx_body(body_hex) else {
        return false;
    };
    let Ok(top) = decode_protobuf(&proto_bytes) else {
        return false;
    };
    for f in top {
        if f.number != 3 {
            continue;
        }
        let ProtoValue::Bytes(node_bytes) = f.value else {
            continue;
        };
        let Ok(node_fields) = decode_protobuf(&node_bytes) else {
            continue;
        };
        for nf in node_fields {
            if nf.number != 9 {
                continue;
            }
            let ProtoValue::Bytes(f9_bytes) = nf.value else {
                continue;
            };
            let Ok(f9_fields) = decode_protobuf(&f9_bytes) else {
                continue;
            };
            for sf in f9_fields {
                if sf.number != 1 {
                    continue;
                }
                let ProtoValue::Bytes(corr_bytes) = sf.value else {
                    continue;
                };
                if let Ok(corr_fields) = decode_protobuf(&corr_bytes) {
                    if corr_fields
                        .iter()
                        .any(|cf| cf.number == 1 && matches!(cf.value, ProtoValue::Varint(DRX_STAB_CORRECTOR_ID)))
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Build `<pLmVerTable>` XML block attaching an active native DRX grade version onto a `<Sm2TiVideoClip>`.
pub fn build_version_table_xml(grade_body: &[u8]) -> String {
    let table_id = Uuid::new_v4();
    let ver_id = Uuid::new_v4();
    let body_hex = hex_encode(grade_body);
    format!(
        "<pLmVerTable><ListMgt::LmVersionTable DbId=\"{table_id}\"><FieldsBlob>{VERSION_TABLE_FIELDS_BLOB}</FieldsBlob><VerType>0</VerType><pActive>{ver_id}</pActive><Locals><Element><ListMgt::LmVersion DbId=\"{ver_id}\"><FieldsBlob/><Name>Version 1</Name><HasCorrection>true</HasCorrection><VerType>0</VerType><ImplVersion>1</ImplVersion><IncludedInRecording>true</IncludedInRecording><FlatPassEnabled>false</FlatPassEnabled><RGBAOutputEnabled>false</RGBAOutputEnabled><Body>{body_hex}</Body><UseVersionClipProcParams>true</UseVersionClipProcParams></ListMgt::LmVersion></Element></Locals></ListMgt::LmVersionTable></pLmVerTable>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_wheel_yrgb_roundtrips_across_all_hue_sectors() {
        for hue in [0.0, 35.0, 60.0, 120.0, 163.1, 180.0, 231.8, 240.0, 303.4, 355.0] {
            let orig = ColorWheel {
                hue_deg: hue,
                luma: 0.43,
                saturation: 0.27,
            };
            let (dr, dg, db, dy) = wheel_hsl_to_yrgb_delta(orig, 0.25, 0.35);
            // Verify Rec.709 luminance preservation
            let check_y = LUMA_R * dr + LUMA_G * dg + LUMA_B * db;
            assert!((check_y - dy).abs() < 1e-12, "luma drift at hue {hue}");

            let back = yrgb_delta_to_wheel(dr, dg, db, dy, 0.25, 0.35);
            assert!(
                (back.hue_deg - orig.hue_deg).abs() < 1e-6,
                "hue mismatch: orig={} back={}",
                orig.hue_deg,
                back.hue_deg
            );
            assert!((back.luma - orig.luma).abs() < 1e-9);
            assert!((back.saturation - orig.saturation).abs() < 1e-9);
        }
    }

    #[test]
    fn drx_multi_node_primaries_and_wheels_roundtrip() {
        let node1 = LumetriData {
            order: 0,
            display_name: "Lumetri Color".into(),
            shadows_wheel: Some(ColorWheel {
                hue_deg: 303.37,
                luma: 0.439,
                saturation: 0.239,
            }),
            midtones_wheel: Some(ColorWheel {
                hue_deg: 264.65,
                luma: 0.563,
                saturation: 0.116,
            }),
            highlights_wheel: Some(ColorWheel {
                hue_deg: 136.18,
                luma: 0.458,
                saturation: 0.264,
            }),
            ..Default::default()
        };
        let node2 = LumetriData {
            order: 1,
            display_name: "Lumetri Color".into(),
            temperature: Some(-3.4),
            tint: Some(5.26),
            exposure: Some(0.42),
            contrast: Some(6.71),
            highlights: Some(-4.64),
            shadows: Some(7.74),
            saturation: Some(108.36),
            vibrance: Some(12.0),
            sharpen: Some(4.0),
            ..Default::default()
        };

        let body = build_lumetri_grade_body(&[node1, node2], 1920, 1080).expect("build DRX body");
        let parsed = parse_resolve_grade_nodes(&hex_encode(&body));
        assert_eq!(parsed.len(), 2);

        // Node 1: disambiguated label + 3 color wheels preserved accurately
        assert_eq!(parsed[0].display_name, "Lumetri Color 1");
        let sw = parsed[0].shadows_wheel.expect("lift -> shadows_wheel");
        assert!((sw.hue_deg - 303.37).abs() < 0.05);
        assert!((sw.luma - 0.439).abs() < 1e-3);
        assert!((sw.saturation - 0.239).abs() < 1e-3);
        let mw = parsed[0].midtones_wheel.expect("gamma -> midtones_wheel");
        assert!((mw.hue_deg - 264.65).abs() < 0.05);
        let hw = parsed[0].highlights_wheel.expect("gain -> highlights_wheel");
        assert!((hw.hue_deg - 136.18).abs() < 0.05);

        // Node 2: Basic Correction + Creative bars preserved accurately
        assert_eq!(parsed[1].display_name, "Lumetri Color 2");
        assert!((parsed[1].temperature.unwrap() - (-3.4)).abs() < 0.01);
        assert!((parsed[1].tint.unwrap() - 5.26).abs() < 0.01);
        assert!((parsed[1].exposure.unwrap() - 0.42).abs() < 0.01);
        assert!((parsed[1].contrast.unwrap() - 6.71).abs() < 0.01);
        assert!((parsed[1].highlights.unwrap() - (-4.64)).abs() < 0.01);
        assert!((parsed[1].shadows.unwrap() - 7.74).abs() < 0.01);
        assert!((parsed[1].saturation.unwrap() - 108.36).abs() < 0.05);
        assert!((parsed[1].vibrance.unwrap() - 12.0).abs() < 0.01);
        assert!((parsed[1].sharpen.unwrap() - 4.0).abs() < 0.01);
    }
}
