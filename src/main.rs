use std;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;

enum Statement<'mainbuf> {
    Integer(i64),
    ByteString(&'mainbuf [u8]),
    List(Vec<Statement<'mainbuf>>),
    Dictionary(HashMap<&'mainbuf [u8], Statement<'mainbuf>>),
}

fn main() {
    const FILE_NAME: &str = "./big-buck-bunny.torrent";
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

    println!("got announce url {} and base url {}", announce_url, base_url);

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

    return Ok(statements);
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
