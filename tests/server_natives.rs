// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The server library's natives, loft's `net` component (`[native] component = "net"`;
//! `src/c/loft_server.h` in `src/c/components/net.c`, built by `loft-components`), answer the
//! same on every backend: the interpreter calls its bridges, the Rust writer and the C backend its
//! typed symbols.  The program
//! is the SERVER, reaching the natives through `tests/fixtures/server_natives` (their
//! declarations word for word); this test is the client, and every answer it must see is fixed
//! in advance:
//!
//! - a bind that works, the same port bound twice (-1), an idle listener (nothing to accept);
//! - a POST with a header and a body, answered with a status and type of the program's choice;
//!   a binary body carrying a NUL and a 0xff byte; caller headers with their own length; a
//!   plain 404; `tcp_slice` clamped by byte;
//! - one WebSocket after a blocking accept, its first frame sent right behind the upgrade
//!   request, a ping answered and read past, a binary frame, the server's close;
//! - the event pump on one port: two clients, a plain request answered between them, a
//!   broadcast, a ping, the client set, a disconnect;
//! - TLS: a request and a WebSocket over an EC certificate (the pump's look-ahead through a
//!   session), a request over an RSA one, both validated by the client against the test CA for
//!   `127.0.0.1`; a listener over a certificate it cannot use refuses (-1).

use loft::file_access as fa;
#[path = "common/ws_frame.rs"]
mod ws_frame;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use ws_frame::{OP_BINARY, OP_CLOSE, OP_PING, OP_PONG, OP_TEXT};

const PROGRAM: &str = r#"use server_natives;

fn port(name: text) -> integer {
  (env_variable(name) ?? "0") as integer ?? 0
}

fn main() {
  h = server_natives::tcp_listen_on("127.0.0.1", port("SN_PORT"));
  q = server_natives::tcp_listen_on("127.0.0.1", port("SN_QUIET"));
  t = server_natives::tcp_listen_tls_on("127.0.0.1", port("SN_TPORT"), env_variable("SN_EC_CERT") ?? "", env_variable("SN_EC_KEY") ?? "");
  r = server_natives::tcp_listen_tls_on("127.0.0.1", port("SN_RPORT"), env_variable("SN_RSA_CERT") ?? "", env_variable("SN_RSA_KEY") ?? "");
  println("LISTEN {h >= 0} {q >= 0} {t >= 0} {r >= 0}");
  println("DUP {server_natives::tcp_listen_on("127.0.0.1", port("SN_PORT"))}");
  println("BADTLS {server_natives::tcp_listen_tls_on("127.0.0.1", 0, "junk", "junk")}");
  println("QUIET {server_natives::ws_accept_native(q)} {server_natives::tcp_accept_nonblocking(q)} {server_natives::ws_next_event_native(q)}");
  server_natives::tcp_close(q);
  if server_natives::tcp_accept(h) {
    b = server_natives::tcp_body();
    println("REQ {server_natives::tcp_method()} {server_natives::tcp_path()} {b} {server_natives::tcp_headers().to_lowercase().contains("x-mine: v1")}");
    server_natives::tcp_respond_typed(201, "made {b}", "text/x-test");
  }
  if server_natives::tcp_accept(h) {
    bytes: vector<u8> = [0, 255, 65, 10];
    server_natives::tcp_respond_bytes(200, bytes, "");
  }
  if server_natives::tcp_accept(h) {
    server_natives::tcp_respond_full(200, "", "Content-Length: 1234\nX-Extra: yes\n");
  }
  if server_natives::tcp_accept(h) {
    server_natives::tcp_respond(404, "nope");
  }
  println("SLICE {server_natives::tcp_slice("abcdef", 2, 3)} {server_natives::tcp_slice("abc", 5, 2).len()} {server_natives::tcp_slice("abc", -1, 2)}");
  if server_natives::tcp_accept(h) {
    ws = server_natives::ws_upgrade_native();
    got = "";
    for i in 0..20 {
      if !server_natives::ws_recv_native(ws) { break; }
      m = server_natives::ws_message_native();
      if server_natives::ws_opcode_native() == 2 {
        got += " b:{m.len()}";
        server_natives::ws_send_binary_native(ws, "bin:{m}");
        break;
      }
      got += " t:{m}";
      server_natives::ws_send_native(ws, "echo:{m}");
    }
    println("WS {ws}{got}");
    server_natives::ws_close_native(ws);
  }
  events = "";
  for i in 0..20000 {
    if !server_natives::ws_next_event_native(h) {
      server_natives::ws_idle_sleep_ms_native(2);
      continue;
    }
    kind = server_natives::ws_event_kind_native();
    cid = server_natives::ws_event_client_id_native();
    p = server_natives::ws_event_payload_native();
    if kind == 0 {
      events += " c{cid}";
      server_natives::ws_send_native(cid, "welcome {cid}");
    } else if kind == 3 {
      events += " h{p}";
      server_natives::tcp_respond_typed(200, "page {p}", "text/html");
    } else if kind == 1 {
      events += " m{cid}:{p}";
      if p == "a1" {
        n = server_natives::ws_broadcast_native(h, "all {cid}");
        server_natives::ws_send_native(1, "count {n}");
      }
      if p == "a2" {
        server_natives::ws_send_native(cid, "clients {server_natives::ws_clients_len_native()} {server_natives::ws_client_active_native(1)} {server_natives::ws_client_active_native(7)}");
      }
    } else if kind == 2 {
      events += " d{cid}";
      server_natives::ws_send_native(1, "gone {cid}");
      break;
    }
  }
  println("PUMP{events}");
  server_natives::ws_close_native(1);
  if server_natives::tcp_accept(t) {
    server_natives::tcp_respond_typed(200, "secure {server_natives::tcp_path()}", "");
  }
  events = "";
  for i in 0..20000 {
    if !server_natives::ws_next_event_native(t) {
      server_natives::ws_idle_sleep_ms_native(2);
      continue;
    }
    kind = server_natives::ws_event_kind_native();
    cid = server_natives::ws_event_client_id_native();
    p = server_natives::ws_event_payload_native();
    if kind == 0 {
      events += " c{cid}";
      server_natives::ws_send_native(cid, "welcome {cid}");
    } else if kind == 1 {
      events += " m{cid}:{p}";
      server_natives::ws_send_native(cid, "tls-echo {p}");
    } else if kind == 2 {
      events += " d{cid}";
      break;
    }
  }
  println("TLSPUMP{events}");
  if server_natives::tcp_accept(r) {
    server_natives::tcp_respond(200, "rsa {server_natives::tcp_path()}");
  }
  println("DONE");
}
"#;

/// The lines the program prints, fixed by the client's script below.
const WANT: [&str; 10] = [
    "LISTEN true true true true",
    "DUP -1",
    "BADTLS -1",
    "QUIET -1 false false",
    "REQ POST /echo?x=1 hello true",
    "SLICE cde 0 ab",
    "WS 0 t:hi b:3",
    "PUMP c0 c1 h/page m0:a1 m0:a2 d0",
    "TLSPUMP c0 m0:t1 d0",
    "DONE",
];

fn tls_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/server_natives/tls")
}

fn pem(name: &str) -> String {
    fa::read_to_string(tls_dir().join(name)).expect("a fixture PEM")
}

/// A port nothing listens on yet.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("a free port")
        .port()
}

/// Connect, retrying while the program compiles and binds.
fn connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            s.set_read_timeout(Some(Duration::from_secs(30)))
                .expect("timeout");
            return s;
        }
        assert!(
            Instant::now() < deadline,
            "the program never listened on {port}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The whole response a `Connection: close` server sends: its head (status line and headers)
/// and its body bytes.
fn read_response<S: Read>(s: &mut S) -> (String, Vec<u8>) {
    let mut all = Vec::new();
    let _ = s.read_to_end(&mut all);
    let split = all
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a response head");
    (
        String::from_utf8_lossy(&all[..split]).into_owned(),
        all[split + 4..].to_vec(),
    )
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

const MASK: Option<[u8; 4]> = Some([0x11, 0x22, 0x33, 0x44]);

fn send<S: Write>(s: &mut S, op: u8, payload: &[u8]) {
    assert!(ws_frame::write(s, op, payload, MASK), "a client frame");
}

/// The next frame, which the server must not mask.
fn frame<S: Read>(s: &mut S) -> (u8, Vec<u8>) {
    match ws_frame::read(s, 1 << 20) {
        ws_frame::Read1::Frame(op, p) => (op, p),
        _ => panic!("a server frame"),
    }
}

fn text_frame<S: Read>(s: &mut S) -> String {
    let (op, p) = frame(s);
    assert_eq!(
        op,
        OP_TEXT,
        "a text frame, got {:?}",
        String::from_utf8_lossy(&p)
    );
    String::from_utf8(p).expect("UTF-8")
}

/// The upgrade request; `then` goes out in the same write, before the 101 is read.
fn upgrade<S: Read + Write>(s: &mut S, then: &[u8]) {
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let mut req = format!(
        "GET /ws HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    .into_bytes();
    req.extend_from_slice(then);
    s.write_all(&req).expect("the upgrade request");
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        assert_eq!(s.read(&mut b).expect("the 101"), 1, "the 101 ended early");
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 101"), "a 101: {head}");
    assert_eq!(
        header(&head, "sec-websocket-accept"),
        Some(loft::ws_handshake::ws_accept_key(key).as_str())
    );
}

fn tls_client(port: u16) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    use rustls::pki_types::pem::PemObject;
    let mut roots = rustls::RootCertStore::empty();
    for c in rustls::pki_types::CertificateDer::pem_slice_iter(pem("ca.pem").as_bytes()) {
        roots.add(c.expect("the CA")).expect("a usable CA");
    }
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    let name =
        rustls::pki_types::ServerName::IpAddress(std::net::IpAddr::from([127, 0, 0, 1]).into());
    let mut conn = rustls::ClientConnection::new(Arc::new(cfg), name).expect("a session");
    let mut sock = connect(port);
    while conn.is_handshaking() {
        conn.complete_io(&mut sock)
            .expect("the TLS handshake, verified");
    }
    rustls::StreamOwned::new(conn, sock)
}

struct Ports {
    plain: u16,
    quiet: u16,
    ec: u16,
    rsa: u16,
}

/// The client's side of the script, in the program's order; it panics on the first answer
/// that differs.
fn client(p: &Ports) {
    // A request with a header and a body.
    let mut s = connect(p.plain);
    s.write_all(b"POST /echo?x=1 HTTP/1.1\r\nX-Mine: v1\r\nContent-Length: 5\r\n\r\nhello")
        .expect("write");
    let (head, body) = read_response(&mut s);
    assert!(head.starts_with("HTTP/1.1 201 Created\r\n"), "{head}");
    assert_eq!(header(&head, "content-type"), Some("text/x-test"));
    assert_eq!(header(&head, "connection"), Some("close"));
    assert_eq!(body, b"made hello");
    // A binary body.
    let mut s = connect(p.plain);
    s.write_all(b"GET /bin HTTP/1.1\r\n\r\n").expect("write");
    let (head, body) = read_response(&mut s);
    assert_eq!(
        header(&head, "content-type"),
        Some("application/octet-stream")
    );
    assert_eq!(header(&head, "content-length"), Some("4"));
    assert_eq!(body, [0, 255, 65, 10]);
    // The caller's headers, its own length kept.
    let mut s = connect(p.plain);
    s.write_all(b"HEAD /data HTTP/1.1\r\n\r\n").expect("write");
    let (head, body) = read_response(&mut s);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert_eq!(header(&head, "content-length"), Some("1234"));
    assert_eq!(header(&head, "x-extra"), Some("yes"));
    assert_eq!(header(&head, "connection"), Some("close"));
    assert!(body.is_empty());
    // A plain respond.
    let mut s = connect(p.plain);
    s.write_all(b"GET /x HTTP/1.1\r\n\r\n").expect("write");
    let (head, body) = read_response(&mut s);
    assert!(head.starts_with("HTTP/1.1 404 Not Found\r\n"), "{head}");
    assert_eq!(
        header(&head, "content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(body, b"nope");
    // One WebSocket: the first frame rides with the upgrade request.
    let mut s = connect(p.plain);
    let mut first = Vec::new();
    ws_frame::write(&mut first, OP_TEXT, b"hi", MASK);
    upgrade(&mut s, &first);
    assert_eq!(text_frame(&mut s), "echo:hi");
    send(&mut s, OP_PING, b"p1");
    send(&mut s, OP_BINARY, &[1, 2, 3]);
    assert_eq!(frame(&mut s), (OP_PONG, b"p1".to_vec()));
    assert_eq!(frame(&mut s), (OP_BINARY, b"bin:\x01\x02\x03".to_vec()));
    assert_eq!(frame(&mut s).0, OP_CLOSE);
    // The event pump: two clients and a plain request on one port.
    let mut a = connect(p.plain);
    upgrade(&mut a, &[]);
    assert_eq!(text_frame(&mut a), "welcome 0");
    let mut b = connect(p.plain);
    upgrade(&mut b, &[]);
    assert_eq!(text_frame(&mut b), "welcome 1");
    let mut page = connect(p.plain);
    page.write_all(b"GET /page HTTP/1.1\r\n\r\n")
        .expect("write");
    let (head, body) = read_response(&mut page);
    assert_eq!(header(&head, "content-type"), Some("text/html"));
    assert_eq!(body, b"page /page");
    send(&mut a, OP_TEXT, b"a1");
    assert_eq!(text_frame(&mut a), "all 0");
    assert_eq!(text_frame(&mut b), "all 0");
    assert_eq!(text_frame(&mut b), "count 2");
    send(&mut a, OP_PING, b"p2");
    send(&mut a, OP_TEXT, b"a2");
    assert_eq!(frame(&mut a), (OP_PONG, b"p2".to_vec()));
    assert_eq!(text_frame(&mut a), "clients 2 true false");
    send(&mut a, OP_CLOSE, &[]);
    drop(a);
    assert_eq!(text_frame(&mut b), "gone 0");
    assert_eq!(frame(&mut b).0, OP_CLOSE);
    // TLS over the EC certificate: a request, then a WebSocket through the pump.
    let mut s = tls_client(p.ec);
    s.write_all(b"GET /secure HTTP/1.1\r\n\r\n").expect("write");
    let (head, body) = read_response(&mut s);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert_eq!(body, b"secure /secure");
    let mut s = tls_client(p.ec);
    upgrade(&mut s, &[]);
    assert_eq!(text_frame(&mut s), "welcome 0");
    send(&mut s, OP_PING, b"p3");
    send(&mut s, OP_TEXT, b"t1");
    assert_eq!(frame(&mut s), (OP_PONG, b"p3".to_vec()));
    assert_eq!(text_frame(&mut s), "tls-echo t1");
    send(&mut s, OP_CLOSE, &[]);
    s.conn.send_close_notify();
    let _ = s.flush();
    drop(s);
    // TLS over the RSA certificate.
    let mut s = tls_client(p.rsa);
    s.write_all(b"GET /rsa HTTP/1.1\r\n\r\n").expect("write");
    let (_, body) = read_response(&mut s);
    assert_eq!(body, b"rsa /rsa");
    let _ = p.quiet;
}

/// Build the component where a `loft` beside `loft-components` finds it (`<binary>/../components`),
/// as `make components` does.
fn build_component() {
    let loft = Path::new(env!("CARGO_BIN_EXE_loft"));
    let out = loft
        .parent()
        .and_then(Path::parent)
        .expect("the binary's directory")
        .join("components");
    let built = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft-components"))
        .arg("--out")
        .arg(&out)
        .arg("net")
        .output()
        .expect("loft-components runs");
    assert!(
        built.status.success(),
        "loft-components net:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
}

fn run(backend: &str) {
    // The component needs the C runtime's sockets (POSIX or Winsock, @PLN203 C8).
    if !loft::platform::c_runtime_has_sockets() {
        return;
    }
    build_component();
    let dir = std::env::temp_dir().join(format!(
        "loft_server_natives_{}_{}",
        &backend[2..],
        std::process::id()
    ));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("a scratch directory");
    let file = dir.join("server.loft");
    fa::write(&file, PROGRAM).expect("write the program");
    let ports = Ports {
        plain: free_port(),
        quiet: free_port(),
        ec: free_port(),
        rsa: free_port(),
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let child = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(&fixtures)
        .arg(&file)
        .current_dir(&dir)
        .env_remove("LOFT_COMPONENTS")
        .env("LOFT_TIMEOUT", "240")
        .env("SN_PORT", ports.plain.to_string())
        .env("SN_QUIET", ports.quiet.to_string())
        .env("SN_TPORT", ports.ec.to_string())
        .env("SN_RPORT", ports.rsa.to_string())
        .env("SN_EC_CERT", pem("ec-cert.pem"))
        .env("SN_EC_KEY", pem("ec-key.pem"))
        .env("SN_RSA_CERT", pem("rsa-cert.pem"))
        .env("SN_RSA_KEY", pem("rsa-key.pem"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("loft runs");
    let script = std::thread::spawn(move || client(&ports));
    let out = child.wait_with_output().expect("the program ends");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if let Err(e) = script.join() {
        let why = e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        panic!(
            "{backend}: the client's script failed: {why}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, WANT, "{backend}\nstderr:\n{stderr}");
    let _ = fa::remove_dir_all(&dir);
}

#[test]
fn the_server_natives_answer_the_same_on_the_interpreter() {
    run("--interpret");
}

#[test]
fn the_server_natives_answer_the_same_on_the_rust_writer() {
    run("--native-rust");
}

#[test]
fn the_server_natives_answer_the_same_on_the_c() {
    run("--native-c");
}
