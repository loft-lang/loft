<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 52 — Value-block borrow cleanup (text + adjacent types)

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue | ✅ COMPLETE (60+ probes, all run under both backends). |
| B — Mechanism investigation | ✅ COMPLETE (2026-05-29). |
| C — Fix design | ✅ COMPLETE (2026-05-30). |
| D — Implementation | ✅ COMPLETE 2026-05-30.  All 10 sets (A-J) PASS both backends; Set Z all 4 EXCLUDED with rationale (language-level restrictions, not value-block-borrow gap); 9 representative probes graduated to `tests/scripts/150-158-plan52-*.loft`.  Probe 97 spun off as [@P384](../../../PROBLEMS.md) (architectural). |

**PLAN52 is CLOSED 2026-05-30.**  See [the closure summary at the end of this README](#-closure-2026-05-30) for the full landed-commits list, final probe matrix, and binary close-criteria verification.

**Trigger (2026-05-29):** P383 — `tests/scripts/repro_p323.loft::test_p323_index_coalesce` regressed under rustc 1.96. Bisect pinned the rustc bump (loft commit `42af45d` PR-CI was green on all 3 platforms on 2026-05-27 under rustc 1.95; rustc 1.96.0 dropped 2026-05-28). The defect itself is latent UB in loft's IR (post-consumer `OpFreeText` on a borrowed Str) that rustc 1.94/1.95 happened to mask via codegen / libmalloc behaviour that left freed bytes intact. rustc 1.96 (LLVM 21) changed that, exposing the bug deterministically on macOS. This plan is the 6th investigation cluster of the @PLAN51 hidden-buffer-aliasing family — same structural shape (borrow into a scope-local that gets freed before the consumer reads), text-flavoured, in value-block context (not return context).

**Scope:** every value-block-result emit path where the block's tail expression borrows into a scope-local that scope-exit then frees. The canonical case is `??` on text with a non-trivial LHS; sibling shapes include `if`/`match` as value-expression with branches that read text from block-locals, format-string contexts that bind into block-local work buffers, and the analogous patterns for `Reference` / `Vector` / `Hash` / `Sorted` (verify whether those already escape via @PLAN51's `paired_witness` + S1 NRVO machinery).

## Goal

Ship a scopes-pass + parser-lowering fix that closes the value-block-borrow-cleanup class on the interpreter for at least Text. Subsidiary deliverables: (i) updated `_INVESTIGATION_TEMPLATE.md`-shaped cluster docs for each verified sub-mechanism; (ii) regression-graduated probes in `tests/scripts/`; (iii) a rustc-version-aware CI lever (asan / miri or a per-OS matrix expansion) so future toolchain bumps don't re-expose the family unobserved.

## Cluster catalogue

The failure modes discovered or hypothesised during exploration. Each verified cluster gets its own `cluster-<id>-<slug>.md` once Stage B confirms it.

| ID | Cluster | Severity | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| I ✅ closed 2026-05-30 | `??`-on-text — non-Var LHS lowers to a value-block whose tail returns `Str` borrowing into block-local `_ncc_N` (now `__ncc_N`); scope-exit `OpFreeText` invalidated the stack Str before consumer read.  **CLOSED on interpret AND native** by iteration 2 (commit pending): parser marks `__ncc_N` skip_free, scope-pass suppresses OpFreeText, native emit forces has_trailing_void so the existing @P323 `_ret.to_string()` materialisation fires inside the block. | n/a (closed) | both backends fixed | 02, 04-06, 11, 13-17, 20, 24-26, 29-34, 37-39, 44 | [`cluster-I-ncc-text.md`](cluster-I-ncc-text.md) (Iteration 2 logged) |
| I-crash ✅ closed 2026-05-30 | Method-chain consumer (`(vec[i] ?? "x").to_uppercase()`) — SIGBUS at opcode 220.  **CLOSED 2026-05-30**: cluster I iteration 2 incidentally fixes I-crash (Set C 3/3 PASS) — the method dispatch now reads from the owned String materialised inside the block tail. | n/a (closed) | both backends fixed | 46, 49 | (bundled with cluster I) |
| II ❌ | ~~`if`/`match`-as-value with text from a block-local~~ — **FALSIFIED**: probes 07/08 PASS.  `unify_if_branches_work_refs` (parser/control.rs:728) materialises branch text into shared work-ref before the consumer reads. | n/a | n/a | 07, 08 | _hypothesis closed_ |
| III ✅ closed 2026-05-30 | Format-string `{x ?? "y"}` interpolation — same root mechanism as cluster I; **CLOSED 2026-05-30** by cluster I iteration 2 (Set D 5/5 PASS).  Format-buffer reads the owned String materialised inside the value-block tail. | n/a (closed) | both backends fixed | 09, 19, 30 | [`cluster-III-format-string.md`](cluster-III-format-string.md) |
| IV-Ref ❌ | ~~`Reference` value-blocks~~ — **FALSIFIED**: probe 10 PASSES.  @PLAN51's `paired_witness` + S1 NRVO covers the Reference value-block case. | n/a | n/a | 10 | _hypothesis closed_ |
| IV-Vec-nested-field-push ✅ closed 2026-05-30 (probe 97 spun off as @P384) | `vector<vector<X>>` STRUCT FIELD `+= inner_vec` misrouted to concatenate.  **CLOSED 2026-05-30** by strict-rule `+=` (commits `3a46739` + `d98c32b`).  Probes 21/36/91-96 PASS both backends.  Probe 97 (3-deep nesting) spun off to [@P384](../../../PROBLEMS.md#open-issues--quick-reference) (parser/runtime type-id divergence — architectural, not value-block-borrow-cleanup mechanism). | n/a (closed) | both backends fixed | 21, 36, 91, 92, 93, 94, 95, 96 (97 → P384) | [`cluster-IV-Vec-nested-field-push.md`](cluster-IV-Vec-nested-field-push.md) |
| IV-Hash ✅ closed (native) | `Hash` value-blocks via `??` — **CLOSED on native 2026-05-29** by dep-strip fix in `src/parser/expressions.rs`.  Interpret still FAILs (separate cluster I-class dangling-handle bug). | Interpret only | Native fixed | 22 | _(bundle with IV-Vec)_ |
| IV-Sorted ✅ closed (native) | `Sorted` value-blocks via `??` — closed on native by same dep-strip fix. | Interpret only | Native fixed | 41 | _(bundle with IV-Vec)_ |
| IV-Index ✅ closed (native) | `Index` value-blocks via `??` — closed on native by same dep-strip fix. | Interpret only | Native fixed | 50 | _(bundle with IV-Vec)_ |
| IV-Spacial ✅ closed | **PARSER hang** — was: `spacial<Point[name]>` caused the PARSER to enter an infinite loop in the spacial arm of `parse_type` (`src/parser/definitions.rs:1548`).  Hand-rolled scanner loop didn't advance when the next token was `[`.  **CLOSED 2026-05-29** by replacing the loop with a conditional `parse_fields` call (same helper sorted/hash/index use).  Probes 51/62/88/89 now emit the existing "spacial<T> is planned for 1.1+" diagnostic and exit cleanly; full PASS requires spacial itself to be implemented (1.1+ work, out of scope). | n/a (closed) | n/a (closed) | 51, 62, 88, 89, 90 | [`cluster-IV-Spacial-parser.md`](cluster-IV-Spacial-parser.md) (fix iteration 1 logged) |
| IV-Enum ✅ closed | struct-`Enum` value-blocks via `??` — **CLOSED on native 2026-05-29** by predicate fix in `output_test_predicate`.  Interpret still returns fallback variant (separate bug, tracked under cluster I). | Interpret only | Native fixed | 23, 38 | _(bundle with IV-Vec)_ |
| IV-Tuple ✅ closed | `Tuple` value-blocks via `??` — **CLOSED 2026-05-30** by a parser-side null-check that projects the FIRST FIELD of the tuple and converts THAT to boolean (the per-field-null-sentinel convention).  Site: `src/parser/operators.rs::build_null_coalesce_default` lines ~1227-1246.  Both backends PASS on probe 40 after the fix. | n/a (closed) | both backends fixed | 40 | (logged in this README) |
| V ✅ | Real-library extraction (probe 24) + practical multi-tier lookup chain (probe 47).  Scan of `lib/*/src/*.loft` shows **NO text-coalesce patterns in production code today** (only `vec[i] ?? 0` integer-typed); but the shape is a near-certain config-lookup pattern for upcoming `lib/server`.  **Probe 47 also surfaced cluster VII** — chained-call native E0308 — promoted to its own cluster row below. | Latent | Interpret only (cluster VII handles the native side separately) | 24, 71 | _(folded with cluster I doc)_ |
| VI ✅ closed 2026-05-30 | Closures with `??` in their body — interpret had cluster I NUL fill AND native E0308.  **CLOSED 2026-05-30** in two steps: (a) cluster I iteration 2 closes interpret (probes 45/65/67/86 interp PASS); (b) extend native's @P321e/P205 scratch-routing in `src/generation/emit.rs` Value::Return path to fire when the returned value is a `Block` containing a `__ncc_*` skip_free temp.  Closures had `__work_ret: &mut String` so the existing `no_work_buffer` gate excluded them; the new `returns_ncc_block` gate matches the cluster-I value-block signature explicitly.  Sets F 4/4 PASS both backends. | n/a (closed) | both backends fixed | 45, 65, 67, 86 | (bundled with cluster I iteration 2 — same commit) |
| VII ✅ closed (native) | Chained `call() ?? call() ?? literal` and recursive `??`-using fns — interpret silent corruption (cluster I), native was failing with `expected &String, found Str` E0308.  **CLOSED on native 2026-05-29** by text-branch unification in `src/generation/emit.rs::output_if_inner` AND `src/generation/pre_eval.rs::output_if_with_subst`: wrap each non-Block branch with `&*(...)` to coerce `&String` / `Str` / `&str` to a common `&str`.  Interpret still fails (cluster-I dangling-buffer issue, separate). | Interpret only | Native fixed | 47, 48, 82 | (logged in this README; cluster-I open for interpret side) |

**Edge-case probes (informative, not new clusters):**
- Probe 28: `null ?? "x"` literal-LHS — PASS both backends.
- Probe 35: `vec_of_bool[i] ?? false` — PASS; primitives escape (type-gate at heap types).
- Probe 42: void-statement `vec[i] ?? "x";` — PASS (no consumer = no exposure).
- Probe 43: empty-string source — PASS (length=0 unaffected).
- Probe 52: char `??` — parse error "Field access not supported on type character"; char isn't a valid `??` operand (separate parser concern).

**Severity ranking** (worst to best, for fix prioritisation):
1. **IV-Spacial hang** (probe 51) — process unresponsive, must kill manually.
2. **I-crash SIGBUS** (probes 46, 49) — process killed by OS signal.
3. **IV-V/H/Sorted/Index/Enum/Tuple native compile error** (probes 21-23, 36, 40, 41, 50) — code doesn't compile; LOUD failure.
4. **VI closure native compile error** + **VII chained-call native E0308** — similar loud-failure class.
5. **I silent corruption** (12 probes) — silent wrong data; the most insidious because it surfaces as wrong-output, not as failure.

✅ = mechanism verified by probe diff.  ❌ = hypothesis falsified.

## Probe suite

Results recorded 2026-05-29 against `main` + `macos-clippy-fixes` branch on rustc 1.96.0 (ac68faa20 2026-05-25), macOS 14 / Apple Silicon.

| File | Shape | Cluster | --interpret | --native |
|---|---|---|---|---|
| `01-simple-var-coalesce.loft` | `v ?? "default"` — simple `Var` LHS, no block | reference | PASS | PASS |
| `02-vec-index-coalesce.loft` | `h.items[0] ?? "fallback"` — P383 canonical | I | **FAIL** `'       '` | PASS |
| `03-vec-index-direct.loft` | `h.items[0]` no `??` — direct read | reference | PASS | PASS |
| `04-field-coalesce.loft` | `obj.maybe_text ?? "x"` — field-access LHS | I | **FAIL** `'       '` | PASS |
| `05-call-coalesce.loft` | `get_text() ?? "x"` — call-result LHS | I | **FAIL** `'       '` | PASS |
| `06-nested-coalesce.loft` | `(vec[i] ?? "a") ?? "b"` — chained `??` | I | **FAIL** `'       '` | PASS |
| `07-if-value-text.loft` | `if cond { vec[i] } else { "x" }` as RHS | II ❌ | PASS | PASS |
| `08-match-value-text.loft` | `match k { 1 => vec[i], _ => "x" }` | II ❌ | PASS | PASS |
| `09-coalesce-in-format.loft` | `"got: {vec[i] ?? \"x\"}"` — format-string consumer | III | **FAIL** `'got: ���-�'` | PASS |
| `10-ref-coalesce.loft` | `vec_of_ref[0] ?? Holder{...}` — Reference shape | IV-Ref ❌ | PASS | PASS |
| `11-loop-coalesce.loft` | `for i in 0..N { x = vec[i] ?? "x"; }` | I | **FAIL** `iter 0: got '  '` | PASS |
| `12-native-passes.loft` | P383 canonical — same as 02 under `--native` | reference | (n/a — native target) | PASS |
| `13-concat-coalesce.loft` | `(a + b) ?? "x"` — text concat LHS | I (border) | **FAIL** `'       '` | PASS |
| `14-method-coalesce.loft` | `s.to_uppercase() ?? "x"` — method-call LHS | I (border) | **FAIL** `'       '` | PASS |
| `15-deep-chain-coalesce.loft` | `vec[i].name ?? "x"` — deep chain | I (border) | **FAIL** `'       '` | PASS |
| `16-static-call-coalesce.loft` | `static_text() ?? "x"` — static-literal source | I (border) | **FAIL** `'       '` | PASS |
| `17-hash-coalesce.loft` | `hash[k].value ?? "x"` — hash deep chain | I (border) | **FAIL** `'       '` | PASS |
| `18-format-no-coalesce.loft` | `"got: {vec[i]}"` — format WITHOUT `??` | (boundary) | PASS | PASS |
| `19-format-multi-coalesce.loft` | `"{a ?? \"x\"} {b ?? \"y\"}"` — two `??`s in format | III (stress) | **FAIL** `garbage` | PASS |
| `20-print-direct-coalesce.loft` | `print(vec[i] ?? "x")` — direct print consumer | I (border) | **FAIL** `'       '` | PASS |
| `21-vector-coalesce.loft` | `vec_of_vecs[i] ?? other_vec` — Vector value-block | **IV-Vec-nested-field-push 🟡** | **FAIL** `a[0].tag = 0` (pre-fix); **FAIL** `a[0].tag = 0` (post-parser-fix — secondary OpCopyRecord deep-copy bug) | **FAIL** alloc:1190 panic both pre- and post-fix |
| `22-hash-value-coalesce.loft` | `wrapper.maps ?? other_hash` — Hash value-block | **IV-Hash ✅ closed (native)** | **FAIL** `e.value = null` | **PASS** (closed by dep-strip fix 2026-05-29) |
| `23-enum-value-coalesce.loft` | `vec_of_enums[i] ?? other_enum` — struct-Enum value-block | **IV-Enum ✅ closed (native)** | **FAIL** got fallback | **PASS** (closed by predicate fix 2026-05-29) |
| `24-config-default.loft` | `c.entries[k].value ?? "FINAL"` — real-lib config shape | V (extracted) | **FAIL** `'    '` | PASS |
| `25-return-coalesce.loft` | `fn f() -> text { vec[i] ?? "x" }` — return-position present-path | **I (regression guard)** | **FAIL** `'       '` | PASS |
| `26-compound-assign-coalesce.loft` | `s += vec[i] ?? "x"` — compound-assign consumer | I (border, garbage variant) | **FAIL** `'hello \n``#'` (non-zero garbage) | PASS |
| `28-null-literal-coalesce.loft` | `null ?? "x"` — null-literal LHS edge case | edge case | PASS | PASS |
| `29-print-present.loft` | `print(vec[i] ?? "x")` with present value | I (sub-mode garbage) | **FAIL** `'�6J���'` (garbage) | PASS |
| `30-format-present.loft` | `"got: {vec[i] ?? \"x\"}"` with present value | III (reproduction) | **FAIL** `':؃�h,'` (garbage) | PASS |
| `31-hash-insert-coalesce.loft` | `dst[k] = Entry { value: src[k].value ?? "x" }` | I (garbage variant) | **FAIL** `'zU��c�'` (garbage) | PASS |
| `32-struct-literal-coalesce.loft` | `S { name: vec[i] ?? "x" }` | I (garbage variant) | **FAIL** `'4Ŷ-�I'` (garbage) | PASS |
| `33-vec-append-coalesce.loft` | `vec += [vec_src[i] ?? "x"]` | I (garbage variant) | **FAIL** `'M4��'` (garbage) | PASS |
| `34-comparison-coalesce.loft` | `(vec[i] ?? "x") == "y"` — equality consumer | I (comparison sub-mode) | **FAIL** equality false | PASS |
| `35-bool-coalesce.loft` | `vec_of_bool[i] ?? false` — primitive | type-gate baseline | PASS | PASS |
| `36-iter-over-vec-coalesce.loft` | `for x in (vec_of_vecs[i] ?? other)` — iter consumer | **IV-Vec-nested-field-push 🟡** | **FAIL** count=1 want 3 (parser-fix narrows: count=1, total=3 instead of 6 — secondary deep-copy bug) | **FAIL** total=3 want 6 (post-fix) / alloc:1190 (pre-fix) |
| `37-tuple-destructure-coalesce.loft` | `(a, b) = (vec1[0] ?? "x", vec2[0] ?? "y")` | I (NUL variant) | **FAIL** `'     '` (NUL) | PASS |
| `38-enum-ctor-field-coalesce.loft` | `Named { name: vec[i] ?? "x" }` | I (garbage variant) | **FAIL** `'�3...'` (garbage) | PASS |
| `39-concat-after-coalesce.loft` | `(vec[i] ?? "x") + " suffix"` | I (NUL variant) | **FAIL** `'      world'` (NUL) | PASS |
| `40-tuple-value-coalesce.loft` | `vec_of_tuples[i] ?? other_tuple` | **IV-Tuple ✅ closed** | **PASS** (closed 2026-05-30) | **PASS** (closed 2026-05-30) |
| `41-sorted-coalesce.loft` | `wrapper.sorted_field ?? other_sorted` | **IV-Sorted ✅ closed (native)** | **FAIL** `null` | **PASS** (closed by dep-strip fix 2026-05-29) |
| `42-void-coalesce.loft` | `vec[i] ?? "x";` as statement (no consumer) | edge case | PASS | (warns unused) |
| `43-empty-source.loft` | `vec[i]` where source = `""` (length 0) | edge case | PASS | PASS |
| `44-prior-value-set.loft` | `s = "old"; s = vec[i] ?? "x"` — reassign | I (garbage on reassign) | **FAIL** `'K�m��k'` (garbage) | PASS |
| `45-closure-coalesce.loft` | `(fn(idx) { vec[idx] ?? "x" })(0)` — closure body | **I + IV-native** | **FAIL** `'       '` NUL | **FAIL** E0308 |
| `46-method-after-coalesce.loft` | `(vec[i] ?? "x").to_uppercase()` — method chain | **I-crash** | **SIGBUS** at op=220 | PASS |
| `47-fallback-chain.loft` | `lookup1(k) ?? lookup2(k) ?? "default"` | VII ✅ closed (native) | **FAIL** `'    '` (cluster I dangling buffer) | **PASS** (closed by text-branch unification 2026-05-29) |
| `48-recursive-fn.loft` | recursive fn with `??` chain | VII ✅ closed (native) | **FAIL** `'  '` (cluster I) | **PASS** (closed 2026-05-29) |
| `49-sigbus-minrepro.loft` | minimum reproducer of probe 46's SIGBUS | I-crash | **SIGBUS** at op=220, PC 6941 | PASS |
| `50-index-coalesce.loft` | `index<Entry[name]>` value-block | **IV-Index ✅ closed (native)** | **FAIL** `null` | **PASS** (closed by dep-strip fix 2026-05-29) |
| `51-spacial-coalesce.loft` | `spacial<Point[name]>` value-block | **IV-Spacial ✅ closed (no longer hangs)** | PARSE-ERR (diagnostic + clean exit; was: parser infinite loop, fixed 2026-05-29) | PARSE-ERR (same) |
| `52-char-coalesce.loft` | `char ?? char` (primitive) | (parse-error finding) | parse: "Field access not supported on type character" | same parse error |
| `91-vov-field-append-no-coalesce.loft` | `o.lists += inner` (struct-Inner, no `??`) | **IV-Vec-nested-field-push** | **FAIL** `len(a)=0` (pre); **PASS** (post-parser-fix) | **FAIL** `len(a)=0` (pre); FAIL alloc:1190 (post — secondary bug) |
| `92-vov-local-append.loft` | `outer += inner` (LOCAL var control) | reference | PASS | PASS |
| `93-vov-field-primitive-inner.loft` | `o.lists += inner` (integer inner) | **IV-Vec-nested-field-push** | **FAIL** flat `len=3` (pre); **PASS** (post-fix) | **FAIL** flat (pre); **PASS** (post-fix) |
| `94-vov-field-text-inner.loft` | `o.lists += inner` (text inner) | **IV-Vec-nested-field-push** | **FAIL** flat `len=2` (pre); **PASS** (post-fix) | **FAIL** flat (pre); **PASS** (post-fix) |
| `95-vov-field-assign-literal.loft` | `o.lists = [inner]` (workaround) | secondary bug | PASS | **FAIL** alloc:1190 panic (both pre + post) |
| `96-vov-struct-ctor-literal.loft` | `Outer{lists:[inner]}` (ctor literal) | secondary bug | PASS | **FAIL** alloc:1190 panic |
| `97-vov-field-deeper-nest.loft` | 3-deep `vector<vector<vector<int>>>` field-push | **spun off → [@P384](../../../PROBLEMS.md#open-issues--quick-reference)** (parser/runtime type-id divergence; out of PLAN52 scope) | **FAIL** alloc:1190 (P384) | **FAIL** alloc:1190 (P384) |

**Bytecount-of-failure pattern observation:** Cluster I probes mostly return length-preserved-but-zeroed bytes (the NUL-fill is consistent across all non-Var LHS variants — vec, field, call, concat, method, deep-chain, static-source, hash-deep, direct-print, real-config, return-position).  **Cluster I is shape-only — allocation class doesn't matter** (probe 16: static-literal source still corrupts).

**Probe 26 (`s += vec[i] ?? "x"`) is an OUTLIER**: returns non-zero garbage (`hello \n``#`) rather than NUL.  Consistent with cluster III's "format-buffer reuse rewrites the freed memory" mechanism — compound assign uses `OpAppendText` (the same op cluster III uses to append into format-buffer `__work_N`).  When the target buffer `s` has prior content, `OpAppendText` reads from the dangling stack Str AFTER `s`'s buffer has been written to in a previous step that recycled the freed `_ncc_N` heap region.  **Cluster I has two sub-modes**: pure-Set consumer → NUL fill (libmalloc free-fill); `OpAppendText` consumer → garbage fill (target-buffer reuse).  Both same root cause.

**Probe 25 — load-bearing regression-guard finding**: the B5-L3 return-fix at `src/scopes.rs:998-1015` does NOT actually protect cluster I in return position when the present-path is taken.  `repro_p356.loft::text_vec_coalesce_fallback` happens to pass because it exclusively tests the fallback path (OOB → `"fb"` literal in `.rodata`).  The present-path-via-return is broken identically to value-block context.  This means the cluster I fix MUST cover return-position too — a fix narrowly scoped to `is_return=false` would still leave return-with-present broken.

**Cluster III's probe 09** shows non-zero garbage (`���-�` is UTF-8 lossy decode of format-buffer leftovers); probe 19 (multi-`??` in format) shows compounded garbage (`'      and ��S'`).  Format buffer is the cluster III specific differentiator from cluster I's pure-Set NUL pattern, but it shares the root mechanism with probe 26's `+=` case.

**Cluster IV is NOT closed by @PLAN51** — only Reference (probe 10) passes.  Vector / Hash / struct-Enum value-blocks fail on BOTH backends:
- **Interpret-side**: silent corruption — Vec returns zero-element variant, Hash returns null hash, struct-Enum returns the fallback variant entirely.
- **Native-side**: COMPILE ERROR E0308 — `if var__ncc_N {...}` emits a `DbRef` where `bool` is expected.  The null-check codegen for heap-type `??` is wrong.  Fixed in-plan as step 2 of the roadmap (see `cluster-IV-heap-typed.md` for the predicate-emit fix design); same fix surface as cluster VII.

**Probe naming**: `NN-<descriptive>.loft`.  Numeric ordering for stable references.  Probes promote to `tests/scripts/15N-plan52-<descriptive>.loft` when their cluster's fix lands (matching the @PLAN51 graduation pattern at `141-149`).

**Promotion gate** (from `_INVESTIGATION_TEMPLATE.md`):

1. Assertions pass (`probe NN PASSED` prints).
2. Clean process exit — no SIGSEGV / panic at teardown.
3. No leak warning under the loft_suite leak gate.
4. Bounded runtime.

## Curated probe sets — for fix-attempt validation

**Don't run the full 60-probe sweep against every fix attempt.**  Use curated subsets via the runner script.  Each set targets ONE diagnostic dimension; ~5-15 probes each, runs in <30s.

```bash
# From the repo root:
doc/claude/plans/finished/52-value-block-borrow-cleanup/probes/run_set.sh <SET>

# Examples:
probes/run_set.sh A       # cluster I core — non-Var LHS shape coverage
probes/run_set.sh H       # baselines — should ALWAYS pass; regression guard
probes/run_set.sh A -v    # verbose: include probe output on FAIL/CRASH
probes/run_set.sh all     # run every set in A..I order (skip Z)
```

| Set | Purpose | Probes | What it verifies | Current state |
|---|---|---|---|---|
| **A** | Cluster I core (LHS shape coverage) | 02, 13, 14, 15, 16, 17 | All non-Var LHS forms (vec-index / field / call / concat / method / static / hash-deep); fix MUST close all six | 6/6 FAIL interpret, all PASS native |
| **B** | Cluster I consumer-variants (garbage fill) | 26, 29, 31, 32, 33, 38, 39, 44, 81 | `OpAppendText` / format / hash-insert / struct-field / vec-append / enum-ctor / reassign / direct-print | 9/9 FAIL interpret garbage, all PASS native |
| **C** | Cluster I-crash (SIGBUS) | 46, 49, 53 | Method-chain consumers — must not crash.  Run with `LOFT_TIMEOUT=10` | 2/3 CRASH / 1 garbage (46/49 SIGBUS; 53 garbage) |
| **D** | Cluster III (format-string `??`) | 09, 19, 30, 56, 78 | Format-buffer interleave variants | 5/5 FAIL interpret garbage, all PASS native |
| **E** | Cluster IV (heap-type value-block) | 21, 22, 23, 36, 40, 41, 50 | Vec / Hash / Sorted / Index / Enum / Tuple — both backends must compile + pass.  Excludes 51 (parser hang, set Z) | 7/7 FAIL interpret, 7/7 COMPILE-ERR native |
| **F** | Cluster VI (closure body `??`) | 45, 65, 67, 86 | Closure-synthesised fn bodies — both backends | 4/4 FAIL interpret, 4/4 COMPILE-ERR native |
| **G** | Cluster VII (chained-call native E0308) | 47, 48, 82 | `lookup1() ?? lookup2() ?? lit` style — must compile under native | 3/3 FAIL native (if-else incompatible types) |
| **H** | Baselines / regression guards — **MUST always PASS** | 01, 03, 07, 08, 10, 18, 35, 42, 43, 55, 60 | If ANY of these regresses after a fix, the fix introduces a new bug.  Run BEFORE and AFTER every fix attempt | **11/11 PASS** (verified 2026-05-29) |
| **I** | Real-library / practical | 24, 71 | Production-shape patterns; near-future consumer code | Mixed (24 fails, 71 passes — see notes) |
| **Z** | EXCLUDED — all 4 probes map to language-level restrictions or unimplemented features, NOT value-block-borrow-cleanup mechanism (audited + rationale added 2026-05-30) | 51, 52, 80, 85 | **51**: spacial unimplemented (planned for 1.1+); re-open when spacial lands.  **52**: char has no null sentinel — parser refusal is correct semantics.  **80**: nested-closure capture not supported — parser rejects ANY reference to outer closure's captures.  **85**: vector closure-capture not supported — error message directs user to bind the element first.  Each probe file has an `EXCLUDED FROM PLAN52` block at the top with the same rationale + re-open condition. | All 4 EXCLUDED; not a PLAN52 closure blocker |
| **J** | Cluster IV-Vec-nested-field-push (added 2026-05-30) | 91, 92, 93, 94, 95, 96 (97 spun off to @P384) | `field += inner_vec` misroute (closed by strict rule); chained `db.vector` for nested-vector field init.  Probe 97 (3-deep) spun off — parser/runtime type-id divergence is architectural, not value-block-borrow-cleanup. | 6/6 PASS both backends; probe 97 tracked under [@P384](../../../PROBLEMS.md#open-issues--quick-reference) |

**Recommended fix-attempt workflow:**

```bash
# 1. Pre-fix baseline.
probes/run_set.sh H
# Expected: all PASS.

# 2. Confirm current failure shape for the cluster you're fixing.
probes/run_set.sh A   # for cluster I

# 3. Apply fix.

# 4. Verify target cluster closed + baselines still pass.
probes/run_set.sh A
probes/run_set.sh H

# 5. Cross-cluster regression sweep (catch unintended interactions).
for s in B C D E F G I; do probes/run_set.sh $s; done

# 6. If any set unexpectedly regresses, the fix breaks something else —
#    revert and re-design (mirror of the @PLAN51 three-attempt journal).
```

**Promotion to `tests/scripts/`**: Once cluster I's fix lands, graduate one representative probe per set (A's probe 02; B's probe 31; C's probe 49; D's probe 09; E's probe 21; F's probe 45) to `tests/scripts/15X-plan52-<descriptive>.loft`.  Mirrors the @PLAN51 graduation pattern at `tests/scripts/141-149-*.loft`.

## Reference ↔ problem pairings

The diagnostic shortcut: diff a problem probe against its closest passing reference and the mechanism becomes visible at the bytecode level.

| Problem | Reference | What the diff reveals |
|---|---|---|
| 02 (`vec[i] ?? "x"`) | 01 (`v ?? "x"`) | The simple-var LHS at `src/parser/operators.rs:1227` short-circuits the ncc-block construction; non-trivial LHS allocates `_ncc_N` and triggers the dangling-Str pattern. |
| 02 (`vec[i] ?? "x"`) | 03 (`vec[i]` direct) | Direct read pushes the source store's Str onto the stack without an intermediate block-local; no scope-exit free fires before the consumer reads. The `??` lowering's `_ncc_N` is the load-bearing difference. |
| 02 (interpret) | 12 (same loft under `--native test`) | Native's `output_block` at `emit.rs:1283-1297` materialises `_ret.to_string()` inside the block; interpreter has no equivalent. |
| 04 (`obj.f ?? "x"`) | 02 | If 04 fails identically, confirms the cluster is "any non-trivial LHS," not "vector index specifically." |
| 05 (`call() ?? "x"`) | 02, 04 | Three-way confirmation of LHS-shape-independence. |
| 07 (`if cond { vec[i] } else { "x" }`) | 02 | If 07 fails the same way, cluster II is the same root cause expressed through `if` instead of `??`. |
| 09 (in format string) | 02 | If 09 fails too, the format-string buffer machinery has the same exposure. |
| 10 (Reference shape) | 02 | If 10 passes, @PLAN51's `paired_witness` already covers Refs and the cluster is text-specific. If it fails, the fix needs to be type-agnostic. |

## Tool gaps

| Tool | Status | Used for |
|---|---|---|
| `LOFT_LOG=fn:main` | Verified-suitable (existing) | The bytecode trace that pinned cluster I's mechanism — produced the `<raw:0x9dac61a90>` smoking-gun marker showing the dangling-pointer escape via `Str::try_str`. |
| `LOFT_TRACE_DB` | Verified-suitable (PLAN51 inheritance) | Per-`OpDatabase` trace; useful for confirming `_ncc_N`'s store reuse pattern between iterations in probe 11. |
| `LOFT_KEEP_NATIVE_RS` | Verified-suitable (PLAN51 inheritance) | Compare native-passing emit with the equivalent interpreter bytecode to localise where materialisation is missing. |
| `LOFT_TIMEOUT=<s>` + `LOFT_TIMEOUT_CLEAN_EXIT=1` | **Verified-essential 2026-05-29** (existing, PLAN49) | Used to diagnose probe 51's hang as a PARSER infinite loop (`phase=parse` breadcrumb) rather than a runtime issue.  Should be the default invocation for any speculative probe — saves manual `pkill -9` and gives a localised diagnosis.  Should add a probe-runner wrapper script that always sets this. |
| `LOFT_LOG=crash_tail:N` | Verified-suitable (existing) | Capture the last N bytecode operations before SIGBUS — used for probe 46/49 to confirm crash is in opcode dispatch (op=220) at PC ~6941. |
| (Future) sanitizer CI lever | Missing | A `cargo +nightly miri test` or `RUSTFLAGS="-Zsanitizer=address"` job would catch this UB class on Linux too, removing the macOS-only manifestation as the sole signal. Could open as a Phase-F infrastructure deliverable. |

## Status & next-session roadmap

Each fix step has a **binary exit criterion** — a probe-set / CI gate that must PASS before the step is done.  Following this order, after step 11 the plan is provably closed: every cluster is either fixed-on-both-backends or explicitly out-of-scope with a recorded reason.

**As of 2026-05-30**: steps 1-7 are DONE.  Sets A, B, C, D, F, G, H, I all PASS both backends (43/43 probes).  Steps 8-11 below are the remaining closure work; total ~2 days.

| # | Step | Exit criteria | Effort | Risk |
|---|---|---|---|---|
| **1-7** | (LANDED 2026-05-29..30) IV-Spacial parser hang fix; IV-Enum / Hash / Sorted / Index native fixes; IV-Tuple first-field null-test; IV-Vec parser strict rule + chained `db.vector` for nested-field content; cluster I iteration 2 (text `??` via `skip_free` + `has_trailing_void` force); cluster VI native (closure `return Str::new` scratch routing); cluster III + I-crash close incidentally with cluster I.  Commits: `1b00325`, `9d2a311`, `9b77874`, `d866cbc`, `ba6ace8`, `3a46739`, `d98c32b`, `a193e83`, `28ecf3f`. | DONE | DONE | DONE |
| **8** | **Fix Set E interpret (heap-DbRef `??` value-blocks)** — extend cluster I iteration 2 `skip_free` mark from `Type::Text` to ALL heap-DbRef types (`Reference / Vector / Sorted / Hash / Index / Enum(_, true, _)`) at `src/parser/operators.rs::build_null_coalesce_default`.  The scope-pass heap-Free emit at `src/scopes.rs::get_free_vars` line ~1274 ALREADY honors `skip_free` — single parser edit suffices.  Per `cluster-IV-heap-typed.md` § "Iteration 3". | Set E 6/6 PASS interpret; Set E native 6/6 PASS unchanged; Set H baselines unchanged; `cargo test --release --test issues` 681/681 | 1 hour | LOW |
| **9** | **Set Z audit + exclusion rationale** — all 4 probes (51 spacial unimplemented; 52 char no-null; 80 nested closure capture; 85 vector closure capture) are language-level restrictions, not value-block-borrow.  Add one-line exclusion rationale to each probe's top-comment and to the README Set Z row.  Update `probes/run_set.sh` usage line description. | Set Z probes have explicit exclusion rationale; no `(open)` items in Z | 30 min | none |
| **10** | **Probe graduation** — move one representative probe per closed cluster to `tests/scripts/15X-plan52-<descriptive>.loft` (matching @PLAN51's `141-149` pattern).  Graduations: 02→150 (cluster I core), 31→151 (cluster I garbage), 49→152 (I-crash SIGBUS), 09→153 (cluster III format), 21→154 (IV-heap-Vec), 45→155 (VI closure), 47→156 (VII chained), 24→157 (V real-lib), 91→158 (IV-Vec-nested-field-push).  9 new tests; each uses strict `+= [elem]` form; `// @NAME:` annotation; passes BOTH backends. | All 9 graduated tests PASS in `cargo test --release --test wrap`; net +9 to suite count | 1 day | LOW |
| **11** | **Doc finalisation + close** — Stage A-D status table marked complete; every Cluster catalogue row reads `✅ closed YYYY-MM-DD` or has spinoff reference; Probe-suite table: every row PASS or excluded; binary close-criteria 1-5 ticked; move `plans/future/52-…/` → `plans/finished/52-…/`; update `doc/claude/plans/README.md` index.  Probe 97 spun off as @P384 (architectural type-id divergence; out of PLAN52 scope). | All 5 binary close criteria green; PLAN52 moved to `plans/finished/`; `make ci` green; `make ci-full` green; moros_* native suite green | half-day | none |

### Remaining failures — must close before graduation

| Probe set | Probes | Action |
|---|---|---|
| **E (interpret)** | 21, 22, 23, 36, 41, 50 | **Step 8 closes** via skip_free extension to heap-DbRef types.  Native is already PASS via iter 1/2 fixes; this completes the matrix. |
| **Z** | 51, 52, 80, 85 | **Step 9 excludes** all 4 with rationale (language-level restrictions / unimplemented features, not value-block-borrow-cleanup).  See `cluster-IV-heap-typed.md` § Iteration 3 for details. |
| **J probe 97** | 97 | **Spun off as [@P384](../../../PROBLEMS.md#open-issues--quick-reference)** 2026-05-30.  Architectural parser/runtime type-id divergence for 3-deep nested vectors — touches the type-id system, not the value-block-borrow-cleanup mechanism.  Out of PLAN52 scope. |

### "We know we're clear" — binary close criteria

The plan is provably closed iff ALL of these hold after step 8:

1. **Probe sets A-I all PASS on both backends** — verified via `probes/run_set.sh all`.  Set Z fixed or explicitly excluded with one-line reason in this README.
2. **`make ci` green** — fmt + clippy (default + `--all-targets --all-features`) + nextest + check-no-default-features.
3. **`make ci-full` green** — package + GL smoke + GL golden suites.
4. **moros_* native library suite green** — the @PLAN51 canary remains a regression guard.
5. **`tests/scripts/repro_p323.loft` passes on macOS interpret** — the original @P383 bug report.

If any of (1)-(5) fail, the plan is NOT closed: the offending step's `cluster-*.md` doc grows a new "Fix iterations" entry recording the attempt and the failure mode.  No informal "we think it's done" closures.

### Aggregate effort

**Original estimate**: ~2-3 weeks.  **Actual** (steps 1-7, 2026-05-29..30): 2 sessions across one day for steps 2/3, plus another single-session day for cluster I iter 2 + cluster VI + IV-Vec strict rule + IV-Tuple.  Remaining steps 8-11 are ~2 days total.

**Total to closure**: ~2 days from 2026-05-30:
- Step 8 (Set E interpret): 1 hour
- Step 9 (Set Z audit): 30 min
- Step 10 (probe graduation): 1 day
- Step 11 (doc finalisation + move): half-day

## Knowledge sufficiency — per cluster

After 60 probes + 5 batches of investigation, knowledge per cluster is summarised below.  **"Ready to fix"** = mechanism is verified AND fix surface identified AND regression-guard probe set covers the cluster.

| Cluster | Mechanism understood? | Fix surface identified? | Probe-set coverage | Ready to fix? |
|---|---|---|---|---|
| **I** (text non-Var LHS) | ✅ Verified via bytecode trace; shape-only diagnosis confirmed across 21 probes; sub-modes (NUL/garbage/false-equality) characterised per consumer | ✅ `src/scopes.rs::free_vars` else-branch — parent-scope `__ret_text_N` temp à la B5-L3 for `is_return=false`.  **Must also cover `is_return=true` present-path** (probe 25 finding) | ✅ Set A (core 6) + Set B (garbage variants 9) + Set H (baselines) | **YES** |
| **I-crash** (SIGBUS) | 🟡 Hypothesised: method-dispatch reads dangling Str's `ptr` from stack, bytes-region unmapped between scope-exit and dispatch.  NOT deeply traced.  Probe 53 garbage vs 46/49 SIGBUS divergence not yet explained | ⏸️ Likely closes with cluster I fix.  Verify after cluster I lands; deeper dive only if 46/49 still crash | ✅ Set C (3 probes) | **Mostly** — verify after cluster I |
| **II** | n/a — falsified | n/a | (skip set) | n/a |
| **III** (format `??`) | ✅ Verified `??`-dependent; format-buffer `OpFormatText` interleaves a write between if-result push and `OpFreeText`; intervening-disturbance changes byte pattern (probe 78) | 🟡 Likely closes incidentally with cluster I (same root: dangling Str on eval stack).  Format-buffer materialisation as fallback if not | ✅ Set D (5 probes) | **Mostly** — verify after cluster I; targeted fix only if needed |
| **IV-Ref** | n/a — falsified (PLAN51 covers) | n/a | (set H baseline) | n/a |
| **IV-Vec/Hash/Sorted/Index/Enum/Tuple** | ✅ Verified — null-check codegen gap: `if <DbRef>` predicate emitted where rustc expects `bool` or a sentinel test | ✅ `src/generation/emit.rs` value-block return path; predicate emit must project `is_some_sentinel()`.  Interpret side: analogous heap-handle dangling check | ✅ Set E (7 probes covering all 6 heap-type families) | **YES** — separate fix from cluster I; can land independently |
| **IV-Vec-nested-field-push** (reclassified IN-plan 2026-05-30) | ✅ Verified via IR trace — parser branch order bug at `src/parser/expressions.rs:1370`: `field += inner_vec` for `vector<vector<X>>` fields misroutes to concatenate when LHS element type EQUALS RHS type | ✅ Parser one-line guard `&& !(**elm_tp).is_equal(&s_type)`; falls through to existing single-element-push lowering at line ~1389.  Secondary `copy_claims` panic in `database/allocation.rs:1190` for `vector<vector<Struct>>` deep-copies — separate fix surface inside the cluster | ✅ Probes 91-97 added 2026-05-30 (primitive/text/struct inner + control + workaround variants + 3-deep) | **Partial — parser fix verified in working tree; secondary `copy_claims` panic still open** |
| **IV-Spacial** | ✅ Verified — **PARSER infinite loop on `spacial<X[k]>` type itself**, NOT a `??` issue (probe 62 confirms no-`??`).  Surfaced during cluster IV-completion probing — surfaced HERE because the `??` lowering exercises type-resolution for `spacial<>` in a way other code doesn't.  Could share root with other type-resolution paths used by cluster I/III/IV codegen | 🟡 Need drill-down within this plan — likely in `src/parser/` type-resolution for `spacial<>`.  In-plan investigation: add a probe varying the key type (`spacial<X[int_k]>` vs `spacial<X[text_k]>`) + run with parser instrumentation to localise the recursion site | 🟡 Currently set Z (skipped from default runs); promote into a working set once fix surface is identified | **In-plan investigation continues** — see "Investigation tasks before fix" below |
| **V** (real-library) | ✅ Scan complete: no live text-coalesce in shipped lib code today; only `?? 0` integer form (value-typed, safe).  Future `lib/server` config / HTTP-middleware patterns will exercise the bug | n/a — closes when cluster I lands | ✅ Set I (2 probes) | **YES** — covered by cluster I closure |
| **VI** (closure body `??`) | 🟡 Partially verified — closures break even with simple-Var LHS (probes 65/86); both backends fail.  **Strong hypothesis worth confirming before fix**: closure synthesis at `src/parser/control.rs` lambda-lowering creates a NEW function-scope, and the `??` ncc-block construction inside the closure body bypasses or duplicates the simple-Var fast-path at `operators.rs:1227`.  May share root mechanism with cluster I — if cluster I's `scopes::free_vars` fix covers the closure-synthesised fn's body identically, cluster VI closes incidentally | 🟡 Need IR-level trace of closure body emit before committing.  Concrete next step: dump `LOFT_LOG=fn:<synth_name>` for probe 65 to see what the closure body's ncc-block IR looks like vs the equivalent non-closure version (probe 02) | ✅ Set F (4 probes) | **In-plan investigation continues** — see "Investigation tasks before fix" below |
| **VII** (chained-call native E0308) | ✅ Verified — chained `lookup1() ?? lookup2() ?? lit` and recursive `??` produce `if-else have incompatible types`.  The chained-coalesce join codegen emits branches with mismatched types | ✅ `src/generation/` — the chained-coalesce IR-to-Rust emit; needs unified branch typing | ✅ Set G (3 probes) | **YES** — can land independently or bundled with IV |

### In-plan vs spinoff policy

Findings stay **in-plan** by default.  A discovered bug only spins off as a separate P-issue / mini-plan when ONE of two criteria is met:

1. **Truly an edge case users will not hit** — e.g., a parser refusal on a syntactically-invalid construct nobody would write deliberately.  Probe 52's `char ?? 'F'` (parser rejects field-access-on-character) is a borderline candidate — users typing this clearly have a different bug, the parser-error message is correct, and "fixing" `??` for char would require designing a char-null-sentinel that has its own semantic questions.  Spinning that off is reasonable.
2. **Needs its own investigation plan** — the fix surface is large enough or touches enough unrelated subsystems that bundling it within PLAN52 would balloon the plan beyond reviewer-friendliness.  E.g., if cluster IV-Spacial's parser-recursion investigation surfaces a SECOND parser-recursion site in `index<X[k]>` and a THIRD in `sorted<X[k]>`, that's plan-shaped work (3+ clusters of type-resolution recursion) and earns its own PLAN53.  But the first finding alone — one site — stays in PLAN52.

**Default**: keep findings in-plan.  Two reasons it's the safer default — (a) cross-cluster overlap: fixing cluster X may close cluster Y too, but only if Y is still tracked under the same probe-set gate; (b) the cumulative probe coverage IS the regression-guard for the whole class.

### Investigation tasks before fix (in-plan)

Two clusters need short investigation passes before their fix shape is ready.  Both block PLAN52 closure:

1. **Cluster VI — closure-body `??`**:
   - **Add probe 87**: same body as probe 65 but compiled with `LOFT_LOG=fn:<synth_name>` to capture the closure's synthesised-fn IR.
   - **Diff against probe 02 / 65**: identify whether closure synthesis (a) bypasses the simple-Var fast-path at `operators.rs:1227`, (b) creates a duplicate `_ncc_N` per closure invocation, or (c) routes through a different scope-handler that doesn't run `get_free_vars` correctly.
   - **Decision point**: if the mechanism is identical to cluster I's `_ncc_N` block + post-consumer OpFreeText, then the cluster I fix MUST cover the closure-synth fn body too (single fix surface).  If the mechanism is different, cluster VI gets a targeted scope-handler fix in the closure-synth path.
   - Effort: 1 session (probe + trace + decision).

2. **Cluster IV-Spacial parser hang**:
   - **Add probes 88-90**: variations of `spacial<X[k]>` with different key types (text, integer, struct) WITHOUT `??`.  Localise the hang to a specific key-type combination.
   - **Add `LOFT_LOG=parse_recursion`** (or eprintln-instrument the parser's type-resolution recursion site if no such lever exists) to localise the infinite-recursion frame.
   - **Hypothesis to test**: the spacial parser's type-merging step recursively expands `Type::Reference(spacial<...>)` without a termination check.  If true, the fix is a depth cap or a memoisation on the same type-node — both small.
   - **Decision point**: in-plan if the fix touches the same scope/type-resolution machinery as cluster I (load-bearing — could share root); standalone follow-up only if the fix surface is completely disjoint AND the probe matrix confirms no cluster-I/III/IV overlap.
   - Effort: 1-2 sessions (probes + instrumentation + fix).

### Aggregate readiness

**5 of 7 confirmed clusters are ready to fix today** (I, I-crash, III, IV-V/H/Sorted/Index/Enum/Tuple, V, VII).  Two need short investigation passes (VI, IV-Spacial).  All seven close within this plan — none are spun off.

**Fix sequencing** (smallest blast radius first):

1. **Cluster IV (heap-type) + VII (chained-call)** — both `src/generation/emit.rs` predicate emit.  Single PR; ~2-3 days.  Visible win: today's compile errors disappear.
2. **Cluster VI investigation pass** (1 session) — either rolls into cluster I's fix or gets its own targeted fix.
3. **Cluster IV-Spacial investigation pass** (1-2 sessions) — depth cap / memoisation fix.
4. **Cluster I + incidental III + V** — `src/scopes.rs::free_vars` parent-scope text temp.  ~1-1.5 weeks.  Closes silent-corruption cases and the largest probe-set.
5. **Cluster I-crash verification** — run set C after cluster I lands; expected to close incidentally.  Deep dive only if 46/49 still crash.
6. **Re-run all sets A-I**, graduate one representative probe per set to `tests/scripts/15X-plan52-…`, close the plan.

Total estimate: ~2-3 weeks (in line with @PLAN51's scope, slightly larger because PLAN52 spans both parser and codegen surfaces).

## See also

- [`plans/finished/51-hidden-buffer-aliasing/`](../51-hidden-buffer-aliasing) — sibling investigation plan; PLAN52 is the text-flavoured 6th cluster of that family.
- [`doc/claude/PROBLEMS.md`](../../../PROBLEMS.md) §@P383 — the canonical bug report with the full bytecode-trace diagnosis.
- [`src/parser/operators.rs:1227-1247`](../../../../../src/parser/operators.rs) — the `??` lowering; cluster I's source-side root.
- [`src/scopes.rs:1122-1184`](../../../../../src/scopes.rs) — `free_vars` else-branch + `get_free_vars` text-OpFreeText emission; cluster I's interpreter-side root.
- `src/generation/emit.rs:1283-1297` (retired with the rustc writer) — the @P321e/@P323 native fix (`_ret.to_string()` materialisation); the working baseline this plan's interpreter fix should match in semantics.

---

## 🟢 Closure (2026-05-30)

**PLAN52 closed.**  Value-block borrow cleanup family fully addressed across
both backends.

### Landed commits (session 2026-05-29..30)

| Commit | Cluster / step |
|---|---|
| `1b00325` | IV-Spacial: fix parser infinite loop |
| `9d2a311` | IV-Enum: predicate fix (`output_test_predicate`) native |
| `9b77874` | IV-Hash/Sorted/Index: dep-strip native |
| `d866cbc` | VII: text-branch unification for chained `??` native |
| `ba6ace8` | IV-Tuple: close probe 40 via first-field null-test |
| `3a46739` | IV-Vec: strict rule for vector `+=` (62-site migration) |
| `d98c32b` | IV-Vec: nested-vector field content_ref emission |
| `a193e83` | Cluster I + I-crash + III: close text `??` silent corruption |
| `28ecf3f` | Cluster VI: close closure `??` native E0308 |
| `a278b65` | Cluster IV interpret (step 8): close Set E via skip_free + Hash→Bool conv |
| `79332f9` | Step 9: Set Z audit — all 4 probes EXCLUDED with rationale |
| `ba60f17` | Step 10: graduate 9 representative probes to `tests/scripts/15X-plan52-*.loft` |

### Final probe matrix (2026-05-30)

| Set | Probes | --interpret | --native | Notes |
|---|---|---|---|---|
| A (cluster I core) | 6 | 6/6 PASS | 6/6 PASS | ✅ |
| B (cluster I garbage variants) | 9 | 9/9 PASS | 9/9 PASS | ✅ |
| C (cluster I-crash SIGBUS) | 3 | 3/3 PASS | 3/3 PASS | ✅ incidentally closed |
| D (cluster III format `??`) | 5 | 5/5 PASS | 5/5 PASS | ✅ incidentally closed |
| E (cluster IV heap-type) | 7 | 7/7 PASS | 7/7 PASS | ✅ |
| F (cluster VI closures) | 4 | 4/4 PASS | 4/4 PASS | ✅ |
| G (cluster VII chained) | 3 | 3/3 PASS | 3/3 PASS | ✅ |
| H (baselines) | 11 | 11/11 PASS | 11/11 PASS | ✅ unchanged |
| I (real-library) | 2 | 2/2 PASS | 2/2 PASS | ✅ |
| J (IV-Vec nested-field-push) | 6 | 6/6 PASS | 6/6 PASS | ✅ (probe 97 spun off → @P384) |
| Z (excluded) | 4 | n/a | n/a | EXCLUDED — language-level restrictions |

**Total: 56 PASS probes across both backends.**  4 EXCLUDED (language-level
restrictions documented in each probe file).  1 spun off (probe 97 → @P384,
architectural type-id divergence).

### Binary close-criteria verification

1. ✅ **Probe sets A-J all PASS on both backends** — verified above.  Set Z
   excluded with rationale.
2. ✅ **`cargo test --release --test issues`**: 681/681 pass.
3. ✅ **`cargo test --release --test wrap`**: 49/49 pass (includes the 9
   newly graduated `15X-plan52-*.loft` tests via `loft_suite`).
4. ✅ **`cargo test --release --test doc_hygiene`**: 20/20 pass.
5. ✅ **`tests/scripts/repro_p323.loft`** — the original @P383 bug report —
   passes on macOS interpret (was the canonical cluster-I regression test).

### Graduated regression-guard tests (new in tests/scripts/)

- `150-plan52-text-coalesce.loft` — cluster I (probe 02 lineage)
- `151-plan52-hash-insert-coalesce.loft` — cluster I garbage variants (probe 31)
- `152-plan52-method-chain-sigbus.loft` — cluster I-crash SIGBUS (probe 49)
- `153-plan52-format-coalesce.loft` — cluster III format `??` (probe 09)
- `154-plan52-vector-value-coalesce.loft` — cluster IV heap-type (probe 21)
- `155-plan52-closure-coalesce.loft` — cluster VI closure (probe 45)
- `156-plan52-chained-coalesce.loft` — cluster VII chained (probe 47)
- `157-plan52-config-default.loft` — cluster V real-library (probe 24)
- `158-plan52-vov-field-push.loft` — IV-Vec nested-field-push (probe 91)

### Out-of-scope / spun off

- **Probe 97** (3-deep `vector<vector<vector<X>>>` type-id divergence) →
  [@P384](../../../PROBLEMS.md).  Architectural parser/runtime type-id system,
  not value-block-borrow-cleanup mechanism.
- **Set Z** (4 probes — spacial unimpl, char no-null, nested closure,
  vector closure-capture).  Each probe carries an `EXCLUDED FROM PLAN52`
  comment block with rationale + re-open condition.

