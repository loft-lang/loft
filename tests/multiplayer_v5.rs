// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! TIC_TAC_TOE v5 — primitive integration tests.
//!
//! v5 introduces the binary-WebSocket surface and (eventually) the
//! session-tagged blob protocol that plan-36's audience-generative-art
//! demo will sit on top of.  Each `t<N>_*` test is a focused
//! validation of one primitive in the v5 layer:
//!
//!   t1 — `send_binary` + `last_opcode` round-trip a 4-byte payload
//!        through opcode 0x02 in both directions
//!   t2 — session-tagged blob header (5 bytes: type + session-u32)   (TODO)
//!   t3 — N-client routing (server tags responses; clients see only
//!        their own + broadcasts)                                     (TODO)
//!   t4 — catch-up replay cache after reconnect                       (TODO)
//!   t5 — sluggish world tick + decay timing harness                  (TODO)
//!
//! Each test pair lives under `tests/integration/multiplayer/v5_*.loft`
//! and is launched as a subprocess to dodge P245 (single-process
//! `parallel{}` + I/O hangs when one arm accepts and another connects
//! to a loopback port).  The harness pattern mirrors
//! `tests/multiplayer_v2.rs` — the helpers below are deliberately
//! the same shape so future ttt-vN suites can copy this file.

#![allow(clippy::too_many_lines)]

use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

mod common;

/// A wall-clock budget for waiting on a child process, stretched when the machine is
/// shared (`common::deadline_scale`).
///
/// Every timeout in this file waits on a spawned `loft` process — server start, a
/// client's whole game — and the variance is in process startup under contention, not
/// in the work itself.  A fixed number is a bet against the box: measured at load
/// average 40, a 60 s server-start budget failed four tests that need under 2 s idle.
fn budget(secs: u64) -> Duration {
    Duration::from_secs(secs * common::deadline_scale())
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn examples_dir() -> PathBuf {
    workspace_root().join("tests/integration/multiplayer")
}

/// Pick a free TCP port via a transient bind to `127.0.0.1:0`.  Same
/// idiom as `tests/multiplayer_v2.rs::pick_free_port` (P231).
fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let port = listener.local_addr().expect("local_addr after bind").port();
    drop(listener);
    port
}

/// Wait for the server child to print a marker line on its stdout.
/// Single-client v5 servers (`srv.next()` blocking accept) cannot
/// tolerate the v2-style TCP-connect probe — accepting the probe
/// socket counts as the test's "client", so the probe consumes the
/// server's only `srv.next()` call and the server exits with
/// "accept-failed" before the real client connects.  Reading stdout
/// for "listening port=" is probe-free: the server prints it
/// immediately after `tcp_listen` succeeds, before entering accept.
///
/// Spawns a background reader thread that streams stdout lines into
/// a channel.  Returns the (lines-so-far, residual stdout reader)
/// after the marker is seen, or (lines-so-far, None) on timeout.
fn await_stdout_marker(
    stdout: ChildStdout,
    marker: &str,
    timeout: Duration,
) -> (String, mpsc::Receiver<String>, bool) {
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let br = BufReader::new(stdout);
        for line in br.lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + timeout;
    let mut accumulated = String::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(line) => {
                accumulated.push_str(&line);
                accumulated.push('\n');
                if line.contains(marker) {
                    return (accumulated, rx, true);
                }
            }
            Err(_) => continue,
        }
    }
    (accumulated, rx, false)
}

fn drain_with_timeout(mut child: Child, timeout: Duration) -> (String, Option<i32>) {
    // Phase timing — prints to stderr (hidden by nextest on PASS, SHOWN on a
    // failure).  Every wait here is now BOUNDED (wait-loop `timeout`, then the
    // JOIN_CAP drain below), so this can no longer produce a 600s TMT; the
    // `[v5-timing]` lines still name which phase ran long when a test fails.
    let t0 = Instant::now();
    let mut stdout_text = String::new();
    let stdout = child.stdout.take().expect("child stdout was piped");
    let (txt_tx, txt_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut buf = String::new();
        let mut br = BufReader::new(stdout);
        let _ = br.read_to_string(&mut buf);
        let _ = txt_tx.send(buf); // receiver may be gone if we capped out — harmless
    });

    let deadline = Instant::now() + timeout;
    let mut exit_status: Option<i32> = None;
    while Instant::now() < deadline {
        if let Ok(Some(s)) = child.try_wait() {
            exit_status = s.code();
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    eprintln!(
        "[v5-timing] drain: wait-loop done +{:?} exit={exit_status:?}",
        t0.elapsed()
    );
    if exit_status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("[v5-timing] drain: child killed +{:?}", t0.elapsed());
    }
    // Bound the drain.  A killed child can orphan a `cargo build` /
    // cdylib-compile grandchild that keeps the stdout pipe OPEN, so the reader's
    // `read_to_string()` never sees EOF — a plain join would block to the 600s
    // terminate-after.  Wait at most JOIN_CAP for the text: log WHY at JOIN_WARN
    // (naming the held-pipe mechanism, cross-ref the build-lock ledger), and at
    // the cap return whatever arrived instead of hanging.  The orphaned build is
    // HARMLESS — it finishes in the background and warms the cdylib cache for the
    // retry; only BLOCKING on it was the bug.  This keeps multiplayer_v5 fully
    // parallel (no serial group) while turning a 600s TMT into a fast
    // fail-and-retry.  On the normal path the child has already EOF'd, so the
    // text is waiting and the first recv returns instantly.
    let killed = exit_status.is_none();
    const JOIN_WARN: Duration = Duration::from_secs(15);
    const JOIN_CAP: Duration = Duration::from_secs(45);
    match txt_rx.recv_timeout(JOIN_WARN) {
        Ok(s) => stdout_text = s,
        Err(mpsc::RecvTimeoutError::Disconnected) => {} // reader died without sending
        Err(mpsc::RecvTimeoutError::Timeout) => {
            eprintln!(
                "[v5-timing] drain: reader blocked >{JOIN_WARN:?} +{:?} (child_killed={killed}) — a \
                 cargo/cdylib grandchild holds the stdout pipe open; waiting up to {JOIN_CAP:?} then \
                 returning partial output (the orphaned build warms the cache for the retry, \
                 cross-ref the build-lock ledger)",
                t0.elapsed()
            );
            match txt_rx.recv_timeout(JOIN_CAP - JOIN_WARN) {
                Ok(s) => stdout_text = s,
                Err(_) => eprintln!(
                    "[v5-timing] drain: reader CAPPED at {JOIN_CAP:?} +{:?} — returning partial \
                     output instead of blocking to the 600s TMT",
                    t0.elapsed()
                ),
            }
        }
    }
    eprintln!("[v5-timing] drain: reader done +{:?}", t0.elapsed());
    (stdout_text, exit_status)
}

/// Auto-kills its server child on drop.  Avoids leaking a listener
/// when an assertion panics.
struct ServerGuard {
    child: Option<Child>,
    port: u16,
    /// Lines of stdout accumulated up to (and including) the
    /// "listening port=" marker — preserved so a downstream failure
    /// can include the early server output in its diagnostic.
    early_stdout: String,
    /// Receiver for the rest of the server's stdout.  Drained at the
    /// end of the test by `drain_remaining_stdout`.
    stdout_rx: Option<mpsc::Receiver<String>>,
    /// The server's stderr, read on a thread.  Carries the native listener's
    /// bind verdict, which stdout does NOT: a server that lost the port race still
    /// prints its own "listening port=" line (its handle is `-1`), so the stdout
    /// marker cannot tell "I own this port" from "someone else does".
    stderr_rx: Option<mpsc::Receiver<String>>,
}

impl ServerGuard {
    fn spawn(server_script: &str, port: u16) -> Self {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg("--interpret")
            .arg(examples_dir().join(server_script))
            .env("LOFT_TICTACTOE_PORT", port.to_string())
            .current_dir(examples_dir())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn loft v5 server");
        let stderr_rx = child.stderr.take().map(|s| {
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                for line in BufReader::new(s).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
            rx
        });
        ServerGuard {
            child: Some(child),
            port,
            early_stdout: String::new(),
            stdout_rx: None,
            stderr_rx,
        }
    }

    /// Whether this server LOST the port it was told to use.
    ///
    /// `pick_free_port` binds `:0`, reads the port and closes it, so another test can
    /// take that port before this server binds it.  The loser is invisible on stdout —
    /// it prints "listening port=" regardless — so its client connects to whoever won,
    /// receives nothing it expects, and the test hangs until the harness watchdog fires
    /// (measured: four v5 tests killed at 300 s each).  The native listener does say so
    /// on stderr, which is the only place the truth appears.
    ///
    /// `grace` bounds the wait because stdout and stderr are separate pipes: the
    /// "cannot bind" line is WRITTEN before "listening port=", but the two can be READ
    /// out of order.  Missing it degrades to the previous behaviour rather than to
    /// something worse, so a short grace is the right trade.
    fn bind_lost(&mut self, grace: Duration) -> bool {
        let Some(rx) = self.stderr_rx.as_ref() else {
            return false;
        };
        let deadline = Instant::now() + grace;
        let mut lost = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(line) => {
                    let is_loss = line.contains("cannot bind");
                    self.early_stdout.push_str(&line);
                    self.early_stdout.push('\n');
                    if is_loss {
                        lost = true;
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        lost
    }

    /// Wait until the server prints its "listening port=" marker.
    /// Probe-free — see `await_stdout_marker` for why.  On success,
    /// stashes the early-stdout lines + retains the residual reader
    /// so the rest of the run can be drained later.
    fn wait_listening(&mut self) -> bool {
        let stdout = match self.child.as_mut().and_then(|c| c.stdout.take()) {
            Some(s) => s,
            None => return false,
        };
        let marker = format!("listening port={}", self.port);
        let (early, rx, ok) = await_stdout_marker(stdout, &marker, budget(60));
        self.early_stdout = early;
        self.stdout_rx = Some(rx);
        ok
    }

    /// Drain whatever the server has emitted since `wait_listening`
    /// returned, plus the early lines.  Cheap when the test passed;
    /// useful as a panic message when something failed downstream.
    fn snapshot_stdout(&mut self) -> String {
        let mut s = self.early_stdout.clone();
        if let Some(rx) = &self.stdout_rx {
            // Drain non-blocking: anything already on the channel.
            while let Ok(line) = rx.try_recv() {
                s.push_str(&line);
                s.push('\n');
            }
        }
        s
    }

    fn diagnose_listen_failure(&mut self) -> String {
        let mut out = String::new();
        if !self.early_stdout.is_empty() {
            out.push_str("\n  --- server stdout (pre-marker) ---\n");
            out.push_str(&self.early_stdout);
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    out.push_str(&format!(
                        "\n  server child already exited: status={status:?}"
                    ));
                }
                Ok(None) => {
                    out.push_str("\n  server child still running but not listening");
                }
                Err(e) => {
                    out.push_str(&format!("\n  failed to query child status: {e}"));
                }
            }
            // Drain the reader thread's channel — `spawn` took `child.stderr`, so the
            // handle is no longer here to read directly.
            let mut stderr_buf = String::new();
            if let Some(rx) = self.stderr_rx.as_ref() {
                while let Ok(line) = rx.try_recv() {
                    stderr_buf.push_str(&line);
                    stderr_buf.push('\n');
                }
            }
            if !stderr_buf.is_empty() {
                out.push_str("\n  --- server stderr ---\n");
                out.push_str(&stderr_buf);
            }
            if self.early_stdout.is_empty() && stderr_buf.is_empty() {
                out.push_str("\n  (server produced no stdout/stderr)");
            }
        } else {
            out.push_str("\n  (no server child captured)");
        }
        out
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let t = Instant::now();
            let _ = c.kill();
            let _ = c.wait();
            eprintln!("[v5-timing] server drop: kill+wait +{:?}", t.elapsed());
        }
    }
}

fn spawn_client(client_script: &str, port: u16) -> Child {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret")
        .arg(examples_dir().join(client_script))
        .current_dir(examples_dir())
        .env("LOFT_TICTACTOE_PORT", port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd.spawn().expect("failed to spawn loft v5 client")
}

/// Same as `spawn_client` but appends a single `label` argv arg and
/// extra env vars (e.g. `LOFT_T3_CLIENTS`).  Used by t3 to spawn
/// multiple distinguishable client subprocesses against one server.
fn spawn_client_with_args(
    client_script: &str,
    port: u16,
    label: &str,
    extra_env: &[(&str, &str)],
) -> Child {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret")
        .arg(examples_dir().join(client_script))
        .arg(label)
        .current_dir(examples_dir())
        .env("LOFT_TICTACTOE_PORT", port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.spawn().expect("failed to spawn loft v5 client")
}

/// Convenience: spawn server + wait for its "listening port=N"
/// marker, panic with a useful diagnostic if it doesn't appear.
/// Start `script` on a port it actually OWNS, re-picking when it loses the race.
///
/// Prefer this over picking a port yourself: `pick_free_port` cannot reserve the port
/// it reports, and a server that loses it still says "listening", so the test proceeds
/// against a stranger's server and hangs.  Re-picking costs milliseconds; the hang
/// costs the harness watchdog (300 s per test).  Same idiom as
/// `multiplayer_v2::spawn_server_on_free_port`, for the same reason.
fn spawn_listening_server_on_free_port(script: &str, label: &str) -> (ServerGuard, u16) {
    const ATTEMPTS: usize = 5;
    let mut last_lost = 0;
    for _ in 0..ATTEMPTS {
        let port = pick_free_port();
        let mut s = spawn_listening_server(script, port, label);
        // 250 ms: the verdict is already written by the time the stdout marker
        // arrives; this only covers cross-pipe read ordering.
        if s.bind_lost(Duration::from_millis(250)) {
            last_lost = port;
            continue; // `s` drops here, killing the server that never owned the port
        }
        return (s, port);
    }
    panic!(
        "{label}: could not obtain a port {script} owns in {ATTEMPTS} attempts          (last contended port {last_lost})"
    );
}

fn spawn_listening_server(script: &str, port: u16, label: &str) -> ServerGuard {
    let t = Instant::now();
    let mut s = ServerGuard::spawn(script, port);
    let ok = s.wait_listening();
    eprintln!(
        "[v5-timing] {label} server spawn+listen +{:?} ok={ok}",
        t.elapsed()
    );
    if !ok {
        let diag = s.diagnose_listen_failure();
        panic!("{label} server failed to start within 60s{diag}");
    }
    s
}

// ── t1: 4-byte binary frame round-trip ──────────────────────────────────

/// Validates that `send_binary` + `last_opcode` compose end-to-end
/// across `lib/server` (single-client path) and `lib/web`
/// (`WsHandler`).  The 4-byte payload is the smallest non-trivial
/// frame; the cell layout that plan-36 will use (u8 color + u8
/// height + u16 age) is exactly 4 bytes.
#[test]
fn v5_t1_binary_round_trip() {
    let (mut server, port) =
        spawn_listening_server_on_free_port("v5_t1_binary_server.loft", "v5-t1");

    let client = spawn_client("v5_t1_binary_client.loft", port);
    let (out, status) = drain_with_timeout(client, budget(30));

    let server_out = server.snapshot_stdout();
    assert_eq!(
        status,
        Some(0),
        "v5-t1 client did not exit cleanly; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    assert!(
        out.contains("v5-t1c: ok"),
        "v5-t1 client did not reach the success line; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    // Lock in that the BINARY opcode actually went out — opcode 1 (text)
    // would be a regression to the legacy path masquerading as a pass.
    assert!(
        out.contains("v5-t1c: recv opcode=2"),
        "v5-t1 client received non-binary opcode; client stdout=\n{out}"
    );
}

// ── t2: session-tagged binary blobs ────────────────────────────────────

/// Server sends 5 binary blobs with session id 7, then 1 with
/// session id 8.  Client groups by session id (flushing on every
/// session-change) and reports "applied session=N blobs=M" for each.
/// Validates:
///   - lib/web pack_u8 / pack_u32_le / pack_u16_le / pack_take build
///     the wire bytes correctly (NUL bytes preserved past the
///     character-null-sentinel limitation of `text` interpolation)
///   - lib/web byte_at decodes individual bytes from the inbound
///     binary text payload
///   - the new-session-id transition triggers a flush on the client
///   - no blobs are coalesced or dropped (5 + 1, not 4 + 1 or 5 + 0)
#[test]
fn v5_t2_session_blob_grouping() {
    let (mut server, port) =
        spawn_listening_server_on_free_port("v5_t2_session_blobs_server.loft", "v5-t2");

    let client = spawn_client("v5_t2_session_blobs_client.loft", port);
    let (out, status) = drain_with_timeout(client, budget(30));

    let server_out = server.snapshot_stdout();
    assert_eq!(
        status,
        Some(0),
        "v5-t2 client did not exit cleanly; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    assert!(
        out.contains("v5-t2c: ok"),
        "v5-t2 client did not reach the success line; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    assert!(
        out.contains("v5-t2c: applied session=7 blobs=5"),
        "v5-t2 client did not flush session 7 with 5 blobs; client stdout=\n{out}"
    );
    assert!(
        out.contains("v5-t2c: applied session=8 blobs=1"),
        "v5-t2 client did not flush session 8 with 1 blob; client stdout=\n{out}"
    );
}

// ── t3: N-client routing + broadcast ───────────────────────────────────

/// Spawns 5 client subprocesses concurrently against one server; each
/// sends a "Hello" frame; server broadcasts each Hello to every
/// connected client.  Locks in:
///   - srv.run() handles N>2 connections without dropping events
///     (v2's tests cap at 2)
///   - srv.broadcast() reaches every active connection
///   - per-client routing correctness as N concurrent connect /
///     message storms interleave
///
/// Compressed from the v5 plan's "30 clients × 5 minutes" — that
/// scenario is a deployment liveness check, not a CI test.  N=5 ×
/// ~1 s validates the routing pattern without bloating CI runtime.
#[test]
fn v5_t3_n_client_broadcast() {
    const N: usize = 5;
    let n_str = N.to_string();
    let env = [("LOFT_T3_CLIENTS", n_str.as_str())];

    // Same port-ownership retry as the other tests, spelled out here because this one
    // does not use `spawn_listening_server`: its readiness check is N connects, not the
    // plain marker.
    let (mut server, port) = {
        let mut got = None;
        for _ in 0..5 {
            let port = pick_free_port();
            let mut s = ServerGuard::spawn("v5_t3_n_clients_server.loft", port);
            if !s.wait_listening() {
                let diag = s.diagnose_listen_failure();
                panic!("v5-t3 server failed to start within 60s{diag}");
            }
            if s.bind_lost(Duration::from_millis(250)) {
                continue;
            }
            got = Some((s, port));
            break;
        }
        got.expect("v5-t3: could not obtain a port the server owns in 5 attempts")
    };
    // The server reads LOFT_T3_CLIENTS from env, but it doesn't need
    // it for this assertion shape — we just need the server to log
    // "connected seat=" once per client and "broadcast hello from"
    // once per inbound Hello.
    let labels = ["A", "B", "C", "D", "E"];
    let mut children = Vec::with_capacity(N);
    for label in &labels[..N] {
        children.push(spawn_client_with_args(
            "v5_t3_n_clients_client.loft",
            port,
            label,
            &env,
        ));
    }

    // Drain each client in parallel — each one prints its line, no
    // ordering guarantee.  Generous per-client timeout (30 s) lets
    // a slow CI runner finish without flake.
    let mut handles = Vec::with_capacity(N);
    for child in children {
        let h = thread::spawn(move || drain_with_timeout(child, budget(30)));
        handles.push(h);
    }
    let outs: Vec<(String, Option<i32>)> = handles
        .into_iter()
        .map(|h| h.join().expect("drain join"))
        .collect();

    let server_out = server.snapshot_stdout();
    for (i, (out, status)) in outs.iter().enumerate() {
        let label = labels[i];
        assert_eq!(
            *status,
            Some(0),
            "v5-t3 client {label} did not exit cleanly; stdout=\n{out}\n--- server stdout:\n{server_out}"
        );
        let needle = format!("v5-t3c[{label}]: ok received={N}");
        assert!(
            out.contains(&needle),
            "v5-t3 client {label} did not see all {N} hellos; stdout=\n{out}\n--- server stdout:\n{server_out}"
        );
    }

    // Server-side: assert it logged exactly N connects and N broadcasts.
    let connects = server_out.matches("v5-t3: connected seat=").count();
    let broadcasts = server_out.matches("v5-t3: broadcast hello from").count();
    assert_eq!(
        connects, N,
        "v5-t3 server logged {connects} connects, expected {N}; server stdout:\n{server_out}"
    );
    assert_eq!(
        broadcasts, N,
        "v5-t3 server logged {broadcasts} broadcasts, expected {N}; server stdout:\n{server_out}"
    );
}

// ── t4: stateless catch-up across reconnect ────────────────────────────

/// Single client runs two sequential WebSocket sessions against the
/// same server.  Phase A subscribes to the first half of the cached
/// sessions; the client closes the WS; phase B reopens, sends a
/// `catch_up:N` request, and drains the remainder.  The server holds
/// no per-client identity — the catch-up replay is computed purely
/// from the request's `last_session` value.  Locks in:
///   - srv.run() handles disconnect-then-reconnect on the same port
///     without state leakage
///   - the catch-up replay returns exactly the gap (no overlap, no
///     missed sessions)
///   - binary blob session ids decode correctly across the
///     disconnect boundary
#[test]
fn v5_t4_catch_up_after_reconnect() {
    const TOTAL: usize = 5;
    let total_str = TOTAL.to_string();
    let env = [("LOFT_T4_SESSIONS", total_str.as_str())];

    let (mut server, port) =
        spawn_listening_server_on_free_port("v5_t4_catch_up_server.loft", "v5-t4");

    let client = spawn_client_with_args("v5_t4_catch_up_client.loft", port, "", &env);
    let (out, status) = drain_with_timeout(client, budget(30));

    let server_out = server.snapshot_stdout();
    assert_eq!(
        status,
        Some(0),
        "v5-t4 client did not exit cleanly; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    assert!(
        out.contains(&format!("v5-t4c: ok last_session={TOTAL}")),
        "v5-t4 client did not reach the success line; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    // The server should have logged BOTH the subscribe and the catch_up
    // — failure to log either means one phase silently went down a
    // wrong path.
    assert!(
        server_out.contains("v5-t4: subscribe cid="),
        "v5-t4 server did not log subscribe; server stdout=\n{server_out}"
    );
    assert!(
        server_out.contains("v5-t4: catch_up cid="),
        "v5-t4 server did not log catch_up; server stdout=\n{server_out}"
    );
}

// ── t5: world tick + decay timings ─────────────────────────────────────

/// Standalone (no server, no client subprocess pairing).  Spins
/// `lib/hex_world::tick_and_decay` over compressed timing constants and
/// confirms the expected lifecycle markers appear in stdout in the
/// right order.  Validates:
///   - isolated cells decay around `base_lifetime + decay_window`
///   - cluster cells live considerably longer because each filled
///     neighbour grants a `neighbour_lease`
///   - the centre cascades on the tick AFTER its surroundings
///     vanish (one-step deferral, not same-tick)
///
/// Compressed scale chosen so the test runs in <1 s while still
/// exercising the full lifecycle: base=10, lease=10, window=5
/// (production constants are 5 min / 0.5 s / 30 s at 10 Hz —
/// same algebraic shape, different units).
///
/// `hex_world` resolves via the registry (loft-lang/loft-libs-world);
/// 0.1.2 republished 2026-07-02 with value-preserving casts for the
/// narrowing-cast enforcement that had broken 0.1.1, un-blocking this
/// test (it had been ignored since Stage B removed the monorepo libs).
#[test]
fn v5_t5_world_tick_and_decay() {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret")
        .arg(examples_dir().join("v5_t5_world_timings.loft"))
        .current_dir(examples_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = cmd.spawn().expect("failed to spawn loft v5-t5");
    let (out, status) = drain_with_timeout(child, budget(30));

    assert_eq!(
        status,
        Some(0),
        "v5-t5 did not exit cleanly; stdout=\n{out}"
    );
    // Lifecycle markers must appear in order — assert each one.
    for needle in [
        "v5-t5: setup placed=8 isolated=1 cluster=7",
        "v5-t5: tick=14 cells=8", // isolated still in window
        "v5-t5: tick=15 cells=7", // isolated removed
        "v5-t5: tick=44 cells=7", // cluster still fully alive
        "v5-t5: tick=45 cells=1", // surrounding decayed; centre alone
        "v5-t5: tick=46 cells=0", // centre cascaded one tick later
        "v5-t5: ok",
    ] {
        assert!(
            out.contains(needle),
            "v5-t5 missing lifecycle marker `{needle}`; stdout=\n{out}"
        );
    }
}
