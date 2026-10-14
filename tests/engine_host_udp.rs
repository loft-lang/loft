// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 phase 05a — the state-sync UDP channel end-to-end:
//!
//! 1. the kernel issues a cookie and carries it on the WS 101 upgrade
//!    response (`X-Loft-UDP`) — negotiation is kernel-internal, no loft
//!    code on either side touches it;
//! 2. before the hello, `sync_send` falls back to WS frames;
//! 3. `H:<cookie>` binds the source addr (acked `A:<cid>`), after which the
//!    SAME `sync_send` call rides UDP;
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

const PORT_BASE: u16 = 18088;
/// Long ticks make the conflation window deterministic.
const TICK_US: u32 = 200_000;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft")
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

/// Connect + upgrade; returns the stream and the `X-Loft-UDP` cookie from the
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
    let deadline = Instant::now() + Duration::from_secs(15);
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
    let cookie = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("x-loft-udp")
                .then(|| v.trim().to_string())
        })
        .unwrap_or_default();
    (stream, cookie)
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

fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [9u8, 8, 7, 6];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8, 0x80 | bytes.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

/// Receive one datagram as text, or `None` on timeout.
fn udp_recv(sock: &UdpSocket) -> Option<String> {
    let mut buf = [0u8; 2048];
    match sock.recv(&mut buf) {
        Ok(n) => Some(String::from_utf8_lossy(&buf[..n]).into_owned()),
        Err(_) => None,
    }
}

/// Strip the kernel's outbound `S:<seq>:` stamp; panics on a malformed frame.
fn unstamp(dgram: &str) -> (i64, String) {
    let rest = dgram.strip_prefix("S:").expect("S: stamp");
    let (seq, payload) = rest.split_once(':').expect("seq separator");
    (seq.parse().expect("numeric seq"), payload.to_string())
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

    // 1. Connect; the cookie rides the 101 upgrade response (X-Loft-UDP) —
    //    transport negotiation is kernel-internal, the loft program above
    //    never references it.
    let (ws, cookie) = ws_connect(port);
    assert_eq!(cookie.len(), 16, "16-hex-char cookie: {cookie:?}");
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
        udp.send(format!("H:{cookie}").as_bytes()).unwrap();
        match udp_recv(&udp) {
            Some(d) if d == "A:0" => break,
            Some(d) if d.starts_with("S:") => {
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
            match udp_recv(&udp) {
                Some(d) => break d,
                None => assert!(Instant::now() < deadline, "no UDP beacon"),
            }
        },
    };
    let (seq, payload) = unstamp(&dgram);
    assert!(seq >= 1, "outbound seq stamped: {dgram:?}");
    assert!(payload.starts_with("8:"), "UDP beacon: {dgram:?}");

    // 5. Conflation: a same-tick burst (10, then 12, then stale 11) must
    //    yield exactly ONE echo — the newest (12).  Sync right after a beacon
    //    so the burst lands inside one 200 ms tick window.
    let _ = udp_recv(&udp); // align roughly to a tick boundary
    udp.send(b"S:10:posA").unwrap();
    udp.send(b"S:12:posC").unwrap();
    udp.send(b"S:11:posB").unwrap();
    let mut echoes = Vec::new();
    let until = Instant::now() + Duration::from_millis(600); // ~3 ticks
    while Instant::now() < until {
        if let Some(d) = udp_recv(&udp) {
            let (_, p) = unstamp(&d);
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

    // 6. Stale discard: seq 5 < 12 is never applied — no echo follows.
    udp.send(b"S:5:staleZ").unwrap();
    let until = Instant::now() + Duration::from_millis(600);
    while Instant::now() < until {
        if let Some(d) = udp_recv(&udp) {
            let (_, p) = unstamp(&d);
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
        if let Some(d) = udp_recv(&udp) {
            let (_, p) = unstamp(&d);
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
/// native-style client that hellos with the 101-header cookie and receives
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
    let (ws_a, _cookie_a) = ws_connect(port);
    let (ws_b, cookie_b) = ws_connect(port);
    ws_send(&ws_a, "1:0,0,0,0");
    ws_send(&ws_b, "1:1,10,0,0");

    // B hellos — earning the UDP fast path with the kernel-negotiated cookie.
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.connect(("127.0.0.1", port)).unwrap();
    udp.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut b_pose: Option<String> = None;
    loop {
        udp.send(format!("H:{cookie_b}").as_bytes()).unwrap();
        match udp_recv(&udp) {
            Some(d) if d == "A:1" => break,
            Some(d) if d.starts_with("S:") => {
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
        match b_pose.take().or_else(|| udp_recv(&udp)) {
            Some(d) => {
                let (_, payload) = unstamp(&d);
                if payload.starts_with("2:0,") {
                    break d;
                }
            }
            None => assert!(Instant::now() < deadline, "B never saw a UDP pose"),
        }
    };
    let (seq, _) = unstamp(&dgram);
    assert!(seq >= 1, "outbound seq stamped: {dgram:?}");
}
