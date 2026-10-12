
# Runtime Logging Framework

Documents the loft runtime logging system.

---

## Contents
- [Overview](#overview)
- [User-Visible Loft API](#user-visible-loft-api)
- [Log Record Format](#log-record-format)
- [Configuration File](#configuration-file)
- [Rate Limiting](#rate-limiting)
- [Production Mode](#production-mode)
- [Hidden Compile-Time File + Line Parameters](#hidden-compile-time-file--line-parameters)
- [Architecture](#architecture)
- [Files Changed](#files-changed)
- [Deferred](#deferred)
- [Resolved Design Decisions](#resolved-design-decisions)

---

## Overview

The logging framework adds structured, file-based output to running loft programs.  It is distinct from `src/log_config.rs` (the compile/test trace framework): that file controls debug output during development; this framework is for production-visible events emitted by user loft code and the runtime itself.

Key properties:
- Severities: `info`, `warn`, `error`, `fatal`
- Each log record carries: severity, UTC timestamp, source file + line of the *loft* call site
- Rate limiting: at most 5 messages per minute per call site (file+line)
- File rotation: by size (default 500 MB) or daily, keeping at most 10 files
- Config file: well-documented, hot-reloaded every ~5 s without restart
- CLI option `--generate-log-config` writes a documented default config to stdout/file
- `panic` → `fatal` log and `assert` → `error` log in production mode (program continues)
- Loft source file + line injected at **compile time** as hidden parameters to `panic`/`assert`

---

## User-Visible Loft API

Four severity-keyed functions are added to `default/01_code.loft`:

```loft
// Write a log record at the given severity.
// The message is a plain text or formatted string (use "{var}" interpolation).
pub fn log_info(message: text);
pub fn log_warn(message: text);
pub fn log_error(message: text);
pub fn log_fatal(message: text);
```

`panic` and `assert` already exist.  Their *user signatures* are unchanged:

```loft
pub fn panic(message: text);
pub fn assert(test: boolean, message: text);
```

Internally the parser rewrites these two to pass the call-site file and line as hidden extra arguments (see §6).  In non-production mode they behave exactly as today (Rust `panic!` / `assert!`).  In production mode they write to the log file instead.

---

## Log Record Format

Each line written to the log file has the format:

```
2026-03-13T14:05:32.417Z WARN  src/compute.loft:142  division result may overflow
```

Fields (tab-separated internally, displayed as aligned columns):

| Field | Example | Notes |
|---|---|---|
| UTC timestamp | `2026-03-13T14:05:32.417Z` | ISO 8601, millisecond precision |
| Severity | `INFO` / `WARN` / `ERROR` / `FATAL` | 5-char padded |
| Source location | `src/compute.loft:142` | relative to the project root (the nearest `loft.toml` above the entry), or to the entry's own directory when there is no manifest, + line |
| Message | `division result may overflow` | fully-interpolated user string |

Rate-suppression notice (emitted when a window resets after suppression):

```
2026-03-13T14:06:00.001Z WARN  src/compute.loft:142  [suppressed 7 identical messages in the last 60s]
```

---

## Configuration File

Default location: `log.conf` in the same directory as the main `.loft` file — exactly that
directory; there is no upward search, and a config one level up is not found.
Alternative: pass `--log-conf <path>` on the CLI.
Print a documented template with defaults: `loft --generate-log-config`, which writes to
STDOUT; `loft --generate-log-config <output-path>` writes it to that file instead.

⚠ A `[levels]` key is the one config key that is a STRING rather than a fixed word, so it
sheds surrounding quotes the way a value does — `"app.loft" = info` and `app.loft = info`
are the same key. Until 2026-09-01 only the value was unquoted, so the quoted spelling this
file's own generated template prints was stored WITH its quote characters and silently
matched nothing.

**A key is matched against the RELATIVE path**, which is the same string the record shows:
the basename form (`app.loft`) against the file name, the prefix form (`src/`) against the
path from the project root. The prefix form had no working spelling at all until then,
because the path it was matched against was absolute and no key ending in `/` could be a
prefix of one (loft#1264). A prefix therefore only means something inside a project — for a
bare script the base is the script's own directory, so there is no prefix left to match, and
the basename form is the one that addresses it.

The relativising happens in `Logger::log`, the single point where a loft path enters the
logger, so the override match, the rate-limit key and the written record cannot disagree
about what a file is called. **Diagnostics are deliberately NOT relativised**: they are read
by the author on the machine that produced them, where the full path is what makes the
message actionable, while a log is read by someone else somewhere else.

### Format

Plain `key = value` pairs, `#` comments.  Sections use `[header]` markers.

```conf
# Lavition runtime log configuration
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
```

### File Rotation Naming

Given `file = log.txt`:

| Slot | File |
|---|---|
| Current | `log.txt` |
| Most recent archive | `log.1.txt` |
| Oldest archive | `log.N.txt` (N = max_files − 1) |

On rotation: `log.(N-1).txt → log.N.txt`, …, `log.txt → log.1.txt`, new `log.txt` opened.  Files beyond `max_files` are deleted.

---

## Rate Limiting

The rate limit is per **call site** (loft source file + line number), not per message text.  This is reliable because:
- The call site is injected at compile time (see §6).
- Text content varies with interpolated variables; the "same message" semantic is correctly captured by same source location.

Each call site has a `RateEntry { count: u32, window_start: Instant, suppressed: u32 }`.

Algorithm (checked before writing each record):
1. If `rate_per_minute == 0`, skip (unlimited).
2. If `now − window_start > 60s`, reset `count = 0, suppressed = 0, window_start = now`.
3. If `count < rate_per_minute`, write the record, increment `count`.
4. Else increment `suppressed`, skip writing.
5. On window reset in step 2, if `suppressed > 0` write the suppression notice first.

---

## Production Mode

When `production = true` (or `--production` CLI flag):

| Call | Normal behaviour | Production behaviour |
|---|---|---|
| `panic(msg)` | Rust `panic!`, program aborts | `fatal` log entry, execution continues |
| `assert(false, msg)` | Rust `assert!`, program aborts | `error` log entry, execution continues |

Both backends, through one decision — `runtime_error::logged_in_production`, called by the
interpreter's builtins (`src/native.rs`) and by the `n_panic` / `n_assert` bodies the native
generator writes (`src/generation/mod.rs`), which have no `#rust` template and so are hand-
written. It was interpreter-only until 2026-09-01: `--native`, the DEFAULT backend, went
straight to `report_and_exit()`, so a `panic()` under `production = true` aborted and wrote
nothing — the outcome the feature exists to prevent (loft#1263).

⚠ The process exits non-zero on either backend even when execution continues, so an
exit-code check cannot tell production mode from a halt. What separates them is whether the
statements AFTER the fault ran, and whether a record reached the log.

In production mode the hidden return value from the fatal/error log call is `Value::Null` so that the IR position after the call is reachable.  `assert` is a native function call that checks the production flag at runtime.

The production flag is stored on the `Logger` (and thus on `Stores`).  It is read by `n_panic` and `n_assert` to decide whether to `panic!` or log.

---

## Hidden Compile-Time File + Line Parameters

The parser already special-cases `assert` and `parallel_for` in `parse_call` (`parser.rs:6304`).  The same mechanism is extended to inject source location:

### `panic`

User writes: `panic("overflow in {x}")` at `src/compute.loft:142`.

Parser rewrites the call to: `n_panic(message, file_str, line_int)`

```
Value::Call(n_panic_d_nr, [
    <message expr>,
    Value::str("src/compute.loft"),   // self.lexer.pos().file   at parse time
    Value::Int(142),                   // self.lexer.pos().line
])
```

### `assert`

User writes: `assert(x > 0, "x must be positive")` at `src/compute.loft:200`.

The parser emits a native call, which can carry the location an inline operator could not:

```
Value::Call(n_assert_d_nr, [
    <test expr>,
    <message expr>,
    Value::str("src/compute.loft"),
    Value::Int(200),
])
```

The `n_assert` native then checks the production flag.

### loft Log Functions

`log_info(msg)` etc. also need source location.  They are similarly special-cased:

```
Value::Call(n_log_info_d_nr, [
    <message expr>,
    Value::str(file),
    Value::Int(line),
])
```

The loft declarations remain one-argument (`pub fn log_info(message: text)`) because the extra parameters are hidden — inserted only by the parser, never visible to the user.

---

## Architecture

### New File: `src/logger.rs`

```rust
pub enum Severity { Info, Warn, Error, Fatal }

struct RateEntry {
    count: u32,
    window_start: std::time::Instant,
    suppressed: u32,
}

pub struct RuntimeLogConfig {
    pub log_path: std::path::PathBuf,
    pub default_level: Severity,
    pub production: bool,
    pub max_size_bytes: u64,
    pub daily_rotation: bool,
    pub max_files: u32,
    pub rate_per_minute: u32,
    pub file_levels: std::collections::HashMap<String, Severity>,
}

pub struct Logger {
    config: RuntimeLogConfig,
    config_path: std::path::PathBuf,
    config_mtime: Option<std::time::SystemTime>,
    last_config_check: std::time::Instant,
    file: Option<std::io::BufWriter<std::fs::File>>,
    current_size: u64,
    current_ymd: (u32, u32, u32),  // UTC (year, month, day) for daily rotation
    rate_map: std::collections::HashMap<(String, u32), RateEntry>,
}

impl Logger {
    pub fn from_config_file(path: &std::path::Path, main_file: &str) -> Self { … }
    pub fn log(&mut self, sev: Severity, loft_file: &str, line: u32, msg: &str) { … }
    pub fn check_reload(&mut self) { … }  // called at most every 5s, checks mtime
    fn rotate(&mut self) { … }
    fn open_file(&mut self) { … }
    fn effective_level(&self, loft_file: &str) -> Severity { … }
}
```

No external time crate is required: use `std::time::SystemTime::now()` for timestamps (format manually as ISO 8601) and `std::time::Instant` for rate-limit windows.

### Changes to `src/database/mod.rs` → `Stores`

Add a field:

```rust
pub logger: Option<Arc<Mutex<Logger>>>,
```

`Stores::new` initialises to `None`.  `main.rs` creates the `Logger` and sets `stores.logger`.  Workers created by `clone_for_worker` share the same logger handle — use `Arc<Mutex<Logger>>` so all threads can log.

### Changes to `src/native.rs`

Replace the current `n_panic`/`n_assert` bodies:

```rust
fn n_panic(stores: &mut Stores, stack: &mut DbRef) {
    let v_line    = *stores.get::<i32>(stack);
    let v_file    = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    if let Some(logger) = &mut stores.logger {
        let mut lg = logger.lock().unwrap();
        if lg.config.production {
            lg.log(Severity::Fatal, v_file.str(), v_line as u32, v_message.str());
            return;
        }
    }
    panic!("{} ({}:{})", v_message.str(), v_file.str(), v_line);
}

fn n_assert(stores: &mut Stores, stack: &mut DbRef) {
    let v_line    = *stores.get::<i32>(stack);
    let v_file    = *stores.get::<Str>(stack);
    let v_message = *stores.get::<Str>(stack);
    let v_test    = *stores.get::<bool>(stack);
    if v_test { return; }
    if let Some(logger) = &mut stores.logger {
        let mut lg = logger.lock().unwrap();
        if lg.config.production {
            lg.log(Severity::Error, v_file.str(), v_line as u32, v_message.str());
            return;
        }
    }
    panic!("{} ({}:{})", v_message.str(), v_file.str(), v_line);
}

fn n_log_info(stores: &mut Stores, stack: &mut DbRef) { … }  // same pattern
fn n_log_warn(stores: &mut Stores, stack: &mut DbRef) { … }
fn n_log_error(stores: &mut Stores, stack: &mut DbRef) { … }
fn n_log_fatal(stores: &mut Stores, stack: &mut DbRef) { … }
```

### Changes to `src/parser/control.rs` → `parse_call`

Extend the `if name == "assert"` block to emit a native `n_assert` call with injected location.  Add parallel handling for `panic`, `log_info`, `log_warn`, `log_error`, `log_fatal`.

`assert` emits `Value::Call(n_assert_d_nr, args_with_location)`.  There is no `OpPanic` operator: `assert` and `panic` are both native calls.

```rust
fn parse_call(&mut self, val: &mut Value, source: u16, name: &str) -> Type {
    // … collect list / types as before …
    self.lexer.token(")");

    let pos = self.lexer.pos();          // Position { file, line, … }
    let loc = [
        Value::str(&pos.file),
        Value::Int(pos.line as i32),
    ];

    if name == "assert" {
        // Rewrite to n_assert(test, message, file, line)
        let test = …;
        let msg  = …;
        *val = Value::Call(n_assert_d_nr,
            vec![test, msg, loc[0].clone(), loc[1].clone()]);
        Type::Void
    } else if name == "panic" {
        // Rewrite to n_panic(message, file, line)
        *val = Value::Call(n_panic_d_nr,
            vec![list[0].clone(), loc[0].clone(), loc[1].clone()]);
        Type::Void
    } else if matches!(name, "log_info"|"log_warn"|"log_error"|"log_fatal") {
        // Rewrite to n_log_*(message, file, line)
        let fn_name = format!("n_{name}");
        let d_nr = self.data.def_nr(&fn_name);
        *val = Value::Call(d_nr,
            vec![list[0].clone(), loc[0].clone(), loc[1].clone()]);
        Type::Void
    } else if name == "parallel_for" {
        // … existing …
    } else {
        self.call(val, source, name, &list, &types)
    }
}
```

### Changes to `default/01_code.loft`

Remove the `#rust` inline bodies for `assert` and `panic`; they become native functions:

```loft
// Immediately terminates execution with message.
// In production mode, writes a fatal log entry instead of aborting.
pub fn panic(message: text);

// Panics with message if test is false.
// In production mode, writes an error log entry instead of aborting.
pub fn assert(test: boolean, message: text);

// Write a structured log record at the chosen severity.
pub fn log_info(message: text);
pub fn log_warn(message: text);
pub fn log_error(message: text);
pub fn log_fatal(message: text);
```

### Changes to `src/main.rs`

```
--generate-log-config [<path>]   write a documented config file with defaults and exit
--log-conf <path>                use this log config file instead of the default
--production                     enable production mode (panic/assert → log, no abort)
```

After parsing:
1. If `--generate-log-config`: write the config template, exit 0.
2. Create `Logger` from config file (or defaults if no file found).
3. Set `stores.logger = Some(Arc::new(Mutex::new(logger)))`.

---

## Files Changed

| File | Change |
|---|---|
| `src/logger.rs` | **New** — `Logger`, `RuntimeLogConfig`, `Severity`, rotation, rate-limiting, config parsing |
| `src/database/mod.rs` | Add `logger: Option<Arc<Mutex<Logger>>>` to `Stores`; update `clone_for_worker` in `allocation.rs` |
| `src/native.rs` | Rewrite `n_panic`/`n_assert`; add `n_log_info/warn/error/fatal` |
| `src/parser/control.rs` | `parse_call`: special-case `panic`, `assert`, `log_*` to inject file+line |
| `default/01_code.loft` | Remove `#rust` from `panic`/`assert`; add `log_info/warn/error/fatal` declarations |
| `src/main.rs` | New CLI flags: `--generate-log-config`, `--log-conf`, `--production` |
| `src/lib.rs` | Added `pub mod logger;` |
| `tests/docs/20-logging.loft` | **New** — test coverage |
| `tests/wrap.rs` | Added `logging()` test |

---

## Implementation Status

The following pieces from the Architecture section are **implemented**:
- `--production` CLI flag (main.rs)
- `n_panic` / `n_assert` with production-mode branching, on **both backends** —
  the decision is `runtime_error::logged_in_production`, called by the
  interpreter's builtins (native.rs) and by the bodies the native generator
  emits for those two (generation/mod.rs).  One home on purpose: while each
  backend decided it separately only the interpreter honoured it, so
  `loft app.loft` aborted and logged nothing where `loft --interpret app.loft`
  logged and carried on (loft#1263).  Guarded by
  `tests/panic_halts_both_backends.rs`
- `n_log_info/warn/error/fatal` (native.rs)
- Source-location injection for all six functions (parser/control.rs)
- `check_reload()` defined in logger.rs (but not yet called — see A2.1 below)
- `production = true/false` in log.conf (logger.rs)

## Remaining Work (A2)

See [PLANNING.md](PLANNING.md) § A2 for the phased breakdown.

---

### CLI Mode Flags

Four modes form a strict superset chain:

```
(no flag)     Default — assert() panics; log_* write to log.conf target.
--debug       Default + extra runtime null/overflow warnings per type (see §Type Safety Rules).
--production  assert()/panic() log instead of abort; bytecode unchanged.
--release     Implies --production; also elides assert() and debug_assert() from bytecode.
```

`--debug` and `--production`/`--release` are mutually exclusive (debug mode implies less
trust in the program; release mode implies more trust).

---

### RunMode — Shared State

A new `RunMode` enum is stored on `Stores` (replacing the current implicit production flag):

```rust
// src/database/mod.rs  (or a new src/run_mode.rs)
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum RunMode {
    #[default]
    Default,     // current behaviour
    Debug,       // extra null/overflow logging
    Production,  // panic/assert → log
    Release,     // implies Production; assert/debug_assert absent from bytecode
}

// Add to Stores:
pub run_mode: RunMode,
```

`main.rs` sets `state.database.run_mode` after parsing flags.
`Logger.config.production` is set when `run_mode` is `Production` or `Release`.
All runtime checks read `stores.run_mode` instead of locking the logger to check the flag
(cheaper: no mutex, just an enum comparison).

---

### A2.1 — Wire hot-reload (trivial)

Call `lg.check_reload()` at the top of each `n_log_*`, `n_panic`, and `n_assert` body in
`native.rs`, before the main logic.  The throttle in `check_reload()` ensures a `stat()`
syscall happens at most once per 5 s.

```rust
// In n_log_info (and the other five functions):
if let Ok(mut lg) = logger.lock() {
    lg.check_reload();   // ← add this line
    lg.log(Severity::Info, v_file.str(), v_line as u32, v_message.str());
}
```

Files: `src/native.rs` (6 lines added).

---

### A2.2 — `is_production()` and `is_debug()` (small)

Two new natives read `stores.run_mode`:

```rust
fn n_is_production(stores: &mut Stores, _stack: &mut DbRef) -> bool {
    matches!(stores.run_mode, RunMode::Production | RunMode::Release)
}

fn n_is_debug(stores: &mut Stores, _stack: &mut DbRef) -> bool {
    stores.run_mode == RunMode::Debug
}
```

Declarations in `default/01_code.loft`:

```loft
// Returns true when running with --production or --release.
pub fn is_production() -> boolean;
// Returns true when running with --debug.
pub fn is_debug() -> boolean;
```

Files: `src/native.rs` (+2 fns), `default/01_code.loft` (+2 declarations).

---

### A2.3 — `--release` flag with zero-overhead assert elision (small–medium)

`--release` implies `--production` AND strips `assert()` and `debug_assert()` from the
bytecode at parse time, so they produce zero instructions.

New `debug_assert(test: boolean, message: text)` mirrors `assert()` but is only ever
emitted in default and debug mode — it is the loft equivalent of Rust's `debug_assert!`.

```
Mode          assert()         debug_assert()
──────────    ─────────────    ──────────────
Default       native call      native call
Debug         native call      native call
Production    native call      native call (logs instead of panics)
Release       elided (Null)    elided (Null)
```

Parser change in `parse_call_diagnostic`:

```rust
if name == "assert" || name == "debug_assert" {
    if self.release_mode {
        *val = Value::Null;
        return Type::Void;
    }
    // ... existing n_assert emit
}
```

Changes:
| File | Change |
|---|---|
| `src/parser/mod.rs` | Add `pub release_mode: bool` to `Parser` |
| `src/parser/control.rs` | `parse_call_diagnostic`: early `Value::Null` when `release_mode`; handle `debug_assert` |
| `src/main.rs` | Parse `--release`; set `run_mode = Release`, `p.release_mode = true`, `production = true`; update help text |
| `default/01_code.loft` | Add `debug_assert(test: boolean, message: text)` declaration |

---

### A2.4 — `--debug` flag with per-type runtime safety logging (medium)

When `stores.run_mode == RunMode::Debug`, the runtime logs warnings for conditions that
are currently silent (null propagation, overflow, OOB returns).

#### Type Safety Rules by Mode

| Type | Condition | Default | Debug | Production | Release |
|---|---|---|---|---|---|
| `integer` | arithmetic overflow | null (silent) | `warn` log | null (silent) | null (silent) |
| `integer` | shift out of [0,64) | null (silent) | `warn` log | null (silent) | null (silent) |
| `float` | NaN result from non-NaN inputs | NaN (silent) | `warn` log | NaN (silent) | NaN (silent) |
| reference | field access on null ref | null (silent) | `warn` log | null (silent) | null (silent) |
| `vector<T>` | index OOB on non-empty vector | null (silent) | `warn` log | null (silent) | null (silent) |
| `text` | slice OOB on non-empty text | `""` (silent) | `warn` log | `""` (silent) | `""` (silent) |
| `assert()` | test fails | panic | panic | error log | elided |
| `debug_assert()` | test fails | panic | panic | error log | elided |
| `panic()` | called | panic | panic | fatal log | fatal log |

All debug-mode warnings carry the current bytecode position, translated to a loft file+line
via the line-number table in `State` (same mechanism that produces error messages).

#### Implementation sketch

The cheapest implementation avoids putting a mode check in every opcode:

1. Add `debug_null_warn(stores, op_name, file, line)` helper in `native.rs`.
2. The operations that can silently null (`op_add_int`, vector index, etc.) are called via
   wrappers in `fill.rs` that check `run_mode == Debug` before calling the helper.  The
   `run_mode` check is a single enum comparison; it is predicted-not-taken in default/release
   mode so branch misprediction cost is negligible.
3. Source location: `State` already has a line-number lookup (`debug_line`); pass
   `state.code_pos` to the helper to resolve file+line.

Files: `src/fill.rs` (wrappers for overflow/null paths), `src/native.rs` (helper),
`src/database/mod.rs` (add `run_mode` field).

---

## Deferred

- **Async / non-blocking writes**: the current design writes synchronously in the worker thread holding the mutex.  For high-throughput use a channel-based background writer thread.
- **Structured (JSON) output**: the plain-text format is sufficient for now.  JSON is a future option for log-aggregation pipelines.
- **Syslog / journald**: forward to system log daemons on Linux (future).
- **`log_warn(msg)` call-site-free form**: a version without the parse special-case (no injected location) could be useful for library code that builds messages dynamically and doesn't want source pinning.

---

## Resolved Design Decisions

1. **`chrono` vs `std::time`** — uses `std::time` only. ISO 8601 timestamps are formatted using the Hinnant calendar algorithm (`days_to_ymd` in `logger.rs`) with no external crate dependency.
2. **Config file name** — `log.conf` (beside the main `.loft` file by default).
3. **Config hot-reload** — `check_reload()` does a `stat()` syscall at most once every 5 s (last-check `Instant` tracked on `Logger`). Acceptable since logging is already I/O-bound.
4. **`log_fatal` in non-production mode** — only writes to the log file at severity `Fatal`. It does not abort. Use `panic()` to abort unconditionally.

---

## See also
- [STDLIB.md](STDLIB.md) — `log_info`, `log_warn`, `log_error`, `log_fatal` user-facing API
- [INTERNALS.md](INTERNALS.md) — `src/logger.rs` implementation details
- [PLANNING.md](PLANNING.md) — A2 (remaining work: hot-reload wiring, is_production(), --release, --debug)
