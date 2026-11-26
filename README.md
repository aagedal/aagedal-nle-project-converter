<img src="assets/icon.svg" width="64" height="64" alt="">

# Aagedal NLE Project Converter

A free, open-source (**GPL-3.0**) desktop GUI and CLI built in **100% Rust** (**Rust + GPUI**) to convert NLE project files directly — without going through lossy XML exports.

## Highlights

- **Direct Native `.prproj` ↔ `.drp` Conversion**: Reads and writes both Adobe Premiere Pro's gzipped XML object graph (`.prproj`, exact `254,016,000,000` ticks/sec rational time math, native bins, sequences, markers, links, Motion/Opacity/Lumetri, nested sequences, and `<IsMulticam>` cuts) and DaVinci Resolve's `.drp` project archives (`project.xml`, `MediaPool/**/MpFolder.xml`, `SeqContainer/<UUID>.xml`, Fairlight `FLStudioModelBA`, `SetupBA`, `MediaTimemapBA`, clip colors, markers, compound/multicam clips, and native DRX Lumetri grades).
- **Multi-NLE Architecture**: Built around a canonical rational-time Intermediate Representation (`nle-core`), plus Final Cut Pro (`.fcpxml`), OpenTimelineIO (`.otio`), and FCP7 XML (`.xml`) exporters.
- **Interactive Effect Translation & Mapping**: Automatically scans every clip for app-exclusive effects (e.g. *Warp Stabilizer*, *Ultra Key*, *Gaussian Blur*, *Essential Graphics*, *Morph Cut*, *DeNoise*, third-party plugins), asks the user how to map each one to target-NLE equivalents (or place color-coded timeline review markers with original parameters), and supports reusable JSON mapping presets.
- **Single-Language Native Rust Stack**: Core library, CLI, and GPU-accelerated desktop GUI (`gpui-kit`) are all written in native Rust in a single Cargo workspace — zero WebView, WASM, or Node/JS build pipeline required.

## Workspace Layout

| Path | Description |
|---|---|
| `crates/nle-core` | Canonical IR (`Project`, `Sequence`, `Track`, `Clip`, `EffectInstance`, exact `Rational` time) & traits |
| `crates/nle-effects` | Cross-NLE effect equivalency catalog, interactive `EffectMappingPlan`, and preset system |
| `crates/nle-premiere` | Direct Adobe Premiere Pro `.prproj` reader, native `.prproj` writer, & synthetic demo fixture generator |
| `crates/nle-resolve` | Direct DaVinci Resolve `.drp` native writer & `.drp` reader |
| `crates/nle-interchange` | Final Cut Pro `.fcpxml` 1.11, OpenTimelineIO `.otio`, and FCP7 XMEML v5 writers |
| `crates/nle-cli` | `nle-convert` CLI (`inspect`, `convert`, `demo`) |
| `crates/nle-gui` | Native GPU-accelerated GPUI desktop GUI (`aagedal-nle-converter`) with native file dialogs, drag-and-drop, and Reveal-in-Finder |

## Prerequisites

- Rust toolchain (`cargo` / `rustc`)

## Running & Building

```bash
# Run all workspace unit & round-trip tests
cargo test --workspace

# Run the native GPUI desktop GUI
cargo run -p nle-gui

# Build a release desktop binary
cargo build --release -p nle-gui

# Use the CLI directly
cargo run -p nle-cli -- demo /tmp/demo.prproj
cargo run -p nle-cli -- inspect /tmp/demo.prproj
cargo run -p nle-cli -- convert /tmp/demo.prproj -o /tmp/demo.drp
```

## License

Licensed under the **GNU General Public License v3.0 (GPL-3.0-or-later)** — see [LICENSE](LICENSE).  
See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for MIT-licensed open-source format research and template attributions.
