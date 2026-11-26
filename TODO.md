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
  - Schema documented in [docs/PREMIERE_2026_SCHEMA.md](docs/PREMIERE_2026_SCHEMA.md) alongside live Resolve → Premiere FCP7 XML import findings.
  - Remaining: test native `.drp → .prproj` writer re-import back into Premiere Pro 2026 once implemented.

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
- [ ] **Optional unused Media Pool bin media preservation**
  - Add a conversion option to include unplaced project bin media items in the target Resolve Media Pool (currently only timeline-referenced media is written).
- [ ] **Native codec metadata on generated Resolve `MediaPoolItem`s**
  - Avoid inheriting placeholder `H.264/AAC` codec metadata strings from the scaffold template when source media is ProRes, HEVC, MXF, or MP3/WAV so Resolve displays accurate clip properties before relink/refresh.
- [ ] **Connected video/audio lanes & transitions in FCPXML writer (`nle-interchange`)**
  - Emit connected `<asset-clip lane="...">` elements for `V2+` and audio tracks plus `<transition>` elements in `.fcpxml` output.
- [ ] **Native `.prproj` writer (`nle-premiere`)**
  - Enable direct `.drp → .prproj` and `.fcpxml → .prproj` conversion without intermediate FCP7 XML.
