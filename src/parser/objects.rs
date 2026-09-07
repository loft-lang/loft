// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    AmpHead, DefType, HashSet, I32, IntegerSpec, Level, LexItem, LexResult, Mode, OUTPUT_DEFAULT,
    OutputState, Parser, Position, SKIP_TOKEN, SKIP_WIDTH, ToString, Type, Value,
    diagnostic_format, to_default, v_block, v_if, v_set,
};

// Variable resolution, struct construction, and object parsing.

/// One member of a linked collection GROUP — a field whose collection is one route to a
/// record set shared with its siblings (@FR-Col-Group).
pub(crate) struct GroupMember {
    /// Index of the field in its container's attribute list, which is DECLARATION order.
    pub(crate) a_nr: usize,
    pub(crate) name: String,
    /// Keyed kinds are what make a group form at all; a plain `vector` member is a member
    /// but cannot be the reason there is a group.
    pub(crate) keyed: bool,
}

/// The ops a struct-literal field parse cannot emit in place, collected for the
/// caller (`Parser::parse_object`) to splice at a fixed point.  Both exist
/// because a field's ops are order-sensitive against the record-creation
/// prelude (`Set(x, Null)` + `OpDatabase`), and the field parse does not know
/// where that prelude ends.
#[derive(Default)]
pub(crate) struct FieldSinks {
    /// #330 — a field initialiser that READS the in-place target, lifted into a
    /// temp that must run BEFORE the prelude re-inits the record.
    hoists: Vec<Value>,
    /// #437 — the 4-byte header zeroing for each VECTOR field, spliced as one
    /// block directly AFTER the prelude so no field's header is cleared after
    /// its contents land.
    vector_headers: Vec<Value>,
    /// loft#924 — byte offsets of the fields whose header `parse_object` already
    /// zeroed in the prelude because they belong to a LINKED COLLECTION GROUP.
    /// The field parse and `object_init` both consult it so the header is written
    /// once, in the one place that can order it ahead of every member's fill.
    group_primed: HashSet<u16>,
    /// loft#926 — collection fields this literal gives RECORDS to, as opposed to the
    /// `field: []` that every construction of a linked group writes.  Only a member
    /// actually handed records can surprise the author about which set they landed in,
    /// so this, not `found_fields`, is what the advice counts.
    filled_collections: HashSet<String>,
    /// loft#1266 — the VECTOR members this literal filled in bulk, each owing its linked
    /// group's other members an `OpIndexGroup`.  Deferred to the end of `parse_object`
    /// rather than emitted at the field, because the answer depends on which OTHER members
    /// the literal fills — and a member later in the same literal is not known yet when an
    /// earlier one is handled.
    group_fills: Vec<Value>,
}

impl Parser {
    /// loft#1008 — the receiver TYPES of every method registered under the bare name `name`.
    ///
    /// A method is stored as `t_<len><Type>_<name>`, so a bare name has no definition of its
    /// own and reads as "unknown" wherever a value is wanted. Answering the receivers lets the
    /// diagnostic say what the name IS rather than that it is missing, and there may be more
    /// than one — arg-type dispatch means `area` can be a method on three shapes at once.
    /// Sorted and de-duplicated, because a diagnostic whose wording depends on hash order is
    /// not a contract.
    pub(crate) fn method_receivers_named(&self, name: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in 0..self.data.definitions() {
            let raw = self.data.def(d).name();
            let Some(body) = raw.strip_prefix("t_") else {
                continue;
            };
            let digits: String = body.chars().take_while(char::is_ascii_digit).collect();
            let Ok(len) = digits.parse::<usize>() else {
                continue;
            };
            let rest = &body[digits.len()..];
            if rest.len() < len {
                continue;
            }
            let (ty, tail) = rest.split_at(len);
            if tail.strip_prefix('_') == Some(name) {
                out.push(ty.to_string());
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// @P387 / @P383 — the user-facing argument types of a fn used as a
    /// first-class `fn` value.  Excludes the SYNTHETIC return buffer the fn-ref
    /// dispatch injects: `__work_ret` / `__retbuf` while still `__`-prefixed, AND
    /// the @PLAN59 signature-time `__retbuf` after it's renamed to the returned
    /// local's name (the return type's deps still name that attr).  Without this
    /// a value-returning fn mis-types as `fn(integer, S) -> S` and can't be used
    /// as a `fn` value.
    fn fn_ref_arg_types(&self, fn_d_nr: u32) -> Vec<Type> {
        let n_args = self.data.attributes(fn_d_nr);
        // The fn-ref TYPE is the VISIBLE parameters only.  Synthetic
        // return-buffers (struct/vector `__retbuf`, the text work-buffer) are
        // marked `hidden`, so excluding by `hidden` drops exactly them — and,
        // unlike the old "in the return deps" test, KEEPS a genuinely-returned
        // parameter (`fn f(s: text) -> text { s }` has `s` in its return deps
        // but `s` is a real, non-hidden arg — @P387 case 2).
        (0..n_args)
            .filter(|&a| {
                !self.data.def(fn_d_nr).attributes()[a].hidden
                    && !self.data.attr_name(fn_d_nr, a).starts_with("__")
            })
            .map(|a| self.data.attr_type(fn_d_nr, a))
            .collect()
    }

    /// @PLN22 Phase 1 — true if `tp` denotes an enum, either directly
    /// (`Type::Enum`) or via a `Type::Reference` to an enum def.  Used to seed
    /// the expected-enum context for bare value-position variant resolution
    /// once variants are no longer globally keyed.
    ///
    /// loft#1065 — read through `base()`: a NULLABLE enum target is an enum context too.
    /// Without the peel, `s: Shape? = Shape::Circle { r: 7 }` never resolved the variant
    /// against `Shape`, so the local was retyped to `Circle` and the declaration refused
    /// itself ("cannot change type from Shape? to Circle") — while the same literal in a
    /// bare `Shape` slot beside it was accepted. Whether the slot may be ABSENT says
    /// nothing about which variants it can hold.
    pub(crate) fn enum_context(&self, tp: &Type) -> bool {
        match tp.base() {
            Type::Enum(_, _, _) => true,
            Type::Reference(d_nr, _) => self.data.def_type(*d_nr) == DefType::Enum,
            _ => false,
        }
    }

    /// #511 / @PLN93 — the five collection kinds a closure captures by shared DbRef.
    /// A captured collection is stored in the closure record as a `Reference` DbRef, so
    /// the body must recover its real (collection) type from `capture_context` to keep
    /// `h[key]` / iteration typed correctly.
    /// loft#1071 — does this value view a collection ELEMENT SLOT that may be absent?
    ///
    /// A `for e in v` loop variable is a sub-reference INTO `v`'s element slot rather than
    /// a handle of its own, so absence lives in the four-byte word at that slot and not in
    /// a store-pointer sentinel.  The question has to follow the dep CHAIN rather than read
    /// the first link, because a use-site type deps on the variable itself and only the
    /// declaration deps on the collection.
    ///
    /// The reached collection's ELEMENT must be nullable, and that half is what makes this
    /// a slot question instead of a borrow question.  An inline absent slot exists only
    /// where an element is allowed to be absent: a value viewing a DENSE `vector<t>` is
    /// necessarily a whole handle, and its absence is the store sentinel.  Asked without
    /// it, a plain local bound from a view-returning call — `b = head(v)`, whose deps name
    /// the caller's vector because that is what the return borrows — took the slot test,
    /// read a discriminant through a null handle, and answered PRESENT for the value the
    /// callee had just said was absent (loft#1421).
    ///
    /// Bounded, because a self-dep (`e` depending on `e`) is exactly the shape that makes
    /// the walk necessary and would otherwise make it loop.
    pub(crate) fn views_a_nullable_element_slot(&self, tp: &Type) -> bool {
        let mut deps: Vec<u16> = tp.depend();
        let mut seen: Vec<u16> = Vec::new();
        for _ in 0..3 {
            let mut next: Vec<u16> = Vec::new();
            for d in deps {
                if d >= self.vars.count() || seen.contains(&d) {
                    continue;
                }
                seen.push(d);
                let dt = self.vars.tp(d);
                if Self::is_collection_type(dt.base()) {
                    // The walk stops at the FIRST collection: that is the one this value
                    // would be a slot of, and a dense one settles the question.
                    return matches!(dt.base(), Type::Vector(elm, _) if matches!(**elm, Type::Optional(_)));
                }
                next.extend(dt.depend());
            }
            if next.is_empty() {
                return false;
            }
            deps = next;
        }
        false
    }

    /// loft#1071 — the `(base, field)` an INLINE struct-enum slot read addresses, when
    /// `v` is such a read.
    ///
    /// A struct-enum FIELD stores a four-byte record pointer, and `0` is how absence is
    /// spelled there. `OpGetField` on it answers a sub-REFERENCE rather than that word, so
    /// a caller that needs to ask "is this slot empty" has to re-address the word itself.
    pub(crate) fn inline_slot_word(&self, v: &Value) -> Option<(Value, Value)> {
        let Value::Call(d, args) = v.unspan() else {
            return None;
        };
        if self.data.def(*d).name() != "OpGetField" || args.len() < 2 {
            return None;
        }
        Some((args[0].clone(), args[1].clone()))
    }

    /// Is this IR node the value a source-level `null` becomes?
    ///
    /// Two spellings reach a store site: the bare `Value::Null` the parser starts with, and
    /// the typed sentinel `convert` rewrites it into once it knows the target type — for a
    /// heap target that is `OpNullRefSentinel()`.  A site that must recognise "the author
    /// wrote `null` here" has to accept both, because which one arrives depends on whether
    /// the target's type was resolved before the value was.
    pub(crate) fn is_null_source(&self, val: &Value) -> bool {
        match val.unspan() {
            Value::Null => true,
            Value::Call(d, args) => {
                args.is_empty() && self.data.def(*d).name() == "OpNullRefSentinel"
            }
            _ => false,
        }
    }

    pub(crate) fn is_collection_type(tp: &Type) -> bool {
        // One home: `vectors::is_collection` (formal/IMPLEMENTATIONS.md #4).
        crate::parser::vectors::is_collection(tp)
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_var(
        &mut self,
        code: &mut Value,
        name: &str,
        parent_tp: &mut Type,
        name_pos: &Position,
    ) -> Type {
        // '$' refers to the current record in struct field default expressions
        if name == "$" && matches!(self.data.def_type(self.context), DefType::Struct) {
            // Noted HERE rather than at the field it selects: a `$.x` naming a field
            // declared later in the struct does not resolve in pass 1, and a default that
            // reads the record must be recognised as one in BOTH passes.
            if self.init_field_tracking {
                self.init_reads_record = true;
            }
            *code = Value::Var(0);
            return Type::Reference(self.context, crate::data::Deps::none());
        }
        let mut source = u16::MAX;
        let qualified = self.lexer.has_token("::");
        let nm = if qualified {
            source = self.data.get_source(name);
            // A package qualifying a call with its OWN name means "in this file".
            // `use_names` never holds the main file, so the lookup above misses and
            // the code below would report "Unknown library" for the library being
            // read right now (loft#656).  Resolving to the current source is what
            // the author wrote: those definitions are already parsed.
            if source == u16::MAX && self.own_lib.as_deref() == Some(name) {
                source = self.data.source;
            }
            if let Some(id) = self.lexer.has_identifier() {
                id
            } else {
                diagnostic!(self.lexer, Level::Error, "Expecting identifier after ::");
                name.to_string()
            }
        } else {
            name.to_string()
        };
        // @PLN22 Phase 1 — for a qualified `Enum::Variant` (the qualifier is a
        // local enum, not a library), the variant is resolved WITHIN that enum
        // via the variant_of chokepoint inside parse_constant_value, so it keeps
        // working once variants are no longer globally keyed (step 4).
        let qualifier_enum = if qualified && source == u16::MAX {
            let q = self.data.def_nr(name);
            if q != u32::MAX && self.data.def_type(q) == DefType::Enum {
                q
            } else {
                u32::MAX
            }
        } else {
            u32::MAX
        };
        // Tier-0 auto-`use`: a qualified `name::…` whose lowercase `name` is
        // neither a loaded library (`source` == MAX) nor a known definition is
        // an unknown library.  After the use-region's pre-scan, any *available*
        // library would already be loaded, so this is the genuine "no such
        // library" case — report it directly instead of the downstream "Unknown
        // function" (reported once types are known, in the second pass).
        if qualified
            && source == u16::MAX
            && !self.first_pass
            && self.data.def_nr(name) == u32::MAX
            && name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        {
            // loft#1043 — before calling it unknown, ask whether it is one of THIS
            // package's own modules.  `use self::<m>;` registers the module under
            // `<pkg>::<m>` and deliberately does NOT take the flat `<m>::` qualifier
            // slot: that slot is shared by the whole dependency graph, and not
            // sharing it is the whole point of the `self::` spelling (loft#976).  So
            // the module is bound and reachable BARE, and only the qualifier is
            // missing — which "Unknown library" states as the opposite, sending the
            // reader to look for a file that is right there.
            let own_module = self
                .own_package_name()
                .filter(|pkg| self.data.use_exists(&format!("{pkg}::{name}")));
            if let Some(pkg) = own_module {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{name}` is this package's own module, bound by `use self::{name};` — it has no `{name}::` qualifier.\n  The short qualifier is ONE slot shared by every package in the graph, and staying out of it is what `use self::` is for — so it is withheld on purpose, not missing.\n  fix: drop the qualifier (`use self::{name};` already imports its names bare), or bind one with `use self::{name} as <alias>;` and write `<alias>::`.\n  note: an alias DOES take the shared slot, so choose a name no other package would — `{pkg}_{name}` rather than `{name}`."
                );
            } else {
                diagnostic!(self.lexer, Level::Error, "Unknown library '{name}'");
            }
            // Consume a trailing call so the parser does not also choke on the
            // arguments and emit a second, less-helpful error.
            if self.lexer.has_token("(") && !self.lexer.has_token(")") {
                loop {
                    let mut arg = Value::Null;
                    self.expression(&mut arg);
                    if !self.lexer.has_token(",") {
                        self.lexer.has_token(")");
                        break;
                    }
                }
            }
            return Type::Unknown(0);
        }
        // vector<T>.parse(text) — parse a JSON array into a vector of T.
        if nm == "vector" && self.lexer.has_token("<") {
            if let Some(elem_name) = self.lexer.has_identifier() {
                let elem_d_nr = self.data.def_nr(&elem_name);
                self.lexer.token(">");
                if self.lexer.has_token(".") && self.lexer.has_keyword("parse") {
                    if elem_d_nr != u32::MAX {
                        return self.parse_vector_parse(elem_d_nr, code);
                    }
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown type '{elem_name}' in vector<{elem_name}>.parse()"
                        );
                    }
                    return Type::Unknown(0);
                }
            }
            // Not a vector<T>.parse() — cannot recover tokens, report error.
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expected '.parse(' after vector<T>"
                );
            }
            return Type::Unknown(0);
        }
        // @PLN25 E2a.3 — transparent construction: when the expected type is a
        // synthetic nullable-struct enum `__nullable<S>` (propagated into
        // `parent_tp` by parse_single) and the literal is the underlying struct
        // `S { … }`, build the `Some` variant instead — the discriminant
        // defaults to present via object_init, so `item: Row{…}` maps onto the
        // enum with the user still writing the struct name.
        // The expected type may arrive either as the inline-nullable `Enum(syn, true)`
        // (vector elements, declared fields) or as `Reference(syn)` (a keyed
        // collection's element ref / a typed-local keyed slot, whose `elm` var is
        // typed `Reference(Some)` and propagates the parent as a Reference).  Accept
        // both so a typed-LOCAL `hash<S[k]> += [S{…}]` builds `Some` in place, exactly
        // like the field path — without it the literal builds a dense `S` that a raw
        // OpCopyRecord then mis-lays into the `Some` record (wrong offsets, no
        // discriminant → null/garbage reads).  Mirrors `unique_elm_var`, which already
        // resolves the element through `Some` for both forms.
        let syn_nullable = match &*parent_tp {
            Type::Enum(syn, true, _) | Type::Reference(syn, _) => Some(*syn),
            _ => None,
        };
        if let Some(syn) = syn_nullable
            && self.data.def(syn).name == format!("__nullable<{nm}>")
            && self.lexer.peek_token("{")
        {
            let some_d = self.data.variant_of(syn, "Some");
            if some_d != u32::MAX {
                let tp = self.parse_object(some_d, code);
                if tp != Type::Unknown(0) {
                    return tp;
                }
            }
        }
        // @PLN22 Phase 1, unqualified twin — a bare `Variant { … }` / `Variant` resolves
        // through the FLAT def key, which is first-wins: with two enums declaring an
        // `Item`, every mention meant the first one's, so `p: PV2 = Item { v: 42 }` failed
        // with "Cannot assign integer to field Item.v of type text" — naming a field the
        // program never mentioned, from an enum it never named.  The annotation says which
        // enum is meant, so route the name through the SAME `variant_of` chokepoint an
        // explicit `PV2::Item` uses.
        //
        // Additive by construction: it fires only for a `{`-CONSTRUCTION whose expected
        // type is an enum that HAS this variant.  If that enum is the first definer the
        // answer is unchanged; if it is not, the program does not compile today.  With no
        // expected enum (a bare `p = Item { … }`) nothing changes and first-wins decides.
        //
        // The `{` is load-bearing, not cosmetic.  Without it a bare UNIT variant
        // (`a: Q1 = Nil` with three enums declaring `Nil`) resolved to the wrong enum and
        // broke a program that compiles today — caught by keeping shared-unit-variant and
        // no-annotation rows in the matrix beside the constructor ones.  A unit variant
        // already resolves correctly through its own path; only the construction form
        // needed the annotation.
        let qualifier_enum = if qualifier_enum == u32::MAX && self.lexer.peek_token("{") {
            let want = match &*parent_tp {
                Type::Enum(e, _, _) => *e,
                Type::Reference(d, _) if self.data.def_type(*d) == DefType::Enum => *d,
                _ => u32::MAX,
            };
            if want != u32::MAX && self.data.variant_of(want, &nm) != u32::MAX {
                want
            } else {
                u32::MAX
            }
        } else {
            qualifier_enum
        };
        let mut t = self.parse_constant_value(code, source, &nm, name_pos, qualifier_enum);
        if t != Type::Null {
            return t;
        }
        if self.lexer.has_token("(") {
            // @F45 — sizeof()
            if name == "sizeof" {
                t = self.parse_size(code);
            } else if name == "type_name" {
                t = self.parse_type_name(code);
            } else if name == "typedef" {
                let mut p = Value::Null;
                let et = self.expression(&mut p);
                self.lexer.token(")");
                let tp = self.data.def(self.data.type_def_nr(&et)).known_type();
                t = Type::Integer(IntegerSpec {
                    min: 0,
                    max: 65536,
                    not_null: false,
                    forced_size: None,
                });
                *code = Value::Int(i32::from(tp));
            } else {
                t = self.parse_call(code, source, &nm, name_pos);
            }
        } else if self.closure_param != u16::MAX
            && !self.first_pass
            && self.data.def(self.context).closure_record() != u32::MAX
            && self
                .data
                .attr(self.data.def(self.context).closure_record(), name)
                != usize::MAX
        {
            // A5.3/A5.4: redirect captured variable reads to closure record field.
            let closure_d_nr = self.data.def(self.context).closure_record();
            let fnr = self.data.attr(closure_d_nr, name);
            *code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
            // @PLN93 (#511): a collection capture's stored attr is a `Reference` DbRef,
            // but the body must see the ORIGINAL collection type (from capture_context)
            // so `h[key]` / iteration type-check — the DbRef value read via OpGetDbRef is
            // exactly what a collection variable is.
            let captured_collection_tp = self
                .capture_context
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, ct)| ct.clone())
                // `.base()` — the same reading its twin below takes: a `vector<τ>?` capture is
                // a collection capture, so the body sees the collection type the source wrote
                // rather than the shared attribute's `Reference(elem)` (loft#1209).
                .filter(|ct| Self::is_collection_type(ct.base()));
            t = captured_collection_tp.unwrap_or_else(|| self.data.attr_type(closure_d_nr, fnr));
            // closure record is a struct — add __closure as dep so the
            // store allocation stays alive while derived text/references are in use.
            t = t.depending(self.closure_param);
        } else if name == "_" && self.at_binding_name() {
            // loft#795 — `_` is the DISCARD, not a variable: nothing ever reads it, so
            // each `_ = …` gets its own slot rather than re-binding one shared `_`.
            //
            // Sharing one slot made `_` the single name exempt from the
            // one-type-per-name rule (`change_var_type` retyped it silently), and the
            // backends then disagreed about what that meant.  The interpreter re-typed
            // the slot per assignment and ran; native takes the Rust local's type from
            // the FIRST assignment, so `_ = delete(…); _ = flag(); _ = delete(…)` emitted
            // a `bool` lowering into a `u8` slot and the program did not compile (E0308).
            // Three assignments were needed — two types alternating is what breaks it —
            // which is why a discard-heavy function could work for a long time and then
            // stop when one more `_` was added.
            //
            // A slot each fixes both halves without costing anyone the spelling: `_`
            // keeps working for any mix of types (making it obey the rule instead would
            // reject programs that run today, for the one name whose entire purpose is
            // to be written more than once).  It is the same treatment `for _` loops
            // already needed for their hidden counter, and for the same reason.
            let v_nr = self.create_unique("discard", &Type::Unknown(0));
            self.var_usages(v_nr, true);
            *code = Value::Var(v_nr);
            t = Type::Unknown(0);
        } else if self.vars.name_exists(name) {
            let index_var = self.vars.var(name);
            // on pass 2, if a variable has Unknown type, it may be a pass-1
            // placeholder for a forward-declared function. Try fn-ref resolution.
            //
            // Not at a BINDING position: there the name is the local being
            // written, and a local may share a function's spelling (see the
            // bare-name path below).  A binding whose type pass 1 could not yet
            // infer — `turn = a_forward_fn();` — still arrives here as an
            // untyped placeholder, and resolving it to the function would hand
            // parse_assign a function-ref where its target belongs.
            if !self.first_pass && self.vars.tp(index_var).is_unknown() && !self.at_binding_name() {
                let prefixed = format!("n_{nm}");
                let fn_d_nr = self.data.def_nr(&prefixed);
                if fn_d_nr != u32::MAX && matches!(self.data.def_type(fn_d_nr), DefType::Function) {
                    // Suppress "never read" warning on the pass-1 placeholder.
                    self.var_usages(index_var, true);
                    *code = Value::Int(fn_d_nr as i32);
                    self.data.def_used(fn_d_nr);
                    self.record_sandbox_fn_ref(fn_d_nr); // @PLN86 L4
                    // A forward-declared value-returning fn re-resolved on pass 2
                    // reaches here; same buffer-exclusion as the bare-name path.
                    let arg_types = self.fn_ref_arg_types(fn_d_nr);
                    let ret_type = self.data.def(fn_d_nr).returned().clone();
                    return Type::Function(
                        arg_types,
                        Box::new(ret_type),
                        crate::data::Deps::none(),
                    );
                }
            }
            // @PLN115 S2 — record this occurrence as a read of the local
            // `index_var` in the enclosing function.  Pass 2 only (the var exists
            // from pass 1, types are resolved, and pass-1 records are cleared at
            // the boundary); gated + zero-cost when recording is off.
            if !self.first_pass {
                self.record(
                    name_pos,
                    name.chars().count() as u16,
                    crate::resolution::Resolution::Local {
                        fn_def: self.context,
                        var_nr: index_var,
                    },
                );
            }
            if self.lexer.has_token("#") {
                self.var_usages(index_var, true);
                if self.lexer.has_keyword("errors") {
                    // s#errors — return the parse errors from the last Type.parse() call.
                    let fn_nr = self.data.def_nr("i_parse_errors");
                    if fn_nr != u32::MAX {
                        *code = Value::Call(fn_nr, vec![]);
                        t = Type::Text(crate::data::Deps::none());
                    }
                    return t;
                }
                self.iter_op(code, name, &mut t, index_var);
            } else if let Value::Var(into) = code {
                let v_nr = self.vars.var(name);
                if matches!(self.vars.tp(v_nr), Type::Text(_)) {
                    t = self.vars.tp(v_nr).clone();
                } else {
                    t = self.vars.tp(v_nr).depending(v_nr);
                }
                // @PLN25 DN3 flow-narrowing: a read of a var PROVEN non-null by an enclosing
                // guard types as the peeled base, not `τ?` (see `narrowed_non_null`).
                if self.narrowed_non_null.contains(&v_nr) {
                    t = t.base().clone();
                }
                self.var_usages(v_nr, true);
                // `@FR-B-Copy`: a plain whole-value bind COPIES, so the destination is
                // made INDEPENDENT of the source and ends up owning its own store.  Read
                // through `base()`, because `S?` is `Optional(Reference(S))` — the same
                // storage behind a nullability marker — and matching the wrapper-free
                // spelling alone left the destination DEPENDING on the source, which is an
                // alias: `bns = ns; ns.v = 99` then read 99 through `bns`, and the same for
                // a nullable `vector` (loft#1319).  Nullability is not one of `(B-Copy)`'s
                // three exceptions — `(B-View)` is a struct PROJECTION, `(B-View-Base)` a
                // borrowed base, `(B-View-Depth)` an index or nested read — and this is a
                // whole value off an owned local.
                //
                // And through `heap_def_nr()`, the one home for "is this a heap RECORD":
                // a struct-enum is the same shape, and spelled as a bare `Reference` this
                // arm kept a nullable struct-enum destination DEPENDING on its source while
                // both emitters copied — a copy nobody freed.  `Data::copies_as` admits the
                // `(C-Var)` widening `c: E = s` with `s` a variant of `E` beside the
                // same-def pair.
                if let Some(d_nr) = self.vars.tp(*into).base().heap_def_nr()
                    && let Some(vd_nr) = self.vars.tp(v_nr).base().heap_def_nr()
                    && self.data.copies_as(d_nr, vd_nr)
                {
                    // Don't create OpCopyRecord here: generate_set handles the copy when
                    // value=Var(src). Using Var(v_nr) directly lets method calls like
                    // `d = c.double()` pass c as `self` without the broken CopyRecord-as-self
                    // pattern that was causing garbage store_nr crashes (Issue 1).
                    let into_var = *into;
                    self.vars.make_independent(into_var, v_nr);
                    *code = Value::Var(v_nr);
                    // The COPY is what this arm decides; whether the value may be ABSENT is
                    // the source's own fact and survives it.  `t` already carries that,
                    // including the flow-narrowing peel above, so a source proven non-null
                    // in this branch answers the bare type and a `τ?` stays `τ?` — dropping
                    // the marker here would make `bns` non-null and lose the null arm.
                    // The destination's own base spelling (`Reference` or struct-`Enum`),
                    // with no deps: the copy owns.
                    let bare = self.vars.tp(into_var).base().without_deps();
                    return if matches!(t, Type::Optional(_)) {
                        Type::Optional(Box::new(bare))
                    } else {
                        bare
                    };
                }
                *code = Value::Var(v_nr);
            } else {
                let v_nr = self.vars.var(name);
                t = self.vars.tp(v_nr).depending(v_nr);
                // @PLN25 DN3 flow-narrowing: proven non-null in this branch → peeled base.
                if self.narrowed_non_null.contains(&v_nr) {
                    t = t.base().clone();
                }
                self.var_usages(v_nr, true);
                *code = Value::Var(v_nr);
            }
        } else if let Some((_cname, ctype)) = self
            .capture_context
            .iter()
            .find(|(n, _)| n == name)
            .cloned()
        {
            // @PLN93 (#511): a collection capture is stored in the closure record as a
            // shared DbRef (synthesize_closure_record maps it to a `Reference` attr — the
            // proven struct-capture representation), so the closure BORROWS the outer
            // collection.  The body still needs the ORIGINAL collection type so `h[key]`
            // / iteration type-check; recover it below from `capture_context`.
            // A capture the closure record stores as a SHARED `DbRef` must still read back
            // as the type the source wrote: the shared reference is the sharing mechanism,
            // not the type.  Collections have always been in this set; a nullable heap
            // value joins it, because `S?` is a `DbRef` whose `rec == 0` means absent and
            // it shares exactly as its dense twin does (loft#1114).
            // `.base()`, because this is the READING half of the same fact `closure_attr_type`
            // decides the STORAGE half of, and the two must agree about what a nullable
            // collection is.  Asked unpeeled, a `vector<τ>?` / `hash<τ[k]>?` capture answered
            // "not a collection", so the body took the attribute's own `Reference(elem)` type
            // and `v += [x]` inside the lambda reported *"No matching operator 'Add' on
            // 'integer'"* — the ELEMENT's type, for an append to the collection (loft#1209).
            // @FR-L-CapRef — a `&T` capture is the capture of its POINTEE, which is the
            // reading half of
            // the fact `closure_attr_type` decides the storage half of — so the peel has to
            // happen on both sides or they disagree about what the attribute holds.  Asked
            // unpeeled, a `&vector<τ>` answered "not a collection", the body took the
            // attribute's own `Reference(elem)` type, and `p += [9]` inside the lambda
            // reported *"No matching operator 'Add' on 'integer' and 'integer'"* — the
            // ELEMENT's type, for an append to the collection.  That is loft#1209's shape
            // exactly, reached through `&` instead of through `?` (loft#1276).
            let ctype = match ctype {
                Type::RefVar(inner) => *inner,
                other => other,
            };
            let is_collection_capture = Self::is_collection_type(ctype.base())
                || self.data.nullable_struct_payload(&ctype).is_some();
            // record the capture for closure record synthesis.
            if !self.captured_names.iter().any(|(n, _)| n == name) {
                self.captured_names.push((name.to_string(), ctype.clone()));
            }
            // if we have a closure parameter (second pass), emit field read
            // from the closure record.  Otherwise create a placeholder variable.
            // if we have a closure parameter (second pass), emit field read.
            let closure_d_nr = if self.closure_param == u16::MAX || self.first_pass {
                u32::MAX
            } else {
                self.data.def(self.context).closure_record()
            };
            let fnr = if closure_d_nr == u32::MAX {
                usize::MAX
            } else {
                self.data.attr(closure_d_nr, name)
            };
            if fnr == usize::MAX {
                // First pass, no closure param, or field not found — placeholder variable.
                let v_nr = self.create_var(name, &ctype);
                self.var_usages(v_nr, true);
                t = ctype;
                *code = Value::Var(v_nr);
            } else {
                *code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
                // A collection capture's stored attr is a `Reference` DbRef, but the body
                // must see the original collection type (from capture_context) — the value
                // (a 12-byte DbRef read via OpGetDbRef) is exactly what a collection var is.
                t = if is_collection_capture {
                    ctype
                } else {
                    self.data.attr_type(closure_d_nr, fnr)
                };
                // closure record is a struct — add __closure as dep.
                t = t.depending(self.closure_param);
            }
        } else if qualified && source != u16::MAX && self.data.source_nr(source, &nm) != u32::MAX {
            // loft#1039 — a library-qualified name in VALUE position resolves in the
            // source the qualifier names, exactly as it already does in TYPE position
            // (`f: std::Format = NotExists`) and for a qualified call (`std::abs(-5)`)
            // or constant (`stage::LOOP`).  The branch below asks `def_nr(name)`, and
            // for `std::Format.NotExists` `name` is the QUALIFIER `std` — a library,
            // never a definition — so the value spelling fell through to the
            // unknown-variable arm and reported *"Unknown variable 'std'"*, naming the
            // library while the enum sat one token to its right.  It bites hardest
            // under an alias-only `use lib as a;`, where a qualifier is the whole
            // point and the bare spelling is not imported at all.
            //
            // `parse_constant_value` above already answered for every qualified form
            // that resolves to a VALUE (a constant, a variant, a struct literal); what
            // reaches here is the qualified name of a DEFINITION, and it takes its
            // definition's type — which is what puts `.NotExists` on an enum-typed
            // expression and routes it through the same variant access the bare
            // `Format.NotExists` uses.
            let dnr = self.data.source_nr(source, &nm);
            self.data.def_used(dnr);
            t = self.data.def(dnr).returned().clone();
        } else if self.data.def_nr(name) != u32::MAX
            && !self.at_binding_name()
            // A default-file `<T>` placeholder is invisible here (loft#1049): it is an
            // internal construct, and letting it resolve is what blocked a user `enum T`.
            && !self.stdlib_type_var_placeholder(self.data.def_nr(name))
            && !matches!(
                self.data.def_type(self.data.def_nr(name)),
                // @PLN22 Phase 1 — exclude EnumValue: a bare variant used as a
                // VALUE resolves via the context branches below (or errors with no
                // context), never via this flat key.  Struct-variant CONSTRUCTION
                // and qualified forms are handled earlier in parse_constant_value.
                DefType::Function | DefType::EnumValue
            )
        {
            // @P335: functions are stored mangled as `n_<name>` and are reached
            // ONLY via the `n_`+ident lookup below — never by matching the RAW
            // identifier here.  Without this guard a user variable spelled
            // `n_day` raw-matches function `day` (stored `n_day`) and the
            // declaration mis-parses ("Expect token ;").  Enums / types stored
            // under their plain name still resolve here.
            let dnr = self.data.def_nr(name);
            // loft#788 — this bare name may be one that two imports both bind.
            self.refuse_ambiguous_import(name, dnr);
            if self.data.def_type(dnr) == DefType::Enum {
                t = self.data.def(dnr).returned().clone();
            } else if matches!(self.data.def_type(dnr), DefType::Unknown) {
                // A forward-reference STUB is not "no such name" — it is a type
                // whose declaration has not been reached yet, and in a package it
                // may be in the very file that is suspended at the `use` which
                // pulled this one in. `Null` sent it to the field access as a
                // resolved type of `null`, which reported `Unknown type null —
                // did you mean 'JNull'?`: two names the author never wrote, in
                // PASS 1, aborting before the pass that can resolve it (loft#803).
                //
                // `Unknown(0)` is the deferral the field access already handles
                // quietly, and it keeps the error rather than dropping it: a stub
                // that never gets adopted is still `Unknown` in pass 2, where the
                // same site reports "Field of unknown variable".
                t = Type::Unknown(0);
                // ...but a stub used as a BARE VALUE has no such downstream site, and
                // that was the one consumer with nobody to report it (loft#934).  Every
                // other one does: `Zzz {…}` says "unknown type", `Zzz.x` says "Field of
                // unknown variable", `y: Zzz` and `sizeof(Zzz)` say "Undefined type".
                // A bare `y = Zzz` said NOTHING, and the value it produced was whatever
                // the slot happened to hold — `fn f() -> integer { Zzz }` returned
                // uninitialised memory on `--interpret` and `0` on `--native`, while
                // `if Zzz {…}` silently took the else arm and `--native` handed the user
                // a raw rustc `expected bool, found ()`.
                //
                // Only for a stub pass 1 registered on SPECULATION — a name that merely
                // LOOKED like a type (`objects.rs`'s CamelCase test).  A stub from a
                // written `y: Zzz` annotation is already reported by
                // `resolve_deferred_unknown`, so reporting here too is one typo, two
                // errors.  And only when no `.` follows: that is the field/qualifier
                // form, which has its own report.
                //
                // The `Unknown(0)` above is what the assignment's @P376 poison keys on,
                // so the root error lands and the cascade it used to hide behind
                // ("missing argument for parameter 'v1' of `OpLtInt`" — an internal
                // opcode name reaching the user) stays suppressed.
                if !self.first_pass
                    && self.speculative_type_refs.contains(&dnr)
                    && !self.lexer.peek_token(".")
                {
                    let suggestion = self.suggest_type_name(name).or_else(|| {
                        let candidates: Vec<&str> = (0..self.vars.count())
                            .filter(|&v| self.vars.is_defined(v) && !self.vars.tp(v).is_unknown())
                            .map(|v| self.vars.name(v))
                            .collect();
                        crate::diagnostics::suggest_similar(name, &candidates)
                            .map(std::string::ToString::to_string)
                    });
                    // loft#1008 — the name may be a METHOD, in which case it is not unknown at
                    // all: a `self`/`both` function is registered as `t_<len><Type>_<name>` and
                    // has no `n_<name>` to bind, so naming it where a VALUE is wanted (a fn-ref
                    // argument, `map(v, f)`) reported that the file's own function does not
                    // exist. Say what it is and what to write; the receiver types are listed
                    // because a bare name can be a method on several.
                    let receivers = self.method_receivers_named(name);
                    if !receivers.is_empty() {
                        let on = receivers.join("`, `");
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            "`{name}` is a method on `{on}`, and a method is not a function \
                             VALUE — there is nothing to bind here. Wrap it: `|x| {{ x.{name}(…) \
                             }}`, or declare the function with a plain first-parameter name \
                             (not `self` / `both`), which makes it a free function and a usable \
                             fn-ref"
                        );
                    } else if let Some(s) = suggestion {
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            code = "unknown-variable",
                            "Unknown variable '{name}' — did you mean '{s}'?"
                        );
                    } else {
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            "Unknown variable '{name}'"
                        );
                    }
                    // Now that the root error is reported, poison the type so nothing
                    // downstream re-reports it — the same `Never` @P376 uses on an
                    // errored assignment RHS.  `Unknown(0)` is what the arity check
                    // reads as "no argument supplied", which is where
                    // "missing argument for parameter 'v1' of `OpLtInt`" came from.
                    t = Type::Never;
                }
            } else {
                // loft#1008 — the OTHER half. A `both` receiver registers a dispatch entry
                // under the PLAIN name (which is what makes the free-call spelling `f(x)`
                // work), so unlike a `self` method the bare name is FOUND here — as a
                // `Dynamic` def — and fell through to a silent null. `x = f` bound null with
                // no diagnostic at all, and the error surfaced later as whatever used it
                // ("Cannot format type null"); in a fn-ref argument it reached the call check
                // as a bare `Value::Null` with no name attached, reported as *"expected
                // fn(P) -> integer, got null"* — a value the author wrote nowhere.
                //
                // The name IS available here, which is what the argument site lacked. Same
                // message as the `self` spelling gets, so the two receivers finally answer
                // alike. Pass 2 only, and only when the name really is a method — every other
                // def kind that lands here keeps the null it always produced.
                let mut reported_method = false;
                if !self.first_pass {
                    let receivers = self.method_receivers_named(name);
                    if !receivers.is_empty() {
                        reported_method = true;
                        let on = receivers.join("`, `");
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            "`{name}` is a method on `{on}`, and a method is not a function \
                             VALUE — there is nothing to bind here. Wrap it: `|x| {{ x.{name}(…) \
                             }}`, or declare the function with a plain first-parameter name \
                             (not `self` / `both`), which makes it a free function and a usable \
                             fn-ref"
                        );
                    }
                }
                // Poison once the root error is out, exactly as the `self` path does: a
                // `Null` here is a real value downstream, so the call check reported the
                // generic *"got null"* on top and the arity check then counted the argument
                // as missing — three errors for one mistake. `Never` is what both of those
                // read as "already reported".
                t = if reported_method {
                    Type::Never
                } else {
                    Type::Null
                };
            }
        } else if matches!(self.data.def_type(self.context), DefType::Struct)
            && self.data.attr(self.context, name) != usize::MAX
        {
            let fnr = self.data.attr(self.context, name);
            *code = self.get_field(self.context, fnr, Value::Var(0));
            t = self.data.attr_type(self.context, fnr);
        // @PLN22 Phase 1 — the bare variant VALUE resolves against the expected
        // enum from context: `Type::Enum` (match subject, `==` LHS) or
        // `Type::Reference(enum)` (typed decl / reassignment / field init / call
        // arg / return).  emit_variant_value picks the right discriminant (and
        // the mixed-enum allocation form) for that enum.
        // Read through `base()`: whether the target may be ABSENT says nothing about
        // which variants it can hold, so `v: vector<Color?> = [Green]` has to resolve
        // `Green` exactly as the dense spelling beside it does.  Asked bare, a nullable
        // element type fell past both arms and the bare variant was reported as having no
        // type at all — the same peel `enum_context` already does to decide there IS an
        // enum context here (loft#1065 one site over, loft#1416).
        } else if let Type::Enum(enr, _, _) = parent_tp.base()
            && self.data.def(*enr).attr_names.contains_key(name)
        {
            let enr = *enr;
            t = self.emit_variant_value(enr, name, code);
        } else if let Type::Reference(enr, _) = parent_tp.base()
            && self.data.def_type(*enr) == DefType::Enum
            && self.data.def(*enr).attr_names.contains_key(name)
        {
            let enr = *enr;
            t = self.emit_variant_value(enr, name, code);
        } else {
            // try resolving as a bare function reference.
            // On the first pass, only do this when the identifier is NOT followed
            // by '=' (assignment position), so that `double = 5` still creates a
            // local variable that shadows the function name.
            let fn_d_nr = {
                let prefixed = format!("n_{nm}");
                let nr = self.data.def_nr(&prefixed);
                if nr == u32::MAX {
                    // @P335: the RAW fallback resolves names stored un-prefixed,
                    // but must NOT match a FUNCTION — otherwise a user identifier
                    // spelled `n_<x>` aliases function `<x>` (stored `n_<x>`).
                    // Functions are only reachable via the `n_`+ident form above.
                    let raw = self.data.def_nr(&nm);
                    if raw != u32::MAX && !matches!(self.data.def_type(raw), DefType::Function) {
                        raw
                    } else {
                        u32::MAX
                    }
                } else {
                    nr
                }
            };
            // A name at a BINDING position is the author naming a local, not a
            // reference to a function that happens to share the spelling — so it
            // binds, and the function stays reachable as a call.
            //
            // loft keeps values and functions in SEPARATE namespaces, which the
            // other binding forms already rely on: a parameter, a `for` variable
            // and a struct field may all be called `chr` while `chr(65)` in the
            // same scope still reaches the stdlib function.  Three forms route
            // through here and must agree with them — `<name> = …`, the typed
            // local `<name>: T = …` (the lexer is parked on `:`), and a
            // tuple-destructuring element (spelled `,` / `)`).
            //
            // Refusing them instead made every public function a library adds a
            // breaking change for any consumer already using that word as a local
            // — a cost the consumer cannot see coming and cannot prepare for
            // (loft#852; loft#756 is the same collision against the stdlib).
            // The refusal was never a namespace rule, only this path: `for turn`
            // and `fn go(turn: integer)` accepted the name throughout.
            //
            // The typed local is what @P392 added: without it the form produced a
            // function-ref `Value::Int`, back in parse_assign the `if let
            // Value::Var(_) = code` arm did not match, the `:` was never consumed,
            // and the user saw a confusing `Expect token ;`.  Binding yields the
            // `Value::Var` that arm needs, so the form parses.  It lived here as a
            // local `peek_token(":")` beside `at_binding_name` until loft#1079,
            // where the flat-`def_nr` site ABOVE — which has only the one
            // predicate — returned first for a `both:` function and reproduced the
            // exact `Expect token ;` @P392 had cured here.  Now all three forms
            // come from `at_binding_name`, so the sites cannot disagree again.
            if fn_d_nr != u32::MAX
                && matches!(self.data.def_type(fn_d_nr), DefType::Function)
                && !self.at_binding_name()
            {
                *code = Value::Int(fn_d_nr as i32);
                self.data.def_used(fn_d_nr);
                self.record_sandbox_fn_ref(fn_d_nr); // @PLN86 L4
                let arg_types = self.fn_ref_arg_types(fn_d_nr);
                let ret_type = self.data.def(fn_d_nr).returned().clone();
                t = Type::Function(arg_types, Box::new(ret_type), crate::data::Deps::none());
            } else {
                // @PLN22 Phase 1 — a bare name that is a VARIANT of some enum,
                // used with no type context, is the "needs qualification" error.
                // Emit a targeted diagnostic, then RECOVER by resolving it against
                // its enum (the first when the name is shared) so the rest of the
                // function parses cleanly — without this recovery a placeholder var
                // would shadow every later variant reference on the second pass and
                // bury the real error under a cascade of "Unknown variable".
                let variant_enums = self.data.enums_with_variant(name);
                if let Some(&e_nr) = variant_enums.first() {
                    // Emit unconditionally (not pass-2-gated): the recovery below
                    // types the target from the variant's enum, so on the SECOND
                    // pass the target has context and this branch is not re-reached
                    // — emitting only on pass 2 would never fire.  Diagnostics
                    // dedupe by position, so a both-pass emit still shows once.
                    let enum_name = self.data.def(e_nr).name().to_string();
                    if variant_enums.len() == 1 {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "bare variant '{name}' has no type here — qualify it as \
                             '{enum_name}.{name}', or give the target an enum type"
                        );
                    } else {
                        let names: Vec<String> = variant_enums
                            .iter()
                            .map(|&e| self.data.def(e).name().to_string())
                            .collect();
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "ambiguous variant '{name}' (a variant of {}) — qualify it, \
                             e.g. '{enum_name}.{name}'",
                            names.join(", ")
                        );
                    }
                    t = self.emit_variant_value(e_nr, name, code);
                } else if !self.first_pass {
                    if name == "_" {
                        // loft#795 — `_` DISCARDS its value (each `_ = …` gets its own
                        // slot), so there is nothing here to read back.  Say that rather
                        // than "Unknown variable '_'", which reads as a typo in the one
                        // case where the name is deliberate.
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "`_` discards the value assigned to it — there is nothing to \
                             read back; give the value a name if you need it"
                        );
                    } else {
                        // loft#1008 — a METHOD is registered as `t_<len><Type>_<name>`, so its
                        // bare name has no definition to bind and reads as unknown wherever a
                        // VALUE is wanted (a fn-ref argument, `map(v, f)`). Naming what it is
                        // beats reporting that the file's own function does not exist.
                        let receivers = self.method_receivers_named(name);
                        if receivers.is_empty() {
                            diagnostic!(self.lexer, Level::Error, "Unknown variable '{}'", name);
                        } else {
                            let on = receivers.join("`, `");
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "`{name}` is a method on `{on}`, and a method is not a function \
                                 VALUE — there is nothing to bind here. Wrap it: \
                                 `|x| {{ x.{name}(…) }}`, or declare the function with a plain \
                                 first-parameter name (not `self` / `both`), which makes it a \
                                 free function and a usable fn-ref"
                            );
                        }
                    }
                    t = Type::Unknown(0);
                } else {
                    // Pass 1, and the name resolves to nothing yet.  A CamelCase one can
                    // only be a TYPE here: loft gives variables and functions `lower_case`
                    // and constants `UPPER_CASE`, and a name that is a known enum variant
                    // was resolved by the branch above.  So leave the same
                    // forward-reference stub a written type annotation leaves, and a
                    // declaration parsed later — in this file or in the module that
                    // `use`d it — adopts it in place.
                    //
                    // Without it `Colour.Green` and `sizeof(Roofs)` had nothing to resolve
                    // through: the placeholder variable below is all pass 1 left behind,
                    // and a variable is not a type, so pass 2 reported the name unknown
                    // even where the declaration was two lines down (loft#801).
                    //
                    // No placeholder VARIABLE for such a name, which is the other half of
                    // the same fault.  A function's variable table survives into pass 2
                    // (`data.reset()` keeps definitions), so a pass-1 placeholder named
                    // `Roofs` was still there when pass 2 looked the name up, and it
                    // shadowed the type the declaration had meanwhile produced — the stub
                    // resolved and the name still read as an unknown variable.  Deferring
                    // as `Unknown(0)` with no slot is what the sibling `Name { … }` branch
                    // below already does, for the same reason.
                    //
                    // A `.` may follow.  `Colour.Green` is a type qualifying a VALUE, and
                    // this branch used to refuse it because the stub alone made the
                    // program compile and evaluate to `unknown` for every variant — a
                    // wrong answer where there had been an error.  That `unknown` was not
                    // this site's doing: an ADOPTED enum def was never registered, so it
                    // had no db type and `enum_val` answers `unknown` for exactly that
                    // (loft#803, fixed in `typedef.rs`).  With the discriminants
                    // established the stub yields the variant the author wrote, so the
                    // qualifier resolves here like any other forward reference.
                    //
                    // The name test is NOT `is_camel`, which answers "not lower_case and
                    // no underscore" and so accepts `FOO`, `N` and `X` — the UPPER_CASE
                    // constant style.  Treating those as types took the placeholder
                    // variable away from every misspelled constant, which is what the
                    // `upper-case-local` advice and "Unknown variable 'N'" are written
                    // against.  A type name carries a lowercase letter.
                    //
                    // A QUALIFIER settles it without the spelling test.  `D.N` is a name
                    // qualifying a value, which a misspelled constant never is — the
                    // `upper-case-local` case this guard protects is a BARE `N`.  So an
                    // uppercase name followed by `.` takes the type-stub path too, which is
                    // what lets a one-letter `enum D` declared BELOW its use resolve: the
                    // spelling test rejects `D` (no lowercase letter), the `else` branch
                    // leaves a placeholder VARIABLE, and — per the note above — that
                    // placeholder survives into pass 2 and shadows the type the declaration
                    // meanwhile produced, so the name still read as an unknown variable
                    // (loft#1047).  Same fault as loft#801, reached by a different spelling.
                    let looks_like_a_type = (name.starts_with(char::is_uppercase)
                        && name.contains(char::is_lowercase)
                        && !name.contains('_'))
                        || (name.starts_with(char::is_uppercase) && self.lexer.peek_token("."));
                    if looks_like_a_type {
                        // A hidden default-file `<T>` placeholder does not count as "the
                        // name is taken" — without this the forward-reference stub is never
                        // registered and pass 2 has nothing to adopt (loft#1049).
                        let taken = self.data.def_nr(name);
                        if taken == u32::MAX || self.stdlib_type_var_placeholder(taken) {
                            self.speculative_type_refs.insert(self.data.add_def(
                                name,
                                name_pos,
                                DefType::Unknown,
                            ));
                        }
                        t = Type::Unknown(0);
                    } else {
                        // Nothing in the table answers to this name.  In pass 2 that is a
                        // typo and the diagnostics downstream say so; in pass 1 it is
                        // usually a forward reference, and the caller parsing a stretch of
                        // source needs to know one happened — see
                        // `Parser::unresolved_names`.
                        self.unresolved_names = self.unresolved_names.saturating_add(1);
                        *code = Value::Var(self.create_var(name, &Type::Unknown(0)));
                        t = Type::Unknown(0);
                    }
                }
            }
        }
        // Plan-22 phase 02d-iii.b — read auto-deref for boxed
        // scalars.  When `t` is `Reference(__cell_<T>, _)`
        // (set by phase 02d-iii.a's flip helper for mutated-
        // scalar-capture locals), wrap the resolved IR in
        // `Call(OpGet<T>, [code, Int(0)])` so reads load the
        // cell's `value` field instead of yielding the bare
        // DbRef.  Dormant in production until 02d-iii.e
        // activates the flip — no production variable has the
        // trigger type today, so the hook is a no-op for every
        // read.
        self.auto_deref_boxed_scalar(code, t)
    }

    /// Plan-22 phase 02d-iii.b — wrap a resolved variable read
    /// in an `OpGet<T>(code, 0)` call when its type is
    /// `Reference(__cell_<T>, _)` (a boxed scalar created by
    /// phase 02d-iii.a's flip).
    ///
    /// Returns the underlying scalar type (the cell's `value`
    /// field type).  No-op for any other input type.
    ///
    /// Mirrors `wrap_vector_get_val` in `parser/mod.rs:2409` —
    /// the same opcode-per-type table, the same boolean
    /// `OpGetByte` + `OpEqInt` shape.
    ///
    /// Limitations (acceptable for 02d-iii.b foundation):
    /// - Doesn't fire from `parse_var`'s early-return paths
    ///   (Reference-aliasing arm at lines 141-154, the special
    ///   `$` ref at line 18, etc.).  Those paths handle other
    ///   concerns and aren't on the critical path for
    ///   boxed-scalar reads.
    /// - Cells whose `value` field type isn't in the supported
    ///   primitive set (Integer / Float / Single / Boolean /
    ///   Character / Text / plain Enum) fall through with the
    ///   bare DbRef — phase 02d-ii's silent gap for exotic
    ///   shapes carries through here.
    pub(crate) fn auto_deref_boxed_scalar(&mut self, code: &mut Value, t: Type) -> Type {
        let Some(d_nr) = crate::parser::vectors::boxed_cell_def(&t, &self.data) else {
            return t;
        };
        let Some(value_attr) = self.data.def(d_nr).attributes().first() else {
            return t;
        };
        if value_attr.name != "value" {
            return t;
        }
        let value_tp = value_attr.typedef.clone();
        let pos = Value::Int(0);
        // `.base()`: a nullable cell's `value` is stored in-band (C90), so the READ op is its
        // dense twin's.  `value_tp` itself is returned unpeeled, so the expression keeps the
        // `?` and the ordinary discharge rules apply to it (loft#1408).
        let (op_name, is_bool) = match value_tp.base() {
            Type::Integer(_) => ("OpGetInt", false),
            Type::Float => ("OpGetFloat", false),
            Type::Single => ("OpGetSingle", false),
            Type::Text(_) => ("OpGetText", false),
            Type::Character => ("OpGetCharacter", false),
            Type::Enum(_, false, _) => ("OpGetEnum", false),
            // @PLN17: byte-stored boolean read, preserving 0/1/255 (like enum).
            Type::Boolean => ("OpGetBoolean", false),
            _ => return t,
        };
        let op_d_nr = self.data.def_nr(op_name);
        if op_d_nr == u32::MAX {
            return t;
        }
        if is_bool {
            let byte_call = Value::Call(op_d_nr, vec![code.clone(), pos, Value::Int(0)]);
            let eq_d_nr = self.data.def_nr("OpEqInt");
            if eq_d_nr == u32::MAX {
                *code = byte_call;
            } else {
                *code = Value::Call(eq_d_nr, vec![byte_call, Value::Int(1)]);
            }
        } else {
            *code = Value::Call(op_d_nr, vec![code.clone(), pos]);
        }
        value_tp
    }

    pub(crate) fn is_file_var(&self, var_nr: u16) -> bool {
        self.is_file_var_type(self.vars.tp(var_nr))
    }

    pub(crate) fn file_op(&mut self, code: &mut Value, t: &mut Type, var_nr: u16) {
        self.vars.in_use(var_nr, true);
        if self.lexer.has_keyword("format") {
            let file_ref = Value::Var(var_nr);
            *code = self.cl("OpGetEnum", &[file_ref, Value::Int(32)]);
            let fmt_def = self.data.def_nr("Format");
            *t = Type::Enum(fmt_def, false, crate::data::Deps::none());
        } else if self.lexer.has_keyword("exists") {
            let file_ref = Value::Var(var_nr);
            let fmt = self.cl("OpGetEnum", &[file_ref, Value::Int(32)]);
            let fmt_def = self.data.def_nr("Format");
            let enum_tp = Type::Enum(fmt_def, false, crate::data::Deps::none());
            let ne_val = if let Some(&a_nr) = self.data.def(fmt_def).attr_names.get("NotExists") {
                self.data.attr_value(fmt_def, a_nr)
            } else {
                diagnostic!(self.lexer, Level::Error, "Format.NotExists not found");
                Value::Null
            };
            self.call_op(code, "!=", &[fmt, ne_val], &[enum_tp.clone(), enum_tp]);
            *t = Type::Boolean;
        } else if self.lexer.has_keyword("size") {
            *code = self.cl("OpSizeFile", &[Value::Var(var_nr)]);
            *t = crate::data::I64.clone();
        } else if self.lexer.has_keyword("index") {
            // Read the current field at offset 8 (pre-2c layout restored now that i32 is 4B)
            *code = self.cl("OpGetInt", &[Value::Var(var_nr), Value::Int(8)]);
            *t = crate::data::I64.clone();
        } else if self.lexer.has_keyword("next") {
            // Read the next field at offset 16 (pre-2c layout restored)
            *code = self.cl("OpGetInt", &[Value::Var(var_nr), Value::Int(16)]);
            *t = crate::data::I64.clone();
        } else if self.lexer.has_keyword("read") {
            // Size argument is optional: `f#read(n) as T` reads n bytes,
            // `f#read as T` derives n from T's fixed byte width.  Bare
            // `f#read as text` is rejected — text has no fixed width and
            // requires the explicit `(n)`.
            let has_explicit_size = self.lexer.has_token("(");
            let mut n_code = Value::Null;
            if has_explicit_size {
                self.expression(&mut n_code);
                self.lexer.token(")");
            }
            // Determine read type from optional "as T", remembering the
            // type's natural byte width so the size-less form can infer.
            // If no `as T` is given but the surrounding assignment has a
            // known destination type (`s.field = f#read`), use THAT — the
            // destination field's declared type drives the byte width,
            // symmetric with how `f += s.field` already takes its width
            // from the field's type.
            let has_cast = self.lexer.has_token("as");
            let target_hint = if !has_cast && !has_explicit_size {
                let hint = self.read_target_type();
                if matches!(hint, Type::Integer(_) | Type::Float | Type::Single) {
                    Some(hint)
                } else {
                    None
                }
            } else {
                None
            };
            let (read_type, db_tp, inferred_size) = if has_cast {
                if let Some(type_name) = self.lexer.has_identifier() {
                    // Capture the alias def_nr so size(N) can pick Parts::Int.
                    let alias_nr = self.data.def_nr(&type_name);
                    let tp = self
                        .parse_type(u32::MAX, &type_name, false)
                        .unwrap_or(Type::Text(crate::data::Deps::none()));
                    if let Type::Reference(d_nr, _) = &tp
                        && let Some(field) = Self::first_unserialisable_field(*d_nr, &self.data)
                    {
                        let tname = self.data.def(*d_nr).name().to_string();
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "read_file: '{}' has variable-width field '{}' (text/vector/collection) that binary I/O cannot round-trip; serialise a plain fixed-width struct",
                            tname,
                            field
                        );
                    }
                    self.ensure_io_type(&tp.clone());
                    // Post-2c: honor `as i32` by routing to Parts::Int (4B) when
                    // the alias has size(4).
                    let forced = self.data.forced_size(alias_nr);
                    let id = if let Type::Integer(IntegerSpec { min, .. }) = &tp
                        && forced == Some(4)
                    {
                        if self.first_pass {
                            u16::MAX
                        } else {
                            self.database.int(*min, false)
                        }
                    } else if let Type::Integer(IntegerSpec { min, .. }) = &tp
                        && forced == Some(1)
                    {
                        if self.first_pass {
                            u16::MAX
                        } else {
                            self.database.byte(*min, false)
                        }
                    } else if let Type::Integer(IntegerSpec { min, .. }) = &tp
                        && forced == Some(2)
                    {
                        if self.first_pass {
                            u16::MAX
                        } else {
                            self.database.short(*min, false)
                        }
                    } else {
                        self.get_type(&tp)
                    };
                    // Byte width inferred from the cast — reuses the
                    // three-tier resolution that `sizeof(T)` already
                    // uses (parse_size in control.rs): forced_size of
                    // the alias > packed size of a range-constrained
                    // integer > database-allocated size of the base
                    // type.  Variable-width types (text, struct refs
                    // with collection fields) fall back to None and
                    // still require explicit `(n)`.
                    let nat_size = if self.first_pass {
                        Some(0_i64)
                    } else if let Some(n) = forced {
                        Some(i64::from(n))
                    } else {
                        let packed = tp.size(false);
                        if packed > 0 {
                            Some(i64::from(packed))
                        } else if matches!(tp, Type::Text(_)) {
                            None
                        } else {
                            let db_sz = self
                                .database
                                .size(self.data.def(self.data.type_elm(&tp)).known_type());
                            if db_sz == 0 {
                                None
                            } else {
                                Some(i64::from(db_sz))
                            }
                        }
                    };
                    (tp, id, nat_size)
                } else {
                    let text_tp = Type::Text(crate::data::Deps::none());
                    let id = self.get_type(&text_tp);
                    (text_tp, id, None)
                }
            } else if let Some(hint) = target_hint {
                // No `as T` — use the assignment-LHS type as the cast.
                // `IntegerSpec` carries TWO pieces of width info:
                // (a) `forced_size` (set by `pub type i32 = …size(4)`
                // typedefs — fixed-width regardless of range), and
                // (b) the implicit packed size derived from `min`/`max`
                // (`size(false)` returns 1/2/8 based on range).  The
                // forced size wins: an `i32` field has range covering
                // all 32-bit values which packs to 8, but its forced
                // size is 4.  Falls back to packed when no forced, and
                // to the database-allocated size otherwise.
                let forced_width: Option<u8> = if let Type::Integer(spec) = &hint {
                    spec.forced_size.map(std::num::NonZero::get)
                } else {
                    None
                };
                let nat = if let Some(n) = forced_width {
                    Some(i64::from(n))
                } else {
                    let packed = hint.size(false);
                    if packed > 0 {
                        Some(i64::from(packed))
                    } else {
                        let db_sz = self
                            .database
                            .size(self.data.def(self.data.type_elm(&hint)).known_type());
                        if db_sz == 0 {
                            None
                        } else {
                            Some(i64::from(db_sz))
                        }
                    }
                };
                let id = if self.first_pass {
                    u16::MAX
                } else if let Type::Integer(IntegerSpec { min, .. }) = &hint
                    && forced_width == Some(4)
                {
                    self.database.int(*min, false)
                } else if let Type::Integer(IntegerSpec { min, .. }) = &hint
                    && (forced_width == Some(1) || hint.size(false) == 1)
                {
                    self.database.byte(*min, false)
                } else if let Type::Integer(IntegerSpec { min, .. }) = &hint
                    && (forced_width == Some(2) || hint.size(false) == 2)
                {
                    self.database.short(*min, false)
                } else {
                    self.get_type(&hint)
                };
                (hint, id, nat)
            } else {
                let text_tp = Type::Text(crate::data::Deps::none());
                let id = self.get_type(&text_tp);
                (text_tp, id, None)
            };
            // W3/W4 (@PLN47): `get_type` folds `character` to `integer` (an
            // 8-byte read that mismatched the 4-byte native path and crashed
            // the interp read) and has no `boolean` arm at all (`u16::MAX` →
            // an out-of-bounds type index).  Both are fixed-width base types;
            // route them to their own db type so interp and native agree.
            let db_tp = self.io_scalar_db_tp(&read_type).unwrap_or(db_tp);
            let db_tp = self.checked_io_db_tp(db_tp, &read_type, "f#read");
            // Resolve the final size expression: explicit `(n)` wins;
            // otherwise inferred from the cast type.  Bare `f#read`
            // with no `as T` or with a variable-width cast (`as text`)
            // is a parse error.
            if !has_explicit_size {
                if let Some(n) = inferred_size {
                    n_code = Value::Int(n as i32);
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "f#read without (size) requires a fixed-width 'as <type>' cast (i32, u8, u16, integer, single, float, ...)"
                    );
                    n_code = Value::Int(0);
                }
            }
            // W8 (@PLN47): `f#read(N) as vector<T>` reads N *bytes* (the shipped
            // convention — callers pass the byte length: `f#read(24)` for three
            // 8-byte elements, GLB-style).  A literal N that is not a whole
            // multiple of the element width silently yields a truncated vector
            // (`f#read(3)` of 8-byte integers → 0 elements) — the silent-data-
            // loss class this plan targets — so warn at the misuse site.
            if !self.first_pass
                && matches!(read_type, Type::Vector(_, _))
                && let Value::Int(n) = n_code
            {
                let elem = i32::from(self.database.size(self.database.content(db_tp)));
                if elem > 1 && n % elem != 0 {
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        code = "read-size-not-element-multiple",
                        "f#read({n}) as vector<T> counts BYTES, and {n} is not a multiple of \
                         the {elem}-byte element width; this drops the trailing {} byte(s) \
                         and reads {} element(s)",
                        n % elem,
                        n / elem
                    );
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Mechanical,
                        title: format!("pass the byte length — `element_count * {elem}`"),
                        condition: None,
                        edit: None,
                        concept: "file I/O",
                        concept_ref: "@F40",
                    });
                }
            }
            // loft#899 — a `vector<T>` read builds its result in a temp DECLARED
            // here, from the `as` cast type, with no assignment behind it.  Every
            // other vector local gets its `main_vector<T>` wrapper registered by
            // the assignment path (`Parser::change_var_type`); this temp reaches
            // none of them, so without this the wrapper does not exist and
            // `gen_set_first_vector_null` emits `OpDatabase(db_tp = u16::MAX)`.
            // The store is then created with no type at all, so its header is the
            // wrong width: `f#read(8) as vector<single>` answered length 1 and
            // yielded the SECOND element alone.  It only ever surfaced when
            // nothing ELSE in the file declared a `vector<T>` local, which is what
            // made an unrelated declaration elsewhere change what a read returns.
            if let Type::Vector(elm, _) = &read_type {
                let elm = (**elm).clone();
                self.data.vector_def(&mut self.lexer, &elm);
            }
            let mut ls = Vec::new();
            let temp_var = if let Type::Text(_) = read_type {
                self.vars.work_text(&mut self.lexer)
            } else if !self.first_pass && matches!(read_type, Type::Reference(_, _)) {
                // W9 (@PLN47): a struct read yields an OWNED heap record.  Its
                // slot must hold a real record (OpDatabase) so OpReadFile's
                // per-field write lands in storage, not the null-sentinel slot
                // (store u16::MAX → panic).  The block's value (`t`) is
                // `PutRef`-aliased into the assignment LHS (which is empty-dep,
                // i.e. it ADOPTS the store).
                let t = self.vars.unique("read", &read_type, &mut self.lexer);
                ls.push(v_set(t, Value::Null));
                ls.push(self.cl("OpDatabase", &[Value::Var(t), Value::Int(i32::from(db_tp))]));
                t
            } else {
                let t = self.vars.unique("read", &read_type, &mut self.lexer);
                ls.push(v_set(t, self.null(&read_type)));
                t
            };
            let var_ref = self.cl("OpCreateStack", &[Value::Var(temp_var)]);
            ls.push(self.cl(
                "OpReadFile",
                &[
                    Value::Var(var_nr),
                    var_ref,
                    n_code,
                    Value::Int(i32::from(db_tp)),
                ],
            ));
            ls.push(Value::Var(temp_var));
            *code = v_block(ls, read_type.clone(), "reading file");
            *t = read_type;
        } else {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown # operation on File");
            }
            *t = Type::Unknown(0);
        }
    }

    /// loft#753 — is `tp` a `File` handle, INCLUDING one reached through a `&`
    /// parameter?  A `&File` is `RefVar(Reference(File))`, and codegen already
    /// dereferences such a slot (`OpVarRef` + `OpGetStackRef`), so every File
    /// operation works through it once this says yes.  While it said no, the
    /// three sites that ask — `+=`, the `#` attribute surface, and field
    /// assignment — each fell through to the GENERIC code and reported what
    /// that code saw: `f#read` became "Unknown loop attribute '#f'", `f += v`
    /// became "No matching operator 'Add' on '&File' and '&File'".  `File` was
    /// the only type whose `&` form was special, so the wall read as "I wrote
    /// it wrong" and was walked around rather than reported.
    ///
    /// The peel lives here, in the ONE predicate both callers share, rather
    /// than at any single question site — the shape of loft#740, where two
    /// guards decided one question and only one of them peeled.
    pub(crate) fn is_file_var_type(&self, tp: &Type) -> bool {
        let file_def = self.data.def_nr("File");
        let mut tp = tp.base();
        while let Type::RefVar(inner) = tp {
            tp = inner.base();
        }
        matches!(tp, Type::Reference(d, _) if *d == file_def)
    }

    /// Ensure byte/short integer types used in file I/O are registered in the database.
    pub(crate) fn ensure_io_type(&mut self, t: &Type) {
        match t {
            Type::Integer(IntegerSpec { min, .. }) => match t.size(false) {
                1 => {
                    self.database.byte(*min, false);
                }
                2 => {
                    self.database.short(*min, false);
                }
                _ => {}
            },
            Type::Vector(tp, _) => {
                let tp = tp.clone();
                self.ensure_io_type(&tp);
            }
            _ => {}
        }
    }

    /// Return the name of the first field in `d_nr` that binary file I/O cannot
    /// serialise, or `None` when every field is fixed-width.
    ///
    /// `f += s` / `f#read as S` walk a struct field-by-field with a FIXED byte
    /// width per field (no length prefix), so any variable-width field breaks the
    /// round-trip: a `text`/`vector` field writes payload bytes the fixed-width
    /// read can't locate (W10 — it read past the record and panicked), and the
    /// collection kinds (sorted/index/hash/spatial) hold store-internal pointers
    /// that don't serialise at all.  Nested plain structs ARE fixed-width and
    /// round-trip (W11), so recurse into them — reporting the offending field as
    /// `outer.inner`.  Callers emit a compile-time error when this returns `Some`.
    fn first_unserialisable_field(d_nr: u32, data: &super::Data) -> Option<String> {
        Self::first_unserialisable_field_rec(d_nr, data, &mut Vec::new())
    }

    fn first_unserialisable_field_rec(
        d_nr: u32,
        data: &super::Data,
        seen: &mut Vec<u32>,
    ) -> Option<String> {
        // Guard against a (pointer-mediated) type cycle so recursion terminates.
        if seen.contains(&d_nr) {
            return None;
        }
        seen.push(d_nr);
        for a in data.def(d_nr).attributes() {
            match &a.typedef {
                Type::Sorted(..)
                | Type::Index(..)
                | Type::Hash(..)
                | Type::Radix(..)
                | Type::Trie(..)
                | Type::Text(_)
                | Type::Vector(..) => return Some(a.name.clone()),
                Type::Reference(inner, _) => {
                    if let Some(f) = Self::first_unserialisable_field_rec(*inner, data, seen) {
                        return Some(format!("{}.{}", a.name, f));
                    }
                }
                _ => {}
            }
        }
        seen.pop();
        None
    }

    /// Binary db type for a scalar file operand whose `get_type` mapping is
    /// wrong for I/O: `character` (folded to 8-byte `integer`) and `boolean`
    /// (no `get_type` arm → `u16::MAX`).  Returns their fixed-width base type
    /// (`character` = 4 bytes, `boolean` = 1 byte); `None` for every other type,
    /// which keeps its existing `get_type` routing.  Yields `None` in the first
    /// pass, where db types are not yet registered.
    fn io_scalar_db_tp(&self, t: &Type) -> Option<u16> {
        if self.first_pass {
            return None;
        }
        match t.base() {
            Type::Character => Some(self.database.name("character")),
            Type::Boolean => Some(self.database.name("boolean")),
            _ => None,
        }
    }

    /// The db type a file read/write is about to bake into its op, checked.
    ///
    /// `u16::MAX` is `get_type`'s "I have no arm for this type" answer, and the
    /// I/O ops index `Stores::types` with what they are given: on the
    /// interpreter that is `index out of bounds: the len is N but the index is
    /// 65535`, and on `--native` an unsatisfied `FileVal` bound reported against
    /// generated Rust the author never wrote (loft#708).  Both said the wrong
    /// thing about a program whose only fault is naming a type binary I/O
    /// cannot serialise — `f += (1, 2)`.
    ///
    /// Refusing here, where the constant is minted, covers every such type at
    /// once: `boolean`, `character` and `vector<T>` each arrived as their own
    /// crash and were fixed one at a time, and the next `Type` variant would
    /// have arrived the same way.  The substituted type keeps the IR walkable
    /// for the rest of the parse; the error has already failed the compile.
    fn checked_io_db_tp(&mut self, db_tp: u16, tp: &Type, op: &str) -> u16 {
        if self.first_pass || db_tp != u16::MAX {
            return db_tp;
        }
        let name = tp.name(&self.data);
        diagnostic!(
            self.lexer,
            Level::Error,
            "{op}: '{name}' has no byte layout, so it cannot be written to or read \
             from a file; use a number, `character`, `boolean`, `text`, a vector \
             of those, or a struct whose fields are all fixed-width",
        );
        self.database.name("integer")
    }

    pub(crate) fn write_to_file(
        &mut self,
        file_var: u16,
        val: Value,
        val_type: &Type,
        cast_alias: u32,
    ) -> Value {
        if let Type::Reference(d_nr, _) = val_type
            && let Some(field) = Self::first_unserialisable_field(*d_nr, &self.data)
        {
            let type_name = self.data.def(*d_nr).name().to_string();
            diagnostic!(
                self.lexer,
                Level::Error,
                "write_file: '{}' has variable-width field '{}' (text/vector/collection) that binary I/O cannot round-trip; serialise a plain fixed-width struct",
                type_name,
                field
            );
            return Value::Null;
        }
        let val_type_clone = val_type.clone();
        self.ensure_io_type(&val_type_clone);
        // Post-2c: if the value was written as `… as <alias>` and the alias
        // has size(N), narrow the serialisation to the alias's db type.
        let db_tp = if let Type::Integer(IntegerSpec { min, .. }) = val_type
            && let Some(n) = self.data.forced_size(cast_alias)
        {
            if self.first_pass {
                u16::MAX
            } else {
                match n {
                    1 => self.database.byte(*min, false),
                    2 => self.database.short(*min, false),
                    4 => self.database.int(*min, false),
                    _ => self.get_type(val_type),
                }
            }
        } else {
            // Post-2c lint: a bare `f += <integer>` writes 8 bytes.  For
            // binary file formats (BigEndian / LittleEndian) that silently
            // breaks record alignment with older specs that assumed i32;
            // for text files 8 bytes of decimal is fine.  We can't tell
            // the file format at parse time, so warn generically — the
            // user silences the lint by writing `f += x as i32` (or the
            // correct byte-width alias).  Skip on the stdlib (`!self.default`)
            // and on explicit `as integer` (full 8-byte) where `cast_alias`
            // is the integer base — those are intentional wide writes.
            if !self.first_pass
                && !self.default
                && matches!(val_type, Type::Integer(_))
                && cast_alias == u32::MAX
            {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    code = "file-write-width",
                    "`f += <integer>` without a width cast writes 8 bytes, and a binary \
                     file (BigEndian / LittleEndian) usually wants an exact width"
                );
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: "cast to the width you mean (`as i32`, `as u8`, …)".to_string(),
                    condition: Some(
                        "8 bytes is not the record width the reader expects — `as integer` \
                         says the 8-byte write is deliberate"
                            .to_string(),
                    ),
                    edit: None,
                    concept: "file I/O",
                    concept_ref: "@F40",
                });
            }
            self.get_type(val_type)
        };
        // W3/W4 (@PLN47): route `character`/`boolean` to their fixed-width base
        // type (see `f#read`'s matching override) so the write width matches the
        // read and both backends agree.
        let db_tp = self.io_scalar_db_tp(val_type).unwrap_or(db_tp);
        let db_tp = self.checked_io_db_tp(db_tp, val_type, "write_file");
        let temp_var = self.vars.unique("wf", val_type, &mut self.lexer);
        self.vars.depend_on_all(temp_var, &val_type.depend());
        let assign = v_set(temp_var, val);
        let var_ref = self.cl("OpCreateStack", &[Value::Var(temp_var)]);
        let write = self.cl(
            "OpWriteFile",
            &[Value::Var(file_var), var_ref, Value::Int(i32::from(db_tp))],
        );
        Value::Insert(vec![assign, write])
    }

    /// @PLN22 Phase 1 — emit the VALUE of `enum_nr`'s variant `variant_name`,
    /// resolved within that enum (so a name two enums share picks the right one).
    /// A plain enum yields the discriminant (`attr_value`); a MIXED struct-enum's
    /// unit variant yields a freshly-allocated DbRef record with the discriminant
    /// at offset 0 (the B2-runtime form) — never a bare `u8` into a DbRef slot.
    /// Returns the enum's value type.  The single emitter every variant-value
    /// resolution site routes through (context branches + the qualified path).
    pub(crate) fn emit_variant_value(
        &mut self,
        enum_nr: u32,
        variant_name: &str,
        code: &mut Value,
    ) -> Type {
        let ret = self.data.def(enum_nr).returned().clone();
        let variant_nr = self.data.variant_of(enum_nr, variant_name);
        if matches!(ret, Type::Enum(_, true, _))
            && !self.first_pass
            && variant_nr != u32::MAX
            && self.data.def(variant_nr).known_type() != u16::MAX
        {
            // Mixed struct-enum unit variant: allocate a record + set the
            // discriminant, mirroring the struct-variant construction path.
            let w = self.vars.work_refs(&ret, &mut self.lexer);
            let known_type = i32::from(self.data.def(variant_nr).known_type());
            let mut list = vec![
                v_set(w, Value::Null),
                self.cl("OpDatabase", &[Value::Var(w), Value::Int(known_type)]),
            ];
            self.object_init(
                &mut list,
                variant_nr,
                0,
                &Value::Var(w),
                &HashSet::new(),
                &HashSet::new(),
            );
            list.push(Value::Var(w));
            // #394 — `w` is a NORMAL work-ref (freed at scope end), NOT skip_free.
            // A `skip_free` here assumed the consumer ALIASES w's store (the `x = B`
            // case, where the assignment transfers ownership to the LHS).  But every
            // DEEP-COPY consumer — a vector literal/element, a struct field, a fn
            // arg, a nested vector — copies the record and orphans w, so skip_free
            // leaked w's store (bounded, one per construction site).  Aligning with
            // the struct-construct path (`parse_object`, also a plain work-ref): the
            // ownership-transfer logic already claims w for the alias / return
            // consumers, and copy consumers free it at scope end — no double-free.
            *code = v_block(
                list,
                Type::Enum(enum_nr, true, crate::data::Deps::none()),
                "EnumUnitLit",
            );
            return ret;
        }
        if let Some(a_nr) = self.data.def(enum_nr).attr_names.get(variant_name) {
            *code = self.data.attr_value(enum_nr, *a_nr);
        }
        ret
    }

    /// Re-point a pasted text constant's build buffer at a fresh buffer owned by the
    /// function doing the pasting.
    ///
    /// A file-scope constant is stored as IR and PASTED at every reference.  A text
    /// initialiser builds its value in place in ONE buffer, held as a variable number
    /// that only means what it says in the file-scope table where the constant was
    /// parsed.  Pasted into a function that number names something else: inside a
    /// formatted string it lands on the format's own `__work_N`, so the block cleared
    /// the text being built — `"[{B}]"` for `B = "x" + "y"` printed `xyxy]`, the `[`
    /// gone and the value appended to itself — and past the end of the table it
    /// panicked the parser instead.
    ///
    /// The declaration already refused any block this cannot re-point
    /// (`constant_block_is_rebindable`), so the rewrite is total.  The buffer is minted
    /// on BOTH passes: the work-buffer counter has to advance identically, or pass 2
    /// numbers every later buffer differently from pass 1.
    fn rebind_constant_buffer(&mut self, mut code: Value, mut tp: Type) -> (Value, Type) {
        let fresh = self.vars.work_text(&mut self.lexer);
        crate::parser::definitions::visit_constant_vars(&mut code, &mut |v| *v = fresh);
        // The type names the same buffer (`text["b"]`), so it has to move with it: a dep
        // still pointing at the declaration's numbering makes the text-return path
        // promote a variable this function does not have.
        if let Type::Text(deps) = &mut tp
            && !deps.is_empty()
        {
            *deps = crate::data::Deps::frame1(fresh);
        }
        (code, tp)
    }

    /// Whether a stored constant's IR carries the file-scope "no slot" sentinel.
    ///
    /// `create_var` answers `u16::MAX` when there is no frame to allocate in, which is
    /// every file-scope initialiser, so a name it could not resolve reads back as
    /// `Var(u16::MAX)` rather than as a missing definition.
    ///
    /// The sentinel alone does not mean "unresolved" — a struct-valued constant needs a
    /// work slot to build its record and gets the same answer for an entirely different
    /// reason.  The caller supplies the rest of the question.
    ///
    /// Takes `&mut` and rewrites nothing: `visit_constant_vars` is the exhaustive walker,
    /// and a read-only twin of it would be a second list of the same facts.
    fn constant_carries_no_slot(code: &mut Value) -> bool {
        let mut sentinel = false;
        crate::parser::definitions::visit_constant_vars(code, &mut |v| {
            if *v == u16::MAX {
                sentinel = true;
            }
        });
        sentinel
    }

    pub(crate) fn parse_constant_value(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        name_pos: &Position,
        qualifier_enum: u32,
    ) -> Type {
        let mut t;
        let mut d_nr = if source == u16::MAX {
            // `(G-Gen)` — inside a generic header the spelling names THAT header's type
            // variable, in a value position as much as in a type one.
            self.def_nr_in_scope(name)
        } else {
            self.data.source_nr(source, name)
        };
        // loft#1049 — a default-file `<T>` placeholder is invisible from a user file, so it
        // is not "the name is taken" here either.  Without this the qualified form `T.N`
        // resolved against the stdlib placeholder, consumed the `.` on its way to a variant
        // that does not exist, and left the fallback looking at `N` with the qualifier
        // already gone — which is why the user saw `Expect token ;` at the `;`.
        if self.stdlib_type_var_placeholder(d_nr) {
            d_nr = u32::MAX;
        }
        // @PLN22 Phase 1 — a qualified `Enum::Variant` resolves WITHIN the
        // qualifier enum via the variant_of chokepoint, NOT the first-wins flat
        // key (which may point at a different enum's same-named variant).
        if qualifier_enum != u32::MAX {
            d_nr = self.data.variant_of(qualifier_enum, name);
        }
        // @PLN22 Phase 1 — a library-qualified `lib::Variant` falls back to the
        // variant_in_source chokepoint.  Harmless for non-variant `lib::name`
        // (no enum has the variant → MAX → falls through).
        if d_nr == u32::MAX && source != u16::MAX {
            d_nr = self.data.variant_in_source(source, name);
        }
        // loft#788 — THE bare-name chokepoint: `source == MAX` is exactly "the
        // author wrote the name with nothing in front of it", which is the only
        // spelling an ambiguity can bite. A qualified `pkg::Name` took the other
        // branch above and says which package it means.
        if source == u16::MAX && qualifier_enum == u32::MAX {
            self.refuse_ambiguous_import(name, d_nr);
        }
        // #493 — a QUALIFIED `Enum::UnknownVariant` (a typo like `Color::Bleu`)
        // that resolves to no variant: recover as a null enum value.  Without
        // this `code` keeps the caller's default (the assignment target itself),
        // so `c = Color::Bleu` lowered to `c = c` — a first-Set self-reference
        // that reads an uninitialised slot (a garbage DbRef under a normal build,
        // a codegen self-ref assert under debug-assertions).  A `{`-construction
        // keeps resolving below (a struct-variant literal).
        if qualifier_enum != u32::MAX && d_nr == u32::MAX && !self.lexer.peek_token("{") {
            // A typo like `Color::Bleu` still recovers as null (below) but must
            // also REPORT: silently nulling an unknown variant hid the typo
            // (exit 0, `null` printed).  Emit once, on pass 2, with an
            // enum-scoped suggestion — variants live in the enum's attributes,
            // so `suggest_field_name` finds them (same source the `.`-access
            // path already suggests from).
            if !self.first_pass {
                let enum_name = self.data.def(qualifier_enum).name().to_string();
                if let Some(s) = self.suggest_field_name(qualifier_enum, name) {
                    diagnostic_at!(
                        self.lexer,
                        name_pos,
                        Level::Error,
                        "unknown variant {enum_name}::{name} — did you mean '{s}'?"
                    );
                } else {
                    diagnostic_at!(
                        self.lexer,
                        name_pos,
                        Level::Error,
                        "unknown variant {enum_name}::{name}"
                    );
                }
            }
            *code = Value::Null;
            return self.data.def(qualifier_enum).returned().clone();
        }
        // @PLN22 Phase 1 — a BARE variant used as a VALUE (not qualified, not a
        // `{ … }` construction) resolves ONLY via context: defer to parse_var's
        // context branches, which resolve it against the expected enum or error
        // if none is in scope.  So `s = Red` with no type context is an error
        // even when `Red` is currently unique — adding a second enum with that
        // variant name can never silently re-point an existing bare assignment.
        // A following `{` is a struct-variant CONSTRUCTION (`Circle { … }`) and
        // keeps resolving below, as does an `Enum::`/`lib::` qualified value.
        if d_nr != u32::MAX
            && source == u16::MAX
            && qualifier_enum == u32::MAX
            && self.data.def_type(d_nr) == DefType::EnumValue
            && !self.lexer.peek_token("{")
        {
            return Type::Null;
        }
        // A forward / cross-package type reference earlier in the body (an
        // `-> Cell` return type or a `c: Cell` annotation parsed above `struct
        // Cell`) registers an `Unknown` STUB def for the name.  For dispatch
        // here that stub is not yet a real type, so treat it like a never-seen
        // name: fall through to the construction-deferral branch below (consume
        // the `{ … }`, return Unknown) instead of the struct path, which would
        // mis-handle the stub and desync the parser into a spurious "Expect
        // token ;".  The stub upgrades to the real struct after this pass, so
        // pass-2 sees a concrete `DefType::Struct` here and builds for real.
        if d_nr != u32::MAX && matches!(self.data.def_type(d_nr), DefType::Unknown) {
            d_nr = u32::MAX;
        }
        if d_nr != u32::MAX {
            self.data.def_used(d_nr);
            t = self.data.def(d_nr).returned().clone();
            if self.data.def_type(d_nr) == DefType::Function {
                t = Type::Routine(d_nr);
            } else if matches!(
                self.data.def_type(d_nr),
                DefType::Struct | DefType::EnumValue
            ) && !matches!(self.data.def(d_nr).returned(), Type::Enum(_, false, _))
            {
                if self.lexer.peek_token("{") {
                    let tp = self.parse_object(d_nr, code);
                    if tp != Type::Unknown(0) {
                        return tp;
                    }
                } else if self.lexer.peek_token(".") {
                    self.lexer.cont();
                    if self.lexer.has_keyword("parse") {
                        return self.parse_type_parse(d_nr, code);
                    }
                }
            // Type.parse() for struct-enums.  Must not consume the
            // "." unless "parse" follows — `Enum.Variant` is a qualified
            // variant reference, not a method call.  Save a link and
            // revert if "parse" doesn't follow.
            } else if self.data.def_type(d_nr) == DefType::Enum
                && matches!(self.data.def(d_nr).returned(), Type::Enum(_, true, _))
                && self.lexer.peek_token(".")
            {
                let link = self.lexer.link();
                self.lexer.cont();
                if self.lexer.has_keyword("parse") {
                    return self.parse_type_parse(d_nr, code);
                }
                self.lexer.revert(link);
            } else if self.data.def_type(d_nr) == DefType::Constant {
                let mut const_code = self.data.def(d_nr).code().clone();
                let const_tp = self.data.def(d_nr).returned().clone();
                // loft#962 — a constant reached before pass 2 has re-read its own
                // declaration still holds pass 1's answer, and pass 1 resolves names
                // against an incomplete table.  `create_var` has no frame at file scope,
                // so a name it could not find left `Var(u16::MAX)` behind, and pasting
                // that panicked the variable allocator at `index 65535` — one file away
                // from the constant that failed, with the caret on an unrelated function.
                // Refuse it here, the only site that knows a paste is about to happen.
                //
                // Three conditions, and the last two are what keep it off working code:
                //
                // * pass 2 — on pass 1 the sentinel is the expected forward-reference
                //   stub, and the declaration has not been re-read yet either way;
                // * the use is textually ABOVE the declaration in the same file.  That is
                //   precisely the window the pass-2 re-store cannot reach, and it is the
                //   sentence the message says.  Without it this fired on a struct-valued
                //   constant used normally: `POINT_NONE = Point { x: 1 };` needs a work
                //   slot to build the record and file scope has none, so `Var(u16::MAX)`
                //   is what that initialiser legitimately holds;
                // * not a `Reference` — that struct-valued kind is refused at the
                //   declaration with a message that names the real limitation, and one
                //   rule keeps one home.
                let decl_pos = self.data.def(d_nr).position().clone();
                let use_pos = self.lexer.pos();
                let reads_above_its_declaration = use_pos.file == decl_pos.file
                    && (use_pos.line, use_pos.pos) < (decl_pos.line, decl_pos.pos);
                if !self.first_pass
                    && reads_above_its_declaration
                    && !matches!(const_tp.base(), Type::Reference(_, _))
                    && Self::constant_carries_no_slot(&mut const_code)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "constant '{name}' is read here, above its declaration at \
                         {decl_pos}, and that declaration reads a name of its own that is \
                         only known later — move `{name}`'s declaration above this use"
                    );
                    *code = Value::Null;
                    return Type::Unknown(0);
                }
                // vector constants are pre-built in CONST_STORE during
                // byte_code(). Emit OpConstRef + OpCopyRecord to deep-copy
                // from the constant store into a fresh runtime store.
                // On pass 1 const_ref is None but we still emit the same IR
                // shape so create_unique runs on both passes (counter sync).
                if matches!(const_tp, Type::Vector(_, _)) && matches!(const_code, Value::Block(_)) {
                    // Emit a simple Call to OpConstRef. The constant's DbRef
                    // will be deep-copied at the call site — the caller's
                    // gen_set_first_ref_call_copy handles the CopyRecord.
                    *code = self.cl("OpConstRef", &[Value::Int(d_nr as i32)]);
                    return const_tp;
                }
                // A text constant builds its value in a buffer whose NUMBER is only
                // valid where the constant was parsed — re-point it at one this
                // function owns before pasting.
                if matches!(const_tp.base(), Type::Text(_))
                    && matches!(const_code.unspan(), Value::Block(_))
                {
                    let (rebound, tp) = self.rebind_constant_buffer(const_code, const_tp);
                    *code = rebound;
                    return tp;
                }
                // loft#744 — an initialiser that carries a frame slot is refused at
                // the DECLARATION (`parse_constant`), where the text case refuses
                // too and where the constant can be named. By the time a paste
                // reaches here the useful line is already behind us, so there is no
                // second policy at the use site — one home for the rule.
                *code = const_code;
                return const_tp;
            }
            // @PLN22 Phase 1 — a qualified `Enum::Variant` / `lib::Variant` value:
            // emit via the shared emitter (plain discriminant or mixed-enum
            // allocation), resolved within the variant's own enum.
            if let Type::Enum(en, _, _) = t
                && self.data.def(en).attr_names.contains_key(name)
            {
                return self.emit_variant_value(en, name, code);
            }
        }
        // #271 — a brace-construction `Name { … }` of a type that exists in a
        // `use`d library but is NOT `pub` arrives here unresolved (only pub names
        // import), so the `{` would otherwise trip a baffling "Expect token ;".
        // Name the real cause instead.  Pass-2 only, to emit once.
        // #271 — a brace-construction `Name { … }` of a type that exists in a
        // `use`d library but is NOT `pub` arrives here unresolved (`use` only
        // imports pub names), so the `{` would otherwise trip a baffling
        // "Expect token ;".  Name the real cause and recover by consuming the
        // balanced `{ … }` so neither pass cascades.  The structural error fires
        // on pass 1 (before pass 2 runs), so emit there; recover on both passes.
        if d_nr == u32::MAX && self.lexer.peek_token("{") && self.data.has_private_type(name) {
            if self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "type '{name}' is private — declare it `pub` to construct it outside its library"
                );
            }
            self.lexer.has_token("{");
            let mut depth = 1u32;
            while depth > 0 {
                let before = self.lexer.peek().position;
                if self.lexer.has_token("{") {
                    depth += 1;
                } else if self.lexer.has_token("}") {
                    depth -= 1;
                } else {
                    self.lexer.cont();
                }
                if self.lexer.peek().position == before {
                    break; // no forward progress (EOF) — bail rather than spin
                }
            }
            return Type::Unknown(0);
        }
        // A brace-construction `Name { field: … }` of a type that does not
        // exist at all.  Without naming the real cause, the unconsumed `{`
        // trips a baffling `Expect token ;` mid-literal.  Recover by consuming
        // the balanced `{ … }` so the statement parses.  `!name_exists` gates
        // out known variables (`items { … }` opening a loop body, `b { … }` an
        // if body) BEFORE the stateful struct-shape lookahead — a genuinely
        // unknown type has not been turned into a placeholder variable yet, so
        // it still passes — and the `ident :` / `ident ,` shape check (the same
        // `parse_block` uses) keeps a control-flow block from matching.
        if d_nr == u32::MAX
            && !self.vars.name_exists(name)
            && self.lexer.peek_token("{")
            && self.peek_struct_literal_body()
        {
            // Pass 1 leaves a forward-reference STUB for the name, the same one a written
            // type would leave (`r: Roofs = …` goes through `parse_type`, which registers
            // one).  That stub is the entire mechanism by which a cross-module forward
            // reference ever resolves: `use inner;` imports the module's names into the
            // importer, and the importer's own `struct Roofs` then ADOPTS the stub in
            // place, so both files end up sharing one def.  Without a stub there is
            // nothing to import and nothing to adopt, which is why `r: Roofs = Roofs {…}`
            // compiled and the identical `r = Roofs {…}` did not (loft#801).
            //
            // Only in pass 1, and only for a `Name { … }` construction — the shape check
            // above has already ruled out a variable and a control-flow block.
            //
            // Ask the def table, not `d_nr`: an existing stub was blanked to `u32::MAX`
            // just above so this branch would handle it, so `d_nr` cannot tell "no such
            // name" from "a stub is already waiting".  Registering a second def under a
            // name the source already has aborts `add_def` — which is what a declaration
            // plus a construction of the same type (`h: Roofs` and `Roofs { … }`, the
            // ordinary shape) did.
            if self.first_pass && self.data.def_nr(name) == u32::MAX {
                self.speculative_type_refs.insert(self.data.add_def(
                    name,
                    name_pos,
                    DefType::Unknown,
                ));
            }
            // Emit on pass 2, NOT pass 1: unlike the private-type branch above
            // (a private type is `u32::MAX` in BOTH passes, so pass-1 emission
            // fires once and is always correct), a FORWARD-REFERENCED or
            // cross-package struct is `u32::MAX` only in pass-1 — its definition
            // registers before pass-2.  Emitting in pass-1 raised a false
            // "unknown type" for `Cell { … }` written above `struct Cell`.
            // Pass-2 has every def, so `u32::MAX` there means the type is
            // genuinely undefined; emit then (still once).  Both passes still
            // recover by consuming the balanced `{ … }`, so pass-1 never
            // cascades while it waits for pass-2 to resolve the forward ref.
            if !self.first_pass {
                if let Some(s) = self.suggest_type_name(name) {
                    diagnostic_at!(
                        self.lexer,
                        name_pos,
                        Level::Error,
                        "unknown type '{name}' — did you mean '{s}'?"
                    );
                } else {
                    diagnostic_at!(self.lexer, name_pos, Level::Error, "unknown type '{name}'");
                }
            }
            self.lexer.has_token("{");
            let mut depth = 1u32;
            while depth > 0 {
                let before = self.lexer.peek().position;
                if self.lexer.has_token("{") {
                    depth += 1;
                } else if self.lexer.has_token("}") {
                    depth -= 1;
                } else {
                    self.lexer.cont();
                }
                if self.lexer.peek().position == before {
                    break; // no forward progress (EOF) — bail rather than spin
                }
            }
            // @P376 — POISON the errored construction, PASS 2 ONLY.
            //
            // Pass 1 must DEFER as `Unknown(0)`: the construction may be a
            // forward / cross-package `Cell { … }` (or `[Cell { … }]`) whose
            // `struct Cell` registers before pass 2.  `Unknown` is the type
            // every container / materialisation path already handles, so the
            // forward ref re-resolves cleanly in pass 2.  (Returning the poison
            // in pass 1 leaks `Never` into e.g. the vector-literal materialiser,
            // which then can't find a type for it.)
            //
            // Pass 2 is final — a still-unknown name is a genuine typo (the
            // `unknown type '…'` above just fired).  Leaving the variable
            // `Unknown` makes every downstream use (`p.name`, the `{p.name}`
            // format string, the post-parse unknown-type sweep) re-report: a
            // 9-error cascade off one typo (#376).  `Type::Never` (the bottom
            // type) makes the variable a registered, silently-typed poison that
            // field access, format interpolation, and the sweep all skip, so the
            // single `unknown type '…'` is the only diagnostic.  The program
            // still aborts on that error, so the poison never reaches runtime.
            // (`change_var_type` overwrites the pass-1 `Unknown` with this
            // `Never`; single-pass #284/#302 got this for free by skipping
            // pass 2, but two-pass deferral needs the explicit poison.)
            if self.first_pass {
                return Type::Unknown(0);
            }
            return Type::Never;
        }
        Type::Null
    }

    /// Peek past a `{` to decide whether it opens a struct literal (`{ field:
    /// … }` / `{ field, … }` / `{ }`) rather than a control-flow block.
    /// Non-consuming (uses a lexer link/revert); mirrors the disambiguation in
    /// `parse_block`.
    ///
    /// loft#986 — an EMPTY body is a struct literal too, and it is the one spelling the
    /// field-shape test cannot recognise: `T { }` asks for the whole default record, so
    /// there is no `field:` or `field,` to look at.  Without this the `{` went
    /// unconsumed and the statement failed with `Expect token ;` pointing at the line
    /// rather than at the type — but ONLY when `T` was declared BELOW the use, since a
    /// declared type never reaches this fallback at all.  Legality by declaration order,
    /// for the spelling a reader reaches for first.
    ///
    /// Checked BEFORE the identifier peek, which consumes.  The caller has already ruled
    /// out a known variable (`items { … }` opening a loop body), so a bare unknown name
    /// followed by `{ }` here is a construction of a type that does not exist yet.
    fn peek_struct_literal_body(&mut self) -> bool {
        let link = self.lexer.link();
        self.lexer.token("{");
        let looks_like_struct = (self.lexer.peek_token("}") && !self.in_control_head)
            || (self.lexer.has_identifier().is_some()
                && ((self.lexer.peek_token(":") && !self.lexer.peek_token(":="))
                    || self.lexer.peek_token(",")));
        self.lexer.revert(link);
        looks_like_struct
    }

    pub(crate) fn known_var_or_type(&mut self, code: &Value, pos: &Position) {
        if let Value::Var(nr) = code {
            if !self.vars.exists(*nr) {
                return;
            }
            if self.default && matches!(self.vars.tp(*nr), Type::Vector(_, _)) {
                return;
            }
            if !self.first_pass && (self.vars.tp(*nr).is_unknown() || !self.vars.is_defined(*nr)) {
                let name = self.vars.name(*nr).to_string();
                if name == "_" {
                    // loft#795 — `_` DISCARDS its value (each `_ = …` gets its own slot),
                    // so there is nothing here to read back.  Say that rather than
                    // "Unknown variable '_'", which reads as a typo in the one case where
                    // the name is deliberate — and never offer a rename suggestion for it.
                    diagnostic_at!(
                        self.lexer,
                        pos,
                        Level::Error,
                        "`_` discards the value assigned to it — there is nothing to read \
                         back; give the value a name if you need it"
                    );
                    return;
                }
                // loft#826 — a file-scope constant declared by the file that
                // `use`d this one reaches here as an unknown VARIABLE, because
                // that is all a name with no definition behind it can be.  Same
                // boundary as the unknown-function and undefined-type cases, and
                // the same cure; without it the reader is told `TOP` is a typo
                // while looking straight at `TOP` in the importing file.
                if let Some(note) = self.importer_boundary_note(&name) {
                    diagnostic_at!(self.lexer, pos, Level::Error, "{note}");
                    return;
                }
                let candidates: Vec<&str> = (0..self.vars.count())
                    .filter(|&v| {
                        v != *nr && self.vars.is_defined(v) && !self.vars.tp(v).is_unknown()
                    })
                    .map(|v| self.vars.name(v))
                    .collect();
                // Plan-07 phase 5: skip suggestions for very short names
                // (1 char) where typos are too ambiguous to be meaningful
                // (`x` vs `y` is a coin flip).  Standard `suggest_similar`
                // (distance ≤ 2) used here instead of the more aggressive
                // `suggest_similar_capped` because variable typos like
                // `result` vs `reuslt` (6-char transposition, distance 2)
                // are common and worth suggesting; the capped version's
                // `min(2, n/4)` formula is too strict for short names.
                // loft#1008 — a METHOD is registered as `t_<len><Type>_<name>`, so its bare
                // name has no definition to bind and reads as unknown wherever a VALUE is
                // wanted: a fn-ref argument, `map(v, f)`. Reporting that the file's own
                // function does not exist sends the reader looking for a typo; name what it
                // is and what to write instead. Checked BEFORE the spelling suggestion,
                // which would otherwise offer the nearest local.
                let receivers = self.method_receivers_named(&name);
                if !receivers.is_empty() {
                    let on = receivers.join("`, `");
                    diagnostic_at!(
                        self.lexer,
                        pos,
                        Level::Error,
                        "`{name}` is a method on `{on}`, and a method is not a function VALUE — \
                         there is nothing to bind here. Wrap it: `|x| {{ x.{name}(…) }}`, or \
                         declare the function with a plain first-parameter name (not `self` / \
                         `both`), which makes it a free function and a usable fn-ref"
                    );
                    return;
                }
                let suggestion = if name.chars().count() <= 1 {
                    None
                } else {
                    crate::diagnostics::suggest_similar(&name, &candidates)
                };
                if let Some(s) = suggestion {
                    diagnostic_at!(
                        self.lexer,
                        pos,
                        Level::Error,
                        code = "unknown-variable",
                        "Unknown variable '{}' — did you mean '{}'?",
                        name,
                        s
                    );
                    self.lexer.suggest_last(s);
                    // `pos` is the name's START (unlike the field site, which reports at
                    // the cursor), so the span is the name exactly.
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Mechanical,
                        title: format!("rename to `{s}`"),
                        condition: None,
                        edit: Some(crate::diagnostics::Edit {
                            line: pos.line,
                            col: pos.pos,
                            len: u32::try_from(name.len()).unwrap_or(0),
                            text: s.to_string(),
                        }),
                        concept: "declarations",
                        concept_ref: "@F16",
                    });
                } else {
                    diagnostic_at!(self.lexer, pos, Level::Error, "Unknown variable '{}'", name);
                }
            }
        }
    }

    /// `Type.parse(arg)` — populate a struct from a JsonValue.
    ///
    /// Single-walker design: regardless of the struct's
    /// shape, this emits exactly one IR call to
    /// `n_struct_from_jsonvalue(arg, struct_kt)`.  The walker uses
    /// `stores.types[struct_kt].parts` at runtime to dispatch on each
    /// field's declared type — primitives get extracted with
    /// path-qualified Q1 schema-side type checks, nested struct
    /// fields recurse, JsonValue-passthrough fields byte-copy, and
    /// vector fields iterate the JArray and recurse per element for
    /// struct elements.
    ///
    /// **Auto-wrap:** when the argument is plain
    /// text, transparently wrap with `json_parse(text)` first so
    /// legacy `Struct.parse(text)` keeps compiling but routes
    /// through the typed-tree pipeline (malformed input populates
    /// `json_errors()` instead of silently zero-filling the struct).
    /// Users wanting explicit staging can write
    /// `Struct.parse(json_parse(text))` themselves.
    fn parse_type_parse(&mut self, d_nr: u32, code: &mut Value) -> Type {
        self.lexer.token("(");
        let mut arg_expr = Value::Null;
        let arg_tp = self.expression(&mut arg_expr);
        self.lexer.token(")");
        if !self.first_pass {
            // JsonValue resolves to either Type::Reference (if a
            // user-declared alias) or Type::Enum(_, true, _) (mixed
            // struct-enum, the actual stdlib decl shape).
            let is_jsonvalue = match &arg_tp {
                Type::Reference(d, _) | Type::Enum(d, true, _) => {
                    self.data.def(*d).name() == "JsonValue"
                }
                _ => false,
            };
            if is_jsonvalue {
                // Direct JsonValue → walker.  This is the new
                // typed-tree path used by `Struct.parse(json_parse(text))`
                // and by the `Struct.parse(JsonValue)` codegen elsewhere.
                let n_walker = self.data.def_nr("n_struct_from_jsonvalue");
                debug_assert_ne!(
                    n_walker,
                    u32::MAX,
                    "n_struct_from_jsonvalue must be registered in NATIVE_FNS"
                );
                let known_tp = self.data.def(d_nr).known_type();
                *code = Value::Call(n_walker, vec![arg_expr, Value::Int(i32::from(known_tp))]);
            } else {
                // Text or other → legacy lenient text-parse path
                // (`OpCastVectorFromText` calls
                // `src/database/structures.rs::parsing`, which accepts
                // both standard JSON and loft-native bare-key syntax).
                // Preserves the legacy data-import semantics.
                let mut text_expr = arg_expr;
                if !matches!(arg_tp, Type::Text(_)) {
                    self.convert(
                        &mut text_expr,
                        &arg_tp,
                        &Type::Text(crate::data::Deps::none()),
                    );
                }
                let known_tp = self.data.def(d_nr).known_type();
                *code = self.cl(
                    "OpCastVectorFromText",
                    &[text_expr, Value::Int(i32::from(known_tp))],
                );
            }
        }
        Type::Reference(d_nr, crate::data::Deps::none())
    }

    /// Parse `vector<T>.parse(text)` — parse a JSON array into a vector of T.
    /// Returns `Type::Vector(T)` so the result is directly iterable.
    fn parse_vector_parse(&mut self, elem_d_nr: u32, code: &mut Value) -> Type {
        self.lexer.token("(");
        let mut text_expr = Value::Null;
        let tp = self.expression(&mut text_expr);
        self.lexer.token(")");
        let elem_tp = Type::Reference(elem_d_nr, crate::data::Deps::none());
        let vec_type = Type::Vector(Box::new(elem_tp.clone()), crate::data::Deps::none());
        if !self.first_pass {
            if !matches!(tp, Type::Text(_)) {
                self.convert(&mut text_expr, &tp, &Type::Text(crate::data::Deps::none()));
            }
            // Get the database vector type for vector<elem>.
            let elem_kt = self.data.def(elem_d_nr).known_type();
            let vec_kt = self.database.vector(elem_kt);
            let parse_call = self.cl(
                "OpCastVectorFromText",
                &[text_expr, Value::Int(i32::from(vec_kt))],
            );
            // The parse returns a DbRef to the wrapper struct main_vector<T>.
            // Extract the vector field (at position 0) so the result is directly iterable.
            let wrapper_name = format!("main_vector<{}>", self.data.def(elem_d_nr).name());
            let wrapper_d_nr = self.data.def_nr(&wrapper_name);
            if wrapper_d_nr == u32::MAX {
                *code = parse_call;
            } else {
                *code = self.get_field(wrapper_d_nr, 0, parse_call);
            }
        }
        // Ensure the vector def exists for type resolution.
        self.data.vector_def(&mut self.lexer, &elem_tp);
        vec_type
    }

    // @F35 — string literals ({expr} interpolation + backtick multiline)
    //
    // @PLN124 — `target` is the struct this string BUILDS (from
    // `Parser::interpolation_target`), or `u32::MAX` for an ordinary text string.
    // On the build path the literal/hole boundary the parser already knows is
    // handed to the type instead of being erased into one buffer: literals reach
    // `lit`, values reach `hole_<kind>`. That erasure is the only reason a value
    // can become syntax, so not erasing it is the whole feature.
    pub(crate) fn parse_string(&mut self, code: &mut Value, string: &str, target: u32) -> Type {
        let mut append_value = u16::MAX;
        *code = Value::str(string);
        let mut var = u16::MAX;
        let mut list = vec![];
        if target != u32::MAX {
            // The accumulator is built even when the string has NO holes: a
            // `SqlText` with no parameters is still a `SqlText`, and the target
            // type is what the caller asked for.
            var = self.begin_format_object(target, &mut list);
            self.format_lit(target, var, string, &mut list);
        } else if self.lexer.mode() == Mode::Formatting {
            // Define a new variable to append to
            var = self.vars.work_text(&mut self.lexer);
            list.push(v_set(var, code.clone()));
        }
        while self.lexer.mode() == Mode::Formatting {
            self.lexer.set_mode(Mode::Code);
            let mut format = Value::Null;
            // @PLN124 — `in_format_expr` also gates the interpolation TARGET (see
            // `constant` in vectors.rs): a hole is not the destination, so a string
            // literal inside one does not inherit the destination's type.
            let saved_in_fmt = self.in_format_expr;
            self.in_format_expr = true;
            let mut tp = if self.lexer.has_token("for") {
                self.iter_for(&mut format, &mut append_value)
            } else {
                self.expression(&mut format)
            };
            self.in_format_expr = saved_in_fmt;
            // Plan-07 phase 4e.1 — format strings are the user's
            // observability surface and must NEVER halt, log, or warn
            // (per C66 + DESIGN_DECISIONS 2026-05-11).  Walk the
            // interpolated expression tree and swap every fault-prone
            // op to its Nullable peer so `println("{a / b}")` /
            // `println("{user.name}")` / `println("{v[i]}")` always
            // render the silent sentinel ("null") instead of taking
            // out the print statement that the developer is using to
            // diagnose the problem in the first place.
            //
            // Phase 4e.3 — when the OUTERMOST swapped op is
            // fault-prone (div / mod / vector-index / text-index),
            // append an `OpTagFault(kind)` SIBLING statement to the
            // statement list BEFORE the format-conversion op so the
            // conversion op sees the tag and renders `null(<reason>)`
            // instead of bare `null` on the null sentinel.  Inner
            // faults (`"{a + v[i] / b}"`) get their Nullable peer
            // swap from the recursion but do NOT tag — there's no
            // renderer to consume their tag in mid-expression.
            let outer_fault_kind = if self.first_pass {
                None
            } else {
                Self::rewrite_subtree_to_nullable_kind(&mut format, &self.data)
            };
            if let Some(kind) = outer_fault_kind
                && self.data.def_nr("OpTagFault") != u32::MAX
            {
                let tag_call = self.cl("OpTagFault", &[Value::Int(i32::from(kind))]);
                list.push(tag_call);
            }
            self.un_ref(&mut tp, &mut format);
            // @P376 — the format expression resolved to `Unknown`: a directly
            // interpolated unresolved name (`print("{zzz}")`, zzz undefined)
            // whose root "Unknown variable 'zzz'" already fired.  Returning
            // `Void` here aborted mid-placeholder — the lexer never entered
            // `Formatting` mode nor consumed the rest of the `{…}`, cascading
            // "Expect token )", "expected text, got void", and a nested-string
            // fatal.  Poison `tp` to `Never` and fall through to the normal flow:
            // the placeholder parses + is consumed cleanly, the format dispatch
            // silences `Never`, and only the root error remains.
            if !self.first_pass && tp.is_unknown() {
                tp = Type::Never;
            }
            self.lexer.set_mode(Mode::Formatting);
            let mut state = OUTPUT_DEFAULT;
            let mut token = "0".to_string();
            let mut spec_string = String::new();
            // @PLN99 Arc B — a value whose type defines its own `to_text` owns its
            // `{x:spec}` DSL: read the spec RAW (v1: one identifier/token) instead
            // of the numeric width/radix grammar (which rejects `date`/`dollars`
            // as an "Unknown variable" width expression). Pass-stable: the
            // `t_<len><Type>_to_text` def is collected in both parser passes, so
            // both take this branch.
            let custom_fmt = if let Type::Reference(fd, _) = &tp {
                self.data.def_type(*fd) == DefType::Struct && {
                    let nm = self.data.def(*fd).name().to_string();
                    self.data.def_nr(&format!("t_{}{}_to_text", nm.len(), nm)) != u32::MAX
                }
            } else {
                false
            };
            let had_spec = self.lexer.has_token(":");
            if had_spec {
                if custom_fmt {
                    if let LexResult {
                        has: LexItem::Token(t) | LexItem::Identifier(t),
                        position: _pos,
                    } = self.lexer.peek()
                    {
                        spec_string = t;
                        self.lexer.cont();
                    }
                } else {
                    if let LexResult {
                        has: LexItem::Token(t),
                        position: _pos,
                    } = self.lexer.peek()
                    {
                        let st: &str = &t;
                        if !SKIP_TOKEN.contains(&st) {
                            token.clear();
                            token += &t;
                            state.token = &token;
                            self.lexer.cont();
                        }
                    }
                    self.string_states(&mut state);
                    let LexResult {
                        has: h,
                        position: _pos,
                    } = self.lexer.peek();
                    if match h {
                        LexItem::Token(st) | LexItem::Identifier(st) => {
                            let s: &str = &st;
                            !SKIP_WIDTH.contains(&s) && crate::parser::radix_for(s).is_none()
                        }
                        LexItem::Integer(_, _) | LexItem::Float(..) => true,
                        _ => false,
                    } {
                        // @FR-F-Spec — a leading zero on the WIDTH is the zero-pad flag.
                        // Both literal spellings carry it: `{n:08}` lexes as an Integer and
                        // the dotted `{f:08.2}` — the only spelling that gives a width and a
                        // precision at once — lexes as a Float, whose parsed value cannot
                        // answer the question because `08.2` and `8.2` are the same number.
                        //
                        // A bare `.P` is the one spelling where the literal ahead is the
                        // PRECISION and not the width, and a precision has no padding to
                        // flag.  `state.float` is set by the `.` that `string_states` just
                        // consumed, so it is what tells the two apart: without it `{f:.0}`
                        // read its own precision digit as a leading zero and zero-padded a
                        // field it never asked for.
                        if !state.float
                            && matches!(
                                self.lexer.peek().has,
                                LexItem::Integer(_, true) | LexItem::Float(_, true)
                            )
                        {
                            state.token = "0";
                        }
                        self.lexer.set_mode(Mode::Code);
                        let w_tp = self.expression(&mut state.width);
                        self.lexer.set_mode(Mode::Formatting);
                        // @FR-F-Spec — the width is a NUMBER.  The slot parses a full
                        // expression so a variable can supply the width, and it used to
                        // accept whatever that expression produced.  `{n:0>5}` — the
                        // zero-pad-right spelling a Rust reader writes — parsed `0 > 5`
                        // as a comparison and handed a BOOLEAN to the width: no padding
                        // at all on `--interpret`, and `E0308 expected i64, found bool`
                        // straight from rustc on `--native`.  Neither named the spec.
                        //
                        // It is the residual of the defect `string_states` closed for the
                        // FLAGS.  A pad character is claimed before the flags, but only
                        // when it lexes as a Token — a digit lexes as an Integer, so the
                        // pad branch cannot claim it and it falls through to the width
                        // exactly as an out-of-order flag used to.
                        //
                        // A dotted spec (`{f:8.3}`) legitimately arrives as a Float
                        // LITERAL, which `append_data_fp` splits into width and
                        // precision; a float VARIABLE is not that spelling and is refused
                        // with the rest.
                        let width_is_a_number = matches!(w_tp, Type::Integer(_) | Type::Unknown(_))
                            || (matches!(w_tp, Type::Float)
                                && matches!(state.width, Value::Float(_)));
                        if !self.first_pass && !width_is_a_number {
                            if matches!(w_tp, Type::Boolean) {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "a format width must be a number, and this one is a \
                                     comparison — a digit before `<`, `>` or `^` reads as \
                                     an operator (`0>5` is `0 > 5`); write `05` to \
                                     zero-pad, or `*>5` to pad with a non-digit character"
                                );
                            } else {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "a format width must be a number, not {}",
                                    w_tp.name(&self.data)
                                );
                            }
                        }
                    }
                    state.radix = self.get_radix();
                }
            }
            state.spec = &spec_string;
            if target == u32::MAX {
                self.append_data(tp, &mut list, var, append_value, &format, state);
            } else {
                self.format_hole(target, var, &tp, format, had_spec, &mut list);
            }
            if let Some(text) = self.lexer.has_cstring() {
                if target != u32::MAX {
                    self.format_lit(target, var, &text, &mut list);
                } else if !text.is_empty() {
                    let call = if matches!(self.vars.tp(var), Type::RefVar(_)) {
                        "OpAppendStackText"
                    } else {
                        "OpAppendText"
                    };
                    list.push(self.cl(call, &[Value::Var(var), Value::str(&text)]));
                }
            } else {
                diagnostic!(self.lexer, Level::Error, "Formatter error");
                return Type::Void;
            }
        }
        if target != u32::MAX {
            list.push(Value::Var(var));
            let tp = Type::Reference(target, crate::data::Deps::frame1(var));
            *code = v_block(list, tp.clone(), "Formatted object");
            return tp;
        }
        if var < u16::MAX {
            list.push(Value::Var(var));
            *code = v_block(
                list,
                Type::Text(crate::data::Deps::frame1(var)),
                "Formatted string",
            );
            Type::Text(crate::data::Deps::frame1(var))
        } else {
            Type::Text(crate::data::Deps::none())
        }
    }

    /// @PLN124 — mint the accumulator a format string builds into, and emit its
    /// construction: an empty record of the target type, every field defaulted.
    ///
    /// The same prelude a `T { }` literal in value position emits, for the same
    /// reason — this IS such a literal, the parser just wrote it instead of the
    /// author. Reusing `object_init` is what keeps field defaults, vector headers
    /// and constraint-free construction in ONE place rather than a second copy
    /// that drifts.
    fn begin_format_object(&mut self, target: u32, list: &mut Vec<Value>) -> u16 {
        let ret = self.data.def(target).returned();
        let var = self.vars.work_format(ret, &mut self.lexer);
        let kt = i32::from(self.data.def(target).known_type());
        self.data.set_referenced(target, self.context, Value::Null);
        list.push(v_set(var, Value::Null));
        list.push(self.cl("OpDatabase", &[Value::Var(var), Value::Int(kt)]));
        if !self.first_pass {
            let none = HashSet::new();
            let code = Value::Var(var);
            self.object_init(list, target, 0, &code, &none, &HashSet::new());
        }
        var
    }

    /// Emit `acc.lit("…")` for a literal chunk the AUTHOR wrote.
    ///
    /// An empty chunk is skipped: it carries no bytes, and a `lit("")` would only
    /// make the emitted sequence depend on where the holes happen to sit. What a
    /// target must NOT infer from that is chunk boundaries — those come from the
    /// `hole_*` calls, which is the one thing `lit` cannot be asked to report.
    fn format_lit(&mut self, target: u32, var: u16, text: &str, list: &mut Vec<Value>) {
        if text.is_empty() {
            return;
        }
        let nm = self.data.def(target).name();
        let d_nr = self.data.def_nr(&format!("t_{}{}_lit", nm.len(), nm));
        if d_nr == u32::MAX || self.data.attributes(d_nr) != 2 {
            return;
        }
        list.push(Value::Call(d_nr, vec![Value::Var(var), Value::str(text)]));
    }

    /// Emit `acc.hole_<kind>(value)` for an interpolated VALUE.
    ///
    /// The kind is read off the expression's own type, so the target receives the
    /// value rather than a rendering of it — which is the property that makes a
    /// hole unable to become syntax. A kind the target does not accept is REFUSED
    /// naming the method to add, and never quietly rendered to text: silently
    /// falling back to `hole_text` would put a value back on the text path, i.e.
    /// exactly the hole this design exists to close.
    fn format_hole(
        &mut self,
        target: u32,
        var: u16,
        tp: &Type,
        value: Value,
        had_spec: bool,
        list: &mut Vec<Value>,
    ) {
        // @PLN25 — nullability is not a KIND. A `text?` hole is a text hole whose
        // value may be absent, and whether that is acceptable is decided by the
        // target's own `hole_text` parameter type (`v: text?` takes both, `v: text`
        // takes only the non-null one). Peeling here is what lets a type make SQL
        // NULL a distinct bound value rather than the text "null".
        let (tp, _) = tp.peel_optional();
        let kind = match tp {
            Type::Text(_) => "text".to_string(),
            Type::Integer(_) => "int".to_string(),
            Type::Float => "float".to_string(),
            Type::Single => "single".to_string(),
            Type::Boolean => "boolean".to_string(),
            Type::Character => "character".to_string(),
            // @PLN124 H6 — a hole may also be a value of the library's OWN type,
            // and that is what lets a target hold something apart from both a
            // literal and a bound value: a `SqlIdent` is a table name, which is
            // genuinely syntax, so it goes in INLINE. The safety then rests on
            // the type rather than on the parser — nothing constructs a
            // `SqlIdent` but its validating constructor, so there is still one
            // place to audit. The kind is the type's own name in the case a loft
            // method is spelled in, so `SqlIdent` asks for `hole_sql_ident`.
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Struct => {
                Self::hole_kind(self.data.def(*d_nr).name())
            }
            Type::Enum(d_nr, _, _) => Self::hole_kind(self.data.def(*d_nr).name()),
            // `Never` is a poisoned hole whose own diagnostic already fired
            // (@P376); adding a second one would bury the root error.
            Type::Never | Type::Unknown(_) => return,
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a {} cannot be interpolated into a {} — a hole is a scalar or a value \
                         of a named type, handed to the type rather than rendered into it",
                        tp.name(&self.data),
                        self.data.def(target).name()
                    );
                }
                return;
            }
        };
        if had_spec && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a format spec has no meaning on a {} hole — the value is handed to the type, \
                 not rendered, so there is nothing for the spec to format",
                self.data.def(target).name()
            );
        }
        let nm = self.data.def(target).name().to_string();
        let d_nr = self.data.def_nr(&format!("t_{}{nm}_hole_{kind}", nm.len()));
        if d_nr == u32::MAX || self.data.attributes(d_nr) != 2 {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{nm} has no `fn hole_{kind}(self: {nm}, v: {})` — declare one to accept \
                     this hole",
                    tp.name(&self.data)
                );
            }
            return;
        }
        list.push(Value::Call(d_nr, vec![Value::Var(var), value]));
    }

    /// @PLN124 H6 — the hole kind a value of type `name` asks for: the type's own
    /// name, in the case a loft method is written in (`SqlIdent` → `sql_ident`,
    /// so the method is `hole_sql_ident`).
    ///
    /// Derived rather than chosen, so a target and the parser cannot disagree
    /// about what a type's hole is called, and the diagnostic can name the exact
    /// method to add. An acronym run keeps its boundary at the last capital, the
    /// place where the next word starts (`SQLIdent` → `sql_ident`).
    fn hole_kind(name: &str) -> String {
        let chars: Vec<char> = name.chars().collect();
        let mut out = String::with_capacity(name.len() + 4);
        for (i, c) in chars.iter().enumerate() {
            if i > 0 && c.is_uppercase() {
                let prev = chars[i - 1];
                let starts_word = prev.is_lowercase()
                    || prev.is_numeric()
                    || (prev.is_uppercase() && chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
                if starts_word {
                    out.push('_');
                }
            }
            out.extend(c.to_lowercase());
        }
        out
    }

    /// Read the flags of a `{value:spec}` placeholder, in whatever order they are
    /// written.
    ///
    /// Order used to be fixed — alignment, then `+` — and a flag written out of that
    /// order was simply left in the stream for the WIDTH expression to find: `{f:+<8.3}`
    /// read `+`, then parsed `<8.3` as a comparison.  The interpreter rendered `0.5`
    /// (no sign, no width, no precision) and the native backend emitted a comparison
    /// between an i64 and an f64, so the program failed to compile with rustc errors
    /// about loft's own internals.  Neither said the spec was the problem.
    ///
    /// Looping until a round consumes nothing is what makes the order not matter.  Each
    /// flag is a distinct token, so a round that matches none of them has reached the
    /// width — and a round that matches one has consumed it, which is what ends the loop.
    pub(crate) fn string_states(&mut self, state: &mut OutputState) {
        loop {
            let mut consumed = false;
            if self.lexer.has_token("<") {
                state.dir = -1;
                consumed = true;
            } else if self.lexer.has_token("^") {
                state.dir = 0;
                consumed = true;
            } else if self.lexer.has_token(">") {
                state.dir = 1;
                consumed = true;
            }
            if self.lexer.has_token("+") {
                state.plus = true;
                consumed = true;
            }
            if self.lexer.has_token("#") {
                // show 0x 0b or 0o in front of numbers when applicable
                state.note = true;
                consumed = true;
            }
            if self.lexer.has_token(".") {
                state.float = true;
                consumed = true;
            }
            if !consumed {
                return;
            }
        }
    }

    /// Read the radix letter closing a `{x:…}` spec, defaulting to decimal when the spec
    /// has none.  The letter set lives in [`crate::parser::radix_for`], which the width
    /// decision above consults too, so the two cannot drift apart.
    pub(crate) fn get_radix(&mut self) -> i32 {
        let Some(id) = self.lexer.has_identifier() else {
            return 10;
        };
        if let Some(radix) = crate::parser::radix_for(&id) {
            radix
        } else {
            diagnostic!(self.lexer, Level::Error, "Unexpected formatting type: {id}");
            10
        }
    }

    // Iterator for
    // <for> ::= <identifier> 'in' <range> '{' <block>
    pub(crate) fn iter_for(&mut self, val: &mut Value, append_value: &mut u16) -> Type {
        if let Some(src_id) = self.lexer.has_identifier() {
            // loft#915 — the name this loop BINDS; its companions hang off it too.
            let id = self.vars.loop_binding(&src_id);
            // Create {id}#index first (always needed, regardless of type).
            let index_var = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(index_var);
            self.lexer.token("in");
            let loop_nr = self.vars.start_loop();
            let mut expr = Value::Null;
            let in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
            // For text loops: {id}#next drives the loop; {id}#index is saved per-iteration.
            let (iter_var, pre_var) = if matches!(in_type, Type::Text(_)) {
                let pos_var = self.create_var(&format!("{id}#next"), &I32);
                self.vars.defined(pos_var);
                (pos_var, Some(index_var))
            } else {
                (index_var, None)
            };
            let var_tp = self.for_type(&in_type);
            *append_value = self.create_unique("val", &Type::Unknown(0));
            let for_var = self.create_loop_var(&id, &var_tp);
            // The body reads the name the program wrote (loft#915).
            if id != src_id {
                self.vars.set_name(&src_id, for_var);
            }
            self.vars.defined(for_var);
            let if_step = if self.lexer.has_token("if") {
                let mut if_expr = Value::Null;
                self.expression(&mut if_expr);
                if_expr
            } else {
                Value::Null
            };
            let mut create_iter = expr;
            let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
            let iter_next = self.iterator(&mut create_iter, &in_type, &it, iter_var, pre_var);
            if !self.first_pass && iter_next == Value::Null {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Need an iterable expression in a for statement"
                );
                return Type::Null;
            }
            let for_next = v_set(for_var, iter_next);
            self.vars.loop_var(for_var);
            let in_loop = self.in_loop;
            self.in_loop = true;
            let mut block = Value::Null;
            let format_type = self.parse_block("for", &mut block, &Type::Unknown(0));
            self.change_var_type(*append_value, &format_type);
            self.in_loop = in_loop;
            let mut lp = vec![for_next];
            if !matches!(in_type, Type::Iterator(_, _)) {
                lp.push(v_if(
                    self.single_op("!", Value::Var(for_var), var_tp.clone()),
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            }
            if if_step != Value::Null {
                lp.push(v_if(if_step, Value::Null, Value::Continue(0)));
            }
            let result_tp = if let Value::Block(bl) = &block {
                bl.result.clone()
            } else {
                var_tp.clone()
            };
            lp.push(block);
            let tp = Type::Iterator(Box::new(format_type), Box::new(Type::Null));
            // For text loops, extra_init holds v_set(index_var, 0) which must be emitted at
            // the same scope level as the iterator init (outside the loop) so the slot
            // assigner sees {id}#index as live across the entire loop body.
            let extra_init = if let Some(idx_var) = pre_var {
                Box::new(v_set(idx_var, Value::Int(0)))
            } else {
                Box::new(Value::Null)
            };
            *val = Value::Iter(
                for_var,
                Box::new(create_iter),
                Box::new(v_block(lp, result_tp, "Iter For")),
                extra_init,
            );
            self.vars.finish_loop(loop_nr);
            return tp;
        }
        diagnostic!(self.lexer, Level::Error, "Expect variable after for");
        Type::Null
    }

    /// Resolve one slice bound to a valid `[0, len]` index: a negative bound
    /// counts from the end (`b + len`), an inclusive end is shifted to its
    /// exclusive form (`b + 1`) so a single clamp path serves both, and the
    /// result is clamped into range.  This keeps a vector slice's iteration
    /// endpoints in bounds so it never runs off either side — see loft#384.
    /// `len_var` must already hold `OpLengthVector(base)`; `bound` should be a
    /// `Var` so the duplicated reads below cost nothing and re-run no effects.
    fn slice_clamp_bound(&mut self, bound: Value, len_var: u16, inclusive_end: bool) -> Value {
        // from-end: (b < 0) ? b + len : b
        let is_neg = self.conv_op("<", bound.clone(), Value::Int(0), I32.clone(), I32.clone());
        let add_len = self.conv_op(
            "+",
            bound.clone(),
            Value::Var(len_var),
            I32.clone(),
            I32.clone(),
        );
        let resolved = v_if(is_neg, add_len, bound);
        let resolved = if inclusive_end {
            self.conv_op("+", resolved, Value::Int(1), I32.clone(), I32.clone())
        } else {
            resolved
        };
        // clamp high to len: (len < r) ? len : r   (loft has `<`/`<=`, not `>`)
        let gt_len = self.conv_op(
            "<",
            Value::Var(len_var),
            resolved.clone(),
            I32.clone(),
            I32.clone(),
        );
        let hi = v_if(gt_len, Value::Var(len_var), resolved);
        // clamp low: (r < 0) ? 0 : r
        let lt_zero = self.conv_op("<", hi.clone(), Value::Int(0), I32.clone(), I32.clone());
        v_if(lt_zero, Value::Int(0), hi)
    }

    // range ::= rev(<expr> '..' ['='] <expr>) | <expr> [ '..' ['='] <expr> ]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_in_range_body(
        &mut self,
        expr: &mut Value,
        data: &Value,
        name: &str,
        in_type: Type,
        reverse: bool,
    ) -> Type {
        let mut incl = self.lexer.has_token("=");
        // O8.5: capture range bounds for const-unroll detection.
        self.last_range_from = Some(expr.clone());
        let mut till = Value::Null;
        let mut till_tp = if self.lexer.peek_token("]") {
            till = if *data == Value::Null {
                Value::Int(i32::MAX)
            } else {
                self.cl("OpLengthVector", std::slice::from_ref(data))
            };
            in_type.clone()
        } else {
            self.expression(&mut till)
        };
        // O8.5: store till value (adjusted for inclusive ranges).
        if incl {
            // 0..=9 means till is 9, but the range includes 9.
            // Store till+1 so the unroller can use from..till_exclusive.
            if let Value::Int(t) = &till {
                self.last_range_till = Some(Value::Int(t + 1));
            } else {
                self.last_range_till = None;
            }
        } else {
            self.last_range_till = Some(till.clone());
        }
        // @PLN102 strict-index lint — for a pure exclusive `for i in 0..len(X)` range, record
        // X's `VecKey` on the current loop so a mismatched-vector index (`w[i]` with `w != X`)
        // can be flagged under `LOFT_LINT_STRICT_INDEX`. Only a bare `len(<addressable vector>)`
        // upper bound qualifies; `0..n`, `0..=len(X)` (inclusive), and slices do not.
        // `len(v)` stays as the `len` builtin call at parse time; `LengthVector` is the internal
        // slice-bound form. Match both, mirroring the bounds-proof recogniser in `operators.rs`
        // (`matches!(name, "len" | "LengthVector")`).
        // A bound HELD IN A LOCAL (`n = len(s); for i in 0..n`) counts as the same
        // bound: `len_bound_locals` carries `len(X)` forward from the assignment.
        // That form is not a stylistic variant — it is the one the published `cbor`
        // encoder shipped, so a lint that only sees the inline `0..len(X)` would have
        // missed the real bug.
        if *data == Value::Null && !incl {
            let vk = match till.unspan() {
                Value::Call(d, largs)
                    if matches!(
                        self.data.def(*d).original_name().as_str(),
                        "len" | "LengthVector"
                    ) && largs.len() == 1 =>
                {
                    crate::parser::operators::vec_key(&largs[0], &self.data)
                }
                Value::Var(n) => self.len_bound_locals.get(n).copied(),
                _ => None,
            };
            if let Some(vk) = vk {
                self.vars.set_loop_len_bound(vk);
            }
        }
        // loft#384: a vector slice (`data` present, not a pure `0..n` range) must
        // resolve negative bounds from the end and clamp into `[0, len]`, else the
        // iteration endpoints run off an edge: a negative end breaks immediately
        // (silent empty), a negative start wraps past the end, and an over-range
        // end reads OOB nulls/garbage in raw iteration.  Both bounds are bound to
        // temps (evaluated once) and clamped; the slice then runs as a plain
        // exclusive range, so pure ranges keep their raw bounds untouched.
        let mut iter_prelude = Vec::new();
        if *data != Value::Null {
            let len_var = self.create_unique("slice_len", &I32);
            let lo_var = self.create_unique("slice_lo", &I32);
            let hi_var = self.create_unique("slice_hi", &I32);
            iter_prelude.push(v_set(
                len_var,
                self.cl("OpLengthVector", std::slice::from_ref(data)),
            ));
            iter_prelude.push(v_set(lo_var, expr.clone()));
            let lo = self.slice_clamp_bound(Value::Var(lo_var), len_var, false);
            iter_prelude.push(v_set(lo_var, lo));
            iter_prelude.push(v_set(hi_var, till.clone()));
            let hi = self.slice_clamp_bound(Value::Var(hi_var), len_var, incl);
            iter_prelude.push(v_set(hi_var, hi));
            *expr = Value::Var(lo_var);
            till = Value::Var(hi_var);
            till_tp = I32.clone();
            incl = false;
        }
        // The loop counter is named after the binder — which COLLIDES for `_`, the one
        // binder people write more than once in a function.  Two `for _ in 0..n` loops
        // both bind `_#index`, so the inner loop's counter IS the outer's: the inner
        // runs to its end, the outer sees an exhausted counter and stops after ONE
        // iteration.  Silently: `for _ in 0..3 { for _ in 0..4 { … } }` counted 4
        // instead of 12 on both backends, and a two-layer world file saved correctly
        // then loaded back with one layer, reporting success (moros H5 / H8).
        //
        // `_` is exempt from the C61 nested-same-name guard in `parse_for_iter_setup`
        // — it has to be, since `_` must work across different element types in one
        // function — so nothing else was left to catch this.
        //
        // Give each `_` loop its own counter, the same way `$` already does. The
        // VISIBLE binding is untouched, so a body that reads `_` is unaffected; only
        // the hidden companion becomes distinct.
        let ivar = if name == "$" || name == "_" {
            self.create_unique("index", &in_type.clone())
        } else {
            self.create_var(&format!("{name}#index"), &in_type)
        };
        let mut ls = Vec::new();
        // A `rev(...)` wrapping a slice subscript (`rev(v[2..5])`) arrives via the
        // reverse_iterator flag: the inner subscript parse never sees the `rev`
        // token, so `reverse` (the param) is false here.  Honour the flag for the
        // loop direction and consume it.  The closing `)` for that form is consumed
        // by the enclosing `parse_in_range`, so only the `rev(range)` param drives
        // the `token(")")` below — leave that gated on `reverse`.
        let want_reverse = reverse || self.reverse_iterator;
        self.reverse_iterator = false;
        let test = if want_reverse {
            if incl {
                ls.push(v_set(
                    ivar,
                    v_if(
                        self.single_op("!", Value::Var(ivar), in_type.clone()),
                        till,
                        self.conv_op(
                            "-",
                            Value::Var(ivar),
                            Value::Int(1),
                            in_type.clone(),
                            I32.clone(),
                        ),
                    ),
                ));
            } else {
                ls.push(v_if(
                    self.single_op("!", Value::Var(ivar), in_type.clone()),
                    v_set(ivar, till),
                    Value::Null,
                ));
                ls.push(v_set(
                    ivar,
                    self.conv_op(
                        "-",
                        Value::Var(ivar),
                        Value::Int(1),
                        in_type.clone(),
                        I32.clone(),
                    ),
                ));
            }
            self.conv_op(
                "<",
                Value::Var(ivar),
                expr.clone(),
                in_type.clone(),
                till_tp,
            )
        } else {
            ls.push(v_set(
                ivar,
                v_if(
                    self.single_op("!", Value::Var(ivar), in_type.clone()),
                    expr.clone(),
                    self.conv_op(
                        "+",
                        Value::Var(ivar),
                        Value::Int(1),
                        in_type.clone(),
                        I32.clone(),
                    ),
                ),
            ));
            self.conv_op(
                if incl { "<" } else { "<=" },
                till,
                Value::Var(ivar),
                till_tp,
                in_type.clone(),
            )
        };
        ls.push(v_if(test, Value::Break(0), Value::Null));
        ls.push(Value::Var(ivar));
        // The loop init runs once before iteration.  For a slice it carries the
        // bound-clamp prelude (len/lo/hi temps) ahead of the iterator-var reset;
        // `iterator()` keeps this init slot (it drops only `extra_init`), so the
        // clamp is emitted on both the for-loop and the materialisation paths.
        let init_ivar = v_set(ivar, self.null(&in_type));
        let iter_init = if iter_prelude.is_empty() {
            init_ivar
        } else {
            // `Insert` is a flat statement sequence, NOT a scoped block: the clamp
            // temps must live across the loop body, so they cannot sit in a nested
            // block scope (its slots are reclaimed on block exit — two-zone design).
            iter_prelude.push(init_ivar);
            Value::Insert(iter_prelude)
        };
        *expr = Value::Iter(
            u16::MAX,
            Box::new(iter_init),
            Box::new(v_block(ls, in_type.clone(), "Iter range")),
            Box::new(Value::Null),
        );
        if reverse {
            self.lexer.token(")");
            self.reverse_iterator = false;
        }
        Type::Iterator(Box::new(in_type), Box::new(Type::Null))
    }

    pub(crate) fn parse_in_range(&mut self, expr: &mut Value, data: &Value, name: &str) -> Type {
        let mut reverse = false;
        if let LexItem::Identifier(rev) = self.lexer.peek().has
            && &rev == "rev"
        {
            self.lexer.has_identifier();
            self.lexer.token("(");
            reverse = true;
            // set the reverse flag BEFORE parsing the inner expression so that
            // rev(col[lo..hi]) passes the flag through parse_key → fill_iter.
            self.reverse_iterator = true;
        }
        // D-key-1: mark that we are parsing the *iterable* of a `for`/comprehension, so a
        // keyed range / partial-key subscript in this position is legitimate (it produces a
        // `for`-only `Value::Iter`).  Restored immediately after, so the loop BODY parsed
        // later by the caller sees it false — `x = coll[lo..hi]` in a body is still rejected.
        let prev_iterable_context = self.iterable_context;
        self.iterable_context = true;
        let in_type = if self.lexer.peek_token("..") || self.lexer.peek_token("..=") {
            // Open-start range: treat missing start as 0.
            *expr = Value::Int(0);
            I32.clone()
        } else {
            self.expression(expr)
        };
        self.iterable_context = prev_iterable_context;
        if !self.lexer.has_token("..") {
            if reverse {
                // if the inner expression was a subscript that already produced
                // a range iterator (parse_key consumed the `..`), the Value::Iter is
                // ready with the reverse flag — just consume ')' and return.
                if matches!(expr, Value::Iter(_, _, _, _)) {
                    self.lexer.token(")");
                    self.reverse_iterator = false;
                    return in_type;
                }
                // rev() wrapping a bare collection (not a range subscript).
                if matches!(
                    in_type,
                    Type::Sorted(_, _, _) | Type::Index(_, _, _) | Type::Vector(_, _)
                ) {
                    // reverse_iterator stays set; consumed and reset by iterator()
                } else if !matches!(in_type, Type::Null) {
                    self.reverse_iterator = false;
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "rev() on a non-range expression must wrap a sorted, index, or vector collection"
                    );
                }
                self.lexer.token(")");
            }
            return in_type;
        }
        self.parse_in_range_body(expr, data, name, in_type, reverse)
    }

    pub(crate) fn parse_object_field(
        &mut self,
        td_nr: u32,
        code: &mut Value,
        list: &mut Vec<Value>,
        found_fields: &mut HashSet<String>,
        in_place_var: Option<u16>,
        sinks: &mut FieldSinks,
    ) -> bool {
        // Accept both bare identifiers and JSON-style quoted strings as field names.
        let field = if let Some(id) = self.lexer.has_identifier() {
            id
        } else if let Some(s) = self.lexer.has_cstring() {
            s
        } else {
            return false;
        };
        if !self.lexer.has_token(":") {
            return false;
        }
        let nr = self.data.attr(td_nr, &field);
        if nr == usize::MAX {
            if let Some(s) = self.suggest_field_name(td_nr, &field) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown field {}.{field} — did you mean '{s}'?",
                    self.data.def(td_nr).name()
                );
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown field {}.{field}",
                    self.data.def(td_nr).name()
                );
            }
            // Recover past the field's `: value` so the parser continues at the
            // next `,`/`}` instead of choking on the orphaned value and cascading
            // ("Expect token }" / "Expect token ;").  Only runs on an already-
            // errored unknown field, so valid literals are unaffected.
            let mut discard = Value::Null;
            self.expression(&mut discard);
        } else {
            let td = self.data.attr_type(td_nr, nr);
            // @PLN25 — whether the field carries a record-pointer HEADER is a question
            // about its storage, and `Optional(τ)` shares τ's storage exactly.  Peel the
            // marker for that question only; the checks further down (`n_store_violation`,
            // `convert`, the sentinel hint) read `td` itself, because those ARE about
            // nullability.  Unpeeled, a nullable collection field was not recognised as one:
            // it built through a standalone temp instead of in place, and that temp — minted
            // with a dep on the struct it sits in — skipped the `vector_db` that would have
            // defined it, so it reached codegen with no stack slot at all (loft#909).
            let td_base = td.base().clone();
            let pos = self.field_position(td_nr, &field);
            found_fields.insert(field.clone());
            // loft#926 — is this collection field being given records, or is it the
            // `field: []` that constructing a linked group writes for every member?  An
            // empty literal is exactly two tokens, so the question is answered by looking
            // at them and putting the cursor straight back.  Scoped to a collection field
            // with the advice enabled, so no other parse meets the lookahead.
            if crate::keys::linked_group_lint_enabled()
                && Self::collection_element(&td_base).is_some()
            {
                let before = self.lexer.link();
                let empty = self.lexer.has_token("[") && self.lexer.has_token("]");
                self.lexer.revert(before);
                if !empty {
                    sinks.filled_collections.insert(field.clone());
                }
            }
            let mut value = if let Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
            | Type::Enum(_, true, _)
            | Type::Index(_, _, _) = td_base
            {
                // Collection/enum-big header is a 4-byte u32 record pointer.
                // Post-2c `OpSetInt` writes 8 bytes and overflows the field.
                //
                // #437 — a VECTOR field's prime goes to `primes`, which the caller
                // splices as one block directly after the `OpDatabase` prelude, so
                // every vector header is zeroed before ANY field's value is built.
                // Emitted inline it sat between the PREVIOUS field's fill and its
                // own, and a vector local whose literal build is retargeted INTO the
                // field (the elision that fires for a literal-initialised,
                // otherwise-unused local) then ran its fill BEFORE this zeroing —
                // erasing the header it had just written.  Only the first field
                // mentioned in the literal escaped, because the prelude it rides
                // along with is hoisted above the retargeted builds.
                //
                // KEYED collections (sorted / hash / index / radix) and enum-big keep
                // the inline prime: their fills are record INSERTS whose ordering
                // against a sibling's prime is load-bearing (hoisting them merged two
                // keyed fields' records — `502-keyed-slice-for-only`).  They also do
                // not take the retarget path, so #437 does not reach them.
                //
                // loft#924 — a member of a LINKED COLLECTION GROUP takes neither
                // route: `parse_object` zeroed the whole group's headers together,
                // before any member's fill, because an insert through any one of
                // them indexes the record into all of them.
                if !sinks.group_primed.contains(&pos) {
                    let prime = self.cl(
                        "OpSetInt4",
                        &[code.clone(), Value::Int(i32::from(pos)), Value::Int(0)],
                    );
                    if matches!(td_base, Type::Vector(_, _)) {
                        sinks.vector_headers.push(prime);
                    } else {
                        list.push(prime);
                    }
                }
                let info = self.type_info(&td);
                self.cl(
                    "OpGetField",
                    &[code.clone(), Value::Int(i32::from(pos)), info],
                )
            } else {
                Value::Null
            };
            let mut parent_tp = Type::Reference(td_nr, crate::data::Deps::none());
            // A `u16::MAX` destination is the "no slot" sentinel a file-scope
            // construction carries (`P p = P{}` at module scope); it has no frame
            // var to borrow from, and `depending` asserts on the marker — leave the
            // parent type independent rather than panicking.
            if let Value::Var(v) = code
                && *v != u16::MAX
            {
                parent_tp = parent_tp.depending(*v);
            }
            // Collection fields prime `value` with an in-field write target
            // (the literal writes THROUGH the field) — those must not be
            // hoisted; only pure value expressions (`value` started Null).
            let primed = !matches!(value, Value::Null);
            // `{}` is an empty Void BLOCK, not a collection literal — loft's empty
            // collection literal is `[]`.  For a collection field an empty `{}`
            // silently lowered to a 0-byte block that UNDER-FILLED the struct
            // record (a sibling field landed at the wrong offset → SIGSEGV /
            // use-after-free).  Accept it as the already-primed empty collection
            // (the `OpSetInt4(.., 0)` above zeroed the header) but steer toward
            // the canonical `[]`.
            // Filled by the brace scan below when `{}` is what stands here.
            let mut braces_span: Option<(u32, u32, u32)> = None;
            let empty_braces = crate::parser::vectors::is_collection(&td_base) && {
                let link = self.lexer.link();
                // loft#1003 — each brace's own position, taken before it is consumed, so the
                // fix can spell `{}` -> `[]` as an edit.  Both are needed: `{ }` is the same
                // construct with a gap, and a length assumed from the opener would leave the
                // `}` behind.
                let open = self.lexer.peek_pos().clone();
                let empty = self.lexer.has_token("{") && {
                    let close = self.lexer.peek_pos().clone();
                    self.lexer.has_token("}") && {
                        braces_span = (open.line == close.line && close.pos >= open.pos)
                            .then(|| (open.line, open.pos, close.pos + 1 - open.pos));
                        true
                    }
                };
                if !empty {
                    self.lexer.revert(link);
                }
                empty
            };
            let exp_tp = if empty_braces {
                // Pass 2 only: the field is parsed in both passes, so an ungated notice is
                // reported twice at one position — which reads as two findings and, now that
                // the fix spells an edit, offered the same rewrite twice.  Every other
                // deprecation-style notice gates the same way.
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        code = "empty-braces-not-collection",
                        "empty `{{}}` is not a collection literal"
                    );
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Mechanical,
                        title: "write `[]` for an empty collection".to_string(),
                        condition: None,
                        edit: braces_span.map(|(line, col, len)| crate::diagnostics::Edit {
                            line,
                            col,
                            len,
                            text: "[]".to_string(),
                        }),
                        concept: "vector",
                        concept_ref: "@F6",
                    });
                }

                td.clone()
            } else {
                // @PLN87 B-Ref-AnnotationOnly — a `reference<T>` FIELD is the third place a
                // prefix `&` legally annotates a binding: `Linked { link: &pool[i] }` stores
                // a cross-store pointer, which is what the field's type asks for.  (It is a
                // different type former from `&τ`, the stack link the rest of the rule is
                // about — see `Type::Reference` vs `Type::RefVar`.)  Open the head only for
                // that field type, so a `&` in a field of any OTHER type stays the
                // sub-expression use the rule forbids.
                //
                // Read through `base()`: a `?` on the field says the pointer may be
                // absent, not that the field stopped being a pointer — `@FR-L-Null`
                // gives `τ?` the same bytes as `τ`, so `reference<T>?` asks for the
                // same `&` binding `reference<T>` does.  Matching the unpeeled type
                // refused the terminator-carrying spelling of the very idiom this arm
                // exists for (loft#1316).
                self.amp_head = if matches!(td.base(), Type::Reference(_, _)) {
                    AmpHead::StoredRefField
                } else {
                    AmpHead::No
                };
                // loft#1067 — a field's DECLARED type is an inference context, so a short
                // lambda may stand as its value: `H { f: |x| { x * 2 } }` says exactly what
                // `takes(|x| { x * 2 })` says, and used to be refused only because the `⇐`
                // channel was never pushed here. `var_tp` (`td`, below) already carries the
                // type for everything that reads it, but `lambda_hint` reads the channel.
                let saved_expected = std::mem::replace(&mut self.expected, Type::Unknown(0));
                if Self::seeds_lambda_hint(&td) {
                    self.expected = td.clone();
                }
                let t = self.parse_operators(&td, &mut value, &mut parent_tp, 0);
                self.expected = saved_expected;
                self.amp_head = AmpHead::No;
                t
            };
            // #330: an initialiser that READS the in-place target is hoisted
            // into a typed temp; the temps run before the OpDatabase re-init
            // (spliced in parse_object), so they see the OLD record.
            if let Some(xv) = in_place_var
                && !primed
                && value.reads_var(xv)
            {
                let tmp = self.vars.work_refs(&exp_tp, &mut self.lexer);
                if !self.first_pass {
                    self.change_var_type(tmp, &exp_tp);
                }
                let prev = std::mem::replace(&mut value, Value::Var(tmp));
                sinks.hoists.push(v_set(tmp, prev));
            }
            if let Some(bulk) = self.handle_field(td_nr, code, list, &field, &mut value, &exp_tp) {
                sinks.group_fills.push(bulk);
            }
        }
        true
    }

    /// @PLN87 P2.1 — get-or-create the `__orig` witness for a rebindable heap
    /// parameter `param`.  The witness is a skip-free work-ref that will hold the
    /// param's caller-supplied DbRef (stashed at function entry in the parser
    /// preamble); `scopes::check` reads the recorded mapping to emit the
    /// function-exit `OpFreeRefIfDistinct(param, witness)`.  Idempotent — a param
    /// reassigned several times reuses one witness.  Final pass only.
    /// @PLN87 P2.1 — true if `v_nr` is a SYNTHETIC parameter: a `hidden`
    /// return-buffer the compiler promoted (e.g. an NRVO `result` out-param —
    /// user-named, so it is NOT `_`-prefixed, yet `hidden` in the attributes).
    /// Such a buffer MUST keep its in-place write — the caller receives the
    /// value through it — so it is never rebound.
    pub(crate) fn is_hidden_param(&self, v_nr: u16) -> bool {
        self.context != u32::MAX
            && self
                .data
                .def(self.context)
                .attributes()
                .iter()
                .any(|a| a.hidden && self.vars.var(&a.name) == v_nr)
    }

    pub(crate) fn ensure_rebind_witness(&mut self, param: u16) -> u16 {
        if let Some(orig) = self.vars.rebind_orig(param) {
            return orig;
        }
        let tp = self.vars.tp(param).clone();
        let orig = self.vars.work_refs(&tp, &mut self.lexer);
        // The witness only WITNESSES the caller's store (its store_nr is the
        // distinctness key) — it must never free it, so it is skip_free.  And it
        // never OWNS a store: mark it inline_ref so its entry null-init lowers to
        // `OpInitRefSentinel` (no allocation) rather than `OpInitRef` (which would
        // allocate a store the stash then orphans — a leak); the value is
        // supplied by the entry stash `Set(orig, param)` (a raw DbRef copy).
        self.vars.set_skip_free(orig);
        self.vars.mark_inline_ref(orig);
        self.vars.set_rebind_orig(param, orig);
        orig
    }

    pub(crate) fn parse_object(&mut self, td_nr: u32, code: &mut Value) -> Type {
        // @PLN25 single-payload: a `__nullable<S>::Some` variant's body uses S's field names,
        // which live in the inline `payload` field — not `Some`'s direct fields {enum, payload}.
        // Allocate the `Some` record, set the discriminant present, and parse the body as a
        // dense `S` directly into the inline `payload` sub-ref.  See single-payload-refactor.md.
        if self.data.def_type(td_nr) == DefType::EnumValue {
            let payload_attr = self.data.attr(td_nr, "payload");
            let parent = self.data.def(td_nr).parent;
            if payload_attr != usize::MAX
                && parent != u32::MAX
                && self.data.def(parent).name.starts_with("__nullable<")
                && let Type::Reference(struct_d, _) = self.data.attr_type(td_nr, payload_attr)
            {
                return self.parse_some_payload_object(parent, td_nr, struct_d, code);
            }
        }
        let link = self.lexer.link();
        if !self.lexer.has_token("{") {
            self.lexer.revert(link);
            return Type::Unknown(0);
        }
        // The omitted-field advice is only decidable once the whole body is read, and by then
        // the cursor sits past the closing `}` — on the next statement for a one-line literal.
        // Keep the opening brace's position to point the caret at the literal it names
        // (DIAGNOSTICS.md § Adding a code, step 4).
        let literal_pos = self.lexer.pos().clone();
        let mut list = Vec::new();
        let mut new_object = false;
        let mut in_place_var: Option<u16> = None;
        let mut sinks = FieldSinks::default();
        let work = self.vars.work_ref();
        // Both sequences: the construction arms below mint from the pass-2-only
        // `__ref_p2_N` one (loft#848), and an abandoned construction must clean
        // whichever it took.
        let work_p2 = self.vars.work_ref_p2();
        // `code` arriving as `Value::Var(dest)` is the assignment's destination HINT — build
        // here instead of into a temp.  It is valid for `m = S { … }`, where the literal IS the
        // whole right-hand side, and invalid the moment the parser descends into a
        // sub-expression: the hint is threaded down as one `&mut Value` and nothing clears it
        // on the way (loft#1304).
        //
        // A unary prefix operator is the descent this DOES cover — `Parser::prefix_operand` is
        // set across `-x` / `!x` / `~x`'s operand parse, so `m = -S { … }` no longer builds the
        // literal into the variable that receives the NEGATION's result.
        //
        // ⚠ The POSTFIX descent (`m = S { … }.f(…)`, `m = S { … } + S { … }`) is NOT covered
        // and stays open on loft#1304.  Two cures were built and measured, and both are
        // recorded on the issue: a look-ahead past the balanced body is NOT transparent —
        // `Lexer::revert` replays tokens but its closing `cont()` resets `prev_end` from
        // wherever the walk stopped, and three parse-error baselines moved a column even when
        // the answer was DISCARDED — and declining the hint outright loses the `&`-link
        // reshape refusal, which is derived from the in-place construction.
        let hint_is_the_whole_value = !self.prefix_operand && !self.inplace_hint_declined;
        if let Value::Var(v_nr) = code
            && hint_is_the_whole_value
        {
            let var_tp = self.vars.tp(*v_nr).clone();
            let type_matches =
                var_tp.is_unknown() || matches!(&var_tp, Type::Reference(d, _) if *d == td_nr);
            // loft#660 — a vector-literal ELEMENT alias is never an in-place
            // allocation target.  Its storage is the slot `OpNewRecord` already
            // carved out of the container, so re-allocating it here (`OpDatabase`)
            // hands the field initialisers a DIFFERENT record and leaves the slot
            // holding whatever the surrounding writes put there: `OpFinishRecord`
            // then commits the wrong record — the element vanishes (length 0), or
            // its payload lands on an outer struct's field, or the read walks a
            // bogus rec-id and SIGSEGVs.
            //
            // Ownership is the invariant, not the presence of a dep: an element
            // whose container is a field DbRef (a vector inside an enum payload) has
            // no container VARIABLE to depend on, so a dep-only test read it as
            // owning.  `owns_store` is the one predicate that answers it, shared with
            // `generation::dispatch` so parser and codegen cannot drift (loft#664).
            if self.vars.owns_store(*v_nr) && type_matches {
                // #330: remember the in-place target — a field initialiser
                // that READS it must be hoisted ABOVE the OpDatabase re-init
                // (see the hoist in parse_object_field and the splice after
                // the field loop), because the re-init clears the record
                // before the initialisers run.
                if !self.first_pass && !self.vars.is_compiler_generated(*v_nr) {
                    in_place_var = Some(*v_nr);
                }
                self.data.set_referenced(td_nr, self.context, Value::Null);
                let tp = i32::from(self.data.def(td_nr).known_type());
                // @PLN87 P2.1 — whole-binding reassignment locality.
                if !self.first_pass
                    && self.vars.is_argument(*v_nr)
                    && !self.vars.is_compiler_generated(*v_nr)
                    && !self.is_hidden_param(*v_nr)
                {
                    // A user-visible heap PARAM's slot ALIASES the caller's value;
                    // it does not own a store.  Reassigning it wholesale REBINDS
                    // locally: free a PRIOR rebind store but never the caller's
                    // original (`OpFreeRefIfDistinct` against the entry witness),
                    // null the slot WITHOUT freeing (`OpInitRefSentinel`), then
                    // `OpDatabase` allocates a FRESH store.  So `o = Obj{..}` no
                    // longer mutates the caller — reference-default still
                    // propagates field writes (`o.x = 9`); only whole-binding
                    // reassignment is local.  The witness is stashed at function
                    // entry and the rebound store freed at function exit (both in
                    // `scopes::check`).  A `&`/RefVar param never reaches here
                    // (`type_matches` is false for RefVar → temp + write-back,
                    // P2.2, already correct).
                    let orig = self.ensure_rebind_witness(*v_nr);
                    list.push(self.cl(
                        "OpFreeRefIfDistinct",
                        &[Value::Var(*v_nr), Value::Var(orig)],
                    ));
                    list.push(self.cl("OpInitRefSentinel", &[Value::Var(*v_nr)]));
                    // Enforces @FR-O-Detach: the detach lands AFTER the field initialisers
                    // have been hoisted into temporaries, so a field that reads the parameter
                    // (`p = S{x: p.x + 1}`) reads it while it is still intact.  Its twin for
                    // every other right-hand side is `expressions.rs::rebind_local_heap_param`,
                    // which lost that ordering and answered null (loft#1312).
                    //
                    // The literal builds IN PLACE, so the detach has to sit here, between
                    // the construction's own ops.  Tell `parse_assign_op` — which carries
                    // the same lowering for every OTHER right-hand side — that this
                    // statement already has one (loft#1290).
                    self.rebind_lowered = *v_nr;
                } else if !self.vars.is_argument(*v_nr) {
                    // A non-arg local OWNS its store; the `Set(Null)` lets the
                    // allocator reuse-or-fresh it in place (pre-existing).  A
                    // compiler-generated arg (NRVO hidden return-buffer) keeps the
                    // bare in-place `OpDatabase` reuse — writing the caller's
                    // buffer IS its purpose.
                    list.push(v_set(*v_nr, Value::Null));
                }
                list.push(self.cl("OpDatabase", &[Value::Var(*v_nr), Value::Int(tp)]));
            } else if (!type_matches
                || (!self.vars.is_independent(*v_nr) && !self.vars.is_compiler_generated(*v_nr)))
                && !self.first_pass
            {
                // Two shapes route here:
                // - LHS variable already has an incompatible type (e.g. integer from a
                //   prior pass) — `!type_matches`.
                // - LHS variable is a user-declared local whose type was inferred
                //   as dependent on some other variable (e.g. `x` had `x = bs[i]` in a
                //   later statement, giving x type `Reference(T, [bs])`), so
                //   `is_independent` returns false even though `type_matches` is true
                //   for this struct-literal assignment.  Without this branch, the
                //   in-place `v_set(x, Null) + OpDatabase` init above was skipped,
                //   leaving only field-init calls that wrote into uninitialised storage
                //   — the subsequent codegen then asserted
                //   `Incorrect var x[N] versus M` when x's slot was read later.
                //
                // The `is_compiler_generated` guard excludes internal aliases like
                // `_elm_N` / `__ref_N` / `_vector_N` (created by parser helpers via
                // `Function::unique`) whose storage is already allocated by the
                // enclosing vector slot or struct field — they correctly receive
                // field-inits without v_set/OpDatabase, so routing them through
                // new_object would break the aliasing and create an orphan allocation.
                //
                // Falls through to new_object so the struct gets a fresh work ref and
                // the result is a proper Value::Block — not a Value::Insert — which can be
                // used safely as a method-call argument.
                new_object = true;
                self.data.set_referenced(td_nr, self.context, Value::Null);
                let ret = self.data.def(td_nr).returned();
                // This arm is PASS-2-ONLY (its own `!self.first_pass` guard) and it builds
                // a value bound to a NAMED LOCAL, which outlives the statement — so it
                // draws from the `__ref_p2_N` sequence rather than the shared one.  On the
                // shared counter its position shifted relative to pass 1 and it was handed
                // the name pass 1 left on the return buffer, so `v: E = EA { … }` built the
                // variant INTO the buffer the return re-mints, and the return copied from
                // the destroyed store: `null` (loft#848).  See `Vars::work_ref_p2`.
                let w = self.vars.work_refs_p2(ret, &mut self.lexer);
                let tp = i32::from(self.data.def(td_nr).known_type());
                list.push(v_set(w, Value::Null));
                list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]));
                // @PLN87 P2.2 — `&`-param write-back (`o = Obj{..}` on a `&Obj`):
                // the new value `w` is TRANSFERRED to the caller through the
                // double-indirect `o` (codegen's RefVar-set lowering frees the
                // displaced caller store first), so the caller owns it and frees
                // it — `w` must NOT be freed here (skip_free), else the temp's
                // scope-exit free would orphan the caller's binding (a UAF) AND
                // leave the OLD caller store unfreed (the pre-P2.2 leak).
                if matches!(self.vars.tp(*v_nr), Type::RefVar(_)) {
                    self.vars.set_skip_free(w);
                }
                *code = Value::Var(w);
            }
        } else if !self.first_pass && !self.is_field(code) && !self.is_captured_dbref(code) {
            new_object = true;
            self.data.set_referenced(td_nr, self.context, Value::Null);
            let ret = self.data.def(td_nr).returned();
            // loft#1078 — the SECOND pass-2-only arm of this same function, and it kept the
            // shared counter after loft#848 moved its sibling above off it.  Same failure,
            // one arm over: `fn pick(c) -> S { w = S{a:7}; r = if c { S{a:9} } else { w }; r }`
            // mints nothing here on pass 1 (the view-return materialiser takes `__ref_1` and
            // `ref_return` renames it onto the return buffer), so on pass 2 this literal is
            // handed the name pass 1 left on that buffer — `return_buffer()` resolves the
            // buffer BY NAME, so the arm's record and the return destination became one
            // slot.  The return then re-mints it with `OpDatabase` before copying, and the
            // fresh arm answered `0` on BOTH backends while the borrow arm was correct.
            // A pass-2-only mint site draws from the pass-2 sequence — see
            // `Vars::work_ref_p2`.
            let w = if crate::keys::p2_object_workref_enabled() {
                self.vars.work_refs_p2(ret, &mut self.lexer)
            } else {
                self.vars.work_refs(ret, &mut self.lexer)
            };
            let tp = i32::from(self.data.def(td_nr).known_type());
            list.push(v_set(w, Value::Null));
            list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]));
            *code = Value::Var(w);
        }
        // @PLN93 (#511): a captured-collection append target (`h += K{…}` inside a closure,
        // where `code` is the `OpGetDbRef` of the closure-record field) is a DbRef lvalue like
        // a struct field — skip the fresh-`Object` allocation above so the field inits build a
        // `Value::Insert` targeting the captured DbRef.  `new_record` then allocates the element
        // INSIDE the shared store (`OpNewRecord`/`OpFinishRecord` against that DbRef) rather than
        // in a throwaway store that is immediately freed (the silent no-op this fixes).
        let mut found_fields = HashSet::new();
        // #437 — everything pushed so far is the record-creation prelude
        // (`Set(x, Null)` + `OpDatabase`).  Vector-field headers are zeroed as one
        // block right after it, before any field value; see the note in
        // `parse_object_field`.
        let prelude_len = list.len();
        // loft#924 — a LINKED COLLECTION GROUP's headers are zeroed as one block
        // here, whether or not the literal names the field, because every member
        // shares one record set: `OpFinishRecord` through the field the author
        // wrote indexes the record into the siblings too, so a sibling primed
        // afterwards drops the spine it was just handed.  Doing it per field made
        // the literal's field ORDER decide which member could find the records.
        let mut group_headers = Vec::new();
        if !self.first_pass {
            sinks.group_primed = self.linked_group_offsets(td_nr);
            let mut offsets: Vec<u16> = sinks.group_primed.iter().copied().collect();
            // A `HashSet` iterates in an unspecified order and these ops land in
            // the emitted stream, so sort them: identical source must compile to
            // identical bytecode.
            offsets.sort_unstable();
            for off in offsets {
                group_headers.push(self.cl(
                    "OpSetInt4",
                    &[code.clone(), Value::Int(i32::from(off)), Value::Int(0)],
                ));
            }
        }
        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            if !self.parse_object_field(
                td_nr,
                code,
                &mut list,
                &mut found_fields,
                in_place_var,
                &mut sinks,
            ) {
                self.lexer.revert(link);
                self.vars.clean_work_refs(work);
                self.vars.clean_work_refs_p2(work_p2);
                return Type::Unknown(0);
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
        // Was the destination hint the right call?  Only now can it be asked: the hint is valid
        // for `m = S { … }`, where the literal IS the whole right-hand side, and a POSTFIX turns
        // it into a sub-expression — `m = S { … }.f(…)` built the receiver into the variable that
        // also receives the call's RESULT, so the return buffer overwrote the receiver's store
        // and the frame freed a reference that no longer described it (loft#1304).
        //
        // A single-token `peek_token` is transparent — it takes `&self` and reads the already
        // lexed token.  Walking PAST the body to ask the same question before the fact is not:
        // `Lexer::revert` restores `position` by replaying, but its closing `cont()` resets
        // `prev_end` from wherever the walk stopped, and three parse-error baselines moved a
        // column even with the answer discarded.
        //
        // So the answer arrives too late to have built differently, and the literal is parsed
        // AGAIN with the hint declined — through the same abandon path the field loop below
        // uses, which already reverts the lexer and returns the work-refs.  Declining outright
        // instead of retrying was measured and costs the `&`-link reshape refusal, which is
        // derived from the in-place construction.
        if let Some(v_nr) = in_place_var
            && !self.inplace_hint_declined
            && !(self.lexer.peek_token(";")
                || self.lexer.peek_token("}")
                || self.lexer.peek_token(",")
                || self.lexer.peek_token(")"))
        {
            self.lexer.revert(link);
            self.vars.clean_work_refs(work);
            self.vars.clean_work_refs_p2(work_p2);
            *code = Value::Var(v_nr);
            let outer = std::mem::replace(&mut self.inplace_hint_declined, true);
            let tp = self.parse_object(td_nr, code);
            self.inplace_hint_declined = outer;
            return tp;
        }
        // #437 splice: every vector-field header zeroed as one block, directly
        // after the prelude and before the first field's value.  loft#924's group
        // headers lead it — same position, same reason, and a member of a group is
        // primed only here.
        group_headers.append(&mut sinks.vector_headers);
        for (i, header) in group_headers.into_iter().enumerate() {
            list.insert(prelude_len + i, header);
        }
        // #330 splice: run the hoisted self-reading field values BEFORE the
        // in-place `Set(x, Null) + OpDatabase(x)` prelude clears the record.
        if !sinks.hoists.is_empty() {
            sinks.hoists.append(&mut list);
            list = std::mem::take(&mut sinks.hoists);
        }
        if !self.first_pass {
            self.warn_omitted_fields(td_nr, &found_fields, &literal_pos);
            self.advise_linked_group_fill(td_nr, &sinks.filled_collections, &literal_pos);
            let primed = std::mem::take(&mut sinks.group_primed);
            self.object_init(&mut list, td_nr, 0, code, &found_fields, &primed);
            // loft#1266 — the linked-group maintenance a bulk vector fill skips, emitted
            // once the whole body is read and AFTER `object_init`, which is the last thing
            // that can write a member's header.  A member this literal filled itself is
            // skipped: it owns its records, and indexing the group's into it releases them
            // out from under the member that holds them (measured on the loft#889 shape,
            // where only `LOFT_POISON=1` shows the read landing on freed bytes).
            let fills = std::mem::take(&mut sinks.group_fills);
            if !fills.is_empty() {
                let skip: std::collections::HashSet<u16> = sinks
                    .filled_collections
                    .iter()
                    .map(|f| self.field_position(td_nr, f))
                    .collect();
                let parent_ty = Type::Reference(td_nr, crate::data::Deps::none());
                for to in &fills {
                    let ops = self.keyed_sibling_view_fills(to, &parent_ty, &skip);
                    list.extend(ops);
                }
            }
            // emit all field constraint checks after construction completes.
            let assert_dnr = self.data.def_nr("n_assert");
            for a_nr in 0..self.data.def(td_nr).attributes().len() {
                let check = self.data.def(td_nr).attributes()[a_nr].check.clone();
                if check != Value::Null {
                    let bound = Self::replace_record_ref(check, code);
                    let nm = self.data.attr_name(td_nr, a_nr);
                    let msg = match &self.data.def(td_nr).attributes()[a_nr].check_message {
                        Value::Text(s) => Value::Text(s.clone()),
                        _ => Value::Text(format!(
                            "field constraint failed on {}.{nm}",
                            self.data.def(td_nr).name()
                        )),
                    };
                    let pos = self.lexer.pos();
                    list.push(Value::Call(
                        assert_dnr,
                        vec![
                            bound,
                            msg,
                            Value::Text(pos.file.clone()),
                            Value::Int(pos.line as i32),
                        ],
                    ));
                }
            }
        }
        if new_object && let Value::Var(v) = code {
            list.push(Value::Var(*v));
            *code = v_block(
                list,
                Type::Reference(td_nr, crate::data::Deps::frame1(*v)),
                "Object",
            );
            Type::Reference(td_nr, crate::data::Deps::none())
        } else {
            *code = Value::Insert(list);
            Type::Rewritten(Box::new(Type::Reference(td_nr, crate::data::Deps::none())))
        }
    }

    /// @PLN25 single-payload — construct a `__nullable<S>::Some` value from an `S{…}` (or
    /// anonymous `{…}`) literal: allocate the `Some` record, set the discriminant present,
    /// then parse the body as a dense `S` directly into the inline `payload` sub-ref.  The
    /// sub-ref is an `OpGetField`, so `parse_object(struct_d, …)` writes the body fields in
    /// place (its `is_field` path) and defaults the rest — exactly the dense-`S` construction,
    /// landing verbatim in the payload region.
    fn parse_some_payload_object(
        &mut self,
        syn: u32,
        some_d: u32,
        struct_d: u32,
        code: &mut Value,
    ) -> Type {
        // Peek the literal body without consuming it — parse_object(struct_d) consumes `{…}`.
        let link = self.lexer.link();
        if !self.lexer.has_token("{") {
            self.lexer.revert(link);
            return Type::Unknown(0);
        }
        self.lexer.revert(link);
        let enum_tp = Type::Enum(syn, true, crate::data::Deps::none());
        if self.first_pass {
            // Type-only: consume the body so the parser stays aligned.
            let mut throwaway = Value::Null;
            self.parse_object(struct_d, &mut throwaway);
            // …and leave a NON-NULL placeholder behind.  Pass 1 builds no IR here, so `code`
            // would keep the `Value::Null` its caller initialised it with — and a caller that
            // asks "is this operand the `null` LITERAL?" cannot tell that apart from "not
            // built yet".  `??`'s `?? null` soundness check asks exactly that, so on pass 1 it
            // read `v[i] ?? S { … }` as a nullable fallback and typed the result `τ?`, which
            // pass 2 could not take back: `s.x` then resolved against `__nullable<S>` and the
            // program was refused with a synthetic type name the author never wrote.
            *code = Value::Insert(Vec::new());
            return enum_tp;
        }
        let some_kt = self.data.def(some_d).known_type();
        let struct_kt = self.data.def(struct_d).known_type();
        let disc_pos = self.database.position(some_kt, "enum");
        let payload_pos = self.database.position(some_kt, "payload");
        // Allocate the `Some` record in a work-ref + set the discriminant present.
        let ret = self.data.def(some_d).returned().clone();
        let w = self.vars.work_refs(&ret, &mut self.lexer);
        let mut list = vec![
            v_set(w, Value::Null),
            self.cl(
                "OpDatabase",
                &[Value::Var(w), Value::Int(i32::from(some_kt))],
            ),
            self.cl(
                "OpSetEnum",
                &[
                    Value::Var(w),
                    Value::Int(i32::from(disc_pos)),
                    Value::Enum(2, u16::MAX),
                ],
            ),
        ];
        // Parse the body as a dense `S` directly into the inline payload sub-ref.
        let mut payload_ref = self.cl(
            "OpGetField",
            &[
                Value::Var(w),
                Value::Int(i32::from(payload_pos)),
                Value::Int(i32::from(struct_kt)),
            ],
        );
        self.parse_object(struct_d, &mut payload_ref);
        list.push(payload_ref);
        list.push(Value::Var(w));
        *code = v_block(
            list,
            Type::Enum(syn, true, crate::data::Deps::frame1(w)),
            "NullableSome",
        );
        enum_tp
    }

    /// Recursively replace `Value::Var(0)` (the record placeholder used in field default
    /// expressions) with the actual record reference from the calling context.
    pub(crate) fn replace_record_ref(mut val: Value, record: &Value) -> Value {
        val.map_nodes(&mut |n| {
            if matches!(n, Value::Var(0)) {
                *n = record.clone();
            }
        });
        val
    }

    /// Advice: this literal leaves a field out, so that field takes its type's zero.
    ///
    /// Reports the omission, not a fault — the zero is what an omitted field is documented to
    /// get. What it points at is the spelling that makes the omission SAFE: a declared field
    /// default (`palette_pick: integer = -1`), which is additive and costs existing callers
    /// nothing. See `keys::omitted_field_lint_enabled` for why this advises rather than warns,
    /// and for the shapes deliberately left quiet.
    ///
    /// Called from the literal path only. The synthesised whole-record constructions
    /// (`default_object`, the nested-`Reference` recursion, enum-variant init) reach
    /// `object_init` with no `found_fields` at all and were never written by an author, so
    /// keying on a NON-empty `found_fields` keeps them out on the same test that exempts a
    /// bare `S {}`.
    /// The element type a collection field holds records of, or `None` for a field that is
    /// not a collection.
    ///
    /// This is the key a linked group is formed on, so it has to name types the way
    /// `Stores::finish_type` does: a keyed kind carries its element as a definition number
    /// already, while a `vector` carries a whole `Type` and only a record element (a struct,
    /// or the `__nullable<S>` enum a nullable vector holds) can be shared with a sibling.
    fn collection_element(tp: &Type) -> Option<u32> {
        // Peeled, because a member's own `?` is not part of what the group is: `Optional(τ)`
        // is τ's slot plus a compile-time bit (@FR-L-Null) and the group forms at runtime for
        // a nullable member exactly as for a dense one.  Its sibling test
        // `is_keyed_collection` peels through `is_keyed`, so a bare match here made the two
        // halves of one home disagree — a `hash<S[k]>?` was dropped before `keyed` was even
        // consulted, and both advices went silent on a group that really exists.
        match tp.base() {
            Type::Sorted(e, _, _)
            | Type::Index(e, _, _)
            | Type::Hash(e, _, _)
            | Type::Radix(e, _, _)
            | Type::Trie(e, _, _) => Some(*e),
            Type::Vector(inner, _) => match inner.as_ref() {
                Type::Reference(d, _) | Type::Enum(d, true, _) => Some(*d),
                _ => None,
            },
            _ => None,
        }
    }

    /// Whether this collection kind is KEYED, which is what makes a group form at all — a
    /// pair of plain `vector` fields over one element type stays two collections.
    fn is_keyed_collection(tp: &Type) -> bool {
        crate::parser::vectors::is_keyed(tp)
    }

    /// The linked collection GROUPS declared by struct/enum-value `td_nr` — every set of two
    /// or more collections over ONE element type of which at least one is keyed
    /// (@FR-Col-Group).  Members come back in DECLARATION order, each with the attribute
    /// index that fixes its place in the declaration.
    ///
    /// One home for "which fields are a group", so the two advices over that question — the
    /// literal that fills two members, and the declaration that spreads one out — cannot
    /// disagree about what a group is.  Pairs that are not a group (two plain vectors, two
    /// collections over different element types) never appear.
    pub(crate) fn collection_element_of(tp: &Type) -> Option<u32> {
        Self::collection_element(tp)
    }
    pub(crate) fn is_keyed_collection_of(tp: &Type) -> bool {
        Self::is_keyed_collection(tp)
    }

    pub(crate) fn collection_groups(&self, td_nr: u32) -> Vec<(u32, Vec<GroupMember>)> {
        let mut groups: Vec<(u32, Vec<GroupMember>)> = Vec::new();
        for a_nr in 0..self.data.attributes(td_nr) {
            let tp = self.data.attr_type(td_nr, a_nr);
            let Some(elem) = Self::collection_element(&tp) else {
                continue;
            };
            let entry = GroupMember {
                a_nr,
                name: self.data.attr_name(td_nr, a_nr),
                keyed: Self::is_keyed_collection(&tp),
            };
            match groups.iter_mut().find(|(e, _)| *e == elem) {
                Some((_, members)) => members.push(entry),
                None => groups.push((elem, vec![entry])),
            }
        }
        // A group needs a keyed member; two plain vectors over one element type are two
        // collections and always were.
        groups.retain(|(_, m)| m.len() >= 2 && m.iter().any(|x| x.keyed));
        groups
    }

    /// Advise when ONE literal fills two members of a linked collection group (loft#926).
    ///
    /// See [`crate::keys::linked_group_lint_enabled`] for why this fires at the literal
    /// rather than at the declaration, and why it advises rather than warns. The short of
    /// it: a declaration that forms a group is usually deliberate and fills one member, so
    /// speaking there would be noise on correct code; a literal handing each member its own
    /// records is the shape that only makes sense if the author thinks they are independent.
    ///
    /// That reasoning holds for a group written TOGETHER, which is the idiom. A group whose
    /// members are declared APART is the case it does not cover, and
    /// `Parser::advise_group_apart` speaks there — see
    /// [`crate::keys::group_apart_lint_enabled`].
    fn advise_linked_group_fill(
        &mut self,
        td_nr: u32,
        filled: &HashSet<String>,
        at: &crate::lexer::Position,
    ) {
        if self.default || filled.len() < 2 || !crate::keys::linked_group_lint_enabled() {
            return;
        }
        let groups = self.collection_groups(td_nr);
        for (_, members) in &groups {
            let given: Vec<&String> = members
                .iter()
                .map(|m| &m.name)
                .filter(|nm| filled.contains(*nm))
                .collect();
            let ([holder], rest) = given.split_at(1.min(given.len())) else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            let others = rest
                .iter()
                .map(|nm| format!("`{nm}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let (route, own, fills) = if rest.len() == 1 {
                ("is a second route", "a collection of its own", "both")
            } else {
                (
                    "are second routes",
                    "collections of their own",
                    "all of them",
                )
            };
            diagnostic_at!(
                self.lexer,
                at,
                Level::Advice,
                code = "linked-group-double-fill",
                "{others} {route} to `{holder}`'s records, not {own} — this literal fills \
                 {fills}, so one record set ends up holding everything they were given"
            );
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "give each field its own element type so they stay independent".to_string(),
                condition: Some("the fields were meant to be two separate collections".to_string()),
                edit: None,
                concept: "keyed collections",
                concept_ref: "@F7",
            });
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "fill the group once, through one member".to_string(),
                condition: Some(
                    "the fields were meant as two routes to one record set".to_string(),
                ),
                edit: None,
                concept: "keyed collections",
                concept_ref: "@F7",
            });
        }
    }

    fn warn_omitted_fields(
        &mut self,
        td_nr: u32,
        found_fields: &HashSet<String>,
        at: &crate::lexer::Position,
    ) {
        if self.default || found_fields.is_empty() || !crate::keys::omitted_field_lint_enabled() {
            return;
        }
        let mut omitted: Vec<String> = Vec::new();
        for a_nr in 0..self.data.attributes(td_nr) {
            let nm = self.data.attr_name(td_nr, a_nr);
            if found_fields.contains(&nm) {
                continue;
            }
            let attr = &self.data.def(td_nr).attributes()[a_nr];
            // A computed / constant / compiler-injected field is not the author's to write,
            // and one carrying a declared default is the author saying the omission is fine.
            if attr.constant || attr.hidden || attr.value != Value::Null {
                continue;
            }
            let tp = self.data.attr_type(td_nr, a_nr);
            if matches!(tp, Type::Routine(_)) {
                continue;
            }
            // A nullable field is exempt: absence is a value it can hold, and the author wrote
            // the `?` that says so. Both spellings reach here — the `Optional` marker, and the
            // synthetic `__nullable<S>` enum a nullable struct field is rewritten to.
            if matches!(tp, Type::Optional(_))
                || matches!(&tp, Type::Enum(e, true, _)
                    if self.data.def(*e).name.starts_with("__nullable<"))
            {
                continue;
            }
            // A POINTER field (`reference<T>`, the `u16::MAX` share marker) and a FN-REF field
            // are exempt for the same reason a nullable one is: their omitted default is a null
            // SENTINEL, not a zeroed record, so absence is what the declaration already promises
            // and a reader gets it — and for a fn-ref there is no other default to declare. An
            // INLINE `Reference` (a dense embedded struct) is a different thing and stays in
            // scope: omitting one does hand back a silently zeroed record.
            if matches!(&tp, Type::Reference(_, deps) if deps.contains(&u16::MAX))
                || matches!(tp, Type::Function(_, _, _))
            {
                continue;
            }
            // A collection or text field is exempt, and the reason is the FIX rather than the
            // hazard: their zero is the identity — empty — and the only default an author could
            // declare for one (`= []`, `= ""`) IS that zero, so the advice would resolve to a
            // no-op. A diagnostic whose cure changes nothing is worse than silence; it spends
            // the credibility that makes the other sites worth reading. The scalars kept below
            // are the ones where the zero is a real value of the domain and a different default
            // is expressible — `0` is a palette index, `false` is a choice.
            if matches!(
                tp.base(),
                Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Radix(_, _, _)
                    | Type::Trie(_, _, _)
                    | Type::Text(_)
            ) {
                continue;
            }
            omitted.push(nm);
        }
        if omitted.is_empty() {
            return;
        }
        let type_name = self.data.def(td_nr).name().to_string();
        let list = omitted.join("`, `");
        let plural = if omitted.len() == 1 { "" } else { "s" };
        let takes = if omitted.len() == 1 { "takes" } else { "take" };
        diagnostic_at!(
            self.lexer,
            at,
            Level::Advice,
            code = "omitted-field-zero",
            "`{type_name}` literal omits the field{plural} `{list}`, which {takes} the type's \
             zero — nothing in the declaration chose that value"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: "declare the field's default on the type (`palette_pick: integer = -1`)"
                .to_string(),
            condition: Some(
                "the zero is not what an omitting caller should get — adding a default is \
                 additive, so existing callers keep working"
                    .to_string(),
            ),
            edit: None,
            concept: "struct records",
            concept_ref: "@F12",
        });
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: "write the field at this literal".to_string(),
            condition: Some("only this one site wants a value other than the zero".to_string()),
            edit: None,
            concept: "struct records",
            concept_ref: "@F12",
        });
    }

    // fill the not mentioned fields with their default value
    /// loft#924 — the byte offsets of every field of `td_nr` that is a member of a
    /// LINKED COLLECTION GROUP (`vector<E>` beside `hash<E[k]>`, or any two keyed
    /// collections over one element type — DATABASE.md § Clearing one member).
    ///
    /// Their headers cannot be zeroed field-by-field. A group is several routes to
    /// ONE record set, so `OpFinishRecord` on any member indexes the record into
    /// every other member; a member whose 4-byte header is zeroed AFTER that
    /// insert loses the spine it was just given, and the records stay reachable
    /// only through whichever member the literal happened to write first. The
    /// caller primes all of them together, ahead of any fill, and the two sites
    /// that would otherwise prime one at a time skip what is listed here.
    ///
    /// Empty for a struct with no group, which leaves every other literal emitting
    /// exactly the ops it did before.
    fn linked_group_offsets(&mut self, td_nr: u32) -> HashSet<u16> {
        let mut out = HashSet::new();
        let struct_tp = self.data.def(td_nr).known_type();
        if struct_tp == u16::MAX {
            return out;
        }
        for aid in 0..self.data.attributes(td_nr) {
            // A group member is a collection field, so ask the schema only about
            // those — an offset read for a computed or non-stored field could
            // collide with a real field's and prime the wrong header.
            let tp = self.data.attr_type(td_nr, aid);
            if !Self::is_collection_type(tp.base()) {
                continue;
            }
            let nm = self.data.attr_name(td_nr, aid);
            let off = self.database.position(struct_tp, &nm);
            if self.database.keyed_field_is_linked(struct_tp, off) {
                out.insert(off);
            }
        }
        out
    }

    /// Write the reserved ABSENT record id into a collection field's 4-byte slot — the one
    /// way a collection field says *absent* rather than *empty*.
    ///
    /// Zero is the EMPTY collection, so a slot left at its zero-init cannot mean absence;
    /// `DbRef::ABSENT_REC` is the id reserved for it (loft#917), read raw by
    /// `vectors::is_absent_collection` and mapped back to `0` by every other reader.  One
    /// home, because the two spellings that must agree — `H { xs: null }` and a field
    /// declared `xs: τ? = null` that the literal OMITS — are written in different
    /// functions, and a marker only one of them wrote is exactly how the omitted spelling
    /// came to read back present-and-empty.
    pub(crate) fn mark_collection_absent(&mut self, code: &Value, item_pos: i32) -> Value {
        #[allow(clippy::cast_possible_wrap)]
        let absent = Value::Int(crate::keys::DbRef::ABSENT_REC as i32);
        self.cl("OpSetInt4", &[code.clone(), Value::Int(item_pos), absent])
    }

    pub(crate) fn object_init(
        &mut self,
        list: &mut Vec<Value>,
        td_nr: u32,
        pos: u16,
        code: &Value,
        found_fields: &HashSet<String>,
        group_primed: &HashSet<u16>,
    ) {
        for aid in 0..self.data.attributes(td_nr) {
            let tp = self.data.attr_type(td_nr, aid);
            let nm = self.data.attr_name(td_nr, aid);
            let fld = self
                .database
                .position(self.data.def(td_nr).known_type(), &nm);
            // Skip computed fields (not stored) and already-provided fields.
            if found_fields.contains(&nm)
                || matches!(tp, Type::Routine(_))
                || self.data.def(td_nr).attributes()[aid].constant
            {
                continue;
            }
            // @PLN25 E2a.4 — a synthetic nullable-struct enum field omitted from the
            // literal is left at its zero-init (discriminant 0 = null) by
            // OpDatabase/set_default_value.  The Reference-recursion / to_default paths
            // below would write inner-struct field defaults over the inline enum bytes
            // and corrupt the record; "absent" is exactly discriminant 0, so skip it.
            if matches!(&tp, Type::Enum(e, true, _) if self.data.def(*e).name.starts_with("__nullable<"))
            {
                continue;
            }
            let mut default = self.data.attr_value(td_nr, aid);
            // #697 — a COLLECTION field MENTIONED in the literal is primed first: the
            // mentioned-field path emits `OpSetInt4(pos, 0)` to zero the 4-byte header
            // before anything writes through it (see `sinks.vector_headers`).  A field
            // left to its DEFAULT never reached that code, so its header kept whatever
            // the record's bytes happened to hold and every later read followed a garbage
            // rec number — `Bag { … }` omitting one `vector<integer> = []` panicked on the
            // FIRST access to an unrelated field, with an index that changed run to run.
            //
            // Prime it here too, and note this covers keyed collections as well: a
            // `hash<T[k]> = []` failed identically, and `text` never did because it is not
            // header-shaped.
            if crate::parser::vectors::is_collection(&tp) {
                // loft#924 — a group member's header was already zeroed with its
                // siblings', ahead of every fill. Writing it again here is what made
                // an OMITTED view field lose the records the primary already holds:
                // `object_init` runs after the whole literal body.
                if !group_primed.contains(&(pos + fld)) {
                    let prime = self.cl(
                        "OpSetInt4",
                        &[
                            code.clone(),
                            Value::Int(i32::from(pos + fld)),
                            Value::Int(0),
                        ],
                    );
                    list.push(prime);
                }
                // An EMPTY collection default (`= []`) parses to `Insert([Null])`, and the
                // zeroed header above already IS the empty collection.  Letting it through
                // to `set_field_no_check` emitted `OpAppendVector(field, null)` — appending
                // the Null as an element, which is the garbage the reader then walked.
                if matches!(&default, Value::Insert(items)
                    if items.iter().all(|i| matches!(i, Value::Null)))
                {
                    continue;
                }
                // loft#924 — likewise for a group member with NO declared default:
                // the prelude's zeroing is its whole initialisation, and falling
                // through writes the zero a SECOND time (`set_field_no_check` stores
                // a keyed collection by writing its 4-byte header), now after the
                // siblings' records went in.  That is what left an OMITTED view
                // field empty while the primary held the records.
                if group_primed.contains(&(pos + fld)) && default == Value::Null {
                    continue;
                }
            }
            // loft#917's other half — an OMITTED nullable COLLECTION field.  The MENTIONED
            // spelling (`H { xs: null }`) writes the reserved absent id; omitting the field
            // fell through to the zero its type takes, and zero IS the empty collection —
            // the one value absence has to be distinguishable from.  So a field declared
            // `xs: vector<τ>? = null` read back present-and-empty and `xs == null` answered
            // false, with a `?? []` at every use site hiding it.
            //
            // @FR-L-Null: absence is a sentinel IN the field's bytes, so a nullable field's
            // zero is its null.  The synthetic `__nullable<S>` field is skipped further up
            // for the same rule with the opposite conclusion — ITS absence IS discriminant
            // zero, which the zero-init already writes.  Reads `tp.base()`, because the
            // shape that needs this is by definition the wrapped one.
            if !self.first_pass
                && matches!(&tp, Type::Optional(_))
                && crate::parser::vectors::is_collection(tp.base())
                && default == Value::Null
            {
                let mark = self.mark_collection_absent(code, i32::from(pos + fld));
                list.push(mark);
                continue;
            }
            // #328/#332: a POINTER field (`reference<T>`, the u16::MAX share
            // marker) is a 12-byte DbRef — its omitted default is the null
            // sentinel.  The inline recursion below would write the INNER
            // struct's field defaults over the DbRef bytes (it only looked
            // harmless while integer defaults were zeros).
            if let Type::Reference(_, deps) = &tp
                && deps.contains(&u16::MAX)
                && default == Value::Null
            {
                let sentinel = self.cl("OpNullRefSentinel", &[]);
                list.push(self.set_field_no_check(td_nr, aid, pos, code.clone(), sentinel));
                continue;
            }
            if let Type::Reference(tp, _) = tp
                && default == Value::Null
            {
                // The prelude primed THIS struct's group fields; an inline nested
                // struct's own fields are none of them, and nothing has filled them
                // yet, so they prime here as they always did.
                self.object_init(list, tp, pos + fld, code, &HashSet::new(), &HashSet::new());
                continue;
            } else if default == Value::Null {
                // @PLN116 — a BARE (non-`Optional`) enum field cannot be silently
                // zero-filled: an enum's 0 is its null/undefined value (variants are
                // 1-based), so filling a NON-null enum field with 0 puts null into a
                // non-null slot — a contradiction the null model otherwise forbids (a
                // scalar's 0 is a valid value; an enum's 0 is the absence of one).  The
                // record author must make an explicit choice — provide the field, give it
                // `= <variant>`, or type it `E?` (where null IS allowed).  So an OMITTED
                // bare enum field is a compile error.  The synthetic `__nullable<…>` field
                // was skipped above; a genuinely `Optional` enum field is `Optional(Enum)`
                // here (not `Enum`), so it null-fills correctly through `to_default` below.
                // This is the `S{}` half of the one `has_default` rule `x?` enforces.
                if let Type::Enum(e, _, _) = &tp
                    && !self.data.def(*e).name.starts_with("__")
                    && !self.first_pass
                {
                    let tn = tp.name(&self.data);
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "field `{nm}: {tn}` is an enum with no default — specify it in the \
                         constructor, give it `= <variant>`, or make it `{tn}?` (defaults null)"
                    );
                }
                // LOFT.md § constructors: an omitted field gets "the zero
                // value for its type" — numerics default to 0 (NOT null;
                // tests/scripts/06-structs.loft locks this).  Pointer
                // fields take the sentinel branch above: a pointer's zero
                // value IS null.
                default = to_default(&tp, &self.data);
            } else if matches!(&default, Value::Block(b) if b.name == "EnumUnitLit") {
                // @PLN22 Phase 1 — a mixed struct-enum unit-variant default
                // (`mode: Mode = Idle`) is a SELF-CONTAINED allocation block whose
                // `Var(0)` is its OWN work-ref, not the record placeholder.  Re-home
                // it to a FRESH work-ref in THIS construction context: leaving
                // `Var(0)` for the `replace_record_ref(_, code)` below would rewrite
                // it to the struct's own ref, so the default's `OpDatabase`
                // re-allocates the struct variable and clobbers already-written
                // sibling fields (the `Widget { color: Red }` corruption).  The
                // record is deep-copied into the field (set_field_no_check emits an
                // OpCopyRecord), so the temp is a SEPARATE store — leave it to be
                // freed at scope end (NOT skip_free, unlike the `s = Idle`
                // assignment case where the LHS owns the store directly).
                let fresh = self.vars.work_refs(&tp, &mut self.lexer);
                default = Self::replace_record_ref(default, &Value::Var(fresh));
            } else {
                default = Self::replace_record_ref(default, code);
            }
            // loft#698 — a default that CALLS something (the function a default needing a
            // temporary is lowered into, or a plain `= mk()`) is stored with its user
            // arguments only.  A call returning a heap value also takes a caller-allocated
            // return buffer, and "caller" is THIS frame, not the struct the default was
            // written in — so the hidden slots are filled here, once per construction site,
            // exactly as `patch_tret_call` fills them for a promoted return.  Left unfilled
            // the call reached codegen a parameter short and tripped its arity assert.
            if let Value::Call(d, args) = &default
                && args.len() < self.data.attributes(*d)
            {
                let (d, mut actual) = (*d, args.clone());
                let mut types = vec![Type::Unknown(0); actual.len()];
                self.add_defaults(d, &mut actual, &mut types);
                default = Value::Call(d, actual);
            }
            list.push(self.set_field_no_check(td_nr, aid, pos, code.clone(), default));
        }
    }

    /// @P308 — the specific keyed-collection db type id (the id
    /// `OpReplaceKeyed` / `copy_claims` need) to deep-copy a `hash`/`sorted`/
    /// `index` field from an expression, else `None` (the caller keeps the
    /// bare-push).  `spatial` is excluded (`copy_claims` unimplemented, per
    /// @P295).  Mirrors the keyed-LOCAL `keyed_kt` logic in
    /// `expressions.rs::parse_assign_op`.  (Sorted/index were briefly
    /// HASH-only while @P309 — a deep-copy data-loss/hang when `index<T>`
    /// grew the shared element struct — was open; now fixed in
    /// `copy_claims_array_body`.)
    ///
    /// Asks through `.base()`, the reading [`crate::parser::vectors::is_keyed`] already
    /// takes: a `hash<E[k]>?` field is stored as the hash it names plus one reserved null,
    /// so which keyed store it is does not depend on the wrapper.  Matched unpeeled, every
    /// nullable keyed field fell to the `None` arm, and the two callers that gate a WRITE on
    /// it emitted no write at all — `h.c += rows` on a `hash<E[k]>?` field silently added
    /// nothing where the dense twin added two, for all five keyed kinds and every non-literal
    /// source (loft#1207).
    pub(crate) fn keyed_field_kt(&mut self, td: &Type) -> Option<u16> {
        match td.base() {
            Type::Hash(d, key, _) => {
                let c = self.data.def(*d).known_type();
                (c != u16::MAX).then(|| self.database.hash(c, key))
            }
            Type::Sorted(d, key, _) => {
                let c = self.data.def(*d).known_type();
                (c != u16::MAX).then(|| self.database.sorted(c, key))
            }
            Type::Index(d, key, _) => {
                let c = self.data.def(*d).known_type();
                (c != u16::MAX).then(|| self.database.index(c, key))
            }
            // @PLN48 — a `spatial` STRUCT FIELD is keyed like the others; without this
            // arm `keyed_field_kt` returned None and the field construction crashed
            // (`record_new` resolved the wrong field index).
            Type::Radix(d, key, _) => {
                let c = self.data.def(*d).known_type();
                (c != u16::MAX).then(|| self.database.spatial(c, key))
            }
            Type::Trie(d, key, _) => {
                let c = self.data.def(*d).known_type();
                (c != u16::MAX).then(|| self.database.trie(c, key))
            }
            _ => None,
        }
    }

    // Eight with `self`, and the list is two groups that are already named: the FIELD being
    // handled (`td_nr` / `field` / `value` / `exp_tp`) and where its output goes (`code` /
    // `list` / `sinks`, of which `sinks` is itself the bundle for the literal-level
    // accumulators).  There is exactly ONE call site, so a further struct would be packed
    // once and unpacked once and name nothing that these parameters do not — the same trade
    // `tree::range_cursors` records for its own list.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_field(
        &mut self,
        td_nr: u32,
        code: &mut Value,
        list: &mut Vec<Value>,
        field: &str,
        value: &mut Value,
        exp_tp: &Type,
    ) -> Option<Value> {
        let nr = self.data.attr(td_nr, field);
        let td = self.data.attr_type(td_nr, nr);
        // @PLN25 — how a value REACHES the field (a deep copy for a collection, a plain
        // store otherwise) follows the field's storage, which `Optional(τ)` shares with τ.
        // The sibling classification in `parse_object_field` peels the same way; the
        // nullability checks at the bottom of this function keep `td` itself (loft#909).
        let td_base = td.base().clone();
        // @PLN25 — null-source convert: a nullable struct SOURCE (a call / variable
        // of type `Reference(S)`, possibly the null sentinel) assigned to a synthetic
        // `__nullable<S>` field.  Build the `Some` variant from the source when
        // present (discriminant 2 + per-field copy at the Some offsets); leave the
        // zero-init (discriminant 0 = null) when the source is null — NEVER
        // OpCopyRecord a null source (the crash the representation retires).  An
        // inline `S{…}` literal already took the Some-construction path in parse_var,
        // so this fires only for an expression source.
        // loft#896 — a LITERAL `null` in the constructor (`H { maybe: null }`).  The source is
        // null at COMPILE time, so there is nothing to test at runtime and no source record to
        // copy: write the absent discriminant directly, exactly as `obj.f = null` does.
        // Without this the field-store convert reached `Null` against `__nullable<S>` and
        // refused the one spelling the declaration exists to allow.  The value is in
        // `found_fields`, so `object_init` will not also default it.
        if !self.first_pass
            && matches!(value.unspan(), Value::Null)
            && let Type::Enum(syn, true, _) = &td
            && self.data.def(*syn).name.starts_with("__nullable<")
        {
            let syn = *syn;
            let enum_kt = i32::from(self.data.def(syn).known_type());
            let item_pos = i32::from(
                self.database
                    .position(self.data.def(td_nr).known_type(), field),
            );
            let field_ref = self.cl(
                "OpGetField",
                &[code.clone(), Value::Int(item_pos), Value::Int(enum_kt)],
            );
            let clear = self.build_nullable_set_null(syn, field_ref);
            list.push(clear);
            return None;
        }
        // loft#1071 — a literal `null` into a USER struct-enum field (`Box { s: null }`
        // where `s: Shape?`).  The sibling arm above answers it for the synthetic
        // `__nullable<S>`, whose absence is a discriminant; a user struct-enum has no
        // discriminant of its own in an inline slot — the slot IS a four-byte record
        // pointer, and `0` is what absent means there (it is what the field prime writes,
        // and what every keyed-collection and enum-big field starts as).
        //
        // Left to the generic path this emitted `OpCopyRecord(<the null>, <the field>)`:
        // a record COPY of the null's own record into the slot, which is not how absence
        // is spelled in four bytes.  Nothing observed it, because the null TEST for the
        // type was refused — so the write had never been right and could not be seen.
        // Write the pointer directly, the same shape and the same reason as the arm above.
        if !self.first_pass
            && self.is_null_source(value)
            && let Type::Enum(syn, true, _) = &td_base
            && !self.data.def(*syn).name.starts_with("__nullable<")
        {
            let item_pos = i32::from(
                self.database
                    .position(self.data.def(td_nr).known_type(), field),
            );
            list.push(self.cl(
                "OpSetInt4",
                &[code.clone(), Value::Int(item_pos), Value::Int(0)],
            ));
            return None;
        }
        // The source's own spelling decides nothing here: `S?`, a bare `S` and a bare `null`
        // all name the same dense payload, and `needs_nullable_wrap` is the one place that
        // knows it.  Asking for `Reference(S)` by hand instead missed the `Optional(Reference)`
        // spelling entirely — every value a function RETURNS as `S?`, and every local declared
        // `S?` — so the dense record went in untagged: the first field became the
        // discriminant, `s.a` answered `s.b`, and a runtime null overwrote nothing at all.
        if !self.first_pass
            && let Type::Enum(syn, true, _) = td.base()
            && self.needs_nullable_wrap(*syn, exp_tp)
        {
            let syn = *syn;
            let enum_kt = i32::from(self.data.def(syn).known_type());
            let item_pos = i32::from(
                self.database
                    .position(self.data.def(td_nr).known_type(), field),
            );
            let field_ref = self.cl(
                "OpGetField",
                &[code.clone(), Value::Int(item_pos), Value::Int(enum_kt)],
            );
            let write = self.emit_nullable_slot_write(syn, &field_ref, value.clone());
            list.extend(write);
            return None;
        }
        if crate::parser::vectors::is_collection(&td_base) {
            // loft#917 — `H { xs: null }` on a field declared `?`.  The header prime in
            // `parse_object` has already zeroed the slot, and zero is the EMPTY collection;
            // leaving it there is what made `xs: null` and `xs: []` byte-identical and
            // `xs == null` answer false forever.  Write the reserved absent id over it.
            //
            // The sibling of the `__nullable<S>` arm above, which does the same job for a
            // nullable STRUCT field (loft#896) — same question, different storage.  Gated on
            // the declared `?` for the reason given at `clear_vector_field_as`: without one,
            // the field's own type says it can never be absent.
            if !self.first_pass && matches!(td, Type::Optional(_)) && self.is_null_source(value) {
                let item_pos = i32::from(
                    self.database
                        .position(self.data.def(td_nr).known_type(), field),
                );
                let mark = self.mark_collection_absent(code, item_pos);
                list.push(mark);
                return None;
            }
            // Issue #120: for vector fields assigned from a bare variable
            // (e.g. `BigBox { data: d }`), parse_operators overwrites the
            // field ref with Var(d) — no copy operation is generated.
            // Emit OpAppendVector to deep-copy the source vector into the
            // struct's field so the data is independent of the source store.
            //
            // @FR-N-Store — this lowering never reaches `convert`, so it asks for itself:
            // `H { f: w[i] }` with `w[i]: vector<integer>?` was the one struct-field cell of
            // loft#1366's matrix that said nothing.
            self.n_store_violation(exp_tp, &td, "the field", None);
            //
            // the same holds for any non-Insert vector-typed
            // expression (e.g. `C { v: build() }` where `build` returns a
            // vector).  Before this was a plain push, which left the field
            // uninitialised.
            if let Type::Vector(ref content, _) = td_base {
                if !self.first_pass && !matches!(value, Value::Insert(_) | Value::Null) {
                    let pos = self
                        .database
                        .position(self.data.def(td_nr).known_type(), field);
                    // `vector_of` consults
                    // `Data::narrow_vector_content` and registers a
                    // narrow element type when the content is a narrow
                    // alias (i32, u8).  `elem_tp` is the narrow type
                    // (Parts::Byte / Parts::Int) when applicable.
                    let vec_tp = self.vector_of(content);
                    // #628 — `elem_tp` is the store type of ONE element: the stride
                    // `vector_add` copies by.  For a NESTED `vector<vector<T>>` field
                    // that element is the inner vector's own 4-byte handle row; for a
                    // scalar or narrow-scalar field it is that scalar's row.
                    // `append_elem_tp` derives both from the one shared resolver, the
                    // same id the `+=` append and the literal build pass to
                    // `record_new` — a separately re-derived id is how the copy came
                    // to stride 8 bytes over 4-byte handles, giving `Bag { a: v }` the
                    // right OUTER length with every inner row empty, and a SIGSEGV
                    // once the struct carried three such fields.
                    let elem_tp = self.append_elem_tp(content) as u16;
                    let field_ref = self.cl(
                        "OpGetField",
                        &[
                            code.clone(),
                            Value::Int(i32::from(pos)),
                            Value::Int(i32::from(vec_tp)),
                        ],
                    );
                    list.push(self.cl(
                        "OpAppendVector",
                        &[
                            field_ref.clone(),
                            value.clone(),
                            Value::Int(i32::from(elem_tp)),
                        ],
                    ));
                    // loft#1266 — this bulk write owes its linked group the maintenance it
                    // skips.  A record joining a group one at a time reaches
                    // `Stores::record_finish`, which walks `other_indexes` and puts it in
                    // every member; `OpAppendVector` moves them in bulk and reaches none of
                    // it, so the keyed views stayed empty while the vector held everything —
                    // `len` answering `0` and a lookup answering `null`, both legal readings
                    // of a group that is simply empty.  The assignment and append spellings
                    // gained this in loft#1152; the constructor is their sibling.
                    //
                    // Recorded rather than emitted, because which siblings are owed it is not
                    // decidable here: a member the SAME literal fills owns its own records and
                    // must be left alone, and a member later in the literal has not been seen
                    // yet.  `parse_object` knows both once the body is read.
                    return Some(field_ref);
                }
                list.push(value.clone());
            } else if let Some(kt) = self.keyed_field_kt(&td_base)
                && !self.first_pass
                && !matches!(value, Value::Insert(_) | Value::Null)
            {
                // @P308 — a keyed-collection field (hash/sorted/index)
                // initialised from an EXPRESSION (`F{ h: build() }` /
                // `F{ h: c }`) must be DEEP-COPIED into the field, exactly
                // like the Vector branch above deep-copies via OpAppendVector.
                // Before this it fell to the bare `list.push(value)` below,
                // which left the field empty (the value's result was dropped)
                // and leaked a call source's store.  `OpReplaceKeyed(src,
                // field_ref, kt)` = remove_claims(field) (no-op on the
                // zero-inited field) + copy_claims(src, field) — the same
                // deep-copy that runs when a struct with a keyed field is
                // copied.  The `0x8000` bit frees a fresh-storage call source.
                // An empty/literal `[]` keeps the bare push (field stays
                // empty — correct).  Radix is excluded by `keyed_field_kt`
                // (copy_claims panics for it, per @P295).
                let pos = self
                    .database
                    .position(self.data.def(td_nr).known_type(), field);
                let field_ref = self.cl(
                    "OpGetField",
                    &[
                        code.clone(),
                        Value::Int(i32::from(pos)),
                        Value::Int(i32::from(kt)),
                    ],
                );
                let tp_val = if self.is_struct_returning_call(value) {
                    i32::from(kt) | 0x8000
                } else {
                    i32::from(kt)
                };
                // loft#1266 — ask what the SOURCE is, the same question the field
                // ASSIGNMENT and the two append sites ask (`is_keyed` in
                // `parse_assign_op_inner` / `parse_assign_op`).  This one did not, so a
                // plain `vector<E>` reached `OpReplaceKeyed`, which hands the source to
                // `copy_claims` under the DESTINATION's type and walks a vector's storage
                // as if it were a hash / index / trie: `hash` found nothing, `index` and
                // `trie` found one node, and only `sorted` came out right — because a
                // sorted's own storage IS a sequential vector.  `H { a: rows() }` and
                // `h.a = rows()` name the same records, so the constructor owes the same
                // answer the assignment gives, and `OpFillKeyed` is that answer: every
                // record placed by its own key through `record_finish`.
                //
                // No clear precedes it here, unlike the assignment site: the field's
                // header prime in `parse_object_field` has already zeroed the slot (and
                // `group_primed` zeroed the whole linked group's), so there is nothing in
                // the destination to replace.
                if crate::parser::vectors::is_keyed(exp_tp) {
                    list.push(self.cl(
                        "OpReplaceKeyed",
                        &[value.clone(), field_ref, Value::Int(tp_val)],
                    ));
                } else {
                    let parent_ty = Type::Reference(td_nr, crate::data::Deps::none());
                    let (parent, parent_tp_id, field_nr) =
                        self.fill_keyed_site(&field_ref, &parent_ty, kt);
                    list.push(self.cl(
                        "OpFillKeyed",
                        &[
                            parent,
                            value.clone(),
                            Value::Int(tp_val),
                            Value::Int(i32::from(parent_tp_id)),
                            Value::Int(i32::from(field_nr)),
                        ],
                    ));
                }
            } else {
                list.push(value.clone());
            }
        } else if let Value::Insert(ops) = value {
            for o in ops {
                list.push(o.clone());
            }
        } else {
            // @P279 — when pass-1 sees an Unknown value being assigned
            // to a typed field, suppress the diagnostic.  Unknown
            // values in pass-1 almost always come from a forward fn
            // ref whose return type hasn't been registered yet;
            // pass-2 re-runs this check with all defs visible and
            // fires the diagnostic for any GENUINE mismatch.  Mirrors
            // the pass-1 tolerance in `field()` / `parse_index()`
            // that closed @P281 / @P278 (same architectural fix:
            // pass-1 mustn't emit errors pass-2 will naturally
            // resolve).  `set_field_no_check` still runs so codegen
            // stays consistent with pass-2.
            //
            // loft#661 — the same tolerance is owed to the FIELD side.  A field
            // declared with a type defined LATER in the file holds a
            // `Type::Unknown(stub)` for all of pass 1: `parse_field` stores the
            // type on pass 1 only, and stubs are not bound to their real defs
            // until `actual_types_deferred` runs at the END of that pass.  So a
            // correct `S { f: t }` compared `ref(T)` against `unknown(N)` here,
            // errored, and that pass-1 error ABORTED the run before pass 2 —
            // which re-checks against the resolved type and passes.  Two
            // mutually-referential types cannot dodge this by reordering:
            // whichever is declared first names the other before it exists.
            // loft#944 — `is_unknown()` answers for a bare `Unknown` (and a vector of one),
            // not for an unresolved member nested inside a wrapper, so a tuple FIELD
            // (`struct W { t: (integer, Q) }` with `Q` below) slipped past this very guard
            // and aborted in pass 1 with `(integer, unknown(0))` vs `(integer, unknown(708))`
            // — one type printed twice, because both spellings render the unresolved member
            // the same way.  Ask the recursive question the guard always meant.
            if !self.first_pass
                || !(crate::data::Data::type_has_unresolved(exp_tp)
                    || crate::data::Data::type_has_unresolved(&td))
            {
                // A FIELD STORE: a literal that fits the type but lands on the
                // reserved null sentinel of a nullable narrow field is rejected
                // here too (not just on `obj.f = …`), so `U8N { x: 255 }` doesn't
                // silently store null.  The sentinel reservation is store-only —
                // it is NOT applied to the `convert` type-fit (params/casts).
                let dst_name = self.int_type_name(&td);
                if let Some(hint) = self.nullable_sentinel_hint(value, &td, &dst_name) {
                    diagnostic!(self.lexer, Level::Error, "{hint}");
                } else if !self.convert_store(value, exp_tp, &td, "the field", None) {
                    // @FR-N-Store is asked inside the store face; this arm is the plain
                    // type mismatch.
                    // Plan-07 phase 6 (partial) — name the value side first
                    // ("cannot assign <got> to <expected>"), the field-type
                    // side last.  Old shape "Cannot write {field_type} on
                    // field {S}.{f}:{value_type}" used a colon that read
                    // as "field declared as <value_type>" — backwards.
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot assign {} to field {}.{field} of type {}",
                        exp_tp.show(&self.data, &self.vars),
                        self.data.def(td_nr).name(),
                        td.show(&self.data, &self.vars)
                    );
                }
            }
            list.push(self.set_field_no_check(td_nr, nr, 0, code.clone(), value.clone()));
        }
        None
    }

    pub(crate) fn parse_enum_field(
        &mut self,
        list: &mut Vec<Value>,
        into: Value,
        d_nr: u32,
        pos: u16,
        enum_nr: u8,
    ) {
        let e_nr = self
            .data
            .def_nr(&self.data.def(d_nr).attributes()[enum_nr as usize - 1].name);
        let tp = self.data.def(e_nr).returned().clone();
        let v = self.create_unique("enum", &tp);
        let mut cd = if pos != 0 {
            list.push(v_set(
                v,
                self.cl("OpGetField", &[into, Value::Int(i32::from(pos))]),
            ));
            Value::Var(v)
        } else {
            into.clone()
        };
        self.parse_object(e_nr, &mut cd);
        if let Value::Insert(ls) = &cd {
            for l in ls {
                list.push(l.clone());
            }
        }
    }
}

pub(crate) fn collection_groups_of(
    data: &crate::data::Data,
    td_nr: u32,
) -> Vec<(u32, Vec<GroupMember>)> {
    let mut groups: Vec<(u32, Vec<GroupMember>)> = Vec::new();
    for a_nr in 0..data.attributes(td_nr) {
        let tp = data.attr_type(td_nr, a_nr);
        let Some(elem) = Parser::collection_element_of(&tp) else {
            continue;
        };
        let entry = GroupMember {
            a_nr,
            name: data.attr_name(td_nr, a_nr),
            keyed: Parser::is_keyed_collection_of(&tp),
        };
        match groups.iter_mut().find(|(e, _)| *e == elem) {
            Some((_, members)) => members.push(entry),
            None => groups.push((elem, vec![entry])),
        }
    }
    groups.retain(|(_, m)| m.len() >= 2 && m.iter().any(|x| x.keyed));
    groups
}
