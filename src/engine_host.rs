// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I85 — Engine-host kernel natives

//! @PLN18 phase 01 — the **engine-host kernel natives**: the Rust mechanics behind
//! the kernel's loft library (`lib/engine_host`).  The host-boundary principle
//! ([`plans/18-engine-host/ENGINE_HOST.md`]): this module owns **mechanics** —
//! the socket pump (non-blocking, the phase-00 **peek pattern** from day one),
//! the event queue, drift-free tick scheduling, send/broadcast — and *no game
//! meaning*.  The loft side owns meaning: `run(port, tick_us, on_event, on_tick)`
//! loops over these natives and invokes the user's closures via ordinary fn-ref
//! calls (probe 2: no Rust→closure machinery exists or is needed).
//!
//! Scope: the **events class** (WS text frames, queue-to-empty) from phase 01,
//! plus the phase-05a **state-sync class over UDP** — the quick channel beside
//! the websockets.  Events and bulk STAY on WS (reliable+ordered for free); the
//! UDP side carries ONLY latest-value state, so it needs no retransmit,
//! fragmentation, or congestion machinery:
//!
//! - One UDP socket on the SAME port number as the TCP listener (zero config).
//! - Identity = the datagram source address bound by a **cookie handshake**:
//!   the kernel issues a per-connection cookie (`n_kernel_udp_cookie`); the
//!   loft side transports it inside its own protocol (cookie *issuance* is
//!   mechanics, cookie *transport* is meaning); the client echoes it in a
//!   `H:<cookie>` datagram; the kernel binds the source addr to that cid and
//!   acks `A:<cid>`.
//! - Inbound `S:<seq>:<payload>` datagrams **conflate to newest per sender**
//!   (a higher seq overwrites the slot; stale/reordered seqs are discarded —
//!   never apply an old pose).  The loft side drains the dirty slots at tick
//!   time via `n_kernel_sync_next` — late-latch by construction.
//! - Outbound `n_kernel_sync_send` stamps a per-cid seq and sends a datagram
//!   when the client has a live UDP path, else falls back to a WS frame —
//!   phones stay `wss`, native peers ride UDP, same world.
//! - Any datagram refreshes the path's keepalive; a silent path unbinds after
//!   [`UDP_TIMEOUT_US`] and sends fall back to WS (the client keeps a steady
//!   keepalive cadence — which doubles as the phone radio wake).
//! - Datagrams are capped at [`UDP_MAX_DATAGRAM`] bytes (overlay-shaved MTUs
//!   fragment silently above ~1200): oversized sync sends are dropped with a
//!   once-per-kernel warning, never a halt.
//!
//! Wire: WebSocket text frames (`<msg_id>:<payload>` convention is the loft
//! side's concern — the kernel passes payloads through verbatim).

use std::cell::RefCell;
// VecDeque / keys::Str are used only in the native (non-wasm) engine-host
// paths, so they read as unused in wasm / feature-restricted builds.
#[allow(unused_imports)]
use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::hash::{BuildHasher, Hash, Hasher};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{ErrorKind, Read, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

use crate::database::Stores;
#[allow(unused_imports)]
use crate::keys::{DbRef, Str};

/// One pumped event: connect (0), message (1), disconnect (2).
#[allow(dead_code)] // @PLN18 phase-05a UDP state-sync scaffolding
struct Event {
    cid: i64,
    kind: i64,
    payload: String,
    /// HTTP completion status for kind-3 events (negative = transport
    /// error / timeout; -2 = http support not compiled in); 0 otherwise.
    status: i64,
}

// ── Non-blocking outbound HTTP (the events-class integration) ─────────────
//
// The one invariant: `http_fetch` returns BEFORE any network I/O happens,
// and the completion is delivered through the SAME queue / drain / budget
// as every other event — a slow or stalled request can never stall the
// loop.  The kernel is thread_local, so workers park completions in this
// global queue; each pump turn drains it into the host's event queue as
// ordinary kind-3 events (cid = the request id).

#[cfg(not(target_arch = "wasm32"))]
static HTTP_NEXT_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
#[cfg(not(target_arch = "wasm32"))]
static HTTP_DONE: std::sync::Mutex<Vec<HttpDone>> = std::sync::Mutex::new(Vec::new());

#[cfg(not(target_arch = "wasm32"))]
struct HttpDone {
    id: i64,
    status: i64,
    body: String,
}

#[cfg(not(target_arch = "wasm32"))]
/// Outbound requests time out here (no knob — the F-principle: the engine
/// takes the correct path unasked; a stalled socket must not leak workers
/// forever).  Generous enough for Overpass-style upstreams (`timeout:25`).
#[cfg(feature = "registry")]
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(not(target_arch = "wasm32"))]
/// Move every finished request into the host's event queue.  Called at the
/// top of BOTH pumps (server kernel + client/windowed), so whichever host
/// this process runs sees its completions as ordinary events.
fn drain_http_done(events: &mut VecDeque<Event>) {
    let mut done = HTTP_DONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for d in done.drain(..) {
        events.push_back(Event {
            cid: d.id,
            kind: 3,
            payload: d.body,
            status: d.status,
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `http_fetch(method, url, body, headers) -> integer` — begin a
/// non-blocking outbound HTTP request; returns the request id immediately.
/// The completion arrives as a kind-3 event: cid = this id, status = the
/// HTTP status (negative on transport error / timeout), payload = the
/// response body.  `headers` is newline-separated `Name: value` lines;
/// an empty `body` sends a bodyless request.
pub fn n_kernel_http_fetch(stores: &mut Stores, stack: &mut DbRef) {
    let headers = stores.get::<Str>(stack).str().to_owned();
    let body = stores.get::<Str>(stack).str().to_owned();
    let url = stores.get::<Str>(stack).str().to_owned();
    let method = stores.get::<Str>(stack).str().to_owned();
    let id = http_fetch_impl(method, url, body, headers);
    stores.put(stack, id);
}

#[cfg(not(target_arch = "wasm32"))]
/// The fetch body shared by both calling conventions (see `listen_impl`).
// Args are owned so the `registry` branch can `move` them into the worker
// thread; without `registry` they are only borrowed for a no-op, so silence
// the by-value lint in that build rather than break the move path.
#[cfg_attr(
    not(feature = "registry"),
    allow(clippy::needless_pass_by_value, unused_variables)
)]
fn http_fetch_impl(method: String, url: String, body: String, headers: String) -> i64 {
    let id = HTTP_NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "registry")]
    std::thread::spawn(move || {
        let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
        let mut req = agent.request(&method, &url);
        for line in headers.lines() {
            if let Some((name, value)) = line.split_once(':') {
                req = req.set(name.trim(), value.trim());
            }
        }
        let result = if body.is_empty() {
            req.call()
        } else {
            req.send_string(&body)
        };
        let (status, text) = match result {
            Ok(resp) => (
                i64::from(resp.status()),
                resp.into_string().unwrap_or_default(),
            ),
            // Non-2xx is a COMPLETION, not a transport error — the handler
            // decides what a 404 means.
            Err(ureq::Error::Status(code, resp)) => {
                (i64::from(code), resp.into_string().unwrap_or_default())
            }
            Err(e) => (-1, e.to_string()),
        };
        HTTP_DONE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(HttpDone {
                id,
                status,
                body: text,
            });
    });
    #[cfg(not(feature = "registry"))]
    {
        let _ = (&headers, &body, &url, &method);
        HTTP_DONE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(HttpDone {
                id,
                status: -2,
                body: "http support not compiled in (registry feature)".to_string(),
            });
    }
    id
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_event_status() -> integer` — the status of the last-popped event.
pub fn n_kernel_event_status(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last.status).unwrap_or(0);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// A silent UDP path unbinds after this long; the client's keepalive cadence
/// (~500 ms — also the phone radio wake) keeps a live path far inside it.
const UDP_TIMEOUT_US: i64 = 3_000_000;
#[cfg(not(target_arch = "wasm32"))]
/// Datagram payload cap — overlay/VPN-shaved MTUs fragment silently above
/// ~1200 bytes, and a fragmented "fast path" is slower than the WS one.
const UDP_MAX_DATAGRAM: usize = 1200;

#[cfg(not(target_arch = "wasm32"))]
/// A client's bound UDP path (the source addr proven by the cookie handshake).
struct UdpPath {
    addr: SocketAddr,
    last_seen_us: i64,
}

/// One inbound conflation slot: the newest `S:` datagram from this peer for
/// one `(msg_id, key)`.  Latest-value semantics — a higher seq overwrites, a
/// stale seq is dropped.
#[allow(dead_code)] // @PLN18 phase-05a UDP state-sync scaffolding
struct SyncSlot {
    /// The `msg_id` parsed from the datagram's payload (`-1` = unframed).
    msg_id: i64,
    /// For a `sync_class_keyed` kind: the payload's first field (the entity
    /// id — "2:7,…" → "7"), so N entities keep N latest-values.  "" for
    /// unkeyed kinds (one latest-value per kind).
    key: String,
    seq: i64,
    payload: String,
    dirty: bool,
}

#[cfg(not(target_arch = "wasm32"))]
/// Per-connection network state beyond the TCP stream itself; lives and dies
/// with the connection slot (a reused cid starts fresh).
struct ClientNet {
    /// The UDP handshake cookie for this WS session ("" = no UDP listener).
    cookie: String,
    path: Option<UdpPath>,
    /// One outbound seq space per peer for ALL sync-channel sends — datagrams
    /// AND keyframes — so the receiver's slots totally order them even when
    /// the carriers race (a keyframe on TCP vs in-flight datagrams).
    out_seq: i64,
    /// Conflation slots, one per `msg_id` this client has sent (newest per
    /// sender PER MESSAGE KIND — two sync kinds never collapse each other).
    slots: Vec<SyncSlot>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ClientNet {
    fn new(cookie: String) -> Self {
        ClientNet {
            cookie,
            path: None,
            out_seq: 0,
            slots: Vec::new(),
        }
    }
}

// The wire-schema-as-data table, minimal form (user-directed 2026-06-10):
// `msg_id -> keyed?` for the kinds whose messages are **latest-value state**
// (the sync class — conflate, ride UDP when the peer can).  `keyed = true`
// (`sync_class_keyed`) conflates per the payload's first field — the entity
// id — so one kind carries N entities' latest-values ("state-sync keyed by
// cid" from the class table); `false` keeps one latest-value per kind.
// Everything undeclared is the event class (must-deliver, rides WS).
// Lives outside `Kernel` so declarations may precede `run()`.
thread_local! {
    static SYNC_IDS: RefCell<std::collections::HashMap<i64, bool>> =
        RefCell::new(std::collections::HashMap::new());
}

/// The leading `<digits>:` of a wire message, or `-1` when unframed.
#[allow(dead_code)] // @PLN18 phase-05a UDP state-sync scaffolding
fn msg_id_of(msg: &str) -> i64 {
    msg.split_once(':')
        .and_then(|(id, _)| id.parse::<i64>().ok())
        .unwrap_or(-1)
}

#[allow(dead_code)] // @PLN18 phase-05a UDP state-sync scaffolding
fn is_sync_msg(msg: &str) -> bool {
    let id = msg_id_of(msg);
    id >= 0 && SYNC_IDS.with(|s| s.borrow().contains_key(&id))
}

/// The conflation key for a payload: the first comma-field after the kind
/// for keyed kinds ("2:7,x,y" → "7"), "" for unkeyed/unframed ones.
#[allow(dead_code)] // @PLN18 phase-05a UDP state-sync scaffolding
fn sync_key_of(payload: &str) -> String {
    let id = msg_id_of(payload);
    let keyed = id >= 0 && SYNC_IDS.with(|s| s.borrow().get(&id).copied().unwrap_or(false));
    if !keyed {
        return String::new();
    }
    let body = payload.split_once(':').map_or("", |(_, b)| b);
    body.split(',').next().unwrap_or("").to_string()
}

/// Conflate one inbound `S:` payload into a slot set: per `msg_id`, a higher
/// seq overwrites, a stale/reordered seq is discarded.  Shared by both roles
/// (the listener's per-client slots and the connector's server slots) — the
/// queue machinery must never fork between them.
#[allow(dead_code)] // @PLN18 phase-05a UDP state-sync scaffolding
fn find_or_create_slot<'a>(
    slots: &'a mut Vec<SyncSlot>,
    payload: &str,
) -> Option<&'a mut SyncSlot> {
    // Bounded slot count — a peer inventing endless ids/keys cannot grow
    // memory; sync is loss-tolerant by definition, so over-cap drops.
    const SYNC_SLOTS_MAX: usize = 256;
    let msg_id = msg_id_of(payload);
    let key = sync_key_of(payload);
    match slots
        .iter_mut()
        .position(|s| s.msg_id == msg_id && s.key == key)
    {
        Some(i) => Some(&mut slots[i]),
        None if slots.len() >= SYNC_SLOTS_MAX => None,
        None => {
            slots.push(SyncSlot {
                msg_id,
                key,
                seq: -1,
                payload: String::new(),
                dirty: false,
            });
            slots.last_mut()
        }
    }
}

/// Ordered-carrier conflation: a sync-class message arriving over WS (a
/// phone's pose; a native client's pre-bind fallback).  The carrier is
/// ordered and lossless, so the arrival order IS the seq — continue the
/// slot's space, so a later datagram (the sender counts its sync sends
/// across BOTH carriers) still reads as newer.
#[allow(dead_code)] // @PLN18 phase-05a UDP state-sync scaffolding
fn conflate_ws(slots: &mut Vec<SyncSlot>, payload: &str) {
    if let Some(slot) = find_or_create_slot(slots, payload) {
        slot.seq += 1;
        slot.payload.clear();
        slot.payload.push_str(payload);
        slot.dirty = true;
    }
}

#[allow(dead_code)] // @PLN18 phase-05a UDP state-sync scaffolding
fn conflate_slot(slots: &mut Vec<SyncSlot>, seq: i64, payload: &str) {
    let Some(slot) = find_or_create_slot(slots, payload) else {
        return;
    };
    if seq > slot.seq {
        slot.seq = seq;
        slot.payload.clear();
        slot.payload.push_str(payload);
        slot.dirty = true;
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct Kernel {
    listener: TcpListener,
    /// The state-sync datagram socket (same port number); `None` = bind
    /// failed at listen time (warned once; everything rides WS).
    udp: Option<UdpSocket>,
    /// Slot-indexed connections; `None` = free slot (cid = index, reused).
    conns: Vec<Option<TcpStream>>,
    /// Parallel to `conns`: cookie / UDP path / conflation slot per client.
    net: Vec<ClientNet>,
    events: VecDeque<Event>,
    /// The event handed out by the last `n_kernel_next_event`.
    last: Event,
    /// The slot handed out by the last `n_kernel_sync_next`.
    last_sync: (i64, i64, String),
    start: Instant,
    tick_interval_us: i64,
    last_tick_us: i64,
    warned_oversize: bool,
    /// False once `kernel_stop()` ran — a windowed listener's exit: `run`
    /// returns at the top of its next turn (mirror of the connector's
    /// `client_stop`).
    alive: bool,
    /// @PLN98 P3.4 — debug RELAY: a client that opted into debugging
    /// (`--debug=<name>`) registers its NAME here (conn slot → name), so an
    /// agent's `D!:@<name>:<cmd>` frame forwards to that client's socket.
    debug_names: std::collections::HashMap<usize, String>,
    /// @PLN98 P3.4 — while a forwarded debug command is outstanding: the client
    /// conn slot → the agent conn slot that sent it, so the client's `D!:reply`
    /// routes back to the requesting agent.
    debug_relay: std::collections::HashMap<usize, usize>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Kernel {
    fn now_us(&self) -> i64 {
        self.start.elapsed().as_micros() as i64
    }

    /// A fresh handshake cookie: 16 hex chars from the OS-seeded `RandomState`
    /// hasher mixed with the clock and cid.  Spoof-resistant on a LAN; not
    /// cryptographic (DTLS is the eventual answer for hostile networks).
    fn fresh_cookie(&self, cid: usize) -> String {
        if self.udp.is_none() {
            return String::new();
        }
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        self.start.elapsed().as_nanos().hash(&mut h);
        cid.hash(&mut h);
        format!("{:016x}", h.finish())
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static KERNEL: RefCell<Option<Kernel>> = const { RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
fn with_kernel<R>(f: impl FnOnce(&mut Kernel) -> R) -> Option<R> {
    KERNEL.with(|k| k.borrow_mut().as_mut().map(f))
}

// ── WS mechanics (unbuffered TcpStream reads — a BufReader would steal bytes
//    between pump turns; the frame reader mirrors the phase-00-patched pump:
//    peek the header non-blocking, then read the in-flight frame with a short
//    blocking timeout bound) ─────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
/// Upgrade a freshly-accepted stream: parse the HTTP request head, answer the
/// WebSocket handshake.  Returns the stream ready for frame traffic, or `None`
/// (not an upgrade / malformed — the connection is dropped).
///
/// A non-empty `udp_cookie` rides the 101 response as an `X-Loft-UDP` header —
/// the transport negotiation is **fully kernel-internal** (user directive,
/// 2026-06-10: the game developer is never bothered with transport).  Browsers
/// cannot read upgrade-response headers from JS and ignore it; a native
/// client's kernel reads it and auto-hellos, earning the UDP fast path with
/// zero meaning-level code on either side.
fn ws_upgrade(mut stream: TcpStream, udp_cookie: &str) -> Option<TcpStream> {
    // BSD/macOS accepted sockets INHERIT the listener's non-blocking flag
    // (Linux ones don't): force blocking before the bounded head read, or
    // the first byte EAGAINs instantly and every upgrade drops.
    stream.set_nonblocking(false).ok()?;
    // The upgrade request is in flight from a live client; bound the read.
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    // Read header bytes until CRLFCRLF (unbuffered — no over-read).
    while !buf.ends_with(b"\r\n\r\n") {
        match crate::net_profile::time(
            "ws_upgrade/header_read",
            Some(Duration::from_millis(500)),
            || stream.read(&mut byte),
        ) {
            Ok(1) => buf.push(byte[0]),
            _ => return None,
        }
        if buf.len() > 16 * 1024 {
            return None; // header flood
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let mut key = None;
    let mut is_upgrade = false;
    for line in head.lines() {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "upgrade" => is_upgrade = v.trim().eq_ignore_ascii_case("websocket"),
                "sec-websocket-key" => key = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    let key = key?;
    if !is_upgrade {
        return None;
    }
    let accept = crate::serve::ws_accept_key(&key);
    let udp_header = if udp_cookie.is_empty() {
        String::new()
    } else {
        format!("X-Loft-UDP: {udp_cookie}\r\n")
    };
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n{udp_header}\r\n"
    );
    stream.write_all(resp.as_bytes()).ok()?;
    // Frame phase: short timeout bounds a torn frame; the peek keeps idle free.
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .ok()?;
    Some(stream)
}

#[cfg(not(target_arch = "wasm32"))]
enum FrameRead {
    None,
    Text(String),
    Closed,
}

#[cfg(not(target_arch = "wasm32"))]
/// Read one client frame if pending — **peek first** (idle costs µs, the
/// phase-00 lesson), then unbuffered `read_exact` for the in-flight frame.
fn read_frame(stream: &mut TcpStream) -> FrameRead {
    let mut hdr = [0u8; 2];
    let _ = stream.set_nonblocking(true);
    let peeked = stream.peek(&mut hdr);
    let _ = stream.set_nonblocking(false);
    match peeked {
        Ok(0) => return FrameRead::Closed,
        Ok(n) if n < 2 => return FrameRead::None, // header still arriving
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
            return FrameRead::None;
        }
        Err(_) => return FrameRead::Closed,
    }
    if stream.read_exact(&mut hdr).is_err() {
        return FrameRead::Closed;
    }
    let opcode = hdr[0] & 0x0F;
    let masked = hdr[1] & 0x80 != 0;
    let mut len = u64::from(hdr[1] & 0x7F);
    if len == 126 {
        let mut b = [0u8; 2];
        if stream.read_exact(&mut b).is_err() {
            return FrameRead::Closed;
        }
        len = u64::from(u16::from_be_bytes(b));
    } else if len == 127 {
        let mut b = [0u8; 8];
        if stream.read_exact(&mut b).is_err() {
            return FrameRead::Closed;
        }
        len = u64::from_be_bytes(b);
    }
    if len > 16 * 1024 * 1024 {
        return FrameRead::Closed; // bound a hostile length
    }
    let mut mask = [0u8; 4];
    if masked && stream.read_exact(&mut mask).is_err() {
        return FrameRead::Closed;
    }
    let mut payload = vec![0u8; usize::try_from(len).unwrap_or(0)];
    if stream.read_exact(&mut payload).is_err() {
        return FrameRead::Closed;
    }
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    match opcode {
        0x1 => FrameRead::Text(String::from_utf8_lossy(&payload).into_owned()),
        0x8 => FrameRead::Closed,
        0x9 => {
            // PING → PONG, then report nothing pending this turn.
            let _ = write_frame(stream, 0xA, &payload);
            FrameRead::None
        }
        _ => FrameRead::None, // binary/pong/continuation: ignored in v1
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Write one unmasked server frame.
fn write_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x80 | opcode);
    if payload.len() <= 125 {
        frame.push(payload.len() as u8);
    } else if payload.len() <= 0xFFFF {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame)
}

#[cfg(not(target_arch = "wasm32"))]
/// Bind with `SO_REUSEADDR` so a restarted server rebinds through TIME_WAIT —
/// the arcade flow (restart the cabinet mid-evening) depends on it; Rust's std
/// `TcpListener::bind` does not set it.
#[cfg(unix)]
fn bind_reuseaddr(port: u16) -> Option<TcpListener> {
    use std::os::fd::FromRawFd;
    unsafe {
        // SOCK_CLOEXEC: kernel sockets belong to ONE process.  Without it,
        // every spawned child (the S4 rebuild driver, the S5 swap target)
        // inherits this listening fd across exec — the zombie copy stays in
        // the SO_REUSEPORT group and eats load-balanced SYNs into a backlog
        // nobody accepts (probe-caught: post-swap dials failed by hash luck).
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return None;
        }
        // Portable CLOEXEC: macOS has no SOCK_CLOEXEC socket flag — set the
        // fd flag right after creation (single-threaded; no exec in between).
        let _ = libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        let one: libc::c_int = 1;
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            std::ptr::addr_of!(one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        // @PLN18 08-S5 — SO_REUSEPORT: during a build swap the NEW process
        // binds the same port while the old one still serves; the overlap is
        // what makes rollback trivial (the old build never stops listening
        // until the new one is proven serving).
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            std::ptr::addr_of!(one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        // Zero-init then set fields: BSD's sockaddr_in has an extra sin_len
        // a struct literal would have to cfg around.
        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_family = libc::AF_INET as libc::sa_family_t;
        addr.sin_port = port.to_be(); // sin_addr stays 0.0.0.0
        if libc::bind(
            fd,
            std::ptr::addr_of!(addr).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ) != 0
            || libc::listen(fd, 128) != 0
        {
            libc::close(fd);
            return None;
        }
        Some(TcpListener::from_raw_fd(fd))
    }
}

#[cfg(all(not(unix), not(target_arch = "wasm32")))]
fn bind_reuseaddr(port: u16) -> Option<TcpListener> {
    {
        let t0 = std::time::Instant::now();
        let r = TcpListener::bind(("0.0.0.0", port));
        crate::net_profile::record(
            "listener/bind",
            t0.elapsed(),
            if r.is_ok() {
                crate::net_profile::Outcome::Ok
            } else {
                crate::net_profile::Outcome::Failed
            },
            None,
        );
        r.ok()
    }
}

// ── The natives (registered in native.rs; declared in lib/engine_host) ──────

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_listen(port, tick_interval_us) -> boolean` — binds the WS listener
/// and, on the same port number, the state-sync UDP socket (05a).  A UDP bind
/// failure is a warning, not an error: everything rides WS until a restart.
pub fn n_kernel_listen(stores: &mut Stores, stack: &mut DbRef) {
    let tick_us = stores.get::<i64>(stack);
    let port = stores.get::<i64>(stack);
    let ok = listen_impl(port, tick_us);
    stores.put(stack, ok);
}

/// The listen body, shared by the bytecode-stack native and the typed
/// (`--native` codegen) twin — one implementation, two calling conventions.
#[cfg(not(target_arch = "wasm32"))]
fn listen_impl(port: i64, tick_us: i64) -> bool {
    bind_reuseaddr(port as u16)
        .map(|listener| {
            let _ = listener.set_nonblocking(true);
            let udp = match bind_udp_reuseport(port as u16) {
                Ok(s) => {
                    let _ = s.set_nonblocking(true);
                    Some(s)
                }
                Err(e) => {
                    eprintln!("engine_host: UDP bind on {port} failed ({e}); state-sync rides WS");
                    None
                }
            };
            KERNEL.with(|k| {
                *k.borrow_mut() = Some(Kernel {
                    listener,
                    udp,
                    conns: Vec::new(),
                    net: Vec::new(),
                    events: VecDeque::new(),
                    last: Event {
                        cid: -1,
                        kind: -1,
                        payload: String::new(),
                        status: 0,
                    },
                    last_sync: (-1, -1, String::new()),
                    start: Instant::now(),
                    tick_interval_us: tick_us.max(1),
                    last_tick_us: 0,
                    warned_oversize: false,
                    alive: true,
                    debug_names: std::collections::HashMap::new(),
                    debug_relay: std::collections::HashMap::new(),
                });
            });
            // @PLN18 08-S5 — the swap-resume handshake: when this process was
            // booted as a swap target, the parent polls this file; touching
            // it means "the new build is serving" and the parent retires.
            if let Ok(ready) = std::env::var("LOFT_SWAP_READY") {
                let _ = std::fs::write(&ready, b"serving");
                eprintln!("loft-swap: new build serving on port {port} (ready file touched)");
            }
        })
        .is_some()
}

/// UDP bind with `SO_REUSEPORT` (the swap-overlap requirement — see
/// `bind_reuseaddr`).  Datagrams during the brief dual-bind window
/// load-balance between old and new; the sync class tolerates that loss
/// by design (latest-value semantics).
#[cfg(unix)]
fn bind_udp_reuseport(port: u16) -> std::io::Result<UdpSocket> {
    use std::os::fd::FromRawFd;
    unsafe {
        // CLOEXEC — same one-process invariant as the TCP listener (set via
        // fcntl: macOS has no SOCK_CLOEXEC socket flag).
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let _ = libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        let one: libc::c_int = 1;
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            std::ptr::addr_of!(one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_family = libc::AF_INET as libc::sa_family_t;
        addr.sin_port = port.to_be(); // sin_addr stays 0.0.0.0
        if libc::bind(
            fd,
            std::ptr::addr_of!(addr).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ) != 0
        {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        Ok(UdpSocket::from_raw_fd(fd))
    }
}

#[cfg(all(not(unix), not(target_arch = "wasm32")))]
fn bind_udp_reuseport(port: u16) -> std::io::Result<UdpSocket> {
    UdpSocket::bind(("0.0.0.0", port))
}

#[cfg(not(target_arch = "wasm32"))]
/// Drain every pending datagram into the conflation slots; bind/refresh UDP
/// paths; expire silent ones.  Called from the pump sweep — datagram arrivals
/// do NOT count as loop work (they conflate; the tick reads the newest).
fn pump_udp(k: &mut Kernel) {
    let Some(udp) = k.udp.as_ref() else {
        return;
    };
    let now = k.now_us();
    let mut buf = [0u8; 2048];
    loop {
        let (len, from) = match udp.recv_from(&mut buf) {
            Ok(r) => r,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => break,
        };
        let Ok(text) = std::str::from_utf8(&buf[..len]) else {
            continue; // not ours (binary lands with the wire-schema table)
        };
        if let Some(cookie) = text.strip_prefix("H:") {
            // Handshake: bind the source addr to the cid owning this cookie.
            let found = k
                .net
                .iter()
                .position(|n| !n.cookie.is_empty() && n.cookie == cookie);
            if let Some(cid) = found
                && k.conns.get(cid).is_some_and(Option::is_some)
            {
                k.net[cid].path = Some(UdpPath {
                    addr: from,
                    last_seen_us: now,
                });
                let _ = udp.send_to(format!("A:{cid}").as_bytes(), from);
            }
            // Unknown cookie: silently dropped (spoof/staleness — the client
            // retries its hello until acked).
            continue;
        }
        // Everything else requires a bound path (identity = source addr).
        let Some(cid) = k
            .net
            .iter()
            .position(|n| n.path.as_ref().is_some_and(|p| p.addr == from))
        else {
            continue;
        };
        if let Some(p) = k.net[cid].path.as_mut() {
            p.last_seen_us = now; // any datagram is a keepalive
        }
        if let Some(rest) = text.strip_prefix("S:")
            && let Some((seq_s, payload)) = rest.split_once(':')
            && let Ok(seq) = seq_s.parse::<i64>()
        {
            // Conflate per (sender, msg_id): two sync kinds from one client
            // each keep their own newest; stale/reordered seqs drop.
            conflate_slot(&mut k.net[cid].slots, seq, payload);
        }
        // "K:" (bare keepalive) needs nothing beyond the refresh above.
    }
    // Expire silent paths — sends fall back to WS transparently.
    for n in &mut k.net {
        if n.path
            .as_ref()
            .is_some_and(|p| now - p.last_seen_us > UDP_TIMEOUT_US)
        {
            n.path = None;
        }
    }
}

/// `kernel_pump() -> integer` — accept pending connections and drain every
/// ready frame into the event queue; returns the number of events enqueued.
/// One sweep, non-blocking throughout (idle clients cost a peek-µs each).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_pump(stores: &mut Stores, stack: &mut DbRef) {
    let n = with_kernel(pump_kernel).unwrap_or(0);
    stores.put(stack, n);
}

/// The pump body, shared by both calling conventions.
#[cfg(not(target_arch = "wasm32"))]
fn pump_kernel(k: &mut Kernel) -> i64 {
    drain_http_done(&mut k.events);
    {
        let mut added = 0i64;
        // Accept every pending connection this turn.
        loop {
            match k.listener.accept() {
                Ok((stream, _)) => {
                    let cid = k
                        .conns
                        .iter()
                        .position(Option::is_none)
                        .unwrap_or(k.conns.len());
                    // Mint the cookie BEFORE the upgrade so the 101 response
                    // carries the same value `ClientNet` stores — the whole
                    // negotiation stays kernel-internal.
                    let cookie = k.fresh_cookie(cid);
                    if let Some(s) = ws_upgrade(stream, &cookie) {
                        let net = ClientNet::new(cookie);
                        if cid == k.conns.len() {
                            k.conns.push(Some(s));
                            k.net.push(net);
                        } else {
                            k.conns[cid] = Some(s);
                            k.net[cid] = net; // reused cid starts fresh
                        }
                        k.events.push_back(Event {
                            cid: cid as i64,
                            kind: 0,
                            payload: String::new(),
                            status: 0,
                        });
                        added += 1;
                    }
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        // Drain ready frames from every connection (drain-to-empty per conn is
        // safe here: the queue grows in memory, and the LOFT loop budget-drains
        // the queue — the kernel never blocks on a quiet socket).
        for cid in 0..k.conns.len() {
            while let Some(stream) = k.conns[cid].as_mut() {
                match read_frame(stream) {
                    FrameRead::None => break,
                    FrameRead::Text(payload) => {
                        // @PLN18 08-S7 — the debug control channel: `D!:`
                        // frames are kernel-handled (the game script never
                        // sees them), gated on LOFT_DEBUG_CONTROL=1 and a
                        // LOOPBACK peer.
                        if let Some(cmd) = payload.strip_prefix("D!:") {
                            handle_debug_control(k, cid, cmd);
                            continue;
                        }
                        // Wire-schema routing on the inbound reliable carrier
                        // too: a sync-class frame (a phone's pose) conflates
                        // exactly like a datagram — the server script reads
                        // ONE surface (sync_next) regardless of the sender's
                        // transport.
                        if is_sync_msg(&payload) {
                            conflate_ws(&mut k.net[cid].slots, &payload);
                        } else {
                            k.events.push_back(Event {
                                cid: cid as i64,
                                kind: 1,
                                payload,
                                status: 0,
                            });
                            added += 1;
                        }
                    }
                    FrameRead::Closed => {
                        disconnect(k, cid);
                        added += 1;
                        break;
                    }
                }
            }
        }
        // 05a: conflate pending state-sync datagrams (not counted as work —
        // the tick reads the newest; see pump_udp).
        pump_udp(k);
        added
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Close a connection slot: the cookie dies with the WS session (the UDP path
/// and conflation slot go with it) and a disconnect event is queued.
fn disconnect(k: &mut Kernel, cid: usize) {
    k.conns[cid] = None;
    k.net[cid] = ClientNet::new(String::new());
    k.events.push_back(Event {
        cid: cid as i64,
        kind: 2,
        payload: String::new(),
        status: 0,
    });
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_next_event() -> boolean` — pop the queue head into the event
/// getters below.
pub fn n_kernel_next_event(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_kernel(|k| match k.events.pop_front() {
        Some(ev) => {
            k.last = ev;
            true
        }
        None => false,
    })
    .unwrap_or(false);
    stores.put(stack, got);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_event_cid(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last.cid).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_event_kind(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last.kind).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// Destination-passing text return (@PLN10's convention for text-producing
/// natives): the caller allocates the destination and passes its `DbRef`;
/// routed by `is_text_dest_native("n_kernel_event_payload")`.
pub fn n_kernel_event_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let v = with_kernel(|k| k.last.payload.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_tick_due() -> boolean` — drift-free: when a tick is due, advance
/// `last_tick += interval` (never `= now`), so missed time is caught up tick
/// by tick instead of silently dropped.
pub fn n_kernel_tick_due(stores: &mut Stores, stack: &mut DbRef) {
    let due = with_kernel(tick_due_kernel).unwrap_or(false);
    stores.put(stack, due);
}

#[cfg(not(target_arch = "wasm32"))]
fn tick_due_kernel(k: &mut Kernel) -> bool {
    let now = k.start.elapsed().as_micros() as i64;
    if now - k.last_tick_us >= k.tick_interval_us {
        if k.last_tick_us == 0 {
            k.last_tick_us = now; // first tick anchors the grid
        } else {
            k.last_tick_us += k.tick_interval_us;
        }
        true
    } else {
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Deliver one message to one client, routed by the wire-schema table: a
/// sync-class message (`sync_class(msg_id)`) rides a seq-stamped datagram
/// when the client's UDP path is bound; everything else — and every client
/// without a path — gets a WS frame.  ONE function, the fastest path that
/// client supports; meaning never branches on transport.
fn deliver(k: &mut Kernel, cid: usize, msg: &str, sync: bool) -> bool {
    if sync
        && let Some(net) = k.net.get_mut(cid)
        && let Some(addr) = net.path.as_ref().map(|p| p.addr)
        && let Some(udp) = k.udp.as_ref()
    {
        net.out_seq += 1;
        let dgram = format!("S:{}:{msg}", net.out_seq);
        if dgram.len() > UDP_MAX_DATAGRAM {
            if !k.warned_oversize {
                k.warned_oversize = true;
                eprintln!(
                    "engine_host: dropped a {} B sync datagram (cap {} B; \
                     state frames must stay small — bulk belongs on the WS channel)",
                    dgram.len(),
                    UDP_MAX_DATAGRAM
                );
            }
            return false;
        }
        return udp.send_to(dgram.as_bytes(), addr).is_ok();
    }
    // Event class, or no UDP path: the reliable channel.
    let Some(Some(stream)) = k.conns.get_mut(cid) else {
        return false;
    };
    if write_frame(stream, 0x1, msg.as_bytes()).is_err() {
        disconnect(k, cid);
        return false;
    }
    true
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_send(cid, msg) -> boolean` — class-routed delivery (see `deliver`).
pub fn n_kernel_send(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let cid = stores.get::<i64>(stack);
    let sync = is_sync_msg(&msg);
    let ok = with_kernel(|k| deliver(k, cid as usize, &msg, sync)).unwrap_or(false);
    stores.put(stack, ok);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_broadcast(msg) -> integer` — class-routed delivery to every live
/// connection (each client gets its own fastest path); returns the delivery
/// count.  A failed WS send disconnects that client.
pub fn n_kernel_broadcast(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let sync = is_sync_msg(&msg);
    let n = with_kernel(|k| {
        let mut sent = 0i64;
        for cid in 0..k.conns.len() {
            if k.conns[cid].is_none() {
                continue;
            }
            if deliver(k, cid, &msg, sync) {
                sent += 1;
            }
        }
        sent
    })
    .unwrap_or(0);
    stores.put(stack, n);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_idle(max_us)` — sleep, but never past the next tick boundary.
/// Called by the loft loop only when a turn produced no work.
pub fn n_kernel_idle(stores: &mut Stores, stack: &mut DbRef) {
    let max_us = stores.get::<i64>(stack);
    let sleep_us = with_kernel(|k| {
        let now = k.start.elapsed().as_micros() as i64;
        let until_tick = if k.last_tick_us == 0 {
            k.tick_interval_us
        } else {
            (k.last_tick_us + k.tick_interval_us - now).max(0)
        };
        max_us.clamp(0, until_tick.max(1))
    })
    .unwrap_or(max_us.max(0));
    std::thread::sleep(Duration::from_micros(sleep_us as u64));
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_clients() -> integer` — live connection count (diagnostics).
pub fn n_kernel_clients(stores: &mut Stores, stack: &mut DbRef) {
    let n = with_kernel(|k| k.conns.iter().filter(|c| c.is_some()).count() as i64).unwrap_or(0);
    stores.put(stack, n);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_alive() -> boolean` — `run`'s loop condition; false after
/// `kernel_stop()` (the windowed listener's window-close exit).
pub fn n_kernel_alive(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.alive).unwrap_or(false);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_stop()` — mark the listener done so `run` returns at the top of
/// its next turn (mirror of the connector's `client_stop`).
pub fn n_kernel_stop(_stores: &mut Stores, _stack: &mut DbRef) {
    let _ = with_kernel(|k| k.alive = false);
}

/// `kernel_frame()` — the per-turn yield point in `run`'s loop: a no-op on
/// native (the loop owns its thread), a frame-yield in the browser.  The
/// listener twin of `kernel_client_frame`, so a windowed LISTENER frames too.
pub fn n_kernel_frame(_stores: &mut Stores, _stack: &mut DbRef) {}

/// The local-event enqueue shared by both calling conventions: window input
/// (or any script-side source) becomes an ordinary events-class message —
/// `cid: -1` marks local origin, handlers treat keys and remote messages
/// identically (and K4-style intent shipping serializes the SAME stream).
/// Posts to the active role's queue: connector/local first, else listener.
/// False = no kernel is booted.
#[cfg(not(target_arch = "wasm32"))]
fn post_impl(msg: &str) -> bool {
    let ev = |payload: String| Event {
        cid: -1,
        kind: 1,
        payload,
        status: 0,
    };
    if with_client(|c| c.events.push_back(ev(msg.to_string()))).is_some() {
        return true;
    }
    with_kernel(|k| k.events.push_back(ev(msg.to_string()))).is_some()
}

#[cfg(not(target_arch = "wasm32"))]
/// `post(msg) -> boolean` — see [`post_impl`].
pub fn n_kernel_post(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let ok = post_impl(&msg);
    stores.put(stack, ok);
}

// ── 05a — the state-sync UDP channel ────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
/// `udp_bound(cid) -> boolean` — does this client have a live UDP path?
pub fn n_kernel_udp_bound(stores: &mut Stores, stack: &mut DbRef) {
    let cid = stores.get::<i64>(stack);
    let v =
        with_kernel(|k| k.net.get(cid as usize).is_some_and(|n| n.path.is_some())).unwrap_or(false);
    stores.put(stack, v);
}

/// `sync_class(msg_id)` — declare a `msg_id` as latest-value state in the
/// wire-schema table: `send`/`broadcast` then route messages of that kind to
/// the sync channel (datagram when the client can, WS frame when it can't)
/// and inbound datagrams of that kind conflate.  Data, not a per-call API —
/// the developer states what a message IS once; the kernel picks transports.
pub fn n_kernel_sync_class(stores: &mut Stores, stack: &mut DbRef) {
    let msg_id = stores.get::<i64>(stack);
    SYNC_IDS.with(|s| {
        s.borrow_mut().insert(msg_id, false);
    });
}

/// `sync_class_keyed(msg_id)` — a sync kind whose payload's FIRST field is an
/// entity id: conflation keeps the newest per (peer, kind, entity), so one
/// kind carries N entities' latest-values ("state-sync keyed by cid").
pub fn n_kernel_sync_class_keyed(stores: &mut Stores, stack: &mut DbRef) {
    let msg_id = stores.get::<i64>(stack);
    SYNC_IDS.with(|s| {
        s.borrow_mut().insert(msg_id, true);
    });
}

#[cfg(not(target_arch = "wasm32"))]
/// Deliver one PRIORITY KEYFRAME — a sync-stream sample promoted to
/// must-deliver (the class table's discontinuity rule: a bounce must not be
/// lost the way a smooth sample may be).  Rides the reliable channel, stamped
/// in the SAME seq space as the datagrams so the receiver's slots totally
/// order the two carriers: a bound connector gets an `S:`-framed WS frame
/// (it lands in the conflation slots, and any in-flight OLDER datagram is
/// then discarded as stale); an unbound peer (a web page) gets the plain
/// message — which is already its normal delivery.  WHAT counts as a
/// discontinuity is meaning; only delivery is mechanics.
fn deliver_keyframe(k: &mut Kernel, cid: usize, msg: &str) -> bool {
    let Some(net) = k.net.get_mut(cid) else {
        return false;
    };
    let framed = if net.path.is_some() {
        net.out_seq += 1;
        Some(format!("S:{}:{msg}", net.out_seq))
    } else {
        None
    };
    let Some(Some(stream)) = k.conns.get_mut(cid) else {
        return false;
    };
    let wire = framed.as_deref().unwrap_or(msg);
    if write_frame(stream, 0x1, wire.as_bytes()).is_err() {
        disconnect(k, cid);
        return false;
    }
    true
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_keyframe(cid, msg) -> boolean` — see `deliver_keyframe`.
pub fn n_kernel_keyframe(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let cid = stores.get::<i64>(stack);
    let ok = with_kernel(|k| deliver_keyframe(k, cid as usize, &msg)).unwrap_or(false);
    stores.put(stack, ok);
}

#[cfg(not(target_arch = "wasm32"))]
/// `sync_next() -> boolean` — load the next dirty conflation slot (newest
/// state from one client) into the getters below and mark it read.  Draining
/// inside `on_tick` gives late-latch by construction: the freshest datagram
/// that arrived before the tick is the one read.
pub fn n_kernel_sync_next(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_kernel(sync_next_kernel).unwrap_or(false);
    stores.put(stack, got);
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_next_kernel(k: &mut Kernel) -> bool {
    for cid in 0..k.net.len() {
        for slot in &mut k.net[cid].slots {
            if slot.dirty {
                slot.dirty = false;
                // The payload carries its own `<msg_id>:` framing, so the
                // drained message reads exactly like an event payload.
                k.last_sync = (cid as i64, slot.seq, slot.payload.clone());
                return true;
            }
        }
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_sync_cid(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last_sync.0).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_sync_seq(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last_sync.1).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// Destination-passing text return — see `n_kernel_event_payload_dest`.
pub fn n_kernel_sync_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let v = with_kernel(|k| k.last_sync.2.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

// ── The connector role — the client-side kernel (one core, two roles) ───────
//
// A native client's half of the auto-path: connect + WS upgrade, read the
// `X-Loft-UDP` cookie from the 101 head, auto-hello until acked, keepalive
// cadence (the phone-radio wake), class-routed `client_send`, and inbound
// conflation through the SAME `conflate_slot`/`SYNC_IDS` machinery the
// listener uses — the queue semantics never fork between roles.  The loft
// surface is `run_client(host, port, tick_us, on_event, on_tick)`, which
// returns when the server connection dies (a connector without a server has
// nothing left to do — unlike `run`, which serves forever).

#[cfg(not(target_arch = "wasm32"))]
/// Hello retry cadence while unbound (the ack races real traffic, so this is
/// a retry, not a timeout).
const HELLO_INTERVAL_US: i64 = 200_000;
#[cfg(not(target_arch = "wasm32"))]
/// Keepalive cadence once bound — far inside the listener's 3 s path timeout.
const KEEPALIVE_INTERVAL_US: i64 = 500_000;

#[cfg(not(target_arch = "wasm32"))]
struct ClientKernel {
    /// `None` = a local (transportless) client kernel: the same loop drives a
    /// windowed host with no server — ticks, swap machinery, and the debug
    /// control endpoint all work; sends report false, the pump reads nothing.
    conn: Option<TcpStream>,
    udp: Option<UdpSocket>,
    /// From the 101 head; "" = the server offered no UDP (everything WS).
    cookie: String,
    udp_bound: bool,
    last_hello_us: i64,
    last_keepalive_us: i64,
    out_seq: i64,
    /// Inbound conflation slots for the server's sync traffic (per msg_id).
    slots: Vec<SyncSlot>,
    /// (seq, payload) handed out by the last `client_sync_next`.
    last_sync: (i64, String),
    events: VecDeque<Event>,
    last: Event,
    start: Instant,
    tick_interval_us: i64,
    last_tick_us: i64,
    /// False once the server connection closed — `run_client` then returns.
    alive: bool,
    /// @PLN18 08-S7 — the client-side debug control endpoint.  A debuggable
    /// CLIENT has no game port a debugger could dial, so under
    /// LOFT_DEBUG_CONTROL=1 it announces its own loopback listener
    /// ("engine_host: debug control on 127.0.0.1:<port>" on stdout — the
    /// editor scrapes it exactly like a server's port announce).
    ctl_listener: Option<TcpListener>,
    ctl_conns: Vec<Option<TcpStream>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ClientKernel {
    fn now_us(&self) -> i64 {
        self.start.elapsed().as_micros() as i64
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static CLIENT: RefCell<Option<ClientKernel>> = const { RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
fn with_client<R>(f: impl FnOnce(&mut ClientKernel) -> R) -> Option<R> {
    CLIENT.with(|c| c.borrow_mut().as_mut().map(f))
}

#[cfg(not(target_arch = "wasm32"))]
/// Client→server frames are masked (RFC 6455 requires it; `read_frame` on the
/// listener side already handles both).  The mask key needs no randomness for
/// security — it exists to defeat broken transparent proxies — so a cheap
/// counter-derived key is fine.
fn write_frame_masked(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    if payload.len() <= 125 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= 0xFFFF {
        frame.push(0x80 | 0x7E);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 0x7F);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let mask = (payload.len() as u32)
        .wrapping_mul(0x9E37_79B9)
        .to_be_bytes();
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    stream.write_all(&frame)
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_connect(host, port, tick_interval_us) -> boolean` — TCP connect,
/// WS upgrade (the fixed RFC sample key: the key field is anti-cache, not
/// auth), capture the `X-Loft-UDP` cookie, prepare the UDP socket.  Queues a
/// kind-0 event so `on_event` sees the connect like the listener side does.
pub fn n_kernel_connect(stores: &mut Stores, stack: &mut DbRef) {
    let tick_us = stores.get::<i64>(stack);
    let port = stores.get::<i64>(stack);
    let host = stores.get::<Str>(stack).str().to_owned();
    let ok = client_connect(&host, port as u16, tick_us).is_some();
    stores.put(stack, ok);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_local(tick_interval_us) -> boolean` — boot the client kernel with
/// NO transport: a standalone windowed host gets the same loop (drift-free
/// ticks, swap machinery, debug control endpoint, frame yield) without a
/// server.  `client_send` reports false; the event queue only ever holds
/// what a future local source enqueues.
pub fn n_kernel_local(stores: &mut Stores, stack: &mut DbRef) {
    let tick_us = stores.get::<i64>(stack);
    local_init(tick_us);
    stores.put(stack, true);
}

#[cfg(not(target_arch = "wasm32"))]
fn local_init(tick_us: i64) {
    CLIENT.with(|c| {
        *c.borrow_mut() = Some(ClientKernel {
            conn: None,
            udp: None,
            cookie: String::new(),
            udp_bound: false,
            last_hello_us: i64::MIN / 2,
            last_keepalive_us: 0,
            out_seq: 0,
            slots: Vec::new(),
            last_sync: (-1, String::new()),
            // No peer, so no kind-0 connect event — the queue starts empty.
            events: VecDeque::new(),
            last: Event {
                cid: -1,
                kind: -1,
                payload: String::new(),
                status: 0,
            },
            start: Instant::now(),
            tick_interval_us: tick_us.max(1),
            last_tick_us: 0,
            alive: true,
            ctl_listener: bind_ctl_listener(),
            ctl_conns: Vec::new(),
        });
        // The swap-resume handshake (08-S5): a local kernel's "serving" is
        // simply BOOTED — same signal as the connector's connected.
        if let Ok(ready) = std::env::var("LOFT_SWAP_READY") {
            let _ = std::fs::write(&ready, b"connected");
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn client_connect(host: &str, port: u16, tick_us: i64) -> Option<()> {
    use std::net::ToSocketAddrs;
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    // @PLN18 08-S5 — seat reconnect semantics: a build swap closes every
    // connection and the new process binds moments later; retry the dial
    // across that gap (5 s ≫ the measured handover) so `run_client` in a
    // `while` wrapper rides a swap through.  First-connect failures to a
    // dead host still fail fast enough for scripts (one bounded window).
    let dial_deadline = Instant::now() + Duration::from_secs(5);
    let mut conn = loop {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Ok(c) => break c,
            Err(_) if Instant::now() < dial_deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    };
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let req = format!(
        "GET /ws HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    conn.write_all(req.as_bytes()).ok()?;
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match conn.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return None,
        }
        if head.len() > 16 * 1024 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&head);
    if !head.contains(" 101 ") {
        return None;
    }
    let cookie = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("x-loft-udp")
                .then(|| v.trim().to_string())
        })
        .unwrap_or_default();
    // Frame phase: the listener's peek pattern bounds idle reads.
    conn.set_read_timeout(Some(Duration::from_millis(20)))
        .ok()?;
    let udp = if cookie.is_empty() {
        None
    } else {
        UdpSocket::bind("0.0.0.0:0").ok().and_then(|s| {
            s.connect(addr).ok()?;
            s.set_nonblocking(true).ok()?;
            Some(s)
        })
    };
    CLIENT.with(|c| {
        *c.borrow_mut() = Some(ClientKernel {
            conn: Some(conn),
            udp,
            cookie,
            udp_bound: false,
            last_hello_us: i64::MIN / 2,
            last_keepalive_us: 0,
            out_seq: 0,
            slots: Vec::new(),
            last_sync: (-1, String::new()),
            events: {
                let mut q = VecDeque::new();
                q.push_back(Event {
                    cid: 0,
                    kind: 0,
                    payload: String::new(),
                    status: 0,
                });
                q
            },
            last: Event {
                cid: -1,
                kind: -1,
                payload: String::new(),
                status: 0,
            },
            start: Instant::now(),
            tick_interval_us: tick_us.max(1),
            last_tick_us: 0,
            alive: true,
            ctl_listener: bind_ctl_listener(),
            ctl_conns: Vec::new(),
        });
        // @PLN18 08-S5 — the swap-resume handshake, CLIENT form: a client's
        // "serving" is CONNECTED.  Touching the file tells the retiring
        // parent the new build is live (mirror of listen_impl's signal).
        if let Ok(ready) = std::env::var("LOFT_SWAP_READY") {
            let _ = std::fs::write(&ready, b"connected");
            eprintln!("loft-swap: new build connected (ready file touched)");
        }
        Some(())
    })
}

/// Bind the client's loopback debug-control listener (None unless
/// LOFT_DEBUG_CONTROL=1).  Ephemeral port, announced on stdout.
#[cfg(not(target_arch = "wasm32"))]
fn bind_ctl_listener() -> Option<TcpListener> {
    if !debug_control_enabled() {
        return None;
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let _ = listener.set_nonblocking(true);
    if let Ok(addr) = listener.local_addr() {
        println!("engine_host: debug control on 127.0.0.1:{}", addr.port());
    }
    Some(listener)
}

/// Accept + serve the client's control connections: upgrade new dials, read
/// `D!:` frames, dispatch through the SAME command core the server side
/// uses.  Replies ride the control conn (see `debug_send`'s role routing).
#[cfg(not(target_arch = "wasm32"))]
fn pump_ctl(c: &mut ClientKernel) {
    let Some(listener) = c.ctl_listener.as_ref() else {
        return;
    };
    while let Ok((stream, _)) = listener.accept() {
        if let Some(upgraded) = ws_upgrade(stream, "") {
            let slot = c.ctl_conns.iter().position(Option::is_none);
            match slot {
                Some(i) => c.ctl_conns[i] = Some(upgraded),
                None => c.ctl_conns.push(Some(upgraded)),
            }
        }
    }
    for i in 0..c.ctl_conns.len() {
        while let Some(stream) = c.ctl_conns[i].as_mut() {
            match read_frame(stream) {
                FrameRead::None => break,
                FrameRead::Text(payload) => {
                    if let Some(cmd) = payload.strip_prefix("D!:") {
                        let reply = debug_cmd_dispatch(i as i64, cmd);
                        if let Some(msg) = reply
                            && let Some(stream) = c.ctl_conns[i].as_mut()
                        {
                            let _ = write_frame(stream, 1, msg.as_bytes());
                        }
                        if QUIT_AFTER_REPLY.load(std::sync::atomic::Ordering::Relaxed) {
                            std::process::exit(0);
                        }
                    }
                }
                FrameRead::Closed => {
                    c.ctl_conns[i] = None;
                    break;
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_pump() -> integer` — drain server WS frames into the event
/// queue and server datagrams into the conflation slots; drive the hello /
/// keepalive cadences.  Datagram arrivals are not counted as work (they
/// conflate; the tick reads the newest — same rule as the listener).
pub fn n_kernel_client_pump(stores: &mut Stores, stack: &mut DbRef) {
    let n = with_client(pump_client).unwrap_or(0);
    stores.put(stack, n);
}

/// The connector pump body, shared by both calling conventions.
#[cfg(not(target_arch = "wasm32"))]
fn pump_client(c: &mut ClientKernel) -> i64 {
    pump_ctl(c); // the debug control endpoint rides every pump (incl. pauses)
    drain_http_done(&mut c.events);
    {
        let mut added = 0i64;
        // WS frames: events from the server — except `S:`-framed keyframes,
        // which are promoted sync samples riding the reliable channel in the
        // datagram seq space: they conflate (the tick reads the newest, and
        // an in-flight older datagram becomes stale), never queue.
        while c.alive {
            let Some(conn) = c.conn.as_mut() else { break };
            match read_frame(conn) {
                FrameRead::None => break,
                FrameRead::Text(payload) => {
                    if let Some(rest) = payload.strip_prefix("S:")
                        && let Some((seq_s, body)) = rest.split_once(':')
                        && let Ok(seq) = seq_s.parse::<i64>()
                    {
                        conflate_slot(&mut c.slots, seq, body);
                        continue;
                    }
                    // Plain sync-class frames (the pre-bind fallback) conflate
                    // too — the script's surface is sync_next either way.
                    if is_sync_msg(&payload) {
                        conflate_ws(&mut c.slots, &payload);
                        continue;
                    }
                    c.events.push_back(Event {
                        cid: 0,
                        kind: 1,
                        payload,
                        status: 0,
                    });
                    added += 1;
                }
                FrameRead::Closed => {
                    c.alive = false;
                    c.udp_bound = false;
                    c.events.push_back(Event {
                        cid: 0,
                        kind: 2,
                        payload: String::new(),
                        status: 0,
                    });
                    added += 1;
                }
            }
        }
        // Datagrams: the ack and the server's sync traffic.
        if let Some(udp) = c.udp.as_ref() {
            // Measurement-only loss injection (@PLN18 phase 04, the loss%
            // axis): `LOFT_UDP_DROP_NTH=N` drops every Nth received sync
            // datagram, deterministically — loopback has no real loss, and
            // the sync class's claim is graceful degradation under it.
            // Read once; 0 = off.
            thread_local! {
                static DROP_NTH: u64 = std::env::var("LOFT_UDP_DROP_NTH")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                static RECV_COUNT: RefCell<u64> = const { RefCell::new(0) };
            }
            let drop_nth = DROP_NTH.with(|d| *d);
            let mut buf = [0u8; 2048];
            loop {
                let len = match udp.recv(&mut buf) {
                    Ok(l) => l,
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => break,
                };
                let Ok(text) = std::str::from_utf8(&buf[..len]) else {
                    continue;
                };
                if text.starts_with("A:") {
                    c.udp_bound = true;
                } else if let Some(rest) = text.strip_prefix("S:")
                    && let Some((seq_s, payload)) = rest.split_once(':')
                    && let Ok(seq) = seq_s.parse::<i64>()
                {
                    if drop_nth > 0 {
                        let nth = RECV_COUNT.with(|r| {
                            let mut r = r.borrow_mut();
                            *r += 1;
                            *r
                        });
                        if nth.is_multiple_of(drop_nth) {
                            continue; // injected loss — the datagram never existed
                        }
                    }
                    conflate_slot(&mut c.slots, seq, payload);
                }
            }
            // Hello until acked; keepalive once bound (the radio wake).
            let now = c.now_us();
            if !c.udp_bound && !c.cookie.is_empty() && now - c.last_hello_us > HELLO_INTERVAL_US {
                let _ = udp.send(format!("H:{}", c.cookie).as_bytes());
                c.last_hello_us = now;
            }
            if c.udp_bound && now - c.last_keepalive_us > KEEPALIVE_INTERVAL_US {
                let _ = udp.send(b"K:");
                c.last_keepalive_us = now;
            }
        }
        added
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_alive() -> boolean` — false once the server connection
/// closed; `run_client`'s loop condition.
pub fn n_kernel_client_alive(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.alive).unwrap_or(false);
    stores.put(stack, v);
}

/// `client_stop()` — the client's own exit: mark the connection dead so
/// `run_client` returns at the top of its next turn.  The GL projector calls
/// it when the WINDOW closes (the loop owner is the connector, so the
/// script needs a way to leave it).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_client_stop(_stores: &mut Stores, _stack: &mut DbRef) {
    let _ = with_client(|c| c.alive = false);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_client_next_event(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_client(|c| match c.events.pop_front() {
        Some(ev) => {
            c.last = ev;
            true
        }
        None => false,
    })
    .unwrap_or(false);
    stores.put(stack, got);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_client_event_kind(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.last.kind).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_event_cid() -> integer` — the last event's origin: `0` =
/// the server, `-1` = a local `post` (window input).  Connector events were
/// all server-origin before `post` existed; the cid keeps them apart.
pub fn n_kernel_client_event_status(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.last.status).unwrap_or(0);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_event_cid() -> integer`.
pub fn n_kernel_client_event_cid(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.last.cid).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// Destination-passing text return — see `n_kernel_event_payload_dest`.
pub fn n_kernel_client_event_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let v = with_client(|c| c.last.payload.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_tick_due() -> boolean` — the listener's drift-free rule.
pub fn n_kernel_client_tick_due(stores: &mut Stores, stack: &mut DbRef) {
    let due = with_client(tick_due_client).unwrap_or(false);
    stores.put(stack, due);
}

#[cfg(not(target_arch = "wasm32"))]
fn tick_due_client(c: &mut ClientKernel) -> bool {
    let now = c.now_us();
    if now - c.last_tick_us >= c.tick_interval_us {
        if c.last_tick_us == 0 {
            c.last_tick_us = now;
        } else {
            c.last_tick_us += c.tick_interval_us;
        }
        true
    } else {
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_idle(max_us)` — sleep, capped at the next tick boundary.
pub fn n_kernel_client_idle(stores: &mut Stores, stack: &mut DbRef) {
    let max_us = stores.get::<i64>(stack);
    let sleep_us = with_client(|c| {
        let now = c.now_us();
        let until_tick = if c.last_tick_us == 0 {
            c.tick_interval_us
        } else {
            (c.last_tick_us + c.tick_interval_us - now).max(0)
        };
        max_us.clamp(0, until_tick.max(1))
    })
    .unwrap_or(max_us.max(0));
    std::thread::sleep(Duration::from_micros(sleep_us as u64));
}

#[cfg(not(target_arch = "wasm32"))]
/// `client_send(msg) -> boolean` — class-routed, mirroring the listener's
/// `deliver`: a sync-class message rides a seq-stamped datagram once the
/// path is acked; everything else — and everything before the ack — goes as
/// a masked WS frame.
pub fn n_kernel_client_send(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let sync = is_sync_msg(&msg);
    let ok = with_client(|c| client_send_impl(c, &msg, sync)).unwrap_or(false);
    stores.put(stack, ok);
}

/// The connector send body, shared by both calling conventions.
#[cfg(not(target_arch = "wasm32"))]
fn client_send_impl(c: &mut ClientKernel, msg: &str, sync: bool) -> bool {
    if !c.alive {
        return false;
    }
    if sync {
        // One counter across BOTH carriers: pre-bind WS sync sends count
        // too, so the first datagram after binding is newer than every
        // ordered-carrier send the server already conflated.
        c.out_seq += 1;
    }
    if sync
        && c.udp_bound
        && let Some(udp) = c.udp.as_ref()
    {
        let dgram = format!("S:{}:{msg}", c.out_seq);
        if dgram.len() > UDP_MAX_DATAGRAM {
            return false; // state frames must stay small (see the listener)
        }
        return udp.send(dgram.as_bytes()).is_ok();
    }
    let Some(conn) = c.conn.as_mut() else {
        return false; // local kernel: there is no peer to send to
    };
    if write_frame_masked(conn, 0x1, msg.as_bytes()).is_err() {
        c.alive = false;
        c.udp_bound = false;
        c.events.push_back(Event {
            cid: 0,
            kind: 2,
            payload: String::new(),
            status: 0,
        });
        return false;
    }
    true
}

#[cfg(not(target_arch = "wasm32"))]
/// `client_sync_next() -> boolean` — drain the newest unread server state,
/// one slot per call (drain inside `on_tick`: late-latch, the listener rule).
pub fn n_kernel_client_sync_next(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_client(sync_next_client).unwrap_or(false);
    stores.put(stack, got);
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_next_client(c: &mut ClientKernel) -> bool {
    for slot in &mut c.slots {
        if slot.dirty {
            slot.dirty = false;
            c.last_sync = (slot.seq, slot.payload.clone());
            return true;
        }
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_client_sync_seq(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.last_sync.0).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// Destination-passing text return — see `n_kernel_event_payload_dest`.
pub fn n_kernel_client_sync_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let v = with_client(|c| c.last_sync.1.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

#[cfg(not(target_arch = "wasm32"))]
/// `client_udp_bound() -> boolean` — read-only introspection (diagnostics).
pub fn n_kernel_client_udp_bound(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.udp_bound).unwrap_or(false);
    stores.put(stack, v);
}

/// `kernel_client_frame()` — the per-turn yield point in `run_client`'s loop:
/// a no-op on native (the loop owns its thread), a frame-yield in the browser
/// (the tab must return to the event loop every turn).  Lives in the SHARED
/// lib source so scripts never see the difference.
pub fn n_kernel_client_frame(_stores: &mut Stores, _stack: &mut DbRef) {}

/// `default_host() -> text` — the host a client should connect to when the
/// script doesn't say: `LOFT_HOST` env, else loopback.  The browser variant
/// returns the page's serving origin (the cabinet).  Destination-passing.
pub fn n_kernel_default_host_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let v = std::env::var("LOFT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

// ── @PLN18 08-S7 — the debug control channel (transport half) ──────────────
//
// `D!:` frames from a LOOPBACK peer (gated on LOFT_DEBUG_CONTROL=1) drive
// the debugger loop: breakpoints, frame eval, resume, reload-now, rebuild,
// swap.  THIS module is transport only — semantics (the parked State, the
// pause loop, the mailbox) live in live_dispatch.  Replies are plain text
// frames to the requesting client; `D:hit` notifications go to the client
// that set the breakpoint.

#[cfg(not(target_arch = "wasm32"))]
fn debug_control_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_DEBUG_CONTROL").is_ok_and(|v| v == "1"))
}

/// Send a control reply/notification to `cid` (no-op for -1 / dead conns).
/// Role-routed: on a LISTENER process `cid` is a kernel client; on a
/// CONNECTOR process it is a control-conn slot (a process is one role).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn debug_send(cid: i64, msg: &str) {
    if cid < 0 {
        return;
    }
    if with_kernel(|k| deliver(k, cid as usize, msg, false)).is_some() {
        return;
    }
    let _ = with_client(|c| {
        if let Some(Some(stream)) = c.ctl_conns.get_mut(cid as usize) {
            let _ = write_frame(stream, 1, msg.as_bytes());
        }
    });
}

/// The mechanics-only mini-pump for the pause loop: accepts, reads frames
/// (control frames are handled en route; game events QUEUE for after the
/// resume), answers keepalives.  Never dispatches meaning.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn debug_pump() {
    let _ = with_kernel(pump_kernel);
    let _ = with_client(pump_client); // incl. the client's control endpoint
}

#[cfg(not(target_arch = "wasm32"))]
fn handle_debug_control(k: &mut Kernel, cid: usize, cmd: &str) {
    if !debug_control_enabled() {
        return; // not a debug host: the frame is silently dropped
    }
    let cmd = cmd.trim();

    // @PLN98 P3.4 — a debug CLIENT (a game peer that opted into `--debug=<name>`)
    // registers its name, so an agent can address it by name.  From ANY peer.
    if let Some(name) = cmd.strip_prefix("iam ") {
        let name = name.trim().to_string();
        k.debug_names.insert(cid, name.clone());
        let _ = deliver(k, cid, &format!("D:registered {name}"), false);
        return;
    }
    // @PLN98 P3.4 — a debug CLIENT relays a `D:` reply back to the agent that
    // sent the outstanding forwarded command.
    if let Some(msg) = cmd.strip_prefix("reply ") {
        if let Some(&agent) = k.debug_relay.get(&cid) {
            let _ = deliver(k, agent, msg.trim(), false);
        }
        return;
    }

    // Everything else is an AGENT command — trusted, so LOOPBACK-gated.
    let loopback = k.conns[cid]
        .as_ref()
        .and_then(|c| c.peer_addr().ok())
        .is_some_and(|a| a.ip().is_loopback());
    if !loopback {
        return;
    }

    // @PLN98 P3.4 — forward `@<name>:<inner>` to the named client's socket and
    // remember the reply route (client → this agent).  The client applies the
    // `D!:<inner>` frame over its own host_input pump and answers with `D!:reply`.
    if let Some(rest) = cmd.strip_prefix('@')
        && let Some((name, inner)) = rest.split_once(':')
    {
        let client = k
            .debug_names
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .map(|(&slot, _)| slot);
        match client {
            Some(client_cid) => {
                k.debug_relay.insert(client_cid, cid);
                let framed = format!("D!:{}", inner.trim());
                let sent = k
                    .conns
                    .get_mut(client_cid)
                    .and_then(|c| c.as_mut())
                    .is_some_and(|conn| write_frame(conn, 1, framed.as_bytes()).is_ok());
                let verdict = if sent { "relayed" } else { "unreachable" };
                let _ = deliver(k, cid, &format!("D:{verdict} {name}"), false);
            }
            None => {
                let _ = deliver(k, cid, &format!("D:err no debug client {name}"), false);
            }
        }
        return;
    }

    // Local (THIS server's own interpreter) debug — the server-authoritative path.
    if let Some(msg) = debug_cmd_dispatch(cid as i64, cmd) {
        let _ = deliver(k, cid, &msg, false);
    }
    if QUIT_AFTER_REPLY.load(std::sync::atomic::Ordering::Relaxed) {
        std::process::exit(0);
    }
}

/// Set by the `quit` command; the reply writers exit after flushing.
#[cfg(not(target_arch = "wasm32"))]
static QUIT_AFTER_REPLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The PROCESS-AGNOSTIC debug command core, shared by both roles (the
/// listener's game-port channel and the connector's loopback endpoint).
/// `cid` is the role-routed reply id (see `debug_send`).  Returns the
/// immediate reply; mailbox-routed commands (eval/resume/reload) answer
/// later through the pause loop.
#[cfg(not(target_arch = "wasm32"))]
fn debug_cmd_dispatch(cid: i64, cmd: &str) -> Option<String> {
    let reply: Option<String> = match cmd.split_once(' ').map_or((cmd, ""), |(a, b)| (a, b)) {
        ("bp", name) if !name.is_empty() => {
            let ok = crate::live_dispatch::debug_set_bp(name, cid);
            Some(format!("D:{} bp {name}", if ok { "ok" } else { "err" }))
        }
        ("flip", rest) => {
            let (name, on) = rest.split_once(' ').unwrap_or((rest, "1"));
            let ok = crate::live_dispatch::set_flip(name, on != "0");
            Some(format!("D:flip {ok}"))
        }
        ("eval", name) if !name.is_empty() => {
            crate::live_dispatch::debug_mailbox_push(crate::live_dispatch::DebugCmd::Eval(
                name.to_string(),
                cid,
            ));
            None // answered by the pause loop
        }
        ("resume", _) => {
            crate::live_dispatch::debug_mailbox_push(crate::live_dispatch::DebugCmd::Resume(cid));
            None
        }
        ("reload", _) => {
            crate::live_dispatch::debug_mailbox_push(crate::live_dispatch::DebugCmd::Reload(cid));
            None
        }
        ("rebuild", _) => {
            let ok = crate::live_dispatch::n_rebuild_start_ctl();
            Some(format!(
                "D:rebuild {}",
                if ok { "started" } else { "refused" }
            ))
        }
        ("rebuild?", _) => Some(format!(
            "D:rebuild {}",
            crate::live_dispatch::rebuild_status_ctl()
        )),
        ("swap", artifact) => {
            let target = if artifact == "auto" || artifact.is_empty() {
                crate::live_dispatch::rebuild_artifact_ctl()
            } else {
                artifact.to_string()
            };
            let ok = swap_start_impl(&target);
            Some(format!("D:swap {ok}"))
        }
        ("quit", _) => {
            // The editor's stop for a SWAPPED game (no longer its child).
            // The exit happens AFTER the caller flushes this reply — sending
            // here would re-borrow the role cell the caller already holds.
            eprintln!("loft-debug: quit via the control channel");
            QUIT_AFTER_REPLY.store(true, std::sync::atomic::Ordering::Relaxed);
            Some("D:quitting".to_string())
        }
        _ => Some(format!("D:err unknown command {cmd:?}")),
    };
    reply
}

/// @PLN98 P3.3 — one message off the shared input channel, classified.
#[cfg(not(target_arch = "wasm32"))]
pub enum InputFrame {
    /// A `D!:`-tagged DEBUG control frame — already dispatched through
    /// [`debug_cmd_dispatch`]; the payload is its immediate reply (if any) for
    /// the caller to send back out (`loft_host_print` on wasm).
    Debug(Option<String>),
    /// PROGRAM input — hand it to the running program's `host_input()`.
    Program(String),
}

/// @PLN98 P3.3 — route ONE message off the shared JS→wasm input channel (`inQ`):
/// a `D!:`-tagged frame is a DEBUG control command (stripped + dispatched through
/// the SAME [`debug_cmd_dispatch`] core the TCP channel feeds), anything else is
/// PROGRAM input.  This is the tag that lets debug control and program input ride
/// ONE queue without colliding — the browser debug pump drains `Debug` frames
/// while the program's `host_input()` consumes `Program` messages.  `cid` is the
/// reply route (the debug output channel).
#[cfg(not(target_arch = "wasm32"))]
pub fn route_input_frame(cid: i64, msg: &str) -> InputFrame {
    match msg.strip_prefix("D!:") {
        Some(cmd) => InputFrame::Debug(debug_cmd_dispatch(cid, cmd.trim())),
        None => InputFrame::Program(msg.to_string()),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod p98_p33_tests {
    use super::{InputFrame, route_input_frame};

    // @PLN98 P3.3 — debug control frames and program input share ONE input channel
    // and must not collide: a `D!:`-tagged message routes to `debug_cmd_dispatch`
    // (the shared command core), everything else passes through as program input.
    #[test]
    fn debug_frames_route_off_the_shared_input_channel() {
        // Program input passes straight through, untouched.
        match route_input_frame(0, "player says: attack") {
            InputFrame::Program(m) => assert_eq!(m, "player says: attack"),
            InputFrame::Debug(_) => panic!("program input was misrouted as a debug frame"),
        }
        // A leading colon / lookalike that is NOT the `D!:` tag stays program input.
        match route_input_frame(0, "Don't panic") {
            InputFrame::Program(m) => assert_eq!(m, "Don't panic"),
            InputFrame::Debug(_) => panic!("a non-tagged message was misrouted as debug"),
        }
        // A `D!:bp` frame is stripped and reaches `debug_cmd_dispatch`'s `bp` arm,
        // which accepts it (no live world is borrowed here, so it is queued on the
        // debug mailbox to apply once a pause holds the State) → "D:ok". This
        // asserts the ROUTING reached the dispatcher's `bp` arm; the set-breakpoint
        // EFFECT over a live world is debug_cmd_dispatch's own contract.
        match route_input_frame(0, "D!:bp some_fn") {
            InputFrame::Debug(reply) => assert_eq!(reply.as_deref(), Some("D:ok bp some_fn")),
            InputFrame::Program(_) => panic!("a D!: frame was misrouted as program input"),
        }
        // An unknown command still routes to the dispatcher (proves strip+dispatch,
        // not merely a prefix check that discards the payload).
        match route_input_frame(0, "D!:frobnicate x") {
            InputFrame::Debug(reply) => {
                assert!(
                    reply.as_deref().unwrap_or("").starts_with("D:err unknown"),
                    "unknown command reached the dispatcher: {reply:?}"
                );
            }
            InputFrame::Program(_) => panic!("unknown D!: command was misrouted"),
        }
    }
}

// ── @PLN18 08-S5 — the native build swap: a new process under a running ────
// world.  The OLD process drives: at a frame boundary it snapshots the
// registered world (schema-walked JSON — the lenient-serialization seed),
// spawns the S4 artifact with LOFT_RESUME pointing at the snapshot, and
// KEEPS SERVING (meaning frozen, mechanics alive) until the child touches
// the READY file (= bound via SO_REUSEPORT and serving).  Then it closes its
// sockets and retires; seats reconnect into the new build (bounded gap).
// Rollback is the default: the child dying or timing out un-freezes the old
// build — it never stopped listening.
//
// v1 bounds (documented in 08-live-build-swap.md): the world is ONE
// record graph named via `swap_world(w)` (scalars/text/vectors/inline
// structs — `populate_struct_from_jsonvalue`'s matrix); events arriving
// INSIDE the freeze window die with the old process (visible as the
// connection close); layout changes between builds are the lenient
// deserializer's problem (missing fields → null, extra → ignored).

#[cfg(not(target_arch = "wasm32"))]
enum SwapPhase {
    Idle,
    Requested(String),
    Waiting {
        child: std::process::Child,
        ready: std::path::PathBuf,
        snap: std::path::PathBuf,
        deadline: Instant,
    },
    Done,
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static SWAP: RefCell<SwapPhase> = const { RefCell::new(SwapPhase::Idle) };
    /// The snapshot root registered by `swap_world`: (record, known_type).
    static WORLD_ROOT: std::cell::Cell<Option<(DbRef, u16)>> = const { std::cell::Cell::new(None) };
}

/// `swap_world(w)` — name the world record: registers it as the snapshot
/// root for future swaps, and — when THIS process is a swap target
/// (`LOFT_RESUME`) — restores the previous build's world INTO it, in
/// place, so every alias (main's var, the handler captures) sees the
/// restored state.  Returns true iff a restore happened.
#[cfg(not(target_arch = "wasm32"))]
fn swap_world_impl(stores: &mut Stores, w: DbRef) -> bool {
    let kt = stores.allocations[w.store_nr as usize].known_type;
    if kt == u16::MAX {
        eprintln!("loft-swap: swap_world got a record with no known type; swaps disabled");
        return false;
    }
    WORLD_ROOT.with(|r| r.set(Some((w, kt))));
    let Ok(snap_path) = std::env::var("LOFT_RESUME") else {
        return false;
    };
    let Ok(json) = std::fs::read_to_string(&snap_path) else {
        eprintln!("loft-swap: LOFT_RESUME set but {snap_path} unreadable; starting fresh");
        return false;
    };
    let jv = crate::native::json_parse_into_stores(stores, &json);
    crate::native::populate_struct_from_jsonvalue(stores, &w, kt, &jv);
    eprintln!("loft-swap: world restored from {snap_path}");
    true
}

/// `swap_start(artifact)` — request a swap to the given binary (normally
/// `rebuild_artifact()`).  The run loop acts at the next frame boundary.
#[cfg(not(target_arch = "wasm32"))]
fn swap_start_impl(artifact: &str) -> bool {
    if artifact.is_empty() || !std::path::Path::new(artifact).exists() {
        eprintln!("loft-swap: no such artifact `{artifact}` — swap refused");
        return false;
    }
    SWAP.with(|sw| {
        let mut sw = sw.borrow_mut();
        if !matches!(*sw, SwapPhase::Idle) {
            eprintln!("loft-swap: a swap is already in progress");
            return false;
        }
        *sw = SwapPhase::Requested(artifact.to_string());
        true
    })
}

/// The per-turn swap step `run()` drives: 0 = serve normally, 1 = FROZEN
/// (mechanics only — pump runs, meaning waits), 2 = handed over (run
/// returns; this process retires).  All failure paths roll back to 0.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_lines)]
fn swap_step_impl(stores: &mut Stores) -> i64 {
    SWAP.with(|sw| {
        let mut sw = sw.borrow_mut();
        match &mut *sw {
            SwapPhase::Idle => 0,
            SwapPhase::Done => 2,
            SwapPhase::Requested(artifact) => {
                let artifact = artifact.clone();
                let Some((w, kt)) = WORLD_ROOT.with(std::cell::Cell::get) else {
                    eprintln!("loft-swap: rolled back (no swap_world registered)");
                    *sw = SwapPhase::Idle;
                    return 0;
                };
                // The frame boundary: meaning is frozen from here on, so
                // this snapshot is THE world state the new build resumes.
                let mut json = String::new();
                stores.show_json(&mut json, &w, kt, false);
                let base = std::env::temp_dir().join(format!("loft_swap_{}", std::process::id()));
                let snap = base.with_extension("snap.json");
                let ready = base.with_extension("ready");
                let _ = std::fs::remove_file(&ready);
                if std::fs::write(&snap, &json).is_err() {
                    eprintln!("loft-swap: rolled back (cannot write snapshot)");
                    *sw = SwapPhase::Idle;
                    return 0;
                }
                let mut cmd = std::process::Command::new(&artifact);
                cmd.env("LOFT_RESUME", &snap)
                    .env("LOFT_SWAP_READY", &ready)
                    // Dispatch reset: the new build has the edits COMPILED —
                    // startup flips must not resurrect the interpreter tier.
                    .env_remove("LOFT_FLIP_FNS")
                    .stdin(std::process::Stdio::null());
                // The new build is a HANDOVER TARGET, not part of this
                // process tree: it must survive the old chain's exit and any
                // group-scoped kill aimed at the retiring driver hierarchy
                // (probe-caught: a group signal reaped the new server after
                // a clean handover).  Its own group makes the cut explicit.
                #[cfg(unix)]
                std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
                match cmd.spawn() {
                    Ok(child) => {
                        eprintln!("loft-swap: booting {artifact} (meaning frozen)");
                        *sw = SwapPhase::Waiting {
                            child,
                            ready,
                            snap,
                            deadline: Instant::now() + std::time::Duration::from_secs(15),
                        };
                        1
                    }
                    Err(e) => {
                        eprintln!("loft-swap: rolled back (cannot spawn {artifact}: {e})");
                        let _ = std::fs::remove_file(&snap);
                        *sw = SwapPhase::Idle;
                        0
                    }
                }
            }
            SwapPhase::Waiting {
                child,
                ready,
                snap,
                deadline,
            } => {
                if ready.exists() {
                    // The new build is serving: hand over.  Dropping the
                    // Kernel closes the listener, the UDP socket and every
                    // connection — seats reconnect into the new process.
                    eprintln!("loft-swap: handing over — this build retires");
                    let _ = std::fs::remove_file(snap);
                    let _ = std::fs::remove_file(ready);
                    *sw = SwapPhase::Done;
                    KERNEL.with(|k| *k.borrow_mut() = None);
                    return 2;
                }
                if let Ok(Some(status)) = child.try_wait() {
                    eprintln!("loft-swap: rolled back (new build exited {status} before serving)");
                    let _ = std::fs::remove_file(snap);
                    let _ = std::fs::remove_file(ready);
                    *sw = SwapPhase::Idle;
                    return 0;
                }
                if Instant::now() > *deadline {
                    eprintln!("loft-swap: rolled back (new build never became ready)");
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(snap);
                    let _ = std::fs::remove_file(ready);
                    *sw = SwapPhase::Idle;
                    return 0;
                }
                1
            }
        }
    })
}

/// `swap_world(w: reference) -> boolean` (stack native).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_swap_world(stores: &mut Stores, stack: &mut DbRef) {
    let w = stores.get::<DbRef>(stack);
    let resumed = swap_world_impl(stores, w);
    stores.put(stack, resumed);
}

/// `swap_start(artifact: text) -> boolean` (stack native).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_swap_start(stores: &mut Stores, stack: &mut DbRef) {
    let artifact = stores.get::<Str>(stack).str().to_owned();
    let ok = swap_start_impl(&artifact);
    stores.put(stack, ok);
}

/// `kernel_swap_step() -> integer` (stack native; run()'s per-turn driver).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_swap_step(stores: &mut Stores, stack: &mut DbRef) {
    let phase = swap_step_impl(stores);
    stores.put(stack, phase);
}

/// `swap_retired() -> boolean` — did THIS process hand its world to a new
/// build?  A loop wrapper (the projector's reconnect-on-drop) must
/// distinguish "the server vanished" (reconnect) from "I retired" (exit):
/// both return from `run_client`, and the swap phase is sticky by design.
#[cfg(not(target_arch = "wasm32"))]
pub fn n_swap_retired(stores: &mut Stores, stack: &mut DbRef) {
    let v = SWAP.with(|sw| matches!(*sw.borrow(), SwapPhase::Done));
    stores.put(stack, v);
}

// ── The BROWSER kernel (@PLN18 phase 07) — the same loft script on a phone ──
//
// The connector role rebuilt on what the browser already provides: the
// `host_ws_*` bridge as the pump, the event loop as the idle (via the
// frame-yield contract), `time_ticks` as the tick grid.  The pure machinery
// above (conflation, the wire-schema table, the queues) is THE SAME CODE —
// one implementation, two targets, so the never-fork rule holds by
// construction.  Registered under the SAME `n_kernel_*` symbols
// (`native.rs::KERNEL_FUNCTIONS_WASM`), so `lib/engine_host`'s loft source
// — and every script over it — is shared verbatim.
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub mod browser {
    use super::*;
    use crate::wasm::{
        host_origin_host, host_time_ticks, host_ws_connect, host_ws_last_message, host_ws_ready,
        host_ws_recv, host_ws_send,
    };

    struct BrowserClient {
        ws: i32,
        alive: bool,
        /// The async half of "connect blocks until upgraded": kind-0 fires
        /// and the outbox flushes only once the socket reports open.
        opened: bool,
        outbox: Vec<String>,
        events: VecDeque<Event>,
        last: Event,
        slots: Vec<SyncSlot>,
        last_sync: (i64, String),
        out_seq: i64,
        tick_interval_us: i64,
        last_tick_us: i64,
    }

    thread_local! {
        static CLIENT: RefCell<Option<BrowserClient>> = const { RefCell::new(None) };
    }

    fn with_client<R>(f: impl FnOnce(&mut BrowserClient) -> R) -> Option<R> {
        CLIENT.with(|c| c.borrow_mut().as_mut().map(f))
    }

    /// @PLN18 08-S6 — `swap_world(w)` in the browser: register the world as
    /// the page's snapshot root, and when the PAGE staged a snapshot (the
    /// swap target's boot), restore it INTO `w` in place — the same
    /// in-place-restore contract as the native LOFT_RESUME leg.
    pub fn n_swap_world(stores: &mut Stores, stack: &mut DbRef) {
        let w = stores.get::<DbRef>(stack);
        let kt = stores.allocations[w.store_nr as usize].known_type;
        if kt == u16::MAX {
            eprintln!("loft-swap: swap_world got a record with no known type; swaps disabled");
            stores.put(stack, false);
            return;
        }
        crate::wasm::swap_root_set(w, kt);
        let resumed = if let Some(json) = crate::wasm::swap_stage_take() {
            let jv = crate::native::json_parse_into_stores(stores, &json);
            crate::native::populate_struct_from_jsonvalue(stores, &w, kt, &jv);
            true
        } else {
            false
        };
        stores.put(stack, resumed);
    }

    /// Browser builds cannot self-swap (the PAGE drives the swap) — a
    /// script's `swap_start` is a warned no-op, never a halt.
    pub fn n_swap_start(stores: &mut Stores, stack: &mut DbRef) {
        let _artifact = stores.get::<Str>(stack).str().to_owned();
        eprintln!("loft-swap: swap_start is page-driven in a browser; ignored");
        stores.put(stack, false);
    }

    /// `kernel_swap_step() -> integer` in the browser: always 0, "run normally".
    ///
    /// The shared `client_loop` calls this EVERY turn, so it is not optional the way the
    /// swap surface looks from outside — and it was the one `n_kernel_*` the browser kernel
    /// never registered, which turned every `run_client` page into a panic the moment its
    /// loop began (loft#1189).  Nothing saw it because the committed bundle predated the
    /// call.
    ///
    /// Zero is the honest answer rather than a placeholder: the browser swap is PAGE-driven
    /// — `n_swap_start` above refuses, so this build can never enter the frozen (1) or
    /// handed-over (2) phases the native driver reports.  The page boots a new instance and
    /// discards this one; the loft program inside never observes the handover.
    pub fn n_kernel_swap_step(stores: &mut Stores, stack: &mut DbRef) {
        stores.put(stack, 0_i64);
    }

    /// `swap_retired() -> boolean` in the browser: always false, for the same reason.
    ///
    /// A reconnect wrapper asks this to tell "the server vanished, redial" from "I retired
    /// after a swap, exit".  A page never takes the second exit, so the answer that keeps a
    /// wrapper redialling is the correct one.
    pub fn n_swap_retired(stores: &mut Stores, stack: &mut DbRef) {
        stores.put(stack, false);
    }

    /// `post(msg) -> boolean` — the local-event enqueue in the browser:
    /// touch/key input becomes an events-class message on the client queue
    /// (`cid: -1` = local origin).  False = no client kernel is booted.
    pub fn n_kernel_post(stores: &mut Stores, stack: &mut DbRef) {
        let msg = stores.get::<Str>(stack).str().to_owned();
        let ok = with_client(|c| {
            c.events.push_back(Event {
                cid: -1,
                kind: 1,
                payload: msg.clone(),
                status: 0,
            });
        })
        .is_some();
        stores.put(stack, ok);
    }

    /// The listener loop's per-turn yield — same browser contract as
    /// `n_kernel_client_frame` (a browser listener doesn't exist today, but
    /// the shared lib source calls it; honest yield either way).
    pub fn n_kernel_frame(stores: &mut Stores, _stack: &mut DbRef) {
        stores.frame_yield = true;
    }

    /// `kernel_local(tick_interval_us) -> boolean` — the transportless client
    /// kernel in the browser: a windowed (canvas) host with no server.  No
    /// WebSocket (`ws: -1`, guarded in pump/send); `opened` so nothing waits
    /// on a handshake that never comes.
    pub fn n_kernel_local(stores: &mut Stores, stack: &mut DbRef) {
        let tick_us = stores.get::<i64>(stack);
        CLIENT.with(|c| {
            *c.borrow_mut() = Some(BrowserClient {
                ws: -1,
                alive: true,
                opened: true,
                outbox: Vec::new(),
                events: VecDeque::new(),
                last: Event {
                    cid: -1,
                    kind: -1,
                    payload: String::new(),
                    status: 0,
                },
                slots: Vec::new(),
                last_sync: (-1, String::new()),
                out_seq: 0,
                tick_interval_us: tick_us.max(1),
                last_tick_us: 0,
            });
        });
        stores.put(stack, true);
    }

    /// `kernel_connect(host, port, tick_interval_us) -> boolean` — open the
    /// browser WebSocket (async; the pump completes the handshake contract).
    pub fn n_kernel_connect(stores: &mut Stores, stack: &mut DbRef) {
        let tick_us = stores.get::<i64>(stack);
        let port = stores.get::<i64>(stack);
        let host = stores.get::<Str>(stack).str().to_owned();
        let ws = host_ws_connect(&format!("ws://{host}:{port}/ws"));
        let ok = ws >= 0;
        if ok {
            CLIENT.with(|c| {
                *c.borrow_mut() = Some(BrowserClient {
                    ws,
                    alive: true,
                    opened: false,
                    outbox: Vec::new(),
                    events: VecDeque::new(),
                    last: Event {
                        cid: -1,
                        kind: -1,
                        payload: String::new(),
                        status: 0,
                    },
                    slots: Vec::new(),
                    last_sync: (-1, String::new()),
                    out_seq: 0,
                    tick_interval_us: tick_us.max(1),
                    last_tick_us: 0,
                });
            });
        }
        stores.put(stack, ok);
    }

    /// `kernel_client_pump() -> integer` — drain browser-queued frames into
    /// the SAME machinery the native connector uses; fire kind-0 + flush the
    /// outbox once the socket opens.
    pub fn n_kernel_client_pump(stores: &mut Stores, stack: &mut DbRef) {
        let n = with_client(|c| {
            let mut added = 0i64;
            if !c.opened && host_ws_ready(c.ws) {
                c.opened = true;
                c.events.push_back(Event {
                    cid: 0,
                    kind: 0,
                    payload: String::new(),
                    status: 0,
                });
                added += 1;
                for msg in std::mem::take(&mut c.outbox) {
                    let _ = host_ws_send(c.ws, &msg, false);
                }
            }
            while c.alive && c.ws >= 0 && host_ws_recv(c.ws) == 1 {
                let payload = host_ws_last_message();
                // Keyframes ride `S:`-framed reliable frames for BOUND peers
                // only — a browser is never bound, but parse defensively.
                if let Some(rest) = payload.strip_prefix("S:")
                    && let Some((seq_s, body)) = rest.split_once(':')
                    && let Ok(seq) = seq_s.parse::<i64>()
                {
                    conflate_slot(&mut c.slots, seq, body);
                    continue;
                }
                if is_sync_msg(&payload) {
                    conflate_ws(&mut c.slots, &payload);
                    continue;
                }
                c.events.push_back(Event {
                    cid: 0,
                    kind: 1,
                    payload,
                    status: 0,
                });
                added += 1;
            }
            added
        })
        .unwrap_or(0);
        stores.put(stack, n);
    }

    pub fn n_kernel_client_alive(stores: &mut Stores, stack: &mut DbRef) {
        let v = with_client(|c| c.alive).unwrap_or(false);
        stores.put(stack, v);
    }

    /// `client_stop()` — mirror of the native connector's exit surface.
    pub fn n_kernel_client_stop(_stores: &mut Stores, _stack: &mut DbRef) {
        let _ = with_client(|c| c.alive = false);
    }

    pub fn n_kernel_client_next_event(stores: &mut Stores, stack: &mut DbRef) {
        let got = with_client(|c| match c.events.pop_front() {
            Some(ev) => {
                c.last = ev;
                true
            }
            None => false,
        })
        .unwrap_or(false);
        stores.put(stack, got);
    }

    pub fn n_kernel_client_event_kind(stores: &mut Stores, stack: &mut DbRef) {
        let v = with_client(|c| c.last.kind).unwrap_or(-1);
        stores.put(stack, v);
    }

    /// Status of the last event (kind-3 http completions carry it; `0` otherwise).
    pub fn n_kernel_client_event_status(stores: &mut Stores, stack: &mut DbRef) {
        let v = with_client(|c| c.last.status).unwrap_or(0);
        stores.put(stack, v);
    }

    /// Origin of the last event: `0` = the server, `-1` = a local `post`.
    pub fn n_kernel_client_event_cid(stores: &mut Stores, stack: &mut DbRef) {
        let v = with_client(|c| c.last.cid).unwrap_or(-1);
        stores.put(stack, v);
    }

    pub fn n_kernel_client_event_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
        let dest = stores.get::<DbRef>(stack);
        let v = with_client(|c| c.last.payload.clone()).unwrap_or_default();
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(&v);
    }

    /// Drift-free tick on the bridge clock (µs via `time_ticks`).
    pub fn n_kernel_client_tick_due(stores: &mut Stores, stack: &mut DbRef) {
        let due = with_client(|c| {
            let now = host_time_ticks();
            if now - c.last_tick_us >= c.tick_interval_us {
                if c.last_tick_us == 0 {
                    c.last_tick_us = now;
                } else {
                    c.last_tick_us += c.tick_interval_us;
                }
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
        stores.put(stack, due);
    }

    /// `kernel_client_idle(max_us)` — a no-op in the browser: the event loop
    /// IS the idle (the per-turn frame yield returns control to it).
    pub fn n_kernel_client_idle(stores: &mut Stores, stack: &mut DbRef) {
        let _ = stores.get::<i64>(stack);
    }

    /// `client_send(msg)` — everything rides the WebSocket (a browser cannot
    /// UDP); sync sends still count the shared seq space (carrier symmetry).
    pub fn n_kernel_client_send(stores: &mut Stores, stack: &mut DbRef) {
        let msg = stores.get::<Str>(stack).str().to_owned();
        let sync = is_sync_msg(&msg);
        let ok = with_client(|c| {
            if !c.alive {
                return false;
            }
            if sync {
                c.out_seq += 1;
            }
            if c.ws < 0 {
                return false; // local kernel: there is no peer to send to
            }
            if !c.opened {
                c.outbox.push(msg.clone());
                return true; // queued; flushed on open (ordered carrier)
            }
            host_ws_send(c.ws, &msg, false) == 1
        })
        .unwrap_or(false);
        stores.put(stack, ok);
    }

    pub fn n_kernel_client_sync_next(stores: &mut Stores, stack: &mut DbRef) {
        let got = with_client(|c| {
            for slot in &mut c.slots {
                if slot.dirty {
                    slot.dirty = false;
                    c.last_sync = (slot.seq, slot.payload.clone());
                    return true;
                }
            }
            false
        })
        .unwrap_or(false);
        stores.put(stack, got);
    }

    pub fn n_kernel_client_sync_seq(stores: &mut Stores, stack: &mut DbRef) {
        let v = with_client(|c| c.last_sync.0).unwrap_or(-1);
        stores.put(stack, v);
    }

    pub fn n_kernel_client_sync_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
        let dest = stores.get::<DbRef>(stack);
        let v = with_client(|c| c.last_sync.1.clone()).unwrap_or_default();
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(&v);
    }

    /// A browser never has a UDP path — the honest stub.
    pub fn n_kernel_client_udp_bound(stores: &mut Stores, stack: &mut DbRef) {
        stores.put(stack, false);
    }

    /// The browser's per-turn yield: hand the tab back to the event loop;
    /// the host resumes the session next animation frame.
    pub fn n_kernel_client_frame(stores: &mut Stores, _stack: &mut DbRef) {
        stores.frame_yield = true;
    }

    /// `default_host()` in the browser = the cabinet that served the page.
    pub fn n_kernel_default_host_dest(stores: &mut Stores, stack: &mut DbRef) {
        let dest = stores.get::<DbRef>(stack);
        let v = host_origin_host();
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(&v);
    }
}

// ── Typed twins for `--native` codegen (@PLN18 08 scenario S1) ──────────────
//
// A compiled loft program calls natives by NAME with TYPED Rust signatures
// (the codegen_runtime convention: `cell: &UnsafeCell<Stores>`, `i64`
// scalars, `&str` text args, `String` text returns, `u8` booleans).  These
// twins share the kernel internals with the bytecode-stack natives above —
// one implementation, two calling conventions; the queue machinery never
// forks.  Re-exported by `codegen_runtime` (glob-imported into generated
// crates) and registered in `CODEGEN_RUNTIME_FNS`.
#[cfg(not(target_arch = "wasm32"))]
#[allow(non_snake_case, clippy::missing_panics_doc)]
pub mod typed {
    #[allow(clippy::wildcard_imports)] // the twins mirror the whole module
    use super::*;
    use std::cell::UnsafeCell;

    // ── Listener role ──
    pub fn n_kernel_listen(_cell: &UnsafeCell<Stores>, port: i64, tick_us: i64) -> u8 {
        u8::from(listen_impl(port, tick_us))
    }
    pub fn n_kernel_pump(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(pump_kernel).unwrap_or(0)
    }
    pub fn n_kernel_next_event(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(
            with_kernel(|k| match k.events.pop_front() {
                Some(ev) => {
                    k.last = ev;
                    true
                }
                None => false,
            })
            .unwrap_or(false),
        )
    }
    pub fn n_kernel_event_cid(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last.cid).unwrap_or(-1)
    }
    pub fn n_kernel_event_kind(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last.kind).unwrap_or(-1)
    }
    pub fn n_kernel_event_payload(_cell: &UnsafeCell<Stores>) -> String {
        with_kernel(|k| k.last.payload.clone()).unwrap_or_default()
    }
    pub fn n_kernel_event_status(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last.status).unwrap_or(0)
    }
    pub fn n_kernel_client_event_status(_cell: &UnsafeCell<Stores>) -> i64 {
        with_client(|c| c.last.status).unwrap_or(0)
    }
    pub fn n_kernel_http_fetch(
        _cell: &UnsafeCell<Stores>,
        method: &str,
        url: &str,
        body: &str,
        headers: &str,
    ) -> i64 {
        http_fetch_impl(
            method.to_owned(),
            url.to_owned(),
            body.to_owned(),
            headers.to_owned(),
        )
    }
    pub fn n_kernel_tick_due(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_kernel(tick_due_kernel).unwrap_or(false))
    }
    pub fn n_send(_cell: &UnsafeCell<Stores>, cid: i64, msg: &str) -> u8 {
        let sync = is_sync_msg(msg);
        u8::from(with_kernel(|k| deliver(k, cid as usize, msg, sync)).unwrap_or(false))
    }
    pub fn n_broadcast(_cell: &UnsafeCell<Stores>, msg: &str) -> i64 {
        let sync = is_sync_msg(msg);
        with_kernel(|k| {
            let mut sent = 0i64;
            for cid in 0..k.conns.len() {
                if k.conns[cid].is_none() {
                    continue;
                }
                if deliver(k, cid, msg, sync) {
                    sent += 1;
                }
            }
            sent
        })
        .unwrap_or(0)
    }
    pub fn n_kernel_idle(_cell: &UnsafeCell<Stores>, max_us: i64) {
        let sleep_us = with_kernel(|k| {
            let now = k.start.elapsed().as_micros() as i64;
            let until_tick = if k.last_tick_us == 0 {
                k.tick_interval_us
            } else {
                (k.last_tick_us + k.tick_interval_us - now).max(0)
            };
            max_us.clamp(0, until_tick.max(1))
        })
        .unwrap_or(max_us.max(0));
        std::thread::sleep(Duration::from_micros(sleep_us as u64));
    }
    pub fn n_clients(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.conns.iter().filter(|c| c.is_some()).count() as i64).unwrap_or(0)
    }
    pub fn n_udp_bound(_cell: &UnsafeCell<Stores>, cid: i64) -> u8 {
        u8::from(
            with_kernel(|k| k.net.get(cid as usize).is_some_and(|n| n.path.is_some()))
                .unwrap_or(false),
        )
    }
    pub fn n_sync_class(_cell: &UnsafeCell<Stores>, msg_id: i64) {
        SYNC_IDS.with(|s| {
            s.borrow_mut().insert(msg_id, false);
        });
    }
    pub fn n_sync_class_keyed(_cell: &UnsafeCell<Stores>, msg_id: i64) {
        SYNC_IDS.with(|s| {
            s.borrow_mut().insert(msg_id, true);
        });
    }
    pub fn n_keyframe(_cell: &UnsafeCell<Stores>, cid: i64, msg: &str) -> u8 {
        u8::from(with_kernel(|k| deliver_keyframe(k, cid as usize, msg)).unwrap_or(false))
    }
    pub fn n_sync_next(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_kernel(sync_next_kernel).unwrap_or(false))
    }
    pub fn n_sync_cid(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last_sync.0).unwrap_or(-1)
    }
    pub fn n_sync_seq(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last_sync.1).unwrap_or(-1)
    }
    pub fn n_kernel_sync_payload(_cell: &UnsafeCell<Stores>) -> String {
        with_kernel(|k| k.last_sync.2.clone()).unwrap_or_default()
    }
    pub fn n_kernel_client_frame(_cell: &UnsafeCell<Stores>) {}
    /// @PLN18 08-S5 — typed twins of the swap natives.
    pub fn n_swap_world(cell: &UnsafeCell<Stores>, w: DbRef) -> u8 {
        let stores: &mut Stores = unsafe { &mut *cell.get() };
        u8::from(super::swap_world_impl(stores, w))
    }

    pub fn n_swap_start(_cell: &UnsafeCell<Stores>, artifact: &str) -> u8 {
        u8::from(super::swap_start_impl(artifact))
    }

    pub fn n_kernel_swap_step(cell: &UnsafeCell<Stores>) -> i64 {
        let stores: &mut Stores = unsafe { &mut *cell.get() };
        super::swap_step_impl(stores)
    }

    pub fn n_swap_retired(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(super::SWAP.with(|sw| matches!(*sw.borrow(), super::SwapPhase::Done)))
    }

    pub fn n_kernel_default_host(_cell: &UnsafeCell<Stores>) -> String {
        std::env::var("LOFT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
    }

    // ── Connector role ──
    pub fn n_kernel_connect(_cell: &UnsafeCell<Stores>, host: &str, port: i64, tick_us: i64) -> u8 {
        u8::from(client_connect(host, port as u16, tick_us).is_some())
    }
    pub fn n_kernel_local(_cell: &UnsafeCell<Stores>, tick_us: i64) -> u8 {
        local_init(tick_us);
        1
    }
    pub fn n_post(_cell: &UnsafeCell<Stores>, msg: &str) -> u8 {
        u8::from(post_impl(msg))
    }
    pub fn n_kernel_alive(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_kernel(|k| k.alive).unwrap_or(false))
    }

    // @PLN119 arc F — the four the loft surface now reaches through a WRAPPER.
    //
    // A library function that already carries a native symbol cannot be PLACED
    // (placement works by giving one), so `send` / `broadcast` / `clients` /
    // `post` became loft wrappers over private `kernel_*` natives. The native
    // backend resolves a runtime fn by the loft DEF name, so the rename moved
    // them out of its table — these are the same implementations under the names
    // it now looks for.
    pub fn n_kernel_send(cell: &UnsafeCell<Stores>, cid: i64, msg: &str) -> u8 {
        n_send(cell, cid, msg)
    }
    pub fn n_kernel_broadcast(cell: &UnsafeCell<Stores>, msg: &str) -> i64 {
        n_broadcast(cell, msg)
    }
    pub fn n_kernel_clients(cell: &UnsafeCell<Stores>) -> i64 {
        n_clients(cell)
    }
    pub fn n_kernel_post(cell: &UnsafeCell<Stores>, msg: &str) -> u8 {
        n_post(cell, msg)
    }
    pub fn n_kernel_stop(_cell: &UnsafeCell<Stores>) {
        let _ = with_kernel(|k| k.alive = false);
    }
    pub fn n_kernel_frame(_cell: &UnsafeCell<Stores>) {}
    pub fn n_kernel_client_pump(_cell: &UnsafeCell<Stores>) -> i64 {
        with_client(pump_client).unwrap_or(0)
    }
    pub fn n_kernel_client_stop(_cell: &UnsafeCell<Stores>) {
        let _ = super::with_client(|c| c.alive = false);
    }

    pub fn n_kernel_client_event_cid(_cell: &UnsafeCell<Stores>) -> i64 {
        with_client(|c| c.last.cid).unwrap_or(-1)
    }
    pub fn n_kernel_client_alive(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_client(|c| c.alive).unwrap_or(false))
    }
    pub fn n_kernel_client_next_event(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(
            with_client(|c| match c.events.pop_front() {
                Some(ev) => {
                    c.last = ev;
                    true
                }
                None => false,
            })
            .unwrap_or(false),
        )
    }
    pub fn n_kernel_client_event_kind(_cell: &UnsafeCell<Stores>) -> i64 {
        with_client(|c| c.last.kind).unwrap_or(-1)
    }
    pub fn n_kernel_client_event_payload(_cell: &UnsafeCell<Stores>) -> String {
        with_client(|c| c.last.payload.clone()).unwrap_or_default()
    }
    pub fn n_kernel_client_tick_due(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_client(tick_due_client).unwrap_or(false))
    }
    pub fn n_kernel_client_idle(_cell: &UnsafeCell<Stores>, max_us: i64) {
        let sleep_us = with_client(|c| {
            let now = c.now_us();
            let until_tick = if c.last_tick_us == 0 {
                c.tick_interval_us
            } else {
                (c.last_tick_us + c.tick_interval_us - now).max(0)
            };
            max_us.clamp(0, until_tick.max(1))
        })
        .unwrap_or(max_us.max(0));
        std::thread::sleep(Duration::from_micros(sleep_us as u64));
    }
    pub fn n_client_send(_cell: &UnsafeCell<Stores>, msg: &str) -> u8 {
        let sync = is_sync_msg(msg);
        u8::from(with_client(|c| client_send_impl(c, msg, sync)).unwrap_or(false))
    }
    pub fn n_client_sync_next(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_client(sync_next_client).unwrap_or(false))
    }
    pub fn n_client_sync_seq(_cell: &UnsafeCell<Stores>) -> i64 {
        with_client(|c| c.last_sync.0).unwrap_or(-1)
    }
    pub fn n_kernel_client_sync_payload(_cell: &UnsafeCell<Stores>) -> String {
        with_client(|c| c.last_sync.1.clone()).unwrap_or_default()
    }
    pub fn n_client_udp_bound(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_client(|c| c.udp_bound).unwrap_or(false))
    }
}
