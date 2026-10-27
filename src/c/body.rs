// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! One function's body as C: statements, expressions and their C shapes.
//!
//! Every temporary is declared at the top of the function, so one created inside a branch is
//! still in scope where it is freed.  An owned text temporary is freed after the statement
//! that made it; every `return` first copies its value out, then frees the function's texts.

mod files;
mod generator;
mod keyed;
mod misc;
mod par;

use super::{CType, Cx, c_name, held, is_operator, type_word};
use crate::data::{Block, Value};
use crate::variables::Function;
use std::fmt::Write as _;

/// A C-safe name for variable `v`: its number first, so two loft names that sanitise alike
/// stay apart.
pub(crate) fn var_name(name: &str, v: u16) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("v{v}_{clean}")
}

/// The shape of a C expression's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K {
    /// `int64_t`.
    Int,
    /// A C condition, 0 or 1.
    Cond,
    /// A stored boolean, `uint8_t` 0, 1 or `LOFT_BOOL_NULL`.
    Bool,
    /// A `loft_str` view.
    View,
    /// A `loft_ref`.
    Ref,
    /// A plain enum value, `uint8_t`.
    Enum,
    /// `double`.
    Float,
    /// `float`.
    Single,
    /// A character's code point, `uint32_t`; 0 is null.
    Char,
    /// A function value, `loft_fn`.
    Fn,
    /// A tuple struct, by its number.
    Tup(u32),
    /// No value.
    Void,
}

/// The collection a new record is added to (`FnWriter::record_collection`).
pub(super) enum Coll {
    /// A plain vector.
    Vector,
    /// A record collection of this type (`Cx::keyed`).
    Keyed(u16),
    /// A member of a linked group: the parent read once, and each member's byte position and
    /// type, the one written first.
    Group(String, Vec<(u16, u16)>),
}

/// A C expression and the shape of its value.
struct E {
    code: String,
    kind: K,
}

impl E {
    fn new(code: impl Into<String>, kind: K) -> E {
        E {
            code: code.into(),
            kind,
        }
    }
}

/// The expression kind that reads a held value of shape `c`.
fn kind_of(c: CType) -> K {
    match c {
        CType::Int => K::Int,
        CType::Bool => K::Bool,
        CType::Text => K::View,
        CType::Ref => K::Ref,
        CType::Enum => K::Enum,
        CType::Float => K::Float,
        CType::Single => K::Single,
        CType::Char => K::Char,
        CType::Fn => K::Fn,
        CType::Tuple(id) => K::Tup(id),
        CType::Void => K::Void,
    }
}

/// One function's body under construction.
pub(crate) struct FnWriter<'a> {
    cx: &'a Cx<'a>,
    /// The function's definition number.
    d: u32,
    vars: &'a Function,
    code: &'a Value,
    /// The C shape of every variable, parameters first.
    shapes: Vec<CType>,
    /// Which variables are the function's parameters (`Function::arguments`).
    params: Vec<bool>,
    /// Which parameters are `&` links: a pointer to the caller's variable, read and written
    /// through (`(*v)`), as the bytecode reads and writes through the stack reference.
    links: Vec<bool>,
    /// Which `&` locals link a STORE place (`c = &v[1]`, `f = &o.x`): every bind names an
    /// element or a field, never a variable.  Held as the place's `loft_ref`; a read or a
    /// write goes through the place's own operator, as the bytecode's `OpGetInt`/`OpSetInt`
    /// through the link's `DbRef`.  A link bound to a variable on one path and a place on
    /// another is not carried.
    store_links: Vec<bool>,
    ret: CType,
    body: String,
    /// Every temporary: its name and shape, declared at the top.
    temps: Vec<(String, CType)>,
    /// The owned text temporaries the current statement made.
    pending: Vec<String>,
    /// The label number of each enclosing loop, innermost last.
    loops: Vec<u32>,
    labels: u32,
    /// The vectors each enclosing loop holds (`loft_vh`), innermost last (@FR-R-Header), each
    /// with whether its element base stays valid for the loop's extent (`@FR-R-Base`: the
    /// loop grows no store).  An `__inv` twin starts with its inputs as one frame.
    headers: Vec<Vec<(crate::opt::writes::PathKey, String, bool)>>,
    /// The record push headers each enclosing loop binds (`loft_ph`, `@FR-R-PushRec`): the
    /// path, the header and the element size, innermost last.
    push_heads: Vec<Vec<(crate::opt::writes::PathKey, String, u32)>>,
    /// The header gate's memo of which callees write a store.
    hoist_cache: std::collections::HashMap<u32, bool>,
    /// The locals `(R-Place)` put in another record's store (`hoist::placed_locals`), for
    /// the store facts the loop verdict proves bases apart with.
    placed: std::collections::HashSet<u16>,
    /// `@FR-R-Alias`'s versioned clause: the variable pairs the loop copy being written may
    /// take as holding apart stores, tested at its entry.  Empty outside such a copy.
    assumed: Vec<(u16, u16)>,
    /// `@FR-R-Range` (`opt::range`, native's proof): this function's non-null and range facts,
    /// computed on first use.
    range_facts: Option<
        std::rc::Rc<(
            std::collections::HashMap<u16, bool>,
            crate::opt::range::Ranges,
        )>,
    >,
    /// Declarations beside the temporaries: the held vectors.
    decls: Vec<String>,
    /// Which variables a statement written so far has bound: a later `Set` is a rebind.
    bound: Vec<bool>,
    /// A generator's element shape (`body::generator`): its `yield`s and its `return` differ.
    gen_elem: Option<CType>,
    /// The `yield`s written so far, each a resume label.
    yields: u32,
    /// The source line the statements now being written come from (`Value::Line`), which a
    /// call records as its frame's site when the unit keeps frames (`Cx::frames`).
    cur_line: u32,
    /// `LOFT_C_WHERE` reported this function's refusal already.
    where_told: bool,
    /// `@FR-R-RetAdopt` — the result local that IS the hidden return buffer
    /// (`opt::ret_adopt::ret_adopt`, the analysis the Rust generator reads): it builds in
    /// the caller's buffer, its witness store is never minted, and the delivery copy at the
    /// exit has nothing to move.  `LOFT_NO_RETBUF_ADOPT=1` keeps the copies.
    adopt: Option<crate::opt::ret_adopt::RetAdopt>,
    /// Inside an adopted function's `one_buffer_vec_copy` delivery block.
    in_delivery: u32,
    /// `@FR-R-PushFill`'s window clause — the push windows the enclosing counted loops hold:
    /// each path, its `loft_win`, the `loft_ref` the vector operand was read into, and the
    /// element width.
    windows: Vec<(crate::opt::writes::PathKey, String, String, u32)>,
    /// `@FR-R-TextBorrow` (`opt::text_borrow::borrowed_text_walks`, the analysis native reads): the
    /// text locals a walk binds to an element it only READS, under a loop that writes no store —
    /// each a `loft_str` view of the element, as a text parameter is, never a copy.
    borrowed: Vec<bool>,
    /// `@FR-R-CharWalk` (`opt::char_walk::char_walks`, which the interpreter and native read):
    /// this function's `for c in t` walks by loop scope — their step takes an ASCII byte without
    /// decoding it, and their null test is asked once where the loop never writes `t`.
    /// `LOFT_NO_CHAR_WALK=1` writes every step as the lowering gives it.
    char_walks: std::collections::BTreeMap<u16, crate::opt::char_walk::CharWalk>,
    /// `@FR-R-LazySplit` (`hoist::lazy_splits`, native's analysis): the hidden vectors of
    /// `for piece in t.split(c)` loops, by variable — each walked as a `loft_ls` over the text,
    /// the vector never built.  `LOFT_NO_LAZY_SPLIT=1` builds them, as on native.
    lazy: std::collections::BTreeMap<u16, crate::opt::lazy_split::LazySplit>,
    /// `@FR-R-CompleteWrite` (`hoist::complete_writes`): the element types every mint of this
    /// function writes completely, so the default image copied into a fresh element writes
    /// nothing that survives; computed on first use.
    complete_mints: Option<std::collections::HashSet<u16>>,
    /// `@FR-R-FileHold` (`lower::file_hold::held_file`, native's gate): the files the enclosing
    /// loops read through a held cursor, each with its `loft_fcur` and the index in `loops` of
    /// the loop that holds it.
    file_holds: Vec<(u16, String, usize)>,
}

/// A counted loop a push window is opened for (`FnWriter::window_loop`): the pushed path,
/// its vector operand, the range that counts its passes and the pushes one pass runs at most.
struct WindowLoop<'b> {
    path: crate::opt::writes::PathKey,
    vector: std::borrow::Cow<'b, Value>,
    hi: &'b Value,
    index_var: u16,
    next_var: Option<u16>,
    inclusive: bool,
    pushes: u32,
    size: u32,
}

/// `@FR-R-TextBorrow` — the text locals of function `d` the C holds as a `loft_str` view rather
/// than an owned copy: the walk variables `opt::text_borrow::borrowed_text_walks` admits and the
/// plain locals of `hoist::borrowed_store_texts` (its run-time-tested and work-text clauses need
/// emitter support the C does not have, so they keep their copies).  A local is kept only when
/// EVERY bind of it is a shape the view form writes without a temporary — another borrowed
/// local or a text parameter, a literal, a text field read, a block ending in one — so
/// admitting it never turns a carried program into a refused one.
/// `LOFT_NO_TEXT_BORROW=1` keeps every copy, as on native.
fn borrowed_texts(cx: &Cx, d: u32) -> Vec<bool> {
    let def = cx.data.def(d);
    let vars = def.variables();
    let n = vars.count() as usize;
    let mut b = vec![false; n];
    if std::env::var("LOFT_NO_TEXT_BORROW").is_ok_and(|v| v != "0") {
        return b;
    }
    let walks = crate::opt::text_borrow::borrowed_text_walks(
        cx.data,
        d,
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeMap::new(),
    );
    let store = crate::opt::store_text_borrow::borrowed_store_texts(cx.data, d, &walks);
    let plain = store
        .locals
        .iter()
        .filter(|v| !store.tested.contains_key(v) && !store.work.contains(v))
        .copied();
    for v in walks.keys().copied().chain(plain) {
        let i = v as usize;
        if i < n
            && !vars.is_argument(v)
            && matches!(vars.tp(v).base(), crate::data::Type::Text(_))
            && !matches!(vars.tp(v).base(), crate::data::Type::RefVar(_))
        {
            b[i] = true;
        }
    }
    // Every bind of a candidate, gathered once.
    let mut binds: Vec<(u16, &Value)> = Vec::new();
    def.code().any_node(&mut |node| {
        if let Value::Set(v, x) = node
            && b.get(*v as usize).copied().unwrap_or(false)
        {
            binds.push((*v, x));
        }
        false
    });
    loop {
        let mut dropped = false;
        for (v, x) in &binds {
            if b[*v as usize] && !view_bind(cx, vars, &b, x) {
                b[*v as usize] = false;
                dropped = true;
            }
        }
        if !dropped {
            return b;
        }
    }
}

/// Is `x` a text the view form of a borrowed bind writes with no temporary (`borrowed_texts`)?
fn view_bind(cx: &Cx, vars: &Function, borrowed: &[bool], x: &Value) -> bool {
    match x.unspan() {
        Value::Block(bl) => bl
            .operators
            .iter()
            .rev()
            .find(|o| !matches!(o.unspan(), Value::Line(_)))
            .is_some_and(|last| view_bind(cx, vars, borrowed, last)),
        Value::Var(w) => {
            borrowed.get(*w as usize).copied().unwrap_or(false)
                || (vars.is_argument(*w)
                    && matches!(vars.tp(*w).base(), crate::data::Type::Text(_))
                    && !matches!(vars.tp(*w).base(), crate::data::Type::RefVar(_)))
        }
        Value::Text(_) => true,
        // A substring is left out: the C may copy its source into a statement temporary first,
        // which a view would outlive (three corpus programs did).
        Value::Call(d, _) => cx.data.def(*d).name() == "OpGetText",
        _ => false,
    }
}

impl<'a> FnWriter<'a> {
    pub(crate) fn new(cx: &'a Cx<'a>, d: u32, shapes: Vec<CType>, ret: CType) -> FnWriter<'a> {
        let def = cx.data.def(d);
        FnWriter {
            cx,
            d,
            vars: def.variables(),
            code: def.code(),
            shapes,
            params: {
                let vars = def.variables();
                let mut p = vec![false; vars.count() as usize];
                for a in vars.arguments() {
                    p[a as usize] = true;
                }
                p
            },
            links: (0..def.variables().count())
                .map(|v| matches!(def.variables().tp(v).base(), crate::data::Type::RefVar(_)))
                .collect(),
            // A store instance's text parameter (`n_app@st1`, `parser/store_text.rs`) is a
            // store link: the IR reads and writes it as the field it names.
            store_links: {
                let vars = def.variables();
                let seed: Vec<u16> = vars
                    .arguments()
                    .into_iter()
                    .filter(|&a| vars.is_store_text_link(a))
                    .collect();
                store_links(cx, def, &seed)
            },
            ret,
            body: String::new(),
            temps: Vec::new(),
            pending: Vec::new(),
            loops: Vec::new(),
            labels: 0,
            headers: Vec::new(),
            push_heads: Vec::new(),
            hoist_cache: std::collections::HashMap::new(),
            placed: crate::generation::hoist::placed_locals(def.code(), cx.data),
            assumed: Vec::new(),
            range_facts: None,
            cur_line: 0,
            where_told: false,
            adopt: if std::env::var("LOFT_NO_RETBUF_ADOPT").is_ok_and(|v| v != "0") {
                None
            } else {
                crate::opt::ret_adopt::ret_adopt(cx.data, d)
            },
            in_delivery: 0,
            windows: Vec::new(),
            file_holds: Vec::new(),
            borrowed: borrowed_texts(cx, d),
            lazy: if std::env::var("LOFT_NO_LAZY_SPLIT").is_ok_and(|v| v != "0") {
                std::collections::BTreeMap::new()
            } else {
                crate::opt::lazy_split::lazy_splits(cx.data, d)
            },
            char_walks: if std::env::var("LOFT_NO_CHAR_WALK").is_ok_and(|v| v != "0") {
                std::collections::BTreeMap::new()
            } else {
                crate::opt::char_walk::char_walks(cx.data, d)
            },
            complete_mints: None,
            decls: Vec::new(),
            // A parameter arrives bound: its first `Set` is a rebind.
            bound: {
                let vars = def.variables();
                let mut b = vec![false; vars.count() as usize];
                for a in vars.arguments() {
                    b[a as usize] = true;
                }
                b
            },
            gen_elem: None,
            yields: 0,
        }
    }

    /// loft#1254 — a stub with an empty body answers its type's default (`Data::to_default`),
    /// a handle-carried result the null reference, as the bytecode writer's stub path does;
    /// `None` for any other body, or a type with no default.
    fn empty_stub_default(&self) -> Option<Value> {
        use crate::data::Type;
        if !matches!(self.code, Value::Block(b) if b.operators.is_empty()) {
            return None;
        }
        let data = self.cx.data;
        let ret = data.def(self.d).returned();
        if data.has_default(ret).is_err() {
            return None;
        }
        Some(
            if matches!(
                ret.base(),
                Type::Reference(..)
                    | Type::Vector(..)
                    | Type::Sorted(..)
                    | Type::Index(..)
                    | Type::Hash(..)
                    | Type::Radix(..)
                    | Type::Trie(..)
                    | Type::Enum(_, true, _)
            ) {
                Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new())
            } else {
                crate::data::to_default(ret, data)
            },
        )
    }

    /// This is the place twin for `mask` (`twin_name`): each `&` parameter whose bit is set is
    /// a store link — a `loft_ref` to the place — as a `&` local bound to a field is.
    pub(crate) fn places(mut self, mask: u64) -> Self {
        if mask != 0 {
            let def = self.cx.data.def(self.d);
            let seed: Vec<u16> = self
                .vars
                .arguments()
                .iter()
                .enumerate()
                .filter(|&(i, &a)| mask & (1 << i) != 0 || self.vars.is_store_text_link(a))
                .map(|(_, &a)| a)
                .collect();
            self.store_links = store_links(self.cx, def, &seed);
        }
        self
    }

    /// This is an `__inv` twin (`@FR-R-Inputs`): `frame` is the headers its caller hands it,
    /// valid for the call, since an admitted callee grows no store.
    pub(crate) fn inputs(
        mut self,
        frame: Vec<(crate::opt::writes::PathKey, String, bool)>,
    ) -> Self {
        if !frame.is_empty() {
            self.headers.push(frame);
        }
        self
    }

    /// This function is a generator of `elem`s: written by `write_gen`.
    pub(crate) fn yielding(mut self, elem: CType) -> Self {
        self.gen_elem = Some(elem);
        self
    }

    /// The body: the locals, the temporaries and the statements.
    pub(crate) fn write(mut self) -> Result<String, String> {
        let code = self.code;
        if self.ret == CType::Void {
            self.stmt(code, 1)?;
            self.epilogue(1);
        } else if let Some(dflt) = self.empty_stub_default() {
            self.ret_value(Some(&dflt), 1)?;
        } else {
            self.ret_value(Some(code), 1)?;
        }
        let mut out = String::new();
        for v in (0..self.shapes.len()).filter(|v| !self.params[*v]) {
            let c = self.shapes[v];
            if self.store_links[v] {
                let name = var_name(self.vars.name(v as u16), v as u16);
                let _ = writeln!(out, "    loft_ref {name} = {{ 0xFFFF, 0, 0 }};");
                continue;
            }
            if self.links[v] {
                let name = var_name(self.vars.name(v as u16), v as u16);
                let _ = writeln!(out, "    {} *{name} = 0;", held(c));
                continue;
            }
            if self.borrowed[v] && c == CType::Text {
                let _ = writeln!(out, "    loft_str {} = {{ 0, 0 }};", self.var(v as u16));
                continue;
            }
            let _ = writeln!(
                out,
                "    {} {} = {};",
                held(c),
                self.var(v as u16),
                self.cx.initial(c)
            );
        }
        for (t, c) in &self.temps {
            let _ = writeln!(out, "    {} {t} = {};", held(*c), self.cx.initial(*c));
        }
        for d in &self.decls {
            let _ = writeln!(out, "    {d}");
        }
        let slots: Vec<String> = self.held_texts().iter().map(|t| format!("&{t}")).collect();
        if !slots.is_empty() {
            let _ = writeln!(out, "    LOFT_HOLD({}, {});", slots.len(), slots.join(", "));
        }
        out.push_str(&self.body);
        Ok(out)
    }

    fn var(&self, v: u16) -> String {
        let name = var_name(self.vars.name(v), v);
        if self.links[v as usize] {
            format!("(*{name})")
        } else {
            name
        }
    }

    /// A text variable this function holds as a `loft_text`: a local, or the caller's
    /// through a link; a plain text parameter is a borrowed `loft_str`.
    fn text_held(&self, v: u16) -> bool {
        self.shapes[v as usize] == CType::Text
            && (!self.params[v as usize] || self.links[v as usize])
            && !self.borrowed[v as usize]
    }

    fn line(&mut self, ind: usize, s: &str) {
        let _ = writeln!(self.body, "{}{s}", "    ".repeat(ind));
    }

    fn temp(&mut self, c: CType) -> String {
        let t = format!("t{}", self.temps.len() + 1);
        self.temps.push((t.clone(), c));
        t
    }

    /// Free the owned text temporaries the statement just finished made.
    fn flush(&mut self, ind: usize) {
        for t in std::mem::take(&mut self.pending) {
            self.line(ind, &format!("loft_text_free(&{t});"));
        }
    }

    /// `@FR-R-FileHold` — give back the cursor of every hold of a loop at `loops` index `from`
    /// or deeper, innermost first: what Rust's guard does by its drop.  Loft has no exception,
    /// so these are every exit there is: the loop's own end (`loop_stmt`), a `break` or a
    /// `continue` to an outer loop, a `return`.
    fn release_holds(&mut self, from: usize, ind: usize) {
        let gone: Vec<String> = self
            .file_holds
            .iter()
            .rev()
            .filter(|(_, _, at)| *at >= from)
            .map(|(_, c, _)| c.clone())
            .collect();
        for c in gone {
            self.line(ind, &format!("loft_fcur_release(&{c});"));
        }
    }

    /// Free every text the function holds: its locals and its temporaries.
    fn epilogue(&mut self, ind: usize) {
        // A `return` from inside a held loop: the cursors go back before the frame does.
        self.release_holds(0, ind);
        for t in self.held_texts() {
            self.line(ind, &format!("loft_text_free(&{t});"));
        }
    }

    /// The texts the function holds, each freed by its epilogue — and released by a halt in a
    /// unit that runs tests (`LOFT_HOLD`, loft#2198): its text locals, the texts its tuple
    /// locals own, its text temporaries and the texts its tuple temporaries hold.
    fn held_texts(&self) -> Vec<String> {
        let mut texts: Vec<String> = (0..self.shapes.len())
            .filter(|&v| {
                !self.params[v]
                    && !self.links[v]
                    && !self.borrowed[v]
                    && self.shapes[v] == CType::Text
            })
            .map(|v| self.var(v as u16))
            .collect();
        // The texts a tuple local owns.
        for v in (0..self.shapes.len()).filter(|&v| !self.params[v] && !self.links[v]) {
            if let CType::Tuple(id) = self.shapes[v] {
                self.tuple_text_leaves(&self.var(v as u16), id, &mut texts);
            }
        }
        texts.extend(
            self.temps
                .iter()
                .filter(|(_, c)| *c == CType::Text)
                .map(|(t, _)| t.clone()),
        );
        // And the texts a tuple temporary holds.
        let tuple_temps: Vec<(String, u32)> = self
            .temps
            .iter()
            .filter_map(|(t, c)| match c {
                CType::Tuple(id) => Some((t.clone(), *id)),
                _ => None,
            })
            .collect();
        for (t, id) in tuple_temps {
            self.tuple_text_leaves(&t, id, &mut texts);
        }
        texts
    }

    /// Before loop `b`: each vector the loop verdict holds (`hoist::hoistable`, native's own —
    /// the vectors the body indexes and the ones its admitted callees index, `@FR-R-Inputs`)
    /// in a `loft_vh`, the frame the loop's element accesses read.  A loop that grows a store
    /// (a push, a mint, a text set) keeps the record and the length, which no growth moves; it
    /// keeps the base only of a vector whose store is proven apart from every store it grows
    /// (`@FR-R-Base`'s growth clause, `StoreFacts`), and derives the others' base again at
    /// each access (`loft_vh_fresh`).  `LOFT_NO_VECTOR_HOIST=1` holds none; the tier switches
    /// are native's (`lower::switches`).
    /// Answers the minted paths the verdict gives a record push header (`mint_pushes`), for
    /// [`Self::hold_pushes`] once the loop's window is chosen.
    fn hold_vectors(
        &mut self,
        b: &Block,
        ind: usize,
    ) -> Result<Vec<(crate::opt::writes::PathKey, Value)>, String> {
        let mut frame = Vec::new();
        let mut mints = Vec::new();
        if let Some(verdict) = self.verdict(b) {
            mints.clone_from(&verdict.mint_pushes);
            let bases = self.kept_bases(&verdict);
            for ((path, expr), base) in verdict.vectors.into_iter().zip(bases) {
                // A path an enclosing frame holds as well — with a base, or without one where
                // this loop would keep none either — is served by that frame.
                let outer = self
                    .headers
                    .iter()
                    .rev()
                    .flat_map(|f| f.iter())
                    .find(|(p, _, _)| *p == path)
                    .map(|(_, _, b)| *b);
                if outer.is_some_and(|b| b || !base) {
                    continue;
                }
                let e = self.nested(|w| w.expr(&expr, ind))?;
                let h = format!("h{}", self.decls.len() + 1);
                self.decls
                    .push(format!("loft_vh {h} = {{ 0xFFFF, 0, 0, 0 }};"));
                self.line(ind, &format!("{h} = loft_vh_of({});", as_ref(e)?));
                frame.push((path, h, base));
            }
        }
        self.headers.push(frame);
        Ok(mints)
    }

    /// `@FR-R-PushRec`, as native's record push header: before loop `b`, a `loft_ph` for each
    /// minted path of `mints` that no window and no enclosing loop serves; the path's mints and
    /// finishes go through it (`Self::pushed_mint`).  `LOFT_NO_RECORD_PUSH=1` admits none, in
    /// the verdict, as on native.
    fn hold_pushes(
        &mut self,
        b: &Block,
        mints: Vec<(crate::opt::writes::PathKey, Value)>,
        ind: usize,
    ) -> Result<(), String> {
        let mut frame = Vec::new();
        for (path, expr) in mints {
            if self.windows.iter().any(|(p, _, _, _)| *p == path) || self.push_head(&path).is_some()
            {
                continue;
            }
            let Some(size) = self.mint_size(b, &path) else {
                continue;
            };
            let e = self.nested(|w| w.expr(&expr, ind))?;
            let h = format!("ph{}", self.decls.len() + 1);
            self.decls
                .push(format!("loft_ph {h} = {{ {{ 0xFFFF, 0, 0 }}, 0, 0, 0 }};"));
            self.line(ind, &format!("{h} = loft_ph_of({}, {size});", as_ref(e)?));
            crate::rewrite_census::fired("R-PushRec", 1);
            frame.push((path, h, size));
        }
        self.push_heads.push(frame);
        Ok(())
    }

    /// The element size of the records loop `b` mints into `path`, read off its first mint.
    fn mint_size(&self, b: &Block, path: &crate::opt::writes::PathKey) -> Option<u32> {
        let data = self.cx.data;
        let mut size = None;
        for op in &b.operators {
            op.any_node(&mut |n| {
                if size.is_none()
                    && let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpNewRecord"
                    && let Some(t) = crate::opt::writes::mint_target(
                        data,
                        self.cx.stores,
                        "OpNewRecord",
                        args,
                        self.vars,
                    )
                    && t.path == *path
                {
                    size = Some(u32::from(
                        self.cx.stores.size(self.cx.stores.content(t.vector_tp)),
                    ));
                }
                false
            });
        }
        size
    }

    /// The record push header an enclosing loop binds for `path`, with its element size.
    fn push_head(&self, path: &crate::opt::writes::PathKey) -> Option<(String, u32)> {
        self.push_heads
            .iter()
            .rev()
            .flat_map(|f| f.iter())
            .find(|(p, _, _)| p == path)
            .map(|(_, h, size)| (h.clone(), *size))
    }

    /// A mint, a finish or a length of a path a record push header serves.
    fn pushed_mint(&mut self, name: &str, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        let data = self.cx.data;
        if name == "OpLengthVector" {
            return Ok(args
                .first()
                .and_then(|v| crate::opt::writes::vector_path(data, v))
                .and_then(|p| self.push_head(&p))
                .map(|(h, _)| E::new(format!("((int64_t) {h}.len)"), K::Int)));
        }
        // A group's reservation is capacity, observably nothing; the header's mint grows the
        // record itself, and a reservation made beside it would claim a record it does not hold.
        if name == "OpPreAllocVector" {
            return Ok(args
                .first()
                .and_then(|v| crate::opt::writes::vector_path(data, v))
                .and_then(|p| self.push_head(&p))
                .map(|_| E::new("((void) 0)", K::Void)));
        }
        if !matches!(name, "OpNewRecord" | "OpFinishRecord") {
            return Ok(None);
        }
        let Some(t) = crate::opt::writes::mint_target(data, self.cx.stores, name, args, self.vars)
        else {
            return Ok(None);
        };
        let Some((h, size)) = self.push_head(&t.path) else {
            return Ok(None);
        };
        if name == "OpFinishRecord" {
            return Ok(Some(E::new(format!("loft_ph_finish(&{h})"), K::Void)));
        }
        let elem = self.cx.stores.content(t.vector_tp);
        let ptp = u16::try_from(Self::literal(&args[1], "type")?)
            .map_err(|_| "a type number out of range")?;
        let proof_at = format!("{}:{h}", data.def(self.d).original_name());
        let e = self.temp(CType::Ref);
        self.line(
            ind,
            &format!(
                "{e} = (LOFT_PROVED(loft_ph_same({h}, {size}), {}), loft_ph_new(&{h}, {size}));",
                c_string(&proof_at)
            ),
        );
        if !self.mint_complete(ptp) {
            let (image, n) = self.cx.default_image(elem);
            self.line(ind, &format!("loft_fill({e}, {image}, {n});"));
        }
        Ok(Some(E::new(e, K::Ref)))
    }

    /// The store facts the verdict proves stores apart with (`hoist::StoreFacts`), under
    /// the versioned copy's `assumed` pairs; `None` under `LOFT_NO_DISTINCT_GROWTH=1`.
    fn store_facts(&self) -> Option<crate::generation::hoist::StoreFacts<'_>> {
        (!self.cx.switches.distinct_growth_disabled).then(|| crate::generation::hoist::StoreFacts {
            vars: self.vars,
            placed: &self.placed,
            adopted: self.adopt.as_ref().map(|a| (a.v, a.vdb)),
            owned: None,
            assumed: &self.assumed,
        })
    }

    /// Loop `b`'s header verdict (`hoist::hoistable`), or `None` where none is asked: in a
    /// generator, whose locals live in its frame between resumes, or under
    /// `LOFT_NO_VECTOR_HOIST=1`.
    fn verdict(&mut self, b: &Block) -> Option<crate::generation::hoist::LoopHoist> {
        if self.gen_elem.is_some() || std::env::var("LOFT_NO_VECTOR_HOIST").is_ok_and(|v| v != "0")
        {
            return None;
        }
        let sw = self.cx.switches;
        let tiers = crate::opt::writes::HoistTiers {
            in_place: !sw.write_hoist_disabled,
            scalars: false,
            push: !sw.push_hoist_disabled,
            mint: !sw.mint_hoist_disabled,
            record_push: !sw.record_push_disabled,
            heap_push: !sw.heap_record_push_disabled,
            rebound_movers: !sw.rebound_mover_disabled,
            text_set: false,
        };
        let mut cache = std::mem::take(&mut self.hoist_cache);
        let facts = self.store_facts();
        let memo = &mut *self.cx.hoist_memo.borrow_mut();
        let v = crate::generation::hoist::hoistable(
            b,
            self.cx.data,
            self.cx.stores,
            self.d,
            &mut cache,
            &mut memo.writes,
            tiers,
            (!sw.callee_inputs_disabled).then_some(&mut memo.inputs),
            None,
            facts.as_ref(),
        );
        self.hoist_cache = cache;
        Some(v)
    }

    /// Per held vector of `v`, whether its base stays valid for the loop's extent: every one
    /// in a loop that grows no store, else each whose store is proven apart from every store
    /// the loop grows (`StoreFacts::distinct`), as native's `begin_vector_hoist` decides.
    fn kept_bases(&self, v: &crate::generation::hoist::LoopHoist) -> Vec<bool> {
        if v.growth_free {
            return vec![true; v.vectors.len()];
        }
        let facts = self.store_facts();
        v.vectors
            .iter()
            .map(|(p, _)| {
                !v.movers.is_empty()
                    && facts
                        .as_ref()
                        .is_some_and(|f| v.movers.iter().all(|m| f.distinct(p.0, *m)))
            })
            .collect()
    }

    /// `@FR-R-Alias`'s versioned clause, as native's `distinct_version`: when loop `lp` grows
    /// this function's return buffer and would hold more headers if each parameter's store were
    /// apart from the buffer's, answer the pairs to test at run time and the pairs the tested
    /// copy may assume apart.  `None` for a loop that gains nothing, cannot be copied (a
    /// `yield`, a `par`, a function-value call), or under `LOFT_NO_DISTINCT_VERSION=1`.
    #[expect(clippy::type_complexity, reason = "two pair lists, named in the doc")]
    fn distinct_version(&mut self, lp: &Block) -> Option<(Vec<(u16, u16)>, Vec<(u16, u16)>)> {
        let sw = self.cx.switches;
        if sw.distinct_version_disabled
            || sw.distinct_growth_disabled
            || !self.assumed.is_empty()
            || lp.operators.iter().any(|op| {
                op.any_node(&mut |n| {
                    matches!(
                        n,
                        Value::Yield(_) | Value::Parallel(_) | Value::CallRef(_, _)
                    )
                })
            })
        {
            return None;
        }
        let data = self.cx.data;
        let rb = crate::opt::attrs::retbuf_var(data, self.d)?;
        let adopted = self
            .adopt
            .as_ref()
            .filter(|a| a.buf == rb)
            .map(|a| (a.v, a.vdb));
        let bufs: Vec<u16> = std::iter::once(rb)
            .chain(adopted.into_iter().flat_map(|(v, vdb)| [v, vdb]))
            .collect();
        let tested = adopted.map_or(rb, |(v, _)| v);
        let vars = self.vars;
        let params: Vec<u16> = (0..vars.count())
            .filter(|&v| vars.is_argument(v) && v != rb && crate::data::is_dbref(vars.tp(v).base()))
            .collect();
        if params.is_empty() {
            return None;
        }
        let plain = self.verdict(lp)?;
        let grows =
            crate::generation::hoist::mover_roots(&lp.operators, data, self.cx.stores, vars)
                .iter()
                .any(|&m| chain_reaches(vars, m, &bufs));
        if plain
            .pushes
            .iter()
            .chain(plain.mint_pushes.iter())
            .all(|(p, _)| !bufs.contains(&p.0))
            && !plain.movers.iter().any(|m| bufs.contains(m))
            && !grows
        {
            return None;
        }
        let assumed: Vec<(u16, u16)> = params
            .iter()
            .flat_map(|&p| bufs.iter().map(move |&b| (p, b)))
            .collect();
        self.assumed.clone_from(&assumed);
        let versioned = self.verdict(lp);
        self.assumed.clear();
        let versioned = versioned?;
        // A path counts only when no enclosing frame holds it already.
        let mut gained: Vec<u16> = Vec::new();
        for (p, _) in &versioned.vectors {
            if !plain.vectors.iter().any(|(q, _)| q == p)
                && self.held_path(p).is_none()
                && let Some(r) = param_root(vars, p.0, &params)
                && !gained.contains(&r)
            {
                gained.push(r);
            }
        }
        if gained.is_empty() {
            return None;
        }
        let mut tests: Vec<(u16, u16)> = Vec::new();
        let mut pairs: Vec<(u16, u16)> = Vec::new();
        for r in gained {
            if !tests.contains(&(r, tested)) {
                tests.push((r, tested));
            }
            for &b in &bufs {
                if !pairs.contains(&(r, b)) {
                    pairs.push((r, b));
                }
            }
        }
        Some((tests, pairs))
    }

    /// `@FR-R-PushFill`'s window clause, as the Rust writer's `push_reserve`: before a COUNTED
    /// push loop (`hoist::push_loop`) whose pushes nothing else in the body can observe
    /// (`hoist::push_window_ok`: the root exclusive, the other statements naming no store it
    /// may view and writing none), reserve the loop's pushes once and open a `loft_win` on the
    /// vector; every push of the path then goes through it, its per-push pre-allocation is
    /// the reservation's, and the length is written once at the loop's exit.  A reservation
    /// is capacity, observably nothing.  `LOFT_NO_PUSH_FILL=1` / `LOFT_NO_PUSH_WINDOW=1` open
    /// none, as on native.  Answers whether a window was opened.
    fn open_window(&mut self, lp: &Block, ind: usize) -> Result<bool, String> {
        let env_on = |k: &str| std::env::var(k).is_ok_and(|v| v != "0");
        if self.gen_elem.is_some() || env_on("LOFT_NO_PUSH_FILL") || env_on("LOFT_NO_PUSH_WINDOW") {
            return Ok(false);
        }
        let Some(p) = self.window_loop(lp) else {
            return self.deep_window(lp, ind);
        };
        let n = self.decls.len() + 1;
        let (w, pv) = (format!("pw{n}"), format!("pv{n}"));
        self.decls
            .push(format!("loft_win {w} = {{ 0, 0, 0, 0, 0xFFFF }};"));
        self.decls
            .push(format!("loft_ref {pv} = {{ 0xFFFF, 0, 0 }};"));
        let vector = self.expr(&p.vector, ind)?;
        self.line(ind, &format!("{pv} = {};", as_ref(vector)?));
        // The trip count: the range's end less its start (the `next` counter, or the index
        // the loop steps from plus one), plus one for an inclusive range — checked, so a
        // count that does not fit reserves nothing.
        let hi = self.expr(p.hi, ind)?;
        let hi = as_int(hi)?;
        let lo = match p.next_var {
            Some(nx) => self.var(nx),
            None => format!("loft_add({}, INT64_C(1))", self.var(p.index_var)),
        };
        let incl = u8::from(p.inclusive);
        self.flush(ind);
        self.line(
            ind,
            &format!(
                "loft_vec_reserve_more({pv}, loft_mul(loft_add(loft_sub({hi}, {lo}), INT64_C({incl})), INT64_C({})), {});",
                p.pushes, p.size
            ),
        );
        self.line(ind, &format!("{w} = loft_win_of({pv}, {});", p.size));
        self.windows.push((p.path, w, pv, p.size));
        Ok(true)
    }

    /// `@FR-R-PushFill`'s DEEP window clause, as native's `deep_window`: a loop whose scalar
    /// pushes, at any depth, reach ONE path no enclosing window holds, where
    /// `hoist::deep_window_ok` admits it — the root exclusive, nothing else in the body naming
    /// it or writing a store, no exit but the loop's own end — pushes through a window opened
    /// with no reservation: a push that does not fit takes the window's growth arm.
    fn deep_window(&mut self, lp: &Block, ind: usize) -> Result<bool, String> {
        let env_on = |k: &str| std::env::var(k).is_ok_and(|v| v != "0");
        if env_on("LOFT_NO_DEEP_WINDOW") {
            return Ok(false);
        }
        let data = self.cx.data;
        let mut paths: Vec<(crate::opt::writes::PathKey, Value, u32)> = Vec::new();
        for op in &lp.operators {
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && let Some(fp) =
                        crate::opt::writes::fused_push(data, data.def(*d).name(), args)
                    && !paths.iter().any(|(p, _, _)| *p == fp.path)
                {
                    paths.push((fp.path, fp.vector.clone(), fp.size));
                }
                false
            });
        }
        let [(path, vector, size)] = paths.as_slice() else {
            return Ok(false);
        };
        if self.windows.iter().any(|(p, _, _, _)| p == path) {
            return Ok(false);
        }
        let mut cache = std::mem::take(&mut self.hoist_cache);
        let facts = self.store_facts();
        let ok = crate::generation::hoist::deep_window_ok(
            lp,
            path,
            data,
            self.d,
            Some(self.cx.stores),
            &mut cache,
            facts.as_ref(),
        );
        self.hoist_cache = cache;
        if ok.is_err() {
            return Ok(false);
        }
        let (path, vector, size) = (path.clone(), vector.clone(), *size);
        let n = self.decls.len() + 1;
        let (w, pv) = (format!("pw{n}"), format!("pv{n}"));
        self.decls
            .push(format!("loft_win {w} = {{ 0, 0, 0, 0, 0xFFFF }};"));
        self.decls
            .push(format!("loft_ref {pv} = {{ 0xFFFF, 0, 0 }};"));
        let v = self.expr(&vector, ind)?;
        self.line(ind, &format!("{pv} = {};", as_ref(v)?));
        self.flush(ind);
        self.line(ind, &format!("{w} = loft_win_of({pv}, {size});"));
        crate::rewrite_census::fired("R-PushFill", 1);
        self.windows.push((path, w, pv, size));
        Ok(true)
    }

    /// The counted loop `lp` a window is admitted for: scalar pushes (`hoist::push_loop`,
    /// `push_window_ok`), or record mint groups (`hoist::mint_loop`, `mint_window_ok`, under
    /// `LOFT_NO_RECORD_PUSH` / `LOFT_NO_HEAP_RECORD_PUSH` as on native).  A path an enclosing
    /// loop holds a window on is served by that window: a reservation would move its record.
    fn window_loop<'b>(&mut self, lp: &'b Block) -> Option<WindowLoop<'b>> {
        use crate::opt::push_window as hoist;
        let env_on = |k: &str| std::env::var(k).is_ok_and(|v| v != "0");
        let data = self.cx.data;
        let open = |w: &Self, path: &crate::opt::writes::PathKey| {
            w.windows.iter().any(|(p, _, _, _)| p == path)
        };
        if let Some(p) = hoist::push_loop(lp, data) {
            if p.fill.is_some() || open(self, &p.path) {
                return None;
            }
            hoist::push_window_ok(
                lp,
                &p,
                data,
                self.d,
                Some(self.cx.stores),
                &mut self.hoist_cache,
            )
            .ok()?;
            return Some(WindowLoop {
                path: p.path,
                vector: std::borrow::Cow::Borrowed(p.vector),
                hi: p.hi,
                index_var: p.index_var,
                next_var: p.next_var,
                inclusive: p.inclusive,
                pushes: p.pushes,
                size: p.size,
            });
        }
        if env_on("LOFT_NO_RECORD_PUSH") {
            return None;
        }
        let (m, hi) = hoist::mint_loop(
            lp,
            data,
            self.cx.stores,
            self.vars,
            !env_on("LOFT_NO_HEAP_RECORD_PUSH"),
        )
        .ok()?;
        if open(self, &m.path) {
            return None;
        }
        hoist::mint_window_ok(
            lp,
            &m,
            data,
            self.d,
            Some(self.cx.stores),
            &mut self.hoist_cache,
        )
        .ok()?;
        Some(WindowLoop {
            path: m.path,
            vector: std::borrow::Cow::Owned(m.vector),
            hi,
            index_var: m.index_var,
            next_var: m.next_var,
            inclusive: m.inclusive,
            pushes: m.mints_per_pass,
            size: m.size,
        })
    }

    /// `@FR-R-CompleteWrite` — does every mint of element type `tp` in this function write it
    /// completely?  Then the default image a fresh element is filled with writes nothing that
    /// survives, and the copy is left out, as native calls `OpNewRecordNP`.  A function with a
    /// record copy into a field of a fresh element keeps every fill: a null source leaves that
    /// field what the fill wrote (loft#2044).
    fn mint_complete(&mut self, tp: u16) -> bool {
        if std::env::var("LOFT_NO_COMPLETE_WRITE").is_ok_and(|v| v != "0") {
            return false;
        }
        if self.complete_mints.is_none() {
            let c = crate::opt::ret_adopt::complete_writes(self.cx.data, self.cx.stores, self.d);
            self.complete_mints = Some(if c.fresh_fields.is_empty() {
                c.mint_tps
            } else {
                std::collections::HashSet::new()
            });
        }
        self.complete_mints
            .as_ref()
            .is_some_and(|m| m.contains(&tp))
    }

    /// A push, or its pre-allocation, onto a path an enclosing loop holds a window on: the
    /// element's bytes as the ordinary push writes them, through the window.  `None` for any
    /// other operator.
    fn windowed(&mut self, name: &str, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        let data = self.cx.data;
        let by_path = |w: &Self, path: &crate::opt::writes::PathKey| {
            w.windows
                .iter()
                .find(|(p, _, _, _)| p == path)
                .map(|(_, w, pv, size)| (w.clone(), pv.clone(), *size))
        };
        let window_of = |w: &Self, v: &Value| {
            let path = crate::opt::writes::vector_path(data, v)?;
            by_path(w, &path)
        };
        if name == "OpPreAllocVector" {
            return Ok(args
                .first()
                .and_then(|v| window_of(self, v))
                .map(|_| E::new("((void) 0)", K::Void)));
        }
        if matches!(name, "OpNewRecord" | "OpFinishRecord")
            && let Some(t) =
                crate::opt::writes::mint_target(data, self.cx.stores, name, args, self.vars)
            && let Some((w, pv, size)) = by_path(self, &t.path)
        {
            if name == "OpFinishRecord" {
                return Ok(Some(E::new(
                    format!("loft_win_finish(&{w}, {pv}, {size})"),
                    K::Void,
                )));
            }
            let elem = self.cx.stores.content(t.vector_tp);
            let ptp = u16::try_from(Self::literal(&args[1], "type")?)
                .map_err(|_| "a type number out of range")?;
            let site = format!("{}:{w}", data.def(self.d).original_name());
            let e = self.temp(CType::Ref);
            self.line(
                ind,
                &format!(
                    "{e} = (LOFT_PROVED(loft_win_same({w}, {pv}, {size}), {}), loft_win_slot(&{w}, {pv}, {size}));",
                    c_string(&site)
                ),
            );
            if !self.mint_complete(ptp) {
                let (image, n) = self.cx.default_image(elem);
                self.line(ind, &format!("loft_fill({e}, {image}, {n});"));
            }
            return Ok(Some(E::new(e, K::Ref)));
        }
        let Some(fp) = crate::opt::writes::fused_push(data, name, args) else {
            return Ok(None);
        };
        let Some((w, pv, _)) = window_of(self, fp.vector) else {
            return Ok(None);
        };
        let val = self.expr(fp.val, ind)?;
        let (put, bits) = match name {
            "OpPushInt" => ("loft_win_put8", format!("(uint64_t) {}", as_int(val)?)),
            "OpPushFloat" => (
                "loft_win_put8",
                format!("loft_f64_bits({})", as_float(val)?),
            ),
            "OpPushSingle" => (
                "loft_win_put4",
                format!("loft_f32_bits({})", as_single(val)?),
            ),
            "OpPushInt4" => ("loft_win_put4", format!("loft_i4_bits({})", as_int(val)?)),
            "OpPushCharacter" => ("loft_win_put4", as_char(val)?),
            "OpPushBoolean" => ("loft_win_put1", as_bool(val)?),
            "OpPushEnum" => ("loft_win_put1", as_enum(val)?),
            "OpPushByte" => {
                let min = Self::literal(
                    fp.bias.ok_or("a byte push without its minimum")?,
                    "byte minimum",
                )?;
                (
                    "loft_win_put1",
                    format!("loft_byte_raw({min}, {})", as_int(val)?),
                )
            }
            _ => return Ok(None),
        };
        let site = format!("{}:{w}", data.def(self.d).original_name());
        Ok(Some(E::new(
            format!(
                "(LOFT_PROVED(loft_win_same({w}, {pv}, {}), {}), {put}(&{w}, {pv}, {bits}))",
                fp.size,
                c_string(&site)
            ),
            K::Void,
        )))
    }

    /// `(R-Header)`'s proof site for a held access: the vector still is what `held` holds.
    /// Checked only under `LOFT_VERIFY`; `((void) 0)` when the vector is not a plain place.
    fn held_proof(&self, v: &Value, held: &str) -> String {
        match self.place_of(v) {
            Some(place) => {
                let site = format!("{}:{held}", self.cx.data.def(self.d).original_name());
                format!(
                    "LOFT_PROVED(loft_vh_same({held}, {place}), {})",
                    c_string(&site)
                )
            }
            None => "((void) 0)".into(),
        }
    }

    /// A vector path as a C expression with no statement before it: a variable, or fields of one.
    fn place_of(&self, v: &Value) -> Option<String> {
        match v.unspan() {
            Value::Var(var) if self.shapes[*var as usize] == CType::Ref => Some(self.var(*var)),
            Value::Call(d, a) if a.len() == 3 && self.cx.data.def(*d).name() == "OpGetField" => {
                let Value::Int(fld) = a[1].unspan() else {
                    return None;
                };
                Some(format!("loft_field({}, {fld})", self.place_of(&a[0])?))
            }
            _ => None,
        }
    }

    /// The `loft_vh` an enclosing loop holds for vector expression `v`, if any, as the
    /// expression an access reads: the header itself, or under growth the header with its
    /// base derived again (`loft_vh_fresh`).
    fn held_vector(&self, v: &Value) -> Option<String> {
        let path = crate::opt::writes::vector_path(self.cx.data, v)?;
        self.held_path(&path)
    }

    fn held_path(&self, path: &crate::opt::writes::PathKey) -> Option<String> {
        self.headers
            .iter()
            .rev()
            .flat_map(|f| f.iter())
            .find(|(p, _, _)| p == path)
            .map(|(_, h, base)| {
                if *base {
                    h.clone()
                } else {
                    format!("loft_vh_fresh({h})")
                }
            })
    }

    /// An element access of a held vector: `OpGetVector(v, size, i)` itself, or a scalar read
    /// or in-place write of a field of one — through the held base.  `None` for any other.
    fn held_access(&mut self, name: &str, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        let elem = |v: &Value| -> Option<(Value, Value, Value)> {
            match v.unspan() {
                Value::Call(d, a) if a.len() == 3 => {
                    let n = self.cx.data.def(*d).name();
                    (n == "OpGetVector" || n == "OpGetVectorNullable")
                        .then(|| (a[0].clone(), a[1].clone(), a[2].clone()))
                }
                _ => None,
            }
        };
        match (name, args) {
            ("OpGetVector" | "OpGetVectorNullable", [v, size, idx]) => {
                let Some(h) = self.held_vector(v) else {
                    return Ok(None);
                };
                let size = Self::literal(size, "element size")?;
                let i = self.expr(idx, ind)?;
                let proof = self.held_proof(v, &h);
                Ok(Some(E::new(
                    format!("({proof}, loft_vh_get({h}, {size}, {}))", as_int(i)?),
                    K::Ref,
                )))
            }
            ("OpLengthVector", [v]) => Ok(self
                .held_vector(v)
                .map(|h| E::new(format!("{h}.len"), K::Int))),
            ("OpGetInt" | "OpGetFloat", [inner, fld]) => {
                let Some((vec, size, idx)) = elem(inner) else {
                    return Ok(None);
                };
                let Some(held) = self.held_vector(&vec) else {
                    return Ok(None);
                };
                let size = Self::literal(&size, "element size")?;
                let fld = Self::literal(fld, "field")?;
                let index = self.expr(&idx, ind)?;
                let (f, k) = if name == "OpGetInt" {
                    ("loft_vh_get_int", K::Int)
                } else {
                    ("loft_vh_get_float", K::Float)
                };
                let proof = self.held_proof(&vec, &held);
                Ok(Some(E::new(
                    format!("({proof}, {f}({held}, {size}, {}, {fld}))", as_int(index)?),
                    k,
                )))
            }
            ("OpSetInt" | "OpSetFloat", [inner, fld, val]) => {
                let Some((vec, size, idx)) = elem(inner) else {
                    return Ok(None);
                };
                let Some(held) = self.held_vector(&vec) else {
                    return Ok(None);
                };
                let size = Self::literal(&size, "element size")?;
                let fld = Self::literal(fld, "field")?;
                let (index, value) = self.two(name, &[idx, val.clone()], ind)?;
                let (f, value) = if name == "OpSetInt" {
                    ("loft_vh_set_int", as_int(value)?)
                } else {
                    ("loft_vh_set_float", as_float(value)?)
                };
                let proof = self.held_proof(&vec, &held);
                Ok(Some(E::new(
                    format!(
                        "({proof}, {f}({held}, {size}, {}, {fld}, {value}))",
                        as_int(index)?
                    ),
                    K::Void,
                )))
            }
            _ => Ok(None),
        }
    }

    /// The loop label `depth` loops out from the innermost.
    fn label(&self, depth: u16) -> Result<u32, String> {
        let n = self.loops.len();
        if (depth as usize) < n {
            Ok(self.loops[n - 1 - depth as usize])
        } else {
            Err(format!("a break or continue {depth} loops out of {n}"))
        }
    }

    /// Return `v`'s value (or nothing): copy it out, free the function's texts, return.
    fn ret_value(&mut self, v: Option<&Value>, ind: usize) -> Result<(), String> {
        let v = v.filter(|v| !matches!(v.unspan(), Value::Null));
        match (self.ret, v) {
            (CType::Void, Some(v)) => {
                self.stmt(v, ind)?;
                self.epilogue(ind);
                self.line(ind, "return;");
            }
            (CType::Void, None) => {
                self.epilogue(ind);
                self.line(ind, "return;");
            }
            (c, v) => {
                let r = self.temp(c);
                match v {
                    Some(v) => self.assign(v, &r, c, ind)?,
                    None => self.null_into(&r, c, ind),
                }
                self.ret_held(&r, ind);
            }
        }
        Ok(())
    }

    /// Return the value temporary `r` already holds, of the function's result shape.
    fn ret_held(&mut self, r: &str, ind: usize) {
        // The result leaves the function: its text buffers — its own, or the ones a tuple
        // holds — move out before the epilogue frees every text the function holds.
        let mut moved = Vec::new();
        match self.ret {
            CType::Text => moved.push(r.to_string()),
            CType::Tuple(id) => self.tuple_text_leaves(r, id, &mut moved),
            _ => {}
        }
        if moved.is_empty() {
            self.epilogue(ind);
            self.line(ind, &format!("return {r};"));
        } else {
            self.line(ind, &format!("{} {r}_out = {r};", held(self.ret)));
            for m in moved {
                self.line(ind, &format!("{m}.p = 0;"));
            }
            self.epilogue(ind);
            self.line(ind, &format!("return {r}_out;"));
        }
    }

    /// A value the scopes pass left for the bytecode's evaluation stack: `[value, frees…,
    /// return null]`, where `OpReturn` answers the value still on the stack top.  The
    /// positions of the value and of the return, when `ops` ends that way; the native
    /// writer's `detect_ref_tail_capture` is the same reading.
    fn tail_capture(&self, ops: &[Value]) -> Option<(usize, usize)> {
        if self.ret == CType::Void {
            return None;
        }
        let ret = ops
            .iter()
            .rposition(|o| !matches!(o.unspan(), Value::Line(_)))?;
        let Value::Return(x) = ops[ret].unspan() else {
            return None;
        };
        if !matches!(x.unspan(), Value::Null) && !self.is_free(x) {
            return None;
        }
        let at = ops[..ret]
            .iter()
            .rposition(|o| !matches!(o.unspan(), Value::Line(_)) && !self.is_free(o))?;
        (self.shape_of(&ops[at]).ok() == Some(self.ret)).then_some((at, ret))
    }

    /// A scope-exit free, or the store lock the cleanup window may hold.
    fn is_free(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Call(d, _) => {
                let name = self.cx.data.def(*d).name();
                // A drop cascade releases as a free does: a closure's (`OpDropFnRef`) and a
                // record type's (`t_<n><Type>_OpDrop` / `_OpDropAll`), each ahead of its
                // store's free.
                name.starts_with("OpFree")
                    || name == "OpDropFnRef"
                    || (name.starts_with("t_")
                        && (name.ends_with("_OpDrop") || name.ends_with("_OpDropAll")))
                    || name == "n_set_store_lock"
            }
            // The guarded release `if <holds a record> <release> else null`.
            Value::If(_, t, e) => matches!(e.unspan(), Value::Null) && self.is_free(t),
            _ => false,
        }
    }

    /// `ops` for their effect, returning the value a `tail_capture` names.
    fn ops_stmt(&mut self, ops: &[Value], ind: usize) -> Result<(), String> {
        let Some((at, ret)) = self.tail_capture(ops) else {
            return ops.iter().try_for_each(|o| self.stmt(o, ind));
        };
        ops[..at].iter().try_for_each(|o| self.stmt(o, ind))?;
        let r = self.temp(self.ret);
        self.assign(&ops[at], &r, self.ret, ind)?;
        self.flush(ind);
        ops[at + 1..ret]
            .iter()
            .try_for_each(|o| self.stmt(o, ind))?;
        if let Value::Return(x) = ops[ret].unspan() {
            self.stmt(x, ind)?;
        }
        self.ret_held(&r, ind);
        Ok(())
    }

    fn null_into(&mut self, target: &str, c: CType, ind: usize) {
        let s = match c {
            CType::Int => format!("{target} = LOFT_NULL;"),
            CType::Bool => format!("{target} = LOFT_BOOL_NULL;"),
            CType::Text => format!("loft_text_set(&{target}, loft_null_text());"),
            CType::Ref => format!("{target} = loft_ref_null();"),
            CType::Enum => format!("{target} = 255;"),
            CType::Float => format!("{target} = (double) NAN;"),
            CType::Single => format!("{target} = (float) NAN;"),
            CType::Char => format!("{target} = 0;"),
            CType::Fn => format!("{target} = loft_fn_of(LOFT_NULL);"),
            CType::Tuple(id) => {
                for (i, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    self.null_into(&format!("{target}.f{i}"), e, ind);
                }
                return;
            }
            CType::Void => return,
        };
        self.line(ind, &s);
    }

    /// loft#1753 — a statement that calls into a frame first records the line it calls from,
    /// which `stack_trace()` reports as the callee frame's `line` (`cr_call_site`).
    fn note_site(&mut self, v: &Value, ind: usize) {
        if let Value::Line(n) = v.unspan() {
            self.cur_line = *n;
        } else if self.cur_line != 0
            && !matches!(
                v.unspan(),
                Value::Block(_) | Value::Insert(_) | Value::Span(_)
            )
            && crate::opt::frame_calls::calls_a_frame(self.cx.data, v)
        {
            self.line(ind, &format!("loft_frame_site({});", self.cur_line));
        }
    }

    /// Emit `v` for its effect.
    fn stmt(&mut self, v: &Value, ind: usize) -> Result<(), String> {
        let r = self.stmt_kind(v, ind);
        // `LOFT_C_WHERE=1`: a refusal names the innermost statement it came from, on stderr.
        if let Err(why) = &r
            && !self.where_told
            && std::env::var("LOFT_C_WHERE").is_ok_and(|w| w != "0")
            && !matches!(
                v.unspan(),
                Value::Block(_) | Value::Loop(_) | Value::If(..) | Value::Insert(_)
            )
        {
            self.where_told = true;
            let mut at = format!("{v:?}");
            at.truncate(600);
            eprintln!(
                "loft: --emit-c: {}: {why}\n  at: {at}",
                self.cx.data.def(self.d).original_name()
            );
        }
        r
    }

    /// A loop: its held vectors and its push window before it, the window closed after —
    /// written twice under `@FR-R-Alias`'s versioned clause (`distinct_version`): the copy
    /// that assumes the parameters apart from the return buffer runs when a test of their
    /// store numbers says so at its entry, the plain one otherwise.
    fn loop_stmt(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        if let Some((tests, pairs)) = self.distinct_version(b) {
            let test = tests
                .iter()
                .map(|&(p, rb)| format!("{}.store_nr != {}.store_nr", self.var(rb), self.var(p)))
                .collect::<Vec<_>>()
                .join(" && ");
            self.flush(ind);
            self.line(ind, &format!("if ({test}) {{"));
            self.assumed = pairs;
            let r = self.loop_copy(b, ind + 1);
            self.assumed.clear();
            r?;
            self.line(ind, "} else {");
            self.loop_copy(b, ind + 1)?;
            self.line(ind, "}");
            crate::rewrite_census::fired("R-AliasVersion", 1);
            return Ok(());
        }
        self.loop_copy(b, ind)
    }

    fn loop_copy(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        let held_file = self.open_file_hold(b, ind)?;
        let mints = self.hold_vectors(b, ind)?;
        let window = self.open_window(b, ind)?;
        self.hold_pushes(b, mints, ind)?;
        self.labels += 1;
        let l = self.labels;
        self.loops.push(l);
        if let Some(walk) = self.char_walks.get(&b.scope).cloned() {
            self.char_walk_loop(b, &walk, ind)?;
        } else {
            self.line(ind, "for (;;) {");
            self.block_stmt(b, ind + 1)?;
        }
        self.line(ind, &format!("cont_{l}:;"));
        self.line(ind, "}");
        self.line(ind, &format!("brk_{l}:;"));
        self.loops.pop();
        self.headers.pop();
        self.push_heads.pop();
        if window {
            // The loop's one exit: the counted push loop has no break, return or
            // continue of its own (`hoist::push_loop`).
            let (_, w, _, _) = self.windows.pop().expect("the window this loop opened");
            self.line(ind, &format!("loft_win_sync({w});"));
        }
        if held_file {
            // The loop's own end; its other exits gave the cursor back where they leave.
            let (_, c, _) = self.file_holds.pop().expect("the cursor this loop holds");
            self.line(ind, &format!("loft_fcur_release(&{c});"));
        }
        Ok(())
    }

    /// `@FR-R-CharWalk` — a `for c in t` walk: its null test once before the loop where the loop
    /// never writes `t`, and its step's ASCII arm — a byte below 0x80 at `next` is the character,
    /// one byte wide — ahead of the step as the lowering writes it, which every other byte takes.
    /// Opens the loop's `for (;;) {`; the caller closes it.
    fn char_walk_loop(
        &mut self,
        b: &Block,
        walk: &crate::opt::char_walk::CharWalk,
        ind: usize,
    ) -> Result<(), String> {
        if walk.hoist_null
            && let Some(test) = b.operators.get(walk.null_test)
        {
            self.stmt(test, ind)?;
        }
        self.line(ind, "for (;;) {");
        for (i, op) in b.operators.iter().enumerate() {
            if walk.hoist_null && i == walk.null_test {
                continue;
            }
            if i == 0 {
                let src = self.expr(&Value::Var(walk.src), ind + 1)?;
                let src = as_view(src)?;
                let (next, index, c) = (
                    self.var(walk.next),
                    self.var(walk.index),
                    self.var(walk.loop_var),
                );
                self.line(
                    ind + 1,
                    &format!(
                        "if ({next} >= 0 && {next} < (int64_t) ({src}).len && (unsigned) (unsigned char) ({src}).p[{next}] - 1u < 0x7Fu) {{"
                    ),
                );
                self.line(ind + 2, &format!("{index} = {next};"));
                self.line(
                    ind + 2,
                    &format!("{c} = (uint32_t) (unsigned char) ({src}).p[{next}];"),
                );
                self.line(ind + 2, &format!("{next} = {next} + 1;"));
                self.line(ind + 1, "} else {");
                self.stmt(op, ind + 2)?;
                self.line(ind + 1, "}");
                continue;
            }
            self.stmt(op, ind + 1)?;
        }
        Ok(())
    }

    fn stmt_kind(&mut self, v: &Value, ind: usize) -> Result<(), String> {
        self.note_site(v, ind);
        match v {
            // A value block's result where nothing takes it: a leaf has no effect.
            Value::Null
            | Value::Line(_)
            | Value::Var(_)
            | Value::Int(_)
            | Value::Long(_)
            | Value::Boolean(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Enum(..)
            | Value::Text(_) => Ok(()),
            Value::Span(b) => self.stmt(&b.1, ind),
            Value::Block(b) => self.block_stmt(b, ind),
            Value::Insert(ops) => ops.iter().try_for_each(|o| self.stmt(o, ind)),
            Value::Set(var, x) => self.set_stmt(*var, x, ind),
            Value::TuplePut(var, i, x) => {
                let CType::Tuple(id) = self.shapes[*var as usize] else {
                    return Err("a tuple write into a non-tuple".into());
                };
                let shapes = self.cx.tuple_shapes(id);
                let c = *shapes
                    .get(*i as usize)
                    .ok_or("a tuple index out of range")?;
                let target = format!("{}.f{i}", self.var(*var));
                self.assign(x, &target, c, ind)?;
                self.flush(ind);
                Ok(())
            }
            Value::If(c, t, e) => {
                let null_test = self.null_test(c);
                let c = self.expr(c, ind)?;
                let c = as_cond(c)?;
                self.line(ind, &format!("if ({}) {{", bare(&c)));
                self.stmt(t, ind + 1)?;
                if !matches!(e.unspan(), Value::Null) {
                    self.line(ind, "} else {");
                    if null_test {
                        self.line(ind + 1, "LOFT_COLD_PATH();");
                    }
                    self.stmt(e, ind + 1)?;
                }
                self.line(ind, "}");
                self.flush(ind);
                Ok(())
            }
            Value::Loop(b) => self.loop_stmt(b, ind),
            Value::Break(n) => {
                let l = self.label(*n)?;
                self.flush(ind);
                // The holds of the loops it leaves; the target's own goes back after its end.
                self.release_holds(self.loops.len() - *n as usize, ind);
                self.line(ind, &format!("goto brk_{l};"));
                Ok(())
            }
            Value::Continue(n) => {
                let l = self.label(*n)?;
                self.flush(ind);
                // The holds of the loops it leaves; the target's own stays held.
                self.release_holds(self.loops.len() - *n as usize, ind);
                self.line(ind, &format!("goto cont_{l};"));
                Ok(())
            }
            // A generator's `return` ends it: what it returns is done for its effect.
            Value::Return(x) if self.gen_elem.is_some() => {
                self.stmt(x, ind)?;
                self.line(ind, "goto loft_gen_end;");
                Ok(())
            }
            Value::Return(x) => self.ret_value(Some(x), ind),
            Value::Yield(x) => self.gen_yield(x, ind),
            // `parallel { … }`: the arms run one after another — each arm's effect is the one
            // its thread has (State's `OpParallelBegin`).
            Value::Parallel(arms) => arms.iter().try_for_each(|a| self.stmt(a, ind)),
            // A dropped branch or block runs as a statement: each arm for its effect, a value
            // arm's value discarded (`@FR-F-Drop`) — where an arm assigns, it has no value to read.
            Value::Drop(x)
                if matches!(
                    x.unspan(),
                    Value::If(..) | Value::Block(_) | Value::Insert(_)
                ) =>
            {
                self.stmt(x, ind)
            }
            Value::Drop(_) | Value::Call(..) | Value::CallRef(..) => {
                let x = if let Value::Drop(x) = v { &**x } else { v };
                let e = self.expr(x, ind)?;
                if e.kind == K::Void {
                    self.line(ind, &format!("{};", e.code));
                } else {
                    self.line(ind, &format!("(void) {};", e.code));
                }
                self.flush(ind);
                Ok(())
            }
            other => Err(unsupported(other)),
        }
    }

    /// `v = src` binding one record variable to another of a type that copies
    /// (`@FR-B-Copy`, @C86): the source and the store type — the bytecode writer's
    /// `gen_set_first_ref_var_copy` test, `record_copy_source` and `copies_as`.
    fn whole_value_bind(&self, v: u16, x: &Value) -> Option<(u16, u16)> {
        let Value::Var(src) = x.unspan() else {
            return None;
        };
        let data = self.cx.data;
        let d = self.vars.tp(v).base().heap_def_nr()?;
        let sd = self.vars.record_copy_source(data, v, *src)?;
        data.copies_as(d, sd)
            .then(|| (*src, data.def(d).known_type()))
    }

    /// `v = f(…)` binding a record variable to a loft function's result that the bind must
    /// copy rather than adopt — the bytecode writer's first-bind test: a callee that hands
    /// over a fresh store, an elided view and a minted store are adopted; anything else is
    /// copied, its source freed when `call_return_frees_source` says the copy owns it.  The
    /// record's store type and that flag; `None` to adopt.
    /// A rebind whose result is fresh on one arm and borrowed on the other (`Own::Join`) that
    /// `join_stmt` does not bind — the call reads the variable, or the variable does not own
    /// what it displaces — is this copy, as the bytecode writer's #360 path copies it.
    fn call_copy(&self, v: u16, x: &Value) -> Option<(u16, bool)> {
        use crate::data::Type;
        use crate::use_analysis as ua;
        let data = self.cx.data;
        let shape = ua::first_bind_shape(self.vars.tp(v)).clone();
        // A `τ?` that `first_bind_shape` leaves wrapped (`LOFT_NO_NULLABLE_RETBUF`) is not
        // bound here, as in the bytecode writer.
        if matches!(shape, Type::Optional(_)) {
            return None;
        }
        let (Type::Reference(d, _) | Type::Enum(d, true, _)) = shape.base().clone() else {
            return None;
        };
        let f = ua::callee_of(data, self.d, x)?;
        if !data.def(f).is_loft_defined() {
            return None;
        }
        if data.def(f).return_adopts_fresh_store()
            || self.vars.is_view_elided(v)
            || ua::adopts_minted_at_bind(data, self.vars, v, x)
        {
            return None;
        }
        Some((
            data.def(d).known_type(),
            ua::call_return_frees_source(data, self.d, x),
        ))
    }

    /// `v = f(…)` where the callee's result is fresh on one arm and a view of `base` on the
    /// other (`Own::Join`): the record type and `base`, the witness `OpBindOrCopy` tests.
    fn join_bind(&self, v: u16, x: &Value) -> Option<(u16, u16)> {
        use crate::data::Type;
        use crate::use_analysis as ua;
        let data = self.cx.data;
        let shape = ua::first_bind_shape(self.vars.tp(v)).clone();
        // A `τ?` that `first_bind_shape` leaves wrapped (`LOFT_NO_NULLABLE_RETBUF`) is not
        // bound here, as in the bytecode writer.
        if matches!(shape, Type::Optional(_)) {
            return None;
        }
        let (Type::Reference(d, _) | Type::Enum(d, true, _)) = shape.base().clone() else {
            return None;
        };
        let f = ua::callee_of(data, self.d, x)?;
        if !data.def(f).is_loft_defined()
            || !crate::keys::join_own_enabled()
            || self.vars.is_view_elided(v)
        {
            return None;
        }
        match ua::ownership_of(data, self.d, x) {
            ua::Own::Join { base } if base != u16::MAX => Some((data.def(d).known_type(), base)),
            _ => None,
        }
    }

    /// `v = c[i]` binding an owning record variable to an element of a container: the
    /// element is copied out (the bytecode writer's `gen_set_first_ref_elem_copy` test — no
    /// dependency, no owner witness, a projection of a container).  The store type.
    fn element_copy(&self, v: u16, x: &Value) -> Option<u16> {
        let tp = self.vars.tp(v);
        let d = tp.base().heap_def_nr()?;
        (tp.depend().is_empty()
            && self.vars.owner_witness(v).is_none()
            && crate::use_analysis::projection_container_var(self.cx.data, x).is_some())
        .then(|| self.cx.data.def(d).known_type())
    }

    /// Register record type `tp` for the deep copy, or refuse a kind it does not walk yet.
    fn copyable(&self, tp: u16) -> Result<u16, String> {
        self.cx.copy_type(tp)?;
        Ok(tp)
    }

    /// `v = f(…)` whose result is fresh on one arm and borrowed on the other (`Own::Join`):
    /// a first bind, or a rebind of a variable that owns what it displaces.  False when the
    /// bind is not one.
    fn join_stmt(
        &mut self,
        var: u16,
        x: &Value,
        first: bool,
        name: &str,
        ind: usize,
    ) -> Result<bool, String> {
        let join = self.join_bind(var, x);
        // A rebind releases the store it displaces when the variable owns it, as the bytecode
        // writer's owned reassignment does — never the base's, nor a store the call was handed.
        let rebind_owned = !first
            && join.is_some()
            && self.vars.owns_displaced_store(var, x, self.cx.data)
            && !x.reads_var(var);
        if let Some((tp, base)) = join.filter(|_| first || rebind_owned) {
            // OpBindOrCopy: a result in the base's store is a view, copied into a fresh store;
            // any other is the callee's fresh store, adopted.
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            let w = self.var(base);
            let old = self.temp(CType::Ref);
            self.line(ind, &format!("{old} = {name};"));
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.line(
                ind,
                &format!("if ({r}.store_nr != 0xFFFF && {r}.store_nr == {w}.store_nr) {{"),
            );
            self.line(ind + 1, &format!("{name} = loft_ref_null();"));
            self.line(
                ind + 1,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
            );
            self.line(ind, &format!("}} else {{ {name} = {r}; }}"));
            if rebind_owned {
                let mut keep = vec![
                    format!("{old}.store_nr != {name}.store_nr"),
                    format!("{old}.store_nr != {w}.store_nr"),
                ];
                for a in crate::use_analysis::protectable_ref_args(self.cx.data, self.d, x).0 {
                    keep.push(format!("{old}.store_nr != {}.store_nr", self.var(a)));
                }
                self.line(
                    ind,
                    &format!("if ({}) loft_free_ref({old});", keep.join(" && ")),
                );
            }
            self.flush(ind);
            return Ok(true);
        }
        Ok(false)
    }

    /// A rebind's copy lands in a fresh store unless `var` owns the store it displaces
    /// (`Function::owns_displaced_store`, @FR-O-Proxy): a witnessed or borrowing local may hold
    /// a view of another record at that moment, and refilling its store in place wrote the copy
    /// into the viewed record.  An owned store is refilled, which is its free and a fresh mint.
    fn fresh_unless_owned(&mut self, var: u16, x: &Value, first: bool, name: &str, ind: usize) {
        // A witnessed local (`@FR-O-Witness`) never owns the store its slot names — its witness
        // does — and its slot may hold a VIEW into another store when its declaration runs again
        // in a loop; the bytecode's `OpInitRef` resets it there, so the copy mints a fresh store
        // instead of clearing the viewed one (the hash the view was into, measured).
        let witnessed = self.vars.owner_witness(var).is_some();
        if (first && witnessed) || (!first && !self.vars.owns_displaced_store(var, x, self.cx.data))
        {
            self.line(ind, &format!("{name} = loft_ref_null();"));
        }
    }

    /// `p = x` for a `&` parameter of a heap kind — a record, a vector, a struct-enum, a keyed
    /// collection: the caller's variable takes the new value (`OpSetStackRef`), and when the
    /// value is OWNED the one it held is released unless the new one lives in its store
    /// (`OpFreeRefIfDistinct`, @FR-B-Ref-Repoint) — the interpreter's `set_var` on a link.
    fn link_rebind(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        use crate::data::Type;
        let Type::RefVar(inner) = self.vars.tp(var).base() else {
            return Err("a link that is not a parameter".into());
        };
        if !matches!(
            inner.base(),
            Type::Reference(..)
                | Type::Vector(..)
                | Type::Enum(_, true, _)
                | Type::Hash(..)
                | Type::Sorted(..)
                | Type::Index(..)
                | Type::Radix(..)
                | Type::Trie(..)
        ) {
            return Err(format!("a rebind of a & {} parameter", type_word(inner)));
        }
        let owned = matches!(
            crate::use_analysis::ownership_of(self.cx.data, self.d, x),
            crate::use_analysis::Own::Owned
        );
        let e = self.expr(x, ind)?;
        let r = self.temp(CType::Ref);
        let name = self.var(var);
        self.line(ind, &format!("{r} = {};", as_ref(e)?));
        if owned {
            let old = self.temp(CType::Ref);
            self.line(ind, &format!("{old} = {name};"));
            self.line(ind, &format!("{name} = {r};"));
            self.line(ind, &format!("loft_free_displaced({old}, {name});"));
        } else {
            self.line(ind, &format!("{name} = {r};"));
        }
        self.flush(ind);
        Ok(())
    }

    /// `var = x`: a copy where loft copies a record, else the plain assignment.
    /// `@FR-R-TextBorrow` — the bind of a borrowed walk variable: the view the element read
    /// answers.  A view that would point into a temporary of this statement (freed when the
    /// statement ends) is refused by name rather than left dangling.
    fn borrowed_bind(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        // A block (the walk's `iter next` step): its statements run, its value is bound.
        if let Value::Block(b) = x.unspan()
            && let Some(at) = b
                .operators
                .iter()
                .rposition(|o| !matches!(o.unspan(), Value::Line(_)))
        {
            for op in &b.operators[..at] {
                self.stmt(op, ind)?;
            }
            self.flush(ind);
            return self.borrowed_bind(var, &b.operators[at], ind);
        }
        // The element's text field read as the view it is: the analysis proved the loop writes
        // no store, so the bytes stay where they are for as long as the variable is read.
        if let Value::Call(d, args) = x.unspan()
            && self.cx.data.def(*d).name() == "OpGetText"
            && let [r, fld] = args.as_slice()
            && self.lazy_vector(r, "OpGetVectorNullable").is_none()
        {
            let fld = Self::literal(fld, "field")?;
            let before = self.pending.len();
            let r = self.expr(r, ind)?;
            if self.pending.len() != before {
                return Err("a borrowed text would view a temporary".into());
            }
            let name = self.var(var);
            self.line(
                ind,
                &format!("{name} = loft_get_text({}, {fld});", as_ref(r)?),
            );
            return Ok(());
        }
        let before = self.pending.len();
        let e = self.expr(x, ind)?;
        if self.pending.len() != before {
            return Err("a borrowed text would view a temporary".into());
        }
        let view = as_view(e)?;
        let name = self.var(var);
        self.line(ind, &format!("{name} = {view};"));
        Ok(())
    }

    /// `@FR-R-LazySplit` — the lazy vector `vec` when `v` is `reader(Var(vec), …)`.
    fn lazy_vector(&self, v: &Value, reader: &str) -> Option<u16> {
        let Value::Call(d, args) = v.unspan() else {
            return None;
        };
        if self.cx.data.def(*d).name() != reader {
            return None;
        }
        let Value::Var(vec) = args.first()?.unspan() else {
            return None;
        };
        self.lazy.contains_key(vec).then_some(*vec)
    }

    /// `@FR-R-LazySplit` — the three operators a lazy loop's vector is mentioned by: its element
    /// read is the iterator's next piece, its length test (`len <= i`, the loop's exit) is the
    /// iterator having run out, and the mint of the split's own buffer, which only this call
    /// served, is nothing.  `None` for every other operator.
    fn lazy_split_op(&self, name: &str, args: &[Value]) -> Option<E> {
        match (name, args) {
            ("OpGetText", [elem, _]) => {
                let vec = self.lazy_vector(elem, "OpGetVectorNullable")?;
                Some(E::new(
                    format!("loft_ls_next(&ls{vec}, &lsd{vec})"),
                    K::View,
                ))
            }
            ("OpLeInt", [len, _]) => {
                let vec = self.lazy_vector(len, "OpLengthVector")?;
                Some(E::new(format!("(lsd{vec} != 0)"), K::Cond))
            }
            ("OpDatabase", [Value::Var(b), _])
                if self.lazy.values().any(|ls| ls.dead_buf == Some(*b)) =>
            {
                Some(E::new("((void) 0)", K::Void))
            }
            _ => None,
        }
    }

    /// `@FR-R-LazySplit` — the bind of a lazy loop's vector: an iterator over the source text.
    /// A text parameter nothing in the function writes is walked in place; any other source is
    /// copied first, so no write in the loop body can reach what is being walked.
    fn lazy_split_bind(
        &mut self,
        var: u16,
        ls: &crate::opt::lazy_split::LazySplit,
        x: &Value,
        ind: usize,
    ) -> Result<(), String> {
        // The vector's own null initialisation: it is never built, so there is nothing to set.
        if matches!(x.unspan(), Value::Null) {
            return Ok(());
        }
        let Value::Call(_, args) = x.unspan() else {
            return Err("a lazy split bound from something other than its call".into());
        };
        let src = args.first().ok_or("a split call without its text")?;
        if !self
            .decls
            .iter()
            .any(|d| d.starts_with(&format!("loft_ls ls{var} ")))
        {
            self.decls.push(format!(
                "loft_ls ls{var} = {{ {{ 0, 0 }}, 0, {{ 0 }}, 0 }};"
            ));
            self.decls.push(format!("uint8_t lsd{var} = 0;"));
        }
        let sep = u32::from(ls.separator);
        let in_place = matches!(src.unspan(), Value::Var(p)
            if self.vars.is_argument(*p)
                && !self.links[*p as usize]
                && self.shapes[*p as usize] == CType::Text
                && !crate::opt::char_walk::text_written(self.code, *p, self.cx.data));
        let e = self.expr(src, ind)?;
        let view = as_view(e)?;
        if in_place {
            self.line(ind, &format!("ls{var} = loft_ls_of({view}, {sep}u);"));
        } else {
            let own = self.temp(CType::Text);
            self.line(ind, &format!("loft_text_set(&{own}, {view});"));
            self.line(
                ind,
                &format!("ls{var} = loft_ls_of(loft_view(&{own}), {sep}u);"),
            );
        }
        self.line(ind, &format!("lsd{var} = 0;"));
        self.flush(ind);
        Ok(())
    }

    /// The binds a rewrite writes itself: a borrowed text's view (`@FR-R-TextBorrow`) and a
    /// lazy split's iterator (`@FR-R-LazySplit`).  `None` for every other bind.
    fn rewritten_bind(&mut self, var: u16, x: &Value, ind: usize) -> Option<Result<(), String>> {
        if self.shapes[var as usize] == CType::Text && self.borrowed[var as usize] {
            return Some(self.borrowed_bind(var, x, ind));
        }
        let ls = self.lazy.get(&var).cloned()?;
        Some(self.lazy_split_bind(var, &ls, x, ind))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per kind of bind; the rewritten binds are already helpers"
    )]
    fn set_stmt(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        // An `&` local is bound to the variable `OpCreateStack` names: it points there, and
        // every later read and write of it goes through.
        if self.store_links[var as usize] {
            return self.store_link_set(var, x, ind);
        }
        if self.link_repoint(var, x, ind)? {
            return Ok(());
        }
        if self.links[var as usize] && self.shapes[var as usize] == CType::Ref {
            return self.link_rebind(var, x, ind);
        }
        if let Some(a) = self.adopt
            && var == a.v
            && matches!(x.unspan(), Value::Call(d, _) if self.cx.data.def(*d).name() == "OpGetField")
        {
            self.bound[var as usize] = true;
            self.adopted_init(a, ind)?;
            return Ok(());
        }
        let first = !std::mem::replace(&mut self.bound[var as usize], true);
        // An owned vector variable bound to null gets a store of its own whose record holds
        // the (empty) vector — the bytecode writer's `gen_set_first_vector_null`: the variable
        // then names that field.  A skip-free, inline or lazy-buffer one, and a view of another
        // variable (bound to its target later), keep the sentinel.  A lazy buffer's LATER
        // `Set(v, Null)` is its mint (`@FR-O-LazyBuffer`): the scopes pass guards it with
        // `OpRefIsNull(v)`.
        let lazy = self.vars.is_lazy_buffer(var);
        if matches!(x.unspan(), Value::Null)
            && crate::parser::vectors::owns_keyed_store(self.vars.tp(var))
        {
            self.keyed_null(var, first, ind);
            return Ok(());
        }
        if matches!(x.unspan(), Value::Null)
            && let crate::data::Type::Vector(_, dep) = self.vars.tp(var).base()
            && dep.is_empty()
            && (if lazy {
                !first
            } else {
                !(self.vars.is_skip_free(var) || self.vars.is_inline_ref(var))
            })
        {
            self.owned_vector_null(var, ind);
            return Ok(());
        }
        let c = self.shapes[var as usize];
        if let Some(done) = self.rewritten_bind(var, x, ind) {
            return done;
        }
        if c == CType::Text && !self.text_held(var) {
            return Err("assigns a text parameter".into());
        }
        let name = self.var(var);
        if self.join_stmt(var, x, first, &name, ind)?
            || self.opaque_callref_stmt(var, x, first, &name, ind)?
        {
            return Ok(());
        }
        if let Some((tp, frees)) = self.call_copy(var, x) {
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.fresh_unless_owned(var, x, first, &name, ind);
            self.line(
                ind,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
            );
            if frees {
                // The stores of the call's reference arguments are protected, as the bytecode
                // writer protects them (`protectable_ref_args`): a result that is a view into
                // one of them is not the callee's to give away.
                let mut keep = vec![format!("{r}.store_nr != {name}.store_nr")];
                for a in crate::use_analysis::protectable_ref_args(self.cx.data, self.d, x).0 {
                    keep.push(format!("{r}.store_nr != {}.store_nr", self.var(a)));
                }
                self.line(
                    ind,
                    &format!("if ({}) loft_free_ref({r});", keep.join(" && ")),
                );
            }
            self.flush(ind);
            return Ok(());
        }
        if let Some(tp) = self.element_copy(var, x) {
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.fresh_unless_owned(var, x, first, &name, ind);
            self.line(
                ind,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
            );
            self.flush(ind);
            return Ok(());
        }
        if let Some((src, tp)) = self.whole_value_bind(var, x) {
            let bytes = self.copyable(tp)?;
            let src = self.var(src);
            self.fresh_unless_owned(var, x, first, &name, ind);
            self.line(
                ind,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {src});"),
            );
            return Ok(());
        }
        if !first && c == CType::Ref && self.displacing_rebind(var, x, &name, ind)? {
            return Ok(());
        }
        self.assign(x, &name, c, ind)?;
        self.flush(ind);
        Ok(())
    }

    /// The variable `OpCreateStack(v)` names, whose shape a link to it shares; `None` for any
    /// other value, and for a target that is itself a parameter passed by value.
    /// `var = x` for a `&` variable that `x` re-points (`@FR-B-Ref-Repoint`): it points at a
    /// variable, a tuple member, what another link names, or a store place from here on.
    /// Answers whether it wrote the re-point; a write through the link is no re-point.
    fn link_repoint(&mut self, var: u16, x: &Value, ind: usize) -> Result<bool, String> {
        let repoint = self.links[var as usize]
            && (!self.params[var as usize]
                || crate::scopes::link_set_repoints(self.cx.data, self.vars, var, x));
        if !repoint {
            return Ok(false);
        }
        let to = match self.stack_target(x) {
            Some(to) => Some(to),
            None if self.shapes[var as usize] != CType::Ref => self.direct_place(var, x, ind)?,
            None => None,
        };
        if let Some(to) = to {
            self.bound[var as usize] = true;
            let name = var_name(self.vars.name(var), var);
            self.line(ind, &format!("{name} = {to};"));
            return Ok(true);
        }
        if !self.bound[var as usize] {
            return Err("an & local bound to something other than a variable".into());
        }
        Ok(false)
    }

    /// A link of `var`'s kind re-pointed to a store place — a field or an element — whose bytes
    /// are the C value itself (an 8-byte integer, a float, a single, a character, a boolean, a
    /// plain enum): the address of the place, as `--native` takes it.  An absent place (a null
    /// record) is a temporary holding the null, which a write through the link leaves unseen.
    fn direct_place(&mut self, var: u16, x: &Value, ind: usize) -> Result<Option<String>, String> {
        use crate::data::Type;
        let Value::Call(d, _) = x.unspan() else {
            return Ok(None);
        };
        if !matches!(self.cx.data.def(*d).name(), "OpGetVector" | "OpGetField") {
            return Ok(None);
        }
        let Type::RefVar(tp) = self.vars.tp(var).base() else {
            return Ok(None);
        };
        let direct = match tp.base() {
            Type::Integer(_) => crate::data::NarrowSlot::of_type(tp).is_none(),
            Type::Float | Type::Single | Type::Character | Type::Boolean => true,
            Type::Enum(_, false, _) => true,
            _ => false,
        };
        if !direct {
            return Ok(None);
        }
        let c = self.shapes[var as usize];
        let absent = self.temp(c);
        self.null_into(&absent, c, ind);
        let e = self.expr(x, ind)?;
        Ok(Some(format!(
            "({} *) loft_place({}, &{absent})",
            held(c),
            as_ref(e)?
        )))
    }

    fn stack_target(&self, x: &Value) -> Option<String> {
        let Value::Call(d, args) = x.unspan() else {
            return None;
        };
        match (self.cx.data.def(*d).name(), args.as_slice()) {
            ("OpCreateStack", [Value::Var(t)])
                if !(self.params[*t as usize]
                    && !self.links[*t as usize]
                    && self.shapes[*t as usize] == CType::Text) =>
            {
                Some(format!("&{}", self.var(*t)))
            }
            // A member of a tuple local is a place a link names (`@FR-B-Ref-Lvalue`).
            // A text member of a tuple parameter is the caller's: no link writes it.
            ("OpCreateStack", [Value::TupleGet(t, i)])
                if !self.params[*t as usize]
                    || self.links[*t as usize]
                    || !matches!(self.shapes[*t as usize], CType::Tuple(id)
                        if self.cx.tuple_shapes(id).get(*i as usize) == Some(&CType::Text)) =>
            {
                Some(format!("&{}.f{i}", self.var(*t)))
            }
            // A link to a link names what that link names.
            ("OpVarRef", [Value::Var(t)])
                if self.links[*t as usize] && !self.store_links[*t as usize] =>
            {
                Some(var_name(self.vars.name(*t), *t))
            }
            _ => None,
        }
    }

    /// An owned vector variable bound to null: a fresh store minted as `main_vector<elm>`, the
    /// type the bytecode writer's `OpDatabase` names (`gen_owned_vector_store`), so a later
    /// wrapper mint of it clears it in place.
    fn owned_vector_null(&mut self, var: u16, ind: usize) {
        let crate::data::Type::Vector(elm, _) = self.vars.tp(var).base() else {
            return;
        };
        let data = self.cx.data;
        let known = data.owned_vector_type(elm, data.def(self.d).source());
        let size = self.cx.stores.enum_parent_size(known);
        self.cx.describe_type(known);
        let name = self.var(var);
        self.line(ind, &format!("{name} = loft_ref_null();"));
        self.line(
            ind,
            &format!("loft_database_typed(&{name}, {size}, {known});"),
        );
    }

    /// A keyed local bound to null gets a store too, unless it borrows one
    /// (`gen_keyed_null`); a later bind to null empties that store in place.
    fn keyed_null(&mut self, var: u16, first: bool, ind: usize) {
        let name = self.var(var);
        if first {
            self.line(ind, &format!("{name} = loft_ref_null();"));
        }
        if !(self.vars.is_inline_ref(var) || self.vars.is_overwritten_view(var)) {
            // The store is minted AS the collection type, as `gen_keyed_null`'s `OpDatabase`
            // mints it: its root is that type's record and carries its tag.
            match self.cx.keyed_local_type(self.vars.tp(var)) {
                Some(tp) => {
                    let size = self.cx.stores.enum_parent_size(tp);
                    self.cx.describe_type(tp);
                    self.line(ind, &format!("loft_database_typed(&{name}, {size}, {tp});"));
                }
                None => self.line(ind, &format!("loft_database(&{name}, 8);")),
            }
        }
    }

    /// `v = f(…)` through a function value whose target nothing here resolves
    /// (`use_analysis::opaque_callref_bind`, `@FR-O-Unknown`): the record handed back is adopted
    /// when the call minted its store, and copied into one of `v`'s own when it predates the
    /// call — an argument or a capture, owned further up (`State::bind_fn_ref_result`, native's
    /// `cr_fnref_adopt`).  False when the bind is not one.
    fn opaque_callref_stmt(
        &mut self,
        var: u16,
        x: &Value,
        first: bool,
        name: &str,
        ind: usize,
    ) -> Result<bool, String> {
        let data = self.cx.data;
        let Value::CallRef(fv, _) = x.unspan() else {
            return Ok(false);
        };
        let Some(rec) =
            crate::use_analysis::opaque_callref_bind(data, self.d, self.vars.tp(var), x.unspan())
        else {
            return Ok(false);
        };
        let tp = self.copyable(data.def(rec).known_type())?;
        let size = self.cx.stores.enum_parent_size(tp);
        let witnessed = self.vars.owner_witness(var).is_some();
        let (dst, src, since) = (
            self.temp(CType::Ref),
            self.temp(CType::Ref),
            self.temp(CType::Int),
        );
        let clos = format!("{}.clos", self.var(*fv));
        self.line(ind, &format!("{dst} = {name};"));
        self.line(ind, &format!("{since} = (int64_t) loft_stores_allocated;"));
        let e = self.expr(x, ind)?;
        self.line(ind, &format!("{src} = {};", as_ref(e)?));
        self.line(
            ind,
            &format!("if (loft_fnref_minted({src}, (uint64_t) {since}, {clos})) {{"),
        );
        // A parameter's displaced store is the caller's (a promoted return buffer the caller
        // pools, `@FR-R-Reuse`): the adopt only rebinds it, as `State::bind_fn_ref_result` does.
        if !witnessed && !self.params[var as usize] {
            self.line(
                ind + 1,
                &format!(
                    "if ({dst}.store_nr != 0xFFFF && {dst}.store_nr != {src}.store_nr) loft_free_ref({dst});"
                ),
            );
        }
        self.line(ind + 1, &format!("{name} = {src};"));
        self.line(ind, "} else {");
        if witnessed && !first {
            self.line(ind + 1, &format!("{name} = loft_ref_null();"));
        }
        self.line(
            ind + 1,
            &format!("loft_database_typed(&{name}, {size}, {tp});"),
        );
        self.line(
            ind + 1,
            &format!("loft_copy_record(loft_types, {tp}, {src}, {name}, 4);"),
        );
        self.line(ind, "}");
        self.flush(ind);
        Ok(true)
    }

    /// A rebind of an owned reference from an expression that produces a store releases the
    /// store it displaces, unless the new value or the call's buffer lives there (native's
    /// `owned_ref_reassign`; the interpreter's post-Set `OpFreeRefIfDistinct`).  False when
    /// the rebind displaces nothing it owns.
    fn displacing_rebind(
        &mut self,
        var: u16,
        x: &Value,
        name: &str,
        ind: usize,
    ) -> Result<bool, String> {
        if self.vars.is_argument(var)
            || !matches!(
                x.unspan(),
                Value::Call(..)
                    | Value::CallRef(..)
                    | Value::Insert(_)
                    | Value::Block(_)
                    | Value::If(..)
            )
            || !self.vars.owns_displaced_store(var, x, self.cx.data)
        {
            return Ok(false);
        }
        let old = self.temp(CType::Ref);
        self.line(ind, &format!("{old} = {name};"));
        self.assign(x, name, CType::Ref, ind)?;
        let mut keep = format!("{old}.store_nr != {name}.store_nr");
        if let Some(buf) = self.vars.displaced_buffer_witness(var, x, self.cx.data) {
            let _ = write!(keep, " && {old}.store_nr != {}.store_nr", self.var(buf));
        }
        self.line(ind, &format!("if ({keep}) loft_free_ref({old});"));
        self.flush(ind);
        Ok(true)
    }

    fn block_stmt(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        let delivery = self.delivery_block(b);
        self.in_delivery += u32::from(delivery);
        let r = self.ops_stmt(&b.operators, ind);
        self.in_delivery -= u32::from(delivery);
        r
    }

    /// `@FR-R-RetAdopt` — is `b` the exit delivery of an adopted result local?  Inside it the
    /// buffer's clear would wipe the result and its append would copy the buffer into itself,
    /// so both emit nothing; the entry clear (outside any such block) stays, as the buffer's
    /// reuse contract.
    fn delivery_block(&self, b: &Block) -> bool {
        self.adopt.is_some() && b.name == "one_buffer_vec_copy"
    }

    /// `@FR-R-RetAdopt` — the adopted local's bind: it IS the return buffer.  A caller that
    /// offered none gets one minted exactly as the witness would have been, so ownership
    /// travels the same route; an offered one is emptied, as the delivery's clear did.
    fn adopted_init(
        &mut self,
        a: crate::opt::ret_adopt::RetAdopt,
        ind: usize,
    ) -> Result<(), String> {
        let mint =
            self.store_operator("OpDatabase", &[Value::Var(a.buf), Value::Int(a.db_tp)], ind)?;
        self.cx.name_types();
        let buf = self.var(a.buf);
        self.line(
            ind,
            &format!(
                "if ({buf}.store_nr == 0xFFFF || {buf}.rec == 0) {{ {}; }} else {{ loft_vec_clear_release(loft_types, loft_ntypes, {buf}); }}",
                mint.code
            ),
        );
        let v = self.var(a.v);
        self.line(ind, &format!("{v} = {buf};"));
        Ok(())
    }

    /// The adopted return buffer's variable when `v` names it.
    fn adopted_buffer(&self, v: &Value) -> bool {
        matches!((self.adopt, v.unspan()), (Some(a), Value::Var(b)) if a.buf == *b)
    }

    /// Emit the statements `v` needs and put its value in `target`, of shape `c`.
    fn assign(&mut self, v: &Value, target: &str, c: CType, ind: usize) -> Result<(), String> {
        match v.unspan() {
            // A tuple that owns texts is copied element by element: a struct copy would share them.
            Value::Var(src) if matches!(c, CType::Tuple(id) if self.cx.tuple_has_text(id)) => {
                let CType::Tuple(id) = c else { unreachable!() };
                for (i, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    let i16 = u16::try_from(i).map_err(|_| "a tuple too wide")?;
                    self.assign(
                        &Value::TupleGet(*src, i16),
                        &format!("{target}.f{i}"),
                        e,
                        ind,
                    )?;
                }
                Ok(())
            }
            // The same for a tuple read out of a tuple local (`pr = t.1`): its texts are the
            // ones the epilogue frees, so the local takes copies — a struct copy shared the
            // source's, and the free released text the caller still held.
            Value::TupleGet(src, i) if matches!(c, CType::Tuple(id) if self.cx.tuple_has_text(id)) =>
            {
                let CType::Tuple(id) = c else { unreachable!() };
                let from = format!("{}.f{i}", self.var(*src));
                self.copy_member(&from, target, CType::Tuple(id), ind);
                Ok(())
            }
            // A function value read out of a store: its definition number and its closure, the
            // two values the bytecode pushes as the pair (`fn_ref_field_read`).
            Value::Block(b) if c == CType::Fn && b.name == "fn_ref_field_read" => {
                let ops: Vec<&Value> = b
                    .operators
                    .iter()
                    .filter(|o| !matches!(o.unspan(), Value::Line(_)))
                    .collect();
                // The definition number is pushed first; statements between the two values
                // (a displaced record's release) run before the closure is read.
                let (Some(d), Some(clos)) = (ops.first(), ops.last()) else {
                    return Err("a function value read with an unexpected shape".into());
                };
                if ops.len() < 2 {
                    return Err("a function value read with an unexpected shape".into());
                }
                let first = self.expr(d, ind)?;
                let dt = self.temp(CType::Int);
                self.line(ind, &format!("{dt} = {};", as_int(first)?));
                for o in &ops[1..ops.len() - 1] {
                    self.stmt(o, ind)?;
                }
                let d = E::new(dt, K::Int);
                let clos = self.expr(clos, ind)?;
                self.line(
                    ind,
                    &format!(
                        "{target} = loft_fn_make({}, {});",
                        as_int(d)?,
                        as_ref(clos)?
                    ),
                );
                Ok(())
            }
            // A function named as a value: its definition number.
            Value::Int(d) if c == CType::Fn && self.cx.is_function(*d) => {
                let f = self.cx.fn_value(d.unsigned_abs());
                self.line(ind, &format!("{target} = {f};"));
                Ok(())
            }
            Value::Null => {
                self.null_into(target, c, ind);
                Ok(())
            }
            Value::Block(b) => {
                let delivery = self.delivery_block(b);
                self.in_delivery += u32::from(delivery);
                let r = self.assign_seq(&b.operators, target, c, ind);
                self.in_delivery -= u32::from(delivery);
                r
            }
            Value::Insert(ops) => self.assign_seq(ops, target, c, ind),
            Value::If(cond, t, e) => self.assign_if(cond, t, e, target, c, ind),
            Value::Tuple(vals) => self.assign_tuple(vals, target, c, ind),
            v @ (Value::Return(_) | Value::Break(_) | Value::Continue(_)) => self.stmt(v, ind),
            // A self tail call's loop (`opt::tail_self`) leaves only by a `return`: it has no
            // value to put anywhere.
            v @ Value::Loop(lp) if lp.name == "tail_self" => self.stmt(v, ind),
            v => {
                let e = self.expr(v, ind)?;
                // A temporary tuple that owns texts keeps them — its buffers are reused and the
                // epilogue frees them — so the target takes copies; a call's result moves.
                if let CType::Tuple(id) = c
                    && self.cx.tuple_has_text(id)
                    && self.temps.iter().any(|(t, _)| *t == e.code)
                {
                    self.copy_member(&e.code, target, c, ind);
                    return Ok(());
                }
                let s = match c {
                    CType::Text => format!("loft_text_set(&{target}, {});", as_view(e)?),
                    CType::Void => format!("{};", e.code),
                    c => format!("{target} = {};", as_shape(e, c)?),
                };
                self.line(ind, &s);
                Ok(())
            }
        }
    }

    /// A tuple literal into `target`, of shape `c`.
    fn assign_tuple(
        &mut self,
        vals: &[Value],
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        let CType::Tuple(id) = c else {
            return Err("a tuple literal into a non-tuple".into());
        };
        let shapes = self.cx.tuple_shapes(id);
        if shapes.len() != vals.len() {
            return Err("a tuple literal of another arity".into());
        }
        // Every element is evaluated before any is stored, as the bytecode pushes them
        // all first: `s = (s.1, s.0)` reads the old members.  A literal reads nothing.
        let constant = |x: &Value| {
            matches!(
                x.unspan(),
                Value::Int(_)
                    | Value::Long(_)
                    | Value::Float(_)
                    | Value::Single(_)
                    | Value::Boolean(_)
                    | Value::Text(_)
                    | Value::Null
            )
        };
        if vals.len() > 1 && !vals.iter().all(constant) {
            let mut held = Vec::with_capacity(vals.len());
            for (x, s) in vals.iter().zip(&shapes) {
                let t = self.temp(*s);
                self.assign(x, &t, *s, ind)?;
                held.push(t);
            }
            for (i, (t, s)) in held.iter().zip(shapes).enumerate() {
                self.copy_member(t, &format!("{target}.f{i}"), s, ind);
            }
            return Ok(());
        }
        for (i, (x, s)) in vals.iter().zip(shapes).enumerate() {
            self.assign(x, &format!("{target}.f{i}"), s, ind)?;
        }
        Ok(())
    }

    fn assign_seq(
        &mut self,
        ops: &[Value],
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        if self.tail_capture(ops).is_some() {
            return self.ops_stmt(ops, ind);
        }
        // The value is the last operator that is not a free: frees after it run once it is
        // taken, as they run under it on the bytecode's evaluation stack.  A `Set` has no value,
        // so the reset that follows a free (`__disp = OpNullRefSentinel()`) is one of them.
        let at = ops
            .iter()
            .rposition(|o| {
                !matches!(o.unspan(), Value::Line(_) | Value::Set(..)) && !self.is_free(o)
            })
            .ok_or("a value block with no value")?;
        ops[..at].iter().try_for_each(|o| self.stmt(o, ind))?;
        self.assign(&ops[at], target, c, ind)?;
        self.flush(ind);
        ops[at + 1..].iter().try_for_each(|o| self.stmt(o, ind))
    }

    /// `to = from` for a value of shape `c` at two C places: a text copied, a tuple member by
    /// member at any depth (its texts the destination's own), anything else assigned.
    pub(super) fn copy_member(&mut self, from: &str, to: &str, c: CType, ind: usize) {
        match c {
            CType::Text => self.line(ind, &format!("loft_text_set(&{to}, loft_view(&{from}));")),
            CType::Tuple(id) => {
                for (j, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    self.copy_member(&format!("{from}.f{j}"), &format!("{to}.f{j}"), e, ind);
                }
            }
            _ => self.line(ind, &format!("{to} = {from};")),
        }
    }

    /// `t = null` for a tuple whose texts stay where they are: every member but a text set to
    /// its null, so the text buffers are reused rather than lost.
    fn null_tuple_keeping_texts(&mut self, t: &str, c: CType, ind: usize) {
        match c {
            CType::Text => {}
            CType::Tuple(id) => {
                for (j, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    self.null_tuple_keeping_texts(&format!("{t}.f{j}"), e, ind);
                }
            }
            other => {
                let init = self.cx.initial(other);
                self.line(ind, &format!("{t} = ({}) {init};", super::held(other)));
            }
        }
    }

    /// Every text a tuple of struct `id` at C place `at` holds, nested tuples included.
    pub(super) fn tuple_text_leaves(&self, at: &str, id: u32, out: &mut Vec<String>) {
        for (j, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
            match e {
                CType::Text => out.push(format!("{at}.f{j}")),
                CType::Tuple(inner) => self.tuple_text_leaves(&format!("{at}.f{j}"), inner, out),
                _ => {}
            }
        }
    }

    /// `var = x` on a store link: a re-point takes the new place, anything else is written
    /// through the place (`link_set_repoints` tells them apart, as for the bytecode).
    fn store_link_set(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        if crate::scopes::link_set_repoints(self.cx.data, self.vars, var, x) {
            self.bound[var as usize] = true;
            let name = var_name(self.vars.name(var), var);
            let e = self.expr(x, ind)?;
            self.line(ind, &format!("{name} = {};", as_ref(e)?));
            self.flush(ind);
            return Ok(());
        }
        let write = self.link_access(var, Some(x))?;
        self.stmt(&write, ind)
    }

    /// The read (`value` none) or the write of store link `v`, as the operator the bytecode
    /// uses through the link's `DbRef`: the place's own get or set, a narrow integer's with its
    /// `min`, on `OpVarRef(v)` — the place itself.
    fn link_access(&self, v: u16, value: Option<&Value>) -> Result<Value, String> {
        use crate::data::Type;
        let Type::RefVar(tp) = self.vars.tp(v).base() else {
            return Err("a store link that is not a link".into());
        };
        let narrow = crate::data::NarrowSlot::of_type(tp);
        let (get, set) = match tp.base() {
            Type::Integer(_) if narrow.is_some() => {
                let n = narrow.expect("checked");
                (n.get_op(), n.set_op())
            }
            Type::Integer(_) => ("OpGetInt", "OpSetInt"),
            Type::Character => ("OpGetCharacter", "OpSetCharacter"),
            Type::Single => ("OpGetSingle", "OpSetSingle"),
            Type::Float => ("OpGetFloat", "OpSetFloat"),
            Type::Boolean => ("OpGetBoolean", "OpSetBoolean"),
            Type::Enum(_, false, _) => ("OpGetEnum", "OpSetEnum"),
            Type::Text(_) => ("OpGetText", "OpSetText"),
            _ => return Err("a store link of this kind".into()),
        };
        let data = self.cx.data;
        let op = data.def_nr(if value.is_some() { set } else { get });
        let place = Value::Call(data.def_nr("OpVarRef"), vec![Value::Var(v)]);
        let mut args = vec![place, Value::Int(0)];
        // A narrow kind's ops take the `min` after the field (`OpGetByte(r, fld, min)`).
        if data.def(op).attributes().len() == if value.is_some() { 4 } else { 3 } {
            args.push(Value::Int(narrow.map_or(0, |n| n.min)));
        }
        if let Some(x) = value {
            args.push(x.clone());
        }
        Ok(Value::Call(op, args))
    }

    /// The typed null of an operand of type `tp`, as an operator call, or `None` for a type
    /// whose null the bytecode does not push as one value.
    fn typed_null(&self, tp: &crate::data::Type) -> Option<Value> {
        use crate::data::Type;
        let op = match tp.base() {
            Type::Integer(..) => "OpConvIntFromNull",
            Type::Float => "OpConvFloatFromNull",
            Type::Single => "OpConvSingleFromNull",
            Type::Text(_) => "OpConvTextFromNull",
            Type::Boolean => "OpConvBoolFromNull",
            Type::Character => "OpConvCharacterFromNull",
            Type::Reference(..)
            | Type::Vector(..)
            | Type::Sorted(..)
            | Type::Hash(..)
            | Type::Index(..)
            | Type::Radix(..)
            | Type::Trie(..) => "OpNullRefSentinel",
            _ => return None,
        };
        let nr = self.cx.data.def_nr(op);
        (nr < self.cx.data.definitions()).then(|| Value::Call(nr, Vec::new()))
    }

    /// The C shape a call through function value `v` answers.
    fn call_ref_shape(&self, v: u16) -> Result<CType, String> {
        match self.vars.tp(v).base() {
            crate::data::Type::Function(_, ret, ..) => self
                .cx
                .ctype(ret)
                .ok_or_else(|| format!("a function value returning {}", type_word(ret))),
            _ => Err("a call through a value that is not a function".into()),
        }
    }

    /// The C shape `v` evaluates to, when it is a compound the caller must hold.
    fn shape_of(&self, v: &Value) -> Result<CType, String> {
        let cx = self.cx;
        match v.unspan() {
            Value::Block(b) => cx
                .ctype(&b.result)
                .ok_or_else(|| format!("a block of type {}", type_word(&b.result))),
            Value::Insert(ops) => ops.last().map_or(Ok(CType::Void), |l| self.shape_of(l)),
            Value::If(_, t, e) => match self.shape_of(t) {
                Ok(CType::Void) | Err(_) => self.shape_of(e),
                ok => ok,
            },
            Value::Int(_) | Value::Long(_) => Ok(CType::Int),
            Value::Enum(..) => Ok(CType::Enum),
            Value::Float(_) => Ok(CType::Float),
            Value::Single(_) => Ok(CType::Single),
            Value::Boolean(_) => Ok(CType::Bool),
            Value::Text(_) => Ok(CType::Text),
            Value::Var(v) => Ok(self.shapes[*v as usize]),
            Value::FnRef(..) => Ok(CType::Fn),
            Value::FnRefDnr(_) => Ok(CType::Int),
            Value::CallRef(v, _) => self.call_ref_shape(*v),
            Value::TupleGet(v, i) => match self.shapes[*v as usize] {
                CType::Tuple(id) => cx
                    .tuple_shapes(id)
                    .get(*i as usize)
                    .copied()
                    .ok_or_else(|| "a tuple index out of range".into()),
                _ => Err("a tuple read of a non-tuple".into()),
            },
            Value::Tuple(vals) => {
                let shapes = vals
                    .iter()
                    .map(|x| self.shape_of(x))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CType::Tuple(cx.tuple_id(shapes)))
            }
            Value::Call(d, _) if !is_operator(cx.data, *d) => {
                let tp = cx.data.def(*d).returned();
                cx.ctype(tp)
                    .ok_or_else(|| format!("a call returning {}", type_word(tp)))
            }
            Value::Call(d, _) => {
                let name = cx.data.def(*d).name();
                op_shape(name).ok_or_else(|| format!("operator {name}"))
            }
            other => Err(unsupported(other)),
        }
    }

    /// A C expression for `v`'s value; the statements it needs are emitted first.
    fn expr(&mut self, v: &Value, ind: usize) -> Result<E, String> {
        match v.unspan() {
            Value::Int(i) => Ok(E::new(format!("INT64_C({i})"), K::Int)),
            Value::Long(l) if *l == i64::MIN => Ok(E::new("LOFT_NULL", K::Int)),
            Value::Long(l) => Ok(E::new(c_long(*l), K::Int)),
            Value::Boolean(b) => Ok(E::new(if *b { "1" } else { "0" }, K::Cond)),
            Value::Enum(v, _) => Ok(E::new(format!("(uint8_t) {v}"), K::Enum)),
            Value::Float(f) => Ok(E::new(
                format!("loft_f64(UINT64_C({:#x}))", f.to_bits()),
                K::Float,
            )),
            Value::Single(f) => Ok(E::new(
                format!("loft_f32(UINT32_C({:#x}))", f.to_bits()),
                K::Single,
            )),
            Value::Text(s) => Ok(E::new(
                format!("loft_lit({}, {})", c_string(s), s.len()),
                K::View,
            )),
            Value::Var(var) if self.store_links[*var as usize] => {
                let read = self.link_access(*var, None)?;
                self.expr(&read, ind)
            }
            Value::Var(var) => {
                let name = self.var(*var);
                Ok(match self.shapes[*var as usize] {
                    CType::Text if self.text_held(*var) => {
                        E::new(format!("loft_view(&{name})"), K::View)
                    }
                    CType::Void => return Err("a void variable".into()),
                    c => E::new(name, kind_of(c)),
                })
            }
            Value::TupleGet(var, i) => {
                let c = self.shape_of(&Value::TupleGet(*var, *i))?;
                let place = format!("{}.f{i}", self.var(*var));
                if c == CType::Text {
                    return Ok(E::new(format!("loft_view(&{place})"), K::View));
                }
                Ok(E::new(place, kind_of(c)))
            }
            Value::Call(d, args) => self.call(*d, args, ind),
            Value::CallRef(v, args) => self.call_ref(*v, args, ind),
            // The definition number a function value names, as an integer.
            Value::FnRefDnr(v) => Ok(E::new(format!("{}.d", self.var(*v)), K::Int)),
            Value::FnRef(d, clos, _) => {
                let f = self.cx.fn_value(d.unsigned_abs());
                if *clos == u16::MAX {
                    Ok(E::new(f, K::Fn))
                } else {
                    Ok(E::new(
                        format!("loft_fn_make(INT64_C({d}), {})", self.var(*clos)),
                        K::Fn,
                    ))
                }
            }
            compound @ (Value::Block(_) | Value::Insert(_) | Value::If(..) | Value::Tuple(_)) => {
                let c = self.shape_of(compound)?;
                self.nested(|w| {
                    if c == CType::Void {
                        w.stmt(compound, ind)?;
                        Ok(E::new("(void) 0", K::Void))
                    } else {
                        let t = w.temp(c);
                        w.assign(compound, &t, c, ind)?;
                        Ok(w.held(t, c))
                    }
                })
            }
            Value::Null => Err("a null without a type".into()),
            other => Err(unsupported(other)),
        }
    }

    /// Statements written inside an expression: they free only the temporaries they make, as
    /// the enclosing expression's are still to be read once they are done.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let outer = std::mem::take(&mut self.pending);
        let r = f(self);
        let inner = std::mem::replace(&mut self.pending, outer);
        self.pending.extend(inner);
        r
    }

    /// The expression reading temporary `t` of shape `c`.
    fn held(&mut self, t: String, c: CType) -> E {
        match c {
            CType::Text => {
                self.pending.push(t.clone());
                E::new(format!("loft_view(&{t})"), K::View)
            }
            CType::Void => E::new("(void) 0", K::Void),
            c => E::new(t, kind_of(c)),
        }
    }

    /// The name of the text local `v` an operator writes into.
    fn text_dest(&self, v: &Value) -> Result<String, String> {
        match v.unspan() {
            Value::Var(var) if self.text_held(*var) => Ok(self.var(*var)),
            Value::TupleGet(var, i)
                if matches!(self.shapes[*var as usize], CType::Tuple(id)
                    if self.cx.tuple_shapes(id).get(*i as usize) == Some(&CType::Text)) =>
            {
                Ok(format!("{}.f{i}", self.var(*var)))
            }
            Value::Var(var) => Err(format!(
                "a text operator writing into a {:?} variable",
                self.shapes[*var as usize]
            )),
            other => Err(format!(
                "a text operator writing into {}",
                unsupported(other)
            )),
        }
    }

    /// Evaluate `args` in order: anything but a variable or a literal goes into a temporary
    /// first, because C leaves the order of a call's arguments open and loft does not.
    ///
    /// A variable is read in place unless a later operand may write it (@FR-E-Left: an
    /// operand is read before a later operand writes) — then it is read into a temporary too.
    fn args(&mut self, args: &[Value], ind: usize) -> Result<Vec<E>, String> {
        let mut out = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let e = self.expr(a, ind)?;
            let later_writes = args[i + 1..].iter().any(|b| self.may_write(b));
            if later_writes && matches!(a.unspan(), Value::Var(_)) && e.kind == K::View {
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_set(&{t}, {});", e.code));
                out.push(self.held(t, CType::Text));
                continue;
            }
            let simple = match a.unspan() {
                Value::Var(_) => !later_writes,
                Value::Int(_) | Value::Long(_) | Value::Boolean(_) | Value::Text(_) => true,
                _ => false,
            };
            // A temporary is already stable — and a second one would share the texts a tuple
            // temporary owns, which the epilogue frees through each.
            if simple
                || matches!(e.kind, K::View | K::Void)
                || self.temps.iter().any(|(t, _)| *t == e.code)
            {
                out.push(e);
                continue;
            }
            let (c, code) = match e.kind {
                K::Int => (CType::Int, e.code),
                K::Ref => (CType::Ref, e.code),
                K::Tup(id) => (CType::Tuple(id), e.code),
                K::Enum => (CType::Enum, e.code),
                K::Float => (CType::Float, e.code),
                K::Single => (CType::Single, e.code),
                K::Char => (CType::Char, e.code),
                K::Fn => (CType::Fn, e.code),
                _ => (CType::Bool, as_bool(e)?),
            };
            let t = self.temp(c);
            self.line(ind, &format!("{t} = {code};"));
            out.push(E::new(t, kind_of(c)));
        }
        Ok(out)
    }

    /// May evaluating `v` write a variable?  A statement, a block or a call that is not a
    /// pure operator may; a conservative answer only costs a temporary.
    fn may_write(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Var(_)
            | Value::Int(_)
            | Value::Long(_)
            | Value::Boolean(_)
            | Value::Text(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Enum(..)
            | Value::Null
            | Value::TupleGet(..)
            | Value::FnRef(..) => false,
            Value::Call(d, xs) if is_operator(self.cx.data, *d) => {
                xs.iter().any(|x| self.may_write(x))
            }
            _ => true,
        }
    }

    fn call(&mut self, d: u32, args: &[Value], ind: usize) -> Result<E, String> {
        let cx = self.cx;
        if is_operator(cx.data, d) {
            // A bare `null` argument is the null of the operand's declared type, as the
            // bytecode pushes it (`OpSetDbRef(g, 0, null)`, `OpPushInt(v, null)`).
            if args.iter().any(|a| matches!(a.unspan(), Value::Null)) {
                let attrs = cx.data.def(d).attributes();
                let typed: Vec<Value> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| match a.unspan() {
                        Value::Null => attrs
                            .get(i)
                            .and_then(|at| self.typed_null(&at.typedef))
                            .unwrap_or(Value::Null),
                        _ => a.clone(),
                    })
                    .collect();
                return self.operator(cx.data.def(d).name(), &typed, ind);
            }
            return self.operator(cx.data.def(d).name(), args, ind);
        }
        let def = cx.data.def(d);
        if let Some(e) = self.keyed_native(def.name(), args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.par_native(def.name(), args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.unicode_native(def.name(), args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.struct_json_native(def.name(), args, ind)? {
            return Ok(e);
        }
        if def.name() == "n_env_variables" {
            // The element is the stdlib's `EnvVariable`, sized from this program's table.
            let size = self.cx.stores.size(self.cx.stores.name("EnvVariable"));
            return Ok(E::new(format!("loft_env_variables({size})"), K::Ref));
        }
        let callee = def.original_name();
        let ret = cx
            .ctype(def.returned())
            .ok_or_else(|| format!("a call to {callee} returning {}", type_word(def.returned())))?;
        let shapes: Vec<CType> = def
            .attributes()
            .iter()
            .map(|a| {
                let tp = a.typedef.peel_link();
                cx.ctype(tp).ok_or_else(|| {
                    format!(
                        "a call to {callee} with a {} parameter",
                        type_word(&a.typedef)
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        if shapes.len() != args.len() {
            return Err("a call whose arguments do not match its parameters".into());
        }
        // `v.len()` of a vector an enclosing loop holds: the header's length.
        if def.name() == "t_6vector_len"
            && let [v] = args
        {
            if let Some(h) = self.held_vector(v) {
                return Ok(E::new(format!("{h}.len"), K::Int));
            }
            if let Some(e) = self.pushed_mint("OpLengthVector", args, ind)? {
                return Ok(e);
            }
        }
        let name = super::native_c(def.name()).map_or_else(|| c_name(cx.data, d), str::to_string);
        // A link argument is the address of the caller's variable; the others are values,
        // evaluated in order.
        let mut xs = vec![String::new(); args.len()];
        let (values, places) = self.link_args(d, args, &shapes, &mut xs, ind)?;
        self.value_args(args, &shapes, &values, &mut xs, ind)?;
        // `@FR-R-Inputs`: a callee whose every header input this body holds takes them.
        let mut mask = places;
        if super::native_c(def.name()).is_none()
            && args.len() < 64
            && let Some(headers) = self.twin_headers(d, args)
        {
            xs.extend(headers);
            mask |= super::INV_BIT;
        }
        crate::rewrite_census::fired("R-Inputs", usize::from(mask & super::INV_BIT != 0));
        let name = if mask == 0 {
            name
        } else if super::native_c(def.name()).is_some() || args.len() > 63 {
            return Err(format!("a store place handed to a & parameter of {callee}"));
        } else {
            cx.request_twin(d, mask);
            super::twin_name(cx.data, d, mask)
        };
        let call = format!("{name}({})", xs.join(", "));
        Ok(match ret {
            CType::Text => {
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(ind, &format!("{t} = {call};"));
                self.held(t, CType::Text)
            }
            c => E::new(call, kind_of(c)),
        })
    }

    /// The headers `d`'s `__inv` twin takes for a call with `args`, as native's
    /// `twin_call_inputs` keys them: each header input's path re-spelled over the argument —
    /// every one held by an enclosing frame, or `None`.  A header passed under growth carries
    /// a base derived at the call, valid while the callee runs, which grows no store.
    fn twin_headers(&self, d: u32, args: &[Value]) -> Option<Vec<String>> {
        if self.gen_elem.is_some() || self.headers.iter().all(Vec::is_empty) {
            return None;
        }
        let fi = self.cx.callee_inputs(d)?;
        if fi.headers.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(fi.headers.len());
        for (p, offs, _) in &fi.headers {
            let (c, mut key) =
                crate::opt::writes::vector_path(self.cx.data, args.get(*p as usize)?)?;
            key.extend_from_slice(offs);
            out.push(self.held_path(&(c, key))?);
        }
        Some(out)
    }

    /// Evaluate the arguments at `values` into `xs`, in order; a function named where a
    /// function value is passed becomes that value.
    fn value_args(
        &mut self,
        args: &[Value],
        shapes: &[CType],
        values: &[usize],
        xs: &mut [String],
        ind: usize,
    ) -> Result<(), String> {
        let mut rest = Vec::new();
        for &i in values {
            match args[i].unspan() {
                Value::Int(d) if shapes[i] == CType::Fn && self.cx.is_function(*d) => {
                    xs[i] = self.cx.fn_value(d.unsigned_abs());
                }
                // A bare null takes the parameter's shape.
                Value::Null if shapes[i] != CType::Void => {
                    xs[i] = match shapes[i] {
                        CType::Text => "loft_null_text()".into(),
                        c => format!("(({}) {})", held(c), self.cx.initial(c)),
                    };
                }
                _ => rest.push(i),
            }
        }
        let vals: Vec<Value> = rest.iter().map(|&i| args[i].clone()).collect();
        let es = self.args(&vals, ind)?;
        for (e, &i) in es.into_iter().zip(&rest) {
            xs[i] = as_shape(e, shapes[i])?;
        }
        Ok(())
    }

    /// A call through function value `v`: its signature's dispatcher.  A text result's work
    /// buffers the IR appends after the visible arguments are the dispatcher's to make.
    fn call_ref(&mut self, v: u16, args: &[Value], ind: usize) -> Result<E, String> {
        let crate::data::Type::Function(params, ret, ..) = self.vars.tp(v).base().clone() else {
            return Err("a call through a value that is not a function".into());
        };
        let cx = self.cx;
        // A `&` parameter passes the address of the caller's variable, as a direct call does.
        let sig_params = params
            .iter()
            .map(|p| match p.base() {
                crate::data::Type::RefVar(inner) => cx
                    .ctype(inner)
                    .filter(|c| *c != CType::Void)
                    .map(|c| (c, true)),
                p => cx
                    .ctype(p)
                    .filter(|c| *c != CType::Void)
                    .map(|c| (c, false)),
            })
            .collect::<Option<Vec<_>>>()
            .ok_or("a function value with a parameter of no C shape")?;
        let shapes: Vec<CType> = sig_params.iter().map(|(c, _)| *c).collect();
        let ret = cx
            .ctype(&ret)
            .ok_or_else(|| format!("a function value returning {}", type_word(&ret)))?;
        if args.len() < shapes.len() {
            return Err("a call through a function value with arguments left out".into());
        }
        let mut xs = vec![String::new(); shapes.len()];
        let mut values = Vec::new();
        // Each parameter's mode: a value (0), a link to a variable (1), a store place (2) —
        // the last reaches each function through its place twin (`twin_name`).
        let mut modes = Vec::with_capacity(shapes.len());
        for (i, (c, link)) in sig_params.iter().enumerate() {
            if *link {
                if let Some(p) = self.place_arg(&args[i], &params[i], ind)? {
                    xs[i] = p;
                    modes.push((*c, 2));
                } else {
                    xs[i] = self.link_arg(&args[i], *c, ind)?;
                    modes.push((*c, 1));
                }
            } else {
                values.push(i);
                modes.push((*c, 0));
            }
        }
        let sig = cx.fn_sig(modes, ret);
        self.value_args(&args[..shapes.len()], &shapes, &values, &mut xs, ind)?;
        let mut list = vec![self.var(v)];
        list.extend(xs);
        let call = format!("loft_call{sig}({})", list.join(", "));
        Ok(match ret {
            CType::Text => {
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(ind, &format!("{t} = {call};"));
                self.held(t, CType::Text)
            }
            c => E::new(call, kind_of(c)),
        })
    }

    /// The `&` arguments of a call to `d` into `xs`: the address of a variable, or the
    /// `loft_ref` of a store place — a field, an element, a store link the caller holds —
    /// which calls the callee's place twin, taking that parameter as a `loft_ref`.  Answers
    /// the positions of the value arguments and the twin's mask.
    fn link_args(
        &mut self,
        d: u32,
        args: &[Value],
        shapes: &[CType],
        xs: &mut [String],
        ind: usize,
    ) -> Result<(Vec<usize>, u64), String> {
        let def = self.cx.data.def(d);
        let callee_args = def.variables().arguments();
        let mut values = Vec::new();
        let mut places = 0u64;
        for (i, arg) in args.iter().enumerate() {
            let param = &def.attributes()[i].typedef;
            if !matches!(param.base(), crate::data::Type::RefVar(_)) {
                values.push(i);
                continue;
            }
            let Some(p) = self.place_arg(arg, param, ind)? else {
                xs[i] = self.link_arg(arg, shapes[i], ind)?;
                continue;
            };
            // A store instance takes the place as it is; a text place reaches no other
            // function (the IR retargets each such call to its instance).
            if !callee_args
                .get(i)
                .is_some_and(|&v| def.variables().is_store_text_link(v))
            {
                if matches!(param.peel_link().base(), crate::data::Type::Text(_)) {
                    return Err(format!("a text place handed to {}", def.original_name()));
                }
                places |= 1 << i;
            }
            xs[i] = p;
        }
        Ok((values, places))
    }

    /// A `&` argument that names a store place — a field (`OpGetField`), an element
    /// (`OpGetVector`), a store link the caller holds — as the `loft_ref` of the place, for a
    /// parameter of a kind a store link carries (`store_links`); `None` for a variable.
    fn place_arg(
        &mut self,
        a: &Value,
        param: &crate::data::Type,
        ind: usize,
    ) -> Result<Option<String>, String> {
        let crate::data::Type::RefVar(inner) = param.base() else {
            return Ok(None);
        };
        if !link_kind(inner) {
            return Ok(None);
        }
        match a.unspan() {
            Value::Var(v) if self.store_links[*v as usize] => {
                Ok(Some(var_name(self.vars.name(*v), *v)))
            }
            Value::Call(op, _)
                if matches!(
                    self.cx.data.def(*op).name(),
                    "OpGetField" | "OpGetVector" | "OpVarRef"
                ) =>
            {
                let e = self.expr(a, ind)?;
                let t = self.temp(CType::Ref);
                self.line(ind, &format!("{t} = {};", as_ref(e)?));
                Ok(Some(t))
            }
            _ => Ok(None),
        }
    }

    /// The address a `&` argument passes: the caller's own variable, or a link it holds.
    fn link_arg(&mut self, a: &Value, c: CType, ind: usize) -> Result<String, String> {
        let v = match a.unspan() {
            // A text result's buffer: `{ __work = ""; OpCreateStack(__work) }`.
            Value::Block(b) => {
                let Some((last, rest)) = b.operators.split_last() else {
                    return Err("a link block with no operators".into());
                };
                self.nested(|w| rest.iter().try_for_each(|o| w.stmt(o, ind)))?;
                return self.link_arg(last, c, ind);
            }
            Value::Call(op, xs) if self.cx.data.def(*op).name() == "OpCreateStack" => {
                match xs.as_slice() {
                    [x] => match x.unspan() {
                        Value::Var(v) => *v,
                        Value::TupleGet(tup, member) => {
                            let CType::Tuple(id) = self.shapes[*tup as usize] else {
                                return Err("a link to a member of no tuple".into());
                            };
                            if self.cx.tuple_shapes(id).get(*member as usize) != Some(&c) {
                                return Err("a link to a member of another shape".into());
                            }
                            return self
                                .stack_target(a)
                                .ok_or_else(|| "a link to a tuple member it cannot name".into());
                        }
                        _ => return Err("a link to a computed place".into()),
                    },
                    _ => return Err("OpCreateStack without one operand".into()),
                }
            }
            Value::Var(v) if self.links[*v as usize] => *v,
            Value::Call(op, _) => {
                return Err(format!("a link made by {}", self.cx.data.def(*op).name()));
            }
            other => return Err(format!("a link to {}", unsupported(other))),
        };
        if self.shapes[v as usize] != c || (c == CType::Text && !self.text_held(v)) {
            return Err("a link to a variable of another shape".into());
        }
        Ok(format!("&{}", self.var(v)))
    }

    fn one(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        let es = self.args(args, ind)?;
        let [a] = <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
        Ok(a)
    }

    fn two(&mut self, name: &str, args: &[Value], ind: usize) -> Result<(E, E), String> {
        let es = self.args(args, ind)?;
        let [a, b] = <[E; 2]>::try_from(es).map_err(|_| format!("{name} without two operands"))?;
        Ok((a, b))
    }

    fn operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        // The text writes through a `&` parameter: the C names the caller's text the same way.
        let name = match name {
            "OpAppendStackText" => "OpAppendText",
            "OpClearStackText" => "OpClearText",
            "OpFormatStackInt" => "OpFormatInt",
            "OpFormatStackFloat" => "OpFormatFloat",
            "OpFormatStackSingle" => "OpFormatSingle",
            "OpFormatStackText" => "OpFormatText",
            "OpFormatStackDatabase" => "OpFormatDatabase",
            n => n,
        };
        if !self.headers.is_empty()
            && let Some(e) = self.held_access(name, args, ind)?
        {
            return Ok(e);
        }
        if !self.windows.is_empty()
            && let Some(e) = self.windowed(name, args, ind)?
        {
            return Ok(e);
        }
        if self.push_heads.iter().any(|f| !f.is_empty())
            && let Some(e) = self.pushed_mint(name, args, ind)?
        {
            return Ok(e);
        }
        if !self.lazy.is_empty()
            && let Some(e) = self.lazy_split_op(name, args)
        {
            return Ok(e);
        }
        if let Some(e) = self.keyed_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.file_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.gen_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.range_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.probe_operator(name, args, ind)? {
            return Ok(e);
        }
        let arith = match name {
            // The `Nullable` forms answer the same values; only the overflow report differs.
            "OpAddInt" | "OpAddLong" | "OpAddIntNullable" => Some("loft_add"),
            "OpMinInt" | "OpMinLong" | "OpMinIntNullable" => Some("loft_sub"),
            "OpMulInt" | "OpMulLong" | "OpMulIntNullable" => Some("loft_mul"),
            "OpDivInt" | "OpDivLong" | "OpDivIntNullable" => Some("loft_div"),
            "OpRemInt" | "OpRemLong" | "OpRemIntNullable" => Some("loft_rem"),
            "OpLandInt" | "OpLandLong" => Some("loft_and"),
            "OpLorInt" | "OpLorLong" => Some("loft_or"),
            "OpEorInt" | "OpEorLong" => Some("loft_xor"),
            "OpSLeftInt" | "OpSLeftLong" => Some("loft_shl"),
            "OpSRightInt" | "OpSRightLong" => Some("loft_shr"),
            _ => None,
        };
        if let Some(f) = arith {
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(
                format!("{f}({}, {})", as_int(a)?, as_int(b)?),
                K::Int,
            ));
        }
        let cmp = match name {
            "OpEqInt" | "OpEqLong" => Some("=="),
            "OpNeInt" | "OpNeLong" => Some("!="),
            "OpLtInt" | "OpLtLong" => Some("<"),
            "OpLeInt" | "OpLeLong" => Some("<="),
            _ => None,
        };
        if let Some(op) = cmp {
            let (a, b) = self.two(name, args, ind)?;
            return Ok(compare(&as_int(a)?, op, &as_int(b)?));
        }
        self.text_operator(name, args, ind)
    }

    /// `target = if cond { t } else { e }`, each arm assigning it; a not-null test's else opens
    /// with `LOFT_COLD_PATH()` (see [`Self::null_test`]).
    fn assign_if(
        &mut self,
        cond: &Value,
        t: &Value,
        e: &Value,
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        let null_test = self.null_test(cond);
        let cond = self.expr(cond, ind)?;
        let cond = as_cond(cond)?;
        self.line(ind, &format!("if ({}) {{", bare(&cond)));
        self.assign(t, target, c, ind + 1)?;
        self.line(ind, "} else {");
        if null_test {
            self.line(ind + 1, "LOFT_COLD_PATH();");
        }
        self.assign(e, target, c, ind + 1)?;
        self.line(ind, "}");
        Ok(())
    }

    /// Is `c` a not-null test (`x != null` of a number)?  Its else is the null's path, which
    /// `LOFT_COLD_PATH()` keeps a branch: a discharge is then a predicted jump, not a select in
    /// the dependency chain of the code around it.
    fn null_test(&self, c: &Value) -> bool {
        matches!(c.unspan(), Value::Call(d, a) if a.len() == 1
            && matches!(self.cx.data.def(*d).name(),
                "OpConvBoolFromFloat" | "OpConvBoolFromSingle"
                | "OpConvBoolFromInt" | "OpConvBoolFromLong"))
    }

    /// `@FR-R-Range`, as native writes it: an integer operator whose result the range proof
    /// (`opt::range::op_range`, over this function's facts) shows fits the type cannot fault,
    /// so it is the plain C operator — the exact value, the processor's instruction, and a
    /// signed operation the compiler may reason about (a counted loop's step is one: its
    /// wrapping or checked forms keep GCC from counting the loop).  Both lanes; a division
    /// only where the range excludes a zero divisor.  `LOFT_NO_RANGE_ARITH=1` keeps every
    /// operator checked, as on native.
    fn range_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let Some(form) = crate::opt::range::plain_form(name, args.len()) else {
            return Ok(None);
        };
        if crate::env_once!(std::env::var("LOFT_NO_RANGE_ARITH").is_ok_and(|v| v != "0")) {
            return Ok(None);
        }
        let facts = if let Some(f) = &self.range_facts {
            f.clone()
        } else {
            {
                let data = self.cx.data;
                let def = data.def(self.d);
                let nn =
                    crate::opt::non_sentinel::non_sentinel_vars(data, def.variables(), def.code());
                let rv = crate::opt::range::range_vars(
                    data,
                    def.variables(),
                    def.code(),
                    &nn,
                    &self.char_walks,
                    &crate::opt::range::Ranges::default(),
                );
                let f = std::rc::Rc::new((nn, rv));
                self.range_facts = Some(f.clone());
                f
            }
        };
        if crate::opt::range::op_range(self.cx.data, &facts.0, &facts.1, name, args, 0).is_none() {
            return Ok(None);
        }
        crate::rewrite_census::fired("R-Range", 1);
        if form == "wrapping_neg" {
            let a = self.one(name, args, ind)?;
            return Ok(Some(E::new(format!("(-({}))", as_int(a)?), K::Int)));
        }
        let op = match form {
            "wrapping_add" => "+",
            "wrapping_sub" => "-",
            "wrapping_mul" => "*",
            other => other,
        };
        let (a, b) = self.two(name, args, ind)?;
        Ok(Some(E::new(
            format!("(({}) {op} ({}))", as_int(a)?, as_int(b)?),
            K::Int,
        )))
    }

    /// `LOFT_RELEASE_PASS_PROBE=1` (generation time): the release-pass probe's form of an
    /// integer operator — the processor's wrapping arithmetic, each wrap defined
    /// (`loft_wrap_*` in `loft_rt.h`) — for exactly the operators and operand shapes
    /// `generation::ops::int_arith::probe_form` gives the Rust generator, so the C and Rust
    /// ship lanes measure the same tier.  The `Nullable` forms and the null test keep their
    /// checks: they are the language's null semantics, not fault protection.  A MEASUREMENT
    /// instrument (@C129), never a build anyone ships.
    fn probe_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if !crate::env_once!(std::env::var("LOFT_RELEASE_PASS_PROBE").is_ok_and(|v| v != "0")) {
            return Ok(None);
        }
        let shift_k =
            || matches!(args.get(1).map(Value::unspan), Some(Value::Int(k)) if (0..64).contains(k));
        let form = match (name, args.len()) {
            ("OpAddInt", 2) => "loft_wrap_add",
            ("OpMinInt", 2) => "loft_wrap_sub",
            ("OpMulInt", 2) => "loft_wrap_mul",
            ("OpDivInt", 2) => "loft_wrap_div",
            ("OpRemInt", 2) => "loft_wrap_rem",
            ("OpLandInt", 2) => "&",
            ("OpLorInt", 2) => "|",
            ("OpEorInt", 2) => "^",
            ("OpSRightInt", 2) if shift_k() => ">>",
            ("OpMinSingleInt", 1) => {
                let a = self.one(name, args, ind)?;
                return Ok(Some(E::new(
                    format!("loft_wrap_neg({})", as_int(a)?),
                    K::Int,
                )));
            }
            _ => return Ok(None),
        };
        let (a, b) = self.two(name, args, ind)?;
        let (a, b) = (as_int(a)?, as_int(b)?);
        let text = if form.starts_with("loft_") {
            format!("{form}({a}, {b})")
        } else {
            format!("(({a}) {form} ({b}))")
        };
        Ok(Some(E::new(text, K::Int)))
    }

    /// The boolean and text operators, and printing.
    #[expect(clippy::too_many_lines, reason = "one arm per operator")]
    fn text_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match name {
            "OpEqBool" | "OpNeBool" => {
                let (a, b) = self.two(name, args, ind)?;
                let op = if name == "OpEqBool" { "==" } else { "!=" };
                Ok(compare(&as_bool(a)?, op, &as_bool(b)?))
            }
            "OpNot" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| "OpNot without one operand".to_string())?;
                Ok(E::new(format!("(!{})", as_cond(a)?), K::Cond))
            }
            "OpEqText" | "OpNeText" => {
                let (a, b) = self.two(name, args, ind)?;
                let neg = if name == "OpNeText" { "!" } else { "" };
                Ok(E::new(
                    format!("{neg}loft_str_eq({}, {})", as_view(a)?, as_view(b)?),
                    K::Cond,
                ))
            }
            "OpLtText" | "OpLeText" => {
                let (a, b) = self.two(name, args, ind)?;
                let f = if name == "OpLtText" {
                    "loft_text_lt"
                } else {
                    "loft_text_le"
                };
                Ok(E::new(
                    format!("{f}({}, {})", as_view(a)?, as_view(b)?),
                    K::Cond,
                ))
            }
            "OpAppendText" => {
                let [dest, val] = args else {
                    return Err("OpAppendText without two operands".into());
                };
                let dest = self.text_dest(dest)?;
                let val = self.expr(val, ind)?;
                Ok(E::new(
                    format!("loft_text_concat(&{dest}, {})", as_view(val)?),
                    K::Void,
                ))
            }
            "OpFormatInt" | "OpFormatLong" => self.format_long(name, args, ind),
            "OpFormatText" => {
                let [dest, val, width, dir, token] = args else {
                    return Err("OpFormatText with an unexpected operand list".into());
                };
                let dest = self.text_dest(dest)?;
                let (Value::Int(dir), Value::Int(token)) = (dir.unspan(), token.unspan()) else {
                    return Err("OpFormatText with a computed format".into());
                };
                let (val, width) = self.two(name, &[(*val).clone(), (*width).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_format_text(&{dest}, {}, {}, {dir}, {token})",
                        as_view(val)?,
                        as_int(width)?
                    ),
                    K::Void,
                ))
            }
            "OpFormatDatabase" => self.format_database(args, ind),
            "OpFreeText" => {
                let [dest] = args else {
                    return Err("OpFreeText without one operand".into());
                };
                // A member that is itself a tuple frees each text it holds, at any depth, as the
                // bytecode's `emit_tuple_text_free` does.
                if let Value::TupleGet(var, i) = dest.unspan()
                    && let CType::Tuple(id) = self.shapes[*var as usize]
                    && let Some(CType::Tuple(inner)) = self.cx.tuple_shapes(id).get(*i as usize)
                {
                    let mut frees = Vec::new();
                    self.tuple_text_leaves(&format!("{}.f{i}", self.var(*var)), *inner, &mut frees);
                    let code = frees
                        .iter()
                        .map(|t| format!("loft_text_free(&{t})"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Ok(E::new(
                        if code.is_empty() {
                            "((void) 0)".into()
                        } else {
                            format!("({code})")
                        },
                        K::Void,
                    ));
                }
                // A borrowed walk variable views the element: its release frees nothing.
                if matches!(dest.unspan(), Value::Var(v) if self.borrowed[*v as usize]) {
                    return Ok(E::new("((void) 0)", K::Void));
                }
                let dest = self.text_dest(dest)?;
                Ok(E::new(format!("loft_text_free(&{dest})"), K::Void))
            }
            "OpPrint" => {
                let es = self.args(args, ind)?;
                let [a] = <[E; 1]>::try_from(es)
                    .map_err(|_| "OpPrint without one operand".to_string())?;
                Ok(E::new(format!("loft_print({})", as_view(a)?), K::Void))
            }
            _ => self.scalar_operator(name, args, ind),
        }
    }

    /// An integer into a format string: `ops::format_long`.
    fn format_long(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        let [dest, val, radix, width, token, plus, note, dir] = args else {
            return Err(format!("{name} with an unexpected operand list"));
        };
        let dest = self.text_dest(dest)?;
        let (
            Value::Int(radix),
            Value::Int(token),
            Value::Boolean(plus),
            Value::Boolean(note),
            Value::Int(dir),
        ) = (
            radix.unspan(),
            token.unspan(),
            plus.unspan(),
            note.unspan(),
            dir.unspan(),
        )
        else {
            return Err(format!("{name} with a computed format"));
        };
        let (val, width) = self.two(name, &[(*val).clone(), (*width).clone()], ind)?;
        Ok(E::new(
            format!(
                "loft_format_long(&{dest}, {}, {radix}, {}, {token}, {}, {}, {dir})",
                as_int(val)?,
                as_int(width)?,
                u8::from(*plus),
                u8::from(*note)
            ),
            K::Void,
        ))
    }

    /// `T.to_json()` / `T.to_json_pretty()` of a record (`Stores::show_json`): its type's
    /// render function in the JSON form; `T.parse(JsonValue)`, the walk over the tree; `None`
    /// for another native.
    fn struct_json_native(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let pretty = match name {
            "n_struct_to_json" => 0,
            "n_struct_to_json_pretty" => 1,
            // `T.parse(JsonValue)`: the struct walk over the tree (`loft_parse.h`).
            "n_struct_from_jsonvalue" => {
                let [val, tp] = args else {
                    return Err(format!("{name} with an unexpected operand list"));
                };
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.parse_type(tp)?;
                let val = self.expr(val, ind)?;
                return Ok(Some(E::new(
                    format!(
                        "loft_struct_from_jsonvalue(loft_ptypes, {}, {tp})",
                        as_ref(val)?
                    ),
                    K::Ref,
                )));
            }
            _ => return Ok(None),
        };
        let [val, tp] = args else {
            return Err(format!("{name} with an unexpected operand list"));
        };
        let tp = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        self.cx.show_type(tp)?;
        let val = self.expr(val, ind)?;
        let t = self.temp(CType::Text);
        self.line(ind, &format!("loft_text_free(&{t});"));
        self.line(
            ind,
            &format!(
                "{}(&{t}, {}, {}, 0);",
                super::Cx::show_fn(tp),
                as_ref(val)?,
                2 | pretty
            ),
        );
        Ok(Some(self.held(t, CType::Text)))
    }

    /// `{v}` of a record or a vector: the type's render function (`show.rs`).
    fn format_database(&mut self, args: &[Value], ind: usize) -> Result<E, String> {
        let [dest, val, tp, fmt] = args else {
            return Err("OpFormatDatabase with an unexpected operand list".into());
        };
        let dest = self.text_dest(dest)?;
        let tp = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        let fmt = Self::literal(fmt, "format")?;
        self.cx.show_type(tp)?;
        let val = self.expr(val, ind)?;
        Ok(E::new(
            format!(
                "{}(&{dest}, {}, {fmt}, 0)",
                super::Cx::show_fn(tp),
                as_ref(val)?
            ),
            K::Void,
        ))
    }

    /// The conversions and the text measures.
    fn scalar_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match name {
            "OpLengthText" | "OpSizeText" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                let f = if name == "OpLengthText" {
                    "loft_text_length"
                } else {
                    "loft_text_size"
                };
                Ok(E::new(format!("{f}({})", as_view(a)?), K::Int))
            }
            "OpRangeDefault" => {
                let [v, lo, hi, dflt] = args else {
                    return Err("OpRangeDefault with an unexpected operand list".into());
                };
                let lo = Self::literal(lo, "range bound")?;
                let hi = Self::literal(hi, "range bound")?;
                let dflt = Self::literal(dflt, "range default")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!(
                        "loft_range_default({}, {}, {}, {})",
                        as_int(v)?,
                        c_long(lo),
                        c_long(hi),
                        c_long(dflt)
                    ),
                    K::Int,
                ))
            }
            "OpMinSingleInt" | "OpMinSingleLong" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                Ok(E::new(format!("loft_neg({})", as_int(a)?), K::Int))
            }
            "OpConvBoolFromInt" | "OpConvBoolFromLong" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                Ok(E::new(format!("({} != LOFT_NULL)", as_int(a)?), K::Cond))
            }
            "OpConvIntFromNull" | "OpConvLongFromNull" => Ok(E::new("LOFT_NULL", K::Int)),
            "OpConvEnumFromNull" => Ok(E::new("(uint8_t) 255", K::Enum)),
            "OpConstEnum" => self.expr(args.first().ok_or("OpConstEnum without a value")?, ind),
            "OpConvIntFromEnum" => {
                let a = self.one(name, args, ind)?;
                let a = as_enum(a)?;
                Ok(E::new(
                    format!("({a} == 255 ? LOFT_NULL : (int64_t) {a})"),
                    K::Int,
                ))
            }
            "OpCastEnumFromInt" => {
                let a = self.one(name, args, ind)?;
                let a = as_int(a)?;
                Ok(E::new(
                    format!("({a} == LOFT_NULL ? (uint8_t) 255 : (uint8_t) {a})"),
                    K::Enum,
                ))
            }
            "OpConvBoolFromEnum" => {
                let a = self.one(name, args, ind)?;
                let a = as_enum(a)?;
                Ok(E::new(format!("({a} != 255 && {a} != 0)"), K::Cond))
            }
            "OpCastTextFromEnum" => {
                let [v, tp] = args else {
                    return Err("OpCastTextFromEnum with an unexpected operand list".into());
                };
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type out of range".to_string())?;
                let names = self.cx.enum_names(tp);
                let v = self.expr(v, ind)?;
                let t = self.temp(CType::Enum);
                self.line(ind, &format!("{t} = {};", as_enum(v)?));
                Ok(E::new(
                    format!(
                        "({t} == 0 || {t} == 255 ? loft_null_text() : loft_lit({names}[{t}], strlen({names}[{t}])))"
                    ),
                    K::View,
                ))
            }
            "OpCastVectorFromText" | "OpCastEnumFromText" => self.cast_from_text(name, args, ind),
            _ => self.float_operator(name, args, ind),
        }
    }

    /// A value read from a text: a type's whole record (`Stores::record_from_text`) or a plain
    /// enum's value by its name (`Stores::to_enum`).
    fn cast_from_text(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match name {
            // `Stores::record_from_text`: a new store holding the type the text describes.
            "OpCastVectorFromText" => {
                let [v, tp] = args else {
                    return Err("OpCastVectorFromText with an unexpected operand list".into());
                };
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type out of range".to_string())?;
                self.cx.parse_type(tp)?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("loft_record_from_text(loft_ptypes, {}, {tp})", as_view(v)?),
                    K::Ref,
                ))
            }
            // `Stores::to_enum`: the value whose name the text is, 0 for none.
            "OpCastEnumFromText" => {
                let [v, tp] = args else {
                    return Err("OpCastEnumFromText with an unexpected operand list".into());
                };
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type out of range".to_string())?;
                let names = self.cx.enum_names(tp);
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!(
                        "loft_enum_of_text({names}, sizeof {names} / sizeof *{names}, {})",
                        as_view(v)?
                    ),
                    K::Enum,
                ))
            }
            _ => Err(format!("operator {name}")),
        }
    }

    /// The float and single operators: `Float` names a double, `Single` a float.
    fn float_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        /// `(operator, single?, C spelling)` for the binary arithmetic.
        const INFIX: &[(&str, bool, &str)] = &[
            ("OpAddFloat", false, "+"),
            ("OpMinFloat", false, "-"),
            ("OpMulFloat", false, "*"),
            ("OpDivFloat", false, "/"),
            ("OpDivFloatNullable", false, "/"),
            ("OpAddSingle", true, "+"),
            ("OpMinSingle", true, "-"),
            ("OpMulSingle", true, "*"),
            ("OpDivSingle", true, "/"),
            ("OpDivSingleNullable", true, "/"),
        ];
        /// `(operator, single?, C function)` for the calls of two operands.
        const CALL2: &[(&str, bool, &str)] = &[
            ("OpRemFloat", false, "fmod"),
            ("OpRemFloatNullable", false, "fmod"),
            ("OpPowFloat", false, "pow"),
            ("OpRemSingle", true, "fmodf"),
            ("OpRemSingleNullable", true, "fmodf"),
            ("OpPowSingle", true, "powf"),
        ];
        /// `(operator, single?, C comparison, negated?)`.
        const CMP: &[(&str, bool, &str, bool)] = &[
            ("OpEqFloat", false, "loft_feq", false),
            ("OpNeFloat", false, "loft_feq", true),
            ("OpLtFloat", false, "loft_flt", false),
            ("OpLeFloat", false, "loft_fle", false),
            ("OpEqSingle", true, "loft_feq", false),
            ("OpNeSingle", true, "loft_feq", true),
            ("OpLtSingle", true, "loft_flt", false),
            ("OpLeSingle", true, "loft_fle", false),
        ];
        let shape = |single: bool| -> (K, fn(E) -> Result<String, String>) {
            if single {
                (K::Single, as_single)
            } else {
                (K::Float, as_float)
            }
        };
        if let Some((_, single, op)) = INFIX.iter().find(|(n, _, _)| *n == name) {
            let (k, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(format!("({} {op} {})", conv(a)?, conv(b)?), k));
        }
        if let Some((_, single, f)) = CALL2.iter().find(|(n, _, _)| *n == name) {
            let (k, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(format!("{f}({}, {})", conv(a)?, conv(b)?), k));
        }
        if let Some((_, single, f, neg)) = CMP.iter().find(|(n, _, _, _)| *n == name) {
            let (_, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            let neg = if *neg { "!" } else { "" };
            return Ok(E::new(
                format!("{neg}{f}({}, {})", conv(a)?, conv(b)?),
                K::Cond,
            ));
        }
        let unary = match name {
            "OpMinSingleFloat" => Some((false, "-")),
            "OpMinSingleSingle" => Some((true, "-")),
            "OpAbsFloat" => Some((false, "fabs")),
            "OpAbsSingle" => Some((true, "fabsf")),
            _ => None,
        };
        if let Some((single, f)) = unary {
            let (k, conv) = shape(single);
            let a = self.one(name, args, ind)?;
            return Ok(E::new(format!("({f}({}))", conv(a)?), k));
        }
        self.float_conversion(name, args, ind)
    }

    /// The conversions to and from floats, their math functions and their fields.
    fn float_conversion(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpConvFloatFromNull", []) => Ok(E::new("(double) NAN", K::Float)),
            ("OpConvSingleFromNull", []) => Ok(E::new("(float) NAN", K::Single)),
            ("OpConvFloatFromInt" | "OpConvFloatFromLong", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_i2f({})", as_int(a)?), K::Float))
            }
            ("OpConvSingleFromInt", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_i2s({})", as_int(a)?), K::Single))
            }
            ("OpCastIntFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_f2i({})", as_float(a)?), K::Int))
            }
            ("OpCastIntFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(
                    format!("loft_f2i((double) {})", as_single(a)?),
                    K::Int,
                ))
            }
            ("OpConvBoolFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("(!isnan({}))", as_float(a)?), K::Cond))
            }
            ("OpConvBoolFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("(!isnan({}))", as_single(a)?), K::Cond))
            }
            ("OpCastSingleFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("((float) {})", as_float(a)?), K::Single))
            }
            ("OpConvFloatFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("((double) {})", as_single(a)?), K::Float))
            }
            // The constants as the shortest literals that round-trip to Rust's own: `M_PI` and
            // `M_E` are POSIX, and `-std=c99` does not declare them.
            ("OpMathPiFloat", []) => Ok(E::new(format!("{:?}", std::f64::consts::PI), K::Float)),
            ("OpMathEFloat", []) => Ok(E::new(format!("{:?}", std::f64::consts::E), K::Float)),
            ("OpMathFuncFloat" | "OpMathFuncSingle", [id, v]) => {
                let id = Self::literal(id, "math function")?;
                let a = self.expr(v, ind)?;
                Ok(if name.ends_with("Single") {
                    E::new(format!("loft_math1s({id}, {})", as_single(a)?), K::Single)
                } else {
                    E::new(format!("loft_math1({id}, {})", as_float(a)?), K::Float)
                })
            }
            ("OpMathFunc2Float" | "OpMathFunc2Single", [id, a, b]) => {
                let id = Self::literal(id, "math function")?;
                let (a, b) = self.two(name, &[(*a).clone(), (*b).clone()], ind)?;
                Ok(if name.ends_with("Single") {
                    E::new(
                        format!("loft_math2s({id}, {}, {})", as_single(a)?, as_single(b)?),
                        K::Single,
                    )
                } else {
                    E::new(
                        format!("loft_math2({id}, {}, {})", as_float(a)?, as_float(b)?),
                        K::Float,
                    )
                })
            }
            ("OpFormatFloat" | "OpFormatSingle", [dest, val, width, prec, token, plus, dir]) => {
                let dest = self.text_dest(dest)?;
                let (Value::Int(token), Value::Boolean(plus)) = (token.unspan(), plus.unspan())
                else {
                    return Err(format!("{name} with a computed format"));
                };
                let es = self.args(
                    &[
                        (*val).clone(),
                        (*width).clone(),
                        (*prec).clone(),
                        (*dir).clone(),
                    ],
                    ind,
                )?;
                let [val, width, prec, dir] =
                    <[E; 4]>::try_from(es).map_err(|_| "format operands".to_string())?;
                let (val, single) = if name == "OpFormatSingle" {
                    (format!("(double) {}", as_single(val)?), 1)
                } else {
                    (as_float(val)?, 0)
                };
                Ok(E::new(
                    format!(
                        "loft_format_float(&{dest}, {val}, {}, {}, {token}, {}, (int) {}, {single})",
                        as_int(width)?,
                        as_int(prec)?,
                        u8::from(*plus),
                        as_int(dir)?
                    ),
                    K::Void,
                ))
            }
            _ => self.store_operator(name, args, ind),
        }
    }

    /// An integer literal operand of an operator: a field offset, a size, a type number.
    fn literal(v: &Value, what: &str) -> Result<i64, String> {
        match v.unspan() {
            Value::Int(i) => Ok(i64::from(*i)),
            Value::Long(l) => Ok(*l),
            _ => Err(format!("a computed {what}")),
        }
    }

    /// The operators over stores: records and their fields, vectors.
    /// The frees of a store, or `None` for another operator.
    fn free_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let e = match (name, args) {
            // A function variable's scope-exit free reads its definition number as a
            // reference and releases nothing: its closure record is freed through its own.
            ("OpFreeRef", [Value::Var(v)]) if self.shapes[*v as usize] == CType::Fn => {
                Ok(E::new("(void) 0", K::Void))
            }
            // A variable's free resets it to the sentinel, as native's does: the next pass's
            // pre-build free of it is then a no-op instead of a release of a reused slot.
            ("OpFreeRef", [Value::Var(v)]) if self.shapes[*v as usize] == CType::Ref => {
                Ok(E::new(format!("loft_free_var(&{})", self.var(*v)), K::Void))
            }
            ("OpFreeRef", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("loft_free_ref({})", as_ref(r)?), K::Void))
            }
            ("OpFreeRefIfDistinct", [Value::Var(v), w])
                if self.shapes[*v as usize] == CType::Ref =>
            {
                let w = self.expr(w, ind)?;
                Ok(E::new(
                    format!("loft_free_var_distinct(&{}, {})", self.var(*v), as_ref(w)?),
                    K::Void,
                ))
            }
            // `OpFreeRefOrHandUp` (@PLN150): the placeholder is freed unless the lambda hands
            // its store back — then the caller decides, as for any value a call through a
            // function value returns (`loft_fnref_minted`), so nothing is released here.
            ("OpFreeRefIfDistinct" | "OpFreeRefOrHandUp", [_, _]) => {
                let (p, w) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("loft_free_displaced({}, {})", as_ref(p)?, as_ref(w)?),
                    K::Void,
                ))
            }
            _ => return Ok(None),
        };
        e.map(Some)
    }

    fn store_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        if let Some(e) = self.free_operator(name, args, ind)? {
            return Ok(e);
        }
        match (name, args) {
            // `@FR-R-RetAdopt` — the adopted local's witness is never minted, so neither is
            // its store nor the zero its vector field would have been given.
            ("OpDatabase" | "OpSetInt4", [Value::Var(w), ..])
                if self.adopt.is_some_and(|a| a.vdb == *w) =>
            {
                Ok(E::new("((void) 0)", K::Void))
            }
            ("OpDatabase", [Value::Var(v), tp]) => {
                let tp = Self::literal(tp, "type")?;
                let tp = u16::try_from(tp).map_err(|_| "a type number out of range".to_string())?;
                let size = self.cx.stores.enum_parent_size(tp);
                self.cx.describe_type(tp);
                let var = self.var(*v);
                if self.cx.stores.is_vector_wrapper(tp) {
                    return Ok(E::new(
                        format!("loft_database_wrapper(&{var}, {size}, {tp})"),
                        K::Void,
                    ));
                }
                Ok(E::new(
                    format!("loft_database_typed(&{var}, {size}, {tp})"),
                    K::Void,
                ))
            }
            ("OpRefIsNull", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("({}.store_nr == 0xFFFF)", as_ref(r)?),
                    K::Cond,
                ))
            }
            ("OpNullRefSentinel", []) => Ok(E::new("loft_ref_null()", K::Ref)),
            ("OpConvBoolFromRef", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("({}.rec != 0)", as_ref(r)?), K::Cond))
            }
            ("OpRefAlias", [r]) => self.expr(r, ind),
            ("OpGetInt", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_int({}, {fld})", as_ref(r)?),
                    K::Int,
                ))
            }
            ("OpSetInt" | "OpSetInt4", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                let f = if name == "OpSetInt" {
                    "loft_set_int"
                } else {
                    "loft_set_int4"
                };
                Ok(E::new(
                    format!("{f}({}, {fld}, {})", as_ref(r)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpCopyRecord", [src, dst, tp]) => {
                let raw = Self::literal(tp, "type")?;
                let raw =
                    u16::try_from(raw).map_err(|_| "a type number out of range".to_string())?;
                let tp = self.copyable(raw & crate::keys::COPY_TP_MASK)?;
                let free = u8::from(raw & crate::keys::COPY_FREE_SOURCE != 0)
                    | (u8::from(self.cx.swap_safe(tp)) << 1)
                    | (u8::from(raw & crate::keys::COPY_FRESH_DEST != 0) << 2);
                let (src, dst) = self.two(name, &[(*src).clone(), (*dst).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_copy_record(loft_types, {tp}, {}, {}, {free})",
                        as_ref(src)?,
                        as_ref(dst)?
                    ),
                    K::Void,
                ))
            }
            ("OpMoveRecord" | "OpMoveField", [src, dst, tp]) => {
                self.move_op(name, src, dst, tp, ind)
            }
            ("OpGetField", [r, fld, _] | [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("loft_field({}, {fld})", as_ref(r)?), K::Ref))
            }
            _ => self.ref_operator(name, args, ind),
        }
    }

    /// `OpEvacuate(holder, fld, home, tp)` (`Stores::evacuate`, `@FR-H-RefHome`): the copy is
    /// placed in the holder's store as `OpPlaceRecord` places one.
    fn evacuate(
        &mut self,
        holder: &Value,
        fld: &Value,
        home: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let name = "OpEvacuate";
        let fld = Self::literal(fld, "field")?;
        let tp = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        let tp = self.copyable(tp)?;
        let size = self.cx.stores.enum_parent_size(tp);
        let (image, n) = self.cx.default_image(tp);
        // The copy is placed in the holder's store as `OpPlaceRecord` places one.
        let foreign = self
            .value_type(holder)
            .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
            .and_then(|kt| self.cx.stores.root_vector_element_of(kt))
            .is_some_and(|elem| elem != tp);
        let (holder, home) = self.two(name, &[holder.clone(), home.clone()], ind)?;
        Ok(E::new(
            format!(
                "loft_evacuate(loft_types, {tp}, {}, {fld}, {}, {size}, {image}, {n}, {})",
                as_ref(holder)?,
                as_ref(home)?,
                u8::from(foreign)
            ),
            K::Void,
        ))
    }

    /// `OpMoveRecord` (`Stores::move_record_out`, `@FR-R-MoveLast`) and `OpMoveField`: a record
    /// moved into the field `dst` names.
    fn move_op(
        &mut self,
        name: &str,
        src: &Value,
        dst: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let tp = Self::literal(tp, "type")?;
        let tp = u16::try_from(tp).map_err(|_| "a type number out of range".to_string())?;
        let tp = self.copyable(tp)?;
        let (src, dst) = self.two(name, &[src.clone(), dst.clone()], ind)?;
        let f = if name == "OpMoveRecord" {
            "loft_move_record_out"
        } else {
            "loft_move_field"
        };
        Ok(E::new(
            format!("{f}(loft_types, {tp}, {}, {})", as_ref(src)?, as_ref(dst)?),
            K::Void,
        ))
    }

    /// `OpAppendVector(v, other, tp)` (`Stores::vector_add`, a linked element type's records by
    /// `vector_add_array`) and `OpPrependVector` (`Stores::prepend_vector`), `tp` the element.
    fn append_vector_op(
        &mut self,
        name: &str,
        v: &Value,
        other: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let tp = u16::try_from(Self::literal(tp, "element type")?)
            .map_err(|_| "an element type out of range")?;
        let linked = self.cx.stores.is_linked(tp);
        self.cx.copy_type(tp)?;
        let (v, other) = self.two(name, &[v.clone(), other.clone()], ind)?;
        let (v, other) = (as_ref(v)?, as_ref(other)?);
        Ok(if name == "OpPrependVector" {
            E::new(
                format!(
                    "loft_vec_prepend(loft_types, {tp}, {v}, {other}, {})",
                    i32::from(linked)
                ),
                K::Bool,
            )
        } else if linked {
            E::new(
                format!("loft_vec_add_array(loft_types, {tp}, {v}, {other})"),
                K::Void,
            )
        } else {
            E::new(
                format!("loft_vec_add(loft_types, {tp}, {v}, {other})"),
                K::Void,
            )
        })
    }

    /// References as values: constants, whole references in fields, raw rebinds.
    fn ref_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpConstRef", [d]) => {
                let d = u32::try_from(Self::literal(d, "constant")?)
                    .map_err(|_| "a constant out of range")?;
                Ok(E::new(self.cx.const_ref(d)?, K::Ref))
            }
            ("OpGetDbRef", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_dbref({}, {fld})", as_ref(r)?),
                    K::Ref,
                ))
            }
            ("OpSetDbRef", [r, fld, x]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, x) = self.two(name, &[(*r).clone(), (*x).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_dbref({}, {fld}, {})", as_ref(r)?, as_ref(x)?),
                    K::Void,
                ))
            }
            // The closure record releases what it captured, running the captured values'
            // drop hooks (`State::drop_fn_ref`).
            ("OpDropFnRef", [Value::Var(f)]) if self.shapes[*f as usize] == CType::Fn => Ok(
                E::new(format!("loft_drop_closure({})", self.var(*f)), K::Void),
            ),
            ("OpPutRef", [Value::Var(v), x]) => {
                let x = self.expr(x, ind)?;
                Ok(E::new(
                    format!("{} = {}", self.var(*v), as_ref(x)?),
                    K::Void,
                ))
            }
            // A reference equality: two absent references are equal, an absent one equals no
            // present one, two present ones when they name the same place.
            ("OpEqContent" | "OpNeContent", [_, _, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.eq_type(tp)?;
                let (a, b) = self.two(name, &args[..2], ind)?;
                let neg = if name == "OpNeContent" { "!" } else { "" };
                Ok(E::new(
                    format!(
                        "{neg}loft_eq_content({}, {}, {})",
                        super::Cx::eq_fn(tp),
                        as_ref(a)?,
                        as_ref(b)?
                    ),
                    K::Cond,
                ))
            }
            ("OpPlaceRecord", [host, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                let size = self.cx.stores.enum_parent_size(tp);
                let (image, n) = self.cx.default_image(tp);
                // `@FR-H-RootExtent`: a record that is not an element of the host store's root
                // vector is noted, so a clear does not reset the store under it.  The host's
                // store is minted as the host value's type, so the Rust store answers here.
                let foreign = self
                    .value_type(host)
                    .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
                    .and_then(|kt| self.cx.stores.root_vector_element_of(kt))
                    .is_some_and(|elem| elem != tp);
                let host = self.expr(host, ind)?;
                Ok(E::new(
                    format!(
                        "loft_place_record({}, {tp}, {size}, {image}, {n}, {})",
                        as_ref(host)?,
                        u8::from(foreign)
                    ),
                    K::Ref,
                ))
            }
            ("OpEvacuate", [holder, fld, home, tp]) => self.evacuate(holder, fld, home, tp, ind),
            ("OpFreeRecordIn", [r, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.copy_type(tp)?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_free_record_in(loft_types, {tp}, {})", as_ref(r)?),
                    K::Void,
                ))
            }
            ("OpStoreLive", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("loft_store_live({})", as_ref(r)?), K::Cond))
            }
            ("OpDistinctStore", [_, _]) => {
                let (a, b) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("({}.store_nr != {}.store_nr)", as_ref(a)?, as_ref(b)?),
                    K::Cond,
                ))
            }
            _ => self.field_operator(name, args, ind),
        }
    }

    /// The enum and text fields of a record.
    fn field_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpGetEnum", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_enum({}, {fld})", as_ref(r)?),
                    K::Enum,
                ))
            }
            ("OpSetEnum", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_byte({}, {fld}, {})", as_ref(r)?, as_enum(val)?),
                    K::Void,
                ))
            }
            ("OpGetText", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(
                    ind,
                    &format!("{t} = loft_text_own(loft_get_text({}, {fld}));", as_ref(r)?),
                );
                Ok(self.held(t, CType::Text))
            }
            ("OpSetText" | "OpSetTextReplace", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                let f = if name == "OpSetText" {
                    "loft_set_text"
                } else {
                    "loft_refill_text"
                };
                Ok(E::new(
                    format!("{f}({}, {fld}, {})", as_ref(r)?, as_view(val)?),
                    K::Void,
                ))
            }
            ("OpGetFloat" | "OpGetSingle", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                let (f, k) = if name == "OpGetFloat" {
                    ("loft_get_float", K::Float)
                } else {
                    ("loft_get_single", K::Single)
                };
                Ok(E::new(format!("{f}({}, {fld})", as_ref(r)?), k))
            }
            ("OpSetFloat" | "OpSetSingle", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                let (f, v) = if name == "OpSetFloat" {
                    ("loft_set_float", as_float(val)?)
                } else {
                    ("loft_set_single", as_single(val)?)
                };
                Ok(E::new(format!("{f}({}, {fld}, {v})", as_ref(r)?), K::Void))
            }
            ("OpPushBoolean", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_byte({}, {})", as_ref(v)?, as_bool(val)?),
                    K::Void,
                ))
            }
            ("OpPushFloat" | "OpPushSingle", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                let (f, x) = if name == "OpPushFloat" {
                    ("loft_vec_push_float", as_float(val)?)
                } else {
                    ("loft_vec_push_single", as_single(val)?)
                };
                Ok(E::new(format!("{f}({}, {x})", as_ref(v)?), K::Void))
            }
            _ => self.narrow_operator(name, args, ind),
        }
    }

    /// The narrow integer fields: bytes, shorts and 4-byte integers, each in its encoding.
    fn narrow_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            (
                "OpGetByte" | "OpGetByteNullable" | "OpGetShort" | "OpGetShortRaw"
                | "OpGetShortFull",
                [r, fld, min],
            ) => {
                let fld = Self::literal(fld, "field")?;
                let min = Self::literal(min, "minimum")?;
                let f = match name {
                    "OpGetByte" => "loft_get_byte",
                    "OpGetByteNullable" => "loft_get_byte_nullable",
                    "OpGetShort" => "loft_get_short",
                    _ => "loft_get_short_raw",
                };
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("{f}({}, {fld}, {})", as_ref(r)?, c_long(min)),
                    K::Int,
                ))
            }
            (
                "OpSetByte" | "OpSetByteNullable" | "OpSetShort" | "OpSetShortRaw",
                [r, fld, min, val],
            ) => {
                let fld = Self::literal(fld, "field")?;
                let min = Self::literal(min, "minimum")?;
                let f = match name {
                    "OpSetByte" => "loft_set_byte_v",
                    "OpSetByteNullable" => "loft_set_byte_nullable",
                    "OpSetShort" => "loft_set_short",
                    _ => "loft_set_short_raw",
                };
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "{f}({}, {fld}, {}, {})",
                        as_ref(r)?,
                        c_long(min),
                        as_int(val)?
                    ),
                    K::Void,
                ))
            }
            ("OpGetInt4" | "OpGetInt4Raw" | "OpGetInt4Full", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                let f = match name {
                    "OpGetInt4" => "loft_get_int4",
                    "OpGetInt4Raw" => "loft_get_int4_raw",
                    _ => "loft_get_int4_full",
                };
                Ok(E::new(format!("{f}({}, {fld})", as_ref(r)?), K::Int))
            }
            ("OpSetInt4Raw", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_int4_raw({}, {fld}, {})", as_ref(r)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpPushInt4", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_int4({}, {})", as_ref(v)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpSizeVector", [v, stride]) => {
                let stride = Self::literal(stride, "stride")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("(loft_vec_len({}) * {stride})", as_ref(v)?),
                    K::Int,
                ))
            }
            ("OpFnRefClosure", [Value::Var(f)]) if self.shapes[*f as usize] == CType::Fn => {
                Ok(E::new(format!("{}.clos", self.var(*f)), K::Ref))
            }
            ("OpInitRefSentinel", [Value::Var(v)]) if self.shapes[*v as usize] == CType::Ref => Ok(
                E::new(format!("{} = loft_ref_null()", self.var(*v)), K::Void),
            ),
            _ => self.vector_operator(name, args, ind),
        }
    }

    /// The collection `OpNewRecord`/`OpFinishRecord` add to and its element type: the parent
    /// itself when `fld` is `u16::MAX`, else the parent's field.  A vector, any record
    /// collection `Cx::keyed` names, or a member of a linked group (`@FR-Col-Group`), whose
    /// record enters every member; any other collection is refused.
    pub(super) fn record_collection(
        &mut self,
        data: &Value,
        ptp: &Value,
        fld: &Value,
        ind: usize,
    ) -> Result<(String, u16, Coll), String> {
        use crate::database::Parts;
        let stores = self.cx.stores;
        let ptp = u16::try_from(Self::literal(ptp, "type")?)
            .map_err(|_| "a type out of range".to_string())?;
        let fld = u16::try_from(Self::literal(fld, "field")?)
            .map_err(|_| "a field out of range".to_string())?;
        let data = self.expr(data, ind)?;
        let data = as_ref(data)?;
        if fld == u16::MAX {
            if let Parts::Vector(c) = stores.types[ptp as usize].parts {
                return Ok((data, c, Coll::Vector));
            }
            let Some((_, c)) = self.cx.keyed(ptp) else {
                return Err("a new record in a collection other than a vector".into());
            };
            return Ok((data, c, Coll::Keyed(ptp)));
        }
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
            &stores.types[ptp as usize].parts
        else {
            return Err("a new record in a field of a non-struct".into());
        };
        let vtp = stores.field_type(ptp, fld);
        let pos = stores.field_position(ptp, fld);
        let others: Vec<u16> = fields
            .get(fld as usize)
            .map(|f| {
                f.other_indexes()
                    .iter()
                    .copied()
                    .filter(|m| *m != u16::MAX)
                    .collect()
            })
            .unwrap_or_default();
        if !others.is_empty() {
            // The parent is named twice — the primary's place and every sibling's — so it is
            // read once.
            let parent = self.temp(CType::Ref);
            self.line(ind, &format!("{parent} = {data};"));
            let mut members = vec![(pos, vtp)];
            for m in others {
                members.push((stores.field_position(ptp, m), stores.field_type(ptp, m)));
            }
            let Some((_, c)) = self.cx.keyed(vtp) else {
                return Err(
                    "a new record in a linked group member that is not a collection".into(),
                );
            };
            return Ok((
                format!("loft_field({parent}, {pos})"),
                c,
                Coll::Group(parent, members),
            ));
        }
        let place = format!("loft_field({data}, {pos})");
        if let Some(Parts::Vector(c)) = stores.types.get(vtp as usize).map(|t| &t.parts) {
            return Ok((place, *c, Coll::Vector));
        }
        let Some((_, c)) = self.cx.keyed(vtp) else {
            return Err("a new record in a field that is not a vector".into());
        };
        Ok((place, c, Coll::Keyed(vtp)))
    }

    /// The operators over vectors.
    fn vector_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpNewRecord", [data, ptp, fld]) => {
                let (v, elem, coll) = self.record_collection(data, ptp, fld, ind)?;
                match coll {
                    Coll::Keyed(tp) => return self.keyed_new(&v, tp, ind),
                    Coll::Group(_, members) => return self.keyed_new(&v, members[0].1, ind),
                    Coll::Vector => {}
                }
                let size = self.cx.stores.size(elem);
                let t = self.temp(CType::Ref);
                self.line(ind, &format!("{t} = loft_vec_append({v}, {size});"));
                let ptp = u16::try_from(Self::literal(ptp, "type")?)
                    .map_err(|_| "a type number out of range")?;
                if !self.mint_complete(ptp) {
                    let (image, n) = self.cx.default_image(elem);
                    self.line(ind, &format!("loft_fill({t}, {image}, {n});"));
                }
                Ok(E::new(t, K::Ref))
            }
            ("OpFinishRecord", [data, rec, ptp, fld]) => {
                let (v, _, coll) = self.record_collection(data, ptp, fld, ind)?;
                match coll {
                    Coll::Keyed(tp) => {
                        let rec = self.expr(rec, ind)?;
                        let rec = as_ref(rec)?;
                        return self.keyed_finish(&v, &rec, tp);
                    }
                    Coll::Group(parent, members) => {
                        let rec = self.expr(rec, ind)?;
                        let rec = as_ref(rec)?;
                        return self.group_finish(&parent, &rec, &members);
                    }
                    Coll::Vector => {}
                }
                Ok(E::new(format!("loft_vec_finish({v})"), K::Void))
            }
            ("OpClearVector", [v]) if self.in_delivery > 0 && self.adopted_buffer(v) => {
                Ok(E::new("((void) 0)", K::Void))
            }
            ("OpClearVector", [v]) => {
                self.cx.name_types();
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_clear_release(loft_types, loft_ntypes, {})",
                        as_ref(v)?
                    ),
                    K::Void,
                ))
            }
            ("OpLengthVector", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_len({})", as_ref(v)?), K::Int))
            }
            ("OpPreAllocVector", [v, count, size]) => {
                let count = Self::literal(count, "count")?;
                let size = Self::literal(size, "element size")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("loft_vec_prealloc({}, {count}, {size})", as_ref(v)?),
                    K::Void,
                ))
            }
            ("OpPushEnum", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_byte({}, {})", as_ref(v)?, as_enum(val)?),
                    K::Void,
                ))
            }
            ("OpPushInt", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_int({}, {})", as_ref(v)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpGetVector" | "OpGetVectorNullable", [v, size, idx]) => {
                let size = Self::literal(size, "element size")?;
                let (v, idx) = self.two(name, &[(*v).clone(), (*idx).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_get({}, {size}, {})", as_ref(v)?, as_int(idx)?),
                    K::Ref,
                ))
            }
            _ => self.collection_operator(name, args, ind),
        }
    }

    /// The character operators: a code point, 0 the null character.
    fn char_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpConvCharacterFromInt", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_char_of({})", as_int(v)?), K::Char))
            }
            ("OpConvIntFromCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("loft_char_int({})", as_char(c)?), K::Int))
            }
            ("OpConvBoolFromCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("({} != 0)", as_char(c)?), K::Cond))
            }
            ("OpLengthCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("loft_char_len({})", as_char(c)?), K::Int))
            }
            ("OpTextCharacter" | "OpTextCharacterNullable", [_, _]) => {
                let (t, i) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("loft_text_char({}, {})", as_view(t)?, as_int(i)?),
                    K::Char,
                ))
            }
            ("OpAppendCharacter" | "OpAppendStackCharacter", [dest, c]) => {
                let dest = self.text_dest(dest)?;
                let c = self.expr(c, ind)?;
                Ok(E::new(
                    format!("loft_push_code(&{dest}, {})", as_char(c)?),
                    K::Void,
                ))
            }
            ("OpGetCharacter", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_char({}, {fld})", as_ref(r)?),
                    K::Char,
                ))
            }
            ("OpSetCharacter", [r, fld, c]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, c) = self.two(name, &[(*r).clone(), (*c).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_char({}, {fld}, {})", as_ref(r)?, as_char(c)?),
                    K::Void,
                ))
            }
            ("OpPushCharacter", [v, c]) => {
                let (v, c) = self.two(name, &[(*v).clone(), (*c).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_char({}, {})", as_ref(v)?, as_char(c)?),
                    K::Void,
                ))
            }
            _ => self.misc_operator(name, args, ind),
        }
    }

    /// `r += src[lo..hi]` of scalar elements: `OpSliceVector` through `loft_vec_slice`, and
    /// `OpSliceView` through `loft_vec_slice_view`, which views a foreign source instead.
    fn vec_slice(&mut self, args: &[Value], ind: usize, func: &str) -> Result<E, String> {
        let [v, src, lo, hi, tp] = args else {
            return Err("a slice with an unexpected operand list".into());
        };
        let tp = u16::try_from(Self::literal(tp, "element type")?)
            .map_err(|_| "an element type out of range")?;
        // The parser emits these for scalar elements only; anything else is refused.
        let scalar = (tp <= 6 && tp != 5)
            || matches!(
                self.cx.stores.types.get(tp as usize).map(|t| &t.parts),
                Some(
                    crate::database::Parts::Byte(..)
                        | crate::database::Parts::Short(..)
                        | crate::database::Parts::ShortRaw(..)
                        | crate::database::Parts::Int(..)
                        | crate::database::Parts::IntRaw(..)
                )
            );
        if !scalar {
            return Err("a slice of elements that are not scalars".into());
        }
        let size = self.cx.stores.size(tp);
        let es = self.args(
            &[(*v).clone(), (*src).clone(), (*lo).clone(), (*hi).clone()],
            ind,
        )?;
        let [v, src, lo, hi] =
            <[E; 4]>::try_from(es).map_err(|_| "a slice without four operands".to_string())?;
        Ok(E::new(
            format!(
                "{func}({}, {}, {}, {}, {size})",
                as_ref(v)?,
                as_ref(src)?,
                as_int(lo)?,
                as_int(hi)?
            ),
            K::Void,
        ))
    }

    /// `OpRemoveVector` of a linked element type: the slot unlinked, the record released.
    fn remove_linked(&mut self, tp: u16, v: &Value, idx: &Value, ind: usize) -> Result<E, String> {
        self.cx.name_types();
        self.cx.describe_type(tp);
        let (v, idx) = self.two("OpRemoveVector", &[v.clone(), idx.clone()], ind)?;
        Ok(E::new(
            format!(
                "loft_vec_remove_linked(loft_types, {tp}, {}, {})",
                as_ref(v)?,
                as_int(idx)?
            ),
            K::Cond,
        ))
    }

    /// The whole-vector operators: append, replace, remove, repeat — each with the deep
    /// copies its store method makes.
    /// OpAdoptVector (`Stores::vector_adopt`): a local takes a native call's vector answer.
    fn adopt_vector(
        &mut self,
        v: &Value,
        other: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let name = "OpAdoptVector";
        let tp = u16::try_from(Self::literal(tp, "element type")?)
            .map_err(|_| "an element type out of range")?;
        if self.cx.stores.is_linked(tp) {
            return Err("an adopted vector of linked records".into());
        }
        self.cx.copy_type(tp)?;
        let (v, other) = self.two(name, &[v.clone(), other.clone()], ind)?;
        Ok(E::new(
            format!(
                "loft_vec_adopt(loft_types, {tp}, {}, {})",
                as_ref(v)?,
                as_ref(other)?
            ),
            K::Void,
        ))
    }

    fn collection_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpAdoptVector", [v, other, tp]) => self.adopt_vector(v, other, tp, ind),
            ("OpAppendVector", [v, ..]) if self.in_delivery > 0 && self.adopted_buffer(v) => {
                Ok(E::new("((void) 0)", K::Void))
            }
            ("OpPrependVector" | "OpAppendVector", [v, other, tp]) => {
                self.append_vector_op(name, v, other, tp, ind)
            }
            ("OpReplaceVector", [v, other, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                let f = if self.cx.stores.is_linked(tp) {
                    "loft_vec_replace_array"
                } else {
                    "loft_vec_replace"
                };
                self.cx.copy_type(tp)?;
                let (v, other) = self.two(name, &[(*v).clone(), (*other).clone()], ind)?;
                Ok(E::new(
                    format!("{f}(loft_types, {tp}, {}, {})", as_ref(v)?, as_ref(other)?),
                    K::Void,
                ))
            }
            ("OpSliceVector", [_, _, _, _, _]) => self.vec_slice(args, ind, "loft_vec_slice"),
            ("OpSliceView", [_, _, _, _, _]) => self.vec_slice(args, ind, "loft_vec_slice_view"),
            ("OpRemoveVector", [v, tp, idx]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                if self.cx.stores.is_linked(tp) {
                    return self.remove_linked(tp, v, idx, ind);
                }
                let size = self.cx.stores.size(tp);
                let (v, idx) = self.two(name, &[(*v).clone(), (*idx).clone()], ind)?;
                // An element owning heap releases it first, as `remove_vector_at` does.
                if self.cx.stores.type_owns_heap(tp) {
                    self.cx.name_types();
                    self.cx.describe_type(tp);
                    return Ok(E::new(
                        format!(
                            "loft_vec_remove_at(loft_types, {tp}, {}, {})",
                            as_ref(v)?,
                            as_int(idx)?
                        ),
                        K::Cond,
                    ));
                }
                Ok(E::new(
                    format!("loft_vec_remove({}, {size}, {})", as_ref(v)?, as_int(idx)?),
                    K::Cond,
                ))
            }
            ("OpAppendCopy", [v, count, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "vector type")?)
                    .map_err(|_| "a vector type out of range")?;
                let ctp = self.cx.stores.content(tp);
                if self.cx.stores.is_linked(ctp) {
                    return Err("a repeat of a linked element type".into());
                }
                self.cx.copy_type(ctp)?;
                let (v, count) = self.two(name, &[(*v).clone(), (*count).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_append_copy(loft_types, {ctp}, {}, {})",
                        as_ref(v)?,
                        as_int(count)?
                    ),
                    K::Void,
                ))
            }
            _ => self.vector_move_operator(name, args, ind),
        }
    }

    /// A local vector given up to a field, and a vector cut to a range.
    fn vector_move_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        match (name, args) {
            ("OpMoveVector", [dst, src, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                let f = if self.cx.stores.is_linked(tp) {
                    "loft_move_vector_array"
                } else {
                    "loft_move_vector"
                };
                self.cx.copy_type(tp)?;
                let (dst, src) = self.two(name, &[(*dst).clone(), (*src).clone()], ind)?;
                Ok(E::new(
                    format!("{f}(loft_types, {tp}, {}, {})", as_ref(dst)?, as_ref(src)?),
                    K::Void,
                ))
            }
            ("OpKeepVectorRange", [v, tp, lo, hi]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                // A linked element's slot is its record number, its record removed when dropped.
                let size = if self.cx.stores.is_linked(tp) {
                    0
                } else {
                    self.cx.stores.size(tp)
                };
                let es = self.args(&[(*v).clone(), (*lo).clone(), (*hi).clone()], ind)?;
                let [v, lo, hi] = <[E; 3]>::try_from(es)
                    .map_err(|_| "a range without three operands".to_string())?;
                Ok(E::new(
                    format!(
                        "loft_keep_vector_range({}, {size}, {}, {})",
                        as_ref(v)?,
                        as_int(lo)?,
                        as_int(hi)?
                    ),
                    K::Void,
                ))
            }
            _ => self.slot_operator(name, args, ind),
        }
    }

    /// Booleans and bytes in records and vectors, absent vectors, store identity, text
    /// clears and the text null.
    /// `OpClear(r, tp)` (`Stores::remove_claims`): the value's claims released, or with
    /// `@FR-R-Reuse`'s reset flag its pooled store reset where `reset_pool_root` allows.
    fn clear_op(&mut self, r: &Value, tp: &Value, ind: usize) -> Result<E, String> {
        let raw = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        let tp = raw & crate::keys::POOL_TP_MASK;
        self.cx.copy_type(tp)?;
        let r = self.expr(r, ind)?;
        let r = as_ref(r)?;
        let walk = format!("loft_remove_claims(loft_types, {tp}, {r})");
        if raw & crate::keys::POOL_RESET == 0 {
            return Ok(E::new(walk, K::Void));
        }
        let size = self.cx.stores.enum_parent_size(tp);
        let (image, n) = self.cx.default_image(tp);
        Ok(E::new(
            format!("if (!loft_reset_pool_root({r}, {tp}, {size}, {image}, {n})) {walk}"),
            K::Void,
        ))
    }

    fn slot_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpGetBoolean", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_bool({}, {fld})", as_ref(r)?),
                    K::Bool,
                ))
            }
            ("OpSetBoolean", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_byte({}, {fld}, {})", as_ref(r)?, as_bool(val)?),
                    K::Void,
                ))
            }
            ("OpPushByte", [v, min, val]) => {
                let min = Self::literal(min, "byte minimum")?;
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_push_byte({}, loft_byte_raw({min}, {}))",
                        as_ref(v)?,
                        as_int(val)?
                    ),
                    K::Void,
                ))
            }
            ("OpVectorIsNull", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_absent({})", as_ref(v)?), K::Cond))
            }
            ("OpLengthSorted", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_len({})", as_ref(v)?), K::Int))
            }
            ("OpEqRef" | "OpNeRef", [_, _]) => {
                let (a, b) = self.two(name, args, ind)?;
                let neg = if name == "OpNeRef" { "!" } else { "" };
                Ok(E::new(
                    format!("{neg}loft_ref_eq({}, {})", as_ref(a)?, as_ref(b)?),
                    K::Cond,
                ))
            }
            ("OpClear", [r, tp]) => self.clear_op(r, tp, ind),
            ("OpClearText", [dest]) => {
                let dest = self.text_dest(dest)?;
                Ok(E::new(format!("{dest}.len = 0"), K::Void))
            }
            ("OpConvTextFromNull", []) => Ok(E::new("loft_null_text()", K::View)),
            ("OpConvBoolFromNull", []) => Ok(E::new("LOFT_BOOL_NULL", K::Bool)),
            ("OpCastIntFromText" | "OpCastLongFromText", [t]) => {
                let t = self.expr(t, ind)?;
                Ok(E::new(format!("loft_int_of_text({})", as_view(t)?), K::Int))
            }
            ("OpGetTextSub", [_, _, _]) => {
                let es = self.args(args, ind)?;
                let [t, from, till] = <[E; 3]>::try_from(es)
                    .map_err(|_| "OpGetTextSub without three operands".to_string())?;
                Ok(E::new(
                    format!(
                        "loft_text_sub({}, {}, {})",
                        as_view(t)?,
                        as_int(from)?,
                        as_int(till)?
                    ),
                    K::View,
                ))
            }
            ("OpConvCharacterFromNull", []) => Ok(E::new("((uint32_t) 0)", K::Char)),
            _ if name.contains("Character") => self.char_operator(name, args, ind),
            ("OpCastTextFromBool", [b]) => {
                let b = self.expr(b, ind)?;
                Ok(E::new(format!("loft_bool_text({})", as_bool(b)?), K::View))
            }
            ("OpConvBoolFromText", [t]) => {
                let t = self.expr(t, ind)?;
                Ok(E::new(
                    format!("(!loft_str_is_null({}))", as_view(t)?),
                    K::Cond,
                ))
            }
            // The chain ends in `misc_operator`, which holds the last operators and the refusal.
            _ => self.misc_operator(name, args, ind),
        }
    }
}

/// `a op b` for two integers or booleans.  A variable compared with itself has a fixed answer
/// (a null equals itself, as in loft), and C compilers reject the tautology under `-Werror`.
fn compare(a: &str, op: &str, b: &str) -> E {
    if a == b && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        let same = matches!(op, "==" | "<=" | ">=");
        return E::new(if same { "1" } else { "0" }, K::Cond);
    }
    E::new(format!("({a} {op} {b})"), K::Cond)
}

/// `n` as a C `int64_t` constant: the null as `LOFT_NULL`, since `INT64_C` cannot spell it.
fn c_long(n: i64) -> String {
    if n == i64::MIN {
        "LOFT_NULL".into()
    } else {
        format!("INT64_C({n})")
    }
}

fn as_int(e: E) -> Result<String, String> {
    match e.kind {
        K::Int => Ok(e.code),
        // The slot the bytecode writes is the integer's: a character's code point, a
        // boolean's byte, a function value's definition number (`FnRefDnr`).
        K::Char | K::Bool => Ok(format!("((int64_t) ({}))", e.code)),
        K::Fn => Ok(format!("(({}).d)", e.code)),
        k => Err(format!("an integer expected, a {k:?} found")),
    }
}

fn as_cond(e: E) -> Result<String, String> {
    match e.kind {
        K::Cond => Ok(e.code),
        K::Bool => Ok(format!("({} == 1)", e.code)),
        k => Err(format!("a condition expected, a {k:?} found")),
    }
}

fn as_bool(e: E) -> Result<String, String> {
    match e.kind {
        K::Bool => Ok(e.code),
        K::Cond => Ok(format!("(uint8_t) ({})", bare(&e.code))),
        k => Err(format!("a boolean expected, a {k:?} found")),
    }
}

fn as_ref(e: E) -> Result<String, String> {
    match e.kind {
        K::Ref => Ok(e.code),
        k => Err(format!("a reference expected, a {k:?} found")),
    }
}

fn as_tuple(e: E, id: u32) -> Result<String, String> {
    match e.kind {
        K::Tup(i) if i == id => Ok(e.code),
        k => Err(format!("a tuple expected, a {k:?} found")),
    }
}

fn as_enum(e: E) -> Result<String, String> {
    match e.kind {
        K::Enum => Ok(e.code),
        k => Err(format!("an enum value expected, a {k:?} found")),
    }
}

fn as_float(e: E) -> Result<String, String> {
    match e.kind {
        K::Float => Ok(e.code),
        k => Err(format!("a float expected, a {k:?} found")),
    }
}

fn as_single(e: E) -> Result<String, String> {
    match e.kind {
        K::Single => Ok(e.code),
        k => Err(format!("a single expected, a {k:?} found")),
    }
}

fn as_char(e: E) -> Result<String, String> {
    match e.kind {
        K::Char => Ok(e.code),
        // An integer written as a character: its low 32 bits, as the store keeps them (an
        // integer null is the character null, 0).
        K::Int => Ok(format!("((uint32_t) ({}))", e.code)),
        k => Err(format!("a character expected, a {k:?} found")),
    }
}

fn as_fn(e: E) -> Result<String, String> {
    match e.kind {
        K::Fn => Ok(e.code),
        k => Err(format!("a function value expected, a {k:?} found")),
    }
}

fn as_view(e: E) -> Result<String, String> {
    match e.kind {
        K::View => Ok(e.code),
        k => Err(format!("a text expected, a {k:?} found")),
    }
}

/// The value of `e` converted to shape `c`, for a parameter or a store.
fn as_shape(e: E, c: CType) -> Result<String, String> {
    match c {
        CType::Int => as_int(e),
        CType::Bool => as_bool(e),
        CType::Text => as_view(e),
        CType::Ref => as_ref(e),
        CType::Tuple(id) => as_tuple(e, id),
        CType::Enum => as_enum(e),
        CType::Float => as_float(e),
        CType::Single => as_single(e),
        CType::Char => as_char(e),
        CType::Fn => as_fn(e),
        CType::Void => Err("a void value".into()),
    }
}

/// The operators of each C shape, for the operators this backend writes.
const OP_INT: &[&str] = &[
    "OpAddInt",
    "OpAddLong",
    "OpMinInt",
    "OpMinLong",
    "OpMulInt",
    "OpMulLong",
    "OpDivInt",
    "OpDivLong",
    "OpRemInt",
    "OpRemLong",
    "OpGetInt",
    "OpLengthVector",
    "OpLengthText",
    "OpSizeText",
    "OpMinSingleInt",
    "OpBitNotSingleInt",
    "OpSizeStruct",
    "OpSizeHash",
    "OpMinSingleLong",
    "OpConvIntFromNull",
    "OpConvLongFromNull",
    "OpConvIntFromEnum",
    "OpLengthSorted",
    "OpRangeDefault",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetShort",
    "OpGetShortRaw",
    "OpGetShortFull",
    "OpGetInt4",
    "OpLandInt",
    "OpLandLong",
    "OpLorInt",
    "OpLorLong",
    "OpEorInt",
    "OpEorLong",
    "OpSLeftInt",
    "OpSLeftLong",
    "OpSRightInt",
    "OpSRightLong",
    "OpConvIntFromCharacter",
    "OpLengthCharacter",
    "OpAddIntNullable",
    "OpMinIntNullable",
    "OpMulIntNullable",
    "OpDivIntNullable",
    "OpRemIntNullable",
    "OpCastIntFromText",
    "OpCastLongFromText",
];
const OP_BOOL: &[&str] = &[
    "OpEqInt",
    "OpEqLong",
    "OpNeInt",
    "OpNeLong",
    "OpLtInt",
    "OpLtLong",
    "OpLeInt",
    "OpLeLong",
    "OpEqBool",
    "OpNeBool",
    "OpNot",
    "OpEqText",
    "OpNeText",
    "OpRefIsNull",
    "OpConvBoolFromInt",
    "OpConvBoolFromLong",
    "OpConvBoolFromEnum",
    "OpConvBoolFromRef",
    "OpGetBoolean",
    "OpVectorIsNull",
    "OpDistinctStore",
    "OpConvBoolFromText",
    "OpConvBoolFromCharacter",
    "OpConvBoolFromNull",
    "OpRemoveVector",
    "OpStoreLive",
    "OpEqContent",
    "OpNeContent",
    "OpEqRef",
    "OpNeRef",
];
const OP_REF: &[&str] = &[
    "OpPlaceRecord",
    "OpConstRef",
    "OpGetDbRef",
    "OpGetField",
    "OpGetVector",
    "OpGetVectorNullable",
    "OpNullRefSentinel",
    "OpInsertVector",
    "OpRefFromChildRec",
    "OpChildRec",
    "OpCoroutineRetain",
];
const OP_ENUM: &[&str] = &[
    "OpGetEnum",
    "OpCastEnumFromInt",
    "OpConvEnumFromNull",
    "OpConstEnum",
];
const OP_TEXT: &[&str] = &[
    "OpGetText",
    "OpCastTextFromEnum",
    "OpCastEnumFromText",
    "OpCastVectorFromText",
    "OpConvTextFromNull",
    "OpCastTextFromBool",
    "OpGetTextSub",
];
const OP_FLOAT: &[&str] = &[
    "OpAddFloat",
    "OpMinFloat",
    "OpMulFloat",
    "OpDivFloat",
    "OpDivFloatNullable",
    "OpRemFloat",
    "OpRemFloatNullable",
    "OpPowFloat",
    "OpMinSingleFloat",
    "OpAbsFloat",
    "OpConvFloatFromNull",
    "OpConvFloatFromInt",
    "OpConvFloatFromLong",
    "OpConvFloatFromSingle",
    "OpMathPiFloat",
    "OpMathEFloat",
    "OpMathFuncFloat",
    "OpMathFunc2Float",
    "OpGetFloat",
];
const OP_SINGLE: &[&str] = &[
    "OpAddSingle",
    "OpMinSingle",
    "OpMulSingle",
    "OpDivSingle",
    "OpDivSingleNullable",
    "OpRemSingle",
    "OpRemSingleNullable",
    "OpPowSingle",
    "OpMinSingleSingle",
    "OpAbsSingle",
    "OpConvSingleFromNull",
    "OpConvSingleFromInt",
    "OpCastSingleFromFloat",
    "OpMathFuncSingle",
    "OpMathFunc2Single",
    "OpGetSingle",
];
const OP_FCMP: &[&str] = &[
    "OpEqFloat",
    "OpNeFloat",
    "OpLtFloat",
    "OpLeFloat",
    "OpEqSingle",
    "OpNeSingle",
    "OpLtSingle",
    "OpLeSingle",
    "OpConvBoolFromFloat",
    "OpConvBoolFromSingle",
];
const OP_FINT: &[&str] = &["OpCastIntFromFloat", "OpCastIntFromSingle"];
const OP_CHAR: &[&str] = &[
    "OpConvCharacterFromInt",
    "OpConvCharacterFromNull",
    "OpTextCharacter",
    "OpTextCharacterNullable",
    "OpGetCharacter",
];

/// The C shape an operator's value is held in, for the operators this backend writes.
fn op_shape(name: &str) -> Option<CType> {
    let shapes = [
        (OP_INT, CType::Int),
        (OP_BOOL, CType::Bool),
        (OP_REF, CType::Ref),
        (OP_ENUM, CType::Enum),
        (OP_TEXT, CType::Text),
        (OP_FLOAT, CType::Float),
        (OP_SINGLE, CType::Single),
        (OP_FCMP, CType::Bool),
        (OP_FINT, CType::Int),
        (OP_CHAR, CType::Char),
    ];
    shapes
        .iter()
        .find(|(names, _)| names.contains(&name))
        .map(|(_, c)| *c)
}

/// `s` as a C string literal: printable ASCII as itself, every other byte as an octal escape
/// (three digits, so the next character can never extend it), `?` escaped against trigraphs.
pub(crate) fn c_string(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'?' => out.push_str("\\?"),
            0x20..=0x7e => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push('"');
    out
}

/// `c` without one pair of outer parentheses that enclose all of it: `if ((a == b))` is a
/// warning in every C compiler that reads it as a mistyped assignment.
fn bare(c: &str) -> &str {
    let Some(inner) = c.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return c;
    };
    let mut depth = 0i32;
    for ch in inner.chars() {
        depth += match ch {
            '(' => 1,
            ')' => -1,
            _ => 0,
        };
        if depth < 0 {
            return c;
        }
    }
    if depth == 0 { inner } else { c }
}

fn unsupported(v: &Value) -> String {
    let kind = format!("{v:?}");
    let kind = kind.split(['(', ' ', '{']).next().unwrap_or("value");
    format!("the construct {kind}")
}

/// A kind a store link carries: the place's own get and set read and write it
/// (`FnWriter::link_access`).
fn link_kind(tp: &crate::data::Type) -> bool {
    use crate::data::Type;
    matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Character
            | Type::Single
            | Type::Float
            | Type::Boolean
            | Type::Enum(_, false, _)
            | Type::Text(_)
    )
}

/// The `&` locals of `def` that link a store place: every bind (`link_set_repoints`) names an
/// element or a field, or another store link (`OpVarRef`), none a variable (`OpCreateStack`),
/// and the kind has a field operator.  A link to a link follows it, so the answer is the
/// fixpoint over the binds.
fn store_links(cx: &Cx, def: &crate::data::Definition, seed: &[u16]) -> Vec<bool> {
    use crate::data::Type;
    let vars = def.variables();
    let n = vars.count() as usize;
    let args = vars.arguments();
    let mut binds: Vec<Vec<&Value>> = vec![Vec::new(); n];
    def.code().walk(&mut |node| {
        if let Value::Set(v, x) = node
            && !args.contains(v)
            && matches!(vars.tp(*v).base(), Type::RefVar(_))
            && crate::scopes::link_set_repoints(cx.data, vars, *v, x)
        {
            binds[*v as usize].push(x);
        }
    });
    let kind_ok = |v: usize| {
        let Type::RefVar(tp) = vars.tp(v as u16).base() else {
            return false;
        };
        link_kind(tp)
    };
    let mut store = vec![false; n];
    for &a in seed {
        store[a as usize] = true;
    }
    loop {
        let mut changed = false;
        for v in 0..n {
            if store[v] || binds[v].is_empty() || !kind_ok(v) {
                continue;
            }
            let all_places = binds[v].iter().all(|x| match x.unspan() {
                Value::Call(d, xs) => match cx.data.def(*d).name() {
                    "OpCreateStack" => false,
                    "OpVarRef" => matches!(xs.as_slice(), [Value::Var(a)] if store[*a as usize]),
                    _ => true,
                },
                _ => true,
            });
            if all_places {
                store[v] = true;
                changed = true;
            }
        }
        if !changed {
            return store;
        }
    }
}

/// Does `v`'s single-dep chain reach one of `bufs`?  As native's `grows_buffer` follows a
/// mover's root.
fn chain_reaches(vars: &crate::variables::Function, v: u16, bufs: &[u16]) -> bool {
    let mut cur = v;
    for _ in 0..vars.var_count() {
        if bufs.contains(&cur) {
            return true;
        }
        match vars.tp(cur).depend().as_slice() {
            [d] if *d != cur => cur = *d,
            _ => return false,
        }
    }
    false
}

/// The member of `params` that `v`'s dep chain ends at, through single-dep views (sixteen
/// steps at most), as native's `param_root`.
fn param_root(vars: &crate::variables::Function, v: u16, params: &[u16]) -> Option<u16> {
    let mut cur = v;
    for _ in 0..16 {
        if params.contains(&cur) {
            return Some(cur);
        }
        let [d] = vars.tp(cur).depend()[..] else {
            return None;
        };
        cur = d;
    }
    None
}
