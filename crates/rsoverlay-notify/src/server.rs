//! WebSocket server for the notification API.
//!
//! Binds `127.0.0.1:<port>` (default 42069), accepts both plain-JSON POSTs
//! over WebSocket (XSOverlay-style — one notification per frame) and
//! JSON-as-text. Loopback-only, no auth required for local origins.
//!
//! Received notifications are pushed through a bounded channel so the UI
//! thread can drain them on its own cadence without blocking the socket.

use crate::Notification;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

#[derive(Clone, Debug)]
pub struct NotifyConfig {
    pub bind: SocketAddr,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:42069".parse().expect("hardcoded addr valid"),
        }
    }
}

/// Receiver side returned to the UI thread. Drain with `try_recv` from the
/// render loop — never blocks.
pub struct NotifyChannel {
    pub rx: mpsc::UnboundedReceiver<Notification>,
}

/// Spawn the notification server on the given tokio runtime handle.
///
/// Returns a receiver that yields notifications in arrival order. The task
/// exits when the runtime is dropped or the TcpListener fails to bind — the
/// error is logged and the receiver simply never produces.
pub fn spawn(rt: &tokio::runtime::Handle, cfg: NotifyConfig) -> NotifyChannel {
    let (tx, rx) = mpsc::unbounded_channel();
    rt.spawn(async move {
        if let Err(e) = run(cfg, tx).await {
            warn!("notification server exited: {e:#}");
        }
    });
    NotifyChannel { rx }
}

async fn run(cfg: NotifyConfig, tx: mpsc::UnboundedSender<Notification>) -> Result<()> {
    let listener = TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("binding notification server on {}", cfg.bind))?;
    info!("notification server listening on ws://{}/api/v1/send", cfg.bind);
    loop {
        let (stream, peer) = listener.accept().await?;
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer, tx).await {
                debug!("peer {peer} disconnected: {e:#}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    tx: mpsc::UnboundedSender<Notification>,
) -> Result<()> {
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .context("websocket handshake")?;
    debug!("peer connected: {peer}");
    let (_sink, mut stream) = ws.split();
    while let Some(msg) = stream.next().await {
        let msg = msg?;
        match msg {
            Message::Text(json) => match serde_json::from_str::<Notification>(&json) {
                Ok(n) => {
                    if tx.send(n).is_err() {
                        break;
                    }
                }
                Err(e) => warn!("rejecting malformed notification from {peer}: {e}"),
            },
            Message::Binary(bytes) => match serde_json::from_slice::<Notification>(&bytes) {
                Ok(n) => {
                    if tx.send(n).is_err() {
                        break;
                    }
                }
                Err(e) => warn!("rejecting malformed binary notification from {peer}: {e}"),
            },
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => break,
            Message::Frame(_) => {}
        }
    }
    Ok(())
}
