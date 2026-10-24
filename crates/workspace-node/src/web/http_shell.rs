//! Loopback bridge between the versioned node and its desktop shell.
use super::standalone::Shell;
use anyhow::{Context, Result, ensure};
use reqwest::{Client, Method, Response, redirect::Policy};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use url::{Host, Url};

fn loopback_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("Invalid loopback URL")?;
    ensure!(
        url.scheme() == "http"
            && match url.host() {
                Some(Host::Ipv4(ip)) => ip.is_loopback(),
                Some(Host::Ipv6(ip)) => ip.is_loopback(),
                _ => false,
            },
        "A literal loopback HTTP address is required"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URL credentials are not allowed"
    );
    Ok(url)
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(15))
        .build()?)
}

pub struct HttpShell {
    client: Client,
    url: Url,
    token: String,
}
impl HttpShell {
    pub fn new(base: &str, token: String) -> Result<Self> {
        let mut url = loopback_url(base)?;
        ensure!(
            url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
            "Shell URL must be an origin"
        );
        ensure!(!token.is_empty(), "Shell token is required");
        url.set_path("/shell");
        Ok(Self {
            client: client()?,
            url,
            token,
        })
    }

    pub fn from_env() -> Result<Option<Arc<dyn Shell>>> {
        match (
            std::env::var_os("CRANIBOX_SHELL_URL"),
            std::env::var_os("CRANIBOX_SHELL_TOKEN"),
        ) {
            (Some(url), Some(token)) => Ok(Some(Arc::new(Self::new(
                url.to_str().context("Invalid shell URL encoding")?,
                token
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("Invalid shell token encoding"))?,
            )?))),
            _ => Ok(None),
        }
    }
}
#[async_trait::async_trait]
impl Shell for HttpShell {
    async fn call(&self, operation: &str, body: Value) -> Result<Value> {
        let response = self
            .client
            .post(self.url.clone())
            .bearer_auth(&self.token)
            .json(&json!({"operation": operation, "body": body}))
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        json_response(response).await
    }
}

async fn json_response(response: Response) -> Result<Value> {
    let status = response.status();
    let value: Value = response.json().await.map_err(reqwest::Error::without_url)?;
    ensure!(
        status.is_success(),
        "Local API ({status}): {}",
        value["error"]
            .as_str()
            .or_else(|| value["message"].as_str())
            .unwrap_or("Request failed")
    );
    Ok(value)
}

/// A desktop tray uses the same cookie and CSRF checks as the local page.
pub struct LocalInterface {
    client: Client,
    base: Url,
    cookie: String,
    csrf: String,
}
impl LocalInterface {
    pub async fn connect(sign_in: &str) -> Result<Self> {
        let mut base = loopback_url(sign_in)?;
        let client = client()?;
        let response = client
            .get(base.clone())
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        ensure!(
            response.status() == reqwest::StatusCode::SEE_OTHER,
            "Local interface sign-in failed"
        );
        let cookie = response
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .context("Local interface did not issue a session")?
            .to_owned();
        base.set_path("/");
        base.set_query(None);
        base.set_fragment(None);
        let page = client
            .get(base.clone())
            .header(reqwest::header::COOKIE, &cookie)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?
            .text()
            .await
            .map_err(reqwest::Error::without_url)?;
        let config = page
            .split_once("window.workspaceWeb=")
            .and_then(|(_, rest)| rest.split_once(";</script>"))
            .context("Local interface has no page configuration")?
            .0;
        let config: Value =
            serde_json::from_str(config).context("Invalid local page configuration")?;
        let csrf = config["csrf"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("Local interface has no CSRF token")?
            .to_owned();
        Ok(Self {
            client,
            base,
            cookie,
            csrf,
        })
    }

    /// `path` is a local API path, never an origin or an externally supplied URL.
    pub async fn request(&self, method: &str, path: &str, body: Value) -> Result<Value> {
        ensure!(
            path.starts_with("/api/") && !path.contains(['?', '#']),
            "Local API path required"
        );
        let mut url = self.base.clone();
        url.set_path(path);
        let method: Method = method.parse()?;
        let mut request = self
            .client
            .request(method.clone(), url)
            .header(reqwest::header::COOKIE, &self.cookie)
            .header("x-workspace-csrf", &self.csrf);
        if method != Method::GET {
            request = request.json(&body);
        }
        json_response(request.send().await.map_err(reqwest::Error::without_url)?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_non_loopback_and_non_http_urls() {
        for url in [
            "https://127.0.0.1",
            "http://192.0.2.1",
            "http://example.com",
            "http://localhost",
            "http://127.0.0.1.example.com",
            "http://[::]",
            "http://user@127.0.0.1",
            "http://127.0.0.1/path",
            "http://127.0.0.1/?token=x",
        ] {
            assert!(HttpShell::new(url, "test".into()).is_err(), "{url}");
        }
        assert!(HttpShell::new("http://127.0.0.1:1234", "test".into()).is_ok());
        assert!(HttpShell::new("http://[::1]:1234", "test".into()).is_ok());
    }
}
