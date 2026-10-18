#!/bin/sh
# Maintained from Docker's official apt installation instructions. No curl | sh.
set -eu
test "$(id -u)" = 0 || { echo 'Administrator permission required' >&2; exit 1; }
. /etc/os-release
case "$ID" in ubuntu|debian) ;; *) echo 'Automatic installation currently supports Ubuntu and Debian. Install Docker Engine for your distribution, then recheck.' >&2; exit 1 ;; esac
# Do not replace another installation or remove conflicting user packages automatically.
if command -v docker >/dev/null 2>&1; then
    echo 'Docker CLI already exists. Start the installed runtime or resolve its access permissions, then recheck.' >&2
    exit 1
fi
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y ca-certificates curl
install -m 0755 -d /etc/apt/keyrings
curl -fsSL "https://download.docker.com/linux/$ID/gpg" -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
architecture=$(dpkg --print-architecture)
codename=${UBUNTU_CODENAME:-$VERSION_CODENAME}
printf 'Types: deb\nURIs: https://download.docker.com/linux/%s\nSuites: %s\nComponents: stable\nArchitectures: %s\nSigned-By: /etc/apt/keyrings/docker.asc\n' "$ID" "$codename" "$architecture" > /etc/apt/sources.list.d/docker.sources
apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
if command -v systemctl >/dev/null 2>&1 && test -d /run/systemd/system; then
    systemctl enable --now docker
else
    service docker start
fi
if test -n "${1:-}" && test "$1" != root; then
    usermod -aG docker "$1"
    echo 'Sign out and back in to activate Docker group membership.'
elif test -n "${WSL_DISTRO_NAME:-}"; then
    # Ubuntu's normal WSL user is the first UID 1000 account; root remains root.
    wsl_user=$(getent passwd 1000 | cut -d: -f1)
    if test -n "$wsl_user"; then usermod -aG docker "$wsl_user"; fi
fi
