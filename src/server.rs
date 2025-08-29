use std::{io, net::TcpListener};

pub struct Server {
    pub s: TcpListener,
}

impl Server {
    pub fn start() -> Result<Self, io::Error> {
        let s = TcpListener::bind("0.0.0.0:6881")?;

        println!("started server, accepting connections at *:6881");

        Ok(Server { s: s })
    }
}
