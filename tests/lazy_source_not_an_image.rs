// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#994 — a lazy source that EXISTS and is not a store image must report a fault.
//!
//! The binding reported every failure to OBTAIN bytes and no failure to INTERPRET them.
//! A missing file, an HTTP 404 and a refused connection each set `store_lazy_faults` and
//! `store_lazy_error`; an empty file, a truncated download, a directory and an HTTP `200`
//! serving an error page set neither — so an unusable source was indistinguishable from a
//! valid image that simply lacks the key (`faults 0`, `err ""`, `store_verify true`).
//!
//! `PageSource::open` validated nothing: it opened the file and read its size, so a
//! non-image never reached a `refuse_paged` site at all and failed deep inside the load,
//! which has only `false` to return. It now reads the four-byte store signature — the
//! format has always had one — through `Store::has_signature`, the same predicate the
//! startup cache's `is_store_file` uses, so the fact lives in one place. Refusing there
//! is what lets every `refuse_paged` call site inherit a report it already words
//! correctly.
//!
//! Each cell runs in its own process, and the reason is a CONTRACT rather than a bug. A
//! fault is sticky by design: `tests/scripts/129-lazy-bind.loft` pins fail → rebind →
//! succeed and requires the collection to still read faulted, because the working set is
//! missing the row that failed and "healthy" would be the silent wrong answer this
//! channel exists to prevent. Only `store_lazy_clear` clears it. So a single program
//! probing six sources reads the first cell's fault under every later answer — correctly
//! — and separate processes are what make each cell a measurement of its own source.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A directory of this cell's own — the three cells run in parallel and each removes its
/// tree when it is done, so one shared path is one cell deleting another's fixture.
fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("loft_994_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("mkdir");
    d
}

/// Writes a real store image holding ids 7 and 91, and the one-cell probe beside it.
fn fixture(d: &Path) {
    fa::write(
        d.join("mk.loft"),
        "struct Part { const id: integer, name: text }\n\
         fn main() {\n\
         \x20   parts: hash<Part[id]> = [];\n\
         \x20   parts += Part { id: 7, name: \"bolt\" };\n\
         \x20   parts += Part { id: 91, name: \"nut\" };\n\
         \x20   assert(store_persist_copy(parts, \"good.store\"), \"write the image\");\n\
         }\n",
    )
    .expect("write mk");
    let out = loft::platform::process::harness_command(loft_bin())
        .args(["--interpret", "mk.loft"])
        .env("LOFT_TIMEOUT", "120")
        .current_dir(d)
        .output()
        .expect("spawn loft");
    assert!(
        out.status.success() && fa::exists(d.join("good.store")),
        "the fixture image must be written — every cell below compares against it\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    fa::write(
        d.join("one.loft"),
        "struct Part { const id: integer, name: text }\n\
         fn main() {\n\
         \x20   path = arguments()[0] ?? \"\";\n\
         \x20   parts: hash<Part[id]> = [];\n\
         \x20   _b = store_bind_lazy(parts, path);\n\
         \x20   p = parts[7];\n\
         \x20   println(\"null={p == null} faults={store_lazy_faults(parts)} \
         err=[{store_lazy_error(parts)}]\");\n\
         }\n",
    )
    .expect("write one");
}

fn probe(d: &Path, path: &str) -> String {
    let out = loft::platform::process::harness_command(loft_bin())
        .args(["--interpret", "one.loft", path])
        .env("LOFT_TIMEOUT", "120")
        .current_dir(d)
        .output()
        .expect("spawn loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    all.lines()
        .find(|l| l.starts_with("null="))
        .unwrap_or_else(|| panic!("the probe must report\n{all}"))
        .to_string()
}

/// The four rows that were silent, plus a missing file as the shape they should have had
/// all along.
#[test]
fn a_source_that_is_not_an_image_reports_a_fault() {
    let d = dir("silent");
    fixture(&d);
    fa::write(d.join("empty.store"), b"").expect("write");
    fa::write(d.join("junk.store"), b"not-a-store").expect("write");
    fa::write(d.join("rand.store"), vec![0xABu8; 8192]).expect("write");

    for (label, path) in [
        ("a missing file", "no_such.store"),
        ("an empty file", "empty.store"),
        ("eleven bytes of text", "junk.store"),
        ("8 KB of one repeated byte", "rand.store"),
        ("a directory", "."),
    ] {
        let line = probe(&d, path);
        assert!(
            line.contains("faults=1"),
            "{label} cannot be consulted, which is not \"no such key\" — the fault \
             channel must say so (loft#994): {line}"
        );
        assert!(
            !line.contains("err=[]"),
            "{label} must carry a reason, not an empty error (loft#994): {line}"
        );
    }
    let _ = fa::remove_dir_all(&d);
}

/// The control, and the reason the cells above are a comparison rather than an assertion
/// that everything faults: a real image still answers its key, silently and with no fault.
#[test]
fn a_real_image_still_answers_its_key() {
    let d = dir("control");
    fixture(&d);
    let line = probe(&d, "good.store");
    let _ = fa::remove_dir_all(&d);
    assert!(
        line.contains("null=false") && line.contains("faults=0") && line.contains("err=[]"),
        "a valid image must answer key 7 with a quiet channel: {line}"
    );
}

/// The boundary's other side, measured because the marker check moves ONE of these.
///
/// A valid image TRUNCATED to half still answers — the lazy reader touches only the pages
/// it needs and those were intact, and that is unchanged. An image whose first four bytes
/// are overwritten no longer does: it used to answer correctly, which was luck rather
/// than a promise, and refusing a file whose magic is wrong is what a magic number is
/// for. Both directions are pinned so neither moves by accident.
#[test]
fn truncation_still_reads_and_a_broken_signature_does_not() {
    let d = dir("edges");
    fixture(&d);
    let good = fa::read(d.join("good.store")).expect("read image");

    let half = d.join("half.store");
    fa::write(&half, &good[..good.len() / 2]).expect("write half");
    let line = probe(&d, "half.store");
    assert!(
        line.contains("null=false"),
        "a half-length image still holds the pages this key needs: {line}"
    );

    let mut broken = good;
    broken[..4].fill(0);
    fa::write(d.join("nosig.store"), &broken).expect("write nosig");
    let line = probe(&d, "nosig.store");
    let _ = fa::remove_dir_all(&d);
    assert!(
        line.contains("faults=1") && !line.contains("err=[]"),
        "a file whose store signature is gone is not an image, whatever the rest of it \
         holds (loft#994): {line}"
    );
}
