//! Peer wire framing (BEP 3, BEP 10).

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::error::wire;
use crate::{Error, InfoHash};

pub(crate) const MSG_CHOKE: u8 = 0;
pub(crate) const MSG_UNCHOKE: u8 = 1;
pub(crate) const MSG_INTERESTED: u8 = 2;
pub(crate) const MSG_NOT_INTERESTED: u8 = 3;
pub(crate) const MSG_HAVE: u8 = 4;
pub(crate) const MSG_BITFIELD: u8 = 5;
pub(crate) const MSG_REQUEST: u8 = 6;
pub(crate) const MSG_PIECE: u8 = 7;
pub(crate) const MSG_CANCEL: u8 = 8;
pub(crate) const MSG_EXTENDED: u8 = 20;

const PROTOCOL: &[u8] = b"BitTorrent protocol";
pub(crate) const BLOCK_SIZE: usize = 16 << 10;
/// The largest block we serve; larger requests drop the peer.
pub(crate) const MAX_BLOCK_REQUEST: usize = 32 << 10;
/// Bounds any message except a bitfield.
pub(crate) const MAX_MESSAGE: usize = 1 << 20;

/// The 68-byte opening exchange.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Handshake {
    pub reserved: [u8; 8],
    pub info_hash: InfoHash,
    pub peer_id: [u8; 20],
}

impl Handshake {
    pub fn supports_extensions(&self) -> bool {
        self.reserved[5] & 0x10 != 0
    }
}

pub(crate) fn handshake_bytes(ih: &InfoHash, id: &[u8; 20]) -> Vec<u8> {
    let mut b = Vec::with_capacity(68);
    b.push(PROTOCOL.len() as u8);
    b.extend_from_slice(PROTOCOL);
    let mut reserved = [0u8; 8];
    reserved[5] |= 0x10; // BEP 10 extension protocol
    b.extend_from_slice(&reserved);
    b.extend_from_slice(&ih.0);
    b.extend_from_slice(id);
    b
}

pub(crate) async fn read_handshake<R: AsyncRead + Unpin>(r: &mut R) -> Result<Handshake, Error> {
    let mut buf = [0u8; 68];
    r.read_exact(&mut buf).await?;
    if buf[0] as usize != PROTOCOL.len() || &buf[1..20] != PROTOCOL {
        return Err(wire("not a BitTorrent handshake"));
    }
    let mut h = Handshake { reserved: [0; 8], info_hash: InfoHash::default(), peer_id: [0; 20] };
    h.reserved.copy_from_slice(&buf[20..28]);
    h.info_hash.0.copy_from_slice(&buf[28..48]);
    h.peer_id.copy_from_slice(&buf[48..68]);
    Ok(h)
}

/// One length-prefixed frame; keep-alives are not returned.
pub(crate) struct Message {
    pub id: u8,
    pub payload: Vec<u8>,
}

/// Reads the next non-keepalive message; `limit` bounds the frame.
pub(crate) async fn read_message<R: AsyncRead + Unpin>(r: &mut R, limit: usize) -> Result<Message, Error> {
    loop {
        let n = r.read_u32().await? as usize;
        if n == 0 {
            continue;
        }
        if n > limit {
            return Err(wire(format!("message of {n} bytes exceeds {limit}")));
        }
        let mut buf = vec![0u8; n];
        r.read_exact(&mut buf).await?;
        let payload = buf.split_off(1);
        return Ok(Message { id: buf[0], payload });
    }
}

pub(crate) fn frame(id: u8, parts: &[&[u8]]) -> Vec<u8> {
    let n = 1 + parts.iter().map(|p| p.len()).sum::<usize>();
    let mut b = Vec::with_capacity(4 + n);
    b.extend_from_slice(&(n as u32).to_be_bytes());
    b.push(id);
    for p in parts {
        b.extend_from_slice(p);
    }
    b
}

pub(crate) fn u32s(vals: &[u32]) -> Vec<u8> {
    vals.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// One requested block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BlockRef {
    pub piece: usize,
    pub begin: usize,
    pub length: usize,
}

pub(crate) fn parse_block_ref(p: &[u8]) -> Result<BlockRef, Error> {
    if p.len() != 12 {
        return Err(wire(format!("request/cancel of {} bytes", p.len())));
    }
    let at = |i: usize| u32::from_be_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]) as usize;
    Ok(BlockRef { piece: at(0), begin: at(4), length: at(8) })
}

pub(crate) fn request_frame(id: u8, b: BlockRef) -> Vec<u8> {
    frame(id, &[&u32s(&[b.piece as u32, b.begin as u32, b.length as u32])])
}
