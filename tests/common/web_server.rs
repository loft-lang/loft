// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The local HTTP and WebSocket server the web natives' guards drive (`tests/web_natives.rs`,
//! and `tests/html_wasm.rs`'s page cell): every answer is fixed, so every line a program prints
//! is known in advance.
//!
//! HTTP: `GET /bin` a body holding a NUL and a 0xff byte with `X-Test: yes`; `POST /echo` the
//! method, the request body and its `X-Mine` header; `HEAD /sized` a `Content-Length` of 1234;
//! anything else a 404 whose body is `not here`.  WebSocket (`/ws`): a text `hi` is answered by a
//! ping first, then `echo:hi` once the pong is back; any other text by `echo:<text>`; a binary
//! frame by itself; a close is counted.

#![allow(dead_code)] // each test binary uses the part it needs

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// What the server saw that a program cannot print.
#[derive(Default)]
pub struct Seen {
    pub pongs: usize,
    pub closes: usize,
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

/// A server on a free loopback port, answering HTTP and WebSocket on the same port.
pub fn start() -> (u16, Arc<Mutex<Seen>>) {
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
