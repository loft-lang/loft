<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN206 — one store model

The plan is [loft-lang/plans#206](https://github.com/loft-lang/plans/issues/206): its phases,
their state and its documentation list live on the issue.  The model itself, and every
divergence between the two runtimes with its decision, is
[`STORE_MODEL.md`](../../STORE_MODEL.md); a closed divergence moves to
[`STORE_MODEL-history.md`](../../STORE_MODEL-history.md).

- [`p3-p4-design.md`](p3-p4-design.md) — P3 (one store layout) and P4 (a hosted C unit on the
  interpreter's stores): the invariant, the shared prefix, the failure paths, the phases, the
  two instruments that compare the runtimes by their stores (`scripts/store_diff.py`,
  `LOFT_HOSTED_VERIFY`), and what was measured.

The instruments, run by hand:

```sh
python3 scripts/c_alloc_diff.py --attribute FILE…   # allocator events, interpreter against C
python3 scripts/store_diff.py --attribute FILE…     # every store's final bytes, the same pair
python3 scripts/store_diff.py --plant FILE          # its positive control
LOFT_LIB_BACKEND=c LOFT_HOSTED_VERIFY=1 LOFT_POISON_CLAIM=1 loft --interpret --lib DIR prog.loft
```

The two Python tools need the op-census build
(`cargo build --release --lib --bin loft --features op-census --target-dir target/op-census`).
