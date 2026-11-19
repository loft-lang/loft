# @PLN91 — a non-boolean operand of `&&` / `||` (prepared subject)

**The family:** "a `&&` or `||` operand that is not a boolean" (25 sites in 13 programs at try
7ae757a80; the sole refusal of 3). Its operands are a nullable value (`p && true` for `p: S?`,
a `text?` call), a vector (`v && true`, `while w && c < 2`), or a text.

**The rule:** `(E-Truthy)` / `(E-Truthy-1)` (formal/operational.md). Both operands of `&&`/`||`
are truthiness positions: falsity is ABSENCE and nothing else, a present value is true, an
empty vector `[]` is true, and the result is a definite two-state boolean.

**Judged right** on the interpreter, --native-c and --native, hand-computed from the rule:
`mk(1) && true` true, `mk(0) && true` false (null), `false || p` true, `q || false` false,
`nt(1) && true` true, `nt(0) && true` false, `[] && true` true (non-optional heap),
`while [1] && c < 2` runs twice.

## Rust's form

```
A && B        if <truth of A> <B> else false
A || B        if <truth of A> true else <truth of B>

the truth of an operand, by its type:
    boolean                 the value itself                       (b)
    a record, nullable      OpConvBoolFromRef(<e>)                 (p, n_mk(k, __ref_1))
    text                    OpConvBoolFromText(<e>)                (n_nt(1, …))
    a vector                OpNot(OpVectorIsNull(<e>))             (v, w)

fn via(v: vector<integer>) -> boolean { v && true }
    if OpNot(OpVectorIsNull(v)) true else false;
fn orr(b: boolean, k: integer) -> boolean { b || mk(k) }
    if b true else OpConvBoolFromRef(n_mk(k, __ref_1));
fn andd(k: integer, b: boolean) -> boolean { mk(k) && b }
    if OpConvBoolFromRef(n_mk(k, __ref_1)) b else false;
while w && c < 2 { … }
    if OpNot(if OpNot(OpVectorIsNull(w)) OpLtInt(c, 2) else false) { break(0) } else null;
```

**What to code:** an operand of `&&`/`||` whose type is not boolean is converted to its truth
value by type, as tabled above (a vector by `OpNot(OpVectorIsNull)`, even a non-optional one;
Rust doesn't fold it to `true`), then `&&` lowers to `if A B else false` and `||` to `if A true
else B`. The same truth conversion applies to the RIGHT operand of `||` (`OpConvBoolFromRef`
in `orr`); a boolean right operand of `&&` is used as it is (`b` in `andd`).

## The sites (try 7ae757a80)

```
kr-suite/rules/operational/E-Truthy-1/1922-a-vector-slice-is-a-condition.loft::n_test_a_slice_operand_and_negation:49
kr-suite/rules/operational/E-Truthy/1919-a-vector-in-a-truthiness-position-is-true.loft::n_test_an_absent_vector_is_false:60
kr-suite/rules/operational/E-Truthy/1919-a-vector-in-a-truthiness-position-is-true.loft::n_test_a_vector_on_either_side_is_true:45
kr-suite/rules/operational/E-Truthy/1919-a-vector-in-a-truthiness-position-is-true.loft::n_test_control_struct_and_text:90
kr-suite/rules/operational/E-Truthy/1919-a-vector-in-a-truthiness-position-is-true.loft::n_test_every_element_kind_and_a_field_is_true:55
kr-suite/rules/operational/E-Truthy/1919-a-vector-in-a-truthiness-position-is-true.loft::n_test_every_provenance_and_collection_kind:96
kr-suite/rules/operational/E-Truthy/1919-a-vector-in-a-truthiness-position-is-true.loft::n_test_every_truthiness_position:78
kr-suite/rules/operational/E-Truthy/1919-a-vector-in-a-truthiness-position-is-true.loft::n_via:39
kr-suite/rules/operational/E-Truthy/a-boolean-operator-answers-a-definite-boolean.loft::n_main:140
kr-suite/rules/operational/E-Truthy/a-boolean-operator-answers-a-definite-boolean.loft::n_main:172
kr-suite/rules/types/N-Store/1660-a-nullable-into-a-wide-element-warns-at-any-index.loft::n_main:40
kr-try/lib/docs.loft::t_5Lexer_identifier:302
kr-try/lib/parser.loft::t_5Lexer_identifier:302
kr-try/tests/docs/15-lexer.loft::t_5Lexer_identifier:302
kr-try/tests/docs/16-parser.loft::t_5Lexer_identifier:302
kr-try/tests/scripts/1092-lexer-revert-restores-the-cursor.loft::t_5Lexer_identifier:302
kr-try/tests/scripts/1259-the-parser-library-accepts-the-language-it-documents.loft::t_5Lexer_identifier:302
kr-try/tests/scripts/1339-the-parser-is-not-charged-for-a-prelude-that-never-existed.loft::t_5Lexer_identifier:302
kr-try/tests/scripts/174-foreign-file.loft::n_main:112
kr-try/tests/scripts/1800-the-parser-library-reads-while-match-and-lambdas.loft::t_5Lexer_identifier:302
```
