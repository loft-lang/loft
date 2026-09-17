// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @F48 — the `loft` CLI: engine behind the `loft api-surface` subcommand (@PLN102 C1).

//! @PLN102 C1 commit 4 — the STRICT, tier-aware API diff: two canonical surfaces
//! ([`crate::api_surface::Member`]) → a [`Verdict`]. **What a caller can observe is the
//! rule:** every existing public symbol present, plus additions → `Superset` (a drop-in);
//! anything an existing call site can see change → `Break`, naming it.
//!
//! The test is *observable by a caller*, not *byte-identical*, and the difference is not a
//! grace note — a rendering that changes while every call keeps compiling and keeps meaning
//! what it meant is not a break, and reporting one is its own defect: the only remedy the
//! report offers is to raise `api_compatible_with`, so a false positive here makes a library
//! publish a withdrawal it never made. Two changes are additive for that reason, each
//! measured against a real release that would otherwise have been failed:
//! * an aggregate gaining a **method** — nothing constructed or called stops working
//!   (`server` 0.3.1 → 0.5.0), see [`aggregate_break`];
//! * a function gaining a **trailing optional parameter** — every existing call still
//!   compiles and still binds the same way (`graphics` 0.8.1 → 0.9.0, loft#1191), see
//!   [`signature_break`]. [COMPATIBILITY.md § Per-surface](../doc/claude/COMPATIBILITY.md)
//!   states this one directly: under **Stdlib API**, "a new optional parameter" is additive.
//!
//! Everything else stays strict, and both of those carve narrowly: an added FIELD breaks
//! (a literal construction must supply it), and a parameter that is required, or optional but
//! inserted before an existing one, breaks (it re-binds every positional call).
//!
//! Tier-aware, via **sealed inlining.** A consumer can neither name nor construct a *sealed*
//! type (only read a value's fields through a public signature that returns it), so a sealed
//! type's identity is its SHAPE, not its name. Before comparing, each sealed type's signature
//! is inlined (recursively) into the public signatures that mention it. Then only PUBLIC
//! members are compared, by name:
//! - a **sealed rename** (same shape) leaves every inlined public signature unchanged → no
//!   break — the sealed granularity stays usable without over-flagging;
//! - a **sealed field change** (different shape) flows into every public signature that
//!   returns it → break, named on the public members a consumer actually calls (the break
//!   points C-app later intersects with a consumer's usage).

use crate::api_surface::{Member, Tier};
use std::collections::HashMap;

/// The result of diffing an old surface against a new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A drop-in: every existing public symbol is unchanged; only additions.
    Superset,
    /// A break: the named public symbols were removed or changed.
    Break(Vec<String>),
}

/// Diff `old` against `new`. Additions never break; a removed or changed existing public
/// symbol does (with sealed shapes inlined, so a sealed rename is not a break).
#[must_use]
pub fn diff(old: &[Member], new: &[Member]) -> Verdict {
    let old_pub = inlined_public(old);
    let new_pub = inlined_public(new);
    let new_map: HashMap<(&str, &str), &str> = new_pub
        .iter()
        .map(|(n, k, s)| ((n.as_str(), *k), s.as_str()))
        .collect();

    let mut breaks = Vec::new();
    for (name, kind, sig) in &old_pub {
        match new_map.get(&(name.as_str(), *kind)) {
            None => breaks.push(format!("removed {kind} `{name}`")),
            Some(new_sig) if *new_sig != sig.as_str() => {
                // A textual difference is not automatically a break for an aggregate. A
                // struct's signature is its member list, so GAINING a method rewrites the
                // string while leaving every existing use valid. Measured on real libraries:
                // `server` 0.3.1 -> 0.5.0 added the method `bound` and read as a break, which
                // would have failed a purely additive release had this been a gate.
                let reason = if matches!(*kind, "fn" | "method" | "operator") {
                    signature_break(sig, new_sig)
                } else {
                    aggregate_break(sig, new_sig)
                };
                if let Some(reason) = reason {
                    breaks.push(format!("changed {kind} `{name}` — {reason}"));
                }
            }
            Some(_) => {}
        }
    }
    if breaks.is_empty() {
        Verdict::Superset
    } else {
        Verdict::Break(breaks)
    }
}

/// Why a FUNCTION signature changed in a way a caller can observe, or `None` when the change
/// is one the caller never sees: parameters APPENDED to the end, each of them optional.
///
/// [COMPATIBILITY.md § Per-surface](../doc/claude/COMPATIBILITY.md) states it directly — under
/// **Stdlib API**, *"a new optional parameter"* is additive, and the regression beside it is
/// *"a signature change that breaks existing calls"*. A trailing default breaks none: every
/// call written against the shorter list still compiles and still means what it meant.
///
/// Both halves of that are load-bearing, and each is a real failure the other would let past:
/// * **TRAILING**, because parameters bind by POSITION. A default inserted before an existing
///   parameter re-binds every call site — `f(1, 2)` starts feeding `2` to the new parameter —
///   which is the silent version of the failure and worse than the loud one.
/// * **OPTIONAL**, because an appended REQUIRED parameter stops every existing call compiling.
///
/// What it does not see: a changed default VALUE. The surface records that a parameter is
/// optional, not what it falls back to, so `= false` becoming `= true` reads as no change
/// while every call that omitted it silently gets a different answer. That is a real gap and
/// a pre-existing one — the surface never carried the value — filed separately rather than
/// widened into here, because rendering a value means rendering an arbitrary expression.
fn signature_break(old_sig: &str, new_sig: &str) -> Option<String> {
    let (Some((old_p, old_r)), Some((new_p, new_r))) =
        (split_signature(old_sig), split_signature(new_sig))
    else {
        return Some("shape changed".to_string());
    };
    if old_r != new_r {
        return Some(format!("return type `{old_r}` became `{new_r}`"));
    }
    if new_p.len() < old_p.len() {
        return Some(format!(
            "parameters dropped from {} to {}",
            old_p.len(),
            new_p.len()
        ));
    }
    for (i, old_param) in old_p.iter().enumerate() {
        let (old_cmp, new_cmp) = if i == 0 {
            (receiver_spelling(old_param), receiver_spelling(&new_p[i]))
        } else {
            (old_param.clone(), new_p[i].clone())
        };
        if new_cmp != old_cmp {
            return Some(format!(
                "parameter {} `{old_param}` became `{}`",
                i + 1,
                new_p[i]
            ));
        }
    }
    let required: Vec<&String> = new_p[old_p.len()..]
        .iter()
        .filter(|p| !p.ends_with(" = default"))
        .collect();
    if required.is_empty() {
        None
    } else {
        Some(format!(
            "added required parameter(s) {}",
            required
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// One receiver, written either way: `both` and `self` name the SAME parameter.
///
/// C123 revised: *"`both` still compiles and means `self`"* — the two spellings were measured
/// identical across method and free calls, overloads, **named and default arguments**, nullable
/// and scalar receivers, enum-variant and generic dispatch. So a first parameter renamed between
/// them changes nothing a caller can observe, and reporting it as a break would refuse the very
/// migration C123 prescribes — it names `regex`'s `matches` as the published case to rename.
///
/// Applied at position 0 ONLY, because only the first parameter is the receiver: `both` at any
/// other position is an ordinary name whose rename IS a break. The type is untouched, so
/// `both: text` → `self: integer` still breaks on the half that matters.
///
/// ⚠ This is the ninth site spelling "these two names are one receiver" — `registry_index.rs`'s
/// `receiver != "self" && receiver != "both"` and `triggers.rs`'s `recv_name == "self" ||
/// recv_name == "both"` ask the same question, and `data.rs` carries three `is_self`/`is_both`
/// pairs. There is no canonical home; collapsing them is its own arc (the duplicate-predicate
/// audit in `formal/IMPLEMENTATIONS.md`), not a rider on a compat fix.
fn receiver_spelling(param: &str) -> String {
    match param.strip_prefix("both:") {
        Some(rest) => format!("self:{rest}"),
        None => param.to_string(),
    }
}

/// Split `(a: integer, b: hash<x, y>) -> R` into its parameter list and return type.
///
/// Depth-aware on `<>`, because a generic type argument carries commas of its own and a naive
/// split would tear `hash<x, y>` in half and report two parameters where there is one.
fn split_signature(sig: &str) -> Option<(Vec<String>, String)> {
    let rest = sig.trim().strip_prefix('(')?;
    let close = {
        let (mut depth, mut at) = (0usize, None);
        for (i, c) in rest.char_indices() {
            match c {
                '(' | '<' => depth += 1,
                '>' => depth = depth.saturating_sub(1),
                ')' if depth == 0 => {
                    at = Some(i);
                    break;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }
        at?
    };
    let inner = &rest[..close];
    let returns = rest[close + 1..]
        .trim()
        .strip_prefix("->")?
        .trim()
        .to_string();
    let mut params = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '<' | '(' => depth += 1,
            '>' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                params.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = inner[start..].trim();
    if !tail.is_empty() {
        params.push(tail.to_string());
    }
    Some((params, returns))
}

/// Why an aggregate's member list changed in a way a consumer can observe, or `None` when the
/// change is purely additive.
///
/// The distinction that matters is what a consumer may do with the type:
/// * a **removed** member, or one whose type changed, breaks every existing use;
/// * an added **method** (`name: fn`) is additive — nothing that compiled stops compiling;
/// * an added **field** is NOT, because it changes what a whole-record read answers:
///   `{s:j}` gains a key, and so does any reflection walk.
///
/// ⚠ That last one is NOT "a literal must now supply it", which this comment used to say and
/// which is false for **every** field in loft, not merely a defaulted one: a struct literal
/// may omit anything. Measured — `S { }` compiles, and each omitted field takes its declared
/// default or its type's zero. The reason matters because it is what a reader checks the
/// verdict against, and checking against the false one leads to the conclusion that a
/// defaulted field is additive (loft#1192, closed on this measurement).
///
/// The rules do not settle it cleanly, which is why the verdict stays where it is rather
/// than moving on one reading. [COMPATIBILITY.md § Per-surface](../doc/claude/COMPATIBILITY.md)
/// has two rows that point opposite ways for this exact change: under **Stdlib API** a
/// *"different result for the same inputs"* is a regression, and the serialised form is
/// different; under **On-disk + wire**, *"new optional fields with defaults"* is additive.
/// Moving the verdict is a design decision about which surface a loft struct is, not a fix.
/// Contrast the PARAMETER case, where one rule said "a new optional parameter" is additive
/// with nothing against it — that one was unambiguous, and [`signature_break`] follows it.
///
/// Anything whose shape this cannot parse falls back to "changed", so an unrecognised
/// rendering is reported rather than waved through — the conservative direction for a check
/// whose whole value is that a silent break is impossible.
fn aggregate_break(old_sig: &str, new_sig: &str) -> Option<String> {
    let (Some(old_m), Some(new_m)) = (members_of(old_sig), members_of(new_sig)) else {
        return Some("shape changed".to_string());
    };
    let mut reasons = Vec::new();
    for (name, ty) in &old_m {
        match new_m.iter().find(|(n, _)| n == name) {
            None => reasons.push(format!("removed `{name}`")),
            Some((_, new_ty)) if new_ty != ty => {
                reasons.push(format!("`{name}` changed type"));
            }
            Some(_) => {}
        }
    }
    for (name, ty) in &new_m {
        if ty != "fn" && !old_m.iter().any(|(n, _)| n == name) {
            reasons.push(format!(
                "added field `{name}` (a literal construction must supply it)"
            ));
        }
    }
    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join(", "))
    }
}

/// Parse `{ a: fn, b: integer }` into `[(a, fn), (b, integer)]`. `None` when the signature is
/// not a brace-delimited member list (a plain function signature, say).
fn members_of(sig: &str) -> Option<Vec<(String, String)>> {
    let inner = sig.trim().strip_prefix('{')?.strip_suffix('}')?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    inner
        .split(',')
        .map(|part| {
            let (n, t) = part.split_once(':')?;
            Some((n.trim().to_string(), t.trim().to_string()))
        })
        .collect()
}

/// The PUBLIC members of a surface, each with sealed types inlined into its signature (so a
/// sealed rename is invisible and a sealed field change flows through).
fn inlined_public(surface: &[Member]) -> Vec<(String, &'static str, String)> {
    let sealed: HashMap<&str, &str> = surface
        .iter()
        .filter(|m| m.tier == Tier::Sealed)
        .map(|m| (m.name.as_str(), m.signature.as_str()))
        .collect();
    surface
        .iter()
        .filter(|m| m.tier == Tier::Public)
        .map(|m| (m.name.clone(), m.kind, inline(&m.signature, &sealed, 0)))
        .collect()
}

/// Replace each identifier in `sig` that names a sealed type with that type's (recursively
/// inlined) shape. `depth` bounds recursive/self-referential sealed types.
fn inline(sig: &str, sealed: &HashMap<&str, &str>, depth: u8) -> String {
    if depth > 16 {
        return sig.to_string();
    }
    let mut out = String::new();
    let mut ident = String::new();
    for c in sig.chars() {
        if c.is_alphanumeric() || c == '_' {
            ident.push(c);
        } else {
            push_ident(&ident, sealed, depth, &mut out);
            ident.clear();
            out.push(c);
        }
    }
    push_ident(&ident, sealed, depth, &mut out);
    out
}

fn push_ident(ident: &str, sealed: &HashMap<&str, &str>, depth: u8, out: &mut String) {
    if ident.is_empty() {
        return;
    }
    if let Some(shape) = sealed.get(ident) {
        out.push_str(&inline(shape, sealed, depth + 1));
    } else {
        out.push_str(ident);
    }
}

#[cfg(test)]
mod tests {
    use super::{Verdict, diff};
    use crate::api_surface::{Member, Tier};

    fn pubm(name: &str, kind: &'static str, sig: &str) -> Member {
        Member {
            name: name.to_string(),
            kind,
            tier: Tier::Public,
            signature: sig.to_string(),
        }
    }
    fn sealed(name: &str, kind: &'static str, sig: &str) -> Member {
        Member {
            name: name.to_string(),
            kind,
            tier: Tier::Sealed,
            signature: sig.to_string(),
        }
    }
    fn is_break(v: &Verdict) -> bool {
        matches!(v, Verdict::Break(_))
    }

    #[test]
    fn pure_additions_are_a_superset() {
        let old = vec![pubm("make", "fn", "() -> integer")];
        let new = vec![
            pubm("make", "fn", "() -> integer"),
            pubm("extra", "fn", "(a: integer) -> integer"),
        ];
        assert_eq!(diff(&old, &new), Verdict::Superset);
    }

    #[test]
    fn public_rename_is_a_break() {
        let old = vec![pubm("make", "fn", "() -> integer")];
        let new = vec![pubm("produce", "fn", "() -> integer")];
        assert!(is_break(&diff(&old, &new)), "renaming a public fn breaks");
    }

    #[test]
    fn a_receiver_renamed_both_to_self_is_not_a_break() {
        // C123: `both` still compiles and MEANS `self`, so this rename — the migration C123
        // prescribes, naming `regex`'s `matches` — changes nothing a caller can observe.
        let old = vec![pubm(
            "matches",
            "fn",
            "(both: text, pattern: text) -> boolean",
        )];
        let new = vec![pubm(
            "matches",
            "fn",
            "(self: text, pattern: text) -> boolean",
        )];
        assert_eq!(diff(&old, &new), Verdict::Superset);
    }

    #[test]
    fn an_ordinary_first_parameter_rename_still_breaks() {
        // The control for the rule above: it must equate the two RECEIVER spellings, not stop
        // comparing the first parameter at all.
        let old = vec![pubm("f", "fn", "(a: text) -> boolean")];
        let new = vec![pubm("f", "fn", "(b: text) -> boolean")];
        assert!(
            is_break(&diff(&old, &new)),
            "a plain first-param rename breaks"
        );
    }

    #[test]
    fn both_to_self_away_from_the_receiver_still_breaks() {
        // Only position 0 is the receiver; `both` elsewhere is an ordinary parameter name.
        let old = vec![pubm("f", "fn", "(a: text, both: text) -> boolean")];
        let new = vec![pubm("f", "fn", "(a: text, self: text) -> boolean")];
        assert!(
            is_break(&diff(&old, &new)),
            "non-receiver rename still breaks"
        );
    }

    #[test]
    fn a_receiver_whose_type_changed_still_breaks() {
        // The spelling is equated; the TYPE is not.
        let old = vec![pubm("f", "fn", "(both: text) -> boolean")];
        let new = vec![pubm("f", "fn", "(self: integer) -> boolean")];
        assert!(
            is_break(&diff(&old, &new)),
            "a retyped receiver still breaks"
        );
    }

    #[test]
    fn a_safe_widening_still_breaks() {
        // strict: ANY change to an existing public symbol is a break, no "compatible" grace.
        let old = vec![pubm("f", "fn", "(a: integer) -> integer")];
        let new = vec![pubm("f", "fn", "(a: number) -> integer")];
        assert!(is_break(&diff(&old, &new)), "a widened param still breaks");
    }

    #[test]
    fn sealed_rename_is_not_a_break() {
        // `make` returns a sealed type; rename the sealed type (same shape). The inlined
        // public signature is unchanged, so it is a drop-in.
        let old = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: integer }"),
        ];
        let new = vec![
            pubm("make", "fn", "() -> Gadget"),
            sealed("Gadget", "struct", "{ x: integer }"),
        ];
        assert_eq!(
            diff(&old, &new),
            Verdict::Superset,
            "a sealed rename with an unchanged shape is a drop-in"
        );
    }

    #[test]
    fn sealed_field_change_is_a_break() {
        let old = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: integer }"),
        ];
        let new = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: text }"),
        ];
        assert!(
            is_break(&diff(&old, &new)),
            "a sealed field change flows into the public sig and breaks"
        );
    }

    #[test]
    fn sealed_field_change_names_the_public_break_point() {
        // The break must name the PUBLIC member a consumer calls (`make`), not the sealed type.
        let old = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: integer }"),
        ];
        let new = vec![
            pubm("make", "fn", "() -> Widget"),
            sealed("Widget", "struct", "{ x: integer, y: text }"),
        ];
        match diff(&old, &new) {
            Verdict::Break(symbols) => {
                assert!(
                    symbols.iter().any(|s| s.contains("make")),
                    "names make: {symbols:?}"
                );
            }
            Verdict::Superset => panic!("expected a break"),
        }
    }

    #[test]
    fn nested_sealed_field_change_breaks_transitively() {
        // `f` returns sealed A; A has a field of sealed B; changing B's field must break `f`.
        let old = vec![
            pubm("f", "fn", "() -> A"),
            sealed("A", "struct", "{ b: B }"),
            sealed("B", "struct", "{ n: integer }"),
        ];
        let new = vec![
            pubm("f", "fn", "() -> A"),
            sealed("A", "struct", "{ b: B }"),
            sealed("B", "struct", "{ n: text }"),
        ];
        assert!(
            is_break(&diff(&old, &new)),
            "a nested sealed field change breaks transitively"
        );
    }
}
