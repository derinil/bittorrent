use std::{io::Error, net::UdpSocket, thread::sleep, time::Duration};

// https://www.bittorrent.org/beps/bep_0015.html

pub struct Tracker {
    connection_id: Option<u64>,
    socket: UdpSocket,
}

impl Tracker {
    pub fn new() -> Result<Self, std::io::Error> {
        Ok(Tracker {
            connection_id: None,
            socket: UdpSocket::bind("0.0.0.0:0")?,
        })
    }

    pub fn initiate(self: &mut Self, announce_url: &str) -> Result<(), std::io::Error> {
        self.socket.connect(announce_url)?;

        self.socket.send(&create_connect_packet())?;

        self.socket.set_read_timeout(Some(Duration::from_secs(5)))?;

        let mut buf = [0; 128];
        self.socket.recv(&mut buf)?;

        let mut cid = [0; 8];
        cid.copy_from_slice(&buf[8..16]);
        let connection_id = u64::from_be_bytes(cid);

        println!("received connection id {}", connection_id);

        self.connection_id = Some(connection_id);

        Ok(())
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

fn create_connect_packet() -> [u8; 16] {
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

    println!("{:?}", buf);

    buf
}

fn backoff(tries: u32) -> u64 {
    return 15 * (2 as u64).pow(tries);
}
