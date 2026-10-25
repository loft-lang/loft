<!-- reference for the loft-write skill: read when a program uses a tuple -->
# loft-write: tuples reference

The tuple half of the [loft-write skill](SKILL.md).

## Tuples

Anonymous, fixed-arity, stack-allocated compound values.  Use them to
return multiple values without naming a struct.  See
[doc/claude/TUPLES.md](../../../doc/claude/TUPLES.md).

<!-- from tests/reference/skill-tuples.loft -->
```loft
// Type notation — two or more element types (single-element tuples
// are not allowed; use the bare type instead).
t: (integer, text) = (3, "hi");

// Literal
pair = (1, "hello");

// Element access (zero-based integer literal — NOT a variable index).
a = pair.0;        // integer
b = pair.1;        // text

// Element assignment
pair.0 = 5;
pair.0 += 3;

// Destructuring
(lo, hi) = min_max(values);
```
<!-- from tests/reference/skill-tuples.loft -->
```loft
// Function return
fn classify(t: (integer, text)) -> text {
    match t {
        (0, _)   => "zero",
        (n, msg) => "{n}: {msg}",
    }
}
```
<!-- from tests/reference/skill-tuples.loft -->
```loft
// Nested tuples — chain the `.N` accessors.
nested: ((integer, integer), boolean) = ((1, 2), true);
inner_first  = nested.0.0;       // 1
inner_second = nested.0.1;       // 2
flag         = nested.1;          // true
```

**Restrictions:**
- **No single-element tuples** — `(integer)` is just `integer`.
- **No named tuple fields** — use a struct.
- **No tuple iteration / whole-tuple formatting** — access elements one
  by one.
- **Scalar elements are NON-NULL by default (@PLN25 DN1)** — a tuple
  `integer` / `text` / … element rejects `null`; append `?` to the element
  type (`(integer?, text)`) to allow it.
- **Compound assignment on tuple LHS is rejected** — `(a, b) += (1, 2)`
  is a compile error; rewrite as `a += 1; b += 2;` or rebuild the tuple.
