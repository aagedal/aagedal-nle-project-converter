use crate::binary::{encode_protobuf, hex_encode, ProtoField};
use nle_core::LumetriData;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const DRX_GAIN_MASTER_ID: u64 = 100_663_328;
const DRX_SATURATION_ID: u64 = 100_663_301;
const DRX_LOG_CONTRAST_ID: u64 = 2_248_147_137;
const VERSION_TABLE_FIELDS_BLOB: &str = "00000001000000010000002200410063007400690076006500560065007200730069006f006e0054007900700065000000020000";

fn grade_parameter(param_id: u64, value: f32) -> ProtoField {
    let val_msg = encode_protobuf(&[ProtoField::f32_le(1, value)]);
    let entry = encode_protobuf(&[ProtoField::varint(1, param_id), ProtoField::bytes(2, val_msg)]);
    ProtoField::bytes(3, entry)
}

fn primary_corrector(lumetri: &LumetriData) -> Vec<u8> {
    let mut entries = Vec::new();
    if let Some(exp) = lumetri.exposure {
        if exp.is_finite() && (-5.0..=5.0).contains(&exp) && exp.abs() > 1e-6 {
            let gain = 2.0_f64.powf(exp) as f32;
            entries.push(grade_parameter(DRX_GAIN_MASTER_ID, gain));
        }
    }
    if let Some(sat) = lumetri.saturation {
        if sat.is_finite() && (0.0..=200.0).contains(&sat) && (sat - 100.0).abs() > 1e-6 {
            let resolve_sat_norm = ((sat / 2.0) / 50.0) as f32;
            entries.push(grade_parameter(DRX_SATURATION_ID, resolve_sat_norm));
        }
    }
    if let Some(con) = lumetri.contrast {
        if con.is_finite() && (-100.0..=100.0).contains(&con) && con.abs() > 1e-6 {
            let resolve_contrast = (1.0 + con / 100.0) as f32;
            entries.push(grade_parameter(DRX_LOG_CONTRAST_ID, resolve_contrast));
        }
    }
    if entries.is_empty() {
        return Vec::new();
    }
    let mut container_fields = vec![ProtoField::varint(1, 1)];
    container_fields.extend(entries);
    let param_container = encode_protobuf(&container_fields);
    let corr_params = encode_protobuf(&[ProtoField::bytes(2, param_container)]);
    let corrector = encode_protobuf(&[
        ProtoField::varint(1, 1),
        ProtoField::varint(3, 1),
        ProtoField::bytes(6, corr_params),
    ]);
    encode_protobuf(&[ProtoField::bytes(1, corrector)])
}

fn drx_node(node_id: u64, index: u64, lumetri: &LumetriData, ts: u64) -> ProtoField {
    let label = if lumetri.display_name.is_empty() {
        format!("Lumetri {index}")
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
        let companion_param = encode_protobuf(&[ProtoField::bytes(
            3,
            encode_protobuf(&[
                ProtoField::varint(1, 2_284_847_105),
                ProtoField::bytes(2, encode_protobuf(&[ProtoField::varint(2, 0)])),
            ]),
        )]);
        let companion_container = encode_protobuf(&[
            ProtoField::varint(1, 1),
            ProtoField::bytes(3, companion_param), // wait: keep exact wire bytes below
        ]);
        let _ = companion_container;
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
    if effects.is_empty() {
        return None;
    }
    let w = width.max(1) as u64;
    let h = height.max(1) as u64;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1_760_000_000);

    // Reserve marker IDs to avoid collisions
    let mut ids: Vec<u64> = Vec::with_capacity(effects.len());
    let mut candidate = 2u64;
    while ids.len() < effects.len() {
        if !matches!(candidate, 8 | 17 | 24) {
            ids.push(candidate);
        }
        candidate += 1;
    }

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

    let mut fields = vec![
        ProtoField::varint(1, *ids.last().unwrap()),
        ProtoField::varint(2, 1),
        res,
    ];
    for (idx, (&nid, eff)) in ids.iter().zip(effects.iter()).enumerate() {
        fields.push(drx_node(nid, (idx + 1) as u64, eff, ts));
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

    let root_meta = encode_protobuf(&[
        ProtoField::varint(1, 0xC000_0001),
        ProtoField::bytes(2, encode_protobuf(&[ProtoField::varint(2, 2)])),
    ]);
    let root_pos = encode_protobuf(&[
        ProtoField::varint(4, 190),
        ProtoField::varint(5, 180),
        ProtoField::varint(7, 1),
        ProtoField::varint(8, 84),
        ProtoField::bytes(9, Vec::new()),
        ProtoField::bytes(10, encode_protobuf(&[ProtoField::bytes(1, root_meta)])),
        ProtoField::varint(12, ts),
    ]);

    let proto = encode_protobuf(&[
        ProtoField::bytes(1, encode_protobuf(&fields)),
        ProtoField::bytes(3, root_pos),
    ]);
    let mut body = Vec::with_capacity(1 + proto.len());
    body.push(0x80);
    body.extend_from_slice(&proto);
    Some(body)
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
