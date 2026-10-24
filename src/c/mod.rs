// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! The C emitter (@PLN200 B5, @PLN91 strand 5's P-cemit probe): loft functions written as C99
//! and built with `cc`.  It is a part of its own, beside the Rust writer (`generation`) and
//! the interpreter: it reads the IR, the analyses in `opt` and the plans in `lower`, and
//! names neither sibling (`@C153`, `scripts/crate_layers.py`).
//!
//! It starts PLAIN (the owner's decision in the plan): every value takes the general form,
//! and no per-function decision is shared yet.  This first cut carries the SCALAR subset —
//! 64-bit integers, booleans as conditions, operators, `if`, loops with `break`/`continue`,
//! calls between such functions and `return` — and refuses everything else with a
//! [`Refusal`] that names the function and the construct.  Values follow the interpreter's
//! rules: an integer operand that is null (`i64::MIN`) makes the result null, and so does an
//! overflow or a division by zero (`ops::op_add_long` and its peers).  What the C does NOT do
//! yet is report: a division by zero the interpreter raises as a recoverable error is a null
//! here and nothing more, because how a C program reaches the runtime is @PLN91's decision.

use crate::data::{Block, Data, DefType, Type, Value};
use std::fmt::Write as _;

/// Why a function cannot be written as C yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The loft name of the function.
    pub function: String,
    /// The construct the C emitter does not carry.
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.function, self.why)
    }
}

/// The C operators behind loft's sentinel-aware integer arithmetic, and the null sentinel.
const PRELUDE: &str = r"#include <stdint.h>

#define LOFT_NULL INT64_MIN

static int64_t loft_add(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    if ((b > 0 && a > INT64_MAX - b) || (b < 0 && a < INT64_MIN - b)) return LOFT_NULL;
    return a + b;
}

static int64_t loft_sub(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    if ((b < 0 && a > INT64_MAX + b) || (b > 0 && a < INT64_MIN + b)) return LOFT_NULL;
    return a - b;
}

static int64_t loft_mul(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    if (a > 0) {
        if (b > 0) { if (a > INT64_MAX / b) return LOFT_NULL; }
        else if (b < INT64_MIN / a) return LOFT_NULL;
    } else if (a < 0) {
        if (b > 0) { if (a < INT64_MIN / b) return LOFT_NULL; }
        else if (b != 0 && a < INT64_MAX / b) return LOFT_NULL;
    }
    return a * b;
}

static int64_t loft_div(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b == 0) return LOFT_NULL;
    return a / b;
}

static int64_t loft_rem(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b == 0) return LOFT_NULL;
    return a % b;
}
";

/// The C expression of a binary operator over two C expressions, or `None`.
fn binary(op: &str, a: &str, b: &str) -> Option<String> {
    let call = |f: &str| Some(format!("{f}({a}, {b})"));
    let infix = |o: &str| Some(format!("({a} {o} {b})"));
    match op {
        "OpAddInt" | "OpAddLong" => call("loft_add"),
        "OpMinInt" | "OpMinLong" => call("loft_sub"),
        "OpMulInt" | "OpMulLong" => call("loft_mul"),
        "OpDivInt" | "OpDivLong" => call("loft_div"),
        "OpRemInt" | "OpRemLong" => call("loft_rem"),
        "OpEqInt" | "OpEqLong" | "OpEqBool" => infix("=="),
        "OpNeInt" | "OpNeLong" | "OpNeBool" => infix("!="),
        "OpLtInt" | "OpLtLong" => infix("<"),
        "OpLeInt" | "OpLeLong" => infix("<="),
        _ => None,
    }
}

/// Is `tp` a type this cut carries as a C value: a plain 64-bit integer, nullable or not?  A
/// null integer is the same `int64_t` holding `LOFT_NULL`, so the `?` changes nothing here.
fn scalar(tp: &Type) -> bool {
    match tp.base() {
        Type::Integer(spec) => {
            spec.forced_size.is_none()
                && spec.min == i32::MIN + 1
                && (spec.max == i64::from(i32::MAX) || spec.max == i64::from(u32::MAX))
        }
        _ => false,
    }
}

/// The C name of definition `d`: the identifier every compiled target gives it.
#[must_use]
pub fn c_name(data: &Data, d: u32) -> String {
    let dups = crate::opt::names::duplicate_fn_names(data);
    crate::opt::names::disambiguated_fn_ident(&dups, data.def(d))
}

/// C99 for `roots` and every function they reach: the prelude, a prototype per function and
/// each body.  A function or construct this cut does not carry refuses the whole unit.
///
/// # Errors
/// The first [`Refusal`]: a reached definition that is not a loft function of the scalar
/// subset, or a construct inside one.
pub fn emit_unit(data: &Data, roots: &[u32]) -> Result<String, Refusal> {
    let mut fns: Vec<u32> = crate::opt::names::reachable_functions(data, roots)
        .into_iter()
        // An operator is a definition too; the call site writes it (`binary`).
        .filter(|&d| {
            matches!(data.def(d).def_type(), DefType::Function)
                && !data.def(d).name().starts_with("Op")
        })
        .collect();
    // The roots first, so a refusal names the function asked for before one it reaches.
    fns.sort_unstable_by_key(|d| (!roots.contains(d), *d));
    let mut out = String::from(PRELUDE);
    let mut bodies = String::new();
    out.push('\n');
    for &d in &fns {
        let def = data.def(d);
        let refuse = |why: String| Refusal {
            function: def.original_name().clone(),
            why,
        };
        if def.code() == &Value::Null {
            return Err(refuse(
                "it has no loft body (a native or built-in function)".into(),
            ));
        }
        let ret = match def.returned().base() {
            Type::Void => "void",
            t if scalar(t) => "int64_t",
            t => return Err(refuse(format!("it returns {t:?}"))),
        };
        let vars = def.variables();
        let mut params = Vec::new();
        for (i, a) in def.attributes().iter().enumerate() {
            if !scalar(&a.typedef) {
                return Err(refuse(format!("parameter {} is {:?}", a.name, a.typedef)));
            }
            params.push(format!(
                "int64_t {}",
                var_name(vars.name(i as u16), i as u16)
            ));
        }
        let params = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };
        let sig = format!("static {ret} {}({params})", c_name(data, d));
        let _ = writeln!(out, "{sig};");
        let mut f = FnWriter {
            data,
            vars,
            body: String::new(),
            temps: 0,
            loops: Vec::new(),
            labels: 0,
        };
        let mut locals = String::new();
        for v in def.attributes().len() as u16..vars.count() {
            if !scalar(vars.tp(v)) {
                return Err(refuse(format!(
                    "local {} is {:?}",
                    vars.name(v),
                    vars.tp(v)
                )));
            }
            let _ = writeln!(
                locals,
                "    int64_t {} = LOFT_NULL;",
                var_name(vars.name(v), v)
            );
        }
        if ret == "void" {
            f.stmt(def.code(), 1).map_err(refuse)?;
        } else {
            let value = f.expr(def.code(), 1).map_err(refuse)?;
            let _ = writeln!(f.body, "    return {value};");
        }
        let _ = writeln!(bodies, "\n{sig} {{\n{locals}{}}}", f.body);
    }
    out.push_str(&bodies);
    Ok(out)
}

/// A C-safe name for variable `v`: its number first, so two loft names that sanitise alike
/// stay apart.
fn var_name(name: &str, v: u16) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("v{v}_{clean}")
}

/// One function's body under construction.
struct FnWriter<'a> {
    data: &'a Data,
    vars: &'a crate::variables::Function,
    body: String,
    temps: u32,
    /// The label number of each enclosing loop, innermost last.
    loops: Vec<u32>,
    labels: u32,
}

impl FnWriter<'_> {
    fn line(&mut self, ind: usize, s: &str) {
        let _ = writeln!(self.body, "{}{s}", "    ".repeat(ind));
    }

    fn temp(&mut self) -> String {
        self.temps += 1;
        format!("t{}", self.temps)
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

    /// Emit `v` for its effect.
    fn stmt(&mut self, v: &Value, ind: usize) -> Result<(), String> {
        match v {
            Value::Null | Value::Line(_) => Ok(()),
            Value::Span(b) => self.stmt(&b.1, ind),
            Value::Block(b) => self.block_stmt(b, ind),
            Value::Insert(ops) => ops.iter().try_for_each(|o| self.stmt(o, ind)),
            Value::Set(var, x) => {
                let name = var_name(self.vars.name(*var), *var);
                self.assign(x, &name, ind)
            }
            Value::If(c, t, e) => {
                let c = self.expr(c, ind)?;
                self.line(ind, &format!("if ({}) {{", bare(&c)));
                self.stmt(t, ind + 1)?;
                if !matches!(**e, Value::Null) {
                    self.line(ind, "} else {");
                    self.stmt(e, ind + 1)?;
                }
                self.line(ind, "}");
                Ok(())
            }
            Value::Loop(b) => {
                self.labels += 1;
                let l = self.labels;
                self.loops.push(l);
                self.line(ind, "for (;;) {");
                self.block_stmt(b, ind + 1)?;
                self.line(ind, &format!("cont_{l}:;"));
                self.line(ind, "}");
                self.line(ind, &format!("brk_{l}:;"));
                self.loops.pop();
                Ok(())
            }
            Value::Break(n) => {
                let l = self.label(*n)?;
                self.line(ind, &format!("goto brk_{l};"));
                Ok(())
            }
            Value::Continue(n) => {
                let l = self.label(*n)?;
                self.line(ind, &format!("goto cont_{l};"));
                Ok(())
            }
            Value::Return(x) => {
                if matches!(**x, Value::Null) {
                    self.line(ind, "return;");
                } else {
                    let x = self.expr(x, ind)?;
                    self.line(ind, &format!("return {x};"));
                }
                Ok(())
            }
            Value::Drop(x) => {
                let x = self.expr(x, ind)?;
                self.line(ind, &format!("(void)({x});"));
                Ok(())
            }
            Value::Call(..) => {
                let x = self.expr(v, ind)?;
                self.line(ind, &format!("(void)({x});"));
                Ok(())
            }
            other => Err(unsupported(other)),
        }
    }

    fn block_stmt(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        b.operators.iter().try_for_each(|o| self.stmt(o, ind))
    }

    /// Emit the statements `v` needs and assign its value to `target`.
    fn assign(&mut self, v: &Value, target: &str, ind: usize) -> Result<(), String> {
        match v {
            Value::Span(b) => self.assign(&b.1, target, ind),
            Value::Block(b) => self.assign_seq(&b.operators, target, ind),
            Value::Insert(ops) => self.assign_seq(ops, target, ind),
            Value::If(c, t, e) => {
                let c = self.expr(c, ind)?;
                self.line(ind, &format!("if ({}) {{", bare(&c)));
                self.assign(t, target, ind + 1)?;
                self.line(ind, "} else {");
                self.assign(e, target, ind + 1)?;
                self.line(ind, "}");
                Ok(())
            }
            Value::Return(_) | Value::Break(_) | Value::Continue(_) => self.stmt(v, ind),
            _ => {
                let x = self.expr(v, ind)?;
                self.line(ind, &format!("{target} = {x};"));
                Ok(())
            }
        }
    }

    fn assign_seq(&mut self, ops: &[Value], target: &str, ind: usize) -> Result<(), String> {
        let Some((last, rest)) = ops.split_last() else {
            return Err("a value block with no operators".into());
        };
        rest.iter().try_for_each(|o| self.stmt(o, ind))?;
        self.assign(last, target, ind)
    }

    /// A C expression for `v`'s value; the statements it needs are emitted first.
    fn expr(&mut self, v: &Value, ind: usize) -> Result<String, String> {
        match v {
            Value::Span(b) => self.expr(&b.1, ind),
            Value::Int(i) => Ok(format!("INT64_C({i})")),
            Value::Long(l) if *l == i64::MIN => Ok("LOFT_NULL".into()),
            Value::Long(l) => Ok(format!("INT64_C({l})")),
            Value::Boolean(b) => Ok(if *b { "1" } else { "0" }.into()),
            Value::Var(var) => Ok(var_name(self.vars.name(*var), *var)),
            Value::Call(d, args) => self.call(*d, args, ind),
            Value::Block(_) | Value::Insert(_) | Value::If(..) => {
                let t = self.temp();
                self.line(ind, &format!("int64_t {t} = LOFT_NULL;"));
                self.assign(v, &t, ind)?;
                Ok(t)
            }
            other => Err(unsupported(other)),
        }
    }

    fn call(&mut self, d: u32, args: &[Value], ind: usize) -> Result<String, String> {
        let name = self.data.def(d).name().to_string();
        // Each argument in its own temporary, in order: C leaves the order in which a call's
        // arguments are evaluated open, and loft evaluates them left to right.
        let mut xs = Vec::with_capacity(args.len());
        for a in args {
            let x = self.expr(a, ind)?;
            if matches!(
                a.unspan(),
                Value::Var(_) | Value::Int(_) | Value::Long(_) | Value::Boolean(_)
            ) {
                xs.push(x);
            } else {
                let t = self.temp();
                self.line(ind, &format!("int64_t {t} = {x};"));
                xs.push(t);
            }
        }
        if let [a, b] = &xs[..]
            && let Some(x) = binary(&name, a, b)
        {
            return Ok(x);
        }
        if name == "OpNot"
            && let [a] = &xs[..]
        {
            return Ok(format!("(!{a})"));
        }
        if name.starts_with("Op") {
            return Err(format!("the operator {name}"));
        }
        Ok(format!("{}({})", c_name(self.data, d), xs.join(", ")))
    }
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
