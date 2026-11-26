// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The web library's natives, loft's `net` component (`[native] component = "net"`;
//! `src/c/components/net.c` over `src/c/loft_web.h`, built by `loft-components`), answer the same
//! on every backend: the interpreter calls its bridges, the Rust writer and the C backend its
//! typed symbols.  The program
//! reaches them through `tests/fixtures/web_natives`, their declarations word for word, against a
//! local server whose answers fix every expected line in advance: a body carrying a NUL and a
//! 0xff byte, a header, a request body and header echoed, a 404 with its body, a refused port, a
//! HEAD size; a WebSocket echo of a text and a binary frame, a ping the client must answer, a
//! round-robin group poll and a close.  The SERVER witnesses what the program cannot print: the
//! pong and the close frame.

use loft::file_access as fa;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[path = "common/web_server.rs"]
mod web_server;

const PROGRAM: &str = r#"use web_natives;

fn main() {
  base = env_variable("WN_BASE") ?? "";
  ws = env_variable("WN_WS") ?? "";
  st = web_natives::http_do("GET", "{base}/bin", "", "");
  b = web_natives::http_body();
  println("BIN {st} {b.len()} {web_natives::byte_at(0, b)} {web_natives::byte_at(1, b)} {web_natives::byte_at(2, b)} {web_natives::byte_at(3, b)} {web_natives::byte_at(4, b)}");
  h = web_natives::http_headers().to_lowercase();
  println("HDR {h.contains("x-test: yes")}");
  st = web_natives::http_do("POST", "{base}/echo", "hello", "X-Mine: v1\n");
  println("POST {st} {web_natives::http_body()}");
  st = web_natives::http_do("GET", "{base}/missing", "", "");
  println("MISSING {st} {web_natives::http_body()}");
  st = web_natives::http_do("GET", "http://127.0.0.1:1/x", "", "");
  println("REFUSED {st}");
  println("SIZE {web_natives::http_head_size("{base}/sized")}");
  other = env_variable("WN_OTHER") ?? "";
  st = web_natives::http_do("GET", "{base}/hop", "", "X-Mine: v2\n");
  println("HOP {st} [{web_natives::http_body()}]");
  st = web_natives::http_do("GET", "{base}/away/{other}", "", "X-Mine: v3\n");
  println("AWAY {st} [{web_natives::http_body()}]");
  st = web_natives::http_do("POST", "{base}/away307/{other}", "secret", "X-Mine: v4\n");
  println("AWAY307 {st}");
  tls = env_variable("WN_TLS") ?? "";
  st = web_natives::http_do("GET", "{tls}/stay", "", "X-Mine: v5\n");
  println("STAY {st} {web_natives::http_body()}");
  st = web_natives::http_do("GET", "{tls}/down/{other}", "", "X-Mine: v6\n");
  println("DOWN {st}");
  st = web_natives::http_do("GET", "{base}/echo", "", "X-Mine: a\rInjected: 1\n");
  println("CTRL {st}");
  st = web_natives::http_do("GET", "{base}/a b", "", "");
  println("SPACE {st}");
  st = web_natives::http_do("GET", "{base}/fits", "", "");
  println("FITS {st} {web_natives::http_body().size()}");
  st = web_natives::http_do("GET", "{base}/over", "", "");
  println("OVER {st} {web_natives::http_body().size()}");
  web_natives::pack_reset();
  web_natives::pack_u8(0);
  web_natives::pack_u8(255);
  web_natives::pack_u16_le(258);
  web_natives::pack_u32_le(4294967295);
  p = web_natives::pack_take();
  println("PACK {p.len()} {web_natives::byte_at(0, p)} {web_natives::byte_at(1, p)} {web_natives::byte_at(2, p)} {web_natives::byte_at(3, p)} {web_natives::byte_at(7, p)}");
  web_natives::pack_reset();
  web_natives::pack_u8(263);
  web_natives::pack_u16_le(-1);
  q = web_natives::pack_take();
  e = web_natives::pack_take();
  println("PACK2 {q.len()} {web_natives::byte_at(0, q)} {web_natives::byte_at(2, q)} {web_natives::byte_at(-1, q)} {web_natives::byte_at(3, q)} {e.len()}");
  c = web_natives::ws_connect("{ws}/ws");
  web_natives::ws_client_send(c, "hi");
  got = "";
  for i in 0..400 {
    if web_natives::ws_client_recv(c) {
      got = "{web_natives::ws_client_message()} {web_natives::ws_client_opcode()}";
      break;
    }
    web_natives::sleep_ms(5);
  }
  println("WS {got}");
  web_natives::ws_client_send_binary(c, p);
  got = "";
  for i in 0..400 {
    if web_natives::ws_client_recv(c) {
      m = web_natives::ws_client_message();
      got = "{web_natives::ws_client_opcode()} {m.len()} {web_natives::byte_at(1, m)} {web_natives::byte_at(2, m)}";
      break;
    }
    web_natives::sleep_ms(5);
  }
  println("WSBIN {got}");
  d = web_natives::ws_connect("{ws}/ws");
  web_natives::ws_client_send(d, "g");
  web_natives::ws_group_clear();
  web_natives::ws_group_add(c);
  web_natives::ws_group_add(d);
  fired = -1;
  for i in 0..400 {
    fired = web_natives::ws_group_poll();
    if fired >= 0 { break; }
    web_natives::sleep_ms(5);
  }
  println("GROUP {fired == d} {web_natives::ws_client_message()}");
  println("BAD {web_natives::ws_connect("http://not-a-ws")}");
  web_natives::frame_yield();
  web_natives::ws_client_close(c);
  web_natives::ws_client_close(d);
  web_natives::sleep_ms(50);
  println("DONE");
}
"#;

/// The lines the program prints, fixed by the server's answers.
const WANT: [&str; 22] = [
    "BIN 200 5 65 0 255 66 10",
    "HDR true",
    "POST 200 POST hello v1",
    "MISSING 404 not here",
    "REFUSED 0",
    "SIZE 1234",
    // A redirect on the same server carries the caller's header; one to another server
    // (the second test server) arrives without it; one that would carry the body there is
    // refused.  With the header forwarded the second line reads `[GET  v3]`.
    "HOP 200 [GET  v2]",
    "AWAY 200 [GET  ]",
    "AWAY307 0",
    // Over https a redirect on the same server is followed; one to plain http is refused, as
    // the request was made for a TLS channel (followed, it reads `DOWN 200`).
    // @falsified-at: PENDING
    "STAY 200 secure",
    "DOWN 0",
    // A header holding a bare CR, and a URL holding a space, are refused before anything is
    // sent: written as they are, each would end a request line the program did not end.
    "CTRL 0",
    "SPACE 0",
    // The run's ceiling on a response body is 2000 bytes (`LOFT_MAX_DOWNLOAD`): a body of
    // exactly that arrives whole, and one byte more is refused — no status, no body.
    "FITS 200 2000",
    "OVER 0 0",
    "PACK 8 0 255 2 1 255",
    "PACK2 3 7 255 -1 -1 0",
    "WS echo:hi 1",
    "WSBIN 2 8 255 2",
    "GROUP true echo:g",
    "BAD -1",
    "DONE",
];

const TLS_FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tls/");

/// An HTTPS server on a free port, with the certificate of `tests/fixtures/tls/` (it names
/// `127.0.0.1`): `/stay` redirects to `/home` on itself, `/home` answers `secure`, and
/// `/down/<port>` redirects to `/echo` over plain http on that port.
fn start_tls_server() -> u16 {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs = CertificateDer::pem_file_iter(format!("{TLS_FIXTURES}server.pem"))
        .expect("server.pem is readable")
        .map(|c| c.expect("server.pem parses"))
        .collect();
    let key =
        PrivateKeyDer::from_pem_file(format!("{TLS_FIXTURES}server.key")).expect("server.key");
    let cfg = Arc::new(
        rustls::ServerConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_safe_default_protocol_versions()
        .expect("ring supports the default versions")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("the fixture certificate and key match"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let port = listener.local_addr().expect("local address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(tcp) = stream else { continue };
            let cfg = Arc::clone(&cfg);
            std::thread::spawn(move || {
                let Ok(conn) = rustls::ServerConnection::new(cfg) else {
                    return;
                };
                let mut tls = rustls::StreamOwned::new(conn, tcp);
                let mut head = Vec::new();
                let mut buf = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&head)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let path = line.split(' ').nth(1).unwrap_or_default().to_string();
                let answer = if path == "/stay" {
                    "HTTP/1.1 302 Found\r\nLocation: /home\r\nContent-Length: 0\r\n\r\n".to_string()
                } else if path == "/home" {
                    "HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecure".to_string()
                } else if let Some(to) = path.strip_prefix("/down/") {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{to}/echo\r\nContent-Length: 0\r\n\r\n"
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string()
                };
                let _ = tls.write_all(answer.as_bytes());
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    port
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("loft_web_natives_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("a scratch directory");
    d
}

/// Build the component where a `loft` beside `loft-components` finds it (`<binary>/../components`),
/// as `make components` does; the guard then exercises that default lookup, not an override.
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

fn run(backend: &str, dir: &Path, port: u16, other: u16, tls: u16) -> (String, String) {
    let file = dir.join("web.loft");
    fa::write(&file, PROGRAM).expect("write the program");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(&fixtures)
        .arg(&file)
        .current_dir(dir)
        .env_remove("LOFT_COMPONENTS")
        .env("LOFT_TIMEOUT", "120")
        .env("WN_BASE", format!("http://127.0.0.1:{port}"))
        .env("WN_WS", format!("ws://127.0.0.1:{port}"))
        .env("WN_OTHER", other.to_string())
        .env("WN_TLS", format!("https://127.0.0.1:{tls}"))
        .env("SSL_CERT_FILE", format!("{TLS_FIXTURES}ca.pem"))
        .env_remove("SSL_CERT_DIR")
        .env("LOFT_MAX_DOWNLOAD", "2000")
        .output()
        .expect("loft runs");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The backends the component is called from.
fn backends() -> [&'static str; 2] {
    ["--interpret", "--native"]
}

#[test]
fn the_web_natives_answer_the_same_on_every_backend() {
    if !loft::platform::c_runtime_posix_host() {
        // The component needs the C runtime's POSIX host layer (@PLN203 C8 is Windows').
        return;
    }
    build_component();
    for backend in backends() {
        let dir = scratch(&backend[2..]);
        let (port, seen) = web_server::start();
        let (other, _) = web_server::start();
        let tls = start_tls_server();
        let (out, err) = run(backend, &dir, port, other, tls);
        for want in WANT {
            assert!(
                out.lines().any(|l| l == want),
                "{backend}: wants `{want}`\nstdout:\n{out}\nstderr:\n{err}"
            );
        }
        let seen = seen.lock().expect("seen");
        assert_eq!(
            seen.pongs, 1,
            "{backend}: the client answers the server's ping once"
        );
        assert_eq!(
            seen.closes, 2,
            "{backend}: each closed session sends its close frame"
        );
        let _ = fa::remove_dir_all(&dir);
    }
}
