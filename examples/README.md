# Examples

Small, self-contained loft programs. Each one runs from a clean checkout with no
setup, and each has a twin in the browser playground if you would rather not
install anything.

## Looking for the games?

loft's pitch is small browser-playable games, and these examples are deliberately
not that — they are the language basics, kept short. The games live here:

- **[Play Brick Buster](https://loft-lang.org/loft/brick-buster.html)** — a complete
  arcade game: paddle, ball, powerups, music, levels. Press Space to start.
- **[Read its source](../tools/brick-buster/25-brick-buster.loft)** — the whole game
  is that one file. It is the most useful thing in this repo to read if you want to
  see what a real loft program looks like.

## The examples

| File | What it shows | In the browser |
|---|---|---|
| [`hello.loft`](hello.loft) | The smallest program — a script with no `fn main` | [run](https://loft-lang.org/loft/playground.html?example=ex_hello) |
| [`fizzbuzz.loft`](fizzbuzz.loft) | Loops, conditionals, string interpolation | [run](https://loft-lang.org/loft/playground.html?example=ex_fizzbuzz) |
| [`fibonacci.loft`](fibonacci.loft) | Functions, recursion, `assert` | [run](https://loft-lang.org/loft/playground.html?example=ex_fibonacci) |
| [`structs.loft`](structs.loft) | Records and field access | [run](https://loft-lang.org/loft/playground.html?example=ex_structs) |
| [`collections.loft`](collections.loft) | `vector<T>` and `hash<T[key]>` over the same records | [run](https://loft-lang.org/loft/playground.html?example=ex_collections) |
| [`match.loft`](match.loft) | Pattern matching, including destructuring | [run](https://loft-lang.org/loft/playground.html?example=ex_match) |
| [`files.loft`](files.loft) | Reading and writing files | [run](https://loft-lang.org/loft/playground.html?example=ex_files) |
| [`front_end.loft`](front_end.loft) | A loft front end in loft: a lexer over a character cursor, a recursive-descent parser building an arena AST, total matches over tokens and nodes, constant folding, a dispatch table of `fn` values — what a compiler written in loft leans on | — |

## Running them

```sh
loft examples/hello.loft            # compiles via rustc when available
loft --interpret examples/hello.loft   # always available, no toolchain needed
```

A downloaded release runs interpreted — it ships no native runtime, and that is
the normal path, not a fallback you need to fix.

## Where to go next

- **[The tutorial](https://loft-lang.org/loft/)** — the language, chapter by chapter.
- **[The playground](https://loft-lang.org/loft/playground.html)** — type loft, press
  run. That *is* the whole tutorial, if you prefer to poke at it.
