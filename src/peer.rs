use std::io;
use std::io::Write;
use std::net;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time;

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
        let mut conn =
            TcpStream::connect_timeout(&self.to_socket_addr(), time::Duration::from_secs(10))?;

        conn.set_read_timeout(Some(time::Duration::from_secs(10)))?;
        conn.set_write_timeout(Some(time::Duration::from_secs(10)))?;

        let packet = Peer::create_handshake_packet(info_hash, peer_id);

        conn.write(&packet)?;

        conn.

        Ok(())
    }

    fn to_socket_addr(self: &Self) -> SocketAddr {
        SocketAddr::new(
            net::IpAddr::V4(net::Ipv4Addr::new(
                (self.ip_address >> 24) as u8,
                (self.ip_address >> 24) as u8,
                (self.ip_address >> 24) as u8,
                (self.ip_address >> 24) as u8,
            )),
            self.port,
        )
    }

    // https://wiki.theory.org/BitTorrentSpecification#Handshake
    fn create_handshake_packet(info_hash: [u8; 20], peer_id: [u8; 20]) -> [u8; 68] {
        let mut buf = [0; 68];

        buf[0] = 19;
        buf[1..19].copy_from_slice("BitTorrent protocol".as_bytes());
        // reserved: eight (8) reserved bytes. All current implementations use all zeroes
        buf[27..47].copy_from_slice(&info_hash);
        buf[47..67].copy_from_slice(&peer_id);

        buf
    }
}
