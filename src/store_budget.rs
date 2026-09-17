// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I69 — Word-addressed store (the heap ceiling and its attribution)

//! A memory ceiling for the store heap, and the report that says what filled it.
//!
//! A corrupted length or count does not always end in a bad dereference. Often it
//! ends in an allocation: the same fault that SIGSEGVs on one run drives a store to
//! grow without bound on the next. A time bound cannot catch that — a runaway store
//! reaches tens of gigabytes in seconds — and the kernel's OOM killer is worse than
//! useless as a diagnostic, because it reports only that a process died and is free
//! to kill a bystander instead of the culprit.
//!
//! So the ceiling lives here, inside loft, where the thing being allocated still has
//! a name. When a growth would cross it the run stops at that growth and says which
//! type was growing, how big it had already become, where the program was, and how
//! the rest of the heap was distributed. That last part is what turns "out of memory"
//! into a lead: one type holding 4 GiB in ONE store is a runaway length, and the same
//! 4 GiB spread over a million stores is a leak.
//!
//! **Off unless asked.** loft is unbounded by default and a real program may want the
//! whole machine, so a plain run is never capped. Test runs are, because a test that
//! wants tens of gigabytes is a bug either way, and because taking the developer's
//! machine down with it costs far more than the run.
//!
//! # The invariant
//!
//! [`total`] is the sum of `size * 8` over every store that OWNS its buffer. Stores
//! that borrow another's buffer, and file-backed stores whose bytes live in a
//! mapping, own no heap bytes and are not counted. The invariant is re-asserted at
//! every site in `store.rs` that allocates, reallocates or frees such a buffer —
//! `new`, `open`, `resize_store`, `shrink_to`, `clone_locked`, `snapshot_copy` and
//! `Drop`. A site that forgets makes the counter drift, which shows up as a ceiling
//! that trips early rather than as silent damage.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Live bytes across every store that owns its buffer.
static TOTAL: AtomicU64 = AtomicU64::new(0);

/// The ceiling in bytes; `0` means no ceiling, which is the default.
static LIMIT: AtomicU64 = AtomicU64::new(0);

/// The default ceiling for a test run, used when nothing else says otherwise.
/// Generous for anything a test legitimately does, and small enough that a runaway
/// stops while the machine is still comfortable.
pub const DEFAULT_TEST_LIMIT: u64 = 2 << 30; // 2 GiB

/// Live bytes and store count per store type, maintained only while a ceiling is
/// set — with no ceiling there is no report to build, so there is nothing to pay for.
static BY_TYPE: Mutex<BTreeMap<u16, (u64, u32)>> = Mutex::new(BTreeMap::new());

/// Type id → name, so the report can say `Layer` rather than `kt=112`.
/// Filled as stores are created; a type nothing ever allocated needs no name.
static NAMES: Mutex<BTreeMap<u16, String>> = Mutex::new(BTreeMap::new());

/// @PLN140 arc A — live bytes and store count per ALLOCATION SITE, keyed by the
/// bytecode position that allocated the store and the type it holds.
///
/// The same ledger the ceiling keeps, split one level finer. It answers "which loft
/// line is holding the heap", which no report before it could: `LOFT_LEAK_SITES`
/// groups by site but over the LEAKED stores at exit, and [`breakdown`] weighs live
/// bytes but only by type.
static BY_SITE: Mutex<BTreeMap<(u32, u16), (u64, u32)>> = Mutex::new(BTreeMap::new());

/// The high-water mark of [`total`] over the run — the peak the report is about.
static PEAK: AtomicU64 = AtomicU64::new(0);

/// [`BY_SITE`] as it stood at the last capture, with the total it was taken at.
///
/// Cloning the ledger at *every* new high-water mark would cost O(sites) per
/// allocation in a program that grows steadily, so a capture is taken only when live
/// bytes rise a further sixteenth above the captured total. That bounds the number of
/// captures to a logarithm of the peak — and it means the snapshot is taken at *most*
/// of the peak rather than exactly at it, which the report states rather than hides.
static PEAK_SNAPSHOT: Mutex<(u64, Vec<SiteRow>)> = Mutex::new((0, Vec::new()));

/// One row of the site ledger: `(born_at, known_type, bytes, stores)`.
pub type SiteRow = (u32, u16, u64, u32);

/// Whether allocation-site attribution is armed (`LOFT_ALLOC_SITES`).
///
/// Read once. It gates a mutex-guarded map update per allocation and free, which is
/// far too much for an ordinary run and unremarkable for a profiling one.
#[must_use]
pub fn sites_armed() -> bool {
    static ARMED: OnceLock<bool> = OnceLock::new();
    *ARMED.get_or_init(|| std::env::var_os("LOFT_ALLOC_SITES").is_some())
}

/// Move `bytes` of type `kt`, allocated at `born_at`, into the site ledger, and
/// capture the ledger when the run reaches a new high-water mark worth recording.
/// `stores` is 1 when a store went live and 0 when an existing one grew, so the
/// count column stays a store count rather than an event count.
fn site_add(kt: u16, bytes: usize, born_at: u32, total_after: u64, stores: u32) {
    if let Ok(mut by) = BY_SITE.lock() {
        let e = by.entry((born_at, kt)).or_insert((0, 0));
        e.0 += bytes as u64;
        e.1 += stores;
    }
    if total_after <= PEAK.load(Ordering::Relaxed) {
        return;
    }
    PEAK.store(total_after, Ordering::Relaxed);
    let Ok(mut snap) = PEAK_SNAPSHOT.lock() else {
        return;
    };
    // A sixteenth above the captured total — proportional, with no floor, so the
    // FIRST allocation always captures (a program whose whole heap is 3 KiB still
    // gets a report) and the number of captures stays logarithmic in the peak.
    if total_after <= snap.0 + snap.0 / 16 {
        return;
    }
    let Ok(by) = BY_SITE.lock() else { return };
    snap.0 = total_after;
    snap.1 = by
        .iter()
        .map(|(&(pc, kt), &(b, n))| (pc, kt, b, n))
        .collect();
}

/// Take `bytes` of type `kt`, allocated at `born_at`, back out of the site ledger.
fn site_release(kt: u16, bytes: usize, born_at: u32, stores: u32) {
    if let Ok(mut by) = BY_SITE.lock()
        && let Some(e) = by.get_mut(&(born_at, kt))
    {
        e.0 = e.0.saturating_sub(bytes as u64);
        e.1 = e.1.saturating_sub(stores);
        if e.0 == 0 && e.1 == 0 {
            by.remove(&(born_at, kt));
        }
    }
}

/// The peak the run reached, and the site ledger as it stood at the last capture
/// below it: `(peak_bytes, captured_at_bytes, rows)`.
///
/// `captured_at` is reported beside the peak on purpose — a table that claims to
/// describe a 1.5 GiB peak while having been taken at 900 MiB is the kind of
/// plausible-looking wrong answer this plan exists to refuse.
#[must_use]
pub fn peak_sites() -> (u64, u64, Vec<SiteRow>) {
    let peak = PEAK.load(Ordering::Relaxed);
    let Ok(snap) = PEAK_SNAPSHOT.lock() else {
        return (peak, 0, Vec::new());
    };
    (peak, snap.0, snap.1.clone())
}

/// Set the ceiling in bytes. `0` removes it.
pub fn set_limit(bytes: u64) {
    LIMIT.store(bytes, Ordering::Relaxed);
}

/// The ceiling in bytes, or `0` when there is none.
#[must_use]
pub fn limit() -> u64 {
    LIMIT.load(Ordering::Relaxed)
}

/// Live store bytes right now.
#[must_use]
pub fn total() -> u64 {
    TOTAL.load(Ordering::Relaxed)
}

/// Parse a ceiling written the way a person writes one: `2G`, `512M`, `1048576`.
/// A bare number is bytes; `K`/`M`/`G` (upper or lower, optional `B`) scale it.
/// `0` means no ceiling. Returns `None` when the text is not a size at all.
#[must_use]
pub fn parse_limit(text: &str) -> Option<u64> {
    let t = text.trim();
    let t = t.strip_suffix(['b', 'B']).unwrap_or(t);
    let (digits, scale) = match t.chars().last()? {
        'k' | 'K' => (&t[..t.len() - 1], 1u64 << 10),
        'm' | 'M' => (&t[..t.len() - 1], 1u64 << 20),
        'g' | 'G' => (&t[..t.len() - 1], 1u64 << 30),
        _ => (t, 1),
    };
    digits.trim().parse::<u64>().ok()?.checked_mul(scale)
}

/// Read the ceiling from `LOFT_MEMORY_LIMIT`, falling back to `default`.
/// An unparseable value is reported and the default is kept — a typo in a limit
/// must not silently remove the limit.
pub fn apply_env_limit(default: u64) {
    let Ok(v) = std::env::var("LOFT_MEMORY_LIMIT") else {
        set_limit(default);
        return;
    };
    if let Some(bytes) = parse_limit(&v) {
        set_limit(bytes);
    } else {
        eprintln!(
            "loft: LOFT_MEMORY_LIMIT='{v}' is not a size (try 2G, 512M or 0) — \
             keeping the default {}",
            human(default)
        );
        set_limit(default);
    }
}

/// Record what a store type is called, so the report can name it.
pub fn note_type_name(kt: u16, name: &str) {
    if name.is_empty() || (LIMIT.load(Ordering::Relaxed) == 0 && !sites_armed()) {
        return;
    }
    if let Ok(mut names) = NAMES.lock() {
        names.entry(kt).or_insert_with(|| name.to_string());
    }
}

/// A store of type `kt`, allocated at bytecode position `born_at`, took `bytes` more
/// heap.
pub(crate) fn add(kt: u16, bytes: usize, born_at: u32) {
    let total = TOTAL.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64;
    if sites_armed() {
        site_add(kt, bytes, born_at, total, 1);
    }
    if LIMIT.load(Ordering::Relaxed) == 0 {
        return;
    }
    if let Ok(mut by) = BY_TYPE.lock() {
        let e = by.entry(kt).or_insert((0, 0));
        e.0 += bytes as u64;
        e.1 += 1;
    }
}

/// Subtract from the running total, stopping at zero.
///
/// **This ledger is per-LINKAGE-UNIT, and a store outlives the one it was created in.**
/// A library's native cdylib links its own copy of libloft, so it has its own `TOTAL`,
/// starting at zero — and a store the host allocated is routinely released through the
/// library's `OpDatabase`, which reaches this function with `bytes` the cdylib's ledger
/// never saw. A plain `fetch_sub` then wrapped `TOTAL` to near `u64::MAX`, and the next
/// allocation's `TOTAL + bytes` aborted the program with "attempt to add with overflow" —
/// pointing at whichever allocation came next rather than at the crossing, and taking a
/// CORRECT program down with it (loft#862: `moros_glb_cli_end_to_end`, a debug build).
///
/// So the floor is real, not defensive rounding: below zero is not a quantity of heap,
/// it is the ledger being asked about bytes that belong to another one. What it costs is
/// stated rather than hidden — the HOST's total still counts those bytes as live, because
/// its own ledger never sees the release either, so the ceiling reads high for a program
/// that frees inside a library. That direction is the safe one (it can refuse early, never
/// late) and it is the pre-existing behaviour; making the two ledgers one is a separate
/// change that needs a `loft_ffi` hand-off.
fn sub_total(bytes: u64) {
    // `update`, not `try_update`: the saturating subtraction cannot fail, so there is no
    // `None` case to report.  (Both replace the deprecated `fetch_update`.)
    TOTAL.update(Ordering::Relaxed, Ordering::Relaxed, |t| {
        t.saturating_sub(bytes)
    });
}

/// A store of type `kt`, allocated at bytecode position `born_at`, gave `bytes` of
/// heap back.
pub(crate) fn release(kt: u16, bytes: usize, born_at: u32) {
    sub_total(bytes as u64);
    if sites_armed() {
        site_release(kt, bytes, born_at, 1);
    }
    if LIMIT.load(Ordering::Relaxed) == 0 {
        return;
    }
    if let Ok(mut by) = BY_TYPE.lock()
        && let Some(e) = by.get_mut(&kt)
    {
        e.0 = e.0.saturating_sub(bytes as u64);
        e.1 = e.1.saturating_sub(1);
    }
}

/// A store of type `kt`, created at bytecode position `born_at`, is about to grow
/// from `old` to `new` bytes.
///
/// # Panics
/// When the growth would cross the ceiling.  The panic carries the full report and
/// fires BEFORE the reallocation, so the store is left exactly as it was and the
/// message describes the growth that was refused rather than one already made.
pub(crate) fn grow(kt: u16, old: usize, new: usize, born_at: u32) {
    let added = new.saturating_sub(old) as u64;
    let cap = LIMIT.load(Ordering::Relaxed);
    assert!(
        cap == 0 || TOTAL.load(Ordering::Relaxed) + added <= cap,
        "{}",
        refusal(kt, old, new, cap, born_at)
    );
    let total = TOTAL.fetch_add(added, Ordering::Relaxed) + added;
    if sites_armed() {
        site_add(kt, added as usize, born_at, total, 0);
    }
    if cap == 0 {
        return;
    }
    if let Ok(mut by) = BY_TYPE.lock() {
        by.entry(kt).or_insert((0, 0)).0 += added;
    }
}

/// A store of type `kt`, allocated at bytecode position `born_at`, shrank from `old`
/// to `new` bytes.
pub(crate) fn shrink(kt: u16, old: usize, new: usize, born_at: u32) {
    let freed = old.saturating_sub(new) as u64;
    sub_total(freed);
    if sites_armed() {
        site_release(kt, freed as usize, born_at, 0);
    }
    if LIMIT.load(Ordering::Relaxed) == 0 {
        return;
    }
    if let Ok(mut by) = BY_TYPE.lock()
        && let Some(e) = by.get_mut(&kt)
    {
        e.0 = e.0.saturating_sub(freed);
    }
}

/// A store's bytes moved from one ledger key to another — its type was named, or its
/// allocation site was stamped, after the buffer already existed.
///
/// A store is routinely created before either fact is known: `Store::new` files its
/// bytes under `(site 0, no type)` and `database_named` stamps the site, then the
/// opcode names the type. Left where they started, every byte in the run would be
/// filed under site 0, which is a report that names nothing. Store slots are also
/// POOLED — a reused slot keeps its buffer and gets a new site — so this is the
/// normal path, not a corner.
/// A store's `bytes` move from type `from` to type `to`, allocated at `born_at`.
///
/// The total does not change, so the move is only owed to the two ledgers keyed by
/// type: the per-type breakdown a ceiling keeps, and the allocation-site ledger.
pub(crate) fn retype(from: u16, to: u16, bytes: usize, born_at: u32) {
    if LIMIT.load(Ordering::Relaxed) == 0 && !sites_armed() {
        return;
    }
    release(from, bytes, born_at);
    add(to, bytes, born_at);
}

pub(crate) fn relabel(from: (u32, u16), to: (u32, u16), bytes: usize) {
    if from == to || !sites_armed() {
        return;
    }
    site_release(from.1, bytes, from.0, 1);
    // The peak cannot move: the same bytes are only changing key, so pass the current
    // total rather than one that would trip a spurious capture.
    site_add(to.1, bytes, to.0, TOTAL.load(Ordering::Relaxed), 1);
}

/// `4.9 GiB`, `180.0 MiB`, `512 B` — a size a person can compare at a glance.
#[must_use]
pub fn human(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")];
    for (scale, name) in UNITS {
        if bytes >= scale {
            #[allow(clippy::cast_precision_loss)] // display only
            return format!("{:.1} {name}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes} B")
}

fn name_of(kt: u16) -> String {
    NAMES
        .lock()
        .ok()
        .and_then(|n| n.get(&kt).cloned())
        .unwrap_or_else(|| "?".to_string())
}

/// The heap by type, biggest first — the part that separates a runaway length from
/// a leak.  At most `top` rows; the rest are summarised.
#[must_use]
pub fn breakdown(top: usize) -> String {
    let Ok(by) = BY_TYPE.lock() else {
        return String::new();
    };
    let mut rows: Vec<(u16, u64, u32)> = by.iter().map(|(&k, &(b, n))| (k, b, n)).collect();
    drop(by);
    rows.sort_by_key(|&(_, b, _)| std::cmp::Reverse(b));
    let mut out = String::new();
    for &(kt, bytes, stores) in rows.iter().take(top) {
        let plural = if stores == 1 { "store" } else { "stores" };
        let _ = writeln!(
            out,
            "    {:<24} kt={:<6} {:>10}  in {stores} {plural}",
            name_of(kt),
            kt,
            human(bytes)
        );
    }
    if rows.len() > top {
        let rest: u64 = rows[top..].iter().map(|&(_, b, _)| b).sum();
        let _ = writeln!(
            out,
            "    … and {} more types holding {}",
            rows.len() - top,
            human(rest)
        );
    }
    out
}

/// The message a refused growth carries.
fn refusal(kt: u16, old: usize, new: usize, cap: u64, born_at: u32) -> String {
    let mut m = format!(
        "loft: store memory limit reached — {} in use, limit {}\n\n  \
         the growth that crossed it\n    a store of type `{}` (kt={kt}) growing {} → {}\n",
        human(total()),
        human(cap),
        name_of(kt),
        human(old as u64),
        human(new as u64),
    );
    // The bytecode position the store was allocated at, and deliberately NOT a
    // `file:line` derived from it.  The interpreter's span table records CALL SITES
    // only, so resolving an arbitrary pc through it returns the nearest span BELOW —
    // which is routinely in an unrelated function, and a diagnostic that sends the
    // reader to the wrong file costs more than one that stays quiet.  `LOFT_STORES=timeline`
    // prints the same pc with a line number, resolved against the denser per-run
    // table this module cannot reach.
    if born_at != 0 {
        let _ = writeln!(m, "    the store was allocated at pc={born_at}");
    }
    let rows = breakdown(8);
    if !rows.is_empty() {
        m.push_str("\n  where the memory is\n");
        m.push_str(&rows);
        m.push_str(
            "\n  One type holding nearly all of it in ONE store is a runaway length;\n  \
             the same total spread over very many stores is a leak.\n",
        );
    }
    m.push_str("\n  Raise or remove the limit with LOFT_MEMORY_LIMIT=<size|0>, e.g. 8G.\n");
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes_a_person_would_write() {
        assert_eq!(parse_limit("1024"), Some(1024));
        assert_eq!(parse_limit("2G"), Some(2 << 30));
        assert_eq!(parse_limit("512M"), Some(512 << 20));
        assert_eq!(parse_limit("4k"), Some(4 << 10));
        assert_eq!(parse_limit("8GB"), Some(8 << 30));
        assert_eq!(parse_limit("0"), Some(0));
        assert_eq!(parse_limit("plenty"), None);
    }

    #[test]
    fn sizes_read_at_a_glance() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2 << 30), "2.0 GiB");
        assert_eq!(human(180 << 20), "180.0 MiB");
    }

    /// loft#862 — releasing bytes this ledger never counted must stop at zero.
    ///
    /// A library's cdylib links its own libloft, so its `TOTAL` starts at zero while the
    /// stores it frees were counted by the host's. A plain `fetch_sub` wrapped to near
    /// `u64::MAX` and the NEXT allocation aborted on `TOTAL + bytes` — naming an innocent
    /// allocation, in a correct program. The assertion is the wrap specifically, not just
    /// "no panic": a saturating floor and a wrap both survive a debug `fetch_sub`, and
    /// only the value tells them apart.
    #[test]
    fn releasing_more_than_was_added_stops_at_zero() {
        // `TOTAL` is a process-wide static, so put back whatever was there. nextest gives
        // each test its own process, but `cargo test` shares one across threads.
        let restore = TOTAL.swap(0, Ordering::Relaxed);

        sub_total(4344); // the crossing size from the filed repro
        assert_eq!(
            TOTAL.load(Ordering::Relaxed),
            0,
            "a release the ledger never saw must leave it at zero, not wrap"
        );

        // And the floor must not cost an honest release its effect.
        TOTAL.store(1000, Ordering::Relaxed);
        sub_total(400);
        assert_eq!(
            TOTAL.load(Ordering::Relaxed),
            600,
            "ordinary release still counts"
        );

        TOTAL.store(restore, Ordering::Relaxed);
    }
}
