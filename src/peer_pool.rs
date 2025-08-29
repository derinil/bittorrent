use crate::{
    peer::{self, Peer},
    server::Server,
    torrent,
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
    // these are unconnected peers
    backlog_peers: Vec<Peer>,
    info_hash: [u8; 20],
}

const MAX_CONNECTIONS: usize = 64;

impl SharedPeerPool {
    pub fn new(info_hash: [u8; 20]) -> Result<SharedPeerPool, io::Error> {
        Ok(SharedPeerPool {
            pool: Arc::new(Mutex::new(PeerPool {
                peers: Vec::new(),
                server: Server::start()?,
                backlog_peers: Vec::new(),
                info_hash: info_hash,
            })),
        })
    }

    pub fn submit_peer(self: &mut Self, mut p: peer::Peer) {
        println!("submitting peer {:?}", p);
        match p.connect() {
            Ok(_) => {}
            Err(e) => {
                println!("failed to connect {:?}", e);
                return;
            }
        }
        match p.handshake(self.pool.lock().unwrap().info_hash) {
            Ok(_) => {}
            Err(e) => {
                println!("failed to handshake {:?}", e);
                return;
            }
        }
        if self.count_connected() >= MAX_CONNECTIONS {
            if let Err(e) = p.disconnect() {
                println!("failed to disconnect client bc of max connections {:?}", e);
            }
            self.pool.lock().unwrap().backlog_peers.push(p);
            println!("put peer into backlog");
            return;
        }
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
        // TODO: clean this up better
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
