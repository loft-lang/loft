// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// D-html-vec regression (2026-07-04): the `--html`/wasm codegen must EMIT the
// `#native` host-import calls that take a `vector` argument — the GL buffer
// uploads (`gl_upload_vertices`), texture uploads (`gl_upload_canvas`), and
// matrix uniforms (`gl_set_uniform_mat4`). Commit 12567706 (@PLN85 #423)
// broadened a heap-arg guard from `Type::Reference` to `heap_dep().is_some()`,
// which swallowed `Type::Vector` too: those upload calls were routed into the
// @P321c Phase-1 "graceful stub" and their extern declarations skipped, so NO
// buffer data ever reached WebGL and every `--html` WebGL program (Brick Buster,
// the website hero) rendered a blank canvas — "broken long before PLN25".
//
// This guard runs the produced WASM "natively" (via the JS WebAssembly engine,
// no browser / no SwiftShader) and asserts the vector-argument host import is
// present in the module's IMPORT TABLE — it is there only if the codegen both
// DECLARED it and EMITTED a call to it (an unused import is pruned). Before the
// fix it is absent; after, present.
//
// Skips cleanly when a prerequisite is missing (node, the wasm32 toolchain, a
// built `loft` binary, or the `graphics` library — which resolves from the
// registry and so needs network). The skip is printed so a green run never
// hides reduced coverage.

use loft::file_access as fa;
use std::path::PathBuf;

fn which(cmd: &str) -> bool {
    loft::platform::process::harness_command("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wasm32_installed() -> bool {
    loft::platform::process::harness_command("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-unknown-unknown"))
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// @speed 1.0
#[test]
fn dhtml_vector_arg_gl_host_import_is_emitted() {
    if !wasm32_installed() {
        eprintln!(
            "SKIP dhtml_vector_arg_gl_host_import_is_emitted: wasm32-unknown-unknown target not installed"
        );
        return;
    }
    vector_arg_import_is_emitted("--native-rust", "rust");
}

/// The same page from the C backend (@PLN203 C6): the import comes from the one contract both
/// writers read (`host_imports`), and the module runs from its exported `loft_start` on the
/// page's WASI layer (`doc/loft-wasi.js`).
// @speed 1.0
#[test]
fn the_c_page_imports_the_vector_argument_host_import() {
    if !which("clang") || !which("wasm-ld") {
        eprintln!(
            "SKIP the_c_page_imports_the_vector_argument_host_import: clang / wasm-ld not installed"
        );
        return;
    }
    vector_arg_import_is_emitted("--native-c", "c");
}

fn vector_arg_import_is_emitted(backend: &str, tag: &str) {
    if !which("node") {
        eprintln!("SKIP {tag}: node not installed");
        return;
    }
    let root = repo_root();
    let loft = std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"));
    if !fa::exists(&loft) {
        eprintln!("SKIP {tag}: target/release/loft not built");
        return;
    }
    let helper = root.join("tools/wasm_imports.mjs");
    assert!(fa::exists(&helper), "tools/wasm_imports.mjs missing");

    // Minimal program that calls the vector-argument host import
    // `graphics::gl_upload_vertices(vector<single>, integer)`.
    let dir = std::env::temp_dir().join(format!("loft_html_gl_imports_{tag}"));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create tempdir");
    let src = dir.join("gl_import_probe.loft");
    let html = dir.join("gl_import_probe.html");
    fa::write(
        &src,
        "use graphics;\n\
         fn main() {\n\
         \x20 v: vector<single> = [];\n\
         \x20 v += [0.0 as single];\n\
         \x20 v += [0.0 as single];\n\
         \x20 v += [0.0 as single];\n\
         \x20 vao = graphics::gl_upload_vertices(v, 3);\n\
         \x20 print(\"{vao}\");\n\
         }\n",
    )
    .expect("write source");

    let out = loft::platform::process::harness_command(&loft)
        .args([backend, "--html", html.to_str().unwrap()])
        .arg("--path")
        .arg(format!("{}/", root.display()))
        .arg(&src)
        .output()
        .expect("invoke loft --html");
    if !out.status.success() {
        // The most common reason is that `graphics` (a registry library) could
        // not be resolved — no network on this runner. That is an environmental
        // skip, not a codegen regression.
        eprintln!(
            "SKIP {tag}: `loft --html` failed (graphics unresolved / offline?)\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }

    let check = loft::platform::process::harness_command("node")
        .arg(&helper)
        .arg(&html)
        .arg("loft_gl_upload_vertices")
        .output()
        .expect("invoke node wasm_imports helper");
    assert!(
        check.status.success(),
        "{backend}: the vector-argument host import `loft_gl_upload_vertices` is MISSING from \
         the --html wasm import table — the codegen dropped/stubbed the buffer-upload call \
         (D-html-vec; the #423 regression that rendered every --html WebGL page blank).\n\
         helper stdout: {}\nhelper stderr: {}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
    );

    // The module runs: the harness answers every `loft_gl` import with 0, so the page prints
    // the vertex-array handle it got, `0`.
    let page = fa::read_to_string(&html).expect("read the page");
    let marker = "const wasmB64=\"";
    let at = page.find(marker).expect("the page carries its module") + marker.len();
    let b64 = &page[at..at + page[at..].find('"').expect("the module's closing quote")];
    let wasm = dir.join("gl_import_probe.wasm");
    fa::write(&wasm, loft::base64::decode(b64)).expect("write wasm");
    let ran = loft::platform::process::harness_command("node")
        .arg(root.join("tools/wasm_repro.mjs"))
        .arg(&wasm)
        .output()
        .expect("invoke node wasm_repro");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "0",
        "{backend}: the page's module prints the stubbed handle\nstderr: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
}
