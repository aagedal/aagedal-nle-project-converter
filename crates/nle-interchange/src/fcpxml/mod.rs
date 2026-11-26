pub mod reader;
pub mod time;
pub mod writer;

pub use reader::{parse_fcpxml_str, read_fcpxml_file, FcpXmlReader};
pub use writer::{write_fcpxml_file, FcpXmlWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::*;
    use std::collections::BTreeMap;

    #[test]
    fn parses_real_world_fcpxml_115_features() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.15">
  <resources>
    <format id="r1" name="FFVideoFormat1080p25" frameDuration="100/2500s" width="1920" height="1080" colorSpace="9-16-9 (Rec. 2020 PQ)"/>
    <format id="r2" name="FFVideoFormat3840x2160p50" frameDuration="1000/50000s" width="3840" height="2160" colorSpace="1-1-1 (Rec. 709)"/>
    <format id="r5" frameDuration="10/600s" width="2160" height="3840" colorSpace="9-18-9 (Rec. 2020 HLG)"/>
    <format id="r7" name="FFVideoFormat1080p25" frameDuration="512/12800s" width="1920" height="1080" colorSpace="1-1-1 (Rec. 709)"/>
    <format id="r10" name="FFVideoFormat3840x2160p50" frameDuration="256/12800s" width="3840" height="2160" colorSpace="9-16-9 (Rec. 2020 PQ)"/>
    <asset id="r3" name="C2PA_Clip" uid="AAA" start="0s" duration="240000/50000s" hasVideo="1" format="r2" hasAudio="1" videoSources="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/Original%20Media/C2PA_Clip.mp4"/>
    </asset>
    <asset id="r4" name="IMG_0151" uid="BBB" start="0s" duration="1390/600s" hasVideo="1" format="r5" hasAudio="1" videoSources="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/Original%20Media/IMG_0151.mov"/>
    </asset>
    <asset id="r6" name="TimeCode_HEVC" uid="CCC" start="3600s" duration="660480/12800s" hasVideo="1" format="r7" hasAudio="1" videoSources="1" audioSources="2" audioChannels="1" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/Original%20Media/TimeCode_HEVC.mov"/>
    </asset>
    <asset id="r9" name="ShortPlantHDR" uid="DDD" start="0s" duration="267250/12800s" hasVideo="1" format="r10" hasAudio="1" videoSources="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/Original%20Media/ShortPlantHDR.mp4"/>
    </asset>
    <effect id="r8" name="Basic Title" uid=".../Titles.localized/Basic Title.moti"/>
    <effect id="r11" name="Redact Anonymizer" uid="~/Effects.localized/Aagedal/Redact Anonymizer/Redact Anonymizer.moef"/>
  </resources>
  <library colorProcessing="wide-hdr">
    <event name="2026-11-27" uid="EV-1">
      <project name="TestProject" uid="PR-1">
        <sequence format="r1" duration="38080000/1600000s" tcStart="0s" tcFormat="NDF" audioLayout="stereo" audioRate="48k">
          <spine>
            <clip offset="0s" name="C2PA_Clip" duration="160000/50000s" format="r2" tcFormat="NDF">
              <video ref="r3" offset="0s" duration="240000/50000s">
                <adjust-stabilization type="automatic"/>
                <audio ref="r3" lane="-1" offset="0s" duration="3456000/720000s" srcCh="1, 2"/>
              </video>
            </clip>
            <asset-clip ref="r4" offset="160000/50000s" name="IMG_0151" duration="5700/2500s" format="r5" tcFormat="NDF" audioRole="dialogue">
              <adjust-transform rotation="-13.6905" scale="1.22 1.22"/>
            </asset-clip>
            <asset-clip ref="r6" offset="8768000/1600000s" name="TimeCode_HEVC" start="1158003200/320000s" duration="3507200/320000s" format="r7" tcFormat="NDF" audioRole="dialogue">
              <title ref="r8" lane="2" offset="9049200/2500s" name="Very Basic Title - Basic Title" start="3600s" duration="25100/2500s">
                <text><text-style ref="ts1">Very Basic Title</text-style></text>
              </title>
              <asset-clip ref="r9" lane="1" offset="9065700/2500s" name="ShortPlantHDR" start="7200/2500s" duration="27000/2500s" format="r10" tcFormat="NDF" audioRole="dialogue">
                <adjust-blend mode="14 (Overlay)"/>
              </asset-clip>
              <filter-video-mask>
                <mask-shape name="Shape Mask 1" blendMode="add"/>
                <filter-video ref="r11" name="Redact Anonymizer"/>
              </filter-video-mask>
            </asset-clip>
          </spine>
        </sequence>
      </project>
    </event>
  </library>
</fcpxml>"#;

        let proj = parse_fcpxml_str(xml, "TestProject").expect("parse FCPXML 1.15");
        assert_eq!(proj.source_format, NleFormat::FinalCutPro);
        assert_eq!(
            proj.color_config.working_space.as_deref(),
            Some("9-16-9 (Rec. 2020 PQ)")
        );
        assert_eq!(proj.bins.len(), 1);
        assert_eq!(proj.bins[0].name, "2026-11-27");
        assert_eq!(proj.media_assets.len(), 4);
        assert_eq!(proj.sequences.len(), 1);

        let seq = &proj.sequences[0];
        assert_eq!(seq.width, 1920);
        assert_eq!(seq.height, 1080);
        assert_eq!(seq.frame_rate, Rational::FPS_25);
        assert_eq!(seq.computed_duration_frames(), 595);
        assert_eq!(seq.video_tracks.len(), 2);
        assert_eq!(seq.video_tracks[0].clips.len(), 3);
        assert_eq!(seq.video_tracks[1].clips.len(), 1);

        // Clip 1: trimmed by parent <clip duration="3.2s"> = 80 frames (not inner <video> 120 frames!)
        let c0 = &seq.video_tracks[0].clips[0];
        assert_eq!(c0.start_frame, 0);
        assert_eq!(c0.end_frame, 80);
        assert!(c0.effects.iter().any(|e| e.match_name == "fcp.stabilization"));

        // Clip 2: vertical 4K 60fps clip with scale=122% and rotation=+13.6905 deg CW
        let c1 = &seq.video_tracks[0].clips[1];
        assert_eq!(c1.start_frame, 80);
        assert_eq!(c1.end_frame, 137);
        assert!((c1.motion.scale - 122.0).abs() < 1e-3);
        assert!((c1.motion.rotation - 13.6905).abs() < 1e-3);

        // Clip 3: TimeCode_HEVC with asset start=3600s, clip start=3618.76s -> source_in=469 frames
        let c2 = &seq.video_tracks[0].clips[2];
        assert_eq!(c2.start_frame, 137);
        assert_eq!(c2.end_frame, 411);
        assert_eq!(c2.source_in_frame, 469);
        assert!(c2
            .effects
            .iter()
            .any(|e| e.display_name == "Redact Anonymizer"));

        // V2 connected clip ShortPlantHDR at frame 325..595 with Overlay blend mode
        let v2 = &seq.video_tracks[1].clips[0];
        assert_eq!(v2.name, "ShortPlantHDR");
        assert_eq!(v2.start_frame, 325);
        assert_eq!(v2.end_frame, 595);
        assert_eq!(v2.source_in_frame, 144); // 2.88s @ 50fps
        assert!(v2.effects.iter().any(|e| e.match_name == "fcp.compositing"));

        // Connected Basic Title on lane 2 captured as Cyan marker at frame 160 (dur 251)
        assert_eq!(seq.markers.len(), 1);
        assert_eq!(seq.markers[0].frame, 160);
        assert_eq!(seq.markers[0].duration_frames, 251);
        assert!(seq.markers[0].comment.contains("Very Basic Title"));
    }

    #[test]
    fn roundtrips_multitrack_transitions_retiming_compound_multicam_and_markers() {
        let dir = std::env::temp_dir().join("aagedal_fcpxml_roundtrip");
        let _ = std::fs::create_dir_all(&dir);
        let bundle_dir = dir.join("EditRoundtrip.fcpxmld");
        let _ = std::fs::create_dir_all(&bundle_dir);
        let xml_path = bundle_dir.join("Info.fcpxml");

        let mut asset1_meta = BTreeMap::new();
        asset1_meta.insert("Tape Name".to_string(), "REEL_A".to_string());
        asset1_meta.insert("Scene".to_string(), "12B".to_string());
        let mut custom_meta = BTreeMap::new();
        custom_meta.insert("Secret.MAM.Id".to_string(), "must-not-leak".to_string());

        let project = Project {
            name: "EditRoundtrip".into(),
            source_format: NleFormat::DaVinciResolve,
            source_path: None,
            color_config: ColorConfig::default(),
            scale_to_frame_size: true,
            bins: vec![],
            media_assets: vec![
                MediaAsset {
                    id: "m-cam-a".into(),
                    name: "CamA.mov".into(),
                    path: "/Volumes/Edit/Cam A/Søndag_01.mov".into(),
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
                    metadata: asset1_meta,
                    custom_metadata: custom_meta,
                },
                MediaAsset {
                    id: "m-cam-b".into(),
                    name: "CamB.mov".into(),
                    path: "/Volumes/Edit/CamB.mov".into(),
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
                    metadata: BTreeMap::new(),
                    custom_metadata: BTreeMap::new(),
                },
                MediaAsset {
                    id: "m-music".into(),
                    name: "Score.wav".into(),
                    path: "/Volumes/Edit/Score.wav".into(),
                    media_kind: MediaKind::Audio,
                    width: 0,
                    height: 0,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 500,
                    source_tc_frame: 0,
                    source_tc_string: "00:00:00:00".into(),
                    audio_channels: 2,
                    audio_sample_rate: 48_000,
                    color_space: None,
                    metadata: BTreeMap::new(),
                    custom_metadata: BTreeMap::new(),
                },
            ],
            sequences: vec![
                Sequence {
                    id: "seq-mc".into(),
                    name: "Interview_MC".into(),
                    kind: SequenceKind::MulticamClip,
                    width: 1920,
                    height: 1080,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 200,
                    start_tc_frame: 0,
                    mark_in: None,
                    mark_out: None,
                    video_tracks: vec![
                        Track {
                            index: 1,
                            kind: TrackKind::Video,
                            name: "Camera A".into(),
                            enabled: true,
                            locked: false,
                            muted: false,
                            solo: false,
                            audio_channels: 0,
                            output_bus_id: None,
                            clips: vec![make_clip("mc-a", "CamA.mov", "m-cam-a", "/Volumes/Edit/Cam A/Søndag_01.mov", 0, 200, 0, 200)],
                            transitions: vec![],
                            markers: vec![],
                        },
                        Track {
                            index: 2,
                            kind: TrackKind::Video,
                            name: "Camera B".into(),
                            enabled: true,
                            locked: false,
                            muted: false,
                            solo: false,
                            audio_channels: 0,
                            output_bus_id: None,
                            clips: vec![make_clip("mc-b", "CamB.mov", "m-cam-b", "/Volumes/Edit/CamB.mov", 0, 200, 0, 200)],
                            transitions: vec![],
                            markers: vec![],
                        },
                    ],
                    audio_tracks: vec![],
                    audio_buses: vec![],
                    markers: vec![],
                    metadata: BTreeMap::new(),
                },
                Sequence {
                    id: "seq-main".into(),
                    name: "Main_Timeline".into(),
                    kind: SequenceKind::Timeline,
                    width: 1920,
                    height: 1080,
                    frame_rate: Rational::FPS_25,
                    duration_frames: 300,
                    start_tc_frame: 90_000,
                    mark_in: None,
                    mark_out: None,
                    video_tracks: vec![
                        Track {
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
                                {
                                    let mut c = make_clip("c1", "CamA.mov", "m-cam-a", "/Volumes/Edit/Cam A/Søndag_01.mov", 0, 100, 25, 125);
                                    c.link_group_id = Some("lg1".into());
                                    c.motion.scale = 115.0;
                                    c.motion.position_x = 0.1;
                                    c.motion.position_y = -0.05;
                                    c.motion.rotation = 12.5;
                                    c.motion.crop_left = 10.0;
                                    c.opacity = 85.0;
                                    c
                                },
                                {
                                    let mut c = make_clip("c2", "CamB_Rev", "m-cam-b", "/Volumes/Edit/CamB.mov", 100, 200, 50, 150);
                                    c.reversed = true;
                                    c.speed_percent = 100.0;
                                    c
                                },
                                {
                                    let mut c = make_clip("c3", "Interview_MC", "seq-mc", "", 200, 300, 0, 100);
                                    c.source_kind = ClipSourceKind::MulticamClip;
                                    c.active_angle = Some("Camera B".into());
                                    c
                                },
                            ],
                            transitions: vec![Transition {
                                name: "Cross Dissolve".into(),
                                effect_id: "Cross Dissolve".into(),
                                start_frame: 88,
                                end_frame: 112,
                                alignment: "center".into(),
                            }],
                            markers: vec![],
                        },
                        Track {
                            index: 2,
                            kind: TrackKind::Video,
                            name: "V2".into(),
                            enabled: true,
                            locked: false,
                            muted: false,
                            solo: false,
                            audio_channels: 0,
                            output_bus_id: None,
                            clips: vec![make_clip("c-overlay", "CamB.mov", "m-cam-b", "/Volumes/Edit/CamB.mov", 25, 75, 10, 60)],
                            transitions: vec![],
                            markers: vec![],
                        },
                    ],
                    audio_tracks: vec![
                        Track {
                            index: 1,
                            kind: TrackKind::Audio,
                            name: "A1".into(),
                            enabled: true,
                            locked: false,
                            muted: false,
                            solo: false,
                            audio_channels: 2,
                            output_bus_id: None,
                            clips: vec![{
                                let mut a = make_clip("a1", "CamA.mov", "m-cam-a", "/Volumes/Edit/Cam A/Søndag_01.mov", 0, 100, 25, 125);
                                a.link_group_id = Some("lg1".into());
                                a.audio_channel = 1;
                                a
                            }],
                            transitions: vec![],
                            markers: vec![],
                        },
                        Track {
                            index: 2,
                            kind: TrackKind::Audio,
                            name: "A2".into(),
                            enabled: true,
                            locked: false,
                            muted: false,
                            solo: false,
                            audio_channels: 2,
                            output_bus_id: None,
                            clips: vec![{
                                let mut a = make_clip("a-music", "Score.wav", "m-music", "/Volumes/Edit/Score.wav", 0, 250, 0, 250);
                                a.media_kind = MediaKind::Audio;
                                a.audio_channel = 1;
                                a
                            }],
                            transitions: vec![],
                            markers: vec![],
                        },
                    ],
                    audio_buses: vec![],
                    markers: vec![Marker {
                        frame: 50,
                        duration_frames: 1,
                        name: "Check Grade".into(),
                        comment: "Verify skin tones".into(),
                        color: "Red".into(),
                        scope: MarkerScope::Sequence,
                    }],
                    metadata: BTreeMap::new(),
                },
            ],
            metadata: BTreeMap::new(),
            custom_metadata_schema: BTreeMap::new(),
        };

        let opts = ConversionOptions {
            target_format: NleFormat::FinalCutPro,
            ..Default::default()
        };
        let rep = write_fcpxml_file(&project, &xml_path, &opts).expect("write fcpxml");
        assert_eq!(rep.sequences_converted, 1);
        assert_eq!(rep.video_clips_converted, 4);
        assert_eq!(rep.audio_clips_converted, 2);
        assert_eq!(rep.markers_written, 1);

        // Verify custom metadata did NOT leak
        let raw_xml = std::fs::read_to_string(&xml_path).unwrap();
        assert!(!raw_xml.contains("Secret.MAM.Id"));
        assert!(!raw_xml.contains("must-not-leak"));

        // Read back via the .fcpxmld bundle directory path!
        let reread = read_fcpxml_file(&bundle_dir).expect("read .fcpxmld bundle");
        assert_eq!(reread.source_format, NleFormat::FinalCutPro);

        let main = reread
            .sequences
            .iter()
            .find(|s| s.kind == SequenceKind::Timeline)
            .expect("main timeline");
        assert_eq!(main.start_tc_frame, 90_000);
        assert_eq!(main.video_tracks.len(), 2);
        assert_eq!(main.video_tracks[0].clips.len(), 3);
        assert_eq!(main.video_tracks[1].clips.len(), 1);
        assert_eq!(main.video_tracks[0].transitions.len(), 1);
        assert_eq!(main.video_tracks[0].transitions[0].start_frame, 88);
        assert_eq!(main.video_tracks[0].transitions[0].end_frame, 112);

        // Verify V1 clip 1 transform, crop, opacity, source_in, and Unicode path
        let v0 = &main.video_tracks[0].clips[0];
        assert_eq!(v0.media_path, "/Volumes/Edit/Cam A/Søndag_01.mov");
        assert_eq!(v0.source_in_frame, 25);
        assert!((v0.motion.scale - 115.0).abs() < 1e-2);
        assert!((v0.motion.position_x - 0.1).abs() < 1e-3);
        assert!((v0.motion.position_y - (-0.05)).abs() < 1e-3);
        assert!((v0.motion.rotation - 12.5).abs() < 1e-2);
        assert!((v0.motion.crop_left - 10.0).abs() < 1e-2);
        assert!((v0.opacity - 85.0).abs() < 1e-2);

        // Verify V1 clip 2 reversed retiming
        let v1 = &main.video_tracks[0].clips[1];
        assert!(v1.reversed);
        assert!((v1.speed_percent - 100.0).abs() < 0.5);

        // Verify V1 clip 3 multicam clip + active angle
        let v2 = &main.video_tracks[0].clips[2];
        assert_eq!(v2.source_kind, ClipSourceKind::MulticamClip);
        assert_eq!(v2.active_angle.as_deref(), Some("Camera B"));

        // Verify V2 connected clip timing
        let ov = &main.video_tracks[1].clips[0];
        assert_eq!(ov.start_frame, 25);
        assert_eq!(ov.end_frame, 75);

        // Verify audio tracks include linked CamA + connected Score.wav
        let all_audio: Vec<&Clip> = main.audio_tracks.iter().flat_map(|t| t.clips.iter()).collect();
        assert!(all_audio.iter().any(|c| c.name == "CamA.mov"));
        assert!(all_audio.iter().any(|c| c.name == "Score.wav" && c.end_frame == 250));

        // Verify marker survived with exact frame & color
        assert_eq!(main.markers.len(), 1);
        assert_eq!(main.markers[0].frame, 50);
        assert_eq!(main.markers[0].name, "Check Grade");
        assert_eq!(main.markers[0].color, "Red");

        // Verify standard metadata survived
        assert!(reread.media_assets.iter().any(|a| {
            a.metadata.get("Tape Name").map(String::as_str) == Some("REEL_A")
                && a.metadata.get("Scene").map(String::as_str) == Some("12B")
        }));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_clip(
        id: &str,
        name: &str,
        media_id: &str,
        path: &str,
        start: i64,
        end: i64,
        src_in: i64,
        src_out: i64,
    ) -> Clip {
        Clip {
            id: id.into(),
            name: name.into(),
            media_id: media_id.into(),
            media_path: path.into(),
            media_kind: MediaKind::Video,
            source_kind: ClipSourceKind::Media,
            active_angle: None,
            start_frame: start,
            end_frame: end,
            source_in_frame: src_in,
            source_out_frame: src_out,
            source_in_ticks: None,
            source_out_ticks: None,
            source_fps: Rational::FPS_25,
            source_width: 1920,
            source_height: 1080,
            speed_percent: 100.0,
            reversed: false,
            enabled: true,
            audio_channel: 0,
            link_group_id: None,
            label_name: None,
            label_preset_index: None,
            motion: MotionTransform::default(),
            opacity: 100.0,
            lumetri_effects: vec![],
            effects: vec![],
            markers: vec![],
        }
    }
}
