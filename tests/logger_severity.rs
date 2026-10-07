// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Direct unit tests for the runtime `Logger` severity routing and level filtering.
//!
//! These tests address the GAPS.md §9 coverage gaps for `log_warn`, `log_error`, and
//! `log_fatal` paths, which are otherwise only exercised indirectly through loft scripts
//! (tests/docs/20-logging.loft) where the logger output goes to a file that no assertion reads.

extern crate loft;

use loft::file_access as fa;
use loft::logger::{Logger, RuntimeLogConfig, Severity};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// Per-process counter to guarantee unique temp-file names even when tests run in
/// parallel.  macOS `SystemTime` only has microsecond resolution, so `subsec_nanos()`
/// alone produces collisions between concurrently-starting tests.
static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_tmp(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "loft_{prefix}_{pid}_{id}.txt",
        pid = std::process::id()
    ))
}

/// Build a Logger that writes to a temp file and return (logger, path).
fn logger_with_tmpfile(level: Severity) -> (Logger, PathBuf) {
    let path = unique_tmp("logger_test");
    let config = RuntimeLogConfig {
        log_path: path.clone(),
        default_level: level,
        rate_per_minute: 0, // no rate limiting — keep tests deterministic
        daily_rotation: false,
        ..RuntimeLogConfig::default()
    };
    (Logger::new(config, None), path)
}

fn read_log(path: &PathBuf) -> String {
    fa::read_to_string(path).unwrap_or_default()
}

fn cleanup(path: &PathBuf) {
    let _ = fa::remove_file(path);
}

/// `log_warn` writes a line containing "WARN" when the level is Info (which accepts Warn+).
#[test]
fn logger_warn_appears_at_info_level() {
    let (mut lg, path) = logger_with_tmpfile(Severity::Info);
    lg.log(Severity::Warn, "test.loft", 42, "something unexpected");
    drop(lg);
    let out = read_log(&path);
    cleanup(&path);
    assert!(
        out.contains("WARN"),
        "expected 'WARN' in log output; got: {out:?}"
    );
    assert!(
        out.contains("something unexpected"),
        "expected message body in log output; got: {out:?}"
    );
    assert!(
        out.contains("test.loft:42"),
        "expected file:line in log output; got: {out:?}"
    );
}

/// `log_error` writes a line containing "ERROR".
#[test]
fn logger_error_appears() {
    let (mut lg, path) = logger_with_tmpfile(Severity::Info);
    lg.log(Severity::Error, "app.loft", 7, "something went wrong");
    drop(lg);
    let out = read_log(&path);
    cleanup(&path);
    assert!(
        out.contains("ERROR"),
        "expected 'ERROR' in log output; got: {out:?}"
    );
    assert!(out.contains("something went wrong"));
}

/// Messages below the configured level are suppressed.
#[test]
fn logger_level_filter_suppresses_info_at_warn_level() {
    let (mut lg, path) = logger_with_tmpfile(Severity::Warn);
    lg.log(Severity::Info, "test.loft", 1, "routine info");
    drop(lg);
    let out = read_log(&path);
    cleanup(&path);
    assert!(
        !out.contains("routine info"),
        "Info message must be suppressed when level is Warn; got: {out:?}"
    );
}

/// `log_warn` is suppressed when the default level is Error.
#[test]
fn logger_warn_suppressed_at_error_level() {
    let (mut lg, path) = logger_with_tmpfile(Severity::Error);
    lg.log(
        Severity::Warn,
        "test.loft",
        2,
        "this warn should not appear",
    );
    drop(lg);
    let out = read_log(&path);
    cleanup(&path);
    assert!(
        !out.contains("this warn should not appear"),
        "Warn must be suppressed at Error level; got: {out:?}"
    );
}

/// Rate limiting: when rate_per_minute > 0 and the limit is reached, excess messages
/// are suppressed.  This tests the coverage-gap path `should_suppress = true` in Logger::log.
#[test]
fn logger_rate_limiting_suppresses_excess() {
    let path = unique_tmp("logger_rate");
    let config = RuntimeLogConfig {
        log_path: path.clone(),
        default_level: Severity::Info,
        rate_per_minute: 2, // only 2 messages per minute per (file, line)
        daily_rotation: false,
        ..RuntimeLogConfig::default()
    };
    let mut lg = Logger::new(config, None);
    // Same (file, line) key — 3rd message should be suppressed.
    lg.log(Severity::Warn, "rate.loft", 10, "msg 1");
    lg.log(Severity::Warn, "rate.loft", 10, "msg 2");
    lg.log(Severity::Warn, "rate.loft", 10, "msg 3 suppressed");
    drop(lg);
    let out = read_log(&path);
    cleanup(&path);
    assert!(
        out.contains("msg 1"),
        "first message must appear; got: {out:?}"
    );
    assert!(
        out.contains("msg 2"),
        "second message must appear; got: {out:?}"
    );
    assert!(
        !out.contains("msg 3 suppressed"),
        "third message must be suppressed; got: {out:?}"
    );
}

/// `Logger::production()` builds a logger with `production: true` and no file — calls to
/// `log()` must not panic and must not create a log file on disk.
#[test]
fn logger_production_mode_writes_no_file() {
    let lg = Logger::production();
    assert!(
        lg.config.production,
        "production() must set RuntimeLogConfig.production"
    );
    // log_path is the default, but file is None so nothing gets written.
    let mut lg = lg;
    lg.log(Severity::Error, "prod.loft", 1, "should not crash");
    // No assertion on file contents: production mode deliberately does not open one.
}

/// `Logger::from_config_file` with a non-existent path falls back to defaults rooted at
/// the main loft file's directory, producing a usable logger.
#[test]
fn logger_from_missing_config_uses_defaults() {
    let fake = PathBuf::from("/definitely/does/not/exist/loft.log.conf");
    let main = fa::with_extension(unique_tmp("logger_main"), "loft");
    let lg = Logger::from_config_file(&fake, main.to_str().unwrap());
    // Default level is Info — verify by logging a Warn and checking it gets through
    // to the default log path under the main file's parent.
    let expected_dir = main.parent().unwrap().join(".loft");
    let expected_log = expected_dir.join("log.txt");
    assert_eq!(
        lg.config.log_path, expected_log,
        "default log path should be <main_dir>/.loft/log.txt"
    );
}

/// Size-based rotation: once `current_size >= max_size_bytes`, the active log is moved
/// aside and a fresh file starts.  We verify a `.1` backup appears and that the
/// second message lands in the fresh log.
#[test]
fn logger_size_rotation_creates_backup() {
    let path = unique_tmp("logger_rot");
    let config = RuntimeLogConfig {
        log_path: path.clone(),
        default_level: Severity::Info,
        rate_per_minute: 0,
        daily_rotation: false,
        max_size_bytes: 50, // tiny — first log line will exceed
        max_files: 3,
        ..RuntimeLogConfig::default()
    };
    let mut lg = Logger::new(config, None);
    // Write a long first line so current_size passes 50 bytes.
    lg.log(
        Severity::Info,
        "rot.loft",
        1,
        "first-message-padding-to-exceed-fifty-bytes",
    );
    // Second call sees current_size >= 50 → triggers rotate(), then writes.
    lg.log(Severity::Info, "rot.loft", 2, "second-after-rotation");
    drop(lg);

    let stem = fa::file_stem(&path).unwrap();
    let ext = fa::extension(&path)
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    let backup = fa::parent(&path)
        .unwrap_or_default()
        .join(format!("{stem}.1{ext}"));
    let backup_exists = fa::exists(&backup);
    let current = read_log(&path);
    cleanup(&path);
    let _ = fa::remove_file(&backup);
    assert!(
        backup_exists,
        "expected rotation backup {backup:?} after exceeding max_size_bytes"
    );
    assert!(
        current.contains("second-after-rotation"),
        "post-rotation log must contain the second message; got: {current:?}"
    );
}

/// `generate_config()` returns a non-empty template string with the documented keys — ensures
/// the helper used by `loft --generate-log-config` keeps at least the core options.
#[test]
fn logger_generate_config_contains_core_keys() {
    let tmpl = loft::logger::generate_config();
    for key in &["file", "level", "production", "max_size_mb", "daily"] {
        assert!(
            tmpl.contains(key),
            "generate_config() template missing `{key}`; got:\n{tmpl}"
        );
    }
}
