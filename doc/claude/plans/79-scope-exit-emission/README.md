# @PLN79 — Scope-exit gate simplification

**Status: CLOSED — decided against (won't-do) 2026-07-09.** The driver was mis-framed and the
proposal was superseded by the ownership rework. @PLN79 was opened (2026-05-02) as a @P203 fix; that
framing was wrong (@P203 turned out to be a template double-substitution, tracked in
[PROBLEMS.md](../../PROBLEMS.md)). It was then rescoped to a structural simplification: strip the
dep-derived half of the scope-exit `OpFreeRef` gate — `let emit = (dep.is_empty() || is_work_ref) &&
!in_ret && !function.is_skip_free(v)` → `let emit = !in_ret && !function.is_skip_free(v)` — relying
on `OpFreeRef` being safe on an already-freed slot (`codegen_runtime.rs:100-104`).

That simplification is now the **wrong shape**. Since @PLN79 was deferred:

- The gate moved and grew — it is `src/scopes.rs:3776`'s `(owns || is_work_ref || inject_free) &&
  !in_ret && !function.is_skip_free(v) && !captured_ref` (`owns` = `dep.is_empty()` + an @P302
  keyed-collection exception). The dep-coupling @PLN79 flagged accreted MORE special cases (@P302,
  #323 `captured_ref`, @PLN94 `inject_free`), not fewer.
- @PLN94 landed a flow-sensitive `ownership_of` oracle (pure-observer cross-check). The clean
  decoupling — promote that oracle to the emission *authority* and collapse the special-case gate
  into one ownership query (the move @PLN85 already made on the free-side tracker) — is better than
  @PLN79's "emit more frees, rely on idempotency," and is a **fresh plan** to file once the oracle
  graduates from observer to authority (still stabilizing — caught #495/#500/#501 this cycle).

Full decision record: [DESIGN_DECISIONS.md § @C88](../../DESIGN_DECISIONS.md). The phase-00
characterisation ([00-characterize.md](00-characterize.md), incl. the @P203 strace diagnostic) and
the phase-01 sketch ([01-simplify-gate.md](01-simplify-gate.md)) are retained below as historical
record.
