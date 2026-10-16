//! Proxy handshakes run on a TCP stream before SSH starts: SOCKS5 and HTTP `CONNECT`.

pub(crate) mod http;
pub(crate) mod socks5;
