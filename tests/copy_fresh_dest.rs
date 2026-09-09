// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-j (loft#1426) — the copy into a FRESH element carries `COPY_FRESH_DEST`.
//!
//! `v += [f]` lowers to `OpNewRecord` (the slot defaulted) and `OpCopyRecord`, and the copy
//! opened with `remove_claims` on a destination that holds nothing — a walk allocating a
//! child list per record and per owned field to find zero handles, 7 % of `fronds`.  The
//! parser now marks that destination fresh in the copy's `tp` operand (`keys::COPY_FRESH_DEST`)
//! and both runtimes skip the clear.  Values cannot tell the two apart, so this pins the
//! emission: the append's copy carries the bit, and a copy into an EXISTING element — a
//! whole-element write, whose old value is exactly what the clear releases — does not.
//! Read off `loft introspect`, the instrument the design was written on.
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "\
struct In { a: integer }
struct R { id: integer, ins: vector<In> }
fn mk(n: integer) -> vector<R> {
  out: vector<R> = [];
  for i in 0..n { out += [R { id: i, ins: [In { a: i }] }]; }
  out
}
fn c_append() -> integer { out: vector<R> = []; for f in mk(3) { out += [f]; } len(out) }
fn c_write() -> integer { out = mk(3); src = mk(2); out[0] = src[1]?; len(out) }
fn main() { println(\"{c_append()} {c_write()}\"); }
";

const FRESH: i64 = 0x4000;

fn introspect(src: &std::path::Path) -> String {
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("introspect")
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The IR of one function, as `introspect` prints it (up to its byte-code section).
fn ir_of<'a>(dump: &'a str, name: &str) -> &'a str {
    let start = dump
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no IR for {name}"));
    let rest = &dump[start..];
    let end = rest.find("\nbyte-code").unwrap_or(rest.len());
    &rest[..end]
}

/// Every `OpCopyRecord` type operand in one function's IR.  The operands nest
/// (`f(3)`, a `{#ncc(2):…}` block), so the call's end and its last operand are found by
/// paren depth, not by the first `)`.
fn copy_tps(ir: &str) -> Vec<i64> {
    ir.match_indices("OpCopyRecord(")
        .map(|(i, key)| {
            let args = &ir[i + key.len()..];
            let (mut depth, mut last_comma, mut end) = (0usize, 0usize, None);
            for (j, c) in args.char_indices() {
                match c {
                    '(' | '{' | '[' => depth += 1,
                    ')' | '}' | ']' if depth == 0 => {
                        end = Some(j);
                        break;
                    }
                    ')' | '}' | ']' => depth -= 1,
                    ',' if depth == 0 => last_comma = j + 1,
                    _ => {}
                }
            }
            let end = end.expect("a closed OpCopyRecord");
            let last = args[last_comma..end].trim();
            last.trim_end_matches("i32")
                .parse::<i64>()
                .unwrap_or_else(|_| panic!("a literal type operand, got `{last}`"))
        })
        .collect()
}

#[test]
fn an_append_copies_into_a_fresh_element_and_says_so() {
    let src = std::env::temp_dir().join("loft_copy_fresh_dest_append.loft");
    std::fs::write(&src, PROBE).expect("write probe");
    let dump = introspect(&src);
    let ir = ir_of(&dump, "n_c_append");
    let tps = copy_tps(ir);
    assert!(!tps.is_empty(), "the append must copy the element:\n{ir}");
    assert!(
        tps.iter().all(|t| t & FRESH != 0),
        "every copy into the appended element carries COPY_FRESH_DEST, got {tps:?}:\n{ir}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn a_whole_element_write_keeps_the_clear() {
    let src = std::env::temp_dir().join("loft_copy_fresh_dest_write.loft");
    std::fs::write(&src, PROBE).expect("write probe");
    let dump = introspect(&src);
    let ir = ir_of(&dump, "n_c_write");
    let tps = copy_tps(ir);
    assert!(
        !tps.is_empty(),
        "the element write must copy into the existing element:\n{ir}"
    );
    assert!(
        tps.iter().all(|t| t & FRESH == 0),
        "a copy into an EXISTING element must not carry COPY_FRESH_DEST, got {tps:?}:\n{ir}"
    );
    let _ = std::fs::remove_file(&src);
}
