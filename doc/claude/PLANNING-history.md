<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PLANNING-history.md — shipped, withdrawn and superseded backlog items

The timeline behind [PLANNING.md](PLANNING.md): milestone reasoning, the old release order, and ticket bodies for items that shipped or were withdrawn.  Sections keep their original heading text.

---

### Milestone Reevaluation

The previous plan had 1.0 as a language-stability contract for the interpreter alone,
with the Web IDE deferred indefinitely to "post-1.0".  This reevaluation changes both
milestones and adds the small-steps goal.  The reasoning:

**Why introduce 0.9.0?**
The old plan reached the current state (0.8.1) and declared "L1 is the last blocker
before 1.0", but that understated what "fully featured" actually requires.  Several items
(P1 lambdas, A9 vector CoW, A6 slot pre-pass, A8 string efficiency, A1
parallel completeness) are not optional polish — they close correctness and usability
gaps that a production-ready interpreter must not have.  A 0.9.0 milestone gives these
items a home without inflating the 1.0 scope.

**Why include the IDE in 1.0.0?**
A standalone interpreter 1.0 that is later extended with a breaking IDE integration
produces two separate stability contracts to maintain.  The Web IDE (W1–W6) is already
concretely designed in [WEB_IDE.md](lib_plans/62-web-ide/README.md) and is bounded, testable work.  Deferring
it to "post-1.0" without a milestone risks it never shipping.  In 2026, "fully featured"
for a scripting language includes browser-accessible tooling; shipping a 1.0 without it
would require walking back that claim at 1.1.

**Why include native codegen (Tier N) in 0.8.2?**
`src/generation/` already translates the loft IR to Rust source; the code exists but
does not compile.  The N items are incremental bug fixes — each is Small or Medium effort,
independent of each other and of the other 0.8.2 items — they can be interleaved freely.
Fixing them in 0.8.2 means 0.9.0 ships a binary where `--native` actually works, at no
extra milestone cost.  Deferring them would mean shipping a 0.9.0 that silently generates
uncompilable output.

**Why include REPL (P2) in 0.9.0?**
The Web IDE covers the browser-based interactive use case, but a terminal REPL is
independently useful for development workflows where a browser is not available or
convenient.  P2 is self-contained (new `src/repl.rs`, small changes to `main.rs`)
and depends on L1 (error recovery) which is already in 0.9.0.  Including it rounds
out the "prototype-friendly" goal without affecting the IDE track.

**Why split syntax into 0.8.3?**
Lambda expressions, nested patterns, and field iteration all touch the parser and type
system simultaneously.  Grouping them in a dedicated milestone means syntax decisions can
be reviewed and refined in isolation, before runtime infrastructure work in 0.9.0 begins.
It also keeps each milestone small enough to be fully understood in a single pass.

**The small-steps principle in practice:**
Each milestone is a strict subset of the next.  0.8.2 hardens correctness; 0.8.3 adds new
syntax; 0.8.4 adds HTTP and JSON on top of lambdas; 0.9.0 completes runtime infrastructure
and tooling; 0.8.3 adds R1 + W1 (WASM runtime); 1.0.0 adds W2–W6 (IDE) on top of a complete 0.9.0.  No item moves
forward until the test suite for the previous item is green.  This prevents the "everything
at once" failure mode where half-finished features interact and regressions are hard to pin.

---

### Recommended Implementation Order

Ordered by unblocking impact and the small-steps principle (each item leaves the codebase
in a better state than it found it, with passing tests).

**Released as 0.8.2 (2026-03-24).**

**For 0.8.3 (after 0.8.2 is tagged):**
1. **P3** + **L2** — aggregates and nested patterns; P3 depends on P1 (done in 0.8.2); batch together
2. **P5** — generic functions; independent of P3/L2; land after data.rs changes settle
3. **A10** — field iteration; independent, medium; can land in parallel with P3

**For 0.8.4 (after 0.8.3 is tagged):**
1. **REG.1** — Registry file parser + download/extract; Small, independent of H-items; adds `ureq` + `zip` under `registry` feature
2. **REG.2** — `loft install <name>` CLI extension; Small, depends on REG.1; no parser changes
3. **REG.3** — `loft registry sync`; Small, reuses `ureq` from REG.1; adds `source:` header parsing
4. **REG.4** — `loft registry check` + `list`; Small, pure filesystem + registry parsing; no new deps
5. **H1** — `#json` + `to_json`; Small, no new Rust deps; validates annotation parsing
4. **H2** — JSON primitive stdlib; Small–Medium, new `src/database/json.rs` (~80 lines, no new dep); test each extractor in isolation
5. **H3** — `from_json` scalar codegen; Medium, depends on H1 + H2; verify `Type.from_json` as fn-ref
8. **H4** — HTTP client + `HttpResponse`; Medium, `ureq` already present from REG.1; test against httpbin.org or mock
7. **H5** — nested/array/enum `from_json` + integration tests; Med–High, depends on H3 + H4

**For the 0.8.5 → 0.8.6 → 0.9.0 advertising-readiness sequence (after 0.8.4 is tagged):**

Ordered by (immediate leverage) × (low scope risk) ×
(dependencies-on / unblocking), now split across three releases so
each ship is a standalone tag with its own CHANGELOG entry.  The
split was introduced after today's "can we advertise loft?"
assessment.  These releases predate calendar versioning; a cycle's scope
and ship criteria are its [releases/](releases/README.md) directory.

**Release 0.8.5 — "loft is learnable" (~2 weeks)**

1. **DX.4** — native CI parity.  Promote `tests/native.rs` into the
   fast `cargo nextest run --profile ci` gate.  One CI config
   change + timing-budget check.  Start here — smallest item,
   biggest safety return.  Catches P143/P144/P157/P171/P180-class
   regressions pre-commit instead of mid-release.
2. **SH.1 + SH.2** — TextMate grammar + VS Code extension.  Land
   together since SH.2 consumes SH.1.  Gives newcomers syntax
   highlighting + a "Run loft file" task button.  Needed before
   DX.3 so the tutorial can screenshot real VS Code.
3. **DX.1** — quick-start `examples/` directory at repo root.
   XS effort.  Gathers scattered examples
   (`lib/graphics/examples/*.loft`, brick-buster, moros-editor)
   under a discoverable path with one-paragraph READMEs.  Feeds
   DX.3.
4. **DX.3** — "Learn loft in 30 minutes" walkthrough.  Writing
   work, not coding.  Start from the house-scene canvas demo
   (already working, gold-tested) and narrate forward.  Single
   GitHub Pages page.  Depends on SH.1 / SH.2 / DX.1 for concrete
   screenshots and referential examples.

Ship criterion: one external programmer can install SH.2 from VS
Code Marketplace, open an example, read DX.3 top-to-bottom, and
run the demo within 30 minutes from zero prior exposure.

**Release 0.8.6 — "loft is extensible" (~3 weeks)**

5. **FFI.1 → FFI.2 → FFI.3 → FFI.4** in that order.  Generic
   type marshaller, generic cdylib loader, per-function glue
   elimination, docs.  Each is S to MH.  Landing all four
   together shrinks the boilerplate bar for extracted libraries —
   `lib/graphics/native/` has ~15 hand-written type-punning
   functions today that FFI.1–3 subsume.  Prerequisite for
   0.9.0's PKG.EXTRACT.
6. **PKG.7** — lock file (`loft.lock`).  Cheap, precedes PKG.REG.
7. **PKG.REG** — registry MVP.  Design is complete in
   [PACKAGES.md](PACKAGES.md); implementation is the
   `loft install <name>` fetcher + a `registry.txt` on GitHub
   seeded with 3–5 first-party libraries that stay in-repo
   for 0.8.6 (extraction is 0.9.0's PKG.EXTRACT).

Ship criterion: `loft install <name>` resolves and installs from
the public registry for at least 3 libraries; a third-party
library outside the `loft` repo proves the registry is genuinely
federated.

**Release 0.9.0 — "fully working loft language" (~6 weeks)**

8. **L1** — error recovery after token failures.  Standalone UX
   improvement; RELEASE.md H blocker for 0.9.0.  Also unblocks
   P2.4.
9. **A2** — logger remaining work.  Independent, small-medium;
   can land any time a hand's free.
10. **P2** — REPL.  High effort; RELEASE.md H blocker.  Land
    after L1 (needed for P2.4 error recovery).
11. **W-warn** — developer warnings (Clippy-inspired).  RELEASE.md
    M blocker.
12. **AOT** — auto-compile libraries to native shared libs.
    Medium; design in PLANNING.md.
13. **C52** — stdlib name clash warning + `std::` prefix.
    RELEASE.md M blocker.
14. **C53** — match arms for library enums + bare variant names.
    Medium.
15. **CS.B / CS.C1 / CS.C2 / CS.C3** — compilation cache finish.
    CS.C1 is the biggest (MH — ~2K lines of recursive-enum binary
    serialisation in `data.rs`); budget a focused run.
16. **P117 / P120 / P121 / P124** — verification passes (RELEASE.md
    M blockers).  Each is a hands-on re-run of a fix that landed
    earlier; not reopening the bug.
17. **PKG.EXTRACT** — **last 0.9.0 item**.  Move every `lib/*/`
    into separate GitHub projects (logical bundling allowed — see
    [lib_plans/12-library-extraction](lib_plans/12-library-extraction/README.md)).
    Depends on PKG.REG + DX.4 + FFI.1–4 (all shipped in 0.8.5 /
    0.8.6).  Starting earlier duplicates work; starting later is
    fine.  L effort per bundle; moves happen one family at a time
    ("extract loft-moros, land, extract loft-net, land, …") so a
    failed extract doesn't strand the others.

**Explicitly excluded from the 0.8.5 → 0.9.0 window** to avoid scope creep:
- LSP — stays in 1.0.0 per roadmap.  Months-long on its own.
- HTTP stdlib / `server` / `game_client` libraries — 1.1+ (`WEB_SERVER_LIB.md`, `GAME_CLIENT_LIB.md`).
- Moros hex RPG editor (web version) — [independent lifecycle](ROADMAP.md#demo-applications--independent-lifecycles); does not gate any language tag.

**For 1.0.0 (after 0.9.0 is tagged):**
7. **R1** — workspace split; small change, unblocks all Tier W
8. **W1** — WASM foundation; highest risk in the IDE track; do first
9. **W2** + **W4** — editor shell + multi-file projects; can develop in parallel after W1
10. **W3** + **W5** — symbol navigation + docs browser; can follow independently
11. **W6** — export/import + PWA; closes the loop

---

### A5  Closure capture for lambda expressions
**Sources:** Depends on P1
**Description:** P1 defines anonymous functions without variable capture.  Full closures
require the compiler to identify captured variables, allocate a closure record, and pass
it as a hidden argument to the lambda body.  This is a significant IR and bytecode change.
**Fix path:**

**Phase 1 — Capture analysis** *(completed 0.8.3)*:
Parser detects variables from enclosing scopes referenced inside lambdas.  Emits a clear
error ("lambda captures variable 'name' — closure capture is not yet supported") and
creates a placeholder variable so parsing continues without cascading errors.  Capture
context saved/restored in both parse_lambda and parse_lambda_short.

**Phase 2 — Closure record layout** *(completed 0.8.3)*:
For each capturing lambda, the parser synthesizes `__closure_N` with fields matching
the captured variables.  The record def_nr is stored on Definition.closure_record.
Diagnostic emitted with field count/names/types for test verification.

**Phase 3 — Capture at call site** *(completed 0.8.3)*:
Capture diagnostic updated from generic "not yet supported" to specific "closure body
reads not yet implemented (A5.4)".  Closure record struct (A5.2) is still synthesized.
Actual closure record allocation IR and codegen deferred to A5.4.

**Phase 4 — Closure body reads** *(completed 0.8.3)*:
Hidden `__closure` parameter added on second pass.  Captured variable reads redirect
to `get_field` on the closure record.  Read-only captures work; mutable captures
(`count += x`) pending — codegen panics on self-reference for write targets.

**Phase 5 — Lifetime and cleanup** *(completed 0.8.3)*:
Closure record work variable (Type::Reference with empty deps) is already freed by
the existing OpFreeRef scope-exit logic in get_free_vars.  No new code needed.
Per-field text/reference cleanup inside the record is pending — only matters when
text captures become testable.

**Phase 6 — Mutable capture + text capture** (C1 remaining, tracked as A5.6):
Two remaining restrictions after A5.1–A5.5:

**A5.6a — Mutable capture** *(completed 0.8.3)*:
`capture_detected` passes without source changes.  The mutable-capture path
(`count += x`) routes through `call_to_set_op` → `OpSetInt`, which never hits the
`generate_set` self-reference guard.  The earlier plan for a `SetClosureField` IR
variant was not needed.  Test: `tests/parse_errors.rs::capture_detected`.

**A5.6b.1 — Text capture: garbage DbRef in `CallRef` stack frame** (✓ implemented in `safe` branch):
Text-capturing, text-returning lambdas (e.g. `fn(name: text) -> text { "{prefix} {name}" }`)
produce a garbage `__closure` DbRef at runtime, causing panics such as "Unknown record
49745" or "Store write out of bounds".  Integer-only captures work correctly.

**Root cause — `text_return()` adds captured text variables as spurious work-buffer attributes:**

When the lambda body `"{prefix} {name}"` is compiled, the format-string processor calls
`text_return(ls)` (control.rs:1550) where `ls` contains the text variables referenced in
the format string — including the captured variable `prefix`.

`text_return` iterates over `ls` and for each text variable that is NOT already an
attribute of the lambda, it adds it as a `RefVar(Text)` attribute (a hidden work-buffer
argument) and calls `self.vars.become_argument(v)`.  The guard that skips already-registered
attributes (line 1557: `attr_names.get(n)`) does NOT catch captured variables — at the point
`text_return` runs, `prefix` is not yet registered as an attribute (the hidden `__closure`
parameter is added later in `parse_lambda`).

Result: `prefix` is added as a `RefVar(Text)` attribute of the lambda, giving the lambda
an **extra 12-byte argument slot** that the caller knows nothing about.

**Broken argument layout (with the bug):**

The lambda’s `def_code` processes attributes in order:
1. `name: text` → slot 0, 16 bytes (`size_of::<&str>()`)
2. `prefix: RefVar(Text)` → slot 16, 12 bytes ← spurious, added by `text_return`
3. `__closure: Reference` → slot 28, 12 bytes

Total argument area = 40 bytes; `+4` for return addr → TOS at 44.
Reading `__closure`: `var_pos = 44 - 28 = 16`.  At runtime `stack_pos - 16 = args_base + 16`.

But the caller only pushes 28 bytes (`name` 16 + `__closure` DbRef 12):
- `args_base + 0..16`: `name` ✓
- `args_base + 16..28`: closure DbRef ← callee reads this as `prefix` slot
- `args_base + 28..40`: **nothing** ← callee reads this as `__closure` slot → garbage

**Fix (concrete — `src/parser/control.rs`, `text_return`):**

Add a captured-variable guard immediately after the existing `attr_names` check:

```rust
pub(crate) fn text_return(&mut self, ls: &[u16]) {
    if let Type::Text(cur) = &self.data.definitions[self.context as usize].returned {
        let mut dep = cur.clone();
        for v in ls {
            let n = self.vars.name(*v);
            let tp = self.vars.tp(*v);
            // skip related variables that are already attributes
            if let Some(a) = self.data.def(self.context).attr_names.get(n) {
                if !dep.contains(&(*a as u16)) {
                    dep.push(*a as u16);
                }
                continue;
            }
            // A5.6b.1: skip captured variables — they are read from the closure
            // record at runtime, not passed as hidden work-buffer arguments.
            // Adding them as RefVar(Text) attributes shifts __closure to the wrong
            // argument slot, giving the lambda a garbage DbRef.
            if self.captured_names.iter().any(|(name, _)| name == n) {
                continue;
            }
            if matches!(tp, Type::Text(_)) {
                // ... rest unchanged
```

After this fix, the lambda’s argument layout becomes:
1. `name: text` → slot 0, 16 bytes
2. `__closure: Reference` → slot 16, 12 bytes
Total = 28 bytes, matching what the caller pushes. ✓

**Why `name` is still handled correctly:**

`name` IS already an attribute (it’s the declared parameter), so the `attr_names.get(n)`
check catches it and just adds its attribute index to `dep`.  The format string’s text
dependency tracking for `name` still works — only the spurious insertion of `prefix` as
a work-buffer attribute is suppressed.

**Scope of fix:** Only affects lambdas that (a) return text AND (b) capture text variables
from an outer scope.  No other code path is changed.

**Test scope note:** The existing `closure_capture_text` test (`make_greeter("Hello")("world")`)
crosses function scope — the closure is returned from `make_greeter` and called from
outside.  The `last_closure_alloc` block references variable slots in `make_greeter`'s
frame; calling the returned fn-ref from a different scope would access those stale slots.
This pattern requires A5.6 (0.8.3) — returning a closure alongside its DbRef.

After this fix, add a **same-scope** test that exercises A5.6b.1 directly:
```
prefix = "Hello";
f = fn(name: text) -> text { "{prefix} {name}" };
f("world")  // expected: "Hello world"
```
Same-scope calls use `last_closure_alloc` correctly (consumed at the call site within
the same definition) and do not require the returning-closure architecture.  The
existing `closure_capture_text` test should remain `#[ignore]` until A5.6 (0.8.3).

**A5.6b.2 — `generate_call_ref`: text work buffers not pre-allocated** (✓ implemented):
Text-returning lambdas called via `CallRef` now correctly push the hidden `__work_N`
work-buffer DbRef argument that the callee expects.

**Fix** (`src/parser/control.rs`, `try_fn_ref_call` and zero-param closure path):
- Both passes call `work_text()` for each dep in the return type’s deps list.
- `work_text()` adds each variable to `work_texts`; `parse_code` (expressions.rs:79)
  inserts `v_set(wv, Text(""))` so the Zone 2 slot allocator fires.
- In pass 2, a `v_block([OpCreateStack(Var(wv))], Type::Reference(...))` is injected
  between the visible args and the closure arg — producing the required 12-byte DbRef.
- `generate_call_ref` simplified to a single `for arg in args { generate(...) }` loop;
  the blocks produce the correct sizes automatically.

**Verified:** `closure_capture_text_return` passes; all other closure tests unaffected.

**A5.6c — Mutable capture write-back: void-return lambdas** (✓ implemented in `safe` branch):
A void-return capturing lambda (`fn(x: integer) { count += x; }`) updates the
`count` field inside the closure record, but the outer `count` variable (in the
caller’s stack frame) is never updated.  After `f(10); f(32)`, the outer `count`
remains 0.

The lambda’s IR correctly modifies the closure record field (A5.6a is done — the
`capture_detected` test proves mutable field writes work inside the lambda body).
The missing step is the write-back from closure record to outer variable after each
`CallRef` returns.

**Fix path (concrete — parser `control.rs`, call site generation):**

At the call site where `Value::CallRef(v_nr, args)` is built (control.rs:2000),
after constructing `converted`, emit write-back IR for each mutable captured variable:

```rust
// A5.6c: after CallRef to a closure, write captured mutable fields back to
// the outer variables so the caller sees the updated values.
if let Some(&closure_w) = self.closure_vars.get(&v_nr) {
    // closure_vars maps fn-ref var → closure work var (the __clos DbRef in scope).
    let closure_rec = self.data.def(d_nr).closure_record;
    if closure_rec != u32::MAX {
        for aid in 0..self.data.attributes(closure_rec) {
            let cap_name = self.data.attr_name(closure_rec, aid);
            let outer_v = self.vars.var(&cap_name);
            if outer_v != u16::MAX {
                // Emit: outer_var = get_field(__clos, aid)
                write_back_ops.push(self.get_field(closure_rec, aid, 0,
                    Value::Var(closure_w)));
                write_back_ops.push(Value::Set(outer_v, /* get_field result */));
            }
        }
    }
}
```

The exact IR construction follows the existing `set_field_no_check` / `get_field`
helpers.  The write-back IR is emitted as statements immediately after the
`Value::CallRef(...)` expression in the enclosing block.

**Prerequisite:** `closure_vars` must be populated for the fn-ref variable `v_nr`.
Currently `closure_vars.insert` fires only when `last_closure_work_var != u16::MAX`,
but `last_closure_work_var` is never set.  Fix: in `emit_lambda_code` (vectors.rs),
after creating `w` (the `__clos` work var), set `self.last_closure_work_var = w`.
Then in `parse_assign` (expressions.rs:710–712), `closure_vars.insert(var_nr, w)`
fires correctly.

**Test:** Remove `#[ignore]` from `tests/issues.rs::p1_1_lambda_void_body` and
update the ignore reason from the old "A5, 1.1+" text to "A5.6c" once the fix is
implemented.

**Effort:** A5.6b.1 Medium · A5.6b.2 Small · A5.6c Medium
**Target:** 0.8.3 (A5.6b.1, A5.6b.2, A5.6c, A5.6d, A5.6e, A5.6f completed; full cross-scope A5.6 also 0.8.3)

---

**A5.6 — Full closure semantics: 16-byte fn-ref + chained-call parser** *(completed 0.8.3)*:
After A5.6b.1, A5.6b.2, and A5.6c are implemented, the last open item for
`closure_capture_text` is the **cross-scope** pattern: a capturing lambda returned
from a function and then called from outside.  Two distinct problems remain:

---

#### The opcode problem: `Type::Function` is 4 bytes — no room for closure DbRef

`size(Type::Function, _)` returns 4 (same arm as `Type::Integer` in
`src/variables/mod.rs:995`).  `fn_call_ref` in `state/mod.rs:221` reads exactly 4
bytes: `*get_var::<i32>(fn_var)` = the d_nr.

A closure DbRef is 12 bytes (store_nr + rec + pos — same layout as every other
`DbRef`).  When `make_greeter` returns the inner lambda as its return value, only
the 4-byte d_nr lands on the caller's stack; the 12-byte DbRef for the closure
record has nowhere to go and is lost.  The closure record itself stays alive in the
store (it was heap-allocated via `OpDatabase`), but no pointer to it survives the
return — so the lambda body's `__closure` parameter can never be populated.

**Fix — 16-byte fn-ref slot:**

```
offset 0..4:  d_nr (i32)        — function definition index
offset 4..8:  store_nr (i32) ─┐
offset 8..12: rec (i32)        ├─ closure DbRef (12 bytes; all-zero = no closure)
offset 12..16: pos (i32)      ─┘
```

`size(Type::Function, _)` → 16 (move `Type::Function` out of the `4`-byte arm in
`src/variables/mod.rs:995`; add a new arm `Type::Function(_, _) => 4 + size_of::<DbRef>() as u16`).

**Emitting the fn-ref value (vectors.rs `emit_lambda_code`):**

Non-capturing lambdas: `*code = Value::Int(d_nr as i32)` unchanged — `OpPutInt`
writes d_nr to bytes 0..4; bytes 4..16 stay zero (zeroed by `OpReserveFrame`).

Capturing lambdas: emit a `v_block` that:
1. Runs the existing `alloc_steps` to allocate and fill the closure record into work
   var `w` (type `Type::Reference`).
2. Emits `v_set(fn_ref_var, Value::Int(d_nr as i32))` — writes d_nr to bytes 0..4
   of the new 16-byte work var `fn_ref_var` (type `Type::Function`).
3. Emits `cl("OpStoreClosure", [Var(fn_ref_var), Var(w)])` — a new opcode that
   copies the 12-byte DbRef from `w`'s stack slot into `fn_ref_var`'s bytes 4..16.
4. Yields `Value::Var(fn_ref_var)`.

Then **drop** `self.last_closure_alloc` — the closure is now embedded in the fn-ref
value and no longer needs to be injected separately at call sites.

**New opcode: `OpStoreClosure(fn_ref_var: u16, closure_var: u16)`** (fill.rs):
Reads the absolute stack position of `fn_ref_var` and `closure_var`; copies 12 bytes
from `closure_var`'s slot to `fn_ref_var`'s slot at byte offset 4.  No stack push/pop.

**Calling through the 16-byte fn-ref (state/mod.rs `fn_call_ref`):**

```rust
pub fn fn_call_ref(&mut self, fn_var: u16, arg_size: u16) {
    let d_nr = *self.get_var::<i32>(fn_var) as usize;
    // Read closure DbRef from bytes 4..16 of the 16-byte fn-ref slot.
    // The slot start is at (stack_pos - fn_var); byte 4 is one i32 further.
    let store_nr = *self.get_var::<i32>(fn_var - 4);   // fn_var_abs + 4
    let has_closure = store_nr != -1;  // -1 is the null sentinel for store_nr
    let total = arg_size + if has_closure { size_of::<DbRef>() as u16 } else { 0 };
    if has_closure {
        let rec = *self.get_var::<i32>(fn_var - 8);
        let pos = *self.get_var::<i32>(fn_var - 12);
        // Push DbRef (12 bytes) onto the stack as __closure argument
        self.push_stack(store_nr);
        self.push_stack(rec);
        self.push_stack(pos);
    }
    let code_pos = self.fn_positions[d_nr] as i32;
    self.fn_call(d_nr as u32, total, code_pos);
}
```

Note: the fn-ref variable's absolute position is `stack_pos - fn_var`.  Because the
stack grows upward, `fn_var_abs + 4` is referenced as `stack_pos - (fn_var - 4)`.
Verify the offset arithmetic matches `get_var`'s addressing in the implementation.

**Call-site codegen (parser/control.rs `try_fn_ref_call`, zero-param path):**

Remove the `last_closure_alloc.take()` and `closure_vars.get(&v_nr)` injection.
The closure is now pushed by `fn_call_ref` at runtime from the embedded DbRef —
no parser-level injection needed.  `generate_call_ref` is unchanged (already
simplified by A5.6b.2): all args in `converted` are visible params and work bufs.

**`generate_var` for `Type::Function` (codegen.rs line 1210):**

Change from `OpVarInt` (4 bytes) to a new `OpVarFnRef` (16 bytes).  This is the
read side of the 16-byte push: push all 16 bytes of the fn-ref slot onto the stack
so fn-ref values can be passed, returned, and assigned.

`OpVarFnRef` implementation (fill.rs): read `pos: u16` from bytecode; push 16 bytes
starting at `stack_pos - pos` onto the stack (similar to `OpVarRef` which pushes 12
bytes, but 4 bytes larger).

**`OpPutInt` for `Type::Function` (codegen.rs lines 1521, 1210):**

Assignment `v_set(fn_ref_var, Value::Int(d_nr))` still uses `OpPutInt` — it writes
4 bytes to the variable's slot at offset 0 (the d_nr).  Bytes 4..16 are untouched
(already zeroed by `OpReserveFrame` or set by a preceding `OpStoreClosure`).
So `OpPutInt` at call sites for fn-ref assignment is **correct as-is** when the
RHS is `Value::Int(d_nr)`.

For the case where a fn-ref is copied variable-to-variable (`f = g` where both are
`Type::Function`), use `OpVarFnRef` to push 16 bytes then `OpPutFnRef` (new) to
store them — OR reuse `OpPutRef`-style logic for 16 bytes.

---

#### The parser problem: `expr(args)` chained calls not handled

`parse_part` (operators.rs:277) loops on `.` and `[` only.  After
`make_greeter("Hello")` returns `Type::Function`, the `("world")` token is not
consumed as a chained call — it is parsed as a separate parenthesised expression.

**Fix (operators.rs `parse_part`):**

Extend the loop to handle `(` when `t` is `Type::Function`:

```rust
while self.lexer.peek_token(".")
    || self.lexer.peek_token("[")
    || (self.lexer.peek_token("(") && matches!(t, Type::Function(_, _)))
{
    if self.lexer.has_token("(") {
        if let Type::Function(param_types, ret_type) = t.clone() {
            // Store fn-ref expression in a work var so CallRef can name it.
            let fn_work = self.create_unique("__fnref_tmp", &t);
            if !self.first_pass {
                let orig = std::mem::replace(code, Value::Var(fn_work));
                // emit: fn_work = <fn_ref_expression>
                // (parse_code will insert the assignment via inline-ref logic)
                // Actually: wrap in a block: { fn_work = orig; fn_work }
                // ... see implementation note below
            }
            t = self.call_fn_work_var(fn_work, param_types, *ret_type);
        }
    } else { /* existing . and [ handlers */ }
}
```

`call_fn_work_var(work_var, param_types, ret_type)`: parse argument list, emit
`Value::CallRef(work_var, args)`, return `ret_type`.  Because the closure DbRef is
embedded in the 16-byte fn-ref slot of `work_var`, `fn_call_ref` pushes it at
runtime — no explicit closure injection needed.

**Implementation note:** Storing `orig` into `fn_work` before the call requires
either:
(a) Wrapping in a `v_block([v_set(fn_work, orig), Value::CallRef(fn_work, args)], ret_type)`, or
(b) Using the inline-ref temp pattern from `parse_part`'s existing chained-ref logic
    (lines 342–361) — mark `fn_work` as an inline-ref temp; `parse_code` inserts the
    null-init.

Option (a) is simpler for the first implementation.

---

#### Remaining deferred sub-items (post-0.8.3)

After the 16-byte fn-ref lands, these edge cases remain deferred:

1. **Lambda re-definition:** if `f = fn(x) { ... }` is followed by `f = fn(x) { ... }`,
   the old closure record (bytes 4..16 of the old fn-ref) must be freed before overwriting.
   `get_free_vars` must emit `OpFreeRef` reading from the fn-ref slot before the
   `OpPutInt`/`OpStoreClosure` of the new lambda.

2. **Lambdas in collections / struct fields:** `closure_vars` is irrelevant with 16-byte
   fn-refs; the closure DbRef travels with the fn-ref value.  But for collections,
   `OpVarFnRef` / store operations need to work correctly for the 16-byte size.

3. **Concurrent sharing:** two parallel workers calling the same closure simultaneously
   share the closure record.  Requires per-call copy or locking — deferred to the
   parallel safety audit.

---

**Implementation steps (independently testable):**

**A5.6-1 — Widen `Type::Function` to 16 bytes**

- `src/variables/mod.rs`: change the `Type::Function(_, _)` arm in `size()` from `4` to
  `4 + size_of::<DbRef>() as u16` (= 16).
- `src/state/codegen.rs` (`generate_var`): change the `Type::Function` arm from emitting
  `OpVarInt` (4 bytes) to a new `OpVarFnRef` (16 bytes).
- `src/fill.rs`: add `op_var_fn_ref` — reads `pos: u16` from bytecode; pushes 16 bytes
  starting at `stack_pos - pos` onto the stack (same as `op_var_ref` but 4 bytes larger).

**Pass:** all existing non-capturing lambda tests pass; fn-ref variable occupies 16 bytes.

---

**A5.6-2 — `OpStoreClosure` + embed closure DbRef in fn-ref**

- `src/fill.rs`: add `op_store_closure` — reads `fn_ref_pos: u16` and `closure_pos: u16`
  from bytecode; copies 12 bytes from `stack_pos - closure_pos` to
  `(stack_pos - fn_ref_pos) + 4`. No stack push/pop.
- `src/parser/vectors.rs` (`emit_lambda_code`): for capturing lambdas, after the existing
  `alloc_steps` (which produce the closure record in work var `w`), emit:
  1. `v_set(fn_ref_var, Value::Int(d_nr as i32))` — writes d_nr into bytes 0..4.
  2. `cl("OpStoreClosure", &[Value::Var(fn_ref_var), Value::Var(w)])` — embeds the
     12-byte DbRef from `w` into fn-ref bytes 4..16.
  Store result in `fn_ref_var` (a new Zone-1 work variable of type `Type::Function`).
  **Drop** `self.last_closure_alloc` — the closure is now embedded in the fn-ref value and
  no longer injected at call sites.

**Pass:** a capturing lambda assigned to a local variable carries its closure DbRef in the
fn-ref slot; `LOFT_LOG=ref_debug` shows the DbRef bytes 4..16 non-zero.

---

**A5.6-3 — `fn_call_ref` reads closure from fn-ref bytes 4..16**

- `src/state/mod.rs` (`fn_call_ref`): after reading `d_nr` from `*get_var::<i32>(fn_var)`,
  read `store_nr` from `*get_var::<i32>(fn_var - 4)`.  If `store_nr != -1` (non-null),
  read `rec` and `pos` and push the 12-byte DbRef onto the stack as the `__closure`
  argument.  Adjust `total_arg_size` accordingly.
  ```rust
  let store_nr = *self.get_var::<i32>(fn_var - 4);
  let has_closure = store_nr != -1;
  if has_closure {
      let rec = *self.get_var::<i32>(fn_var - 8);
      let pos = *self.get_var::<i32>(fn_var - 12);
      self.push_stack(store_nr);
      self.push_stack(rec);
      self.push_stack(pos);
  }
  ```
  (Offset arithmetic: fn-ref occupies bytes `[fn_var_abs .. fn_var_abs+16]`; d_nr is at
  offset 0, store_nr at +4, rec at +8, pos at +12.  `get_var::<i32>(fn_var)` reads from
  `stack_pos - fn_var` = `fn_var_abs`; `fn_var - 4` reads `fn_var_abs + 4`, etc.)
- `src/parser/control.rs` (`try_fn_ref_call`, both paths): remove
  `last_closure_alloc.take()` injection and `closure_vars` lookup — the closure is now
  pushed by `fn_call_ref` at runtime from the embedded DbRef.

**Pass:** `closure_capture_text_return` and `closure_capture_text_integer_return` pass
without the closure being injected at the call site.

---

**A5.6-4 — `parse_part`: chained `(...)` call on `Type::Function`**

- `src/parser/operators.rs` (`parse_part`): extend the postfix loop:
  ```rust
  while self.lexer.peek_token(".")
      || self.lexer.peek_token("[")
      || (self.lexer.peek_token("(") && matches!(t, Type::Function(_, _)))
  {
      if self.lexer.has_token("(") {
          if let Type::Function(param_types, ret_type) = t.clone() {
              // Store fn-ref in work var so CallRef can name it.
              let fn_work = self.create_unique("__fnref_tmp", &t);
              if !self.first_pass {
                  let orig = std::mem::replace(code, Value::Var(fn_work));
                  *code = Value::Block(Box::new(Block {
                      ops: vec![Value::Set(fn_work, Box::new(orig))],
                      result: Box::new(Value::Var(fn_work)),
                      ..Default::default()
                  }));
              }
              t = self.call_fn_work_var(fn_work, param_types, *ret_type);
          }
      } else { /* existing . and [ handlers */ }
  }
  ```
  `call_fn_work_var`: parse argument list inside `(...)`, emit
  `Value::CallRef(fn_work, args)`, return `ret_type`.

**Pass:** `make_greeter("Hello")("world")` parses and produces "Hello world".

---

**A5.6-5 — Un-ignore `closure_capture_text`; full test pass**

- `tests/expressions.rs`: remove `#[ignore]` from `closure_capture_text`.
- `tests/wrap.rs` (WASM_SKIP): keep `19-threading.loft` skipped (that is W1.18, not A5.6).

**Pass:** `cargo test --test expressions closure_capture_text` succeeds; full `make test`
green.

---

**Files changed:**

| File | Change |
|------|--------|
| `src/variables/mod.rs` | `size(Type::Function)` → 16 |
| `src/fill.rs` | Add `op_store_closure`, `op_var_fn_ref` |
| `src/state/mod.rs` | `fn_call_ref`: read closure from bytes 4..16, push if present |
| `src/state/codegen.rs` | `generate_var`: `OpVarFnRef` for `Type::Function` |
| `src/parser/vectors.rs` | `emit_lambda_code`: emit `OpStoreClosure`; drop `last_closure_alloc` |
| `src/parser/control.rs` | Remove closure injection from `try_fn_ref_call` (both paths) |
| `src/parser/operators.rs` | `parse_part`: handle chained `(...)` on `Type::Function` |
| `tests/expressions.rs` | Remove `#[ignore]` from `closure_capture_text` |

**Guards and debugging:**
- `fn_call_ref`: `debug_assert!(d_nr < self.fn_positions.len())` before indexing.
- `OpStoreClosure`: `debug_assert!` that the fn_ref_var slot has 16 bytes allocated.
- Strip internal text deps from the public fn-type: in `emit_lambda_code` (vectors.rs:667),
  replace `Text(deps)` with `Text(vec![])` in the return type of the constructed
  `Type::Function`.  Internal dependency tracking is for the lambda body, not the interface.
- Add `LOFT_LOG=closure` mode that prints fn-ref slot contents (d_nr + DbRef) at call sites
  in `fn_call_ref` — catches misaligned reads immediately.

**Effort:** High (8 files, 2 new opcodes, 5 independently testable steps)
**Depends on:** A5.6b.1 ✓, A5.6b.2 ✓, A5.6c ✓
**Target:** 0.8.3

---


### L9  Format specifier / type mismatch — escalate to compile error
**Status: completed**
Changed `Level::Warning` → `Level::Error` in `append_data()` for radix specifiers on
text/boolean and zero-padding on text.  Tests updated in `38-parse-warnings.loft`.
CAVEATS.md C14 closed.

---

### L10  `while` loop syntax sugar
**Status: completed**
Added `while` keyword to the lexer and `parse_while()` in `expressions.rs`.
Desugars to `v_loop([if !cond { break }, body])`.  Tests in `46-caveats.loft`.
CAVEATS.md C11 closed.

---

### L11  Digit-group separators (`_`) in number literals + thousands-position lint
**Status: completed** — `src/lexer.rs::get_number` accepts `_` between digits (stripped before
parse) across all bases + float/exponent parts; misplaced `_` (trailing / doubled / next to a
prefix·`.`·`e`) is one `Level::Error`; decimal groups that aren't thousands (leftmost 1-3, rest
exactly 3) emit a `Level::Warning` but still accept. Tests: `01-integers.loft` (accept),
`38-parse-warnings.loft` (warn), `102-expected-errors.loft` (errors). Both backends (shared lexer).
**Sources:** user request 2026-06-25 (verified absent — the lexer rejected `1_000_000`).
**Description:** Let `_` separate digit groups in numeric literals so big numbers read
easily — `1_000_000`, `0xFF_FF`, `1_000.000_5` — and **warn** (accept, do not reject) when
the separators are *not* on standard thousands boundaries (`1_00_000`, `10_0`). The warning
nudges toward conventional 3-digit grouping without forcing it.

Today `_` is rejected: `get_number()` (`src/lexer.rs:977`) accepts only digits / `b` / `o`
/ hex chars, and `_` is an identifier character (`src/lexer.rs:966`), so `1_000_000` lexes
as `1` followed by the identifier `_000_000`, and the parser reports "Expect token `;`".

**Design (XS–S):**
1. **Lexer (`get_number`)** — accept `_` *only between two digits*; strip it from the
   collected string before `parse`. Reject (Error) a leading `_` (`_1`, which is an
   identifier), a trailing `_` (`1_`), a doubled `__`, or a `_` adjacent to the radix
   prefix / `.` / `e` (`0x_F`, `1._5`, `1e_3`). The same function scans the integer,
   fraction, and exponent parts and every base (dec/hex/bin/oct), so one change covers all.
2. **Thousands-position lint** — while scanning the integer part, record the digit count
   between separators. If any *interior* group is not exactly 3 digits (the most-significant
   group may be 1–3), emit a `Level::Warning` ("digit separators not on thousands
   boundaries") and still accept the value. Decimal only at first — hex/bin group by 4/8,
   not 3, so either skip the lint there or add nibble/byte grouping as a follow-up.
3. **Tests** (`tests/` parse-warnings suite): `1_000_000` parses to `1000000`; warn on
   `1_00_000` / `10_0`; error on `_1` / `1_` / `1__0` / `0x_F`. The lexer is shared, so
   interp and native agree by construction (no separate codegen).

**Files:** `src/lexer.rs` (`get_number` + the lint), a regression test, and a one-line note
in `LOFT.md` § number literals once shipped. No parser/codegen change — the value is already
a plain integer by the time it leaves the lexer.

---

### TR1  Stack trace introspection
**Sources:** STACKTRACE.md
**Description:** `stack_trace()` stdlib function returning `vector<StackFrame>`, where each frame exposes function name, source file, and line number. Full design in [STACKTRACE.md](STACKTRACE.md). Prerequisite for CO1 (coroutines use the frame vector for yield/resume).

- **TR1.1** — Shadow call-frame vector *(completed 0.8.3)*: CallFrame struct and call_stack on State; OpCall encodes d_nr and args_size; fn_call pushes, fn_return pops.
- **TR1.2** — Type declarations *(completed 0.8.3)*: ArgValue, ArgInfo, VarInfo, StackFrame in `default/04_stacktrace.loft`.
- **TR1.3** — Materialisation *(completed 0.8.3)*: `stack_trace()` native function builds `vector<StackFrame>` from snapshot. Tests blocked by Problem #85.
- **TR1.4** — Call-site line numbers *(completed 0.8.3)*: CallFrame stores source line directly; resolved in fn_call. Tests blocked by Problem #85.

**Effort:** Medium
**Completed:** 0.8.3 (phases 1–4; phases 5–6 deferred to 1.1+)

---


### S25  CO1.3d — coroutine text serialisation
**Sources:** SAFE.md § P2-R1/R2/R3, CAVEATS.md C23/C24, COROUTINE.md § CO1.3d/SC-CO-1/SC-CO-8/SC-CO-10

#### S25.1 — Text arg serialisation at coroutine create *(completed 0.8.3)*

`serialise_text_args` in `State` walks each attribute slot in `stack_bytes`
(only arg-sized `Str` slots, 16 bytes each), clones dynamic strings into
owned `String` objects stored in `text_owned`, and patches the `Str` pointer
in `stack_bytes` to point to the owned buffer.  Called from `coroutine_create`.
This fixed C23 (use-after-free on first resume for generators with `text` args).

#### S25.2 — Pointer-patch on resume + String drain on exhaustion *(completed 0.8.3)*

`coroutine_next` re-patches text-arg `Str` pointers from `text_owned` into the
cloned `bytes` before copying them to the live stack (M6-b).
`coroutine_return` calls `frame.text_owned.clear()` before `stack_bytes.clear()`,
which drops the owned String objects via RAII (M7-a).

#### S25.3 — Text local leak on early `break` from a generator loop *(completed 0.8.3 — C24)*

**Severity:** High — memory leak affects every generator with at least one text
local variable that is consumed via `break` (not iterated to exhaustion).

**Precise diagnosis (2026-03-29):**

Text local variables (e.g. `word = "hello"` inside a generator body) are `String`
objects (24 bytes: ptr+len+cap) held on the generator's live stack.  At
`coroutine_yield`, the raw bytes `[base..value_start]` are bitwise-copied to
`frame.stack_bytes`.  The copy is safe across yield/resume cycles because:

- String heap buffers are not freed while the generator is suspended (no Rust
  destructor runs on the abandoned live-stack copy).
- On resume, `coroutine_next` raw-copies `frame.stack_bytes` back to the live
  stack — the same heap pointer is restored and remains valid.
- At exhaustion via `coroutine_return`, `OpFreeText` has already been emitted
  before `OpCoroutineReturn` by `scopes::check`.  The live-stack String is freed
  by `OpFreeText`; `frame.stack_bytes` then contains stale bytes pointing to an
  already-freed allocation, which `frame.stack_bytes.clear()` discards safely.

**The single remaining leak path** — generator is `Suspended` (has yielded), then
the consumer breaks from the for-loop before exhaustion:

1. `OpFreeCoroutine` fires → `free_coroutine(idx)`
2. `free_coroutine` sets `self.coroutines[idx] = None`
3. This drops `Box<CoroutineFrame>`, which drops `stack_bytes: Vec<u8>`
4. `Vec<u8>::drop` frees the raw byte buffer but does NOT call `String::drop` on
   embedded String structs — their heap allocations (`"hello"`, etc.) are leaked.

**Complication — uninitialized text local slots:**

Zone 2 variables (including text locals) are pre-claimed at function entry via
`OpReserveFrame` (which only bumps `stack_pos`, does not zero memory).  If a
text local is assigned AFTER the yield point, its slot in `frame.stack_bytes`
contains garbage bytes from the store.  Calling `drop_in_place::<String>` on
garbage bytes is undefined behaviour.

**Fix design (S25.3):**

Step 1 — **Zero Zone 2 at generator startup** (in `coroutine_next`, `Created` status only).
After copying `frame.stack_bytes` (args+return-slot only) to the live stack,
compute the Zone 2 region extent from `def.variables` and zero those store bytes:

```rust
// After: std::ptr::copy_nonoverlapping(bytes, dst, bytes.len())
// New:   zero the Zone-2 region so uninitialised text locals start with null ptr.
let zone2_abs = self.stack_cur.pos + stack_base + bytes.len() as u32;
let zone2_size = Self::generator_zone2_size(d_nr, self.data_ptr);
if zone2_size > 0 {
    let store = self.database.store_mut(&self.stack_cur);
    let ptr = store.addr_mut::<u8>(self.stack_cur.rec, zone2_abs);
    unsafe { std::ptr::write_bytes(ptr, 0, zone2_size); }
}
```

```rust
/// Compute the total Zone-2 variable extent for generator function `d_nr`.
/// Returns bytes above the args+return-slot region (= `args_size + 4`).
fn generator_zone2_size(d_nr: u32, data_ptr: *const Data) -> usize {
    if data_ptr.is_null() { return 0; }
    let data = unsafe { &*data_ptr };
    let def = data.definitions.get(d_nr as usize)?;
    let vars = &def.variables;
    let mut top: u16 = 0;
    for v in 0..vars.count() {
        if vars.is_argument(v) { continue; }
        let slot = vars.stack(v);
        if slot == u16::MAX { continue; }
        let sz = vars.size(v, &Context::Variable);
        top = top.max(slot.saturating_add(sz));
    }
    top as usize
}
```

Step 2 — **Drop text locals in `free_coroutine`** before setting the slot to `None`:

```rust
pub fn free_coroutine(&mut self, idx: usize) {
    if idx > 0 && idx < self.coroutines.len() {
        // C24 / S25.3: drop text-local String objects from a suspended frame.
        if let Some(frame) = self.coroutines[idx].as_mut() {
            if frame.status == CoroutineStatus::Suspended {
                let d_nr = frame.d_nr;
                let data_ptr = self.data_ptr; // raw ptr — no borrow conflict
                Self::drop_text_locals_in_bytes(d_nr, &mut frame.stack_bytes, data_ptr);
            }
        }
        self.coroutines[idx] = None;
    }
}

/// Drop String objects embedded at text-local slots in `bytes`.
/// Guards against uninitialized slots via null-ptr check (Step 1 zeroed them).
fn drop_text_locals_in_bytes(d_nr: u32, bytes: &mut Vec<u8>, data_ptr: *const Data) {
    if data_ptr.is_null() { return; }
    let data = unsafe { &*data_ptr };
    let Some(def) = data.definitions.get(d_nr as usize) else { return };
    let vars = &def.variables;
    for v in 0..vars.count() {
        if vars.is_argument(v) { continue; }
        if !matches!(vars.tp(v), Type::Text(_)) { continue; }
        let slot = vars.stack(v);
        if slot == u16::MAX { continue; }
        let off = slot as usize;
        if off + std::mem::size_of::<String>() > bytes.len() { continue; }
        // Check the String's ptr field (first word on 64-bit).
        // Null means uninitialized (zeroed in Step 1); skip.
        let ptr_val: usize = unsafe {
            std::ptr::read_unaligned(bytes.as_ptr().add(off).cast::<usize>())
        };
        if ptr_val == 0 { continue; }
        // Drop in place and zero to prevent any future double-drop.
        unsafe { std::ptr::drop_in_place(bytes.as_mut_ptr().add(off).cast::<String>()); }
        unsafe { std::ptr::write_bytes(bytes.as_mut_ptr().add(off), 0, std::mem::size_of::<String>()); }
    }
}
```

Step 3 — **Fix misleading comment** in `coroutine_yield` (line ~723):
Remove the sentence "CO1.3d is now implemented — text locals are serialised to
frame.text_owned above".  Replace with accurate text: "The raw-bytes copy of text
locals in `stack_bytes` is safe across yield/resume cycles because no external code
frees the String heap buffers while suspended.  The early-break leak is fixed by
`free_coroutine` (S25.3)."

**Files changed:** `src/state/mod.rs` (3 locations: `free_coroutine`, `coroutine_next`,
new `generator_zone2_size` + `drop_text_locals_in_bytes` helpers)

**Tests to add** (`tests/expressions.rs`):
- `coroutine_text_local_early_break` — generator has text local, loop breaks after
  first yield.  Run under Miri to verify no leak.
- `coroutine_text_local_declared_after_first_yield` — text local declared after
  the first yield; no panic at break.  Verifies the null-ptr guard.

**Atomicity:** Steps 1 and 2 must land in the same commit.  If Step 1 lands without
Step 2, Zone 2 is zeroed but Strings are still leaked.  If Step 2 lands without
Step 1, `drop_in_place` may fire on garbage bytes (UB).

**Effort:** Small (1–2 hours)
**Target:** 0.8.3

---

### S32  Fix slot conflict in `20-binary.loft` (`rv` / `_read_34`) — **Done**
**Sources:** CAVEATS.md C28
**Status:** Fixed — `20-binary.loft` runs unconditionally in `tests/wrap.rs::binary` and `tests/native.rs`; no longer in `ignored_scripts()` or `SCRIPTS_NATIVE_SKIP`, no `#[ignore]`.

**Tests:** `binary` and `loft_suite` (wrap) pass; `20-binary.loft` passes in native mode.

---

### S34  Interpreter: `20-binary.loft` `pos >= TOS` assertion at codegen.rs:751 — **Done**
**Sources:** `tests/scripts/20-binary.loft`, `src/state/codegen.rs:751`
**Status:** Fixed (0.8.3) — `skip_free` mechanism in `src/state/codegen.rs` and
`src/variables/validate.rs` aliases the inner `_read_*` variable to its TOS slot,
suppresses the double-free, and skips the conflict check.  `wrap::binary` passes
unconditionally; `20-binary.loft` removed from `ignored_scripts()`.
Side effect: exposed a pre-existing native codegen bug (S35) for the same pattern.

---

### ~~H2  JSON primitive extraction stdlib~~ — WITHDRAWN

**Status:** Withdrawn 2026-04 — superseded by [P54 § JsonValue
enum](QUALITY-history.md#active-sprint--p54-jsonvalue-enum).  The
text-based `json_text/int/long/float/bool/items/nested` surface
this section designed has been replaced wholesale by the typed
`JsonValue` tree (`json_parse(text) -> JsonValue` plus six
variants and dedicated read/write helpers — see
[STDLIB.md § JSON](STDLIB.md)).  The original design is preserved
below as a historical record; do not implement.

**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B; CODE.md § Dependencies
**Description:** Add a new stdlib module `default/06_web.loft` with JSON field-extraction
functions.  Functions extract a single typed value from a JSON object body supplied as
a `text` string.  No `serde_json` dependency — the existing parsing primitives in
`src/database/structures.rs` are sufficient; a new `src/database/json.rs` module adds
schema-free navigation on top.
**Fix path:**

**Step 1 — Cargo dependency** (`Cargo.toml`):
Add only `ureq` (used in H4) under a new `http` optional feature.  No `serde_json`.
```toml
[features]
http = ["ureq"]

[dependencies]
ureq = { version = "2", optional = true }
```

**Step 2 — `src/database/json.rs`** (new file, ~80 lines, no new dependency):
Add as a submodule of `src/database/`.  Provides three `pub(crate)` building blocks:

```rust
// Find `key` in a top-level JSON object; return raw value slice (unallocated).
pub(crate) fn json_get_raw<'a>(text: &'a str, key: &str) -> Option<&'a str>

// Return raw JSON text for each element of a top-level JSON array.
pub(crate) fn json_array_items(text: &str) -> Vec<String>

// Parse a raw value slice into a Rust primitive (loft null sentinels on failure):
pub(crate) fn as_text(raw: &str) -> String   // strips quotes + handles \n \t \\
pub(crate) fn as_int(raw: &str) -> i32       // i32::MIN on failure
pub(crate) fn as_long(raw: &str) -> i64      // i64::MIN on failure
pub(crate) fn as_float(raw: &str) -> f64     // f64::NAN on failure
pub(crate) fn as_bool(raw: &str) -> bool     // false on failure
```

Internally `json.rs` uses its own `skip_ws`, `skip_value`, and `extract_string` helpers
(~50 lines combined).  These mirror the primitives in `structures.rs` but operate
schema-free: no `Stores`, no `DbRef`, no type lookup.  The byte-scanning logic is
identical in style to the existing `match_text` / `skip_float` functions.

*Design note:* The primitives in `structures.rs` (`match_text`, `match_integer`, etc.)
are `fn` (module-private) because they are only called by `parsing()` within the same
module.  Rather than widening their visibility, `json.rs` keeps its own small copies
to preserve the clean boundary between schema-driven and schema-free parsing.

**Step 3 — Loft declarations** (`default/06_web.loft`):
```loft
// Extract primitive values from a JSON object body.
// Returns zero/empty/null-sentinel if the key is absent or type does not match.
pub fn json_text(body: text, key: text) -> text;
pub fn json_int(body: text, key: text) -> integer;
pub fn json_long(body: text, key: text) -> long;
pub fn json_float(body: text, key: text) -> float;
pub fn json_bool(body: text, key: text) -> boolean;

// Split a JSON array body into element bodies (each element as raw JSON text).
pub fn json_items(array_body: text) -> vector<text>;

// Extract a named field as raw JSON text (object, array, or primitive).
// Use for nested structs and array fields: json_nested(body, "field").
pub fn json_nested(body: text, key: text) -> text;
```

**Step 4 — Rust implementation** (new `src/native_http.rs`, registered in `src/native.rs`):
Each native function calls `json::json_get_raw` then the appropriate `as_*` converter.
All functions return the loft null sentinel (or empty string) on any error — never panic.
- `json_text`: `json_get_raw(body, key).map(as_text).unwrap_or_default()`
- `json_int`: `json_get_raw(body, key).map(as_int).unwrap_or(i32::MIN)`
- `json_long`: `json_get_raw(body, key).map(as_long).unwrap_or(i64::MIN)`
- `json_float`: `json_get_raw(body, key).map(as_float).unwrap_or(f64::NAN)`
- `json_bool`: `json_get_raw(body, key).map(as_bool).unwrap_or(false)`
- `json_items`: `json_array_items(body)` → build a `vector<text>` via `stores.text_vector`
- `json_nested`: `json_get_raw(body, key).unwrap_or_default().to_string()`

**Step 5 — Feature gate** (`src/native.rs` or `src/main.rs`):
Register the H2 natives only when compiled with `--features http`.  Without the feature,
calling any `json_*` function raises a compile-time error:
`"json_text requires the 'http' Cargo feature"`.

*Tests:*
- Valid JSON object: each extractor returns the correct value.
- Missing key: returns zero/empty/null-sentinel without panic.
- Invalid JSON body: returns zero/empty/null-sentinel without panic.
- Nested object value: `json_nested` returns a string parseable by `json_int` etc.
- `json_items` on a 3-element array returns a `vector<text>` of length 3.
- Unicode and `\"` escapes inside string values are handled correctly.

**Effort:** Small–Medium (new `json.rs` ~80 lines + 7 native functions; no new dependency)
**Target:** 0.8.4
**Depends on:** H1 (for the `http` feature gate pattern)

---

## C43 — superseded by the V2 slot allocator

C43 targeted the V1 allocator's zone 2 (`place_large_and_recurse` in `src/variables/slots.rs`), which was deleted when `assign_slots_v2` became the production allocator.  V2 reuses a dead slot of the same kind and size, text included (`src/variables/slots_v2.rs`, the exact-match reuse under invariant I5).  The item as it stood:

### C43 — Text slot reuse: zone-2 dead-slot tracking

**Problem:** Text variables (24 bytes each) cannot reuse dead slots, wasting
stack space when many short-lived text variables are used sequentially.

**Root cause:** Text variables are placed by zone 2 (`place_large_and_recurse`
in `slots.rs`), which assigns slots sequentially at TOS without dead-slot
reuse.  Zone 1 has dead-slot reuse but only handles variables ≤ 8 bytes.

**Failed attempt:** A naive same-type reuse check caused slot conflicts
because it only compared against one dead variable, not ALL assigned
variables.  `nums` at [40,52) was still live when `_map_result_5` reused
slot 44.  Full conflict scan (like zone-1) is required.

**Files:** `src/variables/slots.rs`

P70 (text TOS-override) is NOT blocking: text-to-text same-size reuse
places the variable at the dead slot's existing position — no movement
occurs, so the `generate_set` TOS-override path is never triggered.

---

### C43.1 — Zone-2 dead-slot finder with full conflict scan

**Goal:** A standalone `find_reusable_zone2_slot` function that returns a
safe reuse slot or `None`.

**File:** `src/variables/slots.rs`

**Implementation:**
```rust
/// Find a dead zone-2 variable whose slot can be reused by variable `v`.
/// Returns `Some(slot)` if a conflict-free candidate exists, `None` otherwise.
/// Guards: same size, same type discriminant, dead (last_use < v.first_def),
/// no spatial+temporal overlap with any other assigned variable.
fn find_reusable_zone2_slot(
    function: &Function,
    v: usize,
    scope: u16,
) -> Option<u16> {
    let v_size = size(&function.variables[v].type_def, &Context::Variable);
    let v_first = function.variables[v].first_def;
    let v_last = function.variables[v].last_use;
    let v_disc = std::mem::discriminant(&function.variables[v].type_def);
    for (j, jv) in function.variables.iter().enumerate() {
        if j == v || jv.stack_pos == u16::MAX || jv.scope != scope {
            continue;
        }
        let j_size = size(&jv.type_def, &Context::Variable);
        // Same size + same type family (e.g., text-to-text only).
        if j_size != v_size || std::mem::discriminant(&jv.type_def) != v_disc {
            continue;
        }
        // Dead: candidate's last use is before our first definition.
        if jv.last_use >= v_first {
            continue;
        }
        // Full conflict scan: verify no other variable overlaps both
        // spatially (byte range) and temporally (live interval).
        let slot = jv.stack_pos;
        let conflict = function.variables.iter().enumerate().any(|(k, kv)| {
            if k == v || k == j || kv.stack_pos == u16::MAX {
                return false;
            }
            let ks = kv.stack_pos;
            let ke = ks + size(&kv.type_def, &Context::Variable);
            // Spatial overlap: [slot, slot+v_size) ∩ [ks, ke) ≠ ∅
            let spatial = slot < ke && ks < slot + v_size;
            // Temporal overlap: [v_first, v_last] ∩ [k_first, k_last] ≠ ∅
            let temporal = v_first <= kv.last_use && v_last >= kv.first_def;
            spatial && temporal
        });
        if !conflict {
            return Some(slot);
        }
    }
    None
}
```

**Debug guard:** When `function.logging` is true, emit:
```
[assign_slots]   zone2-reuse '{}' reuses dead '{}' at slot={}
```

**Verification:**
1. Add a unit test `zone2_reuse_conflict_free` that creates three 24-byte
   variables: v1 (live 0–10), v2 (live 5–15, overlaps v1), v3 (live 11–20,
   does not overlap v1).  Assert v3 reuses v1's slot but v2 does not.
2. `cargo test --lib assign_slots` — all slot tests pass.

---

### C43.2 — Wire zone-2 reuse into `place_large_and_recurse`

**Goal:** Call `find_reusable_zone2_slot` before advancing `*tos`.

**File:** `src/variables/slots.rs`, function `place_large_and_recurse`

**Change:** In the `if v_size > 8` block (line ~183), before `let v_slot = *tos`:
```rust
let v_slot = if let Some(slot) = find_reusable_zone2_slot(function, v, scope) {
    slot
} else {
    let s = *tos;
    *tos += v_size;
    s
};
```

Remove the existing `*tos += v_size` after `pre_assigned_pos = v_slot`.

**Debug guard:** `function.logging` message distinguishes "zone2" (new slot)
from "zone2-reuse" (reused slot).

**Verification:**
1. `cargo test --lib assign_slots` — all unit tests pass including the new
   `zone2_reuse_conflict_free` from C43.1.
2. `cargo test warning_only_program` — the `46-caveats.loft` script that
   triggered the original failure must pass (the full conflict scan prevents
   the `nums` / `_map_result_5` partial overlap).

---

### C43.3 — Enable `assign_slots_sequential_text_reuse` test — **Done**

`#[ignore]` removed; test runs unconditionally in `src/variables/slots.rs`.

---

### C43.4 — Integration test: text-heavy script with slot validation

**Goal:** Verify text slot reuse works end-to-end in a loft program.

**File:** `tests/expressions.rs`

**Test:**
```rust
#[test]
fn text_slot_reuse_sequential() {
    // Two sequential text variables with non-overlapping lifetimes
    // should not cause stack corruption.
    code!(
        "fn check() -> text {
             a = \"hello\";
             b = a + \" world\";
             c = \"goodbye\";
             d = c + \" world\";
             d
         }"
    )
    .expr("check()")
    .result(Value::str("goodbye world"));
}
```

**Verification:** `make ci` — zero failures across all test suites.
