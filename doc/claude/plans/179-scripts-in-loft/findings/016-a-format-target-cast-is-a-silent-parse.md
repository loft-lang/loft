# `"…" as Command` is a struct PARSE, not the format hook — and answers an empty command
axis: behaviour
met-by: lib/process (strand 4b) — the first run probe wrote `("printf %s|{w}|" as Command).run()`
status: open
fix: a type that opts into typed format strings (declares `lit`) refuses `text as T` with a diagnostic naming the two spellings that build it — `c: T = "…"` and an argument typed `T` — or routes the cast through the hook; either way a failed parse must not answer a default record nobody asked for
ref: 
probe: 
expect: 
checked: 
holds: 

`c: Command = "git log -n {n}"` goes through the PLN124 hook: literals reach `lit`, values reach `hole_*`. The cast spelling `("git log -n {n}" as Command)` does not: the string is first built as plain TEXT — the hole rendered into it, which is the erasure the hook exists to prevent — and then `text as Struct` PARSES it as a `Command` record literal (LOFT.md § conversions, `raw as Program`). The parse fails, its errors go to `record#errors` where nothing reads them, and the result is a `Command` with every field defaulted: `run()` answered "an empty command: there is no program to run". Nothing at compile time says the cast took the other road. In a script the cast is the natural way to build a value inline, so the safe spelling is the one an author is least likely to reach for.
