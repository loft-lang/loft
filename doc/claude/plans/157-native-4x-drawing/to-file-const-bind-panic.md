# To file: A plain local bound from a top-level vector constant and then written panics: "Write to read-only store" (both backends)

<!-- Found 2026-09-25 while building (R-Const).  This box's GitHub token was invalid for
     writes (`gh auth login` needed), so the issue text waits here.  File it with
     `gh issue create --title … --label sev:high --label area:parser --label wa:clean
     --label hit-by:loft --body-file <this file's body>`, then replace the references to
     this file in formal/rewrites.md and bench/portal/analysis/libraries-wide.md with the
     number, and delete this file. -->

### Minimal reproducer

```loft
CODES = [32, 33, 35];

fn param_writes(v: vector<integer>) -> integer { v[0] = 9; v[0] ?? -1 }

fn main() {
  c = CODES;
  c[0] = 5;            // panics here — also `c += [7]`
  println("{c[0] ?? -1} {CODES[0] ?? -1}");
  println("{param_writes(CODES)}");   // and here, on its own
}
```

### Expected behaviour

`5 32` then `9`: a plain bind copies (`(B-Copy)`), so `c` is the program's own vector and the constant is untouched; a constant handed to a callee that writes its by-value parameter is copied for that call.  `formal/rewrites.md` (R-Const) states it for the view a constant's use site answers: *"a plain bind of that view is a view too where c is only READ … and is B-Copy's copy the moment c is written … or handed to a parameter the callee writes."*

### Actual behaviour

Both backends panic at the first write:

```
thread 'main' panicked at src/store.rs:3428:9:
Write to read-only store at rec=5 fld=8 (locked by: Store::lock)
```

The constant's use site (`parser/objects.rs`, the `DefType::Constant` arm) emits `OpConstRef` and the bind `c = OpConstRef(k)` is a VIEW of the write-locked constant store with no copy road: the comment there says the bind "deep-copies at the call site — gen_set_first_ref_call_copy handles the CopyRecord", but the emitted bytecode is `ConstRef; PutRef` and nothing copies.  `(H-WriteLocked)` is the backstop that fires.  A store INTO a field (`h.v = CODES`, `H { v: CODES }`) and a `return CODES` do copy and behave.

### loft version

91ad2af68 (branch 157-native-4x), 2026-09-25

### Execution mode

Both (`--interpret` and `--native`)

### Platform

Linux x86-64

### Workaround

Spell the table as a zero-argument function: `fn codes() -> vector<integer> { [32, 33, 35] }` then `c = codes(); c[0] = 5` — the call builds a fresh vector, and since `(R-Const)` (2026-09-25) a call whose result is only READ costs no build either.  Verified on both backends (cells c5–c7 of `tests/scripts/a-literal-bodied-function-is-a-constant.loft`).

### Is there a clean workaround?

Yes — the diagnostics for unbuildable constants already prescribe the function form.

### Filed as a direct result of another ticket?

Found-via: the `(R-Const)` build (@PLN158), probing what a bind from a constant does today.

### Who hit it?

hit-by:loft
