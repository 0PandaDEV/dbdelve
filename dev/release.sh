#!/usr/bin/env bash
#
# Cuts a release by pushing its tag. .github/workflows/release.yml does the
# rest: builds the DMG and the Linux assets, publishes the release and
# repoints the Homebrew cask. A tag with a hyphen (v0.2.0-rc.1) is published
# as a pre-release and leaves the cask alone.
#
# Usage: dev/release.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

# The tag is what gets built, so it has to be the commit on main everyone
# else sees -- not a local one that was never pushed.
git fetch -q origin main
if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
  echo "HEAD is not origin/main -- check out an up-to-date main first" >&2
  exit 1
fi

git tag -a "v$VERSION" -m "DBDelve $VERSION"
git push origin "v$VERSION"

# The wait covers the few seconds GitHub takes to register the run. A tag push
# records the tag as the run's head branch, which is what --branch filters on.
RUN=""
for _ in $(seq 30); do
  RUN="$(gh run list --workflow release.yml --event push --branch "v$VERSION" \
    --limit 1 --json databaseId --jq '.[0].databaseId // empty')"
  [[ -n "$RUN" ]] && break
  sleep 5
done
if [[ -z "$RUN" ]]; then
  echo "no release run for v$VERSION -- check the Actions tab" >&2
  exit 1
fi

gh run watch "$RUN" --exit-status
echo "released v$VERSION"
