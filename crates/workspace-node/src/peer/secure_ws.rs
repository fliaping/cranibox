//! Server WebSocket that encrypts data frames end to end for streams opened through an
//! end-to-end session (see `agent_workspace_runtime::peers::e2e::stream`), and is a
//! transparent pass-through for local and bearer-authorized streams.
use agent_workspace_runtime::peers::e2e::stream::{Cipher, Data, OVERHEAD, Secret};
use axum::extract::ws::{Message, WebSocket};

/// Stream secret recovered from a verified stream proof, attached to the upgrade request.
#[derive(Clone)]
pub struct StreamSecret {
    pub secret: Secret,
    pub path: String,
}

pub struct SecureSocket {
    inner: WebSocket,
    cipher: Option<Cipher>,
}
/// Extra bytes to allow per message when the stream may be encrypted.
pub const MARGIN: usize = OVERHEAD;

impl SecureSocket {
    pub fn plain(inner: WebSocket) -> Self {
        Self {
            inner,
            cipher: None,
        }
    }
    pub fn new(inner: WebSocket, secret: Option<&StreamSecret>) -> Self {
        Self {
            inner,
            cipher: secret.map(|s| Cipher::target(&s.secret, &s.path)),
        }
    }
    pub async fn recv(&mut self) -> Option<Result<Message, axum::Error>> {
        let message = match self.inner.recv().await? {
            Ok(message) => message,
            Err(error) => return Some(Err(error)),
        };
        let Some(cipher) = &mut self.cipher else {
            return Some(Ok(message));
        };
        Some(match message {
            Message::Binary(frame) => match cipher.open(&frame) {
                Ok(Data::Text(text)) => Ok(Message::Text(text.into())),
                Ok(Data::Binary(bytes)) => Ok(Message::Binary(bytes.into())),
                Err(error) => Err(axum::Error::new(error)),
            },
            Message::Text(_) => Err(axum::Error::new(anyhow::anyhow!(
                "plaintext data frame on an end-to-end encrypted stream"
            ))),
            other => Ok(other),
        })
    }
    pub async fn send(&mut self, message: Message) -> Result<(), axum::Error> {
        let message = match (&mut self.cipher, message) {
            (Some(cipher), Message::Text(text)) => Message::Binary(
                cipher
                    .seal(&Data::Text(text.as_str().to_owned()))
                    .map_err(axum::Error::new)?
                    .into(),
            ),
            (Some(cipher), Message::Binary(bytes)) => Message::Binary(
                cipher
                    .seal(&Data::Binary(bytes.to_vec()))
                    .map_err(axum::Error::new)?
                    .into(),
            ),
            // Close reasons are application text; drop them on encrypted streams.
            (Some(_), Message::Close(_)) => Message::Close(None),
            (_, other) => other,
        };
        self.inner.send(message).await
    }
}
