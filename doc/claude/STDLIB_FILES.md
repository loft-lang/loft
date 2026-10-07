# Standard Library: File System

Paths, files, directories, binary I/O, durable and path-backed stores, lazy store binding and images.  Part of the standard library reference: [STDLIB.md](STDLIB.md).

---

## File System

Types and functions for reading and writing files. A `File` value is obtained via `file()` and carries the path, format, and an internal reference.

### Path resolution (program-relative by default)

A **relative** file path resolves against **the program's own directory**, not
the process working directory:

- under `--interpret` / tests → the directory of the main source file;
- under `--native` → the directory of the compiled executable;
- queryable as `source_dir()` (works on every backend).

So `file("assets/font.ttf")` loads the asset that ships **next to the program**,
regardless of where it was launched from — "program + assets" is a portable
bundle.  **Absolute paths are never rewritten.**

This applies uniformly to every path-taking operation — `file()`, `exists()`,
`read_file`/`write_file`, the `File` methods, `delete`/`move`/`mkdir`/`mkdir_all`/`rmdir`,
and image loads — so they all agree on where a relative path points.

**Opting into cwd-relative (CLI tools).** A program that takes a *user-supplied*
relative path (`loft tidy.loft data.csv` — `data.csv` is in the user's cwd, not
beside the script) declares the file-top directive:

```grammar
#cwd
fn main(args: vector<text>) { ... }   // relative paths now resolve against the cwd
```

`#cwd` is whole-program and must precede the first declaration.  Per-invocation,
the `LOFT_PATHS` environment variable overrides both: `LOFT_PATHS=program` forces
program-relative, `LOFT_PATHS=cwd` forces cwd-relative.  `source_dir() -> text`
returns the anchor (empty only when there is none, e.g. a wasm host with no
filesystem).

### Types

**`Format`** (enum): Describes how a file is opened.

| Value           | Description |
|-----------------|-------------|
| `Format.TextFile`     | Default. Read or write as UTF-8 text. |
| `Format.LittleEndian` | Binary mode, least-significant byte first. |
| `Format.BigEndian`    | Binary mode, most-significant byte first. |
| `Format.Directory`    | Represents a directory path. |

**`File`**: A handle to a filesystem entry. Fields: `path: text`, `size: integer`, `format: Format`.

### Opening Files

| Function | Description |
|----------|-------------|
| `file(path: text) -> File` | Opens the file at `path` and returns a `File` handle. |

**A relative path resolves against the program's own directory** (#255 / @PLN9),
so `../data.txt` names the file above the script — the same file its absolute
form names, and it answers the same either way. There is no path-shape filter:
`..` is resolved, not inspected.

If a program must not reach outside a directory, check that yourself before the
call; the stdlib does not, and did not meaningfully do so before.

### Reading Text Files

| Function | Description |
|----------|-------------|
| `content(self: File) -> text?` | Reads the entire file as a UTF-8 text value. **Null** when there is no text to read: the file is missing, the path is a directory, or the bytes are not valid UTF-8. `""` means the file really is empty. |
| `lines(self: File) -> vector<text>` | Reads the file and splits it into lines. **Empty** wherever `content()` is null — a missing file, a directory, or bytes that are not UTF-8 — so a loop over it runs zero times and reads exactly like a loop over an empty file. `lines()` has no null of its own to carry the distinction; ask `content()` when it matters. |

`content()` is nullable because `""` cannot carry three different meanings — a
missing file, a directory and non-UTF-8 bytes are each `null`, never an empty text
(guard `tests/binary_io_matrix.rs`).  Discharge with `?? ""` where the distinction
does not matter.

Reading such a file is not the failure — asking for it as *text* is.  Use
[`read_bytes(path) -> vector<u8>?`](#filesystem-operations), which is
byte-exact and round-trips with `write_bytes`, or open the file in binary mode
and read it a field at a time (see [Binary Files](#binary-files)).  A non-UTF-8
`content()` also prints one stderr warning naming both, on **both** backends.

### Writing Text Files

| Function | Description |
|----------|-------------|
| `write(self: File, v: text)` | Writes `v` as UTF-8 text at the file's position. The first write through a `File` replaces the file's content; later writes through the same `File` follow it. `#index`/`#next` then frame what was written, and a read through the same `File` continues from there (`tests/scripts/one-file-handle-reads-and-writes-and-a-short-read-is-null.loft`). |

### Binary Files

Binary mode must be activated before reading or writing raw data. Use `f#format = LittleEndian` or `f#format = BigEndian` to enable binary mode (`tests/reference/skill-files.loft`).

| Function | Description |
|----------|-------------|
| `seek(self: File, pos: integer) -> boolean` | Moves the read/write position to `pos` bytes from the start — random access into a binary file. `false` (a no-op) for a directory, an absent file, a negative `pos`, or a file this process has not read from or written to yet (the OS handle opens on first I/O). Seeking PAST the end is allowed: a following write extends the file. Operator form: `f#next = pos`. |
| `position(self: File) -> integer` | The byte offset the next read or write will land at — the read side of `seek`. Operator form: `f#next`. Distinct from `f#index`, which is where the LAST read *started*: after one `f#read as i32` on a fresh file, `position` is 4 and `f#index` is 0. **Null** for a file this process has not opened yet (0 is a real position, so it is not used to mean "no position"). |

**Binary attribute operators on `f: File`:**

| Syntax | Description |
|--------|-------------|
| `f += value` | Writes `value` at the width of its declared type — `i32`/`u32` → 4B, `u8` → 1B, `u16` → 2B, range-constrained `integer limit(0,255)` → 1B, bare `integer` (i64 storage) → 8B, `single` → 4B, `float` → 8B, `text` → raw UTF-8 bytes, `vector<T>` → each element at its declared width. |
| `f#read as T` | **Preferred** — reads `T`'s natural byte width and returns a value of type `T`. `as i32` reads 4B, `as u8` reads 1B, `as u16` reads 2B, `as integer` reads 8B. |
| `s.field = f#read` | **LHS-inferred** — width comes from `s.field`'s declared type; symmetric with `f += s.field`. No `as T` needed. |
| `f#read(n) as T` | Legacy explicit form — reads exactly `n` bytes and interprets as `T`. **Null** when the file holds fewer than `n` bytes past the position, or when `n` is less than `T`'s storage width; the position does not move then. |
| `f#read(n) as text` | Reads exactly `n` bytes (or fewer at EOF) as a UTF-8 string. The `(n)` is REQUIRED for text — variable-width types have no inferable count. |
| `f#size` | The file size in bytes as `integer`, as it was when the handle was opened — a handle's own `+=` writes are not counted until the file is reopened (both backends; `tests/reference/skill-files.loft`). |
| `f#index` | Returns the byte offset where the last read started (the `current` field). |
| `f#next` | Returns the current byte position (after last read). |
| `f#next = pos` | Seeks the file to `pos` (integer). Only works after the file has been opened by a prior read or write. |
| `f#exists` | Returns `true` if the file or directory exists (format ≠ `Format.NotExists`). |
| `f#format` | Reads the `Format` enum value of `f`.  **Avoid on binary files** — accessing `.format` on a file with unrecognized magic panics in `src/database/io.rs:276`.  Use `exists(path)` instead for existence checks. |
| `f#format = Format.X` | Sets the format of `f`. |

**Notes:**
- `f += "text"` writes raw UTF-8 bytes; supported for TextFile, LittleEndian, and BigEndian modes.
- A `File` is one handle for reading and writing, whichever came first: a write after a read lands at the read's position (`f#next`), and a read after a write starts after what was written.
- For new files (format=NotExists), `f += value` defaults to TextFile mode and creates the file.  An EXISTING file is opened without truncation, so `f += value` appends after its last byte; `delete(path)` or `f.set_file_size(0)` first to start over.
- `f#next = pos` (and the `seek` method above) is a no-op if called before the first read or write — the OS file handle does not exist until first I/O. Always perform a read or write before seeking. `seek` returns `false` in that case; the operator form reports nothing, which is why the method is the better choice when the position matters.

#### Struct and vector binary round-trip (@PLN47 — shipped 2026-07-09)

`f += my_struct` and `s = f#read as MyStruct` now round-trip for any
struct whose fields are all fixed-width scalars (integer, float, single,
boolean, character, i8/u8/i16/u16/i32/u32).  Nested plain structs (a
struct field of struct type) also round-trip.  The record is allocated
before reading, so each field is filled in declared order.

**`f#read(N) as vector<T>`** reads exactly `N` **bytes** (not elements)
and returns a `vector<T>`.  Example: `f#read(24) as vector<integer>`
reads three 8-byte ints.  A parse-time warning fires when a literal `N`
is not a multiple of the element byte-width, catching the silent
empty-vector footgun.

**`f += ch` / `f#read as character`** round-trips as 4 bytes on both
backends.  **`f += b` / `f#read as boolean`** round-trips as 1 byte on
both backends.  **Signed narrow ints** (`i8`, `i16`) read back with
correct sign extension (e.g. `-12345 as i16` round-trips to `-12345`).

**Structs with variable-width fields** (`text`, `vector`, or any
collection field) are **rejected at compile time** on both backends:

```
read_file: 'T' has variable-width field 'name' (text/vector/collection)
that binary I/O cannot round-trip; serialise a plain fixed-width struct
```

The write-side diagnostic says `write_file:`.  Nested-struct fields are
reported as `outer.inner`.

**`f += text` / `f += vector<T>`** still write raw bytes with no length
prefix.  Callers that manage their own length tracking (e.g. GLB chunks
with the count in the outer header) continue to use this form and read
with `f#read(N) as text` / `f#read(N) as vector<T>`.

**Retained caveat (@P293)**: `u32`/`i32` values ≥ 2³¹ round-trip via
raw bytes but read back as negative i64 in loft expressions.

**Known limitation**: `q = f#read as Struct` leaks one record per read.
This is a pre-existing general loft ownership bug — `x = { …; struct_temp }`
leaks the temp store even with no file I/O (function returns and direct
struct literals are clean; only the inline-block-returning-a-struct form
leaks).  Tracked separately as an ownership-model issue.

Test harness: `tests/binary_io_matrix.rs` (32 cross-mode cells,
`#[ignore]`, run with
`cargo test --release --test binary_io_matrix -- --ignored`).

### Directories

| Function | Description |
|----------|-------------|
| `files(self: File) -> vector<File>` | Returns the entries inside a directory, sorted by path — the same order as `list_dir`, so an index means the same entry in either listing. The `File` must have `format == Format.Directory`; anything else lists as `[]` (never null, unlike `list_dir`). |

### Filesystem Operations

Mutating filesystem operations return a `FileResult` enum:

| Variant | Meaning |
|---------|---------|
| `FileResult.Ok` | Operation succeeded. |
| `FileResult.NotFound` | Path does not exist. |
| `FileResult.PermissionDenied` | OS permission denied. |
| `FileResult.IsDirectory` | A file operation targeted a directory (e.g. `delete()` on a directory). |
| `FileResult.Other` | Any other OS error. |

Every variant is actually produced: `--native` and `--interpret` classify from the OS
error; the wasm host reports only `Ok` / `Other`, and `NotFound` still comes from the
loft-level existence check.

| Function | Description |
|----------|-------------|
| `ok(self: FileResult) -> boolean` | Returns `true` if `Ok`. |
| `exists(path: text) -> boolean` | Returns `true` if the path exists. |
| `exists(self: const File) -> boolean` | Method form: `f.exists()` or `exists(f)`. |
| `delete(path: text) -> FileResult` | Removes a file. |
| `move(from: text, to: text) -> FileResult` | Renames or relocates a file. |
| `mkdir(path: text) -> FileResult` | Creates a single directory level. |
| `mkdir_all(path: text) -> FileResult` | Creates a directory and all missing parents. |
| `rmdir(path: text) -> FileResult` | Removes an EMPTY directory; `NotEmpty` when it still holds entries. A recursive removal is the caller's walk — `list_dir`, `delete`, then `rmdir` deepest-first. |
| `is_dir(path: text) -> boolean` | Returns `true` if the path exists and is a directory. |
| `is_file(path: text) -> boolean` | Returns `true` if the path exists and is a regular file. |
| `is_symlink(path: text) -> boolean` | Returns `true` if the path is itself a symbolic link, whatever it points at — `is_dir`/`is_file` answer for the target, so a walk that must not follow links asks this first. `false` on a browser host. |
| `list_dir(path: text) -> vector<text>?` | Entry names (base names, sorted) of a directory. **Null** when the path is missing or is not a readable directory; `[]` means the directory really is empty. Discharge with `?? []`. |
| `read_bytes(path: text) -> vector<u8>?` | Reads the whole file as raw bytes. **Null** when the file is missing or unreadable; `[]` means the file really is empty. Binary-exact (round-trips with `write_bytes`). Discharge with `?? []`. |
| `file_map(path: text) -> const vector<u8>?` | Maps the whole file READ-ONLY without copying it: the vector's bytes are the file's, mapped for as long as the vector lives. **Null** when missing or unreadable; `[]` for an empty file. Every read works as on any `vector<u8>`, and a slice is a view of the same bytes. The vector is `const`: a write to it or a slice, or passing it to a parameter that is not `const`, is refused when the program is compiled — copy first (`w = v`, a bind copies). |
| `write_bytes(path: text, bytes: vector<u8>) -> boolean` | Writes raw bytes to a file, truncating existing content; `true` on success. |
| `set_file_size(self: File, size: integer) -> FileResult` | Truncates or extends a file to exactly `size` bytes. |

### Durable stores (`@PLN43`)

A "durable store" is a regular on-disk file plus a 40-byte `.dmeta`
sidecar holding signature + tier + CRC32 over the main file + a
clean-close timestamp.  After a successful write session, call
`store_durable_seal(path)` to record the current state; on next
startup, call `store_durable_check(path)` to verify integrity.  A
failing check means the file is missing, corrupt, or the sidecar is
stale — the caller is expected to rebuild from authoritative sources.

| Function | Description |
|----------|-------------|
| `store_durable_check(path: text) -> boolean` | Returns `true` iff the `.dmeta` sidecar at `<path>.dmeta` validates against the main file at `path` (signature, header CRC, payload length, payload CRC, tier_id all OK).  Returns `false` on any failure or missing file.  Phase-01b Tier-1 only (no msync discipline). |
| `store_durable_seal(path: text) -> boolean` | Writes a fresh `.dmeta` sidecar capturing the current main-file's byte length + CRC32 + a clean-close timestamp.  Returns `false` on any I/O error.  Pair with `store_durable_check` to bracket each write session. |

**Both are desktop-only, and say so by answering `false`** (loft#1063).  The sidecar
machinery is built over memory-mapped files, which the wasm targets have no equivalent
for, so on `--native-wasm` and `--html` each call compiles and returns `false` — the same
route `store_persist_bind` takes.  `false` is the honest answer on that target and the safe
one: it means *the seal did not happen* and *nothing validates*, which is exactly the state
that sends a caller down its rebuild branch.  Seal where the store is WRITTEN (a desktop
build) and use the portable readers — `store_load`, `store_load_key*` — in the browser; an
image sealed by `--native` loads byte-identically on wasm.

Usage pattern:

<!-- from tests/reference/durable-stores.loft -->
```loft
path = "{temp_dir()}/loft-reference-data.bin";
if !store_durable_check(path) {
  rebuild_from_source(path);  // consumer-defined
}
// ... use the database that lives in `path` ...

// graceful shutdown
flush_database();
store_durable_seal(path);
```

If the program crashes between the last write and the seal, the
sidecar stays stale relative to the file → next start's
`store_durable_check` returns `false` → caller rebuilds.  This is
**by design** — Tier 1 trades durability for cheap writes (no
`msync` on the hot path) and recovers via rebuild from a
re-derivable source.  **Do not use** for data that cannot be
re-derived; Tiers 2 (snapshots) and 3 (WAL) are planned for that
case but not yet shipped.

Full design + format spec:
[`doc/claude/plans/43-loft-store-durable/`](plans/43-loft-store-durable/README.md).

### Path-backed hash storage (`@PLN43`)

`store_persist_bind(h, path)` re-roots the Store backing a hash at a
file path so mutations are durable via mmap without an explicit save
loop — "the hash IS the file."  Dryopea's persistence destination ask
([`QUESTIONS_FOR_LOFT.md` § Path-backed user-data Store binding](https://github.com/jjstwerff/dryopea)),
also the canonical pattern for any single-collection on-disk state.

| Function | Description |
|----------|-------------|
| `store_persist_bind(h: hash, path: text) -> boolean` | Re-roots the Store backing `h` at a file at `path`.  Fresh-path branch: snapshots the current bytes (padded to ≥1024 words with a valid tail-free block), writes them, and mmaps the file.  Existing-path branch: opens the file via mmap and adopts its contents (discarding the in-memory state at that slot) — unless the `.dschema` beside it records a different layout than `h`'s type, which is refused as `store_load` refuses it.  Returns `false` on any I/O / format / layout error — no panic; callers fall back to JSON or rebuild. |
| `store_persist_copy(r: reference, path: text) -> boolean` | Writes an image of `r` laid out for PAGING and leaves the live collection where it is — nothing moves, so every reference stays valid, and the file is NOT bound.  The image is REBUILT, so each record sits in its collection's own order (key order for a `trie`), which is what makes a paged prefix query cheap: measured 4.9 requests / 0.32 MB against a bound image's 19.9 / 1.28 MB on a 74,692-word vocabulary.  Sized to its content, with none of the growth slack a bound file keeps.  Use it for a file another program or a browser will READ; use `store_persist_bind` for a store you go on writing to. @PLN134 |

Usage pattern:

<!-- from tests/reference/durable-stores.loft -->
```loft
h: hash<Entry[key]> = [];

// Bind first — on a fresh path the empty hash is serialised; on an
// existing path the on-disk contents are loaded into this slot.
store_persist_bind(h, "{temp_dir()}/loft-reference-world.store");

// Subsequent mutations hit the mmap'd buffer.  No explicit save.
h += Entry { key: 7, value: 700 };
// ... OS msyncs on idle / clean exit ...
```

**Semantics in detail:**

- When `path` does not exist at call time, `store_persist_bind`
  captures the current in-memory bytes of the hash's Store, pads
  them out to a valid ≥8192-byte image, writes them to disk, and
  swaps the slot's allocator over to the mmap'd file.  The caller's
  `h` is unchanged from a record-layout perspective — the DbRef
  shape `(store_nr, rec, pos)` stays valid, only the underlying
  buffer moves from anonymous heap to mmap.
- When `path` exists, the call first compares the layout recorded
  in `<path>.dschema` with `h`'s type.  A different layout — a
  struct that gained, lost or changed a field — is refused: the
  call returns `false`, prints what differs on stderr, and leaves
  the file, its `.dschema` and `h` untouched.  A file without a
  `.dschema` is not checked.  Otherwise the call invokes
  `Store::open(path)`, which validates the loft Store signature and
  rebuilds the free-list.
  The caller's prior in-memory state at that slot is discarded.
  The caller's existing `DbRef`s into the hash remain valid IFF the
  on-disk layout describes the same type — the standard pattern is
  to allocate an empty hash and immediately bind, so the empty
  in-memory state is harmlessly discarded in favour of the on-disk
  view.
- Pair with `store_durable_check` / `store_durable_seal` (above)
  when you also want Tier-1 integrity assurance — the bracket
  pattern is unchanged, only "what's between check and seal"
  becomes "nothing — the hash mutations ARE the writes."

**Persisted size = the store's *peak* arena, not its live size — a trap
worth planning for.** The fresh-path snapshot writes the hash's whole
Store arena at the capacity it has ever reached, free slack included; it
is not compacted down to the live records (minimum ~8 KB image). Three
facts combine to make this bite:

- the snapshot copies the arena's current capacity verbatim — no repack;
- a Store arena only ever grows — it never shrinks;
- freeing or clearing records returns their slots to a free list but does
  **not** hand the arena bytes back.

So the file is the high-water mark the bound Store reached during its
life. Build a large structure *in place* in the same Store you then bind
— inserting then pruning, or rebuilding as you go — and the file freezes
that peak: one consumer saw a 264 MB file for 3.5 MB of live data (~90×).

**Keep the scratch out of the Store you bind: build the result in a
helper and return it.** A helper's transient containers are scope-local
Stores, freed at the return boundary (see [LIFETIME.md](LIFETIME.md)), so
their growth never lands in the Store that survives as the return value.
Bind *that* returned hash:

<!-- from tests/reference/durable-stores.loft -->
```loft
fn build_world() -> hash<Hex[q, r]> {
  w: hash<Hex[q, r]> = [];
  // fill w; any transient collections here are freed on return
  w += Hex { q: 0, r: 0, kind: "plain" };
  return w;
```

and bind the result once it is built:

<!-- from tests/reference/durable-stores.loft -->
```loft
world = build_world();                 // carries only the live result
store_persist_bind(world, "{temp_dir()}/loft-reference-hexes.store");
```

Whether a top-level build actually bloats depends on the pattern
(in-place insert-then-prune is the classic case); the reliable rule is to
keep transient growth out of the Store you bind.

**Failure modes (returns `false`):**

- Path is empty or not valid UTF-8.
- Existing file's signature doesn't match the loft Store format
  (caught via `catch_unwind`; no panic propagates).
- I/O error writing the fresh-path snapshot.
- `mmap` feature disabled in this build.

Off when the `mmap` Cargo feature is disabled at build time:
returns `false` so consumers branch into a JSON fallback (or
rebuild-from-source).

### Lazy store binding (`@F108`)

Bind a collection to a source and let the LOOKUPS do the loading: a lookup that
misses fetches exactly that one entry and inserts it, so the next lookup for the
same key is an ordinary resident hit.  The collection is therefore its own cached
working set — there is no second structure to keep in step with it.  Contrast
`store_load_key` ([REMOTE_STORES.md](REMOTE_STORES.md)), where the program names
the entries to fetch.

The model behind these calls — what a query is derived from, why `len` answers the
resident count, and what a binding refuses — is [LAZY_STORES.md](LAZY_STORES.md).

| Function | Description |
|----------|-------------|
| `store_bind_lazy(c: reference, source: text) -> boolean` | Bind collection `c` to `source` — an IMAGE (a local `.store` file or an `http(s)://` URL served with Range, i.e. whatever `store_load_key` accepts) or a DATABASE (`sqlite:<path>`), where the `SELECT` is derived from `c`'s own type: table = the element type's name lowercased, columns = its fields, `WHERE` = its key.  Read-only; the database source serves a keyed lookup on any ordered or hashed kind, and a binding it cannot turn into a query is refused through `store_lazy_error` rather than served wrongly.  Per COLLECTION, not per store: two collections of one type may bind differently.  Binding replaces any previous binding, and may be done before `c` holds anything.  **`false` is worth checking**: besides a null collection, an IMAGE is read a page at a time and only a `hash` or a `trie` supports that, so a `sorted`/`index`/`spatial` bound to one is refused HERE rather than answering `null` at every later lookup (guard `tests/scripts/802-lazy-refusal-visible.loft`) — those kinds load whole, with `store_load` / `store_load_url_trusted`.  A DATABASE source judges its own schema on the first fault instead, since what it can serve is a fact about the other end. |
| `store_lazy_range(c: reference, lo: integer, hi: integer) -> integer` | Pull a whole KEY RANGE from `c`'s bound DATABASE source in ONE query (bounds inclusive, in the collection's own key order); answers how many records `c` gained.  The cure for N+1: 500 records fetched one lookup at a time is 500 round trips, and the same 500 as a range is one.  `c` must be ORDERED (`sorted`/`index`) and keyed on one column — a `hash` has no order to range over and a composite key needs `store_lazy_query`.  A record already resident is left alone. |
| `store_lazy_query(c: reference, condition: text) -> integer` | Run an explicit SQL `condition` against `c`'s bound DATABASE source and pull every matching row INTO `c`; answers how many records `c` gained.  The escape hatch for what the key cannot express (`name LIKE 'Ada%'`, a predicate on another column) — derived queries need no call, this one cannot be derived, so it is written down and visible.  Rows land in the collection rather than in a detached result, and a row already resident is left alone: a person found this way and the same person found by key are ONE record.  Answers `0` both for "nothing matched" and for "the query could not run"; `store_lazy_error` tells those apart. |
| `store_lazy_error(c: reference) -> text` | Why a fetch could not REACH the source, or `""` when healthy.  The FIRST failure's reason, kept — it names the original cause, so a later and often more actionable one reaches stderr but not this call.  Nothing clears it but `store_lazy_clear`: neither a genuine absence nor a later success is an acknowledgement, because reaching the source now says nothing about what an earlier failure already lost. |
| `store_lazy_faults(c: reference) -> integer` | How many fetches could not reach the source.  `0` is healthy; after a traversal it answers "how incomplete am I". |
| `store_lazy_clear(c: reference) -> boolean` | Acknowledge those faults, answering whether there was anything to acknowledge.  The ONLY thing that clears them. |
| `store_lazy_fail(c: reference, why: text)` | **The writing end of that channel**, for a lazy driver written in loft (`fn lazy_fetch(…)`, below).  A driver's three answers do not fit its return value: `1` inserted and `0` absent are integers, and "the source is down" carries a REASON — answering `0` for it is the silent wrong answer this channel exists to prevent.  Sticky and counted exactly like a Rust source's failure. |

**Ask after a null, because a null cannot say why.**  C80 means a value read never
raises, so a miss answers `null` whether the key is genuinely absent or the source
was unreachable — two different facts, one stable and one not:

<!-- from tests/reference/lazy-error.loft -->
```loft
p = persons[42];
if p == null {
  why = store_lazy_error(persons);
  if why == "" { verdict = "really no such person"; } else { verdict = "could not reach: {why}"; }
}
```

**Faults are sticky, and only `store_lazy_clear` clears them.**  A later fetch
that happens to succeed does not: a traversal whose first lookup could not reach
the source and whose second could is MISSING data, and reporting "healthy"
afterwards would be exactly the silent wrong answer this channel exists to
prevent.

The source is pinned at bind time, so a traversal sees one consistent world; an
image that changes underneath is REFUSED and reported through the fault channel
rather than silently mixing two versions.  `len` is the RESIDENT count, not the
source's.  Assigning `= []` reclaims what the collection holds while keeping the
binding and preserving held references — the blunt way to cap a working set.

### Images — `use imaging;`

These live in the **`imaging` package**, not the always-loaded stdlib: `Image` and
`Pixel` were drained out of `default/` because image types are not language
primitives. Add `use imaging;` (or call them qualified, `imaging::…`) or the names
do not resolve at all.

`png` is a METHOD, so a missing import reads as `Unknown field File.png` with no
package named — the did-you-mean hint that redirects a free function to its package
does not cover methods yet.

| Function | Description |
|----------|-------------|
| `png(self: File) -> Image` | Decodes a PNG file and returns an `Image`. Returns null unless the file exists and is readable (`Format.TextFile`, which is loft's classification for any ordinary file — a PNG included). |

**`Image`** struct fields: `name: text`, `width: integer`, `height: integer`, `data: vector<Pixel>`.

**`Pixel`** struct fields: `r: integer`, `g: integer`, `b: integer` (each 0–255).

| Function | Description |
|----------|-------------|
| `value(self: Pixel) -> integer` | Returns the pixel colour as a packed 24-bit integer (`0xRRGGBB`). Use for fast colour comparison or storage. |

**Example — read a PNG's dimensions:**
<!-- from library:imaging -->
```loft
use imaging;
fn main() {
  img = file("assets/map.png").png();
  println("{img.width}x{img.height}");
}
```
