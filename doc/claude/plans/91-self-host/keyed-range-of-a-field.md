# @PLN91 — a `for` over a keyed range of a field, and a range with text bounds (prepared subject)

**The family:** "an index that is a range, a key list or a limit" (23 sites in 18 programs at
try e27ec2076).  Its keyed part is `for e in <record>.<sorted or index field>[lo..hi]`:
`for s in sdb.map[15..25]` (tests/scripts/23-field-overlap-structs, held by it alone),
`db.circles[15..25]`, `db.rows[6..12]`, `a.m[1..4]` (1272, beside `#remove`),
`d.words["banana"..="date"]` (tests/docs/10-sorted, also refused as "a key range whose bound
is not an integer: TyText").  The family's text slices of a `&text` (1011, R-SliceBorrow, 1357,
192) are the porter's 2f3471676, in its tries.  Its compound spatial keys (`s[3, 4]`) are
another shape and aren't part of this page.

**Judged right** on the interpreter (LOFT_POISON), --native-c and --native-rust (leak check), values
hand-computed; a wrong expected value fails the run.  Probe:
[probes/keyed-range-walk.loft](probes/keyed-range-walk.loft), 12 cases.  On a15f52718 the
LOCAL case (`local_sorted`) agrees and every FIELD case is refused.  A local over integer keys
agrees for every bound form (`..=`, open start, open end, a `[-k]` key, an index).  A local
with TEXT bounds is refused ("a key range whose bound is not an integer").

## Rust's form: the local's walk, the subject read as the place

```
for r in s[10..25]  (local)        _iter_1 = OpIterate(s, 3, 4, &[Key{type_nr: 1, position: 0, start: 0}], 1, 10, 1, 25);
                                     loop { r: ref(R)["s"] = {#Iterate keys OpStep(_iter_1, s, 3, 4) } … }
for r in d.m[15..35]  (field)      _iter_1 = OpIterate(OpGetField(d, 0, 102), 3, 4, &[Key{…}], 1, 15, 1, 35);
                                     loop { r: ref(R)["d"] = {#Iterate keys OpStep(_iter_1, OpGetField(d, 0, 102), 3, 4) } … }
for w in d.words["banana"..="date"] _iter_1 = OpIterate(OpGetField(d, 0, 109), 130, 16, &[Key{type_nr: 6, position: 8, start: 0}], 1, "banana", 1, "date");
d.m[45..15] over sorted<R[-k]>     … &[Key{type_nr: -1, …}], 1, 45, 1, 15   (the bounds in the key's own order)
```

The field is read again at every `OpStep` (no hold), and the loop variable borrows the place's
ROOT (`["d"]`).

## What to code

1. **`key_range_local`** (el_ex_match.loft ~978) admits only a bare local (`SName`, no member).
   Admit a field place of a local whose type is a sorted or an index: `coll` is the place's
   read code (`OpGetField(root, …)`), used at both `OpIterate` and `OpStep`, and the element's
   deps are the root's.
2. **The bound type** (~1023, ~1034): a bound is converted to the key field's type, not
   required to be an integer.  A text key takes a text bound as is (`"banana"`).

Leave alone: a range over more than one key (`db.map[83..92, "Two"]`, 11-index; its own
refusal); `rev(s[..])` (689); `#remove` inside the walk (1272's other refusal).

## The sites (try e27ec2076), the keyed part

```
kr-try/tests/scripts/23-field-overlap-structs.loft:49        for s in sdb.map[15..25]        (sole)
kr-try/tests/scripts/25-sorted-enum-variant-range.loft:37    for c in db.circles[15..25]
kr-try/tests/scripts/158-keyed-fast-paths.loft:255           for e in db.rows[6..12]
kr-try/tests/docs/10-sorted.loft:152                         for w in d.words["banana"..="date"]
kr-try/tests/scripts/1272-a-remove-inside-a-keyed-range.loft:57,77,95,106,120,138   (+ `#remove`)
```
