#!/usr/bin/env python3
"""Generate PNG, macOS .icns, and Windows .ico app icons from assets/icon.svg.

Requires `rsvg-convert` (librsvg) and macOS `iconutil` (for .icns).
"""

from __future__ import annotations

import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SVG_PATH = ROOT / "assets" / "icon.svg"
ICONS_DIR = ROOT / "assets" / "icons"

MACOS_WRAPPER_TEMPLATE = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>
    <filter id="dock-shadow" x="-20%" y="-20%" width="140%" height="145%">
      <feDropShadow dx="0" dy="18" stdDeviation="20" flood-color="#000000" flood-opacity="0.38"/>
    </filter>
  </defs>
  <g transform="translate(100, 100) scale(0.8046875)" filter="url(#dock-shadow)">
    {inner}
  </g>
</svg>
"""


def render_svg(svg_path: Path, out_png: Path, size: int) -> None:
    subprocess.run(
        [
            "rsvg-convert",
            "-w",
            str(size),
            "-h",
            str(size),
            "-o",
            str(out_png),
            str(svg_path),
        ],
        check=True,
    )


def build_ico(png_frames: list[tuple[int, bytes]], out_ico: Path) -> None:
    """Pack multiple PNG-compressed frames into a valid Windows .ico file."""
    count = len(png_frames)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    entries = bytearray()
    payload = bytearray()

    for size, data in png_frames:
        dim = 0 if size >= 256 else size
        entries.extend(
            struct.pack(
                "<BBBBHHII",
                dim,
                dim,
                0,
                0,
                1,
                32,
                len(data),
                offset,
            )
        )
        payload.extend(data)
        offset += len(data)

    out_ico.write_bytes(header + bytes(entries) + bytes(payload))


def build_icns(png_entries: list[tuple[bytes, bytes]], out_icns: Path) -> None:
    """Pack PNG-encoded icon frames into a valid macOS .icns container."""
    body = bytearray()
    for ostype, data in png_entries:
        assert len(ostype) == 4
        body.extend(ostype)
        body.extend(struct.pack(">I", 8 + len(data)))
        body.extend(data)
    total_len = 8 + len(body)
    out_icns.write_bytes(b"icns" + struct.pack(">I", total_len) + bytes(body))


def make_macos_padded_svg(src_svg: Path, dst_svg: Path) -> None:
    raw = src_svg.read_text(encoding="utf-8")
    # Extract inner SVG content between <svg ...> and </svg>
    start = raw.find(">") + 1
    end = raw.rfind("</svg>")
    inner = raw[start:end]
    dst_svg.write_text(MACOS_WRAPPER_TEMPLATE.format(inner=inner), encoding="utf-8")


def main() -> None:
    ICONS_DIR.mkdir(parents=True, exist_ok=True)

    # Full-bleed PNGs for in-app UI, Linux desktop entries, README, etc.
    for filename, size in [
        ("32x32.png", 32),
        ("64x64.png", 64),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
    ]:
        render_svg(SVG_PATH, ICONS_DIR / filename, size)

    with tempfile.TemporaryDirectory() as tmp:
        tmp_dir = Path(tmp)

        # Windows multi-resolution .ico (PNG-encoded frames)
        ico_frames: list[tuple[int, bytes]] = []
        for size in [16, 32, 48, 64, 128, 256]:
            frame_png = tmp_dir / f"ico_{size}.png"
            render_svg(SVG_PATH, frame_png, size)
            ico_frames.append((size, frame_png.read_bytes()))
        build_ico(ico_frames, ICONS_DIR / "icon.ico")

        # macOS HIG-padded icon (824x824 squircle + subtle drop shadow inside 1024x1024)
        macos_svg = tmp_dir / "icon_macos.svg"
        make_macos_padded_svg(SVG_PATH, macos_svg)
        render_svg(macos_svg, ICONS_DIR / "icon-macos.png", 512)

        # Build multi-resolution macOS .icns (standard 1x + Retina 2x PNG chunks)
        icns_sizes: dict[int, bytes] = {}
        for size in [16, 32, 64, 128, 256, 512, 1024]:
            png_path = tmp_dir / f"icns_{size}.png"
            render_svg(macos_svg, png_path, size)
            icns_sizes[size] = png_path.read_bytes()

        icns_entries = [
            (b"icp4", icns_sizes[16]),    # 16x16
            (b"icp5", icns_sizes[32]),    # 32x32
            (b"icp6", icns_sizes[64]),    # 64x64
            (b"ic07", icns_sizes[128]),   # 128x128
            (b"ic08", icns_sizes[256]),   # 256x256
            (b"ic09", icns_sizes[512]),   # 512x512
            (b"ic10", icns_sizes[1024]),  # 512x512@2x (1024x1024)
            (b"ic11", icns_sizes[32]),    # 16x16@2x
            (b"ic12", icns_sizes[64]),    # 32x32@2x
            (b"ic13", icns_sizes[256]),   # 128x128@2x
            (b"ic14", icns_sizes[512]),   # 256x256@2x
        ]
        build_icns(icns_entries, ICONS_DIR / "icon.icns")

    print("Generated icons in", ICONS_DIR)


if __name__ == "__main__":
    main()
