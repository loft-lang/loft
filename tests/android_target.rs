// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN106 B1+B2 — the `--native-android` build target (src/android.rs).
//
// The design's invariant (doc/claude/plans/106-android-build-target/README.md): a
// loft build target is a *descriptor* over ONE target-agnostic generated-Rust core,
// so the same source `--native` compiles for the host cross-compiles to a bionic
// AArch64 `.so` with only toolchain/entry/packaging differences — no Android codegen
// path. This checks the loft-driven pipeline end to end: emit → wrap in a generated
// NativeActivity crate → cross-build a cdylib that exports `android_main` +
// `ANativeActivity_onCreate` (the B2 runtime entry).
//
// The build test needs the Android NDK AND a working cross toolchain, so it is a
// SEPARATE, opt-in leg gated on `LOFT_ANDROID_BUILD_TEST` (like the daily Windows
// leg, never a required check).  NOTE: it can NOT gate on `ANDROID_NDK_HOME`
// presence — the GitHub-hosted ubuntu/macos images ship an NDK and set that var,
// so a presence-gate would wrongly run the heavy build in required CI (where the
// cross-build fails).  A dedicated opt-in flag keeps it a manual, local leg.  The
// error-path test always runs — it is cheap and guards that the flag exists and
// fails with an actionable message.

use loft::file_access as fa;
use std::io::Write;
use std::process::Command;

fn write_probe(name: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_android_target");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(name);
    fa::create(&path)
        .expect("create probe")
        .write_all(src.as_bytes())
        .expect("write probe");
    path
}

/// True if `path` is an ELF64 shared object for AArch64 (`e_machine == 183`).
/// Reads only the header, so it needs no `readelf`/NDK tool on the test host.
fn is_aarch64_elf(path: &std::path::Path) -> bool {
    let Ok(bytes) = fa::read(path) else {
        return false;
    };
    // ELF magic + 64-bit class + little-endian, then e_machine (offset 18, u16 LE).
    bytes.len() > 19
        && &bytes[0..4] == b"\x7fELF"
        && bytes[4] == 2 // ELFCLASS64
        && bytes[5] == 1 // ELFDATA2LSB
        && u16::from_le_bytes([bytes[18], bytes[19]]) == 183 // EM_AARCH64
}

/// True if the `.so` contains `sym` in its (dynamic) symbol string table. A raw
/// byte scan — enough to confirm an exported entry without an NDK `readelf` on the
/// test host (the name lives verbatim in `.dynstr`).
fn so_names_symbol(path: &std::path::Path, sym: &str) -> bool {
    fa::read(path).is_ok_and(|bytes| bytes.windows(sym.len()).any(|w| w == sym.as_bytes()))
}

/// `--native-android` on a non-trivial program (struct + fn + for-loop + string
/// interpolation) produces a real AArch64 `NativeActivity` shared object — a valid
/// AArch64 ELF that exports the `android_main` + `ANativeActivity_onCreate` entry
/// (B2). Skipped unless `LOFT_ANDROID_BUILD_TEST` is set, so a normal `cargo test`
/// run (including CI images that happen to ship an NDK) never triggers the build.
#[test]
fn android_native_so_is_aarch64_elf() {
    // Opt-in ONLY on a dedicated flag — NOT on ambient NDK vars, which the
    // GitHub-hosted runners set (see the module header).
    if std::env::var_os("LOFT_ANDROID_BUILD_TEST").is_none() {
        eprintln!(
            "skipping android build leg: set LOFT_ANDROID_BUILD_TEST=1 (needs an NDK + cross toolchain)"
        );
        return;
    }
    if std::env::var_os("ANDROID_NDK_HOME").is_none()
        && std::env::var_os("ANDROID_NDK_ROOT").is_none()
    {
        eprintln!("skipping: LOFT_ANDROID_BUILD_TEST set but no ANDROID_NDK_HOME/ANDROID_NDK_ROOT");
        return;
    }
    let src = r#"
struct Point { x: integer, y: integer }
fn dist2(p: Point) -> integer { p.x * p.x + p.y * p.y }
fn main() {
  total = 0;
  for i in 0..5 {
    p = Point { x: i, y: i + 1 };
    total = total + dist2(p);
  }
  print("total={total}\n");
}
"#;
    let probe = write_probe("prog.loft", src);
    let out_so = std::env::temp_dir()
        .join("loft_android_target")
        .join("prog.so");
    let _ = fa::remove_file(&out_so);
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native-android")
        .arg(&out_so)
        .arg(&probe)
        // The cargo cross-build of android-activity + loft can take a minute cold.
        .env("LOFT_TIMEOUT", "600")
        .output()
        .expect("spawn loft");
    assert!(
        out.status.success(),
        "--native-android failed:\nstdout:{}\nstderr:{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        is_aarch64_elf(&out_so),
        "output {} is not an AArch64 ELF shared object\nstderr:{}",
        out_so.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    // The B2 runtime entry: the .so is a NativeActivity, not a bare cdylib.
    for sym in ["android_main", "ANativeActivity_onCreate"] {
        assert!(
            so_names_symbol(&out_so, sym),
            "output {} does not export `{sym}` (not a NativeActivity)",
            out_so.display()
        );
    }
}

/// Without an NDK, `--native-android` fails fast with a message that names the env
/// var to set — not an opaque `rustc`/`cc` error deep in the build.
#[test]
fn android_without_ndk_reports_actionable_error() {
    let probe = write_probe("noNdk.loft", "fn main() { print(\"hi\\n\"); }");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native-android")
        .arg(&probe)
        .env_remove("ANDROID_NDK_HOME")
        .env_remove("ANDROID_NDK_ROOT")
        .output()
        .expect("spawn loft");
    assert!(
        !out.status.success(),
        "expected --native-android to fail without an NDK"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ANDROID_NDK_HOME"),
        "error should name ANDROID_NDK_HOME; got:\n{stderr}"
    );
}
