<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design decisions — binding, ownership and closures

The entries of the [decision register](DESIGN_DECISIONS.md) about what a binding holds, who owns a heap value, when it is freed, and what a closure may capture.  Each states
the decision, its reason and what would reopen it; its **record** link opens the question, the
trade-offs and the evidence in [DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md).

---

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

## C75 — A closure-carrying struct owns what its closures adopted

**Decision.** A struct whose fn field holds a capturing closure may leave the frame that built
it — returned, or written into a struct received as an argument.  The closure record is built
in the struct's own store and adopts its captures there, and the struct's drop cascade runs the
record's, so the struct owns that release like any other member's.  A copy of such a struct is
judged by `(H-Copy-Refuse)` and a move by `(H-Spent)`, as for any type that owns a release.  A
collection of such structs stays refused (C116), and so does placing one into another struct's
field as a value (loft#1877, D-clo-49).  **Why.** `(L-Escape)` makes a closure an ordinary value;
building the record where the struct lives makes the transfer the ordinary ownership of a
member, with every release emitted code (CODEGEN_METHOD.md § Ownership and copy semantics are
emitted code).

**Revisit when.** loft#1877 settles how an overwritten field releases a displaced closure's
captures.  Decided 2026-06-10, revised 2026-10-05 (loft#1867) — [record](DESIGN_DECISIONS-history.md#c75--closure-carrying-struct-values-are-frame-bound).
**Holds at:** `OpChildRec` (`Parser::emit_fn_ref_field_write`), `Parser::cascade_fn_fields`,
`capture_adoption::claimed_into_delivered`;
`tests/scripts/1867-a-returned-struct-keeps-its-closure-capture.loft`, `tests/issues.rs::issue_1867_*`.
**Catalogue:** @F22 (closures & lambdas).

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
**Holds at:** `./scripts/idx tag:@C86`; `formal/binding.md` `(B-Ref-Reshape)`; [OWNERSHIP_MODEL.md § A view lasts as long as the thing it names](OWNERSHIP_MODEL.md#a-view-lasts-as-long-as-the-thing-it-names--and-loft-says-when-it-does-not).

## C88 — the scope-exit free gate stays dep-derived; simplify it (if ever) by promoting @PLN94's ownership oracle to authority, NOT by @PLN79's "drop the gate half + rely on idempotent free" (@PLN79 closed)

**Decision.** The scope-exit `OpFreeRef` gate in `src/scopes.rs` keeps its dep-derived half; "emit
more frees and rely on a free being idempotent" is declined.  **Why.** The concern — cleanup
correctness should not hang on dep-tracker precision — is real, but the right vehicle is one
ownership query (the `ownership_of` oracle) replacing the gate's special cases, not a blunter gate.

**Revisit when.** The `ownership_of` oracle graduates from observer to authority; making it the
free-emission authority is then a fresh plan.  Decided 2026-07-09 — [record](DESIGN_DECISIONS-history.md#c88--the-scope-exit-free-gate-stays-dep-derived-simplify-it-if-ever-by-promoting-pln94s-ownership-oracle-to-authority-not-by-pln79s-drop-the-gate-half--rely-on-idempotent-free-pln79-closed).

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

**Revisit when.** Not stated.  Decided 2026-09-04 (loft#1358) — [record](DESIGN_DECISIONS-history.md#c116--a-collection-element-holds-a-plain-fn-ref-never-a-capturing-closure).
Holds at `Parser::refuse_capturing_closure_in_collection`.

## C121 — a copy of a droppable takes its own lease or is refused; the release no longer moves with a copy

**Decision.** A copy of a type with `OpDrop` and no `OpCopy` is refused on the line writing it (`x =
a`, `S { h: a }`, `return s.h`, `return p`); `OpCopy` makes a second lease.  Only written positions
move: a fresh value placed where made, a `return` of what the function owns, a block yielding its
own variable.  **Why.** A release moving with a copy leaves a structure without its resource or
releases twice; inferring moves from later use lets a later line redefine an earlier one.

**Revisit when.** A consumer needs a droppable OUT of a container it keeps using — that wants a
named `take`, not an implicit move.  Decided 2026-09-15 — [record](DESIGN_DECISIONS-history.md#c121--a-copy-of-a-droppable-takes-its-own-lease-or-is-refused-the-release-no-longer-moves-with-a-copy).
Holds at `(H-Copy-Lease)`, `(H-Copy-Refuse)`, [formal/heap.md](formal/heap.md).
**Catalogue:** @F-drop (`OpDrop`) · @PLN163 · revises C111's whole-value extension

## C124 — a `const` value reaches only a `const` parameter; semantics is judged by the line, optimisation by the proof

**Decision.** A value-const value may reach a record or collection parameter only if it is declared
`const` (including `fn(const T)` and builtin callbacks), and a `&` parameter never; judged by the
SIGNATURE, never the callee's body.  Both are errors; the plain case, `const-to-plain-parameter`,
shipped as a warning until every shipped library had declared its read-only parameters `const`.
**Why.** What a line means is decided by what is written on it and the signatures it
names (C121); that a body does not write is a proof an optimisation may use, not a license.

**Revisit when.** A read-only helper cannot be declared `const` — a gap in `const`'s syntax to
close.  Decided 2026-09-15, an error from 2026-09-28 — [record](DESIGN_DECISIONS-history.md#c124--a-const-value-reaches-only-a-const-parameter-semantics-is-judged-by-the-line-optimisation-by-the-proof).
**Holds at:** `@C124` — `./scripts/idx tag:@C124`.
**Catalogue:** @PLN40 const-model rule 4 · `formal/binding.md` (Const-Value), D-bind-45 (closed) · loft#1540 · reads C121 and C122

## C125 — The store model stays simple: performance work removes objects, it does not add a second kind of object

**Decision.** A store is one object, identity is the store, a free releases a store.  An
optimisation may not add a second object kind (an arena record with pair identity); it removes
the temporary instead — `(R-ValueRecord)`, `(O-ViewField)`, `(R-Place)`, `(R-InPlaceLiteral)`,
`(R-ElemFirst)` — or reuses a store per call site.  **Why.** With one kind of value the programmer never
chooses between a long-lived store and a Rust-like local structure, and so cannot choose
wrong: which values live on the stack is the compiler's to prove (`(R-Apart)`), never a
decision in the program.  A model that grows a second object kind puts that choice back on
the programmer, becomes hard to reason about and so impossible to verify.

**Revisit when.** Temporaries that cannot be removed or reused cost more than a few percent,
measured on the release binary with `perf`.  Decided 2026-09-17 — [record](DESIGN_DECISIONS-history.md#c125--the-store-model-stays-simple-performance-work-removes-objects-it-does-not-add-a-second-kind-of-object).
**Holds at:** `@C125` — the rule `(O-One-Kind)` in [formal/ownership.md](formal/ownership.md); guard `layout_golden.rs::every_heap_value_is_carried_as_a_dbref`
(a heap value is one `DbRef` on both backends — a second kind needs a second representation).
**Catalogue:** @PLN164 (Open design question E1, tier 1 / A1–A2) · reads C122 · `formal/ownership.md` `(O-Buffer)`

## C128 — a yielded lambda owns copies of what it captures

**Decision.** Every heap capture of a lambda a generator yields is copied at the yield into a
store the closure record owns and releases; a lambda not yielded still shares, and a `&` capture
still aliases.  Eliding the copy where unobservable is allowed.  **Why.** Sharing would let the
closure outlive the frame that owns the value; a generator hands out values, not windows into its
state, as `(G-Own)` already says for a yielded record.

**Revisit when.** Not stated.  Decided 2026-09-25 — [record](DESIGN_DECISIONS-history.md#c128--a-yielded-lambda-owns-copies-of-what-it-captures).
**Catalogue:** loft#1676 · `formal/coroutines.md` `(G-Own)`
