# TODO & Roadmap

Follow-up items and live-NLE test coverage tracked from real-world project testing (Premiere Pro 2026, DaVinci Resolve Studio 21.2, and Final Cut Pro).

## Live NLE Round-Trip Test Matrix

- [ ] **Adobe Premiere Pro 2026 live test project**
  - Build a multi-track reference `.prproj` using real footage (multi-codec video, multi-channel audio, embedded timecode, Unicode filenames, retimed clips, Lumetri grades, Motion keyframes, transitions, markers, Essential Sound, and nested bins).
  - Verify direct `.prproj → .drp` conversion opens cleanly in DaVinci Resolve with all media online and frame-accurate in/out points.
  - Test `.drp → FCP7 XML (.xml)` and `.drp → .prproj` (future native writer) re-import back into Premiere Pro 2026.

- [ ] **Final Cut Pro live test project**
  - Export real timelines from Final Cut Pro (`.fcpxml` / `.fcpxmld`) with connected clips, secondary storylines, compound clips, titles, transitions, retiming, and markers.
  - Verify `.prproj → .fcpxml` and `.drp → .fcpxml` imports cleanly into Final Cut Pro with connected video lanes (`V2+`), audio lanes, transitions, and accurate `tcStart`.
  - Add native `.fcpxml` reader support to enable full `Final Cut Pro → DaVinci Resolve (.drp)` and `Final Cut Pro → OTIO / FCP7 XML` conversion.

## Format & Fidelity Improvements

- [ ] **Resolve 21.2 `0x81` zstd blob decoding (`nle-resolve`)**
  - Decode zstd-compressed `EffectFiltersBA` (clip transforms, audio volume/fade curves) and `BtVideoInfo/Clip` blobs when reading `.drp` files exported from DaVinci Resolve 21.2+.
- [ ] **Stereo vs. mono Fairlight track configuration & clip linking**
  - Preserve Premiere Pro stereo vs. mono track layouts (`AudioClipTrack` `ChannelType`) in Resolve's `FLStudioModelBA` and `VirtualAudioTrackBA`.
  - Translate Premiere `<Link>` groups to Resolve `LinkedItemSync` so linked video/audio clips stay grouped on the timeline.
- [ ] **Optional unused Media Pool bin media preservation**
  - Add a conversion option to include unplaced project bin media items in the target Resolve Media Pool (currently only timeline-referenced media is written).
- [ ] **Native codec metadata on generated Resolve `MediaPoolItem`s**
  - Avoid inheriting placeholder `H.264/AAC` codec metadata strings from the scaffold template when source media is ProRes, HEVC, MXF, or MP3/WAV so Resolve displays accurate clip properties before relink/refresh.
- [ ] **Connected video/audio lanes & transitions in FCPXML writer (`nle-interchange`)**
  - Emit connected `<asset-clip lane="...">` elements for `V2+` and audio tracks plus `<transition>` elements in `.fcpxml` output.
- [ ] **Native `.prproj` writer (`nle-premiere`)**
  - Enable direct `.drp → .prproj` conversion without intermediate FCP7 XML.
