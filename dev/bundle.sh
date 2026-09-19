#!/usr/bin/env bash
#
# Builds DBDelve as a macOS .app and installs it to /Applications.
#
# Usage: dev/bundle.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
APP="target/DBDelve.app"
INSTALLED="/Applications/DBDelve.app"

cargo build --release

rm -rf "$APP" target/DBDelve.iconset
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

swift dev/icon.swift target/DBDelve.iconset
iconutil -c icns target/DBDelve.iconset -o "$APP/Contents/Resources/DBDelve.icns"
rm -rf target/DBDelve.iconset
# Capitalised: the menu bar, Force Quit and Activity Monitor all name the app
# after its executable, not after CFBundleName.
cp target/release/dbdelve "$APP/Contents/MacOS/DBDelve"

# The OFL asks that the licence travel with the fonts, and the fonts are
# compiled into the binary above -- so the notices ship inside the bundle
# rather than only sitting in the repository.
cp NOTICES.md "$APP/Contents/Resources/"
cp -R licenses "$APP/Contents/Resources/"

# CFBundleIdentifier is what the Keychain scopes saved profile passwords to.
# Changing it orphans every password already stored.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key><string>DBDelve</string>
	<key>CFBundleDisplayName</key><string>DBDelve</string>
	<key>CFBundleIdentifier</key><string>com.shayanabbas.dbdelve</string>
	<key>CFBundleExecutable</key><string>DBDelve</string>
	<key>CFBundleIconFile</key><string>DBDelve</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>${VERSION}</string>
	<key>CFBundleVersion</key><string>${VERSION}</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>LSMinimumSystemVersion</key><string>12.0</string>
	<key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
	<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
plutil -lint -s "$APP/Contents/Info.plist"

# An arm64 binary will not launch without a signature, and copying it into the
# bundle invalidates the one rustc left behind. DBDELVE_SIGN_ID takes a real
# identity when there is one to distribute under, and dev/identity.sh's
# self-signed one is what keeps the Keychain from re-asking after every
# rebuild. Ad-hoc is the fallback, and it runs -- it just prompts.
DEV_IDENTITY="DBDelve Dev Signing"
if [[ -z "${DBDELVE_SIGN_ID:-}" ]] &&
  security find-certificate -c "$DEV_IDENTITY" >/dev/null 2>&1; then
  DBDELVE_SIGN_ID="$DEV_IDENTITY"
fi
codesign --force --sign "${DBDELVE_SIGN_ID:--}" "$APP"
codesign --verify --strict "$APP"

# dev/release.sh wants the bundle but not the install: the release is signed
# ad-hoc, and dropping that over /Applications costs the Keychain's "Always
# Allow" on the copy actually being used day to day.
if [[ "${DBDELVE_INSTALL:-1}" != 1 ]]; then
  echo "built $APP (v$VERSION)"
  exit 0
fi

rm -rf "$INSTALLED"
ditto "$APP" "$INSTALLED"

# Finder and the Dock both cache icons per bundle path, so a swapped icon does
# not show up until the mtime moves and the Dock restarts.
touch "$INSTALLED"
killall Dock 2>/dev/null || true

echo "installed $INSTALLED (v$VERSION)"
