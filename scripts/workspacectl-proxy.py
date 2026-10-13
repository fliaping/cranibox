#!/usr/bin/env python3
"""Internal custom-domain routing implementation for workspacectl."""
import argparse
import ipaddress
import json
import os
import re
import shlex
import socket
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from functools import lru_cache


DEFAULT_STATE_DIR = "/config/proxyctl"
DEFAULT_CADDY_ADMIN = "http://127.0.0.1:2019"
DEFAULT_LISTEN = ":80"
DEFAULT_CODE_SERVER_TARGET = "127.0.0.1:8443"

HOST_RE = re.compile(r"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$")
LABEL_RE = re.compile(r"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$")


class ProxyctlError(Exception):
    pass


@lru_cache(maxsize=1)
def config_env():
    root = Path(os.environ.get("PROXYCTL_STATE_DIR", DEFAULT_STATE_DIR))
    path = root / "env"
    values = {}
    if not path.is_file():
        return values
    for raw_line in path.read_text().splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, raw_value = line.split("=", 1)
        try:
            parsed = shlex.split(raw_value, comments=True, posix=True)
        except ValueError as exc:
            raise ProxyctlError(f"invalid environment line in {path}: {raw_line}") from exc
        values[key.strip()] = parsed[0] if parsed else ""
    return values


def env(name, default=None):
    if name in os.environ:
        return os.environ[name]
    return config_env().get(name, default)


def state_dir():
    return Path(env("PROXYCTL_STATE_DIR", DEFAULT_STATE_DIR))


def routes_file():
    return state_dir() / "routes.json"


def backups_dir():
    return state_dir() / "backups"


def audit_file():
    return state_dir() / "audit.log"


def env_file():
    root = Path(os.environ.get("PROXYCTL_STATE_DIR", DEFAULT_STATE_DIR))
    return root / "env"


def write_env_values(values):
    path = env_file()
    try:
        lines = path.read_text().splitlines()
        mode = path.stat().st_mode & 0o777
    except FileNotFoundError:
        lines = []
        mode = 0o600
    pending = dict(values)
    updated = []
    for line in lines:
        stripped = line.strip()
        key = stripped.split("=", 1)[0].strip() if "=" in stripped else ""
        if key in pending and re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
            updated.append(f"{key}={shlex.quote(str(pending.pop(key)))}")
        else:
            updated.append(line)
    for key, value in pending.items():
        updated.append(f"{key}={shlex.quote(str(value))}")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text("\n".join(updated) + "\n")
    temporary.chmod(mode)
    temporary.replace(path)


def root_domain():
    domain = env("PROXY_ROOT_DOMAIN")
    if not domain:
        raise ProxyctlError("PROXY_ROOT_DOMAIN is required, for example: dev.example.com")
    domain = normalize_host(domain)
    if not valid_host(domain):
        raise ProxyctlError(f"invalid PROXY_ROOT_DOMAIN: {domain}")
    return domain


def caddy_admin():
    return env("CADDY_ADMIN", DEFAULT_CADDY_ADMIN).rstrip("/")


def listen_addr():
    return env("PROXY_LISTEN", DEFAULT_LISTEN)


def access_log_file():
    return env("PROXY_ACCESS_LOG", str(state_dir() / "access.log"))


def port_routing_mode():
    mode = env("PROXY_PORT_ROUTING", "code-server").strip().lower()
    if mode not in {"code-server", "direct"}:
        raise ProxyctlError("PROXY_PORT_ROUTING must be code-server or direct")
    return mode


def host_prefixes():
    raw = env("PROXY_HOST_PREFIXES")
    if raw is None:
        legacy = [env("PROXY_HOST_PREFIX", ""), env("PROXY_API_HOST_PREFIX", "")]
        prefixes = [item.strip().lower() for item in legacy if item.strip()]
    else:
        prefixes = [item.strip().lower() for item in raw.split(",") if item.strip()]
    seen = set()
    result = []
    for prefix in prefixes:
        if not LABEL_RE.match(prefix):
            raise ProxyctlError("PROXY_HOST_PREFIXES entries must be DNS label fragments")
        if prefix not in seen:
            result.append(prefix)
            seen.add(prefix)
    return result


def host_prefix():
    prefixes = host_prefixes()
    return prefixes[0] if prefixes else ""


def public_host(label, root):
    prefix = host_prefix()
    if prefix:
        return f"{prefix}-{label}.{root}"
    return f"{label}.{root}"


def code_server_host(root):
    label = env("CODE_SERVER_SUBDOMAIN", "code").strip().lower()
    if not LABEL_RE.match(label):
        raise ProxyctlError("CODE_SERVER_SUBDOMAIN must be a single DNS label")
    return public_host(label, root)


def code_server_target():
    return parse_target(env("CODE_SERVER_TARGET", DEFAULT_CODE_SERVER_TARGET))


def normalize_host(host):
    host = host.strip().lower().rstrip(".")
    if "://" in host or "/" in host or ":" in host:
        raise ProxyctlError("host must be a bare hostname, not a URL")
    return host


def valid_host(host):
    return len(host) <= 253 and HOST_RE.match(host) is not None


def normalize_managed_host(value, root):
    value = normalize_host(value)
    if "." not in value:
        value = public_host(value, root)
    if not valid_host(value):
        raise ProxyctlError(f"invalid host: {value}")
    if not (value == root or value.endswith(f".{root}")):
        raise ProxyctlError(f"host must be inside {root}: {value}")
    first_label = value[: -(len(root) + 1)] if value.endswith(f".{root}") else ""
    prefixes = host_prefixes()
    managed_label = first_label
    if prefixes:
        for prefix in prefixes:
            expected = f"{prefix}-"
            if first_label.startswith(expected):
                managed_label = first_label[len(expected):]
                break
        else:
            allowed = ", ".join(f"{prefix}-*" for prefix in prefixes)
            raise ProxyctlError(f"host must match one of: {allowed}")
    if managed_label.isdigit():
        raise ProxyctlError("numeric subdomains are reserved for code-server port proxying")
    if value == code_server_host(root):
        raise ProxyctlError(f"{value} is reserved for code-server")
    return value


def parse_target(raw):
    raw = raw.strip()
    if "://" in raw:
        try:
            url = urllib.parse.urlsplit(raw)
            port = url.port
        except ValueError as exc:
            raise ProxyctlError("invalid upstream URL") from exc
        if url.scheme not in {"http", "https"} or not url.hostname:
            raise ProxyctlError("upstream URL must use HTTP or HTTPS")
        if url.username is not None or url.password is not None or url.query or url.fragment or url.path not in {"", "/"}:
            raise ProxyctlError("upstream URL must be an origin without credentials, path, query or fragment")
        authority = normalize_upstream_host(url.netloc)
        if port is not None and not 1 <= port <= 65535:
            raise ProxyctlError("invalid upstream port")
        return f"{url.scheme}://{authority}"
    if raw.startswith("http://"):
        raw = raw[len("http://") :]
    elif raw.startswith("https://"):
        raise ProxyctlError("HTTPS upstreams are intentionally disabled for this lightweight HTTP-only proxy")
    if "/" in raw:
        raise ProxyctlError("target must be host:port without a path")
    if raw.count(":") != 1:
        raise ProxyctlError("target must be host:port")
    host, port_text = raw.rsplit(":", 1)
    if not host:
        raise ProxyctlError("target host is required")
    try:
        port = int(port_text)
    except ValueError as exc:
        raise ProxyctlError("target port must be an integer") from exc
    if port < 1 or port > 65535:
        raise ProxyctlError("target port must be between 1 and 65535")
    validate_target_host(host)
    return f"{host}:{port}"


def normalize_upstream_host(raw):
    raw = raw.strip()
    if not raw or any(ord(char) < 32 or char.isspace() for char in raw):
        raise ProxyctlError("upstream Host must be a non-empty hostname with no whitespace")
    if "://" in raw or "/" in raw:
        raise ProxyctlError("upstream Host must be a hostname, not a URL")
    host = raw
    port_text = None
    if raw.startswith("["):
        close = raw.find("]")
        if close < 0:
            raise ProxyctlError("invalid bracketed upstream Host")
        host = raw[1:close]
        suffix = raw[close + 1 :]
        if suffix:
            if not suffix.startswith(":"):
                raise ProxyctlError("invalid upstream Host suffix")
            port_text = suffix[1:]
        try:
            ipaddress.ip_address(host)
        except ValueError as exc:
            raise ProxyctlError("invalid upstream Host IP address") from exc
    elif raw.count(":") <= 1:
        if ":" in raw:
            host, port_text = raw.rsplit(":", 1)
        try:
            ipaddress.ip_address(host)
        except ValueError:
            if host != "localhost" and not valid_host(host):
                raise ProxyctlError("invalid upstream Host hostname")
    else:
        raise ProxyctlError("IPv6 upstream Host values must use bracket notation")
    if port_text is not None:
        try:
            port = int(port_text)
        except ValueError as exc:
            raise ProxyctlError("upstream Host port must be an integer") from exc
        if port < 1 or port > 65535:
            raise ProxyctlError("upstream Host port must be between 1 and 65535")
    return raw


def normalize_upstream_origin(raw):
    raw = raw.strip()
    if not raw or any(ord(char) < 32 or char.isspace() for char in raw):
        raise ProxyctlError("upstream Origin must be a non-empty URL with no whitespace")
    try:
        parsed = urllib.parse.urlsplit(raw)
    except ValueError as exc:
        raise ProxyctlError("invalid upstream Origin URL") from exc
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise ProxyctlError("upstream Origin must use http:// or https://")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ProxyctlError("upstream Origin must not contain credentials, query, or fragment")
    if parsed.path not in {"", "/"}:
        raise ProxyctlError("upstream Origin must not contain a path")
    normalize_upstream_host(parsed.netloc)
    return f"{parsed.scheme}://{parsed.netloc}"


def validate_target_host(host):
    allowed = [item.strip() for item in env("PROXY_ALLOWED_TARGETS", "127.0.0.0/8,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16").split(",") if item.strip()]
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        if env("PROXY_ALLOW_TARGET_HOSTNAMES", "0") == "1":
            if valid_host(host) or host == "localhost":
                return
        raise ProxyctlError("target host must be an allowed IP address unless PROXY_ALLOW_TARGET_HOSTNAMES=1")
    for cidr in allowed:
        if ip in ipaddress.ip_network(cidr, strict=False):
            return
    raise ProxyctlError(f"target host {host} is outside PROXY_ALLOWED_TARGETS")


def ensure_state():
    state_dir().mkdir(parents=True, exist_ok=True)
    backups_dir().mkdir(parents=True, exist_ok=True)
    if not routes_file().exists():
        save_routes([])


def load_routes():
    ensure_state()
    try:
        data = json.loads(routes_file().read_text())
    except json.JSONDecodeError as exc:
        raise ProxyctlError(f"invalid routes state: {routes_file()}") from exc
    if not isinstance(data, list):
        raise ProxyctlError("routes state must be a JSON array")
    return sorted(data, key=lambda item: item["host"])


def save_routes(routes):
    state_dir().mkdir(parents=True, exist_ok=True)
    temp = routes_file().with_suffix(".json.tmp")
    temp.write_text(json.dumps(sorted(routes, key=lambda item: item["host"]), indent=2) + "\n")
    temp.replace(routes_file())


def build_config(routes):
    root = root_domain()
    code_host = code_server_host(root)
    code_target = code_server_target()
    route_blocks = []

    for route in sorted(routes, key=lambda item: item["host"]):
        upstream_host = route.get("upstream_host")
        if upstream_host:
            upstream_host = normalize_upstream_host(upstream_host)
        upstream_origin = route.get("upstream_origin")
        if upstream_origin:
            upstream_origin = normalize_upstream_origin(upstream_origin)
        rewrite_client_ip = route.get("rewrite_client_ip", False)
        if not isinstance(rewrite_client_ip, bool):
            raise ProxyctlError("rewrite_client_ip must be a boolean")
        route_blocks.append(reverse_proxy_route(
            [route["host"]],
            route["target"],
            route_id=f"proxyctl:{route['host']}",
            upstream_host=upstream_host,
            upstream_origin=upstream_origin,
            rewrite_client_ip=rewrite_client_ip,
        ))

    route_blocks.append(reverse_proxy_route([code_host], code_target, route_id=f"proxyctl:{code_host}"))
    prefixes = host_prefixes()
    port_target = code_target if port_routing_mode() == "code-server" else None
    if prefixes:
        for index, prefix in enumerate(prefixes):
            route_blocks.append(header_regexp_reverse_proxy_route(
                "Host",
                f"port_{index}",
                rf"^{re.escape(prefix)}-([0-9]+)\.{re.escape(root)}(:[0-9]+)?$",
                port_target or f"127.0.0.1:{{http.regexp.port_{index}.1}}",
                route_id=f"proxyctl:ports:{prefix}-*.{root}",
            ))
    else:
        route_blocks.append(header_regexp_reverse_proxy_route(
            "Host",
            "port",
            rf"^([0-9]+)\.{re.escape(root)}(:[0-9]+)?$",
            port_target or "127.0.0.1:{http.regexp.port.1}",
            route_id=f"proxyctl:ports:*.{root}",
        ))
    route_blocks.append({
        "@id": "proxyctl:not-found",
        "handle": [{
            "handler": "static_response",
            "status_code": 404,
            "body": "not found\n",
        }],
    })

    return {
        "admin": {"listen": caddy_admin().removeprefix("http://")},
        "logging": {
            "logs": {
                "proxyctl-access": {
                    "writer": {
                        "output": "file",
                        "filename": access_log_file(),
                        "roll_size_mb": 10,
                        "roll_keep": 5,
                    },
                    "encoder": {"format": "json"},
                    "include": ["http.log.access.proxyctl-access"],
                }
            }
        },
        "apps": {
            "http": {
                "servers": {
                    "proxyctl": {
                        "listen": [listen_addr()],
                        "automatic_https": {"disable": True},
                        "logs": {"default_logger_name": "proxyctl-access"},
                        "routes": route_blocks,
                    }
                }
            }
        },
    }


def reverse_proxy_route(
    hosts,
    target,
    route_id,
    upstream_host=None,
    upstream_origin=None,
    rewrite_client_ip=False,
):
    request_headers = {"X-Forwarded-Proto": ["http"]}
    transport = None
    if "://" in target:
        url = urllib.parse.urlsplit(parse_target(target))
        hostname = url.hostname
        dial_host = f"[{hostname}]" if ":" in hostname else hostname
        target = f"{dial_host}:{url.port or (443 if url.scheme == 'https' else 80)}"
        request_headers["Host"] = [upstream_host or url.netloc]
        request_headers["X-Forwarded-Proto"] = [url.scheme]
        if url.scheme == "https":
            transport = {"protocol": "http", "tls": {"server_name": hostname}}
    if upstream_host:
        request_headers["Host"] = [upstream_host]
    if upstream_origin:
        request_headers["Origin"] = [upstream_origin]
    if rewrite_client_ip:
        request_headers["X-Forwarded-For"] = ["{http.request.remote.host}"]
        request_headers["X-Real-IP"] = ["{http.request.remote.host}"]
    result = {
        "@id": route_id,
        "match": [{"host": hosts}],
        "handle": [{
            "handler": "reverse_proxy",
            "upstreams": [{"dial": target}],
            "headers": {
                "request": {
                    "set": request_headers
                }
            },
        }],
    }
    if transport:
        result["handle"][0]["transport"] = transport
    return result


def header_regexp_reverse_proxy_route(header, name, pattern, target, route_id):
    return {
        "@id": route_id,
        "match": [{
            "header_regexp": {
                header: {
                    "name": name,
                    "pattern": pattern,
                }
            }
        }],
        "handle": [{
            "handler": "reverse_proxy",
            "upstreams": [{"dial": target}],
            "headers": {
                "request": {
                    "set": {
                        "X-Forwarded-Proto": ["http"],
                    }
                }
            },
        }],
    }


def http(method, path, data=None):
    body = None
    headers = {}
    if data is not None:
        body = json.dumps(data).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(f"{caddy_admin()}{path}", data=body, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=5) as res:
            payload = res.read()
            if not payload:
                return None
            return json.loads(payload.decode())
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode(errors="replace")
        raise ProxyctlError(f"Caddy API {method} {path} failed: HTTP {exc.code}: {detail}") from exc
    except urllib.error.URLError as exc:
        raise ProxyctlError(f"Caddy API is not reachable at {caddy_admin()}: {exc.reason}") from exc


def backup_current_config():
    try:
        current = http("GET", "/config/")
    except ProxyctlError:
        return None
    stamp = time.strftime("%Y%m%d-%H%M%S")
    path = backups_dir() / f"caddy-{stamp}.json"
    path.write_text(json.dumps(current, indent=2) + "\n")
    return path


def backup_routes(routes):
    ensure_state()
    stamp = time.strftime("%Y%m%d-%H%M%S")
    path = backups_dir() / f"routes-{stamp}.json"
    path.write_text(json.dumps(sorted(routes, key=lambda item: item["host"]), indent=2) + "\n")
    return path


def latest_route_backup():
    ensure_state()
    backups = sorted(backups_dir().glob("routes-*.json"))
    if not backups:
        raise ProxyctlError("no route backups found")
    return backups[-1]


def apply_config(routes, dry_run=False):
    config = build_config(routes)
    if dry_run:
        print(json.dumps(config, indent=2))
        return
    ensure_state()
    backup_current_config()
    http("POST", "/load", config)


def audit(action, message):
    ensure_state()
    audit_file().open("a").write(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {action} {message}\n")


def cmd_init(args):
    ensure_state()
    routes = load_routes()
    apply_config(routes, dry_run=args.dry_run)
    if not args.dry_run:
        audit("init", f"root={root_domain()} code={code_server_host(root_domain())} target={code_server_target()}")
        prefixes = ", ".join(host_prefixes()) if host_prefixes() else "<none>"
        print(f"initialized {listen_addr()} for {root_domain()} prefixes: {prefixes}")


def cmd_add(args):
    root = root_domain()
    host = normalize_managed_host(args.host, root)
    target = parse_target(args.target)
    existing_routes = load_routes()
    routes = [item for item in existing_routes if item["host"] != host]
    route = {"host": host, "target": target}
    if args.upstream_host:
        route["upstream_host"] = normalize_upstream_host(args.upstream_host)
    if args.upstream_origin:
        route["upstream_origin"] = normalize_upstream_origin(args.upstream_origin)
    if args.rewrite_client_ip:
        route["rewrite_client_ip"] = True
    routes.append(route)
    if not args.dry_run:
        backup_routes(existing_routes)
    apply_config(routes, dry_run=args.dry_run)
    if not args.dry_run:
        save_routes(routes)
        audit("add", f"{host} -> {target}")
        print(f"{host} -> {target}")


def cmd_remove(args):
    root = root_domain()
    host = normalize_managed_host(args.host, root)
    routes = load_routes()
    next_routes = [item for item in routes if item["host"] != host]
    if len(next_routes) == len(routes):
        raise ProxyctlError(f"route does not exist: {host}")
    if not args.dry_run:
        backup_routes(routes)
    apply_config(next_routes, dry_run=args.dry_run)
    if not args.dry_run:
        save_routes(next_routes)
        audit("remove", host)
        print(f"removed {host}")


def cmd_list(_args):
    root = root_domain()
    print(f"root: {root}")
    prefixes = host_prefixes()
    print(f"prefixes: {', '.join(prefixes) if prefixes else '<none>'}")
    print(f"code-server: {code_server_host(root)} -> {code_server_target()}")
    print(f"port routing: {port_routing_mode()}")
    if prefixes:
        for prefix in prefixes:
            target = "code-server" if port_routing_mode() == "code-server" else "127.0.0.1:<port>"
            print(f"port wildcard: {prefix}-<port>.{root} -> {target}")
    else:
        target = "code-server" if port_routing_mode() == "code-server" else "127.0.0.1:<port>"
        print(f"port wildcard: <port>.{root} -> {target}")
    routes = load_routes()
    if routes:
        print("routes:")
        for item in routes:
            details = []
            if item.get("upstream_host"):
                details.append(f"Host: {item['upstream_host']}")
            if item.get("upstream_origin"):
                details.append(f"Origin: {item['upstream_origin']}")
            if item.get("rewrite_client_ip"):
                details.append("client IP headers: rebuilt")
            suffix = f" ({', '.join(details)})" if details else ""
            print(f"  {item['host']} -> {item['target']}{suffix}")
    else:
        print("routes: none")


def cmd_check(_args):
    routes = load_routes()
    build_config(routes)
    for route in routes:
        parse_target(route["target"])
        normalize_managed_host(route["host"], root_domain())
        if route.get("upstream_host"):
            normalize_upstream_host(route["upstream_host"])
        if route.get("upstream_origin"):
            normalize_upstream_origin(route["upstream_origin"])
        if not isinstance(route.get("rewrite_client_ip", False), bool):
            raise ProxyctlError("rewrite_client_ip must be a boolean")
    if env("PROXYCTL_SKIP_CONNECT_CHECK", "0") != "1":
        failures = []
        for route in routes:
            try:
                check_tcp(route["target"])
            except OSError as exc:
                failures.append(f"{route['host']} -> {route['target']}: {exc}")
        code_host = code_server_host(root_domain())
        code_target = code_server_target()
        try:
            check_tcp(code_target)
        except OSError as exc:
            failures.append(f"{code_host} -> {code_target}: {exc}")
        if failures:
            detail = "\n  ".join(failures)
            raise ProxyctlError(f"unreachable upstreams:\n  {detail}")
    print("ok")


def cmd_rollback(args):
    path = Path(args.file) if args.file else latest_route_backup()
    try:
        routes = json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        raise ProxyctlError(f"invalid route backup: {path}") from exc
    if not isinstance(routes, list):
        raise ProxyctlError(f"route backup must be a JSON array: {path}")
    for route in routes:
        normalize_managed_host(route["host"], root_domain())
        parse_target(route["target"])
        if route.get("upstream_host"):
            normalize_upstream_host(route["upstream_host"])
        if route.get("upstream_origin"):
            normalize_upstream_origin(route["upstream_origin"])
        if not isinstance(route.get("rewrite_client_ip", False), bool):
            raise ProxyctlError("rewrite_client_ip must be a boolean")
    apply_config(routes, dry_run=args.dry_run)
    if not args.dry_run:
        backup_routes(load_routes())
        save_routes(routes)
        audit("rollback", str(path))
        print(f"rolled back routes from {path}")


def check_tcp(target):
    if "://" in target:
        url = urllib.parse.urlsplit(parse_target(target))
        host, port_text = url.hostname, url.port or (443 if url.scheme == "https" else 80)
    else:
        host, port_text = target.rsplit(":", 1)
    with socket.create_connection((host, int(port_text)), timeout=2):
        return


def cmd_render(_args):
    print(json.dumps(build_config(load_routes()), indent=2))


def cmd_domain(args):
    old_root = root_domain()
    new_root = normalize_host(args.domain)
    if not valid_host(new_root):
        raise ProxyctlError(f"invalid root domain: {new_root}")
    if new_root == old_root:
        print(f"root domain is already {new_root}")
        return

    routes = load_routes()
    migrated = []
    for route in routes:
        old_host = route["host"]
        if old_host == old_root:
            new_host = new_root
        elif old_host.endswith(f".{old_root}"):
            new_host = old_host[: -(len(old_root))] + new_root
        else:
            raise ProxyctlError(f"route is outside the current root domain: {old_host}")
        if not valid_host(new_host):
            raise ProxyctlError(f"migrated route is invalid: {new_host}")
        migrated.append({**route, "host": new_host})

    current_proxy_domain = env("CODE_SERVER_PROXY_DOMAIN", "") or ""
    if old_root in current_proxy_domain:
        next_proxy_domain = current_proxy_domain.replace(old_root, new_root)
    elif current_proxy_domain:
        next_proxy_domain = current_proxy_domain
    else:
        next_proxy_domain = f"{{{{port}}}}.{new_root}"

    original_env = env_file().read_bytes()
    if not args.dry_run:
        backup_routes(routes)
        stamp = time.strftime("%Y%m%d-%H%M%S")
        env_backup = backups_dir() / f"env-{stamp}"
        env_backup.write_bytes(original_env)
    try:
        write_env_values(
            {
                "PROXY_ROOT_DOMAIN": new_root,
                "CODE_SERVER_PROXY_DOMAIN": next_proxy_domain,
            }
        )
        config_env.cache_clear()
        apply_config(migrated, dry_run=args.dry_run)
    except Exception:
        env_file().write_bytes(original_env)
        config_env.cache_clear()
        if not args.dry_run:
            try:
                apply_config(routes)
            except Exception:
                pass
        raise
    if args.dry_run:
        env_file().write_bytes(original_env)
        config_env.cache_clear()
        return
    save_routes(migrated)
    audit("domain", f"{old_root} -> {new_root}")
    print(f"root domain: {old_root} -> {new_root}")
    print(f"code-server proxy domain: {next_proxy_domain}")


def build_parser():
    parser = argparse.ArgumentParser(
        prog="workspacectl proxy",
        description="Manage optional custom-domain routes through the internal Caddy backend.",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    init = sub.add_parser("init", help="initialize Caddy routes")
    init.add_argument("--dry-run", action="store_true")
    init.set_defaults(func=cmd_init)

    add = sub.add_parser("add", help="add or replace a named subdomain route")
    add.add_argument("host", help="subdomain label or full hostname")
    add.add_argument("target", help="upstream host:port or HTTP(S) origin, for example https://example.com")
    add.add_argument("--upstream-host", help="rewrite the upstream HTTP Host header")
    add.add_argument("--upstream-origin", help="rewrite the upstream HTTP Origin header")
    add.add_argument(
        "--rewrite-client-ip",
        action="store_true",
        help="overwrite X-Forwarded-For and X-Real-IP with the direct client address",
    )
    add.add_argument("--dry-run", action="store_true")
    add.set_defaults(func=cmd_add)

    remove = sub.add_parser("remove", help="remove a named subdomain route")
    remove.add_argument("host", help="subdomain label or full hostname")
    remove.add_argument("--dry-run", action="store_true")
    remove.set_defaults(func=cmd_remove)

    sub.add_parser("list", help="list managed routes").set_defaults(func=cmd_list)
    sub.add_parser("check", help="validate state and check upstream TCP connectivity").set_defaults(func=cmd_check)
    sub.add_parser("render", help="print generated Caddy JSON config").set_defaults(func=cmd_render)

    domain = sub.add_parser("domain", help="change the root domain and migrate managed routes")
    domain.add_argument("domain", help="new bare root domain")
    domain.add_argument("--dry-run", action="store_true")
    domain.set_defaults(func=cmd_domain)

    rollback = sub.add_parser("rollback", help="restore routes from the latest route backup or a specific file")
    rollback.add_argument("file", nargs="?")
    rollback.add_argument("--dry-run", action="store_true")
    rollback.set_defaults(func=cmd_rollback)
    return parser


def main():
    parser = build_parser()
    args = parser.parse_args()
    try:
        args.func(args)
    except (ProxyctlError, OSError) as exc:
        print(f"Routing backend: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
