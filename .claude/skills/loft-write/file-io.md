<!-- reference for the loft-write skill: read when a program touches files -->
# loft-write: file I/O reference

The file I/O half of the [loft-write skill](SKILL.md).

## File I/O patterns

### Path resolution — relative means program-relative

A **relative** path resolves against the **program's own directory**
(`source_dir()` — source dir under `--interpret`, exe dir under `--native`), not
the process cwd.  So `file("assets/x")` loads the asset bundled beside the
program on every backend, regardless of launch directory.  **Absolute paths are
untouched.**  The file builtins (`file`, `exists`, `read_file`/`write_file`,
`delete`/`move`/`mkdir`, image loads) all resolve this way — so **don't hand-roll
`"{source_dir()}/{path}"` joins** in normal loft; just pass the relative path.
(Do the explicit join only when handing a path to a *non-loft* consumer, e.g. a
native asset loader that reads the filesystem directly.)

A program that must resolve a *user-supplied* relative path against the **working
directory** (CLI tools — `loft tidy.loft data.csv`) declares `#cwd` as the
file-top directive (before the first declaration).  `LOFT_PATHS=program|cwd`
overrides per-invocation.

### Text files (UTF-8)

<!-- from tests/reference/skill-files.loft -->
```loft
out = file("{temp_dir()}/loft-reference-output.txt");
out.write("one\ntwo\n");
f = file("{temp_dir()}/loft-reference-output.txt");
content = f.content();         // full text content (UTF-8)
lines = f.lines();             // vector<text> of lines
dir = file(temp_dir());
names = "";
for ef in dir.files() {
  path = ef.path;
  names += path;
}
size_bytes = f.size;            // integer (i64) — works for any file
```

**`f.content()` is UTF-8-only** (`-> text?`).  On a binary file it
returns **null** with a warning.  For non-text data use
`read_bytes(path) -> vector<u8>` / `write_bytes`, or the structured
binary idiom below.

### Binary files (structured reads and writes)

Set `f#format` to `LittleEndian` or `BigEndian`, then use `f#read`
for reads and `f += value` for writes.  `#next` seeks to an
absolute byte offset.  All file-handle operations should live
inside a `{ ... }` scope block so the handle flushes/closes at
block exit:

<!-- from tests/reference/skill-files.loft -->
```loft
// `file()` opens WITHOUT truncating — `f += …` appends to what is there, so a rerun would
// double the file; start from nothing.
delete("{temp_dir()}/loft-reference-model.glb");
// --- Write a binary chunk-structured file ---
{
  f = file("{temp_dir()}/loft-reference-model.glb");
  f#format = LittleEndian;
  f += (0x46546C67 as i32);   // 4 bytes: i32 magic
  f += (2 as i32);            // 4 bytes: i32 version
  f += (32 as u8);            // 1 byte (ASCII space)
  f += "chunk of text";       // raw UTF-8 bytes
  f += my_float_vector;       // vector<single> → 4 bytes per element
}
```
<!-- from tests/reference/skill-files.loft -->
```loft
// --- Read the 12-byte GLB header ---
{
  f = file("{temp_dir()}/loft-reference-model.glb");
  f#format = LittleEndian;
  magic   = f#read as i32;            // 0x46546C67 = 'glTF'
  version = f#read as i32;
  space   = f#read as u8;
  // Seek past the header + JSON data to a later chunk:
  f#next = (9 + json_len) as integer;
  text_len = 13;
  assert(magic == 0x46546C67 and version == 2 and space == 32, "the header fields read back");
  assert(f.size == 4 + 4 + 1 + text_len + 8, "and the file is exactly the bytes written");
}
```

Notes:
- **Prefer `f#read as <type>` (no parens) for fixed-width reads.**
  The byte count is inferred from the type — `as i32` reads 4
  bytes, `as u8` reads 1, `as u16` reads 2, `as integer` reads 8.
  The legacy `f#read(n) as T` form still works but the `(n)` must
  match the type's storage width exactly or the runtime panics.  The inferred form makes that mismatch
  impossible.  `as text` still needs `f#read(n) as text` because
  text has no fixed width.
- **`s.field = f#read` (no `as T`) infers width from the LHS field's
  declared type** — symmetric with `f += s.field`.  For a struct
  `S { a: i32, b: u8, c: u16 }`, both sides become:
  `f += s.a; f += s.b; f += s.c` to write, `s.a = f#read; s.b = f#read;
  s.c = f#read` to read.  Changing a field's declared type
  (`u16` → `i32`) automatically updates both sites at the next compile —
  no manual cast edits needed.
- **Always cast scalar writes to the intended width.**  Bare
  `f += int_var` writes 8 bytes (loft stores integers as i64).  To
  write 4 bytes use `f += (int_var as i32)`; for 1 / 2 bytes use
  `as u8` / `as u16`.  Strongly-typed struct fields (`u8`, `u16`, or a
  range-limited `integer limit(0, 255)`) write at their declared width
  automatically.
- `f += expr` appends `expr` to the file, respecting the `#format`
  endianness.  `text` → raw bytes, `vector<T>` → each element in
  sequence at its declared width.
- `f.size` returns an `integer` (i64); compare with `0`.
- `f#next = offset as integer` seeks.  Reading position advances
  automatically after each `f#read` — don't manually advance it
  between sequential reads.
- **Whole-buffer reads: `read_bytes(path) -> vector<u8>`** (and
  `write_bytes(path, v)`), from the stdlib.  Reach for the `f#read`
  loop only for structured, offset-driven access.

Example binary reader/writer patterns live in
`tests/fixtures/libs/graphics/src/glb.loft` (writer) and
`tests/fixtures/libs/graphics/tests/glb.loft` (reader).
