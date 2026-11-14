//! The app <-> daemon protocol over the Unix socket.
//!
//! Frames are `[length: u32 BE][kind: u8][body]`:
//! - `S` sync message for a document: `[id length: u8][doc id][Automerge sync message]`
//! - `L` (daemon -> app) a whole document, answering `Open`: `[id length][doc id][saved doc]`
//! - `J` a [`Control`] message as JSON
//!
//! An app sends `Hello`, then `Open` for each document it wants. The daemon
//! answers with `L`, then both sides run Automerge's sync protocol with `S`
//! frames, exactly as between devices. The daemon pushes `Status` whenever
//! paired devices or pairing change.

use std::io::{Read, Write};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::Status;
use crate::net::{tagged, untag};

pub(crate) const VERSION: u32 = 1;
const MAX_FRAME: usize = 64 << 20;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum Control {
    Hello { version: u32 },
    Open { doc: String },
    StartPairing,
    CancelPairing,
    Pair { req: u64, code: String },
    Unpair { id: String },
    SyncNow,
    Status { status: Status },
    Paired { req: u64, result: std::result::Result<String, String> },
}

pub(crate) enum Frame {
    Sync { doc: String, msg: Vec<u8> },
    Load { doc: String, bytes: Vec<u8> },
    Control(Control),
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let (kind, body) = match self {
            Frame::Sync { doc, msg } => (b'S', tagged(doc, msg)),
            Frame::Load { doc, bytes } => (b'L', tagged(doc, bytes)),
            Frame::Control(c) => (b'J', serde_json::to_vec(c).unwrap()),
        };
        let mut out = Vec::with_capacity(5 + body.len());
        out.extend_from_slice(&(body.len() as u32 + 1).to_be_bytes());
        out.push(kind);
        out.extend_from_slice(&body);
        out
    }

    /// Decodes a frame without its length prefix.
    pub fn decode(frame: &[u8]) -> Result<Frame> {
        let Some((&kind, body)) = frame.split_first() else { bail!("empty frame") };
        Ok(match kind {
            b'S' | b'L' => {
                let (doc, data) = untag(body)?;
                let (doc, data) = (doc.to_string(), data.to_vec());
                if kind == b'S' { Frame::Sync { doc, msg: data } } else { Frame::Load { doc, bytes: data } }
            }
            b'J' => Frame::Control(serde_json::from_slice(body)?),
            _ => bail!("unknown frame kind {kind}"),
        })
    }
}

/// Blocking read of one frame (without its length prefix).
pub(crate) fn read_frame(r: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

pub(crate) fn write_frame(w: &mut impl Write, frame: &Frame) -> std::io::Result<()> {
    w.write_all(&frame.encode())
}

pub(crate) async fn read_frame_async(r: &mut (impl tokio::io::AsyncRead + Unpin)) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let len = r.read_u32().await? as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}
