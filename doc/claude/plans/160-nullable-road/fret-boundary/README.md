<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The `(F-Ret)` boundary behind `element-over-param` — loft#1482

The cells that classified @PLN160's `element-over-param` divergence.  Each is a PAIR
(`<cell>.dense.loft` / `<cell>.opt.loft`) differing only in the return spelling, and each runs
`(F-Ret)`'s own stated test — *"a caller that mutates one call's result does not affect another
call's result"*:

```loft
bump(f(q));            // mutate one call's result
println("{f(q).a}");   // read another call's.  (F-Ret) wants 7; 8 means they are the same record.
```

| cell | how the returned element is reached | dense | nullable |
|---|---|---|---|
| `index` | `p[0]` | 7 | 7 |
| `matcharm` | `match p { [a, ..] => a, _ => … }` | **8** | 7 |
| `localbind` | `e = p[0]; e` | **8** | 7 |
| `loopvar` | `for e in p { return e; }` | 7 | 7 |
| `minted` | `S { a: p[0].a }` | 7 | 7 |
| `keepparam` | `fn f(s: S) -> S { s }` — a DECLARED parameter | 7 | 7 |
| `wa.loft` | the workaround: `r = f(q); bump(r);` | 7 | — |

Both backends agree in every cell.  `keepparam` is the control that keeps loft#1368's exemption
standing: handing a *declared* parameter straight back is fresh, because the caller copies it.
`minted` and `index` are the controls that stop the reading being "every struct return aliases".

The boundary is **a named local whose deps name a parameter, returned as the bare tail** — which is
wider than loft#1468's title (`localbind` is not a match arm) and lands on the DENSE side, the
opposite of what that issue recorded.
