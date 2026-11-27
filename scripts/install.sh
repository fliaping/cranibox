#!/bin/sh
# Install the Cranibox command line from a GitHub Release, checked against SHA256SUMS.
#
#   curl -fsSL https://raw.githubusercontent.com/fliaping/cranibox/main/scripts/install.sh | sh
#
# Linux x86_64/arm64 and macOS arm64/x86_64. No root needed: without root it installs to
# ~/.local/bin, as root to /usr/local/bin. Settings (environment variables):
#   CRANIBOX_VERSION        release tag or version to install (default: the latest release)
#   CRANIBOX_INSTALL_DIR    where to put `cranibox`
#   CRANIBOX_DOWNLOAD_BASE  a mirror holding the release files (<base>/<file>)
#   CRANIBOX_REPO           GitHub repository (default fliaping/cranibox)
# Afterwards, on a machine without a service manager: `cranibox serve --daemon`.
set -eu

REPO="${CRANIBOX_REPO:-fliaping/cranibox}"

say() { printf '%s\n' "$*" >&2; }
fail() { say "cranibox install: $*"; exit 1; }

# The release file for this machine, e.g. cranibox-linux-x86_64.
asset_name() {
  os=$1
  arch=$2
  case "$os" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) fail "unsupported system $os (Linux and macOS only; Windows: see the README)" ;;
  esac
  case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) fail "unsupported processor $arch (x86_64 and arm64 only)" ;;
  esac
  printf 'cranibox-%s-%s\n' "$os" "$arch"
}

# Release tag for a version: v1.2.1, 1.2.1 -> v1.2.1.
release_tag() {
  case "$1" in
    v*) printf '%s\n' "$1" ;;
    *) printf 'v%s\n' "$1" ;;
  esac
}

# Where the release files are.
download_base() {
  if [ -n "${CRANIBOX_DOWNLOAD_BASE:-}" ]; then
    printf '%s\n' "${CRANIBOX_DOWNLOAD_BASE%/}"
  elif [ -n "${CRANIBOX_VERSION:-}" ]; then
    printf 'https://github.com/%s/releases/download/%s\n' "$REPO" "$(release_tag "$CRANIBOX_VERSION")"
  else
    printf 'https://github.com/%s/releases/latest/download\n' "$REPO"
  fi
}

# The checksum of FILE in a SHA256SUMS file ("<hex>  <name>" or "<hex> *<name>").
expected_sum() {
  sums=$1
  file=$2
  awk -v f="$file" '{ name = $2; sub(/^\*/, "", name); if (name == f) { print tolower($1); found = 1; exit } } END { if (!found) exit 1 }' "$sums"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print tolower($1) }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{ print tolower($1) }'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 -r "$1" | awk '{ print tolower($1) }'
  else
    fail "need sha256sum, shasum or openssl to verify the download"
  fi
}

# Verify FILE (named NAME in the release) against SUMS.
verify() {
  file=$1
  name=$2
  sums=$3
  want=$(expected_sum "$sums" "$name") || fail "$name is not listed in SHA256SUMS"
  case "$want" in
    *[!0-9a-f]* | "") fail "SHA256SUMS has an invalid checksum for $name" ;;
  esac
  [ "${#want}" -eq 64 ] || fail "SHA256SUMS has an invalid checksum for $name"
  got=$(sha256_of "$file")
  [ "$got" = "$want" ] || fail "checksum mismatch for $name (expected $want, got $got); not installing"
}

default_install_dir() {
  if [ -n "${CRANIBOX_INSTALL_DIR:-}" ]; then
    printf '%s\n' "$CRANIBOX_INSTALL_DIR"
  elif [ "$(id -u)" = 0 ]; then
    printf '/usr/local/bin\n'
  else
    [ -n "${HOME:-}" ] || fail "HOME is not set; set CRANIBOX_INSTALL_DIR"
    printf '%s/.local/bin\n' "$HOME"
  fi
}

fetch() {
  url=$1
  out=$2
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --proto '=https,file' -o "$out" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$out" "$url"
  else
    fail "need curl or wget"
  fi
}

on_path() {
  case ":${PATH:-}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

main() {
  name=$(asset_name "${CRANIBOX_INSTALL_OS:-$(uname -s)}" "${CRANIBOX_INSTALL_ARCH:-$(uname -m)}")
  base=$(download_base)
  dir=$(default_install_dir)
  tmp=$(mktemp -d 2>/dev/null || mktemp -d -t cranibox)
  trap 'rm -rf "$tmp"' EXIT INT TERM
  say "Downloading $name from $base"
  fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "could not download $base/SHA256SUMS"
  fetch "$base/$name" "$tmp/$name" || fail "could not download $base/$name (is there a $name build in this release?)"
  verify "$tmp/$name" "$name" "$tmp/SHA256SUMS"
  mkdir -p "$dir" || fail "cannot create $dir; set CRANIBOX_INSTALL_DIR"
  chmod 0755 "$tmp/$name"
  # Replace atomically so a running copy keeps working.
  cp "$tmp/$name" "$dir/.cranibox.new" || fail "cannot write to $dir; set CRANIBOX_INSTALL_DIR"
  mv -f "$dir/.cranibox.new" "$dir/cranibox"
  say "Installed $("$dir/cranibox" --version 2>/dev/null || echo cranibox) to $dir/cranibox"
  if ! on_path "$dir"; then
    say ""
    say "$dir is not on your PATH. Add it, e.g.:"
    say "  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile && . ~/.profile"
  fi
  say ""
  say "Start this device in the background (no systemd needed):"
  say "  cranibox serve --daemon      # then: cranibox serve status | cranibox serve stop"
}

# Sourcing with CRANIBOX_INSTALL_LIB=1 loads the functions only (tests).
if [ "${CRANIBOX_INSTALL_LIB:-}" != 1 ]; then
  main "$@"
fi
