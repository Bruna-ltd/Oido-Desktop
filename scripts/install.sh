#!/usr/bin/env bash
# Installs Oido Desktop from its latest GitHub release:
#
#   curl -fsSL https://raw.githubusercontent.com/Bruna-ltd/Oido-Desktop/main/scripts/install.sh | bash
#
# Downloads the binary for this machine and its SHA-256, checks them, and puts the binary at
# ~/.local/bin/oido-desktop. The first launch adds Oido to the app launcher, and a binary in a
# folder you can write to updates itself from then on.
#
# OIDO_VERSION=0.2.0   install that release rather than the latest
# OIDO_INSTALL_DIR=…   install there rather than in ~/.local/bin
set -euo pipefail

repo=Bruna-ltd/Oido-Desktop

die() {
    echo "oido install: $*" >&2
    exit 1
}

main() {
    [[ $(uname -s) == Linux ]] || die "Oido Desktop is built for Linux only"
    command -v curl >/dev/null || die "curl is needed"
    command -v sha256sum >/dev/null || die "sha256sum is needed"

    local asset="oido-desktop-$(uname -m)-linux"
    local dir=${OIDO_INSTALL_DIR:-$HOME/.local/bin}
    local base
    if [[ -n ${OIDO_VERSION:-} ]]; then
        base="https://github.com/$repo/releases/download/v${OIDO_VERSION#v}"
    else
        base="https://github.com/$repo/releases/latest/download"
    fi

    # Global: the trap runs after main has returned and its locals are gone.
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    echo "Downloading $asset…"
    for file in "$asset" "$asset.sha256"; do
        curl -fSL --retry 3 --progress-bar -o "$tmp/$file" "$base/$file" ||
            die "could not download $base/$file (no release for $(uname -m), or the repository is not public)"
    done
    (cd "$tmp" && sha256sum --check --quiet "$asset.sha256") ||
        die "the download does not match its checksum, so it was not installed"

    # Renamed into place, as an update does, so a copy that is running keeps running.
    mkdir -p "$dir"
    install -m 755 "$tmp/$asset" "$dir/.oido-desktop.new"
    mv -f "$dir/.oido-desktop.new" "$dir/oido-desktop"

    local version
    version=$("$dir/oido-desktop" --version 2>&1) ||
        die "installed $dir/oido-desktop but it does not start: $version"
    echo "Installed $version at $dir/oido-desktop"

    case ":$PATH:" in
    *":$dir:"*) echo "Run it with: oido-desktop" ;;
    *) echo "$dir is not on your PATH. Run $dir/oido-desktop, or add it to PATH." ;;
    esac
    echo "The first launch adds Oido to your app launcher."
}

main "$@"
