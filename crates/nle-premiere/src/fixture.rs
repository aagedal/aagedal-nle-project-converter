use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::path::Path;

/// Returns a rich, valid synthetic Adobe Premiere Pro `<PremiereData>` XML document
/// containing 2 sequences, nested bins, video + audio clips, Motion scale/rotation,
/// Lumetri Color, sequence markers, clip labels, AND app-exclusive effects
/// (Warp Stabilizer, Gaussian Blur, Ultra Key, DeNoise, and a third-party plugin)
/// so users and automated tests can exercise the entire direct conversion pipeline.
pub fn sample_prproj_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<PremiereData Version="3">
  <ProjectSettings ObjectID="1">
    <ShouldScaleMedia>true</ShouldScaleMedia>
  </ProjectSettings>
  <WorkingColorSpace>{"baseColorProfile":{"colorProfileName":"Rec. 709"}}</WorkingColorSpace>
  <OutputColorSpace>{"baseColorProfile":{"colorProfileName":"Rec. 709"}}</OutputColorSpace>

  <BinProjectItem ObjectUID="bin-footage-001">
    <ProjectItem><Name>01_Camera_Originals</Name></ProjectItem>
    <ProjectItemContainer>
      <Items>
        <Item ObjectRef="cpi-1"/>
        <Item ObjectRef="cpi-2"/>
      </Items>
    </ProjectItemContainer>
  </BinProjectItem>

  <BinProjectItem ObjectUID="bin-edits-001">
    <ProjectItem><Name>02_Sequences</Name></ProjectItem>
    <ProjectItemContainer>
      <Items>
        <Item ObjectRef="cpi-seq-1"/>
      </Items>
    </ProjectItemContainer>
  </BinProjectItem>

  <ClipProjectItem ObjectID="cpi-1">
    <MasterClip ObjectURef="mc-1"/>
  </ClipProjectItem>
  <ClipProjectItem ObjectID="cpi-2">
    <MasterClip ObjectURef="mc-2"/>
  </ClipProjectItem>
  <ClipProjectItem ObjectID="cpi-seq-1">
    <MasterClip ObjectURef="mc-seq-1"/>
  </ClipProjectItem>

  <MasterClip ObjectUID="mc-seq-1">
    <Clips><Clip ObjectRef="clip-seq-1"/></Clips>
  </MasterClip>
  <VideoClip ObjectID="clip-seq-1">
    <Clip><Source ObjectRef="src-seq-1"/></Clip>
  </VideoClip>
  <VideoClipSource ObjectID="src-seq-1">
    <SequenceSource><Sequence ObjectURef="seq-main-001"/></SequenceSource>
  </VideoClipSource>

  <!-- Media 1: 4K 25fps A-Cam clip -->
  <VideoStream ObjectID="vs-1">
    <FrameRect>0,0,3840,2160</FrameRect>
  </VideoStream>
  <Media ObjectUID="media-1">
    <FilePath>/Media/StudioShoot/A001_C004_Interview.mov</FilePath>
    <Title>A001_C004_Interview.mov</Title>
    <VideoStream ObjectRef="vs-1"/>
  </Media>
  <ClipLoggingInfo ObjectID="cli-1">
    <MediaFrameRate>10160640000</MediaFrameRate>
    <MediaInPoint>914457600000000</MediaInPoint>
    <MediaOutPoint>919537920000000</MediaOutPoint>
  </ClipLoggingInfo>
  <MasterClip ObjectUID="mc-1">
    <LoggingInfo ObjectRef="cli-1"/>
    <Clips><Clip ObjectRef="clip-v1"/></Clips>
  </MasterClip>

  <!-- Media 2: 1080p 25fps B-Roll clip -->
  <VideoStream ObjectID="vs-2">
    <FrameRect>0,0,1920,1080</FrameRect>
  </VideoStream>
  <Media ObjectUID="media-2">
    <FilePath>/Media/StudioShoot/B002_C011_Handheld.mov</FilePath>
    <Title>B002_C011_Handheld.mov</Title>
    <VideoStream ObjectRef="vs-2"/>
  </Media>
  <ClipLoggingInfo ObjectID="cli-2">
    <MediaFrameRate>10160640000</MediaFrameRate>
    <MediaInPoint>0</MediaInPoint>
    <MediaOutPoint>3048192000000</MediaOutPoint>
  </ClipLoggingInfo>
  <MasterClip ObjectUID="mc-2">
    <LoggingInfo ObjectRef="cli-2"/>
    <Clips><Clip ObjectRef="clip-v2"/></Clips>
  </MasterClip>

  <!-- Media 3: Standalone WAV dialogue -->
  <Media ObjectUID="media-3">
    <FilePath>/Media/Audio/Dialogue_Lav_01.wav</FilePath>
    <Title>Dialogue_Lav_01.wav</Title>
  </Media>
  <ClipLoggingInfo ObjectID="cli-3">
    <MediaFrameRate>5292000</MediaFrameRate>
    <MediaInPoint>0</MediaInPoint>
    <MediaOutPoint>4064256000000</MediaOutPoint>
  </ClipLoggingInfo>
  <MasterClip ObjectUID="mc-3">
    <LoggingInfo ObjectRef="cli-3"/>
    <Clips><Clip ObjectRef="clip-a1"/></Clips>
  </MasterClip>

  <!-- Clip 1 sources & components (Motion + Lumetri + Ultra Key) -->
  <VideoClipSource ObjectID="src-v1">
    <MediaSource><Media ObjectURef="media-1"/></MediaSource>
  </VideoClipSource>
  <VideoClip ObjectID="clip-v1">
    <asl.clip.label.name>BE.Prefs.LabelColors.8</asl.clip.label.name>
    <Clip>
      <Source ObjectRef="src-v1"/>
      <InPoint>254016000000</InPoint>
      <OutPoint>1524096000000</OutPoint>
      <PlaybackSpeed>1.0</PlaybackSpeed>
    </Clip>
  </VideoClip>
  <SubClip ObjectID="sub-v1">
    <Name>A001_C004_Interview.mov</Name>
    <MasterClip ObjectURef="mc-1"/>
    <Clip ObjectRef="clip-v1"/>
  </SubClip>

  <VideoComponentParam ObjectID="p-mot-scale"><ParameterID>2</ParameterID><Name>Scale</Name><StartKeyframe>0,55.0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="p-mot-rot"><ParameterID>5</ParameterID><Name>Rotation</Name><StartKeyframe>0,-1.5</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="comp-mot-1">
    <MatchName>AE.ADBE Motion</MatchName>
    <InstanceName>Motion</InstanceName>
    <Params><Param ObjectRef="p-mot-scale"/><Param ObjectRef="p-mot-rot"/></Params>
  </VideoFilterComponent>

  <VideoComponentParam ObjectID="p-lum-exp"><ParameterID>11</ParameterID><Name>Exposure</Name><StartKeyframe>0,0.45</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="p-lum-con"><ParameterID>12</ParameterID><Name>Contrast</Name><StartKeyframe>0,18.0</StartKeyframe></VideoComponentParam>
  <VideoComponentParam ObjectID="p-lum-sat"><ParameterID>20</ParameterID><Name>Saturation</Name><StartKeyframe>0,112.0</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="comp-lum-1">
    <MatchName>AE.ADBE Lumetri</MatchName>
    <InstanceName>Lumetri Color</InstanceName>
    <Params><Param ObjectRef="p-lum-exp"/><Param ObjectRef="p-lum-con"/><Param ObjectRef="p-lum-sat"/></Params>
  </VideoFilterComponent>

  <VideoComponentParam ObjectID="p-uk-tol"><ParameterID>1</ParameterID><Name>Tolerance</Name><StartKeyframe>0,15</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="comp-uk-1">
    <MatchName>AE.ADBE Ultra Key</MatchName>
    <InstanceName>Ultra Key</InstanceName>
    <Params><Param ObjectRef="p-uk-tol"/></Params>
  </VideoFilterComponent>

  <VideoComponentChain ObjectID="chain-v1">
    <ComponentChain>
      <Components>
        <Component ObjectRef="comp-mot-1"/>
        <Component ObjectRef="comp-lum-1"/>
        <Component ObjectRef="comp-uk-1"/>
      </Components>
    </ComponentChain>
  </VideoComponentChain>

  <VideoClipTrackItem ObjectID="ti-v1">
    <ClipTrackItem>
      <TrackItem><Start>0</Start><End>1270080000000</End></TrackItem>
      <SubClip ObjectRef="sub-v1"/>
      <ComponentOwner><Components ObjectRef="chain-v1"/></ComponentOwner>
    </ClipTrackItem>
  </VideoClipTrackItem>

  <!-- Clip 2 sources & components (Warp Stabilizer + Gaussian Blur + 3rd party FilmConvert) -->
  <VideoClipSource ObjectID="src-v2">
    <MediaSource><Media ObjectURef="media-2"/></MediaSource>
  </VideoClipSource>
  <VideoClip ObjectID="clip-v2">
    <asl.clip.label.name>BE.Prefs.LabelColors.2</asl.clip.label.name>
    <Clip>
      <Source ObjectRef="src-v2"/>
      <InPoint>0</InPoint>
      <OutPoint>1270080000000</OutPoint>
      <PlaybackSpeed>0.5</PlaybackSpeed>
    </Clip>
  </VideoClip>
  <SubClip ObjectID="sub-v2">
    <Name>B002_C011_Handheld.mov</Name>
    <MasterClip ObjectURef="mc-2"/>
    <Clip ObjectRef="clip-v2"/>
  </SubClip>

  <VideoComponentParam ObjectID="p-ws-smooth"><ParameterID>1</ParameterID><Name>Smoothness</Name><StartKeyframe>0,25</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="comp-ws-2">
    <MatchName>AE.ADBE SubspaceStabilizer</MatchName>
    <InstanceName>Warp Stabilizer</InstanceName>
    <Params><Param ObjectRef="p-ws-smooth"/></Params>
  </VideoFilterComponent>

  <VideoComponentParam ObjectID="p-gb-blur"><ParameterID>1</ParameterID><Name>Blurriness</Name><StartKeyframe>0,12.5</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="comp-gb-2">
    <MatchName>AE.ADBE Gaussian Blur 2</MatchName>
    <InstanceName>Gaussian Blur</InstanceName>
    <Params><Param ObjectRef="p-gb-blur"/></Params>
  </VideoFilterComponent>

  <VideoComponentParam ObjectID="p-fc-grain"><ParameterID>1</ParameterID><Name>Grain</Name><StartKeyframe>0,35</StartKeyframe></VideoComponentParam>
  <VideoFilterComponent ObjectID="comp-fc-2">
    <MatchName>AE.RubberMonkey.FilmConvertNitrate</MatchName>
    <InstanceName>FilmConvert Nitrate</InstanceName>
    <Params><Param ObjectRef="p-fc-grain"/></Params>
  </VideoFilterComponent>

  <VideoComponentChain ObjectID="chain-v2">
    <ComponentChain>
      <Components>
        <Component ObjectRef="comp-ws-2"/>
        <Component ObjectRef="comp-gb-2"/>
        <Component ObjectRef="comp-fc-2"/>
      </Components>
    </ComponentChain>
  </VideoComponentChain>

  <VideoClipTrackItem ObjectID="ti-v2">
    <ClipTrackItem>
      <TrackItem><Start>1270080000000</Start><End>3810240000000</End></TrackItem>
      <SubClip ObjectRef="sub-v2"/>
      <ComponentOwner><Components ObjectRef="chain-v2"/></ComponentOwner>
    </ClipTrackItem>
  </VideoClipTrackItem>

  <!-- Audio Clip 1 (DeNoise) -->
  <AudioClipSource ObjectID="src-a1">
    <MediaSource><Media ObjectURef="media-3"/></MediaSource>
  </AudioClipSource>
  <AudioClip ObjectID="clip-a1">
    <Clip>
      <Source ObjectRef="src-a1"/>
      <InPoint>0</InPoint>
      <OutPoint>3810240000000</OutPoint>
      <PlaybackSpeed>1.0</PlaybackSpeed>
    </Clip>
  </AudioClip>
  <SubClip ObjectID="sub-a1">
    <Name>Dialogue_Lav_01.wav</Name>
    <MasterClip ObjectURef="mc-3"/>
    <Clip ObjectRef="clip-a1"/>
  </SubClip>

  <AudioComponentParam ObjectID="p-dn-amt"><ParameterID>1</ParameterID><Name>Amount</Name><StartKeyframe>0,40</StartKeyframe></AudioComponentParam>
  <AudioFilterComponent ObjectID="comp-dn-1">
    <MatchName>AE.ADBE DeNoise</MatchName>
    <InstanceName>DeNoise</InstanceName>
    <Params><Param ObjectRef="p-dn-amt"/></Params>
  </AudioFilterComponent>

  <AudioComponentChain ObjectID="chain-a1">
    <ComponentChain>
      <Components>
        <Component ObjectRef="comp-dn-1"/>
      </Components>
    </ComponentChain>
  </AudioComponentChain>

  <AudioClipTrackItem ObjectID="ti-a1">
    <ClipTrackItem>
      <TrackItem><Start>0</Start><End>3810240000000</End></TrackItem>
      <SubClip ObjectRef="sub-a1"/>
      <ComponentOwner><Components ObjectRef="chain-a1"/></ComponentOwner>
    </ClipTrackItem>
  </AudioClipTrackItem>

  <!-- Tracks -->
  <VideoClipTrack ObjectUID="vtrack-1">
    <ClipTrack>
      <Track>
        <IsMuted>false</IsMuted>
        <IsLocked>false</IsLocked>
        <Node><Properties><MZ.TrackName>Main Video</MZ.TrackName></Properties></Node>
      </Track>
      <TrackItems>
        <TrackItem ObjectRef="ti-v1"/>
        <TrackItem ObjectRef="ti-v2"/>
      </TrackItems>
    </ClipTrack>
  </VideoClipTrack>

  <AudioClipTrack ObjectUID="atrack-1">
    <ClipTrack>
      <Track>
        <IsMuted>false</IsMuted>
        <IsLocked>false</IsLocked>
        <Node><Properties><MZ.TrackName>Dialogue</MZ.TrackName></Properties></Node>
      </Track>
      <TrackItems>
        <TrackItem ObjectRef="ti-a1"/>
      </TrackItems>
    </ClipTrack>
  </AudioClipTrack>

  <VideoTrackGroup ObjectID="vtg-1">
    <FrameRect>0,0,1920,1080</FrameRect>
    <TrackGroup>
      <FrameRate>10160640000</FrameRate>
      <Tracks><Track ObjectURef="vtrack-1"/></Tracks>
    </TrackGroup>
  </VideoTrackGroup>

  <AudioTrackGroup ObjectID="atg-1">
    <TrackGroup>
      <Tracks><Track ObjectURef="atrack-1"/></Tracks>
    </TrackGroup>
  </AudioTrackGroup>

  <!-- Sequence Markers -->
  <Marker ObjectID="mk-1">
    <DVAMarker>{"DVAMarker":{"mName":"Intro Cut","mComments":"Check skin tone grade","mStartTime":{"ticks":508032000000},"mDuration":{"ticks":254016000000}}}</DVAMarker>
  </Marker>
  <Markers ObjectID="markers-seq-1">
    <Marker><Second ObjectRef="mk-1"/></Marker>
  </Markers>

  <Sequence ObjectUID="seq-main-001">
    <Name>Main_Edit_v03</Name>
    <Node>
      <Properties>
        <MZ.InPoint>0</MZ.InPoint>
        <MZ.OutPoint>3810240000000</MZ.OutPoint>
      </Properties>
    </Node>
    <MarkerOwner><Markers ObjectRef="markers-seq-1"/></MarkerOwner>
    <TrackGroups>
      <TrackGroup><Second ObjectRef="vtg-1"/></TrackGroup>
      <TrackGroup><Second ObjectRef="atg-1"/></TrackGroup>
    </TrackGroups>
  </Sequence>
</PremiereData>
"#
    .to_string()
}

/// Write a gzip-compressed `.prproj` file from the built-in sample XML.
pub fn write_sample_prproj(path: &Path) -> std::io::Result<()> {
    let xml = sample_prproj_xml();
    let file = std::fs::File::create(path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(xml.as_bytes())?;
    encoder.finish()?;
    Ok(())
}
