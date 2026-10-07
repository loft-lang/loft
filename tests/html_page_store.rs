// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN146 F4 — a `--html` page reads a store out of its OWN filesystem.
//!
//! `wasm32-unknown-unknown` has no filesystem, so `Store::load`'s `std::fs::read` could
//! never answer in a browser and `store_load` returned `false` for every path — politely,
//! with nothing to act on. That made "a pack IS a loft store" true on desktop and
//! HTTP-only in a browser, which is the parity this plan exists to hold.
//!
//! The loader now falls back to the `loft_host_fs_*` bridge, which is what
//! `doc/loft-fs.js` serves `globalThis.loftBaseFS` through. So a page that CARRIES its
//! pack can read it with the same `store_load` call the desktop makes.
//!
//! The gate is a differential on ONE variable: the same emitted wasm, run twice, with
//! the store present in the page tree and absent from it. Absent must still answer
//! `false` — a loader that reported success for a file nobody supplied would pass a
//! "does it load?" check and be worse than the refusal it replaced.
//!
//! Skips cleanly without chrome / node / python3, in the shape of `tests/html_fonts.rs`.

use loft::file_access as fa;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
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
    (!path.is_empty()).then(|| PathBuf::from(path))
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

struct ServerGuard(Child);
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_server(py: &Path, dir: &Path, port: u16) -> Option<Child> {
    let child = loft::platform::process::harness_command(py)
        .args(["-m", "http.server", &port.to_string(), "-d"])
        .arg(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some(child);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Read the page's `<pre>` after the wait, via the harness's `--assert`.
fn page_output(url: &str, port: u16) -> String {
    let out = loft::platform::process::harness_command("node")
        .arg(repo_root().join("tools/html_render_check.mjs"))
        .arg(url)
        .args(["--wait-ms", "4000"])
        .args(["--port", &port.to_string()])
        .args(["--assert", "document.getElementById('out').textContent"])
        .output()
        .expect("invoke node harness");
    // The assertion fails by design (the value is text, not `true`), and the harness
    // prints it back — which is the only way to read a page's own output from here.
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// @PLN146 F4 — the same page reads its store when the page carries it, and refuses when
/// it does not.
// @speed 30.0
#[test]
fn a_browser_page_reads_a_store_out_of_its_own_filesystem() {
    let Some(_chrome) = any_of(&["google-chrome", "chromium", "chromium-browser", "chrome"]) else {
        eprintln!("SKIP: no chrome binary in PATH");
        return;
    };
    let Some(_node) = which("node") else {
        eprintln!("SKIP: node not installed");
        return;
    };
    let Some(py) = which("python3").or_else(|| which("python")) else {
        eprintln!("SKIP: python3 not installed");
        return;
    };
    let loft = repo_root().join("target/release/loft");
    if !fa::exists(&loft) {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }

    let dir = std::env::temp_dir().join("loft_html_page_store");
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create tempdir");

    // One program, two jobs: write the store natively, then read it back. The page runs
    // only the reading half, so the bytes it reads were produced by the desktop — which
    // is the direction a pack actually travels.
    let src = dir.join("page_store.loft");
    fa::write(
        &src,
        "struct Rec { r_key: text, r_n: integer }\n\
         fn main() {\n\
         \x20 if (env_variable(\"MAKE_STORE\") ?? \"\") != \"\" {\n\
         \x20   h: hash<Rec[r_key]> = [];\n\
         \x20   h[\"a\"] = Rec { r_key: \"a\", r_n: 7 };\n\
         \x20   println(\"persist={store_persist_copy(h, \\\"page.store\\\")}\");\n\
         \x20   return\n\
         \x20 }\n\
         \x20 q: hash<Rec[r_key]> = [];\n\
         \x20 ok = store_load(q, \"page.store\");\n\
         \x20 r = q[\"a\"];\n\
         \x20 println(\"load={ok} read={r != null}\");\n\
         }\n",
    )
    .expect("write source");

    let made = loft::platform::process::harness_command(&loft)
        .arg("--interpret")
        .arg(&src)
        .current_dir(&dir)
        .env("MAKE_STORE", "1")
        .output()
        .expect("invoke loft");
    assert!(
        fa::exists(dir.join("page.store")),
        "the fixture store was not written: {}",
        String::from_utf8_lossy(&made.stdout)
    );

    let html = dir.join("page_store.html");
    let built = loft::platform::process::harness_command(&loft)
        .args(["--html", html.to_str().expect("utf-8 path")])
        .arg(&src)
        .current_dir(&dir)
        .output()
        .expect("invoke loft --html");
    if !built.status.success() || !fa::exists(&html) {
        eprintln!(
            "SKIP: `loft --html` failed (no wasm toolchain?)\nstderr: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        return;
    }

    // The seeded variant: identical wasm, with the store handed to the page's own
    // filesystem the way `doc/loft-fs.js` documents (`globalThis.loftBaseFS`).
    let page = fa::read_to_string(&html).expect("read page");
    let bytes = fa::read(dir.join("page.store")).expect("read store");
    let b64 = loft::base64::encode(&bytes);
    let seed = format!(
        "<script>globalThis.loftBaseFS={{\"/page.store\":Uint8Array.from(atob(\"{b64}\"),c=>c.charCodeAt(0))}};</script>\n"
    );
    let seeded = page.replacen("<script>", &format!("{seed}<script>"), 1);
    assert!(seeded.len() > page.len(), "the seed was not spliced in");
    fa::write(dir.join("seeded.html"), &seeded).expect("write seeded page");

    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let Some(server) = spawn_server(&py, &dir, port) else {
        eprintln!("SKIP: failed to start python http.server");
        return;
    };
    let _guard = ServerGuard(server);

    let carried = page_output(
        &format!("http://127.0.0.1:{port}/seeded.html"),
        port.wrapping_add(1),
    );
    let bare = page_output(
        &format!("http://127.0.0.1:{port}/page_store.html"),
        port.wrapping_add(2),
    );

    assert!(
        carried.contains("load=true") && carried.contains("read=true"),
        "a page carrying its store in its own filesystem could not read it — the loader's \
         host-FS fallback is gone.\n{carried}"
    );
    // The control, and the half that matters: absent must still REFUSE. A loader that
    // answered success for a file nobody supplied would pass the assertion above and be
    // worse than the `false` it replaced.
    assert!(
        bare.contains("load=false") && bare.contains("read=false"),
        "a page with NO store in its filesystem reported a successful load — the gate \
         above proves nothing if this one does not hold.\n{bare}"
    );
}
