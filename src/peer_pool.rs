use crate::{
    peer::{self, Peer},
    server::Server,
};
use std::{
    io,
    net::{Ipv4Addr, TcpListener},
    sync::{Arc, Mutex},
    thread, time,
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

    pub fn submit_peer(self: &mut Self, p: peer::Peer) {
        println!("submitting peer {:?}", p);
        self.pool.lock().unwrap().peers.push(p);
        println!("submitted peer");
    }

    pub fn count_connected(self: &Self) -> usize {
        self.pool.lock().unwrap().peers.iter().count()
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
        let connected = self.count_connected();
        println!("got {connected} connected peers");

        self.pool.lock().unwrap().peers.iter().for_each(|p| {
            if let Some(s) = p.get_peer_id().unwrap() {
                println!("handling peer {:?}", s);
            }
        });
    }
}
