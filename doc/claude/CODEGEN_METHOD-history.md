<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# CODEGEN_METHOD — history

The incidents behind the codegen method's rules, as told in the `loft-codegen` skill before it kept only the rules; the method itself is [CODEGEN_METHOD.md](CODEGEN_METHOD.md).

## The working-bytecode gate: @PLN85 probes 04 and 05

@PLN85 probe 05 was fixed correctly — working bytecode proven standalone
(`bytecode-comparisons/` rungs, the hand-correct `a:vec=[]; a+=enc()` form) → types → codegen
→ clean on both backends. Probe 04 was NOT — a parser change went in with no working IR proven
first; it regressed `loft_suite` and never closed the case. Same method, opposite outcomes. The
difference was the gate. The rungs are under
`doc/claude/plans/85-store-lifetime-retirement/bytecode-comparisons/`.

## Reading the record of attempts first: loft#1096 and loft#1097

**The anti-example is loft#1096** (`formal/ownership-history.md` D-own-9). Its obvious repair —
skip `OpFreeRefIfDistinct` when the witness is null — measured clean on both backends, then the
poison sweep exhausted the store table. The register had already recorded that exact move as
*"a trade, not a closure"*, reverted as inert. Reading it first would have skipped the attempt:
CLAUDE.md's *"READ THE FORMAL SPEC FIRST"* with a price tag on it.

**But a closure names where ITS unsound step was, not yours.** The register pointed at the
promotion (loft#1081's site); loft#1096's unsound step was one site over, in
`scopes::free_vars`. Read the register for what was TRIED and measured; the next defect in the
same machinery is measured, not inherited.

**Grep the BELIEF, not the op.** loft#1096 and loft#1097 are one wrong sentence — *a
collection slot holds the null sentinel on a path that did not write it* — at two sites, each
of which wrote the belief down in its own comment. When a register entry names a false premise,
the next search is for other places that hold it.

## Equal today is not the same rule: the narrow-width family

The narrow-width family once had five sites spelling one list and only three were the same
question — the other two write a RAW slot where the three write an encoded field, and their own
comments said so. Merging on the list alone would have folded a raw-slot writer onto an
encoded-field writer. The evaluation is `formal/IMPLEMENTATIONS.md` § The narrow widths.
