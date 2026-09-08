// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

// The kernel lifecycle suite is unix-only v1: the swap machinery rides
// SO_REUSEPORT + process groups (killpg cleans the --native grandchildren),
// neither of which Windows has in this shape — the Windows kernel story is
// the WINDOWS.md G-gap set.
#![cfg(unix)]

//! @PLN18 phase 01 — the kernel end-to-end: a loft program on
//! `engine_host::run` (the Rust-mechanics loop), driven by a real WebSocket
//! client.  Proves: connect event → handler closure (captures mutating) →
//! broadcast; the drift-free tick fires; multiple round trips on one
//! connection.  State is a STRUCT world (per #314: a bare scalar captured by a
//! reader closure + a writer closure crashes; struct-held state is the correct
//! idiom and works).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
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

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft")
}

/// Kills the whole PROCESS GROUP: a `--native` run spawns the compiled
/// binary as a grandchild of the loft driver — killing only the child
/// orphans the actual server (probe-caught: a rerun connected to the
/// previous test's orphan and read its counter).
struct Guard(Option<Child>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            unsafe {
                libc::killpg(c.id() as i32, libc::SIGKILL);
            }
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn ws_connect(port: u16) -> TcpStream {
    let deadline = vm_deadline(15);
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
    // Read the 101 head to its blank line (unbuffered).
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.contains("101"), "upgrade accepted: {head}");
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

#[test]
fn kernel_event_broadcast_and_ticks() {
    run_kernel_scenario(common::bind_port(18087), true);
}

/// @PLN18 08 scenario S1 — the COMPILED baseline serves the same game:
/// the identical fixture, built `--native` (the kernel natives' typed twins
/// in `codegen_runtime`), must produce the identical transcript.  The first
/// run pays the rustc build (cached by content hash afterwards).
#[test]
fn s1_native_baseline_matches_interpreted() {
    run_kernel_scenario(common::bind_port(18094), false);
}

/// @PLN18 08 scenario S3 — edit the interpreted fn; the MIXED build keeps
/// running.  The flip target is interpreted from startup (`LOFT_FLIP_FNS`);
/// the test then EDITS the source file under the serving kernel: a good
/// edit (+1 -> +100) applies live — observed as the world counter's STEP
/// changing while the counter itself stays monotone (continuity: no
/// restart, no reset); a broken edit (a real parse ERROR) keeps the +100
/// body serving; a signature change is rejected (call sites embed frame
/// sizes).  The timeline is differential: the interpreted leg (the original
/// tier-0 reload path) passes the SAME milestones.  Application is
/// asynchronous (dispatch-path poll vs the interp loop's op counter), so
/// the legs assert step timelines + reload milestones, not raw vectors;
/// the stderr file is the synchronization point for the REJECTED edits.
// @speed 5.2
#[test]
fn s3_live_edit_under_native_baseline() {
    let native = run_s3_scenario(common::bind_port(18097), false);
    let interp = run_s3_scenario(common::bind_port(18098), true);
    assert!(
        native.stderr.contains("live-dispatch: n_bump_events"),
        "the native leg must dispatch through the interpreter:\n{}",
        native.stderr
    );
    for (leg, run) in [("native", &native), ("interp", &interp)] {
        assert!(
            run.stderr.contains("'bump_events' v1 live"),
            "{leg}: the good edit must reload:\n{}",
            run.stderr
        );
        assert!(
            run.stderr.contains("kept its old body"),
            "{leg}: the broken edit must be refused:\n{}",
            run.stderr
        );
        assert!(
            run.stderr.contains("changed its signature"),
            "{leg}: the signature change must be refused:\n{}",
            run.stderr
        );
    }
}

fn s3_fixture(port: u16, bump_decl: &str, bump_stmt: &str) -> String {
    format!(
        r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
{bump_decl} {{
  {bump_stmt}
  w.events
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      engine_host::broadcast("got:{{ev.payload}}#{{n}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
    )
}

/// One ping round trip; returns the counter and asserts it advanced
/// (monotone = the world survived whatever just happened).
fn s3_ping(ws: &TcpStream, last: &mut i64) -> i64 {
    ws_send(ws, "p");
    let r = ws_recv(ws);
    let n: i64 = r
        .rsplit('#')
        .next()
        .and_then(|x| x.parse().ok())
        .unwrap_or_else(|| panic!("unparseable reply {r:?}"));
    assert!(n > *last, "world counter went backwards: {} -> {n}", *last);
    let step = n - *last;
    *last = n;
    step
}

/// Ping until the counter's step matches `want` (the edit has applied).
fn s3_await_step(ws: &TcpStream, last: &mut i64, want: i64) {
    let deadline = vm_deadline(15);
    loop {
        if s3_ping(ws, last) == want {
            return;
        }
        assert!(Instant::now() < deadline, "step never became {want}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Block until the kernel's stderr contains `needle` — the deterministic
/// sync point for an edit whose only observable is a refusal line.
/// Keeps PINGING while it waits: in the mixed build the reload poll runs
/// inside the dispatch path, so an edit is only examined when the flipped
/// fn is actually called (lazy by construction — an S3 finding; the pings
/// drive it, and stay step-correct on the old body, proving refusal en
/// route).
fn s3_await_stderr(ws: &TcpStream, last: &mut i64, path: &std::path::Path, needle: &str) {
    let deadline = vm_deadline(15);
    loop {
        s3_ping(ws, last);
        if std::fs::read_to_string(path)
            .unwrap_or_default()
            .contains(needle)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "stderr never contained {needle:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn run_s3_scenario(port: u16, interpret: bool) -> S2Run {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return S2Run {
            replies: Vec::new(),
            stderr: String::new(),
        };
    }
    let decl_v0 = "fn bump_events(w: W) -> integer";
    let prog = test_tmp().join(format!("eh_s3_{port}_{}.loft", std::process::id()));
    std::fs::write(&prog, s3_fixture(port, decl_v0, "w.events = w.events + 1;")).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = test_tmp().join(format!("eh_s3_{port}_{}.err", std::process::id()));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1"); // hermetic fixtures
    cmd.env("LOFT_LIVE_RELOAD", "1");
    if interpret {
        cmd.arg("--interpret");
    } else {
        cmd.env("LOFT_LIVE_FLIP", "1")
            .env("LOFT_FLIP_FNS", "bump_events")
            .env("LOFT_DISPATCH_DEBUG", "1");
    }
    cmd.process_group(0); // own group, so Guard can kill driver + grandchild
    let child = cmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));

    let ws = ws_connect(port);
    let mut last = 0i64;
    assert_eq!(s3_ping(&ws, &mut last), 1, "original body steps by 1");
    // Good edit: the step becomes +100; the world counter continues.
    std::fs::write(
        &prog,
        s3_fixture(port, decl_v0, "w.events = w.events + 100;"),
    )
    .unwrap();
    s3_await_step(&ws, &mut last, 100);
    // Broken edit: a real parse ERROR (the lenient parser downgrades a
    // missing operand to warning + null, so use an unknown name).  Sync on
    // the refusal line, then prove the +100 body still serves.
    std::fs::write(
        &prog,
        s3_fixture(port, decl_v0, "w.events = w.events + nosuchvar;"),
    )
    .unwrap();
    s3_await_stderr(&ws, &mut last, &err_path, "kept its old body");
    assert_eq!(
        s3_ping(&ws, &mut last),
        100,
        "broken edit must not change the body"
    );
    // Signature change: rejected; the +100 body still serves.
    std::fs::write(
        &prog,
        s3_fixture(
            port,
            "fn bump_events(w: W, extra: integer) -> integer",
            "w.events = w.events + extra;",
        ),
    )
    .unwrap();
    s3_await_stderr(&ws, &mut last, &err_path, "changed its signature");
    assert_eq!(
        s3_ping(&ws, &mut last),
        100,
        "sig change must not change the body"
    );
    drop(_guard); // kill now so the stderr file is complete
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
    S2Run {
        replies: vec![last.to_string()],
        stderr,
    }
}

/// One full connect+upgrade ATTEMPT; `None` on any failure.  During the
/// swap's dual-bind overlap a dial can land on the DYING process's listener
/// queue and drop mid-upgrade — seats (and this harness) retry the whole
/// attempt, which converges on the new build within the gap bound.
fn ws_try_connect(port: u16) -> Option<TcpStream> {
    let stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let req = "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
               Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
               Sec-WebSocket-Version: 13\r\n\r\n";
    (&stream).write_all(req.as_bytes()).ok()?;
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).ok()?;
        head.push(b[0]);
    }
    String::from_utf8_lossy(&head)
        .contains("101")
        .then_some(stream)
}

/// Tolerant frame read for swap-gap detection: `None` = the connection
/// closed (the handover) or timed out.
fn ws_try_recv(stream: &TcpStream) -> Option<String> {
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
    if hdr[0] & 0x0F == 0x08 {
        return None; // close frame
    }
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).ok()?;
    String::from_utf8(payload).ok()
}

/// @PLN18 08 scenario S5 — the native swap: a new process under a running
/// world.  One test drives the WHOLE heart pipeline: a compiled kernel
/// serves with a fn flipped to the interpreter (S2) → rollback legs (a
/// missing artifact is refused; a build that dies before serving rolls
/// back while the world keeps counting) → a live source edit (S3's
/// subject) → background rebuild (S4) → swap: the world counter crosses
/// the cutover EXACTLY continuous, the tick stamp never resets, the WS
/// seat's gap is bounded (measured — probe gate 5's seat-reconnect
/// verdict), and the flipped fn runs COMPILED in the new build (dispatch
/// reset: zero post-swap interp dispatches).
/// Kill any process running an `eh_s5` cache binary — OUR OWN fixture's
/// stem, exactly anchored.  The swap child outlives its parent chain by
/// DESIGN (it is the new server), so the process-group Guard alone cannot
/// be relied on across runs; with SO_REUSEPORT a stale orphan silently
/// SHARES the test port and poisons every dial (probe-caught).
fn s5_kill_stale(stem: &str) {
    if let Ok(out) = Command::new("pgrep").arg("-f").arg(stem).output() {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            if let Ok(pid) = pid.parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }
}

/// Reap ANY process still bound to `port` before this test reuses it.  `s5_kill_stale`
/// greps for the cache-path stem, but a leaked native **swap child** (the hot-swapped v2
/// build) runs from a `loft_native_bin_*` path in the scratch dir — its command line does
/// NOT contain the stem, so the stem `pgrep` misses it.  If a prior run's `Guard::drop`
/// (the process-group kill) was skipped — e.g. nextest SIGKILLs a timed-out test, bypassing
/// unwinding — that orphan survives on the port, and the next `ws_connect(port)` binds to
/// it (a stale, already-flipped, high-count world → the s5/s7 full-suite flake).  A
/// port-scoped reap closes that gap regardless of the orphan's command line.  Call AFTER
/// `s5_kill_stale` and BEFORE spawning this run's child, so it never kills our own child.
fn reap_port(port: u16) {
    // Killing is not enough: `SIGKILL` is asynchronous, and with `SO_REUSEPORT` a dying
    // orphan's listener can still accept a connection while the kernel tears it down — so
    // a kill-then-immediately-bind still races onto the stale world.  The orphans are
    // reparented to init (not our children), so we cannot `waitpid` them; instead POLL
    // `lsof` until the port is genuinely free.  The happy path (no orphan) is a single
    // `lsof` that returns empty and exits at once; the loop is bounded so a genuinely
    // stuck holder surfaces via the spawn/connect below rather than hanging here.
    for _ in 0..40 {
        let pids: Vec<i32> = match Command::new("lsof")
            .arg("-ti")
            .arg(format!("tcp:{port}"))
            .output()
        {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .filter_map(|p| p.parse::<i32>().ok())
                .collect(),
            Err(_) => return, // no lsof available — nothing to poll
        };
        if pids.is_empty() {
            return; // port is genuinely free
        }
        for pid in pids {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Kill this test's own cache-binary orphans when it ends, however it ends.
///
/// The stem is OWNED because the one that matters carries the RESOLVED port
/// (`common::bind_port`), which is not known until the test runs — a `&'static str` can only
/// spell the BASE port, and the two differ by this checkout's `LOFT_TEST_PORT_OFFSET` band,
/// which is never zero (`common::test_port`, `(cksum(path) % 6 + 1) * 2000`).  A stem built
/// from the base port therefore matches nothing on any checkout, and the guard runs its
/// `pgrep` against a port the process never had.
struct S5Hygiene(String);
impl Drop for S5Hygiene {
    fn drop(&mut self) {
        s5_kill_stale(&self.0);
    }
}

// @speed 1.8
#[test]
fn s5_native_swap_under_running_world() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    // Both stems carry a PORT, and the two ports are not the same one: a prior run's orphan
    // sits on this checkout's CANONICAL port (`test_port`, pure), while our own child lands on
    // whatever `bind_port` could actually take — which differs exactly when an orphan already
    // holds the canonical one, the case the sweep exists for.
    s5_kill_stale(&format!("/.loft/cache/eh_s5_{}-", common::test_port(18100))); // a stale orphan from a prior run shares the port
    std::thread::sleep(Duration::from_millis(200));
    let port = common::bind_port(18100);
    reap_port(port); // reap a leaked swap-child orphan the stem pgrep misses (flake guard)
    let _hygiene = S5Hygiene(format!("/.loft/cache/eh_s5_{port}-")); // OUR swap child dies at exit
    // A test-OWNED always-fails binary: /bin/false varies across platforms
    // and runners (macOS CI refused it — forensics pending); a temp script
    // is deterministic everywhere this unix-only suite runs.
    let bad_bin = test_tmp().join(format!("eh_s5_false_{port}.sh"));
    std::fs::write(&bad_bin, "#!/bin/sh\nexit 1\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bad_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let bad_bin = bad_bin.to_string_lossy().into_owned();
    let fixture = |ver: &str| {
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn bump_events(w: W) -> integer {{
  w.events = w.events + 1;
  w.events
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  resumed = engine_host::swap_world(w);
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      if ev.payload == "rebuild" {{
        engine_host::send(ev.cid, "rebuild:{{engine_host::rebuild_start()}}");
        return;
      }}
      if ev.payload == "status" {{
        engine_host::send(ev.cid, "status:{{engine_host::rebuild_status()}}");
        return;
      }}
      if ev.payload == "swap" {{
        engine_host::send(ev.cid, "swap:{{engine_host::swap_start(engine_host::rebuild_artifact())}}");
        return;
      }}
      if ev.payload == "badpath" {{
        engine_host::send(ev.cid, "badpath:{{engine_host::swap_start("/nonexistent/binary")}}");
        return;
      }}
      if ev.payload == "badswap" {{
        engine_host::send(ev.cid, "badswap:{{engine_host::swap_start("{bad_bin}")}}");
        return;
      }}
      engine_host::send(ev.cid, "{ver}:{{ev.payload}}#{{n}}t{{w.ticks}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        )
    };
    let prog = test_tmp().join(format!("eh_s5_{port}.loft"));
    std::fs::write(&prog, fixture("v1")).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = test_tmp().join(format!("eh_s5_{port}.err"));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1"); // hermetic fixtures
    cmd.env("LOFT_LIVE_FLIP", "1")
        .env("LOFT_FLIP_FNS", "bump_events")
        .env("LOFT_DISPATCH_DEBUG", "1");
    cmd.process_group(0);
    let child = cmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));

    let stderr_now = || std::fs::read_to_string(&err_path).unwrap_or_default();
    let mut count = 0i64; // every event bumps the world counter
    let ws = ws_connect(port);
    let ask = |ws: &TcpStream, msg: &str, count: &mut i64| -> String {
        ws_send(ws, msg);
        *count += 1;
        ws_recv(ws)
    };
    // The flipped baseline serves (S2 still alive under this fixture).
    let r = ask(&ws, "a", &mut count);
    assert_eq!(
        r,
        format!("v1:a#{count}t")
            .rsplit_once('t')
            .unwrap()
            .0
            .to_string()
            + "t"
            + r.rsplit_once('t').unwrap().1,
        "shape — first-ask flake probe: raw={r:?} count={count}; subprocess stderr:\n{}",
        stderr_now()
    );
    assert!(r.starts_with(&format!("v1:a#{count}t")), "baseline: {r}");
    assert!(stderr_now().contains("live-dispatch: n_bump_events"));

    // Rollback leg 1: a missing artifact is refused outright (no freeze).
    assert_eq!(ask(&ws, "badpath", &mut count), "badpath:false");
    // Rollback leg 2: a build that dies before serving.  The freeze defers
    // the next replies; they drain after the rollback (recv timeout rides
    // it).  The world must keep counting and the old build keep serving.
    let badswap = ask(&ws, "badswap", &mut count);
    assert_eq!(
        badswap,
        "badswap:true",
        "swap_start must accept the test's false-binary; server stderr:\n{}",
        std::fs::read_to_string(&err_path).unwrap_or_default()
    );
    let r = ask(&ws, "b", &mut count);
    assert!(
        r.starts_with(&format!("v1:b#{count}t")),
        "post-rollback: {r}"
    );
    assert!(
        stderr_now().contains("rolled back"),
        "the dead build must roll back:\n{}",
        stderr_now()
    );

    // S3+S4: live edit, then background rebuild to ready.
    std::fs::write(&prog, fixture("v2")).unwrap();
    assert_eq!(ask(&ws, "rebuild", &mut count), "rebuild:true");
    let deadline = vm_deadline(300);
    loop {
        let st = ask(&ws, "status", &mut count);
        if st == "status:2" {
            break;
        }
        assert!(Instant::now() < deadline, "rebuild never ready");
        std::thread::sleep(Duration::from_millis(300));
    }

    // THE SWAP.  The ack arrives before the freeze; the world counter and
    // tick stamp in the LAST pre-swap reply are the continuity anchors.
    let r = ask(&ws, "c", &mut count);
    assert!(r.starts_with(&format!("v1:c#{count}t")), "pre-swap: {r}");
    let t_pre: i64 = r.rsplit_once('t').unwrap().1.parse().unwrap();
    assert_eq!(ask(&ws, "swap", &mut count), "swap:true");

    // Wait for the handover: the old process closes this connection.
    ws.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let gap_start = Instant::now();
    while ws_try_recv(&ws).is_some() {}
    // Reconnect into the NEW build (seat-reconnect semantics; measure it).
    // Full-attempt retry: a dial in the dual-bind overlap can land on the
    // dying process and drop mid-upgrade.
    let reconnect_deadline = vm_deadline(10);
    let new_ws = loop {
        if let Some(ws) = ws_try_connect(port) {
            break ws;
        }
        if Instant::now() >= reconnect_deadline {
            // Diagnostics: who exists, who listens, what the server said.
            let ps = Command::new("pgrep").args(["-af", "eh_s5_18100"]).output();
            let ss = Command::new("ss").args(["-tlnp"]).output();
            panic!(
                "never reconnected into the new build\n--- pgrep:\n{}\n--- ss -tlnp:\n{}\n--- server stderr tail:\n{}",
                String::from_utf8_lossy(&ps.map(|o| o.stdout).unwrap_or_default()),
                String::from_utf8_lossy(&ss.map(|o| o.stdout).unwrap_or_default())
                    .lines()
                    .filter(|l| l.contains("18100"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                stderr_now()
                    .lines()
                    .rev()
                    .take(8)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let gap = gap_start.elapsed();
    eprintln!("S5 swap gap (close -> serving reconnect): {gap:?}");
    assert!(
        gap < Duration::from_secs(3),
        "the swap gap must stay under the keepalive timeout: {gap:?}"
    );

    // Continuity: the new build serves v2 meaning over the SAME world —
    // counter exactly +1 (nothing lost, nothing duplicated), ticks never
    // reset.
    let marker = stderr_now();
    let r = ask(&new_ws, "d", &mut count);
    assert!(
        r.starts_with(&format!("v2:d#{count}t")),
        "the new build must serve the new meaning over the OLD world: {r}"
    );
    let t_post: i64 = r.rsplit_once('t').unwrap().1.parse().unwrap();
    assert!(
        t_post >= t_pre,
        "tick stamp must cross the swap monotonically ({t_pre} -> {t_post})"
    );
    assert!(
        marker.contains("loft-swap: world restored from"),
        "the new build must restore the snapshot:\n{marker}"
    );
    // Dispatch reset: bump_events is COMPILED in the new build — no interp
    // dispatches after the handover marker.
    let full = stderr_now();
    let after = full.split("handing over").nth(1).unwrap_or("");
    let _ = ask(&new_ws, "e", &mut count); // drive one more dispatch window
    assert!(
        !after.contains("live-dispatch:"),
        "the swap must reset the dispatch tier:\n{after}"
    );
    drop(_guard);
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
}

/// @PLN18 08-S5 (connector half) — swap a CLIENT process under its running
/// world.  Driven over the client's own control endpoint: rebuild (a cache
/// hit — the self-swap) then swap; the retiring client signals the new
/// build via the READY file's CLIENT form ("connected" — a client's
/// serving is its connection), `run_client`'s swap step freezes meaning
/// while mechanics pump, and `swap_retired()` lets a reconnect wrapper
/// tell retirement from a dropped server (the projector's spin,
/// probe-caught live).  World continuity: the tick counter is a SCALAR, so
/// the snapshot restores it fully — the new process resumes counting from
/// the old one's value.
#[test]
fn s5_client_swap_under_running_world() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = common::bind_port(18116);
    const STEM: &str = "/.loft/cache/eh_s5c_";
    s5_kill_stale(STEM);
    let _hygiene = S5Hygiene(STEM.to_string());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let srv_prog = test_tmp().join(format!("eh_s5c_srv_{port}.loft"));
    std::fs::write(
        &srv_prog,
        format!(
            r#"
use engine_host;
struct W {{ t: integer not null }}
fn main() {{
  w = W {{ t: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{ }},
    fn() {{ w.t = w.t + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let mut scmd = Command::new(loft_bin());
    scmd.process_group(0);
    let server = scmd
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&srv_prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let _sguard = Guard(Some(server));

    let cli_prog = test_tmp().join(format!("eh_s5c_cli_{port}.loft"));
    std::fs::write(
        &cli_prog,
        format!(
            r#"
use engine_host;
struct C {{ ticks: integer not null }}
fn main() {{
  c = C {{ ticks: 0 }};
  resumed = engine_host::swap_world(c);
  if resumed {{ println("client: resumed ticks={{c.ticks}}"); }}
  engine_host::run_client(engine_host::default_host(), {port}, 5000,
    fn(ev: engine_host::Event) {{ }},
    fn() {{
      c.ticks = c.ticks + 1;
      if c.ticks > 12000 {{ engine_host::client_stop(); }}
    }});
  if engine_host::swap_retired() {{ println("client: retired"); }}
}}
"#
        ),
    )
    .unwrap();
    let out_path = test_tmp().join(format!("eh_s5c_cli_{port}.out"));
    let err_path = test_tmp().join(format!("eh_s5c_cli_{port}.err"));
    let mut ccmd = Command::new(loft_bin());
    ccmd.env("LOFT_LIVE_FLIP", "1")
        .env("LOFT_DEBUG_CONTROL", "1");
    ccmd.process_group(0);
    let client = ccmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&cli_prog)
        .current_dir(&root)
        .stdout(Stdio::from(std::fs::File::create(&out_path).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&err_path).unwrap()))
        .spawn()
        .expect("spawn client");
    let mut cguard = Guard(Some(client));

    // Scrape the FIRST control endpoint.
    let deadline = vm_deadline(120);
    let ctl_port: u16 = loop {
        let out = std::fs::read_to_string(&out_path).unwrap_or_default();
        if let Some(rest) = out.split("debug control on 127.0.0.1:").nth(1) {
            break rest
                .split_whitespace()
                .next()
                .and_then(|p| p.trim().parse().ok())
                .expect("port parse");
        }
        assert!(Instant::now() < deadline, "client never announced: {out}");
        std::thread::sleep(Duration::from_millis(200));
    };

    // Drive the self-swap over the channel.
    let ctl = ws_connect(ctl_port);
    std::thread::sleep(Duration::from_millis(300)); // let some ticks land
    ws_send(&ctl, "D!:rebuild");
    assert_eq!(ws_recv(&ctl), "D:rebuild started");
    let deadline = vm_deadline(300);
    loop {
        std::thread::sleep(Duration::from_millis(200));
        ws_send(&ctl, "D!:rebuild?");
        let st = ws_recv(&ctl);
        if st == "D:rebuild 2" {
            break;
        }
        assert!(st == "D:rebuild 1", "rebuild status: {st}");
        assert!(Instant::now() < deadline, "rebuild never ready");
    }
    ws_send(&ctl, "D!:swap auto");
    assert_eq!(ws_recv(&ctl), "D:swap true");

    // The OLD driver chain exits (run_client returns 2 -> main ends).
    let deadline = vm_deadline(30);
    loop {
        if let Some(child) = cguard.0.as_mut()
            && child.try_wait().ok().flatten().is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "old client never retired");
        std::thread::sleep(Duration::from_millis(100));
    }

    // Evidence: retirement printed; the NEW process resumed the world (a
    // scalar tick counter restores fully -> it resumed counting from a
    // positive value) and announced its own endpoint.
    let deadline = vm_deadline(30);
    let (resumed_ticks, ctl2_port): (i64, u16) = loop {
        let out = std::fs::read_to_string(&out_path).unwrap_or_default();
        let announce2 = out.match_indices("debug control on 127.0.0.1:").nth(1);
        if let (Some(rest), Some((idx, _))) =
            (out.split("client: resumed ticks=").nth(1), announce2)
        {
            let ticks = rest
                .split_whitespace()
                .next()
                .and_then(|t| t.parse().ok())
                .expect("ticks parse");
            let port = out[idx + "debug control on 127.0.0.1:".len()..]
                .split_whitespace()
                .next()
                .and_then(|p| p.parse().ok())
                .expect("ctl2 parse");
            break (ticks, port);
        }
        assert!(
            Instant::now() < deadline,
            "the new build never resumed/announced: {out}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        resumed_ticks > 0,
        "the world must cross the swap (resumed ticks={resumed_ticks})"
    );
    let out = std::fs::read_to_string(&out_path).unwrap_or_default();
    assert!(
        out.contains("client: retired"),
        "swap_retired must fire: {out}"
    );

    // The new build is debuggable: quit it over ITS endpoint.
    let ctl2 = ws_connect(ctl2_port);
    ws_send(&ctl2, "D!:quit");
    assert_eq!(ws_recv(&ctl2), "D:quitting");
    let _ = std::fs::remove_file(&srv_prog);
    let _ = std::fs::remove_file(&cli_prog);
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
}

/// @PLN18 08-S7 (connector half) — debug a CLIENT process via its own
/// control endpoint.  A connector has no game port a debugger could dial,
/// so under LOFT_DEBUG_CONTROL=1 it binds a loopback listener and announces
/// "engine_host: debug control on 127.0.0.1:<port>" on stdout (the editor
/// scrapes that line exactly like a server's port announce).  The SAME D!:
/// protocol drives it: bp (entry, name-keyed, implies the flip) -> hit with
/// frame bindings while the client's tick is HELD (its mini-pump keeps the
/// server connection alive) -> eval -> resume -> a SECOND hit (still armed,
/// still serving) -> quit over the channel.
#[test]
fn s7_client_debug_over_its_own_endpoint() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = common::bind_port(18115);
    const STEM: &str = "/.loft/cache/eh_s7c_";
    s5_kill_stale(STEM);
    let _hygiene = S5Hygiene(STEM.to_string());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // A minimal kernel server for the client to ride against.
    let srv_prog = test_tmp().join(format!("eh_s7c_srv_{port}.loft"));
    std::fs::write(
        &srv_prog,
        format!(
            r#"
use engine_host;
struct W {{ t: integer not null }}
fn main() {{
  w = W {{ t: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{ }},
    fn() {{ w.t = w.t + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let mut scmd = Command::new(loft_bin());
    scmd.process_group(0);
    let server = scmd
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&srv_prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let _sguard = Guard(Some(server));

    // The COMPILED debuggable client: tick_step is the breakpoint target.
    let cli_prog = test_tmp().join(format!("eh_s7c_cli_{port}.loft"));
    std::fs::write(
        &cli_prog,
        format!(
            r#"
use engine_host;
struct C {{ ticks: integer not null }}
fn tick_step(c: C) -> integer {{
  c.ticks = c.ticks + 1;
  c.ticks
}}
fn main() {{
  c = C {{ ticks: 0 }};
  engine_host::run_client(engine_host::default_host(), {port}, 50000,
    fn(ev: engine_host::Event) {{ }},
    fn() {{
      t = tick_step(c);
      if t > 2400 {{ engine_host::client_stop(); }}
    }});
  println("client: done");
}}
"#
        ),
    )
    .unwrap();
    let out_path = test_tmp().join(format!("eh_s7c_cli_{port}.out"));
    let err_path = test_tmp().join(format!("eh_s7c_cli_{port}.err"));
    let mut ccmd = Command::new(loft_bin());
    ccmd.env("LOFT_LIVE_FLIP", "1")
        .env("LOFT_DEBUG_CONTROL", "1")
        .env("LOFT_DISPATCH_DEBUG", "1");
    ccmd.process_group(0);
    let client = ccmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&cli_prog)
        .current_dir(&root)
        .stdout(Stdio::from(std::fs::File::create(&out_path).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&err_path).unwrap()))
        .spawn()
        .expect("spawn client");
    let mut cguard = Guard(Some(client));

    // Scrape the announced control port (the editor's exact move).
    let deadline = vm_deadline(120);
    let ctl_port: u16 = loop {
        let out = std::fs::read_to_string(&out_path).unwrap_or_default();
        if let Some(rest) = out.split("debug control on 127.0.0.1:").nth(1) {
            break rest
                .split_whitespace()
                .next()
                .and_then(|p| p.trim().parse().ok())
                .expect("port parse");
        }
        assert!(
            Instant::now() < deadline,
            "client never announced its control endpoint: {out}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };

    let ctl = ws_connect(ctl_port);
    ws_send(&ctl, "D!:bp tick_step");
    assert_eq!(ws_recv(&ctl), "D:ok bp tick_step");
    let hit = ws_recv(&ctl); // the next tick pauses
    assert!(
        hit.starts_with("D:hit tick_step") && hit.contains("c="),
        "hit with bindings: {hit}"
    );
    ws_send(&ctl, "D!:eval c");
    let ev = ws_recv(&ctl);
    assert!(
        ev.starts_with("D:eval c=") && ev.contains("ticks"),
        "frame eval: {ev}"
    );
    ws_send(&ctl, "D!:resume");
    assert_eq!(ws_recv(&ctl), "D:resumed");
    // Still armed, still serving: the NEXT tick hits again.
    let hit2 = ws_recv(&ctl);
    assert!(hit2.starts_with("D:hit tick_step"), "re-hit: {hit2}");
    ws_send(&ctl, "D!:quit");
    assert_eq!(ws_recv(&ctl), "D:quitting");
    // The client process exits on quit.
    let deadline = vm_deadline(10);
    loop {
        if let Some(child) = cguard.0.as_mut()
            && child.try_wait().ok().flatten().is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "client never exited on D!:quit");
        std::thread::sleep(Duration::from_millis(100));
    }
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    assert!(
        stderr.contains("loft-debug: paused in tick_step"),
        "the pause must be real:\n{stderr}"
    );
    let _ = std::fs::remove_file(&srv_prog);
    let _ = std::fs::remove_file(&cli_prog);
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
}

/// @PLN18 08 scenario S8 — THE STANDING DIFFERENTIAL: one meaning scenario
/// executed in four tier states — interpreted, compiled, mixed (the S2/S3
/// standing state: a fn flipped to the interpreter), and post-swap (the
/// process REPLACED mid-sequence via a control-channel self-swap) — with
/// byte-equal game transcripts.  Goal D's sweep extended to the mixed
/// states; it pins "a target change is observable only as speed"
/// permanently.  Positive controls per leg: the mixed leg must really
/// dispatch through the interpreter; the swap leg must really restore the
/// world across a new process.
// @speed 1.1
#[test]
fn s8_standing_four_state_differential() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let legs = [
        ("interpreted", 18110u16),
        ("compiled", 18111u16),
        ("mixed", 18112u16),
        ("post-swap", 18113u16),
    ];
    let mut transcripts = Vec::new();
    for (mode, port) in legs {
        transcripts.push((mode, run_s8_leg(mode, port)));
    }
    let want = vec!["got:a#1", "got:b#2", "got:c#3", "got:d#4"];
    for (mode, t) in &transcripts {
        assert_eq!(
            t, &want,
            "S8: the {mode} tier must produce the canonical transcript"
        );
    }
}

fn run_s8_leg(mode: &str, port: u16) -> Vec<String> {
    let stem = format!("/.loft/cache/eh_s8_{port}-");
    s5_kill_stale(&stem);
    let fixture = format!(
        r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn bump_events(w: W) -> integer {{
  w.events = w.events + 1;
  w.events
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  resumed = engine_host::swap_world(w);
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      engine_host::send(ev.cid, "got:{{ev.payload}}#{{n}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
    );
    let prog = test_tmp().join(format!("eh_s8_{port}.loft"));
    std::fs::write(&prog, &fixture).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = test_tmp().join(format!("eh_s8_{port}.err"));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1"); // hermetic fixtures
    match mode {
        "interpreted" => {
            cmd.arg("--interpret");
        }
        "compiled" => {}
        "mixed" => {
            cmd.env("LOFT_LIVE_FLIP", "1")
                .env("LOFT_FLIP_FNS", "bump_events")
                .env("LOFT_DISPATCH_DEBUG", "1");
        }
        "post-swap" => {
            cmd.env("LOFT_LIVE_FLIP", "1")
                .env("LOFT_DEBUG_CONTROL", "1");
        }
        _ => unreachable!(),
    }
    cmd.process_group(0);
    let child = cmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));
    let _hygiene_kill = scopeguard_kill(stem.clone());

    let mut replies = Vec::new();
    let ws = ws_connect(port);
    ws_send(&ws, "a");
    replies.push(ws_recv(&ws));
    ws_send(&ws, "b");
    replies.push(ws_recv(&ws));

    let tail_ws = if mode == "post-swap" {
        // THE SELF-SWAP between b and c: rebuild the UNCHANGED source (a
        // cache hit on the artifact this very process runs), swap to it —
        // a new process under the same world — then finish the sequence.
        let ctl = ws_connect(port);
        ws_send(&ctl, "D!:rebuild");
        assert_eq!(ws_recv(&ctl), "D:rebuild started");
        let deadline = vm_deadline(120);
        loop {
            ws_send(&ctl, "D!:rebuild?");
            let st = ws_recv(&ctl);
            if st == "D:rebuild 2" {
                break;
            }
            assert!(Instant::now() < deadline, "self-rebuild never ready");
            std::thread::sleep(Duration::from_millis(200));
        }
        ws_send(&ctl, "D!:swap auto");
        assert_eq!(ws_recv(&ctl), "D:swap true");
        ws.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        while ws_try_recv(&ws).is_some() {}
        let reconnect_deadline = vm_deadline(10);
        loop {
            if let Some(nws) = ws_try_connect(port) {
                break nws;
            }
            assert!(
                Instant::now() < reconnect_deadline,
                "never reconnected after the self-swap"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    } else {
        ws
    };
    ws_send(&tail_ws, "c");
    replies.push(ws_recv(&tail_ws));
    ws_send(&tail_ws, "d");
    replies.push(ws_recv(&tail_ws));
    drop(_guard);

    // Per-leg positive controls: the tier state must be REAL, not silent.
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    match mode {
        "mixed" => assert!(
            stderr.contains("live-dispatch: n_bump_events"),
            "the mixed leg must dispatch through the interpreter:\n{stderr}"
        ),
        "post-swap" => {
            assert!(
                stderr.contains("loft-swap: world restored"),
                "the swap leg must restore the world:\n{stderr}"
            );
            assert!(
                stderr.contains("handing over"),
                "the swap leg must hand over:\n{stderr}"
            );
        }
        _ => {}
    }
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
    replies
}

/// Kill any leftover process running the leg's cache binary at scope end
/// (the swap child outlives the Guard's process group by design).
fn scopeguard_kill(stem: String) -> impl Drop {
    struct K(String);
    impl Drop for K {
        fn drop(&mut self) {
            s5_kill_stale(&self.0);
        }
    }
    K(stem)
}

/// @PLN18 08 scenario S7 — the debugger loop end-to-end (@PLN16 6b/6c
/// convergence): a scripted control-channel session against a serving
/// compiled kernel, asserting each stage IN ORDER — breakpoint hit with
/// frame bindings (the pause's mini-pump keeps mechanics alive), frame
/// eval, live edit acknowledged through the pause (the S3 poll-now),
/// resume, the SECOND hit proving breakpoint re-resolution across the
/// reload (offsets moved, identity did not), rebuild driven over the
/// channel, swap — and a post-swap breakpoint hitting in the NEW build
/// over the RESTORED world.
// @speed 2.4
#[test]
fn s7_debugger_loop_end_to_end() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    // The canonical port for the prior run's orphan, the resolved one for our own child —
    // see `s5_native_swap_under_running_world` for why they are two different numbers.
    s5_kill_stale(&format!("/.loft/cache/eh_s7_{}-", common::test_port(18108)));
    std::thread::sleep(Duration::from_millis(200));
    let port = common::bind_port(18108);
    reap_port(port); // reap a leaked swap-child orphan the stem pgrep misses (flake guard)
    let _hygiene = S5Hygiene(format!("/.loft/cache/eh_s7_{port}-"));
    // The edit touches ONLY the named fn (lambdas don't reload — the
    // documented v1 boundary); each build's identity shows in the STEP:
    // +1 = original, +100 = the edit (and post-swap, its compiled form).
    let fixture = |step: u32| {
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn bump_events(w: W) -> integer {{
  w.events = w.events + {step};
  w.events
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  resumed = engine_host::swap_world(w);
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      engine_host::send(ev.cid, "got:{{ev.payload}}#{{n}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        )
    };
    let prog = test_tmp().join(format!("eh_s7_{port}.loft"));
    std::fs::write(&prog, fixture(1)).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = test_tmp().join(format!("eh_s7_{port}.err"));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1"); // hermetic fixtures
    cmd.env("LOFT_LIVE_FLIP", "1")
        .env("LOFT_LIVE_RELOAD", "1")
        .env("LOFT_DEBUG_CONTROL", "1")
        .env("LOFT_DISPATCH_DEBUG", "1");
    cmd.process_group(0);
    let child = cmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));

    let ctl = ws_connect(port);
    let game = ws_connect(port);

    // Stage 1: breakpoint set over the control channel (sets + flips).
    ws_send(&ctl, "D!:bp bump_events");
    assert_eq!(ws_recv(&ctl), "D:ok bp bump_events");

    // Stage 2: the hit, with frame bindings (the game's reply is HELD —
    // the dispatch is paused; mechanics stay alive underneath).
    ws_send(&game, "a");
    let hit = ws_recv(&ctl);
    assert!(
        hit.starts_with("D:hit bump_events") && hit.contains("w="),
        "first hit with bindings: {hit}"
    );

    // Stage 3: evaluate a frame variable at the pause.
    ws_send(&ctl, "D!:eval w");
    let ev = ws_recv(&ctl);
    assert!(
        ev.starts_with("D:eval w=") && ev.contains("events"),
        "frame eval: {ev}"
    );

    // Stage 4: live edit THROUGH the pause (the S3 poll-now finding),
    // structured ack.  The 200 ms reload throttle needs the edit to settle.
    std::fs::write(&prog, fixture(100)).unwrap();
    std::thread::sleep(Duration::from_millis(350));
    ws_send(&ctl, "D!:reload");
    assert_eq!(ws_recv(&ctl), "D:reload applied");

    // Stage 5: resume — the held call completes on the OLD body (append-only).
    ws_send(&ctl, "D!:resume");
    assert_eq!(ws_recv(&ctl), "D:resumed");
    assert_eq!(ws_recv(&game), "got:a#1");

    // Stage 6: the next call runs the NEW body AND hits again — breakpoint
    // re-resolution across the reload (offsets moved, identity did not).
    ws_send(&game, "b");
    let hit2 = ws_recv(&ctl);
    assert!(
        hit2.starts_with("D:hit bump_events"),
        "re-resolved hit: {hit2}"
    );
    ws_send(&ctl, "D!:resume");
    assert_eq!(ws_recv(&ctl), "D:resumed");
    assert_eq!(ws_recv(&game), "got:b#101");

    // Stage 7: rebuild over the channel; poll to ready.
    ws_send(&ctl, "D!:rebuild");
    assert_eq!(ws_recv(&ctl), "D:rebuild started");
    let deadline = vm_deadline(300);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        ws_send(&ctl, "D!:rebuild?");
        let st = ws_recv(&ctl);
        if st == "D:rebuild 2" {
            break;
        }
        assert!(st == "D:rebuild 1", "rebuild status: {st}");
        assert!(Instant::now() < deadline, "rebuild never ready");
    }

    // Stage 8: swap over the channel; both seats reconnect into the new
    // build and the debugger RE-ARMS — still debuggable after the swap.
    ws_send(&ctl, "D!:swap auto");
    assert_eq!(ws_recv(&ctl), "D:swap true");
    ctl.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    while ws_try_recv(&ctl).is_some() {}
    let reconnect_deadline = vm_deadline(10);
    let (ctl2, game2) = loop {
        if let (Some(c), Some(g)) = (ws_try_connect(port), ws_try_connect(port)) {
            break (c, g);
        }
        assert!(
            Instant::now() < reconnect_deadline,
            "never reconnected into the new build"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    ws_send(&ctl2, "D!:bp bump_events");
    assert_eq!(ws_recv(&ctl2), "D:ok bp bump_events");

    // Stage 9: the post-swap hit, over the RESTORED world (events=101).
    ws_send(&game2, "c");
    let hit3 = ws_recv(&ctl2);
    assert!(
        hit3.starts_with("D:hit bump_events") && hit3.contains("101"),
        "post-swap hit over the restored world: {hit3}"
    );
    ws_send(&ctl2, "D!:resume");
    assert_eq!(ws_recv(&ctl2), "D:resumed");
    assert_eq!(ws_recv(&game2), "got:c#201");

    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    assert!(
        stderr.contains("loft-debug: paused in bump_events"),
        "{stderr}"
    );
    assert!(stderr.contains("loft-swap: world restored"), "{stderr}");
    drop(_guard);
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
}

/// @PLN18 08 scenario S4 — the background rebuild: the serve host compiles
/// the full project (a real rustc run) while the OLD build keeps serving.
/// Asserts the design's three clauses:
/// (a) the artifact lands and corresponds to the source — a repeat request
///     on unchanged source is an instant cache hit on the SAME path;
/// (b) build isolation — the tick counter keeps advancing in every poll
///     interval while rustc runs (probe gate 4);
/// (c) an edit DURING the build invalidates it — "requeued" on stderr, and
///     the final artifact is the settled source's build.
/// The during-build edit sets the source back to the spawn content; the
/// cache keeps ONE binary per source stem, so the unique-content build
/// evicts it and the requeued build is a second real rustc — both builds
/// are measurement windows for (b).
// @speed 2.3
#[test]
fn s4_background_rebuild_under_serving_kernel() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = common::bind_port(18099);
    // Tick step: unique per run -> the rebuild is ALWAYS a cache miss (a
    // real rustc window for (b)/(c)); behavior-equal (ticks just advance).
    let unique_step = (std::process::id() % 1000) + 2;
    let fixture = |tick_step: u32| {
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      if ev.payload == "rebuild" {{
        engine_host::send(ev.cid, "rebuild:{{engine_host::rebuild_start()}}");
        return;
      }}
      if ev.payload == "status" {{
        engine_host::send(ev.cid, "status:{{engine_host::rebuild_status()}}");
        return;
      }}
      if ev.payload == "artifact" {{
        engine_host::send(ev.cid, "artifact:{{engine_host::rebuild_artifact()}}");
        return;
      }}
      engine_host::send(ev.cid, "ticks:{{w.ticks}}");
    }},
    fn() {{ w.ticks = w.ticks + {tick_step}; }});
}}
"#
        )
    };
    let prog = test_tmp().join(format!("eh_s4_{port}.loft"));
    std::fs::write(&prog, fixture(1)).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = test_tmp().join(format!("eh_s4_{port}.err"));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1"); // hermetic fixtures
    cmd.process_group(0);
    let child = cmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));

    let ws = ws_connect(port);
    let ask = |msg: &str| -> String {
        ws_send(&ws, msg);
        ws_recv(&ws)
    };
    let ticks = |reply: String| -> i64 {
        reply
            .strip_prefix("ticks:")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("unparseable {reply:?}"))
    };

    // ── Phase 1 (b): a clean unique-content rebuild — a real rustc window
    // with no interference; every poll interval must show tick progress.
    // (The running v0 binary is unaffected: reload is OFF.)
    std::fs::write(&prog, fixture(unique_step)).unwrap();
    assert_eq!(ask("rebuild"), "rebuild:true");
    let mut last_ticks = ticks(ask("t"));
    let deadline = vm_deadline(300);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now_ticks = ticks(ask("t"));
        assert!(
            now_ticks > last_ticks,
            "tick stalled during the background build (build isolation broken)"
        );
        last_ticks = now_ticks;
        let st = ask("status");
        if st == "status:2" {
            break;
        }
        assert!(
            st == "status:1",
            "rebuild must stay building/ready, got {st}"
        );
        assert!(Instant::now() < deadline, "rebuild never became ready");
    }

    // ── Phase 2 (c): an edit DURING a build invalidates it.  The snapshot
    // is taken at request time, so wherever the edit lands relative to the
    // child's own file read, completion sees drift -> requeue.  The settled
    // content (v0) is already cached -> the requeued build converges fast.
    std::fs::write(&prog, fixture(unique_step + 1)).unwrap();
    assert_eq!(ask("rebuild"), "rebuild:true");
    std::fs::write(&prog, fixture(1)).unwrap();
    let deadline = vm_deadline(300);
    loop {
        let st = ask("status");
        if st == "status:2" {
            break;
        }
        assert!(
            st == "status:1",
            "requeue path must stay building, got {st}"
        );
        assert!(Instant::now() < deadline, "requeued rebuild never ready");
        std::thread::sleep(Duration::from_millis(300));
    }
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    assert!(
        stderr.contains("rebuild stale (source changed during build) — requeued"),
        "the mid-build edit must requeue:\n{stderr}"
    );
    // (a) the artifact exists and is the settled source's build.
    let artifact = ask("artifact");
    let path = artifact.strip_prefix("artifact:").unwrap().to_string();
    assert!(!path.is_empty(), "ready artifact must have a path");
    assert!(
        std::path::Path::new(&path).exists(),
        "artifact must exist: {path}"
    );
    // (a) repeat request on unchanged source: instant cache hit, SAME path.
    assert_eq!(ask("rebuild"), "rebuild:true");
    let deadline = vm_deadline(60);
    loop {
        let st = ask("status");
        if st == "status:2" {
            break;
        }
        assert!(Instant::now() < deadline, "cache-hit rebuild never ready");
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(
        ask("artifact"),
        format!("artifact:{path}"),
        "unchanged source must yield the SAME artifact (hash-stable)"
    );
    drop(_guard);
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
}

/// @PLN18 08 scenario S2 — the debugger pushes one fn to the interpreter,
/// LIVE, under a serving kernel.  The flip target (`bump_events(w: W) ->
/// integer`) mutates the shared world; the event counter must count
/// monotonically across compiled -> interp -> compiled (state continuity is
/// the whole claim).  The flip is driven by a CONTROL INPUT (a WS message ->
/// `live_flip`), and is observable ONLY via the `LOFT_DISPATCH_DEBUG`
/// sentinel — the meaning transcript is byte-identical to the interpreted
/// run of the same fixture (the differential leg).
#[test]
fn s2_live_flip_under_serving_kernel() {
    let native = run_s2_scenario(18095, false);
    let interp = run_s2_scenario(18096, true);
    assert_eq!(
        native.replies, interp.replies,
        "S2 differential: a tier flip must be meaning-invisible"
    );
    assert!(
        native.stderr.contains("live-dispatch: n_bump_events"),
        "the native leg must really dispatch through the interpreter:\n{}",
        native.stderr
    );
    assert!(
        !interp.stderr.contains("live-dispatch"),
        "the interpreted leg has no live dispatch"
    );
}

struct S2Run {
    replies: Vec<String>,
    stderr: String,
}

fn run_s2_scenario(port: u16, interpret: bool) -> S2Run {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return S2Run {
            replies: Vec::new(),
            stderr: String::new(),
        };
    }
    let prog = test_tmp().join(format!("eh_s2_{port}_{}.loft", std::process::id()));
    std::fs::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn bump_events(w: W) -> integer {{
  w.events = w.events + 1;
  w.events
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      if ev.payload == "flip" {{
        engine_host::live_flip("bump_events", true);
        engine_host::send(ev.cid, "flip-ack#{{n}}");
        return;
      }}
      if ev.payload == "unflip" {{
        engine_host::live_flip("bump_events", false);
        engine_host::send(ev.cid, "unflip-ack#{{n}}");
        return;
      }}
      engine_host::broadcast("got:{{ev.payload}}#{{n}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = test_tmp().join(format!("eh_s2_{port}_{}.err", std::process::id()));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1"); // hermetic fixtures
    if interpret {
        cmd.arg("--interpret");
    } else {
        cmd.env("LOFT_LIVE_FLIP", "1")
            .env("LOFT_DISPATCH_DEBUG", "1");
    }
    cmd.process_group(0); // own group, so Guard can kill driver + grandchild
    let child = cmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));

    let ws = ws_connect(port);
    let mut replies = Vec::new();
    // compiled -> flip -> interp -> unflip -> compiled; the counter must
    // cross every tier boundary without a gap or repeat (1..=5).
    for msg in ["a", "flip", "b", "unflip", "c"] {
        ws_send(&ws, msg);
        replies.push(ws_recv(&ws));
    }
    assert_eq!(
        replies,
        vec![
            "got:a#1",
            "flip-ack#2",
            "got:b#3",
            "unflip-ack#4",
            "got:c#5"
        ],
        "the world's counter must be continuous across tier flips ({})",
        if interpret { "interp" } else { "native+flip" }
    );
    drop(_guard); // kill now so the stderr file is complete
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
    S2Run { replies, stderr }
}

fn run_kernel_scenario(port: u16, interpret: bool) {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let prog = test_tmp().join(format!("eh_kernel_{port}_{}.loft", std::process::id()));
    std::fs::write(
        &prog,
        format!(
            r#"
use engine_host;

struct W {{ events: integer not null, ticks: integer not null }}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      w.events = w.events + 1;
      if ev.payload == "stats" {{
        engine_host::send(ev.cid, "stats:events={{w.events}},ticks_pos={{w.ticks > 0}}");
        return;
      }}
      engine_host::broadcast("got:{{ev.payload}}#{{w.events}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1"); // hermetic fixtures
    if interpret {
        cmd.arg("--interpret");
    }
    cmd.process_group(0); // own group, so Guard can kill driver + grandchild
    let child = cmd
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

    let ws = ws_connect(port);
    // Two round trips: handler closure capture (events counter) must advance.
    ws_send(&ws, "hello");
    assert_eq!(ws_recv(&ws), "got:hello#1");
    ws_send(&ws, "again");
    assert_eq!(ws_recv(&ws), "got:again#2");
    // Give the 10ms tick a beat, then ask for stats: ticks must be counting.
    std::thread::sleep(Duration::from_millis(100));
    ws_send(&ws, "stats");
    let stats = ws_recv(&ws);
    assert_eq!(
        stats, "stats:events=3,ticks_pos=true",
        "captures + drift-free ticks live: {stats}"
    );
    let _ = std::fs::remove_file(&prog);
}

/// `run_local` — the standalone windowed-host entry: the SAME client loop
/// with NO transport.  The program must tick (drift-free clock), exit via
/// `client_stop()` from inside `on_tick`, and `client_send` must honestly
/// report false (there is no peer).  Both backends: the native leg proves
/// the `n_kernel_local` typed twin + registration.
#[test]
fn run_local_ticks_and_stops_without_a_server() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let fixture = r#"use engine_host;

fn main() {
    frames = 0;
    engine_host::run_local(1000,
        fn(ev: engine_host::Event) { println("ev kind={ev.kind}"); },
        fn() {
            frames += 1;
            if frames >= 5 {
                engine_host::client_stop();
            }
        });
    sent = engine_host::client_send("1:hello");
    println("done frames={frames} sent={sent}");
}
"#;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for interpret in [true, false] {
        let prog =
            std::env::temp_dir().join(format!("eh_local_{}_{interpret}.loft", std::process::id()));
        std::fs::write(&prog, fixture).unwrap();
        let mut cmd = Command::new(loft_bin());
        cmd.env("LOFT_OFFLINE", "1");
        if interpret {
            cmd.arg("--interpret");
        }
        let out = cmd
            .arg("--no-warnings")
            .arg("--lib")
            .arg(root.join("lib"))
            .arg(&prog)
            .current_dir(&root)
            .output()
            .expect("run local kernel");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("done frames=5 sent=false"),
            "interpret={interpret}: expected 5 ticks then a clean stop, got:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_file(&prog);
    }
}

/// The K2 flow-back trio: `post` (a local event reaches `on_event` with
/// `cid: -1` — window input as the events class), the listener's `stop()`
/// (`run` returns — the windowed listener's window-close exit), and the
/// listener loop's per-turn `kernel_frame()` riding every turn (a no-op on
/// native; exercised by the loop completing).  One self-stopping program per
/// role, both backends.
// @speed 1.3
#[test]
fn post_and_stop_in_both_roles() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    // `head`: the boot call up to the tick arg (run_local takes no port;
    // run listens).  `exit`: client_stop leaves the connector loop, stop
    // the listener loop.
    let shape = |head: &str, exit: &str| {
        format!(
            r#"use engine_host;

fn main() {{
    frames = 0;
    got = "";
    engine_host::{head},
        fn(ev: engine_host::Event) {{
            got = "{{got}}[{{ev.cid}}:{{ev.kind}}:{{ev.payload}}]";
        }},
        fn() {{
            frames += 1;
            if frames == 2 {{
                engine_host::post("K:input");
            }}
            if frames >= 4 {{
                engine_host::{exit}();
            }}
        }});
    println("done frames={{frames}} got={{got}}");
}}
"#
        )
    };
    let local = shape("run_local(1000", "client_stop");
    // Through `bind_port` like every other server test in this file: a hard-coded port
    // cannot be moved by `LOFT_TEST_PORT_OFFSET`, so a second checkout — or any other
    // process on the box that happens to hold it — fails this test with "cannot listen
    // on port …" and nothing pointing at the port as the cause.
    let listener = shape(&format!("run({}, 1000", common::bind_port(18099)), "stop");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for (tag, src) in [("local", &local), ("listener", &listener)] {
        for interpret in [true, false] {
            let prog = std::env::temp_dir().join(format!(
                "eh_post_{tag}_{}_{interpret}.loft",
                std::process::id()
            ));
            std::fs::write(&prog, src).unwrap();
            let mut cmd = Command::new(loft_bin());
            cmd.env("LOFT_OFFLINE", "1");
            if interpret {
                cmd.arg("--interpret");
            }
            let out = cmd
                .arg("--no-warnings")
                .arg("--lib")
                .arg(root.join("lib"))
                .arg(&prog)
                .current_dir(&root)
                .output()
                .expect("run kernel role");
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                stdout.contains("done frames=4 got=[-1:1:K:input]"),
                "{tag} interpret={interpret}: post must arrive with local origin, \
                 then the loop must stop, got:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let _ = std::fs::remove_file(&prog);
        }
    }
}

/// @PLN18 08-S5, LOCAL role (#352) — the build swap completes for a
/// `run_local` (windowed, transportless) program: the snapshot/ready
/// protocol is file-based, so "serving" for a local kernel is BOOTED
/// (`local_init` touches the ready file).  The whole cycle in one run:
/// rebuild → swap_start → the new incarnation restores the world
/// (`RESUMED`) → the old loop exits with `swap_retired() == true` → the
/// resumed process runs on and exits by itself.
// @speed 2.8
#[test]
fn s5_local_swap_hands_over() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let _hygiene = S5Hygiene("eh_s5local".to_string());
    let fixture = r#"use engine_host;

struct World { ticks: integer, gen: integer }

fn main() {
    w = World { ticks: 0, gen: 1 };
    resumed = engine_host::swap_world(w);
    if resumed {
        println("RESUMED ticks={w.ticks}");
        w.gen += 1;
    }
    started = false;
    swapped = false;
    engine_host::run_local(20000,
        fn(ev: engine_host::Event) {},
        fn() {
            w.ticks += 1;
            if w.ticks == 5 && !started && w.gen == 1 {
                started = engine_host::rebuild_start();
            }
            if started && !swapped && engine_host::rebuild_status() == 2 {
                swapped = engine_host::swap_start(engine_host::rebuild_artifact());
            }
            if w.ticks >= 100 {
                engine_host::client_stop();
            }
        });
    println("exited gen={w.gen} retired={engine_host::swap_retired()}");
}
"#;
    let prog = std::env::temp_dir().join(format!("eh_s5local_{}.loft", std::process::id()));
    std::fs::write(&prog, fixture).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new(loft_bin())
        .env("LOFT_OFFLINE", "1")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .output()
        .expect("run local swap cycle");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("RESUMED"),
        "the new incarnation must restore the world (#352).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("exited gen=1 retired=true"),
        "the OLD local loop must observe the handover (#352).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("exited gen=2 retired=false"),
        "the resumed incarnation must run on and exit by itself.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let _ = std::fs::remove_file(&prog);
}

/// F11-3 regression — `parallel_ctx` is program-scoped: a re-entered
/// dispatch's completion must NOT tear it down.  Before the fix,
/// `State::resume()` cleared the ctx the live-dispatch host wired at boot,
/// so the SECOND call of a flipped fn using `par_*` panicked in
/// `n_parallel_*`'s expect ("called outside State::execute()") and the
/// kernel died.  Three pings — each runs `par_fold` inside the flipped fn.
#[test]
fn s2_flipped_fn_with_par_survives_repeat_dispatch() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = common::bind_port(18121);
    let prog = test_tmp().join(format!("eh_f11par_{port}_{}.loft", std::process::id()));
    std::fs::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn add_two(a: integer, b: integer) -> integer {{ a + b }}
fn bump_events(w: W) -> integer {{
  items: vector<integer> = [10, 20, 30, 40];
  s = par_fold(items, 0, add_two, 2);
  w.events = w.events + 1;
  w.events * 1000 + s
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      engine_host::send(ev.cid, "got:{{ev.payload}}#{{n}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1")
        .env("LOFT_LIVE_RELOAD", "1")
        .env("LOFT_LIVE_FLIP", "1")
        .env("LOFT_FLIP_FNS", "bump_events");
    cmd.process_group(0);
    let child = cmd
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

    let ws = ws_connect(port);
    for i in 1..=3i64 {
        ws_send(&ws, "p");
        let r = ws_recv(&ws);
        // events*1000 + par sum (10+20+30+40): 1100, 2100, 3100.
        assert_eq!(
            r,
            format!("got:p#{}", i * 1000 + 100),
            "dispatch {i} (par_fold inside the flipped fn) must keep working"
        );
    }
    let _ = std::fs::remove_file(&prog);
}

// @PLN98 — the debugger driven THROUGH A GAME-SERVER SETUP: a running engine_host
// kernel (the server) with the live tier, debugged over the SAME WebSocket a
// client connects on, via the REAL `D!:` control channel (debug_cmd_dispatch +
// debug_pause_loop). A client sets a breakpoint on a flipped game fn; a game event
// triggers it; the server PAUSES (`D:hit` carries the live frame); the client evals
// a frame local and resumes; the game continues with the right result. This is the
// server-side of the browser relay, end-to-end on loopback — no browser needed.
#[test]
fn debugger_drives_a_running_game_server_over_websocket() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = common::bind_port(19312);
    let prog = test_tmp().join(format!("eh_dbg_{port}_{}.loft", std::process::id()));
    std::fs::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn hit_me(w: W, delta: integer) -> integer {{
  w.events = w.events + delta;
  w.events
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = hit_me(w, 7);
      engine_host::broadcast("got#{{n}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1")
        .env("LOFT_LIVE_FLIP", "1")
        .env("LOFT_FLIP_FNS", "hit_me")
        .env("LOFT_DEBUG_CONTROL", "1")
        .process_group(0)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = cmd.spawn().expect("spawn kernel");
    let _guard = Guard(Some(child));

    let ws = ws_connect(port);
    let recv_until = |ws: &TcpStream, needle: &str| -> String {
        let deadline = vm_deadline(15);
        loop {
            let m = ws_recv(ws);
            if m.contains(needle) {
                return m;
            }
            assert!(
                Instant::now() < deadline,
                "never saw {needle:?} on the debug channel"
            );
        }
    };

    // Set a breakpoint on the flipped game fn over the D!: control channel.
    ws_send(&ws, "D!:bp hit_me");
    recv_until(&ws, "D:ok bp hit_me");
    // A game event triggers hit_me -> the SERVER pauses at the breakpoint, and the
    // `D:hit` frame carries the live frame's locals (delta the caller passed).
    ws_send(&ws, "p");
    let hit = recv_until(&ws, "D:hit");
    assert!(
        hit.contains("hit_me") && hit.contains("delta=7"),
        "server paused with the live frame (delta=7): {hit}"
    );
    // Eval a frame local over the paused server, then resume.
    ws_send(&ws, "D!:eval delta");
    assert!(
        recv_until(&ws, "D:eval").contains("delta=7"),
        "eval delta == 7 over the paused game server"
    );
    ws_send(&ws, "D!:resume");
    recv_until(&ws, "D:resumed");
    // The game RESUMED and ran to the broadcast with the right result.
    assert!(
        recv_until(&ws, "got#").contains("got#7"),
        "the game continued after resume with got#7"
    );
    let _ = std::fs::remove_file(&prog);
}

// @PLN98 P3.4 (item 2) — the SERVER per-name debug RELAY: an agent debugs a
// SEPARATE named client THROUGH the game server, over the WebSockets both hold.
// A client registers `D!:iam <name>`; the agent sends `D!:@<name>:<cmd>`; the
// server forwards `D!:<cmd>` to that client's socket and relays the client's
// `D!:reply <msg>` back to the agent. This is the agent->server->client hop the
// browser debugger needs (the client half is verified on wasm; here the client is
// a native WS peer standing in for it).
#[test]
fn server_relays_debug_frames_to_a_named_client() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = common::bind_port(19318);
    let prog = test_tmp().join(format!("eh_relay_{port}_{}.loft", std::process::id()));
    std::fs::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ ticks: integer not null }}
fn main() {{
  w = W {{ ticks: 0 }};
  engine_host::run({port}, 100000,
    fn(ev: engine_host::Event) {{ if ev.kind != 1 {{ return; }} }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_OFFLINE", "1")
        .env("LOFT_DEBUG_CONTROL", "1")
        .process_group(0)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = cmd.spawn().expect("spawn kernel");
    let _guard = Guard(Some(child));

    let recv_until = |ws: &TcpStream, needle: &str| -> String {
        let deadline = vm_deadline(15);
        loop {
            let m = ws_recv(ws);
            if m.contains(needle) {
                return m;
            }
            assert!(Instant::now() < deadline, "never saw {needle:?}");
        }
    };

    // The CLIENT (standing in for the browser) registers its debug name.
    let client = ws_connect(port);
    ws_send(&client, "D!:iam alice");
    recv_until(&client, "D:registered alice");

    // The AGENT forwards a debug command addressed to that client BY NAME.
    let agent = ws_connect(port);
    ws_send(&agent, "D!:@alice:bp tick");
    recv_until(&agent, "D:relayed alice"); // the server accepted + forwarded

    // The client receives the FORWARDED frame and answers with a D:reply, which
    // the server routes back to the agent.
    let fwd = recv_until(&client, "D!:bp tick");
    assert!(
        fwd.contains("D!:bp tick"),
        "client got the forwarded frame: {fwd}"
    );
    ws_send(&client, "D!:reply D:ok bp tick applied-at-client");
    let relayed = recv_until(&agent, "applied-at-client");
    assert!(
        relayed.contains("D:ok bp tick applied-at-client"),
        "the client's reply was relayed back to the agent: {relayed}"
    );
    let _ = std::fs::remove_file(&prog);
}

// ── The browser kernel's native registry ────────────────────────────────────
//
// `KERNEL_FUNCTIONS_WASM` is a hand-maintained list, and what it registers has to agree with
// two other things nothing was checking: the implementations beside it, and the natives the
// shared loop calls.  loft#1189 was one defect of each kind — `n_kernel_client_event_status`
// was implemented in `engine_host::browser` and never listed, and `n_kernel_swap_step` was
// called by `client_loop` on every turn and neither implemented nor listed.  Both showed up
// only as a panicking stub inside a browser page, and the committed bundle was old enough to
// predate them, so the browser tests never saw either.
//
// The two guards below are exact — a name is in a list or it is not — rather than a call-graph
// walk, which over-approximates: followed transitively from the client roots it reaches the
// SERVER family through the role-generic wrappers and would demand `n_kernel_listen` be
// registered for a browser.

/// The `#native` symbol each `fn name(...)` in the lib maps to, plus each `pub fn n_*` the
/// browser module defines and each name `KERNEL_FUNCTIONS_WASM` registers.
fn browser_kernel_sources() -> (String, String, String) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    (
        std::fs::read_to_string(root.join("lib/engine_host/src/engine_host.loft"))
            .expect("engine_host.loft"),
        std::fs::read_to_string(root.join("src/engine_host.rs")).expect("engine_host.rs"),
        std::fs::read_to_string(root.join("src/native.rs")).expect("native.rs"),
    )
}

/// The names `KERNEL_FUNCTIONS_WASM` registers, read out of the source text.
///
/// The table is `cfg(target_arch = "wasm32")`, so a host test cannot look at the value — the
/// text is the only view of it from here.
fn registered_for_wasm(native_rs: &str) -> std::collections::HashSet<String> {
    let start = native_rs
        .find("pub const KERNEL_FUNCTIONS_WASM")
        .expect("KERNEL_FUNCTIONS_WASM");
    let end = native_rs[start..].find("\n];").expect("end of the table") + start;
    native_rs[start..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|s| s.starts_with("n_"))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_browser_kernel_native_is_registered_for_wasm() {
    let (_, engine_host_rs, native_rs) = browser_kernel_sources();
    let registered = registered_for_wasm(&native_rs);
    let open = engine_host_rs
        .find("\npub mod browser {")
        .expect("`pub mod browser`");
    // The module ends at the first `}` in column zero after it — every item inside is
    // indented.  Without the bound this read on past the module and collected the NATIVE and
    // codegen implementations that follow it, which a browser build has no business with.
    let close = engine_host_rs[open + 1..]
        .find("\n}")
        .map_or(engine_host_rs.len(), |i| open + 1 + i);
    let module = &engine_host_rs[open..close];
    // The module's own items are indented four spaces; anything deeper belongs to a nested
    // block and anything at column zero is past its closing brace.
    let mut missing = Vec::new();
    for line in module.lines() {
        let Some(rest) = line.strip_prefix("    pub fn n_") else {
            continue;
        };
        let Some(name) = rest.split('(').next().map(str::trim) else {
            continue;
        };
        let sym = format!("n_{name}");
        if !registered.contains(&sym) {
            missing.push(sym);
        }
    }
    assert!(
        missing.is_empty(),
        "`engine_host::browser` implements {missing:?} and `KERNEL_FUNCTIONS_WASM` does not \
         register them — a browser page reaches the panicking stub instead (loft#1189).  Add \
         each to the table in src/native.rs, or delete the implementation."
    );
}

#[test]
fn the_shared_client_loop_calls_only_natives_the_browser_kernel_has() {
    let (lib, _, native_rs) = browser_kernel_sources();
    let registered = registered_for_wasm(&native_rs);
    // The natives the lib declares, by loft name: `fn kernel_x(…) -> τ;` followed by `#native`
    // (with an explicit symbol, or `n_` + the name).
    let lines: Vec<&str> = lib.lines().collect();
    let mut symbol_of: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t != "#native" && !t.starts_with("#native \"") {
            continue;
        }
        let explicit = t
            .strip_prefix("#native \"")
            .and_then(|r| r.split('"').next());
        let Some(decl) = lines[..i].iter().rev().find_map(|l| {
            let l = l.trim_start();
            let l = l.strip_prefix("pub ").unwrap_or(l);
            l.strip_prefix("fn ").and_then(|r| r.split('(').next())
        }) else {
            continue;
        };
        symbol_of.insert(
            decl,
            explicit.map_or_else(|| format!("n_{decl}"), str::to_string),
        );
    }
    // `client_loop` is THE loop every browser page runs, and its body is scanned directly
    // rather than followed transitively: what it calls itself is exactly what a page reaches
    // on every turn, and it is decidable without guessing.
    let start = lib
        .find("fn client_loop(")
        .expect("`client_loop` in engine_host.loft");
    let body = &lib[start..];
    let end = body
        .find("\nfn ")
        .or_else(|| body.find("\npub fn "))
        .unwrap_or(body.len());
    let body = &body[..end];
    // A call is the name at a word boundary: a bare `contains` reported `client_loop` calling
    // `sync_next` because it calls `client_sync_next`, and the guard's whole value is that its
    // failures mean something.
    let calls = |name: &str| {
        let needle = format!("{name}(");
        body.match_indices(&needle).any(|(at, _)| {
            at == 0
                || !body[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    };
    let mut missing = Vec::new();
    for (name, sym) in &symbol_of {
        if !calls(name) {
            continue;
        }
        // A text-returning native is dispatched under its `_dest` variant (see
        // `state::codegen`'s `_dest` list), so either spelling counts as supplied.
        if !registered.contains(sym) && !registered.contains(&format!("{sym}_dest")) {
            missing.push(sym.clone());
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "`client_loop` calls {missing:?} and the browser kernel does not supply them — every \
         browser page running `run_client` / `run_local` reaches the panicking stub on its \
         first turn (loft#1189).  Implement each in `engine_host::browser` and register it in \
         `KERNEL_FUNCTIONS_WASM`."
    );
}
