//! The desktop owns the launcher; the versioned child owns the node and interface.
use crate::AppState;
use anyhow::{Context, Result};
use cranibox_node::{
    update::{
        launcher::{self, LauncherConfig},
        layout::Layout,
    },
    web::{endpoint, http_shell::LocalInterface, standalone::Shell},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::Manager;

pub fn adopted(root: &Path) -> Result<bool> {
    Ok(Layout::new(root).pointer()?.is_some())
}

pub async fn start(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    settings: PathBuf,
    root: PathBuf,
    shell: Arc<dyn Shell>,
) -> Result<()> {
    let mut task = state.managed_task.lock().await;
    if state.stop.is_cancelled() {
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let token = agent_workspace_runtime::config::token();
    let mut config = LauncherConfig::new(
        root,
        vec!["--config".into(), settings.as_os_str().to_owned()],
    );
    config.env = vec![
        (
            "CRANIBOX_SHELL_URL".into(),
            format!("http://{}", listener.local_addr()?).into(),
        ),
        ("CRANIBOX_SHELL_TOKEN".into(), token.clone().into()),
    ];
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let app = app.clone();
        let state = state.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = state.stop.cancelled() => return,
                _ = terminate.recv() => {},
                _ = tokio::signal::ctrl_c() => {},
            }
            crate::quit_app(app, state).await;
        });
    }
    let peers = cranibox_node::peer::config::path_for(&settings);
    let stop = state.stop.child_token();
    let mut server = tokio::spawn(cranibox_node_desktop::shell_server::serve(
        listener,
        token,
        shell,
        stop.clone(),
    ));
    cranibox_node::update::start_terminal_host(peers.parent().unwrap_or(&peers)).await;
    let mut launcher = tokio::spawn(launcher::run(config, stop.clone()));
    let running = state.clone();
    *task = Some(tokio::spawn(async move {
        let result = loop {
            tokio::select! {
                result = &mut launcher => break match result {
                    Ok(Ok(code)) => format!("Runtime launcher exited ({code})"),
                    Ok(Err(error)) => format!("Runtime launcher: {error:#}"),
                    Err(error) => format!("Runtime launcher task: {error}"),
                },
                result = &mut server => {
                    stop.cancel();
                    let _ = launcher.await;
                    break format!("Shell server stopped: {result:?}");
                }
                refreshed = async {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    refresh(&app, &running, &peers).await
                } => {
                    if let Err(error) = refreshed {
                        let mut service = running.service.lock().await;
                        service.url = None;
                        service.interface = None;
                        service.error = Some(format!("Waiting for runtime interface: {error:#}"));
                    }
                }
            }
        };
        stop.cancel();
        server.abort();
        let mut service = running.service.lock().await;
        service.url = None;
        service.interface = None;
        if !running.stop.is_cancelled() {
            service.error = Some(result);
        }
    }));
    Ok(())
}

async fn refresh(app: &tauri::AppHandle, state: &AppState, peers: &Path) -> Result<()> {
    let url = endpoint::ui_url(peers, None).await?;
    {
        let service = state.service.lock().await;
        if service.url.as_ref() == Some(&url) && service.interface.is_some() {
            return Ok(());
        }
    }
    let interface = Arc::new(LocalInterface::connect(&url).await?);
    if state.stop.is_cancelled() {
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("main") {
        window.navigate(url.parse()?)?;
    }
    if let Some(window) = app.get_webview_window("terminal") {
        let mut terminal: url::Url = url.parse()?;
        terminal.query_pairs_mut().append_pair("window", "terminal");
        window.navigate(terminal)?;
    }
    let mut service = state.service.lock().await;
    service.url = Some(url);
    service.interface = Some(interface);
    service.error = None;
    Ok(())
}

/// Keep native decisions on the same App API as browser decisions.
pub async fn dispatch(state: &AppState, method: &str, path: &str, body: Value) -> Result<Value> {
    let (managed, interface, local) = {
        let service = state.service.lock().await;
        (
            service.managed,
            service.interface.clone(),
            service.local.clone(),
        )
    };
    if managed {
        let interface = interface.context("Runtime interface unavailable")?;
        interface
            .request(method, &format!("/api/v1/app{path}"), body)
            .await
    } else {
        cranibox_node::app::dispatch(local, method, path, body)
            .await
            .map_err(|error| anyhow::anyhow!("{}", error.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranibox_node::update::{npm::executable_relative_path, version::Version};

    #[test]
    fn adoption_requires_a_valid_pointer() {
        let root = std::env::temp_dir().join(format!(
            "desktop-adoption-{}",
            agent_workspace_runtime::config::token()
        ));
        assert!(!adopted(&root).unwrap());
        assert!(!root.exists(), "Detection must not create a runtime");
        let layout = Layout::new(&root);
        let version = Version::parse("1.0.0").unwrap();
        let stage = layout.begin_stage(&version).unwrap();
        let executable = stage.join(executable_relative_path());
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(executable, b"test").unwrap();
        layout.commit_stage(&version).unwrap();
        assert!(!adopted(&root).unwrap());
        layout.activate(&version).unwrap();
        assert!(adopted(&root).unwrap());
        std::fs::write(root.join("current.json"), b"invalid").unwrap();
        assert!(
            adopted(&root).is_err(),
            "Corrupt pointers must not fall back to an embedded node"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
