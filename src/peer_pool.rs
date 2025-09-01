use crate::{
    peer::{KEEP_ALIVE_MAX_DURATION, MessageType, Peer},
    server::Server,
    torrent::{self, Block, DEFAULT_BLOCK_LENGTH, DownloadBlock, Torrent},
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
    torrent: Torrent,
    have_pieces: HashSet<u32>,
    pieces_in_progress: HashSet<u32>,

    server: Server,

    active_peers: Vec<Peer>,
    thread_peers: Vec<DownloadThread>,
    backlog_peers: Vec<Peer>, // these are unconnected peers

    last_choke_update: time::Instant,
}

struct DownloadThread {
    piece: u32,
    thread: JoinHandle<(Peer, bool)>,
}

const MAX_CONNECTIONS: usize = 64;
const MAX_FAILED_CONNECTION_ATTEMPTS: u32 = 5;

impl PeerPool {
    pub fn new(torrent: Torrent) -> Result<PeerPool, io::Error> {
        Ok(PeerPool {
            torrent: torrent,
            have_pieces: HashSet::new(),
            pieces_in_progress: HashSet::new(),
            server: Server::start()?,
            backlog_peers: Vec::new(),
            active_peers: Vec::new(),
            thread_peers: Vec::new(),
            last_choke_update: time::Instant::now(),
        })
    }

    pub fn connect_peers(self: &mut Self, mut peers: Vec<Peer>) {
        let mut ts: Vec<JoinHandle<(Peer, bool)>> = Vec::new();

        for _ in 0..peers.len() {
            let mut peer = peers.remove(0);
            let info_hash = self.torrent.info_hash.clone();
            let t = thread::spawn(move || -> (Peer, bool) {
                match peer.connect() {
                    Ok(_) => {}
                    Err(e) => {
                        println!("failed to connect {:?}", e);
                        peer.failed_connection_attempts += 1;
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
                        peer.failed_connection_attempts += 1;
                        return (peer, false);
                    }
                }
                peer.failed_connection_attempts = 0;
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
                        if p.0.failed_connection_attempts < MAX_FAILED_CONNECTION_ATTEMPTS {
                            self.backlog_peers.push(p.0);
                        } else {
                            let _ = p.0.get_peer_id().map(|r| {
                                if let Some(pid) = r {
                                    println!(
                                        "removing peer {} from backlog, failed too many times",
                                        pid
                                    );
                                }
                            });
                        }
                    }
                }
                Err(e) => {
                    println!("failed to join thread {:?}", e);
                }
            }
        }
    }

    pub fn handle(self: &mut Self) {
        loop {
            // TODO: accept connections

            // Try to connect to backlog peers.
            // Only do this while downloading, no need to actively seek
            // peers after download is finished.
            if self.count_pieces_left() > 0 {
                self.attempt_backlog_connections();
            }

            // TODO: decide who gets choked every 10 seconds

            self.consume_messages();
            if self.count_pieces_left() > 0 {
                self.download(); // TODO: add another step to actually write pieces to filesystem
            }
            // TODO: send have messages
            self.upload(); // TODO: this will send pieces
            self.check_keep_alive();
        }
    }

    fn consume_messages(self: &mut Self) {
        let mut ts: Vec<JoinHandle<(Peer, bool)>> = Vec::new();
        for _ in 0..self.active_peers.len() {
            let mut peer = self.active_peers.swap_remove(0);
            ts.push(thread::spawn(|| -> (Peer, bool) {
                'peerLoop: loop {
                    match peer.has_data() {
                        Ok(b) => {
                            if !b {
                                break 'peerLoop;
                            }
                        }
                        Err(e) => {
                            println!("failed to check if peer has data {:?}", e);
                            return (peer, false);
                        }
                    }

                    if let Err(e) = handle_peer(&mut peer) {
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

    fn download(self: &mut Self) {
        // TODO: join download threads and consolidate all pieces downloaded

        let pieces_left = self.get_pieces_left();
        // TODO: random pieces

        let mut downloadable_peers: Vec<Peer> = self
            .active_peers
            .extract_if(.., |p| -> bool {
                if !p.can_download() {
                    return false;
                }

                for b in &pieces_left {
                    if p.has_piece(b.clone()) {
                        return true;
                    }
                }

                false
            })
            .collect();

        let ts = spawn_peer_threads(&mut self.active_peers, |mut p: Peer| -> Option<Peer> {
            if let Err(e) = p.set_interested(false) {
                println!("failed to set uninterested for peer {:?}", e);
                return None;
            }
            Some(p)
        });
        for t in ts {
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

        let ts = spawn_peer_threads(&mut downloadable_peers, |mut p: Peer| -> Option<Peer> {
            if let Err(e) = p.set_interested(true) {
                println!("failed to set interested for downloadble peer {:?}", e);
                return None;
            }
            Some(p)
        });
        for t in ts {
            match t.join() {
                Ok(p) => {
                    if let Some(peer) = p {
                        downloadable_peers.push(peer);
                    }
                }
                Err(e) => {
                    println!("failed to join thread {:?}", e);
                }
            }
        }

        let mut assigned_pieces = HashSet::new();

        for mut peer in downloadable_peers {
            let mut peer_piece = None;
            for piece in &pieces_left {
                if !assigned_pieces.contains(piece) {
                    peer_piece = Some(*piece);
                    break;
                }
            }
            if peer_piece.is_none() {
                continue;
            }
            assigned_pieces.insert(peer_piece.unwrap());

            let piece_len = self.torrent.get_piece_len(peer_piece.unwrap());
            self.thread_peers.push(DownloadThread {
                piece: peer_piece.unwrap(),
                thread: thread::spawn(move || -> (Peer, bool) {
                    if let Err(e) =
                        download_piece_from_peer(&mut peer, peer_piece.unwrap(), piece_len)
                    {
                        println!("failed to download piece from peer {:?}", e);
                        return (peer, false);
                    }

                    (peer, true)
                }),
            });
        }

        self.pieces_in_progress.extend(assigned_pieces);

        let done_threads: Vec<DownloadThread> = self
            .thread_peers
            .extract_if(.., |dt| dt.thread.is_finished())
            .collect();

        for dt in done_threads {
            match dt.thread.join() {
                Ok(p) => {
                    if p.1 {
                        self.have_pieces.insert(dt.piece);
                        self.active_peers.push(p.0);
                    } else {
                        self.backlog_peers.push(p.0);
                    }
                }
                Err(e) => {
                    println!("failed to join thread {:?}", e);
                }
            }
            self.pieces_in_progress.remove(&dt.piece);
        }
    }

    fn upload(self: &mut Self) {
        // TODO: loop through peers, check request queue
    }

    fn check_keep_alive(self: &mut Self) {
        self.active_peers.retain(|ap| -> bool {
            ap.last_message_at.is_none()
                || ap.last_message_at.unwrap().elapsed() < KEEP_ALIVE_MAX_DURATION
        });
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

    fn count_pieces_left(&self) -> u32 {
        self.torrent.get_total_piece_count()
            - self.have_pieces.len() as u32
            - self.pieces_in_progress.len() as u32
    }

    fn get_pieces_left(&self) -> Vec<u32> {
        let mut pl = HashSet::new();

        let piece_count = self.torrent.get_total_piece_count();
        for i in 0..piece_count {
            pl.insert(i);
        }

        pl.retain(|i| !self.have_pieces.contains(i));
        pl.retain(|i| !self.pieces_in_progress.contains(i));

        pl.drain().collect()
    }
}

fn download_piece_from_peer(
    peer: &mut Peer,
    piece: u32,
    piece_len: u32,
) -> Result<bool, io::Error> {
    let mut piece_data: Vec<u8> = Vec::new();
    piece_data.reserve_exact(piece_len as usize);

    let mut block_start = 0;
    let mut requested_blocks = HashSet::new();

    println!(
        "requesting piece {} from peer {}",
        piece,
        peer.get_peer_id().unwrap().unwrap()
    );

    while block_start < piece_len {
        println!("piece {} len {} start {}", piece, piece_len, block_start);
        let mut block_len = DEFAULT_BLOCK_LENGTH;
        if block_start + DEFAULT_BLOCK_LENGTH > piece_len {
            block_len = piece_len - block_start;
        }

        let b = Block::new(piece, block_start, block_len);
        requested_blocks.insert(b);

        let mut payload = Vec::new();
        payload.extend(b.to_bytes());
        peer.send_message(MessageType::Request, Some(&payload))?;

        block_start += DEFAULT_BLOCK_LENGTH;
    }

    println!(
        "requested piece {} from peer {}, waiting for download",
        piece,
        peer.get_peer_id().unwrap().unwrap()
    );

    while requested_blocks.len() > 0 {
        handle_peer(peer)?;
        peer.downloaded_blocks.iter().for_each(|b| {
            if requested_blocks.remove(&Block {
                piece_index: b.piece_index,
                byte_offset: b.byte_offset,
                requested_length: b.data.len() as u32,
            }) {
                println!("got block piece {} offset {}", b.piece_index, b.byte_offset);
            }
        });
    }

    println!(
        "downloaded piece {} from peer {}",
        piece,
        peer.get_peer_id().unwrap().unwrap()
    );

    // TODO: verify piece

    let mut piece_blocks: Vec<DownloadBlock> = peer
        .downloaded_blocks
        .extract_if(.., |db| db.piece_index == piece)
        .collect();
    piece_blocks.sort_by(|db1, db2| db1.byte_offset.cmp(&db2.byte_offset));
    piece_blocks.iter().for_each(|pb| {
        piece_data.extend(&pb.data);
    });

    let _ = fs::create_dir("./download");
    let _ = fs::write(format!("./download/{}", piece), piece_data);

    Ok(true)
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
            peer.downloaded_blocks
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

fn spawn_peer_threads<F, T>(peers: &mut Vec<Peer>, f: F) -> Vec<JoinHandle<T>>
where
    F: Fn(Peer) -> T + Copy,
    F: Send + 'static,
    T: Send + 'static,
{
    let mut ts = Vec::new();

    peers.drain(..).for_each(|p| {
        ts.push(thread::spawn(move || -> T { f(p) }));
    });

    ts
}
