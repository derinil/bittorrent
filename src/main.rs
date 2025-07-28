use percent_encoding::{self, NON_ALPHANUMERIC};
use sha1_smol;
use std;
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::net::TcpStream;
use std::thread;
use std::time;

mod bencoding;
mod peer;
mod udp;

fn main() {
    const FILE_NAME: &str = "./wired-cd.torrent";
    // const FILE_NAME: &str = "./new.torrent";
    let mut file = File::open(FILE_NAME).unwrap();
    let mut file_content: Vec<u8> = Vec::new();
    file.read_to_end(&mut file_content).unwrap();
    file_content = file_content.trim_ascii_end().to_vec();
    println!("read torrent file {}", file_content.len());

    let statements = bencoding::parse(&file_content).unwrap();
    println!("parsed torrent file with {} statements", statements.len());

    if statements.len() == 0 {
        println!("got 0 statements, exiting");
        return;
    }

    let metainfo = match &statements[0] {
        bencoding::Statement::Dictionary(map) => map,
        _ => {
            println!("metainfo dict is not dict");
            return;
        }
    };

    if metainfo.is_empty() {
        println!("main dict is empty");
        return;
    }

    let mut announce_urls = Vec::new();

    match &metainfo.get("announce-list".as_bytes()) {
        Some(v) => match v {
            bencoding::Statement::List(list) => {
                for s in list {
                    match s {
                        bencoding::Statement::List(sublist) => {
                            for ss in sublist {
                                match ss {
                                    bencoding::Statement::ByteString(str) => {
                                        announce_urls.push(str::from_utf8(str).unwrap());
                                    }
                                    _ => {
                                        println!("announce list url is not string");
                                        return;
                                    }
                                }
                            }
                        }
                        _ => {
                            println!("announce list element is not list");
                            return;
                        }
                    }
                }
            }
            _ => {
                println!("announce list is not list");
                return;
            }
        },
        _ => {}
    };

    if !metainfo.contains_key("announce-list".as_bytes()) {
        let announce_url = match &metainfo.get("announce".as_bytes()).unwrap() {
            bencoding::Statement::ByteString(link) => str::from_utf8(link).unwrap(),
            _ => {
                println!("announce url is not string");
                return;
            }
        };
        announce_urls.push(announce_url);
    }

    println!("got announce url {:?}", announce_urls);

    let peer_id = create_peer_id();
    println!(
        "created peer id {}",
        String::from_utf8(peer_id.to_vec()).unwrap()
    );

    let info = match &metainfo.get("info".as_bytes()).unwrap() {
        bencoding::Statement::Dictionary(map) => map,
        _ => {
            println!("info dict is not dict");
            return;
        }
    };

    if let bencoding::Statement::ByteString(name) = &info.get("name".as_bytes()).unwrap() {
        println!("got file name {}", str::from_utf8(name).unwrap());
    }

    let info_hash_bs =
        sha1_smol::Sha1::from(bencoding::marshal(metainfo.get("info".as_bytes()).unwrap()))
            .digest()
            .bytes();

    let info_hash: String =
        percent_encoding::percent_encode(&info_hash_bs, NON_ALPHANUMERIC).to_string();

    let mut total_bytes: u64 = 0;
    match &info.get("length".as_bytes()) {
        Some(l) => {
            if let bencoding::Statement::Integer(len) = l {
                total_bytes = *len as u64;
            }
        }
        _ => {}
    }
    if total_bytes == 0 {
        if let bencoding::Statement::List(fs) = &info.get("files".as_bytes()).unwrap() {
            fs.iter().for_each(|s| {
                if let bencoding::Statement::Dictionary(dict) = s {
                    if let bencoding::Statement::Integer(len) =
                        &dict.get("length".as_bytes()).unwrap()
                    {
                        total_bytes += *len as u64;
                    }
                }
            });
        }
    }

    println!("got total bytes {} and hash {}", total_bytes, info_hash);

    let listener = thread::spawn(|| match start_server() {
        Ok(_) => {}
        Err(err) => {
            println!("server returned error: {}", err);
        }
    });

    'announceLoop: for announcer in announce_urls.split_at(2).1 {
        if announcer.starts_with("udp://") {
            let u = announcer.split("udp://").nth(1).unwrap();
            // "tracker.opentrackr.org:1337"
            match handle_udp_tracker(u) {
                Some(mut tr) => {
                    handle_download(&mut tr, info_hash_bs, peer_id).unwrap();
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
    let seeders = tr.announce(info_hash, peer_id, 1)?;
    if seeders.len() == 0 {
        println!("finishing download, no seeders found");
        return Ok(());
    }
    match seeders.get(0) {
        Some(seeder) => {
            seeder.handshake(info_hash, peer_id)?;
        },
        None => {}
    }
    Ok(())
}

fn start_server() -> Result<(), std::io::Error> {
    let s = TcpListener::bind(":6881")?;
    _ = s;
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

fn get_request(
    u: &str,
    peer_id: &str,
    info_hash: &str,
    total_bytes: u64,
) -> Result<(), std::io::Error> {
    let mut a = u.to_string();
    a.push_str(":80");
    let mut stream = TcpStream::connect(&a).unwrap();

    let payload = format!(
        "GET /?port={}&info_hash={}&peer_id={}&uploaded=0&downloaded=0&left={}&compact=1&event=started HTTP/1.1\r\nHost: {}\r\nUser-Agent: bittorrent001\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        6881, info_hash, peer_id, total_bytes, u
    );

    println!("sending payload {}", payload);

    stream.write_all(payload.as_bytes())?;

    let mut resp = Vec::new();
    stream.read_to_end(&mut resp)?;

    println!("got response {}", str::from_utf8(&resp).unwrap());

    Ok(())
}

fn get_base_url(u: &str) -> &str {
    let mut prot = u.find("://").unwrap_or_default();
    prot += if prot > 0 { 3 } else { 0 };
    let mut end = u[prot..]
        .find(':')
        .or(u[prot..].find('/'))
        .or(u[prot..].find('?'))
        .unwrap_or(u.len());
    end += if end != u.len() { prot } else { 0 };
    return &u[prot..end];
}
