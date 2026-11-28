#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The loft-in-loft list: the programs the loft-written toolchain carries, held as a gate.

A program on the list (`loft_in_loft.list` beside this file) must pass both halves of the
loft-written toolchain (@PLN91, doc/claude/plans/91-self-host/scope-pass-port.md):

  interpreter  the whole loft path — the elaborator, the scope pass, the IR passes, the
               bytecode writer and the runner, all loft (`run_front.loft`, the `front` layer of
               `run_image_survey.py`) — prints what `loft --interpret` prints and leaves no
               store the interpreter does not;
  compiler     the C generator in loft (`selfhost/front/cgen.loft`) writes the unit the Rust
               writer writes, byte for byte, and that unit builds and runs as the interpreter
               does (`cgen_survey.py --run`).

  lil_check.py [--jobs N]                       check every listed program: exit 1 on any that
                                                no longer passes both, naming the half it fails
  lil_check.py --discover [--write] [--jobs N]  run the whole corpus: the programs that pass both
                                                and are not listed, and listed ones that fail;
                                                --write re-pins the list to what passes

The list only grows: a program leaves it in the commit whose change makes it fail, with the
reason in the message, as every baseline here moves.  `LOFT_LIL_RUNNER=<exe>` and
`LOFT_LIL_CGEN=<exe>` take tools built before (the builds are minutes each).
"""
import argparse, atexit, concurrent.futures as cf, os, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, "selfhost", "cover"))
import baseline  # noqa: E402
import census  # noqa: E402
import cgen_survey  # noqa: E402
import run_image_survey  # noqa: E402

LIST = os.path.join(HERE, "loft_in_loft.list")


def build(loft, src, out, env, ship):
    os.makedirs(out, exist_ok=True)
    p = subprocess.run([loft, "--native-c", *(["--ship"] if ship else []), "--out", out,
                        "--lib", os.path.join(ROOT, "selfhost", "src"),
                        "--lib", os.path.join(ROOT, "selfhost", "crt", "src"), src],
                       capture_output=True, text=True, env=env, timeout=3600)
    exe = os.path.join(out, os.path.splitext(os.path.basename(src))[0])
    if not os.path.exists(exe):
        sys.exit(f"{os.path.basename(src)} did not build:\n{p.stderr[-2000:]}")
    return exe


def path_of(name, suite):
    """The file a listed name names: `loft-suite/…` under the suite checkout, the rest under the tree."""
    if name.startswith("loft-suite/"):
        return os.path.join(suite, name[len("loft-suite/"):])
    return os.path.join(ROOT, name)


def read_list():
    if not os.path.exists(LIST):
        return []
    with open(LIST, encoding="utf-8") as fh:
        return [l.strip() for l in fh if l.strip() and not l.startswith("#")]


def write_list(names):
    with open(LIST, "w", encoding="utf-8") as fh:
        fh.write("# The programs the loft-written toolchain carries: the whole loft interpreter path runs\n"
                 "# each as `loft --interpret` does, and the C generator in loft writes the Rust writer's\n"
                 "# unit for it, which runs alike.  Checked by selfhost/bench/lil_check.py; re-pinned by\n"
                 "# `lil_check.py --discover --write` in the commit that moves it.\n")
        for n in sorted(names):
            fh.write(n + "\n")


def check(f, loft, runner, cgen, std, tmp, env, timeout):
    """(interpreter verdict, compiler verdict): `agree`/`pass` when the half holds, else what it read."""
    front_env = {**env, "LOFT_NO_TEXT_BORROW": "1", "LOFT_SCOPE_PORT": "1"}
    _, fv, what = run_image_survey.one(f, loft, runner, tmp, front_env, timeout, front_std=std)
    front = "agree" if fv == "agree" else f"{fv} {what[0] if what else ''}".strip()
    _, cv, cwhat = cgen_survey.run_one(f, loft, cgen, tmp, env, timeout)
    comp = "pass" if cv == "pass" and cwhat.startswith("=") else (
        "pass, unit differs" if cv == "pass" else f"{cv} {cwhat.splitlines()[0] if cwhat else ''}".strip())
    return front, comp


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--timeout", type=int, default=300)
    ap.add_argument("--discover", action="store_true", help="run the whole corpus, not only the list")
    ap.add_argument("--write", action="store_true", help="with --discover: re-pin the list")
    o = ap.parse_args()
    suite = os.path.realpath(os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(ROOT), "loft-suite")))
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    tmp = tempfile.mkdtemp(prefix="loft-lil-", dir=os.environ.get("TMPDIR"))
    # Every run leaves its programs, images and built tools here; gone at exit unless
    # LOFT_KEEP_TMP=1 asks to keep them for a look.
    if not os.environ.get("LOFT_KEEP_TMP"):
        atexit.register(shutil.rmtree, tmp, True)
    # The interpreter path holds the elaborator, which an optimised build compiles past a CI
    # runner's memory (`elab-gate.yml`); the C generator builds optimised, as its survey does.
    runner = os.environ.get("LOFT_LIL_RUNNER") or build(
        o.loft, os.path.join(HERE, "run_front.loft"), os.path.join(tmp, "runner"), env, False)
    cgen = os.environ.get("LOFT_LIL_CGEN") or build(
        o.loft, os.path.join(ROOT, "selfhost", "front", "cgen.loft"), os.path.join(tmp, "cgen"), env, True)
    empty = os.path.join(tmp, "empty.loft")
    with open(empty, "w", encoding="utf-8") as fh:
        fh.write("fn main() {}\n")
    std = os.path.join(tmp, "std.store")
    subprocess.run([o.loft, empty], env={**env, "LOFT_NO_TEXT_BORROW": "1", "LOFT_EMIT_PARSED": std},
                   capture_output=True, timeout=300)
    listed = read_list()
    if o.discover:
        names = [baseline.name_of(os.path.abspath(f), ROOT, suite)
                 for f in census.corpus(ROOT, suite if os.path.isdir(suite) else None, [], False)
                 if os.path.isfile(f) and census.plain(f)]
    else:
        if not listed:
            sys.exit(f"{os.path.relpath(LIST, ROOT)} lists no program: take it with --discover --write")
        names = listed
    results = {}
    with cf.ThreadPoolExecutor(o.jobs) as ex:
        futs = {ex.submit(check, os.path.abspath(path_of(n, suite)), o.loft, runner, cgen, std, tmp, env,
                          o.timeout): n for n in names}
        for fut in cf.as_completed(futs):
            results[futs[fut]] = fut.result()
    passing = {n for n, (fr, co) in results.items() if fr == "agree" and co == "pass"}
    failing = sorted(n for n in listed if n in results and n not in passing)
    for n in failing:
        fr, co = results[n]
        print(f"FAIL\t{n}\tinterpreter: {fr}\tcompiler: {co}")
    if o.discover:
        new = sorted(passing - set(listed))
        for n in new:
            print(f"new\t{n}")
        print(f"{len(passing)} of {len(names)} programs pass both halves; {len(new)} not listed, "
              f"{len(failing)} listed that fail")
        if o.write:
            write_list(passing | (set(listed) - set(results)))
            print(f"{os.path.relpath(LIST, ROOT)}: {len(passing)} programs")
    else:
        print(f"{len(listed) - len(failing)} of {len(listed)} listed programs pass both halves")
    return 1 if failing else 0


if __name__ == "__main__":
    sys.exit(main())
