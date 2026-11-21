# @PLN91 — `x#errors` asks a parsed record or vector for its parse's errors (prepared subject)

**The family:** "a variable attribute", the `#errors` reason: 21 sites in 7 programs.  One program
is held by it alone, H-Claim/1995-a-parsed-record-has-every-field-written (`n_shape` :39,
`n_outer` :44, `n_many` :49).  The other six (57-json, 24-json, 2268, 1987, 2028,
a-checked-parse-answers-null-for-an-enum-field) carry it beside other refusals.  The port's
variable-attribute dispatch (el_ex_null ~672) has `#count`, `#first`, `#lock`, `#remove`,
`#index` and `#next`, but no `#errors`, so it refuses by name.

**The decision:** `@C148`: `s#errors` answers the errors of the parse that built the value `s`
holds, one element per error, as a fresh vector the caller owns.

## Rust's form

`Parser` in src/parser/objects.rs ~1075, after `#` on a variable:
```rust
self.var_usages(index_var, true);
if self.lexer.has_keyword("errors") {
    // second pass only: the variable must be a Reference, a struct-enum (Enum(_, true, _)) or a
    // Vector, else the error "`{name}#errors` asks a parsed record or vector …"
    let fn_nr = self.data.def_nr("n_parse_errors_of");
    if fn_nr != u32::MAX {
        *code = Value::Call(fn_nr, vec![Value::Var(index_var)]);
        t = self.data.def(fn_nr).returned.clone();          // vector<text>
    }
    return t;
}
```
`default/06_json.loft` ~237: `fn parse_errors_of(r: const reference) -> vector<text>;`

Measured on try b55f97520:
```
1995:39   fn shape(d: text) -> text { s = Shape95.parse(d); "{s} {s#errors}" }
  [38] s: ref(Shape95) = OpCastVectorFromText(d, 100);
  [39] {#Formatted string … OpFormatStackDatabase(__work_1, s, 100, 0);  OpAppendStackText(__work_1, " ");
          OpFormatStackDatabase(__work_1, n_parse_errors_of(s), 44, 0); … }   <- the call, formatted as vector<text>
```

## What to code

In el_ex_null's variable-attribute dispatch, `#errors` with no argument and no cast becomes the
call `n_parse_errors_of(<the variable>)`, typed as the function's return type.  The variable
is marked used.  On the second pass, refuse by name (Rust's error) when the variable is not a
reference, a struct-enum or a vector.

## Probes

```
kr-suite/rules/heap/H-Claim/1995-a-parsed-record-has-every-field-written.loft:39, :44, :49   (sole)
then the shared sites, e.g. 2028-both-parse-forms-name-every-mismatch-on-the-record :32 (`len(u#errors)`)
   and 57-json :110 (`err = s#errors`, a bind of the fresh vector)
```
