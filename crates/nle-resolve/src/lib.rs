pub mod audio;
pub mod binary;
pub mod grade;
pub mod reader;
pub mod state;
pub mod timemap;
pub mod writer;

pub use reader::{read_drp_file, ResolveReader};
pub use writer::{write_drp_file, ResolveWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::*;
    use std::collections::BTreeMap;

    #[test]
    fn write_and_read_drp_roundtrip() {
        let project = Project {
            name: "Roundtrip Test".into(),
            source_format: NleFormat::PremierePro,
            source_path: None,
            color_config: ColorConfig {
                working_space: Some("Rec. 709".into()),
                output_space: Some("Rec. 709".into()),
            },
            scale_to_frame_size: true,
            bins: vec![ProjectBin {
                id: "b1".into(),
                name: "01_Rushes".into(),
                parent_id: None,
                media_paths: vec!["/Media/A001.mov".into()],
                sequence_ids: vec!["seq-1".into()],
                custom_metadata: BTreeMap::new(),
            }],
            media_assets: vec![MediaAsset {
                id: "m1".into(),
                name: "A001.mov".into(),
                path: "/Media/A001.mov".into(),
                media_kind: MediaKind::Video,
                width: 3840,
                height: 2160,
                frame_rate: Rational::FPS_25,
                duration_frames: 500,
                source_tc_frame: 90_000,
                source_tc_string: "01:00:00:00".into(),
                audio_channels: 2,
                audio_sample_rate: 48_000,
                color_space: None,
                video_codec: Some("apch".into()),
                audio_codec: Some("Linear PCM".into()),
                alpha_mode: AlphaMode::Default,
                data_levels: DataLevels::Auto,
                pixel_aspect_ratio: None,
                metadata: BTreeMap::from([
                    ("Description".into(), "Studio interview A-cam".into()),
                    ("Comments".into(), "Best take".into()),
                    ("Scene".into(), "12A".into()),
                ]),
                custom_metadata: BTreeMap::from([
                    ("External.Item.Id".into(), "11111111-2222-3333-4444-555555555555".into()),
                    ("External.Artifact".into(), r#"{"type":"Segment","id":"abc"}"#.into()),
                ]),
            }],
            sequences: vec![Sequence {
                id: "seq-1".into(),
                name: "Edit_01".into(),
                kind: SequenceKind::Timeline,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 150,
                start_tc_frame: 0,
                mark_in: Some(10),
                mark_out: Some(100),
                video_tracks: vec![Track {
                    index: 1,
                    kind: TrackKind::Video,
                    name: "Main V1".into(),
                    enabled: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    audio_channels: 0,
                    output_bus_id: None,
                    clips: vec![Clip {
                        id: "c1".into(),
                        name: "A001.mov".into(),
                        media_id: "m1".into(),
                        media_path: "/Media/A001.mov".into(),
                        media_kind: MediaKind::Video,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 150,
                        source_in_frame: 25,
                        source_out_frame: 175,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: Rational::FPS_25,
                        source_width: 3840,
                        source_height: 2160,
                        speed_percent: 100.0,
                        reversed: false,
                        speed_keyframes: vec![],
                        retime_interpolation: RetimeInterpolation::OpticalFlow,
                        enabled: true,
                        audio_channel: 1,
                        link_group_id: None,
                        label_name: Some("BE.Prefs.LabelColors.8".into()),
                        label_preset_index: Some(8),
                        motion: MotionTransform {
                            scale: 50.0,
                            rotation: 2.5,
                            ..Default::default()
                        },
                        opacity: 100.0,
                        blend_mode: BlendMode::Normal,
                        lumetri_effects: vec![LumetriData {
                            order: 0,
                            display_name: "Lumetri Color".into(),
                            exposure: Some(0.5),
                            contrast: Some(15.0),
                            saturation: Some(110.0),
                            ..Default::default()
                        }],
                        effects: vec![],
                        markers: vec![],
                    }],
                    transitions: vec![],
                    markers: vec![],
                }],
                audio_tracks: vec![Track {
                    index: 1,
                    kind: TrackKind::Audio,
                    name: "Dialogue".into(),
                    enabled: true,
                    locked: true,
                    muted: false,
                    solo: false,
                    audio_channels: 1,
                    output_bus_id: Some("bus-1".into()),
                    clips: vec![Clip {
                        id: "a1".into(),
                        name: "A001.mov".into(),
                        media_id: "m1".into(),
                        media_path: "/Media/A001.mov".into(),
                        media_kind: MediaKind::Audio,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 150,
                        source_in_frame: 25,
                        source_out_frame: 175,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: Rational::FPS_25,
                        source_width: 0,
                        source_height: 0,
                        speed_percent: 100.0,
                        reversed: false,
                        speed_keyframes: vec![],
                        retime_interpolation: RetimeInterpolation::OpticalFlow,
                        enabled: true,
                        audio_channel: 1,
                        link_group_id: None,
                        label_name: None,
                        label_preset_index: None,
                        motion: MotionTransform::default(),
                        opacity: 100.0,
                        blend_mode: BlendMode::Normal,
                        lumetri_effects: vec![],
                        effects: vec![],
                        markers: vec![],
                    }],
                    transitions: vec![],
                    markers: vec![],
                }],
                audio_buses: vec![
                    AudioBus {
                        id: "bus-0".into(),
                        name: "MAIN MIX".into(),
                        channels: 2,
                        is_main: true,
                        parent_bus_id: None,
                    },
                    AudioBus {
                        id: "bus-1".into(),
                        name: "DIALOGUE".into(),
                        channels: 2,
                        is_main: false,
                        parent_bus_id: Some("bus-0".into()),
                    },
                    AudioBus {
                        id: "bus-2".into(),
                        name: "MUSIC".into(),
                        channels: 2,
                        is_main: false,
                        parent_bus_id: Some("bus-0".into()),
                    },
                ],
                markers: vec![Marker {
                    frame: 50,
                    duration_frames: 25,
                    name: "Check Grade".into(),
                    comment: "Lumetri translated".into(),
                    color: "Purple".into(),
                    scope: MarkerScope::Sequence,
                }],
                metadata: BTreeMap::new(),
            }],
            metadata: BTreeMap::new(),
            custom_metadata_schema: BTreeMap::new(),
        };

        let out_path = std::env::temp_dir().join("aagedal_nle_roundtrip_test.drp");
        let report = write_drp_file(&project, &out_path, &ConversionOptions::default())
            .expect("write native .drp");
        assert_eq!(report.sequences_converted, 1);
        assert_eq!(report.video_clips_converted, 1);
        assert_eq!(report.audio_clips_converted, 1);
        assert_eq!(report.grades_attached, 1);
        assert_eq!(report.markers_written, 1);
        assert_eq!(report.bins_created, 1);

        let reread = read_drp_file(&out_path).expect("read back generated .drp");
        assert_eq!(reread.name, "Roundtrip Test");
        assert_eq!(reread.sequences.len(), 1);
        assert_eq!(reread.sequences[0].name, "Edit_01");
        assert_eq!(reread.sequences[0].width, 1920);
        assert_eq!(reread.sequences[0].height, 1080);
        assert_eq!(reread.sequences[0].frame_rate, Rational::FPS_25);
        assert_eq!(reread.sequences[0].video_tracks[0].clips.len(), 1);
        assert_eq!(reread.sequences[0].video_tracks[0].clips[0].name, "A001.mov");
        assert_eq!(reread.sequences[0].video_tracks[0].clips[0].label_preset_index, Some(8));
        assert_eq!(reread.sequences[0].markers.len(), 1);
        assert_eq!(reread.sequences[0].markers[0].name, "Check Grade");
        assert_eq!(reread.sequences[0].markers[0].color, "Purple");
        assert!(reread.sequences[0].audio_tracks[0].locked);
        assert_eq!(reread.sequences[0].audio_tracks[0].audio_channels, 1);
        assert_eq!(reread.sequences[0].audio_buses.len(), 3);
        assert_eq!(reread.sequences[0].audio_buses[0].name, "MAIN MIX");
        assert_eq!(reread.sequences[0].audio_buses[1].name, "DIALOGUE");
        assert_eq!(reread.sequences[0].audio_buses[2].name, "MUSIC");
        assert_eq!(
            reread.sequences[0].audio_tracks[0].output_bus_id.as_deref(),
            Some("bus-1")
        );
        assert_eq!(reread.media_assets.len(), 1);
        assert_eq!(reread.media_assets[0].video_codec.as_deref(), Some("apch"));
        assert_eq!(
            reread.media_assets[0].audio_codec.as_deref(),
            Some("Linear PCM")
        );
        assert_eq!(
            reread.sequences[0].video_tracks[0].clips[0].retime_interpolation,
            RetimeInterpolation::OpticalFlow
        );
        assert_eq!(
            reread.media_assets[0].metadata.get("Description").map(String::as_str),
            Some("Studio interview A-cam")
        );
        assert_eq!(
            reread.media_assets[0].metadata.get("Scene").map(String::as_str),
            Some("12A")
        );
        assert_eq!(
            reread.media_assets[0]
                .custom_metadata
                .get("External.Item.Id")
                .map(String::as_str),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(
            reread.media_assets[0]
                .custom_metadata
                .get("External.Artifact")
                .map(String::as_str),
            Some(r#"{"type":"Segment","id":"abc"}"#)
        );

        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn roundtrips_start_tc_offset_and_transitions() {
        let project = Project {
            name: "TC & Transitions".into(),
            source_format: NleFormat::DaVinciResolve,
            sequences: vec![Sequence {
                id: "seq-tc".into(),
                name: "Timeline 1".into(),
                kind: SequenceKind::Timeline,
                width: 3840,
                height: 2160,
                frame_rate: Rational::FPS_24,
                duration_frames: 200,
                start_tc_frame: 86_400, // 01:00:00:00 @ 24fps
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
                    clips: vec![Clip {
                        id: "v1".into(),
                        name: "ShotA.mp4".into(),
                        media_id: "m1".into(),
                        media_path: "/Media/ShotA.mp4".into(),
                        media_kind: MediaKind::Video,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 200,
                        source_in_frame: 10,
                        source_out_frame: 210,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: Rational::FPS_24,
                        source_width: 3840,
                        source_height: 2160,
                        speed_percent: 100.0,
                        reversed: false,
                        speed_keyframes: vec![],
                        retime_interpolation: RetimeInterpolation::OpticalFlow,
                        enabled: true,
                        audio_channel: 1,
                        link_group_id: None,
                        label_name: None,
                        label_preset_index: None,
                        motion: MotionTransform::default(),
                        opacity: 100.0,
                        blend_mode: BlendMode::Normal,
                        lumetri_effects: vec![],
                        effects: vec![],
                        markers: vec![],
                    }],
                    transitions: vec![Transition {
                        name: "Cross Dissolve".into(),
                        effect_id: "Cross Dissolve".into(),
                        start_frame: 88,
                        end_frame: 112,
                        alignment: "center".into(),
                    }],
                    markers: vec![],
                }],
                audio_tracks: vec![Track {
                    index: 1,
                    kind: TrackKind::Audio,
                    name: "A1".into(),
                    enabled: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    audio_channels: 1,
                    output_bus_id: None,
                    clips: vec![],
                    transitions: vec![Transition {
                        name: "Constant Power".into(),
                        effect_id: "Constant Power".into(),
                        start_frame: 0,
                        end_frame: 12,
                        alignment: "start".into(),
                    }],
                    markers: vec![],
                }],
                audio_buses: vec![],
                markers: vec![],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        let out_path = std::env::temp_dir().join("aagedal_nle_tc_trans_test.drp");
        write_drp_file(&project, &out_path, &ConversionOptions::default()).expect("write .drp");

        let reread = read_drp_file(&out_path).expect("re-read .drp");
        let seq = &reread.sequences[0];
        assert_eq!(seq.start_tc_frame, 86_400);
        assert_eq!(seq.computed_duration_frames(), 200);
        assert_eq!(seq.video_tracks[0].clips[0].start_frame, 0);
        assert_eq!(seq.video_tracks[0].transitions.len(), 1);
        assert_eq!(seq.video_tracks[0].transitions[0].name, "Cross Dissolve");
        assert_eq!(seq.video_tracks[0].transitions[0].start_frame, 88);
        assert_eq!(seq.video_tracks[0].transitions[0].end_frame, 112);
        assert_eq!(seq.audio_tracks[0].transitions.len(), 1);
        assert_eq!(seq.audio_tracks[0].transitions[0].name, "Cross Fade +3DB");
        assert_eq!(seq.audio_tracks[0].transitions[0].alignment, "start");

        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn merges_same_frame_markers_and_builds_sequence_setup() {
        let markers = vec![
            Marker {
                frame: 10,
                duration_frames: 20,
                name: "FX A".into(),
                comment: "Detail A".into(),
                color: "Purple".into(),
                scope: MarkerScope::Sequence,
            },
            Marker {
                frame: 10,
                duration_frames: 50,
                name: "FX B".into(),
                comment: "Detail B".into(),
                color: "Purple".into(),
                scope: MarkerScope::Sequence,
            },
        ];
        let hex = state::encode_timeline_markers(&markers);
        let bytes = binary::hex_decode(&hex).expect("valid hex");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("FX A + FX B"));
        assert!(text.contains("Detail A\nDetail B"));

        let setup = state::build_sequence_setup_blob(1080, 1920, 25);
        assert_eq!(setup.len(), 422);
        assert_eq!(u64::from_be_bytes(setup[40..48].try_into().unwrap()), 1080);
        assert_eq!(u64::from_be_bytes(setup[48..56].try_into().unwrap()), 1920);
        assert_eq!(u64::from_be_bytes(setup[76..84].try_into().unwrap()), 25);
    }

    #[test]
    fn timemap_encodes_and_decodes_forward_reverse_and_freeze() {
        let id_hex = timemap::build_media_timemap(250, 25.0, 100.0, false);
        assert_eq!(timemap::decode_media_timemap(&id_hex), Some((100.0, false)));

        let slow_hex = timemap::build_media_timemap(250, 25.0, 50.0, false);
        let (spd, rev) = timemap::decode_media_timemap(&slow_hex).expect("decode 50%");
        assert!((spd - 50.0).abs() < 1e-4);
        assert!(!rev);

        let fast_hex = timemap::build_media_timemap(250, 25.0, 200.0, true);
        let (spd, rev) = timemap::decode_media_timemap(&fast_hex).expect("decode 200%");
        assert!((spd - 200.0).abs() < 1e-4);
        assert!(!rev);

        let rev_hex = timemap::build_media_timemap(250, 25.0, -100.0, false);
        let (spd, rev) = timemap::decode_media_timemap(&rev_hex).expect("decode -100%");
        assert!((spd - (-100.0)).abs() < 1e-4);
        assert!(rev);

        let freeze_hex = timemap::build_media_timemap(250, 25.0, 0.0, false);
        let (spd, rev) = timemap::decode_media_timemap(&freeze_hex).expect("decode 0%");
        assert!(spd.abs() < 1e-6);
        assert!(!rev);
    }

    #[test]
    fn decodes_resolve21_zstd_0x81_envelope() {
        // Minimal valid zstd frame (uncompressed block) containing "Camera 3" (8 bytes):
        // magic: 28 b5 2f fd, fhd: 20 (single segment, 1B fcs), fcs: 08, block header: 01 00 00 -> raw block len 8, last=1: (8<<3)|1 = 0x41 00 00
        let mut zstd_frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x08, 0x41, 0x00, 0x00];
        zstd_frame.extend_from_slice(b"Camera 3");
        let mut envelope = Vec::new();
        envelope.extend_from_slice(&2u32.to_be_bytes());
        envelope.extend_from_slice(&((zstd_frame.len() + 1) as u32).to_be_bytes());
        envelope.push(0x81);
        envelope.extend_from_slice(&zstd_frame);

        let decoded = binary::decode_wrapper_hex(&binary::hex_encode(&envelope))
            .expect("decode 0x81 zstd envelope");
        assert_eq!(decoded, b"Camera 3");
    }

    #[test]
    fn roundtrips_nested_multicam_linked_clips_opacity_and_reverse() {
        let project = Project {
            name: "Nested & Multicam RT".into(),
            source_format: NleFormat::PremierePro,
            media_assets: vec![MediaAsset {
                id: "m1".into(),
                name: "CamA.mov".into(),
                path: "/Media/CamA.mov".into(),
                media_kind: MediaKind::Video,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 500,
                source_tc_frame: 0,
                source_tc_string: "00:00:00:00".into(),
                audio_channels: 2,
                audio_sample_rate: 48_000,
                color_space: None,
                video_codec: None,
                audio_codec: None,
                alpha_mode: AlphaMode::Default,
                data_levels: DataLevels::Auto,
                pixel_aspect_ratio: None,
                metadata: BTreeMap::new(),
                custom_metadata: BTreeMap::new(),
            }],
            sequences: vec![
                Sequence {
                    id: "mc-src".into(),
                    name: "MC_3Cam".into(),
                    kind: SequenceKind::MulticamClip,
                    width: 1920,
                    height: 1080,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 250,
                    start_tc_frame: 0,
                    mark_in: None,
                    mark_out: None,
                    video_tracks: vec![Track {
                        index: 1,
                        kind: TrackKind::Video,
                        name: "Camera 1".into(),
                        enabled: true,
                        locked: false,
                        muted: false,
                        solo: false,
                        audio_channels: 0,
                        output_bus_id: None,
                        clips: vec![Clip {
                            id: "ang1".into(),
                            name: "CamA.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Media/CamA.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 0,
                            end_frame: 250,
                            source_in_frame: 0,
                            source_out_frame: 250,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 1920,
                            source_height: 1080,
                            speed_percent: 100.0,
                            reversed: false,
                            speed_keyframes: vec![],
                            retime_interpolation: RetimeInterpolation::Default,
                            enabled: true,
                            audio_channel: 1,
                            link_group_id: None,
                            label_name: None,
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            blend_mode: BlendMode::Normal,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![],
                        }],
                        transitions: vec![],
                        markers: vec![],
                    }],
                    audio_tracks: vec![],
                    audio_buses: vec![],
                    markers: vec![],
                    metadata: BTreeMap::new(),
                },
                Sequence {
                    id: "main-edit".into(),
                    name: "Main_Edit".into(),
                    kind: SequenceKind::Timeline,
                    width: 1920,
                    height: 1080,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 200,
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
                                id: "v-mc".into(),
                                name: "MC_3Cam".into(),
                                media_id: "mc-src".into(),
                                media_path: String::new(),
                                media_kind: MediaKind::Video,
                                source_kind: ClipSourceKind::MulticamClip,
                                active_angle: Some("Camera 2".into()),
                                start_frame: 0,
                                end_frame: 100,
                                source_in_frame: 0,
                                source_out_frame: 100,
                                source_in_ticks: None,
                                source_out_ticks: None,
                                source_fps: Rational::FPS_25,
                                source_width: 1920,
                                source_height: 1080,
                                speed_percent: 100.0,
                                reversed: false,
                                speed_keyframes: vec![],
                                retime_interpolation: RetimeInterpolation::Default,
                                enabled: true,
                                audio_channel: 1,
                                link_group_id: Some("link-0".into()),
                                label_name: None,
                                label_preset_index: None,
                                motion: MotionTransform::default(),
                                opacity: 85.0,
                                blend_mode: BlendMode::Normal,
                                lumetri_effects: vec![],
                                effects: vec![],
                                markers: vec![],
                            },
                            Clip {
                                id: "v-rev".into(),
                                name: "CamA.mov".into(),
                                media_id: "m1".into(),
                                media_path: "/Media/CamA.mov".into(),
                                media_kind: MediaKind::Video,
                                source_kind: ClipSourceKind::Media,
                                active_angle: None,
                                start_frame: 100,
                                end_frame: 200,
                                source_in_frame: 50,
                                source_out_frame: 150,
                                source_in_ticks: None,
                                source_out_ticks: None,
                                source_fps: Rational::FPS_25,
                                source_width: 1920,
                                source_height: 1080,
                                speed_percent: -100.0,
                                reversed: true,
                                speed_keyframes: vec![],
                                retime_interpolation: RetimeInterpolation::Default,
                                enabled: true,
                                audio_channel: 1,
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
                        transitions: vec![],
                        markers: vec![],
                    }],
                    audio_tracks: vec![Track {
                        index: 1,
                        kind: TrackKind::Audio,
                        name: "A1".into(),
                        enabled: true,
                        locked: false,
                        muted: false,
                        solo: false,
                        audio_channels: 0,
                        output_bus_id: None,
                        clips: vec![Clip {
                            id: "a-mc".into(),
                            name: "MC_3Cam".into(),
                            media_id: "mc-src".into(),
                            media_path: String::new(),
                            media_kind: MediaKind::Audio,
                            source_kind: ClipSourceKind::MulticamClip,
                            active_angle: Some("Camera 2".into()),
                            start_frame: 0,
                            end_frame: 100,
                            source_in_frame: 0,
                            source_out_frame: 100,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 0,
                            source_height: 0,
                            speed_percent: 100.0,
                            reversed: false,
                            speed_keyframes: vec![],
                            retime_interpolation: RetimeInterpolation::Default,
                            enabled: true,
                            audio_channel: 1,
                            link_group_id: Some("link-0".into()),
                            label_name: None,
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            blend_mode: BlendMode::Normal,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![],
                        }],
                        transitions: vec![],
                        markers: vec![],
                    }],
                    audio_buses: vec![],
                    markers: vec![],
                    metadata: BTreeMap::new(),
                },
            ],
            ..Default::default()
        };

        let out_path = std::env::temp_dir().join("aagedal_nle_mc_nested_rt.drp");
        write_drp_file(&project, &out_path, &ConversionOptions::default()).expect("write .drp");

        let reread = read_drp_file(&out_path).expect("reread .drp");
        // No ghost media created for MC_3Cam
        assert_eq!(reread.media_assets.len(), 1);
        assert_eq!(reread.media_assets[0].path, "/Media/CamA.mov");

        let mc_seq = reread
            .sequences
            .iter()
            .find(|s| s.name == "MC_3Cam")
            .expect("multicam sequence");
        assert_eq!(mc_seq.kind, SequenceKind::MulticamClip);

        let main = reread
            .sequences
            .iter()
            .find(|s| s.name == "Main_Edit")
            .expect("main edit");
        let v_mc = &main.video_tracks[0].clips[0];
        let a_mc = &main.audio_tracks[0].clips[0];
        assert_eq!(v_mc.source_kind, ClipSourceKind::MulticamClip);
        assert_eq!(v_mc.active_angle.as_deref(), Some("Camera 2"));
        assert!((v_mc.opacity - 85.0).abs() < 1e-4);
        assert!(v_mc.link_group_id.is_some());
        assert_eq!(v_mc.link_group_id, a_mc.link_group_id);

        let v_rev = &main.video_tracks[0].clips[1];
        assert!(v_rev.reversed);
        assert!((v_rev.speed_percent - (-100.0)).abs() < 1e-4);
        // 500 - 150 = 350 reversed record-in frame
        assert_eq!(v_rev.source_in_frame, 350);

        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn native_codec_metadata_avoids_h264_aac_placeholder_leak() {
        let project = Project {
            name: "Codec Metadata Test".into(),
            source_format: NleFormat::PremierePro,
            media_assets: vec![
                MediaAsset {
                    id: "m-prores".into(),
                    name: "Interview_ProRes422HQ.mov".into(),
                    path: "/Media/Interview_ProRes422HQ.mov".into(),
                    media_kind: MediaKind::Video,
                    width: 3840,
                    height: 2160,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 250,
                    source_tc_frame: 0,
                    source_tc_string: "00:00:00:00".into(),
                    audio_channels: 2,
                    audio_sample_rate: 48_000,
                    color_space: None,
                    video_codec: None,
                    audio_codec: None,
                    alpha_mode: AlphaMode::Default,
                    data_levels: DataLevels::Auto,
                    pixel_aspect_ratio: None,
                    metadata: BTreeMap::new(),
                    custom_metadata: BTreeMap::new(),
                },
                MediaAsset {
                    id: "m-mxf".into(),
                    name: "A001C002.mxf".into(),
                    path: "/Media/A001C002.mxf".into(),
                    media_kind: MediaKind::Video,
                    width: 1920,
                    height: 1080,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 250,
                    source_tc_frame: 0,
                    source_tc_string: "00:00:00:00".into(),
                    audio_channels: 4,
                    audio_sample_rate: 48_000,
                    color_space: None,
                    video_codec: None,
                    audio_codec: None,
                    alpha_mode: AlphaMode::Default,
                    data_levels: DataLevels::Auto,
                    pixel_aspect_ratio: None,
                    metadata: BTreeMap::new(),
                    custom_metadata: BTreeMap::new(),
                },
                MediaAsset {
                    id: "m-mp3".into(),
                    name: "Music_Bed.mp3".into(),
                    path: "/Media/Music_Bed.mp3".into(),
                    media_kind: MediaKind::Audio,
                    width: 0,
                    height: 0,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 250,
                    source_tc_frame: 0,
                    source_tc_string: "00:00:00:00".into(),
                    audio_channels: 2,
                    audio_sample_rate: 44_100,
                    color_space: None,
                    video_codec: None,
                    audio_codec: None,
                    alpha_mode: AlphaMode::Default,
                    data_levels: DataLevels::Auto,
                    pixel_aspect_ratio: None,
                    metadata: BTreeMap::new(),
                    custom_metadata: BTreeMap::new(),
                },
                MediaAsset {
                    id: "m-flac".into(),
                    name: "Score_Master.flac".into(),
                    path: "/Media/Score_Master.flac".into(),
                    media_kind: MediaKind::Audio,
                    width: 0,
                    height: 0,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 250,
                    source_tc_frame: 0,
                    source_tc_string: "00:00:00:00".into(),
                    audio_channels: 1,
                    audio_sample_rate: 48_000,
                    color_space: None,
                    video_codec: None,
                    audio_codec: None,
                    alpha_mode: AlphaMode::Default,
                    data_levels: DataLevels::Auto,
                    pixel_aspect_ratio: None,
                    metadata: BTreeMap::new(),
                    custom_metadata: BTreeMap::new(),
                },
            ],
            sequences: vec![Sequence {
                id: "s1".into(),
                name: "Main".into(),
                kind: SequenceKind::Timeline,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 100,
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
                    clips: vec![Clip {
                        id: "c1".into(),
                        name: "Interview_ProRes422HQ.mov".into(),
                        media_id: "m-prores".into(),
                        media_path: "/Media/Interview_ProRes422HQ.mov".into(),
                        media_kind: MediaKind::Video,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 100,
                        source_in_frame: 0,
                        source_out_frame: 100,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: Rational::FPS_25,
                        source_width: 3840,
                        source_height: 2160,
                        speed_percent: 100.0,
                        reversed: false,
                        speed_keyframes: vec![
                            SpeedKeyframe { timeline_sec: 0.0, source_sec: 0.0 },
                            SpeedKeyframe { timeline_sec: 2.0, source_sec: 1.0 },
                            SpeedKeyframe { timeline_sec: 4.0, source_sec: 5.0 },
                        ],
                        retime_interpolation: RetimeInterpolation::FrameBlending,
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
                        markers: vec![],
                    }],
                    transitions: vec![],
                    markers: vec![],
                }],
                audio_tracks: vec![Track {
                    index: 1,
                    kind: TrackKind::Audio,
                    name: "A1".into(),
                    enabled: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    audio_channels: 1,
                    output_bus_id: None,
                    clips: vec![Clip {
                        id: "a-ch3".into(),
                        name: "Score_Master.flac".into(),
                        media_id: "m-flac".into(),
                        media_path: "/Media/Score_Master.flac".into(),
                        media_kind: MediaKind::Audio,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 100,
                        source_in_frame: 0,
                        source_out_frame: 100,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: Rational::FPS_25,
                        source_width: 0,
                        source_height: 0,
                        speed_percent: 100.0,
                        reversed: false,
                        speed_keyframes: vec![],
                        retime_interpolation: RetimeInterpolation::Default,
                        enabled: true,
                        audio_channel: 3,
                        link_group_id: None,
                        label_name: None,
                        label_preset_index: None,
                        motion: MotionTransform::default(),
                        opacity: 100.0,
                        blend_mode: BlendMode::Normal,
                        lumetri_effects: vec![],
                        effects: vec![],
                        markers: vec![],
                    }],
                    transitions: vec![],
                    markers: vec![],
                }],
                audio_buses: vec![],
                markers: vec![],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        let out_path = std::env::temp_dir().join("aagedal_nle_codec_ramp_test.drp");
        write_drp_file(&project, &out_path, &ConversionOptions::default()).expect("write .drp");

        let reread = read_drp_file(&out_path).expect("re-read .drp");
        let prores = reread.media_assets.iter().find(|a| a.name.contains("ProRes")).unwrap();
        assert_eq!(prores.video_codec.as_deref(), Some("apch"));
        assert_eq!(prores.audio_codec.as_deref(), Some("Linear PCM"));

        let mxf = reread.media_assets.iter().find(|a| a.name.ends_with(".mxf")).unwrap();
        assert_eq!(mxf.video_codec, None, "MXF must not inherit placeholder avc1");
        assert_eq!(mxf.audio_codec.as_deref(), Some("Linear PCM"));
        assert_eq!(mxf.audio_channels, 4);

        let mp3 = reread.media_assets.iter().find(|a| a.name.ends_with(".mp3")).unwrap();
        assert_eq!(mp3.audio_codec.as_deref(), Some("MP3"));
        assert_eq!(mp3.audio_sample_rate, 44_100);

        let flac = reread.media_assets.iter().find(|a| a.name.ends_with(".flac")).unwrap();
        assert_eq!(flac.audio_codec.as_deref(), Some("FLAC"));
        assert_eq!(flac.audio_channels, 1);

        // Multi-point speed ramp + FrameBlending + VirtualAudioTrackBA channel 3 roundtrip
        let vclip = &reread.sequences[0].video_tracks[0].clips[0];
        assert_eq!(vclip.speed_keyframes.len(), 3);
        assert!((vclip.speed_keyframes[1].timeline_sec - 2.0).abs() < 1e-6);
        assert!((vclip.speed_keyframes[1].source_sec - 1.0).abs() < 1e-6);
        assert_eq!(vclip.retime_interpolation, RetimeInterpolation::FrameBlending);

        let aclip = &reread.sequences[0].audio_tracks[0].clips[0];
        assert_eq!(aclip.audio_channel, 3);

        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn roundtrips_crop_clip_markers_and_disabled_clips() {
        let project = Project {
            name: "CropMarkersDisabled".into(),
            source_format: NleFormat::PremierePro,
            media_assets: vec![MediaAsset {
                id: "m1".into(),
                name: "ShotA.mov".into(),
                path: "/Media/ShotA.mov".into(),
                media_kind: MediaKind::Video,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 200,
                source_tc_frame: 0,
                source_tc_string: "00:00:00:00".into(),
                audio_channels: 2,
                audio_sample_rate: 48_000,
                color_space: None,
                video_codec: Some("apcn".into()),
                audio_codec: Some("Linear PCM".into()),
                alpha_mode: AlphaMode::Default,
                data_levels: DataLevels::Auto,
                pixel_aspect_ratio: None,
                metadata: BTreeMap::new(),
                custom_metadata: BTreeMap::new(),
            }],
            sequences: vec![Sequence {
                id: "seq-1".into(),
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
                            name: "ShotA.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Media/ShotA.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 0,
                            end_frame: 75,
                            source_in_frame: 0,
                            source_out_frame: 75,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 1920,
                            source_height: 1080,
                            speed_percent: 100.0,
                            reversed: false,
                            speed_keyframes: vec![],
                            retime_interpolation: RetimeInterpolation::Default,
                            enabled: true,
                            audio_channel: 0,
                            link_group_id: None,
                            label_name: Some("BE.Prefs.LabelColors.8".into()),
                            label_preset_index: Some(8),
                            motion: MotionTransform {
                                crop_left: 10.0,
                                crop_right: 5.0,
                                crop_top: 12.5,
                                crop_bottom: 12.5,
                                ..Default::default()
                            },
                            opacity: 100.0,
                            blend_mode: BlendMode::Normal,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![Marker {
                                frame: 20,
                                duration_frames: 5,
                                name: "Clip Note".into(),
                                comment: "Letterbox crop check".into(),
                                color: "Green".into(),
                                scope: MarkerScope::Clip,
                            }],
                        },
                        Clip {
                            id: "c2".into(),
                            name: "ShotA_Disabled.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Media/ShotA.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 75,
                            end_frame: 150,
                            source_in_frame: 75,
                            source_out_frame: 150,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 1920,
                            source_height: 1080,
                            speed_percent: 100.0,
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
                    transitions: vec![],
                    markers: vec![],
                }],
                audio_tracks: vec![],
                audio_buses: vec![],
                markers: vec![],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        let out_path = std::env::temp_dir().join("aagedal_nle_crop_clip_markers_disabled.drp");
        let report =
            write_drp_file(&project, &out_path, &ConversionOptions::default()).expect("write .drp");
        assert_eq!(report.markers_written, 1);

        let reread = read_drp_file(&out_path).expect("re-read .drp");
        let clips = &reread.sequences[0].video_tracks[0].clips;
        assert_eq!(clips.len(), 2);

        // Clip 1: Crop Filter 6 + clip color + clip marker
        let c1 = &clips[0];
        assert!(c1.enabled);
        assert!((c1.motion.crop_left - 10.0).abs() < 1e-4);
        assert!((c1.motion.crop_right - 5.0).abs() < 1e-4);
        assert!((c1.motion.crop_top - 12.5).abs() < 1e-4);
        assert!((c1.motion.crop_bottom - 12.5).abs() < 1e-4);
        assert_eq!(c1.label_preset_index, Some(8));
        assert_eq!(c1.markers.len(), 1);
        assert_eq!(c1.markers[0].name, "Clip Note");
        assert_eq!(c1.markers[0].comment, "Letterbox crop check");
        assert_eq!(c1.markers[0].color, "Green");
        assert_eq!(c1.markers[0].frame, 20);
        assert_eq!(c1.markers[0].duration_frames, 5);

        // Clip 2: disabled clip flag survived roundtrip
        assert!(!clips[1].enabled);

        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn roundtrips_clip_attributes_data_levels_and_alpha_mode() {
        let mk_asset = |id: &str, name: &str, path: &str, alpha: AlphaMode, levels: DataLevels, cs: Option<&str>| MediaAsset {
            id: id.into(),
            name: name.into(),
            path: path.into(),
            media_kind: MediaKind::Video,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 100,
            source_tc_frame: 0,
            source_tc_string: "00:00:00:00".into(),
            audio_channels: 2,
            audio_sample_rate: 48_000,
            color_space: cs.map(ToString::to_string),
            video_codec: Some("ap4h".into()),
            audio_codec: Some("Linear PCM".into()),
            alpha_mode: alpha,
            data_levels: levels,
            pixel_aspect_ratio: None,
            metadata: BTreeMap::new(),
            custom_metadata: BTreeMap::new(),
        };

        let project = Project {
            name: "ClipAttrsRoundtrip".into(),
            source_format: NleFormat::PremierePro,
            media_assets: vec![
                mk_asset("m1", "FullStraightPQ.mov", "/Media/FullStraightPQ.mov", AlphaMode::Straight, DataLevels::Full, Some("Rec. 2100 PQ")),
                mk_asset("m2", "VideoPremul.mov", "/Media/VideoPremul.mov", AlphaMode::Premultiplied, DataLevels::Video, None),
                mk_asset("m3", "AutoNone.mov", "/Media/AutoNone.mov", AlphaMode::None, DataLevels::Auto, None),
            ],
            sequences: vec![Sequence {
                id: "seq-1".into(),
                name: "Main".into(),
                kind: SequenceKind::Timeline,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 100,
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
                    clips: vec![Clip {
                        id: "c1".into(),
                        name: "FullStraightPQ.mov".into(),
                        media_id: "m1".into(),
                        media_path: "/Media/FullStraightPQ.mov".into(),
                        media_kind: MediaKind::Video,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 50,
                        source_in_frame: 0,
                        source_out_frame: 50,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: Rational::FPS_25,
                        source_width: 1920,
                        source_height: 1080,
                        speed_percent: 100.0,
                        reversed: false,
                        speed_keyframes: vec![],
                        retime_interpolation: RetimeInterpolation::Default,
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
                        markers: vec![],
                    }],
                    transitions: vec![],
                    markers: vec![],
                }],
                audio_tracks: vec![],
                audio_buses: vec![],
                markers: vec![],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        let out_path = std::env::temp_dir().join("aagedal_nle_clip_attrs_roundtrip.drp");
        write_drp_file(&project, &out_path, &ConversionOptions::default()).expect("write .drp");

        let reread = read_drp_file(&out_path).expect("re-read .drp");
        let a1 = reread.media_assets.iter().find(|a| a.name == "FullStraightPQ.mov").expect("m1");
        assert_eq!(a1.alpha_mode, AlphaMode::Straight);
        assert_eq!(a1.data_levels, DataLevels::Full);

        let a2 = reread.media_assets.iter().find(|a| a.name == "VideoPremul.mov").expect("m2");
        assert_eq!(a2.alpha_mode, AlphaMode::Premultiplied);
        assert_eq!(a2.data_levels, DataLevels::Video);

        let a3 = reread.media_assets.iter().find(|a| a.name == "AutoNone.mov").expect("m3");
        assert_eq!(a3.alpha_mode, AlphaMode::Default);
        assert_eq!(a3.data_levels, DataLevels::Auto);

        let _ = std::fs::remove_file(&out_path);
    }
}
