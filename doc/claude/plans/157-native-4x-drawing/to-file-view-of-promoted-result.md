# To file: returning a callee's VIEW of the promoted result local answers `[]`

Found 2026-09-25 while widening `(R-WorkBuffer)` to by-value hand-offs; present with
`LOFT_NO_WORK_BUFFER=1` and on the 2026.8.0 release (`~/.local/bin/loft`), both backends.
Not filed because the `gh` API token is invalid on this box.  Labels: `sev:high`,
`silent-wrong`, `area:codegen`, `hit-by:loft`, `wa:none`.

```loft
fn view_of(xs: vector<integer>) -> vector<integer> { xs }
fn c7b(n: integer) -> vector<integer> {
  v: vector<integer> = [];
  for i in 0..n { v += [i]; }
  view_of(v)
}
fn main() {
  a = c7b(3);
  println("a={a}");          // a=[]   — expected a=[0,1,2]
  b = view_of([1, 2]);
  println("b={b}");          // b=[1,2] — right
}
```

`view_of` answers a view of its parameter (`-> vector<integer>["xs"]`).  In `c7b` the tail
`view_of(v)` makes the parser promote `v` onto the return buffer (`fn n_c7b(n:integer,
v:vector<integer>) -> vector<integer>["??"]` — `v` becomes the hidden argument), the call is
then handed the caller's buffer as `xs`, and what comes back reads as empty: the delivery of a
callee's view of the promoted result local is lost somewhere between the tail and the caller's
read.  The direct call `view_of([1, 2])` is right, so it is the promotion-plus-forward shape.
Compare `(R-ValueRecord)`'s forward clause (`LOFT_NO_FORWARD_TUPLE`) and `returns_one_of_
several_args`, which name this shape for records.
