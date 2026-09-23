<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# tests/comparisons — the comparison claims, as programs that run

Every file here is a **working loft program that proves one claim** a language comparison
makes. They run in `make ci` through `wrap::comparisons`, on the interpreter and under the
same `run_test` the reference pages use, so a claim that stops being true turns a test red
instead of leaving a stale sentence on a page.

## Why this directory exists

`doc/00-vs-rust.html` and `doc/00-vs-python.html` were, until 2026-09-23, **the only reference
pages with no `.loft` source**. Every other page under `doc/` is generated from
`tests/docs/*.loft` and therefore executed; those two are hand-written HTML, and between them
they carried **28 loft code samples that nothing ran** — on the two pages a newcomer reads
first. `scripts/doc_review.py` and `scripts/reference-review.py` name them, but those schedule
a by-hand read; neither executes anything.

These comparisons have earned better than that. Ten defects were filed from the OCaml and Lua
measurements and all ten are closed; the Julia comparison produced `@F122`, multiple dispatch,
shipped. The comparisons are a *source* of bugs and designs, so their claims are worth gating.

## The contract

One file per subject, named for the subject key — the anchor the two comparison pages already
share (`null`, `variables`, `closures`, …). Each file carries a header:

```loft
// @SUBJECT: null
// @CLAIM: a nullable field reads straight and answers null; nothing forces a discharge
// @SEEN-FROM: 00-vs-rust.html#null · 00-vs-python.html#null
```

and then **asserts** the claim. Not `println` — an assertion, so the file fails when the claim
does. The other language's snippet goes in a comment beside it, which is what makes the file
readable as the documentation it is.

Rules, all of them the corpus's:

1. **Assert, don't print.** A printing file passes while answering anything.
2. **One subject per file.** The failure has to name the claim.
3. **No `use` of a library.** This harness builds no packages — the same reason
   `tests/docs/` holds none.
4. **Both backends where the claim is about behaviour.** `wrap::comparisons` runs the
   interpreter; `make test-native` covers `tests/docs/` and this directory is added to it for
   the same reason.

## What this does NOT do yet

It does not check that the code on the HTML page is the code in the file. That gate is worth
having — it is the drift `check_doc_drift.sh` exists for elsewhere — and it is not built. Until
it is, a file here proves the CLAIM is true; it does not prove the PAGE says it accurately.

Subject index and the links to every rationale: [`doc/claude/SUBJECTS.md`](../../doc/claude/SUBJECTS.md).
