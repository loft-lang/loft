# @PLN91 — a `parallel { … }` block (prepared subject)

**The family:** "an expression form", reason `SParallel`.  Its 12 sites sit in 2 programs held by
it alone:
- 2112-a-parallel-arm-stops-at-its-own-frame (`void_with_param`, `returns_value`, `no_params`,
  `inner_block`, `with_text`, `Pt::span`, a test body, a lambda);
- C-Arms/2568-parallel-arms-run-at-once (multi-arm blocks of calls).

The port PARSES the block (`syntax.loft` ~1143: `SParallel { at, items }`) and its back end
already carries the IR node (`NdParallel { arms }` in bgen, binds, cgen_cold).  But the
elaborator has no case for `SParallel`, so the expression dispatch refuses it as a form.

**The rule:** `@F33` (`par`): each arm runs in an isolated worker.  An arm may READ an enclosing
local, whose value is copied in.  Writing an enclosing local, or capturing a parameter at all,
is a compile ERROR.

## Rust's form

`Parser::parse_parallel` (src/parser/control.rs ~17638):
- each arm is `self.expression(&mut arm)`, and a `Null` arm is dropped; `;` between arms is optional;
- the block's value is `Value::Parallel(arms)`, typed `Void` (expressions.rs ~1505);
- on the second pass only:
  - an empty block is the `empty-parallel-block` WARNING;
  - `reject_unsound_parallel_captures` raises the two errors above, judged against the locals
    DEFINED at the opening brace (`vars.is_defined`, taken before the arms are parsed).

Measured on try dfdb2d185:
```
2112:50   fn no_params() -> integer { n = 5; parallel { check(n, 5); } n * 3 }
  [49] n: integer = 5;
  [50] parallel {
          n_check(n, 5);
  };
  [51] OpMulInt(n, 3);

2568:62   parallel { answer2568("c2568_a.txt", "c2568_c.txt", "C"); answer2568(…, "A"); mark2568("c2568_b.txt", "B"); }
  [62] parallel {
          n_answer2568("c2568_a.txt", "c2568_c.txt", "C");
          n_answer2568("c2568_b.txt", "c2568_a.txt", "A");
          n_mark2568("c2568_b.txt", "B");
  };
```
Each arm is the call as it stands.  A call answering void has no `drop` around it.  The block
is a statement of its enclosing block.  (2112's `with_text` reads a text local, `t`, in its
arm: `n_check_text(t, "<ab>")`.)

## What to code

In the expression dispatch, `SParallel { items }` elaborates each item as an expression, drops
the null ones and answers `NdParallel { arms }` typed void.  Two second-pass checks fail on
programs Rust itself refuses, so a refusal by name is enough for now: an arm that writes an
enclosing local, and an arm that names a parameter.  (2112's `void_with_param` copies its
parameter into a local `n` before the block and reads only `n`.)  The empty block is a warning,
not a refusal.

## Probes (the two sole programs)

```
kr-try/tests/scripts/2112-a-parallel-arm-stops-at-its-own-frame.loft:39 … :115   (one arm each, a lambda's included)
kr-suite/rules/concurrency/C-Arms/2568-parallel-arms-run-at-once.loft:50, :62, :80, :99   (two and three arms)
```
