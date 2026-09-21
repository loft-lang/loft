
# Loft Standard Library Reference

This document describes all public functions, constants, and types available in the loft standard library.

## Contents
- [Implementation notes](#implementation-notes)
- [Types](#types)
- [Math](#math)
- [Text](#text)
- [Collections](#collections)
- [Keyed collections (hash / index / sorted)](#keyed-collections-hash--index--sorted)
- [Output and Diagnostics](#output-and-diagnostics)
- [Logging](#logging)
- [File System](#file-system)
- [Parallel](#parallel)
- [Reflection](#reflection)
- [Environment](#environment)
- [Time](#time)
- [Random](#random)

---

## Implementation notes

Standard library functions fall into two implementation categories:

- **Loft-implemented** — defined in `default/01_code.loft`, `default/02_files.loft`, or `default/03_text.loft` using the loft language itself. These have a normal function body.
- **Native (Rust)** — declared in the default library with a `#rust "..."` annotation and implemented as hand-written Rust functions in `src/native.rs`. These handle OS interaction and operations that cannot be expressed in loft (file I/O, environment variables, string classification, etc.).

See [INTERNALS.md](INTERNALS.md) for the full list of native functions, their Rust names, and the naming convention (`n_<func>` for globals, `t_<N><Type>_<method>` for methods).

---

## Types

The primitive types built into loft.

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

```loft
s = "abcdef";
s[1..3]                 // "bc" — from byte 1 up to, not including, byte 3
s.char_slice(1, 3)      // "bc" — the same, counting CHARACTERS instead of bytes
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
as `null` (loft#755 — it used to end the loop there).  A NUL therefore round-trips
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

> `text_from_bytes` and `byte_at` existed for two releases and were reported
> missing (loft#748) because the generated reference filed them under Environment
> — a keyword sweep of the Text page came back empty and was read as a language
> gap. Check an instrument against something it *should* find before trusting it
> to report an absence; `grep default/*.loft` answers in one call.

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

```loft
cache: hash<Entry[key]> = [];
reserve(cache, expected_rows);
for row in rows { cache += Entry { key: row.id, value: row.value }; }
```

Filling a million-entry hash rebuilds the table 18 times without it. Measured on
`--native-release`, 1M `integer` keys (2026-09-21): **240 → ~120 ms**, and the finished
table is **a third smaller** (11.5 MB → 8.0 MB) — the growth ladder doubles *past* the
trigger and lands at load 0.35, while a reserved table sits at the 0.5 it asked for. So
it buys time and memory at once.  (Until 2026-09-21 the trigger was 0.75: tables were
half this size, a miss walked five buckets where it now walks one, and a lookup at this
size was 10 % slower on a hit and 35 % on a miss — `bench/portal/analysis/keyed.md`.)

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

```loft
interface Walkable {
  fn children(self: Self) -> vector<Self>
}

struct Node { val: integer, kids: vector<Node> }
fn children(self: Node) -> vector<Node> { return self.kids; }

for n in tree_walk(root, 1000) { visit(n.val); }
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
| `xs[x, y] = mob` | Insert-or-replace at that point (the key comes from `mob`'s own coordinate fields, as for `hash`/`sorted`/`index`). |
| `xs[x, y] = null` | Remove the record at that point; a no-op when the point is empty. |
| `xs[(x,y)..]` | Walk OUTWARD from that point, nearest first; caller `break`s to stop. Approximate — ordered by Morton distance, so a truly-near point can arrive a little late. |
| `xs[(x,y)..:n]` | Same, capped at `n` records. Answers `n` from any origin (it used to answer the Morton TAIL, so it under-delivered near the end of the curve — loft#1002). |
| `xs[(x1,y1)..(x2,y2)]` | Bounding-box range — exactly what is inside the box. |

There are no `.near`/`.within`/`.nearest` methods — proximity is ordinary
range slicing. The BOX form gives exactly the records inside the box and
nothing outside it, in the collection's own order (loft#800 — it used to
answer the raw code interval between the corners, a strict superset, because
Z-order threads out of the box and back). A corner-swapped axis names the
same box.

The two OPEN forms are the Morton **tail**: they yield only records at or
after the query point in the collection's order, so what you get depends on
where that point sits in the curve, and a record just BEHIND it is not
returned however close it is. `..:n` therefore under-delivers near the end of
the curve — over five records it answers 3, 3, 3, 2, 1, 0 as the query moves
along — and a query past every record answers nothing (loft#1002). For a
neighbourhood in every direction use a symmetric box, `xs[(x-r, y-r)..(x+r,
y+r)]`.

A slice is a `for`-loop iterator, not a value, same as other keyed range
slices. See [DATABASE.md § Spatial Index](DATABASE.md#spatial-index-srcradix_treers)
for the implementation.

A text key is refused here — `spatial<Word[w]>` names `trie<Word[w]>` instead
(loft#799).  Before that, it compiled and then answered `null` for a key just
inserted.

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
[DATABASE.md § Text Trie](DATABASE.md#text-trie-srctrie_dbrs) for the
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

```loft
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

```loft
count = 41
print("{count + 1}\n")        // 42 — a single value
print("{a} {b} {c}\n")        // several values, separators written in place
p = Point { x: 3, y: 4 }
print("{p}\n")                // a user type via its to_text
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

Types and functions for reading and writing files. A `File` value is obtained via `file()` and carries the path, format, and an internal reference.

### Path resolution (program-relative by default)

A **relative** file path resolves against **the program's own directory**, not
the process working directory:

- under `--interpret` / tests → the directory of the main source file;
- under `--native` → the directory of the compiled executable;
- queryable as `source_dir()` (works on every backend).

So `file("assets/font.ttf")` loads the asset that ships **next to the program**,
regardless of where it was launched from — "program + assets" is a portable
bundle.  **Absolute paths are never rewritten.**

This applies uniformly to every path-taking operation — `file()`, `exists()`,
`read_file`/`write_file`, the `File` methods, `delete`/`move`/`mkdir`/`mkdir_all`/`rmdir`,
and image loads — so they all agree on where a relative path points.

**Opting into cwd-relative (CLI tools).** A program that takes a *user-supplied*
relative path (`loft tidy.loft data.csv` — `data.csv` is in the user's cwd, not
beside the script) declares the file-top directive:

```loft
#cwd
fn main(args: vector<text>) { ... }   // relative paths now resolve against the cwd
```

`#cwd` is whole-program and must precede the first declaration.  Per-invocation,
the `LOFT_PATHS` environment variable overrides both: `LOFT_PATHS=program` forces
program-relative, `LOFT_PATHS=cwd` forces cwd-relative.  `source_dir() -> text`
returns the anchor (empty only when there is none, e.g. a wasm host with no
filesystem).

### Types

**`Format`** (enum): Describes how a file is opened.

| Value           | Description |
|-----------------|-------------|
| `Format.TextFile`     | Default. Read or write as UTF-8 text. |
| `Format.LittleEndian` | Binary mode, least-significant byte first. |
| `Format.BigEndian`    | Binary mode, most-significant byte first. |
| `Format.Directory`    | Represents a directory path. |

**`File`**: A handle to a filesystem entry. Fields: `path: text`, `size: integer`, `format: Format`.

### Opening Files

| Function | Description |
|----------|-------------|
| `file(path: text) -> File` | Opens the file at `path` and returns a `File` handle. |

**A relative path resolves against the program's own directory** (#255 / @PLN9),
so `../data.txt` names the file above the script — the same file its absolute
form names, and it answers the same either way. There is no path-shape filter:
`..` is resolved, not inspected.

That is a change (loft#712). A lexical filter used to refuse any relative path
containing `..`, and reported the refusal as a **null size** — indistinguishable
from a missing or empty file, so a reader doing `if f#size < HEADER` turned it
into "the file is truncated" and reported a *data* error for what was a *path*
decision. It was not containment either: the same bytes by absolute path were
served, and a `..` that normalised back inside the root was refused too. loft has
no filesystem sandbox — admission is decided at load time and carries no runtime
checks ([SANDBOX.md](SANDBOX.md)) — so the resolved path is the whole answer and
the filesystem gives it.

If a program must not reach outside a directory, check that yourself before the
call; the stdlib does not, and did not meaningfully do so before.

### Reading Text Files

| Function | Description |
|----------|-------------|
| `content(self: File) -> text?` | Reads the entire file as a UTF-8 text value. **Null** when there is no text to read: the file is missing, the path is a directory, or the bytes are not valid UTF-8. `""` means the file really is empty. |
| `lines(self: File) -> vector<text>` | Reads the file and splits it into lines. **Empty** wherever `content()` is null — a missing file, a directory, or bytes that are not UTF-8 — so a loop over it runs zero times and reads exactly like a loop over an empty file. `lines()` has no null of its own to carry the distinction; ask `content()` when it matters. |

`content()` is nullable because `""` cannot carry three different meanings.  A
non-UTF-8 file used to answer `""`, indistinguishable from an empty one, so a
gate of the shape *"write bytes, read them back, compare"* passed **vacuously**
on binary data — both sides were `""` (loft#829).  Discharge with `?? ""` to
keep the old shape where the distinction does not matter.

Reading such a file is not the failure — asking for it as *text* is.  Use
[`read_bytes(path) -> vector<u8>?`](#filesystem-operations), which is
byte-exact and round-trips with `write_bytes`, or open the file in binary mode
and read it a field at a time (see [Binary Files](#binary-files)).  A non-UTF-8
`content()` also prints one stderr warning naming both, on **both** backends.

### Writing Text Files

| Function | Description |
|----------|-------------|
| `write(self: File, v: text)` | Writes `v` as UTF-8 text to the file. Overwrites existing content. |

### Binary Files

Binary mode must be activated before reading or writing raw data. Use `f.format = Format.LittleEndian` or `f.format = Format.BigEndian` to enable binary mode.

| Function | Description |
|----------|-------------|
| `little_endian(self: File)` | Switches the file to little-endian binary mode. |
| `big_endian(self: File)` | Switches the file to big-endian binary mode. |
| `write_bin(self: File, v: reference)` | Writes a struct value as raw binary data. File must be in binary mode first. |
| `read(self: File, v: reference)` | Reads binary data into a struct value. File must be in binary mode first. |
| `seek(self: File, pos: integer) -> boolean` | Moves the read/write position to `pos` bytes from the start — random access into a binary file. `false` (a no-op) for a directory, an absent file, a negative `pos`, or a file this process has not read from or written to yet (the OS handle opens on first I/O). Seeking PAST the end is allowed: a following write extends the file. Operator form: `f#next = pos`. |
| `position(self: File) -> integer` | The byte offset the next read or write will land at — the read side of `seek`. Operator form: `f#next`. Distinct from `f#index`, which is where the LAST read *started*: after one `f#read as i32` on a fresh file, `position` is 4 and `f#index` is 0. **Null** for a file this process has not opened yet (0 is a real position, so it is not used to mean "no position"). |

**Binary attribute operators on `f: File`:**

| Syntax | Description |
|--------|-------------|
| `f += value` | Writes `value` at the width of its declared type — `i32`/`u32` → 4B, `u8` → 1B, `u16` → 2B, range-constrained `integer limit(0,255)` → 1B, bare `integer` (i64 storage) → 8B, `single` → 4B, `float` → 8B, `text` → raw UTF-8 bytes, `vector<T>` → each element at its declared width. |
| `f#read as T` | **Preferred** — reads `T`'s natural byte width and returns a value of type `T`. `as i32` reads 4B, `as u8` reads 1B, `as u16` reads 2B, `as integer` reads 8B. |
| `s.field = f#read` | **LHS-inferred** — width comes from `s.field`'s declared type; symmetric with `f += s.field`. No `as T` needed. |
| `f#read(n) as T` | Legacy explicit form — reads exactly `n` bytes and interprets as `T`. `n` MUST match `T`'s storage width or the runtime panics. |
| `f#read(n) as text` | Reads exactly `n` bytes (or fewer at EOF) as a UTF-8 string. The `(n)` is REQUIRED for text — variable-width types have no inferable count. |
| `f#size` | Returns the current file size in bytes as `integer`. |
| `f#index` | Returns the byte offset where the last read started (the `current` field). |
| `f#next` | Returns the current byte position (after last read). |
| `f#next = pos` | Seeks the file to `pos` (integer). Only works after the file has been opened by a prior read or write. |
| `f#exists` | Returns `true` if the file or directory exists (format ≠ `Format.NotExists`). |
| `f#format` | Reads the `Format` enum value of `f`.  **Avoid on binary files** — accessing `.format` on a file with unrecognized magic panics in `src/database/io.rs:276`.  Use `exists(path)` instead for existence checks. |
| `f#format = Format.X` | Sets the format of `f`. |

**Notes:**
- `f += "text"` writes raw UTF-8 bytes; supported for TextFile, LittleEndian, and BigEndian modes.
- For new files (format=NotExists), `f += value` defaults to TextFile mode and creates the file.
- `f#next = pos` (and the `seek` method above) is a no-op if called before the first read or write — the OS file handle does not exist until first I/O. Always perform a read or write before seeking. `seek` returns `false` in that case; the operator form reports nothing, which is why the method is the better choice when the position matters.

#### Struct and vector binary round-trip (@PLN47 — shipped 2026-07-09)

`f += my_struct` and `s = f#read as MyStruct` now round-trip for any
struct whose fields are all fixed-width scalars (integer, float, single,
boolean, character, i8/u8/i16/u16/i32/u32).  Nested plain structs (a
struct field of struct type) also round-trip.  The record is allocated
before reading, so each field is filled in declared order.

**`f#read(N) as vector<T>`** reads exactly `N` **bytes** (not elements)
and returns a `vector<T>`.  Example: `f#read(24) as vector<integer>`
reads three 8-byte ints.  A parse-time warning fires when a literal `N`
is not a multiple of the element byte-width, catching the silent
empty-vector footgun.

**`f += ch` / `f#read as character`** round-trips as 4 bytes on both
backends.  **`f += b` / `f#read as boolean`** round-trips as 1 byte on
both backends.  **Signed narrow ints** (`i8`, `i16`) read back with
correct sign extension (e.g. `-12345 as i16` round-trips to `-12345`).

**Structs with variable-width fields** (`text`, `vector`, or any
collection field) are **rejected at compile time** on both backends:

```
read_file: 'T' has variable-width field 'name' (text/vector/collection)
that binary I/O cannot round-trip; serialise a plain fixed-width struct
```

The write-side diagnostic says `write_file:`.  Nested-struct fields are
reported as `outer.inner`.

**`f += text` / `f += vector<T>`** still write raw bytes with no length
prefix.  Callers that manage their own length tracking (e.g. GLB chunks
with the count in the outer header) continue to use this form and read
with `f#read(N) as text` / `f#read(N) as vector<T>`.

**Retained caveat (@P293)**: `u32`/`i32` values ≥ 2³¹ round-trip via
raw bytes but read back as negative i64 in loft expressions.

**Known limitation**: `q = f#read as Struct` leaks one record per read.
This is a pre-existing general loft ownership bug — `x = { …; struct_temp }`
leaks the temp store even with no file I/O (function returns and direct
struct literals are clean; only the inline-block-returning-a-struct form
leaks).  Tracked separately as an ownership-model issue.

Test harness: `tests/binary_io_matrix.rs` (32 cross-mode cells,
`#[ignore]`, run with
`cargo test --release --test binary_io_matrix -- --ignored`).

### Directories

| Function | Description |
|----------|-------------|
| `files(self: File) -> vector<File>` | Returns the entries inside a directory, sorted by path — the same order as `list_dir`, so an index means the same entry in either listing. The `File` must have `format == Format.Directory`; anything else lists as `[]` (never null, unlike `list_dir`). |

### Filesystem Operations

Mutating filesystem operations return a `FileResult` enum:

| Variant | Meaning |
|---------|---------|
| `FileResult.Ok` | Operation succeeded. |
| `FileResult.NotFound` | Path does not exist. |
| `FileResult.PermissionDenied` | OS permission denied. |
| `FileResult.IsDirectory` | A file operation targeted a directory (e.g. `delete()` on a directory). |
| `FileResult.Other` | Any other OS error. |

Every variant is actually produced: `--native` and `--interpret` classify from the OS
error; the wasm host reports only `Ok` / `Other`, and `NotFound` still comes from the
loft-level existence check.

| Function | Description |
|----------|-------------|
| `ok(self: FileResult) -> boolean` | Returns `true` if `Ok`. |
| `exists(path: text) -> boolean` | Returns `true` if the path exists. |
| `exists(self: const File) -> boolean` | Method form: `f.exists()` or `exists(f)`. |
| `delete(path: text) -> FileResult` | Removes a file. |
| `move(from: text, to: text) -> FileResult` | Renames or relocates a file. |
| `mkdir(path: text) -> FileResult` | Creates a single directory level. |
| `mkdir_all(path: text) -> FileResult` | Creates a directory and all missing parents. |
| `rmdir(path: text) -> FileResult` | Removes an EMPTY directory; `NotEmpty` when it still holds entries. A recursive removal is the caller's walk — `list_dir`, `delete`, then `rmdir` deepest-first. |
| `is_dir(path: text) -> boolean` | Returns `true` if the path exists and is a directory. |
| `is_file(path: text) -> boolean` | Returns `true` if the path exists and is a regular file. |
| `list_dir(path: text) -> vector<text>?` | Entry names (base names, sorted) of a directory. **Null** when the path is missing or is not a readable directory; `[]` means the directory really is empty. Discharge with `?? []`. |
| `read_bytes(path: text) -> vector<u8>?` | Reads the whole file as raw bytes. **Null** when the file is missing or unreadable; `[]` means the file really is empty. Binary-exact (round-trips with `write_bytes`). Discharge with `?? []`. |
| `write_bytes(path: text, bytes: vector<u8>) -> boolean` | Writes raw bytes to a file, truncating existing content; `true` on success. |
| `set_file_size(self: File, size: integer) -> FileResult` | Truncates or extends a file to exactly `size` bytes. |

### Durable stores (`@PLN43`)

A "durable store" is a regular on-disk file plus a 40-byte `.dmeta`
sidecar holding signature + tier + CRC32 over the main file + a
clean-close timestamp.  After a successful write session, call
`store_durable_seal(path)` to record the current state; on next
startup, call `store_durable_check(path)` to verify integrity.  A
failing check means the file is missing, corrupt, or the sidecar is
stale — the caller is expected to rebuild from authoritative sources.

| Function | Description |
|----------|-------------|
| `store_durable_check(path: text) -> boolean` | Returns `true` iff the `.dmeta` sidecar at `<path>.dmeta` validates against the main file at `path` (signature, header CRC, payload length, payload CRC, tier_id all OK).  Returns `false` on any failure or missing file.  Phase-01b Tier-1 only (no msync discipline). |
| `store_durable_seal(path: text) -> boolean` | Writes a fresh `.dmeta` sidecar capturing the current main-file's byte length + CRC32 + a clean-close timestamp.  Returns `false` on any I/O error.  Pair with `store_durable_check` to bracket each write session. |

**Both are desktop-only, and say so by answering `false`** (loft#1063).  The sidecar
machinery is built over memory-mapped files, which the wasm targets have no equivalent
for, so on `--native-wasm` and `--html` each call compiles and returns `false` — the same
route `store_persist_bind` takes.  `false` is the honest answer on that target and the safe
one: it means *the seal did not happen* and *nothing validates*, which is exactly the state
that sends a caller down its rebuild branch.  Seal where the store is WRITTEN (a desktop
build) and use the portable readers — `store_load`, `store_load_key*` — in the browser; an
image sealed by `--native` loads byte-identically on wasm.

Usage pattern:

```loft
fn main() {
  path = "data.bin";
  if !store_durable_check(path) {
    rebuild_from_source(path);  // consumer-defined
  }
  // ... use the database that lives in `path` ...

  // graceful shutdown
  flush_database();
  store_durable_seal(path);
}
```

If the program crashes between the last write and the seal, the
sidecar stays stale relative to the file → next start's
`store_durable_check` returns `false` → caller rebuilds.  This is
**by design** — Tier 1 trades durability for cheap writes (no
`msync` on the hot path) and recovers via rebuild from a
re-derivable source.  **Do not use** for data that cannot be
re-derived; Tiers 2 (snapshots) and 3 (WAL) are planned for that
case but not yet shipped.

Full design + format spec:
[`doc/claude/plans/43-loft-store-durable/`](plans/43-loft-store-durable/README.md).

### Path-backed hash storage (`@PLN43`)

`store_persist_bind(h, path)` re-roots the Store backing a hash at a
file path so mutations are durable via mmap without an explicit save
loop — "the hash IS the file."  Dryopea's persistence destination ask
([`QUESTIONS_FOR_LOFT.md` § Path-backed user-data Store binding](https://github.com/jjstwerff/dryopea)),
also the canonical pattern for any single-collection on-disk state.

| Function | Description |
|----------|-------------|
| `store_persist_bind(h: hash, path: text) -> boolean` | Re-roots the Store backing `h` at a file at `path`.  Fresh-path branch: snapshots the current bytes (padded to ≥1024 words with a valid tail-free block), writes them, and mmaps the file.  Existing-path branch: opens the file via mmap and adopts its contents (discarding the in-memory state at that slot) — unless the `.dschema` beside it records a different layout than `h`'s type, which is refused as `store_load` refuses it.  Returns `false` on any I/O / format / layout error — no panic; callers fall back to JSON or rebuild. |
| `store_persist_copy(r: reference, path: text) -> boolean` | Writes an image of `r` laid out for PAGING and leaves the live collection where it is — nothing moves, so every reference stays valid, and the file is NOT bound.  The image is REBUILT, so each record sits in its collection's own order (key order for a `trie`), which is what makes a paged prefix query cheap: measured 4.9 requests / 0.32 MB against a bound image's 19.9 / 1.28 MB on a 74,692-word vocabulary.  Sized to its content, with none of the growth slack a bound file keeps.  Use it for a file another program or a browser will READ; use `store_persist_bind` for a store you go on writing to. @PLN134 |

Usage pattern:

```loft
fn main() {
  h: hash<Entry[key]> = [];

  // Bind first — on fresh path the empty hash is serialised; on
  // existing path the on-disk contents are loaded into this slot.
  store_persist_bind(h, "world.store");

  // Subsequent mutations hit the mmap'd buffer.  No explicit save.
  h += Entry { key: 7, value: 700 };
  // ... OS msyncs on idle / clean exit ...
}
```

**Semantics in detail:**

- When `path` does not exist at call time, `store_persist_bind`
  captures the current in-memory bytes of the hash's Store, pads
  them out to a valid ≥8192-byte image, writes them to disk, and
  swaps the slot's allocator over to the mmap'd file.  The caller's
  `h` is unchanged from a record-layout perspective — the DbRef
  shape `(store_nr, rec, pos)` stays valid, only the underlying
  buffer moves from anonymous heap to mmap.
- When `path` exists, the call first compares the layout recorded
  in `<path>.dschema` with `h`'s type.  A different layout — a
  struct that gained, lost or changed a field — is refused: the
  call returns `false`, prints what differs on stderr, and leaves
  the file, its `.dschema` and `h` untouched.  A file without a
  `.dschema` is not checked.  Otherwise the call invokes
  `Store::open(path)`, which validates the loft Store signature and
  rebuilds the free-list.
  The caller's prior in-memory state at that slot is discarded.
  The caller's existing `DbRef`s into the hash remain valid IFF the
  on-disk layout describes the same type — the standard pattern is
  to allocate an empty hash and immediately bind, so the empty
  in-memory state is harmlessly discarded in favour of the on-disk
  view.
- Pair with `store_durable_check` / `store_durable_seal` (above)
  when you also want Tier-1 integrity assurance — the bracket
  pattern is unchanged, only "what's between check and seal"
  becomes "nothing — the hash mutations ARE the writes."

**Persisted size = the store's *peak* arena, not its live size — a trap
worth planning for.** The fresh-path snapshot writes the hash's whole
Store arena at the capacity it has ever reached, free slack included; it
is not compacted down to the live records (minimum ~8 KB image). Three
facts combine to make this bite:

- the snapshot copies the arena's current capacity verbatim — no repack;
- a Store arena only ever grows — it never shrinks;
- freeing or clearing records returns their slots to a free list but does
  **not** hand the arena bytes back.

So the file is the high-water mark the bound Store reached during its
life. Build a large structure *in place* in the same Store you then bind
— inserting then pruning, or rebuilding as you go — and the file freezes
that peak: one consumer saw a 264 MB file for 3.5 MB of live data (~90×).

**Keep the scratch out of the Store you bind: build the result in a
helper and return it.** A helper's transient containers are scope-local
Stores, freed at the return boundary (see [LIFETIME.md](LIFETIME.md)), so
their growth never lands in the Store that survives as the return value.
Bind *that* returned hash:

```loft
fn build_world() -> hash<Hex[q, r]> {
  w: hash<Hex[q, r]> = [];
  // fill w; any transient collections here are freed on return
  return w;
}
fn main() {
  world = build_world();                 // carries only the live result
  store_persist_bind(world, "world.store");
}
```

Whether a top-level build actually bloats depends on the pattern
(in-place insert-then-prune is the classic case); the reliable rule is to
keep transient growth out of the Store you bind.

**Failure modes (returns `false`):**

- Path is empty or not valid UTF-8.
- Existing file's signature doesn't match the loft Store format
  (caught via `catch_unwind`; no panic propagates).
- I/O error writing the fresh-path snapshot.
- `mmap` feature disabled in this build.

Off when the `mmap` Cargo feature is disabled at build time:
returns `false` so consumers branch into a JSON fallback (or
rebuild-from-source).

### Lazy store binding (`@F108`)

Bind a collection to a source and let the LOOKUPS do the loading: a lookup that
misses fetches exactly that one entry and inserts it, so the next lookup for the
same key is an ordinary resident hit.  The collection is therefore its own cached
working set — there is no second structure to keep in step with it.  Contrast
`store_load_key` ([REMOTE_STORES.md](REMOTE_STORES.md)), where the program names
the entries to fetch.

The model behind these calls — what a query is derived from, why `len` answers the
resident count, and what a binding refuses — is [LAZY_STORES.md](LAZY_STORES.md).

| Function | Description |
|----------|-------------|
| `store_bind_lazy(c: reference, source: text) -> boolean` | Bind collection `c` to `source` — an IMAGE (a local `.store` file or an `http(s)://` URL served with Range, i.e. whatever `store_load_key` accepts) or a DATABASE (`sqlite:<path>`), where the `SELECT` is derived from `c`'s own type: table = the element type's name lowercased, columns = its fields, `WHERE` = its key.  Read-only; the database source serves a keyed lookup on any ordered or hashed kind, and a binding it cannot turn into a query is refused through `store_lazy_error` rather than served wrongly.  Per COLLECTION, not per store: two collections of one type may bind differently.  Binding replaces any previous binding, and may be done before `c` holds anything.  **`false` is worth checking**: besides a null collection, an IMAGE is read a page at a time and only a `hash` or a `trie` supports that, so a `sorted`/`index`/`spatial` bound to one is refused HERE rather than answering `null` at every later lookup (loft#802) — those kinds load whole, with `store_load` / `store_load_url_trusted`.  A DATABASE source judges its own schema on the first fault instead, since what it can serve is a fact about the other end. |
| `store_lazy_range(c: reference, lo: integer, hi: integer) -> integer` | Pull a whole KEY RANGE from `c`'s bound DATABASE source in ONE query (bounds inclusive, in the collection's own key order); answers how many records `c` gained.  The cure for N+1: 500 records fetched one lookup at a time is 500 round trips, and the same 500 as a range is one.  `c` must be ORDERED (`sorted`/`index`) and keyed on one column — a `hash` has no order to range over and a composite key needs `store_lazy_query`.  A record already resident is left alone. |
| `store_lazy_query(c: reference, condition: text) -> integer` | Run an explicit SQL `condition` against `c`'s bound DATABASE source and pull every matching row INTO `c`; answers how many records `c` gained.  The escape hatch for what the key cannot express (`name LIKE 'Ada%'`, a predicate on another column) — derived queries need no call, this one cannot be derived, so it is written down and visible.  Rows land in the collection rather than in a detached result, and a row already resident is left alone: a person found this way and the same person found by key are ONE record.  Answers `0` both for "nothing matched" and for "the query could not run"; `store_lazy_error` tells those apart. |
| `store_lazy_error(c: reference) -> text` | Why a fetch could not REACH the source, or `""` when healthy.  The FIRST failure's reason, kept — it names the original cause, so a later and often more actionable one reaches stderr but not this call.  Nothing clears it but `store_lazy_clear`: neither a genuine absence nor a later success is an acknowledgement, because reaching the source now says nothing about what an earlier failure already lost. |
| `store_lazy_faults(c: reference) -> integer` | How many fetches could not reach the source.  `0` is healthy; after a traversal it answers "how incomplete am I". |
| `store_lazy_clear(c: reference) -> boolean` | Acknowledge those faults, answering whether there was anything to acknowledge.  The ONLY thing that clears them. |
| `store_lazy_fail(c: reference, why: text)` | **The writing end of that channel**, for a lazy driver written in loft (`fn lazy_fetch(…)`, below).  A driver's three answers do not fit its return value: `1` inserted and `0` absent are integers, and "the source is down" carries a REASON — answering `0` for it is the silent wrong answer this channel exists to prevent.  Sticky and counted exactly like a Rust source's failure. |

**Ask after a null, because a null cannot say why.**  C80 means a value read never
raises, so a miss answers `null` whether the key is genuinely absent or the source
was unreachable — two different facts, one stable and one not:

```loft
p = persons[42];
if p == null {
  why = store_lazy_error(persons);
  if why == "" { /* really no such person */ } else { /* could not reach: {why} */ }
}
```

**Faults are sticky, and only `store_lazy_clear` clears them.**  A later fetch
that happens to succeed does not: a traversal whose first lookup could not reach
the source and whose second could is MISSING data, and reporting "healthy"
afterwards would be exactly the silent wrong answer this channel exists to
prevent.

The source is pinned at bind time, so a traversal sees one consistent world; an
image that changes underneath is REFUSED and reported through the fault channel
rather than silently mixing two versions.  `len` is the RESIDENT count, not the
source's.  Assigning `= []` reclaims what the collection holds while keeping the
binding and preserving held references — the blunt way to cap a working set.

### Images — `use imaging;`

These live in the **`imaging` package**, not the always-loaded stdlib: `Image` and
`Pixel` were drained out of `default/` because image types are not language
primitives. Add `use imaging;` (or call them qualified, `imaging::…`) or the names
do not resolve at all.

`png` is a METHOD, so a missing import reads as `Unknown field File.png` with no
package named — the did-you-mean hint that redirects a free function to its package
does not cover methods yet.

| Function | Description |
|----------|-------------|
| `png(self: File) -> Image` | Decodes a PNG file and returns an `Image`. Returns null unless the file exists and is readable (`Format.TextFile`, which is loft's classification for any ordinary file — a PNG included). |

**`Image`** struct fields: `name: text`, `width: integer`, `height: integer`, `data: vector<Pixel>`.

**`Pixel`** struct fields: `r: integer`, `g: integer`, `b: integer` (each 0–255).

| Function | Description |
|----------|-------------|
| `value(self: Pixel) -> integer` | Returns the pixel colour as a packed 24-bit integer (`0xRRGGBB`). Use for fast colour comparison or storage. |

**Example — read a PNG's dimensions:**
```loft
use imaging;
fn main() {
  img = file("assets/map.png").png();
  println("{img.width}x{img.height}");
}
```

---

## JSON / Parsing

JSON support has two layers:

1. **`JsonValue` enum** — a first-class typed tree (preferred for new code; covers dynamic shapes).
2. **`{value:j}` interpolation + `Type.parse(text)`** — legacy text-based path; `Type.parse(JsonValue)` is the in-progress replacement (P54 step 5).

### JsonValue surface

```loft
pub enum JsonValue {
  JNull,
  JBool    { value: boolean },
  JNumber  { value: float not null },
  JString  { value: text },
  JArray   { items: vector<JsonValue> },
  JObject  { fields: vector<JsonField> },
  JInteger { value: integer }   // @PLN109 — integer-shaped number, exact i64
}
pub struct JsonField { name: text, value: JsonValue }
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

```loft
v = json_parse(`{{"users":[{{"name":"Alice"}}]}}`);
name = v.field("users").item(0).field("name").as_text();   // "Alice"
// every intermediate failure produces JNull, never a trap

match v {
  JObject _ => for f in v.fields() { handle(f) },
  JArray _  => for elm in v.items() { handle(elm) },
  _         => log_warn("expected container: {json_errors()}")
}
```

#### Building

```loft
reply = json_object([
  JsonField { name: "ok",    value: json_bool(true) },
  JsonField { name: "count", value: json_number(3.0) }
]);
text = reply.to_json();   // {"ok":true,"count":3}
```

#### Forwarding a captured subtree

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

```loft
user = User.parse(`{{"id":42,"name":"Alice"}}`);
scores = vector<Score>.parse(`[{{"value":10}},{{"value":20}}]`);

// A failed parse leaves the record at its type's zeros, so ask whether it failed.
if json_errors() != "" { log_warn(json_errors()); }
errs = user#errors;              // TEXT, not a collection — and the read clears it, so
if errs != "" { log_warn(errs); } // `for e in user#errors` iterates nothing
```

Reach for this form when you KNOW the document's shape and it maps onto a struct you have
declared, and for the `JsonValue` surface above when you do not, or when your program has
to report precisely what was wrong.  Both are supported.  (This section was headed
"Legacy text-based API (transitional)" and announced a withdrawal in 0.9.0 — that was
written on 2026-04-14 for the 0.8.4 RC and no roadmap row, decision record or
`#superseded` marker ever scheduled it.  What P54 actually withdrew was `json_items` and
its peers, which are gone.)

---

## Higher-order functions

`map`, `filter`, and `reduce` are compiler special-cases (like `parallel_for`) — they take a `fn <name>` function reference and a vector and produce a new vector or scalar.

| Signature | Description |
|---|---|
| `map(v: vector<T>, f: fn(T) -> U) -> vector<U>` | Applies `f` to each element and collects the results |
| `filter(v: vector<T>, pred: fn(T) -> boolean) -> vector<T>` | Keeps only elements for which `pred` returns `true` |
| `reduce(v: vector<T>, init: U, f: fn(U, T) -> U) -> U` | Left-folds `v` starting from `init`, applying `f(acc, elm)` at each step. `U` may be a scalar or `text`; a COLLECTION accumulator is refused for now (loft#956). Write the loop instead: `acc = <init>; for x in v { acc = f(acc, x); }` |

```loft
fn double(x: integer) -> integer { x * 2 }
fn is_pos(x: integer) -> boolean { x > 0 }
fn add(a: integer, b: integer) -> integer { a + b }

doubled  = map(nums, fn double);         // [2, 4, 6, ...]
positive = filter(nums, fn is_pos);      // only positive elements
total    = reduce(nums, 0, fn add);      // sum of all elements
```

`U` really is free in `map` — the callback's return type is what the result vector holds, and
an inline lambda takes its return type from its own body:

```loft
labels = map(nums, |x| { "n{x}" });      // vector<text> from a vector<integer>
sizes  = words.map(|w| { len(w) });      // vector<integer> from a vector<text>
pairs  = nums.map(|x| { [x, x + 1] });   // vector<vector<integer>>
```

`reduce` folds into `text` as well as a scalar:

```loft
joined = words.reduce("", |a, w| { "{a}{w}" });   // one string from a vector<text>
```

A COLLECTION accumulator (`v.reduce([], f)`) is refused with a message pointing at the
loop to write instead — it reads back empty and is an internal compiler error on
`--native`, tracked as loft#956.

All three accept either a named function reference (`fn <name>`) or a lambda expression:

```loft
doubled = map(nums, fn(x: integer) -> integer { x * 2 });
evens   = filter(nums, fn(x: integer) -> boolean { x % 2 == 0 });
total   = reduce(nums, 0, fn(acc: integer, x: integer) -> integer { acc + x });
```

Lambdas that capture variables from the enclosing scope (closures) also work:

```loft
factor = 3;
scaled = map(nums, fn(x: integer) -> integer { x * factor });
```

Capture is by value at definition time — later changes to `factor` do not
affect the lambda.  See [LOFT.md § Closures](LOFT.md) for details.

---

## Parallel

The public parallel API is the `par(...)` for-loop clause. The internal functions `parallel_for` and `parallel_for_int` are not part of the user API.

Function references (`fn <name>`, type `fn(T) -> R`) are first-class callable values — they can be stored in variables, passed as parameters, and called directly (`f(args)`), not only as `par(...)` worker arguments. See [LOFT.md](LOFT.md) § Literals for the full syntax.

### `par(...)` Parallel For-Loop

```loft
for a in <vector> par(b=<worker_call>, <threads>) {
    // body — b holds the worker result for this element
}
```

Two worker call forms:

| Form | Example | Description |
|---|---|---|
| Form 1 | `func(a)` | Global function called with the loop element |
| Form 2 | `a.method()` | Method on the element type |

Supported return types: `integer`, `float`, `single`, `boolean`, inline `enum`, `text`.
Extra context arguments are forwarded: `par(b=scale(a, mult), N)`.
Input must be a `vector<T>`.

```loft
struct Score { value: integer }

fn double_score(r: const Score) -> integer { r.value * 2 }
fn get_value(self: const Score) -> integer { self.value }

fn main() {
    q = make_scores();   // vector of Score

    // Form 1: global function
    sum = 0;
    for a in q.items par(b=double_score(a), 4) {
        sum += b;
    }

    // Form 2: method
    total = 0;
    for a in q.items par(b=a.get_value(), 1) {
        total += b;
    }
}
```

**Worker function rules:**
- Accept a single `const` reference as the first parameter.
- Do not name the first parameter `self` (this makes it a method, looked up differently).
- Workers receive a read-only store snapshot; writing to input data panics.
- No nested parallelism.

**Limitations:**
- Float/integer result accumulation in the loop body: if `b` is float/integer, using it in arithmetic with a pre-declared float/integer variable can trigger a first-pass type-inference conflict. Workaround: use `b` only in boolean comparisons or cast (`sum += b as integer`).
- Implementation: `src/parallel.rs`; see [THREADING.md](THREADING.md) for internals.

---

## Reflection

The declared shape of a type, as data — what a generic serialiser, an ORM mapping
or a schema check needs.  Declared in `default/07_reflect.loft`.

| Function | Description |
|----------|-------------|
| `type_of(x) -> TypeInfo` | The declared shape of `x`'s type. **The argument is read for its TYPE and is not evaluated** (the contract C's `sizeof` has), so pass a variable, a field or a parameter rather than an expression with a side effect. |
| `type_named(name: text) -> TypeInfo?` | The declared shape of the type called `name`, or **null** when this program has no such type. Use when the name is a runtime value — a config file, a database catalogue, a command line — so there is nothing to call `type_of` on. |

`TypeInfo` carries `name`, `kind`, `size` (bytes per record), `fields`,
`variants`, `element`, `collection` and `keys`; each `FieldInfo` carries `name`,
`type_name`, `position`, `kind` and `nullable`.  Match on `kind` first: only a
record and a struct-enum variant have `fields`, only an enum has `variants`, and
only a vector or a keyed collection names an `element`.  Empty is the honest
answer for a kind that has no such thing.

```loft
t = type_of(row);
println("{t.name} ({t.size} bytes)");
for f in t.fields { println("  {f.name}: {f.type_name} @{f.position}") }
```

`TypeKind` is `IntegerKind` · `LongKind` · `SingleKind` · `FloatKind` ·
`BooleanKind` · `TextKind` · `CharacterKind` · `RecordKind` · `EnumKind` ·
`VariantKind` · `VectorKind` · `KeyedKind` · `RefKind` · `OtherKind`.  A kind
this loft version has no name for is reported as `OtherKind`, never guessed at.

### A keyed collection: which one, and on which fields

`KeyedKind` says a type is walked by cursor and stops there, which is not enough
to derive a query FROM.  `collection` says which of the five it is —
`KeyedHash` · `KeyedIndex` · `KeyedSorted` · `KeyedOrdered` · `KeyedRadix`, and
`NotKeyed` for every other type — and `keys` lists its key fields **in key
order**, each a `KeyInfo` of `name`, `position` and `ascending`.

```loft
t = type_of(people);                       // hash<Person[id]>
if t.collection == KeyedHash {
  for k in t.keys { println("key {k.name} @{k.position}") }
}
```

- **`position` is the ELEMENT record's byte offset** — the same number that
  element type's `FieldInfo.position` carries, so a key joins to a field by
  value.  A name is a display fact; the position is what the collection keys on.
- **`ascending` is `true` for a kind with no order of its own** (`KeyedHash`,
  `KeyedRadix`), because there is no descending answer to give.  Match
  `collection` first where "ascending" and "unordered" differ: only the three
  ordered kinds can serve a range.
- **`KeyedSorted` vs `KeyedOrdered`** is the same declaration over a different
  structure: `sorted<T[…]>` is a tree, and becomes a sorted by-value vector
  (`KeyedOrdered`) when `T` is co-located by a `hash` / `index` / `spatial`
  field elsewhere.  Both are ordered.
- **`keys` is delivered whole or empty.**  A query built from half a composite
  key reads the wrong rows, so a key the descriptor cannot resolve to an element
  field drops the whole list — which is also what a `__nullable<S>` element
  reports, because its keys live in the `Some` payload rather than in the
  element node.

Two limits worth knowing before you reach for it:

- **Not inside a generic.**  A generic body is parsed once against its type
  variable, so `type_of(v)` there answers `__typevar_T` — the same reason
  `"{v:j}"` in a generic body renders `{}`.  Call it where the concrete type is
  known.
- **Storage where the declaration cannot be recovered.**  A narrow `i32` field
  reports `IntegerKind`; its width is in `size`, not in a separate kind.
  `boolean` and `character` report what was declared.

Read-only, and it describes a TYPE.  `nullable` is reported even though it is not
a layout fact — a nullable field occupies the same bytes and spells absence with
a sentinel — because the compiler records it for you; it is what a generated
`CREATE TABLE` needs for `NOT NULL`.  Whether a field is `const` is NOT reported:
it constrains loft code rather than data.  WRITING a value by field name is
deliberately still out of scope (@PLN127).

### Reading a value: `field_value`

`type_of` says a record has a `text` at byte 16; `field_value(x, position)` reads
it.  Together they are what a generic serialiser, an ORM write or a value diff
needs — neither half is enough alone.

| Function | Description |
|----------|-------------|
| `field_value(x, position: integer) -> ValueInfo` | What `x` holds at that byte position, tagged with the kind the type's own descriptor gives it. |
| `field_value(x, path: vector<integer>) -> ValueInfo` | The same, reached through a chain of INLINE record fields — `field_value(doc, [8, 0])` is `doc.origin.x`. A one-element path answers exactly what the bare position does. |

**A nested record needs the path form, and the offsets must not be added up.**
`type_of` reports a nested record's fields at positions relative to THAT record,
and a `ValueInfo` for a record field carries no handle to call `field_value` on
again — so one number cannot name `origin.x`. Summing them does not work either,
and that is the point rather than a limitation: the check that makes this
ordinary loft rather than a pointer is that a position must **begin a field of
the type it is read against**, and `origin`'s offset plus `x`'s begins nothing in
the owner. The walk therefore happens inside, one descriptor step per element,
each checked the same way.

Every step but the last must land on an inline record. A step onto a scalar, a
vector, a keyed collection or a stored reference answers `OtherKind` and reads
nothing — a stored reference names a record with its own identity, and following
it would be a pointer chase rather than a field read. An empty path answers
`OtherKind` for the same reason a bad position does.

`ValueInfo` carries `kind`, `is_null`, and three payloads — `i` (integer, long,
boolean as 1/0, character as its code point), `f` (float, single) and `t` (text).
Match `kind` to know which one carries the answer.

```loft
t = type_of(row);
for f in t.fields {
  v = field_value(row, f.position);
  if v.is_null { println("{f.name} is null") }
  else if v.kind == TextKind { println("{f.name}={v.t}") }
  else { println("{f.name}={v.i}") }
}
```

- **The position is CHECKED against the descriptor, never trusted.**  Only a
  field the type says BEGINS there is read.  A hand-made offset, a stale one, one
  belonging to a different type, the interior of a wider field, or an `index`
  element's `#left_1` bookkeeping all answer `OtherKind` with no read at all — so
  no argument reads bytes the layout did not name.
- **`kind` is the descriptor's answer, not yours.**  Passing the position of an
  `integer` field does not make the reading an integer; it makes it whatever that
  field is.  That is why the kind is reported rather than requested.
- **Three answers stay distinct**: `OtherKind` (nothing begins there), a
  non-scalar kind (a nested record, a vector, a keyed collection, a stored
  reference — nothing to read out), and `is_null` (the scalar holds loft's null).
  `is_null` is `false` for the first two, because a field that was never read is
  not a field that read as null.
- **`null` is not `""`, `0` or `false`.**  A null `text?` answers `is_null`; an
  empty `text` answers a zero-length string.  An ORM that confused them would
  write the wrong row, which is why the flag rides beside the payload rather than
  inside it.
- **A narrow field is read at ITS width**, with its own sentinel — an `i32` field
  reports `IntegerKind` and does not take its neighbour's bytes.
- **REFUSED inside a generic**, and refused rather than quietly unsupported.  A
  generic body is parsed once against its type variable, so there is no concrete
  type; left to answer, every call there reports `OtherKind`, which for an ORM is
  an EMPTY ROW rather than an error.  It is a compile error naming the type
  variable — call it one frame out and pass the values in.

The two halves have one consumer each in the tree:
`tests/scripts/pln127-reflect-consumer.loft` generates `CREATE TABLE` from a loft
struct through the type half alone, and `tests/fixtures/sqldb/round_trip.loft`
builds an `INSERT`'s values through this one (@PLN23 S5) — walking the table
definition's columns, each of which carries the byte position of the field that
fills it.

### The other way round: `for f in s#fields`

`s#fields` is SYNTAX, not a function: the loop is unrolled at compile time into
one block per field, and `f` is a `StructField` carrying `name`, `value` and
`nullable`.

```loft
for f in m#fields {
  if f.value is FvText { v } { println("{f.name} = {v}") }
  if f.value is FvInt  { v } { println("{f.name} = {v}") }
}
```

`value` is a `FieldValue` struct-enum — `FvBool` · `FvInt` · `FvLong` ·
`FvFloat` · `FvSingle` · `FvChar` · `FvText` — so the payload arrives already
typed and no position arithmetic is involved.

**Which of the two to reach for.**  `#fields` gives you per-field CODE with typed
payloads and no runtime cost; `type_of` + `field_value` give you per-field DATA
you can carry, index and compare.  Use `#fields` to *write* something per field;
use reflection when the field list itself is the value — when it has to line up
with a table definition, a wire format, or another type's fields.

Three things to know:

- **Only scalar fields are visited.**  A nested record, a vector or a keyed
  collection has no `FieldValue` payload and is skipped.  `type_of(x).fields`
  reports those, so the two lists agree on scalars and reflection is the longer
  one.
- **`nullable` says a sentinel is possible.**  The payload variants are typed
  non-null and `τ?` shares `τ`'s runtime layout, so a null field's value arrives
  inside `FvText` / `FvInt` / … as loft's null.  The flag is what tells you that
  can happen; it mirrors `FieldInfo.nullable`.  (Nullable fields used to be
  skipped entirely — the loop simply ran fewer times than the struct has fields,
  including for nullable fields holding real values.  Fixed; guarded by
  `tests/scripts/pln23-field-iter-nullable.loft`.)
- **It is an unroll, so it costs code.**  Two work-refs per field per loop, live
  until scope exit.  Collect several answers in ONE loop rather than writing
  three loops over the same struct.

---

## Environment

Functions for interacting with the host operating system.

### Command-Line Arguments

| Function | Description |
|----------|-------------|
| `arguments() -> vector<text>` | Returns the command-line arguments passed to the program. The first element is typically the program name. |

### Environment Variables

| Function | Description |
|----------|-------------|
| `env_variable(name: text) -> text` | Returns the value of the environment variable `name`, or **`""` if it is not set** — measured on both backends. |
| `env_variables() -> vector<EnvVariable>` | Returns all environment variables as a vector of `EnvVariable` records (fields: `name`, `value`). |

> **An unset variable answers `""`, not null**, so `env_variable("X") ?? "default"`
> **never fires** — the `??` is dead code and a test written on it passes whatever
> the program does. Treating empty as absent is the caller's job:
> `v = env_variable("X"); if v == "" { v = "default" }`. (The return type is `text`,
> not `text?`, which is why this is consistent rather than a bug — but it reads as
> one, and it silently voided a real test in @PLN23.)

### Paths

| Function | Description |
|----------|-------------|
| `directory(v: &text = "") -> text` | Returns the current working directory, optionally with `v` appended as a subpath. Use to construct absolute paths relative to where the program was launched. |
| `user_directory(v: &text = "") -> text` | Returns the current user's home directory, optionally with `v` appended. |
| `program_directory(v: &text = "") -> text` | Returns the directory containing the running executable, optionally with `v` appended. |

### Memory diagnostics

| Function | Description |
|----------|-------------|
| `store_memory() -> text` | Returns a multi-line snapshot of all LIVE heap stores' internal utilisation — total capacity vs actual claimed data vs free space, record + free-block counts, **mergeable adjacent-free pairs** (free neighbours that should have coalesced), **`tail%` / `inner%`** (see below), and the largest stores by capacity with their type name and creation site (`bc:<pos>` — a bytecode position on the interpreter, mapping to source via `LOFT_LOG=static`; `0` on `--native`). Use to watch memory growth / fragmentation in a running program. See also `LOFT_STORES=log\|warn` (alloc/free trace). |
| `store_reclaim(collection) -> integer` | Give back the free space at the END of a store-rooted collection's store, and answer with the BYTES handed back (`0` when there was nothing). For a collection bound with `store_persist_bind` that is the **file** shrinking; otherwise it is memory returned to the allocator. Records never move, so every reference stays valid. It keeps an eighth of the live content as slack — the store stays in use, and one trimmed to the byte would pay a 2.33× re-grow on its next claim — so a store already at that size answers `0`, and asking twice is free. Returns `0` and changes nothing for a store that is read-only, shares another store's memory, or carries a `store_durable_seal` sidecar. |
| `store_release(collection) -> integer` | Say "everything I have written so far is finished": start writing it out to the bound file and stop holding it in memory, answering the BYTES dropped from the resident set (`0` when there was nothing to drop, when the collection is not bound to a file, or when the TARGET cannot honour the hint — the drop is `madvise(MADV_DONTNEED)`, so a build without `mmap` and any non-unix target answer `0` by construction; the call is a residency hint, and a program running where it cannot be honoured is not a program that is wrong). For a GENERATOR streaming a large collection into a store bound with `store_persist_bind` — measured on a 20 000-record build, one call per record: peak memory **44.3 MB → 2.2 MB (20×) at no cost in wall clock**. Content is untouched and every reference stays valid; reading a released record re-reads it from the file at the cost of one page fault, so this is a hint that can cost speed and never an answer. **It pays when records are written in KEY ORDER and not returned to** — a build that keeps many records open at once scatters the arena with free blocks (3 691 against 10) and the allocator then keeps re-reading them, giving 1.0× instead of 20×. Not `store_reclaim`, which changes the file's LENGTH; this never does. Not a durability barrier — it asks for writeback to *start*, and `store_durable_seal` is what promises it landed. |

**`tail%` and `inner%` say WHERE a store's free space sits**, which is the
difference between free space you get back and free space you do not.

- **`tail`** — above the last record. This is what `store_reclaim` returns, less
  the eighth it leaves behind. A persisted store's image already ends at the last
  record, so the tail is arena capacity, not file bytes — until the store is
  BOUND, where it is both. ⚠ **On a bound store, MID-RUN, that tail is why the
  FILE SIZE compares nothing**: capacity grows by 7/3 and never shrinks by
  itself, so between the bind and the release the file is a rung on a ladder —
  two points a rung apart differ by 133% holding identical records, and one
  holding twice the data can be byte-identical. Call `store_reclaim` before
  reading a size in the middle of a run. The file a program LEAVES BEHIND needs
  no such call: releasing the collection hands the tail back, so the finished
  file follows its content (loft#752).
- **`inner`** — between records. It is reusable for future allocation, but it
  *is* written to the file, because the image has to span up to the last record.
  `store_reclaim` does not touch it — **loading the store does**, automatically:
  a collection read back by `store_load`, or bound to an existing file, is
  rebuilt dense when its interior is worth it (@PLN123 arc B, see
  [DATABASE.md](DATABASE.md)). So a high `inner` is a number you act on with the
  next load, not with this call.

A store built once reads `inner 0%`. One whose live set fell well below its peak
reads a high `inner` — 71% in the case behind loft#713 — and that is the part
only relocation could recover. Coalescing does not touch it: forcing a sweep took
2,700 free blocks to 6 and left `inner` unchanged at 45%, because merging free
blocks never moves a live one.

**Reading the two before calling `store_reclaim`** is the whole workflow: `tail%`
is what you would get, `inner%` is what you would not.

**You say when, and that is deliberate.** Only a live set that drops far below
its peak *and stays there* has anything to give back — whether a drop is
permanent is something the program knows and the runtime cannot infer. A
measured steady-state churn (40 cycles of +300/−300 records over a 2,000-record
hash) ends at 0.29 MB / 56% used if left alone; calling `store_reclaim` every
cycle ends denser, at 0.17 MB / 93% used, but moves **9.5 MB** of grow-and-shrink
traffic to get there — 55× the store's own size, to save 0.11 MB. Called once
after a permanent drop it costs one walk; called on a cycle it pays for a re-grow
every time.

---

## Time

Two clocks, answering two different questions. Both return loft's 64-bit `integer`, which a
millisecond epoch stamp needs. See [tests/docs/22-time.loft](../22-time.loft) for the
chapter and `tests/scripts/the-reference-clock-units-are-the-ones-it-names.loft` for the
guard that pins the units against each other.

| Function | Description |
|----------|-------------|
| `now() -> integer` | Wall-clock time as **milliseconds** since the Unix epoch (1970-01-01T00:00:00 UTC). For timestamps and anything compared against a calendar. It reads the SYSTEM clock, so an NTP correction or a manual change can step it in either direction — never subtract two `now()` values to time something. |
| `ticks() -> integer` | **Microseconds** elapsed since program start, from a monotonic clock. Never steps backward whatever happens to the system clock, which is the guarantee `now()` does not carry. For benchmarks and frame timing. |

There is no calendar in the standard library — no year/month/weekday, no formatting. That
is the **`time` package**'s subject (`loft install time`, then `use time;`), and it works on
the same millisecond-since-epoch integer `now()` returns, so nothing is converted:
`format_iso(now())`, `weekday_name(now())`, `add_days(now(), 30)`.

---

## Random — `use random;`

These live in the **`random` package**, not the always-loaded stdlib. Add
`use random;` (or call them qualified, `random::…`) or the names do not resolve;
loft names the package for you when they are missing.

A fast PCG64 generator, seeded with a fixed default at startup. Call `rand_seed` before use
when reproducibility matters.

| Function | Description |
|----------|-------------|
| `rand(lo: integer, hi: integer) -> integer` | Returns a uniformly distributed random integer in `[lo, hi]` (inclusive). Returns null if `lo > hi` or either bound is null. |
| `rand_seed(seed: integer)` | Seeds the thread-local RNG. Same seed always produces the same sequence. |
| `rand_indices(n: integer) -> vector<integer>` | Returns a vector of `n` integers `[0, 1, ..., n-1]` in a random order. Empty when `n ≤ 0`. Useful for random iteration or sampling without replacement. |

**Example — pick 3 distinct items at random:**
```loft
use random;
fn main() {
  rand_seed(42);
  items = ["a", "b", "c", "d", "e"];
  order = rand_indices(len(items));
  for i in 0..3 { println(items[order[i]]) }
}
```

---

## Open work

Stdlib gaps surfaced by dogfood usage (the @PLAN35 viewer,
@PLN42 tag indexer, lib/markdown extraction).  Each row
names where it bit, the proposed shape, and rough effort.
XS = single-line/single-fn change; S = focused half-day fix.

| Item | Where it bit | Shape | Effort |
|---|---|---|---|
| `vector.sort()` (text added 2026-05-18); `vector.sort_by(fn)` deferred | scan.loft (3 sites), viewer's plan-bucket sort, activity feed date sort | Pre-existing `sort(v)` builtin extended to dispatch on text element type via `vector::sort_text_vector` (lexicographic, sorts u32 string offsets by what they point at).  `sort_by(fn)` for user types still open — needs callback-passing infrastructure.  Replaces the `sorted<T[K]>` set-as-sort-proxy pattern for text. | **`sort()` text-element shipped (@PLN42 phase 10.8)**; `sort_by(fn)` open. |
| JSON emission helpers | scan.loft has 80+ lines of manual `json_escape` + per-row format-string emission + comma management.  viewer reads via `value.field("x").as_text()` — no symmetric write API. | `to_json(value) -> text` for primitives + `JsonBuilder` for nested structures.  Mirror of the existing `json_parse` + `JsonValue` read API. | S–M |
| ~~Path helpers — stdlib `path` module~~ — shipped 2026-05-18 as text methods | scan.loft, viewer, lib/markdown each rolled their own `dir_of` / `basename` / `resolve_relative` | Pure-loft as four text methods in `default/03_text.loft`: `p.dir()`, `p.basename()`, `p.join(other)`, `p.resolve(target)`.  Module-prefix style (`path::dir(p)`) couldn't ship — loft uses self-type method dispatch, not module namespaces.  `file().path` `./<name>` normalisation deferred to a follow-up. | **Shipped (@PLN42 phase 10.9)** |
| ~~`text.split(text)`~~ — shipped 2026-05-18 as `text.split_text(text)` | scan.loft's link extractor walks char-by-char to find `](` (only `text.split(char)` exists today) | Renamed from `split` to `split_text` because loft doesn't allow fn overloading by non-self parameter type — `split(text, character)` and `split(text, text)` collide on the name `split`.  Underlying overloading limitation deserves its own follow-up (file when a second consumer hits it). | **Shipped (@PLN42 phase 10.4)** |
| ~~`text.starts_with_at(pos, prefix)`~~ — shipped 2026-05-18 | scan.loft's @PLAN matcher does `line[i+1]=='P' && line[i+2]=='L' && …` instead of `line.starts_with_at(i, "PLAN")` | Sugar over the existing slice + comparison.  Pure-loft body in `default/03_text.loft`; works in both backends. | **Shipped (@PLN42 phase 10.5)** |
| ~~`hash.contains(key) -> boolean`~~ — **deferred (not XS)** | scan.loft uses `vector<text>` + linear `set_contains` for valid_pids/valid_plans because `hash<T[K]>` isn't ergonomic as a "set of text" | The XS pitch assumed a simple sugar method.  Reality: `hash<T[K]>` keys are typed per-instance, so a generic `contains()` needs parser-level typed-dispatch — M+ work, not XS.  Use `h[key] != null` or `if h[key] { … }` idiom (established pattern in `tests/scripts/32-collections-regressions.loft`).  The deeper "set of text without wrapper struct" gap is a bigger language feature; defer until a second consumer asks for the sugar. | **Wontfix unless a 2nd consumer demands it** (@PLN42 phase 10.6 deferral) |
| ~~`text::escape_html(s)`~~ — shipped 2026-05-18 | viewer's main.loft rolled its own `escape(s)` for HTML output | Pure-loft.  Escapes the standard 5 entities (`&`, `<`, `>`, `"`, `'`) safely for both element bodies and attribute values.  **Drained out of the default stdlib into `lib/html/` (lib_plans/12 Phase 3.6, 2026-05-27) — now opt-in via `use html;`.** | **Shipped (@PLN42 phase 10.7); moved to `lib/html`** |
| `args() -> vector<text>` builtin | scan.loft uses env var `LOFT_INDEX_BUCKETED` as a CLI-arg workaround; viewer doesn't support args at all | Add the builtin that returns the program's invocation args. | XS |

Driver doc: see the "Loft gaps surfaced" section in
[`plans/42-tracker-index/07-loft-native-scanner.md`](plans/42-tracker-index/07-loft-native-scanner.md)
for the consumer-side narrative.

## See also
- [LOFT.md](LOFT.md) — Loft language reference (syntax, types, operators, control flow)
- [INTERNALS.md](INTERNALS.md) — Native function registry, `src/native.rs`, `src/ops.rs`
