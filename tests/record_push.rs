// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-t (loft#1426) — a record append inside a loop EMITS through a push header
//! (`@FR-R-PushRec`): the fresh element is the header's next slot (`push_record_hoisted`,
//! no `record_new` dispatch and no default prefill — the group's writes fill every field
//! explicitly), and the length bump is the finish (`push_record_finish`), exactly where
//! `record_finish`'s was.  Admitted only where the mint is (`@FR-R-Mint`) and the element
//! is a plain struct owning no heap.
//!
//! The cell corpus (`bytecode-comparisons/V-t-record-push-cells.loft`) can only say the
//! VALUES hold; this pins the EMISSION per cell — which appends fuse, and that the
//! declining cells keep their templates — and the switch (`LOFT_NO_RECORD_PUSH=1`) that
//! restores the mint-group templates, which is what makes it red on the build before the
//! fusion and on one that lost it.  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-t-record-push-cells.loft";

/// `(function, record-push headers a LOOP bound, GROUP headers bound (`@FR-R-GroupPush`),
/// fused uses (slot + finish), template `OpNewRecord(cell,` calls left)` — the predictions
/// written beside the cells before the emitters existed; the group column was added
/// 2026-09-18 with the group header, and each row it moved is re-derived beside it.
// Uses, group headers and template mints count ×2 where the loop is an INNERMOST loop
// with a chain: its body is emitted twice under its chain guard (`@FR-R-GuardedChain`,
// 2026-09-20); a LOOP header is bound once, outside the arms.
const EXPECTED: &[(&str, usize, usize, usize, usize)] = &[
    ("n_m1", 1, 0, 4, 0), // c1: the literal append across growth
    ("n_m2", 1, 0, 2, 0), // c2: the builder append (smooth's shape), delivery guard kept
    ("n_m3", 1, 0, 2, 0), // c3: the partial literal writes every field explicitly
    ("n_m4", 1, 0, 4, 0), // c4: the same pin through a call
    ("n_m5", 0, 0, 0, 1), // c5: a borrow-returning element declines the loop (V-s c12's
    //                        class) and its delivery declines the group too — template
    ("n_m6", 1, 0, 4, 0), // c6: boolean, integer, float fields through the slot
    ("n_m7", 0, 0, 0, 1), // c7: a text field — the formatting write into the element is a
    //                        store write the admission declines, loop and group alike
    ("n_m8", 0, 2, 4, 0), // c8: a nested collection field — the element owns heap: the
    //                        GROUP header takes it, its slot zeroed at the mint (the
    //                        literal `xs` mints a store, which declines the LOOP)
    ("n_m9", 0, 0, 0, 2), // c9: a keyed container is a keyed insert — not admitted
    ("n_m10", 0, 0, 0, 1), // c10: a `__nullable<Pt>` element is not a plain struct
    ("n_m11", 0, 1, 2, 0), // c11: the self-reading push declines the LOOP (V-s), but the
    //                        group's own header serves it: `len(out)` reads the store
    //                        and the bump lands at the finish
    ("n_m12", 2, 0, 8, 0), // c12: two minted paths, one header each
    ("n_m13", 1, 0, 2, 0), // c13: a mint and a scalar push, both fused
    ("n_m14", 0, 0, 0, 1), // c14: a field-path mint is outside the bare-variable admission
    ("n_m15", 0, 2, 4, 0), // c15: the two singleton appends outside any loop — one GROUP
    //                        header each (the reservation gives it its capacity)
    ("n_m16", 1, 0, 4, 0), // c16: mints in opposite arms share the loop's one header
];

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

/// Per emitted function: `(loop record-push headers, group headers, fused record-push uses,
/// template mints)`.
fn counts(rust: &str) -> HashMap<String, (usize, usize, usize, usize)> {
    let mut map: HashMap<String, (usize, usize, usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        let row = map.entry(current.clone()).or_default();
        if line.contains("V-t record push header") {
            row.0 += 1;
        }
        if line.contains("@FR-R-GroupPush group push header") {
            row.1 += 1;
        }
        // `push_record_hoisted_zero` (the heap clause) is a slot use like the plain one.
        row.2 += line.matches("push_record_hoisted").count()
            + line.matches("push_record_finish").count();
        // @PLN157 § V-y — the no-prefill twin IS the template call (minus the default
        // walk), so both spellings count as "template left".
        row.3 +=
            line.matches("OpNewRecord(cell,").count() + line.matches("OpNewRecordNP(cell,").count();
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_fuses_exactly_the_appends_predicted() {
    let out = std::env::temp_dir().join("loft_record_push_on.rs");
    let rust = emit(&cells(), &out, &[]);
    let got = counts(&rust);
    for (name, headers, groups, uses, templates) in EXPECTED {
        let (h, g, u, t) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(h, *headers, "{name}: loop record-push headers bound");
        assert_eq!(g, *groups, "{name}: group push headers bound");
        assert_eq!(u, *uses, "{name}: fused slot + finish uses");
        assert_eq!(t, *templates, "{name}: template OpNewRecord calls left");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_mint_group_templates() {
    let out = std::env::temp_dir().join("loft_record_push_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_RECORD_PUSH", "1")]);
    let got = counts(&rust);
    for (name, ..) in EXPECTED {
        let (h, g, u, _) = got.get(*name).copied().unwrap_or((0, 0, 0, 0));
        assert_eq!(
            h + g + u,
            0,
            "{name}: under LOFT_NO_RECORD_PUSH=1 no record append fuses"
        );
    }
    // The switch narrows the EMISSION only — the mint admission (§ V-s) and its scalar
    // hoists stay; the templates must be back for every fusing cell.
    for (name, headers, groups, ..) in EXPECTED {
        if *headers + *groups > 0 {
            let (.., t) = got.get(*name).copied().unwrap_or((0, 0, 0, 0));
            assert!(
                t > 0,
                "{name}: under LOFT_NO_RECORD_PUSH=1 the OpNewRecord template returns"
            );
        }
    }
    let _ = std::fs::remove_file(&out);
}
