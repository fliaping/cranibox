#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod app_api;
mod approvals;
mod desktops;
mod terminals;

use cranibox_node::{
    node::{self, Control},
    settings::{self, Settings},
};
use serde::Deserialize;
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

struct Running {
    control: Control,
    task: tauri::async_runtime::JoinHandle<()>,
}
#[derive(Default)]
struct Service {
    local: Option<Arc<cranibox_node::local::LocalNode>>,
    settings: Option<Settings>,
    running: Option<Running>,
    error: Option<String>,
}
struct AppState {
    service: tokio::sync::Mutex<Service>,
    applications: tokio::sync::Mutex<Option<cranibox_node::spaces::applications::Gateway>>,
    quit_requested: AtomicBool,
    quitting: AtomicBool,
}
fn start(settings: Settings, local: Arc<cranibox_node::local::LocalNode>) -> Running {
    let control = Control::default();
    let task_control = control.clone();
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = node::connect_managed(settings, task_control.clone(), local).await {
            task_control.status.send_replace(node::ConnectionStatus {
                state: "failed".into(),
                message: Some(format!("{error:#}")),
            });
        }
    });
    Running { control, task }
}
fn resume(service: &mut Service) -> Result<(), String> {
    let settings = service
        .settings
        .clone()
        .ok_or("Configure this device first")?;
    let local = service
        .local
        .clone()
        .ok_or("Local device service unavailable")?;
    if service
        .running
        .as_ref()
        .is_none_or(|running| running.task.inner().is_finished())
    {
        service.running = Some(start(settings, local));
    } else if let Some(running) = &service.running {
        running.control.reconnect.notify_one();
    }
    service.error = None;
    Ok(())
}
fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    update_autostart(&app, enabled).map_err(|error| format!("{error:#}"))
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
#[tauri::command]
async fn snapshot(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let service = state.service.lock().await;
    let status = service
        .running
        .as_ref()
        .map(|running| running.control.status.borrow().clone());
    let autostart = autostart_enabled(&app).map_err(|error| format!("{error:#}"))?;
    Ok(
        json!({"settings":service.settings,"status":status,"local":service.local.as_ref().map(|local| json!({"id":local.id,"online":true})),"error":service.error,"autostart":autostart,"quit_requested":state.quit_requested.load(Ordering::Relaxed)}),
    )
}
#[derive(Deserialize)]
struct Configure {
    server: String,
    id: String,
    name: String,
    token: String,
    allow_insecure: bool,
    autostart: bool,
}
#[tauri::command]
async fn configure(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    input: Configure,
) -> Result<(), String> {
    configure_inner(app, state, input)
        .await
        .map_err(|error| format!("{error:#}"))
}
async fn configure_inner(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    input: Configure,
) -> anyhow::Result<()> {
    let path = settings::default_path();
    let token_file = path.with_file_name("node.token");
    // Validate everything before saving a new credential.
    let token = input.token.trim();
    anyhow::ensure!(
        token.is_empty()
            || (token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())),
        "Token must contain 64 hexadecimal characters"
    );
    let settings = Settings {
        server: input.server.trim().trim_end_matches('/').to_owned(),
        id: input.id.trim().into(),
        name: if input.name.trim().is_empty() {
            None
        } else {
            Some(input.name.trim().into())
        },
        token_file: token_file.clone(),
        allow_insecure: input.allow_insecure,
    };
    let url = agent_workspace_runtime::config::check_url(
        &settings.server,
        true,
        settings.allow_insecure,
    )?;
    anyhow::ensure!(
        url.path() == "/"
            && agent_workspace_runtime::protocol::valid_node_id(&settings.id)
            && settings
                .name
                .as_ref()
                .is_none_or(|name| name.len() <= 128 && !name.chars().any(char::is_control)),
        "Invalid node settings"
    );
    if token.is_empty() {
        agent_workspace_runtime::config::read_token(&token_file)?;
    }
    update_autostart(&app, input.autostart)?;
    let mut service = state.service.lock().await;
    let changed = service.settings.as_ref() != Some(&settings);
    if changed && let Some(running) = service.running.take() {
        running.control.stop.cancel();
        running.task.await?;
    }
    if !token.is_empty() {
        agent_workspace_runtime::config::write_private(
            &token_file,
            format!("{token}\n").as_bytes(),
            true,
        )?;
    }
    settings.write(&path)?;
    service.settings = Some(settings.clone());
    resume(&mut service).map_err(anyhow::Error::msg)?;
    Ok(())
}
#[tauri::command]
async fn reconnect(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut service = state.service.lock().await;
    resume(&mut service)
}
#[tauri::command]
async fn peers(state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let settings = state
        .service
        .lock()
        .await
        .settings
        .clone()
        .ok_or("Configure this device first")?;
    let api = settings.api().map_err(|error| error.to_string())?;
    api.get("/api/v1/nodes")
        .await
        .map_err(|error| error.to_string())
}
#[tauri::command]
async fn open_network_link(url: String) -> Result<(), String> {
    let parsed = url::Url::parse(&url).map_err(|_| "Invalid network link")?;
    if url.len() > 2048
        || parsed.scheme() != "https"
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || !(parsed.host_str() == Some("login.tailscale.com")
            || (parsed.host_str() == Some("tailscale.com")
                && parsed.path() == "/download"
                && parsed.query().is_none()))
    {
        return Err("Only official Tailscale download and sign-in links can be opened.".into());
    }
    open_browser(parsed.as_str()).await
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum DesktopGuide {
    WindowsDownload,
    WindowsSettings,
    MacSharing,
}
#[tauri::command]
async fn open_desktop_guide(guide: DesktopGuide) -> Result<(), String> {
    let url = match guide {
        DesktopGuide::WindowsDownload => "https://uvnc.com/downloads/ultravnc.html",
        DesktopGuide::WindowsSettings => {
            "https://uvnc.com/docs/ultravnc-server/49-ultravnc-server-configuration.html"
        }
        DesktopGuide::MacSharing => {
            "https://support.apple.com/guide/mac-help/turn-screen-sharing-on-or-off-mh11848/mac"
        }
    };
    open_browser(url).await
}
async fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = tokio::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = tokio::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = tokio::process::Command::new("xdg-open");
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), command.status())
        .await
        .map_err(|_| "Opening the browser timed out.")?
        .map_err(|e| format!("Open system browser: {e}"))?;
    if !status.success() {
        return Err("The system browser could not open this link.".into());
    }
    Ok(())
}
#[tauri::command]
async fn network(state: State<'_, Arc<AppState>>, command: Value) -> Result<Value, String> {
    let command = serde_json::from_value(command).map_err(|e| e.to_string())?;
    let local = state
        .service
        .lock()
        .await
        .local
        .clone()
        .ok_or("Local device service unavailable")?;
    local
        .manage_network(command)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn desktop_service(state: State<'_, Arc<AppState>>, command: Value) -> Result<Value, String> {
    let command = serde_json::from_value(command).map_err(|e| e.to_string())?;
    let local = state
        .service
        .lock()
        .await
        .local
        .clone()
        .ok_or("Local device service unavailable")?;
    local
        .manage_desktop_service(command)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn spaces(command: Value) -> Result<Value, String> {
    let command = serde_json::from_value(command).map_err(|e| e.to_string())?;
    cranibox_node::spaces::Store::new(cranibox_node::peer::config::path_for(
        &settings::default_path(),
    ))
    .manage(command, true)
    .await
    .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn space_application(
    state: State<'_, Arc<AppState>>,
    id: String,
    application: cranibox_node::spaces::applications::Application,
) -> Result<(), String> {
    let id = id.parse().map_err(|_| "Invalid space ID")?;
    let store = cranibox_node::spaces::Store::new(cranibox_node::peer::config::path_for(
        &settings::default_path(),
    ));
    let mut gateway = state.applications.lock().await;
    if gateway.is_none() {
        *gateway = Some(
            cranibox_node::spaces::applications::Gateway::start()
                .await
                .map_err(|e| e.to_string())?,
        );
    }
    let url = gateway
        .as_ref()
        .unwrap()
        .open(&store, id, application)
        .await
        .map_err(|e| format!("{e:#}"))?;
    drop(gateway);
    open_browser(&url).await
}
#[tauri::command]
async fn direct_peers(state: State<'_, Arc<AppState>>, command: Value) -> Result<Value, String> {
    let path = cranibox_node::peer::config::path_for(&settings::default_path());
    let command = serde_json::from_value(command).map_err(|e| e.to_string())?;
    let local = state.service.lock().await.local.clone();
    match local {
        Some(local) => local.manage_peers(command).await,
        None => {
            agent_workspace_runtime::peers::Store::new(path)
                .manage(command)
                .await
        }
    }
    .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn projects(state: State<'_, Arc<AppState>>, command: Value) -> Result<Value, String> {
    let settings = state
        .service
        .lock()
        .await
        .settings
        .clone()
        .ok_or("Configure this device first")?;
    settings
        .api()
        .map_err(|e| e.to_string())?
        .post("/api/v1/projects", command)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn local_projects(state: State<'_, Arc<AppState>>, command: Value) -> Result<Value, String> {
    let local = state
        .service
        .lock()
        .await
        .local
        .clone()
        .ok_or("Local device service unavailable")?;
    let command = serde_json::from_value(command).map_err(|e| e.to_string())?;
    local.projects(command).await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn local_sessions(state: State<'_, Arc<AppState>>, command: Value) -> Result<Value, String> {
    let local = state
        .service
        .lock()
        .await
        .local
        .clone()
        .ok_or("Local device service unavailable")?;
    let command = serde_json::from_value(command).map_err(|e| e.to_string())?;
    local.sessions(command).await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn terminal_sessions(
    state: State<'_, Arc<AppState>>,
    node: String,
    command: Value,
) -> Result<Value, String> {
    if !agent_workspace_runtime::protocol::valid_node_id(&node) {
        return Err("Invalid node ID".into());
    }
    let settings = state
        .service
        .lock()
        .await
        .settings
        .clone()
        .ok_or("Configure this device first")?;
    settings
        .api()
        .map_err(|e| e.to_string())?
        .post(&format!("/api/v1/nodes/{node}/terminal/sessions"), command)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn mcp_config(include_runs: Option<bool>) -> Result<Value, String> {
    Ok(cranibox_node_desktop::launch::Launcher::current()
        .map_err(|error| format!("{error:#}"))?
        .mcp_config(&settings::default_path(), include_runs.unwrap_or(false)))
}
#[tauri::command]
async fn hide_window(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or("Window unavailable")?
        .hide()
        .map_err(|error| error.to_string())
}
#[tauri::command]
async fn quit(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.quitting.store(true, Ordering::Relaxed);
    let running = state.service.lock().await.running.take();
    if let Some(running) = running {
        running.control.stop.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), running.task).await;
    }
    if let Some(local) = state.service.lock().await.local.take() {
        let _ = local.shutdown().await;
    }
    app.exit(0);
    Ok(())
}
#[tauri::command]
fn dismiss_quit(state: State<'_, Arc<AppState>>) {
    state.quit_requested.store(false, Ordering::Relaxed);
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
        applications: Default::default(),
        quit_requested: AtomicBool::new(false),
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
        .manage(Arc::new(terminals::Terminals::default()))
        .manage(Arc::new(desktops::Desktops::default()))
        .invoke_handler(tauri::generate_handler![
            space_application,
            app_api::app_call,
            app_api::app_status,
            app_api::app_devices,
            app_api::app_pair_invite,
            app_api::app_pair_accept,
            app_api::app_pair_confirm,
            app_api::app_approvals,
            app_api::app_approval_decide,
            app_api::app_agents,
            app_api::app_agent_update,
            app_api::app_models,
            app_api::app_events,
            app_api::app_open_external,
            snapshot,
            set_autostart,
            configure,
            reconnect,
            peers,
            direct_peers,
            desktop_service,
            network,
            open_network_link,
            open_desktop_guide,
            spaces,
            terminals::terminal_open,
            terminals::terminal_input,
            terminals::terminal_close,
            desktops::desktop_open,
            desktops::desktop_input,
            desktops::desktop_ack,
            desktops::desktop_close,
            projects,
            local_projects,
            local_sessions,
            terminal_sessions,
            mcp_config,
            hide_window,
            quit,
            dismiss_quit
        ])
        .setup(move |app| {
            approvals::setup(app.handle())?;
            let open = MenuItem::with_id(app, "open", "打开 Cranibox Node", true, None::<&str>)?;
            let held_item = MenuItem::with_id(
                app,
                "held",
                cranibox_node_desktop::notify::menu_label(0),
                true,
                None::<&str>,
            )?;
            let status_item = MenuItem::with_id(app, "status", "正在启动 / Starting", false, None::<&str>)?;
            let pause_screen = MenuItem::with_id(app, "pause-screen", "暂停共享屏幕 / Pause screen sharing", true, None::<&str>)?;
            let reconnect = MenuItem::with_id(
                app,
                "reconnect",
                "重新连接（保留 Shell）",
                true,
                None::<&str>,
            )?;
            let exit = MenuItem::with_id(app, "quit", "退出并结束本机 Shell…", true, None::<&str>)?;
            let decisions = Submenu::with_id(app, "decisions", "处理审批 / Decide requests", false)?;
            let menu = Menu::with_items(app, &[&status_item, &held_item, &decisions, &open, &pause_screen, &reconnect, &exit])?;
            TrayIconBuilder::with_id("node")
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/32x32.png"
                ))?)
                .tooltip("Cranibox Node — 正在启动")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "held" => approvals::inbox(app),
                    "reconnect" => {
                        let state = app.state::<Arc<AppState>>().inner().clone();
                        tauri::async_runtime::spawn(async move {
                            let mut service = state.service.lock().await;
                            let _ = resume(&mut service);
                        });
                    }
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
                        app.state::<Arc<AppState>>()
                            .quit_requested
                            .store(true, Ordering::Relaxed);
                        show(app);
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
            let initial = if path.exists() {
                Settings::read(&path).map(Some)
            } else {
                Ok(None)
            };
            let ready = tauri::async_runtime::block_on(async {
                let mut service = setup_state.service.lock().await;
                match cranibox_node::local::LocalNode::start(cranibox_node::peer::config::path_for(
                    &path,
                ))
                .await
                {
                    Ok(local) => {
                        if let Err(error) = local.models().await {
                            service.error = Some(format!("{error:#}"));
                        }
                        service.local = Some(Arc::new(local));
                    }
                    Err(error) => service.error = Some(format!("{error:#}")),
                }
                match initial {
                    Ok(Some(settings)) => {
                        service.settings = Some(settings);
                        if let Err(error) = resume(&mut service) {
                            service.error = Some(error);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => service.error = Some(format!("{error:#}")),
                }
                service.local.is_some() && service.error.is_none()
            });
            if !background || !ready {
                show(app.handle());
            }
            let handle = app.handle().clone();
            let status_state = setup_state.clone();
            let held_menu = held_item.clone();
            tauri::async_runtime::spawn(async move {
                let mut seen = std::collections::BTreeSet::new();
                let mut previous_cards = Value::Null;
                loop {
                    let status = {
                        let service = status_state.service.lock().await;
                        service
                            .running
                            .as_ref()
                            .map(|running| running.control.status.borrow().state.clone())
                            .unwrap_or(
                                if service.local.is_some() {
                                    "本机服务运行中"
                                } else {
                                    "本机服务不可用"
                                }
                                .into(),
                            )
                    };
                    let local = status_state.service.lock().await.local.clone();
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
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())?;
    app.run(move |app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && !state.quitting.load(Ordering::Relaxed)
        {
            api.prevent_exit();
            state.quit_requested.store(true, Ordering::Relaxed);
            show(app);
        }
    });
    Ok(())
}
