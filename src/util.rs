use std::io;

pub fn easy_err(msg: &str) -> io::Error {
    return std::io::Error::new(std::io::ErrorKind::Other, msg);
}
