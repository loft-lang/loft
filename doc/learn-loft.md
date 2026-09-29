<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Learn loft in 30 minutes

A guided tour for first-time loft users.  Read top-to-bottom; every
code block is taken from `examples/` and is verified to run.

**Prerequisites:** the `loft` binary on your `PATH`.  See the
project [README](../README.md) for install instructions.

**Time budget:** ~25-30 minutes including running each block.  If
you only have ten minutes, read sections 1–4 — they cover 80% of
day-to-day loft.

---

## 1. Hello, loft (3 min)

```loft
name = "world"
print("Hello, {name}!\n")
```

Run it:

```sh
loft examples/hello.loft
```

Output:

```
Hello, world!
```

That is the whole file — no `fn main`, no semicolons, no imports.  A `.loft`
file with loose statements is a **script**: they run once, top to bottom.  You
can try the same two lines in the browser [playground](playground.html) without
installing anything.

Once a program grows past a handful of statements you give it functions, and
those *are* typed — that step is covered in §5.  A file with a `fn main` is
compiled exactly as it always was:

```loft
fn main() {
  name = "world";
  print("Hello, {name}!\n");
}
```

Three loft conventions in three lines:

- **No `let`, no `var`, no type annotations on locals.**  You assign
  `name = "world"` and the compiler infers the type.
- **String interpolation in braces.**  `"Hello, {name}!"` substitutes
  the variable.  For literal braces use `{{` / `}}`.
- **No entry-point ceremony.**  Loose statements run as a script; `fn main()`
  is there when you want an explicit entry point, like Rust or C.

---

## 2. Variables and types (3 min)

Loft is statically typed but the compiler does the bookkeeping.  You
get a clear compile error if you mix types — never a runtime crash.

```loft
fn main() {
  age = 30;             // inferred: integer (i64)
  name = "Alice";       // inferred: text
  is_admin = true;      // inferred: boolean
  pi = 3.14159;         // inferred: float (f64)

  // The compiler refuses incompatible reassignments at parse time.
  // Uncommenting the next line yields:
  //   error: cannot assign text to variable of type integer
  // age = "thirty";
  assert(age == 30, "age");
}
```

`assert(condition, message)` is the canonical "did this hold?" probe.
Use it freely — it disappears in release builds with no overhead.

Loft has primitive types: `integer` (i64), `boolean`, `float` (f64),
`single` (f32), `character` (Unicode codepoint), `text` (UTF-8
string), and the narrow integer aliases `i8 / i16 / i32 / u8 / u16 /
u32`.

---

## 3. Functions (3 min)

```loft
fn add(a: integer, b: integer) -> integer {
  a + b               // last expression IS the return value
}

fn greet(name: text, greeting: text = "Hello") -> text {
  "{greeting}, {name}!"
}

fn main() {
  print("{add(2, 3)}\n");           // 5
  print("{greet(\"world\")}\n");    // Hello, world!
  print("{greet(\"Loft\", \"Hi\")}\n");  // Hi, Loft!
}
```

Things to notice:

- **Return type after `->`**, like Rust.  Omit it for void functions.
- **Last expression is the return value** — no `return` keyword
  needed for the trailing value.  Use `return value;` to exit early.
- **Default arguments** with `name: T = default`.  Order matters; all
  optional args must come last.

---

## 4. Control flow (3 min)

```loft
fn classify(n: integer) -> text {
  if n < 0 {
    "negative"
  } else if n == 0 {
    "zero"
  } else if n < 100 {
    "small"
  } else {
    "big"
  }
}

fn main() {
  for n in 1..=15 {
    if n % 15 == 0 {
      print("FizzBuzz\n");
    } else if n % 3 == 0 {
      print("Fizz\n");
    } else if n % 5 == 0 {
      print("Buzz\n");
    } else {
      print("{n}\n");
    }
  }
}
```

(See `examples/fizzbuzz.loft` for the runnable version.)

- `if`/`else` is an expression — chain `else if` for ladders.
- `for x in <range or collection>` walks integers (`1..=15` is
  inclusive, `1..15` exclusive) and any iterable type.
- `while`, `break`, and `continue` work as you'd expect.
- `match` (covered in §8) is the fourth control-flow keyword.

---

## 5. Strings (3 min)

```loft
fn main() {
  greeting = "Hello";
  name = "Loft";
  // Interpolation: brace the expression.
  msg = "{greeting}, {name}!";
  print("{msg}\n");

  // Literal braces: double them up.
  print("{{not interpolation}}\n");

  // Escape sequences: \n \t \\ \" \0
  print("Line one\nLine two\n");

  // Length and slicing.
  print("len = {len(msg)}\n");
}
```

Loft text is **UTF-8 internally**.  `len` returns bytes, not visible
characters — be careful with multi-byte content like emoji.  See
[STDLIB.md § Text](claude/STDLIB.md) for the full string API.

---

## 6. Collections (3 min)

```loft
struct Word {
  text: text not null,
  count: integer,
}

fn main() {
  // Vector: insertion-ordered, integer-indexed.
  fruits = ["apple", "banana", "cherry"];
  for f in fruits {
    print("{f}\n");
  }

  // Hash: O(1) lookup by a key field.
  // Both views share the same records — no copying.
  words: hash<Word[text]> = {};
  for label in ["apple", "banana", "apple"] {
    words += Word { text: label, count: 1 };
  }
  print("Hash lookup of 'apple': {words[\"apple\"].text}\n");
}
```

(See `examples/collections.loft` for the runnable version.)

Collections come in four flavours:

| Type | Lookup | Order | Use when |
|---|---|---|---|
| `vector<T>` | by index | insertion | append-only sequence |
| `sorted<T[key]>` | by key | sorted by key | range queries |
| `hash<T[key]>` | by key | unspecified | O(1) random access |
| `index<T[key]>` | by key | sorted | both range + O(log n) lookup |

Add elements with `+=`.  Iterate with `for x in collection`.

---

## 7. Structs (3 min)

```loft
struct Point {
  x: float,
  y: float,
}

fn distance(self: Point, other: Point) -> float {
  dx = self.x - other.x;
  dy = self.y - other.y;
  sqrt(dx * dx + dy * dy)
}

fn main() {
  origin = Point { x: 0.0, y: 0.0 };
  target = Point { x: 3.0, y: 4.0 };
  // Method syntax: `origin.distance(target)` is the same as
  // `distance(origin, target)`.
  print("d = {origin.distance(target)}\n");  // d = 5
}
```

(See `examples/structs.loft` for the runnable version.)

- **Define a struct** with `struct Name { field: type, ... }`.
- **Construct** with `Name { field: value, ... }`.
- **Methods** are functions whose first parameter is named `self`.
  Call them with dot syntax: `obj.method(args)`.
- **No inheritance.**  Compose with other structs or use enums (§8).

---

## 8. Enums (3 min)

```loft
enum Shape {
  Circle { radius: float },
  Rect   { width: float, height: float },
  Square { side: float },
}

fn area(self: Shape) -> float {
  match self {
    Circle { radius } => 3.14159 * radius * radius,
    Rect   { width, height } => width * height,
    Square { side } => side * side,
  }
}

fn main() {
  shapes = [
    Circle { radius: 1.0 },
    Square { side: 4.0 },
  ];
  for s in shapes {
    print("area = {s.area()}\n");
  }
}
```

(See `examples/match.loft` for the runnable version.)

Loft enums come in two shapes:

- **Plain enums** with bare variant names: `enum Direction { North,
  South, East, West }`.
- **Struct-enums** where each variant carries its own fields (above).

`match` is exhaustive — leave a variant out and the compiler tells
you which one.

---

## 9. Files and JSON (3 min)

```loft
fn main() {
  path = "/tmp/loft_walkthrough.txt";
  // Open + write (creates the file if absent).
  f = file(path);
  f.write("first line\nsecond line\n");

  // Read it back.
  contents = file(path).content();
  print("read {len(contents)} bytes:\n{contents}");
}
```

(See `examples/files.loft` for a fuller version with line counting.)

For JSON, loft has a built-in `json` library:

```loft
use json;

fn main() {
  // Parse JSON into a Value tree, walk by field name.
  v = json.parse("{\"name\": \"loft\", \"version\": \"0.8.5\"}");
  print("name = {v[\"name\"].text()}\n");
}
```

See [STDLIB.md § File I/O](claude/STDLIB.md) and
[STDLIB.md § JSON](claude/STDLIB.md) for the full API.

---

## 10. Where next (2 min)

You've seen the language surface.  Pick one of:

- **Read the standard library reference** —
  [`doc/claude/STDLIB.md`](claude/STDLIB.md) lists every built-in
  function with examples.  Good when you're writing real code and
  need "how do I split a string?" or "how do I sort a vector?"
- **Browse more examples** — `examples/` has runnable demos for
  every concept above.  Variations and extensions live in
  `tests/docs/`, which doubles as a tested doc tour.
- **Pull in a library** — `loft install graphics` (or `imaging`,
  `input`, `server`, `markdown`) vendors a package — itself written
  in loft — into your project.  The
  [graphics gallery](https://loft-lang.org/loft/gallery.html) shows
  what the rendering library can do.
- **Try the parallel surface** — loft makes parallelism a one-line
  addition: `for s in items par(r = work(s), 4) { ... }` runs the
  worker on 4 threads.  See [STDLIB_RUNTIME.md § Parallel] and
  `examples/structs.loft` for a starter.
- **Set up your editor** — see
  [editors/vscode/](../editors/vscode) for the VS Code extension
  with syntax highlighting + snippets.  TextMate grammar at
  [syntaxes/loft.tmLanguage.json](../syntaxes/loft.tmLanguage.json)
  works in Sublime Text and any TextMate-compatible editor.
- **Step through native code** — `loft --native --native-debug
  yourprog.loft` produces a debug binary that GDB / LLDB can step
  through.  Variable names are rust-internal (`var_x`); proper
  source mapping is planned for 0.8.6.
- **Ask questions** — open an issue at
  <https://github.com/loft-lang/loft/issues>.

You're done.  Welcome to loft.
