#!/usr/bin/env bash
# Publishes a release that installed apps update to: scripts/release.sh 0.2.0
#
# Sets the version in Cargo.toml, tests, builds, commits and tags vX.Y.Z, pushes, and creates the
# GitHub release with this machine's binary and its SHA-256. Run it from a clean tree on the
# branch you release from. Builds here, not in CI: Cargo.toml patches Rune to ../rune.
set -euo pipefail

version=${1:-}
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "usage: scripts/release.sh X.Y.Z" >&2
    exit 1
fi
cd "$(dirname "$0")/.."
if [[ -n $(git status --porcelain) ]]; then
    echo "commit or stash your changes first" >&2
    exit 1
fi
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
    echo "v$version is already tagged" >&2
    exit 1
fi

# The package's own version: the first `version =` line, under [package].
sed -i "0,/^version = \".*\"/s//version = \"$version\"/" Cargo.toml
cargo test --quiet
cargo build --release

asset="oido-desktop-$(uname -m)-linux"
dist=target/dist
rm -rf "$dist" && mkdir -p "$dist"
cp target/release/oido-desktop "$dist/$asset"
(cd "$dist" && sha256sum "$asset" >"$asset.sha256")
[[ $("$dist/$asset" --version) == "oido-desktop $version" ]]

git commit -qam "Release v$version"
git tag -a "v$version" -m "v$version"
git push origin HEAD "v$version"
gh release create "v$version" "$dist/$asset" "$dist/$asset.sha256" \
    --title "v$version" --generate-notes
