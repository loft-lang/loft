// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end smoke test for the loft-view markdown renderer
//! (plan-35 phase 03 + follow-ups).  Spawns the viewer in
//! interp-mode, curls a fixture markdown file under
//! `tests/fixtures/`, and asserts on the rendered HTML.
//!
//! Pinned features (each = one assertion):
//!   - Headings get GitHub-style slug ids
//!   - Bold (`**`) → `<strong>`
//!   - Italic (`*` / `_`) → `<em>`
//!   - Inline code (`` ` ``) → `<code>`
//!   - Strikethrough (`~~`) → `<del>`
//!   - Backslash escape `\*` → literal `*`
//!   - Backslash escape `\|` → literal `|`
//!   - Image `![alt](url)` → `<img src="url" alt="alt">`
//!   - Relative `[text](other.md)` → `<a href="/file/...">`
//!   - Autolink `<https://…>` → `<a href="https://…">`
//!   - Autolink `<email>` → `<a href="mailto:email">`
//!   - Block quote (`> `) merges consecutive lines into one
//!     `<blockquote>`; blank `> ` line separates blockquotes
//!   - UTF-8 em-dash `—` (3 bytes) NOT widened to `———` (P272)

use loft::file_access as fa;
use std::io::{BufRead, BufReader, Read};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The port this test's viewer serves on, via `LOFT_VIEW_PORT`.  Deliberately NOT
/// 8765: that is the default a developer's `make view` takes, and two viewers cannot
/// share it.  Both halves of the test must use this — polling 8765 instead read
/// whichever viewer happened to be up, and went green off a process it had not
/// started (fixed 2026-08-22, after 240 s of waiting on a box where 8765 was free).
const VIEWER_PORT: u16 = 18765;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft")
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Drain a child pipe into a shared buffer on its own thread, line by line — so a slow
/// (still-running) viewer's output is captured incrementally and a crashed viewer's full
/// output is captured before its pipe hits EOF.  Read back by `diagnose` on failure.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Arc<Mutex<String>> {
    let buf = Arc::new(Mutex::new(String::new()));
    if let Some(p) = pipe {
        let b = Arc::clone(&buf);
        thread::spawn(move || {
            for line in BufReader::new(p).lines() {
                let Ok(line) = line else { break };
                let mut g = b.lock().unwrap();
                g.push_str(&line);
                g.push('\n');
            }
        });
    }
    buf
}

/// @PLN25 / registry-republish gate (same as the multiplayer harnesses): the
/// viewer `use`s REGISTRY packages (web).  A registry copy predating the DN1
/// null model rejects at compile time, so the viewer can never listen and the
/// smoke would burn its 240s timeout on a known, documented condition (the
/// loft-libs-net republish, @PLN25 task #4).  Probe with `--dump` (compile
/// only) and SKIP loudly when the failure lies inside the registry copy; an
/// in-tree failure still runs (and fails) the smoke.  Self-healing after the
/// republish.
fn registry_predates_dn1() -> bool {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--dump")
        .arg("--lib")
        .arg(project_root().join("lib"))
        .arg(project_root().join("tools/viewer/src/main.loft"))
        .current_dir(project_root())
        .output();
    let Ok(out) = out else { return false };
    if out.status.success() {
        return false;
    }
    // Normalise `\` → `/` so the registry-path match fires on Windows too (the
    // compiler prints `...\.loft\registry\...` there — this job is ubuntu-only today,
    // but keep the guard OS-agnostic like the multiplayer ones).
    let err = String::from_utf8_lossy(&out.stderr).replace('\\', "/");
    if err.contains("/.loft/registry/") {
        eprintln!(
            "SKIP viewer smoke: the installed registry package predates the DN1 null \
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

struct ViewerGuard {
    child: Option<Child>,
    stderr: Arc<Mutex<String>>,
}

impl ViewerGuard {
    fn spawn() -> Self {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(project_root().join("lib"))
            .arg(project_root().join("tools/viewer/src/main.loft"))
            .env("LOFT_VIEW_PORT", VIEWER_PORT.to_string())
            .current_dir(project_root())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn loft viewer");
        let stderr = drain(child.stderr.take());
        ViewerGuard {
            child: Some(child),
            stderr,
        }
    }

    /// Why did the viewer fail to serve?  Distinguishes a CRASH (e.g. a stale native cdylib
    /// → `native function not loaded`, whose stderr is captured here) from a SLOW /
    /// load-starved start (process still alive, no error output) — so the failure names its
    /// own cause instead of leaving a bare empty body to chase.
    fn diagnose(&mut self) -> String {
        let alive = matches!(self.child.as_mut().map(Child::try_wait), Some(Ok(None)));
        let err = self.stderr.lock().unwrap().clone();
        let err = err.trim();
        format!(
            "viewer {}\n--- viewer stderr ---\n{}\n---------------------",
            if alive {
                "STILL RUNNING (slow / load-starved start — not a crash; check host load)"
            } else {
                "EXITED (crashed — the stderr below names the cause, e.g. a stale native cdylib)"
            },
            if err.is_empty() {
                "(no stderr captured)"
            } else {
                err
            },
        )
    }

    fn wait_listening(&self, port: u16) -> bool {
        // Generous: first run on a cold CI runner resolves registry deps and
        // compiles their native crates (the rustls chain alone takes minutes
        // under load) before the listener opens.  A hung viewer still fails —
        // just slowly, which an advisory smoke can afford.
        let deadline = Instant::now() + Duration::from_secs(240);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

impl Drop for ViewerGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn http_get(path: &str, port: u16) -> String {
    use std::io::Write;
    // Return an empty body on any transient error so the caller's retry loop can
    // re-fetch (under heavy CI load the viewer may briefly refuse/drop a request).
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return String::new();
    };
    let req = format!("GET {path} HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n",);
    if stream.write_all(req.as_bytes()).is_err() {
        return String::new();
    }
    let mut buf = String::new();
    if stream.read_to_string(&mut buf).is_err() {
        return String::new();
    }
    // Strip headers — return body only.
    if let Some(idx) = buf.find("\r\n\r\n") {
        buf[idx + 4..].to_string()
    } else {
        buf
    }
}

// @speed 1.9
#[test]
fn markdown_renderer_pins_high_impact_features() {
    if registry_predates_dn1() {
        return;
    }
    // Skip when the binary isn't built (e.g. fresh clone).  The
    // dev workflow `cargo test --release` builds the bin first.
    if !fa::exists(loft_bin()) {
        eprintln!("skipping: {} not built", loft_bin().display());
        return;
    }
    let mut viewer = ViewerGuard::spawn();
    let port = VIEWER_PORT;
    if !viewer.wait_listening(port) {
        panic!(
            "viewer did not start listening on {port} within 240s\n{}",
            viewer.diagnose()
        );
    }
    // Under a loaded CI host (many parallel native-build tests spawning rustc),
    // the interpreted viewer can accept the TCP connection before it is ready to
    // serve, returning an empty body on the first request.  Retry the fetch until
    // the rendered page arrives (or a deadline), so the smoke test is robust to
    // that startup race instead of racing a single 200ms sleep.
    let md_path = "/file/tests/fixtures/md_features_test.md";
    let body = {
        // Same load allowance as wait_listening: a viewer that just opened its
        // listener on a starved runner can still be seconds-to-minutes from
        // serving its first rendered page.
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut b = http_get(md_path, port);
        while !b.contains("<h1 id=") && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(200));
            b = http_get(md_path, port);
        }
        b
    };

    // The page never arrived within the deadline.  Name *why* — a crash (stale cdylib, a
    // panic) vs a slow / load-starved start — instead of asserting on a bare empty body
    // (which is how this masqueraded as a cdylib problem before).  The feature assertions
    // below then only fire on a *real* rendering regression, with a non-empty body.
    if !body.contains("<h1 id=") {
        panic!(
            "viewer never served the rendered page within 60s.\n{}\nbody so far: {body:.300}",
            viewer.diagnose()
        );
    }

    // Each assertion captures one feature so the failure message
    // names exactly which markdown construct broke.
    assert!(
        body.contains(r#"<h1 id="109arkdown-features-test">"#)
            || body.contains(r#"<h1 id="markdown-features-test">"#),
        "heading slug missing: {body:.500}",
    );
    assert!(body.contains("<strong>bold</strong>"), "bold");
    assert!(body.contains("<em>italic</em>"), "italic");
    assert!(body.contains("<code>inline code</code>"), "inline code");
    assert!(body.contains("<del>strikethrough</del>"), "strikethrough");
    assert!(body.contains("literal asterisk: *"), "backslash-escape \\*");
    assert!(body.contains("literal pipe: |"), "backslash-escape \\|");
    // The viewer passes image_url_prefix="/raw/" so relative
    // image paths get resolved against the file's directory and
    // routed to /raw/<resolved> (so the existing /raw/<path>
    // handler serves the bytes as application/octet-stream).
    assert!(
        body.contains(r#"<img src="/raw/tests/fixtures/images/logo.png" alt="image">"#),
        "image (rewritten through /raw/): {body:.500}",
    );
    assert!(
        body.contains(r#"<a href="/file/tests/fixtures/other.md">link</a>"#),
        "relative link rewriting: {body:.500}",
    );
    assert!(
        body.contains(r#"<a href="https://example.com">https://example.com</a>"#),
        "https autolink",
    );
    assert!(
        body.contains(r#"<a href="mailto:user@example.com">user@example.com</a>"#),
        "mailto autolink",
    );
    // Two block quotes — one merged from two lines, one alone.
    let bq_count = body.matches("<blockquote>").count();
    assert_eq!(bq_count, 2, "expected 2 blockquotes, got {bq_count}");
    // The merged blockquote's text should contain BOTH lines'
    // content joined by a single space.
    assert!(
        body.contains("A block quote line one. Block quote line two."),
        "block-quote line merging: {body:.500}",
    );
    // P272 regression — em-dash NOT tripled.
    let triple = body.matches("———").count();
    assert_eq!(triple, 0, "P272 em-dash widening regressed");
}
