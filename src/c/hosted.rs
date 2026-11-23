// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@PLN206 P4` — a HOSTED unit: a library's functions as C that the interpreter adopts, working
//! on the interpreter's own stores (`doc/claude/plans/206-one-store-model/p3-p4-design.md`).
//!
//! The unit is an ordinary one (`emit_unit`) built with `LOFT_HOSTED`: its `loft_stores` is the
//! interpreter's table, a store's birth and death are the interpreter's (`HostedTable`), and
//! everything inside a store it does itself on the shared `loft_store_t` prefix.  It exports the
//! interface a rustc artifact exports, so the interpreter's dispatch is the same: one
//! `loft_shared_<name>` bridge per function over `LibArg` slots, `loft_shared_init`, and the type
//! layout the unit was written for.  A function whose slots the C bridge does not carry yet is
//! not exported, and stays interpreted.

use super::{CType, Cx, c_name};
use crate::data::{Data, Type};
use crate::database::Stores;
use crate::native_gate::BridgeAttrKind;
use std::collections::HashSet;
use std::fmt::Write as _;

pub use crate::keys::{hosted_planted as planted, lib_backend_c as selected, poison_claim};

/// What the unit is told about the interpreter it runs in, as `#define`s before the runtime:
/// the length of a `Store`'s own fields, the `HostTable` it is handed, and where a `LibArg`
/// keeps each part.  The host writes them from its own layout, so the unit reads that layout
/// exactly.
pub(crate) fn defines() -> String {
    use crate::host_abi::{HostTable, LibArg};
    use crate::keys::DbRef;
    use std::mem::{offset_of, size_of};
    format!(
        "#define LOFT_HOSTED 1\n\
         #define LOFT_HOST_STORE_TAIL {}\n\
         #define LOFT_HOST_ABI {}\n\
         #define LOFT_HT_HOSTED {}\n\
         #define LOFT_HT_TYPE_ID {}\n\
         #define LOFT_HT_CALL_REF {}\n\
         #define LOFT_HOST_FN (INT64_C(1) << 62)\n\
         #define LOFT_STORES_VIEW_AT {}\n\
         #define LOFT_LA_SIZE {}\n#define LOFT_LA_SCALAR {}\n#define LOFT_LA_DBREF {}\n\
         #define LOFT_LA_TPTR {}\n#define LOFT_LA_TLEN {}\n\
         #define LOFT_DB_STORE {}\n#define LOFT_DB_REC {}\n#define LOFT_DB_POS {}\n",
        crate::store::HOST_STORE_TAIL,
        crate::host_abi::HOST_ABI,
        offset_of!(HostTable, hosted),
        offset_of!(HostTable, type_id),
        offset_of!(HostTable, call_ref),
        offset_of!(crate::database::Stores, hosted_view),
        size_of::<LibArg>(),
        offset_of!(LibArg, scalar),
        offset_of!(LibArg, dbref),
        offset_of!(LibArg, text_ptr),
        offset_of!(LibArg, text_len),
        offset_of!(DbRef, store_nr),
        offset_of!(DbRef, rec),
        offset_of!(DbRef, pos),
    )
}

/// The `LibArg` accessors and the bridge's entry and exit, written once per unit.
const SLOTS: &str = r"
/* ── The bridge's slots (`host_abi::LibArg`, by the offsets above) ── */
static int64_t loft_la_bits(const unsigned char *a, size_t i) {
    int64_t v;
    memcpy(&v, a + i * LOFT_LA_SIZE + LOFT_LA_SCALAR, 8);
    return v;
}
/* The interpreter's call of a function value (`HostTable::call_ref`, `State::call_back`). */
typedef void (*loft_call_ref_fn)(void *, uint32_t, const unsigned char *, const unsigned char *,
                                 size_t, unsigned char *);
static loft_call_ref_fn loft_host_call_ref = 0;
/* A `DbRef` as the interpreter holds one in memory — in a slot, or on its stack. */
static loft_ref loft_db_read(const unsigned char *d) {
    uint16_t nr;
    loft_ref r = { 0, 0, 0, 0 };
    memcpy(&nr, d + LOFT_DB_STORE, 2);
    memcpy(&r.rec, d + LOFT_DB_REC, 4);
    memcpy(&r.pos, d + LOFT_DB_POS, 4);
    r.store_nr = nr;
    return r;
}
static void loft_db_write(unsigned char *d, loft_ref r) {
    uint16_t nr = r.store_nr;
    memcpy(d + LOFT_DB_STORE, &nr, 2);
    memcpy(d + LOFT_DB_REC, &r.rec, 4);
    memcpy(d + LOFT_DB_POS, &r.pos, 4);
}
static loft_ref loft_la_ref(const unsigned char *a, size_t i) {
    return loft_db_read(a + i * LOFT_LA_SIZE + LOFT_LA_DBREF);
}
/* The reference a `&` argument's slot holds: the caller's variable, on the interpreter's
 * stack, in its own layout — not a record field's. */
static loft_ref loft_slot_ref(loft_ref at, uint32_t fld) {
    return loft_db_read(loft_stores[at.store_nr].buf + (size_t) at.rec * 8 + at.pos + fld);
}
static void loft_slot_set_ref(loft_ref at, uint32_t fld, loft_ref r) {
    loft_db_write(loft_stores[at.store_nr].buf + (size_t) at.rec * 8 + at.pos + fld, r);
}
/* A function value the interpreter hands in: its number is the program's, not the unit's, so
 * it is tagged (`LOFT_HOST_FN`) and a dispatcher hands it back to the interpreter. */
static loft_fn loft_la_fn(const unsigned char *a, size_t i) {
    loft_fn f;
    f.d = loft_la_bits(a, i) | LOFT_HOST_FN;
    f.clos = loft_la_ref(a, i);
    return f;
}
static loft_str loft_la_text(const unsigned char *a, size_t i) {
    loft_str s;
    memcpy(&s.p, a + i * LOFT_LA_SIZE + LOFT_LA_TPTR, sizeof s.p);
    memcpy(&s.len, a + i * LOFT_LA_SIZE + LOFT_LA_TLEN, sizeof s.len);
    return s;
}
static void loft_la_set_bits(unsigned char *a, int64_t v) {
    memcpy(a + LOFT_LA_SCALAR, &v, 8);
}
/* A text answer: its bytes, valid until the unit's next call — the dispatcher copies them at
 * once.  The text a function answers, owned: held until the unit's next text answer. */
static LOFT_TLS loft_text loft_br_answer;
static const char loft_br_empty[1] = { 0 };
static void loft_la_set_text(unsigned char *a, loft_str s) {
    const char *p = s.len ? s.p : loft_br_empty;
    memcpy(a + LOFT_LA_TPTR, &p, sizeof p);
    memcpy(a + LOFT_LA_TLEN, &s.len, sizeof s.len);
}
static void loft_la_set_ref(unsigned char *a, loft_ref r) {
    loft_db_write(a + LOFT_LA_DBREF, r);
}

/* A bridge call works on the `Stores` it is handed: the table is read from it, and the unit's
 * constants are made in it at the first call.  A nested call (a callback that calls the
 * library again) restores the outer one's on the way out. */
static int loft_hosted_ready = 0;
static void *loft_host_enter(void *stores) {
    void *prev = loft_host_stores;
    /* A store parked for a function's next call would outlive the bridge call in a slot the
     * interpreter never sees, and no program end drains it: a unit frees what it would park. */
    loft_no_park = 1;
    loft_host_stores = stores;
    loft_host_sync();
    if (!loft_hosted_ready) {
        loft_hosted_ready = 1;
        /* The program's directory is the interpreter's: what `source_dir()` answers and a
         * relative path resolves against. */
        size_t n = 0;
        const unsigned char *d = loft_hosted->source_dir(stores, &n);
        char *anchor = (char *) malloc(n + 1);
        if (anchor) {
            memcpy(anchor, d, n);
            anchor[n] = 0;
            loft_anchor = anchor;
        }
        loft_init();
    }
    return prev;
}
#ifdef LOFT_HOSTED_PLANT
/* The positive control of `LOFT_HOSTED_VERIFY` (`LOFT_HOSTED_PLANT`): the first bridge call that
 * takes a record writes one byte of it the function's body never writes — a defect the verify
 * must report. */
static void loft_hosted_plant(loft_ref r) {
    static int planted = 0;
    if (planted || r.store_nr >= loft_nstores || r.rec == 0 || !loft_stores[r.store_nr].live) return;
    if (r.rec >= loft_stores[r.store_nr].size) return;
    planted = 1;
    loft_stores[r.store_nr].buf[(size_t) r.rec * 8 + r.pos] ^= 0x5A;
}
#endif
static void loft_host_leave(void *prev) {
    loft_host_stores = prev;
    if (prev) loft_host_sync();
}
";

/// The C value of argument slot `slot` read as shape `c`, when one slot carries it.
fn slot_arg(c: CType, slot: usize) -> Option<String> {
    let bits = format!("loft_la_bits(loft_a, {slot})");
    Some(match c {
        CType::Int => bits,
        CType::Int32 => format!("(int32_t) {bits}"),
        CType::Float => format!("loft_f64({bits})"),
        CType::Single => format!("loft_f32((uint32_t) {bits})"),
        CType::Bool | CType::Enum => format!("(uint8_t) {bits}"),
        CType::Char => format!("(uint32_t) {bits}"),
        CType::Ref => format!("loft_la_ref(loft_a, {slot})"),
        CType::Text => format!("loft_la_text(loft_a, {slot})"),
        CType::Fn => format!("loft_la_fn(loft_a, {slot})"),
        _ => return None,
    })
}

/// The bridge of function `d` under `symbol`, or why the C bridge does not carry it yet.
fn bridge(cx: &Cx<'_>, d: u32, symbol: &str) -> Result<String, String> {
    let data = cx.data;
    let def = data.def(d);
    let ret_text = matches!(def.returned().base(), Type::Text(_));
    // A generator lives in its runtime's coroutine table: one the unit makes is not the
    // interpreter's to drive, and one the interpreter made is not the unit's.  The rustc
    // artifact forwards its coroutines to the host (`HostTable::coroutines`); a hosted C unit
    // does not yet, so an iterator crossing either way stays interpreted.
    if matches!(def.returned().base(), Type::Iterator(..))
        || def
            .attributes()
            .iter()
            .any(|a| matches!(a.typedef.base(), Type::Iterator(..)))
    {
        return Err("an iterator".into());
    }
    let mut args = Vec::new();
    // A text work slot is the call's own: one per slot, as a C caller holds one local per slot
    // — two slots sharing a buffer write over each other (a local held in one while the result
    // is built in the other), and a bridge reached again through a call back would empty the
    // outer call's.
    let mut work = 0usize;
    let mut slot = 0usize;
    let (mut places, mut back) = (String::new(), String::new());
    // The tuple parameters the function takes by address (`@FR-R-ValueRecord`'s pointer clause).
    let by_address = cx.vr_ptr_params(d);
    for (i, a) in def.attributes().iter().enumerate() {
        let kind = crate::native_gate::classify_bridge_attr(a, ret_text)
            .ok_or_else(|| format!("parameter `{}` crosses no bridge", a.name))?;
        match kind {
            BridgeAttrKind::WorkText => {
                args.push(format!("&loft_w{work}"));
                work += 1;
                slot += 1;
            }
            BridgeAttrKind::HiddenDest => {
                // The caller's record or vector when it handed one; a null otherwise, which the
                // body mints, as a C caller hands it.  A vector the caller handed is a place the
                // body fills (`loft_database_wrapper` empties it and builds in it).
                if !matches!(
                    a.typedef.base(),
                    Type::Reference(..) | Type::Enum(_, true, _) | Type::Vector(..)
                ) {
                    return Err("an answer of a shape the bridge does not carry".into());
                }
                args.push(format!(
                    "(loft_n > {slot} && loft_la_ref(loft_a, {slot}).rec != 0 ? loft_la_ref(loft_a, {slot}) : loft_ref_null())"
                ));
                slot += 1;
            }
            // A `&T` argument is the `DbRef` of the slot it names (`marshal_arg_t`); the C
            // function takes a pointer to the value held: read into a local, its address
            // handed in, written back after the call — the callee may bind it anew.
            BridgeAttrKind::Marshal if matches!(a.typedef.base(), Type::RefVar(_)) => {
                let Type::RefVar(inner) = a.typedef.base() else {
                    unreachable!()
                };
                let (get, set, held) = match cx.ctype(inner) {
                    Some(CType::Ref) => ("loft_slot_ref", "loft_slot_set_ref", "loft_ref"),
                    Some(CType::Int) => ("loft_get_int", "loft_set_int", "int64_t"),
                    Some(CType::Float) => ("loft_get_float", "loft_set_float", "double"),
                    _ => {
                        return Err(format!(
                            "parameter `{}` of a shape the bridge does not carry",
                            a.name
                        ));
                    }
                };
                let at = format!("loft_la_ref(loft_a, {slot})");
                let _ = writeln!(places, "    {held} loft_p{slot} = {get}({at}, 0);");
                let _ = writeln!(back, "    {set}({at}, 0, loft_p{slot});");
                args.push(format!("&loft_p{slot}"));
                slot += 1;
            }
            BridgeAttrKind::Marshal => {
                let c = cx
                    .ctype(&a.typedef)
                    .ok_or_else(|| format!("parameter `{}` has no C shape", a.name))?;
                let unc = || {
                    format!(
                        "parameter `{}` of a shape the bridge does not carry",
                        a.name
                    )
                };
                // A tuple crosses as its leaves, one slot each in element order, as the
                // dispatcher pops them (`SharedSig::pops`): the C tuple is built from them.
                if let CType::Tuple(id) = c {
                    let shapes = cx.tuples.borrow()[id as usize].clone();
                    let mut leaves = Vec::new();
                    for leaf in &shapes {
                        // A text leaf is the tuple's own `loft_text`, not a view of the slot.
                        if *leaf == CType::Text {
                            return Err(unc());
                        }
                        leaves.push(slot_arg(*leaf, slot).ok_or_else(unc)?);
                        slot += 1;
                    }
                    let amp = if by_address.contains(&i) { "&" } else { "" };
                    args.push(format!("{amp}(loft_tup{id}) {{ {} }}", leaves.join(", ")));
                    continue;
                }
                args.push(slot_arg(c, slot).ok_or_else(unc)?);
                slot += 1;
            }
        }
    }
    let call = format!("{}({})", c_name(data, d), args.join(", "));
    let plant = match def.attributes().iter().position(|a| !a.hidden) {
        Some(0) if cx.ctype(&def.attributes()[0].typedef) == Some(CType::Ref) => {
            "#ifdef LOFT_HOSTED_PLANT\n    loft_hosted_plant(loft_la_ref(loft_a, 0));\n#endif\n"
        }
        _ => "",
    };
    let ret = cx
        .ctype(def.returned())
        .ok_or_else(|| "a result of no C shape".to_string())?;
    let store = match ret {
        CType::Void => format!("    {call};\n"),
        CType::Int | CType::Int32 => format!("    loft_la_set_bits(loft_r, (int64_t) {call});\n"),
        CType::Bool | CType::Enum | CType::Char => {
            format!("    loft_la_set_bits(loft_r, (int64_t) {call});\n")
        }
        CType::Float => {
            format!(
                "    {{ double loft_v = {call}; int64_t loft_b; memcpy(&loft_b, &loft_v, 8); loft_la_set_bits(loft_r, loft_b); }}\n"
            )
        }
        CType::Single => {
            format!(
                "    {{ float loft_v = {call}; uint32_t loft_b; memcpy(&loft_b, &loft_v, 4); loft_la_set_bits(loft_r, (int64_t) loft_b); }}\n"
            )
        }
        CType::Ref => format!("    loft_la_set_ref(loft_r, {call});\n"),
        // A tuple of scalars crosses as its leaves, one slot each, as the dispatcher reads them
        // (`SharedSig::ret_leaves`): a float as its bits, the rest as an integer.  A tuple with
        // a text, a record or a tuple inside is not carried.
        CType::Tuple(id) => {
            let shapes = cx.tuples.borrow()[id as usize].clone();
            let mut out = format!("    {{ loft_tup{id} loft_v = {call};\n");
            for (i, c) in shapes.iter().enumerate() {
                let slot = format!("loft_r + {i} * LOFT_LA_SIZE");
                let _ = writeln!(
                    out,
                    "{}",
                    match c {
                        CType::Int | CType::Int32 | CType::Bool | CType::Enum | CType::Char => {
                            format!("      loft_la_set_bits({slot}, (int64_t) loft_v.f{i});")
                        }
                        CType::Float => format!(
                            "      {{ int64_t loft_b; memcpy(&loft_b, &loft_v.f{i}, 8); loft_la_set_bits({slot}, loft_b); }}"
                        ),
                        CType::Single => format!(
                            "      {{ uint32_t loft_b; memcpy(&loft_b, &loft_v.f{i}, 4); loft_la_set_bits({slot}, (int64_t) loft_b); }}"
                        ),
                        _ => return Err("a tuple with a text, a record or a tuple inside".into()),
                    }
                );
            }
            out.push_str("    }\n");
            out
        }
        // A text answer crosses as its bytes, which the dispatcher copies into the caller's
        // destination at once (`extensions::shared_store_dispatch_raw`).
        CType::Text => format!(
            "    loft_text_free(&loft_br_answer);\n    loft_br_answer = {call};\n    \
             loft_la_set_text(loft_r, loft_view(&loft_br_answer));\n"
        ),
        _ => return Err("a result of a shape the bridge does not carry".into()),
    };
    let mut held = String::new();
    let mut freed = String::new();
    for k in 0..work {
        let _ = writeln!(held, "    loft_text loft_w{k} = {{ 0 }};");
        let _ = writeln!(freed, "    loft_text_free(&loft_w{k});");
    }
    Ok(format!(
        "\n__attribute__((visibility(\"default\"))) void {symbol}(void *loft_st, const unsigned char *loft_a, size_t loft_n, unsigned char *loft_r) {{\n    \
         void *loft_prev = loft_host_enter(loft_st);\n    (void) loft_n;\n{held}{places}{store}{back}{freed}{plant}    loft_host_leave(loft_prev);\n}}\n"
    ))
}

/// The exports of `export` the C bridge carries: the ones a hosted unit writes.
#[must_use]
pub fn hosted_exports(data: &Data, stores: &Stores, export: &HashSet<u32>) -> HashSet<u32> {
    hosted_split(data, stores, export).0
}

/// [`hosted_exports`], and each export the bridge does not carry with why, by name: what
/// `LOFT_HOSTED_VERIFY` reports a unit's reach with.
#[must_use]
pub fn hosted_split(
    data: &Data,
    stores: &Stores,
    export: &HashSet<u32>,
) -> (HashSet<u32>, Vec<(String, String)>) {
    let mut cx = Cx::new(data, stores, false);
    cx.open = true;
    let (mut carried, mut declined) = (HashSet::new(), Vec::new());
    for &d in export {
        match bridge(&cx, d, "x") {
            Ok(_) => {
                carried.insert(d);
            }
            Err(why) => declined.push((data.def(d).original_name().clone(), why)),
        }
    }
    declined.sort_unstable();
    (carried, declined)
}

/// The unit's exports: a bridge per function of `carried`, named over the library's whole
/// `export` set as the interpreter names it (`native_adopt::bridge_symbol`), then
/// `loft_shared_init` and the layout symbol.
pub(crate) fn exports(
    cx: &Cx<'_>,
    export: &HashSet<u32>,
    carried: &[u32],
    layout_fp: u64,
) -> Result<String, String> {
    let mut out = String::from(SLOTS);
    for &d in carried {
        let symbol = format!(
            "loft_shared_{}",
            crate::opt::names::library_bridge_ident(cx.data, export, d)
        );
        out.push_str(&bridge(cx, d, &symbol)?);
    }
    out.push_str(&cx.upcall_code());
    out.push_str(&type_keys(cx));
    let _ = write!(
        out,
        "\n/* The interpreter's `loft_shared_init`: the table it hands every artifact, whose hosted\n \
         * half this unit keeps; refused when the table or the store layout is another's. */\n\
         __attribute__((visibility(\"default\"))) uint32_t loft_shared_init(const unsigned char *loft_t, const void *loft_ids) {{\n    \
         uint32_t abi;\n    const loft_hosted_table *h;\n    (void) loft_ids;\n    \
         memcpy(&abi, loft_t, 4);\n    if (abi != LOFT_HOST_ABI) return 0;\n    \
         memcpy(&h, loft_t + LOFT_HT_HOSTED, sizeof h);\n    \
         if (h->store_tail != LOFT_HOST_STORE_TAIL) return 0;\n    \
         loft_hosted = h;\n    memcpy(&loft_host_call_ref, loft_t + LOFT_HT_CALL_REF, sizeof loft_host_call_ref);\n    loft_map_types(loft_t, loft_ids);\n    loft_bind_tables();\n    return 1;\n}}\n\
         __attribute__((visibility(\"default\"))) uint64_t loft_type_layout_fp_v1(void) {{ return UINT64_C({layout_fp}); }}\n"
    );
    Ok(out)
}

/// The C compiler a hosted unit is built with, as `build-native` reports it beside the unit.
#[must_use]
pub fn compiler_identity() -> String {
    crate::c_shim::cc_identity()
}

/// What a hosted build's error starts with when the C writer declined the library — a shape
/// it does not carry yet, like a function its bridge declines — rather than failed to build
/// one: the library then interprets, which answers the same, only slower.
pub const REFUSED: &str = "the C writer does not carry this library: ";

/// What a hosted build's error starts with when no C compiler could be run to build the unit:
/// the library then interprets, as a declined one does.
pub const NO_COMPILER: &str = "no C compiler to build the library with: ";

/// `@PLN206 P5` — the key of each of the unit's type numbers (`lib_ids::TypeNames::key`), and
/// `loft_map_types`, which asks the interpreter for its number of each at `loft_shared_init` and
/// fills the two maps a store's root type crosses by (`loft_tp_out`, `loft_tp_in`).  A key the
/// interpreter has no type for maps to none: the unit never writes such a type into its stores.
fn type_keys(cx: &Cx<'_>) -> String {
    let names = crate::lib_ids::TypeNames::new(cx.data, cx.stores);
    let n = cx.stores.types.len();
    let mut out = String::from(
        "\n/* `@PLN206 P5` — each type number's key, for the interpreter's numbers. */\n",
    );
    let _ = writeln!(
        out,
        "static const char *const loft_tkeys[{}] = {{",
        n.max(1)
    );
    for tp in 0..n {
        let key = names.key(tp as u16);
        let _ = writeln!(out, "    {},", super::body::c_string(&key));
    }
    let _ = writeln!(out, "}};\nstatic const uint32_t loft_ntkeys = {n};");
    out.push_str(
        "typedef uint16_t (*loft_type_id_fn)(const void *ids, const uint8_t *key, size_t len);
static void loft_map_types(const unsigned char *loft_t, const void *loft_ids) {
    loft_type_id_fn id;
    uint32_t i, top = 0;
    memcpy(&id, loft_t + LOFT_HT_TYPE_ID, sizeof id);
    free(loft_tp_host);
    free(loft_tp_unit);
    loft_tp_host = (uint16_t *) malloc((loft_ntkeys + 1) * sizeof *loft_tp_host);
    for (i = 0; i < loft_ntkeys; i++) {
        const char *k = loft_tkeys[i];
        uint16_t h = id(loft_ids, (const uint8_t *) k, strlen(k));
        loft_tp_host[i] = h;
        if (h != 0xFFFF && (uint32_t) h + 1 > top) top = (uint32_t) h + 1;
    }
    loft_tp_host_n = loft_ntkeys;
    loft_tp_unit = (uint16_t *) malloc((top + 1) * sizeof *loft_tp_unit);
    for (i = 0; i < top; i++) loft_tp_unit[i] = 0xFFFF;
    /* The first unit type of a key is its type: a later one of the same key (a type variable's
     * placeholder, a structural twin) is the same type to the interpreter. */
    for (i = loft_ntkeys; i-- > 0;) {
        if (loft_tp_host[i] != 0xFFFF) loft_tp_unit[loft_tp_host[i]] = (uint16_t) i;
    }
    loft_tp_unit_n = top;
}
",
    );
    out
}

/// Build the hosted unit for the library functions `export` (each one the C bridge carries,
/// [`hosted_exports`]) as the shared library `stem` in `out_dir`, written for the key `fp`
/// its layout symbol answers (`native_adopt::artifact_matches_layout`).  Published by a rename,
/// so a reader never opens a half-written library.
///
/// # Errors
/// When the unit refuses a function, or the C compiler fails.
pub fn build_artifact(
    data: &mut Data,
    stores: &mut Stores,
    library: &HashSet<u32>,
    pkg_dir: &str,
    out_dir: &std::path::Path,
    stem: &str,
    fp: u64,
) -> Result<std::path::PathBuf, String> {
    use crate::file_access as fa;
    // `@PLN206 P5` — the unit is the library's: `data` and `stores` are the library parsed alone
    // (`native_artifact::parse_library_alone`), so every program that adopts it adopts the same
    // code, whichever built it first.  Its constants are built here, as a program's compile
    // builds its own.
    crate::const_literal::build_const_vectors(stores, data, None);
    let (data, stores) = (&*data, &*stores);
    let export = hosted_exports(data, stores, library);
    let mut carried: Vec<u32> = export.iter().copied().collect();
    carried.sort_unstable();
    if carried.is_empty() {
        return Err(format!(
            "{REFUSED}no function of the library crosses the C bridge"
        ));
    }
    // A source is named from the package's parent directory (`Cx::source_name`).
    let root = std::path::Path::new(pkg_dir)
        .parent()
        .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
    let unit = super::emit_hosted_unit(data, stores, &carried, &export, fp, &root)
        .map_err(|r| format!("{REFUSED}{}", r.message()))?;
    fa::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let c = out_dir.join(format!("{stem}.c"));
    fa::write(&c, &unit).map_err(|e| format!("write {}: {e}", c.display()))?;
    let so = out_dir.join(crate::opt::native_key::platform_cdylib_name(stem));
    let tmp = out_dir.join(format!("{stem}.{}.part", std::process::id()));
    let links: Vec<String> = unit
        .lines()
        .filter_map(|l| l.strip_prefix("/* link: ")?.strip_suffix(" */"))
        .map(str::to_string)
        .chain(
            crate::platform::c_system_link_libs()
                .iter()
                .map(|l| (*l).to_string()),
        )
        .collect();
    let cc = crate::platform::c_unit_compiler();
    // A claim's unwritten bytes read as the interpreter's do under `LOFT_POISON_CLAIM`, which
    // `LOFT_HOSTED_VERIFY` compares by (the artifact's key carries the switch).
    let poison = poison_claim().then_some("-DLOFT_POISON_CLAIM");
    let plant = planted().then_some("-DLOFT_HOSTED_PLANT");
    let ran = crate::platform::process::Spawn::new(crate::platform::process::Program::search(&cc))
        .args(poison)
        .args(plant)
        .args(["-std=c11", "-O2", "-shared", "-fPIC", "-w", "-o"])
        .arg(&tmp)
        .arg(&c)
        .arg("-lm")
        .args(&links)
        .run(&[])
        .map_err(|e| format!("{NO_COMPILER}{cc}: {e}"))?;
    if !ran.status.success() {
        let _ = fa::remove_file(&tmp);
        return Err(format!(
            "{cc} refused the hosted unit {}:\n{}",
            c.display(),
            String::from_utf8_lossy(&ran.stderr)
        ));
    }
    fa::rename(&tmp, &so).map_err(|e| format!("publish {}: {e}", so.display()))?;
    Ok(so)
}
