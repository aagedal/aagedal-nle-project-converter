<img src="assets/icon.svg" width="64" height="64" alt="">

# Aagedal NLE Project Converter

> **Alpha Release (`v0.1.0-alpha.1`)** — Core Premiere Pro ↔ DaVinci Resolve and Final Cut Pro conversions are actively tested against live NLE builds, but edge cases remain. **Always convert copies of production projects** and keep original project backups.

A free, open-source (**GPL-3.0-or-later**) desktop GUI and CLI built in **100% Rust** (**Rust + GPUI**) to convert NLE project files directly — 100% locally, without cloud dependencies or lossy intermediate XML round-trips.

## Supported Formats & Compatibility

| Format | Read | Write | Verified Target Version / Notes |
|---|:---:|:---:|---|
| **Adobe Premiere Pro** (`.prproj`) | Yes | Yes | Writes Premiere Pro 2026 (`Version="45"`). Earlier Premiere Pro versions may reject native `.prproj` output (use FCP7 XML `.xml` as fallback). |
| **DaVinci Resolve** (`.drp` / `.drt`) | Yes | Yes | Writes DaVinci Resolve 21 (`DbPrjVer="17"`). Tested live on Resolve Studio 21.2. Earlier Resolve versions may reject native `.drp` output (use `.fcpxml` or `.otio` as fallback). |
| **Final Cut Pro** (`.fcpbundle`, `.fcpxml`, `.fcpxmld`) | Yes | Yes (`.fcpxml` 1.11) | Pure-Rust `.fcpbundle` CoreData/plist and FCPXML 1.8–1.15 reader verified against Final Cut Pro 11. `.fcpxml` output is currently **experimental** (pending full live FCP import verification). |
| **OpenTimelineIO** (`.otio`, `.otiod`, `.otioz`) | Yes | Yes (`.otio`) | Reads `Timeline.1`, `SerializableCollection.1`, and nested `Stack.1` (including `.otiod`/`.otioz` bundles and modern `media_references`). Writer exports `Timeline.1` for single-timeline or `SerializableCollection.1` for multi-timeline selections (auto-flattens nested/multicam clips). |
| **FCP7 XML** (`.xml`) | Yes | Yes | XMEML v4/v5; reads and writes full `<project>` bins, multiple `<sequence>`s, Motion/Crop/Opacity/Blend/Time Remap filters (including variable speed ramps), transitions, and `<link>` groups (reader auto-detects `<fcpxml>` in `.xml` files; writer auto-flattens nested/multicam clips). |
| **CMX 3600 EDL** (`.edl`) | Yes | Yes | Cuts, dissolves/wipes, blanks/gaps, `M2` retiming/reverse/freeze, `* FROM CLIP NAME:` / `* SOURCE FILE:`, Avid `* LOC:` & Resolve pipe markers, and `A1`–`A4` audio. Ideal for Final Cut Pro conform & VFX handoffs. |
| **AAF** (`.aaf`) | Yes | Yes (experimental) | Pure-Rust OLE2/CFB Structured Storage (v3/v4) AAF v1.1 reader & writer (`CompositionMob`, `MasterMob`, `SourceMob`, `NetworkLocator`, multi-track video/audio, transitions, `VideoSpeedControl`, markers, nested sequences). Enables Final Cut Pro ↔ Avid / Pro Tools / Resolve AAF workflows. `.aaf` output is currently **experimental** pending live Avid/Pro Tools verification. |

## Highlights

- **Direct Native `.prproj` ↔ `.drp` Conversion**: Reads and writes Adobe Premiere Pro's gzipped XML object graph (`.prproj`, exact `254,016,000,000` ticks/sec rational time math) and DaVinci Resolve's `.drp` project archives (`project.xml`, `MediaPool/**/MpFolder.xml`, `SeqContainer/<UUID>.xml`, Fairlight `FLStudioModelBA`, `SetupBA`, `MediaTimemapBA`, `EffectFiltersBA`, DRX primary grades).
- **Direct Final Cut Pro Input**: Reads both `.fcpxml` / `.fcpxmld` exports and native `.fcpbundle` libraries directly in pure Rust — no Final Cut Pro installation required.
- **High-Fidelity Timeline Preservation**: Preserves bins and unplaced media, standard & custom clip metadata (Premiere ↔ Resolve), Motion/Crop/Opacity/Blend Modes, Lumetri ↔ Resolve DRX primary color grades, video/audio transitions & fades, clip colors, sequence & clip markers, stereo/mono tracks & multi-bus audio routing, constant/reverse/freeze retiming, nested timelines, compound clips, and multicam cuts (plus optional `--flatten-nested` flattening).
- **Interactive Effect Mapping & Cross-NLE Plugin Detection**: Scans clips for app-exclusive effects (e.g. *Warp Stabilizer*, *Ultra Key*, *Gaussian Blur*, *DeNoise*, third-party plugins) and locally installed cross-NLE plugins (Premiere MediaCore, Resolve OFX, FCP FxPlug / Motion Templates, AU, VST3), lets you map each to native equivalents or color-coded timeline review markers with original parameters, and supports reusable JSON presets.
- **Workstation Media Relinking**: Prefix-based media path relinking (e.g. `D:\Media` → `/Volumes/RAID/Media`) in both GUI and CLI.
- **Single-Language Native Rust Stack**: Core libraries, CLI (`nle-convert`), and GPU-accelerated desktop GUI (`aagedal-nle-converter` via `gpui-kit`) in a single Cargo workspace — zero WebView, Electron, or Node/JS runtime.

## Known Alpha Limitations

- **Variable speed ramps**: Multi-point `SpeedKeyframe` curves round-trip across FCPXML, FCP7 XML, and Resolve `MediaTimemapBA`; native Premiere `TimeRemapping` keyframe curve writing is still in progress.
- **Resolve Clip Attributes blobs**: `DataLevels` and `AlphaMode` round-trip into Resolve Media Pool blobs is still pending.
- **Final Cut Pro**: `.fcpbundle` compound/multicam reading and live Final Cut Pro import verification of generated `.fcpxml` files are still in progress.

See [TODO.md](TODO.md) for the full roadmap and live-NLE test matrix.

## Workspace Layout

| Path | Description |
|---|---|
| `crates/nle-core` | Canonical IR (`Project`, `Sequence`, `Track`, `Clip`, `EffectInstance`, exact `Rational` time), flattening pass, & traits |
| `crates/nle-effects` | Cross-NLE effect equivalency catalog, local plugin scanner, interactive `EffectMappingPlan`, & JSON presets |
| `crates/nle-premiere` | Direct Adobe Premiere Pro `.prproj` reader, native `.prproj` writer, & synthetic demo fixture generator |
| `crates/nle-resolve` | Direct DaVinci Resolve `.drp` / `.drt` native reader & writer |
| `crates/nle-interchange` | Pure-Rust `.fcpbundle` & FCPXML 1.8–1.15, OpenTimelineIO (`.otio`/`.otiod`/`.otioz`), FCP7 XMEML v4/v5, CMX 3600 EDL (`.edl`), and AAF (`.aaf`) readers & writers |
| `crates/nle-cli` | `nle-convert` CLI (`inspect`, `convert`, `plugins`, `demo`) |
| `crates/nle-gui` | Native GPU-accelerated GPUI desktop GUI (`aagedal-nle-converter`) with drag-and-drop, settings, presets, & Reveal-in-Finder |

## Prerequisites

- Rust toolchain (`cargo` / `rustc`, Rust edition 2024)

## Running & Building

```bash
# Run all workspace unit & round-trip tests
cargo test --workspace

# Run the native GPUI desktop GUI
cargo run -p nle-gui

# Build release binaries (GUI + CLI)
cargo build --release -p nle-gui -p nle-cli

# Package a macOS .app bundle, .dmg, and .zip into dist/
./scripts/package_macos.sh
```

## CLI Usage (`nle-convert`)

```bash
# Print CLI version
cargo run -p nle-cli -- --version

# Generate a sample Premiere Pro project with bins, Lumetri, Motion, and effects
cargo run -p nle-cli -- demo /tmp/demo.prproj

# Inspect a project (.prproj, .drp, .drt, .fcpxml, .fcpxmld, .fcpbundle, .otio/.otiod/.otioz, .xml, .edl, or .aaf)
cargo run -p nle-cli -- inspect /tmp/demo.prproj --target drp --save-preset /tmp/preset.json

# Convert directly between NLE formats (with optional preset, relinking, sequence filter, or flattening)
cargo run -p nle-cli -- convert /tmp/demo.prproj -o /tmp/demo.drp \
  --preset /tmp/preset.json \
  --relink "D:\Media=/Volumes/RAID/Media"

# List locally installed cross-NLE third-party plugins (add --all for single-NLE plugins)
cargo run -p nle-cli -- plugins
```

## License

Licensed under the **GNU General Public License v3.0 or later (GPL-3.0-or-later)** — see [LICENSE](LICENSE).  
See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for MIT-licensed open-source format research and template attributions.
