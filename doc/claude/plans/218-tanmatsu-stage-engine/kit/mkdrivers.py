#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Turn each tests/*.loft of a game into a program: its `main` renamed, a new `main` that runs it and
then every `fn test_…()` in order, printing `ok <name>` after each.  c_sweep compares the programs'
output on the interpreter and as C."""
import re, sys, pathlib
src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
out.mkdir(exist_ok=True)
for f in sorted(p for p in src.glob("*.loft") if p.is_file()):
    s = f.read_text()
    tests = re.findall(r"^fn (test_\w+)\(\)", s, re.M)
    if not tests:
        continue
    has_main = re.search(r"^fn main\(\)", s, re.M) is not None
    s = re.sub(r"^fn main\(\)", "fn file_main()", s, flags=re.M)
    calls = (["  file_main();"] if has_main else []) + [f'  {t}();\n  println("ok {t}");' for t in tests]
    s += "\nfn main() {\n" + "\n".join(calls) + "\n}\n"
    (out / ("t_" + f.name)).write_text(s)
    print(f.name, len(tests))
