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
