#!/usr/bin/env python3
"""The emission audit (@PLN157 § V-r) — validate emitted Rust against the rewrite rules.

Reads one `--native-emit` output and checks, per function, the assumptions the native
rewrites make about the loop they sit in (doc/claude/formal/rewrites.md):

  R-State   one holder per vector path per frame — a `let __vh_N = vector::vec_header(&(P),
            …)`, a `let mut __ph_N = vector::push_header(&(P), …)` or a view header `for
            var_d` — and every hoisted read (`get_elem_hoisted`), write
            (`vec_set_hoisted_or_raise_runtime`), push (`push_hoisted`), length
            (`i64::from(H.len)`), scalar (`__vs_N`) and twin argument names a holder that is
            LIVE at that line and bound for that path.  Two live holders for one path is a
            violation whatever the values say.
  R-Refresh a mover that does not refresh a holder — a template append (`stores.append_*`),
            `pre_alloc_vector`, `vector_add`, or a template mint (`OpNewRecord(cell, P`,
            `OpFinishRecord(cell, P`) — on a path with a live holder is a violation: the
            record push (@PLN157 § V-t) is the sanctioned form there
            (`push_record_hoisted` / `push_record_finish`, validated like a push).
  R-Inputs  a twin call (`<fn>__inv(cell, …)`) hands in only live holders.
  R-Base    (twin clause) a twin's `__ib_k` inputs are holders, and a view's shared base
            (`let __vb_N = __ib_k; //@FR-R-Base view base for …`) is derived from a live one.
  R-RecPtr  a record view's address (`let __pa_N: *const u8 = vector::rec_ptr(…)`) is a holder
            for the rest of its block, and every `rec_get`/`rec_set` through one names a live one.
            (base clause) an address taken from a held element base — `if (idx as u64) <
            u64::from(H.len) { B.add(idx * size) }` — names a live header H and a live base B
            of the SAME path.

A runtime read on a held path (`vec_get_or_raise_runtime`, `length_vector`) is reported as a
NOTE — a missed hoist, never a wrong answer.  Textual by design: the emitter spells a path
by one expression text wherever it names it, so string equality is the pairing.  Exit 1 on
any violation; `--quiet` prints only the summary.

Usage: scripts/emission_audit.py <emitted.rs> [--quiet]
"""
import re
import sys

BIND_HEADER = re.compile(r"^\s*let (__vh_\d+) = vector::vec_header\(&\((.*)\), &stores\.allocations\);")
BIND_PUSH = re.compile(r"^\s*let mut (__ph_\d+) = vector::push_header\(&\((.*)\), &stores\.allocations\);")
BIND_VIEW = re.compile(r"^\s*let (__vh_\d+) = .*//@PLN157 § V-n view header for (\S+?)(?:,| |$)")
# R-Base (@PLN157 § V-ak): an element base is derived FROM a live holder of the same path —
# never a second derivation of the path — and only in a loop that grows no store.
BIND_BASE = re.compile(r"^\s*let (__vb_\d+): \*const u8 = vector::vec_base\(&([\w.]+), &stores\.allocations\);")
# R-Base's twin clause: a view inside a twin SHARES the twin's base input (`__ib_k`) — or a
# loop's base — under its own name; the source must be live here.
BIND_VIEW_BASE = re.compile(r"^\s*let (__vb_\d+) = (__[iv]b_\d+); //@FR-R-Base view base for (\S+?)(?:,| |$)")
# R-RecPtr: a record VIEW's address, bound right after the binding and live to the block's
# end; every `rec_get`/`rec_set` through it must name a live one.
# The view's operand may carry spaces: a `&`-link local reads as `unsafe { *var_al }`.
BIND_RECPTR = re.compile(r"^\s*let (__pa_\d+): \*const u8 = vector::rec_ptr\(&\((.+?)\), &stores\.allocations\); //@FR-R-RecPtr record view address for")
# R-RecPtr's base clause: the loop variable of `for e in v` takes its address from the element
# base the loop holds — the header and the base it names must both be live, on one path.
BIND_RECPTR_BASE = re.compile(r"^\s*let (__pa_\d+): \*const u8 = if \(var_\w+ as u64\) < u64::from\(([\w.]+)\.len\) \{ unsafe \{ (__vb_\d+)\.add\(var_\w+ as usize \* \d+(?: \+ \d+)?\) \} \} else \{ std::ptr::null\(\) \}; //@FR-R-RecPtr record view address for (\S+?),")
USE_RECPTR = re.compile(r"vector::rec_(get|set)::<[^>]*>\((__pa_\d+), ")
BIND_SCALAR = re.compile(r"^\s*let (__vs_\d+) = (.*);")
SCALAR_KEY = re.compile(r"let db = \((var_\w+)\);.*db\.pos \+ \((\d+)_i64\)")
FN_HEAD = re.compile(r"^fn (\w+)\((.*)\)")
USE_ELEM = re.compile(r"(get_elem_hoisted|vec_set_hoisted_or_raise_runtime|get_elem_at|vec_set_at)::<[^>]*>\(&([\w.]+), (?:(__vb_\d+), )?&\((.*?)\), \(\d+_i64\) as u32")
USE_PUSH = re.compile(r"push_hoisted::<[^>]*>\(&mut (__ph_\d+), &\((.*?)\), \d+, __pv\)")
GROUP_CLOSE = re.compile(r"^\s*// @FR-R-GroupPush group (__ph_\d+) closed")
USE_PUSHREC = re.compile(r"push_record_(?:hoisted|hoisted_zero|finish)::<[^>]*>\(&mut (__ph_\d+), &\((.*?)\)(?:, \d+)?\)")
MOVER_MINT = re.compile(r"\b(OpNewRecord|OpFinishRecord)\(cell, (var_\w+),")
USE_LEN = re.compile(r"\(i64::from\(([\w.]+)\.len\)\)")
USE_SCALAR = re.compile(r"\b(__vs_\d+)\b")
TWIN_CALL = re.compile(r"\b\w+__inv\(cell,(.*)\)")
MOVER = re.compile(r"(stores\.append_\w+|vector::pre_alloc_vector)\(&\((.*?)\), ")
MOVER_ADD = re.compile(r"let _av_t = (var_\w+); ")
RUNTIME_READ = re.compile(r"(vec_get_or_raise_runtime|vector::length_vector)\(&\((.*?)\), ")
STRING = re.compile(r'"(?:[^"\\]|\\.)*"')


class Holder:
    def __init__(self, name, kind, path, depth, line):
        self.name, self.kind, self.path, self.depth, self.line = name, kind, path, depth, line


def audit(text, quiet=False):
    violations, notes = [], []
    bound_total, uses_total = 0, 0
    fn = None
    depth = 0
    holders = []  # live holders, innermost last
    for nr, raw in enumerate(text.splitlines(), 1):
        line = STRING.sub('""', raw)
        m = FN_HEAD.match(line)
        if m:
            fn = m.group(1)
            depth = 0
            holders = []
            # a twin's inputs are holders for the whole body, path unknown
            # (a base input `__ib_k` is the twin clause of R-Base: a holder like the rest)
            for param in re.findall(r"(__i[shb]_\d+): ", m.group(2)):
                holders.append(Holder(param, "input", None, 1, nr))
        code = line.split("//")[0]
        # R-GroupPush — a group header's Rust binding outlives its group; the emitter marks
        # where the holder dies, and from there it is not a holder of the path.
        mg = GROUP_CLOSE.match(raw)
        if mg:
            holders = [h for h in holders if h.name != mg.group(1)]
            continue

        def live(name):
            base = name[:-2] if name.endswith(".h") else name
            for h in reversed(holders):
                if h.name == base:
                    return h
            return None

        def holder_for(path):
            return [h for h in holders if h.path == path]

        # ---- bindings (R-State: one holder per path per frame) ----
        bound = None
        mh = BIND_HEADER.match(line)
        mp = BIND_PUSH.match(line)
        mv = BIND_VIEW.match(line)
        ms = BIND_SCALAR.match(line)
        mrb = BIND_RECPTR_BASE.match(line)
        if mrb:
            hdr, base = live(mrb.group(2)), live(mrb.group(3))
            if hdr is None or base is None:
                violations.append(f"{fn}:{nr}: R-RecPtr — {mrb.group(1)} takes its address from {mrb.group(2)} / {mrb.group(3)}, which is not live here")
            elif hdr.path != base.path:
                violations.append(f"{fn}:{nr}: R-RecPtr — {mrb.group(1)} pairs header {mrb.group(2)} (`{hdr.path}`) with base {mrb.group(3)} (`{base.path}`)")
            holders.append(Holder(mrb.group(1), "recptr", f"rec:{mrb.group(4)}", depth, nr))
            bound_total += 1
            depth += code.count("{") - code.count("}")
            holders = [h for h in holders if h.depth <= depth]
            continue
        mr = BIND_RECPTR.match(line)
        if mr:
            holders.append(Holder(mr.group(1), "recptr", f"rec:{mr.group(2)}", depth, nr))
            bound_total += 1
            depth += code.count("{") - code.count("}")
            holders = [h for h in holders if h.depth <= depth]
            continue
        mb = BIND_BASE.match(line) or BIND_VIEW_BASE.match(line)
        if mb:
            # A base holds the path of the header it was derived from; that header must be
            # live here (R-Base), and the base is a SECOND holder of the path on purpose —
            # it is the header's address, not another derivation — so R-State does not
            # count it.
            src = live(mb.group(2))
            if src is None:
                violations.append(f"{fn}:{nr}: R-Base — {mb.group(1)} derives a base from {mb.group(2)}, which is not live here")
            # A view's shared base names the view it serves; a twin's base INPUT carries no
            # path of its own, so the view's name is the path there.
            path = src.path if src else None
            if path is None and mb.re is BIND_VIEW_BASE:
                path = mb.group(3)
            holders.append(Holder(mb.group(1), "base", path, depth, nr))
            bound_total += 1
            depth += code.count("{") - code.count("}")
            holders = [h for h in holders if h.depth <= depth]
            continue
        if mp:
            bound = Holder(mp.group(1), "push", mp.group(2), depth, nr)
        elif mv:
            bound = Holder(mv.group(1), "view", mv.group(2), depth, nr)
        elif mh:
            bound = Holder(mh.group(1), "header", mh.group(2), depth, nr)
        elif ms:
            k = SCALAR_KEY.search(ms.group(2))
            bound = Holder(ms.group(1), "scalar", f"{k.group(1)}#{k.group(2)}" if k else None, depth, nr)
        if bound:
            if bound.path is not None:
                for other in holder_for(bound.path):
                    violations.append(f"{fn}:{nr}: R-State — {bound.name} binds path `{bound.path}` already held by {other.name} (line {other.line})")
            holders.append(bound)
            bound_total += 1
        else:
            # ---- uses ----
            uses_total += (len(USE_ELEM.findall(code)) + len(USE_PUSH.findall(code))
                           + len(USE_PUSHREC.findall(code))
                           + len(USE_LEN.findall(code)) + len(USE_SCALAR.findall(code)))
            for kind, ptr in USE_RECPTR.findall(code):
                uses_total += 1
                if live(ptr) is None:
                    violations.append(f"{fn}:{nr}: R-RecPtr — rec_{kind} through {ptr}, which is not live here")
            for kind, name, base, path in USE_ELEM.findall(code):
                if base and live(base) is None:
                    violations.append(f"{fn}:{nr}: R-Base — {kind} through {base}, which is not live here")
                h = live(name)
                if h is None:
                    violations.append(f"{fn}:{nr}: R-State — {kind} names {name}, which is not live here")
                elif h.path is not None and h.path != path:
                    violations.append(f"{fn}:{nr}: R-State — {kind} names {name} (bound for `{h.path}`) on path `{path}`")
            for name, path in USE_PUSH.findall(code):
                h = live(name)
                if h is None or h.kind != "push":
                    violations.append(f"{fn}:{nr}: R-Push — push_hoisted names {name}, which is not a live push header")
                elif h.path != path:
                    violations.append(f"{fn}:{nr}: R-State — push_hoisted names {name} (bound for `{h.path}`) on path `{path}`")
            for name, path in USE_PUSHREC.findall(code):
                h = live(name)
                if h is None or h.kind != "push":
                    violations.append(f"{fn}:{nr}: R-PushRec — a record push names {name}, which is not a live push header")
                elif h.path != path:
                    violations.append(f"{fn}:{nr}: R-State — a record push names {name} (bound for `{h.path}`) on path `{path}`")
            for name in USE_LEN.findall(code):
                if live(name) is None:
                    violations.append(f"{fn}:{nr}: R-State — `.len` of {name}, which is not live here")
            for name in USE_SCALAR.findall(code):
                if live(name) is None:
                    violations.append(f"{fn}:{nr}: R-Scalar — {name} read, which is not live here")
            for args in TWIN_CALL.findall(code):
                for name in re.findall(r"\b(__(?:vs|vh|ph|is|ih)_\d+(?:\.h)?)", args):
                    if live(name) is None:
                        violations.append(f"{fn}:{nr}: R-Inputs — a twin is handed {name}, which is not live here")
            for op, path in MOVER.findall(code):
                held = holder_for(path)
                if held:
                    violations.append(f"{fn}:{nr}: R-Refresh — {op} moves path `{path}` while {held[-1].name} holds it (line {held[-1].line})")
            for op, path in MOVER_MINT.findall(code):
                held = holder_for(path)
                if held:
                    violations.append(f"{fn}:{nr}: R-Refresh — template {op} mints on path `{path}` while {held[-1].name} holds it (line {held[-1].line}); the record push is the sanctioned form")
            for var in MOVER_ADD.findall(code):
                held = holder_for(var)
                if held:
                    violations.append(f"{fn}:{nr}: R-Refresh — vector_add moves `{var}` while {held[-1].name} holds it (line {held[-1].line})")
            for op, path in RUNTIME_READ.findall(code):
                held = holder_for(path)
                if held:
                    notes.append(f"{fn}:{nr}: note — {op} on held path `{path}` ({held[-1].name}): a runtime read the holder could serve")
        # ---- scope: braces after this line, then drop holders bound deeper ----
        depth += code.count("{") - code.count("}")
        holders = [h for h in holders if h.depth <= depth]
    if not quiet:
        for v in violations:
            print("VIOLATION", v)
        for n in notes:
            print(n)
    print(f"emission audit: {len(violations)} violation(s), {len(notes)} note(s); "
          f"{bound_total} holder(s) bound, {uses_total} hoisted use(s) resolved")
    return violations


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    quiet = "--quiet" in argv
    text = open(argv[1], encoding="utf-8").read()
    return 1 if audit(text, quiet) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
