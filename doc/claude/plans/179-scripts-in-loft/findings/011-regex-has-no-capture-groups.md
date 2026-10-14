# The `regex` library has no capture groups, `find_all` or `replace`
axis: clarity
met-by: tests/dump_ignored_tests (strand 3) — `re.match(...).group(1)` and `re.sub` with a callback in the originals
status: fixed
fix: the library's own next increment (its README § roadmap): `replace` / `replace_all`, `find_all -> vector<Match>`, capture groups and named groups — a loft-libs-core change, not the stdlib
ref: @PLN192 — regex 0.5.0 (registry, 2026-10-14): `capture` / `capture_all` with groups, `group`, `named`, `replace_on` with `$1` / `${name}` templates; tests/dump_ignored_tests re-twinned over `capture` the same day (SCOREBOARD.md) — the reason and the function name are the originals' groups, read once
probe: 011.probe.loft
expect: refused
checked: f570a06e
holds: no

`regex` 0.3.3 ships `search`, `matches`, `split_on` and their qualified twins. A Python script that pulls a value OUT of a match — `IGNORE.match(line).group(1)` for the reason text, `re.match(r"fn (\w+)", ...)` for a name, `ESCAPE.sub(lambda m: chr(int(m.group(1), 16)), s)` for an unescape — has to re-find the span by hand after `matches` says there is one, which is the same work twice and the drift the regex was there to prevent.
