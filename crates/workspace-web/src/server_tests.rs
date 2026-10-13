use super::*;
use axum::body::Body;
use futures_util::StreamExt;
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, State, SessionRef) {
    let directory = tempfile::tempdir().unwrap();
    let state = State::new(
        directory.path().into(),
        directory.path().join("ctl"),
        Config::default(),
    )
    .unwrap();
    let session = Arc::new(Mutex::new(Session::new()));
    state
        .sessions
        .lock()
        .await
        .insert(session.lock().await.id.clone(), session.clone());
    let mut cache = state.snapshot.lock().await;
    cache.at = millis();
    cache.value = Some(json!({"schema_version":1,"summary":{"healthy":true},"services":[]}));
    drop(cache);
    (directory, state, session)
}
async fn call(
    state: &State,
    session: &SessionRef,
    route: &str,
    body: Value,
    authorized: bool,
) -> Response {
    let s = session.lock().await;
    let request = Request::builder()
        .method("POST")
        .uri(route)
        .header("host", "localhost:9080")
        .header("content-type", "application/json")
        .header("cookie", format!("aw_session={}", s.id))
        .header(
            "x-workspace-csrf",
            if authorized {
                s.csrf.as_str()
            } else {
                "invalid"
            },
        )
        .body(Body::from(body.to_string()))
        .unwrap();
    drop(s);
    router(state.clone()).oneshot(request).await.unwrap()
}
async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}
async fn frame(
    stream: &mut (impl futures_util::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin),
) -> Value {
    loop {
        let bytes = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        if let Some(data) = text.lines().find_map(|line| line.strip_prefix("data: ")) {
            return serde_json::from_str(data).unwrap();
        }
    }
}
#[tokio::test]
async fn history_and_node_mutations_require_csrf_and_keep_tokens_private() {
    let (directory, state, session) = fixture().await;
    let path = directory
        .path()
        .join(".config/agent-workspace/runtime.json");
    agent_workspace_runtime::config::write_private(
        &path,
        &serde_json::to_vec(&agent_workspace_runtime::config::Credentials {
            controller_token: token(),
            nodes: Default::default(),
        })
        .unwrap(),
        false,
    )
    .unwrap();
    for route in [
        "/api/history/list",
        "/api/history/get",
        "/api/nodes/enroll",
        "/api/nodes/revoke",
        "/api/nodes/exec",
        "/api/events",
        "/api/metrics",
    ] {
        assert_eq!(
            call(&state, &session, route, json!({}), false)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let enrolled = json_body(
        call(
            &state,
            &session,
            "/api/nodes/enroll",
            json!({"id":"laptop"}),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(enrolled["token"].as_str().unwrap().len(), 64);
    assert_eq!(
        call(
            &state,
            &session,
            "/api/nodes/enroll",
            json!({"id":"laptop"}),
            true
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let rotated = json_body(
        call(
            &state,
            &session,
            "/api/nodes/enroll",
            json!({"id":"laptop","rotate":true}),
            true,
        )
        .await,
    )
    .await;
    assert_ne!(enrolled["token"], rotated["token"]);
    let history =
        json_body(call(&state, &session, "/api/history/list", json!({}), true).await).await;
    assert!(
        !history
            .to_string()
            .contains(enrolled["token"].as_str().unwrap())
    );
    assert_eq!(
        call(
            &state,
            &session,
            "/api/nodes/revoke",
            json!({"id":"laptop"}),
            true
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(
        agent_workspace_runtime::config::Credentials::read(&path)
            .unwrap()
            .nodes
            .is_empty()
    );
}
#[tokio::test]
async fn streams_send_only_changed_data_recover_after_disconnect_and_release_capacity() {
    let (_directory, state, session) = fixture().await;
    let mut streams = Vec::new();
    for _ in 0..4 {
        let response = call(&state, &session, "/api/events", json!({}), true).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        assert!(frame(&mut body).await["snapshot"].is_object());
        streams.push(body);
    }
    assert_eq!(
        call(&state, &session, "/api/events", json!({}), true)
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    session.lock().await.append("中文日志\n");
    for stream in &mut streams {
        let value = frame(stream).await;
        assert_eq!(value["outputDelta"], "中文日志\n");
        assert_eq!(value["outputReset"], false);
        assert!(value.get("snapshot").is_none());
    }
    streams.clear();
    let mut reconnected = call(&state, &session, "/api/events", json!({}), true)
        .await
        .into_body()
        .into_data_stream();
    let value = frame(&mut reconnected).await;
    assert_eq!(value["outputDelta"], "中文日志\n");
    assert_eq!(value["outputReset"], true);
    drop(reconnected);
    assert_eq!(
        session
            .lock()
            .await
            .streams
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}
#[tokio::test]
async fn persisted_history_rolls_utf8_cursors_and_recovers_without_replay() {
    let (directory, state, session) = fixture().await;
    let id = token();
    let mut record = json!({"id":id,"outcome":"running","created_at":millis(),"output":""});
    append_record(&mut record, &"中文".repeat(30000));
    state.history.lock().await.insert(record).unwrap();
    let value = json_body(
        call(
            &state,
            &session,
            "/api/history/get",
            json!({"id":id,"outputCursor":0}),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(value["outputReset"], true);
    assert!(!value["outputDelta"].as_str().unwrap().contains('�'));
    let cursor = value["outputCursor"].as_u64().unwrap();
    state
        .history
        .lock()
        .await
        .update(&id, |v| append_record(v, "后续\n"));
    let delta = json_body(
        call(
            &state,
            &session,
            "/api/history/get",
            json!({"id":id,"outputCursor":cursor}),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(delta["outputDelta"], "后续\n");
    assert_eq!(delta["outputReset"], false);
    state.maintenance().await;
    let restored =
        Journal::open(directory.path().join(".local/state/cranibox/web-history")).unwrap();
    assert_eq!(restored.get(&id).unwrap()["outcome"], "interrupted");
    let list = json_body(call(&state, &session, "/api/history/list", json!({}), true).await).await;
    assert!(list["records"][0].get("output").is_none());
}
#[cfg(unix)]
#[tokio::test]
async fn local_commands_record_exit_cancel_and_timeout() {
    use std::os::unix::fs::PermissionsExt;
    let (directory, mut state, session) = fixture().await;
    state.command = directory.path().join("ctl");
    std::fs::write(&state.command,"#!/bin/sh\ncase \"$1\" in fail) printf failure; exit 7;; slow) printf started; sleep 60;; *) printf complete;; esac\n").unwrap();
    std::fs::set_permissions(&state.command, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        !state
            .operation(&session, "failure", vec!["fail".into()], 5)
            .await
            .unwrap()
    );
    let record = state.history.lock().await.list()[0].clone();
    assert_eq!(record["outcome"], "failed");
    assert_eq!(record["exit_code"], 7);
    assert!(
        !state
            .operation(&session, "timeout", vec!["slow".into()], 0)
            .await
            .unwrap()
    );
    assert_eq!(
        state
            .history
            .lock()
            .await
            .list()
            .iter()
            .find(|v| v["title"] == "timeout")
            .unwrap()["outcome"],
        "timed_out"
    );
    let task_state = state.clone();
    let task_session = session.clone();
    let operation = tokio::spawn(async move {
        task_state
            .operation(&task_session, "cancel", vec!["slow".into()], 30)
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    session.lock().await.cancel.send_replace(true);
    assert!(!operation.await.unwrap().unwrap());
    assert_eq!(
        state
            .history
            .lock()
            .await
            .list()
            .iter()
            .find(|v| v["title"] == "cancel")
            .unwrap()["outcome"],
        "cancelled"
    );
}
#[tokio::test]
async fn output_reader_preserves_characters_across_byte_chunks() {
    use tokio::io::AsyncWriteExt;
    let (_directory, state, session) = fixture().await;
    let id = token();
    state
        .history
        .lock()
        .await
        .insert(json!({"id":id,"outcome":"running","output":""}))
        .unwrap();
    let (reader, mut writer) = tokio::io::duplex(32);
    let reading = stream(reader, session.clone(), state.history.clone(), id.clone());
    for byte in "中文\n".as_bytes() {
        writer.write_all(&[*byte]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(writer);
    reading.await.unwrap();
    assert_eq!(session.lock().await.output, "中文\n");
    assert_eq!(
        state.history.lock().await.get(&id).unwrap()["output"],
        "中文\n"
    );
}
#[tokio::test]
#[ignore = "explicit offline payload measurement; writes CRANIBOX_STREAM_REPORT"]
async fn measure_stream_payloads() {
    let (_directory, state, session) = fixture().await;
    let services:Vec<_>=(0..39).map(|i|json!({"id":format!("service-{i}"),"name":format!("Synthetic service {i}"),"kind":"systemd","status":"running","pid":2000+i})).collect();
    state.snapshot.lock().await.value.as_mut().unwrap()["services"] = json!(services);
    session.lock().await.append(&"x".repeat(128 * 1024));
    let mut streams = Vec::new();
    let mut pushed = 0;
    let mut polled = 0;
    for _ in 0..4 {
        let mut body = events(state.clone(), session.clone())
            .await
            .into_body()
            .into_data_stream();
        pushed += frame(&mut body).await.to_string().len();
        streams.push(body);
        polled += to_bytes(
            call(&state, &session, "/api/poll", json!({}), true)
                .await
                .into_body(),
            1024 * 1024,
        )
        .await
        .unwrap()
        .len();
    }
    for i in 0..20 {
        let line = format!("increment {i}: {}\n", "y".repeat(80));
        session.lock().await.append(&line);
        for body in &mut streams {
            loop {
                let value = frame(body).await;
                pushed += value.to_string().len();
                if value["outputDelta"]
                    .as_str()
                    .is_some_and(|v| v.contains(&line))
                {
                    break;
                }
            }
            polled += to_bytes(
                call(&state, &session, "/api/poll", json!({}), true)
                    .await
                    .into_body(),
                1024 * 1024,
            )
            .await
            .unwrap()
            .len();
        }
    }
    let report = json!({"scope":"in-process actual Rust API JSON payloads; HTTP headers excluded; synthetic 39-service snapshot, retained 128 KiB log, 4 tabs and 20 appended lines",
        "poll_bytes":polled,"event_bytes":pushed,"reduction_percent":100.0*(1.0-pushed as f64/polled as f64),
        "cache_hits":state.metrics.lock().await.cache_hits});
    let path = std::env::var_os("CRANIBOX_STREAM_REPORT").expect("set CRANIBOX_STREAM_REPORT");
    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
