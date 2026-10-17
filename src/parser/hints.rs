// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The expected-type hints a parse hands DOWN into an expression (`⇐`, `@FR-T-Chk`): the
//! element type a vector literal builds with, the enum a bare variant belongs to, the lambda
//! parameter types a call seeds, the tuple shape a push carries, the struct a format string
//! builds — and whether a constant may be handed to a parameter at all (C124).

use super::{ConstHandOff, DefType, Level, Parser, Type, Value, diagnostic_format};

impl Parser {
    /// canonical entry point for building a vector
    /// database type from a content `Type`.  Resolves the element id
    /// through `Data::vector_element_type` (the single derivation of
    /// that fact — narrow leaf, nested vector, or plain `known_type`;
    /// shared with `typedef.rs::fill_database` for struct fields).
    /// Falls back to the default `integer` slot (0) when the content
    /// has no registered type yet.  Every `database.vector(...)` call
    /// in `src/parser/` should route through this helper so locals,
    /// parameters, returns, and literals get the same narrow storage
    /// that struct fields get via fill_database.
    pub(crate) fn vector_of(&mut self, content: &Type) -> u16 {
        if let Some(elem) = self.data.vector_element_type(content, &mut self.database) {
            return self.database.vector(elem);
        }
        let c_nr = self.data.type_elm(content);
        if c_nr == u32::MAX {
            return self.database.vector(u16::MAX);
        }
        let c_tp = self.data.def(c_nr).known_type();
        // Known_type may be unset for forward references; fall back to
        // the default integer slot (0) so the vector type still
        // registers correctly.  The content's own fill pass will
        // update once it runs.
        //
        // EXCEPT inside a generic template: there the unresolved content is
        // the type VARIABLE, which never fills.  Registering the placeholder
        // `vector<integer>` row from the template's parse SHIFTS the runtime
        // type table for everything registered after it, silently breaking
        // the layout-coincident derivations downstream (#483 — a stdlib
        // generic made an unrelated nested-vector literal read garbage).
        // Bake the MAX sentinel without registering anything; the
        // instantiation fixups re-derive the concrete ids.
        if c_tp == u16::MAX
            && self.context != u32::MAX
            && self.data.def_type(self.context) == DefType::Generic
        {
            return u16::MAX;
        }
        let resolved = if c_tp == u16::MAX { 0 } else { c_tp };
        self.database.vector(resolved)
    }

    /// @P314 — the element STORAGE type id to pass as the `OpAppendVector` /
    /// `vector_add` `tp` arg for a vector whose element type is `content`.
    /// Routes through `vector_of` (which consults `narrow_vector_content`) so a
    /// NARROW element (`u8`/`i16`/`i32`) resolves to its narrow storage type
    /// rather than the wide `integer`.  Using `def(type_def_nr(content))
    /// .known_type` instead loses the narrowness for integer aliases, so
    /// `vector_add` strides by 8 over a 1-/2-/4-byte-packed vector and corrupts
    /// (zeroes) the appended elements.  `single` (a distinct base type) is
    /// unaffected either way; this only matters for narrow integer aliases.
    pub(crate) fn append_elem_tp(&mut self, content: &Type) -> i32 {
        // One derivation for every element shape — `Data::vector_element_type`.
        // A NESTED element (`content` is itself a vector) resolves to the inner
        // vector's OWN database type, the 4-byte handle row `record_new` writes;
        // it used to be spelled `vector_of(content)`, which was the same id only
        // while a nested element registered level-COLLAPSED.  Now that
        // `vector<vector<T>>` registers honestly, `vector_of` is the CONTAINER
        // and passing it strode `vector_add` one level too deep.
        // An element with no row answers `u16::MAX`, the "no such type" sentinel, rather than
        // indexing the schema with it.  A template's type variable is one: `a + b` over a
        // `vector<T>` was an internal compiler error here while its concrete twin got the
        // refusal it earns.  A monomorph asks again with the concrete element.
        i32::from(
            self.data
                .vector_element_type(content, &mut self.database)
                .unwrap_or_else(|| {
                    let vec_tp = self.vector_of(content);
                    if vec_tp == u16::MAX {
                        u16::MAX
                    } else {
                        self.database.content(vec_tp)
                    }
                }),
        )
    }

    /// Get an iterator.
    /// The iterable expression is in *code.
    /// Creating the iterator will be in *code afterward.
    /// Return the next expression; with `Value::None` the iterator creation was impossible.
    /// Expected function type for a short-form lambda (`|x| {…}`) — the `⇐` push
    /// ([`Self::expected`]) filtered to `Type::Function` (D1: one channel, not four).
    ///
    /// Read through `base()`, so a nullable expectation (`Optional(fn(…) -> R)`)
    /// still names the parameter types. Nullability is about whether the SLOT may be
    /// absent; it says nothing about the signature a value in it would have, and a
    /// hint that dropped out on `?` would make the rule depend on it (loft#1067).
    pub(crate) fn lambda_hint(&self) -> Type {
        if matches!(self.expected.base(), Type::Function(..)) {
            self.expected.base().clone()
        } else {
            Type::Unknown(0)
        }
    }

    /// Does a binding of this type NAME the value it was given — a record, a struct-enum or a
    /// collection — rather than hold its own copy, as a scalar and `text` do?  The question
    /// C124 asks of a parameter (`calls.md` F-ParamHeap) and loft#1540 of a closure's capture
    /// (LOFT_LITERALS.md § Closures: a record or collection capture shares the value).  A `&` link is
    /// peeled first; whether the binding IS a link is the caller's separate question.
    pub(crate) fn names_callers_value(tp: &Type) -> bool {
        matches!(
            tp.peel_link().base(),
            Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _)
                | Type::Hash(_, _, _)
        )
    }

    /// loft#1540 / C124 — may this value be handed to this parameter?  The one question every
    /// hand-off asks: a value-const value (a `const` parameter or local, a view of one, a read
    /// through a value-const field) reaches a `&` parameter never, and a record or collection
    /// parameter only when that parameter is `const`.  `text` and scalar parameters take their
    /// own copy and are not asked.
    ///
    /// Decided by the SIGNATURE the hand-off names — a declared function's, a function type's
    /// (`fn(const T)`), a builtin callback's — never by the body behind it: what a call means is
    /// judged from the call and the declaration it names (@C121), while a proof that a body does
    /// not write stays an optimisation's to use (@C122).  A `&` parameter is refused as an error
    /// (plan 40's rule 4), and so is a plain one, `const-to-plain-parameter` (@C124): the
    /// warning that rolled it out ended once every shipped library declared its read-only
    /// parameters `const`.  The cure is in the message.
    ///
    /// `@FR-N-Shape` — "is this parameter a `&` link" is a shape question, and a `&τ?` parameter
    /// links exactly as its dense twin does, so it is asked through `base()`.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(crate) fn report_const_argument(
        &mut self,
        actual: &Value,
        param_tp: &Type,
        param_const: bool,
        at: ConstHandOff<'_>,
    ) {
        if param_const {
            return;
        }
        let is_link = matches!(param_tp.base(), Type::RefVar(_));
        let is_heap = !is_link && Self::names_callers_value(param_tp);
        if !is_link && !is_heap {
            return;
        }
        let Some(place) = self.const_view_place(actual, true) else {
            return;
        };
        // A view names itself and what it views: the author passed `f`, not `ps`.
        let what = match (&at, actual.unspan()) {
            (ConstHandOff::Callback { .. }, _) => format!("the elements of {place}"),
            (_, Value::Var(v)) if self.const_views.contains_key(&(self.context, *v)) => {
                format!("'{}', a view of {place},", self.vars.written_name(*v))
            }
            _ => place,
        };
        let (fix_title, fix_condition) = match at {
            ConstHandOff::Declared { callee, nr, param } => {
                if is_link {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot pass {what} to the `&` parameter {} of `{callee}`, which may \
                         modify it; its value is read-only — pass a local copy, or make the \
                         parameter `const` if `{callee}` only reads it",
                        nr + 1
                    );
                    return;
                }
                let param = param.map_or_else(String::new, |a| format!(" `{a}`"));
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    code = "const-to-plain-parameter",
                    "Cannot pass {what} to parameter {}{param} of `{callee}`, which is not \
                     `const`: its value is read-only, and a plain parameter names the caller's \
                     value — declare the parameter `const` if `{callee}` only reads it, or \
                     pass a local copy",
                    nr + 1
                );
                (
                    format!("declare parameter{param} of `{callee}` `const`"),
                    format!("`{callee}` only reads that parameter"),
                )
            }
            ConstHandOff::FnRef { callee, nr } => {
                let label = callee.map_or_else(
                    || "the called function value".to_string(),
                    |c| format!("the function reference `{c}`"),
                );
                if is_link {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot pass {what} to the `&` parameter {} of {label}, which may modify \
                         it; its value is read-only — pass a local copy, or declare the \
                         parameter `const` in the function type (`fn(const …)`) if every \
                         function it holds only reads it",
                        nr + 1
                    );
                    return;
                }
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    code = "const-to-plain-parameter",
                    "Cannot pass {what} to parameter {} of {label}, whose function type does not \
                     declare it `const`: its value is read-only, and a plain parameter names the \
                     caller's value — declare it `const` in the function type (`fn(const …)`) if \
                     every function it holds only reads it, or pass a local copy",
                    nr + 1
                );
                (
                    format!("declare parameter {} of {label}'s type `const`", nr + 1),
                    format!("every function {label} holds only reads that parameter"),
                )
            }
            ConstHandOff::Callback { builtin, nr } => {
                if is_link {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot pass {what} to `{builtin}`'s callback, whose parameter {} is `&` \
                         and may modify them; their value is read-only — pass a copy of the \
                         collection",
                        nr + 1
                    );
                    return;
                }
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    code = "const-to-plain-parameter",
                    "Cannot pass {what} to `{builtin}`'s callback, whose parameter {} is not \
                     `const`: their value is read-only, and a plain parameter names the caller's \
                     value — declare that parameter `const` if the callback only reads it, or \
                     pass a copy of the collection",
                    nr + 1
                );
                (
                    format!(
                        "declare parameter {} of the callback passed to `{builtin}` `const`",
                        nr + 1
                    ),
                    "the callback only reads that parameter".to_string(),
                )
            }
        };
        // Conditional, and without an edit: whether the callee only reads is the author's to
        // affirm (C124 decides by the signature, not by reading the body for them), and the
        // parameter is usually declared in another place than this hand-off.
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: fix_title,
            condition: Some(fix_condition),
            edit: None,
            concept: "const parameters",
            concept_ref: "@F18",
        });
    }

    /// May this expected type seed a short lambda's parameter types?
    ///
    /// The one predicate behind every `⇐` push site that can carry a `fn(…)`, so the
    /// answer cannot differ between a call argument, a struct-literal field, a vector
    /// element, a block tail and a parameter default (loft#1067). LOFT.md states the
    /// rule as *the expected type wherever there is one* — this is "wherever".
    /// @PLN165 D3 — is `tp` an INSTANCE of a generic struct (`Box<integer>`)?  A literal of
    /// the template (`Box { … }`) cannot say which instance it builds, so an expected type
    /// that is one tells it — the way an empty `[]` learns its element type.
    pub(crate) fn seeds_instance_hint(&self, tp: &Type) -> bool {
        matches!(tp.base(), Type::Reference(d, _)
            if (*d as usize) < self.data.definitions.len()
                && self.data.def(*d).instance_of != u32::MAX)
    }

    pub(crate) fn seeds_lambda_hint(tp: &Type) -> bool {
        matches!(tp.base(), Type::Function(..))
    }

    /// The tuple type a `⇐` push should carry for `tp` — the one home for the tuple
    /// question on that channel, asked identically at every push site a tuple literal can
    /// stand in: a block tail / `return`, and a call argument (positional and named).
    /// [`Self::seeds_tuple_member_hint`] then answers for each MEMBER once the literal has
    /// the tuple type in hand.
    ///
    /// A member whose PARSE needs the expected type has nothing else to resolve against: a
    /// bare variant (`(Dot, 9)`) has no enum context, and an empty collection literal
    /// (`([], 9)`) has no element type. Both were accepted in a DECLARED LOCAL and refused
    /// one position over, because the local reads its destination from `var_tp` while a
    /// return and an argument have only this channel (loft#1122).
    ///
    /// ⚠ **The notion has two spellings, and a return only ever shows the second.** A
    /// source-level `(τ₁, …, τₙ)` is a `Type::Tuple`; a tuple RETURN is PROMOTED to
    /// `Reference(__tuple<…>)` — the synthetic struct carrying the caller's `__retbuf` ABI
    /// — before the body is parsed, so a matcher that knows only `Type::Tuple` is blind at
    /// exactly the position the defect is about, and is blind SILENTLY (it answers "no
    /// tuple here" and the member parses with nothing). The members come back off the
    /// struct's `tuple_group`, which is element ORDER rather than attribute order.
    ///
    /// Seed the `⇐` expected-type channel for a value LEAVING this function.
    ///
    /// One home for the admission list two spellings of one act share: the block tail /
    /// `return`, and `yield`.  Both hand a value out of the function against a type the
    /// DECLARATION already names, and a literal that cannot say its own shape — an empty
    /// collection literal, a keyed literal (`[K { … }]` is a `vector<K>` wherever it
    /// stands), a bare enum variant, an interpolation target, a lambda whose parameter
    /// types come from context, a tuple member — has no other way to learn it.
    ///
    /// The list grew one `||` per bug at the block tail while `yield` had NO seeding at
    /// all, so a keyed collection yielded as a literal was built as a `vector<T>` and the
    /// consumer received a collection with the wrong length and no keys, on both backends
    /// and with no diagnostic (loft#1130).  The general rule LOFT.md already states is
    /// *the expected type wherever there is one*; the census of this channel's remaining
    /// push sites and their differing admission lists is QUALITY-history.md § B6t.
    pub(crate) fn seed_leaving_value_hint(&mut self, result: &Type) {
        if self.enum_context(result)
            || crate::parser::vectors::is_collection(result)
            || self.interpolation_target(result) != u32::MAX
            || Self::seeds_lambda_hint(result)
            || self.seeds_instance_hint(result)
        {
            self.expected = result.clone();
        } else if let Some(tuple) = self.tuple_hint_type(result) {
            // The tuple hint is the promoted type read BACK to the source spelling, so
            // what reaches the literal is `(τ₁, …, τₙ)` and not the synthetic struct
            // reference the retbuf ABI turned it into.
            self.expected = tuple;
        }
    }

    /// The `⇐` push an ARGUMENT or a parameter DEFAULT makes (`@FR-T-Chk`): which shapes of
    /// the parameter's type reach the value being parsed.  One list for every spelling of
    /// the position — positional or named, free function or method, or the default a caller
    /// omits — because the spelling is not the axis: `f(X)` resolved a bare variant against
    /// the parameter's enum while `f(p: X)`, `h.m(X)` and `fn f(p: B = X)` reported it
    /// ambiguous, and a method argument took no tuple member types (`@FR-T-Chk-Var`).
    pub(crate) fn argument_hint(&self, expected: Type) -> Option<Type> {
        if Self::seeds_lambda_hint(&expected)
            || self.enum_context(&expected)
            || self.seeds_instance_hint(&expected)
            || self.seeds_collection_hint(&expected)
            || self.interpolation_target(&expected) != u32::MAX
        {
            Some(expected)
        } else {
            self.tuple_hint_type(&expected)
        }
    }

    /// Read through `base()`, so a nullable tuple asks what its base asks — whether a slot
    /// may be absent says nothing about what its members are.
    pub(crate) fn tuple_hint_type(&self, tp: &Type) -> Option<Type> {
        match tp.base() {
            Type::Tuple(_) => Some(tp.base().clone()),
            // A tuple whose member names an undefined type has no record behind it: its
            // reference is `u32::MAX`, and the member's own "Undefined type" is the report.
            Type::Reference(d_nr, _) if *d_nr != u32::MAX => {
                let group = self.data.def(*d_nr).tuple_group()?;
                let members: Vec<Type> = group
                    .field_indices
                    .iter()
                    .map(|&f| self.data.attr_type(*d_nr, f as usize))
                    .collect();
                (!members.is_empty()).then_some(Type::Tuple(members))
            }
            _ => None,
        }
    }

    /// Does this declared tuple MEMBER type say anything the element's own parse needs
    /// to hear on the `⇐` channel?
    ///
    /// LOFT.md states the rule as *the expected type wherever there is one*, and a tuple
    /// member is one of those places: the element is checked against the member type, so
    /// it should be PARSED against it too.  A member with a known type therefore seeds,
    /// and only an `Unknown` — which has nothing to say — does not.
    ///
    /// The channel used to carry `fn(…)` alone, held back until loft#1069 because a
    /// `fn(…)` in a tuple could not be called back out of one whatever spelling put it
    /// there.  That bound is what made three shapes fail in a DECLARED tuple local while
    /// every other position accepted them: a bare variant name (`(Dot, 9)`) had no enum
    /// context to resolve against, and an empty collection literal (`([], 9)`) had no
    /// element type, which reached codegen as *"Incorrect var `__ret_1[32]` versus 24"*
    /// rather than as a diagnostic.  Seeding is what gives each of them the type the
    /// declaration already named.
    ///
    /// A `Type::Tuple` member recurses so a NESTED literal is reached as well: seeding
    /// only the outer step made it per top-level member, and
    /// `((fn(integer) -> integer, integer), text)` left the inner `|x|` untyped
    /// (loft#1073).  With every member seeding, the recursion is what the caller's
    /// `tuple_members` walk does anyway; the arm stays because the predicate is also
    /// asked about a member in isolation.
    pub(crate) fn seeds_tuple_member_hint(tp: &Type) -> bool {
        !matches!(tp.base(), Type::Unknown(_))
    }

    /// Expected enum type for a bare value-position variant (`f(Red)`) — `expected`
    /// filtered to enum context.
    pub(crate) fn enum_hint(&self) -> Type {
        if self.enum_context(&self.expected) {
            self.expected.without_deps()
        } else {
            Type::Unknown(0)
        }
    }

    /// Expected `vector<…>` element type for a bare literal — `expected` filtered to a vector
    /// whose element type names no type variable ([`Self::seeds_vector_hint`]).
    pub(crate) fn vector_hint(&self) -> Type {
        if self.seeds_vector_hint(&self.expected) {
            self.expected.without_deps()
        } else {
            Type::Unknown(0)
        }
    }

    /// Expected destination type for an `f#read` (no `(n)`, no `as T`) — the raw `⇐` push,
    /// any shape; the read infers its byte width from it.
    pub(crate) fn read_target_type(&self) -> Type {
        self.expected.without_deps()
    }

    /// @PLN124 — the struct a format string should BUILD rather than render into
    /// text, or `u32::MAX` when the string is ordinary text.
    ///
    /// The TARGET TYPE decides, which is why the hook needs no new syntax:
    /// `q: SqlText = "… {id} …"` and `f("… {id} …")` are the same string, and only
    /// what receives it differs.  Satisfaction is structural — the rule every loft
    /// interface already uses — and `lit` is the whole test: a type that can accept
    /// the author's literal bytes is a type that can be built, and the `hole_*`
    /// methods it goes on to define say which value kinds it takes natively.
    ///
    /// This is a fifth SHAPE read off the one `⇐` channel, beside `lambda_hint` /
    /// `enum_hint` / `vector_hint` / `read_target_type` — not a sixth side-channel.
    ///
    /// Pass-stable: method defs are collected on BOTH parser passes, so both take
    /// the same branch — the same property the `to_text` hook relies on.  That
    /// matters more here than it looks, because taking the branch mints an
    /// accumulator variable, and a mint that fired on only one pass would shift
    /// the name-keyed variable tables underneath every later work variable.
    /// The method `method` of the type `d_nr` by its mangled name, looked up in the source
    /// that DEFINES the type: `def_nr` searches the current source and the stdlib, so a
    /// type imported by name (`use pkg::(T);`, or a type trigger) kept its methods out of
    /// reach of the hooks that ask for them by name — `lit`, `hole_*`, `to_text` — while a
    /// call on it dispatched fine through the receiver.  `u32::MAX` when there is none.
    pub(crate) fn type_method(&self, d_nr: u32, method: &str) -> u32 {
        let def = self.data.def(d_nr);
        let nm = def.name();
        if nm.is_empty() {
            return u32::MAX;
        }
        let mangled = crate::data::Data::mangle_method(nm, method);
        let found = self.data.def_nr(&mangled);
        if found != u32::MAX {
            return found;
        }
        self.data.source_nr(def.source, &mangled)
    }

    pub(crate) fn interpolation_target(&self, tp: &Type) -> u32 {
        let Type::Reference(d_nr, _) = tp else {
            return u32::MAX;
        };
        if self.data.def_type(*d_nr) != DefType::Struct {
            return u32::MAX;
        }
        if self.type_method(*d_nr, "lit") == u32::MAX {
            return u32::MAX;
        }
        *d_nr
    }
}
