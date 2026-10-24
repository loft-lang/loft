// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Unicode in C: a character class (`c.is_alphabetic()`, …) and a text's case mapping
//! (`t.to_lowercase()`) answer what Rust's `char` answers, because the emitter computes their
//! tables from Rust's own functions when it writes the unit — only the tables a program uses.
//! A class is a sorted list of code-point ranges; a case mapping lists every character whose
//! mapping is not itself, with the UTF-8 it maps to.
use super::Cx;
use std::fmt::Write as _;

/// The character classes the stdlib asks about, by the C suffix of their test.
pub(crate) const CLASSES: &[(&str, fn(char) -> bool)] = &[
    ("lowercase", char::is_lowercase),
    ("uppercase", char::is_uppercase),
    ("numeric", char::is_numeric),
    ("alphanumeric", char::is_alphanumeric),
    ("alphabetic", char::is_alphabetic),
    ("whitespace", char::is_whitespace),
    ("control", char::is_control),
];

impl Cx<'_> {
    /// The C test for class `name` (`loft_uc_{name}`), its table registered.
    pub(crate) fn unicode_class(&self, name: &str) -> Option<String> {
        CLASSES.iter().find(|(n, _)| *n == name)?;
        self.unicode.borrow_mut().insert(name.to_string());
        Some(format!("loft_uc_{name}"))
    }

    /// The C case mapping of a text (`loft_text_lower` / `loft_text_upper`), its table
    /// registered.
    pub(crate) fn unicode_case(&self, upper: bool) -> String {
        let name = if upper { "upper" } else { "lower" };
        self.unicode.borrow_mut().insert(name.to_string());
        format!("loft_text_{name}")
    }

    /// Every registered table and the function that reads it.
    pub(crate) fn unicode_code(&self) -> String {
        let mut out = String::new();
        for name in self.unicode.borrow().iter() {
            if let Some((_, test)) = CLASSES.iter().find(|(n, _)| n == name) {
                Self::class_code(&mut out, name, *test);
            } else {
                Self::case_code(&mut out, name == "upper");
            }
        }
        out
    }

    fn class_code(out: &mut String, name: &str, test: fn(char) -> bool) {
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for c in (0..=0x10_FFFFu32).filter_map(char::from_u32) {
            if !test(c) {
                continue;
            }
            let u = c as u32;
            match ranges.last_mut() {
                Some((_, hi)) if *hi + 1 == u => *hi = u,
                _ => ranges.push((u, u)),
            }
        }
        let list: Vec<String> = ranges
            .iter()
            .map(|(a, b)| format!("{{ {a}u, {b}u }}"))
            .collect();
        let _ = writeln!(
            out,
            "static const uint32_t loft_ur_{name}[][2] = {{ {} }};\nstatic uint8_t loft_uc_{name}(uint32_t c) {{ return loft_in_ranges(c, loft_ur_{name}, {}); }}",
            list.join(", "),
            ranges.len()
        );
    }

    fn case_code(out: &mut String, upper: bool) {
        let name = if upper { "upper" } else { "lower" };
        let mut rows = Vec::new();
        for c in (0..=0x10_FFFFu32).filter_map(char::from_u32) {
            let mapped: String = if upper {
                c.to_uppercase().collect()
            } else {
                c.to_lowercase().collect()
            };
            if mapped.chars().eq(std::iter::once(c)) {
                continue;
            }
            rows.push(format!(
                "{{ {}u, {} }}",
                c as u32,
                super::body::c_string(&mapped)
            ));
        }
        let _ = writeln!(
            out,
            "static const loft_umap loft_um_{name}[] = {{ {} }};\nstatic loft_text loft_text_{name}(loft_str s) {{ return loft_text_case(s, loft_um_{name}, {}, {}); }}",
            rows.join(", "),
            rows.len(),
            i32::from(!upper)
        );
    }
}
