// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! TIC_TAC_TOE v3 — server-delivers-the-client integration test.
//!
//! v3 lets one loft program serve both static assets (HTML index,
//! loft client source) AND the WebSocket game protocol from v2 on
//! a single port — no separate static-file server, no nginx, no
//! second process.
//!
//! This harness validates the SERVER-SIDE half end-to-end:
//!
//!   - GET / returns `text/html` whose body is the index page.
//!   - GET /client.loft returns `text/plain` with the loft client
//!     source.
//!   - GET /favicon.ico (and any unrecognised path) returns 404.
//!   - The WebSocket endpoint on /ws still upgrades + echoes the
//!     message a loft probe sends — proves the HTTP routing layer
//!     coexists with the existing WS plumbing.
//!
//! The browser-side bootstrap (loading loft-rt.js + the
//! interpreter WASM + the loft client through the index page) is
//! the natural follow-up session — out of scope here.

#![allow(clippy::too_many_lines)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// ── Helpers (same shape as multiplayer_v5.rs) ──────────────────────────

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn examples_dir() -> PathBuf {
    workspace_root().join("tests/integration/multiplayer")
}

fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let port = listener.local_addr().expect("local_addr after bind").port();
    drop(listener);
    port
}

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

/// @PLN25 / registry-republish gate: the server scripts `use web` — a REGISTRY
/// package.  A registry copy predating the DN1 null model (`-> text` +
/// `return null`) REJECTS at compile time, so the server can never listen and
/// the test would burn its full timeout on a known, documented condition (the
/// loft-libs-net web/server republish, @PLN25 task #4).  Probe with `--dump`
/// (compile only, no execution) and SKIP — loudly — when the compile failure
/// lies inside the registry copy.  An IN-TREE failure still runs (and fails)
/// the test.  Self-healing: once the republished package is installed the
/// probe passes and the tests run again.
fn registry_predates_dn1(server_script: &str) -> bool {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--dump")
        .arg(examples_dir().join(server_script))
        .current_dir(examples_dir())
        .output();
    let Ok(out) = out else { return false };
    if out.status.success() {
        return false;
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("/.loft/registry/") {
        eprintln!(
            "SKIP {server_script}: the installed registry package predates the DN1 null \
             model (awaiting the web/server republish — @PLN25 task #4):\n{}",
            err.lines()
                .filter(|l| l.contains("/.loft/registry/"))
                .take(2)
                .collect::<Vec<_>>()
                .join("\n")
        );
        return true;
    }
    false
}

struct ServerGuard {
    child: Option<Child>,
    port: u16,
    early_stdout: String,
    stdout_rx: Option<mpsc::Receiver<String>>,
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
        let child = cmd.spawn().expect("failed to spawn loft v3 server");
        ServerGuard {
            child: Some(child),
            port,
            early_stdout: String::new(),
            stdout_rx: None,
        }
    }

    fn wait_listening(&mut self) -> bool {
        let stdout = match self.child.as_mut().and_then(|c| c.stdout.take()) {
            Some(s) => s,
            None => return false,
        };
        let marker = format!("listening port={}", self.port);
        let (early, rx, ok) = await_stdout_marker(stdout, &marker, Duration::from_secs(60));
        self.early_stdout = early;
        self.stdout_rx = Some(rx);
        ok
    }

    fn snapshot_stdout(&mut self) -> String {
        let mut s = self.early_stdout.clone();
        if let Some(rx) = &self.stdout_rx {
            while let Ok(line) = rx.try_recv() {
                s.push_str(&line);
                s.push('\n');
            }
        }
        s
    }

    /// Drain stdout until `marker` appears (or `timeout` elapses).  Used
    /// after an HTTP probe whose server-side log entry is async w.r.t.
    /// the curl response — the curl returns when the response is written
    /// to the socket, but the server's `println` for the same request
    /// may not have flushed its stdout pipe yet.
    fn wait_for_stdout_marker(&mut self, marker: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let snap = self.snapshot_stdout();
            if snap.contains(marker) || Instant::now() >= deadline {
                return snap;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn diagnose_listen_failure(&mut self) -> String {
        let mut out = String::new();
        if !self.early_stdout.is_empty() {
            out.push_str("\n  --- server stdout (pre-marker) ---\n");
            out.push_str(&self.early_stdout);
        }
        if let Some(child) = self.child.as_mut() {
            let mut stderr_buf = String::new();
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut stderr_buf);
            }
            if !stderr_buf.is_empty() {
                out.push_str("\n  --- server stderr ---\n");
                out.push_str(&stderr_buf);
            }
        }
        out
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// ── HTTP probe — minimal "send GET, parse status + body" without ureq.
//
// ureq pulls in TLS + connection pooling + retry logic; for a localhost
// loopback against a single-shot HTTP/1.1 server with `Connection:
// close` the std::net path is ~30 lines and avoids the dep.

struct HttpResponse {
    status: u16,
    headers: String,
    body: String,
}

fn http_get(port: u16, path: &str) -> std::io::Result<HttpResponse> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_secs(5),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes())?;
    stream.flush()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let split = raw.find("\r\n\r\n").unwrap_or(raw.len());
    let head = &raw[..split];
    let body = if split + 4 <= raw.len() {
        &raw[split + 4..]
    } else {
        ""
    };
    let status_line = head.lines().next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Ok(HttpResponse {
        status,
        headers: head.to_string(),
        body: body.to_string(),
    })
}

fn drain_with_timeout(mut child: Child, timeout: Duration) -> (String, Option<i32>) {
    let stdout = child.stdout.take().expect("child stdout was piped");
    let reader_handle = thread::spawn(move || {
        let mut buf = String::new();
        let mut br = BufReader::new(stdout);
        let _ = br.read_to_string(&mut buf);
        buf
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
    if exit_status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout_text = reader_handle.join().unwrap_or_default();
    (stdout_text, exit_status)
}

fn spawn_listening_server(script: &str, port: u16, label: &str) -> ServerGuard {
    let mut s = ServerGuard::spawn(script, port);
    if !s.wait_listening() {
        let diag = s.diagnose_listen_failure();
        panic!("{label} server failed to start within 60s{diag}");
    }
    s
}

fn spawn_client(client_script: &str, port: u16) -> Child {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret")
        .arg(examples_dir().join(client_script))
        .current_dir(examples_dir())
        .env("LOFT_TICTACTOE_PORT", port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd.spawn().expect("failed to spawn loft v3 client")
}

// ── Tests ───────────────────────────────────────────────────────────────

/// HTTP routing — `/`, `/client.loft`, `/favicon.ico`, unknown
/// paths.  Locks in:
///   - `n_tcp_respond_typed` actually sets the right Content-Type.
///   - The v3 server's path-routing dispatch handles all four
///     branches without crashing.
///   - The HTTP body bytes survive round-trip (no truncation, no
///     extra bytes from header / CRLF accounting).
#[test]
fn v3_http_routing() {
    if registry_predates_dn1("tictactoe_server_v3.loft") {
        return;
    }
    let port = pick_free_port();
    let mut server = spawn_listening_server("tictactoe_server_v3.loft", port, "v3");

    // GET / — text/html, body contains the page title marker.
    let r = http_get(port, "/").expect("GET /");
    assert_eq!(r.status, 200, "GET / status; headers:\n{}", r.headers);
    assert!(
        r.headers.contains("Content-Type: text/html"),
        "GET / Content-Type; headers:\n{}",
        r.headers
    );
    assert!(
        r.body.contains("tic-tac-toe (loft v3)"),
        "GET / body missing title marker; body:\n{}",
        r.body
    );

    // GET /client.loft — text/plain, body contains the loft fn marker.
    let r = http_get(port, "/client.loft").expect("GET /client.loft");
    assert_eq!(
        r.status, 200,
        "GET /client.loft status; headers:\n{}",
        r.headers
    );
    assert!(
        r.headers.contains("Content-Type: text/plain"),
        "GET /client.loft Content-Type; headers:\n{}",
        r.headers
    );
    assert!(
        r.body.contains("fn main()"),
        "GET /client.loft body missing fn marker; body:\n{}",
        r.body
    );

    // GET /favicon.ico — 404.
    let r = http_get(port, "/favicon.ico").expect("GET /favicon.ico");
    assert_eq!(
        r.status, 404,
        "GET /favicon.ico status; headers:\n{}",
        r.headers
    );

    // GET /unknown.html — 404 + server logs the unrouted path.
    let r = http_get(port, "/unknown.html").expect("GET /unknown.html");
    assert_eq!(
        r.status, 404,
        "GET /unknown.html status; headers:\n{}",
        r.headers
    );

    // Drain server stdout snapshot — the unrouted-path log should
    // surface, proving the dispatch reached the `else` arm rather
    // than e.g. crashing silently.  Curl returns when the response
    // hits the wire, but the server's `println` lands on its stdout
    // pipe asynchronously, so wait for the marker rather than
    // sampling once.
    let server_out =
        server.wait_for_stdout_marker("v3-srv: unrouted GET /unknown.html", Duration::from_secs(2));
    assert!(
        server_out.contains("v3-srv: unrouted GET /unknown.html"),
        "v3 server didn't log the unrouted GET within 2s; server stdout:\n{server_out}"
    );
}

/// Full game over WebSocket — the same v3 server that just served
/// HTML on `/` upgrades a WS request on `/ws` and plays a complete
/// tic-tac-toe game using the v3 wire protocol:
///   client → server: `click:<row>,<col>`
///   server → client: `place:<mark>,<row>,<col>` (X then O each turn)
///   server → client: `gameover:<X|O|draw>`
///
/// The probe plays X's column-0 win — clicks (0,0), (1,0), (2,0).
/// Server places O at the first empty cell each turn (deterministic),
/// so the trajectory is fixed and the harness can assert on every
/// frame.  Locks in:
///   - HTTP routing + WebSocket coexist on the same port (HTTP
///     probe runs first; server then accepts a WS upgrade on /ws).
///   - The full game-protocol round-trip works end-to-end.
///   - The interactive HTML page in INDEX_HTML embeds the
///     equivalent JS protocol logic (verified by string-search of
///     the served body for the protocol markers).
#[test]
fn v3_full_game_over_websocket() {
    if registry_predates_dn1("tictactoe_server_v3.loft") {
        return;
    }
    // This is a real two-process integration probe (loft server + loft
    // client over a TCP socket, WebSocket-upgraded).  On a loaded CI runner
    // the WS round-trip can stall and the client misses its deadline — a
    // transient, not a protocol regression (macOS/Windows runners pass the
    // same code).  Make it robust by retrying the whole interaction on a
    // fresh port (the previous server is killed by `ServerGuard`'s `Drop`),
    // with a generous per-attempt timeout.  Deterministic checks (HTTP
    // markers, game trajectory) stay hard asserts on the successful attempt;
    // a genuine hang still fails after all attempts.
    let mut out = String::new();
    let mut server_out = String::new();
    let mut clean_exit = false;

    for attempt in 1..=3 {
        let port = pick_free_port();
        let mut server = spawn_listening_server("tictactoe_server_v3.loft", port, "v3");

        // Drive an HTTP probe first to confirm the server is multiplexing
        // both protocols correctly.  The v3 server loops through accepts,
        // so HTTP-then-WS-then-HTTP is the realistic shape.
        let r = http_get(port, "/").expect("GET / pre-WS probe");
        assert_eq!(r.status, 200, "GET / pre-WS probe");
        // The HTML body must embed the matching JS-side protocol —
        // surface this as a check so the page-level client and the
        // server-level handler can't drift silently.
        for marker in ["click:", "place:", "gameover:"] {
            assert!(
                r.body.contains(marker),
                "served HTML missing protocol marker `{marker}`; body:\n{}",
                r.body
            );
        }

        let client = spawn_client("tictactoe_client_v3_probe.loft", port);
        let (o, status) = drain_with_timeout(client, Duration::from_secs(60));
        out = o;
        server_out = server.snapshot_stdout();
        if status == Some(0) {
            clean_exit = true;
            break;
        }
        eprintln!(
            "v3 attempt {attempt}/3: client did not exit cleanly (status={status:?}) — \
             likely a transient WS stall on a loaded runner; retrying on a fresh port"
        );
        // `server` is dropped here → killed before the next attempt.
    }

    assert!(
        clean_exit,
        "v3 game probe did not exit cleanly after 3 attempts; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    // Expected trajectory — assert each step in order.
    for needle in [
        "v3-probe: place X 0,0",
        "v3-probe: place O 0,1",
        "v3-probe: place X 1,0",
        "v3-probe: place O 0,2",
        "v3-probe: place X 2,0",
        "v3-probe: gameover X",
        "v3-probe: ok",
    ] {
        assert!(
            out.contains(needle),
            "v3 game probe missing step `{needle}`; client stdout=\n{out}\n--- server stdout:\n{server_out}"
        );
    }
    // Server-side: each step logged from the game handler.
    for needle in [
        "v3-game: X at 0,0",
        "v3-game: O at 0,1",
        "v3-game: X at 1,0",
        "v3-game: O at 0,2",
        "v3-game: X at 2,0",
        "v3-game: gameover X",
    ] {
        assert!(
            server_out.contains(needle),
            "v3 server missing step `{needle}`; server stdout:\n{server_out}"
        );
    }
}
