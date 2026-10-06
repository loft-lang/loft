// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging Strategy: store faults

Store faults: random-looking crashes, symptoms in neighbours, ownership bugs (leaks, double-frees) and the debug-assertions calibration run.  Part of the debugging guide: [DEBUG.md](DEBUG.md).

---

## Before you believe a fault is RANDOM

A fault that reproduces some runs and not others is usually not random — it is a
run that starts from different state than you think. Check that *first*, because
"random" is expensive: it sends you to repeat-run harnesses and mechanism traces
when a one-line probe would have settled it.

**The tell is the ratio itself.** `1/12` or `1/20` is the signature of *the first
run differs*, not of randomness — a genuinely random fault lands on a ratio that
moves when you re-measure. Read the *sequence*, not the count.

Two probes, both cheap, before any theory:

```bash
# 1. Is it ordered?  A cold/warm split reads ok, BAD, BAD, BAD — not scattered.
rm -rf ~/.cache/loft/program-*
for i in 1 2 3 4; do loft --native p.loft; done

# 2. Turn the suspected state off.  If the fault vanishes, you have located it.
LOFT_NO_CACHE=1 loft --native p.loft
```

**State a run inherits, none of it cleared by removing `.loft`:** the
whole-program bundle in `$XDG_CACHE_HOME/loft` (`cache::program_cache_paths`,
default-ON — `LOFT_NO_CACHE` disables; and already OFF for a binary under
`target/{debug,release}/`, so on a from-source loft the bundle is not one of
your variables — STARTUP_CACHE.md § Which loft am I measuring), the stdlib bundle
(`LOFT_STDLIB_CACHE`), `target/` build artefacts, and an installed
`$(which loft)` on `PATH`.

This is not hypothetical: the native duplicate-type-mint fault
([plans/native-type-mint](plans/native-type-mint/README.md)) was recorded as
"per-process random" for a whole session and a harness was built around that
belief. Its `repeat.sh` cleared the per-directory `.loft` cache before every run
but never touched the program bundle, so it measured
cold-once-then-warm-forever and reported it as 1-in-12. The real fault was
deterministic: warm load, every time.

**So the rule is about the instrument, not the bug.** Before theorising about a
random fault, prove each run really starts from the state you believe. A harness
that clears the wrong cache reports a ratio with total confidence, and the ratio
is fiction.

---

## When the symptom is in the NEIGHBOURS, not the crashing code

A write that lands outside its record damages whatever sits next to it, so the fault
surfaces in unrelated code and its shape depends on what the neighbouring bytes held.
That produces the most misleading bug report there is: "non-deterministic",
"non-monotonic in the input size", "crashes under the test runner but is fine as a
program". Every one of those is a downstream artefact, and chasing them costs days.

loft#796 was exactly this. `Stores::position` answers `u16::MAX` for a field the layout
has no slot for, and that answer was used as the field OFFSET — so a ten-field struct
laid out with nine wrote at `record + 65535`. One program gave a SIGSEGV inside an
unrelated claim walk, a 59.6 GiB allocation, or a clean pass, run to run.

**The escalation that worked, in order:**

```bash
# 1. A real backtrace beats any amount of tracing.  Name the faulting FUNCTION first.
gdb -q -batch -ex run -ex "bt 25" --args loft --interpret --tests probe.loft

# 2. Turn the store bounds sentinels ON.  They are OFF in ordinary builds
#    (`profile.dev.package.loft` sets debug-assertions = false for speed), so a
#    release binary built WITH them is the instrument — and it usually turns a
#    random crash into a deterministic, named assertion.
CARGO_TARGET_DIR=/tmp/loft_dbg RUSTFLAGS="-C debug-assertions=on" cargo build --release
cp /tmp/loft_dbg/release/loft target/release/loft_dbg   # so `default/` still resolves

# 3. With it deterministic, the interpreter trace names the op outright.
LOFT_LOG=crash_tail:35 target/release/loft_dbg --interpret --tests probe.loft
#   -> GetField(v1=ref(15,1,8), fld=65535) -> ref(15,1,65543)=<oob>
```

Step 2 is the one worth remembering: the sentinels exist and are disabled by default, so
"the release binary does not check that" is a fact about the PROFILE, not the code.

**Also cap the process.** A corrupted length ends in a bad dereference on one run and an
unbounded allocation on the next; `LOFT_TIMEOUT` bounds time, not memory. Wrap a
repeat-run harness in `( ulimit -v 6000000; exec loft … )` — the kernel's OOM killer is
free to kill a bystander instead of the runaway (it took out two unrelated sessions
during this hunt). Test runs additionally carry loft's own store ceiling, which names
the type that filled the heap (RUN_BOUNDS.md § Store-memory ceiling).

---

## Debugging store-ownership bugs (leaks, double-frees, non-determinism)

The word-addressed `Store` arena (`Vec<u64>`) is **invisible to valgrind** —
the buffer is validly allocated, so corruption *within* it (a stale `DbRef`
read, a record reused while still referenced, a length read before it is
written) shows up only as a wrong or **non-deterministic** result, never as a
valgrind error.  `claim()` does NOT zero reclaimed slack, and a freed
tree-tracked block stores its LLRB free-list pointers at **offset 4 — exactly
where a vector's length word lives**.  This family (`@P311`, `@P313`, `@P314`,
`@P317`) is the hardest to pin; these levers cut the time dramatically:

> **Start with `LOFT_STRICT_STORES=1` for a PREMATURE free** — a callee freeing a
> store its caller still holds, which surfaces as a field the program never wrote
> reading back as another record's data.  It names the access and the free.
> `LOFT_UAF` is the row that *sounds* like the one for this and is not: it scans the
> LIVE FRAME only, so a cross-frame premature free reports nothing while the same run
> emits unrelated same-frame noise (loft#939 — the report reads as "no detector sees
> it", which is what sends you off building one).

**One net is always on and needs no switch: every store access is bounded.**
`Store::addr` / `addr_mut` / `read_span` / `write_span` / `buffer` check the offset
against the store's capacity in every build and on every target, and a failure reads

```
Store access out of bounds: rec=4294967295 fld=0 width=8 store_bytes=32 type=…
  — the reference is corrupt, not merely out of range; run with
    LOFT_STRICT_STORES=1 to name the free
```

Read it as a *store-lifetime* report, not an indexing one: a `rec` that trips this is
garbage rather than off by one, so the fault to hunt is whatever produced the
reference. On `--html` the panic text and the loft frames under it reach the browser
console.

This used to be a `debug_assert!`, which loft's library build compiles out
(`[profile.dev.package.loft]` sets `debug-assertions = false`, so such a guard is
vacuous in `cargo build`, `cargo test` and `make ci` alike),
so the only bound surviving a release build was `checked_offset`'s `isize::try_from`
— and that can fail **solely where `isize` is 32 bits**. One corrupt `DbRef` therefore
trapped in a browser page while every 64-bit backend addressed whatever lay at the
offset it computed: a silently wrong scalar, or a wild `&mut` into process memory.
loft#950 cost a day to that asymmetry, because "the browser traps and the interpreter
is green" reads as evidence about the browser and is only evidence about where the
guard could speak. Cost of making it real: +2.5 % instructions on `--native` (the
default backend), +9.4 % on `--interpret`, on a loop that does nothing but touch
struct fields.



| Lever | What it does | Use when |
|---|---|---|
| `LOFT_STORE_GUARD=1` | Reports each block-confined vector store that is scoped (and freed) later than the block it is confined to — the lifetime model under-freeing (Goal E).  Read-only, off by default.  Confinement is the least-common-ancestor of every reference's scope-path, with escape exclusions (return/yield/break, block-result, tuple-element, dep-aliasing) and loop-internal reuse excluded — adversarially hardened by `plans/2-vector-store-watermark/probes/cluster-I/`. | "Does a program hold more heap than the source implies?"  Drive the store-lifetime fix until it is silent corpus-wide, then promote to a `debug_assertions` assert.  See [GOALS.md Goal E](GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth). |
| `LOFT_LOG=zero_claim` (or `LOFT_ZERO_CLAIM=1`) | Zeroes every freshly-claimed payload and grown tail (off by default: a claim writes nothing, `@FR-H-Claim`), so a read-before-write / stale read returns a deterministic `0` instead of arena garbage. | A result is **non-deterministic** run-to-run.  If `zero_claim` makes it deterministic-and-correct → a read-before-write: find it with `LOFT_POISON_CLAIM=1` and fix the READER (write the value it assumed where the value is built), never the claim.  If it stays non-deterministic → NOT a claimed-slack read (rule it out; suspect a deep-copy logic bug or addresses-as-data). |
| `LOFT_LOG=poison_free` | Overwrites a store's buffer with `0xDEADBEEF` on free. | Suspected use-after-free of a *whole store*.  No effect ⇒ not a freed-store UAF. |
| `LOFT_UAF_GEN=1` (+ `LOFT_UAF_SRC=1`) | Detector (c): stamps every DbRef pushed on the operand stack with its store's generation and reports at the matching pop when the slot was freed since.  `_SRC` adds the freeing pc + op. | The freed-then-**reused** read `LOFT_POISON` is blind to (the new occupant is live, so the bytes look fine).  **Scope: only the window between a push and its pop** — a ref that goes stale sitting in a FRAME slot is invisible to it, which is why it never saw loft#723. |
| `LOFT_UAF_GEN_INJECT=1` | Ages every ref just after its push stamps it, so each is stale while live.  The positive control for the row above. | Before believing a silent `LOFT_UAF_GEN` run.  A detector that cannot fire and a clean corpus look identical; under injection ~471/548 corpus scripts report, against 0 without it. |
| `LOFT_STACK_CENSUS=1` (@PLN154 phase 0) | **Which code writes the interpreter stack.** After every operator it diffs the stack store's live frame against a snapshot and subtracts the spans `put_stack` declared, so a write through a route nobody has listed is counted like any other — the ground truth is the memory, not an inventory of callers.  Reports the share of changed bytes that arrived through `put_stack` and the opcodes responsible for the rest.  `LOFT_STACK_CENSUS_MAX_OPS=N` reports after `N` ops and stops, which is what makes a corpus sweep tractable: without it the few scripts that run tens of millions of ops eat the wall clock and then report nothing, because the timeout kills them before exit. | **"Is this accessor the chokepoint?"** — asked of the stack before a shadow keys its tags there.  Measured over 1106 corpus programs: `put_stack` carries 74.5 % of the bytes and is 1 of 33 write sites, and **no program** is covered by it alone.  Read the shares as a FLOOR — a byte written and restored inside one op, or written with the value it already held, does not change and so is invisible to a diff; pair it with the static site count.  `--interpret`, main dispatch loop only; ~20x, so a probe and never a sweep in CI. |
| `LOFT_VERIFY_STACK=1` (@PLN154 phases 1-3) | **A frame slot nothing wrote, a HANDLE read as a value, or a handle whose record has MOVED — each reported at the READ.** One tag word per stack byte, carried by the stack `Store` itself: a write through `Store::addr_mut::<T>` tags the bytes it covers with `T`'s family and width, a byte move (`copy_block`, `addr_span_mut`) carries the tags with the bytes, and a slot that leaves the live frame — a pop, a `reserve_frame`, any route that lowers the stack pointer — loses them.  The check sits at `get_stack` / `get_var`, never at `Store::addr`, which is what the debugger and the frame renderer read stale slots with on purpose.  **Tag low, check high.**  Three findings: a span NO write reached; a `DbRef` read as a value or a value read as one; and a handle whose record was relocated by a container outgrowing its allocation — `Store::resize`'s claim/copy/delete logs the move, and the dispatch loop then walks the live frame reading only the slots the shadow already says are the base of a handle, so the scan is exact rather than a guess at which aligned words are references.  A width disagreement is COUNTED, not reported — the frame carries composite slots the compiler addresses field by field (the 20-byte fn-ref slot read as an `i64` and a `DbRef`, `OpStep`'s two `u32`s, a `boolean` consumed at its stepped 8-byte slot), and a width rule reported 43 of the first 180 corpus programs. | **The definite-assignment class `LOFT_POISON` was built for, plus the monomorph-layout class.**  Reach for it when a value appears from nowhere, when a result differs between RUNS, when a free refuses on a frame word, when a generic answers a plausible number its concrete twin does not, or when a value bound out of a container goes wrong only once that container GROWS.  Calibrated both ways: silent on all 1106 runnable corpus programs at HEAD; four sites on `64437246` (the nullable-local pre-init control), `handle 12` read as `i64` on loft#1028's, four sites on loft#1016's; and one site each on loft#1373 / #1377 / #1384, which are OPEN, so HEAD is the broken build for those.  Its reach ENDS at the frame: loft#1070's control answers `4294967198` and the shadow is silent, because the wrong layout is in a heap RECORD and the slot holds a correct handle.  `--interpret` only. |
| `LOFT_VERIFY_STACK_TRACE=1` | Names every handle-tagged frame slot the stale scan read, with the relocations it was compared against. | **When a stale check is SILENT and you want to know which half was silent.**  The summary says whether anything moved and whether any slot named it; this says which slot.  It is what found the scan's own addressing bug — the shadow is indexed absolutely (`rec * 8 + fld`) and `Store::addr` takes the FIELD, so counting the record twice made every "handle" it printed a `store=8 rec=3414097922`. |
| `LOFT_VERIFY_STACK_INJECT=1` | Suppresses the tag at the write hook, so every checked read reports.  The positive control for the row above. | Before believing a silent `LOFT_VERIFY_STACK` run — the same reason `LOFT_UAF_GEN_INJECT` exists.  A trivial three-line program reports 14 distinct sites under it and none without. |
| `LOFT_NO_SLOT_REUSE=1` **+** `LOFT_POISON=1` | No freed slot is ever reclaimed, so a freed store stays freed *and* poisoned. | **Ground truth for "is this reported stale read real?"** — with reuse off, a genuine stale read must land on `0xDEADBEEF`.  Clean + correct here ⇒ the report is a false positive.  This is what convicted the `LOFT_UAF_GEN` offset-keyed-stamp bug. |
| `LOFT_STRICT_STORES=1` (@PLN130 F8) | **Strict store lifetime, and both faults are ERRORS.** A freed store stays dead (it implies `LOFT_NO_SLOT_REUSE`), so its number is retired: a run that makes more than 65535 stores in all, live or freed, cannot be checked this way and stops saying so — *"exhausted by the checking mode, not by a leak"*, with the live count — check such a program with a smaller loop and any read or write through a reference naming it is reported AT the access; a store still live at exit is reported too; a non-zero exit if either fired, on **both** backends. Reports developer detail — store slot, type, rec/pos, and `killed by the free of \`<var>\``, plus the created/last-op/freed/now pcs on `--interpret` (generated Rust has no pc, so native omits them rather than printing `pc=0` four times). | **The frame-slot blindness the row above names.** `LOFT_UAF_GEN` only watches the push→pop window, so a ref that goes stale sitting in a variable reported nothing — it scored 0 on @PLN130 probe 36, which poison proves is dangling. Exactness comes from no-reuse: an unrecycled slot cannot be legitimately re-occupied, so `free == true` at an access is unambiguous — no stamps, no `DbRef` widening, no false positives to explain away. **For PROBES only**: never reusing a slot walks a long run off the end of the `u16` store space, which is exactly why it is opt-in. Calibrated both ways (fires 4× on probe 36; silent across 40 clean scripts on both backends) — check both directions before trusting a silent run. |
| `LOFT_COPY_MANIFEST=1` (@PLN130 F5) | **The copy-diagnostic completeness guard.** Every generator records each deep copy it WRITES — at the branch that writes it, past every early return, so a last-use move or an adopt is never miscounted — and the guard diffs that manifest against `use_analysis`'s verdicts, reporting the copies **no diagnostic accounts for**. Compile-time only; nothing reaches a compiled program. Origins: `InterpRecordBind` / `InterpCallReturn` / `InterpTupleBind` (`state/codegen.rs`) and `NativeRecordBind` / `NativeCallReturn` (`generation/dispatch.rs`, the latter a runtime adopt-or-copy so rendered as *may*-copy). | **"Does the copy report cover everything?"** — a question the report itself cannot answer, because a copy it never classified is exactly the one it stays silent about. Validate a manifest against KNOWN-uncovered cases, never by reading the code: instrumenting the plausibly-named `gen_set_first_ref_copy` left the probe silent (it fires zero times in the corpus) while the real emitter is `gen_set_first_ref_call_copy` — a hole that only a calibrated guard catches. The uncovered set is **not empty today** (29 sites over a 90-script sample), which is why this is opt-in rather than a gate; @PLN130 decision 3 makes an `Avoidable` copy a legitimate resting state so long as it is STATED. **With `LOFT_DROP_COPY_CENSUS=1` beside it** (@PLN163 P2b) each report also prints `lease-manifest: N emitted copies of a droppable, U without a lease verdict` and one `lease-unjudged` row per such copy: a copy the census judged at the same destination is covered, and a bind of a call result is covered by the census's verdict on the callee's `return`. One line per generator — two under `--native-emit`. Gated by `ownership_drop_gate::every_emitted_copy_of_a_droppable_has_a_lease_verdict`, at zero. |
| `LOFT_DROP_COPY_CENSUS=1` (@PLN163 P0) | **Every copy of a record that has a release to run, one line per site, then the count.** Read after the scope pass: an `OpCopyRecord` of a type with a cascade or hook, and a whole-value bind of a type that owns a droppable. Each line names the function, the line, the destination kind (`bind`, `bind-view`, `place`, `element`, `buffer`, `record`, `snapshot`, `tuple`, `item`, `append`, `return`), the type, the root of every value the source may take — through a join's arms and a compiler view temp's dependencies — the destination, `lease=` and `liveness=` (below), and `frees-source` when the op frees its source. `snapshot` is the scope pass's copy of a displaced record for its hook (`Scopes::displaced_drop`), not a copy the program asks for (`lease=-`). A tuple literal that copies every member of one tuple in order — `u = t`, or a nested tuple copied through a hold — is one `tuple` site from that tuple; a droppable placed in a tuple literal as an item (`(p, 1)`, `(s.h, 1)`) is an `item` site whether the compiler copies or views it; a bind whose type depends on its own source (a destructure's `__ref_2 = u`, a hold) is a view and is not listed; a `return` of a parameter or a place reached through one, which copies nothing in the callee, is a `return` site.  A whole COLLECTION placed in another one is an `append` site, because the parser writes it as an append and never as a bind — `d = v` mints a `__vdb_N` backing and fills it with `OpAppendVector(d, v)`, and so do `d = b.v`, the concat `v += w`, the self-append `v += v` and `d = a + b` (two sites, one per operand).  The SOURCE decides and not the op: one parts loop emits that node both for a copy and for a fresh literal, so `v += mkv()` answers `move`, `v += [mk()]` emits no append at all, and an append into a FIELD (`b.v += w`) is `Uses::construct_copy`'s site rather than one of these. The parser's erased `x = x` and a join arm that is a variable as it is (`a = a ?? mk()`) are `bind` sites too — neither emits a copy, and the rule judges the line all the same. **`lease=`** is `(H-Copy-Refuse)` read off the line (`src/lease.rs`, `Frame::written_verdict`): `move` for a fresh value, a `return` of what the function owns, a block yielding its own variable, or a member read through a copy made only to read it (`mk_s(7).h.id`); otherwise `refuse:` with the reason — `copy:<var>` (an existing value placed without a copy), `container:<root>` (a member the container still holds), `caller:<param>`, `captured:<var>`. **`liveness=`** is the superseded first version, kept for `(H-Elide)`: `move`, `refuse:later:<var>@<line>` (used again on some path), the same `container`/`caller`/`captured` reasons, `unreached` for an IR shape the liveness pass does not walk, or `-` on a site it does not judge. The count prints even at zero. Compile-time only. | **"Which copies will the copy-lease rules decide about, and how?"** — a type without `OpCopy` will refuse the `refuse:` rows and a type with one will run it. A projection bound to a variable or placed in a tuple member is a VIEW and is not listed. Calibrated by `ownership_drop_gate::the_census_names_the_copy_each_cell_makes` against every CROSS and COALESCE cell, and falsified by disabling its bind arm (54 cells fail) or its append arm (`p_v1`, `p_v2`, `p_v3` fail, naming the spelling the enumeration stopped reaching). It reads the IR, not the emitted code: the copies a generator mints at emission are `LOFT_COPY_MANIFEST`'s, above. |
| `LOFT_HOIST_VERIFY=1` (loft#885) | **The gate on the vector-header hoist.** `--native` derives a vector's `(store, record, length)` once before a loop it proved writes no store, and reads elements against that triple. This emits the CHECKING form of every such read: it re-derives the header and panics naming the stale one. A **generation**-time switch, not a runtime one — the check costs exactly the loads the hoist removed — and a const parameter rather than a `debug_assert!`, which loft's library build compiles out. | **"Did the hoist gate let a write through?"** — the failure mode is a silent wrong read out of a moved record, so run a suspect program (or the suite) under it. Calibrated: re-classifying `OpRemoveVector` as a reader on purpose makes an unarmed run answer 120 where every other backend says 38, and an armed one panic. Pairs with the row below. |
| `LOFT_NO_VECTOR_HOIST=1` (loft#885) | Emits every indexed read the way it was emitted before the hoist landed. Generation-time; nothing else changes. | **The before-half of an A/B on ONE binary** — for the performance comparison, and as the first bisect step when `--native` answers differently from `--interpret` on a loop that indexes a vector. Same answer under both settings ⇒ the hoist is not your bug. |
| `LOFT_DEBUG_F8=1` (@PLN130 F8) | Names every VIEW binding whose deps `scopes::collect_views_to_materialise` stripped — i.e. the views it judged live across a re-establishment of their container, so they materialise into their own store instead of aliasing. One line per function that has any. | **A materialise decision that fires where it should not, or not where it should.** The analysis is keyed on the BINDING, not the container, and the difference is invisible in the output: a per-function *"is the container ever reassigned"* reading strips an unrolled `for pf in fields(p)` iteration subject and puts an `OpFreeRef` in a scope where the declaration is not visible (`45-field-iter` stops compiling under `--native`). Read this against the `advice:` lines — a view listed here with no advice, or advice with no strip, means the two disagree. |
| `LOFT_STORES=log` | Per-alloc/free trace (`+ alloc #N`, `- free #N`). | Find a `free` then `alloc` of the same store while a `DbRef` is still live.  Note: a store is logged under the var name at *free* time, which may differ from its *alloc* name. |
| `LOFT_STORES=warn` | Warns when >30 stores are active. | Catch a runaway leak early.  **Note (@PLN103):** this OVER-warns — a large *working set* (many concurrently-live stores, all freed) trips it though it is not a leak.  Prefer `=timeline` to disambiguate. |
| `LOFT_STORES=timeline` (@PLN103) | Per-store lifeline with a STABLE id `#<store_nr>.<seq>` (the `seq` disambiguates the reused `store_nr` slot, so a `free` prints the same id as its `alloc`), plus an exit SUMMARY: `<allocs>, <frees>, peak <N> concurrently-live (working set)` reconciled with the authoritative leak count. Both backends. | The working-set-vs-leak question `=warn` can't answer: a high `peak` with `NO leak` is a big-but-clean working set; a real leak reports `N user store(s) LEAKED`.  Also: match a freed-then-reused slot by its `.seq`.  **The label column is native-only** (loft#759): generated code passes the variable to `free_named`, so a native line reads `free #3.5 var___ref_1`, while every interpreter line reads `·` — the interp `FreeRef` opcode takes its `DbRef` off the stack and carries no name.  So on `--interpret` the timeline says WHICH store died, never which site killed it; pair it with `loft introspect` (which does name the free) rather than reading the interp column for attribution. |
| `LOFT_TEXT_TIMELINE` (@PLN104) | The **text-buffer** analogue of `LOFT_STORES=timeline` — text values are Rust `String`s on the stack frame, so their heap allocation is INVISIBLE to the store timeline / `check_store_leaks`. Any value → exit SUMMARY (`<allocs>, <frees>, peak <N> bytes live`) + a `LEAKED #seq fn=<d_nr> <bytes> <content>` line for every `String` still live at exit; `=timeline` adds a per-op `grow`/`free` lifeline. Interp only (native RAII frees text). Realloc-safe (capacity delta by ptr). ONE ledger for the process, so a `par` worker's or a `parallel` arm's buffers count; and the summary prints at the end of a `--tests` / `loft test` run as well as at a program's exit (loft#1357 — before that a worker's orphan read as "NO text leak" and a `main`-less guard printed nothing). | **The loft#568 owned-text-return orphan class** — the leak `LOFT_STORES=timeline` is blind to and the ASan `ir_read` suppression measures UNRELIABLY (stack-substring, false-pos/negs by `malloc_context_size`). This is DETERMINISTIC and names the leaking fn + content. Reach for it first on a suspected text-return leak. |
| `LOFT_TRACE_DB=1` | Every `OpDatabase` call with the type it allocates and the `DbRef` the target slot held ON ENTRY, **and every free with an `already_free` column**.  **Both backends** — the native runtime's line names the type too, so a call that crosses into a package's shared library still prints (it did not before loft#810, which is exactly where the adoption was). | Pin cross-iter slot dangling (a slot's stale DbRef gets `clear+claim`'d, clobbering another var's record).  The ENTRY `DbRef` is the whole point: a non-null one means this allocation ADOPTED a slot, and if a fresh `null_named` handed that same slot to somebody else first, the record now has two owners.  Pair with `LOFT_STORES=log` — the interleaving of `+ alloc #N` against the adoptions is what names the second owner — and confirm with `LOFT_NO_SLOT_REUSE=1`.  **`already_free=false` is the second owner arriving the OTHER way**: the free itself is the fault, because the `DbRef` reaching it is a stale copy and the slot it releases is still in use — the next allocation then takes it legitimately, so the allocation trace alone shows nothing wrong (loft#1085).  Added during PLAN51 Cluster II diagnosis. |
| `LOFT_TRACE_CR=1` | Every interp `OpCopyRecord` with src+dst + Canvas field reads BEFORE and AFTER copy. | Pin same-store copy corruption (`remove_claims` frees nested vec records before `copy_block` reads them) or wrong-source mid-copy.  Added during PLAN51 Cluster II diagnosis. |
| `LOFT_TRACE_LEX=1` (#625) | The lexer's POSITION bookkeeping: every recorded identifier position (`idpos`), every `to()` seek, every `revert`, every memory `replay`. | **A diagnostic naming the wrong LINE.** The reporting cursor is shared and long-lived: any warning pass may seek it BACKWARDS to point at an earlier site, and `to()` moves only that cursor — the tokenizer keeps counting lines from wherever it was left, so an unrestored seek shifts every LATER diagnostic and the symptom surfaces in an unrelated message. Run it and **diff the two passes**: the pass that records a token at the wrong line names the seek just before it. |
| `LOFT_TRACE_SCHEMA=1` (#618) | Every `Stores` type registration and rollback, plus the **DEF** behind each (`fill d_nr=… name=… -> reg=…`). | **"Double structure type" aborts**, and any suspicion that a speculative parse (REPL capture, `infer_type`) is not schema-neutral. The abort names only the colliding type; the fault is normally ONE def filled twice (a rolled-back parse re-creating it), which is visible only as the same `d_nr` registering a bare name and then a `src0::`-qualified one. |
| `LOFT_TRACE_COPY=1` | Native-side OpCopyRecord trace (src, dst, size, free_src). | Companion to `LOFT_TRACE_CR` for native; pin schema-mismatch copies (compile-side layout vs runtime-side layout disagree). |
| `LOFT_TRACE_FINISH=1` | Every `finish_type` entry/exit for tuple types (size, align, field_groups count). | Pin tuple-schema propagation gaps (compiler side has groups, runtime side doesn't → wrong size).  Added during PLAN51 V-a diagnosis. |
| `LOFT_TRACE_KEYS=1` | The baked key-comparison descriptors per keyed collection (`t_nr`, name, the descriptors). | Key ARITY desynchronised from the lookup twice (a `spatial<T[x,y]>`, a tuple key field), and both times the symptom was a collection read from the wrong store, naming no key. |
| `LOFT_TRACE_VADD=1` | One line per vector concat / append-copy with the STRIDE `vector_add` resolved (read once; the op is runtime-hot). | A wrong element or width out of `v += w` over a nested element type — the instrument that settled the nested stride. |
| `LOFT_TRACE_RETFRESH=1` | Per callee, the return-freshness verdict WITH the tail it read (`tail=…`). | A `false` costs a leak the caller never sees and a wrong `true` a use-after-free; "no return sites" and "a tail this cannot read" are different answers with the same verdict, so the tail comes along. |
| `LOFT_TRACE_PREAMBLE=1` | The parser's call-preamble decisions per argument (`pass1`, the argument, whether it inlines, its deps, the hidden-buffer flag). | An argument evaluated twice, or a hidden buffer handed where a value was due. |
| `LOFT_TRACE_INSTANCE_KEY=1` | Each method-shaped generic instance key rewritten (`old key -> new key`) with the spelling of every binding. | Two instances of one generic that should be one (a duplicate `t_…` emitted), or one that should be two. |
| `LOFT_TRACE_CLOSURE_KEEP=1` | Per function, the closure-keep analysis: the records kept, their holders and which are owning. | A captured record freed under a live closure, or kept past its last use. |
| `LOFT_TRACE_PAR_WORKERS=1` | Per `parallel` dispatch (native only, read once): the site, `n_rows`, threads, DISTINCT workers and the indices each took. | A `par` that ran on one thread, or two workers that took the same row. |
| `LOFT_KEEP_NATIVE_RS=1` | Preserves the generated Rust at `/tmp/loft_native_*.rs` instead of cleaning it. | Read the generated Rust at a specific line a runtime panic cites.  Added during PLAN51 V-c diagnosis. |
| `check_store_leaks` (interp, **`--interpret` only** — see note) / `LOFT_NATIVE_LEAK_CHECK=1` (native) | At-exit summary of unfreed stores, **aggregated by type** (`kt=68 ChunkKey×6026`). | Pin *which type* leaks.  Run the **same** repro on both backends — a leak on one and not the other means a backend-specific free emission bug (the @P317 symptom-2 shape). |
| `--native-emit out.rs` | Writes the generated Rust and exits. | A native-only bug.  Read the generated function: look for a `null_named(...)` placeholder that is overwritten without a free, or a missing/extra `OpFreeRef`. |
| `"Allocating a used store #N (known_type=…, requested by=…)"` panic (`allocation.rs:104`) | The store-pool tripwire (free-bitmap vs `store.free` disagree), now with slot + type + requester. | Fires at the *next* allocation after the real over-free/leak — a tripwire, not the bug site.  The pool near `u16::MAX` ⇒ a leak exhausted the pool and `max` wrapped to 0; otherwise a double-free. |

> **Gotcha — the interpreter leak check needs `--interpret`.** Bare `loft prog.loft` runs the
> **default `--native` mode** (`main.rs` `native_mode = true`): it compiles + runs the native binary
> and exits via the subprocess status BEFORE the interpreter's `check_store_leaks` is reached, so a
> bare run prints NO leak warning even on a real interpreter leak. Always leak-check with
> `loft --interpret prog.loft` (or rely on the test harness, which is interpreter-based:
> `tests/leak.rs::leaks_for`, `loft_suite`). Native leak-checking is the separate
> `LOFT_NATIVE_LEAK_CHECK=1` axis. Note also that some leaks are interpreter-only: the eager
> `OpInitRef` null-init allocates a store in the interpreter but native lowers it to `DbRef::NULL`
> (no allocation), so an interpreter `kt=65535` leak can be genuinely absent under native.

Workflow: reproduce minimally, run on **both** backends (divergence localises
the backend), use `zero_claim` to classify the non-determinism, then `--native-emit`
+ `LOFT_STORES=log` to pin the site.  Mirror the @P311/@P313 fix shape (a
missing/spurious `0x8000` free-source bit or a `null_named`-vs-sentinel
choice in `src/generation/dispatch.rs::emit_null_dbref`).

---

## The debug-assertions calibration run (`target-da`)

**Every lib-side `debug_assert!` / `#[cfg(debug_assertions)]` check is compiled
OUT of every ordinary build** — dev, test, AND `--release` —
by `[profile.dev.package.loft] debug-assertions = false` (dev/test) and the
release profile default.  That covers `Store::valid`/`Store::validate`, the
`keys.rs`/`store.rs` boundary guards, codegen sanity asserts
(`generate_set`/`generate_call`), the `get_stack` corrupt-DbRef guard, and the
`[set_var]` width warnings.  The only standing build that checks them is the
cargo-fuzz target.

**The H5 two-pass contract is NOT one of them** — `assert_pass2_def_attr_stable` is a plain
`assert!` and runs in every build, which is why a program that trips it aborts an ordinary
`cargo build --bin loft` run rather than passing quietly.  Its sibling
`check_argument_geometry` (`src/state/codegen.rs`) is the same: always on, always fatal.  Both
are compile-time contracts, so a listing of what the calibration run covers must not claim
them ([COMPILER.md § The H5 two-pass contract](COMPILER.md)).  So for any claim guarded by a debug assert, "the suite is
green" is a **calibration failure** — the instrument is not installed in that
build.  The first-ever full calibration (2026-07-03, @PLN85) found four
long-latent H5 producers plus a latent-assert inventory; the open cells live in
`plans/85-store-lifetime-retirement/fuzz-proof-gate.md` § final honest DA map.

Run the calibration in a **separate target dir** (one-time ~full rebuild,
then incremental):

```bash
# one-time: the CLI resolves the stdlib relative to the exe, and
# project_dir() hardcodes target/release|debug — non-standard dirs miss it
ln -sfn ../../default target-da/release/default

RUSTFLAGS="-C debug-assertions=on" CARGO_TARGET_DIR=target-da \
  cargo test --release --no-fail-fast
```

**For ONE specific assert, sweep the corpus instead.**  The calibration run above
arms every check at once, which is what you want for an inventory.  When you are
chasing a single fault and a dead `debug_assert!` already names it, the cheaper
move is to replace that one assert with an env-gated `eprintln!` and run it over
every `.loft` in the tree:

```bash
find tests doc -name '*.loft' | while read f; do
  LOFT_PROBE_X=1 LOFT_TIMEOUT=20 ./target/release/loft --interpret "$f" 2>&1 >/dev/null \
    | grep '^PROBE-X' | sed "s|^|$f: |"
done
```

That answers "is this one site or a CLASS?" with a measured producer set rather
than a reading of the code, and a corpus that produces NO hits is itself the
finding — it means nothing in the suite covers the shape, which is usually why
the defect shipped.  Both were true of loft#899's `OpDatabase(db_tp = u16::MAX)`:
exactly one producer, zero corpus coverage.  Remove the probe once the invariant
is enforced at its chokepoint.

**Never set `RUSTFLAGS` against the MAIN target dir.**  Cargo keeps BOTH
flag-generations of every dep in `target/release/deps/` — including two
`libloft_ffi-*.rlib` — and anything that sweeps `deps/` (the cdylib
auto-builds pass `--extern` for every rlib there) then dies on
`colliding StableCrateId values`, while `loft-ffi`'s fingerprint change
invalidates every cached cdylib.  Recovery, in order: `cargo clean --release`
→ full `cargo build --release` → `make rebuild-native-cdylibs` → rebuild the
registry graphics cdylib (`cd ~/.loft/registry/graphics-*/native && cargo
build --release`) → `loft cache prune --all` (the whole-cache sweep; plain
`loft cache prune` keeps the live generation and drops only what this loft
cannot select — see loft#861) → rebuild the wasm rlib (the `html_wasm`
staleness guard checks it against source mtimes).

**Prevent + auto-heal it.**  Run sanitizer/nightly builds through
**`scripts/asan.sh`**, which sets `CARGO_TARGET_DIR=target/asan` so nightly
artifacts never land in the shared `target/` — the pollution simply cannot
happen.  If a stray nightly build already polluted it (E0514 "incompatible
version of rustc" on the native tests), **`find_problems.sh` self-heals**: its
`ffi_toolchain_guard` reads the rustc version embedded in each
`libloft_ffi-*.rlib`, and when it differs from the active `rustc` it deletes the
stale rlib so the next build recompiles it — turning the silent, mtime-immune
E0514 into an automatic rebuild.

Reading the results: CLI-spawning tests fail en masse if the stdlib symlink
is missing (lens artifact, not a finding); confirm any surprising cell
against an `origin/main` control build in a throwaway worktree before
calling it a regression — most DA findings are long-latent, first seen the
day the assert is first *checked*, not the day it was written.
