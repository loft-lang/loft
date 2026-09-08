# @PLN155 phase 2a — the difference set, written down BEFORE the run

The plan's verify: *"the corpus diff must differ **only** in the cells phase 0 predicted —
that difference set is written down before the run."*

## What 2a does

Introduce `Own::Unknown` and return it from the TWO fail-opens in `Ownership::classify`'s
`Value::CallRef` arm — the ones whose own comment names the cure:

1. **unresolved fn-ref target** — `defs.fnref_targets` has no entry, or `u32::MAX`.
   Today: `return Own::Owned`.
2. **a callee that returns a BORROW whose base the caller cannot name** — the trailing
   `_ => Own::Owned` after `caller_arg_base` and `closure_capture_base` both answered
   `u16::MAX`, and the non-`Join` verdicts.

Then give EVERY reader of `Own` an explicit arm for `Unknown` that preserves exactly today's
behaviour.

## The prediction

**The corpus diff is EMPTY: `introspect_diff.sh` reads IDENTICAL 1412/1412.**

That is the whole prediction, and it is a strong one — it says the refactor is a pure
re-spelling of a verdict, with no reader's disposal changed. Anything non-empty is a reader I
mis-transcribed, not a discovery, and it names the file to look at.

## Why an empty diff is the RIGHT target for 2a, not a weak one

Phase 2's product is *"the permissive default becomes a verdict every caller must dispose of
explicitly."* That is a statement about the SOURCE, not about the emitted program: after 2a,
a reader that wants to treat an unknown verdict as owned has to say so, and a new reader
cannot inherit the permissive answer by writing `_ =>`.

Changing what a reader DOES with `Unknown` is a separate decision per reader, each with its
own measurement, and folding it into the same commit would make the byte-identical gate
unavailable exactly where it is most needed.

## What would falsify 2a

- a non-empty corpus diff (a reader's disposal moved without being chosen);
- `make licence-census` moving on its 40-file sample — the licence buckets read
  `ownership_evidence`, so a changed verdict would show there even where the emit did not;
- `o_proxy_check.py` collapsing again (it should be untouched — no `deps` read changes).

## Recorded before the run

Baselines to compare against, measured on `62502c1d1`:

- `introspect_diff.sh`: the run is before-vs-after on this change alone.
- `licence_census --limit 40`: oracle-derived 1157, minted 548, oracle-disagrees 288,
  proxy-alone 149, veto 57 (2199 frees).
- `o_proxy_check.py`: 30 positive, 9 of 30 reach a free, green.

---

## AMENDMENT, written before the run

Transcribing the readers turned up FIVE that absorb `Unknown` silently — `matches!(own,
Own::Owned)` and friends, where the new variant flips the answer without the compiler saying
so. Four are preserved by spelling `Own::Owned | Own::Unknown`, which keeps the empty-diff
prediction intact. They are the concrete instance of what phase 2 is for: the compiler catches
an exhaustive `match`, and these were the ones it could not.

The fifth is `ownership_cfg::run_licence_census` — phase 0's own instrument. It buckets
`!matches!(own, Own::Owned)` as `oracle-disagrees`, whose documented meaning is *"the oracle
answers Borrowed/Join"*. `Unknown` is not that, so folding it there would make the census say
something false.

**So the census gains a `no-answer` bucket, and its numbers WILL move. Predicted, before the
run, in one direction only:**

- the TOTAL stays 2199 on the 40-file sample and 80 325 over the corpus — no free appears or
  disappears;
- `oracle-disagrees` does NOT grow (that would be the wrong-meaning fold);
- whatever lands in `no-answer` comes out of `proxy-alone` and/or `oracle-derived`, because
  those are the two buckets a `CallRef` fail-open could previously reach — it answered `Owned`,
  and the evidence was `Fallback` (→ `proxy-alone`) or `Derived` (→ `oracle-derived`);
- `minted` and `veto` are untouched: both are decided before the verdict is read.

This makes phase 0's headline number MORE accurate, not different: a free whose licence rests
on a `CallRef` the oracle could not resolve was never really "the proxy alone" — it was "no
answer at all", which is the sharper statement.

**The emit prediction is unchanged and unweakened: IDENTICAL 1412/1412.**
