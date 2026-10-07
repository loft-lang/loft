// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! @PLN126 — why `store_release` names a FRONTIER and not a record.
//!
//! The plan wanted a program to tell a bound store *"this record is finished"*, so a
//! generator's resident set follows its working set instead of the kernel learning it
//! by evicting the wrong pages first. `madvise(MADV_DONTNEED)` works on PAGES, so a
//! per-record hint can only be honoured where a record's pages are its own — and the
//! plan opened on measuring exactly that, on `routing`'s generator shape: a
//! `hash<TTile[tkey]>` whose tiles own `vector<TRoad>` and `vector<TStep>` grown by
//! append as features arrive.
//!
//! **The answer is no, by two to three orders of magnitude.** A record's span is
//! 356–1348× the bytes it owns and **0.0%** of the 4 KB pages it touches hold only that
//! record — at every window and every scale tried. The cause is not vector
//! reallocation: the hash keeps its entries in a chunked arena claimed EARLY while a
//! record's vectors are claimed at the frontier LATE, so a record's own bytes sit
//! either side of the whole store. `one_tile_footprint_is_the_blocks_it_owns` shows it
//! with a single record and nothing else alive.
//!
//! What survived is the other claim: 87–99% of the pages below a record's finish
//! frontier hold nothing written afterwards. That needs no per-record contiguity, and
//! it is what `Store::release_resident` was built on.
//!
//! **It reads the final layout, not a model of it.** Every word of the arena is painted
//! with the record that owns it, through [`Stores::for_each_owned_child`] — the same
//! ownership walk `remove_claims` frees by, so "which bytes are this record's" has one
//! definition in the tree and this measurement cannot quietly disagree with the
//! runtime. The paint is then read four ways: contiguity, foreign bytes inside a
//! record's span, page exclusivity, and what a frontier hint is allowed to drop.
//!
//! The two sweeps are `#[ignore]` and PRINT rather than assert: they answer a design
//! question, they do not defend an invariant. What they do assert is their own
//! coverage — a walk that missed a block would understate every span it reports, in the
//! direction that makes a per-record release look buildable. The calibration cell is
//! not ignored, and it is what fixes the arithmetic the sweeps are read against.
//!
//! ```text
//! cargo test --release --lib database::spans -- --ignored --nocapture
//! ```
//!
//! Workings and the decision they led to: `doc/claude/plans/126-record-frontier.md`.

// @PLN184 A1 exemption: store persistence keeps its OS calls; the daily windows-latest run proves them.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use super::{Parts, Stores};
use crate::hash;
use crate::keys::DbRef;
use crate::store::PRIMARY;
use crate::vector;

/// The page a hint can drop.
///
/// Not `page_metrics::PAGE` (64 KB), which is a remote reader's FETCH size: the unit
/// `madvise` operates on is the OS page, and a plan about dropping pages has to be
/// measured at the granularity it would drop them. Words, because a record number IS a
/// word offset and every address here is one.
const PAGE_WORDS: u32 = 4096 / 8;

/// Owner sentinels painted into [`Layout::owner`] beside real tile indices.
const FREE: u32 = u32::MAX;
/// Bytes no single tile owns and that stay hot to the end: the root record, the hash's
/// bucket table (rewritten on every rehash, read on every insert) and the slack in the
/// entry arena's chunks. Counted apart rather than folded into a tile, because a page
/// they sit on is not droppable at any point and pretending otherwise would flatter the
/// plan.
const SHARED: u32 = u32::MAX - 1;

/// `routing`'s tile-generator schema, imported rather than invented
/// (`lib/routing_kernel/src/routing_kernel.loft`, `tools/gen-tiles.loft`).
///
/// The three fields that carry the question are the outer `hash` (its entries live
/// packed in a chunked arena, so a tile's own bytes share a record with other tiles from
/// the start) and the two inner vectors (grown by append, so they RELOCATE). The route
/// and barrier vectors the real schema also carries are the same shape again and would
/// only multiply the counts.
struct Shape {
    db: Stores,
    root: DbRef,
    main: u16,
    tiles_tp: u16,
    ttile: u16,
    troad: u16,
    tstep: u16,
    /// Byte offsets inside a `TTile`, and the element strides of its two vectors.
    off_tkey: u32,
    off_roads: u32,
    off_steps: u32,
    road_size: u32,
    step_size: u32,
    /// What one tile occupies in an arena chunk.
    stride: u32,
}

impl Shape {
    fn new() -> Shape {
        let mut db = Stores::new();
        let long = db.name("long");
        // `i32`, not `integer`: `TStep` is three 4-byte fields, and the element stride
        // is what decides how often the vector relocates.
        let int = db.int(0, false);
        let byte = db.byte(0, false);
        let short = db.short(0, false);

        let tstep = db.structure("TStep", 0);
        db.field(tstep, "x", int);
        db.field(tstep, "y", int);
        db.field(tstep, "h", int);

        let troad = db.structure("TRoad", 0);
        db.field(troad, "tp", byte);
        db.field(troad, "flags", short);
        db.field(troad, "nets", short);
        db.field(troad, "steps", byte);

        let vroad = db.vector(troad);
        let vstep = db.vector(tstep);

        let ttile = db.structure("TTile", 0);
        db.field(ttile, "tkey", long);
        db.field(ttile, "ox", long);
        db.field(ttile, "oy", long);
        db.field(ttile, "oh", long);
        db.field(ttile, "roads", vroad);
        db.field(ttile, "steps", vstep);

        let tiles_tp = db.hash(ttile, &["tkey".to_string()]);
        let main = db.structure("Main", 0);
        db.field(main, "tiles", tiles_tp);
        db.finish();

        let root = db.database(8);
        let field = DbRef {
            store_nr: root.store_nr,
            rec: root.rec,
            pos: root.pos + u32::from(db.position(main, "tiles")),
        };
        db.set_default_value(tiles_tp, &field);

        let off_tkey = u32::from(db.position(ttile, "tkey"));
        let off_roads = u32::from(db.position(ttile, "roads"));
        let off_steps = u32::from(db.position(ttile, "steps"));
        let road_size = u32::from(db.size(troad));
        let step_size = u32::from(db.size(tstep));
        let stride = hash::stride_for(u32::from(db.size(ttile)));
        Shape {
            db,
            root,
            main,
            tiles_tp,
            ttile,
            troad,
            tstep,
            off_tkey,
            off_roads,
            off_steps,
            road_size,
            step_size,
            stride,
        }
    }

    /// The `Main.tiles` field — the collection root every walk starts from.
    fn field(&self) -> DbRef {
        DbRef {
            store_nr: self.root.store_nr,
            rec: self.root.rec,
            pos: self.root.pos + u32::from(self.db.position(self.main, "tiles")),
        }
    }

    /// `idx += TTile { tkey: key, … }` — allocate the entry, write the key the hash
    /// will place it by, then insert. Same order the interpreter emits
    /// (`OpNewRecord` → fields → `OpFinishRecord`), because the key must be readable
    /// before `hash::add` decides a bucket.
    fn add_tile(&mut self, key: i64) -> DbRef {
        let tile = self.db.record_new(&self.root, self.main, 0);
        self.db
            .store_mut(&tile)
            .set_long(tile.rec, tile.pos + self.off_tkey, key);
        self.db.record_finish(&self.root, &tile, self.main, 0);
        tile
    }

    /// `t.steps += [TStep{…}]; t.roads += [TRoad{…}]` — one way binned into one tile,
    /// the append pattern `add_way` runs per feature.
    fn add_way(&mut self, tile: &DbRef, steps: u32, seed: i64) {
        let steps_at = DbRef {
            store_nr: tile.store_nr,
            rec: tile.rec,
            pos: tile.pos + self.off_steps,
        };
        for s in 0..steps {
            let slot = vector::vector_append(&steps_at, self.step_size, &mut self.db.allocations);
            let x = u32::from(self.db.position(self.tstep, "x"));
            let y = u32::from(self.db.position(self.tstep, "y"));
            // `TStep`'s fields are 4-byte `i32`s: an 8-byte `set_int` spilled into the next one.
            self.db.store_mut(&slot).set_i32_raw(
                slot.rec,
                slot.pos + x,
                (seed + i64::from(s)) as i32,
            );
            self.db.store_mut(&slot).set_i32_raw(
                slot.rec,
                slot.pos + y,
                (seed - i64::from(s)) as i32,
            );
            vector::vector_finish(&steps_at, &mut self.db.allocations);
        }
        let roads_at = DbRef {
            store_nr: tile.store_nr,
            rec: tile.rec,
            pos: tile.pos + self.off_roads,
        };
        let slot = vector::vector_append(&roads_at, self.road_size, &mut self.db.allocations);
        let tp = u32::from(self.db.position(self.troad, "tp"));
        self.db
            .store_mut(&slot)
            .set_byte(slot.rec, slot.pos + tp, 0, (seed & 15) as i32);
        vector::vector_finish(&roads_at, &mut self.db.allocations);
    }

    /// The arena's high-water mark: the word past the last CLAIMED block. This is the
    /// write frontier a hint would name.
    fn frontier(&self) -> u32 {
        self.db.allocations[self.root.store_nr as usize]
            .usage()
            .live_end_words
    }
}

/// How the stream reaches the tiles. Every axis the matrix moves is here, so a cell
/// that says something surprising can be re-read against what it held fixed.
#[derive(Clone, Copy)]
struct Run {
    /// How many tiles the generator writes.
    tiles: u32,
    /// How many tiles are OPEN at once. `1` is the plan's premise taken at its word —
    /// strict key order, a tile finished before the next is touched. Larger values are
    /// the generator a stream of ways actually produces: a way is keyed by its first
    /// vertex and reaches into neighbours, and a country's feature order is not its
    /// cell order.
    window: u32,
    /// Ways binned into each tile.
    ways: u32,
    /// Vertices per way — what makes `steps` the big vector, as it is in the real store.
    steps: u32,
}

impl Run {
    fn label(self) -> String {
        format!(
            "W={:<3} tiles={:<5} ways={:<3} steps={}",
            self.window, self.tiles, self.ways, self.steps
        )
    }
}

/// The arena after a run, painted one word at a time, plus the derived views every
/// question below is answered from.
///
/// The views are built in ONE pass over the paint rather than per tile. That is not
/// only speed: a per-tile scan is quadratic, and a sweep that has to shrink its corpus
/// to stay affordable is a sweep whose largest cell is the one it never ran.
struct Layout {
    /// Per word: the tile that owns it, or [`FREE`] / [`SHARED`].
    owner: Vec<u32>,
    /// Per tile, in finish order: the lowest and one past the highest word it occupies,
    /// its live word count, and the frontier when it finished.
    lo: Vec<u32>,
    hi: Vec<u32>,
    live: Vec<u32>,
    frontier: Vec<u32>,
    /// Live words in `owner[..w]`, so "live words in a span" is a subtraction.
    live_upto: Vec<u32>,
    /// Per page: live words on it, the highest tile index living on it, and whether any
    /// of the collection's spine does.
    page_live: Vec<u32>,
    page_last: Vec<u32>,
    page_spine: Vec<bool>,
    /// Per tile: the pages it has words on, and how many on each.
    tile_pages: Vec<Vec<(u32, u32)>>,
    words: u32,
}

impl Layout {
    fn read(owner: Vec<u32>, frontier: Vec<u32>, tiles: usize, words: u32) -> Layout {
        let pages = (words.div_ceil(PAGE_WORDS) + 1) as usize;
        let mut out = Layout {
            lo: vec![u32::MAX; tiles],
            hi: vec![0; tiles],
            live: vec![0; tiles],
            live_upto: Vec::with_capacity(owner.len() + 1),
            page_live: vec![0; pages],
            page_last: vec![0; pages],
            page_spine: vec![false; pages],
            tile_pages: vec![Vec::new(); tiles],
            frontier,
            owner,
            words,
        };
        let mut seen = 0;
        out.live_upto.push(0);
        for (at, owner) in out.owner.iter().enumerate() {
            let (at, owner) = (at as u32, *owner);
            if owner != FREE {
                seen += 1;
                let page = (at / PAGE_WORDS) as usize;
                out.page_live[page] += 1;
                if owner == SHARED {
                    out.page_spine[page] = true;
                } else {
                    out.page_last[page] = out.page_last[page].max(owner);
                    let tile = owner as usize;
                    out.lo[tile] = out.lo[tile].min(at);
                    out.hi[tile] = out.hi[tile].max(at + 1);
                    out.live[tile] += 1;
                    match out.tile_pages[tile].last_mut() {
                        Some((last, n)) if *last == page as u32 => *n += 1,
                        _ => out.tile_pages[tile].push((page as u32, 1)),
                    }
                }
            }
            out.live_upto.push(seen);
        }
        out
    }
}

/// Paint every word of `rec` with `who`, refusing to paint a word twice.
///
/// The refusal is the harness's own gate. Two tiles cannot own one byte, so a double
/// paint means the walk followed an edge twice or a stale handle — and a footprint
/// built from a walk that double-counts reports spans that are too SMALL, which is the
/// direction that makes the plan look buildable.
fn paint(owner: &mut [u32], at: u32, words: u32, who: u32) {
    for w in at..at + words {
        assert_eq!(
            owner[w as usize], FREE,
            "word {w} painted twice: already {}, now {who}",
            owner[w as usize]
        );
        owner[w as usize] = who;
    }
}

/// Paint the whole block at record `rec`, header word included — a block is claimed or
/// it is not, and the size word is part of what the tile costs.
fn paint_block(shape: &Shape, owner: &mut [u32], rec: u32, who: u32) {
    let words = shape.db.allocations[shape.root.store_nr as usize].record_words(rec);
    paint(owner, rec, words, who);
}

/// Every block the record at `at` owns BELOW itself, painted with `who`.
///
/// Reached through [`Stores::for_each_owned_child`], the Cluster-C ownership keystone.
/// A second enumeration of "what a record owns" would be a second definition of
/// ownership, and which bytes belong to whom is the entire question.
fn paint_owned(shape: &Shape, owner: &mut [u32], at: &DbRef, tp: u16, who: u32) {
    let walk = shape.db.for_each_owned_child(at, tp);
    if let Some(c) = walk.container_rec {
        paint_block(shape, owner, c, who);
    }
    for r in &walk.extra_recs {
        paint_block(shape, owner, *r, who);
    }
    for ch in &walk.children {
        if let Some(elem) = ch.owning_elem {
            paint_block(shape, owner, elem, who);
        }
        paint_owned(shape, owner, &ch.child, ch.child_tp, who);
    }
    // A text is a leaf to the keystone ("no cascade") and it is still a BLOCK. This
    // shape has none; the arm is here so adding one cannot silently under-report.
    if matches!(shape.db.types[tp as usize].parts, Parts::Base)
        && shape.db.types[tp as usize].name == "text"
    {
        let store = &shape.db.allocations[at.store_nr as usize];
        let r = store.get_u32_raw(at.rec, at.pos);
        if r != 0 && r < store.capacity_words() {
            paint_block(shape, owner, r, who);
        }
    }
}

/// Run the generator and paint what it built.
fn measure(run: Run) -> Layout {
    let mut shape = Shape::new();
    let mut tiles: Vec<DbRef> = Vec::with_capacity(run.tiles as usize);
    let mut frontier: Vec<u32> = Vec::with_capacity(run.tiles as usize);

    let mut next = 0;
    while next < run.tiles {
        let end = (next + run.window).min(run.tiles);
        let group: Vec<DbRef> = (next..end).map(|k| shape.add_tile(i64::from(k))).collect();
        // The ways of a group arrive interleaved: that is what an open window MEANS —
        // one tile's vector cannot grow into the free tail because another tile's block
        // now follows it.
        for w in 0..run.ways {
            for (i, t) in group.iter().enumerate() {
                let seed = i64::from(next + i as u32) * 1_000 + i64::from(w);
                shape.add_way(t, run.steps, seed);
            }
        }
        // Every tile in the group is finished here, at one frontier.
        let mark = shape.frontier();
        for t in group {
            tiles.push(t);
            frontier.push(mark);
        }
        next = end;
    }

    let store_nr = shape.root.store_nr;
    let words = shape.db.allocations[store_nr as usize].capacity_words();
    let mut owner = vec![FREE; words as usize];

    // The root record and the collection's own spine: hot to the end by construction.
    paint_block(&shape, &mut owner, PRIMARY, SHARED);
    let field = shape.field();
    let walk = shape.db.for_each_owned_child(&field, shape.tiles_tp);
    if let Some(c) = walk.container_rec {
        paint_block(&shape, &mut owner, c, SHARED);
    }
    for r in &walk.extra_recs {
        paint_block(&shape, &mut owner, *r, SHARED);
    }
    // Then each tile ON TOP of its chunk: the slot is the tile's, the rest of the chunk
    // is not. This is the one place a word is repainted, so it asserts what it is
    // replacing instead of using `paint` — a slot that was not already chunk means the
    // arena handed out an address outside the chunks it reported.
    let slot_words = shape.stride.div_ceil(8);
    for (i, t) in tiles.iter().enumerate() {
        let who = i as u32;
        let slot_lo = t.rec + t.pos / 8;
        for w in slot_lo..slot_lo + slot_words {
            assert_eq!(
                owner[w as usize], SHARED,
                "tile {who}'s slot at word {w} is not inside a chunk the arena reported"
            );
            owner[w as usize] = who;
        }
        paint_owned(&shape, &mut owner, t, shape.ttile, who);
    }

    // Coverage: the paint must account for every claimed word. A walk that missed a
    // block would report spans that are too small and a droppable fraction that is too
    // high — both in the plan's favour.
    let usage = shape.db.allocations[store_nr as usize].usage();
    let painted: u32 = owner.iter().filter(|o| **o != FREE).count() as u32;
    assert!(
        usage.walk_complete,
        "the block chain does not tile the store — the arena is corrupt, not merely fragmented"
    );
    assert_eq!(
        painted,
        usage.claimed_words,
        "the ownership walk painted {painted} of {} claimed words — {} words of live \
         store belong to nothing this measurement can see",
        usage.claimed_words,
        usage.claimed_words.saturating_sub(painted)
    );

    Layout::read(owner, frontier, tiles.len(), words)
}

impl Layout {
    /// Live words inside tile `t`'s span that are NOT tile `t`'s — the direct measure of
    /// what a per-record release would have to drop along with it.
    fn foreign_in_span(&self, t: usize) -> u32 {
        let live = self.live_upto[self.hi[t] as usize] - self.live_upto[self.lo[t] as usize];
        live - self.live[t]
    }

    /// Of the pages tile `t` touches, how many hold nothing else live — the pages a
    /// per-record hint could drop without taking a neighbour with it.
    fn exclusive_pages(&self, t: usize) -> (u32, u32) {
        let mut exclusive = 0;
        for (p, mine) in &self.tile_pages[t] {
            if self.page_live[*p as usize] == *mine {
                exclusive += 1;
            }
        }
        (self.tile_pages[t].len() as u32, exclusive)
    }

    /// What a frontier hint would be ALLOWED to drop when tile `t` finishes.
    ///
    /// A page fully below the frontier is droppable when every live word on it belongs
    /// to a tile already finished. A word owned by a later tile means the page is
    /// written again after the drop; a [`SHARED`] word means it is read again on every
    /// insert. Read off the FINAL layout, which makes this conservative in one known
    /// direction: a block that tile A freed and tile B later took reads as B's, so a
    /// window where it really was droppable is not counted. That is the right
    /// direction — the claim under test is that everything below the frontier IS
    /// droppable.
    fn droppable(&self, t: usize) -> (u32, u32) {
        let pages = self.frontier[t] / PAGE_WORDS;
        let mut ok = 0;
        for p in 0..pages as usize {
            if !self.page_spine[p] && self.page_last[p] <= t as u32 {
                ok += 1;
            }
        }
        (pages, ok)
    }

    /// The arena as runs of one owner — what the calibration cell is read off.
    fn runs(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let mut at = 0usize;
        while at < self.owner.len() {
            let who = self.owner[at];
            let mut till = at;
            while till < self.owner.len() && self.owner[till] == who {
                till += 1;
            }
            let name = match who {
                FREE => "free".to_string(),
                SHARED => "spine".to_string(),
                nr => format!("tile {nr}"),
            };
            let _ = writeln!(out, "  [{at:>6}..{till:>6})  {:>6}w  {name}", till - at);
            at = till;
        }
        out
    }
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((s.len() - 1) as f64 * p) as usize]
}

/// One row of the matrix.
fn report(run: Run) {
    let l = measure(run);
    let n = l.lo.len();
    let ratio: Vec<f64> = (0..n)
        .map(|t| f64::from(l.hi[t] - l.lo[t]) / f64::from(l.live[t].max(1)))
        .collect();
    let foreign: Vec<f64> = (0..n)
        .map(|t| {
            let span = l.hi[t] - l.lo[t];
            f64::from(l.foreign_in_span(t)) / f64::from(span.max(1))
        })
        .collect();
    let (mut touched, mut exclusive) = (0u32, 0u32);
    for t in 0..n {
        let (a, b) = l.exclusive_pages(t);
        touched += a;
        exclusive += b;
    }
    // The frontier claim is asked at the LAST tile of each decile: an early tile has
    // almost no arena below it and would answer "droppable" for lack of anything to
    // drop.
    let mut drop_pages = 0u32;
    let mut drop_ok = 0u32;
    for d in 1..=10 {
        let t = (n * d / 10) - 1;
        let (p, ok) = l.droppable(t);
        drop_pages += p;
        drop_ok += ok;
    }
    println!(
        "{}  span/live {:>5.2} mean {:>6.2} p95 | foreign-in-span {:>5.1}% | \
         exclusive pages {:>5.1}% ({exclusive}/{touched}) | droppable below frontier {:>5.1}% \
         ({drop_ok}/{drop_pages}) | arena {} KB",
        run.label(),
        mean(&ratio),
        pct(&ratio, 0.95),
        100.0 * mean(&foreign),
        100.0 * f64::from(exclusive) / f64::from(touched.max(1)),
        100.0 * f64::from(drop_ok) / f64::from(drop_pages.max(1)),
        l.words / 128,
    );
}

/// The whole matrix, one axis moved at a time from a common base — and the base itself
/// stated, because a sweep that pins four axes while moving one reads as proof and is
/// not.
#[test]
#[ignore = "a design measurement: reads a layout and PRINTS, rather than defending an invariant — run with --release --lib --ignored --nocapture"]
fn record_spans_across_the_generator_matrix() {
    let base = Run {
        tiles: 2000,
        window: 1,
        ways: 8,
        steps: 12,
    };
    println!("\n-- window: how many tiles the stream keeps open at once --");
    for window in [1, 2, 4, 16, 64] {
        report(Run { window, ..base });
    }
    println!("\n-- ways per tile: how much a tile GROWS after it is created --");
    for ways in [1, 4, 32, 128] {
        report(Run { ways, ..base });
        report(Run {
            ways,
            window: 16,
            ..base
        });
    }
    println!("\n-- steps per way: the size of the append, at a fixed count --");
    for steps in [2, 12, 64] {
        report(Run { steps, ..base });
        report(Run {
            steps,
            window: 16,
            ..base
        });
    }
    println!("\n-- tiles: does the answer hold as the arena outgrows a page --");
    for tiles in [200, 2000, 8000] {
        report(Run { tiles, ..base });
        report(Run {
            tiles,
            window: 16,
            ..base
        });
    }
}

/// This process's resident set, in KB — what a frontier hint is supposed to move.
///
/// Field 2 of `/proc/self/statm` is resident PAGES. Read from `/proc` rather than
/// timed or modelled: the plan's cost is memory, and memory is the one thing here that
/// can be observed exactly.
#[cfg(target_os = "linux")]
fn rss_kb() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|f| f.parse().ok())
        .unwrap_or(0);
    pages * 4
}

/// Build the same shape into a store BOUND to a file — the mode @PLN126 is about —
/// and report peak RSS with and without a release at each tile boundary.
///
/// The measurement that decides whether the call is worth having. A hint that ships
/// without one reads well and buys nothing, and nothing about `MADV_DONTNEED`
/// succeeding says the working set followed it.
#[cfg(all(feature = "mmap", target_os = "linux"))]
fn payoff(run: Run, release: u32) -> (u64, u64, u128) {
    let dir = std::env::temp_dir().join(format!("loft-pln126-{}-{}", std::process::id(), release));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("tiles.store");
    let _ = std::fs::remove_file(&path);

    let mut shape = Shape::new();
    // Bind FIRST: the file IS the arena, so what the generator writes lands in
    // file-backed pages instead of anonymous heap. DATABASE.md § "Binding FIRST is the
    // low-memory choice" is the measurement that makes this the only mode worth
    // asking the question in.
    assert!(
        shape.db.bind_path(shape.root.store_nr, &path),
        "bind_path failed — without a file there is nothing to flush to"
    );
    let base = rss_kb();
    let faults0 = minor_faults();
    let start = std::time::Instant::now();
    let mut peak = base;
    let mut next = 0;
    while next < run.tiles {
        let end = (next + run.window).min(run.tiles);
        let group: Vec<DbRef> = (next..end).map(|k| shape.add_tile(i64::from(k))).collect();
        for w in 0..run.ways {
            for (i, t) in group.iter().enumerate() {
                let seed = i64::from(next + i as u32) * 1_000 + i64::from(w);
                shape.add_way(t, run.steps, seed);
            }
        }
        if release > 0 && (next / run.window).is_multiple_of(release) {
            shape.db.allocations[shape.root.store_nr as usize].release_resident();
        }
        peak = peak.max(rss_kb());
        next = end;
    }
    let wall = start.elapsed().as_millis();
    let file = std::fs::metadata(&path).map_or(0, |m| m.len());
    // Read the collection back through the same walk the span measurement uses: a
    // release that lost a byte would show up as a store the ownership walk cannot
    // tile, and a peak RSS of nearly nothing is exactly what a corrupted arena also
    // looks like.
    let usage = shape.db.allocations[shape.root.store_nr as usize].usage();
    assert!(
        usage.walk_complete,
        "the arena no longer tiles after {} releases",
        run.tiles
    );
    assert_eq!(
        crate::hash::count(&shape.field(), &shape.db.allocations),
        run.tiles,
        "the collection lost records"
    );
    let _ = std::fs::remove_dir_all(&dir);
    // The two attributions that cost nothing to keep, and that are the whole reason an
    // interleaved build gains nothing: FREE BLOCKS is how scattered the arena is, and
    // the fault count separates "not dropped" from "dropped and read straight back".
    println!(
        "      [why] minor faults {:>8} | free blocks {:>6} | largest free {:>8} KB",
        minor_faults() - faults0,
        usage.free_count,
        usage.largest_free_words / 128,
    );
    (peak.saturating_sub(base), file / 1024, wall)
}

/// Minor page faults this process has taken — field 10 of `/proc/self/stat`.
///
/// The instrument that tells a page NOT DROPPED from a page dropped and immediately
/// read back. Both look identical in peak RSS, and they need opposite fixes.
#[cfg(target_os = "linux")]
fn minor_faults() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // The second field is the comm, parenthesised and free to contain spaces, so
    // fields are counted from the closing paren rather than from the start.
    let tail = stat.rsplit_once(')').map(|(_, t)| t).unwrap_or_default();
    tail.split_whitespace()
        .nth(7)
        .and_then(|f| f.parse().ok())
        .unwrap_or(0)
}

/// Does the hint move the resident set, and what does it cost?
#[cfg(all(feature = "mmap", target_os = "linux"))]
#[test]
#[ignore = "a design measurement: builds ~90 MB of store and PRINTS peak RSS — run with --release --lib --ignored --nocapture"]
fn a_frontier_release_moves_the_resident_set() {
    for run in [
        Run {
            tiles: 4000,
            window: 1,
            ways: 8,
            steps: 12,
        },
        Run {
            tiles: 4000,
            window: 16,
            ways: 8,
            steps: 12,
        },
        Run {
            tiles: 20000,
            window: 1,
            ways: 8,
            steps: 12,
        },
        Run {
            tiles: 20000,
            window: 16,
            ways: 8,
            steps: 12,
        },
    ] {
        let (off, file, wall_off) = payoff(run, 0);
        println!(
            "{}  file {file:>7} KB | no release: peak RSS {off:>7} KB, wall {wall_off:>5} ms",
            run.label()
        );
        for every in [1, 16, 256] {
            let (on, _, wall_on) = payoff(run, every);
            println!(
                "    release every {every:>4} tiles | peak RSS {on:>7} KB ({:.2}x) | wall {wall_on:>6} ms ({:.1}x)",
                off as f64 / on.max(1) as f64,
                wall_on as f64 / wall_off.max(1) as f64,
            );
        }
    }
}

/// The cell the matrix is calibrated against: ONE tile, whose every block is counted by
/// hand.
///
/// Without it the matrix is a set of numbers that agree with each other. `vector_append`
/// claiming 11 elements, a 40-byte tile in an arena slot, and a bucket table are all
/// things this measurement claims to see; here is the one place they are checked against
/// arithmetic instead of against a second run of the same code.
///
/// It also fixes the FIELD SIZES the matrix's cells are read against — the first draft
/// of this cell was written against a 12-byte `TStep` while the shape had built a
/// 24-byte one, and every span in the sweep was quietly twice what the arithmetic said.
#[test]
fn one_tile_footprint_is_the_blocks_it_owns() {
    let shape = Shape::new();
    assert_eq!(shape.step_size, 12, "TStep is three 4-byte fields");
    assert_eq!(shape.road_size, 6, "TRoad is u8 + u16 + u16 + u8");
    assert_eq!(
        shape.stride, 40,
        "TTile is four longs and two 4-byte handles"
    );

    let l = measure(Run {
        tiles: 1,
        window: 1,
        ways: 1,
        steps: 3,
    });
    assert_eq!(l.lo.len(), 1, "one tile");

    // Three steps and one road fit the minimum claim, so no vector ever relocates and
    // the tile's whole cost is exactly three blocks.
    let steps = (11u32 * 12 + 15) / 8; // `checked_vec_cap(11, 12)` — header included
    let roads = (11u32 * 6 + 15) / 8;
    let slot = shape.stride.div_ceil(8);
    assert_eq!(
        l.live[0],
        steps + roads + slot,
        "one tile owns its arena slot ({slot}w) and its two vectors ({steps}w + {roads}w)"
    );

    // And it is not everything: the bucket table, the chunk slack and the root record
    // are the collection's spine, not this tile's.
    let claimed = l.owner.iter().filter(|o| **o != FREE).count() as u32;
    assert!(
        l.live[0] < claimed,
        "the tile was painted over the collection's own spine: {} of {claimed} claimed words",
        l.live[0]
    );

    // The finding this cell exists to make un-ignorable: with ONE tile in the store and
    // nothing else alive, the tile is already NOT contiguous. Its arena slot and its
    // vectors sit either side of the hash's own spine, so the interval between its
    // corners is several times the bytes it owns — before a second tile exists to
    // interleave with.
    assert!(
        l.foreign_in_span(0) > l.live[0],
        "a lone tile's span holds {} foreign words against its own {}",
        l.foreign_in_span(0),
        l.live[0]
    );
    println!("{}", l.runs());
}
