pub mod fcp7xml;
pub mod fcpbundle;
pub mod fcpxml;
pub mod otio;

pub use fcp7xml::{write_fcp7_xml_file, Fcp7XmlWriter};
pub use fcpbundle::{read_fcpbundle_file, FcpBundleReader};
pub use fcpxml::{
    parse_fcpxml_str, read_fcpxml_file, write_fcpxml_file, FcpXmlReader, FcpXmlWriter,
};
pub use otio::{write_otio_file, OtioWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::*;
    use std::collections::BTreeMap;

    fn sample_interchange_project() -> Project {
        Project {
            name: "InterchangeFidelity".into(),
            source_format: NleFormat::PremierePro,
            sequences: vec![Sequence {
                id: "s1".into(),
                name: "Main".into(),
                kind: SequenceKind::Timeline,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 150,
                start_tc_frame: 0,
                mark_in: None,
                mark_out: None,
                video_tracks: vec![Track {
                    index: 1,
                    kind: TrackKind::Video,
                    name: "V1".into(),
                    enabled: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    audio_channels: 0,
                    output_bus_id: None,
                    clips: vec![
                        Clip {
                            id: "c1".into(),
                            name: "SlowMo.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Media/SlowMo.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 0,
                            end_frame: 50,
                            source_in_frame: 0,
                            source_out_frame: 25,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 1920,
                            source_height: 1080,
                            speed_percent: 50.0,
                            reversed: false,
                            speed_keyframes: vec![],
                            retime_interpolation: RetimeInterpolation::OpticalFlow,
                            enabled: true,
                            audio_channel: 0,
                            link_group_id: None,
                            label_name: None,
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            blend_mode: BlendMode::Normal,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![Marker {
                                frame: 10,
                                duration_frames: 5,
                                name: "Beat".into(),
                                comment: "Slow down here".into(),
                                color: "Green".into(),
                                scope: MarkerScope::Clip,
                            }],
                        },
                        Clip {
                            id: "c2".into(),
                            name: "FreezeDisabled.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Media/SlowMo.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 50,
                            end_frame: 100,
                            source_in_frame: 25,
                            source_out_frame: 25,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 1920,
                            source_height: 1080,
                            speed_percent: 0.0,
                            reversed: false,
                            speed_keyframes: vec![],
                            retime_interpolation: RetimeInterpolation::Default,
                            enabled: false,
                            audio_channel: 0,
                            link_group_id: None,
                            label_name: None,
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            blend_mode: BlendMode::Normal,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![],
                        },
                    ],
                    transitions: vec![Transition {
                        name: "Cross Dissolve".into(),
                        effect_id: "AE.ADBE Cross Dissolve".into(),
                        start_frame: 40,
                        end_frame: 60,
                        alignment: "center".into(),
                    }],
                    markers: vec![],
                }],
                audio_tracks: vec![],
                audio_buses: vec![],
                markers: vec![Marker {
                    frame: 25,
                    duration_frames: 1,
                    name: "Seq Marker".into(),
                    comment: "Notes".into(),
                    color: "Blue".into(),
                    scope: MarkerScope::Sequence,
                }],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn otio_exports_transitions_clip_markers_timewarps_and_respects_include_markers() {
        let proj = sample_interchange_project();
        let dir = std::env::temp_dir().join("aagedal_nle_otio_fidelity_test");
        let _ = std::fs::create_dir_all(&dir);

        let otio_with = dir.join("with_markers.otio");
        let rep = write_otio_file(&proj, &otio_with, &ConversionOptions::default()).unwrap();
        assert_eq!(rep.markers_written, 2);
        let text = std::fs::read_to_string(&otio_with).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let children = parsed["tracks"]["children"][0]["children"].as_array().unwrap();
        assert!(children.iter().any(|c| c["OTIO_SCHEMA"] == "Transition.1"));
        assert!(text.contains("LinearTimeWarp.1"));
        assert!(text.contains("FreezeFrame.1"));
        assert!(text.contains("Slow down here"));

        let otio_no_mk = dir.join("no_markers.otio");
        let opts_no = ConversionOptions {
            include_markers: false,
            ..Default::default()
        };
        let rep_no = write_otio_file(&proj, &otio_no_mk, &opts_no).unwrap();
        assert_eq!(rep_no.markers_written, 0);
        let text_no = std::fs::read_to_string(&otio_no_mk).unwrap();
        assert!(!text_no.contains("Seq Marker"));
        assert!(!text_no.contains("Slow down here"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fcp7xml_exports_transitions_speed_disabled_clips_and_respects_include_markers() {
        let proj = sample_interchange_project();
        let dir = std::env::temp_dir().join("aagedal_nle_fcp7_fidelity_test");
        let _ = std::fs::create_dir_all(&dir);

        let xml_with = dir.join("with_markers.xml");
        let rep = write_fcp7_xml_file(&proj, &xml_with, &ConversionOptions::default()).unwrap();
        assert_eq!(rep.markers_written, 2);
        let text = std::fs::read_to_string(&xml_with).unwrap();
        assert!(text.contains("<transitionitem>"));
        assert!(text.contains("<enabled>FALSE</enabled>"));
        assert!(text.contains("<effectid>timeremap</effectid>"));
        assert!(text.contains("<value>50.00</value>"));
        assert!(text.contains("<name>Beat</name>"));

        let xml_no = dir.join("no_markers.xml");
        let opts_no = ConversionOptions {
            include_markers: false,
            ..Default::default()
        };
        let rep_no = write_fcp7_xml_file(&proj, &xml_no, &opts_no).unwrap();
        assert_eq!(rep_no.markers_written, 0);
        let text_no = std::fs::read_to_string(&xml_no).unwrap();
        assert!(!text_no.contains("<marker>"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
