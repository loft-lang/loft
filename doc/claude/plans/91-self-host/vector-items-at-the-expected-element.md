# @PLN91 — vector literal items checked at the expected element: a `null` item, and a variant name two enums share (prepared subject)

**The family:** "a vector item of another type than the first": 34 sites.  None is held by it
alone, but it sits in many programs one or two refusals from agreeing (1416, 1739, 923, 1466,
N-Match/a-null-arm-matches-a-null-subject-of-any-kind, T-Chk-Vec, a-walk-over-a-link-parameter,
2557, …).  The port types a literal by its FIRST item and refuses an item of another type.  Rust
checks every item against the EXPECTED element.  This page takes the two most common forms:
```
a `null` item, the expected element nullable   (most sites: `C? then null`, `vector<integer>? then …`)
   N-Match:173   ev: vector<C?> = [null, B, A];
   1739:32       v: vector<vector<integer>?> = [[1, 2], null, [3]];
a variant name two enums declare, resolved at the expected enum   (`A then B`)
   T-Chk-Vec:46  enum A { X, Y }  enum B { Z, X }   fn tail_e() -> vector<B> { [X, Z] }
   T-Chk-Vec:42  struct FD { …, v: vector<B> = [X, Z], … }   (the field default)
```
The second form is what numbers the porter's T-Chk-Vec definitions +2.  The port resolves `X` as
`A::X`, declares `vector<A>`/`main_vector<A>` at 42:46, and only then refuses.  Rust never
declares them.  So the definitions after it read as differing until this form is carried.

**The rules:** `@FR-T-Chk-Vec` (`[e₁ … eₙ] ⇐ vector<τ>  ⟸  ∀i. eᵢ ⇐ τ`), with
`@FR-T-Chk-Var` (`V ⇐ Enum E  ⟸  V ∈ variants(E)`): checked against `B`, `X` IS `B::X`.  A
`null` item checks against a nullable `τ?` as `τ`'s null.

## Rust's forms, measured on try e420bc11c

```
N-Match:173   enum C { A, B, D }   ev: vector<C?> = [null, B, A];
  [173] OpDatabase(__vdb_1, 106);  ev: vector<C?>["__vdb_1"] = OpGetField(__vdb_1, 0, 107);  OpSetInt4(__vdb_1, 0, 0);
        OpPreAllocVector(ev, 3, 1);
        OpPushEnum(ev, OpConvEnumFromNull(101));       <- the null item: C's null (101 = C's definition)
        OpPushEnum(ev, 2u8(101));                      <- B
        OpPushEnum(ev, 1u8(101));                      <- A

T-Chk-Vec:46   fn tail_e() -> vector<B> { [X, Z] }      (B { Z, X }: Z = 1, X = 2)
  __vdb_1: vector<B> hidden   -> vector<B>
  {#Vector: vector<B>["__vdb_1"]  OpDatabase(__vdb_1, 110);  _vec_1 = OpGetField(__vdb_1, 0, 109);  OpSetInt4(__vdb_1, 0, 0);
     OpPreAllocVector(_vec_1, 2, 1);
     OpPushEnum(_vec_1, 2u8(105));                    <- X resolved as B::X (105 = B), not A::X
     OpPushEnum(_vec_1, 1u8(105));
     _vec_1 }
```

## Each form judged

- **Every item checked at the expected element; a variant name resolved in the expected enum:**
  a RULE (`@FR-T-Chk-Vec`, `@FR-T-Chk-Var`).
- **A `null` item pushed as the element type's null (`OpConvEnumFromNull(<enum>)` for an enum):** a
  RULE.
- **The port's `vector<A>` declared before the refusal:** a port DEFECT of numbering, not
  Rust's.  It goes once the item is resolved at `B`.

## What to code

Where the port builds a vector literal with a known expected element (a typed local, a
result's element through `expect_node`, a field default, a parameter's element hint):
- elaborate each item against that element, not against the first item's type;
- resolve a bare variant name in the expected enum first;
- push a `null` item as the element's null.

Start with the enum forms (N-Match:173 / :159 / :200 / :215, the T-Chk-Vec `tail_e`), then
`vector<vector<integer>?>` (1739:32).

## Probes

```
kr-suite/rules/types/N-Match/a-null-arm-matches-a-null-subject-of-any-kind.loft:159, :173, :200, :215
kr-suite/rules/types/T-Chk-Vec/an-expected-element-type-reaches-every-literal-position.loft:42, :46
kr-suite/rules/types/N-Shape/1739-a-null-inner-vector-element-is-absent-on-every-route.loft:32
```
(Run `where.tsv` for the family after the change: the tuple forms, such as `(integer, T8S?) then (…`,
are the next layer.)
