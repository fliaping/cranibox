#!/usr/bin/env bash
# Image build step: put the desktop switch in front of every Selkies dashboard
# and route its control API through the nginx site that serves the page.
set -euo pipefail
source_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
patched=0
selkies_root="${SELKIES_ROOT:-/usr/share/selkies}"
nginx_conf="${NGINX_DEFAULT_CONF:-/defaults/default.conf}"
for dashboard in "$selkies_root"/*/; do
  index="${dashboard}index.html"
  [[ -f "$index" && "$dashboard" != */web/ ]] || continue
  install -m 0644 "$source_dir/cranibox-desktop.js" "${dashboard}cranibox-desktop.js"
  python3 - "$index" <<'PY'
import re, sys
path = sys.argv[1]
html = open(path, encoding="utf-8").read()
if "cranibox-desktop.js" in html:
    sys.exit(0)
pattern = re.compile(r'<script\s+type="module"\s+crossorigin\s+src="([^"]+)"\s*></script>')
match = pattern.search(html)
if not match:
    sys.exit(f"no module entry script in {path}")
loader = f'<script src="cranibox-desktop.js" data-entry="{match.group(1)}"></script>'
open(path, "w", encoding="utf-8").write(html[:match.start()] + loader + html[match.end():])
PY
  patched=$((patched + 1))
done
(( patched > 0 )) || { echo "no Selkies dashboard found" >&2; exit 1; }

# One location per nginx server block (HTTP 3000 and HTTPS 3001).
python3 - "$nginx_conf" <<'PY'
import sys
path = sys.argv[1]
conf = open(path, encoding="utf-8").read()
if "cranibox-desktop/" not in conf:
    block = (
        "  location ^~ SUBFOLDERcranibox-desktop/ {\n"
        "    proxy_pass              http://127.0.0.1:3079/;\n"
        "    proxy_http_version      1.1;\n"
        "    proxy_set_header        Host $host;\n"
        "    proxy_read_timeout      30s;\n"
        "    proxy_buffering         off;\n"
        "    client_max_body_size    4k;\n"
        "  }\n"
    )
    marker = "  error_page 500 502 503 504"
    count = conf.count(marker)
    if count < 1:
        sys.exit("nginx default.conf layout changed")
    conf = conf.replace(marker, block + marker)
    open(path, "w", encoding="utf-8").write(conf)
PY
# XFCE: xfsettingsd treats the stream's resizable output as a newly connected
# monitor and opens "xfce4-display-settings -m" or re-applies a saved display
# profile; either can switch the only output off (a black desktop on connect).
# Defaults only: a user's own displays settings still win.
xfconf_dir="${XFCONF_SYSTEM_DIR:-/etc/xdg/xfce4/xfconf/xfce-perchannel-xml}"
if [[ -d "$xfconf_dir" ]]; then
  cat > "$xfconf_dir/displays.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>

<channel name="displays" version="1.0">
  <property name="Notify" type="int" value="0"/>
  <property name="AutoEnableProfiles" type="bool" value="false"/>
</channel>
XML
  chmod 0644 "$xfconf_dir/displays.xml"
fi
echo "[desktop-power] patched $patched dashboard(s) and nginx"
