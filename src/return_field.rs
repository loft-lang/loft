// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `(R-ReturnField)` — the returned FIELD of an owned local is answered as the local's own
//! store at the field's position.  Enforces `@FR-R-ReturnField`.
//!
//! The decoder shape: `d = decode(bytes); return d.value` (or `v = d.value; return v`).  The
//! parser materialises such an exit — the return buffer is minted, the field deep-copied into
//! it, the local freed (`materialized_view_return`) — because a view of a frame local would
//! dangle once the frame's free ran.  When the local is the frame's OWN store and this exit is
//! the last thing that names it, the copy buys nothing: the store is handed to the caller as
//! it stands, the handle pointing at the field, and the caller releases it whole exactly as it
//! releases any adopted store (a store's free is keyed on the store, never on the handle's
//! type or position).
//!
//! Admission, per exit block: the block is the parser's `materialized_view_return` over the
//! function's own return buffer; the copy's source is a field path rooted at a LOCAL that is
//! not a parameter, not a hidden buffer, and owned (dep-empty) — or a local VIEW of such a root
//! (`v = d.value`); the returned record type owns heap (a scalar-only record is the native
//! value form's to answer, and its copy is a few words); the block frees the root; and every
//! other statement between the copy and the return is a store free or a text free.  The
//! rewrite drops the buffer's mint, the copy and the root's own free, returns the source, and
//! re-witnesses every other store free against the ROOT — a buffer that aliases the handed
//! store (a pooled `__ref_N` the root adopted, r12; an alias `q = p`, r5) is then skipped, and
//! one that does not is freed as before.  Declines keep the copy: any other statement in the
//! block, a root that is a parameter or a view, a source that is not a field path, a
//! return buffer that is not the function's own.  `LOFT_NO_RETURN_FIELD=1` is the switch;
//! `LOFT_TRACE_PLACE=1` names each admission and decline.  Both backends: decided in the
//! scope pass on the settled IR, after the exit vector it may compose with.
use crate::data::{Block, Data, Deps, Type, Value};
use crate::database::Stores;
use crate::variables::Function;

/// The block a rewritten exit becomes: a hand-over, which no reader of the materialised
/// block's shape (the native value form, the copy census) should take for a copy.
pub const HANDOVER: &str = "return_field_handover";

struct Ops {
    database: u32,
    copy: u32,
    get_field: u32,
    free_ref: u32,
    free_if_distinct: u32,
    free_text: u32,
    is_null: u32,
}

impl Ops {
    fn lookup(data: &Data) -> Option<Self> {
        let nr = |n: &str| {
            let d = data.def_nr(n);
            (d != u32::MAX).then_some(d)
        };
        Some(Self {
            database: nr("OpDatabase")?,
            copy: nr("OpCopyRecord")?,
            get_field: nr("OpGetField")?,
            free_ref: nr("OpFreeRef")?,
            free_if_distinct: nr("OpFreeRefIfDistinct")?,
            free_text: nr("OpFreeText")?,
            is_null: nr("OpRefIsNull")?,
        })
    }
}

pub fn rewrite(data: &mut Data, database: &Stores, d_nr: u32) {
    if !crate::keys::return_field_enabled() {
        return;
    }
    let Some(ops) = Ops::lookup(data) else {
        return;
    };
    let def = data.def(d_nr);
    if !def.is_loft_defined() {
        return;
    }
    let Some(rb) = own_return_buffer(data, d_nr) else {
        return;
    };
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let def = data.def(d_nr);
    let mut cx = Cx {
        data,
        database,
        function: def.variables(),
        ops: &ops,
        rb,
        name: def.name(),
        fired: 0,
    };
    cx.walk(&mut code);
    let fired = cx.fired;
    data.definitions[d_nr as usize].code = code;
    if fired > 0 {
        crate::rewrite_census::fired("R-ReturnField", fired);
    }
}

fn own_return_buffer(data: &Data, d_nr: u32) -> Option<u16> {
    let def = data.def(d_nr);
    let idx = def.hidden_return_buffer_attr()?;
    let name = &def.attributes().get(idx)?.name;
    let v = def.variables().var(name);
    (v != u16::MAX && def.variables().is_argument(v)).then_some(v)
}

struct Cx<'a> {
    data: &'a Data,
    database: &'a Stores,
    function: &'a Function,
    ops: &'a Ops,
    rb: u16,
    name: &'a str,
    fired: usize,
}

/// What the copy's source is: the field path's root local, and the source expression the
/// exit will answer instead of the buffer.
struct Source {
    root: u16,
}

impl Cx<'_> {
    fn walk(&mut self, node: &mut Value) {
        if let Value::Block(bl) = node.unspan_mut()
            && bl.name == "materialized_view_return"
        {
            match self.admit(bl) {
                Ok(src) => {
                    self.fired += 1;
                    if crate::keys::trace_place() {
                        eprintln!(
                            "[return-field] fn={} root={}: ADMITTED",
                            self.name,
                            self.function.name(src.root)
                        );
                    }
                    self.apply(bl, &src);
                    return;
                }
                Err(why) => {
                    if crate::keys::trace_place() {
                        eprintln!("[return-field] fn={}: DECLINED — {why}", self.name);
                    }
                }
            }
        }
        node.for_each_child_mut(&mut |c| self.walk(c));
    }

    /// The root local of a field path — `OpGetField(… OpGetField(Var(root), _, _) …, _, _)`.
    fn path_root(&self, v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::Call(d, args) if *d == self.ops.get_field && args.len() == 3 => {
                match args[0].unspan() {
                    Value::Var(root) => Some(*root),
                    inner => self.path_root(inner),
                }
            }
            _ => None,
        }
    }

    /// The source's root: a field path's root, or a local view whose ONE dep is such a root.
    fn source_root(&self, src: &Value) -> Result<u16, &'static str> {
        if let Some(root) = self.path_root(src) {
            return Ok(root);
        }
        let Value::Var(v) = src.unspan() else {
            return Err("the copy's source is not a field path or a local");
        };
        if self.function.is_argument(*v) {
            return Err("the source is a parameter");
        }
        let deps = self.function.tp(*v).depend();
        if deps.len() != 1 || deps[0] >= self.function.count() {
            return Err("the source local does not view one local");
        }
        Ok(deps[0])
    }

    fn owned_local(&self, root: u16) -> Result<(), &'static str> {
        if self.function.is_argument(root) {
            return Err("the root is a parameter");
        }
        if self.function.is_caller_hidden_buf(root) || root == self.rb {
            return Err("the root is a hidden buffer");
        }
        if self.function.is_skip_free(root) {
            return Err("the root is never freed here");
        }
        // `@FR-N-Shape` — read the nullability marker rather than fall through a missing arm.
        let (shape, nullable) = self.function.tp(root).peel_optional();
        if nullable {
            return Err("the root is nullable");
        }
        match shape {
            Type::Reference(_, deps) | Type::Enum(_, true, deps) if deps.is_empty() => Ok(()),
            _ => Err("the root is not an owned record local"),
        }
    }

    fn free_subject(&self, op: &Value) -> Option<u16> {
        match op.unspan() {
            Value::Call(d, args)
                if (*d == self.ops.free_ref || *d == self.ops.free_if_distinct)
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(_))) =>
            {
                match args[0].unspan() {
                    Value::Var(x) => Some(*x),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// A statement that may stand between the buffer's mint and the copy: a bind (a lifted
    /// root's, r10), or the lazy mint-or-release of the HIDDEN buffer a lifted call is handed
    /// (`@FR-O-LazyBuffer`, `if OpRefIsNull(b) { mint b } else { release b }`).  Neither names
    /// the function's own buffer or reads the root, so the copy's source and the frees after it
    /// are what they are without them; the natural spelling `return f(x).k` puts the guard here,
    /// where `d = f(x); return d.k` leaves it outside the block.
    fn before_copy(&self, op: &Value) -> bool {
        match op.unspan() {
            Value::Set(_, _) => true,
            Value::If(cond, _, _) => matches!(cond.unspan(), Value::Call(d, args)
                if *d == self.ops.is_null
                    && matches!(args.as_slice(), [b] if matches!(b.unspan(), Value::Var(b)
                        if *b != self.rb && self.function.is_caller_hidden_buf(*b)))),
            _ => false,
        }
    }

    fn admit(&self, bl: &Block) -> Result<Source, &'static str> {
        let ops = bl.operators.as_slice();
        let n = ops.len();
        if n < 4 {
            return Err("the block is too short");
        }
        // The buffer's mint first.
        match ops[0].unspan() {
            Value::Call(d, args)
                if *d == self.ops.database
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if *b == self.rb) =>
                {}
            _ => return Err("the block does not mint the function's own buffer first"),
        }
        // `return <buffer>` last.
        match ops[n - 1].unspan() {
            Value::Return(r) if matches!(r.unspan(), Value::Var(b) if *b == self.rb) => {}
            _ => return Err("the block does not return the buffer last"),
        }
        // The copy: the first op after the mint that is not a bind or a lifted call's buffer
        // guard ([`Self::before_copy`]).
        let mut i = 1;
        while i < n - 1 && self.before_copy(&ops[i]) {
            i += 1;
        }
        let (src, tp) = match ops[i].unspan() {
            Value::Call(d, args)
                if *d == self.ops.copy
                    && args.len() == 3
                    && matches!(args[1].unspan(), Value::Var(b) if *b == self.rb) =>
            {
                match args[2].unspan() {
                    Value::Int(tp) => (&args[0], *tp),
                    _ => return Err("the copy's type is not a literal"),
                }
            }
            _ => return Err("no copy into the buffer follows the mint"),
        };
        let root = self.source_root(src)?;
        self.owned_local(root)?;
        let known = u16::try_from(tp).map_err(|_| "the copy's type is out of range")?;
        if !self.database.owns_heap(known) {
            return Err("the returned record owns no heap");
        }
        // Everything between the copy and the return is a store free or a text free, and one
        // of the store frees releases the root.
        let mut frees_root = false;
        for op in &ops[i + 1..n - 1] {
            match op.unspan() {
                Value::Call(d, args)
                    if *d == self.ops.free_text
                        && args.len() == 1
                        && matches!(args[0].unspan(), Value::Var(_)) => {}
                Value::Call(d, args)
                    if *d == self.ops.free_if_distinct
                        && args.len() == 2
                        && matches!(args[0].unspan(), Value::Var(_))
                        && matches!(args[1].unspan(), Value::Var(_)) => {}
                Value::Call(d, args)
                    if *d == self.ops.free_ref
                        && args.len() == 1
                        && matches!(args[0].unspan(), Value::Var(_)) => {}
                _ => return Err("a statement between the copy and the return is not a free"),
            }
            if self.free_subject(op) == Some(root) {
                frees_root = true;
            }
        }
        if !frees_root {
            return Err("the block does not free the root");
        }
        // The source must be read before any free runs, which the order above guarantees,
        // and the root must not be named by a bind before the copy other than as its own.
        Ok(Source { root })
    }

    fn apply(&self, bl: &mut Block, src: &Source) {
        let root = src.root;
        let mut ops = std::mem::take(&mut bl.operators);
        let n = ops.len();
        let mut out = Vec::with_capacity(n);
        let mut source: Option<Value> = None;
        for (k, op) in ops.drain(..).enumerate() {
            if k == 0 || k == n - 1 {
                continue; // the mint and `return <buffer>`
            }
            if source.is_none() {
                match op.unspan() {
                    _ if self.before_copy(&op) => {
                        out.push(op);
                        continue;
                    }
                    Value::Call(d, _) if *d == self.ops.copy => {
                        let Value::Call(_, mut args) = op.unspan().clone() else {
                            unreachable!("matched a call");
                        };
                        source = Some(args.swap_remove(0));
                        continue;
                    }
                    _ => unreachable!("admission checked the shape"),
                }
            }
            // A free after the copy.
            let mut op = op;
            match op.unspan_mut() {
                Value::Call(d, args) if *d == self.ops.free_text => out.push(op),
                Value::Call(d, args) if *d == self.ops.free_if_distinct => {
                    if matches!(args[0].unspan(), Value::Var(x) if *x == root) {
                        continue; // the root's own free: moved out
                    }
                    args[1] = Value::Var(root);
                    out.push(op);
                }
                Value::Call(d, args) if *d == self.ops.free_ref => {
                    if matches!(args[0].unspan(), Value::Var(x) if *x == root) {
                        continue;
                    }
                    *d = self.ops.free_if_distinct;
                    args.push(Value::Var(root));
                    out.push(op);
                }
                _ => unreachable!("admission checked the shape"),
            }
        }
        out.push(Value::Return(Box::new(
            source.expect("admission found the copy"),
        )));
        bl.operators = out;
        bl.name = HANDOVER;
        // The block now yields a handle into the root's store, not a view of the buffer.
        // A nullable result never reaches here (a `-> S?` return has no buffer to admit);
        // the marker is read all the same (`@FR-N-Shape`).
        bl.result = match bl.result.peel_optional() {
            (Type::Reference(d, _), false) => Type::Reference(*d, Deps::none()),
            (Type::Enum(d, true, _), false) => Type::Enum(*d, true, Deps::none()),
            _ => bl.result.clone(),
        };
        let _ = self.data;
    }
}
