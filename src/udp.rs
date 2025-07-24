use std::{
    fmt,
    io::{Error, Read},
    net::UdpSocket,
    thread::sleep,
    time::Duration,
};

// https://www.bittorrent.org/beps/bep_0015.html

pub struct Tracker {
    connection_id: Option<u64>,
    socket: UdpSocket,

    downloaded: Option<u64>,
    left: Option<u64>,
    uploaded: Option<u64>,
}

struct Packet<const PACKET_SIZE: usize> {
    tx_id: u32,
    bytes: [u8; PACKET_SIZE],
}

impl Tracker {
    pub fn new() -> Result<Self, std::io::Error> {
        Ok(Tracker {
            connection_id: None,
            socket: UdpSocket::bind("0.0.0.0:0")?,
            downloaded: None,
            left: None,
            uploaded: None,
        })
    }

    pub fn initiate(self: &mut Self, announce_url: &str) -> Result<(), std::io::Error> {
        self.socket.connect(announce_url)?;

        let conn_packet = self.create_connect_packet();
        self.socket.send(&conn_packet.bytes)?;

        self.socket.set_read_timeout(Some(Duration::from_secs(5)))?;

        let mut buf = [0; 128];
        self.socket.recv(&mut buf)?;

        let connection_id = u64::from_be_bytes(buf[8..16].try_into().unwrap());
        let tx_id = u32::from_be_bytes(buf[4..8].try_into().unwrap());

        if tx_id != conn_packet.tx_id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "got unexpected tx id",
            ));
        }

        println!("received connection id {}", connection_id);

        self.connection_id = Some(connection_id);

        Ok(())
    }

    pub fn announce(
        self: &mut Self,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
        event: u32,
    ) -> Result<(), std::io::Error> {
        let packet = self.create_announce_packet(info_hash, peer_id, event);
        self.socket.send(&packet.bytes)?;

        self.socket.set_read_timeout(Some(Duration::from_secs(5)))?;

        let mut buf: [u8; 8192] = [0; 8192];
        let len_read = self.socket.recv(&mut buf)?;
        println!("read {len_read} bytes");

        let action = u32::from_be_bytes(buf[0..4].try_into().unwrap());

        if action == 3 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "got error response {}",
                    str::from_utf8(&buf[8..256]).unwrap()
                ),
            ));
        }

        println!("{:?}", &buf[..len_read]);

        let tx_id = u32::from_be_bytes(buf[4..8].try_into().unwrap());
        let interval = u32::from_be_bytes(buf[8..12].try_into().unwrap());
        let leechers = u32::from_be_bytes(buf[12..16].try_into().unwrap());
        let seeders = u32::from_be_bytes(buf[16..20].try_into().unwrap());

        if tx_id != packet.tx_id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "got unexpected tx id",
            ));
        }

        println!("got {interval} seconds, {leechers} leechers, {seeders} seeders");

        Ok(())
    }

    fn create_connect_packet(self: &mut Self) -> Packet<16> {
        const PROTOCOL_ID: u64 = 0x41727101980;
        let mut buf = [0; 16];

        buf[0..8].copy_from_slice(&PROTOCOL_ID.to_be_bytes());

        let action: u32 = 0;
        buf[8..12].copy_from_slice(&action.to_be_bytes());

        let tx_id: u32 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u32;
        buf[12..16].copy_from_slice(&tx_id.to_be_bytes());

        Packet {
            tx_id: tx_id,
            bytes: buf,
        }
    }

    fn create_announce_packet(
        self: &mut Self,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
        event: u32,
    ) -> Packet<98> {
        let mut buf = [0; 98];

        buf[0..8].copy_from_slice(&self.connection_id.unwrap_or(0).to_be_bytes());

        let action: u32 = 1;
        buf[8..12].copy_from_slice(&action.to_be_bytes());

        let tx_id: u32 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u32;
        buf[12..16].copy_from_slice(&tx_id.to_be_bytes());

        buf[16..36].copy_from_slice(&info_hash);
        buf[36..56].copy_from_slice(&peer_id);
        buf[56..64].copy_from_slice(&self.downloaded.unwrap_or(0).to_be_bytes());
        buf[64..72].copy_from_slice(&self.left.unwrap_or(0).to_be_bytes());
        buf[72..80].copy_from_slice(&self.uploaded.unwrap_or(0).to_be_bytes());
        buf[80..84].copy_from_slice(&event.to_be_bytes());

        let ip: u32 = 0;
        buf[84..88].copy_from_slice(&ip.to_be_bytes());

        let key: u32 = 0;
        buf[88..92].copy_from_slice(&key.to_be_bytes());

        let numwant: i32 = -1;
        buf[92..96].copy_from_slice(&numwant.to_be_bytes());

        let port: u16 = 6881;
        buf[96..98].copy_from_slice(&port.to_be_bytes());

        Packet {
            tx_id: tx_id,
            bytes: buf,
        }
    }
}

fn attempt_with_backoff<F: Fn() -> Result<(), Error>>(cl: F) -> Result<(), Error> {
    let mut err: Option<Error> = None;

    for i in 0..3 {
        match cl() {
            Ok(v) => {
                return Ok(v);
            }
            Err(e) => {
                err = Some(e);
            }
        }
        sleep(std::time::Duration::from_secs(backoff(i)));
    }

    if let Some(err) = err {
        return Err(err);
    }

    Ok(())
}

fn backoff(tries: u32) -> u64 {
    return 15 * (2 as u64).pow(tries);
}
