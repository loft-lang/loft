# To file: three comprehension findings beside the declared-element-type fix (2026-09-25)

<!-- Found while fixing `D-Comp-Decl` (formal/types.md).  This box's GitHub token was invalid
     for writes, so the issue texts wait here.  File each with `gh issue create` (labels below),
     then delete this file.  Every cell below was run on the pre-fix control build
     (`make falsify`'s cc67bbe71 worktree) and on the fixed tree; the verdicts are per build. -->

## 1. A typed local `v: vector<P?> = [for … { P { … } }]` is refused (regression since 2026.8.0)

`sev:medium area:parser hit-by:loft wa:clean` — the workaround is the FIELD form or the
`+=` form, both of which answer correctly.

```loft
struct P { x: integer, y: integer }
fn main() { v: vector<P?> = [for i in 0..3 { P { x: i, y: i * 2 } }]; println("{v[2]?.y} {len(v)}"); }
```

Expected `4 3` (the 2026.8.0 release prints it).  HEAD cc67bbe71 and the fixed tree refuse:
*"Variable 'v' cannot change type from vector<__nullable<P>> to vector<integer>"*.  The
nullable-hint arm of `parse_vector_for` hands the synthetic `__nullable<P>` to the body as its
expected type; on pass 1 that body types `Void` (recovered to `integer`), so the vector is retyped
`vector<integer>` on pass 1 against the declared `vector<__nullable<P>>`.  A struct FIELD of the
same type and a `v += [P { … }]` loop both work.  Not touched by the fix: the arm is deliberately
outside the declared-type conversion.

## 2. A nested comprehension into a declared `vector<vector<integer>>` is refused

`sev:low area:parser hit-by:loft wa:clean` — the workaround is a `vector<integer>`-typed local
for the inner comprehension, or a body that does not narrow (`i * j` instead of `(i * j) % 5`).

```loft
fn main() { v: vector<vector<integer>> = [for i in 0..3 { [for j in 0..3 { (i * j) % 5 }] }]; println("{v}"); }
```

Expected `[[0,0,0],[0,1,2],[0,2,4]]`.  Refused on HEAD (*"cannot change type from
vector<vector<integer>> to vector<vector<integer(-4, 4)>>"*) and on the fixed tree (*"cannot
store vector<integer(-4, 4)> elements in a vector<vector<integer>>"*).  The inner comprehension
is parsed with no expected type, so it infers the narrow element and a vector cannot convert.
The cure is the literal element's rule (@PLAN58 III-a: the declared element type is the body's
expected type) — see finding 3 for why it is not applied yet.

## 3. Seeding the declared element type into a comprehension body makes `w.ns[i] ?? []` a use-after-free

`sev:high area:scopes hit-by:loft silent-wrong wa:none-needed` — LATENT: no program reaches it
today, because the seeding is not done.  It blocks finding 2.

With `body_expected = in_t.clone()` for every declared element type (one line in
`parse_vector_for`), `tests/scripts/1195-a-comprehension-reads-its-destination-field.loft`'s
`test_the_element_type_is_varied` fails on the interpreter with
`[strict-store] USE AFTER FREE (read) store #3 type=Wide rec=15 pos=12` at

```loft
w.ns = [for i in 0..w.ns.len() { w.ns[i] ?? [] }];   // w.ns: vector<vector<integer>>
```

The hint types the `[]` arm (`seed_leaving_value_hint` seeds a collection result), and that
arm's fresh vector inside the `(I-Comp)` field-buffer route is read after its free — the
"fresh-vector-in-arm double `OpFreeRef`" `parse_block_inner`'s @PLN90 W8 comment already calls
pre-existing.  The route owes the fix before the hint can be handed down.
