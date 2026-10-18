use super::*;
use agent_workspace_runtime::peers::desktop::{Event, MAX_CHUNK, Stream};

pub(super) async fn open(app: &App, caller: Option<Caller>, direct: bool) -> Result<Stream> {
    let configuration = Config::read(&app.path)?;
    anyhow::ensure!(
        configuration.node_id == app.node && !app.stop.is_cancelled(),
        "local node identity changed or stopping"
    );
    let epoch = configuration.desktop_epoch;
    let desktop = configuration
        .desktop
        .context("configure a local desktop server before sharing this desktop")?;
    desktop.validate()?;
    let permit = app
        .desktops
        .clone()
        .try_acquire_owned()
        .context("desktop already has an active controller; disconnect it first")?;
    let address = desktop.address;
    let protocol = desktop.protocol;
    let app = app.clone();
    let valid = move || {
        !app.stop.is_cancelled()
            && (!direct || !app.connections.is_cancelled())
            && Config::read(&app.path).is_ok_and(|config| {
                config.node_id == app.node
                    && config.desktop_epoch == epoch
                    && config.desktop.as_ref() == Some(&desktop)
                    && caller.as_ref().is_none_or(|caller| {
                        config.grants.get(&caller.node).is_some_and(|grant| {
                            grant.permissions.desktop && grant.same_credential(&caller.grant)
                        })
                    })
            })
    };
    anyhow::ensure!(valid(), "desktop access not granted or sharing stopped");
    if protocol == agent_workspace_runtime::peers::config::DesktopProtocol::Selkies {
        return agent_workspace_runtime::peers::desktop::selkies::connect(address, permit, valid)
            .await;
    }
    let tcp = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(address),
    )
    .await?
    .context("local VNC server is unavailable")?;
    tcp.set_nodelay(true)?;
    Ok(Stream::from_tcp(tcp, permit, valid))
}

pub(super) async fn upgrade(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response<Body>, Failure> {
    check_target(&app, &node)?;
    if !caller.grant.permissions.desktop {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "desktop capability not granted by target node",
        ));
    }
    let mut desktop = open(&app, Some(caller), true)
        .await
        .map_err(|e| failure(StatusCode::CONFLICT, &e.to_string()))?;
    Ok(ws.max_message_size(MAX_CHUNK).max_frame_size(MAX_CHUNK).on_upgrade(move |mut socket| async move {
        let run: Result<()> = async {
            loop {
                tokio::select! {
                    event = desktop.output.recv() => match event {
                        Some(Event::Data(data)) => { tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Binary(data.into()))).await??; }
                        _ => break,
                    },
                    message = socket.recv() => match message {
                        Some(Ok(Message::Binary(bytes))) => desktop.send(bytes.to_vec())?,
                        Some(Ok(Message::Ping(data))) => { tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await??; }
                        Some(Ok(Message::Pong(_))) => {},
                        _ => break,
                    },
                }
            }
            Ok(())
        }.await;
        let _ = run;
        drop(desktop);
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
    }))
}
