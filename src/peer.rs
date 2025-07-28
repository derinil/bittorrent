use std::io;
use std::io::Read;
use std::io::Write;
use std::net;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time;

const BITTORRENT_PROTOCOL: &str = "BitTorrent protocol";

pub struct Peer {
    pub ip_address: u32,
    pub port: u16,
    am_choked: bool,
    am_interested: bool,
    peer_choked: bool,
    peer_interested: bool,
}

impl Peer {
    pub fn new(ip_addr: u32, port: u16) -> Self {
        Self {
            ip_address: ip_addr,
            port: port,
            // https://wiki.theory.org/BitTorrentSpecification#Overview
            am_choked: true,
            am_interested: false,
            peer_choked: true,
            peer_interested: false,
        }
    }

    fn connect(self) {}

    pub fn handshake(self: &Self, info_hash: [u8; 20], peer_id: [u8; 20]) -> Result<(), io::Error> {
        println!("handshaking");

        let sa = self.to_socket_addr();
        println!("{}", sa);

        let mut conn = TcpStream::connect_timeout(&sa, time::Duration::from_secs(5))?;

        conn.set_read_timeout(Some(time::Duration::from_secs(10)))?;
        conn.set_write_timeout(Some(time::Duration::from_secs(10)))?;

        let packet = HandshakePacket::new(info_hash, peer_id).build();

        conn.write(&packet)?;

        let mut buf = [0; 512];
        let nread = conn.read(&mut buf)?;

        println!("read {nread} bytes");

        match HandshakePacket::parse(&buf[0..nread]) {
            Some(p) => {
                println!("got peer id {}", str::from_utf8(&p.peer_id).unwrap());
            }
            None => {}
        }

        Ok(())
    }

    fn to_socket_addr(self: &Self) -> SocketAddr {
        SocketAddr::new(
            net::IpAddr::V4(net::Ipv4Addr::new(
                (self.ip_address >> 24) as u8,
                (self.ip_address >> 16) as u8,
                (self.ip_address >> 8) as u8,
                (self.ip_address) as u8,
            )),
            self.port,
        )
    }

    pub fn to_ip_string(self: &Self) -> String {
        format!(
            "{}.{}.{}.{}",
            (self.ip_address >> 24) as u8,
            (self.ip_address >> 16) as u8,
            (self.ip_address >> 8) as u8,
            (self.ip_address) as u8,
        )
    }
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
