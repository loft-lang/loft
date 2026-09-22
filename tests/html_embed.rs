// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN146 F4 — a `--html` page carries the pack it reads.
//!
//! The loader half shipped first (`tests/html_page_store.rs`): a page CAN read a store
//! out of `globalThis.loftBaseFS`. But nothing put anything there, so a page could only
//! carry its pack if someone spliced the bytes in by hand. `[[embed]]` in `loft.toml` is
//! the other half, and this is its gate.
//!
//! One invariant, and both tests here are about it:
//!
//! > A file declared `[[embed]] path = "P"` is readable inside the page by exactly the
//! > call that reads it on the desktop — same function, same string `"P"`.
//!
//! So the browser test asserts the page prints what the DESKTOP run printed, from the
//! same source with only the target changed, reading a **nested** path (`assets/…`) —
//! the shipped loader gate only ever proved a flat one, which pins the directory axis.
//! Its control strips the emitted seed from the same page and must go back to
//! `load=false`: one variable, and a page that could not fail proves nothing.
//!
//! The second test is the refusals, and it is where the silent failure lives. A page
//! carrying a file under a key nothing asks for answers `store_load` → `false`, draws no
//! art, and says nothing — F5's failure in a new place. Each refusal must land BEFORE
//! the wasm compile, which is asserted structurally: no page was written.
//!
//! Skips cleanly without chrome / node / python3, in the shape of `tests/html_fonts.rs`.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
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
    let child = Command::new(py)
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
fn page_output(url: &str, port: u16) -> Option<String> {
    let out = Command::new("node")
        .arg(repo_root().join("tools/html_render_check.mjs"))
        .arg(url)
        .args(["--wait-ms", "4000"])
        .args(["--port", &port.to_string()])
        .args(["--assert", "document.getElementById('out').textContent"])
        .output()
        .expect("invoke node harness");
    // The assertion fails by design (the value is text, not `true`), and the harness
    // prints it back — which is the only way to read a page's own output from here.
    let text = String::from_utf8_lossy(&out.stderr).to_string();
    // The harness's own contract: no browser to drive — none installed, or one that never
    // answered its debugging port — is a `SKIP: …` line and exit 2, which the CALLER turns
    // into a skip.  Read as page output it failed the assertion below on a runner whose
    // Chrome was slow to start, naming a pack that was never read.
    if out.status.code() == Some(2) || text.trim_start().starts_with("SKIP:") {
        eprintln!("{}", text.trim());
        return None;
    }
    Some(text)
}

/// The program both halves use: it WRITES the pack natively and READS it back, and the
/// page runs only the reading half. So the bytes the page reads were produced by the
/// desktop, which is the direction a pack actually travels. Two keys with distinctive
/// values, because a store that loaded EMPTY would pass a `!= null` check.
const PROGRAM: &str = "struct Rec { r_key: text, r_n: integer }\n\
     \n\
     fn main() {\n\
     \x20 if (env_variable(\"MAKE_STORE\") ?? \"\") != \"\" {\n\
     \x20   h: hash<Rec[r_key]> = [];\n\
     \x20   h[\"a\"] = Rec { r_key: \"a\", r_n: 7 };\n\
     \x20   h[\"b\"] = Rec { r_key: \"b\", r_n: 41 };\n\
     \x20   println(\"persist={store_persist_copy(h, \\\"assets/page.store\\\")}\");\n\
     \x20   return\n\
     \x20 }\n\
     \x20 q: hash<Rec[r_key]> = [];\n\
     \x20 ok = store_load(q, \"assets/page.store\");\n\
     \x20 a = q[\"a\"];\n\
     \x20 b = q[\"b\"];\n\
     \x20 println(\"load={ok} a={if a != null { a.r_n } else { -1 }} \
     b={if b != null { b.r_n } else { -1 }}\")\n\
     }\n";

/// Lay out a one-file package with `manifest` as its `loft.toml`, and write the pack
/// the program reads. Answers the package dir, or `None` when this box cannot build.
fn package(loft: &Path, name: &str, manifest: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("loft_html_embed_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("create package dir");
    std::fs::write(dir.join("loft.toml"), manifest).expect("write manifest");
    std::fs::write(dir.join("embed_page.loft"), PROGRAM).expect("write source");
    let made = Command::new(loft)
        .arg("--interpret")
        .arg("embed_page.loft")
        .current_dir(&dir)
        .env("MAKE_STORE", "1")
        .output()
        .expect("invoke loft");
    // The desktop half of the parity claim: this is the output the page must match.
    let desktop = Command::new(loft)
        .arg("--interpret")
        .arg("embed_page.loft")
        .current_dir(&dir)
        .output()
        .expect("invoke loft");
    let desktop = String::from_utf8_lossy(&desktop.stdout).to_string();
    assert!(
        dir.join("assets/page.store").exists() && desktop.contains("load=true a=7 b=41"),
        "the fixture pack was not written or does not read back on the desktop, so \
         nothing below would mean anything.\nwrite: {}\nread: {desktop}",
        String::from_utf8_lossy(&made.stdout)
    );
    Some(dir)
}

/// @PLN146 F4 — a page declaring `[[embed]]` reads its pack with the same call, by the
/// same name, as the desktop run that wrote it.
// @speed 60.0
#[test]
fn a_declared_pack_is_readable_in_the_page_by_the_programs_own_path() {
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
    if !loft.exists() {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }

    // A NESTED path on purpose: `tests/html_page_store.rs` proved a flat `page.store`,
    // which holds the directory axis fixed and would pass whether or not the page key
    // is built the way `loft-fs.js` resolves one.
    let Some(dir) = package(
        &loft,
        "carried",
        "[package]\nname = \"embedpage\"\n\n[[embed]]\npath = \"assets/page.store\"\n",
    ) else {
        return;
    };

    let built = Command::new(&loft)
        .args(["--html", "page.html"])
        .arg("embed_page.loft")
        .current_dir(&dir)
        .output()
        .expect("invoke loft --html");
    if !built.status.success() || !dir.join("page.html").exists() {
        eprintln!(
            "SKIP: `loft --html` failed (no wasm toolchain?)\nstderr: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        return;
    }

    // The control, and it is the same page with ONE thing removed: the seed statement
    // that hands the pack to the page's filesystem. Everything else — the wasm, the
    // program, the path it reads — is byte-identical.
    let page = std::fs::read_to_string(dir.join("page.html")).expect("read page");
    let head = "globalThis.loftBaseFS=Object.assign(";
    let start = page
        .find(head)
        .expect("the emitted page carries no base-FS seed at all");
    let end = page[start..]
        .find("});\n")
        .map(|e| start + e + 4)
        .expect("the seed statement is unterminated");
    let control = format!("{}{}", &page[..start], &page[end..]);
    assert!(
        control.len() + 1000 < page.len(),
        "the stripped seed carried no bytes, so the control moves nothing"
    );
    std::fs::write(dir.join("control.html"), &control).expect("write control page");

    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let Some(server) = spawn_server(&py, &dir, port) else {
        eprintln!("SKIP: failed to start python http.server");
        return;
    };
    let _guard = ServerGuard(server);

    let Some(carried) = page_output(
        &format!("http://127.0.0.1:{port}/page.html"),
        port.wrapping_add(1),
    ) else {
        return;
    };
    let Some(bare) = page_output(
        &format!("http://127.0.0.1:{port}/control.html"),
        port.wrapping_add(2),
    ) else {
        return;
    };

    // Value AND cardinality: a store that loaded empty would answer `load=true` with
    // both records absent, which is the failure a `!= null` check waves through.
    assert!(
        carried.contains("load=true a=7 b=41"),
        "a page declaring `[[embed]] path = \"assets/page.store\"` did not read its pack \
         by the path the program passes — the desktop run of this same source prints \
         `load=true a=7 b=41`.\n{carried}"
    );
    assert!(
        bare.contains("load=false a=-1 b=-1"),
        "the same page with the seed removed still read the pack, so the assertion \
         above says nothing about what put it there.\n{bare}"
    );
}

/// @PLN146 F4 — the declared file is the one the PROGRAM reads, not the one beside the
/// manifest.
///
/// loft resolves a path a program passes relative to the PROGRAM file, so with the
/// standard `src/<name>.loft` layout `assets/game.pack` means `src/assets/game.pack`.
/// Rooting the source at the manifest instead put a DIFFERENT file in the page under the
/// key the program asks for — measured, before the fix, as a desktop run answering
/// `load=false` while its own page answered `load=true`. The decoy at the manifest root
/// is what makes this a reading rather than a check that a file was found.
// @speed 40.0
#[test]
fn the_embedded_file_is_the_one_the_program_would_open() {
    let loft = repo_root().join("target/release/loft");
    if !loft.exists() {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }
    let root = std::env::temp_dir().join("loft_html_embed_srclayout");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src/assets")).expect("create src tree");
    std::fs::create_dir_all(root.join("assets")).expect("create decoy tree");
    std::fs::write(
        root.join("loft.toml"),
        "[package]\nname = \"layout\"\n\n[[embed]]\npath = \"assets/game.pack\"\n",
    )
    .expect("write manifest");
    // What the program reads, and what the page must carry.
    std::fs::write(root.join("src/assets/game.pack"), b"BESIDE-THE-PROGRAM")
        .expect("write program-side file");
    // The decoy: same relative name, beside the MANIFEST, which the program's own
    // `store_load` would never open.
    std::fs::write(root.join("assets/game.pack"), b"BESIDE-THE-MANIFEST").expect("write decoy");
    std::fs::write(
        root.join("src/layout.loft"),
        "fn main() { println(\"page\") }\n",
    )
    .expect("write source");

    let built = Command::new(&loft)
        .args(["--html", "page.html", "src/layout.loft"])
        .current_dir(&root)
        .output()
        .expect("invoke loft --html");
    if !built.status.success() || !root.join("page.html").exists() {
        eprintln!(
            "SKIP: `loft --html` failed (no wasm toolchain?)\nstderr: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        return;
    }
    let page = std::fs::read_to_string(root.join("page.html")).expect("read page");
    assert!(
        page.contains(&loft::base64::encode(b"BESIDE-THE-PROGRAM")),
        "the page does not carry the file the program's own `store_load` would open"
    );
    assert!(
        !page.contains(&loft::base64::encode(b"BESIDE-THE-MANIFEST")),
        "`source` resolved against the MANIFEST — the page carries a file the desktop \
         run of this program never reads, under the key it asks for"
    );
}

/// @PLN146 F4 — a LIBRARY's `[[embed]]` reaches a consumer's page, and brings the
/// library's own file.
///
/// The consumer here holds a file of the same name with different bytes. Without that
/// decoy the test would pass whichever directory `source` resolved against, which is the
/// only thing it is trying to measure.
// @speed 40.0
#[test]
fn a_librarys_declaration_brings_the_librarys_own_file() {
    let loft = repo_root().join("target/release/loft");
    if !loft.exists() {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }
    let root = std::env::temp_dir().join("loft_html_embed_lib");
    let _ = std::fs::remove_dir_all(&root);
    let lib = root.join("libs/embedlib");
    std::fs::create_dir_all(lib.join("src")).expect("create lib dir");
    std::fs::create_dir_all(lib.join("assets")).expect("create lib assets");
    std::fs::write(
        lib.join("loft.toml"),
        "[package]\nname = \"embedlib\"\nversion = \"0.1.0\"\n\n\
         [[embed]]\npath = \"assets/lib.pack\"\n",
    )
    .expect("write lib manifest");
    std::fs::write(lib.join("assets/lib.pack"), b"LIBRARY-OWN-BYTES").expect("write lib pack");
    std::fs::write(
        lib.join("src/embedlib.loft"),
        "pub fn lib_greeting() -> text { \"from the library\" }\n",
    )
    .expect("write lib source");

    let app = root.join("app");
    std::fs::create_dir_all(app.join("assets")).expect("create app dir");
    std::fs::write(app.join("loft.toml"), "[package]\nname = \"app\"\n").expect("write app toml");
    // The decoy: same name, different bytes, in the directory the consumer builds from.
    std::fs::write(app.join("assets/lib.pack"), b"CONSUMER-DECOY-BYTES").expect("write decoy");
    std::fs::write(
        app.join("main.loft"),
        "use embedlib;\nfn main() { println(\"{embedlib::lib_greeting()}\") }\n",
    )
    .expect("write app source");

    let built = Command::new(&loft)
        .args(["--html", "app.html", "--lib", "../libs", "main.loft"])
        .current_dir(&app)
        .output()
        .expect("invoke loft --html");
    if !built.status.success() || !app.join("app.html").exists() {
        eprintln!(
            "SKIP: `loft --html` failed (no wasm toolchain?)\nstderr: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        return;
    }
    let page = std::fs::read_to_string(app.join("app.html")).expect("read page");
    assert!(
        page.contains("\"/assets/lib.pack\":"),
        "the library's [[embed]] never reached the consumer's page"
    );
    let want = loft::base64::encode(b"LIBRARY-OWN-BYTES");
    let decoy = loft::base64::encode(b"CONSUMER-DECOY-BYTES");
    assert!(
        page.contains(&want),
        "the consumer's page does not carry the library's file"
    );
    assert!(
        !page.contains(&decoy),
        "`source` resolved against the CONSUMER's directory — the page carries the app's \
         same-named file instead of the library's, which is a file nobody declared"
    );
}

/// @PLN146 F4 — a declaration that cannot produce a working page stops the build, and
/// stops it before the wasm compile.
///
/// Every cell here is a spelling that would otherwise be carried FAITHFULLY, under a key
/// the program never asks for: the page loads, the pack does not, and nothing says so.
// @speed 60.0
#[test]
fn a_declaration_that_cannot_work_stops_the_build_before_the_wasm() {
    let loft = repo_root().join("target/release/loft");
    if !loft.exists() {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }
    let Some(dir) = package(
        &loft,
        "refuse",
        "[package]\nname = \"embedpage\"\n\n[[embed]]\npath = \"assets/page.store\"\n",
    ) else {
        return;
    };
    let abs = dir.join("assets/page.store");
    let abs = abs.to_string_lossy();

    let build = |manifest: &str| -> (bool, bool, String) {
        std::fs::write(dir.join("loft.toml"), manifest).expect("write manifest");
        let _ = std::fs::remove_file(dir.join("out.html"));
        let out = Command::new(&loft)
            .args(["--html", "out.html"])
            .arg("embed_page.loft")
            .current_dir(&dir)
            .output()
            .expect("invoke loft --html");
        (
            out.status.success(),
            dir.join("out.html").exists(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    let head = "[package]\nname = \"embedpage\"\n\n";

    for (what, manifest, needle) in [
        (
            "a source that is not there",
            format!("{head}[[embed]]\npath = \"assets/nope.pack\"\n"),
            "has no file at",
        ),
        (
            // Carried under `/home/…`, which only a program naming this box's paths
            // could ever ask for.
            "a build-box absolute path",
            format!("{head}[[embed]]\npath = \"{abs}\"\n"),
            "is not a relative path",
        ),
        (
            // `./assets/x` resolves to `/assets/x`, so the page would hold a key the
            // declaration does not spell.
            "a path that is not in normal form",
            format!("{head}[[embed]]\npath = \"./assets/page.store\"\n"),
            "is not in normal form",
        ),
        (
            "one name from two files",
            format!(
                "{head}[[embed]]\npath = \"assets/page.store\"\n\n\
                 [[embed]]\npath = \"assets/page.store\"\nsource = \"assets/page.store.dschema\"\n"
            ),
            "declared twice from different sources",
        ),
    ] {
        let (ok, page, err) = build(&manifest);
        assert!(!ok, "{what}: the build succeeded\n{err}");
        assert!(
            err.contains(needle),
            "{what}: the message does not name the problem (wanted `{needle}`)\n{err}"
        );
        // Structural, and the reason the phase is cheap to get wrong: a manifest that
        // cannot produce a working page must not cost a wasm compile before it says so.
        assert!(
            !page,
            "{what}: a page was written before the refusal\n{err}"
        );
    }

    // The control. Without it every "no page was written" above is vacuous — a build
    // that never produces a page on this box would pass all four for the wrong reason.
    let (ok, page, err) = build(&format!("{head}[[embed]]\npath = \"assets/page.store\"\n"));
    if !ok && err.contains("wasm") {
        eprintln!("SKIP: `loft --html` cannot build here (no wasm toolchain?)\n{err}");
        return;
    }
    assert!(
        ok && page,
        "the same package with a VALID declaration did not build, so the four refusals \
         above prove nothing about the declaration.\n{err}"
    );
}
