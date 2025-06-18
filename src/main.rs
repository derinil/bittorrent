use sha1_smol;
use std;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time;

enum Statement<'mainbuf> {
    Integer(i64),
    ByteString(&'mainbuf [u8]),
    List(Vec<Statement<'mainbuf>>),
    Dictionary(HashMap<&'mainbuf [u8], Statement<'mainbuf>>),
}

const PORT: u64 = 6884;

fn main() {
    const FILE_NAME: &str = "./wired-cd.torrent";
    // const FILE_NAME: &str = "./new.torrent";
    let mut file = File::open(FILE_NAME).unwrap();
    let mut file_content: Vec<u8> = Vec::new();
    file.read_to_end(&mut file_content).unwrap();
    file_content = file_content.trim_ascii_end().to_vec();
    println!("read torrent file {}", file_content.len());

    let statements = parse_bencoding(&file_content).unwrap();
    println!("parsed torrent file with {} statements", statements.len());

    if statements.len() == 0 {
        println!("got 0 statements, exiting");
        return;
    }

    let mut out_file = File::create("out.torrent").unwrap();
    out_file
        .write_all(&marshal_bencoding(&statements[0]))
        .unwrap();

    let metainfo = match &statements[0] {
        Statement::Dictionary(map) => map,
        _ => {
            println!("metainfo dict is not dict");
            return;
        }
    };

    if metainfo.is_empty() {
        println!("main dict is empty");
        return;
    }

    let announce_url = match &metainfo.get("announce".as_bytes()).unwrap() {
        Statement::ByteString(link) => str::from_utf8(link).unwrap(),
        _ => {
            println!("announce url is not string");
            return;
        }
    };

    let base_url = get_base_url(announce_url);

    println!(
        "got announce url {} and base url {}",
        announce_url, base_url
    );

    let peer_id = create_peer_id();
    println!("created peer id {}", peer_id);

    let info = match &metainfo.get("info".as_bytes()).unwrap() {
        Statement::Dictionary(map) => map,
        _ => {
            println!("info dict is not dict");
            return;
        }
    };

    if let Statement::ByteString(name) = &info.get("name".as_bytes()).unwrap() {
        println!("got file name {}", str::from_utf8(name).unwrap());
    }

    let info_hash =
        sha1_smol::Sha1::from(marshal_bencoding(metainfo.get("info".as_bytes()).unwrap()))
            .digest()
            .to_string();

    let mut total_bytes: u64 = 0;
    match &info.get("length".as_bytes()) {
        Some(l) => {
            if let Statement::Integer(len) = l {
                total_bytes = *len as u64;
            }
        }
        _ => {}
    }
    if total_bytes == 0 {
        if let Statement::List(fs) = &info.get("files".as_bytes()).unwrap() {
            fs.iter().for_each(|s| {
                if let Statement::Dictionary(dict) = s {
                    if let Statement::Integer(len) = &dict.get("length".as_bytes()).unwrap() {
                        total_bytes += *len as u64;
                    }
                }
            });
        }
    }

    println!("got total bytes {} and hash {}", total_bytes, info_hash);

    get_request(base_url, &peer_id, &info_hash, total_bytes).unwrap();
}

fn create_peer_id() -> String {
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
    String::from_utf8(bs.to_vec()).unwrap()
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
        PORT, info_hash, peer_id, total_bytes, u
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

fn parse_bencoding<'mainbuf>(buf: &'mainbuf Vec<u8>) -> Result<Vec<Statement<'mainbuf>>, String> {
    let mut idx: usize = 0;
    let mut statements: Vec<Statement> = Vec::new();
    statements.reserve(5);

    while idx < buf.len() {
        let res = handle_statement(&buf[idx..]).unwrap();
        statements.push(res.0);
        idx += res.1 + 1;
    }

    Ok(statements)
}

fn handle_statement<'mainbuf>(buf: &'mainbuf [u8]) -> Result<(Statement<'mainbuf>, usize), String> {
    let mut idx = 0;

    match buf[idx] {
        b'i' => {
            // Integer
            idx += 1;
            let begin = idx;
            while idx < buf.len() && buf[idx] != b'e' {
                idx += 1;
            }
            if idx >= buf.len() {
                return Err(String::from("integer has no end"));
            }
            let end = idx;
            return Ok((
                Statement::Integer(
                    str::from_utf8(&buf[begin..end])
                        .unwrap()
                        .parse::<i64>()
                        .unwrap(),
                ),
                idx,
            ));
        }
        b'l' => {
            // List
            idx += 1;
            let mut l: Vec<Statement<'mainbuf>> = Vec::new();
            while idx < buf.len() && buf[idx] != b'e' {
                let res = handle_statement(&buf[idx..]).unwrap();
                l.push(res.0);
                idx += res.1 + 1;
            }
            return Ok((Statement::List(l), idx));
        }
        b'd' => {
            // Dictionary
            idx += 1;
            let mut m: HashMap<&'mainbuf [u8], Statement<'mainbuf>> = HashMap::new();
            while idx < buf.len() && buf[idx] != b'e' {
                let key = handle_statement(&buf[idx..]).unwrap();
                idx += key.1 + 1;
                if let Statement::ByteString(key_str) = key.0 {
                    let value = handle_statement(&buf[idx..]).unwrap();
                    idx += value.1 + 1;
                    m.insert(key_str, value.0);
                } else {
                    return Err(String::from("Dictionary key must be string"));
                }
            }
            return Ok((Statement::Dictionary(m), idx));
        }
        _ => {
            // Byte string
            if idx >= buf.len() {
                return Err(String::from("index is over buf length"));
            }
            let begin = idx;
            while idx < buf.len() && buf[idx] >= b'0' && buf[idx] <= b'9' {
                idx += 1;
            }
            if idx >= buf.len() {
                return Err(String::from("string length has no end"));
            }
            let end = idx;
            if buf[idx] != b':' {
                return Err(String::from("string length has no colon"));
            }
            idx += 1;
            let strlen = str::from_utf8(&buf[begin..end])
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let s = Statement::ByteString(&buf[idx..idx + strlen]);
            idx += strlen - 1;
            return Ok((s, idx));
        }
    }
}

fn marshal_bencoding(st: &Statement) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.reserve(500);

    marshal_statement(&mut buf, st);

    buf
}

fn marshal_statement(buf: &mut Vec<u8>, st: &Statement) {
    match st {
        Statement::Integer(num) => {
            buf.push(b'i');
            buf.extend(num.to_string().as_bytes());
            buf.push(b'e');
        }
        Statement::ByteString(str) => {
            buf.extend(str.len().to_string().as_bytes());
            buf.push(b':');
            buf.extend(*str);
        }
        Statement::List(list) => {
            buf.push(b'l');
            list.iter().for_each(|s| {
                marshal_statement(buf, s);
            });
            buf.push(b'e');
        }
        Statement::Dictionary(dict) => {
            buf.push(b'd');
            let mut keys: Vec<&&[u8]> = dict.keys().collect();
            keys.sort();
            keys.iter().for_each(|k| {
                let s = dict.get(*k).unwrap();
                buf.extend(k.len().to_string().as_bytes());
                buf.push(b':');
                buf.extend(**k);
                marshal_statement(buf, s);
            });
            buf.push(b'e');
        }
    }
}
