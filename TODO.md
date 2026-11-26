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

- [x] **Adobe Premiere Pro 2026 live test project** *(Priority #1 & #2)*
  - Automated CEP ExtendScript generator (`scripts/premiere/build_premiere_corpus.py`) producing multi-track V1/V2 + A1-A3, Unicode filenames, Lumetri, Motion keyframes, Opacity, Gaussian Blur, Cross Dissolve, Constant Power, markers, retiming (`50%` Optical Flow, `200%`, `-100%` `<PlayBackwards>`), Interpret Footage (`<IsFrameRateOverridden>` / `<OveriddenFrameRate>` + Start TC + PAR), nested sequence (`Sub_B_Roll`), and 3-angle multicam (`<IsMulticam>` + `<SelectedTrackIndex>`).
  - Schema documented in [docs/PREMIERE_2026_SCHEMA.md](docs/PREMIERE_2026_SCHEMA.md) alongside live Resolve → Premiere FCP7 XML and native `.drp → .prproj` import findings.
  - Verified native `.drp → .prproj` writer live in Premiere Pro 2026 (`Resolve21_Corpus.drp` & `05_Respeed_And_ClipAttributes.drt`: all 7+1 sequences, bins, `MZ.ZeroPoint` 01:00:00:00, sequence markers, `<Link>` groups, `AE.ADBE Motion` / `AE.ADBE Opacity`, Cross Dissolve / Constant Power transitions, nested timelines, compound clips, native 3-angle `<IsMulticam>` cuts, `50%`/`200%`/`-100%`/freeze retiming, and clean Premiere re-save).

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
- [x] **Premiere Pro nested sequence, multicam, reverse & interpret-FPS reading (`nle-premiere`)**
  - Resolved `VideoSequenceSource`/`AudioSequenceSource` inside `parse_clip`, detected `<IsMulticam>` + `<SelectedTrackIndex>` (`Camera N`), `<PlayBackwards>true</PlayBackwards>`, and `<IsFrameRateOverridden>` / `<OveriddenFrameRate>`.
- [x] **Compound clip, nested timeline & multicam writing in Resolve `.drp` (`nle-resolve`)**
  - Emits native `<Sm2MpCompoundClip>` (`SequenceType=1`), `<Sm2MpMulticamClip>` (`SequenceType=2`), and nested `<Sm2MpTimelineClip>` `<MediaRef>` links, plus `<FieldsBlob>` active multicam angle (`f1.f3`) & linked clip UUID groups (`f1.f1.f1`), native Opacity (`Filter 2 param 1`), and reversed `<In>` / `MediaTimemapBA` (`-100%`).
  - Verified live in DaVinci Resolve Studio 21.2 (`Premiere2026_Corpus.prproj → .drp`: 7/7 media online incl. Unicode & interpreted 25fps, native `Multicam`, native nested `Timeline`, `50%`/`200%`/`-100%` speeds, `Opacity=85.0`, `ZoomX=0.45`/`1.1`, linked video+audio groups).
- [ ] **Optional flatten fallback for nested/multicam + FCPXML `<ref-clip>` / `<mc-clip>` writing (`nle-resolve`, `nle-interchange`)**
  - Add a user option to flatten nested/multicam cuts to source clips when targeting formats that don't support them, and emit `<ref-clip>` / `<mc-clip>` in `.fcpxml` writer.
- [ ] **Variable speed ramps & full Clip Attributes IR (`nle-core`, `nle-premiere`, `nle-resolve`)**
  - Capture and model multi-point speed curves, alpha mode, data levels, and source audio channel mapping (`VirtualAudioTracksBA`).
- [ ] **Stereo vs. mono Fairlight track configuration (`nle-resolve` writer)**
  - Preserve Premiere Pro stereo vs. mono track layouts (`AudioClipTrack` `ChannelType`) in Resolve's `FLStudioModelBA` and `VirtualAudioTrackBA` (Premiere `<Link>` clip linking is already translated via `FieldsBlob` `f1.f1.f1`).
- [x] **Unused Media Pool bin media & clip/custom metadata preservation (`nle-core`, `nle-premiere`, `nle-resolve`)**
  - Added `include_unused_media` option (default `true`) to preserve unplaced project bin media items across Premiere Pro and DaVinci Resolve Media Pools (skipping Premiere `<IsProxy>true</IsProxy>` duplicates).
  - Added bidirectional standard clip metadata (`Description`, `Comments`, `Scene`, `Shot`, `Take`, `Keywords`, `Tape Name`, etc.) and custom/third-party project metadata (`Project.Metadata.Schema` / `ClipProjectItem` properties ↔ Resolve 21.2 `BtMetadataLockableBlob` `ThirdPartyMetadata`), scoped strictly to Premiere ↔ Resolve (omitted from FCPXML/OTIO/FCP7 exports).
- [ ] **Native codec metadata on generated Resolve `MediaPoolItem`s**
  - Avoid inheriting placeholder `H.264/AAC` codec metadata strings from the scaffold template when source media is ProRes, HEVC, MXF, or MP3/WAV so Resolve displays accurate clip properties before relink/refresh.
- [ ] **Connected video/audio lanes & transitions in FCPXML writer (`nle-interchange`)**
  - Emit connected `<asset-clip lane="...">` elements for `V2+` and audio tracks plus `<transition>` elements in `.fcpxml` output.
- [x] **Native `.prproj` writer (`nle-premiere`) & Resolve marker/color decoding (`nle-resolve`)**
  - Direct `.drp → .prproj` (and IR → `.prproj`) native Premiere Pro 2026 (`Version="45"`) writer in `crates/nle-premiere/src/writer.rs`, wired into CLI (`nle-convert`) and GPUI GUI (`nle-gui`, now default target when opening `.drp` files).
  - Added Resolve 21.2 `Sm2SequenceLockableBlob` / `Sm2TiItemLockableBlob` decoding (`0x80` and `0x81` zstd) for timeline markers, clip markers, clip colors, and freeze-frame hold offsets in `nle-resolve`, plus `MZ.ZeroPoint` & Motion crop reading in `nle-premiere`.
