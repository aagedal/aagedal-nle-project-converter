pub mod fixture;
pub mod lumetri;
pub mod reader;
pub mod writer;

pub use fixture::{sample_prproj_xml, write_sample_prproj};
pub use reader::{parse_prproj_xml, read_prproj_file, PremiereReader};
pub use writer::{build_prproj_xml, write_prproj_file, PremiereWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::Rational;

    #[test]
    fn parses_synthetic_prproj_end_to_end() {
        let xml = sample_prproj_xml();
        let project = parse_prproj_xml(&xml, "Studio Demo").expect("parse sample prproj");
        assert_eq!(project.name, "Studio Demo");
        assert_eq!(project.color_config.working_space.as_deref(), Some("Rec. 709"));
        assert_eq!(project.bins.len(), 2);
        assert_eq!(project.sequences.len(), 1);

        let seq = &project.sequences[0];
        assert_eq!(seq.name, "Main_Edit_v03");
        assert_eq!(seq.width, 1920);
        assert_eq!(seq.height, 1080);
        assert_eq!(seq.frame_rate, Rational::FPS_25);
        assert_eq!(seq.video_tracks.len(), 1);
        assert_eq!(seq.audio_tracks.len(), 1);
        assert_eq!(seq.video_tracks[0].clips.len(), 2);
        assert_eq!(seq.markers.len(), 1);
        assert_eq!(seq.markers[0].name, "Intro Cut");

        let c1 = &seq.video_tracks[0].clips[0];
        assert_eq!(c1.start_frame, 0);
        assert_eq!(c1.end_frame, 125);
        assert!((c1.motion.scale - 55.0).abs() < 1e-6);
        assert_eq!(c1.lumetri_effects.len(), 1);
        assert_eq!(c1.lumetri_effects[0].exposure, Some(0.45));
        assert_eq!(c1.effects.len(), 1);
        assert_eq!(c1.effects[0].match_name, "AE.ADBE Ultra Key");

        let c2 = &seq.video_tracks[0].clips[1];
        assert_eq!(c2.speed_percent, 50.0);
        assert_eq!(c2.effects.len(), 3);
    }

    #[test]
    fn handles_object_id_collisions_transitions_audio_channels_and_essential_sound() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PremiereData Version="3">
  <Project ObjectID="1">
    <Node><Properties>
      <ColumnList ObjectID="10"><StringColumn ObjectID="20"/></ColumnList>
    </Properties></Node>
  </Project>
  <Sequence ObjectID="2" ObjectUID="seq-uid-1">
    <Name>Broadcast Edit</Name>
    <TrackGroups>
      <TrackGroup><Second ObjectRef="3"/></TrackGroup>
      <TrackGroup><Second ObjectRef="4"/></TrackGroup>
    </TrackGroups>
  </Sequence>
  <VideoTrackGroup ObjectID="3">
    <FrameRect>0,0,1920,1080</FrameRect>
    <TrackGroup><FrameRate>5080320000</FrameRate><Tracks><Track ObjectRef="5"/></Tracks></TrackGroup>
  </VideoTrackGroup>
  <AudioTrackGroup ObjectID="4">
    <TrackGroup><Tracks><Track ObjectRef="6"/></Tracks></TrackGroup>
  </AudioTrackGroup>
  <VideoClipTrack ObjectID="5">
    <ClipTrack>
      <ClipItems><TrackItems><TrackItem ObjectRef="7"/></TrackItems></ClipItems>
      <TransitionItems><TrackItems><TrackItem ObjectRef="8"/></TrackItems></TransitionItems>
    </ClipTrack>
  </VideoClipTrack>
  <AudioClipTrack ObjectID="6">
    <ClipTrack>
      <ClipItems><TrackItems><TrackItem ObjectRef="9"/></TrackItems></ClipItems>
      <TransitionItems><TrackItems><TrackItem ObjectRef="11"/></TrackItems></TransitionItems>
    </ClipTrack>
  </AudioClipTrack>
  <VideoClipTrackItem ObjectID="7">
    <ClipTrackItem>
      <TrackItem><Start>0</Start><End>508032000000</End></TrackItem>
      <SubClip ObjectRef="12"/>
    </ClipTrackItem>
  </VideoClipTrackItem>
  <VideoTransitionTrackItem ObjectID="8">
    <TransitionTrackItem>
      <TrackItem><Start>482630400000</Start><End>533433600000</End></TrackItem>
      <HasOutgoingClip>true</HasOutgoingClip>
      <HasIncomingClip>true</HasIncomingClip>
      <DisplayName>Whip</DisplayName>
      <MatchName>AE.Impact_Whip</MatchName>
    </TransitionTrackItem>
  </VideoTransitionTrackItem>
  <AudioClipTrackItem ObjectID="9">
    <ClipTrackItem>
      <TrackItem>
        <Node><Properties>
          <ESP.AdjustmentsModeID>dialog</ESP.AdjustmentsModeID>
          <ESP.RestorationSection.9>60</ESP.RestorationSection.9>
        </Properties></Node>
        <Start>0</Start><End>508032000000</End>
      </TrackItem>
      <SubClip ObjectRef="13"/>
    </ClipTrackItem>
  </AudioClipTrackItem>
  <AudioTransitionTrackItem ObjectID="11">
    <TransitionTrackItem>
      <TrackItem><End>35562240000</End></TrackItem>
      <HasOutgoingClip>false</HasOutgoingClip>
      <HasIncomingClip>true</HasIncomingClip>
      <DisplayName>Constant Power</DisplayName>
      <MatchName>Constant Power</MatchName>
    </TransitionTrackItem>
  </AudioTransitionTrackItem>
  <SubClip ObjectID="12"><Name>CameraA.mp4</Name><Clip ObjectRef="14"/></SubClip>
  <SubClip ObjectID="13"><Name>CameraA.mp4</Name><Clip ObjectRef="15"/></SubClip>
  <VideoClip ObjectID="14"><Clip><Source ObjectRef="20"/><InPoint>0</InPoint><OutPoint>508032000000</OutPoint></Clip></VideoClip>
  <AudioClip ObjectID="15">
    <Clip><Source ObjectRef="21"/><InPoint>0</InPoint><OutPoint>508032000000</OutPoint></Clip>
    <SecondaryContents><SecondaryContentItem Index="0" ObjectRef="16"/></SecondaryContents>
  </AudioClip>
  <SecondaryContent ObjectID="16"><ChannelIndex>1</ChannelIndex></SecondaryContent>
  <VideoMediaSource ObjectID="20"><MediaSource><Media ObjectURef="media-uid-1"/></MediaSource></VideoMediaSource>
  <AudioMediaSource ObjectID="21"><MediaSource><Media ObjectURef="media-uid-1"/></MediaSource></AudioMediaSource>
  <Media ObjectUID="media-uid-1">
    <ActualMediaFilePath>/Volumes/Media/CameraA.mp4</ActualMediaFilePath>
  </Media>
</PremiereData>"#;

        let project = parse_prproj_xml(xml, "CollisionTest").expect("parse prproj");
        assert_eq!(project.media_assets.len(), 1);
        let seq = &project.sequences[0];
        assert_eq!(seq.frame_rate, Rational::FPS_50);
        // VideoMediaSource ObjectID="20" must NOT be shadowed by nested StringColumn ObjectID="20"
        let vclip = &seq.video_tracks[0].clips[0];
        assert_eq!(vclip.media_path, "/Volumes/Media/CameraA.mp4");
        assert_eq!(seq.video_tracks[0].transitions.len(), 1);
        assert_eq!(seq.video_tracks[0].transitions[0].effect_id, "AE.Impact_Whip");

        // Audio clip channel index (0-based 1 -> 1-based 2), Essential Sound, and omitted Start=0 transition
        let aclip = &seq.audio_tracks[0].clips[0];
        assert_eq!(aclip.audio_channel, 2);
        assert_eq!(aclip.effects.len(), 1);
        assert_eq!(aclip.effects[0].match_name, "Premiere.EssentialSound.dialog");
        assert_eq!(seq.audio_tracks[0].transitions.len(), 1);
        assert_eq!(seq.audio_tracks[0].transitions[0].start_frame, 0);
        assert_eq!(seq.audio_tracks[0].transitions[0].end_frame, 7);
        assert_eq!(seq.audio_tracks[0].transitions[0].alignment, "start");
    }

    #[test]
    fn parses_reverse_interpret_fps_nested_sequence_and_multicam() {
        use nle_core::{ClipSourceKind, SequenceKind};

        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PremiereData Version="3">
  <Sequence ObjectID="1" ObjectUID="mc-src-uid">
    <Name>MC_3Cam_Source</Name>
    <TrackGroups>
      <TrackGroup><Second ObjectRef="2"/></TrackGroup>
    </TrackGroups>
  </Sequence>
  <VideoTrackGroup ObjectID="2">
    <FrameRect>0,0,1920,1080</FrameRect>
    <TrackGroup><FrameRate>10160640000</FrameRate><Tracks/></TrackGroup>
  </VideoTrackGroup>

  <Sequence ObjectID="10" ObjectUID="sub-seq-uid">
    <Name>Sub_B_Roll</Name>
    <TrackGroups>
      <TrackGroup><Second ObjectRef="11"/></TrackGroup>
    </TrackGroups>
  </Sequence>
  <VideoTrackGroup ObjectID="11">
    <FrameRect>0,0,1920,1080</FrameRect>
    <TrackGroup><FrameRate>10160640000</FrameRate><Tracks/></TrackGroup>
  </VideoTrackGroup>

  <Sequence ObjectID="20" ObjectUID="main-seq-uid">
    <Name>Main_Edit</Name>
    <TrackGroups>
      <TrackGroup><Second ObjectRef="21"/></TrackGroup>
    </TrackGroups>
  </Sequence>
  <VideoTrackGroup ObjectID="21">
    <FrameRect>0,0,1920,1080</FrameRect>
    <TrackGroup><FrameRate>10160640000</FrameRate><Tracks><Track ObjectRef="22"/></Tracks></TrackGroup>
  </VideoTrackGroup>
  <VideoClipTrack ObjectID="22">
    <ClipTrack>
      <ClipItems><TrackItems>
        <TrackItem ObjectRef="30"/>
        <TrackItem ObjectRef="31"/>
        <TrackItem ObjectRef="32"/>
      </TrackItems></ClipItems>
    </ClipTrack>
  </VideoClipTrack>

  <VideoClipTrackItem ObjectID="30">
    <ClipTrackItem>
      <TrackItem><Start>0</Start><End>1016064000000</End></TrackItem>
      <SubClip ObjectRef="40"/>
    </ClipTrackItem>
  </VideoClipTrackItem>
  <VideoClipTrackItem ObjectID="31">
    <ClipTrackItem>
      <TrackItem><Start>1016064000000</Start><End>2032128000000</End></TrackItem>
      <SubClip ObjectRef="41"/>
    </ClipTrackItem>
  </VideoClipTrackItem>
  <VideoClipTrackItem ObjectID="32">
    <ClipTrackItem>
      <TrackItem><Start>2032128000000</Start><End>3048192000000</End></TrackItem>
      <SubClip ObjectRef="42"/>
    </ClipTrackItem>
  </VideoClipTrackItem>

  <SubClip ObjectID="40"><Name>ReverseClip.mov</Name><Clip ObjectRef="50"/></SubClip>
  <SubClip ObjectID="41"><Name>Sub_B_Roll</Name><Clip ObjectRef="51"/></SubClip>
  <SubClip ObjectID="42"><Name>MC_3Cam_Source</Name><Clip ObjectRef="52"/></SubClip>

  <VideoClip ObjectID="50">
    <Clip>
      <Source ObjectRef="60"/>
      <PlayBackwards>true</PlayBackwards>
      <InPoint>0</InPoint><OutPoint>1016064000000</OutPoint>
    </Clip>
  </VideoClip>
  <VideoClip ObjectID="51">
    <Clip>
      <Source ObjectRef="61"/>
      <InPoint>0</InPoint><OutPoint>1016064000000</OutPoint>
    </Clip>
  </VideoClip>
  <VideoClip ObjectID="52">
    <Clip>
      <Source ObjectRef="62"/>
      <SelectedTrackIndex>2</SelectedTrackIndex>
      <IsMulticam>true</IsMulticam>
      <InPoint>0</InPoint><OutPoint>1016064000000</OutPoint>
    </Clip>
  </VideoClip>

  <VideoMediaSource ObjectID="60"><MediaSource><Media ObjectURef="media-1"/></MediaSource></VideoMediaSource>
  <VideoSequenceSource ObjectID="61"><SequenceSource><Sequence ObjectURef="sub-seq-uid"/></SequenceSource></VideoSequenceSource>
  <VideoSequenceSource ObjectID="62"><SequenceSource><Sequence ObjectURef="mc-src-uid"/></SequenceSource></VideoSequenceSource>

  <Media ObjectUID="media-1">
    <ActualMediaFilePath>/Media/ReverseClip.mov</ActualMediaFilePath>
    <VideoStream ObjectRef="70"/>
  </Media>
  <VideoStream ObjectID="70">
    <IsFrameRateOverridden>true</IsFrameRateOverridden>
    <OveriddenFrameRate>10160640000</OveriddenFrameRate>
    <FrameRate>5080320000</FrameRate>
    <FrameRect>0,0,1920,1080</FrameRect>
  </VideoStream>
</PremiereData>"#;

        let project = parse_prproj_xml(xml, "Premiere2026SchemaTest").expect("parse prproj");
        // Only the actual media file should be in media_assets (not the nested/multicam sequences)
        assert_eq!(project.media_assets.len(), 1);
        assert_eq!(project.media_assets[0].frame_rate, Rational::FPS_25);

        let mc_seq = project.sequences.iter().find(|s| s.id == "mc-src-uid").unwrap();
        assert_eq!(mc_seq.kind, SequenceKind::MulticamClip);

        let main = project.sequences.iter().find(|s| s.id == "main-seq-uid").unwrap();
        let clips = &main.video_tracks[0].clips;
        assert_eq!(clips.len(), 3);

        // Clip 0: reversed -100% + interpret footage FPS override (50 -> 25)
        assert!(clips[0].reversed);
        assert!((clips[0].speed_percent - (-100.0)).abs() < 1e-6);
        assert_eq!(clips[0].source_fps, Rational::FPS_25);
        assert_eq!(clips[0].source_kind, ClipSourceKind::Media);

        // Clip 1: nested sequence
        assert_eq!(clips[1].source_kind, ClipSourceKind::NestedSequence);
        assert_eq!(clips[1].media_id, "sub-seq-uid");

        // Clip 2: multicam clip with SelectedTrackIndex=2 -> Camera 3
        assert_eq!(clips[2].source_kind, ClipSourceKind::MulticamClip);
        assert_eq!(clips[2].media_id, "mc-src-uid");
        assert_eq!(clips[2].active_angle.as_deref(), Some("Camera 3"));
    }

    #[test]
    fn writes_and_rereads_native_prproj_with_multicam_nested_markers_transitions_and_fx() {
        use nle_core::*;
        use std::collections::BTreeMap;

        let project = Project {
            name: "Resolve To Premiere RT".into(),
            source_format: NleFormat::DaVinciResolve,
            source_path: None,
            color_config: ColorConfig::default(),
            scale_to_frame_size: true,
            bins: vec![
                ProjectBin {
                    id: "bin-media".into(),
                    name: "01_Media".into(),
                    parent_id: None,
                    media_paths: vec!["/Media/ÖgonKrålærepø.mov".into()],
                    sequence_ids: vec![],
                    custom_metadata: BTreeMap::from([("External.Folder.Id".into(), "folder-01".into())]),
                },
                ProjectBin {
                    id: "bin-seqs".into(),
                    name: "02_Sequences".into(),
                    parent_id: None,
                    media_paths: vec![],
                    sequence_ids: vec!["seq-main".into(), "seq-mc".into()],
                    custom_metadata: BTreeMap::new(),
                },
            ],
            media_assets: vec![MediaAsset {
                id: "m1".into(),
                name: "ÖgonKrålærepø.mov".into(),
                path: "/Media/ÖgonKrålærepø.mov".into(),
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
                alpha_mode: AlphaMode::Straight,
                data_levels: DataLevels::Auto,
                pixel_aspect_ratio: Some(1.3333),
                metadata: BTreeMap::from([
                    ("Description".into(), "Field interview".into()),
                    ("Comments".into(), "Keep audio".into()),
                    ("Scene".into(), "4B".into()),
                    ("Tape Name".into(), "CARD_A01".into()),
                ]),
                custom_metadata: BTreeMap::from([
                    ("External.Item.Id".into(), "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()),
                    ("External.Artifact".into(), r#"{"type":"Segment","id":"seg-1"}"#.into()),
                ]),
            }],
            sequences: vec![
                Sequence {
                    id: "seq-mc".into(),
                    name: "MC_3Cam".into(),
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
                            name: "Camera 1".into(),
                            enabled: true,
                            locked: false,
                            muted: false,
                            solo: false,
                            audio_channels: 0,
                            output_bus_id: None,
                            clips: vec![Clip {
                                id: "ang1".into(),
                                name: "ÖgonKrålærepø.mov".into(),
                                media_id: "m1".into(),
                                media_path: "/Media/ÖgonKrålærepø.mov".into(),
                                media_kind: MediaKind::Video,
                                source_kind: ClipSourceKind::Media,
                                active_angle: None,
                                start_frame: 0,
                                end_frame: 200,
                                source_in_frame: 0,
                                source_out_frame: 200,
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
                    duration_frames: 250,
                    start_tc_frame: 90_000, // 01:00:00:00 @ 25fps
                    mark_in: Some(10),
                    mark_out: Some(200),
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
                        clips: vec![
                            Clip {
                                id: "c-v1".into(),
                                name: "ÖgonKrålærepø.mov".into(),
                                media_id: "m1".into(),
                                media_path: "/Media/ÖgonKrålærepø.mov".into(),
                                media_kind: MediaKind::Video,
                                source_kind: ClipSourceKind::Media,
                                active_angle: None,
                                start_frame: 0,
                                end_frame: 100,
                                source_in_frame: 25,
                                source_out_frame: 75,
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
                                audio_channel: 1,
                                link_group_id: Some("grp-1".into()),
                                label_name: Some("BE.Prefs.LabelColors.8".into()),
                                label_preset_index: Some(8),
                                motion: MotionTransform {
                                    scale: 125.0,
                                    position_x: 0.1,
                                    position_y: -0.05,
                                    rotation: 4.5,
                                    crop_left: 5.0,
                                    ..Default::default()
                                },
                                opacity: 80.0,
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
                            },
                            Clip {
                                id: "c-mc".into(),
                                name: "MC_3Cam".into(),
                                media_id: "seq-mc".into(),
                                media_path: String::new(),
                                media_kind: MediaKind::Video,
                                source_kind: ClipSourceKind::MulticamClip,
                                active_angle: Some("Camera 3".into()),
                                start_frame: 100,
                                end_frame: 200,
                                source_in_frame: 0,
                                source_out_frame: 100,
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
                                lumetri_effects: vec![],
                                effects: vec![],
                                markers: vec![],
                            },
                        ],
                        transitions: vec![Transition {
                            name: "Cross Dissolve".into(),
                            effect_id: "Cross Dissolve".into(),
                            start_frame: 90,
                            end_frame: 110,
                            alignment: "center".into(),
                        }],
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
                            id: "c-a1".into(),
                            name: "ÖgonKrålærepø.mov".into(),
                            media_id: "m1".into(),
                            media_path: "/Media/ÖgonKrålærepø.mov".into(),
                            media_kind: MediaKind::Audio,
                            source_kind: ClipSourceKind::Media,
                            active_angle: None,
                            start_frame: 0,
                            end_frame: 100,
                            source_in_frame: 25,
                            source_out_frame: 75,
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
                            link_group_id: Some("grp-1".into()),
                            label_name: None,
                            label_preset_index: None,
                            motion: MotionTransform::default(),
                            opacity: 100.0,
                            lumetri_effects: vec![],
                            effects: vec![],
                            markers: vec![],
                        }],
                        transitions: vec![Transition {
                            name: "Cross Fade +3DB".into(),
                            effect_id: "Cross Fade +3DB".into(),
                            start_frame: 0,
                            end_frame: 12,
                            alignment: "start".into(),
                        }],
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
                    ],
                    markers: vec![Marker {
                        frame: 50,
                        duration_frames: 25,
                        name: "Review Cut".into(),
                        comment: "Check dissolve & multicam".into(),
                        color: "Purple".into(),
                        scope: MarkerScope::Sequence,
                    }],
                    metadata: BTreeMap::new(),
                },
            ],
            metadata: BTreeMap::new(),
            custom_metadata_schema: BTreeMap::from([
                ("External.Item.Id".into(), "External Item ID".into()),
                ("External.Artifact".into(), "External Artifact".into()),
                ("External.Folder.Id".into(), "External Folder ID".into()),
            ]),
        };

        let out_path = std::env::temp_dir().join("aagedal_nle_prproj_writer_rt.prproj");
        let opts = ConversionOptions {
            target_format: NleFormat::PremierePro,
            ..Default::default()
        };
        let report = write_prproj_file(&project, &out_path, &opts).expect("write .prproj");
        assert_eq!(report.sequences_converted, 2);
        assert_eq!(report.video_clips_converted, 3);
        assert_eq!(report.audio_clips_converted, 1);
        assert_eq!(report.bins_created, 2);
        assert_eq!(report.markers_written, 1);
        assert_eq!(report.grades_attached, 1);

        let reread = read_prproj_file(&out_path).expect("reread generated .prproj");
        assert_eq!(reread.bins.len(), 2);
        assert_eq!(
            reread
                .bins
                .iter()
                .find(|b| b.name == "01_Media")
                .and_then(|b| b.custom_metadata.get("External.Folder.Id"))
                .map(String::as_str),
            Some("folder-01")
        );
        assert_eq!(reread.sequences.len(), 2);
        assert_eq!(reread.media_assets.len(), 1);
        assert_eq!(reread.media_assets[0].path, "/Media/ÖgonKrålærepø.mov");
        assert_eq!(
            reread.media_assets[0].metadata.get("Description").map(String::as_str),
            Some("Field interview")
        );
        assert_eq!(
            reread.media_assets[0].metadata.get("Tape Name").map(String::as_str),
            Some("CARD_A01")
        );
        assert_eq!(
            reread.media_assets[0]
                .custom_metadata
                .get("External.Item.Id")
                .map(String::as_str),
            Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        );
        assert_eq!(
            reread.media_assets[0]
                .custom_metadata
                .get("External.Artifact")
                .map(String::as_str),
            Some(r#"{"type":"Segment","id":"seg-1"}"#)
        );
        assert_eq!(
            reread.custom_metadata_schema.get("External.Item.Id").map(String::as_str),
            Some("External Item ID")
        );

        let mc_seq = reread.sequences.iter().find(|s| s.name == "MC_3Cam").unwrap();
        assert_eq!(mc_seq.kind, SequenceKind::MulticamClip);

        let main = reread
            .sequences
            .iter()
            .find(|s| s.name == "Main_Timeline")
            .unwrap();
        assert_eq!(main.start_tc_frame, 90_000);
        assert_eq!(main.mark_in, Some(10));
        assert_eq!(main.mark_out, Some(200));
        assert_eq!(main.audio_buses.len(), 2);
        assert_eq!(main.audio_buses[0].name, "MAIN MIX");
        assert_eq!(main.audio_buses[1].name, "DIALOGUE");
        assert_eq!(main.audio_tracks[0].audio_channels, 1);
        assert_eq!(
            main.audio_tracks[0].output_bus_id.as_deref(),
            Some(main.audio_buses[1].id.as_str())
        );
        assert_eq!(main.markers.len(), 1);
        assert_eq!(main.markers[0].name, "Review Cut");
        assert_eq!(main.markers[0].comment, "Check dissolve & multicam");
        assert_eq!(main.video_tracks[0].transitions.len(), 1);
        assert_eq!(main.video_tracks[0].transitions[0].effect_id, "AE.ADBE Cross Dissolve");
        assert_eq!(main.audio_tracks[0].transitions.len(), 1);
        assert_eq!(main.audio_tracks[0].transitions[0].effect_id, "Constant Power");
        assert!(main.audio_tracks[0].locked);

        let v0 = &main.video_tracks[0].clips[0];
        let a0 = &main.audio_tracks[0].clips[0];
        assert!((v0.speed_percent - 50.0).abs() < 1e-3);
        assert_eq!(v0.retime_interpolation, RetimeInterpolation::OpticalFlow);
        assert_eq!(reread.media_assets[0].alpha_mode, AlphaMode::Straight);
        assert!(
            reread.media_assets[0]
                .pixel_aspect_ratio
                .is_some_and(|p| (p - 1.3333).abs() < 1e-3)
        );
        assert!((v0.motion.scale - 125.0).abs() < 1e-4);
        assert!((v0.motion.position_x - 0.1).abs() < 1e-4);
        assert!((v0.motion.position_y - (-0.05)).abs() < 1e-4);
        assert!((v0.motion.rotation - 4.5).abs() < 1e-4);
        assert!((v0.motion.crop_left - 5.0).abs() < 1e-4);
        assert!((v0.opacity - 80.0).abs() < 1e-4);
        assert_eq!(v0.label_preset_index, Some(8));
        assert_eq!(v0.lumetri_effects.len(), 1);
        assert_eq!(v0.lumetri_effects[0].exposure, Some(0.5));
        assert!(v0.link_group_id.is_some());
        assert_eq!(v0.link_group_id, a0.link_group_id);

        let v1 = &main.video_tracks[0].clips[1];
        assert_eq!(v1.source_kind, ClipSourceKind::MulticamClip);
        assert_eq!(v1.active_angle.as_deref(), Some("Camera 3"));
        assert!(v1.reversed);
        assert!((v1.speed_percent - (-100.0)).abs() < 1e-3);

        let _ = std::fs::remove_file(&out_path);
    }
}
