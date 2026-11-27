# DaVinci Resolve 21.2 Native `.drp` / `.drt` Schema Reference

Verified directly against live **DaVinci Resolve Studio 21.2.0.6** (`DbAppVer="21.2.0.0006"`, `DbPrjVer="17"`) using real footage exports generated via `scripts/resolve/`.

---

## 1. Binary Envelope (`0x80` vs. `0x81` zstd)

Resolve wraps protobuf payloads in hex-encoded fields (`FieldsBlob`, `EffectFiltersBA`, `BtVideoInfo/Clip`, `SetupBA`, etc.) with a 9-byte header:

```text
[0..4]  u32be  wrapper version (2)
[4..8]  u32be  payload byte count (including flag byte)
[8]     u8     compression flag:
                 0x80 = raw uncompressed protobuf payload at [9..]
                 0x81 = Zstandard frame (magic 28 b5 2f fd ...) at [9..]
[9..]   bytes  payload
```

Implemented in `crates/nle-resolve/src/binary.rs` (`decode_wrapper_hex`, pure-Rust `ruzstd`).

---

## 2. Compound Clips, Nested Timelines & Multicam Clips

### Media Pool containers (`MediaPool/**/MpFolder.xml`)

| Container type | XML tag | Inner timeline |
|---|---|---|
| Regular / Nested Timeline | `<Sm2MpTimelineClip DbId="...">` | `<Sequence><Sm2Sequence DbId="..."/></Sequence>` → `SeqContainer/<uuid>.xml` |
| Compound Clip | `<Sm2MpCompoundClip DbId="...">` | `<Sequence><Sm2Sequence DbId="..."/></Sequence>` → `SeqContainer/<uuid>.xml` |
| Multicam Clip | `<Sm2MpMulticamClip DbId="...">` | `<Sequence><Sm2Sequence DbId="..."/></Sequence>` → `SeqContainer/<uuid>.xml` (1 video + 1 audio track per angle) |

### Timeline references (`SeqContainer/<uuid>.xml`)

On the parent timeline, compound clips, nested timelines, and multicam cuts are stored as ordinary `<Sm2TiVideoClip>` / `<Sm2TiAudioClip>` items with:
- `<MediaRef>` pointing to the `DbId` of the `<Sm2MpCompoundClip>`, `<Sm2MpTimelineClip>`, or `<Sm2MpMulticamClip>` (and `<MediaFilePath/>` left empty).
- `<In>` giving the source offset inside the referenced container.

### Multicam active angle & linked clip groups (`Sm2Ti*Clip/FieldsBlob`)

Inside the clip's `0x80` / `0x81` `FieldsBlob` protobuf:

```text
f1 (length-delimited message):
  f1.f1 (length-delimited message):
    f1.f1.f1 (bytes): version-1 keyed-dict ("0".."N" -> UTF-16BE linked sibling item DbId)
  f1.f3 (bytes, UTF-8): active multicam camera/angle name (e.g. "Camera 1", "Camera 3")
  f1.f4 (varint):       multicam active flag (1)
```

> **Note:** On Resolve 21.2 timelines, `<LinkedItemSync/>` is empty and the actual linked video/audio sibling group lives in `FieldsBlob` `f1.f1.f1`.

### FCPXML 1.10 export comparison (from Resolve 21.2)

- **Compound Clips** export as `<media id="rN"><sequence>...</sequence></media>` + `<ref-clip ref="rN"/>`.
- **Multicam Clips** export as `<media id="rN"><multicam><mc-angle angleID="..." name="...">...</mc-angle></multicam></media>` + `<mc-clip ref="rN"><mc-source angleID="..." srcEnable="all"/></mc-clip>`.
- **Nested Timelines** are **dropped to `<gap>`** by Resolve's own FCPXML exporter — direct `.drp` reading preserves the full `<MediaRef>` link to the nested `<Sm2MpTimelineClip>`.

---

## 3. Respeed / Retiming (`MediaTimemapBA`)

`MediaTimemapBA` has two representations:

1. **Identity (100% forward)**: compact binary starting with byte `0x02`, followed by 1 (audio) or 5 (video) big-endian `f64` duration/extent values.
2. **Retimed (`Sm2TimeMap`)**: version-1 keyed-dict (`decode_keyed_fields`) containing `YMin`, `YMax`, `XMax`, `UniqueId`, optional `LastValidYOffset`, `KeyframesBA`, and `DbType = "Sm2TimeMap"`.

`KeyframesBA` starts with a 1-byte flags prefix `0x80` followed by repeated protobuf field `1` (`Keyframe`) messages where sub-field `1` (`fixed64`, `f64le`) is record-time `x` (seconds, omitted when `0.0`) and sub-field `2` (`fixed64`, `f64le`) is source-time `y` (seconds, omitted when `0.0`):

| Mode | Keyframe 0 `(x0, y0)` | Keyframe 1 `(x1, y1)` | Wire pattern (`KeyframesBA`) |
|---|---|---|---|
| **Forward constant (`50%`, `200%`)** | `(0, 0)` | `(end_sec / speed, end_sec)` | `80 0a 00 0a 12 09 <x1> 11 <y1>` |
| **Reverse constant (`-100%`)** | `(0, end_sec)` | `(end_sec / \|speed\|, 0)` | `80 0a 09 11 <y0> 0a 09 09 <x1>` |
| **Freeze frame (`0%`)** | `(0, hold_sec)` | `(60000.0, hold_sec)` | `80 0a 09 11 <y> 0a 12 09 <60000.0> 11 <y>` |
| **Still image hold** | `(0, 0)` | `(60000.0, 0)` | `80 0a 00 0a 09 09 <60000.0>` |

Implemented for both encoding and decoding in `crates/nle-resolve/src/timemap.rs`.

---

## 4. Clip Inspector Filters (`EffectFiltersBA`)

Unwrapping `EffectFiltersBA` (`0x80` or `0x81` zstd) yields repeated field `1` filter messages:
- Filter `f1` (varint) = Filter ID
- Filter `f9` (repeated bytes) = Parameter entries (`p.f1` = Param ID, `p.f3` = Curve message with repeated `c.f1` keyframes containing `k.f2` `f64le` or `k.f4` enum varint or `k.f5` bool varint).

Verified Filter & Parameter IDs from Resolve 21.2:

| Filter ID | Section | Param ID | Meaning | Value encoding |
|---:|---|---:|---|---|
| `2` | Composite | `0` | `CompositeMode` | varint enum (`5` = Screen) |
| `2` | Composite | `1` | `Opacity` | `f64le` (`0.0..=100.0`) |
| `4` | Transform | `40` | `Pan` | `f64le` (normalized by timeline width) |
| `4` | Transform | `41` | `Tilt` | `f64le` (normalized by timeline height) |
| `4` | Transform | `42` | `ZoomX` | `f64le` (1.0 = 100%) |
| `4` | Transform | `43` | `ZoomY` | `f64le` (1.0 = 100%) |
| `4` | Transform | `47` | `RotationAngle` | `f64le` (degrees, Resolve sign) |
| `4` | Transform | `50` | `FlipX` | bool varint (`1`) |
| `4` | Transform | `51` | `FlipY` | bool varint (`1`) |
| `6` | Cropping | `54..=57` | `CropLeft`, `CropRight`, `CropTop`, `CropBottom` | `f64le` (normalized by width/height) |
| `44` | Retime & Scaling | `91` | `RetimeProcess` | varint enum (`2` = Frame Blend, `3` = Optical Flow) |
| `44` | Retime & Scaling | `92` | `Scaling` | varint enum (`2` = Fit, `3` = Fill) |
| `44` | Retime & Scaling | `147` | `MotionEstimation` | varint enum (`5` = Speed Warp Better) |
| `124` | Audio Volume | `95` | `AudioVolume` | `f64le` (dB) |
| `144` | Audio Pan | `96` | `AudioPan` | `f64le` (`-100..=100`) |
| `134` | Audio Pitch | `140` | `AudioPitchSemiTones` | `f64le` (semitones) |

---

## 5. Media Pool Clip Attributes & Metadata (`Sm2MpVideoClip` / `BtMetadataLockableBlob`)

- **Interpret Footage `FPS` & `Start TC` override**: persisted directly in `BtVideoInfo/Time` keyed-dict (`FrameRate`, `StartFrame`, `Timecode`, `NumFrames`), and timeline clips using that media item automatically receive updated `MediaTimemapBA` extents.
- **Audio Channel Mapping** (stereo/mono routing, muted channels): persisted in `<VirtualAudioTracksBA>` (`ChannelsBA`, `AudioType`).
- **Alpha Mode (`Straight` / `Premultiplied` / `None`) & Data Level (`Auto` / `Video` / `Full`)**: stored directly inside `<BtVideoInfo><Radiometry>` (1-byte flag prefix `0x80` raw / `0x81` zstd + protobuf), verified live on DaVinci Resolve Studio 21.2.0.6:
  - Field `6` (`varint`): `Alpha mode` — `1` = `Premultiplied`, `3` = `Straight`, omitted (or `0`) = `None`.
  - Field `13` (`varint`): `Data Level` — `1` = `Video`, `2` = `Full`, omitted (or `0`) = `Auto`.
  - Implemented bidirectionally in `crates/nle-resolve/src/reader.rs` (`decode_bt_radiometry`) and `crates/nle-resolve/src/writer.rs` (`patch_bt_radiometry`).
- **Standard & Third-Party Clip Metadata (`BtMetadataLockableBlob`)**: stored in `project.xml` `<LocableBlobSet>` with `<BlobOwner>` set to the clip's `<BtVideoInfo>` `DbId` (or `<BtAudioInfo>` `DbId` for audio-only items). Its `FieldsBlob` is a version-1 keyed-dict (`BlobData`) wrapping a `10001`-version `0x80`/`0x81` protobuf whose field `2` begins with an 8-byte header `[1_000_001: u32be][inner_len: u32be]` followed by an inner protobuf:
  - Repeated field `2`: standard metadata entries (`f1` = varint enum ID, `f2` = UTF-8 value) — verified IDs include `64` (`Description`), `65` (`Comments`), `66` (`Scene`), `67` (`Shot`), `68` (`Take`), `69` (`Angle`), `74` (`Keywords`), `78` (`Shoot Day`), `80` (`Camera #`), `88` (`Location`), `96` (`Camera Manufacturer`), `97` (`Camera Type`), `100` (`Camera Notes`), `107` (`ISO`), `115` (`LUT Used`), `116` (`Lens Type`), `211` (`Audio Notes`), `304` (`Production Name`), and `336` (`Director`).
  - Repeated field `4`: arbitrary third-party / MAM plugin metadata entries (`f1` = UTF-8 key, `f2` = UTF-8 value, surfaced in Resolve's `GetThirdPartyMetadata` / `SetThirdPartyMetadata` API; `Column.Intrinsic.TapeName` maps bidirectionally to Premiere `<ClipLoggingInfo><TapeName>`). Implemented in `crates/nle-resolve/src/state.rs`.

## 6. Fairlight FlexBus Mixer & Stereo/Mono Audio Tracks (`FLStudioModelBA`)

Stored inside `<Sm2Sequence><FieldsBlob>` under key `"FLStudioModelBA"` as Qt `qCompress` (`[u32be uncompressed_len][zlib]`). Its final `66 88 66 77` (`LastClient`) chunk contains columnar tables framed as `[u32le section_id]` followed by `(version=1, field_id, count, values)` records terminated by `0`:
- **Section `0` (Track channels)**: `0x0B4`/`0x0B8`/`0x0B9`/`0x0BA` encode stereo vs. mono channel width (`2` stereo / `1` mono, matching `<Sm2TiTrack><SubType>` `0` stereo / `1` mono and `VirtualAudioTrackBA` `AudioType` `0` / `1`), `0x0BC` track names, `0x400`/`0x40B` assigned target bus index, and `0x458` timeline audio track index (`0xFFFF_FFFF` on stereo secondary channels).
- **Section `7` (FlexBus buses)**: `0x0AA` main bus flag, `0x0BC` bus names, `0x10A`/`0x145`/`0x1A5` main vs. submix bus flags, `0x400`/`0x40B` parent bus routing index (`0xFFFF_FFFF` for main output bus), and `0x458` strip ID (`track_count + 1 + bus_idx`).
- **Post-bus tail**: Section `9` strip ID `0x458 = track_count`, `0x324 = track_count + bus_count`, `0x32F = track_count`. Implemented bidirectionally in `crates/nle-resolve/src/audio.rs`.

---

## 7. Color Page Primaries (`Corrector 1`) & Inspector Stabilization (`pLmVerTable` / DRX `Body`)

Resolve stores both Color Page serial node grades (`f1`) and Inspector Stabilization (`f3`) inside the clip's `<pLmVerTable><ListMgt::LmVersion><Body>` (`0x80` raw / `0x81` zstd DRX protobuf):

### Primaries – Color Wheels (`f1.f7[*].f9.f1`, Corrector `1`)

Each serial node `f7` carries Primary Corrector (`f1 = 1`) parameters under `f6.f2.f3` (`f1` = varint param ID, `f2.f1` = `f32le` value):

| Parameter ID (hex) | Decimal | Resolve UI Control | Wire Encoding (DRX ↔ UI) |
|---|---:|---|---|
| `0x0600_0004` | `100663300` | Hue | `(ui - 50.0) / 50.0` (neutral `0.0`) |
| `0x0600_0005` | `100663301` | Saturation | `ui / 50.0` (neutral `1.0`) |
| `0x0600_0018..1B` | `100663320..323` | Lift `R, G, B, Y` | `ui * 2.0` (neutral `0.0`) |
| `0x0600_001D..20` | `100663325..328` | Gain `R, G, B, Y` | `ui * 1.0` (neutral `1.0`) |
| `0x0600_0022..25` | `100663330..333` | Gamma `R, G, B, Y` | `ui * 4.0` (neutral `0.0`) |
| `0x0600_007D..7F` | `100663421..423` | Offset `R, G, B` | `(ui - 25.0) / 25.0` (neutral `0.0`) |
| `0x8600_000B` | `2248146955` | Lum Mix | `ui / 100.0` (neutral `1.0`) |
| `0x8600_00C1` | `2248147137` | Contrast | `ui * 1.0` (neutral `1.0`) |
| `0x8600_0110` | `2248147216` | Highlights | `ui * 1.0` (neutral `0.0`) |
| `0x8600_0111` | `2248147217` | Shadows | `ui * 1.0` (neutral `0.0`) |
| `0x8600_0112` | `2248147218` | Color Boost | `ui * 1.0` (neutral `0.0`) |
| `0x8600_0113` | `2248147219` | Mid/Detail | `ui * 1.0` (neutral `0.0`) |
| `0x8600_0115` | `2248147221` | Temp | `ui * 1.0` (neutral `0.0`) |
| `0x8600_0116` | `2248147222` | Tint | `ui * 1.0` (neutral `0.0`) |

### Inspector Stabilization (`f3`, Corrector `10`)

Stored under top-level node **`f3`** (`f8 = 84`, corrector `f9.f1.f1 = 10`):
- **Static settings (`f9.f1.f6.f2.f3`)**: `0x1030_0016` (`271581206`, mode/enabled varint `f2 = 8` for Perspective) and `0x1030_001D` (`271581213`, options varint `f4 = 0`).
- **Analyzed per-frame track (`f9.f1.f6.f2.f4`)**: repeated `f1` entries per frame (`f1 = frame * 2`) with 80-byte tracker sample data (`0x1030_0015`) and two 3×3 `f32` homography matrices (`0x1030_0017`, `0x1030_001A`).
- Implemented bidirectionally in `crates/nle-resolve/src/grade.rs` (`build_grade_and_stabilization_body`, `parse_resolve_grade_nodes`, `has_resolve_stabilization`).

---

## 6. Regenerating the Reference Corpus

With DaVinci Resolve Studio running (or installed at `/Applications/DaVinci Resolve`):

```bash
python3 scripts/resolve/build_resolve_corpus.py --media-dir /path/to/TestVideo --output-dir /tmp/resolve-corpus
python3 scripts/resolve/build_resolve_respeed_attrs.py --media-dir /path/to/TestVideo --output-dir /tmp/resolve-corpus
```

> **API tip:** Call `projectManager.SaveProject()` before `projectManager.ExportProject(...)` when exporting a single-timeline project without switching timelines so Resolve flushes the active `SeqContainer/*.xml` into the `.drp`.
