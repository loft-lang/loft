// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The web library's natives, which live in loft (`[native] in_binary = true`;
//! `src/web_client.rs`, `src/c/loft_web.h`), answer the same on every backend.  The program
//! reaches them through `tests/fixtures/web_natives`, their declarations word for word, against a
//! local server whose answers fix every expected line in advance: a body carrying a NUL and a
//! 0xff byte, a header, a request body and header echoed, a 404 with its body, a refused port, a
//! HEAD size; a WebSocket echo of a text and a binary frame, a ping the client must answer, a
//! round-robin group poll and a close.  The SERVER witnesses what the program cannot print: the
//! pong and the close frame.

use loft::file_access as fa;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
  web_natives::pack_reset();
  web_natives::pack_u8(0);
  web_natives::pack_u8(255);
  web_natives::pack_u16_le(258);
  web_natives::pack_u32_le(4294967295);
  p = web_natives::pack_take();
  println("PACK {p.len()} {web_natives::byte_at(0, p)} {web_natives::byte_at(1, p)} {web_natives::byte_at(2, p)} {web_natives::byte_at(3, p)} {web_natives::byte_at(7, p)}");
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
const WANT: [&str; 12] = [
    "BIN 200 5 65 0 255 66 10",
    "HDR true",
    "POST 200 POST hello v1",
    "MISSING 404 not here",
    "REFUSED 0",
    "SIZE 1234",
    "PACK 8 0 255 2 1 255",
    "WS echo:hi 1",
    "WSBIN 2 8 255 2",
    "GROUP true echo:g",
    "BAD -1",
    "DONE",
];

#[derive(Default)]
struct Seen {
    pongs: usize,
    closes: usize,
}

fn read_head(s: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if s.read(&mut b).ok()? != 1 {
            return None;
        }
        head.push(b[0]);
    }
    Some(String::from_utf8_lossy(&head).to_string())
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

fn http(s: &mut TcpStream, head: &str) {
    let line = head.lines().next().unwrap_or_default().to_string();
    let mut parts = line.split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let len: usize = header(head, "content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    let _ = s.read_exact(&mut body);
    let (status, extra, answer): (&str, String, Vec<u8>) = match path.as_str() {
        "/bin" => ("200 OK", "X-Test: yes\r\n".into(), vec![65, 0, 255, 66, 10]),
        "/echo" => (
            "200 OK",
            String::new(),
            format!(
                "{method} {} {}",
                String::from_utf8_lossy(&body),
                header(head, "x-mine").unwrap_or("")
            )
            .into_bytes(),
        ),
        "/sized" => ("200 OK", "Content-Length: 1234\r\n".into(), Vec::new()),
        _ => ("404 Not Found", String::new(), b"not here".to_vec()),
    };
    let length = if path == "/sized" {
        String::new()
    } else {
        format!("Content-Length: {}\r\n", answer.len())
    };
    let _ = s.write_all(
        format!("HTTP/1.1 {status}\r\n{extra}{length}Connection: close\r\n\r\n").as_bytes(),
    );
    if method != "HEAD" {
        let _ = s.write_all(&answer);
    }
}

fn frame(s: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    s.read_exact(&mut h).ok()?;
    let mut len = u64::from(h[1] & 0x7f);
    if len == 126 {
        let mut b = [0u8; 2];
        s.read_exact(&mut b).ok()?;
        len = u64::from(u16::from_be_bytes(b));
    } else if len == 127 {
        let mut b = [0u8; 8];
        s.read_exact(&mut b).ok()?;
        len = u64::from_be_bytes(b);
    }
    let mut mask = [0u8; 4];
    if h[1] & 0x80 != 0 {
        s.read_exact(&mut mask).ok()?;
    }
    let mut p = vec![0u8; len as usize];
    s.read_exact(&mut p).ok()?;
    for (i, b) in p.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    Some((h[0] & 0x0f, p))
}

fn send_frame(s: &mut TcpStream, op: u8, p: &[u8]) {
    let mut f = vec![0x80 | op, p.len() as u8];
    f.extend_from_slice(p);
    let _ = s.write_all(&f);
}

fn websocket(s: &mut TcpStream, head: &str, seen: &Mutex<Seen>) {
    let key = header(head, "sec-websocket-key").unwrap_or_default();
    let accept = loft::ws_handshake::ws_accept_key(key);
    let _ = s.write_all(
        format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\r\n"
        )
        .as_bytes(),
    );
    while let Some((op, p)) = frame(s) {
        match op {
            1 if p == b"hi" => {
                // A ping first: the client answers it before the echo can arrive.
                send_frame(s, 9, b"p");
                loop {
                    match frame(s) {
                        Some((10, _)) => {
                            seen.lock().expect("seen").pongs += 1;
                            break;
                        }
                        Some(_) => {}
                        None => return,
                    }
                }
                send_frame(s, 1, b"echo:hi");
            }
            1 => send_frame(s, 1, &[b"echo:".as_slice(), &p].concat()),
            2 => send_frame(s, 2, &p),
            8 => {
                seen.lock().expect("seen").closes += 1;
                return;
            }
            _ => {}
        }
    }
}

fn start_server() -> (u16, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                let Some(head) = read_head(&mut s) else {
                    return;
                };
                if header(&head, "upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket")) {
                    websocket(&mut s, &head, &log);
                } else {
                    http(&mut s, &head);
                }
            });
        }
    });
    (port, seen)
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("loft_web_natives_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("a scratch directory");
    d
}

fn run(backend: &str, dir: &Path, port: u16) -> (String, String) {
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
        .env("LOFT_TIMEOUT", "120")
        .env("WN_BASE", format!("http://127.0.0.1:{port}"))
        .env("WN_WS", format!("ws://127.0.0.1:{port}"))
        .output()
        .expect("loft runs");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The backends the natives are carried on: the interpreter, the Rust writer, and the C
/// runtime where it has its POSIX host layer (Windows is @PLN203 C8's).
fn backends() -> Vec<&'static str> {
    let mut b = vec!["--interpret", "--native-rust"];
    if loft::platform::c_runtime_posix_host() {
        b.push("--native-c");
    }
    b
}

#[test]
fn the_web_natives_answer_the_same_on_every_backend() {
    for backend in backends() {
        let dir = scratch(&backend[2..]);
        let (port, seen) = start_server();
        let (out, err) = run(backend, &dir, port);
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
