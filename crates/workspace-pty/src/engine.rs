//! A device exposes the same terminal API regardless of who owns its shells.
use crate::{Event, persistent};
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

pub use persistent::Manage;

#[derive(Clone)]
pub enum Engine {
    Native(persistent::Engine),
    #[cfg(feature = "herdr")]
    Herdr(crate::herdr::Engine),
}
impl Engine {
    pub fn new(home: PathBuf) -> Self {
        Self::Native(persistent::Engine::new(home))
    }
    pub async fn manage(&self, command: Manage) -> Result<Value> {
        match self {
            Self::Native(engine) => engine.manage(command).await,
            #[cfg(feature = "herdr")]
            Self::Herdr(engine) => engine.manage(command).await,
        }
    }
    pub async fn attach(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
        takeover: bool,
    ) -> Result<Attachment> {
        match self {
            Self::Native(engine) => Ok(Attachment::Native(
                engine.attach(id, cols, rows, takeover).await?,
            )),
            #[cfg(feature = "herdr")]
            Self::Herdr(engine) => Ok(Attachment::Herdr(
                engine.attach(id, cols, rows, takeover).await?,
            )),
        }
    }
    pub async fn shutdown(&self) {
        match self {
            Self::Native(engine) => engine.shutdown().await,
            // The external daemon owns these shells across server restarts.
            #[cfg(feature = "herdr")]
            Self::Herdr(_) => {}
        }
    }
}

pub enum Attachment {
    Native(persistent::Attachment),
    #[cfg(feature = "herdr")]
    Herdr(crate::Session),
}
impl Attachment {
    pub fn take_replay(&mut self) -> Vec<Vec<u8>> {
        match self {
            Self::Native(attachment) => std::mem::take(&mut attachment.replay),
            // herdr replays its screen through the attachment stream.
            #[cfg(feature = "herdr")]
            Self::Herdr(_) => Vec::new(),
        }
    }
    pub fn write(&self, data: Vec<u8>) -> Result<()> {
        match self {
            Self::Native(attachment) => attachment.write(data),
            #[cfg(feature = "herdr")]
            Self::Herdr(attachment) => attachment.write(data),
        }
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        match self {
            Self::Native(attachment) => attachment.resize(cols, rows),
            #[cfg(feature = "herdr")]
            Self::Herdr(attachment) => attachment.resize(cols, rows),
        }
    }
    pub async fn recv(&mut self) -> Option<Event> {
        match self {
            Self::Native(attachment) => attachment.output.recv().await,
            #[cfg(feature = "herdr")]
            Self::Herdr(attachment) => attachment.output.recv().await,
        }
    }
}
