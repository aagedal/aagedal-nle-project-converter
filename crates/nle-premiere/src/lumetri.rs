use base64::Engine as _;
use nle_core::{ColorWheel, LumetriData};
use roxmltree::Node;
use std::collections::HashMap;

/// Parse a Premiere `AE.ADBE Lumetri` component node into `LumetriData`.
pub fn parse_lumetri_component(
    component: Node<'_, '_>,
    order: usize,
    by_id: &HashMap<&str, Node<'_, '_>>,
) -> LumetriData {
    let display_name = child_text(component, "InstanceName")
        .or_else(|| nested_text(component, &["Component", "DisplayName"]))
        .or_else(|| child_text(component, "Name"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Lumetri Color {}", order + 1));

    let mut data = LumetriData {
        order,
        display_name,
        ..Default::default()
    };

    let Some(params_node) = find_descendant(component, "Params") else {
        return data;
    };

    let mut basic_enabled = true;
    let mut creative_enabled = true;
    let mut wheels_enabled = true;

    for param_ref in params_node.children().filter(|n| n.has_tag_name("Param")) {
        let Some(oid) = param_ref.attribute("ObjectRef") else {
            continue;
        };
        let Some(param) = by_id.get(oid).copied() else {
            continue;
        };

        let pid = child_text(param, "ParameterID").unwrap_or_default();
        let pname = child_text(param, "Name").unwrap_or_default();
        let is_tv = child_text(param, "IsTimeVarying")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
            || child_text(param, "Keyframes")
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
        if is_tv {
            data.is_animated = true;
        }

        if pid == "1" {
            if let Some(blob_b64) = child_text(param, "StartKeyframeValue") {
                inspect_lumetri_blob(&blob_b64, &mut data);
            }
            continue;
        }

        // Premiere Pro Color Wheels & Match arbitrary parameter:
        // 72 bytes (3 wheels) or 96 bytes (4 wheels incl. HDR Specular) of little-endian f64 triplets:
        // (hue_deg [0..360), luma [0..1] neutral 0.5, saturation [0..1] neutral 0.0)
        // in order: [Shadows, Midtones, Highlights, (optional) Specular].
        if pid == "47" {
            if let Some(wheel_b64) = child_text(param, "StartKeyframeValue") {
                if let Some([sh, mid, hi]) = decode_color_wheels_param47(&wheel_b64) {
                    if !sh.is_neutral() {
                        data.shadows_wheel = Some(sh);
                    }
                    if !mid.is_neutral() {
                        data.midtones_wheel = Some(mid);
                    }
                    if !hi.is_neutral() {
                        data.highlights_wheel = Some(hi);
                    }
                }
            }
            continue;
        }

        let raw_kf = child_text(param, "StartKeyframe");
        let bool_val = raw_kf.as_deref().and_then(parse_keyframe_bool);
        let scalar = raw_kf.as_deref().and_then(parse_keyframe_scalar);

        match pid.as_str() {
            // Section enable toggles
            "3" => {
                if let Some(b) = bool_val {
                    basic_enabled = b;
                }
            }
            "23" => {
                if let Some(b) = bool_val {
                    creative_enabled = b;
                }
            }
            "46" => {
                if let Some(b) = bool_val {
                    wheels_enabled = b;
                }
            }
            // Basic Correction (all natively mapped to Resolve Primaries - Color Wheels)
            "7" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.temperature = Some(v);
                }
            }
            "8" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.tint = Some(v);
                }
            }
            "11" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.exposure = Some(v);
                }
            }
            "12" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.contrast = Some(v);
                }
            }
            "13" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.highlights = Some(v);
                }
            }
            "14" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.shadows = Some(v);
                }
            }
            "15" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.whites = Some(v);
                }
            }
            "16" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.blacks = Some(v);
                }
            }
            "20" => {
                if let Some(v) = scalar.filter(|v| (v - 100.0).abs() > 1e-6) {
                    data.saturation = Some(v);
                }
            }
            // Creative adjustments that map directly to Resolve Primary bars
            "29" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.sharpen = Some(v);
                }
            }
            "30" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.vibrance = Some(v);
                }
            }
            _ => {
                if is_tv && !pname.is_empty() && !data.unmapped_controls.contains(&pname) {
                    data.unmapped_controls.push(pname);
                }
            }
        }
    }

    if !basic_enabled {
        data.temperature = None;
        data.tint = None;
        data.exposure = None;
        data.contrast = None;
        data.highlights = None;
        data.shadows = None;
        data.whites = None;
        data.blacks = None;
        data.saturation = None;
    }
    if !creative_enabled {
        data.sharpen = None;
        data.vibrance = None;
    }
    if !wheels_enabled {
        data.shadows_wheel = None;
        data.midtones_wheel = None;
        data.highlights_wheel = None;
    }

    data
}

/// Decode Premiere `AE.ADBE Lumetri` `ParameterID=47` base64 payload into
/// `[Shadows, Midtones, Highlights]` `ColorWheel`s.
pub fn decode_color_wheels_param47(b64: &str) -> Option<[ColorWheel; 3]> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    if bytes.len() < 72 || bytes.len() % 8 != 0 {
        return None;
    }
    let read_f64 = |idx: usize| -> Option<f64> {
        let start = idx * 8;
        let arr: [u8; 8] = bytes.get(start..start + 8)?.try_into().ok()?;
        let v = f64::from_le_bytes(arr);
        v.is_finite().then_some(v)
    };
    let read_wheel = |wheel_idx: usize| -> Option<ColorWheel> {
        let base = wheel_idx * 3;
        Some(ColorWheel {
            hue_deg: read_f64(base)?.rem_euclid(360.0),
            luma: read_f64(base + 1)?.clamp(0.0, 1.0),
            saturation: read_f64(base + 2)?.clamp(0.0, 1.0),
        })
    };
    Some([read_wheel(0)?, read_wheel(1)?, read_wheel(2)?])
}

/// Encode `[Shadows, Midtones, Highlights]` color wheels into Premiere Pro 2026's 96-byte
/// `ParameterID=47` base64 payload (4 wheels including neutral HDR Specular).
pub fn encode_color_wheels_param47(
    shadows: Option<ColorWheel>,
    midtones: Option<ColorWheel>,
    highlights: Option<ColorWheel>,
) -> String {
    let wheels = [
        shadows.unwrap_or_default(),
        midtones.unwrap_or_default(),
        highlights.unwrap_or_default(),
        ColorWheel::default(), // HDR Specular (neutral)
    ];
    let mut bytes = Vec::with_capacity(96);
    for w in wheels {
        bytes.extend_from_slice(&w.hue_deg.rem_euclid(360.0).to_le_bytes());
        bytes.extend_from_slice(&w.luma.clamp(0.0, 1.0).to_le_bytes());
        bytes.extend_from_slice(&w.saturation.clamp(0.0, 1.0).to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn inspect_lumetri_blob(b64: &str, data: &mut LumetriData) {
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) else {
        return;
    };
    let Ok(xml_str) = std::str::from_utf8(&bytes) else {
        return;
    };
    let Ok(doc) = roxmltree::Document::parse(xml_str) else {
        return;
    };
    let root = doc.root_element();
    if root.tag_name().name() != "Lumetri" {
        return;
    }
    let kfs: Vec<_> = root.children().filter(|n| n.has_tag_name("keyframe")).collect();
    if kfs.len() > 1 {
        data.is_animated = true;
    }
    for kf in kfs {
        for shader in kf.children().filter(|n| n.has_tag_name("shader")) {
            let sname = child_text(shader, "name")
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            if sname == "Wheels" {
                parse_legacy_wheels_shader(shader, data);
                continue;
            }
            if !sname.is_empty()
                && !matches!(
                    sname.as_str(),
                    "BasicCorrection"
                        | "BasicCorrection2"
                        | "BasicCorrection3"
                        | "__GroupBegin"
                        | "__GroupEnd"
                        | "Linearize"
                )
                && !data.unmapped_controls.contains(&sname)
            {
                data.unmapped_controls.push(sname);
            }
        }
    }
}

/// Fallback parser for legacy Lumetri XML blobs storing `<shader><name>"Wheels"</name>`
/// with `<Shadows.X/Y/Z>`, `<Midtones.X/Y/Z>`, `<Highlights.X/Y/Z>` RGB offsets in `[-1, 1]`.
fn parse_legacy_wheels_shader(shader: Node<'_, '_>, data: &mut LumetriData) {
    let Some(params) = shader.children().find(|n| n.has_tag_name("parameters")) else {
        return;
    };
    let get_val = |tag: &str| -> f64 {
        child_text(params, tag)
            .map(|s| {
                let t = s.trim().trim_matches('"');
                t.strip_prefix('D')
                    .or_else(|| t.strip_prefix('N'))
                    .unwrap_or(t)
                    .parse::<f64>()
                    .unwrap_or(0.0)
            })
            .unwrap_or(0.0)
    };
    let to_wheel = |prefix: &str| -> Option<ColorWheel> {
        let r = get_val(&format!("{prefix}.X"));
        let g = get_val(&format!("{prefix}.Y"));
        let b = get_val(&format!("{prefix}.Z"));
        if r.abs() < 1e-6 && g.abs() < 1e-6 && b.abs() < 1e-6 {
            return None;
        }
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let cr = r - y;
        let cg = g - y;
        let cb = b - y;
        let cmax = cr.max(cg).max(cb);
        let cmin = cr.min(cg).min(cb);
        let delta = cmax - cmin;
        let hue_deg = if delta < 1e-6 {
            0.0
        } else if (cmax - cr).abs() < 1e-9 {
            60.0 * (((cg - cb) / delta).rem_euclid(6.0))
        } else if (cmax - cg).abs() < 1e-9 {
            60.0 * (((cb - cr) / delta) + 2.0)
        } else {
            60.0 * (((cr - cg) / delta) + 4.0)
        };
        let w = ColorWheel {
            hue_deg: hue_deg.rem_euclid(360.0),
            luma: (0.5 + y * 0.5).clamp(0.0, 1.0),
            saturation: delta.clamp(0.0, 1.0),
        };
        (!w.is_neutral()).then_some(w)
    };
    if data.shadows_wheel.is_none() {
        data.shadows_wheel = to_wheel("Shadows");
    }
    if data.midtones_wheel.is_none() {
        data.midtones_wheel = to_wheel("Midtones");
    }
    if data.highlights_wheel.is_none() {
        data.highlights_wheel = to_wheel("Highlights");
    }
}

pub(crate) fn parse_keyframe_scalar(raw: &str) -> Option<f64> {
    let part = raw.split(',').nth(1).unwrap_or(raw).trim();
    part.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub(crate) fn parse_keyframe_bool(raw: &str) -> Option<bool> {
    let part = raw.split(',').nth(1).unwrap_or(raw).trim();
    if part.eq_ignore_ascii_case("true") {
        Some(true)
    } else if part.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(|s| s.to_string())
}

pub(crate) fn nested_text(node: Node<'_, '_>, path: &[&str]) -> Option<String> {
    let mut cur = node;
    for &seg in path {
        cur = cur.children().find(|n| n.has_tag_name(seg))?;
    }
    cur.text().map(|s| s.to_string())
}

pub(crate) fn find_descendant<'a, 'input>(
    node: Node<'a, 'input>,
    tag: &str,
) -> Option<Node<'a, 'input>> {
    node.descendants().find(|n| n.has_tag_name(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_wheels_param47_roundtrips() {
        let sh = ColorWheel {
            hue_deg: 170.52,
            luma: 0.5295,
            saturation: 0.1019,
        };
        let mid = ColorWheel {
            hue_deg: 163.11,
            luma: 0.4670,
            saturation: 0.2829,
        };
        let hi = ColorWheel {
            hue_deg: 231.83,
            luma: 0.4204,
            saturation: 0.2627,
        };
        let b64 = encode_color_wheels_param47(Some(sh), Some(mid), Some(hi));
        let [d_sh, d_mid, d_hi] = decode_color_wheels_param47(&b64).expect("decode param 47");
        assert!((d_sh.hue_deg - sh.hue_deg).abs() < 1e-9);
        assert!((d_sh.luma - sh.luma).abs() < 1e-9);
        assert!((d_sh.saturation - sh.saturation).abs() < 1e-9);
        assert!((d_mid.hue_deg - mid.hue_deg).abs() < 1e-9);
        assert!((d_mid.luma - mid.luma).abs() < 1e-9);
        assert!((d_mid.saturation - mid.saturation).abs() < 1e-9);
        assert!((d_hi.hue_deg - hi.hue_deg).abs() < 1e-9);
        assert!((d_hi.luma - hi.luma).abs() < 1e-9);
        assert!((d_hi.saturation - hi.saturation).abs() < 1e-9);
    }

    #[test]
    fn respects_section_enable_toggles() {
        let wheels_b64 = encode_color_wheels_param47(
            Some(ColorWheel {
                hue_deg: 300.0,
                luma: 0.44,
                saturation: 0.25,
            }),
            None,
            None,
        );
        let xml = format!(
            r#"<Root>
                <VideoComponentParam ObjectID="p3"><ParameterID>3</ParameterID><Name> </Name><StartKeyframe>0,false</StartKeyframe></VideoComponentParam>
                <VideoComponentParam ObjectID="p7"><ParameterID>7</ParameterID><Name>Temperature</Name><StartKeyframe>0,15.0</StartKeyframe></VideoComponentParam>
                <VideoComponentParam ObjectID="p46"><ParameterID>46</ParameterID><Name> </Name><StartKeyframe>0,true</StartKeyframe></VideoComponentParam>
                <ArbVideoComponentParam ObjectID="p47"><ParameterID>47</ParameterID><Name> </Name><StartKeyframeValue>{wheels_b64}</StartKeyframeValue></ArbVideoComponentParam>
                <VideoFilterComponent ObjectID="c1">
                    <MatchName>AE.ADBE Lumetri</MatchName>
                    <Component><DisplayName>Lumetri Color</DisplayName><Params>
                        <Param ObjectRef="p3"/><Param ObjectRef="p7"/>
                        <Param ObjectRef="p46"/><Param ObjectRef="p47"/>
                    </Params></Component>
                </VideoFilterComponent>
            </Root>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let mut by_id = HashMap::new();
        for n in doc.descendants() {
            if let Some(id) = n.attribute("ObjectID") {
                by_id.insert(id, n);
            }
        }
        let parsed = parse_lumetri_component(by_id["c1"], 0, &by_id);
        // Basic Correction was disabled (pid 3 = false), Color Wheels was enabled (pid 46 = true)
        assert!(parsed.temperature.is_none());
        let sw = parsed.shadows_wheel.expect("shadows wheel parsed");
        assert!((sw.hue_deg - 300.0).abs() < 1e-6);
        assert!((sw.luma - 0.44).abs() < 1e-6);
        assert!((sw.saturation - 0.25).abs() < 1e-6);
    }
}
