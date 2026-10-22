#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! The desktop app is the node core plus a window, tray and system integration. Its
//! window shows the same loopback web interface that the CLI node serves.
mod approvals;

use cranibox_node::{
    settings,
    web::standalone::{self, Shell},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{
    Manager, State,
    menu::{Menu, MenuItem, Submenu},
    tray::TrayIconBuilder,
};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_autostart::ManagerExt;

#[derive(Default)]
struct Service {
    local: Option<Arc<cranibox_node::local::LocalNode>>,
    /// Sign-in link of this window's interface server.
    url: Option<String>,
    error: Option<String>,
}
struct AppState {
    service: tokio::sync::Mutex<Service>,
    stop: tokio_util::sync::CancellationToken,
    quitting: AtomicBool,
}
fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
fn update_autostart(app: &tauri::AppHandle, enabled: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        cranibox_node_desktop::launch::Autostart::current()?.set_enabled(
            &cranibox_node_desktop::launch::Launcher::current()?,
            enabled,
        )?;
    }
    #[cfg(not(target_os = "linux"))]
    if enabled {
        app.autolaunch().enable()?;
    } else {
        app.autolaunch().disable()?;
    }
    Ok(())
}
fn autostart_enabled(app: &tauri::AppHandle) -> anyhow::Result<bool> {
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        cranibox_node_desktop::launch::Autostart::current()?.is_enabled()
    }
    #[cfg(not(target_os = "linux"))]
    Ok(app.autolaunch().is_enabled()?)
}
async fn open_browser(url: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(url)?;
    anyhow::ensure!(
        url.len() <= 2048
            && matches!(parsed.scheme(), "https" | "http")
            && parsed.username().is_empty()
            && parsed.password().is_none(),
        "Only web links without credentials can be opened"
    );
    #[cfg(target_os = "windows")]
    let mut command = tokio::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = tokio::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = tokio::process::Command::new("xdg-open");
    command
        .arg(parsed.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), command.status())
        .await
        .map_err(|_| anyhow::anyhow!("Opening the browser timed out"))??;
    anyhow::ensure!(
        status.success(),
        "The system browser could not open this link"
    );
    Ok(())
}
async fn quit_app(app: tauri::AppHandle, state: Arc<AppState>) {
    if state.quitting.swap(true, Ordering::Relaxed) {
        return;
    }
    state.stop.cancel();
    if let Some(local) = state.service.lock().await.local.take() {
        let _ = local.shutdown().await;
    }
    app.exit(0);
}

/// Native operations offered to this window's page through `/api/shell`.
struct NativeShell {
    app: tauri::AppHandle,
    state: Arc<AppState>,
}
#[async_trait::async_trait]
impl Shell for NativeShell {
    async fn call(&self, operation: &str, body: Value) -> anyhow::Result<Value> {
        let status = |app: &tauri::AppHandle| -> anyhow::Result<Value> {
            Ok(json!({
                "autostart": autostart_enabled(app)?,
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
            }))
        };
        match operation {
            "status" => status(&self.app),
            "autostart" => {
                let enabled = body["enabled"]
                    .as_bool()
                    .ok_or_else(|| anyhow::anyhow!("enabled must be true or false"))?;
                update_autostart(&self.app, enabled)?;
                status(&self.app)
            }
            "open_external" => {
                open_browser(body["url"].as_str().unwrap_or_default()).await?;
                Ok(json!({}))
            }
            "hide" => {
                if let Some(window) = self.app.get_webview_window("main") {
                    window.hide()?;
                }
                Ok(json!({}))
            }
            "quit" => {
                let (app, state) = (self.app.clone(), self.state.clone());
                // Answer the page before the window and server go away.
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    quit_app(app, state).await;
                });
                Ok(json!({}))
            }
            "mcp_config" => Ok(
                cranibox_node_desktop::launch::Launcher::current()?.mcp_config(
                    &settings::default_path(),
                    body["include_runs"].as_bool().unwrap_or(false),
                ),
            ),
            _ => anyhow::bail!("unknown operation"),
        }
    }
}

/// Startup state for the splash page shown until the interface is ready.
#[tauri::command]
async fn snapshot(state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let service = state.service.lock().await;
    Ok(json!({"ready": service.url.is_some(), "error": service.error}))
}
#[tauri::command]
async fn quit(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    quit_app(app, state.inner().clone()).await;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() && args != ["--background"] {
        return tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?
            .block_on(cranibox_node::cli::run());
    }
    let background = args == ["--background"];
    let state = Arc::new(AppState {
        service: Default::default(),
        stop: Default::default(),
        quitting: AtomicBool::new(false),
    });
    let setup_state = state.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)));
    #[cfg(not(target_os = "linux"))]
    let app = app.plugin(
        tauri_plugin_autostart::Builder::new()
            .args(["--background"])
            .build(),
    );
    let app = app
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![snapshot, quit])
        .setup(move |app| {
            approvals::setup(app.handle())?;
            let open = MenuItem::with_id(app, "open", "打开 Cranibox / Open", true, None::<&str>)?;
            let held_item = MenuItem::with_id(
                app,
                "held",
                cranibox_node_desktop::notify::menu_label(0),
                true,
                None::<&str>,
            )?;
            let status_item = MenuItem::with_id(app, "status", "正在启动 / Starting", false, None::<&str>)?;
            let pause_screen = MenuItem::with_id(app, "pause-screen", "暂停共享屏幕 / Pause screen sharing", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, "quit", "退出并结束本机 Shell / Quit", true, None::<&str>)?;
            let decisions = Submenu::with_id(app, "decisions", "处理审批 / Decide requests", false)?;
            let menu = Menu::with_items(app, &[&status_item, &held_item, &decisions, &open, &pause_screen, &exit])?;
            TrayIconBuilder::with_id("node")
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/32x32.png"
                ))?)
                .tooltip("Cranibox — 正在启动")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "held" => approvals::inbox(app),
                    "pause-screen" => {
                        let state = app.state::<Arc<AppState>>().inner().clone();
                        tauri::async_runtime::spawn(async move {
                            let local = state.service.lock().await.local.clone();
                            if let Some(local) = local {
                                let _ = local.manage_peers(agent_workspace_runtime::peers::Command::ComputerOwner { command: agent_workspace_runtime::peers::computer::OwnerCommand::EmergencyStop {} }).await;
                            }
                        });
                    }
                    "quit" => {
                        let state = app.state::<Arc<AppState>>().inner().clone();
                        tauri::async_runtime::spawn(quit_app(app.clone(), state));
                    }
                    id => {
                        if let Some((decision, request)) = id.strip_prefix("approval:").and_then(|v| v.split_once(':'))
                            && matches!(decision, "allow_once" | "deny") {
                            approvals::decide(app, request.to_owned(), decision.to_owned());
                        }
                    }
                })
                .build(app)?;
            let path = settings::default_path();
            let handle = app.handle().clone();
            let shell: Arc<dyn Shell> = Arc::new(NativeShell {
                app: handle.clone(),
                state: setup_state.clone(),
            });
            let url = tauri::async_runtime::block_on(async {
                let mut service = setup_state.service.lock().await;
                let local = match cranibox_node::local::LocalNode::start(
                    cranibox_node::peer::config::path_for(&path),
                )
                .await
                {
                    Ok(local) => Arc::new(local),
                    Err(error) => {
                        service.error = Some(format!("{error:#}"));
                        return None;
                    }
                };
                service.local = Some(local.clone());
                // A fresh token per launch: only this window ever receives the link.
                let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
                    Ok(listener) => listener,
                    Err(error) => {
                        service.error = Some(format!("Start the interface: {error}"));
                        return None;
                    }
                };
                let address = listener.local_addr().ok()?;
                let token = agent_workspace_runtime::config::token();
                let url = standalone::sign_in_url(address, &token);
                let stop = setup_state.stop.clone();
                tauri::async_runtime::spawn(async move {
                    let options = standalone::Options {
                        token,
                        allow_hosts: Vec::new(),
                        shell: Some(shell),
                        relay: None,
                    };
                    if let Err(error) = standalone::serve(local, listener, options, stop).await {
                        eprintln!("interface server: {error:#}");
                    }
                });
                service.url = Some(url.clone());
                Some(url)
            });
            if let Some(url) = &url
                && let Some(window) = app.get_webview_window("main")
            {
                window.navigate(url::Url::parse(url)?)?;
            }
            if !background || url.is_none() {
                show(app.handle());
            }
            let status_state = setup_state.clone();
            let held_menu = held_item.clone();
            tauri::async_runtime::spawn(async move {
                let mut seen = std::collections::BTreeSet::new();
                let mut previous_cards = Value::Null;
                loop {
                    let (status, local) = {
                        let service = status_state.service.lock().await;
                        let status = if service.local.is_some() {
                            "本机服务运行中 / Running"
                        } else {
                            "本机服务不可用 / Unavailable"
                        };
                        (status, service.local.clone())
                    };
                    let cards = cranibox_node::app::dispatch(local, "GET", "/approvals", Value::Null).await.ok();
                    let held = cards.as_ref().and_then(Value::as_array).map_or(0, |v| v.len() as u64);
                    if let Some(cards) = cards && cards != previous_cards
                        && approvals::refresh(&handle, &decisions, cards.as_array().map_or(&[], Vec::as_slice), &mut seen).is_ok() {
                        previous_cards = cards;
                    }
                    if let Some(tray) = handle.tray_by_id("node") {
                        let _ = tray.set_tooltip(Some(cranibox_node_desktop::notify::tooltip(
                            &status, held,
                        )));
                        // Linux tray hosts ignore tooltips; show the count next to the icon.
                        let _ = tray.set_title(cranibox_node_desktop::notify::title(held));
                    }
                    let _ = status_item.set_text(&status);
                    let _ = held_menu.set_text(cranibox_node_desktop::notify::menu_label(held));
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Closing the window keeps the device running in the tray.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())?;
    app.run(move |app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && !state.quitting.load(Ordering::Relaxed)
        {
            // An OS quit request (Cmd+Q, logout) ends this device's shells cleanly.
            api.prevent_exit();
            tauri::async_runtime::spawn(quit_app(app.clone(), state.clone()));
        }
    });
    Ok(())
}
