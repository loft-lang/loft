// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! @PLN136 — the paged box query answers what the resident one answers, and reads
//! a small part of the image doing it. Module header on `mod paged` in `radix_db.rs`.

use super::{PAYLOAD, add, box_range};
use crate::keys::{DbRef, Key};
use crate::paged_reader::{PagedReader, spatial_box_recs};
use crate::store::Store;

/// An in-memory image that counts the pages asked for — the bounded-read assertion
/// is a COUNT, so the provider has to keep one.
struct Image {
    img: Vec<u8>,
    fetches: usize,
}

impl crate::paged_reader::PageProvider for Image {
    fn size(&self) -> u64 {
        self.img.len() as u64
    }
    fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
        self.fetches += 1;
        let mut buf = vec![0u8; len];
        let start = off as usize;
        if start < self.img.len() {
            let end = (start + len).min(self.img.len());
            buf[..end - start].copy_from_slice(&self.img[start..end]);
        }
        buf
    }
}

/// One integer WIDTH a coordinate axis can have, as the schema spells it.
///
/// Every one of them is a separate decoding on each side — `radix_db`'s `axis_i64`
/// reads a resident store through the checked accessors, `PagedSpatial::axis_value`
/// reads the same bytes out of an image — and the two are only in step because this
/// list drives both. A width tested on one side alone is a width where a record's
/// Morton code can differ between the storages, which does not present as an error:
/// it presents as a box query missing a point inside it.
struct Width {
    type_nr: i8,
    /// Byte offset of axis 1; axis 0 sits at 0.
    stride: u16,
    /// The declared range MINIMUM, which is the bias the narrow encodings store against
    /// (`value - start`) and the `Key::start` every decoder must add back.  A width whose
    /// entries all carry `0` is a width whose bias is never exercised: `u8` is the only
    /// one whose real spelling has a zero bias, so a table of zeroes tests one arm of the
    /// decode and reports it as the whole.
    start: i32,
    /// The coordinate range the width can carry, as `(lo, hi)` — `lo` is `start`, and
    /// `hi` is what the width's own `*_fits` predicate admits above it.
    span: (i64, i64),
    name: &'static str,
}

const WIDTHS: &[Width] = &[
    Width {
        type_nr: 1,
        stride: 8,
        start: 0,
        span: (-40_000, 40_000),
        name: "integer",
    },
    Width {
        type_nr: 2,
        stride: 8,
        start: 0,
        span: (-40_000, 40_000),
        name: "long",
    },
    Width {
        type_nr: 8,
        stride: 4,
        start: 0,
        span: (-40_000, 40_000),
        name: "int32",
    },
    // `0` is the null sentinel, so the stored raw is `value - start + 1`.
    Width {
        type_nr: 9,
        stride: 2,
        start: 0,
        span: (0, 60_000),
        name: "short",
    },
    Width {
        type_nr: 9,
        stride: 2,
        start: 1_000,
        span: (1_000, 61_000),
        name: "short start=1000",
    },
    Width {
        type_nr: 10,
        stride: 1,
        start: 0,
        span: (0, 255),
        name: "byte",
    },
    Width {
        type_nr: 10,
        stride: 1,
        start: -128,
        span: (-128, 127),
        name: "i8",
    },
    Width {
        type_nr: 10,
        stride: 1,
        start: 100,
        span: (100, 355),
        name: "byte start=100",
    },
    // Direct 2-byte: the whole 65536 round-trips, so `hi` sits well above the SIGNED
    // midpoint of the stored word — the region a signed reinterpretation gets wrong.
    Width {
        type_nr: 11,
        stride: 2,
        start: 0,
        span: (0, 65_535),
        name: "u16",
    },
    Width {
        type_nr: 11,
        stride: 2,
        start: -32_768,
        span: (-32_768, 32_767),
        name: "i16",
    },
];

fn keys_for(w: &Width) -> Vec<Key> {
    vec![
        Key {
            type_nr: w.type_nr,
            position: 0,
            start: w.start,
        },
        Key {
            type_nr: w.type_nr,
            position: w.stride,
            start: w.start,
        },
    ]
}

/// Write one axis through the STORE's own setter — the same call a field write makes.
///
/// It is deliberately not a mirror of the `axis_i64` arm that reads it back: a writer
/// written to match the reader shares whatever the reader gets wrong, so the round-trip
/// agrees with itself and never asks the one party that decides what the bytes mean.
fn write_axis(store: &mut Store, rec: u32, key: &Key, v: i64) {
    let p = PAYLOAD + u32::from(key.position);
    let min = key.start;
    let ok = match key.type_nr.unsigned_abs() {
        2 => {
            store.set_long(rec, p, v);
            true
        }
        8 => {
            store.set_i32_raw(rec, p, v as i32);
            true
        }
        9 => store.set_short(rec, p, min, v as i32),
        10 => store.set_byte(rec, p, min, v as i32),
        11 => store.set_i16_raw(rec, p, min, v as i32),
        _ => {
            store.set_int(rec, p, v);
            true
        }
    };
    // A refused write leaves the slot holding the type default, and the read that follows
    // would then be scored against a value never stored — a cell that passes because
    // nothing happened.  The spans are declared to fit; this is what keeps them honest.
    assert!(
        ok,
        "write of {v} refused for type_nr {} min {min}",
        key.type_nr
    );
}

fn lcg(seed: &mut u64) -> i64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (*seed >> 33) as i64
}

/// A clustered point set over `w`'s span: towns, not a uniform grid.
///
/// Clustering is what makes the walk's SKIP matter — a uniform set has every Z-order
/// gap filled, so a walk that stepped through them all would answer identically and
/// cost the same. Every fourth point is duplicated at its neighbour's coordinates,
/// because two records sharing a key differ only in the 32-bit id suffix and that
/// suffix is the only thing ordering them: without a duplicate the suffix region is
/// never reached by a comparison whose outcome matters.
fn points(w: &Width) -> Vec<(i64, i64)> {
    let (lo, hi) = w.span;
    let span = hi - lo;
    let town = (span / 40).max(1);
    let mut seed = 0x51ee_2c40_9a17_0031_u64;
    let mut out = Vec::new();
    for _ in 0..12 {
        let (cx, cy) = (lo + lcg(&mut seed) % span, lo + lcg(&mut seed) % span);
        for _ in 0..60 {
            let x = (cx + lcg(&mut seed) % town).clamp(lo, hi);
            let y = (cy + lcg(&mut seed) % town).clamp(lo, hi);
            out.push((x, y));
            if out.len() % 4 == 0 {
                out.push((x, y));
            }
        }
    }
    out
}

/// The corpus as a spatial collection, in shuffled insertion order — the node array
/// scatters exactly as a real build's does, so the paged walk is not handed a layout
/// that happens to be easy.
fn build(w: &Width, relayout: bool) -> (Store, DbRef, Vec<Key>, usize) {
    let keys = keys_for(w);
    let pts = points(w);
    let mut store = Store::new_in_use(1 << 18);
    let coll_rec = store.claim(1);
    let coll = DbRef {
        store_nr: 0,
        rec: coll_rec,
        pos: 4,
    };
    let mut order: Vec<usize> = (0..pts.len()).collect();
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for i in (1..order.len()).rev() {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        order.swap(i, (seed >> 33) as usize % (i + 1));
    }
    let words = 1 + u32::from(w.stride * 2).div_ceil(8);
    for &i in &order {
        let (x, y) = pts[i];
        let rec = store.claim(words);
        store.zero_fill(rec);
        write_axis(&mut store, rec, &keys[0], x);
        write_axis(&mut store, rec, &keys[1], y);
        add(
            &coll,
            &DbRef {
                store_nr: 0,
                rec,
                pos: PAYLOAD,
            },
            std::slice::from_mut(&mut store),
            &keys,
        );
    }
    if relayout {
        // What `store_persist_bind` does before it writes the image, so this is the
        // layout a reader actually pages.
        let tree = store.get_u32_raw(coll.rec, coll.pos);
        assert!(crate::radix_tree::rtree_relayout(&mut store, tree));
    }
    (store, coll, keys, pts.len())
}

fn reader_over(store: &Store, page: usize, cache: usize) -> PagedReader<Image> {
    PagedReader::with_config(
        Image {
            img: store.raw_bytes().to_vec(),
            fetches: 0,
        },
        page,
        cache,
    )
}

/// The boxes every comparison runs over: viewport-sized, town-sized, the whole
/// plane, the degenerate strips, and shapes with no match.
fn boxes(w: &Width) -> Vec<([i64; 2], [i64; 2])> {
    let (lo, hi) = w.span;
    let span = hi - lo;
    let mut seed = 0x77aa_33cc_11ee_9988_u64;
    let mut out = vec![
        // Everything, and nothing.
        ([lo, lo], [hi, hi]),
        ([hi, hi], [hi, hi]),
        // The degenerate strips: a box over a Morton code is several disjoint runs,
        // and these are the shapes where it is MOST of them.
        ([lo, lo + span / 2], [hi, lo + span / 2 + span / 200]),
        ([lo + span / 2, lo], [lo + span / 2 + span / 200, hi]),
        // Corner-swapped: the same box, written the other way round.
        ([hi, hi], [lo, lo]),
    ];
    for _ in 0..40 {
        let (cx, cy) = (lo + lcg(&mut seed) % span, lo + lcg(&mut seed) % span);
        for div in [4i64, 20, 200] {
            let r = (span / div).max(1);
            out.push(([cx - r, cy - r], [cx + r, cy + r]));
        }
    }
    out
}

/// Where two answers first differ, as one line.
///
/// A whole-vector `assert_eq!` prints both sides, and a box over this corpus holds
/// hundreds of records — a screen of numbers that says nothing about WHICH record
/// went missing. The index and the pair around it do.
fn first_difference(got: &[u32], want: &[u32]) -> String {
    match (0..got.len().max(want.len())).find(|&i| got.get(i) != want.get(i)) {
        Some(i) => format!(
            "; first difference at {i}: paged {:?} vs resident {:?}",
            got.get(i),
            want.get(i)
        ),
        None => String::new(),
    }
}

/// **The paged box query answers what the resident one answers — every width, both
/// layouts, and the cap.**
///
/// The comparison is record for record and in order, not a count: a walk that
/// returned the right NUMBER of records from the wrong Z-order run would satisfy a
/// size assertion, and Z-order is exactly where that mistake lives.
#[test]
fn a_paged_box_agrees_with_the_resident_one() {
    for w in WIDTHS {
        for relayout in [false, true] {
            let (store, coll, keys, records) = build(w, relayout);
            let stores = std::slice::from_ref(&store);
            let mut reader = reader_over(&store, 512, 64);
            let mut widest = 0;
            let mut nonempty = 0;
            for (from, till) in boxes(w) {
                for cap in [None, Some(0), Some(1), Some(7)] {
                    let want = box_range(&coll, stores, &keys, &from, &till, cap);
                    let got =
                        spatial_box_recs(&mut reader, coll.rec, coll.pos, &from, &till, &keys, cap);
                    assert!(
                        got == want,
                        "{} box {from:?}..{till:?} cap={cap:?} relayout={relayout}: \
                         paged returned {} records, resident {}{}",
                        w.name,
                        got.len(),
                        want.len(),
                        first_difference(&got, &want)
                    );
                    if cap.is_none() {
                        widest = widest.max(want.len());
                        nonempty += usize::from(!want.is_empty());
                    }
                }
            }
            // Non-vacuity, twice over: a reader answering an empty vector everywhere
            // would satisfy every equality above, and a corpus whose boxes all missed
            // would too. `records` counts the DUPLICATES, so a walk that skipped the
            // second record of a shared coordinate still fails here.
            assert_eq!(
                widest, records,
                "{}: the whole-plane box must reach every record (relayout={relayout})",
                w.name
            );
            assert!(
                nonempty > 20,
                "{}: only {nonempty} boxes held a record — the fixture is not \
                 exercising the walk",
                w.name
            );
        }
    }
}

/// **A box query reads a small part of the image — the claim the whole plan rests
/// on.**
///
/// Every other cell says the paged walk gives the RIGHT answer; a walk that fetched
/// the whole file would satisfy all of them. This one says it gives that answer
/// cheaply, and it is the property that regresses silently — one stray full read
/// inside the descent costs nothing in correctness and everything in what the
/// feature is for.
///
/// Calibrated against a measured control rather than a constant: the same reader
/// walking the WHOLE collection is what "reading everything" costs in these units,
/// so the bound holds whatever the corpus, the page size or the machine.
#[test]
fn a_box_query_fetches_a_small_fraction_of_what_a_full_scan_does() {
    let w = &WIDTHS[0];
    let (store, coll, keys, records) = build(w, true);
    let (lo, hi) = w.span;

    let mut full = reader_over(&store, 512, 4096);
    let scanned = spatial_box_recs(
        &mut full,
        coll.rec,
        coll.pos,
        &[lo, lo],
        &[hi, hi],
        &keys,
        None,
    );
    assert_eq!(scanned.len(), records, "the control must read everything");
    let whole = full.provider().fetches;

    // A town-sized box around a point the corpus actually holds, so this is a real
    // walk and not a miss that returns before fetching anything.
    let centre = {
        let stores = std::slice::from_ref(&store);
        let all = box_range(&coll, stores, &keys, &[lo, lo], &[hi, hi], Some(1));
        let rec = *all.first().expect("the fixture holds records");
        (
            super::axis_i64(&store, rec, &keys[0]),
            super::axis_i64(&store, rec, &keys[1]),
        )
    };
    let r = (hi - lo) / 40;
    let mut one = reader_over(&store, 512, 4096);
    let got = spatial_box_recs(
        &mut one,
        coll.rec,
        coll.pos,
        &[centre.0 - r, centre.1 - r],
        &[centre.0 + r, centre.1 + r],
        &keys,
        Some(8),
    );
    assert!(
        !got.is_empty(),
        "the box must hold the point it is centred on"
    );
    let capped = one.provider().fetches;
    assert!(
        capped * 3 <= whole,
        "a capped box query fetched {capped} pages against {whole} for the whole \
         collection — it is scanning, not walking"
    );
}

/// **The cap bounds the WALK, not just the answer.**
///
/// The distinction is the whole point of a paged query and it is invisible in the
/// records returned: a walk that materialised the box and truncated afterwards
/// answers identically and fetches every page the box covers. So the assertion is a
/// page COUNT — asking for 4 markers out of a crowded box must cost less than asking
/// for all of them.
#[test]
fn a_capped_box_stops_fetching_at_the_cap() {
    let w = &WIDTHS[0];
    let (store, coll, keys, _) = build(w, true);
    let (lo, hi) = w.span;

    let fetches = |cap: Option<usize>| -> (usize, usize) {
        let mut reader = reader_over(&store, 512, 4096);
        let got = spatial_box_recs(
            &mut reader,
            coll.rec,
            coll.pos,
            &[lo, lo],
            &[hi, hi],
            &keys,
            cap,
        );
        (got.len(), reader.provider().fetches)
    };
    let (all_n, all_pages) = fetches(None);
    let (few_n, few_pages) = fetches(Some(4));
    assert_eq!(few_n, 4, "the cap must be respected");
    assert!(
        all_n > 100,
        "the box must hold far more than the cap, else there is nothing to stop \
         early for (got {all_n})"
    );
    assert!(
        few_pages * 4 <= all_pages,
        "capping at 4 of {all_n} records fetched {few_pages} pages against \
         {all_pages} uncapped — the cap is truncating the answer, not the walk"
    );
}

/// **A record's coordinates read the same through both storages, width for width.**
///
/// The agreement test above compares ANSWERS, and two readers that both misread a
/// narrow width the same way would agree on a wrong one. This compares the decoded
/// value against the coordinate that was written, on each side, which is what fixes
/// the answer to the schema rather than to the other reader.
#[test]
fn every_axis_width_decodes_to_what_was_written() {
    for w in WIDTHS {
        let keys = keys_for(w);
        let mut store = Store::new_in_use(1 << 12);
        let words = 1 + u32::from(w.stride * 2).div_ceil(8);
        let (lo, hi) = w.span;
        let probes = [lo, lo + 1, -1, 0, 1, hi - 1, hi];
        for &v in &probes {
            if v < lo || v > hi {
                continue;
            }
            let rec = store.claim(words);
            store.zero_fill(rec);
            write_axis(&mut store, rec, &keys[0], v);
            write_axis(&mut store, rec, &keys[1], v);
            let resident = super::axis_i64(&store, rec, &keys[0]);
            assert_eq!(resident, v, "{} resident read of {v}", w.name);

            let mut reader = reader_over(&store, 512, 64);
            let paged = crate::paged_reader::spatial_axis_value(&mut reader, rec, &keys[0]);
            assert_eq!(paged, v, "{} paged read of {v}", w.name);
        }
    }
}

/// A spatial collection is not a trie, and the paged reader must say so rather than
/// read a coordinate field as a string pointer.
#[test]
fn a_text_key_is_not_a_spatial_one() {
    let w = &WIDTHS[0];
    let (store, coll, keys, _) = build(w, true);
    let mut reader = reader_over(&store, 512, 64);
    let text_keys = vec![Key {
        type_nr: 6,
        position: 0,
        start: 0,
    }];
    assert!(
        spatial_box_recs(
            &mut reader,
            coll.rec,
            coll.pos,
            &[-1, -1],
            &[1, 1],
            &text_keys,
            None
        )
        .is_empty(),
        "a text-keyed collection has no coordinates to walk"
    );
    // And too few coordinates for the collection's axes is a refusal, not a read of
    // whatever sits past the end of the corner.
    assert!(
        spatial_box_recs(&mut reader, coll.rec, coll.pos, &[0], &[1, 1], &keys, None).is_empty(),
        "a corner must name every axis"
    );
}
