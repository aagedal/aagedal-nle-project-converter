# Adobe Premiere Pro 2026 Native `.prproj` Schema Reference

Verified directly against live **Adobe Premiere Pro 2026 (`26.5.3`)** using real-footage projects generated via `scripts/premiere/build_premiere_corpus.py`.

---

## 1. Nested Sequences & Multicam Clips

### Nested Sequences
When a sequence is placed onto a parent timeline, the clip's `<Clip><Source>` points to a `<VideoSequenceSource>` or `<AudioSequenceSource>` instead of a `<VideoMediaSource>` / `<AudioMediaSource>`:

```xml
<VideoClip ObjectID="...">
  <Clip Version="18">
    <Source ObjectRef="136" />
    <InPoint>121927680000</InPoint>
    <OutPoint>1392007680000</OutPoint>
  </Clip>
</VideoClip>

<VideoSequenceSource ObjectID="136">
  <SequenceSource Version="4">
    <Sequence ObjectURef="fd94d6ea-ccd6-4b56-836e-e98fbdae39d4" />
  </SequenceSource>
</VideoSequenceSource>
```

### Multicam Clips
Premiere represents a Multicam Source Sequence as a normal `<Sequence>` where each camera angle sits on its own video/audio track (`V1` = Camera 1, `V2` = Camera 2, `V3` = Camera 3). When enabled as multicam on an edit sequence, `<VideoClip><Clip>` adds:

```xml
<VideoClip ObjectID="...">
  <Clip Version="18">
    <Source ObjectRef="141" />
    <SelectedTrackIndex>0</SelectedTrackIndex>
    <InPoint>0</InPoint>
    <OutPoint>1270080000000</OutPoint>
    <IsMulticam>true</IsMulticam>
  </Clip>
</VideoClip>
```

- `<IsMulticam>true</IsMulticam>` marks the clip (and its target `<Sequence>`) as multicam.
- `<SelectedTrackIndex>` is the **0-based** track index of the active camera angle (`0` → `Camera 1`, `1` → `Camera 2`, `2` → `Camera 3`).
- Linked `<AudioClip>` instances reference `<AudioSequenceSource>` pointing to the same target `<Sequence ObjectURef="...">`.

---

## 2. Respeed / Retiming & Time Interpolation

- **Constant forward speed**: `<PlaybackSpeed>0.5</PlaybackSpeed>` (50%) or `<PlaybackSpeed>2</PlaybackSpeed>` (200%) inside `<Clip>`. Omitted at 100%.
- **Reverse playback**: stored as `<PlayBackwards>true</PlayBackwards>` inside `<Clip>` — `<PlaybackSpeed>` stays positive (or omitted for `-100%`).
- **Time Interpolation**: `<TimeInterpolationType>` directly under `<VideoClip>`:
  - `0` = Frame Sampling
  - `1` = Frame Blending
  - `2` = Optical Flow

---

## 3. Interpret Footage / Clip Attributes (`<VideoStream>`)

When Interpret Footage overrides are set on a media item (`setOverrideFrameRate`, `setOverridePixelAspectRatio`, `setStartTime`):

```xml
<VideoStream ObjectID="293">
  <IsFrameRateOverridden>true</IsFrameRateOverridden>
  <OveriddenFrameRate>10160640000</OveriddenFrameRate>
  <IsPAROverridden>true</IsPAROverridden>
  <FrameRate>5080320000</FrameRate>
  <FrameRect>0,0,1920,1080</FrameRect>
  <AlphaType>3</AlphaType>
  <IgnoreAlpha>true</IgnoreAlpha>
</VideoStream>
```

> **Schema quirk:** Premiere Pro spells the overridden frame rate element `<OveriddenFrameRate>` (single `r`, double `d`), with values in Adobe ticks (`254,016,000,000 / fps`).

---

## 4. Resolve → Premiere via FCP7 XML Observations

Live import of Resolve 21.2's exported FCP7 `.xml` corpus into Premiere Pro 2026 (`ResolveToPremiere_ImportTest.prproj`) revealed:
- Basic multi-track clips, reverse (`<PlayBackwards>true</PlayBackwards>`), and constant speeds (`0.5`, `2.0`) import cleanly.
- **Sequence markers were dropped** (`0` markers imported across all 5 timelines).
- **Multicam metadata was lost** (`0` `<IsMulticam>` clips; Resolve's FCP7 XML flattened/detached multicam).
- **Freeze frames** imported as an extreme slow-down (`<PlaybackSpeed>0.00525124</PlaybackSpeed>`).

This confirms why a direct native `.drp → .prproj` writer will significantly outperform FCP7 XML interchange.

---

## 5. Native `.prproj` Writer Schema & Live Premiere Pro 2026 Verification

Implemented in `crates/nle-premiere/src/writer.rs` (`PremiereData Version="3"`, `Project Version="45"`):

- **Project scaffold & automatic media relinking**: Emitting `<Project>`, `<RootProjectItem>`, `<BinProjectItem>`, `<ProjectSettings>`, `<CompileSettings>`, `<ScratchDiskSettings>`, `<IngestSettings>`, `<WorkspaceSettings>`, `<DummyCaptureSettings>`, and `<DefaultSequenceSettings>` with valid Premiere 2026 `ClassID`s and omitting `<ImplementationID>` on `<Media>` allows Premiere Pro 2026 to auto-link CFR media silently on open without popping a modal Locate Media dialog. (Note: VFR `.mp4` assets whose exact average frame ticks differ from rounded CFR values, e.g. `49.894fps` vs `50.0fps`, may appear offline until relinked in Premiere.)
- **Timeline start timecode**: Stored under `<Sequence><Node><Properties><MZ.ZeroPoint>` in Adobe ticks (e.g. `914457600000000` for `01:00:00:00`).
- **Markers**: Native `<Markers>` + `<Marker>` with `<DVAMarker>` JSON (`mName`, `mComment`, `mStartTime.ticks`, `mDuration.ticks`, `mCuePointList` color) — all Resolve timeline markers survive directly into Premiere Pro 2026.
- **Native Multicam, Compound & Nested Sequences**: Every sequence gets a `<VideoSequenceSource>`/`<AudioSequenceSource>` + `<MasterClip>` + `<ClipProjectItem>`. Resolve compound clips and nested timelines become native Premiere nested sequences; Resolve multicam clips emit `<IsMulticam>true</IsMulticam>` + 0-based `<SelectedTrackIndex>` and round-trip cleanly through Premiere save.
- **Transitions & Links**: `<VideoTransitionTrackItem>` (`AE.ADBE Cross Dissolve` etc., without an unnecessary empty `<VideoFilterComponent>`) and `<AudioTransitionTrackItem>` (`Constant Power` / `Constant Gain` / `Exponential Fade`) wired via `<HeadTransition>` / `<TailTransition>`, plus `<PersistentGroupContainer><LinkContainer><Links>` for linked video+audio clip groups.
- **Motion, Opacity & Lumetri**: Native `<VideoComponentChain>` emitting `AE.ADBE Motion` (Position, Scale, Scale Width, Uniform Scale, Rotation, Anchor Point, Crop Left/Top/Right/Bottom), `AE.ADBE Opacity`, and `AE.ADBE Lumetri` — all verified intact after live Premiere Pro 2026 open + re-save on `Resolve21_Corpus.drp` (7 sequences, 18V/44A clips, 5 markers, 16 link groups, 3 multicam angle cuts) and `05_Respeed_And_ClipAttributes.drt` (4/4 media online, `50%`/`200%`/`-100%` reverse/freeze).
- **Project & Custom Clip Metadata (`Project.Metadata.Schema` / `ClipProjectItem`)**:
  - Custom project metadata schemas (such as MAM/third-party plugin columns) are declared in `<Project><Node><Properties><Project.Metadata.Schema>` as an XML-escaped `<xmp_definitions>` document under the `http://ns.adobe.com/premierePrivateProjectMetaData/1.0/` namespace (`<xmp_property category="external" name="..." type="text" label="..." />`).
  - Per-clip custom values live directly as child elements `<{name}>{value}</{name}>` inside `<ClipProjectItem><ProjectItem><Node><Properties>` (and bin custom properties inside `<BinProjectItem><ProjectItem><Node><Properties>`).
  - Standard clip metadata lives in `Column.PropertyText.*` (`Description`, `Comment`, `LogNote`, `Scene`, `Shot`, `Take`, `Keywords`, `Client`, `TapeName`, `CameraAngle`, `CameraLabel`) and `<MasterClip><LoggingInfo>` (`<ClipLoggingInfo>`).
  - Proxy `<Media>` elements carry `<IsProxy>true</IsProxy>` and are skipped when collecting project-level media assets so proxy files are not duplicated as standalone clips.
  - Custom/third-party metadata transfers directly between Premiere Pro and DaVinci Resolve (`BtMetadataLockableBlob` `ThirdPartyMetadata`), and is intentionally omitted when exporting to other NLE interchange formats (`.fcpxml`, `.otio`, FCP7 `.xml`).
- **Audio Submixes / Bus Routing & Stereo/Mono Tracks (`AudioTrackGroup`)**:
  - `<AudioTrackGroup>` holds `<MasterTrack ObjectRef="..."/>` (`<AudioMixTrack>` with `<SubType>3</SubType>`, `<Track><ID>1</ID>`) and optional `<SubMixTracks><SubMixTrack ObjectRef="..."/></SubMixTracks>` (`<AudioMixTrack>` with `<SubType>2</SubType>`, `<MZ.TrackName>`, `<Track><ID>`).
  - Each `<AudioClipTrack>` (and child `<AudioMixTrack>`) routes to a submix via `<AudioTrack><Assign>{submix_track_id}</Assign>` (omitted when routed to Master) and is registered as a `<Source>` inside the target `<AudioMixTrack>`'s `<AudioTrackInlet>`.
  - Track channel format uses `<AudioTrack><ChannelType>` (`0` = Mono, `1` / omitted = Stereo, `2` = 5.1, `3` = Adaptive), translating directly to/from DaVinci Resolve Fairlight FlexBus buses and stereo/mono tracks.
