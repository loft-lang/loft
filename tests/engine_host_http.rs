// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Engine-integrated non-blocking outbound HTTP (`engine_host::http_fetch`).
//!
//! The one invariant under test: `http_fetch` returns BEFORE any network
//! I/O happens, and the completion is delivered through the SAME event
//! queue as every other event — so a slow upstream can never stall the
//! loop.  The probe: a deliberately SLOW local HTTP server (400 ms) while
//! the loft program counts ticks; the completion event must carry the
//! exact body AND the tick counter must have advanced meaningfully in
//! between (a blocking fetch would freeze it — the routing consumer
//! measured >10 s loop freezes from one Nominatim call, which is what
//! this feature retires).

use loft::file_access as fa;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::Duration;

fn test_tmp() -> PathBuf {
    let d = std::env::temp_dir().join("loft_eh_http");
    let _ = fa::create_dir_all(&d);
    d
}

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Kill the kernel on drop.  An `--interpret` run spawns no grandchild, so the child is the
/// whole of what can leak.
struct Guard(Option<Child>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// One-shot slow HTTP responder: accept one connection, read the request
/// head, wait `delay`, answer 200 with `body`.
fn slow_http_server(delay: Duration, body: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind http fixture");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            std::thread::sleep(delay);
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    port
}

fn fixture(ws_port: u16, url: &str) -> String {
    format!(
        r#"use engine_host;

struct W {{ ticks: integer not null, issued: integer not null, at: integer not null }}

fn main() {{
  w = W {{ ticks: 0, issued: 0, at: 0 }};
  engine_host::run({ws_port}, 2000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 3 {{
        waited = w.ticks - w.at;
        println("HTTPDONE id={{ev.cid}} status={{ev.status}} body={{ev.payload}} waited={{waited}}");
        engine_host::stop();
      }}
    }},
    fn() {{
      w.ticks = w.ticks + 1;
      if w.issued == 0 {{
        w.issued = engine_host::http_fetch("GET", "{url}", "", "x-probe: eh-http");
        w.at = w.ticks;
      }}
    }});
}}
"#
    )
}

fn run_fixture(name: &str, ws_port: u16, url: &str) -> String {
    let prog = test_tmp().join(format!("{name}_{}.loft", std::process::id()));
    fa::write(&prog, fixture(ws_port, url)).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    let child = cmd
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--timeout")
        .arg("30")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kernel");
    let mut guard = Guard(Some(child));
    let out = guard
        .0
        .take()
        .unwrap()
        .wait_with_output()
        .expect("kernel output");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The invariant probe: a 400 ms upstream while 2 ms ticks keep flowing.
#[test]
fn http_fetch_completes_as_event_without_stalling_ticks() {
    if !fa::exists(loft_bin()) {
        eprintln!("skipping: loft not built");
        return;
    }
    let http_port = slow_http_server(Duration::from_millis(400), "hello-engine");
    let ws_port = free_port();
    let stdout = run_fixture(
        "eh_http_slow",
        ws_port,
        &format!("http://127.0.0.1:{http_port}/probe"),
    );
    let line = stdout
        .lines()
        .find(|l| l.starts_with("HTTPDONE"))
        .unwrap_or_else(|| panic!("no HTTPDONE line; stdout: {stdout}"));
    assert!(
        line.contains("status=200") && line.contains("body=hello-engine"),
        "completion payload: {line}"
    );
    let waited: i64 = line
        .split("waited=")
        .nth(1)
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0);
    // 400 ms upstream over 2 ms ticks ≈ 200 ticks; ≥ 20 proves the loop
    // kept running while the request was in flight (a blocking fetch
    // would make this 0-2).
    assert!(waited >= 20, "loop stalled during the fetch: {line}");
}

/// Error path: an unreachable upstream completes as a NEGATIVE-status
/// event (the loop decides what that means) — it never throws or blocks.
#[test]
fn http_fetch_error_is_a_negative_status_event() {
    if !fa::exists(loft_bin()) {
        eprintln!("skipping: loft not built");
        return;
    }
    // A port with nothing behind it: bind-then-drop.
    let dead = free_port();
    let ws_port = free_port();
    let stdout = run_fixture(
        "eh_http_dead",
        ws_port,
        &format!("http://127.0.0.1:{dead}/nope"),
    );
    let line = stdout
        .lines()
        .find(|l| l.starts_with("HTTPDONE"))
        .unwrap_or_else(|| panic!("no HTTPDONE line; stdout: {stdout}"));
    assert!(
        line.contains("status=-1"),
        "expected transport error: {line}"
    );
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}
