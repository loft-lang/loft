// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Browser-side WebGL rendering gate for `loft --html` artefacts.
//
// `tests/html_wasm.rs` already gates that the WASM instantiates and
// `loft_start` returns without trapping.  That catches WASM-side
// regressions but NOT JS-side failures: a `compileShader` error in the
// JS bridge (the recurring failure mode after JS-bridge or shader
// changes) prints `console.error` and the WASM keeps running cleanly.
// The HTML-instantiation test sees a clean exit, the canvas is blank,
// and the regression ships.
//
// This gate closes that loop with two layers:
//
// - **Layer 1** — fail on any JS console.error / Runtime.exceptionThrown
//   in the first 6 seconds of page load.  Catches shader compile
//   errors, missing WebGL2 contexts, init exceptions.
//
// - **Layer 2** — clip a screenshot to the canvas element, decode the
//   PNG, count distinct RGB triples.  Fail if below 20 distinct colors
//   (a blank canvas has 1-2; a working Brick Buster frame has 128).
//   Catches the "compiles clean, blank canvas" pattern that Layer 1
//   can't see — e.g. a successful shader compile that never gets used
//   to draw, or a draw call that no-ops because of a state-tracking
//   regression.
//
// Skips cleanly when prerequisites (node, chrome, wasm32 toolchain,
// host loft binary) are not available — same shape as
// `tests/html_wasm.rs`.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
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
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !p.join("Cargo.toml").exists() {
        if !p.pop() {
            return PathBuf::from(".");
        }
    }
    p
}

/// `make game` is not concurrent-safe (writes to a fixed
/// `/tmp/loft_html_opt.wasm` + the same output file).  Only one test
/// process should run it at a time.
fn build_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Pick a free TCP port for the per-test HTTP server.  Better than a
/// fixed port — parallel `cargo nextest` workers would collide.
fn pick_free_port() -> Option<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

/// Locate `doc/brick-buster.html` and skip the test if it's missing
/// or older than the source loft program.
///
/// We deliberately do NOT auto-build via `make game`: that would invoke
/// `cargo build --release` mid-`cargo test --release`, racing the
/// parallel rustc invocations from `tests/native.rs::native_dir` etc.
/// over `target/release/deps/`.  The user runs `make test-html-render`
/// (which `make game`s explicitly) or refreshes the artefact manually
/// when touching `--html` / `lib/graphics`.  Stale-artefact skip
/// matches the rest of the wasm test family.
///
/// **A skip here means the gate did not run.**  That is worth saying loudly,
/// because it is how this gate went dormant: it lived in a CI job that builds a
/// fresh wasm rlib but never rebuilds the bundle, so the bundle was always the
/// older of the two and the gate self-skipped on every run — indistinguishable,
/// in a green log, from a gate that passed.  It now runs in the `gallery` job
/// immediately after `make game`.  If you are reading this message in CI, that
/// wiring has regressed; locally it just means `make test-html-render`.
fn locate_fresh_brick_buster(root: &Path) -> Option<PathBuf> {
    let html = root.join("doc/brick-buster.html");
    if !html.exists() {
        eprintln!(
            "SKIP: doc/brick-buster.html not built — run `make game` first \
             (or `make test-html-render` to build + test in one step)"
        );
        return None;
    }
    let html_mtime = html.metadata().ok()?.modified().ok()?;
    // The bundle is stale if it predates EITHER the game source OR the
    // wasm32 runtime rlib it embeds — a rlib rebuilt after the bundle
    // (a common dev-machine state: `cargo build --target
    // wasm32-unknown-unknown` after `make game`) means the embedded wasm
    // no longer matches the runtime, so the render gate would fail on a
    // BUILD skew, not a real regression.  Skip with the rebuild command
    // instead of a false failure; CI rebuilds via `make game` so libloft
    // < bundle there.
    let stale_vs = [
        root.join("tools/brick-buster/25-brick-buster.loft"),
        // The atlas is DRAWN by this script into a pack the page carries via
        // `[[embed]]` (@PLN146 F4), so an edit to it changes the page's pixels
        // without touching the game source.  Left out, the bundle would look
        // fresh while showing the previous build's art.
        root.join("tools/brick-buster/pack_atlas.loft"),
        root.join("tools/brick-buster/assets/bb.blobs.store"),
        root.join("target/wasm32-unknown-unknown/release/libloft.rlib"),
    ];
    for dep in &stale_vs {
        if dep
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .is_some_and(|dep_mtime| html_mtime < dep_mtime)
        {
            eprintln!(
                "SKIP: the browser render gate DID NOT RUN — doc/brick-buster.html is \
                 older than {}, so it would test a bundle that no longer matches its \
                 runtime.  Rebuild with `make game` (or run `make test-html-render`, \
                 which does both).  In CI this gate belongs in the `gallery` job, right \
                 after its `make game` step.",
                dep.display()
            );
            return None;
        }
    }
    // Tag the build_lock as used so the helper doesn't get marked
    // dead-code; it's reserved for any future test in this file that
    // does need to serialise against itself (the current
    // brick_buster_* test is single, so the lock is currently a no-op).
    let _ = build_lock();
    Some(html)
}

/// Spawn `python3 -m http.server PORT -d doc/` as a child process.
/// The harness Node script fetches via http:// so file:// CORS rules
/// don't bite the WebGL2 / WASM imports.
fn spawn_doc_server(root: &Path, port: u16) -> Option<Child> {
    let py = which("python3").or_else(|| which("python"))?;
    let child = Command::new(py)
        .args(["-m", "http.server", &port.to_string(), "-d"])
        .arg(root.join("doc"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // Wait for the server to accept connections (max 2 s)
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some(child);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Brick Buster loads cleanly in headless WebGL2 — no shader compile
/// errors, no exceptions, no `console.error` calls during a 6-second
/// startup window.
///
/// Catches the recurring failure mode: a JS-bridge or loft-shader
/// change ships, the WASM still returns cleanly (so `tests/html_wasm.rs`
/// passes), but every frame logs a `compileShader` error and renders a
/// blank canvas.
#[test]
fn brick_buster_browser_renders_without_console_errors() {
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
    if !harness.exists() {
        eprintln!("SKIP: tools/html_render_check.mjs missing");
        return;
    }
    let loft_bin = root.join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release` first)");
        return;
    }
    let Some(_html_path) = locate_fresh_brick_buster(&root) else {
        return;
    };
    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let Some(mut server) = spawn_doc_server(&root, port) else {
        eprintln!("SKIP: failed to start python http.server");
        return;
    };
    // Make sure the server gets cleaned up even if the assertion below panics.
    let _server_guard = ServerGuard(&mut server);

    let cdp_port = port.wrapping_add(1);
    let url = format!("http://127.0.0.1:{port}/brick-buster.html");
    let screenshot = std::env::temp_dir().join("brick_buster_render_test.png");

    let out = Command::new("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", "6000"])
        .args(["--port", &cdp_port.to_string()])
        .arg("--screenshot")
        .arg(&screenshot)
        // Layer 2: capture a clipped screenshot of the canvas element,
        // decode it, and assert at least 20 distinct RGB colors.  Catches
        // the "compiles clean, blank canvas" pattern that Layer 1 (no
        // console errors) can't see.  Empirical baseline: working Brick
        // Buster frame yields 128 distinct colors; a blank canvas (only
        // clearColor) has 1-2.
        .args(["--canvas", "#c"])
        .args(["--canvas-min-colors", "20"])
        // Layer 3 — the program's OWN verdict, which the two layers above
        // cannot reach.  `load_atlas` answers a 1x1 canvas when the sprite
        // pack does not load and says so on stdout; the game then runs
        // perfectly well and draws its procedural fallback, so Layer 1 sees
        // no console error and Layer 2 counted 767 distinct colours on a page
        // whose sprites were entirely missing.  A pack read through the host
        // filesystem is exactly the half that had no host arm until
        // `LocalFileProvider` grew one, and nothing here would have noticed.
        //
        // `#out` is where the page routes `println`, live, so the check is a
        // plain substring: every one of `load_atlas`'s three failure lines
        // contains "sprite pack" and no success path mentions it.
        .args([
            "--assert",
            "!(document.getElementById('out')||{}).textContent\
             ?.includes('sprite pack')",
        ])
        // The page favicon 404 is benign; the harness filters generic
        // "Failed to load resource" automatically.  Swiftshader emits
        // a one-line GPU-stall performance warning at info level — we
        // only fail on `error`-level events, so no allow-list needed.
        .output()
        .expect("invoke node harness");

    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "Brick Buster headless render gate failed.\nURL: {url}\n\
         stdout:\n{stdout}\nstderr:\n{stderr}\n\
         (screenshot at {})",
        screenshot.display()
    );
}

/// The `--html` target allocates its canvas at the display's real resolution,
/// so a page is crisp on a high-density display instead of upscaled by the
/// browser (loft#1018).
///
/// **This is invisible to the gate above.** An upscaled canvas logs no console
/// error and has just as many distinct colours — it is merely soft. So the
/// browser is forced to a device ratio of 2 and the page is asked about the one
/// relationship that has to hold:
///
/// ```text
/// canvas.width == round(cssBoxWidth * devicePixelRatio)
/// ```
///
/// That single expression covers **both** halves of the fix, which is why it is
/// the one asserted. Drop the backing-store scaling and `canvas.width` stays at
/// the CSS width, so it fails. Drop the explicit CSS size and the box grows to
/// the attribute size, the ratio collapses to 1, and it fails again — and that
/// second half is the one that matters for input, because the pointer is scaled
/// by exactly `canvas.width / rect.width`. A crisp picture whose clicks land at
/// `1/ratio` of where the user pointed passes every other check in this file.
///
/// The ratio itself is asserted first: if the flag were ever ignored the page
/// would run at 1, where the relationship holds trivially and the gate would
/// pass without measuring anything.
#[test]
fn html_canvas_is_allocated_at_the_display_resolution() {
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
    if !harness.exists() {
        eprintln!("SKIP: tools/html_render_check.mjs missing");
        return;
    }
    let Some(_html_path) = locate_fresh_brick_buster(&root) else {
        return;
    };
    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let Some(mut server) = spawn_doc_server(&root, port) else {
        eprintln!("SKIP: failed to start python http.server");
        return;
    };
    let _server_guard = ServerGuard(&mut server);

    let cdp_port = port.wrapping_add(1);
    let url = format!("http://127.0.0.1:{port}/brick-buster.html");
    let assertion = "(() => { \
        const c = document.querySelector('#c'); \
        const r = c.getBoundingClientRect(); \
        return window.devicePixelRatio === 2 \
            && c.width  === Math.round(r.width  * window.devicePixelRatio) \
            && c.height === Math.round(r.height * window.devicePixelRatio); \
    })()";

    let out = Command::new("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", "6000"])
        .args(["--port", &cdp_port.to_string()])
        .args(["--device-scale-factor", "2"])
        .args(["--assert", assertion])
        .output()
        .expect("invoke node harness");

    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the canvas is not allocated at the display resolution (loft#1018).\n\
         URL: {url}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

struct ServerGuard<'a>(&'a mut Child);
impl Drop for ServerGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
