//! Hand-rolled Open Sound Control (OSC) 1.0 encoder, plus a small VRChat
//! chatbox sender. OSC is tiny (null-padded strings, big-endian, 4-byte
//! alignment) so we don't pull in `rosc` just for the one message type we
//! care about.
//!
//! VRChat input OSC listens on UDP `127.0.0.1:9000` by default; see
//! <https://docs.vrchat.com/docs/osc-overview>.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub const VRCHAT_DEFAULT_ADDR: &str = "127.0.0.1:9000";

#[derive(Clone, Debug)]
pub enum OscValue {
    String(String),
    Bool(bool),
    Int32(i32),
    Float32(f32),
}

/// Encode a single OSC 1.0 message to a byte vector.
pub fn encode_message(address: &str, args: &[OscValue]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    push_osc_string(&mut buf, address);
    let mut type_tag = String::from(",");
    for a in args {
        type_tag.push(match a {
            OscValue::String(_) => 's',
            // OSC 1.0 has no native bool — VRChat uses the T/F tag types
            // which carry zero bytes of payload.
            OscValue::Bool(true) => 'T',
            OscValue::Bool(false) => 'F',
            OscValue::Int32(_) => 'i',
            OscValue::Float32(_) => 'f',
        });
    }
    push_osc_string(&mut buf, &type_tag);
    for a in args {
        match a {
            OscValue::String(s) => push_osc_string(&mut buf, s),
            OscValue::Bool(_) => {} // payload-less
            OscValue::Int32(v) => buf.extend_from_slice(&v.to_be_bytes()),
            OscValue::Float32(v) => buf.extend_from_slice(&v.to_be_bytes()),
        }
    }
    buf
}

fn push_osc_string(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    // Trailing null + zero pad to 4-byte alignment.
    buf.push(0);
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

/// Outbound chatbox message.
#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub text: String,
    pub send_immediately: bool,
    pub play_notification: bool,
}

pub struct OscClient {
    pub tx: mpsc::UnboundedSender<ChatMessage>,
}

/// Spawn a background sender on the given tokio runtime. Returns a sender
/// handle; drop it to stop the task.
pub fn spawn(rt: &tokio::runtime::Handle, addr: SocketAddr) -> OscClient {
    let (tx, mut rx) = mpsc::unbounded_channel::<ChatMessage>();
    rt.spawn(async move {
        let sock = match UdpSocket::bind("127.0.0.1:0").await {
            Ok(s) => s,
            Err(e) => {
                warn!("osc: binding local UDP failed: {e:#}");
                return;
            }
        };
        if let Err(e) = sock.connect(addr).await {
            warn!("osc: connect to {addr} failed: {e:#}");
            return;
        }
        info!("osc chatbox sender ready → {addr}");
        while let Some(msg) = rx.recv().await {
            if let Err(e) = send_chatbox(&sock, &msg).await {
                warn!("osc send: {e:#}");
            } else {
                debug!("osc chatbox sent: {} bytes", msg.text.len());
            }
        }
    });
    OscClient { tx }
}

async fn send_chatbox(sock: &UdpSocket, msg: &ChatMessage) -> Result<()> {
    let bytes = encode_message(
        "/chatbox/input",
        &[
            OscValue::String(msg.text.clone()),
            OscValue::Bool(msg.send_immediately),
            OscValue::Bool(msg.play_notification),
        ],
    );
    sock.send(&bytes).await.context("UDP send")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatbox_encoding_shape() {
        let b = encode_message(
            "/chatbox/input",
            &[
                OscValue::String("hi".into()),
                OscValue::Bool(true),
                OscValue::Bool(false),
            ],
        );
        // "/chatbox/input" is 14 bytes + null = 15, padded to 16.
        assert_eq!(&b[..16], b"/chatbox/input\0\0");
        // Type tag ",sTF" + null = 5 bytes, padded to 8.
        assert_eq!(&b[16..24], b",sTF\0\0\0\0");
        // "hi" + null = 3 bytes, padded to 4.
        assert_eq!(&b[24..28], b"hi\0\0");
        assert_eq!(b.len(), 28);
    }
}
