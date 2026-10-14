<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN115 — Parse-time resolution index

**Tracker:** [loft-lang/plans#115](https://github.com/loft-lang/plans/issues/115) ·
**Subject:** loft · **Status:** CLOSED 2026-07-22 (delivered)

> **CLOSED 2026-07-22.** Shipped on `main` in
> [loft-lang/loft#610](https://github.com/loft-lang/loft/pull/610) (merged 2026-07-21,
> bundled with the @PLN63 loft-lsp server); [loft-lang/plans#115](https://github.com/loft-lang/plans/issues/115)
> closed. The S1–S7 recording spine (Local/Global/Field/Method) and every consumer —
> references, rename, inlayHint (feature E, the reason this plan exists),
> go-to-definition, hover, semantic tokens, completion — are live, plus the
> declaration tail (params, `for`, long-lambda). Verified green on this tree:
> `tests/resolution_index.rs` (15, incl. the recording-off-by-default gate) and the LSP
> consumer suites `lsp_transport`/`lsp_semantic`/`lsp_completion`/`lsp_refs`/`lsp_rename`/`lsp_hover`
> (45). Remaining work is **optional** and untracked — the exotic S6 member paths
> (poly-enum, nullable-unwrap, bounded-T stub, vector methods) and the last declaration
> binders (short `|x|` lambdas, `for` destructure / `#fields`, param-with-lambda-default,
> constants); each is safe (no declaration → F-v1 name-scan fallback) and @PLN63 can
> pick any up when a consumer needs it.

## Status

**Phase 0 (design) DONE — see [design.md](design.md)** (concrete code points + the
S1–S7 small-safe-step spine + the byte-identical-IR gate).

**Execution: S1–S7 spine COMPLETE.** The plan delivered its goal — the parse-time
resolution index (Local/Global/Field/Method) plus its first consumers, including
inlayHint (E), the feature this plan was created to unblock.
- **S1** (`34c428c0`) — `pub mod resolution` (`Occurrence` + `Resolution`), the
  `record_resolutions`/`resolutions` fields (default off/empty), the gated `record`
  helper, `Parser::resolutions()`, and `resolutions.clear()` paired with every
  `deferred_unknown.clear()`. Inert — nothing calls `record`.
- **S2** (`da8bcca2`) — first hook: LOCAL occurrences in `parse_var`. At the pass-2
  `name_exists` chokepoint (where every local read/write/return flows, since the var
  was created on pass 1), records `Local { fn_def: self.context, var_nr }` at
  `name_pos`. Keyed on binding IDENTITY — two same-named locals in different fns get
  distinct `(fn_def, var_nr)`. Public `set_record_resolutions` setter added for S3.
  **Gate met:** `loft introspect` byte-identical with the gate off (S1 vs S2 binary,
  empty diff); `tests/resolution_index.rs` proves distinct bindings on + off-by-default.
- **S3** (`b85e84da`) — enable the gate on the LSP parse + expose the index. Factored
  the 7 duplicated `Parser::new()+load_stdlib+parse_source` blocks in `loft::lsp` into
  one `parse_lsp_buffer` helper that flips `set_record_resolutions(true)` *after* the
  stdlib load (only the user buffer records). New `lsp::resolutions(text, name,
  stdlib_dir)` returns the buffer's occurrences — the substrate S4/S7 build on.
  **Gate met:** the LSP parse carries occurrences (s3 test); the CLI parse never
  routes through here → `loft introspect` byte-identical to S2; all LSP suites green
  (refactor behavior-preserving).

- **S4** (`24cbe7a6`) — first CONSUMER: precise local references/rename.
  `lsp::local_binding_refs` resolves the binding under the cursor by identity
  `(fn_def, var_nr)` and returns its exact occurrences, so a same-spelled FIELD
  (`p.x` vs local `x`), method, or global is excluded — which the name-scan can't do.
  The server's references + rename handlers try it first, else fall back to F-v1.
  **Soundness:** the index records `parse_var` occurrences (uses + assignment targets)
  but not declarations the definition/loop/lambda parser makes, so the precise path
  is taken ONLY for an assignment-local whose decl IS captured (not a parameter of its
  fn; earliest occurrence is a declaring `name =` write). Params / loop / lambda
  binders fall back to F-v1 (which catches their decls). Grounded in probes: loft is
  flat-scoped per function; `p.x`'s field is not a recorded local. **Gate met:** unit
  + end-to-end transport tests (field excluded; param/loop fall back); CLI byte-
  identical to S3; all LSP suites green.

**Follow-up worth doing (would make S4 fully precise for ALL locals):** record the
missing DECLARATIONS in the index — a parameter's signature name (needs a `name_pos`
on `Argument` + a pass-2 hook in `definitions.rs`), a `for i` / lambda binder
(`collections.rs` / lambda parse). Then params/loop-vars take the precise path too
and the F-v1 fallback retires. Deferred from S4 as it touches the delicate definition
parser and each site needs its own byte-identical gate.

- **S5** (`f24cd1c3`) — record free-function CALLS as `Global(def_nr)` at the
  `mod.rs::call` chokepoint (after `find_fn` + generic-skip), for user free functions
  (`n_<name>`). Gated on `record_resolutions` first (zero-cost off) + pass 2.
  **Scope note:** the design sketched "Global/Method at call + constants at
  objects.rs", but methods actually resolve in `fields.rs` and constants in
  `parse_constant_value` (tangled with enum-variant/qualifier resolution) — neither is
  a clean single chokepoint here, so both move to **S6** (member/field access) where
  they group naturally. **Gate met:** `loft introspect` byte-identical off on a corpus
  that contains free-fn calls (so the branch is traversed + no-ops); test records
  Global(n_helper) with locals still recorded alongside; all LSP suites green.

- **S6** (`f4a66242`) — record field + method access. At the `fields.rs` member
  chokepoint (`fnr = attr(dnr, field)` resolved), a Routine attribute records
  `Method{recv_type, method_def}`, any other attribute `Field{type_def, attr}`. So
  `p.x`→`Field{P,x}` and `s.len()`→`Method{text, len}` — keyed on the receiver TYPE,
  so `text.len` is distinct from `vector.len`, and a field `x` from a local `x`. The
  member position is captured only when recording (`Position` holds a String). **Gate
  met:** byte-identical off on a field+method corpus; tests assert the exact
  Field/Method keys. Covers the common struct-field + attribute-method dispatch; exotic
  paths (poly-enum, nullable-unwrap, bounded-T stub, vector methods) not yet hooked.

- **S7** (`35a1c4c3`) — inlayHint (feature E, the reason this plan exists).
  `lsp::inlay_hints` emits `: <type>` after each assignment-local's declaration —
  position from the index (the earliest `name =` occurrence), type from
  `def(fn_def).variables().tp(var_nr)` via `type_name_str`. Params (explicit types) +
  loop/lambda binders + unresolved types skipped. The server advertises
  `inlayHintProvider`. **Gate met:** unit + end-to-end transport tests
  (`n: integer`, `s: text`; param/loop/reassignment skipped; positioned after the
  name); CLI byte-identical (pure consumer, parser untouched).

**Recording spine (S2–S6): Local, Global, Field, Method all resolved. Consumers:
S4 (precise local references/rename), S7 (inlayHint), and index-driven navigation.**

- **Navigation integration** (`22622fc4`) — go-to-definition + hover now resolve
  through the index via `lsp::resolve_at`: `Global`/`Method` reuse `hover_of_def`
  (real signature + `///` doc — a method jumps INTO the stdlib), a `Local` synthesizes
  `name: type` at its declaration, a `Field` shows `Type.field: type`. Both handlers
  try the index first, falling back to name-based `symbol_at` on a definition's own
  name. NEW live capabilities: definition + hover on LOCALS and METHODS (previously
  unresolvable by name lookup), globals position-precise. Pure consumer — CLI
  byte-identical. Tested unit + end-to-end (local use → its decl; method → stdlib).

- **Method find-references** (`925349e2`) — `lsp::method_refs` resolves the method
  under the cursor and returns every occurrence of THAT method across the workspace,
  keyed on the mangled method name (`t_<len><Type>_<method>`, stable across parses); so
  `text.len` references exclude a same-spelled `vector.len`. Each `.loft` file is parsed
  with the index (open buffers overlaid); the references handler tries it after the
  local path, before the global name-scan. On-demand cost (parse per file), not
  per-keystroke. Limitation: a cross-file USER method the calling file doesn't import
  isn't found (under-match, never over-match). Tested unit + cross-file transport.

- **Semantic tokens + completion consumers** (`e3551d7f`) — `semantic_tokens` matches each
  identifier against the index by position, classifying LOCALS (`variable`), METHODS
  (`method`), and FIELDS (a new `property` legend entry), falling back to the name lookup for
  globals/types. `complete`'s `expr.` receiver resolves scope-precisely in the function
  enclosing the cursor (a same-named `v` typed differently in two fns offers the right members
  in each). Both pure consumers — CLI byte-identical. Tested in `lsp_semantic` + `lsp_completion`.

**Every resolution kind — Local, Global, Field, Method — now has a live LSP consumer:
references, rename, inlayHint, go-to-definition, hover, semantic tokens, completion.**

**Tail — record the missing DECLARATIONS — DONE (params, `for`, long-lambda).** So
that params / loop-vars / lambda params take S4's precise path (excluding a same-named
field) instead of the F-v1 name-scan:
- **`Occurrence.declaration` flag + `record_decl`** mark a binding's declaration, so
  S4 confirms the declaration is captured (a complete rename) without the `name =`
  text heuristic assignment-locals use.
- **Parameters** — the signature name is recorded via a `pending_param_positions`
  side-table (mirroring `pending_param_locks`): captured during `parse_arguments`,
  recorded in `parse_function` once `self.context` is known. Param arg-index == var_nr
  (pass 2 re-types params by index), so no `Argument` struct change.
- **`for` binder** — `parse_for_iter_setup` returns the binder's `var_nr`; the simple
  `for i in …` binder is recorded right after (destructure / `#fields` iteration stay
  on F-v1).
- **`fn(e: T)` lambda** — parses params through the same `parse_arguments`, so
  `parse_lambda` consumes the same side-table.
- **S4** takes the precise path when the binding has a recorded declaration (removing
  the param bailout). Byte-identical-off gate held at every site
  (`bytecode-comparisons/tail-corpus.loft`, empty diff both backends).

Remaining tail gaps (each safe — no declaration → F-v1 fallback): **short `|x|`
lambdas** (inline param parse, not via `parse_arguments`), **`for` destructure /
`#fields`** binders, a **param with a lambda default** (the nested `parse_arguments`
clears the side-table), and **constants**. Optional follow-ups.

**Remaining (optional): exotic member paths** in S6 (poly-enum, nullable-unwrap,
bounded-T stub, vector methods) — hook for full Field/Method coverage.

This is the deferred FOUNDATION under @PLN63 (loft-lsp): every LSP feature that needs to
know *what an identifier occurrence refers to* — not just its spelling — depends on it.
It is filed as its own plan because it touches loft's most delicate subsystem (the
parser) and needs design + rigor before any code.

## Goal

Record, **during parse**, the binding each identifier occurrence resolves to — a map
from a source position to a `Resolution`:

- **Global** — a top-level def (`def_nr`): fn / struct / enum / typedef / constant / interface.
- **Local** — a variable / parameter: its function, `var_nr`, resolved `Type`, and scope.
- **Field** — `expr.field`: the receiver's type + the attribute.
- **Method** — `expr.method(…)`: the receiver's type + the method def.
- **Unresolved** — a name that bound to nothing (a typo, or @C80 value-undefinedness).

Expose it on the parser after a parse so tooling can answer *"what is the symbol at
`(line, col)`?"* and *"where are all the occurrences of THIS binding?"* precisely —
replacing @PLN63's lexical, name-based resolution.

## Why (the imprecision it removes)

`loft-lsp` today resolves identifiers **lexically** — by matching names. That is correct
for globally-unique names but wrong or blocked for everything scoped or type-dependent:

| Feature | Today (lexical) | With the index |
|---|---|---|
| find-references / rename of a LOCAL | approximated by enclosing block (F v1); no shadowing | exact binding, block-scoped |
| find-references of a METHOD (`len`) | over-matches every `len` token | only `text.len` (the resolved method) |
| **inlayHint (E)** | **BLOCKED** — variable positions unreliable in the parse path | binding position + `Type` available |
| completion after `expr.` | best-effort (first fn with a var of that name) | scope-precise receiver type |
| semanticTokens | globals + keywords only | locals / params / fields classified too |

All five need the *same* thing: per-occurrence resolution recorded at parse time. Building
it once lifts all of them — the reason it earns a plan rather than five separate hacks.

## The core design tension (Phase 0 decides)

The parser is loft's #1-weakness subsystem (heap / store-lifetime — see
[OWNERSHIP_MODEL](../../OWNERSHIP_MODEL.md), [STABILITY_HOTSPOTS](../../STABILITY_HOTSPOTS.md)).
Two ways to obtain the resolution:

1. **Gated instrumentation** at the parser's resolution chokepoints — where it ALREADY
   computes the binding: variable lookup (`Function::var`/`name_exists` in
   `parser/objects.rs`), field/method access (`parser/fields.rs`), call dispatch
   (`parser/mod.rs::call` / `parser/control.rs`). Append a `(span, Resolution)` record at
   each site. **Pro:** the resolution is right there — no re-derivation. **Con:** touches
   delicate code; must be strictly gated (zero cost when off) and proven behavior-identical.
2. **Separate resolver pass** over the parsed IR + the `Data`/variable tables. **Pro:**
   clean separation, the parser is untouched. **Con:** re-implements loft's scope + type
   resolution — a second source of truth that will drift (the exact anti-pattern the
   stability red-flags chase: two generators reading one IR).

**Leaning:** gated instrumentation (§1) — a single fact recorded where it is already
known beats a parallel resolver. But this is the load-bearing decision; Phase 0 proves it
with a byte-identical-IR gate (below) before committing.

## Sub-arcs / Phases

- **Phase 0 — design — DONE ([design.md](design.md)).** Decided: gated instrumentation
  (not a separate pass); the `Resolution` enum + `Occurrence`; storage on the `Parser`
  behind a `record_resolutions` gate (default off, zero-cost); the query API; and the
  **S1–S7 small-safe-step spine** with a byte-identical-IR gate on every step. The
  chokepoints are named to `file:line` — and the identifier positions are already in
  scope there (`parse_var`'s `name_pos`, threaded in @PLN63's fix-b).
- **Phase 1 — recording infra + one hook.** A `record_resolutions` gate on the parser,
  a `Vec<(Span, Resolution)>` store, and ONE hook — variable resolution. Prove on a probe
  that hovering / referencing a local resolves via the index. **Gate the gate:** with
  recording OFF, `loft introspect` IR is byte-identical to before (the recording must not
  perturb the parse — the [CODEGEN_METHOD](../../CODEGEN_METHOD.md) before/after diff).
- **Phase 2 — extend hooks.** Field access, method dispatch, call resolution, global refs.
  Each hook lands with a probe; the byte-identical-off gate holds throughout.
- **Phase 3 — wire the @PLN63 consumers.** Replace lexical resolution where it matters:
  E (inlayHint) reads binding position + `Type`; F-v2 makes references/rename per-occurrence
  exact for methods + shadowed locals; completion's `expr.` receiver resolves scope-precisely.
- **Phase 4 — validate.** Full suite both backends unaffected with recording off; the
  @PLN63 feature gates sharpened; graduate probes to regressions.

## Open questions

1. **Instrument vs. separate pass** (the §1/§2 tension) — Phase 0.
2. **Which parse pass records?** loft parses twice; pass 2 has resolved types. Record on
   pass 2 (types known), or both (positions on pass 1)? Interacts with two-pass dedup.
3. **Granularity** — all identifiers, or only the ambiguous (local / field / method)?
   Globals are already name-precise, so recording them may be redundant weight.
4. **Storage** — per-parse on the `Parser` (LSP re-parses anyway), or persisted in `Data`
   (reusable, but grows the cached bundle — cf. the @I86 stdlib cache round-trip).
5. **Span vs point** — record the full identifier span, or a start point + re-derive length?

## Risks

- **Perturbing the parse.** The #1 risk. Recording must be a pure side-append that changes
  nothing else; the byte-identical-IR-when-off gate (Phase 1) is non-negotiable.
- **Performance on the compile path.** The gated-OFF path must be provably zero-cost (a
  branch that never runs), or every `cargo run` / build pays for LSP-only machinery.
- **Two-pass subtlety.** Recording in the wrong pass captures pre-resolution types or
  duplicates.

## See also

- **Consumer:** [@PLN63 loft-lsp](../../lib_plans/63-lsp/README.md) — the plan whose E +
  F-v2 steps this unblocks (see its "Build order — remaining features").
- Method: [CODEGEN_METHOD](../../CODEGEN_METHOD.md) (the byte-identical before/after gate),
  [engineering-rigor](../../../../.claude/skills/engineering-rigor) (matrix-first).
- Substrate already in place: `Definition.position`, `Function::{var, name_exists, tp}`,
  `Parser.member_access`, `api_surface::classify` — the resolution index formalizes into a
  positional map what these expose piecemeal.
