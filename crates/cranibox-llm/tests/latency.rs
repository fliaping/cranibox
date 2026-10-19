//! Run explicitly on an idle target: cargo test -p cranibox-llm --test latency -- --ignored --nocapture
use cranibox_contracts::{
    llm::{
        Wire,
        mgmt::{LlmMgmt, NewAccount},
    },
    mock::MemoryAccountStore,
};
use serde_json::json;
use std::{sync::Arc, time::Instant};
#[tokio::test]
#[ignore = "host-dependent latency acceptance benchmark"]
async fn native_passthrough_overhead_p50_under_two_ms() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = format!("http://{}", listener.local_addr().unwrap());
    let mock = tokio::spawn(async move {
        axum::serve(listener,axum::Router::new().route("/v1/chat/completions",axum::routing::post(||async{axum::Json(json!({"id":"fake","choices":[{"message":{"role":"assistant","content":"OK"}}]}))}))).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let mut config = cranibox_llm::Config::new(
        "local".into(),
        dir.path().join("run"),
        Arc::new(MemoryAccountStore::default()),
    );
    config.listen = vec!["127.0.0.1:0".parse().unwrap()];
    let handle = cranibox_llm::spawn(config).await.unwrap();
    let account = handle
        .mgmt
        .add_account(NewAccount::Custom {
            label: "benchmark".into(),
            key: "fake-only".into(),
            endpoints: [(Wire::Chat, format!("{upstream}/v1"))].into(),
            models: vec!["model".into()],
        })
        .await
        .unwrap();
    let proxy = format!("http://{}/v1/chat/completions", handle.addresses[0]);
    let direct = format!("{upstream}/v1/chat/completions");
    let client = reqwest::Client::new();
    let mut direct_us = Vec::new();
    let mut proxy_us = Vec::new();
    for i in 0..140 {
        for proxied in if i % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let started = Instant::now();
            let response=client.post(if proxied{&proxy}else{&direct}).json(&json!({"model":if proxied{format!("{}/model",account.provider)}else{"model".into()},"messages":[{"role":"user","content":"hello"}]})).send().await.unwrap();
            assert_eq!(response.status(), 200);
            response.bytes().await.unwrap();
            if i >= 20 {
                if proxied {
                    &mut proxy_us
                } else {
                    &mut direct_us
                }
                .push(started.elapsed().as_micros());
            }
        }
    }
    direct_us.sort_unstable();
    proxy_us.sort_unstable();
    let direct = direct_us[direct_us.len() / 2];
    let proxy = proxy_us[proxy_us.len() / 2];
    let overhead = proxy.saturating_sub(direct);
    println!(
        "native latency samples=120 per path; direct_p50_us={direct}; proxy_p50_us={proxy}; overhead_us={overhead}"
    );
    assert!(overhead < 2000);
    handle.shutdown().await;
    mock.abort();
}
