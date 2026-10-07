// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 G2 / M0 — the read-site migration's equivalence harness, end-to-end.
//!
//! `LOFT_IR_CHECK` materialises the parsed program into a store, reads it back,
//! and asserts the round-trip is bit-for-bit identical to the native `Data`
//! (`ir_roundtrip_check`) before continuing to bytecode.  This guards the
//! store-mirror invariant on a **real program** — user struct + fn + control
//! flow, not just the stdlib round-trip unit tests — and is the bedrock every
//! later representation swap is verified against.

use loft::file_access as fa;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

#[test]
fn ir_check_passes_on_real_program() {
    let pid = std::process::id();
    let script = std::env::temp_dir().join(format!("loft_g2irc_{pid}.loft"));
    fa::write(
        &script,
        "struct Point { x: integer, y: integer }\n\
         fn dist2(p: Point) -> integer { p.x * p.x + p.y * p.y }\n\
         fn main() {\n\
         \x20 pts = [Point{x:3, y:4}, Point{x:5, y:12}];\n\
         \x20 for p in pts { print(\"d2={dist2(p)}\\n\"); }\n\
         }\n",
    )
    .expect("write script");

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(&script)
        .env("LOFT_IR_CHECK", "1")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("invoke loft");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "LOFT_IR_CHECK run must exit 0 (store mirror == native); stderr:\n{stderr}"
    );
    // The divergence path prints this and exits 1 — its absence confirms the check ran clean.
    assert!(
        !stderr.contains("diverges from native"),
        "store-backed IR diverged: {stderr}"
    );
    assert!(
        stdout.contains("d2=25") && stdout.contains("d2=169"),
        "output: {stdout}"
    );

    let _ = fa::remove_file(&script);
}
