<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 — INDEX (`v[i]`) FLIP: the F1a landing phase (START HERE next session)

Cold-start entry point for landing the index (`v[i]` → `τ?`) flip as the default. The
**foundation is already built and committed** (dev-gated, inert); this doc is the step-by-step
to flip it ON without breaking the tree. Read [RESUME.md § MOST RECENT](RESUME.md) first for the
one-paragraph context, then this.

## State (what's already done, on `tuxedo-pln85-fuzz-proof-gate`, pushed)

- **Mechanism (`c0aad2b2`, `c58d7623`):** `parse_vector_index` (fields.rs) sets `self.last_index_fit`
  for each SCALAR `v[i]` read; `parse_index` (fields.rs ~656) wraps the element type `Optional`
  when `!last_index_fit`, **gated behind `LOFT_INDEX_DEV`** (temporary dev-gate; folds into
  `pln25_dn1_enabled` at the end).
- **Fit-proofs (`index_provably_fit`, fields.rs)** — a `v[i]` types NON-null when the index is:
  - any compile-time constant (`self.const_int(index).is_some()` — covers `v[0]`, `v[-1]`, `v[2+3]`);
  - a for-loop iter var (`self.vars.is_active_loop_var(v)` — no separate stack);
  - proven `< len(vec)` by an enclosing `if idx < len(vec)` guard (`index_bounded: Vec<(u16, VecKey)>`,
    fed in `parse_if` (control.rs) by REUSING `operators::collect_guard_pairs`, THEN-only).
- **Reject mechanism proven:** undefended general-var index into a non-null store rejects with the
  standard `(N-Store)` diagnostic. Accept path (const / iter / guard / `?? d`) all run.

## The blocker that stopped the naive "flip + migrate 8 sites" (READ THIS)

Flipping `LOFT_INDEX_DEV` ON breaks **6 wrap tests**, and the reject-count measurement MISSED most
of them (see [[measure-flip-by-running-suite]] — measure by RUNNING the suite, not grepping rejects):

1. **SILENT copy-elision wrong-answer (the load-bearing fix, do FIRST).** `v[i]` typing `Item?`
   mis-classifies a MUTATED / ESCAPING element borrower (`e = v[i]; e.x = 99`) in the copy-elision
   analysis, so the copy-keep decision flips and the mutation LEAKS to the source. Repro:
   `tests/scripts/85-borrow-elision-element-borrower.loft` `mut_elem` returns the wrong value on
   BOTH backends (a wrong ANSWER, no compile error). This is the deps subsystem — loft's #1 weakness.
2. **loft-in-loft COMPILER ripple:** `dir` / `last` / `parser_debug` / `wasm_dir` — these run the
   compiler over other files, and `lib/parser.loft` + `lib/code.loft` use `v[i]` (~130 undefended
   reads; 0 direct rejects, but they ripple through these tests). Same class as the DN1 flip's F1a.
3. **`library_suite` (audience_crystal)** — a library `v[i]` / field-defense interaction.
4. **`loft_suite`** — the .loft corpus (85-borrow + the direct rejects below).

## The steps (each ends green; commit per step)

**Step 0 — reproduce + MEASURE by running (not grepping).** Flip the gate in fields.rs
(`std::env::var_os("LOFT_INDEX_DEV").is_some() && …` → `crate::keys::pln25_dn1_enabled() && …`),
build, then run and read FAILURES (asserts / panics / wrong values), both backends:
`cargo test --release --test wrap --test issues` and `LOFT_TIMEOUT=90 loft --native <corpus>`.
Then REVERT the gate flip and work under `LOFT_INDEX_DEV=1` per step. (Keep the gate dev-gated until
Step 6 so the tree stays green between commits.)

**Step 1 — ✅ DONE (copy-elision `Optional`-peel).** Fix = peel `.base()` before reading the
borrower's deps in `use_analysis` (the `borrowers` filter, `analyze_fn` ~504): `function.tp(e)
.base().depend().contains(&v)`. `Type::depend()` has no `Optional` arm (it recurses only through
`RefVar`/`Tuple`), and the index flip attaches the element's dep to the INNER type before the
`Optional` wrap (`parse_index` fields.rs 632-676 → `Optional(Reference(Item, [xs]))`), so without
the peel a nullable element borrower's dep was invisible → a mutated/escaping `e = v[i]` was missed,
the copy wrongly elided, and the mutation LEAKED to the source (SILENT wrong-answer).
- **Why detection-only is the whole fix (no re-point change needed):** the peel makes a mutated/
  escaping borrower `∈ ineligible` → the plan is REFUSED → copy kept (the fix). A read-only borrower
  is now detected + planned, but the re-point (`make_independent`/`depending`, also `Optional`-opaque)
  no-ops — which is BENIGN: `elide_rewrite` inlines the source into the borrower's def (`e = g.c[i]`),
  reads use that stored ref (not `e`'s dep), and the free pass already peels `Optional`. Proven on the
  boundary matrix (read-only elides+reads right / mutated keeps copy / escaping keeps copy) + a
  read-only stress set (late read, two borrowers, into-call), value+leak, BOTH backends, both gates.
- **Safe-by-construction for the default suite:** `.base()` is identity for non-`Optional` types;
  for an `Optional` borrower it can only ADD a detection, which only ever REFUSES a plan (keeps a
  copy — always correct) or adds a harmless read-only detection. It cannot introduce a wrong answer.
- **Regression:** `tests/scripts/25-index-elision-borrower.loft` (self-asserting; gate-OFF
  byte-identical) + `tests/pln25_dn1_consumption.rs::index_dev_elision_borrower_{interpret,native}`
  (runs it under `LOFT_INDEX_DEV=1`, both backends). NOT touched: `depend()`/`depending()`/`deps_ref()`
  globally (74/31/3 callers, DN1-default-live — wider than the failing region, no correctness gain).

<details><summary>Original Step-1 notes (starting points)</summary>

Make the element-borrower classification peel `Optional` so `e = v[i]` where `e: Item?` is still
recognized as borrowing `v`, and its mutation/escape is still detected (so a mutated/escaping
borrower KEEPS the copy). Starting points found this session:
- `src/use_analysis.rs` — `ElidePlan` (struct ~195; `borrowers` field ~202 "Vars that BORROW `v`
  (`e = v[i]` …)"; `visit` ~210; the re-point comment ~498). The plan is only emitted when "every
  borrower is read-only and non-escaping" — that read-only/escaping classification is what breaks
  when `e` is `Optional`-typed.
- `src/scopes.rs:301` — "A var that BORROWS `v` (`e = v[i]`, `deps` ∋ `v`)".
- **Method:** boundary matrix on the 85-borrow trio (read-only ELIDES + reads right; mutated KEEPS
  COPY + source unchanged; escaping KEEPS COPY) — value + leak, BOTH backends (the
  `engineering-rigor` + `loft-codegen` skills; the deps north-star is `OWNERSHIP_MODEL.md`). The
  fix is almost certainly a `.base()`/`peel_optional` at the point the classifier reads the
  borrower's type or matches the `e = v[i]` shape — but PROVE it on the matrix, don't guess (this
  is the deps system; symptom-patching it is how the heap model became loft's #1 weakness).

</details>

> **🟠 BRANCH WAS RED (pre-existing, NOT caused by Step 1) — surfaced 2026-07-01b by a full-suite run.**
> Mid-step-f debt the RESUME's targeted "issues/wrap/format green" claims did not cover. **Run via
> `find_problems`, NOT raw `cargo nextest`** — the wrapper rebuilds cdylibs + clears stale `.loftc`; a
> raw nextest shows multiplayer/viewer/html_asyncify/runtime_warnings as STALE-FIXTURE FALSE failures.
> The authoritative pre-fix set was **17 real failures**, none copy-elision:
> 1. **✅ FIXED (`73fe4f6c`) — store codec now serializes `Type::Optional` (`TyOptional` variant, disc 25).**
>    Cleared all 11 store-codec failures (`ir_read::*round_trip*`, `corpus_store_codec_round_trips`,
>    `g2_ir_check`). Root: `ir_store::write_type` peeled the wrapper (persisted `Optional(Integer)` as
>    `Integer`); read had no arm. Note: a full `extract.py` regen is currently unsafe (the binary's
>    `--show-rust` now emits `db.dbref()` for struct-ref fields, which extract.py drops + would shift
>    offsets) — the `TyOptional` schema line is hand-added, `ir.loft` is the source of truth.
> 2. **✅ DONE (`fa698799`) — 9 DN1/DN3 Rust-test migrations + div-warning retirement + clippy.**
>    `expressions::{call_int_null,call_text_null}` → `-> τ?`; `{bounded_mixed,bounded_unary}` → `?? 0`;
>    the div/mod runtime warning is RETIRED under DN1 (already in code) — `runtime_warnings` +
>    `exit_codes::div_by_literal_constant_no_warning` migrated to assert its absence; `error_messages`
>    goldens 17/18/19/28 regenerated; `handle_operator` clippy allow. (The constant-divisor fit-proof
>    already worked — `divisor_provably_nonzero`; the migration was the variable-divisor case.)
>
> **REMAINING is NOT core — registry-gated + environmental:** multiplayer(v2/v3/v5) + viewer_markdown
> fail on DN1 rejects in the `web-0.2.1` / `server-0.2.0` REGISTRY libs (`return null` into `-> text`) —
> needs the `text?` migration + a touch-gated republish (loft-library; USER-GATED). `html_asyncify` (chrome +
> stale GL cdylib) + `error_messages::38_import_unknown` (sandbox DNS) are environmental.
>
> The compiler-side green gate is MET. Continue with the index flip (Steps 2-6 below), OR the registry republish.

> **PROGRESS (2026-07-02) — mechanism + issues/85-borrow/audience_crystal done; a full corpus sweep
> mapped the rest.** ⚠️ CORRECTION: an earlier note claimed "wrap PASSES → Step 2 done" — that was a
> DEAD-BACKGROUND-RUN measurement error (an empty log misread as a pass; [[measure-flip-by-running-suite]]).
> The authoritative full sweep (`LOFT_INDEX_DEV=1 find_problems`) shows the REMAINING index-flip failures:
> - **wrap dir/last/parser_debug/wasm_dir** — the loft-in-loft compiler: **lib/code.loft** has 4
>   cast-from-nullable rejects `self.code[b] as Block/If/If/Loop` (b = `self.blocks[len-1]` computed →
>   `self.code[b]` is `Value?` → `as Block` = "Unknown cast from Value? to Block"). lib/parser.loft is
>   CLEAN. This is the loft-in-loft codegen — DELICATE (mis-migration breaks self-hosting); needs a
>   discharge idiom for `nullable as T` (NOT laundering — DN5 stays closed; discharge THEN cast).
> - **html_wasm moros_editor_html_smoke** — a moros consumer with v[i] sites (unmeasured).
> - **runtime_warnings skip_loop_bounded_arithmetic_index** — `sum += m[i*4+j]` (loop-bounded ARITHMETIC
>   index, not a bare loop var → not fit → `float?` into `sum`); migrate (`?? 0.0`) — the fit-proof
>   deliberately doesn't do range analysis on `i*4+j`.
> - **loft_suite** — one corpus script rejects under the flip (identify + migrate).
> - Pre-existing/environmental (NOT index): error_messages(38 DNS), html_asyncify(chrome), multiplayer(registry).
>
> So Step 2 (lib/code.loft) is REAL + delicate, and there's more corpus (html_wasm, loft_suite,
> runtime_warnings) before Step 6. Committed so far:
> - **✅ Element-WRITE mechanism (`81641e7d`)** — `v[i] = h` (moros_map:115, p379) errored "Cannot assign
>   to attribute on OpGetVector" because `towards_set`'s copy_ref gate didn't peel the `Optional(Reference)`
>   read-marker. Fixed: peel `.base()` in the copy_ref decision (collections.rs:761). An element-write is
>   an lvalue SLOT, not a nullable read. Gate-OFF byte-identical; issues 748/0.
> - **✅ Step 4 direct read-rejects (`b46413d0`)** — issues p124×2 (`arr[idx] ?? 0.0`), p155 (`?? H {}`,
>   SEGV/undo aliasing preserved), p170 (`p170_bs[len-1] ?? P170Bag {}`), 85-borrow esc_elem (discharge
>   `xs[i] ?? Item {}`, keep `-> Item`). p379 fixed by the element-write mechanism, not a migration.
> - **⚠️ DEFERRED DEPS GAP** — a bare `-> Item?` escaping-BORROWER return (esc_elem's naive form) LEAKS
>   (the Optional-return × copy-elision interaction; a plain `-> S?` struct return is leak-clean, so it's
>   specific to an escaping vector-copy element). Avoided via the discharge migration; the underlying
>   free-path peel is a separate deps-subsystem fix.
> - **🟡 Step 3 audience_crystal (`cc7cb722`, PARTIAL)** — palettes + cell_h/cell_h_at + sort-swap
>   discharged; a few `-> integer` accessor sites remain (n_store_violation line attribution is
>   MIS-ATTRIBUTED to fn signatures — read the body by hand; the loop-var fit-proof is fine, verified).
>   Large index-heavy library; gate-OFF compiles clean, library_suite green.
> - **Step 5 (len-capture): NOT surfaced** — no site needed it (const / iter-var / `??` covered all).
> - **Step 6: NOT STARTED** — needs audience_crystal finished + a full under-flip corpus/library sweep first.

**Step 2 — lib compiler migration** — 🟡 casts FIXED (`0cae0a6d`), but a DEEPER memory corruption blocks it.
The 4 cast-from-nullable rejects (`self.code[b] as Block/If/If/Loop`) are fixed with the guard idiom
(gate-OFF wrap 5/5). But running the now-compiling loft-in-loft compiler under the flip **SIGSEGVs / corrupts**.

> **🔬 DEBUG FINDINGS (2026-07-02, session) — the loft-in-loft memory corruption under the flip.**
> - **Repro (minimal):** `parser::parse("s", "struct S {{ x: integer }}"); parser::parse("shape", "enum
>   Shape {{ Circle {{ radius: float }} }}")` under `LOFT_INDEX_DEV=1`. First parse corrupts memory; the
>   second reports a GARBAGE position `Expected : on :281479271743489:1` (interp) or **SIGSEGV** (debugger,
>   opcode 206, at lib/lexer.loft:476). gate-OFF: clean (`shape:1:60`).
> - **Trigger (sharp):** a struct WITH A FIELD. `struct S {{}}` (empty) and `fn f(){{}}` are BOTH clean;
>   only a field triggers it → the corruption is in the **struct-field storage**.
> - **Site:** `Code.field` (lib/code.loft:133-135): `in_type = self.definitions[self.cur_def ?? 0].typedef;
>   self.types[in_type].type_fields += [Field {{…}}]`. Under the flip `self.definitions[…]` / `self.types[…]`
>   are `Optional`-wrapped element reads; a store/ref goes bad and corrupts the def/type tables (a write to
>   wrong memory — hence SIGSEGV later, and the corrupt value `0x0001_0001_0001_0001` = four u16=1 packed).
> - **Ruled out (isolated probes all PASS under the flip):** narrow (u16) field READ of an Optional element
>   (`toks[i].line ?? 0`); scalar field WRITE (`defs[i].typedef = tp`, adjacent field intact); vector-field
>   APPEND of an Optional element (`defs[i].fields += [v]`). So it is EMERGENT — a deps/store-lifetime
>   corruption in the full context (loft's #1 weakness), not a simple width bug.
> - **NEXT (focused deps investigation):** matrix-first on the STORE — LOFT_LOG=ref_debug / a debug build to
>   name opcode 206 + the exact corrupting write; likely a dangling/wrong DbRef from an Optional-wrapped
>   `v[i]` element whose store-lifetime the deps pass mishandles. Fix at the deps chokepoint (OWNERSHIP_MODEL.md),
>   then re-run wrap dir/last/parser_debug/wasm_dir + loft_suite under the flip. This is the real Step-2 blocker.

**Step 3 — audience_crystal library** — 🟡 PARTIAL (`cc7cb722`). Finish the remaining `-> integer`
accessor discharges (read bodies by hand past the mis-attributed line numbers), then measure the rest
of the corpus/libraries under `LOFT_INDEX_DEV=1` (only issues/wrap/85-borrow/audience_crystal measured).

**Step 4 — the direct N-Store rejects** — ✅ DONE (`b46413d0`): issues p124×2/p155/p170 + 85-borrow.
p379 resolved by the element-write mechanism fix (`81641e7d`).

**Step 5 — len-capture proof** — not needed (no site surfaced it).

**Step 6 — flip the gate + retire the warning + graduate.** Change the fields.rs gate
`LOFT_INDEX_DEV` → `pln25_dn1_enabled`; RETIRE the now-redundant `FaultKind::VectorIndex` /
`TextIndex` warning under DN1 in `emit_undefended_warning` (operators.rs — same as Div/Rem got in
`7042d94c`, since the TYPE is now the enforcement); graduate `tests/scripts/25-index-nullable.loft`
(accept: const / iter-var / then-guard / else-guard / `??` / honest `τ?`) + a reject twin in
`102-expected-errors.loft`. Full suite green both backends → done; update RESUME F1b + the
formal/types.md deviation.

## Also on the DN3 fault-op docket (separate slices, not part of F1a)

- **gap #3** — `??` discharges regardless of its fallback's nullability (`handle_null_coalesce`,
  operators.rs:1366 `*ctp = ctp.base()`); `x ?? null` unsoundly types non-null. Rule: result is `τ?`
  iff the fallback is nullable. Own blast radius (every `a ?? nullableVar`).
- **call-arg N-Store** — a nullable passed into a non-null CALL ARG is not checked (`takes(v[j])`
  doesn't reject). Affects division equally; closing it covers all fault-ops at once.
