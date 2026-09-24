// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A `&text` parameter handed a text FIELD or ELEMENT (@PLN167 decision 2, arc C3).
//!
//! A text link has two kinds.  The STACK kind names a text variable and is what every
//! `&text` parameter was built for.  The STORE kind names a string slot inside a record and
//! holds that slot's `DbRef`; each mention of it is spelled as the field it names,
//! `OpGetText(OpVarRef(t), 0)`, and each write as the field's setter.  The kind is static, so a
//! function whose `&text` parameter is called with the store kind gets its own INSTANCE, and
//! the stack instance — the function as written — stays byte-identical.
//!
//! The instance is a clone of the function, minted after pass 2 (like `@PLN104`'s targeted
//! promotion: once every body is parsed, and past H5's pass-1/pass-2 definition count), with
//! the parameter marked `store_text_link` and its body rewritten to the store spelling.  The
//! rewrite is total over a closed set: a parameter of the stack kind is WRITTEN only by `Set`
//! and by the `Op…Stack…` write ops (`OpAppendStackText`, `OpAppendStackCharacter`,
//! `OpClearStackText`, `OpFormatStack*`), each of which has a twin taking a text variable; every
//! other mention is a read.
//!
//! Which kind a call passes is read off the argument: `OpCreateStack(x)` and a stack link or
//! parameter are the stack kind; a place op (`OpGetField`, `OpGetVector`, `OpVarRef` of a
//! store-kind link) is the store kind.

use crate::data::{Type, Value};
use crate::diagnostics::{Level, diagnostic_format};
use std::collections::{HashMap, HashSet};

use super::Parser;

/// The stack-kind write ops, each with the name of its twin on a text variable.
fn stack_write_twin(name: &str) -> Option<&'static str> {
    Some(match name {
        "OpAppendStackText" => "OpAppendText",
        "OpAppendStackCharacter" => "OpAppendCharacter",
        "OpClearStackText" => "OpClearText",
        "OpFormatStackText" => "OpFormatText",
        "OpFormatStackInt" => "OpFormatInt",
        "OpFormatStackLong" => "OpFormatLong",
        "OpFormatStackFloat" => "OpFormatFloat",
        "OpFormatStackSingle" => "OpFormatSingle",
        "OpFormatStackDatabase" => "OpFormatDatabase",
        _ => return None,
    })
}

fn is_text_link_type(tp: &Type) -> bool {
    matches!(tp, Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)))
}

impl Parser {
    /// Is `arg`, handed to a `&text` parameter, the STORE kind — a place in a record rather
    /// than a text variable?
    fn is_store_text_arg(&self, arg: &Value) -> bool {
        match arg.unspan() {
            Value::Call(g, _) => {
                let def = self.data.def(*g);
                def.name() != "OpCreateStack"
                    && matches!(def.returned.base(), Type::Reference(_, _))
            }
            _ => false,
        }
    }

    /// Mint a store instance for every call that hands a text place to a `&text` parameter,
    /// and point those calls at it.  Instances close transitively: an instance that forwards
    /// its parameter to another `&text` function asks for that function's instance in turn.
    pub(crate) fn mint_store_text_instances(&mut self) {
        if !self.store_text_args {
            return;
        }
        let mut memo: HashMap<(u32, u64), u32> = HashMap::new();
        let mut work: Vec<u32> = (0..self.data.definitions()).collect();
        while let Some(c) = work.pop() {
            if !matches!(self.data.def(c).code, Value::Block(_)) {
                continue;
            }
            let mut code =
                std::mem::replace(&mut self.data.definitions[c as usize].code, Value::Null);
            let mut minted = Vec::new();
            self.retarget_store_text_calls(&mut code, c, &mut memo, &mut minted);
            self.data.definitions[c as usize].code = code;
            work.extend(minted);
        }
    }

    fn retarget_store_text_calls(
        &mut self,
        node: &mut Value,
        from: u32,
        memo: &mut HashMap<(u32, u64), u32>,
        minted: &mut Vec<u32>,
    ) {
        node.for_each_child_mut(&mut |c| self.retarget_store_text_calls(c, from, memo, minted));
        let Value::Call(d, args) = node else {
            return;
        };
        let callee = *d;
        if !matches!(
            self.data.def(callee).def_type,
            crate::data::DefType::Function
        ) {
            return;
        }
        let mut mask = 0u64;
        for (i, a) in self.data.def(callee).attributes.iter().enumerate() {
            if i < 64
                && is_text_link_type(&a.typedef)
                && args.get(i).is_some_and(|x| self.is_store_text_arg(x))
            {
                mask |= 1 << i;
            }
        }
        if mask == 0 {
            return;
        }
        let inst = if let Some(&i) = memo.get(&(callee, mask)) {
            i
        } else {
            let i = self.mint_store_text_instance(callee, mask, from);
            memo.insert((callee, mask), i);
            if i != u32::MAX {
                minted.push(i);
            }
            i
        };
        if inst != u32::MAX
            && let Value::Call(d, _) = node
        {
            *d = inst;
        }
    }

    /// Clone `d` as the instance whose parameters in `mask` are store-kind text links.
    fn mint_store_text_instance(&mut self, d: u32, mask: u64, caller: u32) -> u32 {
        if !matches!(self.data.def(d).code, Value::Block(_)) {
            let name = self.data.def(d).original_name();
            let pos = self.data.def(caller).position.clone();
            diagnostic_at!(
                self.lexer,
                &pos,
                Level::Error,
                "`{name}` has no loft body to link a text field or element through, so its \
                 `&text` parameter cannot take one. Copy it into a local, pass the local and \
                 write it back (`t = o.s; {name}(t); o.s = t`)"
            );
            return u32::MAX;
        }
        let mut def = self.data.definitions[d as usize].clone();
        // `@` occurs in no other key (a dispatch stub's `n_hit__dyn_E#E` already uses `#`), so
        // the key stays unique and every name decoder cuts at it: the instance is the author's
        // function in every message, trace and profile.
        let name = format!("{}@st{mask}", def.name);
        let nd = self
            .data
            .add_def(&name, &def.position, crate::data::DefType::Function);
        def.name = name;
        def.source = self.data.definitions[nd as usize].source;
        self.data.definitions[nd as usize] = def;
        let params: HashSet<u16> = self.data.definitions[nd as usize]
            .attributes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i < 64 && mask & (1 << i) != 0)
            .map(|(_, a)| self.data.definitions[nd as usize].variables.var(&a.name))
            .filter(|v| *v != u16::MAX)
            .collect();
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[nd as usize].variables,
        );
        self.context = nd;
        let mut links = params.clone();
        let mut code = std::mem::replace(&mut self.data.definitions[nd as usize].code, Value::Null);
        // A local link bound from a store-kind parameter (`u = &t`) names the same slot, so it
        // is a store-kind link too.  Closed before the rewrite, which spells every one alike.
        loop {
            let before = links.len();
            code.walk(&mut |v| {
                if let Value::Set(u, rhs) = v
                    && let Value::Call(g, a) = rhs.unspan()
                    && self.data.def(*g).name() == "OpVarRef"
                    && let [Value::Var(x)] = a.as_slice()
                    && links.contains(x)
                {
                    links.insert(*u);
                }
            });
            if links.len() == before {
                break;
            }
        }
        for &v in &links {
            self.vars.set_store_text_link(v);
        }
        self.vars.sync_work_counters();
        let work = self.vars.work_text(&mut self.lexer);
        self.rewrite_store_text(&mut code, &links, &params, work);
        if let Value::Block(bl) = &mut code {
            bl.operators
                .insert(0, crate::data::v_set(work, Value::Text(String::new())));
        }
        self.data.definitions[nd as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[nd as usize].variables,
        );
        self.context = saved_ctx;
        nd
    }

    fn store_text_read(&mut self, v: u16) -> Value {
        let r = self.cl("OpVarRef", &[Value::Var(v)]);
        self.cl("OpGetText", &[r, Value::Int(0)])
    }

    fn store_text_write(&mut self, v: u16, val: Value) -> Value {
        let r = self.cl("OpVarRef", &[Value::Var(v)]);
        self.cl("OpSetText", &[r, Value::Int(0), val])
    }

    /// Rewrite one body to the store spelling of the links in `links`.  `params` are the
    /// parameters among them: a local link's bind (`u = &t`) is kept as the bind it is.
    fn rewrite_store_text(
        &mut self,
        node: &mut Value,
        links: &HashSet<u16>,
        params: &HashSet<u16>,
        work: u16,
    ) {
        match node {
            Value::Var(x) if links.contains(x) => {
                *node = self.store_text_read(*x);
            }
            Value::Set(x, rhs) if links.contains(x) => {
                let x = *x;
                let bind = matches!(rhs.unspan(), Value::Call(g, _)
                    if matches!(self.data.def(*g).name(), "OpVarRef" | "OpCreateStack"));
                if bind {
                    if params.contains(&x)
                        || !matches!(rhs.unspan(), Value::Call(g, a)
                        if self.data.def(*g).name() == "OpVarRef"
                            && matches!(a.as_slice(), [Value::Var(y)] if links.contains(y)))
                    {
                        let name = self.vars.name(x).to_string();
                        let pos = self.data.def(self.context).position.clone();
                        diagnostic_at!(
                            self.lexer,
                            &pos,
                            Level::Error,
                            "`{name}` links a text field or element here, so it cannot be \
                             re-pointed at a text variable — a text field or element and a text \
                             variable are different places to a link. Use a second link for the \
                             other one"
                        );
                    }
                    return;
                }
                let mut val = std::mem::replace(rhs.as_mut(), Value::Null);
                self.rewrite_store_text(&mut val, links, params, work);
                *node = self.store_text_write(x, val);
            }
            Value::Call(g, args) => {
                let g = *g;
                let name = self.data.def(g).name().to_string();
                // The slot itself (a link bound from the parameter, or a hand-on below) — not
                // a read of the text.
                if name == "OpVarRef" {
                    return;
                }
                if let Some(twin) = stack_write_twin(&name)
                    && let Some(Value::Var(x)) = args.first().map(Value::unspan)
                    && links.contains(x)
                {
                    let x = *x;
                    let mut rest: Vec<Value> = args.drain(1..).collect();
                    for a in &mut rest {
                        self.rewrite_store_text(a, links, params, work);
                    }
                    if twin == "OpClearText" {
                        *node = self.store_text_write(x, Value::Text(String::new()));
                        return;
                    }
                    let read = self.store_text_read(x);
                    let mut call = vec![Value::Var(work)];
                    call.extend(rest);
                    let op = self.cl(twin, &call);
                    let back = self.store_text_write(x, Value::Var(work));
                    *node = Value::Insert(vec![crate::data::v_set(work, read), op, back]);
                    return;
                }
                // A store-kind link handed on to another `&text` parameter is handed on as
                // the slot it holds; `retarget_store_text_calls` then picks that callee's
                // store instance.
                let attrs: Vec<bool> =
                    if matches!(self.data.def(g).def_type, crate::data::DefType::Function) {
                        self.data
                            .def(g)
                            .attributes
                            .iter()
                            .map(|a| is_text_link_type(&a.typedef))
                            .collect()
                    } else {
                        Vec::new()
                    };
                for (i, a) in args.iter_mut().enumerate() {
                    if attrs.get(i).copied().unwrap_or(false)
                        && let Value::Var(x) = a.unspan()
                        && links.contains(x)
                    {
                        *a = self.cl("OpVarRef", &[Value::Var(*x)]);
                        continue;
                    }
                    self.rewrite_store_text(a, links, params, work);
                }
            }
            _ => node.for_each_child_mut(&mut |c| self.rewrite_store_text(c, links, params, work)),
        }
    }
}
