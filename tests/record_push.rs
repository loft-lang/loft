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

/// `(function, record-push headers bound, fused uses (slot + finish), template
/// `OpNewRecord(cell,` calls left)` — the predictions written beside the cells before the
/// emitters existed.
const EXPECTED: &[(&str, usize, usize, usize)] = &[
    ("n_m1", 1, 2, 0),  // c1: the literal append across growth
    ("n_m2", 1, 2, 0),  // c2: the builder append (smooth's shape), delivery guard kept
    ("n_m3", 1, 2, 0),  // c3: the partial literal writes every field explicitly
    ("n_m4", 1, 2, 0),  // c4: the same pin through a call
    ("n_m5", 0, 0, 1),  // c5: a borrow-returning element declines the loop (V-s c12's class)
    ("n_m6", 1, 2, 0),  // c6: boolean, integer, float fields through the slot
    ("n_m7", 0, 0, 1),  // c7: a text field — the element owns heap, template kept
    ("n_m8", 0, 0, 1),  // c8: a nested collection field — same ask, template kept
    ("n_m9", 0, 0, 1),  // c9: a keyed container is a keyed insert — not admitted
    ("n_m10", 0, 0, 1), // c10: a `__nullable<Pt>` element is not a plain struct
    ("n_m11", 0, 0, 1), // c11: the self-reading push materialises a copy — declines
    ("n_m12", 2, 4, 0), // c12: two minted paths, one header each
    ("n_m13", 1, 2, 0), // c13: a mint and a scalar push, both fused
    ("n_m14", 0, 0, 1), // c14: a field-path mint is outside the bare-variable admission
    ("n_m15", 0, 0, 2), // c15: the singleton append outside a loop keeps its templates
    ("n_m16", 1, 4, 0), // c16: mints in opposite arms share the loop's one header
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

/// Per emitted function: `(record-push headers, fused record-push uses, template mints)`.
fn counts(rust: &str) -> HashMap<String, (usize, usize, usize)> {
    let mut map: HashMap<String, (usize, usize, usize)> = HashMap::new();
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
        row.1 += line.matches("push_record_hoisted").count()
            + line.matches("push_record_finish").count();
        row.2 += line.matches("OpNewRecord(cell,").count();
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
    for (name, headers, uses, templates) in EXPECTED {
        let (h, u, t) = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(h, *headers, "{name}: record-push headers bound");
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
        let (h, u, _) = got.get(*name).copied().unwrap_or((0, 0, 0));
        assert_eq!(
            h + u,
            0,
            "{name}: under LOFT_NO_RECORD_PUSH=1 no record append fuses"
        );
    }
    // The switch narrows the EMISSION only — the mint admission (§ V-s) and its scalar
    // hoists stay; the templates must be back for every fusing cell.
    for (name, headers, ..) in EXPECTED {
        if *headers > 0 {
            let (.., t) = got.get(*name).copied().unwrap_or((0, 0, 0));
            assert!(
                t > 0,
                "{name}: under LOFT_NO_RECORD_PUSH=1 the OpNewRecord template returns"
            );
        }
    }
    let _ = std::fs::remove_file(&out);
}
