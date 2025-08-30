use crate::{
    peer::{MessageType, Peer},
    server::Server,
    torrent::Block,
};
use std::{
    fs, io,
    sync::{
        self, Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
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
    receiving: Vec<Block>,
    desired: Vec<Block>,
    received: Vec<Block>,
    cleanup_thread: Option<JoinHandle<()>>,
    active_threads: Vec<PeerThread>,
}

struct PeerThread {
    thread: JoinHandle<()>,
    send_to_thread: sync::mpsc::Sender<PeerThreadMessage>,
    read_from_thread: sync::mpsc::Receiver<PeerThreadMessage>,
}

enum PeerThreadMessage {
    RequestBlock(Block),
    DownloadingBlock(Block),
    DownloadedBlock(Block),
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
                cleanup_thread: None,
            })),
        })
    }

    pub fn submit_peer(self: &mut Self, mut p: Peer) {
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
        if self.count_connected() >= MAX_CONNECTIONS {
            if let Err(e) = p.disconnect() {
                println!("failed to disconnect client bc of max connections {:?}", e);
            }
            self.pool.lock().unwrap().backlog_peers.push(p);
            println!("put peer into backlog");
            return;
        }
        self.pool
            .lock()
            .unwrap()
            .active_threads
            .push(handle_peer(p));
        println!("submitted peer");
    }

    pub fn count_connected(self: &Self) -> usize {
        self.pool.lock().unwrap().peers.iter().count()
    }

    pub fn submit_desired_block(self: &mut Self, block: Block) {
        self.pool.lock().unwrap().desired.push(block);
    }

    // Starts a thread that joins finished threads and peers and runs maintenance
    pub fn start(self: &mut Self) {
        let s = self.clone();
        self.pool.lock().unwrap().cleanup_thread = Some(thread::spawn(move || {
            loop {
                let f = || {
                    let mut p = s.pool.lock().unwrap();
                    let mut new_threads = Vec::new();
                    for _ in 0..p.active_threads.len() {
                        let at = p.active_threads.remove(0);
                        if at.thread.is_finished() {
                            match at.thread.join() {
                                Ok(_) => {}
                                Err(err) => {
                                    println!("failed to join peer thread {:?}", err);
                                }
                            }
                        } else {
                            new_threads.push(at);
                        }
                    }
                    p.active_threads = new_threads;
                    println!("active connections {}", p.active_threads.len());
                };
                f();
                thread::sleep(time::Duration::from_secs(3));
            }
        }));
    }

    pub fn cleanup(self: &mut Self) {
        if let Some(t) = self.pool.lock().unwrap().cleanup_thread.take() {
            match t.join() {
                Ok(_) => {}
                Err(err) => {
                    println!("failed to join cleanup thread {:?}", err);
                }
            }
        }
    }
}

fn handle_peer(mut peer: Peer) -> PeerThread {
    let send_to_thread = mpsc::channel();
    let read_from_thread = mpsc::channel();

    let t = thread::spawn(move || {
        let recv: Receiver<PeerThreadMessage> = send_to_thread.1;
        let send: Sender<PeerThreadMessage> = read_from_thread.0;

        if let Some(s) = peer.get_peer_id().unwrap() {
            println!("handling peer {:?}", s);
        }

        loop {
            let has_data = peer.has_data().unwrap();
            if has_data {
                match peer.receive_message() {
                    Ok(msg) => {
                        match msg.message_type {
                            MessageType::KeepAlive => {}
                            MessageType::Choke => {
                                peer.am_choked = true;
                            }
                            MessageType::Unchoke => {
                                peer.am_choked = false;
                            }
                            MessageType::Interested => {
                                peer.peer_interested = true;
                            }
                            MessageType::NotInterested => {
                                peer.peer_interested = false;
                            }
                            MessageType::Have => {
                                if msg.payload.len() != 4 {
                                    println!(
                                        "peer sent incorrect have message length {}",
                                        msg.payload.len()
                                    );
                                    peer.disconnect().expect("failed to disconnect");
                                    return;
                                }
                                let have_idx = u32::from_be_bytes(msg.payload.try_into().unwrap());
                                peer.peer_has.insert(have_idx);
                            }
                            MessageType::Bitfield => {
                                peer.use_bitfield(&msg.payload);
                                println!(
                                    "peer gotbitfield {:?} {}",
                                    peer.peer_has,
                                    peer.peer_has.len()
                                );
                            }
                            MessageType::Request => {
                                // TODO:
                            }
                            MessageType::Piece => {
                                let piece_idx = u32::from_be_bytes(
                                    msg.payload.get(0..4).unwrap().try_into().unwrap(),
                                );
                                let byte_offset = u32::from_be_bytes(
                                    msg.payload.get(4..8).unwrap().try_into().unwrap(),
                                );
                                println!("downloading piece {} {}", piece_idx, byte_offset);
                                fs::create_dir("./download").expect("failed to mkdir");
                                fs::write(
                                    format!("./download/{}-{}", piece_idx, byte_offset),
                                    msg.payload.get(8..).unwrap(),
                                )
                                .expect("failed to write block file");
                                send.send(PeerThreadMessage::DownloadedBlock(Block::new(
                                    piece_idx,
                                    byte_offset,
                                )))
                                .expect("failed to send peer thread message");
                            }
                            MessageType::Cancel => {
                                // TODO:
                            }
                            MessageType::Port => {
                                // TODO: dht
                            }
                        };
                        peer.last_message_at = Some(time::Instant::now());
                    }
                    Err(err) => {
                        peer.disconnect().expect("failed to disconnect");
                        println!("failed to receive message {}", err);
                        return;
                    }
                }
            }

            // TODO: each second check KEEP_ALIVE_MAX_DURATION and try_iter
            for msg in recv.try_iter() {
                match msg {
                    PeerThreadMessage::RequestBlock(block) => {
                        if !peer.has_piece(block.piece_index) {
                            continue;
                        }
                        let mut payload: Vec<u8> = Vec::new();
                        payload.extend(block.piece_index.to_be_bytes());
                        payload.extend(block.byte_offset.to_be_bytes());
                        payload.extend(block.requested_length.to_be_bytes());
                        peer.send_message(MessageType::Request, Some(&payload))
                            .expect("failed to send request message");
                        println!("sent request")
                    }
                    PeerThreadMessage::DownloadedBlock(block) => {
                        let mut payload: Vec<u8> = Vec::new();
                        payload.extend(block.piece_index.to_be_bytes());
                        payload.extend(block.byte_offset.to_be_bytes());
                        payload.extend(block.requested_length.to_be_bytes());
                        peer.send_message(MessageType::Cancel, Some(&payload))
                            .expect("failed to send cancel message");
                        println!("sent cancel")
                    }
                    _ => {}
                }
            }
        }
    });

    PeerThread {
        thread: t,
        send_to_thread: send_to_thread.0,
        read_from_thread: read_from_thread.1,
    }
}
