# @PLN91 — `rev(range)` beyond a constant exclusive range (prepared subject)

**The family:** "a reversed range that is inclusive, open or not constant" (6 programs at try
5d712c497, 3 held by it alone, among them tests/scripts/07-control-flow).  The port's
`rev_range_for` (el_ex_pattern ~1824) carries `rev(lo..hi)` with CONSTANT bounds, `lo >= 0`
and `hi` exclusive, and refuses every other reversed range by name (~1841).  The sites are
`rev(1..=5)` (inclusive), `rev(a..b)` / `rev(0..n)` / `rev(p..5)` (a bound not constant) and
`rev(a..=b)`.

**The rule:** `@FR-I-Range` / `@FR-I-RangeIncl`: a range iterates its integers, `b` excluded or
included.  `rev(r)` walks the same values from the last to the first.  `@FR-I-RangeNull`: a null
bound makes the range empty.  `@FR-R-Counter`: the counters (`#index`, the `next` counter) are
never the sentinel, so the loop variable is never null.

## Rust's forms, measured on try 5d712c497

```
INCLUSIVE, constant bounds   tests/scripts/07-control-flow.loft:24   for i in rev(1..=5) { t = t * 10 + i; }
  [24] {#For block
     { !! INSERT  _next_1: integer = 5
                  i#2#index: integer = OpConvIntFromNull() };
     loop {#For loop
       i#2 = {#Iter range
         if OpLtInt(_next_1, 1) break(0) else null;            <- stop below the low bound
         i#2#index = _next_1;
         _next_1 = OpMinInt(_next_1, 1);                        <- a counter one below, read next pass
         i#2#index };
       { t = OpAddInt(OpMulInt(t, 10), i#2) } } }

EXCLUSIVE, bounds not constant   tests/scripts/157-next-counter.loft:28   fn c15(a, b) { … for i in rev(a..b) … }
  {#For block
     { !! INSERT  _range_start_1: integer = a
                  _range_end_1: integer = b
                  {#Range null guard                            <- a null bound makes the range empty
                     if OpNot(OpConvBoolFromInt(_range_start_1)) {#empty range _range_start_1 = 1; _range_end_1 = 0 } else null;
                     if OpNot(OpConvBoolFromInt(_range_end_1))   {#empty range _range_start_1 = 1; _range_end_1 = 0 } else null; }
                  i#index: integer = _range_end_1 };
     loop {#For loop
       i = {#Iter range
         i#index = OpMinInt(i#index, 1);                         <- one below FIRST (hi is exclusive)
         if OpLtInt(i#index, _range_start_1) break(0) else null;
         i#index };
       … } }
```

The inclusive form keeps a separate `_next_N` counter and reads it before it steps.  The
exclusive form steps the index first, then tests it.  Measure `c16` (`rev(a..=b)`, an inclusive
range with bounds not constant, 157-next-counter.loft:29) for the combination of the two: the
null guard of the second and the `_next_N` counter of the first.

## What to code

Extend `rev_range_for` past its constant-exclusive case: the inclusive form with a `_next_N`
counter, the not-constant bounds held in `_range_start_N` / `_range_end_N` behind the `Range null
guard` block, and their combination as `c16` shows it.  Keep the constant-exclusive form as it is.

## Probes (the three sole programs, each with hand-computed answers)

```
kr-try/tests/scripts/07-control-flow.loft:24                              rev(1..=5)
kr-try/tests/scripts/a-counted-loop-variable-shares-its-index-slot.loft:137   rev(1..=5)
kr-try/tests/scripts/store_digest_b0.loft:133                             rev(0..n)
(not sole) kr-try/tests/scripts/157-next-counter.loft:28, :29             rev(a..b), rev(a..=b)
```
