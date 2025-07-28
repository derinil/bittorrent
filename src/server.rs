use std::{io, net::TcpListener};

pub struct Server {
    s: TcpListener,
}

impl Server {
    pub fn start() -> Result<(), io::Error> {
        let s = TcpListener::bind("0.0.0.0:6881")?;

        println!("started server, accepting connections at *:6881");

        loop {
            let conn_tup = match s.accept() {
                Ok(c) => c,
                Err(e) => {
                    println!("got error accepting connection {e:?}");
                    continue;
                }
            };

            println!("accepted connection from {}", conn_tup.1.to_string());
        }
    }
}
