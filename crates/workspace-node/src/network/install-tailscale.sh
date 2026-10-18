#!/bin/sh
# Fixed stable-channel setup. Executed only after reviewing the node's plan.
set -eu
export DEBIAN_FRONTEND=noninteractive
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
[ "$(id -u)" = 0 ] || { echo 'Administrator access required' >&2; exit 1; }
[ ! -e /.dockerenv ] && [ ! -e /run/.containerenv ] || exit 1
[ -d /run/systemd/system ] || exit 1
. /etc/os-release
case "${ID:-}" in ubuntu|debian) ;; *) exit 1 ;; esac
case "${VERSION_CODENAME:-}" in ''|*[!a-z0-9]*) exit 1 ;; esac
apt-get update
apt-get install -y --no-install-recommends ca-certificates curl
setup_dir=$(mktemp -d)
trap 'rm -rf "$setup_dir"' EXIT HUP INT TERM
base="https://pkgs.tailscale.com/stable/$ID/$VERSION_CODENAME"
curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --max-time 60 --max-filesize 1048576 "$base.noarmor.gpg" -o "$setup_dir/key.gpg"
curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --max-time 60 --max-filesize 1048576 "$base.tailscale-keyring.list" -o "$setup_dir/tailscale.list"
install -m 0644 "$setup_dir/key.gpg" /usr/share/keyrings/tailscale-archive-keyring.gpg
install -m 0644 "$setup_dir/tailscale.list" /etc/apt/sources.list.d/tailscale.list
apt-get update
apt-get install -y tailscale
systemctl enable --now tailscaled
