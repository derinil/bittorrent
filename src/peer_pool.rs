use crate::{
    peer::{self, Peer},
    server::Server,
};
use std::{
    io,
    net::{Ipv4Addr, TcpListener},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct SharedPeerPool {
    pool: Arc<Mutex<PeerPool>>,
}

struct PeerPool {
    peers: Vec<Peer>,
    server: Server,
}

const MAX_CONNECTIONS: usize = 64;

impl SharedPeerPool {
    pub fn initialize(&mut self) -> Result<(), io::Error> {
        let server = Server::start()?;
        self.pool.lock().unwrap().server = server;
        Ok(())
    }

    pub fn add_peer(self: &mut Self, peer: Peer) {
        let mut p = self.pool.lock().unwrap();
        p.peers.push(peer);
    }

    pub fn run(self: &mut Self) {
        let mut p = self.pool.lock().unwrap();

        match p.server.s.accept() {
            Ok(c) => 'okLabel: {
                let ip: u32;
                if let std::net::IpAddr::V4(v4) = c.1.ip() {
                    ip = v4.to_bits();
                } else {
                    if let Err(e) = c.0.shutdown(std::net::Shutdown::Both) {
                        println!("got error shutting down connection {}", e);
                    }
                    break 'okLabel;
                }
                p.peers.push(Peer::new(ip, c.1.port()));
                println!("accepted connection from {:?}", p.peers.last());
            }
            Err(e) => {
                println!("got error accepting connection {e:?}");
            }
        };

        let mut connected = 0;
        p.peers.iter().for_each(|p: &Peer| {
            connected += 1;
        });

        println!("got {connected} connected peers");
    }
}
