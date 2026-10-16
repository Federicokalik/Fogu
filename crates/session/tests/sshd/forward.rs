//! Port forwards through real OpenSSH (`AllowTcpForwarding yes`): local, dynamic (SOCKS5) and
//! remote forwards to an in-test TCP echo server, a port already in use, the non-loopback warning,
//! and forwards stopping on `Remove` / `Close`.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::Duration;

use fogu_core::{ForwardKind, PortForward};
use fogu_session::{
    ForwardCmd, ForwardId, ForwardState, ForwardStatus, SessionInput, SessionOutput, SessionWarning,
};

use super::no_prompts;
use crate::support::client::Client;
use crate::support::sshd::TestSshd;

type TestClient = Client<fn(&fogu_session::PromptKind) -> fogu_session::PromptReply>;

/// A TCP echo server on 127.0.0.1 (one thread per connection); returns its port.
fn echo_server() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n @ 1..) = stream.read(&mut buf) {
                    if stream.write_all(&buf[..n]).is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

fn free_port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.local_addr().unwrap().port()
}

fn forward(kind: ForwardKind, host: &str, port: u16, target_port: u16) -> PortForward {
    PortForward {
        kind,
        host: host.to_owned(),
        port,
        target_address: "127.0.0.1".to_owned(),
        target_port,
        description: String::new(),
    }
}

/// Connects. Forwards from the profile report as soon as the shell is up, so callers wait for
/// those statuses (`wait_shell` would skip them); runtime `Add`s need `wait_shell` first.
fn session(sshd: &TestSshd, forwards: Vec<PortForward>) -> TestClient {
    sshd.trust();
    let mut ssh = sshd.profile();
    ssh.forwarded_ports = forwards;
    Client::connect(&ssh, sshd.paths(), no_prompts)
}

fn send(client: &TestClient, input: SessionInput) {
    client
        .rt
        .handle()
        .block_on(client.session.input.send(input))
        .unwrap();
}

/// Waits for a status of the forward listening on `port` that `want` accepts.
fn status(
    client: &mut TestClient,
    port: u16,
    want: impl Fn(&ForwardStatus) -> bool,
) -> ForwardStatus {
    let event = client.until(
        |e| matches!(e, SessionOutput::ForwardStatus(s) if s.forward.port == port && want(s)),
    );
    match event {
        Some(SessionOutput::ForwardStatus(status)) => status,
        other => panic!("no matching forward status for port {port}: {other:?}"),
    }
}

fn listening(client: &mut TestClient, port: u16, active: usize) -> ForwardId {
    status(client, port, |s| {
        s.state == ForwardState::Listening && s.active_connections == active
    })
    .id
}

/// Writes `text` on `stream` and expects it back.
fn round_trip(stream: &mut TcpStream, text: &[u8]) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(text).unwrap();
    let mut back = vec![0u8; text.len()];
    stream.read_exact(&mut back).unwrap();
    assert_eq!(back, text);
}

#[test]
fn local_forward_round_trips_counts_connections_and_stops_on_close() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let (echo, port) = (echo_server(), free_port());
    let mut client = session(&sshd, vec![forward(ForwardKind::Local, "", port, echo)]);
    listening(&mut client, port, 0);
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    round_trip(&mut stream, b"hello through -L");
    listening(&mut client, port, 1);
    drop(stream);
    listening(&mut client, port, 0);
    client.close();
    status(&mut client, port, |s| s.state == ForwardState::Stopped);
    client.end();
    assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err());
}

#[test]
fn dynamic_forward_is_a_socks5_server() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let (echo, port) = (echo_server(), free_port());
    let mut client = session(&sshd, Vec::new());
    client.wait_shell();
    let socks = forward(ForwardKind::Dynamic, "127.0.0.1", port, 0);
    send(&client, SessionInput::Forward(ForwardCmd::Add(socks)));
    listening(&mut client, port, 0);
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // Greeting (no auth), then CONNECT localhost:echo by name (the server resolves it).
    stream.write_all(&[5, 1, 0]).unwrap();
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).unwrap();
    assert_eq!(choice, [5, 0]);
    let mut request = vec![5, 1, 0, 3, 9];
    request.extend(b"localhost");
    request.extend(echo.to_be_bytes());
    stream.write_all(&request).unwrap();
    let mut reply = [0u8; 10];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(reply[..2], [5, 0], "SOCKS reply {reply:?}");
    round_trip(&mut stream, b"hello through -D");
}

#[test]
fn remote_forward_reaches_the_local_target_and_remove_cancels_it() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let (echo, port) = (echo_server(), free_port());
    let mut client = session(&sshd, Vec::new());
    client.wait_shell();
    let remote = forward(ForwardKind::Remote, "127.0.0.1", port, echo);
    send(&client, SessionInput::Forward(ForwardCmd::Add(remote)));
    let id = listening(&mut client, port, 0);
    // sshd listens on 127.0.0.1:port; its connections come back to our echo server.
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    round_trip(&mut stream, b"hello through -R");
    listening(&mut client, port, 1);
    send(&client, SessionInput::Forward(ForwardCmd::Remove(id)));
    status(&mut client, port, |s| s.state == ForwardState::Stopped);
    // The server stops listening once it has processed the cancel.
    let mut refused = false;
    for _ in 0..50 {
        if TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err() {
            refused = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(refused, "sshd still listens on {port} after Remove");
}

#[test]
fn port_in_use_is_an_error_and_the_session_carries_on() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    let echo = echo_server();
    let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let busy = taken.local_addr().unwrap().port();
    let exposed = free_port();
    let mut client = session(
        &sshd,
        vec![
            forward(ForwardKind::Local, "127.0.0.1", busy, echo),
            forward(ForwardKind::Local, "0.0.0.0", exposed, echo),
        ],
    );
    // The two forwards start concurrently: collect the error and the warning in any order.
    let (mut failed, mut label) = (None, None);
    client.until(|e| {
        match e {
            SessionOutput::ForwardStatus(s)
                if s.forward.port == busy && matches!(s.state, ForwardState::Error(_)) =>
            {
                failed = Some(s.clone());
            }
            SessionOutput::Warning(SessionWarning::ForwardExposed(text)) => {
                label = Some(text.clone());
            }
            _ => {}
        }
        failed.is_some() && label.is_some()
    });
    let failed = failed.expect("no error status for the busy port");
    let ForwardState::Error(why) = &failed.state else {
        unreachable!()
    };
    assert!(why.contains("in use"), "{why}");
    assert_eq!(
        label.as_deref(),
        Some(format!("L 0.0.0.0:{exposed} → 127.0.0.1:{echo}").as_str())
    );
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, exposed)).unwrap();
    round_trip(&mut stream, b"still forwarding");
    client.send("echo alive-$((20+1))\r");
    client.expect_text("alive-21");
    // A failed forward stays listed until removed.
    send(
        &client,
        SessionInput::Forward(ForwardCmd::Remove(failed.id)),
    );
    status(&mut client, busy, |s| s.state == ForwardState::Stopped);
}
