use crate::{common::*, inspect};
use anyhow::{Context as _, Result, bail, ensure};
use chrono::Offset;
use clap::Parser;
use md5::{Digest, Md5};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub fn update_yaml(path: &Path, key: &str, value: &str) -> Result<()> {
    let original = read(path);
    let pattern = regex::Regex::new(&format!(r"(?m)^[ \t]*{}[ \t]*:.*$", regex::escape(key)))?;
    let updated = if pattern.is_match(&original) {
        pattern
            .replace(&original, regex::NoExpand(&format!("{key}: {value}")))
            .into_owned()
    } else {
        format!(
            "{}{key}: {value}\n",
            if original.is_empty() || original.ends_with('\n') {
                original
            } else {
                original + "\n"
            }
        )
    };
    atomic_write(path, updated, mode(path, 0o600))
}
pub fn register_language_pack(ctx: &Context) -> Result<()> {
    let extension_root = ctx.root.join(".local/share/code-server/extensions");
    let installed = json_file(extension_root.join("extensions.json"));
    let id = "ms-ceintl.vscode-language-pack-zh-hans";
    let entry = installed.as_array().and_then(|entries| entries.iter().find(|entry| inspect::text(&entry["identifier"]["id"]).eq_ignore_ascii_case(id))).context("Simplified Chinese language pack is missing; run: agent-workspace-manager install code-server-extensions")?;
    let extension = PathBuf::from(
        entry["location"]["fsPath"]
            .as_str()
            .or_else(|| entry["location"]["path"].as_str())
            .context("language pack location is missing")?,
    );
    let manifest = json_file(extension.join("package.json"));
    let localization = manifest["contributes"]["localizations"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["languageId"] == "zh-cn"))
        .context("Simplified Chinese translations are unavailable")?;
    let uuid = entry["identifier"]["uuid"]
        .as_str()
        .or_else(|| entry["metadata"]["id"].as_str());
    let version = manifest["version"]
        .as_str()
        .or_else(|| entry["version"].as_str())
        .unwrap_or_default();
    let mut identifier = json!({"id":id});
    if let Some(uuid) = uuid {
        identifier["uuid"] = json!(uuid);
    }
    let mut translations = serde_json::Map::new();
    for translation in localization["translations"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let name = inspect::text(&translation["id"]);
        let path = inspect::text(&translation["path"]);
        if !name.is_empty() && !path.is_empty() {
            let joined = extension.join(path);
            translations.insert(
                name.into(),
                json!(fs::canonicalize(&joined).unwrap_or(joined)),
            );
        }
    }
    let mut hash = Md5::new();
    hash.update(uuid.unwrap_or(id));
    hash.update(version);
    let pack = json!({"hash":format!("{:x}",hash.finalize()),"extensions":[{"extensionIdentifier":identifier,"version":version}],"translations":translations,"label":localization["localizedLanguageName"].as_str().or_else(|| localization["languageName"].as_str()).unwrap_or("简体中文")});
    let path = ctx.root.join(".local/share/code-server/languagepacks.json");
    let mut cache = json_file(&path);
    if !cache.is_object() {
        cache = json!({});
    }
    cache["zh-cn"] = pack;
    atomic_write(&path, cache.to_string(), mode(&path, 0o600))
}
pub fn schedule_restart(ctx: &Context) -> Result<()> {
    let executable = env::current_exe()?;
    let mut command = ctx.command(executable.to_str().context("invalid executable path")?);
    command.arg("__restart-code-server");
    detach(&mut command, &ctx.logs().join("code-server-restart.log"))?;
    Ok(())
}
#[derive(Parser)]
#[command(name = "workspacectl locale")]
pub struct LocaleArgs {
    pub locale: Option<String>,
    #[arg(long)]
    pub restart: bool,
    #[arg(long, hide = true)]
    pub register_language_pack: bool,
}
pub fn locale(ctx: &Context, args: LocaleArgs, json_output: bool) -> Result<i32> {
    if args.register_language_pack {
        ensure!(
            args.locale.is_none() && !args.restart,
            "language pack registration takes no other arguments"
        );
        register_language_pack(ctx)?;
        println!("Simplified Chinese language pack registered");
        return Ok(0);
    }
    if let Some(value) = &args.locale {
        let current = inspect::normalize_locale(value).context("locale must be en or zh-cn")?;
        if current == "zh-cn" {
            register_language_pack(ctx)?;
        }
        let config = env::var_os("CODE_SERVER_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.root.join(".config/code-server/config.yaml"));
        update_yaml(&config, "locale", current)?;
        atomic_write(&ctx.state().join("locale"), format!("{current}\n"), 0o600)?;
        if args.restart {
            schedule_restart(ctx)?;
        }
    }
    let payload = inspect::locale(ctx);
    if json_output {
        print_json(&payload);
    } else {
        println!(
            "Interface language: {} ({})",
            inspect::text(&payload["label"]),
            inspect::text(&payload["current"])
        );
        if args.locale.is_some() {
            println!(
                "{}",
                if args.restart {
                    "code-server restart scheduled"
                } else {
                    "Restart code-server.service to apply the new language"
                }
            );
        }
    }
    Ok(0)
}
#[derive(Parser)]
#[command(name = "workspacectl timezone")]
pub struct TimezoneArgs {
    #[arg(default_value="status",value_parser=["status","set","rollback","apply"])]
    pub action: String,
    pub zone: Option<String>,
    #[arg(long)]
    pub config_only: bool,
}
pub fn validate_zone(zone: &str) -> Result<chrono_tz::Tz> {
    ensure!(
        !zone.is_empty()
            && zone.split('/').all(|part| !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_+.-".contains(&c))),
        "Use an IANA timezone, for example America/Phoenix"
    );
    let timezone = zone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| anyhow::anyhow!("Unknown IANA timezone: {zone}"))?;
    ensure!(
        Path::new("/usr/share/zoneinfo").join(zone).is_file(),
        "System zoneinfo file is missing: {zone}"
    );
    Ok(timezone)
}
fn timezone_path(ctx: &Context) -> PathBuf {
    ctx.root.join(".config/agent-workspace/timezone")
}
fn nullable_read(path: &Path) -> Value {
    if path.is_file() {
        json!(read(path).trim())
    } else {
        Value::Null
    }
}
fn system_zone() -> Option<String> {
    fs::canonicalize("/etc/localtime")
        .ok()?
        .strip_prefix("/usr/share/zoneinfo")
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}
async fn apply_zone(ctx: &Context, zone: &str) -> Result<()> {
    validate_zone(zone)?;
    let zone_path = format!("/usr/share/zoneinfo/{zone}");
    #[cfg(unix)]
    let root = unsafe { libc::geteuid() == 0 };
    #[cfg(not(unix))]
    let root = false;
    let args = if root {
        vec!["ln", "-snf", &zone_path, "/etc/localtime"]
    } else {
        vec!["sudo", "-n", "ln", "-snf", &zone_path, "/etc/localtime"]
    };
    ctx.probe(&args, 10.)
        .await
        .checked("timezone configuration")?;
    Ok(())
}
fn timezone_hooks(ctx: &Context) -> Result<()> {
    let state = timezone_path(ctx);
    let quoted = shell_words::quote(&state.to_string_lossy()).into_owned();
    let block = format!(
        "# BEGIN Workspace timezone\nif [ -r {quoted} ]; then\n  IFS= read -r TZ < {quoted}\n  export TZ\nfi\n# END Workspace timezone\n"
    );
    let managed =
        regex::Regex::new(r"(?ms)^# BEGIN Workspace timezone\n.*?^# END Workspace timezone\n?")?;
    let exports = regex::Regex::new(r"(?m)^export TZ=[A-Za-z0-9_/+.-]+[ \t]*$")?;
    for name in [".profile", ".bashrc"] {
        let path = ctx.root.join(name);
        let original = read(&path);
        let content = managed.replace_all(&original, "");
        let content = exports.replace_all(&content, "");
        let content = format!("{block}\n{}\n", content.trim());
        if content != original {
            let backup = ctx.root.join(format!("{name}.before-workspace-timezone"));
            if path.exists() && !backup.exists() {
                atomic_write(&backup, &original, 0o600)?;
            }
            atomic_write(&path, content, mode(&path, 0o600))?;
        }
    }
    let executable = env::current_exe()?;
    let script = format!(
        "#!/bin/sh\nAGENT_WORKSPACE_CONFIG_ROOT={} {} timezone apply || exit $?\nexec sleep infinity\n",
        shell_words::quote(&ctx.root.to_string_lossy()),
        shell_words::quote(&executable.to_string_lossy())
    );
    atomic_write(
        &ctx.root.join("custom-services.d/workspace-timezone/run"),
        script,
        0o755,
    )
}
pub async fn set_zone(ctx: &Context, zone: &str, config_only: bool) -> Result<()> {
    validate_zone(zone)?;
    let path = timezone_path(ctx);
    let previous = if path.is_file() {
        Some(read(&path).trim().to_owned())
    } else {
        system_zone()
    };
    if !config_only {
        apply_zone(ctx, zone).await?;
    }
    timezone_hooks(ctx)?;
    if let Some(previous) = previous.filter(|previous| previous != zone) {
        atomic_write(
            &path.with_file_name("timezone.previous"),
            format!("{previous}\n"),
            0o600,
        )?;
    }
    atomic_write(&path, format!("{zone}\n"), mode(&path, 0o600))?;
    atomic_write(
        &path.with_file_name("timezone.env"),
        format!("TZ={zone}\n"),
        0o600,
    )?;
    atomic_write(
        &path.with_file_name("timezone.compose.yaml"),
        format!(
            "services:\n  agent-workspace:\n    environment:\n      TZ: {}\n",
            json!(zone)
        ),
        0o600,
    )
}
pub fn timezone_status(ctx: &Context) -> Result<Value> {
    let path = timezone_path(ctx);
    let configured = nullable_read(&path);
    let local = if let Some(zone) = configured.as_str() {
        Some(chrono::Utc::now().with_timezone(&validate_zone(zone)?))
    } else {
        None
    };
    Ok(
        json!({"configured":configured,"system":system_zone(),"process_TZ":env::var("TZ").ok(),"previous":nullable_read(&path.with_file_name("timezone.previous")),"offset":local.as_ref().map(|time| time.offset().fix().to_string().replace(':',"")),"local_time":local.map(|time| time.to_rfc3339()),"container_env_file":path.with_file_name("timezone.env"),"compose_override":path.with_file_name("timezone.compose.yaml"),"browser_verification":"not_checked","note":"Existing shells, services and browsers may require restart; IP and fonts are unchanged."}),
    )
}
pub async fn timezone(ctx: &Context, args: TimezoneArgs, json_output: bool) -> Result<i32> {
    ensure!(
        args.zone.is_none() || args.action == "set",
        "a timezone argument is only accepted by set"
    );
    match args.action.as_str() {
        "set" => {
            set_zone(
                ctx,
                args.zone.as_deref().context("Supply a timezone")?,
                args.config_only,
            )
            .await?
        }
        "rollback" => {
            let zone = read(timezone_path(ctx).with_file_name("timezone.previous"));
            ensure!(!zone.trim().is_empty(), "No previous timezone is available");
            set_zone(ctx, zone.trim(), args.config_only).await?;
        }
        "apply" => {
            let zone = read(timezone_path(ctx));
            ensure!(
                !zone.trim().is_empty(),
                "No persisted timezone; run workspacectl timezone set <IANA-zone>"
            );
            if !args.config_only {
                apply_zone(ctx, zone.trim()).await?;
            }
        }
        _ => {}
    }
    let value = timezone_status(ctx)?;
    if json_output {
        print_json(&value);
    } else {
        for (key, value) in value.as_object().unwrap() {
            println!(
                "{key}: {}",
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            );
        }
    }
    Ok(0)
}

#[derive(Parser)]
#[command(name = "workspacectl web")]
pub struct WebArgs {
    #[arg(default_value="status",value_parser=["install","start","stop","restart","status","url","configure"])]
    pub action: String,
    #[arg(long)]
    pub public_url: Option<String>,
    #[arg(long)]
    pub workspace_url: Option<String>,
    #[arg(long)]
    pub desktop_url: Option<String>,
    #[arg(long)]
    pub runtime_url: Option<String>,
    #[arg(long)]
    pub port: Option<u16>,
}
fn web_config(ctx: &Context) -> Result<Value> {
    let mut config = json!({"port":9080,"publicUrl":"","workspaceUrl":"http://localhost:8443","desktopUrl":"https://localhost:3001","runtimeUrl":"http://127.0.0.1:9090"});
    let path = ctx.root.join(".config/agent-workspace/web.json");
    if path.exists() {
        let value: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        config.as_object_mut().unwrap().extend(
            value
                .as_object()
                .context("web configuration must be an object")?
                .clone(),
        );
    }
    Ok(config)
}
fn validate_web_url(value: &str, dedicated: bool) -> Result<String> {
    let url = reqwest::Url::parse(value)?;
    ensure!(
        ["http", "https"].contains(&url.scheme())
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none(),
        "Use an http(s) URL without credentials"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "URL must not include query or fragment"
    );
    ensure!(
        !dedicated || url.path() == "/",
        "public-url must use a dedicated origin without a path"
    );
    Ok(value.trim_end_matches('/').into())
}
pub async fn web(ctx: &Context, args: WebArgs) -> Result<i32> {
    ensure!(
        args.action == "configure"
            || (args.public_url.is_none()
                && args.workspace_url.is_none()
                && args.desktop_url.is_none()
                && args.runtime_url.is_none()
                && args.port.is_none()),
        "configuration options require web configure"
    );
    let unit = "agent-workspace-web.service";
    match args.action.as_str() {
        "start" | "stop" | "restart" | "status" => {
            return ctx
                .execute(&["systemctl", "--user", &args.action, unit])
                .await;
        }
        "url" => {
            let config = web_config(ctx)?;
            let public = inspect::text(&config["publicUrl"]);
            println!(
                "{}",
                if public.is_empty() {
                    format!("http://127.0.0.1:{}", config["port"])
                } else {
                    public.into()
                }
            );
        }
        "configure" => {
            let mut value = web_config(ctx)?;
            if let Some(port) = args.port {
                ensure!(port >= 1024, "port must be between 1024 and 65535");
                value["port"] = json!(port);
            }
            for (key, option) in [
                ("publicUrl", args.public_url),
                ("workspaceUrl", args.workspace_url),
                ("desktopUrl", args.desktop_url),
                ("runtimeUrl", args.runtime_url),
            ] {
                if let Some(url) = option {
                    let validated =
                        validate_web_url(&url, key == "publicUrl" || key == "runtimeUrl")?;
                    if key == "runtimeUrl" {
                        let parsed = reqwest::Url::parse(&validated)?;
                        ensure!(
                            parsed.scheme() == "http"
                                && matches!(
                                    parsed.host_str(),
                                    Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
                                ),
                            "runtime-url must be an HTTP loopback origin"
                        );
                    }
                    value[key] = json!(validated);
                }
            }
            atomic_write(
                &ctx.root.join(".config/agent-workspace/web.json"),
                format!("{}\n", serde_json::to_string_pretty(&value)?),
                0o600,
            )?;
            if ctx.units().join(unit).exists() {
                let code = ctx
                    .execute(&["systemctl", "--user", "restart", unit])
                    .await?;
                ensure!(code == 0, "Web service restart failed");
            }
            print_json(&value);
        }
        "install" => {
            let installer = ctx.implementation_script("install-workspace-web.sh")?;
            ensure!(
                ctx.execute(&["bash", installer.to_str().context("Invalid source path")?])
                    .await?
                    == 0,
                "Web service installation failed"
            );
            println!(
                "Web Control Center: http://127.0.0.1:{}",
                web_config(ctx)?["port"]
            );
        }
        _ => bail!("unsupported web action"),
    }
    Ok(0)
}
