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

/// Anything the sender task can put on the wire.
#[derive(Clone, Debug)]
pub enum Outbound {
    Chat(ChatMessage),
    /// `/avatar/parameters/<name>` with a single value.
    Param { name: String, value: OscValue },
    /// `/input/<name>` (VRChat's input endpoints, e.g. `Voice`).
    Input { name: String, value: OscValue },
}

pub struct OscClient {
    tx: mpsc::UnboundedSender<Outbound>,
}

impl OscClient {
    pub fn send_chat(&self, msg: ChatMessage) -> Result<()> {
        self.tx.send(Outbound::Chat(msg)).map_err(|_| anyhow::anyhow!("OSC sender stopped"))
    }

    /// Press VRChat's mic button once (`/input/Voice` 0 → 1 → 0): with
    /// the default Toggle Voice this flips mute.
    pub fn toggle_voice(&self) -> Result<()> {
        for v in [0, 1, 0] {
            self.tx
                .send(Outbound::Input { name: "Voice".into(), value: OscValue::Int32(v) })
                .map_err(|_| anyhow::anyhow!("OSC sender stopped"))?;
        }
        Ok(())
    }

    pub fn send_param(&self, name: impl Into<String>, value: OscValue) -> Result<()> {
        self.tx
            .send(Outbound::Param { name: name.into(), value })
            .map_err(|_| anyhow::anyhow!("OSC sender stopped"))
    }
}

/// Spawn a background sender on the given tokio runtime. Returns a sender
/// handle; drop it to stop the task.
pub fn spawn(rt: &tokio::runtime::Handle, addr: SocketAddr) -> OscClient {
    let (tx, mut rx) = mpsc::unbounded_channel::<Outbound>();
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
        while let Some(out) = rx.recv().await {
            let res = match &out {
                Outbound::Chat(msg) => send_chatbox(&sock, msg).await,
                Outbound::Param { name, value } => {
                    let bytes = encode_message(&format!("/avatar/parameters/{name}"), std::slice::from_ref(value));
                    sock.send(&bytes).await.map(|_| ()).context("UDP send")
                }
                Outbound::Input { name, value } => {
                    let bytes = encode_message(&format!("/input/{name}"), std::slice::from_ref(value));
                    // VRChat reads a button press across frames: a short gap.
                    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                    sock.send(&bytes).await.map(|_| ()).context("UDP send")
                }
            };
            match res {
                Err(e) => warn!("osc send: {e:#}"),
                Ok(()) => debug!("osc sent {out:?}"),
            }
        }
    });
    OscClient { tx }
}

/// VRChat's mic state as it last reported it over OSC.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MicState {
    /// Nothing heard yet (VRChat closed, OSC off, or port taken).
    Unknown,
    Live,
    Muted,
}

/// Listens on VRChat's OSC output port for `/avatar/parameters/MuteSelf`
/// while enabled (the port is released when disabled, for other OSC apps).
#[derive(Clone)]
pub struct MuteWatch {
    state: std::sync::Arc<std::sync::atomic::AtomicU8>,
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl MuteWatch {
    pub fn state(&self) -> MicState {
        match self.state.load(std::sync::atomic::Ordering::Relaxed) {
            1 => MicState::Live,
            2 => MicState::Muted,
            _ => MicState::Unknown,
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, std::sync::atomic::Ordering::Relaxed);
    }
}

/// VRChat sends OSC to this port by default.
pub const VRCHAT_OUT_PORT: u16 = 9001;

/// Listen for VRChat's mute state on `127.0.0.1:port` whenever enabled.
/// If the port is taken (another OSC app), the state stays Unknown.
pub fn watch_mute(rt: &tokio::runtime::Handle, port: u16, enabled: bool) -> MuteWatch {
    use std::sync::atomic::Ordering;
    let w = MuteWatch { state: Default::default(), enabled: std::sync::Arc::new(enabled.into()) };
    let (state, on) = (w.state.clone(), w.enabled.clone());
    rt.spawn(async move {
        let mut buf = vec![0u8; 4096];
        let mut warned = false;
        loop {
            if !on.load(Ordering::Relaxed) {
                state.store(0, Ordering::Relaxed);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            let sock = match UdpSocket::bind(("127.0.0.1", port)).await {
                Ok(s) => {
                    info!("osc: listening on {port} for VRChat's mute state");
                    warned = false;
                    s
                }
                Err(e) => {
                    if !warned {
                        info!("osc: can't listen on {port} ({e}); VRChat mute state unknown");
                        warned = true;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    continue;
                }
            };
            while on.load(Ordering::Relaxed) {
                match tokio::time::timeout(std::time::Duration::from_secs(1), sock.recv(&mut buf)).await {
                    Ok(Ok(n)) => {
                        if let Some(muted) = mute_self(&buf[..n]) {
                            state.store(if muted { 2 } else { 1 }, Ordering::Relaxed);
                        }
                    }
                    Ok(Err(_)) => break,
                    Err(_) => {}
                }
            }
        }
    });
    w
}

/// `MuteSelf` from one OSC packet (a message or a bundle of them).
pub fn mute_self(p: &[u8]) -> Option<bool> {
    if p.starts_with(b"#bundle\0") {
        // "#bundle\0", 8-byte timetag, then (size, element)*.
        let mut i = 16;
        let mut found = None;
        while i + 4 <= p.len() {
            let n = u32::from_be_bytes(p[i..i + 4].try_into().ok()?) as usize;
            let el = p.get(i + 4..i + 4 + n)?;
            found = mute_self(el).or(found);
            i += 4 + n;
        }
        return found;
    }
    let (addr, rest) = read_osc_str(p)?;
    if addr != "/avatar/parameters/MuteSelf" {
        return None;
    }
    let (tags, args) = read_osc_str(rest)?;
    match tags.as_bytes().get(1)? {
        b'T' => Some(true),
        b'F' => Some(false),
        b'i' => Some(i32::from_be_bytes(args.get(..4)?.try_into().ok()?) != 0),
        b'f' => Some(f32::from_be_bytes(args.get(..4)?.try_into().ok()?) != 0.0),
        _ => None,
    }
}

/// A null-terminated, 4-byte padded OSC string and what follows it.
fn read_osc_str(p: &[u8]) -> Option<(&str, &[u8])> {
    let end = p.iter().position(|b| *b == 0)?;
    let s = std::str::from_utf8(&p[..end]).ok()?;
    let next = (end + 4) & !3;
    Some((s, p.get(next..)?))
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
    fn reads_vrchat_mute_state_from_messages_and_bundles() {
        let m = |v: OscValue| encode_message("/avatar/parameters/MuteSelf", &[v]);
        assert_eq!(mute_self(&m(OscValue::Bool(true))), Some(true));
        assert_eq!(mute_self(&m(OscValue::Bool(false))), Some(false));
        assert_eq!(mute_self(&m(OscValue::Int32(1))), Some(true));
        assert_eq!(mute_self(&encode_message("/avatar/parameters/Voice", &[OscValue::Float32(0.2)])), None);
        let mut b = b"#bundle\0".to_vec();
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        for msg in [encode_message("/avatar/parameters/Viseme", &[OscValue::Int32(3)]), m(OscValue::Bool(false))] {
            b.extend_from_slice(&(msg.len() as u32).to_be_bytes());
            b.extend_from_slice(&msg);
        }
        assert_eq!(mute_self(&b), Some(false));
    }

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
