#!/usr/bin/env bash
set -euo pipefail

# Package Aagedal NLE Project Converter as a macOS .app bundle (+ .zip and .dmg) in dist/.
#
# Usage:
#   ./scripts/package_macos.sh [--target <rust-triple>] [--skip-build] [--no-dmg]
#
# Environment variables:
#   CODESIGN_IDENTITY   Optional Developer ID Application identity (defaults to ad-hoc "-")

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TARGET=""
SKIP_BUILD=0
BUILD_DMG=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      TARGET="${2:?missing value for --target}"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --no-dmg)
      BUILD_DMG=0
      shift
      ;;
    -h|--help)
      sed -n '3,11p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

VERSION="$(awk -F'"' '/^version[[:space:]]*=/{print $2; exit}' Cargo.toml)"
SHORT_VERSION="${VERSION%%-*}"
APP_NAME="Aagedal NLE Project Converter"
BUNDLE_ID="me.aagedal.nle-project-converter"
GUI_BIN="aagedal-nle-converter"
CLI_BIN="nle-convert"

if [[ -n "$TARGET" ]]; then
  ARCH_LABEL="$TARGET"
  BIN_DIR="$ROOT_DIR/target/$TARGET/release"
  CARGO_TARGET_ARGS=(--target "$TARGET")
else
  ARCH_LABEL="macos-$(uname -m)"
  BIN_DIR="$ROOT_DIR/target/release"
  CARGO_TARGET_ARGS=()
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "Building release binaries ($VERSION)..."
  cargo build --release -p nle-gui -p nle-cli "${CARGO_TARGET_ARGS[@]}"
fi

if [[ ! -x "$BIN_DIR/$GUI_BIN" ]]; then
  echo "Expected GUI binary not found: $BIN_DIR/$GUI_BIN" >&2
  exit 1
fi
if [[ ! -x "$BIN_DIR/$CLI_BIN" ]]; then
  echo "Expected CLI binary not found: $BIN_DIR/$CLI_BIN" >&2
  exit 1
fi

DIST_DIR="$ROOT_DIR/dist"
APP_DIR="$DIST_DIR/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$BIN_DIR/$GUI_BIN" "$MACOS_DIR/$GUI_BIN"
cp "$BIN_DIR/$CLI_BIN" "$MACOS_DIR/$CLI_BIN"
chmod +x "$MACOS_DIR/$GUI_BIN" "$MACOS_DIR/$CLI_BIN"
cp "$ROOT_DIR/assets/icons/icon.icns" "$RESOURCES_DIR/AppIcon.icns"
cp "$ROOT_DIR/LICENSE" "$RESOURCES_DIR/LICENSE"
cp "$ROOT_DIR/THIRD_PARTY_NOTICES.md" "$RESOURCES_DIR/THIRD_PARTY_NOTICES.md"

cat > "$CONTENTS_DIR/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key>
  <string>${APP_NAME}</string>
  <key>CFBundleIdentifier</key>
  <string>${BUNDLE_ID}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>CFBundleShortVersionString</key>
  <string>${SHORT_VERSION}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleExecutable</key>
  <string>${GUI_BIN}</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.video</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key>
  <true/>
  <key>NSHumanReadableCopyright</key>
  <string>Licensed under GNU GPL v3.0 or later (GPL-3.0-or-later)</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key>
      <string>Adobe Premiere Pro Project</string>
      <key>CFBundleTypeRole</key>
      <string>Viewer</string>
      <key>CFBundleTypeExtensions</key>
      <array><string>prproj</string></array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key>
      <string>DaVinci Resolve Project</string>
      <key>CFBundleTypeRole</key>
      <string>Viewer</string>
      <key>CFBundleTypeExtensions</key>
      <array><string>drp</string><string>drt</string></array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key>
      <string>Final Cut Pro Project / Library</string>
      <key>CFBundleTypeRole</key>
      <string>Viewer</string>
      <key>CFBundleTypeExtensions</key>
      <array><string>fcpxml</string><string>fcpxmld</string><string>fcpbundle</string></array>
    </dict>
  </array>
</dict>
</plist>
EOF

SIGN_ID="${CODESIGN_IDENTITY:--}"
echo "Signing app bundle with identity: $SIGN_ID"
codesign --force --deep --sign "$SIGN_ID" "$APP_DIR"

ZIP_PATH="$DIST_DIR/aagedal-nle-project-converter-${VERSION}-${ARCH_LABEL}.zip"
rm -f "$ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"
echo "Created $ZIP_PATH"

if [[ "$BUILD_DMG" -eq 1 ]]; then
  DMG_STAGE="$DIST_DIR/.dmg-stage"
  DMG_PATH="$DIST_DIR/aagedal-nle-project-converter-${VERSION}-${ARCH_LABEL}.dmg"
  trap 'rm -rf "$DMG_STAGE"' EXIT
  rm -rf "$DMG_STAGE" "$DMG_PATH"
  mkdir -p "$DMG_STAGE"
  cp -R "$APP_DIR" "$DMG_STAGE/"
  ln -s /Applications "$DMG_STAGE/Applications"
  hdiutil create \
    -volname "$APP_NAME" \
    -srcfolder "$DMG_STAGE" \
    -ov \
    -format UDZO \
    "$DMG_PATH"
  rm -rf "$DMG_STAGE"
  trap - EXIT
  echo "Created $DMG_PATH"
fi
