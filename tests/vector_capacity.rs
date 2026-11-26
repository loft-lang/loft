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
//!
//! @falsified-at: 4e992f30d — by hand: `vector_set_size`'s checked sum and product made wrapping
//! → the interpreter's leg ran on to `Store access out of bounds`; the C's capacity check
//! clamped, its sum unchecked and its resize read as signed → the C leg died of SIGBUS.

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

const READ: &str = r#"fn main() {
  b = read_bytes(env_variable("F") ?? "");
  println("read {len(b)}");
}
"#;

/// `read_bytes` of a file with more bytes than a vector counts (4 GiB and 10 bytes, one past
/// 32 bits' reach by 11) stops the program with the capacity message, count held at the
/// largest, on both backends — and the file is refused by its size, before a byte of it is
/// read, so the run holds no 4 GiB copy.  A file of 10 bytes beside it reads back `read 10`.
/// The large file is sparse: no disk is spent on it, and a file system that cannot make one
/// skips the test, saying so.
///
/// @falsified-at: 4e992f30d — by hand: the size asked first and the count held at the largest,
/// both taken out → the interpreter failed on a slice bound (`range end index 4294967306`);
/// the same two in the C → the C leg died of SIGBUS copying the file into a 10-byte record.
#[test]
fn a_file_with_more_bytes_than_a_vector_counts_stops_read_bytes() {
    let dir = std::env::temp_dir().join(format!("loft_read_bytes_capacity_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("read.loft"), READ).expect("program");
    fa::write(dir.join("small.bin"), b"0123456789").expect("small file");
    let big = dir.join("big.bin");
    fa::create(&big)
        .and_then(|f| f.set_len((1u64 << 32) + 10))
        .expect("the large file");
    if !sparse(&big) {
        let _ = fa::remove_dir_all(&dir);
        eprintln!("skipped: this file system gives a 4 GiB file its blocks, so no test makes one");
        return;
    }
    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        let run = |file: &str| {
            let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
                .args([backend, "--no-warnings", "read.loft"])
                .current_dir(&dir)
                .env("F", file)
                .env("LOFT_TIMEOUT", "300")
                .output()
                .expect("loft runs");
            (
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
                String::from_utf8_lossy(&out.stderr).to_string(),
            )
        };
        let (ok, out, err) = run("small.bin");
        assert!(ok && out == "read 10", "{backend}: ten bytes\n{out}\n{err}");
        let (ok, out, err) = run("big.bin");
        assert!(
            !ok && out.is_empty(),
            "{backend}: the read does not complete\n{out}\n{err}"
        );
        assert!(
            err.contains("Vector capacity overflow: count=4294967295 size=1"),
            "{backend}: and says why\n{err}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
}

/// Does `file` hold far fewer blocks than its length — is it sparse?
#[cfg(unix)]
fn sparse(file: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    fa::metadata(file).is_ok_and(|m| m.blocks() * 512 < (1 << 20))
}

#[cfg(not(unix))]
fn sparse(_file: &std::path::Path) -> bool {
    false
}
