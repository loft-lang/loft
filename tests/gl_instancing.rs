// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! Browser gate for the four `--html` INSTANCING primitives (@PLN144, A3's
//! prerequisite).
//!
//! `gl_upload_instance_buffer`, `gl_instance_attrib`, `gl_draw_instanced` and
//! `gl_update_buffer` were absent from the browser shim.  A0 found it the way the
//! boundary is meant to be found — the page BUILT, each call returned its zero
//! value, and nothing drew.
//!
//! Bridging them makes the build-time surface check pass, and that check is
//! exactly the signal that has been wrong three times: loft#737's text stubs,
//! E1's audio stubs, and this.  So the assertions here read PIXELS back from the
//! framebuffer, and the load-bearing one is that coverage grows with the instance
//! count — without `vertexAttribDivisor` every instance reads offset[0], so 64
//! would paint precisely what 1 paints and every other check would still pass.
//!
//! `tests/data/gl_instancing_probe.html` holds them; a failure is a
//! `console.error`, which `tools/html_render_check.mjs` turns into a non-zero exit.
//!
//! Skips cleanly without chrome / node / python3, in the same shape as
//! `tests/html_render.rs`.

use loft::file_access as fa;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn which(cmd: &str) -> Option<PathBuf> {
    let out = Command::new("sh")
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
    let child = Command::new(py)
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
fn instancing_bridge_draws_every_instance() {
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

    let url = format!("http://127.0.0.1:{port}/tests/data/gl_instancing_probe.html");
    let out = Command::new("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", "3000"])
        .args(["--port", &port.wrapping_add(1).to_string()])
        .output()
        .expect("invoke node harness");

    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    assert!(
        out.status.success(),
        "instancing bridge probe failed.\nURL: {url}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
