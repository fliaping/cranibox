use crate::{common::*, inspect::text};
use anyhow::{Context as _, Result, bail, ensure};
use clap::{Parser, Subcommand};
use fs2::FileExt;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Parser)]
#[command(name = "workspacectl proxy", about = "Manage optional Caddy routes")]
pub struct Args {
    #[command(subcommand)]
    pub action: Action,
}
#[derive(Subcommand)]
pub enum Action {
    Init {
        #[arg(long)]
        dry_run: bool,
    },
    Add {
        host: String,
        target: String,
        #[arg(long)]
        upstream_host: Option<String>,
        #[arg(long)]
        upstream_origin: Option<String>,
        #[arg(long)]
        rewrite_client_ip: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Remove {
        host: String,
        #[arg(long)]
        dry_run: bool,
    },
    List,
    Check,
    Render,
    Domain {
        domain: String,
        #[arg(long)]
        dry_run: bool,
    },
    Rollback {
        file: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
}
#[derive(Clone)]
struct Config {
    state: PathBuf,
    vars: BTreeMap<String, String>,
}
impl Config {
    fn new(ctx: &Context) -> Self {
        let state = env::var_os("PROXYCTL_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.root.join("proxyctl"));
        let mut vars = env_file(&state.join("env"));
        vars.extend(env::vars());
        Self { state, vars }
    }
    fn get(&self, key: &str, default: &str) -> String {
        self.vars
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.into())
    }
    fn root(&self) -> Result<String> {
        let value = normalize_host(&self.get("PROXY_ROOT_DOMAIN", ""))?;
        ensure!(
            valid_host(&value),
            "PROXY_ROOT_DOMAIN must be a valid hostname"
        );
        Ok(value)
    }
    fn admin(&self) -> String {
        self.get("CADDY_ADMIN", "http://127.0.0.1:2019")
            .trim_end_matches('/')
            .into()
    }
    fn prefixes(&self) -> Result<Vec<String>> {
        let raw = self
            .vars
            .get("PROXY_HOST_PREFIXES")
            .cloned()
            .unwrap_or_else(|| {
                format!(
                    "{},{}",
                    self.get("PROXY_HOST_PREFIX", ""),
                    self.get("PROXY_API_HOST_PREFIX", "")
                )
            });
        let mut output = Vec::new();
        for prefix in raw
            .split(',')
            .map(|v| v.trim().to_lowercase())
            .filter(|v| !v.is_empty())
        {
            ensure!(
                valid_label(&prefix),
                "PROXY_HOST_PREFIXES entries must be DNS labels"
            );
            if !output.contains(&prefix) {
                output.push(prefix);
            }
        }
        Ok(output)
    }
    fn public_host(&self, label: &str, root: &str) -> Result<String> {
        Ok(if let Some(prefix) = self.prefixes()?.first() {
            format!("{prefix}-{label}.{root}")
        } else {
            format!("{label}.{root}")
        })
    }
    fn code_host(&self, root: &str) -> Result<String> {
        let label = self
            .get("CODE_SERVER_SUBDOMAIN", "code")
            .trim()
            .to_lowercase();
        ensure!(
            valid_label(&label),
            "CODE_SERVER_SUBDOMAIN must be a DNS label"
        );
        self.public_host(&label, root)
    }
    fn managed_host(&self, value: &str) -> Result<String> {
        let root = self.root()?;
        let value = normalize_host(value)?;
        let value = if value.contains('.') {
            value
        } else {
            self.public_host(&value, &root)?
        };
        ensure!(
            valid_host(&value) && (value == root || value.ends_with(&format!(".{root}"))),
            "host must be inside {root}"
        );
        let label = value.strip_suffix(&format!(".{root}")).unwrap_or_default();
        let prefixes = self.prefixes()?;
        let managed = if prefixes.is_empty() {
            label
        } else {
            prefixes
                .iter()
                .find_map(|prefix| label.strip_prefix(&format!("{prefix}-")))
                .context("host must match a configured prefix")?
        };
        ensure!(
            managed.is_empty() || !managed.bytes().all(|c| c.is_ascii_digit()),
            "numeric subdomains are reserved for port proxying"
        );
        ensure!(
            value != self.code_host(&root)?,
            "hostname is reserved for code-server"
        );
        Ok(value)
    }
    fn target(&self, raw: &str) -> Result<String> {
        let raw = raw.trim();
        if raw.contains("://") {
            let url = origin(raw)?;
            return Ok(url.as_str().trim_end_matches('/').into());
        }
        ensure!(
            raw.matches(':').count() == 1,
            "target must be host:port or an HTTP(S) origin"
        );
        let (host, port) = raw.rsplit_once(':').context("target must include a port")?;
        let port: u16 = port.parse()?;
        ensure!(port > 0, "target port must be between 1 and 65535");
        if let Ok(address) = host.parse::<IpAddr>() {
            let allowed = self.get(
                "PROXY_ALLOWED_TARGETS",
                "127.0.0.0/8,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16",
            );
            let ranges = allowed
                .split(',')
                .map(|value| value.trim().parse::<ipnet::IpNet>())
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ensure!(
                ranges.iter().any(|range| range.contains(&address)),
                "target host is outside PROXY_ALLOWED_TARGETS"
            );
        } else {
            ensure!(
                self.get("PROXY_ALLOW_TARGET_HOSTNAMES", "0") == "1"
                    && (valid_host(host) || host == "localhost"),
                "target must be an allowed IP address"
            );
        }
        Ok(format!("{host}:{port}"))
    }
    fn code_target(&self) -> Result<String> {
        self.target(&self.get("CODE_SERVER_TARGET", "127.0.0.1:8443"))
    }
    fn mode(&self) -> Result<String> {
        let mode = self
            .get("PROXY_PORT_ROUTING", "code-server")
            .trim()
            .to_lowercase();
        ensure!(
            ["code-server", "direct"].contains(&mode.as_str()),
            "PROXY_PORT_ROUTING must be code-server or direct"
        );
        Ok(mode)
    }
    fn routes(&self) -> Result<Vec<Value>> {
        load_routes(&self.state.join("routes.json"))
    }
    fn validate_route(&self, route: &Value) -> Result<()> {
        self.managed_host(text(&route["host"]))?;
        self.target(text(&route["target"]))?;
        for key in ["upstream_host", "upstream_origin"] {
            if let Some(value) = route.get(key) {
                let value = value.as_str().context("upstream header must be a string")?;
                if !value.is_empty() {
                    if key == "upstream_host" {
                        upstream_host(value)?;
                    } else {
                        origin(value)?;
                    }
                }
            }
        }
        ensure!(
            route.get("rewrite_client_ip").is_none_or(Value::is_boolean),
            "rewrite_client_ip must be a boolean"
        );
        Ok(())
    }
    fn build(&self, routes: &[Value]) -> Result<Value> {
        let root = self.root()?;
        let target = self.code_target()?;
        let code = self.code_host(&root)?;
        let mut sorted = routes.to_vec();
        sorted.sort_by(|a, b| text(&a["host"]).cmp(text(&b["host"])));
        let mut blocks = Vec::new();
        for route in &sorted {
            self.validate_route(route)?;
            blocks.push(reverse_proxy(
                text(&route["host"]),
                text(&route["target"]),
                route,
            )?);
        }
        blocks.push(reverse_proxy(&code, &target, &json!({}))?);
        let mode = self.mode()?;
        let prefixes = self.prefixes()?;
        let prefixes = if prefixes.is_empty() {
            vec![String::new()]
        } else {
            prefixes
        };
        for (index, prefix) in prefixes.iter().enumerate() {
            let name = if prefix.is_empty() {
                "port".into()
            } else {
                format!("port_{index}")
            };
            let pattern = format!(
                r"^{}([0-9]+)\.{}(:[0-9]+)?$",
                if prefix.is_empty() {
                    String::new()
                } else {
                    format!("{}-", regex::escape(prefix))
                },
                regex::escape(&root)
            );
            let dial = if mode == "code-server" {
                target.clone()
            } else {
                format!("127.0.0.1:{{http.regexp.{name}.1}}")
            };
            let id = format!(
                "proxyctl:ports:{}*.{root}",
                if prefix.is_empty() {
                    String::new()
                } else {
                    format!("{prefix}-")
                }
            );
            blocks.push(json!({"@id":id,"match":[{"header_regexp":{"Host":{"name":name,"pattern":pattern}}}],"handle":[{"handler":"reverse_proxy","upstreams":[{"dial":dial}],"headers":{"request":{"set":{"X-Forwarded-Proto":["http"]}}}}]}));
        }
        blocks.push(json!({"@id":"proxyctl:not-found","handle":[{"handler":"static_response","status_code":404,"body":"not found\n"}]}));
        Ok(
            json!({"admin":{"listen":self.admin().trim_start_matches("http://")},"logging":{"logs":{"proxyctl-access":{"writer":{"output":"file","filename":self.get("PROXY_ACCESS_LOG",&self.state.join("access.log").to_string_lossy()),"roll_size_mb":10,"roll_keep":5},"encoder":{"format":"json"},"include":["http.log.access.proxyctl-access"]}}},"apps":{"http":{"servers":{"proxyctl":{"listen":[self.get("PROXY_LISTEN",":80")],"automatic_https":{"disable":true},"logs":{"default_logger_name":"proxyctl-access"},"routes":blocks}}}}}),
        )
    }
    async fn apply(&self, ctx: &Context, routes: &[Value], dry_run: bool) -> Result<()> {
        let config = self.build(routes)?;
        if dry_run {
            print_json(&config);
            return Ok(());
        }
        if let Ok(current) = ctx.get_json(&format!("{}/config/", self.admin()), 5.).await {
            write_json(&self.backup_path("caddy", "json"), &current)?;
        }
        ctx.http
            .post(format!("{}/load", self.admin()))
            .json(&config)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    fn backup_path(&self, name: &str, extension: &str) -> PathBuf {
        self.state.join("backups").join(format!(
            "{name}-{}.{}.{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S-%f"),
            std::process::id(),
            extension
        ))
    }
    fn backup_routes(&self, routes: &[Value]) -> Result<()> {
        write_json(&self.backup_path("routes", "json"), &json!(routes))
    }
    fn save(&self, routes: &[Value], action: &str) -> Result<()> {
        let mut routes = routes.to_vec();
        routes.sort_by(|a, b| text(&a["host"]).cmp(text(&b["host"])));
        write_json(&self.state.join("routes.json"), &json!(routes))?;
        let mut audit = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.state.join("audit.log"))?;
        writeln!(audit, "{} {action}", chrono::Utc::now().to_rfc3339())?;
        Ok(())
    }
}
pub fn normalize_host(value: &str) -> Result<String> {
    let value = value.trim().to_lowercase().trim_end_matches('.').to_owned();
    ensure!(!value.contains(['/', ':']), "host must be a bare hostname");
    Ok(value)
}
fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}
pub fn valid_host(value: &str) -> bool {
    value.len() <= 253 && value.contains('.') && value.split('.').all(valid_label)
}
fn origin(value: &str) -> Result<reqwest::Url> {
    ensure!(
        !value.chars().any(|c| c.is_whitespace() || c.is_control()),
        "upstream origin contains whitespace or control characters"
    );
    let url = reqwest::Url::parse(value)?;
    ensure!(
        ["http", "https"].contains(&url.scheme())
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/"
            && url.port() != Some(0),
        "upstream URL must be an HTTP(S) origin without credentials, path, query or fragment"
    );
    let host = url.host_str().unwrap().trim_matches(['[', ']']);
    ensure!(
        host.parse::<IpAddr>().is_ok() || valid_host(host) || host == "localhost",
        "invalid upstream host"
    );
    Ok(url)
}
fn upstream_host(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        !value.contains(['/', '?', '#', '@']),
        "invalid upstream Host header"
    );
    let _ = origin(&format!("http://{value}"))?;
    Ok(value.into())
}
fn reverse_proxy(host: &str, target: &str, route: &Value) -> Result<Value> {
    let mut headers = json!({"X-Forwarded-Proto":["http"]});
    let mut transport = None;
    let dial = if target.contains("://") {
        let url = origin(target)?;
        let hostname = url.host_str().unwrap();
        let authority = if let Some(port) = url.port() {
            format!("{hostname}:{port}")
        } else {
            hostname.into()
        };
        headers["Host"] = json!([authority]);
        headers["X-Forwarded-Proto"] = json!([url.scheme()]);
        if url.scheme() == "https" {
            transport = Some(
                json!({"protocol":"http","tls":{"server_name":hostname.trim_matches(['[',']'])}}),
            );
        }
        format!("{hostname}:{}", url.port_or_known_default().unwrap())
    } else {
        target.into()
    };
    if let Some(value) = route["upstream_host"].as_str().filter(|v| !v.is_empty()) {
        headers["Host"] = json!([upstream_host(value)?]);
    }
    if let Some(value) = route["upstream_origin"].as_str().filter(|v| !v.is_empty()) {
        origin(value)?;
        headers["Origin"] = json!([value.trim_end_matches('/')]);
    }
    if route["rewrite_client_ip"] == true {
        headers["X-Forwarded-For"] = json!(["{http.request.remote.host}"]);
        headers["X-Real-IP"] = json!(["{http.request.remote.host}"]);
    }
    let mut value = json!({"@id":format!("proxyctl:{host}"),"match":[{"host":[host]}],"handle":[{"handler":"reverse_proxy","upstreams":[{"dial":dial}],"headers":{"request":{"set":headers}}}]});
    if let Some(transport) = transport {
        value["handle"][0]["transport"] = transport;
    }
    Ok(value)
}
fn load_routes(path: &Path) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let mut routes = value
        .as_array()
        .context("routes state must be a JSON array")?
        .clone();
    routes.sort_by(|a, b| text(&a["host"]).cmp(text(&b["host"])));
    Ok(routes)
}
async fn change_domain(ctx: &Context, config: Config, domain: &str, dry_run: bool) -> Result<i32> {
    let old = config.root()?;
    let new = normalize_host(domain)?;
    ensure!(valid_host(&new), "invalid root domain");
    if new == old {
        println!("root domain is already {new}");
        return Ok(0);
    }
    let routes = config.routes()?;
    let mut migrated = routes.clone();
    for route in &mut migrated {
        let host = text(&route["host"]);
        let next = if host == old {
            new.clone()
        } else {
            format!(
                "{}.{new}",
                host.strip_suffix(&format!(".{old}"))
                    .context("route is outside the current root domain")?
            )
        };
        route["host"] = json!(next);
    }
    let old_proxy = config.get("CODE_SERVER_PROXY_DOMAIN", "");
    let new_proxy = if old_proxy.is_empty() {
        format!("{{{{port}}}}.{new}")
    } else {
        old_proxy.replace(&old, &new)
    };
    let mut candidate = config.clone();
    candidate
        .vars
        .insert("PROXY_ROOT_DOMAIN".into(), new.clone());
    candidate
        .vars
        .insert("CODE_SERVER_PROXY_DOMAIN".into(), new_proxy.clone());
    if dry_run {
        candidate.apply(ctx, &migrated, true).await?;
        return Ok(0);
    }
    let path = config.state.join("env");
    let original = fs::read(&path)?;
    config.backup_routes(&routes)?;
    atomic_write(&config.backup_path("env", "bak"), &original, 0o600)?;
    candidate.apply(ctx, &migrated, false).await?;
    let persist = (|| -> Result<()> {
        let mut lines = read(&path)
            .lines()
            .filter(|line| {
                !["PROXY_ROOT_DOMAIN", "CODE_SERVER_PROXY_DOMAIN"]
                    .contains(&line.trim().split('=').next().unwrap_or_default().trim())
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        lines.extend([
            format!("PROXY_ROOT_DOMAIN={}", shell_words::quote(&new)),
            format!(
                "CODE_SERVER_PROXY_DOMAIN={}",
                shell_words::quote(&new_proxy)
            ),
        ]);
        atomic_write(&path, format!("{}\n", lines.join("\n")), mode(&path, 0o600))?;
        candidate.save(&migrated, "domain")
    })();
    if let Err(error) = persist {
        atomic_write(&path, original, mode(&path, 0o600))?;
        write_json(&config.state.join("routes.json"), &json!(routes))?;
        if let Err(recovery) = config.apply(ctx, &routes, false).await {
            eprintln!("Routing recovery failed: {recovery}");
        }
        return Err(error);
    }
    println!("root domain: {old} -> {new}\ncode-server proxy domain: {new_proxy}");
    Ok(0)
}
pub async fn run(ctx: &Context, args: Args) -> Result<i32> {
    let config = Config::new(ctx);
    if !config.state.join("env").exists() && !config.state.join("bin/caddy").exists() {
        if matches!(args.action, Action::Check | Action::List) {
            println!("Custom-domain routing: not enabled (optional)");
            return Ok(0);
        }
        bail!("Custom-domain routing is not installed; run workspacectl network domain <domain>");
    }
    // Serialize mutations so simultaneous control-center actions cannot overwrite each other.
    let read_only = matches!(
        args.action,
        Action::List
            | Action::Check
            | Action::Render
            | Action::Init { dry_run: true }
            | Action::Add { dry_run: true, .. }
            | Action::Remove { dry_run: true, .. }
            | Action::Domain { dry_run: true, .. }
            | Action::Rollback { dry_run: true, .. }
    );
    let _lock = if read_only {
        None
    } else {
        fs::create_dir_all(&config.state)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(config.state.join("control.lock"))?;
        file.try_lock_exclusive()
            .context("Another routing operation is running")?;
        Some(file)
    };
    let old = config.routes()?;
    let (next, dry_run, action) = match args.action {
        Action::Render => {
            print_json(&config.build(&old)?);
            return Ok(0);
        }
        Action::List => {
            println!(
                "root: {}\ncode-server: {} -> {}\nport routing: {}",
                config.root()?,
                config.code_host(&config.root()?)?,
                config.code_target()?,
                config.mode()?
            );
            for route in old {
                println!("  {} -> {}", text(&route["host"]), text(&route["target"]));
            }
            return Ok(0);
        }
        Action::Check => {
            config.build(&old)?;
            if config.get("PROXYCTL_SKIP_CONNECT_CHECK", "0") != "1" {
                let mut targets = old
                    .iter()
                    .map(|row| text(&row["target"]).to_owned())
                    .collect::<Vec<_>>();
                targets.push(config.code_target()?);
                for target in targets {
                    let (host, port) = if target.contains("://") {
                        let url = origin(&target)?;
                        (
                            url.host_str().unwrap().trim_matches(['[', ']']).to_owned(),
                            url.port_or_known_default().unwrap(),
                        )
                    } else {
                        let (host, port) = target.rsplit_once(':').unwrap();
                        (host.into(), port.parse::<u16>()?)
                    };
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        tokio::net::TcpStream::connect((host.as_str(), port)),
                    )
                    .await
                    .with_context(|| format!("unreachable upstream: {target}"))??;
                }
            }
            println!("ok");
            return Ok(0);
        }
        Action::Domain { domain, dry_run } => {
            return change_domain(ctx, config, &domain, dry_run).await;
        }
        Action::Init { dry_run } => (old.clone(), dry_run, "init"),
        Action::Add {
            host,
            target,
            upstream_host,
            upstream_origin,
            rewrite_client_ip,
            dry_run,
        } => {
            let host = config.managed_host(&host)?;
            let target = config.target(&target)?;
            let mut next = old
                .iter()
                .filter(|row| row["host"] != host)
                .cloned()
                .collect::<Vec<_>>();
            let mut row = json!({"host":host,"target":target});
            if let Some(value) = upstream_host {
                row["upstream_host"] = json!(self::upstream_host(&value)?);
            }
            if let Some(value) = upstream_origin {
                origin(&value)?;
                row["upstream_origin"] = json!(value.trim_end_matches('/'));
            }
            if rewrite_client_ip {
                row["rewrite_client_ip"] = json!(true);
            }
            next.push(row);
            (next, dry_run, "add")
        }
        Action::Remove { host, dry_run } => {
            let host = config.managed_host(&host)?;
            let next = old
                .iter()
                .filter(|row| row["host"] != host)
                .cloned()
                .collect::<Vec<_>>();
            ensure!(next.len() != old.len(), "route does not exist");
            (next, dry_run, "remove")
        }
        Action::Rollback { file, dry_run } => {
            let path = file
                .or_else(|| {
                    directories(config.state.join("backups"))
                        .into_iter()
                        .rfind(|path| {
                            path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with("routes-")
                                    && name.to_string_lossy().ends_with(".json")
                            })
                        })
                })
                .context("no route backups found")?;
            (load_routes(&path)?, dry_run, "rollback")
        }
    };
    config.build(&next)?;
    if !dry_run {
        config.backup_routes(&old)?;
    }
    config.apply(ctx, &next, dry_run).await?;
    if !dry_run {
        if let Err(error) = config.save(&next, action) {
            // Reload the previous routes if local persistence fails after Caddy accepts the change.
            write_json(&config.state.join("routes.json"), &json!(old))?;
            if let Err(recovery) = config.apply(ctx, &old, false).await {
                eprintln!("Routing recovery failed: {recovery}");
            }
            return Err(error);
        }
        println!("Routing {action} completed");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_origins() {
        for value in [
            "https://user:secret@example.com",
            "https://example.com/path",
            "https://example.com?x=1",
            "http://example.com:0",
            "https://example.com\r\nX:test",
        ] {
            assert!(origin(value).is_err(), "{value}");
        }
        assert!(origin("https://example.com").is_ok());
        assert!(origin("http://[::1]:9000").is_ok());
    }
    #[test]
    fn https_upstream_has_tls_and_headers() {
        let value = reverse_proxy(
            "api.example.com",
            "https://upstream.example.org",
            &json!({"rewrite_client_ip":true}),
        )
        .unwrap();
        let handler = &value["handle"][0];
        assert_eq!(handler["upstreams"][0]["dial"], "upstream.example.org:443");
        assert_eq!(
            handler["transport"]["tls"]["server_name"],
            "upstream.example.org"
        );
        assert_eq!(
            handler["headers"]["request"]["set"]["Host"][0],
            "upstream.example.org"
        );
    }
    #[test]
    fn reserved_and_private_targets() {
        let config = Config {
            state: PathBuf::from("/tmp/proxyctl"),
            vars: BTreeMap::from([("PROXY_ROOT_DOMAIN".into(), "workspace.example.com".into())]),
        };
        assert!(config.managed_host("8443").is_err());
        assert!(config.managed_host("code").is_err());
        assert!(config.target("8.8.8.8:80").is_err());
        assert_eq!(config.target("127.0.0.1:9000").unwrap(), "127.0.0.1:9000");
        assert!(
            config
                .build(&[json!({"host":"outside.example.com","target":"127.0.0.1:80"})])
                .is_err()
        );
    }
}
