// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-SpecChain` (@PLN219) in the C: an integer local assigned a CHAIN of the silent
//! checked operators — `+ - *` and `& | ^` — is written as the processor's operators, the
//! overflow builtins collecting every event the checked chain would answer null for into one
//! flag: a null operand, an overflow, an inner result equal to the sentinel.  An unlikely test
//! of that flag recomputes the chain checked from the same operand values.
//!
//! Every checked operator answers null for exactly those events and otherwise the bits of the
//! processor's operator, and none of them reports anything; so where no event happened the
//! plain chain answers what the checked one would, and where one did the checked chain itself
//! answers.  The operands are evaluated once, in the checked chain's order.  What moves is the
//! dependency chain: a checked operator compiles to selects on the value it answers, and a loop
//! that carries the value (`acc = (acc ^ i) + (i & 7)`) waits on them every trip.
//!
//! An operator `(R-Range)` proves cannot fault is written plain and tested for nothing.  A
//! chain whose every operator is proven plain is left to that rule, a single operator over two
//! operands to the checked template (nothing to gain), and a chain inside a guarded loop's copy
//! to `(R-GuardedChain)`.  Only inside a loop, where a carried value waits on the chain: outside
//! one there is no chain to shorten, and the longer statement would cost a small function its
//! `inline` (`(R-Inline)` counts statements).  `LOFT_NO_SPEC_CHAIN=1`.

use super::{FnWriter, as_int};
use crate::data::Value;

/// One node of the chain, in post-order: a leaf (its temp, and whether it may be null) or an
/// operator over two nodes (whether a proof writes it plain).
enum Node {
    Leaf(String, bool),
    Op {
        name: &'static str,
        lhs: usize,
        rhs: usize,
        proven: bool,
        /// The range proof bounds the result: never the sentinel.
        bounded: bool,
    },
}

fn chain_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "OpAddInt" => "OpAddInt",
        "OpMinInt" => "OpMinInt",
        "OpMulInt" => "OpMulInt",
        "OpLandInt" => "OpLandInt",
        "OpLorInt" => "OpLorInt",
        "OpEorInt" => "OpEorInt",
        _ => return None,
    })
}

impl FnWriter<'_> {
    /// `var = x` for an integer local and a chain of at least two operators not all proven
    /// plain: written as the speculative plain chain.  Answers whether it wrote it.
    pub(super) fn spec_chain(&mut self, var: u16, x: &Value, ind: usize) -> Result<bool, String> {
        if crate::env_once!(std::env::var("LOFT_NO_SPEC_CHAIN").is_ok_and(|v| v != "0"))
            || self.gen_elem.is_some()
            || self.loops.is_empty()
        {
            return Ok(false);
        }
        let Value::Call(def, args) = x.unspan() else {
            return Ok(false);
        };
        let data = self.cx.data;
        if (*def as usize) >= data.definitions.len()
            || chain_name(data.def(*def).name()).is_none()
            || args.len() != 2
        {
            return Ok(false);
        }
        // Count the operators and the ones a proof already writes plain.
        let (mut ops, mut open) = (0usize, 0usize);
        self.chain_census(x, &mut ops, &mut open);
        // One operator is worth it only where the local carries itself: `acc = acc + e`.
        let carried = args
            .iter()
            .any(|a| matches!(a.unspan(), Value::Var(w) if *w == var));
        if (ops < 2 && !carried) || open == 0 || self.chain_admitted(x) {
            return Ok(false);
        }
        let mut nodes: Vec<Node> = Vec::new();
        let root = self.chain_nodes(x, &mut nodes, ind)?;
        let bad = self.temp(super::CType::Int);
        self.line(ind, &format!("{bad} = 0;"));
        for n in &nodes {
            if let Node::Leaf(t, true) = n {
                self.line(ind, &format!("{bad} |= ({t} == LOFT_NULL);"));
            }
        }
        let mut vals: Vec<String> = Vec::with_capacity(nodes.len());
        for (i, n) in nodes.iter().enumerate() {
            match n {
                Node::Leaf(t, _) => vals.push(t.clone()),
                Node::Op {
                    name,
                    lhs,
                    rhs,
                    proven,
                    bounded,
                } => {
                    let (a, b) = (vals[*lhs].clone(), vals[*rhs].clone());
                    let r = self.temp(super::CType::Int);
                    let plain = |op: &str| format!("{r} = ({a}) {op} ({b});");
                    // A product by a literal keeps `loft_mul`, whose bounds fold against the
                    // constant (the overflow builtin is a high multiply on the chain); its null
                    // answer — an overflow, a null operand, the sentinel — sets the flag.
                    let literal = |v: &str| v.starts_with("INT64_C(");
                    if *name == "OpMulInt" && !*proven && (literal(&a) || literal(&b)) {
                        self.line(ind, &format!("{r} = loft_mul({a}, {b});"));
                        self.line(ind, &format!("{bad} |= ({r} == LOFT_NULL);"));
                        vals.push(r);
                        continue;
                    }
                    let site = format!("\"R-Range:{}\"", self.cx.data.def(self.d).name());
                    let proof = |mac: &str| format!("{r} = {mac}({a}, {b}, {site});");
                    let line = match (*name, *proven) {
                        ("OpAddInt", true) => proof("LOFT_PLAIN_ADD"),
                        ("OpMinInt", true) => proof("LOFT_PLAIN_SUB"),
                        ("OpMulInt", true) => proof("LOFT_PLAIN_MUL"),
                        ("OpAddInt", false) => format!("{bad} |= LOFT_ADD_OVF({a}, {b}, &{r});"),
                        ("OpMinInt", false) => format!("{bad} |= LOFT_SUB_OVF({a}, {b}, &{r});"),
                        ("OpMulInt", false) => format!("{bad} |= LOFT_MUL_OVF({a}, {b}, &{r});"),
                        ("OpLandInt", _) => plain("&"),
                        ("OpLorInt", _) => plain("|"),
                        _ => plain("^"),
                    };
                    if *proven {
                        crate::rewrite_census::fired("R-Range", 1);
                    }
                    self.line(ind, &line);
                    // An inner result equal to the sentinel reads as null to the operator above;
                    // a result the range proof bounds never is.
                    if i != root && !*proven && !*bounded {
                        self.line(ind, &format!("{bad} |= ({r} == LOFT_NULL);"));
                    }
                    vals.push(r);
                }
            }
        }
        let checked = Self::checked_chain(&nodes, root);
        let name = self.var(var);
        self.line(ind, &format!("if (LOFT_UNLIKELY({bad})) {{"));
        self.line(ind + 1, &format!("{name} = {checked};"));
        self.line(ind, "} else {");
        self.line(ind + 1, &format!("{name} = {};", vals[root]));
        self.line(ind, "}");
        crate::rewrite_census::fired("R-SpecChain", 1);
        Ok(true)
    }

    /// Does `(R-GuardedChain)` own an operator of the chain under `x` (its admitted operators,
    /// by argument-slice address)?  Then the chain is that rule's.
    fn chain_admitted(&self, x: &Value) -> bool {
        if self.plain_chains.is_empty() {
            return false;
        }
        let mut hit = false;
        x.walk(&mut |n| {
            if let Value::Call(_, args) = n {
                let at = args.as_ptr() as usize;
                hit |= self.plain_chains.iter().any(|(set, _)| set.contains(&at));
            }
        });
        hit
    }

    /// The operators of the chain under `x` and how many of them no proof writes plain.
    fn chain_census(&mut self, x: &Value, ops: &mut usize, open: &mut usize) {
        if self.invariant_of(x).is_some() {
            return;
        }
        if let Value::Call(d, args) = x.unspan()
            && args.len() == 2
            && let Some(name) = chain_name(self.cx.data.def(*d).name())
        {
            *ops += 1;
            let bitwise = matches!(name, "OpLandInt" | "OpLorInt" | "OpEorInt");
            if bitwise || !self.range_proves(name, args) {
                *open += 1;
            }
            self.chain_census(&args[0], ops, open);
            self.chain_census(&args[1], ops, open);
        }
    }

    /// Post-order nodes of the chain under `x`, each leaf evaluated into a temp in the checked
    /// chain's order, marked when the proofs cannot show it never null; answers `x`'s node.
    fn chain_nodes(
        &mut self,
        x: &Value,
        nodes: &mut Vec<Node>,
        ind: usize,
    ) -> Result<usize, String> {
        // A subchain an enclosing loop memoises (`(R-Invariant)`) is a leaf: its value is the
        // memo, which `expr` answers.
        if self.invariant_of(x).is_none()
            && let Value::Call(d, args) = x.unspan()
            && args.len() == 2
            && let Some(name) = chain_name(self.cx.data.def(*d).name())
        {
            let proven = !matches!(name, "OpLandInt" | "OpLorInt" | "OpEorInt")
                && self.range_proves(name, args);
            let bounded = matches!(name, "OpLandInt" | "OpLorInt" | "OpEorInt") && {
                let facts = self.nn_facts();
                let over = self.range_override.last().cloned();
                let ranges = over.as_deref().unwrap_or(&facts.1);
                crate::opt::range::range(self.cx.data, &facts.0, ranges, x, 0).is_some()
            };
            let lhs = self.chain_nodes(&args[0], nodes, ind)?;
            let rhs = self.chain_nodes(&args[1], nodes, ind)?;
            nodes.push(Node::Op {
                name,
                lhs,
                rhs,
                proven,
                bounded,
            });
            return Ok(nodes.len() - 1);
        }
        let leaf = if let Value::Int(k) = x.unspan() {
            Node::Leaf(format!("INT64_C({k})"), false)
        } else {
            let e = self.expr(x, ind)?;
            let t = self.temp(super::CType::Int);
            self.line(ind, &format!("{t} = {};", as_int(e)?));
            let may_be_null = !self.never_null(x);
            Node::Leaf(t, may_be_null)
        };
        nodes.push(leaf);
        Ok(nodes.len() - 1)
    }

    /// Is `x` never null under this function's facts: ranged, or non-sentinel?
    pub(super) fn never_null(&mut self, x: &Value) -> bool {
        let facts = self.nn_facts();
        let over = self.range_override.last().cloned();
        let ranges = over.as_deref().unwrap_or(&facts.1);
        crate::opt::range::range(self.cx.data, &facts.0, ranges, x, 0).is_some()
            || crate::opt::non_sentinel::non_sentinel(self.cx.data, &facts.0, x)
    }

    /// The chain as the checked operators write it, over the same leaves.
    fn checked_chain(nodes: &[Node], at: usize) -> String {
        match &nodes[at] {
            Node::Leaf(t, _) => t.clone(),
            Node::Op { name, lhs, rhs, .. } => {
                let f = match *name {
                    "OpAddInt" => "loft_add",
                    "OpMinInt" => "loft_sub",
                    "OpMulInt" => "loft_mul",
                    "OpLandInt" => "loft_and",
                    "OpLorInt" => "loft_or",
                    _ => "loft_xor",
                };
                format!(
                    "{f}({}, {})",
                    Self::checked_chain(nodes, *lhs),
                    Self::checked_chain(nodes, *rhs)
                )
            }
        }
    }
}
