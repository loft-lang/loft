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
        for r in routines {
            if let Some(ranks) = self.definition_ranks(r, routed) {
                ranked.push((r, ranks));
            }
        }
        Some(ranked)
    }

    /// How far each routed argument is from definition `r`'s parameter, or `None` when `r`
    /// does not take the call (`Disp-Applicable`): more arguments than parameters, a trailing
    /// parameter with no default, or an argument its parameter cannot accept.
    fn definition_ranks(&mut self, r: u32, routed: &[Type]) -> Option<Vec<Rank>> {
        let declared: Vec<Type> = self
            .data
            .def(r)
            .attributes
            .iter()
            .filter(|p| !p.hidden)
            .map(|p| p.typedef.clone())
            .collect();
        if routed.len() > declared.len() {
            return None;
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
            return None;
        }
        routed
            .iter()
            .zip(&declared)
            .map(|(arg, param)| self.dispatch_rank(arg, param))
            .collect()
    }

    /// Does a definition the PROGRAM declares — anything outside the stdlib prelude, a
    /// library's included — take a call of `name` with these argument types?  A compiler
    /// special case for a call name (`sort` over a vector, `type_of`, `assert`, …) is a
    /// candidate of last resort: it is taken only when this answers `false`, so no special
    /// name limits what a program may define, and the call reaches the definition the type
    /// system selects.  Asked the way the ordinary call asks: the name's overload set first,
    /// then the ladder.
    pub(crate) fn program_definition_applies(
        &mut self,
        source: u16,
        name: &str,
        types: &[Type],
    ) -> bool {
        let routed = self.data.routed_types(types);
        let d = match self.select_overload(source, name, &routed) {
            Selection::One(d) => d,
            // Several of the set's definitions tie: the ordinary call reports that.
            Selection::Ambiguous(_) => return true,
            Selection::NoneApplicable => return false,
            Selection::NotDecidable => self.data.select_fn(source, name, types),
        };
        d != u32::MAX
            && self.data.def_type(d) == DefType::Function
            && !crate::portable_path::is_stdlib_source(&self.data.def(d).position().file)
            && self.definition_ranks(d, &routed).is_some()
    }

    /// Does the PROGRAM declare a function named `name` at all?  Asked where a special form
    /// is recognised before its argument is parsed (`sizeof(…)`, `type_name(…)`,
    /// `typedef(…)`), so the argument cannot yet say which definition the call reaches: a
    /// declared function sends the call down the ordinary path, where
    /// [`Self::program_definition_applies`] decides by the argument's type.
    pub(crate) fn program_declares_fn(&self, name: &str) -> bool {
        self.data.definitions.iter().any(|d| {
            d.def_type == DefType::Function
                && d.original_name().as_str() == name
                && !crate::portable_path::is_stdlib_source(&d.position().file)
        })
    }

    /// Is the next argument a TYPE name closing the call — `sizeof(Roster)` — which no
    /// function can take, so the special form keeps it whatever the program declares?
    /// Reads ahead and restores the lexer.
    pub(crate) fn next_is_type_name_argument(&mut self) -> bool {
        let lnk = self.lexer.link();
        let is_type = match self.lexer.has_identifier() {
            Some(id) => {
                let d_nr = self.data.def_nr(&id);
                d_nr != u32::MAX
                    && !matches!(
                        self.data.def_type(d_nr),
                        DefType::EnumValue
                            | DefType::Function
                            | DefType::Dynamic
                            | DefType::Generic
                            | DefType::Unknown
                    )
                    && self.lexer.has_token(")")
            }
            None => false,
        };
        self.lexer.revert(lnk);
        is_type
    }

    /// Does the current program source FILE declare a function named `name` that pass 1 has
    /// not reached yet — one BELOW this call?  Pass 1 meets a call before the definitions
    /// that follow it, so [`Self::program_definition_applies`] cannot see them — and a special
    /// form answering ITS type on pass 1 while the program's definition answers another on
    /// pass 2 refuses the program (*"cannot change type from void to text"*).  The file's
    /// `fn <name>` declarations are counted by a lexical scan and compared with the ones of
    /// this file already registered: while some are still ahead, the special form waits for
    /// pass 2, exactly as any call of a later definition waits.  Once all are registered the
    /// exact type check decides on pass 1 too, so both passes answer alike.  A false positive
    /// — the words in a comment, a `fn(…)` type — only defers the special form to pass 2;
    /// the stdlib's own files are never scanned.
    pub(crate) fn file_has_pending_fn(&mut self, name: &str) -> bool {
        let file = self.lexer.pos().file.clone();
        if crate::portable_path::is_stdlib_source(&file) {
            return false;
        }
        if !self.declared_fn_names.contains_key(&file) {
            let text = self
                .lexer
                .virtual_source(&file)
                .map_or_else(|| Self::read_source(&file), str::to_string);
            let mut counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let mut after_fn = false;
            for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                if word.is_empty() {
                    continue;
                }
                if after_fn {
                    *counts.entry(word.to_string()).or_default() += 1;
                }
                after_fn = word == "fn";
            }
            self.declared_fn_names.insert(file.clone(), counts);
        }
        let declared = self
            .declared_fn_names
            .get(&file)
            .and_then(|counts| counts.get(name))
            .copied()
            .unwrap_or(0);
        if declared == 0 {
            return false;
        }
        let reached = self
            .data
            .definitions
            .iter()
            .filter(|d| {
                matches!(d.def_type, DefType::Function | DefType::Generic)
                    && d.original_name().as_str() == name
                    && d.position().file == file
            })
            .count();
        declared > reached
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
    /// The positions of `routed` held at a struct-enum for which the set names one of that
    /// enum's variants in that position — the positions whose runtime variant decides the
    /// call (`Disp-Dynamic`).  Empty when the static selection is the whole answer, which is
    /// every program that compiled before overload sets existed.  A NULLABLE position counts
    /// only when `nullable` says a null has somewhere to go (D-disp-2: the definition the
    /// call's static types select, which is what a null reached before).
    fn dynamic_positions(&self, main: u32, routed: &[Type], nullable: bool) -> Vec<(usize, u32)> {
        let mut out = Vec::new();
        for (i, t) in routed.iter().enumerate() {
            // An enum VALUE has two spellings — `Enum(e, true, …)` for a local held at the
            // enum, `Reference(e, …)` for an element of a `vector<E>` — and a position test
            // keyed on one of them is blind to the other (the loop over a collection is the
            // design's very shape).  `can_convert` admits both into an enum slot.
            // Nullability is asked first, as the question it is; the shape below is then read
            // through the wrapper.
            if matches!(t, Type::Optional(_)) && !nullable {
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
    /// `fallback` is the definition the call's static types select, when they select one: it
    /// is what a null at a nullable enum position reaches.
    pub(crate) fn dynamic_dispatcher(
        &mut self,
        source: u16,
        name: &str,
        routed: &[Type],
        fallback: Option<u32>,
    ) -> Option<u32> {
        if self.first_pass {
            return None;
        }
        let main = self.data.source_nr(source, name);
        if main == u32::MAX || self.data.def_type(main) != DefType::Dynamic {
            return None;
        }
        let positions = self.dynamic_positions(main, routed, fallback.is_some());
        // `Disp-World` (@PLN162 step 14): in the OPEN profile a static site calls a
        // per-spelling stub too — the one direct call `Disp-Select` picked, as its own
        // function — so that an overload added mid-run has one place per spelling to rebuild
        // and swap; the closed profile keeps step 11's direct call.
        if positions.is_empty() && !(open_world() && self.stub_admissible(main)) {
            return None;
        }
        let spelling = self.data.full_spelling(routed.iter())?;
        let kind = if positions.is_empty() { "sel" } else { "dyn" };
        let dyn_name = format!("{name}__{kind}_{spelling}");
        let existing = self.data.def_nr(&format!("n_{dyn_name}"));
        if existing != u32::MAX {
            return Some(existing);
        }
        self.build_specialisation(source, name, routed, &positions, &dyn_name, fallback)
    }

    /// Can a static call of the set `main` take the open profile's per-spelling stub?  Only a
    /// set the reload host can GROW is worth one: the stdlib is never watched, and a set with
    /// a generic member, or a call inside a generic body, is instantiated per use — a stub
    /// over a type parameter is the wrong shape (measured: the stdlib's `len(both: vector)`
    /// refused its own stub, *expected vector<T>, got vector<T>*).
    fn stub_admissible(&self, main: u32) -> bool {
        if crate::portable_path::is_stdlib_source(&self.data.def(main).position().file) {
            return false;
        }
        if self.context == u32::MAX || self.data.def_type(self.context) == DefType::Generic {
            return false;
        }
        self.data
            .def(main)
            .attributes
            .iter()
            .all(|a| match a.typedef.base() {
                Type::Routine(r) => self.data.def_type(*r) == DefType::Function,
                _ => true,
            })
    }

    /// The body of a specialisation of `name` for the routed types: one leaf per variant
    /// tuple over `positions`, each the definition `Disp-Select` picks for that tuple, under
    /// the discriminant tests that reach it — or, with no dynamic position (the open profile's
    /// static stub), the one leaf as the whole body.  Registered as `dyn_name`.  `fallback` is
    /// the static selection, the leaf a null at a nullable dynamic position reaches.
    fn build_specialisation(
        &mut self,
        source: u16,
        name: &str,
        routed: &[Type],
        positions: &[(usize, u32)],
        dyn_name: &str,
        fallback: Option<u32>,
    ) -> Option<u32> {
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
                    // A static stub has nothing to enumerate: its caller already holds the
                    // verdict for these types, and the ladder answers a name no set decides.
                    if positions.is_empty() {
                        return None;
                    }
                    self.report_dynamic_leaf(name, routed, &leaf_types, &sel);
                    self.reported_dynamic_refusal = true;
                    return None;
                }
                Selection::NotDecidable => return None,
            }
        }
        let ret = self.data.def(leaves[0].1).returned().clone();
        // `Disp-Dynamic` over a NULLABLE enum position (D-disp-2): a present value's variant
        // decides exactly as a dense one's does, and a null — which has no variant — reaches
        // the definition the call's static types select, which is what it reached before the
        // position was dynamic.  Only a call that has a static selection makes a nullable
        // position dynamic (`dynamic_positions`), so a null always has that leaf.
        let null_leaf = fallback.filter(|_| {
            positions
                .iter()
                .any(|(pos, _)| matches!(routed[*pos], Type::Optional(_)))
        });
        // One dispatcher has ONE return type, so the definitions it chooses between must agree
        // on it — DESIGN.md's open question 4, at the one place it cannot stay open.  A
        // `Fireball` leaf answering `integer` beside an enum-level leaf answering `text` read the
        // text through the integer's frame on the interpreter and did not compile on `--native`.
        // A static site calls one definition and is not this question.
        let differs = leaves
            .iter()
            .map(|(_, d, _)| *d)
            .chain(null_leaf)
            .find(|d| {
                self.data.type_spelling(self.data.def(*d).returned())
                    != self.data.type_spelling(&ret)
            });
        if let Some(other) = differs {
            self.report_mixed_returns(name, routed, leaves[0].1, other);
            self.reported_dynamic_refusal = true;
            return None;
        }
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
        // keep the call's types.  Where a null can arrive, a nullable position takes the
        // nullability the static selection declares there, so the call into the dispatcher is
        // checked exactly as the direct call was: the `(N-Store)` warning stays at each call
        // site instead of moving, once, into this body.
        let declared: Vec<Type> = routed
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let dynamic = positions
                    .iter()
                    .find(|(pos, _)| *pos == i)
                    .map(|(_, e)| Type::Enum(*e, true, Deps::none()));
                match null_leaf {
                    Some(fb) if matches!(t, Type::Optional(_)) => {
                        let base = dynamic.unwrap_or_else(|| t.base().clone());
                        let wants_null = self
                            .data
                            .def(fb)
                            .attributes
                            .iter()
                            .filter(|a| !a.hidden)
                            .nth(i)
                            .is_some_and(|a| matches!(a.typedef, Type::Optional(_)));
                        if wants_null {
                            Type::optional(base)
                        } else {
                            base
                        }
                    }
                    _ => dynamic.unwrap_or_else(|| t.clone()),
                }
            })
            .collect();
        let mut args: Vec<Argument> = declared
            .iter()
            .enumerate()
            .map(|(i, tp)| argument(format!("d{i}"), tp.clone(), Value::Null))
            .collect();
        // C124 — a dispatcher position is `const` exactly when EVERY definition it can choose
        // declares that parameter `const`: then a const argument reaches the dispatcher, and the
        // dispatcher hands it on to const parameters only.  One plain member makes the position
        // plain, and a const argument is refused at the call as it would be by that member.
        for (i, arg) in args.iter_mut().enumerate() {
            arg.constant = leaves.iter().map(|(_, d, _)| *d).chain(null_leaf).all(|d| {
                self.data
                    .def(d)
                    .attributes
                    .iter()
                    .filter(|a| !a.hidden)
                    .nth(i)
                    .is_some_and(|a| a.value_const)
            });
        }
        // The hidden buffers (a text return's accumulator) of the first leaf definition that
        // carries any, forwarded only to the leaves that declare them — @F20's rule.
        let hidden: Vec<Attribute> = leaves
            .iter()
            .map(|(_, d, _)| *d)
            .chain(null_leaf)
            .map(|d| {
                self.data
                    .def(d)
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
        let saved_vars = std::mem::replace(&mut self.vars, Function::new(dyn_name, &file));
        let saved_expected = std::mem::replace(&mut self.expected, Type::Unknown(0));
        let fn_nr = self.data.add_fn(&mut self.lexer, dyn_name, &args);
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
            for (tuple, d, _) in &leaves {
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
                // The leaf's static types: the variant at each dynamic position, which is what
                // the tag test just established, so the argument check sees no conversion.
                let mut call_types = declared.clone();
                for ((pos, _), v) in positions.iter().zip(tuple) {
                    call_types[*pos] = Type::Reference(*v, Deps::none());
                }
                let code = self.specialisation_call(*d, call_types, &visible, &hidden_vars);
                match cond {
                    Some(c) => {
                        let ret_call =
                            v_block(vec![Value::Return(Box::new(code))], Type::Void, "ret");
                        ls.push(v_if(c, ret_call, Value::Null));
                    }
                    // The static stub: no tag to test, the one leaf is the body.
                    None => ls.push(Value::Return(Box::new(code))),
                }
            }
            if let Some(fb) = null_leaf {
                // Every tag test failed, which only a null can make happen: the static
                // selection's definition, called with the dispatcher's own parameter types.
                let code = self.specialisation_call(fb, declared.clone(), &visible, &hidden_vars);
                ls.push(Value::Return(Box::new(code)));
            } else if !positions.is_empty() {
                // Unreachable when every tuple has a leaf, which the refusals above
                // guarantee; the typed-null return keeps both backends' bodies well-formed.
                ls.push(Value::Return(Box::new(Value::Null)));
            }
            self.data.definitions[fn_nr as usize].code = v_block(ls, ret, "dynamic_fn");
            self.data.definitions[fn_nr as usize].variables = self.vars.clone();
            Some(fn_nr)
        };
        self.context = saved_context;
        self.vars = saved_vars;
        self.expected = saved_expected;
        built
    }

    /// One leaf of a specialisation: a call of `d` with the dispatcher's own visible
    /// parameters under `call_types`.  An omitted defaulted parameter takes its default here,
    /// exactly as at a direct call, BEFORE the dispatcher's buffers follow: the call is built
    /// positionally, and appended straight after the supplied arguments a forwarded buffer
    /// landed in the omitted parameter's slot (measured: *expected integer, got &text on
    /// argument 2*, on a set with `k: integer = 7`).  The buffers go only to a leaf that
    /// declares them.
    fn specialisation_call(
        &mut self,
        d: u32,
        mut call_types: Vec<Type>,
        visible: &[u16],
        hidden_vars: &[(Value, Type)],
    ) -> Value {
        let supplied = call_types.len();
        let mut call_args: Vec<Value> = visible.iter().map(|v| Value::Var(*v)).collect();
        let omitted: Vec<(Value, Type)> = self
            .data
            .def(d)
            .attributes
            .iter()
            .filter(|a| !a.hidden)
            .skip(supplied)
            .map(|a| (a.value.clone(), a.typedef.clone()))
            .collect();
        for (v, t) in omitted {
            call_args.push(v);
            call_types.push(t);
        }
        if self.data.def(d).attributes.iter().any(|a| a.hidden) {
            for (v, t) in hidden_vars {
                call_args.push(v.clone());
                call_types.push(t.clone());
            }
        }
        let at = self.lexer.pos().clone();
        let arg_pos = vec![at.clone(); call_args.len()];
        let mut code = Value::Null;
        self.call_nr(
            &mut code,
            d,
            &call_args,
            &call_types,
            true,
            &arg_pos,
            Some(&at),
        );
        code
    }
}

/// Is this run the OPEN profile (`Disp-World`, @PLN162 step 14) — the one whose method sets
/// grow while the program runs?  It is `LOFT_LIVE_RELOAD=1`, the same switch that arms tier-0
/// live reload (`live_reload.rs`), read once: the running program and the reload host's shadow
/// session must lower a call the same way, and both read it here.  Off, the lowering is the
/// closed profile's direct call and nothing in this file is reached for a static site.
pub(crate) fn open_world() -> bool {
    static OPEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OPEN.get_or_init(|| std::env::var("LOFT_LIVE_RELOAD").is_ok_and(|v| v != "0"))
}

impl Parser {
    /// `Disp-World` (@PLN162 step 14): the specialisation `old` — a `__sel_` stub or a
    /// `__dyn_` dispatcher — built AGAIN in world `world`, as a new def `<old>__w<world>` over
    /// the same routed types with every leaf selected from the set as it is now, for the
    /// reload host to swap in behind the old def's number.  `None` when the new world cannot
    /// take it: a tuple the set no longer decides is reported exactly as at a dynamic site,
    /// and the host then refuses the add that caused it.
    pub(crate) fn rebuild_specialisation(&mut self, old: u32, world: u32) -> Option<u32> {
        let def = self.data.def(old);
        let full = def.name.clone();
        let source = def.source;
        // The routed positions are the `d<i>` parameters; the buffers a text return forwards
        // (`__work_…`, `___tret_…`) are the specialisation's own and are minted again.
        let mut routed: Vec<Type> = def
            .attributes
            .iter()
            .filter(|a| !a.hidden && !a.name.starts_with("__"))
            .map(|a| a.typedef.clone())
            .collect();
        let bare = full.strip_prefix("n_")?;
        let (name, spelling) = bare
            .split_once("__dyn_")
            .or_else(|| bare.split_once("__sel_"))?;
        let name = name.to_string();
        // A nullable position may be DECLARED dense (it mirrors the static selection, D-disp-2);
        // what the call routed is in the spelling the specialisation is named by.
        let parts: Vec<&str> = spelling.split('#').collect();
        if parts.len() == routed.len() {
            for (t, part) in routed.iter_mut().zip(parts) {
                if part.ends_with('?') && !matches!(t, Type::Optional(_)) {
                    *t = Type::optional(t.clone());
                }
            }
        }
        let main = self.data.source_nr(source, &name);
        if main == u32::MAX || self.data.def_type(main) != DefType::Dynamic {
            return None;
        }
        let fallback = match self.select_overload(source, &name, &routed) {
            Selection::One(d) => Some(d),
            _ => None,
        };
        let positions = self.dynamic_positions(main, &routed, fallback.is_some());
        let dyn_name = format!("{bare}__w{world}");
        self.build_specialisation(source, &name, &routed, &positions, &dyn_name, fallback)
    }
}

impl Parser {
    /// A dynamic site refused because two of the definitions it chooses between return
    /// different types: the dispatcher is one function and has one.
    fn report_mixed_returns(&mut self, name: &str, routed: &[Type], one: u32, other: u32) {
        let call = routed
            .iter()
            .map(|t| t.source_name(&self.data))
            .collect::<Vec<_>>()
            .join(", ");
        let shown = |data: &crate::data::Data, d: u32| {
            format!(
                "{} -> {}",
                data.overload_signature(name, d),
                data.def(d).returned().source_name(data)
            )
        };
        let (a, b) = (shown(&self.data, one), shown(&self.data, other));
        crate::diagnostic!(
            self.lexer,
            crate::diagnostics::Level::Error,
            "`{name}({call})` is decided by the runtime variant, but its definitions return different types — {a} and {b}; give them one return type, or call with a value held at the variant"
        );
    }

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
