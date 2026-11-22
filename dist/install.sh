#!/bin/sh
# Installs td (Turbo Delete) on Linux, macOS or FreeBSD.
#   curl -fsSL https://raw.githubusercontent.com/Balls0fSteel/b0s-turbo-delete/master/dist/install.sh | sh
# Environment: TD_VERSION (e.g. 1.2.3 or v1.2.3; default latest), TD_INSTALL_DIR (default ~/.local/bin).
set -eu

REPO="Balls0fSteel/b0s-turbo-delete"
INSTALL_DIR="${TD_INSTALL_DIR:-$HOME/.local/bin}"

die() {
    echo "error: $*" >&2
    exit 1
}

fetch() { # fetch URL OUTPUT
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        die "curl or wget is required"
    fi
}

os=$(uname -s)
arch=$(uname -m)
case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "unsupported architecture: $arch" ;;
esac
case "$os" in
    Linux) target="$arch-unknown-linux-musl" ;;
    Darwin) target="macos-universal" ;;
    FreeBSD)
        [ "$arch" = x86_64 ] || die "unsupported architecture on FreeBSD: $arch"
        target="x86_64-unknown-freebsd"
        ;;
    *) die "unsupported OS: $os" ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM HUP

version="${TD_VERSION:-}"
if [ -z "$version" ]; then
    fetch "https://api.github.com/repos/$REPO/releases/latest" "$tmp/latest.json"
    version=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/latest.json" | head -n 1)
    [ -n "$version" ] || die "could not determine the latest version"
fi
tag="v${version#v}"
version="${tag#v}"

asset="td-$version-$target.tar.gz"
base="https://github.com/$REPO/releases/download/$tag"
echo "Downloading $asset"
fetch "$base/$asset" "$tmp/$asset"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"

expected=$(awk -v f="$asset" '{ n = $2; sub(/^\*/, "", n) } n == f { print $1; exit }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || die "$asset not listed in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$asset" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$tmp/$asset" | awk '{ print $1 }')
elif command -v sha256 >/dev/null 2>&1; then
    actual=$(sha256 -q "$tmp/$asset")
else
    die "no sha256 tool found (sha256sum, shasum or sha256)"
fi
[ "$expected" = "$actual" ] || die "checksum mismatch for $asset"

tar -xzf "$tmp/$asset" -C "$tmp" td
mkdir -p "$INSTALL_DIR"
if command -v install >/dev/null 2>&1; then
    install -m 755 "$tmp/td" "$INSTALL_DIR/td"
else
    cp "$tmp/td" "$INSTALL_DIR/td"
    chmod 755 "$INSTALL_DIR/td"
fi
echo "Installed td $version to $INSTALL_DIR/td"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "warning: $INSTALL_DIR is not on your PATH; add it, e.g. export PATH=\"$INSTALL_DIR:\$PATH\"" >&2 ;;
esac
