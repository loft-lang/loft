// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// The cache cells run the C build (`--native`), whose binary cache keeps the same defenses.

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

/// The run executed a cached binary: the source-keyed fast path answered, or the binary cache
/// did (the C build's `[loft-timing] binary-c <name> cache=hit` line).
fn served_from_cache(stderr: &str) -> bool {
    stderr.contains("native_source_key=hit")
        || stderr.contains("native_binary_cache=hit")
        || stderr
            .lines()
            .any(|l| l.contains("binary-c ") && l.contains("cache=hit"))
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
        served_from_cache(&stderr),
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

/// A run that asks to keep the symbols (`LOFT_NATIVE_KEEP_SYMBOLS=1`, how a native profile
/// gets readable frames) is never served a stripped binary.  The C build strips nothing, so a
/// plain run's binary already carries its symbols and both runs share it: one binary, with a
/// symbol table (read where the format is ELF).
#[test]
fn a_symbolised_build_is_not_served_the_stripped_one() {
    let dir = tmp_subdir("keep_symbols");
    let marker = "P254_KEEP_SYMBOLS";
    let script = write_marker_script(&dir, marker);
    let run = |keep: bool| {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg("--native").arg(&script).env("LOFT_TIMING", "1");
        if keep {
            cmd.env("LOFT_NATIVE_KEEP_SYMBOLS", "1");
        } else {
            cmd.env_remove("LOFT_NATIVE_KEEP_SYMBOLS");
        }
        cmd.output().expect("run loft --native")
    };
    let plain = run(false);
    assert!(String::from_utf8_lossy(&plain.stdout).contains(marker));
    let kept = run(true);
    assert!(String::from_utf8_lossy(&kept.stdout).contains(marker));
    let binaries: Vec<_> = fa::read_dir(cache_dir_for(&script))
        .expect("read cache dir")
        .into_iter()
        .filter(|e| e.os_spelling().to_string_lossy().contains("native-"))
        .collect();
    assert_eq!(
        binaries.len(),
        1,
        "one binary serves both runs: {binaries:?}"
    );
    let bytes = fa::read(binaries[0].os_spelling()).expect("the binary");
    if bytes.starts_with(b"\x7fELF") {
        assert!(
            bytes.windows(7).any(|w| w == b".symtab"),
            "the binary a symbolised run is served carries its symbol table"
        );
    }
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

// ── Where loft stages what it runs and caches, when the temp directory is shared ─────────
//
// Without `LOFT_TMPDIR`, a native build's scratch (the C source, the binary it then executes)
// and, with neither `HOME` nor `XDG_CACHE_HOME` set, the cache (the stdlib image, compiled
// binaries) live under the system temp directory.  There they go into `loft-<uid>`, a
// directory only this user can enter, and only while it is one: a real directory (not a
// link), owned by the user, with no group or other bits.  When the name is taken by anything
// else, a fresh directory is made instead.  Nothing is written directly into the temp
// directory, where a name another account can predict is a name it can take first.
//
// Each run points `TMPDIR` at a scratch directory of its own, in which `loft-<uid>` is:
// absent; an open directory (mode 0777 — another account's directory looks the same from
// here, apart from the owner a test cannot change); or a link to a private-looking directory
// elsewhere.

/// What sits at `loft-<uid>` in the temp directory before loft runs.
// approved exemption: POSIX owner and mode bits have no Windows equivalent; Windows substitute: none (the temp dir there is the user's own profile directory)
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq)]
enum Plant {
    Nothing,
    OpenDir,
    Link,
}

/// A fresh temp directory under `dir` with `plant` at `loft-<uid>`; answers that temp
/// directory, the planted name and the directory a planted link points at.
#[cfg(unix)]
fn shared_temp(dir: &std::path::Path, plant: Plant) -> (PathBuf, PathBuf, PathBuf) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let tmp = dir.join(format!("tmp_{plant:?}"));
    fa::create_dir_all(&tmp).expect("make the temp dir");
    let uid = fa::metadata(&tmp).expect("stat the temp dir").uid();
    let name = tmp.join(format!("loft-{uid}"));
    let elsewhere = dir.join(format!("elsewhere_{plant:?}"));
    fa::create_dir_all(&elsewhere).expect("make the link target");
    fa::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    match plant {
        Plant::Nothing => {}
        Plant::OpenDir => {
            fa::create_dir(&name).expect("plant the open directory");
            fa::set_permissions(&name, std::fs::Permissions::from_mode(0o777)).expect("chmod");
        }
        Plant::Link => fa::symlink(&elsewhere, &name).expect("plant the link"),
    }
    (tmp, name, elsewhere)
}

/// `path` lies in a directory directly under `tmp` that is this user's alone — a real
/// directory, owned by the user, mode 0700 — which is `loft-<uid>` itself only when nothing
/// was planted there; the planted directory and the link's target stay empty, and every
/// other entry loft left in `tmp` is private the same way.
#[cfg(unix)]
fn assert_private_placement(tmp: &std::path::Path, path: &std::path::Path, plant: Plant) {
    use std::os::unix::fs::MetadataExt;
    let uid = fa::metadata(tmp).expect("stat the temp dir").uid();
    let name = tmp.join(format!("loft-{uid}"));
    let rel = path.strip_prefix(tmp).unwrap_or_else(|_| {
        panic!(
            "{plant:?}: {} is not under {}",
            path.display(),
            tmp.display()
        )
    });
    let first = tmp.join(rel.components().next().expect("a path below the temp dir"));
    assert!(
        rel.components().count() > 1,
        "{plant:?}: {} lies directly in the temp dir",
        path.display()
    );
    if plant == Plant::Nothing {
        assert_eq!(
            first, name,
            "{plant:?}: the user's own directory is loft-<uid>"
        );
    } else {
        assert_ne!(first, name, "{plant:?}: the planted loft-<uid> is not used");
    }
    for entry in fa::read_dir(tmp).expect("list the temp dir") {
        let entry = entry.os_spelling();
        if entry == name && plant != Plant::Nothing {
            if plant == Plant::OpenDir {
                assert!(
                    fa::read_dir(&entry).expect("list the plant").is_empty(),
                    "{plant:?}: nothing is written into the planted directory"
                );
            }
            continue;
        }
        let md = fa::symlink_metadata(&entry).expect("stat an entry");
        assert!(
            md.is_dir() && md.uid() == uid && md.mode() & 0o777 == 0o700,
            "{plant:?}: {} in the temp dir is not a directory private to the user (mode {:o})",
            entry.display(),
            md.mode() & 0o7777
        );
    }
    let elsewhere = tmp.parent().unwrap().join(format!("elsewhere_{plant:?}"));
    assert!(
        fa::read_dir(&elsewhere)
            .expect("list the link target")
            .is_empty(),
        "{plant:?}: nothing is written through the planted link"
    );
}

/// A native build stages its C source and binary in the user's private temp directory, and
/// never in a `loft-<uid>` it did not make private itself.  `LOFT_KEEP_NATIVE_C` keeps the
/// source and names where it was written.
// approved exemption: POSIX owner and mode bits have no Windows equivalent; Windows substitute: none (the temp dir there is the user's own profile directory)
#[cfg(unix)]
#[test]
fn native_build_stages_in_a_directory_private_to_the_user() {
    let dir = tmp_subdir("private_scratch");
    let marker = "PRIVATE_SCRATCH_RAN";
    let script = write_marker_script(&dir, marker);
    for plant in [Plant::Nothing, Plant::OpenDir, Plant::Link] {
        let (tmp, _, _) = shared_temp(&dir, plant);
        let out = loft::platform::process::harness_command(loft_bin())
            .arg("--native")
            .arg(&script)
            .env("TMPDIR", &tmp)
            .env_remove("LOFT_TMPDIR")
            .env("HOME", dir.join("home"))
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .env("LOFT_KEEP_NATIVE_C", "1")
            .env("LOFT_NATIVE_NO_CACHE", "1")
            .output()
            .expect("run loft --native");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains(marker),
            "{plant:?}: the build ran: {stderr}"
        );
        let kept = stderr
            .lines()
            .find_map(|l| l.strip_prefix("loft: C source preserved at "))
            .unwrap_or_else(|| panic!("{plant:?}: the kept source is named: {stderr}"));
        assert_private_placement(&tmp, std::path::Path::new(kept.trim()), plant);
    }
    let _ = fa::remove_dir_all(&dir);
}

/// With neither `HOME` nor `XDG_CACHE_HOME` set, the cache — `cache_dir()`, and the stdlib
/// image the run stores there — falls back to the user's private temp directory, never to
/// the temp directory itself or a `loft-<uid>` it did not make private.
// approved exemption: POSIX owner and mode bits have no Windows equivalent; Windows substitute: none (the temp dir there is the user's own profile directory)
#[cfg(unix)]
#[test]
fn cache_without_a_home_falls_back_to_a_directory_private_to_the_user() {
    let dir = tmp_subdir("private_cache");
    let script = dir.join("cache_dir.loft");
    fa::write(
        &script,
        "fn main() {\n  println(\"at {cache_dir()}\");\n}\n",
    )
    .expect("write");
    for plant in [Plant::Nothing, Plant::OpenDir, Plant::Link] {
        let (tmp, _, _) = shared_temp(&dir, plant);
        let out = loft::platform::process::harness_command(loft_bin())
            .arg("--interpret")
            .arg(&script)
            .current_dir(&dir)
            .env("TMPDIR", &tmp)
            .env_remove("LOFT_TMPDIR")
            .env_remove("HOME")
            .env_remove("XDG_CACHE_HOME")
            .env("LOFT_HOME", dir.join("loft_home"))
            .output()
            .expect("run loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let at = stdout
            .lines()
            .find_map(|l| l.strip_prefix("at "))
            .unwrap_or_else(|| {
                panic!(
                    "{plant:?}: the program printed its cache dir: {stdout}{}",
                    String::from_utf8_lossy(&out.stderr)
                )
            });
        let at = std::path::Path::new(at.trim());
        assert_eq!(
            at.file_name().and_then(|n| n.to_str()),
            Some("loft"),
            "{plant:?}"
        );
        assert_private_placement(&tmp, at, plant);
    }
    let _ = fa::remove_dir_all(&dir);
}
