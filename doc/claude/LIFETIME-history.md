<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# LIFETIME-history.md — how the closure and inline-lift rules were reached

The timeline behind [LIFETIME.md](LIFETIME.md): the closed closure-freeing implementation path and the inline-lift safety phases.

## Historical implementation path (closed)

The three bugs that originally motivated `Type::Function` work in
`get_free_vars` — cross-scope closure freeing, caller-side cleanup,
and capturing-into-struct-field — closed through @P213 (struct-field
layout via `Parts::ChildRec`, 2026-05-04), @P215 (nested-closure
name resolution, 2026-05-05), and @P227 (text-returning fn-ref
calls, 2026-05-05).  Plan-15 phases 03–05 (2026-05-12) confirmed
no residual leak via `tests/leak.rs` 100-iteration tight loops
across capture types (text / Reference / nested) and destinations
(local / struct field).

The detailed step-by-step "Implementation path" that previously
lived here described the contemplated `OpFreeClosureRef` opcode +
`get_free_vars` extension — neither shipped because the
`Parts::ChildRec` cascade plus standard local-cleanup already
covers the surface.  Removed during @PLAN15 phase 06 closeout
(2026-05-12); see git history if you need the original analysis.


---

## Inline-lift safety — history

P181 surfaced the corruption; Phase 1 (2026-04-18) added the gate;
Phase 1b (2026-04-18) added the `parse_return` dep merge; Phase 2
(2026-04-18) audited all `OpCopyRecord` emission sites and confirmed
the invariant holds.  See
`doc/claude/plans/finished/00-inline-lift-safety/` for the full initiative record.

