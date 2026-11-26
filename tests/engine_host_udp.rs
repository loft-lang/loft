// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 phase 05a — the state-sync UDP channel end-to-end:
//!
//! 1. the kernel issues a per-connection session key and carries it, with the
//!    cid, on the WS 101 upgrade response (`X-Loft-UDP`) — negotiation is
//!    kernel-internal, no loft code on either side touches it;
//! 2. before the hello, `sync_send` falls back to WS frames;
//! 3. a hello sealed under that key binds the source addr (acked), after which
//!    the SAME `sync_send` call rides UDP;
//! 4. inbound datagrams conflate to newest per sender — a burst inside one
//!    tick yields exactly the newest payload, and a stale seq is discarded;
//! 5. a silent path times out and sends revert to WS transparently.

use loft::file_access as fa;
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;
#[path = "common/udp_wire.rs"]
mod udp_wire;

const PORT_BASE: u16 = 18088;
/// Long ticks make the conflation window deterministic.
const TICK_US: u32 = 200_000;

fn loft_bin() -> PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

struct Guard(Option<Child>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Connect + upgrade; returns the stream and the `X-Loft-UDP` value from the
/// 101 response head (the kernel-internal transport negotiation channel — no
/// loft code on either side ever touches it).
/// A port the OS has just confirmed free on **both** TCP and UDP.
///
/// The engine-host kernel listens on both for the same number, so a TCP-only probe
/// would hand back a port whose UDP half is taken and the fast-lane assertions would
/// fail for a reason that has nothing to do with the code under test.
///
/// Candidates come from 20000–29999 keyed on the pid, NOT from `bind(":0")`.
/// Binding port 0 draws from the OS ephemeral range (32768–60999 on Linux), which is
/// the same pool every other test's port probe draws from — so asking for one here
/// would trade a collision with a *well-known* port for a collision with a *sibling
/// test*, which is harder to recognise when it bites. A pid-keyed number in a quiet
/// range separates concurrent checkouts and stays out of that pool.
///
/// `SO_REUSEADDR` is deliberately not set: a port that only appears free because of
/// address reuse is not free.
fn free_port() -> u16 {
    use std::net::TcpListener;
    let base = 20000u16 + u16::try_from(std::process::id() % 10000).unwrap_or(0);
    for step in 0..200u16 {
        let port = 20000 + (base - 20000 + step) % 10000;
        // BOTH transports must be free for the SAME number: the kernel listens on
        // TCP and UDP alike, and a TCP-only probe would hand back a port whose UDP
        // half is taken — the fast-lane assertions would then fail for a reason that
        // has nothing to do with the code under test.
        let Ok(tcp) = TcpListener::bind(("127.0.0.1", port)) else {
            continue;
        };
        let udp_free = UdpSocket::bind(("127.0.0.1", port)).is_ok();
        drop(tcp);
        if udp_free {
            return port;
        }
    }
    panic!("no port free on both TCP and UDP in 20000–29999 after 200 attempts");
}

fn ws_connect(port: u16) -> (TcpStream, String) {
    // A `--native-c` kernel compiles before it listens.
    let deadline = Instant::now() + Duration::from_secs(90);
    let stream = loop {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        assert!(Instant::now() < deadline, "kernel never listened on {port}");
        std::thread::sleep(Duration::from_millis(100));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let req = "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
               Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
               Sec-WebSocket-Version: 13\r\n\r\n";
    (&stream).write_all(req.as_bytes()).unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    assert!(head.contains("101"));
    let offer = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("x-loft-udp")
                .then(|| v.trim().to_string())
        })
        .unwrap_or_default();
    (stream, offer)
}

fn ws_recv(stream: &TcpStream) -> String {
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).unwrap();
    let len = match hdr[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).unwrap();
            u16::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).unwrap();
    String::from_utf8(payload).unwrap()
}

/// `ws_recv` that reports a closed or failed stream as `None` instead of panicking.
fn ws_recv_opt(stream: &TcpStream) -> Option<String> {
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).ok()?;
    let len = match hdr[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).ok()?;
            u16::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).ok()?;
    String::from_utf8(payload).ok()
}

fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [9u8, 8, 7, 6];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8, 0x80 | bytes.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

/// Open a server datagram that must be a sync sample: its `(seq, payload)`.
fn unstamp(seat: &Seat, dgram: &[u8]) -> (i64, String) {
    match open(seat, dgram) {
        Some(('S', seq, payload)) => (seq, payload),
        other => panic!("a sealed sync datagram, got {other:?}"),
    }
}

// @speed 6.7
#[test]
fn udp_sync_channel_end_to_end() {
    let port = common::bind_port(PORT_BASE);
    if !fa::exists(loft_bin()) {
        eprintln!("skipping: release loft not built");
        return;
    }
    let prog = std::env::temp_dir().join(format!("eh_udp_{}.loft", std::process::id()));
    fa::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  // Wire-schema table: 8 = tick beacon, 9 = state echo — latest-value kinds.
  engine_host::sync_class(8);
  engine_host::sync_class(9);
  w = W {{ tick: 0 }};
  engine_host::run({port}, {TICK_US},
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{
        engine_host::send(ev.cid, "hi:{{ev.cid}}");
      }} else if ev.kind == 1 {{
        // ONE receive surface (05d): the client's conflated state arrives
        // here at tick time — echo it back on the declared sync kind.
        engine_host::send(ev.cid, "9:{{ev.payload}}");
      }}
    }},
    fn() {{
      w.tick = w.tick + 1;
      engine_host::send(0, "8:{{w.tick}}");
    }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let child = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));

    // 1. Connect; the cid and session key ride the 101 upgrade response
    //    (X-Loft-UDP) — transport negotiation is kernel-internal, the loft
    //    program above never references it.
    let (ws, offer) = ws_connect(port);
    let seat = seat_of(&offer);
    assert_eq!(seat.cid, 0, "the first connection's cid");
    let hello_frame = ws_recv(&ws);
    assert_eq!(hello_frame, "hi:0", "ordinary meaning traffic untouched");

    // 2. Pre-hello: sync_send falls back to WS — beacons arrive as plain
    //    WS text frames (no S: stamp).
    let b = ws_recv(&ws);
    assert!(b.starts_with("8:"), "WS fallback beacon, got {b:?}");

    // 3. Hello: bind the UDP path; the kernel acks.  Datagrams can drop even
    //    on loopback under load — retry the hello until the ack.
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.connect(("127.0.0.1", port)).unwrap();
    udp.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut first_udp_beacon = None;
    loop {
        udp.send(&hello(&seat)).unwrap();
        match udp_recv_raw(&udp) {
            Some(d) if open(&seat, &d).is_some_and(|(k, _, _)| k == 'A') => break,
            Some(d) if open(&seat, &d).is_some_and(|(k, _, _)| k == 'S') => {
                // Already bound on an earlier hello; the ack raced a beacon.
                first_udp_beacon = Some(d);
                break;
            }
            _ => assert!(Instant::now() < deadline, "hello never acked"),
        }
    }

    // 4. Post-hello: the SAME sync_send now rides UDP — beacons arrive as
    //    seq-stamped datagrams.
    let dgram = match first_udp_beacon {
        Some(d) => d,
        None => loop {
            match udp_recv_raw(&udp) {
                Some(d) => break d,
                None => assert!(Instant::now() < deadline, "no UDP beacon"),
            }
        },
    };
    let (seq, payload) = unstamp(&seat, &dgram);
    assert!(seq >= 1, "outbound seq stamped: {payload:?}");
    assert!(payload.starts_with("8:"), "UDP beacon: {payload:?}");

    // 5. Conflation: a same-tick burst (110, then 112, then stale 111 —
    //    reordered, not replayed) must yield exactly ONE echo — the newest
    //    (112).  Sync right after a beacon so the burst lands inside one
    //    200 ms tick window.
    let _ = udp_recv_raw(&udp); // align roughly to a tick boundary
    udp.send(&sync(&seat, 110, "posA")).unwrap();
    udp.send(&sync(&seat, 112, "posC")).unwrap();
    udp.send(&sync(&seat, 111, "posB")).unwrap();
    let mut echoes = Vec::new();
    let until = Instant::now() + Duration::from_millis(600); // ~3 ticks
    while Instant::now() < until {
        if let Some(d) = udp_recv_raw(&udp) {
            let (_, p) = unstamp(&seat, &d);
            if let Some(e) = p.strip_prefix("9:") {
                echoes.push(e.to_string());
            }
        }
    }
    assert_eq!(
        echoes,
        vec!["posC".to_string()],
        "conflate-to-newest: one echo, the highest seq"
    );

    // 6. Stale discard: seq 105 < 112 is never applied — no echo follows.
    udp.send(&sync(&seat, 105, "staleZ")).unwrap();
    let until = Instant::now() + Duration::from_millis(600);
    while Instant::now() < until {
        if let Some(d) = udp_recv_raw(&udp) {
            let (_, p) = unstamp(&seat, &d);
            assert!(
                !p.contains("staleZ"),
                "stale seq must be discarded, got {p:?}"
            );
        }
    }

    // 7. Timeout: stop sending datagrams; after UDP_TIMEOUT_US (3 s) the path
    //    unbinds and beacons revert to WS.  Skip the WS frames queued from
    //    step 2's era by requiring a beacon FRESHER than the last UDP one.
    let mut last_udp_tick = 0i64;
    let until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < until {
        if let Some(d) = udp_recv_raw(&udp) {
            let (_, p) = unstamp(&seat, &d);
            if let Some(t) = p.strip_prefix("8:") {
                last_udp_tick = t.parse().unwrap_or(0);
            }
        }
    }
    ws.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let f = ws_recv(&ws);
        if let Some(t) = f.strip_prefix("8:")
            && t.parse::<i64>().unwrap_or(0) > last_udp_tick
        {
            break; // fresh beacon on WS: the fallback re-engaged
        }
        assert!(Instant::now() < deadline, "beacons never reverted to WS");
    }

    let _ = fa::remove_file(&prog);
}

/// The auto-path proof on the REAL consumer — `probe_server_kernel.loft`
/// (the @PLAN50 pose server, poses ported onto `sync_send`): ONE server, one
/// call site.  Client A is a plain web-page-style WS client (cannot UDP,
/// never hellos) and receives poses as ordinary WS frames; client B is a
/// native-style client that hellos with the 101-header session key and receives
/// the SAME world's poses as seq-stamped datagrams.  The server program
/// contains zero transport logic — the kernel picks the fastest path per
/// client.
#[test]
fn probe_server_poses_ride_the_fastest_path_per_client() {
    if !fa::exists(loft_bin()) {
        eprintln!("skipping: release loft not built");
        return;
    }
    // Ask the OS for a port instead of naming one.  This used to be a hardcoded
    // 18084 to match the fixture's own constant, and on a shared machine that is a
    // well-known number: long-lived `planet_server` / `loft_native_bin` processes
    // from other checkouts sit on it, and this test then failed for someone else's
    // run.  The fixture now takes `LOFT_PROBE_PORT`, so the number can come from
    // `free_port()` — which checks TCP *and* UDP, because the kernel binds both.
    let port = free_port();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let child = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(root.join("tools/audience-demo-50/probe_server_kernel.loft"))
        .env("LOFT_PROBE_PORT", port.to_string())
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn probe server");
    let _guard = Guard(Some(child));

    // A = web-page tier (cid 0), B = native tier (cid 1); both in sight range.
    let (ws_a, _offer_a) = ws_connect(port);
    let (ws_b, offer_b) = ws_connect(port);
    let seat_b = seat_of(&offer_b);
    assert_eq!(seat_b.cid, 1, "the second connection's cid");
    ws_send(&ws_a, "1:0,0,0,0");
    ws_send(&ws_b, "1:1,10,0,0");

    // B hellos — earning the UDP fast path with the kernel-negotiated key.
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.connect(("127.0.0.1", port)).unwrap();
    udp.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut b_pose: Option<Vec<u8>> = None;
    loop {
        udp.send(&hello(&seat_b)).unwrap();
        match udp_recv_raw(&udp) {
            Some(d) if open(&seat_b, &d).is_some_and(|(k, _, _)| k == 'A') => break,
            Some(d) if open(&seat_b, &d).is_some_and(|(k, _, _)| k == 'S') => {
                b_pose = Some(d); // bound on an earlier hello; a pose raced the ack
                break;
            }
            _ => assert!(Instant::now() < deadline, "hello never acked"),
        }
    }

    // A (fallback): poses arrive as plain WS frames — B's plane is id 1.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let f = ws_recv(&ws_a);
        if f.starts_with("2:1,") {
            break; // WS pose of B's plane
        }
        assert!(
            Instant::now() < deadline,
            "A never saw a WS pose, got {f:?}"
        );
    }

    // B (fast path): the same world's poses arrive as seq-stamped datagrams —
    // A's plane is id 0.
    let dgram = loop {
        match b_pose.take().or_else(|| udp_recv_raw(&udp)) {
            Some(d) => {
                let (_, payload) = unstamp(&seat_b, &d);
                if payload.starts_with("2:0,") {
                    break d;
                }
            }
            None => assert!(Instant::now() < deadline, "B never saw a UDP pose"),
        }
    };
    let (seq, _) = unstamp(&seat_b, &dgram);
    assert!(seq >= 1, "outbound seq stamped: {seq}");
}

// ── A datagram's sender: only a holder of the connection's session counts ──────────────────
//
// The three tests below drive the echo kernel of `udp_sync_channel_end_to_end` and send, from
// the client's own socket (the source address a forger copies) or from a second one, datagrams
// nobody holding the session sent.  The wire helpers are the only part that knows the format.

/// The echo kernel: tick beacons (`8:<n>`) and every conflated client sample echoed as `9:…`.
fn start_echo_kernel(port: u16, tag: &str, backend: &str) -> (Guard, PathBuf) {
    let prog = std::env::temp_dir().join(format!(
        "eh_udp_{tag}_{}_{}.loft",
        &backend[2..],
        std::process::id()
    ));
    fa::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  engine_host::sync_class(8);
  engine_host::sync_class(9);
  w = W {{ tick: 0 }};
  engine_host::run({port}, {TICK_US},
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{
        engine_host::send(ev.cid, "hi:{{ev.cid}}");
      }} else if ev.kind == 1 {{
        engine_host::send(ev.cid, "9:{{ev.payload}}");
      }}
    }},
    fn() {{
      w.tick = w.tick + 1;
      engine_host::send(0, "8:{{w.tick}}");
    }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let child = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .env("LOFT_TIMEOUT", "120")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn kernel");
    (Guard(Some(child)), prog)
}

/// The backends a cell runs on, each with its own port base: the interpreter (the Rust kernel)
/// and, where a C compiler and sockets are at hand, the C runtime's twin (`loft_engine.h`).
fn backends(base: u16) -> Vec<(&'static str, u16)> {
    let mut v = vec![("--interpret", base)];
    let have_cc = loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if have_cc && loft::platform::c_runtime_has_sockets() {
        v.push(("--native-c", base + 20));
    }
    v
}

use udp_wire::Seat;

fn seat_of(header: &str) -> Seat {
    Seat::of(header)
}

fn hello(seat: &Seat) -> Vec<u8> {
    seat.hello()
}

fn keepalive(seat: &Seat) -> Vec<u8> {
    seat.keepalive()
}

fn sync(seat: &Seat, seq: u64, payload: &str) -> Vec<u8> {
    seat.sync(seq, payload)
}

fn forged(seat: &Seat, kind: u8, seq: u64, payload: &str) -> Vec<Vec<u8>> {
    seat.forged(kind, seq, payload)
}

fn open(seat: &Seat, d: &[u8]) -> Option<(char, i64, String)> {
    seat.open(d)
}

fn udp_recv_raw(sock: &UdpSocket) -> Option<Vec<u8>> {
    let mut buf = [0u8; 2048];
    sock.recv(&mut buf).ok().map(|n| buf[..n].to_vec())
}

/// Connect, read the greeting, and bind a UDP path: the socket, the seat and the hello sent.
fn seated(port: u16) -> (TcpStream, UdpSocket, Seat, Vec<u8>) {
    let (ws, header) = ws_connect(port);
    let seat = seat_of(&header);
    assert_eq!(ws_recv(&ws), "hi:0");
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.connect(("127.0.0.1", port)).unwrap();
    udp.set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let h = hello(&seat);
        udp.send(&h).unwrap();
        if let Some(d) = udp_recv_raw(&udp)
            && let Some((kind, _, _)) = open(&seat, &d)
            && (kind == 'A' || kind == 'S')
        {
            return (ws, udp, seat, h);
        }
        assert!(Instant::now() < deadline, "hello never acked");
    }
}

/// The echoed (`9:`) payloads that arrive on `udp` within `window`.
fn echoes_within(udp: &UdpSocket, seat: &Seat, window: Duration) -> Vec<String> {
    let mut out = Vec::new();
    let until = Instant::now() + window;
    while Instant::now() < until {
        if let Some(d) = udp_recv_raw(udp)
            && let Some(('S', _, p)) = open(seat, &d)
            && let Some(e) = p.strip_prefix("9:")
        {
            out.push(e.to_string());
        }
    }
    out
}

/// A sample claiming the highest seq there is, sent from the client's own address by someone
/// without the session key (under another key, reflected, untagged), must not be applied —
/// and must not make the next genuine sample stale.
///
/// @falsified-at: 0c122ca5a — red on the tree before the seal (the forged sample was echoed);
/// by hand on d64be8dd2 (the tag check in `seal::open` off: red) and on 60acdddde (the C
/// `loft_eh_open` tag check off: red on the `--native-c` leg).
// @speed 5
#[test]
fn a_forged_max_seq_sample_freezes_no_slot() {
    for (backend, base) in backends(18160) {
        a_forged_max_seq_sample_freezes_no_slot_on(backend, common::bind_port(base));
    }
}

fn a_forged_max_seq_sample_freezes_no_slot_on(backend: &str, port: u16) {
    let (_guard, prog) = start_echo_kernel(port, "freeze", backend);
    let (_ws, udp, seat, _) = seated(port);
    udp.send(&sync(&seat, 100, "alive")).unwrap();
    assert!(
        echoes_within(&udp, &seat, Duration::from_millis(800)).contains(&"alive".to_string()),
        "{backend}: the genuine sample is echoed"
    );
    for d in forged(&seat, b'S', i64::MAX as u64, "frozen") {
        udp.send(&d).unwrap();
    }
    udp.send(&sync(&seat, 101, "moved")).unwrap();
    let echoes = echoes_within(&udp, &seat, Duration::from_millis(800));
    assert!(
        !echoes.contains(&"frozen".to_string()),
        "{backend}: a forged sample was applied: {echoes:?}"
    );
    assert!(
        echoes.contains(&"moved".to_string()),
        "{backend}: the genuine sample after a forged one was dropped: {echoes:?}"
    );
    let _ = fa::remove_file(&prog);
}

/// A hello seen on the path and sent again from another address takes no seat, and neither
/// does a forged one: the second socket gets no ack and none of the client's state, and the
/// client keeps its path.
///
/// @falsified-at: 0c122ca5a — red on the tree before the seal (the second socket was acked);
/// by hand on 0fc5b13fa (the tag check off: red; `Replay::admit` taking every seq as the newest:
/// red) and on 60acdddde (the C tag check off: red on the `--native-c` leg).
// @speed 5
#[test]
fn a_replayed_hello_from_another_address_takes_no_seat() {
    for (backend, base) in backends(18161) {
        a_replayed_hello_from_another_address_takes_no_seat_on(backend, common::bind_port(base));
    }
}

fn a_replayed_hello_from_another_address_takes_no_seat_on(backend: &str, port: u16) {
    let (_guard, prog) = start_echo_kernel(port, "replay", backend);
    let (_ws, udp, seat, seen_hello) = seated(port);
    let thief = UdpSocket::bind("127.0.0.1:0").unwrap();
    thief.connect(("127.0.0.1", port)).unwrap();
    thief
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    thief.send(&seen_hello).unwrap();
    for d in forged(&seat, b'H', seat.seq.get() + 1000, "") {
        thief.send(&d).unwrap();
    }
    let until = Instant::now() + Duration::from_millis(900);
    while Instant::now() < until {
        if let Some(d) = udp_recv_raw(&thief) {
            panic!(
                "{backend}: the replayed hello took the seat: {:?}",
                String::from_utf8_lossy(&d)
            );
        }
    }
    // The genuine client still holds its path: beacons keep arriving there.
    let mut beacon = false;
    let until = Instant::now() + Duration::from_millis(900);
    while Instant::now() < until && !beacon {
        beacon = udp_recv_raw(&udp)
            .and_then(|d| open(&seat, &d))
            .is_some_and(|(k, _, p)| k == 'S' && p.starts_with("8:"));
    }
    assert!(beacon, "{backend}: the genuine client lost its path");
    let _ = fa::remove_file(&prog);
}

/// The source address is a routing hint: the client's next genuine datagram from a new address
/// (a NAT that rebound its port) moves the path there, and the server's datagrams follow it.
///
/// @falsified-at: 60acdddde — by hand: the path never moving after it binds, in
/// `pump_udp` (on d64be8dd2) and in `loft_ehk_pump_udp`: red on each backend's leg.
// @speed 5
#[test]
fn a_genuine_datagram_from_a_new_address_moves_the_path() {
    for (backend, base) in backends(18163) {
        a_genuine_datagram_from_a_new_address_moves_the_path_on(backend, common::bind_port(base));
    }
}

fn a_genuine_datagram_from_a_new_address_moves_the_path_on(backend: &str, port: u16) {
    let (_guard, prog) = start_echo_kernel(port, "rebind", backend);
    let (_ws, old, seat, _) = seated(port);
    let moved = UdpSocket::bind("127.0.0.1:0").unwrap();
    moved.connect(("127.0.0.1", port)).unwrap();
    moved
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    moved.send(&keepalive(&seat)).unwrap();
    let mut beacon = false;
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until && !beacon {
        beacon = udp_recv_raw(&moved)
            .and_then(|d| open(&seat, &d))
            .is_some_and(|(k, _, p)| k == 'S' && p.starts_with("8:"));
    }
    assert!(
        beacon,
        "{backend}: the path did not follow the client to its new address"
    );
    // The old address hears nothing more once the beacons in flight are drained.
    while udp_recv_raw(&old).is_some() {}
    old.set_read_timeout(Some(Duration::from_millis(600)))
        .unwrap();
    assert!(
        udp_recv_raw(&old).is_none(),
        "{backend}: the old address still gets datagrams"
    );
    let _ = fa::remove_file(&prog);
}

/// Keepalives from the client's address by someone without the session key — forged, and a
/// genuine one seen on the path and sent again — do not hold a quiet path open: once the
/// genuine client goes quiet, sends fall back to the WebSocket on time (`UDP_TIMEOUT_US`, 3 s)
/// while the false keepalives keep coming.
///
/// @falsified-at: 0c122ca5a — red on the tree before the seal (the path never timed out);
/// by hand on d64be8dd2 (the replay window's seen-bit check off: red), on 0fc5b13fa (the tag
/// check off: red; every seq taken as the newest: red) and on 60acdddde (the C tag check and
/// seen-bit check off: red on the `--native-c` leg).
// @speed 12
#[test]
fn forged_keepalives_do_not_hold_a_path_open() {
    for (backend, base) in backends(18162) {
        forged_keepalives_do_not_hold_a_path_open_on(backend, common::bind_port(base));
    }
}

fn forged_keepalives_do_not_hold_a_path_open_on(backend: &str, port: u16) {
    let (_guard, prog) = start_echo_kernel(port, "keep", backend);
    let (ws, udp, seat, _) = seated(port);
    let seen = keepalive(&seat);
    udp.send(&seen).unwrap();
    // The forger picks a new seq each round: under a working seal none opens, so the replay
    // window never sees them and only the tag stands between them and the keepalive.
    let mut forged_seq = seat.seq.get() + 1000;
    udp.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    // The WebSocket is read on its own thread: a frame read cut by a timeout would tear it.
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = ws.try_clone().unwrap();
    reader.set_read_timeout(None).unwrap();
    std::thread::spawn(move || {
        while let Some(f) = ws_recv_opt(&reader) {
            if tx.send(f).is_err() {
                break;
            }
        }
    });
    let mut last_udp_tick = 0i64;
    let until = Instant::now() + Duration::from_secs(6);
    loop {
        assert!(
            Instant::now() < until,
            "{backend}: forged keepalives held the path open (last UDP beacon {last_udp_tick})"
        );
        forged_seq += 1;
        for d in forged(&seat, b'K', forged_seq, "") {
            udp.send(&d).unwrap();
        }
        udp.send(&seen).unwrap();
        if let Some(d) = udp_recv_raw(&udp)
            && let Some(('S', _, p)) = open(&seat, &d)
            && let Some(t) = p.strip_prefix("8:")
        {
            last_udp_tick = t.parse().unwrap_or(0);
        }
        if rx.try_iter().any(|f| {
            f.strip_prefix("8:")
                .and_then(|t| t.parse::<i64>().ok())
                .is_some_and(|t| last_udp_tick > 0 && t > last_udp_tick)
        }) {
            break; // a beacon fresher than the last datagram, on the WebSocket
        }
    }
    let _ = fa::remove_file(&prog);
}
