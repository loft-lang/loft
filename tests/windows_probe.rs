// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 / WINDOWS.md — FOCUSED Windows probes for the engine-host kernel
//! (run by `.github/workflows/windows-probe.yml` on the `windows-probe`
//! branch; a no-op crate everywhere else).  Each probe answers ONE question
//! the unix-gated lifecycle suite leaves open on Windows; findings graduate
//! into WINDOWS.md and, where green, into un-gating the real tests.
//!
//! Probe 1 — does the kernel SERVE on Windows?  (std-bind fallback path:
//!           listen, ws upgrade, event echo, tick.)
//! Probe 2 — same-port UDP beside the listener (the 05a auto-path's bind).
//! Probe 3 — THE OVERLAP QUESTION: can a second listener bind the same port
//!           while the first still listens (the S5 swap handover rides
//!           SO_REUSEPORT on unix; Windows SO_REUSEADDR semantics differ)?
//!           This probe REPORTS, it does not assert a wish.
//! Probe 4 — does killing a child reap its GRANDCHILD?  `stop_game` reaches a
//!           `--native` game's real server (a grandchild) through `killpg`,
//!           which is `cfg(unix)`; Windows has no process group, so the
//!           question is whether the grandchild survives — and whether
//!           `taskkill /T` reaps it, since that decides the fix's shape.
//! Probe 5 — UDP on the port a TCP listener already holds (the 05a auto-path
//!           binds both).  Different protocols, so unix allows it; this asks
//!           what Windows does.
#![cfg(windows)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Is `port` held by something?  A refused connect means the holder is gone —
/// the liveness signal probe 4 reads, because a process handle says nothing
/// about a process it is not the parent of.
fn port_is_held(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )
    .is_ok()
}

/// Wait until `port` reaches `want`, or give up.  Returns whether it did.
fn await_port(port: u16, want: bool, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if port_is_held(port) == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    port_is_held(port) == want
}

use loft::compile;
use loft::parser::Parser;
use loft::state::State;

/// Drive the kernel IN-PROCESS (no spawned loft binary — fast on a cold
/// runner): parse a minimal server, byte-code it, run its main on a thread.
fn spawn_inprocess_kernel(port: u16) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let src = format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      w.events = w.events + 1;
      engine_host::send(ev.cid, "got:{{ev.payload}}#{{w.events}}t{{w.ticks}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        );
        // Round-1 FINDING, now FIXED + validated here: parse_str with a
        // virtual name died on the use-clause resume ("Fatal: Unknown
        // file:<win-probe>") — never Windows-specific (Linux failed
        // identically); the lexer now re-serves registered in-memory
        // sources on switch.  tests/parse_str.rs is the cross-platform
        // regression; this probe doubles as its Windows leg.
        let mut parser = Parser::new();
        parser.parse_dir("default", true, false).expect("stdlib");
        parser.lib_dirs = vec!["lib".to_string()];
        parser.parse_str(&src, "<win-probe>", false);
        assert!(
            parser.diagnostics.level() < loft::diagnostics::Level::Error,
            "probe server must parse clean: {:?}",
            parser.diagnostics.lines()
        );
        loft::scopes::check(&mut parser.data, &mut parser.database);
        let mut data = parser.data;
        let mut state = State::new(parser.database);
        compile::byte_code(&mut state, &mut data);
        state.execute_argv("main", &data, &[]);
    })
}

fn ws_connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(30);
    let stream = loop {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        assert!(Instant::now() < deadline, "kernel never listened on {port}");
        std::thread::sleep(Duration::from_millis(200));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let req = "GET /ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
               Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
    (&stream).write_all(req.as_bytes()).unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    assert!(
        String::from_utf8_lossy(&head).contains("101"),
        "upgrade accepted"
    );
    stream
}

fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [0x21u8, 0x43, 0x65, 0x87];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8];
    assert!(bytes.len() <= 125);
    frame.push(0x80 | bytes.len() as u8);
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

fn ws_recv(stream: &TcpStream) -> String {
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).unwrap();
    let len = (hdr[1] & 0x7F) as usize;
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).unwrap();
    String::from_utf8(payload).unwrap()
}

/// Probe 1+2: the kernel serves on Windows — listen (std fallback), upgrade,
/// event echo with world state, drift-free ticks, and the same-port UDP bind
/// (its failure is a warning by design; this asserts the WS path regardless).
#[test]
fn probe_kernel_serves_on_windows() {
    let port = 18200u16;
    let _kernel = spawn_inprocess_kernel(port);
    let ws = ws_connect(port);
    ws_send(&ws, "a");
    let r1 = ws_recv(&ws);
    assert!(r1.starts_with("got:a#1t"), "first echo: {r1}");
    std::thread::sleep(Duration::from_millis(120));
    ws_send(&ws, "b");
    let r2 = ws_recv(&ws);
    assert!(r2.starts_with("got:b#2t"), "world advanced: {r2}");
    // Ticks moved between the two echoes (the 10 ms drift-free tick).
    let t1: i64 = r1.rsplit('t').next().unwrap().parse().unwrap();
    let t2: i64 = r2.rsplit('t').next().unwrap().parse().unwrap();
    assert!(t2 > t1, "ticks advance on Windows: {t1} -> {t2}");
    println!("PROBE kernel-serves: OK (echo + world + ticks)");
}

/// Probe 3 — the overlap question, REPORTED not wished: while a listener
/// holds a port, what does a second std bind on the same port do on
/// Windows?  (unix: fails without SO_REUSEPORT; the S5 swap counts on the
/// overlap.)  The answer decides the Windows swap design: overlap-capable
/// (port the unix shape) or sequential-rebind (close-then-bind + rollback
/// rebind).
#[test]
fn probe_same_port_second_bind_semantics() {
    let first = std::net::TcpListener::bind(("0.0.0.0", 18201)).expect("first bind");
    let second = std::net::TcpListener::bind(("0.0.0.0", 18201));
    println!(
        "PROBE overlap: second std bind while first listens -> {:?}",
        second.as_ref().map(|_| "BOUND").map_err(|e| e.kind())
    );
    // Whatever the semantics, the FIRST listener must still accept.
    let probe = TcpStream::connect(("127.0.0.1", 18201));
    println!(
        "PROBE overlap: connect during the experiment -> {:?}",
        probe.as_ref().map(|_| "CONNECTED").map_err(|e| e.kind())
    );
    drop(second);
    drop(first);
}

/// Probe 3b — the SEQUENTIAL-REBIND feasibility (the Windows swap design,
/// given 3's AddrInUse verdict): close the listener, rebind the same port
/// immediately — how fast does it succeed?  (unix SO_REUSEADDR makes this
/// instant; Windows TIME_WAIT behavior is the question.)  Reports the
/// latency; asserts only that it succeeds within the swap's budget.
#[test]
fn probe_sequential_rebind_latency() {
    let port = 18202u16;
    let first = std::net::TcpListener::bind(("0.0.0.0", port)).expect("first bind");
    // Exercise the listener so the close isn't trivially clean.
    let conn = TcpStream::connect(("127.0.0.1", port)).expect("dial");
    let _accepted = first.accept().expect("accept");
    drop(conn);
    drop(first);
    let t0 = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(10);
    let rebound = loop {
        match std::net::TcpListener::bind(("0.0.0.0", port)) {
            Ok(l) => break l,
            Err(e) => {
                assert!(
                    Instant::now() < deadline,
                    "rebind never succeeded within 10 s ({e})"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    println!(
        "PROBE sequential-rebind: succeeded after {:?}",
        t0.elapsed()
    );
    drop(rebound);
}

// ── Probe 4: the orphaned grandchild ────────────────────────────────────────────────
//
// `Repl::stop_game` kills the process GROUP so it reaches a `--native` game's real
// server, which is a GRANDCHILD (driver → compiled binary).  `killpg` is `cfg(unix)`;
// on Windows the same call site has only `child.kill()`, which reaps the child alone.
// Whether that orphans the grandchild is a platform fact, so it is measured here rather
// than reasoned about — and the same probe measures `taskkill /T`, because a fix that
// cannot be shown to work on the runner is not yet a fix.
//
// The role helper below is how one test binary supplies all three processes: it is a
// no-op unless the role env var is set, so the ordinary suite never notices it.

const ROLE: &str = "LOFT_WINPROBE_ROLE";
const ROLE_PORT: &str = "LOFT_WINPROBE_PORT";
/// Where the grandchild writes its PID, when set.  Inherited through the child, so the
/// driver can name the grandchild even after the child that knows it has been killed.
const ROLE_PIDFILE: &str = "LOFT_WINPROBE_PIDFILE";

/// Re-invoke this test binary as `role`, running only the helper cell.
fn spawn_role(role: &str, port: u16) -> std::process::Child {
    let exe = std::env::current_exe().expect("current_exe");
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--exact", "winprobe_role_helper", "--nocapture"])
        .env(ROLE, role)
        .env(ROLE_PORT, port.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn().expect("spawn role")
}

/// The child and grandchild bodies, selected by env.  Without the env var — every
/// ordinary run, including the real suite — it returns immediately.
#[test]
fn winprobe_role_helper() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    let port: u16 = std::env::var(ROLE_PORT)
        .ok()
        .and_then(|p| p.parse().ok())
        .expect("role port");
    match role.as_str() {
        // The driver: spawn the real server, then stay alive holding nothing itself.
        // Nothing here holds the port, so the port answers about the GRANDCHILD alone.
        "child" => {
            let _grandchild = spawn_role("grandchild", port);
            std::thread::sleep(Duration::from_secs(120));
        }
        // The real server: hold the port until killed.
        "grandchild" => {
            if let Ok(pidfile) = std::env::var(ROLE_PIDFILE) {
                std::fs::write(pidfile, std::process::id().to_string()).expect("pid file");
            }
            let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind");
            let deadline = Instant::now() + Duration::from_secs(120);
            while Instant::now() < deadline {
                // Accept so the liveness probe's connect completes rather than backlogs.
                listener.set_nonblocking(true).ok();
                match listener.accept() {
                    Ok(_) => {}
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        }
        other => panic!("unknown role {other}"),
    }
}

/// Probe 4a — REPORTS whether `child.kill()` reaps the grandchild.  This is the
/// `stop_game` shape on Windows exactly: kill the handle we own, and ask whether the
/// process that actually serves is gone.
#[test]
fn probe_child_kill_reaches_the_grandchild() {
    let port = 18203u16;
    let pidfile = std::env::temp_dir().join(format!("loft_winprobe_4a_{}.pid", std::process::id()));
    let _ = std::fs::remove_file(&pidfile);
    // SAFETY: set before the child is spawned, and no other thread of this test reads or
    // writes the environment; the child and grandchild inherit it.
    unsafe { std::env::set_var(ROLE_PIDFILE, &pidfile) };
    let mut child = spawn_role("child", port);
    unsafe { std::env::remove_var(ROLE_PIDFILE) };
    assert!(
        await_port(port, true, Duration::from_secs(30)),
        "the grandchild never took port {port} — the probe measured nothing"
    );
    let _ = child.kill();
    let _ = child.wait();
    let reaped = await_port(port, false, Duration::from_secs(5));
    println!(
        "PROBE grandchild-after-child-kill: port {port} {} — grandchild {}",
        if reaped { "RELEASED" } else { "STILL HELD" },
        if reaped { "reaped" } else { "ORPHANED" }
    );
    // Leave nothing behind whichever way it went: an orphan holding a port would fail the
    // next probe on a warm runner, and one holding the handles this process inherited from
    // the test runner is reported as a LEAK.  It is killed by its OWN pid: `taskkill /T` on
    // the child reaches nothing here, because the child is already dead and the tree walk
    // goes by parent link (probe 4b is that ordering, the right way round).
    if !reaped {
        let pid = std::fs::read_to_string(&pidfile).unwrap_or_default();
        let pid = pid.trim();
        assert!(
            !pid.is_empty(),
            "the grandchild wrote no pid to {} — it cannot be cleaned up",
            pidfile.display()
        );
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", pid])
            .output();
        assert!(
            await_port(port, false, Duration::from_secs(10)),
            "the orphaned grandchild (pid {pid}) still holds port {port} after taskkill"
        );
    }
    let _ = std::fs::remove_file(&pidfile);
}

/// Probe 4b — the sequence `Repl::stop_game` runs on Windows, in its exact order:
/// `taskkill /T /F` on the child, then `child.kill()`, then `child.wait()`.  Asserting
/// the port is released after the first step is what guards the fix, and the ordering is
/// the whole finding: `taskkill /T` walks the tree by parent link, so the child must
/// still be ALIVE to be walked from.  Run it after the kill and there is nothing left to
/// walk — which is why 4a, the same experiment without this step, still reports an orphan.
///
/// 4a stays as the PLATFORM fact (a bare `child.kill()` orphans the grandchild); this
/// cell is the one that goes red if the cure stops working.
#[test]
fn probe_taskkill_tree_reaches_the_grandchild() {
    let port = 18204u16;
    let mut child = spawn_role("child", port);
    assert!(
        await_port(port, true, Duration::from_secs(30)),
        "the grandchild never took port {port} — the probe measured nothing"
    );
    let out = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .output()
        .expect("taskkill runs");
    let reaped = await_port(port, false, Duration::from_secs(10));
    println!(
        "PROBE taskkill-tree: exit={:?} stdout={} — port {port} {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim(),
        if reaped { "RELEASED" } else { "STILL HELD" }
    );
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        reaped,
        "taskkill /T did not reach the grandchild — the Job Object shape is required"
    );
}

/// Probe 5 — UDP beside a TCP listener on one port (the 05a auto-path binds both).
/// REPORTS; the two are different protocols, so a refusal here would be a Windows
/// constraint the auto-path has to design around rather than a bug to fix.
#[test]
fn probe_udp_beside_tcp_on_one_port() {
    let port = 18205u16;
    let tcp = std::net::TcpListener::bind(("0.0.0.0", port)).expect("tcp bind");
    let udp = std::net::UdpSocket::bind(("0.0.0.0", port));
    println!(
        "PROBE udp-beside-tcp: udp bind on the tcp port -> {:?}",
        udp.as_ref().map(|_| "BOUND").map_err(|e| e.kind())
    );
    // A second UDP bind on that same port — the overlap question, for UDP.
    if udp.is_ok() {
        let second = std::net::UdpSocket::bind(("0.0.0.0", port));
        println!(
            "PROBE udp-beside-tcp: a SECOND udp bind -> {:?}",
            second.as_ref().map(|_| "BOUND").map_err(|e| e.kind())
        );
    }
    // Whatever UDP did, the TCP listener must still accept.
    let probe = TcpStream::connect(("127.0.0.1", port));
    println!(
        "PROBE udp-beside-tcp: tcp connect during the experiment -> {:?}",
        probe.as_ref().map(|_| "CONNECTED").map_err(|e| e.kind())
    );
    drop(tcp);
}
