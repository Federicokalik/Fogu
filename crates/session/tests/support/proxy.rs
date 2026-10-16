//! Tiny in-process SOCKS5 / HTTP CONNECT proxies for the transport tests: each accepted
//! connection is handshaked, connected to the requested target and piped both ways on plain
//! threads. What each client asked for is recorded.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;

/// Which protocol the proxy speaks.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Socks5,
    Http,
}

/// A running proxy on 127.0.0.1.
pub struct MockProxy {
    pub port: u16,
    /// One entry per connection: `domain localhost:22`, `ipv4 127.0.0.1:22` (SOCKS5) or the
    /// request line (HTTP).
    pub requests: Arc<Mutex<Vec<String>>>,
}

impl MockProxy {
    pub fn start(kind: Kind) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind proxy");
        let port = listener.local_addr().expect("proxy addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let log = Arc::clone(&log);
                thread::spawn(move || {
                    if let Err(err) = serve(kind, client, &log) {
                        eprintln!("mock proxy: {err}");
                    }
                });
            }
        });
        Self { port, requests }
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }
}

fn serve(kind: Kind, mut client: TcpStream, log: &Mutex<Vec<String>>) -> io::Result<()> {
    let (record, target) = match kind {
        Kind::Socks5 => socks5(&mut client)?,
        Kind::Http => http(&mut client)?,
    };
    log.lock().expect("log").push(record);
    let upstream = TcpStream::connect(target)?;
    match kind {
        Kind::Socks5 => client.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?,
        Kind::Http => client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")?,
    }
    pipe(client, upstream)
}

fn socks5(client: &mut TcpStream) -> io::Result<(String, std::net::SocketAddr)> {
    let mut head = [0u8; 2];
    client.read_exact(&mut head)?;
    let mut methods = vec![0u8; usize::from(head[1])];
    client.read_exact(&mut methods)?;
    client.write_all(&[5, 0])?;
    let mut req = [0u8; 4];
    client.read_exact(&mut req)?;
    let (record, host) = match req[3] {
        1 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip)?;
            let ip = Ipv4Addr::from(ip).to_string();
            (format!("ipv4 {ip}"), ip)
        }
        4 => {
            let mut ip = [0u8; 16];
            client.read_exact(&mut ip)?;
            let ip = Ipv6Addr::from(ip).to_string();
            (format!("ipv6 {ip}"), ip)
        }
        _ => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len)?;
            let mut name = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut name)?;
            let name = String::from_utf8_lossy(&name).into_owned();
            (format!("domain {name}"), name)
        }
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port)?;
    let port = u16::from_be_bytes(port);
    // The test sshd listens on 127.0.0.1 only, and CI runners may resolve `localhost` (here or,
    // with local DNS, in the client) to `::1` alone: any loopback target is reached on 127.0.0.1.
    let host = if host == "localhost" {
        "127.0.0.1".to_owned()
    } else {
        host
    };
    let addrs: Vec<_> = (host.as_str(), port).to_socket_addrs()?.collect();
    let addr = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or_else(|| io::Error::other("no address"))?;
    let addr = if addr.ip().is_loopback() {
        std::net::SocketAddr::from(([127, 0, 0, 1], port))
    } else {
        addr
    };
    Ok((format!("{record}:{port}"), addr))
}

fn http(client: &mut TcpStream) -> io::Result<(String, std::net::SocketAddr)> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        client.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let line = text.lines().next().unwrap_or_default().to_owned();
    let authority = line.split(' ').nth(1).unwrap_or_default().to_owned();
    let addr = authority
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::other("no address"))?;
    Ok((line, addr))
}

fn pipe(client: TcpStream, upstream: TcpStream) -> io::Result<()> {
    let (mut c_read, mut u_write) = (client.try_clone()?, upstream.try_clone()?);
    let up = thread::spawn(move || {
        let _copied = io::copy(&mut c_read, &mut u_write);
        let _closed = u_write.shutdown(Shutdown::Write);
    });
    let (mut u_read, mut c_write) = (upstream, client);
    let _copied = io::copy(&mut u_read, &mut c_write);
    let _closed = c_write.shutdown(Shutdown::Write);
    let _joined = up.join();
    Ok(())
}
