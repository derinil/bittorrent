use crate::{bencoding, util::easy_err};
use percent_encoding::{self, NON_ALPHANUMERIC};
use sha1_smol;
use std::io;

pub const DEFAULT_BLOCK_LENGTH: u32 = 16384;

#[derive(Clone)]
pub struct Torrent {
    pub info_hash: [u8; 20],
    pub announce_urls: Vec<String>,
    pub piece_len: u32,
    pub piece_hashes: Vec<[u8; 20]>,
    pub total_size: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Block {
    pub piece_index: u32,
    pub byte_offset: u32,
    pub requested_length: u32,
}

pub struct DownloadBlock {
    pub piece_index: u32,
    pub byte_offset: u32,
    pub data: Vec<u8>,
}

impl Block {
    pub fn new(piece_index: u32, byte_offset: u32, requested_length: u32) -> Block {
        Block {
            piece_index: piece_index,
            byte_offset: byte_offset,
            requested_length: requested_length,
        }
    }

    pub fn parse(payload: &Vec<u8>) -> Result<Block, io::Error> {
        if payload.len() < 12 {
            return Err(easy_err("payload too short to be block"));
        }

        Ok(Block {
            piece_index: u32::from_be_bytes(payload.get(0..4).unwrap().try_into().unwrap()),
            byte_offset: u32::from_be_bytes(payload.get(4..8).unwrap().try_into().unwrap()),
            requested_length: u32::from_be_bytes(payload.get(8..12).unwrap().try_into().unwrap()),
        })
    }

    pub fn to_bytes(&self) -> [u8; 12] {
        let mut b = [0 as u8; 12];

        b[0..4].copy_from_slice(&self.piece_index.to_be_bytes());
        b[4..8].copy_from_slice(&self.byte_offset.to_be_bytes());
        b[8..12].copy_from_slice(&self.requested_length.to_be_bytes());

        b
    }
}

impl DownloadBlock {
    pub fn parse(payload: &Vec<u8>) -> Result<DownloadBlock, io::Error> {
        if payload.len() < 9 {
            return Err(easy_err("payload too short to be download block"));
        }

        Ok(DownloadBlock {
            piece_index: u32::from_be_bytes(payload.get(0..4).unwrap().try_into().unwrap()),
            byte_offset: u32::from_be_bytes(payload.get(4..8).unwrap().try_into().unwrap()),
            data: payload.get(8..).unwrap().to_vec(),
        })
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
            piece_len: piece_length as u32,
            total_size: total_len,
            piece_hashes: pieces,
        };

        println!(
            "got total bytes {} and hash {}",
            total_bytes,
            s.get_info_hash_str()
        );

        println!("got total pieces {}", s.get_total_piece_count());

        if s.get_total_piece_count() != s.piece_hashes.len() as u32 {
            return Err(easy_err(
                "total piece count is not equal to piece hashes length",
            ));
        }

        Ok(s)
    }

    pub fn get_total_piece_count(self: &Self) -> u32 {
        (self.total_size as f64 / self.piece_len as f64).ceil() as u32
    }

    pub fn get_piece_len(self: &Self, piece: u32) -> u32 {
        if piece == self.get_total_piece_count() - 1 {
            return (self.total_size as u32 - ((piece - 1) * self.piece_len)) as u32;
        }

        self.piece_len as u32
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
