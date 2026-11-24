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
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 150,
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
        assert!(reread.sequences[0].audio_tracks[0].locked);

        let _ = std::fs::remove_file(&out_path);
    }
}
