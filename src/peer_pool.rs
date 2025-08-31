use crate::{
    peer::{KEEP_ALIVE_MAX_DURATION, MessageType, Peer},
    server::Server,
    torrent::Block,
    util::easy_err,
};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    ops::Deref,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time,
};

pub struct PeerPool {
    peers: Vec<Peer>,
    server: Server,
    // these are unconnected peers
    backlog_peers: Vec<Peer>,
    info_hash: [u8; 20],
    desired: HashSet<Block>,

    active_peers: Vec<Peer>,
    thread_peers: Vec<DownloadThread>,
}

struct DownloadThread {
    block: Block,
    thread: JoinHandle<Result<Peer, io::Error>>,
}

const MAX_CONNECTIONS: usize = 64;

impl PeerPool {
    pub fn new(info_hash: [u8; 20]) -> Result<PeerPool, io::Error> {
        Ok(PeerPool {
            peers: Vec::new(),
            server: Server::start()?,
            backlog_peers: Vec::new(),
            info_hash: info_hash,
            active_peers: Vec::new(),
            desired: HashSet::new(),
            thread_peers: Vec::new(),
        })
    }

    pub fn connect_peers(self: &mut Self, mut peers: Vec<Peer>) {
        let mut ts: Vec<JoinHandle<Option<Peer>>> = Vec::new();

        for _ in 0..peers.len() {
            let mut peer = peers.remove(0);
            let info_hash = self.info_hash.clone();
            let t = thread::spawn(move || -> Option<Peer> {
                match peer.connect() {
                    Ok(_) => {}
                    Err(e) => {
                        println!("failed to connect {:?}", e);
                        return None;
                    }
                }
                match peer.handshake(info_hash) {
                    Ok(_) => {}
                    Err(e) => {
                        println!("failed to handshake {:?}", e);
                        if let Err(e) = peer.disconnect() {
                            println!("failed to disconnect client bc of failed handshake {:?}", e);
                        }
                        return None;
                    }
                }
                Some(peer)
            });
            ts.push(t);
        }

        for _ in 0..ts.len() {
            let t = ts.remove(0);
            match t.join() {
                Ok(p) => {
                    if let Some(peer) = p {
                        self.active_peers.push(peer);
                    }
                }
                Err(e) => {
                    println!("failed to join thread {:?}", e);
                }
            }
        }
    }

    pub fn submit_desired_block(self: &mut Self, block: Block) {
        self.desired.insert(block);
    }

    pub fn handle(self: &mut Self) {
        loop {
            {
                let mut ts: Vec<JoinHandle<Option<Peer>>> = Vec::new();
                for _ in 0..self.active_peers.len() {
                    let mut peer = self.active_peers.swap_remove(0);
                    ts.push(thread::spawn(|| -> Option<Peer> {
                        'peerLoop: loop {
                            match peer.has_data() {
                                Ok(b) => {
                                    if !b {
                                        break 'peerLoop;
                                    }
                                }
                                Err(e) => {
                                    println!("failed to check if peer has data {:?}", e);
                                    return None;
                                }
                            }

                            if let Err(e) = handle_peer(&mut peer) {
                                println!("failed to handle peer {:?}", e);
                                return None;
                            }
                        }

                        Some(peer)
                    }));
                }
                for _ in 0..ts.len() {
                    let t = ts.remove(0);
                    match t.join() {
                        Ok(p) => {
                            if let Some(peer) = p {
                                self.active_peers.push(peer);
                            }
                        }
                        Err(e) => {
                            println!("failed to join thread {:?}", e);
                        }
                    }
                }
            }

            let mut untouched_peers = Vec::new();
            for _ in 0..self.active_peers.len() {
                let mut ap = self.active_peers.swap_remove(0);

                if !ap.can_download() {
                    untouched_peers.push(ap);
                    continue;
                }

                let req_block = 'reqBlockBlock: {
                    for block in &self.desired {
                        if !ap.has_piece(block.piece_index) {
                            continue;
                        }
                        break 'reqBlockBlock Some(block);
                    }
                    break 'reqBlockBlock None;
                };

                if let None = req_block {
                    if let Err(e) = ap.set_interested(false) {
                        println!("failed to set uninterested {:?}", e);
                    } else {
                        untouched_peers.push(ap);
                    }
                    continue;
                }

                let block = *req_block.unwrap();

                let t: JoinHandle<Result<Peer, io::Error>> =
                    thread::spawn(move || -> Result<Peer, io::Error> {
                        println!("fetching block {} {}", block.piece_index, block.byte_offset);
                        ap.set_interested(true)?;

                        if ap.has_data()? {
                            handle_peer(&mut ap)?;
                        }

                        let mut payload: Vec<u8> = Vec::new();
                        payload.extend(block.piece_index.to_be_bytes());
                        payload.extend(block.byte_offset.to_be_bytes());
                        payload.extend(block.requested_length.to_be_bytes());
                        ap.send_message(MessageType::Request, Some(&payload))?;

                        handle_peer(&mut ap)?;

                        Ok(ap)
                    });

                self.thread_peers.push(DownloadThread {
                    block: block,
                    thread: t,
                });
                self.desired.remove(&block);
            }

            self.active_peers = untouched_peers;

            for _ in 0..self.thread_peers.len() {
                let tp = self.thread_peers.swap_remove(0);
                if !tp.thread.is_finished() {
                    self.thread_peers.push(tp);
                    continue;
                }
                match tp.thread.join() {
                    Ok(p) => match p {
                        Ok(peer) => {
                            println!(
                                "downloaded block {} {}",
                                tp.block.piece_index, tp.block.byte_offset
                            );
                            self.active_peers.push(peer);
                        }
                        Err(e) => {
                            println!("failed to download block {:?}", e);
                            self.desired.insert(tp.block);
                        }
                    },
                    Err(e) => {
                        println!("failed to join thread {:?}", e);
                        self.desired.insert(tp.block);
                    }
                }
            }

            // TODO: upload
            // TODO: keep alive
        }
    }
}

fn handle_peer(peer: &mut Peer) -> Result<(), io::Error> {
    let msg = match peer.receive_message() {
        Ok(m) => m,
        Err(e) => {
            println!("failed to receive message {:?}", e);
            return Err(e);
        }
    };
    println!("got message type {:?}", msg.message_type);
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
                return Err(easy_err("peer sent incorrect have message length"));
            }
            let have_idx = u32::from_be_bytes(msg.payload.try_into().unwrap());
            peer.peer_has.insert(have_idx);
        }
        MessageType::Bitfield => {
            peer.use_bitfield(&msg.payload);
            println!("peer gotbitfield {}", peer.peer_has.len());
        }
        MessageType::Request => {
            let piece_idx = u32::from_be_bytes(msg.payload.get(0..4).unwrap().try_into().unwrap());
            let byte_offset =
                u32::from_be_bytes(msg.payload.get(4..8).unwrap().try_into().unwrap());

            if let Ok(file_content) = fs::read(format!("./download/{}-{}", piece_idx, byte_offset))
            {
                let mut payload = Vec::new();
                payload.extend(msg.payload.get(0..4).unwrap());
                payload.extend(msg.payload.get(4..8).unwrap());
                payload.extend(file_content);
                peer.send_message(MessageType::Piece, Some(&payload))?;
            }
        }
        MessageType::Piece => {
            let piece_idx = u32::from_be_bytes(msg.payload.get(0..4).unwrap().try_into().unwrap());
            let byte_offset =
                u32::from_be_bytes(msg.payload.get(4..8).unwrap().try_into().unwrap());
            println!("downloading piece {} {}", piece_idx, byte_offset);
            let _ = fs::create_dir("./download");
            fs::write(
                format!("./download/{}-{}", piece_idx, byte_offset),
                msg.payload.get(8..).unwrap(),
            )
            .expect("failed to write block file");
            println!("downloaded piece");
        }
        MessageType::Cancel => {
            // TODO:
        }
        MessageType::Port => {
            // TODO: dht
        }
    };
    peer.last_message_at = Some(time::Instant::now());

    if let Some(t) = peer.last_message_at {
        if t.elapsed() >= KEEP_ALIVE_MAX_DURATION {
            return Err(easy_err("peer exceeded max keep alive duration"));
        }
    }

    Ok(())
}
