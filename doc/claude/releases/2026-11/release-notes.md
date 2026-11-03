loft 2026.11.0 is the C backend release.

**Compile through C.** `loft --native-c prog.loft` writes your program as C99, builds it with the C compiler you already have, and runs it. It works on Linux, on macOS with Apple's clang, and on Windows with MSVC or MinGW. It answers exactly what the interpreter answers; a program it cannot handle yet is refused by name, never compiled into something that answers differently. And it is fast: across 126 measured routines, each checked against a hand-written, optimised Rust version giving the same result, the typical routine takes 1.42 times as long as the Rust one with gcc (1.55 with clang), and the slowest takes 3.1 times as long. A running game built with `engine_host` can also rebuild itself and swap in the new build on the C backend.

**Ship one file.** `loft --ship prog.loft` builds the binary you hand to someone, with its native libraries linked in. It replaces `--native-release`, and works the same way for wasm, the browser and Android.

**Safer by default.** A field is now private to the file that declares it. A mapped file is read-only. A store image is read only beside the schema that describes it. A `match` guard can no longer change the value being matched. Some of these may need a small change in your code; the changelog says what to write instead.

**Smaller things you may enjoy.** A struct can hold its own type (`left: Tree?`). `d = Unit { hp: 0, ..u }` copies a record with a few fields changed. Your own types can use `<` and `>`. `exit(2)` ends a program with its own exit code. A write that lands nowhere can say so with `else`. The Linux download can now load libraries with native code.

And more than 300 bugs fixed this cycle; the changelog describes the fifty-odd you are most likely to notice, most of them in `match`, generators and stored data.

The whole list, and what to change in your code: [CHANGELOG.md](https://github.com/loft-lang/loft/blob/v2026.11.0/CHANGELOG.md#2026-11).

**Downloads:** pick the `.zip` for your platform below, or on Linux and macOS run `curl -fsSL https://raw.githubusercontent.com/loft-lang/loft/main/scripts/install.sh | sh`. Each `.zip` has a `.sha256` beside it. This is the last release with an Intel Mac download.
