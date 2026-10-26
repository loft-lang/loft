// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-y (`@FR-R-CompleteWrite`) — a record built by a COMPLETE literal group
//! skips the default prefill: the parser writes every field explicitly (declared
//! defaults, sentinels, the variant tag), so the emitter proves coverage of every
//! schema field position by the group's contiguous `OpSet*`s and calls the no-prefill
//! twin (`OpDatabaseNP` / `OpNewRecordNP`).  The cell corpus
//! (`bytecode-comparisons/V-y-complete-write-cells.loft`) says the VALUES hold; this
//! pins the EMISSION — which sites earn the twin, and that every declining shape (a
//! vector store, a nested struct arriving by copy, a `__nullable` element whose
//! discriminant the group never writes) keeps the prefill — and the switch
//! (`LOFT_NO_COMPLETE_WRITE=1`).  Read off `--native-emit`.
use loft::file_access as fa;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-y-complete-write-cells.loft";

/// `(function, no-prefill sites, prefilled sites)` — predictions written beside the cells.
const EXPECTED: &[(&str, usize, usize)] = &[
    // c1: every field named.  TWO no-prefill sites for one mint: c1's loop has two admitted
    // chain operators, clears the profitability gate, and its body is emitted twice under
    // the chain guard.  `LOFT_NO_GUARDED_CHAIN=1` gives (1, 0) again.
    ("n_c1", 2, 0),
    ("n_c2", 1, 0), // omitted fields: the parser writes them explicitly
    ("n_c3", 1, 0), // a variant literal writes its own tag
    ("n_c4", 1, 0), // the vector store skips (its group's own OpSetInt4 is exactly the
    // one-u32 prefill, width-equal); the S mint is now a FUSED push (`@FR-R-Mint`'s text
    // clause), which zeroes its slot instead of minting through the template
    ("n_c5", 1, 0), // the FALLBACK literal skips; the cast path is not an emitter site
    ("n_c6", 1, 1), // the inner N literal skips; O (nested by OpCopyRecord) declines
    ("n_c7", 1, 0), // a conditional part still writes its field
    ("n_c8", 4, 2), // the vector store and the discharge fallback skip, and so does the
    // `NullableSome` temporary: its group writes the discriminant and BOTH members of the
    // inline `S` through `OpGetField` (an inline record covered member by member).  The
    // element mints keep the prefill (the discriminant is not in their group's write set).
    // The walk over `outn` is a counted loop (`@FR-R-ForwardWalk`), which admits the
    // guarded chain, so its body — the fallback's mint — is emitted twice, as in c1.
    // `LOFT_NO_GUARDED_CHAIN=1` or `LOFT_NO_FORWARD_WALK=1` gives (3, 2).
    // c9 / c10 — a value of the element type appended WHOLE (`b.ps += [p]`): the one
    // `OpCopyRecord` into the element covers every field, so the heap-free P3 mint skips
    // (n_add_p — its one prefilled site is the live-dispatch prelude's `OpDatabase` that
    // materialises the tuple parameter, not a mint), and so does the text-owning NT mint
    // (n_add_n): its copy is flagged FRESH, so it never reads the slot's handles.  The two
    // drivers mint their bag with a vector store apiece, covered.
    ("n_add_p", 1, 1),
    ("n_add_n", 1, 0),
    ("n_c9", 2, 0),
    ("n_c10", 2, 0),
];

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    fa::read_to_string(out).expect("read the emitted Rust")
}

/// Per emitted function: `(no-prefill sites, prefilled sites)`.
fn counts(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        let row = map.entry(current.clone()).or_default();
        // `@FR-R-FieldAppend` spells a field mint as the field's own append, and a bare
        // vector's mint as the vector's own; the first const argument says whether the slot
        // is prefilled, so each is counted as the same form.
        row.0 += line.matches("OpDatabaseNP(").count()
            + line.matches("OpNewRecordNP(").count()
            + line.matches("field_vector_append::<false>(").count()
            + line.matches("vector_record_append::<false,").count();
        row.1 += line.matches("OpDatabase(").count()
            + line.matches("OpNewRecord(").count()
            + line.matches("field_vector_append::<true>(").count()
            + line.matches("vector_record_append::<true,").count();
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_elides_exactly_the_prefills_predicted() {
    let out = std::env::temp_dir().join("loft_complete_write_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = counts(&rust);
    for (name, np, plain) in EXPECTED {
        let (n, p) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            (n, p),
            (*np, *plain),
            "{name}: (no-prefill, prefilled) sites"
        );
    }
    let _ = fa::remove_file(&out);
}

#[test]
fn the_switch_restores_every_prefill() {
    let out = std::env::temp_dir().join("loft_complete_write_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_COMPLETE_WRITE", "1")]);
    assert_eq!(
        rust.matches("OpDatabaseNP(").count() + rust.matches("OpNewRecordNP(").count(),
        0,
        "under LOFT_NO_COMPLETE_WRITE=1 every record keeps its default prefill"
    );
    let _ = fa::remove_file(&out);
}

const BUFFER_CELLS: &str = "tests/scripts/a-record-built-into-a-callers-slot-needs-no-prefill.loft";

/// The buffer clauses: `(function, no-prefill sites, prefilled sites)`.
const BUFFER_EXPECTED: &[(&str, usize, usize)] = &[
    // the guarded mint heads a covering group: a call's tuple fills the inline record
    ("n_mk", 1, 0),
    // one answer is a call's result, so the buffer is not always written whole here
    ("n_pick", 0, 1),
    // the appended slots handed to `mk` as its buffer: the loop's and the two straight ones
    ("n_c1", 3, 0),
    // the slot handed to `mk` skips; `pick`'s answer arrives through a temporary and ONE
    // whole-record copy flagged FRESH, which covers the slot under the copy clause
    ("n_c2", 2, 0),
];

#[test]
fn a_record_built_into_a_callers_slot_elides_its_prefills() {
    let out = std::env::temp_dir().join("loft_complete_write_buffer.rs");
    let rust = emit(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join(BUFFER_CELLS),
        &out,
        &[],
    );
    let got = counts(&rust);
    for (name, np, plain) in BUFFER_EXPECTED {
        let (n, p) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            (n, p),
            (*np, *plain),
            "{name}: (no-prefill, prefilled) sites"
        );
    }
    let _ = fa::remove_file(&out);
}

/// loft#2193 — a type is complete only when EVERY mint site of it is.  Cell c11 mints `S` by a
/// covering literal and again in a slice copy whose element mint sits directly in the copy
/// loop's body, its copy into the element not FRESH: that element must keep its prefill (the
/// copy releases what the slot's handles hold).  The scan read statement lists of blocks only,
/// so the loop's site was never seen and the literal's coverage marked the whole type — the
/// slice's append then minted with no prefill (`::<false, …>`), a crash under
/// `LOFT_POISON_CLAIM=1` and a release of stale handles without it.
#[test]
fn a_mint_site_in_a_loop_body_keeps_its_type_incomplete() {
    let out = std::env::temp_dir().join("loft_complete_write_c11.rs");
    let rust = emit(&cells(), &out, &[]);
    let start = rust.find("fn n_c11(").expect("n_c11 was emitted");
    let body = &rust[start..];
    let body = &body[..body.find("\n}\n").map_or(body.len(), |e| e + 3)];
    assert!(
        body.contains("vector_record_append::<true,"),
        "the slice's element mint keeps its prefill:\n{body}"
    );
    assert!(
        !body.contains("vector_record_append::<false,"),
        "no mint of S in c11 skips its prefill:\n{body}"
    );
    let _ = fa::remove_file(&out);
}
