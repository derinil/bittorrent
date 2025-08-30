use crate::{
    peer::{KEEP_ALIVE_MAX_DURATION, MessageType, Peer},
    server::Server,
    torrent::Block,
};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
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
    receiving: Vec<Block>,
    desired: HashSet<Block>,
    received: Vec<Block>,
    active_peers: Vec<Peer>,
}

struct PeerThread {
    peer: Peer,
    thread: Option<JoinHandle<()>>,
}

enum PeerThreadMessage {
    Ignore,
    CheckKeepAlive, // TODO: or some other message to receive messages
    Busy,
    CanRespond,
    SearchViablePeer(Block),
    FoundViablePeer(Block),
    RequestBlock(Block),
    DownloadedBlock(Block),
    GetPieces,
    HasPieces(HashSet<u32>),
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
            receiving: Vec::new(),
            desired: HashSet::new(),
            received: Vec::new(),
        })
    }

    pub fn connect_peers(self: &mut Self, mut peers: Vec<Peer>) {
        let mut ts: Vec<JoinHandle<()>> = Vec::new();

        for _ in 0..peers.len() {
            let mut peer = peers.remove(0);
            let info_hash = self.info_hash.clone();
            let t = thread::spawn(move || {
                match peer.connect() {
                    Ok(_) => {}
                    Err(e) => {
                        println!("failed to connect {:?}", e);
                        return;
                    }
                }
                match peer.handshake(info_hash) {
                    Ok(_) => {}
                    Err(e) => {
                        println!("failed to handshake {:?}", e);
                        if let Err(e) = peer.disconnect() {
                            println!("failed to disconnect client bc of failed handshake {:?}", e);
                        }
                        return;
                    }
                }
            });
            ts.push(t);
        }

        for _ in 0..ts.len() {
            let t = ts.remove(0);
            if let Err(e) = t.join() {
                println!("failed to join thread {:?}", e);
            }
        }
    }

    pub fn submit_desired_block(self: &mut Self, block: Block) {
        self.desired.insert(block);
    }

    pub fn handle(self: &mut Self) {
        loop {
            // let mut i = 0;
            // while i < self.active_threads.len() {
            //     if self.active_threads.get(i).unwrap().thread.is_finished() {
            //         if let Err(e) = self.active_threads.swap_remove(i).thread.join() {
            //             println!("failed to join peer thread {:?}", e);
            //         }
            //     } else {
            //         i += 1;
            //     }
            // }

            // println!("active connections {}", self.active_threads.len());

            // for at in &mut self.active_threads {
            //     match at.read_from_thread.try_recv() {
            //         Ok(msg) => match msg {
            //             PeerThreadMessage::Busy => {
            //                 at.can_respond = false;
            //             }
            //             PeerThreadMessage::CanRespond => {
            //                 at.can_respond = true;
            //             }
            //             _ => {}
            //         },
            //         Err(_) => {}
            //     }
            // }

            // // Download
            // for at in &self.active_threads {
            //     if let Err(e) = at.send_to_thread.send(PeerThreadMessage::GetPieces) {
            //         println!("failed to send thread msg {:?}", e);
            //         continue;
            //     }
            //     match at.read_from_thread.recv() {
            //         Ok(msg) => match msg {
            //             PeerThreadMessage::HasPieces(pieces) => {
            //                 for block in &self.desired {
            //                     if !pieces.contains(&block.piece_index) {
            //                         continue;
            //                     }
            //                     if let Err(e) = at
            //                         .send_to_thread
            //                         .send(PeerThreadMessage::SearchViablePeer(*block))
            //                     {
            //                         println!("failed to send request block msg {:?}", e);
            //                     }
            //                     match at.read_from_thread.recv() {
            //                         Ok(msg) => match msg {
            //                             PeerThreadMessage::FoundViablePeer(b) => {}
            //                             _ => {}
            //                         },
            //                         Err(e) => {
            //                             println!("failed to send request block msg {:?}", e);
            //                             continue;
            //                         }
            //                     }
            //                 }
            //             }
            //             _ => {}
            //         },
            //         Err(e) => {
            //             println!("failed to read thread msg {:?}", e);
            //             continue;
            //         }
            //     }
            // }

            // println!("waiting for viable peers");
            // for at in &mut self.active_threads {
            //     if !at.can_respond {
            //         continue;
            //     }
            //     let msg_err = at.read_from_thread.recv();
            //     if let Err(e) = msg_err {
            //         println!("failed to read thread msg {:?}", e);
            //         continue;
            //     }
            //     match msg_err.unwrap() {
            //         PeerThreadMessage::FoundViablePeer(block) => {
            //             println!("got viable peer message");
            //             if let Err(e) = at
            //                 .send_to_thread
            //                 .send(PeerThreadMessage::RequestBlock(block))
            //             {
            //                 println!("failed to send thread msg {:?}", e);
            //             }
            //             self.desired.remove(&block);
            //             at.can_respond = false;
            //             continue;
            //         }
            //         PeerThreadMessage::CanRespond => {
            //             // this will lag by one loop but this is fine
            //             // as it will be marked as not responsibve while searching for peers
            //             at.can_respond = true;
            //         }
            //         PeerThreadMessage::Busy => {
            //             at.can_respond = false;
            //         }
            //         _ => {}
            //     }
            // }

            // TODO: Upload
        }
    }
}

fn handle_peer(mut peer: Peer) {
    let send_to_thread = mpsc::channel();
    let read_from_thread = mpsc::channel();

    let peer_read_thread_message =
        |p: &mut Peer, recv: &Receiver<PeerThreadMessage>, sender: &Sender<PeerThreadMessage>| {
            match recv.recv().unwrap() {
                PeerThreadMessage::SearchViablePeer(block) => {
                    if !p.am_choked && p.has_piece(block.piece_index) {
                        println!("found viable peer");
                        p.set_interested(true).unwrap();
                        sender
                            .send(PeerThreadMessage::FoundViablePeer(block))
                            .unwrap();
                        println!("sent viable peer");
                    } else {
                        println!("found ignore peer");
                        sender.send(PeerThreadMessage::Ignore).unwrap();
                    }
                }
                PeerThreadMessage::RequestBlock(block) => {
                    if !p.has_piece(block.piece_index) {
                        return;
                    }
                    if !p.am_interested {
                        p.send_message(MessageType::Interested, None)
                            .expect("failed to send interested message");
                        p.am_interested = true;
                    }
                    let mut payload: Vec<u8> = Vec::new();
                    payload.extend(block.piece_index.to_be_bytes());
                    payload.extend(block.byte_offset.to_be_bytes());
                    payload.extend(block.requested_length.to_be_bytes());
                    p.send_message(MessageType::Request, Some(&payload))
                        .expect("failed to send request message");
                    println!("sent request")
                }
                // PeerThreadMessage::DownloadedBlock(block) => {
                //     let mut payload: Vec<u8> = Vec::new();
                //     payload.extend(block.piece_index.to_be_bytes());
                //     payload.extend(block.byte_offset.to_be_bytes());
                //     payload.extend(block.requested_length.to_be_bytes());
                //     p.send_message(MessageType::Cancel, Some(&payload))
                //         .expect("failed to send cancel message");
                //     println!("sent cancel")
                // }
                PeerThreadMessage::GetPieces => {
                    sender
                        .send(PeerThreadMessage::HasPieces(p.peer_has.clone()))
                        .unwrap();
                }
                _ => {}
            }
        };

    let peer_receive_message = |p: &mut Peer, sender: &Sender<PeerThreadMessage>| {
        match p.receive_message() {
            Ok(msg) => {
                match msg.message_type {
                    MessageType::KeepAlive => {}
                    MessageType::Choke => {
                        p.am_choked = true;
                    }
                    MessageType::Unchoke => {
                        p.am_choked = false;
                    }
                    MessageType::Interested => {
                        p.peer_interested = true;
                    }
                    MessageType::NotInterested => {
                        p.peer_interested = false;
                    }
                    MessageType::Have => {
                        if msg.payload.len() != 4 {
                            println!(
                                "peer sent incorrect have message length {}",
                                msg.payload.len()
                            );
                            return;
                        }
                        let have_idx = u32::from_be_bytes(msg.payload.try_into().unwrap());
                        p.peer_has.insert(have_idx);
                    }
                    MessageType::Bitfield => {
                        p.use_bitfield(&msg.payload);
                        println!("peer gotbitfield {:?} {}", p.peer_has, p.peer_has.len());
                    }
                    MessageType::Request => {
                        // TODO:
                    }
                    MessageType::Piece => {
                        let piece_idx =
                            u32::from_be_bytes(msg.payload.get(0..4).unwrap().try_into().unwrap());
                        let byte_offset =
                            u32::from_be_bytes(msg.payload.get(4..8).unwrap().try_into().unwrap());
                        println!("downloading piece {} {}", piece_idx, byte_offset);
                        let _ = fs::create_dir("./download");
                        fs::write(
                            format!("./download/{}-{}", piece_idx, byte_offset),
                            msg.payload.get(8..).unwrap(),
                        )
                        .expect("failed to write block file");
                        sender
                            .send(PeerThreadMessage::DownloadedBlock(Block::new(
                                piece_idx,
                                byte_offset,
                            )))
                            .unwrap();
                    }
                    MessageType::Cancel => {
                        // TODO:
                    }
                    MessageType::Port => {
                        // TODO: dht
                    }
                };
                p.last_message_at = Some(time::Instant::now());
            }
            Err(err) => {
                println!("failed to receive message {}", err);
                return;
            }
        }
    };

    let peer_thread_closure = move || {
        let recv: Receiver<PeerThreadMessage> = send_to_thread.1;
        let send: Sender<PeerThreadMessage> = read_from_thread.0;

        if let Some(s) = peer.get_peer_id().unwrap() {
            println!("handling peer {:?}", s);
        }

        loop {
            peer_read_thread_message(&mut peer, &recv, &send);

            let has_data = match peer.has_data() {
                Ok(b) => b,
                Err(e) => {
                    if e.kind() != io::ErrorKind::WouldBlock {
                        println!("error checking has_data {:?}", e);
                    }
                    false
                }
            };
            if has_data {
                send.send(PeerThreadMessage::Busy).unwrap();
                peer_receive_message(&mut peer, &send);
                send.send(PeerThreadMessage::CanRespond).unwrap();
            }

            if let Some(t) = peer.last_message_at {
                if t.elapsed() >= KEEP_ALIVE_MAX_DURATION {
                    println!("peer exceeded max keep alive duration");
                    return;
                }
            }
        }
    };

    let t = thread::spawn(|| {
        peer_thread_closure();
    });

    // PeerThread {
    //     thread: t,
    //     can_respond: true,
    //     send_to_thread: send_to_thread.0,
    //     read_from_thread: read_from_thread.1,
    // }
}
