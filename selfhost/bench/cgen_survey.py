#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which programs' C units the loft-written generator writes byte for byte as the Rust one.

@PLN91 strand 12, doc/claude/plans/91-self-host/12-c-generator.md § Steps, G-0.  For every
plain program of the coverage census's corpus (`selfhost/cover/census.py`: tests/scripts and
the language suite), the unit `loft --emit-c` writes (the ORACLE: a program's unit, or its
tests' unit where it has no `main`) is compared with a second unit:

  default            the Rust generator's unit written from the program IMAGE
                     (`LOFT_EMIT_FROM_IMAGE=1`): how many programs the image carries whole —
                     the corpus a generator that reads the image can reach
  --cgen             the loft-written generator's unit (selfhost/front/cgen.loft, built here
                     through the C backend): `cgen <image> <out.c> <src/c>` on the image
                     `loft --emit-image` writes
  --generator EXE    the same with a generator built elsewhere: `EXE <image> <out.c> <src/c>`

`--form debug` compares the DEBUG unit instead (`--native-c --native-debug --native-emit`: the
debug tier's own image, a `#line` per statement, no whole-program rewrite), which only a
generator can be asked for.

`--layer shapes` compares a LAYER instead of the unit: what an analysis derived, one line per
function (12-c-generator.md § The layers beside the unit).  The Rust side prints it while it
writes the unit (`LOFT_TRACE_SHAPES=1`), the generator with `<image> <out> --layer shapes`;
every function the Rust side printed must have the same line on the other side.

A program is counted `agree`; `differ` (the first line that differs, both sides); `refused`
(the second side wrote no unit: its last line of standard error, ranked); `nounit` (the oracle
writes none: the C backend refuses the program, which is no program of this corpus).

    python3 selfhost/bench/cgen_survey.py [--generator EXE] [--scripts-only] [--jobs N] [--top 30]
"""
import argparse, collections, concurrent.futures as cf, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "cover"))
import census  # noqa: E402  the corpus and the plain-program filter are the census's own
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "scripts"))
from c_sweep import unreported  # noqa: E402  a store report is masked as the C sweep masks it


# Where the C runtime's headers are: the generator reads them, as the Rust build embeds them.
RUNTIME_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "src", "c")


def proc(cmd, env, timeout, cwd=None):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout, cwd=cwd)
        return p.returncode, p.stderr
    except subprocess.TimeoutExpired:
        return -9, "timeout"


def last(err):
    lines = [l for l in err.strip().splitlines() if l.strip()]
    return (lines[-1] if lines else "no message")[:110]


def first_difference(a, b):
    """The first line two files differ at: `(line number, a's line, b's line)`."""
    with open(a, encoding="utf-8", errors="replace") as fa, open(b, encoding="utf-8", errors="replace") as fb:
        n = 0
        while True:
            n += 1
            la, lb = fa.readline(), fb.readline()
            if la != lb:
                return n, la.rstrip("\n")[:100], lb.rstrip("\n")[:100]
            if not la:
                return n, "", ""


def numbered(line):
    """A body with the names the UNIT numbers by first use — a function-value dispatcher
    `loft_call<n>`, a tuple struct `loft_tup<n>` — renumbered by first appearance in the body:
    a function writer that does not carry every function yet cannot know the unit's numbers,
    and the unit comparison is what checks them."""
    for stem in ("loft_call", "loft_tup"):
        seen = {}
        line = re.sub(stem + r"(\d+)", lambda m: stem + "#" + str(seen.setdefault(m.group(1), len(seen))), line)
    return line


def layer_one(f, loft, generator, tmp, env, timeout, layer):
    """One program's `layer`: each line the Rust generator traced against the generator's."""
    stem = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f))
    want, got, img = stem + ".want.c", stem + ".layer", stem + ".store"
    switch = {"shapes": "LOFT_TRACE_SHAPES", "reach": "LOFT_TRACE_REACH", "functions": "LOFT_TRACE_REACH",
              "counters": "LOFT_TRACE_COUNTERS", "iterations": "LOFT_TRACE_COUNTERS", "nn": "LOFT_TRACE_FACTS", "ranges": "LOFT_TRACE_FACTS", "bodies": "LOFT_TRACE_BODIES", "tables": "LOFT_TRACE_TABLES", "texts": "LOFT_TRACE_TEXTS", "writes": "LOFT_TRACE_WRITES"}[layer]
    try:
        rc, err = proc([loft, "--emit-c", want, f], {**env, switch: "1"}, timeout, cwd=os.path.dirname(f))
        if rc != 0:
            return f, "nounit", last(err)
        rust = {}
        for line in err.splitlines():
            if line.startswith(f"[{layer}]\t"):
                rust[line.split("\t")[1]] = numbered(line) if layer == "bodies" else line
        rc, err = proc([loft, "--emit-image", img, f], env, timeout, cwd=os.path.dirname(f))
        if rc == 0:
            rc, err = proc([generator, img, got, "--layer", layer], env, timeout, cwd=os.path.dirname(f))
        if rc != 0 or not os.path.exists(got):
            return f, "refused", last(err)
        mine = {}
        with open(got, encoding="utf-8", errors="replace") as fh:
            for line in fh.read().splitlines():
                if line.startswith(f"[{layer}]\t"):
                    mine[line.split("\t")[1]] = numbered(line) if layer == "bodies" else line
        if layer == "bodies":
            # A SUBSET comparison: the function writer carries some functions and refuses the
            # rest by name; each body it writes must be the oracle's.
            refused = {}
            with open(got, encoding="utf-8", errors="replace") as fh:
                for line in fh.read().splitlines():
                    if line.startswith("[refused]\t"):
                        cells = line.split("\t")
                        refused[cells[1]] = cells[2] if len(cells) > 2 else ""
            equal = [n for n, line in mine.items() if rust.get(n) == line]
            wrong = sorted(n for n, line in mine.items() if rust.get(n) != line)
            detail = ""
            if wrong:
                n = wrong[0]
                a, b = (rust.get(n) or "(no line)").split("\\n"), mine[n].split("\\n")
                k = next((i for i in range(min(len(a), len(b))) if a[i] != b[i]), min(len(a), len(b)))
                detail = (f"{n} line {k}\n    oracle: {(a[k] if k < len(a) else '(end)')[:170]}"
                          f"\n    other:  {(b[k] if k < len(b) else '(end)')[:170]}")
            return f, "differ" if wrong else "agree", (len(rust), len(equal), len(wrong), refused, detail)
        if layer == "tables":
            # A unit-level layer the generator prints only for a unit it writes whole: a unit
            # it refuses is counted refused, never different.
            if "unit" not in mine:
                with open(got, encoding="utf-8", errors="replace") as fh:
                    why = next((l.split("\t", 2)[-1] for l in fh.read().splitlines() if l.startswith("[refused]\t")), "no line")
                return f, "refused", why[:110]
            if rust.get("unit") != mine["unit"]:
                return f, "differ", f"unit\n    oracle: {rust.get('unit') or '(no line)'}\n    other:  {mine['unit']}"
            return f, "agree", "1"
        if layer == "texts":
            # A FIELD comparison: the generator prints the `key=value` fields it derives so
            # far, and each must be the oracle's; a field it does not print is not compared.
            for name, line in sorted(rust.items()):
                fields = dict(c.split("=", 1) for c in line.split("\t")[2:])
                held = dict(c.split("=", 1) for c in (mine.get(name) or "").split("\t")[2:] if "=" in c)
                if name not in mine:
                    return f, "differ", f"{name}\n    oracle: {line[:160]}\n    other:  (no line)"
                for k, v in held.items():
                    if fields.get(k) != v:
                        return f, "differ", f"{name} field {k}\n    oracle: {fields.get(k)}\n    other:  {v}"
            return f, "agree", str(len(rust))
        for name, line in sorted(rust.items()):
            if mine.get(name) != line:
                return f, "differ", f"{name}\n    oracle: {line[:160]}\n    other:  {(mine.get(name) or '(no line)')[:160]}"
        return f, "agree", str(len(rust))
    finally:
        for p in (want, got, img):
            if os.path.exists(p):
                os.unlink(p)


def one(f, loft, generator, tmp, env, timeout, debug=False):
    stem = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f))
    want, got, img = stem + ".want.c", stem + ".got.c", stem + ".store"
    try:
        tier = ["--native-debug"] if debug else []
        oracle = [loft, "--native-c", "--native-debug", "--native-emit", want, f] if debug else [loft, "--emit-c", want, f]
        rc, err = proc(oracle, env, timeout, cwd=os.path.dirname(f))
        if rc != 0 or not os.path.exists(want):
            return f, "nounit", last(err)
        if generator:
            rc, err = proc([loft] + tier + ["--emit-image", img, f], env, timeout, cwd=os.path.dirname(f))
            if rc == 0:
                rc, err = proc([generator, img, got, RUNTIME_DIR] + (["--debug"] if debug else []), env, timeout,
                               cwd=os.path.dirname(f))
        else:
            rc, err = proc([loft, "--emit-c", got, f], {**env, "LOFT_EMIT_FROM_IMAGE": "1"}, timeout,
                           cwd=os.path.dirname(f))
        if rc != 0 or not os.path.exists(got):
            return f, "refused", re.sub(r"^cgen: not carried: (\w+: )?", "", last(err))
        with open(want, "rb") as a, open(got, "rb") as b:
            if a.read() == b.read():
                return f, "agree", ""
        n, la, lb = first_difference(want, got)
        return f, "differ", f"line {n}\n    oracle: {la}\n    other:  {lb}"
    finally:
        for p in (want, got, img):
            if os.path.exists(p):
                os.unlink(p)


def bench_rows(out):
    """A bench program's verdict lines: each routine row's name and hash (its first and last
    columns), the timings left out — what `bench/stats.py` holds every lane to."""
    rows = []
    for line in out.splitlines():
        cells = line.split("\t")
        if len(cells) >= 3 and cells[0] != "routine":
            rows.append(f"{cells[0]}\t{cells[-1]}")
    return "\n".join(rows)


def run_one(f, loft, generator, tmp, env, timeout, bench=False):
    """U-0: build and RUN `f` through the generator's unit (`LOFT_CGEN`), against the interpreter.

    `pass` — the native run's output and exit are the interpreter's, and the interpreter's run
    passed (exit 0); `fail` — they differ, or both fail; `refused` — the generator wrote no unit;
    `nobuild` — the C compiler refused the unit.  The second field says whether the unit equals
    the Rust generator's under the same switches (`=`) or not (`~`)."""
    stem = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f))
    want, got = stem + ".want.c", stem + ".got.c"
    # A `tests/scripts` program runs from the repository root, as the test suite runs it
    # (`19-files.loft` reads `tests/example`); every other one from its own directory.
    marker = os.sep + os.path.join("tests", "scripts") + os.sep
    cwd = f.split(marker)[0] if marker in f else os.path.dirname(f)

    def run(cmd, e):
        try:
            p = subprocess.run(cmd, capture_output=True, text=True, env=e, timeout=timeout, cwd=cwd)
            return p.returncode, p.stdout, p.stderr
        except subprocess.TimeoutExpired:
            return -9, "", "timeout"
    try:
        gen_env = {**env, "LOFT_CGEN": generator, "LOFT_CGEN_RUNTIME": RUNTIME_DIR}
        rc, _, err = run([loft, "--native-c", "--native-emit", got, f], gen_env)
        if rc != 0 or not os.path.exists(got):
            lines = [l for l in err.splitlines() if l.strip()] or ["no message"]
            return f, "refused", re.sub(r"^.*cgen: not carried: (\w+: )?", "", lines[-1])[:110]
        # The Rust unit from the same site the run compiles from (`--native-c`), which can
        # differ from `--emit-c`'s: the run is compared with what the Rust writer would build.
        run([loft, "--native-c", "--native-emit", want, f], env)
        same = "=" if os.path.exists(want) and open(want, "rb").read() == open(got, "rb").read() else "~"
        tail = ["--n", "2"] if bench else []
        irc, iout, ierr = run([loft, "--interpret", f] + tail, env)
        nrc, nout, nerr = run([loft, "--native-c", f] + tail, gen_env)
        if bench:
            iout, nout = bench_rows(iout), bench_rows(nout)
        # A printed `store_memory()` report names each side's own stores (the interpreter's
        # value stack and constants, its bytecode positions), which no compiled program has: it
        # is masked as `scripts/c_sweep.py` masks it, and a program that reads the report's
        # counts checks them itself.
        iout = unreported(iout.encode()).decode(errors="replace")
        nout = unreported(nout.encode()).decode(errors="replace")
        if nrc != 0 and ("error:" in nerr and ".c:" in nerr):
            return f, "nobuild", same + " " + next((l for l in nerr.splitlines() if "error:" in l), "")[:150]
        if irc == 0 and nrc == 0 and iout == nout:
            return f, "pass", same
        a, b = iout.splitlines(), nout.splitlines()
        k = next((i for i in range(min(len(a), len(b))) if a[i] != b[i]), min(len(a), len(b)))
        return f, "fail", (f"{same} exit {irc}/{nrc} line {k}\n    interpreter: {(a[k] if k < len(a) else '(end)')[:150]}"
                           f"\n    loft unit:   {(b[k] if k < len(b) else '(end)')[:150]}"
                           + (f"\n    stderr: {last(nerr)}" if nrc != 0 else ""))
    finally:
        for p in (want, got):
            if os.path.exists(p):
                os.unlink(p)


BASELINE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cgen_survey.baseline")


def gate(o, key, switches, numbers):
    """`--write-baseline` / `--check`: the committed position of one configuration.

    The baseline holds, per configuration `key` (layer, switches, form, scope), the counts it
    was taken at and the switch list it was taken under.  `--check` fails (exit 1) on any count
    below its baseline, on any `differ`, and on a switch list that is not the baseline's — a
    position is only compared with one taken the same way.  Re-pin (`--write-baseline`) in the
    commit of the change that moves it, saying why."""
    import json
    base = {}
    if os.path.exists(BASELINE):
        with open(BASELINE, encoding="utf-8") as fh:
            base = json.load(fh)
    if o.write_baseline:
        base[key] = {"switches": switches, **numbers}
        with open(BASELINE, "w", encoding="utf-8") as fh:
            json.dump(base, fh, indent=1, sort_keys=True)
            fh.write("\n")
        print(f"baseline {key}: written")
        return 0
    if not o.check:
        return None
    want = base.get(key)
    if want is None:
        print(f"baseline {key}: none committed — take one with --write-baseline")
        return 1
    fails = []
    if want.get("switches") != switches:
        fails.append(f"taken under {want.get('switches')}, run under {switches}")
    for k, v in numbers.items():
        if k in ("differ", "fail", "nobuild"):
            if v > 0:
                fails.append(f"{k} {v}")
        elif k in want and isinstance(v, int) and k in GATED and v < want[k]:
            fails.append(f"{k} {v} < {want[k]}")
    if fails:
        print(f"baseline {key}: DROPPED — " + "; ".join(fails))
        return 1
    print(f"baseline {key}: held")
    return 0


# The counts a drop of which fails `--check` (the ones that only grow as the port does).
GATED = ("agree", "equal", "pass", "rows", "same")


def tests_only(path):
    """A program run as its tests (`c::emit_tests`): no `main`, a `test_*` function, none of the
    annotations `census.plain` leaves out."""
    try:
        src = open(path, encoding="utf-8").read()
    except OSError:
        return False
    return (not census.SKIP.search(src) and re.search(r"^\s*fn main\s*\(", src, re.M) is None
            and re.search(r"^\s*(pub\s+)?fn test_\w*\s*\(\s*\)", src, re.M) is not None)


def bench_files(root):
    """The benches `--bench` runs without `--files`: this tree's `bench/NN_*/bench.loft`, then
    every library bench of the checkouts `make perf-libs` keeps (`$LOFT_PERF_LIBS`, by default
    `loft-bench-libs` beside this repository: `<repo>/<library>/bench/bench.loft`)."""
    import glob
    libs = os.environ.get("LOFT_PERF_LIBS", os.path.join(os.path.dirname(root), "loft-bench-libs"))
    own = sorted(glob.glob(os.path.join(root, "bench", "[0-9]*", "bench.loft")))
    lib = sorted(glob.glob(os.path.join(libs, "*", "*", "bench", "bench.loft")))
    if not lib:
        print(f"no library benches under {libs} (make perf-libs clones them)", file=sys.stderr)
    return [os.path.abspath(f) for f in own + lib]


def config_key(o):
    scope = ("files:" + os.path.basename(o.files)) if o.files else (
        "benches" if o.bench else ("scripts" if o.scripts_only else "corpus"))
    if o.tests and not o.files and not o.bench:
        scope += ":tests"
    if o.chapter:
        scope += ":" + ",".join(sorted(o.chapter))
    what = "run" if o.run else (o.layer or "unit")
    return f"{what}|{'plain' if o.plain else 'default'}|{o.form}|{scope}"


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--loft", default=None, help="the loft binary (default: target/release/loft)")
    ap.add_argument("--generator", default=None, help="a built generator: EXE <image> <out.c> <src/c>")
    ap.add_argument("--cgen", action="store_true", help="build selfhost/front/cgen.loft and use it")
    ap.add_argument("--form", choices=["program", "debug"], default="program",
                    help="which unit: the program's, or the debug tier's with its #line marks")
    ap.add_argument("--layer", choices=["shapes", "reach", "functions", "counters", "iterations", "nn", "ranges", "bodies", "tables", "texts", "writes"], default=None,
                    help="compare what an analysis derived instead of the unit")
    ap.add_argument("--plain", action="store_true",
                    help="both sides under the switches of cgen_w.loft's PLAIN_SWITCHES: the rules "
                         "the loft writer does not carry yet, off")
    ap.add_argument("--assume", default=None, metavar="SWITCH",
                    help="with --plain: leave SWITCH out, so the oracle runs that rule, and tell the "
                         "generator to write as if it carried it (LOFT_CGEN_ASSUME): the bodies that "
                         "differ are the ones the rule changes")
    ap.add_argument("--run", action="store_true",
                    help="U-0: build and run each program through the generator's unit (LOFT_CGEN) "
                         "and compare its output and exit with the interpreter's")
    ap.add_argument("--bench", action="store_true",
                    help="with --run: the programs are benches (bench/*/bench.loft, a library's "
                         "bench/bench.loft; without --files every one of them, see bench_files) — "
                         "run with --n 2, and compare each routine row's hash, not the timings")
    ap.add_argument("--files", default=None, metavar="LIST",
                    help="the programs to survey: a file of paths, one per line (the last tab "
                         "field of a line), instead of the census's corpus")
    ap.add_argument("--write-baseline", action="store_true",
                    help="record this configuration's counts and switch list in cgen_survey.baseline")
    ap.add_argument("--check", action="store_true",
                    help="exit 1 when a count is below this configuration's baseline, on any "
                         "differ, or when the switches are not the baseline's")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--scripts-only", action="store_true", help="tests/scripts only, not the language suite")
    ap.add_argument("--tests", action="store_true",
                    help="the corpus's programs WITHOUT a main instead — a suite cell's tests' unit "
                         "(its `test_*` functions as the roots, `c::test_functions`)")
    ap.add_argument("--chapter", action="append", default=[], help="a suite chapter (repeatable)")
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--show", action="append", default=[],
                    help="list the programs of a verdict (agree, differ, refused, nounit; repeatable)")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft or os.path.join(root, "target", "release", "loft")
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    if o.plain:
        with open(os.path.join(root, "selfhost", "src", "cgen_w.loft"), encoding="utf-8") as fh:
            row = re.search(r"^PLAIN_SWITCHES = \[(.*)\];$", fh.read(), re.M)
        for name in re.findall(r'"(LOFT_NO_[A-Z0-9_]+)"', row.group(1)):
            if name != o.assume:
                env[name] = "1"
        if o.assume:
            env["LOFT_CGEN_ASSUME"] = o.assume
    switches = sorted(k for k, v in env.items() if k.startswith("LOFT_NO_") and v == "1")
    tmp = tempfile.mkdtemp(prefix="loft-cgen-", dir=os.environ.get("TMPDIR"))
    if (o.form == "debug" or o.layer) and not (o.cgen or o.generator):
        sys.exit("--form debug and --layer compare a generator's output: give --cgen or --generator")
    try:
        if o.cgen:
            out = os.path.join(tmp, "cgen")
            os.makedirs(out, exist_ok=True)
            p = subprocess.run([loft, "--native-c", "--ship", "--out", out,
                                "--lib", os.path.join(root, "selfhost", "src"),
                                "--lib", os.path.join(root, "selfhost", "crt", "src"),
                                os.path.join(root, "selfhost", "front", "cgen.loft")],
                               capture_output=True, text=True, env=env, timeout=900)
            o.generator = os.path.join(out, "cgen")
            if not os.path.exists(o.generator):
                sys.exit(f"the generator did not build:\n{p.stderr[-2000:]}")
        files = [os.path.abspath(f) for f in census.corpus(root, suite if os.path.isdir(suite) else None,
                                                           o.chapter, o.scripts_only)
                 if (tests_only(f) if o.tests else census.plain(f))]
        if o.files:
            with open(o.files, encoding="utf-8") as fh:
                files = [os.path.abspath(l.rstrip("\n").split("\t")[-1]) for l in fh if l.strip()]
        elif o.bench:
            files = bench_files(root)
        if o.run:
            if not o.generator:
                sys.exit("--run builds the generator's units: give --cgen or --generator")
            counts = collections.Counter()
            why = collections.Counter()
            same = 0
            shown = []
            with cf.ThreadPoolExecutor(o.jobs) as ex:
                for f, verdict, what in ex.map(lambda f: run_one(f, loft, o.generator, tmp, env, o.timeout, o.bench), files):
                    counts[verdict] += 1
                    if verdict in ("pass", "fail", "nobuild") and what.startswith("="):
                        same += 1
                    if verdict in o.show:
                        shown.append(f"{verdict}\t{os.path.relpath(f, root)}\t{what}")
                    if verdict in ("refused", "nobuild"):
                        why[what.lstrip("=~ ")] += 1
            others = ", ".join(f"{counts[k]} {k}" for k in ("fail", "nobuild", "refused") if counts[k])
            print(f"{counts['pass']} of {len(files)} programs pass through the loft unit; " +
                  (f"{others}; " if others else "") + f"{same} units equal the Rust one")
            for line in sorted(shown):
                print(line)
            if why:
                print("why a program did not build or was not written, by programs:")
                for name, n in why.most_common(o.top):
                    print(f"  {n:5d}  {name}")
            gated = gate(o, config_key(o), switches,
                         {"pass": counts["pass"], "fail": counts["fail"], "nobuild": counts["nobuild"],
                          "refused": counts["refused"], "same": same, "programs": len(files)})
            if gated is not None:
                return gated
            return 0 if counts["pass"] == len(files) else 1
        counts = collections.Counter()
        why = collections.Counter()
        shown = []
        run = ((lambda f: layer_one(f, loft, o.generator, tmp, env, o.timeout, o.layer)) if o.layer else
               (lambda f: one(f, loft, o.generator, tmp, env, o.timeout, o.form == "debug")))
        rows = 0
        bodies = collections.Counter()
        body_why = collections.Counter()
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for f, verdict, what in ex.map(run, files) if o.layer else ex.map(lambda f: one(f, loft, o.generator, tmp, env, o.timeout, o.form == "debug"), files):
                counts[verdict] += 1
                if o.layer == "bodies" and verdict in ("agree", "differ"):
                    total, equal, wrong, refused, detail = what
                    for k, v in (("functions", total), ("equal", equal), ("differ", wrong), ("refused", len(refused))):
                        bodies[k] += v
                    for reason in refused.values():
                        body_why[reason] += 1
                    what = f"{equal} equal, {wrong} differ, {len(refused)} refused" + (f"\n    {detail}" if detail else "")
                elif o.layer and verdict == "agree":
                    rows += int(what)
                if verdict in o.show:
                    shown.append(f"{verdict}\t{os.path.relpath(f, root)}\t{what}")
                if verdict == "refused":
                    why[what] += 1
        units = len(files) - counts["nounit"]
        if o.layer == "bodies":
            print(f"layer bodies: of {bodies['functions']} function bodies the oracle wrote, the function writer "
                  f"wrote {bodies['equal']} equal and {bodies['differ']} different, and refused {bodies['refused']}")
            print("why a body was not written, by functions:")
            for name, n in body_why.most_common(o.top):
                print(f"  {n:6d}  {name}")
        elif o.layer:
            print(f"layer {o.layer}: {rows} function rows compared")
        print(f"{counts['agree']} of {units} units agree; " +
              ", ".join(f"{counts[k]} {k}" for k in ("differ", "refused", "nounit") if counts[k]))
        for line in sorted(shown):
            print(line)
        if why:
            print("why the second unit was not written, by programs:")
            for name, n in why.most_common(o.top):
                print(f"  {n:5d}  {name}")
        numbers = {"agree": counts["agree"], "differ": counts["differ"], "refused": counts["refused"],
                   "units": units}
        if o.layer == "bodies":
            numbers.update({"equal": bodies["equal"], "differ": bodies["differ"] + counts["differ"],
                            "functions": bodies["functions"]})
        elif o.layer:
            numbers["rows"] = rows
        gated = gate(o, config_key(o), switches, numbers)
        if gated is not None:
            return gated
        return 0 if counts["agree"] == units else 1
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
