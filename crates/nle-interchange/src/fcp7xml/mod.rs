pub mod reader;
pub mod writer;

pub use reader::{parse_fcp7_xml_str, read_fcp7_xml_file, Fcp7XmlReader};
pub use writer::{write_fcp7_xml_file, Fcp7XmlWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::*;
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn warns_when_multiple_timelines_are_selected() {
        let dir = std::env::temp_dir().join("aagedal_fcp7_multi_seq_test");
        let _ = fs::remove_dir_all(&dir);
        let out = dir.join("multi.xml");

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
            source_format: NleFormat::DaVinciResolve,
            sequences: vec![mk_seq("s1", "Reel_1"), mk_seq("s2", "Reel_2")],
            ..Default::default()
        };

        let report = write_fcp7_xml_file(&proj, &out, &ConversionOptions::default()).unwrap();
        assert_eq!(report.sequences_converted, 1);
        assert!(
            report
                .messages
                .iter()
                .any(|m| m.code == "FCP7_SINGLE_SEQUENCE" && m.severity == Severity::Warning),
            "expected FCP7_SINGLE_SEQUENCE warning, got: {:?}",
            report.messages
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrips_fcp7_xml_motion_crop_opacity_blend_links_transitions_speed_and_markers() {
        let dir = std::env::temp_dir().join("aagedal_fcp7_roundtrip_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let xml_path = dir.join("roundtrip.xml");

        let proj = Project {
            name: "Fcp7Roundtrip".into(),
            source_format: NleFormat::PremierePro,
            media_assets: vec![MediaAsset {
                id: "m1".into(),
                name: "Interview_Søndag.mov".into(),
                path: "/Volumes/Media/Shoot #1/Interview_Søndag.mov".into(),
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
                metadata: BTreeMap::new(),
                custom_metadata: BTreeMap::new(),
            }],
            sequences: vec![Sequence {
                id: "s1".into(),
                name: "Main_Edit".into(),
                kind: SequenceKind::Timeline,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 150,
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
                            name: "Interview_Søndag.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Volumes/Media/Shoot #1/Interview_Søndag.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 0,
                            end_frame: 100,
                            source_in_frame: 10,
                            source_out_frame: 60,
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
                            link_group_id: Some("lg-1".into()),
                            label_name: Some("Iris".into()),
                            label_preset_index: None,
                            motion: MotionTransform {
                                scale: 125.0,
                                position_x: 0.12,
                                position_y: -0.08,
                                rotation: 15.0,
                                crop_left: 5.0,
                                crop_right: 10.0,
                                ..Default::default()
                            },
                            opacity: 75.0,
                            blend_mode: BlendMode::Overlay,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![Marker {
                                frame: 20,
                                duration_frames: 4,
                                name: "ClipNote".into(),
                                comment: "Trim slightly".into(),
                                color: "Green".into(),
                                scope: MarkerScope::Clip,
                            }],
                        },
                        Clip {
                            id: "v2".into(),
                            name: "ReverseDisabled".into(),
                            media_id: "m1".into(),
                            media_path: "/Volumes/Media/Shoot #1/Interview_Søndag.mov".into(),
                            media_kind: MediaKind::Video,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 100,
                            end_frame: 150,
                            source_in_frame: 100,
                            source_out_frame: 150,
                            source_in_ticks: None,
                            source_out_ticks: None,
                            source_fps: Rational::FPS_25,
                            source_width: 1920,
                            source_height: 1080,
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
                        name: "Interview_Søndag.mov".into(),
                        media_id: "m1".into(),
                        media_path: "/Volumes/Media/Shoot #1/Interview_Søndag.mov".into(),
                        media_kind: MediaKind::Audio,
                        source_kind: ClipSourceKind::Media,
                        active_angle: None,
                        start_frame: 0,
                        end_frame: 100,
                        source_in_frame: 10,
                        source_out_frame: 60,
                        source_in_ticks: None,
                        source_out_ticks: None,
                        source_fps: Rational::FPS_25,
                        source_width: 0,
                        source_height: 0,
                        speed_percent: 50.0,
                        reversed: false,
                        speed_keyframes: vec![],
                        retime_interpolation: RetimeInterpolation::Default,
                        enabled: true,
                        audio_channel: 1,
                        link_group_id: Some("lg-1".into()),
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
                    frame: 30,
                    duration_frames: 1,
                    name: "SeqMark".into(),
                    comment: "Check audio".into(),
                    color: "Red".into(),
                    scope: MarkerScope::Sequence,
                }],
                metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        write_fcp7_xml_file(&proj, &xml_path, &ConversionOptions::default()).unwrap();
        let read_back = read_fcp7_xml_file(&xml_path).expect("read back FCP7 XML");

        assert_eq!(read_back.source_format, NleFormat::Fcp7Xml);
        assert_eq!(read_back.sequences.len(), 1);
        let seq = &read_back.sequences[0];
        assert_eq!(seq.name, "Main_Edit");
        assert_eq!(seq.width, 1920);
        assert_eq!(seq.height, 1080);
        assert_eq!(seq.frame_rate, Rational::FPS_25);
        assert_eq!(seq.start_tc_frame, 90_000);

        let v0 = &seq.video_tracks[0].clips[0];
        assert_eq!(v0.media_path, "/Volumes/Media/Shoot #1/Interview_Søndag.mov");
        assert_eq!(v0.start_frame, 0);
        assert_eq!(v0.end_frame, 100);
        assert_eq!(v0.source_in_frame, 10);
        assert_eq!(v0.source_out_frame, 60);
        assert!((v0.speed_percent - 50.0).abs() < 0.05);
        assert!(!v0.reversed);
        assert!((v0.motion.scale - 125.0).abs() < 1e-2);
        assert!((v0.motion.position_x - 0.12).abs() < 1e-3);
        assert!((v0.motion.position_y - (-0.08)).abs() < 1e-3);
        assert!((v0.motion.rotation - 15.0).abs() < 1e-2);
        assert!((v0.motion.crop_left - 5.0).abs() < 1e-2);
        assert!((v0.motion.crop_right - 10.0).abs() < 1e-2);
        assert!((v0.opacity - 75.0).abs() < 1e-2);
        assert_eq!(v0.blend_mode, BlendMode::Overlay);
        assert_eq!(v0.label_name.as_deref(), Some("Iris"));
        assert_eq!(v0.markers.len(), 1);
        assert_eq!(v0.markers[0].color, "Green");

        // Linked V1 + A1 share link_group_id
        let a0 = &seq.audio_tracks[0].clips[0];
        assert!(v0.link_group_id.is_some());
        assert_eq!(v0.link_group_id, a0.link_group_id);

        // Clip 2: reverse + disabled
        let v1 = &seq.video_tracks[0].clips[1];
        assert!(v1.reversed);
        assert!(!v1.enabled);

        // Transition
        assert_eq!(seq.video_tracks[0].transitions.len(), 1);
        assert_eq!(seq.video_tracks[0].transitions[0].start_frame, 90);
        assert_eq!(seq.video_tracks[0].transitions[0].end_frame, 110);

        // Sequence marker
        assert_eq!(seq.markers.len(), 1);
        assert_eq!(seq.markers[0].frame, 30);
        assert_eq!(seq.markers[0].color, "Red");

        // Asset TC
        assert_eq!(read_back.media_assets[0].source_tc_frame, 90_000);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_premiere_style_project_bins_minus_one_transitions_nested_seq_and_effects() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE xmeml>
<xmeml version="5">
  <project>
    <name>Documentary_Project</name>
    <children>
      <bin>
        <name>Dailies</name>
        <children>
          <clip id="master-1">
            <name>A001.mov</name>
            <duration>300</duration>
            <rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate>
            <logginginfo>
              <description>Wide exterior</description>
              <scene>4A</scene>
            </logginginfo>
            <media>
              <video>
                <track>
                  <clipitem id="mc-v1">
                    <file id="file-1">
                      <name>A001.mov</name>
                      <pathurl>file://localhost/Volumes/SSD/Dailies/A001.mov</pathurl>
                      <rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate>
                      <duration>300</duration>
                      <timecode>
                        <rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate>
                        <string>01:00:00:00</string>
                        <frame>86400</frame>
                        <reel><name>REEL_01</name></reel>
                      </timecode>
                      <media>
                        <video><samplecharacteristics><width>3840</width><height>2160</height></samplecharacteristics></video>
                        <audio><samplecharacteristics><samplerate>48000</samplerate></samplecharacteristics><channelcount>2</channelcount></audio>
                      </media>
                    </file>
                  </clipitem>
                </track>
              </video>
            </media>
          </clip>
          <clip id="master-unused">
            <name>Unused_Broll.mov</name>
            <file id="file-unused">
              <name>Unused_Broll.mov</name>
              <pathurl>file://localhost/Volumes/SSD/Dailies/Unused_Broll.mov</pathurl>
              <rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate>
              <duration>120</duration>
            </file>
          </clip>
        </children>
      </bin>
      <sequence id="seq-nested">
        <name>Sub_Graphics</name>
        <duration>48</duration>
        <rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate>
        <media>
          <video>
            <format><samplecharacteristics><width>1920</width><height>1080</height></samplecharacteristics></format>
            <track>
              <clipitem id="sub-c1">
                <name>A001.mov</name>
                <start>0</start><end>48</end><in>0</in><out>48</out>
                <file id="file-1"/>
              </clipitem>
            </track>
          </video>
        </media>
      </sequence>
      <sequence id="seq-main">
        <name>Main_Cut</name>
        <duration>148</duration>
        <rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate>
        <timecode>
          <rate><timebase>24</timebase><ntsc>TRUE</ntsc></rate>
          <string>01:00:00:00</string>
          <frame>86400</frame>
        </timecode>
        <media>
          <video>
            <format><samplecharacteristics><width>1920</width><height>1080</height></samplecharacteristics></format>
            <track>
              <clipitem id="ci-1">
                <name>Shot_1</name>
                <start>0</start>
                <end>-1</end>
                <in>10</in>
                <out>60</out>
                <file id="file-1"/>
                <filter>
                  <enabled>TRUE</enabled>
                  <effect>
                    <name>Gaussian Blur</name>
                    <effectid>AE.ADBE Gaussian Blur 2</effectid>
                    <effectcategory>Blur &amp; Sharpen</effectcategory>
                    <mediatype>video</mediatype>
                    <parameter>
                      <name>Blurriness</name>
                      <parameterid>blur</parameterid>
                      <value>22.5</value>
                    </parameter>
                  </effect>
                </filter>
              </clipitem>
              <transitionitem>
                <start>40</start>
                <end>60</end>
                <alignment>center</alignment>
                <effect>
                  <name>Cross Dissolve</name>
                  <effectid>Cross Dissolve</effectid>
                </effect>
              </transitionitem>
              <clipitem id="ci-2">
                <name>Shot_2</name>
                <start>-1</start>
                <end>100</end>
                <in>60</in>
                <out>110</out>
                <file id="file-1"/>
              </clipitem>
              <clipitem id="ci-nested">
                <name>Sub_Graphics</name>
                <start>100</start>
                <end>148</end>
                <in>0</in>
                <out>48</out>
                <sequence id="seq-nested"/>
              </clipitem>
            </track>
          </video>
        </media>
        <marker>
          <name>PointMarker</name>
          <comment>Check cut</comment>
          <in>50</in>
          <out>-1</out>
        </marker>
      </sequence>
    </children>
  </project>
</xmeml>"#;

        let proj = parse_fcp7_xml_str(xml, "Fallback").expect("parse Premiere XMEML");
        assert_eq!(proj.name, "Documentary_Project");
        assert_eq!(proj.bins.len(), 1);
        assert_eq!(proj.bins[0].name, "Dailies");
        assert_eq!(proj.bins[0].media_paths.len(), 2);
        assert_eq!(proj.media_assets.len(), 2);

        let a001 = proj
            .media_assets
            .iter()
            .find(|a| a.name == "A001.mov")
            .expect("A001 asset");
        assert_eq!(a001.frame_rate, Rational::FPS_23_976);
        assert_eq!(a001.width, 3840);
        assert_eq!(a001.source_tc_frame, 86_400);
        assert_eq!(a001.metadata.get("Scene").map(String::as_str), Some("4A"));
        assert_eq!(a001.metadata.get("Tape Name").map(String::as_str), Some("REEL_01"));

        let main = proj
            .sequences
            .iter()
            .find(|s| s.name == "Main_Cut")
            .expect("Main_Cut");
        assert_eq!(main.frame_rate, Rational::FPS_23_976);
        assert_eq!(main.start_tc_frame, 86_400);
        assert_eq!(main.video_tracks[0].clips.len(), 3);

        // -1 boundaries resolved from transition 40..60 center -> cut frame 50!
        assert_eq!(main.video_tracks[0].clips[0].start_frame, 0);
        assert_eq!(main.video_tracks[0].clips[0].end_frame, 50);
        assert_eq!(main.video_tracks[0].clips[0].effects.len(), 1);
        assert_eq!(
            main.video_tracks[0].clips[0].effects[0].match_name,
            "AE.ADBE Gaussian Blur 2"
        );
        assert_eq!(main.video_tracks[0].clips[1].start_frame, 50);
        assert_eq!(main.video_tracks[0].clips[1].end_frame, 100);

        // Nested sequence clipitem
        let nested_clip = &main.video_tracks[0].clips[2];
        assert_eq!(nested_clip.source_kind, ClipSourceKind::NestedSequence);
        assert_eq!(nested_clip.media_id, "seq-nested");

        // Point marker with out=-1 becomes duration 1
        assert_eq!(main.markers.len(), 1);
        assert_eq!(main.markers[0].frame, 50);
        assert_eq!(main.markers[0].duration_frames, 1);
    }
}
