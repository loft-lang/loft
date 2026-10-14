# A library's TYPE does not fire its trigger — `c: Command = "…"` needs `use process::*;`
axis: clarity
met-by: scripts/check_bundle_fresh (strand 4d) — the first port over `run`, which had to open with `use process::*;` where PROCESS.md says nothing in the script names the library
status: fixed
fix: the type half of the trigger surface — `triggers.rs` derived `pub struct` / `pub enum` names as type triggers from the start, and nothing read them: the auto-use scan collected only `lib::` references and `.method(` calls, and a trigger-enabled dependency's triggers were looked up as a sibling package, never through the way a `use` line finds it; a type that fires is then imported by name, and the typed-format hook looks `lit` / `hole_*` / `to_text` up in the type's own source
ref: macos-scripting (2026-10-08) — `libscan::scan_type_refs`, the parser's `auto_use_type_map` and `type_method`; guarded by tests/lib_process.rs (`a_type_trigger_loads_the_library_with_nothing_naming_it`, both backends, from another working directory) and the scanner's unit test
probe: 
expect: 
checked: 231a813a9
holds: no

The design (PROCESS.md § Where it lives) rests on `Command` being a TYPE trigger: `c: Command = "git log -n {n}"` loads the library and nothing in the script names it.  The port met the opposite — `Undefined type Command` until the file opened with `use process::*;` — because the trigger surface was derived with its types and resolved with its methods only.  Three gaps closed together: the scan reads type positions (`: T`, `-> T`, `as T`, `<T>`, `T {`, `T.Variant`); a dependency's triggers resolve through `lib_path` when it is not a sibling package (a project `lib/`, a `path =` dependency, `--lib`); and a type imported by name keeps its hook methods, which `def_nr` could not see from the importing source.
