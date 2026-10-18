#!/bin/sh
# Fetch without elevation; only Apple's installer receives administrator access.
set -eu
export PATH=/usr/bin:/bin:/usr/sbin:/sbin
export HOMEBREW_NO_AUTO_UPDATE=1
[ "$(uname -s)" = Darwin ] || exit 1
[ ! -d /Applications/Tailscale.app ] || exit 1
brew=$1
[ -x "$brew" ] || exit 1
"$brew" fetch --cask tailscale-app
pkg=$("$brew" --cache --cask tailscale-app)
[ -f "$pkg" ] || exit 1
/usr/sbin/pkgutil --check-signature "$pkg"
[ ! -d /Applications/Tailscale.app ] || exit 1
/usr/bin/osascript - "$pkg" <<'APPLESCRIPT'
on run argv
    set packagePath to item 1 of argv
    do shell script "/usr/sbin/installer -pkg " & quoted form of packagePath & " -target /" with administrator privileges
end run
APPLESCRIPT
/usr/bin/open -a Tailscale
