use crate::common::*;
use anyhow::{Context as _, Result, bail, ensure};
use fs2::FileExt;
use serde_json::{Value, json};
#[cfg(unix)]
use std::env;
use std::{collections::BTreeMap, fs, time::Duration};

#[cfg(unix)]
const KEYS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "DBUS_SESSION_BUS_ADDRESS",
    "HOME",
    "XDG_CONFIG_HOME",
    "DCONF_PROFILE",
    "GSETTINGS_BACKEND",
];
fn desktop_env() -> Result<BTreeMap<String, String>> {
    #[cfg(unix)]
    for path in directories("/proc") {
        use std::os::unix::fs::MetadataExt;
        if !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().bytes().all(|c| c.is_ascii_digit()))
            || !fs::metadata(&path)
                .is_ok_and(|metadata| metadata.uid() == unsafe { libc::getuid() })
            || read(path.join("comm")).trim() != "plasmashell"
        {
            continue;
        }
        let values = read(path.join("environ"))
            .split('\0')
            .filter_map(|item| item.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect::<BTreeMap<_, _>>();
        if values.get("XDG_SESSION_TYPE").map(String::as_str) != Some("wayland")
            || values.get("WAYLAND_DISPLAY").is_none_or(String::is_empty)
            || values
                .get("DBUS_SESSION_BUS_ADDRESS")
                .is_none_or(String::is_empty)
        {
            continue;
        }
        let mut output = env::vars()
            .filter(|(key, _)| !KEYS.contains(&key.as_str()))
            .collect::<BTreeMap<_, _>>();
        output.extend(
            values
                .into_iter()
                .filter(|(key, _)| KEYS.contains(&key.as_str()) && key != "GSETTINGS_BACKEND"),
        );
        return Ok(output);
    }
    bail!("No current-user KDE Wayland session found; start the desktop first")
}
async fn command(ctx: &Context, vars: &BTreeMap<String, String>, args: &[&str]) -> Result<String> {
    let mut command = ctx.command(args[0]);
    command.args(&args[1..]).env_clear().envs(vars);
    capture(command, Duration::from_secs(15))
        .await
        .checked(args[0])
}
fn factor(value: &str) -> Result<f64> {
    let value: f64 = value.parse()?;
    ensure!(
        value.is_finite() && value > 0.,
        "Text scale must be a finite positive number"
    );
    Ok(value)
}
async fn read_scale(ctx: &Context, vars: &BTreeMap<String, String>) -> Result<f64> {
    factor(
        &command(
            ctx,
            vars,
            &[
                "gsettings",
                "get",
                "org.gnome.desktop.interface",
                "text-scaling-factor",
            ],
        )
        .await?,
    )
}
pub async fn run(ctx: &Context, action: &str) -> Result<i32> {
    ensure!(
        ["status", "repair", "restore"].contains(&action),
        "Usage: workspacectl desktop scaling status|repair|restore"
    );
    let vars = desktop_env()?;
    let state = ctx
        .root
        .join(".local/share/agent-workspace/desktop-scaling");
    let backup = state.join("original.json");
    let value = if action == "status" {
        let scale = read_scale(ctx, &vars).await?;
        let info = command(
            ctx,
            &vars,
            &["qdbus6", "org.kde.KWin", "/KWin", "supportInformation"],
        )
        .await?;
        let scales = regex::Regex::new(r"(?m)^Scale: ([0-9.]+)\s*$")?
            .captures_iter(&info)
            .filter_map(|capture| capture[1].parse::<f64>().ok())
            .collect::<Vec<_>>();
        let config = vars.get("XDG_CONFIG_HOME").cloned().unwrap_or_else(|| {
            format!(
                "{}/.config",
                vars.get("HOME")
                    .unwrap_or(&ctx.root.to_string_lossy().into_owned())
            )
        });
        json!({"state":if scale == 1. {"aligned-text-scale"} else {"extra-text-scale"},"wayland_display":vars.get("WAYLAND_DISPLAY"),"compositor_scales":scales,"text_scale":scale,"settings_file":format!("{config}/dconf/user"),"backup_available":backup.is_file(),"note":"This checks desktop settings; verify application scaling across a live Selkies scale change."})
    } else {
        ensure!(
            command(
                ctx,
                &vars,
                &[
                    "gsettings",
                    "writable",
                    "org.gnome.desktop.interface",
                    "text-scaling-factor"
                ]
            )
            .await?
                == "true",
            "The desktop text scale is not writable"
        );
        fs::create_dir_all(&state)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(state.join("lock"))?;
        lock.lock_exclusive()?;
        let before = read_scale(ctx, &vars).await?;
        let target = if action == "restore" {
            let saved: Value = serde_json::from_str(
                &fs::read_to_string(&backup).context("No managed scaling backup is available")?,
            )?;
            factor(&saved["text_scale"].to_string())?
        } else {
            if before != 1. && !backup.exists() {
                write_json(&backup, &json!({"text_scale":before}))?;
            }
            1.
        };
        if before != target {
            command(
                ctx,
                &vars,
                &[
                    "gsettings",
                    "set",
                    "org.gnome.desktop.interface",
                    "text-scaling-factor",
                    &target.to_string(),
                ],
            )
            .await?;
        }
        ensure!(
            read_scale(ctx, &vars).await? == target,
            "Desktop text scale did not retain the requested value"
        );
        if action == "restore" {
            fs::remove_file(&backup)?;
        }
        json!({"action":action,"previous_text_scale":before,"text_scale":target,"changed":before != target,"backup_available":backup.is_file()})
    };
    print_json(&value);
    Ok(0)
}
