use base64::Engine as _;
use nle_core::LumetriData;
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

        let scalar = child_text(param, "StartKeyframe").and_then(|raw| parse_keyframe_scalar(&raw));
        match pid.as_str() {
            "11" => data.exposure = scalar,
            "12" => data.contrast = scalar,
            "20" => data.saturation = scalar,
            "7" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.temperature = Some(v);
                    data.unmapped_controls.push("Temperature".into());
                }
            }
            "8" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.tint = Some(v);
                    data.unmapped_controls.push("Tint".into());
                }
            }
            "13" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.highlights = Some(v);
                    data.unmapped_controls.push("Highlights".into());
                }
            }
            "14" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.shadows = Some(v);
                    data.unmapped_controls.push("Shadows".into());
                }
            }
            "15" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.whites = Some(v);
                    data.unmapped_controls.push("Whites".into());
                }
            }
            "16" => {
                if let Some(v) = scalar.filter(|v| v.abs() > 1e-6) {
                    data.blacks = Some(v);
                    data.unmapped_controls.push("Blacks".into());
                }
            }
            _ => {
                if is_tv && !pname.is_empty() && !data.unmapped_controls.contains(&pname) {
                    data.unmapped_controls.push(pname);
                }
            }
        }
    }

    data
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
            if !sname.is_empty()
                && sname != "BasicCorrection3"
                && !data.unmapped_controls.contains(&sname)
            {
                data.unmapped_controls.push(sname);
            }
        }
    }
}

pub(crate) fn parse_keyframe_scalar(raw: &str) -> Option<f64> {
    let part = raw.split(',').nth(1).unwrap_or(raw).trim();
    part.parse::<f64>().ok().filter(|v| v.is_finite())
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
