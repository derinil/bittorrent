use crate::{bencoding, util::easy_err};
use percent_encoding::{self, NON_ALPHANUMERIC};
use sha1_smol;
use std::io;

pub struct Torrent {
    pub info_hash: [u8; 20],
    pub announce_urls: Vec<String>,
    pub piece_len: u64,
    pub piece_hashes: Vec<[u8; 20]>,
    pub total_size: u64,
}

pub struct Block {
    pub torrent_hash: [u8; 20],
    pub piece_index: u64,
    pub byte_offset: u64,
    pub requested_length: u64,
}

pub const DEFAULT_BLOCK_LENGTH: u64 = 16384;

impl Block {
    pub fn new(torrent_hash: [u8; 20], piece_index: u64, byte_offset: u64) -> Block {
        Block {
            torrent_hash: torrent_hash,
            piece_index: piece_index,
            byte_offset: byte_offset,
            requested_length: DEFAULT_BLOCK_LENGTH,
        }
    }
}

impl Torrent {
    pub fn parse(buf: Vec<u8>) -> Result<Self, io::Error> {
        let statements = bencoding::parse(&buf).unwrap();

        if statements.len() == 0 {
            return Err(easy_err("got no statements"));
        }

        let metainfo = match &statements[0] {
            bencoding::Statement::Dictionary(map) => map,
            _ => {
                return Err(easy_err("metainfo dict is not dict"));
            }
        };

        if metainfo.is_empty() {
            return Err(easy_err("metainfo dict is empty"));
        }

        let mut announce_urls = Vec::new();

        // TODO: this is so ugly
        if let Some(announce_list) = metainfo.get("announce-list".as_bytes()) {
            match announce_list {
                bencoding::Statement::List(list) => {
                    for s in list {
                        match s {
                            bencoding::Statement::List(sublist) => {
                                for ss in sublist {
                                    match ss {
                                        bencoding::Statement::ByteString(str) => {
                                            announce_urls.push(
                                                String::try_from(str::from_utf8(str).unwrap())
                                                    .unwrap(),
                                            );
                                        }
                                        _ => {
                                            return Err(easy_err(
                                                "announce list url is not string",
                                            ));
                                        }
                                    }
                                }
                            }
                            _ => {
                                return Err(easy_err("announce list element is not list"));
                            }
                        }
                    }
                }
                _ => {
                    return Err(easy_err("announce list is not list"));
                }
            }
        }

        if !metainfo.contains_key("announce-list".as_bytes()) {
            let announce_url = match &metainfo.get("announce".as_bytes()).unwrap() {
                bencoding::Statement::ByteString(link) => {
                    String::try_from(str::from_utf8(link).unwrap()).unwrap()
                }
                _ => {
                    return Err(easy_err("announce url is not string"));
                }
            };
            announce_urls.push(announce_url);
        }

        println!("got announce url {:?}", announce_urls);

        let info = match &metainfo.get("info".as_bytes()).unwrap() {
            bencoding::Statement::Dictionary(map) => map,
            _ => {
                return Err(easy_err("info dict is not dict"));
            }
        };

        if let bencoding::Statement::ByteString(name) = &info.get("name".as_bytes()).unwrap() {
            println!("got file name {}", str::from_utf8(name).unwrap());
        }

        let info_hash_bs =
            sha1_smol::Sha1::from(bencoding::marshal(metainfo.get("info".as_bytes()).unwrap()))
                .digest()
                .bytes();

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

        let mut piece_length = 0;
        if let bencoding::Statement::Integer(pl) = &info.get("piece length".as_bytes()).unwrap() {
            piece_length = *pl;
            println!("got piece length {piece_length}");
        }

        let mut pieces = Vec::new();
        if let bencoding::Statement::ByteString(s) = &info.get("pieces".as_bytes()).unwrap() {
            if s.len() % 20 != 0 {
                return Err(easy_err("pieces length is not a multiple of 20"));
            }
            for i in 0..s.len() / 20 {
                pieces.push(s[i..i + 20].try_into().unwrap());
            }
        }

        println!("got piece hashes {}", pieces.len());

        let mut total_len = 0;
        if info.contains_key("files".as_bytes()) {
            if let bencoding::Statement::List(ld) = &info.get("files".as_bytes()).unwrap() {
                for d in ld {
                    if let Some(num) = get_file_len(d) {
                        total_len += num;
                    }
                }
            }
        } else {
            if let Some(num) = get_file_len(&statements[0]) {
                total_len += num;
            }
        }

        println!("got total size {total_len}");

        let s = Self {
            info_hash: info_hash_bs,
            announce_urls: announce_urls,
            piece_len: piece_length as u64,
            total_size: total_len,
            piece_hashes: pieces,
        };

        println!(
            "got total bytes {} and hash {}",
            total_bytes,
            s.get_info_hash_str()
        );

        println!("got total pieces {}", s.get_total_piece_count());

        if s.get_total_piece_count() != s.piece_hashes.len() as u64 {
            return Err(easy_err(
                "total piece count is not equal to piece hashes length",
            ));
        }

        Ok(s)
    }

    pub fn get_total_piece_count(self: &Self) -> u64 {
        (self.total_size as f64 / self.piece_len as f64).ceil() as u64
    }

    pub fn get_info_hash_str(self: &Self) -> String {
        percent_encoding::percent_encode(&self.info_hash, NON_ALPHANUMERIC).to_string()
    }
}

fn get_file_len(d: &bencoding::Statement) -> Option<u64> {
    if let bencoding::Statement::Dictionary(dict) = d {
        dict.get("length".as_bytes()).and_then(|len_stmt| {
            if let bencoding::Statement::Integer(n) = len_stmt {
                Some(*n as u64)
            } else {
                None
            }
        })
    } else {
        None
    }
}
