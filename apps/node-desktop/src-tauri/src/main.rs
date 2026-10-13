#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use cranibox_node::{
    node::{self, Control},
    settings::{self, Settings},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{
    Manager, State,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_autostart::ManagerExt;

struct Running {
    control: Control,
    task: tauri::async_runtime::JoinHandle<()>,
}
#[derive(Default)]
struct Service {
    settings: Option<Settings>,
    running: Option<Running>,
    error: Option<String>,
}
struct AppState {
    service: tokio::sync::Mutex<Service>,
    quit_requested: AtomicBool,
    quitting: AtomicBool,
}
fn start(settings: Settings) -> Running {
    let control = Control::default();
    let task_control = control.clone();
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = node::run_managed(settings, task_control.clone()).await {
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
    if service
        .running
        .as_ref()
        .is_none_or(|running| running.task.inner().is_finished())
    {
        service.running = Some(start(settings));
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
async fn snapshot(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let service = state.service.lock().await;
    let status = service
        .running
        .as_ref()
        .map(|running| running.control.status.borrow().clone());
    let autostart = app
        .autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())?;
    Ok(
        json!({"settings":service.settings,"status":status,"error":service.error,"autostart":autostart,"quit_requested":state.quit_requested.load(Ordering::Relaxed)}),
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
    if input.autostart {
        app.autolaunch().enable()?;
    } else {
        app.autolaunch().disable()?;
    }
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
fn mcp_config() -> Result<Value, String> {
    let executable: PathBuf = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(
        json!({"mcpServers":{"cranibox-devices":{"command":executable,"args":["mcp","--config",settings::default_path()]}}}),
    )
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
        quit_requested: AtomicBool::new(false),
        quitting: AtomicBool::new(false),
    });
    let setup_state = state.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--background"])
                .build(),
        )
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            snapshot,
            configure,
            reconnect,
            peers,
            mcp_config,
            hide_window,
            quit,
            dismiss_quit
        ])
        .setup(move |app| {
            let open = MenuItem::with_id(app, "open", "打开 Cranibox Node", true, None::<&str>)?;
            let reconnect = MenuItem::with_id(
                app,
                "reconnect",
                "重新连接（保留 Shell）",
                true,
                None::<&str>,
            )?;
            let exit = MenuItem::with_id(app, "quit", "退出并结束本机 Shell…", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &reconnect, &exit])?;
            TrayIconBuilder::with_id("node")
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/32x32.png"
                ))?)
                .tooltip("Cranibox Node — 正在启动")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "reconnect" => {
                        let state = app.state::<Arc<AppState>>().inner().clone();
                        tauri::async_runtime::spawn(async move {
                            let mut service = state.service.lock().await;
                            let _ = resume(&mut service);
                        });
                    }
                    "quit" => {
                        app.state::<Arc<AppState>>()
                            .quit_requested
                            .store(true, Ordering::Relaxed);
                        show(app);
                    }
                    _ => {}
                })
                .build(app)?;
            let path = settings::default_path();
            let initial = if path.exists() {
                Settings::read(&path).map(Some)
            } else {
                Ok(None)
            };
            let configured = initial.as_ref().is_ok_and(|settings| settings.is_some());
            tauri::async_runtime::block_on(async {
                let mut service = setup_state.service.lock().await;
                match initial {
                    Ok(Some(settings)) => {
                        service.settings = Some(settings.clone());
                        service.running = Some(start(settings));
                    }
                    Ok(None) => {}
                    Err(error) => service.error = Some(format!("{error:#}")),
                }
            });
            if !background || !configured {
                show(app.handle());
            }
            let handle = app.handle().clone();
            let status_state = setup_state.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let status = {
                        let service = status_state.service.lock().await;
                        service
                            .running
                            .as_ref()
                            .map(|running| running.control.status.borrow().state.clone())
                            .unwrap_or("未配置".into())
                    };
                    if let Some(tray) = handle.tray_by_id("node") {
                        let _ = tray.set_tooltip(Some(format!("Cranibox Node — {status}")));
                    }
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
