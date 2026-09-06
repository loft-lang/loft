# QUALITY — Open Issues, Active Designs, Enhancement Plan
# QUALITY — Open Issues, Active Design  Re-measured again (re-measured after loft#1371 gave the `&` LOCAL link the lowering the `&` PARAMETER has, for a text and a vector source): optional `720 | 360 | 5 | 355`, unspan `405 | 381 | 24`.  Re-measured again (re-measured after @PLN153 phase 4 batch 1 (the `&` lowering's source tests read through base())): optional `717 | 359 | 5 | 353`, unspan `402 | 378 | 24`.  Re-measured again (re-measured after @PLN153 phase 3c joined with the @FR-O-Witness walk): optional `716 | 358 | 5 | 353`, unspan `402 | 378 | 24`.s, Enhancement Plan
# QUALITY — Open Issues, Active Design  Re-measured again (re-measured after loft#1371 gave the `&` LOCAL link the lowering the `&` PARAMETER has, for a text and a vector source): optional `720 | 360 | 5 | 355`, unspan `406 | 382 | 24` (re-measured again after loft#1376's `produces_whole_record`).  Re-measured again (re-measured after @PLN153 phase 4 batch 1 (the `&` lowering's source tests read through base())): optional `717 | 359 | 5 | 353`, unspan `402 | 378 | 24`.  Re-measured again (re-measured after @PLN153 phase 3c joined with the @FR-O-Witness walk): optional `716 | 358 | 5 | 353`, unspan `402 | 378 | 24`.s, Enhancement Plan

This document is the single source of truth for **what's broken, what's
being fixed, and what should be fixed next**.  It replaces the earlier
BITING_PLAN.md (which mixed status, design, and history) and
consolidates the open-issue tracking that previously drifted between
PROBLEMS.md and CAVEATS.md.

Read order:
> **⚠ Reconciled 2026-07-10 — read this before the back half.**  Sections 2–5 below are
> **historical design reference, not a live queue.**  The P54 sprint is complete except one
> residual (the Q1 auto-wrap gap in § Open programmer-biting issues); the B2–B7 compiler
> blockers were AUDITED + CLOSED 2026-05-21 on both backends; C54 (`integer` → i64) LANDED
> 2026-04-21.  § Recommended landing order and § Enhancement tiers § Tier 1 are dated
> **2026-04-13** and name work that has since shipped — do not plan from them.  The live
> queues are [ROADMAP.md](ROADMAP.md) and [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md).

1. § Open programmer-biting issues — the live work queue
2. § Active sprint — P54 — **COMPLETE except the Q1 auto-wrap residual**; kept as design reference
3. § Active design — C54 — **LANDED** 2026-04-21 via @PLAN01; kept as design reference
4. § Compiler blockers — struct-enum bugs (B2…B7) — **all CLOSED 2026-05-21**; kept as design reference
5. § Enhancement tiers — quality investments ranked by leverage (**Tier 1 has shipped**; see the banner)

History and closed items live in [CHANGELOG.md](../../CHANGELOG.md).
Decisions to *not* fix something live in
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).

---

## Open programmer-biting issues

| # | Issue | Severity | Status |
|---|-------|----------|--------|
| P54 | `json_items` returns opaque `vector<text>`; `MyStruct.parse(text)` silently zeroes on malformed input | High | **Steps 4 + 5 + 6 + Q1 schema-side COMPLETE 2026-04-14 (single-walker design)**.  Step 4: arena materialiser.  Step 5: `Type.parse(JsonValue)` lowers to one IR call to `n_struct_from_jsonvalue(arg, struct_kt)` regardless of struct shape.  The runtime walker uses `stores.types[struct_kt].parts` to dispatch on each declared field type — primitive (text / integer / float / boolean) extracts with inline Q1 schema-side type-mismatch checks, nested struct recurses on the embedded sub-struct DbRef, JsonValue-typed fields byte-copy verbatim, and `vector<T>` fields iterate the JArray + recurse per element (struct elements call back into the walker).  Step 6: auto-wrap form — text arguments to `Struct.parse(text)` route through `json_parse` internally so legacy code keeps compiling.  **Known gap — CLOSED 2026-08-20 (was: RE-VERIFIED 2026-07-10 on BOTH backends — identical output, a shared semantic defect rather than a parity bug: the auto-wrap path parsed but DROPPED diagnostics).**  The probe below now reads: A reports the parse error, B is unchanged, and C is EMPTY after a successful parse.  See the Q1 row in § Open work.  Probe (`struct Cfg { name: text, port: integer }`, malformed `"{ this is not json"`):

```
A  Cfg.parse(bad)               → json_errors() == ""        (len 0)   ← silent
B  Cfg.parse(json_parse(bad))   → json_errors() == "parse error at line 1 col 3 …"
C  Cfg.parse(good)   [succeeds] → json_errors() == B's error (len 128) ← STALE
   …and c.name == "x", c.port == 7, so the parse really did succeed
```

So malformed input and schema mismatches leave fields null with `json_errors()` **empty** (A), and — worse — after a **successful** parse `json_errors()` still returns the *previous* call's error (C), so a program that checks `json_errors()` to validate a parse **reports failure on correct data**.  The two-stage form `Struct.parse(json_parse(text))` reports both classes correctly.  This is the remaining Q1 shape, and the only genuinely-open row in ROADMAP § S.  All 25 P54 + Q1 acceptance tests green.  Boolean allocator-corruption fix carried forward (`database(elem_size.max(2))` for handle stores).  **All JSON natives ship natively as of 2026-04-14 (commit `7a2329e` cleared `NATIVE_SKIP` and `SCRIPTS_NATIVE_SKIP`)** — `n_json_parse`, `n_json_array`, `n_json_object`, `n_to_json`, `n_to_json_pretty`, `n_kind`, `n_keys`, `n_fields`, `n_has_field`, `n_struct_from_jsonvalue`, etc. all dispatch through `src/native.rs` and run through `cargo nextest run --release --test native` cleanly.  The user-facing typed-impl refactor (making `MyStruct.parse(text)` enforce text-must-be-JSON typing at compile time instead of routing through the runtime auto-wrap) remains an optional follow-up — orthogonal to the JSON correctness work.  `p54_struct_parse_rejects_plain_text` was deleted (tested a rejected design decision). |
| Q1 | `json_errors()` reports byte offset only — no path, no line:column, no context snippet | Medium | **Q1 COMPLETE 2026-04-14** — parser side: RFC 6901 path + line:column + context snippet with caret, all 5 `p54_err_*` acceptance tests green; 8 unit tests in `src/json::tests`; 6 `q1_*` tests for state-clearing.  Schema side: kind checks live inline in the unified `n_struct_from_jsonvalue` walker — primitive fields receiving a wrong JSON variant (and not `JNull`, which signals "absent field" and stays silent) push a `"<Struct>.<field>: expected <KKind>, got <KKind>"` diagnostic to `json_errors()`.  Symmetric across direct fields and `vector<struct>` element fields (same walker code path).  6 `q1_schema_side_*` tests covering type-mismatch, missing-field-silent, clean-parse, vector-element mismatch, text-receiving-number, boolean-receiving-string. |
| Q2 | No free-form object iteration / key listing / quick `kind(v)` peek | Medium | **Q2 COMPLETE 2026-04-14**: `kind` + `has_field` + `keys` + `fields` all shipped with real JObject walks.  `keys` returns field names in insertion order; `fields` returns name + value pairs with full deep-copy (primitives and container values preserved).  See § Q2 below |
| Q3 | No `to_json(v)` serialiser — reads but can't write or round-trip | Medium | **JsonValue side complete 2026-04-14, T.to_json() complete 2026-05-07.**  JsonValue: `to_json` walks all six variants (primitives, empty containers, non-empty containers, nested containers) — full tree serialisation.  `to_json_pretty` adds 2-space indent + one-element-per-line for non-empty containers (empty stay `[]` / `{}`; `"k": v` with single space after colon).  `T.to_json()` / `T.to_json_pretty()` for any user struct ship via the parser-side intercept in `src/parser/fields.rs::field()` lowering to `n_struct_to_json(self_ref, struct_kt)` — which delegates to `Stores::show_json` reusing the existing `ShowDb` schema walker (`json: true` flag flips text → JSON-escaped, field names → quoted, struct-enum variants → `{"VariantName": …}`, JsonValue fields → semantic subtree).  See § Q3 below |
| Q4 | No way to construct `JsonValue` trees in loft code (fixtures, mocking, forwarding) | — | **Q4 COMPLETE 2026-04-14**: all six constructors ship with real behaviour.  `json_null` / `json_bool` / `json_number` / `json_string` wire the primitives directly; `json_array` / `json_object` deep-copy caller-supplied items/fields into a fresh arena via a shared `dbref_to_parsed` + `materialise_primitive_into` helper, handling nested containers.  See § Q4 below |
| P54-U | Two JSON parsers (`src/json.rs::parse` for the new JsonValue path, `src/database/structures.rs::parsing` for legacy `text→struct` direct write) accept different dialects and have different diagnostic surfaces | Medium | **All three phases LANDED.**  Phase 1 (2026-04-14): `Dialect::Strict` / `Dialect::Lenient` enum + `parse_with(input, dialect)` in `src/json.rs`.  Phase 2 (2026-04-14): schema-driven `walk_parsed_into` + `walk_parsed_struct` + `walk_primitive_into` in `src/database/structures.rs`; `Stores::parse` / `parse_message` route through the unified parser; instrumentation confirmed zero success-path fallback hits across the full test suite.  Phase 3 (2026-05-07): legacy hand-rolled scanner already gone (Phase 2 superseded it); `Stores::parse` now ONLY uses the unified `parse_with` + walker path with no fallback (single error format `"line N:M path:X"` via `format_walk_err`).  Plus walker `at`-position threading: `walk_parsed_into` / `_struct` / `_primitive` take a byte-offset hint that struct field calls populate with the field's `key_at`, so leaf type-mismatches now report the field's real position instead of byte 0.  Pinned by `tests/data_structures.rs::p54u_leaf_mismatch_reports_field_position`. |

Items that look open in the historical sections of PROBLEMS.md /
CAVEATS.md but are now closed: P22, P91, P135 / C58, P137, P139, C60,
INC#3, INC#29, P140 (test-harness reordering 2026-04-13), P141 (false
positive — `x#continue` already works), B3 (ref_return Enum arm
2026-04-13 — struct-enum return types now get hidden caller
pre-alloc args just like Reference/Vector), B2-runtime (2026-04-13
— 4-part fix: fill_all enum-field retrofit, parse_constant_value
v_block wrap, fields.rs Sig.Idle wrap, calc.rs sub-struct size=1),
B7 (2026-04-13 — `def_code` null-code path now sets `self.arguments`
and `stack.position` from def attributes for native fns + native
registry aliases `t_9JsonValue_<method>` → `n_<method>` impls),
**C54** (2026-04-21, @PLAN01: `integer` widened to i64 end-to-end;
`long` keyword + `l` suffix removed; 34 duplicate `Op*Long` opcodes
reclaimed), **P184** (2026-04-22, @PLAN02: narrow integer aliases
inside `vector` / `hash` / `sorted` / `index` now honour `size(N)`
via the Option L-minimal `Parts::{Byte, Short, ShortRaw, Int}`
variants), **P185** (2026-04-23, @PLAN05: slot-aliasing SIGSEGV
closed by retiring `place_orphaned_vars` and extending the main
IR walk), **B5** (all layers landed — Layer 1 + 2 on 2026-04-14,
Layer 3 closed as a side-effect of the struct-enum return work
landed in #168→#174; `p54_b5_recursive_struct_enum` is
un-ignored and passing).  See CHANGELOG.md.

---

## Open work — actionable summary

Items below are "what to BUILD" derived from the design content in this document.  Each row links to the section that holds the full design.  Three clusters: JSON, Native runtime, Compiler-blocker.

### OPEN WORK — the rule-tag / duplication thread (2026-08-24)

The concrete queue for the thread that runs from `formal/IMPLEMENTATIONS.md`.  Each row names
the next ACTION, not the topic.  Status: ☐ open · ⚠ needs a decision · ✅ done.

**Where this thread came from, so the queue below reads as more than housekeeping.** The premise
is an owner diagnosis, not a metric: *"most of the code in loft is not from an early
implementation but from fixing bugs, and during that bug fixing a lot of duplications were
written without design — so we have structure, but in that structure we have multiple
implementations, often in multiple files."* The response has three parts, and two are done:

1. **Anchor duplication on the RULES, not on the code** — a formal rule is the thing two
   implementations are both claiming to implement, so it is the only stable place to ask "is this
   the same question?". Rules therefore needed unique names: `@FR-<Rule>` tags, defined in
   `formal/`, cited from the code, resolved by `scripts/rule_tags.py`, indexed by `./scripts/idx`,
   and gated by `doc_hygiene::every_rule_citation_resolves`. ✅ shipped.
2. **Work the duplications the tags expose** — the eight-family checklist in
   `formal/IMPLEMENTATIONS.md`. ✅ all eight evaluated; four merged, four split with the reason
   recorded. The split verdicts matter as much as the merges: three of the eight were *different
   questions sharing a variant list*, and merging them would have been the early-abstraction
   failure the diagnosis warns about.
3. **Make the sensor fire without being asked** — the blind spot is activation, not capability
   (*"if you think about the issue you will do the correct thing; but without it you build a lot
   of correct things and still miss the correct design"*). Trigger lines now live in
   `design-protocol`, `engineering-rigor` and `loft-codegen`, and each duplication question has a
   TOOL so the answer does not depend on remembering. ⚠ the one unmeasured part is whether the
   triggers actually fire — see C.

**State in one line:** the checklist is finished, the tooling is in place, and the two dead IR
variants are removed (B3); what remains is three spec decisions (B), one unrun measurement (C),
and a branch with **no PR**.  The `unspan` queue is CLOSED (B4h): all 16 sites are measured,
gated, or read, with one latent decision change found and no defect.  The catch-all walker
queue is OPEN and now ranked — 55 of 124 have a fallback that answers no (B6b); **eight are
worked and eight more triaged by the boundary question**, two of which yielded shipped
bugs (B4i/B4j, and the projection pair in B6e).  B6d has the reading rule that ranks the rest: ask whether the doc says why the
FALLBACK is right.  One site (`body_has_buffer_return`) is handed to the sibling checkout rather
than worked, because its machinery is in flight there.
The catch-all backlog is no longer blocked on ranking — `reach` says 125 of its 126 sites are
code production runs (B6b), so it is a read-one-at-a-time queue rather than something a filter
will shrink.  **A SECOND queue now runs beside it (B6g), and it is the sharper one:** the
catch-all list asks who forgot a variant, while `spellings` asks who can only see one of a
notion's two IR spellings — **38** functions resolve a projection by OP NAME and **5** of them
handle `TupleGet` (18 · 2 when B6g wrote this; the SCREEN was widened in B6i, not the family).  Following it produced three defects in one pass, two fixed here and one
filed as a design question (**loft#1102**: a tuple literal ALIASES a heap local while a struct
literal and a vector literal copy it).  Reading the queue a second time produced a fourth
(**loft#1104**, B6h): a tuple-element ARGUMENT cannot witness the @P290 bracket, so a
borrowed-view call leaks one record per call.  **That issue is now closed across its whole matrix
(B6i)** — the two "open" cells rested on a premise that did not survive re-measurement, and
moving the axes the first sweep held fixed found three more before the fix.  Widening the list
those cells run through then exposed a defect with **no tuple in it at all**: `pick(h[k], …)` at
every keyed kind leaked, because `is_projection_op` — the site whose own doc calls itself the
single home — was short by the two ops that three other homes already carried.  Fixed by merging
the homes; four hand-spelled lists remain, as a queue.  Two shapes are filed rather than cured: **loft#1105**, whose obvious cure turns the leak into a
use-after-free, and **loft#1106**, where binding the argument does not help at all because the
gap is in what OWNS a nullable record, not in what can witness it.  Probing #1106 then found the
class ONE LEVEL DOWN — a notion with two TYPE spellings — where a nullable struct FIELD is
`Optional(Reference(S))` on pass 1 and the synthetic `Enum(__nullable<S>)` on pass 2, because the
rewrite that produces the second runs BETWEEN the passes.  Four failures from that one root, two
of them silent wrong answers and two of them legal programs refused; all fixed (B6j), with the
predicate given one home and the 56 hand-spelled recognitions of it left as a ranked queue.
**B6s then measured the floor B6p had left**: ten more opaque verbs over 198 bare call sites,
**eight** `(verb, caller)` pairs — five of which only a second ENTRY POINT can see, because the
dump renderers are on the diagnostic path a passing program never takes — three defects fixed,
two duplications merged, and one one-home peel built, measured to diverge on `--native`, and
BACKED OUT with the measurement written at the site.
**B6t read the first of `spellings`' 17 never-read sites** and the yield came from one former
UP: a tuple MEMBER is not parsed against its declared type, so six shapes the return and
argument positions accept are refused — or ICE — in a declared local.  Two independent causes,
each of which alone reads as the whole cure; both fixed.  The census under it is the entry's
real product: the `⇐` expected-type channel has **ten push sites carrying six different
admission lists**, and `Type::Tuple` is in none of them — which is why the same shapes still
fail in a `return` and in an argument, filed as **loft#1122**.  A `--native`-only silent wrong
answer found beside it is **loft#1123**.
**B7g walked `@FR-G-Mono`, picked because the bug review names generic/monomorph as the
sharpest RISING class and because `formal/interfaces.md` carried the same unmeasured `OPEN: 0`
sentence `formatting.md` had.** One question — relate a template type to a concrete one — had
**five homes carrying four different lists** of which `Type` formers to descend, and the
DECLARATION read the rule while the CALL did not: `fn f<T>(x: T?, d: T)` was accepted where it
was written and reported as *"Unknown function"* at every call, at every type.  The corpus is
why nothing saw it — **166 generic declarations in the tree and every one puts a bare `T` or a
`vector<T>` first**.  Closed by deriving all four from the keystone; unlocking the refused
shapes then produced three more (loft#1175, #1176, #1177), one of which is refused rather than
shipped because its cure turns a refusal into a crash.
**B6w walked the issue B6v had FILED (loft#1134) and found the report inverted**: the route it
called broken was the only correct one, and the two it called correct were two mistakes
cancelling.  The declared layout is what settles such a question — a `LOFT_DUMP_TYPES=1` dump,
not a comparison of the two programs — and it also promotes the defect: *"one field high"* was
the milder half, because the discriminant was aliased onto the payload's first byte and a
PRESENT `S { a: 0, … }` therefore read absent.  Three write sites and two read sites, two shared
homes; the two side findings are **loft#1138** (not tuple-specific — the tag is dropped at every
function boundary) and **loft#1139** (a legal program refused).

#### A — rule-tag adoption (`scripts/rule_tags.py`, `idx tag:@FR-…`)

13 of 285 rules cited, across 24 sites at the time of writing (`python3 scripts/rule_tags.py
check`; see the note under B4c — the DENOMINATOR has since changed).  Each row is one
family from the checklist; the work is *read each site, decide which rule it enforces, cite it* —
and the reading is the point: **four of the eight families split rather than merged**, and each
split is a merge that would have coupled two rules that must stay free to differ.

| # | family | sites | next action |
|---|---|---|---|
| 3 | **is this a KEYED collection** | 16 | ✅ **merged onto `vectors::is_keyed`** — one home for five variants. Exposed rules gap B1 (no rule names the keyed family as a category) |
| 6 | **narrow integer widths** | 12 | ✅ **evaluated — TWO questions, not one** (stored width vs variable-slot representation); split, not merged. Exposed rules gap B2 |
| 2 | **is this carried as a DbRef** | 43 | ✅ **closed** — 3 sites wrong (2 real bugs, both fixed), the other 40 cleared by a corpus-wide SENTINEL rather than by reading. Only 4 ever see a keyed collection, and all 4 probe correct against their own documented failure modes |
| 4 | **is this a collection** (keyed `+ Vector`) | 13 | ✅ **merged** onto `vectors::is_collection` (the one that DERIVES it). Three homes existed, not one; IR byte-identical 854/854 |
| 5 | **is this DbRef-represented** | 11 | ✅ **merged onto `data::is_dbref`**, IR byte-identical on 854/854. Turned up a duplicate home (`Parser::is_heap_handle`) that `is_dbref` itself had duplicated — see IMPLEMENTATIONS.md |
| 7 | **the value-carrying `Value` wrappers** | 59 | ✅ **evaluated — FOUR questions, not one**; not mergeable (arms, not predicates). One omission documented as deliberate. ⚠ its "one real gap fixed" claim was **wrong and is corrected** — `walk_check`'s missing `BreakWith` arm was unreachable, see #8 |
| 8 | **which `Value` shapes hold a statement list** | 13 | ✅ **evaluated — the merged home already exists.** Not a merge: the two arm-sets differ only by whether `Call` shares the body. The finding is one level up — `Value::for_each_child` claims *every* traversal derives from it; measured **31 do, 22 are exhaustive, 127 are a partial match + `_` catch-all**. And **two variants have no producer at all** (`BreakWith`, `ParFor`) — see IMPLEMENTATIONS.md |
| 1 | scalar — the 5 remaining BARE sites | 5 | ⚠ adopting `is_scalar` ADDS value enums at each: a behaviour change per site, one probe each. Not a sweep |

#### B — rules gaps found by citing (spec decisions, not code)

| gap | found by | state |
|---|---|---|
| **no rule names the KEYED FAMILY** as a category — `Col-Hash`/`-Sorted`/`-Index`/`-Spatial`/`-Trie` define one kind each, yet 16 sites tested the category | checklist #3 | ⚠ `is_keyed` cites all five as a stand-in. Minting a family rule is a spec decision |
| **no rule says a narrow value in a VARIABLE slot is a raw `i64`** — `L-Narrow` states the stored width, `L-Null` the field encoding; the `io.rs` pair depends on neither | checklist #6 | ⚠ the code comments the distinction at length; the rules cannot express it |
| `formal/binding.md` **OPEN: 1** — D-bind-11's heap-element half | pre-existing | ⚠ needs a representation choice; the record-backed path is proven to work (see the entry) |

#### B3 — DONE: the two producerless variants are removed (2026-08-24)

`Value::BreakWith` and `Value::ParFor` are gone, with `ParForBody`, 53 pattern alternatives and 39
whole arms across the walkers, both serializer shapes, the `IrNode` accessors, the store-schema
types, and the round-trip tests that were their only exercise. `make ci` green.
`scripts/ir_walker_audit.py dead` now reports **no dead variant at all** — the same instrument that
found them, used as the closing check.

`ParFor`'s own declaration had said what it was: *"spine step 3a lands the variant + walker arms
only. Steps 3b (codegen) and 3c (parser detection) follow."* They never did, and nothing recorded
that they had stopped — the scaffolding simply stayed, and every walker went on paying for it.

**Renumbering was safe, and that was checkable rather than a judgement call.** Removing
`NdBreakWith` (19) and `NdParFor` (33) shifts every later discriminant, but
`startup_cache::save_program` writes `cache::build_signature()` as the manifest's first line, so a
binary upgrade invalidates every stored bundle. `data_store::baked_layout_mirrors_loft_schema`
then proved the result: it failed on the first attempt naming `DISC_CONTINUE` 19-vs-20 exactly.

#### B3a — found on the way: the generated IR schema had drifted from its source

`src/ir_schema_gen.rs` is `@generated … DO NOT EDIT — regenerate` from `tools/ir_schema/ir.loft`.
It had been hand-edited anyway. `src/keys.rs::Key` gained a third field, `start: i32`, in loft#812;
the generated file was updated to match, **`ir.loft` was not**. Nobody regenerated afterwards, so
the two stayed out of step invisibly — and the first regeneration in this session silently dropped
`Key.start`, taking `KEY_STRIDE` from 24 to 16. The layout guard caught it.

Fixed by adding `start` to `ir.loft` (the source), then regenerating; a name-keyed comparison of
the regenerated schema against the committed one now differs by exactly the three removed types
and nothing else.

✅ **The gap this exposed is now CLOSED (2026-08-24).** `scripts/ir_schema_check.sh` runs the
pipeline for real and byte-compares the result against the committed file — ~0.1 s — gated by
`doc_hygiene::ir_schema_gen_matches_its_loft_source`, which shells out to the same script so the
gate and the tool cannot drift apart.  `make ir-schema-check` reports; `make ir-schema-regen`
rewrites the generated file.

The circularity turned out not to bite.  The check needs a built `loft`, but whatever state the
generated file is in, the binary compiled from it still parses `ir.loft` the same way — so a
hand-edit shows up as a diff, and an un-regenerated source edit shows up as one too.  **Both
directions were probed deliberately** (delete a `db.field` line from the generated file; add a
field to `ir.loft` without regenerating) and both fail with the offending line named.  Where
there is no binary or no `python3` it SKIPS with a line saying so, and the test surfaces that
line — a check that quietly did not run must not read as one that passed.

#### B4 — DONE: in-code docs state the CONTRACT, not the INCIDENT (2026-08-24)

**The owner's framing.** A feature and a formal rule are meant to be *timeless*; a bug is
relevant in the moment and stops being so. An algorithm should be documented **as an
algorithm** — what it computes, over what domain, under which invariant. The bug that
caused its rewrite may be *linked*, but it is a side effect, never the main body.

**Shipped:**

| where | what |
|---|---|
| `.claude/skills/doc-quality` | **rule 8** — document the contract, not the incident — plus the deletion test, the CONVERSION move, the regression-test carve-out, and a description that fires on "this used to" / "the bug was" / documenting right after a fix |
| `DOC_QUALITY.md` § B2 + **rule 9** | the reference: why the axis is distinct from rules 1–2, the worked `Key::start` rewrite, and an honest account of how countable it is |
| `scripts/lint_comments.sh` | a third pattern, `incident`, with its own report mode and thermometer line |

**The axis is distinct, and that is the point.** Rules 1–2 already ban past tense and
provenance stamps. A comment can pass both — present-tense, no date, about code that
exists — and still be organised around the bug that produced it. Rules 1–2 are about
*tense and bookkeeping*; this one is about *what the documentation is about*.

**The move is CONVERSION, not deletion.** Most incident narration is a timeless fact
wearing a story's clothes, and deleting it loses real knowledge. Extract the rule the
story contains. Worked through on ten sites: `Key::start`, both `@FR-L-Narrow` arms in
`native.rs`, `is_keyed`, `is_dbref`, `ref_tuple_element_ok`, `write_absent_value`,
`write_narrow_value`, and four the detector flagged. In every case the useful content
survived and got *more* useful — "`u8?` 42 read back `0`" became "a width missing from
this set falls to the catch-all, which leaves zero-init bytes, so the failure here is
always silent".

⚠ **The detector deliberately under-reports, and finding that out was the work.** The
first version matched a broad failure vocabulary: 1 023 of 7 967 doc blocks, 787 of them
invisible to the existing checks. That number does not survive inspection — `SIGSEGV` is
what `crash_report.rs` installs a handler for, `the hole` is Robin Hood hashing and the
lexer's unclosed brace, `silently dropped` describes a live spoof-check, `never reported`
is a contract, and `loft#885's hoisted reads` names a mechanism by its issue, which rule 2
explicitly KEEPS. The axis is semantic, not lexical. So the shipped pattern is narrow
(12 lines, now 8) and the doc says plainly that **the detector finds the loud cases and
the deletion test is the real check**. A noisy thermometer gets ignored, which is the
failure mode that matters here.

**Baseline handling worth knowing:** adding a pattern to `collect_raw` would have made
`--check` report ~490 "new" flags and drowned the signal. Baselining only the new axis
kept the **180 genuine old-pattern flags that have accumulated since the 2026-08-19
baseline** visible instead of silently absorbing them. Those 180 are unfixed and still
reported — the ratchet only works if someone prunes.

#### B4a — the `@FR-` cited sites, swept (2026-08-24)

All **17 comment blocks carrying the 24 citations** are done: every one now leads with the
contract, and the `Enforces @FR-X` line sits at the top of its block rather than trailing
after the prose. `rule_tags.py check` still resolves all 24.

**The rewriting was the smaller half.** Reading the cited sites in order — which is what a
tag route makes possible — turned up two things no comment lint could have found:

- **`parse_assign_op` had no doc comment at all.** Its description ("apply the operator to
  an already-parsed LHS, parse the RHS, rewrite into the assignment IR, returns
  `Type::Void`") was stranded three hundred lines away on `classify_vec_bind`, because
  that function had been inserted *between* the doc and the function it belonged to. So
  one function was undocumented and another carried a description of something it does
  not do — and rustdoc renders the wrong one without complaint. Both fixed.
- **A doc block split by its own attribute.** `ref_tuple_element_ok`'s citation had been
  appended *after* `#[must_use]`, so the source reads as two blocks where rustdoc shows
  one. Eight such splits exist repo-wide (`doc → attribute → doc`); this was the only one
  at a cited site, and it is fixed. The other seven are cosmetic and left alone.

**Worth generalising:** the tags are a *reading* route, not just a citation index. The
misattached doc had been there for as long as `classify_vec_bind` has existed, invisible
to every lint because both functions had *something* above them. Following the citations
one by one is what surfaced it.

#### B4b — cite, then sweep: the loop run twice (2026-08-24)

**13 → 21 rules cited, 24 → 38 citation sites.** Two tranches: `layout.md` (L-Struct,
L-Enum, L-Total, L-Tuple, L-Sound) and `coroutines.md` (G-Return, G-Call, G-Done).

**Citing is a stronger instrument than sweeping.** Writing *"this enforces `@FR-X`"*
forces you to name the rule and then check that the code in front of you is what the rule
says — and that check is what fails. The layout tranche alone turned up:

| found | what it was |
|---|---|
| **`L-Tuple` named a function that no longer exists** | The rule said element offsets are `element_offsets`. @PLN114 had split that into `element_stack_offsets` (stack view) and the storage view *specifically so a site must declare which it means* — and the rule still named the ambiguous one it abolished. The rule now names BOTH and states that their agreement is part of the rule. |
| **13 stale `element_offsets` references in comments** | Same rename, never followed through. |
| **1 stale reference in a runtime diagnostic** | `@PLN114 [{caller}]: … (element_offsets for {types:?})` — a `debug_assert!` message pointing an engineer at a function that does not exist. |
| **`calc.rs` named the wrong caller** | *"Called by `typedef.rs` during type resolution."* Its only caller is `Stores::finish_type`. |
| **`typedef.rs` claimed a call it does not make** | Its header said `actual_types` "compute[s] field positions via `calc::calculate_positions`". It does not — positions are assigned later by `Stores::finish`. Its own inline comments said so, contradicting its header. |

The stale rule is the one that matters: **a formal rule naming a renamed function is worse
than an uncited one**, because it reads as authoritative and sends the reader to a symbol
that is not there. Nothing but citing it would have compared the two.

**And a general lesson about the direction of the doctrine.** `formal/`'s rule is *"the
rules do not change to match the code; the code changes to match the rules."* That governs
SEMANTICS. It does not license a rule to keep a stale implementation NAME — updating the
name (and, here, splitting one view into the two the code now distinguishes) makes the rule
say what it always meant. Reading it as "never touch the rule" would have preserved the
error.

#### B4c — concurrency, interfaces, grammar, capabilities (2026-08-24)

**21 → 39 rules cited, 38 → 63 citation sites.** What each family exposed:

| family | found by citing |
|---|---|
| **concurrency** | `scopes::is_par_safe` and `par_unsafe_reason` have **no production caller** — 18 tests, `#[allow(dead_code)]`, and *"phase 5b proper hooks it"*. The same scaffolding-without-a-consumer shape as `Value::ParFor`. ⚠ **Not a rule deviation**, and getting that right mattered: `C-Impure` says an impure worker is UNDEFINED, not that the compiler must refuse it, so `concurrency.md`'s `OPEN: 0` is correct on its own terms. The analyser is the D8 *diagnostic* that was planned on top, never wired. Both now say so instead of implying a temporary state. |
| **interfaces** | `check_satisfaction`'s doc named `instantiate_generic` — a function that has **never existed under that name** in any commit. The real one is `try_generic_instantiation`, which had **no doc comment at all**. `parse_interface` still said *"semantic satisfaction checking comes in I5/I6"*; it shipped. |
| **grammar** | The precedence table carried one bare line of comment (*"Operators ordered on their precedence"*) for the thing that defines `G-Prec`, and **associativity is not in it at all** — it is decided where `parse_operators` hands the RHS a precedence. Both now cite, and the table says which rule it is *not*. |
| **capabilities** | Nothing stale — the cleanest family. `reachable_set`'s sandboxed / non-sandboxed split turned out to be the `Cap-Own` boundary and the `Cap-Trusted` leaf rule at once, which the citation now states. |

⚠ **I got a citation wrong and the check caught it.** I wrote that `Cap-Trusted` and
`Cap-Own` were "decided by the admission walk before it reaches here" — plausible, and
false: both are decided in `sandbox.rs` itself, by `reachable_set`. Verifying each claim
against the code before committing it is the discipline this thread exists to enforce, and
it is easy to skip precisely when writing the citation feels like documentation rather than
analysis. Two claims about `**` right-associativity and `as` were checked the same way and
held (`operators.rs:3513` states it outright).

**Open:** the wider surface is still not swept, by design. The remaining families are the
big ones — `types.md`, `binding.md`, `matching.md`, `heap.md`, `collections.md`,
`operational.md` — where a rule usually has many enforcement sites rather than one, so the
work per rule is larger and the payoff (a `sites <tag>` query that returns everything
enforcing it) is bigger.

⚠ **The counts in B4a–B4c are DELTAS, and their denominator moved.** Those sections were
written against a `rule_tags.py` that reported **285** defined rules; `main` has since
extended the script (deviation tracking) and it now reports **251**, with the citations
from this thread merged in at **45 cited · 74 sites · 35 deviations (2 open)**. The
13 → 21 → 39 progression is still the record of what this thread cited; the ratio to a
fixed total is not, so re-run the tool rather than reading a number off this page.

#### B4d — types.md and binding.md, and a rule the code contradicts on purpose (2026-08-24)

**45 → 60 rules cited, 74 → 92 citation sites.** The two biggest families, worked by
sub-family rather than rule by rule: `D-*` (defaults), `Const-*` (the const model),
`B-Disturb` / `B-Ref-Reshape`.

**The find: `(D-Opt)` says a nullable's default is `null`. It is not.** Measured on both
backends, so it is a rule/code divergence and not a backend split:

```loft
struct S { a: integer?, b: text?, c: boolean?, d: Colour? }
s = S {};                       // a=0   b=''   c=false   d=null
```

`data::to_default` answers the BASE type's zero and says so outright — base-zero is *"the
settled design call"* (@PLN25), because a bare `null` renders as native unit into a scalar
slot (E0308). So a decision was taken in code and the rule was never updated to match.

⚠ **This one I did not resolve, and that is the point.** `L-Tuple` (B4b) was a stale
implementation NAME — semantics unchanged, so correcting it made the rule say what it
always meant. `D-Opt` is a SEMANTIC disagreement, which is exactly where the doctrine says
the code yields and the rule stands. Editing the rule to match the code would have been the
same act as `L-Tuple` in form and the opposite in substance. Recorded instead as
`formal/types.md` § `D-Opt-Zero`, **flipping that doc's `OPEN: 0` to `OPEN: 1`**, with the
two admissible resolutions named and neither taken. The register already carried the lesson
from last time: *"the answer to the 'OPEN: 0' line above having been too strong"*.

**A near-miss worth recording.** `B-Disturb` is about what invalidates a `&` reference —
remove, re-key, reassign the container. I was about to cite it on `scopes::collect_move_
disturbed`, which is a DIFFERENT question that shares the word "disturb". Reading the rule
before writing the citation is what caught it; the name similarity is the whole trap. It
went to `collect_views_to_materialise`'s remove-detector instead, and `B-Ref-Reshape` to the
tuple-place refusal in `parser/expressions.rs`.

**Also undocumented at a rule site:** `data::to_default` — the one home for
`construct_default`, six rules — had no doc comment at all. Same shape as
`try_generic_instantiation` in B4c.

**Open:** `types.md` still has ~43 uncited rules (the `N-*` nullability family alone is 17)
and `binding.md` ~17 (the `B-Ref-*` family is 11). Both are worth doing by sub-family the
same way; both are larger than a single sitting.

#### B4e — DONE: a nullable field starts null (2026-08-24)

**Owner's call on `D-Opt-Zero`: the rule stands.** A nullable value in a field is null at
the start, so the code changed and `formal/types.md` is back to `OPEN: 0`.

`data::to_default`'s `Optional` arm now builds the base type's null SENTINEL through a new
`data::to_null`. Nothing had to be invented: the `OpConv…FromNull` ops already existed and
already produced the same sentinels the runtime's `Stores::set_default_value_nullable`
writes (`i64::MIN`, `255` for the tri-state boolean, `char::from(0)`), so the compile-time
and runtime paths now agree instead of contradicting each other.

**The E0308 objection in the old comment was real but misdirected.** A bare `Value::Null`
does render as native unit into a scalar slot — the answer is the TYPED null op, not the
base's zero.

⚠ **`to_null` is narrow on purpose, and the first version was not.** Routing EVERY base
through an op leaked: `OpConvRefFromNull` reserves a frame, so a nullable collection field
got a store nothing frees (`heap-value-as-a-condition.loft`, caught by `wrap.rs`'s leak
gate — `--tests` does not leak-check). For a handle-carried base or an enum the zero
already IS the null and costs no allocation, so those delegate to `to_default`. The
distinction is the same one the old shortcut got right by accident and wrong everywhere
else.

**What the old behaviour rested on, checked.** The code cited *"an omitted field gets the
zero value for its type (LOFT.md § constructors; 06-structs.loft locks it)"*. `LOFT.md` has
**no such section** and does not contain that sentence; `06-structs.loft` declares **no
nullable field at all**. Both citations were wrong, and the only thing actually locking the
old answer was one half of `issue_332_nullable_narrow_field_null_roundtrip`.

**Blast radius: 3 tests, all asserting the old rule**, now asserting the new one —
`issue_332_…` (strengthened to omitted / set / re-nulled across three widths),
`875-json-absent-text-field.loft` (a plain `text` still answers `""`, a `text?` answers
null, which makes that guard *more* coherent), and `931b-i32-accepted-forms-run.loft`
(whose real subject is that the literal is ACCEPTED, unchanged).

**Guard:** `tests/scripts/nullable-field-starts-null.loft` — 6 functions, both backends,
every width `i8?…integer? float? single? boolean? text?`, the explicit `= null` spelling,
enum/reference/collection bases, a partial literal, and a CONTROL that non-null fields still
take their zero (a fix that nulled everything would pass every other cell).

**Gates:** `make ci` 4434/4434, and the published-library gate
(`scripts/revalidate_libs_local.sh`) **41 pass, 0 compile-break** — required because
`make ci` green does not cover a language semantic change. One library SKIPPED (`drawing`
0.2.0, tag missing in `loft-libs-graphics`) and a skip is not a pass.

⚠ Not opened here: `character?` cannot represent absence distinctly — its null is
`char::from(0)`, the same as its zero, and the runtime's content-type-6 arm ignores
`nullable` too. Consistent between both paths, so not a divergence, but it is the one base
where the rule cannot be observed.

#### B4f — the catch-all walkers, measured (2026-08-24)

B2 asked the honest question and it is now answered: **for each catch-all walker, which
omitted edges are REACHABLE?**

**Raw omission counts are noise.** 130 walkers descend into some child-bearing `Value`
variant and not all, and ranking by (walkers omitting × corpus frequency) puts `Call`,
`Set`, `Span` and `Block` on top — but most of those omissions are deliberate. The useful
question hid inside one of them.

**`Span` is different, because there is a RULE about it.** `Value::unspan`'s own doc:
*"Every second-pass site that pattern-matches a specific Value variant must call
`code.unspan()` first. Without this, the per-site wraps silently break optimisations that
rely on the unwrapped shape."* That turns a vague worry into a checkable predicate:

| sites discriminating on 2+ specific `Value` variants | peel `Span` | neither |
|---:|---:|---:|
| 412 | 388 | **24** |

Joining the `@FR-O-Owner` walk onto the loft#1389/#1390/#1392 tree re-measures it once more:
**408 · 384 · 24** — neither side's number, as every join so far.  Joining @PLN154 (the stack
shadow) and loft#1397's lint on top: **410 · 386 · 24**, both additions on the peeling side;
loft#1388's capture handover adds one more of the same kind — **411 · 387 · 24** — and
loft#1396's `value_view_container` one more again: **412 · 388 · 24**.  The `@FR-O-Complete` walk (B7u) added one peeling site — `scopes::adopted_work_refs` reads a
right-hand side's `If` arms, `Block` and `Insert` tails through their `Span` to find the
construction work-refs a binding adopts.  loft#1356 added two peeling sites (the eager factory's tail scan reads a `Return` and a `Set` through their `Span`), loft#1362 two (`scopes::in_place_rebuild` reads the statement-level `OpDatabase` through its `Span`, and `copy_hands_off` walks a nested destination place through each level's), loft#1357 one, and the projection-view marking one (`scopes::nullable_view_locals` reads each `Set`'s source through its `Span` to match a `Value::TupleGet` or a projection `Value::Call`) — the statement scan in `scopes::convert` takes a `Span` off an `if` whose condition consumes a `??` temp, so it can put the evaluated condition back under the same position.  The `@FR-O-Witness` walk (B7v) added two peeling sites — `scopes::sink_set_into_arms` reads an `if`/`match`'s arms, `Block` and `Insert` tails through their `Span` to lower a value-branch reassignment to the statement form.  `scripts/ir_walker_audit.py unspan` re-measures it, and
`doc_hygiene::quality_unspan_table_matches_the_audit` fails if this row and the tool disagree.
It moved from 384 · 360 to 385 · 361 with loft#1354's `arm_moves_a_live_tuple_local`, which
discriminates on `Value::Var` and `Value::Block` to find the local an `if` arm hands over — it
peels first, so it lands on the seeing-through side and leaves the opaque column where it was.

⚠ **The row moved from 344 · 327 · 17 to 356 · 332 · 24 with nothing about the code changing,
and the reason is worth keeping: the census USED TO DEPEND ON FORMATTING.** `DISCRIM` ends in
`(?:=>|\||\)\s*=)`, and its `|` alternative was matching the first bar of a boolean `||` — so
`if def.code == Value::Null || f()` counted as discrimination and the same test on its own line
did not. Rewrapping one condition in `scopes.rs` moved the published total by one. An equality
test against a variant discriminates on it exactly as a pattern does, and a `Span` defeats it
the same way, so `EQ_TEST` now recognises it by design rather than by accident. The +12 and the
+7 are sites that were always in scope and never counted; nothing regressed to produce them.
The figures below the fold were measured against the NARROWER matcher this audit shipped with
(221 · 211 · 10); B4g says what widening it added, why the backlog grew without anything
regressing, and which site the measurement then took back off it.

loft#1186 moved it to 336 · 318 with the two predicates the join reading needed —
`parser::tail_joins_with_a_place` and `parser::node_place_root`, both of which unspan the node
before they match, so they land on the peeling side and leave the opaque column where it was;
loft#1185 to 337 · 319 with `parser::tail_calls_a_fnref_parameter`, which unspans for the same
reason.

loft#1329 moved it to 377 · 353 · 24 with `use_analysis::collect_yielded`, the new reading of
which sub-values a right-hand side can evaluate to; it discriminates five variants and unspans
at every level, so it lands on the peeling side. `use_analysis::fnref_target_in` entered the
table in the same change by gaining an `Int` arm beside its two marker arms — and it entered
the OPAQUE column, because it matched nodes `collect_yielded` had already unspanned. That is
the shape this audit is worth having for: a site correct only because of what its one caller
does. It peels for itself now, so the opaque column is where it was.

loft#1332 moved it to 378 · 354 · 24 with `scopes::first_use_of`, the liveness reading that
decides whether a loop-body local is READ after its loop or merely mentioned there. It
discriminates eight variants — `Set`, `Var`, `Block`, `Insert`, `Loop`, `If`, `TupleGet` and
`TuplePut` — and unspans at every level, so it lands on the peeling side and leaves the opaque
column alone. Its two orderings are the reason it must peel: a `Set`'s value is read before its
target, and a `Loop` body may run zero times, so a wrapped node that fell to the catch-all arm
would answer "no use here" for exactly the shapes the question is about.

loft#1331 moved it to 379 · 355 · 24 with `scopes::repointed_literal_accumulator`, which asks
whether a statement leaves a literal-backing accumulator naming a store the frame does not own.
It discriminates `Block`, `Var` and `Set` and unspans at each, so it lands on the peeling side.
The `Set` arm is the one that must peel: the shape it looks for is an assignment of anything
other than `null` to the accumulator, and a wrapped node falling to the catch-all would read as
"not repointed" — the answer that leaves the scope-exit sweep freeing a capture.

loft#1200 moved it to 339 · 322 · 17 with `scopes::nullable_locals_that_displace`, the
pre-scan that decides whether a nullable heap-record local is worth an ownership witness; it
unspans before matching `Set`, so it lands on the peeling side and leaves the opaque column
alone.

loft#1245 moved it to 346 · 329 · 17, and the pair it added is the audit's own subject: both
`use_analysis::callee_of` and `use_analysis::call_return_frees_source` went from naming ONE
`Value` variant to naming two (`Call` and `CallRef`), which is what put them in scope at all.
Both unspan first, so the opaque column is unchanged. The defect that fix closed is the same
shape one variant further out — not a missing `unspan` but a missing VARIANT, a call spelled
`CallRef` that five readers of "is this a call?" could not see. This audit cannot ask that
question (its predicate is `Span`-peeling, not variant coverage); `ir_walker_audit.py
spellings` is the one that can, and the general lesson is in
[IMPLEMENTATIONS.md § One notion, how many SPELLINGS?](formal/IMPLEMENTATIONS.md).

loft#1194/#1195 moved the table twice: 337 · 319 · 18 → 337 · 320 · **17**, then
338 · 321 · 17 as `parser::field_place` entered it — a new site that discriminates `Var` from
`Call` and peels at every level, so it lands on the peeling side and leaves the opaque column
alone. The OPAQUE column moving at all was the first time in a while, and the way it
happened is worth keeping. The comprehension fix added an `unspan`ing match to
`vectors::build_comprehension_code`, and the table moved on its own — because a site here is a
FUNCTION, so one peeling match reclassified a function whose actual blind spot was somewhere
else in it. That blind spot was real: the @P325 coroutine-termination detection matched `Set`,
`Call` and `Var` through `for_next` with no peel at any of the three levels, so a `Span` around
any of them loses the generator var, the loop loses its break, and the unbounded append @P325
closed comes back. It now peels at all three, so the 17 is earned rather than masked. ⚠ The
general lesson is the audit's granularity: adding an unspan to a function can move the column
without fixing anything, so a row that improves is a claim to check, not a result to record.
⚠ **A site can ENTER this table by gaining an arm, and the newest one did.**
`parser::rewrite_generic_type_defaults` discriminated on `Block` alone until loft#1175 gave it
a `CallRef` arm; two variants is the threshold, so it arrived as the eighteenth.  It descends
rather than peels, and cannot be hidden by a wrapper for that reason: its fall-through arm is
`Value::for_each_child_mut`, which treats a `Span` as a child and hands the walk the node
underneath.  Peeling there would be worse than redundant — the walk rebuilds what it visits,
so unwrapping would drop the position the `Span` carries.

loft#1205 moved it to 340 · 323 · 17 with the two predicates the `?`-on-a-place reading needed
— `parser::peel_place_discharge`, which tells a temp-bound `ncc` block from a bare null-check
`if`, and `parser::place_store`, which tells a local from a heap read.  Both unspan before they
match, for the reason the column exists: each is deciding what an assignment WRITES, and a
`Span` that hid the shape would leave the statement writing nothing.

loft#1236 adds one more peeling site — `source_names_a_collection`, which asks what an append's
source IS and therefore has to look through a `Span` to see it — for 346 · 329 · **17**.

loft#1227 moved it to 343 · 326 · **17** with `use_analysis::GroupAppends::collect`, which tells
a `Span`, a `Line` marker, a nested `Block` and an `OpNewRecord` call apart while walking one
block's statements.  It peels, and the third column is unchanged.

loft#1225 moved it to 342 · 325 · **17** with `parser::keyed_place_materialise`, which tells a
VARIABLE destination from a TUPLE ELEMENT one — the two need different builds, because a
variable is repointed by `OpDatabase` directly and a tuple element is a slot that has to be
filled through an accumulator and a `TuplePut`.  It unspans for the same reason the two above
do: it is deciding where a collection gets BUILT, and a `Span` hiding the shape would answer
`None` and leave the write with no store to land in.  The third column is unchanged, which is
the half of this row that matters — a new site that peels is neutral, a new site that does not
is the finding.

**Six false-positive classes, and 41 → 10.** The precision work and the fixes are separate,
and conflating them is how a backlog gets "cleared" with nothing fixed:

| | count | |
|---|---:|---|
| shown IMPOSSIBLE | **22** | the audit could not tell which `Value`, or that something upstream already peeled |
| peeled | **6** | of which only **2** change an answer: `needs_pre_eval` (1264 sites) and `const_eval` (2 lost folds) |
| descended, not peeled | **3** | `par_unsafe_reason`, `collect_callees`, `walk_deep_parent_write` — see B6 |
| still listed | **10** | of which 5 are measured clean and left unpeeled, 1 is gated, the rest unmeasured |

⚠ **The three in the middle row leave by a different door, and the row is there so the
arithmetic closes.** They were fixed by DESCENDING (`for_each_child`), not by peeling, because
what they were missing is a `Return` ARM rather than an `unspan` call. B6 is where they are
worked; keeping them in their own row is the same separation the two columns above exist for —
a count that moves for two different reasons, reported as one, is how a backlog reads as
shrinking with no fix behind it.

⚠ **"Peeled" is not "fixed", and the difference is most of this table.** Of the six,
`find_first_ref_vars` changes 46 decisions and gains 0 variables; `move_rewrite` and
`substitute_var` are measured behaviour-identical over 120 and 150 corpus programs; and
`sandbox::scan` needed no peel at all — the version I first wrote there was a double-count
that made the analysis worse. The honest yield of the whole sweep is **two** sites whose
answer changes, one of which (`const_eval`) changes it twice in 858 programs.

Where the 22 went, by mechanism: **7** matched inside another enum's path (`MValue::Scalar`,
`VariableValue::Long`, `ScalarValue::Single` all end in the substring the pattern looked for);
**5** discriminate on `host::Value`, a separate enum with no `Span` variant; **6** are the body
of an `any_node` / `for_each_child` / `walk` closure, which peel before calling the predicate;
**2** match on a span-transparent scrutinee (`match val.tail()`); **2** are test functions that
build their own IR. In order of
discovery: a non-IR `Value` (`host::Value`, `MValue`); a host-only variant name; a match that
is a traversal closure's body; a match whose scrutinee is span-transparent; a test function;
and — the one that subsumes the first — a match on an enum whose NAME merely ends in `Value`.

⚠ **That last one is the root cause the first fix only patched.** The pattern `Value::(\w+)`
matches inside `MValue::Scalar`, `VariableValue::Long` and `ScalarValue::Single`, and
intersecting against the IR variant set does not save you, because `Long` and `Single` ARE
real IR variant names — so `state::static_call` scored two and read as an unpeeled hazard
while discriminating on a debug enum. A lookbehind for a non-identifier character fixes the
class; it cleared four repl/debug sites at once. The lesson is the ordinary one about
substring matching, arriving late: I fixed the symptom (`MValue`) with an allow-list before
noticing the pattern was simply wrong. Each was found by hand-checking a site the list called a hazard and finding it
could not be one — which is the only way this list gets shorter honestly.

⚠ **The test-region filter is brace-balanced, and the cheap version of it is a trap.**
"Everything after the first `#[cfg(test)]`" looks right because test modules sit at the end
of a file by convention. `src/trie_db.rs` has one at line 245 of 1355, so that shortcut
discards 82 % of a production file. It happened not to move the headline here (20 either way)
but it corrupted the denominator, 212 against the true 226 — a shrinking backlog with no
fix behind it is exactly the failure this whole exercise is meant to avoid.

**A second false-positive class, and it held the two scariest-looking entries.** A match that
is the BODY of a `any_node` / `for_each_child` closure can never be handed a `Span`, because
`any_node` unwraps one before calling the predicate (`if let Value::Span(b) = self { return
b.1.any_node(pred) }`) and `for_each_child` descends through it. Four sites were that shape,
including the two whose failure direction is UNSAFE rather than merely lossy —
`hoist.rs::writes_store`, the vector-header hoist's safety predicate, and `scopes::guard_escapes`.
Neither can be reached with a `Span`. (A fifth, `walk_sub_rule_pure`, discriminates on `Purity`
and never on a `Value` at all.) The tool now strips those closures before counting.

⚠ **Do not diff two runs of this list by `path:line`** — a comment added above a site shifts its
line and reads as a removal plus an addition. Compare by function NAME. That is how
`is_void_value` briefly looked deleted when it had only moved eight lines.

⚠ **The count was 41 until 2026-08-25, and the 8 it lost then were never real** — the audit matched
the NAME `Value::<Variant>` without checking WHICH `Value`.  Three enums answer to that
spelling here: `host::Value` (the host-call ABI, 5 sites) has no `Span` at all, `MValue`
(2 sites) matched because `MValue::Scalar` literally contains the substring
`Value::Scalar`, and one more site matched neither.  The tool now intersects against the
IR enum read out of `data.rs`, and rejects any site naming a host-only variant
(`Void` / `Bool` / `Ref`) — needed because `host::Value` shares `Float` / `Int` / `Text`
with the IR enum, so the intersection alone still let those through.  Re-validated the way
[STABILITY_METHOD](STABILITY_METHOD.md) asks: it still flags `scopes::walk_check`, and
still does NOT flag `find_assigned_vars`, whose fix is the known answer below.
**A count offered as open work has to be right, or it is a bill someone else pays.**

**Instrumented, not argued.** One env-gated line in `scopes::find_assigned_vars`'s
catch-all, over 200 corpus programs: the path is reached **10 208 times**, of which
**2 dropped a Span-wrapped `Set`** — a genuinely missed assignment — and **8 dropped a
whole Span-wrapped `Block`**, whose statements then go unscanned. So the mechanism is real
and reachable, not theoretical.

⚠ **That method cannot measure a `#[cfg(debug_assertions)]` site, and a zero from it there
is an artifact.** Learned on `scopes::walk_check`, the top of the list: instrumenting its
catch-all and running the corpus reported **0** Span arrivals — and also 0 hits on the
catch-all at all, which is impossible for a walker that meets leaf nodes. The site is gated,
and `[profile.dev.package.loft] debug-assertions = false` strips it from `cargo build` and
`cargo test` alike (TESTING.md § Hang guard). Before believing a zero, count the *unfiltered*
hits on the same arm; if those are zero too, the probe never ran.

**Exactly 1 of the then-10 was gated** (`walk_check`) — so the method holds for the rest, and
the bound is worth stating rather than leaving as a general worry. It is only notable because
it is the top of the list by variant count, and so the natural place to start: the one site
where a zero was going to be believed.

**Two sites measured and FIXED 2026-08-25, and the counts are the point.** The raw arrival
count is not the defect count — at both sites most arrivals change nothing, so the measurement
that matters is *does unspanning change the answer*:

| site | spanned values arriving | answer actually changes |
|---|---:|---:|
| `const_eval` (858 programs, interp) | 1000 | **2** — folds lost in `91-null-coalescing`, `store_compact_kinds` |
| `generation::needs_pre_eval` (45 programs, native) | 1807 | **1264** |
| `generation::is_void_value` (45 programs, native) | 22 | **0** — left alone |

`needs_pre_eval` is the one that mattered: its arms single out `Call` / `Block` / `CallRef` /
`Insert` / `Iter`, all of which can answer TRUE, while a `Span` matches none and takes
`_ => false` — so a spanned call was reported as needing no pre-evaluation, which is the
double-borrow the analysis exists to prevent. `is_void_value` is the control that keeps the
rule honest: same file, same shape, and not worth touching.

⚠ **And then it changes nothing.** Adding the peel leaves the IR byte-identical on all six
affected programs. The variables were already covered another way. **So: reachable,
latent.** The peel stays — it is one word, it obeys the documented rule, and the
reachability is exactly what makes it a trap, since Span placement has moved before and the
failure mode is a missing initialisation with nothing to report it. But no defect was
found, and saying otherwise would be the dressed-up version of this result.

**What that settles.** The catch-all concern is no longer unmeasured, and the answer is
milder than it looked: the shape is real, the damage is not demonstrated. The remaining
sites are a ranked backlog to *measure* — several are display and host helpers where a
`Span` cannot arrive — and the tool now says plainly that a hit is a measurement to make,
not a defect found.

**The const-vector pair — measured clean, and the measurement nearly wasn't.**
`compile::build_const_vectors` and `generation::emit_const_vectors` are the same job in the
two backends, and both had the worst-shaped fallback in the list: `_ => {}` around a literal
match, so a spanned literal would write NOTHING and leave a zero in a const vector — silent,
not lossy. The feeding path never unspanned either (`extract_literal_values` takes an
`IrNode`, whose `unspan()` is an explicit call it did not make). Measured anyway: **both
sites are reached and see only plain literals, 0 spanned arrivals**, so const folding hands
them clean values and neither needed the call.

✅ **The shape is now gone, not merely measured harmless.** loft#1090 replaced the `Value`
payload with a `ConstField` enum, so both writers match EXHAUSTIVELY and neither has a `_`
arm to drop a value into — a kind added on one side can no longer be silently ignored by the
other. That defect was not hypothetical when the fallback existed: `boolean` and `character`
fields were being dropped from constants that were built anyway, so `[Row { flag: true, id: 5 }]`
read back `flag = false` with `id` correct. **The measurement above was right about spans and
blind to the kinds** — it asked whether a SPANNED literal could arrive and answered no, while
the arm was already discarding two ordinary literal kinds that did.

**`scopes::find_first_ref_vars` — the sibling of the site that started this, and the same
verdict.** `scan_if` calls it two lines above `find_assigned_vars`, for the same job, and only
one of the pair was peeled. Its arms are `Set` / `Block` / `If` / `Insert`, so a spanned one
took `_ => {}` and contributed nothing for that subtree. Reachable: 108 838 arrivals over 250
programs, 8 of them wrapping an arm. The peel changes the decision at **46 sites in 16
programs** — and newly pre-initialises **0** variables at every one of them, because the same
variables were already reaching `result` another way. Reachable, latent, peel kept. Measured
without an A/B build: the pre-fix code contributed nothing on that path, so running the
subtree into a scratch vector and counting what `result` did not already hold gives the gain
exactly.

**The sandbox batch — one of these was not a lost optimisation.** Measured over the full
858-program corpus (and, for the sandbox, a `[sandbox]`-policy program):

| site | arrivals | spanned around an ARM | verdict |
|---|---:|---:|---|
| `sandbox::intrinsic_space`'s scan | 188 | 6 | **no defect — see below** |
| `scopes::move_rewrite` | 567 | 41 | no defect; peel is a no-op |
| `const_eval::substitute_var` | 41 | 5 | no observable change; peel kept |
| `scopes::is_ref_materialisation` | 2329 | **0** | clean, untouched |
| `control::is_block_divergent` | **17 396** | **0** | clean, untouched |

⚠ **The sandbox entry was first written up as an admission bypass. It was not — the bypass was
mine.** `for_each_child` DESCENDS THROUGH a `Span` (`Value::Span(b) => f(&b.1)`), so a scan that
ends in `v.for_each_child(&mut |c| scan(c, …))` already visits a spanned node's payload one
level down. The bare `match v` was therefore correct. Peeling only the SCRUTINEE — `match
v.unspan()` — makes the node be counted **twice**: once from the peeled match, once when the
trailing walk reaches the same payload. That inflated a program's declared bound from `24 · n²`
to `36 · n²`, and I read the inflation as the fix rather than as the bug, wrote a regression
test asserting `36`, and shipped it green through `make ci` at 4444/4444.

**Nothing caught it, and the test I added was the thing that made it look verified.** No
existing test covered that bound; the one I wrote pinned the over-count as the requirement. It
only came apart when I tried to explain the mechanism afterwards and found `for_each_child`
already handled the case — i.e. when the A/B's *direction* stopped matching the story.

**Swept for the class, 2026-08-25.** Five functions in `src/` peel the scrutinee AND walk the
original binding. None is a live defect, and the reasons differ enough to be worth listing —
this is what the shape looks like when it is harmless:

| site | why it survives |
|---|---|
| `sandbox::intrinsic_space`'s scan | was the live one; now binds |
| `scopes::move_rewrite` | its walk sits INSIDE the `_` arm, and the inner `if let` reads the original on purpose |
| `scopes::elide_rewrite` | peels only to compute a `replacement`, then re-matches the original for recursion |
| `scopes::collect_def_order`'s walk | double-visits, but `or_insert` ignores the second write and `idx` feeds a relative order |
| `scopes::construct_prescan` | double-visits, and its accumulation is NOT idempotent — measured 77 first-appends over 45 files with exactly one mark, which is genuine (it survives binding the peel) |

The last is the one to watch: it is safe by measurement, not by construction. Both double-visiting
sites now carry a comment saying so, because the invariant that protects them — the accumulation
being idempotent — is nowhere else stated and is one edit from being broken silently.

**The rule this yields:** `match x.unspan()` is safe only when nothing else in the body walks
`x` again. Where there is a trailing `for_each_child` / `for_each_child_mut`, bind instead —
`let x = x.unspan();` — so the match and the walk see the same node and each is visited once.
The two forms are indistinguishable by eye and differ by a factor on the answer. `move_rewrite`
keeps its inner `if let … = node` deliberately unpeeled for the same reason.

**`is_block_divergent` is the strongest clean result here, and it looked like the worst.** Its
negated caller does `bl.operators[p] = null_value(…)` on the LAST operator when the block is
judged non-divergent — so a spanned `Return` would have been overwritten, destroying a return
statement. 17 396 calls across 150 programs, and the bare and peeled answers never once
differ. The structural fact behind it: spans DO appear among block operators (`find_first_ref_vars`
sees `Span(Block)` and `Span(Set)` there), but `Return` / `Break` / `Continue` are never
spanned. Worth knowing before anyone reasons from "a Span can be anywhere".

⚠ The first native run of that probe reported 0 and was VACUOUS — the site is exercised by
only 6 of the 858 corpus programs, and none was in the 60-program native sample. The
interpreter run had already fired 12 times, which is the only reason the zero was not
believed. **When a site is rare, sample the programs that REACH it, not the first N.** (All
six are dedicated `*-const-*` regression tests, so the feature is deliberately covered.)

#### B4g — the audit could only see ONE of the three ways to match a variant (2026-08-26)

The `unspan` backlog went from **10 to 20** without anything regressing (and to **19** once
one of the new entries was measured and peeled — see below). `Value::unspan`'s rule
is about *pattern-matching a specific variant*, and Rust spells that three ways; the audit's
regex recognised one of them.

| form | example | was counted |
|---|---|---|
| a plain arm | `Value::Call(nr, args) => …` | yes |
| a **guarded** arm | `Value::Call(nr, args) if data.def(*nr).name() == "OpGetField" => …` | no — the pattern stopped at the `if` |
| a **binding** | `if let Value::Call(d, args) = v` · `let Value::Block(bl) = v else` · `while let` | no |

Population **221 → 322**, unpeeled **10 → 20** (and to 317 · 16 once `map_nodes` joined the
traversal list — see below). Re-validated the way
[STABILITY_METHOD](STABILITY_METHOD.md) asks, on the two answers already known: it still flags
`scopes::walk_check` and still does not flag `find_assigned_vars`.

**Three of the ten are worth naming, because a binding fails SILENTLY where an arm falls to a
catch-all that at least exists:**

| site | what a wrapper costs it |
|---|---|
| `state::codegen::add_const` | `if let Value::Int(nr) = p { self.code_add(…) }` per width, with no `else`. A spanned constant emits **nothing** — the same silent-not-lossy shape as the `build_const_vectors` pair, which loft#1090 had to close by removing the `_` arm entirely |
| `generation::pre_eval::create_stack_var` | decides on `if let Value::Call(d, args) = v` and `let Value::Block(bl) = v else { return None }`; a wrapper takes the `None`, so the `&mut var_…` a by-ref argument needs is never emitted |
| `scopes::check_args` | `if let Value::Insert(ops) = a` guarding the A5.6 corruption panic — and it is `#[cfg(debug_assertions)]`-gated, so it is the second entry that cannot be measured by instrumenting an ordinary build |

**Two of the three measured, 2026-08-26 — one clean, one reachable-and-latent:**

| site | arrivals | spanned | answer changes |
|---|---:|---:|---:|
| `codegen::add_const` (863 programs, interp) | 907 453 | **0** | — clean, left alone |
| `pre_eval::create_stack_var` (54 native programs, 52 reach it) | 95 556 | **4 022** | **0** |

`add_const` is the control this time: the probe fired 907 453 times, so the zero is a
measurement rather than a vacuum, and no `Span` has ever reached it. `create_stack_var` is the
`find_first_ref_vars` verdict again — **reachable, latent**. The A/B ran the function's own
body twice per call, bare and peeled, and the two answers never once differed, which is a
stronger check than diffing emitted code because the return value is the function's only
effect. The peel is in anyway: it is one line, it obeys the documented rule, and 4 022 arrivals
is what makes it a trap rather than a hypothetical — a wrapper there does not pick a different
arm, it answers `None`, and the `&mut var_…` is then simply not emitted.

Written as a BINDING (`let v = v.unspan();`), not `match v.unspan()`, per the rule B4f arrived
at: peeling only the scrutinee while something else still walks the original is what
double-counted the sandbox bound.

**The two native emitters, measured (2026-08-26).** Both decide how code is EMITTED, so a
shape they cannot see is a shape they generate the wrong thing for:

| site | programs reaching it | arrivals | spanned | decision changes |
|---|---:|---:|---:|---:|
| `generation::emit::output_if_inner` | 863 | 110 157 | **4 193** | **1** |
| `generation::ops::key_ops::emit` | 18 of 140 | 41 | **0** | — |

`key_ops::emit` is clean: neither the `Keys` slot nor the `from`/`till` counts ever arrive
wrapped. The count is low, so it is reported with its reach — 18 programs — rather than as a
bare zero.

`output_if_inner` is the more interesting result **because the two decisions inside it answer
differently**. Its `wrap_block` test (`Value::Insert` with ≥2 ops — the statement-lift that
keeps `( stmt; expr )` out of the generated Rust) never once differed in 110 157 calls. Its
`b_true` / `b_false` test (`Value::Block`) differed **once**, in `808-tuple-return-value-abi`,
and that pair feeds `text_unify` / `text_string_unify` — how a text branch is wrapped so the
two arms unify in Rust. A spanned block reads as "not a block", which is simply the wrong
answer to the question being asked.

⚠ **One differing decision in a program that PASSES is not a bug found, and treating it as one
is the failure mode this section keeps repeating.** The peel was applied and
`808-tuple-return-value-abi` still passes, so nothing observable changed — the finding is that
the predicate now answers the question it asks. Both tests are peeled, not just the one that
differed: `peels_span` is body-wide, so peeling only `b_true` would have moved the whole
function into the "handled" column while `test` stayed blind. That is the audit's own
under-reporting hazard, and it is worth stating plainly — **a function that peels in one place
and matches unpeeled in another reads as handled, so 18 is a lower bound.**

**`walk` peels a `Span`; `map_nodes` does not — and both closures are safe, for different
reasons.** Five sites were flagged only because their match sits inside a `map_nodes` closure,
which the stripper did not know. `map_nodes`'s own doc states the contract: *"Unlike the
read-side walkers, `f` SEES `Span` nodes (it may want to replace them); descent still enters
the wrapped value."* So a closure whose `if let` misses the wrapper is handed the payload one
level down, exactly as with `for_each_child`. Adding it to the traversal list takes the
population **322 → 317** and the backlog **18 → 16**.

Worth telling the two apart when reading a closure rather than filing it as a stripper detail:
`walk` returns early on a `Span` so `f` never sees one, `map_nodes` calls `f` on the wrapper
AND on the payload. Moving a closure from one to the other starts feeding it `Span` nodes, and
only the descent makes that harmless — the same double-visit shape the `construct_prescan`
entry above is watched for.

⚠ **The widened matcher over-reports in the same way the narrow one did**, and the report says
so rather than implying otherwise. Four of the sixteen are dismissible by READING, which is the
documented method — the tool is not asked to judge them:

| site | why it is not a traversal |
|---|---|
| `parser::objects::parse_object` | builds IR from tokens; its `Value::` mentions are constructions |
| `ownership_cfg::op_label` | formats a CFG block's DEBUG LABEL; a `Span` yields `"op"` and nothing decides on it |
| `scopes::def_reshape_refusals` | `matches!(def.code, Value::Null)` as an empty-body guard |
| `scopes::construct_rewrite_ops` | its catch-all descends via `for_each_child_mut` into a self-recursive call, so a wrapper is entered rather than dropped — the B6 cure, already applied |
| `parser::expressions::ir_has_user_call` | same, since B4i — it TRADED its explicit `Value::Span` arm for a descending catch-all, which handles the wrapper it named and every one it did not.  The audit reads the loss of the arm and not the gain, which is why the count went 16 → 17 while the site got stronger |

Two more (`scopes::walk_check`, `scopes::check_args`) are `#[cfg(debug_assertions)]`-gated and
cannot be measured by instrumenting an ordinary build at all.

#### B4h — the queue is CLOSED: every site measured, gated, or read (2026-08-26)

All **16** are now accounted for, and the honest headline is that the sweep found **one**
latent decision change (`output_if_inner`, above) and no defect. The measurements, corpus-wide
unless noted:

| site | programs reaching it | arrivals | spanned | answer changes |
|---|---:|---:|---:|---:|
| `codegen::add_const` | 863 | 907 453 | 0 | — |
| `parser::control::is_block_divergent` | 150 | 17 396 | 0 | — |
| `scopes::is_ref_materialisation` | — | 2 329 | 0 | — |
| `parser::control::parse_match`'s null-arm test | 54 | 548 | **3** | **0** |
| `parser::mod::add_defaults` — `&substituted` | 70 | 614 | 0 | — |
| `parser::vectors::build_comprehension_code` | 28 | 84 | 0 | — |
| `parser::vectors::try_const_unroll_comprehension` | 16 | 41 | 0 | — |
| `generation::ops::key_ops::emit` | 18 | 41 | 0 | — |
| `parser::mod::add_defaults` — `&default` | **2** | **10** | 0 | — |
| `sandbox::intrinsic_space`'s scan | — | 188 | 6 | 0 (B4f) |
| `generation::pre_eval::is_void_value` | 45 | 22 | 0 | — (B4f) |

⚠ **`add_defaults`' `&default` position is reported with its reach because 10 arrivals in 2
programs is not coverage.** It is the true corpus count, not a sample — the whole 863 were
run — so the right reading is a suite gap, like `Parallel` being reached 4 times in 854
programs (B2). A zero there is much weaker than the zero beside it at 614.

**`parse_match`'s null-arm test is the third of the three predicates B5 said must not be
folded, and the only one that had never been measured.** 3 spanned arrivals, 0 answer changes —
the same result `arm_body_is_null` gave over 4 323, and for the same structural reason: a
spanned arm is never a null arm. Two independent copies now agree, which is a better argument
for leaving them separate than the original reasoning was.

⚠ **`build_comprehension_code` was listed above as dismissible by reading, and that was a claim
rather than a measurement.** It matches on `for_next`, which is IR its own caller assembled and
which can therefore carry spanned sub-expressions. Measured instead: 84 arrivals over 28
programs, 0 spanned. The verdict held; the reason it held is now evidence.

**Why this was found at all.** `reach` needed a "does this site peel `Span`" predicate, so the
one in `unspan` was extracted and shared — and reading it beside a walker that peels via
`base.unspan()` without ever naming `Value::Span` is what raised the question of what else the
pattern could not see. Both modes now call one `peels_span`, so they cannot answer it
differently.

#### B4i — a compound assign ran its container call TWICE (2026-08-26)

Working the catch-all walker queue (B6b) the same way, `parser::expressions::ir_has_user_call`
is the one that was wrong. It is a subtree predicate — *does this IR contain a user call?* —
with arms for `Span`, `Call` and `CallRef` and `_ => false` for everything else. Both its
callers use it as a RE-EVALUATION guard, so answering false wrongly duplicates a side effect:

| caller | what a false answer costs |
|---|---|
| `parse_assign`'s @PLN102 F2 hoist | the compound-assign place is not bound to a `_place` temp, so the accessor — and its call — is evaluated once for the read and again for the write |
| `control::record_text_payload_view` | a subject carrying a user call is recorded as a re-evaluable view, and its own doc says such a subject "would run that call again" |

**Measured, then constructed.** Over the corpus the bare predicate and a descending one agree
210 times in 32 programs — and only leaves and `Call` ever arrive, so the other arms look
unreachable. That is a statement about the corpus, not about the language, so the shape was
BUILT instead:

```loft
fn getv(c: Ctr, src: vector<integer>) -> vector<integer> { c.n = c.n + 1; src }
getv(k, cw)[0] += 5;          // one call in the source
```

`getv` ran **twice**, on both backends. The accessor lifts to
`Call(OpGet…, [Block("inline_container", [Set(tmp, getv(…)), Var(tmp)]), Int(8), Int(0)])`, and
`_ => false` never enters that `Block`.

**The rule had already decided it.** `formal/operational.md` `(E-Asgn-Compound)`: the
addressing sub-expressions — *"an index, **or a call that produces the container/struct being
indexed**"* — evaluate **exactly once**, and *"this holds for every place a compound assignment
can target."* So this is a deviation, not a design question, and the fix direction was not
open. Fixed by descending via the keystone, the same cure as B6's four.

⚠ **The guard tested one half of a two-half rule.** `pln102-f2-place-once.loft` varied the
INDEX call — `w[nxt(c)] += 5`, every operator, nested indices, const and var controls — and
never once varied the CONTAINER call, though the rule names both in the same sentence. The
corpus cannot find what it never varies. Three cases added (container call, nested container
call, and plain `=` as the contrast the rule itself draws); verified to FAIL against pristine
`origin/main` on both backends at exactly the new assertion.

Only the call COUNT is asserted for those cases: `getv` hands back a copy, so the write lands
in the temporary. Whether it should land at all is the lost-temp-write question (loft#894), a
different rule — and asserting the element would have pinned an answer this test has no opinion
about.

#### B4j — the same hoist had a second arm, and it aliased the wrong struct (2026-08-26)

The probe that found B4i also failed to COMPILE on `--native` — `error[E0425]: cannot find
value var___place_1` — for `pick(c, ps).x += 5`, where `pick` RETURNS a struct. Pre-existing on
pristine `origin/main` and independent of B4i: the hoist already fired here, and this is its
own emission.

**Completing the matrix is what identified the culprit, and it was not the cell that looked
broken.** The interpreter ran the same program "correctly" — until the neighbouring field was
tried:

| case | source after | |
|---|---|---|
| `pick(c,qs).x += 5` (offset 0) | **15** | the write reached `qs[0]` |
| `pick(c,qs).y += 5` (offset 8) | 99 | untouched |
| `cp = pick(c,qs); cp.x += 5` | 10, copy 15 | untouched |
| `cp = pick(c,qs); cp.y += 5` | 99, copy 104 | untouched |
| `pick(c,qs).x = 77` (plain `=`, no hoist) | 10 | untouched |

Two fields of one struct, one expression shape, opposite answers, no diagnostic. Three
independent control cells agree a returned struct is a COPY — binding first leaves the source
alone at BOTH offsets, and so does a plain `=`. So the outlier is the **`.x` write landing**,
not the `.y` write missing, and my first reading had it backwards. The reference route is the
oracle: score the broken cell against the working one rather than against a guess.

**Cause: the hoist had TWO arms for one question.** An offset-0 arm bound the accessor straight
into a scalar `RefVar` on the reasoning that *"the accessor IS a `&element` ref"* — true for
`w[idx()]`, false for a call that returns a struct, where it aliased the callee's SOURCE
instead of the returned copy. The other arm hoists the element reference and rebuilds the field
access, and it answers **both** offsets correctly.

**Fixed by deleting the offset-0 arm.** One shape for every offset; the surviving one is the
one measured right. Both symptoms go with it — the interpreter's silent wrong write and the
native E0425 — and the two backends now agree cell for cell on all five rows. This is the
thread's own subject arriving in the code it was reading: two implementations of one question,
where the second was not a refinement but a wrong special case.

Guard: four cases in `pln102-f2-place-once.loft`, including the bound-copy reference route and
the plain-`=` contrast, verified to fail against pristine `origin/main` — `x=15` on the
interpreter and E0425 on native.

#### B5 — `match` lowers through four paths and three mishandled a `null` arm (2026-08-25)

`match n { 0 => { null }, _ => { [n] } }` answered **null for every `n`**, including the arm
that builds a vector. Wrong value, no diagnostic, on `--interpret`; on `--native` the same
program failed to COMPILE (E0308). Reproduced on pristine `origin/main`, so pre-existing.

loft#936 established the rule — a branch-merge slot carries the result type's typed null
sentinel, never a bare `Value::Null`, which pushes nothing where the merge reads a 12-byte
`DbRef`. The repair exists in four lowerings, and only one of them was right:

| lowering | state |
|---|---|
| enum / struct match | correct — tests bare AND block form |
| `parse_if` (loft#936 itself) | correct |
| **scalar chain** | **FIXED** — tested a bare `Value::Null` only, so `{ null }` walked past it |
| **vector + tuple chains** | **OPEN** — different cause, see below |

**Fixed:** `build_scalar_chain`'s predicate now recognises a block whose last operator is
null, via one shared `arm_body_is_null` / `set_arm_null_typed` pair rather than a fourth copy.
Guard: `tests/scripts/a-block-bodied-null-match-arm-delivers-the-sentinel.loft`, verified to
fail (2 of 4 cells) against the unfixed build, green on both backends.

**OPEN, with the requirement now specified.** The vector and tuple chains promote
`result_type` out of `Void` but not out of `Null`, so a first `null` arm pins the chain to
`Null` and every later arm's type is ignored. Adding `|| result_type == Type::Null` fixes the
wrong value — and simultaneously changes what the NULL arm delivers, from the sentinel to an
allocated empty vector. Measured both ways; the arm repair is load-bearing for the tuple chain
and redundant for the vector one once the promotion is in.

⚠ **This entry twice recorded a wrong reading of that second half. Both are kept because the
sequence is the point.**

*First:* "an unsettled design call" — wrong, because I grepped `formal/` for "collection null"
and stopped. `(H-RefNull)` in [heap.md](formal/heap.md) does speak to it: *"nullref is the
reference null — the per-type SENTINEL (E-Null) of a reference type … a real value (a
reference that points at nothing), not a separate error state."*

*Second:* "so the empty vector is a rule violation" — also wrong, and reading the CODE is what
settled it. `parser::materialize_null_slice_arms` (@PLN85) deliberately rewrites a null arm of
a slice-match tail to a fresh empty vector, *"so a `[]` arm bound to a local … is a real
`DbRef`, not a bare `null` native emits as `()`"*, and its `arm_is_null` already recognises
BOTH the bare form and `{ OpNullRefSentinel() }`. So the empty vector is intentional on that
path, and today's `isnull=true` is the BUG bypassing the materialisation — not the rule being
honoured.

**What is actually open**, then: whether `(H-RefNull)`'s sentinel and @PLN85's materialisation
are reconcilable or whether one of them wants scoping to say where it applies. That is an
owner call with two documented mechanisms behind it, not something to settle by measurement —
which is why the promotion is not shipped here.

⚠ **BEING ANSWERED ELSEWHERE — do not re-derive this (2026-08-26).** The sibling checkout's
`tuxedo-work-2026-08-25` branch is working exactly this reconciliation, off the same
`origin/main`: **loft#1096** (a callee freeing the CALLER's buffer on a null collection return
— a collection return is now excluded from `free_vars`' loft#688 leg), **loft#1097** (a
collection tail join with a null arm answering `f(-1) == null` false while `len(f(-1))`
answered 2 — fixed with the per-arm `materialize_vector_arms_into` delivery), and in progress
**loft#1098** (a null-arm tail with TWO OR MORE value arms leaks one store per call, because
only one arm can BE the buffer and the rest were never delivered).

Their #1098 note settles the scoping question this entry left open, in one sentence: the
direct-`null`-arm exclusion *"was written for the return TYPE, not the delivery."* So the two
mechanisms ARE reconcilable and the scope is delivery-versus-type — not an either/or between
`(H-RefNull)` and @PLN85. **Nothing is cherry-picked here**: those commits carry their own
guards and `Fixes` trailers and land on their own branch, and there is no `src/` file overlap
with this one to force the issue.

⚠ The matrix that located this needed DISTINCTIVE values: a first attempt used `_ => { [] }`
as the wildcard, which made "the null arm answered empty" and "it fell through to the
wildcard" the same observation. `[99, 99]` separated them.

**Three null-arm predicates, and folding them would be WRONG.** The obvious follow-up to B5 is
to notice that `arm_body_is_null`, the enum path's inline test, and `Parser::arm_is_null` all
answer "is this arm a null?" and to consolidate. They do not answer the same question:
`arm_is_null` also counts `OpNullRefSentinel`, the shape an arm has AFTER the repair, because
its caller (@PLN85's slice materialisation) runs later in the pipeline. A repair predicate that
learned the sentinel would re-repair its own output.

Measured rather than argued: over the 858-program corpus the strong and weak predicates
disagree **once**, and that case is a nested-block tail `arm_is_null` recurses into. The
`arm_body_is_null` gap that looked worst — it does not peel its own top — takes **4323** spanned
values and changes the answer **0** times, because a spanned arm is never a null arm. Left as
it is, with the count in its doc comment, for the same reason `is_void_value` was left alone.

#### B6 — the par-safety classifier disagreed with itself (2026-08-26)

`parallel.rs`'s module doc names an undetected UB surface: *"the guarantee is CONDITIONAL
(@FR-C-Impure) … a worker that touches shared mutable state has no defined behaviour — loft
pins no interleaving — and nothing here detects that. See `scopes::is_par_safe`, which
classifies it but is **wired to nothing**."*

Wired to nothing understates it. `is_par_safe` returns a bool and `par_unsafe_reason` returns
the REASON for it, and over the corpus's par workers they **disagreed 16 times in 42** — every
one of them `is_par_safe=false` against `reason=none`. A verdict of "unsafe" with no reason to
give for it.

**Cause, and it is the keystone lesson again.** `par_unsafe_reason` was hand-rolled with named
arms and a `_ => None` fallback, and its arms do not include `Return` — so `fn dbl(x) { return
x * 2; }` was reported par-SAFE without its body ever being looked at. `is_par_safe` walks with
`any_node`, which visits every node, and saw the call. One walker exhaustive, one with a
catch-all, and the catch-all silently answered for a subtree it never entered.

**Fixed:** the fallback descends via `for_each_child`, so a wrapper nobody thought of cannot
reintroduce the gap. The two now agree on **136 of 136** corpus verdicts. Guard:
`scopes::par_safety_tests::the_two_par_classifiers_agree_through_a_return`, verified to fail
against the old walker with exactly that message.

**What still blocks WIRING it, named, sized and TRIAGED.** With the two in agreement the
reasons are actionable and all of one kind: **15 distinct unannotated native ops**, led by
`OpMulInt` (38 hits), `OpAddInt` (11), `OpEqInt` (8), `OpDatabase` (6), `OpLtInt` (3). Only 32
of 136 verdicts come back clean.

The annotation system is not missing — the stdlib carries **230** of them (132 `#pure`, 39
`#impure(host_io)`, 36 `#impure(io)`, 15 `#impure(parent_write)`, 8 `#impure(par_call)`). What
it never reached is the PRIMITIVE operator layer: the arithmetic block in `01_code.loft` runs
about sixty lines with one `#pure` in it.

⚠ **But "annotate the 15" is the wrong instruction, and reading their bodies is what shows it.**
They are three different cases:

| op | body | verdict |
|---|---|---|
| `OpAddInt`, `OpMulInt`, `OpEqInt`, `OpLtInt`, `OpAddFloat`, `OpMulFloat` | integer/float arithmetic, no state | genuinely pure — annotating unblocks |
| **`OpSetInt`** | `stores.store_mut(&db).set…` — a store WRITE | the classifier is **RIGHT** to reject it; this is the shared-mutable-state case `par` has no defined behaviour for |
| `OpGetInt` (a store READ), `OpDivFloat` (raises, so it logs), the `OpFormat*` family (append to a work buffer) | | a judgement, per op |

So a third of the "blockers" are the analysis working. Annotating changes what `par` would
ACCEPT once wired, and one of these ops is precisely the hazard the whole classifier exists to
catch — an owner call, and not a cleanup.

**The same shape a third time, found by screening for it.** `par_unsafe_reason`'s defect —
a self-recursive walker with a `_ =>` catch-all that names SOME value-wrapping variants and not
others — is mechanical to search for. Screening `src/` for it: 86 candidates, cut to **46**
after excluding bodies that descend via the keystone (19) or peel `Span` themselves (98
skipped for that reason across the whole set). Ranked by how many wrappers each omits, the top
hit was `data::collect_callees` — the CALL-GRAPH collector, missing `Return`, `Drop` and `Span`.

So `fn f() { return g(); }` recorded no edge from `f` to `g`. Its own comment said a missed
variant *"would surface as a `callers_of returns empty` regression"* because the phase-5e tests
covered it — and the only recursion test builds a `Value::If`, so nothing did. **Fixed**, with
`callers_of_finds_a_call_under_a_wrapper`, verified to fail against the old arm.

⚠ **`callers_of` has no user impact today** — it is called only from tests, exactly like
`is_par_safe`. A fourth instance turned up in the same screen: `walk_deep_parent_write` has an
IDENTICAL arm list to `walk_par_unsafe_reason_value` — the same walker written twice, missing
`Return` both times — so a worker whose body is `return helper(…)` was reported free of parent
writes without that call being examined. Fixed and guarded alongside its twin, because fixing
one of a near-copy pair and not the other is how they came to differ from `is_par_safe`.

#### B6a — the fourth one was NOT test-only, and it was the live bug (2026-08-26)

⚠ **I wrote "all four test-only, no user impact" and it was wrong about
`walk_deep_parent_write`.** `parse_parallel` calls it through
`worker_calls_parent_write_deep` (`parser/collections.rs`) and turns a `Some` into the C93
`Level::Error` that REFUSES the program. A subtree the walker does not enter is a refusal that
does not happen.

Measured as an A/B on one binary — the fixed arm against the `_ => None` it replaced:

| worker | pre-fix | fixed |
|---|---|---|
| `par(r = via_plain(e), 2)`, body `bump(e)` | clean compile error | clean compile error |
| `par(r = via_return(e), 2)`, body `return bump(e)` | **thread panic: `Write to read-only store at rec=3 fld=16 (locked by: borrow_locked_for_light_worker)`** | clean compile error |

So the promise on the first line of `pln102-c93-par-write.loft` — *"a clean COMPILE error …
no runtime errors, ever"* — was broken by writing the worker with a `return`, and the failure
landed in a worker thread. Guard: that file grew a `ret_outer → bump_ret` case, verified live
by making its `@EXPECT_ERROR` unmatchable and watching the suite fail on it.

**What made me misread it.** The function carried a bare `#[allow(dead_code)]` and no doc
comment at all, and I read the attribute as the author's statement about the function. It was
stale — removing it is clean under `--all-features`, default features and
`--no-default-features` alike. It now carries a doc comment that says it is production-wired
and why the fallback descends.

**The pattern that survives the correction** is the narrower one: Plan-06 phase 5b left a call
graph, a purity classifier, a par-safety verdict and a parent-write detector, **all wrong the
same way** — one unfinished subsystem rather than four bugs, and three of the four carried a
comment asserting they were covered. Three are test-only. The fourth is the one that was
shipping, which is the ordinary way round: the class was found in the dead code and the live
instance came free.

#### B6b — the screen can rank now, and the ranking says the opposite of what I expected

The filter that matters is *does production reach this?*, and a one-level "is it called from
outside a test" cannot answer it — `par_unsafe_reason` and `callers_of` are called by
ordinary-looking functions that are themselves only called from tests. Transitivity is the
whole question, so it is now a mode: **`ir_walker_audit.py reach`** walks the call graph from
each `[[bin]]`'s `main` and annotates every catch-all walker.

**The answer inverts the premise.** One level says **125 of 125 reached** — the useless check,
reproduced so the delta is visible. Transitively it is **124 of 125**, the single exception
being `ir_schema::value_from_parsed`, whose decode half has no production caller at all and
which omits nothing anyway. (Both totals were one higher until B4i fixed
`ir_has_user_call`, which left the list by descending rather than by being read.) So the remaining backlog **cannot be triaged as test-only**: the
four Plan-06 analyses were the anomaly, not the rule, and every other catch-all walker on that
list is code a loft binary runs.

It discriminates — 4 414 of the crate's 7 162 function NAMES reach `main`, so nearly two in
five do not — it simply does not discriminate *here*. Ranking is now by omitted **pass-through** wrappers
(`Span` `Set` `Return` `Drop` `Yield` `TuplePut`, derived from `for_each_child` rather than
listed): skipping `If` or `Call` is usually a decision about a shape the walker does not care
about, while a pass-through carries no information of its own, so skipping one is only ever
"the subtree was not entered and a verdict was issued anyway". All four Plan-06 defects were
exactly that.

**The third column is what made the queue workable, and it came from working it.** Ranking by
omitted pass-throughs still left ~125 equally-suspect rows. The property that separated the two
sites actually measured is whether the fallback ANSWERS NO — `_ => false`, `_ => None`. That is
where a missed wrapper costs something: the walker reports the absence of a property it never
looked for, and a caller guarding on it stops guarding. A fallback answering `true` fails safe
by comparison, and one returning a VALUE is usually a resolver over a narrower grammar
(`holder_type`, `of_const`). **55 of the 124** are that shape, and the report marks them `no!`.

**Worked so far — one bug-bearing site in six, and the contrast is the useful part:**

| site | what a false answer disables | arrivals | disagreements |
|---|---|---:|---:|
| `expressions::ir_has_user_call` | the compound-assign once-only hoist, and a re-evaluability check | 210 over 32 programs | **the corpus said 0 — see B4i** |
| `generation::calls::may_borrow_store` | the argument hoist that keeps two `&mut Stores` borrows apart (E0499) | **151 823** over 214 of 220 | **0** |
| `emit::text_arm_yields_owned_string` + `text_arm_ends_in_text_call` | the `.to_string()` unification that keeps a `String` arm and a `&str` arm compiling (E0308) | **102 073** over 253 of 260 | **0** |
| `scopes::accessor_root_var` | the C93 par refusal (B6b below) | 47 over 7 — useless; settled by construction | **0** |

⚠ **The text pair is the case where the shapes DO arrive and the answer still never moves.**
`If` and `Return` both reach their `_ => false` — 42 `Return`s in a single program — and a
hand-built nested-`if` text arm paired with a literal sibling compiles clean. The boundary is
subtler than "this shape cannot carry the property": the question is what the arm YIELDS to the
join, and a `Return` yields nothing to it. Worth recording because *"the omitted shape arrives"*
is the reading that looks most like a defect and is not one.

**Triaged by the boundary question alone, no probe needed:** `lit_nonzero` (`None` = not a
literal — and a literal is a leaf by definition, the one look-through being the widening cast
it already handles), `index_loop_bounded` (`false` = not provably bounded, which KEEPS the
warning — the safe direction), `tail_has_tuple_leaf` (its scrutinee is `.tail()`, which already
peels `Block`/`Insert`/`Span`).

**Third site: `scopes::accessor_root_var`, clean — and it yields the reading rule.** Its `None`
makes `raw_write_to_captured` bail, so the C93 par refusal never fires: the same soundness
surface as B6's `walk_deep_parent_write`. The corpus is useless here — **7 programs, 47
arrivals, 0 bails**, only `Var` and one `Call` ever arriving — so the shape was built:

```loft
fn getv(s: S) -> vector<integer> { s.v }
fn w(s: S) -> integer { getv(s)[0] = 99; s.v[0] }   // base is a LIFTED container
for e in rows par(r = w(e), 2) {}
```

The bail fires (`kind=Call`) **and the write does not reach parent state** — `rows` is
unchanged — while the control `s.v[0] = 99` is still refused with the C93 error. So there was
nothing to refuse.

**A site the corpus never reaches at all — recorded, not chased.**
`control::body_has_buffer_return` (and its nested `terminal_is_buf`, which handles `Var` /
`Block` / `Insert` and not `If`, while its own sibling `walk` two lines below DOES handle `If`)
is **entered 0 times across all 863 corpus programs**, including `104-split-text` — the
regression its doc comment names. Its `&&` chain short-circuits earlier: the site requires
`!tail_terminal_is_branch(&l[last])`, while the `vec_match_candidate` gate 8 000 lines above
requires that same predicate TRUE, so the two are complementary and only one fires.

Left alone deliberately. The return-buffer machinery is being actively worked in the sibling
checkout (loft#1096/#1097/#1098 — see B5), and their in-flight change edits that very gate.
Sending a second person into the same decision chain from the other side is how a near-copy
pair comes to differ, which is the defect this whole thread keeps finding. **The finding worth
handing over is the asymmetry**: `terminal_is_buf` cannot see an `If` that its sibling `walk`
can, and a false answer there RENAMES a buffer that is already taken — the double-own the
call site's own comment warns about.

#### B6d — the fallback the author never wrote down is the one that was wrong

Six sites in, the sharpest predictor is not which variants a walker omits. It is **whether its
doc comment says why the FALLBACK is right.**

| site | what its doc says about the fallback | verdict |
|---|---|---|
| `borrow_base_guarded` | *"A revisited var yields `None`: a cyclic chain has no single borrow base, and every caller handles `None` conservatively."* | clean |
| `view_root_slots` | enumerates every exclusion with a reason, and states the direction — *"an extra marked store can only REFUSE a free, never license one"* | clean |
| `may_borrow_store` | *"A USER function is not a conflict — it is called with `cell`, not with a live `&mut Stores` — so only `#rust` templates count."* | clean |
| `worker_returns_capturing_closure` | *"Conservative — only flags closures found directly in return position … an indirect return … falls through to the runtime path rather than a false positive."* | clean |
| `text_arm_ends_in_text_call` | ⚠ only PARTLY — it explains an exclusion INSIDE its `Call` arm (`Op*` yields `&str`, which already unifies) and still says nothing about `_ => false`. Listed because it is the honest middle case: measured clean at 102 073 arrivals, but the doc is not what earned that | clean |
| **`ir_has_user_call`** | **nothing.** Its doc explains the QUESTION well — what a user call is, why a place reaching one must be bound once — and never mentions `_ => false` | **two shipped bugs** |

So the correlation is five of six, not six of six — `text_arm_ends_in_text_call` documents an
exclusion but not its fallback, and measured clean anyway. A heuristic, not a law.

`ir_has_user_call` was not undocumented. It had a careful doc comment about what the function
is FOR. What it lacked was a sentence about what happens to everything it does not name, and
that is exactly where the defect was.

**As a reading rule:** for a negative-fallback subtree predicate, ask what the author wrote
about the shapes it does not list. A stated reason is evidence someone considered them; silence
is not evidence of anything, and that is where to spend a probe. Deliberately NOT a detector —
the axis is semantic, and [DOC_QUALITY.md § B2](DOC_QUALITY.md)'s `incident` pattern is the
standing reminder that a lexical thermometer for a semantic property gets ignored.

**As a review rule, it is worth more than as a screen:** a walker of this shape should say why
its fallback is right, in the same block that says what it computes. That is one sentence at
write time against a measured two bugs.

⚠ **The rule that makes 52 remaining sites triage rather than grind: does the fallback encode a
SEMANTIC BOUNDARY, or is it just a shape nobody listed?** `accessor_root_var`'s does — a link
that is neither `Var` nor `OpGet` produced a NEW value, so identity into parent state is lost
there and no write through it can reach the caller. `may_borrow_store`'s arms already name
every wrapper that can carry a call. `ir_has_user_call`'s did **not**: a call is a call wherever
it sits, so `Block` hid one with no boundary to justify it — which is why that one was the bug
and these two are not. Ask the boundary question first; measure only where the answer is no.

⚠ **Those two zeroes are not the same zero, and reading them as equal would have missed the
bug.** `ir_has_user_call`'s 210 arrivals are thin, and only leaves and `Call` ever reached it —
a statement about the corpus, not the language. Building the missing shape by hand found a real
defect (B4i). `may_borrow_store`'s 151 823 arrivals across 97 % of the sample, plus a
hand-built tuple-key probe that also failed to reach it, is a zero worth believing. **The
strength of a zero is the reach behind it**, which is why the report prints both.

⚠ **Both of its false-negative classes were found by hand-checking hits, and both would have
manufactured findings.** The verdict this mode exists to give is *not reached*, so an edge it
cannot see becomes a fabricated one:

| miss | what it cost | seen because |
|---|---|---|
| a **nested `fn`** — the body splitter ended the outer function at the inner `fn` line, dropping the outer's only call to it | 10 unreached → 2 | `parser::operators`'s two `try_swap`s both sit inside their caller |
| a function passed as a **VALUE** (`.and_then(Self::arg_root_var)`), which has no `(` after the name | 2 unreached → 1 | `arg_root_var` has no other use |

And one false POSITIVE nearly shipped in the fix for the second: matching any bare identifier
picked the name out of PROSE — `parallel.rs`'s module doc says *"see `scopes::is_par_safe`,
which is wired to nothing"*, and counting that sentence as a reference reports the function as
production-reached, inverting the one answer the mode exists to give. Comments and strings are
stripped first, and the stripper's own order is load-bearing: `/*.loft` inside a line comment
in `main.rs`, read as a block-comment opener, ate 97 KB and dropped 1 219 functions out of the
reachable set at a stroke.

**Validated against the answers already found by hand** — `is_par_safe`, `par_unsafe_reason`,
`callers_of`, `collect_callees`, `walk_par_unsafe_reason_value` not reached;
`walk_deep_parent_write`, `collect_parallel_violations`, `execute`, `compile`, `const_eval`,
`find_assigned_vars` reached — **11 of 11**, which is what each candidate matcher was scored
on. The graph is keyed by NAME, so same-named functions merge and every merge can only ADD
reachability: *not reached* is the trustworthy verdict and *reached* is the weak one. The
library's public API as called by another crate is not a root — the question is what a loft
binary runs.

Read one at a time, the highest-value site was worth doing:
`collect_parallel_violations` is genuinely production-wired (reachable from `parse_parallel`,
no `#[cfg(test)]` above it), it guards a SOUNDNESS floor — rejecting unsound `parallel {}`
captures — and the screen flagged it `MISSING[Span]`. Measured: **113 calls, 41 fallthroughs**
(all leaves: `Int` 30, `Text` 4, `Boolean` 4, `Null`, `Line`, `Break`) and **0** spanned. A
false positive; that collector already names every wrapper including `Return`/`Drop`/`Yield`.

⚠ The first run of that probe reported zero because it selected programs matching `par(…)`
while the guard is for `parallel { … }` BLOCKS — a different construct. Vacuous, and the second
such miss this session. Selecting the corpus subset is part of the measurement, not setup.

⚠ **The screen over-reports, and `escapes_value` is the shape it cannot judge.** It looked like
a fifth hit; its caller `guard_escapes` handles `Return(v)` and passes the already-unwrapped
payload, so the helper never sees a wrapper. A predicate that takes the PAYLOAD rather than the
node is indistinguishable, in text, from one that takes the node and forgot a case — the same
over-reporting the `unspan` audit had, and the reason each hit is read before it is believed.

#### B6e — the queue's own predicate was the bug: a projection that does not look like one (2026-08-26)

`Value::TupleGet` is a projection — `t.0` reads a container out of a local exactly as
`OpGetField(Var(b), …)` does. It is a `Value` VARIANT carrying its base as a var NUMBER, so
**no call-shaped pattern can see it**, and the two gates that decide whether a returned
projection must be COPIED into the caller's buffer are both call-shaped:
`return_field_base_var` (the #425 rung that suppresses the NRVO rename) and
`return_projects_into_local` (the H12 predicate the vector return arm asks). Both are on the
ranked `no!` queue.

So a function whose tail projects a vector out of a tuple local renamed **the tuple** onto a
vector-shaped `__retbuf`: the prologue cleared a stack tuple slot as a vector, the tail became a
discarded statement, and the function returned null. The interpreter answered the right elements
off the eval stack and then panicked on a corrupt reference at exit; `--native` refused to compile
the program at all (parameter typed `DbRef`, assigned a `(DbRef, i64)`).

```loft
fn make() -> vector<integer> { v = [11, 22, 33]; t = (v, 7); t.0 }
```

Fixed by giving both predicates the tuple spelling (plus the chained `t.0.items` base). The
lowering is now the canonical one the working siblings already produced — clear the caller's
buffer, append into it, free the local store, return the buffer — on both backends. Guard:
`tests/scripts/return-a-tuple-element-that-is-a-vector.loft`, falsified against a pre-fix binary.

**This is the FOURTH sibling of one defect, and the third one's comment says so.** `#425` fixed
the struct-field form, `H12` the vector-element form, `#488` the projection rooted at an inline
call's temporary — *"the third sibling of the same defect, after the struct (#425) and element
(H12) forms"*, in the code, two lines above the predicate that could not see a tuple. The class is
the mechanism, and the mechanism here is not "a walker forgot a variant": it is that **one
language-level notion — a projection — has two IR spellings, and only one of them is a `Call`.**
Anything matching the call spelling silently excludes the other.

⚠ **The churn is what made the measurement real.** The first probe of every cell printed the right
values, so the boundary looked like "only the implicit tail is broken". Adding a loop that
recycles freed records (the `941` idiom) moved the failure to two more cells that had "passed" by
reading their own freed bytes — including the explicit `return t.0;` I had recorded as CLEAN and
was about to write into the test as a control. A dangling read is not an absent one.

⚠ **And it split the finding in two.** With the churn in place, BINDING the projection first
(`e = t.0; e`) still hands back a view — and so does the vector-element spelling of it
(`e = vv[0]; e`, which answers an EMPTY vector, both backends). That shape is not tuple-specific:
the gates read the returned EXPRESSION, and a binding puts the fact in the DEPS instead, where
only the struct path has a leg for it (`return_views_local`, the #306 case). Filed as **loft#1101**
(`silent-wrong`, `wa:clean` — copy out with `o = []; o += t.0; o`, verified on both backends),
not folded into this fix: it is the same question one level up, in machinery the sibling checkout
is actively rewriting.

**How the site was reached, because it is the queue's first hit from a non-fallback route.** Not
by reading the ranked list top-down. The var-identity class — `TupleGet` / `TuplePut` / `FnRef` /
`FnRefDnr` / `CallRef` / `Set` / `Iter` carry a var NUMBER outside a `Value::Var` node — came from
reading `holder_retained` and `escapes_value`, the two Plan-57 soundness gates. Measured over the
896-program corpus: **651 113 arrivals at those two fallbacks, every program reaching them**, the
carrier shapes arriving 6 943 times, and the carried var naming the target exactly **twice** —
both a `Set(target, …)`, which is a WRITE to the target rather than a hand-out, so both are
correct. Those two gates are clean, with the reach to make that a believable zero. The class,
followed one step further into the return path, is where the defect was.

#### B6f — five null-tail walkers, three questions, and the axis none of them documents (2026-08-26)

Working the ranked queue turns up a FAMILY rather than a site: five walkers answering
*"does this yield null at its tail?"*, none deriving from another.

| walker | null spelling it counts | `If` | `Return`/`Drop` |
|---|---|---|---|
| `control::branch_yields_null` | `OpConv*FromNull` | descends, either arm | no |
| `control::arm_yields_direct_null` | `OpConv*FromNull` | **no** (documented) | no |
| `control::arm_is_null` | `OpNullRefSentinel` | **no** (documented) | no |
| `scopes::is_null_terminal` | `OpNullRefSentinel` | **no** (documented) | **yes** |
| `scopes::return_has_null_arm` | `OpNullRefSentinel` | descends, either arm | **yes** |

Two of the three axes are deliberate and written down: the `If` axis separates "an arm that IS
null" from "a branch that CONTAINS one" (descending would catch a `match`'s synthesised
unreachable default and falsely widen), and the spelling axis follows the type — DN1 only fires
for non-null SCALARS, whose bare `null` lowers to `OpConv<T>FromNull`, while the reference
sentinel is a heap value's. **The `Return`/`Drop` axis is documented nowhere**, in any of the
five: the two `scopes` walkers pass through those wrappers, the three `control` ones do not.

Both groups are right, for a reason neither states: `control`'s are asked about a branch's value,
where a `return` yields nothing to the join, and `scopes`' are asked about a return EXPRESSION,
where the wrapper is the thing being examined. That is the B6d rule paying out in the split
direction — silence about the unlisted shapes is where to spend a probe, and here the probe says
the code is right and the docs owe a sentence.

**The one cell that looked like a hole was measured, not argued.** `is_non_null_scalar` includes
`Type::Text`, and text is DbRef-backed, so a text branch's `null` looked like it should lower to
the sentinel the DN1 pair cannot see — a silent null into a non-null `text`. It does not:
`fn pick(b: boolean) -> text { if b { "hi" } else { null } }` warns exactly as the `integer`
version does, because a text null lowers to `OpConvTextFromNull` like every other scalar.

#### B6g — the class B6e named, screened: 18 functions resolve a projection by OP NAME, two see the other spelling (2026-08-26)

B6e's finding was not "a walker forgot a variant" but **one language-level notion with two IR
spellings, only one of which is a `Call`** — a projection is `OpGetField(Var(b), …)` *or*
`Value::TupleGet(b, i)`, and no call-shaped pattern can see the second. That is a mechanical
question, so it was asked mechanically: every site that names a projection op by def-number,
against whether its enclosing function handles `TupleGet` at all.

**18 functions across six files resolve a projection by op name — by `def_nr("OpGet…")` or through
`is_projection_op`. Two of them handle the tuple spelling: `return_field_base_var` and
`return_projects_into_local`, the two B6e had just fixed.** The screen reproduces the answers
already found by hand, which is what makes the other sixteen worth reading.

| functions resolving a projection by OP NAME | ALSO handling `TupleGet` | seeing only the call spelling |
|---:|---:|---:|
| 46 | **12** | 34 |

Re-measured on the tree that holds both streams: **45 · 12 · 33**, and **46 · 12 · 34** once
loft#1396's `value_view_container` joins it — another function resolving a projection by op
name.  Neither side predicted the joined number and neither tried to: a row can only be true on
the tree it is measured on.  The `@FR-O-Owner` walk
folded two byte-identical container-namer loops into one home and the loft#1384 place walk
joined it there, so neither branch's row survives the join — the audit classifies FUNCTIONS,
and a merged body is one function however many branches touched it.

(`./scripts/ir_walker_audit.py spellings`, gated by `doc_hygiene::quality_spellings_table_matches_the_audit`
so the row cannot go stale — the same arrangement the `unspan` table has.)

loft#1186 moved it to 41 · 8 with `parser::node_place_root`, the arm-level half of the join
reading: it resolves a projection by op name AND carries the `TupleGet` spelling, so it lands
on the handling side and leaves the third column where it was.
loft#1345 moved it to 44 · 10 the same way: the vector materialiser's new projection leaf
(`materialize_vector_arms_collect`) copies a projected arm into the return buffer, and it was
written for BOTH spellings from the start — the screen asked, before the row was updated,
whether a tuple element viewed through `q.0` could reach a `-> vector<T>?` return, and it can
(`tests/scripts/1345-…loft`'s tuple-element cell), so the leaf handles `TupleGet` and the
third column stays where it was.

loft#1361 moved it to 44 · 12: `classify_vec_bind` and `parse_assign_op_inner` now read the
member of a TUPLE LOCAL through the `TupleGet` spelling — the whole-tuple bind and the
assignment off `t.i` lower onto the same owned copy a heap local gets, where before each
saw only the call spelling and let the member's handle through.

loft#1195 moved it to 42 · 8 · **34** with `parser::field_place`, which reads a comprehension
destination as a PLACE (root variable + `OpGetField` offsets) and does not carry the
`TupleGet` spelling. Asked the question this screen exists to ask — is the fallback a semantic
boundary or a shape nobody listed? — the answer is measured rather than argued: a tuple-element
destination (`t.0 = [for i in 0..t.0.len() { t.0[i] * 2 }]`) is CORRECT on both backends today,
so it never reaches this predicate and the site's blindness costs nothing. Recorded because
that is a fact about a neighbouring route, not a property of `field_place`: if the tuple
destination ever starts arriving here, this is the row that says the predicate cannot see it.

loft#1214 moved it to 43 · 8 · **35** with `parser::keyed_receiver_discharge`, which asks
whether an assignment place is a KEYED element read (`OpGetRecord`) before peeling a discharge
out of its receiver. Its fallback IS a semantic boundary, and this time the boundary is the
point of the predicate rather than an omission: the question is *"is the accessor a keyed
element write?"*, and a `TupleGet` is not one whatever it contains — a tuple element reached
through a keyed lookup still arrives as `OpGetRecord` with the `TupleGet` inside the SUBJECT,
which this predicate hands to `null_discharge_subject` rather than reading itself.

The measurement that says so also found a neighbouring route that is broken, which is the kind
of fact this screen exists to surface. A keyed collection held in a TUPLE ELEMENT is never
materialised: `t.0[k] = v` on a `(hash<E[k]>?, integer)` panics with a NULL DbRef, and did
before loft#1214 and on the shipped build. The `?` spelling answered length 0 in silence there
and now panics with its bare twin, which is the two spellings agreeing rather than a new
defect — but the place-kind itself has no materialisation, so it is filed apart. The predicate
above is not what is blind to it; `keyed_local_materialise` answers only for a keyed LOCAL, and
a tuple element is not a variable.

The second half of loft#1225 then moved the row the other way, to 43 · **9** · 34, without
adding a site: `parser::towards_set` gained a `TupleGet` arm and crossed from the blind column
into the handling one.  That is the screen reporting a fix rather than a hazard, and it is the
direction to expect — the neighbouring route the paragraph above recorded as broken was broken
BECAUSE that site could see only one spelling of its destination, so teaching it the other is
what closed the keyed half of loft#1225.

⚠ **The row reads 38 · 5 · 33 and the paragraph above it says 18 · 2 · 16, mostly because the
SCREEN was widened rather than because sites appeared.** It has moved four times in one merge —
the sibling checkout's commits add `lift_view_deps`, which resolves a projection by op name AND
carries a `TupleGet` arm, and this branch's two tuple helpers came back out again once that arm
proved to answer their cells. Re-run the tool after any merge rather than reading a number here. The matcher saw two of the three ways Rust resolves an op
here — `def_nr("OpGet…")` and a call to `is_projection_op` — and was blind to the third, a match
on `data.def(d).name()` against a string literal, which is how every hand-spelled list in the tree
is written. That is the B4g lesson arriving one mode later, and it is what B6i found by walking
into one of those lists from the other side. The 18 · 2 · 16 figures are what the narrower matcher
reported; re-run the tool rather than reading a number off this page.


Following it produced three defects, and the first is the one worth the section:

| what the screen pointed at | what was there |
|---|---|
| `classify_vec_bind` — `binding.md` names it *"where the parser asks"* the own-vs-borrow question — has a struct leg and no tuple leg | not a missing leg: **the tuple never OWNS its element**, so the base really is borrowed and the bind really is a view — **loft#1102** |
| the tuple-literal parse, reached while building the probes | a member adopted the assignment's destination as its build accumulator — **fixed** |
| `emit_tuple_put_ops` + two siblings | the five keyed collections were missing from a hand-spelled DbRef list — an ICE — **fixed** |

**loft#1102 — a tuple literal ALIASES a heap local; a struct literal and a vector literal copy
it.** ⚠ **Settled 2026-08-26, and the answer is a model that already ships:** construction may
alias, but only where aliasing cannot change the semantics a normal variable has — so the
contract is `B-Copy` and aliasing is admissible only as the LAST-USE elision. That is what the
STRUCT constructor does, default-on since @PLN90 phase B B1.5. Measured: with the source dead
after, `s = S { n: 1, v: vl }` builds the literal STRAIGHT INTO the field
(`OpNewRecord(OpGetField(s, 8, 21), …)`, no second store, no copy — `construct_fresh_rewrite`);
with `LOFT_NO_MOVE_ELIDE=1` the same function builds `__vdb_1` and copies; and with the source
still LIVE the copy happens. So the filing's cost objection is answered — the common case is a
move, not an allocation, and only the observable case pays. The tuple constructor is outside
BOTH halves: no copy, and therefore no elision either, because `ConstructOps` is record-shaped
(`op_get_field` / `op_new_record` / `op_finish_record`) and `Value::Tuple` is not a container
`move_elide` can see. `t = (vl, 9)` stores `vl`'s handle, so the element and the local are two names for one
store; `s = S { n: 1, v: vl }` and `vv = [vl]` both copy, both backends. Everything downstream
follows from it: `t.0` reads as a projection off a BORROWED base, so `c = t.0; c[0] = 41` writes
through two levels of binding — while `lost-write` warns on that exact line that *"a whole-value
bind COPIES the heap value (C86), so the mutation lands in the copy"*. The write is not lost; it
lands in `vl`.

⚠ **The measurement nearly went out as the wrong finding.** The first matrix scored `c = t.0`
against `B-Copy` and read five failures — including the three CALL-spelled controls. All five are
correct: `binding.md` states five clauses, and a struct-typed projection (`B-View`), an index read
and a nested field read (`B-View-Depth`) are all views by rule. What survived re-scoring against
the actual clause list was ONE cell — a one-level COLLECTION projection off an OWNED base — and
then even that turned out to be a symptom rather than the defect. This is the third time the
incomplete-rules-doc trap has been walked into from this thread; the cure that worked was reading
`binding.md`'s clause list before believing the matrix, not after.

**`formal/tuples.md` reads `OPEN: 0` and does not cover this**, which is the oracle caveat its own
deviation note already carries: its corpus is `(integer, integer)`-shaped, so neither `text`
(loft#1004/#1005) nor a collection element is inside what that zero measures. And
`tests/scripts/bind-copies-or-views-the-whole-boundary.loft` — the file whose header says *"ask it
rather than re-deriving"* — has two tuple cells, both **off a BORROWED base**. The owned-base
tuple axis is the one that file never varies, and it is the axis the defect is on.

##### The accumulator a tuple member should never have adopted

A heap-building RHS adopts the assignment's destination variable as its build accumulator (#501's
watermark reuse, `parse_append_vector`'s `orig_var`). A parenthesised expression IS that whole
value, so adopting is right there. A tuple MEMBER is not — and member 0 is parsed before the `,`
can say which of the two this is.

So `t = ([10, 20], 9)` typed `t` as the MEMBER and then refused the retype it had just caused:
*"Variable 't' cannot change type from vector<integer> to (vector<integer>, integer); use a new
variable name or cast with 'as'"* — a legal program refused, naming a cure that cannot work. The
declared spelling reported the same collision with the two types swapped. Where the member built
through the append path instead (`t = (x + y, 9)`), there was no diagnostic at all: `t.0[1]`
answered `null`.

Fixed by asking the lexer first — `peek_tuple_literal` — and giving a member its own temp. Value
position and argument position have no destination to adopt and were always correct; they are the
controls. The concat member now lands on the P103 refusal every other position already gave it, so
a silent null became an error that names its workaround. Guard:
`tests/scripts/a-tuple-member-is-not-the-assignment-destination.loft`, thirteen cells.

⚠ **The look-ahead itself took two goes, and both failures were the same fact about `revert`:
it restores the token STREAM and nothing else.** Written as `recover_to(&[","])` it walked to
end-of-input on an unclosed `(` — and because a replayed token deliberately leaves `position`
where the scan reached, the next diagnostic's caret moved from the offending line to the end of
the function (`error_messages::baselines_are_locked_in`, which is what that gate is for).
Restoring `position` by hand is worse, not better: during a replay it does not advance, so the
caret then lands short. The bound is the fix — stop at a depth-0 `;`, so the scan is
statement-local and the drift cannot leave the statement. Then the suite found the second one: a
string may carry an interpolation HOLE, and `in_format_expr` / `open_strings` / the backtick
dedent stack are not restored either, so crossing one and coming back left the lexer describing a
string it was no longer inside and the enclosing group never closed. Stop BEFORE a string and
answer "not a tuple" — a text member does not adopt the destination, so that answer costs
nothing. **A look-ahead over this lexer is safe only while it stays inside one statement and out
of a string**, which is now written on `peek_tuple_literal` where the next caller will read it.

##### The carve-out comment was the map, again

`data::is_dbref`'s doc says what happens when its list is spelled inline: *"the three obvious
kinds (`Reference` / `Vector` / struct-`Enum`) get written and the five keyed collections are
forgotten … A short list is not a compile error anywhere — it routes a handle down the scalar
path — so call this function rather than restating it."*

Three of the four tuple emitters spelled exactly that short list, so `hash<S[n]>` in a tuple
aborted the compiler with `emit_tuple_put_ops: unsupported elem Hash(710, ["n"], …)`. The fourth,
`emit_tuple_var_push_recursive`, names every kind — which is why such a tuple could be read back
but never written. **The family had already drifted once the other way**, and its own comment says
so: loft#808 put `character` and payload-less `enum` in three of the four and left null-init
short. Two drifts, opposite directions, one family — so the fix is the shared predicate, not a
fourth copy of the list. Guard:
`tests/scripts/a-tuple-element-of-every-dbref-kind.loft`, eight cells over four keyed kinds plus
destructuring.

⚠ `emit_tuple_var_push_recursive` keeps its own list rather than delegating, for a reason and with
one loose end. The reason: its `Reference`/struct-`Enum` arm registers a known type before the op,
which the collection arm does not, so the two are not one arm. The loose end: its list also carries
`Type::Iterator`, which `is_dbref` does not — and neither does `element_stack_size`, the layout
authority `is_dbref`'s own doc names. So the outlier is the push walker, not the predicate.  Left
as a NOTE and not a change: nothing here builds a tuple with an iterator element, and an arm
removed on that reasoning alone is a guess in the other direction.

⚠ The wider sweep is a queue, not a rewrite. A four-line-window scan of `src/` finds **80 places
across 24 files** where the three obvious kinds appear together with no keyed collection in sight —
an OVER-count by construction, since the window is lexical and several of them are not the
membership question at all. It sizes the backlog (dozens, not a handful) and ranks nothing. The
tuple family earned its fix by having a bug AND a prior drift in the same four functions; the same
evidence has to be found for each of the rest, one at a time.

##### The class has one home now, and the mode has an outside validation

Three instances of one shape in a week — the projection (B6e/B6g), the null at a join (loft#1103),
and a borrow with no dep at all (loft#1101, `e = mk().items`) — is a class, not three bugs, and
stating it once per issue is how it stays three bugs. It is written once in
[formal/IMPLEMENTATIONS.md](formal/IMPLEMENTATIONS.md) § *One notion, how many SPELLINGS?*, as the
dual of that document's own question, with what each instance cost; `engineering-rigor` and
`loft-codegen` carry the trigger that gets it read.

**The `spellings` mode's first use outside this thread found something.** The sibling checkout
ran it against `expr_borrows_local`, which it had just landed for loft#1101: the predicate
resolves by op name (`OpGetField` / `OpGetVector`) and is blind to `TupleGet` exactly as the
18-vs-2 count predicts. Five tuple spellings answer correctly today only because the DEPS leg
covers what the structural leg cannot see — latent, not live, and recorded on D-own-10. That is
the verdict the mode exists to give: a matcher that is right about every site it can see.

##### B6f's owed sentences are written

The `Return`/`Drop` axis none of the five null-tail walkers documented now says, in each of them,
why its answer is right: `parser::control`'s three are asked what a value hands to a JOIN, where a
`return` hands it nothing, and stop at the wrapper; `scopes`' two are asked about a return
EXPRESSION, where the wrapper is the subject, and pass through it. `is_null_terminal` had no doc
comment at all and now has one.

##### loft#1101 — landed in the sibling, and the shared-cause claim it falsified

Deferring it was right: the sibling checkout was inside that machinery and has since fixed it
(`56c6374e`, register entry **D-own-10**).

⚠ **What is worth carrying here is that my reading of the RELATIONSHIP was wrong, and one cell
killed it.** Having found the tuple half first, I offered #1102 as the common cause under #1101
— if a tuple owned its element there would be no freed view to hand back. The sibling answered
with a program that has no tuple in it at all:

```loft
fn f() -> vector<integer> { vv = [[11, 22, 33], [44, 55]]; e = vv[0]; e }   // len=0
```

Re-measured here on this branch: `len=0`. Same defect, same fix site, and the suspected cause
structurally absent — which is [DEBUG.md]'s rule for separating causes, applied to a CAUSE claim
rather than to a repro. A shared-cause story is a hypothesis like any other, and the falsifier is
the cell the suspected cause cannot be in. The tuple spelling was one of four, beside `e = vv[0]`,
`e = s.items` through the lift temp, and an `if`-arm binding.

**The real cause is the one this thread keeps meeting from the other side: a dep list read as an
ownership answer.** `fresh_owned_vector_deps` and the promotion ladder took non-empty deps to mean
"owns a backing store", and a view reads non-empty too. `formal/ownership.md` @FR-O-Proxy already
sharpens it into three meanings — empty; a dep on the binding's OWN mint (`__vdb_N`, owns one);
and a dep on ANOTHER LOCAL (borrows it) — and reading the third as the borrow it is leaves the
candidate on `Bind`, which copies into a separate `__retbuf`.

Two details from that fix generalise to anything reading deps in the parser, and both are worth
having before the next one:

* **Skipping the mint is not a refinement, it is what makes the verdict PASS-STABLE.**
  `vector_db` adds the mint dep on pass 2 only, while a borrow dep is present on both. This
  verdict decides whether the function takes a hidden buffer argument, so bare non-emptiness
  moves the ABI between passes — which is what loft#1099 cost.
* **One shape has no dep at all.** `e = mk().items` borrows a `__lift_N` whose container dep
  loft#882/#889 record at the SUBSCRIPT only, so that leg has to read the DEFINING STATEMENT
  instead.

**The two fixes compose rather than collide.** `t.0` projects into `t`'s store and `t` is a local
this function frees — true whether the element was aliased from `vl` or copied into `t` — so the
sibling's rung still fires on the tuple spelling after #1102 lands, and #1102 does not make it
redundant. Nothing of theirs touches `ConstructOps` / `move_elide` / `construct_fresh_rewrite`.

⚠ **Rebase note:** `src/parser/control.rs` has moved under this branch — `56c6374e` adds
`var_is_mint`, `expr_borrows_local`, `var_defining_expr`, `var_defined_by_projection`,
`var_views_local`, `tail_ret_view_local`, a rung in `classify_ret_promotion_inner`'s
`allow_rename` and an `.or_else` in `tail_ret_owned`; loft#1100 lands in the same file next and
REMOVES the `do_if_acc` nullability term in favour of an `(N-Store)` report before the accumulator
rewrite (`calls.md` D-call-5). The doc-comment-only edits to the five null-tail walkers above
should merge clean; the `do_if_acc` block itself has not.

#### B6h — the queue's second hit: a witness the bracket cannot name (2026-08-26)

`use_analysis::view_root_slots` is one of the sixteen the `spellings` screen lists, and it was
already read once (B6d) and recorded CLEAN — *"enumerates every exclusion with a reason, and
states the direction"*. Reading it a second time with the tuple question in hand gives the
opposite answer, and the difference is instructive: B6d asked whether the doc justifies the
FALLBACK, and this one does. What it does not say is that a projection has a spelling its walk
cannot reach.

**The defect.** A call whose return may BORROW an argument decides borrow-vs-owned at runtime
with the @P290 bracket. `view_root_slots` walks a projection chain to the variable that names
the store, recognising a projection with `is_projection_op` — `OpGetField` / `OpGetVector`. A
tuple element is neither, so the walk answers `None`, the witness set reads incomplete, and the
caller keeps the conservative never-free answer: it copies the returned store and orphans the
one the callee minted. **One record per call, both backends** — `loft#1104`.

| argument | leaked? |
|---|---|
| `pick(q, c)` — a bare var | clean |
| `pick(b.s, c)` — `OpGetField` | clean |
| `pick(v[0], c)` — `OpGetVector` | clean |
| **`pick(t.0, c)` — `TupleGet`** | **one per call** |

⚠ **And the fix is NOT the arm the screen points at.** Widening `view_root_slots` to answer the
tuple's base var would be wrong rather than incomplete: the bracket consumes a slot as a `DbRef`
VALUE — the native emit renders `n_protect_store_frees(cell, var_t)` — and a tuple local's value
is a `(DbRef, …)`, which does not compile there and which the interpreter would read as element
0's bytes whatever index was projected. **The witness mechanism cannot name a tuple element**,
and that is a real boundary. So the screen's verdict here is *"this site cannot answer the
question"*, not *"this site forgot an arm"* — a distinction worth keeping, because the two have
opposite fixes.

**The cure was already in the tree, for the family that cannot be witnessed for the other
reason.** loft#1029 hoists an argument still wrapped in its construction block, because the
bracket is emitted before the block runs. The two are opposites — that argument is not nameable
YET, this one is not nameable AT ALL — and the same hoist serves both: bind the projection to a
temp that BORROWS the tuple base, and the call site gains a name for it. The emitted IR is then
byte-for-byte the spelling an author writes by hand, which was always clean:

```
before:  r = n_pick9(t.0, c, __ref_1);
after:   __lift_1(1):ref(S9)["t"] = t.0;
         r = n_pick9(__lift_1, c, __ref_1);
```

Guard: `tests/scripts/a-tuple-element-argument-can-witness-the-bracket.loft`, falsified against
the pre-fix gate (60 leaked records → 0).

⚠ **Two cells of the matrix are STILL OPEN, and saying so is the point.** ✅ **Both are closed
as of later the same day — see B6i, including why the reason recorded below did not survive
re-measurement.** The axes the first
sweep held fixed were the index (fixed at 0) and the chain DEPTH. Moving the index found nothing
— index 1 is fixed by the same change. Moving the depth found two live cells the fix does not
reach: `pick(t.0.s, c)` (an `OpGetField` over a tuple element) and `pick(t.0.0, c)` (a nested
tuple, which lowers to a `tuple_tmp` block). Both still leak on both backends; both are clean
when bound by hand, so the cure is the same hoist.

What blocks them is the TYPE the hoisted temp must carry. The temp's DEP decides who frees the
store, so it has to be the projection's result type depending on the chain's ROOT — and
`scopes.rs` has no helper that types a `Value`; `inline_struct_return` derives from a callee's
DECLARED return, which a projection does not have. **A wrong dep there turns a leak into a
use-after-free**, which is the one direction this machinery's own comments warn about, so the
cells are recorded on the issue with their measurements rather than guessed at. Fixing one shape
of a two-shape class is what produced this defect in the first place; fixing two of four would
be the same mistake with better manners.

#### B6i — the two open cells closed, and the list under them was short for a second reason (2026-08-26)

B6h left two cells of loft#1104's matrix open and said why: the chained spellings `pick(t.0.s, c)`
and `pick(t.0.0, c)` *"need the hoisted temp to carry the PROJECTION's result type, which
`scopes.rs` has no helper to compute, and a wrong dep there is a use-after-free rather than a
leak."* Both are closed, and **the blocker was a premise, not a shortage** — the register's own
rule that a filed *"why I did not fix it"* is a claim to re-measure.

**Neither cell needs the chain's result type, because neither binds the chain.** Two axes, and
B6h had them fused into one:

| what reads the element | who gets the NAME | where the type comes from |
|---|---|---|
| `t.0` — a bare `TupleGet` | the argument itself | the tuple's declared element type, borrowing the base |
| `t.0.s`, `t.0[0]`, `t.0.w.s` — a projection CHAIN above it | the ELEMENT; the chain is RE-BASED on the temp | the same declared element type |
| `t.0.0`, `vt[0].0` — the element read off a non-variable | the `tuple_tmp` block, bound whole | the block's own result type, deps included |

The parser already wrote both types down. Binding the chain's RESULT would indeed have needed a
`Value`-typing helper this pass does not have; binding its BASE needs one the tuple declares, and
the chain then stands on a `Var` that `view_root_slots` has always walked. The emitted IR is
byte-for-byte the hand-written spelling in every cell — `__lift_1:ref(Wb)["t"] = t.0;
pick(__lift_1.s, …)` against `e = t.0; pick(e.s, …)`.

⚠ **And that arm is now DELETED, because the general question answers the same cells.** The
sibling checkout's loft#1105 cure walks the value for its deps (`lift_view_deps`), which reaches a
`TupleGet` too — so it types the temp for every one of these six spellings without being told the
shape. Measured before removing: the arm was REACHED 15 times across the 881-file corpus, the
emitted IR differs from the general answer on the chained cells (it binds the ELEMENT and re-bases
the chain; the general one binds the chain's RESULT with the dep walked to the same root), and
**every cell passes with the arm disabled** — values, leak gate and `LOFT_POISON` alike. Two
derivations of one answer that agree are still two derivations, and the one that reads the fact
off the value beats the one that asserts it from a declaration. 133 lines out, and the same
verdict the sibling reached about their own version of this arm from the opposite direction —
theirs was unreachable, mine was reachable and redundant.

**The filed matrix had two open cells; the family has six.** The sweep that produced the "two
open cells" had pinned the chain's OP (a field read), the container the tuple sits in (a local),
and the index. Moving those found three more before the fix, and a fourth only after it:

| cell | verdict before |
|---|---|
| `pick(t.0.s, c)` — `OpGetField` over the element | filed open |
| `pick(t.0.0, c)` — nested tuple | filed open |
| `pick(t.0[0], c)` — `OpGetVector` over the element | **leaked, not filed** |
| `pick(vt[0].0, c)` — the tuple in a VECTOR | **leaked, not filed** |
| `pick(t.1.s, c)` — the same chain at index 1 | **leaked, not filed** |
| `pick(t.0.0.s, c)` — a chain OVER a `tuple_tmp` | **leaked, and found only after the first fix** |

The last one is the one worth keeping: it was invisible until the block shape had a cure, because
until then the whole family read as one open cell. Guard:
`tests/scripts/a-tuple-element-argument-can-witness-the-bracket.loft`, now ten defect cells and
six controls, falsified against a pristine pre-fix build at **168 leaked records → 0 on both
backends** (`LOFT_NATIVE_LEAK_CHECK=1` for the native half — a bare `--native` run does not
leak-check).

##### The boundary that is NOT an arm, and why the block test is a `TupleGet` TAIL

Binding a block whole is admissible only because a `tuple_tmp` block's result type describes every
value it can yield. **A block whose tail is a JOIN does not**, and the difference is a
use-after-free rather than a missed optimisation. `v[i] ?? mk()` lowers to an `ncc` block typed
`ref(τ)["v"]` — a borrow of `v` — while on the else arm it holds a store `__ref_2` owns. Binding
that to a temp typed off its own result completes the witness set while protecting the WRONG
store, and the source-free it then licenses releases `__ref_2`'s record before the frame's own
`OpFreeRef`. So the shape is refused structurally (the tail must be a `TupleGet`), the reason is
written where the next reader will hit it, and the leak is filed as **loft#1105** with both cures
named and neither taken.

##### The list `view_root_slots` reads was short by two ops, and three homes already knew it

Walking into `is_projection_op` to widen it exposed the defect this section is really about, and
**there is no tuple in it**:

```loft
h: hash<S[a]> = [S { a: 7 }];
r = pick(h[7], c);        // one leaked record per call, both backends
e = h[7]; r = pick(e, c); // clean
```

`h[k]` lowers to `OpGetRecord(h, …)`, declared in `default/01_code.loft` as
`-> reference[data]` — the record it answers lives in the collection's own store, so the root
variable is exactly the witness the bracket wants. `is_projection_op` listed `OpGetField` and
`OpGetVector` and nothing else, so the site read as uncovered and kept the conservative never-free
answer. Reproduced at **hash, sorted and index** (one op serves all three), through a keyed
FIELD, and through a tuple.

**The op was not missing everywhere — it was missing from the home that claims to be the only
one.** `is_projection_op`'s doc said *"One list, two readers … Two lists of the same two ops would
drift."* The measurement says otherwise:

| site | list |
|---|---|
| `use_analysis::is_projection_op` (read by `view_root_slots` + `parser::projection_root_mut`) | GetField, GetVector |
| `scopes::base_container_var` | GetVector, VectorRef, GetField, **GetRecord** |
| `generation::container_element_base` | GetVector, VectorRef, GetField, **GetRecord** |
| `scopes::amp_writeback_owned_copy` | GetVector, VectorRef, GetField |
| `data::is_place_read` | GetField, GetVector, VectorRef |
| `generation::dispatch` — the `&`-ref construction | GetField, GetVector, VectorRef |
| `parser::operators` — the C86 view materialise | GetField, GetVector, VectorRef, **GetDbRef** |

Seven hand-spelled lists, four distinct memberships, and the two that had the right answer are
byte-identical copies of each other. The keystone claim was a measurement and it did not survive
one (`keystone-claim-is-a-measurement`, third time on this thread).

**Fixed by merging, not by adding an op in a fourth place.** `is_projection_op` now carries the
four ops, states the criterion it is NOT (*"the return deps on parameter 0"* — `OpNewRecord` and
`OpInsertVector` satisfy that and are excluded, because they GROW the store rather than read it),
and `base_container_var` and `container_element_base` call it instead of restating it. Guard:
`tests/scripts/a-keyed-lookup-argument-can-witness-the-bracket.loft`, six defect cells and three
controls, falsified at **156 leaked records → 0 on both backends**. The remaining four lists
stay a queue rather than a sweep: each asks a different question of the same shape (which place a
`&` may name, which view a bind materialises), and merging them on the strength of a shared op
list is the early-abstraction failure this thread was opened to avoid.

**They are also DE-RANKED, because the shape each one's missing op predicts was probed and every
cell is clean.** All four omit `OpGetRecord`, so the prediction is that a keyed lookup behaves
differently from a vector element at each: `e = h[k]; bump(e)` through a `&` parameter,
`r = &h[k]; r.b = 55`, and `w = mkg().h[k]` bound and read after forty stores of churn — the C86
escape the last one materialises for. Every cell answers what its vector twin answers, on both
backends. So the count of drifted lists is seven and the count of defects behind them is one; the
others need their own evidence before anyone spends a session on them, which is what the queue is
for.

##### One more the sweep found, and why binding does not cure it

Moving the argument-spelling axis onto a NULLABLE parameter turned up a leak that looks like the
family above and is not: `pick(q, c)` into `fn pick(a: P?, …) -> P?` leaks the minting arm's
record, one per call, both backends — and **binding `q` to a local first does not help**, which
is exactly the test that separates a witness gap from an ownership gap. What does help is
declaring the local `q: P?`.

The overlay names it in one line: `Optional(Reference)` is classified `— (scalar)`.
`data::is_dbref` lists the eight store-carrying kinds and not the `Optional` wrapper, so a
nullable record local owns nothing and nothing frees it. The obvious cure was measured and
REJECTED before filing — making `is_protectable_store_type` peel to `.base()` leaves the leak
untouched, because the bracket is not what is missing. Filed as **loft#1106** with that negative
result in it, since it is a decision about which of `is_dbref`'s callers see through the wrapper
rather than a widening of the list, and the list is the thing this thread keeps finding drifted.

##### The screen could see two of the three ways to resolve an op

`is_projection_op` dropped OFF the `spellings` list the moment it was rewritten to match by name
— the matcher knew `def_nr("OpGet…")` and `is_projection_op(`, and not
`data.def(d).name() == "OpGetField"`, which is how all seven lists above are written. So the mode
that found this class had been blind to most of the class the whole time. Widening the matcher
moved the count from 21 · 2 · 19 to **38 · 4 · 34** and turned up two more sites that DO handle
the tuple spelling (`generation::dispatch::output_set_inner`, `parser::operators::parse_part`) —
movement on both sides, which is what says the change is precision rather than noise. It also
surfaced `OpGetVectorNullable` as a spelling nothing lists at all.

⚠ **And the probe harness was scoring itself.** Its pass test grepped stdout for the literal the
program prints on success — and loft's error report ECHOES THE SOURCE LINE, so a failing assert
printed the marker as part of the offending line and every failure read as a pass. One cell
(`pick(g.all[0], …)` on a linked group) was recorded CLEAN on that basis when it was in fact
failing on a null: I had filled the group's other member, so the array was empty and the cell
asserted nothing. The fix is a marker the source cannot contain — a computed running sum — plus
the exit code. `channel-captured-never-compared` and `absent-warning-is-not-a-pass`, met together.

#### B6j — one TYPE, two spellings, and the rewrite that happens between the passes (2026-08-26)

B6g named the class as *a notion with two IR spellings*. This is the same class one level down —
**a notion with two TYPE spellings** — and it produced four failures that look nothing alike, in
one afternoon, from one root.

A nullable struct is written `f: S?` and reaches the parser as `Optional(Reference(S))`.
`typedef::synth_nullable_struct_fields` then rewrites the declared FIELD type to the synthetic
`Enum(__nullable<S>, true)`, because an inline field has no `DbRef` of its own and absence needs a
discriminant to live in (@PLN25 E2a.2 / loft#896). **That rewrite runs in `fill_all`, which is
between the two parser passes** — so every site comparing a nullable struct type sees one
spelling on pass 1 and the other on pass 2.

| what the author wrote | what happened | channel |
|---|---|---|
| `s = o.f` | **REFUSED** — *"cannot change type from `S?` to `__nullable<S>`; use a new variable name or cast with 'as'"*: one type reported as two, naming two cures that cannot reach it | a legal program rejected |
| `s = o.f ?? S { x: 6, y: 5 }` | `s.x` answered **5** and `s.y` read past the record | **silent-wrong**, both backends |
| `s = o.f ?? d` (a VALUE default) | the same, and no hint can cure it — a value cannot be re-parsed at another shape | **silent-wrong**, both backends |
| `s = v[i] ?? S { … }` | **REFUSED** with the synthetic name again | a legal program rejected |
| any of the above that reached a dense target | one leaked record per evaluation | the leak gate only |

**Each fix is at the site that had the wrong idea, and they are four different sites.**

* **`change_var_type`** now treats the two spellings as one type — a REFINEMENT across passes,
  not a retype. The synth wins, because the value really is a `__nullable<S>` record and both
  spellings occupy a `DbRef` slot, so the frame the two passes lay out is unchanged.
* **The `??` default's HINT** is the join's shape, not the destination's. Hinted with the dense
  target, `?? S { … }` built a bare `S` beside a `__nullable<S>` one and the payload projection
  over the join then read the literal at the payload's OFFSET. The coalesce deliberately keeps
  its nullable result — `build_null_coalesce_default`'s own *"E2 gap 2"* note says why — so the
  arms have to agree with each other and the dense target is reached afterwards.
* **A default that is a VALUE is WRAPPED** into a `Some` (`wrap_dense_default_as_some`), because
  the hint only reaches a literal. `build_some_present` already existed for the append path; this
  is its second caller.
* **`parse_some_payload_object` leaves a non-null placeholder on pass 1.** It builds no IR there,
  so its operand kept the `Value::Null` its caller initialised — and `??`'s `?? null` soundness
  check asks *"is this operand the `null` LITERAL?"*, which cannot tell that apart from *"not
  built yet"*. It typed the result `τ?` on pass 1 and pass 2 could not take it back, so
  `s.x` resolved against `__nullable<S>` and the program was refused.
* **The unwrap-copy target is typed as an OWNER.** The copy is what makes it independent; pass 1
  had typed it off the un-copied expression, whose deps name the holder. Stripped on the
  VARIABLE and not only on the type, because `change_var_type` treats a deps difference as no
  change at all.

**The predicate has one home now.** `Data::is_nullable_synth_of` asks the question the way the
MINT keys its cache — the name AND the struct's own SOURCE — and `Data::same_nullable_struct`
compares the two spellings as one type. The source half is not decoration: two libraries may each
define a `Chunk`, `nullable_enum_for` is per-`(name, source)` for exactly that reason, and the
**43 `starts_with("__nullable<")` and 13 `format!("__nullable<…")` sites already in the tree ask
it by NAME ALONE**. That is a queue, not a sweep — each is a different question of the same shape
— but it is the sharpest instance of this thread's subject yet: one notion, 56 hand-spelled
recognitions, and a bug in the gap between two of them.

⚠ **The same notion has a THIRD face, and the write-side one had drifted** — found in the
sibling checkout while fixing loft#1106, and it is the sharpest confirmation that the class is
real rather than a story about one type. A dep list is READ by `Type::depend`, SET by
`Type::with_deps` and CLEARED by `Function::make_independent`; the first two peel `Optional` and
`RefVar` explicitly, and the third **spells its own arm list with neither — and without `Text`**.
So a nullable local's dep can be read and written and never cleared, and a strip against one is a
silent no-op. That is why this branch's unwrap-copy strip states the dense-`Reference` target as
a PRECONDITION rather than an incidental fact. The cure is theirs (`Type::deps_mut`, the three
faces on one home); the line worth carrying is that **a membership list has as many faces as it
has verbs, and only the ones a test exercises stay in step.**

✅ **The `Text` hole was a FOURTH member, and it is closed — by measuring it rather than by
arguing it.** `deps_mut` peeled the two wrappers and had no `Text` arm and no `Function` one,
while `depend` reads a dep from both. It was left open on the grounds that adding the arm is a
behaviour change with no measurement under it, which was the right call at the time and the wrong
one afterwards, because the measurement is one env-gated counter:

| `make_independent` calls, 881-file corpus | on `Text` or `Function` |
|---:|---:|
| **13 926** | **0** |

Every call is `Vector` (8 871), `Reference` (5 017), `Optional` (5 — the arm loft#1106 added) or
four others, and twelve hand-written text / fn-ref shapes — a var-to-var bind, a field read, an
element read, `+=`, `??`, a call result, a `&text` parameter, a fn-ref reassignment — added none.
So the arms are **inert today and correct when a caller arrives**, which is the only ordering that
does not require someone to debug the no-op first.

**And the arms are the smaller half: the gate is `data::dep_faces_agree`.** It builds every
single-dep-list variant carrying one dep and asserts the READ face and the CLEAR face reach it,
then does the same through both dep-transparent wrappers — so a variant added to one verb and not
the other is a failing test rather than a silent no-op at whichever site asks the other question.
It was written FIRST and watched fail on `Text` before either arm went in. `Tuple` is the
documented exception (its deps are the union of its elements', so there is no single list to hand
back). **A membership list has as many faces as it has verbs — and now they cannot drift.**

⚠ **What made this findable was a matrix of PAIRS, not of cells.** Every nullable cell was
written beside its non-null twin or its direct-use twin — `s = o.f` beside `s = o.g`,
`s = o.f ?? d` beside `(o.f ?? d).x` — so the expected value never had to be hand-derived. That
is [reference-route-is-the-oracle](STABILITY_METHOD.md) applied to a type, and it is what turned
"`s.x` is 5" from a number into "the twin says 6, so the read is off by a field".

⚠ **And the first cell of that matrix was found by accident, while probing something else.**
The refusal turned up as a parse error in a probe file written for loft#1106's ownership
question — a cell that would not compile, in a matrix about something entirely different. The
matrix for THIS bug only exists because a cell that failed to build was read instead of edited
around.

**loft#1106 is unchanged by any of it, and that is the check that keeps the two apart.** Its
repro still leaks, and the aliasing symptom found beside it — a `-> S?` return ALIASES its
argument where the `-> S` twin COPIES, against `(B-Copy)` — is recorded on the issue, which is
now `silent-wrong` / `sev:high` rather than a leak.

#### B6k — the axis BOTH checkouts held fixed, and the leak it was hiding a use-after-free behind (2026-08-26)

The sibling checkout closed loft#1105 by generalising: instead of a fourth shape it asks
`bracket_can_name`, and binds any argument the witness walk cannot name. That is the right shape
of answer — its own register entry says it, and so does this thread: *a predicate that enumerates
SHAPES will keep being one shape short*. Built in an isolated worktree off this repo and measured,
it is leak-clean on every cell of the loft#1104 matrix, all three keyed kinds included.

✅ **Both are on this branch now, reconciled** — and so is the sibling's own cure for what follows
(`lift_view_deps`, cherry-picked): the temp carries the callee's parameter SHAPE with the deps its
VALUE borrows, and where no source can be named the argument is not bound at all, so a value with
no provenance keeps the leak it already had. That was taken over this branch's first answer — a
`skip_free` temp, which also stops the over-free and says less: `skip_free` answers *"do not free
me"*, a dep answers *"whose store is this"*, and the second is the question `Type::depend`'s other
readers ask. `is_view_op` and `is_projection_op` carried the identical four names by then and are
folded onto one.

**Both are on this branch now, reconciled** — the sibling's eleven commits are cherry-picked
here so the two do not diverge, with the general arm ORDERED LAST and its temp `skip_free`. What
follows is the finding as it was measured against their commit, which is what the guard cells
below still score.

**And it frees a record the CALLER owns.** The temp it binds is typed from
`callee.attributes()[arg_idx].typedef` — the callee's DECLARED parameter, which carries no deps —
so a temp holding a VIEW reads as an OWNER and `get_free_vars` emits a free for it:

```
__lift_1(1):ref(Sb) = OpGetRecord(h(1), 84i32, 1i32, 7i32);   // a view INTO h's store
r(1) = n_pickb(__lift_1(1), c(0), __ref_1(1));
OpFreeRef(__lift_1(1));                                        // frees h's record
```

⚠ **Neither checkout's matrix could see it, and the reason is one axis both of us pinned: every
cell built its container INSIDE the function that called.** A bogus free then lands on a store
that was about to die at the same scope exit, so it is absorbed by `H-FreeTwice`'s silent no-op
and neither the value channel nor the leak channel says anything. Moving that one axis — pass the
container IN, so it outlives the call, then read it back after enough churn to recycle the record —
turns it into `after=null` (hash), `after=12884901900` (tuple), and a `LOFT_POISON=1` panic:
*"Store access out of bounds: rec=3735928559 … the reference is corrupt, not merely out of range"*.
`--native` answers 71 for the hash cell and garbage for the tuple one, which is the
backend-dependent signature of a use-after-free rather than a second defect.

**A leak channel cannot score an over-free.** That is the general lesson and it is the mirror of
[absent-warning-is-not-a-pass](STABILITY_METHOD.md): this whole family was found through the leak
gate, every cure is scored by it, and the gate is monotone in the wrong direction — freeing MORE
than you should always reads as an improvement. The cells are now in both guards
(`kb_outlives_*`, `tb_outlives*`), they fail on the sibling build and pass here, and the rule they
encode is: **whenever a fix ADDS a free, one cell must let the freed store outlive the frame that
freed it.**

**The two fixes compose rather than compete, and the emitted code says which is which.** Where
`is_projection_op` knows the op, the bracket NAMES the store and no temp is created at all —
`n_pickb(OpGetRecord(h, …), c, __ref_1)` against the sibling's bind-then-free. Where a tuple stands
in the way, binding the ELEMENT (typed off the tuple's own declared element type, deps included)
and RE-BASING the projection keeps the projection at the call site. So the precise witness is not
a rival to the general bind; it is what keeps the general bind from having to find a type it
cannot compute. The cure the sibling needs is the same fact from the other side: the temp must
carry the ARGUMENT's deps, and where the chain does not bottom out in a nameable var it must not
be bound at all — a temp you cannot type correctly is one you must not create, which is exactly
why loft#1029's construction is HOISTED rather than bound.

#### B6l — the entry point is part of the harness, and a comparison is only as good as the one both sides share (2026-08-26)

Two method faults, found within an hour of each other, both of which had already produced a
"verified" reading that measured nothing.

**A `--interpret` run of a guard file may execute NOTHING.** `tests/wrap.rs::run_test` runs
`main` if the file HAS one, and otherwise every zero-parameter function. A guard written in the
`fn test_*` idiom therefore has no `main`, and `loft --interpret <guard>` compiles it, prints its
diagnostics and its leak line, and exits 0 having run no assertion. Four cherry-picked guards were
checked that way here and read "clean" for that reason; re-scored under `--tests` they are
genuinely green (7, 8, 9 and 8 assertions), so the readings survived — by luck of file shape, not
by method. The two guards written here carry a `main` and were being run correctly.

The rule is not *"use `--tests`"*, because that is wrong for half the corpus: **`--tests` on a
`main`-ful guard runs the zero-parameter HELPERS and not the assertions** — it reported "4 passed"
for a file whose `main` runs thirty. The rule is that the entry point is part of the harness, the
file's shape decides which one the corpus runner picks, and the number to read is the ASSERTION
COUNT rather than the exit code.

⚠ **THREE ways to run a comparison that measures nothing turned up in one afternoon, all of them
reporting success**, and that is the thing to write down rather than any one of them. Beside the
two below: a probe harness scored on a marker its own SOURCE contains, which loft's error report
echoes (§ B6i); and, in the sibling checkout, a guard cell that passed on a control because a
non-null return never reaches the code path it was written for. Different costumes, one shape —
**a channel that cannot see the subject reports agreement.**

⚠ **The second fault is the one worth the section, because it makes a difference disappear.** An
on/off comparison of a compiler arm was first run through `--tests` on a `main`-ful guard and
answered *"4 passed / 4 passed"* — which reads as *the arm changes nothing* and is instead *the
four helpers do not exercise it*. Re-run through the entry point that runs the assertions, the
same comparison is what settled the arm's fate. **A comparison inherits the blindness of the
entry point both sides share**, and two identical numbers from a channel that cannot see the
subject are the most convincing wrong answer available.

#### B6m — what one day of 11 tickets says about the apparatus that found them (2026-08-26)

Eleven issues (loft#1096–#1106) closed across two checkouts in a day, plus about nine more defects
fixed without a ticket. Worth asking what that says about the quality work itself rather than
only about the code, because the answer is not flattering in the direction one would expect.

| the window | |
|---|---|
| tickets | 11, **all `hit-by:loft`** — no consumer reported any of them |
| both backends | 11 of 11 |
| `silent-wrong` | 5 of 11 · `sev:high` 5 of 11 |
| areas | store-lifetime 7 · parser 6 · runtime 4 · codegen 2 · native 1 |
| commits | 41, of which **3 fixed one of the other 38** |
| guard-bearing commits | 29, of which **12** record a falsification against a control |
| bugs filed in August | **258**, against 67 in July and 54 in June |

##### 1. The gate is biased toward FALSE GREEN, and that is the headline

Four distinct channels reported success while measuring nothing, in one afternoon:

* **the entry point** — `--interpret` on a `main`-less guard runs no assertion at all, and
  `--tests` on a `main`-ful one runs the zero-parameter HELPERS (B6l);
* **the probe harness's own marker** — grepping stdout for the literal the program prints on
  success, which loft's error report echoes as part of the offending source line (B6i);
* **the leak gate's direction** — monotone, so an over-free always reads as an improvement (B6k);
* **a guard cell that cannot reach its subject** — a non-null return never reaches the join bind
  the cell was written for, so it passed on a control (found in the sibling checkout).

And two defects passed a FULL green gate — `make ci` 4464/4464 plus 42/42 published libraries —
and were caught only by a second reader: a use-after-free in loft#1105's first cure, and a silent
wrong answer that needs TWO commits present and so could not fail either commit's own suite.

**The apparatus is good at FINDING and weak at CONFIRMING.** Every instrument built this month
points at candidates; almost nothing verifies that a check can fail. That asymmetry is the single
most actionable thing this window shows.

##### 2. Every matrix pinned an axis that mattered — five times in one day

loft#1104's filed matrix pinned chain depth AND container AND index; loft#1105's pinned container
LIFETIME (every cell built its container inside the calling function, which is what hid the
over-free); this branch's nullable matrix pinned the DEFAULT'S SHAPE (literal vs value); D-own-6's
oracle pinned argument SPELLING and said so in its own closing paragraph; loft#1098 exists because
someone moved loft#1097's axis.

The matrix-first rule is being followed and still fails, because **"which axis did I hold fixed"
has no instrument** — it is entirely a matter of the author remembering, and every one of these
five was caught by a different person, on a different day, or by accident.

##### 3. The instruments find CLASSES; people find DEFECTS

Of the eleven tickets the `spellings` screen produced two. The largest single yield of the day —
four defects in the nullable-type-spelling family — came from **a parse error in a probe written
for a different issue**. loft#1106 came from re-measuring a filed blocker's own excuse. The keyed
lookup came from walking INTO a predicate in order to widen it.

That is not an argument against the tooling: a screen that ranks 35 sites is what makes reading
them tractable. It is an argument against expecting DETECTION from it, and against reading a
quiet screen as a quiet subsystem.

##### 4. The sample is entirely self-generated, so it measures our reach and not the language

Eleven of eleven `hit-by:loft`, seven of eleven in one subsystem. The store-lifetime concentration
may be where the defects are or may be where we were standing; this window cannot tell the
difference, and neither can the monthly bug review while the filing rate is dominated by our own
sweeps. August's 258 against July's 67 is a fact about how hard we looked.

##### 5. Two agents in one subsystem need a sequencing discipline, not just a shared register

The three self-corrections were all integration faults, not coding errors: an arm ordered so that
another fix became unreachable, a temp typed off a declaration that carried no deps, and a slot
written before the call that still needed it. The register numbers collided as well. Both
checkouts had "one home for the fact" and it did not help, because **one home secures the
QUESTION and says nothing about WHICH VALUE each caller hands it, or WHEN.**

##### 5b. A home that is RIGHT and not asked leaves no trace to screen for

The one-home work has so far been about a question with **N spellings**, where the fix is to make
the readers agree and the screen is `scripts/rule_predicate_audit.py` finding the duplicated
variant sets. loft#1250 is the other failure in the family and it is invisible to that screen.

`Variables::const_report_var` maps a promoted `__tp_` text local back to the parameter it came
from and every other variable to itself. It was written for exactly this, its doc-comment says
so — and it was asked by **one of the five sites that report a const modification**: the one
that was already right. The other four read the raw variable, so `fn f(s: const text)` refused
with *"Cannot modify const parameter `'__tp_s'`"*, naming a variable that appears nowhere in the
author's program.

**Nothing about that is detectable by looking for drift.** There is no second home, no
disagreeing spelling, no duplicated variant list — the four silent sites contain nothing to
match, because what they do is the RAW thing the helper wraps, which is also what every correct
site that does not need the helper does. The home being right is what makes it invisible.

If a screen for this is worth writing, the shape to look for is: **a `pub(crate)` predicate
whose doc-comment names a question, with callers performing the raw operation it wraps in the
same diagnostic or decision.** A weaker but cheaper proxy that would have caught this one: a
helper with exactly ONE call site and a doc-comment written as a general rule — the mismatch
between "here is how to answer X" and "answered once" is the signal. Neither is built; the
observation is recorded because the class is now known to exist and cost a shipped diagnostic
that named a variable the program does not contain.

##### 6. Performance is invisible to the whole apparatus

loft#1109 — a 26 % regression on the tuple return introduced by loft#1102 — was found by hand
afterwards. Every gate we run is a CORRECTNESS gate, `make speed` is explicitly a report nobody
blocks on, and not one of the 41 commits measured an op count except where somebody chose to.

##### What to do about it, ranked

1. ✅ **Make the negative control mechanical — DONE.** `scripts/falsify.sh` /
   `make falsify GUARD=… REF=…` builds the control, runs the guard on both trees **through the
   entry point the corpus runner would pick** (derived from the file, because getting that wrong
   is the first of the four channels above), and compares exit code, assertion failures, leaked
   stores and panic APART — so the recorded line names which channel moved rather than only that
   something did. `doc_hygiene::every_new_guard_records_its_control` requires an
   `// @falsified-at:` line on every new file under `tests/scripts/`, ratcheted against
   `tests/falsified.baseline` (894 pre-existing files, shrinks only). `none — <reason>` is the
   honest opt-out. Both the tool and the gate were falsified before they were trusted: the tool
   reports INERT against a ref that already carries the fix, and the gate fails on all three of
   its shapes — a new file with no record, a retrofitted file still in the baseline, and a
   baseline line for a file that no longer exists.
2. ✅ **A screen for `Optional` transparency — DONE.** `ir_walker_audit.py optional`, both halves:
   who discriminates on a `Type` variant without peeling, and — the list that did not exist — who
   goes through `.base()` before asking each opaque verb. It reproduced 15 of 15 hand answers, and
   the corpus ranking behind it says only four (verb, caller) pairs are ever REACHED by a `τ?`.
   Three peels and one lock came out of it; see B6p.
3. ⚠ **Record the axes a matrix HELD FIXED — DONE, but not in the form proposed.** The DECLARED
   form is falsified by this repo's own register: D-own-6 wrote its pinned axis into its closing
   paragraph and four more defects still came from moving it (*"an axis named in a closure is not
   an axis measured by it"*). A declared list is only as good as the author's awareness, which is
   the thing that failed. What shipped is DERIVED — `scripts/matrix_axes.py`, eight axes each
   carrying the DOMAIN the language offers, so the tool can name an axis nobody considered.
   Scored 6 of 6 against hand answers written first. Its own depth-RANKING was then falsified by
   the same oracle, so there is no corpus queue; the products are a per-file census and a PAIR
   cross. See B6q.

#### B6n — retrofitting 878 guards, and five ways the retrofit reported success while doing less than it said (2026-08-27)

The negative-control gate (B6m recommendation 1) shipped with a baseline of **878** guard files
that predate it. Retrofitting them is mechanical in principle — the control for a guard is the
PARENT of the commit that added it, `git log --diff-filter=A` answers that, and
`falsify.sh --bulk` groups by ref so each control is built once. 187 distinct refs, one shared
target dir so the dependency crates compile once: **61 s cold, 8.7 s warm**, which turns six
hours of building into thirty minutes.

**The verdicts split three ways, and the split is the deliverable.** A file that has FAILED on an
earlier build can catch a regression; a file that never has is a LOCK on current behaviour. The
retrofit says which in the line rather than writing one uniform string:

| verdict | the line it writes |
|---|---|
| **falsified** | `@falsified-at: <ref> — <channel and numbers>` — e.g. `exit 139 -> 0` (the control SIGSEGVs), `leaked kt=64 M×150 -> clean` |
| **INERT** | `none — LOCK, not a guard: measured inert against <ref> …, so it has never failed on any build` |
| **annotation-scored** | `none — scored by @EXPECT_ERROR / @EXPECT_FAIL, where a REFUSAL is the passing answer` |
| **not runnable alone** | `none — a plain single-file run cannot score this one` (the `850*` cross-package guards need `--lib` dirs) |

⚠ **The INERT residue is real and its reading is not the obvious one.** Around a quarter of the
corpus passes on its own control — and a large part of that is PROVENANCE, not rot: `@PLN25`'s
finish-line commit added a batch of files for a model that landed behind a default-on gate, so
the parent already answers the same. Those files are not broken; they are locks, and calling them
guards was the inaccuracy. Four were hand-checked before any of it was written down
(`372-field-elem-set-nested-vector-uaf` answers 4 passed on both trees).

##### Five ways the retrofit itself reported success while measuring less than it claimed

This is the part worth the section. The tool built to catch inert measurements produced five of
them, in one sitting, and each was found by a different accident:

| | what reported success |
|---|---|
| **`--path` with no separator** | `run_tests` builds the stdlib directory as `default_dir.to_string() + "default"`, so `--path /tree` looks for `/treedefault` and answers *"cannot load default library"*, exit 1 — which reads as a DIFFERENCE and scored every `main`-less guard as falsified by the tree. A quarter of the first sweep's verdicts. |
| **a missing `--path` on the other side** | giving the HEAD build its own `--target-dir` (so it cannot take the main cargo lock during a gate) without the matching `--path` made HEAD exit 1 for want of a stdlib |
| **stdin swallow** | `git worktree add` and `cargo build` read stdin; inside a `… \| while read` loop they ate the rest of the ref list. The sweep stopped after **51 of 186 refs, in order, with exit status 0** — a run that did 29 % of its work and reported success |
| **no outer bound** | an OLD control running a NEW guard hung somewhere `LOFT_TIMEOUT` does not reach — measured at ten minutes against a 180 s bound — and the sweep stopped silently on that one file |
| **a self-matching `pkill`** | `pkill -f "falsify.sh --bulk"` matched the shell running the `pkill`, so the command killed itself and reported the job as still running |

Three of the five are the SAME shape as the four in B6m — a channel measuring the harness rather
than the subject — and two are new: **a loop that silently processes a prefix**, and **a bound
that is not a bound**. Both now have their cure written where the next reader hits it: the ref
list is read on FD 3 with `</dev/null` on every command that reads stdin, and every run carries a
`timeout` backstop so a run the outer bound kills scores `exit 124`, which is a difference like
any other and says plainly which side could not finish.

**The generalisation, which is not "test your tools":** every one of these five was found by
noticing that a NUMBER was wrong — a quarter of a class sharing one verdict, 51 being suspiciously
round, zero rows after fifteen minutes. None was found by reading the code. **A batch instrument
needs a count you can sanity-check at a glance**, and the ratio between what it processed and what
it was given is the cheapest one there is.

⚠ **And the corollary, from the sibling checkout, which is the sharper half:** *a number that is
wrong in an interesting way is a finding even when it is not the finding you were after.* Their
`--tests` run reported **3255 files** where one was named, and it read as noise for an hour
because they were hunting a leak; mine reported **51 of 186 refs** and read as a completed sweep
because I was reading verdicts. Same failure in opposite directions — a count nobody was looking
at, carrying the answer. What came out of theirs was loft#1113, a SIGSEGV in a three-condition
closure shape that has been in the tree for months, reachable only because the tree walk ran a
probe under `doc/claude/plans/**/probes/` **that no suite runs** — which is a second finding
again: a whole directory of executable `.loft` files no gate reaches.

#### B6o — the directory no gate reaches, measured: 857 files, two faults, both already filed (2026-08-27)

B6n ended on a finding it did not work: `doc/claude/plans/**/probes/` holds executable `.loft`
files that no suite runs, and loft#1113 was reachable only because an unrelated change happened
to walk one.  That is now a measurement rather than an anecdote.

**857 checked-in `.loft` files under `doc/`; 25 of them are named by a test.** The rest are the
residue of finished investigations — they kept parsing and running long after their plan closed,
and nothing would have said if one stopped.  `make doc-probes`
(`scripts/doc_probe_sweep.sh`) runs each on the current build and reports the channel:

| | count | reading |
|---|---:|---|
| clean exit | **798** | ran to completion; says nothing about the ANSWER (see the caveat below) |
| refused (exit 1) | **55** | read: all legitimate — the `40-reshape-refusal/X*` probes assert exactly the diagnostic they get, four more are stale against the `spacial`→`spatial` rename (@PLN48), and `80-nested-closure` is refused by the restriction its own header documents |
| hard fault | **4** | two environmental, two real — **2 after the fix below landed**, both environmental |

**The two real ones are loft#1113, and the second file is new to it.** Both
`52-value-block-borrow-cleanup/probes/85-closure-returns-coalesce.loft` (the file the issue was
filed from) and its sibling `86-prebind-closure.loft` SIGSEGV at `OpAppendText`.  86 is the
WORKAROUND 85's header prescribes — it binds the element before the lambda instead of capturing
the vector — so the two files together already showed that capture is not the axis, which is what
the issue's matrix later established by moving it.  The other two faults are this box, not the
language: a graphics fixture cdylib that registers no marshal bridge for two of its `#native`
functions, and a path dependency on a `loft-ffi` directory that does not exist here.

**The root cause, which the issue does not yet carry.** The fn-ref call ABI passes **exactly one**
text work buffer for a text-returning target, and three sites say so — the injection in
`parse_call_ref` (*"one work_text matches the canonical one return buffer per text fn ABI"*), the
P227 ensure in `text_return` (*"callers always allocate exactly one buffer per text-returning
fn-ref call"*), and the runtime reconciliation in `State::fn_call_ref`, whose two arms are
0 buffers (pop the spurious one) and 1 (keep it).  But `TextDep::PromoteHidden` promotes one
buffer per promotable text LOCAL, and a `return e ?? "fb"` body has two of them — the local and
the `??` accumulator.  The callee then declares two hidden `RefVar(Text)` attributes where the
caller supplied one.  Native makes this visible rather than fatal: it aliases the single binding
into both parameters, `n___lambda_0(cell, _farg_0, _farg_0)`, which is the E0499.  The emitted
lambda BODY is byte-identical to the working named-function twin, so the callee side was never
wrong — only the call.

**Fixed in the sibling checkout and cherry-picked here** (`4458a9ec`, their `1d00a25f`) — the
enforcement point this analysis reached independently: both minting sites read one
`holds_text_work_buf` predicate, so the first promotion to ask takes the buffer and a later text
local stays a local, delivered by copy exactly as `SkipOwnedLocal` already prescribes.  Verified
after the pick rather than assumed: the two doc probes print their own `PASSED` again, the 20
matrix cells above pass on BOTH backends, and the guard
(`tests/scripts/1113-a-lambda-carries-one-text-work-buffer.loft`, 15 asserted cells) carries
`@falsified-at: 20e25e9a — interpret exit 139 -> 0, native exit 1 -> 0` — a line re-measured here
rather than taken on trust, and it reproduces exactly.  `make ci` 4473/4473, and the sweep that
opened this entry now reports two hard faults where it reported four.

⚠ **And their reading of the shape is wider than mine was** — worth recording because my matrix
agreed with the filed scope and the filed scope was a third of the defect.  I moved backend,
result type, capture, entry point and `return`-vs-tail, and concluded the crash needed a lambda +
`return` + `??` + text.  Only the LAMBDA is real: `?` reaches it (the other discharge rule), so
does the null branch, a zero-argument closure, and **two plain text locals with no discharge
anywhere**.  What the shapes share is a SECOND buffer, not the spelling that asked for one — and
`??` was simply the cheapest way to ask.  The axis I never moved was *how many promotions the body
makes*, which is the axis the defect lives on; it is now `formal/closures.md` deviation D-clo-4,
whose `OPEN: 0` has been re-measured twice and broken both times.

##### What the sweep cannot say, and the two ways it said something wrong first

⚠ **Crash channels only.** These files carry no expected values — that is what
`scripts/probe-matrix` requires and what they do not have — so a clean run means *nothing
faulted*, never *everything is correct*.  798 exits of 0 are 798 programs that finished, and the
sweep is blind to every one that finished with the wrong answer.  It is a REPORT for the same
reason: some checked-in probes fault ON PURPOSE (`parallel_read_parentvar_SIGSEGV.loft` is named
for it), so a verdict would need a baseline this does not keep.

B6n's rule — *a batch instrument needs a count you can sanity-check at a glance* — was applied
here and caught both of this sweep's own errors, neither by reading the code:

| | what reported wrong |
|---|---|
| **`.loft` is an extension AND a directory** | a run writes its cache to `.loft/` beside the script, so `find -name '*.loft'` matched 20 cache DIRECTORIES and scored every one as a failure.  The count said 877 where the tree holds 857 |
| **a bound too TIGHT** | at 20s under six-way load, a 16s performance probe and a 28s parse were reported as crashes — five faults that were the harness.  This is B6n's *"a bound that is not a bound"* in mirror image, and the cure is not a bigger number: a run killed by either bound is now its own class, because the only honest verdict on one is *re-run it alone* |

#### B6p — the screen one type former over: 43 opaque sites, 4 that a `τ?` ever reaches (2026-08-27)

B6m's ranked list put a screen for `Optional` transparency second, and named why: *"`is_dbref`,
`heap_dep` and `deps_mut` each decide whether they peel the wrapper, three verbs disagreed this
week, and there is no list of which callers go through `.base()`."*  `Optional(τ)` is `τ` with a
nullability bit and the SAME storage (@FR-L-Null: layout(τ) = layout(τ?)), so a site that resolves
a shape by naming variants answers for `τ` and not for `τ?` — one notion, two spellings, which is
`spellings` (B6g) with the type former swapped for the IR one.

`./scripts/ir_walker_audit.py optional` asks it mechanically, in two halves.  The FUNCTION half
classifies every body that discriminates on a `Type` variant; the CALLER half is the list the
first half cannot give — for each opaque verb in `data.rs`, who peels the receiver before asking
and who does not.

| functions discriminating on a `Type` variant | see through the wrapper | descend via the keystone | opaque |
|---:|---:|---:|---:|
| 729 | 365 | 5 | **359** |

`Parser::for_type` moved from opaque to PEELING — `729 | 365 | 5 | 359` — which is the
direction this table exists to drive, and the entry @PLN25's own dn1 audit had already written
down and nobody had taken: *"`for x in nullable` misses Text/Integer arms → peel in_type"*.
The element type of a nullable collection is its element type; the `?` is a fact about the
collection, not about what it holds. Unpeeled, every arm missed and the fall-through reported
*"Unknown in expression type `vector<T>?`"* — TWICE, plus a kind-list that recited the kind the
author had actually used. Peeling does not make such a loop legal (there is no `τ? ⤳ τ`), so
the refusal still stands; it just stands alone, and the cascade for `for x in v` on a
`vector<integer>?` goes from five errors to three with the informative one first.

loft#1403 added one on the OPAQUE side — `729 | 364 | 5 | 360` — `snapshot_kind` in the
`#remove` refusal, which reads a type to name the collection kind the author wrote.  Opaque is
the right column, and the reason is REACHABILITY: a nullable collection cannot be iterated at
all (*"cannot iterate over `hash<Ent,[\"k\"]>?`"*), so a `τ?` never arrives by the direct
route.  ⚠ It is also the right column for a second reason a peel gets backwards — a nullable
SIBLING field is not a candidate for *"which field is this loop over"*, so counting it made a
decidable case answer with the kind-neutral wording (measured: `{ data: hash<E[k]>, spare:
hash<E[k]>? }` said "this collection" where it can say `hash`).  I shipped the peel first on
the reading that `?` is only a marker over the same storage; the sibling checkout put it in the
opaque column instead and was right, and the two cells above are the measurement that settled
it rather than either reading.

D-bind-25 added one function on the SEEING-THROUGH side — `730 | 365 | 5 | 360` —
`scopes::reshaped_containers`, which now asks a keyed removal's container which KIND it is so a
`sorted` (the inline one, `@FR-Col-RemoveDense`) counts as a reshape where the four own-record
kinds do not.  It reads through `.base()`, so a `sorted<E[k]>?` answers as its dense twin does:
the `?` says the slot may be absent and says nothing about how the kind renumbers.

loft#1403 added one function on the OPAQUE side — `729 | 364 | 5 | 360` — the `snapshot_kind`
helper that names the three SNAPSHOT-iterating collection kinds (`Type::Hash | Type::Trie |
Type::Radix`) so the `#remove` refusal can say which one the author actually wrote.  Opaque is
the right column: the receiver it classifies is a collection the parser has already resolved,
so a `τ?` cannot arrive there, and peeling would only add a step that never fires.

loft#1389 added one function on the OPAQUE side — `Parser::change_var`'s
self-dep strip, which now names `Type::Reference | Type::Enum(_, true, _)` where it used to
name one of them.  Opaque is the right column and the right answer: the strip is stated over
the two RECORD kinds deliberately, and the collection kinds it does not name carry the @P302
re-init-in-place ownership marker rather than a degenerate borrow (6418 of them in the corpus
against no `Enum` at all — `formal/ownership-history.md` D-own-37).  A peel here would widen
the strip onto exactly those, which is the wrong answer, not the wider one.

The `@FR-O-Complete` walk (B7u) moved one function from opaque to seeing-through:
`scopes::needs_pre_init`, which names the locals that get a null before a branch and the
hoist out of a loop body, now asks its shape question through `base()` — the nullable
spellings were the whole finding.  loft#1362 moved one FUNCTION-half site: `scopes::scan_set`'s latest-assignment memo (`owned_refs`) reads the local's type through `base()` now, so a nullable record local has an entry for the reassignment release to read.  The projection-view marking added one on the seeing-through side (`scopes::nullable_view_locals`, which reads `function.tp(v).base()`), taking it to `702 | 345`.  Joining the `@FR-O-Complete` walk (B7u) onto @PLN153 phase 2 re-measures both rows on the tree that holds both streams: optional `713 | 356 | 5 | 352`, unspan `399 | 375 | 24` — neither branch's number, as every join so far.  @PLN153 phase 3a re-measures once more: `714 | 357 | 5 | 352` — `nstore_unwrap_report` and `convert`'s store faces read the wrapper through `Type::Optional` arms and `is_dbref`, the seeing-through side; unspan `399 | 375 | 24`.

The row is re-measured after each join rather than reconciled by arithmetic: the two checkouts had `678 | 324 | 5 | 349` and `678 | 325 | 5 | 348`, and the merged tree is neither.  It happened again on the 2026-09-03 join — one side carried `684 | 330 | 5 | 349` and the other `687 | 331 | 5 | 351`, and the tree that holds both measures `689 | 332 | 5 | 352`; the 2026-09-03 evening join (D-bind-11 onto the #1318 tree) measured `692 | 333 | 5 | 354`, loft#1327's opaque-fn-ref clause moved one function off the opaque column, and the D-own-8 closure moved two more onto the peeled one — one arm peeled (`gen_set_first_at_tos`'s null-init) beside two new scope-pass predicates.  The tree that holds BOTH measures `694 | 337 | 5 | 352`, which is neither side's number: two branches each adding predicates cannot have their counts added, because the audit classifies FUNCTIONS and a merged body is one function however many branches touched it.  loft#1333 then moved it to `695 | 338 | 5 | 352` with `scopes::mixed_ownership_locals`, the pre-scan that asks whether a binding is assigned a VIEW on one path and a delivered collection on another; it reads `function.tp(v).base()`, so it peels the wrapper and lands on the seeing-through side, leaving the opaque column where it was.  loft#1335 moved one function the other way, from opaque to seeing-through — `697 | 341 | 5 | 351` — because `fnref_result_type` stopped listing the shapes it bridges and asks `Type::base()` / `borrow_deps` instead, which is the cure that closed it.  loft#1349 added one opaque function, `Parser::boxed_tuple_return` — `698 | 341 | 5 | 352` — which matches `Type::Tuple` bare on purpose: a nullable tuple is not a shape it boxes, so peeling there would be a wrong answer rather than a wider one.  loft#1357 added two on the seeing-through side — `700 | 343 | 5 | 352` — `Parser::lambda_text_buffer_var` and `scopes::any_text_return_buffer`, both asking which hidden attribute is a `RefVar(Text)` buffer, the one home the text-return deliveries read.  The `@FR-O-Witness` walk (B7v) added one on the opaque side — `712 | 353 | 5 | 354` — a value-branch shape guard that matches a bare heap `Type` variant to decide whether a reassignment is sunk per arm (records only; a nullable is peeled off first).

The most recent movement is loft#1327's, and it goes the right way for the ordinary reason: the
new clause asks `is_dbref(ret_type.base())` of a fn-ref call's return, so the function it sits in
peels the `Optional` wrapper instead of matching a bare variant. The total holds at 692 — the
clause adds no new discriminating function, it changes what an existing one asks.

An earlier movement went the OTHER way, and it was also the screen working. Giving the
capture-adoption rule one home (`capture_adoption_owns_free`) took the `is_dbref(.base())` call
out of `check_ref_leaks`' body — and that call was the only peel in it. The function drops from
"see through the wrapper" to opaque, which is what it always was: the shape test it actually
performs is a bare `if let Type::Reference(_, dep)`, so the assert never examines a `Vector` or
a keyed local at all. A peel elsewhere in the body had been standing in front of that, exactly
the masking B7i's note below describes. The number got worse and the tree did not: the new
predicate is not counted here (it discriminates through `is_dbref` rather than on a variant of
its own), so the total holds at 684 while the honest column gains one.

loft#1308 moved two sites off the opaque column, and they are the ones this screen exists to
catch. `get_free_vars` decided whether a captured local's scope-exit free is suppressed by
asking `matches!(function.tp(v), Type::Reference(_, _))` — bare, so a capture whose store is a
`Vector` or a keyed collection failed the test and the frame freed it under a live escaped
closure. It now asks `is_dbref(.base())`, and `check_ref_leaks`' mirror of the same exemption
asks it identically, because the two going out of step is what kept the defect hidden: the leak
checker agreed the store needed no free, so nothing contradicted the suppression. That is the
sixth and seventh entry in the drifted-list family the loft#1150 note records (`is_dbref` here
and at D-own-13, `deps_mut`, `is_keyed`, `depend`).

The TOTAL rises by two on the same change, which is the honest direction: the fix needs a
predicate that says which captures the record's death cascades through, and giving it a name
(`capture_attr_is_cascade_relevant`) makes it a function this screen can see, where before it
was an inline `matches!` inside a larger body and invisible here. It reads `.base()`, so it
lands in the see-through column rather than the opaque one — `@FR-L-Null` gives a `τ?` capture
the same storage as its dense twin, so an `Optional(Reference)` attribute is exactly as
cascade-relevant. Measured both ways: bare, the row is `682 | 328 | 5 | 349`, and peeling moves
that site across with every capture guard, the ownership oracle and the leak sweep unchanged.
Two callers now share it — `mark_borrowed_captures`, deciding which captures get a verdict, and
`capture_is_adopted`, deciding whether a frame-exit free may be suppressed — and a capture the
first skips must not be one the second adopts, or the store is freed twice.

loft#1313 added two to the total and both to the OPAQUE column, with the argument for it
written at each site — and loft#1316 then DELETED both, which is worth keeping rather than
quietly editing out.  `Parser::field_has_no_nullable_spelling` asked whether a field was the
`reference<T>` back-pointer of a cycle and `Data::reference_cycle_back_to` walked those edges;
both read `Type::Reference` bare, and the case for not peeling was that the absence of the
wrapper WAS the question.  The second half of that case was: *a cycle containing an `Optional`
reference edge is unconstructible, because that is exactly the field loft#1316 reports a layout
error for.*

That premise was a defect, not a fact about the language.  `reference<T>?` failed layout because
the `?` was routed to `@FR-L-Null-Tag`'s inline tagged form when `@FR-L-Null` governs a pointer;
with that fixed the edge is perfectly constructible, the field HAS a nullable spelling, and both
functions lose their subject.  So the screen's verdict was sound and its INPUT was not — an
argument for opacity that leans on "no program can build that shape" is only as good as the
reason the shape cannot be built, and here the reason was a bug one layer down.  That is the
transferable part: when a site justifies reading a type bare by saying the wrapped form is
impossible, the claim to check is the impossibility, not the reading.

So the column counts sites that decide a shape without peeling; it is not a defect list, and a
site whose question IS the wrapper belongs there — but a site whose question is *"can this
wrapper exist?"* is making a claim the register can falsify.

The row moves to `683 | 328 | 5 | 350` on that change, and the arithmetic is worth reading
because it is not "one fixed". Two opaque functions LEFT (both deleted), and one arrived:
`Parser::cure_spelling`, which reads `Type::Reference` bare and must, because the whole
question it answers is *which spelling is this field declared in* — the marker, not the peel,
is what it tests. Net −2 opaque, +1 opaque, and a total down by one because `cure_spelling`
replaces two functions with one. The three sites the fix actually corrected — the field
rewrite, the `&` head gate, the pointer repoint — do not appear in either column: none of
them is a whole FUNCTION discriminating on a `Type`, they are arms inside larger bodies. That
is the B7i masking this screen already warns about, seen from the other side: the unit is the
function, so a defective arm inside a body that peels somewhere else is invisible here. The
count is a queue of predicates, not a census of the shapes a `τ?` can reach.

The loft#1321 attempt moved this row and then gave it back — the fix was reverted, so the
numbers are those of the tree without it. Worth one line for the shape of the movement: the
join predicates it added each tested a block's `result` against `Type::Void | Type::Null` to
decide whether the block yields a value at all, which the audit counts as a bare `Type` match
even though `Void` and `Null` are not shapes a `τ?` can wrap. A column read as a score rather
than a queue would have asked for four meaningless peels.

loft#1319 moves it again, to `684 | 330 | 5 | 349`, and every step is in the good direction:
two more functions see through the wrapper and one fewer is opaque. The CALLER half of the
screen is where that change is legible rather than inferred — `heap_def_nr`'s row goes from
`2 peeled / 10 bare` to `4 / 8`, which is exactly the two call sites in the native generator's
whole-record bind that now read `variables.tp(..).base().heap_def_nr()`. Unpeeled they
answered `None` for `vector<τ>?` and `S?`, the bind reached no copy lowering, and the default
(alias) stood — against `@FR-B-Copy`. `Parser::classify_vec_bind` took the same peel on the
parser side.

That is the same sentence D-bind-13 and loft#1143 already wrote for two other constructs, and
it is what makes this column a queue rather than a scoreboard: the sites it names keep turning
out to be siblings of ones already fixed. The interpreter's half of the same fix is again
invisible here — it is an arm inside `gen_set_first_at_tos`, not a function of its own — which
is the B7i masking from the other side, and the reason the guard for that half is behavioural.

loft#1291 moved one site OFF the opaque column — the first entry here that does.
`Type::is_amp_rebindable_heap` is the one home for *"is this a `&` parameter whose whole-value
write-back displaces a store?"*, and it asks `inner.base()` because a `&hash<T[k]>?` parameter is
rebindable exactly as its dense twin is: @FR-L-Null says the storage is the same, and it is the
storage that gets displaced. It replaced two `matches!` arms that named variants bare, in
`parser/mod.rs` and `scopes.rs` — the two sites that must agree about it, one minting the rebind
witness and the other using it.

loft#1281's refusal added a site on the SEEING-THROUGH side and left the opaque column
alone: `reject_rebound_heap_parameter_captures` asks whether a captured parameter is a heap
kind, and asks it of `tp.base()` — so a `vector<T>?` answers the same as a `vector<T>`, which
is what the question wants, since nullability has nothing to do with whether a rebind can
reach the caller. It is the counterpart of the loft#1286 note below: a question asked about a
TYPE need not add an opaque site if it peels, and peeling was the correct reading here rather
than a concession.

loft#1286's first fix added a site on the OPAQUE side that matched
`Type::RefVar(Type::Reference(..))` on the raw `typedef` to ask whether a callee's parameter
was a `&`. It did not survive: the fix that ships asks the interprocedural question instead
(`callee_param_reassigns` — does the callee REASSIGN it), which needs no wrapper match at
all. Worth recording as a shape rather than a count: a question asked about a TYPE tends to
add an opaque site, and the same question asked about BEHAVIOUR did not need one.

loft#1245 added `use_analysis::callref_captures` on the seeing-through side (the opaque
column unchanged): it asks whether a fn-ref CAPTURES by matching `Type::Function` through
`.base()`, because the same fn-ref reaches it as `fn(τ) -> ρ` and as `fn(τ) -> ρ?` and a
capture is a capture either way.

loft#1291 moved one site OFF the opaque column — the first entry here that does.
`Type::is_amp_rebindable_heap` is the one home for *"is this a `&` parameter whose whole-value
write-back displaces a store?"*, and it asks `inner.base()` because a `&hash<T[k]>?` parameter is
rebindable exactly as its dense twin is: @FR-L-Null says the storage is the same, and it is the
storage that gets displaced. It replaced two `matches!` arms that named variants bare, in
`parser/mod.rs` and `scopes.rs` — the two sites that must agree about it, one minting the rebind
witness and the other using it.

loft#1303 moved a second site off the opaque column and added its sibling already transparent —
the only entry so far to do both, and the reason is that it followed loft#1291's peel rather than
re-deriving one.  `assign_refvar_reference` materialises a `&` parameter's write-back source into
its own store, and it named `Type::Reference` bare; the keyed sibling it needed
(`assign_refvar_keyed`) would have been a second such site.  Both now ask `inner.base()`, which is
the peel `Type::is_amp_rebindable_heap` above already uses for the SAME question — what does this
`&` write-back displace — so a `&hash<T[k]>?` reaches the materialiser exactly as its dense twin
does.

loft#1254 added the empty-stub return classifier on the same side, and the opaque column again
did not move: it asks whether a stub's return is HANDLE-carried, peeling first for the same
reason — a stub declared `-> P?` needs the twelve-byte null exactly as `-> P` does, so the
question is about the storage and not about the wrapper.  Joining the two checkouts put the
row at 668 · 316 · 5 · 347: both sides had moved it, and neither side's number is right after
a join, so it is re-measured rather than added up.

Two moving checkouts, and the movements are independent.  loft#1200 added
`scopes::nullable_locals_that_displace` on the seeing-through side: it asks BOTH questions on
purpose — `Type::Optional` names the spelling it is looking for, and `.base()` peels it to ask
what the storage is.  loft#1204, loft#1207 and loft#1212 then REPAIRED sites out of the opaque
column, which is the movement this column exists to report.  loft#1246 moved both of the first two columns by +2 and left the opaque column where it was:
`uncomputable_default` and `implicit_checked_narrow` are new sites that ask the wrapper
question deliberately — each is the ONE home for a rule whose answer turns on nullability, so
naming `Type::Optional` there is the point rather than a spelling to peel.  loft#1254's
`uninitialised_native_value` is another: it states the two arms where a type's DEFAULT differs
from its NULL and delegates the rest, and its `Optional` arm is that decision, not a peel it
forgot.  loft#1249's `target_holds_null` is the sharpest of the three — it exists BECAUSE
`Type::Optional` on a write target means two things, so naming the variant there is the whole
function rather than a spelling it failed to peel.  loft#1227's `GroupAppends::report` is one the screen caught on BRAND-NEW code rather
than on the backlog: it matched its holder against `Type::Reference` bare, which reads a `Counter` local
and misses a `Counter?` one holding the same fields and the same groups.  Named on the first run
after the lint was written, so the blindness never shipped — `.base()`, and the opaque column
did not grow.

  loft#1229 is another: `parse_vector`
crosses from opaque to seeing-through, because a keyed literal reported its DESTINATION
variable's type whole — so a `hash<E[k]>?` destination gave the constructed literal the type
`Optional(Hash(…))`, and loft#1210's append gate read that construction as an un-discharged
nullable SOURCE and warned about correct code.  A constructed collection is never absent, so the
literal takes `.base()`.  This is the family shape the paragraph below describes, one more time:
the VECTOR branch three lines down had always built its type fresh, and only the keyed sibling
carried the destination's wrapper.

loft#1236 adds one opaque body — `box_nested_capture_attrs`, which asks `cell_struct_name`
whether a capture attribute is a boxable scalar and does not peel: a `τ?` scalar is not boxed
today and the question is about STORAGE, so the wrapper would answer the same either way.  Named
here rather than left to be re-derived, because the screen reports a count and not a reason.

loft#1209 is the largest single move the column has recorded, and it is a
CAPTURE rather than a lowering: `closure_attr_type` and both of `parse_var`'s capture sites asked
`is_collection_type` bare, so the storage half and the reading half of one capture disagreed
about whether a `vector<τ>?` is a collection — an internal compiler error on three lines of
ordinary source.  One notion, two spellings, decided in two files.

⚠ **The queue this column names is not "349 bodies to read".**  Every repair in it so far was
one member of a PREDICATE FAMILY peeled while its siblings were not — `is_keyed` (d1220a1b),
`is_collection`, `collection_element`, `keyed_field_kt`, `assign_var_nr`, and
`Store::collection_rec`, each fixed because a separate issue happened to route through it.
`is_collection` is the sharpest: it is literally `is_keyed(tp) || matches!(tp, Vector)`, so
peeling one arm left the union half-peeled and made `vector<τ>?` the one collection the
predicate denied.  So the readable queue is *which families are peeled in one member and not
their siblings* — a much shorter list, and enumerable.

`collection_rec` is the instance worth remembering, because it is not in this table at all: it
discriminates on a stored VALUE rather than on a `Type`, its header already said *"a missed site
is a SIGSEGV rather than a wrong answer"*, and all twenty of its call sites were in `vector.rs`
while the keyed family read its slots raw (loft#1213).  A family can be split across files that
this screen never compares.

(gated by `doc_hygiene::quality_optional_table_matches_the_audit`, the arrangement the `unspan`
and `spellings` tables have — it read 637 · 367 until the sibling checkout's four commits were
picked in, which added three opaque sites, then 640 · 266 until B6q's `parse_stored_default`
added one that asks through `.base()`, then 641 · 267 until the four picked in B6r moved it
again, then 642 · 270 until B6s peeled seven and merged one body away, then 644 · 281 until loft#1125
peeled the three sites that decided a nullable collection's LAYOUT, then 643 · 284 until B6v added
`data::holds_dbref`, which asks through `.base()` and so lands on the seeing-through side, then
644 · 285 until B6w added four, then 648 · 286 until B6y added `source_spelling`, and
649 · 291 until loft#1145 added `Variables::retype_would_be_refused` — opaque, and deliberately:
it answers *"is this a type CHANGE"* and treats a wrapper mismatch as one, which is the whole
question `decl_accepts` decides beneath it — and 650 · 291 until loft#1156 added
`collect_loop_body_sets`, which discriminates on `Value::Loop` rather than on a `Type` at all
and is counted opaque for want of a wrapper to see through, and 651 · 291 until the
`@FR-E-NullArg` walk gave `boolean_operator` the `Optional(Boolean)` test that definite-ises
`&&`/`||`'s right operand — it reads the wrapper's INNER type to decide, so it sees through — which reads
`Type::Enum(syn, true, …)` to answer with the `Optional` the author wrote, and so is a peel in
the OTHER direction: it sees through by construction.  loft#1190 then moved it to 658 · 297 with
`use_analysis::copy_allocates_nothing`'s inner walk, which asks *"does duplicating this allocate"*
of a value struct's fields: it discriminates through `.base()`, so it lands on the seeing-through
side and leaves the opaque column where it was, and loft#1183 to 658 · 298 by giving the native
function prologue the heap-return test that arms `FnRefBufGuard` — asked through `.base()`, so a
`τ?` heap return is read as the heap return it is.  loft#1185 then moved it to 659 · 300: the
fn-ref-parameter test asks through `.base()` too, and the native call site's heap-return test
does the same, so both land on the seeing-through side.  loft#1234 then moved it to 661 · 350 by
TRADING one entry for another: `substitute_template_body` left the list because the three
hand-spelled `Type` arms it used to carry — an enum one, a heap-ref one and a boolean one, each
answering what a `null` looks like at a parameter — were replaced by a single call to
`write_typed_null_in`, the one home for that question; and `ops::EmitCtx::emit_ref` joined it,
counted opaque because it does not peel a wrapper at all — it CONSTRUCTS a `Type::Reference` to
ask that same home what the heap null is.  The movement is therefore the audit reporting a
de-duplication rather than a new opaque site: the subset that drifted (its enum arm claimed the
struct-enum spelling and answered `255u8` for a DbRef-backed parameter, disagreeing with the
direct-call path) is gone, and what replaced it asks the keystone.  B6w's four were: `needs_nullable_wrap`
asks through `.base()` and sees through,
while `nullable_payload_struct`, `tuple_elem_tag_read` and `tuple_elem_tag_write` are opaque ON
PURPOSE — each discriminates on a type read out of the LAYOUT (`attr_type` of a stored tuple
attribute, or of the `Some` variant's `payload`), and a stored attribute is already the storage
spelling, so an `Optional` cannot reach them.  That is the distinction the opaque column is for:
The `(G-Mono)` walk (B7g) moved the KEYSTONE column rather than either of the other two:
`Type::map_children` and `Type::zip_children` are the SET and PAIR twins of
`Type::for_each_child`, so the two substituters and the unifier that used to hand-spell four
formers, four formers and one now descend through it.  Closing loft#1175 then took one back
(6 → 5) by deleting the refusal's own `any_node` helper, and added a seeing-through site in
its place — the movement is what the column is for, not the level.  That is the column to watch — a site
that derives from the keystone cannot be opaque to a wrapper the keystone knows about, so
moving a body from `opaque` to `keystone` closes the question for every future variant rather
than for `Optional` alone.  loft#1204 then moved it to 659 · 301 · 353: fixing
`link_shared_nullable_views` gave it the `Optional` arm it was missing, so the very body the
per-test unit was built to catch left the opaque column by being repaired.  B7j then moved it
to 659 · 302 · 352 the same way, by giving `collection_element` the peel its sibling
`is_keyed_collection` already had.  loft#1206 moved it once more, to 659 · 303 · 351, for the
third time by REPAIR rather than by addition: `assign_var_nr` decides whether a text `+=` gets
the variable it writes through, its own router already asked through `.base()`, and the
disagreement between the two was an internal compiler error on `n.t += "cd"` for a `text?`
field.  loft#1207 moved it twice more, to 659 · 305 · 349, for the fourth and fifth time by
REPAIR: `is_collection` and `keyed_field_kt`.  loft#1354 then moved it to 698 · 341 with `arm_moves_a_live_tuple_local`, which asks whether an `if` arm hands over a tuple carrying text: it reads the element types through `.base()`, so it sees through the wrapper and the opaque column is unchanged.

Those five repairs are worth reading as ONE finding rather than five, and the reading is
what the CALLER half exists to give.  `is_keyed`, `assign_var_nr`, `collection_element`,
`is_collection` and `keyed_field_kt` are the same predicate family — "which collection is
this?" — peeled at five different times, each because a separate issue happened to route
through it.  `is_collection` is the sharpest case: it is literally `is_keyed(tp) ||
matches!(tp, Vector)`, so when `is_keyed` gained its `.base()` in d1220a1b the union was
left half-peeled, and a `vector<τ>?` became the one collection it denied.  Its own doc
asserted the two predicates "differ by that one variant BY DESIGN" while they in fact
differed on two axes, and 6 of its 23 call sites had already grown a hand-peel at the call
site — which is the tell this column is for: callers working around a predicate one at a
time is what a half-applied peel looks like from outside.

So the queue this column names is not "351 bodies to read" but "which predicate families
have been peeled in one member and not its siblings" — a much shorter list, and one a
`sites` query can enumerate.  Five consecutive movements of this column have been a body
leaving it because it was wrong, which is the pattern the column is worth watching for.

a site is a finding when a `τ?` can arrive there, not merely because it does not peel —
every count here is a snapshot of two moving checkouts, so re-run the tool rather than
reading a number.)  It reproduces **15 of 15** hand answers written down before it was
built — `deps_mut` / `depend` / `with_deps` / `without_deps` / `renumber_frame_deps` /
`for_each_child` / `ret_dep_shape` / `ret_promo_base` see through; `heap_dep` / `is_dbref` /
`is_scalar` / `heap_def_nr` / `is_unknown` are opaque; `contains_def` descends; `is_heap_owned`
is not in the population at all, because it delegates rather than matching.

**370 is a list, not a queue — so the second measurement is the one that ranks it.**  The four
heap-shape verbs the caller table puts at the top (`heap_dep`, `is_dbref`, `heap_def_nr`,
`is_scalar` — 43 bare call sites between them) were instrumented INSIDE the verb: fire when the
argument is an `Optional` *and peeling would change the answer*, and name the caller off the
backtrace.  Over the 883-program `tests/scripts` corpus that is **four (verb, caller) pairs**:

| pair | files | verdict |
|---|---:|---|
| `is_dbref` ← `Parser::block_result` | 883 | the P236 branch-join question, asked bare — **peeled** |
| `heap_def_nr` ← `State::known_type` | 839 | the schema id an `OpReturn` records — **peeled** |
| `heap_dep` ← `Ownership::reassign_sites_of` | 4 | the ownership oracle's heap filter — **peeled** |
| `heap_dep` ← `protectable_ref_args` | 2 | already handled: that caller asks BOTH questions |

⚠ **The fourth row is a false positive by construction, and it is worth keeping visible:** the
probe measures the VERB, so a caller that asks `tp.heap_dep().is_none() && tp.base().heap_dep()
.is_none()` — which is exactly what `protectable_ref_args` does, with the comment saying why —
trips it on the first half.  A screen over a shared verb cannot see the caller's second question.

**What the three fixes actually change, each measured rather than argued.**

*The ownership oracle was blind to nullable locals.*  `reassign_sites_of` filters to heap-typed
vars because "only HEAP-typed vars can carry the over-free leak"; asked bare, every `τ?` local
fell out of that filter.  Two functions differing only by a nullability marker:

```
fn f_bare(c: boolean) -> integer { s = mk(1); s = mk(2); if c { s = mk(3); } s.x }
fn f_opt(c: boolean)  -> integer { s: S? = mk(1); s = mk(2); if c { s = mk(3); } s?.x }

before:  OWN fn=n_f_bare reassign v=1(s) prior=Owned rhs=Owned
         OWN fn=n_f_opt   ← no reassign row at all
after:   both report the same row
```

The over-free leak shape the oracle exists to name is `prior=Owned rhs=Join(...)`, so it could
never have reported one on a nullable local — and loft#1106 was an ownership defect on exactly
that shape.  An instrument blind to a class reports it green.

*A nullable heap return recorded no type at its `OpReturn`.*  `known_type` resolves a heap type
through `heap_def_nr` and otherwise falls back to a lookup BY NAME; nothing registers `"S?"`, so
the fallback answered `u16::MAX` and the one consumer — the execution-trace renderer — had no type
to decode the returned value with.  Visible in the corpus, which is better than the trace could
show it: three programs' disassembly went from a bare `Return(...)` to `Return(...) type=Item 78`,
`type=I877Cell 78`, `type=I882H2 78`.

*The branch join skipped every nullable return, and the corpus could not have said so.*
`block_result` asks `is_dbref(result)` to decide whether an `if`/`match` tail's arms share one
return slot (P236, whose comment says native otherwise "drops the if/else's value and returns the
typed null sentinel").  Peeling leaves the emitted IR of **all 883** corpus programs byte-identical
— so no corpus program has a nullable heap return whose tail this join could unify — and it changes
a hand-written `fn pick(c) -> S? { if c { S { x: 7 } } else { S { x: 9 } } }`: frame 40 → 24 bytes,
two work-refs → one, two `OpFreeRefIfDistinct` → one.  Values were right both ways on both
backends, so this is a slot, not an answer.  **The finding under the finding is that zero**, and
`tests/scripts/a-nullable-return-joins-its-branch-arms.loft` now pins the shape.

**Both filings came back FIXED in the sibling checkout the same day, and re-measuring the pick
here is what mattered — loft#1118 arrived half-cured.**  Their four commits are on this branch
(`12fc2454`…`2df4a99a`); #1117 answers correctly on all four of my repros, and #1118's loop cell
went clean.  But the single-evaluation cell did not, and eleven more contexts with it: the lift's
admission predicate reads the `ncc` block's FIRST statement, and a REUSED `__ncc_N` opens its
block with its own overwrite `OpFreeRef`, which shifts the `Set` to second.  So the lift fired
only where the temp was fresh — a `for` body — and leaked one record per evaluation everywhere
else.  Skipping a leading FREE (not any statement: that would re-admit the `t[p] ?? dflt()` cell
their narrowing was measured to exclude) takes the matrix from 1 of 13 clean to 12 of 13, values
unchanged, and the release fuzz sweep — the 54-cell both-backends replay that falsified three of
their candidate narrowings — passes.  The thirteenth was filed as **loft#1119** — a DISCARDED call statement inside a
loop body — and the sibling's fix for it says my filed diagnosis was wrong in an instructive
way: neither the loop nor the discard decides it.  `Ownership`'s in-flight var set crossed the
caller/callee boundary, where a slot number names a different variable, so a callee's own temp
read as self-referential and the oracle answered `Join { base: MAX }` — no nameable witness —
for a value that has one.  Which caller's temp collides is a numbering accident, which is why
the symptom looked like "loops".

⚠ **The two fixes are orthogonal, and the A/B is the reason both are here.**  With only their
slot-scoping fix, ELEVEN of my eighteen cells still leak; with only my leading-free fix, the
loop-discard cells leak; with both, all eighteen are clean.  So the leading-free half is not a
symptom patch on their cause — it is a second, independent one, and either alone reads as a
complete cure on the half of the matrix it covers.  `tests/scripts/1118b-…` carries the A/B in
its header so the next reader does not have to redo it.

**And the lock earned its place on its first run — loft#1118.**  `make ci` failed on it: one
`SNRet` record leaked.  Not from anything in this thread — the same cell leaks identically on the
control at `81b42f3a` — but because the file is the first corpus program to hand a VARIABLE to a
nullable parameter and use the result **without binding it**.  The cell isolates to three facts
that must meet: the result is used inline, the parameter is nullable, and the callee mints on the
taken arm.  One record per evaluation, `SN×6` in a six-iteration loop, values right on both
backends.  The mechanism is loft#879's inline-`ncc` lift, whose `dep.is_empty()` guard refuses a
`Join` return (it carries a dep on the parameter it may borrow) — and the carve-out is the map
again: that comment already says an unlifted block "leaves the subject's store owned by nothing
when the block is used INLINE — one leaked record per evaluation, unbounded in a loop".  Filed
rather than fixed, because the cure is *lift, but bind through loft#1106's runtime guard*, and that
guard requires an `Optional`-typed temp and a `Value::Call` — the lifted temp is declared
`ref(SN)` and what it would lift is the `?.`'s BLOCK.  The lock keeps the bound spelling and names
the issue for the inline one.

⚠ **Two instruments were blind to it, in the same direction.**  `--tests` does not leak-check —
only `tests/wrap.rs` does — so the guard passed six-for-six on both backends while leaking.  And
`falsify.sh` reads its leak column off the run's stderr, which means **for a `main`-less guard (the
corpus's standard shape) that column can never fire**: it scored `0|0|none|none` on both trees for
the file `make ci` then failed.  A leak guard written in the normal form is therefore recorded
INERT — mislabelled a lock — and B6n's INERT residue is a quarter of the corpus.  The warning is
now in `falsify.sh`'s header where the next reader hits it; the cure is a leak check on `--tests`,
which is a decision about every library's `loft test`, not a tweak.

**Found on the way — loft#1117.**  The enum cell of the branch-join matrix does not compile at
all: `if c { E::A { … } } else { E::B { … } }` is refused with *"expected A, got B on else"*, while
`match k { 0 => E::A { … }, _ => E::B { … } }` — which lowers to nested `Value::If` — accepts it,
and so does an early `return` plus a tail.  `formal/types.md` `(C-Var)` settles which is right
(`Reference(S) ⤳ Enum(E) ⟸ S ∈ variants(E)`), so the refusal is the deviation.  **Fixed in the
sibling checkout and picked in here (`1cc265fe`), as a JOIN rather than a conversion** — and the
half worth knowing is the second one: without it `v: A = if c { E::A { … } } else { E::B { … } }`
is *accepted* and a slot declared as one variant holds another, loft#980's class.  Filed rather
than fixed at the time: the else arm is checked against the THEN arm's type, and pushing the expected type into the
arms is a bidirectional-checking change in the typing core.  Nothing to do with `Optional` — the
non-null twin fails identically.

**One peel deliberately did NOT go in.**  `is_protectable_store_type` is bare `is_dbref` while the
caller two lines up asks the peeled question, and that function's own doc says to keep the two in
step — so peeling looks like the obvious fourth fix.  It cures nothing measured (loft#1118's
mechanism is elsewhere) and it is **not inert**: it changes emitted code in six corpus programs,
every one a guard for this machinery (1021, 1029, 1105, 1106, 1107, 882), in the direction where a
mistake is a use-after-free rather than a leak.  Left alone, with the map written at the site.

⚠ **Three limits, all lower-bound in the same direction.**  A body that peels ANYWHERE reads as
seeing, even where a second match in it stays bare (B6f's caveat, one type former over);
`.base()` is also `use_analysis::Class::base`, a different method sharing the spelling; and the
corpus ranking is only as wide as the four verbs instrumented — `is_equal`, `content`, `show` and
`unrewritten` have 126 bare call sites between them and were not measured.  So **370 is a floor
and four is a floor**.

#### B6q — the axis a matrix pinned, derived instead of declared (2026-08-27)

B6m's ranked list put *"record the axes a matrix HELD FIXED, in the guard file"* third, and
called it cheaper than an instrument. The premise does not survive contact with the repo's own
register. `formal/ownership.md` D-own-6 wrote its pinned axis into its closing paragraph and the
next four defects still came from moving it, which is what that entry says in one line:

> An axis named in a closure is not an axis measured by it.

A DECLARED axis list is only as good as the author's awareness, and awareness is the thing that
failed. So the form that can work is a DERIVED one: a fixed vocabulary of axes, each carrying the
DOMAIN of values the language offers, applied to the guard file by a tool. The domain comes from
the language rather than from the author's list, which is the only reason it can name an axis
nobody considered.

`./scripts/matrix_axes.py` is that tool. Eight axes, each with a citation to a defect that
actually moved it — container kind (loft#1104), container provenance (loft#1105), argument
spelling (D-own-6), statement context (loft#1118), nullability (loft#1106), `??` default shape,
element type (`formal/tuples.md`'s all-`(integer, integer)` oracle), evaluation count. It is a
vocabulary of things that have bitten, not a taxonomy invented up front.

**It was scored against hand answers written before it existed, and the scoring is what found its
bugs.** The oracle is guards later WIDENED: the axis added between a guard's first commit and
today IS the answer. Six of six reproduced — 1104's `{hash, sorted, index, spatial}` (where B6i
later found `pick(h[k], …)` leaking at every keyed kind), 1104's missing `coalesce-result` (which
is loft#1105), 1105's provenance at one of four (the axis B6m names as the one that hid the
over-free), and 1118b's missing `discarded` (which is loft#1119). Getting there took two detector
fixes, both of the same kind — a shape the tool could not spell reading as a shape the file does
not have. `t += (f(x))` has a GROUPING paren, and treating any preceding `(` as an argument list
hid seven of loft#1118's eight statement contexts; and blanking string bodies erased the code
inside an interpolation, so a call written there vanished. An eight-context sweep read as three.

⚠ **The tool's own ranking claim was falsified by that oracle, and the tool now says so.** The
first design ranked files by how many values of an axis they reach — a file reaching several and
stopping short being an author who was enumerating and ran out of ideas, a file reaching one
never having claimed to sweep. loft#1105's killer axis sits at ONE of four. Reaching one value is
not a point test; it is exactly what a pinned axis looks like. There is therefore **no
corpus-wide queue**, because nothing measured supports one — *every* file in the corpus leaves
some axis short (892 of 892), which is a thermometer nobody reads (§ B4). Two measurements
replace it: the per-file census, to run while writing a guard, and the PAIR cross, which is the
sharper one because every failure B6m counted was a matrix that moved one axis and pinned
another. A pair, not a value.

`cross A1 A3` over the corpus reads (file-level co-occurrence, so an UPPER bound on real
interaction — a small number can only be smaller in truth):

```
                literal   local   field element tuple-el coalesce call-res   chain
vector              435     428     166      86       44       47       87      68
hash                126     122      83      57       12       21       23      47
sorted               60      53      41      22        9       11       12      18
index                51      45      36      19        9        8       11      14
spatial              14      12      10       8        4        3        5       7
tuple               149     148      46      22       66       10       28      27
```

The thin row is `spatial`, which 20 of 892 guards reach at all, and its thinnest cells are 3–8
files. Re-run the tool rather than reading these numbers.

⚠ **This table is the tool's SECOND answer, and the first one was wrong in the direction that
would have been quoted.** It originally reported `spatial × tuple-element` and `spatial × chain`
at ZERO — never crossed — and a zero is exactly the finding one writes down. The cause was the
screen's own predicate, which is B6e's lesson recurring: `_classify_arg` returned ONE label per
argument, so `sp[x, y].t.0` — an element access AND a tuple projection AND a chain — was
whichever test ran first. The corpus looked as though no guard ever reached a tuple element
through a container. **What caught it was writing the guard for the zero and finding the tool
still said zero**, which is a check available for free whenever an instrument reports an absence:
construct the thing it says does not exist and ask again. The classifier now answers with the
SET of spellings an argument contains, because a coverage question asks what is present; every
one of the six hand answers is unchanged by the fix, which is what says the correction did not
buy its zeros back somewhere else.

**Probing the thinnest cells found a defect with no store-lifetime in it: a tuple-typed struct
field cannot carry a default.** `t: (text, text) = ("a", "b")` is refused with *"Expect token
}"* — punctuation, for a composition the language supports on both halves. The boundary is exact
and it makes the case: integer, text, float, boolean, nullable, `vector<T>`, `hash<T[k]>`,
`sorted<T[k]>` and a struct field all take a default; tuple is the only type former that does
not, and the same tuple takes a default as a LOCAL and as a function PARAMETER.

Neither half is in doubt. Tuple struct fields are supported — Plan-06 phase 4d lifted the
restriction and TUPLES.md § Non-goals lists named tuple fields, single-element tuples, tuple
iteration, whole-tuple formatting and variadic tuples, not this. Declared field defaults are
advertised: loft#914's `omitted-field-zero` advice names them as the cure that already exists.
Only the composition was missing.

The cause is the shape this whole thread is about. `parse_field` reaches a field's type down two
branches — one for a type written as an IDENTIFIER, one for a type written with a leading `(` —
and the `= expr` shorthand lived in the first. Plan-06 4d added the second branch for the TYPE,
beside a sibling carrying a capability, and did not inherit it. `parse_stored_default` is now the
one home both call.

**Asking what ELSE that branch could not reach found a second capability, which is the point of
asking.** The identifier branch gets a field `assert(...)` by falling back into `parse_field`'s
loop; the tuple branch ends the field with a `break` and never arrives. So `t: (integer, integer)
assert(t.0 > 0)` was refused too, for the same structural reason and with the same message.
`parse_field_assert` is its one home. Both now parse AND fire: a constraint that is accepted is
not one that is enforced, so each was violated on purpose and each refuses with *"field
constraint failed"*.

**The gates, because a refactor's claim is byte-identity and a feature's claim is values.** The
emitted IR + bytecode of **879** corpus programs is byte-identical between the control at
`94bbd860` and here; of 880 compared the only one that moves is the tuple-default guard, which
the control cannot parse. The coverage guard added alongside it does NOT move, which is the same
fact its `INERT` control record states, arrived at by a different instrument.
That comparison was falsified two ways before being trusted — it fires on the new shape (0 lines
vs 2995) and on a one-character change to an ordinary field's default — and its first run was a
FALSE 878-of-878 caused by an asymmetric path normalisation, each binary resolving `default/`
beside itself.

⚠ **That gate had a hole exactly where a parser change is most dangerous, and it is now
closed.** `introspect` emits nothing for a program that does not compile, so the **45
`@EXPECT_ERROR` fixtures** — the refusal corpus, which is precisely the population a parsing
change moves — were counted as "no output" on both trees and compared for nothing. A byte
comparison of their `--interpret` stderr says **45 compared, 0 moved**, and that comparison
fires on a one-token edit to a fixture. So the corpus is covered twice over: 879 programs by
emitted IR, 45 by the diagnostic they refuse with. **A file the instrument cannot read is not a
file that agrees** — it was reported in the same "no-output" bucket as a genuinely empty run.

`tests/scripts/a-tuple-field-takes-a-default.loft` carries the cells, which are the REPLAY axes
rather than the filed shape: a default is lowered once in the struct's context, which has no
frame, and replayed at every construction site, so what decides soundness is the element type,
whether the default needs a TEMPORARY (routed into `__dflt_*` by loft#698), whether it reads `$`,
and how many times it is replayed. Its `@falsified-at:` records `94bbd860 — interpret exit 1 ->
0, native exit 1 -> 0`, and says plainly that the channel which moved is EXIT, because the control
cannot parse the file at all. That says nothing about whether the VALUES are checked, so every
assertion was mutated in turn and each fails on the assert channel. **A guard falsified only on
exit is a guard whose values are unproven, and the record has to say which channel moved or it
reads as more than it is.**

**The thinnest pairs were then crossed, and they are clean.** 30 cells — five container kinds ×
chain / tuple-element / `??`-result × two container provenances, each rooted in a keyed lookup
and handed to a borrow-deciding call — pass on both backends with no leak, against a control
cell that fails. **The third axis is there because the census asked for it.** The first draft
read `A2 container provenance 1/4 — reaches local-literal`: every cell built its container in
the function that indexed it, which is precisely the axis loft#1105's matrix pinned and precisely
why an over-free hid there, a container dying with the frame being unable to witness a free that
outlives it. That is the instrument doing the job B6m asked for — naming a pinned axis to the
author who just pinned it, in a file written by someone who had spent the day reading about that
exact failure.

So the thin cells are a COVERAGE gap and not a defect, which is a result worth having rather
than a null one: `spatial × chain` and `spatial × tuple-element`, 7 and 4 files across the whole
corpus, now have a guard that states an expected value.
`tests/scripts/a-keyed-projection-witnesses-every-kind.loft` graduates them, and its control
record says `none — INERT`, measured rather than assumed (`make falsify` reports
`0|0|none|none` on both trees). Naming a channel there would be a false claim of regression
cover. Two things keep that verdict readable rather than the mislabelling B6p warns about: the
file has a `main`, so the leak column is the live one and not the blind `main`-less shape; and
six cells were mutated by moving their expected total by one, and all six fail on the assert
channel. **Writing the cells also caught one vacuous by construction** — the `??` cells first
reached a NON-null field, where the compiler elides the coalesce as redundant and says so, which
would have left five cells claiming a spelling the program no longer contained.

⚠ **What this does not establish.** The census reads SYNTAX, so every count is a floor: a value
reached through a `use`d library is invisible, and a file that reaches a value only in a
commented-out cell reads as never having considered it. The cross is co-occurrence, not
interaction. And the instrument found this defect the way B6m § 3 predicts instruments work — it
ranked the cells, and the defect came from a person reading one.

#### B6r — a text auto-merge produced a document asserting both a claim and its retraction (2026-08-27)

Four commits picked from the sibling checkout (`64b95b68`, `85bb936b`, `00e0e491`, `2542c527`).
Two needed a decision, and the second is a hazard worth naming because nothing in the gate can
see it.

**Which commits were missing was not answerable by `git cherry`.** It compares patch-ids, and a
pick that needed conflict resolution gets a new one, so it reported 12 missing when 4 were —
including commits whose added guard files were already sitting in the tree. Ancestry lies the
same way (§ *Validate branches by content, not ancestry*). What answered it was file-level
arithmetic on the tree diff: `src/data.rs` differed by 106 lines and the two candidate commits
added 62 and 44; `src/scopes.rs` by 114 against 36 + 76. The four newest accounted for every
differing file exactly, and nothing older did.

⚠ **`2542c527` rewrites a formal-register entry to RETRACT a claim, and the auto-merge kept
both halves.** D-own-13's second face had recorded *"binding `v[0]` to a local first does NOT
cure it — a witness gap is cured by a name, an ownership gap is not"* as its discriminator, and
that commit exists to say the discriminator was itself broken. Git merged the deletion of the
old paragraph and the insertion of the new one as independent hunks, leaving the retracted
claim standing beside its retraction and a `fn local` line orphaned outside its code fence.
It merged **cleanly** — no marker, no conflict, no failing gate. `check_doc_drift.sh` and
`doc_hygiene` both pass on a `formal/` entry that asserts a thing and its negation, because
neither reads for coherence.

**The general shape: a findings document is not mergeable the way code is.** Code has a
compiler that rejects two contradictory definitions; prose has nothing. A `formal/` entry that
supersedes an earlier reading is exactly the shape auto-merge mishandles, because a retraction
is a DELETION whose meaning depends on the text it deletes. So a pick that touches `formal/`
wants its region read, not its exit code checked — and this one was caught only by diffing the
result against the source branch afterwards and finding one hunk that should not have been
there.

**`00e0e491` conflicted only in a comment**, and the code below the markers was identical:
both checkouts derived the same leading-`OpFreeRef` skip independently. The two prose accounts
disagree about WHICH spellings the unskipped free hid, and each is right about the tree it was
measured on — whether a spelling reuses `__ncc_N` is a numbering property, which is the same
reason loft#1119's symptom looked like *"loops"* (B6p). Naming a set that does not hold still
is what both comments were doing, so the resolved comment states the rule and points at
`1118b`, which is the measurement.

All six guards — four ours, three theirs — pass on the merged tree, and the remaining
difference against their branch is exactly our own work: the `parse_field` extraction, B6p's
three `.base()` peels, and four guard files.

#### B6s — the floor B6p left, measured: 10 more verbs, 8 pairs, and 5 of them only a second entry point can see (2026-08-27)

B6p ranked the `Optional`-opacity list by instrumenting four heap-shape verbs inside the verb
and naming the caller off the corpus — four `(verb, caller)` pairs — and closed with the
limit stated plainly: *"the corpus ranking is only as wide as the four verbs instrumented …
so 370 is a floor and four is a floor."* This is that floor measured. The other **ten** opaque
`data.rs` verbs the caller table lists — `is_unknown` (82 bare sites), `content` (44), `show`
(38), `find_fn` (12), `unrewritten` (5), `borrow_deps` (5), `rewrap_deps` (4), `argument` (3),
`owned_elements` (3), `fmt` (2), **198 bare call sites** — were instrumented the same way and
swept over the 895-file corpus.

**`#[track_caller]`, not a backtrace.** Each verb fires only when the receiver is `Optional`
*and peeling would change the answer*, and `std::panic::Location::caller()` names the site
directly. The predicate is written per verb, which is the honest form: *"would peeling change
it"* is a different sentence for a verb returning a `Type` than for one returning a `String`.

**Eight `(verb, caller)` pairs — and the entry point decides five of them.**

| verb ← caller | corpus files | seen by |
|---|---:|---|
| `show` ← `Definition::header` (return type) | 53 | `LOFT_LOG=static` only |
| `argument` ← `Definition::header` (return type) | 53 | `LOFT_LOG=static` only |
| `show` ← `Function::show_code` (block result) | 51 | `LOFT_LOG=static` only |
| `show` / `argument` ← `Definition::header` (parameters) | 15 | `LOFT_LOG=static` only |
| `content` ← `Parser::parse_vector` | 12 | any run |
| `show` ← `Type::show` (tuple element) | 5 | `LOFT_LOG=static` only |
| `owned_elements` ← `scopes::tuple_owned_elem_frees` | 4 | any run |

⚠ **The first sweep found two pairs. The second found six more, and the only difference was
how the corpus was run** — `--tests` for the test functions a `main`-ful `--interpret` skips,
and `LOFT_LOG=static` to make the dump renderers run at all. `show` and `argument` are reached
on the DIAGNOSTIC path, which a passing program never takes, so a sweep that only runs
programs reports zero for them. That is [a guard's entry point decides what
runs](STABILITY_METHOD.md) applied to an instrument rather than to a guard, and it is the
cheapest correction available: the same binary, the same corpus, one environment variable.

**Three defects, and each was invisible in a different way.**

*A tuple element declared `S?` was never freed* — one record per evaluation, unbounded in a
loop, values correct throughout. `owned_elements` was a hand-spelled copy of `is_dbref`'s
list, whose own doc says *"call this function rather than restating it"*; asked bare it did
not recognise `Optional(Reference(S))`. **The corpus had four programs with a nullable tuple
element and none could see it**: every one pins the element to `text?`, `integer?` or a type
variable, and `text` is the one owning shape `tuple_owned_elem_frees` skips ON PURPOSE
(loft#1004). The store-backed cell is the one nobody wrote — [a pinned channel is not an
exercised one](STABILITY_METHOD.md), one axis over.

*Every `τ?` in an IR dump rendered as its lowercased `Debug` spelling.* `fn n_takes(p:SD?,
q:text?, r:vector<integer>?) -> SD["p"]?` read as `fn n_takes(p:optional(reference(710, deps
{ items: [] })), …)` — the struct by def NUMBER and the dep list by INDEX, in the file
CLAUDE.md's debugging policy sends you to read first. `Type::name`, the user-facing renderer,
grew its `Optional` arm in Plan-07 phase 6.1 with a comment saying exactly why; `Type::show`,
the dump renderer, did not. One notion, two renderers, one of them told.

*The element variable of a nullable collection literal was typed as an unrelated struct.*
`content()` answered `Unknown` for `Optional(Vector(τ))`, so `unique_elm_var`'s
`type_def_nr(Unknown)` resolved to def 0 and three corpus programs declared
`_elm_1: ref(i_parse_errors)` where the element is a `Nine09`, an `Inner`, an `Ent`. It is
the loft#666 shape — a variable table naming something impossible — and it had been passing
for as long as nullable collection literals have existed.

**Two duplications merged, both onto a home that already declared itself one.**
`Parser::keyed_type_id` and `Parser::keyed_known_type` are the same function, 40 lines apart
in two files, each spelling the five keyed kinds itself — and they carried DIFFERENT
nullability contracts, one saying *"peel the `?` before calling"* in its doc and the other
peeling nowhere. `keyed_type_id` now delegates. `owned_elements` asks `is_dbref`. And the two
`par` record walkers restated `owned_elements`' membership a third and fourth time, guarded by
a `debug_assert` that says *"hitting it would indicate `owned_elements` and the match above
are out of sync"* — absent from release builds and [absent from the ordinary debug build
too](STABILITY_METHOD.md). They ask the one list now, so there is nothing left to assert.

**A leak cured on the way, in `work_keyed`.** The accumulator a `??` builds for its default
took the target type WHOLE — and in a `??` the target type is the JOIN's, whose deps name the
holder the other arm reads. So the accumulator declared a borrow of something it does not
borrow, no free leg claimed it, and every keyed `h ?? […]` retained one store per evaluation
while the `vector` twin was clean. A hint's deps are not the value's: `Type::without_deps`
exists for exactly this and says so.

⚠ **One chain was BUILT, measured, and BACKED OUT, which is the finding worth keeping.**
`is_keyed` is the declared one home for *"is this a keyed collection"* and **all 24 of its
callers ask it bare**, so `h: hash<S[k]>? = [S { … }]` builds a `vector<S>` and is refused
against its own declared type. Peeling there is the one-home fix; it took four more peels
(`content`, `keyed_known_type`, `gen_keyed_null`) to get from the refusal through an
`unreachable!` and a wrong schema id to a program that compiles and answers correctly — on
`--interpret`. `--native` panics in `keys.rs` on a `u16::MAX` store number. **A refusal is
better than a backend divergence**, so the peel is not in; what IS in is the measurement,
written at `is_keyed` where the next reader meets the question. The three peels that stand on
their own stayed.

⚠ **And the probes for that chain found a defect pair with no `Optional` opacity in it at
all** — **loft#1120**, filed rather than fixed here because curing it wants one representation
decision.  **CLOSED since (2026-08-28), in the sibling checkout and now in this tree**: the
cure is one lowering (`Parser::collection_is_null` → `OpVectorIsNull`, which `??` now asks
instead of carrying a third list) plus widening `vector::is_absent_collection` so a DbRef
reaching no slot — the missed-lookup encoding — answers absent.  That closes all four rows of
the table below at once, including the `spatial` / `trie` omission and the `hash` / `index`
panic.  `formal/collections.md` D-col-null; guard
`tests/scripts/1120-one-null-question-for-a-collection.loft`.  The table stands as the
diagnosis it was:

| spelling | right about | wrong about |
|---|---|---|
| `??` — `OpConvBoolFromRef`, `rec != 0` | a collection LOOKUP miss (`vv[9]`) | a collection FIELD: the read yields a sub-reference whose `rec` is the HOLDER's record, so `b.c ?? d` on a null `vector<T>?` field answers the EMPTY FIELD and drops `d` |
| `== null` — `null_test`, `OpVectorIsNull` | a collection FIELD | a LOOKUP miss: `vv[9] == null` answers `false` |

`null_test`'s doc calls itself *"the ONE place that answers what is `τ`'s null"* and warns
that answering it elsewhere mints another spelling; `??` is that other spelling. **Delegating
is not the cure** — it trades one silent wrong answer for the other, which is how it was
measured: the delegation fixed the field cells and broke `116-default-fallback-operator` and
`85-ncc-literal-return-delivery`, both vector-lookup shapes. `??`'s list is also short by
`Radix` and `Trie`, so `spatial?` / `trie?` answer **0** for a collection holding elements,
and a null `hash?` / `index?` field PANICS. All measured on both backends; the whole matrix
and the reason a third list is not the answer are written at the arm.  The workaround is
`if b.c == null { d } else { … b.c? … }`, verified on both backends and both kinds; **binding
the field to a local first does NOT work** (the sub-reference travels with the value), which
is worth knowing because it is the natural first attempt.

⚠ **`?? []` cannot witness any of it, and that is why a corpus testing nullable collection
fields kept the bug**: an empty default is what the wrong answer looks like, so the cell
agrees with itself. Every cell of the replacement matrix hands `??` a default whose length
differs from the field's. My own first reading — *"the `vector` twin is the clean reference
route"* — was wrong for exactly this reason, and only a NON-EMPTY default separated them.

**Blast radius, measured rather than argued.** Emitted IR compared against a pristine
`HEAD` worktree over all 895 corpus programs, with the renderer change applied to BOTH sides
so the dump improvement could not mask a code change: **4 files differ**, and all four are the
intended ones — `1028` gains the `OpFreeRef` for its nullable tuple element, and `909`,
`909b`, `923` get their element variable's real type. Everything else is byte-identical.

#### B6t — the `⇐` channel has ten push sites and six admission lists, and `Type::Tuple` is in none of them (2026-08-28)

B6g's `spellings` screen left 33 sites resolving a projection by op name, and **17 of them had
never been read**.  This is the first of the 17 read.  It did not produce the defect the screen
predicted — it produced a different one, one TYPE FORMER up, which is B6m § 3 again: the
instrument makes the queue tractable and the person walking it finds something else.

⚠ **BEING ANSWERED ELSEWHERE — loft#1120 was NOT this branch's to take (2026-08-28).**  The
obvious next item after B6s is the defect pair it filed, and the sibling checkout
(`tuxedo-work-2026-08-25`) had an UNCOMMITTED fix for it — `src/parser/operators.rs`,
`src/vector.rs`, `formal/collections.md` and a new
`tests/scripts/1120-one-null-question-for-a-collection.loft`, all touched minutes before this
session started.  Their cure is one lowering (`collection_is_null` → `OpVectorIsNull`) plus
widening `is_absent_collection` so a MISSED lookup answers absent, which closes all four rows
of the filed table at once.  Recorded because the register alone did not say so: the issue was
open, unassigned, and reads as available work.  **`operators.rs` and `vector.rs` were left
untouched by this session for the same reason** — B6m § 5's sequencing point, paid rather than
restated.

**The finding: a tuple MEMBER is not parsed against the type its declaration names.**  Six
shapes fail in a DECLARED tuple local that the RETURN and ARGUMENT positions accept, and the
position axis is what makes them visible — a matrix over member-type × member-expression alone
would have read them as "tuples cannot do this".

| declared local | before | channel |
|---|---|---|
| `t: (Shape, integer) = (Shape::Circle { r: 7 }, 9)` | refused *"cannot change type from (Shape, integer) to (Circle, integer)"* | contradicts `@FR-C-Var` |
| `t: (Shape, integer) = (Dot, 9)` | refused *"bare variant 'Dot' has no type here"* | the target DID have an enum type |
| `t: (Shape?, integer) = (Shape::Circle { r: 7 }, 9)` | refused | — |
| `t: (float, integer) = (5, 9)` | refused | an ordinary numeric coercion |
| `t: (vector<integer>, integer) = ([], 9)` | **ICE** — *"Incorrect var `__ret_1[32]` versus 24"* | — |
| `t: (vector<integer>?, integer) = ([], 9)` | **ICE** | — |

**Two independent causes, and either one alone reads as the whole cure.**  Both were A/B'd on
one grid before either landed, which is the only reason the split is known:

1. **The `⇐` channel carried `fn(…)` alone into a tuple member.**
   `seeds_tuple_member_hint` admitted `Type::Function` and a `Type::Tuple` containing one,
   because loft#1067 held the channel back until a `fn(…)` in a tuple could be called back out
   of one.  Its doc named the bound and pointed at the wider question
   (*"this does not thread member types in general — loft#942/#943"*).  Widening it to every
   member with a KNOWN type cures the bare variant and BOTH ICEs, and nothing else.
2. **The literal was converted AFTER `change_var` retyped the variable.**  loft#1034 routed a
   declared tuple local through `convert` — the same function the return position uses — but
   placed it ~830 lines below the `change_var` that decides acceptance.  Acceptance is
   `decl_accepts`, which answers `(N-Decl)` (a `τ?` slot admits a `τ`) and nothing else, so a
   member needing a real coercion was refused before the conversion that says yes ever ran.
   Hoisting it directly above `change_var` cures the variant and float cells, and nothing else.

⚠ **The `!first_pass` gate on that conversion made it permanently unreachable for exactly the
programs it exists to accept.**  The site is reached in pass 1 only; a pass-1 refusal aborts
before pass 2, so the guarded branch never ran for a refused program.  loft#1034's own guard
passed regardless because `decl_accepts` already admitted `(text?, integer) ← (text, integer)`
— the fix needed only the COERCION, never the acceptance, so the ordering fault was invisible
to it.  Removing the gate ALONE changes nothing (measured); it only matters hoisted.

⚠ **The seeding widening invalidates the justification written above it, and the cell that
checks this was one the grid had PINNED.**  `seeding` clears the ambient expectation for the
duration of the tuple-literal parse, and the comment justifying that for member 0 ends
*"and only a `fn(…)`-typed member seeds"* — which is now false.  The hazard it guards against
is real and recorded (`115-snapshot-roundtrip` went from a text build to *"No matching operator
'&' on 'text' and 'integer'"* when the clear was made unconditional).  What still holds is the
other half: the clear fires only when the destination IS a tuple type.  Measured rather than
argued — a parenthesised WHOLE-tuple expression (`t: (integer, integer) = (mk())`), the same
call unparenthesised, an operator-expression member and a nested tuple all answer correctly on
both backends, and all four are in the guard.

**Blast radius: `make ci` 4474/4474, 35 skipped.**  Guard
`tests/scripts/a-tuple-member-is-parsed-against-its-declared-type.loft`, which keeps four
already-working cells (`(Sm?, …) = (null, …)`, `(i8, …)`, `(integer?, …)`, `(text?, …)`) as
controls so the widening is not scored on its own, and reads BOTH members of every cell —
a broken first member takes the second down with it, and a cell checking only `t.0` cannot see
half the defect.

##### The census the fix stands on: one channel, ten push sites, six admission lists

`seeds_lambda_hint`'s doc calls itself *"the one predicate behind every `⇐` push site that can
carry a `fn(…)`"*, and for the `fn(…)` question that is true.  The CHANNEL is shared, though,
and each site decides independently which OTHER types may use it:

| push site | admits |
|---|---|
| `control.rs` block tail / `return` | `enum_context ∥ is_collection ∥ interpolation_target ∥ seeds_lambda_hint` |
| `control.rs` call argument (×3 separate lists) | `seeds_collection_hint ∥ interpolation_target ∥ seeds_lambda_hint`; `matches!(Function)`; `enum_context` / `seeds_collection_hint` / `interpolation_target` as an if-chain |
| `definitions.rs` field / param default shorthand | `enum_context ∥ seeds_lambda_hint` |
| `definitions.rs` parameter default | `seeds_lambda_hint` |
| `objects.rs` struct-literal field value | `seeds_lambda_hint` |
| `vectors.rs` vector element | `seeds_lambda_hint` |
| `vectors.rs` tuple member | `seeds_tuple_member_hint` |
| `expressions.rs` nested tuple-place assign RHS | `seeds_lambda_hint` |

**`Type::Tuple` is admitted by none of them**, which is why the return and argument positions
still refuse `(Dot, 9)` and `([], 9)` after this fix.  The block-tail site is the one to read:
its comment says a type threads *"for the same reason"*, then *"for the third time for the same
reason"*, then *"for the FOURTH time for the same reason"* — four entries, each added by a
separate bug, and the general rule LOFT.md already states (*the expected type wherever there is
one*) never adopted.  That is [carve-out comment is a map](STABILITY_METHOD.md) with the map
drawn by the author: the phrase counts the hole's remaining occupants.

**Filed rather than fixed here, both with a verified `wa:clean` and both measured on the two
backends.**  **loft#1122** is the census above as a defect: `(Dot, 9)` refused in a `return`
and in an argument, and `([], 9)` in a `return` answering `t.1 == null` for a member declared
`integer` — plus one leaked `__tuple<vector<integer>,integer>` store — while `--native` will
not compile it.  **loft#1123** is a `--native`-only silent wrong answer found while measuring
the workaround for the first: a tuple returned with a PRESENT nullable heap member reads back
`(null, 0)`, both members lost.  Its axis is *nullable and present*, not the member's type
former — a struct reference and a struct-enum fail alike, a DENSE member is correct, and a
nullable member holding `null` is correct.

⚠ **The workaround for #1122 exists only because the declared-local half landed**, which is
the argument for having fixed that half rather than filing the pair together: *bind a declared
tuple local and return THAT* is a cure a user can apply today, and before this entry it was
not one.  ⚠ **And the obvious variant of it does NOT work** — binding the MEMBER first
(`p: W2? = W2 { … }; (p, 9)`) fails exactly as the literal does, which is what says #1123 is
about the tuple that reaches the `return` rather than about the member expression.  It is
worth knowing because it is the natural first attempt, and it is the discriminator that puts
#1123 in loft#1096's family — one notion, two type spellings, the rewrite between the passes
deciding which one a site sees — a position over.

#### B6u — `@FR-L-Null` walked as a lens: 13 sites, two questions, one defect and one filed (2026-08-28)

BUG_REVIEW.md's `2026-08` (3rd) cycle names the class — *rules the code does not represent* —
and says the work per rule is **evaluate the sites → de-duplicate onto one home → fix what the
disagreement was causing → cite last**.  This is the first rule walked that way.
`@FR-L-Null` was chosen because it is the most scattered rule the tree has (13 citation sites
across 8 files, `rule_tags.py dups`) and because 14 of the cycle's 27 bugs name null.

**The 13 sites are TWO questions, not one, and the split is the first product.**

| question | sites | state |
|---|---:|---|
| **the PEEL** — `Optional(τ)` occupies τ's storage, so `.base()` before asking a shape question | 9 | 8 peel; `is_keyed` is the documented LOCK |
| **the SENTINEL** — which reserved bit pattern absence is | 4 | consolidated as IR / store-init / narrow read+write twins |

Merging them would be the early-abstraction failure the checklist warns about: *"is this the
same storage?"* and *"what value means absent in it?"* are different sentences, and the second
has a legitimate three-way split (an IR op, a runtime store-init write, a narrow-width pair).

**The sentinel half is genuinely consolidated, measured rather than assumed.** `data::to_null`
keys on the `Type` variant and answers an `OpConv*FromNull`; `Stores::set_default_value_nullable`
keys on the content-type NUMBER and writes a raw value; `to_null`'s doc claims they produce the
same sentinels *"so a record built by a literal and one filled by a `#read` answer the same"*.
That is a claim, so it was tested: nine nullable field types (`integer`, `float`, `single`,
`boolean`, `character`, `text`, `i8`, `i16`, `i32`) × three routes (field OMITTED so store-init
writes it, an explicit `null` literal so the IR path picks the op, and a later `= null`
assignment) — **27 cells, all agreeing**.  A negative result, and worth the lines: the doc's
claim is now measured, and the next reader does not have to re-derive it.

**The peel half produced the defect, and the duplication was the cause.**  `convert`'s
Null→heap arm asked `matches!(should, Type::Vector(_, _))` — a hand-spelled variant list short
by all five KEYED kinds, and not peeling `Optional` either.  So `h: hash<S[k]>? = null` kept a
bare `Value::Null`, **which writes nothing**, and the scope-exit `OpFreeRef` read the untouched
bytes as store #0 and tried to free the STACK: `BUG (#306)`, *"a stack-record ref was treated as
an owned heap store"*.  Values stayed correct throughout, so only the FREE channel could see it
— `tests/wrap.rs` Part A2 (loft#920) is the gate that fails on it, which is why the guard is
`main`-ful rather than `#[test]`-shaped (`--tests` does not run that gate).

⚠ **The arm immediately BELOW the broken one records the identical fault, in the same words.**
loft#1065 fixed the struct-enum shape and its comment ends *"the scope-exit `OpFreeRef` then read
the untouched bytes as store #0 and tried to free the STACK … BUG #306"*.  Its collection sibling
was two lines up, listing `Vector` alone, and was not touched.  That is
[carve-out comment is a map](STABILITY_METHOD.md) and [audit the siblings of a fixed
rewrite](STABILITY_METHOD.md) arriving together: the comment naming the hole was written by the
person who fixed the neighbouring instance of it.

Cured by asking the one home that already exists — `vectors::is_collection`, whose doc already
says it is the `is_keyed` set plus `Vector` — instead of a sixth spelling of the list.  Verified
on both backends over `hash` / `sorted` / `spatial` / `trie` / `vector` absent, plus dense and
present-nullable controls so the sentinel write is not scored alone.

**Two things the walk found that are NOT this defect, both recorded rather than folded in:**

* **loft#1125** — a nullable `index<T[k]>?` fails type layout outright: `#left_1`, `#right_1`
  and `#color_1` all land at offset `0`, overlapping each other and the first real field, while
  the DENSE `index<T[k]>` of the same struct is fine.  A/B'd against a build with the fix
  reverted and the errors are byte-identical, so it is independent and pre-existing.  The
  nullable-index cell is therefore deliberately ABSENT from the guard, whose axes section says
  so — a cell there would lock #1125 rather than guard this.
* **The generic `OpConv*FromNull` loop does not peel** (`let Type::Reference(_, _) = *should`),
  which predicts the same bare-`Value::Null` fault for a nullable struct REFERENCE local.  It was
  probed — `x: RS? = null`, plus `text?` and `integer?` — and every cell is clean on the same
  harness that shows the keyed `BUG (#306)`, so the shape is covered elsewhere and the missing
  peel is LATENT.  Left alone deliberately: changing it would alter code with nothing to measure
  the change against, which is the trap the `is_keyed` lock below already documents.

#### B6v — `@FR-O-Proxy` walked: one predicate, two questions, and the spelling that belongs to only one (2026-08-28)

Next rule by `rule_tags.py dups` after `@FR-L-Null`, and the one with a **checkable
obligation already written into it** — *"a site that FREES on the proxy MUST also consult
@FR-O-Override"*.  Thirty-one sites read `tp.depend().is_empty()`.

**The split (step 2 of the walk).**  The sites ask two questions, and the whole result turns
on which:

* **LAYOUT** — *"does this value occupy a DbRef slot?"*  `data::is_dbref`.  A tuple correctly
  answers **no**: it is multi-slot and every transport path gives it its own channel — native's
  `next_into` rather than `next_dbref`, tuple ops rather than `OpPutRef`, per-element frees
  rather than one.  **Seventeen of the eighteen remaining `is_dbref` callers ask this**, read
  one by one, and all seventeen are right — including two that look like exceptions and are
  not: `scopes`'s return-source suppression excludes `Tuple` deliberately (*"TEXT and TUPLE
  returns keep their own, mature free paths"*), and `data::owned_elements` is already inside
  the tuple's decomposition, asking per element.
* **BORROW** — *"can this binding REACH a store someone else owns?"*  A tuple answers **yes**,
  through its elements.  `data::holds_dbref` is now the home.

**The defect is one site asking the borrow question with the layout predicate.**
`collections.rs`'s coroutine loop-variable arm binds the loop var as a borrow of the generator
so the consumer never emits a per-iteration free — its own comment says *"⚠ A short list here
does not skip a nicety — it inverts this arm"* — and it gated on `is_dbref`, which rejects
`Type::Tuple`.  Measured on a four-pull generator over `iterator<(integer, S)>`: the
generator's extensible **frame store took a whole-store free on every iteration**, four frees
of one live store, the values surviving only because the allocator handed the slot straight
back.  Only the exhaustion pull's free landed on a stale ref and raised `BUG (#306)`, which is
the channel `tests/wrap.rs` Part A2 gates on and the only one that moves — values and exit
status are identical either way.

**Two homes retired, in one `if` block.**  Under the gate sat a `match` rebuilding each of the
eight DbRef variants with the dep — a THIRD copy of a list the gate above had just been
de-duplicated onto, with an `other => other` fall-through that binds the unspellable type
unchanged while the arm reads as taken.  `Type::with_deps` is the declared home and its doc
already states how a tuple holds a dep (no list of its own; the deps spread to the elements
and `Type::depend` unions them back), so one call replaces the match and reaches nested tuples
without naming them.

⚠ **A short list is not the only way this hides — a NEGATED one is, and it defeated the
rule's own checker.**  `scripts/o_proxy_check.py` reported the obligation set clean while
`scopes::tuple_owned_elem_frees` freed a tuple element on empty element deps with no override
consult.  Its discrimination 1 reads `!tp.depend().is_empty()` as *"this asks whether it is a
borrow"* — true of a condition, false of an early-exit GUARD, where `if !…is_empty() {
continue; }` puts the free on the FALL-THROUGH and the site concludes ownership exactly as a
positive test would.  The check now classifies by what the guard falls through to, and bounds
the region by what the keyword actually exits (`continue` leaves the enclosing loop body,
`return` the function) — taking the rest of the function for both accused a loop that only
pushes to a list.  It fires on both forms and is clean with both vetoes present, proven by
removing each in turn.

**What the walk reports, not the verdict.**  It **converged**: every cell the fix moved was a
tuple spelling of one question, the controls (bare `S`, bare `vector`, a scalar tuple, plain
and captured collection iteration) never moved, and the sibling that asks the borrow question
with the layout predicate — `scopes.rs`'s loft#1029 argument-witness lift — was **probed and
held** on both backends.  One root, three homes retired (gate, attach, free site).  The rules
covered every cell: `@FR-O-Proxy` says the proxy needs the override, `@FR-Col-Store` says
which types reach a store, so nothing here was a design call.

**Three findings FILED rather than folded in** — each a different root, each reproducing
byte-identically on a build with this fix reverted:

* **loft#1130** — `yield [<keyed-collection literal>]` hands back a corrupted collection:
  `hash` counts words instead of records (`5n − 3` for a 5-field element) and loses every key
  lookup; `index` / `trie` / `spatial` report `len == 1` for three elements; `sorted` and
  `vector` survive.  Binding the identical literal to a local and yielding the name is correct
  in all twelve cells, which is what isolates the route.  It also revises a claim in
  `formal/IMPLEMENTATIONS.md` § The DbRef set — *"a `spatial` yield is correct anyway"* holds
  for the bound route that was probed, and `coroutine-yields-a-dbref-value.loft` passes for
  exactly that reason: every one of its generators binds first.
* **loft#1131** — iterating a captured `vector<(…)>` inside a closure reads no elements
  (silently `0`; SIGSEGV when the tuple holds a struct; `--native` cannot compile it).  The
  adjacent @PLN93 capture arm spells its own three-variant list, so this looked like the same
  root — until the control separated them: **a tuple of SCALARS fails too**, and it reaches no
  store, so no ownership story covers it.
* **loft#1132** — `--native` emits invalid Rust rather than refusing for a yield type with no
  transport channel (a tuple with a `text` or nested-tuple element, or a tuple yield in a loop
  body): `tuple_kinds` answers `None` and the selection falls into an `as i64` catch-all.  The
  clear `compile_error!` the struct/vector-in-loop case already gets is the fix shape.

* **loft#1134** — a tuple with a NULLABLE struct element, read by iterating a collection,
  comes back one field high: the first field answers with the second's value and the last
  reads uninitialised bytes (`n=111 tag=4294967200` where `n=11 tag=111` was stored).  The
  same element by INDEX is correct, and so is the same tuple as a LOCAL — which is the axis
  `a-nullable-tuple-element-owns-like-its-dense-twin.loft` never moves: it covers both
  positions, the absent case and four element types, and every one of its tuples is a local.
  Found while checking whether this walk's `Optional` peel changed behaviour for a nullable
  yield element.  It did not; the defect is older and has no generator in it.

**Four reds cleared that were not findings, all from the previous commit on this branch.**
`cargo fmt` and `cargo clippy` were both failing — an inserted function had landed BETWEEN
`to_string_compact`'s doc-comment and its `#[must_use]`, silently moving both onto the new
function, which the compiler warned about and nothing read.

The third was the gate's own subject: **`error_messages::baselines_are_locked_in` had been
failing since the diagnostics-in-cache commit**, whose whole claim is *"a cached run says what
an uncached one says"*.  Two things a normal run PRINTS did not travel, and both come off a
diagnostic's `fixes`, which that commit deliberately dropped:

* the once-per-run *"N diagnostics above suggest what to write instead"* note counts entries
  with non-empty `fixes`, so a warm run dropped the line;
* **every `fix` line under `--explain`** — two cold, none warm.  The justification for
  dropping them was *"`--explain` forces a cold parse"*, and `startup_cache.rs` has no
  `explain` awareness at all.  The claim was never true; nothing had measured it.

`fixes` now travel — kind, title, condition, edit and both catalogue handles.  An `Edit` is a
position into source and the bundle is invalidated whenever a source changes, so a replayed
edit points where it pointed.  Guarded by
`arc_e_program_cache::a_warm_run_renders_the_same_diagnostics_including_their_fixes`, which
compares the two runs' stderr as EQUALITY and separately asserts the cold run produced the
thing being compared — two empty stderrs are equal too.  Falsified by re-encoding zero fixes:
it fails naming the missing note line.

**The fourth red is the same commit's other half, and it is put back rather than fixed.**
That commit also made the program cache default-on EVERYWHERE, removing the two exemptions
(a Cargo invocation, a `target/` binary) on the argument that they were a proxy for
incomplete invalidation now that both keys fold in `binary_signature_tag`.  The argument is
right about rebuilds and incomplete about everything else: a warm load skips the PARSE, so
every parse-time effect has to be carried, and the placement decision for a
`placement = "remote"` library is not.  Measured cold-then-warm on one unchanged tree —
correct refusal, then *"native function not loaded"* — and the flip is what put the whole
test suite on that path, so `placement_remote::a_server_that_stops_answering_is_an_error_
not_a_hang` failed.

⚠ **The sibling checkout reached the same conclusion independently and got there first**:
`main` carries the exemptions, its head is *"…and the cache flip deferred"*, and **loft#1129**
is the open issue — *"the program-cache default cannot flip on until the warm path reproduces
every parse-time effect"*.  So this branch was carrying a decision main had already reversed,
and the fix is to match main.  The `binary_signature_tag` half is orthogonal and stays.

Two method notes from how long that took to see.  It presented as *"the native cdylib is
missing or stale"*, which reads as a build problem and sent me through a `cargo clean -p loft
--release` first — that DID fix a separate stale-artifact fault, which is the trap: a real
cure for the wrong cause.  What actually separated them was running the SAME tree twice and
watching cold pass and warm fail; and the reference build (this branch's own HEAD) showed the
identical cold/warm split, which is what said the defect was older than today's edits.

#### B6w — loft#1134: the route the report called broken was the only correct one (2026-08-28)

Filed off B6v as *"a nullable tuple element read by ITERATING a collection is one field high;
correct as a local and correct by index"*.  Every clause of that is true as an observation and
the conclusion drawn from it was backwards.

**The declared layout settles it, and it takes one command to ask.**  `LOFT_DUMP_TYPES=1`:

```
82:__tuple<integer,S?>[32/1]      _0:integer[0]   _1:__nullable<S>[8]
81:__nullable<S>::Some[24/8]      enum:byte[0]    payload:S[8]
79:S[16/8]                        a:integer[0]    b:integer[8]
```

So `S`'s fields live at tuple offset **16**, behind a discriminant at 8.  The `for` loop was
the ONE reader that went there — `OpGetField(t, 8, 78)`, then the tag, then
`OpGetField(x, 8, 79)`.  The write copied a dense `S` straight into offset 8, and the INDEXED
read projected offset 8 as a dense `S` as well: **two mistakes that cancel**, which is the
entire reason the index route looked right and the loop route looked broken.  Scoring the
routes against each other could only ever elect the majority — [[agreement-is-not-correctness]]
one level up, where the thing agreeing is not two backends but two routes through one program.

**And "one field high" was the milder half.**  With the discriminant aliased onto field `a`,
presence stopped being a fact and became a data byte:

| cell | before |
|---|---|
| `S { a: 0, b: 22 }`, PRESENT, read in the loop | **ABSENT** |
| a `float` first member (`1.5`) | **ABSENT** — the low byte of the payload is the tag |
| element written `null`, read across a call | **present**, a record of zeroes |
| `S { a: 11, b: 111 }` | `a=111`, `b=4294967200` (the filed cell) |

The zero-valued first field is the cell the whole fix turns on, and no route-vs-route
comparison proposes it: it only looks interesting once you know a TAG is supposed to be there.
`formal/types.md` names the property being lost — *a struct `S` as a `vector` element is the
tagged `__nullable<S>` (discriminant + payload; **no collision**)* — so this was a deviation
from a written rule, not a design call.

**Root: one notion, two spellings, and the tuple never bridged them.**  A member is PARSED
against the spelling the author writes (`S?` = `Optional(Reference(S))`) and STORED against the
layout spelling (`Enum(__nullable<S>)`).  `synth_nullable_struct_fields` rewrites a struct
FIELD's typedef from one to the other and skips synthetic hosts — *"tuples, fn-ref, and our own
`__nullable<T>` variants … so the rewrite never recurses into generated layouts"* — while
D-tup-6 (loft#1123, closed the day before) gave the tuple ELEMENT the tagged slot anyway.  The
layout moved and the writers did not.  That is [[audit-the-siblings-of-a-fixed-rewrite]]: the
sibling had the precondition written into its own ⚠ note (*"that makes a tuple ELEMENT a
`__nullable<S>` slot"*) and nothing swept the sites that write one.

**Five sites, two homes.**  `emit_nullable_slot_write` / `emit_nullable_slot_read` are the pair
— tag on the way in, tag on the way out, the discriminant spelled exactly as
`operators.rs::enum_null` spells it so a slot cannot be written by one and read by the other.
`tuple_elem_tag_write` / `tuple_elem_tag_read` select for the tuple positions, and they are the
only place holding BOTH the member's source type and the slot's stored type, which is what the
decision needs.  The three writers were the vector-element literal (`new_record`), the tuple
field/assignment writer (`emit_tuple_set_ops`) and reassignment through an index; the two
readers were the unbox (`v[i]`) and the tuple struct-field read.

⚠ **Fixing the write alone made two passing cells fail**, and that is the shape rather than a
setback: a keyed-collection element and a nested tuple were both dense-write + dense-read, so
correcting one half exposed the other.  A fix that moves a cell from *right by cancellation* to
*wrong* is not a regression to back out — it is the second site announcing itself.

**Three findings filed rather than folded in**, each A/B'd against the control build `make
falsify` had already produced at `2b3691a4`:

* **loft#1138** — an ABSENT nullable struct reads as PRESENT across a function boundary
  (argument or return).  `convert`'s `Enum(__nullable<S>) → Reference(S)` arm projects the
  payload sub-ref without consulting the discriminant, and a sub-ref into an absent slot is a
  valid `DbRef`.  **Not tuple-specific** — it reproduces for a `vector<S?>` element and a plain
  struct field, which is what makes it a separate root; in-place tests, local binds, declared
  `S?` locals and `??` are all correct.  Filed rather than folded because the cure changes the
  unwrap's IR spelling from a bare `OpGetField` to an `If`, and `tail_is_nullable_unwrap` keys
  on the bare shape while `unwrap_source_is_nullable` beside it already peels `Value::If` — the
  two have to move together, which is its own verification.
* **loft#1139** — `v += [f()]` is refused for a tuple with a nullable member while the dense
  twin is accepted, reporting a precision loss between `__tuple<integer,S?>` and
  `(integer, S?)` — two spellings of one type.  A legal program refused.
* the guard reads `1|1|none|none` -> `0|0|none|none` on BOTH backends, so the defect was shared
  semantics rather than a parity bug.

**Two process notes, both of them repeats.**

*A workaround written from plausibility is a wrong workaround.*  I filed #1139 saying *"bind
the call to a local first"* without running it; `t = mk(); v += [t];` is refused identically.
`.github/LABELS.md` says it outright — *"a wrong workaround is worse than `wa:none`"* — and the
`wa:` label is the one field a consumer triages on.  Both issues were re-measured and both
dropped to `wa:partial`; #1139's real workaround additionally answers WRONG on `main` until
this fix lands, which is worth saying in the issue rather than discovering downstream.

*The doc-comment insertion hazard bit again, one commit after it was cleared.*  B6v's own
"four reds" were `cargo fmt` and `cargo clippy` failing because an inserted function had landed
BETWEEN a doc-comment and its `#[must_use]`; anchoring this walk's helpers on
`pub(crate) fn emit_tuple_set_ops(` put them between that function's doc block and the function.
Clippy caught it (`doc_lazy_continuation`), which is the argument for running the two clippy
variants before `make ci` rather than after — but the durable fix is the one already recorded in
[[make-ci-not-find-problems-before-push]]: **anchor an insertion on the doc block, never on the
`fn` line**.  Repeating a recorded lesson the day after recording it says the note was filed
where it is read after the fact, not where the decision is made.

#### B6x — loft#1138: the tag was built on the way in and dropped on the way out (2026-08-28)

B6w gave a tagged slot a writer.  This is the other half, and it was filed as a separate root
for a reason that held up: it reproduces with no tuple in sight.

An ABSENT `S?` arrived at a callee, and came back from a `-> S?`, as a PRESENT record of zeroes.
`convert`'s `Enum(__nullable<S>) → Reference(S)` arm unwraps by sub-referencing the `Some`
payload and never reads the discriminant — and a sub-ref into an absent slot is a perfectly
valid `DbRef`, so absence had nowhere to go.  Every in-function position was already right
(tested in place, bound to a local, assigned to a declared `S?`, discharged with `??`), which is
exactly why it survived: the obvious cells all pass.

**The fix is one arm, and its narrowness is the whole design.**  It sits at `convert`'s
`Optional` peel, so only a NULLABLE target reads through the tag.  A DENSE `S` target keeps the
bare payload sub-ref, because two sites downstream recognise that unwrap BY SHAPE —
`tail_is_nullable_unwrap` (the #306 view-return materialise) and `new_record_field_op` — and an
`If` is not a `Value::Call`.  Splitting on the target keeps one spelling per question instead of
minting a third that both would have to learn.

⚠ **Two wrong causes before the right one, and the IR had said it in one line the whole time.**
The change broke loft#1105's leak gate (12 records, every value correct), and I explained it
twice from the mechanism instead of reading the diff:

1. *"The slot is read twice, so a `??` default builds twice."* REAL — `emit_nullable_slot_read`
   used its `slot_ref` for both the tag and the payload — and fixed, and **not the cause**: the
   leak was identical afterwards.
2. *"`view_root_slots` cannot walk the new shape."* Also wrong; it already walks `If`,
   `OpNullRefSentinel` and projection chains, and a `None` answer is FINE — it is what makes
   `scan_args` LIFT the argument, which is loft#1105's own cure.
3. The actual cause, visible in `diff` of the two IRs: `r(1):ref(S1105g)?` became
   `r(1):ref(S1105g)["v"]?` and `OpFreeRef(r(1))` vanished.  I had built the result type with
   `Deps::none()`.  The value is a VIEW into the container's store, so with no deps the @P290
   bracket saw nothing to protect, the argument was never lifted to a name, and the caller kept
   the callee's minted return unfreed.  `with_deps_of(src_tp)` is the fix.

The rule that would have saved two rounds: **a new IR shape in argument position inherits its
source's deps**, and the check is whether the `__lift_1` bind is still emitted.  Both wrong
causes were mechanism stories told from the code; the diff was one command away and decided it.

⚠ **And the guard reports nothing under either obvious invocation.**
`1105-an-unnameable-argument-borrow-witness.loft` has no `main`, so `--interpret` runs nothing,
and `--tests` does not leak-check at all — only `tests/wrap.rs` does.  Reproducing it standalone
meant appending a `main` that calls all eleven `test_*` functions.  That is
[[loft-tests-flag-skips-leak-gate]] and [[guard-entry-point-decides-what-runs]] meeting in one
file, and it is worth knowing before hunting a leak the suite reports and no hand-run does.

**A screen caught a real half-answer in the new code, which is what they are for.**
`is_repeatable_place` asks *"is this free to evaluate twice?"* and answered it for the CALL
spelling of a projection only, so `ir_walker_audit.py spellings` moved 38 · 5 → 39 · 5.  A
`TupleGet` is a projection too and is equally free to repeat; the arm is one line and the count
now reads 39 · 6 · 33.  The cost of the omission was only a needless stash, but it is the same
shape as B6g's finding and it was found by running the screen rather than by reasoning.

Guard: `tests/scripts/1138-an-absent-nullable-struct-stays-absent-across-a-call.loft`, falsified
`1|1|none|none` → `0|0|none|none` on both backends.  It carries the four in-function controls
and a PRESENT half in every position, because a fix that answered "absent" everywhere would pass
an absent-only cell list.

#### B6y — loft#1139: three sites derive one def, and the refusal was cheaper than half a fix (2026-08-28)

The third of B6w's findings, and the one that shows what a REFUSAL is worth.

`v += [f()]` was refused for `f() -> (integer, S?)` while the dense twin was accepted, reporting
a precision loss between `__tuple<integer,S?>` and `(integer, S?)` — two spellings of one type,
the class B6g named.  `unboxes_stored_tuple` compared members with `is_equal` alone, and
`Data::same_nullable_struct` already existed for exactly that pair, so the acceptance is one
condition.

**Shipping only that condition makes the program silently wrong**, which the sibling checkout
measured, reverted, and said so on the issue — the right call, and the reason the fix has a
guard rather than a one-liner.  Accepting the pair without the rest turns a program that will
not compile into one that answers `a=9` for `a=4` and `k=0` for `k=3`.

**The root is one mistake at three sites: a `__tuple<…>` def RE-DERIVED from element types read
in the STORED spelling.**  The def is NAMED by what the author wrote, so handing it a tagged
member mints `__tuple<__nullable<S>,integer>` — a different def, different offsets:

| site | what it did | what it does |
|---|---|---|
| `convert`'s unbox arm | passed `stored_tuple_elements(is_type)` | passes the DESTINATION elements, which re-derive the def the value actually lives in — and each member's declared type is also what tells the unboxer to read a tagged slot through its tag |
| `set_field_check`'s `tuple_elem_set` arm | built elems from `def(inner_tp).attributes()` | maps them through `source_spelling` first.  **This is the one that made the append wrong**: it wrote the scalar member at byte 16 while the read looked at 24 |
| `unboxes_stored_tuple` | `is_equal` only | also accepts `same_nullable_struct` |

`Parser::source_spelling` (stored `Enum(__nullable<S>)` → `Optional(Reference(S))`, identity for
everything else) is the shared home, and the read-side counterpart of `needs_nullable_wrap`.

**A positional bound cannot fix this, and knowing why is the transferable part.**  The first
attempt admitted the two spellings for the LAST member only, which is sound-looking and holds
exactly as long as nothing follows the tagged member — every member AFTER one is displaced.  So
the guard moves the tagged member through first, last, middle-of-three and BOTH ends of a
three-member tuple, and reads a member on each side of it.  Those four cells are the ones a
last-member rule passes and a correct fix earns.

⚠ **`make falsify` scores this file on the EXIT channel only, and the header says so.**  The
control REFUSES the program, so not one assertion runs there — the assert channel reads `0` on
both sides.  What the tool proves is rejected → accepted, which is the cheap half.  The VALUE
half was falsified by hand against the build that matters — acceptance WITHOUT the def fix,
the state the file exists to make impossible — where every append cell is wrong and every
declared-local cell passes.  A guard scored only on the parse would have called that build
fixed.  This is [[channel-captured-never-compared]] in the shape a REFUSAL takes: when the
control cannot run, the automated verdict is about admission and nothing else.

**Two-checkout note.**  This is the first defect today where the two streams' fixes were
genuinely coupled rather than merely adjacent: the sibling's acceptance change is unshippable
without this branch's def work, and the def work is unobservable without the acceptance.  It was
resolved by saying so early — who holds which issue, what each has measured, and what the other
must not build on — rather than by either side finishing alone.  The message that mattered was
the one reporting a NEGATIVE result (*"the declared local is still refused on my tree"*), which
is what told the other side its wrong-value cell was its own to own.

#### B6z — `@FR-O-Move` walked: two clauses, five hand-spelled `Vector` lists, and the clause nothing implemented (2026-08-28)

Third rule walked, next after `@FR-L-Null` and `@FR-O-Proxy` by `rule_tags.py dups` (7 sites).

**The split (step 2).**  `(O-Move)` is two sentences and the seven sites divide cleanly along
them, which is the whole result:

| clause | sites | state |
|---|---:|---|
| **the TRANSFER** — a returned store is the caller's; the callee must not free what it transfers | 4 | implemented, and correct |
| **the BORROW** — *"if the return borrows a parameter, the return type records it and the caller COPIES"* | 3 | implemented **for `Vector` and `Reference` only** |

**The second clause had no implementation for keyed collections at all**, and the shape is
the one B6u found: `block_result`'s heap-return delivery dispatches on `Text`, `Type::Vector`
and `Type::Reference`, so the five keyed `Type` variants matched no arm, `ref_return` never
ran for them, and nothing recorded that a returned keyed collection borrows a parameter.
`Def::returns_borrowed_view` reads an empty return-dep list as *owned*, so the keyed copy set
its `0x8000` source-free bit on a store the caller still held: **`fn id(x: hash<T[k]>) ->
hash<T[k]> { x }` freed the caller's collection, and every call after the first read it
empty** — both backends, no diagnostic (loft#1140).

`vectors::is_collection` is the declared one home for the store-backed set, and **five sites
spell a `Type::Vector`-only list beside it**: the promotion-pass guard, the write-back match,
`var_bound_to_branch`'s call site, `views_local`'s, and `fresh_owned_vector_deps`.  The last
three are correct — keyed returns never reach the ladder, so gating them on `Vector` is right
by construction — and that is a **negative result worth its lines**: the carve-out comments
there (*"Vector only: the record return reaches its own view repair earlier"*) read as if the
world were {vector, record}, and it is not, but the conclusion survives.

**The defect is where the two questions in one predicate came apart.**  The site setting the
source-free bit asked `is_struct_returning_call` — *is the RHS a call* — while its own comment
claimed *"a fresh-storage call"*.  A borrowing return satisfies the first and not the second.
That is `[[one-predicate-two-questions]]` again, and the cure was **not** a sibling predicate
this time: `use_analysis::call_return_frees_source` already answers exactly this question,
was written for this bit (loft#981/#982), and reads both the callee's return deps and whether
the site's @P290 bracket covers every ref argument.  The site simply did not consult it — and
emitted no bracket, so its licence did not hold either.  Fixed by doing both.

⚠ **Three edits changed nothing before one changed everything, and that is the transferable
part.**  Recording the return dep, then routing the caller's adopt-vs-copy gate, then wiring
in `call_return_frees_source` — each was a defensible reading, each left the behaviour
byte-identical, because none of them was read by the site that actually frees.  The IR diff
against a pristine `origin/main` worktree is what ended it: the emitted `OpReplaceKeyed(…,
32847)` names its own `0x8000` bit, and `32847` is a fact no amount of reading upstream
predicates was going to produce.  **Stop patching after the second no-op and go read what is
emitted.**

**A conservative fix and a precise one, and the difference is measurable.**  Answering only
the callee-side half (*never free a borrowing return's source*) closes the use-after-free but
leaks one store per call on the MINTING arm of a borrowing signature — measured ×1 → ×2
against `origin/main`.  Emitting the bracket instead lets the runtime decide, which is what
the borrow/owned split needs: a protected store is refused the free, a callee-minted one is
not.  Both were built and measured; the second ships.

**Two findings recorded rather than folded in.**  loft#1142 — a keyed return through a branch
join leaks the UNTAKEN arm's store, one per CALL, for an inline literal and for a local alike,
and identical on `origin/main` and on this build.  Its cells are **deliberately absent** from
the guard, whose header says so; the join coverage there mints nothing.  And the two entry
points — `block_result`'s tail dispatch and `parse_return` — are two spellings of one act, so
both new arms route through `is_keyed` rather than each carrying its own five-variant list;
folding them into one function is a byte-identical-IR refactor that does not belong in a
behaviour change.

**The guard needed a churn cell to be non-vacuous.**  A wrongly-freed store still reads
correctly until its slot is RECYCLED, so the first call always answers right — and a program
measuring several kinds in sequence reports whichever kind happens to get its slot reused and
calls the rest clean.  The first matrix built for this read `hash` broken and the other four
correct; each kind in its own file read all five broken.  Every loop in the guard now
allocates and drops a collection between the call and the assertion.

#### B7a — `@FR-Col-Store` walked: the residual a previous walk measured as harmless, and the shape its probe could not reach (2026-08-29)

Fourth rule walked, and the first where the walk started from **another walk's own record**
rather than from `dups`.  `IMPLEMENTATIONS.md` checklist #2 already carried the finding —
`Reference | Vector | Enum(_, true, _)` at 43 sites, *"⚠ the short list is a BUG source"* — and
its § The DbRef set closed 2 of them, cleared 34 by sentinel, and left one residual written
down:

> ⚠ `coroutine_layout::next_operands` is still short — six of the eight, no `Radix`/`Trie`.
> Probed: a `spatial` yield is correct anyway, so it does not bite.  The guard now carries a
> spatial cell to keep it that way.

**That record is accurate and the residual bit anyway**, which is the transferable half.  The
short list lives in `YieldSlot::classify`, which runs only on the MEMBERS of a yielded tuple.
A **bare** `spatial` yield never reaches it — so the probe and the hole never overlapped, and
the cell added to keep it honest could not have moved.  `yield (a_spatial, 7)` and
`yield (a_trie, 7)` were refused by `--native` for programs `--interpret` ran correctly.

> **"Probed and it does not bite" is a claim about the SHAPE probed.**  Recording a residual
> as harmless is worth doing; recording *which shape was probed* is what makes the record
> re-checkable, and is the line the guard header now carries.

Cured by asking `data::is_dbref` — the declared home, whose own doc had already predicted this
exact failure (*"a short list is not a compile error anywhere — it routes a handle down the
scalar path — so call this function rather than restating it"*).  Third site of one list: two
were folded on when the original bug was fixed, and `classify` is the residual they left.

**The guard carries all five keyed kinds, not the two that failed**, because the member set and
the bare-yield set are the same set — naming two would leave the next kind to be found the same
way.

⚠ **`make falsify` cannot score this guard, and the tool says so misleadingly.**  It runs the
file without `--tests`, and the file has no `main`, so its interpret row executes nothing and
reads `0 asserts` on BOTH trees — which the summary renders as *"INERT — the control and this
tree answer the same"* and then *"NOT falsified"*, while the native row says `falsified`.  That
interpret row is **vacuous, not agreement**.  Measured by hand against a real `d1220a1b`
worktree instead: `loft test --native` 12 passed → 12 FAILED.  A cached falsify control also
failed to load its `default/` library, producing a `1 failed` line that looked like a
measurement and was not — both traps are recorded in the guard's annotation.

**Two findings filed rather than folded in**, both pre-existing on `d1220a1b`:

* **loft#1148** — a `vector`-member and a keyed-member tuple yield in ONE program make
  `--native` emit invalid Rust for the WHOLE file (`E0425`).  Order-independent, two generators
  is enough, from a plain `main` as well as under `--tests`; six keyed generators are fine.
  This is why the guard has **no `vector` tuple-member cell** despite it being the natural
  control — it would lock that failure here.  Reading (not yet measured) says the cause is a
  **type-id mint**, not a channel: `t{N}` is `type_id_ref`'s rendering of a `known_type`, and
  those bindings are emitted by `output_init`, so `E0425` is a store-type variable referenced
  without its declaration.  That fits the whole-FILE blast radius, which a per-call-site cast
  fault would not have.
* **loft#1149** — a `--native` yield refusal naming a `Vec`-keyed collection emits a malformed
  `compile_error!`: the rendered name embeds quoted key names (`spatial<P,["x", "y"]>`), so the
  quote ends the literal and the comma becomes a second macro argument.  `trie` escaped it only
  because its type carries ONE `String` key.  `refusal_text`'s doc states the false premise
  (*"contains no quote or backslash"*) — true of the template, false of the name spliced into
  it.  Fixed by the sibling checkout.

**A drift found by reading and not yet measured**, recorded so it is not lost: `output_init`'s
`field_keyed` set detects a field-referenced keyed type with `Parts::Sorted | Hash | Index` —
three of the five — while the `bare_io` arms directly below handle all five.  A `spatial` or
`trie` FIELD would therefore be emitted both inline and in the bare stream, which the comment
there says swaps the container's field source-order.  Whether it bites is unmeasured; the
asymmetry is real either way and is the same rule drifting at the `Parts` level rather than the
`Type` level.

#### B7b — the residual measured, and the silent-wrong the measurement led to (2026-08-29)

Fifth rule walked, and the second to start from another walk's own record.  B7a closed with a
drift found by READING and explicitly not measured:

> `output_init`'s `field_keyed` set detects a field-referenced keyed type with
> `Parts::Sorted | Hash | Index` — three of the five — while the `bare_io` arms directly below
> handle all five.  A `spatial` or `trie` FIELD would therefore be emitted both inline and in
> the bare stream, which the comment there says swaps the container's field source-order.
> Whether it bites is unmeasured.

**The asymmetry is real and the predicted consequence is false.**  A one-struct probe emits
`let trie_look = db.trie(t78, "nm")` and `let t80 = db.trie(t78, "nm")` two lines apart, so the
double emission is exactly as described.  It swaps nothing: every `db.*` collection constructor
is **interned by the type NAME it builds**, so the second call returns the existing id and the
FIRST position wins.  Only `db.index` has a side effect (the `#left_N` / `#right_N` / `#color_N`
triple it appends to the element struct) and it dedups early for precisely that reason.

> **What makes a second emission safe is NAME AGREEMENT, not the exclusion.**  That reframing is
> the whole of this walk: the guard everyone reads is a partial one, and the invariant under it
> was never written down.

So the question became *do the two spellings agree?*, and the probe for that is a collection
whose keys are not where the naive reader looks — a synth `__nullable<S>` element, which keeps
S's keys inside the `Some` payload.  Building one found a defect that has nothing to do with the
emitter.

**A keyed view beside a `vector<S?>` was silently a SECOND, independent collection.**  Records
put in through the vector were missing from the view; `len` answered 0 and a lookup answered
null, both legal values.  Both backends agreed, so a differential oracle could not see it.

| view kind | beside `vector<S>` | beside `vector<S?>` |
|---|---|---|
| `hash` | ✅ | ✅ |
| `sorted` / `index` / `trie` / `spatial` | ✅ | ❌ empty view, no diagnostic |

**The carve-out comment was the map.**  `link_shared_nullable_hash` rewrote a `Type::Hash`
element to the sibling's `__nullable<S>` and said so:

> *(Sorted/Index sharing is left dense — no consumer exercises it and it needs the index
> bookkeeping on the `Some` variant.)*

`Trie` / `Radix` are not even named — they post-date it (loft#927).  Of the two reasons given,
one was a scope note and one was a real mechanism, and they cover different kinds: `sorted`
needed **nothing** beyond the rewrite, and `index` needed exactly what the sentence said.

**One question, six homes.**  *Which field list do a keyed collection's key NUMBERS index?*
`Stores::key_owner` is the declared home — a synth `__nullable<S>` answers the `Some` payload,
everything else answers itself.

| site | asked `key_owner`? | what the short spelling cost |
|---|---|---|
| `Stores::hash` | ✅ (inline) | — |
| `Stores::create_key` (sorted, index) | ✅ | — |
| `typedef::key_bearing_def` (the DEF-level twin) | ✅ | — |
| `Stores::field_name` (spatial, trie) | ❌ | `trie`/`spatial` over a `__nullable<S>` element REFUSED: *"`nm` is not a field of `__nullable<W>`"* |
| `Stores::key_name` (the `sorted` → `ordered` group rename) | ❌ | the promoted type was named `ordered<__nullable<W>[]>` — an empty key list |
| `generation::bare_field_name` (the bare `init()` stream) | ❌ | `db.sorted(t78, &[("?", true)])` — a name nothing else uses, so it MINTED a second type and `verify_schema_ids` reported loft#739's drift |

The last row is the one the walk was chasing: the double emission and the short key rendering are
harmless on their own and a wrong program together.

**And the RB-link offset had three spellings.**  `Stores::fields`, `Stores::find_index` and
`Stores::build_index_sorted_vec` each recomputed `8 + fields[left_field].position`; the two
copies read the ENUM's own field list, so `index` over a nullable element aborted on a corrupt
reference (`fld=65539` = `u16::MAX + RB_RIGHT`) even after its element type matched.  Both now
call `fields`, which resolves through the same `index_owner` the append uses — so where the
bookkeeping is WRITTEN and where the tree DESCENDS from cannot drift apart.

**A second silent-wrong, found by permuting an axis the corpus never moved.**  Group formation
asked *"is the field being ADDED keyed?"*, so a plain `vector<E>` declared AFTER its keyed sibling
formed no group at all — `{ look: sorted<E[k]>, data: vector<E> }` built two collections where
`{ data: vector<E>, look: sorted<E[k]> }` built one, for all five kinds and in BOTH fill
directions, with nothing in either declaration saying which you had.  Same defect as the one-way
`others` link loft#843 fixed, one level up.  The test is now on the PAIR.

⚠ **An existing control pinned the wrong half of that, and the file it lives in had already
made the same mistake once.**  `901-linked-group-fill.loft` § c1 asserted *"a plain vector is not
auto-linked"* with a reason attached: *"the group forms off the KEYED field being declared, and
widening that to any collection would make two independent vectors alias."*  The hazard is real
and the cell does not test it — two plain VECTORS were never in the file.  What c1 actually
pinned was the ORDER dependency, and `make ci` failing on it is the only reason it was read at
all.

The file says the rest itself, about the control right below: *"c2 … used to read 0 here, and
this file pinned that as the scope — which was the defect, not the boundary"* (loft#927).  Two of
this file's three controls have now turned out to be pinned defects, and both were pinned with a
plausible sentence.

> **A control with a REASON attached is still only as good as the cell under it.**  c1's reason
> describes a widening nobody proposed — dropping the keyed requirement — while the change it
> blocked keeps that requirement and asks it of the PAIR.  The cure is the cell the reason was
> reaching for: c4 now pins two plain vectors staying independent, so the boundary is tested
> rather than asserted.

`c1` now reads 2 in both fill directions, `c1b` adds the reverse route, and `c4` is the boundary.
This is a **shipped-surface semantic change**: a program with `{ look: hash<E[k]>, data:
vector<E> }` that relied on the two being independent now has one record set.  The alternative is
worse — the same declaration written the other way round already means one record set, so leaving
it would keep two spellings of one declaration meaning different things.  `revalidate-libs` was
run locally over the published registry for exactly this reason.

**Guards.**  Two, because they are two contracts and a failure in one must not mask the other:
`a-keyed-view-joins-a-nullable-element-vector.loft` (all five kinds, both fill routes, the dense
twins, and two plain vectors as the negative control) and
`a-collection-group-does-not-depend-on-declaration-order.loft` (all five kinds × both orders).
Both `@falsified-at: 0785871f`, both on BOTH backends — `exit 1 → 0`.

**One finding filed rather than folded in — loft#1152.**  A vector VALUE assigned or appended to
a group member (`a.data = rows()`) reaches only that member; the sibling views stay empty.
`Stores::record_finish` is the per-record chokepoint that maintains a group, and
`vector_add` / `vector_replace` move records in bulk without reaching it.  Pre-existing on
`0785871f` (verified against the cached control build) and untouched by either fix.  The append
half is mechanical; the ASSIGN half has to clear and rebuild every sibling and free what they
held, which is an ownership design call, so the two halves are filed to be decided together.
Workaround verified: add the records element by element (`for r in rows() { a.data += [r]; }`).

#### B7c — the lint the walk asked for, and the sibling the walk itself did not audit (2026-08-29)

Two follow-ons to B7b, one requested and one self-inflicted.

**`advice[linked-group-apart]`.** Every bug in the @FR-Col-Group family — loft#843, loft#927 and
both of B7b's — has one signature: *the group formed, or did not, and nothing said so.*  A group
that did not form looks exactly like an empty one, so `len(view) == 0` is a legal value and the
first diagnosis anyone gets is a wrong answer.  The declaration is the only place the question is
decidable.

I had argued a declaration-site lint would be noise, and `keys.rs::linked_group_lint_enabled`
says so in as many words as the reason the double-fill advice speaks at the LITERAL instead.
**The owner's refinement is what made it viable: fire only when the members are declared APART.**
That reasoning holds for a group written TOGETHER — which is exactly what makes non-adjacency
informative.  The idiom is written as one thing with two views; a group nobody intended is two
fields added at different times for different reasons, and that is when unrelated fields end up
between them.

> **A carve-out's reason can be true of a narrower case than the carve-out covers.**  Second time
> in one day: `901`'s `c1` said "widening that to any collection would make two independent
> vectors alias" about a widening nobody proposed, and this one said "a declaration that forms a
> group is usually deliberate" about the adjacent case.  Both readings were right about what they
> described and wrong about what they were being used to block.

The quiet half is the design, so it is what the test file pins — `tests/group_apart_lint.rs` is
five silent cases behind one that fires, because an advice that fires on the idiom is one every
reader learns to ignore.  Owned source only: a consumer cannot rearrange a library's struct.

**And the sibling the walk did not audit.**  Extending the lint to struct-enum variants exposed
that B7b's own fix never reached them: `link_shared_nullable_views` ran from `parse_struct` only,
so **all five keyed kinds read 0 beside a `vector<S?>` in a variant — `hash` included**, meaning
that half was broken before the struct half was fixed.  `synth_nullable_struct_fields` and
`Stores::field` both handle `EnumValue`; only the parser site did not.

> That is [[audit-the-siblings-of-a-fixed-rewrite]] going unapplied on the walk that produced the
> lesson.  The question to have asked at the fix is not "did I fix this site" but **"what else
> holds fields?"** — and `Stores::field`'s own `Parts::Struct(_) | Parts::EnumValue(_, _)` match
> answers it in the same file I was editing.  The DECLARATION-ORDER half needed nothing, because
> it lives in `Stores::field` and inherited the variant arm for free; the split between the two
> halves' reach is the tell that one of them was written per-container and the other per-question.

A second-order trap in the same fix: the advice resolved its source position by attribute INDEX,
and a variant carries an implicit `enum` discriminator field the source never wrote — so the
lookup ran one field past the end and the advice was silently absent in variants while the
rewrite worked.  Positions are now resolved by field NAME.

**Two findings filed, both pre-existing on `0785871f` and both reached through the `is` binding:**

* **loft#1155** — binding a variant's keyed collection field with `is` or `match` LEAKS its
  store, one per call, for every keyed kind (`vector` is clean).  Both leaking spellings are the
  ones `warning[variant-field-unchecked]` recommends, and that warning gates a library's CI — so
  a library author with a keyed collection in a variant chooses between a leak and a red gate.
* **loft#1152, third shape** — a group member written through an `is` binding reaches that member
  and not its siblings (`a=2 b=0`), where the direct field write reaches both.  Not the
  binding-copies rule: the struct analogue `d = x.a; d = [...]` reads `a=0 d=2`, which IS the copy
  rule and is correct.  Recorded on the issue so whoever takes the assign/append halves checks it
  against the same fix.

#### B7d — `@FR-F-Spec` walked: a rules doc whose `OPEN: 0` was about its own genre (2026-08-29)

Sixth rule walked, and the first in `formatting.md` — ten rules, no code citation, `OPEN: 0`, so
it had never been walked at all. The walk found **four defects, two of them silent-wrong and one
a backend divergence**, plus two more it filed rather than fixed (loft#1165, loft#1166).

**The zero was never a measurement.** Its parenthetical said *"a rules doc — it shrinks
operational.md's D-op-1, adds no code deviation"*, which is a claim about the DOC'S GENRE, not
about the code. No oracle stood under it, so nothing could have moved it off zero. That is a
sharper version of the standing warning that an `OPEN: 0` is only as strong as its oracle: here
there was no oracle to be weak, and the line still read like a result.

**All four had a correct neighbour, which is what kept them invisible.** The conformance section
already warns that *"a differential oracle cannot see a flag both backends drop"*; every one of
these is the next case along — both backends agreed, and agreed on the wrong answer.

| defect | the neighbour that was right |
|---|---|
| `{a / b:<12}` right-aligns `null(/0)` | `{n:<12}` left-aligns a bare `null` |
| `{p:J}` / `{p:json}` are not `{p:j}` | `{p:j}` renders JSON correctly |
| `{n:0>5}` — a comparison reaches the width | every FLAG is already order-independent |
| `{n:e}` / `{n:j}` panic `"Unknown radix"` | the four radixes an integer has arms for |

The first is [[keystone-claim-is-a-measurement]] in miniature: six lines existed in THREE copies,
and a comment claimed the interpreter inlined them "to keep the hot path tight" — the inline body
was a `format!` and a call, the same work. Folding the two copies onto the shared function fixed
the bug and removed the place it could come back.

**The third is a carve-out comment naming its own residual, for the second time this month.**
`string_states` was rewritten to consume the flags in any order, and its note records the failure
exactly: an out-of-order flag *"was simply left in the stream for the WIDTH expression to find"*.
A `0` fill is left there for the identical reason — the fill branch can only claim a lexer TOKEN
and a digit lexes as an Integer — so `{n:0>5}`, how a reader coming from Rust spells zero-pad,
rendered unpadded on `--interpret` and reached rustc as `E0308 expected i64, found bool` on
`--native`. [[carve-out-comment-is-a-map]]: grep the carve-out, not the symptom.

**The rule wanted extending, not just enforcing.** F-Spec listed the flags and never mentioned
the FILL character at all, though it is implemented and is the whole reason a digit in that
position is ambiguous. An edge the rules cannot express is the rules asking to grow: `F-Spec-Fill`
now says fill comes first and cannot be a digit, and `F-Spec-Exec` says a part the renderer
cannot execute is refused rather than dropped — which is the L9 rule that was already written
beside the code, and had only ever been asked about `text` and `boolean`.

**A guard whose control fails by COMPILE ERROR scores its runtime cells vacuously.** The first
version of the guard pinned everything in one file; `make falsify` reported `1|0` on the control
— exit 1, **zero** assertion failures — because the `{p:J}` cells do not compile there, so the
program never ran and the alignment cell it was written for was never reached. Split in two, each
half fails on the control through its own channel: `1|1` for the runtime file, `1|0` for the
spelling one, and the second file says in its header why its assert count is 0 on purpose. This is
[[absent-warning-is-not-a-pass]] one level up — not the wrong channel, but a channel that cannot
speak because an earlier one already stopped the run.

**The side finding: the test harness dropped assertions.** Writing the refusal guard surfaced that
`@EXPECT_ERROR` bound to nothing in some positions. `parse_annotations` states one rule — *"any
pending annotations not followed by a fn → file-level"* — and had three sites implementing it, of
which only the EOF one did; the `struct`/`enum` and non-comment arms **cleared** instead.
Measured over `tests/scripts` + `tests/docs`: **7 annotations in 2 files were bound to nothing and
never checked**, six of them in `102b-pass1-expected-errors.loft` in front of the very
`struct integer` / `enum hash` declarations they describe. Falsified rather than assumed — the one
in `persist-bind-field-store-757.loft` was given a warning text that exists nowhere and the file
passed exactly as before; after the fix that same edit FAILS. All seven claims turn out to be
true, so nothing was hiding behind them; they are simply live now.

⚠ **A second shape from the same census is measured and NOT fixed.** An annotation in the file
HEADER is routed to file-level even when a `fn` follows it immediately, which contradicts the
binder's own comment (*"still binds the annotation to test_foo"*). It is uniform: **12 files have
exactly one such annotation each, always the first**, including `36-parse-errors.loft` (34 other
per-function annotations) and `102b` (15). Those annotations are not dead — they pass if ANY error
in the file matches — but they cannot detect that their own function stopped producing the error.
Fixing it makes 50 files strictly stricter and each red would need its own attribution check, so
it wants its own pass rather than a ride on this one.

#### B7e — `@FR-E-NullArg` walked: the rule that forbade what the language ships (2026-08-29)

Seventh rule walked, and the first in `operational.md` — 17 rules, two of them cited, and
`E-NullArg` itself uncited. The walk found **one silent-wrong that breaks a type-system
promise** (fixed here, D-op-6), **one rule that over-claimed** (fixed in the doc), and **one
misattributed diagnostic** (filed, loft#1169). A position sweep of the fix then turned up a
second root, unrelated to the rule and also filed (loft#1170).

**The finding: `&&` and `||` kept a null RIGHT operand.** C73 — the three-state boolean — says
`&&`/`||`/`!` coerce `null` to `false`, and the parser types the whole expression the non-null
`Type::Boolean` on the strength of it. But `true && maybe()` answered **`null`**, so

```loft
r: boolean = t && maybe_bool();   // compiles clean
r == null                         // true, on a variable declared `boolean`
```

and the same value reached a `boolean` STRUCT FIELD and a `vector<boolean>` element — non-null
storage holding the 255 sentinel — while `(t && maybe()) ?? true` discharged it to `true`, so a
defensive fallback answered the opposite of the decision. Both backends agreed throughout;
there was nothing for a differential oracle to see.

**One home, and the right operand never reached it.** The lowering is `a && b` → `if a { b }
else { false }`, so the LEFT operand becomes the `if` CONDITION and the jump coerces it
(`OpGotoFalse` tests `!= 1`), while the RIGHT operand becomes a branch VALUE that nothing
coerces. `convert` looks like the second home and is not: every *other* nullable type reaching a
boolean position picks up a real conversion (`integer?` gets `OpConvBoolFromInt`, whose
`!= i64::MIN` is already 0/1) — which is why `t && maybe_int()` was correct all along and is
kept as the control cell — but `boolean?` → `boolean` shares a base type and converts to
**nothing at all**. Fixed in `Parser::boolean_operator`, the one site that knows both operands
are truthiness positions, by wrapping a nullable-boolean right operand in `b == true`; that is
C73's own raw compare, it is parser-side so both backends inherit it from one IR change, and
short-circuit is untouched (measured with a counting right operand, not argued).

**The rule is what let it stand, and no oracle could have moved the register.** `(E-NullArg)`
named comparisons as the ONLY exception to contagion and never mentioned truthiness — so a `&&`
answering `null` read as the rule being *obeyed*, not as C73 being broken. The register said
`OPEN: 2` throughout. This is a sharper version of B7d's lesson: there, an `OPEN: 0` was only as
strong as its oracle; here it was only as strong as **the rules above it**, and no amount of
measuring would have found a deviation from a rule that described the wrong contract.
`(E-Truthy)` now names the positions that coerce, and is what the fix cites.

⚠ **The same rule over-claimed in the other direction.** Its ordering clause said null orders
low *"the SAME for `integer`, `character`, `float`, `single`, `boolean`"* — but `<` on two
booleans is REFUSED at compile time, deliberately: there is no `OpLtBool` and `Ord` lists
`integer`/`single`/`float`/`text`. Equality is uniform across all of them, ordering only across
the ordered ones. The existing uniformity guard covered float, single, integer and character —
four of the five types its own rule named — so the two it omitted were exactly the two the rule
got wrong. **A guard that carries a subset of the types its rule enumerates is where an
over-claim survives**; `boolean` and `text` cells were added to it.

⚠ **Twelve of the new guard's cells are BLIND to the bug it guards, and that is worth knowing
before writing the next one.** The natural spelling of a truth-table cell is
`assert(!(t && maybe()))` — and `!` is *itself* a coercing position, so `!null` is `true` and
every one of those cells passed on the broken build. Only `== false` / `== null` — the raw
compare — can see the sentinel. Each load-bearing group was then measured against the control
**separately**, because a failed assert stops the run and one falsified line says nothing about
the twenty after it.

The compiler was also making the claim out loud: `s.on == null` emits `redundant-null-check`,
*"'on' is 'not null', comparison is always false"* — beside a comparison that answered `true`.
A lint stating an invariant is a place to check that the invariant holds.

⚠ **The fix had a hole at one position, and only a POSITION SWEEP found it.** The first version
was gated on `!self.first_pass` — reflex, not reasoning — and a parameter default is parsed
**once, in pass 1**, so `fn f(b: boolean = t && maybe())` still answered `null` while a struct
field default, a return, a lambda body, a `for` body and a `while` condition were all fixed. The
matrix that found the bug could not have found this: it varied the OPERAND, and this varies where
the EXPRESSION sits. Sweeping the positions a construct can occupy is cheap and belongs in the
verification of any parser-side fix.

**And the sweep found a second, unrelated defect — loft#1170, filed.** A parameter default that
is a COMPOUND expression whose operand calls a function declared BELOW drops that operand:
`= 1 + late(0)` stores just `1` and `= true && late(0)` answers `false` where the truth is
`true`, both backends, no diagnostic, with the interpreter corrupting its stack on the way out.
That is `#1086`'s class one axis over — its hoist triggers on `unresolved_names`, and a
forward-declared CALL resolves its name while leaving its RETURN TYPE unlinked, so the identical
collapse happens with the counter reading zero. Two plausible detectors were measured and
rejected before filing (the default's own `dtype`, which `&&` overwrites with a concrete
`Type::Boolean` regardless of its operands; and `can_convert`, which is itself behind
`!first_pass`) — recorded in the issue so the next attempt does not re-spend them.

**Filed, not fixed — loft#1169.** A null that merely *passes through* a fault-prone op is
rendered as that fault: `{v[1]}` on a `vector<integer?>` whose element is genuinely null says
`null(oob)` with the index in range, and `{n / a}` with a null dividend and `a == 5` says
`null(/0)`. The tag is chosen at PARSE time from the op's shape and consumed at run time from
the VALUE, so the two facts that must meet — *this op could fault* and *this op did fault* — are
one and none. The runtime log is correct throughout, so `(E-Report)` holds; it is the render
path alone. Not fixed here because the missing fact lives in nine `#rust` bodies on the hot path
(`OpDivFloatNullable` is bare `@v1 / @v2` with no `s` in scope), and `src/parser/operators.rs`
already carries a deferred note pointing at the shape the fix probably wants.

#### B7f — the two the walk filed, closed (2026-08-29)

Both roots B7e filed were then fixed in the same session, and closing them turned up a third
thing plus one measurement that looked like a fourth and was not.

**loft#1170 — a parameter default dropped its forward-declared call.** `= 1 + late(0)` stored
the bare `1`; `= true && late(0)` answered `false` and left the interpreter a short stack that
SIGSEGV'd on the way out. Nine spellings collapsed (`+ - * / % == >` and both short-circuits),
both backends, silently.

The cause is #1086's exactly one axis over, and its own carve-out named the spot: that fix
hoists a default whenever pass 1 could not resolve something, and measures it with
`unresolved_names` — a count of identifiers that resolved to NOTHING. A forward-declared CALL
resolves its NAME (definitions are recorded before bodies are parsed); what is missing is the
RETURN TYPE, so `call_op_as` defers, returns `Unknown` **without building the operator**, and
leaves `code` as the bare left operand — with the counter reading zero the whole time.
`unresolved_types` is its sibling, incremented at the two sites where pass 1 actually gives up,
and the hoist is the cure that already existed.

⚠ **`&&` erases the evidence, which is why the obvious detector fails.** `handle_operator`
publishes `Type::Boolean` for a short-circuit whatever its operands did, so `dtype` — the
default's own type, which the type-check three lines below already reads — is concrete even
when an operand was never typed. That candidate was built and measured before being rejected;
it is recorded in the issue so the next attempt does not re-spend it. **A type published by
the construct is not evidence about its operands.**

**loft#1169 — a null that passed THROUGH a fault-prone op wore its name.** The tag is armed at
parse time from the op's SHAPE and consumed at run time from the VALUE, so *could fault* and
*did fault* were one fact and none: `{v[1]}` on a genuinely-null element read `null(oob)` with
the index in range, and `{n / a}` with `a == 5` read `null(/0)`. Cells A and B were then
indistinguishable, and so were C and D — **a tag that cannot be wrong is also carrying no
information**. `Stores::keep_format_fault_if` is the rule in one place and every fault-prone
`*Nullable` peer calls it with its own answer; the peers err toward CLEARING where the two
cases are not cheaply separable, because a missing tag is honest and a wrong one is not. The
filed issue judged this needs-design on the grounds that the fact "lives in nine `#rust` bodies
on the hot path" — true about the location, wrong about the cost: every one of those tests sits
on a branch the op already takes, and the peers are emitted only at guarded sites, never on the
common `v[i]` read. **A blocker written from the shape of a fix is a hypothesis** — see B6.

⚠ **The first version of this fix had the peers CLEAR the tag when they had not faulted, and
that broke a case the unfixed build got right.** Only the OUTERMOST op in a hole is armed, but
every fault-prone op in it runs — so a clearing peer erases a cause an INNER op just recorded.
`{v[0] / z}` — a genuine division by zero after a successful read — lost its `/0`. It was
caught by asking *"what did this build answer that the old one got right?"*, which is the
[[optimisation-guard-needs-a-control-cell]] question in a non-optimisation setting: **a fix
that removes wrong output needs a cell where output must SURVIVE**, or "removed the tag
entirely" passes every cell. Every inherited-null cell in the guard would have passed.
The shape it forced is better than the one it replaced: `OpTagFault` now only ARMS the hole,
`note_format_fault` is the single place a cause is written, and a peer that inherits a null
LEAVES the tag — so `{v[9] / 2}` reports the overrun that actually produced its null, which no
build before this one did. Arming is what confines it to format scope, since the same peers
serve a `??` discharge.

⚠ **And the state had to leave `Stores` for a reason no probe on `--interpret` could show.**
Written as `stores.note_format_fault(…)`, the fix passed every cell on the interpreter and
every hand-run `--native` probe, then failed `native_scripts` on ONE of 898 corpus programs
with `E0502`: the native emitter inlines an op's `#rust` body into whatever expression contains
it, so the body landed inside another `stores.` call's argument list —
`stores.enum_val(80, ({ … stores.note_format_fault(…) … }))` — one immutable borrow, one
mutable, both live. `fill.rs` emits each body as its own statement, so the interpreter can
never see it. The cause now lives in a thread-local in `ops` and the peers call free
functions, which borrow nothing and compose in any position; per-thread is also the right
scope, since a `par` worker renders its own strings.

**The lesson is about which corpus can see a class.** A `#rust` body is a fragment pasted into
positions the author never picks, so its blast radius is *every context the emitter can put it
in* — and the only instrument that enumerates those is the 898-program native compile. Two
hand-written `--native` probes and a three-guard suite all passed. For any `#rust` body that
gains a `stores.`/`s.` CALL, `cargo test --test native native_scripts` is the gate, not a
probe.

**A third defect, found by asserting the fix rather than the bug: `"hi"[9]` disagreed across
backends.** Writing the guard cell for a REAL text overrun turned up `null(oob)` on
`--interpret` and empty on `--native`. `(F-Render)` settles it in one line — a null character
renders as nothing, *so that iterating text past its end appends no garbage* — so the
interpreter's extra was the deviation, and `append_character` now drops the tag it still takes.
`D-fmt-4`, opened and closed. Nothing pinned it: the four `fmt43_*` cases are all integer holes.

⚠ **And one measurement that read exactly like a fourth defect and was correct behaviour.**
`5.0 / 0.0` renders `inf`, not `null(/0)`, which `(E-Uncomp)` — "op is `/`/`%` with v₂ = 0, the
result is null" — appears to forbid. It is deliberate: the float null IS the NaN, `inf` is a
representable value rather than a missing one, and loft#983 reverted forcing NaN because it made
one expression answer `inf` inline and `null` once bound, and made `a / b ?? 0.0` guard nothing.
I had already written "want null" into a probe and a guard comment before reading the note at
`OpDivFloat` that says all of this.

**That is the third false positive this family of rules has produced, and it is the same shape
as the defect the walk started from.** `(E-NullArg)` forbade what C73 ships; `(E-Uncomp)` forbade
what loft#983 decided. An incomplete rule does not merely fail to catch bugs — it MANUFACTURES
them, and each costs a probe, a hypothesis and very nearly a wrong fix. Both carve-outs are
written down now, each beside the rule it corrects —
[[incomplete-rules-doc-is-costlier-than-none]] for the earlier count.

#### C — process / skills

| item | state |
|---|---|
| a duplication trigger line in `engineering-rigor` + `loft-codegen` | ✅ done — `engineering-rigor` § *The second always-on sensor* (generic, beside *the tell*) and `loft-codegen` § *Before you add the arm* (with the project's three instruments). `engineering-rigor`'s DESCRIPTION carries it too, since that is what decides whether the skill is entered at all |
| `skill-creator`'s description-optimisation loop against `design-protocol` | ☐ offered, not run — triggering is the thing being fixed, so it is the one part worth measuring |
| `rule_tags.py` in a gate | ✅ done — `doc_hygiene::every_rule_citation_resolves` shells out to the same command a person runs, so gate and tool cannot drift. Proven to fire; skips (not fails) without `python3` |
| a tool for the axis a matrix HELD FIXED | ✅ done — `scripts/matrix_axes.py`, derived rather than declared (the declared form is falsified by D-own-6). `file <path>` censuses one guard against the language's own domains; `cross <A> <B>` names the value PAIRS no corpus file reaches, which is the shape every failure B6m counted actually had. Scored 6 of 6 against hand answers written before it was built, and that scoring found two detector bugs — a grouping paren read as an argument list, and `strip()` erasing the code inside an interpolation. Its depth ranking was falsified by its own oracle and removed. All REPORTS. See B6q |
| a tool for the DUPLICATION question over the IR tree | ✅ done — `scripts/ir_walker_audit.py`, seven modes. `walkers` counts who hand-rolls `Value`'s tree shape instead of deriving from the keystone; `producers` / `dead` intersect a construction screen with an 854-program corpus census to find variants nothing can build; `unspan` finds sites a `Span` hides a shape from; `reach` says which of them production actually runs (B6b); `spellings` asks the question one level up — who resolves a projection by OP NAME and so cannot see its `TupleGet` spelling (B6g); `optional` asks the same question over the TYPE former — who resolves a shape without peeling `τ?`, plus the caller-side `.base()` list (B6p). All REPORTS. Each was **scored against answers already found by hand before it shipped** — the first was rejected twice for failing to reproduce them, and `reach` went through three candidate call matchers on an 11-cell oracle — the `make profile-corpus` discipline, applied to a new instrument |
| the `optional` screen's UNIT — a function, where it should be a shape TEST | ✅ **done (B7i)** — scored 10/10 against a hand oracle written first, and the change found three faults in the detector itself plus loft#1204 on the first queue row read. The former note, kept because it is the measurement that motivated it: ⚠ **open, and measured**: the four sites B7h fixed all sit in its "see through the wrapper" bucket, because each function peels `Optional` somewhere else in its body. `handle_field` peels `td` and then matches `exp_tp` bare; `generate_set` peels for the keyed kinds and then matches `Reference`/`Enum` bare. So the screen's 354 opaque is a FLOOR over functions with no peel at all, and the class it exists to find hides in the 300. Splitting it per shape test is the change; the count it reports today is not wrong, it answers a narrower question than its name |
| a gate over the executable files under `doc/` | ✅ **a REPORT, not a gate** — `make doc-probes` (`scripts/doc_probe_sweep.sh`) runs all 857 and names the hard faults (B6o). It cannot gate: the files carry no expected values, and some fault on purpose. It found the 857 (not 877 — 20 were cache DIRECTORIES) and it scores crash channels only |
| the negative-control gate's LEAK channel | ⚠ **blind for the corpus's standard guard shape** — `falsify.sh` reads "stores not freed" off stderr, which only a `main`-ful `--interpret` run prints; `--tests` does not leak-check at all (that gate lives in `tests/wrap.rs`). So a leak guard written `main`-less scores INERT on both trees and is recorded as a LOCK. Measured on `a-nullable-return-joins-its-branch-arms.loft`, which `make ci` failed while falsify read `0|0|none|none` (B6p). Warning written into the tool's header; the cure — a leak check on `--tests` — is a decision about every library's `loft test` |

#### B7g — `@FR-G-Mono` walked: the declaration read the rule and the call did not (2026-08-29)

Picked because the bug review names **generic/monomorph** as the sharpest RISING class
(+7.0 pp, 13 of this cycle's issues against a 1.4 % peak), and because `formal/interfaces.md`
carried the same unmeasured `OPEN: 0` sentence `formatting.md` had — *"a rules doc … adds no
code deviation"*, a claim about the doc's GENRE with no oracle under it.  Two independent
signals at one doc.

**The disagreement, found by reading before any probe.** One question — *"relate a template
type and a concrete type"* — has five homes, and they carry four different lists of which
`Type` formers to descend:

| former | `for_each_child` (keystone) | `rewrite_type_opt` | `rewrite_unknown` | `substitute_type` ×2 | `resolve_type_var` | `extract_type_var` |
|---|---|---|---|---|---|---|
| Vector · Optional · Tuple | ✓ | ✓ | ✓ | ✓ | Vector only | Vector only |
| Iterator | ✓ | ✓ | · | ✓ | · | · |
| RefVar · Rewritten | ✓ | ✓ | ✓ | · | · | · |
| Function | ✓ | ✓ | · | · | · | · |

`Type::contains_def`'s own doc claims the GET side *"had drifted behind the SET side"* and names
Function among the children *"that `substitute_type` DOES rewrite"*.  It does not, and has never
— the GET side was derived from the keystone and the SET side never was, so the comment records
the repair of one half as if it were both.

**The defect the disagreement produced is a legal declaration no call can reach.**  The
DECLARATION-side check that a generic's first parameter carries the type variable is
`arguments[0].typedef.contains_def(tv_nr)` — keystone-derived, all seven formers.  The
CALL-side reads were the two narrowest rows above.  So `fn f<T>(x: T?, d: T)` is accepted where
it is written and reported as **`Unknown function f`** at every call, at every instantiating
type.  Same for `(T, T)`, `(T, integer)`, `iterator<T>`, `vector<T>?` and `fn(T) -> …`; and
`fn(T) -> T` in a LATER parameter was refused with *"expected `fn(T) -> T`, got
`fn(integer) -> integer`"* — the substitution the message itself asks for.

**Why no oracle saw it, measured rather than asserted.** Across `tests/scripts`, `tests/docs`,
`default/` and `doc/`, **166 generic declarations put a bare `T` or a `vector<T>` in the first
parameter and not one puts anything else** — exactly the two arms the descent knew.  The
implementation and its corpus were written against each other.  This register's own axis list
(TYPE for #1028, OPERATION for the `??` check, SPELLING for the write, RETURN TYPE for #1032,
ARGUMENT SPELLING for #1029) had never included *which FORMER the first parameter wears*, and
the `T?` guards are the sharpest illustration: every one of them writes
`fn g<T>(v: vector<T>, a: T? = null)`, putting the carrier first, so the file that exists to
test nullable type variables would not compile with the `T?` in front.

Closed by deriving all four from the keystone — `Type::map_children` (the SET twin) and
`Type::zip_children` (the PAIR twin, for a walk descending two type trees at once), both
exhaustive.  `extract_type_var`'s LEAF also became precise, a type-var placeholder rather than
any `Reference`, so `(P, T)` answers with `T` instead of with whichever the walk reached first.

**Unlocking a refused shape is where the walk earned its next three findings**, which is the
[[refusal-beats-backend-divergence]] rule paying out: every newly-reachable cell has to be run
on both backends, and three of them were not clean.

- **loft#1175 — CLOSED.** `fn(T) -> T` at `T = text` entered its callee one hidden `&text` work
  buffer short, because the count is read off the return type where the call is LOWERED and the
  return is still `T` there.  `--interpret` faulted on the corrupt frame, `--native` answered
  correctly.  Closed by DEFERRAL — the count is re-asked per monomorph, which is the cure this
  class already has (loft#1020's null test, loft#1028's null literal, loft#1032's yield channel).
  ⚠ The obvious cure was built and measured first: `fnref_text_buffers`' doc says its loose
  candidate test can only *"mint a buffer nothing uses, which the pop removes"*, so counting a
  parametric return as a text candidate looks free.  It cured `text` and made **all six other
  instantiations abort** — a non-text return has no `__retbuf` protocol for the pop to trim
  against.  The looseness is safe within the text family, not across its boundary; the site's
  own claim was the thing to falsify, and all six are cells in the guard for that reason.
  ⚠ **And the deferral's own first version diverged on the OTHER backend.** A buffer minted
  after the parse is not declared at the top level, so `scopes::check` scoped it to the argument
  block and freed it before the callee filled it: the interpreter stayed correct while
  `--native` emitted a `String` declared inside the block and an empty `OpCreateStack`, which
  does not compile.  The repair is a top-level `Set` hoist — a replay `patch_tret_callers`
  already performs, with its reason written at the site, two hundred lines from where I needed
  it.
- **loft#1177 — CLOSED, and it was two defects.** A lambda with a DECLARED `-> vector<…>`
  aborted the compiler: a lambda gets a return buffer from neither reservation path — the
  signature-time one excludes lambdas by name, and the between-passes one skipped a lambda whose
  return was declared rather than adopted — so pass 2 GREW `__vdb_1` and H5 reported the
  divergence.  The sentence justifying the skip, *"the signature-time path already served it"*,
  was never true of a lambda; it was true of the RETURNS that need no buffer, which is why a
  declared `-> P` and `-> E` were fine and only a collection was not.  **Not a generic defect at
  all** — the concrete twin ICEs identically, the loft#1029 lesson again.
  Reserving the buffer then exposed the second, and it is this walk's own class one more time:
  `scopes::callref_owned_return` decides whether a closure call hands back a store the caller
  must own, and its arms named `Reference` and record-`Enum` over a `_ => None` that reads as
  *"nothing else needs owning"*.  A store-backed collection contradicts that.  **A HASH return
  leaked the same way and always had**, which is what says the `_` was short by the whole
  collection family rather than by the former the issue is named for — found only because the
  vector fix made a sibling cell worth running.
- **loft#1176 — CLOSED, and it took its opposite down with it.** A monomorph whose tail is a
  FN-REF call leaked its returned struct when used inline: the arm loft#1066's fix does not
  reach, and that commit names it in advance — `monomorph_return_is_fresh` is a positive proof
  read off the body, and *"a `return` of a CALL is the callee's fact and answers false"*.
  Checking #1066's own repro first is what made this a sibling rather than a re-report.
  ⚠ **The obvious discriminator was built, measured, and does not discriminate.**
  `scopes::inline_struct_return` decides the same question for a `??` subject with *"only a
  CAPTURING fn-ref can hand back a store the caller's scope owns"* (loft#1114), reading the
  fn-ref type's own deps.  Applied here it answered *capture-free* for a capturing lambda and a
  minting one alike — because there the fn-ref is a LOCAL whose type was INFERRED at the bind,
  so its deps name the closure record, and here it is a PARAMETER whose type was DECLARED, and
  a declared fn-type carries no deps whatever is passed.  **The same predicate, sound in one
  position and inert in the other, distinguished by where the type came from.**
  The fact is not unreachable, only unreachable from INSIDE the callee: at the CALL SITE the
  caller named the closure it passed, so `fnref_target` resolves it and the target's own
  body-shaped proof decides.  Both ownership reads are required and neither is redundant —
  `returns_borrowed_view` catches a lambda handing back its own PARAMETER, and
  `monomorph_return_is_fresh` catches one handing back a CAPTURE, which the deps proxy calls
  owned because the dep names the hidden `__closure`.
  ⚠ **And the reference route was the one that was silently wrong.** Scoring the broken
  monomorph against the hand-written concrete twin — the [[reference-route-is-the-oracle]]
  move — is what found it: the twin lifts on the deps proxy ALONE, so a capture-returning
  closure had its record FREED, answering another value on the next iteration and garbage
  after the scope, on both backends.  The two routes were wrong in opposite directions, and
  the one this issue was filed against was the safe half.  So the resolution now gates BOTH,
  ahead of every signature-carried fact: a `-> P` says the same thing whether the closure
  mints, hands back the caller's argument, or hands back a capture.
- **loft#1179 — CLOSED, and it was the formal register's own open deviation.** A fn-ref call
  site allocates one store per hidden return attribute, because it cannot know which function
  the slot holds — and a callee that delivers its return some other way left it owned by
  nobody.  `formal/closures.md` D-clo-7 had named the mechanism a month earlier and left it
  open: *"a direct call site mints the return buffer as a caller LOCAL it frees at scope exit,
  while the fn-ref path has `fn_call_ref` allocate a store the rebinding body never adopts"*.
  Reading that sentence is what turned three separate-looking reports into one free in
  `State::fn_return`, keeping the buffer the callee handed back — identified by STORE, since a
  callee that delivered through it may answer a record inside it.
  `--native` never had it, which is where the shape of the cure came from: its dispatch passes
  the null sentinel for a Reference return and frees an unfilled `__vc_hbuf` for a vector one.
  ⚠ **Two of the three reports it was supposed to close are only half-closed, and the guard
  says which half.**  loft#1180's leak is gone and its SILENT WRONG is not — a lambda handing
  back a captured collection has its capture ADOPTED by the bind and released at scope exit,
  so the captured variable reads empty from the second call on, both backends.  That was
  filed as a leak because the probe called it INLINE; binding the result is what shows it, and
  [[print-inside-the-loop-is-vacuous]]'s lesson is the same one — the spelling you probe with
  decides which channel can move.  loft#1178's reservation is safe to widen on `--interpret`
  now and still refused, because `--native` cannot COMPILE the widened shape (the map desugar
  declares `var__map_result_1` inside the comprehension block and `ref_return` returns it from
  outside), and one backend accepting what the other refuses is worse than both refusing.
- **loft#1180 — CLOSED, and the report it started from was measuring the wrong channel.**  A
  lambda handing back a captured COLLECTION had its capture ADOPTED by the caller's bind and
  released at scope exit: the captured variable answered EMPTY from the second call onward, on
  both backends.  `fnref_result_type` drops a return-dep index naming no visible argument, on
  the grounds that *"the value arrives OWNED"* — true of a hidden work buffer, false of
  `__closure`, which is the caller's own record.  Third position for loft#1114's sentence.
  ⚠ **It was filed as a LEAK because the probe called the lambda INLINE.**  Nothing binds an
  inline result, so nothing adopts it, and the wrong answer cannot appear —
  [[print-inside-the-loop-is-vacuous]]'s lesson from the other side: the SPELLING a probe uses
  decides which channel can move, and a leak channel that moves is not evidence that the value
  channel is clean.  The cell that scores it binds, in a loop, and reads the capture back.
  ⚠ **And the repair had to be narrowed TWICE, both times against a measured cost.**  A
  dep-index test alone cannot separate `{ cap }` from `{ sr_make(k) }` — a fresh store built
  FROM a captured value carries the same out-of-range index — so restricting to a CAPTURING
  slot was not enough and the second restriction is the type former: a struct, record-enum or
  text return is materialised into a fresh copy before it leaves, so only a COLLECTION return
  hands the capture across.  Without that, eleven stores leak in
  `717-closure-struct-return.loft`, which is the guard that caught it.

⚠ **And one measurement that read as a fourth and was not.** The `T = struct` fn-ref cell first
looked like a pre-existing leak, because the "twin" beside it leaked too — but that twin applied
the function TWICE (`f(f(x))`) while the generic applied it once.  The one-application twin is
clean, so the leak is monomorph-only and belongs to #1176.  A twin that differs from its subject
in a second way is not a twin, and the leak channel cannot tell you which difference produced the
warning.

The `optional` table above moved on its KEYSTONE column for the first time (4 → 6): a body that
derives from `for_each_child` cannot be opaque to a wrapper the keystone knows about, so moving
a site from `opaque` to `keystone` closes the question for every future variant rather than for
`Optional` alone.

#### B7h — `@FR-L-Null-Tag` walked: the rule names its own home, and three writers were not in it (2026-08-30)

Picked because the bug review makes **ownership/free** the largest rising class (+6.5 pp, 34 of
this cycle's 161 issues) and because `formal/ownership.md` was the only register still carrying
an OPEN deviation with a live repro — `D-own-16`, *"a SELF-referential join never frees the
store it displaces"*.  Working that repro is what led to the rule this section is named for,
which is the ordinary shape of the walk: the filed cell is a door, not the room.

**The first probe was the filed program with its interesting feature REMOVED, and that was the
whole boundary.**  `D-own-16` is `c = mk(i) ?? c`, and its entry reads *"it is genuinely the
hard shape rather than an oversight"* — the borrow arm IS the variable being assigned, so a
pre-assignment free would be a use-after-free on the arm that takes it.  Deleting the `?? c`
leaks identically: nine stores in ten rounds for a plain `c = mk(i)` in a loop, values right
throughout, on both backends.  The join was never the axis, and the "measured and reverted"
experiment recorded under that entry could not have moved anything, because the shape never
reaches the witness machinery it was aimed at.

The axis is one former's nullable spelling, and the census says so cleanly:

| local's declared type | reassigned in a loop from a call |
|---|---|
| `S` · `E` (dense struct, dense record enum) | clean |
| **`S?` · `E?`** | **9 of 10 stores retained** |
| `vector<T>` · `vector<T>?` · `hash<K[k]>` · `hash<K[k]>?` · `text` · `text?` | clean |

Every other former is right in BOTH spellings because `Optional` is transparent to `depend()`
and to `is_keyed`; only a bare `matches!(tp(v), Type::Reference(_,_) | Type::Enum(_,true,_))`
was not, in the interpreter's `owned_ref` and in the native emitter's `owned_ref_reassign`.
Same fact, short by the same shape, on both backends — @FR-O-NoDiverge holding while
@FR-O-Owner did not.  Filed as **loft#1200**; `D-own-16` stays OPEN with its boundary corrected.

⚠ **The obvious cure was built, measured and REVERTED, and that is the finding.**  Peeling the
`?` in both shape tests fixes every leak cell on both backends — and is unsound, because the
empty dep list those tests stand on (@FR-O-Proxy) reads *owner* for at least three unrelated
kinds of borrow that a nullable `Reference` local can hold:

| slot | what it really holds | caught by |
|---|---|---|
| the `__lift_N` of an inline `f(x) != null` | the eval-stack record — a `-> S?` return is NOT delivered into a caller-owned buffer the way its dense twin is | `1085-ret-buffer-passthrough-free.loft` |
| a local a lambda CAPTURES | a slot shared with the closure record | `1114-…-capture-is-shared-…` |
| a local bound from a reflection builtin (`t = type_named(name)`) | a borrowed handle into a store the runtime owns | `pln127-reflect-consumer.loft` |

Every one of them was found by the **REFUSAL** channel (`BUG (#306)`), never by a value and
never by a leak: a widened free moves the channel a leak matrix is blind to, which is
[[a-leak-channel-cannot-score-an-overfree]] paying out three times in one afternoon.  Two
exclusions were added and the third kind arrived anyway — and three unrelated borrows reaching
one predicate is what says the predicate is the wrong PLACE, not that it needs a fourth
exclusion.  The fix belongs where the ownership fact is known (@FR-O-Oracle).  ⚠ `Vector` was
never in the peel either, and its carve-out comment says why: *"a nullable vector already
releases through its own path and widening that one would free twice"* — a warning about
exactly this, written beside the test, one former over.

**Then the same class turned up on the write side, and that one answers WRONG.**  `S?` is
`Optional(Reference(S))` at the type level but a tagged `__nullable<S>` in an inline slot, and
`(L-Null-Tag)` ends with the sentence a walk exists to test: *"every writer and reader of such a
slot goes through the tag; the pair that holds this is `emit_nullable_slot_write` /
`emit_nullable_slot_read`"*.  It was a description of ONE writer out of four.  Deciding to tag
needs the SOURCE's type, and the source has two spellings meaning one thing:

| writer | the source test it spells | sees `S?` |
|---|---|---|
| `mod.rs::needs_nullable_wrap` (the declared home; the tuple member asks it) | `match src_tp.base()` | yes |
| `objects.rs::handle_field` (struct field, construction AND assignment) | `let Type::Reference(src_d, _) = exp_tp` | **no** |
| `collections.rs` (`v[i] = expr`, field store) | `let Type::Reference(src_d, _) = src_tp` | **no** |
| `vectors.rs` (`v += [expr]`) | `let Type::Reference(s_d, _) = &t` | **no** |
| `operators.rs::wrap_dense_default_as_some` (`?? dflt`) | `rhs_type.base()` | yes |

So for every source a function RETURNS as `S?` or a local DECLARES as `S?`, the dense record
went into the tagged slot untagged.  Two faces, both silent, both backends byte-identical:

| destination \ source | literal | call `-> S` | call `-> S?` | local `S?` | local `S` |
|---|---|---|---|---|---|
| field, at construction · assigned · nested · an element's own field | ok | ok | **wrong** | **wrong** | ok |
| vector element, `+=` · `[i] =` | ok | ok | **wrong** | **wrong** | ok |
| **tuple member** | ok | ok | ok | ok | ok |

A present value landed one field low, so every read came back one field HIGH (`s.a` answered
`s.b`; the last field read off the end).  And a value the callee withheld at RUNTIME wrote
nothing at all — assigning a null into an occupied slot was a silent no-op that left the slot
reading PRESENT with its previous value.  With the discriminant aliased onto the payload's
first field, an ordinary `S { a: 0, … }` read back ABSENT.  Filed as **loft#1198**;
`D-layout-3` opened and closed.

**Why no oracle saw it, measured rather than asserted.**  The two dense columns are the half a
hand-written test can see, and the corpus writes literals.  The tuple row is the control that
names the cause rather than the symptom — it is the one writer that asks the shared predicate,
so it was right on all five sources while its three siblings were wrong on the same two.

⚠ **The three hand-rolled writers were not identical, and absorbing them without reading what
each DOES would have traded a wrong answer for a leak.**  Only `collections.rs` released the
payload the slot already held before overwriting it, with the reason written at the site; the
shared home cleared only on its ABSENT arm.  So the home gained the present-arm clear — exactly
one clear on either path, because `OpClearKeyed` reads the discriminant and a second one over a
slot still tagged `Some` would release the same claims twice.  That is the
[[deconflation-drops-a-half]] hazard from the merge side: the site that is about to disappear is
the one holding the fact nobody wrote down anywhere else.

⚠ **The `optional` screen reported all four sites as COMPLIANT, and finding that out is worth
more than the row it did not move.**  Its counts are unchanged by this walk (659 · 300 · 5 ·
354, before and after) because it classifies per FUNCTION: `handle_field`, `generate_set` and
`output_set_body` each peel `Optional` SOMEWHERE in their body, so all three sit in the
"see through the wrapper" bucket while a second shape test inside them stays bare.  A function
is not the unit — the shape TEST is.  Listed in C as the next instrument change, because this
walk is the second time a `τ?` opacity has been found by hand in a function the screen calls
clean.

#### B7i — the `optional` screen's unit changed to the shape TEST, and the first thing it found (2026-08-30)

Picked because C named it: the four writers B7h fixed by hand all sat in the screen's
"see through the wrapper" bucket, because each function peels `Optional` SOMEWHERE in its
body while a second shape test inside it stays bare.  A function is not the unit of this
question — the shape TEST is.

**Scored against a hand oracle written before the detector, 10 of 10.**  Six cells that MUST
flag (the three pre-fix writers, two bare tests each) and four that must NOT (the four
spellings of a legitimate peel: in the scrutinee, in a tuple scrutinee, bound to a local under
a new name, and bound to a local under the SAME name — `let tp = tp.base()`).  Run on the
pre-fix tree at `2bb7a7e1^`, where B7h had already established the answers by hand, and again
here, where the three fixed sites leave the queue and their siblings stay.

⚠ **Three detector faults, each found by the oracle or by reading the queue's own top row.**
Each was the failure the screen exists to find, in the screen:

| fault | how it read | found by |
|---|---|---|
| the per-test pass inherited the FUNCTION unit's gate | `wrap_dense_default_as_some` — one of the five writers `@FR-L-Null-Tag` names — was invisible, because the old unit's three regexes want a `Type::X` followed by `=>`, `\|` or a `let`, and a tuple pattern is none of those | control 7 absent, then probed rather than assumed — [[check-an-instruments-zero]] |
| a `match`'s arm BODIES and GUARDS counted as its patterns | `borrow_root`'s `match val.unspan()` — over a `Value`, not a `Type` — inherited the `Type::` list of a `matches!` in one arm's guard and went to the HEAD of the disagreement queue, one function apparently peeling in one place and not the other | reading the top row |
| `type_discriminated` cannot see the LAST alternative of a `\|`-chain | it needs a trailing `=>` or `\|`, and `\| Type::Trie(d, _, dep) = &in_type` ends in the binding `=`.  `Trie` dropped out of `for_type` and `index_type`, splitting the keyed family into a five-variant list and a four-variant one and MANUFACTURING a "these homes are short by Trie" finding out of the detector's own short list | reading two of the sites it accused |

The third is the sharpest: the instrument for "one notion, two spellings" had a list that was
short by one spelling, and the finding it invented was that other people's lists were short.
Only opening the accused sites separates the two.

**The ranking is what makes 707 sites readable.**  The flat queue is a floor over every body
with a peel anywhere; the useful question is the project's own recurring shape — group the
tests by the variant LIST they spell, keep the lists of three or more (a shorter one is a
generic test, not a shared notion), and report the groups where some homes peel and some do
not.  **19 lists disagree, over 98 bare sites**, and a disagreement is a claim that two homes
answer one question differently.  The `data.rs` definitions in it are NOT hits — `is_dbref`
and `is_scalar` are layout predicates over a bare `Type` by design, with the peel at the
caller (`ref_tuple_element_ok` is `is_scalar(tp.base())`) — which is what the caller-half
table already exists to read.

**loft#1204, from the first row read.**  `link_shared_nullable_views` is the rewrite that
points a keyed view at a nullable sibling's `__nullable<S>`, and both of its halves ask an
unpeeled type.  So a member spelled `hash<S[k]>?` — or a vector spelled `vector<S?>?` — falls
to `_ => None`, the view stays over `S` while the vector is over `__nullable<S>`, and the
declaration silently builds a SECOND independent collection that every insert misses.  Twelve
cells: all five keyed kinds broken in both spellings, on both backends, byte-identical.
`@FR-Col-Group` settles it without a design call — membership is a fact about the pair, and
`hash<S[k]>?` is a collection over `S` in that struct — so the rule's clarification list gains
the axis rather than the rule changing.

⚠ **The fourth row is the CONTROL that names the cause rather than the symptom.**  A
`?`-keyed member beside a DENSE `vector<E>` reads 1: plain group forming was never blind to
the `?`, so the defect is confined to the rewrite and the fix belongs in it, not in the
pairing test.  Both of the rewrite's call sites carried it — `parse_struct` and
`parse_variant` — measured apart against the control binary, because falsify's assert count
is 1 either way (a failed assert stops the run) and cannot tell one cell from six.

⚠ **A test file's "axes HELD FIXED" note was a stale measurement, and it pointed away from
this.**  `1158`'s header recorded *"the rewrite covers `hash` and no other kind — measured:
`sorted<E[k]>` beside `vector<E?>` is two independent collections in BOTH orders"* and drew
the reasonable conclusion that the nullable-element axis was not worth moving.  Re-measured:
ten cells, five kinds, both orders, all complete.  The note describes the state before a fix
that landed in its own PR — `collections.md` already lists that fix among `Col-Group`'s
instances.  A held-fixed note is a claim with a date on it, and this one would have stopped
the walk that found loft#1204.

#### B7j — auditing the fixed rewrite's siblings: two more, one fixed and one filed (2026-08-30)

The class B7i closed is *a declaration-time question about an ELEMENT asked of an unpeeled
type*, so the sibling audit is the immediate next step ([[audit-the-siblings-of-a-fixed-rewrite]]).
`advise_group_apart` is called on the line after `link_shared_nullable_views`, and it delegates
to `collection_groups` — whose own doc says it is *"one home for 'which fields are a group', so
the two advices over that question ... cannot disagree about what a group is."*

**The two halves of that one home disagreed.**  `is_keyed_collection` delegates to `is_keyed`,
which peels; `collection_element` beside it matched bare.  So a `hash<S[k]>?` member was
dropped from the group before `keyed` was ever consulted, and BOTH advices — `linked-group-apart`
and `linked-group-double-fill` — went silent on a group that demonstrably forms at runtime.
Quiet on a real group is the one failure these lints cannot afford, because the declaration is
the only place the question is decidable.  Fixed at the shared home, so both advices move
together; pinned by `group_apart_lint::a_member_carrying_its_own_question_mark_is_still_a_member`
over three spellings (`?` keyed, `?` vector, both), falsified by reverting the peel.

**And a value defect the same probe turned up, filed rather than fixed (loft#1205).**
`b.d? += [rec]` on a nullable vector FIELD appends the record TWICE and gives its keyed sibling
nothing.  The separating controls are what make it readable: the same declaration written
`b.d += [rec]` is correct throughout, and a nullable LOCAL takes the same write spelling
correctly — so it is neither the field's nullability nor the `?` itself, but the `?`-discharged
place.  The IR says why: the place lowers as a re-evaluable BLOCK, the RHS literal's backing var
is set to that same block, and the record is built INTO the destination and then appended to
itself.  `group_reindex_after_vector_write`'s structural `args[0] == to` test cannot recognise
the discharged spelling either, which is the keyed half.

`(E-Asgn-Compound)` settles the direction — the addressing sub-expressions evaluate exactly
once, *"for every place a compound assignment can target"* — so it is a deviation, the same rule
loft#1145 closed under, one place-spelling over.  It is FILED rather than fixed because the two
admissible cures (hoist the place to a `_place` temp, or refuse `x? +=` as an lvalue when
`x +=` already works) are a design call, and either wants its own matrix over every operator and
place spelling on both backends.

⚠ **Three sites of the same row read CLEAN, and the reason ranks the rest.**  `keyed_field_kt`,
`index_type` and the `for`-loop element type all agree with their dense controls, because a `?`
on a keyed collection is DISCHARGED at the point of use — by the time those run they hold a
dense type.  The `?` survives where a type is read from a DECLARATION, or from an expression
that has not passed a discharge (`?`, `??`, `match`, a non-null parameter store).  That is the
reading rule for the remaining bare sites, and it is why both defects here sit at declaration
time and at an lvalue place rather than on the use path.

⚠ **One probe in this pass was VACUOUS and only its control said so.**  A key-field write
through an element of a nullable keyed collection, routed via a parameter, produced no advice —
and none for the DENSE control either, so the probe never reached `note_key_field_write` at all.
[[a-count-of-zero-must-prove-it-ran]]; the cell is withdrawn rather than reported as clean.

#### B7k — the `+=` routing table, made total: over-claiming and under-claiming are one defect (2026-08-30)

Picked because B7j's own probe left it: the walk that closed loft#1205 filed loft#1215 at a
site whose problem it had named — *"that push site never compares `s_type` with `elm_tp` at
all"* — and the file-instead-of-fix note said it *"serves no correct program and funnels the
broken ones"*. Both halves of that sentence turned out to be claims worth re-measuring, and
one of them was wrong.

**The mechanism is one sentence: `towards_set`'s `+=` handling is a chain of route branches
with no `else`, so it is neither exclusive nor total.** Every defect below follows from that,
and the two directions had both shipped:

| direction | destination | what happened |
|---|---|---|
| over-claim | vector | the single-element push compares nothing, so an unrelated source was written RAW: `float` → its IEEE-754 bits as an i64, `boolean` → 8705, `text` → allocator panic, a struct source and a `vector<text>` element → SIGSEGV, `--native` → E0308 on all of them |
| over-claim | keyed | the bulk-fill route claims any VECTOR source without reading its element, so a `vector<text>` filled a `hash<E[k]>`: `len` 1, nothing reachable by key |
| under-claim | keyed | no catch-all, so an unrelated source emitted no write at all — `len` 0, in silence |
| under-claim | keyed | a record VARIABLE at a FIELD, because the record route is gated on the source being a struct LITERAL — a VECTOR's requirement, since a vector's bare element is @PLAN52's ambiguity. The keyed LOCAL and the bracketed FIELD were both correct, so one question had three answers |
| under-claim | vector | a VARIANT at a vector over its enum: the ambiguity check asks `is_equal`, `Reference(Named)` vs `Enum(Tagged, …)` reads unrelated, and the generic path grew the vector by THREE |
| under-claim | keyed | a whole keyed collection appended to another of the same type — nothing written, nothing said |

Filed as loft#1215 (the over-claims) and loft#1221 (the under-claims), fixed together because
splitting them would put the same chokepoint in two commits.

**The cure is one classifier, and its element test is the interesting part.**
`Parser::append_source` names which of three shapes a source is, so `Unrelated` becomes
expressible. What it must NOT do is spell a fifth copy of "is this an element" — this file
already asks that question in four places — so it delegates to `can_convert`, the predicate
arguments, returns and struct literals already ask. That delegation is load-bearing:
**the element type has two spellings**, `Reference(d)` where a keyed kind's `content()` reads
a nullable record and `Enum(d, true, …)` where a vector carries it, plus `(C-Var)`'s variant.
A fresh `is_equal` refuses two working corpus programs. `can_convert` was missing `(C-Var)`
entirely, so the rule went there rather than here — [[one-notion-two-ir-spellings]] a third
time in this walk, now at the level of a coercion rule rather than an op name.

⚠ **Delegating to a validator needed one guard, and the corpus found it rather than the
matrix.** `can_convert` answers TRUE for an unknown `test_type` — correct for a validator,
which must not report a generic body's placeholder as a mismatch — but read as *"this IS an
element"* every unresolved element type becomes a hit. A struct-enum's collection field
resolves lazily, so `j.xs += [Item { … }]` earned the ambiguity refusal *with the brackets
already written*, in a live gate (`977-struct-enum-collection-field-write.loft`). The lesson
is not "guard unknowns": it is that a predicate's ANSWER FOR THE UNKNOWN CASE is part of its
contract, and the safe value flips when the caller is a refusal instead of a validator.

⚠ **The filed note's "serves no correct program" was RIGHT, and its implied conclusion was
wrong.** With the refusal in place the push branch should be unreachable by argument —
`Element` is refused earlier by @PLAN52's bracket rule, `Whole` is claimed by concat — and
deleting it looked like the clean end of the thread. A probe at its head over ~2000 `.loft`
files says otherwise: **one caller**, a nullable ELEMENT source, which reaches it because
`(N-Store)`'s peel runs AFTER the ambiguity check and the peeled type is never re-asked. So
the branch stays, and what the measurement actually found is that the `?` spelling of one
statement is more permissive than the plain one (loft#1223). [[keystone-claim-is-a-measurement]]
— an "unreachable" derived from the code is a claim, and this one cost nothing to check.

⚠ **A guard's cure must itself work, and one draft of the message advertised a dead end.** The
first refusal text offered *"or the whole `hash<E[k]>` to concatenate"* — which is the third
under-claim row, a silent drop. Rewritten to name only the two spellings measured to work at
every destination kind, and the dead end became its own cell instead.

⚠ **Two refusal cells could not share a file, and the reason is the PASS.** @PLAN52's
ambiguity check is not pass-gated and fires in pass 1; the keyed-whole refusal is
`!first_pass`. Put together, the pass-1 error stops the file before pass 2 runs and the second
`@EXPECT_ERROR` goes unmatched for a reason that has nothing to do with the fix
([[which-pass-does-the-site-run-in]]). Split into `1221b` and `1221c`, with the reason written
at the top of each.

⚠ **A native failure in the probe file was the PROBE's fault, and separating it found a real
bug.** `--native` rejected the five-kind guard with `E0425: cannot find value t88`, which
reads as a divergence the fix introduced. It is not: the probe declared its trie's element
struct AFTER the struct holding the field, and a `trie` field with a forward reference emits
`db.trie(t, "k")` before `t` is bound. Two controls settle it — the same forward reference
with a struct-LITERAL source (a route neither fix touches) fails identically, and `hash` /
`sorted` / `index` / `vector` all tolerate it. loft#1222, filed and not fixed.

Blast radius: an env-gated probe at the check over ~2000 `.loft` files reports **one** append
the classifier calls unrelated, in an archive probe that already failed on a later line.

⚠ **The blast-radius sweep was scored on the wrong channel, and `make ci` is what said so.**
The sweep above reports one hit over ~2000 files and that number is true — but it greps for the
two new DIAGNOSTICS, so it can only ever find programs the change newly REFUSES. Half of this
fix makes a previously-dropped statement start EXECUTING, and that half is invisible on the
diagnostic channel *by construction*. `make ci` found what the sweep could not:
`373-empty-braces-collection-field.loft` read `total=20` against its own `expect 10`, because it
fills both members of a linked group and the second fill had been a no-op.
[[absent-warning-is-not-a-pass]] is the same lesson one channel over — there a leak channel
scored what a value channel should have; here a diagnostic channel did.

**The instrument that answers it is a differential on VALUES:** the same corpus through both
binaries, comparing stdout. Run debug-vs-debug it reports exactly TWO differing files —
`examples/collections.loft` and this walk's own new guard. Two traps in it, both hit:

  * **Compare like-for-like PROFILES.** The first run used the RELEASE build against the parent's
    DEBUG one and accused the change of a SIGSEGV in `1062-self-append-reallocation.loft`. Both
    builds print `1062 ok` on debug; the fault is release-only and is the already-filed
    loft#1216. A profile difference reads exactly like a regression.
  * **It is blind to every `test_`-only file.** Those have no `main`, so `--interpret` runs
    nothing and the cell is vacuous — which is why the differential did NOT find 373 and the wrap
    suite did ([[guard-entry-point-decides-what-runs]], one directory wider than usual).

⚠ **The doubling led to a defect that is NOT this walk's, and the control is what says so.**
Double-filling a linked group writes one good record and one whose `text` field reads `null`
while the `integer` beside it is correct. Reproduced byte-identical on the parent through the two
spellings that compile there, so loft#1221 makes a third spelling reach the path rather than
creating it — loft#1226. The shipped `examples/collections.loft` was written in the one spelling
that was a no-op, so it is one bracket away from corrupting on the released build; it and the
373 cell are corrected here by DROPPING the redundant append, since filling one member of a group
already fills the other. loft#1227 is the lint half: `linked-group-double-fill` covers the struct
literal and not the append, which is now the more dangerous spelling.

⚠ **A `COMPATIBILITY.md` register entry was proposed for the behaviour change and DECLINED, on
the document's own terms.** *"Contract 0 is pre-1.0 — the only era with no promise. Until the
freeze, every surface may move; this whole document takes effect at the `0 → 1` flip."* The
"a fix that changes an observable result is a regression" line is real and does not bind yet; §
What a falling bug rate does and does not license is the positive form — at contract 0 a defect
on a walked path *is simply fixed*, and landing this class before the flip is the point. An entry
would assert a promise that does not exist. The migration note went to CHANGELOG.md instead,
where an upgrading reader looks.

⚠ **The `optional` audit row moves 659 · 305 · 349 → 660 · 306 · 349, and the shape of that
movement is the point.** `Parser::append_source` ENTERS the table peeling — it reads
`dest.base()` before it classifies anything, because a `vector<τ>?` is the collection it names
plus one reserved null and which routes exist does not depend on the wrapper. So the
denominator and the peel column move together and the opaque column does not move at all,
which is what a new site added with the question already answered looks like. Contrast the
five REPAIRS loft#1207 recorded, where the opaque column fell: [[keystone-claim-is-a-measurement]]
applies to this table too — a row that improves is a claim to check, and a row that grows
evenly is the honest reading of a site that was never part of the backlog.

#### B7l — the refusal I nearly narrowed, and the appearance that argued for it (2026-08-31)

A sibling checkout reported that B7k's keyed whole-collection refusal had taken a WORKING
operation away: `b += a` between two keyed LOCALS answered `b=1 a=1` on the parent build and is
refused on the fixed one. Measured, and it reproduces. I wrote the narrowing — restrict the
refusal to `var_nr == u16::MAX`, the field destination where the drop was actually measured —
built it, and confirmed both cells.

**The narrowing was wrong, and the thing that says so is one more cell.** The sibling then read
the IR: `b += a` lowers to a plain `b = a`. It REBINDS — the destination is repointed at the
source's store and takes a dep on it. So `b=1 a=1` is not a merge that worked; it is an alias
seen from an empty destination, where an alias and a merge produce identical output. Two cells
separate them, and I ran both rather than taking the reading:

| cell | result | reads as |
|---|---|---|
| `b = []; b += a` | `b=1 a=1` | a successful merge |
| …then `a[2] = …` | `b=2 a=2` | **b follows a — it is an alias** |
| `d[1] = …; d += c` | `d[1]` ABSENT, `d[9]` present | **the destination's own records are gone** |

The populated-destination cell is the sharper of the two and it was not in the sibling's list:
the rebind does not merely fail to merge, it DESTROYS what the destination held, silently. So
the refusal is right at every place kind, the narrowing is reverted, and the comment at the site
now carries the measurement with a ⚠ saying what a build that re-adds the clause has been told.

⚠ **The corpus could not have caught this in either direction, and that is the transferable
part.** No `.loft` file in the tree merges two keyed locals, so the whole-corpus differential
that cleared B7k ran clean over both the original refusal AND the narrowing — the same
instrument, blind to the same gap, would have blessed either answer.
[[sweep-must-score-the-changed-channel]] says a sweep must be scored on the channel the change
can move; this adds the other half — **a sweep can only see shapes the corpus contains**, and
"the differential is clean" is a statement about the corpus, not about the change. The cell now
lives in `1221c` because that is the only thing that reports it.

⚠ **A peer's first framing was a design question and their second retracted it.** The first
message asked the owner to choose between "`h += other_h` means merge, so revisit the refusal"
and "it stays refused, so the tuple copy needs its own primitive". The IR measurement dissolved
the question — there is no merge to preserve — and the peer withdrew it unprompted. Worth
recording because the first framing was reasonable and I had begun acting on it: a design
question raised from BEHAVIOUR is only as good as the mechanism under the behaviour, and the
cheapest way to check is to read what the statement lowers to
([[consult-formal-spec-first]] one level down — the IR, not the rules).

**What it leaves open, which is not mine:** `D-tup-4`'s keyed half needs a keyed collection COPY
and there is none in the language — the vector cure (`o = []; o += vl; o`) has no keyed mirror,
because the keyed `+=` was an alias all along. That is plan-sized new parse-time machinery over
five kinds, on loft#1230.

#### B7m — loft#1223 closed, and the reachability count that changed under it (2026-08-31)

The last of the append walk's own filings, and it closes on a precedent rather than a decision.
B7k had measured that the vector single-element push has exactly ONE caller in the corpus — an
un-bracketed nullable element — and drew the conclusion that the branch is therefore not dead.
That reading was right and it is now stale: this entry refuses that caller, so the branch has
zero.

**The issue was filed as a design call and it was not one.** @PLAN52's bracket rule is a blanket
requirement on the SPELLING, and @PLN25 had already met the nullability axis on the DESTINATION
half of the same check — *"matched on the target's STORAGE, so a `vector<T>?` is refused the same
way"*. The SOURCE is that reading one position over. So the two-cures framing in the issue was
mine and wrong, and the correction is a `.base()` mirroring the one already there.
[[consult-formal-spec-first]] with the spec being a comment above the check: the precedent that
settles a question is not always in `formal/`.

⚠ **A refusal's cure can be under-diagnosed without being a dead end, and the two are worth
separating.** `d.c += [n]` — the spelling this sends the reader to — stores a null into a dense
`vector<integer>` in silence, on both backends and on the shipped release. B7k's rule was *a
refusal whose cure is broken sends the reader to a dead end*, and it made me hesitate here. The
cure is not broken: it compiles and appends exactly what it says. It is UNDER-DIAGNOSED, which is
a different defect with a different owner (loft#1232, filed, covering the whole vector-literal
family — the local binding and the struct constructor are silent too). Shipping the refusal moves
one reader from warned to silent for as long as that is open, and that is still the right trade:
a rule violation must not stay shipped to preserve a warning that the CORRECT spelling ought to
carry as well.

⚠ **The push branch is dead by measurement AND by argument, and is kept anyway.** Zero callers
over eight directories; and for a `Type::Vector` destination every source shape is claimed
earlier — an element by this refusal, a vector by concat, anything else by B7k's classifier. It
stays because the costs are asymmetric: a dead branch costs attention, a wrong deletion drops the
shape to the generic path which emits no write — the exact failure B7k's other half was. The
argument rests on the ordering of four checks that a later change may move, and **this branch was
called dead once already on a reading a measurement contradicted**. The reasoning is at its head
so the next reader deletes it behind a fresh probe rather than behind a comment.

⚠ **A guard carried a claim that a later commit in the same walk falsified.** `1215b`'s header
said the nullable-element cell was *"the only shape in the whole corpus that reaches the push …
so the push is not dead code to delete"*. True and measured when written; false three commits
later. It is CORRECTED IN PLACE rather than deleted, with the reason, because the useful thing is
not the current number — it is that **a reachability count is a measurement with a date on it**,
and the thing that moved this one was our own subsequent fix. [[keystone-claim-is-a-measurement]]
extends to the counts a walk leaves behind, not only the ones it starts from.

#### B7n — `@FR-B-Copy` walked: nineteen sites, two questions, one home short by one shape, and the release that followed the copy (2026-09-04)

Picked by `rule_tags.py dups`: at 19 sites across six files it was the most scattered rule not
yet walked, and the copy-vs-view class is where this week's six `silent-wrong` issues sat
(loft#1336/#1337/#1345/#1346/#1349/#1353).  The branch is `tuxedo-quality-2026-09`, cut from
the 2026.9.0 PR tip `12e58a4d`.

**The split.**  The nineteen citations ask two questions.  *Which binds copy?* — the parser's
`classify_vec_bind`, the dep-strip in `objects.rs`, the interpreter's first-bind and rebind
arms, native's record-copy arm, the branch-arm lift's `arm_bind`.  *How is the copy made?* —
`OpBindOrCopy` with the source as its own witness, `gen_set_first_ref_var_copy`,
`OpReplaceVector`, the tuple constructor's member copy, a `&`-parameter write-back, native's
`.clone()`.  The first question has a home: `Type::heap_def_nr` names the two heap-record
shapes (a struct, a struct-enum), and the interpreter's null-init arm, its call-source arm and
both native arms already read it.

**The disagreement.**  Three sites of the first question spelled `Type::Reference` bare —
the interpreter's two variable-source copy arms and the parser's dep-strip — and the arm lift
declined a struct-enum arm *because* codegen had no copy for it.  Measured on 45 cells
(`scripts/probe-matrix`, both backends), shallowest first: a struct-enum whole-value bind
ALIASED on the interpreter at a first bind, a rebind, from a parameter and through a
`(C-Var)` widening `c: E = s` — while `--native` copied every one (D-op-1, interpreter on the
wrong side); the `if`-join of two struct-enum variables aliased on BOTH backends; and a
NULLABLE struct-enum destination kept its source dep while both emitters copied, a copy nobody
freed.  All three read `heap_def_nr` now, and `Data::copies_as` is the new one home for which
record PAIRS copy (the same def, or a variant into its enum — native already admitted it, the
interpreter refused it).  Beside it the field resolver's struct-enum branch matched the
receiver bare too: `e.n` on an `e: Sh?` was "Unknown field" on a read and "cannot change type
from Sh? to integer" on a write (pass 1 re-typed the receiver), while `s.v` on `s: S?` resolved
through `type_elm`'s peel one line above.

**The negative result, 30 cells.**  Parameters, loop variables, field and element
destinations, `??` subjects, deep copies of nested vectors and structs, rebinds, text,
sorted / index, `if`-arm and loop-body contexts, and the destination-write direction all copy
as `(B-Copy)` says, identically on both backends.  Written into the boundary file as its
sixteenth and seventeenth cells, and the two guards
(`a-struct-enum-whole-value-bind-copies-like-a-struct`, 10 cells;
`a-nullable-struct-enum-payload-field-resolves-like-a-struct-field`, 5) are falsified at
`12e58a4d` on both backends.

**The defect the fix uncovered, and the rule that settled it.**  The struct-enum arm of
`139-drop-cascade.loft` c8 went from `alive,30` to `alive,30,30` once the arm copied — and
its struct twin already read `alive,30,30`: a whole-value copy of a record holding a
droppable released it once per RECORD.  `h2 = h` twice, `t = s` twice, `t = s; return t`
THREE times with two of them before the caller had read its copy.  C111's own words name the
failure (*"two records holding one resource"*) and its answer (move-on-construction), and
`INTERFACES.md`'s move rule covered a field, a payload and an element but not the plain
copy that is the same step with no container around it.  `scopes::copy_moves_drop_from` is
the one home now: the collector's `Set(v, Var(src))` and its `OpCopyRecord` into a `__ref*`
buffer (a materialised branch arm, a return buffer), the arm lift's `__lift_N = a` (built
AFTER the collector ran — the third home the walk had to find), and the `double-move` lint,
which reports `t = s; u = s` as it reported two containers.  A copy off a parameter leaves the
caller as owner.  Deliberately kept: a multiply-assigned source or copy (the transfer set is
per variable, the fact per assignment — `@FR-O-Latest`), because a rebind never releases what
it displaces — measured, pre-existing, filed as **loft#1362** with its cells.  Guard
`a-whole-value-copy-of-a-droppable-releases-once` (12 cells, two controls), falsified at
`12e58a4d` on both backends.

**Filed apart.**  A tuple with a heap member is SHARED by a whole-tuple bind, a destructure
and a projection, and by the literal for a STRUCT member — `(T-Cons)`'s copy lives in
`tuple_member_owned_copy`, short by the struct member and never reached by the bind
(**loft#1361**, `tuples.md` D-tup-8, taken by the sibling checkout for the tag).

**Method notes.**  Two of my own probes were truncated by the `head` I piped them through,
and each cost a wrong hypothesis (*"the block is not in the collector's input"*) before the
untruncated run showed the collector saw the copy and the DESTINATION's null placeholder was
what failed the single-assignment test.  Print a diagnostic whole, then filter.  And the
arm-lift comment said *"a struct-Enum … has no `Var`-copy lowering to hand the temp to"* — a
carve-out that stated the defect as its reason, exactly the shape
[[carve-out-comment-can-state-itself-as-the-rule]] warns about.

#### B7o — loft#1362 closed: a reassignment releases what it displaces, and the drop hand-off fact follows the assignment (2026-09-05)

The open issue the B-Copy walk filed, taken next because it sits in the same family with
its matrix half-built and because the owner's standard does not ship around an open
issue.  Fourteen rebind cells on both backends, baseline: seven never released the
displaced record (the droppable itself, a struct rebuilt in place, a struct-enum, a nullable
local, a rebind to null, a call result, a right-hand side that reads the old value); a loop
body's local already released per iteration; and a literal handed into a NESTED field or an
element released twice.

**The mechanism, and why the fix is a snapshot.**  `s = S {…}` on a live local is not a `Set`
in the IR at all: the parser rebuilds the record in place (`OpDatabase(s, tp)` on the
existing store), so the old bytes are gone before any hook could read them; every other
rebind freed the displaced store without its hook.  A hook before the statement is wrong
where the right-hand side reads the old value (`s = grow(s)`), a hook after is wrong where
the rebuild is in place — so `scopes::displaced_drop` copies the record into a null-safe
temp at the head of the scan's prefix and runs the hook on the temp after the statement.
Three wrong cuts, each a measurement: the temp marked never-free for the sweep silenced its
own explicit free (a leak of every snapshot store, interpreter only, visible only with the
leak line in view); the temp left visible was dropped AGAIN by the sweep on a freed
reference that still reads `rec != 0` (the cure is the true sentinel after the free); and
the transition free's loop-depth guard hid the rebind of a local declared outside the loop
(the outer owner fact is trusted when THIS assignment owns too).

**The fact belongs to the assignment.**  With rebinds releasing, the copy-move's
per-variable single-assignment guard read wrong in both directions: `t = s; s = …` released
16 twice (the copy AND the displaced source), `t = s; t = …` released nothing.
`drop_transferred` is now re-armed by every statement's hand-offs in scan order and retired
by an unconditional reassignment — `@FR-O-Latest` for drops — and the guard is gone.  A
reassignment inside a deeper scope keeps the hand-off (the leak direction; a conditional
hand-off is a runtime fact this pass does not have).

**The sibling short list.**  `copy_hands_off` read one field level: `o.s = S {…}` copies
into the nested `o.s.h` and `v[0] = S {…}` into an element, and neither counted, so the
literal's work-ref dropped beside the container's cascade.  It peels field and element
reads to the root now, through a keyed read never.

**Rule.**  `formal/heap.md` had no drop clause at all — every drop question of these two
days was settled from C111 and INTERFACES.md prose.  `(H-Drop)` states the one release per
resource at the owner's death (scope end, reassignment, container cascade) and the move
with a copy; `(H-Drop-Not)` names the boundary (an overwritten field's or element's old
value, a removed element, a keyed collection's records).  Guard
`1362-a-rebind-releases-the-droppable-it-displaces.loft`, 13 cells, both backends, both
leak checks armed.

**Left open, on the record.**  A conditional hand-off (a droppable-holding variable yielded
by ONE arm of a branch) suppresses the source's release on every path — the untaken arm's
resource is released by nobody (leak direction, pre-existing since the copy-move).  And an
overwritten FIELD's old value is the same class as a variable rebind one level in; it is
kept on the documented boundary because the parser's field write cannot yet tell a
construction from an overwrite, and a hook on a zeroed fresh field would be a false release.

#### B7p — `@FR-O-Latest` walked: the residual reads a single-argument dep as ownership, and frees a view (2026-09-05)

Picked by `rule_tags.py dups`: at 18 sites `@FR-O-Latest` is the most scattered rule not
yet walked, and it sits under the store-lifetime class this cycle keeps producing.  Its
sites split into two questions — *which store does the LATEST assignment give this binding*
(`owned_refs`, the memo, and the transition frees that read it) and *which assignment does a
CAPTURE / a fn-ref join name AT ITS BUILD* (`capture_build_backing`, `callref_join_bases`).
The walk built the latest-assignment matrix: a nullable heap local reassigned from a second
source, KIND × first source × second source × POSITION, 1266 cells on both backends, scored
under `LOFT_POISON` because the defect it found is invisible on a plain build.

**The disagreement.**  Codegen's `borrows_one_argument` (the D-own-16 residual, spelled again
at this file's scope-exit `borrow_witness` and in `generation/dispatch.rs`) reads a nullable
local's single-ARGUMENT dep as ownership and frees the store it DISPLACES at a reassignment.
Sound for the shape it was written for — `d: S? = p` WHOLE-value-borrows the argument, whose
store is free-protected, so the free is declined on the borrow path and taken on the mint
path.  Wrong for a PROJECTION: `d: In? = q.inner` aliases q's NESTED store, which carries no
free-protection, so the reassignment released the CALLER'S record; the same fired for a view
of a LOCAL's field (`d: In? = o.inner`) and a vector element (`d: In? = vs[i]`), because the
projection's dep still names its base.  A SILENT-WRONG: the read was correct until a later
allocation reused the freed slot, at which point `q.inner.v` returned the filler's value —
`777` for `71`, both backends — and the vector-element shape crashed out of bounds.

**The fix, at the fact.**  A view owns no store (@FR-O-Owner), so the proxy that licenses
the free is wrong for a view-holder and @FR-O-Override vetoes it.  `scopes::nullable_view_locals`
names the nullable heap locals that hold a projection view (the oracle calls it `Borrowed`
and it is not a bare `Var` — a whole-value bind COPIES, @FR-B-Copy) and marks them
never-free before the scan, which all three free-site twins already consult through
`is_skip_free`.  The two mixed-ownership shapes that DO own a store are excluded and keep
their machinery: a solely-owned minting call its loft#1200 runtime flag, a view+mint mix its
owner witness (loft#1336).  What remains owns nothing it must free — all views, or a view
plus a literal that frees through its own work-ref — so never-free leaks nothing.

**Verified.**  1266 cells green on the interpreter and the fixed shapes on native, all under
poison, with the 42 collection-off-a-parameter cells re-scored as a CORRECTED oracle (B-Copy's
own `d = self.data` copies, so a parameter is an owned base for a collection projection — the
first read expected a view and was wrong).  Guard
`a-nullable-view-local-does-not-free-what-it-displaces.loft`: four displaced-view cells that
force slot reuse so the free is a wrong VALUE rather than surviving bytes, and two controls
(the whole-value parameter bind that still frees, the view+mint that keeps its witness),
falsified at `51646648` on both backends.

#### B7q — `@FR-O-Override` walked: a contract that named one spelling of five, and the release the language ships that it excluded (2026-09-05)

Picked by `rule_tags.py dups`: at 19 sites `@FR-O-Override` is the most scattered rule not yet
walked (271 mentions of `skip_free`, 44 writers).  Its sites ask THREE questions, and the walk
measured each over the whole 1247-file corpus (`tests/scripts` + `tests/docs` + `examples`,
compile-only under `LOFT_OWN_ORACLE=check`, a wildcard on the marking trace, and four
env-gated probes at the bytecode-level frees that have no IR):

- **"May a free be emitted for this binding?"** — the veto, read at a free site.  The rule's
  sentence said *"no `OpFreeRef` is ever emitted"*, and a free is a NOTION with five
  spellings: `OpFreeRef`, `OpFreeRefTag`, `OpFreeText`, `OpFreeRefIfDistinct`,
  `OpFreeRefOrHandUp`.  Both backends intercept the flag DOWNSTREAM for two of them — the
  interpreter's `generate_call` and native's `OpFreeRefEmitter`/`OpFreeRefTagEmitter` emit
  nothing for a bare never-free variable — and for none of the other three.  So a new
  **Check D** in `ownership_cfg` asks the contract by notion: every free op whose first
  argument (a `Var`, or a `TupleGet` of one) is never-free is a RED in a live spelling and a
  NOTE in a dropped one.
- **"Is this binding a view / non-owner?"** — the flag read as an OWNERSHIP fact, which the
  rule says it is not (*"exactly that sentence and nothing weaker"*).  Twelve readers do it.
  The shared home is `Function::owns_store` (`!inline_ref && !skip_free && is_independent`,
  loft#664 — native allocation, coroutine persistence, the interpreter's frame teardown, the
  parser's in-place target).  Every other reader is either CONSERVATIVE (the null-init
  sentinel, the reclaim/NRVO `REJECT`, an arm view that only skips a free) or PREFIX-QUALIFIED
  — `pre_eval`'s `__ret_` + flag, `emit.rs`'s `__ncc_` + flag, `is_overwritten_view`'s
  `_mv_`/`__ncc_` + flag — which is the de-conflation loft#1155 already established.  Two
  DIAGNOSTIC blind spots remain and are recorded, not fixed: the leak scan skips every
  never-free binding (a leaked transferred or staged binding is invisible to it) and
  `validate_slots` skips any slot pair with a never-free member (written for the S34 slot
  share, it covers every other meaning too).
- **"What makes a binding never-free?"** — 44 writers, and they mean FOUR things: (i) a VIEW
  the owner frees (match/capture payload bindings, element and field views, the borrowed
  `??` arm, B7p's nullable view locals — the large majority); (ii) TRANSFERRED, the owner is
  elsewhere (`__ret_` stages, the moved source, a witnessed local whose witness frees, a
  rebind's `__vdb`); (iii) a DEAD declaration with no store at all (`unregister_work_ref`,
  `clean_work_refs`, collapsed work-refs); and (iv) a DEFERRED release — never-free for the
  SWEEP, freed by the pass that marked it, on a consumption fact.  Of the 44, 38 fired on
  the corpus, five never did (among them `state/codegen.rs`'s single-use MOVE shortcut, which
  the @PLN90 move plans superseded and which no corpus program reaches), and one wrote the
  field directly and so was invisible to the trace (`mark_skip_free_by_name`, now routed
  through the setter).

**The disagreement, and it was between the rule and the code.**  Check D's first run reported
**9054 REDs over 217 function–binding pairs — every one an `OpFreeText`, every one a `__ncc_N` or `__ret_N`
text temp**, and not one `OpFreeRefIfDistinct`, `OpFreeRefOrHandUp` or tuple-element free of a
never-free binding anywhere in 1247 files.  Those are meaning (iv): the `??` text subject is
marked never-free so the scope-exit sweep does not free the value its block yields, and the
@PLN85 ncc-orphan pass frees it right after the statement that copied it out; the loft#1357
return stage does the same after the bytes move into the caller's buffer.  The language ships
that release, it is tested (156, 622, 1357), and the rule's literal sentence forbade it — so
the RULE was extended, not the code: `(O-Override)` now reads *no free DERIVED FROM OWNERSHIP,
in any of the five spellings*, and names the ONE admissible free — the release the marking
pass places itself, on a fact of its own.  That shape has a name now,
`Function::is_staged_text_temp` (flag + `__ncc_`/`__ret_` prefix + text, the same
prefix-and-flag pattern as `is_overwritten_view`); the orphan pass reads it instead of its
own three-line spelling, and Check D admits exactly it.  Recorded as D-own-31, opened and
closed.

**The second finding was two mechanisms on one local.**  Check D's 14 NOTEs — an IR
`OpFreeRef(c)` of a never-free `c`, dropped at codegen — were all in loft#1200's own guard:
a local whose assignments mix ownership received BOTH the loft#1200 displacement flag
(`if __lbo_c { OpFreeRef(c) }`) and the loft#1336 owner witness, whose never-free mark then
made the codegen veto drop the flag's free on both backends.  Right by accident, and dead
weight: 172 `__lbo_` lines in that file's IR.  The witness block now runs BEFORE the
displacement flags, so `nullable_locals_that_displace`'s existing never-free exclusion keeps
a witnessed local out — one release mechanism per local, 0 NOTEs over the corpus, the 1200
and 1336 guards green on both backends under `LOFT_POISON` + `LOFT_STRICT_STORES`.

**The third was the notion spelled nine ways.**  "Which ops free their first argument?" was a
hand-spelled name list in nine places — `ownership_cfg` (twice), `pre_eval::free_op_var`,
`scopes::scope_free_op_var`, `check_ref_leaks`, `use_analysis::FreeOps` (twice), codegen's
`is_cleanup`, the debugger's UAF tracker and `introspect`'s per-iteration-free check — and
**no two lists agreed**: three knew nothing of `OpFreeRefOrHandUp`, five nothing of
`OpFreeRefTag`, `is_cleanup` knew two of the five.  `check_ref_leaks`'s own comment had
predicted this — *"every matcher keyed on the op NAME went blind to the new spelling at
once"* (loft#1186) — and then stayed the only list that was complete.  `OpSets` (the
`Data`-cached op-set home) now carries `frees`, `unconditional_ref_frees`,
`conditional_ref_frees` and `text_free`, and all nine read it.  Verified as a refactor:
IR and emitted Rust BYTE-IDENTICAL before and after on the nine corpus files whose IR carries
`OpFreeRefOrHandUp` plus the 1336 guard; the `is_cleanup` probe had already shown its 1966
misses (1554 `IfDistinct`, 19 `OrHandUp`, 393 `OpFreeScratch`) all landed with
`return_expr = 0, has_return = false`, so widening it changes no bytecode on the corpus.

**Side-finding, recorded.**  Check B's doc claimed *"0 FP across scripts+docs+lib+examples"*;
the test behind the claim runs nine files.  Over 1247 it reports 110 `free-of-borrowed` (100
from the fact-based pass, all a vector header naming its own `__vdb_N` backing; 10 from the
dep-based pass, all a pass-2 work-ref borrowing an inline call's NRVO buffer,
`__ref_p2_1["__ref_1"]`), and each file runs clean under `LOFT_STRICT_STORES` + `LOFT_POISON`
on both backends — a precision residual of the checker, now stated as measured.  Check B also
consults the veto now: a free the IR names for a never-free binding is dropped downstream and
is not an over-free.

**Verified.**  Check D 0 RED / 0 NOTE over 1247 files; `oracle_override_check_flags_an_injected_free_of_a_never_free_binding`
proves it can fire (`LOFT_OWN_INJECT_FREE_SKIPFREE=<var>` makes the sweep name a
witness-guarded free of a never-free binding against itself — a run-time no-op the IR still
names); `a_witnessed_local_carries_no_dead_displacement_free` pins the one-mechanism rule on
the 1200 guard.  Guards 1200, 1336, B7p, 1357, 156, 622 and 1186 green on both backends.
`LOFT_SKIPFREE_TRACE=*` (new wildcard) is the writer census instrument.

#### B7r — `@FR-O-Oracle` walked: two derivations of one question, fourteen disagreements, and both sides had one defect (2026-09-05)

Picked by `rule_tags.py dups`: at 17 sites `@FR-O-Oracle` is the most scattered rule not yet
walked.  The rule says there is ONE own-vs-borrow derivation (`use_analysis::ownership_of`),
that it reads the IR and not `deps`, and that a chokepoint reads it rather than re-deriving.
Its sites ask three questions — *what does this VALUE own or borrow* (the oracle's readers),
*is this a second derivation of the same question* (the re-derivations the rule forbids), and
*how does a callee's answer travel back to the caller* (the interprocedural half, whose
failure mode loft#1318 named).  The walk did not need a new matrix: the corpus census the
previous walk ran under `LOFT_OWN_ORACLE=check` had already recorded **14 `fact-disagree`
lines** — the @PLN94 flow-sensitive shadow (`ownership_cfg`) and the oracle answering the same
variable differently — and Check A's own doc says each such line is *"a real defect in one
implementation"*.  Its zero was measured on nine files; over 1247 it read fourteen, in four
files and three shapes.

**Two shapes were the oracle's defect.**  `classify`'s first arm read *"a var `OpDatabase`
minted a fresh store into is Owned regardless of any other def"* — the retbuf a
`materialized_view_return` fills, generalised to every minted variable.  `c: M = M { x: 5 };
c = cond(c, 3)`, where `cond` mints on one arm and returns its argument on the other, is a
`Join` of the mint with that call; the shortcut said `Owned`, the verdict that licenses a
free.  A keyed literal built inside a closure into a captured collection (`__kvb_1`: minted,
then repointed at the capture's store, which `OpDatabase` clears IN PLACE) is a view of the
capture; the shortcut said `Owned` there too.  Both were masked at run time — the
distinctness guard `OpFreeRefIfDistinct(old, new)` on the first, loft#1331's detach on the
second — which is the shape @FR-O-Oracle's caveat names: the wrong answer in the over-free
direction, held right by something downstream.  The arm now JOINS the mint with the variable's
other definitions, in which a bare-`Var` right-hand side is a copy (@FR-B-Copy, and the mint is
the copy's own store) and so `Owned`, while a call or a projection is whatever the oracle
says of it; a minted variable with no `Set` at all (the retbuf) stays `Owned`.

**One shape was the shadow's.**  `for wrd in wlist`, where `wlist = file(…).lines()`: the
oracle roots the iteration temp at the caller's own delivery buffer (`Borrowed(__ref_2)`,
loft#1318's *"a hidden buffer is nameable"*), the shadow said `Join(MAX)` — because its
private copy of the callee-to-caller base translation, written to *"mirror"* the oracle's,
had none of loft#1318's three fixes.  The translation now has ONE home,
`use_analysis::structural_arg_base` (the hidden-parameter rule, the delivery-buffer
exception, the projection-root walk), read by the oracle and the shadow alike; the shadow asks
the oracle for the one thing a structural walk cannot root, a call-shaped argument, exactly
as the oracle asks itself.  The independence @PLN94 keeps is in the FLOW, not in the
translation.

**Verified.**  Check A: 14 → **0** disagreements over the 1247-file corpus, 1017b/1326/1331
added to `oracle_clean_on_correct_corpus`.  Emission: the pre-change compiler (the release
binary the morning's gate built) and the new one produce BYTE-IDENTICAL `introspect` output —
IR, bytecode and emitted Rust — on every one of the 1247 files, so the refinement changed
facts the checks compare and nothing the backends emit.  The A1b true-positive gate
(`oracle_flags_the_a1b_wrong_plan`) lost its disagreement in the process, and it had to: the
disagreement it asserted WAS the two defects above meeting on one fixture (`Borrowed(MAX)`
against `Owned`), and the known-wrong plan fails its own fixture at run time (`len: 0`, on the
old binary too), so the doc's *"the class the runtime gates structurally miss"* was already
stale.  The gate now asserts the runtime failure, the correct plan's pass, and Check A CLEAN on
both plans; Check A's true positive is injected (`LOFT_OWN_INJECT_FACT_OWNED=<var>`, forcing
the shadow's `Owned` where the oracle reads a plain borrow), like the leak and over-free
controls before it.

**The second derivation of a callee's RETURN, measured.**  `Definition::return_adopts_fresh_store`
(the deps proxy the emitters' buffer logic reads) and `return_ownership` (the oracle's
IR-derived class) were compared over every heap-returning function in the corpus (`RETSUM`
lines under `LOFT_OWN_ORACLE=own`): 1244 functions, 277 where the two differ.  245 are
`adopts=false, oracle=Owned` — the `r = Rec {…}; r` style whose renamed-buffer dep the
proxy's own doc explains and deliberately keeps.  **32 are the risky direction**,
`adopts=true` with the oracle reading a borrow of a visible parameter — 30 generic MONOMORPHS
(`t_5S1066_*`, `t_4Cell_same`, `t_6GwNode_gw_walk`, …) whose instantiation carries an EMPTY
return dep where the template's return borrows its argument, plus two closures.  Not a live
defect: every free decider that reads the proxy (`displaces_owned_through_fresh_callee`,
`delivers_into_buffer`, the codegen adopt) keys on the callee's hidden delivery BUFFER, which
a borrow-returning function has none of, and the copy-vs-adopt of the value itself reads the
oracle (loft#1346).  Recorded as a `@FR-G-Mono` observation: the monomorph's declared return
dep is a proxy that reads "fresh" for a borrowed return, and only the oracle knows better.

#### B7s — `@FR-O-NoDiverge` walked: the twin gates kept "verbatim" by hand, and a receipt that named readers it never had (2026-09-05)

Picked by `rule_tags.py dups`: 13 sites.  The rule says both backends translate the SAME
`deps` facts and therefore cannot diverge — every free/copy/move question is answered by
reading a carried fact, never re-worked out in a code generator.  Its sites ask two
questions: *which decisions still LIVE in a backend* (the re-derivations the rule tolerates
only if both backends spell them identically), and *which have moved INTO the IR* so one op
serves both (the transition frees, the owner witness, the detach — the rule's preferred
mechanism, and where every recent fix went).

**The disagreement was structural rather than measured.**  The one decision still made in
both backends — the displacement free at a heap reassignment — is two predicates,
`state/codegen.rs`'s `owned_ref` and `generation/dispatch.rs`'s `owned_ref_reassign`, each
carrying the other's condition list "verbatim" by hand, and their comments record four
rounds of drift, each found by a leak or an abort on one backend alone: the keyed kinds, the
VECTOR destination (loft#1328, one store per iteration to frame exit and a `store table
exhausted` abort at 70 000 iterations on native only), the `@FR-O-Override` veto, and the
detach (loft#1331).  A third spelling of the one-argument borrow test they share
(`d: S? = p`, D-own-16's residual) sat in the scope-exit sweep's `borrow_witness`.  The
fact-reading half now has ONE home, `Function::owns_displaced_store` (the store-backed kinds
through `base()`, the empty-dep proxy or the one-argument borrow, the override veto, the
capture exclusion, the detach), with `Function::borrows_one_argument` beneath it; both backends
and the sweep read it, and what stays per backend is only what IS per backend — the
interpreter's hidden-buffer-argument exclusion; native's declared-local, store-producing-rhs
and retbuf-witness conditions.  Verified as a refactor: `introspect` output — IR, bytecode and
emitted Rust — BYTE-IDENTICAL across all 1247 corpus files against the committed compiler, so
the two hand-kept lists agreed everywhere the corpus reaches, and cannot drift again.

**One citation was a false receipt.**  `Function::has_borrow_arm`'s doc said *"read by BOTH
backends' displacement frees, so neither can free on the proxy where the other declines
(@FR-O-NoDiverge)"*.  Neither reads it, and neither ever did: its one reader since loft#1333
is the fn-ref collection-delivery strip in `scopes.rs`, which leaves a mixed binding's dep in
place — and it is that DEP, read by both frees, that keeps them agreeing.  The mechanism the
receipt described would have been a third fact beside the deps; the actual one is the deps.
Corrected at the site.  A record-typed mixed local is covered by a different route again, the
owner witness — two predicates (`mixed_ownership_locals`, gated on a fn-ref call;
`owner_witness_locals`, on a user heap-record local) for two overlapping readings of "mixed",
recorded rather than merged: they answer different sites and neither is redundant today.

**The census, for the next walk.**  Ownership-fact reads that remain in a backend rather than
in the IR: about 30 lines in `state/codegen.rs` and 20 in `generation/dispatch.rs`, of which
the displacement gate above was the only PAIR spelled twice; the remainder are single-sided
by design (the interpreter's `owned_reassigned` sentinel reset, native's `_own_store_`
runtime-Join witness and `_rb_w_` entry-buffer witness — each the other backend's equivalent
of an IR op, reached by a different mechanism and agreed by result, which `differential_oracle`
and `leak_cross_mode` measure).  The rule's direction of travel is unchanged: a decision both
backends need belongs in the IR; one they cannot share belongs behind one predicate.

#### B7t — `@FR-F-Ret` walked: a generic's instance returned the argument it was handed, and the tuple return leg wrote three members wrong (2026-09-05)

Picked by `rule_tags.py dups`: 13 sites.  The rule says a returned whole heap value is FRESH —
mutating one call's result changes neither the argument nor another call's result.  Its sites
split into *is this return leaf a view or owned* (five parser-side classifiers, the deps
proxy, and the oracle) and *how is an owned return DELIVERED* (the buffer machinery, and
`boxed_tuple_return`, which a named declaration and both lambda forms pass through).  The walk
did not start from the sites: it built the matrix the rule itself states — T binding (record,
vector, keyed, text) × return shape (the argument, through a local, an early return, an
if-arm, a tuple literal, a tuple through a local) × generic vs concrete, 48 cells, each
mutating the first result and reading both the argument and a second call's result — and the
CONCRETE twin is every cell's oracle.

**Every concrete cell passed; 13 generic cells failed identically on both backends.**  A
template binds `T` as a record, so its instance keeps the RECORD lowering whatever `T`
becomes, and a generic's return promotion is deferred to instantiation by the declaration
(`return_shape_depends_on_type_var`) where nothing received it — the site @PLN85's
generic-tuple-return-fix.md had already named as missing.  Four consequences, four cures, all
at instantiation and all mirroring what the concrete twin does:

- A `-> T` record return carried NO deps (the declaration's `ref_return` never ran, so the
  `MergeAttr` that writes `-> Ctr["x"]` on a concrete twin never happened); the caller bound
  the argument's own store.  The instance now takes its return deps from the ORACLE
  (`return_ownership`, @FR-O-Oracle), and only where every return leaf is literally the
  parameter — a local bound from it copies at codegen, a copy the IR does not show, and
  declaring THAT a borrow made the caller decline its lift and free nothing (three corpus
  generics leaked one record per call under `LOFT_STRICT_STORES`, caught by the strict sweep
  of the corpus files whose IR moved under that first cut).
- A `-> (T, integer)` stayed a STACK tuple whose heap member was the argument (the teammate's
  D-tup-9 collection half, loft#1365).  `tuple_return_rewrite` — the one function the pass-1
  prediction and the pass-2 signature share — now boxes a lifetime-bearing literal tuple the
  declaration deferred, and `promote_monomorph_tuple_return` rewrites the body's tuple tails
  and `return (…)`s into the synthetic record, the tuple twin of the text promotion.
- A vector `s = x` in the instance ALIASED where a concrete bind copies, and the frame then
  freed the caller's vector; a vector `-> T { x }` handed the argument up where the concrete
  callee copies into its buffer (a caller never copies a vector it is handed).
  `promote_monomorph_vector_return` gives both the copy: `OpReplaceVector` into the local's
  own store, and into one fresh local the frame returns.
- A keyed member reached the synthetic tuple record through `emit_set_one_element` as a
  4-byte header (`OpSetInt4`) where a struct field write copies (`OpReplaceKeyed`): the
  interpreter wrote into a released, reused store (`Write to read-only store`) and native
  refused the int for a `DbRef` — an accept/reject split on a CONCRETE `s = x; t = (s, 7);
  return t` with a keyed `x`, the cell D-tup-8's guard did not cross.

**Boxing the generic routed it through the concrete tuple-return leg, and that leg had two
defects of its own, both pre-existing on concrete code.**  A NULLABLE record member: the plain
field write put the dense payload on the tagged slot's discriminant, and `(x, 1)` read back
`4294967199` for `7` (the loft#1134 shape at the return; `tuple_elem_tag_write`, which the
element-wise path already used, now runs first — decided by the ELEMENT's own type, because a
struct literal in that position is already lowered to the tagged `#NullableSome` record and the
first cut wrapped it a second time, reading the tag `2` for `7`: two corpus tests, 1123 and
1139, went red on both backends, and the IR census over the corpus is what caught it — a walk
verifies the FILES whose IR moved, not the cells it wrote).  A nullable VECTOR member's `null`: appended
nothing and left an EMPTY vector, so `miss.0 == null` was false (the reserved absent id
`mark_collection_absent` writes for `H { xs: null }` now goes into the slot, for the bare
`null` a declaration spells and the typed `OpNullRefSentinel()` a template gives a `T?`).

**Verified.**  Guard `a-generic-instance-returns-what-its-concrete-twin-returns.loft`: 52
cells (the 48 plus nullable record and vector members, generic and concrete), green on both
backends under `LOFT_POISON` + `LOFT_STRICT_STORES`, falsified at `babf9e64` (interpret exit
1 → 0 with 12 assertion failures → 0, native exit 1 → 0).  Corpus: `introspect` output moved
in five of 1247 files, the generic-return and nullable-tuple-return tests (1028, 1273, 808,
1123, 1139), every one green on both backends under strict stores; `template_matrix` (827)
and `issues` (29) green.  The first cut moved eleven — the oracle deps attached through a
local and on a `Join` as well — and the strict sweep of those eleven is what narrowed the
deps rule to the parameter-only leaf.  The `optional` and `unspan` audit
rows moved by the six new IR walkers and five `Type` discriminators, all peeling.

**Recorded, not fixed.**  A `-> T` whose body MIXES a mint and the argument (`Join`) still
adopts on the borrow arm — a named function delivers that through a return buffer the
instance does not have; the cure is the buffer at instantiation, and the teammate's
`boxed_tuple_return` note names the same machinery.  The five parser-side return classifiers
(`return_leaf_is_owned_or_null`, `return_views_local`, `return_projects_into_local`,
`classify_reference_delivery`, `returns_borrowed_view`) each answer *is this leaf a view* by
their own walk beside the oracle; the walk did not fold them, and it is the next question
this rule asks.

#### B7u — `@FR-O-Complete` walked: the statement form its guards never crossed, and four nullable locals not treated as the heap locals they are (2026-09-05)

Picked by `rule_tags.py dups`: 12 sites.  The rule says the ownership fact is per binding
and per PATH — a set-and-reconcile across every `if`/`match` arm, not a structural walk.  Its
sites split into the static reconcile at a join (`scan_if`'s intersect of `owned_refs`, the
`Loop`/`Iter` retain), the deps side of the join (`arm_join_type`, `Type::joined_deps`), the
per-arm temps a bound value branch gets (D-own-8's `arm_bind` / `lift_join_arm_tails`, and
the `??` hoist's own copy of that classification), and the runtime witnesses where one static
site cannot separate the paths.  `match` lowers to `if`, so the join has one home.  The walk
did not start from the sites: it built the matrix the rule states and its guards had not
crossed — the STATEMENT form, a local assigned on two paths with different ownership (a
mint, a view of a parameter's field, a copy of a parameter) × record / nullable record /
vector / keyed × both arms / pre-init then one arm / inside a loop, every cell called TWICE
with a fresh source, scored on value, strict stores and poison, both backends.

**Record, vector and keyed: 81 of 81 green** (a vector or keyed field bind is a whole-value
COPY under `(B-Copy)` and only a struct projection views — the matrix's first draft expected
write-through there, and the draft was wrong, not the compiler).  **The nullable column was
red across the board, and the isolating probes split it into four defects**, none of them the
mixed-path join the matrix was drawn for, every one a nullable local not treated as the heap
local it is — and the shallowest with no branch in it at all:

- **A binding that adopts a literal's work-ref inside a loop body.**  `for … { o = O { opt: S
  { n: 6 } }; y: S? = S { n: 3 }; }`: the literal builds in a function-scoped `__ref_p2_N` the
  binding aliases; the binding dies per iteration, its plain free returns the store, the
  buffer keeps the number, and the next pass re-mints through `OpDatabase`, which reuses the
  slot's store in place — by then `o`'s.  The second iteration's literal was written over
  `o`'s record on both backends and nothing reported it; a struct-enum literal read its `h`
  back through `o.opt`; a dense `if`-valued and `match`-valued literal and a nested loop the
  same.  loft#1317 had recognised the two-names-one-store shape and paired the buffer's
  forced exit free with the local, and DECLINED the pairing where the local is inner-scoped —
  which is exactly a loop body.  The first cut here was a MOVE at the adopt (reset the
  buffer's slot after the bind): right for the loop, wrong everywhere the rest of the model
  assumes the buffer owns — the owner witness classifies a literal adopt as *not a sole
  mint*, loft#1200's flag likewise, the `??` lift borrows, and 848/810 leaked on native; four
  leaks from one reset, so it was reverted.  The cure with one home is the one the
  CALL-shaped buffer already had, @P378(a)'s `witness_buffer`: an inner-scoped adopter's free
  becomes `OpFreeRefIfDistinct(y, buffer)` — declined while they alias (the buffer keeps its
  store, reuses it in place, frees it once at exit), a real free where the binding moved on
  — now carrying EVERY arm's buffer so a two-arm literal branch declines against both, and
  reached through `adopted_work_refs`, which sees the literal at the tail of each `if` arm
  and value block.
- **A nullable local first assigned inside a branch.**  `needs_pre_init` listed the bare
  heap spellings without peeling `Optional`, so an `S?`, `vector<T>?` or `text?` local got no
  `Set(x, null)` before the branch: the second arm's `Set` was a REASSIGNMENT whose guarded
  displacement free read an uninitialised frame word — a refused free of `0xDEADBEEF`, or on
  the SECOND call of the function the free of whatever live store the previous frame had left
  there (`o.opt` read 0).  Both backends; a one-arm assignment read on the other path was the
  same word.  One peel, at the one predicate.
- **A nullable local first assigned inside a loop body** stayed scoped to the body — the
  hoist `loop_locals_read_after` reads the same predicate — so the read after the loop that
  LOFT.md promises was a use-after-free on the interpreter and an unresolved `var_x` under
  rustc.  The same peel; and a nullable VECTOR's pre-init then needed a lowering of its own
  (`gen_set_first_nullable_collection_null`): the dense arm allocates a store, or for a
  borrowed vector a stack placeholder into the DEP's slot with `x` left unwritten, and the
  untaken path read a poisoned word — absent is the sentinel.  A keyed nullable local is NOT
  routed there: its assignment is `OpReplaceKeyed` into its own store, its null-init
  allocates, and on the untaken path it reads present-and-empty on both backends — what
  absence means for a keyed local is @PLN153's question, recorded and not frozen.
- **A keyed local bound through a `match`** copied the taken arm's fresh store out and
  abandoned it, one per evaluation on both backends, with the `if … else if …` spelling of the
  same arms clean.  `join_source_frees` (loft#1154) licenses the free-source bit per arm, and
  `join_arms` descended an `if` and the `??` block and took any OTHER block as one arm — a
  `match` is a block that binds its subject and then holds the `if` chain.  It now reaches a
  value block's tail.  The matrix's keyed `match` cell found it; the leak predates the walk.
- **Two spellings of `S?` in one binding — filed as loft#1367, not fixed.**  `x = y; x =
  o.opt` (a pointer, then a projection of the tagged `__nullable<S>` field) types `x` by
  whichever assignment parsed LAST and never converts the other: one order frees the caller's
  store, the other writes the TAG byte and reaches neither record; the branch form fails in
  one arm order; a declared type does nothing; the `??` spelling refuses with `__nullable<S>`
  in the message.  `(L-Null-Which)` already picks the pointer for a local, so the cure is the
  conversion at the bind — the store-flavoured junction @PLN153 phase 3 folds, which the
  sibling stream owns; the issue carries every cell so that commit takes `Fixes`.  The first
  matrix's remaining nullable rows (a copy or a mint beside a view, the view first) are all
  this one defect.

**Verified.**  Three guards, each `make falsify`'d at 64437246:
`a-binding-that-adopts-a-literal-buffer-inside-a-loop-frees-it-once` (10 cells: the four
failing shapes, the in-place reference route, the `??` arm, the returned and the reassigned
literal local — falsified with `LOFT_POISON=1` armed, because in plain mode the allocator
hands the stale buffer its own number back on every shape tried and the reclaim lands on a
free store; the guard says so, `falsify.sh` now passes the arena instruments through when the
caller arms them, and the nightly poison sweep is the CI leg that scores it),
`a-nullable-local-first-assigned-inside-a-branch-or-loop-holds-null-on-the-other-path` (19
cells: record / vector / text × both arms / one arm / nested / loop / loop-then-loop, keyed ×
branch and loop), `a-keyed-local-bound-through-a-match-frees-the-arm-that-ran` (4).  Every
neighbouring guard green on both backends under strict stores (1181, 810, 848, 1078, 1013,
1201, loft#1317's captured-store file, 1019, 1140, 1142, 1154, 1157, 981); the corpus IR
census moved 20 of 1241 files (comprehension and file temps whose free became the guarded
form, a `last: SAItem?` that now gets its entry null), every one green on both backends.

**Method.**  The matrix CONFLATED two mechanisms — every `i=1 src=0` row was the loop-literal
face, not the branch face, and only the single-call and no-call probes separated them.  A
fix right for the cell and wrong for the model shows as leaks in four unrelated places at
once; the walk's "find the ONE home" step is what named the existing twin mechanism instead.
And an observable that only an instrument can see is still an observable: the guard names
the instrument, and the falsification tool carries it.  `matrix_axes.py file` on the three
guards (derived, not declared): all three reach both values of A5 nullability (the
literal-buffer guard through its dense and nullable cells) and A9 evaluation count, and the
pre-init guard reaches six of ten A4 statement contexts including `if-arm` and `loop-body`,
hash and vector of A1, callee-return and parameter of A2.  What they do NOT reach: A1
`sorted` / `index` / `spatial` / `tuple` (the keyed `match` cure is kind-agnostic — it is
about the block, not the collection — but only `hash` is measured), A3 `tuple-element` and
`coalesce-result`, A7 `float` / `boolean` / `narrow-int`, and A4 `coalesce-subject` and
`block`.  The axes the previous guards held fixed and this walk moved are the nullable
spelling and the scope of the binding against its buffer's (loop body against function).

#### B7v — `@FR-O-Witness` walked: the caller-side matrix B7u never built, and the fact the emitters read that did not survive the cache (2026-09-05)

Picked by `rule_tags.py dups`: 13 sites.  B7u walked the rule from the DECLARATION side (a local
whose OWN assignments mix ownership).  Its thirteen sites also ask a caller-side question — *"is
a nullable local bound from a call that borrows its argument treated as the heap local it is?"* —
and a cache question — *"does the fact the two emitters read survive the startup cache?"*.  Two
matrices B7u's had not crossed: (1) a nullable local first-bound / reassigned from a callee that
answers its argument, its argument's element, its argument's field, on the plain / early-return /
if-valued / null spellings, dense twin beside each; (2) the same program run COLD then WARM
through the program cache, on both backends, scored on value and strict stores and poison.

**Four defects, one shape** — a nullable local not treated as the heap local it is, none the
mixed-path join the matrix was drawn for:

- **The owner witness did not survive the cache.**  `owner_witness` lived in the IR and in no
  snapshot field, so a warm run served the pre-witness copy arm: the sharp cell of loft#1336
  (`s = a; s = a.next; s = a`) wrote a copy INTO the record `s` viewed and read `b == 7` warm,
  `b == 2` cold, both backends.  `__own_<name>` is now the tenth stored `Variable` field, through
  every codec, and `CACHE_FORMAT_VERSION` is bumped to 5.  A fact the emitters read must survive
  the snapshot exactly as `skip_free` does — that is the one-home statement.

- **A nullable local bound from a borrow-returning call aliased its argument.**  The dispatch
  asks its shape against the bare type; its one nullable arm admitted only a `Join`, so a pure
  `Borrowed` (a callee that ALWAYS hands its argument back) stayed a plain alias, `x: S? =
  keep(a); x.value = 9` reaching `a` while the dense twin copied.  `nullable_join_first_bind`
  now admits a single-witness `Borrowed`; the strips peel `base()`.

- **A `-> S?` callee freed a PARAMETER on its null path** (the caller's store, F-ParamHeap): a
  parameter is no longer a null-arm return source.

- **A record reassigned from a value branch handed up the chosen arm's STORE**: the reassignment
  is lowered to the statement form so each arm's `Set` copies, as the first bind's per-arm lift
  already did.

**Verified.**  Three new guards (`a-nullable-local-bound-from-a-borrow-returning-call-copies-it`,
`a-null-answer-does-not-free-the-argument-the-other-arm-hands-up`,
`a-record-reassigned-from-a-value-branch-copies-the-chosen-arm`) plus the cache guard
`a_warm_run_keeps_the_owner_witness`; each `make falsify`'d at e575a33f.  Neighbours green both
backends under strict stores and poison (`1336`, `1181`, `1202`, `1106`, `1337`).  Corpus IR
census moved 7 of 1241 files, every one green on both backends.

**Method.**  A trap the walk caught in its own work: peeling the var-copy strip through `base()`
to reach the nullable spelling widened it onto a CAPTURED nullable local, whose closure holds the
store at capture — freeing it read `null` through the capture (`c23`).  The strip now excludes a
captured or never-free local (`@FR-L-CapHeap`); `1181`/`1202` are the standing controls, and the
scratch cell `c23` is the one that named the regression before the census would have.  The
disagreement the walk turned on: a `-> S` return copies through its buffer and a `-> S?` return
has no buffer, so the same body delivered its value fresh in one spelling and raw in the other —
`agreement is not correctness` in reverse, one backend's two spellings disagreeing with each
other.  Held FIXED and filed apart: the two-source nullable return (loft#1368) and the
vector/keyed value-branch reassign (loft#1370).

#### B7w — loft#1370 closed, and the parameter rebind the calls oracle never crossed: a vector local bound from a value branch copies what a single bind copies (2026-09-05)

Picked as this branch's own deferred work: B7v fixed the RECORD reassigned from a value branch
and filed the vector/keyed twin apart (loft#1370, `silent-wrong`), and an open issue is fixed
before the PR by its filer.  Matrix: 33 `probe-matrix` cells and one wrong-on-purpose control,
`--interpret` first, then both backends — {`if`, `else if`, `match`, `??`} × {dense, nullable,
null-initialised local} × {integer, text, record elements} × {whole-variable, owned-projection,
call, literal, mixed, index-read, block-tail, parameter-source arms} × {reassignment, first
bind} × {straight line, loop}, the keyed twin, a parameter as the TARGET, and a forward-declared
callee in one arm beside a nested literal (the pass-drift shape).

**Findings.**

- **The filed scope was half right.**  EVERY value-branch bind of a vector local aliased the
  chosen arm, both backends: every spelling, every element kind, the nullable and the
  null-initialised local, projection and mixed arms, parameter sources, inside a loop — and the
  FIRST bind too whenever a wrapper block carried it (a `match`, a `??`), where a plain `if`'s
  first bind copied through the post-parse lift.  The KEYED twin does not reproduce:
  `OpReplaceKeyed` copies whatever the arm, and always did — the guard keeps it as the control.
- **`x = s.v ?? va` viewed `s.v`**, first bind and reassignment alike, where `x = s.v` copies
  (`@FR-B-Copy`: off an owned base a collection projection copies).  The `??` hoists the
  projection into a `__ncc_N` the arm hands back as a compiler temp.
- **A vector PARAMETER reassigned from a variable refilled the CALLER's store in place** —
  statement form, both backends, on the old binary too.  `@FR-F-ParamRebind` names the `p =
  other` spelling in its own text; its oracle (`1290-…`) crosses {struct, struct-enum} × {literal,
  call, local} and carries the vector kind only as the literal spelling, so `OPEN: 0` was exactly
  as strong as that.  The copy lowering asks `@FR-O-Proxy` whether the local owns the store it
  holds, and a parameter's carve-out (no `__vdb_N` of its own) read as "owns".
- **One the fix surfaced:** once the parameter rebind became a var-copy bind, the Tier-0 elision
  rewrote every read of the parameter onto the source — the loop's first-turn read AHEAD of the
  rebind included (`885`'s `rebind_while_reading` answered 24 for 27, both backends).  Its verdict
  says "read-only local" and never asked that the destination be a local; a parameter is defined
  at entry, a definition the def count does not see.

**The one home.**  The vector copy lives in the parser's assignment lowering — the selector
`classify_vec_bind` and the arm that mints or clears and appends — where the record copy lives
in codegen, which is why B7v's record sink sits in `scopes.rs` and this one sits at the
selector: `Parser::sink_vec_bind_into_arms` writes a value-branch bind out per arm and classifies
every tail by the same selector, so an arm gets exactly the lowering a single bind of its tail
has.  What that took: a copy INSIDE an arm always mints (the local carries the join's deps
there, so the proxy cannot say what it holds — a null-initialised local was cleared through its
sentinel); a parameter's first rebind mints (`vec_copy_needs_db`); a `??` hoist is judged by
what it was bound from, and a temp rooted at a compiler variable (a literal's or a
comprehension's own buffer) is not; a block that yields its own buffer is bound WHOLE, so the
buffer stays homed with the binding; a first bind through a wrapper block is declared at the
statement by a null `Set` the post-parse scan elides on a reassignment; a PROMOTED RETURN
BUFFER (`is_hidden_param`) is left to the value form, because the caller receives the value
through it and F-Ret's adopt-or-materialise is its mechanism; and the fact the return-promotion
ladder read off the body — *bound to a branch*, `Set(v, If)`, which keeps a returned local its
own store (`Bind`) instead of renaming it onto the buffer — is carried explicitly for a local
the rewrite sank (`branch_sunk_vectors`), since the shape it was read from is gone.  The
elision now asks `v_is_local`.

**Verified.**  Guards `a-vector-local-bound-from-a-value-branch-copies-the-chosen-arm` (7 test
fns, 33 cells' shapes) and `a-vector-parameter-reassigned-from-a-variable-rebinds-locally`
(with mutate-through, a self arm and a `&` parameter as the controls), each `make falsify`'d at
faa38979 on both backends.  Corpus IR census: 20 of 1260 files moved, two of them the new
guards; every one green on both backends under `LOFT_STRICT_STORES=1 LOFT_POISON=1` and
`LOFT_NATIVE_LEAK_CHECK=1`.

**Method.**  The census earned its place three times over in one walk, each a class the 33 cells
could not see: (1) `is_argument` is also the local PROMOTED to the caller's return buffer, so
"a parameter mints" and "an arm mints" both stole the buffer the caller was to receive (`1081`,
`1321`, `85-*`, `905`, `link-*` — a freed-store read and a leak); (2) sinking a literal arm as
`Set(x, _vec_N)` inside a void block homed the literal's store in that block, freed at its end
while `x` still named it; (3) the elision above; (4) the one the `make ci` wrap suite found
after the census read green — its leak gate runs the corpus with the program-exit accounting
my verification did not: the return-promotion ladder reads *bound to a branch* off pass-1 IR,
the rewrite had removed that shape, so a returned join was RENAMED onto the return buffer on
pass 1, and on pass 2 the rewrite declined the now-hidden parameter and assigned the lift's
temps into the buffer var, which nobody freed (`1081`).  Two passes, two answers: the fact is
now carried by name across them.  Eight files red on the first census, three classes named by
the second, zero on the third, one more from the gate — a differ list from an intermediate
binary is stale the moment the code moves, and a census verified under one leak gate has not
been verified under the other.  The issue's keyed claim did not reproduce: the
first cell of any walk is the filed reproducer, run, not read.  Held FIXED and named: a promoted
return buffer reassigned from a value branch keeps the value form.

#### B7x — `@FR-Col-Group` walked: the three element-level writes through the vector member that reached no chokepoint, and a shape the rule reads one way and the pairing test another (2026-09-05)

Picked over `@FR-O-Move` (16 sites) because every one of those sites lives in the return-delivery
code the sibling's @PLN153 branch is rewriting (+539 in `control.rs`, +834 in `scopes.rs`), while
the group rule's nine sites sit in four files that branch barely touches.  Split into three
questions: **which fields form a group** (`Stores::field`, `Parser::collection_groups`,
`link_shared_nullable_views` — three derivations), **which write routes maintain every member**
(`Stores::record_finish` for adding; the unlink loop `keyed_group_remove` and `loop_group_remove`
each carried for leaving), and **which removal routes free once**.  Matrix: 31 `probe-matrix`
cells and one wrong-on-purpose control, `--interpret` first, then native — a forward-declared
element in both orders, index write (replace / null / replace into a null slot, dense and
`vector<E?>` holders), `remove(i)` (first, last, out of range, in a loop, through a parameter),
three and four members, two groups in one struct, nested under `vector<R>` and `vector<R?>`, a
struct returned from a callee, a struct copy, a nullable vector member, a type alias, a whole
value from a local, a variant-typed holder, a struct-enum element, a JSON round trip, two locals,
a duplicate key through either route, a local record mutated after entry, a struct literal, and
the two-vectors-plus-hash shape; plus nine lint shapes read against the db's answer.

**Findings.**

- **Every element-level write through the VECTOR member left the keyed views stale**, both
  backends, pre-existing (A/B'd against the main-based build): `w.es[0] = E{k:11}` copied INTO
  the record in place, so `by_k` held it under the hash of the OLD key — `by_k[11]` null,
  `by_k[7]` null, `len(by_k)` still 2; `w.es[0] = null` on a `vector<E?>` left the view one
  entry long; `w.es.remove(0)` left the removed key findable and a re-add of it counted twice
  (`len` 3 over 2 records); the same one nesting level down.  Silent every time — a `len` that
  disagrees with its own lookups is a legal reading of a group that happens to hold that many.
  `e#remove` (loft#903) and the keyed removal (loft#900) were the only LEAVING routes covered.
- **The two derivations of group formation agree.**  Nine declaration shapes — forward-declared
  element, alias, variant, `hash<E[k]>?`, `vector<E?>`, three members, two groups interleaved,
  `vector<E>?`, two plain vectors — and `linked-group-apart` fired exactly where the db linked.
  A negative result, now measured.
- **The rule reads a shape one way and the pairing test another** (loft#1375, `D-col-1` OPEN):
  `{ a: vector<E>, b: vector<E>, h: hash<E[k]> }` — each vector links to `h` and never to the
  other, so `h` holds the union and each vector only its own entries (`via a: a=2 b=0 h=2`).
  Contrived, `wa:clean`; which vector HOLDS is a design call, filed for the sibling and left out
  of the guard so its green is not read as covering it.
- **A key write through the vector member is refused with a clear message** (`Cannot write to
  key field E.id — create a record instead`), through `&` too; the same shape the language
  reference documents for the keyed spelling.  A struct-enum element cannot be keyed at all,
  refused at the declaration naming the variants.  Both SEE/SAY-clean.
- **`v.remove(i)` typed `void`** while STDLIB.md and the op both said `boolean`: `ok =
  v.remove(2)` failed with *Cannot format type void*.  Additive; now `boolean`.
- **DATABASE.md carried two paragraphs contradicting their own neighbours** — loft#1152 "still
  open" beside the section describing its fix, and "group formation is still order-dependent"
  beside loft#1158's guard.  Both rewritten.

**The one home.**  `Parser::group_elem_write` (`src/parser/collections.rs`): the element is
bound ONCE to a temporary (`hoist_index_arg` keeps the index single-evaluation), every keyed
sibling unlinks it through `Parser::group_sibling_unlinks` — the loop both removal spellings
carried by hand, now shared — the write runs against the temporary, and a replace ends with
`OpLinkRecord` → `Stores::link_record_siblings`, `record_finish`'s sibling half factored out
(the primary already holds the record; `record_finish` would append it a second time).  Reached
from the three `towards_set` arms (null write, nullable convert, `copy_ref`) and from
`vector_operations` for `remove`.  The temporary is typed as the element PLACE resolves, deps
included — typed from the vector's element alone the native emitter read `found = v[i]` as an
owning bind and deep-copied the record, the unlinks ran on the copy, and the run died in
`store.rs` with a corrupt reference (`@FR-B-Copy` / `@FR-O-NoDiverge`, the interpreter had
passed).  `holder_type` reads the field type an `OpGetField` carries as its third operand and
resolves a vector-element base, so a nested group (and one under `vector<R?>`) is found.
`link_record_siblings` tests `rec.rec == 0` before resolving any store (an out-of-range place
reads null; the sibling's absent-read change makes a store resolved first a panic).  Rule text
extended with the LEAVING clause and `(Col-Group-Dup)` — a duplicate key through ANY member
displaces the older record from the keyed members and leaves it in the vector (measured through
both routes).

**Verified.**  Guard `a-group-element-written-through-the-vector-member-reaches-every-member`
(19 rows, every row its own element type), `make falsify`'d at 2b992851 on both backends.
Targeted suites `store` 262, `parser` 1514, `codegen` 257, `runtime` 181, `scopes` 619 — all
green; fmt, clippy, `cargo check --no-default-features` clean; `index/target_surface.json`
regenerated (97 builtins) and `surface-check` in sync.  Corpus IR census, 1270 files against
the falsify control of 2b992851: 12 structural moves after normalising the stdlib line numbers
the new declaration shifted, 11 of them exactly the `remove` retyping (`drop OpRemoveVector` +
`FreeStack(discard)`) and the twelfth the new guard; all 44 files whose text moved at all green
on both backends under `LOFT_STRICT_STORES=1 LOFT_POISON=1` / `LOFT_NATIVE_LEAK_CHECK=1`.

**Method.**  The first cell of every route was its filed spelling, RUN: the keyed-dup clause was
written only after the keyed route was measured, not inferred from the vector route.  A census
baseline built from a control needs the control's OWN `default/` — the work tree's stdlib
declares an op the old table does not have, and a positional op table would have dispatched
every later op one slot off; and a census that compares source-path comments reports the whole
corpus moved.  Held FIXED and named: `Parser::field_site` (expressions.rs) and
`Parser::keyed_field_site` + `holder_type` (collections.rs) are still two derivations of *which
struct field does this collection expression name*; a merge threads the assign's `parent_tp`
into the removal sites and has no defect behind it today.

#### B7y — `@FR-E-Uncomp-NN` walked: the non-nullable matrix loft#1246 never built, a value-`if` every narrow store mistook for a `??`, and an `else if` chain never held to its own type (2026-09-06)

Picked because its deviations closed five days earlier (D-op-7/8) and its eight sites sit in
`data.rs`, `generation/mod.rs` and `expressions.rs`, outside the sibling's @PLN153 churn.  Split
into three questions: **what is `default(τ)`** (`IntegerSpec::default_value`, one home since
loft#1254, asked by `to_default`, `uninitialised_native_value` and the parser's range guard);
**null or default** (`uncomputable_default`, one home since D-op-8, asked by both range paths);
and **where does an uncomputable result LAND** — the question whose siblings the fixes' guards
never enumerated.  1246 scored the NULLABLE slot in every position and the NON-nullable one only
on the compound path (`n += 10`); 1030 added a `u8`/`u32` field and a `u32` element.  So the
matrix was the non-nullable slot × {plain assignment, reassignment, struct literal, field write,
element write, argument, return} × {`+`, `*`, `-`, `/ 0`, `% 0`, unary `-`} × {`u8`, `i8`,
`i16`, `u16`, `u32`, `i32`, `limit(10,20)`}, hand-computed, both backends — 32 cells, then 60
more once the first defect named the axis it lived on (the SPELLING of the stored expression).

**The compound path is right everywhere** — every `+= -= *= /= %=` cell answers the rule's
default (`0`, or `10` for `limit(10,20)`) on a local, a field, an element, a struct in a vector
and a return, and `i32` answers `null` as C85 decided.  The plain-assignment cells are right by
REFUSAL: `c: u8 = a + b` — `a` and `b` themselves `u8` — is *"cannot implicitly narrow integer
to u8"*, because C85 types the sum `integer`; so are the `match`, the block, the argument, the
return and the literal field.  Two spellings were not refused, and both answered wrong:

- **A value-`if` was a `??` to every narrow store (loft#1379, `sev:high`, `silent-wrong`; in
  2e6a04ba, so on `main` and in 2026.9.0).**  `c: u8 = if t { a + b } else { a }` read `null`
  in a `u8`; `q: integer limit(10,20) = if t { o + p } else { o }` read `null` where `q = o + p`
  answers `10`; and `c: u8 = if k == 1000 { a } else { b }` answered **10** for a TRUE
  condition.  `range_guard_inside_discharge` (@PLN152) recognised the bare-variable `??` —
  which lowers to a plain `if coalesce_not_null(v) { v } else { d }` with no marker — by the
  node alone, so every author's `if` matched: it wrapped the then arm in a checked cast (null in
  a slot with no code for one; the `limit` default lost), range-cast the FIRST OPERAND OF THE
  CONDITION (`(k as u8?) == 1000`), and told the seam the store was discharged, so the refusal
  never fired.  The classifier now asks the BUILDER: `Parser::bare_variable_discharge` accepts
  an `if` only when its then arm is a plain read of `v` and its condition is exactly
  `coalesce_not_null(v)` for `v`'s type.  An author's `if x != null { x } else { 5 }` is not one
  (`!= null` spells `OpNeInt`, the builder `OpConvBoolFromInt`) and is judged as the narrowing it
  is.  `null_discharge_subject`'s looser `If` arm stays, documented as sound on a LEFT-hand side
  only, where no author's `if` can stand.  Baselined: b1ccf0e9 refuses all three.
- **An `else if` chain was typed by its first arm and never converted to it (loft#1380,
  `sev:high`, `silent-wrong`; pre-existing on b1ccf0e9).**  `x: integer = if a { 1 } else if b
  { 2.5 } else { 3 }` printed the float's bits (`4612811918334230528`), `f: float = … else if b
  { 2 } …` the integer's, and 260 reached a `u8` local, argument and return — a field or element
  read `0` because the STORE's width check (984) caught what the parser let through.  `parse_if`
  parsed the chain through a recursive `parse_if` expecting nothing, and kept its type out of the
  join (loft#936/#978: only what it borrows).  Now `parse_if_expecting` threads the enclosing
  then arm's type into the chain's then block, so `parse_block`'s tail conversion covers it as it
  covers the plain else — the literal-fit exemption (`else if k == 2 { 7 }` into a `u8` is
  accepted, as `match` and the plain `else` accept it; an after-the-fact `convert` of the whole
  chain refused it and was discarded for that), the sibling-variant carve-out and the loft#1350
  tuple boxing (both keyed on `arm_of_sibling`, "handed a sibling expression's type", rather than
  on the `else` keyword) and the honest deps.  A `Void` then arm expects nothing of its chain, as
  before.  Baselined: b1ccf0e9 prints the same bits.  **One shape narrowed, measured by loft3
  and relayed the same night:** a STATEMENT chain whose then arm yields a value and whose
  middle arm is a statement — `if k == 1 { 5 } else if k == 2 { println("two") } else { 9 };` —
  compiled before and is now *"expected integer, got void on if"*, on both backends.
  `(F-Block)` allows it: the `;` discards every arm's value and `(F-Drop)` still runs the
  `println`.  It is loft#1382's gate one construct out — the plain `else` twin
  `if k == 1 { 5 } else { println("two") };` was already refused while the mirror
  `if k == 1 { println("one") } else { 5 };` is accepted — because the arm-agreement check
  cannot tell statement position from value position: a top-level statement `if` arrives with
  `Unknown(0)` expected, not `Void`, so `parse_if_expecting` cannot either.  Left as the loud
  side rather than re-opened here: the two silent-wrongs it closes outrank it, and the cure is
  #1382's statement-position fact, which closes both together.  Recorded on #1380 as well.

**Filed, not fixed:** loft#1381 — a statement `if` whose else arm yields a value it discards
(`else { 5 }`) fails rustc natively (E0308) while the interpreter runs it; loud, pre-existing,
`area:native`, `wa:clean` (`else { 5; }`).

**Measured negatives:** the bare-variable `??` cells into a `u8` (`integer?`, an `i16?` and a
`u8?` nulled by overflow, the `ncc` expression subject) keep the author's fallback; a `u8?`
target through an `if` stays `null`; agreeing chains of `u8`, text, vector, enum-variant, tuple
and nullable arms answer the taken arm; the 152 / 1211 / 1212 / 1214 / 1205 / 1246 / 1249 /
1030 / 984 / 1009 / 1254 / 936 / 978 / 1117 / 1103 / 1019 guards are green on both backends.
Guards: `1379` + `1379b` (6 value functions over 6 positions, 10 refusal cells), `1380` +
`1380b` (7 value functions, 7 refusal cells), falsified at 2b992851 on both backends.  Audit
row: `optional` 713→714 / 354→355 (the `tp.base()` in the new predicate).

**A register note.**  `types.md` read `OPEN: 0` over both — `(N-Decl)` and `(I-Narrow)` were
complete and settled the answer; what nobody had re-measured was the code against them for the
`if` and `else if` SPELLINGS of a store.  The third time (types-history.md), and the doc's own
warning applied to itself: complete rules, a register at zero, two live silent-wrongs.

#### B7z — loft#1378 closed: the generic path's own vector-element stride, and the self-reference it could not size (2026-09-06)

Taken from the sibling's filing because it is parser work.  `rewrite_vector_write_triplets`
computed the element stride of a `vector<T>` write for EVERY monomorph body — before it had
found a write to rewrite — through `type_element_size`, a type-alone re-derivation of the
struct's byte size that summed the fields and recursed into `next: reference<Node>?`, so `fn
id<T>(v: T) -> T? { v }` at a self-referential struct was a bare SIGSEGV on both backends and
under `introspect` (confirmed as unbounded: an unlimited stack turns it into a hang).  The
concrete `+=` append already asks the one home for the element's storage type
(`Data::vector_element_type`, loft#624's "every writer AND reader routes here") and the store for
its stride (`database.size(known)`); the generic path now asks the same two and
`type_element_size` is gone.  Guard `1378` moves the return (nullable, dense, a `vector<T>`
that writes), the self-reference (direct, mutual) and the element type the rewritten write is
sized for (`Node`, plain and nested struct, `i32`, `u8`, `i16`, `text`, `float`, two elements
each, both read back), falsified by hand on ffae9ce6 (SIGSEGV → 0, both backends; the crash
predates every cached falsify ref).  **The cell that read `200 0`.**  The stride was one of
THREE disagreeing derivations in the generic path, and fixing it alone turned a wrong read into
an eight-byte write into a two-byte slot: `primitive_setter_call` keyed the element WRITE's
width on the alias def's `forced_size` — `type_elm(concrete)` resolves every integer to the one
`integer` def, which has none — and `wrap_vector_get_val` read every integer through `OpGetInt`.
`Parser::narrow_elm_set` (the concrete literal / append / slice write, loft#1036's home) already
derived the op from `NarrowIntKind`; it is now a wrapper over the free `vectors::narrow_elm_write`,
which the monomorph rewrite calls too, and `narrow_elm_read` is its read twin for a generic
body's `v[i]`.  Measured cell for cell at `u8`, `i16`, `i32`, both backends.  **Filed:**
loft#1383 — a generic instantiated at two integer widths in one program collides into one
monomorph (`is_equal` ignores the width), so the guard carries one generic per width.
**Named residual:** the vector-element STRIDE still has three derivations — `Stores::size` via
`vector_element_type`, `par_elem_size` (collections.rs, the `par` worker) and
`data::element_stack_size` — a walk of its own (`@FR-L-Narrow` is the rule it would start from).

**The flake the gate reported, run down (B7x's r4, `sev:high` had it shipped).**  `make ci`
on ffae9ce6 passed with one flaky: `a-group-element-written-through-the-vector-member-…` r4
(a record into a NULL slot of a `vector<E?>` group member) read `len(by_k) == 1`, reproduced
at 2 seeds in 40 (`LOFT_HASH_SEED=0x0044fd4163d6edde`), deterministic per seed, both backends.
Matrix over the keyed KIND split it: `hash` lost a live sibling under some seeds, `index`
panicked `tree.rs: Item not found` on every seed, `trie` was clean; three live keys lost
exactly ONE, which one changing with the seed; a plain `h[missing] = null` was clean.  Root:
B7x's unlink loop hands the OLD element to `Stores::remove` even when the slot is null — a
record whose key reads as zero and that no view holds — and `hash::hash_rec_pos`, probing for a
record the table does not hold, wrapped to the home bucket and answered it, which `remove`
zeroed.  Two homes: `Stores::absent_nullable_record` is the one null test both halves of
`@FR-Col-Group` now ask (`link_siblings` on ENTER already skipped a non-`Some`; `Stores::remove`
on LEAVE did not), and `hash_rec_pos` answers `Option`, stopping at the first empty slot, so a
remove of an absent record is a no-op.  Guard `a-null-element-of-a-linked-group-leaves-nothing`:
0 failures in 60 seeds, both known seeds pass, falsified by hand on ffae9ce6 (exit 101 → 0,
both backends).  A lesson for the guard convention: a hash-seed-dependent cell needs a
deterministic sibling (the `index` cell) to be falsifiable in one run.

#### B8a — `@FR-O-Detach` walked: the literal that was never held to `I-Comp`, the native join that declined its detach, and the brace beside it (2026-09-06)

**The split.**  Eight sites, one static question — *does the value being assigned READ the
binding?* — with one home already, `Value::reads_var` (#330's predicate): four sites ask it
(the interpreter's reassignment, native's Join reassignment, the parameter rebind, the witness
classifier), the struct-literal lowering hoists unconditionally, the accumulator detach
sequences after the whole statement.  Three admissible placements, each with a home: hoist the
reads into temporaries (parser), defer the free past the assignment (both emitters), release by
store identity after the `Set` (the witness).  The third question is the runtime one — *is the
displaced store the new one?* — `Stores::free_displaced` on the interpreter and three
hand-spelled `_old.store_nr != place.store_nr` tests plus a `PASSTHROUGH` const on native.
Nothing disagreed among the eight; the yield was step 4.

**The matrix.**  37 cells (`scripts/probe-matrix`, one control): 14 binding kinds (owned
local, nullable local, heap parameter, nullable parameter, `&` parameter, struct-enum local,
vector local and parameter, keyed field, text, vector field, an element's vector field, a
captured collection, a witnessed local) × 20 right-hand-side shapes (a call reading a field,
the binding passed whole, a callee returning its argument, a literal nesting it whole, a
projection of itself, a value-`if` reading it in the condition and as an arm, `??` with it as
the fallback, a method on itself, a block, a nested call, a closure reading it, a read through
a VIEW, a vector literal of its own elements, `map`, a loop, passed twice, a fn-ref callee).
70 backend-cells green after the fixes; one cell (a witnessed local bound from a tagged
projection) is loft#1367's shape, refused here and measured passing on the sibling's tree.

**Fixed.**  (1) The vector literal — sixteen spellings wrong on both backends: local `=`,
typed local, parameter, struct field, `+=` (a `len` read appended the growing length), a
struct element, a text element, a bound-guarded read, a loop.  The comprehension had been
walked three times for the same sentence (loft#1194/#1195/#1196) and the literal never —
`create_vector` inserted the `=` repoint and `clear_vector_field` the field clear at the head
of the build, before the element reads.  One home now: `Parser::snapshot_read_destination`,
which the comprehension's deferred route calls and the literal asks; the two detach sites
insert after it.  (2) `--native` declined a value-`if`'s displaced free (`owned_ref_reassign`
listed calls, inserts and blocks): one leaked store per execution of `s = if c { mk(7) } else
{ s }`, interpreter clean — `(O-NoDiverge)`.  (3) The `match` spelling did not compile
natively: `output_if_inner` decided the arm's opening brace on a peeled value and its closing
brace on the bare one.  Guards `a-vector-literal-reads-what-its-destination-held` (16 cells)
and `a-join-reassignment-whose-other-arm-is-the-binding-frees-and-compiles` (6), both
falsified at 6f9c0886; the leak channel by hand on the baseline (`LOFT_NATIVE_LEAK_CHECK=1`,
1 → 0).

**Filed.**  loft#1388 — a captured struct local's reassignment from a call retains the
displaced store (8-cell matrix: struct × {inline, stored} × call leak, a loop 1:1, a vector
inline leak; the literal cells clean; two of the three shapes new since b1ccf0e9 with
loft#1324's change of who frees the build-time store).  This is the rule's own forbidden third
option — *declining the detach* — spelled as `owns_displaced_store`'s `!is_captured` veto, a
per-binding answer to a per-store question.  loft#1389 — `e: Sh = Circle{r: 1}` gives `e` a
dep on ITSELF (`change_var_type → depend`, twice), so a join reassignment reads it as borrowed
and leaks; the struct twin and the call-bound enum are clean.  loft#1390 — a variant literal
and a binding of its enum type do not join (`cannot unify: Circle and Sh`; two variants do,
loft#1117).  loft#1391 — two destinations the snapshot cannot name: a field reached through
an element, a captured collection.

**Convergence.**  The literal fix closed a class — sixteen spellings across three
destinations through one home — and its residual is exactly the two destinations
`field_place` cannot name.  The native fixes closed the `if` and `match` spellings together.
The closure leak BRANCHES: three leaking shapes with a different mechanism per kind (a stored
closure's record adopts the build-time store; nothing adopts an inline argument's), so it is
filed for the closure model rather than folded in.  No shared root among the three fixed
defects, and none among the four filed.

**Whether the rules covered the cells.**  `(O-Detach)` settled every fixed cell.  `(I-Comp)`
had the sentence and not the word: it named the comprehension, and now names the literal
(D-iter-4).  `(O-NoDiverge)` settled the native two.  The variant/enum join (loft#1390) is a
typing question `types.md`'s join rules do not spell for a variant against its own enum — a
gap in the definition, not in the code.

**Residuals named.**  The comprehension's field / `+=` own-buffer route is a second ACTION
beside the snapshot for one question; `create_vector`'s `__trail_tmp` materialisation of a
concat operand is a third copy of the snapshot idea; native's same-store test has three
inline spellings beside `PASSTHROUGH`.

#### B8b — `@FR-O-Owner` walked: one question with four namers, and the payload view none of them could see (2026-09-06)

**The split.**  The rule's eight citations ask one question — *who owns this store* — and the
walk's yield was not in them.  Its channels are what a matrix can vary: ZERO owners is a leak
(`LOFT_STRICT_STORES`, `LOFT_NATIVE_LEAK_CHECK`), TWO owners is an alias visible through a
mutation or a double free (`LOFT_POISON`).  20 cells over the axes the rule's own guards never
crossed: ownership THROUGH a container in and out (append, keyed insert, field assign, element
assign, nested vector, two containers, a field displaced in a loop), the NRVO buffer under a
loop / a conditional / an early return of a mint / an early return of a parameter, a view whose
BASE is replaced, and a `&` local link.  Both backends, both leak channels.

**The negative result, 17 cells.**  Every container in-and-out cell copies exactly as
`(B-Copy)` says and frees once; the four NRVO shapes are flat over 20 and 50 calls with no
leak on either backend; a returned element view and a returned field view are both copied.
That half of the rule is measured, not assumed.

**Fixed — one question, four namers, and a shape none of them could see.**  A struct-ENUM
payload projection does not name its subject directly: `sh.inner` lowers to `OpGetField(if
<tag == Holder> { sh } else { OpNullRefSentinel() }, …)`.  Every chain that peels a projection
to its container variable stopped at that `if`, so `(B-View)`'s materialise clause did not
apply to a payload view live across its subject's reassignment — the interpreter read the NEW
subject's bytes at the payload's offset (`x.a` = `0` where the payload said `1`), `--native`
answered the old payload by another route, and nothing was said on either backend, where the
plain-struct twin has copied and warned since @PLN130 F8.  Two of those chains were
byte-identical and each documented as *"mirroring"* the other, which is what made one peel
into two: `use_analysis::projection_container_var` is the home now, beside the
`is_projection_op` list whose own doc already named its four readers.  Three sites fell out of
the same peel — `established_stores` did not count a struct-enum reassignment at all (the
literal hands the variable a BLOCK tailing in a work-ref, and only the block's own
`OpDatabase` was read, which names the compiler temp); the materialise arm made a binding an
owner without asking `@FR-O-Override`, the veto its var-copy sibling already asks, which
leaked a record per call for a never-free `_mv_` binding until the guard was added (my own
first cut, caught by A/B against the baseline); and the ORACLE classified a payload view
`Owned` — the over-free direction its own caveat names — whose user-visible face was the
`lost-write` warning telling an author a landing write was lost, on the tier that gates a
library's CI.  Guard `a-payload-view-materialises-when-its-subject-is-reassigned` (6 cells,
three controls), falsified at 5f4ac074; oracle Check A clean over the 1247-file corpus and
the fuzz corpus; `binding-history.md` D-bind-19.

**The peel's own false positive, and what caught it.**  The first cut read the variant check
as *"one arm names a variable"*, which is also the shape of `a?` discharging a nullable
parameter (`if a.rec != 0 { a } else { <mint> }`).  Claiming that one turned a generic's
return delivery from an adopt into a copy nobody freed — one leaked record per call.  No
targeted suite saw it: `--tests` does not leak-check, the file's own values were unchanged,
and only `wrap.rs`'s per-file gate under `make ci` reported it.  The test now names the OP the
other arm calls (`OpNullRefSentinel`), which is the same lesson loft#1379 taught three days
earlier from the other side — a lowering is recognised by what it BUILDS, never by node shape
alone.  Two independent instruments were needed to place it: the full gate to see it at all,
and an IR diff against the baseline binary to name the delivery that had changed.

**Filed.**  loft#1392 — a `&` link to a VECTOR or TEXT local reads the store the source held
at the bind, so a rebind of the source leaves the link stale on both backends (the struct
spelling is loft#1371's, right on the sibling's branch; the scalar link follows its source, so
the kinds disagree).  loft#1394 — the payload binding written INSIDE a `match`/`is` arm whose
subject is reassigned there is still invisible, because the walk handles a `Set` whose
right-hand side is a value branch whole, through the `leaf` arm its own doc calls
*"deliberately coarse in both directions"*; reaching it needs the walk's ordering model rather
than another classifier, and its field-subject twin needs `field_place` (loft#1391's shape).
loft#1395 — the `lost-write` false positive, filed because it is user-facing and its fix is on
an unmerged branch.

**Convergence.**  One root under all four fixed sites — the unpeelable variant check — and the
route table SHRANK as it went: the peel closed the direct spelling, the establishment test and
the oracle in one move, and what remains (loft#1394) is a different mechanism in a different
part of the walk, not a fifth namer.  The `&`-link finding is unrelated and was filed rather
than folded in.

**Whether the rules covered the cells.**  `(B-View)` × `(B-Disturb)` settled every fixed
cell — the payload spelling was never in doubt, only invisible.  `@FR-O-Oracle`'s caveat
predicted its own defect in words (*"a projection local is mis-classed `Owned`"*) and had no
site enforcing the prediction.  `(B-Ref-Alias)` covers loft#1392 and the code disagrees with
it per KIND, which is a deviation rather than a gap.

#### B8c — loft#1394 closed: a `Set` whose value is a branch is walked in the order it runs (2026-09-06)

The residual B8b filed, taken up when the sibling checkout measured that its OTHER half — the
place comparison — was not what it needed.  Thirteen cells over the position of the bind (top
level, inside a `match` arm, inside an `is` capture, inside an `if` arm, an arm's TAIL), the
container kind (struct-enum payload, plain struct field, vector element), the disturbance
(reassign to another variant, to the same variant, overwrite a field's place, reassign after
the statement) and the execution count (once, a loop).

**The rule settled two of them before any code moved.**  The `w.st = Empty{…}` row of the
filed issue is NOT a defect: `(B-Disturb)` says in as many words that overwriting a place is
not disturbing it, so a view of it survives and reads what is there now — 0 on both backends
is the answer the rules give, and the issue body was wrong to list it.  What that row exposes
is a payload binding outliving its variant, which is loft#980's class reached through the one
spelling loft#980 exempts (filed apart, **loft#1397**).  And the LOOP cell's expected value was
my own miscalculation: the second pass legitimately reads what the first pass left, and a fix
that froze the materialised value would have been wrong.  Both corrections came from reading
the rule rather than the run.

**Fixed.**  The walk read a `Set` with a value-branch right-hand side WHOLE, through the `leaf`
arm its own doc calls *"deliberately coarse in both directions"* — right for a form whose
internal order is unknown, and a `Set`'s is not.  It is now walked in the order it runs: the
value's statements, then the target's own establishment at the point the slot is written, then
the target recorded (`leaf`'s third step split out as `record_target`).  Two facts had to
travel with the deps for a copy to actually happen: a materialised view stops being a view, so
its never-free mark is lifted; and a binding carrying no deps is admitted beside one naming the
container, which is what the `is` spelling of a payload capture carries where its `match` twin
carries a dep.  **The hole was not enum-specific** — a plain struct view bound in an `if` arm
was wrong on BOTH backends, which the filed body did not say.  Guard
`a-view-bound-inside-a-branch-arm-sees-its-containers-reassignment` (9 cells, four controls),
falsified at 6c09de23; `binding-history.md` D-bind-18.

**The widening I measured, and backed out.**  A view that IS an arm's value
(`x = if k > 0 { h.inner } else { … }`) can be named by both halves — the walk's record site
and the deps strip — through a branch-aware lookup, and I built it: the walk then reports the
view and the deps are stripped.  It is unsound alone.  The emitters ask
`container_element_base`, which answers `None` for an `If`, so nothing copies, and a binding
whose deps are stripped without a copy becomes the OWNER of a store it only views — its
scope-exit free then names the container's store, which is loft#778's class.  Backed out whole
and filed as **loft#1396** with the diagnosis, because the cure is a per-arm materialise in the
emitters rather than another name.  The same paragraph is why the branch peel stayed out of
`projection_container_var`, which the oracle reads.

**Convergence.**  Each fix in this walk closed a class and the route table shrank: B8b's peel
closed the naming, B8c's ordering closed the position, and what is left (loft#1396) is a
different layer — the emitters — rather than a fourth namer.  Three of the four things filed
across B8b and B8c are shapes the RULES settle and the code does not yet reach; one
(loft#1397) is a rule that settles the value and a diagnostic that does not exist.

#### B8d — `@FR-O-Proxy` walked: the rule whose split was already enforced, and the spelling its checker could not see (2026-09-06)

The most scattered rule left — 50 citations across twelve files — and the one whose walk looks
finished before it starts: `scripts/o_proxy_check.py` already classifies every site by WHICH of
four questions it asks (alloc / copy / free / oracle), gates that a site freeing on the proxy
consults `@FR-O-Override`, and reports clean.  Step 2 of the walk is done and enforced, which
moves the whole yield to the method's own warning: *a rule's own CHECKER can have a classifier
hole, and it reports that as compliance.*

**The hole.**  The checker matched one spelling, `depend().is_empty()` on a single expression —
41 of them in `src/`.  The same read written across two statements (`let deps = <…>.depend();`
… `deps.is_empty()`) was invisible, and the tree uses that form twice: `ownership_cfg.rs`'s
Check B and `control.rs`'s arm-return free.  **Both are compliant**, so the hole had cost
nothing yet — which is exactly what makes it worth closing before it does, and is the negative
result this walk mostly produced.

**Three changes, each measured in both directions.**  The checker learns the aliased spelling
(function-scoped, because a name means nothing outside the function that bound it — file-scoped
first, which read unrelated `xs.is_empty()` calls as ownership questions and inflated the census
from 40 sites to 74).  It learns to read ENCLOSING guards: the newly-visible `control.rs` site
does consult the override, from an `if !skip_free(local) {` wrapping its whole block, and
reading only the statement reported that correct code as a violation — a false positive the
widening created and the same widening had to fix.  And the site now declares its question,
which is the checker doing its job on a site it could not previously see.

**The true-positive direction, on the new spelling.**  Removing the enclosing veto makes the
widened checker fire on exactly that line; restoring it (by the inverse edit, from a copy — never
`git checkout`) returns it to clean.  A gate that has only ever been green is a claim about its
classifier, and this one is now measured in both directions on the form it just learned.

**What the walk did NOT find.**  No site reads the proxy by a third spelling: `depend().len()
== 0`, `borrow_deps().is_none()`, `deps.first().is_none()` and the rest return nothing across
`src/`.  The one `heap_dep().is_none()` read (`protectable_ref_args`) is a different question —
*does this TYPE carry a store* — and is not an ownership proxy at all.  The four-way split
holds: 28 positive sites, 9 reaching a free, every one of those consulting the override.

#### B8e — loft#1396 closed: the naming through a branch, and the copy per ARM (2026-09-06)

The shape B8c backed out, taken up rather than handed over because the measurement and the
formal reading were already here.  Two gaps, and the second is why the first could not be
closed on its own.

**Naming.**  The walk that names views to materialise asked which container a `Set`'s value
projects from, and asked it of the whole `if` — which projects from nothing.  So
`x = if k > 0 { h.inner } else { mk(0) }` was never named, and nothing downstream had anything
to act on.  `scopes::value_view_container` now looks through a branch: any arm's container,
none where two arms name different ones, arms that mint ignored rather than disqualifying.

**Copy.**  B8c's widening stripped the named binding's deps — and that is measured WRONG,
which is the entry's substance: no emitter has a whole-statement copy for a branch-valued
right-hand side (`container_element_base` answers `None` for an `If`), so the binding became
the owner of a store it only views and its scope-exit free named the CONTAINER's store.
`(O-Complete)` says the fact is per binding and per PATH, and that is the form that works:
only the PROJECTING arm needs a store of its own, so `Scopes::arm_bind` — the arm-lift
machinery that already binds a call or a variable tail into a `__lift_N` — gains the
projection case, gated on the walk having named the binding.  The minting arm keeps its own
store, an undisturbed arm keeps aliasing, and nothing re-derives a copy: the temp is bound by
the single-bind lowering, which is `arm_bind`'s whole contract.

**What that closed for free.**  The REASSIGNMENT spelling was right on the interpreter and
wrong on `--native` — an `(O-NoDiverge)` split I had filed as a second, separate half needing
the join-own machinery.  It needed nothing: once the arm materialises, both backends agree.
A cure at the right layer closed a divergence that looked like its own defect from the layer
above.

**Verified.**  The 15-cell ordering matrix, this session's four earlier guards, and a new one
(6 cells, two controls: an UNDISTURBED arm that must still alias, and a loop whose every pass
reads the container as it then stands), falsified at bd629983.

⚠ **The alias control is TAIL-SPECIFIC, and stating it without that qualification cost a
correct fix its credibility.**  Every cell in that guard has a RECORD tail, where `(B-View)`
makes a projection a view that aliases.  A COLLECTION tail is the opposite — `(B-View-Base)`
puts it on `(B-Copy)`, so the write reaching nothing is the documented answer, in the branch
spelling exactly as in the plain one (measured here, both backends, on a branch carrying
neither half of the joined tree's collection work).  A cell list of mine for loft#1399 said
*"an undisturbed arm must still alias"* with no tail named, the sibling checkout read it as
written, and a correct fix looked over-wide for several minutes.  The general form, now in
TESTING.md: the control for *"did this fix copy too much?"* is not the same cell for every
element type, because what a PLAIN bind already does differs by type — so the comparison that
settles it is the plain spelling of the SAME tail on the SAME build.

**Not closed, and not this.**  The chained spelling — `t = dv.tiles;
prev = if … { t.proto } … ; dv = …` — stays wrong, and the control that says so is that the
SAME chain without a branch is equally wrong: that is loft#1393's view-of-a-view, which the
sibling checkout has fixed on its own tree and this one does not carry.

#### B8f — `@FR-Col-Remove` walked: the removal that keeps what the element owned, and the leak that was holding a silent wrong together (2026-09-06)

Picked because the rule had **zero citations** — `collections.md` names four spellings for
"delete one element" and no code site said which of them it was enforcing — and because
neither stream was in collections.  Split into the questions its sites ask: which slot does a
spelling name (by INDEX vs by RECORD), what happens to the OTHERS (dense renumber vs keys
untouched), what happens to what the element OWNED, and where the loop cursor lands.

**What holds.**  Most of the rule is enforced and the walk is the receipt.  `c[key] = null`
removes by key and leaves every other key reachable on all five keyed kinds; a linked group is
one record set through every route (remove through the vector member, through the keyed member,
or through a loop — all three leave neither); `v.remove(-1)` counts from the end and an
out-of-range index answers `false` and changes nothing; `for x in v { x#remove; }` visits every
element exactly once, forwards and under `rev`.  A 27-cell value matrix over container kind ×
child kind × spelling × position is green on both backends.

**The instrument.**  A leak inside a live store is invisible to `collect_store_leaks`, and the
allocation profiler is `--interpret`-only and reports BYTES.  `store_memory()` already reports
`records N`, which is an exact integer, works on both backends, and is readable from loft — so
the assertion is *a constant population costs a constant record count*, which is the invariant
itself and is allocator-independent.  Two runs of the same loop at different N, compared.

**loft#1402 — a vector removal never releases what the element owned.**  Same element type,
2000 add-then-remove cycles, final population 0: `v.remove(i)` and `#remove` hold 2004 records,
`sorted`'s `#remove` 2004, while `sorted[k] = null` holds 4, `hash[k] = null` 6, `index`'s
`#remove` 3, and the linked-group route 4.  One record retained per removal, without bound;
`--native` confirms it 84 MB above the flat baseline at 2M cycles.  The boundary is exactly
`remove_vector_at`'s unlinked branch, which shifts the bytes and calls no `remove_claims` —
while `remove_owned`'s inline branch, the by-RECORD twin, does.  Only the element's CHILDREN
leak: a scalar-only element is flat.

**loft#1401 — and the leak is what was holding a silent wrong together.**  The cure is one
call through `vector::get_vector`, which already maps an index to an element and already
answers `rec == 0` for exactly the indices that remove nothing.  It made every route flat and
kept all 27 value cells green — and broke `445-generic-tree-walk`, which was right to break:

```loft
gd_cur = gd_stack[gd_n - 1] ?? gd_root;   // NOT materialised
gd_stack.remove(gd_n - 1);
gd_order += [gd_cur];                     // ... still a view of the removed element
```

A projection discharged with `??` escapes `(H-Materialise)`.  The plain `c = v[1]` materialises
and says so; `c = v[1] ?? Box{n:0}` stays a live alias and says nothing — so after
`v.remove(0)` it reads **3 where its element held 2**, on both backends, and a write through it
still reaches the container while the advice that promises otherwise never fires.  `(N-Index)`
types `v[i]` as `τ?`, so `??` is the discharge the language REQUIRES for a non-null binding:
this is the ordinary spelling.  Both filed rather than fixed, in that order — nothing may
release the element's children while such a binding is still a live view of it.

**Two cures built, measured and backed out** (recorded on #1401 so they are not re-derived).
`value_view_container`'s `Value::Block` arm asks the tail; a `??` lowers to a block whose tail
is the `if` the discharge became, whose arms name the ncc temp and a fresh default — two
different names, so the branch arm answers `None`.  Taking the block's own RESULT TYPE instead
(which already reads `{#ncc(2):ref(Box)["v"]}`, and which `lift_view_deps` reads one function
below for the same question) makes the walk register the view — `LOFT_DEBUG_F8=1` prints
`c(Reshaped)` — and changes no value.  Making the strip site ask the same namer it does (it
gates on `base_container_var`, a DIFFERENT namer from the walk's) makes every gate pass —
instrumented, the `??` case reads `tp_ok=true in_walk=true namer=Some(0) deps=[0]`, identical
to the plain case but for the RHS node kind — and still changes no value, because no emitter
has a copy for a block-valued right-hand side.  That second one is worse than inert: the advice
then fires and ASSERTS "writes through `c` no longer reach `v`" while the write still lands.
The same conclusion B8c and B8e reached for a branch-valued RHS, one node kind over: naming and
copy land together or not at all.

**Fixed here.**  The refusal that told a `trie` author their loop was "hash iteration".  Three
kinds take the snapshot substitution — `hash`, `trie`, `spatial` — and share the one scratch
variable, so the message spelled for the hash also prescribed `hash[key] = null` for a
collection the author never wrote.  It now names the kind, recovered from the scratch's own
deps for a local and from the struct's one snapshot-walked field for `for e in b.data`, and
stays kind-neutral where two such fields make it undecidable rather than guessing.  A `spatial`
gets its own cure spelling, since its key is coordinate axes.  The message is a pinned surface
— `tests/issues.rs`, `the-reference-quotes-its-refusals-word-for-word.loft` and CAVEATS.md all
quote it — and all three moved with it, which is what that script exists to force.

**Both are CLOSED now, in the order the walk said** (loft#1401 then loft#1402), and the
closing found more than the walk had. loft#1401 was FOUR holes, not the one cure the analysis
above scopes: the naming (a discharge block's tail names the temp it hoisted, so a tail is now
resolved through the block's OWN bindings), the per-arm copy, `is_value_branch` (a `?? return`
tail is an unconditional `Var` rather than an `if`, so that arm never reached the copy at all),
and `OpGetVectorNullable` not counting as a projection for the naming question — it meets
`is_projection_op`'s criterion and is deliberately off that list because the deps PROXY strands
a store on it, which is a fact about the proxy and not about the notion. A fifth cell, a
binding assigned twice, needed the `multi_assigned` bail lifted for a NAMED binding: that guard
is about the type-level dep list, which the lift already declines to rewrite for one.

⚠ **And the first cut regressed loft#1399**, which is the paragraph above earning its keep from
the other side. Letting a block tail resolve through its own bindings also let a `[]` MINT arm
name the hidden `__vdb_N` it reads its own store out of — a projection by every structural test
— and two arms naming different containers name none, so the whole binding stopped being a
view. A place inside a compiler-generated container is not a place any disturbance can name;
`resolve_view_root` already stopped at one for exactly that reason. **The corpus caught it and
no targeted suite did**, which is the same sentence this section already ends on.

The matrix that closed loft#1401 left THREE more silent-wrong cells, all of them failing
IDENTICALLY in their plain spelling — so none was a discharge defect, and each was its own
finding: a `sorted` removal renumbers positions (`Col-RemoveDense`) and was not recorded as a
disturbance, where `hash` and `index` are measured correct; a removal whose container is
reached through a FIELD was not recorded either; and a value branch whose two arms view
DIFFERENT containers named neither.  **All three are closed** (D-bind-25/26/27), and the shape
they share is worth the line: the VIEW side of `(B-Disturb)` had been made precise three times
over — through a branch, through a chain of views, through a discharge block — while the
DISTURBANCE side still answered whole variables from a two-op list.  A rule enforced by two
walks that must MEET is only as good as its shorter half, and every fix that widened one of
them left the pair further apart.  The disturbance side now answers places too, and the advice
reads its container off the walk's own answer instead of re-deriving one from the right-hand
side — which was a restatement before more than one place made it a wrong answer.

**The lesson.**  A leak fix can be load-bearing for a silent wrong.  This one reads as
obviously right in isolation, has a one-line cure at a chokepoint the rule names, passes 262/262
on its subject suite and a 27-cell matrix — and shipping it alone would have turned a wrong
answer into a use-after-free.  What caught it was running the full corpus rather than the
targeted suite, and the failing test was not a guard for any of this: it was a generic tree
walk whose author had simply written the ordinary spelling.

#### B8g — `@FR-Col-Order` walked: the rule that stated the opposite of the rule it cites (2026-09-06)

Picked for the same reason B8f was and one more: `@FR-Col-Order` had **zero citations**, and
`collections.md` calls it *"the divergence-prone rule (interp store-walk vs native emitted loop)
— the whole reason the area needs pinning"* while its own conformance plan still listed it as to
be pinned.  A rule that names itself the risk and has neither a citation nor a guard is the
cheapest thing on the queue to be wrong.

**It was wrong in the DOC, not the code.**  `(Col-Order)` read `hash → UNSORTED bucket walk (no
key order) — the C-Order decided edge`, and the edge `concurrency.md (C-Order)` actually decides
is the other one: the SEQUENTIAL walk is key-ordered and only the `par` walk gives that up,
*"because the parallel queue has no use for key order"*.  So the rule contradicted the rule it
cites as its source, three lines under a sentence claiming to generalise it.

Measured, both backends: a `hash<E[id]>` filled 49 down to 0 iterates `0,1,…,49`; a
`hash<E[k]>` on text iterates alphabetically; the same collection under `par(…, 4)` comes out
`9,2,8,1,7,6,0,10,11,5,4,3`.  The parser builds the ordered snapshot that makes it so — the
`hash_scratch` in `parse_for`, an O(n log n) key sort for a hash and nothing for a radix, which
is already ordered — and LOFT.md and STDLIB.md both describe it (*"hash iterates via its
internal ordered index"*).  So the code, `C-Order` and both user-facing docs already agreed and
one line dissented: a transcription inverted in one place, not a rule the code had drifted from.
That distinction is what makes correcting the doc the right move rather than a violation of
*"the code changes to match the rules"* — there was no rule here to change, only two copies of
one, and the copy disagreed with its original.

**What the walk pinned.**  `a-collection-iterates-in-the-order-its-kind-defines.loft`, 9 cells
over all six kinds, green on BOTH backends — which is the half of the rule ("identical on both
backends") that nothing had ever asserted.  Every cell inserts in an order that is not the
iteration order, so a walk returning insertion order fails rather than passes by coincidence,
and the 50-key descending cell is the one that cannot come out ascending by luck.

**The Morton convention was measured, not assumed.**  A `spatial`'s curve is stated as
"Morton / Z-order" and nothing said which axis takes the low bit of each pair — a fact a guard
has to have, since the two conventions give different sequences.  Four unit-square points name
it outright: the walk answers `(0,0) (0,1) (1,0) (1,1)`, so the SECOND axis is the low bit and
the first-declared axis is the major one.  The 4x4 cell then follows by hand — codes 0, 6, 9,
10, 15 — and matches the measurement exactly, which is the check that the convention read off
the small case actually explains the larger one.

**A `@falsified-at: none`, and why that is the honest answer here.**  This is a conformance pin
rather than a regression guard: every cell passes on every build measured, because what was
broken was the doc.  The harness was still shown able to fail it — expecting the other
interleave fails exactly the discriminating cell and leaves the other eight green.

**The audit row moved, and the full gate is what caught it — again.**  `snapshot_kind`, the
helper B8f added to name the collection kind in the `#remove` refusal, discriminates on `Type`
variants, so `quality_optional_table_matches_the_audit` went red at 728 → 729.  The audit's
question was the right one to be asked — and my first answer to it was wrong.  I peeled with
`.base()`, reasoning that `?` is a marker over the same storage; the sibling checkout put the
site in the OPAQUE column instead, and measuring settled it their way twice over: a nullable
collection cannot be iterated at all, so a `τ?` never reaches the question, and a nullable
SIBLING field is not a candidate for *which* field the loop is over, so peeling counted it and
made `{ data: hash<E[k]>, spare: hash<E[k]>? }` answer "this collection" where it can answer
`hash`.  The row lands 729 · 364 · 5 · **360**.  A peel that "cannot hurt" hurt, and the cell
that showed it took a minute to write.  Three walks running, three audit-row moves, three times
the full gate and never a targeted suite.

#### B8h — the iteration surface's own refusals: a nullable source, and the audit entry that named its cure (2026-09-06)

Not a rule walk — a question from the owner, and the kind worth following because it starts
from *"why is this refused at all?"* rather than from a symptom.  Why can a nullable collection
not be iterated, when returning nothing for a null source is the obvious reading?

**The refusal is right, and the capability already exists.**  `for e in h?` and `for e in h ?? []`
each give an absent collection ZERO iterations — measured, both spellings.  So the behaviour the
question asks for is there; the language only requires it to be said, which is the same
`(N-Coal)`/`(N-Default)` discharge `v[i]` needs.  A loop that accepted a null source silently
would be the implicit unwrap `types.md` rules out everywhere else (*"there is NO `τ? ⤳ τ`"*).

**The DIAGNOSTIC was the defect.**  It said *"cannot iterate over `vector<integer>?`; expected
vector, sorted, index, hash, text, or range"* — a list that recites the kind the author had
picked correctly — and never named the one character that fixes it.  Same class as B8f's
`#remove` refusal telling a `trie` author their loop was "hash iteration": a message that
misdescribes what was written and prescribes a cure for something else.

**And the second line was an entry already written down — TWICE, with opposite verdicts, and
the second one is why this needed care.**  The shape also reported *"Unknown in expression type
`vector<integer>?`"*, twice, from the element-type resolver.  `@PLN25`'s dn1 audit has a
NEEDS-FIX row for exactly that site — *"`for x in nullable` misses Text/Integer arms → peel
in_type"* — and, further down the same document, the verdict that **dropped it**:
*"`for x in <τ?>` — NOT A BUG (dropped) … Peeling `for_type`/`iterator` only routed `text?` to a
text-char-iteration path that PANICS … → reverted."*  Reading only the row would have re-walked
into a crash.

What makes peeling safe here is that it is only HALF of what was reverted: `for_type` peels, so
the element type resolves and the duplicate line goes; `iterator` does NOT peel — it gains a
REFUSAL — so the text-char path that panicked is never reached.  Measured on `text?` null,
`text?` present, and `text ?? ""`: a clean refusal, a clean refusal, and `a,b,c,`.  Five errors
become three, the informative one first, and the optional audit gains a PEELING site — the
direction that table exists to drive.

⚠ **The message names the inner type's OWN default.**  `text?`'s is the empty text, so its cure
is `?? ""` and not `?? []` — telling a `text` author to write `?? []` would have been a second
wrong cure inside the message written to fix the first one, which is the failure mode B8f's
`#remove` refusal already demonstrated once.

**A second recollection checked, and it holds.**  The owner also recalled that iteration used to
STOP at a null element and thought it was resolved.  It is: `vector<E?>`, `vector<integer?>` and
`vector<text?>` each visit every element, deliver the null as a value rather than a terminator,
and `x == null` inside the loop still tells a null element from a real zero (`rec0, NULL, rec7`
where `x?.n` reads 0 for both).  Worth measuring rather than assuming — a resolved bug is a
claim about a build like any other.

#### B8i — `@FR-I-NullSrc` walked: the formal line held and its own gloss reached one case past it (2026-09-07)

`iteration.md` is one of the two docs that are **100 % uncited** — 11 rules, not one named by any
code site (`matching.md` is the other, at 23).  It is also where B8h should have looked first: it
carries a rule about a NULL SOURCE, and B8h had just spent a session improving a refusal for one.

**The rule holds.**  `(I-NullSrc)` says `for x in nullref { body }` runs the body zero times, "a
null source is empty… no halt".  Measured: a collection field never filled, and a call whose
declared `vector<τ>` return answers null, each iterate zero times with no fault.  `(I-Empty)`
holds by the same line of code, which is the point — they are one question, *how many elements
does this source have*, and `vector::length_vector` answers 0 for both.  Both rules now cite it.

**Its "In words" gloss did not.**  The same paragraph ended *"so a `for` over a possibly-null
collection is safe without a guard"* — and the type-level spelling of "possibly-null" is `τ?`,
which is exactly the one case the rule does not cover and the compiler refuses.  A reader
following the prose writes `for x in v` with `v: vector<integer>?` and does not compile; a reader
following the formal line writes what ships.  Corrected, with the distinction stated: `nullref`
is a RUNTIME null of a NON-nullable type, and a `τ?` needs the discharge that `(N-Coal)` /
`(N-Default)` require of every other position.

**A reconnaissance of `matching.md` in the same pass, and it is a NEGATIVE.**  It is the other
100 %-uncited doc (23 rules) and it also declares `OPEN: 0`, so it was the obvious next target.
Every claim checked HOLDS: `(M-Wild)` refuses an arm after `_` and says *"move `_` to the end"*;
`(M-Exhaust)` names the missing variant; `(M-Bool)`, `(P-Seq)`/`(P-Whole)`, `(P-Rest)`,
`(P-Alt)`, `(P-Rep)` and `(P-Opt)` all behave as written, including *"P-Opt never Fails"*.  So
the prose-vs-rule shape is not universal — two docs had it and the third does not.

⚠ **What it cost instead was FOUR wrong probes, and the reason is worth the note it got.**  The
rules write a repetition `⟨(a)*, κ⟩`, and those parentheses are METANOTATION — but the concrete
syntax uses literal parens for a VARIANT element (`[ (x: Num)*, ..rest ]`) and NONE for a scalar
one (`[ xs:integer* ]`).  Reading `(a)*` as source is what a first reader does; each wrong
spelling reports `Expect token ,`, which names nothing.  Three of my four failures were that, and
the fourth was `--tests` swallowing the directory again and attributing one file's parse error to
another — the third time today ([[tests-flag-swallows-its-path]]).  **A refusal that reads as a
defect is a spelling error until an authoritative example says otherwise**; the one here was
`tests/parse_errors.rs`'s `scalar_rep_type_mismatch`, which quotes the form outright.  Both
spellings are now written beside `(P-Rep)`.

**Two docs, two walks, the same shape.**  B8g found `collections.md` stating the opposite of the
rule it cites; this one finds `iteration.md` promising one case more than the rule it explains.
Neither was a code defect and both would have sent a reader wrong — which is the argument for
walking a rule by READING it against the code even when the code turns out to be right, and the
reason an `OPEN: 0` is a claim about the deviation register and never about the prose around it.

⚠ **And it is the check B8h owed and did not make.**  CLAUDE.md's rule is to read the formal spec
before shipping a REFUSAL, because *"a rule may say it must work, making the refusal a
deviation"*.  B8h improved a refusal's wording without first asking whether `iteration.md` had
anything to say about a null source — it did, and had the answer been about `τ?` the work would
have been polishing something that should not exist.  It was not, so B8h stands; the process gap
is the finding, and the cost of closing it was one grep.
#### B2 — open, and the owner's call

| decision | evidence | why it is not mine to take |
|---|---|---|
| ~~remove `Value::BreakWith` and `Value::ParFor`~~ | — | ✅ **DONE** — see B3 below |
| ~~the 127 catch-all walkers~~ | — | ✅ **MEASURED** (2026-08-24) — see B4f below. Reachable, and so far latent |
| **`Parallel` is reached 4 times in 854 programs** | corpus census | a coverage gap in the suite, not a defect — but `par` is the construct with the least IR-level exercise of anything still alive |

#### D — carried, unchanged by this thread

`STABILITY_ROADMAP.md` still owns these: Plan-53 cluster 2 S4 (parked WIP, M), @PLN130's 504
uncovered copy sites (L, cost unestablished), gate 4 durability (@PLN43, needs an in-or-out
decision), H6 `i32::MIN` (deferred).  **On `main` as of 2026-08-24** — PR #1084 absorbed the
bulk of this thread; the branch now carries only the tranches after it.

### The catch-all audit — every type-driven op choice, classified (2026-08-22)

Two defects in one week had the same shape: a `match` on a TYPE that picks an **op**,
ending in a silent `_ =>`.  So the shape was swept rather than the symptom —
**23 such sites**, classified by how the catch-all FAILS:

- **9 fail LOUDLY** (`panic!` / `unreachable!`).  An ICE is not a wrong answer, and four
  of them (the tuple get/put family) are additionally gated by `ref_tuple_element_ok`,
  whose comment records why the two lists are one list: *"loft#1006 was them
  disagreeing"*.
- **14 fail SILENTLY.**  This is the class that produced both defects.

Result over the silent ones — **1 live bug, 1 stale mirror, 1 already-filed, the rest
clean with a stated reason**:

| site | verdict |
|---|---|
| `parser/mod.rs` fn-ref field write | **BUG — fixed.** A source nobody enumerated (an `if`/`match` arm) wrote four bytes of a twenty-byte fn-ref: SIGSEGV on `--interpret`, `unreachable!("invalid fn-ref")` on `--native`. Now refused like its siblings. |
| `parser/mod.rs::null()` | the undiscriminated `Type::Enum` arm — **half of loft#1065**, filed |
| `substitute_type_in_value`'s `OpConvBoolFromRef` map | a stale MIRROR of `coalesce_not_null`, missing the same two types the `??` check was. **Measured: fires 0× across all 826 `tests/scripts`, the 33 `tests/docs` and every generic probe** — a `for` now terminates on the length, not on a null element. COMPLETED and made exhaustive rather than deleted: completing can only repair more paths than today, never fewer. |
| `wrap_vector_get_val` | was a bug, fixed earlier the same day; now exhaustive and off the scan |
| `cell_value_set_op` + its read twin in `objects.rs` | clean — `cell_struct_name` gates both, and the three copies of that list agree |
| `primitive_setter_call` | clean — `is_primitive_vector_element_target` gates it, lists agree |
| `materialize_tuple_element` | clean, and PROBED (text element, record, vector, destructuring) |
| `emit_typed_null` | probed on its documented trigger — an omitted `= null` parameter — at seven types incl. a value enum, both backends: the parser's own `null()` answers first |
| `narrow_route_for` | clean — the catch-all is a documented fallback to the general wide path, which is the SAFE direction |
| `emit.rs` format-count | clean — a `with_capacity` HINT; falling through costs a preallocation, never a value |
| `codegen.rs` op-name inference | clean — the catch-all is "use the op's declared return type", which is the correct default |
| `sandbox.rs` growth walker | clean — the recursion is AFTER the match via `for_each_child`, so a catch-all cannot skip a subtree |

**The reusable half:** classify a catch-all by how it fails before reading what it
covers.  A panic is a bounded risk; `_ => None` / `_ => return <input>` is where a
type-driven decision goes missing quietly, and it is worth grepping for on its own.

⚠ **A 24th site, found 2026-08-22, and the sweep's own boundary was the lesson.**
`populate_struct_from_jsonvalue`'s `_ => { /* not yet handled … Leave at zero-init
default */ }` is the same silent shape and cost more than any of the 14 — it dropped every
narrow-integer field the `JsonValue` walker was handed, values included (see the Q1/P54
entry under § JSON cluster). The sweep did not have it because the sweep's own frame was
*"a `match` on a TYPE that picks an **op**"*, and this one picks a **write**: no opcode is
selected, so it did not match the grep that found the other 23. The failure mode is
identical, which said the frame was one word too narrow — the class is a type-driven
DECISION with a silent default, whatever the decision produces.

### Re-run with the wider frame (2026-08-22) — 210 sites, and the shape that actually bites

Swept every `match` whose scrutinee is a TYPE source (`.parts`, `known_type`, `content_kt`,
a `tp`, a discriminant): **210 sites — 56 already exhaustive, 15 with a loud catch-all, 139
silent.** 139 is too many to read, and reading them is the wrong move: almost all are
FALLBACKS, where the catch-all hands a value back (`_ => None`, `_ => false`,
`_ => u16::MAX`) and the caller decides. A fallback is visible to whoever asked.

The shape that bites is narrower and worth naming: **a side-effecting walk whose catch-all
does NOTHING** (`_ => {}` / `_ => ()` / a bare `return`), because then the destination
silently keeps whatever it already held and nobody is handed anything to check. Filtering
the 139 to `.parts` walks of that shape gives **24**, which IS readable. Result:

| verdict | sites |
|---|---|
| **the live bug** | `populate_struct_from_jsonvalue` ×2 — fixed today (§ JSON cluster) |
| **correct, but coupled to a refusal in another file — now exhaustive** | `copy_claims` (below) |
| **guards** — "only a keyed collection has keys", "only a struct has fields", "only a container can be inserted into": `set_keyed`, `insert_record`, `determine_keys_for`, `load_key_text`, `load_key_sets`, `relocate_ptr_fields`, `collect_sub_records`, `tree_roots`, `layout_closure`, `validate_all_layouts`, `validate_layout_by_nr` | 11 |
| **diagnostics** — a skip costs a missed warning, never a value: `walk_copy_cmp` (the `copy_check` validator), `first_oob_text`, `search.rs::validate` (whose own doc comment lists what it does not yet check) | 3 |
| **actually loud** — the catch-all raises a user diagnostic; the regex only read it as silent because the arm body opens a block: `fill_iter` | 1 |
| **narrow surface, value-returning fallback**: `file_to_bytes` / `file_from_bytes`, `output_init` (which states why an inline container field is emitted elsewhere) | 5 |

**The reusable half, sharpened:** classify by how the catch-all fails, *then* by whether
anyone is told. A panic is bounded. A value-returning fallback is visible to the caller. A
**do-nothing arm in a walk that writes** is the one that goes missing quietly — and that
filter cut 139 unreadable sites to 24 readable ones without dropping either real find.

### `copy_claims` skips `Parts::DbRef` — correct, and now it says so

Measured rather than reasoned. An env-gated counter in the catch-all over the whole
`tests/scripts` corpus: `Byte` 72276, `Base` 64588, `Int` 14184, `ShortRaw` 3110, and
**`DbRef` 3** — so the one kind that could own something really does arrive here, from
`85-poison-return-tail-uaf.loft` passing a `fn(…)`-holding struct by value.

The skip is deliberate: the block copy preceding this walk leaves the destination's 12-byte
`DbRef` pointing at the SOURCE's closure record, so a copied fn-ref field ALIASES rather
than owns — which is why a bound fn-ref field read is marked `skip_free`. **That is sound
only because the copy can never outlive the source, and nothing guaranteeing it lives in
this file:** #318 refuses a capturing-closure holder as a return value, as a collection
element, and as a field of another struct, and a fn-ref struct field admits ONE capture
shape program-wide. Probed all four escape routes — every one is refused at compile time —
and the two legal copies (passed down by value; copied out of an inner scope into a longer-
lived local) answer correctly on both backends under `LOFT_POISON=1` with store churn.

So: not a bug, but a guarantee held together across two files with nothing at the load-
bearing end saying so. The arm set is now EXHAUSTIVE — each remaining kind states why it
has no claim, and `DbRef` carries the coupling and the measurement. Proven: adding a probe
`Parts` variant fails the build at `allocation.rs:2876` (`copy_claims`) alongside `:2157`
(`validate_claims`, made exhaustive the day before for the same reason). Relax #318 and the
build still won't complain — but the comment now tells the next reader where to look.

### `character` on the JSON surface — five sites, one sentinel, none of them agreeing (2026-08-22)

`formal/types.md` pins `Char`'s in-band null at **codepoint 0**, reserved even in a
non-null slot, and loft#1014 made every site that WRITES one agree. The sites that READ
it, and the ones that put a character on the wire, never did. Found by pulling on a dead
line noticed while fixing the JSON walker: `Stores::is_null`'s `character` arm sits under
`if known_type < 6`, so it can never run.

| # | site | what it did |
|---|---|---|
| 1 | `Stores::is_null` | the arm was unreachable AND tested `u32::MAX`, not the sentinel — two wrongs cancelling into *"a character slot is never null"*. An absent field rendered as a value: `{"a":' '}`, **a space on the wire**, while `x.a == null` answered `true` |
| 2 | the character renderer (`format.rs`) | re-derived the same `u32::MAX` test, so a null element printed `['a',' ','c']` — and in the plain display, nothing at all: `['a',,'c']` |
| 3 | `to_json()` | wrote the loft spelling `'q'` in JSON mode. **That is not JSON**, and loft's own parser rejected the document — losing every other field with it |
| 4 | `walk_parsed_into` (the `text` walker) | put a character through type 0's arm — an 8-byte `set_int` into a **4-byte slot** — running over whatever the layout put next |
| 5 | `populate_struct_from_jsonvalue` | had no character arm at all, so it dropped the field |

Site 3 is the one that would bite hardest in the field: **a program that saves state with
`to_json()` writes a file nothing can read, including itself.** Site 4 is the one that
corrupts: `{"tail":5,"c3":99,"c2":98,"c1":97}` into `T { c1, c2, c3: character, tail:
integer }` answered `a`, NUL, NUL, `5` — only the LAST character written survived, so
document ORDER decided whether the damage was visible. That is why the forward-order probe
passed and looked like a clean bill.

Fixed at the sentinel: `is_null` answers codepoint 0 (and its arm is reachable), the
renderer asks instead of re-deriving, a character goes on the wire as the one-character
**string** `to_json` writes, and both walkers read either that string or a NUMBER as its
codepoint. Guarded by `tests/scripts/character-across-the-json-surface.loft`, six cases on
both backends, falsified by re-breaking sites 1 and 4.

**Deliberately NOT changed, and now asserted so it stays a decision:** string
concatenation SKIPS a bare `'\0'` (`OpAppendCharacter`'s documented behaviour), so
`"a{'\0'}b" == "ab"` and `{c}` on a null character is empty while the structured render
says `null`. The sentinel IS the literal NUL — `types.md` records the collision and
loft#1014 asserts it — so the two channels genuinely differ, and the difference is the
price of an in-band sentinel rather than a sixth disagreement.

### A Join whose arms each OWN a store — one leak, and two wrong answers behind it (2026-08-23)

loft#1078, filed by `loft_planet` against `main`.  A tail `if`/`match` that returns a fresh
record on one arm and a named LOCAL on the other retained one record per call:

```loft
fn pick(c: boolean) -> S { w = S { a: 7 }; if c { S { a: 9 } } else { w } }
```

`w` is renamed onto the hidden return buffer (NRVO), so the `else` arm delivers the buffer and
the `if` arm delivers a different store — and on the arm that does not deliver `w`, the store
`w` minted was returned by nobody and freed by nobody.  The value was right; only the ownership
was wrong, which is why a single call looks clean and only a loop shows it.  In the field:
~16,000 retained records per planet, and four planets exhausted the 65,535-entry `store_nr`
table.

`scopes::free_vars` already reaches this class through three legs — a null arm, a promoted
buffer no arm names (loft#688), and arms that disagree about ownership (loft#1022).  The one
that covers *"several owned candidates, one winner"* excluded every ARGUMENT.  Right for a user
parameter; wrong for the promoted buffer, which is the one argument that is really a local this
function minted.  **loft#1022's own comment had already written the carve-out down** and applied
it inside its own gate, noting that loft#688's leg "cannot claim it here because it excludes
anything in `sources`".  The multi-source leg needed the identical sentence.

⚠ **The filed report's matrix moved one axis and pinned two, and the two it pinned each hid a
`silent-wrong`.**  It varied *what the non-taken arm names* (local / parameter / vector element)
and held the RETURN POSITION and the ARM COUNT fixed.  Moving those:

| moved axis | what appeared |
|---|---|
| a SECOND owned local (`if c { u } else { w }`) | `u` answered **0**.  The first candidate is renamed onto the buffer; the second's copy leg then emits `OpDatabase(buf); OpCopyRecord(<tail that reads buf>, buf)` — the re-mint destroys the store the copy is about to read.  A three-arm `match` broke only its FIRST arm, which is what named the RENAME rather than the join. |
| BOUND, then returned (`r = if c { … } else { w }; r`) | the fresh arm answered **0**.  Not a tail join at all — this is loft#848's class one arm over: `parser/objects.rs`'s value-position `Object` arm is `!first_pass`-guarded, so it mints on pass 2 only, and on the shared `__ref_N` counter it was handed the name pass 1 left on the return buffer.  `return_buffer()` resolves the buffer BY NAME, so the literal's record and the return destination became one slot.  loft#848 had moved the SIBLING arm of the same function onto `__ref_p2_N` and left this one. |

Both wrong answers were **identical on both backends**, so neither backend could witness the
other and the `--interpret`/`--native` differential — the workhorse gate for this subsystem —
was structurally blind to them.  Collections and text were measured CLEAN on the same shape:
each carries its own aliasing-aware delivery (`OpReplaceVector` is a documented no-op when the
source still aliases the buffer; the B5-L3 text hoist copies first), so the defect is the record
path's re-mint.

The three cures are three INDEPENDENT guards on one collapse, which the ownership oracle proves:
`oracle_flags_the_a1b_wrong_plan` needed `LOFT_NO_A1B` + `LOFT_NO_WORKREF_STEPOVER` to have a
defect to catch, and now needs `LOFT_NO_P2_OBJECT_WORKREF` as well.  That test failing is how
the third guard's independence was measured rather than argued.

Guard: `tests/scripts/1078-join-arms-that-each-own-a-store.loft` — 10 cells, the leak half
proven to fire the wrap leak gate on a pristine worktree at `f7a57124` and the value half proven
to fail its assertions there.  `formal/ownership.md` gained D-own-7, opened and closed the same
day.

**The sweep, and the number that explains why it shipped.**  `LOFT_TRACE_WORKREF` prints one
line per work-ref mint with the SITE that asked for it, so the collision has an exact
signature: *one variable minted from TWO different sites inside one function*.  Sweeping all
844 `tests/scripts` on that signature gives **138 hits across 24 files** — and almost all are
benign, because a work-ref name IS scratch and pass 2 re-resolving it to the same scratch slot
is the intended reuse.  The harmful half is narrower: the name resolves to an **argument**,
which for a `__ref_N` can only mean `ref_return` promoted it to the return buffer on pass 1.

The trace could not say which — including for its own headline example — so `arg=yes|no` was
added to it.  Filtered on that predicate the sweep reads:

| | argument-resolving collisions | files |
|---|---|---|
| with the fix | **0** | 0 |
| with `LOFT_NO_P2_OBJECT_WORKREF=1` | 29 | **7** |

Six of those seven are scripts that were **already in the suite and already passing** (`85`,
`744`, `877`, `882`, `889`, `890`).  They reached the collision every run and never witnessed
it — the buffer was handed out, re-minted, and the value still arrived, so nothing failed.
That is the whole answer to "how did this ship": the corpus had the shape six times over and
no channel that could see it.  The detector was proven able to fire before the zero was
believed: it reports the loft#1078 repro under the opt-out and goes silent with the guard on.
A zero from an instrument that was never shown to fire is not a measurement.

**The reusable half:** when a fix's own comment says *"that other leg cannot claim it because
…"*, the sentence is a map of where the same hole is.  loft#1022 wrote down the promoted-buffer
carve-out and applied it to one gate; the sibling gate three lines up needed it too, and nothing
connected them.  Grep the carve-out, not the symptom.  And when a sweep over a whole corpus
reports a number too large to read, the fix is usually a missing FIELD on the instrument, not a
narrower grep — one `arg=` flag took 138 unreadable hits to 7 readable ones.

### A `&` on a tuple LOCAL linked nothing, at every element type (2026-08-23)

`formal/tuples.md` D-tup-2, open since 2026-08-20, said the admitted-element rule was asked
at the signature and not at the local — a `&(text, text)` LOCAL reaching codegen and dying
there as an internal compiler error while the identical PARAMETER was refused with a message.

Re-measured across POSITIONS rather than at the filed cell, the ICE was the mild half.  The
whole binding was unimplemented at a local, for **every** element type including the admitted
ones, and how loud it was depended on what the tuple happened to hold:

| written | was | should be |
|---|---|---|
| `b = &a` | the `&` **dropped**: a plain copy, so `b.0 = 5` left `a` untouched — no diagnostic, both backends | `a.0 == 5` |
| `b: &(integer, integer) = a` | a reference typed over a value: interp read an ELEMENT as a store index (`(7, 9)` → *"index is 9"*), `--native` handed the user a raw rustc `E0308` | the link |
| `b: &(boolean, boolean) = a` | `truefalse` where the swap says `falsetrue`, **exit code 0** | `falsetrue` |
| `b: &(float, float) = a` | `null` for a present element | `9.5` |
| `b: &(text, text) = a` | the filed ICE | a refusal |
| `b: &(integer, integer) = v[0]` | bound a COPY silently; `--native` would not compile | a refusal |

**Both backends agreed on every one of those**, so the tuple differential this subsystem
leans on (D-op-1) was structurally blind — the two implementations were wrong the same way.
That is the third time in a week that a defect survived because the workhorse gate compares
two things that share the mistake.

**The fix is the chokepoint the deviation asked for, plus the mechanism it then needed
something to admit.**  `Parser::ref_var_type` is now the one place a `&` in source becomes a
`Type::RefVar`, so the parameter, the annotated local and the inferred `b = &a` ask one list
(`data::ref_tuple_element_ok`) and cannot disagree.  And a tuple local lives in the FRAME, so
it joins the scalars at `OpCreateStack` — exactly the stack ref a `&(…)` PARAMETER is already
handed at its call site, read at the same `(ref, offset)` pair.  Native represents the local
link as the raw `*mut (…)` @PLN87 L1 gives every local link; two sites read one predicate
(`generation::is_raw_tuple_link`) to decide it, the element base and the call that forwards
the local to a `&(…)` parameter.

A tuple PLACE (`b = &v[0]`, `b = &s.pair`) is now refused rather than bound to a copy — the
place is read element by element into a fresh tuple before the `&` is seen, so nothing
survives to link to, and `binding.md` B-Ref-Reshape already settles that case: *"loft will not
quietly downgrade the reference to a copy"*.  `T-Ref` gained the local position, `T-Ref-Src`
the place refusal; tuples.md is back to **OPEN: 0**.

**Three reusable halves.**  First, **a deviation entry inherits the framing of the report that
raised it** — this one said "element types" because an ICE on `text` is what got filed, and
sweeping element types while pinning POSITION left a `silent-wrong` cell that no deviation
named.  A rule quantified over *"ANY binding"* (B-Ref-Alias) is falsified by a position as
readily as by a type.  Second, **the refusal boundary was not where the rule seemed to put
it**: the record-backed `RefVar(Tuple)` a `for` loop builds over a `vector<(text, text)>` reads
and writes `text` elements correctly on both backends, so putting the gate in a universal
`RefVar(Tuple)` constructor would have refused a shape that works.  Measuring the OTHER
construction is what kept the chokepoint at *the `&` written in source*.  Third, the guard was
proven able to fail on a pristine tree at `1e9d7910` — 6 of 7 cells on `--interpret`, 7 of 7 on
`--native` — and the prefix-`&` cell fails there on its **value** assertion, not on a crash,
which is the channel that had been missing.

### 81 assertions the corpus contained and never ran — and the wrong line one of them reported (2026-08-23)

The differential oracle's finding one level further out.  That pass converted 153 hand-written
expectations from COMMENTS into `assert`s, on the reasoning that a channel captured and never
compared is not a gate.  The question this pass asks is the next one: **of the assertions the
corpus does contain, which ones EXECUTE?**

It is not answerable by reading.  A file skipped for an expected error, a function the entry
point never calls, a branch never taken and a passing test all look identical from outside —
green.  So the instrument: **`LOFT_TRACE_ASSERTS=<path>` appends `file:line` for every `assert`
that runs**, in `n_assert`, which is the interpreter's implementation AND the one a `--native`
binary links, so one hook covers both backends and many processes.  Diffing the trace against
the `assert(` sites in the source names the silent ones.

**Result over `tests/scripts`: 9 722 sites executed, 81 never.** Three mechanisms, none of
which any file said anything about:

| # | mechanism | sites |
|---|---|---|
| 1 | **a firing `@EXPECT_ERROR:` stops the whole file** — `run_test` returns at *"ok (errors consumed)"* and `native_scripts` skips the file, so every runtime cell in it is compiled and dropped | 52 |
| 2 | **a file with `main` runs ONLY `main`** — every other zero-parameter function is compiled and dropped | 21 |
| 3 | deliberate: `assert(false, "unreachable")` markers, a branch not taken, an `assert` that IS the refused expression's use | 8 |

Mechanism 1's sharpest case is **`1067-lambda-expected-type.loft`, whose entire positive half
— 13 cells, 21 assertions — had never run.** Its own header states that the negative cell
exists *"or this file would pass on a compiler that simply stopped checking"*; that negative
cell is what stopped the positive cells from running, so the file passed on the refusal alone.
The cure is the corpus's own convention (`102`/`102b`, `36`/`36b`): a file asserts a refusal
OR runs.  Split into `1067b-lambda-no-expected-type-refused.loft`; same for the stranded
positive cells in `36-parse-errors.loft` and `pln119-assign-to-file-scope-text.loft`.

Mechanism 2 was `05-enums.loft` and `06-structs.loft` — struct formatting, `limit()` narrow
fields, `&`-default parameters, copy-on-bind, a named codegen regression guard — 21
assertions, wired into `main` now.  All 21 pass on both backends, which is the point: nobody
knew.

**A fourth mechanism, on the native side only.** `native_scripts` decided to skip a file with
`src.contains("@EXPECT_ERROR")` over the whole source, so a file that merely NAMED the tag in
prose dropped out of the native suite — silently, since a skip prints and passes.  Five files,
**79 assertions**, including `93-vector-advanced.loft`'s 49.  Every one of the five was
carrying a comment recording that it had **stopped** being a refusal case: *"this file used to
be an @EXPECT_ERROR case"*, *"stayed here live and `@EXPECT_FAIL` until #1055 was fixed"*.  The
sentence saying a file was no longer refused is what stopped it being tested.  Both runners now
read one `common::expect_tag`, which is the same lesson `ref_tuple_element_ok` carries:
*two lists that must agree are one list*.  Native goes 801 → 806 scripts, all green.

**And the trace found a live bug, because it records the position the COMPILER injected.**
Every assert in `685-mutated-scalar-param-capture.loft` traced exactly seven lines early, and
every assert in `50-tuples.loft` five.  Breaking one proved it reaches a user: an assertion on
line 184 failed and the diagnostic printed

```
error: assertion failed: repeat call, two params
  --> …/685-mutated-scalar-param-capture.loft:177:1
177 |   assert(inner == 13, "by-value: callee sees its own writes, got {inner}");
```

— **this assert's message under a different assert's source line**, and the line it named is
itself an `assert`, so the report looks entirely plausible.

The mechanism is loft#625's, at a site that fix did not reach.  `Lexer::to` moves the
REPORTING position without moving the read cursor, and the tokenizer keeps incrementing that
position on every physical line it pulls — so a seek that is never undone shifts every position
derived from the lexer for the rest of the file: the caret, a runtime span, and the line the
compiler injects into `assert`.  `parse_function` wraps its warning passes in a save/restore
and its comment says exactly this — *"Each warning pass below seeks the lexer to a diagnostic
site … Save the true position and restore it once the passes finish"* — and
`check_ref_mutations` (the needless-`&` / `needless-const-parameter` pass) runs **eighteen lines
above that save**.  The carve-out comment was the map: *below* was doing the work of a fence.

Fixed at the chokepoint rather than by a second save/restore, because a rule spelled at each
call site is how the first one came to be missed.  `to()` now records where it seeked FROM, and
the next token scanned from source restores it: **a reporting seek lasts until the next token,
so a missing restore costs the one diagnostic it was made for instead of every position after
it.**  A file switch clears the pending seek — without that, the first token of a `use`d file
inherited the previous file's line, which the corpus caught immediately (`88-imports`,
`850*`, at −13 to −20 lines).

Minimal repro, six lines, both backends, `silent-wrong` in the diagnostic channel:

```loft
fn f(n: const integer) -> integer { n }   // draws `needless-const-parameter`, which seeks
fn main() { assert(false, "MARK"); }      // reported line 2, not 5
```

Guard: `runtime_warnings.rs::a_seek_to_a_warning_site_does_not_shift_later_positions`, proven
able to fail (it reports line 2 with the restore disabled).  Corpus-wide re-measure: the two
constant shifts are gone and no file has one.

**Two ratchets, both static, both proven able to fire** (`tests/wrap.rs`):
`a_refusal_file_carries_no_runtime_assertions` (mechanism 1) and
`every_assertion_is_reachable_from_the_entry_point` (mechanism 2).  They are
UNDER-approximations by construction and say so: they cannot see a branch never taken, and they
deliberately allow the documented dual guard `751`/`432b` uses, where an annotated `main`
carries assertions that run only if the refusal ever regresses.  `LOFT_TRACE_ASSERTS` is how
the remainder gets re-measured — a report, not a gate, because a gate over 9 800 sites would
need an allow-list keyed on line numbers and would rot.

**Measured residual, so it is a decision and not an oversight.** `751`'s near-side cell —
*"a `vector<u8>` built from integer LITERALS … must stay legal"* — is inert for the same
reason the far side is, because both live in that file's annotated `main`.  Coherent as
written (the whole file runs only if the refusal regresses), and the shape is positively
covered by the running `432-untyped-vector-literal-arg.loft`; recorded here because a
ratchet that allows a pattern owes a reader the list of what the allowance costs.

**The reusable half.** A test suite's guarantee is the set of assertions it RUNS, and that set
is not the set it CONTAINS. The difference is invisible in every channel a suite reports —
exit code, pass count, output — so it has to be measured directly. Every mechanism found here
was a file being SKIPPED for a good reason that quietly took a second thing with it.

### The caret follows the CURSOR, and the cursor is one token past the code (2026-08-23)

The assertion-line pass one level out again.  That pass found a whole-file line lag by reading
the position the compiler INJECTED into `assert`; the position channel it read is the same one
every diagnostic uses, and nothing in the suite compares it.  `check_diagnostics` matches an
`@EXPECT_ERROR` / `@EXPECT_WARNING` by SUBSTRING — `diag.contains(pat)` — so the `file:line:col`
each of the corpus's **272** annotations carries is captured and dropped, the exact shape
loft#1063's stderr channel had.

`tests/parse_errors.rs` DOES pin `line:col` exactly, on all 248 fixtures.  It was blind for the
usual reason: **its corpus holds one axis fixed.**  Almost every fixture is a one-liner or a
statement ending in `;`, and the `;` is what hides this.

**The mechanism, and why `;` hides it.**  `Lexer::position` is the scan CURSOR — the end of the
token the parser is *holding*, not of the code it has decided about.  A check that can only run
once a construct is complete (a `const` write, a nullable reaching a non-null slot, a capture in
a `parallel` arm) raises with the cursor already advanced.  A `;` keeps that next token on the
statement's own line and the two answers agree; drop it — which loft invites, being
expression-oriented — and the caret goes wherever the next token is:

| written | caret landed on |
|---|---|
| `a = 42;` then `}` | line of `a = 42` — correct, **by luck** |
| `a = 42` then `}` | the `}`, one line down |
| `a = 42`, two blank lines, `}` | the `}`, **three lines down**, with a different statement under it |

Same statement, three answers.  Corpus-wide the miss was not rare: an identifier-position oracle
(*a message that quotes a name must point at a line containing that name*) reported **41
suspects, 16 of them a caret sitting on a closing brace** — const writes, `parallel`-arm captures
and the whole null-flow family.  And two diagnostics were naming an entirely **different
construct**: `circular init dependency` landed on the `fn` after the struct, and *"Not all code
paths return a value — function `classify`"* landed on the function after `classify`.

**The fix is one place, and the measurement is what kept it there.**  `Lexer::report_pos`:
a diagnostic goes to the end of the CONSUMED source (`prev_end`, captured in `cont()` before the
cursor runs on) when the current token starts on a LATER line, and to the cursor otherwise, so
the same-line column contract 248 fixtures pin is untouched.  A `Lexer::to` seek outranks both
(that position was chosen — the item-5 rule), and the file must match.

⚠ **The obvious wider fix is wrong, and one A/B settled it.**  Attributing *every* diagnostic to
the consumed source moved **107 of 248** fixtures, and every one that moved was a syntax error
about the token the parser is HOLDING — `Expect name in function definition` on `fn assert(…)`
went from the `assert` to the `fn`.  So the class genuinely has two halves, and only the site
knows which it is in.  **Three** sites say so explicitly now, each with a comment saying why:
`'struct' definitions must be at file scope` and the `..hi` open-range refusal are both raised
while LOOKING at the offending keyword, and `unreachable-code` is raised holding the first token
of the unreachable statement.  That is the same shape as the 48 sites already reaching for
`peek_pos`, and all three now put the caret ON the token rather than just past it.

⚠ **`parse_errors.rs` did not find all of them — the whole suite is the position oracle, and it
had to run `--no-fail-fast` to say so.**  Beyond the 11 fixtures parse_errors moved, three more
turned up only in `tests/issues.rs` and one in the `error_messages` golden: `..hi` (the third
opt-out), and two more instances of the defect that nothing had ever looked at — the deferred
`OpMinInt` arity errors and `Unknown field Point.z` were BOTH being reported on the closing `}`
of their function, and now land on the expression.  A max-fail run reports one per cycle and
reads like a single stray fixture; `find_problems.sh --bg` is what showed the set.

**Net over the corpus, measured in both directions.**  The sharp filter is *the caret sits on a
line that is nothing but a closing brace*, which is the shape a cursor-following caret produces:
**19 → 4** over the 773 diagnostics the 882-file corpus emits, the before-number taken by
disabling `report_pos` on the fixed tree rather than remembered.  The looser identifier filter
(*a message quoting a name must point at a line containing it*) reads 41 → 25, and the 25 that
remain are its own false positives — the quoted name is a TYPE the line never spells.

**The 4 that remained were whole-CONSTRUCT judgements — and a second pass took them to 0.**
`circular init dependency`, a generator's discarded tail and an `i32` narrowing each complete
only when their construct does, so the consumed source genuinely ends at that brace.  That is
right and useless: a struct has many fields, and *"somewhere in this struct"* is not an answer.
The chokepoint cannot guess which part is meant, but each SITE holds a better position:

| check | now names | the datum it already had |
|---|---|---|
| `circular init dependency` | the field the cycle STARTS from | none — the one that needed threading (`init_deps` gained the field name's position) |
| a generator's discarded tail | the tail expression | `l[last].span_pos()` — a call is span-wrapped at its `(` on pass 2 |
| a tail conversion (narrowing, `not null`) | the tail statement | `block_result`'s `tail_pos`, already a PARAMETER and read by exactly one check |

**Measure before widening a site, because most of it already worked.**  Three of the four
narrowing POSITIONS — assignment, argument, struct-literal field — already named their own line;
only the return tail did not, because only it is checked after the block closes.  The guard pins
all four so a later change cannot move the working three unnoticed.  Same for the field COUNT
axis: the circular-init guard runs a `a -> b -> c -> a` cycle among four fields that are NOT in
it, so a caret that merely picked the first field, the struct, or its brace is visible.

⚠ **Seek with `Lexer::to`, end with `Lexer::end_seek` — never with a second `to`.**  Seeking
back leaves `seek_return` pending, and `report_pos` reads a live seek as a deliberate choice and
stops attributing to the consumed source, so every diagnostic the pass raises AFTER the seek
reverts to the scan cursor.  Measured: seeking around the block-tail conversion that way sent
*"Not all code paths return a value — function `classify`"* back onto the FOLLOWING function —
re-introducing, three sites away, the exact defect the pass had just removed.  The existing
`missing_return_not_null` fixture caught it, which is the argument for pinning the positions
that already work.

The instrument is `scripts/diag_position_audit.py` — a REPORT, never a gate, for the same reason
`LOFT_TRACE_ASSERTS` is: both filters have false positives, and a gate over them would need a
line-numbered allow-list that rots.

Guards: `a_diagnostic_names_its_own_line_{whatever_follows_it, with_no_terminator,
across_blank_lines}` assert the three layouts AGREE rather than pinning a hand-picked line — a
hand-picked expectation only ever pins the layout it was written for, which is how the corpus
came to hold this axis fixed in the first place.  **The first cell is the one that always
passed, and it is in the file to show what hid the other two.**
`a_current_token_diagnostic_still_names_the_current_token` pins the opt-out direction so a later
widening of the default cannot take it silently.  All proven able to fail by disabling
`report_pos`.

**The reusable half:** a suite that pins a channel is not the same as a suite that EXERCISES it.
248 fixtures asserted `line:col` and none of them varied the one thing the position depends on —
what follows the construct.  When an oracle reads clean, ask what its corpus never varies; here
the answer was a single character.  And the detector that made 565 raise-sites readable was not
a narrower grep but an extra FIELD — *does the current token start on a later line than the
consumed source ends* — which cut them to 25 raises across 13 sites, the same move as
loft#1078's `arg=` flag.

### Pulling on D-bind-11 found three tuple defects the register did not know about (2026-08-23)

`formal/binding.md` D-bind-11 (`&(τ,…)` admits only scalar elements) records a blocker
measured on 2026-08-19: adding the `text` arms SIGSEGVs.  D-tup-2 changed that
representation on 2026-08-23 — a tuple local now joins the scalars at `OpCreateStack` — so
the blocker was a claim to re-measure ([[filed-blocker-is-a-hypothesis]], the standing rule).

**The blocker HOLDS, and the re-measurement moved its cause one level down.**  Re-adding the
arms still corrupts (`rec=179867128`).  Not because "the ops speak different families", as the
entry said, but because a `text` on the STACK is a 16-byte `Str` — `{ptr, len}`, a raw BORROW
— while the record form is a 4-byte handle.  That also answers the question the entry never
did: **why `fn f(s: &text)` works while `&(text, text)` cannot.**  The `&text` parameter writes
into the caller's 24-byte owned `String` through `OpClearStackText`/`OpAppendStackText`, so the
owner never changes; a tuple's text element has no owner of its own on the stack.

**And the entry's second escape route is already running.**  It offers *"either an op family
that writes the STACK form through a DbRef, or backing a `&(…)` with a real record"*.  A
`for p in v` over `vector<(text, text)>` performs the EXACT swap the refusal declines,
correctly, on both backends.  ⚠ **Nothing guarded it** — no script in the corpus wrote a text
tuple element through the record path — so the evidence the whole design option rests on was
one refactor from vanishing.  Now pinned by
`tests/scripts/reference-tuple-heap-element-through-a-record.loft`.

**Three defects fell out of the matrix, all `silent-wrong`, none of them in any register.**

| # | shape | verdict |
|---|---|---|
| 1 | `v[0] ?? fb` where the tuple's FIRST element is a COLLECTION | **FIXED** — answered the FALLBACK for a PRESENT element, `--interpret` only |
| 2 | `hv = p.0` where the element is a COLLECTION | **D-bind-12** — aliases where `B-Copy` says it copies; both backends |
| 3 | `hs = p.0; p.1 = hs` where the element is a STRUCT | **D-bind-12** — the write-back is a NO-OP and leaks; both backends |

**#1's fix is one line, and the code's own carve-out comment was the map.**  A tuple has no
`.rec` discriminant, so the convention is *"a tuple is null when its FIRST field holds its
type's null sentinel"*.  `coalesce_not_null` built that test with the GENERIC
`convert(first_tp, Boolean)` — and the heap-DbRef branch **three lines below it** exists
precisely because that generic path has no registered `OpConv*FromX → Boolean` for a
collection: it hands back the bare Var, and the interpreter then tests raw BYTES instead of
`.rec != 0`.  The comment describing the hole sat directly under the call falling into it
([[carve-out-comment-is-a-map]] again).  The fix RECURSES into `coalesce_not_null` instead of
asking `convert`, so the null test and the heap-DbRef branch cannot drift.  A `Reference`
first element HID it — that type does have a generic path — which is why the axis had to be
the first element's TYPE.  Guard: `tests/scripts/tuple-null-check-reads-its-first-element.loft`,
proven to fail 3-of-7 on a pristine worktree at `aa8f02dd`; the 4 that pass there are exactly
the cells that hid it.

⚠⚠ **Two probes on the way here were VACUOUS, and it is the sharpest lesson of the pass.**  A
`print` placed between the steps of the struct swap made it answer CORRECTLY (`y|x`) where the
same loop without it answers `y|y` — **the observation materialised the value being measured**.
Everything scored off those prints was wrong, including a confident reading that a preceding
loop was corrupting a later one.  Re-scored with `assert` after the loop, the picture is the
disjoint table above.  The same reading also mis-explained the struct cell as `B-View`
*"aliasing by design"*, reaching for a rule to EXPLAIN an observation instead of to PREDICT
one — the rule says view, the measurement says copy, and the measurement was never in doubt.
In this subsystem: no `print` inside the loop under test.

### D-bind-12 fixed — and half of it turned out to be the RULE, not the code (2026-08-23)

Filed the day before as two halves.  Measuring the second one properly split them apart, which
is the whole result: **one was a real defect, the other was `formal/binding.md` under-stating a
model the language deliberately depends on.**

**Half one — FIXED, and the map was a doc comment describing a condition the code did not
check.**  `for p in w { hs = p.0; p.1 = hs; }` left `w[0].1` unchanged and leaked the record,
on both backends.  The write was not "a runtime no-op" — it was **absent from the IR**.

`move_elidable_source`'s last gate is *"owns a transferable store"*, read off
`Uses::def_vdb`, whose own doc says *`v = OpGetField(vdb, 0, _)` **where vdb is
OpDatabase'd***.  The walk inserted on ANY `Set(v, OpGetField(Var(x), …))` and never checked
the second half — so `hs = p.0`, a read of an EXISTING element through a borrow, counted as
owning a transferable store.  `move_rewrite` then dropped its `OpCopyRecord`, which is sound
only when the source is CONSTRUCTED (its build ops are retargeted onto the destination): `hs`
has no build ops, so **the copy WAS the write**.  `collect_uses` now enforces the documented
condition after the whole body is walked — it cannot be checked at insertion, because the
`OpDatabase` may not have been visited yet.

**The scope was measured, not argued:** emitted IR is **byte-identical on 120 of 120 scripts**
(after normalising the worktree path — the first run said 116/120 differed, which was the
absolute path in the dump and nothing else), and `857`'s allocation count is unchanged at 27,
so the pointer-bind it protects is untouched.  The change can only ever REMOVE a spurious
entry, so it is conservative-only by construction.

**Half two — NOT a code deviation.**  `hv = p.0` on a COLLECTION element aliases, and the
first reading scored that against `B-Copy`.  The 2×2 off a BORROWED base says otherwise:

| construct | element type | behaviour |
|---|---|---|
| struct field | vector-typed | view |
| struct field | struct-typed | view |
| tuple element | vector-typed | **view** — the filed cell |
| tuple element | struct-typed | copy |

The implemented model is *a projection off a BORROWED base is a view; off an OWNED base it
copies* — gated explicitly by `classify_vec_bind`'s `depend().is_empty()`, deliberate
(`cells = sc.v; cells[i] = h` writing through is @PLN25 p379's point), and with its
alternative measured to CORRUPT (#426).  Verified both directions, including that the p379
write-through still reaches the source.  `B-View` states the view for a **struct-typed**
projection only, so the rules cannot express a model the language depends on — which
[formal/README](formal/README.md) says means the RULE wants extending.  **Deliberately not
decided here:** widening `B-Copy` instead would delete p379's idiom and re-enter #426.  The
one real inconsistency left is the fourth cell (a struct-typed tuple element copies while its
three siblings view); no cell of it answers wrongly, so it is recorded rather than counted.

**The reusable half:** *"fix the filed thing"* was the wrong instruction to give myself.  The
filed report had two halves and one of them was not a defect at all — and the way to find that
out was to complete the matrix (2×2 over construct × element type) rather than to fix the cell
that was reported.  Had I "fixed" the alias, I would have deleted a documented idiom and
walked back into a corruption bug closed months ago.

### The last tuple-projection inconsistency — and the spec had already decided it (2026-08-23)

The residual D-bind-12 recorded: off a BORROWED base, three of four projection cells are
views and a STRUCT-typed TUPLE element copied.  I had written it up as a consistency question
for the owner.  It was not — `B-View` says *"a STRUCT-typed PROJECTION is a VIEW that aliases
WITHOUT `&`"*, so the direction was settled and only the code had to move.  **Consulting the
rule turned a judgement call into a lookup**, which is the whole point of the register.

**One site of three did not carry the base's lifetime.**  A stored-tuple element read took the
synthetic struct's attribute type VERBATIM:

```rust
let elem_tp = elems[idx].clone();
*code = self.get_val(&elem_tp, false, elem_offset, code.clone(), u32::MAX);
t = elem_tp;                                   // no deps, no base var
```

Its two siblings already did it right, and one of them says why — the plain-tuple site's P197
comment: *"propagate parent tuple's deps … without this, `a.v.0` returns a `Str` whose ptr
points into a freed host"* — while `fields.rs`'s struct-field read carries the base deps AND
`depending(base_var)`, which is exactly why `b = s.strf` typed `ref(In)["s"]` and `d = p.0`
typed a bare `ref(In)`.  A bind typed as an OWNER while holding someone else's handle, with an
`OpFreeRef` to match.  All four cells are views now, the dep is attached, the spurious free is
gone, and emitted IR is unchanged on **80 of 80** tuple-bearing scripts.

**Measuring the direction twice was necessary, and the first method was vacuous.**  The 2×2 was
first read with `print`s inside the loop — the method already recorded as unreliable.  Re-run
with asserts it gave the same answer, but a THIRD reading was needed before acting: a
write-through test showed `e += [7]` not reaching the source while `e[0] = 7` did, because an
append REALLOCATES.  Only `e[0] = 7` measures aliasing; the append measures something else.

**The consequence is pinned, not left to be discovered:** a three-step swap through a bound
element does not swap.  That is what the three sibling cells already did, and the cure —
hold the VALUE (a scalar/text local), rebuild after the write — is a cell of its own.

⚠ **A THIRD defect surfaced while writing that cure, and it is PRE-EXISTING** (reproduces on
`80a05a5c`): the move-elide retargets a construction into its destination at the
CONSTRUCTION's position, across a read of that destination.

```loft
for p in v { held = Tg { name: p.0.name }; p.0 = p.1; p.1 = held; }   // reads x|x, wants y|x
```

`held`'s build is moved into `p.1` before `p.0 = p.1` reads `p.1`.  `LOFT_NO_MOVE_ELIDE=1` is
the bisect step that names it; the Record shape has `bad_containers`, `ambiguous` and
`def_order` guards but nothing that asks whether the DESTINATION is read between the
construction and the copy.  Not fixed here — a separate concern from the projection rule, and
it belongs in its own change.

### The move-elide outran a read of its own destination (2026-08-23)

@PLN90 phase B transfers a dead-after owned source INTO its copy destination: it drops the
source's `OpDatabase` / `OpCopyRecord` / `OpFreeRef` and retargets the source's CONSTRUCTION ops
onto the destination slot.  So the destination is written at the CONSTRUCTION's position rather
than at the copy's — **that is a REORDER**, and nothing checked what sits between.

`collect_move_dest`'s guards all ask whether the destination is a STABLE container
(`bad_containers`, a compiler temp, `def_order`).  None asks whether it is TOUCHED.  The
pre-rewrite IR is the whole story:

```
    held = null;  OpDatabase(held, 78);  OpSetText(held, 0, …)     ← the build
    OpCopyRecord(OpGetField(p,4,78), OpGetField(p,0,78), 78)       ← p.0 = p.1, READS p.1
    OpCopyRecord(held, OpGetField(p,4,78), 78)                     ← p.1 = held, the copy
```

Retargeting moves the write of `p.1` up past the statement that reads it, so `p.0 = p.1` copied
the NEW value back.

**The filed repro held three axes fixed, and the defect was wider than all three.**  Measured on
`d672d261`:

| shape | was | wants |
|---|---|---|
| tuple swap through a rebuilt value | `x\|x` | `y\|x` |
| a **plain READ** of the destination (`seen = p.1.name`) | `n\|n` | `y\|n` |
| **straight-line**, no loop | `x\|x` | `y\|x` |
| destination is a struct **FIELD** | `n\|n` | `y\|n` |
| destination is a **VECTOR ELEMENT** | `n\|n` | `y\|n` |

The read cell is the one that shows the size of it: the intervening statement does not have to
WRITE anything — a plain read of the destination already sees a value the program has not
assigned yet.

**The fix is the missing predicate, and it is deliberately conservative.**
`collect_move_disturbed` refuses a source whose destination's BASE container is mentioned by any
statement between the source's definition and the copy.  Two carve-outs keep it from
over-refusing: the source's OWN construction ops are excluded (they are what gets retargeted, so
`o.f = T { x: o.g }` — building FROM the container into it — stays elidable), and a source whose
definition is not in this operator list is treated as "from the top", the conservative reading.
A slot-EXACT test would admit a few more cases and cannot be spelled reliably — two spellings of
one slot is the shape loft#1006 was.

**The cost is measured, not asserted: emitted IR is byte-identical on 851 of 851
`tests/scripts`.**  So no legitimate elide anywhere in the corpus is lost, and the five broken
shapes simply never appeared in it.

**The guard's last two cells are the ones that matter for a reviewer.**  Five cells fail on a
pristine tree; the two that PASS there are `test_the_clean_move_still_elides` and
`test_build_from_the_container_into_it` — the control and the carve-out.  A "fix" that simply
disabled the elision passes every other cell in the file, which is exactly why the control is in
it.

### The same hole in the sibling rewrite — and one of them grew a vector without bound (2026-08-24)

Two silent-wrongs in one day had come out of @PLN90 phase B, so the class was the MECHANISM
rather than either bug: **a rewrite that MOVES a statement needs to say what may not sit between
the two positions.**  There are four such rewrites in `scopes.rs`; the previous pass fixed one.
Auditing the other three found the same hole in a second, and the audit is the whole result —
neither of the two new defects would have been reached by pulling on the first bug's repro.

| rewrite | shape | verdict |
|---|---|---|
| `move_rewrite` | Record, `OpCopyRecord` | fixed the previous pass |
| `construct_move_rewrite` (B1.3b) | Construct, `OpAppendVector` | **the same hole — 2 defects, fixed** |
| `construct_fresh_rewrite` (B1.3c) | fresh container built AFTER the source | clean, and structurally so: the destination does not EXIST between, so nothing can read it.  Probed anyway (3 shapes) |
| `construct_replace_rewrite` (B1.3d) | whole-vector replace | **already had the guard** |

**B1.3d is where the sentence was already written.**  `try_replace_one` carries it verbatim —
*"`base`'s BUILD must not read the destination container `a` (a SELF-ASSIGN like `s.v = s.v[1..]`
… moving the `OpClearVector` ahead of that read would empty `s.v` before the slice copies it)"* —
and its two siblings in the same file did not.  That is the third time this month a carve-out
comment turned out to be a map of where the same hole still is, and the first time the map was
for a rewrite rather than for a type.

**What the Construct shape did.**  It guards the SOURCE being read between its build and the
move (`escaping` / `source_escapes`) and never guarded the DESTINATION — the identical asymmetry:

```loft
seen = len(b.v);  b.v += tmp;                        // seen reported 3 for a 2-element vector
for x in c.v { doubled += [x * 10]; }  c.v += doubled;   // SIGABRT
```

The second is the one that matters: the retarget pointed the loop's appends at the vector the
loop was ITERATING, so it grew without bound — `SIGABRT` on `--interpret`, and on `--native`
a `store offset overflow: requested 2203149353 words`.  Under the test harness it trips the 2 GiB
store ceiling at 1.7 GiB, which is how the guard fails on a pristine tree rather than taking the
box down with it.  **A time bound does not bound memory**, and this is a compiler rewrite
producing the unbounded allocation, not user code.

**One predicate, both shapes.**  `collect_move_disturbed` is now parameterised by the copy op and
which argument is the source — `OpCopyRecord(src, dst)` at arg 0, `OpAppendVector(dst, src)` at
arg 1 — rather than copied per shape, because two copies of one rule is the shape loft#1006 was.

**Cost measured, not asserted:** emitted IR byte-identical on **851 of 852** `tests/scripts`, the
one difference being the guard's own file.  Both fixes together cost no legitimate elision
anywhere in the corpus, which also says why these survived: the corpus never wrote the shapes.

### The rules doc stated three of five clauses, and I filed correct behaviour as a bug three times (2026-08-24)

Closing the statement-moving audit: the FIFTH member of the family — the borrow elision
(`elide_borrows` / `idiom_drop` / `elide_rewrite`) — is **clean**.  Twelve cells across the axes
the previous sweeps had held fixed (what mutates the source between: `+=` / element write /
whole-field reassign / a `&`-call / `remove`; the use: print / len / argument / return; position:
straight-line / loop / branch arm).  Do not re-run it.

**Two cells DID alias, and chasing them is the actual result.**  A NESTED field read
(`c = o.inner.v`) and a vector INDEX read both aliased where I read `B-Copy` as promising a copy.
Both survive `LOFT_NO_BORROW_ELIDE=1`, so neither was the elision — and
`tests/scripts/85-store-lifetime-reference-default-views.loft` turned out to carry the answer in
its header: *"#426's premise — that `a = vv[0]` / `c = o.inner.v` must COPY — was the **WRONG
read**."*  Both are decided VIEWS, guarded green, citing `OWNERSHIP_MODEL § The law`.

**So the defect is in `formal/binding.md`, and it is load-bearing.**  It states `B-Copy` plus ONE
exception (a *struct-typed* projection views).  Measured, there are three, and the boundary over
11 cells — identical on both backends — is:

| bind | result |
|---|---|
| a whole VALUE (`d = v`, `p = o`), and every scalar | COPY |
| a one-level **collection** projection off an **OWNED** base (`af = bx.v`) | COPY |
| a one-level **struct** projection off an OWNED base | VIEW — `B-View` |
| a vector **INDEX** read · a **NESTED** field read | VIEW — #426's resolution |
| **ANY** projection off a **BORROWED** base, at every element type | VIEW |

The two missing clauses are now written as **`B-View-Base`** (ownership of the BASE is the axis,
not the element type) and **`B-View-Depth`** (index and nested reads, with #426's premise
recorded as the wrong read so the next reader does not re-file it).

**The cost of the omission is measured, and I paid it three times in one week** — D-bind-12's
collection half, then a nested field read, then an index read: three correct behaviours filed
against `B-Copy`, each costing a full investigation.  Two of the three I recorded as "an owner
question"; neither was.  A rules doc that is incomplete does not fail loudly — it produces
confident, well-evidenced wrong conclusions, which is more expensive than a doc that says
nothing.

**The boundary now has ONE home:** `tests/scripts/bind-copies-or-views-the-whole-boundary.loft`.
The cells existed before, scattered across four files (`294-vector-element-view-semantics`,
`85-store-lifetime-reference-default-views`, `reference-tuple-heap-element-through-a-record`, the
C86 field cells) and no single one said what the rule WAS.  Ask that file rather than re-deriving
it from the code.

### One rule, how many implementations? — the checklist, and its first entry was drifted (2026-08-24)

A rule in `formal/` is usually enforced by a **membership test over `Type` variants** — *is this
a scalar*, *is this a keyed collection*, *does this own a store*.  Written inline at each site,
the copies drift, and a drifted copy is a defect rather than untidiness: loft#1006 was two
spellings of one tuple-element list disagreeing.

`scripts/rule_predicate_audit.py` measures it: **32 distinct type-lists of 3+ variants; 30 appear
at 2 or more sites.**  `--near` reports the pairs differing by exactly ONE variant, which is the
drift that is already there rather than the drift that might happen.  The verdicts live in
[formal/IMPLEMENTATIONS.md](formal/IMPLEMENTATIONS.md) — a checklist, because most entries need a
judgement the script cannot make.

**Entry #1 was already wrong.**  "Is this a scalar" is spelled 8 times in three variants, and the
variants matter: `generation/`'s two copies include `Type::Enum(_, false, _)` and
`data::ref_tuple_element_ok` did not.  A value enum and a `boolean` have the SAME 1-byte slot
(`element_stack_size`: `Boolean | Enum(_, false, _) => 1`), so:

```loft
fn sw(p: &(boolean, boolean)) { … }   // admitted
fn sw(p: &(Col, Col)) { … }           // "may only hold scalar elements, and this one holds `Col`"
```

The refusal read as a rule because the boolean case works.  It was drift.  Adding the value-enum
arms to `RefTupleGet`/`RefTuplePut` makes the swap answer correctly on **both backends** first
try — no representation question, unlike D-bind-11's `text` (a 16-byte `Str` borrow against a
4-byte record handle), which stays refused.

**The merge, and what was deliberately NOT merged.**  `data::is_scalar` is the one home;
`ref_tuple_element_ok` delegates and `generation`'s two copies are gone.  Emitted IR is
**byte-identical on 852 of 853** scripts (the one difference is the guard file's own new cells),
so the `generation` half is a proven behaviour-preserving refactor and the only semantic change
is the intended admission.

The **5 remaining sites spell the BARE five** — `scopes.rs`'s return-type check,
`generation/emit.rs`'s RefVar inner, `parser/operators.rs`'s `Const-ScalarCollapse`,
`parser/mod.rs`'s `size` receiver.  Adopting them would ADD value enums at each, which is a
behaviour change per site and needs its own probe.  They stay on the checklist rather than being
swept, because "these lists are equal today" is not the same claim as "these are one rule" — and
a merge that couples two rules which must stay free to differ is worse than the duplication.

### The heap-record family — one declared home, four sites that drifted off it (2026-08-30)

**The family:** *"is this a struct-like heap record?"* — `Type::Reference(d, _)` and
`Type::Enum(d, true, _)`, the two spellings of one notion. The declared home is
`Type::heap_def_nr`, which already documents itself as *"the definition number for
struct-like heap types (Reference or struct-enum)"*.

**The measurement** (`grep`, this tree): **13** sites ask through `heap_def_nr`; **27** more
spell the pair by hand. A further 37 spell `Reference | Vector | Enum(_, true, _)`, which is a
DIFFERENT question — *"is this carried on the heap at all"*, collections included — and is
correctly not this family.

The 27 are not the problem; a hand-spelled pair is right. **The problem is a site that spells
only `Type::Reference` where the family is meant**, because that reads as a deliberate
narrowing and is indistinguishable from one. loft#1202 found four, all in the return-ownership
path, and they compounded:

| site | what it decided | what the record enum got |
|---|---|---|
| `control.rs::block_result` — the record arm of the return-DELIVERY chain | which delivery a return takes | **no arm at all** — no delivery, so no return dep, so the caller read a borrow as OWNED and freed a lambda's capture |
| `scopes.rs` — the ownership-transition free (@FR-O-Latest) | free what an assignment displaces | no free: a local that owned a store and was then assigned a view leaked it |
| `scopes.rs` — the `owned_refs` TRACKING beside it | maintains the fact the block above reads | never updated, which made the two blocks that DID pair the spellings dead for a record enum as well |
| `scopes.rs` — the inline-call lift | give an unbound call result a name to free | the enum arm carried only `!returns_borrowed_view()`, so a `__retbuf` delivery was not lifted: one orphaned store per evaluation |

**The shape worth carrying forward.** Only the first site was reachable by a user program
before the fix — the other three were *downstream of a gate that never opened for this
spelling*, so they had quietly specialised to the traffic the gate let through. Opening the
gate is what made them visible, and each showed up as a NEW failure of an existing test rather
than by reading: the first by the nightly debug-assertions gate, the rest by
`tests/data/ownership_corpus.loft`. That is the argument for widening a narrow shape test
**and then running the whole suite**, rather than reading the neighbours and declaring them
fine: a specialised downstream site looks correct in isolation because, for the traffic it
has ever seen, it is.

Two of the four are now folded onto `heap_def_nr` (the delivery arm, and the lift — which was
literally two adjacent arms asking one question with two different predicates). The other two
are hand-spelled pairs matching the two blocks beside them, which is the local convention
there.

### A stray NUL byte made four files invisible to `grep` — now gated

While tracing site 2 above, `grep -rn` insisted `ShowDb::has_visible_field` existed
nowhere, in a tree where a backtrace had just named it. `src/database/format.rs` — 99 KB
of source — contained **one NUL byte**, pasted into a comment that was quoting the
one-character string a bug produced. `grep` and `ripgrep` classify such a file as binary
and skip it **silently**: no match, no warning, no non-zero exit.

That is worth a gate rather than a fix, because the failure is invisible in both
directions — the file reads fine in an editor, and the search that misses it reports
success. loft's development model runs on grepping this tree, so a file the tools cannot
see is a hole in the METHOD, not in one search.

`tests/doc_hygiene.rs::no_source_file_is_invisible_to_grep` scans `src/`, `doc/claude/`,
`tests/scripts/` and `default/` and names the file, line and byte offset. **It found three
more on its first run** — `src/extensions.rs`, `doc/claude/CHANGELOG_TECHNICAL.md` and
`tests/scripts/57-json.loft` — all the same slip, and three of the four were the same
comment about loft#769's sentinel copy-pasted between files, carrying the byte each time.
One bad paste, four blind spots. All four now describe the NUL in words.

### The library-CI gate reddens library repos for reasons they cannot cure

Found 2026-08-21 opening the eight `unify-library-ci-fpm` adoption PRs. Six went green; two did
not, **neither for anything in the PR** — both branches only add workflow files. The gate is
versioned centrally (`loft-lang/loft@main`), so a change here reddens every library repo on its
next PR, landing on whoever opens one.

| Repo | Red because | Owner |
|---|---|---|
| `loft-libs-docs` #4 | `markdown/src/markdown.loft:261` — *"Cannot index text with 'unknown'"*. A **forward-referenced** `atx_heading_level` (defined :383, called :252) leaves the slice bound unresolved in pass 1. | ✅ **Already fixed** — `1133e272` + `d822dd91`, unmerged on `tuxedo-pln145`. Goes green when that reaches `main`. |
| `loft-libs-game` #11 | `missing: examples-index.tsv — run 'make examples-index'`. The `exindex` check landed here in `7786d28c` (2026-08-18); that repo's last CI run was 2026-08-17, so its first run after the change is its first red. | ✅ **Cured at the source** — the two cross-repo checks are now ADVISORY in a library repo (see below); this red becomes a job-summary note. Goes green when this branch reaches `main`. |

⚠⚠ **The `exindex` message prescribed a cure the repo does not have — FIXED.** `make
examples-index` maps to a target in **this** repo; `loft-libs-game` has no Makefile at all, and
no library repo carries the generator (CI checks loft out separately as `loft-src`). So the gate
told a library maintainer to run something they cannot run.

The check now tests the **target** repo for the Make target and, when it is absent, names the
form that actually works there:

```
missing: examples-index.tsv (this repo defines worked-example tags)
         generate it from a loft checkout, which owns the generator:
         EXAMPLES_REPO_ROOT=$PWD <loft>/scripts/check_doc_drift.sh write-examples-index
```

⚠ The first cut of that fix tested `-f Makefile` in the **current** directory, which is the loft
checkout the script runs from — so it kept printing the `make` form for every library repo. The
control caught it; without one it would have read as fixed. Coverage is unchanged (the index is
still required wherever tags are defined), which is the half worth keeping.

### ✅ Resolved — the two cross-repo checks are tiered

`examples` and `examples-index` now **gate inside loft and advise in a library repo**, by
this repo's own rule that a diagnostic gates iff ignoring it can produce a wrong result. A
dangling doc citation is a broken link, so it advises — loudly: the findings go to the
library PR's job summary in full, with the runnable regenerate command, which is the
`compat check` pattern (*"ADVISORY, never gating"*) applied to docs.

⚠ The scanner's own **selftests still gate everywhere** — a scanner that stops following
its documented rules is loft's bug whoever runs it — and `EXAMPLES_GATE=hard` restores
blocking for a repo that wants it. Full rationale: [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md)
§ Why a by-hand pass exists.

⚠⚠ **And the deeper smell is fixed too: `examples-index.tsv` is no longer committed in a
library repo.** It is DERIVED and its generator lives in loft, so a copy committed there
could only rot — it cannot be regenerated where it sits. CI now emits it every run
(`check_doc_drift.sh emit-examples-index`), folds it into the job summary and uploads it as
an artifact. A derived file that is never committed cannot be stale, which retires the
failure mode instead of downgrading it. loft keeps its own committed copy, because loft owns
the generator and a greppable offline index is what the agent development model runs on.

⚠ Found while doing it: the file's header claimed it existed so *"loft's cross-repo `idx`"*
could resolve tags without a checkout. Measured — `scripts/idx` does not open it and never
has, and `check_examples` resolves through a local checkout. Its only automated consumer was
the check verifying it. That is what made deleting it safe, and it is why the check was worth
reading before the file was trusted.

⚠ The `loft-libs-docs` row is the sharper lesson: **a published library stopped compiling and
nothing said so for three weeks**, because that repo's CI had not run since 2026-07-28. A gate
that only runs on a PR cannot report a break that arrives from outside the repo.

### JSON cluster

| Item | Section | Status |
|---|---|---|
| **P54** — `JsonValue` enum (active sprint) | [§ Active sprint — P54](#active-sprint--p54-jsonvalue-enum) | Multi-step transition from text-based JSON to first-class `JsonValue` enum |
| **Q1** — JSON parse-error diagnostics | [§ Active design — Q1](#active-design--q1-json-parse-error-diagnostics) | **CLOSED 2026-08-20.**  The one-stage `Struct.parse(text)` filed its diagnostics only under `#errors`, and `json_errors()` reads the other register — so the documented JSON pairing answered `""` for malformed input AND for a schema mismatch, while the program read a struct of zeros.  Nothing cleared the json register there either, so `json_errors()` after a SUCCESSFUL one-stage parse still returned an earlier call's error (a validator reporting failure on correct data), and a `vector<T>.parse` reported an error naming a different type entirely.  One entry point now files both: cleared on entry, written on failure.  Guarded by `tests/scripts/json-one-stage-parse-reports.loft` on both backends |
| **Q2** — free-form object iteration + kind peek | [§ Active design — Q2](#active-design--q2-free-form-object-iteration--kind-peek) | API for iterating untyped JSON + peeking at value kinds |
| **Q3** — `to_json` serialiser | [§ Active design — Q3](#active-design--q3-to_json-serialiser--struct-serialisation) | Symmetric serialisation API to `Type.parse()` |
| **Q4** — `JsonValue` construction in loft code | [§ Active design — Q4](#active-design--q4-jsonvalue-construction-in-loft-code) | Builder API for constructing `JsonValue` trees in loft |
| **P54-U** — unified JSON parser | [§ Active design — P54-U](#active-design--p54-u-unified-json-parser) | Phase 3 deletes ~540 lines of legacy scanner |

**Q3 correctness — narrow nullable fields serialised wrong, fixed 2026-08-20.**
`T.to_json()` delegates to the `ShowDb` schema walker, and that walker re-derived "is this
slot null?" per width instead of asking `Stores::is_null`. Every re-derivation compared a
DECODED value against a RAW sentinel, so on one struct it produced three wrong values in a
single line:

```
struct Cfg { name: text, port: u16?, gain: i16?, level: integer limit(10,255)? }
Cfg { name: "x", port: null, gain: -300, level: null }.to_json()
  was  {"name":"x","port":-2147483648,"gain":-301,"level":265}
  now  {"name":"x","gain":-300}
```

`port` was an absent value serialised as a number, `level` likewise, and `gain` was a
PRESENT value one too low — a nullable SIGNED narrow slot sacrifices its bottom value to
the null code, so its ops encode against `min + 1` while the schema carried the declared
`min`. All three are on the wire, so a consumer of that JSON read numbers where the value
was absent. Fixed at the one home (`is_null` + `IntegerSpec::part_min`); guarded by
`tests/scripts/narrow-null-render.loft` on both backends.

**Q1/P54 — an absent field is the DECLARATION's question, and the second JSON walker was
answering it itself — CLOSED 2026-08-22.** The row this replaces named three widths
(`u16?` → `0`, `i16?` → `-32767`, `boolean?` → `false`) and pointed at
`structures.rs`'s clear-to-default arm. **Re-measured, it was stale in one direction and
far too narrow in the other**, and both halves of that are worth keeping.

*Stale:* those three widths were fixed at exactly the named site before this pass —
`set_default_value_nullable` writes `i32::MIN` for a `Short` and `255` for a boolean, and
its own comment quotes the `0` / `-32767` the row reports. That fix is on `main`. Re-run,
the one-stage `T.parse(text)` is clean at every width.

*Too narrow:* the same probe on the **two-stage** `T.parse(json_parse(text))` — the form
this document recommends because it reports diagnostics — was wrong about nearly every
absent field, and the failing widths were the COMPLEMENT of the ones filed:

```
struct D { u: u8?, b: boolean?, i: integer, t: text, dd: u8 = 9 }  ..parse("{}")
  two-stage was  u=0     b=false  i=null  t=null  dd=0
  one-stage      u=null  b=null   i=0     t=""    dd=9
```

So an absent `u8?` read back the VALUE `0`, an absent non-null `integer` read back **null
in a slot `(N-Decl)` says cannot hold one**, and a declared field default was ignored
outright. `u16?`/`i16?` passed here only by luck — `Parts::Short` reserves the raw code
`0`, so zero-init bytes happen to spell its null.

And the same gap **dropped narrow fields that WERE given a value**, which is data loss
rather than a sentinel mix-up: `u8?` handed `42` read back `0`, `u16?` handed `300` read
back `null`, non-null `u8` handed `7` read back `0`, and `vector<u8?>` given `[1,null,3]`
read back `[0,0,0]`. A narrow field simply had no arm in that walker; it fell to
`_ => { /* not yet handled … Leave at zero-init default */ }`. Wrong-KIND values fell
there too, so the mismatch report never named them either.

Root cause is one sentence: **`populate_struct_from_jsonvalue` answered "what does an
absent field hold?" per type, instead of asking the declaration.** The rule it broke is
[formal/layout.md](formal/layout.md) `(L-Null)` — absence is a sentinel inside the slot's
own bytes, so the sentinel is part of the LAYOUT and belongs at one address. Both walkers
now call `Stores::write_absent_value` (declared default, else the type's absent value) and
`Stores::write_narrow_value` (the four narrow encodings); the one-stage walker's four
open-coded narrow arms and its two copies of the absent-field decision collapsed onto the
same two calls, so there is no second spelling left to drift. Guarded by
`tests/scripts/json-walker-absent-field.loft` on both backends.

⚠ **The blast radius is wider than `T.parse`.** `engine_host.rs`'s live build swap
(@PLN18 08-S5) restores a running world into the new process through this same walker —
`swap_world_impl` reads the snapshot with `json_parse` and hands it straight to
`populate_struct_from_jsonvalue`. The snapshot is WRITTEN by loft's schema walker, whose
narrow-null spelling was fixed on 2026-08-20, and read by this one, which was not: so
every narrow field in a swapped world came back at its zero and every nullable narrow
field came back as a number, silently, on hot reload. The round-trip is now pinned as an
identity (`test_snapshot_restore_is_the_identity`) — worth having because a snapshot test
written the obvious way is closed under its own encoder and would have passed throughout.

**Two method notes, both measured here.** First, *a filed table is a hypothesis about
which cells are broken, not just about why* — this one had the right symptom, the wrong
widths, the wrong route and a suspect that was already fixed, and re-running the probe
cost less than reading the row carefully. Second, **the reference implementation is the
oracle**: every cell above is scored against what the one-stage route answers for the same
document, which is what turned "some widths look odd" into a complete list.

**`record#errors` is a text, and the docs said "iterate" — OPEN (design, XS-if-decided).**
Both `STDLIB.md` and `LOFT.md` showed `for e in user#errors { log_warn(e); }`.  That
iterates NOTHING: the accessor returns one newline-separated text, the for-loop
re-evaluates it per iteration, and the read CLEARS it — so the first evaluation empties the
register and the loop ends.  Bound to a variable first it iterates the message's
CHARACTERS, which is what `for` over a text means.

Measured, not inferred: `for e in a#errors` → 0 iterations; `t = a#errors; for e in t` → 14
(the message's length).

The docs now show the shape that works (read it into a variable and test it), so nobody is
taught a no-op.  What is NOT decided is whether the accessor should BE a collection —
`vector<text>`, one entry per error — which is what the old text promised.  That is a
compatibility call, not a bug fix: `tests/scripts/197-struct-json-parse-errors-dest.loft`
depends on the text shape (`len(b1#errors) > 0`, `b2#errors != empty`,
`"[{b3#errors}]"`) and on the clear-on-read, and it says so in its own comment.  Deciding it
belongs to [COMPATIBILITY.md](COMPATIBILITY.md)'s process, not to a fix pass.

### Native runtime cluster

| Item | Section | Status |
|---|---|---|
| **Dep-inference** — for native fn returns (zero-leak unblock) | [§ Active design — Dep-inference](#active-design--dep-inference-for-native-fn-returns-zero-leak-unblock) | **SHIPPED; row was stale — re-measured 2026-08-20.**  The inference is in `parser/definitions.rs` (a native `;`-terminated fn with a `self` parameter returning the SAME struct-enum gets `dep=[0]`), and the bite it was written for is gone: 1200+ constructor calls and 1000 accessor-chain calls in loops leave the ledger balanced (1213 allocs / 1211 frees, peak 9 live) on both backends.  Guarded by `tests/scripts/json-value-chain-ownership.loft`.  See the caveat under § Active design — Dep-inference: the inference no longer changes any behaviour I could measure |

### Compiler-blocker cluster

| Item | Section | Status |
|---|---|---|
| **B2-B7** — struct-enum bugs gating P54 | [§ Compiler blockers](#compiler-blockers--struct-enum-bugs) | **AUDITED + CLOSED 2026-05-21 on BOTH backends.**  18 `p54_b*`/`b7_*` interpreter guards green (0 ignored); the one native-only residual (@P301 — struct-enum returned via an intermediate local) was fixed the same day (added the `Type::Enum(_, true, _)` arm to `add_defaults`) and is guarded cross-mode by `tests/scripts/121-struct-enum-return-local.loft`. |

### Store-lifetime cluster

| Item | Section | Status |
|---|---|---|
| ~~**The per-execution ownership witness**~~ — the cluster is DISSOLVED: its premise was measured false | [formal/closures.md](formal/closures.md) D-clo-7 (CLOSED) / D-clo-14 (CLOSED) / D-own-16 (CLOSED); D-own-8 (CLOSED 2026-09-03, loft#1320/#1321/#1323 — every arm of a bound value branch its own binding; its two declined shapes took a witness SNAPSHOT of the base at the bind, the one slot the cluster predicted, and only where the base cannot stand witness itself) | **DISSOLVED 2026-09-03 — all three closed by the day's end, none with a witness slot: D-clo-7 fell to reading the capture SLOT off the callee's body, which made its base nameable after all.**  Two of the three closed with NO witness, and the third is residual.**  The identity route was measured against both closure rows: `D-clo-14` is closed for every spelling but a call in an `if`/`match` ARM (guard `1257b-a-lifted-collection-return-is-freed-by-identity.loft`, falsified at d9a2ec21 on both backends; 389 live stores at N=400 -> 1, the keyed kinds included), and `D-clo-7` is NOT reached — its return dep names `__closure`, so there is no base to compare against.  **The axis was never "is there a witness" but "is there a NAMEABLE base"**, and reading it the first way is what kept three rows filed as one piece of work for a month.  What follows is the original entry, kept because its measurements and its two blind-alley warnings are still true.  **OPEN, and the cluster's PREMISE is now known to be too strong.**  `D-own-16` was its third member and closed WITHOUT a witness: where the type already NAMES the variable a local might be aliasing, the owner is decidable at run time by store IDENTITY (`OpFreeRefIfDistinct` against that dep), which costs no witness slot.  So *"a runtime witness is the only mechanism"* is false as stated; the open question for the two closure rows is narrower — whether either has a nameable variable to compare against, and `D-clo-7`'s remaining half is exactly the case where the return dep names `__closure` and not WHICH slot, i.e. where there is none.  Recorded 2026-09-02 as a cluster because they are one piece of work.  Each is a store whose owner is decidable only at RUN time, and each is stuck at the same place: nothing static separates the arm that MINTS from the arm that HANDS BACK a caller's store, because they are the same call.  Where a witness exists the question is already answered — `OpBindOrCopy` settles the `Reference` / struct-`Enum` join per execution (D-clo-7's argument half, loft#1248), and `OpFreeRefIfDistinct` frees a placeholder only when it is genuinely a distinct store.  What has no witness: a COLLECTION join (D-clo-14, where the cure was first to DECLINE the lift and pay a leak, and is now to compare the store against the `Join` base the temp's own dep names), a capture whose return dep names only `__closure` and not WHICH slot (D-clo-7's remaining half), and a local reassigned from a join over ITSELF (D-own-16 — `c = mk(i) ?? c`, where freeing the displaced store before the assignment is a use-after-free on the arm that takes it).  ⚠ **Do not measure D-clo-14 on the program-exit leak channel**: its stores are freed at FRAME exit, so `Warning: N stores not freed` is silent for the life of the defect — `LOFT_ALLOC_SITES=1` reads 389 live stores at N=400 against 1 when the lift is allowed.  ⚠ **And a cure needs BOTH frees**, which is why a half-fix failed before: the loop-scope `OpFreeRef(__lift_N)` in `scopes.rs`, AND the implicit dep-empty pre-Set free on the next iteration's reassignment, which lives in codegen.  Taking these as one plan is what stops the third rediscovery of the same wall.  *(Both are answered by ONE change — putting the base on the lifted temp's TYPE, which stops the pre-Set free being emitted at all and leaves `get_free_vars` to guard the other.)* |
| **Cluster III Route 2** — reassignment store-free across shared blocks | [plan-57](plans/2-vector-store-watermark/cluster-III-reassignment-pin.md) | **CLOSED 2026-08-21 — shipped, default ON.**  A local reassigned across sibling `if`/`else if`/`match` arms kept EVERY arm's store to scope exit, so the watermark grew with the number of reassignment SITES and not with how many ran: a 16-site function measured peak 20 whichever single arm was taken.  Route 2 (`recover_backer`) confines each block's store to its block — peak is now a flat 5 at 2/4/8/16 sites.  It had sat behind `LOFT_CONF_RECOVER` since 2026-06 pending an un-gate decision; re-measured on 4232 tests (2.2× the evidence it was parked on) and un-gated.  `LOFT_NO_CONF_RECOVER=1` is the opt-out and the first bisect step for a wrong answer in such a function.  Soundness is `store_dead_after_block`, not the flag: a local READ after the blocks does not confine, because freeing a confined store while the local still holds it returns the wrong element on the branch NOT taken.  Both branches verified green (4232 each), values identical on both backends, pinned by `tests/scripts/reassign-across-sibling-blocks.loft`. |
| **@PLN85 cluster I** — FFI struct-return read gap (latent) | [@PLN85 README probe 01](plans/85-store-lifetime-retirement/README.md) | **Latent / unreachable** residual of the now-closed @PLN85.  A `#native` fn returning a non-vector **struct** has no `alloc_struct` helper to lay out a loft-readable ref, so the read path can't be exercised — gated on a future struct-return helper.  The reachable FFI **vector**-return instances (#409/#410) are FIXED.  Re-probe when the helper lands. |
| **@PLN85 cluster IV** — @PLAN51 hidden-buffer-aliasing latent residuals | [@PLN85 README cluster IV](plans/85-store-lifetime-retirement/README.md).  Cites the `@PLAN51` probe set, preserved at [`plans/finished/51-hidden-buffer-aliasing/`](plans/finished/51-hidden-buffer-aliasing/) — the **legacy local plan** `@PLAN51`, *not* the tracker issue `@PLN51` (which is "[audience] Bumper-airplanes") | **RE-MEASURED 2026-08-21: 3 of 62, not ~11, and INTERPRETER-only — the row's "native-mostly" was inverted.**  All 62 probes run clean on both backends (exit 0); `40-nested-tuple-of-canvases`, `47-tuple-local-not-return` and `51-tuple-as-arg` leak on `--interpret` and are clean under `LOFT_NATIVE_LEAK_CHECK=1`, matching @PLAN51 cluster II's own note that *"`--interpret` leaked; `--native` was always clean"*.  The lambda/operator-vector-return, capture-heap-return and mixed-lit-call shapes the row named are now clean.  All three reduce to ONE mechanism, filed as **loft#1051** and **FIXED 2026-08-21**; guarded by `tests/scripts/1051-tuple-destructure-ownership.loft` on both backends.  ⚠ **The root cause recorded here on 2026-08-21 was WRONG, and the correction is the useful part.**  This row (and the issue) named `materialize_tuple_element` and concluded that "who owns a tuple's records" was an ownership-model question that had to be answered before the leak could be fixed.  Instrumented, that function is **never reached** for this shape — it serves the tuple-RETURN path (`t = pair()`), which was already clean, and its copy does not appear in the IR dump because it is inserted during IR-to-bytecode.  The real site is `codegen.rs`'s `gen_set_first_ref_tuple_copy`, which deep-copied a `Type::Reference` element UNCONDITIONALLY: the copy allocates a store and makes the binding its owner, while the binding's `deps` said BORROW, and a borrow is skip-free.  No ownership question needed answering — [formal/ownership.md](formal/ownership.md) **O-Deps** had already answered it (*the free DERIVES from `deps`; a codegen condition that re-derives it is the bug*), and the element-read arm three lines above carried the correct guard, `depend().is_empty()`, for exactly this reason.  Adding that same guard is the whole fix.  It also closed a backend divergence (O-NoDiverge): `--native` reads the same deps and aliases a borrow, which is why it was always clean.  **Why the earlier attempt failed and read as "the model must change first":** the dep was stripped in the parser, at a site this shape never executes, so nothing moved and the no-op looked like evidence for a deeper problem.  **Instrument note:** `LOFT_STORES=warn` reports NOTHING for these (below its floor, as this row said); `LOFT_STORES=timeline` states leak status explicitly and is what separated 3 from 59. |

| **@PLN130 Q6** — which uncovered copy families should be accepted rather than eliminated | [COPY_DIAGNOSTICS.md § What remains open](COPY_DIAGNOSTICS.md) | **Design question, framing settled.**  The owner's rule is recorded — an accept written at the SITE, never a blanket env exemption, so the decision stays reviewable — but which of the measured families qualify is not chosen.  Blocks nothing: an unaccepted copy is reported, which is the state the model requires. |
| **@PLN130** — the uncovered copy set | [COPY_DIAGNOSTICS.md § What remains open](COPY_DIAGNOSTICS.md) | **RE-MEASURED 2026-08-21 over the WHOLE corpus: 504 distinct uncovered sites across 811 scripts (`--interpret`), not "29 over a 90-script sample"** — `InterpCallReturn` 225, `InterpReassignCall` 177, `InterpRecordBind` 72, `InterpReassignVar` 30; a both-backends sample adds `NativeCallReturn` + `NativeRecordBind`, so **five** origins carry uncovered sites rather than four.  `InterpTupleBind` and `ParserMaterialise` never appear (the latter by design — it is classified `Implicit`).  **The guarantee HOLDS: the `Unknown` bucket is empty, 0 rows corpus-wide**, which is the claim that matters — every emitted copy is attributed to a named emitter.  Head of the set by type: `File` 47, `Box` 41, `A` 24, `S` 23, `__tuple` 13, with the stdlib's own `exists(path)` the most-cited shape (`file(path).format` lifts and copies a whole `File` to read one field — the case `Origin::InterpReassignCall`'s doc already names).  Still a legitimate `Avoidable` resting state under decision 3; what changed is that it is now ranked rather than merely sized.  **Cost is NOT established** — a record copy is cheap beside the syscall, and size should not be read as impact.  L effort. |

For the open programmer-biting issues list (running, not plan-shaped), see [§ Open programmer-biting issues](#open-programmer-biting-issues) above.  For ranked enhancement work, see [§ Enhancement tiers](#enhancement-tiers).  For ordering across all open items, see [§ Recommended landing order](#recommended-landing-order).

---

## Active sprint — P54 (`JsonValue` enum)

**Bite.** `MyStruct.parse(text)` silently returns a zero-valued struct
on malformed JSON — no type check, no runtime diagnostic — contradicting
loft's "static types catch mistakes" promise.

**Decision.** Replace the text-based JSON surface with a first-class
`JsonValue` enum.  `json_parse(text) -> JsonValue` is the one entry
point; `MyStruct.parse(JsonValue)` accepts only the typed tree; the
old `json_items` / `json_nested` / `json_long` / `json_float` /
`json_bool` family is withdrawn.

### Surface (`default/06_json.loft`)

```loft
pub enum JsonValue {
    JNull,
    JBool   { value: boolean },
    JNumber { value: float not null },
    JString { value: text },
    JArray  { items_id: integer },     // arena index — see § B5 workaround
    JObject { fields_id: integer },
}

pub fn json_parse(raw: text) -> JsonValue;
pub fn json_errors() -> text;
pub fn field(self: JsonValue, name: text) -> JsonValue;
pub fn item(self: JsonValue, index: integer) -> JsonValue;
pub fn len(self: JsonValue) -> integer;
pub fn as_text(self: JsonValue) -> text;
pub fn as_number(self: JsonValue) -> float;
pub fn as_long(self: JsonValue) -> integer;
pub fn as_bool(self: JsonValue) -> boolean;
```

Pattern matching falls out of existing struct-enum machinery:

```loft
match json_parse(raw) {
    JObject { fields_id } => { … },
    JArray  { items_id }  => { … },
    JNull                 => println("parse error: {json_errors()}"),
    _                     => println("unexpected root kind"),
}
```

### Status (2026-04-14)

| Layer | State |
|---|---|
| Stdlib enum + surface signatures | **Shipped** (`default/06_json.loft`) |
| Rust JSON parser (`src/json.rs`) | **Shipped** — full RFC 8259, 9 unit tests |
| `n_json_parse` (all variants — primitives + arrays + objects + nested) | **Shipped** — step 4 complete |
| `n_json_errors` | **Shipped** |
| `n_as_text`, `n_as_number`, `n_as_long`, `n_as_bool` | **Shipped** |
| `n_field` (JObject lookup), `n_item` (JArray index), `n_len` | **Shipped** — real arena reads, not stubs |
| `n_kind`, `n_has_field`, `n_to_json`, `n_to_json_pretty` | **Shipped** (Q2 / Q3) |
| `n_json_null`, `n_json_bool`, `n_json_number`, `n_json_string` | **Shipped** (Q4 primitives) |
| `n_keys`, `n_fields` (Q2 vector-returning) | **Shipped** — JObject walk allocates a result vector via `database()` + `vector_append`, deep-copies each name (`n_keys`) or each `JsonField` entry (`n_fields`, including container values via `dbref_to_parsed`) |
| `n_json_array`, `n_json_object` (Q4 containers) | **Shipped** (full deep-copy via `dbref_to_parsed`) |
| `T.parse(JsonValue)` codegen (step 5) | **Pending** |
| `T.to_json()` codegen (Q3 struct serialiser) | **Pending** (mirror of step 5) |
| Acceptance tests | **39+ green, 6 ignored** in `tests/issues.rs::p54_*` |

### Remaining steps

**Step 4 (arena materialisation) — COMPLETE 2026-04-14 (four slices in one day).**

**First slice — empty containers.**  `[]` and `{}` now materialise
as real `JArray` / `JObject` variants rather than the earlier
`JNull`-stub, because they have no children and so don't need the
arena allocator.  Specifically:

- `src/native.rs::n_json_parse` — new branches for
  `Parsed::Array(v) if v.is_empty()` and
  `Parsed::Object(v) if v.is_empty()` that set the correct
  discriminant byte + clear diagnostics.  Non-empty containers
  still fall through to the JNull stub with the "materialisation
  pending" diagnostic.
- `src/native.rs::n_len` — returns 0 for `JV_DISCR_ARRAY` /
  `JV_DISCR_OBJECT` (today every container is empty; when the
  full arena ships this path reads the arena vector length).
- `src/native.rs::json_to_text` (shared by `to_json` + pretty) —
  renders `"[]"` / `"{}"` for container discriminants.
- Regression guards in `tests/issues.rs` (12 total):
  - **Per-surface**: `p54_step4_empty_{array,object}_has_{jarray,jobject}_kind`,
    `…_len_is_zero`, `…_to_json`,
    `p54_step4_empty_array_round_trips_through_to_json`
    (parse→serialise→parse agrees on the empty-array
    discriminant), `p54_step4_nonempty_array_still_stubs_as_jnull`
    (prevents accidental partial impl that would claim wrong
    length).
  - **Cross-integration** (added 2026-04-14): locks the
    interactions between step 4's materialisation and the Q2
    (`has_field`) / Q3 (`to_json_pretty`) / existing (`field` /
    `item`) surfaces so a future refactor can't silently break
    the chain while keeping individual per-surface tests green:
    `p54_step4_empty_object_has_no_field`,
    `p54_step4_empty_{object_field,array_item}_lookup_returns_jnull`,
    `p54_step4_empty_{array,object}_pretty_matches_canonical`.

**Second slice — non-empty primitive arrays.**  Arrays whose
elements are all primitive variants (JNull / JBool / JNumber /
JString — no nested containers) now materialise as real JArray
with elements stored in an arena sub-record inside the root's
store.  The sub-record is allocated via `vector_append` (shared
with the rest of the stdlib's vector plumbing), so the entire
tree lives in one store and frees as one unit.

* `src/native.rs::n_json_parse` — new guarded branch for
  `Parsed::Array(v) if v.iter().all(matches!(Null|Bool|Number|Str))`;
  pre-initialises the JArray items field, calls
  `vector_append` per element, delegates to the helper
  `materialise_primitive_into(stores, slot, child)` for the
  discriminant + payload write.
* `src/native.rs::n_len` — JArray arm now reads the arena
  vector's length word at offset 4 (empty arrays still return
  0 via the `items_rec <= 0` guard).
* `src/native.rs::n_item` — full implementation: dispatches on
  JArray discriminant, walks to the i-th slot via
  `8 + i * sizeof(JsonValue)`, returns a borrowed DbRef into the
  parent's store.  Out-of-range indices / non-JArray receivers
  return a fresh JNull.
* `src/native.rs::json_to_text` — JArray recursive rendering:
  walks each arena slot and recurses via `json_to_text` so
  mixed-primitive arrays serialise correctly.  Empty arrays
  still render `"[]"` via the same branch.
* `materialise_primitive_into` helper — one-line-per-variant
  dispatch that writes discriminant + payload into a
  pre-allocated JsonValue slot.  Shared by the
  vector-append path today; nested-container handler (later
  slice) will call it for leaf rewrites.
* Closed one ignored test: **`p54_parse_array_item_access`**
  (was `#[ignore]` "P54 step 4: parse_array + item() indexed
  access") is now green.  Baseline drops from 8 → 7.
* Regression guards in `tests/issues.rs` (9 new):
  `p54_step4_nonempty_primitive_array_has_jarray_kind`,
  `…_length_correct`, `…_item_0_is_first`,
  `…_item_1_is_middle`, `…_item_out_of_range_returns_jnull`,
  `p54_step4_nonempty_bool_array_item_kind`,
  `p54_step4_nonempty_string_array_item_value`,
  `p54_step4_nonempty_array_to_json_round_trips`,
  `p54_step4_nonempty_array_to_json_text_shape` (e.g.
  `json_parse("[1,2,3]").to_json()` = `"[1,2,3]"`).
* Negative guard retained:
  `p54_step4_nested_array_still_stubs_as_jnull` — arrays
  containing other arrays (`[[1,2],[3,4]]`) still hit the
  stub; the nested-container materialiser is a later slice.

**Third slice — non-empty primitive objects.**  Objects of the
shape `{"k1": v1, "k2": v2, ...}` where every value is a
primitive (no nested containers) now materialise as real
JObject variants with `JsonField { name, value }` entries
stored in an arena sub-record.  Same arena pattern as the
JArray slice, plus a per-element name-text write.

* `src/native.rs::n_json_parse` — new guarded branch for
  `Parsed::Object(v) if v.iter().all(|(_, p)| primitive)`;
  allocates the fields-vector sub-record via `vector_append`,
  writes the name text via `set_str`, then delegates to
  `materialise_primitive_into` for the nested JsonValue slot.
* `src/native.rs::n_len` — JObject arm now reads the arena
  vector's length at offset 4.
* `src/native.rs::n_field` — full implementation: dispatches
  on JObject discriminant, linear-scans the JsonField vector
  comparing each name to the query, returns a borrowed DbRef
  into the matched slot's `value` field (or fresh JNull on miss).
* `src/native.rs::n_has_field` — real implementation: same
  linear scan, returns boolean instead of a DbRef.  No longer
  a forward-compatible stub.
* `src/native.rs::json_to_text` — JObject arm recurses into
  each JsonField slot, writes `"<name>":<value>` pairs with
  the same escape rules as JString keys.
* Closed one ignored test: **`p54_parse_object_field_access`**
  (was `#[ignore]` "P54 step 4: parse_object + field() chained
  access") is now green.  Baseline drops from 7 → 6.
* Regression guards (`tests/issues.rs`, 9 new):
  `p54_step4_nonempty_primitive_object_has_jobject_kind`,
  `…_length_correct`, `p54_step4_nonempty_object_field_{hit,miss}_...`,
  `p54_step4_nonempty_object_has_field_{hit,miss}`,
  `p54_step4_nonempty_object_to_json_text_shape`,
  `p54_step4_nonempty_object_to_json_round_trips`,
  `p54_step4_nonempty_object_mixed_primitive_values`.

**Fourth slice — nested containers.**  Arrays-of-arrays,
objects-of-objects, and arbitrary-depth mixes all materialise
now.  `materialise_primitive_into` (despite its now-anachronistic
name) was extended with `Parsed::Array` and `Parsed::Object`
recursive arms.  Each nested container's items / fields vector
is allocated via `vector_append` in the **slot's own store**, so
the entire tree shares the root JsonValue's store and frees
together (File-pattern arena).  `n_json_parse`'s previous
all-primitive-only guards on the array / object branches were
dropped — both now unconditionally call into the recursive
helper.  The earlier "materialisation pending" stub branch was
deleted (no longer reachable).

* Sites: `src/native.rs::materialise_primitive_into` — added
  Array + Object arms; `src/native.rs::n_json_parse` — removed
  primitive-only `where v.iter().all(...)` clauses, removed
  fallback stub branch.  Simpler control flow.
* Negative-stub regression `p54_step4_nested_array_still_stubs_as_jnull`
  REPLACED with positive `p54_step4_nested_array_materialises`.
* Regression guards (`tests/issues.rs`, 9 new):
  `p54_step4_nested_array_outer_length`, `…_inner_length`,
  `…_inner_item_value` (3-deep navigation),
  `p54_step4_nested_object_chained_field` (chained `.field()`),
  `p54_step4_array_of_objects_field_lookup` (mixed: outer
  array, inner object),
  `p54_step4_object_with_array_field` (mixed: outer object,
  inner array — locks both directions of recursion),
  `p54_step4_nested_array_to_json_text_shape` (`[[1,2],[3,4]]`
  serialises canonically),
  `p54_step4_object_with_array_to_json_text_shape`
  (`{"k":[1,2]}` serialises canonically).

**Step 4 status:** **COMPLETE.**  Every JSON document `json_parse`
now produces a fully materialised JsonValue tree.  The arena
contract holds: one root store per parse, all sub-records frees
together when the root DbRef leaves scope.  Q2 `keys` / `fields`,
Q3 nested-container serialisation, and Q4 container constructors
remain — but they now sit on a working arena, not a stub.
The recursive enum form `JArray { items: vector<JsonValue> }` trips
B5.  Workaround: arena indirection — children are stored in a per-parse
allocation and referenced by integer index (`items_id`, `fields_id`).
The arena is allocated in the **same store** as the root JsonValue so
the entire tree frees as one unit when the root DbRef goes out of
scope (the `File` pattern, not `stores.database()`).

**Current state (2026-04-14 explore walk).**
* `src/native.rs:1316-1323` — `jv_alloc` allocator stub: calls
  `stores.database(words.max(2))` and claims a fresh single-record
  store per JsonValue.  This is the file-pattern bottleneck —
  nested children want to share the root's store, not each get
  their own.
* `src/native.rs:1325-1392` — `n_json_parse` materialises
  primitives (JNull / JBool / JNumber / JString) via discriminant +
  variant-field writes.  Arrays and objects hit the
  "materialisation pending (P54 step 4)" diagnostic at line 1379.
* `src/native.rs:1401-1453` — `n_as_text` / `n_as_number` /
  `n_as_long` / `n_as_bool` extractors are **real** (not stubs).
* `src/native.rs:1461-1488` — `n_field`, `n_item`, `n_len` return
  JNull / `i32::MIN` stubs.

**Step 4 change set.**
1. Extend `jv_alloc` (`src/native.rs:1316-1323`) to accept an
   optional parent store so nested allocations land in the root's
   store.  New signature: `jv_alloc_arena(stores, root, words,
   children_count) -> DbRef`.
2. Extend `n_json_parse` (`src/native.rs:1325-1392`) to walk
   `Parsed::Array` / `Parsed::Object` recursively — materialise
   each child via `jv_alloc_arena(root, …)`, write the arena
   record index as `items_id` / `fields_id` on the variant payload.
3. Replace the three dispatch stubs at `src/native.rs:1461-1488`
   with real implementations: read the discriminant byte, dispatch
   on JArray/JObject, fetch the arena record, index or search by
   name, return `JNull` on absent / OOB.

**Step 5 (`Type::parse(JsonValue)` codegen).**  Per-struct unwrap that
walks the schema, calls `n_field` for each declared field, converts
via the `n_as_*` extractors, stores into the destination.  Site:
`src/parser/objects.rs:568-584` (`parse_type_parse`).  Today that
function is text-only — argument coerced to text, emitted as
`OpCastVectorFromText`.  Step 5 adds a JsonValue-unwrap path branch
before the text branch; step 6 rejects plain text for struct
targets at the same site.

**Field-type matrix** (explicit policy — the P54 bite was silent
field-level zeroing; this spells out the replacement):

| Declared field type | JSON produces | Target value |
|---|---|---|
| `text` | `JString`        | value |
| `text` | anything else    | null text + diagnostic |
| `integer` | `JNumber` (integral) | value |
| `integer` | `JNumber` (fractional) | null + diagnostic (lossy cast) |
| `float` | `JNumber` | value |
| `boolean` | `JBool` | value |
| `T` (nested struct) | `JObject` | recurse `T.parse(subtree)` |
| `vector<T>` | `JArray` | iterate + `T.parse` each element |
| `JsonValue` (explicit typing) | any kind | capture the subtree verbatim — the hybrid case, lets typed ingestion coexist with deferred free-form inspection |
| any | `JNull` | declared default |
| any | missing field | declared default |

**Strict vs. permissive** (opt-in per call):

```loft
u = User.parse(v);                  // permissive (default)
u = User.parse(v, strict: true);    // rejects on any deviation
```

- **Permissive** (default): missing fields, extra fields, and
  type-mismatch leaves keep the declared default.  Every deviation
  appends an entry to `json_errors()` so users can opt in to
  diagnostics even without `strict`.  This matches how loft's
  `null`-sentinel discipline is used elsewhere — absence is not
  failure.
- **Strict**: first deviation returns `null` at the top-level
  `parse` call, and `json_errors()` contains the full list of
  deviations with their paths (via Q1 infrastructure).

**Diagnostic shape** (Q1 path + line:column extend to schema errors):

```
User.parse error at /users/3/age (byte 12847, line 423 col 20):
  expected integer, got JString "thirty"
```

`vector<T>.parse(v)` — when a top-level array maps to a homogeneous
vector of T, the same machinery applies per-element.  Each
mismatched element appends a path `/N` diagnostic.

**Root-shape rules**:
- `T.parse(v)` where `v` is not `JObject` → returns `null`, logs
  `"expected JObject at /, got JArray"`.
- `vector<T>.parse(v)` where `v` is not `JArray` → returns an empty
  vector, logs `"expected JArray at /, got JObject"`.

**Step 6 (gate `MyStruct.parse(text)`).**  Same parser site.  If the
argument type is `Type::Text(_)` and the target is a struct, emit
`"MyStruct.parse expects a JsonValue, got text — call json_parse(text)
first"`.  Migration blocked: `tests/scripts/57-json.loft` and
`tests/docs/24-json.loft` have ~20 legitimate `Struct.parse(text)`
sites that must be rewritten first.

**Step 7 (unignore acceptance tests).**  13 `#[ignore]`'d in
`tests/issues.rs::p54_*`.  Each goes green automatically as the
corresponding layer lands.  Five of those — the text-return-through-fn
family + chained-access — depend on **B7** in § Compiler blockers
below; one fix unblocks all five.

**Step 8 (docs).**  LOFT.md JSON section in pattern-matching chapter;
STDLIB.md JSON chapter; CHANGELOG entry.

### Acceptance

`cargo test --release --test issues p54_` — all 39+ tests green.
Brick Buster / Moros editor read JSON via the new surface.  No call
site in `default/`, `lib/`, or `tests/` uses `Struct.parse(text)`.

---

## ~~C54 — integer i64~~ — LANDED 2026-04-21

Shipped via @PLAN01 (`doc/claude/plans/finished/01-integer-i64/`).
`integer` is i64 end-to-end; `Type::Long` / `long` keyword / `l` literal
suffix removed; 34 duplicate `Op*Long` opcodes reclaimed; binary-format
lint in place; `.loftc` cache removed.  C54.D (Rust-style literal
suffixes) was closed by decision — see `DESIGN_DECISIONS.md`.  User-
facing shape: CHANGELOG.md § "Integer → i64 migration".

---


## Active design — Q1 (JSON parse-error diagnostics)

**Bite.** `json_errors()` today returns `"{msg} (byte {at})"` — a
human-readable message plus the raw byte offset into the source.  For
a 50 KB configuration file or an API response, this is effectively
unusable: users can't tell *which field* failed, what line:column to
open the file at, or what the surrounding JSON looks like.  The whole
P54 pitch is "typed tree catches what `Struct.parse(text)` used to
silently swallow" — that win is half-delivered if the diagnostic on
failure is `byte 12847`.

**Status (2026-04-13).**  Parser side **shipped**.
`src/json.rs::parse` returns `Result<Parsed, ParseError>` carrying
`message`, `byte_offset`, and an RFC 6901 `path`.  Path-stack is
threaded through `parse_object` / `parse_array`.  `format_error`
builds the line:column + context snippet on demand.  `n_json_parse`
calls it; `json_errors()` returns the rich text.

```
err: parse error at line 1 col 9 (byte 8):
  path: /x
  expected digit after `.`
    1 │ {"x": 1.}
      │         ^
```

8 unit tests in `src/json::tests` (path for root / array index /
object field / nested / RFC 6901 escapes; line:col conversion;
format_error covering path / line / col / caret) plus 4
acceptance tests in `tests/issues.rs::q1_*` (path for object
field, path for array index, caret marker present, line+byte
markers present).

**Schema-side still pending**: `Type::parse(JsonValue)` failures
will reuse the same path + format_error infrastructure when P54
step 5 lands.  Recovering parser (continue past first error,
return list of failures) remains a follow-up with its own
trade-offs.

### Target diagnostic

```
parse error at line 423 col 17 (byte 12847):
  path: /users/3/address/zip
  expected digit after `.`
    421 │       {
    422 │         "address": {
    423 │           "zip": 1.}
                          ^
    424 │         }
```

Three pieces, each independently useful:

1. **JSON Pointer path (RFC 6901).**  `/users/3/address/zip` — names
   the field.  Accumulated during descent: push `/users` entering
   that object's field, push `/3` entering the array element, …  On
   error, the current path is the location.  Storage: `Vec<String>`
   in the parser; push on descent, pop on ascent.

2. **Line:column.**  One pass over `bytes[0..offset]` counting `\n`
   converts the byte offset at error time.  O(n) but only executed
   on failure, not per token.

3. **Context snippet.**  Two lines before, the error line with a
   caret under the offending byte, one line after.  Trivial once
   line:column is known.

### Surface changes

**`src/json.rs`:**

```rust
pub struct ParseError {
    pub message: String,
    pub byte_offset: usize,
    pub path: String,        // RFC 6901 pointer; "" for root
}

pub fn parse(input: &str) -> Result<Parsed, ParseError>;
```

Internal parser functions gain a `&mut Vec<String>` path stack.
`parse_object` pushes `/escape(name)` before recursing on each
field's value, pops after.  `parse_array` pushes `/{index}` and
pops the same way.  Push/pop is O(1); no extra allocation per
token.

RFC 6901 escaping: `~` → `~0`, `/` → `~1`.  Five-line helper.

**`src/native.rs::n_json_parse`:**

```rust
Err(ParseError { message, byte_offset, path }) => {
    let (line, col) = line_col_of(raw.as_bytes(), byte_offset);
    let snippet = context_snippet(raw, byte_offset, 2, 1);  // 2 before, 1 after
    stores.last_json_errors.clear();
    stores.last_json_errors.push(format!(
        "parse error at line {line} col {col} (byte {byte_offset}):\n\
         \x20 path: {path}\n\
         \x20 {message}\n\
         {snippet}"
    ));
}
```

Multiple errors: keep `Vec<String>` shape; future step (not this
landing) can teach the parser to continue past recoverable errors
— `json_errors()` would then return one line per failure.  For
today's single-error-at-first-fail parser, the Vec holds one well-
formatted entry.

**`default/06_json.loft`:**
No change — `json_errors()` signature (`-> text`) is already the
right shape.  What callers *see* in that text becomes useful.

### Implementation cost

~60 lines in `src/json.rs` (`ParseError` struct, path-stack plumbing
in 6 parse functions, RFC 6901 escape helper, line:column converter,
context-window formatter).  ~20 lines in `n_json_parse` to replace
the tuple-destructure with the rich format.

### Tests (landed 2026-04-14)

All five spec-named acceptance tests live in `tests/issues.rs`:

- `p54_err_reports_path_into_nested_object` — parse of
  `{"a": {"b": 1.}}` reports `/a/b`. ✅
- `p54_err_reports_path_into_array_element` — parse of
  `[1, 2, 1.]` reports `/2`. ✅
- `p54_err_reports_line_and_column` — 3-line input fails on
  line 2, diagnostic contains `line 2`. ✅
- `p54_err_context_snippet_includes_caret` — snippet carries
  a `^` under the offending column. ✅
- `p54_err_path_escapes_slash_and_tilde` — a field named
  `a/b~c` renders as `/a~1b~0c` in the diagnostic (RFC 6901
  escape round-trips through `n_json_parse`). ✅

Supporting coverage:
- `src/json::tests` — 8 unit tests covering `parse` path
  threading (root / array / object / nested / RFC 6901),
  `line_col_of` on simple + multi-line input, and
  `format_error` shape (path / line / col / caret / message).
- `tests/issues.rs::q1_*` — 6 acceptance tests covering
  state-clearing (`cleared_after_successful_parse`,
  `empty_after_clean_parse`), path substrings, and format
  shape assertions.

### Why Tier 2 (not Tier 1)

This doesn't unblock any ignored test and doesn't close a crash.
It's an *ergonomics* win that substantially improves the P54 value
proposition.  Landing it inside the P54 sprint — between step 5
(`Type::parse(JsonValue)`) and step 6 (`.parse(text)` rejection
diagnostic) — is natural: step 6 will want to print a useful
diagnostic when users pass text, and that diagnostic can reuse the
line:column + context-snippet helper.

### Schema-side reuse (P54 step 5)

`Type::parse(JsonValue)` generates its own deviations (missing
required field, type mismatch at a leaf, wrong root kind).  These
reuse the same path + line:column + snippet infrastructure:

```
User.parse error at /address/zip (byte 2047, line 48 col 20):
  expected integer, got JString "10012"
```

Implementation: schema codegen passes its current path (struct
field name or `/N` for vector elements) into the same formatter
used by the parser.  No second diagnostic system.

### What this design is not

- Not a JSON Schema validator — the diagnostic reports *where* the
  parser or schema-walker gave up, not *what a user's business
  rules* expected.
- Not a recovering parser — first parser error still stops.  A
  recovering mode is a follow-up with its own design trade-offs.

---

## Active design — Q2 (free-form object iteration + kind peek)

**Bite.** A user holding a `JsonValue` of unknown shape has no way
to list an object's keys or iterate its fields.  `JObject {
fields_id }` exposes an arena index, not something loopable.
Without this, "free-form" reduces to "guess candidate key names
and try `field()` on each" — which isn't free-form at all.

`match`'s seven-arm dispatch also isn't great for a one-line
"what kind did I get?" peek in logs or conditional branches.

### Surface

```loft
/// Returns the variant name as text: "JNull", "JBool",
/// "JNumber", "JString", "JArray", "JObject".  Cheap — reads the
/// discriminant byte, formats a literal.
pub fn kind(self: JsonValue) -> text;            // ★ LANDED 2026-04-14

/// JObject: returns the vector of declared field names in
/// insertion order.  Any other variant: empty vector.
pub fn keys(self: JsonValue) -> vector<text>;

/// JObject: returns the vector of (name, value) entries so a
/// user can `for entry in fields(v) { … entry.name … entry.value … }`.
/// Any other variant: empty vector.
pub fn fields(self: JsonValue) -> vector<JsonField>;

/// JObject: true if the key is present (even if its value is JNull).
/// Distinguishes "absent" from "present-but-null".
pub fn has_field(self: JsonValue, name: text) -> boolean;
```

`JsonField` already exists in the stdlib for schema-internal use;
this promotes it to the public surface.

### Implementation

- `n_kind` — **LANDED 2026-04-14 in `src/native.rs`**.  Reads the
  discriminant byte at offset 0, returns one of six variant
  names via `stores.scratch` + `Str::new`.  Unknown bytes map
  to `"JUnknown"` defensively.  Registered as both free (`n_kind`)
  and method alias (`t_9JsonValue_kind`).  Guard tests in
  `tests/issues.rs`: `q2_kind_of_jnull_free_form` and
  `q2_kind_of_jnull_method_form` (dispatch), plus one per
  primitive variant (`jbool`, `jnumber`, `jstring`), and
  `q2_kind_of_parsed_primitive` locking the discriminant agreement
  between `n_json_parse` and `n_kind`.

  **B7 note:** this is the first Q2 method that dispatches on a
  `JsonValue` local — shipping it exercised the method-call
  surface that B7 was originally supposed to block.  The method
  form works ok today (`v.kind()`) in both debug and release,
  suggesting that some combination of the B2-runtime retrofit,
  the B5 layer-1/2 fixes, and the `t_9JsonValue_*` method-alias
  registration for the older `n_as_*` / `n_field` / `n_item` /
  `n_len` natives has narrowed B7's actual scope to just the
  character-interpolation text-return path
  (`b7_character_interpolation_return_crashes`, still `#[ignore]`).
  See § Compiler blockers — B7 for the narrowed symptom.

- **`n_keys` — JObject walk LANDED 2026-04-14.**  Returns an
  empty `vector<text>` for non-JObject variants; for JObject,
  walks the fields vector and copies each name into the result
  vector store.  Establishes the vector-from-native pattern for
  text elements: `database(text_size)` claims the handle store
  with the right per-element size; `vector_append` claims the
  inner vector record on first call; `set_str` allocates a
  string sub-record for each name and the new record-nr is
  written into the slot.  Insertion order preserved (linear
  walk).  Registered as both `n_keys` (free) and
  `t_9JsonValue_keys` (method alias).  Regression guards:
  `q2_keys_on_jnull_is_empty`, `…jbool…`,
  `q2_keys_on_jobject_returns_field_names_length`,
  `q2_keys_on_jobject_returns_multiple_field_names_length`,
  `q2_keys_on_jobject_preserves_first_name`,
  `q2_keys_on_jobject_collects_all_names`,
  `q2_keys_for_loop_is_safe`.
- **`n_fields` — JObject walk LANDED 2026-04-14 (full deep-copy
  2026-04-14 PM).**  Mirrors `n_keys`'s walk pattern; each result
  element is a `JsonField` struct.  Names copy verbatim.
  **All value kinds deep-copy** via a shared
  `dbref_to_parsed(stores, src) -> crate::json::Parsed` helper
  that walks the source arena recursively, plus the existing
  `materialise_primitive_into` writer on the result side —
  primitives (JNull / JBool / JNumber / JString) and containers
  (JArray / JObject with arbitrary nesting) all round-trip.
  Regression guards: `q2_fields_on_jnull_is_empty`,
  `q2_fields_on_jstring_is_empty`,
  `q2_fields_on_jobject_returns_field_entries_length`,
  `q2_fields_on_jobject_collects_multiple_entries`,
  `q2_fields_collects_all_names`,
  `q2_fields_preserves_primitive_number_values`,
  `q2_fields_preserves_container_values_array`,
  `q2_fields_preserves_container_values_object`,
  `q2_fields_for_loop_is_safe`.

  **Q2 cross-integration:**
  `q2_full_surface_smoke_on_jobject` exercises kind + has_field
  + keys + fields on the same JObject value and sums to 4 — every
  helper now returns its real JObject answer.

- **`n_has_field` — LANDED 2026-04-14 (stub 2026-04-14 AM,
  real impl 2026-04-14 PM with P54 step 4 third slice).**
  First shipped as a forward-compatible stub returning `false`
  unconditionally (JObject couldn't be constructed at that
  point).  After the step 4 third slice materialised primitive
  JObjects, rewritten to do a real linear scan: dispatches on
  JObject discriminant, walks the JsonField vector, compares
  each name to the query, returns true on first match.
  Primitive variants still return false through the short-
  circuit path.  Registered as both `n_has_field` (free) and
  `t_9JsonValue_has_field` (method alias).  Regression guards:
  - Primitives return false:
    `q2_has_field_on_jnull_is_false`, `…jbool…`, `…jnumber…`,
    `…jstring…`.
  - Dispatch paths:
    `q2_has_field_free_form_on_parsed_primitive`
    (free-dispatch + method-alias lock),
    `q2_has_field_gates_conditional_safely` (control-flow
    pattern).
  - JObject positive + negative (step 4 third slice):
    `p54_step4_nonempty_object_has_field_{hit,miss}`.

### Iteration example

```loft
v = json_parse(raw);
match v {
    JObject { fields_id } => {
        for entry in fields(v) {
            println("{entry.name}: {kind(entry.value)}");
        }
    }
    _ => println("not an object"),
}
```

### Tests (landed)

Coverage shipped under family-prefixed names rather than the
spec names originally proposed; the originals are kept here as
intent labels with a pointer to the actual test set:

- `kind` — `q2_kind_of_jnull_free_form`, `…_jnull_method_form`,
  `…_jbool`, `…_jnumber`, `…_jstring`, `…_parsed_primitive`
  (six assertions across the primitive variants).
- `keys` insertion order — `q2_keys_on_jobject_preserves_first_name`,
  `…_collects_all_names`.
- `fields` iteration — `q2_fields_on_jobject_collects_multiple_entries`,
  `q2_fields_collects_all_names`,
  `q2_fields_preserves_primitive_number_values`,
  `q2_fields_preserves_container_values_array/object`.
- `has_field` absent-vs-null — `q2_has_field_on_jnull/jbool/jnumber/jstring_is_false`,
  `q2_has_field_free_form_on_parsed_primitive`,
  `q2_has_field_gates_conditional_safely`.
- `kind` on intermediate `field()` results —
  `p54_step4_field_on_jstring_returns_jnull` exercises this
  via `v.field("missing").kind()`.
- Cross-surface: `q2_full_surface_smoke_on_jobject` sums to 4.

### Depends on

P54 step 4 (arena materialisation).  Landed immediately after.

---

## Active design — Q3 (`to_json` serialiser + struct serialisation)

**Bite.** The current surface is read-only.  Users who parse a
JSON response, modify a subtree, and want to forward it — or
users building a JSON reply from a loft struct — have no way to
emit JSON text.  Round-trip testing (parse → compare →
serialise → compare) is impossible.

### Surface

```loft
/// Serialise a JsonValue tree to canonical JSON text.
/// Object keys emitted in insertion order; no extraneous
/// whitespace; numbers formatted per RFC 8259.
pub fn to_json(self: JsonValue) -> text;          // ★ primitives LANDED 2026-04-14

/// Pretty-printed variant — 2-space indent, one element per line
/// for arrays/objects with >1 element.  Useful for logs and
/// golden-file tests.
pub fn to_json_pretty(self: JsonValue) -> text;

/// Struct serialisation — inverse of `T.parse(JsonValue)`.
/// Walks the struct's schema, builds a JObject, recurses into
/// nested struct / vector fields.  Fields with null sentinel
/// values serialise as JSON null (or are omitted under
/// `skip_null: true`).
pub fn to_json(self: T) -> text;                  // one per type; codegen-generated
pub fn to_json_pretty(self: T) -> text;
```

**Canonical + pretty — full tree 2026-04-14.**  Both
`to_json(self: JsonValue)` and `to_json_pretty(self: JsonValue)`
ship for all six variants.  Implementation: `src/native.rs`
factors the core rendering into a shared helper
`json_to_text_at(stores, v, pretty, depth)` — `pretty` controls
indent emission, `depth` tracks the recursion level.  Containers
recurse into each child slot; pretty mode emits `\n  …` at depth+1
for each element/field, dedents the closing bracket back to depth.
Empty containers stay `[]` / `{}` (no newline padding either way).
After object keys, pretty inserts a single space after the colon
(`"k": v`).  `n_to_json` and `n_to_json_pretty` are registered as
both free and method-alias forms.

The canonical path dispatches on the discriminant byte, writes
`"null"` / `"true"` / `"false"` for `JNull` / `JBool`, uses
Rust's `f64::Display` shortest-round-trip for `JNumber`, and
applies the canonical escape set (`"` / `\\` / `\n` / `\r` /
`\t` / `\b` / `\f`, plus `\uXXXX` for other control bytes) to
`JString`.  Non-finite numbers serialise as `null` (RFC 8259
constraint).

Regression guards in `tests/issues.rs` (13 total):
- `to_json` (canonical): `q3_to_json_of_jnull`,
  `q3_to_json_of_jbool_true/false`,
  `q3_to_json_of_jnumber_integer/fractional`,
  `q3_to_json_of_nan_becomes_null` (non-finite → `"null"`),
  `q3_to_json_of_jstring_plain` (`"hello"` round-trip).
- `to_json_pretty` (byte-identical to canonical for primitives):
  `q3_to_json_pretty_of_jnull/jbool/jnumber/jstring`,
  `q3_to_json_pretty_free_form` (free-fn dispatch + method-alias
  registration), and `q3_to_json_and_pretty_agree_on_primitive`
  — directly asserts `to_json(v) == to_json_pretty(v)` so a
  future divergence on primitives is caught at the call site.

**Container slice — LANDED 2026-04-14.**  The recursive walk
ships in `json_to_text_at`; the algorithm matches the original
plan (primitive dispatch recursed, escape logic shared between
JString values and JObject keys via a `write_json_string`
helper).  Six new pretty-mode regression guards lock the
indent layout: `q3_to_json_pretty_empty_array`,
`…_empty_object`, `…_array_indents_elements`,
`…_object_indents_fields`, `…_nested_array_in_object`,
`q3_to_json_and_pretty_differ_on_nonempty_container` (asserts
the active divergence so a regression that loses pretty's
indent gets caught).

**Deferred — escape-sequence regressions in `code!()` tests.**
Two additional guards for `"a\"b\\c"` and `"a\nb"` round-trips
were attempted but the first hung the test harness (loft
parser's interpretation of double-escaped strings fed through
Rust's `code!()` macro needs isolated investigation; the
Rust-side escape logic in `n_to_json` is exercised by unit
inspection).  Move escape-sequence repros to standalone
`.loft` files for debugging before re-adding the tests.

### Field-type matrix for struct → JSON

| Field type | Serialisation |
|---|---|
| `text` | `JString` |
| `integer` | `JNumber` (integral) |
| `float` | `JNumber`; `NaN` / `inf` → JSON `null` + diagnostic |
| `boolean` | `JBool` |
| `T` (nested struct) | `JObject` (recurse) |
| `vector<T>` | `JArray` (iterate) |
| `JsonValue` | serialised verbatim (round-trip the captured subtree) |
| null sentinel | `null` by default; configurable |

### Canonical form

- **No whitespace** outside strings (pretty-printed form adds it
  back).
- **Numbers** use shortest round-trip representation (same as
  `{f}` formatter).
- **Strings** escape `"`, `\\`, and control bytes `< 0x20`; UTF-8
  bytes pass through verbatim (no `\uXXXX` escaping of BMP
  characters — RFC 8259 allows both; shortest wins).
- **Object key order** — insertion order for `to_json(JsonValue)`,
  declaration order for `to_json(T)`.  Not sorted — stable
  insertion order is useful for diffing and avoids surprise
  reordering when programs read-modify-write.

### Implementation

- `src/json.rs` gains `pub fn format(v: &Parsed, pretty: bool) ->
  String` — recursive walk writing into a `String` buffer.
- `n_to_json` — reads a `JsonValue` DbRef, walks the arena into a
  `Parsed`-shaped temporary, formats.  Or format directly from
  the arena representation; same cost.
- `T.to_json()` codegen at the struct-method generation site —
  walks the schema, emits `n_build_json_field` calls per field
  into a work-buffer arena, then formats.  Mirror image of step 5.

### Round-trip property

`parse(to_json(v)) == v` for every `JsonValue`.  Property test
asserts this on a generated corpus (null, booleans, numbers
including 0.1-family, unicode strings, nested up to depth 5).

### Tests

- `q3_primitives_round_trip` — each primitive variant.
- `q3_nested_object_round_trip`.
- `q3_array_of_mixed_kinds_round_trip`.
- `q3_pretty_form_valid_json` — `parse(to_json_pretty(v)) == v`.
- `q3_unicode_string_escaping` — `"α β 😊"` round-trips without
  `\uXXXX` escaping.
- `q3_struct_to_json` — `User { name: "Bob", age: 30 }.to_json()`
  produces `{"name":"Bob","age":30}`.
- `q3_struct_with_nested` — recurses into `Address`.
- `q3_struct_with_jsonvalue_field` — raw subtree forwards
  verbatim.
- `q3_null_float_becomes_json_null`.

### Depends on

P54 step 4 for the `JsonValue` serialisation side.  `T.to_json()`
lands after step 5 (same codegen machinery in reverse).

---

## Active design — Q4 (JsonValue construction in loft code)

**Bite.** Today a loft program can read a `JsonValue` but cannot
build one.  Test fixtures ("given this JSON, when I call my
function…"), reply-construction in a web service, and forwarding
synthesised payloads are all impossible.

The obvious syntax — `v = JString { value: "hi" }` — trips
**B2-runtime** (unit-variant / struct-enum literal construction
at runtime crashes).  Waiting for B2-runtime blocks Q4 on
multi-session compiler surgery.

### Surface — helper constructors (bypass B2-runtime)

```loft
pub fn json_null() -> JsonValue;            // ★ LANDED 2026-04-14
pub fn json_bool(v: boolean) -> JsonValue;  // ★ LANDED 2026-04-14
pub fn json_number(v: float) -> JsonValue;  // ★ LANDED 2026-04-14
pub fn json_string(v: text) -> JsonValue;   // ★ LANDED 2026-04-14
pub fn json_array(items: vector<JsonValue>) -> JsonValue;   // blocked on step 4
pub fn json_object(fields: vector<JsonField>) -> JsonValue; // blocked on step 4
```

Plus a struct-literal shortcut for JsonField:

```loft
f = JsonField { name: "age", value: json_number(30.0) };
```

These are **native** functions that allocate arena records
directly — the same path `n_json_parse` uses internally.  They
sidestep B2-runtime because the variant is constructed in Rust,
not via loft's struct-enum literal syntax.

**Primitive slice — 2026-04-14 (four of six shipped).**
`json_null`, `json_bool`, `json_number`, and `json_string` all
landed.  `src/native.rs` grows four `n_json_*` fns, each using
the existing `jv_alloc` helper and the same
discriminant-byte + payload-field layout `n_json_parse` already
writes for parsed primitives.  Registered in `NATIVE_FNS`;
declarations added to `default/06_json.loft` under the
extractors.  `json_number` rejects non-finite inputs (NaN /
±Inf) by storing `JNull` + appending a diagnostic to
`json_errors()`, matching the RFC 8259 constraint.
`json_string` copies the text into the JsonValue's own store so
the returned value lifetime-extends its payload.

Regression guards (`tests/issues.rs`, 9 total):
- `q4_json_null_returns_jnull_variant`
- `q4_two_json_nulls_via_match_works`
- `q4_json_bool_round_trips_true`
- `q4_json_bool_round_trips_false`
- `q4_json_number_round_trips_finite`
- `q4_json_number_negative_finite`
- `q4_json_number_nan_becomes_jnull`
- `q4_json_string_round_trips`
- `q4_json_string_empty`

All guards use pattern-match destructuring for the variant
payload — not method calls — so they ride on the working path
guarded by `b7_multiple_json_parse_via_match_works`, avoiding
the still-open B7 method-surface bug.  The string tests
specifically measure `value.len()` inside the match arm rather
than returning the bound `value: text` (the text-escape path
trips the same native-returned-text lifecycle issue as
`b7_character_interpolation_return_crashes`).

**Container slice (empty input) — 2026-04-14.**
`json_array(items)` / `json_object(fields)` shipped with
empty-input support today.  Implementation: read the input
vector's DbRef from the stack, query its length via
`vector::length_vector`; if 0, build the empty-container
variant via the same path `json_parse("[]")` /
`json_parse("{{}}")` use.  For non-empty input, the
constructors deep-copy each element / field into the new
arena via a shared `dbref_to_parsed(stores, src) -> Parsed`
helper that walks the source JsonValue tree recursively,
and the existing `materialise_primitive_into` writer
materialises each Parsed sub-tree into the destination
root's store.  Nested containers round-trip
(`json_array([json_array([…])])`, objects inside arrays,
arrays inside objects).

* Sites: `src/native.rs::n_json_array`, `n_json_object` —
  each ~30 lines, mirror shape.  Registered as both free
  fns (`n_*`).  Method aliases not added because these are
  free constructors, not methods on a receiver.
* Shared helper: `dbref_to_parsed` (same file) walks a
  JsonValue DbRef tree and produces the transient
  `crate::json::Parsed` snapshot used by the existing
  writer.  Also used by `n_fields` to deep-copy container
  values while walking a JObject.
* Regression guards (`tests/issues.rs`, 13 total):
  `q4_json_array_empty_vector_returns_jarray`,
  `…_empty_has_zero_length`,
  `…_empty_serialises_as_brackets`,
  `q4_json_array_nonempty_input_returns_jarray`,
  `q4_json_array_multi_element_round_trips`,
  `q4_json_array_item_access_after_construction`,
  `q4_json_array_nested_construction`,
  `q4_json_object_empty_vector_returns_jobject`,
  `…_empty_has_zero_length`,
  `…_empty_serialises_as_braces`,
  `q4_json_object_single_field_round_trips`,
  `q4_json_object_multi_field_length`,
  `q4_json_object_serialisation`.

**Container slice (non-empty deep-copy) — LANDED
2026-04-14.**

### Builder ergonomics

For object-heavy construction, a vector-of-fields literal reads
cleanly:

```loft
reply = json_object([
    JsonField { name: "status", value: json_string("ok") },
    JsonField { name: "count",  value: json_number(42.0) },
    JsonField { name: "data",   value: forwarded_subtree },
]);
```

If usage patterns show this is too verbose, a second-round API
(`json_object_of([("status", "ok"), ("count", 42)])` with inferred
variants) can land; deferred until real call sites exist.

### Mutation — deferred

Mutating an existing tree (`v.set_field(name, value)`,
`v.push_item(item)`, `v.remove_field(name)`) is a natural
follow-up but **not in scope** for Q4.  Reason: arena indirection
+ the current `OpFreeRef` discipline make in-place mutation of a
tree's children expensive to reason about.  The construction
helpers above let users build a new tree from parts; replacing a
subtree in a parsed tree can be done by constructing the new
object and handing it to the consumer.

### Tests (landed)

Coverage shipped under family-prefixed names rather than the
spec names originally proposed:

- Primitive constructors — `q4_json_null_returns_jnull_variant`,
  `q4_json_bool_round_trips_true/false`,
  `q4_json_number_round_trips_finite`,
  `q4_json_number_negative_finite`, `…_nan_becomes_jnull`,
  `q4_json_string_round_trips`, `q4_json_string_empty`,
  `q4_two_json_nulls_via_match_works`.
- Array round-trip — `q4_json_array_empty_*`,
  `q4_json_array_nonempty_input_returns_jarray`,
  `q4_json_array_multi_element_round_trips`,
  `q4_json_array_item_access_after_construction`.
- Object round-trip — `q4_json_object_empty_*`,
  `q4_json_object_single_field_round_trips`,
  `q4_json_object_multi_field_length`,
  `q4_json_object_serialisation`.
- Nested construction — `q4_json_array_nested_construction`
  (array of arrays).
- Forward captured subtree — `q4_forward_captured_subtree_array`,
  `…_object`, `…_round_trip` (parse → embed in fresh JObject →
  serialise → re-parse — locks the deep-copy preserves arena-
  origin container values too).
- Pending: `q4_fixture_for_parse` (build tree → hand to
  `User.parse(v)`) — gated on P54 step 5 codegen.

### Depends on

P54 step 4 (arena machinery).  Q3's serialiser closes the
round-trip test surface but isn't strictly required — Q4's
constructors can land first.

### Why this belongs in P54 scope

Without Q4, P54 ships a one-way JSON pipeline.  Users can *read*
structured data but can't *write* it — so a loft web service
answering a request with JSON, a test that wants to mock a
response body, or any system that composes JSON from loft values
hits a wall.  "General-purpose JSON support" is the explicit P54
goal; Q4 is required for that, not an extra.

---

## Active design — P54-U (unified JSON parser)

**Bite.**  After P54 step 5 + 6 + Q1 schema-side landed, two JSON
parsers coexist in the codebase, and they accept slightly
different dialects:

- **`src/json.rs::parse`** — schema-free, two-pass, RFC 8259
  strict.  Produces a `Parsed` enum tree consumed by
  `n_json_parse` (P54 arena materialiser) and `n_struct_from_jsonvalue`
  (Q1-aware schema walker).  Rejects bare-key objects like
  `{val: 7}` (only `{"val": 7}` accepted).
- **`src/database/structures.rs::parsing`** — schema-driven,
  single-pass.  Walks JSON text and writes directly into struct
  records via the database's known-type schema.  Lives behind the
  `OpCastVectorFromText` opcode used by `vector<T>.parse(text)`,
  `text as Type` casts, and the fallback in `parse_type_parse`
  for non-text non-JsonValue arguments.  Accepts BOTH standard
  RFC 8259 JSON AND loft-native bare-key syntax (`{val: 7}`,
  `{name: "x"}`).  Production-tested for years.

The dialect drift is the user-visible symptom: the same loft
program parsing the same text via `User.parse(text)` (auto-wrap →
strict) versus `vector<User>.parse(text)` (legacy → lenient)
applies different acceptance rules.  The doc comment in
`tests/scripts/57-json.loft::test_json_parse_loft_native` already
notes the lenient form was renamed to use standard JSON when the
auto-wrap path was wired — but the legacy parser still accepts
either form transparently.

**Decision: one parser, two modes.**

A unified parser exposes a `dialect: Dialect` parameter
(`Dialect::Strict` / `Dialect::Lenient`).  Strict mode is RFC
8259 verbatim — bare keys rejected.  Lenient mode also accepts
loft-native unquoted identifier keys.  All other features (number
syntax, string escapes, structural punctuation, depth handling,
RFC 6901 path tracking, line:col tracking, context-snippet
diagnostics) are identical between modes.

**Critically: the current data-import path stays unchanged.**
The lenient-mode acceptance set is a strict superset of the
strict-mode set, AND a strict superset of what `structures.rs`
accepts today.  No `.loft` file or `.txt` data file that parses
today stops parsing under the unified parser — the lenient mode
is the new default for legacy entry points.

### Mode selection

| Entry point | Default mode | Rationale |
|---|---|---|
| `json_parse(text) -> JsonValue` | Strict | RFC 8259 spec match; the typed JsonValue surface is for new code |
| `Struct.parse(text)` (auto-wrapped via `json_parse`) | Strict | Inherits json_parse's mode |
| `Struct.parse(json_parse(text))` | Strict | Same as above |
| `vector<T>.parse(text)` | Lenient | Preserves the existing data-import path |
| `text as Type` / `text as vector<T>` cast | Lenient | Preserves existing semantics |
| `Struct.parse(text)` direct (non-auto-wrap fallback) | Lenient | Preserves existing semantics |

A user who wants strict JSON for a vector parse explicitly opts
in: `vector<T>.parse(json_parse(text))` (once `vector<T>.parse`
accepts JsonValue alongside text — a small extension once the
unified walker covers `vector<struct>` end-to-end, which it
already does in P54 step 5).

### Surface changes (`src/json.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// RFC 8259 strict — bare-key objects rejected.
    Strict,
    /// Strict + loft-native bare keys (`{val: 7}` ≡ `{"val": 7}`).
    Lenient,
}

impl Default for Dialect {
    fn default() -> Self { Dialect::Strict }
}

pub fn parse(input: &str) -> Result<Parsed, ParseError>;            // existing — Strict
pub fn parse_with(input: &str, dialect: Dialect) -> Result<Parsed, ParseError>;
```

The existing `parse(input)` keeps its signature (calls
`parse_with(input, Dialect::Strict)`) so all current callers stay
green.  New callers wanting lenient mode invoke `parse_with`.

### Bridging `OpCastVectorFromText` to the unified parser

The legacy `OpCastVectorFromText` body in `src/database/structures.rs`
gets reimplemented as:

```rust
pub fn parsing(stores: &mut Stores, text: &str, target_kt: u16) -> DbRef {
    // 1. Parse via unified parser, lenient by default to preserve
    //    legacy data-import compat.
    let parsed = match crate::json::parse_with(text, Dialect::Lenient) {
        Ok(p) => p,
        Err(e) => {
            // Legacy behaviour: zero-fill struct, push s#errors.
            stores.last_parse_errors.push(format_error(text, &e, 1, 1));
            return zero_struct(stores, target_kt);
        }
    };
    // 2. Walk the Parsed tree into the target struct/vector via a
    //    new helper that mirrors the JsonValue walker but consumes
    //    Parsed directly (no arena round-trip — Parsed lives only
    //    on the Rust stack).
    walk_parsed_into_target(stores, target_kt, &parsed)
}
```

The walker `walk_parsed_into_target` handles both struct and
vector targets (the latter for `vector<T>.parse(text)`'s wrapper-
struct shape).  It reuses the per-field-type dispatch matrix
already in `n_struct_from_jsonvalue` — extracted into a shared
helper that operates on either a `Parsed` ref or a JsonValue
DbRef.

After this, `src/database/structures.rs::parsing` shrinks from
~600 lines of hand-rolled scanner + dispatcher to ~50 lines of
parse-then-walk.

### Handling the dialect divergence carefully

A `.loft` test or data file parsed lenient today might also be
syntactically valid strict JSON (most are).  The migration
strategy:

1. **Add the Dialect enum + `parse_with` to `src/json.rs`** —
   pure addition, no behaviour change.  ✅ **Landed 2026-04-14**
   (`Dialect::Strict`, `Dialect::Lenient`, `parse_with(input,
   dialect)`; existing `parse(input)` is a shim over
   `parse_with(input, Dialect::Strict)`).
2. **Implement bare-key acceptance in `parse_object`** behind a
   dialect check.  Single conditional in the key-parsing
   branch.  ✅ **Landed 2026-04-14** (extracted
   `parse_object_key` helper; accepts `[A-Za-z_][A-Za-z0-9_]*`
   under `Dialect::Lenient`, rejects under `Dialect::Strict`).
3. **Reimplement `OpCastVectorFromText`'s `parsing`** to call
   `parse_with(text, Lenient)` + `walk_parsed_into_target`.
   ✅ **Landed 2026-04-14** — `Stores::walk_parsed_into` +
   `walk_parsed_struct` + `walk_primitive_into` in
   `src/database/structures.rs`.  Dispatches on every `Parts::*`
   variant (Base, Struct, EnumValue, Enum, Vector/Sorted/Array/
   Ordered/Hash/Spatial/Index, Byte, Short).  `Stores::parse`
   and `parse_message` route unified-first with legacy fallback
   gated for error-path position reporting.
4. **Verify** via the existing test scripts (`57-json.loft`,
   `58-constraints.loft`, `24-json.loft`) — every previously
   passing parse still passes.  ✅ **Verified 2026-04-14** —
   full `cargo test --release` pass (897/0 failed), plus
   instrumented `LOFT_P54U_TRACE` run showing zero success-path
   fallback hits across `issues` (437), `data_structures`
   (16), `wrap` (45), docs, and scripts.
5. **Delete** the now-unused scanner code in
   `src/database/structures.rs` (only the entry point and the
   Parsed-walker stay).  ✅ **Landed 2026-05-07** — the legacy
   hand-rolled scanner was already gone (Phase 2 walker
   superseded it organically).  Phase 3 work: cleaned up doc
   comments mentioning the "fallback" / "still kept for the
   transition", and added byte-offset threading through
   `walk_parsed_into` / `_struct` / `_primitive` so leaf
   type-mismatches report the field's real position via the
   `key_at` field already carried on `Parsed::Object` entries.
   The `"line N:M path:X"` shape via `format_walk_err` continues
   to back `tests/data_structures.rs::record` (`"line 1:7 path:blame"`).
   New regression: `p54u_leaf_mismatch_reports_field_position`
   in `tests/data_structures.rs` — locks the invariant that a
   primitive type mismatch inside a struct body reports a
   non-byte-0 position.

No public API changes.  No script-side migration required.  No
diagnostic regressions — both modes produce the rich Q1 errors
already shipped on the strict path.

### Implementation cost

- `src/json.rs`: ~30 lines (Dialect enum + parse_with + the
  bare-key conditional in parse_object).
- `src/database/structures.rs`: -540 lines (delete the hand-
  rolled scanner) + ~50 lines (parse-then-walk shim).
- New shared helper `walk_parsed_into_target`: ~120 lines
  (mirrors `n_struct_from_jsonvalue` but consumes `Parsed`).
- Tests: 3 new acceptance tests (bare-key accepted under
  Lenient, rejected under Strict; dialect-difference one-liner;
  legacy `text as Type` still works on a bare-key input).

### Why this belongs as a follow-up rather than a P54 sub-step

P54 already delivered the user-facing typed-JSON surface +
struct-from-JsonValue codegen.  The two-parser drift is an
internal cleanup — a user holding the typed `JsonValue` surface
can already parse, navigate, build, serialise, and unwrap into
structs.  The unification is about reducing maintenance surface
(one scanner instead of two, one dialect knob instead of two
divergent acceptance rules) and delivering Q1 diagnostics
uniformly across every text→JSON entry point.

### Unified diagnostic shape

Today three error sources produce three different formats:

| Source | Origin | Format example |
|---|---|---|
| Parser-side (`json.rs::format_error`) | Syntax error during `json_parse(text)` | `parse error at line N col M (byte B):\n  path: /a/b\n  <message>\n  <snippet with caret>` |
| Schema-side (`n_struct_from_jsonvalue` walker) | Type mismatch unwrapping JsonValue → struct | `User.age: expected JNumber, got JString` |
| Legacy (`s#errors` from `OpCastVectorFromText`) | Syntax or semantic error during `Type.parse(text)` | Free-form `format!()` strings — no consistent shape |

The unification step ships a single `Diagnostic` representation
that all three sources populate.  The text rendering degrades
gracefully when fields are missing — no diagnostic is worse for
the unification.

```rust
// src/json.rs (extends the existing ParseError into a richer
// shape that also carries schema-side info).
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// RFC 6901 pointer accumulated through parser descent +
    /// (for schema errors) struct-field path.  `""` = root.
    pub path: String,
    /// Human-readable message.
    pub message: String,
    /// Source location — present whenever the diagnostic can be
    /// traced to original text bytes (parser-side always; schema-
    /// side iff the JsonValue arena tracks per-element source
    /// offsets, see Phase 2 below).
    pub location: Option<SourceLocation>,
    /// Type-mismatch detail (Schema kind only).
    pub expected: Option<String>,
    pub actual: Option<String>,
}

pub enum DiagnosticKind {
    Syntax,    // parser couldn't read the input
    Schema,    // walker found a kind/shape mismatch
    Conversion, // numeric over/underflow during extraction
}

pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
    pub byte_offset: usize,
}

pub fn format_diagnostic(input: Option<&str>, d: &Diagnostic) -> String;
```

### Rendered forms

**Full info (parser-side syntax error):**
```
parse error at line 5 col 12 (byte 87):
  path: /users/3/age
  expected digit after `.`
    4 │       {"name": "Carol",
    5 │        "age": 1.}
                       ^
    6 │       },
```

**Schema mismatch with source location** (Phase 2, when arena
tracks byte_offset):
```
schema error at line 3 col 9 (byte 26):
  path: /users/1/age
  expected JNumber, got JString
    2 │   "users": [
    3 │     {"age": "twenty"}
                    ^
    4 │   ]
```

**Schema mismatch without source location** (Phase 1, when only
the JsonValue tree exists with no source-offset metadata):
```
schema error at /users/1/age:
  expected JNumber, got JString
```

**Cast (`text as Type`) failure** — same shape as parser-side:
```
parse error at line 1 col 8 (byte 7):
  path: /value
  unexpected character `,` after object key
    1 │ {value, 7}
             ^
```

### Single access surface

`json_errors()` returns the formatted diagnostic trail for ALL
sources (parser, schema, cast).  Each diagnostic renders into
the standard text shape above; the trail joins them with a
blank line separator (one blank line between blocks, no
trailing blank).

`s#errors` — the legacy per-record accessor kept for backward
compat — also resolves to the same trail, scoped to the record
constructed by the failing call.  No behavioural change for
existing callers; they get richer diagnostics for free.

### Path accumulation

The path is built incrementally through both parser and walker
phases:

- **Parser-side** (RFC 6901): pushed during `parse_object` /
  `parse_array` descent, popped on ascent.  Already shipped.
- **Schema-side** (struct walker): each recursion into a nested
  struct pushes `/<field_name>`; vector-element walks push
  `/<index>`.  When a diagnostic fires deep in the walker, the
  path captures the full descent.

Combined-path example: `Inbox.parse(text)` where `text` is
`{"users":[{"name":"A"},{"name":"B","age":"x"}]}` and User has
`age: integer`.  The schema diagnostic carries
`/users/1/age` — same RFC 6901 form parser-side errors use.

### Phase plan

**Phase 1 (with the parser-unification ship):** Introduce
`Diagnostic` + `format_diagnostic` + the trail accumulator.
Migrate all three sources to populate `Diagnostic` instead of
hand-rolling text strings.  Schema-side diagnostics initially
have `location: None` (no source-offset tracking yet) and render
as the "without source location" form.

**Phase 2 (follow-up):** Extend the JsonValue arena materialiser
to record the source byte offset for each element (one i32 slot
per record, ~12% memory overhead).  The walker reads these
offsets and populates `Diagnostic.location` so schema errors
also get line:col + context snippet.  Once shipped, every
diagnostic from every source has full location info.

### Why this design

- **No regression possible.**  The trail still gets populated
  the same way callers expect (json_errors trail + s#errors per-
  record).  The text gets richer.
- **Single source of truth for formatting.**  Adding a context-
  snippet style change happens in one function (format_diagnostic)
  — currently it would need to be duplicated across the three
  source paths.
- **Forward-compatible to structured access.**  A future Q-ticket
  could expose `JsonError` as a loft struct (`{ path: text, line:
  integer, column: integer, message: text, ... }`) so loft code
  can pattern-match on diagnostics rather than string-search.
  Same `Diagnostic` shape; just a new public surface.
- **Phase 1 is shippable independently** of arena-offset
  tracking.  The schema-side gets a consistent shape immediately;
  source location lands later when the arena tracks it.

### Tests for the diagnostic unification

- `p54_u_diagnostic_parser_format_unchanged` — the existing
  parser-side `q1_*` and `p54_err_*` tests still pass with
  the new `format_diagnostic` rendering the same text.
- `p54_u_diagnostic_schema_includes_path_and_kinds` — schema
  mismatch diagnostic includes the RFC 6901 path AND
  expected/actual variant names.
- `p54_u_diagnostic_cast_uses_same_shape_as_parse` — a `text as
  Type` failure renders identically to a `json_parse(text)`
  failure for the same input.
- `p54_u_diagnostic_trail_separator_format` — multiple errors
  render as separate blocks with a blank line between (not
  pipe-separated).
- `p54_u_diagnostic_path_combines_parser_and_walker_segments` —
  a schema error inside a deeply nested parsed structure shows
  the full `/<field>/<index>/<field>...` path.
- (Phase 2) `p54_u_diagnostic_schema_includes_source_location` —
  schema errors carry line:col + caret snippet once the arena
  tracks per-element byte offsets.

### Acceptance criteria

- `cargo test --release` — all suites green.
- `tests/scripts/57-json.loft::test_json_parse_loft_native` (the
  bare-key test that was renamed when auto-wrap landed) restored
  to bare-key form and passing under the Lenient default for
  vector parses.
- `src/database/structures.rs` line count down to ~250 lines
  (parse-then-walk shim + the existing struct/field-write
  helpers, which stay).
- `json_errors()` populates with the same RFC 6901 path + line:col
  + caret diagnostic for `text as Type` failures as for
  `json_parse(text)` failures.

### Tests

- `p54_u_lenient_accepts_bare_keys` — parser produces correct
  tree on `{val: 7}` under Lenient.
- `p54_u_strict_rejects_bare_keys` — parser returns ParseError
  on `{val: 7}` under Strict.
- `p54_u_text_as_type_still_accepts_bare_keys` — locks the
  data-import compat invariant.
- `p54_u_unified_diagnostic_for_cast` — `text as Type` failure
  produces the same Q1-format diagnostic as `json_parse` failure.

---

## Active design — Dep-inference for native fn returns (zero-leak unblock)

**Bite (2026-04-14).**  P54 ships a JsonValue surface
(`json_null`, `json_bool`, `json_number`, `json_string`,
`json_array`, `json_object` constructors plus `field`, `item`,
`kind`, `keys`, `fields`, `as_*` accessors).  Every chained
expression like `json_null().as_bool()` or
`v.field("x").kind()` leaks the temporary JsonValue store at
scope exit.  CI's debug-mode `execute_log_steps` assertion
(`Database X not correctly freed` at `src/state/debug.rs:994`)
catches it; release mode silently leaks per call.

Root cause: scope analysis's `inline_struct_return`
(`src/scopes.rs:~1026`) only lifts user-defined Reference returns
(`def.code != Value::Null`).  Native struct-enum returns
(`Type::Enum(_, true, dep)`) are never lifted — but the
constructors DO allocate fresh stores that need freeing.  The
existing system can't distinguish constructors (need lift) from
accessors (must NOT lift — they borrow into self's arena).

The discriminator is the `dep` field on the return type.  An
accessor borrows from `self` so its return should declare
`dep=[<self_attr_index>]`.  A constructor has no self so its
return should declare `dep=[]`.  Today both are declared
`dep=[]` because native function declarations never run through
`ref_return` (which only fires for fns with bodies).

**Status 2026-08-20 — shipped, and no longer observably load-bearing.**  The inference is
implemented (`src/parser/definitions.rs`, the `;`-terminated native-fn arm) and the bite
above is fixed: `json_null().kind()`, `v.field("a").item(1).kind()` and the rest neither
leak nor read freed memory, in loops, on `--interpret` and `--native`.

What could NOT be shown is that the inference is what fixes it.  Disabled (`if false &&`)
and rebuilt, every measurement is unchanged: the same values, the same store ledger
(1213 allocs / 1211 frees, peak 8 vs 9 live), clean under `LOFT_POISON=1` and
`LOFT_STORE_GUARD=1`, on both backends — and `loft_suite` still passes, so no existing
test covers its absence either.  The store-lifetime work that landed after this design was
written (@PLN85 and the `inline_struct_return` gates around it) appears to keep these
shapes in order on its own.

That is a measurement, not a verdict: "no shape I constructed distinguishes it" is weaker
than "it does nothing", and the probes were JsonValue-shaped because that is where the
surface is.  A native `self`-taking method on some OTHER struct-enum is the case most
likely to still need it.  Two things follow, and neither is a fix: the inference should not
be deleted on this evidence alone, and anyone who touches it should know the suite will not
tell them if they break it.

**Decision (2026-04-14): implicit dep inference for native fn returns.**

When a native function declaration `pub fn name(self: T, ...)
-> R;` is parsed and the return type R structurally matches the
self type T (same `Reference(d, _)` or `Enum(d, true, _)` with
the same `d`), automatically populate the return's `dep` with
`[<self_attr_index>]`.  No syntax change required; no per-fn
annotation; the parser infers borrowing from "returns the same
thing self is".

Cases handled correctly:

| Native | Self type | Return type | Inferred dep | Lifted? |
|---|---|---|---|---|
| `json_null()` | (none) | `JsonValue` | `[]` | YES (constructor, owned) |
| `json_string(text)` | (none) | `JsonValue` | `[]` | YES |
| `json_array(vec<JV>)` | (none) | `JsonValue` | `[]` | YES |
| `json_parse(text)` | (none) | `JsonValue` | `[]` | YES |
| `field(self: JV, text)` | `JsonValue` | `JsonValue` | `[0]` (= self) | NO (borrows) |
| `item(self: JV, integer)` | `JsonValue` | `JsonValue` | `[0]` | NO |
| `kind(self: JV)` | `JsonValue` | `text` | n/a (text) | n/a |
| `as_bool(self: JV)` | `JsonValue` | `boolean` | n/a (bool) | n/a |
| `Type.parse(text)` | (none) | `Type` | `[]` | YES |

The accessor-method tests added in P54 (`field()`, `item()`)
return JsonValue from a JsonValue self → infer dep=[0] → not
lifted → no use-after-free.  The constructor tests (`json_null`,
`json_bool`, etc.) return JsonValue with no self → dep=[] → lift
→ OpFreeRef fires at scope exit → no leak.

### Surface change (`src/parser/definitions.rs` or wherever
native fn parsing happens)

After parsing a native fn declaration with an empty body, before
storing the return type, check:

```rust
if let Type::Reference(ret_d, ref mut dep) | Type::Enum(ret_d, true, ref mut dep)
        = &mut def.returned
    && dep.is_empty()
{
    for (i, attr) in def.attributes.iter().enumerate() {
        if attr.name == "self" {
            let self_d = match &attr.typedef {
                Type::Reference(d, _) | Type::Enum(d, true, _) => Some(*d),
                _ => None,
            };
            if self_d == Some(*ret_d) {
                dep.push(i as u16);
            }
            break;
        }
    }
}
```

### Surface change (`src/scopes.rs::inline_struct_return`)

Once accessors carry a non-empty `dep` and constructors carry
`dep=[]`, extend the lift to native struct-enum constructors:

```rust
fn inline_struct_return(val: &Value, data: &Data, _outer_call: u32) -> Option<u32> {
    if let Value::Call(fn_nr, _) = val {
        let def = data.def(*fn_nr);
        // existing rule: user-defined struct return
        if def.name.starts_with("n_")
            && def.code != Value::Null
            && let Type::Reference(d_nr, _) = &def.returned
        {
            return Some(*d_nr);
        }
        // new rule: native struct-enum constructor (dep-empty)
        if (def.name.starts_with("n_") || def.name.starts_with("t_"))
            && let Type::Enum(d_nr, true, dep) = &def.returned
            && dep.is_empty()
        {
            return Some(*d_nr);
        }
    }
    None
}
```

### Tests to un-ignore once the dep-fix lands

All 34 entries in `tests/ignored_tests.baseline` tagged
`p54-leak: chained json call temp not freed (zero-leak gate)`
should pass once dep inference is correct AND the lift extends
to struct-enum constructors.  Iterate: regenerate baseline via
`python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline`,
run `cargo test --test issues p54_` in DEBUG mode, expect green.

### Acceptance criteria

- `cargo test --test issues p54_` — all p54 tests green in DEBUG
  build (no `Database X not correctly freed` panic).
- 34 ignore entries removed from `tests/ignored_tests.baseline`
  and from `tests/issues.rs` `#[ignore]` attributes.
- `tests/wrap.rs::loft_suite` — leak warnings on scripts
  `42-file-result.loft`, `62-index-range-queries.loft`,
  `76-struct-vector-return.loft` either disappear or are
  separately diagnosed (not all of them are this same root
  cause).
- 0.8.4 tag attempt resumes (per RELEASE.md § Safety gate
  deferral).

### Implementation cost

- ~30 lines in `src/parser/definitions.rs` for the inference.
- ~10 lines in `src/scopes.rs::inline_struct_return` to lift
  the new constructor case.
- One regression test in `tests/issues.rs` that asserts the
  inferred dep on `field()` and json_null() (read via
  `Data::def(...)`).
- ~5 deletions from `tests/ignored_tests.baseline` per
  unignored test (× 34).
- One CHANGELOG entry under `[Unreleased]`.

### Why this belongs here

This is the unblock for the 0.8.4 tag.  Without it, the P54
JsonValue surface ships with a real production leak — every
short-lived JsonValue (constructor or arena lookup) leaks
unbounded in any program that exercises the API.  RELEASE.md §
Safety gate explicitly blocks the tag on this.  Task #46
tracks the implementation.

---

## Compiler blockers — struct-enum bugs

**AUDIT 2026-05-21 — all B1-B7 closed on the interpreter.**  Ran the
full B-family regression set (`cargo test --release --test issues
p54_b` + `b7_`): **18 tests pass, 0 ignored**.  The `#[ignore]`
markers the historical notes below reference are gone; the biting
reproducers each verified live:

- **B2-runtime** (`s = Idle;` bare unit-variant in a mixed enum) —
  `p54_b2_unit_variant_literal_construction` + the qualified form pass
  on both backends.  The "layer 3 interpreter codegen NOT landed" note
  below is **stale** — it closed via the cross-PR struct-enum
  return-slot work.
- **B3** (struct-enum tail-expression / intermediate-local return) —
  `p54_b3_float_via_intermediate` etc. pass on the interpreter.  The
  "4-layer surgery, open" design below is **stale for the interpreter**.
- **B1/B5/B6/B7** — confirmed FIXED (as the notes already record).

**Native-only residual @P301 — FIXED 2026-05-21.**  The B3
intermediate-local form (`fn mk() -> JV { n = A{..}; n }`, and the
explicit `return n;` form) used to fail *native* compilation — the call
site emitted `n_mk(cell, ())` for a callee whose local was hoisted into
a hidden `DbRef` return-slot param (E0308).  The `p54_b3_*` guards are
interpreter-only, so this went unnoticed.  Root cause was NOT the call
site but `add_defaults` (`src/parser/mod.rs`): it had work-ref-allocating
arms for `Type::Vector`/`Type::Reference` hidden params but none for
`Type::Enum(_, true, _)`, so the hidden struct-enum arg stayed
`Value::Null` → `()`.  Fixed by adding the mirror `Type::Enum(_, true, _)`
arm; the call-site emitter then threads the work-ref automatically.  No
interpreter regression.  Cross-mode guard:
`tests/scripts/121-struct-enum-return-local.loft`.

The historical fix-design notes below are preserved as the narrowing
audit trail, not as current-state claims.

---

**Status (2026-04-13):** Concrete fix designs documented for all
four open compiler bugs (B2-runtime, B3, B5, B7) following an
explore-agent investigation.

The B7 single-line-fix prediction was tested and **did NOT close
the bug** — the type-match extension at `src/scopes.rs:1031` is
necessary but not sufficient; at least one other site in the
lifecycle machinery is also wrong.  Revised B7 estimate: **2-3
sessions** with `LOFT_LOG=full` instrumentation to pinpoint the
duplicate OpFreeRef emission.  Design + candidate sites listed in
§ B7 below.

The B5 / B2-runtime / B3 fix designs remain untested but
file:line targets are concrete.  Recommended landing order
restored to "B7 first, then B5, then …" because B7 still has the
largest blast radius even at the higher cost.

These bugs each surface any time a user writes a `Result<T, E>`-style
struct-enum, not just for JSON.  Fixing them unblocks the whole
`Option<T>` / `Result<T, E>` / planned coroutine-yield surfaces.

**B1 — Unit-variant match index-OOB.**  **FIXED** commit `61c36d7`.
Regression: `p54_b1_unit_variant_match_from_binding`.

**B2-runtime — Unit-variant literal construction in struct-enum
crashes.**  `JsonValue.JNull { is_null: true }` constructed at
runtime in a mixed enum doesn't produce a matchable value.
Workaround: build via the constructor path the parser uses; user
code avoids unit variants.  Test: `p54_b2_runtime_*` (`#[ignore]`).

**Fix design (original — stale; see revised note below).**
Unit variants in **mixed** enums (where some variants have fields
and some are unit) leave the payload buffer uninitialised when
constructed at runtime.  The variant tag byte is set correctly,
but the residual bytes beyond the tag carry whatever was on the
stack — match dispatch then reads garbage and either fails to
match or matches the wrong arm.

**Re-diagnosis 2026-04-13** (via `LOFT_LOG=full` on the same
`Sig { Off, Idle, On { level } }` reproducer): the observed
runtime symptom is **not** the predicted garbage-tag mismatch.
Instead, the test loops returning `value=16` thousands of times
until the harness's "Too many operations" guard fires at
`src/state/debug.rs:974`.  The match expression seems to
re-enter the function rather than exit it, which suggests a
codegen issue at the match-dispatch / return-slot layer, not
(only) the parse-time zero-fill.  Before attempting surgery,
capture a narrower trace with `LOFT_LOG=fn:run` and read
`parse_enum_field` → `parse_object` → match-arm return path
together.  Zero-filling the payload is likely necessary but
not sufficient.

**Partial fix landed 2026-04-13.**  Two root causes identified:

1. **Type-layout (LANDED):** `parse_enum_values` only added the
   "enum" discriminant attribute to struct variants (those with
   braces), leaving sibling unit variants with 0 attributes.
   `fill_database` then produced size-0 structures for them, and
   `Store::claim(size=0)` panicked "Incomplete record".  Fix in
   `src/typedef.rs::fill_all`: retroactively add the "enum" field
   to every unit variant whose parent's `returned` is a mixed
   `Type::Enum(_, true, _)`.  Off/Idle/On now all have the
   discriminant field in the native-schema emit.
2. **Bare-identifier construction (LANDED 2026-04-13):** extended
   `parse_constant_value` at `src/parser/objects.rs:481` to emit an
   inline `v_block` when the resolved variant's parent enum is
   mixed.  The block allocates a work-ref DbRef, calls
   `object_init` (which writes the discriminant via the "enum"
   field's default value), and returns the work-ref.  Work-ref is
   marked `skip_free` so only the receiving slot (var_s) frees the
   store at scope exit.  Native-emit verified: `let var_s: DbRef =
   { OpDatabase(__ref_1, 61); set_byte(0)=2; __ref_1 };` with
   single `OpFreeRef(var_s)`.

3. **Interpreter codegen (NOT landed):** `state::execute` still
   panics `Incomplete record` on the same reproducer, meaning the
   bytecode generation in `src/state/codegen.rs` doesn't observe
   the new `v_block` form the same way native-emit does.  Layer 1+2
   pass at the IR + native-Rust output level; the interpreter's
   bytecode emitter needs paired handling for
   `Type::Enum(_, true, _)` destination slots receiving a
   v_block containing an `OpDatabase` + field-init sequence.
   Follow `gen_set_first_ref_*` sites for the struct-Reference
   path and mirror for struct-enums.  Est. 1 session.

**Site:** `src/parser/objects.rs::parse_enum_field` (lines 1286-1314)
constructs the variant struct via `parse_object(e_nr, &mut cd)`.
For unit variants (0 attribute fields), the underlying
`OpDatabase` allocates a record but no field-init writes follow,
so the payload bytes stay garbage.

**Fix:** in `parse_enum_field`, detect the unit-variant case
(`def.attributes.is_empty()` for the variant struct) and emit a
zero-fill of the payload region after the `OpDatabase` /
`OpSetEnum` calls but before returning the value.  The payload
region size is `size(parent_enum) - 1` (everything after the tag
byte).  Reuse the existing bulk zero-fill op in `src/fill.rs` (or
add a 5-line `op_zero_bytes` handler if no exact op exists).

**Files:** `src/parser/objects.rs:1286-1314`; possibly a new op in
`default/01_code.loft` and `src/fill.rs`.
**Estimated scope:** one session.

**Verification path:**
1. `cargo test --release --test issues p54_b2_runtime_*` —
   2 currently-`#[ignore]`'d tests flip to green
   (`p54_b2_runtime_unit_variant_construction`,
   `p54_b2_runtime_qualified_unit_variant_in_mixed_enum`).
2. Full suite green.
3. Smoke: `JsonValue.JNull { is_null: true }` constructed at
   runtime in user code matches correctly via
   `match v { JNull { is_null } => ... }`.

**Side-effect risk:** low.  The fix narrows behaviour
(garbage-payload → zero-payload), making previously-undefined
match results well-defined.  Programs that accidentally relied on
the garbage value were already broken.

**B3 — Struct-enum tail-expression return crashes.**  Five
investigation sessions narrowed the diagnosis: needs **at least
4 coordinated codegen layers** changed (caller-side hidden-slot
allocation, `scopes.rs:307-318` hoist, `OpCopyRecord` deep-copy paths,
OpReturn discard accounting).  Single or even 3-layer attempts mutate
the symptom but never close it.  Workaround: explicit `return n;`
instead of `n` at function tail.  Tests: `p54_b3_*` (`#[ignore]`).
Estimated 8-12 source-line ranges across 2 files when attempted as
one focused refactor.

**Re-diagnosis 2026-04-13** (via `LOFT_LOG=crash_tail:30` on
`p54_b3_float_via_intermediate`).  The observed failure is not a
deep-copy / free-collision; it is `n_mk` **calling itself
infinitely** from its own tail position.  The tail expression `n`
(a local of struct-enum type) compiles to an `OpCall(fn=n_mk, …)`
each time — a fresh store is allocated (`ConvRefFromNull` →
`Database`) and the body re-executes before any return.  The heap
grows by one store per iteration until `free(): invalid next
size` aborts.

This sharpens the original 4-layer design: layer 1
(caller-side hidden-slot pre-alloc when the callee returns
`Type::Enum(_, true, _)`) is the site that currently mis-routes
the tail `n` load as a recursive call.  Without a reserved
return-slot the codegen falls back to the "call expression" path
for the tail local, and the return slot never materialises.
Landing layer 1 first, rerunning the trace, and only then adding
layers 2-4 is now the recommended order (instead of landing all
four together as the original design required).

**Fix design (original 4-layer, still applicable).**
Four coordinated layers must change.  Concrete file:line targets:

| Layer | File | Line(s) | Change |
|---|---|---|---|
| 1. Caller pre-alloc | `src/state/codegen.rs::generate_call` | 1410-1420 (before OpCall emission) | When callee's return type is `Type::Enum(_, true, _)`, emit `OpDatabase` for a 12-byte return slot, mirroring the Reference path |
| 2. Hoist | `src/scopes.rs` | 311 | Extend the hoist-set match from `Type::Reference \| Type::Vector` to also include `Type::Enum(_, true, _)` |
| 3. Deep-copy | `src/state/codegen.rs` | 827, 954-960, 975-1022, 1080, 1101, 1112-1130 | Every `Type::Reference` arm in OpCopyRecord-related match sites grows an `\| Type::Enum(_, true, _)` sibling |
| 4. Type extract | `src/state/codegen.rs::known_type` | 1761-1763 | Match arm currently extracts `Type::Reference(c, _) → c`; extend to `Type::Reference(c, _) \| Type::Enum(c, true, _)` |

**Estimated scope:** 2-3 sessions.  Each layer is independent and
testable; if a session lands only layers 1-2, the symptom mutates
but doesn't close — five investigation sessions confirmed all four
must land together.

**Verification path:**
1. After all 4 layers land: `cargo test --release --test issues p54_b3_*`
   — 4 currently-`#[ignore]`'d tests flip to green
   (`p54_struct_enum_explicit_return_of_local` already passes via
   the `return n;` workaround; the implicit tail-expression form is
   what the fix covers).
2. Full suite green.
3. Manual smoke: write the original BITING_PLAN reproducer
   (`fn mk() -> JV { A { v: 42 } }`) and confirm no crash.

**Side-effect risk:** medium.  OpCopyRecord deep-copy paths are
load-bearing for vector/struct passing; extending each match arm
needs a matching test for the new Enum case to avoid regressing
the existing Reference path.

**Why B3 sits *after* B7 in the recommended order:** they're
independent codegen surgeries with no overlap, and B7 unblocks
5x more downstream work per line of code touched.  B3 closes an
ergonomics gap; the `return n;` workaround stays good for any
user who needs it.

**B5 — Recursive struct-enum runtime crash.**  **FIXED.**  All four
guards (`p54_b5_recursive_struct_enum`,
`p54_b5_recursive_struct_enum_construction`,
`p54_b5_not_taken_arm_with_vector_binding_ok`,
`p54_b5_for_loop_over_enum_variant_vector`) now pass without
`#[ignore]`.  The recursive `count(Node {...})` returns 7 as
expected.  Layer 3 (the recursive tail-call return-PC bug
described historically below) closed as a side-effect of the
struct-enum return-slot work that landed across PR #168 → #174 —
no dedicated commit needed for layer 3 itself.

**Historical layered diagnosis kept for context.**  The reference
loft source:

```loft
pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn count(t: const Tree) -> integer {
    match t {
        Leaf { v } => v,
        Node { kids } => { c = 0; for k in kids { c += count(k); }; c }
    }
}
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    count(root)
}
```

**Layer 1 — type registration (LANDED 2026-04-14).**  `fill_all`
now walks every struct and enum-variant attribute for
`Type::Vector(T)` fields and calls `data.vector_def(lexer, &T)`
before the main `fill_database` loop.  The wrapper struct
`main_vector<Tree>` is then registered and `fill_database` assigns
it a real `known_type`.  Parser-path assignment sites already
called `vector_def`; this covers the struct-enum-variant
declaration site that nothing else hit.  Closes the original
"Incomplete record" panic on `OpDatabase(db_tp=u16::MAX)`.

* Site: `src/typedef.rs::fill_all` (the pre-loop scan before
  line 215).
* Positive guard: `p54_b5_recursive_struct_enum_construction` in
  `tests/issues.rs`.

**Layer 2 — match-arm binding lifetime (LANDED 2026-04-14).**
`src/parser/control.rs:1103` `create_unique("mv_<field>", &field_type)`
now calls `self.vars.set_skip_free(v_nr)` on the binding variable.
The binding is a borrowed view (a `DbRef` field extraction from
the subject's record) — it does not own a store.  Without
`skip_free`, scope cleanup emitted `OpFreeRef(mv_…)` at function
exit.  In the taken arm, that decrements a store the binding
doesn't own.  In the **not-taken** arm, that slot was never
assigned and the free reads garbage bytes as a DbRef — observed
as out-of-bounds `store_nr ≈ 4621` in `Stores::free_named`.
Closes the garbage-FreeRef crash.

* Site: `src/parser/control.rs:1103-1125`.
* Positive guards: `p54_b5_not_taken_arm_with_vector_binding_ok`,
  `p54_b5_for_loop_over_enum_variant_vector` in `tests/issues.rs`.

**Layer 3 — recursive tail-call return PC (OPEN).**  After layers
1 and 2 land, the still-ignored test `p54_b5_recursive_struct_enum`
now gets FURTHER through execution before crashing.  The full
construction + match + for-loop path runs correctly until the
inner recursive `count(k)` call returns.  At that point the
trace shows:

```
4506:[160] GotoWord(jump=4643)                 ← jump to match end of inner call
4643:[160] Return(ret=9[128], value=4, discard=44) -> 3[116]  ← inner Return
   9:[120] Goto(jump=32)                       ← PC=9 is wrong; wanders away
  31:[120] CastIntFromText(v1=<raw:0x0>[104])  ← reads null text, wanders further
```

The inner Return pops `ret=9` from the stack as the return PC,
but PC=9 is nowhere near the caller's `c += count(k)` site — it
lands in unrelated bytecode (`OpCastIntFromText` on a null text),
then wanders into random ops.  The return-PC slot was read from
the wrong address.

**Candidate root cause.**  `src/state/codegen.rs::add_return`
around line 1772-1774 emits OpReturn with `self.code_add(self.arguments)`
— a per-function argument-frame size captured on `State`.  The
observed `ret=9` doesn't match n_count's actual argument frame
(1 × `const Tree` = DbRef = 12 bytes).  Either:

1. **`self.arguments` is stale** at the emit site — it's a `State`
   field reset per-function in `def_code` (`src/state/codegen.rs:57-79`)
   but not captured into the `Stack` context, so if something
   mutates it between function start and `add_return`, the value
   is wrong.  Mitigation: capture into `Stack` at function entry;
   use captured value in `add_return`.
2. **Ret-field semantics don't match "arg size"** — it may be the
   return-slot offset from the frame base.  Compute `ret_slot`
   explicitly at emit time rather than piggy-backing on
   `self.arguments`.
3. **Runtime reader mis-reads** — if emission is correct,
   `src/state/mod.rs:476-495` (`fn_return`) reads PC from the
   wrong stack offset.  The fix lives there.

**Fix path.**  Instrumentation-first: add a debug `eprintln!` in
`add_return` logging `(fn_name, self.arguments, size_of_return,
stack.position)`; correlate with the runtime trace's OpReturn
fields to disambiguate the three candidates before editing.

**Files:** `src/state/codegen.rs:1759-1778` (`add_return`),
`src/state/codegen.rs:57-79` (`def_code` prologue),
`src/state/mod.rs:268` (fn_call PC push),
`src/state/mod.rs:476-495` (`fn_return` PC pop).
**Estimated scope:** 1-2 sessions once the instrumentation
disambiguates which candidate is the actual root cause.

**Verification path:**
1. Instrumentation trace agrees with ONE of the three candidates.
2. The emitted OpReturn's `ret` field matches `sizeof(DbRef) = 12`
   for `const Tree`.
3. `p54_b5_recursive_struct_enum` un-ignored; output = 7.
4. The three positive guards (`..._construction`,
   `..._not_taken_arm_with_vector_binding_ok`,
   `..._for_loop_over_enum_variant_vector`) remain green.

**Related symptom.**  Layer 3's trace matches B3-family
(struct-enum tail-expression return).  B3 itself shipped 2026-04-13
as "struct-enum return types now get hidden caller pre-alloc args
just like Reference/Vector"; layer 3 may require pairing the call-
site pre-alloc with a recursion-aware return-slot accounting fix.

**B6 — Match-arm type unification.**  **FIXED** commit `5684df2`.
Regression: `p54_b6_match_arm_value_text_unifies`.

**B7 — Native-returned temporary lifecycle.**  **FIXED.**  All five
B7 regression guards pass without `#[ignore]`:

- `b7_method_on_jsonvalue_returning_integer_works` — `len(v)` on
  `JsonValue` (the original method-dispatch case).
- `b7_method_on_q4_constructed_jsonvalue_works` — same shape with
  the JsonValue built via `json_*` constructors.
- `b7_repeated_method_dispatch_on_jsonvalue_works` — chained
  method calls.
- `b7_multiple_json_parse_via_match_works` — sequential
  `json_parse` calls consumed via pattern match.
- `b7_character_interpolation_return_crashes` — name kept for
  search-back compatibility but the test passes (`build_b7c() == "h"`).

Closed as a side-effect of the B2-runtime retrofit, B5 layers 1+2,
the `t_9JsonValue_*` method-alias registrations, the dep-inference
fix in PR #171, and the lock-args-around-OpCopyRecord work in
PR #172 — no dedicated B7 commit was needed.

The historical paragraphs below describe the bug's reach at the
time they were written — preserved as the narrowing audit trail,
not as current-state claims.

**Unification finding 2026-04-13.**  B7's signature — `~500 iterations of
Return(ret=0, value=16, discard=0) at PC=0` followed by legitimate code
resuming, followed by store-leak warning + double-free — **matches B2-runtime
and B3 trace-for-trace**.  All three fire `OpReturn` / `OpCall` in a loop at a
function boundary involving a struct-enum value.  Specifically:

* B2-runtime: `s = Idle; match s { ... }` — OpReturn loops after the match.
* B3: `fn mk() -> JV { n = A{..}; n }` — OpCall loops (function calls itself).
* B7: `len(v_b7m)` where `v_b7m: JsonValue` — OpReturn loops after len.

The common thread: the **caller's reserved return slot** is wrong-sized or
wrong-addressed for a `Type::Enum(_, true, _)` value.  OpReturn pops `value=16`
bytes (the DbRef size) but the stack pointer advances because the caller
reserved the slot incorrectly; eventually the stack unwinds to a non-zero PC
and normal code resumes.  A **single fix** to the return-slot reservation /
OpReturn accounting for struct-enums likely closes all three items together.

Scope analysis (`src/scopes.rs`) doesn't emit `OpFreeRef` correctly
for the JsonValue store returned by `json_parse`.  The store leaks
on a chain of method calls AND any subsequent method-call site
trips a double-free at exit even when the method does no
allocation of its own.  Confirmed symptoms:

- `n_json_parse` returning a string variant + `as_text()` →
  caller's text-return path frees the JsonValue store before the
  text copy completes (`free(): invalid next size` at exit).
- Chained JSON access (`v.field("a").item(0).field("b")`) leaks
  intermediate stores.
- `fn f() -> text { c = txt[0]; "{c}" }` SIGSEGVs (discovered
  while writing INC#9 regression tests) — same family:
  native-returned text temporary built via `n_format_text` on a
  character isn't tracked for free on the outer function's
  return path.
- **(new, found 2026-04-13)** ANY method call on a JsonValue
  local crashes — even a method that just reads the discriminant
  byte and returns an integer (`len(v)`).  The crash is exit-time
  double-free, but the test harness sees it as SIGSEGV before
  reporting the function's return value.  Discovered while
  attempting to ship Q2's `kind(v) -> integer` peek; reverted
  the ship and parked the regression guard at
  `b7_method_on_jsonvalue_returning_integer_crashes` (`#[ignore]`).
- **(new symptom — INC#9 caveat)** `fn f() -> text { c = txt[0]; "{c}" }`
  SIGSEGVs.  The text built via `n_format_text` on a character
  isn't tracked for free on the outer function's text-return
  path.  Regression guard: `b7_character_interpolation_return_crashes`
  (`#[ignore]`'d).

**Retraction** (2026-04-13): an earlier note claimed "a second
`json_parse` call in the same function corrupts memory."
Investigation while writing B7 regression tests showed that
multiple `json_parse` calls work fine when each result is
consumed via pattern matching — the corruption observed in
earlier smoke tests came from the subsequent `kind()` / `len()`
method calls, not from `json_parse` itself.  Guard for the
working multi-parse path: `b7_multiple_json_parse_via_match_works`.

**Blast radius**: the entire `(JsonValue) -> T` method surface is
gated on this fix, not just text returns.  This means **Q2**
(`kind`, `keys`, `fields`, `has_field`), **Q3** (`to_json`,
`to_json_pretty`), the planned step-4 implementations of
`field`/`item`/`len`, and parts of step 5 (`Type::parse(JsonValue)`)
all sit downstream.

**Fix design (added 2026-04-13 from explore-agent investigation).**
The bug is in `src/scopes.rs::inline_struct_return` at line **1031**:

```rust
if let Value::Call(fn_nr, _) = val {
    let def = data.def(*fn_nr);
    if def.name.starts_with("n_")
        && def.code != Value::Null
        && let Type::Reference(d_nr, _) = &def.returned   // ← only Reference
    {
        return Some(*d_nr);
    }
}
None
```

The Set path at `scopes.rs:447-449` and `needs_pre_init` at line
1043 already accept `Type::Enum(_, true, _)`.  Only this lifting
site was missed — so native calls returning struct-enum (e.g.
`json_parse(...) -> JsonValue`) bypass lifting, the JsonValue
store is embedded in the argument frame, and the callee's exit
frees the store before the caller's `OpFreeRef` would have fired.

**Single-line fix (proposed by the design):**

```rust
&& let Type::Reference(d_nr, _) | Type::Enum(d_nr, true, _) = &def.returned
```

**Update (2026-04-13, after attempted ship):** the single-line fix
was applied and the test `b7_method_on_jsonvalue_returning_integer_*`
*still crashed* with the same "stores not freed" + "double free or
corruption" pattern, in both the inline form
(`len(json_parse(...))`) and the assigned form
(`v = json_parse(...); len(v)`).  The fix was reverted.

The type-match was demonstrably incomplete (the Set path and
`needs_pre_init` already accept `Type::Enum(_, true, _)` —
`inline_struct_return` was the only outlier), but **necessary is
not sufficient**.  At least one other site in the lifecycle
machinery must also be wrong.  Candidates to investigate next:

1. `n_json_parse` may be allocating with the wrong initial
   ref-count — `stores.database()` returns a fresh store; if the
   initial ref-count is 1 but the caller's `OpFreeRef` is also
   wired to decrement, an unrelated path may also be issuing a
   free.  The original P54 design plan (Step 1) called this out
   as the B7 root cause: "allocate the arena store inside the
   caller's variable's store, not via `stores.database()`".

2. ★ **Most likely candidate (narrowed 2026-04-14 via explore-agent
   walk of `src/scopes.rs`):** the **Set-path at lines 447-466**
   marks the `__ref_*` *temporary* binding `skip_free` when a
   native returns a struct-enum, but **does not mark the
   receiving variable `v` itself**.  Then at scope exit,
   `get_free_vars` at line 759 evaluates
   `emit = dep.is_empty() && !in_ret && !function.is_skip_free(v)`
   — since `v` was never marked, the check returns true and a
   second `OpFreeRef(v)` fires on top of the callee's internal
   free.  **Fix:** extend the Set-path marking logic to also call
   `self.vars.set_skip_free(v)` on the LHS receiving variable
   when its origin is a `Type::Enum(_, true, _)` native return.
   Mirror of the existing temporary-marking code path.

3. The interpreter codegen `state/codegen.rs:1043-1050`
   (referenced in the line 445 comment as the sibling skip-free
   logic) may need parallel treatment — only if candidate 2
   alone is insufficient.

**Estimated scope (revised):** 2-3 sessions.  Not the one-line
fix the design predicted.  Session 1: instrument the run with
`LOFT_LOG=full` for `b7_method_on_jsonvalue_returning_integer_crashes`
and confirm candidate 2's double-emit hypothesis (log every
`OpFreeRef` emission site along with the variable number).
Session 2: ship the `set_skip_free(v)` extension + the
single-line `inline_struct_return` fix together; re-run, confirm
the single-store free; un-ignore the two B7 guard tests.
Session 3: un-ignore the 5 P54 text-return-through-fn family
tests + verify `b7_multiple_json_parse_via_match_works` stays
green.

**Verification path:**
1. Run the currently-`#[ignore]`'d B7-family tests with `--ignored`
   and confirm they flip to passing:
   - `b7_method_on_jsonvalue_returning_integer_crashes`
   - `b7_character_interpolation_return_crashes`
   - `p54_extractor_as_text` and 3 sibling text-return tests
   - `p54_missing_chain_returns_jnull`
2. Then unignore them.
3. Confirm `b7_multiple_json_parse_via_match_works` (currently
   passing) stays green — guards the working multi-parse path.
4. Full suite green.

**Side-effect risk:** low.  Lifting was proven safe for
References in the P135 fix; extending to Enums preserves the
invariant (native function allocates and owns the store, lifted
temp takes ownership, OpFreeRef frees once at scope exit).  No
ref-count machinery involved.

**One fix turns 8 things green together**: 5 ignored P54 tests
(the text-return-through-fn family + chained-access) + 2
B7-prefixed guards + the INC#9 character-interpolation crash.
Highest-leverage compiler bite remaining and the bottleneck for
nearly every JSON deliverable on the roadmap.

---

## Enhancement tiers

Quality investments ranked by leverage.  Pick **one Tier 1** as the
multi-session sprint, pair with **one Tier 2** as a
session-of-the-week background bite.

### Tier 1 — closes whole classes of bugs

> **⛔ HISTORICAL — all three items have SHIPPED (items 1–2 checked 2026-07-10; item 3 closed
> 2026-08-20).**  B7 closed with the B2–B7 audit (2026-05-21, both backends); C54 (`integer` → i64)
> landed 2026-04-21 via @PLAN01 / @PLN88; the ignored-test baseline reached zero known gaps
> 2026-08-20.  Kept as a worked example of the "closes a whole class" selection criterion, which
> still applies — today it selects **Cluster C / H10**
> ([STABILITY_ROADMAP.md](STABILITY_ROADMAP.md)).

1. **B7 lifecycle for native-returned struct-enum temporaries.** ✅ CLOSED 2026-05-21.
   Unblocks 5 P54 ignored tests in one fix.  Scope analysis pattern,
   precedent in `File`'s ref-count handling.

2. **C54 integer → i64.** ✅ LANDED 2026-04-21.  Eliminates the `i32::MIN` sentinel trap
   that has spawned three documented gotchas.  Multi-session,
   sub-tickets land independently (see § C54).

3. **Drive `#[ignore]`'d tests to zero.**  ✅ **CLOSED 2026-08-20 — no known gap is
   parked behind an `#[ignore]` any more.**  `tests/ignored_tests.baseline` is down to
   ONE entry, `regen_fill_rs`, which regenerates `src/fill.rs` from `default/*.loft` and
   is maintenance rather than a gap: it is meant to be run by hand when the stdlib
   changes, and running it in CI would test the generator against its own output.  The
   `#[ignore]`s left elsewhere in `tests/` are deliberate opt-in harnesses in the same
   spirit — the differential oracle (a rustc invocation per corpus program) and one
   `host_call` measurement that is explicitly "a measurement, not a gate".

   The last real gap was **`pln102_one_known_operand_forward_float_still_mistyped`**
   (closed 2026-08-20, below).  Worth keeping the shape of that close: the test's own doc
   comment predicted the fix needed "the operator search to defer on the RESULT type
   rather than on operand knownness — a larger change than the guard widening", and that
   prediction had gone stale.  It was written when the all-unknown restriction really was
   load-bearing; loft#918 then added a second deferral at the reject site and quietly took
   over the job the restriction existed to do.  Nobody re-measured, so the note kept
   sending readers away from a one-word fix (`all` → `any`) for months.  **A parked test's
   stated reason is a claim with a date on it, not a standing fact** — re-measure it before
   budgeting for the redesign it predicts.

   Sustainable cadence when new ones appear: 1–3 per session.

   **Closed 2026-08-20:** `pln102_one_known_operand_forward_float_still_mistyped` — a
   binary op with ONE unresolved operand (`f() - 1`, `f`'s type declared lower in the
   file) was refused, not mis-valued: pass 1 matched `OpSubInt` off the literal and locked
   the local to `integer`, and pass 2's real `float` return then raised *"Variable 'a'
   cannot change type from integer to float"* — a correct line about a decision the reader
   never made.  Writing the type down (`a: float = f() - 1`) was refused too, with the
   message reversed.  The first-pass deferral in `call_op` now fires when ANY operand is
   unresolved rather than only when all are.  Guarded both ways: four `pln102_*` tests in
   `tests/issues.rs` (each verified to FAIL with the predicate reverted, and only those
   four), the pre-existing
   `pln102_one_known_operand_keeps_the_mismatch_diagnostic` for the diagnostic path, and
   `tests/scripts/forward-operand-arithmetic.loft` for both backends.

   **Closed 2026-04-14:** `file_content_nonexistent_trace` — the
   un-ignored test now exercises the regular `execute` path's
   "missing file → empty text" guarantee.  The historical
   SIGSEGV applied only under `execute_log` (LOFT_LOG=full),
   not the regular runtime; the test as written hits the
   regular path and the empty-text contract is stable today.
   The execute_log SIGSEGV (misaligned-slot codegen issue in
   the stack allocator) is a separate, deeper bug that
   doesn't gate this regression guard.

   **Closed 2026-04-14:** `p122_long_running_struct_loop` — ignored
   only because the 10 000-frame × 10-brick struct-alloc loop takes
   ~10 min in debug mode; passes in ~0.05 s in release.  Converted
   the attribute from `#[ignore]` to
   `#[cfg_attr(debug_assertions, ignore = "…")]` so the test now
   runs automatically in `cargo test --release` (CI's default) and
   continues to skip in debug for day-to-day iteration.  Debug-only
   manual run still works via `cargo test --ignored`.

### Tier 2 — preventive, low-risk, high-readability

4. **~~`cargo clippy --no-default-features --all-targets -- -D warnings`
   cleanup.~~**  Landed 2026-04-13.  Full `--no-default-features`
   build now goes clean through clippy on both lib and bin targets
   and both feature combinations still compile identically.  Fix set:
   - **`src/parallel.rs`** — 12 original lints: 7× `needless_pass_by_value`
     and 2× `not_unsafe_ptr_arg_deref` suppressed via
     `#[cfg_attr(not(feature = "threading"), allow(…))]` (the value is
     consumed by `Arc::new(program)` in the threading branch, borrowed
     in the non-threading branch; making the public fn `unsafe` would
     cascade across every `par(...)` site).  3× `needless_range_loop`
     in the non-threading fallbacks refactored to
     `for (row_idx, r) in results.iter_mut().enumerate()`.
   - **`src/parallel.rs`** — cascaded `dead_code` on 5
     `run_parallel_*` fns + `WorkerPool::new`: same cfg-gated allow,
     the binary-crate view compiled by `main.rs` sees no callers
     under `--no-default-features`.
   - **`src/main.rs`** — `extract_toml_version`, `chrono_date`,
     `days_to_ymd` moved under `#[cfg(feature = "registry")]`; the
     `registry_sync()` tail-body (formerly reached only when
     `registry` is enabled) wrapped in `#[cfg(feature = "registry")]`
     to resolve the `unreachable_code` warning that fired after the
     cfg'd-out branch's unconditional `exit(1)`.
   - **`tests/data_structures.rs`** — the lone `index_deletions` test
     that uses `rand_pcg::Pcg64Mcg` gated behind
     `#[cfg(feature = "random")]`; its imports the same.
   - **CI** — `Makefile` `ci:` target now invokes
     `cargo clippy --no-default-features --all-targets -- -D warnings`
     alongside the default-features gate, so the ratchet stays
     green on every push.
   - **Regression guard** —
     `tests/doc_hygiene.rs::ci_target_runs_no_default_features_clippy`
     reads the Makefile and fails if the gate is ever removed.

5. **Migrate `Struct.parse(text)` → `json_parse(text) → match`** in
   `tests/scripts/57-json.loft` and `tests/docs/24-json.loft` once
   P54 step 5 lands.  Unblocks step 6 (the rejection diagnostic)
   and turns the tests into examples of the modern API.

6a. **~~Drop `code!()`'s duplicate-test emission.~~**  Landed
    (investigation) 2026-04-13 — turned out to be a false positive: the
    `duplicate_macro_attributes` warning and "same test name printed
    twice" output both traced to a single orphan `#[test]`
    attribute in `tests/issues.rs` left over from a test-block
    move.  The `code!()` macro is clean.  Removed the orphan; added
    `tests/doc_hygiene.rs::no_orphan_test_attributes_in_tests_issues_rs`
    so the next orphan is caught at test time, not via a
    misattributed warning.  No further action.

6b. **~~Drift guard for `#[ignore]`'d tests.~~**  Landed 2026-04-13:
    `tests/doc_hygiene.rs::ignored_tests_baseline_is_current` loads
    `tests/ignored_tests.baseline` (name + reason per ignored test,
    20 rows today) and fails with a +/- diff when the set drifts.
    Regenerator at `tests/dump_ignored_tests.py`.  Catches
    un-ignored-without-baseline-update, silently-added new
    `#[ignore]`, and reason-string edits.  Does *not* yet run the
    ignored tests themselves and diff pass/fail/panic-message —
    that heavier nightly `--ignored` diff is the remaining gap.

6c. **~~Surface method-vs-free suggestions in diagnostics (both
    directions).~~**  Landed 2026-04-13 in `src/parser/fields.rs` and
    `src/parser/mod.rs`:
    - **method→free** (original): when field access fails and a free
      function `n_<field>` exists whose first parameter is compatible
      with the receiver type, the diagnostic now reads
      `"Unknown field vector.sum_of — did you mean the free function
      `sum_of(…)` ? (stdlib declared `sum_of` as free-only; see
      LOFT.md § Methods and function calls)"`.  Tests:
      `inc08_sum_of_is_free_function_only` locks the hint wording;
      `quality_6c_unknown_field_without_free_fn_has_no_hint` locks
      specificity (a genuinely-misspelled field still gets the plain
      message).
    - **free→method** (follow-on, landed same day): when a free call
      `name(…)` fails and a method `t_<LEN><Type>_<name>` exists on
      some other type (typically the user passed a wrong-type
      receiver to a `self:` method via free syntax), the diagnostic
      now reads `"Unknown function starts_with — did you mean the
      method `x.starts_with(…)` on text? (stdlib declared
      `starts_with` as a method; see LOFT.md § Methods and function
      calls)"`.  Methods declared on multiple receivers (e.g.
      `is_numeric` on both `text` and `character`) are enumerated
      with `/`.  Site: `src/parser/mod.rs::call` uses
      `find_method_receivers` to scan definitions for the
      `t_<LEN><Type>_<name>` pattern.  Tests:
      `quality_6c_free_call_on_wrong_type_suggests_method`,
      `quality_6c_free_call_lists_all_method_receivers`,
      `quality_6c_free_call_unknown_fn_has_no_method_hint` (negative
      — a genuinely-unknown name still prints the plain message).

6d. **~~Better errors for keyed-collection construction.~~**  Landed
    2026-04-13 in `src/parser/fields.rs::index_type`: the
    `"Indexing a non vector"` diagnostic now spells out both the
    missing feature (no generic-constructor expression) and the
    idiom that works (struct-field declaration + vector-literal
    initialisation).  Tests: `quality_6d_keyed_collection_constructor_hint`
    locks the new wording on the `hash<Row[id]>()` reproducer;
    `tests/parse_errors.rs::index_non_indexable` updated to the
    new text on its `v = 5; v[1]` baseline.  Implementing the
    generic constructor itself is a separate, larger task — not
    this diagnostic fix.

6. **Document one inconsistency per session.**  Following the
   INC#3 / INC#12 / INC#26 / INC#29 pattern — write the gotcha into
   LOFT.md, lock the behaviour with 2-3 regression tests.  INC#2
   (vector-vs-keyed-collection API gap), INC#8 (method-vs-free-function
   stdlib choice), INC#18 (`x#break` labelled-break syntax), and INC#27
   (no `x#continue` counterpart — silent bare-continue) all landed
   2026-04-13.  No further INC doc-bite candidates remain; future
   sessions should draw from Tier 1 or Tier 3 backlog items.

### Tier 3 — structural, larger payoff

7. **~~Bytecode cache verification.~~**  Landed 2026-04-13 in
   `tests/bytecode_cache.rs`.  `.loftc` shipped in commit `4039490`;
   the hit / miss / invalidation cycle is now locked with four
   process-level tests that drive the real `loft --interpret` binary
   end-to-end:
   - `first_run_writes_loftc_with_magic_header` — fresh compile
     creates `.loftc` next to the source, beginning with the `"LFC1"`
     magic bytes.
   - `second_run_reuses_cache_bytes_unchanged` — two consecutive runs
     on the same source leave `.loftc` byte-identical (hit path).
   - `source_change_invalidates_and_rewrites_cache` — editing the
     source changes the SHA-256 key; `.loftc` is rewritten and the
     new stdout reflects the new source (not a stale cached image).
   - `missing_loftc_is_recreated` — deleting the cache file between
     runs forces regeneration on the next run.

8. **~~Const store mmap path on Linux.~~**  Closed as
   deferred-by-design 2026-04-14.  [CONST_STORE.md § Phase B
   (mmap)](plans/82-const-store/README.md#memory-mapped-constant-store) reaches the
   opposite conclusion: at today's cache-file sizes (5-10 KB) mmap
   overhead (syscall + page tables) exceeds the memcpy savings, so
   the implementation path is intentionally not taken.  A benchmark
   here would lock in a micro-regression that the design has already
   ruled out.  If Phase C ever ships a large stdlib cache the
   tradeoff flips, at which point the benchmark becomes a useful
   companion to the mmap rollout — re-open then.

   In the meantime, the cache *load* path (not mmap-specific) is
   exercised end-to-end by the Tier 3 #7 bytecode-cache integration
   tests, so cache hit/miss correctness is locked even without a
   timing benchmark.  Regression guard —
   `tests/doc_hygiene.rs::quality_const_store_mmap_matches_const_store_md`
   asserts the two docs don't silently drift back out of sync.

9. **~~WASM FS bridge.~~**  Landed 2026-04-14 as STUBS, and superseded
   2026-08-11 by a real one (loft#851).  The stubs were the right answer
   while `--html` had no reachable filesystem: they made `file("x")`
   answer "absent" reliably instead of depending on what `std::fs` does
   in a given JS embedding.  `--html` now binds an actual filesystem over
   raw `loft_io` imports, so the stubs are gone and the file operations
   ask one question — the `host_fs` cfg (`build.rs`) — instead of a
   hand-written `feature = "wasm"` per site.  See
   [WASM.md § The page filesystem](WASM.md).  Tests:
   - `tests/html_wasm.rs::html_page_has_a_filesystem` and
     `::html_page_filesystem_cursor_matches_the_other_backends`
     — end-to-end over a real `--html` page, with every expected value
     taken from what `--interpret` and `--native` print for the same
     program, so it fails both on losing the filesystem and on growing
     one that answers differently.
   - `tests/html_wasm.rs::q9_html_file_content_returns_empty_on_wasm`
     — the half most easily broken by binding a filesystem: a path
     nobody wrote must still read as null, and "absent" and "an empty
     file" cross the bridge as one import.
   - `tests/html_wasm.rs::html_page_filesystem_unit_checks`
     (`tools/loft_fs_unit.mjs`) — the base tree and the reload, which a
     node-hosted page cannot reach.
   - `tests/doc_hygiene.rs::file_operations_gate_on_host_fs_not_the_wasm_feature`
     — static guard, and the reason the old one existed: these arms
     drift.  They already had, for a year — the interpreter's browser
     branches were real bridges while `codegen_runtime.rs`'s were stubs,
     so `--html` (which runs the generated code) had the empty
     filesystem and nothing said so.
   Separately, the native-only `file_content_nonexistent_trace`
   SIGSEGV under `execute_log` (called out in the test's own
   comment) is a misaligned-slot codegen issue in the stack
   allocator — unrelated to the WASM bridge and tracked
   independently.  The ignored test stays ignored.

### Tier 4 — process / hygiene

10. **PROBLEMS.md Quick-Reference is the source of truth — keep it
    that way.**  Three docs (Quick-Reference, long-form section,
    CAVEATS.md) drift independently and required two
    "doc hygiene" commits this sprint.  Either canonicalise one and
    have the others link, or add a `make docs-check` script that
    greps for FIXED markers in the long form and complains when the
    Quick-Reference still says open.
    - **Landed 2026-04-13:** `tests/doc_hygiene.rs` now guards all
      four sources — INCONSISTENCIES.md (Status blocks ↔ Resolved
      table), PROBLEMS.md (Quick-Reference ↔ long-form
      `### ~~N~~ FIXED` headings), CAVEATS.md (long-form
      `### ~~CX~~ DONE` ↔ Verification-log table), and QUALITY.md
      itself (main open-issues table must contain no crossed-out
      rows; Tier-2 strikethrough items must carry a `Landed
      YYYY-MM-DD` marker in their body).  Caught five existing
      drifts on first runs: #135 (PROBLEMS Quick-Reference), P137,
      C58/P135, and C60 (CAVEATS Verification-log), plus 6a's
      missing landing marker (QUALITY self-guard) — all corrected
      in the same commits.  Item 10's scope is now closed; future
      drift gets caught in CI instead of sprint-hygiene commits.

11. **~~Memory of recent decisions.~~**  Landed 2026-04-13.  Both
    PLANNING.md and PROBLEMS.md now open with a "Before
    proposing/opening …, check [DESIGN_DECISIONS.md]" paragraph in
    their intro — visible above the fold, not buried in the
    cross-references list at the bottom.  PLANNING.md's version
    targets feature proposals; PROBLEMS.md's version targets new
    bug reports (with pointers to C3 / C38 / C54.D as the classic
    re-opens).  Regression guard —
    `tests/doc_hygiene.rs::planning_and_problems_link_to_design_decisions`
    asserts both files mention `DESIGN_DECISIONS.md` in their first
    80 lines, so a future cleanup that strips the intro can't
    silently re-hide the register.

12. **~~A `make ship` target.~~**  Landed 2026-04-13.  `Makefile`
    now defines `ship:` as the canonical pre-push gate.  Four
    invariants chained with `&&` so the first failure aborts and a
    subsequent `git push` never runs:
    1. `cargo fmt --all -- --check` — formatting.
    2. `cargo clippy --all-targets --all-features -- -D warnings` —
       CI's exact Clippy invocation (2026-06-13: was
       `--release --all-targets`; a local pass with the narrower
       variant + a remote fail cost a full CI round, so `ship`
       mirrors the CI job verbatim now — `make gate` runs the same
       lint without the test suite for fast pre-push iteration).
    3. `cargo clippy --no-default-features --all-targets -- -D warnings`
       — the `--no-default-features` ratchet from #4 (previously easy
       to forget).
    4. `cargo test --release` — full suite.

    Distinct from `ci:` which optimises for the remote pipeline
    (logs to `result.txt`, runs GL + packages suites).  `ship` streams
    to the terminal and is the intended `make ship && git push`
    workflow.  Regression guard —
    `tests/doc_hygiene.rs::ship_target_chains_all_required_gates`
    reads the Makefile and asserts all four fragments appear in
    order, chained with `&&`.

---

## Recommended landing order

> **⛔ HISTORICAL — do not plan from this section (checked 2026-07-10).**  Every item it orders
> (B7, B5, B2, B3, C54, P54 steps 4–8) has since **shipped**: B2–B7 audited + closed 2026-05-21 on
> both backends, C54 landed 2026-04-21, P54 steps 4/5/6 complete.  Kept for the investigation
> record — the *method* (explore-agent → file:line targets → tested prediction) is still the model.
> The live ordering lives in [ROADMAP.md](ROADMAP.md) and [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md).

**Updated 2026-04-13** — explore-agent investigation produced
concrete file:line targets for all four compiler bugs.  The B7
single-line-fix prediction was tested same day and did not close
the bug; revised estimate **2-3 sessions** (the type-match
extension is necessary but not sufficient — needs paired
investigation of the duplicate-OpFreeRef site).  B5 reclassified
from "needs arena workaround forever" to "one-session
memoization fix in `fill_database`".  Order rebuilt around these
findings.

1. **B7 — 2-3 session lifecycle fix** starting at
   `src/scopes.rs:1031` plus paired sites yet to be identified
   via `LOFT_LOG=full` instrumentation.  Still highest-leverage
   compiler bite — unblocks 5 ignored P54 tests + 2 B7-prefixed
   guards + the INC#9 character-interpolation crash + every
   `(JsonValue) -> T` method call.
2. **Q2** — `kind` / `keys` / `fields` / `has_field` natives
   become trivially shippable post-B7.  One session.
3. **B5 — memoise `fill_database`** in `src/typedef.rs`.  Removes
   the arena-indirection compulsion from P54 step 4 and unblocks
   future stdlib enums with recursive variants (Tree<T>,
   Result<T, E>, etc.).  One session.
4. **P54 step 4** — array/object materialisation.  Simpler
   post-B5 (natural `vector<JsonValue>` works); Q3 + Q4 unlock.
5. **Q1 schema-side reuse** — when P54 step 5 lands,
   `Type::parse(JsonValue)` reuses the already-shipped
   `format_error` infrastructure for per-field path diagnostics.
6. **P54 step 5** — `Type::parse(JsonValue)` codegen with the
   field-type matrix + strict / permissive policy.
7. **Q4** — `json_null` / `json_bool` / … / `json_object`
   constructors.  Bypasses B2-runtime by allocating in Rust;
   ships any time after step 4.
8. **Q3** — `to_json` / `to_json_pretty` + `T.to_json()` codegen.
   Round-trip tests become possible.
9. **B2-runtime — zero-fill unit-variant payload** in
   `src/parser/objects.rs::parse_enum_field`.  Quality-of-life for
   any user constructing struct-enum literals at runtime; not a
   P54 blocker (Q4 bypass already works).  One session.
10. **B3 — four-layer codegen surgery** for struct-enum tail
    returns.  2-3 sessions.  Closes the implicit-return ergonomics
    gap; the `return n;` workaround stays good for any user who
    needs it.  Lower priority than items 1-9.
11. **P54 step 6** — sweep stdlib/tests off `Struct.parse(text)`,
    ship rejection diagnostic.
12. **P54 steps 7-8** — unignore remaining P54 tests; doc sweep.
13. **C54.A → C → B → E** — integer i64 widening.  Schedule last
    in 0.9.0 so earlier bites are fixed on the existing layout
    before the schema bump.

Tier 2 items run in parallel as session-of-the-week background
bites.  Tier 3 / 4 — at most one per release window.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — historical bug log (interpreter
  robustness, web services, graphics)
- [CAVEATS.md](CAVEATS.md) — verifiable edge cases with reproducers
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design
  asymmetries
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — closed-by-decision
  register
- [PLANNING.md](PLANNING.md) — priority-ordered enhancement backlog
- [ROADMAP.md](ROADMAP.md) — items grouped by milestone
- [DEVELOPMENT.md](DEVELOPMENT.md) — branching, commit order, CI
