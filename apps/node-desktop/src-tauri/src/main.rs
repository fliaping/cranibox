#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! The desktop app is the node core plus a window, tray and system integration. Its
//! window shows the same loopback web interface that the CLI node serves.
mod approvals;
mod managed;

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
    managed: bool,
    interface: Option<Arc<cranibox_node::web::http_shell::LocalInterface>>,
    /// Sign-in link of this window's interface server.
    url: Option<String>,
    error: Option<String>,
}
struct AppState {
    service: tokio::sync::Mutex<Service>,
    stop: tokio_util::sync::CancellationToken,
    quitting: AtomicBool,
    /// A window close is under way; the exit request it triggers keeps the tray running.
    window_closing: AtomicBool,
    /// Last automatic sign-in per window, so a cookie that never sticks cannot loop.
    signed_in_again: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    managed_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
fn show(app: &tauri::AppHandle) {
    show_at(app, None);
}
/// Show the main window, creating it again if it was closed. A closed window is
/// destroyed to release its webview; `page` (path and fragment) is where a new
/// window opens, while an existing one is only brought forward.
fn show_at(app: &tauri::AppHandle, page: Option<&'static str>) -> bool {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return true;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<Arc<AppState>>().inner().clone();
        let url = state.service.lock().await.url.clone();
        if let Err(error) = create_main(&app, url.as_deref(), page) {
            eprintln!("Open the window: {error:#}");
        }
    });
    false
}
fn create_main(
    app: &tauri::AppHandle,
    sign_in: Option<&str>,
    page: Option<&str>,
) -> anyhow::Result<()> {
    if app.get_webview_window("main").is_some() {
        return Ok(());
    }
    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no main window configuration"))?;
    if let Some(sign_in) = sign_in {
        // Without an interface yet the bundled splash page waits for it.
        let mut url = url::Url::parse(sign_in)?;
        if let Some((path, fragment)) = page.map(|page| page.split_once('#').unwrap_or((page, "")))
        {
            if !path.is_empty() {
                url.set_path(path);
            }
            url.set_fragment(Some(fragment).filter(|f| !f.is_empty()));
        }
        config.url = tauri::WebviewUrl::External(url);
    }
    config.visible = true;
    config.focus = true;
    tauri::WebviewWindowBuilder::from_config(app, &config)?.build()?;
    Ok(())
}
/// A page that lost its session (the runtime restarted, or the cookie was dropped)
/// lands on `/sign-in`; sign the window in again with the current link.
fn sign_in_again(window: &tauri::Webview, url: &url::Url) {
    if url.path() != "/sign-in" || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]")) {
        return;
    }
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    let terminal = url
        .query_pairs()
        .any(|(key, value)| key == "window" && value == "terminal");
    tauri::async_runtime::spawn(async move {
        let state = app.state::<Arc<AppState>>().inner().clone();
        {
            let mut recent = state
                .signed_in_again
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let now = std::time::Instant::now();
            if recent
                .get(&label)
                .is_some_and(|at| now.duration_since(*at) < std::time::Duration::from_secs(15))
            {
                // The fresh session did not hold either; leave the explanation on screen.
                return;
            }
            recent.insert(label.clone(), now);
        }
        let (managed, url) = {
            let service = state.service.lock().await;
            (service.managed, service.url.clone())
        };
        // A managed runtime may have restarted under a new link not yet picked up.
        let url = if managed {
            let peers = cranibox_node::peer::config::path_for(&settings::default_path());
            cranibox_node::web::endpoint::ui_url(&peers, None)
                .await
                .ok()
                .or(url)
        } else {
            url
        };
        let Some(Ok(mut url)) = url.map(|url| url::Url::parse(&url)) else {
            return;
        };
        if terminal {
            url.query_pairs_mut().append_pair("window", "terminal");
        }
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.navigate(url);
        }
    });
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
    if stop_app(&state).await {
        app.exit(0);
    }
}
async fn stop_app(state: &AppState) -> bool {
    if state.quitting.swap(true, Ordering::Relaxed) {
        return false;
    }
    state.stop.cancel();
    if let Some(task) = state.managed_task.lock().await.take() {
        let _ = task.await;
    }
    if let Some(local) = state.service.lock().await.local.take() {
        let _ = local.shutdown().await;
    }
    true
}

static TERMINAL_LAUNCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Terminals open in one native window with tabs; later requests add tabs to it.
fn open_terminal(app: &tauri::AppHandle, sign_in: &str, body: &Value) -> anyhow::Result<()> {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for key in ["node", "session", "command", "cwd"] {
        if let Some(value) = body[key]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 4096)
        {
            query.append_pair(key, value);
        }
    }
    if body["create"].as_bool() == Some(true) {
        query.append_pair("create", "1");
    }
    // A fresh counter makes every request a hash change, even for the same target.
    let serial = TERMINAL_LAUNCHES.fetch_add(1, Ordering::Relaxed);
    query.append_pair("n", &serial.to_string());
    let hash = format!("#/launch?{}", query.finish());
    if let Some(window) = app.get_webview_window("terminal") {
        window.eval(format!("location.hash = {}", serde_json::to_string(&hash)?))?;
        let _ = window.unminimize();
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }
    // Same origin as the main window, so its sign-in cookie applies.
    let mut url = url::Url::parse(sign_in)?;
    url.set_path("/");
    url.set_query(Some("window=terminal"));
    url.set_fragment(Some(&hash[1..]));
    tauri::WebviewWindowBuilder::new(app, "terminal", tauri::WebviewUrl::External(url))
        .title("Terminal · Cranibox")
        .inner_size(1100.0, 700.0)
        .min_inner_size(520.0, 320.0)
        .build()?;
    Ok(())
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
            "open_terminal" => {
                let base = self
                    .state
                    .service
                    .lock()
                    .await
                    .url
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("the interface is not running"))?;
                open_terminal(&self.app, &base, &body)?;
                Ok(json!({}))
            }
            "hide" => {
                let app = self.app.clone();
                // Answer the page before its window and webview are released.
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.close();
                    }
                });
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
            "restart" => {
                let (app, state) = (self.app.clone(), self.state.clone());
                tauri::async_runtime::spawn(async move {
                    // Finish the response, then release the node and its private
                    // driver before relaunching the permission-owning process.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    if stop_app(&state).await {
                        app.request_restart();
                    }
                });
                Ok(json!({}))
            }
            // The bundle process raises the macOS prompts so they name this app.
            "request_permissions" => {
                anyhow::ensure!(cfg!(target_os = "macos"), "unsupported on this platform");
                agent_workspace_runtime::peers::computer::cua::embedded::request_permissions();
                Ok(json!({}))
            }
            "command_line" => cranibox_node_desktop::command_line::status(),
            "install_command_line" => cranibox_node_desktop::command_line::install().await,
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
    let argv: Vec<String> = std::env::args().collect();
    let args = &argv[1..];
    // With a command the app is the `cranibox` command line; without one it opens the window.
    if !args.is_empty() && args != ["--background"] {
        #[cfg(target_os = "windows")]
        cranibox_node_desktop::console::attach();
        return tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?
            .block_on(cranibox_node::cli::run_program(
                argv.clone(),
                env!("CARGO_PKG_VERSION"),
            ));
    }
    // Adoption in the window names the first managed version after this program.
    cranibox_node::update::set_product_version(env!("CARGO_PKG_VERSION"));
    let background = args == ["--background"];
    // Started from Finder or a login item, the app has a minimal PATH: take the login
    // shell's, so installed agents and their tools are found.
    if let Some(path) = cranibox_node::login_path::resolve() {
        // SAFETY: no other thread exists yet.
        unsafe { std::env::set_var("PATH", path) };
    }
    let state = Arc::new(AppState {
        service: Default::default(),
        stop: Default::default(),
        quitting: AtomicBool::new(false),
        window_closing: AtomicBool::new(false),
        signed_in_again: Default::default(),
        managed_task: Default::default(),
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
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                sign_in_again(webview, payload.url());
            }
        })
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
            // macOS menu bars take a monochrome template that follows light and dark
            // mode; other trays show the colored app icon.
            let tray_icon: &[u8] = if cfg!(target_os = "macos") {
                include_bytes!("../icons/tray-template.png")
            } else {
                include_bytes!("../icons/32x32.png")
            };
            TrayIconBuilder::with_id("node")
                .icon(tauri::image::Image::from_bytes(tray_icon)?)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Cranibox — 正在启动")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "held" => approvals::inbox(app),
                    "pause-screen" => {
                        let state = app.state::<Arc<AppState>>().inner().clone();
                        tauri::async_runtime::spawn(async move {
                            let (managed, interface, local) = {
                                let service = state.service.lock().await;
                                (service.managed, service.interface.clone(), service.local.clone())
                            };
                            let command = agent_workspace_runtime::peers::Command::ComputerOwner { command: agent_workspace_runtime::peers::computer::OwnerCommand::EmergencyStop {} };
                            if managed {
                                if let Some(interface) = interface {
                                    let result = interface.request("POST", "/api/peers", json!({
                                        "operation": "computer_owner",
                                        "command": {"action": "emergency_stop"},
                                    })).await;
                                    if let Err(error) = result { eprintln!("Emergency stop: {error:#}"); }
                                } else { eprintln!("Emergency stop: runtime interface unavailable"); }
                            } else if let Some(local) = local {
                                let _ = local.manage_peers(command).await;
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
                let root = cranibox_node::update::runtime_root(path.parent().unwrap_or(&path));
                // Remote updates are on by default: the first start makes this program the first
                // managed version. If that fails the app keeps running unmanaged as before.
                let adopted = match managed::adopted(&root).map(|adopted| {
                    if adopted || std::env::var_os("CRANIBOX_NO_MANAGED").is_some() {
                        return adopted;
                    }
                    match cranibox_node::update::cli::adopt(&root) {
                        Ok(_) => true,
                        Err(error) => {
                            eprintln!("Remote updates are not available on this device: {error:#}");
                            false
                        }
                    }
                }) {
                    Ok(adopted) => adopted,
                    Err(error) => {
                        setup_state.service.lock().await.error = Some(format!("Read runtime pointer: {error:#}"));
                        return None;
                    }
                };
                if adopted {
                    setup_state.service.lock().await.managed = true;
                    if let Err(error) = managed::start(handle.clone(), setup_state.clone(), path.clone(), root, shell).await {
                        setup_state.service.lock().await.error = Some(format!("Start runtime: {error:#}"));
                    }
                    return None;
                }
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
                // A fresh token per launch, shared with ui-url through a private runtime record.
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
            // Started in the background, no window (and no webview) exists until opened.
            if let Some(url) = &url
                && let Some(window) = app.get_webview_window("main")
            {
                window.navigate(url::Url::parse(url)?)?;
            }
            let managed_starting = tauri::async_runtime::block_on(async {
                let service = setup_state.service.lock().await;
                service.managed && service.error.is_none()
            });
            if !background || (url.is_none() && !managed_starting) {
                show(app.handle());
            }
            let status_state = setup_state.clone();
            let held_menu = held_item.clone();
            tauri::async_runtime::spawn(async move {
                let mut seen = std::collections::BTreeSet::new();
                let mut access_seen = std::collections::BTreeSet::new();
                let mut previous_cards = Value::Null;
                loop {
                    let (status, managed, interface) = {
                        let service = status_state.service.lock().await;
                        let status = if service.local.is_some() || service.interface.is_some() {
                            "本机服务运行中 / Running"
                        } else {
                            "本机服务不可用 / Unavailable"
                        };
                        (status, service.managed, service.interface.clone())
                    };
                    let cards = managed::dispatch(&status_state, "GET", "/approvals", Value::Null).await.ok();
                    let status = if managed && cards.is_none() {
                        let mut service = status_state.service.lock().await;
                        if service.interface.as_ref().zip(interface.as_ref()).is_some_and(|(current, queried)| Arc::ptr_eq(current, queried)) {
                            service.interface = None;
                            service.url = None;
                            service.error = Some("Runtime interface unavailable".into());
                        }
                        "本机服务不可用 / Unavailable"
                    } else { status };
                    let cards = if managed { Some(cards.unwrap_or_else(|| json!([]))) } else { cards };
                    let access = managed::dispatch(&status_state, "GET", "/access", Value::Null).await.ok();
                    let pending = access.as_ref().and_then(|a| a["pending"].as_array());
                    approvals::access_refresh(&handle, pending.map_or(&[], Vec::as_slice), &mut access_seen);
                    let held = cards.as_ref().and_then(Value::as_array).map_or(0, |v| v.len() as u64) + pending.map_or(0, |p| p.len() as u64);
                    if let Some(cards) = cards && cards != previous_cards
                        && approvals::refresh(&handle, &decisions, cards.as_array().map_or(&[], Vec::as_slice), &mut seen).is_ok() {
                        previous_cards = cards;
                    }
                    if let Some(tray) = handle.tray_by_id("node") {
                        let _ = tray.set_tooltip(Some(cranibox_node_desktop::notify::tooltip(
                            status, held,
                        )));
                        // Linux tray hosts ignore tooltips; show the count next to the icon.
                        let _ = tray.set_title(cranibox_node_desktop::notify::title(held));
                    }
                    let _ = status_item.set_text(status);
                    let _ = held_menu.set_text(cranibox_node_desktop::notify::menu_label(held));
                    tokio::select! {
                        _ = status_state.stop.cancelled() => break,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {},
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            let state = window.state::<Arc<AppState>>();
            match event {
                // Closing a window destroys it to release its webview; the device keeps
                // running in the tray, which opens a new window on demand.
                tauri::WindowEvent::CloseRequested { .. } => {
                    state.window_closing.store(true, Ordering::Relaxed);
                }
                tauri::WindowEvent::Destroyed => {
                    let others = window
                        .app_handle()
                        .webview_windows()
                        .keys()
                        .any(|label| label != window.label());
                    // Only the last window's close leads to an exit request.
                    if others {
                        state.window_closing.store(false, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())?;
    app.run(move |app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event
            && !state.quitting.load(Ordering::Relaxed)
        {
            if code.is_none() && state.window_closing.swap(false, Ordering::Relaxed) {
                // The last window closed: stay in the tray.
                api.prevent_exit();
                return;
            }
            // An OS quit request (Cmd+Q, logout) ends this device's shells cleanly.
            api.prevent_exit();
            tauri::async_runtime::spawn(quit_app(app.clone(), state.clone()));
        }
    });
    Ok(())
}
