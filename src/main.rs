use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::sync::OnceLock;
use std::time;

use crate::peer_pool::PeerPool;
use crate::torrent::Torrent;

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

    let file_name = args.get(1).expect("torrent file name is missing");
    let download_file_name = args.get(2).expect("download file name is missing");

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

    let mut pool = peer_pool::PeerPool::new(torr.clone(), download_file_name.clone())
        .expect("failed to create shared peer pool");

    println!("creating target download file");

    // TODO: seed mode which seeds directly from file
    let download_file = fs::File::create_new(download_file_name).unwrap();
    download_file.set_len(torr.total_size).unwrap();

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
    let seeders = tr.announce(torr.info_hash, PEER_ID.get().unwrap(), 1)?;
    if seeders.len() == 0 {
        println!("cancelling download, no seeders found");
        return Ok(());
    }

    pp.connect_peers(seeders);

    println!("pool setup ready");

    Ok(())
}

fn create_peer_id() -> [u8; 20] {
    let mut v: Vec<u8> = Vec::new();
    v.extend("dips-001-".as_bytes());

    let mut nanos = time::SystemTime::now()
        .duration_since(time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    while v.len() != 20 {
        v.push((nanos % 10) as u8 + '0' as u8);
        nanos >>= 2;
    }

    v.try_into().unwrap()
}
