# @PLN91 tester handoff — 2026-11-24 (second)

Resume point for the tester session (`tester`).  Peers: `porter` (the port, branches
tuxedo-pln204-int32 / -next), `merger` (joins and PRs).

## State

- **Gate at the porter's f39527f74** (R2, vector constants): 1415 of 2142 agree, 0 differ.
- **R10 built and landed** (d067d87a8): the 32-bit regime's lowering in the port; try 9e8e2f176
  on f39527f74 = 1426 of 2142 (+11), 0 differ, nothing lost, 0 first-pass drops.  The porter has
  been asked to merge it.  What is not carried: refusal-worklist.md § R10.
- **Landed on loft30-gate, fixes of main bugs:** loft#2624 (a file directive with an argument
  heading a main-less file read as a script statement), loft#2625 (silent-wrong: a failed
  `assert` in a main-less file exited 0 on the interpreter), both taken into the PR by merger;
  loft#2626 (cf2da5152, silent-wrong: a `match`/`if` arm answering null on every path beside a
  value arm lost the sibling's value on the interpreter, or its own `return` on both backends) —
  moves none of the porter's functions.  All three are in the PR (tuxedo-pr2622-next); R2, R10
  and the handoffs wait for the next PR.
- **Receipts** a-function-type-can-name-a-link-parameter / a-function-type-link-refusals
  re-derived (642eee930), taken by merger.
- **PR held** on the owner's call about tuxedo-pln225's own 47 failing tests.

## Open: four interpreter faults met while running elab.loft `--interpret`

The gate runs the elaborator compiled with the C backend, which is right in all four; the
interpreter is not.  Each is a loft defect (interpreter vs C), unfiled because no small program
reproduces it yet.  Reproduce with the elaborator of 755d420d3 (or the tree named), the Rust
image made by `LOFT_EMIT_PARSED=<img> loft <file>`, and
`loft --interpret selfhost/front/elab.loft <img> <file> <std.store>`:

1. **Use after free** — tests/scripts/a-value-read-and-dropped-is-built-outside-the-store.loft:
   `LOFT_STRICT_STORES=1` reports a read of store type `Ex` freed in `parse_code`
   (el_expressions.loft line 240).  Present before loft#2626 too.
2. **Wrong IR** — `fn name(k: integer) -> text? { if k % 2 == 0 { "n{k}" } else { null } }`
   prints `if OpEqInt() {` (no operands); with `k == 0` the block's name prints empty.  Present
   with the elaborator of d8c7fa070, so it predates R10.
3. **Double free** — in `int_range` (el_convert.loft, the `NdBlock` arm's `return int_range(…)`)
   when `if_expr` asked `el_convert::int_range(e, vars, f.code) != null && …` inside a `match`
   arm's `&&` chain.  R10 asks `value_fits` alone instead.
4. **H5 ICE compiling elab.loft** — `int_range`'s `NdBlock` arm written as
   `r: Range? = null; if … { r = int_range(…) } else if … { r = coalesce_range(…) }; r`:
   `def n_int_range grew a pass-2-only attribute (pass1=4, pass2=6)` on every build.
   R10 keeps the early-return shape.

## Next

1. The four faults above: a small reproduction of each, then fix (the tester's lane).
2. R9 (`np/shift2.txt` in the old scratchpad, re-captured clean with try-676e38bbc's tools):
   the tuple-vector registration first (refusal-worklist.md § R9).
3. Every porter try: test, report, first-pass drops called out.

## Tooling notes learned today

- The interpreted harness (`w32/run.sh`) is for quick iteration; confirm a differ with the
  try's compiled `elab` (`try-<sha>/elab/elab <img> <file> <std>`) before analysing it.
- try.sh applies a patch without a 3-way merge: cut the patch from a worktree where it is
  already applied (`git diff --cached <base>`).
- A copied `loft` finds `default/` beside a `target/release/` layout: copy it as
  `<dir>/target/release/loft` with `<dir>/default` a link.
