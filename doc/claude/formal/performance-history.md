# formal/performance-history.md — the deviation register for [performance.md](performance.md)

> **The rules are next door.**  [performance.md](performance.md) states what must always be true of
> the distribution's speed; this file is its TIMELINE — every place the code was measured not to
> do it, when, what it cost, and what closed it.  The rules doc carries the CURRENT state (how
> many are open, and which); everything below is the record behind it.

- **D-perf-1 — OPENED 2026-09-21, CLOSED 2026-09-29 (loft#1570)** — violates (Perf-Weight) and (Perf-Twin): a drawing routine
  over the 3× bar, and four with no reference twin to judge them against.  Measured 2026-09-21
  on main 9f5cf6a96 (`compare.py --skip-interp --repeat 4 --bar 3.0`, quiet machine, 14/14
  hashes): of the ten routines with a Rust reference only `smooth` is over, at 4.50× (a 200 ns
  reference that swings; 7.50× under load the same afternoon), and the median is 1.41×.
  `parse`, `render_lock`, `render_marks` and `resize` have no reference in the bench, so the
  bench cannot judge them; the 2026-09-17 figures had them at 3.39×, 4.43×, 6.25× and 4.51×.
  The entry closes when every routine has its twin and meets the bar.

  Filed as loft#1426 when the routines ran 10–50× behind their twins on `--native-release`
  (hash-validated lanes, so the comparison is admissible under (Perf-Like)).  Profiler
  attribution showed the loft side's hot loop matching the reference's, placing the cost in
  the value model (`codegen_runtime` / `DbRef` indirection — PERFORMANCE.md's N1 class), not
  the library.  That issue closed when @PLN157 brought every judged row within the then 4×
  bar; @PLN157 and @PLN164 remain the fix streams.

  **Closed** by the @PLN157 hoist work on branch `157-native-4x` — a callee judged with the
  hoist's own allowances, a callee's own buffer counted as a fresh record, and a tuple answer's
  dead buffers (b7e36e517, c5bfb9e30).  Measured 2026-09-29 by `make perf-portal` at 639aaa2c3
  (main joined, quiet laptop, 7 interleaved samples, 14/14 hashes): every routine has its twin
  and every one is within 3× — `parse` 2.73×, `fronds` 2.23×, `smooth` 2.14× (a ~400 ns
  reference, flagged coarse), the rest at or under 1.80× — and the median over the fourteen is
  1.48×.

## D-perf-2 — the measurement it opened on

Measured 2026-09-29 by `make perf-portal` at 639aaa2c3 (every in-repo lane and every library bench
with a Rust twin, hash-validated under (Perf-Like)): 205 routines, median 2.79×, 140 over 3×, 29 of
them at 10× or more — `pluginabi` `check_request` 59.9×, `mesh3d` `mat4_mul` 39.0× and
`mesh_to_floats` 32.1×, `cbor` `encode_bytes` 31.7×.  The per-routine rows are
`bench/portal/results/<host>.tsv`, whose git history `make perf-trend` reads.  Marked not resolvable
in a release by the owner the same day, for the 2026-10 release.

