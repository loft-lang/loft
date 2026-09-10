// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I59 — Type resolver

//! Type resolution and field layout.
//!
//! After the parser's first pass declares all types, this module resolves
//! forward references, computes field sizes and offsets, and initialises
//! database store schemas.  Called between parser pass 1 and pass 2.
//!
//! Key entry points:
//! - [`actual_types`] — resolve forward type references, detect cycles, and settle
//!   each field's TYPE.  It does not assign byte positions: those come later, when
//!   `Stores::finish` runs `finish_type` over the resolved types (@FR-L-Total).
//! - [`fill_all`] — allocate database stores for each struct/enum and
//!   write the type schema into `Stores`.
//! - [`complete_definition`] — finalise a single definition's field layout.

use crate::data::{Data, DefType, Deps, I32, IntegerSpec, Type, Value};
use crate::database::{Stores, TYPEVAR_ROW_PREFIX};
use crate::diagnostics::Level;
use crate::keys::{Content, Str};
use crate::lexer::Lexer;

/// loft#876 — fold a field's declared default (`height: float = 1.5`) to the constant
/// the store schema can carry, or `None` when it is not one.
///
/// A default is an arbitrary expression, and the store layer that answers "what is this
/// field when nobody said" sits below the parser with no evaluator. Rather than give it
/// one, only a default that is already a LITERAL is carried, which is the majority
/// spelling and needs nothing to run.
///
/// The rest keep exactly their previous reach: a default needing a temporary (`= mk()`,
/// `= [1, 2]`, a nested struct literal) is lowered parser-side into a function the
/// CONSTRUCTION site calls, so it still applies to `D {}` and still does not apply to a
/// `text as D` cast. That is the contract — a constant default is part of the type, a
/// computed one is part of construction — and it is stated in LOFT.md rather than left
/// as a hole.
///
/// `Value::Null` is "no default declared". An explicit `= null` is the same answer the
/// absent-value path already writes, so folding it would change nothing.
///
/// `#[inline(never)]` is load-bearing, not a hint. A text default interns its spelling
/// with the same intentional `Box::leak` as `ir_read` / `ir_schema` / `snapshot`, so it
/// needs the same LSan suppression — and a suppression matches by FRAME NAME. Inlined,
/// the allocating frame reads as `typedef::fill_database`, and suppressing that would
/// blind the leak gate to every allocation in the whole type-registration path. Keeping
/// this a real frame lets `.github/lsan_suppressions.txt` name exactly the one deliberate
/// leak and nothing else.
#[inline(never)]
pub(crate) fn fold_declared_default(value: &Value) -> Option<Content> {
    match value.unspan() {
        Value::Int(i) => Some(Content::Long(i64::from(*i))),
        Value::Long(i) => Some(Content::Long(*i)),
        // A boolean field is a byte holding 0/1; `Content` has no boolean spelling and
        // the walker reads this back through the field's own content type.
        Value::Boolean(b) => Some(Content::Long(i64::from(*b))),
        Value::Float(f) => Some(Content::Float(*f)),
        Value::Single(f) => Some(Content::Single(*f)),
        // `Str` is the schema's interned spelling; a declared default outlives the parse
        // exactly like a type name does. `Content::Str` is a raw `{ptr, len}` with no
        // owning variant, so the spelling is interned by an intentional, BOUNDED
        // `Box::leak` — one allocation per field that declares a text default, never one
        // per read — exactly as the IR/schema/snapshot readers do for the same type.
        Value::Text(s) => Some(Content::Str(Str::new(Box::leak(
            s.clone().into_boxed_str(),
        )))),
        _ => None,
    }
}

/// Set the correct type and initial size in definitions.
/// This will not factor in the space for attributes for records
/// as we still need to analyze the actual use of records.
pub fn complete_definition(_lexer: &mut Lexer, data: &mut Data, d_nr: u32) {
    match data.def(d_nr).name.as_str() {
        "vector" => {
            data.set_returned(d_nr, Type::Vector(Box::new(Type::Unknown(0)), Deps::none()));
            data.definitions[d_nr as usize].known_type = 7;
        }
        "integer" => {
            data.set_returned(d_nr, I32.clone());
            data.definitions[d_nr as usize].known_type = 0;
        }
        "float" => {
            data.set_returned(d_nr, Type::Float);
            data.definitions[d_nr as usize].known_type = 3;
        }
        "single" => {
            data.set_returned(d_nr, Type::Single);
            data.definitions[d_nr as usize].known_type = 2;
        }
        "text" => {
            data.set_returned(d_nr, Type::Text(Deps::none()));
            data.definitions[d_nr as usize].known_type = 5;
        }
        "boolean" => {
            data.set_returned(d_nr, Type::Boolean);
            data.definitions[d_nr as usize].known_type = 4;
        }
        "enumerate" => {
            data.set_returned(d_nr, Type::Enum(0, false, Deps::none()));
        }
        "function" => {
            data.set_returned(d_nr, Type::Routine(d_nr));
        }
        "character" => {
            data.set_returned(d_nr, Type::Character);
            data.definitions[d_nr as usize].known_type = 6;
        }
        "radix" | "hash" | "reference" | "index" | "sorted" | "spatial" | "trie" => {
            data.set_returned(d_nr, Type::Reference(d_nr, Deps::none()));
        }
        "keys_definition" => {
            data.set_returned(d_nr, Type::Keys);
            data.definitions[d_nr as usize].known_type = 8;
        }
        _ => {}
    }
}

/// loft#417 / @PLN25 — the ONE home for "is this `vector<S>` struct-element the
/// synthetic `__nullable<S>` enum (the nullable-by-default), and which def is it?".
/// BOTH the parse-time chokepoint (`e2_nullable_elem`) and the deferred forward-ref
/// resolver (`copy_unknown_fields` below) call it, so a `vector<S>` element resolves
/// to the SAME type whether `S` was defined before its use (known → rewritten at
/// parse) or after it (forward-ref → still `Unknown` at parse, resolved here once `S`
/// is finally known).  Before this single home existed the two resolvers disagreed:
/// the field went dense while its params/locals went nullable → element-stride
/// mismatch → corrupted enum-discriminant reads on both backends (loft#417).  Returns
/// the synth `__nullable<S>` enum def for an eligible element (a non-stdlib,
/// non-synthetic struct); `None` leaves the element dense.
/// The storage half of @FR-N-Dense: a `vector<S>` element is stored dense and non-null, and
/// only `vector<S?>` gets the tagged `__nullable<S>` that can hold absence (@FR-L-Null-Tag).
pub(crate) fn nullable_vector_elem(
    data: &mut Data,
    lexer: &mut Lexer,
    struct_d: u32,
) -> Option<u32> {
    if !synth_nullable_target(data, struct_d)
        || data.def(struct_d).source == crate::data::STD_SOURCE
    {
        return None;
    }
    Some(data.nullable_enum_for(lexer, struct_d))
}

/// May a synthetic `__nullable<S>` be minted for definition `struct_d`?
///
/// Call this before [`Data::nullable_enum_for`] from anywhere that decides a nullable
/// gets the ENUM representation rather than staying an `Optional`. It answers the part
/// of that question every caller shares; a caller with a narrower rule (the vector-element
/// path also excludes the stdlib) adds its own condition on top.
///
/// A type VARIABLE is the case worth naming. A template's `T` is a `DefType::Struct` from
/// user source with no attributes, so it satisfies every other condition here and reads as
/// a perfectly ordinary struct — but it is a placeholder, and which representation `τ?`
/// wants is a function of `τ`. Minting for it produced `__nullable<T>` with a payload of
/// no fields, and a `-> (T?, integer)` was then refused with *"field 'payload' has no
/// position"* on both backends. Leaving it an `Optional` is what lets substitution answer
/// it per monomorph, which is what a bare `-> T?` already did.
pub(crate) fn synth_nullable_target(data: &Data, struct_d: u32) -> bool {
    struct_d != u32::MAX
        && matches!(data.def_type(struct_d), DefType::Struct)
        && data.def(struct_d).synthetic.is_none()
        && !data.is_type_var_placeholder(struct_d)
}

fn copy_unknown_fields(data: &mut Data, d: u32) {
    for nr in 0..data.attributes(d) {
        // `Unknown(was)` names the forward-referenced type's STUB def — except for
        // `Unknown(0)`, which is the codebase-wide "no type known" sentinel and names
        // nothing (`Type::Unknown(0)` is what every unresolved expression carries).
        // Resolving THAT against definition #0 hands the field whatever the first
        // definition in the program happens to return — `text`, in practice — so a
        // field with no type silently acquires a plausible wrong one instead of staying
        // visibly unresolved (#686).  The `Vector` arm below already guarded it; this is
        // the same guard on the bare case.
        //
        // A `?` on the field is transparent here: `S?` and `S` name the same
        // forward reference, so peel the marker, resolve, and put it back.  Without
        // that, a `Roofs?` field kept `Optional(Unknown(stub))` after every other
        // spelling had resolved, and the first read of it reported the internal type
        // name (`optional(unknown(700))`) at the CALLER (loft#797).
        let (attr_type, optional) = match data.attr_type(d, nr) {
            Type::Optional(inner) => (*inner, true),
            other => (other, false),
        };
        if let Type::Unknown(was) = attr_type
            && was != 0
        {
            let resolved = data.def(was).returned.clone();
            set_attr_type_keeping_optional(data, d, nr, resolved, optional);
        } else if let Type::Vector(content, dep) = &attr_type
            && let Type::Unknown(was) = **content
            && was != 0
        {
            let dep = dep.clone();
            // Forward-ref element resolves DENSE — the dense-default invariant
            // ("`vector<τ>` is dense for every τ unless an explicit `τ?`").  A
            // `vector<S?>` carries its `?` from parse as a synth `__nullable<S>`
            // enum element (`e2_nullable_elem` registers it eagerly even when S is
            // a forward ref), so only the dense `vector<S>` ever reaches here as a
            // bare `Unknown`.  Wrapping it here (the pre-dense behaviour) is what
            // made a forward-referenced element's FIELD nullable while its
            // construction stayed dense → element-stride mismatch → corrupted
            // enum-discriminant reads / over-free on both backends (@PLN25 #465).
            let resolved = data.def(was).returned.clone();
            set_attr_type_keeping_optional(
                data,
                d,
                nr,
                Type::Vector(Box::new(resolved), dep),
                optional,
            );
        }
    }
}

/// Write a freshly resolved type onto attribute `nr`, restoring the `?` the caller peeled.
///
/// `set_attr_type` guards against overwriting a type that is already settled, and it reads
/// "settled" as `is_unknown() == false` — which an `Optional(…)` wrapper always is, whatever
/// it wraps.  Re-wrapping before the call would therefore trip the guard on exactly the
/// resolution it exists to allow, so the optional case writes the field directly, the same
/// escape [`Data::rewrite_unknown_refs`] takes for `Vector<Unknown>` and friends.
///
/// The re-wrap goes through [`Type::optional`], the idempotent former, and not a bare
/// `Type::Optional(Box::new(…))`: the `?` was peeled off a FORWARD reference, and what the
/// stub resolves to can itself be nullable — `type Maybe = integer?` declared after
/// `struct S { f: Maybe? }`.  A bare wrap built `integer??` there, a type `@FR-N-Idem` says
/// cannot exist, and the first write to the field was refused with that spelling in the
/// message (@PLN153 phase 0, the one constructor route the census found open).
fn set_attr_type_keeping_optional(
    data: &mut Data,
    d: u32,
    nr: usize,
    resolved: Type,
    optional: bool,
) {
    if optional {
        data.definitions[d as usize].attributes[nr].typedef = Type::optional(resolved);
    } else {
        data.set_attr_type(d, nr, resolved);
    }
}

/// Resolve forward type references accumulated during parsing.  When
/// `defer_unknown` is `Some`, every `DefType::Unknown` stub is recorded
/// as `(source, def_nr, position)` in the passed-in vec instead of being
/// emitted as a diagnostic — the caller is then responsible for either
/// patching the stub (via `Data::rewrite_unknown_refs`) or surfacing the
/// final "Undefined type" error later.
///
/// The package-mode driver uses this: cyclic intra-package `use`
/// declarations legitimately produce Unknown stubs for cross-file types
/// that will be resolved by `resolve_deferred_unknowns` after both sides
/// of the cycle have registered their definitions.
pub fn actual_types_deferred(
    data: &mut Data,
    database: &mut Stores,
    lexer: &mut Lexer,
    start_def: u32,
    mut defer_unknown: Option<&mut Vec<(u16, u32, crate::data::Position)>>,
) {
    // Determine the actual type of structs regarding their use
    for d in start_def..data.definitions() {
        if matches!(data.def_type(d), DefType::Struct) {
            data.definitions[d as usize].returned = Type::Reference(d, Deps::none());
        }
    }
    for d in start_def..data.definitions() {
        match data.def_type(d) {
            DefType::Unknown => {
                if let Some(buf) = defer_unknown.as_deref_mut() {
                    let def = data.def(d);
                    buf.push((def.source, d, def.position.clone()));
                    continue;
                }
                let name = &data.def(d).name;
                // `string` used to be special-cased here; it is now one row of the
                // cross-language alias table `suggest_type_name` consults, so this
                // site has one path and the table has one home (Goal E).
                let msg = if let Some(s) = data.suggest_type_name(name) {
                    format!("Undefined type {name} — did you mean '{s}'?")
                } else {
                    format!("Undefined type {name}")
                };
                lexer.pos_diagnostic(Level::Error, &data.def(d).position, &msg);
            }
            DefType::Function => {
                copy_unknown_fields(data, d);
                if let Type::Unknown(was) = data.def(d).returned {
                    data.set_returned(d, data.def(was).returned.clone());
                }
            }
            DefType::Struct => {
                copy_unknown_fields(data, d);
            }
            DefType::Enum => {
                // @PLN25 — a synthetic `__nullable<S>` enum's `Some` variant carries
                // S's fields, so its DB layout depends on S already being resolved.
                // Registering it HERE (before the struct layout loop) `enumerate`s
                // it at an index whose size is computed before S, so in a multi-type
                // program the enum's inline size is wrong (disc-only, 8 B) and
                // `vector<__nullable<S>>` construction/reads use the wrong stride.
                // Leave these to `synth_nullable_struct_fields` in `fill_all`, which
                // runs AFTER the struct-resolution order is established.
                if !(data.def(d).synthetic.is_some() && data.def(d).name.starts_with("__nullable<"))
                {
                    register_enum_db(data, database, d);
                }
            }
            DefType::EnumValue if data.attributes(d) > 0 => {
                copy_unknown_fields(data, d);
            }
            _ => {}
        }
    }
}

/// #682 — carry the post-`scopes::check` capture-ownership verdict into the
/// already-registered schema, so `free_named`'s cascade frees exactly the
/// captures the closure record adopted.
///
/// The interpreter's schema is laid out during parse, but which captures a record
/// owns is only settled by scope analysis (`scopes::mark_borrowed_captures`) —
/// hence this second, layout-preserving pass rather than a decision inside
/// `fill_database`.  `--native` needs no equivalent: its schema is emitted from
/// these same attribute types AFTER scope analysis has run.
///
/// Idempotent, and safe to run before scope analysis has marked anything (it then
/// finds no borrowed attribute and changes nothing).
pub fn sync_capture_ownership(data: &Data, database: &mut Stores) {
    for d in 0..data.definitions() {
        if !data.def(d).name.starts_with("__closure_") {
            continue;
        }
        let known = data.def(d).known_type();
        if known == u16::MAX {
            continue;
        }
        for a in 0..data.attributes(d) {
            if matches!(data.attr_type(d, a), Type::Reference(_, ref deps) if deps.is_borrowed_share())
            {
                database.borrow_dbref_field(known, &data.attr_name(d, a));
            }
        }
    }
}

/// Whether `d`'s layout has to wait, because a field's type is not known yet.
///
/// Laying a struct out anyway is what makes the failure a CORRUPTION rather than a
/// refusal: the field loop in [`fill_database`] silently skips an attribute it cannot
/// size, but the type is still registered and `finish` still sizes it — so the field
/// keeps `position == u16::MAX` forever, and `finish_type` will not revisit an
/// already-sized type.  The declaration and the layout then disagree for the rest of
/// the run: #686's closure body read and wrote its capture at offset 65535, and #797's
/// package field did the same to its record's neighbours.
///
/// Deferring costs nothing.  The registration loop in [`fill_all`] is keyed on
/// `known_type == u16::MAX`, so the next `fill_all` — the next file's, or the one after
/// `resolve_deferred_unknowns` — picks the struct up as soon as the type arrives.  A
/// field that never resolves leaves the struct unregistered, which is harmless: the
/// parser has already reported the undefined type, and `field_position` names the field
/// if anything still reaches for it.
///
/// Two kinds of "not known yet" reach here, and both must block:
///
///  * `Unknown(0)` names nothing.  It is what a field typed from an EXPRESSION carries,
///    and the only producer is the closure record — a capture's type is the type of
///    `w.chunks[1]`, not of a written-down name.  `resolve_forward_captures` repairs it.
///  * `Unknown(stub)` names a forward-referenced type whose declaration has not parsed
///    yet.  Within one file `copy_unknown_fields` resolves it before the layout loop, but
///    it only sweeps the file being finished — a struct in a module the package loaded
///    EARLIER keeps its stub, because the type it names is declared by a module still
///    suspended further up the `use` chain (loft#797).  The sweep at the top of
///    `fill_all` re-resolves those; whatever is still `Unknown` here is genuinely unknown.
///
/// The answer is transitive.  An inline struct field stores its content's bytes, so a
/// host whose field type is itself waiting cannot be laid out either — laying it out
/// would register the field with content id `u16::MAX`.
fn layout_blocked(data: &Data, d: u32, seen: &mut Vec<u32>) -> bool {
    if d == u32::MAX {
        return true;
    }
    if matches!(data.def_type(d), DefType::Unknown) {
        return true;
    }
    if data.def(d).known_type != u16::MAX {
        return false; // already laid out — its fields were known then
    }
    if !matches!(data.def_type(d), DefType::Struct | DefType::EnumValue) {
        return false;
    }
    if seen.contains(&d) {
        // A value cycle is rejected by `fill_all`'s own check; stopping here just
        // keeps this walk finite.
        return false;
    }
    seen.push(d);
    let blocked = (0..data.attributes(d)).any(|a| {
        !data.def(d).attributes[a].constant && type_blocked(data, &data.attr_type(d, a), seen)
    });
    seen.pop();
    blocked
}

/// The [`layout_blocked`] question asked of a TYPE rather than a definition: does laying
/// this out need a size nobody can supply yet?
///
/// The set of forms mirrors what [`fill_database`] actually asks of a field's content, so
/// that the two agree on which fields have a dependency at all.  A keyed collection needs
/// its content's type ID; a `Reference` with EMPTY deps stores the content's bytes inline
/// and so needs its size — but one with deps is a fixed-width `DbRef`, sized whatever the
/// content turns out to be, which is why that case does not wait (the same split the
/// native generator's field-hoist makes).
fn type_blocked(data: &Data, tp: &Type, seen: &mut Vec<u32>) -> bool {
    match tp.base() {
        Type::Unknown(_) => true,
        Type::Vector(c, _) | Type::RefVar(c) => type_blocked(data, c, seen),
        Type::Tuple(elms) => elms.iter().any(|e| type_blocked(data, e, seen)),
        Type::Hash(c, _, _)
        | Type::Index(c, _, _)
        | Type::Sorted(c, _, _)
        | Type::Radix(c, _, _)
        | Type::Trie(c, _, _) => layout_blocked(data, *c, seen),
        Type::Reference(c, deps) if deps.is_empty() => layout_blocked(data, *c, seen),
        _ => false,
    }
}

/// Resolve every pending layout, report the type cycles, and lay the records out.
///
/// Answers **true when a type CYCLE was reported**, which the caller needs before it lets
/// anything else look at these types: a type that contains itself has no finite size, so
/// `Stores::finish` recurses into the cycle and its `u16` offset accumulator wraps.  That
/// panic reaches the user as an internal compiler error and takes the buffered diagnostics
/// with it — including the one naming the cure — so a cyclic program reported nothing at all.
pub fn fill_all(data: &mut Data, database: &mut Stores, lexer: &mut Lexer, start_def: u32) -> bool {
    // Re-resolve the forward references of everything still waiting for a layout.
    //
    // `actual_types_deferred` sweeps only the file it is finishing, so a struct that
    // named a not-yet-declared type keeps `Unknown(stub)` on the attribute after its own
    // file is done.  The stub def is upgraded IN PLACE the moment its real declaration
    // parses (`parse_struct` reuses the stub's def_nr), which makes the attribute
    // resolvable from here on — but nothing was asking again.  That is loft#797: the
    // declaration ended up correct and the layout kept the hole.
    //
    // Ask again for every def whose layout is still pending, so each `fill_all` picks up
    // whatever the files parsed since have declared.  Resolving against a def that is
    // still a stub is a no-op (a stub's `returned` is its own `Unknown`), so this
    // converges without needing to know which pass finally supplies the type.
    for d_nr in 0..data.definitions() {
        if data.def(d_nr).known_type == u16::MAX
            && matches!(data.def_type(d_nr), DefType::Struct | DefType::EnumValue)
        {
            copy_unknown_fields(data, d_nr);
        }
    }
    // Detect type cycles before computing sizes.
    //
    // An ENUM is asked as well as a struct, because a struct-enum variant's payload is stored
    // INLINE in the host's bytes exactly as an embedded field is — so `enum E { Branch { n: E } }`
    // has no finite size either, and with no struct anywhere in it nothing else would ask.
    let mut found_cycle = false;
    for d_nr in start_def..data.definitions() {
        if matches!(data.def_type(d_nr), DefType::Struct | DefType::Enum) {
            let mut visiting = std::collections::HashSet::new();
            if data.has_value_cycle(d_nr, &mut visiting) {
                // Whether or not this def is one to REPORT, its layout is now unreachable —
                // so the flag is set before the reporting question is asked.  Getting that
                // order wrong lets `Stores::finish` run on a cyclic type after all.
                found_cycle = true;
                // A GENERATED def — `__tuple<integer,Node>`, `__nullable<S>` — lies on the
                // cycle exactly when the field that built it does, so reporting it says the
                // same thing twice and the second time names a type nobody wrote.  The walk
                // still travels THROUGH these defs; only the diagnostic stops at the author's
                // own types.
                if !data.def_is_authored(d_nr) {
                    continue;
                }
                let noun = if matches!(data.def_type(d_nr), DefType::Enum) {
                    "Enum"
                } else {
                    "Struct"
                };
                lexer.pos_diagnostic(
                    Level::Error,
                    &data.def(d_nr).position,
                    &format!(
                        "{noun} '{}' contains itself (directly or indirectly) — use reference<{}> to break the cycle",
                        data.def(d_nr).name,
                        data.def(d_nr).name,
                    ),
                );
            }
        }
    }
    if found_cycle {
        // Every step below lays records out, and a cyclic type has no layout to reach.  The
        // report is already made and it names the cure; going on can only replace it with a
        // panic from inside the record builder.
        return true;
    }
    // @PLN25 E2 — register the synthetic `__nullable<T>` enums + (gated) rewrite
    // embedded struct fields, BEFORE the unit-variant discriminant pass + the
    // layout loop below, so each synthetic enum is registered and laid out like
    // any hand-written enum.  Called unconditionally: the registration sweep
    // inside is a no-op when no `__nullable<>` enum exists (gate off), and the
    // field-rewrite arm is gated internally.
    synth_nullable_struct_fields(data, database, lexer);
    // Start from 0 (not start_def) so struct-enum variants defined in earlier
    // default library files are processed when later files trigger fill_all.
    // The has_type guard prevents double-processing.  Fixes S14 (PROBLEMS #80).
    // B2-runtime (2026-04-13): Before laying out records, retroactively
    // add a discriminant "enum" field to every unit variant of a mixed
    // struct-enum.  `parse_enum_values` only adds this field inside the
    // `has_token("{")` branch (struct variants), so sibling unit variants
    // have 0 attributes and would produce a size-0 structure — runtime
    // `OpDatabase(db_tp=…)` then panics `Incomplete record` in
    // `Store::claim(size=0)`.  Check the parent's `returned` (set to
    // `Type::Enum(_, true, _)` when ANY variant has braces) rather than
    // the unit-variant child's (which stays `Type::Enum(_, false, _)`).
    let enumerate_d_nr = data.def_nr("enumerate");
    if enumerate_d_nr != u32::MAX {
        for d_nr in 0..data.definitions() {
            if matches!(data.def_type(d_nr), DefType::EnumValue) && data.attributes(d_nr) == 0 {
                let parent = data.def(d_nr).parent;
                if parent != u32::MAX && matches!(data.def(parent).returned, Type::Enum(_, true, _))
                {
                    let discriminant = {
                        let mut v: u8 = 0;
                        for (a_nr, a) in data.def(parent).attributes.iter().enumerate() {
                            if a.name == data.def(d_nr).name {
                                v = a_nr as u8 + 1;
                                break;
                            }
                        }
                        v
                    };
                    data.add_attribute(
                        lexer,
                        d_nr,
                        "enum",
                        Type::Enum(enumerate_d_nr, false, Deps::none()),
                    );
                    let attr_nr = data.def(d_nr).attr_names["enum"];
                    data.set_attr_value(d_nr, attr_nr, Value::Enum(discriminant, u16::MAX));
                }
            }
        }
    }
    // QUALITY B5 fix: register `main_vector<T>` wrapper structs for every
    // `vector<T>` field found on a struct or enum-value.  Parser paths
    // that assign or construct a `vector<T>` already call
    // `data.vector_def(...)`, but **struct-enum variant fields** (e.g.
    // `Node { kids: vector<Tree> }` inside `enum Tree`) go through
    // `parse_enum_values` / `fill_all` without ever hitting a vector
    // assignment site.  Without the wrapper, `gen_set_first_vector_null`'s
    // `data.name_type("main_vector<Tree>")` lookup returns `u16::MAX`
    // and the interpreter emits `OpDatabase(var, db_tp=u16::MAX)` that
    // panics in `Store::claim` as "Incomplete record".  Register the
    // wrappers here, BEFORE the main `fill_database` loop, so the loop
    // then picks them up and assigns a real `known_type`.
    let mut pending: Vec<Type> = Vec::new();
    for d_nr in 0..data.definitions() {
        if !(matches!(data.def_type(d_nr), DefType::Struct)
            || matches!(data.def_type(d_nr), DefType::EnumValue))
        {
            continue;
        }
        for a_nr in 0..data.attributes(d_nr) {
            if let Type::Vector(content, _) = data.attr_type(d_nr, a_nr) {
                let content_tp = *content;
                let wrapper_name = format!("main_vector<{}>", content_tp.name(data));
                if data.def_nr(&wrapper_name) == u32::MAX {
                    pending.push(content_tp);
                }
            }
        }
    }
    for tp in pending {
        data.vector_def(lexer, &tp);
    }
    for d_nr in 0..data.definitions() {
        // @PLN22 Phase 2 — register every not-yet-registered struct / struct-enum
        // variant.  The guard is PER-DEF (`known_type == u16::MAX`), not
        // per-bare-name: a second def with a name the stdlib/another source
        // already registered (a shadowing `struct File`, or P379's two-library
        // same-name structs) must still be filled — fill_database registers it
        // under a source-qualified name.  A bare-name guard skipped it, leaving
        // `known_type = u16::MAX` and a runtime out-of-bounds on `self.types`.
        if ((matches!(data.def_type(d_nr), DefType::EnumValue) && data.attributes(d_nr) > 0)
            || matches!(data.def_type(d_nr), DefType::Struct))
            && data.def(d_nr).known_type == u16::MAX
            && !layout_blocked(data, d_nr, &mut Vec::new())
        {
            fill_database(data, database, d_nr);
            // @PLN25 E2 — right after building a struct `S`, build its synthetic
            // `__nullable<S>` enum's `Null` + `Some` variant STRUCTURES (if that
            // enum exists), so they take type-ids that follow `S` (and its field
            // types) but PRECEDE any later struct that holds a `hash<__nullable<S>>`
            // field.  Native codegen creates a keyed-collection struct field INLINE
            // and relies on its tid being reachable when the struct emits; building
            // the variants lazily (during that struct's hash field) gives `Some` a
            // tid AFTER the struct, so native interns hash<->Some swapped and a
            // baked `OpGetRecord(hash_tid)` resolves to `Some` → `find called on
            // non-collection type` at runtime.  Building them here (after `S`, not
            // up-front) keeps `S`'s field types created first (no native forward-
            // ref) yet still ahead of consumers.  Gate-inert: `__nullable<>` enums
            // exist only gate-on.
            if matches!(data.def_type(d_nr), DefType::Struct) {
                let syn_name = format!("__nullable<{}>", data.def(d_nr).name());
                let syn = data.def_nr(&syn_name);
                if syn != u32::MAX && data.def(syn).known_type != u16::MAX {
                    for variant in ["Null", "Some"] {
                        let v = data.variant_of(syn, variant);
                        if v != u32::MAX && data.def(v).known_type == u16::MAX {
                            fill_database(data, database, v);
                        }
                    }
                }
            }
        }
    }
    // P191 — pre-register database types for local-var keyed
    // collections (index/hash/spatial) so their bookkeeping fields
    // get appended to the content struct BEFORE database.finish()
    // runs finish_type to assign positions.
    //
    // Only Index appends bookkeeping fields (#left/#right/#color)
    // to the content struct; Hash/Radix just create an entry in
    // self.types without struct mutation.  But registering all three
    // here keeps the codepath uniform with what gen_set_first_keyed_null
    // would do later — and is idempotent (database.{index,hash,spatial}
    // dedup on name).
    //
    // Sorted is NOT in this loop — sorted doesn't append bookkeeping
    // fields, and P190's on-demand registration in get_type already
    // handles it.  Adding Sorted here would be a no-op anyway.
    //
    // **Critical timing**: this runs at the end of fill_all, which
    // runs at the end of EACH parse_file call.  At end of first-pass
    // parse_file, function variables are populated by parse_code (line
    // 804 of definitions.rs, called in both passes).  So the registration
    // happens BEFORE second-pass body parsing, which means
    // database.position() lookups during second-pass IR construction
    // see the post-bookkeeping struct layout.  Without this timing,
    // bookkeeping fields appended later (by gen_set_first_keyed_null
    // at codegen) stay at position 0 because finish_type only runs
    // for types with size == u16::MAX.
    for d_nr in start_def..data.definitions() {
        if !matches!(data.def_type(d_nr), DefType::Function) {
            continue;
        }
        let var_count = data.def(d_nr).variables.count();
        for v in 0..var_count {
            // @FR-L-Null: layout(τ) = layout(τ?).  A nullable keyed local holds the same
            // handle its dense twin does, so it has to reach this pre-registration the same
            // way — read BARE, `Optional(Index(…))` matched nothing, `index`'s bookkeeping
            // triple was appended after `finish()` had already sized the content struct, and
            // `#left_1 / #right_1 / #color_1` kept `position: 0` on top of each other and
            // the first real field.  The nullable form then refused to lay out at all while
            // its dense twin was fine — a layout the `?` changed, which is exactly what
            // @FR-L-Null forbids (loft#1125).  Invisible whenever the same index type also
            // has a DENSE local somewhere in the program: that one registers it in time and
            // the nullable one inherits a correct layout.
            let tp = data.def(d_nr).variables.tp(v).base().clone();
            match tp {
                Type::Hash(c, key, _) => {
                    let c_tp = data.def(c).known_type;
                    if c_tp != u16::MAX {
                        database.hash(c_tp, &key);
                    }
                }
                Type::Index(c, key, _) => {
                    let c_tp = data.def(c).known_type;
                    if c_tp != u16::MAX {
                        database.index(c_tp, &key);
                    }
                }
                Type::Radix(c, key, _) => {
                    let c_tp = data.def(c).known_type;
                    if c_tp != u16::MAX {
                        database.spatial(c_tp, &key);
                    }
                }
                Type::Trie(c, key, _) => {
                    let c_tp = data.def(c).known_type;
                    if c_tp != u16::MAX {
                        database.trie(c_tp, &key);
                    }
                }
                _ => {}
            }
        }
    }
    report_unknown_key_fields(data, lexer);
    false
}

/// loft#874 — report the key fields a keyed collection named that its ELEMENT type
/// does not have.
///
/// `set_mutable` records them because `fill_database` has no lexer; this is the
/// reporting half. The caret lands on the FIELD that declared the collection, which
/// is where the user wrote the name — the panic it replaces pointed at whatever line
/// the layout happened to be reached from, and that line is correct as written.
///
/// The message names the element type, because the most common way in (`hash<K, V>`,
/// the spelling nearly every other language uses) leaves a name that looks like a
/// perfectly good type and is being read as a FIELD — saying only "unknown field"
/// would confirm the user's wrong model instead of correcting it.
fn report_unknown_key_fields(data: &mut Data, lexer: &mut Lexer) {
    for (decl_d, decl_a, elem_d, name) in data.take_unknown_key_fields() {
        let field = data.attr_name(decl_d, decl_a);
        let elem = data.def(elem_d).name().to_string();
        // Only a struct-shaped element HAS fields.  A base type reached here because
        // the element slot itself is wrong — `hash<integer, At>` puts the key where
        // the element goes — and listing what `integer` answers to would offer its
        // METHODS as candidate keys, which is a worse answer than none.
        let structured = matches!(
            data.def_type(elem_d),
            DefType::Struct | DefType::EnumValue | DefType::Enum
        );
        let candidates = if structured {
            data.attr_names_of(elem_d)
        } else {
            Vec::new()
        };
        // The hint carries its own terminator: a did-you-mean ends in a question
        // mark, and appending a full stop to that reads as a typo in the compiler.
        let hint = match crate::diagnostics::suggest_similar_capped(&name, &candidates) {
            Some(s) => format!(" — did you mean `{s}`?"),
            None if !structured => format!(" — `{elem}` is not a struct, so it has no fields."),
            None if candidates.is_empty() => ".".to_string(),
            None => format!(" — the fields of `{elem}` are: {}.", candidates.join(", ")),
        };
        let msg = format!(
            "Field `{field}`: `{name}` is not a field of `{elem}`, so it cannot be a key{hint} \
             A keyed collection names its keys as FIELDS OF ITS ELEMENT — write \
             `hash<Element[key_field]>`, not `hash<key, Element>`"
        );
        let position = data.def(decl_d).position.clone();
        lexer.pos_diagnostic(Level::Error, &position, &msg);
    }
}

/// @PLN25 E2a.2 — rewrite each nullable struct-typed field to the synthetic
/// `__nullable<T>` enum (`Null | Some<fields>`), so an absent value is
/// representable inline (discriminant `0`) instead of crashing the
/// `OpCopyRecord`-of-a-null-source path.  Runs at the very start of `fill_all`
/// (before the unit-variant discriminant pass + the layout loop) so the
/// synthetic enum is registered (`register_enum_db`) and laid out like any
/// hand-written enum.
///
/// Two arms:
/// - **Embedded-field rewrite** (`item: Row?` → `__nullable<Row>`).  Selects on the `?` the
///   author wrote — a field with no `?` cannot be absent and stays dense, so it needs no
///   discriminant and pays for none.
/// - **Registration sweep** — lays out every `__nullable<S>` enum the VECTOR-element path
///   (`e2_nullable_elem`, gated on `LOFT_E2_SYNTH`) created at parse time.  A no-op when
///   none exist (gate off).
fn synth_nullable_struct_fields(data: &mut Data, database: &mut Stores, lexer: &mut Lexer) {
    // @PLN25 E2a.2 / loft#896 — an embedded struct-typed field written `item: Row?`
    // becomes the synthetic `__nullable<Row>` enum, so "absent" is discriminant `0`
    // and has somewhere to live.  A field typed `item: Row` stays DENSE: it cannot be
    // absent, so it needs no discriminant and pays for none.
    //
    // The `?` in the source is the whole trigger, and it reaches here as the
    // `Optional` WRAPPER — `Optional(Reference(Row))`.  Matching a bare `Reference`
    // instead selected the exact complement of that set (every NON-nullable struct
    // field, since `nullable` is a legacy flag that is true by default), which is why
    // this arm sat behind an opt-in: rewriting dense fields tree-wide does break field
    // reads across the stdlib, and it never once fired for the `S?` it was written for.
    //
    // A field VECTOR `items: vector<Row?>` is rewritten at the vector-type chokepoint
    // (`sub_type`'s `vector` arm), so by here its content is already the enum — not an
    // `Optional` — and falls through.  Keyed collections, primitives, fn-refs are out
    // of scope: only a heap-typed field stores its payload inline with no room for
    // absence.
    {
        for host in 0..data.definitions() {
            // ⚠ This skips the ENUM DISPATCHER and nothing else.  `Definition::synthetic` is
            // set at exactly one site (`ir_schema.rs`, `Some("enum_dispatcher")`); the
            // generated types this comment used to name — `__tuple<…>`, `__fn_ref`,
            // `__nullable<T>` — are built by `add_def`, which leaves the field `None`, so the
            // guard has never skipped one.  `Data::def_is_authored` is the predicate that
            // would.  Recorded rather than changed because the rewrite is CORRECT on the host
            // it actually reaches: a `Node?` tuple member is an inline position by
            // `formal/layout.md` (L-Null-Tag), so it wants the tagged form like any embedded
            // field.  Adding the skip would be a behaviour change with no measured case
            // asking for it.
            if data.def(host).synthetic.is_some() {
                continue;
            }
            if !(matches!(data.def_type(host), DefType::Struct)
                || (matches!(data.def_type(host), DefType::EnumValue) && data.attributes(host) > 0))
            {
                continue;
            }
            for a_nr in 0..data.attributes(host) {
                // Skip the per-variant `constant` markers.
                if data.def(host).attributes[a_nr].constant {
                    continue;
                }
                let Type::Optional(inner) = data.attr_type(host, a_nr) else {
                    continue;
                };
                let Type::Reference(struct_d, ref deps) = *inner else {
                    continue;
                };
                // `Type::Reference` is one IR spelling for two source notions, and only
                // one of them wants a tag.  An EMBEDDED struct field (`item: Row`) stores
                // its payload inline and has no bit pattern to spare, so `Row?` takes the
                // tagged `__nullable<Row>` — `@FR-L-Null-Tag`.  A `reference<Row>` field
                // is a 12-byte pointer that already reserves `nullref`, so `@FR-L-Null`
                // governs it instead: `layout(τ?) = layout(τ)` and absence is the
                // sentinel in those same bytes.  The FIELD's own share-marker dep
                // (`u16::MAX`, #328) is what tells the two apart — the same bit
                // `Data::has_value_cycle` reads to skip these edges, so the walks and
                // this rewrite cannot disagree about which edge is which.  This site is
                // `@FR-L-Null-Which`'s one home: it is the only place the choice between
                // the two representations is made for a field.
                //
                // Tagging a reference field is what erased the pointer: `reference<Leaf>?`
                // and `Leaf?` laid out byte-identically, so `?` silently turned a shared
                // pointer into an inline copy, and on a type whose reference graph returns
                // to itself the inline form has no finite size at all — the reader got
                // `field 'next' has no position (u16::MAX)` for a linked list's terminator
                // (loft#1316).
                if deps.contains(&u16::MAX) {
                    continue;
                }
                // The same eligibility question the vector-element path asks, read from
                // the one home.  This site had its own spelling and was missing the
                // type-variable case, which is how a tuple built from a template's `T?`
                // got a `__nullable<T>` over an attribute-less placeholder.
                if !synth_nullable_target(data, struct_d) {
                    continue;
                }
                let syn = data.nullable_enum_for(lexer, struct_d);
                if data.def(syn).known_type == u16::MAX {
                    register_enum_db(data, database, syn);
                }
                data.definitions[host as usize].attributes[a_nr].typedef =
                    Type::Enum(syn, true, Deps::none());
            }
        }
    }
    // E2a.5b — register any synthetic `__nullable<>` enum the LOCAL/param
    // parse-time rewrite (expressions.rs `e2_nullable_vec_local`) created but the
    // field loop above did not reach (no struct field references it).  Doing the
    // `register_enum_db` HERE — in `fill_all`, before the layout loop — instead of
    // mid-body-parse is what keeps the discriminant db-type laid out correctly;
    // registering it during parsing corrupts every read of the shared enum.
    // loft#803 — and any HAND-WRITTEN enum the range-scoped registration missed.
    //
    // `actual_types_deferred` registers enums over `start_def..`, which reads as
    // "everything this file just added". An ADOPTED stub breaks that: a module
    // that names `Colour` before the importer declares it leaves a stub, the
    // importer's own `enum Colour` upgrades that stub IN PLACE, and the stub's
    // def number is BELOW the resuming importer's `start_def`. So the one def
    // that IS the enum sits outside every range that would have registered it,
    // `known_type` stays `u16::MAX`, and `enum_val` answers `unknown` for every
    // variant — a wrong value, not an error.
    //
    // Scanning from 0 is the fix rather than widening the range, because the
    // condition is a fact about the DEF (it has no db type yet), not about where
    // its number happens to fall. Registration is idempotent, so a def the
    // in-order pass already handled is re-stamped and not re-minted.
    for d in 0..data.definitions() {
        if matches!(data.def_type(d), DefType::Enum) && data.def(d).known_type == u16::MAX {
            register_enum_db(data, database, d);
        }
    }
}

/// Register an enum's database type and its variant entries, and stamp each
/// parent variant attribute's discriminant value with the database enum id.
/// Shared by `actual_types_deferred` (hand-written enums) and `fill_all`'s
/// @PLN25 nullable-struct-field synthesis (synthetic `__nullable<T>` enums),
/// so both register identically.
fn register_enum_db(data: &mut Data, database: &mut Stores, d: u32) {
    // ALREADY MINTED — re-stamp only. `parse_enum` rebuilds this def's attributes
    // on every pass, so pass 2's variants carry no discriminant unless the stamp
    // runs again; but `enumerate` PUSHES a type, so running the whole of this
    // twice mints a second `Colour` and renumbers every type id after it — which
    // is what made the generated `init()` reference a `t189` it had not declared
    // (loft#803, attempt 3). The two halves have different idempotence, so they
    // are separated here rather than guarded together at the call site.
    //
    // The name computation below is skipped as well, deliberately: its
    // `__nullable<` disambiguation keys on "the bare name is already a db type",
    // which is true of THIS def's own second visit and would rename it.
    if data.def(d).known_type != u16::MAX {
        stamp_enum_variants(data, database, d, data.def(d).known_type);
        return;
    }
    let mut name = data.def(d).name.clone();
    // @PLN22 (p379 `two_libs_same_struct_name`): two libraries may each define a struct
    // of the same name `S`.  `nullable_enum_for` already gives each `S` its own synth
    // `__nullable<S>` DEF (keyed on the struct's source), but both DEFS share the bare db
    // name `__nullable<S>`, so their `Null`/`Some` variant structures collide in the flat
    // db type table ("Double structure type __nullable<S>::Null") and field access binds
    // to the wrong payload struct.  When the bare name is already a db type (the second
    // definer), disambiguate by the PAYLOAD struct's QUALIFIED def name — keeping the
    // `__nullable<` prefix so `nullable_some_variant` still resolves it (a `lib::`-prefixed
    // `qualified_type_name` would not).  The struct's db name is not laid out yet at this
    // point (register runs before the struct layout loop), so use the def name, not the db
    // name.  The first definer keeps the bare name, so non-colliding programs are unchanged.
    if data.def(d).synthetic.is_some()
        && name.starts_with("__nullable<")
        && database.has_type(&name)
    {
        let some_v = data.variant_of(d, "Some");
        let payload_attr = data.attr(some_v, "payload");
        if payload_attr != usize::MAX
            && let Type::Reference(sd, _) = data.attr_type(some_v, payload_attr)
        {
            name = format!("__nullable<{}>", data.qualified_type_name(sd));
        }
    }
    // MINT, always — idempotence is keyed on the DEF above, never on the name.
    //
    // A db name is NOT unique across defs. The stdlib declares `enum Format`
    // (`02_files.loft`) and a program may declare its own, which is the same
    // collision the `__nullable<` disambiguation right above exists for. Reusing a
    // same-named db type here made the user's `Format` adopt the STDLIB one, so
    // its second variant read back `LittleEndian` — a silent wrong value, and
    // precisely the failure this whole fix is about. `enumerate` shadows the name,
    // and that shadowing is what keeps two same-named enums apart.
    //
    // Adoption needs nothing from here: it upgrades a stub IN PLACE, so there is
    // one def and one `known_type`, and the early return above is the only guard a
    // second visit needs.
    let e_nr = database.enumerate(&name);
    stamp_enum_variants(data, database, d, e_nr);
    data.definitions[d as usize].known_type = e_nr;
}

/// Give each of this enum's variants its discriminant, in both places that hold
/// one: the database's variant list and the def's own attribute values.
///
/// Runs on EVERY registration, including a repeat — `parse_enum` rebuilds the
/// def's attributes each pass, so a stamp that ran only when the type was first
/// minted leaves pass 2 (the pass that generates code) with variants that carry
/// no discriminant, and every one of them renders as `unknown`.
///
/// The database half is add-if-absent because `Stores::value` appends blindly:
/// a second pass over an already-populated enum would give it `Red, Green, Red,
/// Green` and shift what each discriminant names.
fn stamp_enum_variants(data: &mut Data, database: &mut Stores, d: u32, e_nr: u16) {
    for a in 0..data.attributes(d) {
        let v_name = data.attr_name(d, a);
        let known = match &database.types[e_nr as usize].parts {
            crate::database::Parts::Enum(values) => values.iter().any(|(_, n)| *n == v_name),
            _ => false,
        };
        if !known {
            database.value(e_nr, &v_name, u16::MAX);
        }
        data.set_attr_value(d, a, Value::Enum(a as u8 + 1, e_nr));
    }
}

/// @PLN25 — register + lay out a synth `__nullable<S>` enum ON DEMAND, for a FORWARD-referenced `S`
/// whose synth enum is first created during pass-2 body parse — AFTER `fill_all`'s in-order
/// registration ran.  Without it the enum keeps `known_type == u16::MAX`, the `Some` payload byte
/// position + the element size both read 0/MAX, and `v[0].field` reads garbage (371).  `S` is fully
/// laid out by pass 2, so this sizes the enum + `Some` immediately.  No-op once registered.
///
/// CRITICAL: do NOT `fill_database` the ENUM def itself — that runs `structure()` and re-registers it
/// (under a qualified name), OVERWRITING `known_type` away from the enum, so the variant link below
/// targets a struct and the `Some` size never reaches `Parts::Enum`.  Only the VARIANT structs go
/// through `fill_database` (its `EnumValue` arm calls `enum_value` to link each into the enum).
// @PLN25 dense flip — superseded for the paths that exist today by
// `nullable_vector_elem` + `copy_unknown_fields` (e2_nullable_elem), which handle the
// forward-referenced synth layout. Kept (allow dead) for the Phase-0 EXPAND residual:
// when `?` parsing extends to return/param type positions, a forward-ref `vector<S?>`
// synth created there may need this on-demand layout again.
#[allow(dead_code)]
pub(crate) fn register_and_lay_out_synth(data: &mut Data, database: &mut Stores, synth_d: u32) {
    if synth_d == u32::MAX || data.def(synth_d).known_type() != u16::MAX {
        return;
    }
    register_enum_db(data, database, synth_d);
    let variants: Vec<u32> = data.children_of(synth_d).collect();
    for v in &variants {
        fill_database(data, database, *v);
    }
    let enum_kt = data.def(synth_d).known_type();
    let some_d = data.variant_of(synth_d, "Some");
    let some_kt = if some_d == u32::MAX {
        u16::MAX
    } else {
        data.def(some_d).known_type()
    };
    database.lay_out_synth(enum_kt, some_kt);
}

/// A free DB structure name for an enum VARIANT whose bare and source-qualified
/// names are both taken — `<parent enum's DB name>::<variant>`.
///
/// The parent enum is itself a registered structure, so its DB name is already
/// unique and the variant name below it cannot collide. `None` when the def is not
/// a variant, when the parent has no registered type yet, or (defensively) when
/// even that name is taken — the caller then keeps the source-qualified name and
/// the registration aborts with its own diagnostic rather than a silent alias.
fn variant_parent_qualified_name(data: &Data, database: &Stores, d_nr: u32) -> Option<String> {
    if data.def_type(d_nr) != DefType::EnumValue {
        return None;
    }
    let parent = data.def(d_nr).parent;
    let parent_name = database.type_name(data.def(parent).known_type);
    if parent_name.is_empty() {
        return None;
    }
    let name = format!("{parent_name}::{}", data.def(d_nr).name);
    (!database.has_type(&name)).then_some(name)
}

pub(crate) fn fill_database(data: &mut Data, database: &mut Stores, d_nr: u32) {
    if data.def(d_nr).name == "Unknown(0)" {
        return;
    }
    // The generic type-var marker (`<T>`) is a single shared def referenced by every
    // `vector<T>` param across the stdlib generics; fill it ONCE.  A second fill for
    // another such param must be a no-op or `database.structure` would panic on the
    // mangled name below.
    if data.is_type_var_placeholder(d_nr) && data.def(d_nr).known_type != u16::MAX {
        return;
    }
    let mut enum_value = 0;
    if let Type::Enum(nr, true, _) = data.def(d_nr).returned {
        for (a_nr, a) in data.def(nr).attributes.iter().enumerate() {
            if a.name == data.def(d_nr).name {
                enum_value = a_nr as i32 + 1;
                break;
            }
        }
    }
    // @P379 — struct-type registration is a flat table keyed by name.  When
    // two libraries each define a struct of the same bare name (different
    // field layouts), register the second under a library-qualified name
    // (`moros_map::Chunk`) instead of panicking `Double structure type`.
    // The bare name stays for the first/only definer, so non-colliding
    // programs are byte-identical.  The parser already resolves each usage
    // to the correct per-library `d_nr` (and hence `known_type`); this only
    // makes the database table tolerate the shared bare name.
    // @PLN25 — synthetic `__nullable<S>` enums all name their variants `Null` /
    // `Some`, but `Some` carries a DIFFERENT payload per `S`, so they cannot share
    // one structure-table entry (and `Null` would `Double structure type` the
    // moment a second `__nullable<>` enum exists).  Register each under its
    // parent-enum-qualified name (`__nullable<Row>::Some`) so the flat DB type
    // table stays collision-free.  Variant lookup keys on the bare name + parent
    // enum (`database.enum_value` below) and runtime discriminants, so this
    // changes only the structure-table key, not resolution.
    // @PLN22 (p379 `two_libs_same_struct_name`): two libs may define same-named structs `S`, so a
    // synth `__nullable<S>`'s `Null`/`Some` variant structures need a UNIQUE db name.  Key the
    // variant on the PARENT enum's DB name (`register_enum_db` already disambiguated it — bare
    // `__nullable<S>` for the first definer, `__nullable<lib::S>` for the second), NOT the parent's
    // bare DEF name (shared by both), so the two libs' `__nullable<S>::Null` no longer collide.
    let synth_variant_name = if data.def_type(d_nr) == DefType::EnumValue {
        let parent = data.def(d_nr).parent;
        (data.def(parent).synthetic.is_some() && data.def(parent).name.starts_with("__nullable<"))
            .then(|| {
                format!(
                    "{}::{}",
                    database.type_name(data.def(parent).known_type),
                    data.def(d_nr).name
                )
            })
    } else {
        None
    };
    let reg_name = if data.is_type_var_placeholder(d_nr) {
        // The generic type-var marker (`<T>`) is an INTERNAL compile-time construct.
        // Register its runtime type under a name a user type can never share, so
        // nothing it derives (`vector<T>`) collides in the name-keyed type table with
        // a user type of the same name (`enum T`) — which made the user's `vector<T>`
        // reuse the marker's size-0 entry and divide by zero.  The DEF name stays `T`
        // for stdlib `<T>` resolution; only the runtime type name is mangled.
        format!("{TYPEVAR_ROW_PREFIX}{}", data.def(d_nr).name)
    } else if let Some(name) = synth_variant_name {
        name
    } else if database.has_type(&data.def(d_nr).name) {
        // The source qualifier separates two LIBRARIES that define the same name.  It
        // cannot separate two definitions in ONE source, so a third same-named
        // structure aborted the compiler — `enum A { Nil, … } enum B { Nil, … }
        // enum C { Nil, … }` in one file is enough, and the abort read as an internal
        // error on legal code.  A variant's parent enum is itself a registered type,
        // so qualifying with the parent's DB name is unique by construction; it is the
        // same escape the synthetic `__nullable<S>` variants take above.  Reached only
        // once the source-qualified name is ALSO taken, so every program that compiles
        // today keeps the name it has.
        let qualified = data.qualified_type_name(d_nr);
        if database.has_type(&qualified) {
            variant_parent_qualified_name(data, database, d_nr).unwrap_or(qualified)
        } else {
            qualified
        }
    } else {
        data.def(d_nr).name.clone()
    };
    // `LOFT_TRACE_SCHEMA` — see `database::types::schema_trace`.  Logging the DEF
    // behind each registration is what makes a duplicate attributable: the abort
    // names only the colliding type, while the fault is one def being filled
    // twice (a rolled-back parse re-creating it), which shows up here as the same
    // `d_nr` registering a bare name and then a `src0::`-qualified one (#618).
    if std::env::var_os("LOFT_TRACE_SCHEMA").is_some() {
        eprintln!(
            "[schema] fill d_nr={d_nr} src={} name={:?} -> reg={reg_name:?}",
            data.def(d_nr).source,
            data.def(d_nr).name,
        );
    }
    let s_type = database.structure(&reg_name, enum_value);
    data.definitions[d_nr as usize].known_type = s_type;
    if data.def_type(d_nr) == DefType::EnumValue {
        let e_tp = data.def(d_nr).parent;
        let enum_tp = data.def(e_tp).known_type;
        database.enum_value(enum_tp, &data.def(d_nr).name, data.def(d_nr).known_type);
    }
    for a_nr in 0..data.attributes(d_nr) {
        // Computed fields are not stored — skip them in the database layout.
        if data.def(d_nr).attributes[a_nr].constant {
            continue;
        }
        let a_type = data.attr_type(d_nr, a_nr);
        // @PLN25 slice (b): an `Optional(τ)` field lays out exactly like `τ` (same sentinel
        // storage) — peel the marker here so the whole DB-layout path (the `db_type` match +
        // `size`) is transparent to it. Nullability is read separately via `attr_nullable`.
        let a_type = a_type.base().clone();
        let t_nr = data.type_elm(&a_type);
        let nullable = data.attr_nullable(d_nr, a_nr);
        if t_nr < u32::MAX {
            let tp = match a_type {
                Type::Vector(c_type, _) => {
                    let c_nr = data.type_elm(&c_type);
                    // unresolved vector content — parser already emitted
                    // a diagnostic (constant-shadow, undefined type, etc.).
                    // Skip this attribute rather than panicking so the user
                    // sees the proper error instead of an interpreter crash.
                    if c_nr == u32::MAX {
                        continue;
                    }
                    // route through the shared resolver so struct fields, locals,
                    // parameters, return types and literals all derive the element
                    // id the same way (narrow leaf, nested vector, plain
                    // `known_type`).  `None` = the leaf has no id yet, which is
                    // the one case this site can fix itself: fill it, then retry.
                    let c_tp = if let Some(elem) = data.vector_element_type(&c_type, database) {
                        elem
                    } else {
                        fill_database(data, database, c_nr);
                        data.vector_element_type(&c_type, database)
                            .unwrap_or(data.def(c_nr).known_type)
                    };
                    let tp = database.vector(c_tp);
                    data.check_vector(c_nr, tp, &data.def(d_nr).position.clone());
                    tp
                }
                Type::Integer(int_spec) => {
                    let IntegerSpec {
                        not_null,
                        forced_size: spec_forced,
                        ..
                    } = int_spec;
                    let field_nullable = nullable && !not_null;
                    // Post-2c: if the field's alias has a forced size(N)
                    // annotation, prefer it over the limit()-based heuristic.
                    // The alias def_nr was captured in parse_field because
                    // Type::Integer collapses alias names.
                    let alias = data.def(d_nr).attributes[a_nr].alias_d_nr;
                    // @PLN114 — fall back to the width the TYPE carries when the
                    // attribute has no alias to consult.  `parse_field` captures
                    // `alias_d_nr` for a declared struct field, but the synthetic
                    // `__tuple<…>` struct's attributes are built by `tuple_def` from
                    // element Types alone, so `forced_size(alias)` finds nothing and
                    // the range heuristic silently widens: `u8` became a 2-byte
                    // `short` and `u16` an 8-byte `integer`, which is why a tuple
                    // packed to 16 bytes where the identical record packs to 3.
                    // `IntegerSpec.forced_size` is already stamped by `parse_type`
                    // (definitions.rs:1869), so the fact is present — it just was
                    // not being read on this path.
                    let s = data
                        .forced_size(alias)
                        .or_else(|| spec_forced.map(std::num::NonZeroU8::get))
                        .unwrap_or_else(|| a_type.size(field_nullable));
                    // The Part carries the offset the field's READ and WRITE ops encode
                    // against (`part_min`), which is not the declared `min` for a nullable
                    // SIGNED narrow field: it sacrifices its bottom edge to the null
                    // sentinel, so a present `i16?` rendered one too low through the schema
                    // while reading the same field answered correctly.
                    let m = int_spec.part_min(s, field_nullable);
                    // The schema Part MUST match the op the codegen chose via the ONE
                    // width→op home, so both are taken from the SAME `NarrowIntKind`
                    // rather than re-derived here from the width.  What the schema Part
                    // decides is the READ (`ShowDb` / `to_json` / the store round-trip /
                    // every keyed lookup); a slot whose Part names a different encoding
                    // than its ops answers those routes wrong while a direct field access
                    // stays correct, which is the shape both loft#812 (2-byte, the `+1`
                    // shift a direct write never did) and loft#1437 (4-byte, a sign-extended
                    // `u32`) took.  A struct field is not a narrow-vector element, so
                    // `narrow_vec` is false; a width with no narrow Part keeps the wide
                    // 8-byte `integer`.
                    crate::data::NarrowIntKind::of(
                        s,
                        field_nullable,
                        false,
                        int_spec.unsigned_wide(),
                    )
                    .part(database, m, field_nullable)
                    .unwrap_or_else(|| database.name("integer"))
                }
                Type::Hash(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    let kd = key_bearing_def(data, c_nr);
                    // @PLN25 E2 — for a synth `__nullable<S>` element the keys live
                    // in the `Some` variant, which is built up-front by the
                    // eager-variant pass in `fill_all` (so `database.hash` resolves
                    // the key fields through it AND the hash's tid lands after
                    // `Some` for native codegen).  Safety net: if a hash is reached
                    // before that pass has run, build `Some` now so key resolution
                    // still succeeds (idempotent — a no-op once the pass has run).
                    if data.def(kd).known_type == u16::MAX {
                        fill_database(data, database, kd);
                    }
                    set_mutable(data, kd, &key_fields, (d_nr, a_nr));
                    database.hash(c_tp, &key_fields)
                }
                Type::Index(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    // @PLN25 E2 — for a synth `__nullable<S>` element the key fields live in the
                    // `Some` payload, so resolve the key-bearing def (mirror the hash arm) before
                    // marking them immutable; `c_nr` (the enum) has no direct key attribute.
                    let kd = key_bearing_def(data, c_nr);
                    if data.def(kd).known_type == u16::MAX {
                        fill_database(data, database, kd);
                    }
                    set_mutable_directed(data, kd, &key_fields, (d_nr, a_nr));
                    database.index(c_tp, &key_fields)
                }
                Type::Sorted(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    let kd = key_bearing_def(data, c_nr);
                    if data.def(kd).known_type == u16::MAX {
                        fill_database(data, database, kd);
                    }
                    set_mutable_directed(data, kd, &key_fields, (d_nr, a_nr));
                    database.sorted(c_tp, &key_fields)
                }
                Type::Radix(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    // @PLN25 E2 — for a synth `__nullable<S>` element the key fields live in
                    // the `Some` payload, so resolve the key-bearing def (mirror the hash arm).
                    let kd = key_bearing_def(data, c_nr);
                    if data.def(kd).known_type == u16::MAX {
                        fill_database(data, database, kd);
                    }
                    set_mutable(data, kd, &key_fields, (d_nr, a_nr));
                    database.spatial(c_tp, &key_fields)
                }
                Type::Trie(c_nr, key, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    let kd = key_bearing_def(data, c_nr);
                    if data.def(kd).known_type == u16::MAX {
                        fill_database(data, database, kd);
                    }
                    set_mutable(data, kd, std::slice::from_ref(&key), (d_nr, a_nr));
                    database.trie(c_tp, &key)
                }
                Type::Enum(t, _, _) if data.def(t).name == "enumerate" => database.byte(0, false),
                Type::Function(_, _, _) => {
                    // P213: when a capturing-lambda assignment has been
                    // seen at this attribute (its d_nr recorded on
                    // `assigned_lambda_d_nr` during first-pass parsing
                    // of `set_field_check`), split into TWO database
                    // fields:
                    //   `<attr>`              : 4B int, lambda d_nr
                    //   `<attr>__closure_rec` : `Parts::ChildRec(closure_kt)`,
                    //                           4B u32 rec-id of the
                    //                           co-located closure record
                    // For attributes that never received a capturing
                    // assignment (non-capturing fn-ref struct fields,
                    // tuple elements of fn-ref type, default-init only)
                    // stay with the legacy single-field 4B int layout —
                    // closure_rec field would be wasted space and
                    // breaks layouts of containers (tuples) that pre-
                    // computed positions assuming 4B per fn-ref slot.
                    let attr_name = data.def(d_nr).attributes[a_nr].name.clone();
                    let lambda_d = data.def(d_nr).attributes[a_nr].assigned_lambda_d_nr;
                    let closure_rec_d = if lambda_d == u32::MAX {
                        u32::MAX
                    } else {
                        data.def(lambda_d).closure_record
                    };
                    if closure_rec_d == u32::MAX {
                        // Legacy 4B int layout (non-capturing /
                        // tuple-element / default-init).
                        let int_tp = database.int(0, false);
                        database.field(s_type, &attr_name, int_tp);
                    } else {
                        let mut c_tp = data.def(closure_rec_d).known_type;
                        if c_tp == u16::MAX {
                            fill_database(data, database, closure_rec_d);
                            c_tp = data.def(closure_rec_d).known_type;
                        }
                        let dnr_tp = database.int(0, false);
                        let crec_tp = database.child_rec(c_tp);
                        database.field(s_type, &attr_name, dnr_tp);
                        database.field(s_type, &format!("{attr_name}__closure_rec"), crec_tp);
                    }
                    continue;
                }
                Type::Tuple(_) => {
                    // Plan-06 phase 4d: tuple struct fields inline the
                    // synthetic `__tuple<…>` struct's bytes.  The
                    // synthetic struct is registered eagerly by
                    // `parse_type_full`, but its database-side layout
                    // is built by `fill_database` on the synthetic
                    // def itself — recurse first so its `known_type`
                    // is non-`u16::MAX` when we register the host
                    // struct's tuple field below.  Mirrors the
                    // vector / sorted / hash / index recursion above.
                    let mut c_tp = data.def(t_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, t_nr);
                        c_tp = data.def(t_nr).known_type;
                    }
                    c_tp
                }
                Type::Reference(_, ref deps) if !deps.is_empty() => {
                    // Plan-22 phase 02b (2026-05-12): auto-Reference
                    // encoding for mutated captures.  When the
                    // attribute's dep list is non-empty, the field
                    // holds a 12-byte `DbRef` pointing at the source
                    // record (shared storage) instead of inline
                    // bytes (deep-copy).  The dep list is the marker
                    // — phase 02c is the only producer that sets it
                    // for closure-record attributes; today's user
                    // code path always has empty deps so the
                    // legacy inline-bytes path stays active for
                    // every existing struct field.
                    //
                    // #682: which of the two markers decides whether the
                    // record ADOPTS the captured store (cascade-freed with
                    // the record) or merely BORROWS it (freed by its real
                    // owner).  Same 12 bytes either way — `generation` picks
                    // the same pair for `--native`.
                    if deps.is_borrowed_share() {
                        database.dbref_borrow()
                    } else {
                        database.dbref()
                    }
                }
                _ => {
                    // A struct/enum-reference field stored INLINE (`inner: Cell`,
                    // empty deps) — its bytes live inside the host record, so the
                    // host layout needs the content type's size now.  The host can
                    // be declared BEFORE the content (a forward or cross-package
                    // reference), in which case the content's `known_type` is still
                    // u16::MAX here.  Lay it out first — mirroring the vector /
                    // tuple / keyed-collection recursion above — otherwise the
                    // field's content id stays u16::MAX, `finish_type` cannot
                    // position it (the field lands at offset u16::MAX, never
                    // repaired on pass 2 because `finish_type` skips an
                    // already-sized type), and codegen reads the bogus offset and
                    // corrupts the free path (@P373: a SIGSEGV at scope exit
                    // AFTER the correct value prints).  Primitive fields already
                    // carry a real `known_type`, so the guard never recurses for
                    // them; a genuinely-undefined `Unknown(0)` stub short-circuits
                    // in `fill_database` (already diagnosed elsewhere).
                    let mut kt = data.def(t_nr).known_type;
                    if kt == u16::MAX {
                        fill_database(data, database, t_nr);
                        kt = data.def(t_nr).known_type;
                    }
                    kt
                }
            };
            database.field(s_type, &data.attr_name(d_nr, a_nr), tp);
            // @PLN127 arc D — the ONE parse-time site that knows. `a_type` was
            // peeled above (an `Optional(τ)` lays out exactly like `τ`), so
            // without depositing it here the fact is gone by the time anything
            // can be asked about it.
            if nullable {
                database.set_field_nullable(s_type, &data.attr_name(d_nr, a_nr), true);
            }
            // loft#876 — the same "ONE parse-time site that knows" for the field's
            // DECLARED default.  It lives here as an IR node and the store layer has no
            // evaluator, so a cast could not consult it and wrote the type's zero.
            if let Some(c) = fold_declared_default(&data.def(d_nr).attributes[a_nr].value) {
                database.set_field_default(s_type, &data.attr_name(d_nr, a_nr), c);
            }
        }
    }
    // Propagate Data-side LinkedFieldGroups (currently: tuple element
    // groups registered by `tuple_def`) to the Database-side Type so
    // `Stores::finish_type` can place them atomically via
    // `calculate_positions_with_groups`.  Index bookkeeping groups
    // are added directly on the Database side by `Stores::index`, so
    // they don't need this copy.
    let groups = data.def(d_nr).field_groups.clone();
    if !groups.is_empty() {
        database.types[s_type as usize].field_groups.extend(groups);
    }
}

/// @PLN25 E2 — the key-bearing struct for a keyed collection.  When the element
/// was rewritten to the synthetic `__nullable<S>` enum (E2), its key field(s) live
/// inside the `Some` variant, not at the enum's top level, so key-field name lookups
/// must resolve against `Some` (whose payload offsets match the Some-wrapped records
/// the collection shares with its sibling vector).  A non-synthetic content def is
/// returned unchanged — gate-inert.
pub(crate) fn key_bearing_def(data: &Data, c_nr: u32) -> u32 {
    if data.def_type(c_nr) == DefType::Enum && data.def(c_nr).name.starts_with("__nullable<") {
        let some = data.variant_of(c_nr, "Some");
        if some != u32::MAX {
            // Single-payload: the key fields live inside the `Some` variant's inline
            // `payload` field (a dense `S`), so the key-bearing def is the payload's
            // struct, not the `Some` variant (whose direct fields are {enum, payload}).
            let payload_attr = data.attr(some, "payload");
            if payload_attr != usize::MAX
                && let Type::Reference(struct_d, _) =
                    data.def(some).attributes()[payload_attr].typedef
            {
                return struct_d;
            }
        }
    }
    c_nr
}

/// Mark a keyed collection's key fields immutable on the element definition —
/// and record, rather than index with, a key field the element does not have.
///
/// [`Data::attr`] answers `usize::MAX` for a name it cannot find, and that is
/// reachable from ordinary source: a typo (`hash<At[nosuch]>`) and the
/// `hash<K, V>` spelling every other language uses (loft#874) both land here
/// with a name that is not an attribute. Indexing with the sentinel panicked
/// with a Rust location and a caret pointing at a line that was correct as
/// written, which is the one response a user cannot act on.
///
/// The name is deferred to [`Data::take_unknown_key_fields`] rather than
/// reported here because `fill_database` has no lexer — the same
/// record-here / report-there shape as `defer_unknown` above. `decl` is the
/// FIELD that declared the collection, not the element type, so the caret can
/// land on the declaration the user wrote.
fn set_mutable(data: &mut Data, on_d: u32, fields: &[String], decl: (u32, usize)) {
    for f in fields {
        let a_nr = data.attr(on_d, f);
        if a_nr == usize::MAX {
            data.record_unknown_key_field(decl, on_d, f);
            continue;
        }
        data.definitions[on_d as usize].attributes[a_nr].mutable = false;
    }
}

fn set_mutable_directed(data: &mut Data, on_d: u32, fields: &[(String, bool)], decl: (u32, usize)) {
    for f in fields {
        let a_nr = data.attr(on_d, &f.0);
        if a_nr == usize::MAX {
            data.record_unknown_key_field(decl, on_d, &f.0);
            continue;
        }
        data.definitions[on_d as usize].attributes[a_nr].mutable = false;
    }
}
