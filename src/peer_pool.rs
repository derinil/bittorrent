use crate::{
    peer::{self, Peer},
    server::Server,
};
use std::{
    io, net::{Ipv4Addr, TcpListener}, sync::{Arc, Mutex}, thread, time
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
    pub fn new() -> Result<SharedPeerPool, io::Error> {
        Ok(SharedPeerPool {
            pool: Arc::new(Mutex::new(PeerPool {
                peers: Vec::new(),
                server: Server::start()?,
            })),
        })
    }

    pub fn start(self: &mut Self) {
        let mut p = self.clone();
        thread::spawn(move || {
            loop {
                println!("{:?} - running peer pool", time::SystemTime::now());
                p.run_once();
                println!("{:?} - ran peer pool", time::SystemTime::now());
                thread::sleep(time::Duration::from_secs(3));
            }
        });
    }

    pub fn run_once(self: &mut Self) {
        let mut p = self.pool.lock().unwrap();

        let connected = p.peers.iter().count();

        if connected >= MAX_CONNECTIONS {
            p.server.s.incoming().for_each(|i| {
                if let Err(e) = &i {
                    println!("failed to get incoming connection {:?}", e);
                }
                if let Err(e) = &i.unwrap().shutdown(std::net::Shutdown::Both) {
                    println!("failed to shutdown incoming connection {:?}", e);
                }
            });
        }

        if connected < MAX_CONNECTIONS {
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
        }

        p.peers.iter().for_each(|p| {
            println!("handling peer {:?}", p.peer_id);
        });

        println!("got {connected} connected peers");
    }
}
