<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# SUBJECTS — the axes loft made a choice on, and where each choice is recorded

Every language comparison loft keeps — the user-facing
[vs Rust](../00-vs-rust.html) and [vs Python](../00-vs-python.html) pages, the agent-facing
[bars](bars/README.md) — is a different cut through the **same set of subjects**. A subject is
one axis on which loft took a position: absence, mutability, ownership, dispatch, generics,
parallelism, failure.

This file is the hub. Each subject says what loft's answer IS, **why** (a link, never a
restatement), and **how the claim is checked**. Comparisons and registers link in; the
rationale stays in its own register and is not copied here.

**Why one file and not one per language.** A per-language file makes the reader ask "what does
loft do about null?" once per language and answers it three times, in three voices, drifting.
The subject is the stable thing; the language is the lens. Adding a fourth lens must not add a
fourth rationale.

## The two columns that matter

**Rationale** — a link to the decision, never its text. `DESIGN_DECISIONS.md` is the declined-
features register and holds the reasoning; a subject row points at the `C` entry. A row whose
rationale is **`— none recorded`** is a real gap, not an omission in this file: it means loft
behaves a certain way and nobody wrote down why, and it is the first thing to fix about that
subject.

**Verified by** — what makes the claim checkable. In descending strength:

| | |
|---|---|
| `tests/docs/<page>.loft` | the reference page is GENERATED from a program that runs in `make ci`, so every sample on it is executed |
| `tests/scripts/<guard>.loft` | a corpus guard, both backends |
| `formal/<chapter>` rule | a `@FR-` rule with cited enforcement sites (`scripts/rule_tags.py sites`) |
| **`UNVERIFIED`** | nothing runs the code that states this claim |

## ⚠ The verification gap, measured 2026-09-23

Of the reference pages, **`00-vs-rust.html` and `00-vs-python.html` are the only two with no
`.loft` source**. Every other page is generated from `tests/docs/*.loft` and therefore executed
by `make ci` — that is the property OCAML_BAR's tier G relies on when it calls the reference
pages a regression floor already gated.

These two are hand-written HTML. Between them they carry **28 loft code samples that nothing
runs**, on the two pages a newcomer reads first. `scripts/doc_review.py` and
`scripts/reference-review.py` both name them, but those schedule a by-hand read; neither
executes anything.

**The cure is built, and it is `tests/comparisons/`.** Rather than convert the two pages to
generated ones — which would cost them the side-by-side column that is their whole value — each
subject gets a **working loft program that asserts the claim**, run by `wrap::comparisons` in
`make ci` through the same `run_test` the reference pages use. The other language's snippet
lives in a comment beside it, so the file reads as the documentation it is.

Six subjects are covered so far (`null`, `variables`, `parameters`, `enum-dispatch`,
`closures`, `formatting`, `parallel`); the rest still read `UNVERIFIED` below and are meant to.
Writing one is cheap: the contract is [`tests/comparisons/README.md`](../../tests/comparisons/README.md).

**What it does not yet do** is check that the code ON the page is the code IN the file. A file
proves the CLAIM is true; it does not prove the PAGE states it accurately. That gate is worth
having and is not built.

---

## The subjects

Anchors match the two comparison pages, which already agree on eight of them (`variables`,
`null`, `loops`, `enum-dispatch`, `closures`, `generics`, `formatting`, `parallel`) — the
shared anchor is the subject key.

### `null` — how absence is spelled

**loft's answer.** Absence is a value of the type, written `τ?`, with an in-band sentinel per
scalar. Nothing forces a call site to discharge it: a nullable field reads straight and answers
null, and `??` / `?` are offered at the point of use rather than demanded at every read. A
fit-failure yields null rather than stopping the program.

**Rationale.** [C90](DESIGN_DECISIONS.md) (one bit-pattern reserved per nullable scalar) ·
[C80](DESIGN_DECISIONS.md) (the spreadsheet fault model — nothing stops a running calculation) ·
[C85](DESIGN_DECISIONS.md) (overflow types non-null; the game keeps running) ·
[C127](DESIGN_DECISIONS.md) (a declared narrow range has no null; an unfitting value takes the
type's default)

**Verified by.** `formal/types.md` `(N-Reserve)`, `formal/operational.md` `(E-Uncomp)` /
`(E-Uncomp-NN)` · `tests/scripts/1615-every-narrow-slot-answers-the-types-default-for-an-unfitting-value.loft` ·
`tests/scripts/1246-a-nullable-narrow-slot-answers-null.loft` · `tests/docs/23-safety.loft` · **`tests/comparisons/null.loft`**

**Seen from.** vs Rust §2 (against `Option<T>`) · vs Python §2 (against a universal `None`) ·
OCAML_BAR D1

---

### `variables` — declaration and mutability

**loft's answer.** No `let`, no `mut`; first assignment declares and infers, and a variable is
mutable. `const` is opt-in and is a *semantic* claim judged from the line, not an optimisation
hint.

**Rationale.** [C124](DESIGN_DECISIONS.md) (a `const` value reaches only a `const` parameter;
semantics is judged by the line) · the mutable-by-default choice itself is **— none recorded**

**Verified by.** `tests/docs/01-keywords.loft` · **`tests/comparisons/variables.loft`**

**⚠ In flight — do not write a scope cell against today's behaviour.** loft#1600 (owner ruling,
rustc's rule) makes a local bound inside a block end at its `}`; reading it after is the hard
error `local-out-of-scope`. That half is live. An extension ruled the same day takes the LOOP
VARIABLE with it — `for i in 0..3 { } i` becomes out of scope — and is not live yet: measured
2026-09-23 it still answers `2`, the last value, which is Python's behaviour and the opposite
of the rule about to land. Scope is a real axis of this subject and both comparison pages
should carry it; the cell is worth writing **after** the extension lands, not against a value
that is about to change.

**Seen from.** vs Rust §1 · vs Python §1

---

### `parameters` — ownership at the call boundary

**loft's answer.** Ownership exists and is internal. A heap value binds by copy, aliasing is a
last-use elision, a collection parameter is already shared, and `&` binds a live reference for
the cases that need write-back. There is no user-facing borrow checker.

**Rationale.** [C79](DESIGN_DECISIONS.md) (ownership is internal; no user-facing borrow
checker) · [C77](DESIGN_DECISIONS.md) (heap aliases by default; `&` binds a live reference) ·
[C86](DESIGN_DECISIONS.md) (whole-value heap binds COPY; aliasing is a last-use elision)

**Verified by.** `formal/ownership.md` and `formal/binding.md` (the `@FR-B-*` / `@FR-O-*`
families, with cited sites) · `doc/claude/OWNERSHIP_MODEL.md` is the north star · **`tests/comparisons/parameters.loft`**

**Seen from.** vs Rust §3

---

### `enum-dispatch` — choosing behaviour by type

**loft's answer.** A function name may have several definitions distinguished by the concrete
types of all parameters; the most specific applicable one is selected. Enum payloads are named
fields read directly, and `match` is for dispatch, never forced for extraction.

**Rationale.** [C89](DESIGN_DECISIONS.md) (no tuple-style enum variants; a matcher reads like
grammar and is never forced) · [C123](DESIGN_DECISIONS.md) (one name has one body per receiver
type) · [@PLN162](https://github.com/loft-lang/plans/issues/162) § *What NOT to take from
Julia* holds the non-goals

**Verified by.** `@F122` · `plans/162-multiple-dispatch/RULES.md` (deviations OPEN: 0) ·
`tests/introspect_dispatch.rs` · `tests/docs/09-enum.loft` · **`tests/comparisons/enum-dispatch.loft`**

**Seen from.** vs Rust §10 · vs Python §5 (against `isinstance`) · OCAML_BAR F1, F2

---

### `closures` — what a closure may capture

**loft's answer.** Capture is copy-at-definition. Same-scope capture works. A mutated scalar
may be captured by exactly one closure; a capturing closure cannot be stored in a collection;
a closure cannot write through a captured `&` scalar parameter.

**Rationale.** [C38](DESIGN_DECISIONS.md) (copy-at-definition) ·
[C74](DESIGN_DECISIONS.md) (a mutated scalar captured by only ONE closure) ·
[C75](DESIGN_DECISIONS.md) (closure-carrying struct values are frame-bound) ·
[C115](DESIGN_DECISIONS.md) (no write through a captured `&` scalar) ·
[C116](DESIGN_DECISIONS.md) (a collection element holds a plain fn-ref, never a capturing
closure) · [C62](DESIGN_DECISIONS.md) / [C63](DESIGN_DECISIONS.md) (no `|x|` annotations, no
nested `fn`)

**Verified by.** `formal/closures.md` · `tests/docs/26-closures.loft` ·
`tests/closure_matrix.rs` · **`tests/comparisons/closures.loft`**

**Seen from.** vs Rust §11 · vs Python §8 · OCAML_BAR B1–B6 · LUA_BAR LC2

---

### `generics` — type variables and bounds

**loft's answer.** Inferred, with structural interface bounds. Type variables are unrestricted
and a keyed collection stays a record set. Several type variables and generic types are
**in flight**, not absent by decision.

**Rationale.** [C126](DESIGN_DECISIONS.md) (a generic's type variables are unrestricted; a
keyed collection stays a record set) — which *revised* [C110](DESIGN_DECISIONS.md), so read
them in that order

**Planned.** [@PLN165](https://github.com/loft-lang/plans/issues/165) — arc C (several
variables, a variable in any parameter), arc D (generic structs and enums), arc E (`map` /
`reduce` as library generics). **OPEN.**

**Verified by.** `tests/docs/25-generics.loft` · `INTERFACES.md`

**Seen from.** vs Rust §12 · vs Python §11 · OCAML_BAR tier A (A1–A6, B6 — all `planned`
against @PLN165)

---

### `exceptions` — how failure is reported

**loft's answer.** No exception handling and no programmer-side `try`/`catch`. A recoverable
failure is a value (`FileResult`); an internal bug fails at startup, not at runtime; a
production program never aborts on a user-attributable edge case.

**Rationale.** [C66](DESIGN_DECISIONS.md) (production programs never abort on user-attributable
edge cases) · [C67](DESIGN_DECISIONS.md) (fail at startup, not at runtime — no programmer-side
try/catch) · [C80](DESIGN_DECISIONS.md) (the spreadsheet fault model)

**Verified by.** `tests/docs/23-safety.loft` · `tests/docs/13-file.loft` · `formal/` C80's rule
family · **`tests/comparisons/exceptions.loft`**

**Seen from.** vs Python §9

---

### `collections` — what a container is

**loft's answer.** Typed and built in, with keyed collections as record sets: insert dedups,
the subscript is uniformly key-addressed, two `index` collections over one element type and key
are refused, and a keyed collection is refused as a vector element.

**Rationale.** [C68](DESIGN_DECISIONS.md) (keyed collections dedup on insert) ·
[C99](DESIGN_DECISIONS.md) (a keyed subscript is uniformly KEY-addressed) ·
[C113](DESIGN_DECISIONS.md) / [C114](DESIGN_DECISIONS.md) / [C117](DESIGN_DECISIONS.md) /
[C118](DESIGN_DECISIONS.md)

**Verified by.** `tests/docs/07-vector.loft`, `10-sorted`, `11-index`, `12-hash` ·
`formal/collections.md` · `tests/scripts/158-keyed-fast-paths.loft`

**Seen from.** vs Python §7 · LUA_BAR tier A

---

### `parallel` — the concurrency unit

**loft's answer.** `par(...)` is a built-in parallel for-loop. A worker's captured parent state
is read-only and a write to it is a compile error. Under WASM it runs sequentially.

**Rationale.** [C93](DESIGN_DECISIONS.md) (a `par` worker's captured parent state is read-only)
· [C3](DESIGN_DECISIONS.md) (WASM `par()` runs sequentially)

**Verified by.** `tests/docs/19-threading.loft` · `THREADING.md` · **`tests/comparisons/parallel.loft`**

**Seen from.** vs Rust §14 · vs Python §10

---

### `formatting` — how a value becomes text

**loft's answer.** Embedded expressions in a text literal; every string is a format string.
`print` stays text-only — no bare `print(value)`, no variadic `print`.

**Rationale.** [C100](DESIGN_DECISIONS.md) (`print` stays text-only)

**Verified by.** `tests/docs/30-formatting.loft` · **`tests/comparisons/formatting.loft`**

**Seen from.** vs Rust §13 · vs Python §6

---

### `ecosystem` — what ships and what is bound

**loft's answer.** A minimal standard library; capability comes from libraries under their own
module namespace, and the Rust ecosystem is BOUND rather than reimplemented. `server` ships TCP
and WS primitives, not an HTTP framework.

**Rationale.** [C84](DESIGN_DECISIONS.md) (`server` ships minimal primitives) ·
[C97](DESIGN_DECISIONS.md) / [C98](DESIGN_DECISIONS.md) / [C101](DESIGN_DECISIONS.md) (module
namespacing) · [BROADENING.md](BROADENING.md) § Domain fit matrix holds the per-domain
reasoning

**Verified by.** `tests/docs/17-libraries.loft` · the library catalogue (`make libcatalogue`)

**Seen from.** vs Python §13

---

### Subjects with no recorded rationale

These have a documented behaviour and a user-facing comparison section, and **nothing says
why**. Each is a candidate `C` entry, and writing one is cheap now and expensive once someone
has built on the behaviour.

| subject | where it shows | what is missing |
|---|---|---|
| `loops` | vs Rust §5, vs Python §4 | why there is no `loop` keyword (`while true` is the form); OCAML_BAR carries this as an *erratum*, which is not a decision |
| `filtered-loops`, `loop-attributes`, `named-break` | vs Rust §6–8 | these are features (`@F`), not decisions — but the comparison presents them as trades, and the trade is unrecorded |
| `methods` | vs Rust §9 | methods by `self` name rather than `impl` blocks; [C123](DESIGN_DECISIONS.md) covers one name per receiver type but not the absence of `impl` |
| `structs` | vs Python §3 | structs instead of classes AND dicts — two refusals presented as one |
| `signatures` | vs Python §12 | defaults and named arguments, but no `*args` |
| `xor` / `exponentiation` | vs Rust §4, vs Python §14 | `^` is XOR and `**`/`pow()` exponentiates — a spelling choice against two languages' habits |

---

## What links here

| register | how it connects |
|---|---|
| [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) | holds every rationale; a subject row links to the `C` entry and never restates it |
| `@F` catalogue (`loft-lang/features`) | holds the user-facing description of a capability; a subject names the `@F` where one exists |
| `loft-lang/plans` | holds work in flight; a subject with a `planned` answer names the plan and its arc |
| GitHub issues | a defect against a subject's stated answer; the [bars](bars/README.md) record ten of them, all closed |
| [bars/](bars/README.md) | measure a subject against one other language; the bar row is evidence, this file is the position |
| `doc/00-vs-rust.html`, `doc/00-vs-python.html` | the user-facing cut; their anchors are this file's subject keys |

**Direction matters.** The links run FROM a subject TO the register that owns the answer, not
the other way. A subject row that grows its own rationale paragraph has become a second home
for it, and the two will drift — which is the failure this file exists to prevent, not to
commit.
