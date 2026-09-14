// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Selection over a name's OVERLOAD SET (@PLN162): which of several definitions a call
//! reaches, decided from the argument types alone.
//!
//! `Disp-Applicable` — a definition takes the call when every argument may satisfy its
//! parameter, which is the parser's own [`Parser::can_convert`] and not a second spelling of
//! it.  `Disp-Specific` — per position, an exact spelling beats a widening (a variant to its
//! enum, `τ` into `τ?`), which beats a lossy discharge (`τ?` into `τ`, `(N-Store)`), which
//! beats any other conversion; one definition is more specific than another when it is no
//! worse at every position and better at one.  `Disp-Select` — the unique most-specific
//! applicable definition.  `Disp-Ambiguous` — two minimal ones that nothing ranks are refused
//! naming both; `Disp-Exhaustive` — none applicable is refused naming what was passed and what
//! is declared.  The nullability routing of `(F-Recv)`'s argument clause runs FIRST
//! ([`crate::data::Data::routed_types`]), so a nullable argument anywhere reaches the `τ?`
//! overload where one is declared.
//!
//! A name with no overload set — one definition, or a bound holder's stubs — is not decided
//! here at all: the caller falls through to [`crate::data::Data::select`], today's ladder,
//! which is why every program that compiled before overload sets existed is untouched.

use crate::data::{Argument, Attribute, DefType, Deps, Type, Value};
use crate::diagnostics::diagnostic_format;
use crate::parser::{Function, Parser, v_block, v_if};

/// How far a call's argument is from a parameter's declared type, per position.
/// Smaller is more specific.
type Rank = u8;
const EXACT: Rank = 0;
const WIDENED: Rank = 1;
const LOSSY: Rank = 2;
const CONVERTED: Rank = 3;

/// What selection over an overload set answered.
pub(crate) enum Selection {
    /// The unique most-specific applicable definition.
    One(u32),
    /// Two or more minimal definitions that nothing ranks.
    Ambiguous(Vec<u32>),
    /// An overload set exists and no definition takes the call.
    NoneApplicable,
    /// The name has no overload set (or an argument no spelling): not decided here.
    NotDecidable,
}

impl Parser {
    /// The rank of passing an argument of type `arg` to a parameter of type `param`, or
    /// `None` when the argument cannot satisfy the parameter.
    fn dispatch_rank(&mut self, arg: &Type, param: &Type) -> Option<Rank> {
        let (arg_opt, param_opt) = (
            matches!(arg, Type::Optional(_)),
            matches!(param, Type::Optional(_)),
        );
        let (a, p) = (arg.base(), param.base());
        let same = self.data.type_spelling(a).is_some()
            && self.data.type_spelling(a) == self.data.type_spelling(p);
        // The base relation first, then the nullability step on top of it.
        let base: Rank = if same {
            EXACT
        } else if let (Type::Reference(v_nr, _), Type::Enum(e_nr, _, _)) =
            (arg.base(), param.base())
            && self.data.def_type(*v_nr) == DefType::EnumValue
            && self.data.def(*v_nr).parent() == *e_nr
        {
            // @FR-C-Var — a variant satisfies its enum: the one widening the enum lattice has.
            WIDENED
        } else if self.can_convert(a, p) {
            CONVERTED
        } else {
            return None;
        };
        let step: Rank = match (arg_opt, param_opt) {
            (false, true) => WIDENED, // (N-Intro): a present value into a nullable slot, free
            (true, false) => LOSSY,   // (N-Store): a nullable value into a dense slot, warned
            _ => EXACT,
        };
        Some(base.saturating_add(step))
    }

    /// Every overload of `name` that takes the routed argument types, with its rank vector,
    /// or `None` when the name has no overload set or an argument has no spelling.
    fn ranked_overloads(
        &mut self,
        source: u16,
        name: &str,
        routed: &[Type],
    ) -> Option<Vec<(u32, Vec<Rank>)>> {
        let main = self.data.source_nr(source, name);
        if main == u32::MAX || self.data.def_type(main) != DefType::Dynamic {
            return None;
        }
        if routed.iter().any(|t| self.data.type_spelling(t).is_none()) {
            return None;
        }
        let routines: Vec<u32> = self
            .data
            .def(main)
            .attributes
            .iter()
            .filter_map(|a| match a.typedef.base() {
                Type::Routine(r) => Some(*r),
                _ => None,
            })
            .collect();
        let mut ranked = Vec::new();
        'routine: for r in routines {
            let declared: Vec<Type> = self
                .data
                .def(r)
                .attributes
                .iter()
                .filter(|p| !p.hidden)
                .map(|p| p.typedef.clone())
                .collect();
            if routed.len() > declared.len() {
                continue;
            }
            // A trailing parameter is admitted when it has a default (the one optionality a
            // definition carries — owner, 2026-09-14).
            let trailing_defaulted = self
                .data
                .def(r)
                .attributes
                .iter()
                .filter(|p| !p.hidden)
                .skip(routed.len())
                .all(|p| p.value != crate::data::Value::Null);
            if !trailing_defaulted {
                continue;
            }
            let mut ranks = Vec::with_capacity(routed.len());
            for (arg, param) in routed.iter().zip(&declared) {
                match self.dispatch_rank(arg, param) {
                    Some(k) => ranks.push(k),
                    None => continue 'routine,
                }
            }
            ranked.push((r, ranks));
        }
        Some(ranked)
    }

    /// `Disp-Select` over an overload set for the routed argument types.
    pub(crate) fn select_overload(
        &mut self,
        source: u16,
        name: &str,
        routed: &[Type],
    ) -> Selection {
        let Some(ranked) = self.ranked_overloads(source, name, routed) else {
            return Selection::NotDecidable;
        };
        if ranked.is_empty() {
            return Selection::NoneApplicable;
        }
        // The minimal elements of the pointwise order: a definition no other applicable one is
        // strictly better than.
        let dominated = |a: &[Rank], b: &[Rank]| -> bool {
            // b is strictly better than a
            a.iter().zip(b).all(|(x, y)| y <= x) && a.iter().zip(b).any(|(x, y)| y < x)
        };
        let minimal: Vec<u32> = ranked
            .iter()
            .filter(|(_, ra)| !ranked.iter().any(|(_, rb)| dominated(ra, rb)))
            .map(|(r, _)| *r)
            .collect();
        match minimal.as_slice() {
            [one] => Selection::One(*one),
            _ => Selection::Ambiguous(minimal),
        }
    }

    /// The two refusals selection can end in, worded once for both call spellings.
    pub(crate) fn report_selection(
        &mut self,
        name: &str,
        given: &[Type],
        sel: &Selection,
        at: Option<&crate::lexer::Position>,
    ) {
        let shown: Vec<String> = given.iter().map(|t| t.source_name(&self.data)).collect();
        let text = match sel {
            Selection::Ambiguous(taken) => {
                let by: Vec<String> = taken
                    .iter()
                    .map(|&r| self.data.overload_signature(name, r))
                    .collect();
                format!(
                    "`{name}({})` is ambiguous — it is taken by {} and nothing ranks them; give the call the arguments that pick one, or drop one definition",
                    shown.join(", "),
                    by.join(" and ")
                )
            }
            Selection::NoneApplicable => {
                let bare = self.data.def_nr(name);
                let declared: Vec<String> = self
                    .data
                    .def(bare)
                    .attributes
                    .iter()
                    .filter_map(|a| match a.typedef.base() {
                        Type::Routine(r) => Some(*r),
                        _ => None,
                    })
                    .map(|r| self.data.overload_signature(name, r))
                    .collect();
                format!(
                    "no definition of `{name}` takes ({}) — declared: {}",
                    shown.join(", "),
                    declared.join(", ")
                )
            }
            _ => return,
        };
        match at {
            Some(pos) => {
                crate::diagnostic_at!(self.lexer, pos, crate::diagnostics::Level::Error, "{text}")
            }
            None => crate::diagnostic!(self.lexer, crate::diagnostics::Level::Error, "{text}"),
        }
    }
}

impl Parser {
    /// The positions of `routed` held at a DENSE struct-enum for which the set names one of
    /// that enum's variants in that position — the positions whose runtime variant decides
    /// the call (`Disp-Dynamic`).  Empty when the static selection is the whole answer, which
    /// is every program that compiled before overload sets existed.
    fn dynamic_positions(&self, main: u32, routed: &[Type]) -> Vec<(usize, u32)> {
        let mut out = Vec::new();
        for (i, t) in routed.iter().enumerate() {
            // An enum VALUE has two spellings — `Enum(e, true, …)` for a local held at the
            // enum, `Reference(e, …)` for an element of a `vector<E>` — and a position test
            // keyed on one of them is blind to the other (the loop over a collection is the
            // design's very shape).  `can_convert` admits both into an enum slot.
            // A NULLABLE enum position stays static: null has no variant, and the `τ?`
            // definition is what a null reaches.  Asked first, as the nullability question
            // it is; the shape below is then read through the wrapper it cannot have.
            if matches!(t, Type::Optional(_)) {
                continue;
            }
            let e_nr = match t.base() {
                Type::Enum(e, true, _) => *e,
                Type::Reference(e, _)
                    if matches!(self.data.def(*e).returned().base(), Type::Enum(_, true, _)) =>
                {
                    *e
                }
                _ => continue,
            };
            let e_nr = &e_nr;
            let names_a_variant = self.data.def(main).attributes.iter().any(|a| {
                let Type::Routine(r) = a.typedef.base() else {
                    return false;
                };
                self.data
                    .def(*r)
                    .attributes
                    .iter()
                    .filter(|p| !p.hidden)
                    .nth(i)
                    .is_some_and(|p| match p.typedef.base() {
                        Type::Reference(v, _) => {
                            self.data.def_type(*v) == DefType::EnumValue
                                && self.data.def(*v).parent() == *e_nr
                        }
                        _ => false,
                    })
            });
            if names_a_variant {
                out.push((i, *e_nr));
            }
        }
        out
    }

    /// `Disp-Dynamic` (@PLN162 step 13): the synthesised dispatcher for a call of `name` whose
    /// routed types hold an enum at a position the set decides by variant.  It is the
    /// canonical `match` of `Disp-Match-Equiv`, built by the compiler: an ordinary function
    /// over the routed types whose body tests the discriminant at each dynamic position and,
    /// for every variant tuple, calls the definition `Disp-Select` picks for those types — so
    /// the runtime answer equals the static one for the runtime types, deterministically, and
    /// both backends compile it as they compile any function.  A tuple no definition takes,
    /// or two take without ranking, is refused at compile time naming the tuple
    /// (`Disp-Exhaustive` / `Disp-Ambiguous` over the closed enum), and `None` is then the
    /// answer: the diagnostic is the whole of it.  Built once per (name, spelling), on pass 2
    /// only — the set is complete only then, and pass 1's static placeholder is never emitted.
    /// The shape is @F20's synthesised enum dispatcher generalised from one position to any.
    pub(crate) fn dynamic_dispatcher(
        &mut self,
        source: u16,
        name: &str,
        routed: &[Type],
    ) -> Option<u32> {
        if self.first_pass {
            return None;
        }
        let main = self.data.source_nr(source, name);
        if main == u32::MAX || self.data.def_type(main) != DefType::Dynamic {
            return None;
        }
        let positions = self.dynamic_positions(main, routed);
        if positions.is_empty() {
            return None;
        }
        let spelling = self.data.full_spelling(routed.iter())?;
        let dyn_name = format!("{name}__dyn_{spelling}");
        let existing = self.data.def_nr(&format!("n_{dyn_name}"));
        if existing != u32::MAX {
            return Some(existing);
        }
        // Every variant tuple over the dynamic positions, each with the definition
        // `Disp-Select` picks for it — computed before anything is built, so a refused leaf
        // builds nothing.
        let choices: Vec<Vec<u32>> = positions
            .iter()
            .map(|(_, e)| {
                self.data
                    .definitions
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.def_type == DefType::EnumValue && d.parent() == *e)
                    .map(|(i, _)| i as u32)
                    .collect()
            })
            .collect();
        let mut tuples: Vec<Vec<u32>> = vec![Vec::new()];
        for c in &choices {
            tuples = tuples
                .into_iter()
                .flat_map(|t| {
                    c.iter().map(move |v| {
                        let mut n = t.clone();
                        n.push(*v);
                        n
                    })
                })
                .collect();
        }
        let mut leaves: Vec<(Vec<u32>, u32, Vec<Type>)> = Vec::with_capacity(tuples.len());
        for tuple in &tuples {
            let mut leaf_types = routed.to_vec();
            for ((pos, _), v) in positions.iter().zip(tuple) {
                leaf_types[*pos] = Type::Reference(*v, Deps::none());
            }
            match self.select_overload(source, name, &leaf_types) {
                Selection::One(d) => leaves.push((tuple.clone(), d, leaf_types)),
                sel @ (Selection::Ambiguous(_) | Selection::NoneApplicable) => {
                    self.report_dynamic_leaf(name, routed, &leaf_types, &sel);
                    self.reported_dynamic_refusal = true;
                    return None;
                }
                Selection::NotDecidable => return None,
            }
        }
        let ret = self.data.def(leaves[0].1).returned().clone();
        let argument = |name: String, typedef: Type, default: Value| Argument {
            name,
            typedef,
            default,
            constant: false,
            ref_pos: (0, 0),
            const_pos: (0, 0),
        };
        // A dynamic position is declared at the ENUM type whichever spelling the call held it
        // in, so the tag read is on an enum-typed variable as @F20's is; the other positions
        // keep the call's types.
        let mut args: Vec<Argument> = routed
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let tp = match positions.iter().find(|(pos, _)| *pos == i) {
                    Some((_, e)) => Type::Enum(*e, true, Deps::none()),
                    None => t.clone(),
                };
                argument(format!("d{i}"), tp, Value::Null)
            })
            .collect();
        // The hidden buffers (a text return's accumulator) of the first leaf definition that
        // carries any, forwarded only to the leaves that declare them — @F20's rule.
        let hidden: Vec<Attribute> = leaves
            .iter()
            .map(|(_, d, _)| {
                self.data
                    .def(*d)
                    .attributes
                    .iter()
                    .filter(|a| a.hidden)
                    .cloned()
                    .collect::<Vec<Attribute>>()
            })
            .find(|h| !h.is_empty())
            .unwrap_or_default();
        for a in &hidden {
            args.push(argument(a.name.clone(), a.typedef.clone(), a.value.clone()));
        }
        // The dispatcher is built as its own function: the caller's parsing context is put
        // aside and restored, whatever the outcome.
        let saved_context = self.context;
        let file = self.lexer.pos().file.clone();
        let saved_vars = std::mem::replace(&mut self.vars, Function::new(&dyn_name, &file));
        let saved_expected = std::mem::replace(&mut self.expected, Type::Unknown(0));
        let fn_nr = self.data.add_fn(&mut self.lexer, &dyn_name, &args);
        let built = if fn_nr == u32::MAX {
            None
        } else {
            self.data.mark_synthetic(fn_nr, "dynamic_dispatcher");
            self.context = fn_nr;
            self.data.set_returned(fn_nr, ret.clone());
            for a in &args {
                let v = self.create_var(&a.name, &a.typedef);
                if v != u16::MAX {
                    self.vars.become_argument(v);
                }
            }
            let visible: Vec<u16> = (0..routed.len())
                .map(|i| self.vars.var(&format!("d{i}")))
                .collect();
            let hidden_vars: Vec<(Value, Type)> = hidden
                .iter()
                .map(|a| (Value::Var(self.vars.var(&a.name)), a.typedef.clone()))
                .collect();
            let mut ls = Vec::new();
            for (tuple, d, leaf_types) in &leaves {
                let mut cond: Option<Value> = None;
                for ((pos, _), v) in positions.iter().zip(tuple) {
                    let disc = match self.data.def(*v).attributes().first().map(|a| &a.value) {
                        Some(Value::Enum(nr, _)) => i32::from(*nr),
                        _ => 0,
                    };
                    let get_enum =
                        self.cl("OpGetEnum", &[Value::Var(visible[*pos]), Value::Int(0)]);
                    let get_int = self.cl("OpConvIntFromEnum", &[get_enum]);
                    let test = self.cl("OpEqInt", &[get_int, Value::Int(disc)]);
                    cond = Some(match cond {
                        None => test,
                        Some(c) => v_if(c, test, Value::Boolean(false)),
                    });
                }
                let mut call_args: Vec<Value> = visible.iter().map(|v| Value::Var(*v)).collect();
                // The leaf's static types: the variant at each dynamic position, which is what
                // the tag test just established, so the argument check sees no conversion.
                let mut call_types: Vec<Type> = leaf_types.clone();
                if self.data.def(*d).attributes.iter().any(|a| a.hidden) {
                    for (v, t) in &hidden_vars {
                        call_args.push(v.clone());
                        call_types.push(t.clone());
                    }
                }
                let at = self.lexer.pos().clone();
                let arg_pos = vec![at.clone(); call_args.len()];
                let mut code = Value::Null;
                self.call_nr(
                    &mut code,
                    *d,
                    &call_args,
                    &call_types,
                    true,
                    &arg_pos,
                    Some(&at),
                );
                let ret_call = v_block(vec![Value::Return(Box::new(code))], Type::Void, "ret");
                ls.push(v_if(
                    cond.expect("a dynamic position"),
                    ret_call,
                    Value::Null,
                ));
            }
            // Unreachable when every tuple has a leaf, which the refusals above guarantee;
            // the typed-null return keeps both backends' bodies well-formed.
            ls.push(Value::Return(Box::new(Value::Null)));
            self.data.definitions[fn_nr as usize].code = v_block(ls, ret, "dynamic_fn");
            self.data.definitions[fn_nr as usize].variables = self.vars.clone();
            Some(fn_nr)
        };
        self.context = saved_context;
        self.vars = saved_vars;
        self.expected = saved_expected;
        built
    }
}

impl Parser {
    /// A dynamic site refused at one variant tuple, worded from the call the author WROTE —
    /// the static types — down to the tuple the closed enumeration could not decide, which
    /// the author never spelled.
    fn report_dynamic_leaf(&mut self, name: &str, routed: &[Type], leaf: &[Type], sel: &Selection) {
        let shown = |data: &crate::data::Data, ts: &[Type]| -> String {
            ts.iter()
                .map(|t| t.source_name(data))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let call = shown(&self.data, routed);
        let tuple = shown(&self.data, leaf);
        let text = match sel {
            Selection::Ambiguous(taken) => {
                let by: Vec<String> = taken
                    .iter()
                    .map(|&r| self.data.overload_signature(name, r))
                    .collect();
                format!(
                    "`{name}({call})` is ambiguous at ({tuple}) — that pair is taken by {} and nothing ranks them; give the call the arguments that pick one, or drop one definition",
                    by.join(" and ")
                )
            }
            Selection::NoneApplicable => {
                let bare = self.data.def_nr(name);
                let declared: Vec<String> = self
                    .data
                    .def(bare)
                    .attributes
                    .iter()
                    .filter_map(|a| match a.typedef.base() {
                        Type::Routine(r) => Some(*r),
                        _ => None,
                    })
                    .map(|r| self.data.overload_signature(name, r))
                    .collect();
                format!(
                    "`{name}({call})` has no definition for the pair ({tuple}) — declared: {}; add one, or a definition at the enum",
                    declared.join(", ")
                )
            }
            _ => return,
        };
        crate::diagnostic!(self.lexer, crate::diagnostics::Level::Error, "{text}");
    }
}
