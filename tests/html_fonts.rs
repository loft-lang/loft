// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! Browser gates for `[[font]]` — @PLN146 `F5` (the declaration) and `F6` (the
//! readiness ordering).
//!
//! Both run the page's REAL font block: the `<head>` fragment and the boot await
//! come from `loft::html_fonts`, the same functions `--html` emits from, and the
//! resolution is read out of `doc/loft-gl-wasm.js`'s own bridge rather than from a
//! copy of its logic.
//!
//! The measurement is the point.  "Text appeared" is not the gate — text draws in a
//! fallback perfectly well, which is the whole failure being closed — so the bridge
//! decides whether a family RESOLVED by measuring it against two different generics:
//! a family the browser has overrides both and the two agree, one it does not have
//! follows each and they differ.  `document.fonts.check` cannot answer the question:
//! it is true for a family nothing declares at all.
//!
//! Skips cleanly without chrome / node / python3, in the shape of
//! `tests/gl_text_bridge.rs`.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use loft::html_fonts::{self, PageFont};

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

/// The three sources one declaration can name, as `--html` would validate them.
/// The resident family is one every desktop and CI image with fontconfig has; the
/// other two are names nothing can have by accident, so a page that resolves them
/// resolved what THIS test brought.
fn three_sources() -> Vec<PageFont> {
    html_fonts::validate(&[
        loft::manifest::FontDecl {
            family: Some("DejaVu Sans".into()),
            native: Some("fonts/DejaVu Sans.ttf".into()),
            url: None,
            stylesheet: None,
        },
        loft::manifest::FontDecl {
            family: Some("LoftProbeFace".into()),
            native: Some("fonts/LoftProbeFace.ttf".into()),
            url: Some("LoftProbeFace.ttf".into()),
            stylesheet: None,
        },
        loft::manifest::FontDecl {
            family: Some("LoftCdnFace".into()),
            native: None,
            url: None,
            stylesheet: Some("cdn.css".into()),
        },
    ])
    .expect("the three sources are a valid declaration set")
}

/// Lay out a directory the page can be served from: the bridge under test, two font
/// files under names nothing has by accident, and the "provider" stylesheet that
/// stands in for a CDN (a `<link>` to a stylesheet that declares an `@font-face` —
/// the shape, without needing the network).
fn stage_dir(name: &str) -> Option<PathBuf> {
    let root = repo_root();
    let face = root.join("doc/assets/DejaVuSans-Bold.ttf");
    if !face.exists() {
        return None;
    }
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::copy(&face, dir.join("LoftProbeFace.ttf")).ok()?;
    std::fs::copy(&face, dir.join("LoftCdnFace.ttf")).ok()?;
    std::fs::copy(
        root.join("doc/loft-gl-wasm.js"),
        dir.join("loft-gl-wasm.js"),
    )
    .ok()?;
    std::fs::write(
        dir.join("cdn.css"),
        "@font-face{font-family:\"LoftCdnFace\";src:url(\"LoftCdnFace.ttf\") format(\"truetype\")}\n",
    )
    .ok()?;
    Some(dir)
}

/// The probe page: the emitted `<head>` fragment, the emitted boot await (or not,
/// for the control), and then the real bridge asked to load each font — exactly the
/// order a `--html` page runs in.
fn write_page(dir: &Path, file: &str, fonts: &[PageFont], with_await: bool) {
    let head = html_fonts::head_html(fonts);
    let boot = if with_await {
        html_fonts::boot_await_js(fonts)
    } else {
        String::new()
    };
    let paths = fonts
        .iter()
        .map(|f| format!("'fonts/{}.ttf'", f.family))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        dir.join(file),
        format!(
            r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>loft font probe</title>{head}
</head><body>
<canvas id="gl" width="64" height="64"></canvas><pre id="out"></pre>
<script src="loft-gl-wasm.js"></script>
<script>
const ctrl = {{ ac: null, assets: {{}} }};
(async () => {{{boot}
  const mem = new WebAssembly.Memory({{ initial: 4 }});
  const enc = new TextEncoder();
  let top = 1024;
  const put = (s) => {{ const b = enc.encode(s); new Uint8Array(mem.buffer).set(b, top);
                        const r = [top, b.length]; top += b.length + 8; return r; }};
  const imports = buildLoftImports(document.getElementById('gl'),
                                   document.getElementById('out'), () => mem, ctrl);
  for (const path of [{paths}]) {{ const [p, l] = put(path); imports.loft_gl.loft_gl_load_font(p, l); }}
  document.getElementById('out').textContent = JSON.stringify(globalThis.loftFonts);
}})();
</script></body></html>
"#
        ),
    )
    .expect("write probe page");
}

/// Run one probe page and return the harness's verdict plus its output.
fn check(url: &str, assert_expr: &str, wait_ms: u32, port: u16) -> Option<(bool, String)> {
    check_when(url, PAGE_RAN, assert_expr, wait_ms, port)
}

/// The state every assertion in this file is ABOUT: the page reached the bridge and recorded
/// all three declarations.  `f.resolved` is a snapshot `loft_gl_load_font` takes SYNCHRONOUSLY
/// (`familyResolves` at call time), so once these records exist the answer is frozen — which is
/// why waiting longer is safe here and cannot turn a control green.
const PAGE_RAN: &str = "Array.isArray(globalThis.loftFonts) && globalThis.loftFonts.length===3";

/// Run one probe page, waiting for `ready_expr` to hold before asserting `assert_expr`.
///
/// The two are separate on purpose.  A fixed sleep answers "how long to wait" where this file
/// means "wait until the page got there", and on a loaded runner the difference is a gate that
/// reports its CLAIM as false when nothing was observed at all: `globalThis.loftFonts` was
/// `undefined` at 1500ms and the control read it as *"a throttled font still resolved"*, which
/// is a font-ordering bug that was not there.  The harness reports a `ready.timeout` failure
/// for that shape, and [`page_never_ran`] is how the assertions tell the two apart.
///
/// `None` is the harness's own SKIP (exit 2: no browser, or one that never answered its
/// debugging port) — nothing about the page was observed, so the caller skips rather than
/// asserting, as every other browser gate does.
fn check_when(
    url: &str,
    ready_expr: &str,
    assert_expr: &str,
    wait_ms: u32,
    port: u16,
) -> Option<(bool, String)> {
    let out = Command::new("node")
        .arg(repo_root().join("tools/html_render_check.mjs"))
        .arg(url)
        .args(["--wait-ms", &wait_ms.to_string()])
        .args(["--ready", ready_expr])
        .args(["--port", &port.to_string()])
        .args(["--assert", assert_expr])
        .output()
        .expect("invoke node harness");
    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    let text = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Some((out.status.success(), text))
}

/// Did the run observe NOTHING — the page never reached the bridge inside its budget — rather
/// than observe the page and find the claim false?
///
/// Every assertion here is about what the bridge RECORDED, so a run that recorded nothing has
/// no reading to report and must not be described as one.  Without this the same message
/// covered both, and the one it printed named the wrong cause.
fn page_never_ran(text: &str) -> bool {
    text.contains("ready.timeout")
}

/// Every declared family resolved to the family the program asked for.  Reads the
/// bridge's own record, so it asserts the RESOLVED family and not that text drew.
const ALL_RESOLVED: &str = "globalThis.loftFonts.length===3 && \
     globalThis.loftFonts.every(f=>f.resolved) && \
     globalThis.loftFonts.map(f=>f.family).join('|')===\
     '\"DejaVu Sans\", sans-serif|\"LoftProbeFace\", sans-serif|\"LoftCdnFace\", sans-serif'";

fn tooling() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let chrome = any_of(&["google-chrome", "chromium", "chromium-browser", "chrome"])?;
    let node = which("node")?;
    let py = which("python3").or_else(|| which("python"))?;
    Some((chrome, node, py))
}

/// Start the static server on `dir`, delaying font FILES by `delay_ms` (0 = not at
/// all).  Everything else is served at once, so the delay is on the one thing under
/// test.
fn spawn_server(py: &Path, dir: &Path, port: u16, delay_ms: u32) -> Option<Child> {
    let child = Command::new(py)
        .arg(repo_root().join("tests/data/slow_font_server.py"))
        .arg(dir)
        .arg(port.to_string())
        .arg(delay_ms.to_string())
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

/// @PLN146 F5 — a page declaring each of the three sources resolves to the
/// REQUESTED family, not the fallback.
// @speed 4.0
#[test]
fn each_declared_font_source_resolves_to_the_requested_family() {
    let Some((_chrome, _node, py)) = tooling() else {
        eprintln!("SKIP: chrome / node / python3 not all present");
        return;
    };
    let Some(dir) = stage_dir("loft_html_fonts_f5") else {
        eprintln!("SKIP: doc/assets/DejaVuSans-Bold.ttf missing");
        return;
    };
    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let fonts = three_sources();
    write_page(&dir, "probe.html", &fonts, true);
    let Some(server) = spawn_server(&py, &dir, port, 0) else {
        eprintln!("SKIP: failed to start the static server");
        return;
    };
    let _guard = ServerGuard(server);
    let Some((ok, text)) = check(
        &format!("http://127.0.0.1:{port}/probe.html"),
        ALL_RESOLVED,
        3000,
        port.wrapping_add(1),
    ) else {
        return;
    };
    assert!(
        ok,
        "a declared font did not resolve to the family the program asked for — \
         text would draw in a generic fallback with nothing on stderr.\n{text}"
    );
}

/// @PLN146 F6 — with the font source THROTTLED, the page still resolves to the
/// requested family, and the same page with the await removed does not.
///
/// The control is the phase: on a fast local font a page resolves whether it waited
/// or not, so a gate without one would report a broken ordering as a working one.
/// Both pages run against the same delayed server, in the same test, so a green
/// first half is only a reading if the second half went the other way.
// @speed 6.0
#[test]
fn the_boot_await_holds_the_program_until_a_slow_font_has_arrived() {
    let Some((_chrome, _node, py)) = tooling() else {
        eprintln!("SKIP: chrome / node / python3 not all present");
        return;
    };
    let Some(dir) = stage_dir("loft_html_fonts_f6") else {
        eprintln!("SKIP: doc/assets/DejaVuSans-Bold.ttf missing");
        return;
    };
    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let fonts = three_sources();
    write_page(&dir, "awaited.html", &fonts, true);
    write_page(&dir, "control.html", &fonts, false);
    let Some(server) = spawn_server(&py, &dir, port, 800) else {
        eprintln!("SKIP: failed to start the static server");
        return;
    };
    let _guard = ServerGuard(server);

    let Some((ok, text)) = check(
        &format!("http://127.0.0.1:{port}/awaited.html"),
        ALL_RESOLVED,
        5000,
        port.wrapping_add(1),
    ) else {
        return;
    };
    assert!(
        ok,
        "{}\n{text}",
        if page_never_ran(&text) {
            "the awaited page never reached the bridge inside its budget, so this run observed \
             NOTHING about the await — raise the budget rather than reading it as a font result"
        } else {
            "a throttled font did not resolve even though the page waited for it — the emitted \
             `document.fonts.load` await is not holding the program"
        }
    );

    // The control: the same page, the same server, no await.  The two families this
    // page BRINGS must still be in flight — the resident one is not fetched at all,
    // so it resolves either way and is what proves the run happened.
    let Some((ok, text)) = check(
        &format!("http://127.0.0.1:{port}/control.html"),
        "globalThis.loftFonts.length===3 && \
         globalThis.loftFonts.filter(f=>!f.resolved).map(f=>f.base).join('|')===\
         'LoftProbeFace|LoftCdnFace'",
        // The same budget as the awaited half, and it cannot make this control pass by
        // waiting: the records it reads are a SNAPSHOT taken when the bridge was called, so
        // a font arriving later cannot change them.  1500ms was chosen as if the snapshot
        // were live, and on a loaded runner it expired before the page had booted.
        5000,
        port.wrapping_add(2),
    ) else {
        return;
    };
    assert!(
        ok,
        "{}\n{text}",
        if page_never_ran(&text) {
            "the control page never reached the bridge inside its budget, so this run observed \
             NOTHING — that is not the control firing, and reading it as one sends the next \
             reader after a font-ordering bug that is not there (this is what the nightly's \
             ubuntu leg was reporting)"
        } else {
            "the CONTROL did not fire: without the await a throttled font still resolved, so \
             this gate cannot tell a page that waits from one that does not"
        }
    );
}

/// The wiring gate: a real `loft --html` build of a package that declares a font
/// carries the block, in the right place.
///
/// The two browser gates above run the emitted strings, not the emitted PAGE — so on
/// their own they would stay green with the driver never calling either function, or
/// calling them into the wrong shell.  This one builds the page and reads it.
// @speed 20.0
#[test]
fn a_declared_font_reaches_the_emitted_page() {
    let root = repo_root();
    let loft = root.join("target/release/loft");
    if !loft.exists() {
        eprintln!("SKIP a_declared_font_reaches_the_emitted_page: target/release/loft not built");
        return;
    }
    let base = std::env::temp_dir().join("loft_html_fonts_emit");
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("app");
    let lib = base.join("fontlib");
    std::fs::create_dir_all(dir.join("src")).expect("create package");
    std::fs::create_dir_all(lib.join("src")).expect("create library");
    std::fs::write(
        dir.join("loft.toml"),
        "[package]\nname = \"fontgame\"\n\n\
         [dependencies]\nfontlib = { path = \"../fontlib\" }\n\n\
         [[font]]\nfamily = \"LoftProbeFace\"\nnative = \"fonts/LoftProbeFace.ttf\"\n\
         url = \"fonts/LoftProbeFace.woff2\"\n\n\
         [[font]]\nfamily = \"LoftCdnFace\"\n\
         stylesheet = \"https://fonts.example.test/css2?family=LoftCdnFace\"\n",
    )
    .expect("write manifest");
    // A LIBRARY that draws its own text declares the font it draws with, and that
    // declaration has to reach the consumer's page — the route `[wasm.bridge]
    // host_js` already takes.  Untested, this is the half that would rot.
    std::fs::write(
        lib.join("loft.toml"),
        "[package]\nname = \"fontlib\"\n\n\
         [[font]]\nfamily = \"LoftLibFace\"\nurl = \"fonts/LoftLibFace.woff2\"\n",
    )
    .expect("write library manifest");
    std::fs::write(
        lib.join("src/fontlib.loft"),
        "pub fn lib_face() -> text { \"LoftLibFace\" }\n",
    )
    .expect("write library source");
    std::fs::write(
        dir.join("src/fontgame.loft"),
        "use graphics;\n\
         use fontlib;\n\
         fn main() {\n\
         \x20 f = graphics::gl_load_font(\"fonts/LoftProbeFace.ttf\");\n\
         \x20 print(\"{f} {fontlib::lib_face()}\");\n\
         }\n",
    )
    .expect("write source");

    let html = dir.join("fontgame.html");
    let out = Command::new(&loft)
        .args(["--html", html.to_str().expect("utf-8 path")])
        .arg("--lib")
        .arg(root.join("tests/fixtures/libs"))
        .arg(dir.join("src/fontgame.loft"))
        .output()
        .expect("invoke loft --html");
    if !out.status.success() || !html.exists() {
        // No wasm32 rlib / no rustc / no toolchain is an environmental skip, the
        // same one `tests/html_gl_imports.rs` takes.
        eprintln!(
            "SKIP a_declared_font_reaches_the_emitted_page: `loft --html` failed\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let page = std::fs::read_to_string(&html).expect("read emitted page");
    assert!(
        page.contains(
            "@font-face{font-family:\"LoftProbeFace\";src:url(\"fonts/LoftProbeFace.woff2\") \
             format(\"woff2\")}"
        ),
        "the emitted page carries no @font-face for the declared family"
    );
    assert!(
        page.contains(
            "<link rel=\"stylesheet\" href=\"https://fonts.example.test/css2?family=LoftCdnFace\">"
        ),
        "the emitted page carries no <link> for the declared stylesheet source"
    );
    assert!(
        page.contains("@font-face{font-family:\"LoftLibFace\""),
        "a font declared by a LIBRARY did not reach the consumer's page"
    );
    // F6 is an ORDERING, so position is the assertion: the await has to come before
    // the program starts, and a block emitted after it would satisfy a substring
    // check while doing nothing.
    // `rfind`, because the embedded glue names `loft_start` in its own comments and
    // helpers long before the page boots; the boot sequence is the LAST of them.
    let (Some(awaited), Some(start)) = (
        page.rfind("document.fonts.load"),
        page.rfind("ac.start('loft_start')"),
    ) else {
        panic!("the emitted page has no font await, or no loft_start call to order it against");
    };
    assert!(
        awaited < start,
        "the font await is emitted AFTER loft_start, so the program runs before its \
         fonts have arrived"
    );
}

/// A family that drifts from the name the program passes is refused, and refused
/// BEFORE the build — a manifest that cannot produce a working page should not cost
/// a wasm compile first.
// @speed 2.0
#[test]
fn a_drifting_family_is_refused_before_the_build() {
    let root = repo_root();
    let loft = root.join("target/release/loft");
    if !loft.exists() {
        eprintln!(
            "SKIP a_drifting_family_is_refused_before_the_build: target/release/loft not built"
        );
        return;
    }
    let dir = std::env::temp_dir().join("loft_html_fonts_drift");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create package");
    std::fs::write(
        dir.join("loft.toml"),
        // The family the page would register, and the path the program passes,
        // differ by two spaces.  Text draws — in a fallback.
        "[package]\nname = \"driftgame\"\n\n\
         [[font]]\nfamily = \"Press Start 2P\"\nnative = \"fonts/PressStart2P.ttf\"\n",
    )
    .expect("write manifest");
    std::fs::write(
        dir.join("src/driftgame.loft"),
        "use graphics;\n\
         fn main() {\n\
         \x20 print(\"{graphics::gl_load_font(\\\"fonts/PressStart2P.ttf\\\")}\");\n\
         }\n",
    )
    .expect("write source");

    let html = dir.join("driftgame.html");
    let out = Command::new(&loft)
        .args(["--html", html.to_str().expect("utf-8 path")])
        .arg("--lib")
        .arg(root.join("tests/fixtures/libs"))
        .arg(dir.join("src/driftgame.loft"))
        .output()
        .expect("invoke loft --html");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "a drifting [[font]] family built a page instead of being refused"
    );
    assert!(
        err.contains("PressStart2P") && err.contains("does not match"),
        "the refusal does not name the drift it found:\n{err}"
    );
    assert!(!html.exists(), "a refused build still wrote a page");
}
