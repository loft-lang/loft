// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! Platform-specific helpers shared across the crate.

use crate::file_access::{File, Metadata};
#[cfg(unix)]
use std::os::unix::prelude::{MetadataExt as _, PermissionsExt};
use std::sync::OnceLock;

/// One way to run a process (@PLN184 Track P).
#[path = "platform_process.rs"]
pub mod process;

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
            && crate::file_access::create_dir_all(&dir).is_ok()
        {
            use std::io::Write;
            let path =
                std::path::Path::new(&dir).join(format!("timing-{}.tsv", std::process::id()));
            let secs_s = secs.map_or_else(String::new, |s| format!("{s:.2}"));
            if let Ok(mut f) = crate::file_access::open_with(
                path,
                std::fs::OpenOptions::new().create(true).append(true),
            ) {
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
            && crate::file_access::create_dir_all(&dir).is_ok()
        {
            use std::io::Write;
            let path =
                std::path::Path::new(&dir).join(format!("timing-{}.tsv", std::process::id()));
            if let Ok(mut f) = crate::file_access::open_with(
                path,
                std::fs::OpenOptions::new().create(true).append(true),
            ) {
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
    let _ = crate::file_access::create_dir_all(&dir);
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
    let _ = crate::file_access::create_dir_all(&dir);
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
    let _ = crate::file_access::create_dir_all(&dir);
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
    if crate::file_access::read_to_string(&marker).is_ok_and(|s| s.trim() == stamp) {
        return 0;
    }
    let mut freed = 0u64;
    if let Ok(entries) = crate::file_access::read_dir(dir) {
        for entry in entries {
            if !entry
                .file_name()
                .unwrap_or_default()
                .starts_with("loft_native_")
            {
                continue;
            }
            // The entry itself, as a listing reports it: a link is not followed.
            let Ok(meta) = crate::file_access::symlink_metadata(&entry) else {
                continue;
            };
            let fresh = meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() < 120);
            if fresh || !meta.is_file() {
                continue;
            }
            if crate::file_access::remove_file(&entry).is_ok() {
                freed += meta.len();
            }
        }
    }
    // Written last and atomically: a crash between the sweep and the marker only sweeps again.
    let tmp = dir.join(format!(".build.{}", std::process::id()));
    if crate::file_access::write(&tmp, stamp).is_ok() {
        let _ = crate::file_access::rename(&tmp, &marker);
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
    let out = process::Spawn::new(process::Program::search("df"))
        .arg("-P")
        .arg("-k")
        .arg(path)
        .run(b"")
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
        let ok = unsafe { GetExitCodeProcess(handle, &raw mut code) };
        // Read before the close, which would clobber the thread's last error.
        // SAFETY: as above — no arguments, no memory.
        let err = unsafe { GetLastError() };
        // SAFETY: closing a handle this function opened, exactly once.
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            let _ = err;
            return None;
        }
        Some(code == STILL_ACTIVE)
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
    let Ok(entries) = crate::file_access::read_dir(dir) else {
        return 0;
    };
    for entry in entries {
        let name = entry.file_name().unwrap_or_default();
        if !(name.starts_with("loft_native_") || name.starts_with("loft_test_native_")) {
            continue;
        }
        // Provably stale only through the strict shapes; the looser parse below serves the
        // age fallback's "is anyone alive behind this name" question and nothing else.
        let proven_stale = match runtime_scratch_pid(name) {
            Some(p) if p == own_pid => false,
            Some(p) => pid_alive(p) == Some(false),
            None => false,
        };
        let pid = scratch_owner_pid(name);
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
            // The entry itself, as a listing reports it: a link is not followed.
            let old_enough = crate::file_access::symlink_metadata(&entry)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 3600);
            if !old_enough {
                continue;
            }
        }
        let len = crate::file_access::symlink_metadata(&entry).map_or(0, |m| m.len());
        if crate::file_access::remove_file(&entry).is_ok() {
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
        let _ = crate::file_access::remove_dir_all(&dir);
        crate::file_access::create_dir_all(&dir).unwrap();
        let old = |name: &str| {
            let p = dir.join(name);
            crate::file_access::write(&p, b"x").unwrap();
            let t = std::time::SystemTime::now() - std::time::Duration::from_hours(1);
            // Open for WRITE before set_modified: Windows `SetFileTime` needs write access
            // (Unix `futimens` works on a read-only fd), as `cache::touch_now` does.
            crate::file_access::open_with(&p, std::fs::OpenOptions::new().write(true))
                .unwrap()
                .set_modified(t)
                .unwrap();
            p
        };
        let stale_bin = old("loft_native_a_bin");
        let stale_key = old("loft_native_a_bin.key");
        let foreign = old("other_tool_output");
        let fresh = dir.join("loft_native_b_bin");
        crate::file_access::write(&fresh, b"y").unwrap();
        crate::file_access::write(dir.join(".build"), "old-build").unwrap();

        let freed = sweep_own_native_cache(&dir, "new-build");
        assert_eq!(freed, 2, "the two stale entries, one byte each");
        assert!(
            !crate::file_access::exists(&stale_bin) && !crate::file_access::exists(&stale_key),
            "the older build's entries go"
        );
        assert!(
            crate::file_access::exists(&fresh),
            "a fresh entry is a concurrent shard's and stays"
        );
        assert!(
            crate::file_access::exists(&foreign),
            "a name that is not the harness's is never touched"
        );
        assert_eq!(
            crate::file_access::read_to_string(dir.join(".build")).unwrap(),
            "new-build"
        );

        let again = old("loft_native_c_bin");
        assert_eq!(
            sweep_own_native_cache(&dir, "new-build"),
            0,
            "the same build sweeps nothing"
        );
        assert!(crate::file_access::exists(&again));
        let _ = crate::file_access::remove_dir_all(&dir);
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
        crate::file_access::create_dir_all(&dir).unwrap();
        let own = dir.join(format!("loft_native_{}.rs", std::process::id()));
        // u32::MAX-1 — no real pid (Linux pid_max caps far below); provably dead.
        let dead = dir.join("loft_native_4294967294.rs");
        let fresh_no_pid = dir.join("loft_native_bin_notapid");
        // The native suite's stem-named source, whose stem happens to end in digits: not a
        // pid, and not the runtime's to sweep however dead "795" is.
        let stem_named = dir.join("loft_native_discard_slot_per_type_795.rs");
        crate::file_access::write(&own, "live").unwrap();
        crate::file_access::write(&dead, "stale").unwrap();
        crate::file_access::write(&fresh_no_pid, "fresh").unwrap();
        crate::file_access::write(&stem_named, "live source of a sibling worker").unwrap();
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
            let mut names: Vec<String> = crate::file_access::read_dir(&dir).map_or_else(
                |e| vec![format!("<read_dir failed: {e}>")],
                |es| {
                    es.iter()
                        .map(|e| e.file_name().unwrap_or_default().to_string())
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
            !crate::file_access::exists(&dead),
            "dead-pid file must go in the dead-only sweep ({})",
            evidence(dead_only)
        );
        assert!(
            crate::file_access::exists(&own) && crate::file_access::exists(&fresh_no_pid),
            "own-pid and no-pid entries survive the dead-only sweep"
        );
        assert!(
            crate::file_access::exists(&stem_named),
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
        crate::file_access::write(&dead, "stale").unwrap();
        let freed = reclaim_native_scratch(&dir);
        assert!(
            crate::file_access::exists(&own),
            "own-pid file must survive the reclaim"
        );
        assert!(
            crate::file_access::exists(&fresh_no_pid),
            "a fresh file without a parseable pid must survive (age floor)"
        );
        if cfg!(target_os = "linux") {
            assert!(
                !crate::file_access::exists(&dead),
                "dead-pid file must be reclaimed"
            );
            assert_eq!(freed, 5);
        }
        let _ = crate::file_access::remove_dir_all(&dir);
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
        .any(|n| unsafe { crate::file_access::load_library(n) }.is_ok())
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
        .find(|cand| crate::file_access::exists(dir.join(cand)))
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

// ---------------------------------------------------------------------------
// @PLN184 B2 — the platform differences that were `cfg` gates at their call sites.
//
// Each routine below has an answer on every platform and its caller calls it
// unconditionally.  Where Windows has no implementation of its own, the routine gives
// the answer Windows got before the move, and its doc comment says so in one line:
// `Windows: <what it does>; approved exemption (owner, 2026-10-07): <why>`.
// ---------------------------------------------------------------------------

/// An executable's file name on this host: `<stem>.exe` on Windows, `<stem>` elsewhere.
#[must_use]
pub fn exe_file_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// A launcher SCRIPT's file name: `<stem>.bat` on Windows, `<stem>` elsewhere (the Android
/// SDK ships `apksigner` as a shell script beside an `apksigner.bat`).
#[must_use]
pub fn launcher_script_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.bat")
    } else {
        stem.to_string()
    }
}

/// The file names `tool` can have in a `PATH` directory, in the order to try them: `tool`
/// itself, then on Windows `tool.exe` and `tool.cmd`.
#[must_use]
pub fn executable_candidates(tool: &str) -> Vec<String> {
    let mut names = vec![tool.to_string()];
    if cfg!(windows) {
        names.push(format!("{tool}.exe"));
        names.push(format!("{tool}.cmd"));
    }
    names
}

/// The platform shell's program and arguments for running the command line `line`:
/// `sh -c <line>`, or `cmd /C <line>` on Windows.
#[must_use]
pub fn shell_invocation(line: &str) -> (&'static str, [&str; 2]) {
    if cfg!(windows) {
        ("cmd", ["/C", line])
    } else {
        ("sh", ["-c", line])
    }
}

/// The host OS as a display name for a diagnostic: `Linux`, `macOS`, `Windows`, or `this OS`.
#[must_use]
pub fn host_os_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "this OS"
    }
}

/// The Android NDK's prebuilt-toolchain host tag for the machine loft runs on.  NDKs since
/// r23 ship only x86_64 host toolchains, so every supported host maps to its `*-x86_64` tag.
#[must_use]
pub fn ndk_host_tag() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else {
        "linux-x86_64"
    }
}

/// Width of C `long` in bits on this host: 64 on LP64 (Linux, macOS), 32 on LLP64 (Windows).
#[must_use]
pub fn c_long_bits() -> u8 {
    if cfg!(windows) { 32 } else { 64 }
}

/// Extra `cc` arguments for a `#c` shim so it does not depend on its compiler's runtime
/// DLLs: `-static-libgcc` on Windows (a MinGW `cc` links `libgcc_s_seh-1.dll` by default,
/// and a consumer's machine has no MinGW `bin` on `PATH`), nothing elsewhere.
#[must_use]
pub fn shim_cc_runtime_args() -> &'static [&'static str] {
    if cfg!(windows) {
        &["-static-libgcc"]
    } else {
        &[]
    }
}

/// The extension of a shared library on this host, without the dot: `dll`, `dylib` or `so`.
#[must_use]
pub fn dll_extension() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

/// The file name a cdylib built from crate stem `stem` has on this host:
/// `lib<stem>.so`, `lib<stem>.dylib`, or `<stem>.dll`.
#[must_use]
pub fn cdylib_file_name(stem: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(windows) {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
    }
}

/// The platform part of how a package cdylib is linked, beyond the baked RUSTFLAGS — one
/// home, read by the build ([`relocatable_dylib_flags`]) and folded into
/// `cache::native_artifact_cache_key`, so a change to it rebuilds every cached cdylib
/// instead of reusing one linked the old way.  On macOS the linker drops the debug symbols
/// itself and the post-link `strip` is off ([`cdylib_build_env`]): that strip left a TLS
/// package's string table misaligned, and the linker then refused it (`mis-aligned
/// LINKEDIT string pool`).  Empty elsewhere.
pub const NATIVE_LINK_RECIPE: &str = if cfg!(target_os = "macos") {
    "-Clink-arg=-Wl,-S"
} else {
    ""
};

/// The link flags that make a built cdylib RELOCATABLE — empty on every platform but macOS.
///
/// A Mach-O dylib records its own path (`LC_ID_DYLIB`) and a program that links it copies
/// THAT path in, so the loader follows the build-time location and nothing else.  Cargo's
/// default is the absolute output path, `…/target/release/deps/lib<stem>.dylib` — and a
/// package cdylib is CACHED and reused from a different directory than the one it was built
/// in, so the recorded path names a directory that no longer exists (`dyld: Library not
/// loaded: …/.loft_test_tmp_<pid>_0/native/target/release/deps/…` on a cache HIT).
///
/// ELF does not have the problem: a `.so` records only its SONAME and the consumer's
/// `-rpath` resolves it.  `@rpath/<file>` makes Mach-O behave the same way, and the consumer
/// already emits both the absolute `-rpath` of the resolved library and `$ORIGIN` /
/// `@loader_path`.  [`NATIVE_LINK_RECIPE`] (`-Wl,-S`) makes the LINKER drop the debug
/// symbols: Cargo's default post-link `strip` leaves a dylib holding `ring`'s objects with a
/// 4-aligned string table, which the same linker then refuses to link a program against.
#[must_use]
pub fn relocatable_dylib_flags(lib_name: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("-Clink-arg=-Wl,-install_name,@rpath/{lib_name} {NATIVE_LINK_RECIPE}")
    } else {
        String::new()
    }
}

/// Environment for the `cargo build` of a package cdylib: on macOS the post-link `strip` is
/// switched off (see [`relocatable_dylib_flags`]); nothing elsewhere.
#[must_use]
pub fn cdylib_build_env() -> &'static [(&'static str, &'static str)] {
    if cfg!(target_os = "macos") {
        &[("CARGO_PROFILE_RELEASE_STRIP", "none")]
    } else {
        &[]
    }
}

/// The `rustc` arguments that give a native PROGRAM's main thread the stack a Linux one has
/// (8 MiB): Windows defaults to 1 MiB, and the same recursion must not overflow on one
/// platform only.  Empty off Windows, where the main thread already has it.
#[must_use]
pub fn main_stack_link_args() -> Vec<String> {
    if cfg!(all(windows, target_env = "msvc")) {
        vec!["-C".to_string(), format!("link-arg=/STACK:{}", 8 << 20)]
    } else if cfg!(all(windows, target_env = "gnu")) {
        vec![
            "-C".to_string(),
            format!("link-arg=-Wl,--stack,{}", 8 << 20),
        ]
    } else {
        Vec::new()
    }
}

/// The `rustc` argument that lets two native packages each define the same runtime symbol
/// (the linker keeps the first), or `None` where the host linker has no such option: macOS
/// `ld64` rejects `--allow-multiple-definition` and MSVC `link.exe` ignores it with one
/// `LNK4044` warning per occurrence.
#[must_use]
pub fn allow_multiple_definition_arg() -> Option<&'static str> {
    if cfg!(any(target_os = "macos", windows)) {
        None
    } else {
        Some("-Clink-arg=-Wl,--allow-multiple-definition")
    }
}

/// The `link-arg=` value that records `dir` as a run-time library search path (RPATH) in the
/// binary, for `rustc -C`.  `None` on Windows, which has no RPATH: `link.exe` rejects
/// `-Wl,-rpath`, and the loader finds a DLL beside the `.exe` or on `PATH` — so loft stages
/// the DLLs beside the binary instead ([`stages_dlls_beside_binary`]).
#[must_use]
pub fn rpath_link_arg(dir: &std::path::Path) -> Option<String> {
    if cfg!(windows) {
        None
    } else {
        Some(format!("link-arg=-Wl,-rpath,{}", dir.display()))
    }
}

/// macOS's spelling of "beside the binary" in an RPATH (`@loader_path`; ELF's `$ORIGIN`
/// means nothing to dyld), as a `rustc` argument.  `None` elsewhere.
#[must_use]
pub fn loader_path_rpath_arg() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("-Clink-arg=-Wl,-rpath,@loader_path")
    } else {
        None
    }
}

/// Does a native binary find its package DLLs only because loft copies them beside it?
/// True on Windows (no RPATH; the loader searches the `.exe`'s directory and `PATH`), so the
/// staging step runs after every build — and anything that skips the build must not skip it.
#[must_use]
pub fn stages_dlls_beside_binary() -> bool {
    cfg!(windows)
}

/// What [`bridge_import_lib`] found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImportLib {
    /// This host links a shared library directly (ELF, Mach-O): no import library.
    NotUsed,
    /// The import library `-l dylib=<name>` asks for is in place.
    Ready,
    /// Neither `<name>.dll.lib` nor `<name>.lib` is there: the link would die on an opaque
    /// `LNK1181`, so the caller names the cause.
    Missing,
}

/// Windows links a DLL through its IMPORT LIBRARY.  A Rust cdylib's import library is
/// `<libname>.dll.lib`, but `-l dylib=<libname>` makes MSVC `link.exe` open `<libname>.lib`
/// (`LNK1181: cannot open input file 'loft_native_scalar.lib'`), so this copies the first to
/// the second beside it — both are import libraries for the same DLL.  Off Windows nothing
/// is touched and the answer is [`ImportLib::NotUsed`].
#[must_use]
pub fn bridge_import_lib(dir: &std::path::Path, libname: &str) -> ImportLib {
    if !cfg!(windows) {
        return ImportLib::NotUsed;
    }
    let dll_lib = dir.join(format!("{libname}.dll.lib"));
    let plain_lib = dir.join(format!("{libname}.lib"));
    if crate::file_access::exists(&dll_lib) && !crate::file_access::exists(&plain_lib) {
        let _ = crate::file_access::copy(&dll_lib, &plain_lib);
    }
    if !crate::file_access::exists(&plain_lib) && !crate::file_access::exists(&dll_lib) {
        ImportLib::Missing
    } else {
        ImportLib::Ready
    }
}

/// The directories that hold the native import libraries a hand-driven `rustc` must be given
/// as `-L` paths on Windows MSVC (`windows.0.48.5.lib` from `windows-sys`): cargo adds them
/// through `cargo:rustc-link-search`, and a `rustc` loft drives itself does not, so a cdylib
/// link fails `LNK1181`.  `rlib` is `target/<profile>/libloft.rlib` or
/// `target/<profile>/deps/libloft-*.rlib`; the scan reads `build/<crate>-<hash>/` beside it.
/// Empty off Windows, where no import library exists.
#[must_use]
pub fn import_lib_search_dirs(rlib: &std::path::Path) -> Vec<std::path::PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let Some(profile_dir) = rlib.parent().and_then(|p| {
        if crate::file_access::file_name(p).is_some_and(|n| n == "deps") {
            p.parent()
        } else {
            Some(p)
        }
    }) else {
        return Vec::new();
    };
    let Ok(entries) = crate::file_access::read_dir(profile_dir.join("build")) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in entries {
        let build_entry = entry.os_spelling();
        // `out/` and its immediate subdirs (some crates emit into `out/<target>/`).
        let out = build_entry.join("out");
        if crate::file_access::is_dir(&out) {
            dirs.push(out.clone());
            if let Ok(subs) = crate::file_access::read_dir(&out) {
                dirs.extend(
                    subs.iter()
                        .map(crate::file_access::PathText::os_spelling)
                        .filter(|p| crate::file_access::is_dir(p)),
                );
            }
        }
        // `cargo:rustc-link-search` directives cached in `build/<crate>-<hash>/output`
        // (e.g. `windows_x86_64_msvc` ships its `.lib` inside the registry package).
        if let Ok(content) = crate::file_access::read_to_string(build_entry.join("output")) {
            for line in content.lines() {
                if let Some(p) = line
                    .strip_prefix("cargo:rustc-link-search=native=")
                    .or_else(|| line.strip_prefix("cargo:rustc-link-search="))
                {
                    let p = std::path::PathBuf::from(p);
                    if crate::file_access::is_dir(&p) && !dirs.contains(&p) {
                        dirs.push(p);
                    }
                }
            }
        }
    }
    dirs
}

/// Did a process die before `main` because the loader could not resolve a DLL it imports
/// (`STATUS_DLL_NOT_FOUND`, `0xC000_0135`)?  Only Windows reports a start-up failure as an
/// exit code; elsewhere the answer is always `false`.
#[must_use]
pub fn is_dll_not_found(status: std::process::ExitStatus) -> bool {
    cfg!(windows) && status.code() == Some(STATUS_DLL_NOT_FOUND)
}

/// Windows' `STATUS_DLL_NOT_FOUND`, as the exit code a process that could not start reports.
pub const STATUS_DLL_NOT_FOUND: i32 = 0xC000_0135_u32 as i32;

/// The signal that ended a process, or `None` when it exited (or the host has no signals).
/// Windows: always `None` — a process there ends with an exit code, never a signal;
/// approved exemption (owner, 2026-10-07): none needed, the exit code carries the cause.
#[must_use]
pub fn exit_signal(status: std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

/// Can a library declared `placement = "process"` run in a worker process on this host?
/// Windows: `false` — the library runs in-process, and `LOFT_REQUIRE_PLACEMENT=1` refuses;
/// approved exemption (owner, 2026-10-07): the transport's shared file mapping and the worker's parent-death
/// watch (`lib_placement::wire`) are written for Unix only, the watch being Track P's.
#[must_use]
pub fn placement_transport_available() -> bool {
    cfg!(unix)
}

// ── files: permission bits and identity ─────────────────────────────────────

/// Does this host keep Unix permission bits on a file?  `false` on Windows, whose NTFS
/// keeps ACLs instead.
#[must_use]
pub fn has_permission_bits() -> bool {
    cfg!(unix)
}

/// Set a file's Unix permission bits to exactly `mode` (`0o600`, `0o700`, `0o755`).
///
/// # Errors
/// The OS's error, naming the path.
///
/// Windows: a no-op answering `Ok` — a new file inherits its directory's ACL; exemption
/// candidate: NTFS has no mode bits to set.
pub fn set_permission_bits(path: &std::path::Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        crate::file_access::set_permissions(path, PermissionsExt::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// A file's Unix permission bits, or `None` where the host keeps none.
/// Windows: `None`; approved exemption (owner, 2026-10-07): NTFS has no mode bits.
#[must_use]
pub fn permission_bits(md: &Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        Some(md.permissions().mode())
    }
    #[cfg(not(unix))]
    {
        let _ = md;
        None
    }
}

/// Is the file `md` describes this user's alone: owned by the effective user, with no
/// group or other permission bits (and, with `no_setid`, no set-uid / set-gid bit)?  The
/// test a cached binary and its directory pass before loft executes from them.
///
/// Windows: `true` — there is no owner uid or mode to read, and the cache directory's ACL is
/// inherited from the user's profile; approved exemption (owner, 2026-10-07): the substitute is an ACL read
/// (`GetNamedSecurityInfoW`), unwritten.
#[must_use]
pub fn is_private_to_owner(md: &Metadata, no_setid: bool) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: `geteuid` reads a process attribute and cannot fail.
        if md.uid() != unsafe { libc::geteuid() } {
            return false;
        }
        // Group/other bits: an attacker with group access could swap the file between the
        // stat and the exec, and a group-readable file leaks compiled output.
        if md.mode() & 0o077 != 0 {
            return false;
        }
        // A cached binary must never carry a privilege-escalation bit.
        !(no_setid && md.mode() & 0o6000 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = (md, no_setid);
        true
    }
}

/// The identity of the file `md` describes — `(device, inode)` — which a rename-replace
/// changes and an in-place rewrite keeps.  Windows: `None`; approved exemption (owner, 2026-10-07): std's
/// `MetadataExt::file_index` / `volume_serial_number` are unstable, and the substitute is
/// `GetFileInformationByHandle`, unwritten.
#[must_use]
pub fn file_identity(md: &Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        Some((md.dev(), md.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = md;
        None
    }
}

/// Fill `buf` from the operating system's entropy source (`/dev/urandom`).
///
/// # Errors
/// The read's error; `Unsupported` where no source is implemented.
///
/// Windows: `Unsupported` ("needs /dev/urandom"); approved exemption (owner, 2026-10-07): the one caller is the
/// registry key-generation bootstrap, documented to run on an air-gapped Unix machine — the
/// substitute would be `BCryptGenRandom`.
pub fn fill_random(buf: &mut [u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Read as _;
        let mut f = crate::file_access::open("/dev/urandom")?;
        f.read_exact(buf)
    }
    #[cfg(not(unix))]
    {
        let _ = buf;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "loft-keygen needs /dev/urandom (Unix-only).  Run on Linux/macOS instead.",
        ))
    }
}

// ── memory: pages and residency ────────────────────────────────────────────

/// The kernel's page size in bytes — the granularity a residency hint drops at, asked of the
/// host rather than assumed (16 KB on aarch64 macOS).  Windows: 4096; approved exemption (owner, 2026-10-07):
/// nothing there drops pages ([`release_resident_pages`]), so nothing reads it.
#[must_use]
pub fn page_bytes() -> u64 {
    #[cfg(unix)]
    {
        static PAGE: OnceLock<u64> = OnceLock::new();
        #[allow(clippy::cast_sign_loss)]
        *PAGE.get_or_init(|| unsafe { libc::sysconf(libc::_SC_PAGESIZE).max(4096) as u64 })
    }
    #[cfg(not(unix))]
    {
        4096
    }
}

/// Can [`release_resident_pages`] drop pages on this host?  Windows: `false`; see there.
#[must_use]
pub fn releases_resident_pages() -> bool {
    cfg!(unix)
}

/// Start the write-back of `len` bytes of a SHARED file mapping at `at` and drop them from
/// this process's resident set (`msync(MS_ASYNC)` then `madvise(MADV_DONTNEED)`).  The bytes
/// stay in the page cache and the file, so an access afterwards re-faults the same content.
/// Answers whether both calls succeeded.
///
/// # Safety
/// `at` is page-aligned and `at..at + len` lies inside one live `MAP_SHARED` mapping.
///
/// Windows: does nothing and answers `false` — a residency hint not honoured; exemption
/// candidate: the substitute is `FlushViewOfFile` + `OfferVirtualMemory`, unwritten.
#[must_use]
pub unsafe fn release_resident_pages(at: *mut u8, len: usize) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: the caller's contract — a page-aligned range inside a live shared mapping.
        unsafe {
            libc::msync(at.cast(), len, libc::MS_ASYNC) == 0
                && libc::madvise(at.cast(), len, libc::MADV_DONTNEED) == 0
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (at, len);
        false
    }
}

/// This process's resident set in KB, or `None` where it cannot be read.  Linux reads field
/// 2 of `/proc/self/statm` (resident pages, taken as 4 KB).  Elsewhere `None`; exemption
/// candidate: a design measurement's instrument only (`database::spans`).
#[must_use]
pub fn resident_set_kb() -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let statm = crate::file_access::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|f| f.parse().ok())
        .unwrap_or(0);
    Some(pages * 4)
}

/// Minor page faults this process has taken — field 10 of `/proc/self/stat` on Linux — or
/// `None` where it cannot be read.  Elsewhere `None`; approved exemption (owner, 2026-10-07): a design
/// measurement's instrument only (`database::spans`).
#[must_use]
pub fn minor_page_faults() -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let stat = crate::file_access::read_to_string("/proc/self/stat").unwrap_or_default();
    // The second field is the comm, parenthesised and free to contain spaces, so fields
    // are counted from the closing paren rather than from the start.
    let tail = stat.rsplit_once(')').map(|(_, t)| t).unwrap_or_default();
    Some(
        tail.split_whitespace()
            .nth(7)
            .and_then(|f| f.parse().ok())
            .unwrap_or(0),
    )
}

// ── signals: the crash report and the profiler's flush ──────────────────────

/// What [`on_fatal_signal`] hands a fatal signal to.  Read inside the signal handler, where
/// a `OnceLock::get` is one atomic load.
#[cfg(unix)]
static FATAL_HOOK: OnceLock<fn(&'static str)> = OnceLock::new();

/// Can this host hand a fatal fault (`SIGSEGV`, `SIGABRT`, `SIGBUS`) to loft before the
/// process dies?  Windows: `false`; see [`on_fatal_signal`].
#[must_use]
pub fn catches_fatal_signals() -> bool {
    cfg!(unix)
}

/// Arm `report` for the fatal signals `SIGSEGV`, `SIGABRT` and `SIGBUS`: it is called with
/// the signal's name, INSIDE the handler — so it may do only what is async-signal-safe — and
/// the default action (the core dump, the exit) follows, because the handler is armed with
/// `SA_RESETHAND`.  The first `report` armed is the one called.
///
/// Windows: nothing is armed, and a fatal fault ends with Rust's own abort report and the
/// OS's; approved exemption (owner, 2026-10-07): Windows has structured exceptions rather than signals — the
/// substitute is a vectored exception handler (`AddVectoredExceptionHandler`), unwritten.
pub fn on_fatal_signal(report: fn(&'static str)) {
    #[cfg(unix)]
    {
        let _ = FATAL_HOOK.set(report);
        // SAFETY: `sigaction` with a handler that only reads an initialised `OnceLock`
        // and calls the report, whose own contract is async-signal safety.
        unsafe {
            for &sig in &[libc::SIGSEGV, libc::SIGABRT, libc::SIGBUS] {
                let mut act: libc::sigaction = std::mem::zeroed();
                act.sa_sigaction = fatal_handler as *const () as libc::sighandler_t;
                // SA_SIGINFO for the siginfo/ucontext args ignored here; SA_RESETHAND so
                // the default handler runs after the report (produces the core dump).
                act.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND;
                libc::sigemptyset(&raw mut act.sa_mask);
                libc::sigaction(sig, &raw const act, std::ptr::null_mut());
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = report;
    }
}

#[cfg(unix)]
extern "C" fn fatal_handler(
    sig: libc::c_int,
    _info: *mut libc::siginfo_t,
    _ucontext: *mut libc::c_void,
) {
    let name = match sig {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        _ => "signal",
    };
    if let Some(report) = FATAL_HOOK.get() {
        report(name);
    }
}

/// Write `bytes` to standard error without allocating or locking — callable from a signal
/// handler (`write(2)` is async-signal-safe).
///
/// Windows: through `std::io::stderr()`, which locks; approved exemption (owner, 2026-10-07): none needed —
/// nothing calls it from a handler there, because [`on_fatal_signal`] arms none.
pub fn signal_safe_stderr(bytes: &[u8]) {
    #[cfg(unix)]
    // SAFETY: `write` on the stderr descriptor with a valid buffer and its length.
    unsafe {
        let _ = libc::write(
            libc::STDERR_FILENO,
            bytes.as_ptr().cast::<libc::c_void>(),
            bytes.len(),
        );
    }
    #[cfg(not(unix))]
    {
        use std::io::Write as _;
        let _ = std::io::stderr().write_all(bytes);
    }
}

/// Create (or truncate) the file at the NUL-terminated `c_path`, readable by its owner only,
/// and write `bytes` to it — callable from a signal handler (`open`/`write`/`close` are
/// async-signal-safe, and nothing here allocates).  Answers whether anything was written.
///
/// Windows: writes nothing and answers `false`; approved exemption (owner, 2026-10-07): none needed — nothing
/// calls it there, because [`on_fatal_signal`] arms no handler.
#[must_use]
pub fn signal_safe_write_file(c_path: &[u8], bytes: &[u8]) -> bool {
    #[cfg(unix)]
    {
        if c_path.last() != Some(&0) {
            return false;
        }
        // SAFETY: `c_path` is NUL-terminated (checked above); `bytes` is a valid buffer.
        // 0o600: a crash dump names internals, so it is readable by its owner only.
        unsafe {
            let fd = libc::open(
                c_path.as_ptr().cast::<libc::c_char>(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o600 as libc::c_int,
            );
            if fd < 0 {
                false
            } else {
                let n = libc::write(fd, bytes.as_ptr().cast::<libc::c_void>(), bytes.len());
                libc::close(fd);
                n > 0
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (c_path, bytes);
        false
    }
}

/// What [`on_profile_signals`] hands a signal to.
#[cfg(unix)]
static PROFILE_HOOK: OnceLock<fn(u8)> = OnceLock::new();

/// Arm the profiler's flush signals.  `SIGUSR1` calls `flush(1)` (dump and keep running);
/// `SIGINT` and `SIGTERM` call `flush(128 + signal)` (dump and leave with the shell's "died
/// of this signal" code) once, after which the default action is back — so the SECOND
/// signal is the ordinary kill rather than a hang.  No `SA_RESTART`: a blocking read returns
/// `EINTR` and the program comes back to its loop to render.  `flush` runs inside the
/// handler, so it may do only what is async-signal-safe.
///
/// Windows: nothing is armed — a profiled server is stopped without a report; exemption
/// candidate: Windows has no `SIGUSR1`, and the substitute for the two terminating signals
/// is a console control handler (`SetConsoleCtrlHandler`), unwritten.
pub fn on_profile_signals(flush: fn(u8)) {
    #[cfg(unix)]
    {
        let _ = PROFILE_HOOK.set(flush);
        // SAFETY: `sigaction` with a handler that reads an initialised `OnceLock` and calls
        // `flush`, whose own contract is async-signal safety.
        unsafe {
            for &(sig, reset) in &[
                (libc::SIGUSR1, false),
                (libc::SIGINT, true),
                (libc::SIGTERM, true),
            ] {
                let mut act: libc::sigaction = std::mem::zeroed();
                act.sa_sigaction = profile_handler as *const () as libc::sighandler_t;
                act.sa_flags = if reset { libc::SA_RESETHAND } else { 0 };
                libc::sigemptyset(&raw mut act.sa_mask);
                libc::sigaction(sig, &raw const act, std::ptr::null_mut());
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = flush;
    }
}

#[cfg(unix)]
extern "C" fn profile_handler(sig: libc::c_int) {
    let want = if sig == libc::SIGUSR1 {
        1
    } else {
        u8::try_from(128 + sig).unwrap_or(129)
    };
    if let Some(flush) = PROFILE_HOOK.get() {
        flush(want);
    }
}

// ── sockets: the hot-swap handover ───────────────────────────────────────────

/// Bind a TCP listener on `0.0.0.0:port` for a server that may be HANDED OVER to its next
/// build: `SO_REUSEADDR` (a restarted server rebinds through `TIME_WAIT`) and `SO_REUSEPORT`
/// (the new build binds the same port while the old one still serves), close-on-exec so a
/// spawned child never holds the listener.  `None` when the bind or listen fails.
///
/// Windows: a plain `TcpListener::bind`, so the new build binds only after the old listener
/// has closed; approved exemption (owner, 2026-10-07): Windows has no `SO_REUSEPORT` load-balancing group, and
/// its `SO_REUSEADDR` lets a second process steal a port rather than share it.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn bind_tcp_handover(port: u16) -> Option<std::net::TcpListener> {
    // @PLN184 W1.3 — under the emulated Windows host the bind is Windows': no shared port.
    #[cfg(unix)]
    if !crate::file_access::Flavor::emulating() {
        use std::os::fd::FromRawFd;
        // SAFETY: plain socket calls on a descriptor this function owns until it is handed
        // to `TcpListener`, or closed on failure.
        unsafe {
            // CLOEXEC: kernel sockets belong to ONE process.  Without it, every spawned
            // child (the rebuild driver, the swap target) inherits this listening fd across
            // exec — the zombie copy stays in the SO_REUSEPORT group and eats load-balanced
            // SYNs into a backlog nobody accepts.
            let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            if fd < 0 {
                return None;
            }
            // Portable CLOEXEC: macOS has no SOCK_CLOEXEC socket flag — set the fd flag
            // right after creation (single-threaded; no exec in between).
            let _ = libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            let one: libc::c_int = 1;
            let _ = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEADDR,
                std::ptr::addr_of!(one).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
            // During a build swap the NEW process binds the same port while the old one
            // still serves; the overlap is what makes rollback trivial.
            let _ = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                std::ptr::addr_of!(one).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
            // Zero-init then set fields: BSD's sockaddr_in has an extra sin_len a struct
            // literal would have to cfg around.
            let mut addr: libc::sockaddr_in = std::mem::zeroed();
            addr.sin_family = libc::AF_INET as libc::sa_family_t;
            addr.sin_port = port.to_be(); // sin_addr stays 0.0.0.0
            if libc::bind(
                fd,
                std::ptr::addr_of!(addr).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            ) != 0
                || libc::listen(fd, 128) != 0
            {
                libc::close(fd);
                return None;
            }
            return Some(std::net::TcpListener::from_raw_fd(fd));
        }
    }
    {
        let t0 = std::time::Instant::now();
        let r = std::net::TcpListener::bind(("0.0.0.0", port));
        crate::net_profile::record(
            "listener/bind",
            t0.elapsed(),
            if r.is_ok() {
                crate::net_profile::Outcome::Ok
            } else {
                crate::net_profile::Outcome::Failed
            },
            None,
        );
        r.ok()
    }
}

/// Bind a UDP socket on `0.0.0.0:port` with `SO_REUSEPORT` and close-on-exec, the datagram
/// twin of [`bind_tcp_handover`]: during the brief dual-bind window datagrams load-balance
/// between the old build and the new.
///
/// # Errors
/// The OS's error from `socket` or `bind`.
///
/// Windows: a plain `UdpSocket::bind`; approved exemption (owner, 2026-10-07): as [`bind_tcp_handover`].
#[cfg(not(target_arch = "wasm32"))]
pub fn bind_udp_handover(port: u16) -> std::io::Result<std::net::UdpSocket> {
    // @PLN184 W1.3 — under the emulated Windows host the bind is Windows': no shared port.
    #[cfg(unix)]
    if !crate::file_access::Flavor::emulating() {
        use std::os::fd::FromRawFd;
        // SAFETY: as in `bind_tcp_handover`.
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let _ = libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            let one: libc::c_int = 1;
            let _ = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                std::ptr::addr_of!(one).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
            let mut addr: libc::sockaddr_in = std::mem::zeroed();
            addr.sin_family = libc::AF_INET as libc::sa_family_t;
            addr.sin_port = port.to_be(); // sin_addr stays 0.0.0.0
            if libc::bind(
                fd,
                std::ptr::addr_of!(addr).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            ) != 0
            {
                let e = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            return Ok(std::net::UdpSocket::from_raw_fd(fd));
        }
    }
    std::net::UdpSocket::bind(("0.0.0.0", port))
}

#[cfg(all(test, unix))]
mod handover_tests {
    use crate::file_access::{Flavor, with_program_host};

    /// @PLN184 W1.3 — a second bind to a port that is listening: shared on Unix (the hot
    /// swap's overlap), refused on Windows — and under the emulated Windows host.
    #[test]
    fn a_second_bind_is_shared_on_unix_and_refused_on_windows() {
        let first = super::bind_tcp_handover(0).expect("first bind");
        let port = first.local_addr().unwrap().port();
        assert!(
            super::bind_tcp_handover(port).is_some(),
            "Unix shares the port"
        );
        assert!(
            with_program_host(Flavor::Windows, || super::bind_tcp_handover(port)).is_none(),
            "the emulated Windows host refuses the second bind"
        );
        let udp = super::bind_udp_handover(0).expect("first udp bind");
        let uport = udp.local_addr().unwrap().port();
        assert!(
            super::bind_udp_handover(uport).is_ok(),
            "Unix shares the UDP port"
        );
        assert!(with_program_host(Flavor::Windows, || super::bind_udp_handover(uport)).is_err());
    }
}

// ── symbols: what the process already has loaded ───────────────────────────

/// Find `symbol` among what this process ALREADY has loaded, loading nothing new.
///
/// Unix asks the process handle (`dlsym` on the main program): symbols linked in (libc)
/// and anything loaded with global visibility.  Windows has no process-wide symbol table —
/// `GetProcAddress` answers per MODULE, and the C runtime is its own DLL — so it asks the
/// modules the process already has open, in the order a C symbol is most likely to live:
/// the executable itself, the UCRT, the legacy CRT shim, then the Win32 base DLLs.
/// `open_already_loaded` is `GetModuleHandle`: it never loads anything.  Other hosts:
/// `None`.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn symbol_in_process(symbol: &str) -> Option<*const ()> {
    #[cfg(unix)]
    {
        use libloading::os::unix::Library;
        let this = Library::this();
        let mut name = symbol.to_string();
        name.push('\0');
        // SAFETY: the symbol is only looked up here, never called; the caller checks its
        // signature against the `#c` declaration before any call.
        if let Ok(sym) = unsafe { this.get::<*const ()>(name.as_bytes()) } {
            return Some(*sym);
        }
    }
    #[cfg(windows)]
    {
        use libloading::os::windows::Library;
        // SAFETY: as above — a lookup, not a call.
        if let Ok(this) = Library::this()
            && let Ok(sym) = unsafe { this.get::<*const ()>(symbol.as_bytes()) }
        {
            return Some(*sym);
        }
        for module in [
            "ucrtbase.dll",
            "api-ms-win-crt-string-l1-1-0.dll",
            "api-ms-win-crt-convert-l1-1-0.dll",
            "api-ms-win-crt-stdio-l1-1-0.dll",
            "api-ms-win-crt-heap-l1-1-0.dll",
            "msvcrt.dll",
            "kernel32.dll",
        ] {
            // SAFETY: as above.
            if let Ok(lib) = Library::open_already_loaded(module)
                && let Ok(sym) = unsafe { lib.get::<*const ()>(symbol.as_bytes()) }
            {
                return Some(*sym);
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = symbol;
    }
    None
}

/// The exported name of the function at `ptr`, asked of the dynamic loader — only an EXACT
/// hit counts: the nearest preceding symbol (what `dladdr` reports for an address inside a
/// function) would hand codegen a `#[link_name]` for a neighbouring function, so a near miss
/// is `None`.
///
/// Unix: `dladdr` with `dli_saddr == ptr`.  Windows: there is no `dladdr`, so the module's
/// own PE export table answers — `GetModuleHandleExW(FROM_ADDRESS)` names the module the
/// pointer lives in (an `HMODULE` IS its mapped base), then the export directory is walked
/// for the export whose address equals the pointer (loft#972).  `UNCHANGED_REFCOUNT`: this
/// only reads the module, so it must not pin it loaded.  Other hosts: `None`.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn exported_symbol_at(ptr: *const ()) -> Option<String> {
    #[cfg(unix)]
    {
        // SAFETY: `dladdr` fills `info` for any address; a zero answer means "not found".
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        if unsafe { libc::dladdr(ptr.cast(), &raw mut info) } == 0 {
            return None;
        }
        if info.dli_sname.is_null() || !std::ptr::eq(info.dli_saddr.cast_const().cast::<()>(), ptr)
        {
            return None;
        }
        // SAFETY: `dli_sname` is a NUL-terminated name owned by the loader.
        unsafe { std::ffi::CStr::from_ptr(info.dli_sname) }
            .to_str()
            .ok()
            .map(str::to_string)
    }
    #[cfg(windows)]
    {
        exported_symbol_at_pe(ptr)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = ptr;
        None
    }
}

/// The Windows half of [`exported_symbol_at`]: the PE export-table walk.
#[cfg(all(feature = "native-extensions", windows))]
fn exported_symbol_at_pe(ptr: *const ()) -> Option<String> {
    const FROM_ADDRESS: u32 = 0x0000_0004;
    const UNCHANGED_REFCOUNT: u32 = 0x0000_0002;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(
            flags: u32,
            module_name: *const u16,
            module: *mut *mut core::ffi::c_void,
        ) -> i32;
    }

    let mut handle: *mut core::ffi::c_void = std::ptr::null_mut();
    if unsafe {
        GetModuleHandleExW(
            FROM_ADDRESS | UNCHANGED_REFCOUNT,
            ptr.cast::<u16>(),
            &raw mut handle,
        )
    } == 0
    {
        return None;
    }
    let base = handle.cast::<u8>();
    if base.is_null() {
        return None;
    }
    // Every read below is an offset from the mapped base, and each is bounded by the
    // header field that precedes it — a module whose headers do not parse yields `None`
    // rather than a guess, which is the same answer the pre-loft#907 path gave.
    let rd32 = |off: usize| -> u32 { unsafe { base.add(off).cast::<u32>().read_unaligned() } };
    let rd16 = |off: usize| -> u16 { unsafe { base.add(off).cast::<u16>().read_unaligned() } };
    if rd16(0) != 0x5A4D {
        return None; // not `MZ` — not a PE image
    }
    let pe = rd32(0x3C) as usize;
    if rd32(pe) != 0x0000_4550 {
        return None; // not the PE signature `P`,`E`,NUL,NUL
    }
    // The export directory's RVA sits at a different offset in PE32 vs PE32+, and the
    // magic in the optional header is what tells them apart.
    let opt = pe + 24;
    let export_rva = match rd16(opt) {
        0x20B => rd32(opt + 112) as usize, // PE32+
        0x10B => rd32(opt + 96) as usize,  // PE32
        _ => return None,
    };
    let export_size = match rd16(opt) {
        0x20B => rd32(opt + 116) as usize,
        _ => rd32(opt + 100) as usize,
    };
    if export_rva == 0 {
        return None; // the module exports nothing
    }
    let names = rd32(export_rva + 32) as usize;
    let name_count = rd32(export_rva + 24) as usize;
    let functions = rd32(export_rva + 28) as usize;
    let ordinals = rd32(export_rva + 36) as usize;
    for i in 0..name_count {
        let ordinal = rd16(ordinals + i * 2) as usize;
        let func_rva = rd32(functions + ordinal * 4) as usize;
        // An RVA inside the export directory is a FORWARDER string, not code — it names
        // another module's export and has no address here.
        if func_rva >= export_rva && func_rva < export_rva + export_size {
            continue;
        }
        if !std::ptr::eq(unsafe { base.add(func_rva) }.cast::<()>().cast_const(), ptr) {
            continue;
        }
        // Exact hit. Mirrors the `dli_saddr == ptr` requirement on unix.
        let name_ptr = unsafe { base.add(rd32(names + i * 4) as usize) };
        return unsafe { std::ffi::CStr::from_ptr(name_ptr.cast()) }
            .to_str()
            .ok()
            .map(str::to_string);
    }
    None
}

// ── a file mapping two processes share ──────────────────────────────────────

/// Map the first `len` bytes of `file` read-write and SHARED (`MAP_SHARED`), so a write from
/// either process that maps the file is seen by the other.  Answers the page-aligned base.
///
/// # Errors
/// The OS's error from `mmap`; `Unsupported` where no shared mapping is implemented.
///
/// Windows: `Unsupported`, so a placed library cannot get its transport; exemption
/// candidate: the substitute is `CreateFileMappingW` + `MapViewOfFile`, unwritten (and the
/// worker's parent-death watch, Track P, is the other half of placement on Windows).
pub fn map_shared_file(file: &File, len: usize) -> std::io::Result<*mut u8> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: a fresh shared mapping of an open descriptor; the kernel picks the address.
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        Ok(base.cast::<u8>())
    }
    #[cfg(not(unix))]
    {
        let _ = (file, len);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "a shared file mapping is not implemented on this host",
        ))
    }
}

/// Undo [`map_shared_file`].
///
/// # Safety
/// `base` and `len` are exactly what one [`map_shared_file`] call answered and was given,
/// and nothing reads or writes the mapping afterwards.
pub unsafe fn unmap_shared(base: *mut u8, len: usize) {
    #[cfg(unix)]
    // SAFETY: the caller's contract.
    unsafe {
        libc::munmap(base.cast::<libc::c_void>(), len);
    }
    #[cfg(not(unix))]
    {
        let _ = (base, len);
    }
}

// ── wait / wake on a word two processes share ──────────────────────────────
//
// One primitive per platform, one contract: `shared_word_wait` returns when the word may no
// longer equal `expect`, when `limit` passes, or spuriously — every caller re-reads the word
// in a loop — and `shared_word_wake` wakes one waiter.  The word lives in a file mapping
// shared by two processes, so every form must be the SHARED one: a process-private wait
// queues on a key the other side never wakes, and every wake is lost.

/// Wait while the shared word `a` holds `expect`, at most `limit` (forever when `None`); may
/// return spuriously.  Linux: the futex itself, shared (no `FUTEX_PRIVATE_FLAG`: the private
/// variant hashes on the mm, so the two processes would queue on different keys).  macOS:
/// `os_sync_wait_on_address` (macOS 14.4), looked up at run time so an older macOS still
/// runs on the polling fallback.
///
/// Windows: the polling fallback — a 200 µs sleep, then return; approved exemption (owner, 2026-10-07):
/// `WaitOnAddress` does not cross processes, and the substitute is a named event pair.
pub fn shared_word_wait(
    a: &std::sync::atomic::AtomicU32,
    expect: u32,
    limit: Option<std::time::Duration>,
) {
    #[cfg(target_os = "linux")]
    {
        let ts = limit.map(|d| libc::timespec {
            tv_sec: d.as_secs() as libc::time_t,
            tv_nsec: libc::c_long::from(d.subsec_nanos()),
        });
        // SAFETY: a futex wait on a live, aligned 32-bit word; the kernel only reads it.
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                std::ptr::from_ref(a),
                libc::FUTEX_WAIT,
                expect,
                ts.as_ref()
                    .map_or(std::ptr::null(), std::ptr::from_ref::<libc::timespec>),
            );
        }
    }
    #[cfg(target_os = "macos")]
    match darwin_wait::api() {
        Some(api) => darwin_wait::wait(api, a, expect, limit),
        None => poll_wait(a, expect, limit),
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    poll_wait(a, expect, limit);
}

/// Wake one waiter on the shared word `a`.  Linux: `FUTEX_WAKE`; macOS:
/// `os_sync_wake_by_address_any` where it exists.
///
/// Windows: nothing — the waiter polls, so there is no one to wake; approved exemption (owner, 2026-10-07): as
/// [`shared_word_wait`].
pub fn shared_word_wake(a: &std::sync::atomic::AtomicU32) {
    #[cfg(target_os = "linux")]
    // SAFETY: a futex wake on a live, aligned 32-bit word.
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            std::ptr::from_ref(a),
            libc::FUTEX_WAKE,
            1i32,
        );
    }
    #[cfg(target_os = "macos")]
    if let Some(api) = darwin_wait::api() {
        darwin_wait::wake(api, a);
    }
    // Without the API the waiter polls, so there is no one to wake.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = a;
}

/// The fallback wait: a short sleep, then return — the contract allows a spurious return,
/// and the caller re-reads the word and waits again.  Correct everywhere; slower than a
/// kernel wait only for an exchange that has already spun past its budget.
#[cfg(not(target_os = "linux"))]
fn poll_wait(a: &std::sync::atomic::AtomicU32, expect: u32, limit: Option<std::time::Duration>) {
    let step = std::time::Duration::from_micros(200);
    if a.load(std::sync::atomic::Ordering::SeqCst) == expect {
        std::thread::sleep(limit.map_or(step, |l| l.min(step)));
    }
}

/// macOS: the public cross-process wait-on-address (`os_sync_wait_on_address`, macOS 14.4),
/// looked up at run time so an older macOS still runs — it falls back to [`poll_wait`].
#[cfg(target_os = "macos")]
mod darwin_wait {
    use std::sync::OnceLock;

    /// `OS_SYNC_WAIT_ON_ADDRESS_SHARED` / `OS_SYNC_WAKE_BY_ADDRESS_SHARED`.
    const SHARED: u32 = 1;
    /// `OS_CLOCK_MACH_ABSOLUTE_TIME`, the one clock the timed wait takes.
    const CLOCK_MACH_ABSOLUTE: u32 = 32;

    type WaitFn = unsafe extern "C" fn(*mut libc::c_void, u64, libc::size_t, u32) -> libc::c_int;
    type WaitTimeoutFn =
        unsafe extern "C" fn(*mut libc::c_void, u64, libc::size_t, u32, u32, u64) -> libc::c_int;
    type WakeFn = unsafe extern "C" fn(*mut libc::c_void, libc::size_t, u32) -> libc::c_int;

    pub(super) struct Api {
        pub wait: WaitFn,
        pub wait_timeout: WaitTimeoutFn,
        pub wake_any: WakeFn,
    }

    fn sym(name: &std::ffi::CStr) -> *mut libc::c_void {
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) }
    }

    /// The three functions, or `None` on a macOS that does not have them.
    pub(super) fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| {
            let (w, wt, k) = (
                sym(c"os_sync_wait_on_address"),
                sym(c"os_sync_wait_on_address_with_timeout"),
                sym(c"os_sync_wake_by_address_any"),
            );
            if w.is_null() || wt.is_null() || k.is_null() {
                return None;
            }
            // SAFETY: each pointer is the named libSystem function, whose signature the
            // types above spell (os/os_sync_wait_on_address.h).
            unsafe {
                Some(Api {
                    wait: std::mem::transmute::<*mut libc::c_void, WaitFn>(w),
                    wait_timeout: std::mem::transmute::<*mut libc::c_void, WaitTimeoutFn>(wt),
                    wake_any: std::mem::transmute::<*mut libc::c_void, WakeFn>(k),
                })
            }
        })
        .as_ref()
    }

    pub(super) fn wait(
        api: &Api,
        a: &std::sync::atomic::AtomicU32,
        expect: u32,
        limit: Option<std::time::Duration>,
    ) {
        let addr = std::ptr::from_ref(a).cast_mut().cast::<libc::c_void>();
        unsafe {
            match limit {
                // The timed form's duration is in nanoseconds of the clock it names.
                Some(d) => {
                    let ns = u64::try_from(d.as_nanos()).unwrap_or(u64::MAX).max(1);
                    (api.wait_timeout)(addr, u64::from(expect), 4, SHARED, CLOCK_MACH_ABSOLUTE, ns);
                }
                None => {
                    (api.wait)(addr, u64::from(expect), 4, SHARED);
                }
            }
        }
    }

    pub(super) fn wake(api: &Api, a: &std::sync::atomic::AtomicU32) {
        let addr = std::ptr::from_ref(a).cast_mut().cast::<libc::c_void>();
        unsafe {
            (api.wake_any)(addr, 4, SHARED);
        }
    }
}
