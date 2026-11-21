# @PLN91 — an appended record item that reads the vector it is appended to: the two cases left (prepared subject)

**The family:** "an appended record item that reads the vector it is appended to (a snapshot)"
(4 sites, 2 programs held by it alone).  The port already carries the snapshot for one record
item on a plain local (el_ex_pattern ~2811: the vector copied into `_build_src_N`, the item's
reads renamed onto the copy).  Each sole program hits a different guard at ~2829:
```
157-record-push.loft:79                     out += [Pt { ptx: ((len(out) * 10) as float) ?? 0.0, pty: 0.0 }]
   → `vars.vars[v].argument`: `out` is the function's RETURNED vector, renamed onto the hidden
     work-buffer parameter (`out: vector<Pt> hidden`, `-> vector<Pt>`), so it reads as an argument
R-PushRec/a-record-append-takes-its-growth-before-its-field-writes.loft:57
                                            v += [T { n: i, s: "x{i}", xs: [i, len(v)] }]
   → `holds_vector_literal(e, it)`: the item has a vector-literal field (`xs: [i, len(v)]`),
     whose items are staged first (`stage_append_fields`)
```
(The two non-sole sites, H-LongRef :87 and :129, take `&c[…]` references in the item.)

**The rule:** `@FR-I-Comp` / the append's snapshot: an item reads the vector as the statement
began, before the append grows it.

## Rust's forms, measured on try 5ed1570bb

```
157:79   (out: the hidden returned vector)   for _ in 0..n { out += [Pt { ptx: ((len(out) * 10) as float) ?? 0.0, pty: 0.0 }] }
  __vdb_1: ref(main_vector<Pt>) = null;                              <- declared at the function's top
  … loop { … {#block
      OpDatabase(__vdb_1, 104);  _build_src_1: vector<Pt>["__vdb_1"] = OpGetField(__vdb_1, 0, 105);  OpSetInt4(__vdb_1, 0, 0);
      OpAppendVector(_build_src_1, out, 100);                         <- the snapshot, exactly as for a local
      OpPreAllocVector(out, 1, 16);  _elm_1: ref(Pt)["out"] = OpNewRecord(out, 105, 65535);
      OpSetFloat(_elm_1, 0, {#ncc … OpConvFloatFromInt(OpMulInt(t_6vector_len(_build_src_1), 10)) … });
      OpSetFloat(_elm_1, 8, 0f64);  OpFinishRecord(out, _elm_1, 105, 65535) } }

R-PushRec:57   struct T { n: integer, s: text, xs: vector<integer> }   v += [T { n: i, s: "x{i}", xs: [i, len(v)] }]
  __vdb_2 = null;  __ref_2: integer = null;  __ref_1: integer = null;   <- declared at the top; __ref_2 before __ref_1
  … {#block
      OpDatabase(__vdb_2, 107);  _build_src_1: vector<T>["__vdb_2"] = OpGetField(__vdb_2, 0, 108);  OpSetInt4(__vdb_2, 0, 0);
      OpAppendVector(_build_src_1, v, 102);
      OpPreAllocVector(v, 1, 16);  _elm_1: ref(T)["v"] = OpNewRecord(v, 108, 65535);
      OpSetInt4(_elm_1, 12, 0);                                       <- the vector field's header, FIRST
      OpSetInt(_elm_1, 0, i);  OpSetText(_elm_1, 8, {#Formatted string … "x" … i … });
      __ref_1: integer = i;                                           <- the literal's items staged, in order
      __ref_2: integer = t_6vector_len(_build_src_1);                 <- …reading the snapshot
      OpPushInt(OpGetField(_elm_1, 12, 7), __ref_1);
      OpPushInt(OpGetField(_elm_1, 12, 7), __ref_2);
      OpFinishRecord(v, _elm_1, 108, 65535) }
```

## What to code

Two independent widenings of the guard at el_ex_pattern ~2829:
- **the returned vector** (157): let a vector that is an argument only because it was renamed
  onto the hidden return buffer take the same snapshot.  Rust's form is identical to a local's,
  with `__vdb_N` declared at the function's top.  A true parameter stays refused until measured;
- **a vector-literal field** (R-PushRec): zero the field's header first (`OpSetInt4(elm, <pos>, 0)`),
  write the other fields, stage each literal item into a `__ref_N` in source order (reads of the
  vector go to `_build_src_N`), then push each staged value into the field (`OpPushInt(OpGetField(elm,
  <pos>, <vec row>), __ref_N)`).  Note the declaration order at the top: `__ref_2` comes before
  `__ref_1`.

## Each form judged

- **The snapshot** (`_build_src_N`, the item's reads renamed onto it): a RULE.  The item reads
  the vector as the statement began.
- **The same snapshot for the returned vector** (157): a RULE.  A vector does not read
  differently because it is renamed onto the return buffer.
- **The header zeroed first, the items staged into `__ref_N`, then pushed** (R-PushRec): a RULE
  of order.  Every value is read before the field it lands in grows.
- **`__ref_2` declared before `__ref_1`**: an ACCIDENT of internal numbers (@C168); carry it.
- **The snapshot's store is minted in each iteration** (`OpDatabase(__vdb_N)` inside the loop):
  its release is the scope pass's work, not visible in this oracle.  It is not judged here;
  scope_survey shows it before the port's scope pass carries it.

## Probes (the two sole programs)

```
kr-try/tests/scripts/157-record-push.loft:79
kr-suite/rules/rewrites/R-PushRec/a-record-append-takes-its-growth-before-its-field-writes.loft:57
```
