<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 03 — Closure-element tuples (E4 × D1, D2)

**Status: open**

## Goal

Verify tuples whose elements are closures (`Type::Function(args, ret,
dep)`) round-trip through D1 and D2 with **interp/native byte-identical
stdout**.

The cell tests both **storing** the closure inside a tuple and
**calling it through the tuple element** (`t.0(...)`).  Captured state
must survive the round trip.

## Background

Closures in loft are represented as a function-pointer + captured
environment record (DbRef into a closure store).  See LIFETIME.md and
LOFT_LITERALS.md § Closures.  The tuple element occupies the slot size of a
`Type::Function` value — typically `fn-id (4) + dep DbRef (12)` =
16 bytes; verified per cell.

The risk is **dep tracking**: the tuple owns the captured environment
indirectly via the closure's dep list.  Scope exit on the tuple must
walk to the closure's dep correctly.  This phase is the first that
exercises that path through tuple storage.

## Cells closed

| Cell | Test name | Notes |
|---|---|---|
| E4×D1 store | `e4_d1_closure_local` | `t = (counter_closure, "tag")` |
| E4×D1 call | `e4_d1_closure_call` | `t.0(5)` returns expected value |
| E4×D1 swap | `e4_d1_closure_swap` | Two closures in a tuple, both called |
| E4×D2 arg | `e4_d2_closure_arg` | Pass closure-tuple as arg |
| E4×D2 return | `e4_d2_closure_return` | `#[ignore = "T1.8a"]` |
| E4 capture survives | `e4_d1_capture_survives` | Captured value read back through `t.0()` |

## Snippets (illustrative)

```loft
// e4_d1_closure_call
add5 = (x: integer) -> integer { x + 5 };
t = (add5, 99);
result = t.0(10);
print("{result},{t.1}\n");
assert(result == 15 && t.1 == 99, "closure called via tuple");
```

```loft
// e4_d1_capture_survives
captured = 42;
read_captured = () -> integer { captured };
t = (read_captured, "tag");
print("{t.0()}|{t.1}\n");
assert(t.0() == 42 && t.1 == "tag", "captured value through tuple");
```

```loft
// e4_d2_closure_arg
fn invoke(p: (fn(integer) -> integer, text)) -> integer {
    print("{p.1}\n");
    p.0(7)
}
sq = (n: integer) -> integer { n * n };
result = invoke((sq, "sq-tag"));
print("{result}\n");
assert(result == 49, "closure arg invoked");
```

## Pre-flight

```bash
# Does the parser accept Type::Function as a tuple element type today?
echo 'fn test() { f = (n: integer) -> integer { n + 1 }; t = (f, 0); print("{t.0(1)}\n"); }' \
    | cargo run --bin loft -- --interpret /dev/stdin 2>&1 | head -20
```

If parsing rejects the tuple-of-fn type, the fix lives in
`src/parser/types.rs` (or wherever `Type::Tuple` parsing recurses
into element types).  Likely already works — `Type::Function` is a
first-class type — but the dep-tracker side may need an
`owned_elements` extension.

## Likely fix subjects

- `data.rs::Type::owned_elements` — must include closure dep slots
  alongside text and reference slots.
- `scopes.rs:578` (the same stub T1.8c lives in) may need a closure
  arm so scope exit emits cleanup for closure-element tuples.
- Native codegen for tuple-stored closures: the closure dep DbRef
  layout must match interp byte-for-byte.

## Acceptance

- 5 unignored closure cells run green under cross-mode.
- 1 ignored cell (`e4_d2_closure_return`) carries the T1.8a tag.
- No regression in the existing closure test surface (`tests/lambdas.rs`
  if present, or the closure-related entries in `tests/expressions.rs`).
- Matrix + LIFETIME.md updated to mention closure-in-tuple ownership.
- `make ci` green.

## Cross-references

- [LIFETIME.md](../../../LIFETIME.md) — closure dep semantics
- [LOFT_LITERALS.md § Closures](../../../LOFT_LITERALS.md)
- `src/data.rs::Type::Function`, `Type::owned_elements`
- `src/scopes.rs:578` — tuple scope-exit gate
