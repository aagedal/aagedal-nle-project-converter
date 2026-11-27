pub mod reader;
pub mod writer;

pub use reader::{parse_edl_str, read_edl_file, EdlReader};
pub use writer::{write_edl_file, EdlWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::*;
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn roundtrips_cmx3600_edl_with_dissolves_m2_speed_markers_and_source_paths() {
        let dir = std::env::temp_dir().join("aagedal_edl_roundtrip_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let edl_path = dir.join("conform.edl");

        let mut asset1_meta = BTreeMap::new();
        asset1_meta.insert("Tape Name".to_string(), "A001R1".to_string());

        let proj = Project {
            name: "EDL_Conform".into(),
            source_format: NleFormat::FinalCutPro,
            media_assets: vec![
                MediaAsset {
                    id: "m1".into(),
                    name: "A001_C001.mov".into(),
                    path: "/Volumes/RAID/Dailies/A001_C001.mov".into(),
                    media_kind: MediaKind::Video,
                    width: 1920,
                    height: 1080,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 500,
                    source_tc_frame: 90_000,
                    source_tc_string: "01:00:00:00".into(),
                    audio_channels: 2,
                    audio_sample_rate: 48_000,
                    color_space: None,
                    video_codec: None,
                    audio_codec: None,
                    alpha_mode: AlphaMode::Default,
                    data_levels: DataLevels::Auto,
                    pixel_aspect_ratio: None,
                    metadata: asset1_meta,
                    custom_metadata: BTreeMap::new(),
                },
                MediaAsset {
                    id: "m2".into(),
                    name: "B002_SlowMo.mov".into(),
                    path: "/Volumes/RAID/Dailies/B002_SlowMo.mov".into(),
                    media_kind: MediaKind::Video,
                    width: 1920,
                    height: 1080,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 400,
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
            ],
            sequences: vec![Sequence {
                id: "s1".into(),
                name: "FCP_Conform_Cut".into(),
                kind: SequenceKind::Timeline,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 200,
                start_tc_frame: 90_000,
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
                            name: "A001_C001.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Volumes/RAID/Dailies/A001_C001.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 0,
                            end_frame: 100,
                            source_in_frame: 25,
                            source_out_frame: 125,
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
                            label_name: Some("Blue".into()),
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            blend_mode: BlendMode::Normal,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![],
                        },
                        Clip {
                            id: "c2".into(),
                            name: "B002_SlowMo.mov".into(),
                            media_id: "m2".into(),
                            media_path: "/Volumes/RAID/Dailies/B002_SlowMo.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 100,
                            end_frame: 200,
                            source_in_frame: 0,
                            source_out_frame: 50,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 1920,
                            source_height: 1080,
                            speed_percent: 50.0,
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
                        },
                    ],
                    transitions: vec![Transition {
                        name: "Cross Dissolve".into(),
                        effect_id: "AE.ADBE Cross Dissolve".into(),
                        start_frame: 90,
                        end_frame: 110,
                        alignment: "center".into(),
                    }],
                    markers: vec![],
                }],
                audio_tracks: vec![],
                audio_buses: vec![],
                markers: vec![Marker {
                    frame: 50,
                    duration_frames: 1,
                    name: "VFX Check".into(),
                    comment: "Remove boom mic".into(),
                    color: "Red".into(),
                    scope: MarkerScope::Sequence,
                }],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        let rep = write_edl_file(&proj, &edl_path, &ConversionOptions::default()).unwrap();
        assert_eq!(rep.sequences_converted, 1);
        assert_eq!(rep.video_clips_converted, 2);
        assert_eq!(rep.markers_written, 1);

        let raw = fs::read_to_string(&edl_path).unwrap();
        assert!(raw.contains("TITLE: FCP_Conform_Cut"));
        assert!(raw.contains("FCM: NON-DROP FRAME"));
        assert!(raw.contains("M2"));
        assert!(raw.contains("* FROM CLIP NAME: A001_C001.mov"));
        assert!(raw.contains("D    020"));

        let read_back = read_edl_file(&edl_path).expect("read back EDL");
        assert_eq!(read_back.source_format, NleFormat::Edl);
        assert_eq!(read_back.sequences.len(), 1);
        let seq = &read_back.sequences[0];
        assert_eq!(seq.name, "FCP_Conform_Cut");
        assert_eq!(seq.frame_rate, Rational::FPS_25);
        assert_eq!(seq.start_tc_frame, 90_000);

        let v1 = &seq.video_tracks[0];
        assert_eq!(v1.clips.len(), 2);
        assert_eq!(v1.clips[0].name, "A001_C001.mov");
        assert_eq!(v1.clips[0].media_path, "/Volumes/RAID/Dailies/A001_C001.mov");
        assert_eq!(v1.clips[0].start_frame, 0);
        assert_eq!(v1.clips[0].end_frame, 100);
        assert_eq!(v1.clips[0].label_name.as_deref(), Some("Blue"));

        assert_eq!(v1.clips[1].name, "B002_SlowMo.mov");
        assert_eq!(v1.clips[1].start_frame, 100);
        assert_eq!(v1.clips[1].end_frame, 200);
        assert!((v1.clips[1].speed_percent - 50.0).abs() < 0.2);

        assert_eq!(v1.transitions.len(), 1);
        assert_eq!(v1.transitions[0].start_frame, 90);
        assert_eq!(v1.transitions[0].end_frame, 110);

        assert_eq!(seq.markers.len(), 1);
        assert_eq!(seq.markers[0].frame, 50);
        assert_eq!(seq.markers[0].name, "VFX Check");
        assert_eq!(seq.markers[0].color, "Red");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_real_world_resolve_and_avid_edl_with_blanks_audio_reverse_and_locators() {
        let edl = r#"TITLE: AVID_CONFORM_REEL1
FCM: NON-DROP FRAME

001  AX       V     C        01:00:05:00 01:00:09:00 01:00:00:00 01:00:04:00
* FROM CLIP NAME: Scene01_Take03.mxf
* LOC: 01:00:02:00 GREEN   Good performance

002  BL       V     C        00:00:00:00 00:00:02:00 01:00:04:00 01:00:06:00

003  AX       V     C        01:00:10:00 01:00:14:00 01:00:06:00 01:00:10:00
M2   AX       -024.0          01:00:10:00
* FROM CLIP NAME: Reverse_Action.mov

004  A002     AA    C        00:00:00:00 00:00:10:00 01:00:00:00 01:00:10:00
* FROM CLIP NAME: ProductionMix.wav
"#;
        let proj = parse_edl_str(edl, "Fallback").expect("parse Avid EDL");
        let seq = &proj.sequences[0];
        assert_eq!(seq.name, "AVID_CONFORM_REEL1");
        assert_eq!(seq.frame_rate, Rational::FPS_24);
        assert_eq!(seq.start_tc_frame, 86_400);

        // Blank event 002 leaves a 48-frame gap between clip 1 (0..96) and clip 2 (144..240)
        assert_eq!(seq.video_tracks[0].clips.len(), 2);
        assert_eq!(seq.video_tracks[0].clips[0].start_frame, 0);
        assert_eq!(seq.video_tracks[0].clips[0].end_frame, 96);
        assert_eq!(seq.video_tracks[0].clips[1].start_frame, 144);
        assert_eq!(seq.video_tracks[0].clips[1].end_frame, 240);
        assert!(seq.video_tracks[0].clips[1].reversed);
        assert!((seq.video_tracks[0].clips[1].speed_percent - 100.0).abs() < 0.2);

        // Audio track AA -> stereo audio clip
        assert_eq!(seq.audio_tracks.len(), 1);
        assert_eq!(seq.audio_tracks[0].clips[0].name, "ProductionMix.wav");

        // Avid * LOC: marker at 01:00:02:00 -> frame 48
        assert_eq!(seq.markers.len(), 1);
        assert_eq!(seq.markers[0].frame, 48);
        assert_eq!(seq.markers[0].color, "Green");
        assert_eq!(seq.markers[0].comment, "Good performance");
    }
}
