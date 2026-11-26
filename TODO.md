# TODO & Roadmap

Follow-up items and live-NLE test coverage tracked from real-world project testing (Premiere Pro 2026, DaVinci Resolve Studio 21.2, and Final Cut Pro).

## Conversion Priority Order

1. **Premiere Pro → DaVinci Resolve** (`.prproj → .drp`)
2. **DaVinci Resolve → Premiere Pro** (`.drp → .prproj` / FCP7 XML)
3. **Final Cut Pro → DaVinci Resolve** (`.fcpxml`/`.fcpxmld` → `.drp`)
4. **Final Cut Pro → Premiere Pro** (`.fcpxml`/`.fcpxmld` → `.prproj` / FCP7 XML)

---

## Live NLE Reference Corpus & Round-Trip Test Matrix

- [x] **DaVinci Resolve Studio 21.2 automated reference corpus** (`scripts/resolve/`)
  - Automated `DaVinciResolveScript` generators covering multi-track V1/V2 + stereo/mono audio, transitions, transforms, fades, clip colors, markers, compound clips (`Sm2MpCompoundClip`), nested timelines (`Sm2MpTimelineClip`), 3-angle multicam with live SmartSwitch cuts (`Sm2MpMulticamClip`), respeed (`50%`, `200%`, `-100%` reverse, `0%` freeze, Optical Flow / Frame Blend / Pitch Correction), and Clip Attributes (`FPS` override, `Start TC`, `Alpha mode`, `Data Level`, `SetAudioMapping`).
  - Schema documented in [docs/RESOLVE_21_SCHEMA.md](docs/RESOLVE_21_SCHEMA.md).

- [ ] **Adobe Premiere Pro 2026 live test project** *(Priority #1 & #2)*
  - Build a multi-track reference `.prproj` using real footage (multi-codec video, multi-channel audio, embedded timecode, Unicode filenames, retimed clips including reverse/speed-ramp, interpret footage / clip attributes, nested sequences, multicam source sequence with angle cuts, Lumetri grades, Motion keyframes, transitions, markers, Essential Sound, and nested bins).
  - Verify direct `.prproj → .drp` conversion opens cleanly in DaVinci Resolve with all media online and frame-accurate in/out points.
  - Test `.drp → FCP7 XML (.xml)` and `.drp → .prproj` (native writer) re-import back into Premiere Pro 2026.

- [ ] **Final Cut Pro live test project** *(Priority #3 & #4)*
  - Export real timelines from Final Cut Pro (`.fcpxml` / `.fcpxmld`) with connected clips, secondary storylines, compound clips, multicam clips, titles, transitions, retiming, and markers.
  - Add native `.fcpxml` / `.fcpxmld` reader support to enable full `Final Cut Pro → DaVinci Resolve (.drp)` and `Final Cut Pro → Premiere / OTIO / FCP7 XML` conversion.
  - Verify `.prproj → .fcpxml` and `.drp → .fcpxml` import cleanly into Final Cut Pro with connected video lanes (`V2+`), audio lanes, transitions, and accurate `tcStart`.

---

## Format & Fidelity Improvements

- [x] **Resolve 21.2 `0x81` zstd blob decoding (`nle-resolve`)**
  - Pure-Rust `ruzstd` decoding in `decode_wrapper_hex` for `FieldsBlob`, `EffectFiltersBA`, and `BtVideoInfo/Clip`.
- [x] **Resolve 21.2 respeed, compound/nested/multicam, and `EffectFiltersBA` reading (`nle-resolve`, `nle-core`)**
  - Added `SequenceKind` (`Timeline`, `CompoundClip`, `MulticamClip`), `ClipSourceKind`, and `Clip.active_angle` to `nle-core`.
  - Decoded `Sm2MpCompoundClip`, `Sm2MpMulticamClip`, nested `Sm2MpTimelineClip` references, `FieldsBlob` multicam active angle + linked clip groups, `EffectFiltersBA` transforms/crop/opacity, and all 4 `MediaTimemapBA` speed modes (forward, reverse `-100%`, freeze `0%`, identity) in `nle-resolve` (plus reverse/freeze encoding in `timemap.rs`).
- [ ] **Premiere Pro nested sequence & multicam reading (`nle-premiere`)**
  - Resolve `SequenceSource` inside `parse_clip`, detect `IsMultiCam` / active camera angle, and map into `SequenceKind` / `ClipSourceKind` / `active_angle`.
- [ ] **Compound clip, nested timeline & multicam writing + optional flatten fallback (`nle-resolve`, `nle-interchange`)**
  - Emit native `Sm2MpCompoundClip` / `Sm2MpMulticamClip` / nested `Sm2MpTimelineClip` in `.drp` writer and `<ref-clip>` / `<mc-clip>` in `.fcpxml` writer, plus a user option to flatten nested/multicam cuts to source clips when targeting formats that don't support them.
- [ ] **Variable speed ramps & full Clip Attributes IR (`nle-core`, `nle-premiere`, `nle-resolve`)**
  - Capture and model multi-point speed curves, interpret-footage FPS/TC overrides, alpha mode, data levels, and source audio channel mapping (`VirtualAudioTracksBA`).
- [ ] **Stereo vs. mono Fairlight track configuration & clip linking (`nle-resolve` writer)**
  - Preserve Premiere Pro stereo vs. mono track layouts (`AudioClipTrack` `ChannelType`) in Resolve's `FLStudioModelBA` and `VirtualAudioTrackBA`.
  - Write Premiere `<Link>` groups into Resolve clip `FieldsBlob` (`f1.f1.f1` linked DbId keyed-dict) so linked video/audio clips stay grouped on the timeline.
- [ ] **Optional unused Media Pool bin media preservation**
  - Add a conversion option to include unplaced project bin media items in the target Resolve Media Pool (currently only timeline-referenced media is written).
- [ ] **Native codec metadata on generated Resolve `MediaPoolItem`s**
  - Avoid inheriting placeholder `H.264/AAC` codec metadata strings from the scaffold template when source media is ProRes, HEVC, MXF, or MP3/WAV so Resolve displays accurate clip properties before relink/refresh.
- [ ] **Connected video/audio lanes & transitions in FCPXML writer (`nle-interchange`)**
  - Emit connected `<asset-clip lane="...">` elements for `V2+` and audio tracks plus `<transition>` elements in `.fcpxml` output.
- [ ] **Native `.prproj` writer (`nle-premiere`)**
  - Enable direct `.drp → .prproj` and `.fcpxml → .prproj` conversion without intermediate FCP7 XML.
