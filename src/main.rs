use std::fs::File;
use std::io::Read;
use std::sync::OnceLock;
use std::time;
use std::{env, process};

use crate::peer_pool::PeerPool;
use crate::torrent::{Block, DEFAULT_BLOCK_LENGTH, Torrent};

mod bencoding;
mod peer;
mod peer_pool;
mod server;
mod torrent;
mod udp;
mod util;

static PEER_ID: OnceLock<[u8; 20]> = OnceLock::new();

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("missing torrent filename");
        process::exit(1);
    }

    let file_name = args.get(1).unwrap();

    println!("downloading torrent {}", file_name);

    let mut file = File::open(file_name).unwrap();
    let mut file_content: Vec<u8> = Vec::new();
    file.read_to_end(&mut file_content).unwrap();
    file_content = file_content.trim_ascii_end().to_vec();
    println!("read torrent file {}", file_content.len());

    PEER_ID
        .set(create_peer_id())
        .expect("failed to set peer id");
    println!(
        "created peer id {}",
        String::from_utf8(PEER_ID.get().unwrap().to_vec()).unwrap()
    );

    let torr = torrent::Torrent::parse(file_content).expect("failed to parse torrent");

    let mut pool =
        peer_pool::PeerPool::new(torr.info_hash).expect("failed to create shared peer pool");

    'announceLoop: for announcer in torr.announce_urls.split_at(2).1 {
        if announcer.starts_with("udp://") {
            let u = announcer.split("udp://").nth(1).unwrap();
            // "tracker.opentrackr.org:1337"
            match handle_udp_tracker(u) {
                Some(mut tr) => {
                    handle_download(&mut pool, &mut tr, torr).unwrap();
                    break 'announceLoop;
                }
                None => {}
            }
        } else {
            println!("skipping non udp announcer {}", announcer);
            // get_request(base_url, &peer_id, &info_hash, total_bytes).unwrap();
        }
    }

    pool.handle();
}

fn handle_udp_tracker(u: &str) -> Option<udp::Tracker> {
    println!("attempting udp connection to {}", u);
    let mut t = udp::Tracker::new().unwrap();
    match t.initiate(u) {
        Ok(_) => {}
        Err(e) => {
            println!("failed to connect {}", e);
            return None;
        }
    };
    println!("connected");
    return Some(t);
}

fn handle_download(
    pp: &mut PeerPool,
    tr: &mut udp::Tracker,
    torr: Torrent,
) -> Result<(), std::io::Error> {
    println!("downloading");
    let mut seeders = tr.announce(torr.info_hash, PEER_ID.get().unwrap(), 1)?;
    if seeders.len() == 0 {
        println!("finishing download, no seeders found");
        return Ok(());
    }

    for _ in 0..seeders.len() {
        pp.submit_peer(seeders.remove(0));
    }

    for piece_idx in 0..torr.piece_len - 1 {
        let mut block_start = 0;
        while block_start + DEFAULT_BLOCK_LENGTH < torr.piece_len {
            pp.submit_desired_block(Block::new(piece_idx, block_start));
            block_start += DEFAULT_BLOCK_LENGTH;
        }
        pp.submit_desired_block(Block::new(piece_idx, block_start));
    }

    println!("download done");

    Ok(())
}

fn create_peer_id() -> [u8; 20] {
    let mut bs: [u8; 20] = [0; 20];
    let dip = "dips";
    let mut off = 0;
    bs[off..off + dip.len()].copy_from_slice(dip.as_bytes());
    off += dip.len();
    let ver = "001";
    bs[off..off + ver.len()].copy_from_slice(ver.as_bytes());
    off += ver.len();
    let now = time::SystemTime::now();
    let t = now
        .duration_since(time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    // TODO: this can overflow
    bs[off..off + t.len()].copy_from_slice(t.as_bytes());
    bs = bs.map(|n| -> u8 {
        if n == 0 { b'D' } else { n }
    });
    bs
}
