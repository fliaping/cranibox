use super::*;
use cranibox_contracts::vault::StoreError;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct FlakyStore {
    inner: MemoryAccountStore,
    fail: AtomicBool,
}
impl AccountStore for FlakyStore {
    fn list(&self) -> Vec<AccountSummary> {
        self.inner.list()
    }
    fn get(&self, id: &str) -> Option<AccountSummary> {
        self.inner.get(id)
    }
    fn get_secret(&self, id: &str) -> Result<SecretBox, StoreError> {
        self.inner.get_secret(id)
    }
    fn remove(&self, id: &str) -> Result<(), StoreError> {
        self.inner.remove(id)
    }
    fn put(&self, account: AccountSummary, secret: Option<SecretBox>) -> Result<(), StoreError> {
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(StoreError::Other("temporary test write failure".into()));
        }
        self.inner.put(account, secret)
    }
}

async fn start(oauth: &OAuth) -> (String, url::Url) {
    browser(
        oauth
            .start(
                "codex",
                SignInOpts {
                    region: None,
                    label: None,
                },
            )
            .await
            .unwrap(),
    )
}

async fn listener_closed(url: &url::Url) {
    let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let redirect: url::Url = pairs["redirect_uri"].parse().unwrap();
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", redirect.port().unwrap()))
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("callback listener remained open");
}

fn denial(url: &url::Url, state: Option<&str>) -> String {
    let mut url: url::Url = callback(url, state).parse().unwrap();
    let mut pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
    pairs.remove("code");
    pairs.insert("error".into(), "access_denied".into());
    pairs.insert(
        "error_description".into(),
        "private-provider-message".into(),
    );
    url.set_query(None);
    url.query_pairs_mut().extend_pairs(pairs);
    url.into()
}

#[tokio::test]
async fn browser_denial_requires_state_and_ends_flow_without_exchanging_tokens() {
    let mock = Mock::new().await;
    let store = Arc::new(MemoryAccountStore::default());
    let oauth = OAuth::new(
        "local".into(),
        store.clone(),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    let (id, url) = start(&oauth).await;
    assert_eq!(
        reqwest::get(denial(&url, Some("wrong")))
            .await
            .unwrap()
            .status(),
        400
    );
    assert!(matches!(
        oauth.complete(&id, no_input()).await,
        Err(MgmtError::Pending)
    ));
    let response = reqwest::get(denial(&url, None)).await.unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    assert!(
        !response
            .text()
            .await
            .unwrap()
            .contains("private-provider-message")
    );
    listener_closed(&url).await;
    assert!(matches!(
        oauth.complete(&id, no_input()).await,
        Err(MgmtError::Invalid(_))
    ));
    assert!(mock.calls().is_empty());
    assert!(store.list().is_empty());
}

#[tokio::test]
async fn accepted_callback_releases_port_before_token_exchange() {
    let mock = Mock::new().await;
    mock.replies("/token", vec![(200, token())]);
    let store = Arc::new(MemoryAccountStore::default());
    let oauth = OAuth::new(
        "local".into(),
        store.clone(),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    let (id, url) = start(&oauth).await;
    let response = reqwest::get(callback(&url, None)).await.unwrap();
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    listener_closed(&url).await;
    assert!(mock.calls().is_empty());
    let account = oauth.complete(&id, no_input()).await.unwrap();
    oauth.cancel(&id).await.unwrap();
    assert_eq!(store.list().len(), 1);
    assert_eq!(
        oauth.complete(&id, no_input()).await.unwrap().id,
        account.id
    );
}

#[tokio::test]
async fn cancellation_releases_listener_and_stops_device_polling() {
    let mock = Mock::new().await;
    let store = Arc::new(MemoryAccountStore::default());
    let oauth = OAuth::new(
        "local".into(),
        store.clone(),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    let (id, url) = start(&oauth).await;
    oauth.cancel(&id).await.unwrap();
    listener_closed(&url).await;
    assert!(matches!(
        oauth.complete(&id, no_input()).await,
        Err(MgmtError::Invalid(_))
    ));
    let device = start_device(&mock, &oauth, "kimi", 600).await;
    oauth.cancel(&device).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert!(matches!(
        oauth.complete(&device, no_input()).await,
        Err(MgmtError::Invalid(_))
    ));
    assert_eq!(mock.calls().len(), 1); // device authorization only, no token poll
    assert!(store.list().is_empty());
}

#[tokio::test]
async fn dropping_last_oauth_owner_closes_pending_callback() {
    let mock = Mock::new().await;
    let oauth = OAuth::new(
        "local".into(),
        Arc::new(MemoryAccountStore::default()),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    let retained = oauth.clone();
    let (_, url) = start(&oauth).await;
    drop(oauth);
    assert_eq!(
        reqwest::get(callback(&url, Some("wrong")))
            .await
            .unwrap()
            .status(),
        400
    );
    drop(retained);
    listener_closed(&url).await;
}

#[tokio::test]
async fn pasted_denial_is_terminal_and_does_not_exchange_tokens() {
    let mock = Mock::new().await;
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let oauth = OAuth::new(
        "local".into(),
        Arc::new(MemoryAccountStore::default()),
        OAuthConfig::for_loopback_with_callback_port(
            &mock.base,
            occupied.local_addr().unwrap().port(),
        )
        .unwrap(),
    );
    let (id, url) = start(&oauth).await;
    assert!(matches!(
        oauth
            .complete(
                &id,
                SignInInput {
                    callback_url: Some(denial(&url, None)),
                    code: None
                }
            )
            .await,
        Err(MgmtError::Invalid(_))
    ));
    assert!(matches!(
        oauth
            .complete(
                &id,
                SignInInput {
                    callback_url: Some(callback(&url, None)),
                    code: None
                }
            )
            .await,
        Err(MgmtError::Invalid(_))
    ));
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn cancellation_preserves_exchanged_credentials_after_transient_vault_failure() {
    let mock = Mock::new().await;
    mock.replies("/token", vec![(200, token())]);
    let store = Arc::new(FlakyStore::default());
    store.fail.store(true, Ordering::SeqCst);
    let oauth = OAuth::new(
        "local".into(),
        store.clone(),
        OAuthConfig::for_loopback(&mock.base).unwrap(),
    );
    let (id, url) = start(&oauth).await;
    assert!(matches!(
        oauth
            .complete(
                &id,
                SignInInput {
                    callback_url: Some(callback(&url, None)),
                    code: None
                }
            )
            .await,
        Err(MgmtError::Other(_))
    ));
    assert!(matches!(
        oauth.cancel(&id).await,
        Err(MgmtError::Invalid(_))
    ));
    let account = oauth.complete(&id, no_input()).await.unwrap();
    assert_eq!(store.list().len(), 1);
    assert_eq!(
        Credential::decode(&store.get_secret(&account.id).unwrap())
            .unwrap()
            .refresh_token
            .as_deref(),
        Some("fake-refresh")
    );
    assert_eq!(
        mock.calls().len(),
        1,
        "authorization code must not be exchanged twice"
    );
}
