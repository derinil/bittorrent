use crate::{
    peer::{self, MessageType, Peer},
    server::Server,
    torrent::Block,
};
use std::{
    io,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time,
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
    active_threads: Vec<JoinHandle<peer::Peer>>,
    receiving: Vec<Block>,
    desired: Vec<Block>,
    received: Vec<Block>,
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
                active_threads: Vec::new(),
                receiving: Vec::new(),
                desired: Vec::new(),
                received: Vec::new(),
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
                if let Err(e) = p.disconnect() {
                    println!("failed to disconnect client bc of failed handshake {:?}", e);
                }
                return;
            }
        }
        match p.receive_message() {
            Ok(m) => 'msgMatch: {
                if let None = m {
                    break 'msgMatch;
                }
                if !matches!(m.as_ref().unwrap().message_type, MessageType::Bitfield) {
                    if let Err(e) = p.disconnect() {
                        println!(
                            "failed to disconnect client bc of unexpected message {:?}",
                            e
                        );
                    }
                }
                p.parse_bitfield(&m.as_ref().unwrap().payload);
                println!("peer gotbitfield {:?} {}", p.peer_has, p.peer_has.len());
                return;
            }
            Err(e) => {
                println!("failed to receive bitfield message {:?}", e);
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

    pub fn submit_desired_block(self: &mut Self, block: Block) {
        self.pool.lock().unwrap().desired.push(block);
    }

    pub fn run_once(self: &mut Self) {
        let connected = self.count_connected();
        println!("got {connected} connected peers");

        let mut p = self.pool.lock().unwrap();

        for _ in 0..p.peers.len() {
            let peer = p.peers.remove(0);
            let t = thread::spawn(|| -> peer::Peer {
                if let Some(s) = peer.get_peer_id().unwrap() {
                    println!("handling peer {:?}", s);
                }
                peer
            });
            p.active_threads.push(t);
        }

        let mut new_active_threads = Vec::new();
        for _ in 0..p.active_threads.len() {
            let t = p.active_threads.remove(0);
            if t.is_finished() {
                p.peers.push(t.join().unwrap());
            } else {
                new_active_threads.push(t);
            }
        }
        p.active_threads = new_active_threads;
    }
}
