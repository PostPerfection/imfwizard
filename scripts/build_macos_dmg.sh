#!/bin/bash
# macOS .dmg bundle creation script with code signing and notarization
# Prerequisites:
#   - Xcode command line tools
#   - Apple Developer certificate (Developer ID Application)
#   - App-specific password for notarization
#
# Environment variables:
#   APPLE_ID          - Apple ID for notarization
#   APPLE_PASSWORD    - App-specific password
#   APPLE_TEAM_ID    - Developer Team ID
#   SIGNING_IDENTITY  - Certificate name (e.g. "Developer ID Application: ...")

set -euo pipefail

APP_NAME="IMF Wizard"
APP_BUNDLE="${APP_NAME}.app"
DMG_NAME="IMFWizard-$(git describe --tags --always 2>/dev/null || echo 'dev').dmg"
BUILD_DIR="build/macos"
STAGING_DIR="${BUILD_DIR}/staging"

echo "=== Building IMF Wizard for macOS ==="

# Build CLI (universal)
cd rust
cargo build --release --target aarch64-apple-darwin -p imfwizard-cli
cargo build --release --target x86_64-apple-darwin -p imfwizard-cli
lipo -create \
  target/aarch64-apple-darwin/release/imfwizard \
  target/x86_64-apple-darwin/release/imfwizard \
  -output target/release/imfwizard-universal
cd ..

# Build GUI (Tauri)
cd gui
pnpm install --frozen-lockfile
cd src-tauri
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# Create universal binary
lipo -create \
  target/aarch64-apple-darwin/release/imfwizard-gui \
  target/x86_64-apple-darwin/release/imfwizard-gui \
  -output target/release/imfwizard-gui-universal
cd ../..

# Create app bundle structure
rm -rf "${STAGING_DIR}"
mkdir -p "${STAGING_DIR}/${APP_BUNDLE}/Contents/MacOS"
mkdir -p "${STAGING_DIR}/${APP_BUNDLE}/Contents/Resources"

# Copy binaries
cp rust/target/release/imfwizard-universal "${STAGING_DIR}/${APP_BUNDLE}/Contents/MacOS/imfwizard"
cp gui/src-tauri/target/release/imfwizard-gui-universal \
   "${STAGING_DIR}/${APP_BUNDLE}/Contents/MacOS/${APP_NAME}"

# Create Info.plist
cat > "${STAGING_DIR}/${APP_BUNDLE}/Contents/Info.plist" << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>IMF Wizard</string>
  <key>CFBundleDisplayName</key>
  <string>IMF Wizard</string>
  <key>CFBundleIdentifier</key>
  <string>com.imfwizard.desktop</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleExecutable</key>
  <string>IMF Wizard</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeExtensions</key>
      <array>
        <string>mxf</string>
        <string>j2k</string>
        <string>j2c</string>
      </array>
      <key>CFBundleTypeName</key>
      <string>IMF Media</string>
      <key>CFBundleTypeRole</key>
      <string>Editor</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Copy icon
cp gui/src-tauri/icons/icon.icns "${STAGING_DIR}/${APP_BUNDLE}/Contents/Resources/"

# Code sign if identity provided
if [ -n "${SIGNING_IDENTITY:-}" ]; then
  echo "=== Code signing ==="
  codesign --force --deep --sign "${SIGNING_IDENTITY}" \
    --options runtime \
    --entitlements scripts/entitlements.plist \
    "${STAGING_DIR}/${APP_BUNDLE}"
  echo "Code signing complete"
fi

# Create DMG
echo "=== Creating DMG ==="
mkdir -p "${BUILD_DIR}"
hdiutil create -volname "${APP_NAME}" \
  -srcfolder "${STAGING_DIR}" \
  -ov -format UDZO \
  "${BUILD_DIR}/${DMG_NAME}"

# Code sign DMG
if [ -n "${SIGNING_IDENTITY:-}" ]; then
  codesign --force --sign "${SIGNING_IDENTITY}" "${BUILD_DIR}/${DMG_NAME}"
fi

# Notarize if credentials provided
if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  echo "=== Notarizing ==="
  xcrun notarytool submit "${BUILD_DIR}/${DMG_NAME}" \
    --apple-id "${APPLE_ID}" \
    --password "${APPLE_PASSWORD}" \
    --team-id "${APPLE_TEAM_ID}" \
    --wait

  # Staple
  xcrun stapler staple "${BUILD_DIR}/${DMG_NAME}"
  echo "Notarization complete"
fi

echo "=== Done: ${BUILD_DIR}/${DMG_NAME} ==="
ls -lh "${BUILD_DIR}/${DMG_NAME}"
