// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 — the connector role end-to-end: a loft client (`run_client`)
//! against a loft server (`run`), BOTH with zero transport code.
//!
//! Proves the full auto-path: the kernel negotiates the UDP cookie inside
//! the WS handshake, the connector auto-hellos and keepalives, the server's
//! `broadcast` of a `sync_class` kind reaches the client as datagrams into
//! its conflation slots (`udp=true`), events round-trip over WS, and
//! `run_client` RETURNS when the server goes away.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;

/// VM-aware deadline: CI runners are slow and CONTENDED (parallel test
/// binaries + native-build storms) — scale every wait there so timing
/// reflects the machine, not the meaning.
/// Disk-backed scratch for test fixtures.  `std::env::temp_dir()` is a
/// RAM-backed tmpfs on dev boxes (small quota, shared across sessions), and
/// loft's cache-next-to-source rule would put every `--native` fixture's
/// binary cache there too — the disk-quota stall class.  `target/` lives on
/// disk and is cleaned with the build tree.
fn test_tmp() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn vm_deadline(secs: u64) -> Instant {
    // Stretch the budget when the machine is shared — see `common::deadline_scale`.
    Instant::now() + Duration::from_secs(secs * common::deadline_scale())
}

const PORT_BASE: u16 = 18089;

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

/// Block until something is accepting on `port`.
///
/// Replaces a fixed `sleep(800ms)` that guessed how long a loft interpreter needs to
/// parse its program and reach `listen`.  Under a loaded parallel suite that guess is
/// sometimes wrong, and the resulting failure lands nowhere near its cause: the client
/// connects to a port nobody is listening on yet, gets nothing, and the test fails
/// fifteen seconds later reporting a missing keyframe.  Polling turns "probably long
/// enough" into "actually ready", and costs 25ms rather than 800ms when it is.
fn wait_until_listening(port: u16) {
    let deadline = vm_deadline(15);
    loop {
        if let Ok(s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            // Close cleanly: the probe must not sit in the server's accept queue.
            let _ = s.shutdown(std::net::Shutdown::Both);
            return;
        }
        assert!(Instant::now() < deadline, "server never listened on {port}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn spawn_loft(prog: &PathBuf, piped: bool) -> Child {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Command::new(loft_bin())
        .env("LOFT_OFFLINE", "1") // hermetic fixtures
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(prog)
        .current_dir(&root)
        .stdout(if piped { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn loft")
}

/// Environment-skip detection for the node/chromium harness
/// (`tools/html_render_check.mjs`): returns the skip reason when the run
/// died WITHOUT delivering a product verdict.  A real product failure
/// always carries output — exit 1 prints the JSON failure block, exit 3 a
/// `harness error:` line — so the silent shapes are the environment's:
///  * exit 2 — the harness's own no-chrome skip;
///  * `timeout waiting for` — chrome exists but cannot LAUNCH here (CI
///    runner sandbox; the CDP endpoint never comes up);
///  * empty-output non-zero / signal death — node killed under suite load
///    (OOM / chrome contention; `status.code()` is None on a signal, so
///    the exit-2 arm never sees it either).
fn harness_env_skip(out: &std::process::Output) -> Option<String> {
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.code() == Some(2) {
        return Some(format!("SKIP: {stderr}"));
    }
    if stderr.contains("timeout waiting for") {
        return Some(format!(
            "SKIP: chromium present but not launchable: {stderr}"
        ));
    }
    if !out.status.success() && out.stdout.is_empty() && out.stderr.is_empty() {
        return Some(format!(
            "SKIP: harness died without a verdict ({:?} — signal/OOM under suite load)",
            out.status
        ));
    }
    None
}

#[test]
fn connector_auto_path_end_to_end() {
    let port = common::bind_port(PORT_BASE);
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let server_prog = test_tmp().join(format!("eh_conn_srv_{}.loft", std::process::id()));
    std::fs::write(
        &server_prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  w = W {{ tick: 0 }};
  engine_host::run({port}, 100000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 1 {{ engine_host::send(ev.cid, "7:{{ev.payload}}"); }}
    }},
    fn() {{
      w.tick = w.tick + 1;
      engine_host::broadcast("2:{{w.tick}}");
    }});
}}
"#
        ),
    )
    .unwrap();
    let client_prog = test_tmp().join(format!("eh_conn_cli_{}.loft", std::process::id()));
    std::fs::write(
        &client_prog,
        format!(
            r#"
use engine_host;
struct C {{ done: boolean not null }}
fn main() {{
  engine_host::sync_class(2);
  c = C {{ done: false }};
  engine_host::run_client("127.0.0.1", {port}, 100000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{
        println("client: connected");
        engine_host::client_send("hi-event");
      }} else if ev.kind == 1 {{
        if ev.payload.starts_with("2:") {{
          if !c.done {{
            ub = engine_host::client_udp_bound();
            println("client: sync {{ev.payload}} udp={{ub}}");
            if ub {{ c.done = true; }}
          }}
        }} else {{
          println("client: event {{ev.payload}}");
        }}
      }} else if ev.kind == 2 {{
        println("client: disconnected");
      }}
    }},
    fn() {{ }});
  println("client: loop exited");
}}
"#
        ),
    )
    .unwrap();

    let mut server = Guard(Some(spawn_loft(&server_prog, false)));
    wait_until_listening(port);
    let mut client = spawn_loft(&client_prog, true);
    let stdout = client.stdout.take().expect("client stdout piped");
    let _client_guard = Guard(Some(client));

    // Stream the client's lines through a channel so asserts can deadline.
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let wait_for = |rx: &mpsc::Receiver<String>, what: &str, secs: u64| -> String {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "never saw {what:?} on client stdout");
            match rx.recv_timeout(left) {
                Ok(line) if line.contains(what) => return line,
                Ok(_) => continue,
                Err(_) => panic!("never saw {what:?} on client stdout"),
            }
        }
    };

    // 1. The connector connected and its event round-trips over WS.
    wait_for(&rx, "client: connected", 15);
    wait_for(&rx, "client: event 7:hi-event", 10);
    // 2. The fast path goes live with zero transport code on either side:
    //    a sync-class broadcast arrives via the conflation slots over UDP.
    let line = wait_for(&rx, "udp=true", 10);
    assert!(
        line.contains("client: sync 2:"),
        "the udp=true line is a sync beacon: {line:?}"
    );
    // 3. Lifecycle: kill the server; run_client must see the disconnect and
    //    RETURN (unlike the listener's forever-loop).
    if let Some(mut s) = server.0.take() {
        let _ = s.kill();
        let _ = s.wait();
    }
    wait_for(&rx, "client: disconnected", 10);
    wait_for(&rx, "client: loop exited", 10);

    let _ = std::fs::remove_file(&server_prog);
    let _ = std::fs::remove_file(&client_prog);
}

/// Priority keyframes: a sync sample promoted to must-deliver survives a
/// TOTAL datagram blackout.  The client runs with `LOFT_UDP_DROP_NTH=1`
/// (every inbound sync datagram dropped) while bound — so the 100 ms `2:`
/// beacons never arrive — yet the promoted `2:777,…` keyframe lands in the
/// sync slots via its reliable `S:`-framed WS carrier, in the same seq
/// space.  (The positive control — beacons DO arrive without the drop — is
/// `connector_auto_path_end_to_end` above.)
// @speed 7.6
#[test]
fn keyframes_survive_total_datagram_loss() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = common::bind_port(18090);
    let server_prog = test_tmp().join(format!("eh_kf_srv_{}.loft", std::process::id()));
    std::fs::write(
        &server_prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  w = W {{ tick: 0 }};
  engine_host::run({port}, 100000,
    fn(ev: engine_host::Event) {{ }},
    fn() {{
      w.tick = w.tick + 1;
      engine_host::broadcast("2:{{w.tick}}");
      if w.tick - (w.tick / 10) * 10 == 0 {{
        engine_host::keyframe(0, "2:777,{{w.tick}}");
      }}
    }});
}}
"#
        ),
    )
    .unwrap();
    let client_prog = test_tmp().join(format!("eh_kf_cli_{}.loft", std::process::id()));
    std::fs::write(
        &client_prog,
        format!(
            r#"
use engine_host;
fn main() {{
  engine_host::sync_class(2);
  engine_host::run_client("127.0.0.1", {port}, 100000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{ println("kclient: connected"); }}
      else if ev.kind == 1 && ev.payload.starts_with("2:") {{
        ub = engine_host::client_udp_bound();
        println("kclient: sync {{ev.payload}} udp={{ub}}");
      }}
    }},
    fn() {{ }});
}}
"#
        ),
    )
    .unwrap();

    let _server = Guard(Some(spawn_loft(&server_prog, false)));
    wait_until_listening(port);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut client = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&client_prog)
        .current_dir(&root)
        .env("LOFT_UDP_DROP_NTH", "1") // total inbound datagram loss
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn client");
    let stdout = client.stdout.take().expect("piped");
    let _client_guard = Guard(Some(client));

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    // Phase 1: the promoted sample must arrive despite the blackout.  (The
    // pre-bind beacons legitimately arrive over WS and — since the routing
    // symmetry landed — surface through the same sync slots, possibly
    // drained after the bind; they are NOT leaks.)
    let deadline = vm_deadline(15);
    // Keep what DID arrive.  "keyframe never arrived" on its own cannot distinguish a
    // client that never connected from one that connected and got the wrong sync, and
    // the two point at opposite halves of the system.
    let mut seen: Vec<String> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let fail = |seen: &[String]| -> String {
            if seen.is_empty() {
                "keyframe never arrived under blackout; the client printed NOTHING \
                 (it never connected)"
                    .to_string()
            } else {
                format!(
                    "keyframe never arrived under blackout; the client printed:\n  {}",
                    seen.join("\n  ")
                )
            }
        };
        assert!(!left.is_zero(), "{}", fail(&seen));
        let Ok(line) = rx.recv_timeout(left) else {
            panic!("{}", fail(&seen));
        };
        seen.push(line.clone());
        if line.contains("udp=true") && line.contains("sync 2:777,") {
            break; // the promoted sample arrived on the reliable carrier
        }
    }
    // Phase 2: AFTER a keyframe (definitely post-bind), the only sync that
    // may arrive is more keyframes — a plain beacon now would be a datagram
    // that slipped the blackout.
    let until = vm_deadline(2);
    while let Ok(line) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        if line.contains("udp=true") && line.contains("sync 2:") {
            assert!(
                line.contains("sync 2:777,"),
                "a non-keyframe datagram slipped through the blackout: {line:?}"
            );
        }
        if Instant::now() >= until {
            break;
        }
    }

    let _ = std::fs::remove_file(&server_prog);
    let _ = std::fs::remove_file(&client_prog);
}

/// Minimal masked-client WS for the S6 push driver (16-bit length frames —
/// the build blob exceeds 125 bytes).
fn s6_ws_connect(port: u16) -> std::net::TcpStream {
    use std::io::{Read, Write};
    let deadline = vm_deadline(15);
    let stream = loop {
        if let Ok(s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        assert!(Instant::now() < deadline, "server never listened on {port}");
        std::thread::sleep(Duration::from_millis(100));
    };
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
    assert!(String::from_utf8_lossy(&head).contains("101"));
    stream
}

fn s6_ws_send(stream: &std::net::TcpStream, text: &str) {
    use std::io::Write;
    let mask = [0x21u8, 0x43, 0x65, 0x87];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8];
    assert!(bytes.len() <= 0xFFFF, "frame too large for the 16-bit form");
    if bytes.len() <= 125 {
        frame.push(0x80 | bytes.len() as u8);
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

/// Receive one server text frame (unmasked, 7-bit or 16-bit length).
/// `None` on a read timeout / closed stream / non-text frame — callers
/// loop under their own deadline.
fn s6_ws_recv(stream: &std::net::TcpStream) -> Option<String> {
    use std::io::Read;
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).ok()?;
    let len = match hdr[1] & 0x7F {
        126 => {
            let mut l = [0u8; 2];
            s.read_exact(&mut l).ok()?;
            u16::from_be_bytes(l) as usize
        }
        127 => return None, // 64-bit frames: never sent by the kernel
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).ok()?;
    (hdr[0] & 0x0F == 1).then(|| String::from_utf8_lossy(&payload).into_owned())
}

/// Refuse to run against a browser bundle built from a different tree.
///
/// `doc/pkg` is a committed artefact, and the two tests below load it rather than building
/// one — so without this they report on whatever was last committed instead of on the tree
/// under test.  That is not hypothetical: the bundle was a YEAR older than the source when
/// loft#1189 was filed, and it predated `client_loop`'s call to `kernel_swap_step`, a native
/// the browser kernel did not supply.  Both tests were green throughout.
///
/// The stamp comes from `scripts/wasm_bundle_stamp.sh`, which `make wasm` writes beside the
/// bundle — ONE home for what the bundle is built from, so the two sides cannot drift.  Read
/// its header for what the stamp covers and what it deliberately does not.
///
/// Failing rather than skipping is the point: a skip here reads the same as a green run to
/// everything that consumes the suite's result, and a green run about an artefact from another
/// era is exactly what this exists to stop.
fn assert_bundle_describes_this_tree() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = root.join("scripts/wasm_bundle_stamp.sh");
    let Ok(out) = Command::new(&script).current_dir(&root).output() else {
        eprintln!("SKIP-CHECK: cannot run {}", script.display());
        return;
    };
    let want = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let have = std::fs::read_to_string(root.join("doc/pkg-src.stamp"))
        .unwrap_or_default()
        .trim()
        .to_string();
    assert!(
        !want.is_empty() && want == have,
        "doc/pkg was built from a different tree — this test would be reporting on that build, \
         not on this one (loft#1189).  Run `make wasm` and commit the bundle.\n  \
         bundle stamp: {}\n  source stamp: {}",
        if have.is_empty() { "<none>" } else { &have },
        if want.is_empty() {
            "<unreadable>"
        } else {
            &want
        },
    );
}

/// FNV-1a 64 — must match `fnv64` in doc/kernel-swap.html.
fn s6_fnv64(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

/// @PLN18 08 scenario S6 — the browser swap: a new build under a LIVING
/// page.  The kernel server relays a build push (the bulk-channel role);
/// the PAGE (the persistent host layer) verifies the hash, exports the
/// world out of the parked instance A, and boots instance B over the SAME
/// WebSocket (the loft-rt adoption hook).  Asserted in the page
/// (doc/kernel-swap.html, which throws on any unmet clause): (a) socket
/// identity (one open ever), (b) world continuity (B's counter resumes
/// from A's, no reset), (c) B advances within the window, (d) a corrupt
/// push is rejected and A keeps running.  The pushed script is THE build
/// artifact of this tier (the interpreter-bundle tier: the script is the
/// build, the wasm module is the substrate); the compiled-module variant
/// arrives with the --html/kernel integration.
// @speed 27.1
#[test]
fn s6_browser_swap_under_living_page() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chrome_ok = ["google-chrome", "chromium", "chromium-browser", "chrome"]
        .iter()
        .any(|c| {
            Command::new("sh")
                .arg("-c")
                .arg(format!("command -v {c}"))
                .output()
                .is_ok_and(|o| o.status.success())
        });
    let node_ok = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .is_ok_and(|o| o.status.success());
    let harness = root.join("tools/html_render_check.mjs");
    if !chrome_ok || !node_ok || !harness.exists() || !root.join("doc/pkg/loft.js").exists() {
        eprintln!("SKIP: chromium/node/harness/bundle missing");
        return;
    }
    assert_bundle_describes_this_tree();

    let port = common::bind_port(18102);
    // The relay server: ticks the sync class; relays "pushblob:" payloads
    // verbatim to every client (the bulk-channel role — content-agnostic).
    let server_prog = test_tmp().join(format!("eh_s6_srv_{}.loft", std::process::id()));
    std::fs::write(
        &server_prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  w = W {{ tick: 0 }};
  engine_host::run({port}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      if ev.payload.starts_with("pushblob:") {{
        engine_host::broadcast(ev.payload[9..]);
        return;
      }}
      if ev.payload.starts_with("c:") {{
        // Relay the page clients' heartbeats so the push driver can
        // OBSERVE the page's state instead of guessing with sleeps
        // (build A/B ignore "c:" frames; the page's build-push hook
        // only fires on "B!:" blobs).
        engine_host::broadcast(ev.payload);
      }}
    }},
    fn() {{
      w.tick = w.tick + 1;
      engine_host::broadcast("2:{{w.tick}}");
    }});
}}
"#
        ),
    )
    .unwrap();
    let _server = Guard(Some(spawn_loft(&server_prog, false)));

    // Serve doc/ for the page + bundle.
    let http_port = common::bind_port(18103);
    let mut http = Command::new("python3")
        .args([
            "-m",
            "http.server",
            &http_port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(root.join("doc"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("http server");

    // Build B's script — THE SAME template as the page's (kept in step by
    // the comment on both sides), marker v2.
    let v2_script = format!(
        r#"
use engine_host;
struct C {{ count: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  c = C {{ count: 0 }};
  engine_host::swap_world(c);
  engine_host::run_client(engine_host::default_host(), {port}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 1 && ev.payload.starts_with("2:") {{
        c.count = c.count + 1;
      }}
    }},
    fn() {{ engine_host::client_send("c:v2:{{c.count}}"); }});
}}
"#
    );
    let good_blob = format!("B!:{}:{v2_script}", s6_fnv64(&v2_script));
    let bad_blob = format!("B!:{}:{v2_script}", s6_fnv64("not the script"));

    // The push driver, CONDITION-driven (the fixed 4s/2s/12s sleeps it
    // replaces raced suite load: chromium + wasm boot can exceed any fixed
    // wait, and a blob pushed before the page's WS is open broadcasts into
    // the void — `rejected` then never flips and the page times out).  The
    // server relays the page clients' "c:" heartbeats to every connection,
    // so each step waits for the page state it needs.
    let pusher = std::thread::spawn(move || {
        let t0 = Instant::now();
        let log = move |msg: &str| {
            eprintln!("[pusher {:>6}ms] {msg}", t0.elapsed().as_millis());
        };
        let ws = s6_ws_connect(port);
        ws.set_read_timeout(Some(Duration::from_millis(500))).ok();
        log("connected");
        // Wait for a frame matching `pred`, bounded by `deadline`; returns
        // whether it arrived.  Logs every page-protocol frame seen.
        let await_frame = |pred: &dyn Fn(&str) -> bool, deadline: Instant| -> bool {
            loop {
                if let Some(f) = s6_ws_recv(&ws) {
                    if f.starts_with("c:") || f.starts_with("B!:") {
                        log(&format!("saw {}", &f[..f.len().min(24)]));
                    }
                    if pred(&f) {
                        return true;
                    }
                }
                if Instant::now() >= deadline {
                    return false;
                }
            }
        };
        let v1_any = |f: &str| f.starts_with("c:v1:");
        // A v1 heartbeat whose COUNT is >= 1: build A has demonstrably
        // received a sync frame, so the page's world-continuity clause
        // (`lastAtSwap >= 1`) can hold when the swap fires.
        let v1_counting = |f: &str| {
            f.strip_prefix("c:v1:")
                .and_then(|n| n.parse::<i64>().ok())
                .is_some_and(|n| n >= 1)
        };
        let v2_any = |f: &str| f.starts_with("c:v2:");
        // The push protocol, RELOAD-IMMUNE: chromium under load can kill
        // and reload the page tab mid-sequence (renderer death; the page
        // reports `boot=N` in its timeout throw), and a reloaded instance
        // loses the `rejected` flag — so a single corrupt+good pair can
        // straddle two instances and never complete.  Both pushes are
        // idempotent page-side (`if (swapped) return`; `rejected` only
        // flips up), so REPEAT the full pair until some single living
        // instance demonstrably swaps (its v2 heartbeats flow).  Each
        // round still proves the strong sequence within one instance:
        // connected → corrupt rejected (A's heartbeat continues) → good
        // accepted (B heartbeats).
        let overall = vm_deadline(60);
        loop {
            assert!(
                await_frame(&v1_any, overall),
                "pusher never saw build A's heartbeat"
            );
            log("push corrupt blob");
            s6_ws_send(&ws, &format!("pushblob:{bad_blob}"));
            // A SURVIVES the rejection and has COUNTED a sync frame (the
            // page's continuity clause needs `lastAtSwap >= 1`).
            assert!(
                await_frame(&v1_counting, overall),
                "pusher never saw a counting post-rejection heartbeat"
            );
            log("push good blob");
            s6_ws_send(&ws, &format!("pushblob:{good_blob}"));
            // A bounded per-round wait: if the swap doesn't confirm (the
            // instance died mid-round), run another round.
            let round = Instant::now() + Duration::from_secs(3);
            if await_frame(&v2_any, round.min(overall)) {
                break;
            }
            assert!(
                Instant::now() < overall,
                "no instance ever completed the swap sequence"
            );
            log("no v2 heartbeat this round — repushing the pair");
        }
        // The swap is confirmed — but if a reload struck BETWEEN the
        // corrupt and good pushes, the SURVIVING instance swapped without
        // ever seeing a corrupt blob (`rejected` died with instance 1; the
        // page reports it as `boot=2` + `rejected=false`).  The page sets
        // `rejected` on a corrupt push at ANY time (the hash check runs
        // before the swapped guard), so deliver one more to the instance
        // we now KNOW is alive, and prove B keeps pumping after it.
        log("push corrupt blob at the surviving instance");
        s6_ws_send(&ws, &format!("pushblob:{bad_blob}"));
        assert!(
            await_frame(&v2_any, overall),
            "build B stopped after the post-swap corrupt push"
        );
        log("done");
        drop(ws);
    });

    // The page's verdict budget scales with the runner (vm_deadline's
    // factor) and stays INSIDE the harness watch window — page-throw
    // before harness-timeout, never a false pass.
    let scale: u64 = if std::env::var_os("CI").is_some() {
        3
    } else {
        1
    };
    let page_deadline_ms = 20_000 * scale;
    let harness_wait_ms = 25_000 * scale;
    let url = format!(
        "http://127.0.0.1:{http_port}/kernel-swap.html?port={port}&deadline_ms={page_deadline_ms}"
    );
    let out = Command::new("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", &harness_wait_ms.to_string()])
        .args(["--port", "18104"])
        .output()
        .expect("node harness");
    let _ = pusher.join();
    let _ = http.kill();
    let _ = http.wait();
    if let Some(reason) = harness_env_skip(&out) {
        eprintln!("{reason}");
        let _ = std::fs::remove_file(&server_prog);
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "browser swap failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let _ = std::fs::remove_file(&server_prog);
}

/// @PLN18 phase 07 acceptance — the ONE-SCRIPT differential: the same loft
/// client source (the template in `doc/kernel-differential.html`, port
/// substituted) runs natively (`run_client`) and in the BROWSER kernel
/// (headless chromium over the doc/pkg interpreter bundle); both must print
/// the identical transcript.  Self-skips without chromium/node (the
/// html_render harness pattern).
// @speed 13.4
#[test]
fn browser_kernel_one_script_differential() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chrome_ok = ["google-chrome", "chromium", "chromium-browser", "chrome"]
        .iter()
        .any(|c| {
            Command::new("sh")
                .arg("-c")
                .arg(format!("command -v {c}"))
                .output()
                .is_ok_and(|o| o.status.success())
        });
    let node_ok = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .is_ok_and(|o| o.status.success());
    let harness = root.join("tools/html_render_check.mjs");
    if !chrome_ok || !node_ok || !harness.exists() || !root.join("doc/pkg/loft.js").exists() {
        eprintln!("SKIP: chromium/node/harness/bundle missing");
        return;
    }
    assert_bundle_describes_this_tree();

    let port = common::bind_port(18105);
    let server_prog = test_tmp().join(format!("eh_diff_srv_{}.loft", std::process::id()));
    std::fs::write(
        &server_prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  w = W {{ tick: 0 }};
  engine_host::run({port}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 1 {{ println("s:connected"); engine_host::send(ev.cid, "7:hi"); }}
    }},
    fn() {{
      w.tick = w.tick + 1;
      engine_host::broadcast("2:{{w.tick}}");
    }});
}}
"#
        ),
    )
    .unwrap();
    // The SAME client source as the page's template (port substituted the
    // same way) — the one-script invariant, asserted by construction.
    let client_src = format!(
        r#"
use engine_host;

struct C {{ saw_sync: boolean not null }}
fn main() {{
  engine_host::sync_class(2);
  c = C {{ saw_sync: false }};
  engine_host::run_client(engine_host::default_host(), {port}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{
        println("t:connected");
        engine_host::client_send("hello");
      }} else if ev.kind == 1 {{
        if ev.payload.starts_with("2:") {{
          if !c.saw_sync {{
            c.saw_sync = true;
            println("t:sync-ok");
          }}
        }} else {{
          println("t:event {{ev.payload}}");
        }}
      }} else if ev.kind == 2 {{
        println("t:disconnected");
      }}
    }},
    fn() {{ }});
  println("t:exited");
}}
"#
    );
    let client_prog = test_tmp().join(format!("eh_diff_cli_{}.loft", std::process::id()));
    std::fs::write(&client_prog, &client_src).unwrap();
    let expect = [
        "t:connected",
        "t:event 7:hi",
        "t:sync-ok",
        "t:disconnected",
        "t:exited",
    ];

    // ── Native leg ──
    {
        let _server = Guard(Some(spawn_loft(&server_prog, false)));
        wait_until_listening(port);
        let client = Command::new(loft_bin())
            .arg("--interpret")
            .arg("--no-warnings")
            .arg("--lib")
            .arg(root.join("lib"))
            .arg(&client_prog)
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("native client");
        std::thread::sleep(Duration::from_secs(3));
        // Kill the server; the client sees the disconnect and exits.
        drop(_server);
        let out = client.wait_with_output().expect("client output");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("t:")).collect();
        assert_eq!(lines, expect, "native transcript");
    }

    // ── Browser leg ──
    // Piped: the server prints `s:connected` when the page's client has spoken, which is
    // what the kill below waits for.
    let mut server_child = spawn_loft(&server_prog, true);
    let server_out = server_child.stdout.take().expect("piped server stdout");
    let _server = Guard(Some(server_child));
    wait_until_listening(port);
    // Serve doc/ (the page + bundle); kill the kernel server mid-run so the
    // browser client exits and the page compares its transcript.
    let http_port = common::bind_port(18106);
    let mut http = Command::new("python3")
        .args([
            "-m",
            "http.server",
            &http_port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(root.join("doc"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("http server");
    let _http_guard = Guard(None); // placeholder; kill below explicitly
    let killer = std::thread::spawn({
        move || {
            std::thread::sleep(Duration::from_secs(4));
        }
    });
    let url = format!("http://127.0.0.1:{http_port}/kernel-differential.html?port={port}");
    // Kill the kernel server only once the page has CONNECTED and been answered — the
    // server's `s:connected` line — plus a second for the sync frames the page compares.
    // A fixed 4 s from launch killed it before a slow runner's headless chromium had
    // opened its socket, and the page reported `ERR_CONNECTION_REFUSED` (two of three
    // gates on 2026-09-22).  The deadline is the harness's own wait, so a page that never
    // connects still ends the test with the harness's verdict rather than a hang.
    let server_killer = std::thread::spawn(move || {
        let deadline = vm_deadline(12);
        let mut lines = BufReader::new(server_out).lines();
        loop {
            match lines.next() {
                Some(Ok(l)) if l.trim() == "s:connected" => break,
                Some(Ok(_)) if Instant::now() < deadline => continue,
                _ => break,
            }
        }
        std::thread::sleep(Duration::from_secs(1));
        drop(_server);
    });
    let out = Command::new("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", "9000"])
        .args(["--port", "18107"])
        .output()
        .expect("node harness");
    let _ = killer.join();
    let _ = server_killer.join();
    let _ = http.kill();
    let _ = http.wait();
    if let Some(reason) = harness_env_skip(&out) {
        eprintln!("{reason}");
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "browser-kernel differential failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_file(&server_prog);
    let _ = std::fs::remove_file(&client_prog);
}
