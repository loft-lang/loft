<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Closed-by-Decision Register

Questions evaluated and decided, so they do not come back every session: before proposing one as a
feature, fix or plan item, read its entry — if nothing material has changed, it stands.  An entry
states the decision, its reason and what would reopen it; its **record** link opens the question,
trade-offs and evidence in [DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md).
[SUBJECTS.md](SUBJECTS.md) groups the entries by axis;
[DESIGN_DECISIONS_RULES.md](DESIGN_DECISIONS_RULES.md#using-the-register) says how an entry is read,
added and held by a guard.

Every entry, by the subject it decides.  Ids are one flat sequence across the subject files;
`./scripts/idx next-decision` names the next free one.

## Values, null and arithmetic — [DESIGN_DECISIONS_VALUES.md](DESIGN_DECISIONS_VALUES.md)

- [C54.D](DESIGN_DECISIONS_VALUES.md#c54d--rust-style-numeric-literal-suffixes) — Rust-style numeric literal suffixes
- [C64](DESIGN_DECISIONS_VALUES.md#c64--tuple-struct-ref-elements-use-move-semantics-not-copy--null) — Tuple struct-ref elements use MOVE semantics (not copy + null)
- [C65](DESIGN_DECISIONS_VALUES.md#c65--tuple-structure-value-element-type-folded-into-reference-e5--e6) — Tuple "structure value" element type folded into reference (E5 = E6)
- [C69](DESIGN_DECISIONS_VALUES.md#c69--x-on-a-non-boolean-is-a-null-test-not-logical-not) — `!x` on a non-boolean is a null test, not logical-not
- [C73](DESIGN_DECISIONS_VALUES.md#c73--boolean-is-three-state-false--true--null--is-raw-truthiness-coerces) — `boolean` is three-state (false / true / null); `==` is raw, truthiness coerces
- [C85](DESIGN_DECISIONS_VALUES.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--) — Overflow arithmetic types NON-null; the game keeps running (don't force `integer?` on every `*`/`+`/`-`)
- [C90](DESIGN_DECISIONS_VALUES.md#c90--each-nullable-scalar-reserves-one-bit-pattern-for-null-the-in-band-sentinel-residual-accepted-frozen) — Each nullable scalar reserves ONE bit-pattern for null (the in-band sentinel residual; accepted, frozen)
- [C91](DESIGN_DECISIONS_VALUES.md#c91---compares-content-for-every-type-cycles-included-a--b-asks-identity) — `==` compares content for every type, cycles included; `&a == &b` asks identity
- [C94](DESIGN_DECISIONS_VALUES.md#c94--integer--truncates-toward-zero-and--takes-the-dividends-sign-floor_mod-is-the-wrap-around-helper) — Integer `/` truncates toward zero and `%` takes the dividend's sign; `floor_mod` is the wrap-around helper
- [C103](DESIGN_DECISIONS_VALUES.md#c103--int--str--bool-are-suggested-never-legal-no-cross-language-type-aliases) — `int` / `str` / `bool` are suggested, never legal (no cross-language type aliases)
- [C119](DESIGN_DECISIONS_VALUES.md#c119--a-tuple-type-is-never-nullable-a-tuple-that-arrives-absent-is-a-present-tuple-of-null-members) — a tuple type is never nullable; a tuple that arrives absent is a present tuple of null members
- [C120](DESIGN_DECISIONS_VALUES.md#c120--integer-arithmetic-on-native-stays-sentinel-aware-after-a-fault-the-non-null-proof-does-not-close-over----) — Integer arithmetic on native stays sentinel-aware after a fault; the non-null proof does not close over `+`, `-`, `*`
- [C127](DESIGN_DECISIONS_VALUES.md#c127--a-narrow-type-without--has-no-null-an-unfitting-value-takes-the-types-default-and-says-so) — A narrow type without `?` has no null: an unfitting value takes the type's DEFAULT, and says so
- [C129](DESIGN_DECISIONS_VALUES.md#c129--no-opt-in-to-the-processors-arithmetic-no-machine-dependent-scope-or-type) — No opt-in to the processor's arithmetic (no machine-dependent scope or type)

## Binding, ownership and closures — [DESIGN_DECISIONS_OWNERSHIP.md](DESIGN_DECISIONS_OWNERSHIP.md)

- [C38](DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition) — Closure capture is copy-at-definition
- [C74](DESIGN_DECISIONS_OWNERSHIP.md#c74--a-mutated-scalar-may-be-captured-by-only-one-closure) — A mutated scalar may be captured by only ONE closure
- [C75](DESIGN_DECISIONS_OWNERSHIP.md#c75--closure-carrying-struct-values-are-frame-bound) — Closure-carrying struct values are frame-bound
- [C77](DESIGN_DECISIONS_OWNERSHIP.md#c77--binding-ownership-heap-aliases-by-default--binds-a-live-reference) — Binding ownership: heap aliases by default; `&` binds a live reference
- [C79](DESIGN_DECISIONS_OWNERSHIP.md#c79--ownership-is-internal-no-user-facing-borrow-checker) — Ownership is internal; no user-facing borrow checker
- [C83](DESIGN_DECISIONS_OWNERSHIP.md#c83--the-internal-representation-follows-the-user-visible-contract-never-widen-storage-for-implementation-convenience) — The internal representation follows the user-visible contract; never widen storage for implementation convenience
- [C86](DESIGN_DECISIONS_OWNERSHIP.md#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule) — Whole-value heap binds COPY; aliasing is a last-use ELISION (the rustc rule)
- [C88](DESIGN_DECISIONS_OWNERSHIP.md#c88--the-scope-exit-free-gate-stays-dep-derived-simplify-it-if-ever-by-promoting-pln94s-ownership-oracle-to-authority-not-by-pln79s-drop-the-gate-half--rely-on-idempotent-free-pln79-closed) — the scope-exit free gate stays dep-derived; simplify it (if ever) by promoting @PLN94's ownership oracle to authority, NOT by @PLN79's "drop the gate half + rely on idempotent free" (@PLN79 closed)
- [C111](DESIGN_DECISIONS_OWNERSHIP.md#c111--a-drop-cascade-reaches-a-containers-death-not-an-elements-removal) — a drop cascade reaches a container's death, not an element's removal
- [C115](DESIGN_DECISIONS_OWNERSHIP.md#c115--a-closure-cannot-write-through-a-captured--scalar-parameter-and-cannot-rebind-a-captured-heap-parameter) — a closure cannot write through a captured `&` scalar parameter, and cannot rebind a captured heap parameter
- [C116](DESIGN_DECISIONS_OWNERSHIP.md#c116--a-collection-element-holds-a-plain-fn-ref-never-a-capturing-closure) — a collection element holds a plain fn-ref, never a capturing closure
- [C121](DESIGN_DECISIONS_OWNERSHIP.md#c121--a-copy-of-a-droppable-takes-its-own-lease-or-is-refused-the-release-no-longer-moves-with-a-copy) — a copy of a droppable takes its own lease or is refused; the release no longer moves with a copy
- [C124](DESIGN_DECISIONS_OWNERSHIP.md#c124--a-const-value-reaches-only-a-const-parameter-semantics-is-judged-by-the-line-optimisation-by-the-proof) — a `const` value reaches only a `const` parameter; semantics is judged by the line, optimisation by the proof
- [C125](DESIGN_DECISIONS_OWNERSHIP.md#c125--the-store-model-stays-simple-performance-work-removes-objects-it-does-not-add-a-second-kind-of-object) — The store model stays simple: performance work removes objects, it does not add a second kind of object
- [C128](DESIGN_DECISIONS_OWNERSHIP.md#c128--a-yielded-lambda-owns-copies-of-what-it-captures) — a yielded lambda owns copies of what it captures

## Syntax, names and calls — [DESIGN_DECISIONS_SYNTAX.md](DESIGN_DECISIONS_SYNTAX.md)
- [C132](DESIGN_DECISIONS_SYNTAX.md#c132--operators-are-defined-by-the-stdlib-only-a-user-fn-op-is-an-ordinary-function) — operators are defined by the stdlib only; a user `fn Op…` is an ordinary function

- [C62](DESIGN_DECISIONS_SYNTAX.md#c62--no-type-annotations-in-x-shorthand-lambdas) — No type annotations in `|x|` shorthand lambdas
- [C63](DESIGN_DECISIONS_SYNTAX.md#c63--no-nested-fn-definitions-inside-fn-bodies) — No nested `fn` definitions inside fn bodies
- [C76](DESIGN_DECISIONS_SYNTAX.md#c76--selective-imports-group-with--not-rust-style--flat-comma-list-dropped) — Selective imports group with `()`, not Rust-style `{}`; flat comma list dropped
- [C81](DESIGN_DECISIONS_SYNTAX.md#c81---stays-one-token-disambiguated-by-position-bitwise-and-vs-reference) — `&` stays one token, disambiguated by position (bitwise-and vs reference)
- [C82](DESIGN_DECISIONS_SYNTAX.md#c82--lofts-surface-is-deliberately-not-context-free) — loft's surface is deliberately not context-free
- [C89](DESIGN_DECISIONS_SYNTAX.md#c89--no-tuple-style-enum-variants-a-matcher-reads-like-grammar-and-is-never-forced) — No tuple-style enum variants; a matcher reads like grammar and is never forced
- [C92](DESIGN_DECISIONS_SYNTAX.md#c92--compound-assignment-evaluates-its-place-expression-exactly-once) — Compound assignment evaluates its place expression exactly once
- [C95](DESIGN_DECISIONS_SYNTAX.md#c95--a-definition-a-same-named-method-would-silently-shadow-is-a-compile-error-no-silent-redefinition) — A definition a same-named method would silently shadow is a compile error (no silent redefinition)
- [C97](DESIGN_DECISIONS_SYNTAX.md#c97--a-librarys-public-symbols-live-under-its-module-not-the-global-namespace-so-the-stdlib-can-grow-without-breaking-a-shipped-lib) — A library's public symbols live under its module, not the global namespace (so the stdlib can grow without breaking a shipped lib)
- [C98](DESIGN_DECISIONS_SYNTAX.md#c98--use-lib-binds-only-the-lib-namespace-unqualified-access-is-an-explicit-use-lib--use-lib-where-the-imported-name-wins) — `use lib;` binds only the `lib` namespace; unqualified access is an EXPLICIT `use lib::*` / `use lib::(…)`, where the imported name wins
- [C100](DESIGN_DECISIONS_SYNTAX.md#c100--print-stays-text-only-no-bare-printvalue-or-variadic-print) — `print` stays text-only (no bare `print(value)` or variadic `print`)
- [C101](DESIGN_DECISIONS_SYNTAX.md#c101--stdcore-are-reserved-package-names-stdname-is-the-stdlibs-qualified-form-the-shadow-escape-hatch) — `std`/`core` are reserved package names; `std::name` is the stdlib's qualified form (the shadow escape hatch)
- [C110](DESIGN_DECISIONS_SYNTAX.md#c110--a-generics-type-variable-stays-in-the-first-parameter-and-a-keyed-collection-stays-a-record-set) — a generic's type variable stays in the FIRST parameter, and a keyed collection stays a record set
- [C112](DESIGN_DECISIONS_SYNTAX.md#c112--a-binding-position-mints-a-local-whatever-else-carries-that-name-the-function-stays-reachable-as-a-call) — a binding position mints a local whatever else carries that name; the function stays reachable as a call
- [C123](DESIGN_DECISIONS_SYNTAX.md#c123--one-name-has-one-body-per-receiver-type-both-is-how-a-function-takes-both-spellings) — one name has one body per receiver type; `both` is how a function takes both spellings
- [C126](DESIGN_DECISIONS_SYNTAX.md#c126--a-generics-type-variables-are-unrestricted-a-keyed-collection-stays-a-record-set) — a generic's type variables are unrestricted; a keyed collection stays a record set

## Collections — [DESIGN_DECISIONS_COLLECTIONS.md](DESIGN_DECISIONS_COLLECTIONS.md)

- [C68](DESIGN_DECISIONS_COLLECTIONS.md#c68--keyed-collections-dedup-on-insert--and-collkeyvalue) — Keyed collections dedup on insert (`+=` AND `coll[key]=value`)
- [C99](DESIGN_DECISIONS_COLLECTIONS.md#c99--a-keyed-collections-subscript-is-uniformly-key-addressed-lookup--range--removal-never-positional) — A keyed collection's subscript is uniformly KEY-addressed (lookup / range / removal), never positional
- [C104](DESIGN_DECISIONS_COLLECTIONS.md#c104--a-slice-on-a-lazily-bound-collection-never-fetches-a-range-is-an-explicit-call) — a slice on a lazily-bound collection never fetches; a range is an explicit call
- [C105](DESIGN_DECISIONS_COLLECTIONS.md#c105--a-hash-lookup-keeps-its-two-random-reads-no-hash-in-slot-no-entries-in-the-bucket-table) — a hash lookup keeps its TWO random reads (no hash-in-slot, no entries in the bucket table)
- [C113](DESIGN_DECISIONS_COLLECTIONS.md#c113--two-index-collections-over-one-element-type-and-key-are-refused-not-made-to-work) — two `index` collections over one element type and key are refused, not made to work
- [C114](DESIGN_DECISIONS_COLLECTIONS.md#c114--a-keyed-collection-is-refused-as-a-vector-element-not-given-an-element-form) — a keyed collection is refused as a vector ELEMENT, not given an element form
- [C117](DESIGN_DECISIONS_COLLECTIONS.md#c117--a-linked-groups-members-must-share-one-element-layout-the-tag-aware-dense-read-is-declined) — a linked group's members must share one element LAYOUT; the tag-aware dense read is declined
- [C118](DESIGN_DECISIONS_COLLECTIONS.md#c118--an-append-to-an-absent-collection-instantiates-it-empty-and-fills-it-refusing-or-warning-is-declined) — an append to an ABSENT collection instantiates it empty and fills it; refusing or warning is declined

## Failure — [DESIGN_DECISIONS_FAILURE.md](DESIGN_DECISIONS_FAILURE.md)

- [C66](DESIGN_DECISIONS_FAILURE.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt) — Production loft programs never abort on user-attributable edge cases (development may halt)
- [C67](DESIGN_DECISIONS_FAILURE.md#c67--fail-at-startup-not-at-runtime-no-programmer-side-trycatch-for-internal-bugs) — Fail at startup, not at runtime (no programmer-side try/catch for internal bugs)
- [C80](DESIGN_DECISIONS_FAILURE.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation) — The spreadsheet fault model: nothing stops a running calculation
- [C130](DESIGN_DECISIONS_FAILURE.md#c130--a-store-carries-its-own-failure-arm-else-after-the-assignment) — A store carries its own failure arm: else after the assignment
- [C131](DESIGN_DECISIONS_FAILURE.md#c131--a-cast-to-a-variant-answers-the-variant-a-provable-miss-is-refused-an-unproven-one-defaults-and-warns) — A cast to a variant answers the variant: a provable miss is refused, an unproven one defaults and warns

## Backends, execution and libraries — [DESIGN_DECISIONS_PLATFORM.md](DESIGN_DECISIONS_PLATFORM.md)

- [C3](DESIGN_DECISIONS_PLATFORM.md#c3--wasm-par-runs-sequentially) — WASM `par()` runs sequentially
- [C70](DESIGN_DECISIONS_PLATFORM.md#c70--no-per-library-ir-snapshot--cache) — No per-library IR snapshot / cache
- [C71](DESIGN_DECISIONS_PLATFORM.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model) — Native libraries compile, scripts interpret — the steady-state execution model
- [C72](DESIGN_DECISIONS_PLATFORM.md#c72--repl-session-resume-does-not-persist-rng-generator-state) — REPL session resume does not persist RNG generator state
- [C78](DESIGN_DECISIONS_PLATFORM.md#c78--the-rust-engine--loft-library-boundary-mechanism-not-genre-and-no-black-boxes-above-the-engine) — The Rust-engine ↔ loft-library boundary: mechanism not genre, and no black boxes above the engine
- [C84](DESIGN_DECISIONS_PLATFORM.md#c84--server-ships-as-minimal-tcpws-primitives-not-a-fully-featured-http-framework) — `server` ships as minimal TCP/WS primitives, not a fully-featured HTTP framework
- [C87](DESIGN_DECISIONS_PLATFORM.md#c87--rust-template-path-is-kept-do-not-migrate-it-away-to-per-op-emitters-pln81-closed) — `#rust"..."` template path is KEPT; do NOT migrate it away to per-Op emitters (@PLN81 closed)
- [C93](DESIGN_DECISIONS_PLATFORM.md#c93--a-par-workers-captured-parent-state-is-read-only-a-write-to-it-is-a-compile-error) — A `par` worker's captured parent state is read-only; a write to it is a compile error
- [C96](DESIGN_DECISIONS_PLATFORM.md#c96--library-shipping-is-keyed-on-trust-root-presence-a-key-present-machine-ships-autonomously-a-key-absent-one-defers) — Library shipping is keyed on trust-root presence: a key-present machine ships autonomously, a key-absent one defers
- [C102](DESIGN_DECISIONS_PLATFORM.md#c102--a-release-binary-says-nothing-when-it-falls-back-to-the-interpreter) — a release binary says nothing when it falls back to the interpreter
- [C106](DESIGN_DECISIONS_PLATFORM.md#c106--c-has-one-arity-ceiling-for-both-backends-and-it-was-raised-rather-than-lowered) — `#c` has ONE arity ceiling for both backends, and it was raised rather than lowered
- [C107](DESIGN_DECISIONS_PLATFORM.md#c107--the-c-signature-decides-whether-a-vector-carries-a-count-not-the-loft-type) — the C signature decides whether a `vector` carries a count, not the loft type
- [C108](DESIGN_DECISIONS_PLATFORM.md#c108--a-vectort-and-the-c-pointee-are-two-spellings-of-one-layout) — a `vector<T>` and the C pointee are two spellings of ONE layout
- [C109](DESIGN_DECISIONS_PLATFORM.md#c109--a-float-return-crosses-c-a-float-argument-still-does-not) — a float RETURN crosses `#c`; a float ARGUMENT still does not
- [C122](DESIGN_DECISIONS_PLATFORM.md#c122--the-contract-is-semantics-a-rewrite-is-free-wherever-its-conditions-are-validated-and-a-library-api-is-the-one-boundary) — The contract is semantics; a rewrite is free wherever its conditions are validated, and a library API is the one boundary
