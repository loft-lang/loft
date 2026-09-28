// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A standard library that does not match the binary is refused at load.
//!
//! The interpreter dispatches an operator by POSITION: `Data::op_code` hands every operator
//! declaration in `default/*.loft` the next ordinal in parse order, and
//! [`fill::OPERATORS`](crate::fill::OPERATORS) — generated from the same files — carries the
//! body for each ordinal, with no name beside it.  So a binary from one tree run against
//! another tree's `default/` does not fail: one declaration more or fewer moves every later
//! body under the wrong opcode, and the run ends in a corrupt reference somewhere past the
//! first shifted op (`Store access out of bounds … the reference is corrupt` inside a body
//! the program never called).  A bytecode cache laid out by a different binary is the
//! sibling of this class and is keyed on the build id (`build.rs`); a stdlib read from disk
//! has no such key, so it is checked by content, against the names the generator wrote
//! beside the table.
//!
//! The check is one walk over the parsed operator definitions, at the end of the top-level
//! `Parser::parse_dir` of the stdlib.  Three shapes are refused, each with its cure: a slot
//! whose name differs, a declaration past the table (an operator `default/` declares and
//! `make fill` has not yet given a body), and a table entry the library does not declare.
//! A pure-loft edit to `default/` — anything that is not an operator declaration — passes,
//! so the edit-and-rerun loop needs no rebuild.

use crate::data::Data;

/// The loft names of the parsed operator definitions, in opcode order.
#[must_use]
pub fn parsed_operator_names(data: &Data) -> Vec<String> {
    let mut ops: Vec<(u16, String)> = (0..data.definitions())
        .map(|d_nr| data.def(d_nr))
        .filter(|def| def.is_operator() && def.op_code() != u16::MAX)
        .map(|def| (def.op_code(), def.name().to_string()))
        .collect();
    ops.sort_by_key(|(code, _)| *code);
    ops.into_iter().map(|(_, name)| name).collect()
}

/// Refuse a parsed stdlib whose operator declarations are not, slot for slot, the ones this
/// binary's dispatch table was generated from.  `Err` carries the whole user-facing message.
pub fn verify(data: &Data, dir: &str) -> Result<(), String> {
    verify_names(
        &parsed_operator_names(data),
        crate::fill::OPERATOR_NAMES,
        dir,
    )
}

/// The comparison behind [`verify`], on plain names so it can be tested without a parse.
pub fn verify_names(parsed: &[String], expected: &[&str], dir: &str) -> Result<(), String> {
    let build = env!("LOFT_BUILD_ID");
    let intro =
        format!("the standard library at `{dir}` does not match this loft binary (build {build})");
    for (slot, (have, want)) in parsed.iter().zip(expected.iter()).enumerate() {
        if have != want {
            return Err(format!(
                "{intro}: operator slot {slot} declares `{have}` where the binary carries \
                 `{want}`.\n  The binary and `default/` come from different trees.  Rebuild \
                 loft from the tree that owns this `default/` (`cargo build --bin loft`; \
                 `make fill` first when an operator was added or removed), or pass `--path` \
                 pointing at the tree this binary was built from."
            ));
        }
    }
    if parsed.len() > expected.len() {
        let extra = &parsed[expected.len()];
        return Err(format!(
            "{intro}: operator slot {} declares `{extra}`, and this binary has no body for \
             it.\n  An operator was added to `default/` after this binary was built: run \
             `make fill` and rebuild (`cargo build --bin loft`).",
            expected.len()
        ));
    }
    if parsed.len() < expected.len() {
        let missing = expected[parsed.len()];
        return Err(format!(
            "{intro}: the binary carries `{missing}` at operator slot {}, and the library \
             declares nothing there.\n  This `default/` is older than the binary: rebuild \
             loft from the tree that owns it (`cargo build --bin loft`), or pass `--path` \
             pointing at the tree this binary was built from.",
            parsed.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::verify_names;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_matching_library_passes() {
        assert!(verify_names(&names(&["OpA", "OpB"]), &["OpA", "OpB"], "d").is_ok());
        assert!(verify_names(&[], &[], "d").is_ok());
    }

    #[test]
    fn a_shifted_slot_names_both_operators_and_its_slot() {
        let e =
            verify_names(&names(&["OpA", "OpNew", "OpB"]), &["OpA", "OpB"], "lib/x").unwrap_err();
        assert!(e.contains("`lib/x`"), "{e}");
        assert!(
            e.contains("slot 1 declares `OpNew` where the binary carries `OpB`"),
            "{e}"
        );
    }

    #[test]
    fn a_declaration_past_the_table_names_make_fill() {
        let e = verify_names(&names(&["OpA", "OpB", "OpC"]), &["OpA", "OpB"], "d").unwrap_err();
        assert!(
            e.contains("slot 2 declares `OpC`, and this binary has no body"),
            "{e}"
        );
        assert!(e.contains("make fill"), "{e}");
    }

    #[test]
    fn a_library_short_of_the_table_names_the_missing_operator() {
        let e = verify_names(&names(&["OpA"]), &["OpA", "OpB"], "d").unwrap_err();
        assert!(e.contains("carries `OpB` at operator slot 1"), "{e}");
    }
}
