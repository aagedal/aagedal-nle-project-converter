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
