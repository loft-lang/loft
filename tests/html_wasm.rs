// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// P137 regression: `loft --html` must produce WASM that runs cleanly
// under a minimal Node host.  The previous failure was a bare
// `(unreachable)` trap on every `--html` program at Stores::new(),
// caused by `std::time::Instant::now()` panicking on
// wasm32-unknown-unknown without a time source.
//
// This test drives the integration end-to-end:
//   1. Writes a trivial `.loft` program.
//   2. Invokes the `loft` binary with `--html` to produce an HTML bundle.
//   3. Extracts the base64-embedded WASM.
//   4. Runs `tools/wasm_repro.mjs` to instantiate it with stub host
//      imports and invoke `loft_start`.
//   5. Asserts the process exits cleanly.
//
// Skipped when the prerequisites (wasm32-unknown-unknown toolchain,
// node binary) are unavailable — typical CI has them; developer
// machines without WASM rust targets get a clear "skipped" message
// rather than a false failure.

use loft::file_access as fa;
use std::path::PathBuf;

// `loft --html` now writes every build intermediate to a per-PROCESS scratch dir
// (`platform::build_scratch_dir` — `scratch/loft_html_<pid>/`), so concurrent
// invocations no longer race on a shared `scratch/loft_html.{rs,wasm}`.  The old
// process-wide `build_lock()` mutex that serialised these tests is therefore
// gone; parallel_html_builds_do_not_cross_contaminate is the regression guard.

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

fn wasm32_target_installed() -> bool {
    rustup_target_installed("wasm32-unknown-unknown")
}

/// Is `triple` an installed rustup target on this machine?  A test that needs one
/// skips rather than fails without it — a machine that cannot cross-compile says
/// nothing about the code under test.
fn rustup_target_installed(triple: &str) -> bool {
    loft::platform::process::harness_command("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.lines().any(|l| l == triple))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Build a loft program via `--html`, extract the WASM, run it via the
/// Node repro harness with stub imports, and return (stdout, stderr,
/// exit_status).  Returns None when prerequisites are missing.
fn run_html_wasm(name: &str, source: &str) -> Option<(String, String, bool)> {
    run_html_wasm_with_libs_and_assets(name, source, &[], &[])
}

/// Same as `run_html_wasm` but also passes a `--lib <dir>` for each
/// entry in `lib_dirs`.  Needed for programs that `use <pkg>;` out of
/// a local library tree (e.g. `use moros_editor;` from `lib/`).
fn run_html_wasm_with_libs(
    name: &str,
    source: &str,
    lib_dirs: &[&str],
) -> Option<(String, String, bool)> {
    run_html_wasm_with_libs_and_assets(name, source, lib_dirs, &[])
}

/// @P321(c) Phase 3a — extension of `run_html_wasm_with_libs` that also
/// copies `assets` (sibling resource files like `*.png`) into the same
/// `/tmp/` dir as the synthesised `.loft`, so the `--html` driver's
/// asset auto-discovery finds them.  Use from the lib suite, which
/// synthesises entry .loft's outside their original test dir.
fn run_html_wasm_with_libs_and_assets(
    name: &str,
    source: &str,
    lib_dirs: &[&str],
    assets: &[PathBuf],
) -> Option<(String, String, bool)> {
    run_html_wasm_full(name, source, lib_dirs, assets, &[], "tools/wasm_repro.mjs")
}

/// As above, plus arbitrary extra `loft --html` flags (e.g. `--debug=<name>`) and a
/// choice of Node harness (`wasm_repro.mjs` runs `loft_start`; a debug harness may
/// call another export).
fn run_html_wasm_full(
    name: &str,
    source: &str,
    lib_dirs: &[&str],
    assets: &[PathBuf],
    extra_args: &[&str],
    harness_rel: &str,
) -> Option<(String, String, bool)> {
    if which("node").is_none() {
        eprintln!("SKIP: node not installed");
        return None;
    }
    if !wasm32_target_installed() {
        eprintln!("SKIP: rustup target wasm32-unknown-unknown not installed");
        return None;
    }

    let loft_bin = repo_root().join("target/release/loft");
    if !fa::exists(&loft_bin) {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release` first)");
        return None;
    }

    // @PLN100 Slice 1 — no manual rlib freshness guard: `loft --html` auto-builds
    // its own isolated wasm runtime rlib (`target/loft/html/…`) on stale/missing,
    // so a stale or wasm-bindgen-stomped rlib can no longer reach this path.

    // @P321(c) Phase 3a: per-test subdir so `--html`'s asset
    // auto-discovery (which scans the dir of the entry .loft) only
    // finds assets this test asked for — a shared /tmp/ would let
    // an earlier test's PNGs leak into a later test's bundle.
    let tmp = std::env::temp_dir().join(format!("loft_html_{name}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).expect("create per-test dir");
    let src = tmp.join(format!("{name}.loft"));
    let html = tmp.join(format!("{name}.html"));
    let wasm = tmp.join(format!("{name}.wasm"));

    fa::write(&src, source).expect("write source");

    for asset in assets {
        let Some(fname) = fa::file_name(asset) else {
            continue;
        };
        let dest = tmp.join(fname);
        if let Err(e) = fa::copy(asset, &dest) {
            eprintln!("warn: could not copy asset {asset:?} → {dest:?}: {e}");
        }
    }

    // No build serialisation: `loft --html` isolates its intermediates per
    // process (per-PID scratch dir), so parallel invocations can't clobber
    // each other's emitted Rust or wasm output.
    let mut cmd = loft::platform::process::harness_command(&loft_bin);
    cmd.args([
        "--html",
        html.to_str().unwrap(),
        "--path",
        &format!("{}/", repo_root().display()),
    ]);
    for dir in lib_dirs {
        cmd.arg("--lib").arg(repo_root().join(dir));
    }
    cmd.args(extra_args);
    cmd.arg(src.to_str().unwrap());
    let status = cmd.status().expect("invoke loft --html");
    assert!(status.success(), "loft --html failed for {name}");

    let html_content = fa::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = html_content.find(marker).expect("wasmB64 marker present") + marker.len();
    let end = start
        + html_content[start..]
            .find('"')
            .expect("wasmB64 closing quote");
    let b64 = &html_content[start..end];
    let bytes = base64_decode_standard(b64).expect("decode wasmB64");
    fa::write(&wasm, &bytes).expect("write extracted wasm");

    let harness = repo_root().join(harness_rel);
    assert!(fa::exists(&harness), "{harness_rel} missing");

    let out = loft::platform::process::harness_command("node")
        .arg(&harness)
        .arg(&wasm)
        .output()
        .expect("invoke node harness");

    Some((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    ))
}

/// Where `run_html_wasm_full` leaves the wasm it pulled out of the page.
fn extracted_wasm(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("loft_html_{name}"))
        .join(format!("{name}.wasm"))
}

/// Every name in the wasm `name` section's function subsection (loft#954).
///
/// Deliberately a second, independent walk rather than a call into loft's own
/// parser: this test's whole job is to check what the emitted artefact carries,
/// and a shared decoder would let one bug hide on both sides of the assertion.
fn wasm_function_names(wasm: &[u8]) -> Vec<String> {
    fn uleb(b: &[u8], p: &mut usize) -> Option<u64> {
        let (mut v, mut shift) = (0u64, 0u32);
        loop {
            let byte = *b.get(*p)?;
            *p += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }
    fn walk(wasm: &[u8], out: &mut Vec<String>) -> Option<()> {
        if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
            return None;
        }
        let mut p = 8;
        while p < wasm.len() {
            let id = *wasm.get(p)?;
            p += 1;
            let size = usize::try_from(uleb(wasm, &mut p)?).ok()?;
            let (start, end) = (p, p.checked_add(size).filter(|e| *e <= wasm.len())?);
            if id == 0 {
                let mut q = start;
                let len = usize::try_from(uleb(wasm, &mut q)?).ok()?;
                let sect = wasm.get(q..q.checked_add(len)?)?;
                q += len;
                if sect == b"name" {
                    while q < end {
                        let sub = *wasm.get(q)?;
                        q += 1;
                        let sub_size = usize::try_from(uleb(wasm, &mut q)?).ok()?;
                        let sub_end = q.checked_add(sub_size).filter(|e| *e <= end)?;
                        if sub == 1 {
                            let count = uleb(wasm, &mut q)?;
                            for _ in 0..count {
                                uleb(wasm, &mut q)?; // function index
                                let n = usize::try_from(uleb(wasm, &mut q)?).ok()?;
                                let bytes = wasm.get(q..q.checked_add(n)?)?;
                                q += n;
                                out.push(String::from_utf8_lossy(bytes).into_owned());
                            }
                            return Some(());
                        }
                        q = sub_end;
                    }
                }
            }
            p = end;
        }
        Some(())
    }
    let mut out = Vec::new();
    let _ = walk(wasm, &mut out);
    out
}

// loft#954 — `--html --names` must make a browser backtrace readable.  A trap in a
// page hands over a complete stack of `wasm-function[1073]` frames, and resolving
// them needs BOTH halves, which is why one assertion here is not enough:
//
//   * the `name` custom section has to survive wasm-opt (the default build strips
//     it along with the DWARF), and
//   * the loft function has to still BE a function.  Without `#[inline(never)]`
//     LLVM folded every function of this very program into `loft_start`, and the
//     section then named 616 std/alloc internals and not one line of loft.
//
// The no-`--names` control is what proves the test can fail: it pins that the
// default page carries no section at all, so a green here cannot come from names
// that were always present.
#[test]
fn html_names_flag_makes_loft_functions_resolvable_in_a_backtrace() {
    // `--names` is implemented as `wasm-opt -g --strip-dwarf` in place of
    // `--strip-debug`, so with `wasm-opt` absent there is no name section to
    // assert on and the page is merely unoptimised — the build says so and
    // carries on.  Say SKIPPED rather than fail: this is the same self-skip the
    // rest of the file uses for a missing node / wasm target, and a machine
    // without binaryen is not a machine with a broken `--names`.  CI installs
    // binaryen (`.github/workflows/ci.yml`, the Linux tools step) precisely so
    // this stays a real gate there rather than a permanent skip.
    if !loft::build_phase::tool_on_path("wasm-opt") {
        println!("skipped: wasm-opt (binaryen) is not installed — --names has no effect");
        return;
    }
    let source = "fn ring_of(radius: float, steps: integer) -> vector<float> {\n\
                  \x20 out: vector<float> = [];\n\
                  \x20 for i in 0..steps {\n\
                  \x20   out += [radius * (i as float)];\n\
                  \x20 }\n\
                  \x20 out\n\
                  }\n\
                  fn thumb_wire(rings: integer) -> integer {\n\
                  \x20 total = 0;\n\
                  \x20 for r in 1..rings {\n\
                  \x20   for v in ring_of(r as float, 4) { if v > 0.0 { total += 1 } }\n\
                  \x20 }\n\
                  \x20 total\n\
                  }\n\
                  fn main() {\n  print(\"wire={thumb_wire(6)}\")\n}\n";

    let Some((stdout, stderr, ok)) = run_html_wasm_full(
        "n954_names",
        source,
        &[],
        &[],
        &["--names"],
        "tools/wasm_repro.mjs",
    ) else {
        return; // skipped: no node / no wasm target / no release loft
    };
    let all = format!("{stdout}{stderr}");
    // A debug flag that changes the answer is worse than no flag: pin the result.
    assert!(ok, "the --names page trapped.\n{all}");
    assert!(all.contains("wire=15"), "--names changed the answer: {all}");

    let bytes = fa::read(extracted_wasm("n954_names")).expect("read extracted wasm");
    let names = wasm_function_names(&bytes);
    assert!(
        !names.is_empty(),
        "--names must keep the wasm name section (wasm-opt -g + --strip-dwarf)"
    );
    // The generated Rust carries the loft name verbatim inside its v0 symbol
    // (`…_4prog12n_thumb_wire`), so a frame index resolves to something the author
    // wrote.  Both functions: `main` alone would pass on a build that inlined
    // everything into it.
    for want in ["n_thumb_wire", "n_ring_of"] {
        assert!(
            names.iter().any(|n| n.contains(want)),
            "no name section entry mentions `{want}` — a trap in it would still \
             resolve to nothing.\nnames from the program crate: {:?}",
            names
                .iter()
                .filter(|n| n.contains("4prog"))
                .collect::<Vec<_>>()
        );
    }

    // Control: without the flag the page carries no names at all.
    let Some((_, _, ok)) =
        run_html_wasm_full("n954_plain", source, &[], &[], &[], "tools/wasm_repro.mjs")
    else {
        return;
    };
    assert!(ok, "the control page trapped");
    let plain = fa::read(extracted_wasm("n954_plain")).expect("read control wasm");
    assert!(
        wasm_function_names(&plain).is_empty(),
        "the default page must stay stripped — otherwise this test proves nothing"
    );
}

/// P137 root case: an empty `fn main() {}` traps before any user code
/// runs.  This was the minimal reproducer that revealed
/// `Stores::new()` → `Instant::now()` as the panic site.  If WASM init
/// regresses (e.g. a future change calls another non-wasm32-safe
/// std API in `Stores::new()`), this catches it.
#[test]
fn p137_html_empty_main_does_not_trap() {
    let Some((_stdout, stderr, ok)) = run_html_wasm("p137_empty", "fn main() {}\n") else {
        return;
    };
    assert!(
        ok,
        "empty main trapped — P137-shape init regression.\n{stderr}"
    );
}

/// P137: build a `println`-only `.loft` program via `--html`, extract
/// the WASM, and run it via the Node repro harness with stub imports.
/// Exit code 0 means `loft_start` returned without trapping.
#[test]
fn p137_html_hello_world_does_not_trap() {
    let Some((stdout, stderr, ok)) = run_html_wasm(
        "p137_hello",
        "fn main() { println(\"hello from loft\"); }\n",
    ) else {
        return;
    };
    assert!(
        ok,
        "WASM trapped — P137 regression.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("hello from loft") || stderr.contains("loft_start: OK"),
        "expected 'hello from loft' in output.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// P137 follow-up: arithmetic + control flow exercise the bytecode
/// dispatch path in the WASM build.  If a future change introduces a
/// panic-able std call inside the dispatch loop (e.g. an unchecked
/// indexing in a hot opcode), this catches it.
#[test]
fn p137_html_arithmetic_loop_runs() {
    let src = "fn main() {
    sum = 0;
    for i in 0..10 { sum = sum + i; }
    println(\"sum={sum}\");
}
";
    let Some((stdout, stderr, ok)) = run_html_wasm("p137_arith", src) else {
        return;
    };
    assert!(ok, "WASM trapped on arithmetic loop.\n{stderr}");
    assert!(
        stdout.contains("sum=45"),
        "expected 'sum=45' in output.\nstdout: {stdout}"
    );
}

/// QUALITY-history.md Tier 3 #9 — `file("...")` under `--html` (wasm32
/// without the wasm host-bridge) must not trap even though there
/// is nothing at that path.  The page HAS a filesystem now (loft#851) — it just
/// starts empty unless the page seeds a base tree — so a file nobody wrote must
/// still read as `null`: a MISSING file (@PLN102 H4: `content() -> text?`,
/// missing → null, distinct from an empty file), without crashing.
///
/// This is the half of the contract that binding a filesystem could most easily
/// have broken, because the bridge's "absent" and "an empty file" arrive over
/// the same import and are told apart only by a `usize::MAX` length.
#[test]
fn q9_html_file_content_returns_empty_on_wasm() {
    let src = "fn main() {
    f = file(\"/definitely_missing_on_wasm.txt\");
    t = f.content();
    println(\"missing={t == null}\");
}
";
    let Some((stdout, stderr, ok)) = run_html_wasm("q9_file_content", src) else {
        return;
    };
    assert!(ok, "WASM trapped on file().content() call.\n{stderr}");
    assert!(
        stdout.contains("missing=true"),
        "expected 'missing=true' (a missing file reads as null on the wasm32 stub, @PLN102 H4).\
         \nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// loft#851 — a page has a filesystem, and it is the SAME one every other target
/// answers with.
///
/// `--html` used to bind none: the file calls compiled, each took an inert branch
/// and answered "absent", and `loft --html` reported success — so a rendering
/// editor in a page could draw and could not save, and the consumer found out by
/// GREPPING the emitted bundle for `fs_`.
///
/// Every expected value below is what `--interpret` and `--native` print for this
/// exact program, so the test fails both ways: if the page loses its filesystem
/// again, and if it grows one that answers differently from the other backends.
/// The empty-vs-absent pair is deliberate — the two cross the bridge as the same
/// import and are told apart only by a length sentinel.
#[test]
fn html_page_has_a_filesystem() {
    let src = r#"fn main() {
    f = file("story.txt");
    f += "hello";
    f += "-page";
    a = file("story.txt");
    println("content={a.content() ?? "<null>"} size={a#size}");
    write_bytes("blob.bin", [7, 8, 9]);
    bs = read_bytes("blob.bin") ?? [];
    println("bytes={len(bs)} first={bs[0] ?? -1}");
    mkdir_all("d/sub");
    g = file("d/sub/inner.txt");
    g += "x";
    println("dir={is_dir("d/sub")} list={list_dir("d/sub")}");
    delete("story.txt");
    println("gone={exists("story.txt")} still={exists("blob.bin")}");
    empty = file("empty.txt");
    empty += "";
    println("empty_is_null={file("empty.txt").content() == null} absent_is_null={file("nope.txt").content() == null}");
}
"#;
    let Some((stdout, stderr, ok)) = run_html_wasm("h851_page_fs", src) else {
        return;
    };
    assert!(
        ok,
        "WASM trapped while using the page filesystem.\n{stderr}"
    );
    for expected in [
        "content=hello-page size=10",
        "bytes=3 first=7",
        r#"dir=true list=["inner.txt"]"#,
        "gone=false still=true",
        "empty_is_null=false absent_is_null=true",
    ] {
        assert!(
            stdout.contains(expected),
            "expected {expected:?} from the page filesystem (loft#851)\
             \nstdout: {stdout}\nstderr: {stderr}"
        );
    }
}

/// loft#851 — the cursor.  A page has no OS file handle, so the HOST keeps the
/// read/write position per path; `f#next` and a sized `f#read(n)` have to mean
/// the same thing there as they do over a real handle.
///
/// Worth its own test because the first round of this work passed every
/// whole-file cell and still had the cursor wrong: `write_bytes` left the
/// position at the end of the file, and the next `read_bytes` — which asks for
/// the WHOLE file — started from there and answered zero bytes.
#[test]
fn html_page_filesystem_cursor_matches_the_other_backends() {
    let src = r#"fn main() {
    f = file("cur.txt");
    f += "abcde";
    g = file("cur.txt");
    g#next = 1;
    s: text = g#read(3);
    s2: text = g#read(1);
    println("seek={s ?? "<null>"} continue={s2 ?? "<null>"} next={g#next}");
    h = file("cur.txt");
    h#next = 3;
    short: text = h#read(99);
    println("short={short ?? "<null>"}");
    w = file("cur.txt");
    w#next = 0;
    w += "XY";
    println("overwrite={file("cur.txt").content() ?? "<null>"}");
    t = file("cur.txt");
    ok = t.set_file_size(2);
    println("resize={ok} left={file("cur.txt").content() ?? "<null>"}");
    move("cur.txt", "moved.txt");
    println("moved={file("moved.txt").content() ?? "<null>"} old={exists("cur.txt")}");
}
"#;
    let Some((stdout, stderr, ok)) = run_html_wasm("h851_cursor", src) else {
        return;
    };
    assert!(
        ok,
        "WASM trapped on the page filesystem's cursor.\n{stderr}"
    );
    for expected in [
        "seek=bcd continue=e next=5",
        "short=de",
        "overwrite=XYcde",
        "resize=Ok left=XY",
        "moved=XY old=false",
    ] {
        assert!(
            stdout.contains(expected),
            "expected {expected:?} — the page's per-path cursor must read as an \
             OS handle does (loft#851)\nstdout: {stdout}\nstderr: {stderr}"
        );
    }
}

/// P137 follow-up: vectors + iteration.  Exercises store allocation
/// and per-iteration access in the WASM target — would catch a
/// regression in `OpVarVector` or vector-element-access opcodes
/// emitting a panic-able path under wasm32.
#[test]
fn p137_html_vector_iteration_runs() {
    let src = "fn main() {
    items = [10, 20, 30];
    total = 0;
    for x in items { total = total + x; }
    println(\"total={total}\");
}
";
    let Some((stdout, stderr, ok)) = run_html_wasm("p137_vec", src) else {
        return;
    };
    assert!(ok, "WASM trapped on vector iteration.\n{stderr}");
    assert!(
        stdout.contains("total=60"),
        "expected 'total=60' in output.\nstdout: {stdout}"
    );
}

/// ROADMAP 0.8.5 end-to-end smoke: `lib/moros_editor` (which imports
/// `lib/moros_map`) runs cleanly under `--html`.  Exercises the full
/// edit pipeline the browser scene editor drives — paint, height, wall,
/// batched stencil stamp, undo — across the loft-side library + WASM
/// host bridge.  If any moros_editor code path traps under wasm32
/// (e.g. a future change hits a non-wasm32-safe std call), this catches
/// it before the browser build ships.
#[test]
fn moros_editor_html_smoke() {
    let src = r#"use moros_editor::*;
fn main() {
    m = map_empty();
    us = undo_empty();

    paint_material_with_undo(us, m, 0, 0, 0, 1);
    set_height_with_undo(us, m, 0, 0, 0, 3);
    set_wall_with_undo(us, m, 0, 0, 0, 0, 7);

    batch_begin(us);
    stencil_stamp_with_undo(us, m, stencil_house_small(), 2, 2, 0, 0);
    batch_end(us);

    undo_pop(us, m);

    d = undo_depth(us);
    h = map_get_hex(m, 0, 0, 0).h_material;
    println("depth={d} mat={h}");
}
"#;
    let Some((stdout, stderr, ok)) =
        run_html_wasm_with_libs("moros_editor_smoke", src, &["tests/fixtures/libs"])
    else {
        return;
    };
    assert!(ok, "moros_editor smoke trapped under --html.\n{stderr}");
    assert!(
        stdout.contains("depth=3 mat=1"),
        "expected 'depth=3 mat=1'.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// P201 regression: the poisoned-`Mutex` recovery pattern
/// `.lock().unwrap_or_else(|e| e.into_inner())` — a panicking holder must not
/// make every later `.lock()` panic with `PoisonError { .. }` (a noisy cascade
/// that hides the original failure).  The html_wasm suite no longer holds a
/// shared build mutex (builds isolate per-process now), but the recovery shape
/// is worth keeping guarded for any future shared-lock test, so this exercises
/// it on a local mutex.
#[test]
fn p201_poisoned_lock_recovery_pattern() {
    use std::sync::{Arc, Mutex};
    let m = Arc::new(Mutex::new(()));
    let m2 = Arc::clone(&m);
    let h = std::thread::spawn(move || {
        let _g = m2.lock().expect("first acquire");
        panic!("simulated test failure with the lock held");
    });
    let _ = h.join(); // expected to be Err
    assert!(
        m.is_poisoned(),
        "precondition: panicking thread must poison the mutex"
    );
    // The fix's recovery pattern — must NOT panic on a poisoned lock.
    let _guard = m.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    drop(_guard);
}

/// Parallel-compilation isolation: N concurrent `loft --html` builds must each
/// embed THEIR OWN program.  Before per-PID scratch dirs, parallel builds raced
/// on a shared `scratch/loft_html.rs` (one rustc compiled another's source → a
/// page silently embedded the wrong program) and `-o scratch/loft_html.wasm`
/// (two `rust-lld`s truncated each other mid-link → `signal: 7`, SIGBUS).  A
/// local repro hit 29 build failures + 7 cross-contaminations over 30 builds
/// pre-fix; this asserts ZERO of each.  Each `loft` is its own process, so
/// `platform::build_scratch_dir` keys the scratch on the PID.
#[test]
fn parallel_html_builds_do_not_cross_contaminate() {
    if !wasm32_target_installed() {
        eprintln!("SKIP: wasm32-unknown-unknown target not installed");
        return;
    }
    let loft_bin = repo_root().join("target/release/loft");
    if !fa::exists(&loft_bin) {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release`)");
        return;
    }
    const N: usize = 8;
    let dir = std::env::temp_dir().join(format!("loft_html_parallel_iso_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create dir");

    // One distinctively-marked program per builder.
    let srcs: Vec<PathBuf> = (0..N)
        .map(|i| {
            let src = dir.join(format!("prog_{i}.loft"));
            fa::write(
                &src,
                format!("fn main() {{ println(\"MARKER_{i}_UNIQUE\") }}\n"),
            )
            .expect("write source");
            src
        })
        .collect();

    // Launch all builds concurrently (each `loft` = a distinct process → PID).
    let mut children: Vec<_> = srcs
        .iter()
        .map(|src| {
            loft::platform::process::harness_command(&loft_bin)
                .arg("--html")
                .arg(src)
                .env("LOFT_TIMEOUT", "120")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn loft --html")
        })
        .collect();

    let mut fails: Vec<String> = Vec::new();
    for (i, child) in children.iter_mut().enumerate() {
        let status = child.wait().expect("wait for build");
        if !status.success() {
            fails.push(format!("prog_{i}: build exited {:?}", status.code()));
        }
    }

    // Each page's embedded wasm must carry ITS OWN marker and no sibling's.
    for i in 0..N {
        let page = dir.join(".loft").join(format!("prog_{i}.html"));
        let Ok(html) = fa::read_to_string(&page) else {
            fails.push(format!("prog_{i}: no page emitted"));
            continue;
        };
        let Some(b64) = html
            .split("const wasmB64=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
        else {
            fails.push(format!("prog_{i}: no wasm in page"));
            continue;
        };
        let wasm = base64_decode_standard(b64).unwrap_or_default();
        let hay = String::from_utf8_lossy(&wasm);
        if !hay.contains(&format!("MARKER_{i}_UNIQUE")) {
            fails.push(format!("prog_{i}: missing own marker (build corrupted)"));
        }
        for j in 0..N {
            if j != i && hay.contains(&format!("MARKER_{j}_UNIQUE")) {
                fails.push(format!("prog_{i}: CONTAMINATED with prog_{j}'s program"));
            }
        }
    }

    let _ = fa::remove_dir_all(&dir);
    assert!(
        fails.is_empty(),
        "parallel --html builds raced ({} issue(s)):\n{}",
        fails.len(),
        fails.join("\n")
    );
}

// ── WASM library CI gate ─────────────────────────────────────────────────────
//
// `tests/wrap.rs::library_suite` (interpreter) and
// `tests/native.rs::native_library_suite` (native) gate every
// `lib/<pkg>/tests/*.loft`.  This is the missing third backend: run those
// package tests through WebAssembly under whatever runtime is available —
// Node (the browser `feature="wasm"` flavour, via `--html` +
// `tools/wasm_repro.mjs`) and/or wasmtime (the `wasm32-wasip2` flavour).
//
// Opt-in by entry point: a lib test joins the WASM gate iff it declares a
// `fn main()` (the wasm builds use `main` as the entry; the `loft test`
// subcommand's multi-`test_*` discovery has no wasm equivalent).  Tests
// without a `main` are reported as skipped, not failed.  Both runners
// self-skip when their toolchain is absent, so the suite is a no-op on a
// machine with neither Node nor wasmtime.

/// Library packages skipped on EVERY wasm path — their tests need a host facility
/// no wasm target offers.  An entry states the facility, which gates still run
/// the package, and what would remove it (TESTING.md § Every skip says why, how
/// it runs instead, and when it ends).
const LIB_PKGS_WASM_SKIP: &[&str] = &[
    // server: its tests bind a socket and accept connections.  A browser page
    // and a WASI guest have no listen/accept — the sandbox forbids it — so the
    // limit is the platform's, by construction, and has no end condition.  The
    // package's tests run on the interpreter and `--native` gates.
    "server",
];

/// Individual `<pkg>/<file>.loft` lib tests skipped for the WASM gate.
const LIB_TESTS_WASM_SKIP: &[&str] = &[
    // process/run: `run` spawns a child (`sh`, `cat`) and drains its pipes.  A browser page
    // has no processes, and WASI preview 2 defines no process API, so the limit is the
    // platform's and has no end condition.  `process/command.loft` — building a command,
    // the typed format string — still runs here; `run` runs on the interpreter and
    // `--native` gates.
    "process/run.loft",
];

/// Packages skipped ONLY on the node (browser, `wasm32-unknown-unknown`) path.
/// Still run on wasmtime (wasip2) which has a working WASI FS via `--dir`.
/// Use this for libraries that genuinely need the filesystem — the browser
/// has none by construction (correct platform behaviour, not a bug to fix).
const LIB_PKGS_NODE_SKIP: &[&str] = &[
    // hex_world: its tests save and load a binary world file.  The browser has
    // no filesystem without a JS host bridge; wasmtime has one (`--dir <tmp>`),
    // so the package runs on the wasmtime path and on the interpreter and
    // `--native` gates.  Ends when a JS-host VirtFS bridge ships.
    "hex_world",
];

/// Packages skipped ONLY on the wasmtime (wasip2) path.  Use this for
/// libraries whose host bridge depends on a browser-only API (Canvas,
/// createImageBitmap, etc.) — wasmtime has no canvas / image codec; the
/// browser does.
const LIB_PKGS_WASMTIME_SKIP: &[&str] = &[
    // imaging: its PNG decode is the browser's (`createImageBitmap` + canvas
    // `getImageData`, through `lib/imaging/wasm/{src/lib.rs, host.js}` and
    // `doc/loft-gl-wasm.js`).  Wasmtime has no canvas or image codec, so the
    // bridge call answers false and `assert(img.width == 256)` fails for a
    // reason that is not loft's.  The package runs on the browser path and on
    // the interpreter and `--native` gates.  Ends when the wasm bridge carries
    // a pure-wasm PNG decoder, so wasmtime decodes without a host canvas.
    "imaging",
    // input: it depends on `graphics`, whose native crate is not in the
    // wasmtime (wasip2) sysroot, so the `--html` build fails at library
    // resolution (E0463) before any test runs — a packaging limit, not a
    // language one.  The package runs on the interpreter, `--native` and the
    // browser path.  Ends when the graphics crate is built for wasip2.
    "input",
];

/// Collect every `lib/<pkg>/tests/*.loft`, sorted.
fn collect_lib_wasm_tests() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    let Ok(pkgs) = fa::read_dir(repo_root().join("lib")) else {
        return files;
    };
    for pkg in pkgs {
        // Skip the `.loft_test_tmp_*` artifact-isolation dirs that the interp /
        // native lib suites create as siblings inside lib/ (run_lib_test_in_temp_cwd)
        // — they exist only transiently and must not be discovered as packages.
        if pkg.file_name().unwrap_or_default().starts_with('.') {
            continue;
        }
        let tests_dir = pkg.os_spelling().join("tests");
        let Ok(entries) = fa::read_dir(&tests_dir) else {
            continue;
        };
        for f in entries {
            let p = f.os_spelling();
            if fa::extension(&p).is_some_and(|e| e.eq_ignore_ascii_case("loft")) {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

/// `(pkg, file)` key for an entry, e.g. `("time", "01-basics.loft")`.
fn lib_test_key(entry: &std::path::Path) -> (String, String) {
    let file = fa::file_name(entry).unwrap_or_default();
    let pkg = entry
        .parent()
        .and_then(|d| d.parent())
        .and_then(fa::file_name)
        .unwrap_or_default();
    (pkg, file)
}

/// Locate `wasmtime` on `PATH` or in the default `~/.wasmtime/bin` install dir.
fn find_wasmtime() -> Option<PathBuf> {
    if let Some(p) = which("wasmtime") {
        return Some(p);
    }
    let home = std::env::var_os("HOME")?;
    let p = PathBuf::from(home).join(".wasmtime/bin/wasmtime");
    fa::exists(&p).then_some(p)
}

/// Run `source` on `wasm32-wasip2` under wasmtime.  Emits native Rust via the
/// loft binary, injects a WASI-stdout `loft_host_print` shim (the codegen calls
/// `crate::loft_host_print` on wasm32 without the `wasm` feature, but only the
/// browser path declares that import), compiles against the prebuilt loft
/// `wasm32-wasip2` rlib, and runs it.  Returns `(output, ok)`, or `None` when a
/// prerequisite (wasmtime, rustc, the rlib) is missing.
fn run_wasip2_wasm(name: &str, source: &str, lib_dirs: &[&str]) -> Option<(String, bool)> {
    let wasmtime = find_wasmtime()?;
    which("rustc")?;
    let root = repo_root();
    let loft_bin = root.join("target/release/loft");
    if !fa::exists(&loft_bin) {
        return None;
    }
    // Prefer the release rlib (the Makefile-built one); fall back to debug.
    let (rlib, deps) = ["release", "debug"].iter().find_map(|prof| {
        let r = root.join(format!("target/wasm32-wasip2/{prof}/libloft.rlib"));
        fa::exists(&r).then(|| (r, root.join(format!("target/wasm32-wasip2/{prof}/deps"))))
    })?;

    let tmp = std::env::temp_dir();
    let src = tmp.join(format!("{name}.loft"));
    let rs = tmp.join(format!("{name}_wasip2.rs"));
    let wasm = tmp.join(format!("{name}_wasip2.wasm"));
    fa::write(&src, source).ok()?;

    let mut emit = loft::platform::process::harness_command(&loft_bin);
    emit.args([
        "--native-emit",
        rs.to_str()?,
        "--path",
        &format!("{}/", root.display()),
    ]);
    for d in lib_dirs {
        emit.arg("--lib").arg(root.join(d));
    }
    emit.arg(src.to_str()?);
    if !emit.output().ok()?.status.success() {
        return Some(("loft --native-emit failed".to_string(), false));
    }

    // wasip2 print() now lowers to std `print!` (WASI stdout) directly — the
    // generated source is self-contained, no host-import shim needed (#268).
    let compile = loft::platform::process::harness_command("rustc")
        .args([
            "--edition=2024",
            "--target",
            "wasm32-wasip2",
            "--crate-type",
            "bin",
            "-O",
            "--extern",
        ])
        .arg(format!("loft={}", rlib.display()))
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("-o")
        .arg(&wasm)
        .arg(&rs)
        .output()
        .ok()?;
    if !compile.status.success() {
        return Some((String::from_utf8_lossy(&compile.stderr).into_owned(), false));
    }
    // @P334 fix (2026-05-29): preopen the dirs a test program may touch so
    // wasip2 file I/O works.  The wasip2 filesystem is sandboxed — without a
    // preopen every file op fails with "os error 44" and any file-using lib
    // traps the moment it opens a path.
    //
    // Robustness: preopen a SET of roots, not just `std::env::temp_dir()`.
    // A test that hardcodes a `/tmp/...` literal (e.g. lib/hex_world's save/load
    // fixture) traps when the harness's TMPDIR points elsewhere
    // (`/tmp/claude-1000`, a CI sandbox dir, …) and only that dir is preopened.
    // Preopening the literal `/tmp` mount AND the temp dir AND the CWD covers
    // /tmp-literal, temp-default, and CWD-relative file paths alike, so a lib
    // test no longer has to know how the harness configured TMPDIR.  Each dir
    // is mapped host==guest; duplicates and missing dirs are skipped (a host with no
    // `/tmp`, such as Windows, skips that one).
    let mut preopens: Vec<std::path::PathBuf> = Vec::new();
    let mut add = |d: std::path::PathBuf| {
        if fa::is_dir(&d) && !preopens.contains(&d) {
            preopens.push(d);
        }
    };
    add(tmp.clone());
    add(std::path::PathBuf::from("/tmp"));
    if let Ok(cwd) = std::env::current_dir() {
        add(cwd);
    }
    let mut cmd = loft::platform::process::harness_command(&wasmtime);
    for dir in &preopens {
        cmd.arg("--dir").arg(dir);
    }
    let run = cmd.arg(&wasm).output().ok()?;
    // On failure, surface stderr too — a wasip2 trap (sandbox denial, missing
    // preopen, panic) writes its diagnostic to stderr, and reporting only
    // stdout left earlier failures with an empty, undiagnosable message.
    let report = if run.status.success() {
        String::from_utf8_lossy(&run.stdout).into_owned()
    } else {
        let mut s = String::from_utf8_lossy(&run.stdout).into_owned();
        let err = String::from_utf8_lossy(&run.stderr);
        if !err.trim().is_empty() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str("stderr: ");
            s.push_str(err.trim());
        }
        s
    };
    Some((report, run.status.success()))
}

/// @P268 regression: `print()` on wasip2 (`--native-wasm`) used to fail to
/// compile (E0425) — codegen called `crate::loft_host_print`, which is declared
/// only for the browser host-import path. wasip2 has working WASI stdout, so its
/// `print` branch now lowers to std `print!`. A printing program must both
/// compile and emit to stdout under wasmtime. Self-skips when the wasm toolchain
/// (rustc wasm32-wasip2 + wasmtime + rlib) is unavailable.
#[test]
fn wasip2_print_writes_stdout_p268() {
    let src = "fn main() { print(\"p268_ok\\n\"); }\n";
    let Some((out, ok)) = run_wasip2_wasm("p268_print", src, &[]) else {
        return;
    };
    assert!(ok, "wasip2 print program failed to build/run.\n{out}");
    assert!(
        out.contains("p268_ok"),
        "expected wasip2 stdout to contain 'p268_ok'.\nout: {out}"
    );
}

/// #521 regression: #518's native stack-overflow guard wrapped the generated
/// `main` in a `std::thread::Builder::new().stack_size(..).spawn(..)`.  Thread
/// spawn is `Unsupported` on wasm (wasip2 has no threads by default), so EVERY
/// `--native-wasm` program aborted at boot with "failed to spawn main-stack
/// thread" before `main` ran.  The large-stack thread is now
/// `#[cfg(not(target_family = "wasm"))]`; wasm runs `main` directly.  A trivial
/// program must therefore run to completion under wasmtime.  Self-skips when the
/// wasm toolchain (rustc wasm32-wasip2 + wasmtime + rlib) is unavailable.
#[test]
fn native_wasm_main_runs_without_main_stack_thread_521() {
    let src = "fn main() { println(\"wasm521_ok\") }\n";
    let Some((out, ok)) = run_wasip2_wasm("wasm521_main", src, &[]) else {
        return;
    };
    assert!(
        ok,
        "native-wasm program aborted (main-stack thread spawn regression #521?).\n{out}"
    );
    assert!(
        out.contains("wasm521_ok"),
        "expected native-wasm stdout to contain 'wasm521_ok'.\nout: {out}"
    );
    assert!(
        !out.contains("failed to spawn main-stack thread"),
        "#518's main-stack thread spawn reached the wasm target again (#521).\nout: {out}"
    );
}

/// WASM gate over `lib/<pkg>/tests/*.loft` — runs each main()-bearing lib test
/// under Node (browser flavour) and wasmtime (wasip2) when available.  No-op
/// when neither runtime is present.
#[test]
fn wasm_library_suite() {
    let have_node = which("node").is_some();
    let have_wasmtime = find_wasmtime().is_some();
    if !have_node && !have_wasmtime {
        eprintln!("SKIP wasm_library_suite: neither node nor wasmtime available");
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    let mut ran = 0usize;
    for entry in collect_lib_wasm_tests() {
        let (pkg, file) = lib_test_key(&entry);
        if LIB_PKGS_WASM_SKIP.contains(&pkg.as_str())
            || LIB_TESTS_WASM_SKIP.contains(&format!("{pkg}/{file}").as_str())
        {
            println!("skip {pkg}/{file} (LIB_*_WASM_SKIP)");
            continue;
        }
        let source = match fa::read_to_string(&entry) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Opt-in: only main()-bearing lib tests have a wasm entry point.
        if !source.contains("fn main(") {
            println!("skip {pkg}/{file} (no fn main — no wasm entry)");
            continue;
        }
        let stem = fa::file_stem(&entry)
            .unwrap_or_default()
            .replace(['-', '.'], "_");
        let name = format!("libwasm_{pkg}_{stem}");

        // @P321(c) Phase 3a: collect *.png siblings of the source so the
        // synthesised /tmp/<name>.loft has the same assets next to it as
        // the original lib test, and the --html driver's auto-discovery
        // can embed them.
        let assets: Vec<PathBuf> = entry
            .parent()
            .and_then(|d| fa::read_dir(d).ok())
            .into_iter()
            .flatten()
            .map(|d| d.os_spelling())
            .filter(|p| fa::extension(p).is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .collect();

        if have_node && !LIB_PKGS_NODE_SKIP.contains(&pkg.as_str()) {
            // `run_html_wasm_with_libs` asserts on a `--html` build failure;
            // catch it so one un-buildable lib is recorded, not fatal to the
            // whole suite (and can't mask later libs' results).
            let assets_clone = assets.clone();
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_html_wasm_with_libs_and_assets(&name, &source, &["lib"], &assets_clone)
            }));
            match res {
                Ok(Some((stdout, stderr, ok))) => {
                    ran += 1;
                    println!(
                        "wasm[node] {pkg}/{file}: {}",
                        if ok { "ok" } else { "FAIL" }
                    );
                    let _ = stdout;
                    if !ok {
                        let tail: Vec<&str> = stderr.lines().rev().take(3).collect();
                        failures.push(format!("{pkg}/{file} [node]: {}", tail.join(" | ")));
                    }
                }
                Ok(None) => {} // prerequisites missing — self-skipped
                Err(_) => {
                    ran += 1;
                    println!("wasm[node] {pkg}/{file}: FAIL (build panicked)");
                    failures.push(format!("{pkg}/{file} [node]: --html build failed"));
                }
            }
        }
        if have_wasmtime
            && !LIB_PKGS_WASMTIME_SKIP.contains(&pkg.as_str())
            && let Some((out, ok)) = run_wasip2_wasm(&name, &source, &["lib"])
        {
            ran += 1;
            println!(
                "wasm[wasmtime] {pkg}/{file}: {}",
                if ok { "ok" } else { "FAIL" }
            );
            if !ok {
                let tail: Vec<&str> = out.lines().rev().take(3).collect();
                failures.push(format!("{pkg}/{file} [wasmtime]: {}", tail.join(" | ")));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} wasm library run(s) failed:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
    println!("wasm_library_suite: {ran} wasm run(s) passed");
}

// Minimal base64 decoder — avoids adding a dev-dependency for one
// test.  Handles the standard alphabet (no URL-safe variant needed;
// the loft HTML writer uses `+`/`/`/`=`).
fn base64_decode_standard(s: &str) -> Option<Vec<u8>> {
    const T: [i8; 128] = {
        let mut t = [-1i8; 128];
        let mut i = 0;
        while i < 26 {
            t[b'A' as usize + i] = i as i8;
            t[b'a' as usize + i] = (i + 26) as i8;
            i += 1;
        }
        let mut i = 0;
        while i < 10 {
            t[b'0' as usize + i] = (i + 52) as i8;
            i += 1;
        }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };
    let s = s.as_bytes();
    if !s.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut i = 0;
    while i < s.len() {
        let mut v = [0i32; 4];
        let mut pad = 0;
        for j in 0..4 {
            let b = s[i + j];
            if b == b'=' {
                pad += 1;
                v[j] = 0;
            } else if (b as usize) < 128 && T[b as usize] >= 0 {
                v[j] = T[b as usize] as i32;
            } else {
                return None;
            }
        }
        let combined = (v[0] << 18) | (v[1] << 12) | (v[2] << 6) | v[3];
        out.push(((combined >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((combined >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((combined & 0xff) as u8);
        }
        i += 4;
    }
    Some(out)
}

/// @PLN26 phase 3 (#438) — a program that uses a `#native` (`[native] crate`) package
/// compiles to `--native-wasm` and runs under wasmtime: loft cross-builds the package's
/// native crate to `wasm32-wasip2` on demand and links its rlib (+ the host proc-macro
/// deps).  `native_scalar_pkg` is a minimal wasm-clean fixture, so this pins the loft
/// cross-build PIPELINE, isolated from any heavy crate's own wasm-cleanliness.  Skipped
/// when the wasip2 target or wasmtime is absent.
///
/// Both native SHAPES, because they have different linkage requirements and only one of
/// them was ever covered here (loft#967).  A scalar (`native_answer() -> 42`) compiles to
/// a plain `extern "C"` call that asks nothing of loft's runtime.  A vector RETURN
/// (`native_span(4)`) makes codegen wrap the call in `loft::native_call::enter` +
/// `build_store` so the crate can allocate into a loft store — and those two helpers were
/// gated behind `native-extensions`, a feature the wasm runtime rlib cannot enable
/// (it buys `dlopen`).  So every store-touching native refused to build on this target
/// while the scalar cell stayed green and reported the pipeline as covered.
#[test]
fn pln26_phase3_native_package_runs_on_wasm() {
    let Some(wasmtime) = which("wasmtime") else {
        eprintln!("skip: wasmtime not installed");
        return;
    };
    let have_wasip2 = loft::platform::process::harness_command("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.lines().any(|l| l == "wasm32-wasip2"));
    if !have_wasip2 {
        eprintln!("skip: wasm32-wasip2 target not installed");
        return;
    }
    let root = repo_root();
    let loft_bin = root.join("target/release/loft");
    if !fa::exists(&loft_bin) {
        eprintln!("skip: target/release/loft not built");
        return;
    }
    let tmp = std::env::temp_dir().join("loft_pln26_p3");
    let _ = fa::create_dir_all(&tmp);
    let prog = tmp.join("native_pkg_wasm.loft");
    // Values, not just shapes: `native_span` returns `[100, 101, …]` deterministically,
    // so a truncated or misordered store copy is visible rather than merely "non-empty".
    fa::write(
        &prog,
        "use native_scalar_pkg::*;\nfn main() {\n  answer = native_answer();\n  \
         print(\"native-answer={answer}\\n\");\n  v = native_span(4);\n  \
         print(\"native-span={len(v)}:{v[0]},{v[1]},{v[2]},{v[3]}\\n\");\n}\n",
    )
    .unwrap();
    let wasm = tmp.join("native_pkg_wasm.wasm");
    let _ = fa::remove_file(&wasm);
    // Clean any prior wasm cross-build so the ON-DEMAND build path is exercised.
    let _ =
        fa::remove_dir_all(root.join("tests/lib/native_scalar_pkg/native/target/wasm32-wasip2"));

    let build = loft::platform::process::harness_command(&loft_bin)
        .arg("--native-wasm")
        .arg(&wasm)
        .arg("--lib")
        .arg(root.join("tests/lib"))
        .arg(&prog)
        .output()
        .expect("run loft --native-wasm");
    assert!(
        build.status.success() && fa::exists(&wasm),
        "loft --native-wasm of a #native-package program failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = loft::platform::process::harness_command(&wasmtime)
        .arg(&wasm)
        .output()
        .expect("run wasmtime");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("native-answer=42"),
        "wasm `#native` call returned wrong/no value: stdout={stdout:?} stderr={}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        stdout.contains("native-span=4:100,101,102,103"),
        "loft#967 — a store-allocating `#native` returned the wrong vector on wasip2: \
         stdout={stdout:?} stderr={}",
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Tier 1 + 2 of the browser byte channel, round-tripped headlessly:
/// `host_output` (loft->JS structured message) is echoed back by the
/// harness loopback as `echo:<msg>` into the input QUEUE, and
/// `host_input()` pops exactly one message per call — the second read is
/// empty.  This is the whole "JS owns the network" request/response
/// pattern with the fetch replaced by a deterministic echo.
#[test]
fn host_output_input_roundtrip_queue() {
    let src = r#"fn main() {
  host_output("fetch http://unit.test/tile");
  reply = host_input();
  println("reply=[{reply}]");
  second = host_input();
  println("second=[{second}]");
}
"#;
    let Some((stdout, stderr, ok)) = run_html_wasm("host_roundtrip", src) else {
        return; // prerequisites missing — skipped with a note
    };
    assert!(ok, "wasm run failed; stderr: {stderr}");
    assert!(
        stdout.contains("reply=[echo:fetch http://unit.test/tile]"),
        "loopback reply missing; stdout: {stdout}"
    );
    assert!(
        stdout.contains("second=[]"),
        "queue must be empty after one pop; stdout: {stdout}"
    );
}

// @PLN98 P3.4 — the live/debug tier's core primitive verified IN THE BROWSER WASM
// RUNTIME (headless node): a `--debug` client bootstraps the parked interpreter
// from the EMBEDDED source (P3.1, no filesystem), flips its fns to the interpreter,
// and the compiled `main` DISPATCHES a call into the interpreter running over the
// SHARED store — producing the correct result. This is "the interpreter over the
// shared live store" (the whole tier's one primitive) proven on wasm32.
#[test]
fn html_debug_client_dispatches_to_the_interpreter_on_wasm() {
    let source = "fn addup(a: integer, b: integer) -> integer {\n  a + b\n}\n\
                  fn triple(x: integer) -> integer {\n  addup(x, addup(x, x))\n}\n\
                  fn main() {\n  print(\"triple(14)={triple(14)}\")\n}\n";
    let Some((stdout, stderr, ok)) = run_html_wasm_full(
        "p98_debug_dispatch",
        source,
        &[],
        &[],
        &["--debug=alice"],
        "tools/wasm_repro.mjs",
    ) else {
        return; // skipped: no node / no wasm target / no release loft
    };
    let all = format!("{stdout}{stderr}");
    assert!(ok, "the --debug wasm client trapped.\n{all}");
    // The parked interpreter switched on and flipped the dispatchable fns.
    assert!(
        all.contains("flipped 2 fn(s) to the interpreter"),
        "the debug client flipped its fns to the interpreter: {all}"
    );
    // A compiled call dispatched INTO the interpreter over the shared store.
    assert!(
        all.contains("dispatched 1 interp call(s) over the shared store"),
        "a compiled call dispatched into the interpreter on wasm: {all}"
    );
    // And it computed the right answer over that shared store.
    assert!(
        all.contains("triple(14)=42"),
        "the interpreted dispatch produced the correct result: {all}"
    );
}

// @PLN98 P3.4 — the interpreter's COOPERATIVE DEBUG CYCLE (breakpoint pause +
// frame read + resume, P3.2) verified IN THE BROWSER WASM RUNTIME (headless node).
// The `--debug` build exports `loft_debug_selftest`, which runs the exact cycle
// (set a breakpoint, run — `execute_argv` returns at the breakpoint, read the
// paused frame's local, step, resume) and reports the outcome; identical to the
// native result. This proves the make-or-break browser pause works on wasm32.
#[test]
fn html_debug_cooperative_pause_cycle_runs_on_wasm() {
    let Some((stdout, stderr, ok)) = run_html_wasm_full(
        "p98_debug_selftest",
        "fn main() { print(\"x\") }\n",
        &[],
        &[],
        &["--debug=alice"],
        "tools/wasm_debug_selftest.mjs",
    ) else {
        return;
    };
    let all = format!("{stdout}{stderr}");
    assert!(ok, "the debug-selftest harness failed: {all}");
    assert!(
        all.contains("PAUSE n=40 STEP m=42 DONE=true"),
        "the cooperative debug cycle ran on wasm: {all}"
    );
    assert!(
        all.contains("RETURN=1"),
        "the selftest reported success: {all}"
    );
}

// @PLN98 P3.4 — the INTERACTIVE browser debug CLIENT on wasm: the `--html --debug`
// build applies `D!:` control frames relayed over host_input (bp -> run -> eval ->
// resume) and returns `D:` replies over host output, entirely in the wasm runtime
// (no threads / sockets / native debug_cmd_dispatch). This is the client half of
// the server->browser debug relay, verified headlessly via node.
#[test]
fn html_debug_client_applies_relayed_control_frames_on_wasm() {
    let source = "fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\n\
                  fn main() { compute(40); }\n";
    let Some((stdout, stderr, ok)) = run_html_wasm_full(
        "p98_debug_client",
        source,
        &[],
        &[],
        &["--debug=alice"],
        "tools/wasm_debug_client.mjs",
    ) else {
        return;
    };
    let all = format!("{stdout}{stderr}");
    assert!(ok, "the debug-client driver failed: {all}");
    assert!(
        all.contains("D:ok bp compute"),
        "breakpoint set over host_input: {all}"
    );
    assert!(
        all.contains("D:hit compute") && all.contains("n=40"),
        "the client PAUSED at the breakpoint with the live frame (n=40): {all}"
    );
    assert!(
        all.contains("D:eval n=40"),
        "read a live frame local over the relay: {all}"
    );
    assert!(
        all.contains("D:eval n + 2=42"),
        "FULL-EXPRESSION eval over the relayed control channel: {all}"
    );
    assert!(
        all.contains("D:terminated"),
        "resume ran to completion: {all}"
    );
}

// @PLN98 Probe 2 — "sharing is ONE heap on wasm too": a COMPILED write and an
// INTERPRETED read of the same variable must AGREE (and vice versa), or the
// live-dispatch swap didn't carry the world. In the `--html --debug` build,
// `flip_all` flips `reader`+`writer` to the interpreter while `main` stays
// compiled, so: main writes `w.a=777` (compiled) -> `reader` reads it
// (interpreted); `writer` writes `w.b=999` (interpreted) -> main reads it
// (compiled). Both must round-trip over the one shared store.
#[test]
fn html_debug_one_shared_heap_compiled_and_interpreted_agree_on_wasm() {
    let source = "struct W { a: integer, b: integer }\n\
                  fn reader(w: W) -> integer { w.a }\n\
                  fn writer(w: W) { w.b = 999; }\n\
                  fn main() {\n  \
                    w = W { a: 0, b: 0 };\n  \
                    w.a = 777;\n  \
                    r = reader(w);\n  \
                    writer(w);\n  \
                    print(\"r={r} b={w.b}\")\n}\n";
    let Some((stdout, stderr, ok)) = run_html_wasm_full(
        "p98_one_heap",
        source,
        &[],
        &[],
        &["--debug=alice"],
        "tools/wasm_repro.mjs",
    ) else {
        return;
    };
    let all = format!("{stdout}{stderr}");
    assert!(ok, "the one-heap wasm client trapped: {all}");
    assert!(
        all.contains("flipped 2 fn(s) to the interpreter"),
        "reader + writer flipped to the interpreter: {all}"
    );
    // The load-bearing assertion: compiled write -> interpreted read (r=777) AND
    // interpreted write -> compiled read (b=999) both agree over ONE heap.
    assert!(
        all.contains("r=777 b=999"),
        "compiled and interpreted share one heap (both directions): {all}"
    );
    assert!(
        all.contains("dispatched 2 interp call(s) over the shared store"),
        "both flipped fns dispatched over the shared store: {all}"
    );
}

/// #623 — a `#native` symbol with no `[wasm.bridge].routes` entry must produce
/// ONE loft-authored diagnostic naming the symbol, its library, and the
/// `loft.toml` fix — not the raw rustc cascade it used to.
///
/// The emitted crate carried both a host-import `extern` declaration and a local
/// wrapper body under the same name, so the build failed with `E0428` plus a
/// string of `E0610`/`E0061` against generated code (rustc's "remove the extra
/// argument" pointing at nothing the author can act on).  Diagnosing one instance
/// that way cost an hour of bisecting per-native `--html` builds.
///
/// Reachability-scoped, matching P269 / @PLN26: a routeless `#native` that is
/// merely DECLARED must not reject an otherwise-valid program, so this also
/// asserts the uncalled case still builds.
#[test]
fn issue623_routeless_native_reports_missing_wasm_bridge_route() {
    if !wasm32_target_installed() {
        eprintln!("SKIP: rustup target wasm32-unknown-unknown not installed");
        return;
    }
    let loft_bin = repo_root().join("target/release/loft");
    if !fa::exists(&loft_bin) {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release` first)");
        return;
    }

    let tmp = std::env::temp_dir().join("loft_html_issue623");
    let _ = fa::remove_dir_all(&tmp);
    let lib_src = tmp.join("nobridge/src");
    fa::create_dir_all(&lib_src).expect("create fixture lib dir");
    fa::write(
        tmp.join("nobridge/loft.toml"),
        "[package]\nname = \"nobridge\"\nversion = \"0.1.0\"\n\n\
         [library]\nentry = \"src/nobridge.loft\"\nnative = \"loft_nobridge\"\n",
    )
    .expect("write fixture loft.toml");
    // A `#native` with NO [wasm.bridge].routes entry — the #623 shape.
    fa::write(
        lib_src.join("nobridge.loft"),
        "pub fn hash_b64(data: text) -> text;\n#native\n",
    )
    .expect("write fixture lib source");

    let build = |name: &str, program: &str| -> String {
        let src = tmp.join(format!("{name}.loft"));
        fa::write(&src, program).expect("write program");
        let out = loft::platform::process::harness_command(&loft_bin)
            .current_dir(&tmp)
            .arg(&src)
            .arg("--lib")
            .arg(tmp.join("nobridge"))
            .arg("--html")
            .arg(tmp.join(format!("{name}.html")))
            .output()
            .expect("run loft --html");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };

    // CALLED → the clean diagnostic, and none of the old cascade.
    let called = build(
        "issue623_called",
        "use nobridge::*;\nfn main() {\n  h = hash_b64(\"abc\");\n  println(\"h={h}\");\n}\n",
    );
    assert!(
        called.contains("has no [wasm.bridge].routes entry"),
        "expected the missing-route diagnostic, got:\n{called}"
    );
    assert!(
        called.contains("hash_b64") && called.contains("n_hash_b64"),
        "diagnostic must name the loft fn and its #native symbol:\n{called}"
    );
    assert!(
        called.contains("[wasm.bridge.routes]") && called.contains("loft.toml"),
        "diagnostic must name the fix (loft.toml [wasm.bridge.routes]):\n{called}"
    );
    assert!(
        !called.contains("E0428"),
        "the duplicate-definition cascade must be gone:\n{called}"
    );

    // DECLARED BUT NOT CALLED → still builds; an unused routeless native is not
    // an error (it emits no body, so nothing collides).
    let uncalled = build(
        "issue623_uncalled",
        "use nobridge::*;\nfn main() {\n  println(\"no native call here\");\n}\n",
    );
    assert!(
        !uncalled.contains("has no [wasm.bridge].routes entry"),
        "an uncalled routeless #native must not reject the program:\n{uncalled}"
    );
    assert!(
        !uncalled.contains("E0428"),
        "uncalled routeless #native must still build cleanly:\n{uncalled}"
    );
    // Positive proof, so the two `!contains` above cannot pass on a build that
    // failed for some unrelated reason.
    assert!(
        fa::exists(tmp.join("issue623_uncalled.html")),
        "the uncalled case must actually emit its bundle:\n{uncalled}"
    );

    // A routeless `#native` whose SYMBOL DIFFERS from the fn name is the
    // legitimate raw host-import path (`graphics`' `#native
    // "loft_gl_swap_buffers"` on `gl_swap_buffers`): the declaration and the
    // wrapper have different names, so nothing collides and the browser shell
    // supplies the import.  Only the BARE `#native` form — symbol == the fn's own
    // emitted `n_<name>` — is the #623 shape.  Guard it: an over-broad
    // "routeless is an error" rule breaks every WebGL `--html` program.
    // The symbol must be one the browser shell ACTUALLY provides.  It used to be a
    // made-up `host_scalar_op`, which no shell defines — so the case asserted that a
    // page importing a function nothing supplies builds fine, and such a page dies at
    // instantiate with a LinkError (loft#668, which now rejects it at build time).
    // `loft_gl_key_pressed` is real, and its `integer -> integer` shape is the same.
    fa::write(
        lib_src.join("nobridge.loft"),
        "pub fn scalar_op(n: integer) -> integer;\n#native \"loft_gl_key_pressed\"\n",
    )
    .expect("rewrite fixture lib source");
    let distinct = build(
        "issue623_distinct_symbol",
        "use nobridge::*;\nfn main() {\n  n = scalar_op(2);\n  println(\"n={n}\");\n}\n",
    );
    assert!(
        !distinct.contains("has no [wasm.bridge].routes entry"),
        "a #native with a DISTINCT symbol is the working host-import path and \
         must not be rejected:\n{distinct}"
    );
    assert!(
        fa::exists(tmp.join("issue623_distinct_symbol.html")),
        "the distinct-symbol case must still emit its bundle:\n{distinct}"
    );
}

/// #620 regression: `ticks()` and `now()` returned a hard-coded 0 on
/// `--native-wasm` (wasip2).
///
/// The stub was gated on `target_arch = "wasm32"` + `not(feature = "wasm")`,
/// which the comments described as "the `--html` build" but which equally caught
/// wasip2 — a target with a REAL clock (`wasi:clocks`, reached through `std`'s
/// `SystemTime`/`Instant`).  So this was not a missing bridge standing in with a
/// placeholder; it was a working clock being overridden with 0.  The gate is now
/// `not(target_os = "wasi")`, i.e. the browser build it always meant.
///
/// A stopped clock is invisible from inside the program — `0` is a plausible
/// elapsed time — so the guard measures a section with real work in it and
/// asserts the clock MOVED, plus that `now()` is a genuine epoch reading rather
/// than a small counter.  Self-skips without the wasm toolchain.
#[test]
fn issue620_wasip2_clocks_are_real_not_zero() {
    // ~2M iterations of a dependent arithmetic chain: tens of milliseconds of
    // real work, so a live clock cannot read 0 across it.
    let src = "fn burn(seed: integer) -> integer {\n\
               \x20   h = seed + 1;\n\
               \x20   for _ in 0..2000000 { h = (h * 1103515245 + 12345) % 2147483647; }\n\
               \x20   h\n\
               }\n\
               fn main() {\n\
               \x20 t0 = ticks(); n0 = now();\n\
               \x20 guard = burn(1);\n\
               \x20 t1 = ticks();\n\
               \x20 println(\"ticks_moved={t1 - t0 > 0}\");\n\
               \x20 println(\"now_is_epoch={n0 > 1600000000000}\");\n\
               \x20 println(\"guard={guard}\");\n\
               }\n";
    let Some((out, ok)) = run_wasip2_wasm("issue620_clocks", src, &[]) else {
        return;
    };
    assert!(ok, "wasip2 clock program failed to build/run.\n{out}");
    // The work demonstrably ran and produced the same value every backend gives,
    // so a stopped clock cannot be blamed on the section being optimised away.
    assert!(
        out.contains("guard=152472650"),
        "the timed section must actually run (guard value).\nout: {out}"
    );
    assert!(
        out.contains("ticks_moved=true"),
        "ticks() must advance across real work on wasip2 (#620).\nout: {out}"
    );
    assert!(
        out.contains("now_is_epoch=true"),
        "now() must return real epoch milliseconds on wasip2, not 0 (#620).\nout: {out}"
    );
}

/// #620, the OTHER half: the `--html` (browser) target.  The first fix made
/// `--native-wasm` (wasip2) read a real clock, but the browser build has no
/// `std` clock at all and returned a hardcoded 0 — so `now()` read the same
/// instant forever and every duration measured 0ms, silently.  It now reads the
/// host's `Date.now()` / `performance.now()` through two `loft_io` imports.
///
/// `ticks_moved` is the load-bearing assertion: it needs the clock to ADVANCE
/// across real work, which a constant (0 or otherwise) cannot do.  The `guard`
/// value proves the timed section actually ran, so a stopped clock can never be
/// excused as the loop being optimised away.
#[test]
fn issue620_html_browser_clocks_are_real_not_zero() {
    let src = "fn burn(seed: integer) -> integer {\n\
               \x20   h = seed + 1;\n\
               \x20   for _ in 0..2000000 { h = (h * 1103515245 + 12345) % 2147483647; }\n\
               \x20   h\n\
               }\n\
               fn main() {\n\
               \x20 t0 = ticks(); n0 = now();\n\
               \x20 guard = burn(1);\n\
               \x20 t1 = ticks();\n\
               \x20 println(\"ticks_moved={t1 - t0 > 0}\");\n\
               \x20 println(\"now_is_epoch={n0 > 1600000000000}\");\n\
               \x20 println(\"guard={guard}\");\n\
               }\n";
    let Some((stdout, stderr, ok)) = run_html_wasm("issue620_html_clocks", src) else {
        return;
    };
    assert!(
        ok,
        "browser clock program trapped.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("guard=152472650"),
        "the timed section must actually run (guard value).\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("ticks_moved=true"),
        "ticks() must advance across real work on --html (#620).\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("now_is_epoch=true"),
        "now() must return real epoch milliseconds on --html, not 0 (#620).\nstdout: {stdout}"
    );
}

/// @PLN24 arc E — the availability query compiles on the browser target.
///
/// `c_library_available` is the idiom a `#c` library is told to use before
/// calling into C, and under `--html` it did not COMPILE: its `#rust` body reads
/// the `__C_LIBS` / `__C_LIB_SYMS` tables, and those were emitted only on
/// non-browser targets (`E0425: cannot find value __C_LIB_SYMS`).  So the one
/// question that lets a program survive a missing library was itself unavailable
/// exactly where every library is missing.  It now answers **false**, which is
/// not a stub: a wasm module has no `dlopen`, so no C library is available.
#[test]
fn pln24_html_c_library_available_compiles_and_answers_false() {
    let Some((stdout, stderr, ok)) = run_html_wasm(
        "pln24_avail",
        "fn main() { println(\"avail {c_library_available(\"libc.so.6\")}\") }\n",
    ) else {
        return;
    };
    assert!(ok, "the browser build must succeed: {stdout}{stderr}");
    assert!(
        stdout.contains("avail false"),
        "no wasm module can dlopen a C library: {stdout}{stderr}"
    );
}

/// @PLN24 arc E — a reachable `#c` call is refused with ONE named message, on
/// both wasm shapes, and an unreachable declaration still builds.
///
/// The emission-level guarantee is pinned in `tests/native.rs`; this is the half
/// that only an end-to-end run can show — that what reaches the author is a
/// single sentence naming their function, the C symbol and the target, rather
/// than a rustc cascade against loft's own internals.
#[test]
fn pln24_a_reachable_c_binding_is_refused_end_to_end_on_wasm() {
    let loft_bin = repo_root().join("target/release/loft");
    if !fa::exists(&loft_bin) || !wasm32_target_installed() {
        eprintln!("SKIP: release binary or wasm32 target missing");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("loft_pln24_arce_e2e_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).expect("create per-test dir");
    let used = tmp.join("used.loft");
    let unused = tmp.join("unused.loft");
    fa::write(
        &used,
        "fn c_len(s: text) -> integer;  #c \"strlen\" \"size_t(const char*)\"\n\
         fn main() { println(\"len {c_len(\"hello\")}\") }\n",
    )
    .expect("write used");
    fa::write(
        &unused,
        "fn c_len(s: text) -> integer;  #c \"strlen\" \"size_t(const char*)\"\n\
         fn main() { println(\"no c call\") }\n",
    )
    .expect("write unused");

    let build = |flag: &str, out: &str, src: &std::path::Path| -> (bool, String) {
        let o = loft::platform::process::harness_command(&loft_bin)
            .args([flag, tmp.join(out).to_str().unwrap()])
            .arg(src)
            .output()
            .expect("invoke loft");
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };

    for (flag, artifact, target) in [
        ("--html", "r.html", "browser"),
        ("--native-wasm", "r.wasm", "wasm (wasip2)"),
    ] {
        if flag == "--native-wasm" && !rustup_target_installed("wasm32-wasip2") {
            eprintln!("SKIP {flag}: rustup target wasm32-wasip2 not installed");
            continue;
        }
        let (ok, err) = build(flag, artifact, &used);
        assert!(!ok, "{flag} must refuse a reachable #c call: {err}");
        assert!(
            err.contains("`c_len` is bound to the C symbol 'strlen' with #c")
                && err.contains(target)
                && err.contains("drop the"),
            "{flag} must name the function, the symbol and the target: {err}"
        );
        // One message, not a cascade — the failure this replaced emitted one
        // rustc error per bound symbol, plus a `note: the item is gated behind
        // the native-extensions feature` pointing into loft's own source.
        // Counted on the `error:` LINE rather than the message text, because
        // rustc echoes the offending source line underneath and the literal
        // therefore appears twice in a single-error build.
        assert_eq!(
            err.matches("error: loft: `c_len`").count(),
            1,
            "exactly one refusal: {err}"
        );
        assert!(
            !err.contains("native-extensions"),
            "the author must never be shown loft's own feature gates: {err}"
        );

        // The same declaration, never called, still builds.  An unused binding
        // must not reject an otherwise-valid wasm program (the shape @PLN26 /
        // P269 already settled for `#native`).
        let (ok2, err2) = build(flag, &format!("un_{artifact}"), &unused);
        assert!(ok2, "{flag} must build an UNUSED #c declaration: {err2}");
    }
    let _ = fa::remove_dir_all(&tmp);
}

/// loft#851 — the page filesystem's own unit tests (`tools/loft_fs_unit.mjs`),
/// driven from here so they run in the ordinary suite.
///
/// `tests/wasm/*.test.mjs` is the older home for JS unit tests and nothing runs
/// those — a green that means nothing.  The two properties this covers are the
/// ones the end-to-end tests above CANNOT reach: the node harness has no
/// localStorage and no bundled base tree, so "the delta survives a reload" and
/// "a base tree is read-only" are invisible from a `--html` page under node.
#[test]
fn html_page_filesystem_unit_checks() {
    if which("node").is_none() {
        eprintln!("SKIP: node not installed");
        return;
    }
    let out = loft::platform::process::harness_command("node")
        .arg(repo_root().join("tools/loft_fs_unit.mjs"))
        .current_dir(repo_root())
        .output()
        .expect("invoke the loft-fs unit harness");
    assert!(
        out.status.success(),
        "doc/loft-fs.js unit checks failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

// ── loft#950 — a page that faults must say what the fault WAS ────────────────────────

/// A Rust panic inside a `--html` page reaches the page, with the loft frames under it.
///
/// On `wasm32-unknown-unknown` a panic aborts, and abort compiles to the `unreachable`
/// instruction, so the page's whole symptom was `RuntimeError: unreachable` — std's
/// message went to that target's stderr, which is a sink. loft#950 was reported as an
/// undebuggable trap for exactly that reason: the consumer could see where in their own
/// transcript the page died and nothing at all about why.
///
/// Asserted here is what a person acts on: the fault's own message, and the loft call
/// chain it happened under. The Rust `file:line` is deliberately not asserted — it names
/// generated code, whose line numbers move with the emitter.
///
/// loft#1056 moved the mechanism under this cell without changing what it promises. A
/// failed `assert` is no longer a Rust panic caught by the hook; it is
/// `RuntimeError::report_and_exit`, which renders the loft diagnostic and reads the same
/// frames off the shadow call stack the hook used. The frames were the reason the first
/// attempt at that swap was reverted, so the cell now also pins the diagnostic — a page
/// that fell back to a bare Rust panic would still carry the frames and must not pass.
#[test]
fn html_panic_names_itself_and_its_loft_frames() {
    let src = "fn inner950(n: integer) -> integer {
    assert(n < 5, \"distinctive950 text\");
    n
}
fn middle950(n: integer) -> integer { inner950(n + 1) }
fn main() {
    println(\"before950\");
    x = middle950(9);
    println(\"after {x}\");
}
";
    let Some((stdout, stderr, _ok)) = run_html_wasm("p950_panic", src) else {
        return;
    };
    let all = format!("{stdout}{stderr}");
    // The program really reached the fault — without this the cell would pass on a page
    // that failed to build or trapped during init, which says nothing about the hook.
    assert!(
        all.contains("before950"),
        "the page must run up to the fault\n{all}"
    );
    assert!(
        all.contains("distinctive950 text"),
        "the panic message must reach the page, not vanish into wasm stderr\n{all}"
    );
    for frame in ["inner950", "middle950", "main"] {
        assert!(
            all.contains(frame),
            "the loft frame `{frame}` must appear under the panic — a Rust location \
             alone does not say what the program was doing\n{all}"
        );
    }
    assert!(
        all.contains("error: assertion failed: distinctive950 text"),
        "a failed assert renders as the loft diagnostic, the same on every backend \
         (loft#1056)\n{all}"
    );
    assert!(
        !all.contains("panicked at"),
        "…and not as a Rust panic naming generated code\n{all}"
    );
}

/// A loft `panic(...)` renders its normal diagnostic into the page too.
///
/// This is the other half: `RuntimeError::report_and_exit` printed through `eprint!`,
/// which is the same sink. So a page that faulted the way loft INTENDS also said nothing.
#[test]
fn html_loft_panic_renders_its_diagnostic() {
    let src = "fn main() {
    println(\"before950b\");
    panic(\"distinctive950b message\");
}
";
    let Some((stdout, stderr, _ok)) = run_html_wasm("p950_userpanic", src) else {
        return;
    };
    let all = format!("{stdout}{stderr}");
    assert!(
        all.contains("before950b"),
        "the page must run up to the fault\n{all}"
    );
    assert!(
        all.contains("distinctive950b message"),
        "a loft `panic` must render into the page\n{all}"
    );
    assert!(
        all.contains("p950_userpanic.loft"),
        "and name the loft source it came from\n{all}"
    );
}

/// The control: a page that does NOT fault says nothing about panics.
///
/// Without it, a hook that printed on every run — or a harness that reported the same
/// text whatever happened — would pass both cells above.
#[test]
fn html_a_clean_page_reports_no_panic() {
    let Some((stdout, stderr, ok)) =
        run_html_wasm("p950_clean", "fn main() { println(\"clean950\"); }\n")
    else {
        return;
    };
    let all = format!("{stdout}{stderr}");
    assert!(ok, "a clean page must not trap\n{all}");
    assert!(all.contains("clean950"), "the program must run\n{all}");
    assert!(
        !all.contains("panicked at") && !all.contains("TRAP"),
        "a clean page must report no panic\n{all}"
    );
}

/// The wasm module's shadow-stack top (`__stack_pointer`'s initial value) and the linear
/// address of the asyncify canary, decoded straight from the artefact.
///
/// A second, independent walk for the reason `wasm_function_names` gives: the assertion
/// below is about what the emitted module says, and a shared decoder would let one bug
/// stand on both sides of it.  Returns `None` when either cannot be found.
fn wasm_stack_top_and_canary(wasm: &[u8]) -> (Option<u32>, Option<u32>) {
    fn uleb(b: &[u8], p: &mut usize) -> Option<u64> {
        let (mut v, mut shift) = (0u64, 0u32);
        loop {
            let byte = *b.get(*p)?;
            *p += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }
    // `i32.const <sleb> end` — the only initialiser shape a data offset or a
    // pointer-valued global uses.
    fn i32_const(b: &[u8], p: &mut usize) -> Option<i64> {
        if *b.get(*p)? != 0x41 {
            return None;
        }
        *p += 1;
        let (mut v, mut shift) = (0i64, 0u32);
        loop {
            let byte = *b.get(*p)?;
            *p += 1;
            v |= i64::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    v |= -(1i64 << shift);
                }
                return Some(v);
            }
        }
    }
    const CANARY: &[u8] = b"LOFT950!";
    let mut stack_top = None;
    let mut canary_at = None;
    let mut p = 8usize;
    while p < wasm.len() {
        let Some(&id) = wasm.get(p) else { break };
        p += 1;
        let Some(size) = uleb(wasm, &mut p).and_then(|s| usize::try_from(s).ok()) else {
            break;
        };
        let (start, Some(end)) = (p, p.checked_add(size).filter(|e| *e <= wasm.len())) else {
            break;
        };
        // Global section: the FIRST i32 mutable global a Rust wasm module declares is
        // `__stack_pointer`, initialised to the top of the shadow stack.
        if id == 6 && stack_top.is_none() {
            let mut q = start;
            if uleb(wasm, &mut q).is_some_and(|n| n > 0) {
                q += 2; // value type + mutability
                if let Some(v) = i32_const(wasm, &mut q) {
                    stack_top = u32::try_from(v).ok();
                }
            }
        }
        // Data section: find the segment holding the canary and turn its index within
        // that segment into a linear address.
        if id == 11 && canary_at.is_none() {
            let mut q = start;
            let count = uleb(wasm, &mut q).unwrap_or(0);
            for _ in 0..count {
                if uleb(wasm, &mut q) != Some(0) {
                    break; // only the active-at-a-constant form is emitted here
                }
                let Some(off) = i32_const(wasm, &mut q) else {
                    break;
                };
                if wasm.get(q) != Some(&0x0b) {
                    break;
                }
                q += 1;
                let Some(len) = uleb(wasm, &mut q).and_then(|l| usize::try_from(l).ok()) else {
                    break;
                };
                let Some(body) = wasm.get(q..q.saturating_add(len)) else {
                    break;
                };
                if let Some(idx) = body.windows(CANARY.len()).position(|w| w == CANARY) {
                    canary_at = u32::try_from(off)
                        .ok()
                        .and_then(|o| u32::try_from(idx).ok().and_then(|i| o.checked_add(i)));
                }
                q += len;
            }
        }
        p = end;
    }
    (stack_top, canary_at)
}

// loft#950 — the asyncify save region must be memory the MODULE owns.
//
// `wasm-opt --asyncify` spills the whole wasm stack into a caller-chosen buffer on every
// suspend and reads it back on resume.  The JS driver used to choose that address:
// `__heap_base` when the module exported it, and the literal `65536` when it did not.  It
// never did — that export rides in `WASM_THREAD_FLAGS`, which only the THREADED `--html`
// link passes — so every ordinary page took the fallback, and `65536` is not in the heap.
// `__stack_pointer` starts at `0x100000`, so the shadow stack is `[0, 0x100000)` growing
// DOWN and the region sat 966,648 bytes into it: a page whose stack ever reached that
// depth had asyncify writing saved frames over live locals and reading live locals back
// as saved frames.  Browser-only, because no other target runs asyncify.
//
// ⚠ THE ASSERTION IS THE ADDRESS, not the presence of the exports.  A page that exports
// all three and still hands the driver an address inside the shadow stack has exactly the
// bug this is written against, and an export-name check would pass it.  The region is
// derived from the CANARY's linear address, so it is read out of the artefact the browser
// actually gets rather than from anything this build could assert about itself.
#[test]
fn the_asyncify_save_region_is_outside_the_shadow_stack() {
    let Some((stdout, stderr, ok)) = run_html_wasm(
        "asyncify_region",
        "fn main() {\n  v: vector<integer> = [];\n  for i in 0..64 { v += [i]; }\n  \
         print(\"len {len(v)}\\n\");\n}\n",
    ) else {
        return;
    };
    let all = format!("{stdout}{stderr}");
    assert!(ok && all.contains("len 64"), "the page must run\n{all}");

    let wasm = fa::read(extracted_wasm("asyncify_region")).expect("read extracted wasm");
    let (stack_top, canary_at) = wasm_stack_top_and_canary(&wasm);
    let stack_top = stack_top.expect("__stack_pointer initialiser present");
    let canary_at = canary_at.expect(
        "the asyncify canary must be in the module's data — without it the region is not \
         reserved and the driver is choosing an address again",
    );

    // The canary sits immediately after the 8-byte control struct and the save area.
    let size = 16384u32;
    let region = canary_at
        .checked_sub(size + 8)
        .expect("canary cannot precede the region it guards");

    assert!(
        region >= stack_top,
        "the asyncify save region is at 0x{region:x}, inside the shadow stack \
         [0, 0x{stack_top:x}) — a suspend will overwrite live locals (loft#950)"
    );
    // The fallback address the bug shipped with, pinned by value: a regression that
    // restores it lands here with a name rather than as a browser-only wrong answer.
    assert_ne!(
        region, 65536,
        "the region is back at the hardcoded 65536 fallback (loft#950)"
    );
}

/// loft#967 — a `loft::<module>` path the generator writes into generated Rust must be
/// reachable in the feature set the wasm runtime rlib is built with.
///
/// `WasmRuntimeShape::features()` builds that rlib `--no-default-features --features
/// random`, so anything gated behind `native-extensions` (which buys `dlopen`, and wasm
/// has none) is configured out — and the program that references it dies in `rustc`, in
/// generated code, naming loft's internals rather than anything the author wrote.
///
/// It has happened twice from two different producers. @PLN24 arc E fixed the `#c` half
/// by refusing the CALL (`Output::no_c_abi`, the only site that knows a binding is
/// reachable); the `#native` half went on emitting `loft::native_call::enter` +
/// `build_store`, so every store-allocating native — a vector return, a struct
/// `Reference` argument — could not be built for `--native-wasm` at all.
///
/// This is the always-on half of the guard. `pln26_phase3_native_package_runs_on_wasm`
/// proves the real thing end to end, but it self-skips without wasmtime and the wasip2
/// target, which the per-PR CI leg does not install — so on the path where a regression
/// is cheapest to catch, it reports nothing.
///
/// A gate is read at TWO sites, because the two producers were gated differently: the
/// declaration in `src/lib.rs` (how `native_call` was hidden) and a leading inner
/// `#![cfg(…)]` in the module's own file (how `c_call` is).
#[test]
fn generated_loft_paths_survive_the_wasm_feature_set() {
    let root = repo_root();
    // `c_call` is the ONE exception, and it is exempt because a call to it cannot be
    // emitted for wasm: `no_c_abi()` refuses the `#c` binding first.  Asserted below, so
    // the exemption cannot outlive its enforcement.
    const REFUSED_BEFORE_EMISSION: &[&str] = &["c_call"];

    let mut sources = vec![root.join("src/codegen_runtime.rs")];
    let gen_dir = root.join("src/generation");
    let mut entries: Vec<PathBuf> = fa::read_dir(&gen_dir)
        .expect("read src/generation")
        .into_iter()
        .map(|e| e.os_spelling())
        .filter(|p| fa::has_extension(p, "rs"))
        .collect();
    entries.sort();
    sources.extend(entries);

    // Every `loft::<ident>` in the generator.  A leading `.` means a loft FILENAME in a
    // comment (`prog.loft::main`), not a Rust path.
    let mut referenced: Vec<String> = Vec::new();
    for src in &sources {
        let text = fa::read_to_string(src).expect("read generator source");
        let bytes = text.as_bytes();
        for (at, _) in text.match_indices("loft::") {
            if at > 0 && (bytes[at - 1] == b'.' || bytes[at - 1].is_ascii_alphanumeric()) {
                continue; // `prog.loft::x`, or a longer identifier ending in `loft`
            }
            let rest = &text[at + "loft::".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            if !name.is_empty() && !referenced.contains(&name) {
                referenced.push(name);
            }
        }
    }
    assert!(
        referenced.iter().any(|m| m == "native_call"),
        "the scan found no `loft::native_call` at all — it is the path loft#967 was \
         about, so an empty scan means this test stopped reading the generator"
    );

    let lib_rs = fa::read_to_string(root.join("src/lib.rs")).expect("read src/lib.rs");
    let lines: Vec<&str> = lib_rs.lines().collect();
    let mut gated: Vec<String> = Vec::new();
    for name in &referenced {
        if REFUSED_BEFORE_EMISSION.contains(&name.as_str()) {
            continue;
        }
        let decl_mod = format!("pub mod {name};");
        let decl_use = format!("::{name};");
        let Some(at) = lines.iter().position(|l| {
            let t = l.trim();
            t == decl_mod || (t.starts_with("pub use ") && t.ends_with(&decl_use))
        }) else {
            continue; // not a crate-root item — an inherent path, nothing to gate
        };
        // Attributes sit directly above the declaration, interleaved with comments.
        let mut j = at;
        while j > 0 {
            let t = lines[j - 1].trim();
            if t.starts_with("#[") {
                if t.contains("cfg(feature") {
                    gated.push(format!("{name}: src/lib.rs:{} {t}", j));
                }
            } else if !t.starts_with("//") {
                break;
            }
            j -= 1;
        }
        // …and the module's own file can gate itself with an inner attribute, which the
        // declaration above says nothing about.
        for cand in [format!("src/{name}.rs"), format!("src/{name}/mod.rs")] {
            let Ok(body) = fa::read_to_string(root.join(&cand)) else {
                continue;
            };
            if let Some(l) = body
                .lines()
                .take(40)
                .find(|l| l.trim_start().starts_with("#![cfg(feature"))
            {
                gated.push(format!("{name}: {cand} {}", l.trim()));
            }
        }
    }
    assert!(
        gated.is_empty(),
        "generated code emits these `loft::` paths, but they are behind a cargo feature \
         the wasm runtime rlib does not enable (it builds `--no-default-features \
         --features random`), so `--native-wasm` / `--html` fails in rustc inside \
         generated code (loft#967):\n  {}\n\nEither ungate the item, or refuse the \
         construct that emits it before emission — the way `no_c_abi()` refuses a `#c` \
         binding — and add it to REFUSED_BEFORE_EMISSION.",
        gated.join("\n  ")
    );

    // The exemption's enforcement, pinned: `c_call` is safe to skip only while a
    // reachable `#c` binding is REFUSED on a wasm target.
    let gen_mod =
        fa::read_to_string(root.join("src/generation/mod.rs")).expect("read generation/mod.rs");
    assert!(
        gen_mod.contains("if self.no_c_abi() {"),
        "`c_call` is exempt from the gate above because `no_c_abi()` refuses a reachable \
         `#c` binding before any call can be emitted — that refusal is gone, so the \
         exemption is now unbacked (loft#967, @PLN24 arc E)"
    );
}

// ── loft#1059 — a page whose stack runs out must say so ──────────────────────────────

/// A TRAP reaches the page, which a panic hook cannot do for it.
///
/// The hook loft#950 installs renders a Rust PANIC. Deep recursion is not one: the
/// engine's stack runs out and the module traps, which never enters the hook — so the
/// page's only symptom was a thrown `RuntimeError`, printed bare by one shell and lost
/// entirely by the other, which had no `catch` at all.
///
/// Asserted here is the reporter the page actually ships, RUN — extracted from the
/// emitted HTML and driven with the exception an engine really throws. Checking that the
/// text appears in the file would pass on a handler that is never wired up, and checking
/// only that it is wired would pass on one that reports nothing useful.
///
/// The bound itself is not loft's to raise: it belongs to the engine running the module
/// ([WASM.md § How deep a program can recurse]), so what the page owes a reader is to
/// name it rather than to move it.
#[test]
fn html_a_trap_is_reported_to_the_page() {
    if which("node").is_none() {
        eprintln!("SKIP: node not installed");
        return;
    }
    if !wasm32_target_installed() {
        eprintln!("SKIP: rustup target wasm32-unknown-unknown not installed");
        return;
    }
    let root = repo_root();
    let loft = root.join("target/release/loft");
    if !fa::exists(&loft) {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }

    let dir = std::env::temp_dir().join("loft_html_1059");
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create tempdir");
    let src = dir.join("rec1059.loft");
    let html = dir.join("rec1059.html");
    // The issue's own program: deep enough that a default engine stack dies first.
    fa::write(
        &src,
        "fn rec1059(n: integer) -> integer { if n <= 0 { return 0 } return rec1059(n - 1) + 1 }\n\
         fn main() { print(\"depth={rec1059(8000)}\\n\") }\n",
    )
    .expect("write source");
    let out = loft::platform::process::harness_command(&loft)
        .args(["--html", html.to_str().unwrap()])
        .arg(&src)
        .output()
        .expect("invoke loft --html");
    assert!(
        out.status.success(),
        "loft --html failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let page = fa::read_to_string(&html).expect("read emitted page");

    // Wired, not merely present: a handler the boot path never reaches reports nothing.
    assert!(
        page.contains(".catch(loftReportTrap)"),
        "the page's boot path does not route a trap to the reporter"
    );

    // Now RUN the reporter the page ships, against what an engine actually throws.
    let start = page
        .find("function loftReportTrap(e){")
        .expect("the page does not define loftReportTrap");
    let end = page[start..]
        .find("\n}\n")
        .expect("cannot delimit loftReportTrap")
        + start
        + 3;
    let probe = dir.join("trap.mjs");
    fa::write(
        &probe,
        format!(
            "globalThis.document={{getElementById:()=>null}};\n\
             {}\n\
             const e=new RangeError('call stack exhausted');\n\
             e.stack='at wasm://wasm/0168beca:wasm-function[1073]:0x56a035';\n\
             loftReportTrap(e);\n",
            &page[start..end]
        ),
    )
    .expect("write probe");
    let run = loft::platform::process::harness_command("node")
        .arg(&probe)
        .output()
        .expect("run the reporter under node");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        said.contains("call stack exhausted"),
        "the trap's own message must reach the page: {said}"
    );
    assert!(
        said.contains("belongs to this wasm engine"),
        "the report must name whose bound this is — the reader's next move is to raise \
         it on the host, not to change the program: {said}"
    );
    assert!(
        said.contains("wasm-function[1073]"),
        "the engine's own backtrace is the evidence and must survive: {said}"
    );
    assert!(
        said.contains("--names"),
        "…and it is unreadable without the flag that resolves those numbers: {said}"
    );

    // The control: an ordinary exception is reported WITHOUT the stack-exhaustion
    // explanation, so the cell is not passing on a reporter that says it every time.
    let probe2 = dir.join("trap2.mjs");
    fa::write(
        &probe2,
        format!(
            "globalThis.document={{getElementById:()=>null}};\n\
             {}\n\
             loftReportTrap(new Error('something else entirely'));\n",
            &page[start..end]
        ),
    )
    .expect("write control probe");
    let run2 = loft::platform::process::harness_command("node")
        .arg(&probe2)
        .output()
        .expect("run the control under node");
    let said2 = format!(
        "{}{}",
        String::from_utf8_lossy(&run2.stdout),
        String::from_utf8_lossy(&run2.stderr)
    );
    assert!(
        said2.contains("something else entirely") && !said2.contains("belongs to this wasm engine"),
        "an unrelated exception must not be explained as an exhausted stack: {said2}"
    );
}

/// A browser build never loads a package's HOST native library — the page answers its imports
/// — so `loft --html` must not build or even resolve one: neither the package's own cdylib nor
/// a default-native one (`extensions::browser_target`).  Both used to run a full cargo build of
/// code the page never links, and a stale host rlib aborted the browser build outright.
/// `LOFT_TIMING` prints one `cdylib` line per host library resolved, hit or built; before the
/// cut, this program printed two.
#[test]
fn a_browser_build_builds_no_host_native_library() {
    if !wasm32_target_installed() {
        eprintln!("SKIP: rustup target wasm32-unknown-unknown not installed");
        return;
    }
    let root = repo_root();
    let loft_bin = root.join("target/release/loft");
    if !fa::exists(&loft_bin) {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release` first)");
        return;
    }
    let tmp = std::env::temp_dir().join("loft_html_no_host_native");
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).expect("create per-test dir");
    let src = tmp.join("uses_random.loft");
    let html = tmp.join("uses_random.html");
    fa::write(&src, "use random;\nfn main() {\n  println(\"ok\");\n}\n").expect("write source");
    let out = loft::platform::process::harness_command(&loft_bin)
        .current_dir(&root)
        .env("LOFT_TIMING", "1")
        .env("LOFT_NO_CACHE", "1")
        .arg("--html")
        .arg(&html)
        .args(["--lib", "tests/fixtures/libs"])
        .arg(&src)
        .output()
        .expect("run loft --html");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "loft --html failed:\n{log}");
    let host: Vec<&str> = log.lines().filter(|l| l.contains("cdylib")).collect();
    assert!(
        host.is_empty(),
        "a browser build resolved a host native library:\n{}",
        host.join("\n")
    );
    let _ = fa::remove_dir_all(&tmp);
}
