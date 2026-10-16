//! Proxy handshakes run on a TCP stream before SSH starts: SOCKS5 and HTTP `CONNECT`; plus the
//! SOCKS5 server side of dynamic forwards.

pub(crate) mod http;
pub(crate) mod socks5;
pub(crate) mod socks5_server;
