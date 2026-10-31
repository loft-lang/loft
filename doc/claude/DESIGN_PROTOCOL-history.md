<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# DESIGN_PROTOCOL — history

The incidents behind the `design-protocol` skill
([`.claude/skills/design-protocol/SKILL.md`](../../.claude/skills/design-protocol/SKILL.md)),
moved out of the skill so its working text keeps the method and a one-clause example each.
The working companion is [DESIGN_PROTOCOL.md](DESIGN_PROTOCOL.md).

## Why a probe, not more care — the two incidents

A real design's invariant was framed plausibly but **wrong**, and it "also subsumed" an
unrelated bug it actually didn't — both read as correct in prose; re-reading caught neither,
only probing the specific claims did. And the inverse: a design shipped *without* probing,
whose own plan said *"one chokepoint that 18 sites route through"* — a spray wearing the word
**chokepoint** — shipped as 18 bypasses and a silent mismatch.

## The other half — caching and geometry

- *Caching.* A startup-cache desync was symptom-chased across manifestations until the
  **target store state after a round-trip** was plotted concretely — the invariant then read
  out at once (*the round-trip must reproduce every binding identically*, the synthetic
  wrappers included). A one-line fix behind a large discovery cost.
- *Geometry.* A grid wall-straightener resisted eight approximations (line-fit, angle-buckets)
  against an *"exact, not an approximation"* spec, until the construction was drawn as a
  **plotted end-result** (subdivide each cell into triangles; keep the band the boundary cuts,
  drop the full-in / full-out ones). Only pinpointable after a throwaway prototype *produced*
  it — then ported unchanged.

## Capture and diff — the borrowed store-delivery

A borrowed store-delivery corrupted only under allocation pressure; three theories held
without a capture — a stride-type, a missing wrapper op, a lifetime flag — were each wrong.
Capturing the proven sibling (a simpler shape that survives the same stress) beside the broken
output showed the one real difference in a line: the proven path delivered into a **canonical
owned buffer**, the broken one reused an incidental local **as** the buffer — a dep the
lifetime analysis never tracked as the return, so a later allocation reused its slot. The
captured diff landed on the point the three patches kept circling.

## Write the doc before the code — the `StableCrateId` collision

A native-link `StableCrateId` collision was "fixed" by **build-identity keying** — rebuild the
package so its shared deps *match* the consumer's — coded before the design was written down.
It fixed `libloading` (a flags mismatch), then `log` collided (a *profile* mismatch), and the
next would have been build-script reproducibility: a symptom chased one collision at a time.
The load-bearing invariant — *keep the **API identity** (the C-ABI contract) separate from the
**Rust part** (the package's private internals), so the latter never enters the consumer's
link* — only became visible once the **failure paths were written down** and the two
identities were named on the page. *Matching* is whack-a-mole; the doc made *separation*
legible, and writing it first would have skipped the dead-end build entirely.

## When you recommend it — loft#2273

The case that put § When you RECOMMEND it in the skill: "a copy clones what its closure record
OWNS" was recommended and approved on the premise that an owned capture is reachable only
through closures. It is not: the frame that captured a written scalar keeps reading it through
the very store the record "owns", so writes through the copy stopped reaching the variable. A
five-minute control cell showed it — after the approval, not before.
