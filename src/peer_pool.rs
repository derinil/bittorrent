use crate::{
    peer::{KEEP_ALIVE_MAX_DURATION, MessageType, Peer},
    server::Server,
    torrent::{Block, DownloadBlock},
    util::easy_err,
};
use std::{
    cmp::min,
    collections::HashSet,
    fs, io,
    thread::{self, JoinHandle},
    time,
};

pub struct PeerPool {
    server: Server,
    // these are unconnected peers
    backlog_peers: Vec<Peer>,
    info_hash: [u8; 20],
    desired: HashSet<Block>,

    active_peers: Vec<Peer>,
    thread_peers: Vec<DownloadThread>,

    last_choke_update: time::Instant,
}

struct DownloadThread {
    block: Block,
    thread: JoinHandle<(Peer, bool)>,
}

const MAX_CONNECTIONS: usize = 64;

impl PeerPool {
    pub fn new(info_hash: [u8; 20]) -> Result<PeerPool, io::Error> {
        Ok(PeerPool {
            server: Server::start()?,
            backlog_peers: Vec::new(),
            info_hash: info_hash,
            active_peers: Vec::new(),
            desired: HashSet::new(),
            thread_peers: Vec::new(),
            last_choke_update: time::Instant::now(),
        })
    }

    pub fn connect_peers(self: &mut Self, mut peers: Vec<Peer>) {
        let mut ts: Vec<JoinHandle<(Peer, bool)>> = Vec::new();

        for _ in 0..peers.len() {
            let mut peer = peers.remove(0);
            let info_hash = self.info_hash.clone();
            let t = thread::spawn(move || -> (Peer, bool) {
                match peer.connect() {
                    Ok(_) => {}
                    Err(e) => {
                        println!("failed to connect {:?}", e);
                        return (peer, false);
                    }
                }
                match peer.handshake(info_hash) {
                    Ok(_) => {}
                    Err(e) => {
                        println!("failed to handshake {:?}", e);
                        if let Err(e) = peer.disconnect() {
                            println!("failed to disconnect client bc of failed handshake {:?}", e);
                        }
                        return (peer, false);
                    }
                }
                return (peer, true);
            });
            ts.push(t);
        }

        for _ in 0..ts.len() {
            let t = ts.remove(0);
            match t.join() {
                Ok(p) => {
                    if p.1 {
                        self.active_peers.push(p.0);
                    } else {
                        self.backlog_peers.push(p.0);
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
            // TODO: accept connections

            // Try to connect to backlog peers up to 5 times, one successfull connection resets count
            if self.desired.len() > 0 {
                self.attempt_backlog_connections();
            }

            // TODO: decide who gets choked every 10 seconds

            {
                let mut ts: Vec<JoinHandle<(Peer, bool)>> = Vec::new();
                for _ in 0..self.active_peers.len() {
                    let mut peer = self.active_peers.swap_remove(0);
                    ts.push(thread::spawn(|| -> (Peer, bool) {
                        'peerLoop: loop {
                            match peer.has_data() {
                                Ok(b) => {
                                    if !b {
                                        // TODO: this is bad
                                        break 'peerLoop;
                                    }
                                }
                                Err(e) => {
                                    println!("failed to check if peer has data {:?}", e);
                                    return (peer, false);
                                }
                            }

                            if let Err(e) = handle_peer_download(&mut peer) {
                                println!("failed to handle peer {:?}", e);
                                return (peer, false);
                            }
                        }

                        return (peer, true);
                    }));
                }
                for _ in 0..ts.len() {
                    let t = ts.remove(0);
                    match t.join() {
                        Ok(p) => {
                            if p.1 {
                                self.active_peers.push(p.0);
                            } else {
                                self.backlog_peers.push(p.0);
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

                let t = thread::spawn(move || -> (Peer, bool) {
                    println!("fetching block {} {}", block.piece_index, block.byte_offset);
                    if let Err(e) = ap.set_interested(true) {
                        println!("failed to set interested {:?}", e);
                        return (ap, false);
                    }

                    match ap.has_data() {
                        Ok(has_data) => 'apMatch: {
                            if !has_data {
                                break 'apMatch;
                            }
                            if let Err(e) = handle_peer_download(&mut ap) {
                                println!("failed to handle peer before requesting {:?}", e);
                                return (ap, false);
                            }
                        }
                        Err(e) => {
                            println!("failed to check if peer has data before requesting {:?}", e);
                            return (ap, false);
                        }
                    }

                    let mut payload: Vec<u8> = Vec::new();
                    payload.extend(block.piece_index.to_be_bytes());
                    payload.extend(block.byte_offset.to_be_bytes());
                    payload.extend(block.requested_length.to_be_bytes());

                    if let Err(e) = ap.send_message(MessageType::Request, Some(&payload)) {
                        println!("failed to send request message {:?}", e);
                        return (ap, false);
                    }

                    (ap, true)
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
                    Ok(p) => {
                        if p.1 {
                            println!(
                                "downloaded block {} {}",
                                tp.block.piece_index, tp.block.byte_offset
                            );
                            self.active_peers.push(p.0);
                        } else {
                            self.backlog_peers.push(p.0);
                        }
                    }
                    Err(e) => {
                        println!("failed to join thread {:?}", e);
                        self.desired.insert(tp.block);
                    }
                }
            }
        }
        // TODO: send have messages
        // TODO: keep alive check
    }

    fn attempt_backlog_connections(self: &mut Self) {
        if self.active_peers.len() + self.thread_peers.len() >= MAX_CONNECTIONS {
            return;
        }
        if self.backlog_peers.len() == 0 {
            return;
        }
        let backlog: Vec<Peer> = self
            .backlog_peers
            .drain(
                0..min(
                    self.backlog_peers.len(),
                    MAX_CONNECTIONS - (self.active_peers.len() + self.thread_peers.len()),
                ),
            )
            .collect();
        println!("connecting to backlog peers {}", backlog.len());
        self.connect_peers(backlog);
        println!("done attempting connections");
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
            let have_idx = u32::from_be_bytes(msg.payload.try_into().unwrap());
            peer.peer_has.insert(have_idx);
        }
        MessageType::Bitfield => {
            peer.use_bitfield(&msg.payload);
            println!("peer gotbitfield {}", peer.peer_has.len());
        }
        MessageType::Request => {
            peer.request_queue.push(Block::parse(&msg.payload).unwrap());
        }
        MessageType::Piece => {
            peer.downloaded_pieces
                .push(DownloadBlock::parse(&msg.payload).unwrap());
        }
        MessageType::Cancel => {
            let b = Block::parse(&msg.payload).unwrap();
            if let Some(idx) = peer.request_queue.iter().position(|p| p.eq(&b)) {
                peer.request_queue.swap_remove(idx);
            }
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
