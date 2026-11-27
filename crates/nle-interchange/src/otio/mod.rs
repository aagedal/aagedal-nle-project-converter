pub mod reader;
pub mod writer;

pub use reader::{parse_otio_str, read_otio_file, OtioReader};
pub use writer::{write_otio_file, OtioWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Write;

    #[test]
    fn warns_when_multiple_timelines_are_selected() {
        let dir = std::env::temp_dir().join("aagedal_otio_multi_seq_test");
        let _ = fs::remove_dir_all(&dir);
        let out = dir.join("multi.otio");

        let mk_seq = |id: &str, name: &str| Sequence {
            id: id.into(),
            name: name.into(),
            kind: SequenceKind::Timeline,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 0,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: Vec::new(),
            audio_tracks: Vec::new(),
            audio_buses: Vec::new(),
            markers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let proj = Project {
            name: "Multi".into(),
            source_format: NleFormat::PremierePro,
            sequences: vec![mk_seq("s1", "Edit_Main"), mk_seq("s2", "Edit_Alt")],
            ..Default::default()
        };

        let report = write_otio_file(&proj, &out, &ConversionOptions::default()).unwrap();
        assert_eq!(report.sequences_converted, 1);
        assert!(
            report
                .messages
                .iter()
                .any(|m| m.code == "OTIO_SINGLE_SEQUENCE" && m.severity == Severity::Warning),
            "expected OTIO_SINGLE_SEQUENCE warning, got: {:?}",
            report.messages
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrips_otio_timelines_gaps_transitions_retiming_markers_and_unicode_paths() {
        let dir = std::env::temp_dir().join("aagedal_otio_roundtrip_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let otio_path = dir.join("roundtrip.otio");

        let proj = Project {
            name: "OTIO_Roundtrip".into(),
            source_format: NleFormat::DaVinciResolve,
            media_assets: vec![
                MediaAsset {
                    id: "m1".into(),
                    name: "Søndag_A.mov".into(),
                    path: "/Volumes/RAID/Shoot #1/Søndag_A.mov".into(),
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
                    name: "Music.wav".into(),
                    path: "/Volumes/RAID/Audio/Music.wav".into(),
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
                id: "seq1".into(),
                name: "Commercial_Cut".into(),
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
                            id: "c1".into(),
                            name: "Søndag_A.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Volumes/RAID/Shoot #1/Søndag_A.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 25, // 25-frame leading gap
                            end_frame: 100,  // 75 frames on timeline
                            source_in_frame: 10,
                            source_out_frame: 48,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 3840,
                            source_height: 2160,
                            speed_percent: 50.0,
                            reversed: false,
                            speed_keyframes: vec![],
                            retime_interpolation: RetimeInterpolation::OpticalFlow,
                            enabled: true,
                            audio_channel: 0,
                            link_group_id: Some("lg1".into()),
                            label_name: Some("Violet".into()),
                            label_preset_index: None,
                            motion: MotionTransform {
                                scale: 120.0,
                                position_x: 0.1,
                                position_y: -0.05,
                                rotation: 7.5,
                                crop_left: 5.0,
                                ..Default::default()
                            },
                            opacity: 82.0,
                            blend_mode: BlendMode::Screen,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![Marker {
                                frame: 15,
                                duration_frames: 5,
                                name: "Hit".into(),
                                comment: "Slow-mo accent".into(),
                                color: "Green".into(),
                                scope: MarkerScope::Clip,
                            }],
                        },
                        Clip {
                            id: "c2".into(),
                            name: "ReverseShot".into(),
                            media_id: "m1".into(),
                            media_path: "/Volumes/RAID/Shoot #1/Søndag_A.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 100,
                            end_frame: 150,
                            source_in_frame: 50,
                            source_out_frame: 100,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 3840,
                            source_height: 2160,
                            speed_percent: 100.0,
                            reversed: true,
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
                        Clip {
                            id: "c3".into(),
                            name: "FreezeHold".into(),
                            media_id: "m1".into(),
                            media_path: "/Volumes/RAID/Shoot #1/Søndag_A.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 150,
                            end_frame: 200,
                            source_in_frame: 100,
                            source_out_frame: 100,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 3840,
                            source_height: 2160,
                            speed_percent: 0.0,
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
                        name: "Music.wav".into(),
                        media_id: "m2".into(),
                        media_path: "/Volumes/RAID/Audio/Music.wav".into(),
                        media_kind: MediaKind::Audio,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 200,
                        source_in_frame: 0,
                        source_out_frame: 200,
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
                    frame: 50,
                    duration_frames: 2,
                    name: "Review".into(),
                    comment: "Check pacing".into(),
                    color: "Red".into(),
                    scope: MarkerScope::Sequence,
                }],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        write_otio_file(&proj, &otio_path, &ConversionOptions::default()).unwrap();
        let read_back = read_otio_file(&otio_path).expect("read back written .otio");

        assert_eq!(read_back.source_format, NleFormat::OpenTimelineIo);
        assert_eq!(read_back.name, "OTIO_Roundtrip");
        assert_eq!(read_back.sequences.len(), 1);
        let seq = &read_back.sequences[0];
        assert_eq!(seq.name, "Commercial_Cut");
        assert_eq!(seq.width, 3840);
        assert_eq!(seq.height, 2160);
        assert_eq!(seq.frame_rate, Rational::FPS_25);
        assert_eq!(seq.start_tc_frame, 90_000);
        assert_eq!(seq.video_tracks.len(), 1);
        assert_eq!(seq.audio_tracks.len(), 1);

        let v1 = &seq.video_tracks[0];
        assert_eq!(v1.clips.len(), 3);
        // Clip 1: starts after 25f gap, occupies 75 frames on timeline (even at 50% speed!)
        let c0 = &v1.clips[0];
        assert_eq!(c0.start_frame, 25);
        assert_eq!(c0.end_frame, 100);
        assert_eq!(c0.source_in_frame, 10);
        assert!((c0.speed_percent - 50.0).abs() < 1e-3);
        assert!(!c0.reversed);
        assert_eq!(c0.media_path, "/Volumes/RAID/Shoot #1/Søndag_A.mov");
        assert!((c0.motion.scale - 120.0).abs() < 1e-3);
        assert!((c0.motion.position_x - 0.1).abs() < 1e-3);
        assert!((c0.motion.position_y - (-0.05)).abs() < 1e-3);
        assert!((c0.motion.rotation - 7.5).abs() < 1e-3);
        assert!((c0.motion.crop_left - 5.0).abs() < 1e-3);
        assert!((c0.opacity - 82.0).abs() < 1e-3);
        assert_eq!(c0.blend_mode, BlendMode::Screen);
        assert_eq!(c0.label_name.as_deref(), Some("Violet"));
        assert_eq!(c0.markers.len(), 1);
        assert_eq!(c0.markers[0].frame, 15);
        assert_eq!(c0.markers[0].color, "Green");

        // Clip 2: reverse + disabled
        let c1 = &v1.clips[1];
        assert_eq!(c1.start_frame, 100);
        assert_eq!(c1.end_frame, 150);
        assert!(c1.reversed);
        assert!((c1.speed_percent - 100.0).abs() < 1e-3);
        assert!(!c1.enabled);

        // Clip 3: freeze frame
        let c2 = &v1.clips[2];
        assert_eq!(c2.start_frame, 150);
        assert_eq!(c2.end_frame, 200);
        assert!(c2.speed_percent.abs() < 1e-3);

        // Transition at cut frame 100 (90..110)
        assert_eq!(v1.transitions.len(), 1);
        assert_eq!(v1.transitions[0].start_frame, 90);
        assert_eq!(v1.transitions[0].end_frame, 110);
        assert_eq!(v1.transitions[0].alignment, "center");

        // Sequence marker
        assert_eq!(seq.markers.len(), 1);
        assert_eq!(seq.markers[0].frame, 50);
        assert_eq!(seq.markers[0].duration_frames, 2);
        assert_eq!(seq.markers[0].color, "Red");

        // Asset source TC preserved via available_range
        let video_asset = read_back
            .media_assets
            .iter()
            .find(|a| a.path.ends_with("Søndag_A.mov"))
            .expect("video asset");
        assert_eq!(video_asset.source_tc_frame, 90_000);
        assert_eq!(video_asset.duration_frames, 500);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_modern_otio_media_references_collections_nested_stacks_and_bundles() {
        let dir = std::env::temp_dir().join("aagedal_otio_bundles_test");
        let _ = fs::remove_dir_all(&dir);
        let otiod_dir = dir.join("ResolveExport.otiod");
        fs::create_dir_all(otiod_dir.join("media")).unwrap();

        let otio_json = r#"{
          "OTIO_SCHEMA": "SerializableCollection.1",
          "name": "Resolve_Export_Pack",
          "children": [
            {
              "OTIO_SCHEMA": "Timeline.1",
              "name": "Reel_01",
              "global_start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 86400.0 },
              "tracks": {
                "OTIO_SCHEMA": "Stack.1",
                "name": "tracks",
                "children": [
                  {
                    "OTIO_SCHEMA": "Track.1",
                    "name": "Video 1",
                    "kind": "Video",
                    "children": [
                      {
                        "OTIO_SCHEMA": "Clip.2",
                        "name": "A001_C001",
                        "active_media_reference_key": "DEFAULT_MEDIA",
                        "media_references": {
                          "DEFAULT_MEDIA": {
                            "OTIO_SCHEMA": "ExternalReference.1",
                            "name": "A001_C001.mov",
                            "target_url": "media/A001_C001.mov",
                            "available_range": {
                              "OTIO_SCHEMA": "TimeRange.1",
                              "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 86400.0 },
                              "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 240.0 }
                            }
                          }
                        },
                        "source_range": {
                          "OTIO_SCHEMA": "TimeRange.1",
                          "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 86424.0 },
                          "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 48.0 }
                        },
                        "effects": [
                          {
                            "OTIO_SCHEMA": "Effect.1",
                            "name": "Gaussian Blur",
                            "effect_name": "Gaussian Blur",
                            "metadata": { "Blurriness": 15.0 }
                          }
                        ]
                      },
                      {
                        "OTIO_SCHEMA": "Stack.1",
                        "name": "Nested_Title_Comp",
                        "source_range": {
                          "OTIO_SCHEMA": "TimeRange.1",
                          "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 0.0 },
                          "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 72.0 }
                        },
                        "children": [
                          {
                            "OTIO_SCHEMA": "Track.1",
                            "name": "V1",
                            "kind": "Video",
                            "children": [
                              {
                                "OTIO_SCHEMA": "Clip.2",
                                "name": "InnerClip",
                                "media_reference": {
                                  "OTIO_SCHEMA": "ExternalReference.1",
                                  "target_url": "file:///Media/Inner.mov"
                                },
                                "source_range": {
                                  "OTIO_SCHEMA": "TimeRange.1",
                                  "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 0.0 },
                                  "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 72.0 }
                                }
                              }
                            ]
                          }
                        ]
                      }
                    ]
                  }
                ]
              }
            },
            {
              "OTIO_SCHEMA": "Timeline.1",
              "name": "Reel_02",
              "tracks": {
                "OTIO_SCHEMA": "Stack.1",
                "children": [
                  {
                    "OTIO_SCHEMA": "Track.1",
                    "name": "V1",
                    "kind": "Video",
                    "children": [
                      {
                        "OTIO_SCHEMA": "Clip.2",
                        "name": "OfflineShot",
                        "media_reference": {
                          "OTIO_SCHEMA": "MissingReference.1",
                          "name": "OfflineShot"
                        },
                        "source_range": {
                          "OTIO_SCHEMA": "TimeRange.1",
                          "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 0.0 },
                          "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 24.0 }
                        }
                      }
                    ]
                  }
                ]
              }
            }
          ]
        }"#;

        fs::write(otiod_dir.join("content.otio"), otio_json).unwrap();

        // Read .otiod bundle directory
        let proj = read_otio_file(&otiod_dir).expect("read .otiod bundle");
        assert_eq!(proj.name, "Resolve_Export_Pack");
        // 2 main timelines + 1 nested compound Stack
        assert_eq!(proj.sequences.len(), 3);
        let reel1 = proj
            .sequences
            .iter()
            .find(|s| s.name == "Reel_01")
            .expect("Reel_01");
        assert_eq!(reel1.frame_rate, Rational::FPS_24);
        assert_eq!(reel1.start_tc_frame, 86_400);
        let clip0 = &reel1.video_tracks[0].clips[0];
        assert_eq!(clip0.source_in_frame, 24); // 86424 - 86400
        assert!(clip0.media_path.ends_with("media/A001_C001.mov"));
        assert!(clip0.media_path.contains("ResolveExport.otiod"));
        assert_eq!(clip0.effects.len(), 1);
        assert_eq!(clip0.effects[0].display_name, "Gaussian Blur");

        let nested_clip = &reel1.video_tracks[0].clips[1];
        assert_eq!(nested_clip.source_kind, ClipSourceKind::CompoundClip);
        assert_eq!(nested_clip.start_frame, 48);
        assert_eq!(nested_clip.end_frame, 120);

        let compound_seq = proj
            .sequences
            .iter()
            .find(|s| s.id == nested_clip.media_id)
            .expect("nested compound sequence");
        assert_eq!(compound_seq.kind, SequenceKind::CompoundClip);
        assert_eq!(compound_seq.video_tracks[0].clips[0].media_path, "/Media/Inner.mov");

        // Also test .otioz zip bundle
        let otioz_path = dir.join("ResolveExport.otioz");
        {
            let file = fs::File::create(&otioz_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            zw.start_file("content.otio", zip::write::SimpleFileOptions::default())
                .unwrap();
            zw.write_all(otio_json.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let proj_z = read_otio_file(&otioz_path).expect("read .otioz archive");
        assert_eq!(proj_z.sequences.len(), 3);

        let _ = fs::remove_dir_all(&dir);
    }
}
