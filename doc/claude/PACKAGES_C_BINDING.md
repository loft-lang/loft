
# Direct C binding — `#c`

The `#c` annotation and everything around it (numeric libraries, retention, `libs`, `optional-libs`, wasm).  Split out of [PACKAGES.md § Function binding model](PACKAGES.md#function-binding-model).

---

### Direct C binding — `#c` (@PLN24)

`#native` above is the **Rust** binding: a hand-written `extern "C"` fn compiled
by rustc into a cdylib.  A planned sibling annotation, **`#c "<symbol>"`**, binds
a loft function **straight to a C-library symbol — no Rust wrapper, no rustc, no
libffi**.  loft-core `dlopen`s the system library and calls the symbol through a
small **fixed per-arity C-ABI caller** (`extern "C" fn(u64, …) -> u64`, one per
arity — integer-class args collapse to a `u64` slot); the library is **pure loft**
(`#c` decls) plus, for signatures the caller can't express (float / struct-by-
value / varargs arguments), a `cc`-compiled **ANSI-C shim**.  Native-only — a
wasm module cannot `dlopen` a shared library, so both wasm targets refuse a
reachable `#c` call by name (see [below](#the-wasm-and-browser-targets--arc-e)).
It is the foundation for binding system C libraries
(databases, codecs, …) without the rustc toolchain, keeping loft-core minimal —
the linking tool lives in core, all complexity in the library + shim.

**Two things the architecture probe settled**, before anyone writes a binding.
The per-arity caller works for **arguments** — int, long, pointer, `char *`,
bool all cross correctly at every arity, including across the register/stack
boundary — but **not for returns**: a 32-bit C return read back as 64 bits turns
−1 into 4294967295, quietly.  So the declaration carries the C signature
(`#c "PQstatus" "int(void*)"`), and it is the **sole** authority: pointed at a
wrong arity or a variadic function, the caller returned the *right answer* by
luck, so there is no runtime signal to catch a mismatch — the check is at
compile time or nowhere.  Second: `#c` is the declared edge of loft's
no-runtime-errors rule.  Arguments cost nothing (non-null is already the default
and null-flow rejects a `τ?` at compile time), a NULL pointer return maps to
loft null, and a fault *inside* C is undefined — the same failure mode `#native`
already has, through the same crash handler.

#### The declaration (arc A — implemented, inert)

```loft
pub fn status(conn: integer) -> integer;      #c "PQstatus" "int(void*)"
pub fn error(conn: integer) -> text?;         #c "PQerrorMessage" "const char*(void*)"
pub fn sum(v: vector<integer>) -> integer;    #c "lc_sum" "long(const long*, long)"
```

Both strings are required.  The C signature is `<return>(<params>)` in ordinary
C spelling, and **widths resolve against the target**, exactly as a C compiler
reads the same header: `long` is 64 bits on Linux and macOS and 32 on Windows,
plain `char` follows the platform's signedness.  So one declaration stays
correct everywhere.  An unknown type is refused, never guessed.

**What a loft type looks like from C** — the mapping the arity check counts in:

| loft | C | notes |
|---|---|---|
| `integer`, narrow ints, `boolean`, `character` | one C integer | any width; the value is passed full-width |
| `text` argument | one `const char*` | **NUL-terminated**, unlike the `#native` path's `ptr, len` |
| `text` / `text?` RETURN | `char*` | the bytes are **copied** up to the first NUL; loft never frees the pointer |
| `vector<T>` | **two**: element pointer + count | C carries no length.  The pointer is valid *for the call only* — see the retention hazard below |
| `vector<float>` | `double*` + count | C may **write through it**; the writes are visible to loft |
| C-owned handle (`PGconn *`) | `void*` ↔ loft `integer` | the pointer value crosses as an integer |
| `float` / `single` SCALAR | — | **refused**: floats travel in SSE registers a fixed caller does not touch — pass it by pointer as a 1-element `vector<float>` |
| a loft record | — | **refused**: records live in a store that may move them |
| a nullable `τ?` ARGUMENT | — | **refused at compile time**: C has no null model |

The last two refusals are the design, not gaps.  A record's address is a
position in an arena the allocator can relocate, so handing it to C is the
store-lifetime bug class rather than a marshalling detail.  And `#c` is the
declared edge of loft's no-runtime-errors rule: a null crossing *into* C would be
an ordinary number or a fault, so it is rejected where loft still can — which
costs nothing, because non-null is already the default and null-flow already
requires a discharge (`?? 0`, `x?`, `match`).

**Two shape limits worth knowing before you write a binding.**  A `vector<T>`
becomes an element pointer **immediately followed by** its count, so `write(fd,
ptr, n)` binds directly while `memchr(ptr, ch, n)` and `fwrite(ptr, size, n, f)`
— which separate the pair — need a shim.  And a binding may declare at most **32**
C parameters: the interpreter calls through a fixed ladder of per-arity
trampolines, and that ladder is where the ceiling comes from.

**One ceiling, both backends** (@PLN128 arc C).  It used to be 12 and it was
enforced on the interpreter ONLY, which made `#c` two different languages: a
13-slot binding compiled under `--native`, shipped, and failed for whoever
interpreted it — including `loft debug`, which *is* the interpreter, so the
bindings you could not debug were exactly the ones with no other way in.  The
author never saw it either, because the refusal fired at the call site rather
than the declaration.

Unifying downward would have narrowed what already compiles, so the ladder was
extended to 32 instead and `--native` held to the same number.  32 was sized
against `dgemm_` when a by-reference scalar was still thought to cost two slots
(pointer + count); since a `vector` carries a count only where the C signature
has one, `dgemm_` costs **13** and LAPACK's 20-argument drivers fit with room to
spare.  Past 32 a shim is the answer — a ladder cannot be unbounded.

The check now runs on both backends, in two places: at the **declaration**, for
code you own (your program, your library, the stdlib), so you see it as you write
it; and at any **call site**, including one reaching into a dependency, because
that call genuinely cannot work.  Merely *loading* a dependency that declares an
over-ceiling binding you never call is fine — a consumer cannot edit someone
else's declaration, so it must not fail their build.

#### Numeric libraries — what the boundary already does (@PLN128)

BLAS/LAPACK, FFTW, HDF5 and GSL are the same three shapes over and over: an array
in, an array written back, and a scalar passed by reference.  Every cell below was
measured on **both backends** against a C oracle that computed the expected value
itself:

| shape | `--interpret` | `--native` |
|---|---|---|
| read a `double` array — `vector<float>` → `(const double*, int64_t)` | ✅ | ✅ |
| **C writes back through `double*`** (the `daxpy` shape) | ✅ | ✅ |
| 1-element `vector<integer>` as a Fortran scalar `const int64_t*` | ✅ | ✅ |
| a scalar `double` in and out through a pointer shim | ✅ | ✅ |
| **a full Fortran argument list — `dgemm_`'s thirteen bare pointers** | ✅ | ✅ |
| counted and bare vectors **mixed in one signature** | ✅ | ✅ |
| 14 C argument slots | ✅ | ✅ |
| **a `double` RETURNED by value** (`ddot_`, `dnrm2_`) | ✅ | ✅ |
| 33 C argument slots (past `MAX_C_ARITY`) | ❌ refused | ❌ refused |
| a `double` **argument** by value | ❌ refused | ❌ refused |
| a `vector` whose element width differs from the C pointee | ❌ refused | ❌ refused |

Every row above was re-measured against **real OpenBLAS** — `daxpy_`, `dgemm_`,
`dgesv_`, `ddot_`, `dnrm2_` as the library exports them — not only against a
fixture written for loft.

**The write-back cell is the one that matters.**  Every BLAS and LAPACK routine
returns its result by writing through a caller-supplied pointer, so a boundary
that could not carry those writes could not bind the numeric stack at all.  It
carries them.

**A scalar float crosses as a 1-element `vector<float>`** — this is the answer to
the most common question, and the cure the float refusal now names:

```loft
// C:  void shim_scale(double *out, int64_t n_out, const double *v, int64_t n_v)
pub fn shim_scale(out: vector<float>, v: vector<float>);
#c "shim_scale" "void(double*, int64_t, const double*, int64_t)"

v: vector<float> = [2.5];        // a COMPUTED double, not a hand-converted literal
out: vector<float> = [0.0];
shim_scale(out, v);              // out[0] == 5.0
```

There is deliberately no float→bits builtin, so a shim taking "the bit pattern as
an integer" is **not** a route a real program can take: `x as integer` is a value
cast (`2.5` → `2`).  Pointers are the whole answer.

##### A `vector` carries a count only where the C signature has one

C carries no length, so a `vector` normally crosses as **pointer then count** —
and a C library written for loft takes exactly that. Fortran does not. BLAS and
LAPACK pass **every** argument by reference, so each one is a bare pointer and
the routine learns the length from a separate `n`, itself a bare pointer.

**Both are supported, and the C signature is what chooses.** Write the signature
the header shows you and the count appears exactly where the header puts one:

```loft
// A C library written for loft: the count is in the signature, so loft sends it.
pub fn lc_i64_sum(v: vector<integer>) -> integer;
#c "lc_i64_sum" "int64_t(const int64_t*, int64_t)"

// Fortran: thirteen by-reference arguments, thirteen bare pointers, no counts.
pub fn dgemm(transa: text, transb: text, m: vector<integer>, n: vector<integer>,
             k: vector<integer>, alpha: vector<float>, a: vector<float>,
             lda: vector<integer>, b: vector<float>, ldb: vector<integer>,
             beta: vector<float>, c: vector<float>, ldc: vector<integer>);
#c "dgemm_" "void(const char*, const char*, const int64_t*, const int64_t*, const int64_t*, const double*, const double*, const int64_t*, const double*, const int64_t*, const double*, double*, const int64_t*)"
```

So **a Fortran routine costs one slot per argument**: `dgemm_` needs 13, not 26,
and even LAPACK's largest drivers (20+ arguments) fit under the 32-slot ceiling
without an argument-collapsing shim. Pointer-and-integer APIs (HDF5, GSL) spend
one slot per scalar and are nowhere near it either — though see the retention
hazard below before reaching for one that keeps your buffers.

A **scalar** by reference is a 1-element vector, because loft has no address-of:
`m: vector<integer> = [2]` is how you pass Fortran's `const int64_t *m`. A
`character*1` argument is `text` — `"N"` crosses as a NUL-terminated `const
char *`, which is what a C caller passes.

Two things to know before binding real Fortran:

- **Reading of the signature is unambiguous, but only the signature is checked.**
  Omit a count the C function really takes and the declaration is now *accepted*,
  and passes a bare pointer where the callee wants a length. Nothing at runtime
  can catch that — the same standing rule as any other wrong `#c` signature.
- **Hidden string lengths.** A Fortran compiler appends a hidden length argument
  per `character` argument. Reference BLAS and LAPACK do not read them for the
  length-1 flags, which is why C callers pass `"N"` and nothing else, but a
  routine that takes a real Fortran string needs a shim that supplies them.

##### The element type must be the one the C header spells

A `vector` reaches C as a pointer into **loft's own element bytes** — nothing is
converted on the way — so the loft element type and the C pointee are two
spellings of one layout, and a declaration where they disagree is refused:

| loft element | write the pointee as |
|---|---|
| `integer` | a 64-bit integer (`int64_t`, `long` on LP64) |
| `i32` / `u32` | a 32-bit integer (`int`) |
| `u16` | a 16-bit integer (`short`) |
| `u8` / `boolean` | an 8-bit integer (`char`) |
| `character` | a 32-bit integer — the codepoint |
| `float` | `double` |
| `single` | `float` |
| `i8` / `i16` | **refused** — see below |
| `vector<text>` | **refused** — the elements are loft's handles, not `char *` |
| anything above | `void *` — always accepted, and means "these are bytes" |

Signedness is not checked: it changes how a byte is read, never where the next
one starts, so `vector<u8>` against `const char *` is the ordinary byte-buffer
idiom.  `i8` and `i16` are refused because loft stores narrow **signed** elements
as `val - min`, so a loft `0` is the byte `128` — use `i32`, the unsigned
sibling, or `void *` if you really mean raw bytes.

**The width the C header uses is the one to write, and it is not always obvious.**
BLAS and LAPACK ship in two builds: LP64 (Fortran `INTEGER` is 32-bit, which is
what `libopenblas.so.0` on a typical Linux is) and ILP64 (64-bit).  The same
symbol names serve both, so *nothing but the header tells you which is
installed* — and the wrong choice is quiet:

```loft
// Against an LP64 build this reads the right answer for `n`, because the low
// four bytes of an 8-byte 3 are 3 — and then `info` and `ipiv`, which C WRITES,
// come back as -4294967296 and 8589934593.
pub fn dgesv(n: vector<integer>, …);   #c "dgesv_" "void(const int64_t*, …)"
```

Declare the widths your build uses and the check above holds you to them
consistently; it cannot tell you which build is on the machine.

##### A `double` return comes back; a `double` argument still does not

The level-1 BLAS *functions* answer by value — `ddot_`, `dnrm2_`, `dasum_`, and
LAPACK's `dlange_` / `dlamch_` — and that binds directly:

```loft
pub fn ddot(n: vector<i32>, x: vector<float>, incx: vector<i32>,
            y: vector<float>, incy: vector<i32>) -> float;
#c "ddot_" "double(const int*, const double*, const int*, const double*, const int*)"
```

A C `double` returns as loft `float` and a C `float` as loft `single`, and the
pairing is exact — a C `float` leaves a *single* in the return register, so
binding it to `float` would read those bits as a denormal.

A float **argument** is still refused, and that asymmetry is deliberate rather
than unfinished: the register file is chosen per argument position, so supporting
one anywhere would need a trampoline per subset of positions, while the return is
a single axis.  It costs nothing in practice — Fortran passes every argument by
reference, so a numeric binding has no by-value floats to pass.

##### The retention hazard — a pointer C keeps is a use-after-free

`vector<T>` hands C a pointer **valid for the duration of the call**.  loft cannot
see that C stored it, so the vector's lifetime still ends at its last *loft-visible*
use — and a later C read then hits memory loft has reused, silently:

```loft
a: vector<float> = [1.5, 2.25, 4.0];
retain(a);                       // C keeps the pointer; `a` has no later use
b: vector<float> = [100.0, 200.0, 400.0];
reread();                        // reads `b`, not `a` — measured, both backends
```

That reads another variable's data with no fault and no diagnostic.  Adding any
later use of `a` keeps it alive and the read is correct — which is precisely what
makes the bug dangerous: it appears and disappears with edits that look unrelated.

This is the FFTW plan API's exact shape (`fftw_plan_dft` retains the buffers,
`fftw_execute` reads them later), and it is not an FFTW quirk: zlib's `z_stream`
keeps `next_in` / `next_out`, `sqlite3_bind_text(…, SQLITE_STATIC)` keeps your
bytes, and every "context object" API is this.

##### Binding a retaining API: give C the buffer

**The cure is that C owns the memory.**  Allocate on the C side, hold the pointer
as an opaque `integer`, and copy in and out — then nothing loft owns is retained,
the buffer's lifetime is the handle's, and each copy lives only for its own call,
which is what `#c` already guarantees.  It needs no shim, because `memcpy` is
libc and needs no `[c] libs` entry:

```loft
pub fn fftw_malloc(nbytes: integer) -> integer;   #c "fftw_malloc" "void*(size_t)"
pub fn fftw_free(p: integer);                     #c "fftw_free" "void(void*)"
pub fn plan_dft_1d(n: integer, inp: integer, outp: integer, sign: integer, flags: integer) -> integer;
#c "fftw_plan_dft_1d" "void*(int, double*, double*, int, unsigned)"
pub fn execute(p: integer);                       #c "fftw_execute" "void(const void*)"

// The two crossings.  Same libc symbol; only the DIRECTION differs, which is
// what puts the loft vector on a different side of the boundary each time.
pub fn load(dst: integer, src: vector<float>, nbytes: integer) -> integer;
#c "memcpy" "void*(void*, const void*, size_t)"
pub fn store(dst: vector<float>, src: integer, nbytes: integer) -> integer;
#c "memcpy" "void*(void*, const void*, size_t)"
```

```loft
inp = fftw_malloc(n * 16);  outp = fftw_malloc(n * 16);
p = plan_dft_1d(n, inp, outp, -1, 64);   // FFTW_FORWARD, FFTW_ESTIMATE
load(inp, src, n * 16);
execute(p);
store(dst, outp, n * 16);
```

This is not a workaround for a gap — for FFTW it is what the library's own
documentation tells C callers to do, because `fftw_malloc` is what supplies the
SIMD alignment a caller array would forfeit.  Measured on both backends against
a C program computing the same transform.

**What it costs.** Two O(n) copies per call, against an FFT's O(n log n) of work.
And `nbytes` is **your** arithmetic: a `void *` pointee is the opaque escape
hatch, so the element-width check above does not apply and a `store` sized larger
than the destination writes past the end of the vector.  Size the copy off the
same expression that sized the allocation.
There is no annotation for "C keeps this pointer"; until there is, the rule is the
one at the top of this paragraph, and it is a rule about *your* code, not a
guarantee loft enforces.

**The `char *` return — three answers C's type system cannot give**, so the
binding gives them, the same way on every backend:

- **loft never frees it.**  `strerror` and `PQerrorMessage` hand back storage the
  caller must *not* free; `strdup` hands back storage it must.  `const` does not
  separate them — POSIX spells both `char *` — so a guess would free static
  memory, and that failure is not recoverable while a leak is.  A **caller-frees**
  function therefore goes through an ANSI-C shim, which is what shims are for.
- **The bytes end at the first NUL**, because that is what `char *` means.  A loft
  `text` carries a length and may hold an interior NUL; the crossing truncates
  there rather than inventing a length.
- **NULL is loft null, and invalid UTF-8 is replaced** (loft text is UTF-8; a
  locale-encoded byte from C must not take the program down).  Spell the return
  **`text?`** when NULL is a real answer — it does not add the null, it makes the
  null-flow analysis demand a discharge for it, which a bare `text` carries
  silently.

A pointer return that is *not* spelled `char *` is refused against a `text`
declaration: `void*` bound to `text` is either a mistake or a handle that wanted
`integer`, and nothing at runtime tells the two apart.

#### Declaring the library (arc D)

```toml
[c]
libs = "libpq.so.5"            # a soname the dynamic linker knows
# libs = "../../libmine.so"    # or a path, resolved against the package dir
optional-libs = "libduckdb.so" # bound, but not required to be installed
shim = "src/shim.c"            # ANSI-C loft compiles itself, with `cc`
```

The interpreter `dlopen`s each entry and keeps it loaded (a `#c` symbol is
looked up through it); `--native` links the same list.  One declaration, both
halves.  A binding to **libc needs no entry** — it is already in the process.

Distinct from `[native] runtime-libs`, which names what a Rust cdylib needs
present and only probes for it.

#### Optional libraries — `optional-libs` (arc G)

**`libs` means the package does not work without it.**  Absent, the failure is
early and actionable: the interpreter reports it, and `--native` will not even
link.  That is the right answer for a package's one reason to exist.

**`optional-libs` means the package binds it but works without it.**  It is not
linked and not opened at load; it is opened when a symbol from it is first
looked up, and `--native` resolves it at that moment too instead of putting it
on the link line.  So a program that never calls into it **builds and runs on a
machine where the library is not installed** — which is what lets one package
offer several backends without making a user install all of them to use one.

The cost is that presence becomes a question the program must ask:

```loft
if c_library_available("libduckdb.so") { … } else { /* fall back */ }
```

Ask it *before* the first call.  A `#c` symbol that cannot be resolved **faults**
— `#c` is the declared edge of loft's totality, not a null-returning
computation — so this query is what keeps an optional backend inside the
no-runtime-errors rule (C80).

`c_library_available` answers true when the library loads **and** every `#c`
symbol attributable to it resolves.  Both halves matter: a library of the wrong
vintage loads and exports only some of its symbols, so "the file is there" would
say yes where the call still faults.

**Declare at most one optional library per package.**  A `#c` annotation never
names the library it comes from, so symbols are attributed by package — and with
two optional libraries in one package nothing says which one exports what.  Such
a package still answers the load question correctly but gives up the skew half.
Required entries do not count: a package cannot load without them, and one of
them is usually its own `shim`, which loft just built.

**`shim`** names ANSI-C sources the package ships for the signatures the fixed
trampolines cannot express — a `double` argument, a struct by value, varargs, an
out-parameter, a caller-frees `char *` return.  loft compiles them with **`cc`,
never rustc** (the whole point of `#c`), and the result is then registered
*exactly like a `libs` entry*: nothing downstream can tell a shim from any other
C library, so the interpreter's `dlopen`, the `--native` link line and the symbol
resolver stay one code path.

The artifact lands in the package's `native-auto/` and is **content-addressed**
by the shim sources plus the compiler's identity — editing a shim produces a
different file rather than racing to overwrite one another process may be
reading, and a toolchain change rebuilds rather than reusing a stale ABI.  An
existing artifact IS the freshness check, so a warm run costs nothing.

`loft install` builds it at install time, so a package needing a C compiler says
so while the user is installing packages, not inside the first run of their
program.  A failure there is surfaced and the install still succeeds — the
parser reports it again, with the `use` site, if the package is actually used.

#### The wasm and browser targets — arc E

**A `#c` binding is refused on both wasm targets, at the call.**  `--native-wasm`
(wasip2) and `--html` (the browser) each report one message naming the loft
function, the C symbol, the declaring package and the target:

```
error: loft: `client_info` is bound to the C symbol 'mysql_get_client_info' with #c
       (package `mariadb`), and the wasm (wasip2) target has no C ABI to reach it —
       a wasm module cannot open a shared library. Give the library a wasm
       implementation, host it out of process (@PLN119), or drop the
       --native-wasm claim (@PLN24 arc E)
```

**Refused at the CALL, so a declaration is still portable.**  A library may
declare `#c` bindings and still build for wasm as long as the wasm program does
not reach one — the same rule `#native` follows for a routeless browser symbol.

**Why refusal rather than support**, in the order the measurements came in.
`wasm32-wasip2` links a libc, so a binding to `strlen` did *resolve* — and then
trapped, because wasm32 is a third data model (ILP32: `long`, `size_t` and every
pointer are 32 bits) while the extern carried the host's widths.  A symbol the
sysroot does *not* export gave a raw linker error naming neither package nor
library.  Neither is a capability a library can rely on: a `#c` library binds a
system library, and there is no `dlopen` in wasm to reach one with.

The two routes that do work are a **wasm implementation of the library** (the
same answer `#native` needs — a `[wasm.bridge]`), or **hosting the C out of
process** ([@PLN119](https://github.com/loft-lang/plans/issues/119)), where
"another process" and "another machine" are already one mechanism.  Compiling a package's
own `[c] shim` to wasm with a C cross-compiler is a third, and is not built: it
needs a `wasm32-wasi` C toolchain in the build environment, and it would cover
only a shim that is pure computation — never a database client, whose capability
does not exist in a browser at all.

`c_library_available` compiles on both wasm targets and answers **false** there,
so the guard an optional backend already writes keeps working when the same
source is built for wasm.

#### Under the sandbox

**A `#c` binding is gated by `native_ffi`, never by a `#cap` grant.**  A
capability says what data a script may touch; a C call runs machine code loft
cannot inspect, which is the line [`native_ffi`](SANDBOX.md) already draws for a
Rust cdylib bridge — and `#c` is the stronger case, having no marshalling layer
at all.  An allow-listed library (`allow_libs`) still admits its bindings: that
is the host vetting the library as a unit, exactly as for `#native`.

**Status: done on both backends and defined on all four targets** — `--native`
compiles the declaration into a typed `extern "C"` and calls it directly; the
interpreter resolves the symbol and calls it through a fixed ladder of per-arity
trampolines.  The two produce identical results, which is the bar.  Design in [plans/24-c-abi-binding](plans/24-c-abi-binding/README.md) /
[@PLN24](https://github.com/loft-lang/plans/issues/24) (first consumer: the
MariaDB/PostgreSQL clients, @PLN23), matrix + probe in
`tests/fixtures/c_abi/`.  `#native` remains today's path.
