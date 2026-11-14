// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN91 — compiled standard-library bodies for the loft bytecode runner (`loft --emit-c-bodies`).
//!
//! The Rust interpreter runs a standard-library function the Rust build compiled through its
//! `loft_shared_<fn>` bridge, on the program's own `Stores` (`compiled_stdlib`), for the set
//! `stdlib_codegen::export_set` picks.  The loft
//! runner does the same through this fragment: the C of each such function, written by the C
//! writer, and one ENTRY per function that reads its arguments from the runner's stack as the
//! bytecode's call leaves them and writes its answer where the return puts it.
//!
//! The fragment is compiled INTO the runner's C runtime shim (`selfhost/crt/shim/crt.c`
//! includes `loft_bodies.c`): the program's stores and types are the shim's, and every runtime
//! global is private to its C unit, so a body compiled anywhere else would see other stores.
//!
//! The contract the runner calls (`selfhost/crt/src/crt.loft`):
//!
//! - `int64_t loft_body_find(const char *name, int64_t len)` — the entry for definition `name`
//!   (with or without its `loft_shared_` prefix), or -1.  A name lookup: the runner calls it
//!   once per shared native when it loads the image and keeps the entry, never per call;
//! - `int64_t loft_body_call(int64_t entry, int64_t stack, int64_t sp)` — run it on the stack
//!   store `stack` whose top is `sp`: the arguments lie just below `sp`, each in its slot (a
//!   scalar 8 bytes, a text view 16); the answer is written at their base and the new top
//!   returned.  A boolean answer is ONE byte of its 8-byte slot (0, 1, 255 for null), read as
//!   a byte, as the bytecode's boolean is.  Negative: not carried, or the stack does not hold
//!   the arguments — the caller runs the loft body.
//!
//! Every body carried today only READS the program's data.  A body that writes a store would
//! be written against this unit's facts (no locks, no threads), which the shim does not hold
//! for the program it runs; it is not carried until the shim states those facts itself.
//!
//! A function this fragment cannot carry has no entry, so the runner runs its loft body: slower,
//! never wrong.  The fragment says which and why in its header.
use super::{RUNTIME_END, c_name, emit_unit_for, tidy};
use crate::data::{Data, Type};
use crate::database::Stores;
use std::fmt::Write as _;

/// One argument or the answer as the runner's stack holds it.
#[derive(Clone, Copy, PartialEq)]
enum Slot {
    /// An integer, 8 bytes; null is `INT64_MIN`.
    Int,
    /// A float, 8 bytes.
    Float,
    /// A boolean, one byte of an 8-byte slot: 0, 1, or 255 for null.
    Bool,
    /// A text, a 16-byte view (`loft_str`).
    Text,
}

impl Slot {
    fn bytes(self) -> i64 {
        match self {
            Slot::Text => 16,
            _ => 8,
        }
    }
    fn c(self) -> &'static str {
        match self {
            Slot::Int => "int64_t",
            Slot::Float => "double",
            Slot::Bool => "uint8_t",
            Slot::Text => "loft_str",
        }
    }
}

/// `d`'s arguments and answer as slots, or why the fragment does not carry it: a reference, a
/// collection, a `&` parameter, a hidden buffer, a text answer — the later slices.
fn signature(data: &Data, d: u32) -> Result<(Vec<Slot>, Slot), String> {
    let def = data.def(d);
    if def.hidden_return_buffer_attr().is_some() {
        return Err("a hidden return buffer".into());
    }
    let slot = |tp: &Type, what: &str| -> Result<Slot, String> {
        match tp.base() {
            Type::Integer(..) | Type::Character => Ok(Slot::Int),
            Type::Float => Ok(Slot::Float),
            Type::Boolean => Ok(Slot::Bool),
            Type::Text(_) if what == "argument" => Ok(Slot::Text),
            other => Err(format!("{what} of type {}", super::type_word(other))),
        }
    };
    let mut args = Vec::new();
    for a in def.attributes() {
        if matches!(a.typedef.base(), Type::RefVar(_)) {
            return Err("a & parameter".into());
        }
        args.push(slot(&a.typedef, "argument")?);
    }
    let ret = slot(def.returned(), "answer")?;
    Ok((args, ret))
}

/// The fragment: the bodies of the standard-library functions the Rust build compiles
/// (`stdlib_codegen::export_set`) that it carries, their entries, and the lookup.
///
/// # Errors
/// The C writer's refusal of a body it was asked for.
pub fn emit_bodies(data: &Data, stores: &Stores) -> Result<String, super::Refusal> {
    // The Rust build's own set (`stdlib_codegen::export_set`): the functions both compile.
    let mut set: Vec<u32> = crate::stdlib_codegen::export_set(data)
        .into_iter()
        .collect();
    set.sort_unstable_by_key(|d| data.def(*d).name().to_string());
    let mut carried: Vec<(u32, Vec<Slot>, Slot)> = Vec::new();
    let mut declined: Vec<(String, String)> = Vec::new();
    for d in set {
        match signature(data, d) {
            Ok((args, ret)) => carried.push((d, args, ret)),
            Err(why) => declined.push((data.def(d).name().to_string(), why)),
        }
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "/* Generated by `loft --emit-c-bodies` (src/c/bodies.rs) — do not edit: `make crt-bodies`\n \
         * writes it, and tests/crt_bodies.rs fails when it differs from what the C writer writes.\n \
         * Compiled standard-library bodies for the loft bytecode runner, included by crt.c.\n \
         * Carried: {}.\n * Not carried (the runner runs the loft body):",
        carried
            .iter()
            .map(|(d, _, _)| data.def(*d).name())
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (name, why) in &declined {
        let _ = writeln!(out, " *   {name}: {why}");
    }
    out.push_str(" */\n");
    if carried.is_empty() {
        out.push_str(&lookup(&[]));
        return Ok(out);
    }
    let roots: Vec<u32> = carried.iter().map(|(d, _, _)| *d).collect();
    let unit = emit_unit_for(data, stores, &roots, false)?;
    let at = unit.find(RUNTIME_END).map_or(0, |a| a + RUNTIME_END.len());
    let mut written = unit[at..].to_string();
    // The unit's table binding is a program's start-up; the shim binds its own.
    if let Some(s) = written.find("\nstatic void loft_bind_tables(void) {")
        && let Some(e) = written[s..].find("\n}\n")
    {
        written.replace_range(s..s + e + 3, "\n");
    }
    let mut entries = String::new();
    let mut names = Vec::new();
    for (k, (d, args, ret)) in carried.iter().enumerate() {
        entries.push_str(&entry(k, &c_name(data, *d), args, *ret));
        names.push(data.def(*d).name().to_string());
    }
    entries.push_str(&lookup(&names));
    // What nothing names — a callee the carried bodies do not reach — is not written.
    let whole = format!("{RUNTIME_END}{written}{entries}");
    let kept = tidy::drop_unnamed(&whole, RUNTIME_END);
    // A frame site names its source file as the standard library was found on this machine;
    // the committed fragment names it as `default/<file>`, as `compiled_stdlib_gen.rs` does, so
    // it is the same text on every checkout.
    let mut fragment = kept[RUNTIME_END.len()..].to_string();
    for (d, _, _) in &carried {
        let file = data.def(*d).position().file;
        if let Some(at) = file.rfind("/default/") {
            fragment = fragment.replace(&format!("\"{}", &file[..=at]), "\"");
        }
    }
    out.push_str(&fragment);
    Ok(out)
}

/// Entry `k`: the arguments read from their slots below `sp`, the body called, its answer
/// written at the arguments' base, the new top answered.
fn entry(k: usize, body: &str, args: &[Slot], ret: Slot) -> String {
    let size: i64 = args.iter().map(|s| s.bytes()).sum();
    let mut e = format!(
        "\nstatic int64_t loft_body_e{k}(int64_t stk, int64_t sp) {{\n    \
         int64_t base = sp - {size};\n    \
         if (base < 0) return -1;\n"
    );
    let mut off = 0;
    let mut call = Vec::new();
    for (i, s) in args.iter().enumerate() {
        let _ = writeln!(
            e,
            "    {} a{i}; memcpy(&a{i}, loft_crt_slot_at(stk, base + {off}), sizeof a{i});",
            s.c()
        );
        call.push(format!("a{i}"));
        off += s.bytes();
    }
    let _ = writeln!(e, "    {} r = {body}({});", ret.c(), call.join(", "));
    let _ = writeln!(
        e,
        "    memcpy(loft_crt_slot_at(stk, base), &r, sizeof r);\n    return base + {};\n}}",
        ret.bytes()
    );
    e
}

/// The name table and the two calls the runner binds.
fn lookup(names: &[String]) -> String {
    let mut t = String::from(
        "\n/* A text argument's slot is 16 bytes, as the bytecode lays it out; a view must fit it. */\n\
         typedef char loft_body_text_fits[sizeof(loft_str) <= 16 ? 1 : -1];\n\
         typedef int64_t (*loft_body_fn)(int64_t, int64_t);\nstatic const struct { const char *name; loft_body_fn fn; } loft_body_table[] = {\n",
    );
    for (k, n) in names.iter().enumerate() {
        let _ = writeln!(t, "    {{ \"{n}\", loft_body_e{k} }},");
    }
    if names.is_empty() {
        t.push_str("    { 0, 0 },\n");
    }
    let _ = write!(
        t,
        "}};\nstatic const int64_t loft_body_count = {};\n\n\
         int64_t loft_body_find(const char *name, int64_t len) {{\n    \
         static const char shared[] = \"loft_shared_\";\n    \
         if (len >= (int64_t) sizeof shared - 1 && memcmp(name, shared, sizeof shared - 1) == 0) {{\n        \
         name += sizeof shared - 1;\n        len -= (int64_t) sizeof shared - 1;\n    }}\n    \
         for (int64_t k = 0; k < loft_body_count; k++) {{\n        \
         const char *n = loft_body_table[k].name;\n        \
         if (n && (int64_t) strlen(n) == len && memcmp(n, name, (size_t) len) == 0) return k;\n    }}\n    \
         return -1;\n}}\n\n\
         int64_t loft_body_call(int64_t entry, int64_t stk, int64_t sp) {{\n    \
         if (entry < 0 || entry >= loft_body_count || !loft_body_table[entry].fn) return -1;\n    \
         return loft_body_table[entry].fn(stk, sp);\n}}\n",
        names.len()
    );
    t
}
