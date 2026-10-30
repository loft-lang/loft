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
    let loft = std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"));
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
        .args([
            "--native-rust",
            "--html",
            html.to_str().expect("utf-8 path"),
        ])
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
    // The image travels with its layout sidecar, which `store_persist_copy` wrote beside it:
    // a page refuses an image whose layout nothing records (`@FR-S-Gate`).
    let bytes = fa::read(dir.join("page.store")).expect("read store");
    let side = fa::read(dir.join("page.store.dschema")).expect("read sidecar");
    let b64 = loft::base64::encode(&bytes);
    let side64 = loft::base64::encode(&side);
    let seed = format!(
        "<script>globalThis.loftBaseFS={{\"/page.store\":Uint8Array.from(atob(\"{b64}\"),c=>c.charCodeAt(0)),\
         \"/page.store.dschema\":Uint8Array.from(atob(\"{side64}\"),c=>c.charCodeAt(0))}};</script>\n"
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

/// The page's python, once chrome, node, python3 and the loft binary are all present;
/// `None` (with the SKIP reason printed) otherwise.
fn page_prerequisites() -> Option<PathBuf> {
    if any_of(&["google-chrome", "chromium", "chromium-browser", "chrome"]).is_none() {
        eprintln!("SKIP: no chrome binary in PATH");
        return None;
    }
    if which("node").is_none() {
        eprintln!("SKIP: node not installed");
        return None;
    }
    let Some(py) = which("python3").or_else(|| which("python")) else {
        eprintln!("SKIP: python3 not installed");
        return None;
    };
    if !fa::exists(PathBuf::from(env!("CARGO_BIN_EXE_loft"))) {
        eprintln!("SKIP: target/release/loft not built");
        return None;
    }
    Some(py)
}

/// `page` with `files` handed to its own filesystem (`globalThis.loftBaseFS`).
fn seed_page(page: &str, files: &[(&str, Vec<u8>)]) -> String {
    let entries: Vec<String> = files
        .iter()
        .map(|(name, bytes)| {
            format!(
                "\"/{name}\":Uint8Array.from(atob(\"{}\"),c=>c.charCodeAt(0))",
                loft::base64::encode(bytes)
            )
        })
        .collect();
    let seed = format!(
        "<script>globalThis.loftBaseFS={{{}}};</script>\n",
        entries.join(",")
    );
    page.replacen("<script>", &format!("{seed}<script>"), 1)
}

/// `@FR-L-Sound` — a page converts a store written under an OLDER layout, as native does.
///
/// The layout gate reads the sidecar from the page's filesystem and answers "convert"; the
/// converter must then open the same two files there.  It read them from the program's
/// filesystem only, so every page refused the conversion its gate had just approved — the
/// gallery's sprite pack loaded natively and not in the browser (loft#2237's key order made
/// it an older layout).  The control is the same page without the image: still a refusal.
// @speed 30.0
#[test]
fn a_browser_page_converts_a_store_written_under_an_older_layout() {
    let Some(py) = page_prerequisites() else {
        return;
    };
    let loft = std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"));
    let dir = std::env::temp_dir().join("loft_html_page_store_old");
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create tempdir");

    // The writer's `Rec` has no `r_m`; the reader's has.  So the image is an older layout
    // for the reader, and its load converts: `r_n` keeps 7, `r_m` is filled with 0.
    fa::write(
        dir.join("writer.loft"),
        "struct Rec { r_key: text, r_n: integer }\n\
         fn main() {\n\
         \x20 h: hash<Rec[r_key]> = [];\n\
         \x20 h[\"a\"] = Rec { r_key: \"a\", r_n: 7 };\n\
         \x20 println(\"persist={store_persist_copy(h, \\\"old.store\\\")}\");\n\
         }\n",
    )
    .expect("write writer");
    let reader = dir.join("reader.loft");
    fa::write(
        &reader,
        "struct Rec { r_key: text, r_n: integer, r_m: integer }\n\
         fn main() {\n\
         \x20 q: hash<Rec[r_key]> = [];\n\
         \x20 ok = store_load(q, \"old.store\");\n\
         \x20 r = q[\"a\"];\n\
         \x20 println(\"load={ok} read={r != null} n={r.r_n} m={r.r_m}\");\n\
         }\n",
    )
    .expect("write reader");

    let made = loft::platform::process::harness_command(&loft)
        .arg("--interpret")
        .arg(dir.join("writer.loft"))
        .current_dir(&dir)
        .output()
        .expect("invoke loft");
    assert!(
        fa::exists(dir.join("old.store")) && fa::exists(dir.join("old.store.dschema")),
        "the older-layout fixture was not written: {}",
        String::from_utf8_lossy(&made.stdout)
    );
    // The native answer is the oracle the page must give.
    let native = loft::platform::process::harness_command(&loft)
        .arg("--interpret")
        .arg(&reader)
        .current_dir(&dir)
        .output()
        .expect("invoke loft");
    let native = String::from_utf8_lossy(&native.stdout).to_string();
    assert!(
        native.contains("load=true read=true n=7 m=0"),
        "the native control did not convert the older layout: {native}"
    );

    let html = dir.join("reader.html");
    let built = loft::platform::process::harness_command(&loft)
        .args([
            "--native-rust",
            "--html",
            html.to_str().expect("utf-8 path"),
        ])
        .arg(&reader)
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
    let page = fa::read_to_string(&html).expect("read page");
    let image = fa::read(dir.join("old.store")).expect("read store");
    let side = fa::read(dir.join("old.store.dschema")).expect("read sidecar");
    let carried = seed_page(
        &page,
        &[("old.store", image), ("old.store.dschema", side.clone())],
    );
    // The control: the sidecar alone.  The gate still judges the layout older, and with no
    // image to convert the load must refuse rather than answer an empty success.
    let sidecar_only = seed_page(&page, &[("old.store.dschema", side)]);
    fa::write(dir.join("carried.html"), &carried).expect("write carried page");
    fa::write(dir.join("sidecar_only.html"), &sidecar_only).expect("write control page");

    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let Some(server) = spawn_server(&py, &dir, port) else {
        eprintln!("SKIP: failed to start python http.server");
        return;
    };
    let _guard = ServerGuard(server);
    let converted = page_output(
        &format!("http://127.0.0.1:{port}/carried.html"),
        port.wrapping_add(1),
    );
    let refused = page_output(
        &format!("http://127.0.0.1:{port}/sidecar_only.html"),
        port.wrapping_add(2),
    );
    if converted.contains("SKIP:") || refused.contains("SKIP:") {
        eprintln!("SKIP: the page harness did not observe a page\n{converted}\n{refused}");
        return;
    }
    assert!(
        converted.contains("load=true read=true n=7 m=0"),
        "a page carrying a store written under an older layout did not convert it as native \
         did ({native}) — the converter cannot read the page's own filesystem.\n{converted}"
    );
    assert!(
        refused.contains("load=false") && refused.contains("read=false"),
        "a page with no image reported a converted load.\n{refused}"
    );
}
