# Lifetime — closures

How closures (structs with captures) follow the lifetime rules: capture layout, the fn-ref slot, and current closure freeing.  Part of [LIFETIME.md](LIFETIME.md), which holds scope-exit freeing, view returns and text-from-struct deps.

## Closures are structs — the same lifetime rules apply

A closure record is a store-allocated struct.  When a lambda captures variables, the
compiler synthesizes an anonymous struct `__closure_N` with fields matching each
captured variable.  At runtime, the closure record is a `DbRef` pointing to a store
allocation — identical to any other struct.

This means text read from a closure record should follow the exact same dep chain
rules as text read from a regular struct: the text must depend on the closure record
variable, and that dep must keep the closure store allocation alive.

### Closure record allocation — `src/parser/vectors.rs:628-712`

- Anonymous struct `__closure_N` with fields matching each captured variable
- Allocated at lambda **definition time** via `OpDatabase`
- Each captured value is **copied** into the record's fields (set_field)
- The record's `DbRef` is embedded in the 16-byte fn-ref slot
- Work variable `__clos_N` has type `Type::Reference(closure_d_nr, [])` — owned

### Adopted vs borrowed captures — who frees the store behind a capture (#682)

A reference / collection capture is stored as a 12-byte `DbRef`, so exactly one owner
must reclaim it.  `free_named`'s cascade (`src/database/allocation.rs`) frees the
record's `DbRef` fields when the record dies, and `get_free_vars`' `captured_ref`
suppresses the defining frame's own `OpFreeRef` for a captured reference.  Those two
are a PAIR: the suppression is what hands the store over, and it is what makes an
escaping factory closure sound (#323).

The pairing only holds where a frame free existed to suppress.  A captured
**parameter** never enters the scope-exit sweep at all (`variables()`: "never return
function arguments"), and a **projection local** (`ch = w.chunks[1]`, a `for` element)
is `owns == false` — so for both, the cascade was a second free of a live store.  It
destroyed the caller's value, and because a freed-but-unreused store still reads
correctly, the crash surfaced thousands of ops later in whatever function next
touched it.

The two cases are now distinct in the schema.  Both are 12 bytes at align 4, so
nothing about layout, reads or writes changes — only the free decision:

| Marker (`Deps`) | Field storage type | Cascade frees it? |
|---|---|---|
| `share_sentinel()` | `dbref` | yes — the record ADOPTED the store |
| `borrowed_share_sentinel()` | `dbref_borrow` | no — someone else owns it |

**The verdict is not knowable at parse time**, which is why it is not decided in
`synthesize_closure_record`.  `ch = pick(w, 1)` parses as "borrows `w`" from the
callee's declared return, and only `scopes::check`'s call-result rewrite
(`make_independent`, the `!adopts_fresh_store` arm) turns it into OWNED once it knows
the return ABI deep-copies into a fresh store.  Reading the parse-time dep leaked
that copy.  So the decision is `scopes::mark_borrowed_captures`, run after every dep
rewrite has settled; `--native` reads the marked attribute type directly, while the
interpreter's already-registered schema is synced by
`typedef::sync_capture_ownership` from `compile::byte_code_from`.

A `__cell_<T>` capture (plan-22's boxed mutated scalar / text) is ALWAYS adopted: the
cell is minted for that closure alone, so the record is its only possible owner
however the original binding was reached.

Both `dbref` shapes are registered **together**, from either entry point: type
numbers are positional and `--native` replays the registration sequence to rebuild
the schema, so a shape present in only some programs would shift every id after it.

### A mutated scalar PARAMETER is boxed through a shadow local (#685)

`flip_scalars_to_box_types` cannot flip an argument in place: its slot receives the
caller's scalar, and a 12-byte cell `DbRef` type there would change the call ABI. It
used to skip arguments outright — but `box_captured_names_for_outer_scalars` did not,
so the closure record got a 12-byte cell field fed from an 8-byte stack slot, and
`emit_lambda_code`'s `OpSetDbRef` read 12 bytes out of 8 and corrupted the fn-ref
being built beside it. **The two halves of "is this capture boxed?" must be decided by
one fact.**

They are, by making the argument case *be* the local case:
`promote_boxed_scalar_arg` mints `__bx_<name>` with the argument's own type,
`set_promoted_from` + `remap_name` point the name at it before the body parses, and
`parse_code`'s promoted-argument preamble seeds it at function entry. The emitted IR is
then byte-identical to the working local shape — which is why every boxable type is
covered with no per-type work, and why the caller's value stays untouched (a scalar
parameter is by value; the argument slot is never written).

Two constraints worth knowing before touching this:

- The shadow is marked **`defined` at creation**, so the body's first write does not
  ALSO prepend an allocation (`maybe_prepend_cell_alloc`). A second `OpDatabase` would
  replace the seeded cell and silently lose the argument's value.
- `RefVar` arguments are excluded. A user `&T` out-parameter's writes must reach the
  caller (a private cell would swallow them), and a mutable text local the compiler
  already promoted to a hidden `&text` out-parameter is itself the working path — a
  first attempt without this exclusion refused code that worked.

The seed shares `boxed_cell_alloc_and_set` with the first-assignment path — one home
for "a boxed scalar comes into existence", because the seed is the only assignment a
parameter's cell is guaranteed to have (the mutation may live entirely inside the
closure).

### Which mutated captures take a CELL, and which stay inline (#687)

A mutated capture is normally boxed into a shared `__cell_<T>` so the closure's writes and
the enclosing body's reads hit one location.  The exception is a binding that **already
carries its own indirection**: a text local that is the function's RETURN SOURCE, which the
return machinery promotes to a hidden `&text` out-parameter so the caller supplies the
buffer.  That binding cannot also be a cell — and does not need to be, because the record
stores the value inline and the existing per-call write-back propagates the closure's
changes.  Two indirections for one binding is a crash: the record holds a cell DbRef while
the binding is a `&text` stack pointer.

**`RefVar` is the fact.**  Plan-22 02d-vii used to stand in for it with "skip text boxing
when the parent returns text", which was wrong in both directions — too wide (it also
skipped a text local the function does NOT return, which boxes cleanly) and no help for a
PARAMETER, which has no indirection to reuse and so was refused outright.

**Where the decision lives.**  Two halves have to agree — the record's attribute type
(`box_captured_names_for_outer_scalars`, at the LAMBDA's epilogue) and the binding's own
type (`flip_scalars_to_box_types`, pass 2) — and the epilogue is too early to be right: a
to-be-returned text local is still a plain `Text` there and only becomes `RefVar(Text)`
later in the body.  So the epilogue boxes PROVISIONALLY (the common case, and the attribute
must say something because pass 1 freezes the record's storage), and
`Parser::finalize_capture_storage` corrects it at the parent's **pass-1 body end** — the
first moment the fact is final, and still before `fill_all` lays the record out.

One text-returning function can need both answers at once (`keep` returned, `side` not),
which is why no per-function condition can work.

### A capture of a FORWARD-declared type is typed and laid out in pass 2 (#686)

A capture's type is the type of an EXPRESSION (`ch = w.chunks[1]`), so it cannot be
deferred by name the way a written-down `f: World` field can.  With `World` declared later
in the file, pass 1 freezes the record's attribute as `Unknown(0)` — the "no type known"
sentinel.  Two things then went wrong, and the first hid the second:

1. `copy_unknown_fields` read the `0` as a stub def number and set the field to
   `data.def(0).returned` — in practice `text`.  A LYING fact: the field looked resolved,
   so the closure body type-checked against a type the program never mentions
   (`Unknown field text.cells`).  Guarded now (`was != 0`, matching the `Vector` arm).
2. With no invented type, the field was unsized — and `fill_database`'s field loop SKIPS an
   attribute it cannot size while still registering the struct, so `finish` sized the
   record with `position == u16::MAX` and `finish_type` never revisits a sized type.  The
   closure read and wrote its capture at **offset 65535**, intermittently fatal.

**The invariant: a struct is laid out only once its fields are sized.**  `fill_all` skips a
def whose fields are not all known yet (`layout_blocked`); the loop is keyed on
`known_type == u16::MAX`, so this defers rather than drops.  `parse_lambda`'s
`resolve_forward_captures` then re-types the attribute from `capture_context` at LAMBDA
ENTRY — not at the synthesis epilogue, which runs after the body — and lays that one record
out via `Stores::lay_out_record`.

Why the on-demand layout is required: the body bakes field offsets into its IR as it
parses, so the end-of-pass `finish()` is too late, and a full `finish()` mid-parse
re-appends keyed-index bookkeeping.  This is the same deferral `lay_out_synth` already makes
for a forward-referenced synth enum.

⚠ **The second fault was INTERMITTENT** — a positionless field only crashes when the bytes
at offset 65535 happen to be fatal.  Two byte-identical probe files disagreed during the
investigation, and single runs "confirmed" three different stories before a repeat-run
harness (N≥12) replaced them.  Any probe in this area needs one.

### A field whose type another MODULE declares (#797)

The same invariant, reached by a different route, and with a wider blast radius: the
struct here is one a package author wrote down, not a synthesised closure record.

A package entry that `use`s a module before declaring the types that module names
suspends itself at the `use`.  The module is then parsed to completion — layout
included — while every such type is still a `DefType::Unknown` stub.  `fill_database`
skips a field whose `type_elm` is `u32::MAX`, so the field gets no slot, and the stub is
upgraded IN PLACE moments later when the entry resumes and declares the type.  The
DECLARATION therefore ends up correct and the LAYOUT keeps the hole — the two disagree
for the rest of the run, and `position()` answered `u16::MAX` for a field the program
could name.  Same offset-65535 write as #686, but into the neighbouring records: the
symptom depended on what happened to sit next to the record, so one run gave a SIGSEGV
inside an unrelated walk, another an allocation that reached 59.6 GiB, and a third a
clean pass.

Three sites had to agree for the deferral to hold:

- **`layout_blocked`** — the guard, now covering `Unknown(stub)` as well as `Unknown(0)`,
  and TRANSITIVE, because an inline field stores its content's bytes so a host whose
  field type is waiting cannot be laid out either.
- **The sweep at the top of `fill_all`** — re-runs `copy_unknown_fields` over everything
  still unlaid, so each `fill_all` picks up whatever the files parsed since have
  declared.  Without it the deferral never ends: `actual_types_deferred` only sweeps the
  file it is finishing, and nothing was asking again.
- **`copy_unknown_fields` peels `Optional`** — `S?` and `S` name the same forward
  reference.  `Type::Optional` is the wrapper this area keeps forgetting: `rewrite_type_opt`
  and the native `init()` generator's field-hoist match were missing it too, so a nullable
  forward field failed where a plain one worked — the generator emitted
  `db.field(t_host, "f", t_content)` ahead of `let t_content`, i.e. the library did not
  compile.  Adding an arm per site is how that gets missed a fourth time; each of the
  three peels the marker and asks about the base.

Guard: `forward_module_type_gets_a_slot` (`tests/issues.rs`), driving
`tests/multilib/fwd797_layout.loft` over the `fwd797` fixture.  It asserts SIZES as well
as values — a read follows whatever offsets the layout ended up with, so reading a field
back cannot by itself prove the field has storage.

The resolution half — a type named in a function BODY rather than in a field DECLARATION —
was loft#801 and is closed too: the body sites now leave the same forward-reference stub,
and `parse_file` drains its suspended parents on a plain Error instead of abandoning them.
See [COMPILER.md § How a forward reference actually resolves](COMPILER.md).

One member of the family is still open, and it is an ENUM:
[loft#803](https://github.com/loft-lang/loft/issues/803).  A struct field whose enum is
declared later is laid out against an unregistered enum, so the field takes zero width and
the field AFTER it loses its position — this same corruption, one field along.  Do not
patch it from here: the issue records three fixes that each looked right and each broke
something measurable (a silently wrong variant, a layout that never lifts, renumbered
native type ids).  The order in which an enum gets its runtime type and its variant
discriminants is the actual question.

### Inside the lambda: `__closure` is a struct parameter

When parsing the lambda body, the compiler adds a hidden `__closure` parameter:

```rust
// src/parser/vectors.rs:391-399
let closure_tp = Type::Reference(closure_rec, vec![]);
self.data.add_attribute(&mut self.lexer, d_nr, "__closure", closure_tp.clone());
let v_nr = self.create_var("__closure", &closure_tp);
self.vars.become_argument(v_nr);
self.closure_param = v_nr;
```

`__closure` is `Type::Reference(closure_rec, [])` — an owned Reference that is a
function argument (scope 0, never freed by `get_free_vars`).

### Reading captured variables = reading struct fields

When the lambda body references a captured variable like `prefix`, the parser redirects
it to a field read from the closure record.  There are two code paths:

**Path 1** — known closure variable (`src/parser/objects.rs:91-98`):
```rust
let closure_d_nr = self.data.def(self.context).closure_record;
let fnr = self.data.attr(closure_d_nr, name);
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
t = t.depending(self.closure_param);   // A5.6-text: add __closure as dep
```

**Path 2** — capture_context variable (`src/parser/objects.rs:172-175`):
```rust
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
t = t.depending(self.closure_param);   // A5.6-text: add __closure as dep
```

### The analogy to regular struct field access

Compare the closure field read above with normal field access
(`src/parser/fields.rs:130-137`):

```rust
let dep = t.depend();
t = self.data.attr_type(dnr, fnr);       // same — get field's declared type
for on in dep { t = t.depending(on); }  // inherit parent's deps
if let Value::Var(nr) = code {
    t = t.depending(*nr);                // add the struct variable as dep
}
```

Normal field access adds the struct variable (and its deps) to the result type.
For a text field on struct `p`, this produces `Type::Text([v_p])` — the text depends
on `p`, which prevents `p` from being freed while the text escapes.

Both closure field-read paths add `self.closure_param` as a dependency after reading
the field type, matching the pattern for normal struct field access.  This produces
`Type::Text([v___closure])` for a text field read from a closure, matching the
`Type::Text([v_p])` produced by `p.name` for a regular struct.  The dep keeps the
closure record alive while derived text is in use.

---

## The 16-byte fn-ref slot layout

A `Type::Function` variable occupies 16 bytes on the stack:

```
bytes  0.. 4: d_nr       (i32, function definition number)
bytes  4..16: closure     (DbRef, 12 bytes; null sentinel if no closure)
```

### Key codegen paths

| Component | Location | Role |
|-----------|----------|------|
| `gen_set_first_at_tos` | `codegen.rs:843-847` | Delegates to `gen_fn_ref_value` for Function vars |
| `gen_fn_ref_value` | `codegen.rs:466-490` | Ensures every if-else branch produces 16B |
| `OpVarFnRef` | `02_files.loft:350` | Push 16B fn-ref from frame variable |
| `OpPutFnRef` | `02_files.loft:354` | Pop 16B fn-ref into frame variable |
| `OpNullRefSentinel` | `01_code.loft:733` | Pads non-capturing lambdas (4B d_nr → 16B) |
| `fn_call_ref` | `state/mod.rs:221-249` | Reads d_nr at offset 0, closure at offset+4 |

### fn-ref type carries closure dep — `vectors.rs:666-669`

```rust
// A5.6-text: fn-ref depends on closure work var `w` so that
// get_free_vars does not emit OpFreeRef for the closure record
// before the fn-ref escapes the defining scope.
let fn_type = Type::Function(visible_params, Box::new(ret_tp), vec![w]);
```

### Return type dep propagation — `vectors.rs:701-711`

When the enclosing function returns a fn-ref, the closure dep `w` is propagated to
the declared return type so `get_free_vars` at the Return statement sees
`tp.depend()` containing `w`:

```rust
if matches!(self.data.def(self.context).returned, Type::Function(_, _, _)) {
    self.data.definitions[self.context as usize].returned =
        self.data.definitions[self.context as usize].returned.depending(w);
}
```

---

## Current status of closure freeing

### Same-scope closures: WORKING

All same-scope closure tests pass.  `___clos_N` and the fn-ref live in the same
function scope; `get_free_vars` doesn't run between closure allocation and fn-ref use.

**Passing tests** (tests/expressions.rs):
- `closure_capture_integer` (line 317)
- `closure_capture_after_change` (line 323)
- `closure_capture_multiple` (line 335)
- `closure_capture_text_integer_return` (line 355)
- `closure_capture_text_return` (line 364)
- `closure_capture_struct_ref` (line 375)
- `closure_capture_vector_elem` (line 390)
- `closure_capture_text_loop` (line 406)

### Cross-scope closures: WORKING

**`closure_capture_text`** (tests/expressions.rs:343) now passes.

Four bugs were fixed:

1. **Free suppression** — `get_free_vars` used only the block result type (`tp`) for
   the dep check, but the block result type doesn't carry the closure dep that was
   propagated to the function's declared return type.  Fix: also check
   `data.def(self.d_nr).returned.depend()`.

2. **Work-buffer propagation** — the declared `fn(text) -> text` return type didn't
   encode the lambda's work-buffer deps.  `try_fn_ref_call` created zero work buffers,
   so the lambda's `__work_1` parameter received garbage.  Fix: `emit_lambda_code`
   replaces the inner return type with the lambda's actual return type.

3. **fn-ref null pre-init** — `gen_set_first_at_tos` emitted only `NullRefSentinel`
   (12 bytes) for Function variables, but fn-ref slots are 16 bytes.  `PutFnRef`
   overwrote 4 bytes of the next variable.  Fix: emit `ConstInt(i32::MIN)` +
   `NullRefSentinel` for a full 16-byte null slot.

4. **Caller-side closure free** — `get_free_vars` had no `Type::Function` branch.
   The closure DbRef at offset+4 leaked when fn-ref variables went out of scope.
   Fix: add a `Type::Function` arm with a codegen special case that reads the
   closure via `OpVarRef(var_pos - 4)` before `OpFreeRef`.  Same-scope fn-refs
   carry `dep=[w]` (the closure work var), so the free is suppressed — `___clos_N`
   already handles it.

### Caller-side closure free: native codegen path

The interpreter frees closure records via the codegen special case described
above.  Native codegen handles closure-record drop via Rust's RAII at
stack-frame exit (no explicit `OpFreeRef` emission needed); @PLAN15 phase
03–05 leak guards (`tests/leak.rs::p15_phase03_*` / `_phase04_*` /
`_phase05_*`) confirmed both paths produce clean store state under
100-iteration tight loops for text / Reference / nested-closure captures
across D1 + D3.  Cross-mode equivalence (`tests/closure_matrix.rs`)
catches any future native-vs-interp divergence in observable output.

---

The closed history of closure freeing and inline-lift safety is in [LIFETIME-history.md](LIFETIME-history.md).
