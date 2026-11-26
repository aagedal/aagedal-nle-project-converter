pub mod fixture;
pub mod lumetri;
pub mod reader;

pub use fixture::{sample_prproj_xml, write_sample_prproj};
pub use reader::{parse_prproj_xml, read_prproj_file, PremiereReader};

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
}
