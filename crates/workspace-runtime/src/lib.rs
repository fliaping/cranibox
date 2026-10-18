#[cfg(feature = "server")]
pub mod cli;
#[cfg(feature = "client")]
pub mod client;
pub mod config;
#[cfg(feature = "server")]
pub mod journal;
#[cfg(feature = "client")]
pub mod peers;
pub mod projects;
pub mod protocol;
#[cfg(feature = "server")]
pub mod server;

pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
