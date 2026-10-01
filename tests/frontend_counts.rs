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
//! The count is taken in-process by a global allocator armed only around the front end, so
//! the harness's own allocations are outside the window.  The corpus is `bench/frontend`'s,
//! read through `frontend.py --emit`, so both instruments measure the same input.
//!
//! Two pins, because they are two costs (loft#1761):
//! - **`stdlib`** — the COLD stdlib parse alone (`parse_dir` over a pristine copy of
//!   `default/`, no startup cache).  A real install pays it once per stdlib change, not per run.
//! - **`tiny` / `medium`** — the PROGRAM on top of an already-loaded stdlib: the corpus parse,
//!   the scope pass and the post-scope lints, which is what a warm run pays.  Those passes
//!   walk every definition, the stdlib's too, so a new stdlib declaration moves both program
//!   rows by one constant beside its own row; a growth that scales with the corpus is the
//!   front end's.

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

/// Allocations the front end makes compiling `src`, as `(stdlib, program)`: the cold stdlib
/// parse, then the corpus parse, scope pass and post-scope lints on top of it.  Each call is
/// a fresh parser; nothing is cached across calls.
fn front_end_allocations(src: &str, tag: &str) -> (u64, u64) {
    let dir = std::env::temp_dir().join(format!("loft_fc_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("corpus.loft");
    std::fs::write(&file, src).expect("write corpus");
    let path = file.to_string_lossy().to_string();
    let stdlib_dir = pristine_stdlib(&dir);

    ALLOCS.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
    let mut p = loft::parser::Parser::new();
    p.parse_dir(&stdlib_dir, true, false)
        .expect("stdlib parses");
    let stdlib = ALLOCS.with(|c| c.replace(0));
    let parsed = p.parse(&path, false);
    loft::scopes::check(&mut p.data, &mut p.database);
    loft::use_analysis::post_scope_lints(&p.data, &mut p.diagnostics, &path);
    ARMED.with(|a| a.set(false));
    let program = ALLOCS.with(Cell::get);

    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        parsed,
        "the corpus must compile — counting a failed compile counts nothing"
    );
    (stdlib, program)
}

/// A copy of the stdlib's own sources — the `.loft` files of `default/`, nothing else — in
/// `dir/default`, so the count cannot depend on what a run left in the checkout.  Reading
/// a directory costs allocations per ENTRY, skipped or not, so a stray `default/.loft/` (the
/// cache a run on a stdlib file writes beside it) moved the pin by +2 even once the loader
/// skipped it (loft#1761).  The copy is made outside the counting window.
fn pristine_stdlib(dir: &std::path::Path) -> String {
    let to = dir.join("default");
    std::fs::create_dir_all(&to).expect("stdlib copy dir");
    for entry in std::fs::read_dir("default").expect("default/ is readable") {
        let from = entry.expect("default/ entry").path();
        let name = from
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !name.starts_with('.') && name.ends_with(".loft") && from.is_file() {
            std::fs::copy(&from, to.join(&name)).expect("copy a stdlib source");
        }
    }
    to.to_string_lossy().to_string()
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
        "# key\tsize\tallocations — the front end's heap allocations: `stdlib` the cold stdlib\n\
         # parse alone, `tiny`/`medium` bench/frontend's corpus on top of it (a warm run's cost).\n\
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
    // Each counted row and the two runs it took: `stdlib` once per run of either corpus, so
    // four runs must agree on it; each corpus's program half from its own two.
    let mut rows: Vec<(&str, Vec<u64>)> = vec![("stdlib", Vec::new())];
    for size in ["tiny", "medium"] {
        let src = corpus(size);
        front_end_allocations(&src, &format!("{size}-warm"));
        let mut program = Vec::new();
        for i in 0..2 {
            let (stdlib, prog) = front_end_allocations(&src, &format!("{size}{i}"));
            rows[0].1.push(stdlib);
            program.push(prog);
        }
        rows.push((size, program));
    }
    let mut grew_stdlib = Vec::new();
    let mut grew_program = Vec::new();
    for (row, counts) in &rows {
        let n = counts[0];
        let at = pins.iter().position(|(k, s, _)| *k == key && s == row);
        if at.is_none() && !repin {
            eprintln!(
                "{key} {row}: {counts:?} allocations over the runs — no pin for this build; \
                 LOFT_FRONTEND_REPIN=1 records one once they agree"
            );
            continue;
        }
        assert!(
            counts.iter().all(|c| *c == n),
            "the {row} count differs between runs in one process ({counts:?}) — the gate needs \
             an exact count, so find what varies before trusting it"
        );
        if repin {
            match at {
                Some(i) => pins[i].2 = n,
                None => pins.push((key.clone(), (*row).to_string(), n)),
            }
            continue;
        }
        let grew = if *row == "stdlib" {
            &mut grew_stdlib
        } else {
            &mut grew_program
        };
        match at.map(|i| pins[i].2) {
            None => {}
            Some(p) if n > p => grew.push(format!("{row}: {p} → {n} (+{})", n - p)),
            Some(p) if n < p => eprintln!(
                "{key} {row}: {p} → {n} allocations — it fell; re-pin with LOFT_FRONTEND_REPIN=1"
            ),
            Some(_) => {}
        }
    }
    if repin {
        write_pins(&pins);
        return;
    }
    // Both halves in one report: a stdlib change moves the `stdlib` row AND every program row
    // by one constant — the scope pass and the post-scope lints walk every definition on every
    // run, the stdlib's included — so the program rows alone cannot say which change grew.
    let mut report = Vec::new();
    if !grew_stdlib.is_empty() {
        report.push(format!(
            "the cold stdlib parse allocates more than its pin: {}.  A new `default/` declaration \
             moves this row (name the declarations in the commit); with no stdlib change it is \
             the parser's",
            grew_stdlib.join(", ")
        ));
    }
    if !grew_program.is_empty() {
        report.push(format!(
            "compiling a program on a loaded stdlib allocates more than its pin: {}.  The SAME \
             delta on every size, beside a stdlib row that grew, is the scope pass and the lints \
             reading the new stdlib definitions each run; a delta that grows with the corpus is \
             the front end's — find the new allocation (a `to_string()` or `clone()` on a hot \
             path is the usual one) and remove it",
            grew_program.join(", ")
        ));
    }
    assert!(
        report.is_empty(),
        "the front end allocates more than its pins ({key}):\n- {}\nAllocation is where a \
         compile's time goes.  If the growth is meant — a feature that must allocate — re-pin \
         with `LOFT_FRONTEND_REPIN=1 cargo test [--release] --test frontend_counts` and say \
         why in the commit.",
        report.join("\n- ")
    );
}
