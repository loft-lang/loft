// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-ParRows` — the ROW variant of a `par` worker: the worker and every loft function it
//! calls, translated to Rust items that read the element's fields straight from the vector and
//! touch no store, so a batch of rows runs in a loop the compiler can vectorise
//! (`codegen_runtime::n_parallel_rows_native`; design: `bench/portal/analysis/par-rows.md`).
//!
//! The translation covers a SUBSET of the IR and declines everything else, naming why: scalar
//! literals and locals, assignment, blocks, `if`, loops, `return`, a field read of the element,
//! a call to another loft function the subset covers, and an operator whose `#rust` template
//! touches no store.  The values are exact by construction — each operator is expanded from
//! its own template — with two substitutions, both value-preserving:
//!
//! - an operator with a silent `…Nullable` twin whose own template calls `ops::op_…` takes the
//!   twin: the two answer the same value, and the reporting one reports only under
//!   `--dev-soft-halt`, where the runtime takes the exact path for every row;
//! - `OpDivFloat`, which RAISES on a zero divisor, divides plainly and raises the batch's
//!   `__bad` flag instead, so the runtime reruns that batch through the exact path and the
//!   raise happens there.  A nonzero literal divisor needs no flag.
//!
//! A range loop with a constant count of at most [`UNROLL_MAX`] rounds and a straight-line
//! body is UNROLLED: with a select in its body the compiler keeps the loop rolled, and then it
//! cannot vectorise the row loop around it (priced: 3.29× against 1.78× of Rust).

use crate::data::{Block, Data, Definition, Type, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The most rounds a range loop is unrolled to.
const UNROLL_MAX: i32 = 64;

/// `LOFT_NO_PAR_ROWS=1` — `(R-ParRows)` off: every `par` keeps its per-row worker.
#[must_use]
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !crate::keys::env_set("LOFT_NO_PAR_ROWS"))
}

/// `LOFT_TRACE_PAR_ROWS=1` — each `par` admitted or declined, with the reason.
pub fn trace(msg: &str) {
    if std::env::var_os("LOFT_TRACE_PAR_ROWS").is_some() {
        eprintln!("par-rows: {msg}");
    }
}

/// The Rust items of `worker`'s row variant and its callees', named `<fn>__row`, or the reason
/// the worker is not row-pure.  The worker takes the element's address and the batch flag; it
/// must have exactly one parameter, a record, and a scalar result.
///
/// # Errors
/// The first construct the translation does not cover, named: the `par` keeps its per-row worker.
pub fn row_variants(data: &Data, worker: u32) -> Result<String, String> {
    let mut t = Translator {
        data,
        out: String::new(),
        done: Vec::new(),
        active: Vec::new(),
    };
    t.function(worker, true)?;
    Ok(t.out)
}

struct Translator<'a> {
    data: &'a Data,
    out: String,
    done: Vec<u32>,
    active: Vec<u32>,
}

/// One function's translation state: which variable is the element, the locals met so far
/// with their Rust types, and the loop labels open around the current node.
struct FnState<'a> {
    def: &'a Definition,
    params: usize,
    row_var: Option<u16>,
    locals: BTreeMap<u16, &'static str>,
    labels: Vec<usize>,
    next_label: usize,
}

/// A loft type as the Rust scalar a row variant holds it in, with the value a local starts at.
fn scalar(tp: &Type) -> Option<(&'static str, &'static str)> {
    match crate::rust_spelling::rust_type(tp, &crate::data::Context::Variable).as_str() {
        "i64" => Some(("i64", "i64::MIN")),
        "f64" => Some(("f64", "f64::NAN")),
        "f32" => Some(("f32", "f32::NAN")),
        "bool" => Some(("bool", "false")),
        _ => None,
    }
}

impl<'a> Translator<'a> {
    fn function(&mut self, d: u32, worker: bool) -> Result<(), String> {
        if self.done.contains(&d) {
            return Ok(());
        }
        let def = self.data.def(d);
        let name = def.name().to_string();
        if self.active.contains(&d) {
            return Err(format!("`{name}` is recursive"));
        }
        if !def.is_loft_defined() {
            return Err(format!("`{name}` has no loft body"));
        }
        self.active.push(d);
        let mut f = FnState {
            def,
            params: def.attributes().len(),
            row_var: None,
            locals: BTreeMap::new(),
            labels: Vec::new(),
            next_label: 0,
        };
        let mut sig = String::new();
        for (i, a) in def.attributes().iter().enumerate() {
            if a.hidden {
                return Err(format!("`{name}` has a hidden parameter `{}`", a.name));
            }
            if worker && i == 0 {
                if !matches!(a.typedef, Type::Reference(_, _)) {
                    return Err(format!("`{name}`'s parameter is not a record"));
                }
                f.row_var = Some(0);
                sig.push_str("__row: *const u8, ");
                continue;
            }
            let Some((ty, _)) = scalar(&a.typedef) else {
                return Err(format!("`{name}`'s parameter `{}` is not a scalar", a.name));
            };
            let _ = write!(sig, "mut v{i}: {ty}, ");
        }
        if worker && f.params != 1 {
            return Err(format!(
                "`{name}` takes {} parameters, not the element alone",
                f.params
            ));
        }
        let Some((ret, _)) = scalar(def.returned()) else {
            return Err(format!("`{name}` does not return a scalar"));
        };
        let body = self.expr(&mut f, def.code())?;
        let mut locals = String::new();
        for (v, ty) in &f.locals {
            let init = match *ty {
                "i64" => "i64::MIN",
                "f64" => "f64::NAN",
                "f32" => "f32::NAN",
                _ => "false",
            };
            let _ = write!(locals, "let mut v{v}: {ty} = {init}; ");
        }
        let _ = writeln!(
            self.out,
            "#[inline(always)] #[allow(unused, unused_mut, unused_assignments, unreachable_code, clippy::all)] fn {name}__row({sig}__bad: &mut bool) -> {ret} {{ {locals}{body} }}"
        );
        self.active.pop();
        self.done.push(d);
        Ok(())
    }

    /// A local's Rust name, registering its type on first sight; the element and the
    /// parameters are not locals.
    fn var(f: &mut FnState, v: u16) -> Result<String, String> {
        if Some(v) == f.row_var {
            return Err("the element is used other than by a field read".to_string());
        }
        if usize::from(v) >= f.params && !f.locals.contains_key(&v) {
            let Some((ty, _)) = scalar(f.def.variables.tp(v)) else {
                return Err(format!(
                    "local `{}` is not a scalar",
                    f.def.variables.name(v)
                ));
            };
            f.locals.insert(v, ty);
        }
        Ok(format!("v{v}"))
    }

    #[allow(clippy::too_many_lines)]
    #[expect(
        clippy::many_single_char_names,
        reason = "the IR node's parts, named as `data.rs` names them"
    )]
    fn expr(&mut self, f: &mut FnState<'a>, v: &Value) -> Result<String, String> {
        Ok(match v.unspan() {
            Value::Line(_) => "()".to_string(),
            Value::Int(i) => format!("({i}_i64)"),
            Value::Long(l) if *l == i64::MIN => "i64::MIN".to_string(),
            Value::Long(l) => format!("({l}_i64)"),
            Value::Float(x) => format!("f64::from_bits({}_u64)", x.to_bits()),
            Value::Single(x) => format!("f32::from_bits({}_u32)", x.to_bits()),
            Value::Boolean(b) => b.to_string(),
            Value::Var(n) => Self::var(f, *n)?,
            Value::Set(n, e) => {
                let e = self.expr(f, e)?;
                format!("{{ {} = {e}; }}", Self::var(f, *n)?)
            }
            Value::Block(b) => self.block(f, b)?,
            Value::If(c, a, b) => {
                let c = self.expr(f, c)?;
                let a = self.expr(f, a)?;
                let b = if matches!(b.unspan(), Value::Null) {
                    String::new()
                } else {
                    self.expr(f, b)?
                };
                format!("(if {c} {{ {a} }} else {{ {b} }})")
            }
            Value::Loop(b) => {
                let label = f.next_label;
                f.next_label += 1;
                f.labels.push(label);
                let body = self.statements(f, &b.operators)?;
                f.labels.pop();
                format!("'l{label}: loop {{ {body} }}")
            }
            Value::Break(n) => {
                let Some(l) = f.labels.len().checked_sub(1 + usize::from(*n)) else {
                    return Err("a break leaves the function".to_string());
                };
                format!("break 'l{}", f.labels[l])
            }
            Value::Continue(n) => {
                let Some(l) = f.labels.len().checked_sub(1 + usize::from(*n)) else {
                    return Err("a continue leaves the function".to_string());
                };
                format!("continue 'l{}", f.labels[l])
            }
            Value::Return(e) => format!("return {}", self.expr(f, e)?),
            Value::Call(d, args) => self.call(f, *d, args)?,
            Value::Null => return Err("a null value".to_string()),
            other => {
                let s = format!("{other:?}");
                return Err(format!(
                    "`{}` is not covered",
                    s.split(['(', ' ']).next().unwrap_or("?")
                ));
            }
        })
    }

    /// A block's operators as statements, the last one its value unless the block is void.
    /// A range loop with a constant count and a straight-line body is unrolled here, where its
    /// counter's initialisation and the loop sit side by side.
    fn block(&mut self, f: &mut FnState<'a>, b: &Block) -> Result<String, String> {
        let ops: Vec<&Value> = b
            .operators
            .iter()
            .filter(|o| !matches!(o.unspan(), Value::Line(_)))
            .collect();
        let void = matches!(b.result, Type::Void);
        let mut out = String::from("{ ");
        let mut i = 0;
        while i < ops.len() {
            let last = i + 1 == ops.len();
            if let Some((text, used)) = self.unrolled(f, &ops[i..])? {
                out.push_str(&text);
                out.push(' ');
                i += used;
                continue;
            }
            let e = self.expr(f, ops[i])?;
            if last && !void {
                out.push_str(&e);
            } else {
                let _ = write!(out, "{e}; ");
            }
            i += 1;
        }
        out.push_str(" }");
        Ok(out)
    }

    fn statements(&mut self, f: &mut FnState<'a>, ops: &[Value]) -> Result<String, String> {
        let mut out = String::new();
        for o in ops {
            if matches!(o.unspan(), Value::Line(_)) {
                continue;
            }
            let e = self.expr(f, o)?;
            let _ = write!(out, "{e}; ");
        }
        Ok(out)
    }

    /// `idx = -1; loop { v = { idx = idx + 1; if C <= idx break; idx }; body }` with a literal
    /// `1 <= C <= UNROLL_MAX` and a body that holds no loop, `break`, `continue` or `return`:
    /// the body `C` times with `v` set to each round, leaving `idx = C` and `v = C - 1` as the
    /// loop leaves them.  `None` for any other shape.
    #[expect(
        clippy::many_single_char_names,
        reason = "the IR node's parts, named as `data.rs` names them"
    )]
    fn unrolled(
        &mut self,
        f: &mut FnState<'a>,
        ops: &[&Value],
    ) -> Result<Option<(String, usize)>, String> {
        let (Some(Value::Set(idx, init)), Some(Value::Loop(lb))) = (
            ops.first().map(|v| v.unspan()),
            ops.get(1).map(|v| v.unspan()),
        ) else {
            return Ok(None);
        };
        if !matches!(init.unspan(), Value::Int(-1)) {
            return Ok(None);
        }
        let body: Vec<&Value> = lb
            .operators
            .iter()
            .filter(|o| !matches!(o.unspan(), Value::Line(_)))
            .collect();
        let Some((Value::Set(var, iter), rest)) = body.split_first().map(|(h, r)| (h.unspan(), r))
        else {
            return Ok(None);
        };
        let Value::Block(ib) = iter.unspan() else {
            return Ok(None);
        };
        let it: Vec<&Value> = ib
            .operators
            .iter()
            .filter(|o| !matches!(o.unspan(), Value::Line(_)))
            .collect();
        let [step, test, tail] = it.as_slice() else {
            return Ok(None);
        };
        let is_op = |v: &Value, name: &str| match v.unspan() {
            Value::Call(d, _) => self.data.def(*d).name() == name,
            _ => false,
        };
        let step_ok = matches!(step.unspan(), Value::Set(s, e)
            if s == idx && is_op(e, "OpAddInt")
                && matches!(e.unspan(), Value::Call(_, a)
                    if matches!(a.as_slice(), [x, y] if matches!(x.unspan(), Value::Var(n) if n == idx)
                        && matches!(y.unspan(), Value::Int(1)))));
        let count = match test.unspan() {
            Value::If(c, then, other)
                if matches!(then.unspan(), Value::Break(0))
                    && matches!(other.unspan(), Value::Null)
                    && is_op(c, "OpLeInt") =>
            {
                match c.unspan() {
                    Value::Call(_, a) => match a.as_slice() {
                        [n, x] if matches!(x.unspan(), Value::Var(m) if m == idx) => {
                            match n.unspan() {
                                Value::Int(n) => Some(*n),
                                _ => None,
                            }
                        }
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        };
        let tail_ok = matches!(tail.unspan(), Value::Var(n) if n == idx);
        let Some(count) = count.filter(|c| (1..=UNROLL_MAX).contains(c)) else {
            return Ok(None);
        };
        if !step_ok || !tail_ok || rest.iter().any(|o| has_control(o)) {
            return Ok(None);
        }
        let idx_name = Self::var(f, *idx)?;
        let var_name = Self::var(f, *var)?;
        let mut round = String::new();
        for o in rest {
            let e = self.expr(f, o)?;
            let _ = write!(round, "{e}; ");
        }
        let mut out = String::new();
        for k in 0..count {
            let _ = write!(out, "{{ {var_name} = {k}_i64; {round}}} ");
        }
        let _ = write!(out, "{idx_name} = {count}_i64;");
        Ok(Some((out, 2)))
    }

    #[expect(
        clippy::many_single_char_names,
        reason = "the IR node's parts, named as `data.rs` names them"
    )]
    fn call(&mut self, f: &mut FnState<'a>, d: u32, args: &[Value]) -> Result<String, String> {
        let def = self.data.def(d);
        let name = def.name();
        // A field of the element, read where the exact path's getter reads it.
        if let Some(Value::Var(n)) = args.first().map(Value::unspan)
            && Some(*n) == f.row_var
        {
            let Some(Value::Int(fld)) = args.get(1).map(Value::unspan) else {
                return Err(format!("`{name}` on the element"));
            };
            let at = format!("unsafe {{ __row.add({fld}_usize)");
            return Ok(match name {
                "OpGetInt" | "OpGetLong" => format!("{at}.cast::<i64>().read_unaligned() }}"),
                "OpGetFloat" => format!("{at}.cast::<f64>().read_unaligned() }}"),
                "OpGetSingle" => format!("{at}.cast::<f32>().read_unaligned() }}"),
                "OpGetInt4" => format!(
                    "{{ let __r = {at}.cast::<i32>().read_unaligned() }}; if __r == i32::MIN {{ i64::MIN }} else {{ i64::from(__r) }} }}"
                ),
                _ => return Err(format!("`{name}` reads the element")),
            });
        }
        if def.is_loft_defined() {
            self.function(d, false)?;
            let mut out = format!("{name}__row(");
            for a in args {
                let e = self.expr(f, a)?;
                let _ = write!(out, "{e}, ");
            }
            out.push_str("__bad)");
            return Ok(out);
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.expr(f, a)?);
        }
        // `+` and `-` with the overflow found by sign bits: the nullable ops' values in a form
        // the compiler vectorises (`checked_add` lowers to an intrinsic no vectoriser widens).
        let bitwise = match name {
            "OpAddInt" | "OpAddIntNullable" | "OpAddLong" | "OpAddLongNullable" => {
                Some("ops::op_add_long_bitwise")
            }
            "OpMinInt" | "OpMinIntNullable" | "OpMinLong" | "OpMinLongNullable" => {
                Some("ops::op_min_long_bitwise")
            }
            _ => None,
        };
        if let (Some(op), [a, b]) = (bitwise, vals.as_slice()) {
            return Ok(format!("{op}({a}, {b})"));
        }
        if name == "OpDivFloat" {
            let [a, b] = vals.as_slice() else {
                return Err("`OpDivFloat` arity".to_string());
            };
            if matches!(args[1].unspan(), Value::Float(x) if *x != 0.0) {
                return Ok(format!("(({a}) / ({b}))"));
            }
            return Ok(format!(
                "{{ let __b: f64 = {b}; if __b == 0.0 {{ *__bad = true; }} ({a}) / __b }}"
            ));
        }
        let twin = format!("{name}Nullable");
        let used = if def.rust().contains("ops::op_")
            && let Some(t) = (self.data.def_nr(&twin) != u32::MAX)
                .then(|| self.data.def(self.data.def_nr(&twin)))
        {
            t
        } else {
            def
        };
        let tpl = used.rust();
        if tpl.is_empty() {
            return Err(format!("`{name}` has no template"));
        }
        if [
            "stores",
            "cell",
            "s.raise",
            "raise_recoverable",
            "store(",
            "crate::",
        ]
        .iter()
        .any(|w| tpl.contains(w))
        {
            return Err(format!("`{name}` touches the store"));
        }
        let mut res = tpl.replace("{{", "{").replace("}}", "}");
        let mut attrs: Vec<(usize, &str)> = used
            .attributes()
            .iter()
            .enumerate()
            .map(|(i, a)| (i, a.name.as_str()))
            .collect();
        attrs.sort_by_key(|(_, n)| std::cmp::Reverse(n.len()));
        for (i, n) in attrs {
            let Some(val) = vals.get(i) else {
                return Err(format!("`{name}` arity"));
            };
            res = res.replace(&format!("@{n}"), &format!("({val})"));
        }
        if res.contains('@') {
            return Err(format!("`{name}`'s template keeps a parameter"));
        }
        Ok(format!("({res})"))
    }
}

/// Does `v` hold a loop, `break`, `continue` or `return` — anything that would change the
/// shape of an unrolled round?
fn has_control(v: &Value) -> bool {
    match v.unspan() {
        Value::Loop(_) | Value::Break(_) | Value::Continue(_) | Value::Return(_) => true,
        Value::Block(b) => b.operators.iter().any(has_control),
        Value::If(c, a, b) => has_control(c) || has_control(a) || has_control(b),
        Value::Set(_, e) => has_control(e),
        Value::Call(_, args) => args.iter().any(has_control),
        _ => false,
    }
}
