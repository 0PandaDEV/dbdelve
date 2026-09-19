#!/usr/bin/env bash
#
# Cuts a release: builds the .app, wraps it in a DMG, publishes it to GitHub
# and repoints the Homebrew cask at it.
#
# Usage: dev/release.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
DMG="target/DBDelve-$VERSION.dmg"
TAP="${DBDELVE_TAP:-$ROOT/../homebrew-dbdelve}"

# Ad-hoc, deliberately, rather than dev/identity.sh's certificate: that one is
# trusted on this machine and nowhere else, and a signature from an issuer the
# user does not trust reads worse to Gatekeeper than an anonymous one. The
# price is that the hash moves every release, so the Keychain asks once more
# after each update.
DBDELVE_SIGN_ID=- DBDELVE_INSTALL=0 dev/bundle.sh

# A symlink beside the app is the whole of the drag-to-install convention.
rm -rf target/dmg "$DMG"
mkdir -p target/dmg
cp -R target/DBDelve.app target/dmg/
ln -s /Applications target/dmg/Applications
hdiutil create -volname "DBDelve $VERSION" -srcfolder target/dmg -ov -format UDZO "$DMG"

gh release create "v$VERSION" "$DMG" --title "DBDelve $VERSION" --generate-notes

if [[ -d "$TAP/.git" ]]; then
  SHA="$(shasum -a 256 "$DMG" | cut -d' ' -f1)"
  mkdir -p "$TAP/Casks"
  # Rewritten whole rather than patched in place: two substitutions that have to
  # agree are two chances for the version and the checksum to drift apart.
  cat > "$TAP/Casks/dbdelve.rb" <<CASK
cask "dbdelve" do
  version "$VERSION"
  sha256 "$SHA"

  url "https://github.com/ShayanAbbas1/dbdelve/releases/download/v#{version}/DBDelve-#{version}.dmg"
  name "DBDelve"
  desc "Native macOS SQL client"
  homepage "https://github.com/ShayanAbbas1/dbdelve"

  depends_on arch: :arm64
  depends_on macos: ">= :monterey"

  app "DBDelve.app"

  # Saved passwords live in the Keychain, which zap cannot reach.
  zap trash: "~/Library/Application Support/DBDelve"
end
CASK
  git -C "$TAP" add Casks/dbdelve.rb
  git -C "$TAP" commit -m "DBDelve $VERSION"
  git -C "$TAP" push
  echo "cask updated: $TAP/Casks/dbdelve.rb"
else
  echo "no tap at $TAP -- skipped the cask"
fi

echo "released v$VERSION"
