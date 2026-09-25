// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-WorkBuffer` — a vector local that never leaves its frame is the CALLER's buffer.
//!
//! `fn f(…) { v: vector<integer> = []; …; len(v) }` minted a store at the declaration and
//! freed it at the exit on every call: 67 ns of bookkeeping around a value Rust keeps on the
//! stack, and ~45 % of the cbor bench.  Where every mention of `v` — and of a local bound as
//! a copy of it, which is how `for x in v` reads it — is an operand of a vector operator that
//! reads or writes the vector IN PLACE, `v` becomes a hidden `vector<τ>` parameter marked a
//! work buffer: every call site supplies it as the per-site work-ref a return buffer already
//! takes (`add_defaults`' vector arm; minted once per activation by `(O-LazyBuffer)`, freed
//! at the caller's exit), and the callee CLEARS it where the declaration stood.  A callee
//! handed the null sentinel takes the rebound-parameter road at that same site — the shape
//! the parser emits for `if c { v = [] } else { v.clear() }` on a by-value vector parameter:
//! an entry witness, a store of its own, released at exit against the witness — so no route
//! that builds a frame by hand owes the callee a buffer.
//!
//! Decided after pass 2 on the settled IR, beside the targeted `__tret` promotion whose
//! caller patching (`patch_tret_call`) it shares, so a forward- and a backward-referenced
//! caller are patched alike.  The mention test IS the escape proof: with a scalar element,
//! every admitted operand position yields a scalar or nothing, so no view, copy or link of
//! the store can leave the frame.  The fallback of every walk here is a DECLINE, which costs
//! the program the mint it already pays and never a wrong answer — an operator or a node this
//! file does not name keeps the local a local.
//!
//! `LOFT_NO_WORK_BUFFER=1` keeps every local; `LOFT_TRACE_WORK_BUFFER=1` names each local
//! promoted and each candidate declined with the reason; `LOFT_WORK_BUFFER_NULL=1` leaves
//! every caller-side buffer null, the positive control for the null road (`scopes`).
use std::collections::{HashMap, HashSet};

use super::Parser;
use crate::data::{Data, DefType, Deps, Type, Value, is_scalar, v_if, v_set};
use crate::variables::Function;

impl Parser {
    /// Phase A over every definition — each admitted local becomes a hidden work-buffer
    /// parameter and its declaration the null-or-clear site — then Phase B over every caller
    /// of a promoted definition, appending the buffers its calls do not carry yet.  Runs
    /// after the targeted `__tret` promotion, whose `a == v` renumbering settles the argument
    /// order Phase A reads, and before the store-text instances clone their function's final
    /// signature.  Idempotent: a promoted local is an argument and never a candidate again,
    /// and a call already carrying its buffers is left alone.
    pub(crate) fn promote_work_buffers(&mut self) {
        if !crate::keys::work_buffer_enabled() {
            return;
        }
        let addr_taken = address_taken_defs(&self.data);
        let n_defs = self.data.definitions.len() as u32;
        for d in 0..n_defs {
            if self.work_buffer_def_decline(d, &addr_taken).is_some() {
                continue;
            }
            if self.promote_work_buffers_in(d) > 0 {
                self.work_buffer_promoted.insert(d);
            }
        }
        if !self.work_buffer_promoted.is_empty() {
            self.patch_work_buffer_callers();
        }
    }

    /// Why definition `d` takes no work buffer at all, or `None` when its locals may.
    fn work_buffer_def_decline(&self, d: u32, addr_taken: &HashSet<u32>) -> Option<&'static str> {
        let def = self.data.def(d);
        if def.def_type() != DefType::Function {
            return Some("not a function");
        }
        if !def.rust().is_empty() {
            return Some("a native body");
        }
        if def.instance_of != u32::MAX {
            return Some("a generic instance");
        }
        if def.synthetic().is_some() {
            return Some("a synthetic definition");
        }
        // An entry point is called by a harness (`main`, the test runners, the native test
        // `main`) that hands it no buffer, so it has no caller to take one from.
        if def.name() == "n_main" || def.is_corpus_entry_point() {
            return Some("an entry point");
        }
        if def.name().starts_with("n___lambda_") {
            return Some("a lambda");
        }
        if addr_taken.contains(&d) {
            return Some("its address is taken");
        }
        if !matches!(def.code().unspan(), Value::Block(_)) {
            return Some("no body");
        }
        if def
            .code()
            .any_node(&mut |v| matches!(v, Value::Yield(..) | Value::Parallel(..)))
        {
            return Some("a body that suspends or forks");
        }
        None
    }

    /// Phase A for one definition: the number of locals promoted.
    fn promote_work_buffers_in(&mut self, d: u32) -> usize {
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d as usize].variables,
        );
        self.context = d;
        // A re-entry into a parsed function: the pooled counters were reset when its table
        // was stored, so the witness minted below would otherwise REUSE (and retype) a
        // `__ref_N` the parse already gave to something live (`sync_work_counters`).
        self.vars.sync_work_counters();
        let mut code = std::mem::replace(&mut self.data.definitions[d as usize].code, Value::Null);
        let mut promoted = 0;
        let trace = crate::keys::trace_work_buffer();
        let ops = Ops::new(&self.data);
        for v in 0..self.vars.count() {
            let Some(db) = self.work_buffer_candidate(v) else {
                continue;
            };
            // A local written and never read is a dead store the post-scope lint reports;
            // promoted, it would be an argument the lint skips, and the lost write would
            // go unreported.  Nothing reads it, so the buffer would gain nothing either.
            let (reads, writes) =
                crate::use_analysis::dead_store_accesses(&code, &self.vars, &self.data)[v as usize];
            if reads == 0 && writes > 0 {
                if trace {
                    eprintln!(
                        "[work-buffer] fn={} local={} keeps its store: a dead store (written, never read)",
                        self.data.def(d).name(),
                        self.vars.name(v)
                    );
                }
                continue;
            }
            match admit(&code, &self.vars, &self.data, &ops, v, db) {
                Ok(()) => {
                    let Some((v, db)) = self.move_after_arguments(&mut code, v, db) else {
                        if trace {
                            eprintln!(
                                "[work-buffer] fn={} local={} keeps its store: numbered before an argument that is the table's last variable",
                                self.data.def(d).name(),
                                self.vars.name(v)
                            );
                        }
                        continue;
                    };
                    self.promote_one(&mut code, v, db, &ops);
                    promoted += 1;
                    if trace {
                        eprintln!(
                            "[work-buffer] fn={} local={} PROMOTED to a caller buffer",
                            self.data.def(d).name(),
                            self.vars.name(v)
                        );
                    }
                }
                Err(why) => {
                    if trace {
                        eprintln!(
                            "[work-buffer] fn={} local={} keeps its store: {why}",
                            self.data.def(d).name(),
                            self.vars.name(v)
                        );
                    }
                }
            }
        }
        self.data.definitions[d as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d as usize].variables,
        );
        self.context = saved_ctx;
        promoted
    }

    /// The shape test on the variable table alone: a plain local `v: vector<τ> = []` with a
    /// scalar τ, viewing one `__vdb_N` backing, numbered after every argument.  Answers the
    /// backing's number.
    fn work_buffer_candidate(&self, v: u16) -> Option<u16> {
        let vars = &self.vars;
        if vars.is_argument(v) || vars.is_caller_hidden_buf(v) || vars.is_work_ref(v) {
            return None;
        }
        // A compiler temporary is `_`-prefixed; a user local that is too gives the mint up.
        if vars.name(v).starts_with('_') {
            return None;
        }
        // A nullable vector local keeps its mint: null is a value the work buffer, which is
        // cleared where the declaration stood, cannot hold (the nullability question, asked
        // on its own — @FR-N-Shape).
        if vars.tp(v).peel_optional().1 {
            return None;
        }
        let Type::Vector(elem, _) = vars.tp(v).base() else {
            return None;
        };
        let decline = |why: &str| {
            if crate::keys::trace_work_buffer() {
                eprintln!(
                    "[work-buffer] fn={} local={} keeps its store: {why}",
                    self.data.def(self.context).name(),
                    vars.name(v)
                );
            }
            None
        };
        if !is_scalar(elem) {
            return decline("its element is a record or a text");
        }
        let [db] = vars.tp(v).depend()[..] else {
            return decline("not a view of one backing");
        };
        if !vars.name(db).starts_with("__vdb_") {
            return decline("its backing is not a `__vdb_`");
        }
        Some(db)
    }

    /// `check_argument_geometry`: argument slots are laid out in variable order and a call
    /// site pushes attribute order, so the new last attribute needs the last argument
    /// number.  A local numbered before an argument — the NRVO result local is numbered
    /// where its tail stood, after the scratch declared above it — takes the table's last
    /// number and the variable there takes its old one, the dance `av_renumber_retbuf` does
    /// for a promoted text return: the IR, every type's frame deps and the table's own
    /// index-keyed sets move together.  Answers the local's and its backing's new numbers;
    /// `None` when the last variable is itself an argument, whose order the swap would move.
    fn move_after_arguments(&mut self, code: &mut Value, v: u16, db: u16) -> Option<(u16, u16)> {
        if !self.vars.arguments().iter().any(|&a| a > v) {
            return Some((v, db));
        }
        let last = self.vars.count() - 1;
        if last == v || self.vars.is_argument(last) {
            return None;
        }
        if crate::keys::trace_work_buffer() {
            eprintln!(
                "[work-buffer] fn={} local={} ({v}) takes the number of `{}` ({last})",
                self.data.def(self.context).name(),
                self.vars.name(v),
                self.vars.name(last)
            );
        }
        let tmp = self.vars.count();
        Self::renumber_frame_var(code, last, tmp);
        Self::renumber_frame_var(code, v, last);
        Self::renumber_frame_var(code, tmp, v);
        self.vars.renumber_frame_in_types(last, tmp);
        self.vars.renumber_frame_in_types(v, last);
        self.vars.renumber_frame_in_types(tmp, v);
        self.vars.swap_variables(v, last);
        Some((last, if db == last { v } else { db }))
    }

    /// The promotion of one admitted local: the hidden attribute, the argument, the entry
    /// witness, and the declaration's trio replaced by the null-or-clear site.
    fn promote_one(&mut self, code: &mut Value, v: u16, db: u16, ops: &Ops) {
        let d = self.context;
        let name = self.vars.name(v).to_string();
        let Type::Vector(elem, _) = self.vars.tp(v).base().clone() else {
            return;
        };
        // #306 — the attribute's type carries no deps: the local's dep names a callee frame
        // variable, and inherited it would read as a caller variable number.
        let buf_tp = Type::Vector(elem, Deps::none());
        let a = self.data.add_attribute(&mut self.lexer, d, &name, buf_tp);
        let attr = &mut self.data.definitions[d as usize].attributes[a];
        attr.hidden = true;
        attr.work_buffer = true;
        self.vars.become_argument(v);
        // The rebound-parameter road for a null: the witness snapshots what the caller
        // handed at entry, and the scan's exit free declines on it (`ensure_rebind_witness`
        // records the pairing it reads).  The backing takes the marks the rebind path gives
        // its own (a sentinel null-init, no free of its own: the parameter's guarded free
        // releases the store it holds).
        let w = self.ensure_rebind_witness(v);
        self.vars.set_skip_free(db);
        self.vars.mark_inline_ref(db);
        let is_null = self.cl("OpRefIsNull", &[Value::Var(v)]);
        let free = self.cl("OpFreeRefIfDistinct", &[Value::Var(v), Value::Var(w)]);
        let clear = Value::Call(ops.clear, vec![Value::Var(v)]);
        let replaced = replace_trio(code, v, db, ops, |mut trio| {
            trio.insert(0, free);
            v_if(is_null, Value::Insert(trio), Value::Insert(vec![clear]))
        });
        debug_assert!(replaced, "the admitted trio was not found again");
        // The entry stash, as `finish_body` writes it for a rebound parameter: the witness's
        // null-init first (a sentinel: the witness is inline_ref), the raw copy after the
        // leading null-inits.
        if let Value::Block(bl) = code.unspan_mut() {
            let stash = self.cl("OpPutRef", &[Value::Var(w), Value::Var(v)]);
            bl.operators.insert(0, v_set(w, Value::Null));
            let at = bl
                .operators
                .iter()
                .position(|op| !matches!(op.unspan(), Value::Set(_, rhs) if matches!(rhs.unspan(), Value::Null)))
                .unwrap_or(bl.operators.len());
            bl.operators.insert(at, stash);
        }
    }

    /// Phase B: every call of a promoted definition takes the buffers its callee now
    /// declares — `add_defaults`' vector arm mints the per-site work-ref, exactly as it does
    /// for a return buffer — and each buffer minted here gets the top-level null-init the
    /// parser's preamble would have given it.
    fn patch_work_buffer_callers(&mut self) {
        let promoted = self.work_buffer_promoted.clone();
        for c in 0..self.data.definitions.len() as u32 {
            let calls_promoted = self.data.def(c).code.any_node(&mut |v| {
                matches!(v, Value::Call(d, args) if promoted.contains(d) && args.len() < self.data.attributes(*d))
            });
            if !calls_promoted {
                continue;
            }
            let saved_ctx = self.context;
            std::mem::swap(
                &mut self.vars,
                &mut self.data.definitions[c as usize].variables,
            );
            self.context = c;
            let before: HashSet<u16> = self.vars.work_ref_vars().into_iter().collect();
            // The pooling counters lag the names already in the table (`patch_tret_callers`
            // says why); every buffer minted below must be a fresh one.
            self.vars.sync_work_counters();
            let mut code =
                std::mem::replace(&mut self.data.definitions[c as usize].code, Value::Null);
            self.patch_tret_call(&mut code, &promoted);
            if let Value::Block(bl) = code.unspan_mut() {
                for vr in self.vars.work_ref_vars() {
                    if !before.contains(&vr) {
                        self.vars.mark_work_buffer_ref(vr);
                        bl.operators.insert(0, v_set(vr, Value::Null));
                    }
                }
            }
            self.data.definitions[c as usize].code = code;
            std::mem::swap(
                &mut self.vars,
                &mut self.data.definitions[c as usize].variables,
            );
            self.context = saved_ctx;
        }
    }
}

/// Every definition some `FnRef` names — promoting one changes the signature every function
/// value of it was typed against (the targeted `__tret` promotion defers them for the same
/// reason) — and every `par(…)` WORKER: the parallel builtins take the worker as a bare
/// definition number (their last operand), and the scalar-returning worker route builds the
/// frame from the element alone.
fn address_taken_defs(data: &Data) -> HashSet<u32> {
    let mut out = HashSet::new();
    for d in 0..data.definitions.len() as u32 {
        data.def(d).code.walk(&mut |v| match v {
            Value::FnRef(f, _, _) if *f >= 0 => {
                out.insert(*f as u32);
            }
            Value::Call(callee, args) if data.def(*callee).name().starts_with("n_parallel_") => {
                // The worker sits at the builtin's `func` parameter.
                let at = data
                    .def(*callee)
                    .attributes()
                    .iter()
                    .position(|a| a.name == "func");
                if let Some(Value::Int(f)) = at.and_then(|i| args.get(i)).map(Value::unspan)
                    && *f >= 0
                {
                    out.insert(*f as u32);
                }
            }
            _ => {}
        });
    }
    out
}

/// The operator numbers the trio and the clear are spelled with.
struct Ops {
    database: u32,
    get_field: u32,
    set_int4: u32,
    clear: u32,
}

impl Ops {
    fn new(data: &Data) -> Self {
        Ops {
            database: data.def_nr("OpDatabase"),
            get_field: data.def_nr("OpGetField"),
            set_int4: data.def_nr("OpSetInt4"),
            clear: data.def_nr("t_6vector_clear"),
        }
    }
}

fn is_var(v: &Value, x: u16) -> bool {
    matches!(v.unspan(), Value::Var(y) if *y == x)
}

/// The three statements `vector_db` lowers `v: vector<τ> = []` to, at `ops[i..i + 3]`:
/// the backing's mint, the local bound as a view of it, and the length written 0.
fn is_trio(stmts: &[Value], i: usize, v: u16, db: u16, ops: &Ops) -> bool {
    let Some(window) = stmts.get(i..i + 3) else {
        return false;
    };
    let mint = matches!(window[0].unspan(), Value::Call(d, args) if *d == ops.database && args.first().is_some_and(|a| is_var(a, db)));
    let bind = matches!(window[1].unspan(), Value::Set(x, rhs) if *x == v
        && matches!(rhs.unspan(), Value::Call(d, args) if *d == ops.get_field && args.first().is_some_and(|a| is_var(a, db))));
    let len = matches!(window[2].unspan(), Value::Call(d, args) if *d == ops.set_int4 && args.first().is_some_and(|a| is_var(a, db)));
    mint && bind && len
}

/// How many blocks of `node` hold the trio of `v`.
fn trio_sites(node: &Value, v: u16, db: u16, ops: &Ops) -> usize {
    let mut n = 0;
    node.walk(&mut |x| {
        if let Value::Block(bl) | Value::Loop(bl) = x {
            n += (0..bl.operators.len())
                .filter(|&i| is_trio(&bl.operators, i, v, db, ops))
                .count();
        }
    });
    n
}

/// Replace the (one) trio of `v` by `build(trio)`; true when it was found.
fn replace_trio(
    node: &mut Value,
    v: u16,
    db: u16,
    ops: &Ops,
    build: impl FnOnce(Vec<Value>) -> Value,
) -> bool {
    fn go<'b>(
        node: &mut Value,
        v: u16,
        db: u16,
        ops: &Ops,
        build: &mut Option<Box<dyn FnOnce(Vec<Value>) -> Value + 'b>>,
    ) -> bool {
        match node.unspan_mut() {
            Value::Block(bl) | Value::Loop(bl) => {
                if let Some(i) =
                    (0..bl.operators.len()).find(|&i| is_trio(&bl.operators, i, v, db, ops))
                {
                    let trio: Vec<Value> = bl.operators.drain(i..i + 3).collect();
                    let build = build.take().expect("one trio");
                    bl.operators.insert(i, build(trio));
                    return true;
                }
                bl.operators.iter_mut().any(|op| go(op, v, db, ops, build))
            }
            other => {
                let mut found = false;
                other.for_each_child_mut(&mut |c| {
                    if !found {
                        found = go(c, v, db, ops, build);
                    }
                });
                found
            }
        }
    }
    let mut build: Option<Box<dyn FnOnce(Vec<Value>) -> Value + '_>> = Some(Box::new(build));
    go(node, v, db, ops, &mut build)
}

/// Per variable, how many `Set`s target it.
fn set_counts(node: &Value) -> HashMap<u16, usize> {
    let mut out = HashMap::new();
    node.walk(&mut |x| {
        if let Value::Set(v, _) = x {
            *out.entry(*v).or_insert(0) += 1;
        }
    });
    out
}

/// How many `Var(x)` nodes `node` holds.
fn var_mentions(node: &Value, x: u16) -> usize {
    let mut n = 0;
    node.walk(&mut |y| {
        if is_var(y, x) {
            n += 1;
        }
    });
    n
}

/// The whole admission of `v`: one trio, a backing named nowhere else, and every mention of
/// `v` and of its aliases an admitted operand.
fn admit(
    code: &Value,
    function: &Function,
    data: &Data,
    ops: &Ops,
    v: u16,
    db: u16,
) -> Result<(), &'static str> {
    let Value::Block(bl) = code.unspan() else {
        return Err("no body");
    };
    if trio_sites(code, v, db, ops) != 1 {
        return Err("declared other than once as `[]`");
    }
    let sets = set_counts(code);
    if sets.get(&v).copied().unwrap_or(0) != 1 {
        if crate::keys::trace_work_buffer() {
            code.walk(&mut |x| {
                if let Value::Set(y, rhs) = x
                    && *y == v
                {
                    let shape = format!("{:?}", rhs.unspan());
                    eprintln!(
                        "[work-buffer]   `{}` = {}",
                        function.name(v),
                        shape.chars().take(100).collect::<String>()
                    );
                }
            });
        }
        return Err("assigned a second time");
    }
    // The backing: its top-level null-init and the trio's three operands, nothing else.
    let db_inits = bl
        .operators
        .iter()
        .filter(|op| matches!(op.unspan(), Value::Set(x, rhs) if *x == db && matches!(rhs.unspan(), Value::Null)))
        .count();
    if db_inits != 1 || sets.get(&db).copied().unwrap_or(0) != 1 || var_mentions(code, db) != 3 {
        return Err("its backing is named elsewhere");
    }
    let mut walk = Walk {
        data,
        function,
        sets: &sets,
        v,
        db,
        ops,
        tracked: HashSet::from([v]),
        aliases: Vec::new(),
    };
    // An alias found on one round is judged on the next; the set only grows.
    loop {
        walk.node(code, Pos::Other)?;
        let new: Vec<u16> = walk
            .aliases
            .drain(..)
            .filter(|a| !walk.tracked.contains(a))
            .collect();
        if new.is_empty() {
            return Ok(());
        }
        walk.tracked.extend(new);
    }
}

/// Where a node stands: an argument of a call (named) or anywhere else.
#[derive(Clone, Copy)]
enum Pos<'a> {
    Other,
    Arg { name: &'a str, d: u32, i: usize },
}

struct Walk<'a> {
    data: &'a Data,
    function: &'a Function,
    sets: &'a HashMap<u16, usize>,
    v: u16,
    db: u16,
    ops: &'a Ops,
    tracked: HashSet<u16>,
    aliases: Vec<u16>,
}

/// An operator that reads or writes its first operand's vector IN PLACE, or yields an
/// element PLACE of it (admitted only under a scalar accessor, see `is_element_place`).
fn in_place_at_0(name: &str) -> bool {
    name.starts_with("OpPush")
        || matches!(
            name,
            "OpPreAllocVector"
                | "OpReserveVector"
                | "OpLengthVector"
                | "OpSizeVector"
                | "OpVectorIsNull"
                | "OpClearVector"
                | "OpRemoveVector"
                | "OpKeepVectorRange"
                | "OpAppendTextBytes"
                | "OpAppendVector"
                | "OpReplaceVector"
                | "OpGetVector"
                | "OpGetVectorNullable"
                | "OpInsertVector"
                | "OpVectorRef"
                | "OpVectorRefNullable"
        )
}

/// An operator whose SECOND operand's elements are copied out of it.
fn copied_from_at_1(name: &str) -> bool {
    matches!(name, "OpAppendVector" | "OpReplaceVector")
}

/// An operator answering a place inside the vector, which only a scalar accessor may take.
fn is_element_place(name: &str) -> bool {
    matches!(
        name,
        "OpGetVector"
            | "OpGetVectorNullable"
            | "OpInsertVector"
            | "OpVectorRef"
            | "OpVectorRefNullable"
    )
}

/// A getter or setter of a scalar at a place: the place is consumed, never kept.
fn is_scalar_accessor(name: &str) -> bool {
    matches!(
        name,
        "OpGetInt"
            | "OpGetInt4"
            | "OpGetInt4Raw"
            | "OpGetInt4Full"
            | "OpGetCharacter"
            | "OpGetSingle"
            | "OpGetFloat"
            | "OpGetByte"
            | "OpGetByteNullable"
            | "OpGetEnum"
            | "OpGetBoolean"
            | "OpGetShort"
            | "OpGetShortRaw"
            | "OpGetShortSpare"
            | "OpGetShortFull"
            | "OpSetInt"
            | "OpSetInt4"
            | "OpSetInt4Raw"
            | "OpSetCharacter"
            | "OpSetSingle"
            | "OpSetFloat"
            | "OpSetByte"
            | "OpSetByteNullable"
            | "OpSetShort"
            | "OpSetShortRaw"
            | "OpSetEnum"
            | "OpSetBoolean"
    )
}

impl Walk<'_> {
    /// A stdlib one-op wrapper (`len(v)`, `v.clear()`) is its op — `hoist::one_op_wrapper`
    /// is the one home of that reading — provided the wrapper's FIRST parameter is the op's
    /// first operand and no other operand names it.
    fn one_op_wrapper(&self, d: u32) -> Option<u32> {
        use crate::generation::hoist::WrapperOperand;
        let (op, operands) = crate::generation::hoist::one_op_wrapper(self.data, d)?;
        let first = matches!(operands.first(), Some(WrapperOperand::Param(0)));
        let rest_free = !operands
            .iter()
            .skip(1)
            .any(|o| matches!(o, WrapperOperand::Param(0)));
        (first && rest_free).then_some(op)
    }

    /// May a tracked vector stand as operand `i` of `name`?
    fn allowed_operand(&self, name: &str, d: u32, i: usize) -> bool {
        if i == 0 && in_place_at_0(name) {
            return true;
        }
        if i == 1 && copied_from_at_1(name) {
            return true;
        }
        i == 0
            && self
                .one_op_wrapper(d)
                .is_some_and(|op| in_place_at_0(self.data.def(op).name()))
    }

    fn node(&mut self, node: &Value, pos: Pos<'_>) -> Result<(), &'static str> {
        match node.unspan() {
            Value::Var(x) if self.tracked.contains(x) => match pos {
                Pos::Arg { name, d, i } if self.allowed_operand(name, d, i) => Ok(()),
                Pos::Arg { name, i, .. } => {
                    if crate::keys::trace_work_buffer() {
                        eprintln!(
                            "[work-buffer]   `{}` is operand {i} of `{name}`",
                            self.function.name(*x)
                        );
                    }
                    Err("handed to a call")
                }
                Pos::Other => Err("mentioned outside a vector operator"),
            },
            Value::Call(d, args) => {
                let data = self.data;
                let name = data.def(*d).name();
                if is_element_place(name)
                    && args.first().is_some_and(
                        |a| matches!(a.unspan(), Value::Var(x) if self.tracked.contains(x)),
                    )
                    && !matches!(pos, Pos::Arg { name: p, i: 0, .. } if is_scalar_accessor(p))
                {
                    return Err("an element place not read or written as a scalar");
                }
                // Copied into a record's field: the element-first build (`@FR-R-ElemFirst`)
                // builds such a local inside the record, which saves the copy a work
                // buffer keeps, and a promoted local would decline it.
                if copied_from_at_1(name)
                    && args.get(1).is_some_and(
                        |a| matches!(a.unspan(), Value::Var(x) if self.tracked.contains(x)),
                    )
                    && args.first().is_some_and(
                        |a| matches!(a.unspan(), Value::Call(g, _) if *g == self.ops.get_field),
                    )
                {
                    return Err("copied into a record's field");
                }
                for (i, a) in args.iter().enumerate() {
                    self.node(a, Pos::Arg { name, d: *d, i })?;
                }
                Ok(())
            }
            Value::Set(x, rhs) => {
                if self.tracked.contains(x) {
                    // The admitted writes: the trio's bind of `v` to its backing, and an
                    // alias's one binding, met again on the round after it was admitted.
                    let trio_bind = *x == self.v
                        && matches!(rhs.unspan(), Value::Call(d, args) if *d == self.ops.get_field && args.first().is_some_and(|a| is_var(a, self.db)));
                    let alias_bind = *x != self.v
                        && matches!(rhs.unspan(), Value::Var(y) if self.tracked.contains(y));
                    return if trio_bind || alias_bind {
                        Ok(())
                    } else {
                        Err("assigned again")
                    };
                }
                if let Value::Var(y) = rhs.unspan()
                    && self.tracked.contains(y)
                {
                    // A copy into another local is an ALIAS, judged by the same rules on the
                    // next round — provided it is a plain local bound once.
                    if self.function.is_argument(*x) {
                        return Err("copied into a parameter");
                    }
                    if self.sets.get(x).copied().unwrap_or(0) != 1 {
                        return Err("copied into a local assigned more than once");
                    }
                    self.aliases.push(*x);
                    return Ok(());
                }
                self.node(rhs, Pos::Other)
            }
            Value::Iter(x, a, b, c) => {
                if self.tracked.contains(x) {
                    return Err("bound by a loop");
                }
                self.node(a, Pos::Other)?;
                self.node(b, Pos::Other)?;
                self.node(c, Pos::Other)
            }
            Value::TupleGet(base, _) => {
                if self.tracked.contains(base) {
                    Err("in a tuple")
                } else {
                    Ok(())
                }
            }
            Value::TuplePut(base, _, inner) => {
                if self.tracked.contains(base) {
                    return Err("in a tuple");
                }
                self.node(inner, Pos::Other)
            }
            Value::CallRef(f, xs) => {
                if self.tracked.contains(f) {
                    return Err("called as a function value");
                }
                for x in xs {
                    self.node(x, Pos::Other)?;
                }
                Ok(())
            }
            Value::FnRef(_, clos, _) => {
                if self.tracked.contains(clos) {
                    Err("captured by a closure")
                } else {
                    Ok(())
                }
            }
            Value::FnRefDnr(x) => {
                if self.tracked.contains(x) {
                    Err("read as a function reference")
                } else {
                    Ok(())
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    self.node(op, Pos::Other)?;
                }
                Ok(())
            }
            Value::Insert(xs) | Value::Tuple(xs) | Value::Parallel(xs) => {
                for x in xs {
                    self.node(x, Pos::Other)?;
                }
                Ok(())
            }
            Value::If(c, t, e) => {
                self.node(c, Pos::Other)?;
                self.node(t, Pos::Other)?;
                self.node(e, Pos::Other)
            }
            Value::Return(e) | Value::Drop(e) | Value::Yield(e) => self.node(e, Pos::Other),
            // A leaf, or a node that names no variable (`Keys` describes a keyed
            // collection's key, which a scalar vector never has): nothing here can reach
            // the store.  Every child-bearing variant is named above, so a variant this
            // arm meets carries no `Value` and no variable number.
            _ => Ok(()),
        }
    }
}
