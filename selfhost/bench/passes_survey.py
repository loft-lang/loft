#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The passes gate: the IR passes `close_program` runs, written in loft (`selfhost/src/ir_passes.loft`),
against the Rust ones, definition by definition, on the image before and after them.

For each program the Rust front end writes its image right AFTER the scope pass
(`LOFT_EMIT_SCOPED`) and after `close_program` (`--emit-image`); `passed` runs the loft passes over
the first and reports the program's own definitions, `defs` reports the same definitions of the
second, and the reports are compared per definition, as `scope_survey.py` compares the scope pass.
A definition no pass changes agrees whatever the loft side carries; one a pass changes agrees once
that pass is transcribed.  The input is the RUST scope pass's image, so this measures the passes
alone.

  passes_survey.py [--part P] [--jobs N] [--show N] [FILE…]
"""
import argparse, atexit, collections, concurrent.futures as cf, os, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, "selfhost", "front"))
import baseline  # noqa: E402
import elab_gate  # noqa: E402
from scope_survey import own_definitions, shape  # noqa: E402


def one(loft, defs, passed, std, path, tmp, env, i):
    """(verdict, {definition: (verdict, first differing pair or reason)})."""
    with open(path, encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    if "@EXPECT_ERROR" in text:
        return "noimage", {}
    lib_args = [a for d in elab_gate.lib_dirs(text) for a in ("--lib", d)]
    pre, post = os.path.join(tmp, f"p{i}.pre"), os.path.join(tmp, f"p{i}.post")
    try:
        # The scoped image is taken under `--emit-image` too (it exits after writing it, before
        # the passes): a plain run parses some programs otherwise (the parser's `synth text
        # dest` around a library's text-returning call), so its scoped image was not the input
        # the closed image's `close_program` had.
        r = subprocess.run([loft] + lib_args + ["--emit-image", post, path],
                           env=dict(env, LOFT_EMIT_SCOPED=pre), capture_output=True, timeout=180)
        q = subprocess.run([loft] + lib_args + ["--emit-image", post, path], env=env,
                           capture_output=True, timeout=180)
        if r.returncode != 0 or q.returncode != 0 or not os.path.exists(pre) or not os.path.exists(post):
            return "noimage", {}
        # The closed image hands `passed` the host's `Definition::native` marks (set after the
        # scope pass, before `close_program`): see `selfhost/compare/passed.loft`.
        m = subprocess.run([passed, pre, std, post], env=env, capture_output=True, text=True,
                           errors="replace", timeout=300)
        o = subprocess.run([defs, post, "program", std], env=env, capture_output=True, text=True,
                           errors="replace", timeout=300)
    except subprocess.TimeoutExpired:
        return "differ", {"(program)": ("differ", "timed out")}
    finally:
        for f in (pre, post):
            for g in (f, f + ".dschema"):
                if os.path.exists(g):
                    os.remove(g)
    if m.returncode != 0:
        return "differ", {"(program)": ("differ", f"passed exited {m.returncode}: {m.stderr.strip()[-200:]}")}
    lines = m.stdout.splitlines()
    refused, prefill = {}, {}
    for l in lines:
        if l.startswith("unscoped: "):
            name, _, why = l[len("unscoped: "):].partition(": ")
            refused[name] = why
        elif l.startswith("fact: "):
            # A stored fact the report does not print (`no_prefill`, a closure's field
            # layout), differing from Rust's.
            name, _, why = l[len("fact: "):].partition(": ")
            prefill[name] = why
    mine = own_definitions([l for l in lines if not l.startswith(("unscoped: ", "fact: "))])
    want = own_definitions(o.stdout.splitlines())
    fns = {}
    for name in sorted(set(mine) | set(want)):
        if name in refused:
            fns[name] = ("refused", refused[name])
        elif name in prefill and mine.get(name) == want.get(name):
            fns[name] = ("differ", (prefill[name], ""))
        elif mine.get(name) == want.get(name):
            fns[name] = ("agree", "")
        else:
            a, b = want.get(name, []), mine.get(name, [])
            k = next((k for k in range(min(len(a), len(b))) if a[k] != b[k]), min(len(a), len(b)))
            fns[name] = ("differ", (a[k].strip() if k < len(a) else "(end)",
                                    b[k].strip() if k < len(b) else "(end)"))
    vs = {v for v, _ in fns.values()}
    return ("differ" if "differ" in vs else "refused" if "refused" in vs else "agree"), fns


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("files", nargs="*")
    ap.add_argument("--loft", default="target/release/loft")
    ap.add_argument("--part", action="append", choices=elab_gate.PARTS)
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--show", type=int, default=10, help="difference shapes listed")
    ap.add_argument("--verdicts", metavar="FILE", help="write `verdict<TAB>program::definition` rows")
    o = ap.parse_args()
    loft = o.loft if os.path.isabs(o.loft) else os.path.join(ROOT, o.loft)
    suite = os.path.realpath(os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(ROOT), "loft-suite-91")))
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_TIMEOUT="300")
    if o.files:
        todo = [os.path.realpath(f) for f in o.files]
    else:
        files = elab_gate.corpus(ROOT, suite)
        todo = [os.path.realpath(f) for p in (o.part or elab_gate.PARTS) for f in files[p]]
    tmp = tempfile.mkdtemp(prefix="loft-passes-", dir=os.environ.get("TMPDIR"))
    # Every run leaves its programs, images and built tools here; gone at exit unless
    # LOFT_KEEP_TMP=1 asks to keep them for a look.
    if not os.environ.get("LOFT_KEEP_TMP"):
        atexit.register(shutil.rmtree, tmp, True)
    empty = os.path.join(tmp, "empty.loft")
    with open(empty, "w", encoding="utf-8") as fh:
        fh.write("fn main() {}\n")
    std = os.path.join(tmp, "std.store")
    subprocess.run([loft, empty], env=dict(env, LOFT_EMIT_PARSED=std), capture_output=True, timeout=180)
    if not os.path.exists(std):
        sys.exit("the empty program wrote no image")
    # `LOFT_SCOPE_DEFS=<exe>` / `LOFT_PASSES_BIN=<exe>`: tools built before, for a run between edits.
    sh = os.path.join(ROOT, "selfhost")
    defs = os.environ.get("LOFT_SCOPE_DEFS") or elab_gate.build(loft, os.path.join(sh, "compare", "defs.loft"), os.path.join(tmp, "defs"), env)
    passed = os.environ.get("LOFT_PASSES_BIN") or elab_gate.build(loft, os.path.join(sh, "compare", "passed.loft"), os.path.join(tmp, "passed"), env)
    progs, fn_results = {}, {}
    reasons = collections.Counter()
    shapes = collections.defaultdict(list)
    with cf.ThreadPoolExecutor(o.jobs) as ex:
        futs = {ex.submit(one, loft, defs, passed, std, f, tmp, env, i): f for i, f in enumerate(todo)}
        for fut in cf.as_completed(futs):
            f = futs[fut]
            verdict, fns = fut.result()
            if verdict == "noimage":
                continue
            name = baseline.name_of(f, ROOT, suite)
            progs[name] = verdict
            for fname, (fv, detail) in fns.items():
                fn_results[f"{name}::{fname}"] = fv
                if fv == "refused":
                    reasons[detail] += 1
                elif fv == "differ":
                    shapes[tuple(shape(s)[:120] for s in detail)].append(f"{name}::{fname}")
    pv, fv = collections.Counter(progs.values()), collections.Counter(fn_results.values())
    print(f"{pv['agree']} of {len(progs)} programs agree; {pv['refused']} refused, {pv['differ']} differ")
    print(f"{fv['agree']} of {len(fn_results)} definitions agree; {fv['refused']} refused, {fv['differ']} differ")
    if reasons:
        print("refused, by the definitions it stops:")
        for r, n in reasons.most_common(10):
            print(f"  {n:6}  {r}")
    if shapes:
        print("differ, by the shape of the first difference (Rust / loft):")
        for k, fs in sorted(shapes.items(), key=lambda kv: -len(kv[1]))[:o.show]:
            print(f"  {len(fs):6}  rust: {k[0]}\n          loft: {k[1]}\n          e.g. {fs[0]}")
    if o.verdicts:
        with open(o.verdicts, "w", encoding="utf-8") as fh:
            for k in sorted(fn_results):
                fh.write(f"{fn_results[k]}\t{k}\n")


if __name__ == "__main__":
    main()
