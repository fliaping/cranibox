//! A client WebSocket that encrypts data frames end to end when the stream was opened
//! through an end-to-end session, and behaves exactly like the raw socket otherwise.
use super::stream::{Cipher, Data};
use futures_util::{Sink, Stream};
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use tokio_tungstenite::tungstenite::{Error, Message, protocol::CloseFrame};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Raw = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub struct SecureSocket {
    inner: Raw,
    cipher: Option<Cipher>,
}
fn invalid(error: anyhow::Error) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        error.to_string(),
    ))
}
impl SecureSocket {
    pub fn new(inner: Raw, cipher: Option<Cipher>) -> Self {
        Self { inner, cipher }
    }
    pub fn encrypted(&self) -> bool {
        self.cipher.is_some()
    }
    pub async fn close(&mut self, frame: Option<CloseFrame>) -> Result<(), Error> {
        self.inner.close(frame).await
    }
    fn seal(&mut self, message: Message) -> Result<Message, Error> {
        let Some(cipher) = &mut self.cipher else {
            return Ok(message);
        };
        let data = match message {
            Message::Text(text) => Data::Text(text.as_str().to_owned()),
            Message::Binary(bytes) => Data::Binary(bytes.to_vec()),
            // Control frames carry no application data.
            Message::Close(_) => return Ok(Message::Close(None)),
            other => return Ok(other),
        };
        Ok(Message::Binary(cipher.seal(&data).map_err(invalid)?.into()))
    }
    fn open(&mut self, message: Message) -> Result<Message, Error> {
        let Some(cipher) = &mut self.cipher else {
            return Ok(message);
        };
        match message {
            Message::Binary(frame) => Ok(match cipher.open(&frame).map_err(invalid)? {
                Data::Text(text) => Message::Text(text.into()),
                Data::Binary(bytes) => Message::Binary(bytes.into()),
            }),
            Message::Text(_) | Message::Frame(_) => Err(invalid(anyhow::anyhow!(
                "plaintext data frame on an end-to-end encrypted stream"
            ))),
            other => Ok(other),
        }
    }
}
impl Stream for SecureSocket {
    type Item = Result<Message, Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let next = ready!(Pin::new(&mut self.inner).poll_next(cx));
        Poll::Ready(next.map(|item| item.and_then(|message| self.open(message))))
    }
}
impl Sink<Message> for SecureSocket {
    type Error = Error;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(&mut self.inner).poll_ready(cx)
    }
    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Error> {
        let sealed = self.seal(item)?;
        Pin::new(&mut self.inner).start_send(sealed)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}
