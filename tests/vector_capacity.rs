// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A vector sized by a number from the program's input stops the program when its bytes do
//! not fit 32 bits, on both backends, with the same words.
//!
//! `[7; n]` for `n` from the environment: 3 elements read back `len 3 last 7`; 3,000,000,000
//! elements of 8 bytes are 24 GB, past what a store's 32-bit arithmetic holds, and the run
//! fails saying `Vector capacity overflow: count=3000000000 size=8`.  Before the check the
//! compiled build took the wrapped size as "already large enough", set the length and filled
//! from a record of a few words; the interpreter's arithmetic wrapped in a release build.

use loft::file_access as fa;

const PROGRAM: &str = r#"fn main() {
  n = ((env_variable("N") ?? "3") as integer?) ?? 3;
  v = [7; n];
  println("len {len(v)} last {v[n - 1]}");
}
"#;

#[test]
fn a_vector_too_large_for_a_store_stops_the_program() {
    let dir = std::env::temp_dir().join(format!("loft_vector_capacity_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("fill.loft"), PROGRAM).expect("program");
    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        let run = |n: &str| {
            let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
                .args([backend, "--no-warnings", "fill.loft"])
                .current_dir(&dir)
                .env("N", n)
                .env("LOFT_TIMEOUT", "300")
                .output()
                .expect("loft runs");
            (
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
                String::from_utf8_lossy(&out.stderr).to_string(),
            )
        };
        let (ok, out, err) = run("3");
        assert!(
            ok && out == "len 3 last 7",
            "{backend}: three elements\n{out}\n{err}"
        );
        let (ok, out, err) = run("3000000000");
        assert!(
            !ok && out.is_empty(),
            "{backend}: the fill does not run\n{out}\n{err}"
        );
        assert!(
            err.contains("Vector capacity overflow: count=3000000000 size=8"),
            "{backend}: and says why\n{err}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
}
