// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I76 — Logger runtime

//! Runtime logging framework for loft / loft programs.
//!
//! Distinct from `log_config.rs` (the compile/test trace framework):
//! this module handles structured, file-based output from running loft code.

#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Log severity level.  Levels are ordered: Info < Warn < Error < Fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Error,
    Fatal,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "INFO ",
            Severity::Warn => "WARN ",
            Severity::Error => "ERROR",
            Severity::Fatal => "FATAL",
        }
    }

    fn from_str(s: &str) -> Option<Severity> {
        match s.to_ascii_lowercase().as_str() {
            "info" => Some(Severity::Info),
            "warn" | "warning" => Some(Severity::Warn),
            "error" => Some(Severity::Error),
            "fatal" => Some(Severity::Fatal),
            _ => None,
        }
    }
}

/// Runtime logging configuration (parsed from a `.conf` file or built with defaults).
pub struct RuntimeLogConfig {
    pub log_path: PathBuf,
    pub default_level: Severity,
    pub production: bool,
    pub max_size_bytes: u64,
    pub daily_rotation: bool,
    pub max_files: u32,
    pub rate_per_minute: u32,
    pub file_levels: HashMap<String, Severity>,
}

impl Default for RuntimeLogConfig {
    fn default() -> Self {
        RuntimeLogConfig {
            log_path: PathBuf::from("log.txt"),
            default_level: Severity::Warn,
            production: false,
            max_size_bytes: 500 * 1024 * 1024,
            daily_rotation: true,
            max_files: 10,
            rate_per_minute: 5,
            file_levels: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

struct RateEntry {
    count: u32,
    window_start: Instant,
    suppressed: u32,
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// Runtime logger shared across threads via `Arc<Mutex<Logger>>`.
pub struct Logger {
    pub config: RuntimeLogConfig,
    config_path: Option<PathBuf>,
    config_mtime: Option<SystemTime>,
    last_config_check: Instant,
    file: Option<BufWriter<File>>,
    current_size: u64,
    /// (year, month, day) in UTC — used for daily rotation detection.
    current_ymd: (u32, u32, u32),
    rate_map: HashMap<(String, u32), RateEntry>,
    /// What a record's source path is shown RELATIVE to: the project root when the program
    /// is in one, else the main file's own directory.  `None` leaves paths as they arrive.
    ///
    /// A log record is not a diagnostic.  A diagnostic is read by the author on the machine
    /// that produced it, where an absolute path is the useful thing; a log is shipped off
    /// that machine and read by someone else, for whom `/home/<user>/…` on every line is
    /// noise, is long enough to bury the message, and discloses the build account
    /// (loft#1264).  So the relativising happens HERE and diagnostics are left alone.
    source_base: Option<PathBuf>,
}

impl Logger {
    /// Create a logger from an already-built `RuntimeLogConfig`.
    #[must_use]
    pub fn new(config: RuntimeLogConfig, config_path: Option<PathBuf>) -> Self {
        let mut logger = Logger {
            config,
            config_path,
            config_mtime: None,
            last_config_check: Instant::now(),
            file: None,
            current_size: 0,
            current_ymd: (0, 0, 0),
            rate_map: HashMap::new(),
            source_base: None,
        };
        // Record mtime of config file if we have one.
        if let Some(ref p) = logger.config_path.clone() {
            logger.config_mtime = std::fs::metadata(p).ok().and_then(|m| m.modified().ok());
        }
        logger.open_file();
        logger
    }

    /// Build a production-mode logger that suppresses panics (assert/panic set `had_fatal`
    /// instead of aborting).  Does not write to any log file.
    #[must_use]
    pub fn production() -> Self {
        let config = RuntimeLogConfig {
            production: true,
            ..RuntimeLogConfig::default()
        };
        Logger {
            config,
            config_path: None,
            config_mtime: None,
            last_config_check: Instant::now(),
            file: None,
            current_size: 0,
            current_ymd: (0, 0, 0),
            rate_map: HashMap::new(),
            source_base: None,
        }
    }

    /// Where to look for the log config — the ONE answer both backends use.
    ///
    /// `explicit` is `--log-conf` under the interpreter and `LOFT_LOG_CONF` inside a
    /// compiled binary: a `--native` program is a separate process that never sees the
    /// driver's flags, so an env var is the only way to redirect a shipped binary's
    /// logging. Otherwise the search is the same on both: `.loft/log.conf` beside the
    /// program, else `log.conf` beside it.
    ///
    /// Shared deliberately. The two backends resolving this differently is precisely how
    /// `--native` came to log nothing at all: it had no logger, so every `log_info` /
    /// `log_warn` / `log_error` / `log_fatal` was silently dropped on the DEFAULT backend
    /// while `--interpret` wrote them.
    #[must_use]
    pub fn resolve_config_path(explicit: Option<&str>, main_loft_file: &str) -> PathBuf {
        if let Some(p) = explicit {
            return PathBuf::from(p);
        }
        let dir = Path::new(main_loft_file)
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let loft_conf = dir.join(".loft").join("log.conf");
        if loft_conf.exists() {
            loft_conf
        } else {
            dir.join("log.conf")
        }
    }

    /// Build a `Logger` from a config file path (or defaults if the file doesn't exist).
    ///
    /// `main_loft_file` is used to determine the default log directory.
    #[must_use]
    pub fn from_config_file(path: &Path, main_loft_file: &str) -> Self {
        let default_log_dir = Path::new(main_loft_file)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let default_log_path = default_log_dir.join(".loft").join("log.txt");

        let config = if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                let conf_dir = path.parent().unwrap_or(Path::new("."));
                parse_config_str(&content, conf_dir)
            } else {
                RuntimeLogConfig {
                    log_path: default_log_path,
                    ..Default::default()
                }
            }
        } else {
            RuntimeLogConfig {
                log_path: default_log_path,
                ..Default::default()
            }
        };

        let config_path = if path.exists() {
            Some(path.to_path_buf())
        } else {
            None
        };
        let mut logger = Logger::new(config, config_path);
        logger.source_base = source_base_for(main_loft_file);
        logger
    }

    /// Render a loft source path the way a log record shows it: relative to the project
    /// root, or to the main file's directory when there is no manifest.
    ///
    /// Unchanged when nothing to strip applies — a stdlib file, a library from elsewhere,
    /// or a path already relative. An absolute path is still better than a wrong one.
    ///
    /// The separator is always `/`, on every platform, via [`crate::portable_path`]. A record
    /// is shipped OFF the machine that wrote it, so the host's separator is not part of what
    /// the file is called; and the `[levels]` keys this value is matched against are written
    /// `sub/` in a config file that is checked in and shared, so a host-specific spelling
    /// matched nothing there and the record named a file no reader could search for.
    fn display_path(&self, loft_file: &str) -> String {
        let relative = match self.source_base.as_ref().and_then(|base| base.to_str()) {
            Some(base) => loft_file
                .strip_prefix(base)
                .map_or(loft_file, |rest| rest.trim_start_matches(['/', '\\'])),
            None => loft_file,
        };
        crate::portable_path::portable_str(relative)
    }

    /// Write a log record.  Applies rate limiting and level filtering.
    ///
    /// # Panics
    ///
    /// Panics if the internal rate-limit map entry is missing after insertion (indicates a bug).
    pub fn log(&mut self, sev: Severity, loft_file: &str, line: u32, msg: &str) {
        // Relativised ONCE, here, because three things downstream have to agree about what
        // this file is called: the `[levels]` override match, the rate-limit key, and the
        // record itself.  While the record carried an absolute path, a `[levels]` prefix
        // key such as `src/` could not match ANY file and the feature the generated config
        // documents had no working spelling (loft#1264).
        let displayed = self.display_path(loft_file);
        let loft_file: &str = &displayed;

        // Level filter
        if sev < self.effective_level(loft_file) {
            return;
        }

        // Rate limiting
        let key = (loft_file.to_string(), line);
        let now = Instant::now();
        let rate = self.config.rate_per_minute;
        if rate > 0 {
            let window = Duration::from_mins(1);
            // Compute the suppression notice (if any) and update state before any borrow
            let suppression_notice: Option<String> = {
                let entry = self
                    .rate_map
                    .entry(key.clone())
                    .or_insert_with(|| RateEntry {
                        count: 0,
                        window_start: now,
                        suppressed: 0,
                    });
                if now.duration_since(entry.window_start) > window {
                    let notice = if entry.suppressed > 0 {
                        let ts = utc_timestamp();
                        let sev_str = sev.as_str();
                        let file = loft_file;
                        let n = entry.suppressed;
                        Some(format!(
                            "{ts} {sev_str}  {file}:{line}  [suppressed {n} identical messages in the last 60s]\n",
                        ))
                    } else {
                        None
                    };
                    entry.count = 0;
                    entry.suppressed = 0;
                    entry.window_start = now;
                    notice
                } else {
                    None
                }
            };
            // Now check rate (re-borrow entry)
            let should_suppress = {
                let entry = self.rate_map.get_mut(&key).expect("just inserted");
                if entry.count >= rate {
                    entry.suppressed += 1;
                    true
                } else {
                    entry.count += 1;
                    false
                }
            };
            if let Some(notice) = suppression_notice {
                self.write_line(&notice);
            }
            if should_suppress {
                return;
            }
        }

        // Check if daily rotation is needed
        let today = today_ymd();
        if self.config.daily_rotation && self.current_ymd != (0, 0, 0) && today != self.current_ymd
        {
            self.rotate();
        }
        self.current_ymd = today;

        // Check size rotation
        if self.config.max_size_bytes > 0 && self.current_size >= self.config.max_size_bytes {
            self.rotate();
        }

        let ts = utc_timestamp();
        let sev_str = sev.as_str();
        let file = loft_file;
        let line_str = format!("{ts} {sev_str}  {file}:{line}  {msg}\n");
        self.write_line(&line_str);
    }

    /// Plan-07 phase 4 — log a typed runtime event (`RuntimeError`)
    /// at the severity prescribed by the kind table (see DESIGN_DECISIONS
    /// § C66).  Single-line message shape: `[<kind_label>] <kind.describe()>`
    /// — the label is a stable machine-readable identifier for grepping
    /// log files; the describe() gives the human-readable detail with
    /// any structured-field values (idx, len, message, etc.) inline.
    /// Routes through `Logger::log` so rate-limiting + level filtering
    /// + rotation all apply uniformly.
    ///
    /// Used by `State::raise` in production mode (when
    /// `config.production == true`) to surface the runtime event to
    /// operators without halting the program.  In development mode the
    /// caller (`State::raise` else-branch) uses `render_entry_pretty`
    /// instead for the multi-line halt + caret display.
    pub fn log_runtime_kind(
        &mut self,
        kind: &crate::runtime_error::RuntimeErrorKind,
        position: Option<&crate::lexer::Position>,
    ) {
        use crate::runtime_error::RuntimeErrorKind as Rk;
        let severity = match kind {
            // A relayed fault halted the library it fired in, whatever kind it
            // started as — only halting faults cross a placement boundary.
            // A write to the author's own `#lock` halts the run like the two beside it.
            Rk::UserPanic { .. }
            | Rk::StackOverflow
            | Rk::WriteToLockedStore { .. }
            | Rk::Relayed { .. } => Severity::Fatal,
            Rk::AssertionFailed { .. } => Severity::Error,
            Rk::DivideByZero
            | Rk::IndexOutOfBounds { .. }
            | Rk::NegativeIndex { .. }
            | Rk::NullDereference
            | Rk::NarrowCastOverflow { .. }
            | Rk::ShiftOutOfRange
            | Rk::CastOutOfRange
            | Rk::RangeDefaulted { .. } => Severity::Warn,
        };
        let label = kind.label();
        let detail = kind.describe();
        let msg = format!("[{label}] {detail}");
        let (file, line) =
            position.map_or_else(|| (String::new(), 0), |p| (p.file.clone(), p.line));
        self.log(severity, &file, line, &msg);
    }

    /// Check if the config file has changed and reload if so.  Only does I/O if 5+ seconds
    /// have passed since the last check.
    pub fn check_reload(&mut self) {
        if self.last_config_check.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.last_config_check = Instant::now();
        let Some(config_path) = self.config_path.clone() else {
            return;
        };
        let new_mtime = std::fs::metadata(&config_path)
            .ok()
            .and_then(|m| m.modified().ok());
        if new_mtime == self.config_mtime {
            return;
        }
        // Config changed — reload
        self.config_mtime = new_mtime;
        let old_path = self.config.log_path.clone();
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            let conf_dir = config_path.parent().unwrap_or(Path::new("."));
            let new_config = parse_config_str(&content, conf_dir);
            let path_changed = new_config.log_path != old_path;
            self.config = new_config;
            if path_changed {
                self.file = None;
                self.current_size = 0;
                self.open_file();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn effective_level(&self, loft_file: &str) -> Severity {
        // Check per-file overrides: exact basename match or path prefix
        let basename = Path::new(loft_file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(loft_file);

        if let Some(&sev) = self.config.file_levels.get(basename) {
            return sev;
        }
        // Path prefix check (keys ending in "/")
        for (pattern, &sev) in &self.config.file_levels {
            if pattern.ends_with('/') && loft_file.starts_with(pattern.as_str()) {
                return sev;
            }
        }
        self.config.default_level
    }

    #[cfg_attr(feature = "wasm", allow(clippy::unused_self))]
    fn write_line(&mut self, line: &str) {
        #[cfg(not(feature = "wasm"))]
        {
            let bytes = line.as_bytes();
            if let Some(ref mut f) = self.file {
                let _ = f.write_all(bytes);
                let _ = f.flush();
                self.current_size += bytes.len() as u64;
            }
        }
        #[cfg(feature = "wasm")]
        crate::wasm::host_log_write(line);
    }

    fn open_file(&mut self) {
        let path = &self.config.log_path;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(f) => {
                let size = f.metadata().map_or(0, |m| m.len());
                self.current_size = size;
                self.file = Some(BufWriter::new(f));
                self.current_ymd = today_ymd();
            }
            Err(_) => {
                self.file = None;
            }
        }
    }

    fn rotate(&mut self) {
        // Flush and close current file
        if let Some(ref mut f) = self.file {
            let _ = f.flush();
        }
        self.file = None;

        let path = &self.config.log_path;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("log")
            .to_string();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| format!(".{s}"))
            .unwrap_or_default();
        let dir = path.parent().unwrap_or(Path::new("."));

        let max = self.config.max_files;

        // Delete the oldest file if at the limit
        if max > 0 {
            let oldest = dir.join(format!(
                "{stem}.{}.{}",
                max - 1,
                ext.trim_start_matches('.')
            ));
            let _ = std::fs::remove_file(&oldest);
        }

        // Shift: log.(N-2).ext → log.(N-1).ext, …, log.1.ext → log.2.ext
        if max > 1 {
            for i in (1..max - 1).rev() {
                let src = dir.join(format!("{stem}.{i}{ext}"));
                let dst = dir.join(format!("{stem}.{}{ext}", i + 1));
                let _ = std::fs::rename(&src, &dst);
            }
        }

        // log.ext → log.1.ext
        let archive = dir.join(format!("{stem}.1{ext}"));
        let _ = std::fs::rename(path, &archive);

        // Open fresh log file
        self.current_size = 0;
        self.open_file();
    }
}

// ---------------------------------------------------------------------------
// Config parsing
// ---------------------------------------------------------------------------

/// What log records show a source path relative to, for a program whose entry is
/// `main_loft_file`.
///
/// The project root when there is a manifest above the entry — which is what
/// LOGGER.md's "relative to project root" has always claimed — and otherwise the entry's
/// own directory, which is the only meaningful answer for a bare script.  `None` when
/// neither can be determined, and then paths are left exactly as they arrive.
fn source_base_for(main_loft_file: &str) -> Option<PathBuf> {
    if main_loft_file.is_empty() {
        return None;
    }
    if let Some(root) = crate::resolution_scope::project_root(main_loft_file) {
        return Some(root);
    }
    let dir = Path::new(main_loft_file).parent()?;
    if dir.as_os_str().is_empty() {
        return None;
    }
    // Canonicalised, in the plain spelling a record's absolute path carries, so the strip
    // matches; a path that cannot be canonicalised is used as-is rather than dropped.
    Some(crate::portable_path::plain_canonical(dir))
}

fn parse_config_str(content: &str, conf_dir: &Path) -> RuntimeLogConfig {
    let mut config = RuntimeLogConfig::default();
    let mut section = String::new();
    let mut log_path_set = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some(eq) = line.find('=') {
            // Both halves shed surrounding quotes.  Only the value used to, and a `[levels]`
            // key is the one place a key is a STRING rather than a fixed word — so
            // `"app.loft" = info`, the spelling this file's own generated template
            // documents, was stored with its quote characters and never matched the bare
            // basename it is looked up by.  Silently: an override that does nothing looks
            // exactly like one that is not needed.
            let key = line[..eq].trim().trim_matches('"');
            let value = line[eq + 1..].trim().trim_matches('"');
            match (section.as_str(), key) {
                ("log", "file") => {
                    let p = Path::new(value);
                    config.log_path = if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        conf_dir.join(p)
                    };
                    log_path_set = true;
                }
                ("log", "level") => {
                    if let Some(sev) = Severity::from_str(value) {
                        config.default_level = sev;
                    }
                }
                ("log", "production") => {
                    config.production = value.eq_ignore_ascii_case("true") || value == "1";
                }
                ("rotation", "max_size_mb") => {
                    if let Ok(mb) = value.parse::<u64>() {
                        config.max_size_bytes = mb * 1024 * 1024;
                    }
                }
                ("rotation", "daily") => {
                    config.daily_rotation = value.eq_ignore_ascii_case("true") || value == "1";
                }
                ("rotation", "max_files") => {
                    if let Ok(n) = value.parse::<u32>() {
                        config.max_files = n;
                    }
                }
                ("rate_limit", "per_site") => {
                    if let Ok(n) = value.parse::<u32>() {
                        config.rate_per_minute = n;
                    }
                }
                ("levels", _) => {
                    if let Some(sev) = Severity::from_str(value) {
                        config.file_levels.insert(key.to_string(), sev);
                    }
                }
                _ => {}
            }
        }
    }

    if !log_path_set {
        // Keep default relative path unchanged (will be set by caller if needed)
    }
    config
}

// ---------------------------------------------------------------------------
// Config template
// ---------------------------------------------------------------------------

/// Returns the documented default config template as a static string.
#[must_use]
pub fn generate_config() -> &'static str {
    r#"# Lavition runtime log configuration
# Generated by: loft --generate-log-config
#
# Hot-reload: changes are picked up within ~5 seconds without restart.
# All paths are relative to the directory containing this config file
# unless they start with / or ~ (absolute/home).

[log]

# Path to the log file.
# Relative to the directory of the main .loft file.
# Default: log.txt
file = log.txt

# Minimum severity to write.
# Choices: info | warn | error | fatal
# Records below this level are silently discarded.
# Default: warn
level = warn

# Production mode: when true, panic() becomes a fatal log entry and
# assert() becomes an error log entry.  The program continues running
# instead of aborting.
# Default: false
production = false

[rotation]

# Maximum size of a single log file in megabytes before rotation.
# Set to 0 to disable size-based rotation.
# Default: 500
max_size_mb = 500

# Rotate at midnight UTC even if the size limit has not been reached.
# Default: true
daily = true

# Maximum total number of log files to keep (current + archived).
# Oldest files beyond this count are deleted during rotation.
# Default: 10
max_files = 10

[rate_limit]

# Maximum messages allowed from the same source location (file + line)
# within a 60-second sliding window.  Further messages from that site
# are suppressed; a suppression notice is emitted when the window resets.
# Set to 0 to disable rate limiting.
# Default: 5
per_site = 5

[levels]

# Per-file severity overrides.  The key is the loft source file name
# (basename only, e.g. "score.loft") or a path prefix ending in "/".
# Both are matched against the path as the log record shows it: relative
# to the project root, or to the main file's directory if there is no
# loft.toml.  A prefix therefore only addresses files inside a project.
# The value overrides the global [log] level for that file.
#
# Examples:
#   "debug_tool.loft" = info
#   "src/"            = error
"#
}

// ---------------------------------------------------------------------------
// Timestamp helpers (no external crates)
// ---------------------------------------------------------------------------

/// Format `SystemTime::now()` as an ISO 8601 UTC timestamp with millisecond precision.
/// Example: `"2026-03-13T14:05:32.417Z"`
fn utc_timestamp() -> String {
    let now = SystemTime::now();
    let since_epoch = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let total_secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();

    let (y, m, d) = days_to_ymd(total_secs / 86400);
    let rem = total_secs % 86400;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Return today's UTC date as `(year, month, day)`.
fn today_ymd() -> (u32, u32, u32) {
    let since_epoch = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    days_to_ymd(since_epoch.as_secs() / 86400)
}

/// Convert a count of days since the Unix epoch (1970-01-01) to `(year, month, day)`.
/// Exposed `pub(crate)` so `src/native.rs::n_ymd_days_ago` can reuse it without
/// duplicating the Howard Hinnant algorithm.
///
/// Algorithm from Howard Hinnant: <https://howardhinnant.github.io/date_algorithms.html>
#[allow(clippy::many_single_char_names)] // Standard mathematical variable names from the algorithm
pub(crate) fn days_to_ymd(z: u64) -> (u32, u32, u32) {
    // Shift epoch from 1970-01-01 to 0000-03-01
    let z = z as i64 + 719_468;
    let era: i64 = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month of year [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Position;
    use crate::runtime_error::RuntimeErrorKind;

    fn tmp_logger() -> Logger {
        let tmp = std::env::temp_dir().join(format!(
            "loft_logger_{}_{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = RuntimeLogConfig {
            log_path: tmp,
            default_level: Severity::Info,
            rate_per_minute: 0, // disable rate limit for these tests
            ..Default::default()
        };
        Logger::new(config, None)
    }

    fn pos(file: &str, line: u32) -> Position {
        Position {
            file: file.into(),
            line,
            pos: 1,
        }
    }

    fn read_log(logger: &Logger) -> String {
        std::fs::read_to_string(&logger.config.log_path).unwrap_or_default()
    }

    fn drop_log(logger: Logger) {
        let path = logger.config.log_path.clone();
        drop(logger);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn runtime_kind_user_panic_logs_at_fatal() {
        let mut lg = tmp_logger();
        let kind = RuntimeErrorKind::UserPanic {
            message: "boom".into(),
        };
        lg.log_runtime_kind(&kind, Some(&pos("game.loft", 7)));
        let out = read_log(&lg);
        assert!(out.contains("FATAL"), "expected FATAL severity in {out:?}");
        assert!(
            out.contains("[user_panic]"),
            "expected kind label tag; got {out:?}"
        );
        assert!(
            out.contains("panic: boom"),
            "expected describe()d detail; got {out:?}"
        );
        assert!(
            out.contains("game.loft:7"),
            "expected source location; got {out:?}"
        );
        drop_log(lg);
    }

    #[test]
    fn runtime_kind_assertion_failed_logs_at_error() {
        let mut lg = tmp_logger();
        let kind = RuntimeErrorKind::AssertionFailed {
            message: "x == 5".into(),
        };
        lg.log_runtime_kind(&kind, Some(&pos("test.loft", 12)));
        let out = read_log(&lg);
        assert!(
            out.contains("ERROR"),
            "expected ERROR severity; got {out:?}"
        );
        assert!(out.contains("[assertion_failed]"));
        drop_log(lg);
    }

    #[test]
    fn runtime_kind_div_by_zero_logs_at_warn() {
        let mut lg = tmp_logger();
        lg.log_runtime_kind(&RuntimeErrorKind::DivideByZero, Some(&pos("math.loft", 3)));
        let out = read_log(&lg);
        assert!(out.contains("WARN"), "expected WARN severity; got {out:?}");
        assert!(out.contains("[divide_by_zero]"));
        assert!(out.contains("divide by zero"));
        drop_log(lg);
    }

    #[test]
    fn runtime_kind_oob_carries_structured_fields() {
        let mut lg = tmp_logger();
        lg.log_runtime_kind(
            &RuntimeErrorKind::IndexOutOfBounds { idx: 5, len: 3 },
            Some(&pos("v.loft", 8)),
        );
        let out = read_log(&lg);
        assert!(out.contains("[index_out_of_bounds]"));
        assert!(
            out.contains("index 5 out of bounds for length 3"),
            "expected idx + len in message; got {out:?}"
        );
        drop_log(lg);
    }

    #[test]
    fn runtime_kind_stack_overflow_is_fatal() {
        let mut lg = tmp_logger();
        lg.log_runtime_kind(&RuntimeErrorKind::StackOverflow, None);
        let out = read_log(&lg);
        assert!(
            out.contains("FATAL"),
            "expected FATAL severity; got {out:?}"
        );
        assert!(out.contains("[stack_overflow]"));
        drop_log(lg);
    }

    #[test]
    fn runtime_kind_without_position_logs_with_empty_file() {
        let mut lg = tmp_logger();
        lg.log_runtime_kind(
            &RuntimeErrorKind::NullDereference,
            None, // no position resolved
        );
        let out = read_log(&lg);
        assert!(out.contains("[null_dereference]"));
        // No source file → empty file field renders as `:0`
        assert!(
            out.contains(":0"),
            "expected line:0 when no position; got {out:?}"
        );
        drop_log(lg);
    }

    #[test]
    fn runtime_kind_rate_limit_suppresses_repeats() {
        let mut lg = tmp_logger();
        // Override rate limit so the suppression path fires within the test.
        lg.config.rate_per_minute = 2;
        for _ in 0..5 {
            lg.log_runtime_kind(&RuntimeErrorKind::DivideByZero, Some(&pos("loop.loft", 4)));
        }
        let out = read_log(&lg);
        // Should see only 2 emissions of the divide-by-zero line (the rate cap);
        // the remaining 3 are suppressed.
        let div_lines = out
            .lines()
            .filter(|l| l.contains("[divide_by_zero]"))
            .count();
        assert_eq!(div_lines, 2, "expected rate cap to fire; got {out:?}");
        drop_log(lg);
    }
}
