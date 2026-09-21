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
DBDELVE_CHANNEL=release DBDELVE_SIGN_ID=- dev/bundle.sh

# A symlink beside the app is the whole of the drag-to-install convention.
rm -rf target/dmg "$DMG"
mkdir -p target/dmg
cp -R target/DBDelve.app target/dmg/
ln -s /Applications target/dmg/Applications
hdiutil create -volname "DBDelve $VERSION" -srcfolder target/dmg -ov -format UDZO "$DMG"

# The tag is pushed before the release exists because the push is what starts
# the Linux build; the release is a draft until that build's tarballs are on it,
# so the release page never goes public holding macOS alone.
git tag -a "v$VERSION" -m "DBDelve $VERSION"
git push origin "v$VERSION"

gh release create "v$VERSION" "$DMG" --title "DBDelve $VERSION" --generate-notes --draft

# Matched on the tag rather than taken as the newest run: a run for some other
# tag or an earlier re-run would have this script watching the wrong build and
# publishing on the strength of it. A tag push records the tag as the run's head
# branch, which is what --branch filters on. The wait covers the few seconds
# GitHub takes to register the run at all; no run after that means the push
# never reached CI, and the draft is left for a hand-made upload.
RUN=""
for _ in $(seq 30); do
  RUN="$(gh run list --workflow release-linux.yml --event push --branch "v$VERSION" \
    --limit 1 --json databaseId --jq '.[0].databaseId // empty')"
  [[ -n "$RUN" ]] && break
  sleep 5
done
if [[ -z "$RUN" ]]; then
  echo "no release-linux run for v$VERSION -- v$VERSION is still a draft" >&2
  exit 1
fi

gh run watch "$RUN" --exit-status

# A green run is not proof the assets landed: the upload step can be skipped by
# a condition or clobber the wrong release. Every file both arches are supposed
# to produce is named here, so a half-uploaded matrix -- or a matrix that built
# the tarball and lost the AppImage -- fails while the release is still a draft.
ASSETS="$(gh release view "v$VERSION" --json assets --jq '.assets[].name')"
for ARCH in x86_64 aarch64; do
  for ASSET in "dbdelve-$VERSION-linux-$ARCH.tar.gz" "dbdelve-$VERSION-$ARCH.AppImage"; do
    if ! grep -qxF "$ASSET" <<<"$ASSETS"; then
      echo "no $ASSET on v$VERSION -- v$VERSION is still a draft" >&2
      exit 1
    fi
  done
done

gh release edit "v$VERSION" --draft=false

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
  desc "Native SQL client"
  homepage "https://github.com/ShayanAbbas1/dbdelve"

  depends_on arch: :arm64
  depends_on macos: :monterey

  app "DBDelve.app"

  # Lowercase: the app writes "dbdelve", and a zap spelled any other way
  # silently removes nothing. Saved passwords live in the Keychain, which zap
  # cannot reach at all.
  zap trash: "~/Library/Application Support/dbdelve"
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
