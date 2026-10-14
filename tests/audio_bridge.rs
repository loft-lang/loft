// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! Browser gate for the `--html` AUDIO bridge (@PLN146 `E1`).
//!
//! `loft_audio_load` answered `i32::MIN` and `audio_play`/`stop`/`set_volume` were
//! no-ops, so a `--html` game could only make procedural noise: no music, no
//! sampled effects.  The failure mode is loft#737's exactly — the imports EXIST,
//! so every import-shape check passes while nothing is audible — and the lesson
//! that file records applies unchanged: **a test that only asserts the import is
//! defined would repeat the mistake.**
//!
//! So the load-bearing assertions render through an `OfflineAudioContext` and
//! measure SAMPLES: full-volume playback must have peak amplitude, a quarter
//! volume must actually attenuate, and a MISSING file must still answer the null
//! sentinel — which is what stops the fix from being "return a constant handle".
//!
//! `tests/data/audio_bridge_probe.html` holds the assertions; it reports a failure as
//! a `console.error`, which `tools/html_render_check.mjs` turns into a non-zero
//! exit.  There is no canvas layer here — audio has no pixels, so the sample
//! measurement is the whole of the evidence.
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
fn audio_bridge_produces_real_samples() {
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
    if !fa::exists(root.join("tests/data/tone440.wav")) {
        eprintln!("SKIP: tests/data/tone440.wav missing");
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

    let url = format!("http://127.0.0.1:{port}/tests/data/audio_bridge_probe.html");
    let out = loft::platform::process::harness_command("node")
        .arg(&harness)
        .arg(&url)
        // Decode plus fifteen offline renders; the probe polls for each decode
        // rather than sleeping, so this is a ceiling and not a delay it spends.
        .args(["--wait-ms", "6000"])
        // ⚠ WITHOUT THIS the gate is vacuous.  The harness fails on a
        // `console.error`, so a page that never REACHES its checks — one that
        // threw somewhere the catch does not cover, or simply ran past the wait —
        // passes by saying nothing.  Measured: dropping the `looping` flag in the
        // bridge left this test green until the assertion below was added.
        .args([
            "--assert",
            "document.getElementById('out').textContent === 'audio bridge ok'",
        ])
        .args(["--port", &port.wrapping_add(1).to_string()])
        .output()
        .expect("invoke node harness");

    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    assert!(
        out.status.success(),
        "audio bridge probe failed.\nURL: {url}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
