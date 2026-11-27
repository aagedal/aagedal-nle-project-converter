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

- [x] **Final Cut Pro basic support (`.fcpxml` / `.fcpxmld` & native `.fcpbundle`)** *(Priority #3 & #4)*
  - Added pure-Rust FCPXML 1.8–1.15 / `.fcpxmld` bundle reader (`crates/nle-interchange/src/fcpxml/reader.rs`) and native `.fcpbundle` CoreData SQLite + `NSKeyedArchiver` binary plist reader (`crates/nle-interchange/src/fcpbundle/`), enabling `Final Cut Pro → DaVinci Resolve (.drp)` and `Final Cut Pro → Premiere Pro (.prproj)` / OTIO / FCP7 XML conversion in both CLI and GPUI GUI.
  - Upgraded FCPXML 1.11 writer (`crates/nle-interchange/src/fcpxml/writer.rs`) with per-asset/per-sequence `<format>` resources, accurate `tcStart` & source TC `start`, connected video (`V2+`) and audio lanes with linked-audio deduplication, `<transition>` elements, `<adjust-transform>` / `<adjust-crop>` / `<adjust-blend>`, `<timeMap>` retiming, `<ref-clip>` compound clips, `<mc-clip>` multicam clips, and gap/clip markers.
  - Verified against real Final Cut Pro 11 `.fcpbundle` and `.fcpxmld` (`version="1.15"`) test projects.
  - Follow-up: live-verify `.prproj → .fcpxml` and `.drp → .fcpxml` import inside Final Cut Pro and expand `.fcpbundle` coverage for compound/multicam clips, transitions, retiming, and markers.

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
- [x] **Optional flatten fallback for nested/multicam + FCPXML `<ref-clip>` / `<mc-clip>` writing (`nle-core`, `nle-resolve`, `nle-premiere`, `nle-interchange`, `nle-cli`, `nle-gui`)**
  - Added canonical IR pass `flatten_nested_and_multicam_clips` (`crates/nle-core/src/flatten.rs`) resolving multicam angle cuts, multi-layer compound clips, and recursive nested sequences into source media clips, wired to `ConversionOptions.flatten_nested_and_multicam`, CLI (`--flatten-nested`), GPUI GUI checkbox, and automatic flattening for flat interchange targets (`.otio`, FCP7 `.xml`).
- [ ] **Variable speed ramps & full Clip Attributes IR (`nle-core`, `nle-premiere`, `nle-resolve`)**
  - Implemented `SpeedKeyframe` multi-point curves (`MediaTimemapBA` ↔ FCPXML `<timeMap>`), `RetimeInterpolation` (`TimeInterpolationType` ↔ Resolve Filter 44 `RetimeProcess`), `AlphaMode` / `pixel_aspect_ratio` (`VideoStream`), and `VirtualAudioTrackBA` channel decoding; remaining follow-up: Resolve `DataLevels` / `AlphaMode` media-pool blob roundtrip and Premiere native `TimeRemapping` keyframe curve writing.
- [x] **Stereo vs. mono Fairlight tracks & multi-bus / submix audio routing (`nle-core`, `nle-resolve`, `nle-premiere`)**
  - Preserves stereo vs. mono audio track layouts (`Sm2TiTrack/SubType`, `VirtualAudioTrackBA`, `FLStudioModelBA` Section 0 ↔ Premiere `AudioClipTrack` `ChannelType`) and bidirectional multi-bus routing (Resolve Fairlight FlexBus Section 7 main/sub buses ↔ Premiere `AudioTrackGroup` `MasterTrack`/`SubMixTracks`/`AudioTrackInlet`/`Assign`).
- [x] **Unused Media Pool bin media & clip/custom metadata preservation (`nle-core`, `nle-premiere`, `nle-resolve`)**
  - Added `include_unused_media` option (default `true`) to preserve unplaced project bin media items across Premiere Pro and DaVinci Resolve Media Pools (skipping Premiere `<IsProxy>true</IsProxy>` duplicates).
  - Added bidirectional standard clip metadata (`Description`, `Comments`, `Scene`, `Shot`, `Take`, `Keywords`, `Tape Name`, etc.) and custom/third-party project metadata (`Project.Metadata.Schema` / `ClipProjectItem` properties ↔ Resolve 21.2 `BtMetadataLockableBlob` `ThirdPartyMetadata`), scoped strictly to Premiere ↔ Resolve (omitted from FCPXML/OTIO/FCP7 exports).
- [x] **Native codec metadata on generated Resolve `MediaPoolItem`s (`nle-core`, `nle-resolve`)**
  - Selects the matching Resolve audio scaffold template (`WAV`, `MP3`, `FLAC`) and patches/clears `BtVideoInfo/Clip` field 5, `BtAudioInfo/Clip` field 5, and `BtAudioInfo/TracksBA` (`CodecName`, `NumChannels`, `SampleRate`) using explicit or container-inferred codecs so ProRes, HEVC, MXF, RAW, MP3, FLAC, and WAV clips never inherit placeholder `avc1`/`AAC` metadata.
- [x] **Connected video/audio lanes, transitions, retiming & `<ref-clip>` / `<mc-clip>` in FCPXML writer (`nle-interchange`)**
  - Emits connected `<asset-clip lane="...">` elements for `V2+` and audio tracks, `<transition>` elements, `<timeMap>` retiming, `<adjust-crop>` / `<adjust-blend>`, and `<ref-clip>` / `<mc-clip>` resources in `.fcpxml` output.
- [x] **Native `.prproj` writer (`nle-premiere`) & Resolve marker/color decoding (`nle-resolve`)**
  - Direct `.drp → .prproj` (and IR → `.prproj`) native Premiere Pro 2026 (`Version="45"`) writer in `crates/nle-premiere/src/writer.rs`, wired into CLI (`nle-convert`) and GPUI GUI (`nle-gui`, now default target when opening `.drp` files).
  - Added Resolve 21.2 `Sm2SequenceLockableBlob` / `Sm2TiItemLockableBlob` decoding (`0x80` and `0x81` zstd) for timeline markers, clip markers, clip colors, and freeze-frame hold offsets in `nle-resolve`, plus `MZ.ZeroPoint` & Motion crop reading in `nle-premiere`.
