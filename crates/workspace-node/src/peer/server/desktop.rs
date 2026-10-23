use super::*;
use agent_workspace_runtime::peers::{
    config::{Desktop, DesktopProtocol},
    desktop::{Event, MAX_CHUNK, PAGE_UNAVAILABLE, Stream},
};

/// The configured desktop and a check that stays true while sharing and the grant last.
fn shared(
    app: &App,
    caller: Option<Caller>,
    direct: bool,
) -> Result<(Desktop, impl Fn() -> bool + Send + Sync + 'static)> {
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
    let app = app.clone();
    let configured = desktop.clone();
    let valid = move || {
        !app.stop.is_cancelled()
            && (!direct || !app.connections.is_cancelled())
            && Config::read(&app.path).is_ok_and(|config| {
                config.node_id == app.node
                    && config.desktop_epoch == epoch
                    && config.desktop.as_ref() == Some(&configured)
                    && caller.as_ref().is_none_or(|caller| {
                        config.grants.get(&caller.node).is_some_and(|grant| {
                            grant.permissions.desktop && grant.same_credential(&caller.grant)
                        })
                    })
            })
    };
    anyhow::ensure!(valid(), "desktop access not granted or sharing stopped");
    Ok((desktop, valid))
}

pub(super) async fn open(app: &App, caller: Option<Caller>, direct: bool) -> Result<Stream> {
    let (desktop, valid) = shared(app, caller, direct)?;
    let permit = app
        .desktops
        .clone()
        .try_acquire_owned()
        .context("desktop already has an active controller; disconnect it first")?;
    let address = desktop.address;
    if desktop.protocol == DesktopProtocol::Selkies {
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

/// One plain TCP connection to the shared Selkies server, which serves its own web
/// page, API and stream on that port. The viewer sees Selkies' interface unchanged.
pub(super) async fn page(app: &App, caller: Option<Caller>, direct: bool) -> Result<Stream> {
    let (desktop, valid) = shared(app, caller, direct)?;
    anyhow::ensure!(
        desktop.protocol == DesktopProtocol::Selkies,
        PAGE_UNAVAILABLE
    );
    let permit = app
        .desktop_pages
        .clone()
        .try_acquire_owned()
        .context("too many desktop page connections")?;
    let tcp = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(desktop.address),
    )
    .await?
    .context("local Selkies server is unavailable")?;
    tcp.set_nodelay(true)?;
    Ok(Stream::from_tcp(tcp, permit, valid))
}

pub(super) async fn upgrade(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    secret: Option<Extension<crate::peer::secure_ws::StreamSecret>>,
    ws: WebSocketUpgrade,
) -> Result<Response<Body>, Failure> {
    relay(app, caller, node, secret, ws, false).await
}
pub(super) async fn page_upgrade(
    State(app): State<App>,
    Extension(caller): Extension<Caller>,
    Path(node): Path<String>,
    secret: Option<Extension<crate::peer::secure_ws::StreamSecret>>,
    ws: WebSocketUpgrade,
) -> Result<Response<Body>, Failure> {
    relay(app, caller, node, secret, ws, true).await
}
async fn relay(
    app: App,
    caller: Caller,
    node: String,
    secret: Option<Extension<crate::peer::secure_ws::StreamSecret>>,
    ws: WebSocketUpgrade,
    web: bool,
) -> Result<Response<Body>, Failure> {
    check_target(&app, &node)?;
    if !caller.grant.permissions.desktop {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "desktop capability not granted by target node",
        ));
    }
    let opened = if web {
        page(&app, Some(caller), true).await
    } else {
        open(&app, Some(caller), true).await
    };
    let mut desktop = opened.map_err(|e| failure(StatusCode::CONFLICT, &e.to_string()))?;
    let margin = crate::peer::secure_ws::MARGIN;
    Ok(ws.max_message_size(MAX_CHUNK + margin).max_frame_size(MAX_CHUNK + margin).on_upgrade(move |socket| async move {
        let mut socket = crate::peer::secure_ws::SecureSocket::new(socket, secret.as_ref().map(|Extension(s)| s));
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
