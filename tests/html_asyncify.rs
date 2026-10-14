// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Browser-side ASYNCIFY-RESUME gate for `loft --html` artefacts (issue #450).
//
// `tests/html_wasm.rs` runs a `--html` bundle SYNCHRONOUSLY (loft_start →
// exit) so it never drives a suspend/resume cycle, and `tests/html_render.rs`
// only checks that a GL page renders SOME frame without a console error.
// Neither catches the #450 class: an asyncify program (frame_yield /
// gl_swap_buffers / ws_yield) that suspends but never RESUMES past its first
// suspend.  The buggy resume loop still printed the first line and drew the
// first frame, so both existing gates stayed green while every asyncify
// program was stuck on iteration 0.
//
// This gate builds a self-contained program that suspends five times through
// the `loft_gl_swap_buffers` asyncify point (no external graphics lib — it
// binds the native symbol directly) and asserts, in real headless Chromium,
// that its `#out` text reaches the final `done` line.  It runs the page twice:
//
//  - VISIBLE — the resume loop is driven by requestAnimationFrame.
//  - HIDDEN  — document.hidden is forced true and requestAnimationFrame is
//    killed before the page runs (the headless / backgrounded-tab condition
//    where Chromium pauses rAF).  Only the non-rAF MessageChannel pump can
//    drive the page here, so this leg guards the scheduler half of the fix.
//  - STALLED — the page stays VISIBLE but requestAnimationFrame never fires:
//    a throttled or occluded renderer that withholds frames without setting
//    document.hidden (loft#1830 — CI's loaded ubuntu runner stopped the visible
//    leg at `tick 0`).  Only the scheduler's fallback can drive it.
//
// Skips cleanly when prerequisites (node, chrome, wasm32 toolchain, the host
// loft binary) are missing — same shape as the sibling html_* gates.

use loft::file_access as fa;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// The program suspends five times through loft_gl_swap_buffers, then prints
// `done`.  With the pre-#450 AsyncifyCtrl the page sticks on `tick 0`; with the
// fix it reaches `done`.
const SOURCE: &str = "\
fn gl_swap_buffers();
#native \"loft_gl_swap_buffers\"
fn main() {
  println(\"start\");
  i = 0;
  while i < 5 { println(\"tick {i}\"); gl_swap_buffers(); i = i + 1; }
  println(\"done\");
}
";

fn which(cmd: &str) -> Option<PathBuf> {
    let out = loft::platform::process::harness_command("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn any_chrome() -> bool {
    ["google-chrome", "chromium", "chromium-browser", "chrome"]
        .iter()
        .any(|c| which(c).is_some())
}

fn wasm32_target_installed() -> bool {
    loft::platform::process::harness_command("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.lines().any(|l| l == "wasm32-unknown-unknown"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The `loft --html` driver writes a fixed `/tmp/loft_html*.rs` path; serialise
/// concurrent builds so parallel invocations don't clobber each other.
fn build_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn pick_free_port() -> Option<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

/// Build the program to `--html` and return its path, or `None` to skip.
fn build_html(root: &Path) -> Option<PathBuf> {
    let loft_bin = root.join("target/release/loft");
    if !fa::exists(&loft_bin) {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release` first)");
        return None;
    }
    // @PLN100 Slice 1 — no manual rlib freshness guard: `loft --html` auto-builds
    // its own isolated wasm runtime rlib on stale/missing, so a stale or
    // wasm-bindgen-stomped rlib can no longer reach this path.

    let tmp = std::env::temp_dir().join("loft_html_asyncify_resume");
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).expect("create per-test dir");
    let src = tmp.join("main.loft");
    let html = tmp.join("asyncify_resume.html");
    fa::write(&src, SOURCE).expect("write source");

    let _guard = build_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let out = loft::platform::process::harness_command(&loft_bin)
        .args(["--html", html.to_str().unwrap()])
        .arg(src.to_str().unwrap())
        .output()
        .expect("invoke loft --html");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "loft --html build failed:\n{stderr}");
    // The page provides `loft_gl_swap_buffers` itself, so the host's native libraries play
    // no part in a browser build and an unloaded one is nothing to report (loft#1830).
    assert!(
        !stderr.contains("did not load"),
        "a --html build reported a host native library:\n{stderr}"
    );
    Some(html)
}

/// Run the headless-Chromium harness against `html`, asserting `#out` reaches
/// `done`.  `mode` is the harness flag that withholds frames: `--hidden` (the
/// headless/backgrounded condition) or `--stalled-raf` (a visible page that gets
/// no frames); `None` is the ordinary visible page.
fn assert_resumes(root: &Path, html: &Path, mode: Option<&str>) {
    let harness = root.join("tools/html_asyncify_check.mjs");
    assert!(
        fa::exists(&harness),
        "tools/html_asyncify_check.mjs missing"
    );
    let port = pick_free_port().expect("pick a free port");

    let mut cmd = loft::platform::process::harness_command("node");
    cmd.arg(&harness)
        .arg(html)
        .args(["--expect", "done"])
        .args(["--wait-ms", "5000"])
        .args(["--port", &port.to_string()]);
    if let Some(flag) = mode {
        cmd.arg(flag);
    }
    let out = cmd.output().expect("invoke node harness");

    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    assert!(
        out.status.success(),
        "asyncify resume gate failed (mode={mode:?}) — the page did not reach \
         `done` past its first suspend (issue #450).\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// A `--html` asyncify program must RESUME past its first suspend and run to
/// completion — both on a visible page (requestAnimationFrame) and on a
/// hidden/headless page (the MessageChannel pump).  Guards issue #450.
// @speed 0.8
#[test]
fn html_asyncify_program_resumes_to_completion() {
    if !any_chrome() {
        eprintln!("SKIP: no chrome binary in PATH");
        return;
    }
    if which("node").is_none() {
        eprintln!("SKIP: node not installed");
        return;
    }
    if !wasm32_target_installed() {
        eprintln!("SKIP: rustup target wasm32-unknown-unknown not installed");
        return;
    }
    let root = repo_root();
    let Some(html) = build_html(&root) else {
        return;
    };
    assert_resumes(&root, &html, None); // visible — requestAnimationFrame
    assert_resumes(&root, &html, Some("--hidden")); // hidden  — MessageChannel pump
    assert_resumes(&root, &html, Some("--stalled-raf")); // visible, no frames — the fallback
}
