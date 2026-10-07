// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! Platform-specific helpers shared across the crate.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use std::sync::OnceLock;

/// Process-scoped native-compile timing — a gated singleton.  The expensive
/// native work (cdylib `cargo build`s, per-fixture `rustc`) runs across
/// SPAWNED loft processes, so a threaded object can't span the boundary: each
/// process owns this singleton, subsystems record into it, and every event
/// also emits a `[loft-timing]` stderr line so it survives into the parent /
/// CI log (`ci-probe` aggregates the lines into a per-package breakdown).
///
/// Disabled unless `LOFT_TIMING` is set — [`Timing::global`] returns `None`,
/// so recording is one cached env read and a no-op.
pub struct Timing {
    /// `(kind, name, cache_hit, compile_seconds)`.  Accumulated for an
    /// in-process summary; the per-event stderr line is the cross-process channel.
    events: std::sync::Mutex<Vec<(&'static str, String, bool, Option<f64>)>>,
    /// External build-tool invocations — see [`Timing::record_exec`].
    steps: std::sync::Mutex<Vec<BuildStep>>,
}

/// One external build-tool invocation: WHAT ran, on what, WHY, and for how long.
///
/// The cache events above answer *did we reuse an artifact*; this answers the question a
/// reader actually asks when a build is slow — *which tool is spending the time, and what
/// decided it had to run at all*.  A `reason` is required rather than optional because a step
/// with no stated reason is the thing that makes a slow build unreadable: `rustc` appearing
/// three times says nothing, `rustc bridge rlib (web) — bridge source newer than rlib` says
/// everything.
pub struct BuildStep {
    /// The program spawned: `"cargo"`, `"rustc"`, `"wasm-opt"`, …
    pub tool: &'static str,
    /// What it was run ON — an artifact or crate name, not a full path.
    pub subject: String,
    /// Why it ran: the staleness verdict, or the fact that nothing checks.
    pub reason: String,
    /// Wall-clock seconds the invocation took.
    pub secs: f64,
}

static TIMING: OnceLock<Option<Timing>> = OnceLock::new();

impl Timing {
    /// The process singleton, or `None` when `LOFT_TIMING` is unset.
    #[must_use]
    pub fn global() -> Option<&'static Timing> {
        TIMING
            .get_or_init(|| {
                let on = std::env::var("LOFT_TIMING").is_ok()
                    || std::env::var("LOFT_TIMING_LEDGER").is_ok();
                on.then(|| Timing {
                    events: std::sync::Mutex::new(Vec::new()),
                    steps: std::sync::Mutex::new(Vec::new()),
                })
            })
            .as_ref()
    }

    /// Record a native-compile event.  `kind` is `"cdylib"` or `"fixture"`
    /// (compiles), or `"lockwait"` / `"lockheld"` (the global build-lock — a
    /// `lockwait` with no matching `lockheld` is a process stuck behind
    /// another's build); `secs` is the wall-clock on a cache MISS / the lock
    /// wait duration (`None` on a hit or a lock-wait START).
    ///
    /// Two channels, because the expensive native work runs in SPAWNED loft
    /// processes whose stderr a test harness CAPTURES (visible only on
    /// failure):
    /// - a `[loft-timing]` stderr line (for a direct `loft` invocation), and
    /// - when `LOFT_TIMING_LEDGER=<dir>` is set, an append to
    ///   `<dir>/timing-<pid>.tsv` — a file survives the capture, so the parent
    ///   / CI reads it back (the same side-channel shape as the skip ledger).
    pub fn record(&self, kind: &'static str, name: &str, hit: bool, secs: Option<f64>) {
        let cache = if hit { "hit" } else { "miss" };
        match secs {
            Some(s) => eprintln!("[loft-timing] {kind} {name} cache={cache} secs={s:.2}"),
            None => eprintln!("[loft-timing] {kind} {name} cache={cache}"),
        }
        if let Ok(dir) = std::env::var("LOFT_TIMING_LEDGER")
            && std::fs::create_dir_all(&dir).is_ok()
        {
            use std::io::Write;
            let path =
                std::path::Path::new(&dir).join(format!("timing-{}.tsv", std::process::id()));
            let secs_s = secs.map_or_else(String::new, |s| format!("{s:.2}"));
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{kind}\t{name}\t{cache}\t{secs_s}");
            }
        }
        if let Ok(mut e) = self.events.lock() {
            e.push((kind, name.to_string(), hit, secs));
        }
    }

    /// Record one external build-tool invocation.  Same two channels as
    /// [`Timing::record`] — a `[loft-build]` stderr line so a spawned loft's work reaches the
    /// parent log, and the ledger file when `LOFT_TIMING_LEDGER` is set — plus in-process
    /// accumulation for [`Timing::report`].
    pub fn record_exec(&self, tool: &'static str, subject: &str, reason: &str, secs: f64) {
        eprintln!("[loft-build] {tool} {subject} secs={secs:.2} reason={reason}");
        if let Ok(dir) = std::env::var("LOFT_TIMING_LEDGER")
            && std::fs::create_dir_all(&dir).is_ok()
        {
            use std::io::Write;
            let path =
                std::path::Path::new(&dir).join(format!("timing-{}.tsv", std::process::id()));
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "exec\t{tool}\t{subject}\t{reason}\t{secs:.2}");
            }
        }
        if let Ok(mut st) = self.steps.lock() {
            st.push(BuildStep {
                tool,
                subject: subject.to_string(),
                reason: reason.to_string(),
                secs,
            });
        }
    }

    /// Print the per-invocation breakdown, slowest first.
    ///
    /// The `events` list has been accumulated since this struct existed and never read — the
    /// per-event stderr line was the only channel, which is right for a spawned process and
    /// wrong for a direct `loft --html` run, where the reader wants the shape of the build and
    /// not a transcript of it.  Prints nothing when no step was recorded, so a run that
    /// rebuilt nothing stays silent.
    pub fn report(&self, phase: &str) {
        let Ok(steps) = self.steps.lock() else {
            return;
        };
        if steps.is_empty() {
            return;
        }
        let total: f64 = steps.iter().map(|s| s.secs).sum();
        let mut rows: Vec<&BuildStep> = steps.iter().collect();
        rows.sort_by(|a, b| b.secs.total_cmp(&a.secs));
        let width = rows.iter().map(|r| r.subject.len()).max().unwrap_or(0);
        eprintln!(
            "[loft-build] {phase}: {} external invocation(s), {total:.2}s total",
            rows.len()
        );
        for r in rows {
            eprintln!(
                "[loft-build]   {:>7.2}s  {:<9} {:<width$}  {}",
                r.secs, r.tool, r.subject, r.reason
            );
        }
    }
}

/// Record a native-compile event into the process [`Timing`] singleton — a
/// no-op when `LOFT_TIMING` is unset.  The one call subsystems use, so they
/// don't each repeat the `global()` gate.
pub fn timing_record(kind: &'static str, name: &str, hit: bool, secs: Option<f64>) {
    if let Some(t) = Timing::global() {
        t.record(kind, name, hit, secs);
    }
}

/// Time `run` and record it as an external build-tool invocation — a no-op wrapper when
/// `LOFT_TIMING` is unset, so an uninstrumented build pays one cached env read.
///
/// Takes the reason as a closure-free `&str` because every call site knows it statically or
/// has already computed the staleness verdict it is about to act on.
pub fn timing_exec<T>(
    tool: &'static str,
    subject: &str,
    reason: &str,
    run: impl FnOnce() -> T,
) -> T {
    let Some(t) = Timing::global() else {
        return run();
    };
    let started = std::time::Instant::now();
    let out = run();
    t.record_exec(tool, subject, reason, started.elapsed().as_secs_f64());
    out
}

/// Make the child `cmd` will spawn die with this process, however this process ends (loft#1699).
///
/// A child loft starts is part of the run: a `LOFT_TIMEOUT` exit, the watchdog's abort, an OOM
/// kill or a harness reaping `loft` must not leave it behind.  `rustc` for a program that never
/// finishes compiling ran on at 100 % CPU for twenty minutes after its driver had timed out.
/// `graceful` sends `SIGTERM` (a program with a handler gets to run it) rather than `SIGKILL`.
/// The signal reaches the child when the thread that spawned it exits, so the spawn must wait on
/// the child from that thread (every `status()` / `output()` does).  The child's own children
/// are not covered: a killed `rustc` leaves its linker to finish, and a killed `cargo` its
/// in-flight `rustc` workers — each bounded by one unit of work.  Linux only; elsewhere a no-op.
#[allow(unused_variables)]
pub fn dies_with_driver(cmd: &mut std::process::Command, graceful: bool) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;
        let signal = if graceful {
            libc::SIGTERM
        } else {
            libc::SIGKILL
        };
        let driver = std::process::id() as libc::pid_t;
        // SAFETY: the closure runs in the forked child before `exec` and calls only
        // async-signal-safe `prctl` / `getppid` / `_exit`; it touches no allocator or lock.
        unsafe {
            cmd.pre_exec(move || {
                libc::prctl(libc::PR_SET_PDEATHSIG, signal);
                // The driver may have died before `prctl` armed; a child already handed to
                // another parent must not start.
                if libc::getppid() != driver {
                    libc::_exit(0);
                }
                Ok(())
            });
        }
    }
}

/// Print the external-invocation breakdown for `phase` — a no-op when `LOFT_TIMING` is unset
/// or nothing ran.
pub fn timing_report(phase: &str) {
    if let Some(t) = Timing::global() {
        t.report(phase);
    }
}

/// `true` when the runtime filesystem uses `'\\'` as the path separator (Windows).
/// Initialised once at startup from [`std::path::MAIN_SEPARATOR`].
static WINDOWS_FS: OnceLock<bool> = OnceLock::new();

/// Returns `true` when the runtime filesystem uses `'\\'` (Windows).
pub fn is_windows_fs() -> bool {
    *WINDOWS_FS.get_or_init(|| std::path::MAIN_SEPARATOR == '\\')
}

/// Platform path separator as a `char`: `'\\'` on Windows, `'/'` elsewhere.
/// Use this single token instead of probing for both `'/'` and `'\\'`.
#[must_use]
pub fn sep() -> char {
    if is_windows_fs() { '\\' } else { '/' }
}

/// Platform separator as a `&str`, for use as the replacement in [`str::replace`].
#[must_use]
pub fn sep_str() -> &'static str {
    if is_windows_fs() { "\\" } else { "/" }
}

/// The separator that is *not* native to this platform, as a `&str`.
/// Used to normalise incoming paths that may carry the foreign separator.
#[must_use]
pub fn other_sep() -> &'static str {
    if is_windows_fs() { "/" } else { "\\" }
}

// ---------------------------------------------------------------------------
// tmpfs-overflow safeguards for the native-compile paths.
//
// A native compile writes a static binary (~36MB unstripped) plus rustc/cc
// intermediates into the temp dir.  When that dir is a small RAM-backed tmpfs
// (the Linux default for /tmp), a parallel native run can write several GB —
// which is several GB of *RAM* — and exhaust memory hard enough to hang the
// machine, not merely fail a write.  The knobs below let every native path
// (a) strip the binaries so the footprint is tiny, (b) refuse to start a
// compile that would overflow, reclaiming loft's own stale artefacts first,
// and (c) scale parallelism to the available headroom.  They live here, in a
// pub lib module, so both the binary crate (main.rs, test_runner.rs) and the
// integration tests (tests/native.rs, which link `loft` as a library) share
// one implementation.
// ---------------------------------------------------------------------------

/// Directory for loft's own native-compile scratch — the generated `.rs`
/// source, the compiled binary, and the rustc/cc intermediates.
///
/// Honours the loft-specific `LOFT_TMPDIR` env var so a checkout can keep
/// these multi-MB artefacts off the system temp dir, falling back to the
/// system temp dir when unset.  Deliberately a loft-private knob rather than
/// a `TMPDIR` override, so it relocates only loft's own artefacts — never
/// every other tool's temp.  Created if missing, since rustc and loft both
/// assume the directory exists before writing into it.
#[must_use]
pub fn scratch_dir() -> std::path::PathBuf {
    let dir = scratch_from(std::env::var_os("LOFT_TMPDIR"), std::env::temp_dir());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// [`scratch_dir`]'s choice, apart from the environment: `loft` (the `LOFT_TMPDIR` value) when
/// it names something, else `system` (the temp dir), made ABSOLUTE.  An empty value counts as
/// unset — a shell that expands `LOFT_TMPDIR=$TMPDIR` in the statement that sets `TMPDIR`
/// hands `""` — and so does an empty `TMPDIR`, which `temp_dir` passes through.  A relative or
/// empty scratch put a native binary in the working directory under a bare name, which the
/// run then spawned through `PATH` from the program's directory: "failed to run native binary:
/// No such file or directory".
fn scratch_from(
    loft: Option<std::ffi::OsString>,
    system: std::path::PathBuf,
) -> std::path::PathBuf {
    let dir = loft
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| (!system.as_os_str().is_empty()).then_some(system))
        .unwrap_or_else(|| std::path::PathBuf::from(if cfg!(windows) { "." } else { "/tmp" }));
    std::path::absolute(&dir).unwrap_or(dir)
}

/// A unique per-PROCESS build-scratch directory under [`scratch_dir`], holding
/// one compilation's intermediates (the generated `.rs`, the rustc `.wasm`
/// output + its `*.rcgu.o` objects, bridge rlibs).
///
/// Each `loft` invocation is its own OS process, so keying the directory on the
/// PID isolates concurrent compilations: nextest runs every test in a separate
/// process, and a build script can emit many pages at once. Before this, the
/// `--html` / `--native-wasm` drivers wrote a *shared* `scratch/loft_html.rs`
/// and `-o scratch/loft_html.wasm`, so two parallel runs raced two ways — one
/// rustc read the other's generated source (a page silently embedded the wrong
/// program), and two `rust-lld`s truncated each other's output mid-link
/// (`signal: 7`, SIGBUS). This is the same isolation the `--native` binary path
/// already applies via `loft_native_{pid}`. PID (not a random token) suffices
/// because the racing unit is the *process*, and it keeps the path predictable
/// for `LOFT_KEEP_NATIVE_RS` inspection.
#[must_use]
pub fn build_scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = scratch_dir().join(format!("loft_{tag}_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// This checkout's OWN native test cache under [`scratch_dir`]: `loft_native_cache_<hash>`,
/// the hash naming the owner (the checkout's manifest directory).  Everything inside it was
/// written by this checkout's test harness and nothing else, so a sweep of it can never take
/// another process's work: another checkout has another directory, and the runtime's own
/// per-process artefacts (`loft_native_bin_<pid>`) stay in the parent.  Created if missing.
#[must_use]
pub fn native_cache_dir(owner: &str) -> std::path::PathBuf {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in owner.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let dir = scratch_dir().join(format!("loft_native_cache_{h:016x}"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Drop this checkout's native test caches once the loft build they were compiled against
/// is gone — the automatic removal at the start of a build.  `stamp` names the build (the
/// harness passes the rlib's path and content hash, the two facts its cache key folds in, so
/// a changed stamp means EVERY entry's key would miss); the directory remembers the stamp it
/// was last swept for in `.build`.  A different stamp removes every `loft_native_*` entry of
/// `dir` older than two minutes — a shard of the same run that started earlier has already
/// compiled against the new build, and its entries are younger — then records the stamp.
/// Only `dir` is read, which is what makes the rule safe: the caller passes its own
/// [`native_cache_dir`], never a shared directory.  Answers the bytes freed.
pub fn sweep_own_native_cache(dir: &std::path::Path, stamp: &str) -> u64 {
    let marker = dir.join(".build");
    if std::fs::read_to_string(&marker).is_ok_and(|s| s.trim() == stamp) {
        return 0;
    }
    let mut freed = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !name.to_string_lossy().starts_with("loft_native_") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let fresh = meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() < 120);
            if fresh || !meta.is_file() {
                continue;
            }
            if std::fs::remove_file(entry.path()).is_ok() {
                freed += meta.len();
            }
        }
    }
    // Written last and atomically: a crash between the sweep and the marker only sweeps again.
    let tmp = dir.join(format!(".build.{}", std::process::id()));
    if std::fs::write(&tmp, stamp).is_ok() {
        let _ = std::fs::rename(&tmp, &marker);
    }
    freed
}

/// Make room in this checkout's own native test cache by evicting its least-recently-USED
/// binaries (the harness bumps an entry's mtime on every cache hit) until the filesystem has
/// `want_avail` bytes free or nothing evictable is left.  A binary and its `.key` sidecar go
/// together; the generated `.rs` stays (it is small, and a later compile rewrites it).
/// Entries used in the last ten minutes are kept: a concurrent shard of the same run compiles
/// a chunk and runs it right after, so its pending binaries are that young.  Like
/// [`sweep_own_native_cache`], only `dir` is read — the caller passes its own
/// [`native_cache_dir`].  Answers the bytes freed.
pub fn evict_own_native_cache(dir: &std::path::Path, want_avail: u64) -> u64 {
    use crate::file_access::{self as fa, PathText};
    let Ok(entries) = fa::read_dir(PathText::from_os(dir)) else {
        return 0;
    };
    let mut bins: Vec<(std::time::SystemTime, String, u64)> = entries
        .iter()
        .filter_map(|path| {
            let name = path.parts().last()?;
            if !(name.starts_with("loft_native_") && name.ends_with("_bin")) {
                return None;
            }
            let meta = fa::metadata(path).ok()?;
            let used = meta.modified().ok()?;
            let idle = used.elapsed().ok()?.as_secs() >= 600;
            (fa::is_file(path) && idle).then(|| (used, path.native(), meta.len()))
        })
        .collect();
    bins.sort();
    let mut freed = 0u64;
    for (_, bin, len) in bins {
        if fs_avail_bytes(dir).is_none_or(|a| a >= want_avail) {
            break;
        }
        if fa::remove_file(PathText::host(&bin)).is_ok() {
            freed += len;
            let _ = fa::remove_file(PathText::host(&format!("{bin}.key")));
        }
    }
    freed
}

/// Bytes currently available on the filesystem backing `path`.
///
/// Uses `df -P -k` (POSIX output → guaranteed single, unwrapped data row) and
/// parses the Available column.  Returns `None` when `df` is missing or its
/// output is unparseable; callers treat `None` as "unknown — don't block", so
/// a missing `df` degrades to the pre-safeguard behaviour rather than refusing
/// to run.
#[must_use]
pub fn fs_avail_bytes(path: &std::path::Path) -> Option<u64> {
    let out = std::process::Command::new("df")
        .arg("-P")
        .arg("-k")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Line 0 = header, line 1 = the data row.  Columns (POSIX):
    //   Filesystem  1024-blocks  Used  Available  Capacity  Mounted-on
    let avail_kb: u64 = text
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;
    Some(avail_kb.saturating_mul(1024))
}

/// Opt-in native-compile timing (`LOFT_TIMING=1`).  OFF by default — a single
/// cached env read, zero cost when unset.  When set, the native-compile sites
/// (cdylib `auto_build_native`, the test runner's per-fixture rustc) emit one
/// `[loft-timing] …` line per event — cache hit vs miss, and the compile
/// wall-clock on a miss — to stderr.  `ci-probe` (and any local run)
/// aggregates these into a where-did-the-time-go view inside loft that the
/// CI step-level timing cannot see.
#[must_use]
pub fn timing_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_TIMING").is_ok())
}

/// Minimum free space (bytes) a native-compile path insists on before
/// spawning a `rustc` that writes into the temp filesystem.  Default 512MB;
/// override with `LOFT_TMPFS_MIN_FREE_MB` for unusual setups.
#[must_use]
pub fn tmpfs_min_free_bytes() -> u64 {
    const DEFAULT_MB: u64 = 512;
    let mb = std::env::var("LOFT_TMPFS_MIN_FREE_MB")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_MB);
    mb.saturating_mul(1024 * 1024)
}

/// The process id embedded in a `loft_native_<pid>.rs` /
/// `loft_native_bin_<pid>` / `loft_test_native_<pid>*` scratch name — the
/// trailing digit run.  `None` when the name carries no parseable pid.
fn scratch_owner_pid(name: &str) -> Option<u32> {
    let stem = name.split('.').next().unwrap_or(name);
    stem.rsplit('_').next()?.parse().ok()
}
/// The pid of the process that owns `name`, for the two shapes the RUNTIME itself writes
/// per process and nothing else: `loft_native_bin_<pid>` and `loft_native_<pid>.rs`, the
/// pid being the whole of what follows the prefix.  `None` for every other name — the
/// native test suite names its files by script STEM (`loft_native_<stem>.rs`,
/// `loft_native_<stem>_<pid>_bin`), and a stem that ends in digits
/// (`discard_slot_per_type_795`) read as a dead pid to the looser
/// [`scratch_owner_pid`], so a worker's compile swept a sibling's live source out from
/// under its rustc.  A name this cannot claim is judged by age alone, and only under low
/// space.
fn runtime_scratch_pid(name: &str) -> Option<u32> {
    let digits = if let Some(rest) = name.strip_prefix("loft_native_bin_") {
        // ⚠ The MSVC linker writes `<binary>.pdb` beside the executable, so the binary shape
        // has a companion whose name is `loft_native_bin_<pid>.pdb`.  Without stripping that
        // suffix the digit test below fails on `1644.pdb`, the name is claimed by nothing, and
        // it falls to the age rule — surviving the hour that the binary it belongs to does not.
        // Measured on the Windows daily (`native_scratch_hygiene`).  Stripping only this ONE
        // known suffix, and only on the `bin_` shape, keeps the looser
        // `scratch_owner_pid` hazard out: a script STEM ending in digits must still not read
        // as a pid, which is why this does not simply split on '.'.
        rest.strip_suffix(".pdb").unwrap_or(rest)
    } else {
        name.strip_prefix("loft_native_")?.strip_suffix(".rs")?
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Whether `pid` is a live process — `Some(true/false)` on Linux (procfs),
/// `None` (unknown) elsewhere.
// `Option` is the cross-platform contract, not a unix one: the unix arm can always
// decide, and the non-unix arm never can.  Clippy sees only the arm it compiles.
#[cfg_attr(unix, allow(clippy::unnecessary_wraps))]
fn pid_alive(pid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        // A value that does not fit a POSITIVE `pid_t` names no process, and must never
        // reach `kill`: the cast would make it negative, and a negative pid addresses a
        // process GROUP — so `u32::MAX - 1` would ask about group 2 and could answer
        // "alive" for a process that cannot exist.
        let Ok(p) = i32::try_from(pid) else {
            return Some(false);
        };
        if p <= 0 {
            return Some(false);
        }
        // Signal 0 sends nothing; it only asks whether the pid exists.  ESRCH proves it
        // does not, EPERM proves it does and belongs to someone else, success proves it
        // does.  This is decidable on every unix, where `/proc` is Linux-only — so the
        // dead-only sweep reclaims on macOS instead of falling through to the age
        // fallback there.
        // SAFETY: `kill` with signal 0 delivers nothing and touches no memory; `p` is a
        // plain positive integer, and the call's only effect is its return value.
        if unsafe { libc::kill(p, 0) } == 0 {
            return Some(true);
        }
        Some(std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH))
    }
    // Windows has no `kill(pid, 0)`, but a process HANDLE answers the same question.
    //
    // The safety DIRECTION matters more here than completeness: the dead-only sweep
    // DELETES what this calls dead, and a sweep destroying the `.rs` a live compile had
    // just emitted is the exact failure the pid check exists to prevent (see the module
    // doc above).  So anything unrecognised answers `None` — unknowable, fall back to the
    // age rule — and never `Some(false)`.  Being slow to reclaim costs disk; being wrong
    // costs the build.
    // Windows has no `kill(pid, 0)`, but a process HANDLE answers the same question.
    //
    // The safety DIRECTION matters more here than precision: the dead-only sweep DELETES
    // what this calls dead, and a sweep destroying the `.rs` a live compile had just
    // emitted is the exact failure the pid check exists to prevent (see the module doc
    // above).  So anything unrecognised answers `None` — unknowable, fall back to the age
    // rule — never `Some(false)`.  Being slow to reclaim costs disk; being wrong costs the
    // build.
    //
    // Measured on windows-latest (windows-probe.yml, 2026-09-10) rather than assumed, and
    // the measurement overturned the first attempt.  `WaitForSingleObject` is the obvious
    // spelling and it returned WAIT_FAILED (0xffffffff) for EVERY openable process, live or
    // dead, because `PROCESS_QUERY_LIMITED_INFORMATION` does not grant SYNCHRONIZE — an arm
    // that compiled, looked right, and answered `None` for everything.  `GetExitCodeProcess`
    // needs no extra right.  What the runner reported:
    //
    //     own process / pid 4 (System) / a live child   exit_code = 259 (STILL_ACTIVE)
    //     a child that exited with 7                    exit_code = 7   (handle still opens)
    //     pid 0, u32::MAX-1, an unused 999999           OpenProcess = NULL, err 87
    //
    // So pid 0 needs no special case here the way it does on unix, where 0 addresses a
    // process GROUP: Windows simply reports it as no process.  The STILL_ACTIVE ambiguity is
    // real and deliberately accepted — a process that exits WITH code 259 reads as alive —
    // because it errs toward not reclaiming, and loft's own exit codes are single digits.
    #[cfg(windows)]
    {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const ERROR_ACCESS_DENIED: u32 = 5;
        const ERROR_INVALID_PARAMETER: u32 = 87;
        const STILL_ACTIVE: u32 = 259;
        unsafe extern "system" {
            fn OpenProcess(desired: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
            fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
            fn GetExitCodeProcess(handle: *mut core::ffi::c_void, code: *mut u32) -> i32;
            fn GetLastError() -> u32;
        }
        // SAFETY: `OpenProcess` takes three integers by value and returns a handle or null,
        // touching no memory this process owns.  Every path that obtains a handle closes it
        // exactly once.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // SAFETY: reads the calling thread's last-error value; no arguments, no memory.
            return match unsafe { GetLastError() } {
                // The process exists and simply is not ours to open — the EPERM arm.
                ERROR_ACCESS_DENIED => Some(true),
                // No process carries this id.
                ERROR_INVALID_PARAMETER => Some(false),
                _ => None,
            };
        }
        let mut code: u32 = 0;
        // SAFETY: `handle` is a live process handle from the call above and `code` is a
        // valid, initialised u32 this frame owns for the duration of the call.
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
        // Read before the close, which would clobber the thread's last error.
        // SAFETY: as above — no arguments, no memory.
        let err = unsafe { GetLastError() };
        // SAFETY: closing a handle this function opened, exactly once.
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            let _ = err;
            return None;
        }
        return Some(code == STILL_ACTIVE);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

/// Delete loft's leftover native-compile artefacts from `dir` to reclaim
/// space, returning the bytes freed.  Only removes files matching loft's own
/// `loft_native_*` / `loft_test_native_*` naming — never another program's
/// temp files — and only files that are provably STALE:
///
/// - a file whose embedded pid is THIS process or a still-running one is
///   skipped — the original sweep deleted the `.rs` the current compile had
///   just emitted (its own pid matched the prefix), so under CI disk
///   pressure every space-constrained compile destroyed itself ("couldn't
///   read loft_native_<pid>.rs") and could equally race a parallel test's
///   in-flight file;
/// - when liveness is unknowable (no pid in the name, or a non-unix host), only
///   files older than an hour are deleted.
///
/// Those files ARE the binary cache, so this is called only when a path is
/// already space-constrained; the next run recompiles.
pub fn reclaim_native_scratch(dir: &std::path::Path) -> u64 {
    reclaim_native_scratch_by(dir, true)
}

/// The sweep every native compile runs first, silently: the artefacts of processes that are
/// PROVABLY dead — a `loft_native_bin_<pid>` or `loft_native_<pid>.rs` whose pid no longer
/// exists.  A run that ends normally removes its own binary; one killed from outside (a
/// `timeout` wrapper, a harness kill, Ctrl-C) cannot, and with nothing else ever looking at
/// the directory those were accumulating one per killed process, ten megabytes each, until
/// the disk was full (sixteen thousand of them on one box).  Bounded by construction: after
/// this, the directory holds at most one artefact per LIVE process.  The age fallback of
/// [`reclaim_native_scratch`] is deliberately NOT applied here — a name without a pid is the
/// test runner's per-program binary cache (`loft_test_native_<stem>_<key>_bin`), which is a cache only
/// as long as it survives a compile that has room.
pub fn reclaim_dead_native_scratch(dir: &std::path::Path) -> u64 {
    reclaim_native_scratch_by(dir, false)
}

fn reclaim_native_scratch_by(dir: &std::path::Path, aged_too: bool) -> u64 {
    let own_pid = std::process::id();
    let mut freed = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("loft_native_") || name.starts_with("loft_test_native_")) {
            continue;
        }
        // Provably stale only through the strict shapes; the looser parse below serves the
        // age fallback's "is anyone alive behind this name" question and nothing else.
        let proven_stale = match runtime_scratch_pid(&name) {
            Some(p) if p == own_pid => false,
            Some(p) => pid_alive(p) == Some(false),
            None => false,
        };
        let pid = scratch_owner_pid(&name);
        if !proven_stale {
            if !aged_too {
                continue;
            }
            // Liveness unknown (no pid in the name, or a non-unix host where a pid
            // cannot be probed) — fall back to age: anything under an hour old may be an
            // in-flight emission of a parallel process.
            if pid.is_some_and(|p| p == own_pid || pid_alive(p) == Some(true)) {
                continue;
            }
            let old_enough = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 3600);
            if !old_enough {
                continue;
            }
        }
        let len = entry.metadata().map_or(0, |m| m.len());
        if std::fs::remove_file(entry.path()).is_ok() {
            freed = freed.saturating_add(len);
        }
    }
    freed
}

/// Preflight check for a single native compile writing into `scratch`.
/// Returns `true` when there is enough headroom to proceed.  Every call first sweeps the
/// artefacts of dead processes ([`reclaim_dead_native_scratch`], silent), so the scratch
/// directory stays bounded whatever killed the runs before this one.  When space is still
/// below [`tmpfs_min_free_bytes`], the aged artefacts go too (the test runner's per-file
/// binary cache included) and the space is re-checked; only if it is *still* low does it
/// return `false` (callers skip that compile with a warning rather than risk overflowing RAM).
pub fn native_compile_space_ok(scratch: &std::path::Path) -> bool {
    let _ = reclaim_dead_native_scratch(scratch);
    let floor = tmpfs_min_free_bytes();
    match fs_avail_bytes(scratch) {
        // df unavailable → unknown → don't block (preserve old behaviour).
        None => true,
        Some(avail) if avail >= floor => true,
        Some(_) => {
            let freed = reclaim_native_scratch(scratch);
            if freed > 0 {
                eprintln!(
                    "loft: low space in {} — reclaimed {} MB of stale native artefacts",
                    scratch.display(),
                    freed / (1024 * 1024)
                );
            }
            fs_avail_bytes(scratch).is_none_or(|a| a >= floor)
        }
    }
}

/// Should native-compile binaries be stripped of symbols?  Stripping cuts
/// each binary from ~36MB to ~1MB (the bulk is debug info pulled in from
/// `libloft.rlib` + std, useless to a run-and-check test).  Set
/// `LOFT_NATIVE_KEEP_SYMBOLS=1` to keep symbols when debugging a native
/// crash; the generated `.rs` is always retained, so a single fixture can be
/// recompiled with `-g` on demand.
#[must_use]
pub fn native_strip_symbols() -> bool {
    std::env::var_os("LOFT_NATIVE_KEEP_SYMBOLS").is_none()
}

/// Worker-thread count for a parallel native run of `job_count` jobs in
/// `scratch`: the CPU parallelism, capped by the job count and by how many
/// concurrent compiles the temp filesystem can hold.  `reserve_per_worker` is
/// the peak temp footprint of one in-flight compile (callers pass a larger
/// value when binaries are unstripped).  On a roomy disk this never clamps; on
/// a tight tmpfs it scales down so the run finishes instead of hanging.
///
/// Used only by the parallel test suite (`tests/native.rs`); the binary's own
/// native paths compile serially, so it reads as dead in the binary view of
/// this dual lib+bin module.  The suppression goes away once the lib/bin
/// double compilation is removed — see PERFORMANCE-history.md § Design: BUILD1.
#[must_use]
#[allow(dead_code)]
pub fn native_worker_count(
    cpu_max: usize,
    job_count: usize,
    scratch: &std::path::Path,
    reserve_per_worker: u64,
) -> usize {
    let mem_cap = match fs_avail_bytes(scratch) {
        Some(avail) if reserve_per_worker > 0 => (avail / reserve_per_worker).max(1) as usize,
        _ => usize::MAX,
    };
    cpu_max.min(job_count).min(mem_cap).max(1)
}

#[cfg(test)]
mod reclaim_tests {
    use super::*;

    /// `scratch_from`: an empty `LOFT_TMPDIR` or `TMPDIR` is unset, and every answer is absolute
    /// — the two ways a native binary ended up spawned under a bare name through `PATH`.
    #[test]
    fn the_scratch_dir_is_never_empty_or_relative() {
        // Absolute on every platform: on Windows `/x/y` has no drive, so it is relative and
        // rightly resolved against the working directory — the system default too.
        let sys = std::env::temp_dir().join("sys");
        let abs = std::env::temp_dir().join("x").join("y");
        assert_eq!(scratch_from(Some(abs.clone().into()), sys.clone()), abs);
        assert_eq!(scratch_from(Some("".into()), sys.clone()), sys);
        assert_eq!(scratch_from(None, sys.clone()), sys);
        let fallback = scratch_from(Some("".into()), std::path::PathBuf::new());
        assert!(
            fallback.is_absolute() && !fallback.as_os_str().is_empty(),
            "{fallback:?}"
        );
        let rel = scratch_from(Some("rel/dir".into()), sys);
        assert!(rel.is_absolute() && rel.ends_with("rel/dir"), "{rel:?}");
    }

    /// `sweep_own_native_cache`: a changed build stamp removes the directory's `loft_native_*`
    /// entries older than two minutes and keeps a fresh one (a concurrent shard's) and every
    /// foreign name; the same stamp removes nothing; the marker records the stamp.
    #[test]
    fn a_new_build_sweeps_only_the_old_entries_of_its_own_cache() {
        let dir =
            std::env::temp_dir().join(format!("loft_native_cache_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = |name: &str| {
            let p = dir.join(name);
            std::fs::write(&p, b"x").unwrap();
            let t = std::time::SystemTime::now() - std::time::Duration::from_hours(1);
            // Open for WRITE before set_modified: Windows `SetFileTime` needs write access
            // (Unix `futimens` works on a read-only fd), as `cache::touch_now` does.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
            p
        };
        let stale_bin = old("loft_native_a_bin");
        let stale_key = old("loft_native_a_bin.key");
        let foreign = old("other_tool_output");
        let fresh = dir.join("loft_native_b_bin");
        std::fs::write(&fresh, b"y").unwrap();
        std::fs::write(dir.join(".build"), "old-build").unwrap();

        let freed = sweep_own_native_cache(&dir, "new-build");
        assert_eq!(freed, 2, "the two stale entries, one byte each");
        assert!(
            !stale_bin.exists() && !stale_key.exists(),
            "the older build's entries go"
        );
        assert!(
            fresh.exists(),
            "a fresh entry is a concurrent shard's and stays"
        );
        assert!(
            foreign.exists(),
            "a name that is not the harness's is never touched"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(".build")).unwrap(),
            "new-build"
        );

        let again = old("loft_native_c_bin");
        assert_eq!(
            sweep_own_native_cache(&dir, "new-build"),
            0,
            "the same build sweeps nothing"
        );
        assert!(again.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `evict_own_native_cache`: under pressure it takes the idle binaries with their keys,
    /// and leaves a recently used binary (a concurrent shard's pending one), the generated
    /// `.rs` and every foreign name; with room enough it takes nothing.
    #[test]
    fn eviction_takes_idle_binaries_and_their_keys_only() {
        use crate::file_access::{self as fa, PathText};
        let at = |p: &std::path::Path| PathText::from_os(p);
        let dir =
            std::env::temp_dir().join(format!("loft_native_evict_test_{}", std::process::id()));
        let _ = fa::remove_dir_all(at(&dir));
        fa::create_dir_all(at(&dir)).unwrap();
        let old = |name: &str| {
            let p = dir.join(name);
            fa::write(at(&p), b"x").unwrap();
            let t = std::time::SystemTime::now() - std::time::Duration::from_hours(1);
            // `set_modified` takes the handle each platform needs (write on Windows).
            fa::set_modified(at(&p), t).unwrap();
            p
        };
        let there = |p: &std::path::Path| fa::exists(at(p));
        let idle_bin = old("loft_native_a_bin");
        let idle_key = old("loft_native_a_bin.key");
        let source = old("loft_native_a.rs");
        let foreign = old("other_tool_output");
        let recent = dir.join("loft_native_b_bin");
        fa::write(at(&recent), b"y").unwrap();

        assert_eq!(
            evict_own_native_cache(&dir, 0),
            0,
            "room enough: nothing goes"
        );
        assert!(there(&idle_bin));

        assert_eq!(
            evict_own_native_cache(&dir, u64::MAX),
            1,
            "the one idle binary"
        );
        assert!(
            !there(&idle_bin) && !there(&idle_key),
            "an idle binary goes with its key"
        );
        assert!(there(&recent), "a recently used binary stays");
        assert!(
            there(&source) && there(&foreign),
            "sources and foreign names stay"
        );
        let _ = fa::remove_dir_all(at(&dir));
    }

    /// `pid_alive` answers the same three ways on every unix, which is what makes the
    /// dead-only sweep decidable off Linux.  The out-of-range case is the load-bearing
    /// one: a value too large for a positive `pid_t` names no process and must answer
    /// DEAD without reaching `kill`, where the cast would address a process group.
    #[test]
    fn pid_liveness_is_decidable_and_never_asks_about_a_group() {
        assert_eq!(
            pid_alive(std::process::id()),
            Some(true),
            "this process is alive"
        );
        assert_eq!(
            pid_alive(u32::MAX - 1),
            Some(false),
            "a pid past pid_t names no process, and must not become group 2"
        );
        assert_eq!(
            pid_alive(0),
            Some(false),
            "pid 0 addresses a group, not a process"
        );
        // pid 1 exists on every unix and is not ours: the EPERM arm, which must read
        // ALIVE rather than dead.
        #[cfg(unix)]
        assert_eq!(
            pid_alive(1),
            Some(true),
            "pid 1 exists whether or not we may signal it"
        );
    }

    /// The debug-symbol companion is the runtime's own file and has to be claimable, or it
    /// outlives the binary it belongs to.  `pdb` appeared nowhere in `src/` before this, so
    /// the omission was total rather than partial — and unix emits no such file, which is why
    /// only the Windows daily could see it.
    #[test]
    fn the_binarys_debug_symbol_companion_is_claimed_like_the_binary() {
        assert_eq!(runtime_scratch_pid("loft_native_bin_1644"), Some(1644));
        assert_eq!(runtime_scratch_pid("loft_native_bin_1644.pdb"), Some(1644));
        // The looser hazard stays out: a script STEM ending in digits is NOT a pid, whatever
        // extension it carries, or a worker's compile sweeps a sibling's live source away.
        assert_eq!(
            runtime_scratch_pid("loft_native_discard_slot_per_type_795.rs"),
            None
        );
        assert_eq!(runtime_scratch_pid("loft_native_bin_notapid"), None);
        assert_eq!(runtime_scratch_pid("loft_native_bin_notapid.pdb"), None);
        // And only THAT suffix — an unknown one must not be silently accepted.
        assert_eq!(runtime_scratch_pid("loft_native_bin_1644.exe"), None);
    }

    #[test]
    fn reclaim_spares_live_and_fresh_files() {
        let dir = std::env::temp_dir().join(format!("loft_reclaim_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let own = dir.join(format!("loft_native_{}.rs", std::process::id()));
        // u32::MAX-1 — no real pid (Linux pid_max caps far below); provably dead.
        let dead = dir.join("loft_native_4294967294.rs");
        let fresh_no_pid = dir.join("loft_native_bin_notapid");
        // The native suite's stem-named source, whose stem happens to end in digits: not a
        // pid, and not the runtime's to sweep however dead "795" is.
        let stem_named = dir.join("loft_native_discard_slot_per_type_795.rs");
        std::fs::write(&own, "live").unwrap();
        std::fs::write(&dead, "stale").unwrap();
        std::fs::write(&fresh_no_pid, "fresh").unwrap();
        std::fs::write(&stem_named, "live source of a sibling worker").unwrap();
        // The dead-only sweep (every compile): the dead pid goes, the fresh no-pid entry
        // stays whatever its age — it is the test runner's cache, not a leftover.
        //
        // WHICH FILES GO is the sweep's contract; the byte count is a proxy for it, and the
        // two fail for opposite reasons — so the contract is asserted first and the proxy
        // carries the contract's answer in its message.  Asserted the other way round, a
        // reclaim that WORKED and a reclaim that did nothing both read
        // "the dead-only sweep must reclaim the dead-pid file", and the macOS ASan leg of
        // loft#1406 has been failing on exactly that line with no way to tell which it is.
        // A proxy can read 0 for a working sweep: `reclaim_native_scratch_by` takes the
        // length from `entry.metadata()` BEFORE removing, and a failed `metadata()` removes
        // the file and adds zero.
        // loft#1406's macOS leg fails HERE and has never been reproduced off a CI runner, so
        // each question costs a round trip of a day.  The assertions therefore carry their own
        // evidence: the sweep's two DECISION INPUTS for the dead name, and what the directory
        // actually holds afterwards.  Both are decidable by reading on Linux — `4294967294`
        // parses, and `pid_alive` refuses it before `kill` because it would become a negative
        // process-group id — so a macOS run that disagrees names its own cause instead of
        // leaving the next reader another hypothesis.
        let evidence = |freed: u64| {
            let mut names: Vec<String> = std::fs::read_dir(&dir).map_or_else(
                |e| vec![format!("<read_dir failed: {e}>")],
                |es| {
                    es.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                },
            );
            names.sort();
            format!(
                "freed {freed} bytes; runtime_scratch_pid = {:?}, pid_alive = {:?}; {} holds [{}]",
                runtime_scratch_pid("loft_native_4294967294.rs"),
                pid_alive(4_294_967_294),
                dir.display(),
                names.join(", ")
            )
        };
        let dead_only = reclaim_dead_native_scratch(&dir);
        assert!(
            !dead.exists(),
            "dead-pid file must go in the dead-only sweep ({})",
            evidence(dead_only)
        );
        assert!(
            own.exists() && fresh_no_pid.exists(),
            "own-pid and no-pid entries survive the dead-only sweep"
        );
        assert!(
            stem_named.exists(),
            "a stem-named suite file survives the dead-only sweep"
        );
        // The byte count is asserted where it is DETERMINATE, which is the same platform
        // the exact-count assertion below already restricts itself to.  Reaching here with
        // `dead_only == 0` on another platform means the file went and the accounting did
        // not follow — a different defect from the file staying, and one this ordering
        // now reports as itself.
        if cfg!(target_os = "linux") {
            assert!(
                dead_only > 0,
                "the dead-only sweep removed the dead-pid file but accounted no bytes for it ({})",
                evidence(dead_only)
            );
        }
        std::fs::write(&dead, "stale").unwrap();
        let freed = reclaim_native_scratch(&dir);
        assert!(own.exists(), "own-pid file must survive the reclaim");
        assert!(
            fresh_no_pid.exists(),
            "a fresh file without a parseable pid must survive (age floor)"
        );
        if cfg!(target_os = "linux") {
            assert!(!dead.exists(), "dead-pid file must be reclaimed");
            assert_eq!(freed, 5);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Which naming convention [`lib_variants`] should translate into.
///
/// A parameter rather than a `cfg!` inside the translation, so the macOS and
/// Windows spellings are checkable from any machine. Left implicit, each one is
/// only ever exercised on its own platform — and this whole fallback exists
/// because a platform nobody could run locally had been broken for a while.
#[derive(Clone, Copy, PartialEq)]
pub enum LibOs {
    Linux,
    Macos,
    Windows,
}

/// The declared `[c]` library name, then the same library spelled for THIS host.
///
/// A `[c] libs` entry is one string in a manifest, so it cannot say `.so` on
/// Linux and `.dylib` on macOS. Rather than invent a per-platform manifest key,
/// translate at use time: the declared spelling is authoritative and always
/// tried first, and these are what to try when the host does not use it.
///
/// Shared by the two places that resolve such a name — the interpreter's
/// `dlopen` (`extensions::load_c_library`) and the `--native` LINK line
/// (`native_utils::add_c_library_flags`). They must agree: a declaration that
/// loads under `--interpret` and fails to link under `--native` is the backend
/// divergence @PLN24 keeps refusing to ship, and giving each its own copy of
/// the spelling rules is how that happens.
pub fn host_lib_variants(name: &str) -> Vec<String> {
    lib_variants(name, host_lib_os())
}

/// Can a `[c]` library declared as `name` actually be LOADED on this host?
///
/// The question a conditional test has to ask before it decides to skip, and it
/// has to be asked the way the dynamic linker would ask it. Two things make the
/// obvious alternative — look for the file — wrong:
///
/// - **The spelling is the host's, not the declaration's.** A manifest carries
///   one string, `libsqlite3.so.0`, so a search for that name finds nothing on a
///   macOS box holding `libsqlite3.dylib` or a Windows one holding `sqlite3.dll`
///   — and a conditional cell then skips in SILENCE on every platform but the
///   one the string was written for.
/// - **On macOS the file need not exist at all.** System libraries live in the
///   dyld shared cache, so `/usr/lib/libsqlite3.dylib` is absent from the
///   filesystem while `dlopen` of that exact name succeeds. Anything built on
///   `Path::exists` reports "not installed" for a library that works.
///
/// So this opens it. `dlopen` consults the shared cache, `LD_LIBRARY_PATH` /
/// `DYLD_LIBRARY_PATH`, the `PATH` DLL search and the system directories — every
/// route the real call will take — which is what makes a `true` here mean the
/// same thing as a `true` at the call site.
///
/// Deliberately NOT `c_call::library_available`: that additionally requires every
/// `#c` symbol attributed to the library to resolve, and a `#c` annotation never
/// names the library it came from, so a package that declares a library AND
/// builds a shim has both sets attributed to the one name. This answers only the
/// question it is asked.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn host_library_loadable(name: &str) -> bool {
    host_lib_variants(name)
        .iter()
        // SAFETY: loading a library runs its initialisers, which is exactly what
        // asking "can this be loaded" means. The handle drops immediately.
        .any(|n| unsafe { libloading::Library::new(n) }.is_ok())
}

/// Which naming convention THIS host uses. The single `cfg!` read, so the
/// translation itself stays a pure function of its argument.
#[must_use]
pub fn host_lib_os() -> LibOs {
    if cfg!(target_os = "macos") {
        LibOs::Macos
    } else if cfg!(windows) {
        LibOs::Windows
    } else {
        LibOs::Linux
    }
}

/// The `[c]` library that is really present beside `dir`, under whichever
/// spelling this host uses. `None` when nothing is there — a bare soname the
/// dynamic linker resolves, or a library that is simply missing.
///
/// The link line needs this as much as the loader does: a package that ships
/// its own library is declared as a PATH, and reading only the declared
/// spelling meant macOS found nothing beside the package, emitted no `-L` and
/// no rpath, and sent `-l dylib=<stem>` to the system search path where the
/// library is not.
#[must_use]
pub fn existing_lib_beside(dir: &std::path::Path, name: &str, os: LibOs) -> Option<String> {
    lib_variants(name, os)
        .into_iter()
        .find(|cand| dir.join(cand).exists())
}

/// Linker flags that pin a freshly built shared library's own name.
///
/// macOS records an INSTALL NAME inside the Mach-O — whatever `-o` said — and
/// every binary that links the library copies that string in as the thing to ask
/// `dyld` for. loft builds a shim to `<stem>.<pid>.tmp` and renames it over the
/// final name so the publish is atomic; the rename moves the FILE and leaves the
/// recorded name alone, so consumers went looking for a `.tmp` that no longer
/// existed:
///
/// ```text
/// dyld: Library not loaded: …/native-auto/lcshim_shim_<key>.71616.tmp
/// ```
///
/// `@rpath/<final name>` rather than the absolute path: loft already emits
/// `-Wl,-rpath,<dir>` for the shim directory, and a relocatable name keeps a
/// built binary working if the package moves. ELF has no equivalent problem —
/// its `SONAME` is only set when asked — so this is empty off macOS.
///
/// **Every artifact loft publishes by rename needs this**, not only the `cc`-built
/// shim: `native_lib` compiles a package cdylib to `<stem>.building` and renames
/// it the same way. That one went years recording
/// `/Users/…/native-auto/loft_auto_<hash>.building` — the temp stem with an
/// absolute build directory in it — which stayed invisible because those cdylibs
/// are `dlopen`ed BY PATH, and a path load ignores the install name. It would have
/// surfaced the first time one was resolved through `@rpath` or simply moved. The
/// flag is cheap and the failure is silent, so it goes on both.
#[must_use]
pub fn install_name_args(final_file_name: &str, os: LibOs) -> Vec<String> {
    if os == LibOs::Macos {
        vec![format!("-Wl,-install_name,@rpath/{final_file_name}")]
    } else {
        Vec::new()
    }
}

/// Windows only: also emit an IMPORT LIBRARY, at `implib_path`.
///
/// A `.dll` is not linkable by itself. MSVC `link.exe` links against the `.lib`
/// that describes it, and a MinGW `cc` given `-shared` produces the DLL and
/// nothing else unless asked — so the Windows leg died on
///
/// ```text
/// LINK : fatal error LNK1181: cannot open input file 'sqlite_shim_<key>.lib'
/// ```
///
/// asking for a file that was never going to exist. The name it asks for is
/// fixed by `native_utils::add_c_library_flags`, which passes `-l <stem>` for
/// the shim, so the import library must be `<stem>.lib` beside the DLL.
///
/// Separate from [`install_name_args`] because the two do different things: that
/// one records a name INSIDE the artifact, this one produces a SECOND artifact.
/// A path rather than a bare name, because the compiler's working directory is
/// not the output directory — and the caller passes a temporary, since the
/// import library has to be published by the same atomic rename as the DLL.
#[must_use]
pub fn shim_implib_args(implib_path: &str, os: LibOs) -> Vec<String> {
    if os == LibOs::Windows {
        vec![format!("-Wl,--out-implib,{implib_path}")]
    } else {
        Vec::new()
    }
}

/// The import library that goes with a shim named `final_file_name`, or `None`
/// off Windows. `sqlite_shim_<key>.dll` → `sqlite_shim_<key>.lib`.
#[must_use]
pub fn shim_implib_name(final_file_name: &str, os: LibOs) -> Option<String> {
    if os != LibOs::Windows {
        return None;
    }
    let stem = final_file_name
        .strip_suffix(".dll")
        .unwrap_or(final_file_name);
    Some(format!("{stem}.lib"))
}

/// [`host_lib_variants`], with the target convention passed in.
///
/// The versioned forms matter as much as the bare one — a real declaration is
/// `libmariadb.so.3`, whose macOS twin is `libmariadb.3.dylib` (the soversion
/// moves BEFORE the extension) and whose Windows twin drops both the `lib`
/// prefix and the version. Returns just the declared name on Linux, where the
/// spelling already is the host's.
pub fn lib_variants(name: &str, os: LibOs) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if os == LibOs::Linux {
        return out; // the declared spelling already is this host's
    }
    // Split a trailing directory off so the translation only rewrites the file
    // name — `../../liblc_types.so` must stay relative to the same place.
    let (dir, file) = match name.rfind(['/', '\\']) {
        Some(i) => (&name[..=i], &name[i + 1..]),
        None => ("", name),
    };
    // `libfoo.so.3` → stem `libfoo`, version `3`; `libfoo.so` → no version.
    let Some((stem, rest)) = file.split_once(".so") else {
        return out; // not a Linux spelling — nothing to translate
    };
    let version = rest.strip_prefix('.').filter(|v| !v.is_empty());
    let mut push = |f: String| out.push(format!("{dir}{f}"));
    if os == LibOs::Macos {
        if let Some(v) = version {
            push(format!("{stem}.{v}.dylib"));
        }
        push(format!("{stem}.dylib"));
    } else {
        // No `lib` prefix and no soversion in a DLL name; try both spellings
        // because a MinGW-built library keeps the prefix.
        let bare = stem.strip_prefix("lib").unwrap_or(stem);
        push(format!("{bare}.dll"));
        push(format!("{stem}.dll"));
    }
    out
}

#[cfg(test)]
mod shim_name_tests {
    use super::{LibOs, install_name_args, shim_implib_args, shim_implib_name};

    /// A `.dll` is not linkable on its own — MSVC links the `.lib` beside it,
    /// and `cc -shared` writes one only when asked. The name is not free to
    /// choose: `add_c_library_flags` passes `-l <stem>` for the shim, so the
    /// import library must be exactly `<stem>.lib`, which is what this pins.
    ///
    /// Checked from any host by passing the convention in, for the same reason
    /// the install-name test is: left implicit it could only ever run on
    /// Windows, and a `#c` package had been unlinkable there the whole time.
    #[test]
    fn windows_asks_for_an_import_library_and_other_hosts_do_not() {
        assert_eq!(
            shim_implib_name("sqlite_shim_5aed31d6bafbf9f8.dll", LibOs::Windows)
                .expect("Windows needs an import library"),
            "sqlite_shim_5aed31d6bafbf9f8.lib",
            "the DLL's stem — the exact name `-l sqlite_shim_<key>` makes link.exe open"
        );
        assert_eq!(
            shim_implib_args("out/foo.lib", LibOs::Windows),
            ["-Wl,--out-implib,out/foo.lib"]
        );
        for os in [LibOs::Linux, LibOs::Macos] {
            assert!(
                shim_implib_name("libfoo.so", os).is_none(),
                "an ELF/Mach-O shared object is linked directly; there is no second file"
            );
            let unexpected = shim_implib_args("out/foo.lib", os);
            assert!(unexpected.is_empty(), "{unexpected:?}");
        }
    }

    /// macOS bakes the `-o` path into a dylib as its install name, so a library
    /// published by rename must be told the name it will END UP with. Checked
    /// from any host by passing the convention in — left implicit this could
    /// only ever run on a Mac, which is how the `.tmp` install name shipped.
    #[test]
    fn macos_pins_the_final_name_and_other_hosts_add_nothing() {
        assert_eq!(
            install_name_args("lcshim_shim_bda315af5ea4cb63.dylib", LibOs::Macos),
            ["-Wl,-install_name,@rpath/lcshim_shim_bda315af5ea4cb63.dylib"],
            "the FINAL file name, and @rpath so the emitted -rpath resolves it"
        );
        for os in [LibOs::Linux, LibOs::Windows] {
            assert!(
                install_name_args("libfoo.so", os).is_empty(),
                "ELF sets a SONAME only when asked, and a DLL has no such record — \
                 what Windows needs instead is an import library, which is \
                 `shim_implib_args`, not a name recorded inside the artifact"
            );
        }
    }

    /// The name must carry no directory: an install name is what `dyld` is
    /// ASKED for, and `@rpath/` already supplies the search.
    #[test]
    fn the_install_name_is_relative_to_rpath() {
        let a = install_name_args("x.dylib", LibOs::Macos);
        assert!(a[0].starts_with("-Wl,-install_name,@rpath/"), "{a:?}");
        assert!(
            !a[0].contains("/native-auto/") && !a[0].contains(".tmp"),
            "neither the staging path nor an absolute directory may survive: {a:?}"
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod driver_death_tests {
    use super::dies_with_driver;
    use std::os::unix::process::ExitStatusExt as _;

    /// loft#1699 — a build child must not outlive the driver.  The death signal reaches the
    /// child when the thread that spawned it ends, so a thread that spawns and returns without
    /// waiting stands in for a driver that exits: the child must end by `SIGKILL` at once,
    /// not run its five seconds.  Without the helper the child is untouched and this reads
    /// `exit 0` after five seconds.
    #[test]
    fn a_build_child_dies_with_its_driver() {
        let started = std::time::Instant::now();
        let mut child = std::thread::spawn(|| {
            let mut cmd = std::process::Command::new("sleep");
            cmd.arg("5");
            dies_with_driver(&mut cmd, false);
            cmd.spawn().expect("spawn sleep")
        })
        .join()
        .expect("spawning thread");
        let status = child.wait().expect("wait");
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "the child outlived its driver: {status}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
    }

    /// The program itself is asked to leave (`SIGTERM`) so a handler can run.
    #[test]
    fn a_graceful_child_is_asked_to_leave() {
        let mut child = std::thread::spawn(|| {
            let mut cmd = std::process::Command::new("sleep");
            cmd.arg("5");
            dies_with_driver(&mut cmd, true);
            cmd.spawn().expect("spawn sleep")
        })
        .join()
        .expect("spawning thread");
        assert_eq!(child.wait().expect("wait").signal(), Some(libc::SIGTERM));
    }
}

/// `@FR-Path-Utf8` — `{before}<x>{after}` where `<x>` is what this platform's names can hold
/// and loft text cannot: the byte 0xFF on Unix, an unpaired UTF-16 half on Windows.  For the
/// guards that make such a file; a loft program can never spell one.
#[must_use]
pub fn name_that_is_not_text(before: &str, after: &str) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = before.as_bytes().to_vec();
        bytes.push(0xFF);
        bytes.extend_from_slice(after.as_bytes());
        std::ffi::OsString::from_vec(bytes)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut wide: Vec<u16> = before.encode_utf16().collect();
        wide.push(0xD800);
        wide.extend(after.encode_utf16());
        std::ffi::OsString::from_wide(&wide)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::ffi::OsString::from(format!("{before}\u{FFFD}{after}"))
    }
}

/// Like [`name_that_is_not_text`], with a second, different byte (0xFE / the other UTF-16
/// half), so two such names can show alike.
#[must_use]
pub fn another_name_that_is_not_text(before: &str, after: &str) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = before.as_bytes().to_vec();
        bytes.push(0xFE);
        bytes.extend_from_slice(after.as_bytes());
        std::ffi::OsString::from_vec(bytes)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut wide: Vec<u16> = before.encode_utf16().collect();
        wide.push(0xDC00);
        wide.extend(after.encode_utf16());
        std::ffi::OsString::from_wide(&wide)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::ffi::OsString::from(format!("{before}\u{FFFD}{after}"))
    }
}
