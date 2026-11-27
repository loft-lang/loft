#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The scope gate: the scope pass written in loft (`selfhost/src/scope.loft`) against the Rust one,
definition by definition, on the image before and after the pass.

For each program the Rust front end writes its image BEFORE the scope pass (`LOFT_EMIT_PARSED`)
and right AFTER it (`LOFT_EMIT_SCOPED`, before the IR passes); `scoped` lays out the first with
the loft pass and reports the program's own definitions, `defs` reports the same definitions of
the second, and the two reports are compared per definition.  A definition is `agree`,
`unscoped` (the loft pass names why it cannot lay it out yet) or `differ`; a program agrees when
all its definitions do, is `unscoped` when what does not agree is only unscoped, and differs
otherwise.  The input is the RUST front end's image, so this measures the scope pass alone,
whatever the elaborator carries.

  scope_survey.py [--part P] [--jobs N] [--show N] [--check B | --write-baseline B] [FILE…]
"""
import argparse, collections, concurrent.futures as cf, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, "selfhost", "front"))
import baseline  # noqa: E402
import elab_gate  # noqa: E402  the corpus, `@ARGS` libraries and tool builds are gate E's

# A generic instance's name holds spaces (`i_21integer(-128, 127)s1?_n_set_from`): the name runs
# to the first ` type=`, so no instance is folded into the definition before it.
HEAD = re.compile(r"^#\d+ (.+?) type=\d+ .*\bsource=(\d+)\b")


def own_definitions(lines):
    """{name: lines} of the definitions the pass lays out: every source but the standard
    library's (source 0, which arrives scoped) — the program's and its libraries'."""
    out, key = {}, None
    for l in lines:
        m = HEAD.match(l)
        if m:
            key = m.group(1) if m.group(2) != "0" else None
            if key in out:
                key = f"{key}#{sum(1 for k in out if k.split('#')[0] == key) + 1}"
        elif re.match(r"^\d+:", l):
            key = None          # the schema rows that follow the definitions
        if key:
            out.setdefault(key, []).append(l)
    return out


def shape(line):
    line = re.sub(r"^\[\d+\] ", "[N] ", line)
    line = re.sub(r"\b\d+(i32|i64|f64|f32|u8|u16|u32)\b", r"N\1", line)
    line = re.sub(r"_\d+\b", "_N", line)
    return re.sub(r"\b[a-z][a-z0-9]*(?=\(\d+\))", "v", line)


def one(loft, defs, scoped, std, path, tmp, env, i):
    """(verdict, {definition: (verdict, first differing pair or reason)})."""
    with open(path, encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    if "@EXPECT_ERROR" in text:
        return "noimage", {}
    libs = elab_gate.lib_dirs(text)
    lib_args = [a for d in libs for a in ("--lib", d)]
    pre, post = os.path.join(tmp, f"p{i}.pre"), os.path.join(tmp, f"p{i}.post")
    try:
        for var, out in (("LOFT_EMIT_PARSED", pre), ("LOFT_EMIT_SCOPED", post)):
            r = subprocess.run([loft] + lib_args + [path], env=dict(env, **{var: out}),
                               capture_output=True, timeout=180)
            if r.returncode != 0 or not os.path.exists(out):
                return "noimage", {}
        m = subprocess.run([scoped, pre, std], env=env, capture_output=True, text=True,
                           errors="replace", timeout=300)
        o = subprocess.run([defs, post, "program", std], env=env, capture_output=True, text=True,
                           errors="replace", timeout=300)
    except subprocess.TimeoutExpired:
        return "differ", {"(program)": ("differ", "timed out")}
    finally:
        for f in (pre, post):
            if os.path.exists(f):
                os.remove(f)
    if m.returncode != 0:
        return "differ", {"(program)": ("differ", f"scoped exited {m.returncode}: {m.stderr.strip()[-200:]}")}
    lines = m.stdout.splitlines()
    # `unscoped: n_name: reason` — the function the loft pass leaves as the parser wrote it.
    unscoped = {}
    for l in lines:
        if l.startswith("unscoped: "):
            name, _, why = l[len("unscoped: "):].partition(": ")
            unscoped[name] = why
    mine = own_definitions([l for l in lines if not l.startswith("unscoped: ")])
    want = own_definitions(o.stdout.splitlines())
    fns = {}
    reuse_refused = next((r for r in unscoped.values() if "pure_reuse" in r), None)
    for name in sorted(set(mine) | set(want)):
        # A repeated name carries a `#N` suffix (`own_definitions`); a name may hold `#` of its
        # own (`t_18Circle#Tri#integer_tag`), so the whole name is looked up first.
        base = name if name in unscoped else re.sub(r"#\d+$", "", name)
        # Definitions the pass makes or renames after its loop, which only Rust's image holds
        # where the loft pass refused what decides them: a drop cascade
        # `prune_empty_cascades` retired (`<name>_retired`) is the refused `<name>`, and a
        # `shared_call::variant` (`<callee>__given_<f>`) is made for a caller `pure_reuse`
        # refused.
        if name not in mine and base.endswith("_retired") and base[: -len("_retired")] in unscoped:
            fns[name] = ("unscoped", unscoped[base[: -len("_retired")]])
        elif name not in mine and "__given_" in base and reuse_refused is not None:
            fns[name] = ("unscoped", "shared_call::variant: made for a caller pure_reuse refused")
        elif base in unscoped:
            fns[name] = ("unscoped", unscoped[base])
        elif mine.get(name) == want.get(name):
            fns[name] = ("agree", "")
        else:
            a, b = want.get(name, []), mine.get(name, [])
            k = next((k for k in range(min(len(a), len(b))) if a[k] != b[k]), min(len(a), len(b)))
            fns[name] = ("differ", (a[k].strip() if k < len(a) else "(end)",
                                    b[k].strip() if k < len(b) else "(end)"))
    vs = {v for v, _ in fns.values()}
    verdict = "differ" if "differ" in vs else "unscoped" if "unscoped" in vs else "agree"
    return verdict, fns


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("files", nargs="*")
    ap.add_argument("--loft", default="target/release/loft")
    ap.add_argument("--part", action="append", choices=elab_gate.PARTS)
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--show", type=int, default=10, help="differing definitions listed")
    ap.add_argument("--check", metavar="BASELINE", help="exit 1 when a definition that agreed no longer does")
    ap.add_argument("--write-baseline", metavar="BASELINE", help="write the `scope` blocks of BASELINE")
    ap.add_argument("--verdicts", metavar="FILE", help="write every definition's verdict and reason, one per line")
    o = ap.parse_args()
    if (o.check or o.write_baseline) and (o.files or o.part):
        sys.exit("--check and --write-baseline read the whole corpus: drop the files and --part")
    loft = o.loft if os.path.isabs(o.loft) else os.path.join(ROOT, o.loft)
    suite = os.path.realpath(os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(ROOT), "loft-suite-91")))
    pin = baseline.require_pin(ROOT, suite, o.check) if (o.check or o.write_baseline) else None
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_TIMEOUT="300")
    if o.files:
        todo = [("args", os.path.realpath(f)) for f in o.files]
    else:
        files = elab_gate.corpus(ROOT, suite)
        todo = [(p, os.path.realpath(f)) for p in (o.part or elab_gate.PARTS) for f in files[p]]
    tmp = tempfile.mkdtemp(prefix="loft-scope-")
    empty = os.path.join(tmp, "empty.loft")
    with open(empty, "w", encoding="utf-8") as fh:
        fh.write("fn main() {}\n")
    std = os.path.join(tmp, "std.store")
    subprocess.run([loft, empty], env=dict(env, LOFT_EMIT_PARSED=std), capture_output=True, timeout=180)
    if not os.path.exists(std):
        sys.exit("the empty program wrote no image")
    # `LOFT_SCOPE_DEFS=<exe>` / `LOFT_SCOPE_BIN=<exe>`: tools built before, for a run between edits.
    sh = os.path.join(ROOT, "selfhost")
    defs = os.environ.get("LOFT_SCOPE_DEFS") or elab_gate.build(loft, os.path.join(sh, "compare", "defs.loft"), os.path.join(tmp, "defs"), env)
    scoped = os.environ.get("LOFT_SCOPE_BIN") or elab_gate.build(loft, os.path.join(sh, "compare", "scoped.loft"), os.path.join(tmp, "scoped"), env)
    progs = {}
    fn_results = {}
    details = {}
    reasons = collections.Counter()
    shapes = collections.defaultdict(list)
    with cf.ThreadPoolExecutor(o.jobs) as ex:
        futs = {ex.submit(one, loft, defs, scoped, std, f, tmp, env, i): f for i, (_, f) in enumerate(todo)}
        for fut in cf.as_completed(futs):
            f = futs[fut]
            verdict, fns = fut.result()
            if verdict == "noimage":
                continue
            name = baseline.name_of(f, ROOT, suite)
            progs[name] = verdict
            for fname, (fv, detail) in fns.items():
                fn_results[f"{name}::{fname}"] = fv
                details[f"{name}::{fname}"] = detail if isinstance(detail, str) else " / ".join(detail)
                if fv == "unscoped":
                    reasons[detail] += 1
                elif fv == "differ":
                    # The shape of a difference: its first differing pair with the literals,
                    # line marks and generated-name counters taken out — but each variable's
                    # SCOPE kept, the number this pass decides — so one cause counts once.
                    key = tuple(shape(s)[:120] for s in detail)
                    shapes[key].append(f"{name}::{fname}")
    pv, fv = collections.Counter(progs.values()), collections.Counter(fn_results.values())
    print(f"{pv['agree']} of {len(progs)} programs agree; {pv['unscoped']} unscoped, {pv['differ']} differ")
    print(f"{fv['agree']} of {len(fn_results)} definitions agree; {fv['unscoped']} unscoped, {fv['differ']} differ")
    if reasons:
        print("unscoped, by the definitions it stops:")
        for r, n in reasons.most_common(10):
            print(f"  {n:6}  {r}")
    if shapes:
        print("differ, by the shape of the first difference (Rust / loft):")
        for k, fs in sorted(shapes.items(), key=lambda kv: -len(kv[1]))[:o.show]:
            print(f"  {len(fs):6}  rust: {k[0]}\n          loft: {k[1]}\n          e.g. {fs[0]}")
    if o.verdicts:
        with open(o.verdicts, "w", encoding="utf-8") as fh:
            for k in sorted(fn_results):
                fh.write(f"{k}\t{fn_results[k]}\t{details[k]}\n")
    agree_fns = {k: "agree" for k, v in fn_results.items() if v == "agree"}
    if o.write_baseline:
        baseline.write(o.write_baseline, pin, "scope", pv["agree"], len(progs),
                       {k: v for k, v in progs.items() if v != "agree"})
        baseline.write(o.write_baseline, pin, "scope-functions", len(agree_fns), len(fn_results), agree_fns)
        sys.exit(0)
    if o.check:
        fb = baseline.read(o.check).get("scope-functions")
        if fb is None:
            sys.exit("the baseline has no `scope-functions` block: write it with --write-baseline")
        lost = sorted(k for k in fb[2] if k in fn_results and fn_results[k] != "agree")
        for k in lost[:40]:
            print(f"FUNCTION DROP\t{k}\t{fn_results[k]}")
        sys.exit(1 if lost else 0)


if __name__ == "__main__":
    main()
