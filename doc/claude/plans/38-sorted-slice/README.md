
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# @PLN38 — Slicing & Comprehension on `sorted` / `index` (A8)

**Status: CLOSED 2026-07-09 — delivered + tested.**  Every A8 sub-feature ships,
in both layers: parser `src/parser/fields.rs::parse_key` (open-start / open-end /
sorted range / partial-key `idx[k1]` → `idx[k1..=k1]` rewrite) and runtime
`src/state/io.rs` `OpIterate` (index `on==1` / sorted `on==2` open-bound
handling).  Pinned per sub-feature in `tests/expressions.rs`
(`sorted_open_end_range`, `sorted_open_start_range`, `sorted_range_iteration`,
`sorted_range_comprehension`, `sorted_reverse_range`, `index_open_end_range`,
`index_reverse_range`, partial-key `sum_by_nr`) plus cross-mode scripts
`62-index-range-queries.loft`, `502-keyed-slice-for-only.loft`,
`25-sorted-enum-variant-range.loft`.  The design document below is retained as
the reference record.

Design for key-range slicing, open-ended bounds, partial-key match iteration, and
vector comprehensions over `sorted<T>` and `index<T>` collections.

## Contents
- [Current State](#current-state)
- [Feature Overview](#feature-overview)
- [Syntax Reference](#syntax-reference)
- [Runtime Foundations](#runtime-foundations)
- [Design: Open-Ended Bounds (A8.1)](#design-open-ended-bounds-a81)
- [Design: Range Slicing on `sorted` (A8.2)](#design-range-slicing-on-sorted-a82)
- [Design: Partial-Key Match Iterator on `index` (A8.3)](#design-partial-key-match-iterator-on-index-a83)
- [Design: Comprehensions on Key Ranges (A8.4)](#design-comprehensions-on-key-ranges-a84)
- [Design: Reverse Range Iteration (A8.5)](#design-reverse-range-iteration-a85)
- [Design: Match on Collection Results (A8.6)](#design-match-on-collection-results-a86)
- [Implementation Plan](#implementation-plan)

---

## Current State

| Operation | `sorted` | `index` |
|---|---|---|
| Exact key lookup `col[k]` | ✓ | ✓ (full key) |
| Range iteration `col[lo..hi]` | ✗ | ✓ (primary key only) |
| Range with secondary `col[lo..hi, sec]` | N/A | ✓ |
| Open-ended `col[lo..]` | ✗ | ✗ |
| Open-ended `col[..hi]` | ✗ | ✗ |
| Full-range `col[..]` | ✗ | ✗ |
| Partial key match iterator `col[k1]` (multi-key) | N/A | ✗ (returns null) |
| Comprehension `[for v in col[lo..hi] { }]` | ✗ | ✗ (no range on sorted) |
| Reverse range `rev(col[lo..hi])` | ✗ | ✗ |

**What already works at the runtime level** (confirmed in `src/codegen_runtime.rs`
`OpIterate`): `from` and `till` are `&[Content]`. The `key_compare` function in
`src/keys.rs` uses `zip(key.iter(), keys.iter())` — so passing a partial key array
(fewer entries than the collection's key count) already compares only the given prefix
fields, silently treating unspecified fields as unconstrained. This means the runtime
already supports partial-key ranges; only the parser changes are needed.

---

## Feature Overview

### A8.1 — Open-ended bounds

```loft
for v in col[lo..]  { }    // from lo to end of collection
for v in col[..hi]  { }    // from start to hi (exclusive)
for v in col[..]    { }    // entire collection as a range iterator
```

An empty `from` or `till` array in `OpIterate` signals "no constraint on this side".
The runtime uses `tree::first()` / `tree::last()` for index, and offset 0 / vector
length for sorted.

### A8.2 — Range slicing on `sorted`

```loft
s: sorted<Elm[-key]>
for v in s["M".."Z"] { }      // all keys in ["M", "Z") descending order
for v in s["Z".."M"] { }      // ascending (reversed direction for desc sort)
```

The parser currently emits `Value::Iter` with `OpIterate(on=1)` for `index` ranges.
It needs to do the same with `on=2` for `sorted` ranges — the runtime already handles
`on == 2` in `OpIterate`.

### A8.3 — Partial-key match iterator on `index`

```loft
idx: index<Elm[nr, -key]>
for v in idx[83] { }           // all elements where nr == 83 (any key value)
```

Currently `idx[83]` on a two-key index calls `OpGetRecord` with `nr=1`, which attempts
an exact tree find and returns null (wrong). The new behaviour: when `key_types.len() > 1`
and the user provides fewer keys than the index has, emit an iterator with
`from=[k1]` and `till=[k1]` and `inclusive=true` (i.e., `..=` semantics on the partial
prefix). The existing `zip`-based partial key comparison makes this work correctly.

### A8.4 — Comprehensions on key ranges

```loft
values = [for v in col["A".."M"] { v.value }]
active = [for v in col[lo..hi] if v.active { v }]
```

Comprehensions `[for ... { }]` already consume any `Value::Iter` produced by a `for`
source. Once the range subscript produces a valid iterator, comprehensions work without
parser changes.

### A8.5 — Reverse range iteration

```loft
for v in rev(col["A".."M"]) { }      // range, reverse key order
for v in rev(col[83..92, "Two"]) { } // existing range, reverse order
```

The `reverse_iterator` flag is already applied in `fill_iter` (adds bit 64 to the `on`
byte). The parser currently sets this flag only for `rev(col)` without a range. It needs
to also set it when `rev(...)` wraps a subscript expression that produces a range
iterator.

### A8.6 — `match` on collection results

```loft
// match on nullable single-element lookup
match col["key"] {
    null -> println("not found");
    elm  -> println("{elm.value}");
}

// match on element inside a range loop
for v in col[lo..hi] {
    match v.category {
        Category.A -> process_a(v);
        Category.B -> process_b(v);
    }
}
```

`match` already works on nullable values and on struct fields. No new parser or runtime
changes needed — this section documents what is already supported and tests it explicitly.

---

## Syntax Reference

### Full syntax table (after A8 complete)

| Expression | Context | Meaning |
|---|---|---|
| `col[k]` | `sorted<T[key]>` | Exact lookup → element or null |
| `col[k]` | `index<T[k1,k2]>` with `k` as full key | Exact lookup → element or null |
| `col[lo..hi]` | `sorted` or `index` | Range [lo, hi) → iterator |
| `col[lo..=hi]` | `sorted` or `index` | Range [lo, hi] inclusive → iterator |
| `col[lo..]` | `sorted` or `index` | From lo to end → iterator |
| `col[..hi]` | `sorted` or `index` | From start to hi (exclusive) → iterator |
| `col[..]` | `sorted` or `index` | Entire collection as range iterator |
| `col[k1]` | `index<T[k1,k2]>` (partial key) | All elements matching k1 → iterator |
| `col[lo..hi, sec]` | `index<T[k1,k2]>` | Range on k1, exact sec on k2 → iterator |
| `col[lo.., sec]` | `index<T[k1,k2]>` | From lo on k1, exact sec on k2 → iterator |
| `col[..hi, sec]` | `index<T[k1,k2]>` | To hi on k1, exact sec on k2 → iterator |
| `rev(col[lo..hi])` | `sorted` or `index` | Range in reverse order |
| `[for v in col[lo..hi] { expr }]` | comprehension | Build vector from range |
| `[for v in col[lo..hi] if pred { expr }]` | comprehension | Filtered range |

---

## Runtime Foundations

### `OpIterate` signature (unchanged)

```rust
pub fn OpIterate(
    stores: &Stores,
    data: DbRef,
    on: i32,        // 1=index, 2=sorted; +64=reverse, +128=inclusive-end
    arg: i32,       // index: fields offset; sorted: element size
    keys: &[Key],   // key field descriptors from the type schema
    from: &[Content], // lower bound key values (empty = no lower bound)
    till: &[Content], // upper bound key values (empty = no upper bound)
) -> i64
```

### Empty-array open-bound contract (new)

The only runtime change needed is to handle empty `from`/`till` arrays in `OpIterate`:

**For `on == 2` (sorted vector):**

```rust
2 => {
    let size = arg as u16;
    let start = if from.is_empty() {
        // no lower bound: start before the first element
        0u32
    } else {
        vector::sorted_find(&data, true, size, all, keys, from).0
    };
    let finish = if till.is_empty() {
        // no upper bound: end past the last element
        let len = all[data.store_nr as usize].get_int(data.rec, data.pos) as u32;
        len * size as u32 + size as u32  // one past the last
    } else {
        let (t, cmp) = vector::sorted_find(&data, ex, size, all, keys, till);
        if ex || cmp { t } else { t + 1 }
    };
    if reverse {
        pack_iter(start + size as u32, finish)  // adjust for reverse
    } else {
        pack_iter(if start == 0 { u32::MAX } else { start - 1 }, finish)
    }
}
```

*(Note: exact start/finish adjustment matches the existing non-open-ended logic.)*

**For `on == 1` (red-black tree / index):**

```rust
1 => {
    let fields = arg as u16;
    let store = crate::keys::store(&data, all);
    let start_node = if from.is_empty() {
        tree::first(&data, fields, all).rec
    } else {
        tree::find(&data, true, fields, all, keys, from)
    };
    let end_node = if till.is_empty() {
        // sentinel: u32::MAX means "no end limit" — OpStep stops when next() == 0
        u32::MAX
    } else {
        tree::find(&data, ex, fields, all, keys, till)
    };
    // ... existing pack_iter / prev/next adjustment logic ...
}
```

For the "no end limit" case in `OpStep`, when `finish == u32::MAX`, iteration must
continue until `tree::next()` returns a null node (the existing "past-the-end" sentinel).
Looking at the current code, `finish == u32::MAX` already means "no limit" in `OpStep`
(line 376 checks `finish == u32::MAX` as the empty collection guard). The open-ended
upper bound reuses this sentinel cleanly.

**Summary:** two special cases in `OpIterate`, zero changes to `OpStep`.

---

## Design: Open-Ended Bounds (A8.1)

### Parser changes — `parse_key` in `src/parser/fields.rs`

Currently `parse_key` always parses the lower bound expression first, then checks for
`..`. For open-start (`col[..hi]`), the parser must detect `..` *before* parsing any
expression.

```
Current parse_key flow:
  1. parse first expression → p (lower bound value)
  2. if has_token("..") → emit range iterator
  3. else → emit exact lookup

New flow:
  A. if peek is ".." or "..=" → open-start: p = Value::Null (empty from)
     else → parse first expression → p
  B. if has_token("..") (or "..=") → range path:
     - if peek is "]" or "," → open-end: n = Value::Null (empty till)
       else → parse second expression → n (upper bound)
     - emit range iterator with from=p, till=n
  C. else → exact lookup path (no range)
```

`Value::Null` in the `from` / `till` position signals the empty array to codegen:

```rust
// In codegen.rs, when building OpIterate key arrays:
fn encode_bound(v: &Value, from_vals: &mut Vec<Content>, ...) {
    if matches!(v, Value::Null) {
        // open bound — leave the array empty
    } else {
        // encode normally
    }
}
```

### Emitted IR shape

```
// col[lo..]  →  from=[lo], till=[]
Value::Iter(iter_var,
    init: v_set(iter, OpIterate(col, on, arg, keys, [lo], [])),
    step: OpStep(iter, col, on, arg),
    ...
)

// col[..hi]  →  from=[], till=[hi]
Value::Iter(iter_var,
    init: v_set(iter, OpIterate(col, on, arg, keys, [], [hi])),
    step: OpStep(iter, col, on, arg),
    ...
)

// col[..]  →  from=[], till=[]  (identical to `for v in col {}`)
Value::Iter(iter_var,
    init: v_set(iter, OpIterate(col, on, arg, keys, [], [])),
    step: OpStep(iter, col, on, arg),
    ...
)
```

---

## Design: Range Slicing on `sorted` (A8.2)

The only difference from the existing index range path is `on = 2` (sorted) vs `on = 1`
(index). The parser already dispatches through `fill_iter` which sets `on` based on the
collection's `Parts` variant. No new opcodes needed.

**Parser change:** in `parse_key`, the range path (`..` detected) currently calls
`fill_iter` which works for both `Index` and `Sorted`. The fix is to allow `..` to enter
the range path regardless of whether the collection is `sorted` or `index`. Currently the
check is correct — but there may be a guard that only allows `..` on `index`. Verify and
remove any such guard.

**Key ordering note:** `sorted<T[-key]>` stores elements in descending key order.
`col["M".."Z"]` on a descending collection will visit elements from `"M"` down to `"Z"`,
not up. The user must understand that the `lo..hi` bounds refer to key values, not
positions, and the traversal follows the stored order. Document this clearly.

---

## Design: Partial-Key Match Iterator on `index` (A8.3)

### Semantic rule

When a subscript on `index<T[k1, k2, ...]>` provides **fewer keys than the index has**
and **no `..` is present**:

| Keys provided | `..` | Result |
|---|---|---|
| Full (`nr` == key count) | no | Exact lookup → element or null |
| Partial (`nr` < key count) | no | **Partial-prefix iterator** — all elements matching the given prefix |
| Any | yes | Range iterator (existing) |

### IR emitted for `col[k1]` on 2-key index

Equivalent to `col[k1..=k1]` with only the partial key in `from` and `till`:

```rust
// from = [k1], till = [k1], inclusive = true
let start = v_set(iter, self.cl("OpIterate", &[col, on, arg, keys, [k1], [k1], /*incl*/]));
```

Since `key_compare` uses `zip`, providing `[k1]` in both `from` and `till` means:
- Start: first element where k1 >= k1_value (i.e., k1 == k1_value, since < k1_value is excluded)
- End: last element where k1 <= k1_value (inclusive)

All secondary key values pass through unconstrained.

### Disambiguation at parse time

In `parse_key`, after collecting the user-supplied keys:

```rust
if key_types.len() > 1 && nr < key_types.len() && !self.lexer.peek_token("..") {
    // Partial key — emit inclusive range iterator with from=till=key
    // (reuse existing range-emit path with inclusive flag and same values for both bounds)
    self.emit_partial_match_iter(code, typedef, key_types, &key, iter, inclusive=true);
} else if self.lexer.has_token("..") {
    // Existing range path
    ...
} else {
    // Exact lookup — existing path
    *code = self.cl("OpGetRecord", ...);
}
```

---

## Design: Comprehensions on Key Ranges (A8.4)

Comprehension syntax `[for v in source { expr }]` is parsed in `src/parser/vectors.rs`.
The `source` is any expression that produces an iterator type. Range subscripts
(`col[lo..hi]`) already produce `Type::Iterator<ElementType>` from `parse_key` — so
comprehensions over key ranges work as soon as the range parsing itself works.

**Test pattern to verify:**

```loft
struct Elm { nr: integer, value: integer }
struct Db { idx: index<Elm[nr]> }
fn main() {
    db = Db { idx: [Elm{nr:1,value:10}, Elm{nr:2,value:20}, Elm{nr:3,value:30}] };
    vals = [for v in db.idx[1..3] { v.value }];
    assert(vals == [10, 20], "range comprehension");
    highs = [for v in db.idx[2..] { v.value }];
    assert(highs == [20, 30], "open-end comprehension");
}
```

---

## Design: Reverse Range Iteration (A8.5)

`rev()` wrapping a range subscript requires the `reverse_iterator` flag to be set *before*
`fill_iter` is called for the range subscript. The current flow sets the flag in
`parse_in_range` when `rev(col)` is detected (collection, no range). For
`rev(col[lo..hi])`, the `rev(` is parsed first (sets `reverse = true`), then
`col[lo..hi]` is parsed as a subscript. The `reverse_iterator` flag must propagate
through `parse_index` into `parse_key` → `fill_iter`.

**Parser change:** in `parse_index`, if the result is a range iterator, set
`self.reverse_iterator = true` before calling `fill_iter` if the caller has indicated
`rev(...)`.

The cleanest approach: accept `rev()` wrapping any subscript expression, not just bare
collection identifiers, by detecting the `rev(` before calling `parse_part` for the
subscript.

---

## Design: Match on Collection Results (A8.6)

### Single-element nullable match

```loft
match col["key"] {
    null -> println("not found");
    elm  -> println("{elm.value}");
}
```

`col["key"]` returns `elm_type` (nullable). `match` already handles nullable struct types
— `null` arm catches the null case, the variable arm (`elm`) binds the non-null value.
No implementation needed. **Document and test only.**

### Match inside range loop

```loft
for v in col[lo..hi] {
    match v.status {
        Status.Active -> process(v);
        Status.Archived -> archive(v);
        _ -> skip(v);
    }
}
```

`v` is a loop variable of the element struct type. `match` on any field works today.
**Document and test only.**

### Exhaustiveness note

Loft's `match` emits a warning (not error) for non-exhaustive enum matches. This applies
identically inside range loops. Document that adding `_` or all variants suppresses the
warning.

---

## Implementation Plan

| ID | Step | File(s) | Effort |
|----|------|---------|--------|
| A8.1a | Open-end bound: `col[lo..]` — parser detects missing upper bound after `..` | `fields.rs` | S |
| A8.1b | Open-start bound: `col[..hi]` — parser detects `..` before any expression | `fields.rs` | S |
| A8.1c | Full range `col[..]` — both bounds absent | `fields.rs` | XS |
| A8.1d | Runtime: empty `from`/`till` handling in `OpIterate` (on==2 sorted) | `codegen_runtime.rs` | S |
| A8.1e | Runtime: empty `from`/`till` handling in `OpIterate` (on==1 index / tree) | `codegen_runtime.rs` | S |
| A8.1f | Bytecode generator: encode `Value::Null` bound as empty Content array | `codegen.rs` / `dispatch.rs` | S |
| A8.2  | Range slicing on `sorted`: verify `fill_iter` emits `on=2` for `..` on sorted | `fields.rs` | XS |
| A8.3  | Partial-key match iterator: detect `nr < key_types.len()` without `..` | `fields.rs` | M |
| A8.4  | Comprehensions on key ranges: test coverage only (no parser changes expected) | `tests/` | S |
| A8.5  | Reverse range: propagate `reverse_iterator` through subscript path | `fields.rs`, `objects.rs` | S |
| A8.6  | Match on collection results: tests + documentation only | `tests/`, `doc/` | S |
| A8.T  | Test suite: `tests/docs/10-sorted.loft`, `tests/docs/11-index.loft` additions | `tests/docs/` | M |

**Total effort:** M (one focused sprint; no new opcodes, no schema changes)

**Dependencies:** none (all work is in the parser and the `OpIterate` runtime function)

---

## Test Plan

### `tests/docs/10-sorted.loft` additions

```loft
// ## Range Slicing
// Visit elements whose key falls in a range.
s: sorted<Elm[-key]>  // populated with One/Two/Three/Four/Zero
sum = 0;
for v in s["T".."U"] {   // "Three", "Two" (descending order: T ≥ key > U)
    sum += v.value;
}
assert(sum == 5, "sorted range slice: Three(3)+Two(2)");

// ## Open-Ended Slices
// From "T" to the end of the collection (descending: T down to "F"our)
vals = [for v in s["T"..] { v.key }];
assert(vals == ["Three", "Two"], "sorted open-end slice");

// From start to "T" (exclusive: all keys > "T" in descending order)
vals2 = [for v in s[.."T"] { v.key }];
assert(vals2 == ["Zero"], "sorted open-start slice");

// ## Reverse Range
rev_sum = 0;
for v in rev(s["T".."Z"]) {   // ascending: Two then Three
    rev_sum = rev_sum * 10 + v.value;
}
assert(rev_sum == 23, "sorted reverse range");
```

### `tests/docs/11-index.loft` additions

```loft
// ## Open-Ended Range Queries
sum_open = 0;
for v in db.map[83..] {    // nr >= 83: Three, Four, Five, One (sorted by nr, then -key)
    sum_open += v.value;
}
assert(sum_open == 11, "open-end range: {sum_open}");

sum_prefix = 0;
for v in db.map[..92] {   // nr < 92: Six, Three, Four, Five
    sum_prefix += v.value;
}
assert(sum_prefix == 18, "open-start range: {sum_prefix}");

// ## Partial-Key Match Iterator
partial_sum = 0;
for v in db.map[83] {     // all elements where nr == 83
    partial_sum += v.value;
}
assert(partial_sum == 12, "partial key match: Three(3)+Four(4)+Five(5)");

// ## Comprehension on Range
values = [for v in db.map[63..92] { v.value }];
assert(values == [6, 3, 4, 5], "range comprehension");

// ## Match Inside Range Loop
labels = "";
for v in db.map[83..101] {
    labels += match v.key {
        "Three" -> "T";
        "Four"  -> "F";
        "Five"  -> "f";
        _       -> "?";
    };
}
assert(labels == "TFf", "match inside range loop");
```

---

## See also
- [PLANNING.md](../../PLANNING.md) — A8 entry (when added)
- [DATABASE.md](../../DATABASE.md) — `sorted` / `index` storage layout; `OpIterate` / `OpStep`
- [LOFT_DATA.md](../../LOFT.md) § Key-based collections — user-facing reference
- `src/parser/fields.rs` — `parse_key`, `fill_iter`
- `src/codegen_runtime.rs` — `OpIterate`, `OpStep`
- `src/tree.rs` — `find`, `first`, `last`, `next`, `previous`
