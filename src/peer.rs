use std::collections::HashSet;
use std::fmt;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time;

use crate::PEER_ID;
use crate::torrent::Block;
use crate::util::easy_err;

const BITTORRENT_PROTOCOL: &str = "BitTorrent protocol";
pub static KEEP_ALIVE_MAX_DURATION: time::Duration = time::Duration::from_secs(120);

#[derive(Debug)]
pub enum MessageType {
    KeepAlive = -1,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have,
    Bitfield,
    Request,
    Piece,
    Cancel,
    Port,
}

impl MessageType {
    pub fn from_u8(u: u8) -> Option<MessageType> {
        match u {
            0 => Some(MessageType::Choke),
            1 => Some(MessageType::Unchoke),
            2 => Some(MessageType::Interested),
            3 => Some(MessageType::NotInterested),
            4 => Some(MessageType::Have),
            5 => Some(MessageType::Bitfield),
            6 => Some(MessageType::Request),
            7 => Some(MessageType::Piece),
            8 => Some(MessageType::Cancel),
            9 => Some(MessageType::Port),
            10.. => None,
        }
    }
}

pub struct Message {
    pub message_type: MessageType,
    pub payload: Vec<u8>,
}

pub struct Peer {
    pub ip_address: u32,
    pub port: u16,
    conn: Option<TcpStream>,
    addr: SocketAddr,
    pub am_choked: bool,
    pub am_interested: bool,
    pub peer_choked: bool,
    pub peer_interested: bool,
    pub peer_id: Option<[u8; 20]>,

    // List of piece indexes
    pub peer_has: HashSet<u32>,
    pub block_movements: Vec<BlockMovement>,

    pub last_message_at: Option<time::Instant>,
}

enum BlockDirection {
    UploadedToPeer,
    DownloadedFromPeer,
}

struct BlockMovement {
    block: Block,
    when: time::Instant,
    direction: BlockDirection,
}

impl Peer {
    pub fn new(ip_address: u32, port: u16) -> Self {
        Self {
            ip_address: ip_address,
            port: port,
            conn: None,
            addr: ip_to_socket_addr(ip_address, port),
            // https://wiki.theory.org/BitTorrentSpecification#Overview
            am_choked: false,
            am_interested: false,
            peer_choked: false,
            peer_interested: false,
            peer_id: None,
            peer_has: HashSet::new(),
            last_message_at: None,
            block_movements: Vec::new(),
        }
    }

    // TODO: fn accept(self: &Self) {}

    pub fn connect(self: &mut Self) -> Result<(), io::Error> {
        let c = TcpStream::connect_timeout(&self.addr, time::Duration::from_secs(3))?;
        self.conn = Some(c);
        self.configure_connection()?;
        Ok(())
    }

    pub fn disconnect(self: &mut Self) -> Result<(), io::Error> {
        if let None = self.conn {
            return Ok(());
        }
        self.conn.as_mut().unwrap().shutdown(net::Shutdown::Both)
    }

    fn configure_connection(self: &mut Self) -> Result<(), io::Error> {
        if let None = self.conn {
            return Ok(());
        }

        self.conn
            .as_mut()
            .unwrap()
            .set_read_timeout(Some(KEEP_ALIVE_MAX_DURATION))?;
        self.conn
            .as_mut()
            .unwrap()
            .set_write_timeout(Some(KEEP_ALIVE_MAX_DURATION))?;
        self.conn.as_mut().unwrap().set_nonblocking(false)?;

        Ok(())
    }

    pub fn handshake(self: &mut Self, info_hash: [u8; 20]) -> Result<(), io::Error> {
        if let None = self.conn {
            return Ok(());
        }

        let packet = HandshakePacket::new(info_hash).build();

        self.conn.as_ref().unwrap().write(&packet)?;

        let mut buf = [0; 68];
        self.conn.as_ref().unwrap().read_exact(&mut buf)?;

        match HandshakePacket::parse(&buf) {
            Some(p) => {
                println!("got peer id {}", str::from_utf8(&p.peer_id).unwrap());
                self.peer_id = Some(p.peer_id);
            }
            None => {}
        }

        Ok(())
    }

    pub fn send_message(
        self: &Self,
        msg_type: MessageType,
        payload: Option<&Vec<u8>>,
    ) -> Result<(), io::Error> {
        if let None = self.conn {
            return Ok(());
        }

        let msg_buf = match msg_type {
            MessageType::KeepAlive => create_peer_message(0, None, None),
            MessageType::Choke
            | MessageType::Unchoke
            | MessageType::Interested
            | MessageType::NotInterested => create_peer_message(1, Some(msg_type as u8), None),
            MessageType::Have
            | MessageType::Bitfield
            | MessageType::Request
            | MessageType::Piece
            | MessageType::Cancel
            | MessageType::Port => {
                let mut payload_len: u32 = 0;
                if let Some(p) = payload {
                    payload_len = p.len() as u32;
                }
                create_peer_message(1 + payload_len, Some(msg_type as u8), payload)
            }
        };

        self.conn.as_ref().unwrap().write_all(&msg_buf)?;

        Ok(())
    }

    pub fn has_data(self: &Self) -> Result<bool, io::Error> {
        if let None = self.conn {
            return Err(easy_err("not connected"));
        }

        let mut buf = [0 as u8; 10];

        self.conn.as_ref().unwrap().set_nonblocking(true)?;
        let peek_res = self.conn.as_ref().unwrap().peek(&mut buf);
        self.conn.as_ref().unwrap().set_nonblocking(false)?;

        match peek_res {
            Ok(nread) => return Ok(nread > 0),
            Err(e) => {
                if e.kind() == io::ErrorKind::WouldBlock {
                    return Ok(false);
                }
                return Err(e);
            }
        }
    }

    pub fn receive_message(self: &Self) -> Result<Message, io::Error> {
        if let None = self.conn {
            return Err(easy_err("not connected"));
        }

        let mut buf = [0; 4];
        self.conn.as_ref().unwrap().read_exact(&mut buf)?;

        let data_len = u32::from_be_bytes(buf.try_into().unwrap()) as usize;
        if data_len == 0 {
            return Ok(Message {
                message_type: MessageType::KeepAlive,
                payload: Vec::new(),
            });
        }

        let mut data = vec![0; data_len];
        let mut total_read = 0;
        while total_read < data_len {
            let nread = self.conn.as_ref().unwrap().read(&mut data[total_read..])?;
            total_read += nread;
        }

        let mt = MessageType::from_u8(data.remove(0));
        if let None = mt {
            return Err(easy_err("unknown message type"));
        }

        Ok(Message {
            message_type: mt.unwrap(),
            payload: data,
        })
    }

    pub fn has_piece(self: &Self, piece_idx: u32) -> bool {
        self.peer_has.contains(&piece_idx)
    }

    // this will overwrite peer's has list
    pub fn use_bitfield(self: &mut Self, bitfield: &Vec<u8>) {
        let v = parse_bitfield(bitfield);
        v.iter().for_each(|t| {
            self.peer_has.insert(*t as u32);
        });
    }

    pub fn set_interested(self: &mut Self, interested: bool) -> Result<(), io::Error> {
        if self.am_interested != interested {
            if interested {
                self.send_message(MessageType::Interested, None)?;
            } else {
                self.send_message(MessageType::NotInterested, None)?;
            }
        }
        self.am_interested = interested;
        Ok(())
    }

    pub fn can_download(&self) -> bool {
        !self.am_choked
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        if let Err(e) = self.disconnect() {
            println!("failed to disconnect peer when dropped {:?}", e);
        }
    }
}

fn parse_bitfield(bitfield: &Vec<u8>) -> Vec<usize> {
    let mut pieces = Vec::new();

    for (byte_idx, byte) in bitfield.iter().enumerate() {
        for idx in 0..8 {
            if byte & (0b10000000 >> (idx % 8)) != 0 {
                pieces.push(idx + (byte_idx * 8));
            }
        }
    }

    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bitfield() {
        let mut v = Vec::new();
        v.push(0b11110000);
        v.push(0b11111111);
        v.push(0b00000000);
        v.push(0b00000001);
        let b = parse_bitfield(&v);
        println!("{:?} {}", b, b.len());
    }
}

impl fmt::Debug for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", ip_to_str(self.ip_address), self.port)
    }
}

fn create_peer_message(len: u32, id: Option<u8>, payload: Option<&Vec<u8>>) -> Vec<u8> {
    let mut buf = Vec::new();

    buf.extend_from_slice(&len.to_be_bytes());
    if let Some(i) = id {
        buf.push(i);
    }
    if let Some(p) = payload {
        buf.extend_from_slice(&p);
    }

    buf
}

trait Packet {
    fn build(self: &Self) -> Vec<u8>;
    fn parse(buf: &[u8]) -> Option<Box<Self>>;
}

// https://wiki.theory.org/BitTorrentSpecification#Handshake
pub struct HandshakePacket {
    prot: [u8; 19],
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

impl HandshakePacket {
    fn new(info_hash: [u8; 20]) -> Self {
        Self {
            prot: BITTORRENT_PROTOCOL.as_bytes().try_into().unwrap(),
            info_hash: info_hash,
            peer_id: *PEER_ID.get().unwrap(),
        }
    }
}

impl Packet for HandshakePacket {
    fn build(self: &Self) -> Vec<u8> {
        let mut buf = Vec::new();

        buf.push(19);
        buf.extend(&self.prot);
        // 8 reserved bytes
        buf.extend(&[0; 8]);
        buf.extend(&self.info_hash);
        buf.extend(&self.peer_id);

        buf
    }

    fn parse(buf: &[u8]) -> Option<Box<Self>> {
        if buf.len() != 68 || buf[0] != 19 {
            return None;
        }

        if buf[1..20] != BITTORRENT_PROTOCOL.as_bytes()[..] {
            return None;
        }

        Some(Box::new(Self {
            prot: BITTORRENT_PROTOCOL.as_bytes().try_into().unwrap(),
            info_hash: buf.get(28..48).unwrap().try_into().unwrap(),
            peer_id: buf.get(48..68).unwrap().try_into().unwrap(),
        }))
    }
}

fn ip_to_socket_addr(ip: u32, port: u16) -> SocketAddr {
    SocketAddr::new(
        net::IpAddr::V4(net::Ipv4Addr::new(
            (ip >> 24) as u8,
            (ip >> 16) as u8,
            (ip >> 8) as u8,
            (ip) as u8,
        )),
        port,
    )
}

pub fn ip_to_str(ip: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        (ip >> 24) as u8,
        (ip >> 16) as u8,
        (ip >> 8) as u8,
        (ip) as u8,
    )
}
