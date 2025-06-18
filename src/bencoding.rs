use std::collections::HashMap;

pub enum Statement<'mainbuf> {
    Integer(i64),
    ByteString(&'mainbuf [u8]),
    List(Vec<Statement<'mainbuf>>),
    Dictionary(HashMap<&'mainbuf [u8], Statement<'mainbuf>>),
}

pub fn parse<'mainbuf>(buf: &'mainbuf Vec<u8>) -> Result<Vec<Statement<'mainbuf>>, String> {
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

pub fn marshal(st: &Statement) -> Vec<u8> {
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
