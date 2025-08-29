use std::fmt;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::thread;
use std::time;

use crate::torrent;

const BITTORRENT_PROTOCOL: &str = "BitTorrent protocol";

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

pub struct Peer {
    pub ip_address: u32,
    pub port: u16,
    conn: Option<TcpStream>,
    addr: SocketAddr,
    am_choked: bool,
    am_interested: bool,
    peer_choked: bool,
    peer_interested: bool,

    pub peer_id: Option<[u8; 20]>,
    alive_keeper_thread: Option<thread::Thread>,

    peer_has: Vec<torrent::Block>,
    we_have: Vec<torrent::Block>,
}

impl Peer {
    pub fn new(ip_address: u32, port: u16) -> Self {
        Self {
            ip_address: ip_address,
            port: port,
            conn: None,
            addr: ip_to_socket_addr(ip_address, port),
            // https://wiki.theory.org/BitTorrentSpecification#Overview
            am_choked: true,
            am_interested: false,
            peer_choked: true,
            peer_interested: false,
            peer_id: None,
            alive_keeper_thread: None,
            peer_has: Vec::new(),
            we_have: Vec::new(),
        }
    }

    fn accept(self: &Self) {}

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

        Ok(())
    }

    fn configure_connection(self: &mut Self) -> Result<(), io::Error> {
        if let None = self.conn {
            return Ok(());
        }

        self.conn
            .as_mut()
            .unwrap()
            .set_read_timeout(Some(time::Duration::from_secs(10)))?;
        self.conn
            .as_mut()
            .unwrap()
            .set_write_timeout(Some(time::Duration::from_secs(10)))?;
        self.conn.as_mut().unwrap().set_nonblocking(true);

        Ok(())
    }

    pub fn handshake(
        self: &mut Self,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
    ) -> Result<(), io::Error> {
        if let None = self.conn {
            return Ok(());
        }

        let packet = HandshakePacket::new(info_hash, peer_id).build();

        self.conn.as_ref().unwrap().write(&packet)?;

        let mut buf = [0; 512];
        let nread = self.conn.as_ref().unwrap().read(&mut buf)?;

        println!("read {nread} bytes");

        match HandshakePacket::parse(&buf[0..nread]) {
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

    // non-blocking
    pub fn receive_message(self: &Self) -> Result<(), io::Error> {
        if let None = self.conn {
            return Ok(());
        }

        let mut buf = [0; 1];
        self.conn.as_ref().unwrap().read(&mut buf)?;

        Ok(())
    }

    pub fn get_peer_id(self: &Self) -> Result<Option<String>, std::string::FromUtf8Error> {
        if let None = self.peer_id {
            return Ok(None);
        }

        Ok(Some(String::from_utf8(self.peer_id.unwrap().to_vec())?))
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
    lenprot: u8,
    prot: [u8; 19],
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

impl HandshakePacket {
    fn new(info_hash: [u8; 20], peer_id: [u8; 20]) -> Self {
        Self {
            lenprot: 19,
            prot: BITTORRENT_PROTOCOL.as_bytes().try_into().unwrap(),
            info_hash: info_hash,
            peer_id: peer_id,
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
            lenprot: 19,
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
