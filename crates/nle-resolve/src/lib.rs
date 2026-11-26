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
                        enabled: true,
                        audio_channel: 1,
                        link_group_id: None,
                        label_name: None,
                        label_preset_index: None,
                        motion: MotionTransform::default(),
                        opacity: 100.0,
                        lumetri_effects: vec![],
                        effects: vec![],
                        markers: vec![],
                    }],
                    transitions: vec![],
                    markers: vec![],
                }],
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
                        enabled: true,
                        audio_channel: 1,
                        link_group_id: None,
                        label_name: None,
                        label_preset_index: None,
                        motion: MotionTransform::default(),
                        opacity: 100.0,
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
                            enabled: true,
                            audio_channel: 1,
                            link_group_id: None,
                            label_name: None,
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![],
                        }],
                        transitions: vec![],
                        markers: vec![],
                    }],
                    audio_tracks: vec![],
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
                                enabled: true,
                                audio_channel: 1,
                                link_group_id: Some("link-0".into()),
                                label_name: None,
                                label_preset_index: None,
                                motion: MotionTransform::default(),
                                opacity: 85.0,
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
                                enabled: true,
                                audio_channel: 1,
                                link_group_id: None,
                                label_name: None,
                                label_preset_index: None,
                                motion: MotionTransform::default(),
                                opacity: 100.0,
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
                            enabled: true,
                            audio_channel: 1,
                            link_group_id: Some("link-0".into()),
                            label_name: None,
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![],
                        }],
                        transitions: vec![],
                        markers: vec![],
                    }],
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
}
