<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_HOTSPOTS.md — the structures that will keep manufacturing bugs

> Open H-items are ORDERED in [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) —
> the single tracking view across all stability docs.  This file stays the
> canonical home for each hotspot's invariant, evidence, and mitigation design.

> **Cross-cut view:** [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) groups the
> same designs (and two NEW ones — the container-traversal `for_each_owned_child`
> keystone and the `gen_if`/`size_code` wrong-signal siblings) by the **missing
> fact** each re-derives, with leverage-first landing order.  Read it for "which
> one fact, computed once, collapses N forests"; read this file for each
> hotspot's standalone invariant + mitigation.

Companion to [STABILITY_METHOD.md](STABILITY_METHOD.md) (the three-pass
method) and [STABILITY_PASS2.md](STABILITY_PASS2.md) (the executed
relocation pass).  Where those documents look at *routines*, this one looks
at *designs*: the places where the 2026-06 bug history (#299, #306, #313,
#328, #330, #336, #339, @P364, @P377, plan-57's tail) clusters so tightly
that the next dozen bugs are predictable.  Each hotspot names the violated
invariant, the evidence, and the mitigation work — sized, ordered, with its
validation gate.

Assessment date: 2026-06-11 (branch `bugs325`; 5 open issues, all
`fixed-pending-merge`).  The codebase is in its best state so far — this
document is about keeping it that way as features land.

## Reading the sizes

`S` = under a day · `M` = days · `L` = a plan with phases (design doc +
quiet window).  Every L item must run through
[DESIGN_PROTOCOL](DESIGN_PROTOCOL.md) (name the invariant, count
re-assertion sites, falsify the load-bearing claims) before code.

## Landing order (recommendation)

```
H1 (signature ABI)  ──── unlocks ────▶  H5 (deletes most of the contract)
H2 (typed deps)     independent, do early — converts corruption to compile errors
H6 (sentinel codec) independent, S-sized — do in any gap
H7 (codec derivation / F9)  before the next Value/Type variant lands
H3 (ownership as data)  AFTER H1+H2 (both shrink its surface)
H8 (allocations privacy)  the already-planned privacy pass
H9 (LoftStore FFI handle)  with H8 — both harden the raw Stores surface
H4 (backend unification)  continuous discipline now; structural fix is 1.1+ scale
```

---

## H1 — Analysis-dependent function arity (the #1 future bug factory)

**The violated invariant**: *a function's signature should be a pure
function of its declaration.*  Today `ref_return`
(`src/parser/control.rs`) GROWS a fn's attribute list during parsing,
based on NRVO analysis of its body — which depends on its callees'
(also growing) signatures.  That is a call-graph fixpoint being
approximated by "two passes + cross-pass name stability + a retro-patch".

**Evidence**: #299 (wrapper returns aliased buffer), #306
(attr-index/var-number dep confusion), @P364/@P377 (collapse hazards over
the hidden-buffer IR), #339 (late pass-2 growth breaks earlier callers).
#339's fix history is the strongest signal: two simpler fixes failed in
opposite directions — freezing signatures after pass 1 silently
re-introduced #299's corruption (promotion is load-bearing), and a
conditional third parse pass segfaulted `100-enhancements` (per-fn
variable tables are name-stable by design, not re-entrant).  When one bug
has three competing wrong fixes, the design is the bug.

**Mitigation — unconditional heap-return ABI (L)**

Make the hidden return-buffer parameter a property of the RETURN TYPE, not
of body analysis: every fn returning `Reference` / `Vector` /
struct-`Enum` gets the hidden buffer attr at declaration-parse time,
always.

1. Design doc (plans/ slot): enumerate every consumer of "does this fn
   have a hidden buffer?" — `add_defaults`, `collect_hidden_ref_args`,
   `nrvo_collapse_tail_set`, `filter_hidden`, codegen's `OpReturn` width
   logic, the cdylib export wrappers (`native_lib.rs` marshals by
   attribute list), fn-ref dispatch, `introspect`.  Count the
   re-assertion sites per DESIGN_PROTOCOL.
2. Add the attr unconditionally in `parse_fn_signature` (pass 1, before
   any body parses).  `ref_return` shrinks to: decide what flows INTO the
   already-existing buffer (NRVO yes → callee builds in place; NRVO no →
   the copy path fills it).  No `add_attribute` outside signature parse.
3. Delete the machinery the change strands: `retrofit_callers_hidden_args`
   (#339's fix becomes dead), the `__rref_` recursive-self dance in
   `add_defaults` (arity can no longer differ between passes), the
   `signatures-must-not-grow` comment contracts.
4. Cost check: fns that today DON'T promote (reassigned locals, literal
   tails) would now carry an unused buffer param → one extra `DbRef` arg
   per call.  Probe the perf delta on the benchmark suite
   ([PERFORMANCE.md](PERFORMANCE.md)); if measurable, the buffer can be
   passed as the null sentinel and lazily allocated — the ABI stays
   uniform either way.
5. Validation: full matrix on both backends + wasm; the #299/#339/#306
   regression files; crawler + moros + brick-buster end-to-end; cdylib
   dispatch tests (`native_library_suite`) — the C ABI of exported
   wrappers changes shape, so `make rebuild-native-cdylibs` artifacts and
   the registry's `verified` libraries need a re-verify pass.

**Status (2026-06-11, end of day): PHASE 1 SHIPPED** — every
body-carrying plain fn returning Reference / Vector / struct-Enum
carries its hidden `__retbuf` from signature parse; `ref_return` binds
promoted locals by role swap; arity can no longer grow behind a parsed
caller (debug-asserted; lambdas keep in-place growth — no earlier
callers can exist).  The dispatcher census that gated it is complete:
plain calls, par lanes (+ runtime witness-free), entry invocations
(REPL capture), and the cdylib shared bridge (runtime type-name ids)
all speak the uniform ABI.  Full matrix green —
[plans/55-return-abi](plans/55-return-abi/README.md) records the three
rounds, every probe, and the phase-2 cleanups.  Originally: plan opened —
[plans/55-return-abi](plans/55-return-abi/README.md) (@PLN55).  Phase 0
SHIPPED: the H1 census probe caught a LIVE #339 sibling (vector-literal
tails promote late too; 7-line caller-first repro panicked on main) — the
retrofit now covers all three heap-buffer kinds, regression in
`tests/scripts/295`.  Census results validate the one-buffer design
(ls ≤ 1 in 104/104 promotions); claims C1–C5 probed/read, C6–C8 (argument
order ↔ attr order coupling, buffer-vs-return-value consumption,
null-init suppression) are the named pre-implementation probes for
phase 1's own window.

**Until then** (standing rule, loft-write skill): a thin wrapper around a
struct-returning fn is safe ONLY if the wide impl is defined before it in
the file; the widened retro-patch covers the rest but new shapes (fn-refs
to late-promoted fns, generics) should be probed before relying on them.

---

## H2 — The `Vec<u16>` dep-list overload

**The violated invariant**: *one type, one meaning.*  The dep vector on
`Type::Reference/Vector/...` means, depending on context: caller variable
numbers, callee ATTRIBUTE indices, the `u16::MAX` pointer-share marker
(#328), a self-dep-as-ownership-marker (@P302 keyed locals), or
"empty = owned".  Readers must know which meaning the writer intended;
nothing checks it.

**Evidence**: #306 (attr indices read as var numbers → caller store freed
under a borrow), #328 (`u16::MAX` marker invented because nothing else
could express pointer-ness), #330 (a self-dep silently flipped codegen to
`InitCreateStack` — the SIGSEGV took a session to trace), `add_defaults`'
"#306: strip the callee-internal dep list" comments at three sites (each
a manual re-assertion of the convention).

**Mitigation — typed dep semantics (M)**

Step 1 DONE (2026-06-11): **[DEPS_INVENTORY.md](DEPS_INVENTORY.md)** —
the semantic model (two address spaces + five marker overloads, incl. TWO
different `u16::MAX` meanings), all 84+ reader/writer/converter sites
classified, the crossing sites named, and a corpus probe showing the
out-of-range guards in `is_borrowed_view` / `dep_has_var` are fossil
defenses (zero contamination in stdlib + moros + crawler + scripts).
Steps 2–4 DONE (2026-06-11): `Deps` is a constructor-checked NEWTYPE —
every creation site states its meaning (`none` / `frame` / `attrs` /
`pointer_marker` / `share_sentinel` / `unknown`), reads go through
`Deref` or the space-asserting accessors (`frame_vars` /
`as_attr_indices`, debug-tag-checked, zero release cost),
`resolve_deps`/`ref_return` are typed as THE converters, and contaminated
reads scream in debug.  The step-3 bisect upgraded the inventory: the
`dep_has_var` "fossil" was LIVE — block-result deps were mixed-space by
contract (in-range = attr index, out-of-range = frame var; removing the
arm made `26-closures`' factory results share one record).
Step 5 DONE (2026-06-12): the positional contract is RETIRED — the
callee-frame note a closure factory stores in `def.returned` is an
in-band TAGGED value (`Deps::CALLEE_FRAME_BIT`; sole writer
`Deps::callee_frame1` at the vectors.rs lambda propagation; decoded by
`Deps::entries`), chosen over the debug-only tag because VALUES survive
the IR codec (cache round-trips erase the debug tag — corpus-probed).
The block-result dep read in `get_free_vars` is deleted (probed across
five corpora: never decides alone; a debug sentinel guards the claim);
`check_ref_leaks` pools the decoded note instead of dropping it (its
false `___clos_1` leak report is gone).  Regression:
`tests/scripts/297-closure-factory-explicit-return.loft`.  Full record +
residuals found en route (armed-lib-debug baseline redness; @PLN55
growth assert on two lib fns): DEPS_INVENTORY § Status.

---

## H3 — Ownership recomputed by shape analysis

**The violated invariant**: *a fact should be stored where it is decided,
not re-derived where it is needed.*  `src/scopes.rs` (4,668 lines)
re-derives ownership, escape, confinement, and free-placement facts by
pattern-matching IR shapes — every analysis re-asserts what construction
already knew.  Every new construct must be taught to each analysis or it
silently misclassifies.

**Evidence**: plan-57's entire tail (probe-numbered exclusions, the gated
`LOFT_CONF_RECOVER` experiment — shipped default-ON 2026-08-21, opt-out `LOFT_NO_CONF_RECOVER`; cluster I/III routes), #260 (the
`Set(v, Null)` position doubling as the declaration point — fixed this
cycle by moving the fact INTO the variable table, the model for this
hotspot), #316/#323 (five ownership homes), the dominance-twin drift
found in pass 3.

**Mitigation — ownership as carried data (L, after H1+H2)**

1. The variable table (`src/variables/`) becomes the single home for
   per-var ownership state: owned / borrowed-view / caller-buffer /
   captured — written ONCE at the construction site, read by every
   scopes analysis instead of re-derived.  (#260 Fix B and
   `mark_caller_hidden_buf` are the first two facts already living
   there; continue the pattern.)
2. Convert one analysis at a time, matrix-first: `guard_escapes` →
   `reclaim_safe` → confinement.  Each conversion keeps the old shape
   walk as a debug-assert cross-check for one release
   (`debug_assert_eq!(carried, derived)`) — drift between them is a
   found bug, exactly like the pass-3 unifications.
   - **Design-protocol pass 1 (2026-06-17) — over-reach caught BEFORE coding.**
     Classifying the analyses falsified the framing of this step: `guard_escapes(node,
     target)`, `reclaim_safe(code, vars, st)`, `confine_reassign_safe(code, local)` and
     `store_confinement` are all **contextual** — a *(code region × target)* query
     ("does X hold for target WITHIN this code"), NOT a re-derivation of a per-var fact.
     So they are the genuinely-shape-local questions of point 3, not carriable per-var
     state, and "convert the analysis to carried data" would over-unify a contextual
     query under a per-var flag (`guard_escapes` as a single per-var bool loses the
     code-region the query is scoped to).  The carriable per-var *category* (point 1)
     is already largely carried: `captured` + `caller_hidden_buf` on the Variable
     struct, owned/borrowed-view via `Type::is_heap_owned` + the `Deps` borrow set.
     The actionable H3 residual is therefore NARROWER than "carry-convert the
     analyses": probe each contextual analysis's BODY for whether it *re-derives* a
     per-var ownership fact inline (vs only asking a shape-local escape/placement
     question), and have only that inline re-derivation read the carried category —
     the analyses themselves stay walks.
   - **Design-protocol pass 2 (2026-06-17) — the body probe, on the two CORE
     free-placement analyses (`reclaim_safe`, `store_confinement`): no inline
     ownership re-derivation found.**  Both READ carried per-var facts
     (`is_argument`/`is_captured`/`is_skip_free`/`tp().depend()`/`name`) and call
     genuinely-contextual sub-queries (`guard_escapes`, `holder_retained`,
     `confine_reassign_safe`, `recover_backer`).  So this hotspot's premise — "every
     analysis re-asserts what construction already knew" — is **over-stated**: the
     analyses READ what construction knew (the facts ARE carried, on the variable
     table + `Type`/`Deps`), and what remains is the **inherent shape-locality** of
     free-placement (escape / retention / confinement-span genuinely depend on code
     structure, not on a carryable per-var attribute).  **Conclusion: H3's "carry the
     facts" mitigation (points 1–2) is largely already realised; there is no open L
     carry-conversion.**  The real residual pain the evidence (#316/#323) points at is
     the COMPLEXITY of the contextual placement walks (each new construct's shape must
     be handled) — which carrying cannot remove; it is managed by the cross-check
     corpus (plan-57 probes, watermark guard), not by an ownership refactor.  Residual
     verification (not yet done): confirm the remaining analyses (`free_vars`,
     `guard_refs`, `store_lifetime_guard`) also only-read; if so, H3 closes as
     "premise over-stated, facts already carried."
3. The walker keystones (`Value::any_node` family) stay as the mechanism
   for the residual genuinely-shape-local questions (tail position,
   dominance).
4. Validation: the store-leak gate, ASan run, plan-57's probe corpus,
   watermark guard (`reclaim_unfreed_eligible`).

---

## H4 — Two-and-a-half backends

**The violated invariant**: *one semantics, one implementation.*  The
fill.rs generation covers OPCODES (one `#rust` template per op — this
part is healthy).  Above the opcode level, lowering decisions are
implemented independently three times: bytecode (`state/codegen.rs`),
native (`src/generation/`), wasm (native + bridge variations).

**Evidence this cycle alone**: #260 needed PAIRED fixes (native prologue
+ interp `OpInitRefSentinel`); the x=x fix had to be hoisted to the
parser to cover both; #333's fail-fast was native-only plumbing; the
pre_eval free-op recognizers existed only because native re-walks what
bytecode also walks.

**Mitigation — discipline now, structure later**

- (standing, S per change) Any lowering-semantics change MUST land with a
  `cross_mode!` cell or a `tests/scripts/` file that both sweeps run —
  this is mostly already culture; make it a CODE.md checklist line.
- (M) Extend the `#rust`-template idea upward where shapes allow: the
  free-op family, null-init emission, and the GET→SET table are
  per-backend tables that could derive from one declaration each, the
  same way fill.rs derives ops.
- (L, 1.1+ scale) The real fix is one shared lowering IR below today's
  Value-IR (decisions made once, interpreted twice).  Do NOT attempt
  before H1/H3 — they shrink exactly the semantics that would need
  porting.

**Design-protocol finding (2026-06-17) — H4-medium RESOLVED.**  Of the three (M)
candidates only the GET→SET table was a genuine duplication, and it shipped
(`NarrowIntKind::of`, `9153e132`).  The other two are NOT clean merges (premise
over-stated, like H3):

- **Free-op family — already single-homed.**  `scopes.rs` is the one SELECTION
  site (type → `OpFreeText` / `OpFreeRef` / `OpFreeRefIfDistinct`, inserted into
  the shared IR both backends consume) and `pre_eval::free_op_var` is the one
  RECOGNIZER (de-duped in the pass-3 work).  No per-backend free-op table exists
  to merge.
- **Null-init — different facts, not a duplication.**  `emit_typed_null` (the
  live NULL sentinel on the bytecode stack) and `default_native_value` (the
  native default-INIT placeholder) encode different things.  PROBED: the
  live-null path emits the sentinel on BOTH backends (a `null`-returning
  `integer` / `float` fn round-trips identically interp-vs-native), and
  `default_native_value`'s scalar `0` / `0.0` are unreachable as live nulls
  (`floatvar = null` is type-rejected).  Merging them would conflate
  null-sentinel with default-init — the H6-`NullEnc`-phantom mistake avoided.
  The one real residual — `default_native_value`'s undocumented, conflated
  contract — is fixed by a doc comment naming the contract + its relationship to
  `emit_typed_null`.

The standing discipline (every lowering-semantics change lands a `cross_mode!`
cell / `tests/scripts/` file) remains the live guard against future drift.

---

## H5 — The two-pass name-stability contract

**The violated invariant** (implicit, undocumented until now): *pass 2
must reproduce pass 1's synthesized names exactly* — work-ref counters,
lambda naming, attr re-finding by name all depend on it, and the parser
is not re-entrant beyond two passes (the #339 third-pass experiment
segfaulted on half-migrated variable tables).

**Mitigation (S now, mostly dissolved by H1) — DONE 2026-06-16**

1. (S) ✅ Document the contract at the `first_pass` flag declaration
   (`parser/mod.rs`) — what must be deterministic, what is re-found by
   name, why a third pass is unsound.
2. (S) ✅ `debug_assert` the contract where it's cheap: attribute COUNT per
   def equal at end of both passes — landed 961e6c27 (`assert_pass2_def_attr_stable`),
   silent across the 270-script debug corpus.  Post-H1 this is an invariant,
   not an aspiration.
   The other named residual — **work-ref (`__ref_N`) counter equality per fn** —
   was NOT added, by design, after item 3's re-evaluation below.
3. ✅ **Re-evaluation done (item 3 fired post-H1).** H1 (@PLN55) removed the
   only known source of cross-pass signature divergence; probing the question
   "does anything still rely on name stability beyond lambdas?" settled it:
   - `work_refs()` (the `__ref_N` incrementer) fires **zero** times across the
     whole debug corpus, both passes — H1's signature-time `__retbuf` dissolved
     the per-call-site work-ref temporaries that used to need a pass-stable name.
   - A stored-table work-ref-counter assert is **permanently vacuous** anyway:
     `Function::append` unconditionally resets the stored `work_ref` to 0 at
     store time, so both passes read 0 regardless of corpus.
   - The one failure mode such an assert could ever have caught — a cross-pass
     `__ref_N` name shift making `ref_return` add a spurious attr — is **already
     caught by the attr-count assert** (the spurious attr IS a count divergence).
   So the attr-count assert is the complete H5 validation; **lambda naming**
   remains the only live name-stability consumer (and is exercised directly by
   the corpus, unlike `__ref_N`).

---

## H6 — The null-sentinel matrix

**The violated invariant**: *one fact (this value means null) per
type, expressed once.*  Today: `i64::MIN` (integer), `i32::MIN` (i32
fields), 255 (byte/boolean/enum), raw-0 `+1` encoding (Short),
`store_nr == u16::MAX` (DbRef sentinel) vs `(0,0)` zero-default, with
symmetric read/write conventions per width re-implemented at each
`OpGet*/OpSet*` pair.

**Evidence**: the u8/u16 nullable-width work this cycle (one code
reserved, min/max-expressed, symmetric handling had drifted), the
`byte_vec` gate subtlety in `get_val`, `default_native_value`'s
per-type arms (Spacial was missing until last week).

**Mitigation — one sentinel table (S–M)**

1. A `sentinel(tp) -> Encoding` table in `src/data.rs` next to
   `byte_width` — the ONE place mapping type×width to its null code and
   its read/write transform.  `fill.rs` templates, `default_native_value`,
   `emit_typed_null`, and the narrow-vector paths consume it.
2. A doc table in [INTERMEDIATE.md](INTERMEDIATE.md) generated or checked
   against it (the api-lint pattern), so the convention is greppable.
3. Validation: the narrow-width script corpus (p184 family, 292
   three-state boolean), both backends.

**Design notes — matrix gathered 2026-06-16 (not yet built).**

Two axes, NOT one — the consolidation must keep them distinct:
- **Stack / register null** (what `conv_*_from_null`, `state/codegen.rs::emit_typed_null`,
  `generation/mod.rs::default_native_value` produce): `i64::MIN` (int/char/bool/enum),
  `f64::NAN` / `f32::NAN`, `STRING_NULL` (`"\0"`), DbRef sentinel `{u16::MAX,0,0}`.
- **Stored / field null** (the per-width `store.rs` set/get transforms, all decoding to
  the stack form `i32::MIN`/`i64::MIN`):

  | Parts / width | stored null | non-null encode (write) | decode (read) |
  |---|---|---|---|
  | `Int` i32 (4) / `Long` i64 (8) | the MIN value itself | raw | raw; `rec==0`→MIN |
  | `Short` u16 (2, `+1`) | `0u16` | `val-min+1` | `read+min-1`; `0`→`i32::MIN` |
  | `ShortRaw` u16 (2, direct) | `u16::MAX` | `val-min` | `read+min`; `u16::MAX`→`i32::MIN` |
  | `Byte` u8 (1) | `255` (write only) | `val-min` | `read+min` — **does NOT decode 255** |

  *Verified `store.rs:1836-1975`.*

**Load-bearing RISK — SETTLED 2026-06-17 (matrix-first), and it was NOT the
asymmetry the design note hypothesized.** The note read the raw `get_byte`
accessor and concluded null never round-trips. But the nullable consumers
`get_byte_nullable`/`set_byte_nullable` (and the Short/ShortRaw twins) are a
**correct symmetric pair** (`raw 255 ⇔ null` for every `min`), confirmed by a
`min × range-fullness × container × backend` matrix that round-tripped null for
*every representable* case on both backends. There is **no encode/decode
asymmetry to repair** — so the `NullEnc` encode/decode table would have
*enshrined a phantom risk*.

The matrix surfaced the REAL latent bug on a different axis — **range-fullness,
not `min`**: a NULLABLE narrow-integer field with the FULL code range
(`max-min == 255`/`65535`) read its null back as `max-1`. Root: the storage
width was computed in **two disagreeing places** — `IntegerSpec::byte_width`
(read, via `get_val`) correctly reserved a sentinel code, but `Type::size`
(write + allocation, via `set_field_check`/`typedef.rs`) did **not** for the
nullable full range, so the field was under-allocated to 1 byte (Byte) and the
write stored the 1-byte `255` sentinel into a field the read decoded as a 2-byte
Short. THIS is H6's thesis realised (one width-fact, two drifted copies) — fixed
by the proportionate chokepoint move: a single **`IntegerSpec::range_to_width`**
home that both `byte_width` and `Type::size` derive from (commit pending).
Regression: `tests/scripts/389-h6-nullable-full-range-narrow.loft` (both
backends). The matrix prevented building the table to fix the wrong thing — the
remaining `NullEnc` consolidation below is now an optional, lower-risk cleanup
(the per-width pairs already agree), not a load-bearing fix.

Sketch: `enum NullEnc { I32Min, I64Min, ShortPlus1, RawMax, ByteMax, FloatNan,
SingleNan, TextNull, DbRefRec0, DbRefSentinel }` with `null_stored()/encode()/decode()`;
`sentinel(tp)->NullEnc` beside `IntegerSpec::byte_width`. Convert one consumer family
at a time behind `debug_assert_eq!(table_derived, hardcoded)` cross-checks.
Consumers: `store.rs` set_*/get_*, `fill.rs` conv_*_from_null, `state/codegen.rs::
emit_typed_null`, `generation/mod.rs::default_native_value`, `database/structures.rs::
set_default_value`, `database/types.rs` byte/short/short_raw/int.

**Design decision — narrow fixed-width ints are MEMORY-ALLOCATION types; null is
the ALL-ONES byte, and nullability only shifts the `min` offset (2026-06-17).**

For the `forced_size` aliases (`u8`/`i8`/`u16`/`i16`) the storage width is FIXED —
a `u8` is one byte, always, nullable or not; storage NEVER widens for nullability.
The design is chosen for **rustc-codegen simplicity**, not for matching the value
range a reader expects from the type's name:

- **Null is stored as the all-ones byte** (`255` for 1-byte, `65535` for 2-byte),
  uniformly for EVERY narrow type.  So in generated Rust, storing and testing null
  is ONE type-independent instruction — `byte == all-ones → null` — directly in
  memory, no per-type branch.  *(`set_byte`: `val == i32::MIN → 255`.)*
- **A non-null value decodes as `read + min`**; `min` is the only thing that
  varies — by type AND by nullability.  The usable values map to bytes
  `0..=254` (1-byte) so the all-ones byte is never a real value.  Decoded, the
  all-ones byte is always exactly `max+1` — one past the usable range, so the user
  can never produce it: the sentinel is invisible.

| alias | not-null `min` / range | nullable `min` / range | all-ones byte decodes to |
|---|---|---|---|
| `u8`  | `0`   `0..=255`        | `0`    `0..=254`        | `255` = max+1 → null |
| `i8`  | `-128` `-128..=127`   | `-127` `-127..=127`    | `128` = max+1 → null |
| `u16` | `0`   `0..=65535`     | `0`    `0..=65534`     | `65535` = max+1 → null |
| `i16` | `-32768` `-32768..=32767` | `-32767` `-32767..=32767` | `32768` = max+1 → null |

Nullable sacrifices ONE edge value, and which edge is the user-invisible part that
keeps the useful range: **unsigned drops the TOP** (`min` stays `0`, `max-=1`),
**signed drops the BOTTOM** (`max` stays, `min+=1`) — so a not-null `i8` is the
original Rust `-128..=127` and a nullable `i8` is `-127..=127`, differing only in
whether `-128` is available.

**Probed BASELINE — the full matrix on the interpreter (2026-06-17).**  Each cell
stores an edge value into a struct field and reads it back:

| field | edge values → read back |
|---|---|
| `u8` nullable  | `255`→**null** ✗ · `254`✓ `0`✓ `null`✓ |
| `i8` nullable  | `127`→**null** ✗ · `-128`✓ `-127`✓ `null`✓ |
| `u16` nullable | `65535`→**null** ✗ · `65534`✓ `null`✓ |
| `i16` nullable | `32767`→**null** ✗ · `-32768`✓ `null`✓ |
| `u8` **not-null**  | `255`✓ `0`✓ — full range, correct |
| `i8` **not-null**  | `-128`✓ `127`✓ — full range, correct |
| `u16` **not-null** | `65535`→**null** ✗ |
| `i16` **not-null** | `-32768`✓ `32767`→**null** ✗ |

Two DISTINCT defects fall out:

1. **The nullable-sentinel design (this section).**  Today the sacrificed value is
   silently stored as null instead of being out of range, and for SIGNED types the
   sacrificed end is the TOP (`127`/`32767`) — the opposite of the design's
   symmetric `-127..=127` / `-32767..=32767`.  Fix = the staged
   `IntegerSpec::usable_min`/`usable_max` (in `data.rs`): not-null → full native
   range; nullable → signed `min+1`, unsigned `max-1`.  Wire it at the THREE
   `spec.min`/`spec.max` consumers — read op min (`parser/mod.rs:3303`), write op
   min (`:3724`/`:3771`), literal range-check (`:1248`) — all deriving from the one
   method so the read/write/check cannot drift.  The runtime `get_byte`/`set_byte`
   DON'T change (already take `min`; null is already the all-ones byte); the
   narrow-VECTOR element path (`Byte`/`ShortRaw`, raw) is excluded via
   `nullable && !narrow_vec`.

2. **A SEPARATE 2-byte not-null bug (fixed in the same pass).**  A `not null`
   `u16`/`i16` field could not hold its max (`65535`/`32767` → null): the 2-byte
   field path used the `Short` (`+1`) encoding which reserves a null code even when
   the field is not-null (the 1-byte `Byte` op does NOT — that's why not-null
   `u8`/`i8` were already full-range).  Fixed with a new `NarrowIntKind::ShortFull`
   / `OpGetShortFull` (`store::get_short_full`: direct `read + min`, NO sentinel),
   the 2-byte twin of `Byte`; the write reuses `OpSetShortRaw`.

**DONE 2026-06-17 (`4a632251`).**  All of the above LANDED + full suite green, both
backends:
- `IntegerSpec::usable_min`/`usable_max` (the one width/range home) wired at the
  read op, write op, and the literal range-check (`int_value_fits`, gained a
  `narrow_field` flag — a field STORE reserves the sentinel; a param/return/cast is
  a full-width register value, so `f(65535)` to a `u16` param stays legal; function
  params can't be `not null`).  *(The `narrow_field` flag was later REVERTED in
  `ea4a74fe` — see "Developer communication" below — when it proved too coarse
  for struct-init; the same behaviour is now a store-only sentinel check.  The
  `usable_min`/`usable_max` home is unchanged.)*
- Field nullability is stamped onto the stored `IntegerSpec.not_null` at attribute
  registration (the alias path left it at the default), so the range-check reads
  the right bounds.
- `OpGetShortFull` added end-to-end (`default/01_code.loft` `#rust` template →
  `fill.rs` regenerated via `make fill` → native via the template).
- `lib/code.loft` `cur_arg: u8` → `u8 not null` (the `255` "no arg" sentinel is a
  meaningful value, not the null encoding).
- Regression `tests/scripts/389-narrow-alias-ranges.loft`.

Final shape: not-null keeps the full native range; nullable is symmetric for signed
(`-127..=127`, `-32767..=32767`) and top-trimmed for unsigned (`0..=254`,
`0..=65534`); the all-ones byte is the one uniform sentinel, only `min` shifts.

**Follow-up DONE (`f6f660f8`): the inline `Struct{..}.x` byte-field read on
native.**  The pre-eval/hoisting path (`generation/dispatch.rs`) hoisted a
store-mutating call arg into a `_harg_*` temp and emitted the CALL form
`OpGetByteNullable(cell, _harg, …)` for ANY `Value::Call` — including inline
`#rust` ops (byte/short field reads), which have no callable fn, so native
compilation failed with "not found in scope".  It predated this work (broke plain
`OpGetByte` too) and was NOT narrow-int-specific.  Fixed by gating that path on
`def_fn.rust().is_empty()`: only true user-fn / `codegen_runtime` Op-stub calls
take the call-form hoist; inline `#rust` ops fall through to the normal emit, which
inlines the template (its own `let db = @v1; …` sequences the mutating arg before
the read borrow).  Regression `tests/scripts/inline-construct-narrow-read.loft`
(both backends, all four narrow widths × nullable/not-null/null).

**Developer communication — DONE (`ea4a74fe` compile-time, `1b8a3792` runtime).**
A narrow sentinel is invisible to the user (§ design), so a value that *lands* on
it must be SURFACED, not silently nulled.  Two touch-points, each targeted (never
pervasive):

- **Compile-time, for literals.**  Storing the sacrificed value into a nullable
  narrow field is a compile error that names the cause:
  *"255 is reserved as the null sentinel of a nullable u8 (usable 0..=254); declare
  the field `not null` for the full range, or cast with `as u8`"* — applied at BOTH
  field-store sites (`obj.x = 255` in `expressions.rs`, `U8N { x: 255 }` in
  `objects.rs`).  Wiring it to the struct-init site closed a **silent-store
  regression**: the `4a632251` `narrow_field` flag on `int_value_fits` was too
  coarse — struct-init goes through `convert` (full bounds), so `U8N { x: 255 }`
  slipped past and stored null.  The flag is **reverted**: `int_value_fits` is back
  to the plain full-range TYPE-fit (so `f(65535)` to a `u16` param stays legal —
  params/casts are full-width registers, no storage sentinel), and the sentinel
  reservation is now a **store-only** check (`Parser::nullable_sentinel_hint`)
  applied at exactly the two field-store sites.  Regression
  `tests/scripts/389-narrow-sentinel-rejected.loft` — which @PLN25 F2 then made
  moot without anyone noticing: a plain narrow integer is NON-null under DN1, so
  it uses the FULL width and `nullable_sentinel_hint` returns early, and the
  file's `@EXPECT_ERROR` matched nothing from then on (loft#929).  It now pins
  the rule from the value side (`255` stored and read back), plus the half F2
  left open — a NULLABLE narrow still spends its top value on the sentinel, so
  `U8Q { x: 255 }` reads back null with no diagnostic.
- **Runtime, dev-only, for computed values.**  A non-null value computed at runtime
  (e.g. `f() as u8 == 255`) can still collide; the nullable set ops route through
  `Stores::set_byte_nullable` / `set_short_nullable`, which detect the collision and
  log a rate-limited `Warn` (*"value 255 written to a nullable narrow field collides
  with the null sentinel and reads back as null (usable 0..=254); declare … `not
  null` …"*).  loft's logger IS the dev-vs-shipped switch, for free: the interpreter
  attaches it at default `Warn` → the dev sees it in `.loft/log.txt`; a shipped
  game runs with no dev logger / a production config → suppressed, the
  `logger.is_none()` guard collapsing that path to one `Option` check per write;
  rate-limited (keyed on field offset → 1000 colliding writes = 5 warnings, then
  suppressed).  Behaviour is UNCHANGED — the value still stores the sentinel; only
  the diagnostic is added.  Regression
  `tests/scripts/389-narrow-runtime-collision.loft` (both backends).

**Narrow-VECTOR consistency — DONE.**  Narrow-vector elements now reach their FULL
range, consistent with the not-null field path: `vector<u16>` holds `65535` and
`vector<i16>` holds `32767`, matching `vector<u8>` holding `255`.  Root: the vector
storage `Parts::ShortRaw` is ALWAYS non-nullable (`narrow_vector_content` /
`vectors.rs` build it with `nullable = false`), but its read `get_i16_raw` still
decoded `u16::MAX → null` — corrupting ALL five read sites (runtime access,
serialization `io.rs`, format ×2, `types.rs`).  The proportionate chokepoint fix:
`get_i16_raw` now delegates to `get_short_full` (no sentinel), so every path is
full-range at once and the two functions can't drift (H6 thesis).  The write twin
`set_i16_raw` keeps its `i32::MIN → u16::MAX` clamp as an underflow guard — narrow
vectors store concrete values, never null, exactly like `vector<u8>`/`Byte`.
Regression `tests/scripts/389-narrow-vector-full-range.loft` (both backends).

**Open follow-up (separate, pre-existing).**  The 4-/8-byte `i32`/`integer` types
use a `MIN`-sentinel (a not-null `i32` doesn't reclaim `i32::MIN`).

---

## H7 — Hand-maintained IR codecs (F9)

**The violated invariant**: *the codec must cover exactly the schema.*
`ir_store.rs` / `ir_schema.rs` / `ir_read.rs` hand-encode every
`Value`/`Type` variant field-by-field.  A new variant compiles cleanly
with a silent codec gap; the startup cache and G2 store-backed emission
then read garbage.

**Mitigation — derive from one schema declaration (M, design exists as F9)**

1. The `for_each_child` keystone can't drive codecs (they encode FIELDS,
   not just child edges) — F9's "one schema declaration" is the fix:
   a macro or table from which encoder, decoder, AND the exhaustive
   walker match all derive.  A new variant then breaks the build until
   all three know it.
2. Until then (S, do immediately with the next variant): a round-trip
   property test — materialize every construct in `tests/scripts/` into
   a store, read it back, assert IR equality.  Cheap, catches gaps the
   day they're introduced.
3. Validation: `arc_e_program_cache` suite, `LOFT_CODEGEN_STORE=1` run of
   the native sweep (the G2 path).

---

## H8 — The raw `Stores.allocations` surface

**The violated invariant**: *invariant-rich state needs an interface.*
60+ direct `stores.allocations[...]` touches across `parallel.rs`,
`extensions.rs`, `native.rs`, `wasm_gl.rs` — the parallel worker-slot
swap dance is the riskiest patch (store isolation during `par` is
load-bearing for memory safety).

**Mitigation — the already-planned privacy pass (M–L)**

Tracked in [STABILITY_PASS2.md](STABILITY_PASS2.md)'s deferred rows: design
the accessor surface (worker-slot claim/release, swap-back, lock APIs) as
ONE batch with [THREADING.md](THREADING.md) in hand, then convert callers
mechanically.  Not started; should precede any new `par` feature work.

---

## H9 — Raw `*mut Stores` across the shared-store cdylib↔host bridge

**The violated invariant**: *a cross-binary boundary must be C-ABI, not a shared
in-memory layout.*  A loft library compiled to a shared-store cdylib (N9/C71
dispatch) passes the **raw `*mut Stores`** across the `loft_shared_*` bridge to
the host, so host and cdylib must agree on a byte-identical `Stores` layout.  A
feature-divergent loft build silently diverges that layout, and
`loft_ffi_fingerprint` does not catch a change that misses `loft-ffi` — this is
the mechanism behind the `viewer_markdown` flaky-cdylib collision.

**Mitigation (M, redesign)** — decouple to a stable `LoftStore` handle: the same
C-ABI indirection the exec path already uses, so the cdylib↔host interaction
crosses a versioned handle, not a raw pointer.  Design in
[NATIVE_ARTIFACT_IDENTITY.md § Resolution](NATIVE_ARTIFACT_IDENTITY.md#resolution-separate-the-api-id-from-the-rust-part-link-the-cdylib-by-c-abi)
+ [@PLN26](https://github.com/loft-lang/plans/issues/26).  Sibling of
[H8](#h8--the-raw-storesallocations-surface): both are "raw `Stores` state needs
an interface" — H9 is the FFI-boundary site, H8 the in-process `allocations`
site; do them together.

**Status**: already realized (it manifests as the `viewer_markdown` collision),
so this is debt to schedule, not a latent risk.  Was [#389](https://github.com/loft-lang/loft/issues/389)
Part 1 (issue closed; Part 2 shipped as @PLN26 ph.2 — see NATIVE.md § Open work).

---

## What this register is NOT

- Not a bug list — open bugs live in GitHub Issues; this is where the
  NEXT bugs will come from.
- Not a license to rewrite — every H-item follows the pass-2 rules:
  quiet window, matrix-first, one move at a time, suite-green between
  moves, and the old path kept as a cross-check where feasible.
- Not static — when an H-item's mitigation lands, move its entry to a
  short "retired" section below with the closing commit, the same way
  PROBLEMS.md archives fixed bugs.

## Retired

### H1 — Analysis-dependent function arity (RETIRED 2026-06-11)

Phase 1 (the unconditional `__retbuf` ABI + the dispatcher census:
plain calls, par lanes with witness-free, entry invocations, the
cdylib shared bridge with runtime type-name ids) and phase 2 (deleted:
`retrofit_callers_hidden_args`, the `grew_in_pass2` plumbing, the
`__rref_` recursive-self counter dance; documented: the two-pass
contract at the `first_pass` flag, the calling convention in
COMPILER.md § Function calling convention) — all landed on `bugs325`.
Arity is a pure function of the declaration AFTER pass 1: armed builds
showed the original "never grows" assert firing on the
multi-return-site shape (a fn whose return sites are materialized work
refs — forward-referenced callee / generated `.parse` — finds the one
`__retbuf` consumed by its first site and grows a hidden attr per
later site; seen on `moros_map::map_from_json` and graphics'
`glb_pos_min`).  PASS-1 growth there is sound — pass 2 re-parses every
caller against the final arity and re-finds the grown attrs by name,
so arity is pass-stable — and the assert now guards exactly the
dangerous clause: pass-2 growth on a plain fn (2026-06-12).  An
opt-out (leaving site 2+ un-promoted) was tried and REVERTED: the
cdylib emission of un-promoted materialized returns dereferences the
null-sentinel buffer (`map_export_glb` chain, store 65535) — recorded
in the ref_return comment.  Regression:
`tests/scripts/298-multi-return-site-ref-buffer.loft`.  H5's load
mostly dissolved with H1 (the contract is now documented + the assert
enforces its hardest clause).  Full history:
[plans/55-return-abi](plans/55-return-abi/README.md).

## H10 — The per-`Parts` container taxonomy (copy source-fold) — ✅ RESOLVED 2026-07-10

**Status.** The `copy_claims` source enumeration is folded onto `for_each_owned_child` —
all four kinds (`hash_body`, `index_body`, `array_body`, `seq_vector`) now read the one
keystone walk, joining `remove_claims`. Done on branch `tuxedo-cluster-c` (phases 0–3); the
scope was corrected first (see "What actually folded" below) and each fold verified on both
backends. `validate_claims` and construction were **ruled out of scope by a design probe**,
not left undone — folding them would be over-reach. The register entry stays as the record.

**The risk (original framing).** The heap-cascade operations — copy, remove, validate,
construct — each re-encode the same per-`Parts` fact ("which child type, at which stride,
with which container walk") independently. `remove_claims` was collapsed onto
`for_each_owned_child` (REDFLAGS cluster C, C.0–C.3, merged), but **copy never followed**:
`copy_claims` split into 4 per-kind helpers (`copy_claims_seq_vector/array/hash/index`).
A new collection kind had to be taught each source walk separately or it silently
misclassifies.

**What actually folded (scope corrected 2026-06-22, executed 2026-07-10).** Only the copy
**source** enumeration. `validate_claims` does NOT fold — it is a defensive walk that
bounds-checks each pointer before following it, where the keystone trusts its pointers.
`record_new`/`record_finish` is a WRITE path, not a read-walk. And each copy helper's
**destination** build stays per-kind (re-insert vs slot vs bulk-copy). The "~53 arms across
3 dispatchers" framing was the falsified over-scope; the real, landed change is the copy
source walk carried once. Detail: [STABILITY_REDFLAG_REMEDIATION § Cluster C / H10](STABILITY_REDFLAG_REMEDIATION.md#cluster-c--h10--fold-copy_claims-source-enumeration-onto-the-keystone).

**Why it's a future bug factory.** The **densest HISTORICAL bug cluster** in the
tree: @P290 (SIGSEGV — `room*2` vs `(room-1)*2`), @P306/@P318 (hash slot-drift),
@P309 (missing length header), #373 (`types[u32::MAX]`), the `gen_set_first_*_null`
family (#260/#330). Every drift between the dispatchers is a latent corruption.

**The fix — a 3-way split, NOT a uniform fold (the over-unification guard).** Grounding
in the structure (2026-06-22) shows the keystone `Stores::for_each_owned_child(rec, tp) →
OwnedWalk` (`src/database/allocation.rs:95`) yields `children` (each
`{child: DbRef, child_tp, owning_elem: Option<u32>}`) + `container_rec` + `zero_field`;
`remove_claims` (2024) is the model thin-visitor. The three "drifting" dispatchers do NOT
fold equally — spraying all three onto the read-walk keystone would be over-reach:
- **`validate_claims` (1268)** — does **NOT** fold. The 2026-06-22 investigation falsified the
  "pure read-walk folds cleanly" hypothesis: it is a separate DEFENSIVE family. It runs on
  *suspected-corrupt* heaps (the @P306 `LOFT_TRACE_CR` pre-walk before `OpCopyRecord`), so it
  bounds-checks each pointer BEFORE following it and does not recurse into the per-element-record
  kinds (`Array`/`Ordered`/`Hash`/`Index`) at all — whereas this keystone TRUSTS its pointers
  (`debug_assert!` on a freed/out-of-range record). Folding it would turn "name the broken edge"
  back into "fault on it". The boundary is pinned in the keystone's `OwnedChild` doc comment.
- **`copy_claims` (1711) + 4 helpers** — the SOURCE enumeration folds (`copy_claims_hash_body`
  already reads the keystone); the DESTINATION build (allocating into `to`) stays. PARTIAL.
- **construction `record_new`/`record_finish` (`structures.rs:60`/`122`)** — a WRITE/build,
  NOT a read-walk → forcing it onto the keystone is OVER-REACH. If it shares a per-`Parts`
  LAYOUT fact (strides/positions), that is a SEPARATE refactor, not this fold.

**Phasing** (each verifies BOTH backends, with the historical shapes @P290/@P306/@P318/@P309
still caught): (1) `validate_claims` — investigated 2026-06-22, does **NOT** fold (separate
defensive walk; boundary pinned in the keystone doc comment); (2) `copy_claims` source-enum
fold — **the active step**, with a concrete verifiable plan in
[STABILITY_REDFLAG_REMEDIATION.md § Cluster C / H10](STABILITY_REDFLAG_REMEDIATION.md#cluster-c--h10--fold-copy_claims-source-enumeration-onto-the-keystone)
(per-helper Phase 0–4, ~S each: `index_body` is a near drop-in, `array_body`/`seq_vector` need a
container-type call-site touch); (3) assess construction (likely its own layout-descriptor
refactor — do NOT force it). **Size S per copy helper.** Surfaced by the 2026-06-22 code-quality
audit; detail in [STABILITY_REDFLAGS.md cluster C](STABILITY_REDFLAGS.md). Tracked as task #39 /
this H10.
