// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! How many heap allocations loft's front end makes on a fixed corpus (@PLN166 C1).
//!
//! Wall time cannot gate a compiler's speed: it moves with the machine and its load (a
//! loaded box read one mode of `bench/frontend` at twice another's cost while the
//! instruction count said +3.7 %).  An allocation COUNT is exact on one binary and does
//! not move with load, and allocation is where the front end's time goes (libc
//! malloc/free is the largest single share of a compile's profile).  So this counts,
//! and the time stays a report.
//!
//! The count is taken in-process by a global allocator armed only around the front end —
//! the stdlib parse, the corpus parse, the scope pass and the post-scope lints — so the
//! harness's own allocations are outside the window.  The corpus is `bench/frontend`'s,
//! read through `frontend.py --emit`, so both instruments measure the same input.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ARMED.try_with(|a| {
            if a.get() {
                let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let _ = ARMED.try_with(|a| {
            if a.get() {
                let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
            }
        });
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn corpus(size: &str) -> String {
    let out = std::process::Command::new("python3")
        .args(["bench/frontend/frontend.py", "--emit", size])
        .output()
        .expect("python3 bench/frontend/frontend.py --emit");
    assert!(out.status.success(), "frontend.py --emit {size} failed");
    String::from_utf8(out.stdout).expect("utf-8 corpus")
}

/// Allocations the front end makes compiling `src`: stdlib parse, corpus parse, scope
/// pass, post-scope lints.  Each call is a fresh parser; nothing is cached across calls.
fn front_end_allocations(src: &str, tag: &str) -> u64 {
    let dir = std::env::temp_dir().join(format!("loft_fc_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("corpus.loft");
    std::fs::write(&file, src).expect("write corpus");
    let path = file.to_string_lossy().to_string();

    ALLOCS.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, false).expect("stdlib parses");
    let parsed = p.parse(&path, false);
    loft::scopes::check(&mut p.data, &mut p.database);
    loft::use_analysis::post_scope_lints(&p.data, &mut p.diagnostics, &path);
    ARMED.with(|a| a.set(false));
    let n = ALLOCS.with(Cell::get);

    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        parsed,
        "the corpus must compile — counting a failed compile counts nothing"
    );
    n
}

const PINS: &str = "bench/frontend/allocations.tsv";

/// The build this count is valid for: the OS (std allocates differently per platform) and
/// the cargo profile — an unoptimised build allocates ~1 350 more on the medium corpus.
/// The profile is read off this test binary's own path (`target/<profile>/deps/…`), because
/// `cfg!(debug_assertions)` cannot tell them apart here: this repo's dev profile turns the
/// assertions off.  A toolchain upgrade can move a count too, which is a re-pin, never a
/// regression to chase.
fn key() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let profile = exe
        .ancestors()
        .filter_map(|a| a.file_name()?.to_str())
        .find(|n| *n == "debug" || *n == "release")
        .unwrap_or("unknown")
        .to_string();
    format!("{}-{profile}", std::env::consts::OS)
}

fn pinned() -> Vec<(String, String, u64)> {
    std::fs::read_to_string(PINS)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 3 {
                return None;
            }
            Some((f[0].to_string(), f[1].to_string(), f[2].parse().ok()?))
        })
        .collect()
}

fn write_pins(rows: &[(String, String, u64)]) {
    let mut out = String::from(
        "# key\tsize\tallocations — the front end's heap allocations on bench/frontend's corpus.\n\
         # Pinned by `LOFT_FRONTEND_REPIN=1 cargo test [--release] --test frontend_counts`;\n\
         # read by tests/frontend_counts.rs, which fails when a pinned count GROWS.\n",
    );
    let mut rows = rows.to_vec();
    rows.sort();
    for (k, s, n) in rows {
        out.push_str(&format!("{k}\t{s}\t{n}\n"));
    }
    std::fs::write(PINS, out).expect("write the pins");
}

/// **The front end allocates no more than it did** — the gate (@PLN166 C1).
///
/// An allocation count is exact on one binary: three runs in one process and a run in
/// another all read the same count for the medium corpus.  So the pin is
/// exact, and it may only fall.  A count that FELL passes and names the re-pin, so a
/// change that made the front end cheaper is never blocked by its own improvement; a
/// build with no pin (another OS) reports its count and passes until someone pins it.
///
/// The first compile in a process also pays the process's one-time setup, which is no
/// compile's cost and differs by platform — it was 69 allocations on Windows and one on
/// macOS, where it read as two runs disagreeing.  So one uncounted run goes first, and the
/// two counted runs must agree only where a pin is read or written.
///
/// Falsified on its own change: one extra `name.to_string()` in `Data::def_nr` failed it with
/// `tiny: 1077867 → 1400822 (+322955), medium: 3562376 → 4202195 (+639819)` on linux-release —
/// and measured, by the way, how often the front end looks a name up.
#[test]
fn front_end_allocations_do_not_grow() {
    let key = key();
    let repin = std::env::var_os("LOFT_FRONTEND_REPIN").is_some();
    let mut pins = pinned();
    let mut grew = Vec::new();
    for size in ["tiny", "medium"] {
        let src = corpus(size);
        front_end_allocations(&src, &format!("{size}-warm"));
        let counts: Vec<u64> = (0..2)
            .map(|i| front_end_allocations(&src, &format!("{size}{i}")))
            .collect();
        let n = counts[0];
        let at = pins.iter().position(|(k, s, _)| *k == key && s == size);
        if at.is_none() && !repin {
            eprintln!(
                "{key} {size}: {counts:?} allocations over two runs — no pin for this build; \
                 LOFT_FRONTEND_REPIN=1 records one once the two agree"
            );
            continue;
        }
        assert_eq!(
            counts[0], counts[1],
            "the {size} count differs between two runs in one process — the gate needs an \
             exact count, so find what varies before trusting it"
        );
        if repin {
            match at {
                Some(i) => pins[i].2 = n,
                None => pins.push((key.clone(), size.to_string(), n)),
            }
            continue;
        }
        match at.map(|i| pins[i].2) {
            None => {}
            Some(p) if n > p => grew.push(format!("{size}: {p} → {n} (+{})", n - p)),
            Some(p) if n < p => eprintln!(
                "{key} {size}: {p} → {n} allocations — it fell; \
                                           re-pin with LOFT_FRONTEND_REPIN=1"
            ),
            Some(_) => {}
        }
    }
    if repin {
        write_pins(&pins);
        return;
    }
    assert!(
        grew.is_empty(),
        "the front end allocates more than its pin ({key}): {}.\n\
         Allocation is where a compile's time goes; find the new allocation (a `to_string()` \
         or `clone()` on a hot path is the usual one) and remove it.  If the growth is meant \
         — a feature that must allocate — re-pin with \
         `LOFT_FRONTEND_REPIN=1 cargo test [--release] --test frontend_counts` and say why in \
         the commit.",
        grew.join(", ")
    );
}
