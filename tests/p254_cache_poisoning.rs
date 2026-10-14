// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! P254 — `loft --native` cache-poisoning defenses.  Each test
//! drives the real `loft --native` binary against a fresh temp
//! directory, so the tests cover the on-disk cache flow exactly
//! as users hit it.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Allocate a per-test temp directory.  Caller is responsible
/// for removing it on success; on panic the system tmp cleanup
/// will eventually reap it.
fn tmp_subdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "loft_p254_{name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    fa::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Write a trivial loft script that prints a known marker so the
/// test can distinguish "loft compiled and ran the real script"
/// from "loft executed a poisoned binary".
fn write_marker_script(dir: &std::path::Path, marker: &str) -> PathBuf {
    let script = dir.join("p254.loft");
    let body = format!("fn main() {{ println(\"{marker}\"); }}\n");
    fa::write(&script, body).expect("write script");
    script
}

/// `loft --native` writes the cache binary at
/// `<script_dir>/.loft/cache/native-<hash>` — derive the directory
/// path so tests can poison or inspect it.
fn cache_dir_for(script: &std::path::Path) -> PathBuf {
    script.parent().unwrap().join(".loft").join("cache")
}

#[test]
fn first_run_creates_cache_with_safe_permissions() {
    let dir = tmp_subdir("safe_perms");
    let marker = "P254_FRESH_CACHE";
    let script = write_marker_script(&dir, marker);

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("run loft --native");
    assert!(
        out.status.success(),
        "first run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(marker),
        "first run did not print marker: stdout={}",
        String::from_utf8_lossy(&out.stdout)
    );

    let cache_dir = cache_dir_for(&script);
    assert!(fa::is_dir(&cache_dir), "cache dir missing");

    // approved exemption: POSIX owner and mode bits (0o700, group-writable) have no Windows equivalent; Windows substitute: none (the cache check there refuses a symlink only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let md = fa::metadata(&cache_dir).expect("stat cache dir");
        // P254 — directory must be 0o700 after first cache write.
        assert_eq!(md.mode() & 0o777, 0o700, "cache dir mode wrong");
        // P254 — every cached binary inside must also be 0o700.
        for entry in fa::read_dir(&cache_dir).expect("read cache dir") {
            let cmd = fa::metadata(&entry).expect("stat cache file");
            assert_eq!(
                cmd.mode() & 0o777,
                0o700,
                "cache file {} mode wrong",
                entry.os_spelling().display()
            );
        }
    }

    let _ = fa::remove_dir_all(&dir);
}

#[test]
fn second_run_reuses_safe_cache() {
    let dir = tmp_subdir("reuse_safe");
    let marker = "P254_REUSE_SAFE";
    let script = write_marker_script(&dir, marker);

    // Prime the cache.
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("first run");
    assert!(out.status.success());

    let cache_dir = cache_dir_for(&script);
    let entries: Vec<_> = fa::read_dir(&cache_dir)
        .expect("read cache dir")
        .into_iter()
        .map(|e| e.os_spelling())
        .collect();
    assert_eq!(entries.len(), 1, "expected exactly one cached binary");

    // Second run — should reuse the cached binary.  A reuse touches the entry (the cache
    // keeps its most recently used binaries), so the mtime is no longer the witness; the
    // run's own verdict is.
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&script)
        .env("LOFT_TIMING", "1")
        .output()
        .expect("second run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(marker));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("native_source_key=hit") || stderr.contains("native_binary_cache=hit"),
        "recompile happened when the cache was usable: {stderr}"
    );

    let entries2: Vec<_> = fa::read_dir(&cache_dir)
        .expect("read cache dir 2")
        .into_iter()
        .map(|e| e.os_spelling())
        .collect();
    assert_eq!(entries2.len(), 1, "expected exactly one cached binary");
    assert_eq!(
        entries[0], entries2[0],
        "cache file path changed across runs"
    );

    let _ = fa::remove_dir_all(&dir);
}

// approved exemption: POSIX owner and mode bits (0o700, group-writable) have no Windows equivalent; Windows substitute: none (the cache check there refuses a symlink only)
#[cfg(unix)]
// @speed 1.0
#[test]
fn group_writable_cache_is_recompiled() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let dir = tmp_subdir("group_writable");
    let marker = "P254_RECOMPILED";
    let script = write_marker_script(&dir, marker);

    // Prime the cache.
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("first run");
    assert!(out.status.success());

    let cache_dir = cache_dir_for(&script);
    let cached = fa::read_dir(&cache_dir)
        .expect("read cache dir")
        .into_iter()
        .next()
        .expect("at least one cache file")
        .os_spelling();

    // Loosen the cache file's mode to simulate an attacker-friendly cache
    // (group + other write).  The next run must reject it and recompile.
    fa::set_permissions(&cached, std::fs::Permissions::from_mode(0o766))
        .expect("loosen cache mode");

    let mtime_before = fa::metadata(&cached)
        .expect("stat before")
        .modified()
        .expect("mtime before");

    // Sleep so a recompile produces a strictly newer mtime.
    std::thread::sleep(std::time::Duration::from_millis(50));

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("second run");
    assert!(
        out.status.success(),
        "loft should recover by recompiling: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(marker),
        "recompiled binary should print the marker"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("rejecting suspicious cached binary"),
        "missing P254 reject warning; stderr={stderr}"
    );

    // After the recompile the cache file should be back to 0o700.
    let cached_after = fa::read_dir(&cache_dir)
        .expect("read cache dir 2")
        .into_iter()
        .next()
        .expect("at least one cache file")
        .os_spelling();
    let md_after = fa::metadata(&cached_after).expect("stat after");
    assert_eq!(
        md_after.mode() & 0o777,
        0o700,
        "cache mode should be back to 0o700 after recompile"
    );
    assert!(
        md_after.modified().expect("mtime after") > mtime_before,
        "cache mtime should be newer after recompile"
    );

    let _ = fa::remove_dir_all(&dir);
}

// approved exemption: POSIX owner and mode bits (0o700, group-writable) have no Windows equivalent; Windows substitute: none (the cache check there refuses a symlink only)
#[cfg(unix)]
// @speed 0.9
#[test]
fn poisoned_cache_binary_is_not_executed() {
    let dir = tmp_subdir("poisoned");
    let marker = "P254_REAL_BINARY";
    let script = write_marker_script(&dir, marker);

    // Prime the cache so we know the cache filename loft will look for.
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("first run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(marker));

    let cache_dir = cache_dir_for(&script);
    let cached = fa::read_dir(&cache_dir)
        .expect("read cache dir")
        .into_iter()
        .next()
        .expect("at least one cache file")
        .os_spelling();

    // Replace the cached binary with a "poisoned" shell script that
    // would print an attacker marker if loft ran it.  Make it group-
    // writable so the safety check rejects it; the recompile path
    // overwrites it before we ever execute.
    use std::os::unix::fs::PermissionsExt;
    fa::write(
        &cached,
        b"#!/bin/sh\necho P254_POISONED_BINARY_RAN\nexit 0\n",
    )
    .expect("write poison");
    fa::set_permissions(&cached, std::fs::Permissions::from_mode(0o777))
        .expect("loosen poison mode");

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&script)
        .output()
        .expect("second run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "loft should recover by recompiling: stderr={stderr}"
    );
    assert!(
        !stdout.contains("P254_POISONED_BINARY_RAN"),
        "poisoned binary was executed! stdout={stdout}"
    );
    assert!(
        stdout.contains(marker),
        "recompiled real binary should print the original marker; stdout={stdout}"
    );
    assert!(
        stderr.contains("rejecting suspicious cached binary"),
        "missing P254 reject warning; stderr={stderr}"
    );

    let _ = fa::remove_dir_all(&dir);
}

#[test]
fn no_cache_env_var_skips_cache() {
    let dir = tmp_subdir("no_cache_env");
    let marker = "P254_NO_CACHE_ENV";
    let script = write_marker_script(&dir, marker);

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native")
        .arg(&script)
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .output()
        .expect("run loft --native with LOFT_NATIVE_NO_CACHE=1");
    assert!(
        out.status.success(),
        "first run failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains(marker));

    let cache_dir = cache_dir_for(&script);
    assert!(
        !fa::exists(&cache_dir) || fa::read_dir(&cache_dir).map_or(true, |d| d.is_empty()),
        "LOFT_NATIVE_NO_CACHE=1 should not write to the cache directory"
    );

    let _ = fa::remove_dir_all(&dir);
}
