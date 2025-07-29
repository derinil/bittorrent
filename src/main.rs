use std::fs::File;
use std::io::Read;
use std::thread;
use std::time;
use std::{env, io, process};

use crate::server::Server;

mod bencoding;
mod peer;
mod server;
mod torrent;
mod udp;
mod util;

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

    let peer_id = create_peer_id();
    println!(
        "created peer id {}",
        String::from_utf8(peer_id.to_vec()).unwrap()
    );

    let torr = torrent::Torrent::parse(file_content).expect("failed to parse torrent");

    let listener = thread::spawn(|| match start_server() {
        Ok(_) => {}
        Err(err) => {
            println!("server returned error: {}", err);
        }
    });

    'announceLoop: for announcer in torr.announce_urls.split_at(2).1 {
        if announcer.starts_with("udp://") {
            let u = announcer.split("udp://").nth(1).unwrap();
            // "tracker.opentrackr.org:1337"
            match handle_udp_tracker(u) {
                Some(mut tr) => {
                    handle_download(&mut tr, torr.info_hash, peer_id).unwrap();
                    break 'announceLoop;
                }
                None => {}
            }
        } else {
            println!("skipping non udp announcer {}", announcer);
            // get_request(base_url, &peer_id, &info_hash, total_bytes).unwrap();
        }
    }

    _ = listener.join();
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
    tr: &mut udp::Tracker,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
) -> Result<(), std::io::Error> {
    println!("downloading");
    let mut seeders = tr.announce(info_hash, peer_id, 1)?;
    if seeders.len() == 0 {
        println!("finishing download, no seeders found");
        return Ok(());
    }

    for seeder in seeders.iter_mut() {
        // if seeder.addr.to_string() != "212.32.48.136:56462" {
        if seeder.ip_address != 3558879368 {
            continue;
        }

        seeder.connect()?;

        match seeder.handshake(info_hash, peer_id) {
            Ok(_) => {
                println!("finished handshake")
            }
            Err(err) => {
                println!("failed to handshake {err}")
            }
        };
    }

    println!("download done");

    Ok(())
}

fn start_server() -> Result<(), io::Error> {
    Server::start()?;
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
