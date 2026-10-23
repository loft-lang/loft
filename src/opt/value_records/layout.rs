// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The value-record answer's types ([`ValueRecords`], [`ValueTuple`], the view-leaf plan)
//! and the layouts: which record types a tuple can carry, field by field, and with which
//! presence word when the record is nullable.

use crate::data::{Data, Type, Value};
use crate::database::Stores;
use crate::opt::body_writes::{
    Target, WriteCache, WriteSet, body_writes, fresh_rooted, rebound_vars, setter_target,
};
use crate::opt::loop_records::{IN_PLACE_SET_OPS, is_scalar};
use crate::opt::writes::{
    FUSABLE_PUSHES, call_writes_store, mint_path, native_op_is_store_free, plain_record_type,
    retbuf_only_writer,
};
use std::collections::{HashMap, HashSet};

/// @PLN157 § V-aa (`@FR-R-ValueRecord`) — the functions whose NO-HEAP RECORD result is
/// returned BY VALUE (a Rust tuple of its fields, in registers) instead of through a
/// return buffer, with the field offsets that map a `OpGetField` to a tuple index.
#[derive(Default)]
pub struct ValueRecords {
    /// Admitted functions → their record type.
    pub fns: HashMap<u32, u16>,
    /// `(record type, byte offset)` → tuple index, for the call site's field reads.
    pub index: HashMap<(u16, i64), usize>,
    /// Per RECORD TYPE carried as a tuple anywhere — an admitted function's result, a value
    /// local, a tuple parameter — its layout ([`ValueTuple`]).  ONE home: a layout is a
    /// fact about the type, and every function, local and parameter of that type reads it.
    pub types: HashMap<u16, ValueTuple>,
    /// `(R-ValueLocal)` (`@FR-R-ValueLocal`) — per function, the by-value parameters
    /// received as TUPLES: attribute index → the record type ([`TupleParams`]).
    pub params: TupleParams,
    /// @PLN164 C5 — per admitted function with a VIEW LEAF, the places its exits deliver a
    /// view of, which is what a call site must keep undisturbed ([`ViewPlan`]).  A function
    /// whose record owns no heap has no entry.
    pub views: HashMap<u32, ViewPlan>,
    /// @PLN164 C5 — per record type, the byte offsets of its view-leaf fields
    /// ([`ViewOffsets`]): what a call site may read off the tuple's reference.
    pub view_offs: ViewOffsets,
    /// `(R-FnRefValue)` (`@FR-R-FnRefValue`) — the fn-ref call sites that MAY answer a
    /// tuple ([`FnRefSites`]).  One does when every arm of its dispatch is in `fns` with
    /// the site's record ([`fnref_value`]).
    pub fnref_sites: FnRefSites,
    /// `@FR-N-Store` — the admitted DENSE functions whose tuple is SURELY PRESENT
    /// ([`surely_present_fns`](super::surely_present_fns)): every result is a build or another such function's.  Only
    /// their tuple may be widened with the constant presence word.
    pub sp: HashSet<u32>,
}

/// `(R-FnRefValue)` — per fn-ref call site, keyed `(function, fn-ref variable)`: every arm
/// its dispatch can reach in the whole program (`fnref::dispatch_arms`) and the record type
/// its signature returns, when that is a plain record.  Only a site whose signature returns
/// or takes a plain record, whose arms capture nothing and whose `&text` arguments pick no
/// store instance is listed; the arms of any other dispatch keep the buffer road outright.
pub type FnRefSites = HashMap<(u32, u16), (Vec<u32>, Option<u16>)>;

/// `(R-FnRefValue)` — the record type whose tuple a call through fn-ref variable `v` of
/// function `def_nr` answers, when every arm of its dispatch is admitted with that record.
/// The fallback is `None`, the buffer road, which costs the rewrite and never a value: a
/// site that is not listed, or one arm out, keeps the `DbRef` every arm then returns.
#[must_use]
pub fn fnref_value(
    sites: &FnRefSites,
    admitted: &HashMap<u32, u16>,
    def_nr: u32,
    v: u16,
) -> Option<u16> {
    let (arms, rec) = sites.get(&(def_nr, v))?;
    let rec = (*rec)?;
    (!arms.is_empty() && arms.iter().all(|a| admitted.get(a) == Some(&rec))).then_some(rec)
}

/// `LOFT_NO_FNREF_VALUE=1` — every call through a fn-ref keeps the buffer road and every
/// arm of a dispatch its record return (`@FR-R-FnRefValue` off): the first bisect step for a
/// wrong field out of a record answered through a fn-ref on `--native`.
pub(crate) fn fnref_value_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_NO_FNREF_VALUE").map_or(true, |v| v == "0"))
}

/// The register layout of one record type: what a tuple of it carries, in field order.
#[derive(Clone, Debug, Default)]
pub struct ValueTuple {
    /// The Rust tuple type — `(f64, f64, f64)`, or `(i64,)` for a one-field record; a
    /// NULLABLE layout ([`nullable_layout`]) ends in the presence word, `(i64, bool)`.
    pub tuple: String,
    /// The fields in ORDER: `(byte offset, Rust type)` — what the tuple carries, and what
    /// the live-reload arm must read back out of the record the interpreter answers with.
    /// The presence word of a nullable layout is no field: it follows them.
    pub fields: Vec<(i64, &'static str)>,
    /// `@FR-R-ValueRecord`'s nullable clause — the tuple of a `τ?` record: one more
    /// element, the PRESENCE word, after the fields ([`ValueTuple::presence`]).
    pub nullable: bool,
}

impl ValueTuple {
    /// The tuple index of the presence word, for a nullable layout: the element after the
    /// fields.  `None` for a dense layout, whose tuple cannot be absent.
    #[must_use]
    pub fn presence(&self) -> Option<usize> {
        self.nullable.then_some(self.fields.len())
    }

    /// @C144 — the ABSENT tuple of a nullable layout — every field its null, as a field read off the
    /// null record answers (`tuple_reads`), and the presence word `false` — or a dense
    /// layout's tuple of field nulls.  ONE spelling for the `null` exit, a nullable value
    /// local's declaration and the bridge's null destination.
    #[must_use]
    pub fn absent(&self) -> String {
        let mut parts: Vec<&str> = self
            .fields
            .iter()
            .map(|(_, rt)| match *rt {
                "f64" => "f64::NAN",
                "f32" => "f32::NAN",
                "bool" => "false",
                VIEW_LEAF_PART => "DbRef::NULL",
                _ => "i64::MIN",
            })
            .collect();
        if self.nullable {
            parts.push("false");
        }
        let tail = if parts.len() == 1 { "," } else { "" };
        format!("({}{tail})", parts.join(", "))
    }
}

/// `@FR-R-ValueRecord`'s nullable clause (@C144) — the bit that turns a record type into the KEY of
/// its nullable layout.  The tables of [`ValueRecords`] key a tuple by LAYOUT, and a `τ?`
/// tuple carries one element more than a `τ` tuple, so the two need two keys: the record
/// type for the dense layout and the record type with this bit for the nullable one.  Two
/// keys keep the two apart everywhere a key is compared — a site, a leaf, a local and a
/// parameter all agree on a key or decline — so a dense consumer can never be handed the
/// longer tuple.  A record type at or above the bit has no nullable layout (no schema is
/// that large; such a function keeps its buffer).
pub const NULLABLE_LAYOUT: u16 = 0x8000;

/// The key of record type `tp`'s nullable layout, or `None` when the type number already
/// uses the bit.
#[must_use]
pub fn nullable_layout(tp: u16) -> Option<u16> {
    (tp & NULLABLE_LAYOUT == 0).then_some(tp | NULLABLE_LAYOUT)
}

/// The RECORD TYPE a layout key carries: the key itself for a dense layout, the key with
/// [`NULLABLE_LAYOUT`] cleared for a nullable one.  Every place that asks the schema about a
/// key asks it of this.
#[must_use]
pub fn layout_record(key: u16) -> u16 {
    key & !NULLABLE_LAYOUT
}

/// Is `key` a nullable layout's key?
#[must_use]
pub fn is_nullable_layout(key: u16) -> bool {
    key & NULLABLE_LAYOUT != 0
}

/// `LOFT_NO_NULLABLE_VALUE_RECORD=1` keeps every `-> S?` function on its return buffer and
/// every `S?` local a `DbRef` (`@FR-R-ValueRecord`'s nullable clause off, the dense spelling
/// untouched): the first bisect step for a wrong field, or a wrong presence test, out of a
/// nullable record-returning call on `--native`.
pub(crate) fn nullable_value_record_disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_NULLABLE_VALUE_RECORD").is_ok_and(|v| v != "0"))
}

/// `(R-ValueLocal)` — per function, the by-value parameters carried as tuples, keyed by
/// attribute index (what a call site needs) and answering the record type.
pub type TupleParams = HashMap<u32, HashMap<usize, u16>>;

impl ValueRecords {
    /// The layout of admitted function `d`'s result, when it returns by value.
    #[must_use]
    pub fn fn_layout(&self, d: u32) -> Option<&ValueTuple> {
        self.types.get(self.fns.get(&d)?)
    }

    /// The Rust tuple type of admitted function `d`'s result.
    #[must_use]
    pub fn fn_tuple(&self, d: u32) -> Option<&str> {
        self.fn_layout(d).map(|t| t.tuple.as_str())
    }

    /// The fields, in order, of admitted function `d`'s result.
    #[must_use]
    pub fn fn_fields(&self, d: u32) -> Option<&[(i64, &'static str)]> {
        self.fn_layout(d).map(|t| t.fields.as_slice())
    }

    /// Every function that is an arm of a listed fn-ref dispatch (`@FR-R-FnRefValue`),
    /// admitted or not.
    #[must_use]
    pub fn fnref_arms(&self) -> HashSet<u32> {
        self.fnref_sites
            .values()
            .flat_map(|(arms, _)| arms.iter().copied())
            .collect()
    }

    /// The record type parameter `idx` of function `d` is received as, when it is a tuple.
    #[must_use]
    pub fn param_type(&self, d: u32, idx: usize) -> Option<u16> {
        self.params.get(&d)?.get(&idx).copied()
    }

    /// The tuple parameters of function `d` as the body names them: variable → record type.
    #[must_use]
    pub fn param_vars(&self, data: &Data, d: u32) -> HashMap<u16, u16> {
        tuple_param_vars(data, d, &self.params)
    }
}

/// The tuple parameters of `d` by VARIABLE — the attribute's name resolved in the body's
/// variable table — for the walks that meet the parameter as a `Var`.
pub(crate) fn tuple_param_vars(data: &Data, d: u32, params: &TupleParams) -> HashMap<u16, u16> {
    let Some(ps) = params.get(&d) else {
        return HashMap::new();
    };
    let def = data.def(d);
    let vars = def.variables();
    ps.iter()
        .filter_map(|(idx, tp)| {
            let v = vars.var(&def.attributes().get(*idx)?.name);
            (v != u16::MAX).then_some((v, *tp))
        })
        .collect()
}

/// Default-ON since 2026-09-14 (@PLN157 § V-ah stage 1); `LOFT_NO_VALUE_RECORD=1` restores
/// the return buffer for every record return — the bisect step for a wrong field out of a
/// record-returning call on native.
///
/// It was opt-in from 2026-09-12 because the call-site gate did not hold over the script
/// corpus: 376 compile errors in three classes.  Two were shapes the gate could not see —
/// a result bound into a compiler `__lift_` temp whose set lowering reads `.store_nr` off
/// it, and a value branch joining an admitted call with a record expression — and the
/// third, 218 of the errors, was the fn-ref DISPATCH: the `match` a `CallRef` emits takes
/// its arms from a signature scan, so every arm shares one return type, and a gate that
/// asked "which functions can a fn-ref reach?" a second way (by returned record, beside a
/// `FnRef` node) missed a lambda that reached a dispatch through a typed variable alone.
/// The gate now reads the arm set from `fnref::dispatch_arms`, the emitter's own home for
/// that question, and declines every arm.
///
/// The library integration the opt-in phase existed for is closed: a cdylib bridge now
/// MATERIALISES the tuple into the destination record it already owns
/// (`native_lib::shared_bridge_wrapper`), so the C boundary keeps its record contract
/// while loft-to-loft calls inside the library take the value path.
pub(crate) fn value_record_disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_VALUE_RECORD").is_ok_and(|v| v != "0"))
}

/// The scalar field kinds a register tuple can carry: no heap, no collection, no nested
/// record — the fields `rust_type` maps to a plain Rust scalar.
pub(crate) fn value_field_type(tp: &Type) -> Option<&'static str> {
    match tp.base() {
        Type::Float => Some("f64"),
        Type::Single => Some("f32"),
        Type::Integer(_) => Some("i64"),
        Type::Boolean => Some("bool"),
        _ => None,
    }
}

/// `LOFT_NO_VALUE_LOCAL=1` keeps every by-value record parameter a `DbRef` — the bisect
/// step for a wrong field read through a small-record parameter on native, and the
/// before-half of `(R-ValueLocal)`'s A/B.  The interpreter is the values oracle either way.
pub(crate) fn value_local_disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_VALUE_LOCAL").is_ok_and(|v| v != "0"))
}

/// The register layout of record type `tp` (declared by definition `rd`), or `None` where
/// some part of it cannot ride a tuple: no field, more than [`VALUE_RECORD_MAX_FIELDS`]
/// scalars, a field with no declaration of its own, a narrow integer, or a heap field that
/// is not a view leaf (`views_ok` false declines every heap field).  An INLINE sub-record
/// contributes its own scalars at the SUMMED offset (`Vertex { pos: Vec3, … }` is eight
/// floats), so a tuple of a record is its scalar fields in declaration order however deep
/// they sit.  Fills `index` with the `(type, offset)` → tuple-index map the call sites read,
/// keyed by THIS type and the summed offset — the key a folded `OpGetField` chain reaches
/// (`view_field`).  ONE home for the question "what does a tuple of this type carry": a
/// result, a local and a parameter agree by construction.
pub(crate) fn type_layout(
    data: &Data,
    stores: &Stores,
    rd: u32,
    tp: u16,
    views_ok: bool,
    index: &mut HashMap<(u16, i64), usize>,
) -> Option<ValueTuple> {
    let mut parts: Vec<&'static str> = Vec::new();
    let mut order: Vec<(i64, &'static str)> = Vec::new();
    let mut idx: Vec<((u16, i64), usize)> = Vec::new();
    layout_into(
        data, stores, rd, tp, tp, 0, views_ok, &mut parts, &mut order, &mut idx,
    )?;
    if parts.is_empty() || parts.len() > VALUE_RECORD_MAX_FIELDS {
        return None;
    }
    index.extend(idx);
    // A ONE-FIELD record needs the trailing comma: `(bool)` is Rust for a
    // PARENTHESISED bool, not a 1-tuple, so the signature promised a scalar while
    // every call site read `.0` off it and the generated crate would not compile
    // (measured on `pub fn tx_new() -> Tx { Tx { open: false } }` in the sqldb
    // fixture: "`bool` is a primitive type and therefore doesn't have fields").
    // The same comma is required on the VALUE side in `emit.rs`, or the two disagree.
    let tuple = if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    };
    Some(ValueTuple {
        tuple,
        fields: order,
        nullable: false,
    })
}

/// `@FR-R-ValueRecord`'s nullable clause — the layout of `τ?` beside the dense `layout` of
/// `τ`, registered under key `nkey` ([`nullable_layout`]): the same fields at the same
/// tuple indices, then the presence word.  The index entries are copied from the dense
/// key's, so a field read off a nullable tuple is the same `.k` as off a dense one — and an
/// ABSENT tuple holds every field's null ([`ValueTuple::absent`]), so that read answers
/// exactly what a field read off the null record answers, with no test of its own.
pub(crate) fn nullable_layout_of(
    layout: &ValueTuple,
    tp: u16,
    nkey: u16,
    index: &mut HashMap<(u16, i64), usize>,
) -> ValueTuple {
    let copied: Vec<((u16, i64), usize)> = index
        .iter()
        .filter(|((t, _), _)| *t == tp)
        .map(|((_, off), i)| ((nkey, *off), *i))
        .collect();
    index.extend(copied);
    let mut parts: Vec<&str> = layout.fields.iter().map(|(_, rt)| *rt).collect();
    parts.push("bool");
    ValueTuple {
        tuple: format!("({})", parts.join(", ")),
        fields: layout.fields.clone(),
        nullable: true,
    }
}

/// One level of [`type_layout`]: the fields of `tp` (declared by `rd`) laid `base` bytes
/// into the TOP record `top`, an inline sub-record recursing with its own base.  A view
/// leaf is admitted at the top level only: a sub-record's heap field would be a place
/// two records deep, which no site gate proves.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_into(
    data: &Data,
    stores: &Stores,
    rd: u32,
    tp: u16,
    top: u16,
    base: i64,
    views_ok: bool,
    parts: &mut Vec<&'static str>,
    order: &mut Vec<(i64, &'static str)>,
    idx: &mut Vec<((u16, i64), usize)>,
) -> Option<()> {
    let (crate::database::Parts::Struct(fields) | crate::database::Parts::EnumValue(_, fields)) =
        &stores.types.get(tp as usize)?.parts
    else {
        return None;
    };
    if fields.is_empty() {
        return None;
    }
    for f in fields {
        // Pair the schema field with the DECLARATION that named it, by name.  The two
        // lists are not the same list and need not be the same length: a runtime
        // schema can carry a field the definition declares no attribute for, and
        // indexing `attributes` by the schema's position then reads another field's
        // type -- or panics, which is what it did (`882-keyed-element-read-borrows-
        // its-container.loft` crashed the compiler: "len is 2 but the index is 2").
        // No match means the record has a part this analysis cannot account for, so
        // the function declines and keeps its return buffer; declining only ever costs
        // the optimisation.
        let a_nr = data
            .def(rd)
            .attributes
            .iter()
            .position(|a| a.name == f.name)?;
        let ftp = data.attr_type(rd, a_nr);
        let off = base + i64::from(f.position);
        // `integer` at its 8-byte width only: the tuple carries an `i64`, and the
        // getter/setter the value path pairs it with (`OpGetInt`/`OpSetInt`, the live
        // arm's `get_int`) read and write eight bytes.  A narrow field (a ranged or
        // `size(1)` alias) declines the function rather than reading its neighbour.
        if matches!(ftp.base(), Type::Integer(_)) && stores.size(f.content) != 8 {
            return None;
        }
        if let Some(rt) = value_field_type(&ftp) {
            parts.push(rt);
            order.push((off, rt));
        } else if views_ok && base == 0 && view_leaf_type(&ftp) {
            parts.push(VIEW_LEAF_PART);
            order.push((off, VIEW_LEAF_PART));
        } else if let Some(sub) = plain_record_type(data, &ftp)
            && sub == f.content
            && stores.is_struct(sub)
            && let Type::Reference(sub_rd, _) = ftp.peel_link()
        {
            // An inline sub-record: the schema flattens it into the parent, so its scalars
            // are the parent's at the summed offset.
            layout_into(
                data, stores, *sub_rd, sub, top, off, false, parts, order, idx,
            )?;
            continue;
        } else {
            return None;
        }
        idx.push(((top, off), parts.len() - 1));
    }
    Some(())
}

/// `(R-ValueLocal)`'s parameter gate, asked the narrower question the tuple form needs
/// where [`body_writes`] cannot type the body: may `node` — or any loft callee it reaches —
/// write a record of type `tp` that EXISTS before the call?  Growth writes fresh slots only
/// (a mint, a push, an append, a reservation); a scalar set and a whole copy are asked
/// their target's type, a fresh mint variable's target answering no; a free, a store test
/// and a read write nothing; a native this walk does not know, a fn-ref, a `par` and a
/// recursion answer yes.  `fresh` carries the mint variables across the statements of one
/// list, as [`body_writes`] keeps them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn may_write_existing(
    node: &Value,
    tp: u16,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
    fresh: &mut HashSet<u16>,
    buffer: Option<u16>,
    memo: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
) -> bool {
    let mut hit = false;
    node.any_node(&mut |n| match n {
        Value::Set(v, inner) => {
            if matches!(inner.unspan(), Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpNewRecord"
                    && mint_path(data, stores, "OpNewRecord", args, vars).is_some())
            {
                fresh.insert(*v);
            } else if Some(*v) != buffer {
                // The frame's own return buffer stays fresh through a rebind: the callee
                // was handed that record to fill, or minted into it — never a parameter's.
                fresh.remove(v);
            }
            false
        }
        Value::Call(d, args) => {
            if (*d as usize) >= data.definitions.len() {
                hit = true;
                return true;
            }
            let def = data.def(*d);
            let name = def.name();
            if !matches!(def.code(), Value::Null) {
                let callee = if let Some(&k) = memo.get(d) {
                    k
                } else if !active.insert(*d) {
                    true
                } else {
                    // The callee's own hidden return buffer is a record that does not
                    // exist before the call — minted from its sentinel, or the CALLER's
                    // owned local under `(R-Rebind)`, never a parameter's record — so its
                    // mint and every write into it are writes of a FRESH record.
                    let own = own_return_buffer(def);
                    let mut inner_fresh: HashSet<u16> = own.into_iter().collect();
                    let k = may_write_existing(
                        def.code(),
                        tp,
                        data,
                        stores,
                        def.variables(),
                        &mut inner_fresh,
                        own,
                        memo,
                        active,
                    );
                    active.remove(d);
                    memo.insert(*d, k);
                    k
                };
                if callee {
                    crate::rewrite_census::declined(
                        "R-ValueRecord",
                        &format!("{}:{tp}", def.name()),
                        "a callee may write an existing record of the type",
                    );
                    if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
                        eprintln!(
                            "[valuerec]   callee `{name}` may write an existing record of type {tp}"
                        );
                    }
                    hit = true;
                    return true;
                }
                return false;
            }
            if name == "OpFinishRecord"
                && let Some(Value::Var(e)) = args.get(1).map(Value::unspan)
            {
                fresh.remove(e);
            }
            // A mint into a local that OWNS its store — a fresh store, or the local's own
            // previous one reused in place — is a record no parameter of this frame can
            // name: what is written into it afterwards is not an existing record.
            if matches!(name, "OpDatabase" | "OpDatabaseNP")
                && let Some(Value::Var(b)) = args.first().map(Value::unspan)
                && owning_local(vars, *b)
            {
                fresh.insert(*b);
            }
            if native_writes_existing(def, args, tp, data, stores, vars, fresh) {
                crate::rewrite_census::declined(
                    "R-ValueRecord",
                    &format!("{}:{tp}", def.name()),
                    "a native op may write an existing record of the type",
                );
                if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
                    let shown = format!("{n:?}");
                    eprintln!(
                        "[valuerec]   `{name}` may write an existing record of type {tp}: {}",
                        &shown[..shown.len().min(160)]
                    );
                }
                hit = true;
                return true;
            }
            false
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => {
            hit = true;
            true
        }
        _ => false,
    });
    hit
}

/// Does `v` own the store it names — a user or compiler local that is no parameter, no
/// link, no capture, no inline element, and borrows from nothing but its own `__vdb_N`
/// witness?  A record minted into such a local lives in a store no parameter of the frame
/// can reach.
pub(crate) fn owning_local(vars: &crate::variables::Function, v: u16) -> bool {
    (v as usize) < vars.var_count()
        && !vars.is_argument(v)
        && !vars.is_captured(v)
        && !vars.is_inline_ref(v)
        && !vars.is_skip_free(v)
        && !matches!(vars.tp(v).base(), Type::RefVar(_))
        && vars
            .tp(v)
            .depend()
            .iter()
            .all(|d| *d == v || vars.name(*d).starts_with("__vdb"))
}

/// A function's hidden return-buffer variable, when it has one: the record the frame is
/// handed to fill, which no by-value parameter of any frame can name.
pub(crate) fn own_return_buffer(def: &crate::data::Definition) -> Option<u16> {
    let vars = def.variables();
    def.hidden_return_buffer_attr()
        .map(|a| vars.var(&def.attributes()[a].name))
        .filter(|v| *v != u16::MAX)
}

/// [`may_write_existing`]'s verdict for one NATIVE call: a scalar set and a whole copy are
/// asked their target's type (a fresh mint variable's target answers no), growth and a
/// mint write fresh slots only, a free, a store test and a read write nothing, a method
/// whose every collection parameter is `const` reads only — and anything else answers yes.
pub(crate) fn native_writes_existing(
    def: &crate::data::Definition,
    args: &[Value],
    tp: u16,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
    fresh: &HashSet<u16>,
) -> bool {
    let name = def.name();
    if IN_PLACE_SET_OPS.contains(&name) {
        return !args.first().is_some_and(|t| fresh_rooted(t, data, fresh))
            && match args
                .first()
                .map_or(Target::Unknown, |t| setter_target(t, data, stores, vars))
            {
                Target::Record(chain) => chain.iter().any(|(t, _)| *t == tp),
                Target::NoRecord => false,
                Target::Unknown => true,
            };
    }
    if name == "OpCopyRecord" {
        return args.len() != 3
            || (!fresh_rooted(&args[1], data, fresh)
                && !matches!(args[2].unspan(), Value::Int(k)
                    if u16::try_from(*k & i32::from(crate::keys::COPY_TP_MASK)).is_ok_and(|t| t != tp)));
    }
    if matches!(
        name,
        "OpNewRecord"
            | "OpFinishRecord"
            | "OpPreAllocVector"
            | "OpAppendVector"
            | "OpDatabase"
            | "OpDatabaseNP"
            | "OpPlaceRecord"
            | "OpMoveRecord"
            | "OpMoveField"
            | "OpMoveVector"
            | "OpFreeRef"
            | "OpFreeRefIfDistinct"
            | "OpFreeRecordIn"
            | "OpDistinctStore"
            | "OpRefAlias"
            | "OpRefIsNull"
            | "OpConvBoolFromRef"
    ) || FUSABLE_PUSHES.iter().any(|(p, _, _)| *p == name)
        || name.starts_with("OpGet")
        || native_op_is_store_free(def)
    {
        return false;
    }
    let value = |t: &Type| is_scalar(t) || matches!(t.base(), Type::Text(_));
    !def.attributes()
        .iter()
        .all(|a| a.value_const || value(&a.typedef))
}

/// `ops` with every TERMINAL whole-record copy — an `OpCopyRecord` statement whose next
/// statement (past line markers) is a bare `return` — replaced by `Null`, recursively
/// through every statement list.  After such a copy nothing in the frame runs, so no
/// by-value parameter can read a record it wrote; the tuple-parameter gate leaves it out
/// of the write set.  Everything else is the statement it was.
pub(crate) fn without_terminal_copies(ops: &[Value], data: &Data) -> Vec<Value> {
    let copy_nr = data.def_nr("OpCopyRecord");
    fn bare_return(v: &Value) -> bool {
        matches!(v.unspan(), Value::Return(x) if matches!(x.unspan(), Value::Null))
    }
    fn walk(v: &Value, data: &Data) -> Value {
        match v {
            Value::Span(b) => Value::Span(Box::new((b.0, walk(&b.1, data)))),
            Value::Block(bl) => {
                let mut nb = (**bl).clone();
                nb.operators = without_terminal_copies(&bl.operators, data);
                Value::Block(Box::new(nb))
            }
            Value::Loop(bl) => {
                let mut nb = (**bl).clone();
                nb.operators = without_terminal_copies(&bl.operators, data);
                Value::Loop(Box::new(nb))
            }
            Value::Insert(ls) => Value::Insert(without_terminal_copies(ls, data)),
            Value::If(c, a, b) => Value::If(
                Box::new(walk(c, data)),
                Box::new(walk(a, data)),
                Box::new(walk(b, data)),
            ),
            other => other.clone(),
        }
    }
    let mut out: Vec<Value> = Vec::with_capacity(ops.len());
    for (i, op) in ops.iter().enumerate() {
        let terminal = matches!(op.unspan(), Value::Call(d, _) if *d == copy_nr)
            && ops[i + 1..]
                .iter()
                .find(|n| !matches!(n.unspan(), Value::Line(_)))
                .is_some_and(bare_return);
        out.push(if terminal {
            Value::Null
        } else {
            walk(op, data)
        });
    }
    out
}

/// `(R-ValueLocal)` (`@FR-R-ValueLocal`) — the by-value parameters that MAY be received as
/// tuples, by the facts that do not depend on what else is admitted: the parameter is a
/// plain no-heap record of scalars (no view leaf — a parameter's vector field would be a
/// reference into the caller's frame, which the tuple form does not carry), neither hidden,
/// linked (`&`) nor nullable; the function has a loft body that is neither a generic
/// template nor a generator; the body never REBINDS the parameter; and the body — with
/// every function it calls — writes no record of the parameter's type other than through a
/// return buffer.
///
/// That last fact is the rule's whole soundness: a by-value record parameter is a VIEW of
/// the argument's place (`fn bump(p: V3, w: V3) { w.x = 5.0; p.x }` called `bump(a, a)`
/// answers 5), so a write through any other route to that place — a second parameter, a
/// parent record's inline sub-record, an element view of the container the argument came
/// from — must be seen by `p`, and a tuple read at the call cannot see it.  The write set is
/// keyed by record type, so any write of that type anywhere in the body declines; the one
/// exemption is a RETURN BUFFER (`retbuf_only_writer`), because `(R-Rebind)` hands a callee
/// the caller's own record as its buffer only where the callee stages every read of the
/// parameter ahead of its first write, which the tuple's read at the call equals.  A body
/// this analysis cannot type declines.
///
/// Which candidates ARE received as tuples is decided in [`value_records`]'s fixpoint: a
/// parameter handed to another function is served only by a tuple parameter there.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(crate) fn tuple_param_candidates(
    data: &Data,
    stores: &Stores,
    types: &mut HashMap<u16, ValueTuple>,
    index: &mut HashMap<(u16, i64), usize>,
) -> TupleParams {
    let mut out = TupleParams::new();
    let mut existing_memo: HashMap<u32, bool> = HashMap::new();
    if value_local_disabled() {
        return out;
    }
    let trace = std::env::var("LOFT_TRACE_VALUEREC").is_ok();
    let mut cache: HashMap<u32, bool> = HashMap::new();
    let mut writes: WriteCache = HashMap::new();
    for d_nr in 0..data.definitions.len() as u32 {
        let def = data.def(d_nr);
        if !matches!(def.def_type, crate::data::DefType::Function)
            || !def.rust().is_empty()
            || matches!(def.returned().base(), Type::Iterator(_, _))
        {
            continue;
        }
        let Value::Block(body) = def.code().unspan() else {
            continue;
        };
        if def.code().any_node(&mut |n| matches!(n, Value::Yield(_))) {
            continue;
        }
        let vars = def.variables();
        let mut rebound: Option<HashSet<u16>> = None;
        let mut written: Option<Option<WriteSet>> = None;
        for (idx, a) in def.attributes().iter().enumerate() {
            if a.hidden || matches!(a.typedef.base(), Type::RefVar(_)) {
                continue;
            }
            // `@FR-N-Road` — a `p: S?` is a candidate exactly as `p: S` is, carried by the
            // nullable layout: its fields, then whether the argument named a record.
            let (inner, optional) = a.typedef.peel_optional();
            if optional && nullable_value_record_disabled() {
                continue;
            }
            let Type::Reference(rd, _) = a.typedef.base() else {
                continue;
            };
            let Some(tp) = plain_record_type(data, inner) else {
                continue;
            };
            if stores.owns_heap(tp) {
                continue;
            }
            let layout = match types.get(&tp) {
                Some(t) => t.clone(),
                None => match type_layout(data, stores, *rd, tp, false, index) {
                    Some(t) => {
                        types.insert(tp, t.clone());
                        t
                    }
                    None => continue,
                },
            };
            if layout.fields.iter().any(|(_, rt)| is_view_part(rt)) {
                continue;
            }
            let v = vars.var(&a.name);
            if v == u16::MAX {
                continue;
            }
            let rebound = rebound.get_or_insert_with(|| rebound_vars(body, data, vars));
            if rebound.contains(&v) {
                if trace {
                    crate::loft_eprintln!(
                        "[valuerec] {}: parameter `{}` is rebound in the body",
                        def.name(),
                        a.name
                    );
                }
                continue;
            }
            let written = written.get_or_insert_with(|| {
                let mut active = HashSet::new();
                if !call_writes_store(d_nr, data, &mut cache, &mut active, None)
                    || retbuf_only_writer(d_nr, data, &mut cache, &mut active, None)
                {
                    return Some(WriteSet::default());
                }
                let mut set = WriteSet::default();
                // The frame's own return buffer is a record no parameter views: minted from
                // its sentinel, or the caller's owned local under `(R-Rebind)`.  Seeded
                // FRESH, so its mint and every write into it contribute nothing.
                let mut fresh: HashSet<u16> = def
                    .hidden_return_buffer_attr()
                    .map(|a| vars.var(&def.attributes()[a].name))
                    .filter(|v| *v != u16::MAX)
                    .into_iter()
                    .collect();
                // A whole-record copy that is the LAST thing a path does before a bare
                // `return` — `v[i] = h; return;` — is a write no parameter can observe from
                // this frame: nothing runs after it.  It is left out of the set, so a
                // setter whose only write of its own type is that copy takes the tuple.
                let pruned = without_terminal_copies(&body.operators, data);
                for op in &pruned {
                    let w = body_writes(
                        op,
                        data,
                        stores,
                        vars,
                        &mut cache,
                        &mut writes,
                        &mut HashSet::new(),
                        &mut fresh,
                        None,
                    )?;
                    set.extend(&w);
                }
                Some(set)
            });
            // A body [`body_writes`] cannot type (a callee that grows a collection, say) is
            // asked the narrower question the tuple form needs: may it write a record of
            // the parameter's type that EXISTS before the call?
            let sound = if let Some(w) = written.as_ref() {
                !w.reaches_record(tp)
            } else {
                let pruned = without_terminal_copies(&body.operators, data);
                let own = own_return_buffer(def);
                let mut fresh: HashSet<u16> = own.into_iter().collect();
                !pruned.iter().any(|op| {
                    may_write_existing(
                        op,
                        tp,
                        data,
                        stores,
                        vars,
                        &mut fresh,
                        own,
                        &mut existing_memo,
                        &mut HashSet::new(),
                    )
                })
            };
            if !sound {
                if trace {
                    let w = written.as_ref();
                    crate::loft_eprintln!(
                        "[valuerec] {}: parameter `{}` — the body may write a record of its type (whole {:?}, offsets {:?})",
                        def.name(),
                        a.name,
                        w.map(|w| w.whole.iter().filter(|t| **t == tp).count()),
                        w.map(|w| w
                            .offsets
                            .iter()
                            .filter(|(t, _)| *t == tp)
                            .map(|(_, o)| *o)
                            .collect::<Vec<_>>())
                    );
                }
                continue;
            }
            let key = if optional {
                let Some(nkey) = nullable_presence_layout(types, index, tp) else {
                    continue;
                };
                nkey
            } else {
                tp
            };
            out.entry(d_nr).or_default().insert(idx, key);
        }
    }
    out
}

/// The key of record type `tp`'s NULLABLE layout, registered beside its dense one when it
/// is not yet ([`nullable_layout_of`]).  `None` where the record has no nullable layout: its
/// dense layout is unknown, it carries a view leaf (whose null is a reported one the absent
/// tuple does not spell), or its type number already uses the key bit.
pub(crate) fn nullable_presence_layout(
    types: &mut HashMap<u16, ValueTuple>,
    index: &mut HashMap<(u16, i64), usize>,
    tp: u16,
) -> Option<u16> {
    let nkey = nullable_layout(tp)?;
    if types.contains_key(&nkey) {
        return Some(nkey);
    }
    let layout = types.get(&tp)?.clone();
    if layout.fields.iter().any(|(_, rt)| is_view_part(rt)) {
        return None;
    }
    let nlayout = nullable_layout_of(&layout, tp, nkey, index);
    types.insert(nkey, nlayout);
    Some(nkey)
}

/// The widest record the value path carries.  Eight: a tuple past the register file is
/// returned through the caller's stack slot, which is still nothing beside a store record
/// (moros's seven-field `Hex` was the row that moved six to eight).
pub const VALUE_RECORD_MAX_FIELDS: usize = 8;

/// The tuple part a VIEW LEAF carries: the `DbRef` of the place the field views, where a
/// scalar field carries its own value.  It is the same reference the record form's
/// `OpGetField` hands a reader, which is why every READ at a call site is unchanged and
/// only a write or a stale place could tell the difference (`@FR-O-ViewField`).
pub const VIEW_LEAF_PART: &str = "DbRef";

/// Is this field one a view leaf may deliver?  A plain `vector<T>` only: its value IS the
/// field slot's reference, so a reader that takes the slot's address reads the container's
/// own vector.  `text` and the keyed kinds are left out — a `text` field is read through
/// its own getters into a `String` and a keyed collection's reads go through the key
/// machinery, so neither is served by handing over the slot — and a nested record field is
/// not a leaf at all.  Leaving a kind out costs the rewrite, never a value.
pub(crate) fn view_leaf_type(ftp: &Type) -> bool {
    matches!(ftp.base(), Type::Vector(_, _))
}

/// Does `rt` name the view-leaf part?
#[must_use]
pub fn is_view_part(rt: &str) -> bool {
    rt == VIEW_LEAF_PART
}

/// What one exit of an admitted body delivers for a view-leaf field — decided once by the
/// gate ([`leaf_source`](super::leaves::leaf_source)), kept in [`ViewPlan::leaves`], and written by the emitter as it
/// stands, so the two cannot drift.
#[derive(Clone, Debug, PartialEq)]
pub enum LeafSource {
    /// The exit does not NAME the field at all — an empty literal, whose value in the
    /// record form is the prefill's own zero — so the leaf is a null reference, which every
    /// read answers exactly as the empty vector it replaces (measured: `len` 0, an iteration
    /// of nothing, an element read null, a `const` argument the same).  A null view can never
    /// go stale, so it needs no place and no disturbance test.
    Null,
    /// Every path to the exit last appended the SAME element temp: the leaf is that element's
    /// field, `OpGetField(Var(elem), off, tp)` — the destination the append copied into.
    Elem { elem: u16, off: i64, tp: i32 },
    /// The paths to the exit appended DIFFERENT element temps (one per arm): no temp names the
    /// place on every path, and the container's LAST element does — the proof is that on
    /// every path the last change to the container was one of those appends.  `parent_tp` and
    /// `fld` are the mint's own arguments, from which the element's stride derives exactly as
    /// the runtime derives it.
    Last {
        root: crate::scopes::ParamPlace,
        parent_tp: i32,
        fld: i32,
        off: i64,
    },
}

impl LeafSource {
    /// The parameter place a call site must keep undisturbed, or `None` for a null view.
    pub(crate) fn root(
        &self,
        data: &Data,
        stores: &Stores,
        def_nr: u32,
    ) -> Option<crate::scopes::ParamPlace> {
        match self {
            LeafSource::Null => None,
            LeafSource::Last { root, .. } => Some(*root),
            LeafSource::Elem { elem, .. } => elem_mint(data, stores, def_nr, *elem).map(|m| m.0),
        }
    }
}

/// The element temp `e` an append is building, as its single mint names it:
/// `e = OpNewRecord(<parameter>, parent_tp, fld)`.  Answers the parameter place the element
/// lives in and the mint's `(parent_tp, fld)`.  A temp with a second non-null assignment, or
/// minted anywhere but into a parameter, answers `None` — the element is then not one a view
/// may name.
pub(crate) fn elem_mint(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    e: u16,
) -> Option<(crate::scopes::ParamPlace, i32, i32)> {
    let mut mint: Option<&Value> = None;
    let mut many = false;
    data.def(def_nr).code().any_node(&mut |n| {
        if let Value::Set(w, rhs) = n
            && *w == e
            && !matches!(rhs.unspan(), Value::Null)
        {
            many |= mint.is_some();
            mint = Some(rhs);
        }
        false
    });
    if many {
        return None;
    }
    let rhs = mint?;
    let Value::Call(d, args) = rhs.unspan() else {
        return None;
    };
    if (*d as usize) >= data.definitions.len()
        || !matches!(data.def(*d).name(), "OpNewRecord" | "OpNewRecordNP")
    {
        return None;
    }
    let (Some(Value::Int(ptp)), Some(Value::Int(fld))) = (
        args.get(1).map(Value::unspan),
        args.get(2).map(Value::unspan),
    ) else {
        return None;
    };
    let place = leaf_root(data, stores, def_nr, rhs, 0)?;
    Some((place, *ptp, *fld))
}

/// The PARAMETER PLACE a leaf expression roots in — the container whose growth or removal
/// would move the record the leaf's field slot sits in, named as
/// [`crate::scopes::ParamPlace`] so the site gate can ask the disturbance producers about
/// it directly.
///
/// The chain it resolves is one step of indirection: a parameter's vector field, an element
/// of it, and a field of that element.  A local on the way is followed through its single
/// assignment — an element read (`sc.ops[i]?`) or a mint into the container
/// (`OpNewRecord(sc, tp, off)`, the element an append is building).
///
/// `None` is the decline, and it is the answer for everything else: a deeper chain, a local
/// with more than one assignment, a place rooted in a store the frame owns.  A frame-owned
/// root is the case that MUST decline — the view would name a store freed at the return —
/// and it is also the common one, which is why the resolution is positive: a place is a
/// leaf root only when the walk reaches a parameter.
pub(crate) fn leaf_root(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    e: &Value,
    depth: u8,
) -> Option<crate::scopes::ParamPlace> {
    // A bound, not a shape rule: the chain a real discharge spells is already five steps
    // (`o.opts` → `o` → its `ncc` block → the block's `if` → the element read), and the
    // bound is what keeps a local whose assignment mentions itself from recursing forever.
    if depth > 12 {
        return None;
    }
    let def = data.def(def_nr);
    let vars = def.variables();
    match e.unspan() {
        Value::Var(v) => {
            if vars.is_argument(*v) {
                return Some((*v, crate::use_analysis::ANY_FIELD));
            }
            // A local that OWNS what it holds is not a route to a parameter's place, even
            // where its assignment names one: a materialised view (`(B-View)`'s copy, which
            // is what a container grown under the binding produces) holds a COPY of the
            // element, and a leaf naming the container would answer the container's own
            // element instead of the copy the record form answers.  The ownership oracle
            // (`@FR-O-Oracle`) is the one home for that question.
            // A local whose ownership is a JOIN owns what it holds on at least one path,
            // and that path is the frame's: a `?`-DISCHARGE of an element mints the absent
            // arm's record in a store the frame frees at the return, so a leaf naming the
            // container would be a view of that store wherever the element was absent.
            // The oracle (`@FR-O-Oracle`) is the one home for the question; the rest of the
            // chain is decided by SHAPE below, which is what keeps a mint of the frame's
            // own store out: no arm of this walk resolves an `OpDatabase`.
            if matches!(
                crate::use_analysis::ownership_of(data, def_nr, e),
                crate::use_analysis::Own::Join { .. }
            ) {
                return None;
            }
            let mut assigned: Option<&Value> = None;
            let mut many = false;
            def.code().any_node(&mut |n| {
                if let Value::Set(w, rhs) = n
                    && w == v
                    && !matches!(rhs.unspan(), Value::Null)
                {
                    if assigned.is_some() {
                        many = true;
                        return true;
                    }
                    assigned = Some(rhs);
                }
                false
            });
            if many {
                return None;
            }
            leaf_root(data, stores, def_nr, assigned?, depth + 1)
        }
        Value::Block(bl) => leaf_root(data, stores, def_nr, bl.operators.last()?, depth + 1),
        Value::If(_, then_v, _) => leaf_root(data, stores, def_nr, then_v, depth + 1),
        Value::Insert(ops) => leaf_root(data, stores, def_nr, ops.last()?, depth + 1),
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            match name {
                // A field of a parameter, or a field of something rooted at one: the FIELD
                // is the place when its base is the parameter itself, and the base's place
                // otherwise (a field of an element does not name a container of its own).
                "OpGetField" | "OpGetDbRef" => {
                    let base = args.first()?;
                    if let Value::Var(v) = base.unspan()
                        && vars.is_argument(*v)
                        && let Some(Value::Int(off)) = args.get(1).map(Value::unspan)
                        && let Ok(o) = u32::try_from(*off)
                    {
                        return Some((*v, o));
                    }
                    leaf_root(data, stores, def_nr, base, depth + 1)
                }
                // An element read: the place is the CONTAINER it reads from.
                "OpGetVector" | "OpGetVectorNullable" | "OpVectorRef" => {
                    leaf_root(data, stores, def_nr, args.first()?, depth + 1)
                }
                // The element an append is building, minted in the container the last
                // argument names by field NUMBER — which a place carries as a byte OFFSET, so
                // the schema makes the conversion (`Stores::field_position`, the same one
                // `scopes::grown_containers` makes for the same reason).  Read as an offset,
                // the place named field 0's number where the field sits at byte 8, and a
                // removal from the very container the leaf views compared as a different
                // place: the `b8` cell measured it, reading the NEXT element's points.
                "OpNewRecord" | "OpNewRecordNP" => {
                    let Value::Var(v) = args.first()?.unspan() else {
                        return None;
                    };
                    if !vars.is_argument(*v) {
                        return None;
                    }
                    let Some(Value::Int(fld)) = args.get(2).map(Value::unspan) else {
                        return None;
                    };
                    if *fld == i32::from(u16::MAX) {
                        return Some((*v, crate::use_analysis::ANY_FIELD));
                    }
                    let parent = data.type_def_nr(vars.tp(*v).base());
                    if parent == u32::MAX {
                        return None;
                    }
                    let off = stores
                        .field_position(data.def(parent).known_type(), u16::try_from(*fld).ok()?);
                    (off != u16::MAX).then(|| (*v, u32::from(off)))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// @PLN164 C5 — the view-leaf PLAN of one admitted body: per heap field, the parameter
/// places its exits deliver a view of, and per exit the leaf itself.  A field an exit writes
/// not at all contributes no place (a null view can never be stale); a field two exits view
/// through different places contributes both, and a site must keep every one of them
/// undisturbed.
///
/// `None` declines the function, and it is the answer whenever an exit's leaf cannot be
/// PROVEN ([`leaf_source`](super::leaves::leaf_source)) — which includes every body that could disturb the place between
/// the append the view names and the exit, since that is a path on which the fact fails.
pub struct ViewPlan {
    /// Field byte offset → the places its exits view.  Empty set: every exit is a null view.
    pub roots: HashMap<i64, HashSet<crate::scopes::ParamPlace>>,
    /// `(exit Object block address, field byte offset)` → what that exit delivers.  The
    /// emitter reads this and nothing else, so it writes what the gate proved.
    pub leaves: HashMap<(usize, i64), LeafSource>,
}

/// @PLN164 C5 — per record type carried as a tuple, the byte offsets of its VIEW-LEAF
/// fields.  A call site reads such a field off the tuple's own reference, so the read is
/// accounted where a record's collection-field read would decline the local.
pub type ViewOffsets = HashMap<u16, HashSet<i64>>;
