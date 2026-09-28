<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Closed-by-Decision Register

Questions that were evaluated and decided, so they do not come back every session.  Before
proposing one of these as a feature, fix or plan item, read its entry: if nothing material
has changed, the decision stands.  Each entry states the decision, the reason it rests on and
what would reopen it; the question as raised, the trade-offs and the evidence are in
[DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md) under the same heading, which each
entry's **record** link opens.  [SUBJECTS.md](SUBJECTS.md) groups the entries by the axis they
decide.

## Using the register

- **Closed is not backlog.**  A closed question stays out of ROADMAP.md's milestones,
  PLANNING.md's priorities and QUALITY.md's open work; a pointer in "Out of scope" is enough.
- **Reopening needs new evidence** — a use case, an incident or a measurement not available at
  the decision.  Add it to the entry's record and change the entry here; never flip one silently.
- **Adding an entry:** take the next free id (the highest is C129).  Append the deliberation —
  question, evaluation, dated decision, revisit trigger — to the record, and write the compact
  entry here under the same heading, in the shape the entries below use: **Decision** and
  **Why**, then **Revisit when**, the date and the record link.  In the source doc, strike the
  question (`~~…~~`) and point at the entry.
- **A decision is held to what it says (@PLN175).**  Every place that keeps it — the refusal
  site, the code that implements it, the doc that states it, and at least one guard under
  `tests/` that fails on a build breaking it — cites `@C<n>`.  `./scripts/idx tag:@C<n>` lists
  them and `./scripts/idx decisions` counts them per entry (`make index` first).  A decision no
  site can keep is not a decision: reopen it.  Where code and entry disagree, the code moves,
  unless the owner reopens the entry.  Gate: every `@C<n>` names an entry here, and an entry
  numbered C130 or later lands with its guard (`tests/index_hygiene.rs`).
- **`Catalogue:`** names the `@F`/`@I` catalogue entries a decision limits or shapes, so
  `./scripts/idx tag:@F<n>` shows a feature's design bounds beside its code (@PLN92).

---

## C3 — WASM `par()` runs sequentially

**Decision.** In the browser, `par()` runs its body sequentially; the native target keeps real
parallelism.  **Why.** A Web Worker pool costs bundle size, ~50 ms cold start per worker and a
`SharedArrayBuffer` that needs COOP/COEP headers most loft hosts do not send — and no shipping
loft program is CPU-bound in the browser.

**Revisit when.** A loft program shows a browser CPU bottleneck that algorithmic work cannot
remove, AND its host sends COOP/COEP headers — bring the profile.  Decided 2026-04 —
[record](DESIGN_DECISIONS-history.md#c3--wasm-par-runs-sequentially).
**Catalogue:** @F33 (par), @F54 (WASM/browser target).

## C38 — Closure capture is copy-at-definition

**Decision.** A read-only scalar capture is a copy taken at definition; a struct capture references
the live original.  A scalar the body WRITES becomes a heap cell shared by the outer scope and the
one mutating closure (several are refused — C74).  **Why.** Reference capture of scalars needs a
garbage collector or a borrow checker, neither fitting the store heap; a copy surprises least.

**Revisit when.** A critical program cannot be written ergonomically with these captures AND an
alternative is prototyped without destabilising the store-based heap.  Decided 2026-04, amended
2026-05-13 — [record](DESIGN_DECISIONS-history.md#c38--closure-capture-is-copy-at-definition).
**Holds at:** `tests/scripts/56-closures.loft::test_capture_timing`.
**Catalogue:** @F22 (closures & lambdas).

## C54.D — Rust-style numeric literal suffixes

**Decision.** No literal suffixes (`34u8`, `100i32`); the width comes from the binding or
parameter type, or from `34 as u8`.  **Why.** Context typing already covers every common case;
suffixes would add lexer ambiguity and put the intent on the literal instead of the binding.

**Revisit when.** A real program needs a literal-size distinction `as <T>` cannot express
reasonably ("Rust does it" is not evidence).  Decided 2026-04-13 —
[record](DESIGN_DECISIONS-history.md#c54d--rust-style-numeric-literal-suffixes).
**Catalogue:** @F4 (width integers), @F5 (type conversions — `as`).

## C62 — No type annotations in `|x|` shorthand lambdas

**Decision.** `|x: T| { … }` is a compile error that points at `fn(x: T) -> R { … }`.  The
shorthand is for inferred parameters; the `fn(…)` form is the typed one.  **Why.** Typed
shorthand would make the two forms mean the same thing, turn the choice into a coin-flip, and
add parser branches for no capability the `fn` form lacks.

**Revisit when.** Never, short of a rewrite that removes the inferred/explicit distinction.
Decided 2026-04-17 — [record](DESIGN_DECISIONS-history.md#c62--no-type-annotations-in-x-shorthand-lambdas).
**Catalogue:** @F22 (closures & lambdas).

## C63 — No nested `fn` definitions inside fn bodies

**Decision.** A `fn` definition inside a function or block is refused (`'fn' definitions must be
at file scope, not inside a function or block`); a local helper is a lambda,
`let helper = fn(x: integer) -> integer { x * 2 };`.  **Why.** A nested `fn` either captures
(closures under a second syntax) or does not (surprising against every mainstream language);
both are sugar with no capability closures lack.

**Revisit when.** A real workflow shows the typed-lambda form costing more developer time than
implementing nested `fn` would.  Decided 2026-05-04 —
[record](DESIGN_DECISIONS-history.md#c63--no-nested-fn-definitions-inside-fn-bodies).
**Catalogue:** @F16 (functions & declarations).

## C64 — Tuple struct-ref elements use MOVE semantics (not copy + null)

**Decision.** A struct-reference tuple element MOVES on destructure: each destination variable
owns its element and frees it at its own scope exit; the source tuple frees nothing per element.
**Why.** That is what the runtime already does, correctly on both backends; copy-then-null adds
an opcode and a runtime write per destructure with no observable difference.

**Revisit when.** A shape appears where move is observably wrong and copy + null right.  Decided
2026-05-11 — [record](DESIGN_DECISIONS-history.md#c64--tuple-struct-ref-elements-use-move-semantics-not-copy--null).
**Holds at:** `tests/tuple_matrix.rs` (the `e5_*` struct-ref cells).
**Catalogue:** @F11 (tuples).

## C65 — Tuple "structure value" element type folded into reference (E5 = E6)

**Decision.** A tuple element of struct type is a reference (E5); there is no separate by-value
struct element (E6).  **Why.** loft has no inline by-value struct type — a struct value is a
`DbRef` to a store record — so an E6 row would duplicate E5 or need a new type variant with no
consumer.

**Revisit when.** A feature introduces inline value structs.  Decided 2026-05-11 —
[record](DESIGN_DECISIONS-history.md#c65--tuple-structure-value-element-type-folded-into-reference-e5--e6).
**Catalogue:** @F11 (tuples).

## C66 — Production loft programs never abort on user-attributable edge cases (development may halt)

**Decision.** A production run (`--production`, the logger's `production` flag) never aborts:
`panic` and a failed `assert` log, set `had_fatal` and continue; in development they halt with a
typed error.  Calculation faults yield null in EVERY mode (C80); a `{…}` interpolation never raises.
**Why.** loft's targets — games, scenes, servers — must not stop; a frozen program is worse than a
wrong value, while a developer wants the loud stop.

**Revisit when.** A deployment shape surfaces where log-and-continue is wrong.  Decided
2026-05-11, calculation faults revised by C80 on 2026-06-24 —
[record](DESIGN_DECISIONS-history.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt).
**Holds at:** `tests/panic_halts_both_backends.rs`; LOGGER.md.
**Catalogue:** @F38 (arithmetic safety), @F44 (logging — panic/assert).

## C67 — Fail at startup, not at runtime (no programmer-side try/catch for internal bugs)

**Decision.** No `try`/`catch`, no `#panic_safe`, no defensive boilerplate for internal bugs: an
internal bug is refused at compile time (no reachable `todo!()`), a startup fault exits non-zero for
the supervisor, and a steady-state external fault (network, disk) is handled inside the LIBRARY
behind a clean API.  User-domain errors as values stay allowed.  **Why.** Wrap-everything burdens
every programmer and hides crashes from the supervisor.

**Revisit when.** A class of internal bug escapes compile-time analysis, or typed errors grow
more boilerplate than a catch would — the typed-error mechanism may then evolve; try/catch for
internal-bug recovery stays closed.  Decided 2026-05-13 —
[record](DESIGN_DECISIONS-history.md#c67--fail-at-startup-not-at-runtime-no-programmer-side-trycatch-for-internal-bugs).
**Catalogue:** @F44 (logging — panic/assert).

## C68 — Keyed collections dedup on insert (`+=` AND `coll[key]=value`)

**Decision.** On a `hash`, `sorted` or `index`, both `coll += [entry]` and `coll[key] = value`
replace an existing entry with the same key — the latest insert wins, and `len` counts keys.
**Why.** A keyed collection promises key uniqueness; the world-chunk index needs an insert at an
occupied coordinate to replace, not stack a shadowed duplicate.

**Revisit when.** Bulk inserts of known-unique keys are measured too slow for the dedup check.
Decided 2026-05-21 (the same day's append/upsert split was reversed) —
[record](DESIGN_DECISIONS-history.md#c68--keyed-collections-dedup-on-insert--and-collkeyvalue).
**Catalogue:** @F7 (hash), @F8 (sorted), @F9 (index).

## C69 — `!x` on a non-boolean is a null test, not logical-not

**Decision.** `!x` on a non-boolean asks "is `x` null?", so `!0` is `false`; boolean `!` negates,
with `!null == true`.  `!x` on a statically non-null operand warns that it is always false.
**Why.** Null is in-band and the null-test idiom carries load (`!both` in the stdlib's
`min`/`max`/`clamp`); C-style coercion would invert it, a compile error reject it.

**Revisit when.** The idiom is shown to cause a recurring class of bugs the warning and the
documentation miss, AND a replacement is prototyped that does not churn the stdlib ("I expected
C semantics" is not evidence).  Decided 2026-06-03 —
[record](DESIGN_DECISIONS-history.md#c69--x-on-a-non-boolean-is-a-null-test-not-logical-not).
**Holds at:** `tests/parse_errors.rs::gh253_bang_on_not_null_warns`, `gh253_bang_on_nullable_is_quiet`.
**Catalogue:** @F37 (operators — unary `!`), @F1 (null model).

## C70 — No per-library IR snapshot / cache

**Decision.** Only whole-prefix bundles are cached (core, and core plus a script's sorted
library set); a library never ships or caches its own IR.  **Why.** The IR's definition and type
numbers are global and parse-order dependent, so a lone library image would need relocation by
name — the brittlest mechanism available; and the `.loft` source is the better artefact, parsed
as fast as serialized IR loads.

**Revisit when.** First-run parse of a never-seen `use` set is measured to be a real bottleneck
AND a relocation scheme is prototyped without the global-index brittleness.  Decided 2026-06-02 —
[record](DESIGN_DECISIONS-history.md#c70--no-per-library-ir-snapshot--cache).

## C71 — Native libraries compile, scripts interpret — the steady-state execution model

**Decision.** Stable libraries compile to native once, cached per library and keyed on the loft
build fingerprint (`libloft.rlib` content hash, rustc version, target, features — never git HEAD or
mtime); the user's script interprets.  The choice is automatic: code that cannot cross falls back to
the interpreter silently.  WASM and `--html` stay whole-program compiled.  **Why.** Native speed
where code is stable, no `rustc` per edit, and the shared store heap passes `DbRef`s unmarshalled.

**Revisit when.** A dispatch-coverage audit shows the boundary cannot express a critical class
of library API without marshalling cost that erases the speed gain, measured for a real
consumer.  Decided 2026-06-04 — [record](DESIGN_DECISIONS-history.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model).
**Holds at:** [BROADENING.md § Native-library execution model](BROADENING.md#native-library-execution-model--the-steady-state-design).
**Catalogue:** @F53 (native backend), @F55 (package management).

## C72 — REPL session resume does not persist RNG generator state

**Decision.** Resume restores stored values verbatim but not the random generator's state; the
generator continues fresh, seeded from entropy as on any launch.  A reproducible stream is an
explicit `random_seed`.  **Why.** A saved generator state would let anyone who reads the session
file predict future `random()` output, and an explicit seed already covers determinism.

**Revisit when.** A non-security use case needs byte-identical RNG continuation across resume
that an explicit seed cannot give.  Decided 2026-06-08 — [record](DESIGN_DECISIONS-history.md#c72--repl-session-resume-does-not-persist-rng-generator-state).
**Catalogue:** @F49 (REPL), @F43 (random numbers).

## C73 — `boolean` is three-state (false / true / null); `==` is raw, truthiness coerces

**Decision.** A `boolean` is one byte: `false` = 0, `true` = 1, `null` = 255, kept distinct
everywhere a boolean lives.  `==` and `!=` compare the raw value, so `null == false` is `false`
and `b == null` is the null test; `if`, `while`, `!`, `&&` and `||` treat null as false; `??`
tests for null, so `false ?? x` stays `false`.  **Why.** This is exactly what `integer` already
does (`0 == null` is `false`); coercing inside `==` would make boolean the one inconsistent type.

**Revisit when.** Never, short of a change to the in-band null model.  Decided 2026-06-10 — [record](DESIGN_DECISIONS-history.md#c73--boolean-is-three-state-false--true--null--is-raw-truthiness-coerces).
**Holds at:** `tests/scripts/292-pln17-three-state-boolean.loft`.
**Catalogue:** @F3 (scalar types — boolean), @F1 (null model).

## @PLN116 — the `x?` default-fallback operator + enum-field non-null soundness

**Decision.** Postfix `x?` (tightest precedence) yields `x`, or the type's default when `x` is
null; `a ?? b?` parses as `a ?? (b?)`.  One predicate decides "T has a default" for both `x?`
and an omitted field in `S{}`.  A non-optional enum field with no `= expr` has no default, so
`x?` on it and an `S{}` that omits it are compile errors; `x?` on an `E?` value gives the first
variant, and an optional enum field (`Color?`) defaults to null.  **Why.** An enum's 0 is its
null, so zero-filling a non-null enum field would put null into a non-null slot.

**Revisit when.** An enum can nominate its own default variant — an additive extension, not a
change to this rule.  Decided before contract 1 — [record](DESIGN_DECISIONS-history.md#pln116--the-x-default-fallback-operator--enum-field-non-null-soundness).
**Holds at:** [plans/116-default-fallback-operator/](plans/116-default-fallback-operator/README.md).

## C74 — A mutated scalar may be captured by only ONE closure

**Decision.** More than one closure capturing a scalar that any of them mutates is refused:
*"sharing a mutable variable between closures is not supported — hold the shared state in a struct
field instead"*.  One mutating closure, any number of readers, and any number sharing struct-field
state stay supported.  **Why.** A shared scalar cell has no owner in the store model: the first
closure to die would free it under the others.

**Revisit when.** A real consumer's shape is materially clumsier as a struct AND the shared cell
gets a single defined owner first.  Decided 2026-06-10 — [record](DESIGN_DECISIONS-history.md#c74--a-mutated-scalar-may-be-captured-by-only-one-closure).
**Holds at:** `Parser::reject_shared_mutable_scalar_captures` (`src/parser/vectors.rs`);
`tests/issues.rs::issue_314_*`.
**Catalogue:** @F22 (closures & lambdas).

## C75 — Closure-carrying struct values are frame-bound

**Decision.** A struct that holds a capturing closure cannot leave the frame that owns the
captures: returning such a struct type, writing a capturing closure into a struct rooted at an
argument, and a collection of such structs are compile errors.  Local use and passing it down as
an argument are supported.  **Why.** The closure record holds references into the frame's stores,
which are freed at return, so an escaped closure would read and write unrelated live data.

**Revisit when.** A consumer needs factory-built closure-holding structs AND the deep copy gains
a designed ownership transfer of the captured stores, verified on both backends.
Decided 2026-06-10 — [record](DESIGN_DECISIONS-history.md#c75--closure-carrying-struct-values-are-frame-bound).
**Holds at:** `Parser::type_carries_closure` (`src/parser/mod.rs`); `tests/issues.rs::issue_318_*`.
**Catalogue:** @F22 (closures & lambdas).

## C76 — Selective imports group with `()`, not Rust-style `{}`; flat comma list dropped

**Decision.** Import several names as `use lib::(a [as x], b, c);`; a single `use lib::name [as
bind];` is unchanged.  The flat list `use lib::a, b;` is an error ("import multiple names with
parentheses"), and `{}` is not accepted.  **Why.** `()` is loft's grouping delimiter and reads as
"these belong to `lib::`"; `{}` stays reserved for blocks and struct literals.

**Revisit when.** Nested path grouping (`use a::(b::c, d)`) is needed and parses without
colliding with the struct-literal or call grammar.  Decided 2026-06-14 — [record](DESIGN_DECISIONS-history.md#c76--selective-imports-group-with--not-rust-style--flat-comma-list-dropped).
**Holds at:** `tests/imports.rs::pln22_phase4_grouped_import`, `::pln22_phase4_flat_list_rejected`.
**Catalogue:** @F47 (library imports / module system).

## C77 — Binding ownership: heap aliases by default; `&` binds a live reference

**Decision.** `&` is a binding marker that binds a live reference — reads and writes go through to
an addressable source — not a general operator.  A plain binding follows
[C86](#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule): a
whole-value heap bind COPIES (aliased only when the source is dead), a projection (`a = vv[0]`, `w =
o.inner`) is a view, and in-place path mutation writes through.  **Why.** One uniform `&` for "I
want the link", instead of a copy-or-view choice made silently by the binding's form.

**Revisit when.** Views by projection prove a net footgun for a real consumer — writes
propagating through an alias it did not intend.  Decided 2026-06-22, plain binds superseded by C86 — [record](DESIGN_DECISIONS-history.md#c77--binding-ownership-heap-aliases-by-default--binds-a-live-reference).
**Holds at:** [OWNERSHIP_MODEL.md § The law](OWNERSHIP_MODEL.md), @FR-B-Ref-Alias.
**Catalogue:** @F21 (references `&T`).

## C78 — The Rust-engine ↔ loft-library boundary: mechanism not genre, and no black boxes above the engine

**Decision.** The Rust core ships mechanism, never a genre: an opinionated game model (the hex
streaming world) stays an optional, replaceable, unprivileged loft library.  The line is "should a
game developer ever need to open this?" — no (codegen, type checker, allocator) goes to Rust; yes
(world model, gameplay primitives, rendering composition) stays loft, even when hard.  Libraries are
kits of primitives lifted one at a time; needing to move game-facing code into Rust is a language
bug to fix in loft.  **Why.** A genre in the compiler taxes every other author, and Rust code is a
black box the developer cannot read or fork.  Orthogonal to C71 (how a library runs).

**Revisit when.** A genre-neutral mechanism in a library (likely the streaming substrate) is a
measured bottleneck only the engine can fix, and moving it buries neither a worldview nor a
primitive a developer needs to read.  "Hard" or "reused" alone is not enough.
Decided 2026-06-23 — [record](DESIGN_DECISIONS-history.md#c78--the-rust-engine--loft-library-boundary-mechanism-not-genre-and-no-black-boxes-above-the-engine).

## C79 — Ownership is internal; no user-facing borrow checker

**Decision.** No lifetime annotations and no borrow errors on ordinary code: the compiler always
finds a correct free, copy or move, copying when unsure; `&` is the one user-facing ownership
concept.  Where loft cannot honour an explicit ask — a `&` whose place the program then removes,
re-keys or replaces — it refuses the program rather than silently copy.  **Why.** Rust's borrow
checker is the hurdle loft exists to spare; a refusal can later be dropped without breaking anyone,
a silent substitute would freeze into the contract.

**Revisit when.** A measured cost from copying meets a narrow, clearly diagnosable surface — one
named case, never the general Rust model.  Decided 2026-06-24, widened 2026-08-05 — [record](DESIGN_DECISIONS-history.md#c79--ownership-is-internal-no-user-facing-borrow-checker).
**Holds at:** @FR-O-Complete, @FR-B-Ref-Reshape; [COMPATIBILITY.md](COMPATIBILITY.md).
**Catalogue:** @F21 (references `&T`).

## C80 — The spreadsheet fault model: nothing stops a running calculation

**Decision.** A calculation that cannot produce a value — division by zero, integer overflow, an
out-of-bounds index, a deref of an absent value — yields null and execution continues, identically
in development, test and production.  It is silent except for one warning on an unguarded division
by zero; tests observe faults through the debug log.  `panic` and `assert` are not calculations:
they halt in development and tests and log-and-continue in production (C66).  Startup failures still
stop (C67).  **Why.** As in a spreadsheet, one bad cell never stops the others: degradation stays
local, with no unwinding or cleanup blocks to get wrong.

**Revisit when.** Silent null-and-continue loses critical information for a consumer — and even
then the fix is more observability, never a halt.  Decided 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation).
**Holds at:** [formal/operational.md](formal/operational.md) @FR-E-Uncomp, @FR-E-Report.
**Catalogue:** @F38 (arithmetic safety), @F1 (null model), @F44 (logging — panic/assert).

## C81 — `&` stays one token, disambiguated by position (bitwise-and vs reference)

**Decision.** One `&` token: infix is bitwise-and, prefix is the reference annotation.  A prefix
`&` is a parse error anywhere but a binding, so position always decides.  **Why.** The rule is
total, Rust makes the same call, and a second sigil would add surface for no semantic gain.

**Revisit when.** A generated parser that cannot reproduce the positional rule hits a real
ambiguity.  Decided 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c81---stays-one-token-disambiguated-by-position-bitwise-and-vs-reference).
**Holds at:** @FR-B-Ref-AnnotationOnly.
**Catalogue:** @F21 (references `&T`), @F37 (operators — bitwise `&`).

## C82 — loft's surface is deliberately not context-free

**Decision.** loft keeps constructs that need backtracking or lexer modes — type versus
variable, `S { … }` versus a block, `"{e}"` interpolation — and does not pursue a context-free
grammar; the hand-written parser is the grammar, and tooling reuses it.  **Why.** Those points buy
real ergonomics, and nothing has needed a generated parser.

**Revisit when.** A third-party grammar-based tool genuinely cannot reuse loft's parser AND the
context-sensitive points can be expressed without unbounded backtracking.
Decided 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c82--lofts-surface-is-deliberately-not-context-free).

## C83 — The internal representation follows the user-visible contract; never widen storage for implementation convenience

**Decision.** Storage is never widened for implementation uniformity: the internal model serves the
user-visible contract at the smallest encoding keeping its semantics.  `integer` is i64 on every
read and computation, while the IR keeps `Value::Int(i32)` / `Value::Long(i64)` and narrow fields
keep their width.  **Why.** The interpreter and heap are bandwidth-bound; doubling every stored
integer buys nothing a user sees (`formal/types.md` D2).

**Revisit when.** A measurement shows the compact-encoding dispatch costs more than the bandwidth
it saves on a specific hot path (then widen that path only), or a user can observe an i64 value
being clipped (then fix that path).  Decided 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c83--the-internal-representation-follows-the-user-visible-contract-never-widen-storage-for-implementation-convenience).
**Catalogue:** @F3 (scalar types — integer), @F4 (width integers).

## C84 — `server` ships as minimal TCP/WS primitives, not a fully-featured HTTP framework

**Decision.** `server` ships HTTP `listen` / `next` / `respond*`, single-client WebSocket and a
multi-client event pump; an application routes with its own `match` on the request path.  No
`App`/route/middleware/auth/TLS/session framework is on any roadmap.  **Why.** Every consumer
needed only "answer a request" and "run a WebSocket"; auth, certificate automation and rate
limiting are each a library-sized problem to solve when a consumer needs one.

**Revisit when.** A real consumer hits a wall that primitives plus `match` cannot clear; then add
the one piece it needs (auth, TLS or static serving), not the framework.  Decided 2026-07-02 — [record](DESIGN_DECISIONS-history.md#c84--server-ships-as-minimal-tcpws-primitives-not-a-fully-featured-http-framework).
**Catalogue:** the `server` library (`loft-lang/loft-libs-net`).

## C85 — Overflow arithmetic types NON-null; the game keeps running (don't force `integer?` on every `*`/`+`/`-`)

**Decision.** `a*b`, `a+b` and `a-b` on `integer` type non-null; an overflow yields the null
sentinel at run time and execution continues — no trap, no wrap.  The reachable-fault operations
(`/`, `%`, `v[i]`, `s[i]`, text→integer parse) stay `τ?`.  A value equal to the sentinel
(`i64::MIN`, computed or read as data) reads as null; that collision is accepted.  **Why.**
`integer?` on the ubiquitous `*`/`+`/`-` would tax every expression for a fault players never hit; a
detectable null beats a plausible wrapped value.

**Revisit when.** No trigger recorded; the wide-`integer` sentinel is part of the contract-1
freeze, and narrow declared ranges follow [C127](#c127--a-narrow-type-without--has-no-null-an-unfitting-value-takes-the-types-default-and-says-so).  Ratified 2026-07-13 — [record](DESIGN_DECISIONS-history.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--).
**Catalogue:** @F38 (arithmetic safety), @F1 (null model). Refines [C80](#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation) and the @PLN25 `(N-Div)`/`(N-Arith)` rules (formal/types.md § DN3).

## C86 — Whole-value heap binds COPY; aliasing is a last-use ELISION (the rustc rule)

**Decision.** `p = o` of a whole heap value (struct, vector, vector-typed field) is a copy, aliased
only when the source is provably dead afterwards.  A struct-typed projection (`a = vv[0]`, `w =
o.inner`) is a view, materialised (a reported copy) when its container is reshaped, re-keyed or
reassigned.  `&` is the explicit live reference; reshaping a container under a live `&` is a compile
error.  **Why.** "Every variable is its own value" is the easiest rule to carry; a link is the
compiler's invisible optimisation or the programmer's explicit `&`.

**Revisit when.** Bind copies dominate a real consumer's profile and the elision cannot be
widened — which argues for widening `ElidePlan`, never for flipping the semantics.  Decided
2026-07-03 — [record](DESIGN_DECISIONS-history.md#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule).
**Catalogue:** @F21 (references), @I60 (deps).
**Holds at:** `tests/scripts/201-bind-copies-projection-views.loft`,
`tests/scripts/774-view-outlives-reassigned-container.loft`; `formal/binding.md` `(B-Ref-Reshape)`;
[OWNERSHIP_MODEL.md § A view lasts as long as the thing it names](OWNERSHIP_MODEL.md#a-view-lasts-as-long-as-the-thing-it-names--and-loft-says-when-it-does-not).

## C87 — `#rust"..."` template path is KEPT; do NOT migrate it away to per-Op emitters (@PLN81 closed)

**Decision.** The `#rust"..."` template path stays: inline `#rust` is a public library-authoring
mechanism for BRIDGING a capability loft lacks, not stdlib-internal debt.  It is not a speed-up
for a routine loft can express — that is cured in the engine or the loft algorithm
(`(Perf-Cure)`, [formal/performance.md](formal/performance.md)).  **Why.** Deleting the template
path breaks the documented library route and makes a new operator cost a struct and a
registration instead of one line; the emission-bug class is better served by hardening the path.

**Revisit when.** Emission genuinely needs one source of truth — then fold the few hand-written
emitters INTO `#rust`, never the reverse.  Decided 2026-07-08, amended 2026-09-25 — [record](DESIGN_DECISIONS-history.md#c87--rust-template-path-is-kept-do-not-migrate-it-away-to-per-op-emitters-pln81-closed).

## C88 — the scope-exit free gate stays dep-derived; simplify it (if ever) by promoting @PLN94's ownership oracle to authority, NOT by @PLN79's "drop the gate half + rely on idempotent free" (@PLN79 closed)

**Decision.** The scope-exit `OpFreeRef` gate in `src/scopes.rs` keeps its dep-derived half;
"emit more frees and rely on a free being idempotent" is declined.  **Why.** The concern —
cleanup correctness should not hang on dep-tracker precision — is real, but the right vehicle is
one ownership query (the `ownership_of` oracle) replacing the gate's special cases, not a blunter
gate.

**Revisit when.** The `ownership_of` oracle graduates from observer to authority; making it the
free-emission authority is then a fresh plan.  Decided 2026-07-09 — [record](DESIGN_DECISIONS-history.md#c88--the-scope-exit-free-gate-stays-dep-derived-simplify-it-if-ever-by-promoting-pln94s-ownership-oracle-to-authority-not-by-pln79s-drop-the-gate-half--rely-on-idempotent-free-pln79-closed).

## C89 — No tuple-style enum variants; a matcher reads like grammar and is never forced

**Decision.** Enum payloads are always NAMED fields, read as `e.field`; positional variants
(`Ok(a)`) are permanently declined.  A structural matcher (the @PLN35 PEG patterns) stays, on two
conditions: it is never the only way to read data, and its surface reads like grammar notation,
not regex.  **Why.** A wrapped payload forces a matcher just to read a value, and that grows the
whole mitigation layer (`?`, `unwrap`, `if let`, combinators); named fields and `τ?` with `??`
remove the need for it.

**Revisit when.** Never for tuple variants.  The "reads like grammar" bar applies to each PEG
syntax choice.  Decided 2026-07-10 — [record](DESIGN_DECISIONS-history.md#c89--no-tuple-style-enum-variants-a-matcher-reads-like-grammar-and-is-never-forced).

## C90 — Each nullable scalar reserves ONE bit-pattern for null (the in-band sentinel residual; accepted, frozen)

**Decision.** Each nullable scalar keeps one in-band reserved value, frozen with contract 1:
`integer` `i64::MIN`; narrow widths their top stored value (`u8?` → `255`); `boolean` `255`;
`character` codepoint `0`; `float`/`single` a reserved `NaN` (`±inf` are real values).  References
(`nullref`) and structs in a vector (`__nullable<S>`) are out-of-band.  A `τ?` is as wide as `τ`;
its reserved value cannot be stored as data.  **Why.** A tagged representation would widen every
`τ?`, un-densify `vector<τ?>` and reopen the layout to win back one extreme value per type; the
collision sites are guarded.

**Revisit when.** No trigger recorded; frozen with contract 1.  Decided 2026-07-13 — [record](DESIGN_DECISIONS-history.md#c90--each-nullable-scalar-reserves-one-bit-pattern-for-null-the-in-band-sentinel-residual-accepted-frozen).
**Catalogue:** @F1 (null / Optional), @F3 (scalar types), @F4 (width integers). Closes the @PLN102 pre-freeze [null-model keystone](plans/102-stability-contract/keystone-null-model.md) (option B); the sibling of [C85](#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--) (overflow yields the sentinel) and [C80](#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation) (the spreadsheet model).
**Holds at:** `tests/scripts/pln102-null-residual-golden.loft`.

## C91 — `==` is value-by-value / reference-by-identity (bounded, never a reference-chase); `===` reserved for opt-in deep equality

**Decision.** `==`/`!=` compare value types (scalars, `text`, `value struct`, unit enums) by
content over their own storage, and reference types (`struct`, `vector`, `hash`, `index`, …) by
identity in O(1); neither ever follows a reference into another store.  `===`/`!==` are
reserved for an explicit deep equality, which when built must be deep all the way down and
cycle-safe.  A shallow hybrid `==` is rejected.  **Why.** `==` must be cheap and describable in
one sentence; a shallow structural `==` is inconsistent in a way that depends on layout.

**Revisit when.** No trigger recorded; `===` ships when a consumer needs it.  Decided 2026-07-13 — [record](DESIGN_DECISIONS-history.md#c91---is-value-by-value--reference-by-identity-bounded-never-a-reference-chase--reserved-for-opt-in-deep-equality).
**Catalogue:** @F1 (null / equality), @F2 (operators). Closes the @PLN102 pre-freeze **F7** ("`ref ==` is identity, not structural — a decision"). Sibling of [C86](#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule) (whole-value copy) and [C90](#c90--each-nullable-scalar-reserves-one-bit-pattern-for-null-the-in-band-sentinel-residual-accepted-frozen) (the in-band sentinels).

## C92 — Compound assignment evaluates its place expression exactly once

**Decision.** `place op= rhs` (and the keyed-collection forms) evaluates the addressing
sub-expressions of `place` — index expressions, container-producing calls — exactly once, and
reads and writes through the same values.  Part of the contract-1 freeze.  **Why.** A second
evaluation of a side-effecting index reads one slot and writes another, a silent wrong answer;
C, Rust and Python all evaluate the place once.

**Revisit when.** No trigger recorded; frozen with contract 1.  Decided 2026-07-14 — [record](DESIGN_DECISIONS-history.md#c92--compound-assignment-evaluates-its-place-expression-exactly-once).
**Catalogue:** @F2 (operators). Closes the @PLN102 pre-freeze **F2** ("compound-assign double-evaluates its place"). Same class as the F4 assignment eval-order item — an evaluation count/order that would otherwise freeze as impl-defined.

## C93 — A `par` worker's captured parent state is read-only; a write to it is a compile error

**Decision.** State a `par` worker captures from its parent is read-only inside the worker; a
write to it, direct or through a call that writes an aliased argument, is a compile error at the
write.  The worker may read captured state, use its own locals and element, do host I/O and PRNG,
and return a value that is folded sequentially.  `par` is deliberately strict: a possibly benign
write is refused too.  **Why.** A data race has no defined value that a null could stand for, so
the only faithful answer is to make it unexpressible.

**Revisit when.** Never by loosening `par`; broader shared-mutation parallelism would be a new,
inherently safe construct, and none is envisioned.  Decided 2026-07-14 — [record](DESIGN_DECISIONS-history.md#c93--a-par-workers-captured-parent-state-is-read-only-a-write-to-it-is-a-compile-error).
**Catalogue:** @F2 (operators) / threading. Instances the platform rule *no runtime errors, ever* (DESIGN_DECISIONS [C80](#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation)): we do not fault at runtime — we either DISALLOW what cannot work (a compile error) or make it work in a lesser state (null). A `par` data race cannot be made to "work" as null, so it is DISALLOWED. Sibling of the sandbox host-data read-only model.

## C94 — Integer `/` truncates toward zero and `%` takes the dividend's sign; `floor_mod` is the wrap-around helper

**Decision.** Integer `/` truncates toward zero (`-7 / 2 == -3`) and `%` takes the dividend's
sign (`-7 % 2 == -1`), so `a == (a / b) * b + a % b`; division by zero is null.  The wrap-around
case is `x.floor_mod(n)`, landing in `[0, n)` for a positive `n` (`(-1).floor_mod(3) == 2`).
**Why.** `/` and `%` are a matched pair, so the choice is which `/` is least surprising; cyclic
indexing is a distinct operation, and naming it makes the wrap visible at the call site.

**Revisit when.** No trigger recorded; the operator convention is frozen with contract 1.
Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c94--integer--truncates-toward-zero-and--takes-the-dividends-sign-floor_mod-is-the-wrap-around-helper).
**Catalogue:** @F2 (operators) / math. Fixes the sign convention for negative integer division at contract 1 — the one place a language must *pick* (C, Rust, Go, Java, JS truncate; Python, Ruby floor). Both are legitimate; the freeze needs one named default.

## C95 — A definition a same-named method would silently shadow is a compile error (no silent redefinition)

**Decision.** A free function `foo(x: T, …)` is a compile error at the definition when a method
`foo` for first-argument type `T` exists — calls would dispatch to the method and the function could
never run.  The test is exact on the first-argument type, so sharing a name with a method on another
receiver stays legal.  Frozen with contract 1.  **Why.** An uncallable definition is a latent bug;
letting the user's definition win would let a program silently re-point a stdlib call.

**Revisit when.** No trigger recorded; the predicate follows the mangling if it ever keys on more
than the first parameter.  Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c95--a-definition-a-same-named-method-would-silently-shadow-is-a-compile-error-no-silent-redefinition).
**Catalogue:** @F2 (operators) / naming. Instances the platform rule *no runtime errors, ever* ([C80](#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation)) at its **compile-time valve**: what cannot work is *disallowed*, not run into a silent-wrong result. Surfaced by C94 (adding stdlib `floor_mod` collided with an existing library helper).
**Holds at:** `src/data.rs::add_fn`.

## C96 — Library shipping is keyed on trust-root presence: a key-present machine ships autonomously, a key-absent one defers

**Decision.** A machine holding the trust-root key signs and ships (validate, package, sign artifact
and index, append, push); one without it validates and defers through a submission PR to
`submissions/`, which a key-holding ship pass folds in and signs.  Each artifact is signed alone,
the index is append-only and written with its `.sig`, and shipping is single-writer with a
compare-and-swap push.  `scripts/registry-sign.sh` signs with a card when a PKCS#11 module is
present, else with the key file after a typed `yes` or under `--expect <pkg>@<ver>`.  **Why.** The
key must never enter CI, and one human signing every release would bottleneck shipping.

**Revisit when.** Unattended CI publishing becomes a real need (a scoped, delegated CI key).
Decided 2026-07-15, amended 2026-09-25 — [record](DESIGN_DECISIONS-history.md#c96--library-shipping-is-keyed-on-trust-root-presence-a-key-present-machine-ships-autonomously-a-key-absent-one-defers).
**Catalogue:** @F/registry (publishing). Fixes the trust model of the file-based registry ([PKG_REGISTRY.md](PKG_REGISTRY.md), [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md)) at contract 1: the reliable surface (a signed, immutable, append-only index) may only grow, and shipping into it must not bottleneck on one human.

## C97 — A library's public symbols live under its module, not the global namespace (so the stdlib can grow without breaking a shipped lib)

**Decision.** A library's public symbols are `lib::name`, registered scoped to the library's source;
the stdlib is the only global namespace.  A library may define a free function with a stdlib name
(C95 guards a program's own definitions); a library `self:`/`both:` METHOD named like a stdlib
method still errors — accepted.  **Why.** Global library symbols let any stdlib addition break a
published, immutable library, which absolute compatibility forbids.

**Revisit when.** No trigger recorded; frozen with contract 1.  Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c97--a-librarys-public-symbols-live-under-its-module-not-the-global-namespace-so-the-stdlib-can-grow-without-breaking-a-shipped-lib).
**Catalogue:** @F2 (operators) / modules + naming. A contract-1 compat decision — the precondition for a stdlib that is *both* absolute-compat *and* still growable.
**Holds at:** `src/data.rs::add_fn`; `tests/imports.rs` `pln102_c97_library_may_define_a_stdlib_name`;
`.github/workflows/revalidate-libs.yml`.

## C98 — `use lib;` binds only the `lib` namespace; unqualified access is an EXPLICIT `use lib::*` / `use lib::(…)`, where the imported name wins

**Decision.** The ruling: `use lib;` binds only the name `lib`; `use lib::*;` and `use lib::(a, b)`
bind names unqualified, an explicitly imported name wins over a same-spelled stdlib name, and `as`
aliases (`use lib::(a as x)`, `use lib as el`) resolve any clash.  **Why.** A qualifying program
cannot collide with stdlib growth, and an explicit import keeps its binding forever, so no program
breaks and no ambiguity error is needed.  **Not what ships:** a bare `use lib;` still
wildcard-imports every `pub` name ([LOFT.md](LOFT.md)); whether the code or the ruling moves is an
open owner call.

**Revisit when.** That owner call is made; an imported name against the program's own top-level
definition is also unruled.  Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c98--use-lib-binds-only-the-lib-namespace-unqualified-access-is-an-explicit-use-lib--use-lib-where-the-imported-name-wins).
**Catalogue:** @F2 (operators) / modules + naming. The name-resolution HOW that [C97](#c97--a-librarys-public-symbols-live-under-its-module-not-the-global-namespace-so-the-stdlib-can-grow-without-breaking-a-shipped-lib) deferred — the rule that makes "the stdlib can grow without breaking a program" actually hold.

## C99 — A keyed collection's subscript is uniformly KEY-addressed (lookup / range / removal), never positional

**Decision.** On every keyed collection (`sorted`, `index`, `hash`, `spatial`) `coll[k]` is a
key lookup, `coll[k] = null` a key removal, and on the ordered ones `coll[lo..hi]` a key-range or
proximity query; none is positional.  **Why.** The single subscript is already key-addressed,
so rejecting only the range form would make the surface less consistent and break `spatial`; the
likeness to a vector slice is inherent to keyed collections and is documented as a gotcha.

**Revisit when.** The owner prefers rejecting the range slice for a `.range()` call — which would
also have to re-home `s[key]` and exempt `spatial`.  Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c99--a-keyed-collections-subscript-is-uniformly-key-addressed-lookup--range--removal-never-positional).
**Catalogue:** @F8 (sorted) / @PLN102 arc-E lib-audit **H8** (INC#2). The freeze-time resolution of "the sorted key-range slice shares vector's positional-slice syntax."
**Holds at:** `tests/expressions.rs` `sorted_range_iteration`,
`sorted_subscript_is_key_addressed_not_positional`; [INCONSISTENCIES.md](INCONSISTENCIES.md) #2.

## C100 — `print` stays text-only (no bare `print(value)` or variadic `print`)

**Decision.** `print`/`println` take `text` only; any value, or several, print through the format
string (`print("{x}")`, `print("{a} {b}")`).  No bare `print(value)`, no variadic `print`.
**Why.** The capability already ships; the three-character sugar would force `print` to become an
overloaded method, breaking REPL completion and the poisoned-argument error recovery, and a
variadic `print` hides its separator where the format string writes it.

**Revisit when.** The owner wants the bare call enough to accept a print-specific argument
coercion (auto-insert `.to_text()` in the call checker); variadic `print` would also need the
no-variadics stance reversed.  Decided 2026-07-20 — [record](DESIGN_DECISIONS-history.md#c100--print-stays-text-only-no-bare-printvalue-or-variadic-print).
**Catalogue:** @PLN13 (beginner-friendly scripts — step 5).

## C101 — `std`/`core` are reserved package names; `std::name` is the stdlib's qualified form (the shadow escape hatch)

**Decision.** `std::name` is the stdlib's permanent qualified form and reaches a stdlib symbol
that a user definition or an import shadows; it is never required.  `std` and `core` are
reserved package names, refused by `loft new` and by the registry.  **Why.** The resolution rule
freezes with contract 1, so the escape hatch and the names that must never be a library are
fixed now, while no published package uses them.

**Revisit when.** No trigger recorded; frozen with contract 1.  Decided 2026-07-24 — [record](DESIGN_DECISIONS-history.md#c101--stdcore-are-reserved-package-names-stdname-is-the-stdlibs-qualified-form-the-shadow-escape-hatch).
**Catalogue:** @F2 (operators) / modules + naming; completes the C97/C98 namespace model.
**Holds at:** `src/libscan.rs` `RESERVED_PACKAGE_NAMES` / `is_reserved_package_name`; the guard in
`tests/imports.rs`.

## C102 — a release binary says nothing when it falls back to the interpreter

**Decision.** A run without a usable `rustc` falls back to the interpreter silently; the
explanation prints only when native was explicitly asked for (`--native`), and stays in `--help`
and the release `QUICKSTART.md`.  Every fallback still records `native_fallback_reason`, which
`LOFT_REQUIRE_NATIVE=1` turns into a hard error.  A source checkout without `rustc` still explains
itself, because there the message is actionable.  **Why.** Someone who downloaded a release chose
not to install a toolchain, so "install Rust and rebuild" on a run that succeeds is noise, not a
warning; gating on a per-user marker file was rejected because a fresh container has no `~/.loft`.

**Revisit when.** No trigger recorded.  Decided 2026-07-24 — [record](DESIGN_DECISIONS-history.md#c102--a-release-binary-says-nothing-when-it-falls-back-to-the-interpreter).

## C103 — `int` / `str` / `bool` are suggested, never legal (no cross-language type aliases)

**Decision.** `int`, `str`, `string`, `bool`, `i64`, `u64`, `f64`, `f32`, `char`, `double` and
`long` stay undefined; the error names the loft type (`data::builtin_type_alias`, a table, since
edit distance cannot reach these).  **Why.** Full-word type names mean fewer acronyms for a
newcomer and a small nudge against annotations inference makes pointless; an alias is permanent
surface after contract 1; and the lookalikes are not identities (`integer` excludes `i64::MIN`, the
null sentinel), which a suggestion tolerates and an alias would make silent.

**Revisit when.** Evidence that the suggestion is not landing — people hitting it repeatedly, not
once.  Decided 2026-07-24 — [record](DESIGN_DECISIONS-history.md#c103--int--str--bool-are-suggested-never-legal-no-cross-language-type-aliases).
**Holds at:** `tests/parse_errors.rs::t03_cross_language_type_aliases_suggest`.

## C104 — a slice on a lazily-bound collection never fetches; a range is an explicit call

**Decision.** A slice on a lazily-bound collection reads the resident set like any read; a range
fetch is the explicit `store_lazy_range(c, lo, hi)`, an ad-hoc query `store_lazy_query`.  **Why.** A
lazy collection answers "what have I got", never "what exists" ([LAZY_STORES.md](LAZY_STORES.md)); a
fetching slice would make `xs[0..10]`, `for x in xs` and `len(xs)` disagree.

**Revisit when.** A consumer shows the explicit call is a real burden AND the honesty question has
an answer (e.g. a streaming collection whose `len` and iteration mean something else by
declaration).  Decided 2026-08-06 — [record](DESIGN_DECISIONS-history.md#c104--a-slice-on-a-lazily-bound-collection-never-fetches-a-range-is-an-explicit-call).
**Holds at:** `tests/lazy_sql_source.rs`.
**Catalogue:** @F108 (lazy store binding)

## C105 — a hash lookup keeps its TWO random reads (no hash-in-slot, no entries in the bucket table)

**Decision.** A hash lookup reads the bucket slot, then the entry; both stay random reads.  A hash
cached in the slot and entries in the bucket table are both declined; `reserve(h, n)` answers a fill
of known size.  **Why.** The cache saves at most ~12 % of a grown insert (none with `reserve`) for a
doubled table and a persisted-format break; in-table entries move on growth and make outstanding
`DbRef`s read wrong data ([COMPATIBILITY.md](COMPATIBILITY.md)).

**Revisit when.** A like-for-like measurement (same access order, same working set on both sides)
still shows a large gap; or reference stability into a keyed collection is given up; or the bucket
table's width changes for another reason.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c105--a-hash-lookup-keeps-its-two-random-reads-no-hash-in-slot-no-entries-in-the-bucket-table).
**Catalogue:** @F7 (`hash<T[keys]>` keyed collection).

## C106 — `#c` has ONE arity ceiling for both backends, and it was raised rather than lowered

**Decision.** `MAX_C_ARITY = 32` C slots on both backends.  A declaration is checked in code you own
(stdlib or entry project), a call site everywhere, so an unused over-ceiling binding in a dependency
does not fail your build; past 32, write an ANSI-C shim.  The diagnostic keeps its code
`c-binding-not-interpretable`.  **Why.** A backend-specific ceiling is a portability trap that fires
downstream, and extending the trampoline ladder was mechanical.  Arity is refused at the declaration
(unlike `#c` on wasm, at the call) because an over-ceiling binding works on no target.

**Revisit when.** A real C API needs more than 32 integer-class slots; the ceiling then moves for
both backends at once.  Float-by-value does not reopen it.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c106--c-has-one-arity-ceiling-for-both-backends-and-it-was-raised-rather-than-lowered).
**Catalogue:** @F92 (direct C binding), @F53 (native backend)

## C107 — the C signature decides whether a `vector` carries a count, not the loft type

**Decision.** A `vector` parameter crosses `#c` as a bare element pointer where the C signature has
no integer for it, else as pointer-then-count.  `c_signature::plan` derives the assignment once
(lookahead, not greedy), read by the declaration check, the interpreter's `dispatch` and `--native`
emission.  **Why.** BLAS/LAPACK-style libraries take bare pointers and a by-reference `n`; the
assignment is provably unique when it exists.  Cost: a signature omitting a count the C function
takes is accepted, like any wrong `#c` signature.

**Revisit when.** loft grows an address-of, so a by-reference scalar no longer needs a 1-element
vector.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c107--the-c-signature-decides-whether-a-vector-carries-a-count-not-the-loft-type).
**Holds at:** `src/c_signature.rs::every_reachable_shape_has_at_most_one_reading`.
**Catalogue:** @F92 (direct C binding)

## C108 — a `vector<T>` and the C pointee are two spellings of ONE layout

**Decision.** A `vector<T>` binds to a C pointer only when the pointee has one loft element's width
and class (integer vs float); signedness is not checked.  `void *`, and any unresolved pointee, is
opaque and always accepted.  `vector<i8>` / `vector<i16>` (stored offset by their minimum) and
`vector<text>` do not cross.  One home: `CElement::of_loft` reads `data::element_storage_size`;
`c_signature::options` is the only matcher.  **Why.** The vector crosses as a pointer into loft's
own element bytes, so a width mismatch reads plausible wrong numbers or writes past the allocation;
a converting copy would hide the LP64/ILP64 question and defeat calling BLAS.

**Revisit when.** A `#c` declaration can name the library's integer model, so the ILP64 question
is asked once per package.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c108--a-vectort-and-the-c-pointee-are-two-spellings-of-one-layout).
**Catalogue:** @F92 (direct C binding)

## C109 — a float RETURN crosses `#c`; a float ARGUMENT still does not

**Decision.** A C `double` return binds to loft `float` and a C `float` return to loft `single`,
exactly (no widening).  A float argument stays refused; pass it through a pointer.  **Why.** The
return has one axis (the value is in `xmm0` or not), so it costs one more expansion of the same
trampoline arity list; an argument would need a rung per subset of positions.  Without it every
level-1 BLAS function and every LAPACK routine that answers a number needed a C shim.

**Revisit when.** Someone needs a float argument badly enough to pay for an SSE-aware ladder; it
moves for both backends at once, as C106 requires.  Decided 2026-08-10 — [record](DESIGN_DECISIONS-history.md#c109--a-float-return-crosses-c-a-float-argument-still-does-not).
**Catalogue:** @F92 (direct C binding)

## C110 — a generic's type variable stays in the FIRST parameter, and a keyed collection stays a record set

**Decision.** Superseded in part by
[C126](#c126--a-generics-type-variables-are-unrestricted-a-keyed-collection-stays-a-record-set)
(type variables in any parameter; several per generic).  Still holding: keyed collections stay
record sets keyed on a named field (no `hash<K, V>`), and `hole_*` stays per-kind.  **Why.** A
record set carries its own key, so key and value cannot desync and one model covers all five keyed
kinds; a generic `hole<T>` would drop the per-kind opt-in keeping non-literal values out of SQL.

**Revisit when.** A multi-parameter keyed type has a use a record set cannot express.  Decided
2026-08-11, revised 2026-09-21 by C126 — [record](DESIGN_DECISIONS-history.md#c110--a-generics-type-variable-stays-in-the-first-parameter-and-a-keyed-collection-stays-a-record-set).
**Catalogue:** @F26 (interfaces & bounded generics) · @F94 (type-directed interpolation)

## C111 — a drop cascade reaches a container's death, not an element's removal

**Decision.** A drop runs when the value's OWNER dies, not when the value is taken out.  Moving a
droppable into a struct field, enum payload, collection element or another variable transfers the
release to that container's death (a copy whose source stays in use takes its own lease or is
refused — C121).  A rebind releases the record it displaces.  Removing or overwriting a collection
element (`v.remove(i)`, `v[i] = x`) does NOT run its drop: close it first.  **Why.** Element removal
releases through the runtime free cascade; running user code there re-enters the interpreter from
the path the heap invariant rests on, for a drop that cannot fail or report.

**Revisit when.** A consumer churns a long-lived collection of live resources (a connection pool
that evicts, a cache of open handles).  Decided 2026-08-11, revised by C121 — [record](DESIGN_DECISIONS-history.md#c111--a-drop-cascade-reaches-a-containers-death-not-an-elements-removal).
**Holds at:** `formal/heap.md` (H-Drop), (H-Drop-Not); `scopes::copy_moves_drop_from`;
`tests/scripts/a-whole-value-copy-of-a-droppable-releases-once.loft`.
**Catalogue:** @F-drop (`OpDrop`, @PLN125 arc B) · loft#849

## C112 — a binding position mints a local whatever else carries that name; the function stays reachable as a call

**Decision.** A name at any binding position — assignment, typed local, tuple-destructuring element,
parameter, `for` variable, struct field — mints a local whatever function carries that name; the
function stays callable in the same scope (`chr = 65` beside `chr(65)`): parentheses pick the
namespace.  **Why.** Otherwise every short verb a library exports is a word its consumers may not
use.  This does not reopen C95 (a local re-points no call site); C98's own half (a bare `use lib;`
still imports every public name) remains open.

**Revisit when.** No trigger recorded.  Decided 2026-08-11 — [record](DESIGN_DECISIONS-history.md#c112--a-binding-position-mints-a-local-whatever-else-carries-that-name-the-function-stays-reachable-as-a-call).
**Holds at:** `tests/scripts/852-local-shadows-a-function-name.loft`;
`tests/imports.rs::pln102_c98_a_local_may_shadow_a_library_function`.
**Catalogue:** @F2 (operators) / modules + naming · [C98](#c98--use-lib-binds-only-the-lib-namespace-unqualified-access-is-an-explicit-use-lib--use-lib-where-the-imported-name-wins)'s consumer-facing half · loft#852, loft#756

## C113 — two `index` collections over one element type and key are refused, not made to work

**Decision.** A second `index` field with the same element type and key in one struct (or enum
variant) is a compile error at the declaration (`Parser::reject_duplicate_index`), naming the cures:
another kind (`hash` or `sorted`) over the same records, or another key; a different key, struct or
kind stays legal.  **Why.** An index keeps its tree links in the element record, allocated per index
TYPE, so two same-key fields are one tree through two roots and the first removal corrupts it.
Separate storage needs a schema id per field across four layers, and buys nothing: a second same-key
index answers what the first does.

**Revisit when.** A consumer needs two same-key indexes with genuinely different behaviour; a
filtered index would arrive as its own kind.  Decided 2026-08-14 — [record](DESIGN_DECISIONS-history.md#c113--two-index-collections-over-one-element-type-and-key-are-refused-not-made-to-work).
**Holds at:** `tests/scripts/902-duplicate-index-refused.loft` beside `902-duplicate-index-allowed.loft`.
**Catalogue:** @F9 (`index<T[keys]>`) · loft#902 · loft#843 (linked collection groups)

## C114 — a keyed collection is refused as a vector ELEMENT, not given an element form

**Decision.** `vector<hash<…>>`, and the same with `sorted`, `index`, `spatial` and `trie`, is a
compile error where it is written, naming the kind; the cure is a struct holding the keyed
collection, in a `vector` of that struct.  **Why.** No program could ever fill such a vector, and
giving a keyed type an element form is a design (what the vector owns, what a copy does to the
spines), not a missing emitter arm — the cheap arm would shift every runtime schema id after it.

**Revisit when.** A consumer shows the struct wrapper's extra record is measurably in the way; that
arrives with ownership rules, as an inline element region.  Decided 2026-08-15 — [record](DESIGN_DECISIONS-history.md#c114--a-keyed-collection-is-refused-as-a-vector-element-not-given-an-element-form).
**Holds at:** `Parser::refuse_keyed_vector_element` (`src/parser/vectors.rs`);
`tests/parse_errors.rs::every_keyed_kind_is_refused_as_a_vector_element`.

## C115 — a closure cannot write through a captured `&` scalar parameter, and cannot rebind a captured heap parameter

**Decision.** A write to a `&` scalar parameter from inside a closure, and a whole-value replace
(`p = …`) of a captured heap parameter, are compile errors.  A captured local, a `&` parameter
that is only read, and every mutation THROUGH a captured heap parameter (`p += [x]`, `p[i] = v`,
a field write) still work.  **Why.** A closure holds a copy of the value, not the binding, so
neither write can reach the place it names; both compiled to a silently wrong answer.

**Revisit when.** A capture mechanism that carries the BINDING exists.  Decided 2026-09-02
(loft#1276, loft#1281) — [record](DESIGN_DECISIONS-history.md#c115--a-closure-cannot-write-through-a-captured--scalar-parameter-and-cannot-rebind-a-captured-heap-parameter).
Holds at `(L-CapRef)`, [formal/closures.md](formal/closures.md).

## C116 — a collection element holds a plain fn-ref, never a capturing closure

**Decision.** A `vector` or keyed collection over a fn type refuses a capturing closure, at the
literal and at an element assignment.  Per-element state goes in a struct FIELD, which may hold
one: `Stepper { advance: fn(x: integer) -> integer { x + step } }`.  **Why.** A collection has one
element layout and a closure's environment differs per lambda; carrying it would widen every
fn-typed collection and give the collection a second ownership discipline.  A struct field
co-locates the record (C75).

**Revisit when.** Not stated.  Decided 2026-09-04 (loft#1358) —
[record](DESIGN_DECISIONS-history.md#c116--a-collection-element-holds-a-plain-fn-ref-never-a-capturing-closure).
Holds at `Parser::refuse_capturing_closure_in_collection`.

## C117 — a linked group's members must share one element LAYOUT; the tag-aware dense read is declined

**Decision.** A struct declaring both a dense `vector<E>` and a `vector<E?>` beside a keyed member
is refused; the message names the cures (`vector<E?>` for both, or drop the keyed member).
**Why.** One record set that may hold absence cannot be read through a non-null element type:
left alone the dense member silently leaves its group, and made to join it misreads the tag as
data.  A tag-aware dense read would give `vector<E>` a layout its author did not write
(`(N-Dense)`).

**Revisit when.** Not stated; declined, not deferred.  Decided 2026-09-06 (loft#1385) —
[record](DESIGN_DECISIONS-history.md#c117--a-linked-groups-members-must-share-one-element-layout-the-tag-aware-dense-read-is-declined).
Holds at `(Col-Group)`, [formal/collections.md](formal/collections.md).

## C118 — an append to an ABSENT collection instantiates it empty and fills it; refusing or warning is declined

**Decision.** `c += [x]` on a nullable collection holding null creates it empty and inserts, with
no diagnostic; `for` over and `remove` from an absent collection stay refused.  **Why.** Its main
producer is decoding a document with a missing key, and the append binds no element — its only
question is which store the record joins, and an absent collection has one sensible answer.

**Revisit when.** A consumer shows the silent instantiation hid a defect — evidence for an
`advice` note, not an error.  Decided 2026-09-07 (loft#1434) —
[record](DESIGN_DECISIONS-history.md#c118--an-append-to-an-absent-collection-instantiates-it-empty-and-fills-it-refusing-or-warning-is-declined).
Holds at `(Col-Insert-Absent)`, [formal/collections.md](formal/collections.md).
**Catalogue:** @F1 (null model), @F38.

## C119 — a tuple type is never nullable; a tuple that arrives absent is a present tuple of null members

**Decision.** A written `(τ, τ)?` is refused, naming its cures.  Wherever absence is synthesised
for a tuple, the answer is the present tuple of null members:
`optional((τ₁, …, τₙ)) ≡ (τ₁?, …, τₙ?)`.  **Why.** A tagged tuple layout would be re-asserted by
every tuple function and emitter, each a place to misread bytes silently; a tuple has no faithful
document form, so "every member null" is the natural reading of absence.

**Revisit when.** A consumer shows "absent tuple" and "tuple of nulls" carry different information.
Decided 2026-09-07 (loft#1423, loft#1451) — [record](DESIGN_DECISIONS-history.md#c119--a-tuple-type-is-never-nullable-a-tuple-that-arrives-absent-is-a-present-tuple-of-null-members).
Holds at `(T-Absent)`, [formal/tuples.md](formal/tuples.md).
**Catalogue:** @F1 (null model), @F (tuples).

## C120 — Integer arithmetic on native stays sentinel-aware after a fault; the non-null proof does not close over `+`, `-`, `*`

**Decision.** `+`, `-` or `*` over non-null operands is not proven non-null, so after an overflow
native carries null exactly as the interpreter does.  A plain operator is emitted only where a fact
established BEFORE the arithmetic rules out a fault (`(R-Range)`, `(R-GuardedChain)`,
`(R-BoundedNest)`).  A published library always keeps the checks; a check-free release pass for a
game with fault-free checked runs may come, never as a default.  **Why.** A rewrite must leave every
observable value as the interpreter answers it, faults included.

**Revisit when.** Nothing reopens it; the successor is a range proof.  Decided 2026-09-15 —
[record](DESIGN_DECISIONS-history.md#c120--integer-arithmetic-on-native-stays-sentinel-aware-after-a-fault-the-non-null-proof-does-not-close-over----).
Holds at [NATIVE.md § Optimisation tiers](NATIVE.md#optimisation-tiers--semantics-runs-performance-lanes-shipped-binaries).

## C121 — a copy of a droppable takes its own lease or is refused; the release no longer moves with a copy

**Decision.** A copy of a type with `OpDrop` and no `OpCopy` is refused on the line writing it (`x =
a`, `S { h: a }`, `return s.h`, `return p`); `OpCopy` makes a second lease.  Only written positions
move: a fresh value placed where made, a `return` of what the function owns, a block yielding its
own variable.  **Why.** A release moving with a copy leaves a structure without its resource or
releases twice; inferring moves from later use lets a later line redefine an earlier one.

**Revisit when.** A consumer needs a droppable OUT of a container it keeps using — that wants a
named `take`, not an implicit move.  Decided 2026-09-15 —
[record](DESIGN_DECISIONS-history.md#c121--a-copy-of-a-droppable-takes-its-own-lease-or-is-refused-the-release-no-longer-moves-with-a-copy).
Holds at `(H-Copy-Lease)`, `(H-Copy-Refuse)`, [formal/heap.md](formal/heap.md).
**Catalogue:** @F-drop (`OpDrop`) · @PLN163 · revises C111's whole-value extension

## C122 — The contract is semantics; a rewrite is free wherever its conditions are validated, and a library API is the one boundary

**Decision.** The contract is what a program computes and can observe — values, faults, effect order
— never representation or copy count.  A rewrite needs only its conditions, validated on the IR, and
declines otherwise.  A construction escaping the unit keeps the promised representation; which API
is a boundary is a BUILD fact (`--native-release` compiles its loft libraries into the unit; a
`#rust` native, live reload, a stored layout and a placed library stay boundaries).  **Why.** What a
program cannot observe is the compiler's; what it can is the language's (C120, other side).

**Revisit when.** Not stated.  Decided 2026-09-15 —
[record](DESIGN_DECISIONS-history.md#c122--the-contract-is-semantics-a-rewrite-is-free-wherever-its-conditions-are-validated-and-a-library-api-is-the-one-boundary).
Holds at `(R-Escape)`, [formal/rewrites.md](formal/rewrites.md).

## C123 — one name has one body per receiver type; `both` is how a function takes both spellings

**Decision.** A method and a plain-parameter function of one name whose first parameter has the
same type are refused at the second declaration, in either order and at any arity; a function on
another type is an overload.  `self` and `both` now mean the same, so `both` is deprecated
(warning `both-receiver-deprecated`).  **Why.** When `x.doit()` and `doit(x)` can differ, reading
a line requires knowing how loft resolves it.

**Revisit when.** Never for two behaviours under one name on one type — the cure is a second name.
Remove `both` once no published version declares it.  Decided 2026-09-15 —
[record](DESIGN_DECISIONS-history.md#c123--one-name-has-one-body-per-receiver-type-both-is-how-a-function-takes-both-spellings).
**Catalogue:** @F16 (method and function calls) · INC#8 · `formal/calls.md` (F-OneBody)

## C124 — a `const` value reaches only a `const` parameter; semantics is judged by the line, optimisation by the proof

**Decision.** A value-const value may reach a record or collection parameter only if it is declared
`const` (including `fn(const T)` and builtin callbacks), and a `&` parameter never; judged by the
SIGNATURE, never the callee's body.  The plain case is the gating warning
`const-to-plain-parameter` until the shipped libraries mark their read-only parameters, then an
error.  **Why.** What a line means is decided by what is written on it and the signatures it
names (C121); that a body does not write is a proof an optimisation may use, not a license.

**Revisit when.** A read-only helper cannot be declared `const` — a gap in `const`'s syntax to
close.  Decided 2026-09-15 — [record](DESIGN_DECISIONS-history.md#c124--a-const-value-reaches-only-a-const-parameter-semantics-is-judged-by-the-line-optimisation-by-the-proof).
**Catalogue:** @PLN40 const-model rule 4 · `formal/binding.md` (Const-Value), D-bind-45 (closed) · loft#1540 · reads C121 and C122

## C125 — The store model stays simple: performance work removes objects, it does not add a second kind of object

**Decision.** A store is one object, identity is the store, a free releases a store.  An
optimisation may not add a second object kind (an arena record with pair identity); it removes
the temporary instead — `(R-ValueRecord)`, `(O-ViewField)`, `(R-Place)`, `(R-InPlaceLiteral)`,
`(R-ElemFirst)` — or reuses a store per call site.  **Why.** A model that grows a second object
kind becomes hard to reason about and so impossible to verify.

**Revisit when.** Temporaries that cannot be removed or reused cost more than a few percent,
measured on the release binary with `perf`.  Decided 2026-09-17 —
[record](DESIGN_DECISIONS-history.md#c125--the-store-model-stays-simple-performance-work-removes-objects-it-does-not-add-a-second-kind-of-object).
**Catalogue:** @PLN164 (Open design question E1, tier 1 / A1–A2) · reads C122 · `formal/ownership.md` `(O-Buffer)`

## C126 — a generic's type variables are unrestricted; a keyed collection stays a record set

**Decision.** A type variable may appear in any parameter, and a function, struct or enum may
declare several; one that appears in no parameter or field is refused.  Kept from C110: keyed
collections stay record sets (no `hash<K, V>`), and the SQL `hole_*` family stays per-kind.
**Why.** "One variable, first parameter only" is a restriction no reader can derive, and `map` /
`reduce` need a second variable to stop being special forms.

**Revisit when.** A keyed type has a use a record set cannot express, or a target wants an OPEN
set of hole kinds.  Decided 2026-09-21 —
[record](DESIGN_DECISIONS-history.md#c126--a-generics-type-variables-are-unrestricted-a-keyed-collection-stays-a-record-set).
**Catalogue:** @F25 (generics) · @F26 (bounded generics) · @F94 · revises C110.

## C127 — A narrow type without `?` has no null: an unfitting value takes the type's DEFAULT, and says so

**Decision.** A declared narrow range without `?` (`limit(lo, hi)` and the aliases
`u8`/`i8`/`u16`/`i16`/`u32`) has no null: an unfitting value takes the type's DEFAULT — zero where
the range holds it, else the bound nearest zero — for a local, field, element, parameter and return
alike.  Where the author can be made to choose, the narrowing is refused (`?` or `?? d` cures it);
elsewhere a warning says the default was used.  Plain `integer` and `i32` keep C85's sentinel — a
PRAGMATIC exemption, not a line to extend.  **Why.** Null-on-overflow depended on whether the range
left a spare code, so two non-nullable declarations behaved oppositely.

**Revisit when.** The `i32` exemption's cost changes (a program needs the full 32-bit range); that
clause is one predicate.  Decided 2026-09-23 — [record](DESIGN_DECISIONS-history.md#c127--a-narrow-type-without--has-no-null-an-unfitting-value-takes-the-types-default-and-says-so).
**Catalogue:** @F4 (width integers), @F1 (null model). Refines [C85](#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--) at the narrow end and settles `formal/types.md` `(N-Reserve)` against `(E-Uncomp-NN)`.
**Holds at:** `IntegerSpec::non_null_reads_null`;
`tests/scripts/1615-every-narrow-slot-answers-the-types-default-for-an-unfitting-value.loft`.

## C128 — a yielded lambda owns copies of what it captures

**Decision.** Every heap capture of a lambda a generator yields is copied at the yield into a
store the closure record owns and releases; a lambda not yielded still shares, and a `&` capture
still aliases.  Eliding the copy where unobservable is allowed.  **Why.** Sharing would let the
closure outlive the frame that owns the value; a generator hands out values, not windows into its
state, as `(G-Own)` already says for a yielded record.

**Revisit when.** Not stated.  Decided 2026-09-25 —
[record](DESIGN_DECISIONS-history.md#c128--a-yielded-lambda-owns-copies-of-what-it-captures).
**Catalogue:** loft#1676 · `formal/coroutines.md` `(G-Own)`

## C129 — No opt-in to the processor's arithmetic (no machine-dependent scope or type)

**Decision.** No declaration, scope, type or mode licenses wrapping integers or IEEE `inf`/`NaN`
as values; the sentinel checks are the semantics.  A check is retired only where the compiler
PROVES the value cannot be the sentinel.  The one opening kept is C120's evidence-licensed
release tier.  **Why.** Machine-dependent arithmetic makes one program answer differently per
target, silently; a value-range proof is portable by construction.

**Revisit when.** Never for the machine-dependent form.  Decided 2026-09-08, amended 2026-09-15
(renumbered from a duplicate C67) — [record](DESIGN_DECISIONS-history.md#c129--no-opt-in-to-the-processors-arithmetic-no-machine-dependent-scope-or-type).
**Catalogue:** @F38 (arithmetic safety).
