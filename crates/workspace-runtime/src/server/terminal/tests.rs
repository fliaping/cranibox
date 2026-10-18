use super::*;
use crate::{
    client::{Api, ApiOptions},
    peers::terminal::{Event, Input},
};

struct Fixture {
    _root: tempfile::TempDir,
    gateway: Gateway,
    api: Api,
    received: mpsc::Receiver<Wire>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let token = config::token();
        let path = root.path().join("runtime.json");
        let credentials = config::Credentials {
            controller_token: config::token(),
            nodes: [
                ("caller".into(), token.clone()),
                ("target".into(), config::token()),
            ]
            .into(),
            node_aliases: Default::default(),
            mesh_groups: [
                ("caller".into(), "personal".into()),
                ("target".into(), "personal".into()),
            ]
            .into(),
        };
        config::write_private(&path, &serde_json::to_vec(&credentials).unwrap(), false).unwrap();
        let token_path = root.path().join("node.token");
        config::write_private(&token_path, token.as_bytes(), false).unwrap();
        let gateway = Gateway::new(path).unwrap();
        let (sender, received) = mpsc::channel(32);
        gateway.app.inner.lock().await.nodes.insert(
            "target".into(),
            Node {
                name: "Target".into(),
                os: "linux".into(),
                arch: "test".into(),
                terminal: true,
                terminal_sessions: true,
                desktop_relay: true,
                generation: "generation-1".into(),
                tx: Some(sender),
                last_seen: now(),
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = Api::mesh(ApiOptions {
            url: format!("http://{}", listener.local_addr().unwrap()),
            token_file: Some(token_path),
            credentials: PathBuf::new(),
            allow_insecure: false,
        })
        .unwrap();
        let router = gateway.router();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            _root: root,
            gateway,
            api,
            received,
            server,
        }
    }
    async fn receive(&mut self) -> Wire {
        tokio::time::timeout(Duration::from_secs(3), self.received.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn attach(&mut self) -> (crate::peers::terminal::Terminal, String) {
        let session = uuid::Uuid::new_v4().to_string();
        let terminal = self
            .api
            .terminal("target", &session, 101, 31, false)
            .await
            .unwrap();
        let Wire::TerminalOpen {
            id,
            session: actual,
            cols: 101,
            rows: 31,
            takeover: false,
        } = self.receive().await
        else {
            panic!("expected original terminal attachment");
        };
        assert_eq!(actual, session);
        (terminal, id)
    }
    async fn output(&self, id: &str, bytes: &[u8]) {
        self.gateway
            .app
            .inner
            .lock()
            .await
            .terminals
            .get(id)
            .unwrap()
            .tx
            .try_send(Wire::TerminalOutput {
                id: id.into(),
                data: STANDARD.encode(bytes),
            })
            .unwrap();
    }
}

#[tokio::test]
async fn node_client_streams_binary_output_and_detaches_without_ending_session() {
    let mut fixture = Fixture::new().await;
    let (mut terminal, id) = fixture.attach().await;
    let output = b"\0\xff terminal \xe4\xb8\xad\xe6\x96\x87";
    fixture.output(&id, output).await;
    let event = tokio::time::timeout(Duration::from_secs(3), terminal.output.recv())
        .await
        .unwrap();
    assert!(matches!(event, Some(Event::Output(bytes)) if bytes == output));
    terminal
        .send(Input::Input {
            data: "中文\u{3}".into(),
        })
        .unwrap();
    let Wire::TerminalInput { id: actual, data } = fixture.receive().await else {
        panic!("expected terminal input");
    };
    assert_eq!(actual, id);
    assert_eq!(STANDARD.decode(data).unwrap(), "中文\u{3}".as_bytes());
    terminal
        .send(Input::Resize {
            cols: 110,
            rows: 32,
        })
        .unwrap();
    assert!(
        matches!(fixture.receive().await, Wire::TerminalResize { id: actual, cols: 110, rows: 32 } if actual == id)
    );
    drop(terminal);
    assert!(matches!(fixture.receive().await, Wire::TerminalClose { id: actual } if actual == id));
}

#[tokio::test]
async fn active_node_client_loses_attachment_when_token_or_group_is_revoked() {
    for rotate_token in [false, true] {
        let mut fixture = Fixture::new().await;
        let (mut terminal, id) = fixture.attach().await;
        fixture.output(&id, b"attached").await;
        assert!(matches!(
            terminal.output.recv().await,
            Some(Event::Output(_))
        ));
        let path = &fixture.gateway.app.credentials;
        let mut credentials = config::Credentials::read(path).unwrap();
        if rotate_token {
            credentials.nodes.insert("caller".into(), config::token());
        } else {
            credentials.mesh_groups.remove("caller");
        }
        config::write_private(path, &serde_json::to_vec(&credentials).unwrap(), true).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(event) = terminal.output.recv().await {
                assert!(!matches!(event, Event::Output(_) | Event::Exit(_)));
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(fixture.receive().await, Wire::TerminalClose { id: actual } if actual == id)
        );
        assert!(
            fixture
                .api
                .terminal("target", &uuid::Uuid::new_v4().to_string(), 80, 24, false)
                .await
                .is_err()
        );
    }
}
