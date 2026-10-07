// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! @PLN136 step 1 — the measurement. Module header on `mod pages` in `radix_db.rs`.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use super::{Counting, MAX_AXES, PAYLOAD, StoreBox, axis_code, coord_code, morton_words};
use crate::keys::{DbRef, Key};
use crate::page_metrics::{
    CACHE_PAGES, FINE, PAGE, SessionCache, add_span, layouts, mean, pct, row,
};
use crate::radix_tree as rt;
use crate::store::Store;
use std::collections::BTreeSet;

/// Markers one viewport asks for — the spatial counterpart of the search box's 20.
/// A map draws what fits on a screen; nothing about a bounding box says the answer
/// is small, so the surface caps and the cap is what the walk must respect.
const CAP: usize = 200;

/// How many query centres each box shape is measured over.
const PROBES: usize = 200;

/// Coordinates are integers of 1e-7 degrees, which is what an OSM-derived index
/// stores and what `routing`'s `Coord` holds. One degree of latitude is ~111 km, so
/// a unit is ~1.1 cm and the half-widths below read as distances.
const DEG: i64 = 10_000_000;

/// The box shapes a map issues, as (name, half-width x, half-width y).
///
/// Chosen to span what a viewport does — a street, a neighbourhood, a city, a
/// province — plus the two DEGENERATE shapes, because a long thin box is where
/// Z-order is supposed to hurt most: it crosses many cell boundaries per record it
/// contains, which is exactly the case a plan built on "Z-order has 2-D locality"
/// must not skip.
const SHAPES: &[(&str, i64, i64)] = &[
    ("street", DEG / 500, DEG / 500), // ±220 m
    ("viewport", DEG / 50, DEG / 50), // ±2.2 km
    ("city", DEG / 5, DEG / 5),       // ±22 km
    ("province", DEG, DEG),           // ±111 km
    ("wide", DEG, DEG / 500),         // 222 km × 440 m
    ("tall", DEG / 500, DEG),         // 440 m × 222 km
];

/// The corpus file: `x<TAB>y<TAB>name` per line.
fn corpus_path() -> String {
    std::env::var("LOFT_SPATIAL_POINTS").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.cache/loft-spatial/points.tsv")
    })
}

/// Two `integer` axes at payload offsets 0 and 8 — `spatial<Poi[x, y]>`.
fn xy_keys() -> Vec<Key> {
    vec![
        Key {
            type_nr: 1,
            position: 0,
            start: 0,
        },
        Key {
            type_nr: 1,
            position: 8,
            start: 0,
        },
    ]
}

/// The tree every measurement reads, plus the two placements a record can have.
struct Fixture {
    store: Store,
    keys: Vec<Key>,
    tree: u32,
    nodes: u32,
    /// Records in Morton order — the tree's own walk, so a code interval is a
    /// contiguous SLICE of it and a rank is a binary search away.
    ordered: Vec<u32>,
    /// Where each record of `ordered` would sit if an image were written in this
    /// order: `(offset, length)`, the element record and its name together. Parallel
    /// to `ordered`.
    placed: Vec<(u64, u64)>,
    /// Bytes a whole-image download would carry — nodes plus every record.
    image_bytes: u64,
    /// Query centres, drawn from the corpus so every box lands where data is.
    centres: Vec<(i64, i64)>,
}

/// One query's reads, split the way the plan asks for them.
struct Walk {
    /// Node-array byte offsets, which a layout permutes.
    nodes: BTreeSet<u32>,
    /// Records whose coordinates were read — the ones a box test was spent on.
    /// Empty when the caller asked for counts only.
    reads: Vec<u32>,
    /// How many there were, whether or not they were kept.
    read_count: usize,
    /// How many records the box holds, of those returned.
    hits: usize,
}

/// Build the point index, or `None` where the host has no corpus.
fn build() -> Option<Fixture> {
    let path = corpus_path();
    let Ok(text) = crate::file_access::read_to_string(&path) else {
        println!("SKIP — no corpus at {path} (see the module header to build one)");
        return None;
    };
    let pts: Vec<(i64, i64, &str)> = text
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let x = f.next()?.parse().ok()?;
            let y = f.next()?.parse().ok()?;
            Some((x, y, f.next().unwrap_or("")))
        })
        .collect();
    if pts.len() < 100_000 {
        println!("SKIP — only {} points in {path}", pts.len());
        return None;
    }

    let keys = xy_keys();
    // Sized for the whole index up front: growth copies the arena, and the point of
    // this fixture is the tree, not how many times it was reallocated.
    let mut store = Store::new_in_use(1 << 25);
    let coll_rec = store.claim(1);
    let coll = DbRef {
        store_nr: 0,
        rec: coll_rec,
        pos: 4,
    };
    // INSERTION ORDER IS THE VARIABLE UNDER TEST, exactly as it was for the trie:
    // a real index is fed in whatever order its source had, and sorted input would
    // hand the node array the one layout that cannot scatter. Shuffled
    // deterministically so the number is reproducible.
    let mut order: Vec<usize> = (0..pts.len()).collect();
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for i in (1..order.len()).rev() {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        order.swap(i, (seed >> 33) as usize % (i + 1));
    }
    let mut tree = rt::rtree_init(&mut store, 8);
    for &i in &order {
        let (x, y, name) = pts[i];
        let ptr = if name.is_empty() {
            0
        } else {
            store.set_str(name)
        };
        let rec = store.claim(4);
        store.set_int(rec, PAYLOAD, x);
        store.set_int(rec, PAYLOAD + 8, y);
        store.set_u32_raw(rec, PAYLOAD + 16, ptr);
        tree = rt::rtree_insert(&mut store, tree, rec, &super::RadixOracle { keys: &keys });
    }
    store.set_u32_raw(coll.rec, coll.pos, tree);
    let nodes = store.get_u32_raw(tree, rt::NODES);

    // The Morton-order placement: element record and its name, adjacent, in the
    // order the tree walks them — what `store_persist_copy`'s rebuild produces.
    let mut ordered = Vec::with_capacity(pts.len());
    let mut placed = Vec::with_capacity(pts.len());
    let mut at = 24 + 16 * u64::from(nodes);
    let mut it = rt::rtree_first(&store, tree);
    let mut rec = it.rec();
    while rec != 0 {
        let mut len = u64::from(store.record_words(rec)) * 8;
        let ptr = store.get_u32_raw(rec, PAYLOAD + 16);
        if ptr != 0 {
            len += u64::from(store.record_words(ptr)) * 8;
        }
        ordered.push(rec);
        placed.push((at, len));
        at += len;
        rec = it.next(&store, tree).unwrap_or(0);
    }

    let step = (pts.len() / PROBES).max(1);
    let centres = pts
        .iter()
        .step_by(step)
        .take(PROBES)
        .map(|&(x, y, _)| (x, y))
        .collect();
    Some(Fixture {
        store,
        keys,
        tree,
        nodes,
        ordered,
        placed,
        image_bytes: at,
        centres,
    })
}

impl Fixture {
    /// Bytes the node array occupies — `HDR + 16 × nodes`.
    fn node_bytes(&self) -> u64 {
        24 + 16 * u64::from(self.nodes)
    }

    /// A record's full Morton code, as the tree orders it.
    fn code(&self, rec: u32) -> [u64; MAX_AXES] {
        morton_words(2, |a| axis_code(&self.store, rec, &self.keys[a]))
    }

    /// Where `rec` sits in Morton order. The tree keys on `(code, id)`, so the
    /// same pair orders the search.
    fn rank_of(&self, rec: u32) -> usize {
        let want = (self.code(rec), rec);
        self.ordered.partition_point(|&r| (self.code(r), r) < want)
    }

    /// The pruning box walk, with both halves of what it read.
    ///
    /// `keep` records which nodes and records were touched, for the tables that
    /// price a LAYOUT; without it only the counts survive the query, which is what
    /// lets an uncapped province-wide box be measured at all.
    fn walk(&self, c: (i64, i64), hw: (i64, i64), cap: usize, keep: bool) -> Walk {
        let (mut qlo, mut qhi) = ([0u64; MAX_AXES], [0u64; MAX_AXES]);
        qlo[0] = coord_code(c.0 - hw.0);
        qhi[0] = coord_code(c.0 + hw.0);
        qlo[1] = coord_code(c.1 - hw.1);
        qhi[1] = coord_code(c.1 + hw.1);
        let mut src = Counting {
            inner: StoreBox::new(&self.store, self.tree, &self.keys),
            reads: Vec::new(),
            read_count: 0,
            last: 0,
            keep,
        };
        if keep {
            rt::touch_begin();
        }
        let hits = super::box_walk(&mut src, 2, &qlo, &qhi, cap).len();
        Walk {
            nodes: if keep {
                rt::touch_end()
            } else {
                BTreeSet::new()
            },
            reads: src.reads,
            read_count: src.read_count,
            hits,
        }
    }

    /// How many records lie in the CODE INTERVAL between the box's corners — what
    /// the seek-and-filter walk steps through, exactly, without stepping through it.
    ///
    /// The tree's walk order is code order, so the interval is a slice of `ordered`
    /// and its size is two binary searches. Running it would cost a query per
    /// record, which is the very thing the number is here to describe.
    fn interval_len(&self, c: (i64, i64), hw: (i64, i64)) -> usize {
        let lo = morton_words(2, |a| coord_code([c.0 - hw.0, c.1 - hw.1][a]));
        let hi = morton_words(2, |a| coord_code([c.0 + hw.0, c.1 + hw.1][a]));
        let start = self.ordered.partition_point(|&r| self.code(r) < lo);
        let end = self.ordered.partition_point(|&r| self.code(r) <= hi);
        end.saturating_sub(start)
    }

    /// Pages the records `recs` fall on, under the placement the image has today
    /// (as inserted) and the one a rebuilt image would have (Morton order).
    fn record_pages(&self, recs: &[u32], page: u64) -> (usize, usize) {
        let mut built = BTreeSet::new();
        let mut keyed = BTreeSet::new();
        for &r in recs {
            add_span(
                &mut built,
                u64::from(r) * 8,
                u64::from(self.store.record_words(r)) * 8,
                page,
            );
            let (off, len) = self.placed[self.rank_of(r)];
            add_span(&mut keyed, off, len, page);
        }
        (built.len(), keyed.len())
    }
}

/// Header every table repeats, so a printed run says what it was run against.
fn banner(f: &Fixture) {
    println!(
        "\ncorpus {} points | {} nodes | node array {:.2} MB = {} pages of 64 KB\n\
         whole image {:.2} MB = {} pages of 64 KB — the honest baseline, downloaded once",
        f.ordered.len(),
        f.nodes,
        f.node_bytes() as f64 / (1024.0 * 1024.0),
        f.node_bytes().div_ceil(PAGE),
        f.image_bytes as f64 / (1024.0 * 1024.0),
        f.image_bytes.div_ceil(PAGE),
    );
}

/// The node array under five numberings, per box shape.
#[test]
#[ignore = "measurement — run with --release --lib --ignored --nocapture"]
fn box_query_page_span() {
    let Some(f) = build() else { return };
    banner(&f);
    let ls = layouts(&f.store, f.tree, f.nodes);
    println!(
        "\n  distinct pages per query, mean (p95), cap {CAP}      floor = 1 page\n  \
         {:>8} {:>5} {}",
        "box",
        "page",
        row(&ls, |l| format!("{:>13}", l.name))
    );
    for &(name, hx, hy) in SHAPES {
        // One walk alive at a time: the tables are per-probe numbers, and keeping
        // 200 touch sets to summarise afterwards costs gigabytes on the wide shapes.
        let mut cells = vec![[Vec::new(), Vec::new()]; ls.len()];
        let (mut slots, mut read, mut hits) = (Vec::new(), Vec::new(), Vec::new());
        for &c in &f.centres {
            let w = f.walk(c, (hx, hy), CAP, true);
            for (li, l) in ls.iter().enumerate() {
                for (pi, page) in [PAGE, FINE].into_iter().enumerate() {
                    cells[li][pi].push(l.pages(&w.nodes, page));
                }
            }
            slots.push(w.nodes.len());
            read.push(w.read_count);
            hits.push(w.hits);
        }
        for (pi, label) in ["64K", "4K"].into_iter().enumerate() {
            println!(
                "  {name:>8} {label:>5}{}",
                row(&cells, |c| format!(
                    "{:>8.1} ({:>3})",
                    mean(&c[pi]),
                    pct(&c[pi], 0.95)
                ))
            );
        }
        println!(
            "  {:>8} {:>5}  {:.0} node slots read, {:.0} records tested, {:.0} returned",
            "",
            "",
            mean(&slots),
            mean(&read),
            mean(&hits)
        );
    }
}

/// What the CODE INTERVAL holds that the box does not — the question of whether a
/// box query needs to prune at all.
///
/// The surface answers a box; the tree orders by Morton code. Seeking to one corner
/// and walking to the other reads a superset, and how much of a superset is not a
/// matter of taste: it decides whether "read the pages the walk touches" is even a
/// sentence about this kind. Uncapped on both sides, because a cap hides the
/// difference — the interval walk reaches its cap too, just after reading more.
#[test]
#[ignore = "measurement — run with --release --lib --ignored --nocapture"]
fn interval_versus_box() {
    let Some(f) = build() else { return };
    banner(&f);
    println!(
        "\n  records READ to answer one box, uncapped, mean (p95)\n  \
         {:>8} {:>16} {:>16} {:>16} {:>7}",
        "box", "in the box", "box walk reads", "code interval", "ratio"
    );
    for &(name, hx, hy) in SHAPES {
        let (mut hits, mut read, mut ivl) = (Vec::new(), Vec::new(), Vec::new());
        for &c in &f.centres {
            let w = f.walk(c, (hx, hy), usize::MAX, false);
            hits.push(w.hits);
            read.push(w.read_count);
            ivl.push(f.interval_len(c, (hx, hy)));
        }
        println!(
            "  {name:>8} {:>10.0} ({:>4}) {:>10.0} ({:>4}) {:>10.0} ({:>4}) {:>7.1}x",
            mean(&hits),
            pct(&hits, 0.95),
            mean(&read),
            pct(&read, 0.95),
            mean(&ivl),
            pct(&ivl, 0.95),
            mean(&ivl) / mean(&read).max(1.0)
        );
    }
}

/// The records a query READS — a separate allocation with its own layout, and the
/// half the node measurement says nothing about.
///
/// The counterfactual is the placement `store_persist_copy` already produces for a
/// trie: write the records in the collection's own walk order, so a query's records
/// are one run. For a spatial index that order is Morton order, and whether that
/// makes a BOX contiguous is precisely what is in question — a box is not one run
/// in Z-order, so this is not the trie's answer with different words.
#[test]
#[ignore = "measurement — run with --release --lib --ignored --nocapture"]
fn record_placement() {
    let Some(f) = build() else { return };
    banner(&f);
    println!(
        "\n  distinct pages for the records one box query reads, cap {CAP}, mean (p95)\n  \
         {:>8} {:>5} {:>15} {:>15}",
        "box", "page", "as inserted", "Morton order"
    );
    for &(name, hx, hy) in SHAPES {
        let mut cells = [[Vec::new(), Vec::new()], [Vec::new(), Vec::new()]];
        for &c in &f.centres {
            let w = f.walk(c, (hx, hy), CAP, true);
            for (pi, page) in [PAGE, FINE].into_iter().enumerate() {
                let (built, keyed) = f.record_pages(&w.reads, page);
                cells[0][pi].push(built);
                cells[1][pi].push(keyed);
            }
        }
        for (pi, label) in ["64K", "4K"].into_iter().enumerate() {
            println!(
                "  {name:>8} {label:>5}{}",
                row(&cells, |c| format!(
                    "{:>9.1} ({:>4})",
                    mean(&c[pi]),
                    pct(&c[pi], 0.95)
                ))
            );
        }
    }
}

/// One map session: a viewport, then eight pans of half a screen, against a reader
/// that keeps `CACHE_PAGES` pages.
///
/// Every cold number above is what the FIRST viewport costs. A map is panned, and
/// consecutive viewports overlap — they share the top of the tree and half their
/// records — so what a user waits for is the marginal fetch. Nodes and records are
/// counted together here because that is what a fetch is: the two configurations
/// are the image as it is written today and the image a layout pass would write.
#[test]
#[ignore = "measurement — run with --release --lib --ignored --nocapture"]
fn pan_session() {
    let Some(f) = build() else { return };
    banner(&f);
    let ls = layouts(&f.store, f.tree, f.nodes);
    let built = ls.first().expect("as built");
    let veb = ls.last().expect("vEB");
    let steps = 8usize;
    let (hx, hy) = (DEG / 50, DEG / 50);
    println!(
        "\n  pages FETCHED per viewport while panning, {CACHE_PAGES}-page cache, cold at step 1\n  \
         {:>22}{}",
        "image",
        row(1..=steps, |k| format!("{k:>6}"))
    );
    for (label, layout, morton) in [
        ("as built", built, false),
        ("vEB + Morton order", veb, true),
    ] {
        let mut per_step = vec![Vec::new(); steps];
        for &c in &f.centres {
            let mut cache = SessionCache::new();
            for (s, step) in per_step.iter_mut().enumerate().take(steps) {
                // Half a screen east each step: consecutive viewports overlap, which
                // is what makes the marginal cost the interesting one.
                let at = (c.0 + hx * s as i64, c.1);
                let w = f.walk(at, (hx, hy), CAP, true);
                let mut want: BTreeSet<u64> =
                    w.nodes.iter().map(|o| layout.moved(*o) / PAGE).collect();
                // Records live after the node array in a written image, so their
                // pages never collide with a node page.
                for &r in &w.reads {
                    let (off, len) = if morton {
                        f.placed[f.rank_of(r)]
                    } else {
                        (
                            f.node_bytes() + u64::from(r) * 8,
                            u64::from(f.store.record_words(r)) * 8,
                        )
                    };
                    add_span(&mut want, off, len, PAGE);
                }
                step.push(cache.fetch(&want));
            }
        }
        println!(
            "  {label:>22}{}",
            row(&per_step, |v| format!("{:>6.1}", mean(v)))
        );
    }
}
