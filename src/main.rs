use std::fs::File;
use std::io::Read;
use std::sync::OnceLock;
use std::time;
use std::{env, process};

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

    PEER_ID.set(create_peer_id()).expect("failed to set peer id");
    println!(
        "created peer id {}",
        String::from_utf8(PEER_ID.get().unwrap().to_vec()).unwrap()
    );

    let torr = torrent::Torrent::parse(file_content).expect("failed to parse torrent");

    let mut pool = peer_pool::SharedPeerPool::new(torr.info_hash).expect("failed to create shared peer pool");
    pool.start();

    'announceLoop: for announcer in torr.announce_urls.split_at(2).1 {
        if announcer.starts_with("udp://") {
            let u = announcer.split("udp://").nth(1).unwrap();
            // "tracker.opentrackr.org:1337"
            match handle_udp_tracker(u) {
                Some(mut tr) => {
                    handle_download(&mut pool, &mut tr, torr.info_hash).unwrap();
                    break 'announceLoop;
                }
                None => {}
            }
        } else {
            println!("skipping non udp announcer {}", announcer);
            // get_request(base_url, &peer_id, &info_hash, total_bytes).unwrap();
        }
    }
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
    pp: &mut peer_pool::SharedPeerPool,
    tr: &mut udp::Tracker,
    info_hash: [u8; 20],
) -> Result<(), std::io::Error> {
    println!("downloading");
    let mut seeders = tr.announce(info_hash, PEER_ID.get().unwrap(), 1)?;
    if seeders.len() == 0 {
        println!("finishing download, no seeders found");
        return Ok(());
    }

    let mut i = 0;
    let mut max = seeders.len();
    while i < max {
        if let Err(e) = seeders.get_mut(i).unwrap().connect() {
            println!(
                "failed to connect to seeder {:?} {:?}",
                seeders.get_mut(i).unwrap(),
                e
            );
            i += 1;
            continue;
        }

        match seeders.get_mut(i).unwrap().handshake(info_hash) {
            Ok(_) => {
                println!("finished handshake")
            }
            Err(err) => {
                println!("failed to handshake {err}");
                i += 1;
                continue;
            }
        };

        pp.submit_peer(seeders.remove(i));
        max -= 1;
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
