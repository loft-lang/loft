// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F38 — the test ledger: which arithmetic checks the checked runs proved never fired

//! The TEST LEDGER (@PLN201 step 2): what a program's checked runs prove about its fault
//! sites, so a shipped build can later drop the checks they licensed (step 3, `--ship
//! <ledger>`).
//!
//! A run with `LOFT_LEDGER=<file>` set is an ordinary checked run, on either backend, with
//! every FAULT SITE instrumented.  A fault site is an operator whose checked answer can
//! differ from the processor's — integer `+ - * / %`, negation, `& | ^` and a right shift by a
//! constant (the sites `LOFT_RELEASE_PASS_PROBE` drops).  At each one the run computes the
//! CHECKED answer, which it returns, and the PROCESSOR's answer, and counts the site as run,
//! and as faulted when the two differ: an overflow, a null operand the processor would not
//! carry, a zero divisor.  So "never faulted" means exactly "the shipped form would have
//! answered the same on every run".
//!
//! A site is named by what survives a rebuild of the same source: its function, a hash of
//! that function's body ([`crate::opt::ledger_sites`]), and the operator's ordinal in it.  At
//! exit the counts merge into the file: runs add up, faults add up, so many runs, on either
//! backend and in any order, collect one ledger.  The file is text, one site per line, sorted.
//!
//! The site table is armed per program: the interpreter arms it when the program is closed
//! ([`crate::compile::close_program`]), a native binary at the start of its `main`.  A run
//! with no table armed — a binary built with the ledger, run without `LOFT_LEDGER` — computes
//! the checked answers and records nothing.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

/// The operator kinds, as the instrumented call passes them.  Stable: they are not written to
/// the file (the file names each kind), but a native binary bakes them in.
pub const ADD: i64 = 0;
pub const SUB: i64 = 1;
pub const MUL: i64 = 2;
pub const DIV: i64 = 3;
pub const REM: i64 = 4;
pub const NEG: i64 = 5;
pub const AND: i64 = 6;
pub const OR: i64 = 7;
pub const XOR: i64 = 8;
pub const SHR: i64 = 9;

/// The name each kind is written under in the ledger file.
#[must_use]
pub fn kind_name(kind: i64) -> &'static str {
    match kind {
        ADD => "add",
        SUB => "sub",
        MUL => "mul",
        DIV => "div",
        REM => "rem",
        NEG => "neg",
        AND => "and",
        OR => "or",
        XOR => "xor",
        SHR => "shr",
        _ => "?",
    }
}

/// One fault site of a program: the key a ledger line is written and merged under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    /// The function's definition name (`n_foo`, an instance key, `t_4text_x`).
    pub function: String,
    /// [`crate::opt::ledger_sites::body_hash`] of the function before instrumentation.
    pub body: u64,
    /// The operator's ordinal among the function's fault sites, in pre-order.
    pub ordinal: u32,
    /// One of [`ADD`] … [`SHR`].
    pub kind: i64,
}

/// One program's counters, indexed by site.  Leaked when armed, so a `par` worker that read
/// the pointer just before the next program armed still counts into memory that lives:
/// ledger runs are test runs, a handful of programs per process.
struct Counters {
    runs: Box<[AtomicU64]>,
    faults: Box<[AtomicU64]>,
}

struct Armed {
    path: String,
    sites: Vec<Site>,
    counters: &'static Counters,
}

static ARMED: Mutex<Option<Armed>> = Mutex::new(None);
/// The armed program's counters for [`op`]: one load per operator, no lock.
static COUNTERS: AtomicPtr<Counters> = AtomicPtr::new(std::ptr::null_mut());
/// `--ship`: a shipped build is never instrumented, whatever the environment says.
static SUPPRESSED: AtomicBool = AtomicBool::new(false);
static AT_EXIT: std::sync::Once = std::sync::Once::new();

/// The ledger file this run records into: `LOFT_LEDGER`, unless the build is `--ship`.
#[must_use]
pub fn path() -> Option<String> {
    if SUPPRESSED.load(Ordering::Relaxed) {
        return None;
    }
    std::env::var("LOFT_LEDGER").ok().filter(|p| !p.is_empty())
}

/// Does this build instrument its fault sites?
#[must_use]
pub fn armed_for_build() -> bool {
    path().is_some()
}

/// `--ship` — never instrument this build, and record nothing.
pub fn suppress() {
    SUPPRESSED.store(true, Ordering::Relaxed);
}

/// Arm the site table of the program about to run, writing the previous program's counts
/// first (a test runner runs several programs in one process).  An empty table, or no
/// `LOFT_LEDGER`, arms nothing.
pub fn arm(sites: &[Site]) {
    flush();
    let Some(path) = path() else {
        return;
    };
    if sites.is_empty() {
        return;
    }
    let counters: &'static Counters = Box::leak(Box::new(Counters {
        runs: sites.iter().map(|_| AtomicU64::new(0)).collect(),
        faults: sites.iter().map(|_| AtomicU64::new(0)).collect(),
    }));
    if let Ok(mut slot) = ARMED.lock() {
        *slot = Some(Armed {
            path,
            sites: sites.to_vec(),
            counters,
        });
        COUNTERS.store(std::ptr::from_ref(counters).cast_mut(), Ordering::Release);
    }
    AT_EXIT.call_once(register_at_exit);
}

/// [`arm`] for a native binary: its table is baked into the program as
/// `(function, body hash, ordinal, kind)` rows.
pub fn arm_static(rows: &[(&str, u64, u32, i64)]) {
    let sites: Vec<Site> = rows
        .iter()
        .map(|&(function, body, ordinal, kind)| Site {
            function: function.to_string(),
            body,
            ordinal,
            kind,
        })
        .collect();
    arm(&sites);
}

/// The instrumented operator: the CHECKED answer, exactly what the operator answers in an
/// ordinary run, with the site counted as run and, where the processor's answer would differ,
/// as faulted.  A zero divisor answers null here and is REPORTED by the caller
/// (`__ledger_div`), as `OpDivInt` reports it.
#[must_use]
pub fn op(site: i64, kind: i64, a: i64, b: i64) -> i64 {
    use crate::ops;
    let (checked, plain) = match kind {
        ADD => (ops::op_add_int(a, b), Some(a.wrapping_add(b))),
        SUB => (ops::op_min_int(a, b), Some(a.wrapping_sub(b))),
        MUL => (ops::op_mul_int(a, b), Some(a.wrapping_mul(b))),
        DIV | REM if b == 0 => (i64::MIN, None),
        DIV => (ops::op_div_int(a, b), Some(a.wrapping_div(b))),
        REM => (ops::op_rem_int(a, b), Some(a.wrapping_rem(b))),
        NEG => (ops::op_negate_int(a), Some(a.wrapping_neg())),
        AND => (ops::op_logical_and_int(a, b), Some(a & b)),
        OR => (ops::op_logical_or_int(a, b), Some(a | b)),
        XOR => (ops::op_exclusive_or_int(a, b), Some(a ^ b)),
        // Only a constant shift in 0..64 is a site, so the interpreter's range raise
        // cannot happen here.
        SHR => {
            let checked = if a == i64::MIN || b == i64::MIN {
                i64::MIN
            } else {
                a >> (b & 63)
            };
            (checked, Some(a >> (b & 63)))
        }
        _ => (i64::MIN, None),
    };
    record(site, plain != Some(checked));
    checked
}

fn record(site: i64, faulted: bool) {
    let counters = COUNTERS.load(Ordering::Acquire);
    if counters.is_null() {
        return;
    }
    // SAFETY: a non-null pointer was published from a leaked `&'static Counters`.
    let counters = unsafe { &*counters };
    let Ok(i) = usize::try_from(site) else {
        return;
    };
    if let Some(r) = counters.runs.get(i) {
        r.fetch_add(1, Ordering::Relaxed);
        if faulted {
            counters.faults[i].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Merge the armed program's counts into its ledger file and disarm.  Called when the next
/// program arms, and at process exit.
pub fn flush() {
    let armed = match ARMED.lock() {
        Ok(mut slot) => slot.take(),
        Err(_) => return,
    };
    COUNTERS.store(std::ptr::null_mut(), Ordering::Release);
    let Some(armed) = armed else {
        return;
    };
    let rows: Vec<(Site, u64, u64)> = armed
        .sites
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (
                s.clone(),
                armed.counters.runs[i].load(Ordering::Relaxed),
                armed.counters.faults[i].load(Ordering::Relaxed),
            )
        })
        .collect();
    if let Err(e) = merge_into(&armed.path, &rows) {
        crate::loft_eprintln!("loft: the test ledger {} was not written: {e}", armed.path);
    }
}

/// The ledger file's header line; a file that does not start with it is refused, never
/// overwritten.
pub const HEADER: &str = "# loft test ledger 1: function, body hash, site, operator, runs, faults";

/// One ledger line: the site's key, then its counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub function: String,
    pub body: u64,
    pub ordinal: u32,
    pub kind: String,
    pub runs: u64,
    pub faults: u64,
}

/// Read a ledger file's entries.  `Ok(empty)` for an absent or empty file.
///
/// # Errors
/// A file that is not a ledger, or has a malformed line, named with the line.
pub fn read(path: &str) -> Result<Vec<Entry>, String> {
    let text = match crate::file_access::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Ok(Vec::new()),
    };
    parse(&text).map_err(|e| format!("{path}: {e}"))
}

fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let mut lines = text.lines();
    match lines.next() {
        None => return Ok(Vec::new()),
        Some(h) if h == HEADER => {}
        Some(_) => return Err("not a loft test ledger (its first line is not the header)".into()),
    }
    let mut out = Vec::new();
    for (n, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let bad = || format!("line {} is malformed: {line:?}", n + 2);
        if f.len() != 6 {
            return Err(bad());
        }
        out.push(Entry {
            function: f[0].to_string(),
            body: u64::from_str_radix(f[1], 16).map_err(|_| bad())?,
            ordinal: f[2].parse().map_err(|_| bad())?,
            kind: f[3].to_string(),
            runs: f[4].parse().map_err(|_| bad())?,
            faults: f[5].parse().map_err(|_| bad())?,
        });
    }
    Ok(out)
}

/// The file's text for `entries`, sorted by key so two ledgers of the same runs are equal.
#[must_use]
pub fn render(entries: &[Entry]) -> String {
    use std::fmt::Write as _;
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| {
        (&a.function, a.body, a.ordinal, &a.kind).cmp(&(&b.function, b.body, b.ordinal, &b.kind))
    });
    let mut s = String::from(HEADER);
    s.push('\n');
    for e in sorted {
        let _ = writeln!(
            s,
            "{}\t{:016x}\t{}\t{}\t{}\t{}",
            e.function, e.body, e.ordinal, e.kind, e.runs, e.faults
        );
    }
    s
}

/// Add `rows` into `existing`: a site already there sums its runs and faults.
#[must_use]
pub fn merge(mut existing: Vec<Entry>, rows: &[(Site, u64, u64)]) -> Vec<Entry> {
    let mut at: std::collections::HashMap<(String, u64, u32, String), usize> = existing
        .iter()
        .enumerate()
        .map(|(i, e)| ((e.function.clone(), e.body, e.ordinal, e.kind.clone()), i))
        .collect();
    for (site, runs, faults) in rows {
        let key = (
            site.function.clone(),
            site.body,
            site.ordinal,
            kind_name(site.kind).to_string(),
        );
        if let Some(&i) = at.get(&key) {
            existing[i].runs += runs;
            existing[i].faults += faults;
        } else {
            at.insert(key, existing.len());
            existing.push(Entry {
                function: site.function.clone(),
                body: site.body,
                ordinal: site.ordinal,
                kind: kind_name(site.kind).to_string(),
                runs: *runs,
                faults: *faults,
            });
        }
    }
    existing
}

/// Merge under an exclusive lock on a sibling lock file, so test processes running at once
/// each add their counts and none is lost; the new text replaces the file by a rename.
#[cfg(not(target_arch = "wasm32"))]
fn merge_into(path: &str, rows: &[(Site, u64, u64)]) -> Result<(), String> {
    let lock_path = format!("{path}.lock");
    let lock = crate::file_access::open_lock_file(lock_path.as_str()).map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let existing = read(path)?;
    let text = render(&merge(existing, rows));
    let tmp = format!("{path}.{}.tmp", std::process::id());
    crate::file_access::write(tmp.as_str(), text).map_err(|e| e.to_string())?;
    crate::file_access::rename(tmp.as_str(), path).map_err(|e| e.to_string())?;
    drop(lock);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn merge_into(_path: &str, _rows: &[(Site, u64, u64)]) -> Result<(), String> {
    Ok(())
}

/// Flush at process exit, whichever way the process ends short of a signal: a return from
/// `main` and `std::process::exit` (loft's `exit`, a failing run) both run C `atexit` handlers.
#[cfg(not(target_arch = "wasm32"))]
fn register_at_exit() {
    extern "C" fn at_exit() {
        flush();
    }
    unsafe extern "C" {
        fn atexit(f: extern "C" fn()) -> std::ffi::c_int;
    }
    // SAFETY: `atexit` takes a plain function pointer that lives for the whole process.
    unsafe {
        atexit(at_exit);
    }
}

#[cfg(target_arch = "wasm32")]
fn register_at_exit() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(f: &str, o: u32, k: i64) -> Site {
        Site {
            function: f.to_string(),
            body: 0xab,
            ordinal: o,
            kind: k,
        }
    }

    #[test]
    fn a_merge_adds_runs_and_faults_per_site_and_renders_sorted() {
        let first = merge(
            Vec::new(),
            &[(site("n_b", 0, ADD), 3, 0), (site("n_a", 1, MUL), 2, 1)],
        );
        let both = merge(
            first,
            &[(site("n_b", 0, ADD), 4, 1), (site("n_a", 0, NEG), 1, 0)],
        );
        let text = render(&both);
        assert_eq!(
            text,
            format!(
                "{HEADER}\nn_a\t00000000000000ab\t0\tneg\t1\t0\nn_a\t00000000000000ab\t1\tmul\t2\t1\nn_b\t00000000000000ab\t0\tadd\t7\t1\n"
            )
        );
        let mut sorted = both;
        sorted.sort_by(|a, b| (&a.function, a.ordinal).cmp(&(&b.function, b.ordinal)));
        assert_eq!(
            parse(&text).unwrap(),
            sorted,
            "the file reads back as written"
        );
    }

    #[test]
    fn a_file_that_is_not_a_ledger_is_refused() {
        assert!(parse("hello\n").is_err());
        assert!(parse(&format!("{HEADER}\nn_a\tzz\t0\tadd\t1\t0\n")).is_err());
        assert_eq!(parse("").unwrap(), Vec::new());
    }
}
