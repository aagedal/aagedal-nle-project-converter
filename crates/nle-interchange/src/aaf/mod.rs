pub mod cfb;
pub mod codec;
pub mod reader;
pub mod writer;

pub use reader::{parse_aaf_bytes, read_aaf_file, AafReader};
pub use writer::{write_aaf_file, AafWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::*;
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn cfb_v3_roundtrips_mini_and_regular_streams() {
        use cfb::{write_cfb_v3, CfbFile, CfbStorageNode};

        let mut root = CfbStorageNode::new("Root Entry", codec::CLSID_ROOT);
        root.add_stream("small", b"hello-mini".to_vec());
        let big_payload: Vec<u8> = (0..10_000usize).map(|i| (i % 251) as u8).collect();
        root.add_stream("large", big_payload.clone());

        let mut child = CfbStorageNode::new("Header-2", codec::CLSID_HEADER);
        child.add_stream("properties", b"props-data".to_vec());
        root.add_storage(child);

        let bytes = write_cfb_v3(&root);
        let parsed = CfbFile::parse(&bytes).expect("parse written CFB v3");
        let small_id = parsed.find_child(parsed.root_id(), "small").unwrap();
        assert_eq!(parsed.read_stream(small_id).unwrap(), b"hello-mini");

        let large_id = parsed.find_child(parsed.root_id(), "large").unwrap();
        assert_eq!(parsed.read_stream(large_id).unwrap(), big_payload);

        let hdr_id = parsed.find_child(parsed.root_id(), "Header-2").unwrap();
        assert_eq!(parsed.entry(hdr_id).unwrap().clsid, codec::CLSID_HEADER);
        let prop_id = parsed.find_child(hdr_id, "properties").unwrap();
        assert_eq!(parsed.read_stream(prop_id).unwrap(), b"props-data");
    }

    #[test]
    fn roundtrips_aaf_multitrack_gaps_transitions_retiming_nested_and_markers() {
        let dir = std::env::temp_dir().join("aagedal_aaf_roundtrip_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let aaf_path = dir.join("roundtrip.aaf");

        let proj = Project {
            name: "AAF_Roundtrip".into(),
            source_format: NleFormat::FinalCutPro,
            media_assets: vec![
                MediaAsset {
                    id: "m1".into(),
                    name: "Scene01_Søndag.mov".into(),
                    path: "/Volumes/Edit/Shoot #1/Scene01_Søndag.mov".into(),
                    media_kind: MediaKind::Video,
                    width: 3840,
                    height: 2160,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 600,
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
                    metadata: BTreeMap::new(),
                    custom_metadata: BTreeMap::new(),
                },
                MediaAsset {
                    id: "m2".into(),
                    name: "Dialogue.wav".into(),
                    path: "/Volumes/Edit/Audio/Dialogue.wav".into(),
                    media_kind: MediaKind::Audio,
                    width: 0,
                    height: 0,
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
                name: "ProTools_Handoff".into(),
                kind: SequenceKind::Timeline,
                width: 3840,
                height: 2160,
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
                            id: "v1".into(),
                            name: "Scene01_Søndag.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Volumes/Edit/Shoot #1/Scene01_Søndag.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 25, // 25f leading Filler gap
                            end_frame: 100,
                            source_in_frame: 10,
                            source_out_frame: 85,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 3840,
                            source_height: 2160,
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
                        },
                        Clip {
                            id: "v2".into(),
                            name: "Scene01_SlowMo".into(),
                            media_id: "m1".into(),
                            media_path: "/Volumes/Edit/Shoot #1/Scene01_Søndag.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 100,
                            end_frame: 180,
                            source_in_frame: 100,
                            source_out_frame: 140,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 3840,
                            source_height: 2160,
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
                audio_tracks: vec![Track {
                    index: 1,
                    kind: TrackKind::Audio,
                    name: "A1".into(),
                    enabled: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    audio_channels: 2,
                    output_bus_id: None,
                    clips: vec![Clip {
                        id: "a1".into(),
                        name: "Dialogue.wav".into(),
                        media_id: "m2".into(),
                        media_path: "/Volumes/Edit/Audio/Dialogue.wav".into(),
                        media_kind: MediaKind::Audio,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 180,
                        source_in_frame: 0,
                        source_out_frame: 180,
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
                markers: vec![Marker {
                    frame: 60,
                    duration_frames: 5,
                    name: "ADR Cue".into(),
                    comment: "Re-record line 12".into(),
                    color: "Cyan".into(),
                    scope: MarkerScope::Sequence,
                }],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        let rep = write_aaf_file(&proj, &aaf_path, &ConversionOptions::default())
            .expect("write .aaf");
        assert_eq!(rep.sequences_converted, 1);
        assert_eq!(rep.video_clips_converted, 2);
        assert_eq!(rep.audio_clips_converted, 1);
        assert_eq!(rep.markers_written, 1);

        let read_back = read_aaf_file(&aaf_path).expect("read back .aaf");
        assert_eq!(read_back.source_format, NleFormat::Aaf);
        assert_eq!(read_back.sequences.len(), 1);
        let seq = &read_back.sequences[0];
        assert_eq!(seq.name, "ProTools_Handoff");
        assert_eq!(seq.width, 3840);
        assert_eq!(seq.height, 2160);
        assert_eq!(seq.frame_rate, Rational::FPS_25);
        assert_eq!(seq.start_tc_frame, 90_000);
        assert_eq!(seq.video_tracks.len(), 1);
        assert_eq!(seq.audio_tracks.len(), 1);

        let v1 = &seq.video_tracks[0];
        assert_eq!(v1.clips.len(), 2);
        assert_eq!(v1.clips[0].start_frame, 25);
        assert_eq!(v1.clips[0].end_frame, 100);
        assert_eq!(
            v1.clips[0].media_path,
            "/Volumes/Edit/Shoot #1/Scene01_Søndag.mov"
        );
        assert_eq!(v1.clips[1].start_frame, 100);
        assert_eq!(v1.clips[1].end_frame, 180);
        assert!((v1.clips[1].speed_percent - 50.0).abs() < 0.2);

        assert_eq!(v1.transitions.len(), 1);
        assert_eq!(v1.transitions[0].start_frame, 90);
        assert_eq!(v1.transitions[0].end_frame, 110);

        assert_eq!(seq.audio_tracks[0].clips.len(), 1);
        assert_eq!(
            seq.audio_tracks[0].clips[0].media_path,
            "/Volumes/Edit/Audio/Dialogue.wav"
        );

        assert_eq!(seq.markers.len(), 1);
        assert_eq!(seq.markers[0].frame, 60);
        assert_eq!(seq.markers[0].duration_frames, 5);
        assert_eq!(seq.markers[0].name, "ADR Cue");
        assert_eq!(seq.markers[0].color, "Cyan");

        // Asset source TC preserved via SourceMob Timecode slot
        let video_asset = read_back
            .media_assets
            .iter()
            .find(|a| a.path.ends_with("Scene01_Søndag.mov"))
            .expect("video asset");
        assert_eq!(video_asset.source_tc_frame, 90_000);

        let _ = fs::remove_dir_all(&dir);
    }
}
