
# Loft Standard Library Reference

This document describes all public functions, constants, and types available in the loft standard library.
It states the library as it stands; a limitation the library has since lost is recorded in
[STDLIB-history.md](STDLIB-history.md), not here.

## Contents
- [Implementation notes](#implementation-notes)
- [Types](#types)
- [Math](#math)
- [Text](#text)
- [Collections](#collections)
- [Keyed collections (hash / index / sorted)](#keyed-collections-hash--index--sorted)
- [Output and Diagnostics](#output-and-diagnostics)
- [Logging](#logging)
- [File System](#file-system) → [STDLIB_FILES.md](STDLIB_FILES.md)
- [Parallel, Reflection, Environment, Time, Random](#parallel-reflection-environment-time-random) → [STDLIB_RUNTIME.md](STDLIB_RUNTIME.md)

---

## Implementation notes

Standard library functions fall into two implementation categories:

- **Loft-implemented** — defined in `default/01_code.loft`, `default/02_files.loft`, or `default/03_text.loft` using the loft language itself. These have a normal function body.
- **Native (Rust)** — declared in the default library with a `#rust "..."` annotation and implemented as hand-written Rust functions in `src/native.rs`. These handle OS interaction and operations that cannot be expressed in loft (file I/O, environment variables, string classification, etc.).

See [INTERNALS.md](INTERNALS.md) for the full list of native functions, their Rust names, and the naming convention (`n_<func>` for globals, `t_<N><Type>_<method>` for methods).

---

## Types

The primitive types built into loft — their widths are [formal/layout.md](formal/layout.md) `(L-Scalar)`, and a ranged integer's `(L-Narrow)`.

| Type        | Size   | Description |
|-------------|--------|-------------|
| `boolean`   | 1 byte | True or false value. |
| `integer`   | 8 bytes | 64-bit signed integer. |
| `single`    | 4 bytes | 32-bit floating-point. Good for graphics and performance-sensitive math. |
| `float`     | 8 bytes | 64-bit floating-point. Use when precision matters. |
| `text`      | —      | UTF-8 string. |
| `character` | 4 bytes | A single Unicode code point. |

**Integer subtypes** (ranged aliases for compact storage):

| Type  | Range           | Size   |
|-------|-----------------|--------|
| `u8`  | 0 – 255         | 1 byte |
| `i8`  | -128 – 127      | 1 byte |
| `u16` | 0 – 65535       | 2 bytes |
| `i16` | -32768 – 32767  | 2 bytes |
| `i32` | full integer    | 4 bytes |

Use the sized subtypes in struct fields to reduce memory usage. They behave as `integer` in expressions.

---

## Math

Functions for numeric computation. All trigonometric functions work in radians.

In the tables below, **N** = `integer | single | float` for general functions, and **F** = `single | float` for float-only functions. Use `single` for speed, `float` for precision.

### Constants

| Name | Value | Description |
|------|-------|-------------|
| `PI` | 3.14159… | Ratio of a circle's circumference to its diameter. |
| `E`  | 2.71828… | Euler's number, base of natural logarithms. |

### General (N = integer | single | float)

These are **method-or-free** functions: the first parameter is `self`, so each is callable as a
method (`x.abs()`) or free (`abs(x)`) — the same function either way.

| Function | Description |
|----------|-------------|
| `abs(self: N) -> N` | Absolute value. |
| `min(self: N, b: N) -> N` | Smaller of two values. Returns null if either is null. |
| `max(self: N, b: N) -> N` | Larger of two values. Returns null if either is null. |
| `clamp(self: N, lo: N, hi: N) -> N` | Clamps to `[lo, hi]`. Returns null if any arg is null. |
| `approx(self: F, b: F, eps: F) -> boolean` | True when `a`/`b` (F = single \| float) differ by ≤ `eps`. `==` on float/single is **exact IEEE** (@PLN102); use `approx` for tolerance. A null (NaN) operand → false. |
| `floor_mod(self: integer, divisor: integer) -> integer?` | Floor modulo: the remainder that takes the sign of the **divisor**, so it lands in `[0, divisor)` for a positive `divisor`. `%` truncates and keeps the **dividend's** sign (`-1 % 3 == -1`); `floor_mod` wraps (`(-1).floor_mod(3) == 2`) — use it for circular indexing (`grid[(i - 1).floor_mod(w)]`). `floor_mod(x, 0)` is null (like `%`). Integer-only. |

### Rounding and roots (F = single | float)

| Function | Description |
|----------|-------------|
| `floor(v: F) -> F` | Round down to nearest integer value. |
| `ceil(v: F) -> F` | Round up to nearest integer value. |
| `round(v: F) -> F` | Round to nearest (half rounds away from zero). |
| `sqrt(v: F) -> F` | Square root. |

### Power and Logarithm (F = single | float)

| Function | Description |
|----------|-------------|
| `pow(base: F, exp: F) -> F` | Raises `base` to the power `exp`. |
| `exp(v: F) -> F` | Raises E to the power `v`. |
| `ln(v: F) -> F` | Natural logarithm. |
| `log(v: F, base: F) -> F` | Logarithm in the given `base`. |
| `log2(v: F) -> F` | Base-2 logarithm. |
| `log10(v: F) -> F` | Base-10 logarithm. |

### Trigonometry (F = single | float, angles in radians)

| Function | Description |
|----------|-------------|
| `cos(angle: F) -> F` | Cosine. |
| `sin(angle: F) -> F` | Sine. |
| `tan(angle: F) -> F` | Tangent. |
| `acos(v: F) -> F` | Arc cosine — returns angle whose cosine is `v`. |
| `asin(v: F) -> F` | Arc sine — returns angle whose sine is `v`. |
| `atan(v: F) -> F` | Arc tangent — returns angle in (-PI/2, PI/2). |
| `atan2(y: F, x: F) -> F` | Arc tangent of `y/x`, preserving quadrant. |

---

## Text

Functions for working with `text` (UTF-8 strings) and `character` values.

### Length

| Function | Description |
|----------|-------------|
| `len(v: text) -> integer` | Number of characters (Unicode code points) in the text — the human count. |
| `size(v: text) -> integer` | Number of bytes in the text — the bound for byte-positioned `s[i]`, slices, and `find`/`rfind`. |
| `len(v: character) -> integer` | Byte length of the character's UTF-8 encoding (1–4). |

### Searching

| Function | Description |
|----------|-------------|
| `find(self: text, value: text) -> integer?` | Returns the byte index of the first occurrence of `value`, or **null** if not found (the type is honest about the not-found case — @PLN102). |
| `rfind(self: text, value: text) -> integer?` | Returns the byte index of the last occurrence of `value`, or **null** if not found (@PLN102). |
| `contains(self: text, value: text) -> boolean` | Returns true if `value` appears anywhere in `self`. |
| `starts_with(self: text, value: text) -> boolean` | Returns true if `self` begins with `value`. |
| `ends_with(self: text, value: text) -> boolean` | Returns true if `self` ends with `value`. |

### Taking part of a text (substring / substr / slice)

loft has no `substr` or `substring` function. A part of a text is a **slice**, written with
a range:

<!-- from tests/reference/text-slices.loft -->
```loft
s = "abcdef";
a = s[1..3];                // "bc" — from byte 1 up to, not including, byte 3
b = s.char_slice(1, 3);     // "bc" — the same, counting CHARACTERS instead of bytes
```

Which one you want depends on where the numbers came from:

| you have | use | why |
|---|---|---|
| numbers from `find`, `rfind`, `size`, `byte_at` | `s[a..b]` | those are byte positions, and so is the slice |
| a count of CHARACTERS (a width to fit, a column, a caret) | `s.char_slice(a, b)` | `len` counts characters, and a character can be several bytes |

Getting this the wrong way round is the commonest text bug in this stack, and it is silent
on ASCII: a character count used as a byte range fits fewer characters than it measured, so
the text comes back short. `"héllo"[0..4]` is `"hél"`; `"héllo".char_slice(0, 4)` is
`"héll"`. See [`char_slice`](#bytes-and-code-points) for the full rule, and
[`size` vs `len`](#length) for the two counts.

Both ends clamp, a reversed range gives `""`, and a negative bound counts from the end.

### Transformation

| Function | Description |
|----------|-------------|
| `replace(self: text, value: text, with: text) -> text` | Returns a copy of `self` with every occurrence of `value` replaced by `with`. |
| `to_lowercase(self: text) -> text` | Returns a lowercase copy. |
| `to_uppercase(self: text) -> text` | Returns an uppercase copy. |
| `trim(self: text) -> text` | Removes leading and trailing whitespace. Use when processing user input or file content. |
| `trim_start(self: text) -> text` | Removes leading whitespace only. |
| `trim_end(self: text) -> text` | Removes trailing whitespace only. |
| `split(self: text, separator: character) -> vector<text>` | Splits `self` on every occurrence of `separator` and returns the parts as a vector. |
| `join(self: vector<text>, separator: text) -> text` | Concatenates all elements of `self` with `separator` between each pair. Inverse of `split`. |

### Iterating over text

`for c in some_text` yields one `character` per UTF-8 code point — exactly `len(s)` of
them, and the character at each position is the same value `s[…]` reads there.  The
count is a fact about the text, never about the characters in it: a text carrying a
NUL (`text_from_bytes([65, 0, 66])`) yields all three, with the NUL position reading
as `null` (guard `tests/scripts/text-nul-iteration-755.loft`).  A NUL therefore round-trips
through `byte_at`, not through iteration; see
[CAVEATS.md](CAVEATS.md#accepted-trade-offs-not-scheduled-for-change).

The one text this does not describe is loft's **null text**, which IS the one-byte NUL
string: `size` answers 1 for it, and it yields nothing.

Inside the loop body two positional attributes are available:

| Attribute | Type      | Meaning                                                          |
|-----------|-----------|------------------------------------------------------------------|
| `c#index` | `integer` | Byte offset of the **start** of the current character in the string. |
| `c#next`  | `integer` | Byte offset immediately **after** the current character (= start of next char). |

These satisfy: `c#next == c#index + len(c)`.

Example — split on a separator character without using `split()`:
```
parts = [];
p = 0;
for c in path {
    if c == '/' {
        parts += [path[p..c#index]];
        p = c#next;
    }
}
```

### Bytes and code points

Text is UTF-8, so a `text` has two lengths and two ways in. `len()` counts
CHARACTERS and `size()` counts BYTES; `text[i]` decodes the code point *containing*
byte `i`, walking back through continuation bytes. These four are the explicit
routes between the two views.

| Function | Description |
|----------|-------------|
| `char_slice(self: text, from: integer, to: integer) -> text` | The characters `[from, to)` — a slice that counts CHARACTERS where `self[a..b]` counts BYTES. ⚠ **The cure for the commonest text bug in this stack**: a count computed from `len` and applied as a byte range fits FEWER characters than it measured, and loft snaps the cut outward, so the text comes back SHORT rather than corrupt — silently, and only when it is not ASCII. `"héllo"[0..4]` is `"hél"`; `"héllo".char_slice(0, 4)` is `"héll"`. Use it wherever a character count becomes a cut (fitting a label, wrapping, a caret or selection, truncating); use `self[a..b]` only when the numbers came from `find` / `size` / `byte_at`. Negative bounds count from the end as a byte slice's do; both ends clamp; a reversed range is `""`. |
| `byte_at(self: text, i: integer) -> integer` | The raw BYTE at byte offset `i` as 0–255, `0` out of bounds. A pure O(1) read — unlike `text[i]`, no UTF-8 decode — for ASCII-heavy scanning (tokenisers, regex-like loops), ~5–10× faster there. |
| `text_from_bytes(bytes: vector<u8>) -> text` | Build a text from raw UTF-8 bytes — the inverse of `byte_at`. For binary decoders that assemble a buffer and need text back. Bytes that are not valid UTF-8 yield `""` (never a crash), so validate first if you must tell "empty input" from "invalid bytes". Carries an embedded NUL. |
| `chr(cp: integer) -> text` | Build a one-character text from a Unicode CODE POINT — the inverse of the `ch as integer` that iteration gives. `chr(65)` → `"A"`, `chr(20013)` → `"中"`, `chr(128512)` → `"😀"`. For decoding an escape (`\u{…}`, an HTML entity) or reassembling text a code point at a time. |
| `ch as integer` | The code point of a `character` (via `as i32?` for a nullable). The direction that already existed; `chr` is its inverse. |

⚠ **A code point that names no character gives `""`, not an error** (C80): a
surrogate (`D800`–`DFFF`), anything past `U+10FFFF`, a negative number — and `0`,
because `character` uses 0 as its null and text ITERATION STOPS at a NUL, so a
NUL built by `chr` could not be read back by the loop it is the inverse of. The
byte route still carries one: `text_from_bytes([0])` is one byte long.

⚠ **`char_slice` was missing for longer than that, and three trees paid for it.**
`text2d` hand-rolled `take_chars` only after `fit_text` had shipped cutting a
character count as a byte range; `stage`'s text field needed the same walk twice;
and moros's `lavition_ui/src/font.loft` still carries it across **seven** call
sites — every button label, every hotkey, every list entry, the status line, the
subject line and both verb slots, green the whole time because every one of its
tests is ASCII. Two independent hand-rolls is this project's admission test for a
primitive belonging one level down; three is late.

### Character Classification

These functions return true only if **every character** in the text satisfies the condition.
The single-`character` variants test one code point.

| Function | Description |
|----------|-------------|
| `is_lowercase(self: text/character) -> boolean` | All characters are lowercase letters. |
| `is_uppercase(self: text/character) -> boolean` | All characters are uppercase letters. |
| `is_numeric(self: text/character) -> boolean` | All characters are numeric digits (Unicode numeric, not just ASCII 0–9). |
| `is_alphanumeric(self: text/character) -> boolean` | All characters are letters or digits. |
| `is_alphabetic(self: text/character) -> boolean` | All characters are alphabetic. |
| `is_whitespace(self: text) -> boolean` | All characters are whitespace. |
| `is_control(self: text) -> boolean` | All characters are control characters. |

### Joining

| Function | Description |
|----------|-------------|
| `join(parts: vector<text>, sep: text) -> text` | Joins the elements of `parts` with `sep` between each consecutive pair. Returns `""` for an empty vector. Use to build comma-separated lists, path segments, or any delimited output. |

---

## Collections

Operations on `vector<T>` — the primary ordered collection type.

| Function | Description |
|----------|-------------|
| `len(v: vector) -> integer` | Number of elements in the vector. Use in loop bounds: `for i in 0..v.len()`. |
| `reserve(v: vector, n: integer)` | Give `v` room for `n` elements so filling it does not repeatedly reallocate. Changes capacity only — never `len(v)`, its contents, or anything holding it — and an `n` at or below the current capacity does nothing. Also takes a `hash` (see below). |

**When `reserve` is worth it — it is the INTERLEAVING that decides, not the
number of vectors.** Appending grows a vector by doubling, so filling one of N
costs about log N reallocations, each claiming a fresh block and orphaning the
old one, and leaves the last block up to twice the length. A grow can extend in
place when the block after it is free, so what costs is how often growth
*alternates* between vectors: alternate on every element and no grow ever extends
in place, so every step copies.

Many vectors growing at once is not enough on its own. Fed by sorted or
spatially-coherent input, each vector grows in long runs and rarely reallocates
against a neighbour, and there is little overshoot to reclaim — `reserve` then
costs a counting pass and buys nothing. Measured by the consumer that asked for
it (loft#710): a synthetic generator switching collection on *every* element went
3,816,152 → 2,609,808 bytes (−31.6%), while their real OSM generator — which
switches tile only **3.8%** of the time, because the input arrives in roughly
spatial order — moved +0.7%, inside its own run-to-run noise.

So reach for it when growth genuinely interleaves; measure before keeping it when
your input is ordered. The cost it removes is also *persisted*: a store written
with `store_persist_bind` carries the claimed capacity, not the length, so on the
interleaved shape the file went from 1.65× its payload to 1.13× for identical
data.

<!-- from tests/reference/reserve.loft -->
```loft
for tile in tiles { reserve(tile.points, expected_count(tile)); }
for feature in stream { tiles[feature.tile].points += [feature.point]; }
```

An estimate is fine: reserving too little only means the ladder resumes from
there, and too much costs the unused tail until the vector is copied.

**`reserve(h, n)` also takes a `hash`**, where it sizes the bucket table instead
of an element block. Here it pays on the plain shape — no interleaving needed —
because a hash rebuilds its whole table every time it is half full, re-bucketing
every entry it already holds:

<!-- from tests/reference/reserve.loft -->
```loft
cache: hash<Entry[key]> = [];
reserve(cache, expected_rows);
for row in rows { cache += Entry { key: row.id, value: row.value }; }
```

Filling a million-entry hash rebuilds the table 18 times without it. Measured on
`--native-release`, 1M `integer` keys (2026-09-21): **240 → ~120 ms**, and the finished
table is **a third smaller** (11.5 MB → 8.0 MB) — the growth ladder doubles *past* the
trigger and lands at load 0.35, while a reserved table sits at the 0.5 it asked for. So
it buys time and memory at once.

Same contract as the vector form: capacity only. It never changes `len(h)`, the
records, or which keys are found; an `n` the table already covers does nothing;
and reserving a hash that already holds entries re-buckets them with the *same*
seed, so every key stays findable. Reserving too little just means the ladder
resumes from there.

`sorted`, `index`, `spatial` and `trie` have no capacity to set and are refused
with a message that says so.

### Aggregates

| Function | Description |
|----------|-------------|
| `sum<T: Addable>(v: vector<T>, init: T? = null) -> T` | Sum of all elements. `init` is the identity to start from; leave it out and the element type's own zero is used (`0`, `0.0`) — `text` is not `Addable`. |
| `sum_of(v: vector<integer>) -> integer` | Superseded by `sum` — kept working. Sum of all elements; returns 0 for an empty vector. |
**A bound declares the MINIMUM, and the rest derives.** `Ordered` declares `op <` alone and
`Equatable` declares `op ==` alone, so a user type satisfies either by defining one method —
and inside a generic all six comparisons work anyway:

| written | resolved as |
|---|---|
| `a > b` | `b < a` |
| `a <= b` | `!(b < a)` |
| `a >= b` | `!(a < b)` |
| `a != b` | `!(a == b)` |

Each derivation evaluates every operand exactly once, and each agrees with the concrete
operator on every value the bound's types can hold.

| `min_of<T: Ordered>(v: vector<T>) -> T?` | Smallest element, or **null** when the vector is empty (the type is honest about the empty case — @PLN102). |
| `max_of<T: Ordered>(v: vector<T>) -> T?` | Largest element, or **null** when the vector is empty (@PLN102). |

### Tree traversal — `tree_walk` and the `Walkable` interface

`tree_walk<T: Walkable>(root: T, cap: integer) -> vector<T>` visits a tree
without recursion and returns the visitation order — breadth-first (level
order): every parent before its children, siblings left to right.  A type
opts in by satisfying `Walkable`:

<!-- from default/01_code.loft -->
```loft
pub interface Walkable {
  fn children(self: Self) -> vector<Self>
}
```

A type with a `children` method satisfies it, and `tree_walk` walks it breadth-first, capped:

<!-- from tests/reference/tree-walk.loft -->
```loft
struct Node { val: integer, kids: vector<Node> }
fn children(self: Node) -> vector<Node> { return self.kids; }
```
<!-- from tests/reference/tree-walk.loft -->
```loft
for n in tree_walk(root, 1000) { seen += "{n.val} "; }
```

`cap` bounds the visited-node count, so the walk is total on any input —
an over-cap (or cyclic) structure yields the first `cap` nodes.  The
recursion-free shape is the sanctioned tree traversal for sandboxed code
(sandbox admission rejects recursion and unbounded loops; consuming the
result is an ordinary bounded `for`).

Vectors are grown by appending with `+=` and elements are accessed by index. Removal and insertion are handled by the parser's built-in operators.

| Operation | Description |
|-----------|-------------|
| `v += [elem]` | Append one element. |
| `v.remove(i)` | Remove element at index `i` (negative counts from end); returns `boolean`. |
| `v#remove` | Remove current element inside a `for ... if ...` loop. |

---

## Keyed collections (hash / index / sorted)

All three keyed collection types share a common lookup and removal syntax handled by the parser, not the stdlib:

| Syntax | Description |
|--------|-------------|
| `c[key]` | Look up element by key; returns the element or `null` if absent. |
| `c[key] = null` | Remove the element with that key; no-op if absent. |
| `e#remove` | Remove current element during a `for ... if ...` iteration. |

These are parser-level operations; they compile to `OpGetRecord`, `OpHashRemove`, and `OpRemove` respectively. There are no corresponding callable functions.

### `spatial<T[x,y]>` / `spatial<T[x,y,z]>` — spatial keyed collection

A keyed collection backed by a Morton/Z-order radix tree, with 1–3
coordinate key fields (@PLN48):

| Syntax | Description |
|--------|-------------|
| `xs: spatial<Mob[x, y]> = [];` | Construct; also legal as a struct field (`mobs: spatial<Mob[x,y]>`). |
| `xs += [Mob{x: 1, y: 2}];` | Append. |
| `for m in xs { … }` | Iterate in the tree's natural Morton/Z-order — no sort (unlike `hash`, which sorts via its internal ordered index). |
| `xs.len()` | Element count — O(1), reads the tree's cached length word. |
| `m = xs[x, y]` | Look up the record at exactly that point; `null` when nothing sits there. Note the coordinates are separate subscripts (`xs[3, 6]`), not the parenthesised pair the range forms use. |
| `xs[x, y] = mob` | Insert-or-replace a copy of `mob` at that point: the subscript names the place, and the copy's coordinate fields are set to it, as for `hash`/`sorted`/`index` (`Col-Assign`). |
| `xs[x, y] = null` | Remove the record at that point; a no-op when the point is empty. |
| `xs[(x,y)..]` | Walk OUTWARD from that point, nearest first; caller `break`s to stop. Approximate — ordered by Morton distance, so a truly-near point can arrive a little late. |
| `xs[(x,y)..:n]` | Same, capped at `n` records. Answers `n` from any origin (guard `tests/scripts/48b-spatial-slice.loft`). |
| `xs[(x1,y1)..(x2,y2)]` | Bounding-box range — exactly what is inside the box. |

There are no `.near`/`.within`/`.nearest` methods — proximity is ordinary
range slicing. The BOX form gives exactly the records inside the box and
nothing outside it, in the collection's own order (guard
`tests/scripts/800-spatial-box-containment.loft`). A corner-swapped axis names the
same box.

The two OPEN forms walk outward from the query point in both directions, so
`..:n` answers `n` records from any origin, including one past every record.
The order is by Morton distance, which jumps at quadrant boundaries, so a
truly-near point can arrive a little late; when the answer must be exact, use
a symmetric box, `xs[(x-r, y-r)..(x+r, y+r)]`. A cap of zero or below —
`:k` with `k` negative or null — answers no records.

A slice is a `for`-loop iterator, not a value, same as other keyed range
slices. See [DATABASE_INDEXES.md § Spatial Index](DATABASE_INDEXES.md#spatial-index-srcradix_treers)
for the implementation.

A text key is refused here — `spatial<Word[w]>` names `trie<Word[w]>` instead
(guard `tests/parse_errors.rs`).

### `trie<T[k]>` — text-keyed collection

The same radix tree over ONE **text** key, keyed on its bytes rather than a
Morton code:

| Syntax | Description |
|--------|-------------|
| `t: trie<Word[w]> = [];` | Construct; also legal as a struct field. |
| `t += [Word{w: "kerk"}];` | Append. |
| `for x in t { … }` | Iterate in key order — byte order, no sort. |
| `t.len()` | Element count — O(1), the tree's cached length word. |
| `x = t["kerk"]` | Exact lookup; `null` when absent, never a neighbour. |
| `t["kerk"..]` | Every key BEGINNING with `"kerk"`, in key order. |
| `t["kerk"..:n]` | The first `n` of them. |

**The prefix is what earns the kind its place.** A `sorted<T[text]>` range
needs a successor string — `t["kerk".."kerl"]` — that the caller must
construct, gets wrong at a byte boundary, and that answers a key INTERVAL
rather than a prefix.  So `t[a..b]` is refused, and the message names `sorted`
as the kind that answers an interval.

The terminator sorts before any byte, which is why `kerk` precedes
`kerkstraat` precedes `kerkweg`.  Exactly one key field (several keys have no
byte order to share — use `sorted<T[a, b]>`), and it must be `text`
(a numeric key names `spatial` / `sorted` / `index`).  A prefix slice is a
`for`-loop iterator, not a value, as with every keyed range slice.  See
[DATABASE_INDEXES.md § Text Trie](DATABASE_INDEXES.md#text-trie-srctrie_dbrs) for the
implementation.

---

## Output and Diagnostics

| Function | Description |
|----------|-------------|
| `print(v: text)` | Writes `v` to standard output without a newline. |
| `println(v: text)` | Writes `v` followed by a newline. |
| `assert(test: boolean, message: text)` | Panics with `message` if `test` is false. In production mode (`--production` CLI flag), writes an `error` log entry instead of aborting. |
| `assert_eq<T: Equatable + Printable>(got: T, want: T, what: text = "")` | Panics if the two differ, naming **both** sides: `what: got 4, want 5`. The label is optional. |
| `assert_ne<T: Equatable + Printable>(got: T, want: T, what: text = "")` | Panics if the two are equal, naming the value they share: `what: both sides are 5`. |
| `panic(message: text)` | Immediately terminates execution with `message`. In production mode, writes a `fatal` log entry instead of aborting. |

**`assert_eq` says what `assert` cannot.** `assert(got == want, "…")` puts the expected value
in the CONDITION, so a failure reports what was got and leaves the reader to recover what was
wanted by reading the expression back. `assert_eq` reports both, on any type that is
`Equatable` (defines `op ==`) and `Printable` (defines `to_text`) — every built-in scalar, and
a user type defining the two:

```grammar
assert_eq(total, 42, "the running total");
// error: assertion failed: the running total: got 41, want 42
//   --> game.loft:12:3
```

It IS `assert` underneath, so the halt behaves identically — same rendering, same
`--production` demotion to an `error` log entry, same non-zero exit — and the position
reported is the CALL SITE's, not the stdlib's. Drop the label where the two values and the
source position already say enough: `assert_eq(total, 42)` reports `got 41, want 42`.

**What a halt looks like.** `assert` and `panic` are the two explicit halt statements, and
they render the same way as each other on every backend: the message and the program's own
`file:line:col`, the source line with a caret under it, then the loft functions the fault
happened inside, innermost first.

```
error: assertion failed: n was 9
  --> game.loft:12:1
  |
12|     assert(n < 5, "n was {n}");
  | ^
  in fn inner() ← called from
        fn middle()
        fn main()
```

A chain of one frame is not printed — the position already names it. Inside a `par` worker
the frames are the WORKER's, not the parent's; the halt itself is total, stopping the whole
program rather than one arm (loft#1053). The whole rendering is one thing loft prints once,
however many workers reach the fault together (loft#1056).

**Printing values — the format-string idiom.** `print`/`println` take `text`, so any
non-text value is printed through a format string, which interpolates *any* `Printable`
via its `to_text` (every scalar, and a user type once it defines `fn to_text(self: T) ->
text`):

<!-- from tests/reference/print-values.loft -->
```loft
count = 41;
print("{count + 1}\n");        // 42 — a single value
print("{a} {b} {c}\n");        // several values, separators written in place
p = Point { x: 3, y: 4 };
print("{p}\n");                // a user type via its to_text
```

This one tool covers printing a value, separating several values, and appending strings
— loft has no variadic `print(a, b, c)` and no bare `print(42)` (a deliberate decision,
[DESIGN_DECISIONS.md § C100](DESIGN_DECISIONS.md#c100--print-stays-text-only-no-bare-printvalue-or-variadic-print); @PLN13 step 5). Write the separator you
want inside the braces (`"{a} {b}"` spaces, `"{a}, {b}"` commas, `"{a}{b}"` appends).

---

## Logging

Structured file-based output from running loft programs. Logging is configured via `log.conf` beside the main `.loft` file (or `--log-conf <path>`). See [LOGGER.md](LOGGER.md) for full configuration reference.

| Function | Description |
|----------|-------------|
| `log_info(message: text)` | Writes a record at `INFO` severity. Silently discarded if no logger or below the configured level. |
| `log_warn(message: text)` | Writes a record at `WARN` severity (default minimum level). |
| `log_error(message: text)` | Writes a record at `ERROR` severity. |
| `log_fatal(message: text)` | Writes a record at `FATAL` severity. Does **not** abort (use `panic()` to abort). |

The loft source file and line number are injected by the compiler at each call site — the log record always shows exactly where in the loft code the log call was made.

Rate limiting: at most 5 messages per 60-second window per call site (configurable). Suppressed messages are counted and a notice is emitted when the window resets.

```
2026-03-13T14:05:32.417Z WARN  src/compute.loft:142  division result may overflow
```

---

## File System

Paths, files, directories, binary I/O, durable and path-backed stores, lazy store binding and images: [STDLIB_FILES.md](STDLIB_FILES.md).

## JSON / Parsing

JSON support has two layers:

1. **`JsonValue` enum** — a first-class typed tree (preferred for new code; covers dynamic shapes).
2. **`{value:j}` interpolation + `Type.parse(text)`** — legacy text-based path; `Type.parse(JsonValue)` is the in-progress replacement (P54 step 5).

### JsonValue surface

<!-- from default/06_json.loft -->
```loft
pub enum JsonValue {
  JNull,
  JBool { value: boolean },
  JNumber { value: float },
  JString { value: text },
  JArray { items: vector<JsonValue> },
  JObject { fields: vector<JsonField> },
  JInteger { value: integer },
}
```
<!-- from default/06_json.loft -->
```loft
pub struct JsonField {
  name: text,
  value: JsonValue,
}
```

**Number semantics (@PLN109).** A JSON number with **no fraction and no exponent**
that fits `i64` parses to **`JInteger`** and preserves the exact integer — so
`json_parse("9007199254740993").as_long()` and a typed `integer` field both read
`9007199254740993`, not the `f64`-rounded `…992` (fixes @PLN102 H5). A number with
a `.` or an exponent (`1.5`, `1e3`), or one that overflows `i64`, is a **`JNumber`**
(`f64`). `1e3` is a float (`1000.0`), matching mainstream JSON. `as_long()`/an
`integer` field read a `JInteger` exactly and truncate a `JNumber`; `as_number()`/a
`float` field read a `JNumber` as-is and widen a `JInteger`. Serialisation is exact
in both cases. `json_number(x)` always builds a `JNumber` (it takes a `float`).

| Function | Description |
|---|---|
| `json_parse(text) -> JsonValue` | Parse JSON; malformed input returns `JNull` |
| `json_errors() -> text` | Pipe-separated diagnostics from the last `json_parse` |
| `kind(v) -> text` | Variant name: `"JNull"` / `"JBool"` / `"JNumber"` / `"JInteger"` / `"JString"` / `"JArray"` / `"JObject"` |
| `len(v) -> integer` | Length of a `JArray`/`JObject`; null sentinel for any other variant |
| `field(v, name) -> JsonValue` | `JObject` lookup; `JNull` on miss / wrong kind |
| `item(v, index) -> JsonValue` | `JArray` index; `JNull` on out-of-bounds / wrong kind |
| `has_field(v, name) -> boolean` | `true` iff `JObject` carries a field named `name` |
| `keys(v) -> vector<text>` | Field names in insertion order; empty for non-objects |
| `fields(v) -> vector<JsonField>` | Full `(name, value)` entries; values deep-copied |
| `as_text(v) / as_number(v) / as_long(v) / as_bool(v)` | Typed extractor; null on kind mismatch |
| `to_json(v) -> text` | Canonical RFC 8259 serialisation (no whitespace) |
| `to_json_pretty(v) -> text` | 2-space indent, one element per line for non-empty containers |
| `json_null() -> JsonValue` | Constructor — `JNull` |
| `json_bool(v: boolean) -> JsonValue` | Constructor — `JBool` |
| `json_number(v: float?) -> JsonValue` | Constructor — `JNumber`; non-finite (NaN / Inf) or null → `JNull` (param is `float?` since handling null/NaN is its contract) |
| `json_string(v: text) -> JsonValue` | Constructor — `JString` |
| `json_array(items: vector<JsonValue>) -> JsonValue` | Constructor — `JArray`; deep-copies items |
| `json_object(fields: vector<JsonField>) -> JsonValue` | Constructor — `JObject`; deep-copies fields |

#### Reading

<!-- from tests/reference/json-values.loft -->
```loft
v = json_parse(`{{"users":[{{"name":"Alice"}}]}}`);
name = v.field("users").item(0).field("name").as_text();   // "Alice"
// every intermediate failure produces JNull, never a trap

match v {
  JObject { fields } => { kind = "object with {len(fields)} field"; },
  JArray { items }   => { kind = "array of {len(items)}"; },
  _                  => { kind = "other"; }
}
```

#### Building

<!-- from tests/reference/json-values.loft -->
```loft
reply = json_object([
  JsonField { name: "ok",    value: json_bool(true) },
  JsonField { name: "count", value: json_number(3.0) }
]);
body = reply.to_json();   // {"ok":true,"count":3}
```

#### Forwarding a captured subtree

<!-- from tests/reference/json-values.loft -->
```loft
inbox = json_parse(request_body);
response = json_object([
  JsonField { name: "echo", value: inbox.field("payload") }
]);
return response.to_json_pretty();
```

### The struct-shaped API

| Expression | Description |
|---|---|
| `"{value:j}"` | Serialise any struct/enum/vector to JSON text |
| `Type.parse(text)` | Parse JSON or loft-native text into a struct (P54 step 5 will require `Type.parse(JsonValue)` for structs) |
| `vector<T>.parse(text)` | Parse a JSON array into an iterable vector |
| `record#errors` | The last `Type.parse()` call's errors as ONE text (newline-separated), cleared by the read |

<!-- from tests/reference/json-values.loft -->
```loft
user = User.parse(`{{"id":42,"name":"Alice"}}`);
scores = vector<Score>.parse(`[{{"value":10}},{{"value":20}}]`);

// A failed parse leaves the record at its type's zeros, so ask whether it failed.
errs = user#errors;              // TEXT, not a collection — and the read clears it
```

Reach for this form when you KNOW the document's shape and it maps onto a struct you have
declared, and for the `JsonValue` surface above when you do not, or when your program has
to report precisely what was wrong.  Both are supported.

---

## Higher-order functions

`map`, `filter`, and `reduce` are compiler special-cases (like `parallel_for`) — they take a function by its bare name (or a lambda) and a vector and produce a new vector or scalar.

| Signature | Description |
|---|---|
| `map(v: vector<T>, f: fn(T) -> U) -> vector<U>` | Applies `f` to each element and collects the results |
| `filter(v: vector<T>, pred: fn(T) -> boolean) -> vector<T>` | Keeps only elements for which `pred` returns `true` |
| `reduce(v: vector<T>, init: U, f: fn(U, T) -> U) -> U` | Left-folds `v` starting from `init`, applying `f(acc, elm)` at each step. `U` may be a scalar, `text` or a COLLECTION; `U` is read off `f`'s first parameter, so `v.reduce([], f)` and an init spelled `vector<integer>` are the same fold |

<!-- from tests/reference/higher-order.loft -->
```loft
fn double(x: integer) -> integer { x * 2 }
fn is_pos(x: integer) -> boolean { x > 0 }
fn add(a: integer, b: integer) -> integer { a + b }
```

passed by their bare names:

<!-- from tests/reference/higher-order.loft -->
```loft
doubled  = map(nums, double);            // [2, -4, 6]
positive = filter(nums, is_pos);         // only positive elements
total    = reduce(nums, 0, add);         // sum of all elements
```

`U` really is free in `map` — the callback's return type is what the result vector holds, and
an inline lambda takes its return type from its own body:

<!-- from tests/reference/higher-order.loft -->
```loft
labels = map(nums, |x| { "n{x}" });      // vector<text> from a vector<integer>
sizes  = words.map(|w| { len(w) });      // vector<integer> from a vector<text>
pairs  = nums.map(|x| { [x, x + 1] });   // vector<vector<integer>>
```

`reduce` folds into `text` as well as a scalar:

<!-- from tests/reference/higher-order.loft -->
```loft
joined = words.reduce("", |a, w| { "{a}{w}" });   // one string from a vector<text>
```

A COLLECTION accumulator (`v.reduce([], f)`) works, and its type is the fold's: `f: fn(U, T)
-> U` names `U`, and the init only has to be assignable to it, so a bare `[]` is not asked
what it is.  Guard `tests/scripts/956-reduce-untyped-accumulator.loft`.

All three accept either a function's bare name or a lambda expression:

<!-- from tests/reference/higher-order.loft -->
```loft
doubled = map(nums, fn(x: integer) -> integer { x * 2 });
evens   = filter(nums, fn(x: integer) -> boolean { x % 2 == 0 });
total   = reduce(nums, 0, fn(acc: integer, x: integer) -> integer { acc + x });
```

Lambdas that capture variables from the enclosing scope (closures) also work:

<!-- from tests/reference/higher-order.loft -->
```loft
factor = 3;
scaled = map(nums, fn(x: integer) -> integer { x * factor });
```

Capture is by value at definition time — later changes to `factor` do not
affect the lambda.  See [LOFT_LITERALS.md § Closures](LOFT_LITERALS.md) for details.

---

## Parallel, Reflection, Environment, Time, Random

`par` loops, reflection, command-line arguments, environment and memory diagnostics, time, `use random;`: [STDLIB_RUNTIME.md](STDLIB_RUNTIME.md).

## Open work

Stdlib gaps surfaced by dogfood usage (the @PLAN35 viewer,
@PLN42 tag indexer, lib/markdown extraction).  Each row
names where it bit, the proposed shape, and rough effort.
XS = single-line/single-fn change; S = focused half-day fix.

| Item | Where it bit | Shape | Effort |
|---|---|---|---|
| `vector.sort()` (text added 2026-05-18); `vector.sort_by(fn)` deferred | scan.loft (3 sites), viewer's plan-bucket sort, activity feed date sort | Pre-existing `sort(v)` builtin extended to dispatch on text element type via `vector::sort_text_vector` (lexicographic, sorts u32 string offsets by what they point at).  A user type sorts by its own `op <` since @PLN165 E4 (`sort<T: Ordered>`, stable); `sort_by(fn)` — a comparator other than the type's own `<` — is still open.  Replaces the `sorted<T[K]>` set-as-sort-proxy pattern for text. | **`sort()` text-element shipped (@PLN42 phase 10.8)**; `sort_by(fn)` open. |
| JSON emission helpers | scan.loft has 80+ lines of manual `json_escape` + per-row format-string emission + comma management.  viewer reads via `value.field("x").as_text()` — no symmetric write API. | `to_json(value) -> text` for primitives + `JsonBuilder` for nested structures.  Mirror of the existing `json_parse` + `JsonValue` read API. | S–M |
| `args() -> vector<text>` builtin | scan.loft uses env var `LOFT_INDEX_BUCKETED` as a CLI-arg workaround; viewer doesn't support args at all | Add the builtin that returns the program's invocation args. | XS |

Driver doc: see the "Loft gaps surfaced" section in
[`plans/42-tracker-index/07-loft-native-scanner.md`](plans/42-tracker-index/07-loft-native-scanner.md)
for the consumer-side narrative.

## See also
- [LOFT.md](LOFT.md) — Loft language reference (syntax, types, operators, control flow)
- [INTERNALS.md](INTERNALS.md) — Native function registry, `src/native.rs`, `src/ops.rs`
