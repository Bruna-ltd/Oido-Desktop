#!/usr/bin/env bash
# Publishes a release that installed apps update to: scripts/release.sh 0.2.0
#
# Sets the version in Cargo.toml, tests, builds, signs, commits and tags vX.Y.Z, pushes, and creates
# the GitHub release with this machine's binary, its signature and its SHA-256.
#
# Signing needs the release key's PEM in OIDO_SIGNING_KEY (a repository secret on the runner;
# locally: OIDO_SIGNING_KEY="$(cat ~/.config/oido-release/release-key.pem)"). It must be the key
# scripts/release.pub names, which is the one apps check updates against. Only a dry run goes
# without it. Run it from a clean tree on the
# branch you release from. Cargo.toml patches Rune to ../rune, so that must be there: the
# release workflow (.github/workflows/release.yml) puts it there and runs this on every merge.
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

# What apps check: the key's Ed25519 signature of "oido-desktop <version> <sha256>" (src/update.rs).
if [[ -n ${OIDO_SIGNING_KEY:-} ]]; then
    key=$(mktemp) # only this user can read it
    trap 'rm -f "$key"' EXIT
    printf '%s\n' "$OIDO_SIGNING_KEY" >"$key"
    pub=$(openssl pkey -in "$key" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')
    if [[ $pub != $(tr -d ' \n' <scripts/release.pub) ]]; then
        echo "OIDO_SIGNING_KEY is not the key scripts/release.pub names" >&2
        exit 1
    fi
    printf 'oido-desktop %s %s' "$version" "$(cut -d' ' -f1 "$dist/$asset.sha256")" >"$dist/signed"
    openssl pkeyutl -sign -inkey "$key" -rawin -in "$dist/signed" -out "$dist/$asset.sig"
    openssl pkeyutl -verify -inkey "$key" -rawin -in "$dist/signed" -sigfile "$dist/$asset.sig" >/dev/null
elif [[ -n ${OIDO_DRY_RUN:-} ]]; then
    echo "no OIDO_SIGNING_KEY: not signed (a dry run publishes nothing)"
else
    echo "set OIDO_SIGNING_KEY to the release key's PEM: apps install only signed updates" >&2
    git checkout -- Cargo.toml Cargo.lock
    exit 1
fi

# OIDO_DRY_RUN=1 stops here, with everything built and checked and nothing changed or published.
if [[ -n ${OIDO_DRY_RUN:-} ]]; then
    git checkout -- Cargo.toml Cargo.lock
    echo "dry run: built and checked $dist/$asset for v$version; nothing committed, tagged, pushed or published"
    exit 0
fi

# A version already in Cargo.toml (bumped in a pull request) has nothing to commit.
git diff --quiet || git commit -qam "Release v$version"
git tag -a "v$version" -m "v$version"
# Both or neither: a branch that moved on must not leave a tag with no release behind it.
git push --atomic origin HEAD "v$version"
gh release create "v$version" "$dist/$asset" "$dist/$asset.sig" "$dist/$asset.sha256" \
    --title "v$version" --generate-notes
