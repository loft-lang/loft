// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I85 — Engine-host kernel natives (the natives behind the web library)

//! The natives behind the `web` library (`[native] in_binary = true`): an HTTP client and a
//! WebSocket client in loft's own binary, so the library carries no crate of its own (@PLN203,
//! owner 2026-10-27).  HTTP goes through the agent every HTTPS request loft makes uses
//! ([`crate::tls`]: the bundled roots plus the machine's store); a WebSocket session is RFC 6455
//! over TCP, TLS for `wss://`, reconnecting with backoff.  The C runtime's twin is
//! `src/c/loft_web.h`; the browser's is `native::WEB_FUNCTIONS_WASM`.
//!
//! A response body and a binary frame are BYTES that travel in a loft `text` (the library's
//! binary protocol reads them back with `byte_at`), turned into one by
//! [`crate::web_bytes::bytes_text`], which also holds the pack builder every target shares.
//!
//! Each native has two calling conventions: the interpreter's (`n_*` over the value stack,
//! [`FUNCTIONS`]) and the compiled backend's typed twin ([`typed`], re-exported by
//! `codegen_runtime`).  Both share one body.

#![cfg(not(target_arch = "wasm32"))]

use crate::database::{Call, Stores};
use crate::keys::{DbRef, Str};
use std::cell::{Cell, RefCell};

thread_local! {
    static LAST_BODY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static LAST_HEADERS: RefCell<String> = const { RefCell::new(String::new()) };
    static WS_GROUP: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static WS_GROUP_OFFSET: Cell<usize> = const { Cell::new(0) };
}

// ── HTTP ─────────────────────────────────────────────────────────────────────────────────

/// `Key: Value` lines, one per line; a line without a colon is skipped.
fn parse_headers(text: &str) -> Vec<(&str, &str)> {
    text.split('\n')
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            l.split_once(':').map(|(k, v)| (k.trim(), v.trim()))
        })
        .collect()
}

/// One request: `(status, body, headers)`.  A status that is an HTTP error (a 404 page, a 206
/// range) still carries its body and headers; only a transport failure answers status 0.
#[cfg(any(feature = "registry", feature = "remote-store"))]
fn request(
    method: &str,
    url: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> (i64, Vec<u8>, Vec<(String, String)>) {
    use std::io::Read;
    let agent = crate::tls::agent_builder().build();
    let mut req = match method {
        "GET" | "POST" | "PUT" | "DELETE" | "HEAD" => agent.request(method, url),
        _ => return (0, Vec::new(), Vec::new()),
    };
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let answer = match body {
        Some(b) => req.send_string(b),
        None => req.call(),
    };
    let resp = match answer {
        Ok(r) | Err(ureq::Error::Status(_, r)) => r,
        Err(_) => return (0, Vec::new(), Vec::new()),
    };
    let status = i64::from(resp.status());
    let names = resp.headers_names();
    let heads = names
        .iter()
        .filter_map(|n| resp.header(n).map(|v| (n.clone(), v.to_string())))
        .collect();
    let mut bytes = Vec::new();
    let _ = resp.into_reader().read_to_end(&mut bytes);
    (status, bytes, heads)
}

#[cfg(not(any(feature = "registry", feature = "remote-store")))]
fn request(
    _method: &str,
    _url: &str,
    _body: Option<&str>,
    _headers: &[(&str, &str)],
) -> (i64, Vec<u8>, Vec<(String, String)>) {
    (0, Vec::new(), Vec::new())
}

/// `http_do(method, url, body, headers) -> status`: the body and headers wait for
/// `http_body` / `http_headers`.  An empty body sends none.
fn http_do(method: &str, url: &str, body: &str, headers: &str) -> i64 {
    let body = (!body.is_empty()).then_some(body);
    let (status, bytes, heads) = request(method, url, body, &parse_headers(headers));
    LAST_BODY.with(|b| *b.borrow_mut() = bytes);
    let joined = heads
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    LAST_HEADERS.with(|h| *h.borrow_mut() = joined);
    status
}

/// A URL's `Content-Length` by a `HEAD`, or -1.
fn http_size(url: &str) -> i64 {
    let (status, _, heads) = request("HEAD", url, None, &[]);
    if !(200..400).contains(&status) {
        return -1;
    }
    heads
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(-1)
}

fn last_body() -> String {
    LAST_BODY.with(|b| crate::web_bytes::bytes_text(b.borrow().clone()))
}

fn last_headers() -> String {
    LAST_HEADERS.with(|h| h.borrow().clone())
}

fn sleep_ms(ms: i64) {
    if let Ok(ms) = u64::try_from(ms)
        && ms > 0
    {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

fn group_poll() -> i64 {
    let handles = WS_GROUP.with(|g| g.borrow().clone());
    let offset = WS_GROUP_OFFSET.with(Cell::get);
    let fired = ws::poll_group(&handles, offset);
    if fired >= 0
        && let Some(pos) = handles.iter().position(|&h| h == fired)
    {
        WS_GROUP_OFFSET.with(|o| o.set((pos + 1) % handles.len()));
    }
    i64::from(fired)
}

// ── The WebSocket client ─────────────────────────────────────────────────────────────────

/// RFC 6455, the client's half: frames written masked and read unmasked; a session survives a
/// dropped connection and reconnects on the next send or receive, after a backoff.
mod ws {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::{Duration, Instant};

    enum Stream {
        Plain(TcpStream),
        #[cfg(any(feature = "registry", feature = "remote-store"))]
        Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
    }

    impl Stream {
        fn sock(&self) -> &TcpStream {
            match self {
                Stream::Plain(s) => s,
                #[cfg(any(feature = "registry", feature = "remote-store"))]
                Stream::Tls(t) => &t.sock,
            }
        }
    }

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self {
                Stream::Plain(s) => s.read(buf),
                #[cfg(any(feature = "registry", feature = "remote-store"))]
                Stream::Tls(t) => t.read(buf),
            }
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            match self {
                Stream::Plain(s) => s.write(buf),
                #[cfg(any(feature = "registry", feature = "remote-store"))]
                Stream::Tls(t) => t.write(buf),
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            match self {
                Stream::Plain(s) => s.flush(),
                #[cfg(any(feature = "registry", feature = "remote-store"))]
                Stream::Tls(t) => t.flush(),
            }
        }
    }

    struct Conn {
        url: String,
        stream: Option<Stream>,
        inbox: VecDeque<(u8, Vec<u8>)>,
        last_attempt: Instant,
        backoff: usize,
    }

    /// The wait before each reconnect attempt, ms; the step resets on a success.
    const BACKOFF_MS: &[u64] = &[100, 250, 500, 1_000, 2_500, 5_000, 10_000];
    /// A peer that accepts the connection but does not answer the upgrade fails the attempt
    /// after this, rather than blocking (two servers that federate with each other deadlock
    /// otherwise, each waiting on the other's upgrade).
    const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);
    /// A receive waits this long for a frame before answering "none yet".
    const POLL_TIMEOUT: Duration = Duration::from_millis(7);

    pub const OP_TEXT: u8 = 0x01;
    pub const OP_BINARY: u8 = 0x02;
    const OP_CLOSE: u8 = 0x08;
    const OP_PING: u8 = 0x09;
    const OP_PONG: u8 = 0x0A;

    thread_local! {
        static CONNS: RefCell<Vec<Option<Conn>>> = const { RefCell::new(Vec::new()) };
        static LAST: RefCell<(u8, Vec<u8>)> = const { RefCell::new((0, Vec::new())) };
    }

    /// A nonce or a frame mask: entropy-mixed, not cryptographic, as RFC 6455 asks of neither.
    fn pseudo_random(n: usize) -> Vec<u8> {
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0xDEAD_BEEF, |d| d.as_nanos() as u64);
        (0..n)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (seed >> 33) as u8
            })
            .collect()
    }

    /// `(tls, host, port, path)` of a `ws://` or `wss://` URL; the port defaults by scheme.
    fn parse_url(url: &str) -> Option<(bool, String, u16, String)> {
        let (tls, rest, port) = if let Some(r) = url.strip_prefix("wss://") {
            (true, r, 443)
        } else {
            (false, url.strip_prefix("ws://")?, 80)
        };
        let (authority, path) = rest.find('/').map_or((rest, "/"), |i| rest.split_at(i));
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()?),
            None => (authority.to_string(), port),
        };
        Some((tls, host, port, path.to_string()))
    }

    /// The upgrade.  The head is read a byte at a time so a frame the server sends right after
    /// its 101 stays in the socket for the first receive.
    fn handshake<S: Read + Write>(s: &mut S, host: &str, port: u16, path: &str) -> bool {
        let nonce = crate::base64::encode(&pseudo_random(16));
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: {nonce}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        if s.write_all(req.as_bytes()).is_err() {
            return false;
        }
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut b = [0u8; 1];
            if !matches!(s.read(&mut b), Ok(1)) || head.len() > 16 * 1024 {
                return false;
            }
            head.push(b[0]);
        }
        let head = String::from_utf8_lossy(&head);
        let mut lines = head.split("\r\n");
        let status_ok = lines
            .next()
            .is_some_and(|l| l.starts_with("HTTP/1.1 101") || l.starts_with("HTTP/1.0 101"));
        let accept = lines.find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("sec-websocket-accept")
                .then(|| v.trim().to_string())
        });
        let want = crate::ws_handshake::ws_accept_key(&nonce);
        status_ok && accept.as_deref() == Some(want.as_str())
    }

    fn write_frame<S: Write>(s: &mut S, opcode: u8, payload: &[u8]) -> bool {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode);
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if let Ok(short) = u16::try_from(len) {
            frame.push(0x80 | 0x7E);
            frame.extend_from_slice(&short.to_be_bytes());
        } else {
            frame.push(0x80 | 0x7F);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mask = pseudo_random(4);
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        s.write_all(&frame).is_ok()
    }

    fn read_frame<S: Read>(s: &mut S) -> Option<(u8, Vec<u8>)> {
        let mut h = [0u8; 2];
        s.read_exact(&mut h).ok()?;
        let mut len = u64::from(h[1] & 0x7F);
        if len == 126 {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).ok()?;
            len = u64::from(u16::from_be_bytes(b));
        } else if len == 127 {
            let mut b = [0u8; 8];
            s.read_exact(&mut b).ok()?;
            len = u64::from_be_bytes(b);
        }
        let mask = if h[1] & 0x80 != 0 {
            let mut m = [0u8; 4];
            s.read_exact(&mut m).ok()?;
            Some(m)
        } else {
            None
        };
        let mut payload = vec![0u8; usize::try_from(len).ok()?];
        s.read_exact(&mut payload).ok()?;
        if let Some(m) = mask {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= m[i % 4];
            }
        }
        Some((h[0] & 0x0F, payload))
    }

    /// TLS over the connected socket for `host`, validated against [`crate::tls`]'s trust; a
    /// failure refuses this address — a `wss://` URL never falls back to plaintext.
    #[cfg(any(feature = "registry", feature = "remote-store"))]
    fn tls_wrap(tcp: TcpStream, host: &str) -> Option<Stream> {
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).ok()?;
        let mut conn = rustls::ClientConnection::new(crate::tls::client_config(), name).ok()?;
        let mut sock = tcp;
        conn.complete_io(&mut sock).ok()?;
        Some(Stream::Tls(Box::new(rustls::StreamOwned::new(conn, sock))))
    }

    #[cfg(not(any(feature = "registry", feature = "remote-store")))]
    fn tls_wrap(_tcp: TcpStream, _host: &str) -> Option<Stream> {
        None
    }

    /// Connect and upgrade, trying every address the host resolves to (a `localhost` that
    /// resolves to `::1` first must still reach a server on `0.0.0.0`).
    fn open(url: &str) -> Option<Stream> {
        let (tls, host, port, path) = parse_url(url)?;
        for addr in (host.as_str(), port).to_socket_addrs().ok()? {
            let Ok(tcp) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
                continue;
            };
            let _ = tcp.set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT));
            let mut stream = if tls {
                match tls_wrap(tcp, &host) {
                    Some(s) => s,
                    None => continue,
                }
            } else {
                Stream::Plain(tcp)
            };
            if handshake(&mut stream, &host, port, &path) {
                let _ = stream.sock().set_read_timeout(Some(POLL_TIMEOUT));
                return Some(stream);
            }
        }
        None
    }

    fn mark_down(c: &mut Conn) {
        c.stream = None;
        c.last_attempt = Instant::now();
        c.backoff = (c.backoff + 1).min(BACKOFF_MS.len() - 1);
    }

    /// Live, or reconnected once the backoff since the last attempt has passed.
    fn ensure(c: &mut Conn) -> bool {
        if c.stream.is_some() {
            return true;
        }
        if c.last_attempt.elapsed() < Duration::from_millis(BACKOFF_MS[c.backoff]) {
            return false;
        }
        c.last_attempt = Instant::now();
        if let Some(s) = open(&c.url) {
            c.stream = Some(s);
            c.backoff = 0;
            true
        } else {
            c.backoff = (c.backoff + 1).min(BACKOFF_MS.len() - 1);
            false
        }
    }

    /// A session's handle, or -1 for a URL that is not `ws://` / `wss://`.  A first attempt
    /// that fails still makes the session: the next send or receive retries.
    pub fn connect(url: &str) -> i32 {
        if parse_url(url).is_none() {
            return -1;
        }
        let conn = Conn {
            url: url.to_string(),
            stream: open(url),
            inbox: VecDeque::new(),
            last_attempt: Instant::now(),
            backoff: 0,
        };
        CONNS.with(|c| {
            let mut c = c.borrow_mut();
            c.push(Some(conn));
            i32::try_from(c.len() - 1).unwrap_or(-1)
        })
    }

    fn with_conn<R>(handle: i32, f: impl FnOnce(&mut Conn) -> R) -> Option<R> {
        CONNS.with(|c| {
            let mut c = c.borrow_mut();
            let conn = c.get_mut(usize::try_from(handle).ok()?)?.as_mut()?;
            Some(f(conn))
        })
    }

    pub fn send(handle: i32, opcode: u8, payload: &[u8]) -> bool {
        with_conn(handle, |c| {
            if !ensure(c) {
                return false;
            }
            let ok = c
                .stream
                .as_mut()
                .is_some_and(|s| write_frame(s, opcode, payload));
            if !ok {
                mark_down(c);
            }
            ok
        })
        .unwrap_or(false)
    }

    /// One frame read off a live session: a text or binary frame is the answer, a ping is
    /// answered, a close drops the connection.
    fn take_frame(c: &mut Conn, nonblocking: bool) -> Option<(u8, Vec<u8>)> {
        let s = c.stream.as_mut()?;
        if nonblocking {
            let _ = s.sock().set_nonblocking(true);
        }
        let frame = read_frame(s);
        if nonblocking {
            let _ = s.sock().set_nonblocking(false);
            let _ = s.sock().set_read_timeout(Some(POLL_TIMEOUT));
        }
        match frame {
            Some((op, payload)) if op == OP_TEXT || op == OP_BINARY => Some((op, payload)),
            Some((OP_PING, payload)) => {
                let _ = write_frame(s, OP_PONG, &payload);
                None
            }
            Some((OP_CLOSE, _)) => {
                mark_down(c);
                None
            }
            _ => None,
        }
    }

    fn surface(frame: (u8, Vec<u8>)) {
        LAST.with(|l| *l.borrow_mut() = frame);
    }

    /// Is a message ready (it is then `last_message`)?  Waits a few ms for one.
    pub fn recv(handle: i32) -> bool {
        with_conn(handle, |c| {
            if let Some(f) = c.inbox.pop_front() {
                surface(f);
                return true;
            }
            if !ensure(c) {
                return false;
            }
            take_frame(c, false).map(surface).is_some()
        })
        .unwrap_or(false)
    }

    /// The first of `handles`, from `offset` round, with a message ready, or -1; each socket is
    /// probed without waiting.
    pub fn poll_group(handles: &[i32], offset: usize) -> i32 {
        let n = handles.len();
        for i in 0..n {
            let h = handles[(offset + i) % n];
            let fired = with_conn(h, |c| {
                if let Some(f) = c.inbox.pop_front() {
                    surface(f);
                    return true;
                }
                ensure(c) && take_frame(c, true).map(surface).is_some()
            });
            if fired == Some(true) {
                return h;
            }
        }
        -1
    }

    /// The last message surfaced, as the text the library reads: a text frame's UTF-8 (lossy),
    /// a binary frame's bytes.
    pub fn last_message() -> String {
        LAST.with(|l| {
            let (op, bytes) = &*l.borrow();
            if *op == OP_TEXT {
                String::from_utf8_lossy(bytes).into_owned()
            } else {
                crate::web_bytes::bytes_text(bytes.clone())
            }
        })
    }

    pub fn last_opcode() -> i64 {
        LAST.with(|l| i64::from(l.borrow().0))
    }

    /// Closed for good: a close frame is sent and the handle never reconnects.
    pub fn close(handle: i32) {
        CONNS.with(|c| {
            let mut c = c.borrow_mut();
            if let Some(slot) = usize::try_from(handle).ok().and_then(|h| c.get_mut(h)) {
                if let Some(s) = slot.as_mut().and_then(|conn| conn.stream.as_mut()) {
                    let _ = write_frame(s, OP_CLOSE, &[]);
                }
                *slot = None;
            }
        });
    }
}

fn handle(h: i64) -> i32 {
    i32::try_from(h).unwrap_or(-1)
}

// ── The interpreter's calling convention ─────────────────────────────────────────────────

/// A text answer, into the caller's buffer (`bridge_text_dest`: every library `#native` text
/// producer is called with one).  Without one it answers empty, as the cdylib bridge does
/// (`extensions::bridge_text_result`).
fn put_text(stores: &mut Stores, stack: &mut DbRef, s: &str) {
    if let Some(dest) = stores.bridge_text_dest.take() {
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(s);
        return;
    }
    stores.put(stack, Str::new(""));
}

fn n_http_do(stores: &mut Stores, stack: &mut DbRef) {
    let headers = stores.get::<Str>(stack).str().to_owned();
    let body = stores.get::<Str>(stack).str().to_owned();
    let url = stores.get::<Str>(stack).str().to_owned();
    let method = stores.get::<Str>(stack).str().to_owned();
    let status = http_do(&method, &url, &body, &headers);
    stores.put(stack, status);
}

fn n_http_body(stores: &mut Stores, stack: &mut DbRef) {
    put_text(stores, stack, &last_body());
}

fn n_http_headers(stores: &mut Stores, stack: &mut DbRef) {
    put_text(stores, stack, &last_headers());
}

fn n_http_head_size(stores: &mut Stores, stack: &mut DbRef) {
    let url = stores.get::<Str>(stack).str().to_owned();
    stores.put(stack, http_size(&url));
}

fn n_ws_connect(stores: &mut Stores, stack: &mut DbRef) {
    let url = stores.get::<Str>(stack).str().to_owned();
    stores.put(stack, i64::from(ws::connect(&url)));
}

fn n_ws_client_send(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let h = handle(stores.get::<i64>(stack));
    stores.put(stack, ws::send(h, ws::OP_TEXT, msg.as_bytes()));
}

fn n_ws_client_send_binary(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let h = handle(stores.get::<i64>(stack));
    stores.put(stack, ws::send(h, ws::OP_BINARY, msg.as_bytes()));
}

fn n_ws_client_recv(stores: &mut Stores, stack: &mut DbRef) {
    let h = handle(stores.get::<i64>(stack));
    stores.put(stack, ws::recv(h));
}

fn n_ws_client_message(stores: &mut Stores, stack: &mut DbRef) {
    put_text(stores, stack, &ws::last_message());
}

fn n_ws_client_opcode(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, ws::last_opcode());
}

fn n_ws_client_close(stores: &mut Stores, stack: &mut DbRef) {
    ws::close(handle(stores.get::<i64>(stack)));
}

fn n_sleep_ms(stores: &mut Stores, stack: &mut DbRef) {
    sleep_ms(stores.get::<i64>(stack));
}

fn n_pack_reset(_stores: &mut Stores, _stack: &mut DbRef) {
    crate::web_bytes::pack_reset();
}

fn n_pack_u8(stores: &mut Stores, stack: &mut DbRef) {
    crate::web_bytes::pack_u8(stores.get::<i64>(stack));
}

fn n_pack_u16_le(stores: &mut Stores, stack: &mut DbRef) {
    crate::web_bytes::pack_u16_le(stores.get::<i64>(stack));
}

fn n_pack_u32_le(stores: &mut Stores, stack: &mut DbRef) {
    crate::web_bytes::pack_u32_le(stores.get::<i64>(stack));
}

fn n_pack_take(stores: &mut Stores, stack: &mut DbRef) {
    put_text(stores, stack, &crate::web_bytes::pack_take());
}

fn n_byte_at(stores: &mut Stores, stack: &mut DbRef) {
    let t = stores.get::<Str>(stack).str().to_owned();
    let idx = stores.get::<i64>(stack);
    stores.put(stack, crate::web_bytes::byte_at(idx, &t));
}

/// `frame_yield()`: the browser's frame suspend; a no-op off it, as the stdlib's
/// `yield_frame` is.
fn n_frame_yield(_stores: &mut Stores, _stack: &mut DbRef) {}

fn n_ws_group_clear(_stores: &mut Stores, _stack: &mut DbRef) {
    WS_GROUP.with(|g| g.borrow_mut().clear());
    WS_GROUP_OFFSET.with(|o| o.set(0));
}

fn n_ws_group_add(stores: &mut Stores, stack: &mut DbRef) {
    let h = handle(stores.get::<i64>(stack));
    WS_GROUP.with(|g| g.borrow_mut().push(h));
}

fn n_ws_group_poll(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, group_poll());
}

/// The web library's natives for the interpreter (registered by `compile::register_natives`),
/// each named as the library declares it.
pub const FUNCTIONS: &[(&str, Call)] = &[
    ("n_http_do", n_http_do),
    ("n_http_body", n_http_body),
    ("n_http_headers", n_http_headers),
    ("n_http_head_size", n_http_head_size),
    ("n_frame_yield", n_frame_yield),
    ("n_ws_connect", n_ws_connect),
    ("n_ws_client_send", n_ws_client_send),
    ("n_ws_client_send_binary", n_ws_client_send_binary),
    ("n_ws_client_recv", n_ws_client_recv),
    ("n_ws_client_message", n_ws_client_message),
    ("n_ws_client_opcode", n_ws_client_opcode),
    ("n_ws_client_close", n_ws_client_close),
    ("n_sleep_ms", n_sleep_ms),
    ("n_pack_reset", n_pack_reset),
    ("n_pack_u8", n_pack_u8),
    ("n_pack_u16_le", n_pack_u16_le),
    ("n_pack_u32_le", n_pack_u32_le),
    ("n_pack_take", n_pack_take),
    ("n_byte_at", n_byte_at),
    ("n_ws_group_clear", n_ws_group_clear),
    ("n_ws_group_add", n_ws_group_add),
    ("n_ws_group_poll", n_ws_group_poll),
];

/// The compiled backend's twins: `--native` resolves a runtime function by its def name
/// through `CODEGEN_RUNTIME_FNS`.
pub mod typed {
    use crate::database::Stores;
    use std::cell::UnsafeCell;

    pub fn n_http_do(
        _cell: &UnsafeCell<Stores>,
        method: &str,
        url: &str,
        body: &str,
        headers: &str,
    ) -> i64 {
        super::http_do(method, url, body, headers)
    }
    pub fn n_http_body(_cell: &UnsafeCell<Stores>) -> String {
        super::last_body()
    }
    pub fn n_http_headers(_cell: &UnsafeCell<Stores>) -> String {
        super::last_headers()
    }
    pub fn n_http_head_size(_cell: &UnsafeCell<Stores>, url: &str) -> i64 {
        super::http_size(url)
    }
    pub fn n_ws_connect(_cell: &UnsafeCell<Stores>, url: &str) -> i64 {
        i64::from(super::ws::connect(url))
    }
    pub fn n_ws_client_send(_cell: &UnsafeCell<Stores>, h: i64, msg: &str) -> u8 {
        u8::from(super::ws::send(
            super::handle(h),
            super::ws::OP_TEXT,
            msg.as_bytes(),
        ))
    }
    pub fn n_ws_client_send_binary(_cell: &UnsafeCell<Stores>, h: i64, msg: &str) -> u8 {
        u8::from(super::ws::send(
            super::handle(h),
            super::ws::OP_BINARY,
            msg.as_bytes(),
        ))
    }
    pub fn n_ws_client_recv(_cell: &UnsafeCell<Stores>, h: i64) -> u8 {
        u8::from(super::ws::recv(super::handle(h)))
    }
    pub fn n_ws_client_message(_cell: &UnsafeCell<Stores>) -> String {
        super::ws::last_message()
    }
    pub fn n_ws_client_opcode(_cell: &UnsafeCell<Stores>) -> i64 {
        super::ws::last_opcode()
    }
    pub fn n_ws_client_close(_cell: &UnsafeCell<Stores>, h: i64) {
        super::ws::close(super::handle(h));
    }
    pub fn n_sleep_ms(_cell: &UnsafeCell<Stores>, ms: i64) {
        super::sleep_ms(ms);
    }
    pub fn n_pack_reset(_cell: &UnsafeCell<Stores>) {
        crate::web_bytes::pack_reset();
    }
    pub fn n_pack_u8(_cell: &UnsafeCell<Stores>, b: i64) {
        crate::web_bytes::pack_u8(b);
    }
    pub fn n_pack_u16_le(_cell: &UnsafeCell<Stores>, v: i64) {
        crate::web_bytes::pack_u16_le(v);
    }
    pub fn n_pack_u32_le(_cell: &UnsafeCell<Stores>, v: i64) {
        crate::web_bytes::pack_u32_le(v);
    }
    pub fn n_pack_take(_cell: &UnsafeCell<Stores>) -> String {
        crate::web_bytes::pack_take()
    }
    pub fn n_byte_at(_cell: &UnsafeCell<Stores>, idx: i64, t: &str) -> i64 {
        crate::web_bytes::byte_at(idx, t)
    }
    pub fn n_ws_group_clear(_cell: &UnsafeCell<Stores>) {
        super::WS_GROUP.with(|g| g.borrow_mut().clear());
        super::WS_GROUP_OFFSET.with(|o| o.set(0));
    }
    pub fn n_ws_group_add(_cell: &UnsafeCell<Stores>, h: i64) {
        let h = super::handle(h);
        super::WS_GROUP.with(|g| g.borrow_mut().push(h));
    }
    pub fn n_frame_yield(_cell: &UnsafeCell<Stores>) {}
    pub fn n_ws_group_poll(_cell: &UnsafeCell<Stores>) -> i64 {
        super::group_poll()
    }
}
