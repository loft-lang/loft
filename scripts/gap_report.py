#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""gap_report.py — where a routine's distance to its Rust twin lives, and what declined to remove it.

    scripts/gap_report.py --bench 17_consumer_moros_dryopea --fn c_slope_path
    scripts/gap_report.py --package ../loft-bench-libs/loft-libs-core/cbor --fn encode_op
    scripts/gap_report.py … --n 3 --top 25 --out report.txt

`--fn` names the function the routine's timed loop calls, the same name in the loft bench and
in its Rust twin (`name=rust_name` where they differ; a loft method by its emitted name,
`t_5Stage_pack_instances=pack_instances`).  Both binaries run under callgrind with
collection toggled on that function (and its `__variant`s) only, so the counts are the
routine's alone, per call.  The toggle is a toggle — a nested entry switches it off — so a
RECURSIVE kernel is refused, and named through the op that calls it (cbor: `encode_op`).

The report has three parts, and the first is the one to read before any fix:

1. THE GAP — loft's instructions per call against the twin's, split three ways: what the loft
   lines compile to themselves, the runtime rustc INLINED into them (by runtime source file,
   with the hottest lines), and the runtime CALLED (stores, vectors, allocation, copies).  A
   fix that removes 7 % of a routine still a factor off its twin is working on the wrong
   part when the gap lives in another row; this says which row.
2. THE FUNCTIONS — every loft function the kernel executes, by its own instructions, beside
   its emitted sites (stores created, element reads through a reference, null tests,
   checked arithmetic, runtime vector calls) — and the loft source LINES those instructions
   came from (the emitted `// loft:` markers, compiled with line tables).
3. THE DECLINES — every rewrite trace line (`LOFT_TRACE_*`, emission only) that names one of
   those functions: each store, copy or check still emitted is a rule that declined it, and
   the reason is the work item.

Needs valgrind.  A report, never a gate.
"""
import argparse
import atexit
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOFT = os.path.join(REPO, "target", "release", "loft")
HAND_PRICE = os.path.join(REPO, "bench", "portal", "hand_price.sh")

# Trace switches that print per RUN or per token rather than per decision: left off.
NOISY_TRACES = {
    "LOFT_TRACE_LEX", "LOFT_TRACE_DB", "LOFT_TRACE_ASSERTS", "LOFT_TRACE_WARM",
    "LOFT_TRACE_PAR_WORKERS", "LOFT_TRACE_SCHEMA", "LOFT_TRACE_ARENA", "LOFT_TRACE_ALIGN",
    "LOFT_TRACE_PREFILL", "LOFT_TRACE_CR", "LOFT_TRACE_KEYS", "LOFT_TRACE_VISIBILITY",
    "LOFT_TRACE_MINT", "LOFT_TRACE_PREAMBLE",
}

# Decline reasons that concern a scalar, a call with no loft body or a shape no store is
# part of: true, and never the cost the report is after.
DECLINE_NOISE = (
    "the callee has no hidden buffer", "the callee has no loft body", "declines: arguments [",
    "no hoisted integer field", "statements, not one", "the body is not an assignment",
)

# Runtime symbol → the category its instructions are charged to.  First match wins.
CATEGORIES = [
    # A package's own `#native` Rust: unmangled, with no crate path.
    ("the library's own #native Rust", r"^n_[a-z]"),
    ("the bridge to it (loft_ffi, native_call, thread-locals)", r"loft_ffi::|loft::extensions::native_call|__tls_get_addr"),
    ("allocation (malloc/free/realloc)", r"\b(malloc|free|realloc|calloc|_int_malloc|_int_free|cfree|__libc_)"),
    ("block copy / fill (memcpy/memset/memmove)", r"(memcpy|memset|memmove|__memcpy|__memset|__memmove)"),
    ("store claim / free / grow", r"loft::store::|Store>::(claim|delete|free|resize|grow|set_free|valid)|database_named|free_named|op_database|OpDatabase|OpFreeRef|copy_claims|remove_claims"),
    ("vector runtime (headers, get, append, slice)", r"loft::vector::|Stores>::(vector_|vec_|push_|append_|element_reference|clear_vector)"),
    ("text runtime", r"loft::(text|ops::text)|Str::|String|str::|fmt::"),
    ("keyed collections (hash, index, sorted)", r"(hash|Hash|sorted|Sorted|index|Index|radix|Radix|trie|Trie)"),
    ("checked arithmetic / conversions (loft::ops)", r"loft::ops::"),
    ("other loft runtime", r"loft::|<loft::"),
]

EMITTED_SITES = [
    ("stores created", r"OpDatabase[A-Za-z]*\(cell"),
    ("element reads through a reference", r"vec_get_or_raise|get_vector\(|vec_get_hoisted"),
    ("vector length reads", r"length_vector\("),
    ("vector appends/copies (runtime)", r"vector_add|vector_slice|append_bytes|append_i64|push_record"),
    ("null tests", r"op_conv_bool_from_int|== i64::MIN|!= i64::MIN"),
    ("checked arithmetic", r"ops::op_(add|min|mul|div|rem)_(int|long)"),
    ("record field reads/writes through the store", r"get_int\(|set_int\(|rec_get|rec_set|store_mut\("),
]


def run(cmd, cwd=None, env=None, check=True):
    r = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
    if check and r.returncode != 0:
        sys.exit(f"gap_report: {' '.join(cmd[:3])}… failed ({r.returncode}):\n{r.stderr[-3000:]}")
    return r


def locate(a):
    """(dir to run loft in, bench.loft path relative to it, bench.rs path, is_package)"""
    if a.package:
        d = os.path.abspath(a.package)
        return d, os.path.join("bench", "bench.loft"), os.path.join(d, "bench", "bench.rs"), True
    name = a.bench
    cands = [x for x in os.listdir(os.path.join(REPO, "bench")) if x == name or x.startswith(f"{name}_")]
    if len(cands) != 1:
        sys.exit(f"gap_report: --bench {name!r} matches {cands}")
    d = os.path.join(REPO, "bench", cands[0])
    return REPO, os.path.join(d, "bench.loft"), os.path.join(d, "bench.rs"), False


def build(a, work):
    cwd, loft_src, rs_src, package = locate(a)
    env = dict(os.environ, LOFT_NATIVE_NO_CACHE="1", LOFT_TIMEOUT="900")
    args = os.path.join(work, "native.args")
    if package:
        run([LOFT, "--native-release", loft_src, "--n", "1"], cwd=cwd,
            env=dict(env, LOFT_NATIVE_RUSTC_ARGS=args))
    emitted = os.path.join(work, "native.rs")
    trace_env = dict(env)
    names = set(re.findall(r'"(LOFT_TRACE_[A-Z0-9_]+)"', run(
        ["grep", "-rhoE", r'"LOFT_TRACE_[A-Z0-9_]+"', os.path.join(REPO, "src")], check=False).stdout))
    for n in names - NOISY_TRACES:
        trace_env[n] = "1"
    r = run([LOFT, "--native-release", "--native-emit", emitted, loft_src], cwd=cwd, env=trace_env)
    traces = r.stderr.splitlines()
    # The kernel stays a function on both sides, so callgrind can switch collection on at its
    # entry: one call boundary per call each, the same on both.
    for src_path, name in ((emitted, a.loft_sym), (rs_src, a.rust_fn)):
        if recursive(src_path, name):
            sys.exit(f"gap_report: `{name}` is recursive, and callgrind stops counting inside a nested "
                     "entry; name the bench function that calls it in the timed loop (cbor: --fn encode_op)")
    keep_call(emitted, a.loft_sym)
    twin_src = os.path.join(work, "twin.rs")
    shutil.copy(rs_src, twin_src)
    keep_call(twin_src, a.rust_fn)
    rs_src = twin_src
    native = os.path.join(work, "native")
    hp = [HAND_PRICE, emitted, native] + ([args] if package else [])
    run(hp, cwd=REPO, env=dict(os.environ, HAND_PRICE_RUSTFLAGS="-C debuginfo=line-tables-only"))
    twin = os.path.join(work, "twin")
    run(["rustc", "-O", "--edition=2021", "-o", twin, rs_src])
    return cwd, emitted, native, twin, traces


def recursive(path, fn):
    """Whether `fn` (or a `__variant` of it) can reach itself through the functions of `path`:
    callgrind's toggle is a TOGGLE, so a nested entry switches collection off and the
    kernel's own recursion would go uncounted."""
    src = open(path, encoding="utf-8").read()
    bodies = {}
    for m in re.finditer(r"\n[ \t]*(?:pub(?:\(crate\))? )?fn ([A-Za-z0-9_]+)\s*[<(]", src):
        nxt = re.search(r"\n[ \t]*(?:pub(?:\(crate\))? )?fn ", src[m.end():])
        bodies[m.group(1)] = src[m.end(): m.end() + nxt.start() if nxt else len(src)]
    family = {f for f in bodies if f == fn or f.startswith(fn + "__")}
    calls = {f: {g for g in bodies if re.search(rf"\b{re.escape(g)}\(", b)} for f, b in bodies.items()}
    seen, todo = set(), [g for f in family for g in calls.get(f, ())]
    while todo:
        g = todo.pop()
        if g in family:
            return True
        if g not in seen:
            seen.add(g)
            todo.extend(calls.get(g, ()))
    return False


def keep_call(path, fn):
    src = open(path, encoding="utf-8").read()
    # The kernel and its specialised variants (`__rg`, `__ap`, …): a site may call either.
    # A twin's kernel may be a method (`st.pack_instances()`), indented inside its `impl`.
    src, n = re.subn(rf"(\n([ \t]*))((?:pub(?:\(crate\))? )?fn {re.escape(fn)}(?:__[a-z0-9_]+)?[<(])",
                     r"\1#[inline(never)]\n\2\3", src)
    if n == 0:
        sys.exit(f"gap_report: no `fn {fn}(` in {path}")
    open(path, "w", encoding="utf-8").write(src)


def callgrind(binary, fn, n, cwd, work, tag):
    out = os.path.join(work, f"cg.{tag}")
    run(["valgrind", "--tool=callgrind", f"--callgrind-out-file={out}",
         "--compress-pos=no", "--compress-strings=no",
         # A generic kernel is named with its instance: `flow_layout_full::<…>`.
         f"--toggle-collect={fn}", f"--toggle-collect={fn}__*", f"--toggle-collect={fn}::<*",
         binary, "--n", str(n)], cwd=cwd)
    ann = run(["callgrind_annotate", "--inclusive=no", "--auto=no", "--threshold=100", out]).stdout
    total, rows = 0, []
    for line in ann.splitlines():
        m = re.match(r"\s*([\d,]+)\s+\([^)]*\)\s+(.*?)\s*(?:\[[^\]]*\])?$", line)
        if not m:
            continue
        count = int(m.group(1).replace(",", ""))
        # `file:function`: the file ends at the first single colon (a path has none, `::` is
        # the function's own).
        f = re.match(r"(.*?[^:]):(?!:)(.*)", m.group(2))
        sym = f.group(2) if f else m.group(2)
        if "PROGRAM TOTALS" in line:
            total = count
            continue
        if sym and not sym.startswith("file:function") and count:
            rows.append((count, sym))
    if not total:
        total = sum(c for c, _ in rows)
    # Calls of the kernel itself, to normalise per call: callgrind names a function once,
    # `cfn=(id) name`, and refers to it by id after that.
    names, calls, target = {}, 0, None
    want = re.compile(fn.replace("*", ".*") + r"(__[a-z0-9_]+|::<.*>)?$")
    for line in open(out, encoding="utf-8", errors="replace"):
        m = re.match(r"c?fn=(?:\((\d+)\))? ?(.*)", line)
        if m:
            name = m.group(2).strip()
            if m.group(1) and name:
                names[m.group(1)] = name
            elif m.group(1):
                name = names.get(m.group(1), "")
            target = name if line.startswith("cfn=") else None
            continue
        if line.startswith("calls=") and target and want.match(target):
            calls += int(line.split()[0][6:])
    return total, rows, calls


def by_loft_line(cg_out, emitted):
    """Instructions charged to each loft source line: what the emitted lines under its
    `// loft:` marker execute themselves, plus the inclusive cost of their calls into the
    RUNTIME (a call into another emitted function is charged to that function's own lines).
    Runtime code the compiler inlined carries the runtime's file and is not charged here."""
    marker = {}
    current = None
    for nr, line in enumerate(open(emitted, encoding="utf-8"), 1):
        m = re.search(r"// loft:(.+?):(\d+)\s*$", line)
        if m:
            current = f"{m.group(1)}:{m.group(2)}"
        elif line.startswith(("fn ", "pub fn ")):
            current = None
        marker[nr] = current
    base = os.path.basename(emitted)
    cost = defaultdict(int)
    own, inlined, inlined_lines = 0, defaultdict(int), defaultdict(int)
    fl_here = here = False
    cur_file = fl_file = ""
    in_emitted = False
    after_call, callee = False, ""
    for line in open(cg_out, encoding="utf-8", errors="replace"):
        if line.startswith("fl="):
            fl_file = cur_file = line[3:].strip()
            fl_here = here = fl_file.endswith(base)
            continue
        if line.startswith(("fi=", "fe=")):
            cur_file = line[3:].strip()
            here = cur_file.endswith(base)
            continue
        if line.startswith("fn="):
            here, cur_file = fl_here, fl_file
            in_emitted = line[3:].strip().startswith("native::")
            continue
        if line.startswith("cfn="):
            callee = line[4:].strip()
            continue
        if line.startswith("calls="):
            after_call = True
            continue
        m = re.match(r"(\d+) (\d+)", line)
        if not m:
            continue
        inclusive, after_call = after_call, False
        n = int(m.group(2))
        if in_emitted and not inclusive:
            if here:
                own += n
            else:
                inlined[runtime_file(cur_file)] += n
                inlined_lines[(cur_file, int(m.group(1)))] += n
        if not here or (inclusive and callee.startswith("native::")):
            continue
        key = marker.get(int(m.group(1)))
        if key:
            cost[key] += n
    return cost, own, inlined, inlined_lines


def runtime_file(path):
    """`src/database/vectors.rs` → `database/vectors.rs`; a libstd or libc path by its crate."""
    m = re.search(r"/src/((?:[a-z_]+/)?[a-z_0-9]+\.rs)$", path)
    if m and "/rustc/" not in path and "/.cargo/" not in path:
        return m.group(1)
    if "/rustc/" in path or "/library/" in path:
        return "libstd (" + os.path.basename(path) + ")"
    return os.path.basename(path) or "unknown (no line info)"


def loft_source_line(key):
    path, _, nr = key.rpartition(":")
    try:
        return open(path, encoding="utf-8").read().splitlines()[int(nr) - 1].strip()
    except (OSError, IndexError, ValueError):
        return ""


_FN_DEF = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:unsafe\s+)?(?:extern\s+\"C\"\s+)?fn\s+([A-Za-z0-9_]+)")
_sources = {}


def enclosing_fn(path, nr):
    """The runtime function an inlined line belongs to — the mechanism it is part of —
    found by scanning back from the line to the nearest `fn`."""
    if path not in _sources:
        try:
            _sources[path] = open(path, encoding="utf-8").read().splitlines()
        except OSError:
            _sources[path] = []
    lines = _sources[path]
    for i in range(min(nr, len(lines)) - 1, -1, -1):
        m = _FN_DEF.match(lines[i])
        if m:
            return m.group(1)
    return "(no line information)" if nr == 0 else "?"


def category(sym):
    for name, pat in CATEGORIES:
        if re.search(pat, sym):
            return name
    return None


def emitted_functions(path):
    src = open(path, encoding="utf-8").read()
    fns = {}
    for m in re.finditer(r"\nfn ((?:n_|t_\d)[A-Za-z0-9_]+)\(", src):
        start = m.start()
        nxt = re.search(r"\nfn ", src[start + 5:])
        end = start + 5 + nxt.start() if nxt else len(src)
        fns[m.group(1)] = src[start:end]
    return fns


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--bench", help="a suite lane: its number or directory name")
    g.add_argument("--package", help="a library package directory with bench/bench.loft and bench.rs")
    ap.add_argument("--fn", required=True, help="the non-recursive function the timed loop calls: name or loft=rust")
    ap.add_argument("--n", type=int, default=2)
    ap.add_argument("--top", type=int, default=20)
    ap.add_argument("--out")
    ap.add_argument("--keep", action="store_true", help="keep the work directory (emitted Rust, callgrind output)")
    a = ap.parse_args()
    if not shutil.which("valgrind"):
        sys.exit("gap_report: valgrind is not installed")
    loft_fn, _, rust_fn = a.fn.partition("=")
    rust_fn = rust_fn or loft_fn
    a.loft_fn, a.rust_fn = loft_fn, rust_fn
    # A loft function is emitted as `n_<name>`, a method under its own name (`t_5Stage_pack`).
    a.loft_sym = loft_fn if re.match(r"t_\d", loft_fn) else f"n_{loft_fn}"
    work = tempfile.mkdtemp(prefix="gap_report_", dir=os.environ.get("TMPDIR"))
    if a.keep:
        print(f"gap_report: work directory {work}", file=sys.stderr)
    else:
        atexit.register(shutil.rmtree, work, True)
    cwd, emitted, native, twin, traces = build(a, work)
    run_dir = cwd if a.package else REPO
    lt, lrows, lcalls = callgrind(native, f"*::{a.loft_sym}", a.n, run_dir, work, "loft")
    rt, rrows, rcalls = callgrind(twin, f"*::{rust_fn}", a.n, run_dir, work, "rust")
    for side, total, name in (("loft", lt, a.loft_sym), ("twin", rt, rust_fn)):
        if not total:
            sys.exit(f"gap_report: the {side} binary never entered `{name}`: a wrapper the compiler "
                     "merged into the function it calls has no symbol left; name that function")
    if lcalls == 0 or rcalls == 0 or lcalls != rcalls:
        lcalls = rcalls = 1
        unit = "per run"
    else:
        unit = "per call"
    lpc, rpc = lt / lcalls, rt / rcalls
    out = []
    p = out.append
    p(f"gap report — {a.package or a.bench}, kernel `{loft_fn}`" + (f" / `{rust_fn}`" if rust_fn != loft_fn else ""))
    p(f"  loft {lpc:,.0f} instructions {unit} · twin {rpc:,.0f} · {lpc / max(rpc, 1):.2f}× · "
      f"gap {lpc - rpc:,.0f} {unit}" + (f" ({lcalls} calls)" if unit == "per call" else ""))
    p("")
    lines, own, inlined, inlined_lines = by_loft_line(os.path.join(work, "cg.loft"), emitted)
    p(f"1. WHERE LOFT'S INSTRUCTIONS GO ({unit})")
    by_cat = defaultdict(int)
    emitted_self = 0
    fn_self = defaultdict(int)
    for c, sym in lrows:
        m = re.match(r"native::([A-Za-z0-9_]+)", sym)
        if m:
            emitted_self += c
            fn_self[m.group(1)] += c
            continue
        by_cat[category(sym) or "libstd / other"] += c
    def row(c, name):
        p(f"  {c / lcalls:12,.0f}  {100 * c / max(lt, 1):5.1f} %  {name}")
    row(own, "the emitted lines' own work (what the loft lines compile to)")
    inl = sum(inlined.values())
    row(inl, "runtime INLINED into the emitted functions, by source file:")
    for f, c in sorted(inlined.items(), key=lambda kv: -kv[1])[:8]:
        p(f"  {c / lcalls:12,.0f}           {f}")
    other = emitted_self - own - inl
    if abs(other) > lt * 0.005:
        row(other, "emitted functions, unattributed (no line info)")
    p("     hottest inlined runtime lines:")
    for (f, nr), c in sorted(inlined_lines.items(), key=lambda kv: -kv[1])[:10]:
        p(f"  {c / lcalls:12,.0f}           {runtime_file(f)}:{nr}  {loft_source_line(f'{f}:{nr}')[:80]}")
    by_fn = defaultdict(int)
    for (f, nr), c in inlined_lines.items():
        by_fn[f"{enclosing_fn(f, nr)}  ({runtime_file(f)})"] += c
    p("     inlined runtime by the function it came from (the mechanism):")
    for name, c in sorted(by_fn.items(), key=lambda kv: -kv[1])[:12]:
        p(f"  {c / lcalls:12,.0f}           {name}")
    p("  runtime CALLED, by kind:")
    for name, c in sorted(by_cat.items(), key=lambda kv: -kv[1]):
        if c:
            row(c, "  " + name)
    p(f"  {rpc:12,.0f}          the twin's whole routine")
    p(f"  → the emitted lines alone are {own / lcalls / max(rpc, 1):.2f}× the twin's whole routine; the runtime "
      f"(inlined and called) holds {100 * (lt - own) / max(lt, 1):.0f} % of loft's instructions.")
    p("")
    p("   top runtime symbols:")
    for c, sym in sorted([r for r in lrows if not r[1].startswith("native::")], reverse=True)[: a.top]:
        p(f"  {c / lcalls:12,.0f}  {sym[:110]}")
    p("")
    p(f"2. THE FUNCTIONS THE KERNEL RUNS (own instructions {unit}, emitted sites)")
    fns = emitted_functions(emitted)
    for f, c in sorted(fn_self.items(), key=lambda kv: -kv[1])[: a.top]:
        body = fns.get(f, "")
        sites = ", ".join(f"{len(re.findall(pat, body))} {name}" for name, pat in EMITTED_SITES
                          if re.search(pat, body))
        p(f"  {c / lcalls:12,.0f}  {f}  — {sites or 'no runtime sites'}")
    p("")
    p(f"2b. THE LOFT LINES THEY CAME FROM (instructions {unit}, the line's own plus its calls')")
    covered = sum(lines.values())
    for key, c in sorted(lines.items(), key=lambda kv: -kv[1])[: a.top]:
        short = f"{os.path.basename(key.rpartition(':')[0])}:{key.rpartition(':')[2]}"
        p(f"  {c / lcalls:12,.0f}  {short:<22} {loft_source_line(key)[:96]}")
    p(f"  ({100 * covered / max(lt, 1):.0f} % of loft's instructions carry a loft line; the runtime the"
      " compiler inlined keeps its own file, part 1)")
    p("")
    p("3. WHAT DECLINED IN THOSE FUNCTIONS (emission traces; each line is a cost a rule left)")
    # A twin (`__ap`) is its base function's code: its declines are the base's.
    base = {}
    for f in fn_self:
        base.setdefault(re.sub(r"__ap$", "", f), 0)
        base[re.sub(r"__ap$", "", f)] += fn_self[f]
    decline = re.compile(r"declin|DECLINED|keeps its store|not a placeable|refus", re.I)
    hits = defaultdict(list)
    for line in traces:
        if not decline.search(line) or any(n in line for n in DECLINE_NOISE) \
                or line.startswith(("sum:", "nest:")):
            continue
        for f in base:
            if re.search(rf"(?<![A-Za-z0-9_]){re.escape(f)}(?![A-Za-z0-9_])", line):
                hits[f].append(re.sub(r"\s+", " ", line.strip()))
                break
    for f, _ in sorted(base.items(), key=lambda kv: -kv[1]):
        if hits.get(f):
            p(f"  {f}  ({base[f] / lcalls:,.0f} own instructions {unit}):")
            for line in dict.fromkeys(hits[f]):
                p(f"    {line[:170]}")
    text = "\n".join(out)
    print(text)
    if a.out:
        with open(a.out, "w", encoding="utf-8") as fh:
            fh.write(text + "\n")


if __name__ == "__main__":
    main()
