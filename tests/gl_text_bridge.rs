// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! Browser gate for the `--html` text + texture bridge (loft#737, loft#738).
//!
//! These functions were present as no-op stubs, which is the failure mode that
//! matters here: the import EXISTS, so `loft targets` reported the builtin as
//! available and every import-shape check passed, while `graphics::draw_text`
//! compiled, ran, and drew nothing.  A test that only asserts the import is
//! defined would repeat exactly that mistake, so this one runs the bridge in a
//! real browser and checks what it PRODUCES: glyph coverage with ink in it,
//! metrics that scale with the font size, and texture handles that are handles.
//!
//! `tests/data/gl_text_probe.html` holds the assertions; it reports a failure as
//! a `console.error`, which `tools/html_render_check.mjs` turns into a non-zero
//! exit.  It also paints the rasterised coverage into a second canvas, so the
//! harness's distinct-colour layer independently confirms real antialiased
//! glyphs rather than a uniform block.
//!
//! Skips cleanly without chrome / node / python3, in the same shape as
//! `tests/html_render.rs`.

use loft::file_access as fa;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

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

fn any_of(cmds: &[&str]) -> Option<PathBuf> {
    cmds.iter().find_map(|c| which(c))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn pick_free_port() -> Option<u16> {
    let l = TcpListener::bind("127.0.0.1:0").ok()?;
    let p = l.local_addr().ok()?.port();
    drop(l);
    Some(p)
}

struct ServerGuard<'a>(&'a mut Child);
impl Drop for ServerGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Serve the repo ROOT, so the probe page can pull in `doc/loft-gl-wasm.js` —
/// the very file under test — by its real path rather than a copy that could
/// drift from it.
fn spawn_root_server(port: u16) -> Option<Child> {
    let py = which("python3").or_else(|| which("python"))?;
    let child = loft::platform::process::harness_command(py)
        .args(["-m", "http.server", &port.to_string(), "-d"])
        .arg(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some(child);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

// @speed 3.8
#[test]
fn text_and_texture_bridge_produces_real_pixels() {
    let Some(_chrome) = any_of(&["google-chrome", "chromium", "chromium-browser", "chrome"]) else {
        eprintln!("SKIP: no chrome binary in PATH");
        return;
    };
    let Some(_node) = which("node") else {
        eprintln!("SKIP: node not installed");
        return;
    };
    let root = repo_root();
    let harness = root.join("tools/html_render_check.mjs");
    if !fa::exists(&harness) {
        eprintln!("SKIP: tools/html_render_check.mjs missing");
        return;
    }
    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let Some(mut server) = spawn_root_server(port) else {
        eprintln!("SKIP: failed to start python http.server");
        return;
    };
    let _guard = ServerGuard(&mut server);

    let url = format!("http://127.0.0.1:{port}/tests/data/gl_text_probe.html");
    let screenshot = std::env::temp_dir().join("gl_text_bridge_probe.png");
    let out = loft::platform::process::harness_command("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", "3000"])
        .args(["--port", &port.wrapping_add(1).to_string()])
        .arg("--screenshot")
        .arg(&screenshot)
        // The coverage canvas: antialiased glyphs give ~138 distinct greys, a
        // blank or uniform block gives 1-2.  This is the layer that would have
        // caught the stubs — they returned plausible NUMBERS, but no pixels.
        .args(["--canvas", "#c"])
        .args(["--canvas-min-colors", "20"])
        .output()
        .expect("invoke node harness");

    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    assert!(
        out.status.success(),
        "text/texture bridge probe failed.\nURL: {url}\nstdout:\n{}\nstderr:\n{}\n\
         (screenshot at {})",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        screenshot.display(),
    );
}
