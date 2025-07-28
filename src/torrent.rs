use crate::{bencoding, util::easy_err};
use percent_encoding::{self, NON_ALPHANUMERIC};
use sha1_smol;
use std::io;

pub struct Torrent {
    info_hash: [u8; 20],
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
                                            announce_urls.push(str::from_utf8(str).unwrap());
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
                bencoding::Statement::ByteString(link) => str::from_utf8(link).unwrap(),
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

        let s = Self {
            info_hash: info_hash_bs,
        };

        println!(
            "got total bytes {} and hash {}",
            total_bytes,
            s.get_info_hash_str()
        );

        Ok(s)
    }

    pub fn get_info_hash_str(self: &Self) -> String {
        percent_encoding::percent_encode(&self.info_hash, NON_ALPHANUMERIC).to_string()
    }
}
