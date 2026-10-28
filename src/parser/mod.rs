// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR)

//! Parse scripts and create internal code from it.
//! Including type checking.

use crate::data::{
    Argument, Context, Data, DefType, Deps, I32, IntegerSpec, Type, Value, to_default, v_block,
    v_if, v_loop, v_set,
};
use crate::database::{Parts, Stores};
use crate::diagnostics::{DiagEntry, Diagnostics, Level, diagnostic_format};
use crate::lexer::{LexItem, LexResult, Lexer, Link, Mode, Position};
use crate::platform::{other_sep, sep, sep_str};
use crate::variables::{Function, size as var_size};
use crate::{manifest, scopes, typedef};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::string::ToString;
use typedef::complete_definition;

/// The type half of [`registry_fn_hint`]: the message for an unresolved TYPE that a
/// published package declares — `what` is the message's head (`Undefined type`, `unknown
/// type`, …).  Without it an un-imported `Rect` read "did you mean 'text'?" and `Canvas` a
/// bare "Undefined type", while the same file's un-imported `canvas(…)` named its package.
#[cfg(feature = "registry")]
pub(crate) fn registry_type_hint(what: &str, name: &str, resolved: &[String]) -> Option<String> {
    let pkgs = crate::registry_index::packages_exporting_type(name);
    let first = pkgs.first()?;
    if let Some(here) = pkgs.iter().find(|p| resolved.iter().any(|r| &r == p)) {
        return Some(format!(
            "{what} {name} — the `{here}` this build resolved does not have it, and the \
             registry's `{here}` does. These are different packages of the same name"
        ));
    }
    let provider = if pkgs.len() == 1 {
        format!("the `{first}` package provides it")
    } else {
        let names: Vec<String> = pkgs.iter().map(|p| format!("`{p}`")).collect();
        format!("the {} packages provide it", names.join(" / "))
    };
    Some(format!(
        "{what} {name} — {provider}; write `{first}::{name}` after `use {first};`, or add \
         `use {first}::({name});` and write it bare"
    ))
}

/// Registry-less build: no index to consult, so no hint.
#[cfg(not(feature = "registry"))]
pub(crate) fn registry_type_hint(_what: &str, _name: &str, _resolved: &[String]) -> Option<String> {
    None
}

/// Where a value is handed to a parameter, for [`Parser::report_const_argument`] (loft#1540,
/// C124): which signature is asked, and how the diagnostic names it.
#[derive(Clone, Copy)]
pub(crate) enum ConstHandOff<'a> {
    /// Parameter `nr` (0-based) of the declared function `callee`, named `param` when it has a name.
    Declared {
        callee: &'a str,
        nr: usize,
        param: Option<&'a str>,
    },
    /// Parameter `nr` of the function a reference holds: `callee` is the reference's name, or
    /// `None` for a call on a function-typed expression (`make()(x)`).
    FnRef { callee: Option<&'a str>, nr: usize },
    /// The elements of a collection handed to `builtin`'s callback as its parameter `nr`.
    Callback { builtin: &'a str, nr: usize },
}

/// @PLN102 case B (soften-nullflow-discharge.md) — the sign / lower-bound lattice used to
/// prove a domain-fault op's argument is in its safe domain (`sqrt` needs `≥ 0`, `ln` needs
/// `> 0`). `Pos ⊑ NonNeg ⊑ Unknown` (stronger → weaker); `Unknown` is the conservative default.
///
/// `pub(crate)` because `Parser::math_sign_proven` carries it: a zero-comparison guard
/// contributes a `Sign` for a slot, which is what makes `@FR-N-Domain`'s guard licence
/// compose with this lattice rather than sit beside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Sign {
    Pos,
    NonNeg,
    Unknown,
}

/// loft#1040 — what a deferred `par` needs that substitution cannot supply: names and
/// slot numbers, which a monomorph shares with its template because its variable table is
/// a COPY of the template's.  The types are deliberately absent — they are re-read from
/// the monomorph's own table at expansion, which is the whole point of deferring.
#[derive(Clone)]
struct DeferredPar {
    /// The name the clause binds its result to (`par(r = …)`), which also names the
    /// result variable the body was parsed against.
    result_name: String,
    /// The worker as the TEMPLATE resolved it — still a generic when the element type is
    /// a type variable, which is exactly why this clause was deferred.
    worker: u32,
    /// The loop's number, which keys the result variable's name across passes.
    loop_nr: u16,
    /// The loop element's variable, whose type IS the substituted element type once the
    /// monomorph's table has been written.
    elem_var: u16,
    /// The loop's COUNTER variable, or `u16::MAX` when the body needs none.  A number,
    /// not a count — passing `0` here made the lowering initialise and increment variable
    /// zero, which is the input vector.
    count: u16,
}

/// A `&<operand>` that may be one side of `&a == &b` (`@FR-E-Eq`): its source span, and whether
/// the operand is a place (`&` names a record only through one).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct AmpIdentity {
    pub(crate) start: (u32, u32),
    pub(crate) end: (u32, u32),
    pub(crate) place: bool,
}

/// How a monomorph reads an element of `vector<T>` with `T` bound (`vector_elem_read`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ElemRead {
    /// The element stride the read's baked type-variable marker is replaced with.
    pub(crate) stride: i32,
    /// The vector holds record ids (`Stores::is_linked`), so the read dereferences.
    pub(crate) linked: bool,
}

/// Which binding position a leading `&` may occupy at the point the operand parser
/// reaches it, and therefore which token ENDS the operand the `&` annotates.
///
/// `@FR-B-Ref-AnnotationOnly` makes `&` a type annotation rather than an operator, so it
/// is legal only at a bind site; `@FR-B-Ref-StoredRef` names the one other position, a
/// struct-literal field whose declared type is `reference<τ>`.  The two positions END
/// differently — a statement at `;`, a field value at `,` as well — and peeking the next
/// token alone cannot tell the whole right-hand side from the LAST operand of one.  Naming
/// the position is what lets the guard ask both halves of that question from one value.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum AmpHead {
    /// Not a bind site.  A leading `&` here is the parse error the rule names — an
    /// operand, a call argument, a collection element, a condition.
    #[default]
    No,
    /// A plain `=` right-hand side (`a = &b`).  A compound assignment is deliberately not
    /// one: it mutates the target rather than giving it a reference type.
    AssignRhs,
    /// A struct-literal field declared `reference<τ>` (`@FR-B-Ref-StoredRef`).  The rule
    /// admits the `&` on the strength of the FIELD'S TYPE and says nothing about where the
    /// field sits in the literal, so a field followed by another field ends at its `,`.
    StoredRefField,
}

/// The slot a store-flavoured [`Parser::convert`] writes into — what `(N-Store)`'s diagnostic
/// names, where it anchors, and whether this seam may escalate to an ERROR at a narrow width
/// (`never_error`: loft#1232's collection-literal seam warns at every width, and so does every
/// seam this phase covers for the first time — reporting where there was silence is a gain,
/// refusing what compiled yesterday is the break the freeze forbids).
#[derive(Clone, Debug)]
pub(crate) struct StoreCtx {
    pub what: String,
    pub at: Option<Position>,
    pub never_error: bool,
    /// The slot can never be declared nullable — an `iterator<τ>` element is dense
    /// (`?` on it is refused) — so a refusal must not offer `τ?` as the cure.
    pub dense: bool,
}

/// loft#1382 — a recorded arm-agreement mismatch, reported only if the construct that
/// produced it turns out NOT to be a `;`-terminated statement.
pub struct ArmMismatch {
    pub test: Type,
    pub should: Type,
    pub context: String,
    pub at: Position,
}

#[allow(clippy::struct_excessive_bools)]
pub struct Parser {
    pub todo_files: Vec<(String, u16)>,
    /// loft#1621 — the last `limit(…)` refused one of its bounds.  The declaration then
    /// recovers as plain `integer`, and a `size(…)` checked against that recovery would name
    /// a missing `limit(…)` over a line that wrote one — so the size check stands down and the
    /// bound's own refusal is the message.
    ///
    /// ⚠ Reset in `parse_typedef`, per DECLARATION, and NOT in `parse_type_limit`.  That
    /// function is reached only for the literal type name `integer`, so resetting there leaves
    /// the flag standing for a following alias declaration that never calls it:
    /// `type Wide = integer limit(0, 5000000000) size(4); type S = i32 size(1);` measured the
    /// second one SILENT, where it reports its own error alone.  Pinned by
    /// `parse_errors::a_refused_limit_does_not_silence_the_next_declaration`.
    ///
    /// (That example was `limit(-100, -1)` until loft#1630 made a range wholly below zero
    /// DECLARABLE — so it refuses nothing now and demonstrates nothing.  An example inside a
    /// comment is a cell nobody runs; this one went stale in one join.)
    limit_refused: bool,
    /// @PLN11 arc E — set by the driver (`main.rs`) only when the whole-program
    /// startup cache is enabled; gates [`Parser::parsed_sources`] tracking so a
    /// normal (non-cache) run pays nothing.
    pub track_sources: bool,
    /// loft#1648 — when this parse began, for the H5 report alone.
    ///
    /// Both passes read every source from DISK independently (`load_main_file` ends in
    /// `lexer.switch(filename)`), so a file that changes between them — a registry
    /// extraction, an editor save, a concurrent `loft install` — is parsed twice from
    /// different bytes.  The definitions then differ across the passes and H5 fires, with
    /// nothing in the message pointing at the file.  Recorded as ONE instant rather than a
    /// stamp per file: the cost is a single clock read on every parse, and the stats that
    /// answer *"did anything move since then"* are paid only when the assert has already
    /// decided to panic.
    pass_started: Option<std::time::SystemTime>,
    /// @PLN11 arc E — paths of every source file parsed (stdlib + lazily-loaded
    /// libs + user file), in load order, recorded only when `track_sources`.
    /// Only the parser sees the dynamically-loaded lib set; the whole-program
    /// cache hashes these files' contents to key the bundle + detect drift.
    /// Paths only (content is re-read once at save time — no per-file memory);
    /// may contain duplicates across the two parse passes, deduped at use.
    pub parsed_sources: Vec<String>,
    /// @PLN196 — the argument type of every `store_load*` / `store_persist*` / `swap_world`
    /// call pass 2 met: the seeds of `Data::surviving`.
    pub(crate) store_handoffs: Vec<Type>,
    /// @PLN196 — the layout of the world a running game holds (`.dschema` text), set by an
    /// editor that runs one (`loft --serve`): a compile then warns, at the world's type, about
    /// a change the next build could not carry the world across (`check_running_world`).
    pub running_world: Option<String>,
    /// All definitions
    pub data: Data,
    pub database: Stores,
    /// The lexer on the current text file
    pub lexer: Lexer,
    /// Are we currently allowing break/continue statements?
    in_loop: bool,
    /// loft#986 — we are parsing a control-flow HEAD (an `if`/`while` condition, a `for`
    /// iterable), where the `{` that follows opens a BLOCK, not a struct literal.
    ///
    /// Only the EMPTY-literal case consults it. `T { }` has no `field:` to shape-check, so
    /// the fallback that consumes an unresolved `Name { … }` cannot tell it from the body
    /// of `if b { }` with an undefined `b` — and without this it swallowed the block and
    /// answered `Expect token {` where the useful message is `Unknown variable 'b'`.
    /// Saved and restored around the head, like `in_loop`.
    in_control_head: bool,
    /// Cognitive complexity per definition — `context` → score, accumulated AS THE SOURCE IS
    /// PARSED.
    ///
    /// Parse time is the only level at which this is meaningful. loft has no AST between the
    /// parser and the Value IR, so a whole-program pass sees post-desugar code: measured on
    /// the IR, five `??` discharges with no author-written branch score 10 and one plain
    /// `for x in v` scores 5, which would charge people for using the null model and the
    /// loop forms idiomatically. Here the parser is looking at exactly what was written.
    ///
    /// Cognitive, not cyclomatic: a construct costs `1 + nesting`, so DEPTH is what is
    /// expensive rather than branch count. Eight sequential `if`s cost 8; three nested ones
    /// already cost 6; a flat `match` costs 1 however many arms it has. That ordering is the
    /// point — "many branches" and "hard to follow" are different properties.
    pub complexity: HashMap<u32, u32>,
    /// Deepest control-flow nesting reached per definition, with the source line where it
    /// was reached — so the advice can say WHERE to cut instead of only how bad it is.
    pub(crate) cc_deepest: HashMap<u32, (u32, u32)>,
    /// Control-flow nesting depth while parsing, for [`Parser::complexity`]. Bumped only
    /// around a construct's BODY, so an `else if` chain — which re-enters `parse_if` after
    /// the `then` body has closed — charges once per link at the chain's own depth rather
    /// than deepening with each link.
    cc_nest: u32,
    /// True while parsing an expression inside a format string `{…}`.
    /// Prevents the `v: type = expr` annotation from consuming `:`.
    pub(crate) in_format_expr: bool,
    /// Is the value being parsed the OPERAND of a unary prefix operator (`-x`, `!x`, `~x`)?
    ///
    /// The assignment's destination hint — `code` arriving as `Value::Var(dest)`, meaning
    /// "build here instead of into a temp" — is valid only for the OUTERMOST value of a
    /// right-hand side, and the three prefix handlers pass it straight into their operand.
    /// `m = -S { … }` therefore built the literal into `m`, which is also where the NEGATION's
    /// result lands, and the frame then freed a reference that no longer described it
    /// (loft#1304).  `Parser::parse_object` declines the hint while this is set.
    ///
    /// A flag rather than a fresh `Value` for the operand: the hint reaches `parse_object`
    /// through a `&mut Value` threaded down several frames, and swapping it out here would
    /// also take the in-place path away from a vector literal and a comprehension, which are
    /// not what broke.
    pub(crate) prefix_operand: bool,
    /// Set while `Parser::parse_object` RE-parses a literal with the destination hint declined.
    ///
    /// The hint is only valid for the outermost value of a right-hand side, and whether a
    /// literal is that can only be known once its body has been consumed and the next token
    /// read.  So the check happens at the END — a single-token `peek_token`, which takes
    /// `&self` and is transparent, unlike a walk past the body (loft#1304: a multi-token
    /// look-ahead leaves `prev_end` at the end of the scan and shifts every later caret).
    /// A literal that turns out to be a sub-expression is then parsed AGAIN from its own
    /// `link`, through the abandon path the field loop already uses, with this set.
    ///
    /// One retry, never two: the flag both requests the second parse and stops it asking again.
    pub(crate) inplace_hint_declined: bool,
    /// loft#2117 — the literal about to be parsed again is built apart and copied into its
    /// PROJECTION place (`o.f = S { … }`) rather than written into it: a field of it built
    /// inside a collection field read the place, whose collection headers the in-place road
    /// zeroes before any field value runs.  Taken (and cleared) by the literal it is meant
    /// for, so a literal nested inside keeps its own road.
    pub(crate) inplace_place_declined: bool,
    /// The local a literal is being rebuilt into, while one of its COLLECTION members is
    /// parsed, or `u16::MAX`.  Such a member is written through its field after the record is
    /// re-initialised, so a read of the local inside it would read the cleared record
    /// (`(E-Asgn)`: the right-hand side is computed before the store).  `var_usages` sets
    /// [`Self::rebuild_watch_hit`] when the author names it, and the literal is then built
    /// apart and bound instead (loft#1574).
    pub(crate) rebuild_watch: u16,
    pub(crate) rebuild_watch_hit: bool,
    /// True while parsing the LHS of a tuple destructuring — `(a, b) = expr`.
    /// The names there are BINDINGS, exactly like the `x` in `x = expr`, so a
    /// name that also belongs to a definition must still mint a variable
    /// rather than resolve to that definition (loft#756).  See
    /// `Parser::at_binding_name`.
    pub(crate) in_tuple_lhs: bool,
    /// `(T-Destr)` over places — the value the next `parse_assign_op` stores, read off the
    /// pattern's temp, in place of a right-hand side it would otherwise parse itself.
    pub(crate) preset_rhs: Option<(Value, Type)>,
    /// The variable whose `x: const T = …` declaration is being parsed: its own initialising
    /// bind sets the value and is not a write `const` forbids (`@FR-Const-ConstructExempt`).
    /// `u16::MAX` outside one.
    pub(crate) declaring_const: u16,
    /// @PLN86 — the host-supplied sandbox policy (profiles + designations).
    /// Empty by default; set by the embedder before parsing.  A script cannot
    /// designate itself — the designation is read from here, not the source.
    pub(crate) sandbox: crate::sandbox::SandboxConfig,
    /// @PLN86 step 1.2 — the sandbox profile NAME each designated function is
    /// parsed under (`def_nr -> profile`), recorded as the function is parsed.
    /// The compile-time admission walk reads this to know which defs are
    /// restricted.  Side-map (not on `Definition`) so it stays out of the IR
    /// serialization — the tag is policy, re-derivable from `sandbox`.
    pub(crate) def_sandbox: HashMap<u32, String>,
    /// @PLN86 step 3.1 — sandboxed defs that contain an unbounded `while` loop,
    /// mapped to one such loop's position.  Recorded at parse (`parse_while`
    /// uniquely knows it is a `while`, where the IR cannot reliably tell a `while`
    /// `Loop` from a bounded comprehension `Loop`); the totality admission reads
    /// it.  Keyed by def_nr so the two parse passes are idempotent.
    pub(crate) sandbox_unbounded_loops: HashMap<u32, crate::lexer::Position>,
    /// @PLN86 step 2.4 — sandboxed defs that perform a RAW WRITE to heap data
    /// (`x.field = v` / `v[i] = v`), mapped to one such write's position.
    /// Recorded at parse (`parse_assign` knows the LHS is a field/index target,
    /// not a bare local or a struct literal); the no-raw-write admission reads it.
    /// A sandboxed script may mutate host data only via allow-listed `*.write`
    /// ops, never a raw field/element assignment.  Keyed by def_nr (idempotent).
    pub(crate) sandbox_raw_writes: HashMap<u32, crate::lexer::Position>,
    /// @PLN110 3a — locals currently holding a `len(X)` result (`n = len(s)`), so
    /// `for i in 0..n` carries the same strict-index bound as the inline
    /// `for i in 0..len(s)`.  The two forms are the same units error, and the
    /// bound-to-a-local one is what the published `cbor` encoder shipped, so a lint
    /// that saw only the inline form would have missed the real bug.  Any other
    /// assignment to the local DROPS its entry, so an unclear binding yields a miss
    /// rather than a false warning.  Cleared per function.
    pub(crate) len_bound_locals: HashMap<u16, crate::parser::operators::VecKey>,
    /// @PLN86 L4 — the functions a sandboxed def references as a fn-ref VALUE
    /// (`apply(read_file)`, `let h = read_file`, `[read_file]`, a returned
    /// fn-ref), mapped `def_nr -> {referenced fn def_nrs}`.  Recorded at the
    /// fn-ref CREATION site (where a function name becomes a value), so it catches
    /// every flow — call-arg, assignment, struct field, collection element, return
    /// — completely, where a post-parse IR walk could not (a bare `Int(d_nr)`
    /// fn-ref is indistinguishable from an integer literal).  The admission unions
    /// these into the checked set so an indirect call can't escape (L4).
    pub(crate) sandbox_fn_refs: HashMap<u32, std::collections::HashSet<u32>>,
    /// @PLN86 P6.1 — the dotted names of every `capability` group declared in the
    /// program (stdlib + host code).  A `group#right` capability link resolves
    /// against this set; an undeclared/mistyped group is a load error.  The dotted
    /// name IS the namespace (matched hierarchically on the grant side, like the
    /// `fs.read` groups).  Parser-side (off the IR), idempotent across passes; IR
    /// persistence for a warm-cached stdlib is P6.8.
    pub(crate) declared_capabilities: HashSet<String>,
    /// @PLN86 P6.4 — capability links on struct fields, keyed by `(struct def_nr,
    /// field name)` → the `group#right` tokens written after the field's type
    /// (`health: int stats#read stats#update`).  The admission walk gates a
    /// sandboxed read/update/append of a host field on the matching grant.
    /// Parser-side (off the IR), recorded first-pass; IR persistence is P6.8.
    pub(crate) member_access: HashMap<(u32, String), Vec<String>>,
    /// @PLN86 P6.4 (F4) — sandboxed READS of a host field that carries a `#read`
    /// capability link, keyed by the reading sandboxed def → the `(read token,
    /// position)` of each such read.  Recorded second-pass at the field-access site
    /// (where the struct type + field name resolve); admission rejects a read whose
    /// token the profile does not grant.  Reads are default-allow, so only a field
    /// the host marked with a `#read` link is ever recorded here.
    pub(crate) sandbox_field_reads: HashMap<u32, Vec<(String, crate::lexer::Position)>>,
    /// `@FR-Cap-Own` — in a sandboxed def, the locals every assignment of which is a struct
    /// LITERAL, keyed `(def, var)` → still literal-only.  Filled on pass 1, which sees the whole
    /// body, and read on pass 2: such a local is data the script BUILT, so reading a `#read`
    /// field of it reveals only what the script put there.  A parameter, a loop variable, a
    /// copy or a call result never enters as `true` — a copy of host data is still host data.
    pub(crate) sandbox_built: HashMap<(u32, u16), bool>,
    /// @PLN86 P6.4 (F5) — sandboxed UPDATES (raw writes) of a host field that carries an
    /// `#update` capability link, keyed by the writing def → each `(struct def_nr, field,
    /// position)`.  A field write WITH an update link is diverted here (admission admits
    /// iff the token is granted); a write to a field with NO update link stays the coarse
    /// 2.4 `sandbox_raw_writes` reject (read-only by default).
    pub(crate) sandbox_field_updates: HashMap<u32, Vec<(u32, String, crate::lexer::Position)>>,
    /// @PLN86 P6.4 (F6) — sandboxed APPENDS (`e.f += x`, growing a collection field)
    /// of a host field that carries an `#append` link, keyed by the writing def → each
    /// `(struct def_nr, field, position)`.  A `+=` to an `#append`-linked field is
    /// diverted here (admission admits iff the token is granted); without an append
    /// link it falls back to the `#update` (F5) or coarse (2.4) path.
    pub(crate) sandbox_field_appends: HashMap<u32, Vec<(u32, String, crate::lexer::Position)>>,
    /// @PLN86 P6.4 (F5) — transient one-shot: the `(struct def_nr, field, #read-links
    /// recorded)` of the field access `field()` last built, so the assignment site can
    /// resolve which field a raw write targets (and un-record the spurious F4 read it
    /// just logged for a write LHS).  Set per field access in a sandboxed def; consumed
    /// at the write site.
    pub(crate) last_field_target: Option<(u32, String, usize)>,
    /// @PLN86 §7.2 (F7) — parameter `…#default` locks, keyed by `(fn def_nr, param
    /// index)` → the lock token written after the parameter's default (`count: int = 1
    /// spawn.count#default`).  At a sandboxed call site, an argument that OVERRIDES a
    /// locked parameter (differs from its default) is gated on the token.  Parser-side
    /// (off the IR), recorded first-pass; sandboxed programs always parse fresh.
    pub(crate) param_locks: HashMap<(u32, u32), String>,
    /// @PLN86 §7.2 (F7) — sandboxed OVERRIDES of a `…#default`-locked parameter, keyed by
    /// the calling def → each `(lock token, position)` of an argument that differed from
    /// the locked parameter's default.  Recorded second-pass at the call site (where the
    /// callee + its argument values resolve); admission rejects an override whose lock
    /// token the profile does not grant.
    pub(crate) sandbox_param_overrides: HashMap<u32, Vec<(String, crate::lexer::Position)>>,
    /// @PLN86 §7.2 (F7) — transient: the `(param index, lock token)` pairs parsed while
    /// reading a function's parameter list, ferried to `parse_function` to be recorded in
    /// `param_locks` once the function's def_nr is known (parameters parse BEFORE the def
    /// is created).  Cleared at the start of each parameter list; consumed per function.
    pub(crate) pending_param_locks: Vec<(usize, String)>,
    /// @PLN187 — each parameter's fact (where its declared type names an alias), in order,
    /// ferried to `parse_function` like `pending_param_locks` and stored on the parameter's
    /// attribute (`Attribute::fact`).
    pub(crate) pending_param_facts: Vec<crate::data::AliasFact>,
    /// @PLN187 — the fact of the type spelling parsed last (`parser::abstract_alias`).
    pub(crate) type_fact: crate::data::AliasFact,
    /// @PLN187 — the fact the CURRENT `parse_type` level produced (an alias, a vector of one);
    /// a nested level takes its own before returning.
    pub(crate) own_fact: Option<crate::data::AliasFact>,
    /// @PLN187 — where the operand just parsed holds an abstract alias
    /// (`parser::abstract_alias`, the protocol).
    pub(crate) operand_fact: crate::data::AliasFact,
    /// @PLN187 — the locals that hold an abstract alias, by `(function, variable)`: a lambda's
    /// body numbers its variables afresh under its own `context`.
    pub(crate) abstract_vars: std::collections::HashMap<(u32, u16), crate::data::AliasFact>,
    /// @PLN187 — the locals declared with a PLAIN type annotation: an abstract value is not
    /// one of those.
    pub(crate) plain_declared: std::collections::HashSet<(u32, u16)>,
    /// `@FR-F-Ret` (loft#2179) — the parameters an early `return` has merged into the return's
    /// deps, by `(function, first_pass)`.  A mid-body `return` site sees only its own value, so
    /// it cannot tell a parameter another `return` borrows from a stale one; this is how it
    /// knows (`ref_return`'s D-own-2 prune).
    pub(crate) mid_return_param_deps: std::collections::HashSet<(u32, bool, u16)>,
    /// loft#2188 — the parser is reading the right-hand side of a plain local's `=`: a branch
    /// there is a VALUE being bound, whose arms `lift_borrowing_call_value` lifts.  Cleared
    /// inside a lambda's body, whose tail is a return position again (`ref_return`'s).
    pub(crate) in_bind_rhs: bool,
    /// @PLN187 — the fact a producer that is not a variable or a call (a field read, a
    /// literal, a branch's value) left for `settle_operand` to take.
    pub(crate) produced: Option<crate::data::AliasFact>,
    /// @PLN187 — the parameter facts the next lambda's body is seeded with: a generic's type
    /// variable an earlier argument bound (`map(v, |x| …)`).
    pub(crate) pending_lambda_facts: Vec<crate::data::AliasFact>,
    /// @PLN187 — the fact the last lambda body yielded (`check_result` records it).
    pub(crate) lambda_result: crate::data::AliasFact,
    /// @PLN187 — the fact of the enclosing `match`'s subject, for its tuple patterns.
    pub(crate) match_subject: crate::data::AliasFact,
    /// @PLN187 — the facts the arms of the enclosing `match` yielded.
    pub(crate) match_arm_facts: Vec<crate::data::AliasFact>,
    /// @PLN187 — the receiver fact of the method step being parsed (`parse_part` sets it).
    pub(crate) method_receiver: crate::data::AliasFact,
    /// @PLN187 — the positional and named argument facts of the method call being parsed.
    pub(crate) method_facts: (
        Vec<crate::data::AliasFact>,
        Vec<(String, crate::data::AliasFact)>,
    ),
    /// @PLN187 — the method step just parsed was checked as a call; `check_postfix` skips it.
    pub(crate) method_checked: bool,
    /// @PLN187 — the fact of the comprehension just parsed, for the literal that holds it.
    pub(crate) comprehension_fact: Option<crate::data::AliasFact>,
    /// @PLN187 — the facts a keyed lookup's key fields are declared with, for `parse_key`.
    pub(crate) pending_key_facts: Vec<crate::data::AliasFact>,
    /// @PLN187 — the definition the last `call_with_named` selected, or `u32::MAX`: a call
    /// lowered to an operator (`sort` → `OpSortVector`) does not name it.
    pub(crate) last_called: u32,
    /// @PLN187 — the member name the current postfix step read (`v.insert(…)`).
    pub(crate) postfix_member: String,
    /// @PLN115 tail — a parameter's `(arg_index, name_pos, name_len)` captured while
    /// reading the signature (positions there, but the def_nr / var_nr are not yet
    /// established), ferried to `parse_function` to record each param's DECLARATION
    /// occurrence once `self.context` is known.  Populated only when recording;
    /// cleared per parameter list, like `pending_param_locks`.
    pub(crate) pending_param_positions: Vec<(u16, Position, u16)>,
    /// @PLN87 — transient one-shot: set while parsing a `&<lvalue>` binding (the prefix
    /// `&` in `b = &a`, or the `&` in a `b: &T = a` type annotation), consumed by
    /// `parse_assign_op` to lower a SCALAR reference to `OpCreateStack`. Cleared per
    /// binding so it never leaks into the next statement.
    pub(crate) amp_pending: bool,
    /// The variables whose FIRST binding is the assignment being parsed, innermost last.  A
    /// lambda on that right-hand side is created before the variable holds anything, so the
    /// name is not a capture there (`go = fn(i) { go(i - 1) }`); a rebinding is not listed and
    /// captures the previous value as usual.
    pub(crate) first_bind_targets: Vec<String>,
    /// Where each variable is first bound: pass 1 records `(function, variable)` → the
    /// position of the statement whose left-hand side CREATED the variable, and both passes
    /// read it back to tell a first binding from a rebinding.
    pub(crate) first_bind_at: HashMap<(u32, u16), (u32, u32)>,
    /// `@FR-B-Copy` — the struct literals that are UPDATES (`S { f: e, ..base }`), as
    /// `(function, line, column)` of the literal's `{`: pass 1 meets the `..` after reading the
    /// fields as a fresh record's and records it; pass 2 reads it back and takes the update
    /// road before the roads that mint a work reference run, so nothing is minted for a
    /// record the update never builds (loft-lang/plans#202 P2).
    pub(crate) update_literals: HashSet<(u32, u32, u32)>,
    /// `@FR-B-Scope` — the `(function, variable, after_end)` triples whose binding the
    /// statement being parsed STARTS: its first binding, or (`after_end`) a bind after the
    /// name's block ended.  Innermost last.  The variable holds nothing until that statement
    /// completes, so a read of it on the right-hand side is refused — `read-in-own-binding`,
    /// or `local-out-of-scope` when the value it would read is the ended binding's.
    pub(crate) binding_starts: Vec<(u32, u16, bool)>,
    /// loft#1382 — the statement about to be parsed BEGINS with `if` or `match`, so whatever
    /// that construct yields is discarded (`@FR-F-Block`: a `;`-terminated block's value is
    /// dropped, and `@FR-F-Drop` still runs the work).
    ///
    /// Statement position is decidable only by the caller: `parse_block`'s loop is what knows
    /// the `;`, and a construct cannot infer it from its own arms.  A one-token peek is enough
    /// because a value-position `if` never STARTS its statement — `v = if …` starts with `v`.
    /// Read once by `parse_if`, which clears it so a nested value-`if` inside a statement one
    /// does not inherit it.
    pub(crate) stmt_if_pending: bool,
    /// @PLN152 step 5 — the narrow store the statement just parsed made, offered to an
    /// `if !place { … }` standing as the VERY NEXT statement.
    ///
    /// `u8`/`i8`/`u16`/`i16`/`u32` and every `limit(lo, hi)` range fill their own width, so
    /// a value that does not fit takes the type's default and nothing in the program can
    /// tell that answer from a computed one.  The fit status is therefore carried in a
    /// `__fit_N` temp beside the STORE and consumed by the `!` in the next statement's
    /// condition — never stored in the slot, an element or a field, which is what keeps a
    /// `vector<u8>` one byte per element.
    ///
    /// Set at the compound-assignment seam, promoted by [`Self::parse_block_inner`] when the
    /// pushed statement really carries the `OpRangeDefault` guard, and dropped at the next
    /// statement boundary — so nothing survives past the pair the author wrote.
    pub(crate) fit_candidate: Option<crate::parser::fit::FitFusion>,
    /// The promoted [`Self::fit_candidate`], live across exactly the following statement.
    pub(crate) fit_armed: Option<crate::parser::fit::FitFusion>,
    /// True only while the CONDITION of the `if` directly following such a store is parsed.
    /// `parse_if` opens the window (it is the one site that knows statement position) and
    /// `parse_if_expecting` closes it the moment the condition is complete, so a `!place`
    /// in the BODY reads the slot as it always did.
    pub(crate) fit_in_condition: bool,
    /// loft#1382 — an arm-agreement mismatch found while parsing a construct that BEGAN its
    /// statement, held until the `;` after it is visible.
    ///
    /// A leading `if` does not prove statement position — a function TAIL begins its statement
    /// too — and `@FR-F-Block` keys on the `;`.  The gate records here and `parse_block`'s loop
    /// either drops it (a `;` followed: the value is discarded, so the arms need not agree) or
    /// reports it.
    pub(crate) pending_arm_mismatch: Option<ArmMismatch>,
    /// Set when `parse_match_arm_body` has already reported an arm whose value its
    /// siblings' type cannot hold.  The cross-arm `match_arm_types_unify` gate asks the
    /// same question less precisely (`is_same` collapses every `Integer(_)`, so it cannot
    /// see a narrowing), so it stays silent for an arm the conversion already named
    /// rather than reporting the same mistake twice.
    pub(crate) arm_convert_reported: bool,
    /// How many arms of the `match` being parsed are written `null` or `{ null }` — read from
    /// the SOURCE by `parse_match_arm_body`, because on the first pass a comprehension arm is a
    /// placeholder that looks like a null arm in the lowered code (`D-types-5`).  Each chain
    /// saves and zeroes it at its start, so a nested match cannot count into its parent.
    pub(crate) null_literal_arms: u32,
    /// Whether the block `parse_block` just parsed ENDS in the source token `null` — the `if`
    /// arm `{ null }`.  Read by the `(N-Join)` widening straight after the parse: the lowered
    /// arm no longer says null (it is its sibling's typed null by then), and on the first pass
    /// a comprehension arm is a placeholder of the same shape, so only the source can tell.
    pub(crate) block_tail_null_literal: bool,
    /// `@FR-F-Target` — the template accumulator (`(function, variable)`) whose wrong-shape
    /// `lit` was already refused: one refusal per template, not one per literal run.
    pub(crate) lit_shape_refused: Option<(u32, u16)>,
    /// loft#1382 — the arms currently being parsed belong to a construct in STATEMENT
    /// position, so their types need not agree with each other.
    ///
    /// Set by `parse_if` from [`Self::stmt_if_pending`] for the duration of its arms and read
    /// at the arm-agreement gate.  Separate from `stmt_if_pending` because that one is
    /// consumed at the construct and this one has to survive into the arms.
    pub(crate) arms_of_statement_construct: bool,
    /// loft#1386 — some arm of the `match` being parsed yields NOTHING.
    ///
    /// A void arm is exempt from the arm-type unification on purpose: in STATEMENT position
    /// `@FR-F-Block` discards what any arm yields, so the arms need not agree.  In VALUE
    /// position there is no value on that path at all, and the exemption made the whole match
    /// take the other arms' type and answer `null` — a value the program never wrote.  Set as
    /// the arms are joined and read once at the end, because "the running result is void" and
    /// "an arm was void" are different facts: the result starts void before any arm.
    pub(crate) match_void_arm: bool,
    /// @PLN87 B-Ref-AnnotationOnly — true while the parser is positioned at the HEAD
    /// of a place where `&` may legally appear: the whole right-hand side of a plain
    /// `=` binding, or the start of a statement (where the D-bind-7 guard reports a
    /// bare `&a;`).  The FIRST primary parsed consumes it, so a `&` reached after any
    /// operator (`1 + &a`), or inside a nested construct (a call argument, a literal
    /// element, a parenthesis), sees `false` and is rejected.  Without it the guard
    /// could only peek the NEXT token, which cannot tell the whole RHS from the LAST
    /// operand of one — the hole that let `b = 1 + &a;` compile.
    pub(crate) amp_head: AmpHead,
    /// `@FR-E-Eq` / @C91 — a `&<place>` written as a WHOLE operand of `==` / `!=`, which asks
    /// identity (`&a == &b`).  The prefix-`&` parse cannot tell a whole operand from the last
    /// operand of a wider one (`x + &a == y`), so it records the operand's span here and the
    /// comparison claims it only when the span is exactly its operand; anything unclaimed is
    /// refused as the sub-expression `&` it is.
    pub(crate) amp_identity: Option<AmpIdentity>,
    /// Where the right operand of the `==` / `!=` being parsed begins, so a `&` that opens it
    /// is recognised as an identity operand (see [`Parser::amp_identity`]).
    pub(crate) eq_rhs_at: Option<(u32, u32)>,
    /// The left operand of the `==` / `!=` about to be handled was a whole `&<place>` — set by
    /// the operator loop, read once by `handle_operator`.
    pub(crate) eq_amp_left: Option<AmpIdentity>,
    /// The local an assignment is writing, for the duration of that assignment's right-hand
    /// side; `u16::MAX` outside one.  Paired with [`Parser::assign_replaces`], which says
    /// whether the write REPLACES the target (`=`, which repoints it at a fresh store) or
    /// appends to what it already holds (`+=`).
    ///
    /// A comprehension RHS reads both to keep `I-Comp`'s promise that the destination still
    /// holds its old value while the loop runs — see
    /// [`Parser::comprehension_reads_target`].
    pub(crate) assign_target: u16,
    /// Whether [`Parser::assign_target`] is being REPLACED (`=`) rather than appended to.
    /// Meaningless while `assign_target` is `u16::MAX`.
    pub(crate) assign_replaces: bool,
    /// How many ops at the HEAD of the right-hand side's build are the snapshot a literal took
    /// of the destination it reads ([`Parser::snapshot_read_destination`]); `0` when it took
    /// none.  `parse_assign_op_inner` takes it once the right-hand side has parsed and hands
    /// it to the site that inserts the destination's detach — the `=` repoint in
    /// `create_vector`, the field clear — as that site's insertion point, so the detach lands
    /// after the snapshot.  Reset per assignment beside [`Parser::assign_target`].
    pub(crate) build_snapshot_len: usize,
    /// The user-visible heap PARAMETER whose whole-binding reassignment already carried its
    /// own `(F-ParamRebind)` lowering; `u16::MAX` otherwise.  Read and cleared by
    /// [`Parser::parse_assign_op`], which supplies that lowering for every OTHER right-hand
    /// side (loft#1290).
    ///
    /// A struct LITERAL builds IN PLACE, so its detach-and-mint has to sit inside
    /// `parse_object` between the construction's own ops; every other spelling produces a
    /// finished value the statement can be wrapped around.  Two sites, one rule, and this
    /// says which of them ran.
    pub(crate) rebind_lowered: u16,
    /// @PLN86 step 0.1 — true while parsing the BODY of a sandboxed def.  Gates
    /// the parser nesting guard so it never touches trusted code (zero cost
    /// there); set per-def in `parse_function`, cleared at its end.
    pub(crate) in_sandbox: bool,
    /// @PLN86 step 0.1 — current expression-nesting depth.  Recursive descent over
    /// deep nesting (`((((…))))`) overflows the native stack (rc=139); past
    /// `crate::limits::LIMIT` the parser rejects
    /// with a clean diagnostic instead.  Reset to 0 at each def's body.
    pub(crate) parse_depth: u32,
    /// @PLN86 step 0.1 — latched once the depth limit trips, so the diagnostic
    /// is emitted once per def rather than at every frame as the parser unwinds.
    pub(crate) depth_overflowed: bool,
    /// Where the diagnostics stood just after the nesting refusal: the frames above the refused
    /// expression unwind over tokens they were not written for, and what they report there is
    /// not the author's mistake.  `parse_function` rewinds to it once the body is read, so the
    /// def carries the one refusal.
    pub(crate) depth_trip_mark: Option<(usize, Level)>,
    /// The current file number that is being parsed
    file: u32,
    pub diagnostics: Diagnostics,
    default: bool,
    /// The definition that is currently parsed (function or struct)
    context: u32,
    /// Extra library directories for 'use' resolution (from --lib / --project flags)
    pub lib_dirs: Vec<String>,
    /// The library short-name of the file being parsed, when that file is a
    /// package entry — so a self-qualified call (`graphics::color_r(..)` inside
    /// `src/graphics.loft`) resolves to the definitions already in hand.
    ///
    /// `use_names` deliberately never holds the main file (source 1 is reserved so
    /// a user def can shadow a prelude name), so without this a package that
    /// qualifies with its OWN name reported "Unknown library" — for a library that
    /// was not merely present but currently being read (loft#656).
    own_lib: Option<String>,
    /// Resolved paths of native shared libraries to load after `byte_code()`.
    /// Populated during `use` processing when a package manifest contains `native`.
    pub pending_native_libs: Vec<String>,
    /// The `(stem, pkg_dir)` registration inputs behind `pending_native_libs`.
    /// Persisted in the startup-cache manifest so a warm load can re-resolve
    /// the cdylibs with cold-equal freshness semantics (#310 — the resolved
    /// PATHS alone can go stale when loft itself is rebuilt).
    pub native_lib_regs: Vec<(String, String)>,
    /// @PLN11 Arc N / N3 — package dirs of libraries that opted into
    /// auto-compilation (`[library] compile = "native"`).  Recorded during `use`
    /// processing; the driver (`main.rs`) marks each library's public
    /// shared-store-dispatchable functions native (after `scopes::check`, before
    /// `byte_code`) and builds + loads the cdylib (after `byte_code`).  A library's
    /// functions are identified by `def.position().file.starts_with(pkg_dir)`.
    pub pending_native_compile: Vec<String>,
    /// @PLN199 A2 — the standard library functions whose compiled bodies the driver points
    /// their calls at after parsing (`compiled_stdlib::mark`), named here by the driver
    /// before the program is read, so a call to one gets what a bridged call needs
    /// ([`Parser::crosses_text_bridge`]).  Empty for a run that compiles the program.
    pub compiled_bridges: std::collections::HashSet<String>,
    /// @PLN199 — will a `use`d library's calls cross the shared-store bridge?  False when the
    /// driver compiles the whole program, or runs every library interpreted
    /// (`LOFT_NO_NATIVE_LIBS`): no call then needs [`Parser::crosses_text_bridge`]'s temporary.
    pub library_bridges: bool,
    /// @PLN119 arc A — libraries that declared `[library] placement = "process"`,
    /// as (library name, package dir).  Recorded during `use` processing; the
    /// driver (`main.rs`) starts one worker per entry after `scopes::check` and
    /// routes the library's public functions to it.  A library's functions are
    /// identified by `def.position().file.starts_with(pkg_dir)`, the same
    /// ownership guard the native path uses.
    pub pending_placed_libs: Vec<(String, String, crate::manifest::Placement)>,
    /// PKG.3: package dependencies discovered during manifest reading.
    /// Each entry is (name, dir, queued-from file) — sibling packages are
    /// searched in `dir`, and the dep is only pulled in while the lexer is back
    /// on the file that named the package (loft#714).
    pending_pkg_deps: Vec<(String, String, String)>,
    /// Auto-`use` per-file scan cache: a file's `(lib:: refs, .method() calls)`
    /// are deterministic, so it is read + scanned at most once (keyed by path)
    /// and reused across the second pass and any `todo_files` re-parse.
    auto_use_scan_cache:
        std::collections::HashMap<String, (Vec<String>, Vec<String>, Vec<String>, Vec<String>)>,
    /// Per-directory cache for the dep-shadowing guard in `lib_path`: the
    /// nearest ancestor manifest's package root + its declared dependency
    /// names.  `None` = no manifest above that directory.
    pkg_dep_cache:
        std::collections::HashMap<String, Option<(String, std::collections::HashSet<String>)>>,
    /// The FILE each `use` short-name was loaded from, canonicalised (loft#912).
    ///
    /// A module's short name is global across the whole dependency graph, but the
    /// file it names is not: two packages may each hold `src/catalogue.loft`.  Only
    /// the first is ever loaded — the second `use catalogue;` finds the name taken
    /// and imports the other package's module — so the second package's own
    /// functions are simply absent, reported as `Unknown function` at a line in a
    /// package the author did not write.  Keeping the path is what lets the clash be
    /// reported as a clash, naming both files.
    use_paths: std::collections::HashMap<String, String>,
    /// loft#968 — packages already reported as resolved-but-undeclared, so one `use`
    /// speaks once.  Deliberately NOT carried by [`Self::seed_from`]: it is a parse-time
    /// side map driving a diagnostic the base parse has already emitted.  Gated with its
    /// only reader, so a `--no-default-features` build does not carry a field nothing
    /// there can read.
    #[cfg(feature = "registry")]
    undeclared_reported: std::collections::HashSet<String>,
    /// The `use` ids already reported by [`Parser::lib_flag_outranked`] — one advice per
    /// id, for the same reason and with the same non-travel rule as `undeclared_reported`.
    lib_outranked_reported: std::collections::HashSet<String>,
    /// What an auto-install already answered this run: `id -> Some(version)`, or `None`
    /// for a name the registry does not carry.  @PLN143 — a bare `use` is decided by
    /// `probe_auto_install` on EVERY parse pass now that no lockfile pins it, and asking
    /// the registry twice is both the cost and a correctness question: the two passes
    /// must resolve the same file, and re-deciding gives them two chances not to.
    #[cfg(feature = "registry")]
    auto_installed: std::collections::HashMap<String, Option<String>>,
    /// Packages already told about in the "this pin has fallen behind" notice (@PLN143
    /// arc E), for the same reason `undeclared_reported` exists: both parse passes walk
    /// the resolution path, and one pin reported twice reads as two.
    #[cfg(feature = "registry")]
    pin_behind_reported: std::collections::HashSet<String>,
    /// Newest release per package as the CACHED index knows it, read at most once per run
    /// and only when a pin is actually in force (@PLN143 arc E).  `None` until asked.
    #[cfg(feature = "registry")]
    newest_releases: Option<std::collections::BTreeMap<String, String>>,
    /// Module names already reported as captured by another file (loft#912 / loft#949),
    /// for the same reason and with the same non-travel rule as `undeclared_reported`:
    /// the `use` is walked by both the pre-scan and the definition loop, and one
    /// collision printed twice reads as two separate collisions.
    module_clash_reported: std::collections::HashSet<String>,
    /// Root project's `[dependencies]` version constraints (name → req), read
    /// once from the main script's nearest-ancestor `loft.toml` (via
    /// `source_dir`).  Pins source-level auto-installs across the WHOLE tree —
    /// direct AND transitive — so a consumer can pin any package it pulls (e.g.
    /// `glb = "=0.1.0"`), making exact pinning an available option instead of
    /// always resolving the newest release.  `None` until first looked up.
    #[cfg(feature = "registry")]
    root_dep_pins: Option<std::collections::HashMap<String, String>>,
    /// The declaration that governs the whole PROGRAM's registry versions, answered once
    /// from the entry file at the first registry lookup — a program that
    /// `use`s nothing never asks.  A `use` inside a dependency resolves through it too:
    /// read from the file doing the `use`, a dependency in the registry cache found its own
    /// cached `loft.toml` and a lock that never exists there, so the consumer's pin for a
    /// transitive package was ignored.
    program_scope: Option<crate::resolution_scope::ResolutionScope>,
    /// The entry file, as the lexer already holds it (a shared handle, no copy), for
    /// `program_scope`.  `None` before the entry file is loaded.
    program_entry: Option<crate::lexer::FileName>,
    /// loft#1687 — per package a declared scope could not satisfy from the cache: the
    /// constraint and the versions the cache holds, so "library not found" can say which
    /// declaration was unmet instead of implying nothing is there.
    cache_unmet: std::collections::HashMap<String, String>,
    /// The `use` ids an auto-install asked the registry for and was refused — the index it
    /// decided on was fetched in that call — so the "not found" message says the registry
    /// has nothing in range rather than that it could not be asked.
    #[cfg(feature = "registry")]
    registry_refused: std::collections::HashSet<String>,
    /// Tier-1 text-method trigger map: `method name -> providing package`,
    /// derived once per top-level parse from the current package's (and its
    /// trigger-enabled dependencies') declared triggers.  `None` until built.
    auto_use_trigger_map: Option<std::collections::HashMap<String, String>>,
    /// `@FR-Disp-Trigger` (loft#2094) — the method names two trigger-enabled packages of ONE
    /// search step both derive, with those packages: dropped from the map (no directory order
    /// chooses), and named by the unknown-method refusal.  Built with `auto_use_trigger_map`.
    pub(crate) auto_use_ambiguous: std::collections::HashMap<String, Vec<String>>,
    /// The type half of the same surface: `type name -> providing package`, from the
    /// current package's and its trigger-enabled dependencies' `pub struct` / `pub enum`
    /// names (`triggers.rs`).  Built with `auto_use_trigger_map`; local only — a type name
    /// never reaches the registry catalog, so an unknown type loads only a package the
    /// author declared.  What lets `c: Command = "…"` load `process` with nothing in the
    /// script naming it (@PLN179 strand 4).
    auto_use_type_map: std::collections::HashMap<String, String>,
    /// The builder half: `function name -> providing package`, from the `[triggers]
    /// builders` each trigger-enabled package declares and its source confirms
    /// (`triggers.rs` § Builder triggers).  Built with `auto_use_trigger_map`.  What lets a
    /// bare `set(words)` load `script` and import that one name — a program's own `fn set`
    /// and the stdlib's names are never overridden, and `script::set(…)` always resolves.
    auto_use_builder_map: std::collections::HashMap<String, String>,
    /// The manifest errors already reported by [`report_manifest_errors`]: each file's parse
    /// reads the manifests again, and an error is the author's to fix once.
    reported_manifest_errors: Vec<crate::manifest::ManifestError>,
    /// Tier-1 lazy *catalog* fallback: `method name -> providing package`,
    /// derived once from the cached registry `index.json` (`triggers` field).
    /// Consulted only for methods the local `auto_use_trigger_map` did not
    /// resolve — i.e. a package the user has NOT declared as a dependency but
    /// that the registry says provides the method.  `None` until first miss;
    /// cached empty when the catalog is absent or the registry feature is off.
    // Only read by the `#[cfg(feature = "registry")]` `catalog_trigger_map`; with
    // the registry feature off the field is initialised but never consulted.
    #[cfg_attr(not(feature = "registry"), allow(dead_code))]
    auto_use_catalog_map: Option<std::collections::HashMap<String, String>>,
    /// The builder half of the catalog fallback: `function name -> providing package` from
    /// the sidecar's `builders`, for a bare call on a package the user has not declared.
    #[cfg_attr(not(feature = "registry"), allow(dead_code))]
    auto_use_catalog_builders: Option<std::collections::HashMap<String, String>>,
    /// The catalogue's methods two packages provide (`method -> "a,b"`), for the refusal
    /// that names them (loft#2094).  `None` until first asked.
    #[cfg_attr(not(feature = "registry"), allow(dead_code))]
    auto_use_catalog_ambiguous: Option<std::collections::HashMap<String, String>>,
    /// Is this the first pass on parsing:
    /// - Do not assume that all struct / enum types are already parsed.
    /// - Define variables, try to determine their type (can become clear from later code).
    /// - Claim working text variables for expressions that gather text data outside variables.
    /// - Links between memory allocations (text, stores) their type knows the variable numbers.
    /// - Move variables to a lower scope if an expression still links to their content.
    /// - Determine mutations to stores and administer these in arguments.
    ///
    /// The second pass:
    /// - Creates code, assumes that all types are known.
    // The two-pass contract (H5, @PLAN59): pass 1 registers definitions
    // and FINAL signatures — since @PLAN59 every body-carrying plain fn's
    // hidden `__retbuf` exists from its pass-1 signature parse, so attr
    // counts CANNOT change in pass 2 (`ref_return` debug-asserts it; only
    // lambda-class defs may still grow, and they have no earlier callers).
    // Pass 2 re-parses bodies against those fixed signatures.  Variable
    // tables persist across passes BY NAME (pass 2 re-finds rather than
    // re-creates) — the parser is therefore NOT re-entrant beyond two
    // passes: a third pass leaves tables half-migrated (verified by the
    // #339 third-pass experiment, which segfaulted).  The "attr counts cannot
    // change in pass 2" claim above is now ENFORCED (not just asserted at the
    // `ref_return` growth site) by `assert_pass2_def_attr_stable` — a per-def
    // count snapshot compared end-of-pass-1 vs end-of-pass-2 in debug/armed
    // builds, so any future cross-pass divergence fails loud (H5).
    first_pass: bool,
    /// loft#788 — bare names already refused as ambiguous, so the message is
    /// printed ONCE.
    ///
    /// Per name rather than per site, and in whichever pass reaches it first.
    /// Pass 2 is not reliably reached: a first-wins binding to the wrong
    /// package's struct raises "Unknown field" in pass 1, which is the very
    /// symptom the ambiguity explains — waiting for pass 2 would report it in
    /// exactly the case that needs it least.
    ambiguity_reported: std::collections::HashSet<String>,
    /// @PLN104 P3 — def_nrs the post-pass-2 oracle (`report_tret_promotions`) marks
    /// for `__tret` retbuf promotion (a frame-local text return with no buffer). Read
    /// by `do_tret_bind`'s gate on the THIRD pass so the promotion is forward-ref-safe:
    /// the attr is decided before the pass, so every caller re-lowers with the buffer.
    force_tret: crate::fxhash::FxHashSet<u32>,
    /// `@FR-R-WorkBuffer` — every definition given a work-buffer parameter, kept across
    /// `after_pass2` runs so a caller built later (a rebuilt specialisation) is patched too.
    pub(crate) work_buffer_promoted: crate::fxhash::FxHashSet<u32>,
    /// `@FR-R-WorkBuffer`'s per-frame pool (loft#1697): while callers are patched, the
    /// work-buffer argument a call needs is `(caller, element type, k)` — the k-th of that
    /// element type within ONE call — and every call site of the frame shares it.  Calls in a
    /// frame never overlap and a callee clears its buffer on entry, so one store per slot of
    /// the widest call serves them all.  Minted per SITE, the buffers climbed up the call
    /// tree one per site: a function of 150 calls took 150 hidden parameters, and its caller
    /// 150 more per call.  `None` outside the patching.
    pub(crate) work_buffer_pool: Option<std::collections::HashMap<(u32, String, usize), u16>>,
    /// The pool between patching rounds, while `work_buffer_pool` is closed.
    pub(crate) work_buffer_pool_kept: std::collections::HashMap<(u32, String, usize), u16>,
    /// @PLN167 C3 — some call handed a text field or element to a `&text` parameter, so
    /// `after_pass2` owes the store instances (`store_text.rs`).
    store_text_args: bool,
    /// loft#808 — def_nrs resolved as a `par(r = f(x), N)` worker, recorded on BOTH
    /// passes and never cleared between them.  A worker's pure-value tuple return is
    /// the one shape that must still be BOXED into the synthetic `__tuple<…>` record:
    /// par dispatch carries a worker result through per-route buffers that only cover
    /// ≤8-byte primitives, text, fn-refs and references, so a wider bare tuple is
    /// rejected outright (`parse_parallel_for_loop`'s "return type … is not
    /// supported").  Every OTHER tuple return keeps Rust's tuple ABI.
    ///
    /// Pass 1 fills the set as it reaches each par site, so a worker DECLARED BEFORE
    /// its par site is still bare when `parse_function` runs; the gap closes in
    /// `promote_par_worker_tuple_returns`, between the passes and before the H5
    /// attribute snapshot, exactly like `reserve_late_return_buffers`.  By pass 2 the
    /// set is complete, so `parse_function` reproduces the promoted type.
    par_worker_defs: std::collections::HashSet<u32>,
    /// loft#1040 — one entry per `par` clause whose ROUTE could not be decided where it
    /// was written, because the function holding it is a TEMPLATE and the element and
    /// return types are its type variable.
    ///
    /// A par lowering is more than a call target: it picks a queue variant, an element
    /// and a return SIZE, a result accessor and a re-wrap, all from those two types.
    /// Inside a template they are the type VARIABLE, so a route chosen there is a route
    /// for a 12-byte reference; substitution then rewrites the types and leaves the whole
    /// route behind.  So the template parses the BODY (which is what types it) and stops,
    /// leaving a marker call the monomorph replaces by lowering the same clause again —
    /// against the types it actually has.  Everything that must be substituted rides in
    /// the marker's arguments, where the ordinary machinery reaches it; this side holds
    /// only what substitution cannot change.
    par_deferred: Vec<DeferredPar>,
    /// The body of the par clause being REPLAYED, with the loop-counter variable that
    /// belongs to it, so `build_parallel_for_ir` uses the IR the template already parsed
    /// instead of reading tokens that are long gone.
    par_replay_body: Option<(Value, u16)>,
    /// loft#918 — every `(function, tail variable)` whose promotion pass 1 could not
    /// decide: the tail is a bare local, the function returns `text` / `text?`, and the
    /// local's type is still `Unknown` because it was bound to a call declared lower in
    /// the file.  Consumed by `promote_late_text_buffers` between the passes, where the
    /// callee exists — the same late-derivation slot `reserve_late_return_buffers` uses.
    late_text_tails: Vec<(u32, u16)>,
    /// loft#945 — every short `|x|` lambda whose return type is to be read off its own
    /// BODY, because nothing else names it: no `-> τ` (the short form forbids one) and no
    /// return in the hint that placed it (`map`'s callback, whose `U` is free).
    ///
    /// Consumed by [`block_result`](Self::block_result) at the body's tail, which is the
    /// first moment the type exists AND still early enough to run the return machinery on
    /// it — a text or collection return mints a hidden buffer parameter there.  Adopting
    /// the type any later mints that buffer on pass 2 only, which is the H5 two-pass
    /// contract violation, so the timing is the whole point.
    infer_ret_defs: std::collections::HashSet<u32>,
    /// loft#945 — every def that ACTUALLY adopted its return type from its own body tail,
    /// recorded by [`block_result`](Self::block_result) at the moment it does so.
    ///
    /// [`infer_ret_defs`](Self::infer_ret_defs) says a lambda is a CANDIDATE for that and is
    /// cleared when its body closes; this says the adoption happened and survives to
    /// `reserve_late_return_buffers`, which is between the passes.  The two are not the same
    /// set — a candidate whose tail is scalar, `Void` or `Never` adopts nothing.
    ///
    /// It is the late reservation's admission test for a lambda, and the narrowness is the
    /// point: a lambda that DECLARED its return (`fn(i: integer) -> S { … }`) is served by
    /// the signature-time path and reserving a second buffer for it leaks one store per
    /// closure (`717-closure-struct-return.loft`).  Only a return nothing declared can have
    /// missed the earlier slot.
    adopted_ret_defs: std::collections::HashSet<u32>,
    /// Tuple LOCALS that are the source of a `&` link — bound by `b = &a` / `c: &(…) = a`, or
    /// passed to a `&(…)` parameter — and carry a heap element, keyed `(function, name)`.
    /// Such a local is built as a `__tuple<…>` RECORD so the link can name it (tuples.md
    /// T-Ref); every other tuple local keeps its stack form.  Recorded in pass 1 at the link
    /// and consulted at the bind in pass 2, the same shape as `adopted_ret_defs`.
    ref_linked_tuple_locals: std::collections::HashSet<(u32, String)>,
    /// loft#2146 — statements an argument conversion needs run around the call it is an
    /// argument of: a nullable struct SLOT passed to a `&S?` parameter is copied into an
    /// owned temp BEFORE the call and stored back into the slot AFTER it, while the argument
    /// itself stays the bare `OpCreateStack(temp)` both backends pass as a link.  Cleared by
    /// `process_call_args`, taken by `call_nr` once the call is built.
    pending_arg_pres: Vec<Value>,
    pending_arg_posts: Vec<Value>,
    /// Pass 1's tuple-local call arguments, `(function, callee def name, argument index,
    /// local)`, judged against the callee's signature between the passes
    /// (`record_forward_ref_tuple_links`).
    pending_tuple_args: Vec<(u32, String, usize, String)>,
    /// Set while an argument for a record-backed `&(…)` parameter parses: a struct field of
    /// tuple type is then read with its address, as under `amp_pending` (loft#1883).
    pub(crate) tuple_place_wanted: bool,
    /// loft#1695 — `(function, root variable)` of every COLLECTION loop source a body
    /// replaced (`for p in d.rs { d = … }`), recorded on pass 1 so pass 2 walks a copy taken
    /// at loop start (`@FR-I-For`: the source is evaluated once).  Keyed by the root's NAME,
    /// which both passes agree on; two loops over one root both copy, which is conservative.
    loop_sources_replaced: std::collections::HashSet<(u32, String)>,
    /// `@FR-B-Scope` — the names a pattern has pointed at its own bindings since the last
    /// `=>`, each with the variable it named before (`Parser::pattern_binding`).  The arrow
    /// seals them into a frame of `pattern_bind_frames`; the end of the arm's body restores
    /// that frame, so an arm's names end with the arm.
    pattern_binds_pending: Vec<(String, Option<u16>)>,
    pattern_bind_frames: Vec<Vec<(String, Option<u16>)>>,
    /// @PLN167 C1 — the kind of each `&text` link bound so far on pass 2, keyed by
    /// `(function, variable)`: `true` for a text field or element (the store kind), `false` for
    /// a text variable (the stack kind).  A second bind of the other kind is refused.
    text_link_kinds: std::collections::HashMap<(u32, u16), bool>,
    /// loft#1371 — vector LOCALS bound by a `&` link (`pe = &e`, `pe: &vector<T> = e`),
    /// keyed `(function, name)`.  The bind SHARES the source's `DbRef`, which already
    /// aliases every element write and every append; a WHOLE-VALUE write is the one
    /// operation that would re-point the local at a fresh store instead of reaching the
    /// source, so `create_vector` clears the shared store and refills it in place for
    /// these — the lowering a `&vector` PARAMETER already takes (`@FR-B-Ref-Write`).
    /// Recorded at the link and consulted at the literal, the same shape as
    /// `ref_linked_tuple_locals`.
    amp_vector_locals: std::collections::HashSet<(u32, String)>,
    /// The other member of each `&`-linked vector pair, both ways round — `q = &v` records
    /// `q -> v` and `v -> q`.
    ///
    /// A vector link SHARES the source's store, so the two names are one place: a read of
    /// either is a read of the other, which is what `(I-Comp)`'s *"a build reads what its
    /// destination held when the statement began"* has to be asked about.  `Value::reads_var`
    /// answers for one name, and `a = [for i in 0..r.len() { r[i] }]` under `r = &a` reads the
    /// destination without naming it.
    amp_vector_link_partners: std::collections::HashMap<(u32, String), Vec<String>>,
    /// loft#945 — every `(function, variable)` whose vector LITERAL turned out to be the
    /// RECEIVER of a `.map`/`.filter`/`.reduce` chain (`d = [1, 2, 3].map(…)`) or of a
    /// subscript (`a = [10, 20][1]`, `s = [1, 2, 3][1..]`, loft#1923).
    ///
    /// Deliberately NOT cleared between the passes: by the end of such a statement the
    /// variable holds the CHAIN's type, and the variable table outlives the pass, so pass 2
    /// would otherwise build the literal against a type it is not — a `vector<text>` for an
    /// integer literal, or an integer for a vector one.
    ///
    /// Keyed by the literal's own SOURCE POSITION as well (`line`, `pos` of its `[`): one
    /// variable takes several literals, and only the one that IS a receiver may decline the
    /// destination.  Keyed by the variable alone, `b: vector<integer> = []` lost its
    /// declared type on pass 2 because a LATER `b = [1, 2][0..1]` was a receiver (loft#1923).
    literal_chain_lhs: std::collections::HashSet<(u32, u16, u32, u32)>,
    /// @PLN125 — every bound-method stub built on pass 1, as
    /// `(stub, the interface method it stands in for, the holder that replaced `Self`)`.
    /// Consumed by `refresh_bound_method_stubs` between the passes, where a forward-
    /// referenced return type has finally resolved and the stub's hidden parameters can be
    /// decided correctly. A vector rather than a set: the order is the order stubs were
    /// created, so the refresh is deterministic.
    bound_method_stubs: Vec<(u32, u32, u32)>,
    /// Which interface METHOD each bound stub was minted from — durable across both passes.
    ///
    /// [`Parser::bound_method_stubs`] cannot answer this: `refresh_bound_method_stubs` takes
    /// it between the passes, so on pass 2 — the pass that reports — it is empty.  The
    /// conflict check in `create_bound_method_stubs` needs the ORIGIN to compare this bound's
    /// method against the one already holding the stub name (loft#1301).
    stub_origin: std::collections::HashMap<u32, u32>,
    /// Set by `parse_in_range` when `rev(collection)` (without a `..` range) is parsed.
    /// Consumed by `fill_iter` to add the reverse bit (64) into the `on` byte of OpIterate/OpStep.
    reverse_iterator: bool,
    /// `@FR-Col-Unordered` — set by `parse_in_range` when the iterable is `unordered(c)`, and
    /// taken by `keyed_snapshot`, which then builds a hash's snapshot in slot order instead of
    /// key order.  Any other walk ignores it: key order is one of the orders `unordered` allows.
    unordered_iterator: bool,
    /// D-key-1: true only while parsing the *iterable* of a `for`/comprehension (set with
    /// save/restore around the iterable expression in `parse_in_range`).  A keyed range /
    /// partial-key subscript produces a `for`-only iterator (`Value::Iter`); `parse_key`
    /// reads this flag to reject that subscript in a value position (`x = coll[lo..hi]`)
    /// with a clean diagnostic instead of a parse/codegen panic.
    iterable_context: bool,
    /// O8.5: range bounds captured by `parse_in_range_body` for const-unroll detection.
    pub(crate) last_range_from: Option<Value>,
    /// `@FR-R-LiteralWalk` — the last scalar vector literal `parse_vector` built as a block:
    /// the block, its items (each at the element type) and that type.  `parse_for` takes it
    /// when the block is its iterable and walks the items without the vector.
    pub(crate) literal_walk: Option<(Value, Vec<Value>, Type)>,
    pub(crate) last_range_till: Option<Value>,
    /// The default arms of the `??`s built last, each with the type it SYNTHESISED before it
    /// was brought to the coalesce's type.  A store whose value is a `??` checks its default
    /// against the slot (`range_guard_inside_discharge`), and a `u8` field read lowers to an
    /// op typed `integer`, so an arm's own type cannot be read back off the value.  A chain
    /// `a ?? b ?? c` nests, so the inner arm is asked too; a short list covers that.
    pub(crate) coalesce_defaults: Vec<(Value, Type)>,
    /// `@FR-B-Scope` — the blocks now open, innermost last, each named by a per-parser
    /// ordinal; and, per `(function, local)`, the open-block path where a STATEMENT last bound
    /// that local.  A local bound inside a block ends at that block's `}` (rustc's rule), so a
    /// read from outside the path is refused.  Written and read in source order within a
    /// pass, on BOTH passes and keyed by the pass: a bind after the block ends starts a new
    /// binding (loft#1700), and that decision mints a variable, so pass 1 must make it too.
    pub(crate) block_path: Vec<u32>,
    /// A destructuring `for (a, b) in …`'s binders, bound by the header like the loop variable
    /// and scoped to the body the next `for` block opens.
    pub(crate) pending_loop_binders: Vec<u16>,
    pub(crate) block_ord: u32,
    /// The binding's block is kept as its INNERMOST ordinal (`None` = the function's own
    /// scope): ordinals are unique and a block's ancestors never change, so "the recorded
    /// path is a prefix of the open one" is "its innermost block is still open" — and
    /// recording a binding allocates nothing (the front end's allocations are pinned).
    pub(crate) bound_in_block:
        std::collections::HashMap<(bool, u32, u16), (Option<u32>, Option<u16>)>,
    /// Per open block, the spellings a bind after an ended binding re-pointed there, with the
    /// variable each named before — restored at the block's `}` (loft#1700).
    pub(crate) block_rebinds: Vec<Vec<(String, u16)>>,
    /// The variable a binding name just resolved to when its binding had ENDED — read by the
    /// assignment, which splits it off once the new value's type is known (loft#1700).
    /// With the loop ordinal the ended binding was made in, which decides whether loft#1145's
    /// per-loop split owns the bind instead.
    pub(crate) bind_after_end: Option<(u16, Option<u16>)>,
    /// @PLN35 PC1 — set while matching over a CURSOR (a struct with a `vector<T>` source + an
    /// integer `pos`): `(cursor_var, cursor_def, pos_field_idx, pos_var)`.  `pos_var` holds the
    /// current position (reads are offset by it); the match PREFIX-consumes (gate `pos + fixed <=
    /// len`, not `len == fixed`) and advances `cursor.pos` by the consumed count on a match.  `None`
    /// = a plain vector/stream match (whole-consume) — the default, so those paths are unchanged.
    pub(crate) match_cursor: Option<(u16, u32, usize, u16)>,
    /// @PLN35 PC5 — the attribute index of an OPTIONAL `farthest: integer` field on the cursor
    /// struct, if present.  The match maintains it as a monotonic high-water mark (`max(farthest,
    /// new_pos)`) at every advance, so after a failed parse `cursor.farthest` is the PEG
    /// farthest-reached position — the token to point an error at.  `None` = the cursor has no such
    /// field (tracking is opt-in, so a plain `{ src, pos }` cursor is unaffected).
    pub(crate) match_cursor_farthest: Option<usize>,
    /// @PLN35 PC3 — sub-rule invocation edges `(enclosing_rule, invoked_rule, site)` recorded on
    /// pass 2, so a post-parse well-formedness pass can reject a left-recursive grammar (a cycle in
    /// this graph) at compile time rather than hang at runtime.  Every PC2 invocation is at cursor
    /// position 0 (whole-pattern), so any cycle is non-consuming — it cannot terminate.
    pub(crate) subrule_edges: Vec<(u32, u32, Position)>,
    vars: Function,
    /// Last seen line inside the source code, an increase inserts it in the internal code.
    line: u32,
    /// Wildcard and selective imports waiting to be applied once the target source is fully parsed.
    pending_imports: Vec<PendingImport>,
    /// The `use` statement being parsed was written `pub use` (@C98).
    use_public: bool,
    /// The use region consumed a `pub` that opens a definition rather than a `pub use`;
    /// the definitions loop takes it.  Cleared when the lexer switches files.
    pub_taken: bool,
    /// every (for_source, lib_source, ImportSpec) pair that
    /// `apply_pending_imports` applied during this parse pass.  Retained so
    /// that `resolve_deferred_unknowns` can re-apply them with overwrite
    /// semantics after cyclic `use` declarations have left Unknown stubs.
    applied_imports: Vec<PendingImport>,
    /// @PLN204 — the sources whose regime (`Data::width32`) is settled, so a file resumed
    /// after a `use` keeps the `#width` it began with.
    pub(crate) width_decided: std::collections::BTreeSet<u16>,
    /// `DefType::Unknown` stubs collected by `actual_types_deferred`
    /// during each `parse_file` run.  Resolved (or finally reported) by
    /// `resolve_deferred_unknowns` after all files in the recursion have
    /// had their pass-1 / pass-2 definitions registered.
    deferred_unknown: Vec<(u16, u32, Position)>,
    /// Stubs pass 1 registered for a type named only by a `Name { … }` CONSTRUCTION,
    /// so a declaration parsed later can adopt them the way it adopts the stub a written
    /// type annotation leaves (loft#801).
    ///
    /// They are speculative because the shape alone cannot tell a forward reference from
    /// a typo, so an unadopted one must not become a diagnostic of its own:
    /// `resolve_deferred_unknowns` stays quiet about these and leaves the report to the
    /// construction site's own pass-2 `unknown type '…'`, which names the spelling the
    /// author used and carries the "did you mean" suggestion.  Reporting both is the
    /// two-errors-for-one-typo cascade #376 removed.
    ///
    /// NOT cleared between passes: the stub itself survives (pass 2 finds it by name
    /// rather than registering a second one), so the exemption has to survive with it.
    speculative_type_refs: std::collections::HashSet<u32>,
    /// Forward stubs a QUALIFIED `lib::Type` left in the library's own source, with the
    /// qualifier as written — so an unadopted one is reported as the name the author
    /// wrote, `lib::Type`, and not as a bare `Type` in a file that never mentions it.
    qualified_stubs: std::collections::HashMap<u32, String>,
    /// By-name imports (`use lib::(Name)`) that found nothing while `lib` was SUSPENDED at
    /// its own `use` of this file — a mutual `use` pair, where `lib` has declared nothing
    /// yet.  Checked again once every file has registered (`resolve_deferred_unknowns`):
    /// `(lib_source, into_source, name, bind, public, where the use was applied)`.
    deferred_import_misses: Vec<(u16, u16, String, String, bool, Position)>,
    /// How many identifiers pass 1 could not resolve to anything, and so turned into a
    /// fresh variable of unknown type.
    ///
    /// Pass 1 is the pass that RECORDS definitions, so a name declared later in the file
    /// is simply not there yet — that is the two-pass design working, not an error, and
    /// pass 2 resolves it.  But a PARAMETER default is parsed and stored once, in pass 1,
    /// and a call site replays what was stored: the pass-2 re-parse is discarded.  A
    /// default naming a later constant therefore froze pass 1's answer, which for
    /// `b: integer = a + LATER` is the collapsed `a` — the call answered 5 where 15 was
    /// written, with nothing said (loft#1086).
    ///
    /// A COUNTER rather than a flag: what a caller wants to know is whether the stretch
    /// of source IT parsed contained one, which is the difference between two readings.
    unresolved_names: u32,
    /// How many times pass 1 declined to TYPE something because an operand's type was not
    /// linked yet — the sibling of [`Self::unresolved_names`], and the half it cannot see.
    ///
    /// A call to a function declared further down resolves its NAME (definitions are
    /// recorded before bodies are parsed), so the counter above stays still; what is missing
    /// is the callee's RETURN TYPE. `call_op_as` then defers and returns `Type::Unknown`
    /// WITHOUT building the operator, so `1 + late(0)` is left as the bare `1`; the `&&`
    /// path leaves the operand in place but mistyped. Either way pass 2 re-parses and gets
    /// it right — unless the stretch of source is one that is parsed ONCE, which is what a
    /// parameter default is (loft#1170, the same collapse loft#1086 closed for names).
    ///
    /// A COUNTER for the same reason as its sibling: the question is whether the stretch of
    /// source a caller parsed contained one, which is a difference between two readings.
    unresolved_types: u32,
    /// @PLN115 — record each resolved identifier occurrence during parse.  DEFAULT
    /// OFF (only the LSP parse sets it, S3); zero-cost when off.  See
    /// `doc/claude/plans/115-resolution-index/`.
    record_resolutions: bool,
    /// @PLN115 — the recorded occurrences (empty unless `record_resolutions`),
    /// cleared per parse alongside `deferred_unknown`.
    resolutions: Vec<crate::resolution::Occurrence>,
    /// Whether the most recently parsed expression is from a `not null` field access.
    /// Set by `get_field`; consumed by `handle_operator` to warn on redundant null checks.
    expr_not_null: bool,
    /// The field name for the most recently parsed `not null` field access (for diagnostics).
    expr_not_null_name: String,
    /// The struct attribute the most recently parsed `fn(…)` FIELD read came from —
    /// `(struct def_nr, attribute index)`.
    ///
    /// Set by [`Self::get_field`], taken by `parse_assign_op` before it parses the RHS
    /// (which would otherwise overwrite it with the attribute of a fn-ref read on the
    /// right-hand side). It exists for ONE thing pass 1 cannot do any other way: a fn-ref
    /// assignment must recover which attribute it writes, and the byte offset the read
    /// carries is `u16::MAX` on pass 1 because the struct has no database layout yet — so
    /// the layout-based lookup that is authoritative on pass 2 answers nothing on pass 1,
    /// and pass 1 is exactly when a capturing source has to be RECORDED for the attribute
    /// to get its split layout at all (loft#1072).
    fn_ref_read_attr: Option<(u32, usize)>,
    /// The next lambda ordinal per enclosing function, keyed by that function's name
    /// ([`Parser::next_lambda_name`]).  The same names come out of both passes because each
    /// function's body is parsed in the same token order on both.
    pub lambda_counters: HashMap<String, u32>,
    /// The single **expected type** — the `⇐` checking mode (formal `(T-Chk)`) pushed into
    /// the value currently being parsed, where the operand's own `var_tp` does not already
    /// carry it (a call argument, a function return-body tail, an `f#read` RHS).  Set before
    /// the value is parsed and reset to `Type::Unknown(0)` after.  There is ONE channel, not
    /// four: readers dispatch on its SHAPE via the helpers below —
    /// - a `Type::Function` → short-form lambda (`|x| {…}`) parameter inference ([`Self::lambda_hint`]);
    /// - an enum type → a bare value-position variant (`f(Red)`) resolves against it ([`Self::enum_hint`]);
    /// - a `Type::Vector` of a concrete element type → a bare literal (`[10,255,20]`, `[1, 2]`,
    ///   `[North, South]`) is checked element by element against it (`(T-Chk-Vec)`,
    ///   [`Self::vector_hint`]);
    /// - any type → an `f#read` infers its byte width from it ([`Self::read_target_type`]).
    ///
    /// (`var_tp` already carries the type for typed-local decls / `==` / struct-field init,
    /// so those need no push.)  Consolidating the former four `*_hint` fields is
    /// [formal/types.md D1](../../doc/claude/formal/types.md) — one judgment, not four side-channels.
    pub(crate) expected: Type,
    /// Set by `dynamic_dispatcher` when it refused a leaf, so the call site returns rather
    /// than falling to the ladder and reporting the same site a second way.
    pub(crate) reported_dynamic_refusal: bool,
    /// `@FR-Disp-Hint` — while an argument of a call to an overload set is parsed without a
    /// hint, the set's definitions as a refusal names them; a short lambda whose parameter
    /// thereby has no type takes it and says so, naming them (`parse_lambda_short`).
    pub(crate) unhinted_set: Option<String>,
    /// That refusal was made: the call it belongs to ends there, so the definitions are not
    /// reported a second time against an argument typed `fn(unknown)`.
    pub(crate) unhinted_set_refused: bool,
    /// A call refused because two parameters bind one type variable to two types (@PLN165
    /// C1, F9): the refusal names both, and `call` answers nothing further for it.
    pub(crate) reported_binding_clash: bool,
    /// How many functions of each name each program source FILE declares, by a lexical scan —
    /// pass 1's answer to "is a definition of this name still below the call?"
    /// (`Parser::file_has_pending_fn`).
    pub(crate) declared_fn_names: HashMap<String, HashMap<String, usize>>,
    /// The `{id}#index` variables of loops that walk values WITHOUT positions — a generator,
    /// or a type's own `next()` — keyed by (function, variable): `x#index` is the position to
    /// give to `v[i]`, and such a loop has none to offer.
    pub(crate) positionless_loops: HashSet<(u32, u16)>,
    /// Set by `iter_op` when `#fields` is encountered. Holds the struct `def_nr`.
    /// Checked by `parse_for` to take the unrolling path. Reset after use.
    pub(crate) fields_of: u32,
    /// Outer-scope variable names and types, populated when parsing a lambda body.
    /// When a variable is not found in the lambda's scope but exists here, it is a capture.
    /// Empty when not inside a lambda.
    pub(crate) capture_context: Vec<(String, Type)>,
    /// Which DEFINITION owns each name in [`Parser::capture_context`] — the function whose
    /// variable table it lives in.
    ///
    /// For a lambda directly inside a function every visible name is owned by that function,
    /// and this says nothing the enclosing context does not.  For a lambda nested inside a
    /// LAMBDA it is the difference between the enclosing lambda and the scope the name really
    /// comes from, which is what decides where a mutated capture is boxed: the cell has to be
    /// minted in the frame that holds the variable, not in the closure that passes it along
    /// (loft#1236).
    pub(crate) capture_owner: std::collections::HashMap<String, u32>,
    /// loft#1540 — the names in `capture_context` whose value is read-only: an enclosing binding
    /// marked value-const, or a capture the enclosing lambda itself received read-only.  A
    /// capture shares a record or collection with the binding it names (LOFT_LITERALS.md § Closures), so
    /// the closure may write it no more than that binding may.
    pub(crate) capture_const: std::collections::HashSet<String>,
    /// The `capture_const` of each enclosing lambda, restored where `capture_context` is.
    pub(crate) capture_const_saved: Vec<std::collections::HashSet<String>>,
    /// loft#1869 — the closure LAMBDA each fn-ref local of the enclosing scope holds, by name,
    /// where the local is backed by a closure record built in that scope.  A lambda capturing
    /// such a fn-ref keeps the inner closure in a fn field of its own record, laid out for that
    /// lambda's record.  Saved and restored with `capture_const`.
    pub(crate) capture_fn_lambda: std::collections::HashMap<String, u32>,
    /// The capturing lambda the expression just parsed built, both passes — what an assignment
    /// of it to a fn-ref local notes ([`Function::note_fn_lambda`], loft#1869).
    pub(crate) last_closure_lambda: u32,
    pub(crate) capture_fn_lambda_saved: Vec<std::collections::HashMap<String, u32>>,
    /// Captures a lambda REBINDS whole-value (`p = [..]`), keyed by the lambda's def.
    ///
    /// Recorded where the assignment is parsed, because by the time the lambda closes its
    /// body is already lowered — a vector rebind is a clear plus appends by then, and the
    /// `Value::Set` that says "whole value" is gone. Judged at the lambda's exit, which is
    /// the first point that has the ENCLOSING variable table back and can ask whether the
    /// captured name is a parameter (loft#1281).
    pub(crate) rebound_captures: std::collections::HashMap<u32, Vec<String>>,
    /// Accumulates captured variable names and types during lambda body parsing.
    /// Reset at the start of each lambda; read after parsing to synthesize the closure record.
    pub(crate) captured_names: Vec<(String, Type)>,
    /// The vector locals whose value-branch bind was written out per arm
    /// (`sink_vec_bind_into_arms`), by defining function and name.  The return-promotion
    /// ladder reads "bound to a branch" off the body's `Set(v, If)`, a shape the rewrite
    /// removes, and this carries the fact instead: such a local keeps its own store and is
    /// copied into the return buffer at the return (`Bind`), exactly as the value form was.
    /// Renamed onto the buffer it would be filled by neither pass — the rewrite declines a
    /// hidden parameter, and the value form assigns the arms' temps into the buffer var.
    /// Keyed by name because a pass rebuilds the variable table; recorded on both passes.
    pub(crate) branch_sunk_vectors: std::collections::HashSet<(u32, String)>,
    /// The closure-record attributes that SHARE a captured vector, by `(record, attribute)`,
    /// with the vector's type.  `closure_attr_type` stores such a capture as a 12-byte `DbRef`
    /// spelled `Reference(element)` — the spelling a shared RECORD capture has too — so the
    /// drop cascade reads this to walk the vector rather than release one element-typed
    /// record at the vector's slot (loft#1606).
    pub(crate) closure_shared_vectors: std::collections::HashMap<(u32, String), Type>,
    /// loft#1874 — `(lambda, captured name)` → the attribute of the fn field's UNION closure
    /// record that holds it, where another lambda of the field captures the same name as a
    /// different type ([`Parser::unite_field_closure_records`]).  Read through
    /// [`Parser::capture_attr`].
    pub(crate) capture_alias: std::collections::HashMap<(u32, String), String>,
    /// Variable number of the __closure parameter inside a lambda body (second pass).
    /// `u16::MAX` when not inside a capturing lambda.
    pub(crate) closure_param: u16,
    /// The type variables the enclosing generic header declares, each as its SOURCE spelling
    /// (`"T"` for `fn f<T: …>`) and the placeholder definition it binds; empty outside a
    /// generic function.
    ///
    /// `formal/interfaces.md` `(G-Gen)`: a generic header *introduces* its variables, so the
    /// binding is per-header.  Two functions that both write `T` name two different
    /// variables, and `parse_type` therefore resolves a spelling against THIS header before
    /// it asks the global definition table (loft#1300, loft#1301) —
    /// [`Self::def_nr_in_scope`].  @PLN25 E2 reads it too: a `vector<T>` over a header's
    /// variable stays dense, since nullability is decided by whatever the instantiation
    /// binds.  Reset per function.
    pub(crate) cur_type_vars: Vec<(String, u32)>,
    /// The bindings of the instance whose body is being filled — every type variable and
    /// associated type it binds — read where a deferred site needs them and has no argument
    /// to carry them: a template LAMBDA is instantiated with its enclosing instance's
    /// bindings ([`Self::instantiate_template_lambda`]).  Empty outside an instantiation.
    pub(crate) instance_bindings: Vec<(u32, Type)>,
    /// The work-refs a [`Parser::TV_SELECT`] lowering minted in the instance being filled —
    /// the return buffers of the calls it made, which [`Parser::fill_monomorph_body`] declares
    /// at the instance's top level as the function parse declares its own (@PLN165 B3b).
    pub(crate) set_call_refs: Vec<u16>,
    /// @PLN165 D7 — instances of generic structs found to contain themselves when first laid
    /// out on pass 2, each reported once.
    pub(crate) cyclic_instances: std::collections::HashSet<u32>,
    /// A forward reference to a generic struct just had its `<…>` read and set aside
    /// (`skip_forward_type_args`); read by the `reference<…>` arm.
    pub(crate) forward_template_args: bool,
    /// A return type's forward generic struct and its arguments, parked until the function's
    /// definition is known (`note_forward_return`).
    pub(crate) pending_forward_return: Option<(u32, Vec<Type>)>,
    /// `(function, stub, arguments)` of every forward generic return, resolved between the
    /// passes (`resolve_forward_generic_returns`).
    pub(crate) forward_generic_returns: Vec<(u32, u32, Vec<Type>)>,
    /// How many generic literals are reading their values to learn their instance
    /// (`literal_instance`) — a read that must define nothing, so a lambda met there is not
    /// parsed as one (`lambda_signature`, `skip_short_lambda`).
    pub(crate) discovering: u32,
    /// A short lambda was stepped over by that read: the value it sits in binds nothing.
    pub(crate) discovery_skipped_lambda: bool,
    /// The variables a `struct` / `enum` header wrote where the language refuses one: its
    /// fields may name them, and the header's refusal already covers that.
    pub(crate) refused_header_vars: Vec<String>,
    /// The struct whose header `bind_type_header` is binding (@PLN165 D2).
    pub(crate) context_type_template: u32,
    /// The placeholder definition standing for a `(type-variable spelling, bound set)` pair.
    ///
    /// Sharing one placeholder across generic functions is what lets the stdlib's many
    /// `<T>` templates resolve against one definition, and it stays the norm — but the
    /// placeholder is also what KEYS the bound-method stubs (`t_<LEN>T#g_<method>`), and a
    /// stub carries a SIGNATURE.  Two headers whose bounds require different signatures of
    /// one method name therefore need different placeholders, or the second's calls are
    /// checked against the first's stub and refused against a signature the reader never
    /// wrote.  Keyed on the bound NAMES as written, which are the same on both passes even
    /// where an interface is still a forward reference.
    pub(crate) type_var_holders: std::collections::HashMap<(String, String), u32>,
    /// The bound-set key each placeholder in [`Self::type_var_holders`] was claimed for, so
    /// a header meeting an already-claimed spelling can tell "the same variable again" from
    /// "a second variable that happens to be spelled the same".
    pub(crate) type_var_bounds: std::collections::HashMap<u32, String>,
    // last closure work variable created by emit_lambda_code (transient).
    pub(crate) last_closure_work_var: u16,
    // closure allocation expression to inject at the call site.
    pub(crate) last_closure_alloc: Option<Box<Value>>,
    // outer variable numbers captured by the most recently parsed lambda.
    // Consumed by try_fn_ref_call to mark them as read at call-injection time.
    pub(crate) last_closure_captured_vars: Vec<u16>,
    /// loft#1676 — per (function, local): how many closure builds captured it, and how many of
    /// those were yielded and took a copy (`yield_owned_closure`).  Equal counts mean the local
    /// is the generator's own again.
    pub(crate) closure_capture_builds: std::collections::HashMap<(u32, u16), u32>,
    /// loft#1862, `@FR-L-CapRebind` — per (function, local): the closure records built over it
    /// so far in THIS pass, as (record local, capture slot position).  A keyed rebind asks them
    /// whether a record still holds the store the local names ([`Parser::keyed_rebind_mint`]).
    /// Entries carry the pass, so pass 2 reads only the builds it has already parsed — every
    /// assignment after one of them is a reassignment.
    pub(crate) capture_records: std::collections::HashMap<(bool, u32, u16), Vec<(u16, u16)>>,
    pub(crate) yield_copied_captures: std::collections::HashMap<(u32, u16), u32>,
    /// `@FR-H-RefHome` — the local vectors a stored `reference<T>` was set into, as (function,
    /// local).  Kept across both passes, so a reassignment parsed before the write that names
    /// it (a later statement of a loop body) still knows: such a local is emptied in place
    /// when reassigned, and the reference reads null (`@FR-H-Removed`).
    pub(crate) referenced_locals: crate::fxhash::FxHashSet<(u32, u16)>,
    /// The `referenced_locals` pass 2 has seen assigned already: the next `=` is a
    /// reassignment.
    pub(crate) referenced_assigned: crate::fxhash::FxHashSet<(u32, u16)>,
    /// #314: capturing lambdas synthesized during each function body in
    /// pass 1, keyed by the enclosing context's def_nr.  Consumed by
    /// `reject_shared_mutable_scalar_captures` at the parent's body end
    /// — the first moment the parent's `scalars_to_box` accumulation is
    /// complete — to diagnose a mutated scalar captured by more than
    /// one closure (GOALS.md § "Stability trumps features").
    pub(crate) fn_lambdas: std::collections::HashMap<u32, Vec<u32>>,
    /// #91: when > 0, record $.<field> accesses for circular-init detection.
    /// Decremented after each init(expr) is parsed.
    pub(crate) init_field_tracking: bool,
    /// #91: field names accessed via $ during the current init(expr) parse.
    pub(crate) init_field_deps: Vec<String>,
    /// @PLN196 step 9 — the paths of the stored fields the field default being parsed reads
    /// through `$` (`label`, `inner.label`), and each read already built with its path, so a
    /// read of a read extends it.
    pub(crate) default_paths: Vec<String>,
    pub(crate) default_path_reads: Vec<(Value, String, u32)>,
    /// Whether the init(expr) being parsed mentions `$` at all.
    ///
    /// Separate from [`Self::init_field_deps`], which records the RESOLVED field a `$.x`
    /// selects and so answers a different question: the cycle check needs names, and a
    /// name it cannot resolve is not part of a cycle it can see.  Whether the default
    /// reads the record is not about the field — a `$` that selects a field declared
    /// LATER in the struct still reads the record, and pass 1 cannot resolve that field.
    ///
    /// Reading the deps for this made the answer differ between the passes: pass 1 said
    /// "reads nothing" and lowered `a: integer = $.b` into a function, pass 2 said "reads
    /// the record" and did not, and the function pass 1 left behind held a `$` no frame
    /// ever allocated (loft#1086).
    pub(crate) init_reads_record: bool,
    /// @PLN196 — while a field default that reads `$` and builds a value is re-parsed into a
    /// function of its own, the struct whose record `$` is there: inside that function `$`
    /// is its first parameter, `self`.
    pub(crate) dollar_record: Option<u32>,
    /// @PLN196 — synthesise `__fill_<T>__<field>(self: T)` for every field whose default is
    /// not a constant, so a store upgrade can fill an added field the way a constructor does.
    /// Off for an ordinary parse: only `loft layout upgrade` and a live swap ask for them.
    pub upgrade_fills: bool,
    /// M11-a: true while parsing the body of a `for … par(…) { … }` loop.
    /// `yield` inside a `par()` body is illegal — the worker runs in a separate
    /// thread with its own store; there is no safe coroutine resumption path.
    pub(crate) in_par_body: bool,
    /// Field-capture aliases created by `if expr is Variant { field } { body }`.
    /// Drained by `parse_if` after the body to restore previous name mappings.
    pub(crate) is_capture_aliases: Vec<(String, Option<u16>)>,
    /// Where a field-capture list that was reported as standing in the body's place ended
    /// (loft#1828): the next token, when it is not `{`.  `parse_block` reads a body that
    /// starts at exactly this token as the one the capture already stood for, so the
    /// construct's own `{` demand does not report the same mistake again.
    pub(crate) capture_took_body: Option<crate::lexer::Position>,
    /// The variable named as the target of the `=` being parsed (loft#1816), or `u16::MAX`.
    pub(crate) assign_target_discounted: u16,
    /// Whether the route lowering that `=` already took its target's mention back from `uses`.
    pub(crate) assign_target_taken: bool,
    /// Post-2c: captures the most recently parsed `as <alias>` cast target's
    /// def_nr when the alias has a `size(N)` annotation.  Consumed by
    /// `append_to_file` so that `f += x as i32` narrows the serialised
    /// payload to the alias's byte width.  Reset to `u32::MAX` at the start
    /// of each top-level statement; irrelevant outside file-I/O `+=`.
    pub(crate) last_cast_alias: u32,
    /// The narrow target `N` of the just-parsed checked narrowing cast
    /// (`e as N?`, or `e as N` immediately left of `??`).  A discharging `??`
    /// reads this so its result types as `N` instead of the full `integer` the
    /// nullable cast has to carry for its null sentinel (see
    /// `build_null_coalesce_default`).  Set in the `as` handler, consumed by the
    /// next `??`, and cleared by any other intervening operator.
    pub(crate) dn4_checked_narrow: Option<Type>,
    /// True while the `as` handler runs the conversion for an EXPLICIT cast.
    ///
    /// `convert` serves both the implicit stores (a local annotation, an argument, a
    /// field) and the body of an `as` cast, so the implicit-narrowing diagnostic it
    /// raises has to know which one it is in.  Without that, the widened `integer` →
    /// `i32` test (loft#931) fires on the very cast it prescribes as the cure — and on
    /// every `f += (i as i32)` besides.  An explicit cast keeps the range-containment
    /// rule alone; DN4 is what governs it.
    pub(crate) in_explicit_cast: bool,
    /// @FR-N-Store — the slot a `convert` is storing into, pushed by [`Parser::convert_store`]
    /// for the duration of that one conversion (a stack: a nested literal converts inside an
    /// outer one).  `convert`'s `τ? ⤳ τ` arm reads the top for its wording; an EMPTY stack
    /// still asks, worded generically — a site that forgot to name its slot degrades the
    /// message, never the rule.
    pub(crate) store_ctx: Vec<StoreCtx>,
    /// @FR-N-Store — depth of [`Parser::convert_admitting`]: a null TEST, a condition or an
    /// overload trial converts `τ?` to `τ` without storing anything, and says so here.
    pub(crate) admit_unwrap: u32,
    /// @PLN116 `x?` — a pre-built default RHS for the postfix default-fallback
    /// operator.  `x?` desugars to `x ?? construct_default(T)`; rather than parse a
    /// `??` right operand from source, the postfix-`?` site builds the type's default
    /// value here and `build_null_coalesce_default` consumes it INSTEAD of parsing —
    /// so `x?` reuses the whole `??` emission path and is bytecode-identical to a
    /// hand-written `x ?? <default>`.  Set at the `?` site, taken exactly once.
    pub(crate) pending_default_rhs: Option<(Value, Type)>,
    /// @PLN116 `x?` — a synthetic default-value SOURCE for the postfix operator, used
    /// when the default must be parsed IN the `??` right-operand context rather than
    /// pre-built (an empty collection `[]`, whose ownership view-model depends on that
    /// context — parsing it standalone leaks).  `build_null_coalesce_default` swaps the
    /// lexer to this source at its own parse site, so `x?` matches `x ?? []` exactly.
    pub(crate) pending_default_src: Option<String>,
    /// @C118 — the slot a NULL collection local is given before a `c = &a` bind shares its
    /// handle (`Parser::null_local_slot`).  Built where the bind's right side is parsed and
    /// placed before its statement by `parse_assign_op`; empty between binds.
    pub(crate) pending_link_slot: Vec<Value>,
    /// loft#1003 — where the `??` default ENDED, for the `redundant-coalesce` deletion
    /// span.  Set as the default is parsed and consumed by `handle_null_coalesce`, because
    /// the notice fires before the default exists and the caller's cursor has moved past
    /// the statement terminator by the time it returns.
    pub(crate) ncc_default_end: Option<(u32, u32)>,
    /// The last null-check built by [`Parser::build_null_coalesce_default`] came from the
    /// postfix `x?` discharge (@PLN116), not from an explicit `a ?? d`.
    ///
    /// The two spellings build the SAME IR and mean different things on an assignment
    /// place: `x?` names one place and says what to read when it is null, while `a ?? d`
    /// names two values and no place at all.  Read at the assignment dispatcher, which
    /// peels the first and leaves the second refused (loft#1205).
    pub(crate) last_place_discharge: bool,
    /// loft#1023 — definitions whose BODY has been parsed in the current second pass.
    ///
    /// A generic instantiated before its own template appears in this set was built from
    /// the pass-1 body; [`Parser::rebuild_stale_monomorphs`] redoes those at the end of
    /// the pass, when every template body exists.
    pub(crate) pass2_bodies: std::collections::HashSet<u32>,
    /// loft#1023 — `(monomorph, template, bindings)` for each monomorph built
    /// from a template whose pass-2 body had not been parsed yet.
    pub(crate) stale_monomorphs: Vec<(u32, u32, Vec<(u32, Type)>)>,
    /// Generic templates refused at their own declaration (loft#1538): a call to one is
    /// already an error the declaration names, so the call adds no second message.
    pub(crate) refused_templates: std::collections::HashSet<u32>,
    /// @PLN99 Arc C — set by `convert` when it dispatches a struct/reference-returning
    /// USER conversion (`x as T` via `fn OpConvTFromS`).  Such a conversion ALLOCATES a
    /// fresh owned store, so its result must NOT inherit the source's deps (the reinterpret-
    /// cast graft would mark it a view and leak the new store).  The `as` handler reads this
    /// right after `convert` to use the conversion fn's real (Owned) return type as the
    /// result, then clears it.
    pub(crate) conv_owned_result: Option<Type>,
    /// Field-binding Set nodes created by `if expr is Variant { field }`.
    /// Drained by `parse_if` and prepended to the if-body so they only
    /// execute when the discriminant matches.
    pub(crate) is_capture_bindings: Vec<Value>,
    /// `--show-types --trace`: when `true`, `parse_part` appends one
    /// trace line per resolved sub-expression (after each `.field`,
    /// `.tuple_idx`, `[idx]` step).  Surfaces dep-tracking flow that
    /// the per-variable view misses — e.g. P197 was a missing dep on
    /// the `.0` step of a tuple field read.
    pub trace_types: bool,
    /// Accumulated trace entries; drained by the introspection CLI.
    /// Each entry is a tab-separated record:
    /// `<fn_name>\t<line>:<col>\t<step_kind>\t<type_with_deps>`.
    pub trace_types_lines: Vec<String>,
    /// Plan-07 phase 4h — per-(struct_d_nr, attr_idx) read counter.
    /// Incremented by `Parser::field()` on the second pass each time a
    /// field is READ (not assigned) on a user struct.  Surfaced at
    /// end of parse: each non-`not_null` field whose read count >=
    /// `HINT_NOT_NULL_THRESHOLD` AND whose `defended_field_reads` set
    /// does NOT contain the (d_nr, attr_idx) gets a `Level::Warning`
    /// at its declaration suggesting `not null`.  Stdlib (`self.default
    /// == true`) is exempt.  Silenceable via `LOFT_NO_HINT_NOT_NULL=1`.
    pub(crate) field_read_counts: std::collections::HashMap<(u32, u32), u32>,
    /// Plan-07 phase 4h — set of (struct_d_nr, attr_idx) that have at
    /// least one defensive read site (`obj.field ?? default` or `if
    /// obj.field != null` flow analysis).  Membership in this set
    /// suppresses the `not null` hint regardless of read count — the
    /// developer has explicitly acknowledged null is possible.
    pub(crate) defended_field_reads: std::collections::HashSet<(u32, u32)>,
    /// @PLN25 DN3 flow-narrowing — the stack of local-var slots PROVEN non-null by an
    /// enclosing guard (`if v != null { … }` / `if v { … }`, the `== null` else-arm).
    /// A read of `Var(v)` while `v ∈ this` types as the peeled (non-null) base, not `τ?`.
    /// Pushed on entry to the proven branch, truncated to the saved length on exit; a
    /// reassignment of `v` inside the branch removes it (the proof no longer holds).
    pub(crate) narrowed_non_null: Vec<u16>,
    /// loft#2025 — a match arm's root capture (`w: Circle { r } => …`) and the variant its
    /// pattern fixed, so a field access through the capture is the checked one the arm is.
    pub(crate) capture_variant: std::collections::HashMap<u16, u32>,
    /// loft#1540 — the VIEWS of a value-const value, keyed by `(context, view variable)`, each
    /// with a description of the value-const place it reads out of (`Const-Value`; plan 40's
    /// coherence rule 2: *"reading a field/element of a `const` value yields a `const` view"*).
    /// The view is marked value-const itself, so every write guard refuses a write through it
    /// unchanged; this record is only what the refusal SAYS, because the author never wrote
    /// `const` on the view.
    pub(crate) const_views: std::collections::HashMap<(u32, u16), String>,
    /// `@FR-Const-Foreign` — the variables value-const because their first binding is a
    /// `-> const T` producer's value (`bind_foreign_value`): the author wrote no `const`, so a
    /// refused write names the copy as its cure instead.
    pub(crate) foreign_bound: std::collections::HashSet<(u32, u16)>,
    /// The same proof for a PROJECTION rather than a name — `if !db.map[k] { … } else { … }`
    /// proves `db.map[k]` non-null in the else arm, and nothing named it before.
    ///
    /// A separate list because the fact has a different LIFETIME: a name's proof dies at the
    /// next write to that name, which the parser can see; a projection's dies at anything that
    /// could touch any part of it, which it cannot.  So this one is cleared at every statement
    /// boundary rather than tracked — the proof holds inside ONE expression, which is exactly
    /// the shape the idiom needs (`else { db.map[k].val }`) and nothing wider.
    pub(crate) narrowed_non_null_exprs: Vec<Value>,
    /// @PLN25 DN3 fault-op narrowing — local-var slots PROVEN non-zero by an enclosing
    /// `if v != 0 { … }` guard. A division/mod whose divisor is in this set (or a constant
    /// non-zero literal) is provably fit and types NON-null; otherwise it types `τ?`. Same
    /// push/truncate/invalidate discipline as `narrowed_non_null`.
    pub(crate) divisor_nonzero: Vec<u16>,
    /// `(subject, discriminant)` pairs an enclosing `if s is Variant { … }` proves in its THEN
    /// branch — a variable or a projection (`code[b]`), compared by IR shape
    /// (`same_projection`) — so a cast `s as Variant` there is checked directly.
    pub(crate) variant_proven: Vec<(Value, i32)>,
    /// `@FR-N-Domain`'s guard licence for the domain-partial MATH family — local-var slots
    /// proven `> 0` (`Pos`) or `>= 0` (`NonNeg`) by an enclosing comparison against zero.
    /// `domain_sign` reads it for a bare `Value::Var`, so a guarded `sqrt(x)` types non-null
    /// exactly as a guarded `a / d` and a guarded `v[i]` already do.  Same
    /// push/truncate/invalidate discipline as `divisor_nonzero`; the fact is a `Sign` rather
    /// than a bool so it composes with the expression lattice instead of sitting beside it.
    pub(crate) math_sign_proven: Vec<(u16, Sign)>,
    /// #673 — struct-enum `text` payload bindings that WRITE THROUGH to the subject.
    ///
    /// A heap payload binding (`vector`/`ref`/struct-enum) holds a DbRef into the
    /// subject's record, so `items += …` mutates the subject for free. A `text`
    /// payload binding is an owned copy of the field's characters
    /// (`_mv_items = OpGetText(subj, off)`), so the same write would land in the copy
    /// and vanish. Keyed `(enclosing fn, binding var)` → the `OpGetText(subject, off)`
    /// read the binding was initialised from, which `parse_assign` turns back into an
    /// `OpSetText` after every write. Only bindings whose subject expression is
    /// re-evaluable (no user call) are recorded.
    pub(crate) text_payload_views: std::collections::HashMap<(u32, u16), Value>,
    /// @PLN25 DN3 fault-op (index) — set by `parse_vector_index` for each SCALAR `v[i]` read
    /// (true = the index is provably in-bounds: a non-negative constant, a for-loop iter var, or
    /// a var proven `< len(v)` by an enclosing guard), read immediately after by `parse_index` to
    /// decide whether the element type wraps `Optional`. Write-then-read per read; nested reads
    /// (`v[w[j]]`) set inner-then-outer so the outer read sees the outer index's fit.
    pub(crate) last_index_fit: bool,
    /// @PLN25 DN3 fault-op (index) — the `(idx_var, vec)` pairs proven in-bounds by an enclosing
    /// `if idx < len(vec) { … }` guard (skip-pattern 5). A `vec[idx]` whose pair is here types
    /// NON-null. Pushed for the THEN branch in `parse_if`, truncated on exit — same discipline as
    /// `divisor_nonzero`, but keyed on the (index, vector) pair via `VecKey`.
    pub(crate) index_bounded: Vec<(u16, crate::parser::operators::VecKey)>,
    /// Plan-07 phase 4h — site of the most recently parsed field
    /// read.  Set by `Parser::field()` after each read, taken by
    /// `handle_null_coalesce` to mark the read as defended when
    /// `expr ?? default` follows.  `None` between distinct
    /// expressions / statements.  Conservative: covers the common
    /// `p.field ?? default` shape; complex expressions like
    /// `(p.field + 1) ?? 0` and `if p.field != null` are
    /// under-detected today (slice 2).
    pub(crate) last_field_read_site: Option<(u32, u32)>,
    /// @PLAN12 Phase 6.7 — dedupe set for the per-invocation advisory
    /// check.  Once a `(name, version)` tuple has been classified
    /// against the advisory feed during this parser's lifetime, we
    /// don't re-fire the warning.  Critical-severity matches abort
    /// via `process::exit(3)` before the second probe could fire
    /// anyway; this dedupe is for the warning/note tiers.
    #[cfg(feature = "registry")]
    advisory_checked: std::collections::HashSet<(String, String)>,
}

/// The binary operator levels, loosest first.
///
/// Enforces @FR-G-Prec — level N binds tighter than level N-1, and parsing is
/// precedence-climbing over exactly this table, so the table IS the precedence.  A level's
/// operators all bind equally.
///
/// @FR-G-Assoc is NOT in this table: associativity is decided where `parse_operators`
/// recurses, by the precedence it hands the RHS.  Every level is LEFT-associative (RHS at
/// `precedence + 1`) except `**`, which passes its own level so it re-enters itself.  `as`
/// is not associative at all — its right side is a TYPE, not an expression.  So changing
/// this table alone cannot change associativity.
///
/// `??` sits at the loosest level on purpose (@FR-G-Post-Default): a default-fallback binds
/// after everything it might supply a default for.  The lexer coalesces `??` by maximal
/// munch (@FR-G-Lex-Coal), so it never arrives here as two `?` tokens.
static OPERATORS: &[&[&str]] = &[
    &["??"],
    &["||", "or"],
    &["&&", "and"],
    &["==", "!=", "<", "<=", ">", ">="],
    &["|"],
    &["^"],
    &["&"],
    &["<<", ">>"],
    &["-", "+"],
    &["*", "/", "%"],
    &["**"],
    &["as"],
];

/// The binary operators that cannot BEGIN an expression, so one that follows an operand
/// answering nothing belongs to that operand (`parse_operators`).  `-`, `+`, `|` and `&` are
/// absent: each can open the NEXT statement (`-1`, a lambda, a reference).
static VOID_LEFT_OPERATORS: [&str; 18] = [
    "??", "||", "or", "&&", "and", "==", "!=", "<", "<=", ">", ">=", "^", "<<", ">>", "*", "/",
    "%", "**",
];

static SKIP_TOKEN: [&str; 8] = ["}", ".", "<", ">", "^", "+", "-", "#"];
/// Tokens that END a `{x:…}` spec rather than starting its WIDTH expression.  The radix
/// LETTERS are deliberately absent: [`radix_for`] is their one home, and this list plus
/// that function is the whole domain.
static SKIP_WIDTH: [&str; 2] = ["}", "."];

/// Which radix does the letter closing a `{x:…}` spec select — and, by the same answer,
/// is there a radix letter here at all?  `None` means the token starts a WIDTH expression.
///
/// @FR-F-Spec — the one home for the radix half of a spec.
///
/// One function because it is one question asked twice: the spec parser must decide
/// whether to parse a width before it can read the radix.  While those two decisions came
/// from two lists they disagreed — the radix reader accepted `J`, `json` and any case of
/// either, and the skip list named only `j`.  So the width expression consumed the rest:
/// `{p:json}` reported *"Unknown variable 'json'"*, and with a variable of that name in
/// scope it silently took that variable's value as the WIDTH and rendered the loft form
/// instead of the JSON the author asked for.
///
/// `-1` is the JSON pseudo-radix (`OutputState::db_format` reads its sign); `1` is
/// scientific notation; [`ops::HEX_UPPER`](crate::ops::HEX_UPPER) is hexadecimal in upper
/// case, which needs a value of its own because `16` renders lower case.
pub(crate) fn radix_for(id: &str) -> Option<i32> {
    let lower = id.to_lowercase();
    if lower == "j" || lower == "json" {
        Some(-1)
    } else if id == "x" {
        Some(16)
    } else if id == "X" {
        Some(i32::from(crate::ops::HEX_UPPER))
    } else if id == "b" {
        Some(2)
    } else if id == "o" {
        Some(8)
    } else if id == "e" {
        Some(1)
    } else if id == "d" || id == "f" {
        Some(10)
    } else {
        None
    }
}

pub(crate) struct OutputState<'a> {
    pub(crate) radix: i32,
    pub(crate) width: Value,
    pub(crate) token: &'a str,
    pub(crate) plus: bool,
    pub(crate) note: bool,
    pub(crate) dir: i32,
    pub(crate) float: bool,
    /// @PLN99 Arc B — the raw `{x:spec}` spec string for a custom-type value whose
    /// own `to_text(self, spec)` renders it (`""` for a bare `{x}` or a built-in).
    pub(crate) spec: &'a str,
}

impl OutputState<'_> {
    pub(crate) fn db_format(&self) -> i32 {
        i32::from(self.note) + if self.radix < 0 { 2 } else { 0 }
    }
}

pub(crate) const OUTPUT_DEFAULT: OutputState = OutputState {
    radix: 10,
    width: Value::Int(0),
    token: " ",
    plus: false,
    note: false,
    dir: 2, // 2 = unset; text defaults to left (-1), numbers to right (1)
    float: false,
    spec: "",
};

// Sub-modules
pub(super) mod abstract_alias;
pub(super) mod builtins;
pub(super) mod collections;
pub(super) mod control;
pub(super) mod definitions;
pub use definitions::UpgradeRoutineDef;
pub(super) mod dispatch;
pub(super) mod expressions;
pub(super) mod fields;
pub(super) mod fit;
pub(super) mod objects;
pub(super) mod operators;
mod result_buffer;
use closures::{
    callee_param_writes, collect_param_reassigns, collect_param_rebinds_owned, collect_vars_in,
    emit_fn_ref_field_write, lambda_mutated_captures, merge_dependencies,
};
use imports::{ImportSpec, PendingImport};
use packages::{file_exists, registry_fn_hint, script_dir};

/// Does user function `fn_nr` write through its parameter `slot` — a field, an element, a
/// nested place, directly or through a call it makes ([`callee_param_writes`])?  `true` for a
/// slot it does not have, the answer that keeps a caller's assumption safe.
pub(crate) fn callee_writes_param(fn_nr: u32, data: &Data, slot: usize) -> bool {
    let mut cache = crate::fxhash::FxHashMap::default();
    callee_param_writes(fn_nr, data, &mut cache)
        .get(slot)
        .copied()
        .unwrap_or(true)
}
mod advice;
mod bounds;
mod call_args;
mod calls;
mod closures;
mod compare;
mod convert;
mod generic_writes;
mod generics;
mod hints;
mod imports;
mod members;
mod monomorph;
mod nullable_slots;
mod packages;
mod passes;
mod places;
mod sandbox;
mod store_check;
pub(super) mod store_else;
pub(super) mod store_text;
mod suggest;
pub(super) mod vectors;
mod visibility;
mod width32;
pub(crate) mod work_buffer;

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn is_op(name: &str) -> bool {
    name.len() >= 3 && name.starts_with("Op") && name.chars().nth(2).unwrap().is_uppercase()
}

/// Validate function, attribute, value, and field names
fn is_lower(name: &str) -> bool {
    for c in name.chars() {
        if c.is_uppercase() {
            return false;
        }
    }
    true
}

#[allow(dead_code)]
/// Used to validate constant names
fn is_upper(name: &str) -> bool {
    for c in name.chars() {
        if c.is_lowercase() {
            return false;
        }
    }
    true
}

/// Validate type, enum, enum values and struct names
fn is_camel(name: &str) -> bool {
    let c = name.chars().next().unwrap();
    if c.is_lowercase() {
        return false;
    }
    for c in name.chars() {
        if c == '_' {
            return false;
        }
    }
    true
}

/// Outcome of [`Parser::parse_statement`] — the REPL read-eval step's parse half.
#[derive(Debug)]
pub enum ParseResult {
    /// Fully parsed.  Any new top-level definition is registered in `data` and
    /// its body IR is built, ready for codegen + execute.  `entry_def_nr` is the
    /// newest definition's number, or `u32::MAX` when the statement added no
    /// runnable definition.
    Ready { entry_def_nr: u32 },
    /// Input ends mid-construct (open bracket, unterminated string, trailing
    /// operator).  The REPL should read another line and re-call with the
    /// concatenated input.
    NeedMore,
    /// Parse failed.  `data` has been rolled back to its pre-call state; the
    /// entries are the diagnostics this statement produced.
    Error(Vec<DiagEntry>),
}

impl Parser {
    #[must_use]
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub fn new() -> Self {
        // A new parser is a new compilation: the copy manifest's records are keyed by
        // definition and variable number, so the previous program's would name this one's.
        crate::copy_manifest::begin_compilation();
        let data = Data::new();
        Parser {
            todo_files: Vec::new(),
            track_sources: false,
            parsed_sources: Vec::new(),
            store_handoffs: Vec::new(),
            running_world: None,
            pass_started: None,
            speculative_type_refs: std::collections::HashSet::new(),
            qualified_stubs: std::collections::HashMap::new(),
            deferred_import_misses: Vec::new(),
            unresolved_names: 0,
            unresolved_types: 0,
            data,
            database: Stores::new(),
            lexer: Lexer::default(),
            in_loop: false,
            in_control_head: false,
            complexity: HashMap::new(),
            cc_deepest: HashMap::new(),
            cc_nest: 0,
            in_format_expr: false,
            prefix_operand: false,
            inplace_hint_declined: false,
            inplace_place_declined: false,
            rebuild_watch: u16::MAX,
            rebuild_watch_hit: false,
            in_tuple_lhs: false,
            preset_rhs: None,
            declaring_const: u16::MAX,
            sandbox: crate::sandbox::SandboxConfig::default(),
            def_sandbox: HashMap::new(),
            sandbox_unbounded_loops: HashMap::new(),
            sandbox_raw_writes: HashMap::new(),
            sandbox_fn_refs: HashMap::new(),
            declared_capabilities: HashSet::new(),
            member_access: HashMap::new(),
            sandbox_field_reads: HashMap::new(),
            sandbox_built: HashMap::new(),
            sandbox_field_updates: HashMap::new(),
            sandbox_field_appends: HashMap::new(),
            last_field_target: None,
            param_locks: HashMap::new(),
            sandbox_param_overrides: HashMap::new(),
            len_bound_locals: HashMap::new(),
            pending_param_locks: Vec::new(),
            pending_param_facts: Vec::new(),
            type_fact: crate::data::AliasFact::Plain,
            own_fact: None,
            operand_fact: crate::data::AliasFact::Plain,
            abstract_vars: std::collections::HashMap::new(),
            plain_declared: std::collections::HashSet::new(),
            mid_return_param_deps: std::collections::HashSet::new(),
            in_bind_rhs: false,
            produced: None,
            pending_lambda_facts: Vec::new(),
            lambda_result: crate::data::AliasFact::Plain,
            match_subject: crate::data::AliasFact::Plain,
            match_arm_facts: Vec::new(),
            pending_key_facts: Vec::new(),
            comprehension_fact: None,
            method_receiver: crate::data::AliasFact::Plain,
            method_facts: (Vec::new(), Vec::new()),
            method_checked: false,
            last_called: u32::MAX,
            postfix_member: String::new(),
            pending_param_positions: Vec::new(),
            amp_pending: false,
            first_bind_targets: Vec::new(),
            first_bind_at: HashMap::new(),
            update_literals: HashSet::new(),
            binding_starts: Vec::new(),
            stmt_if_pending: false,
            fit_candidate: None,
            fit_armed: None,
            fit_in_condition: false,
            pending_arm_mismatch: None,
            arm_convert_reported: false,
            null_literal_arms: 0,
            block_tail_null_literal: false,
            lit_shape_refused: None,
            arms_of_statement_construct: false,
            match_void_arm: false,
            amp_head: AmpHead::default(),
            amp_identity: None,
            eq_rhs_at: None,
            eq_amp_left: None,
            assign_target: u16::MAX,
            assign_replaces: false,
            build_snapshot_len: 0,
            rebind_lowered: u16::MAX,
            in_sandbox: false,
            parse_depth: 0,
            depth_overflowed: false,
            depth_trip_mark: None,
            file: 1,
            diagnostics: Diagnostics::new(),
            default: false,
            context: u32::MAX,
            first_pass: true,
            limit_refused: false,
            ambiguity_reported: std::collections::HashSet::new(),
            force_tret: crate::fxhash::FxHashSet::default(),
            work_buffer_promoted: crate::fxhash::FxHashSet::default(),
            work_buffer_pool: None,
            work_buffer_pool_kept: std::collections::HashMap::new(),
            store_text_args: false,
            par_worker_defs: std::collections::HashSet::new(),
            par_deferred: Vec::new(),
            par_replay_body: None,
            late_text_tails: Vec::new(),
            infer_ret_defs: std::collections::HashSet::new(),
            adopted_ret_defs: std::collections::HashSet::new(),
            ref_linked_tuple_locals: std::collections::HashSet::new(),
            pending_arg_pres: Vec::new(),
            pending_arg_posts: Vec::new(),
            pending_tuple_args: Vec::new(),
            tuple_place_wanted: false,
            loop_sources_replaced: std::collections::HashSet::new(),
            pattern_binds_pending: Vec::new(),
            pattern_bind_frames: Vec::new(),
            text_link_kinds: std::collections::HashMap::new(),
            amp_vector_locals: std::collections::HashSet::new(),
            amp_vector_link_partners: std::collections::HashMap::new(),
            literal_chain_lhs: std::collections::HashSet::new(),
            bound_method_stubs: Vec::new(),
            stub_origin: std::collections::HashMap::new(),
            reverse_iterator: false,
            unordered_iterator: false,
            iterable_context: false,
            last_range_from: None,
            literal_walk: None,
            last_range_till: None,
            coalesce_defaults: Vec::new(),
            block_path: Vec::new(),
            pending_loop_binders: Vec::new(),
            block_ord: 0,
            bound_in_block: std::collections::HashMap::new(),
            block_rebinds: Vec::new(),
            bind_after_end: None,
            match_cursor: None,
            match_cursor_farthest: None,
            subrule_edges: Vec::new(),
            vars: Function::new("", "none"),
            line: 0,
            lib_dirs: Vec::new(),
            own_lib: None,
            pending_native_libs: Vec::new(),
            native_lib_regs: Vec::new(),
            pending_native_compile: Vec::new(),
            compiled_bridges: std::collections::HashSet::new(),
            library_bridges: true,
            pending_placed_libs: Vec::new(),
            pending_pkg_deps: Vec::new(),
            auto_use_scan_cache: std::collections::HashMap::new(),
            pkg_dep_cache: std::collections::HashMap::new(),
            use_paths: std::collections::HashMap::new(),
            #[cfg(feature = "registry")]
            undeclared_reported: std::collections::HashSet::new(),
            lib_outranked_reported: std::collections::HashSet::new(),
            #[cfg(feature = "registry")]
            auto_installed: std::collections::HashMap::new(),
            #[cfg(feature = "registry")]
            pin_behind_reported: std::collections::HashSet::new(),
            #[cfg(feature = "registry")]
            newest_releases: None,
            module_clash_reported: std::collections::HashSet::new(),
            #[cfg(feature = "registry")]
            root_dep_pins: None,
            program_scope: None,
            program_entry: None,
            cache_unmet: std::collections::HashMap::new(),
            #[cfg(feature = "registry")]
            registry_refused: std::collections::HashSet::new(),
            auto_use_trigger_map: None,
            auto_use_ambiguous: std::collections::HashMap::new(),
            auto_use_type_map: std::collections::HashMap::new(),
            auto_use_builder_map: std::collections::HashMap::new(),
            auto_use_catalog_builders: None,
            auto_use_catalog_ambiguous: None,
            reported_manifest_errors: Vec::new(),
            auto_use_catalog_map: None,
            pending_imports: Vec::new(),
            use_public: false,
            pub_taken: false,
            applied_imports: Vec::new(),
            width_decided: std::collections::BTreeSet::new(),
            deferred_unknown: Vec::new(),
            record_resolutions: false,
            resolutions: Vec::new(),
            expr_not_null: false,
            expr_not_null_name: String::new(),
            fn_ref_read_attr: None,
            lambda_counters: HashMap::new(),
            expected: Type::Unknown(0),
            reported_dynamic_refusal: false,
            unhinted_set: None,
            unhinted_set_refused: false,
            reported_binding_clash: false,
            declared_fn_names: HashMap::new(),
            positionless_loops: HashSet::new(),
            fields_of: u32::MAX,
            capture_context: Vec::new(),
            capture_owner: std::collections::HashMap::new(),
            capture_const: std::collections::HashSet::new(),
            capture_const_saved: Vec::new(),
            capture_fn_lambda: std::collections::HashMap::new(),
            last_closure_lambda: u32::MAX,
            capture_fn_lambda_saved: Vec::new(),
            rebound_captures: std::collections::HashMap::new(),
            captured_names: Vec::new(),
            branch_sunk_vectors: std::collections::HashSet::new(),
            closure_shared_vectors: std::collections::HashMap::new(),
            capture_alias: std::collections::HashMap::new(),
            fn_lambdas: std::collections::HashMap::new(),
            closure_param: u16::MAX,
            cur_type_vars: Vec::new(),
            instance_bindings: Vec::new(),
            set_call_refs: Vec::new(),
            cyclic_instances: std::collections::HashSet::new(),
            forward_template_args: false,
            pending_forward_return: None,
            forward_generic_returns: Vec::new(),
            discovering: 0,
            discovery_skipped_lambda: false,
            refused_header_vars: Vec::new(),
            context_type_template: u32::MAX,
            type_var_holders: std::collections::HashMap::new(),
            type_var_bounds: std::collections::HashMap::new(),
            last_closure_work_var: u16::MAX,
            last_closure_alloc: None,
            last_closure_captured_vars: vec![],
            closure_capture_builds: std::collections::HashMap::new(),
            capture_records: std::collections::HashMap::new(),
            yield_copied_captures: std::collections::HashMap::new(),
            referenced_locals: crate::fxhash::FxHashSet::default(),
            referenced_assigned: crate::fxhash::FxHashSet::default(),
            init_field_tracking: false,
            init_field_deps: Vec::new(),
            default_paths: Vec::new(),
            default_path_reads: Vec::new(),
            init_reads_record: false,
            dollar_record: None,
            upgrade_fills: false,
            in_par_body: false,
            is_capture_aliases: Vec::new(),
            capture_took_body: None,
            assign_target_discounted: u16::MAX,
            assign_target_taken: false,
            is_capture_bindings: Vec::new(),
            last_cast_alias: u32::MAX,
            dn4_checked_narrow: None,
            in_explicit_cast: false,
            store_ctx: Vec::new(),
            admit_unwrap: 0,
            pending_default_rhs: None,
            pending_default_src: None,
            pending_link_slot: Vec::new(),
            ncc_default_end: None,
            last_place_discharge: false,
            pass2_bodies: std::collections::HashSet::new(),
            stale_monomorphs: Vec::new(),
            refused_templates: std::collections::HashSet::new(),
            conv_owned_result: None,
            trace_types: false,
            trace_types_lines: Vec::new(),
            field_read_counts: std::collections::HashMap::new(),
            defended_field_reads: std::collections::HashSet::new(),
            narrowed_non_null: Vec::new(),
            capture_variant: std::collections::HashMap::new(),
            const_views: std::collections::HashMap::new(),
            foreign_bound: std::collections::HashSet::new(),
            narrowed_non_null_exprs: Vec::new(),
            divisor_nonzero: Vec::new(),
            variant_proven: Vec::new(),
            math_sign_proven: Vec::new(),
            text_payload_views: std::collections::HashMap::new(),
            last_index_fit: false,
            index_bounded: Vec::new(),
            last_field_read_site: None,
            #[cfg(feature = "registry")]
            advisory_checked: std::collections::HashSet::new(),
        }
    }

    // ********************
    // * Helper functions *
    // ********************

    /// `len` of an `index<T[k]>` receiver, `len(ix)` and `ix.len()` alike: `OpLengthIndex`, the
    /// length the B+tree keeps in its header (@FR-Col-Len).  An `index<…>?` counts 0 when
    /// absent, through `@FR-N-Store`'s store as every keyed `len`.
    /// `None` when the type or the op is not registered yet.
    pub(crate) fn index_len(
        &mut self,
        code: &mut Value,
        recv: &Type,
        mut args: Vec<Value>,
        at: Option<&Position>,
    ) -> Option<Type> {
        let (base, absent) = recv.peel_optional();
        let base = base.clone();
        let known = self.get_type(&base);
        let op_d_nr = self.data.def_nr("OpLengthIndex");
        if known == u16::MAX || op_d_nr == u32::MAX {
            return None;
        }
        if absent && let Some(first) = args.first_mut() {
            self.convert_store(first, recv, &base, "parameter 1 of `len`", at);
        }
        *code = Value::Call(op_d_nr, args);
        Some(crate::data::I64.clone())
    }

    /// The `Block` name [`build_default`](Parser::build_default) stamps on an `x?`
    /// default it could not decide — the operand's type is a type VARIABLE, so the
    /// answer belongs to the monomorph.  [`rewrite_generic_type_defaults`] finds it by
    /// this name once `T` is concrete (loft#1016).
    pub(crate) const TV_DEFAULT_BLOCK: &'static str = "tvdefault";
    /// loft#1020 — an `x == null` / `x != null` whose operand is still a TYPE VARIABLE.
    ///
    /// Which null test `τ` wants is a function of `τ`, and inside a template `τ` is not
    /// known yet, so the site is stamped and [`rewrite_generic_type_defaults`] answers it
    /// per monomorph.  The block holds the OPERAND and its `result` is the operand's
    /// type, which substitution rewrites to the concrete one.
    pub(crate) const TV_NULLTEST_EQ: &'static str = "tvnulleq";
    /// The `!=` spelling of [`TV_NULLTEST_EQ`] — same operand, negated answer.
    pub(crate) const TV_NULLTEST_NE: &'static str = "tvnullne";
    /// A `??` null CHECK whose subject is still a TYPE VARIABLE.
    ///
    /// Which test says *"this is not null"* is a function of `τ` — a DbRef's `rec`, a
    /// boolean's `255`, a character's `char(0)`, a tuple's first field — and inside a
    /// template `τ` is not known yet.  Undeferred, the site took the placeholder's own
    /// shape (an attribute-less struct, so a reference) and substitution rewrote the
    /// TYPE while leaving the TEST behind: `x ?? fb` looped forever at
    /// `T = boolean`, corrupted a record at `T = character`, and would not compile on
    /// `--native` at either.  The block holds the SUBJECT and its `result` is the
    /// subject's type, which substitution rewrites to the concrete one; the sibling of
    /// [`TV_NULLTEST_EQ`], for the coalesce rather than the comparison.
    pub(crate) const TV_NULLCHECK: &'static str = "tvnullcheck";
    /// loft#1028 — a `null` VALUE whose type is still a TYPE VARIABLE.
    ///
    /// Which sentinel `τ`'s null is written as is a function of `τ`, and inside a
    /// template `τ` is not known yet, so [`convert`](Parser::convert) stamps the site and
    /// [`rewrite_generic_type_defaults`](Parser::rewrite_generic_type_defaults) re-runs
    /// the conversion once `T` is concrete.  The block's `result` is the target type,
    /// which substitution rewrites to the concrete one.
    pub(crate) const TV_NULL_BLOCK: &'static str = "tvnull";
    /// A `vector<T>` element write at a TUPLE `T` (`[element, value]`, `result` the tuple):
    /// lowered in the monomorph's frame through `emit_tuple_set_ops`, the concrete append's
    /// own member-by-member write.
    pub(crate) const TV_TUPLE_ELEM: &'static str = "tvtupleelem";
    /// An element READ of a `vector<T>` whose `T` is bound to a TUPLE: the template read a
    /// reference to the stored element, and every consumer of the value — a callback, a
    /// return, a local — expects the stack tuple the concrete twin unboxes it into.  The
    /// substitution cannot mint the unboxing temp (it has no frame), so it stamps the read and
    /// the monomorph lowers it through `unbox_tuple_from_dbref`.
    pub(crate) const TV_TUPLE_READ: &'static str = "tvtupleread";
    /// A struct field of tuple type read while a `&` link parses: `[Drop(OpGetField(record,
    /// pos, kt)), Tuple(member reads)]`, the address a link to the field names (loft#1883).
    pub(crate) const TUPLE_FIELD_PLACE: &'static str = "tuple_field_place";
    /// The value [`Parser::null_value`] answers for a type still a TYPE VARIABLE — a `match`
    /// join's fallback, a branch with nothing to yield — asked again of the concrete type by
    /// each monomorph.  Apart from [`Self::TV_NULL_BLOCK`] because every type has this one:
    /// that block stands for a `null` the author wrote, which some types refuse.
    pub(crate) const TV_NULL_VALUE: &'static str = "tvnullvalue";
    /// loft#1537 — an `insert(v, i, e)` whose element type is still a TYPE VARIABLE.  The
    /// width, the row and the setter are all functions of the element type, so the site is
    /// stamped with its three arguments and a `result` of the vector's type, and
    /// [`rewrite_generic_type_defaults`] lowers it through `parse_insert` once `T` is real.
    pub(crate) const TV_INSERT: &'static str = "tvinsert";
    /// A `reverse(v)` whose element type is still a TYPE VARIABLE: the width it walks is the
    /// element's, so it is lowered per monomorph like [`TV_INSERT`](Parser::TV_INSERT).
    pub(crate) const TV_REVERSE: &'static str = "tvreverse";
    /// A `reserve(v, n)` whose element type is still a TYPE VARIABLE: the claim is sized in
    /// the element's width, so it is lowered per monomorph like [`TV_INSERT`](Parser::TV_INSERT).
    pub(crate) const TV_RESERVE: &'static str = "tvreserve";
    /// A `sort(v)` whose element type is still a TYPE VARIABLE bounded by `Ordered` (@PLN165
    /// E4): the monomorph lowers it to `OpSortVector` where the runtime compares the element
    /// itself, and to a call of the stdlib declaration's instance otherwise — what the
    /// instance's hand-written twin reaches either way.
    pub(crate) const TV_SORT: &'static str = "tvsort";
    /// A `reduce(v, init, f)` whose accumulator is still a TYPE VARIABLE (@PLN165 E7): the
    /// monomorph folds with the special form where the accumulator is a scalar or `text`, and
    /// calls the declaration's instance otherwise.  The block's result carries the three
    /// argument types as a tuple, so substitution makes them concrete.
    pub(crate) const TV_REDUCE: &'static str = "tvreduce";
    /// A capture READ inside a template lambda: `[Var(__closure), Text(name), read]`.  The
    /// template's closure record lays a capture typed by a type variable out at no width, so
    /// the read is re-lowered by NAME against the instance's own record
    /// ([`Parser::closure_capture_read`]).
    pub(crate) const TV_CAPTURE: &'static str = "tvcapture";
    /// @PLN165 B3b — a call to an overload set whose argument is typed by a TYPE VARIABLE:
    /// `[Int(source), Int(home source), Text(name), TV_SELECT_ARG…, TV_SELECT_NAMED…]`, its
    /// `result` the type the generic body was typed with.  Each instance lowers it through
    /// [`Parser::call`] with its own argument types ([`Parser::defer_set_call`]).
    pub(crate) const TV_SELECT: &'static str = "tvselect";
    /// One positional argument of a [`Parser::TV_SELECT`] site: the value, its `result` the
    /// argument's static type, which substitution rewrites to the instance's.
    pub(crate) const TV_SELECT_ARG: &'static str = "tvselectarg";
    /// One named argument of a [`Parser::TV_SELECT`] site: `[Text(parameter), value]`.
    pub(crate) const TV_SELECT_NAMED: &'static str = "tvselectnamed";
    /// @PLN165 D5 — a field READ of an OPEN instance (`b.v` with `b: Box<T>`):
    /// `[Int(open instance), Int(field), receiver]`, its `result` the field's type.  The
    /// field has no position until the instance is known, so each instance lowers it through
    /// `get_field` on the instance its bindings name.
    pub(crate) const TV_FIELD: &'static str = "tvfield";
    /// A field WRITE of an open instance: `[Int(open instance), Int(field), Text(op),
    /// receiver, TV_SELECT_ARG(value)]`, lowered through `towards_set` per instance.
    pub(crate) const TV_FIELD_SET: &'static str = "tvfieldset";
    /// A LITERAL of an open instance (`Box { v: x }` in a `-> Box<T>` template):
    /// `[Int(open instance), Text(file), Int(line), TV_OBJECT_FIELD…]`, built per instance
    /// into a fresh record; the position is the literal's, which its field checks report.
    pub(crate) const TV_OBJECT: &'static str = "tvobject";
    /// One field of a [`Parser::TV_OBJECT`]: `[Text(field), value]`, its `result` the
    /// value's static type.
    pub(crate) const TV_OBJECT_FIELD: &'static str = "tvobjectfield";

    /// The `tp` a bound `==` is marked with until its schema row is known (`(G-Sat-Eq)`).
    const CONTENT_EQ_PENDING: i32 = i32::MIN;
    /// A `sizeof` whose operand is a bare type variable: `OpSizeofRef(SIZEOF_PENDING, holder)`,
    /// sized at each monomorph from the type the holder is bound to (`resolve_pending_sizeof`).
    const SIZEOF_PENDING: i32 = i32::MIN + 1;

    /// The marker a format hole over a bound type variable carries until its instance lowers
    /// it (`append_data`, `@FR-G-Mono`).
    pub(crate) const FORMAT_PENDING: i32 = i32::MIN + 1;

    fn cl(&mut self, op: &str, list: &[Value]) -> Value {
        let d_nr = self.data.def_nr(op);
        if d_nr == u32::MAX {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Internal error: missing built-in operation (report this as a bug)"
            );
            Value::Null
        } else {
            Value::Call(d_nr, list.to_vec())
        }
    }

    // ********************
    // * Parser functions *
    // ********************

    /// The directives at the top of a file.  `entry` is the file a parse starts from, where
    /// all three apply; a file a `use` switches into (`entry == false`) may only say
    /// `#package`, because `#cwd` and `#link` set how the whole PROGRAM runs and links, and
    /// a library must not change that for the program that uses it.
    ///
    /// `#cwd` (#255 / @PLN9) opts the program out of the program-relative default, so a
    /// *relative* file path resolves against the process cwd (CLI-tool semantics) rather
    /// than the program's own directory.  Whole-program; must precede declarations.  At file
    /// top the lexer's first token can only be a directive (`#rust`/`#native`/etc. are
    /// declaration-scoped, consumed later by `parse_rust`).
    ///
    /// `#link static` / `#link dynamic` names how a native build links the native libraries
    /// the program uses.  The DRIVER acts on it, reading the header before any parse
    /// (`native_utils::apply_link_directive`); here it is only checked, so a misspelt mode
    /// is refused rather than silently linking the default way.
    ///
    /// `#package` (`@FR-F-Package`): this file shows its internals to the other files of its
    /// own directory.
    ///
    /// @PLN204 — `#width 32` puts this file under the 32-bit regime (`Data::width32`) and
    /// `#width 64` keeps it out of a whole-program `LOFT_WIDTH=32`; a file's own setting, so a
    /// used file may say it too.  The stdlib never is.  A suspended file resumed after its
    /// `use` re-enters here mid-file, so the default is only taken for a source not decided yet.
    pub(super) fn parse_file_directives(&mut self, entry: bool) {
        let src = self.data.source;
        self.width_header();
        while self.lexer.has_token("#") {
            match self.lexer.has_identifier().as_deref() {
                Some("width") => self.width_directive(src),
                Some("package") => {
                    let file = self.lexer.pos().file;
                    self.data.note_package_file(&file);
                    let _ = self.lexer.has_token(";");
                }
                Some(whole @ ("cwd" | "link")) if !entry => {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'#{whole}' sets how the whole program runs, so it is written in the \
                         program's own file, not in a file it uses; a used file may say \
                         '#package' or '#width'"
                    );
                    break;
                }
                Some("cwd") => {
                    self.database.program_relative = false;
                    let _ = self.lexer.has_token(";");
                }
                Some("link") => {
                    let mode = self.lexer.has_identifier().unwrap_or_default();
                    if mode != "static" && mode != "dynamic" {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown link mode '#link {mode}' (expected '#link static' or '#link dynamic')"
                        );
                    }
                    let _ = self.lexer.has_token(";");
                }
                other => {
                    let name = other.unwrap_or("").to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown file directive '#{name}' (expected '#cwd', '#link', '#package' or '#width')"
                    );
                    break;
                }
            }
        }
    }

    /// Parse data from the current lexer.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    fn parse_file(&mut self) {
        let start_def = self.data.definitions();
        self.parse_file_directives(true);
        // Tier-0 lazy auto-`use`: the file the lexer is on right now, captured
        // before the use-loop may switch away.  Scanned for `lib::` references
        // after the use-region (see the load loop below).
        let auto_use_scan_file = self.lexer.pos().file;
        self.claim_declared_type_names(&auto_use_scan_file);
        // A file that writes any `use` — or the stdlib, parsed with
        // `self.default` — is in *explicit* mode: the author manages their
        // libraries by hand, so a `lib::` to an un-`use`d library is a forgotten
        // `use`, not a request to auto-load.  Skip the pre-scan for those files.
        let mut had_use = self.default;
        self.pub_taken = false;
        // Use-region fixpoint.  Pre-scan explicit `use`s, then load manifest
        // `[dependencies]`.  Loading a manifest dep `switch_to_dep`s the lexer
        // ONTO it; a MULTI-FILE dependency lands on an entry file that still has
        // its own leading `use` statements — which the pending loop, unlike this
        // pre-scan, does NOT handle.  Loop until the cursor rests on a use-free
        // file with nothing pending, so every file the main definition-loop
        // parses has had its uses processed (otherwise a dependency's legitimate
        // top `use` is misread as "use after definitions" — and never imported).
        loop {
            while self.use_keyword() {
                if let Some(id) = self.lexer.has_identifier() {
                    had_use = true;
                    // loft#949 — `use self::<module>` binds THIS package's own module,
                    // whatever else in the graph ships a file of that name.  Handled
                    // before the shared path because it resolves and registers under a
                    // package-qualified key rather than the bare short name.
                    if id == "self" {
                        self.parse_use_self();
                        continue;
                    }
                    // @PLN22 Phase 3 — optional library alias: `use lib as m;` → `m::fn`.
                    let lib_alias = if self.lexer.has_token("as") {
                        self.lexer.has_identifier()
                    } else {
                        None
                    };
                    // Parse optional import spec: `::*` wildcard, a single
                    // `::name [as bind]`, or the grouped `::(a [as x], b, …)`.
                    // @PLN22 Phase 4 — multiple names MUST be grouped in parentheses;
                    // the flat top-level comma list (`use lib::a, b`) is dropped (it
                    // read poorly — `b` didn't visually bind to `lib::`).
                    let spec = self.parse_import_spec(&id);
                    // Recorded where the `use` BINDS, with the source it bound (@C98).
                    let bare_use = spec.is_none() && lib_alias.is_none();
                    if spec.is_none() {
                        self.refuse_bare_pub_use(&id);
                    }
                    // loft#976 — a package's OWN module wins its own `use`.
                    //
                    // A module's file name is one global name across the whole
                    // dependency graph, so two packages that each ship a `<id>.loft`
                    // and each say a bare `use <id>` used to be resolved by whichever
                    // loaded FIRST — a decision that belongs to the CONSUMER's `use`
                    // order and is visible to neither author.  The loser's own module
                    // never loaded, so its public surface was amputated in a build it
                    // had nothing to do with, and a qualified `pkg::name` could not
                    // rescue it: there was no second module to choose between.
                    //
                    // `use self::<id>` already bound the local file under a
                    // package-qualified key, and the advice already recommended it.
                    // This makes it what a bare `use` MEANS inside a package, so
                    // `self::` is the explicit form for the rare case that wants a
                    // stranger's module rather than the defensive form every library
                    // author has to remember for every file they will ever add.
                    // …but a package's OWN NAME is not one of its module names.  `use
                    // <pkg>` inside `<pkg>` asks for the package — its public surface —
                    // and that is how every library in the ecosystem writes its own test
                    // suite (`tests/<pkg>.loft` saying `use <pkg>;`).  Without this the
                    // rule above binds whatever file happens to be called `<pkg>.loft`,
                    // which for that suite is the TEST FILE, so the entry's `pub` surface
                    // is not loaded and every symbol reads unknown.  Nine published
                    // libraries went red on it.
                    //
                    // The distinction is what keeps a package able to refer to ITSELF: a
                    // name that means the package in one file and a sibling module in
                    // another is a name that means nothing.  `use self::<pkg>` remains
                    // the explicit spelling for the file, in the rare case that is what
                    // was wanted.
                    if !self.package_declares_dep(&id)
                        && let Some(pkg) = self.own_package_name()
                        && id != pkg
                        && let Some(f) = self.own_module_path(&id)
                    {
                        self.bind_own_module(&id, &pkg, &f, lib_alias.as_ref(), spec, "");
                        continue;
                    }
                    // loft#912 — the name is taken, but by WHICH file?  A package with
                    // its own `<id>.loft` took it above; reaching here means this file
                    // has none, so the import binds whichever package's did load.
                    if self.data.use_exists(&id) {
                        self.module_name_clash(&id);
                        // loft#1849 — this file's package is bound by its own range for
                        // `id` too, whichever `use` loaded the copy.
                        let cur_script = self.lexer.pos().file.to_string();
                        if let Some(loaded) = self.use_paths.get(&id).cloned()
                            && let Some(pkg_root) = crate::resolution_scope::project_root(&loaded)
                        {
                            self.loaded_copy_meets_declaring_range(&id, &cur_script, &pkg_root);
                        }
                    }
                    if self.data.use_exists(&id) {
                        let lib_source = self.data.get_source(&id);
                        if bare_use {
                            self.data.record_bare_use(self.data.source, &id, lib_source);
                        }
                        // @PLN22 Phase 3 — register the library alias for `m::` access.
                        if let Some(alias) = &lib_alias {
                            self.data.use_alias(alias, lib_source);
                        }
                        // A bare `use foo;` binds only the `foo::` qualifier, and so does
                        // `use foo as m;` (`m::`): names come in unqualified only through
                        // an explicit `::*` or `::(…)` spec, so a library growing a name
                        // can never collide with the program's own (@C98).
                        // …but a bare one still brings the library's operator definitions
                        // (`Data::import_operators`), so it queues an empty name list.
                        let import_spec = spec.or(Some(ImportSpec::Names(Vec::new())));
                        if let Some(import_spec) = import_spec {
                            self.pending_imports.push(PendingImport {
                                for_source: self.data.source,
                                lib_source,
                                spec: import_spec,
                                public: self.use_public,
                            });
                        }
                        if !self.lexer.has_token(";") {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Missing ';' after 'use {id}' — use statements must end with a semicolon"
                            );
                        }
                        continue;
                    }
                    // A manifest check inside `lib_path` can REFUSE a package it found —
                    // the wrong loft version, an unreadable version constraint, a contract
                    // this loft is too old for.  Each of those raises its own Fatal and then
                    // resolves to nothing, which used to fall through to the "not found"
                    // arm below: the author of a package that needs a newer loft was told
                    // both that it needs a newer loft AND that it does not exist, and the
                    // second sentence is the one that reads like the answer.  So the level
                    // is snapshotted across the search and the specific refusal is left to
                    // speak alone.
                    let level_before = self.lexer.diagnostics().level();
                    let f = self.lib_path(&id);
                    let refused = self.lexer.diagnostics().level() == Level::Fatal
                        && level_before != Level::Fatal;
                    let f_exists = file_exists(&f) || {
                        #[cfg(feature = "wasm")]
                        {
                            crate::wasm::virt_fs_get(&f).is_some()
                        }
                        #[cfg(not(feature = "wasm"))]
                        {
                            false
                        }
                    };
                    if f_exists {
                        let cur = &self.lexer.pos().file;
                        self.todo_files.push((cur.to_string(), self.data.source));
                        self.data.use_add(&id);
                        self.record_use_path(&id, &f);
                        // @PLN22 Phase 3 — register the library alias now that the lib's
                        // source exists (use_add set self.data.source to it).  The
                        // import itself is recorded on the second encounter (via
                        // todo_files re-parse with use_exists=true).
                        if let Some(alias) = &lib_alias {
                            self.data.use_alias(alias, self.data.source);
                        }
                        drop(spec);
                        self.switch_to_dep(&f);
                    } else {
                        // A manifest the lexer could not read is the likelier cause: say so first.
                        self.report_manifest_errors();
                        if !refused {
                            if let Some(unmet) = self.cache_unmet.get(&id).cloned() {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "Library '{id}' not found — {unmet}"
                                );
                            } else {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "Library '{id}' not found — searched lib/, lib_dirs, and sibling packages"
                                );
                            }
                        }
                        self.lexer.has_token(";");
                    }
                }
            }
            // PKG.3: load transitive dependencies discovered during manifest reading.
            // Dependencies are queued by lib_path_manifest when it reads [dependencies].
            //
            // loft#714 — pull a manifest dep in ONLY while the lexer is back on
            // the file that named the package, and one per fixpoint iteration.
            //
            // `switch_to_dep` REPLACES the lexer's source and the abandoned file
            // resumes later off `todo_files`.  That is fine for a `use`, which
            // always switches away from the very file it appears in.  Draining
            // this queue from wherever the descent happened to reach broke it: a
            // dep was pulled in while the lexer sat inside an unrelated library
            // whose definitions had not been parsed yet, and that library had
            // already been `use_add`ed — so every later `use` of it was a no-op
            // against an empty library.  With two manifest deps whose graphs
            // meet, it surfaced deep inside a third, published, CI-gated library
            // as `Unknown variable` on a tuple destructure or `Expect token ;` on
            // a tuple field, with nothing naming resolution.
            let here = self.lexer.pos().file;
            if let Some(pos) = self
                .pending_pkg_deps
                .iter()
                .position(|(dep_id, _, from)| **from == *here && !self.data.use_exists(dep_id))
            {
                let (dep_id, parent_dir, _) = self.pending_pkg_deps.remove(pos);
                // First try the sibling package directory (same parent as the
                // depending package), then fall back to the normal lib_path search.
                let f = if let Some(entry) = self.lib_path_manifest(&parent_dir, &dep_id) {
                    entry
                } else {
                    self.lib_path(&dep_id)
                };
                if file_exists(&f) {
                    let cur = &self.lexer.pos().file;
                    self.todo_files.push((cur.to_string(), self.data.source));
                    self.data.use_add(&dep_id);
                    self.record_use_path(&dep_id, &f);
                    self.switch_to_dep(&f);
                }
            } else {
                // Nothing left for THIS file — drop its entries (already loaded,
                // or unresolvable) so the fixpoint can exit.  Entries queued from
                // other files stay: their own file drains them when it resumes.
                self.pending_pkg_deps.retain(|(_, _, from)| **from != *here);
            }
            // Fixpoint exit: the cursor rests on a use-free file and this file has
            // no manifest dependency still queued.  `peek_token` (not `has_token`)
            // so the check never consumes a leading `use` the next iteration needs.
            let mine_pending = self
                .pending_pkg_deps
                .iter()
                .any(|(_, _, from)| **from == *self.lexer.pos().file);
            if !self.lexer.peek_token("use") && !mine_pending {
                break;
            }
        }
        // Tier-0 lazy auto-`use`: scan this file for `lib::` references and load
        // any that name an unloaded, available library — here in the use-region,
        // before the defs-loop, exactly like an explicit `use` (so the two-pass
        // model never sees a redefinition).  Only when the lexer is still on the
        // scanned file (the use-loop has not switched away to an explicitly-used
        // library — that file re-parses via `todo_files` and scans itself then).
        // Resolve every path first (before a `switch` changes the lexer's cwd),
        // then load with the same `todo_files` + `use_add` + `switch` shape as
        // the `pending_pkg_deps` loop.  A name that is not a real library is
        // skipped and falls through to the normal "unknown" error.
        // `!had_use`: explicit-mode files (any `use`, and the stdlib) are skipped
        // entirely — the author manages their libraries by hand there.  Read +
        // scan each remaining file at most once (cache keyed by path).
        if !had_use && *self.lexer.pos().file == *auto_use_scan_file {
            let (refs, calls, type_refs, bare_calls) =
                if let Some(c) = self.auto_use_scan_cache.get(auto_use_scan_file.as_str()) {
                    c.clone()
                } else {
                    let src = self
                        .lexer
                        .source_text(auto_use_scan_file.as_str())
                        .map_or_else(
                            || Self::read_source(auto_use_scan_file.as_str()),
                            str::to_string,
                        );
                    // A type the file DECLARES is its own, never a trigger: a script with
                    // its own `struct Path` must not load a package that also has one.
                    let declared = crate::libscan::scan_type_declarations(&src);
                    // Likewise a function the file DECLARES: its own `fn set` wins over a
                    // library's builder of that name, which `pkg::set(…)` still reaches.  Nor
                    // is a name the stdlib (or a loaded file) already defines: the scan hands
                    // back slices, and only a survivor is allocated — one key buffer serves
                    // every lookup (the allocation pins count each `format!`).
                    let (bare, fns) = crate::libscan::scan_bare_calls_and_fn_declarations(&src);
                    let mut key = String::with_capacity(64);
                    let bare_calls: Vec<String> = bare
                        .into_iter()
                        .filter(|f| !fns.contains(f))
                        .filter(|f| {
                            key.clear();
                            key.push_str("n_");
                            key.push_str(f);
                            self.data.def_nr(&key) == u32::MAX
                        })
                        .map(str::to_string)
                        .collect();
                    let pair = (
                        crate::libscan::scan_qualified_lib_refs(&src),
                        crate::libscan::scan_method_calls(&src),
                        crate::libscan::scan_type_refs(&src)
                            .into_iter()
                            .filter(|t| !declared.contains(t))
                            .collect(),
                        bare_calls,
                    );
                    self.auto_use_scan_cache
                        .insert(auto_use_scan_file.to_string(), pair.clone());
                    pair
                };
            // Tier-0: `lib::x` — the library is named directly.
            let mut to_load: Vec<String> = Vec::new();
            for name in refs {
                if self.data.use_exists(&name) || self.data.def_nr(&name) != u32::MAX {
                    continue;
                }
                if !to_load.contains(&name) {
                    to_load.push(name);
                }
            }
            // Tier-1: `obj.method(…)` — map the method to its providing package
            // via the trigger surface of the current package (+ trigger-enabled
            // deps), derived once and cached.
            if !calls.is_empty() {
                let map = self.trigger_map(auto_use_scan_file.as_str());
                // Catalog fallback is built lazily — only read index.json once a
                // method misses the local (current package + deps) trigger map.
                let mut catalog: Option<std::collections::HashMap<String, String>> = None;
                for m in calls {
                    let pkg = if let Some(p) = map.get(&m) {
                        Some(p.clone())
                    } else {
                        if catalog.is_none() {
                            catalog = Some(self.catalog_trigger_map());
                        }
                        catalog.as_ref().and_then(|c| c.get(&m).cloned())
                    };
                    if let Some(pkg) = pkg
                        && !self.data.use_exists(&pkg)
                        && !to_load.contains(&pkg)
                    {
                        to_load.push(pkg);
                    }
                }
            }
            // Tier-1, the type half: `c: Command = …` names a type a trigger-enabled
            // dependency declares.  Local map only (no catalog): a type the file does not
            // define and no declared package provides stays the ordinary "Undefined type".
            // A loaded package binds only its `pkg::` qualifier (@C98), so each type that
            // fired is also imported by name, as `use pkg::(Type);` would — a name the file
            // reached for, never the whole library.
            // The import is queued once the package IS loaded: this file resumes from its
            // start after the dependency has been parsed (`todo_files`), the scan runs
            // again, and the name then resolves — the same two visits an explicit
            // `use pkg::(Type);` makes.
            if !type_refs.is_empty() {
                let _ = self.trigger_map(auto_use_scan_file.as_str());
                for t in type_refs {
                    let Some(pkg) = self.auto_use_type_map.get(&t).cloned() else {
                        continue;
                    };
                    if self.data.use_exists(&pkg) {
                        let lib_source = self.data.get_source(&pkg);
                        let cur = self.data.source;
                        if !self.data.import_name(lib_source, cur, &t, &t, false)
                            && !self.pending_imports.iter().any(|pi| {
                                pi.for_source == cur
                                    && matches!(&pi.spec, ImportSpec::Names(n) if n.iter().any(|(a, _)| *a == t))
                            })
                        {
                            self.pending_imports.push(PendingImport {
                                for_source: cur,
                                lib_source,
                                spec: ImportSpec::Names(vec![(t.clone(), t)]),
                                public: false,
                            });
                        }
                    } else if !to_load.contains(&pkg) {
                        to_load.push(pkg);
                    }
                }
            }
            // Tier-1, the builder half: a bare `set(words)` names a function a trigger-enabled
            // package DECLARES as a builder (`[triggers] builders`).  A name the stdlib or an
            // already-loaded file defines is never a trigger (the stdlib's `set` would win and a
            // library cannot take it); the declared packages' map first, then the catalog.  As
            // for a type, a loaded package binds only its qualifier, so the one name that fired
            // is imported, as `use pkg::(set);` would — never the library's whole surface.
            if !bare_calls.is_empty() {
                let _ = self.trigger_map(auto_use_scan_file.as_str());
                let mut catalog: Option<std::collections::HashMap<String, String>> = None;
                for f in bare_calls {
                    if self.data.def_nr(&format!("n_{f}")) != u32::MAX {
                        continue;
                    }
                    let pkg = if let Some(p) = self.auto_use_builder_map.get(&f) {
                        Some(p.clone())
                    } else {
                        if catalog.is_none() {
                            catalog = Some(self.catalog_builder_map());
                        }
                        catalog.as_ref().and_then(|c| c.get(&f).cloned())
                    };
                    let Some(pkg) = pkg else { continue };
                    if self.data.use_exists(&pkg) {
                        let lib_source = self.data.get_source(&pkg);
                        let cur = self.data.source;
                        if !self.data.import_name(lib_source, cur, &f, &f, false)
                            && !self.pending_imports.iter().any(|pi| {
                                pi.for_source == cur
                                    && matches!(&pi.spec, ImportSpec::Names(n) if n.iter().any(|(a, _)| *a == f))
                            })
                        {
                            self.pending_imports.push(PendingImport {
                                for_source: cur,
                                lib_source,
                                spec: ImportSpec::Names(vec![(f.clone(), f)]),
                                public: false,
                            });
                        }
                    } else if !to_load.contains(&pkg) {
                        to_load.push(pkg);
                    }
                }
            }
            // Resolve every path first (before a `switch` changes the cwd), then
            // load with the proven todo_files + use_add + switch shape.
            let mut resolved: Vec<(String, String)> = Vec::new();
            for n in to_load {
                if self.data.use_exists(&n) {
                    continue;
                }
                let f = self.lib_path(&n);
                // A library must never be loaded from the file being parsed RIGHT
                // NOW.  A package that qualifies a call with its own name
                // (`graphics::color_r(..)` inside `src/graphics.loft`) resolves the
                // library to that same file, and parsing it a second time into one
                // parser re-registers every definition — which surfaces as
                // "cannot redefine method `x` on `text`" naming the file's own line,
                // an error about the source disagreeing with itself (loft#656).
                // Free functions merely duplicated silently; a METHOD made it fatal.
                // The definitions are already present, so skipping is not a
                // work-around: the load has nothing left to do.
                if self.is_current_source(&f) {
                    self.data.use_add(&n);
                    self.record_use_path(&n, &f);
                    continue;
                }
                if file_exists(&f) {
                    resolved.push((n, f));
                }
            }
            for (name, f) in resolved {
                if self.data.use_exists(&name) {
                    continue;
                }
                let cur = self.lexer.pos().file.to_string();
                self.todo_files.push((cur, self.data.source));
                self.data.use_add(&name);
                self.record_use_path(&name, &f);
                self.switch_to_dep(&f);
            }
        }
        // The dependencies are resolved: a manifest read on the way is reported as itself
        // before what it failed to declare is reported as undefined.
        self.report_manifest_errors();
        // Apply wildcard/selective imports queued for this source now that the while-use loop
        // has resolved all libraries.  Must run before the definitions loop so that imported
        // names are visible when function bodies and type annotations are parsed.
        self.apply_pending_imports();
        self.file += 1;
        self.line = 0;
        loop {
            let is_pub = std::mem::take(&mut self.pub_taken) || self.lexer.has_token("pub");
            let before = self.data.definitions();
            let adopted_before = self.data.adopted_stub_count();
            if self.lexer.diagnostics().level() == Level::Fatal
                || (!self.parse_capability()
                    && !self.parse_enum()
                    && !self.parse_typedef()
                    && !self.parse_function()
                    && !self.parse_struct()
                    && !self.parse_interface()
                    && !self.parse_constant())
            {
                break;
            }
            // @FR-F-Surface: what is `pub` is what a declaration DECLARES, never where the
            // item was first named.  A type named above its declaration is a stub created
            // inside the declaration that NAMED it, so the range below holds stubs this
            // declaration only USED (still `Unknown`, not its to publish) and misses the one
            // it ADOPTED (created earlier, by a user; `note_stub_adopted` records each).  The
            // adopting declaration decides, either way (loft#1856).
            if is_pub {
                for d_nr in before..self.data.definitions() {
                    if self.data.def_type(d_nr) != DefType::Unknown {
                        self.data.def_mut(d_nr).pub_visible = true;
                    }
                }
            }
            for d_nr in self.data.adopted_since(adopted_before).to_vec() {
                self.data.def_mut(d_nr).pub_visible = is_pub;
            }
        }
        let res = self.lexer.peek().clone();
        if res.has != LexItem::None && self.lexer.diagnostics().level() != Level::Fatal {
            if self.lexer.peek_token("use") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "use statements must appear before all definitions"
                );
            } else {
                let token = match &res.has {
                    crate::lexer::LexItem::Token(s) => format!("'{s}'"),
                    crate::lexer::LexItem::Identifier(s) => format!("'{s}'"),
                    crate::lexer::LexItem::CString(s) => format!("\"{s}\""),
                    other => format!("{other:?}"),
                };
                diagnostic!(self.lexer, Level::Error, "Syntax error: unexpected {token}");
            }
        }
        // defer `Undefined type` errors to `resolve_deferred_unknowns`
        // so forward-references across cyclic intra-package `use` declarations
        // get a chance to resolve once both sides of the cycle are registered.
        typedef::actual_types_deferred(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
            Some(&mut self.deferred_unknown),
        );
        let cyclic_type = typedef::fill_all(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
        );
        // A type that contains itself has no finite size, so there is nothing for the record
        // builder to finish: `finish_type` follows the cycle into itself and the `u16` offset
        // accumulator wraps.  That panic is what the user meets — an internal compiler error,
        // and with the diagnostics still buffered, so the message naming the cure never
        // prints.  `fill_all` has already reported; stop before the layout.
        if !cyclic_type {
            self.database.finish();
        }
        // Validate layouts of all registered types — catches late-
        // mutation bugs (e.g. P191's bookkeeping fields landing at
        // overlapping positions because finish_type already ran).
        // Skip when the parser has already reported errors: an
        // incomplete struct (e.g. a self-referential type that the
        // parser correctly rejected) can have unlaid fields whose
        // position == u16::MAX, and re-flagging that here just adds
        // noise on top of the real diagnostic.
        let already_failed = matches!(
            self.lexer.diagnostics().level(),
            Level::Error | Level::Fatal
        );
        if !already_failed {
            for issue in self.database.validate_all_layouts() {
                diagnostic!(self.lexer, Level::Error, "type layout: {}", issue);
            }
        }
        self.enum_fn();
        let lvl = self.lexer.diagnostics().level();
        // FATAL stops; a plain Error does not.  `todo_files` holds the files SUSPENDED at
        // a `use` — the importer, waiting for the module it pulled in — and they have not
        // been parsed at all yet, so abandoning them does not avoid a cascade, it invents
        // one: the definitions those files carry never register, and every later question
        // about them is answered "undefined".  That is how one error in a module produced
        // a second saying `Colour` was undefined while it was declared two lines away in
        // the importer (loft#801).  Draining them can only ADD real information; any
        // diagnostic that then appears is one the author's own code earned.
        if lvl == Level::Fatal {
            return;
        }
        // Parse all files left in the todo_files list, as they are halted to parse a use file.
        while let Some((t, s)) = self.todo_files.pop() {
            self.lexer.switch(&t);
            self.data.source = s;
            self.parse_file();
        }
    }

    // For now, assume that returned texts are always related to internal variables
}

/// Walk a parsed expression looking for a `Value::FnRef` with a
/// non-MAX closure_var — the marker that the closure captures one or
/// P213: locate the capturing `Value::FnRef(d_nr, w, _)` inside `v` and
/// return `(d_nr, w)`.  Mirrors `capturing_fn_ref` (the bool variant)
/// but extracts the lambda's `d_nr` so callers can record it on the
/// host attribute via `Attribute::assigned_lambda_d_nr`.  Returns
/// `None` when no capturing FnRef is present.  Walks Block / Set /
/// Span wrappers built by `parser/vectors.rs` around the `OpDatabase`
/// allocation steps.
/// Widen a NON-CAPTURING lambda to the full fn-ref shape, in place, when `tp` is a
/// function type.  Answers whether the value was widened.
///
/// A fn-ref value is twenty bytes — an 8-byte `d_nr` then a 12-byte closure `DbRef`
/// (`ValueType::FnRef` in `state/codegen.rs`).  A CAPTURING lambda arrives as
/// `Value::FnRef(d_nr, closure_var, _)` and carries both halves.  A non-capturing one
/// is left by the expression parser as a bare `Value::Int(d_nr)` — the d_nr alone,
/// because its closure half would be the null sentinel anyway.  So one notion reaches
/// the IR in two spellings, and a consumer that writes the value at the width it finds
/// is right about the capturing one and twelve bytes short on the other.
///
/// Use this wherever a lambda's value flows into a `Function`-typed slot that something
/// else will read back at the full width: a branch join, a generator's yield channel.
/// It puts both spellings into the one shape, so the emitters read a single fact rather
/// than each re-deriving the padding.
///
/// A block keeps its block — only its last operator is widened, and its result type is
/// restated — so the arm still delivers through the slot the merge reads.
///
/// The fallback answers `false` for every other shape, and that is safe BECAUSE of the
/// `tp` gate: a `Function`-typed slot cannot hold an integer that is not a d_nr, so a
/// value that is neither bare spelling is one that already carries its own closure half
/// (a complete `FnRef`) or is not a literal at all (a `Var`, a `Call`, a nested branch
/// its own join already widened). Widening any of those would overwrite a live value.
pub(crate) fn widen_bare_fn_ref(v: &mut Value, tp: &Type) -> bool {
    if !matches!(tp.base(), Type::Function(..)) {
        return false;
    }
    let fn_tp = tp.base().clone();
    match v.unspan_mut() {
        Value::Block(bl) => {
            let Some(last) = bl.operators.last_mut() else {
                return false;
            };
            if widen_bare_fn_ref(last, tp) {
                bl.result = fn_tp;
                true
            } else {
                false
            }
        }
        other => {
            let d_nr = match other {
                Value::Int(d) => *d,
                Value::Long(d) => *d as i32,
                _ => return false,
            };
            *other = Value::FnRef(d_nr, u16::MAX, Box::new(fn_tp));
            true
        }
    }
}

pub(crate) fn find_capturing_fn_ref(data: &Data, v: &Value) -> Option<(i32, u16)> {
    match v.unspan() {
        // `w != MAX` only appears in the second pass (`emit_lambda_code`
        // builds the closure-allocation block there).  In the FIRST
        // pass a capturing lambda still emits as a plain
        // `FnRef(d, MAX)`, so also accept a lambda whose def carries a
        // synthesized closure record — `synthesize_closure_record`
        // runs in both passes, making it the pass-1-visible capture
        // marker.  Without this, `assigned_lambda_d_nr` is never set
        // in pass 1 and `fill_database` lays the field out as the
        // legacy 4B int — no `<attr>__closure_rec` half (#313).
        Value::FnRef(d, w, _)
            if *w != u16::MAX || data.def(*d as u32).closure_record() != u32::MAX =>
        {
            Some((*d, *w))
        }
        // First pass only: `emit_lambda_code` emits a lambda as a bare
        // `Int(d_nr)` there (no closure-allocation block yet), so a
        // capturing lambda is recognisable only through its def's
        // closure record.  Inside the fn-ref write arm an Int IS the
        // lambda's d_nr by construction (the non-capturing write path
        // stores exactly this Int).
        Value::Int(d)
            if *d >= 0
                && (*d as usize) < data.definitions.len()
                && data.def(*d as u32).closure_record() != u32::MAX =>
        {
            Some((*d, u16::MAX))
        }
        Value::Block(bl) => bl
            .operators
            .iter()
            .find_map(|op| find_capturing_fn_ref(data, op)),
        Value::Set(_, rhs) => find_capturing_fn_ref(data, rhs),
        _ => None,
    }
}

/// Does this operator WRITE THROUGH its first argument?
///
/// One home for a question two walkers used to answer from lists of their own, and the
/// lists had drifted apart in both directions: `find_written_vars` (which decides whether a
/// `&` parameter is ever modified) knew `OpNewRecord` and the keyed writes but not the text
/// ones, while `walk_for_mutations` (which decides which captures a lambda mutates) knew the
/// text ones and not `OpNewRecord` — so an append to a captured collection, which lowers to
/// `OpNewRecord` + `OpFinishRecord` through the closure record, was invisible to it.  A `&`
/// parameter whose only write was that append then read as never modified, and
/// `fn add(p: &vector<integer>) { g = fn() { p += [9]; }; g(); }` was refused with
/// *"has & but is never modified"* while the append it makes reaches the caller correctly
/// (loft#1276).
///
/// The list is the UNION, because each side's extras are genuine first-argument writes; the
/// name is the RAW one (`OpSetInt`, not `SetInt`) so a user function cannot collide with the
/// `Op` prefix.
///
/// ⚠ `OpCopyRecord` is NOT here — it writes through its SECOND argument, and its callers
/// The fused scalar appends (@PLN157 § V-m): each `OpSet<Kind>` an element literal writes at
/// offset 0 of a fresh element, and the ONE op `Parser::fuse_scalar_append` folds the
/// `OpNewRecord · OpSet<Kind> · OpFinishRecord` triple into.  The one home for the eight
/// names: every classifier that lists the element-build ops reads them from here
/// ([`FUSED_PUSH_OPS`]), so a kind added to the fusion reaches them all at once.
pub const FUSED_PUSH_KINDS: [(&str, &str); 8] = [
    ("OpSetByte", "OpPushByte"),
    ("OpSetInt", "OpPushInt"),
    ("OpSetInt4", "OpPushInt4"),
    ("OpSetFloat", "OpPushFloat"),
    ("OpSetSingle", "OpPushSingle"),
    ("OpSetBoolean", "OpPushBoolean"),
    ("OpSetEnum", "OpPushEnum"),
    ("OpSetCharacter", "OpPushCharacter"),
];

/// The push half of [`FUSED_PUSH_KINDS`].
pub const FUSED_PUSH_OPS: [&str; 8] = [
    "OpPushByte",
    "OpPushInt",
    "OpPushInt4",
    "OpPushFloat",
    "OpPushSingle",
    "OpPushBoolean",
    "OpPushEnum",
    "OpPushCharacter",
];

/// handle that separately.
pub(crate) fn op_writes_first_arg(name: &str) -> bool {
    name.starts_with("OpSet")
        // @PLN157 § V-m — the fused scalar appends (`OpPush<Kind>(v, x)`), the one-op form
        // of `OpNewRecord` + `OpSet<Kind>` + `OpFinishRecord`.
        || name.starts_with("OpPush")
        || name.starts_with("OpAppendStack")
        || name.starts_with("OpClearStack")
        // Every `OpFormat*` renders INTO the text its first argument names — the `"{x}"`
        // interpolation, `t + x` and `t += x` all lower to them.  Missing here, a `&text`
        // parameter whose only write was `t += "{5}"`, `t = t + 5` or `t += 5` was refused
        // as never modified (loft#1827).
        || name.starts_with("OpFormat")
        || name == "OpNewRecord"
        || name == "OpAppendCopy"
        || name == "OpAppendVector"
        // A span appended in one block (`r += src[a..b]`, `(R-ByteCopy)`) and a range kept in
        // place (`r = r[a..b]`) write their first argument as the append and clear above do.
        || name == "OpSliceVector"
        || name == "OpAppendTextBytes"
        || name == "OpKeepRange"
        || name == "OpAppendText"
        || name == "OpAppendCharacter"
        || name == "OpClearVector"
        || name == "OpClearText"
        || name == "OpClearKeyed"
        // @P320: keyed-remove `coll[key] = null` lowers to `OpHashRemove(coll, …)`
        // (collections.rs::towards_set_hash_remove), so it mutates its first arg just like
        // OpSetKeyed / OpClearKeyed.  Without this a `&` param whose only mutation is a
        // keyed remove was wrongly rejected as "never modified".
        || name == "OpHashRemove"
        || name == "OpInsertVector"
        || name == "OpRemoveVector"
        // Reordering ops rewrite every element of the collection they are given, which is a
        // write to the first argument in exactly the sense `OpRemoveVector` above it is.
        // They were missing while `insert` and `remove` were present, so a `&` parameter
        // whose only mutation was a `reverse` or a `sort` was rejected as "never modified"
        // — the shape a caller reaches for when the reordering is the function's whole job.
        // `OpReserveVector` / `OpReserveHash` are deliberately NOT here: reserve is a
        // capacity hint that changes neither the length nor the contents, so a `&` whose
        // only use is a reserve really does serve no purpose.
        || name == "OpReverseVector"
        || name == "OpSortVector"
        // Delivers into its FIRST arg (`vector_replace(&r, &other, tp)`) — the NRVO return
        // buffer. Today every emit site also writes that slot another way (a
        // `__retbuf = call(…)` Set, or the BlockTail path's `OpClearVector`), so the
        // omission is masked INCIDENTALLY rather than by design.
        || name == "OpReplaceVector"
        // The second half of an append through a collection the caller owns; it names the
        // same collection `OpNewRecord` did, and a walker that sees only one of the pair
        // reads a completed append as half of one.
        || name == "OpFinishRecord"
}

/// Recursively walk a Value IR tree and collect all variable indices that are written.
/// A variable is considered written if:
/// - It appears as the target of `Value::Set(v, ...)`,
/// - It is passed as a `RefVar`-typed argument to a `Value::Call`, or
/// - It appears anywhere in the first argument of a field-write operator (`OpSet*`),
///   which covers the pattern `v[idx].field = val` where `v: &vector<T>`.
/// - It flows into a callee whose own body mutates that parameter
///   (directly or transitively via further calls).  The
///   interprocedural lookup is memoised via `callee_cache`.
pub(crate) fn find_written_vars(
    code: &Value,
    data: &Data,
    vars: &crate::variables::Function,
    written: &mut crate::fxhash::FxHashSet<u16>,
    callee_cache: &mut crate::fxhash::FxHashMap<u32, Vec<bool>>,
) {
    match code {
        Value::Set(v, body) => {
            written.insert(*v);
            find_written_vars(body, data, vars, written, callee_cache);
        }
        Value::Call(fn_nr, args) => {
            let def = data.def(*fn_nr);
            let attrs = def.attributes();
            // Operators whose FIRST argument is mutated (collection / field writes).
            // vector ops folded in here so `c.items += other_vec` (where `c.items`
            // is `OpGetField(Var(c), …)`) correctly marks `c` as written via
            // collect_vars_in.  Previously the OpAppend*/OpClear* family only checked for
            // a bare `Value::Var` arg, missing the field-access shape.
            let first_arg_write = op_writes_first_arg(def.name());
            // OpCopyRecord(src, dst, type) writes through `dst` (arg[1]).
            // Used by struct field whole-replacement (`s.i = fresh`) where the
            // destination is `OpGetField(s, …)`.
            let second_arg_write = def.name() == "OpCopyRecord";
            for (i, arg) in args.iter().enumerate() {
                if i < attrs.len()
                    && matches!(attrs[i].typedef, Type::RefVar(_))
                    && let Value::Var(v) = arg
                {
                    written.insert(*v);
                }
                if i == 0 && first_arg_write {
                    collect_vars_in(arg, written);
                }
                if i == 1 && second_arg_write {
                    collect_vars_in(arg, written);
                }
                find_written_vars(arg, data, vars, written, callee_cache);
            }
            // the callee may mutate one of its by-value parameters
            // through a field write (e.g. `fn add(self: Box, x) { self.items += [x] }`).
            // Look up its param-write effects and mark the corresponding
            // caller-side arg vars so `check_ref_mutations` sees them as
            // mutated.  Skip natives (`def.code == Value::Null`) — their
            // effects are already encoded by the OpSet*/OpAppend*/OpCopyRecord
            // patterns above.  Args are collected with `collect_vars_in` so
            // wrapped sources (field access, `OpCreateStack(Var(_))` from
            // the hoisted-preamble path) still propagate the mutation to
            // their root var.
            if *def.code() != Value::Null {
                let callee_writes = callee_param_writes(*fn_nr, data, callee_cache);
                for (i, arg) in args.iter().enumerate() {
                    if i < callee_writes.len() && callee_writes[i] {
                        collect_vars_in(arg, written);
                    }
                }
            }
        }
        // A call through a fn-ref: the callee is not known here, but the fn-ref's TYPE names
        // which parameters are `&`, and handing a value to one IS a write the callee may make —
        // `fn app(f: fn(&P), p: &P) { f(p); }` was told to drop the `&` it needs.
        Value::CallRef(f, args) => {
            let params = match (*f < vars.count()).then(|| vars.tp(*f).base()) {
                Some(Type::Function(params, ..)) => params.clone(),
                _ => Vec::new(),
            };
            for (i, arg) in args.iter().enumerate() {
                if params
                    .get(i)
                    .is_some_and(|p| matches!(p.base(), Type::RefVar(_)))
                {
                    collect_vars_in(arg, written);
                }
                find_written_vars(arg, data, vars, written, callee_cache);
            }
        }
        Value::Block(block) | Value::Loop(block) => {
            for item in &block.operators {
                find_written_vars(item, data, vars, written, callee_cache);
            }
        }
        Value::Insert(list) => {
            for item in list {
                find_written_vars(item, data, vars, written, callee_cache);
            }
        }
        Value::If(cond, then, els) => {
            find_written_vars(cond, data, vars, written, callee_cache);
            find_written_vars(then, data, vars, written, callee_cache);
            find_written_vars(els, data, vars, written, callee_cache);
        }
        Value::Return(v) | Value::Drop(v) => {
            find_written_vars(v, data, vars, written, callee_cache);
        }
        // T1.5: TuplePut writes to the ref-tuple variable via its element assignment.
        Value::TuplePut(var_nr, _, inner) => {
            written.insert(*var_nr);
            find_written_vars(inner, data, vars, written, callee_cache);
        }
        Value::Iter(_, create, next, extra) => {
            find_written_vars(create, data, vars, written, callee_cache);
            find_written_vars(next, data, vars, written, callee_cache);
            find_written_vars(extra, data, vars, written, callee_cache);
        }
        Value::Span(b) => find_written_vars(&b.1, data, vars, written, callee_cache),
        _ => {}
    }
}

/// For each parameter of `fn_nr`, does the body REASSIGN the whole binding (`p = …`)?
///
/// The narrower sibling of [`callee_param_writes`], and the difference is the whole point:
/// that one answers *"is this parameter written at all"*, which a FIELD write satisfies, and a
/// field write is exactly the case the `slow-reference-parameter` advice exists to flag.  Only
/// a whole-binding reassignment is what `&` is for, so only that may silence it.
///
/// Interprocedural for the same reason the write query is: a FORWARDER never reassigns —
/// its callee does — so the one shape where a `&` is carrying someone else's write-back is
/// exactly the shape a body-local walk reads as redundant (loft#1286).
///
/// The recursion is broken by the same placeholder-then-merge the write query uses, and the
/// `cache` is shared with it deliberately: both are keyed by definition and neither writes an
/// entry the other reads, because this one never calls `find_written_vars`.
pub(crate) fn callee_param_reassigns(
    fn_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, Vec<bool>>,
) -> Vec<bool> {
    if let Some(v) = cache.get(&fn_nr) {
        return v.clone();
    }
    let def = data.def(fn_nr);
    let n = def.attributes().len();
    cache.insert(fn_nr, vec![false; n]);
    if *def.code() == Value::Null || n == 0 {
        return vec![false; n];
    }
    let body = def.code().clone();
    let mut out = vec![false; n];
    collect_param_reassigns(&body, data, n, &mut out, cache);
    let prev = cache.get(&fn_nr).cloned().unwrap_or_else(|| vec![false; n]);
    let merged: Vec<bool> = prev.iter().zip(out.iter()).map(|(a, b)| *a || *b).collect();
    cache.insert(fn_nr, merged.clone());
    merged
}

/// For each parameter of `fn_nr`, does a call to it REBIND that parameter to a store the
/// callee OWNS?
///
/// The narrower sibling of [`callee_param_reassigns`]: that one answers *"is the whole
/// binding reassigned"*, which a rebind to a BORROWED value also satisfies.  Only an OWNED
/// value displaces a store — the callee mints a new one and the binding stops naming what it
/// named — so only that asks the ownership-transition question this answers.
///
/// The caller of such a function needs the answer at the CALL SITE: the write-back lands in
/// the caller's binding, so the store the binding stopped naming and the fresh one it now
/// names are both the caller's question to settle, and the callee cannot see which of them it
/// owns (loft#1287).
///
/// Interprocedural for the same reason its sibling is — a FORWARDER never reassigns, its
/// callee does — and it shares the recursion-breaking placeholder-then-merge.  The `cache` is
/// its own: the two queries answer different questions about the same key.
pub(crate) fn callee_param_rebinds_owned(
    fn_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, Vec<bool>>,
) -> Vec<bool> {
    if let Some(v) = cache.get(&fn_nr) {
        return v.clone();
    }
    let def = data.def(fn_nr);
    let n = def.attributes().len();
    cache.insert(fn_nr, vec![false; n]);
    if *def.code() == Value::Null || n == 0 {
        return vec![false; n];
    }
    let body = def.code().clone();
    let mut out = vec![false; n];
    collect_param_rebinds_owned(&body, fn_nr, data, n, &mut out, cache);
    let prev = cache.get(&fn_nr).cloned().unwrap_or_else(|| vec![false; n]);
    let merged: Vec<bool> = prev.iter().zip(out.iter()).map(|(a, b)| *a || *b).collect();
    cache.insert(fn_nr, merged.clone());
    merged
}

/// Like `find_written_vars` but only collects variables that are FIELD-written
/// (OpSet*, OpCopyRecord, OpNewRecord first-arg).  Excludes plain `Value::Set`
/// which includes loop-iterator advance — that's not a user-initiated mutation.
/// Used by check_ref_mutations to detect when a for-loop variable's field
/// writes should propagate back to the iterated `&` collection, and by the
/// @PLN101 value-struct copy-elision pass (`scopes::value_struct_copy`) to prove
/// a read-only view's base is never mutated under it.
pub(crate) fn find_field_written_vars(
    code: &Value,
    data: &Data,
    written: &mut crate::fxhash::FxHashSet<u16>,
) {
    match code {
        Value::Call(fn_nr, args) => {
            let def = data.def(*fn_nr);
            let first_arg_write = def.name().starts_with("OpSet")
                || def.name().starts_with("OpPush")
                || def.name() == "OpNewRecord"
                || def.name() == "OpAppendCopy"
                || def.name() == "OpAppendVector"
                || def.name() == "OpClearVector"
                || def.name() == "OpClearKeyed"
                || def.name() == "OpSetKeyed"
                // @P320: keyed-remove `coll[key] = null` → `OpHashRemove(coll, …)`;
                // mirror the find_written_vars set so a keyed remove inside a
                // `for … in &coll` loop also counts as a mutation.
                || def.name() == "OpHashRemove"
                || def.name() == "OpInsertVector"
                || def.name() == "OpRemoveVector"
                // Delivers into its FIRST arg (`vector_replace(&r, &other, tp)`) — the NRVO
                // return buffer. Today every emit site also writes that slot another way
                // (a `__retbuf = call(…)` Set, or the BlockTail path's `OpClearVector`), so
                // the omission is masked INCIDENTALLY rather than by design.
                || def.name() == "OpReplaceVector";
            let second_arg_write = def.name() == "OpCopyRecord";
            for (i, arg) in args.iter().enumerate() {
                if i == 0 && first_arg_write {
                    collect_vars_in(arg, written);
                }
                if i == 1 && second_arg_write {
                    collect_vars_in(arg, written);
                }
                find_field_written_vars(arg, data, written);
            }
        }
        Value::Set(_, body) => find_field_written_vars(body, data, written),
        Value::Block(block) | Value::Loop(block) => {
            for item in &block.operators {
                find_field_written_vars(item, data, written);
            }
        }
        Value::Insert(list) => {
            for item in list {
                find_field_written_vars(item, data, written);
            }
        }
        Value::If(cond, then, els) => {
            find_field_written_vars(cond, data, written);
            find_field_written_vars(then, data, written);
            find_field_written_vars(els, data, written);
        }
        Value::Return(v) | Value::Drop(v) => find_field_written_vars(v, data, written),
        Value::Iter(_, create, next, extra) => {
            find_field_written_vars(create, data, written);
            find_field_written_vars(next, data, written);
            find_field_written_vars(extra, data, written);
        }
        Value::Span(b) => find_field_written_vars(&b.1, data, written),
        _ => {}
    }
}

/// Map an operator token to its CamelCase name suffix used in `OpCamelCase` identifiers.
/// E.g. `"<"` → `"Lt"`, so the method name becomes `"OpLt"`.
/// Also used by I3.1 (`op <token>` sugar in interface bodies).
pub(crate) fn rename(op: &str) -> &str {
    match op {
        "*" => "Mul",
        "+" => "Add",
        "-" => "Min",
        "/" => "Div",
        "&" => "Land",
        "|" => "Lor",
        "^" => "Eor",
        "<<" => "SLeft",
        ">>" => "SRight",
        "==" => "Eq",
        "!=" => "Ne",
        "<" => "Lt",
        "<=" => "Le",
        ">" => "Gt",
        ">=" => "Ge",
        "%" => "Rem",
        "**" => "Pow",
        "!" => "Not",
        "~" => "BitNot",
        "+=" => "Append",
        _ => op,
    }
}
#[cfg(test)]
mod h5_changed_source_tests;
#[cfg(test)]
mod p269_native_backfill_tests;
#[cfg(test)]
mod plan86_admission_tests;
#[cfg(test)]
mod plan86_nesting_guard_tests;
#[cfg(test)]
mod plan86_reachable_set_tests;
#[cfg(test)]
mod plan86_sandbox_designation_tests;
