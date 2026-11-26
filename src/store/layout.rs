// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I69 — Word-addressed store

//! `@PLN206 P3` — the shared prefix of `Store` against the C runtime's `loft_store_t`: the runtime
//! headers up to `loft_store.h` are compiled with a `main` that prints each field's `offsetof`,
//! and every offset, the bit set's inner layout and the end of the prefix must equal this
//! struct's.  A field added, moved or retyped on one side fails here.
//!
//! @falsified-at: none — the shared layout is new, so the receipt is a SABOTAGE:
//!   with `loft_store_t.live` an `int` again (`src/c/loft_store.h`), every field after it moves.
//!   CHANNEL: the assertion.  WITNESS: "`known`: C offset against Store's".
//!   HOLDS: every offset before `live`.

use super::{Claims, Store, U32List};
use std::fmt::Write as _;
use std::mem::offset_of;

/// `(C field, offset in Store)` for every field of the shared prefix, in the C order.
fn rust_offsets() -> Vec<(&'static str, usize)> {
    vec![
        ("buf", offset_of!(Store, ptr)),
        ("size", offset_of!(Store, size)),
        ("live", offset_of!(Store, live)),
        ("known", offset_of!(Store, known_type)),
        ("pinned", offset_of!(Store, pinned)),
        ("protect", offset_of!(Store, free_protect_depth)),
        ("read_only", offset_of!(Store, read_only)),
        ("serial", offset_of!(Store, alloc_serial)),
        ("claims", offset_of!(Store, claims)),
        ("referenced", offset_of!(Store, referenced)),
        ("removed", offset_of!(Store, removed)),
        ("lazy_free", offset_of!(Store, lazy_free)),
        ("parked_set", offset_of!(Store, parked_set)),
        ("released_root", offset_of!(Store, released_root)),
        ("parked_heads", offset_of!(Store, parked_heads)),
        ("parked_count", offset_of!(Store, parked_count)),
        ("claimed_end", offset_of!(Store, claimed_end)),
        ("free_root", offset_of!(Store, free_root)),
        ("wild", offset_of!(Store, wild)),
        ("dead_words", offset_of!(Store, dead_words)),
        ("needs_coalesce", offset_of!(Store, needs_coalesce)),
        ("lazy", offset_of!(Store, lazy)),
        ("defined", offset_of!(Store, defined)),
        ("no_park", offset_of!(Store, no_park)),
        (
            "placed",
            offset_of!(Store, placed) + offset_of!(U32List, items),
        ),
        (
            "nplaced",
            offset_of!(Store, placed) + offset_of!(U32List, n),
        ),
        (
            "placed_cap",
            offset_of!(Store, placed) + offset_of!(U32List, cap),
        ),
        ("frame_holds", offset_of!(Store, frame_holds)),
        ("orphaned", offset_of!(Store, orphaned)),
        ("legacy_nul", offset_of!(Store, legacy_nul)),
        ("mapped", offset_of!(Store, mapped)),
        ("map_file", offset_of!(Store, map_file)),
        ("closure_shares", offset_of!(Store, c_share_count)),
        ("hgen", offset_of!(Store, hgen)),
    ]
}

/// The C side: `name offset` per line, then the bit set's fields and the prefix's end.
fn c_offsets() -> Option<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/c");
    let mut src = String::new();
    for h in [
        "loft_os.h",
        "loft_width.h",
        "loft_cc.h",
        "loft_cx.h",
        "loft_text.h",
        "loft_store.h",
    ] {
        src += &crate::file_access::read_to_string(root.join(h)).ok()?;
        src.push('\n');
    }
    // `loft_store.h` declares the locked-write refusal `loft_rt.h` defines; an offset probe
    // writes nothing, so it links against a stub.
    src += "static int loft_locked_write_on(loft_cx *cx, uint8_t kind) { (void) cx; (void) kind; return 0; }\n";
    src += "#include <stddef.h>\nint main(void) {\n";
    for (f, _) in rust_offsets() {
        let _ = writeln!(
            src,
            "    printf(\"{f} %zu\\n\", offsetof(loft_store_t, {f}));"
        );
    }
    for f in ["bits", "nwords", "live", "cap"] {
        let _ = writeln!(
            src,
            "    printf(\"bits.{f} %zu\\n\", offsetof(loft_bits, {f}));"
        );
    }
    src += "    printf(\"bits.size %zu\\n\", sizeof(loft_bits));\n";
    src += "    printf(\"end %zu\\n\", offsetof(loft_store_t, hgen) + sizeof(uint64_t));\n";
    src += "    printf(\"hosted %zu\\n\", sizeof(loft_store_t));\n";
    src += "    return 0;\n}\n";
    let dir =
        crate::platform::scratch_dir().join(format!("loft-store-layout-{}", std::process::id()));
    crate::file_access::create_dir_all(&dir).ok()?;
    let c = dir.join("layout.c");
    let exe = dir.join("layout");
    crate::file_access::write(&c, src).ok()?;
    use crate::platform::process::{Program, Spawn};
    let built = Spawn::new(Program::search("cc"))
        .arg(format!("-DLOFT_HOST_STORE_TAIL={}", super::HOST_STORE_TAIL))
        .args(["-std=c11", "-w", "-o"])
        .arg(&exe)
        .arg(&c)
        .arg("-lm")
        .run(&[])
        .ok()?;
    assert!(
        built.status.success(),
        "the runtime header did not compile: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let out = Spawn::new(Program::os(&exe)).run(&[]).ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn the_store_prefix_is_the_c_runtimes_loft_store_t() {
    let Some(c) = c_offsets() else {
        eprintln!("skipped: no cc on PATH");
        return;
    };
    let c: std::collections::HashMap<&str, usize> = c
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(' ')?;
            Some((k, v.parse().ok()?))
        })
        .collect();
    for (f, rust) in rust_offsets() {
        assert_eq!(c.get(f), Some(&rust), "`{f}`: C offset against Store's");
    }
    assert_eq!(c.get("bits.bits"), Some(&offset_of!(Claims, bits)));
    assert_eq!(c.get("bits.nwords"), Some(&offset_of!(Claims, nwords)));
    assert_eq!(c.get("bits.live"), Some(&offset_of!(Claims, live)));
    assert_eq!(c.get("bits.cap"), Some(&offset_of!(Claims, cap)));
    assert_eq!(c.get("bits.size"), Some(&std::mem::size_of::<Claims>()));
    // The interpreter's own fields begin where the shared prefix ends.
    assert_eq!(
        c.get("end"),
        Some(&offset_of!(Store, stack_buffer)),
        "the shared prefix ends at the same byte"
    );
    // A hosted unit's store is a whole `Store`, so its table indexes the interpreter's.
    assert_eq!(
        c.get("hosted"),
        Some(&std::mem::size_of::<Store>()),
        "a hosted loft_store_t is the size of a Store"
    );
}
