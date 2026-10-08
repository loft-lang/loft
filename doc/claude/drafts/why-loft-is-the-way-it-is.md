<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# DRAFT — "Why loft is the way it is"

> **Status: draft, not published.** This is T3.4 of the first-contact work
> ([../FIRST_CONTACT.md](../FIRST_CONTACT.md)). It lives under `doc/claude/` so
> nothing goes live until the owner approves it; on approval it moves to `doc/`
> and gets a nav entry.
>
> **Every entry below is sourced from the repo** (`GOALS.md`,
> `DESIGN_DECISIONS.md`) — nothing here is invented rationale. Entries marked
> **[needs Jurjen]** are the ones the review says should trace to the author's
> previous production languages; they are written generically and need his
> input before they say anything experience-backed.

Language-nerd audiences read and link *decision-with-reason* pages more than any
other page type for a small language. The tone rule for this page is the same as
everywhere else: state the reason once, plainly, and let an honest trade-off be
visible.

---

## Why there is no `let`

A local is introduced by assigning to it. There is no `let`, no `var`, no type
annotation on locals.

The reason is not brevity — it is that **types are rarely written at all**, so the
few places a type *does* appear are the places it carries information. Making a
declaration cheap would invite annotations that restate what the compiler already
knows. The full-word type names (`integer`, `boolean`, `character` — never `int`
or `bool`) do the same work from the other side: their weight is deliberate
friction against a pointless annotation.

That is also why `int` and `str` are **suggested but not legal** — the compiler
tells a newcomer the loft name, and the newcomer learns the vocabulary once
rather than writing a foreign dialect forever
([DESIGN_DECISIONS.md @C103](../DESIGN_DECISIONS.md)).

## Why there is one integer type, and no separate index type

`v[i]` takes any integer. There is no `usize`, and indexing never makes you cast.

The archetypal failure this avoids is Rust's `usize`: indexing must be
pointer-sized, which is a *toolmaker's* memory fact, so everyone who writes `v[i]`
pays for it — `i as usize`, `len() as i32` — all over ordinary code. loft calls
that a **wrong-moment tax**: a cost charged on the common path to serve a fact the
language should be keeping to itself.

The test loft holds itself to is frequency in idiomatic code: **a well-built
library, used as intended, should need zero `as`**. A deliberate `x as u8` at the
moment you *choose* to throw bits away is fine — you are in the editor, doing it
on purpose. ([GOALS.md § Goal F](../GOALS.md))

## Why records with indexes live in the language, not a library

`hash<T[k]>`, `sorted<T[k]>`, `index<T[k]>`, `spatial<T[x,y]>` are **types**, not a
library API and not new syntax. They are one shape over one store, and a spatial
range query reuses ordinary range syntax — `xs[(0,0)..(10,10)]` — instead of
inventing a query language.

Putting them in the language is what lets them share one storage model and one
lifetime story. A library version would need its own allocation discipline and its
own way to talk about keys, and the two would drift. ([GOALS.md](../GOALS.md))

## Why a fault produces `null` and the program keeps running

Divide by zero, index out of bounds, a failed parse, integer overflow: each yields
`null` and execution continues. There are no exceptions and no `try`/`catch`.

The model is a **spreadsheet**: a cell that cannot be computed shows nothing and
the sheet still opens. The whole fallible-value story is then two operators — `??`
supplies the default *you* give, `?` supplies the default the *type* gives — and
the type system forces you to discharge a `τ?` before storing it somewhere
non-null. So "no runtime errors" is not permissiveness; the checking moved to
compile time. ([DESIGN_DECISIONS.md @C80](../DESIGN_DECISIONS.md))

## Why loft compiles through rustc instead of its own backend

The north star is that loft is **correct now and stays correct as the world
changes underneath it** — new toolchains, new platforms. The failure guarded
against is latent undefined behaviour: a bug that passes every test on today's
toolchain and becomes silent corruption after a compiler upgrade.

Emitting Rust and handing it to rustc/LLVM buys that durability, and the honest
cost is visible on the performance page: the interpreter loses to CPython on some
benchmarks, and that page says so. ([GOALS.md § North star](../GOALS.md))

## Why four execution modes

Interpreter, `--native`, wasm, and browser export exist because the wedge is
*share a link, anyone plays* — which needs the browser — while development needs a
fast edit-run loop and CI needs something deterministic.

Keeping four modes honest is expensive: every semantic change ships to the
interpreter and the native backend **in lockstep**, and cross-mode divergence is
treated as a bug rather than a quirk. That cost is accepted deliberately, because
a language whose browser build behaves differently from its native build cannot
make the "share a link" promise. ([GOALS.md § Goal D](../GOALS.md))

## Why the standard library is small, and libraries are chunked

*[needs Jurjen]* — the repo has the mechanism (packages, the registry, chunk
repos) but not the *why* in one place. The interesting version of this entry is
presumably about what a large bundled stdlib costs over a decade.

## Why LGPL

*[needs Jurjen]* — the licence is LGPL-3.0-or-later throughout, but the repo does
not state the reasoning anywhere I can find. Worth one plain paragraph: it is a
question every evaluator asks, and silence invites the wrong guess.

## Why AI agents write the code

The repo is built so that **documentation and tooling rank above writing code**:
the full source, the docs, and executable skills that teach an agent how to fix a
bug, change code generation, or ship a library. Point a capable agent at the repo,
let it load the skills, and it can continue the work.

The claim that follows is the interesting one — the project's knowledge lives in
the repo rather than in a founder, so its **bus factor is effectively zero**, and
[BUS_FACTOR.md](../BUS_FACTOR.md) exists so a reader can verify that rather than
take it on faith.

---

## Entries still to write

- **The niche→general arc** *[needs Jurjen]* — see
  [provenance-paragraph.md](provenance-paragraph.md). Several entries above would
  land harder with one line of "in a previous production language I maintained, X
  caused Y", phrased generically, never naming employer or domain.
- **Why the store is word-addressed / position-independent** — real material
  exists in `DATABASE.md` and `OWNERSHIP_MODEL.md`; needs distilling.
- **Why compatibility is absolute at 1.0** — `COMPATIBILITY.md` has the policy;
  this page wants the one-paragraph reason.
