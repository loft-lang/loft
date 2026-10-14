<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Alias where an alias is correct — a safe refinement of C86 (copy-by-default)

> **Status: design (2026-07-16).** Keeps [@C86](../../DESIGN_DECISIONS.md) (whole-value heap binds
> COPY; `&` is explicit write-through) as the *contract*, and adds the compiler intelligence to
> **alias instead of copy in the cases where an alias is correct** — safely, so the memory-model bug
> that drove the copy (#415's aliasing UAF) never returns. This is exactly @C86's own "revisit when:
> widen `ElidePlan`". Written before the code (design-protocol § "write the doc before the code").

## The request, made precise

**The principle (owner, 2026-07-16): every variable is its own value — copy is the *semantics*,
always, with no observable exception. A "link" (a shared store) is never part of the semantics; it is
either the compiler's *transparent* optimization (allowed only where a link is indistinguishable from
a copy) or the programmer's *explicit* `&` (the one visible way to ask for an observable link).** So
"still make links where that is possible" means **where a link is both SAFE (no use-after-free) and
UNOBSERVABLE (computes the identical result)** — because a link that *changed* the result would mean
the variable no longer semantically has its own copy, which the principle forbids.

This settles the design cleanly — there is no fork. Two consequences:

- **Transparent links (the design's payload):** the compiler shares storage under the hood wherever
  it is safe + unobservable — the programmer never sees it; it is pure performance. This is the
  `ElidePlan` widening (below). *Everything observably stays the same.*
- **The write-through cases stay EXPLICIT.** `th = t.tr_h; th[i] = v` (discarded) does **not** silently
  link — that link would be *observable* (the field changes), so the principle forbids the compiler
  from making it behind the programmer's back. The lost write is instead **surfaced by the dead-store
  lint** — *"this write lands in a copy and is lost; did you mean `&t.tr_h`?"* — steering to the
  explicit `&`. Recovering the write-through is the programmer's `&`, never a hidden alias.

So the design is: **transparent links (widen `ElidePlan`) + explicit `&` + a lint that points at
`&`.** (An earlier draft floated auto-aliasing the dead store; the principle rejects it — an
observable link the programmer didn't write is exactly the spooky action @C86 forbids. Kept below as
§ Rejected, for the record.)

## The one invariant (design-protocol step 1)

> **A bind is COPY semantics, always. The compiler realizes it as a shared-store LINK iff the link is
> SAFE — the source store provably outlives every use of the local (no use-after-free) — AND
> UNOBSERVABLE — copy and link compute the identical result. Any observable link is the programmer's
> explicit `&`, never inferred. Otherwise: a real copy.**

The invariant is **sound by conservatism**, inheriting `use_analysis`'s existing discipline (*"can
only lose an elision, never produce a wrong borrow"*): if safety-and-unobservability cannot be
*proven*, it materializes the copy. So the refinement can never reintroduce #415's aliasing UAF nor
change any observable behaviour — the worst case is a missed link (a real copy that was safe to
share), never an unsafe or observable one. **This is what makes it landable near loft's #1 weakness,
and why it needs no contract-key: it changes nothing a program can observe.**

## The safety gate — the shared precondition (why #415 cannot return)

#415 switched field-read binds to copy because the alias was a **use-after-free**: `a = x.v` aliased
the field's store *without owning it*, so a later free of `x` dangled `a`. So an alias is *correct*
only when it is *safe*, and safety is precisely the store-lifetime fact loft already computes:

- **Provenance:** `use_analysis::Ownership::Borrowed { base }` already carries, for a bound value, the
  caller-visible source var whose store backs it. That is the alias target.
- **Lifetime:** the alias is safe iff `base`'s store outlives the local's last use — the local does
  **not escape** past `base` (the dead-store lint's existing *non-escaping* check, `warn_dead_stores`
  S4a) and `base` is not freed while the local is live (the `deps` / `reclaim_safe` fact).

Only inside this safe set does the design ever prefer an alias. Outside it, @C86's copy stands
unchanged. The gate is one fact read at one place (below), not a per-site re-derivation.

## Transparent links — the design's payload (always-on, invisible)

The compiler realizes a bind as a shared-store link when the link is **indistinguishable** from a
copy AND safe. Sufficient sound conditions:

- **Source dead after the bind** — the current `ElidePlan` last-use elision (already shipped). A
  mutation through the local reaches no observed read of the source.
- **Both source and local read-only after the bind** — with no mutation on either side, shared vs
  independent storage is unobservable. This is the **widening** (most binds are read-only), the
  concrete cash-out of @C86's "widen `ElidePlan`".

This is a pure optimization: **the observable result is byte-identical to copy-everywhere**, so it is
"everything the same" with no contract question and no contract-key. It does **not** touch hex_terrain
— there the source is read again after a mutation through the local, so a link is *observable*, and
the analysis correctly declines it (a real copy stays; the lost write is caught by the lint below).

## The write-through cases stay explicit — the lint points at `&`

`th = t.tr_h; th[i] = v` with `th` discarded is a **provable dead store**: the copy guarantees a lost
write. The principle forbids the compiler from silently linking it (that link is observable). So the
answer is not a hidden alias but a **loud lint** that recovers the intent by *teaching the explicit
form*. The detector already exists: `use_analysis::dead_store_accesses` +`warn_dead_stores` isolate
exactly this case — an **`Owned`, non-escaping** local with `reads == 0 && write_targets > 0`. The
one change is the *message*: surface it as an [arc-C recommended-idiom steer](recommended-idiom-channel.md)
pointing at the fix — *"the write to `th` lands in a copy and is lost; write `th = &t.tr_h` for
write-through, or read `th` back if you meant a copy."* The write-through happens only when the
programmer writes `&`; it is never inferred.

### Rejected: auto-aliasing the dead store (for the record)

An earlier draft floated turning the detected dead store *into* a link (silently, or with a steer) so
hex_terrain would "just work" without `&`. **The principle rejects it:** that link is observable (the
field changes), so making it behind the programmer's back is exactly the spooky, non-local semantics
@C86 forbids — the bind's meaning would depend on whether the local is read later, and a dead store
that was really a *missing read-back* bug would be silently converted into a source mutation instead
of surfaced. The options and why the middle/loud ones still lose:

| Option | Behaviour on the dead-store case | Verdict |
|---|---|---|
| **A — copy + lint-to-`&` (CHOSEN)** | real copy; the lint steers to `&` | keeps copy-is-the-semantics; local; the write-through is the programmer's explicit `&` |
| ~~B — alias + steer~~ | link (write-through) + a steer | **rejected** — announcing an observable link does not make it *not* an inferred observable link; still violates the principle |
| ~~C — alias, silent~~ | link, no diagnostic | **rejected** — maximally spooky; can silently mutate the source when the real bug was a missing read |

## Re-assertion count (design-protocol step 2) — N = 1

The copy-vs-link choice is made in **one place** — `use_analysis`'s verdict feeding
`scopes::elide_borrows` (the `ElidePlan`). The widening extends that single computation; codegen keeps
*reading* the plan, never re-deriving it. The safety gate is one fact (`Ownership::Borrowed { base }`
+ non-escape), the observability gate one fact (the read/write access classification). There is no
per-bind-site spray: one analysis, one plan, one codegen consumer — exactly the shape @C86 already
ships, widened. The lint is a second, independent single pass (`warn_dead_stores`, already shipped).

## Falsification — how it breaks (design-protocol steps 3–4)

- **Claim: "the safety gate makes an unsafe alias impossible."** The gate is sound-by-conservatism
  (proven-safe-or-copy). Falsification target: a boundary matrix over the store-lifetime axes that
  drove #415 — a bound field mutated after its `base` struct is freed / reassigned / escapes via a
  return; each must **copy** (no alias), verified on both backends under `LOFT_POISON` +
  `LOFT_NATIVE_LEAK_CHECK`. A single unsafe alias here is the #415 regression; this matrix is the
  gate.
- **Claim: "the link is unobservable."** Falsify with the distinguishing observations: (a) mutate the
  source then read the local; (b) mutate the local then read the source. Any program that can make
  either observation must **not** be linked (it stays a real copy). Positive control: a read-only bind
  links and is byte-identical; a source-then-mutate-then-local-read bind copies.
- **Claim: "the safety gate is sound."** Even an *unobservable*-looking link must be **safe** — the
  observability check assumes the storage stays alive; a freed/reassigned `base` breaks that. So both
  gates apply. Falsification target below (the #415 matrix).
- **Claim: "this preserves the contract, no key needed."** The design links only where copy and link
  are *both* safe and indistinguishable → **no program can observe any difference**, so nothing is a
  breaking change and no contract-key is required. This is the whole point of the principle: because
  the semantics is always copy, the optimization is free. (Contrast the rejected auto-alias, which
  *would* have needed a contract-key precisely because it was observable — its own evidence that it
  violated the principle.)

## The safe small steps

> **Detailed build + validation plan: [alias-where-correct-build.md](alias-where-correct-build.md)** —
> the instrument-first ladder (prove each oracle report-only against a hand-computed matrix *before*
> codegen), the two boundary matrices (safety + observability, incl. the sibling-alias trap), the
> byte-value-identity gate, and the stop conditions. The table below is the summary.

Inert-first, each verifiable before the next. The whole design is a pure optimization + a lint-message
change — no observable behaviour changes, so there is no gated-measure-then-flip risk beyond proving
byte-value-identity.

| # | Step | What lands | Verify | E |
|---|---|---|---|---|
| 1 | **Safety oracle, exposed + matrixed.** Surface `link_is_safe(local)` = `Ownership::Borrowed { base }` present AND local non-escaping AND `base` outlives the local (reuse `dead_store_accesses` non-escape + `reclaim_safe`). No codegen change — it only *reports*. | the #415 store-lifetime matrix: every "base freed/reassigned/escapes while local live" cell → `unsafe` (would copy); a plain field-read-then-read → `safe`. Both backends. Positive control: an injected unsafe shape reads `unsafe`. | M |
| 2 | **Observability oracle.** Surface `link_is_unobservable(local)` = neither source nor local is mutated after the bind (or the source is dead — the shipped last-use case). Reuses the read/write access classification (`dead_store_accesses` + the elision's last-use). Reports only. | a read-only-both bind → `unobservable`; a mutate-source-then-read-local (and the reverse) → `observable`. Positive control: an injected observable shape reads `observable`. | S |
| 3 | **Widen `ElidePlan` to link the safe + unobservable set (GATED, default off).** In the elision verdict, additionally realize a bind as a link when `link_is_safe && link_is_unobservable` (the read-only-both case, on top of the shipped last-use). Behind `LOFT_LINK_WIDEN` (opt-in). | byte-identical corpus (loft-codegen Mode B) with the flag OFF; ON → those binds emit a borrow not a copy (introspect diff) and the **observable result is unchanged** on the full suite both backends; `--report-copies` shows the copy-count drop; leak-clean. | M |
| 4 | **⏸ Default-on DEFERRED — win measured ≈ 0.** The flip was implemented + the suite proved observably-identical (byte-value-identical + leak/UAF-clean, both backends), so the mechanism is correct. But the copy-count measurement (the cheap disproving probe) is a **stop signal**: the read-only-both field-bind pattern is essentially absent in real loft code — **0 copies eliminated** across the vector-heavy scripts AND the ~2000-line `viewer/main.loft`. @C86's revisit trigger (a profiler showing bind-copies dominating a real consumer) is not met, so it **stays opt-in** (`LOFT_LINK_WIDEN`), ready if a consumer ever demonstrates the win. The premise "more links, copies drop" did not survive contact with real code — the correctness holds, the value does not (yet). | ✅ correctness proven; ✅ win ≈ 0 → deferred | S |
| 5 | **Point the dead-store lint at `&` (the write-through-intent case).** Upgrade `warn_dead_stores`'s message on an `Owned` non-escaping `reads==0 & write_targets>0` local to the arc-C recommended-idiom steer: *"the write to `x` lands in a copy and is lost — write `x = &<src>` for write-through, or read `x` back if a copy was intended."* No behaviour change (still a copy); a clearer nudge to the explicit form. | a hex_terrain-shaped fixture: the lint now names `&<src>` as the fix; the value is unchanged (still a copy — write-through is the programmer's `&`); the escaping-local control does not fire | S |

**Shape:** steps 1–4 are the unconditional win — link more, observe nothing new; the safety oracle
(step 1) is the keystone that guarantees #415 never returns, and step 3's byte-value-identity is the
proof. Step 5 is the ergonomic close: keep the semantics (copy), but make the one case a programmer
actually trips on (the lost write-through) point straight at `&`. No owner fork remains — the
principle chose it.

## Relation to C86, arc C, and the freeze

- **@C86 stays the contract, verbatim.** This is its *"revisit when: widen `ElidePlan`"* clause cashed
  out — the semantic (every variable is its own value) is unchanged; the compiler just realizes the
  link in more of the safe + unobservable set. No @C86 change; no dead-store corner refinement.
- **Arc C is the lint's delivery** — the "use `&`" nudge (step 5) is a recommended-idiom steer on
  owned source (the [arc-C channel](recommended-idiom-channel.md)); the write-through itself is always
  the programmer's explicit `&`, never inferred.
- **The freeze (arc E).** The whole design is contract-clean at any time — it changes nothing a
  program can observe, so it needs no contract-key and can land before or after the freeze freely.

## See also

- [DESIGN_DECISIONS.md § @C86](../../DESIGN_DECISIONS.md) — the copy-by-default contract this refines
  (+ its "revisit when: widen `ElidePlan`").
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the bind law + `#415` (why the alias became a copy:
  the UAF the safety gate now prevents).
- [recommended-idiom-channel.md](recommended-idiom-channel.md) — arc C; the dead-store lint's "use `&`" steer (step 5).
- Code-points: `src/use_analysis.rs` (`Ownership::Borrowed`, `dead_store_accesses`, `warn_dead_stores`
  S4a, the elision verdict) · `src/scopes.rs` (`elide_borrows` / `move_elide` — where a copy becomes an
  alias) · the `#415` store-lifetime guards (`tests/scripts/85-store-lifetime-field-read-copy.loft`).
