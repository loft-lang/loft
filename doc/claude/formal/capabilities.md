<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/capabilities.md — capability admission (strict, aspirational + conditional)

> **Rules then deviations** (see [README](README.md)). Two things set this area apart from
> `types` / `grammar` / `operational`, and both must be held in mind while reading:
>
> 1. **Conditional, not always-on.** The core areas constrain *every* program. These rules
>    bite only inside code a sandbox profile **designates** as restricted; trusted code
>    satisfies the judgment vacuously (`Cap-Trusted`). The judgment is parameterized by a
>    profile `P` — it is a refinement *layered on* the core under a policy, not part of what
>    every program obeys.
> 2. **Was aspirational, now BUILT.** The call gate (incl. closures — a lambda body is descended
>    into), the field-level rights, the parameter `#default` lock, AND the owned-vs-host write
>    classification are all enforced today (`src/sandbox.rs`: `admit_capabilities`,
>    `field_*_violations`, `param_lock_violations`; `parser`: `mark_lambda_sandboxed`,
>    `raw_write_is_host_owned`) — the deviation list ([@PLN86 §7](../plans/86-sandbox-subset-flag/README.md))
>    reached 0. Writing the rules first was direction: it turned "what exactly may a mod do?" into a
>    relation the admission walk is checked against.
>
> The model with worked examples lives in [SANDBOX.md S10](../SANDBOX.md) +
> [@PLN86 §7](../plans/86-sandbox-subset-flag/README.md); this doc is the **rule**. The
> owned-vs-host predicate these rules lean on is [ownership.md](ownership.md)'s — cited, not
> redefined.

## The frame, in one line

A capability is a permission the **host** requires of a **restricted caller**. The host
annotates *its own* surface (functions, parameters, struct fields); a profile grants a set
of `group#right` tokens; admission gates every point a restricted caller's code touches the
host. **A modder's own code carries no links and is never restricted** — only host reach is
gated.

## Notation

- **`P`** — the active profile: `P.allow` (granted `group#right` tokens) and `P.libs`
  (libraries allowed wholesale). The host owns it; a script cannot grant itself (@PLN86 §1).
- **`ctx`** ∈ `{restricted, trusted}` — `restricted` inside a sandbox-designated def,
  `trusted` everywhere else.
- **`P ; ctx ⊢ e ✓`** — "under profile `P` in context `ctx`, operation `e` is **admitted**."
- **`g#r ∈ P`** (granted) ≝ `∃ a#r ∈ P.allow` with `cap_prefix_match(a, g)` — the **right is
  equal** and the grant's group `a` is a dotted-segment prefix of `g` (so `game#read` grants
  `game.entity#read`, never `game#update`). Groups are declared (`capability g`); a link to
  an undeclared group is a load error, prior to this judgment.
- **`owned(e)`** — `e` operates only on **script-owned** data (a locally constructed value,
  or a script-defined type). The complement is **host** data (a parameter root, a
  host-library type). This is a *provenance* fact — [ownership.md](ownership.md).
- **`gate(f)`** — function `f`'s call-gate link (its signature link, §7.1); `⊥` if untagged.
- **`default(p)` / `lock(p)`** — a parameter's default value, and its `…#default` lock
  (§7.2); `lock(p) = ⊥` if the parameter is untagged.
- **`readcap(m)` / `cap(m, r)`** — field `m`'s read link, and its link for right
  `r ∈ {update, append}` (§7.3); `⊥` if untagged.

---

## Rules

> The judgment is **sound** (an admitted operation lies within `P`'s grants — no host effect
> escapes the profile) and **complete** (every host-touching construct has a rule, so
> `Cap-Deny` is reached only by genuine absence of a grant, never by an un-judged construct).

```
  (Cap-Trusted)  ctx = trusted                                           ⟹  P ; ctx ⊢ e ✓
  (Cap-Own)      owned(e)                                                ⟹  P ; ctx ⊢ e ✓
  (Cap-Call)     e = call f ;  gate(f) = g#r ;  ( g#r ∈ P  ∨  lib(f) ∈ P.libs )
                                                                          ⟹  P ; restricted ⊢ e ✓
  (Cap-Set)      e = pass arg a to param p of host f ;
                 a ≡ default(p)  ∨  lock(p) = ⊥  ∨  lock(p) ∈ P           ⟹  P ; restricted ⊢ e ✓
  (Cap-Read)     e = read host field m ;   readcap(m) = ⊥  ∨  readcap(m) ∈ P
                                                                          ⟹  P ; restricted ⊢ e ✓
  (Cap-Write)    e = write host field m, right r ∈ {update, append} ;
                 cap(m, r) = g#r ∈ P ;   ( r = append ⟹ m : collection ) ⟹  P ; restricted ⊢ e ✓
  (Cap-Deny)     no rule above applies                                   ⟹  e is REJECTED at load
```

**In words.** Outside the sandbox nothing is gated (`Cap-Trusted`). Inside it, anything a
script does to data **it owns** is free (`Cap-Own`) — only reaching into the *host* is
checked. To **call** a host function you need its gate granted, or its whole library allowed
(`Cap-Call`); once you may call it, you may pass any **argument** — except a parameter the
host **locked** to its default, which needs the lock (`Cap-Set`). **Reading** a host field is
free unless the host marked it private (`Cap-Read`); **updating or appending** to one needs
the field's grant, and *append* is only meaningful on a collection (`Cap-Write`). Anything
with no admitting rule is rejected **at load** — deny-by-default for the write side,
allow-by-default for reads and for argument-passing.

### Construction is unrestricted (the closed decision)

There is **no** `(Cap-Construct)` rule, by decision (the @PLN86 design's "position 1").
Building a host value — an enum variant `Command.Shutdown`, a struct `Entity{…}` — is
**free**: a constructed value is script-owned, so `Cap-Own` already admits it. Such a value
is inert until it **enters host state**, and every entry point is one of the boundary rules
above (`Cap-Write` to put it in a host field, `Cap-Call`/`Cap-Set` to pass it to a host
function). The "un-forgeable variant" property is therefore a property of the **boundary**,
not of construction: a granted `q#append` of element type `Command` *is* a grant to append
any `Command`, so if a privileged variant must not enter `q`, the host's obligation is to
narrow the element type or validate at the consumer — the **L-write contract** (a granted
write op is invariant-preserving for any value its type admits), not a new right.

### Why the rule set is closed

Every way a restricted caller can touch the host is one of: **call** a function, **pass** it
an argument, **read** / **update** / **append** a field. `Cap-Call`/`Cap-Set`/`Cap-Read`/
`Cap-Write` cover those four, `Cap-Own`/`Cap-Trusted` carve out the un-gated remainder, and
`Cap-Deny` is the deny-by-default floor. The model compressing to **six judgments** is the
evidence it is at the right altitude — an operation that needed a seventh rule would be an
edge the rules can't express, i.e. a signal the *rule* is wrong (README), not a new case.

> **Pattern matching is capability-neutral (@PLN35, SPEC-FIRST).** PEG match patterns
> ([matching.md § Rules — PEG patterns](matching.md)) introduce no new host surface — no I/O, no
> ambient authority. A `match` over an iterator inherits the iterator's own admission (the *pull* is
> the gated operation, not the match), so no new `Cap-*` rule is owed.

---

## Deviations

**OPEN: 0.**  Every deviation this doc has carried is closed; the record is in
the companion [capabilities-history.md](capabilities-history.md).

## Conformance

This area's **falsifying programs** are the adversarial escape suite
(`admission_escape_suite_rejects_every_breakout`, `src/parser/mod.rs`): each is a restricted
program that *tries* to perform a host operation outside its grants — an ungranted call, an
indirect fn-ref call, a raw write — and the rule it must hit is `Cap-Deny`. The positive
controls (a granted call, a read, a script-owned mutation) are the `Cap-Call` / `Cap-Read` /
`Cap-Own` side. The field-right and parameter-lock breakouts have landed as RED/GREEN twins in
`access_corpus_red_green` (update a read-only field; append where only update is granted; pass a
non-default value to a locked parameter) — each a program whose admission flips to the exact rule
above, its GREEN twin proving the rejection is the rule firing, not an incidental reject.

The area is **formal — OPEN 0**: every host-touching
operation in a restricted context is decided by exactly one of the six rules — the call gate
(incl. closures), field read/update/append, the parameter `#default` lock, and script-owned
mutation.  The claim that an admitted script performs no host effect outside its profile is
only as strong as the construct list the judgment covers, and two walks found constructs it did
not (a view of a parameter, a whole-value read — capabilities-history.md D-cap-4/5).  The
falsifying escape suite + the RED/GREEN access corpus are the standing evidence; a new construct
that reaches host data owes a cell in the escape suite.
