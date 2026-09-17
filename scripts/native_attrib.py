#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Charge every perf sample of a `--native-release` loft program to the loft LINE that ran it.

    scripts/native_attrib.py <binary> <emitted.rs> <perf.data>

Answers the question a self-time profile of a release binary cannot: WHICH loft line drives
the runtime's share.  Each physical frame is expanded to its inline chain, so a runtime
routine inlined into a program function still counts as runtime, and each sample is charged
to the innermost frame of the emitted program — whose `// loft:<file>:<line>` comment names
the loft line.  A runtime routine is reported under the ENTRY the program called
(`OpDatabase`, `vector_add`), grouped into families.

The binary must carry frame pointers AND line tables, and so must the runtime rlib it links —
otherwise every call chain stops at the runtime boundary and inclusive equals self:

    RUSTFLAGS=-Cforce-frame-pointers=yes cargo build --profile profiling --lib
    loft --native-release --native-emit out.rs prog.loft
    rustc --edition=2024 -o prog.bin out.rs -C opt-level=3 -C codegen-units=1 \\
        -Cdebuginfo=1 -Cforce-frame-pointers=yes \\
        -Clink-arg=-Wl,--allow-multiple-definition \\
        --extern loft=target/profiling/libloft.rlib -L dependency=target/profiling/deps \\
        --extern loft_ffi=target/profiling/deps/libloft_ffi-<hash>.rlib \\
        [-l dylib=<native lib> -L native=<its dir>]...
    taskset -c 2 perf record -F 4999 --call-graph fp -o perf.data ./prog.bin <args>
    scripts/native_attrib.py prog.bin out.rs perf.data

Pin a performance core (`taskset`) on a hybrid CPU.  Frame pointers cost a few per cent, so
quote shares from this tool and absolute times from the release build.  libc routines carry no
frame pointer, so the frame directly above one is skipped; their samples are still charged to
the program line below.  Needs `perf` and `llvm-symbolizer`.
"""
import collections
import json
import re
import subprocess
import sys

if len(sys.argv) != 4:
    sys.exit(__doc__)
binary, emitted, perf_data = sys.argv[1:4]
base = binary.rsplit('/', 1)[-1]
emitted_base = emitted.rsplit('/', 1)[-1]

# The runtime entries the emitted program calls, by what they spend the time on.  An entry
# not listed reports under its own name.
FAMILIES = [
    ('store mint/free', {
        'OpDatabase', 'OpDatabaseNP', 'OpFreeRef', 'free_named', 'database_named', 'null_named',
        'place_record_prefilled', 'unlock', 'close_file_handle', 'OpFreeRefIfDistinct',
        'OpFreeRecordIn', 'OpPlaceRecord', 'init', 'set_free_header'}),
    ('record mint/copy/move', {
        'OpNewRecord', 'OpNewRecordNP', 'OpFinishRecord', 'OpCopyRecord', 'record_finish',
        'move_record_out', 'OpMoveRecord', 'record_new', 'set_default_value_nullable',
        'copy_claims', 'remove_claims_mode'}),
    ('vector field copy', {'vector_add'}),
    ('vector append/grow', {
        'append_byte', 'append_f64', 'append_i64', 'append_i32', 'pre_alloc_vector',
        'push_record_hoisted<false>', 'push_record_finish<false>', 'vector_append',
        'clear_vector_release', 'vector_finish', 'push_header', 'vector_buffer_reset',
        'append_slot', 'OpAppendVector', 'push_hoisted<f64, false>'}),
    ('frame prelude', {'cr_call_push_lean', 'cr_call_push', 'new', 'drop', 'cr_call_pop'}),
    ('checked arithmetic', {
        'op_add_int', 'op_add_long_nn', 'op_add_long', 'op_mul_long', 'op_sub_long',
        'op_neg_long_nn', 'op_mul_int', 'op_sub_int', 'op_div_long', 'op_rem_long',
        'note_format_fault', 'op_cast_int_from_float', 'op_conv_float_from_int'}),
    ('text', {
        'text_character', 'set_str', 'OpGetTextSub', 'text_byte_at_native', 'append_text',
        'OpAppendText', 'get_str'}),
    ('store access', {
        'store_mut', 'store', 'set_float', 'set_int', 'set_byte', 'set_i32_raw', 'get_float',
        'get_int', 'get_byte', 'get_vector', 'length_vector', 'vec_get_or_raise_runtime',
        'checked_vec_pos', 'get_i32_raw', 'set_long', 'get_vector_hoisted<false>',
        'begin_write_inner<f64>', 'begin_write_inner<i64>', 'begin_write_inner<i32>',
        'begin_write_inner<u32>', 'set_u32_raw', 'get_u32_raw', 'vec_header'}),
]


def family(entry):
    for name, members in FAMILIES:
        if entry in members:
            return name
    return entry


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, check=True, **kw).stdout


addr_of = {}
for line in run(['nm', binary]).splitlines():
    parts = line.split()
    if len(parts) == 3:
        addr_of.setdefault(parts[2], int(parts[0], 16))

# emitted.rs: rust line -> the loft line it lowers, and the program's own function names
loft_at = {}
prog_fns = set()
cur = None
with open(emitted) as f:
    for i, text in enumerate(f, 1):
        m = re.search(r'// loft:(\S+):(\d+)\s*$', text)
        if m:
            cur = f'{m.group(1).rsplit("/", 1)[-1]}:{m.group(2)}'
        m = re.match(r'\s*(?:pub )?fn (\w+)[(<]', text)
        if m:
            prog_fns.add(m.group(1))
            cur = f'{m.group(1)} (entry)'  # the prelude, before the first line comment
        loft_at[i] = cur

samples = []
frames = []
script = run(['perf', 'script', '-i', perf_data, '--no-inline', '--no-demangle',
              '-F', 'ip,sym,symoff,dso'])
for text in script.split('\n'):
    if not text.strip():
        if frames:
            samples.append(frames)
        frames = []
        continue
    m = re.match(r'\s*[0-9a-f]+ (.*?)\+0x([0-9a-f]+) \((.*)\)$', text)
    if m and m.group(3).endswith('/' + base) and m.group(1) in addr_of:
        frames.append((addr_of[m.group(1)] + int(m.group(2), 16), m.group(1), m.group(3)))
    else:
        m = re.match(r'\s*[0-9a-f]+ (.*?)(?:\+0x[0-9a-f]+)? \((.*)\)$', text)
        frames.append((None, m.group(1) if m else text.strip(), m.group(2) if m else '?'))
if frames:
    samples.append(frames)

# every in-binary address once; a caller's return address minus one names the call
want = sorted({a if k == 0 else a - 1
               for fr in samples for k, (a, _, _) in enumerate(fr) if a is not None})
inline = {}
out = run(['llvm-symbolizer', '--obj=' + binary, '--inlining', '--output-style=JSON',
           '--functions=short', '--demangle'], input='\n'.join(hex(a) for a in want))
for text in out.splitlines():
    j = json.loads(text)
    inline[int(j['Address'], 16)] = [
        (s.get('FunctionName', '?'), s.get('FileName', ''), s.get('Line', 0)) for s in j['Symbol']]


def kind(fn, file, line):
    if file.endswith('/' + emitted_base):
        return 'prog'
    # merged code carries line 0 and an arbitrary file; its function name decides
    if line == 0 and fn in prog_fns and (fn.startswith('n_') or fn.startswith('t_')):
        return 'prog'
    if '/rustc/' in file or '/rustlib/' in file:
        return 'std'
    if '/src/' in file or '/loft-ffi/' in file:
        return 'rt'
    return 'other'


total = 0
by_class = collections.Counter()
leaf = collections.Counter()
entry_tot = collections.Counter()
line_tot = collections.Counter()
line_entry = collections.Counter()
prog_line = collections.Counter()
fn_incl = collections.Counter()
fn_own = collections.Counter()
fn_rt = collections.Counter()
fam_fn = collections.Counter()
for fr in samples:
    chain = []  # logical frames, innermost first: (function, file, line, kind)
    for k, (a, sym, dso) in enumerate(fr):
        if a is None:
            lib = 'std' if ('libc' in dso or 'ld-linux' in dso) else dso.rsplit('/', 1)[-1]
            chain.append((sym, dso, 0, lib))
            continue
        for fn, file, line in inline.get(a if k == 0 else a - 1, []):
            chain.append((fn, file, line, kind(fn, file, line)))
    pi = next((i for i, c in enumerate(chain) if c[3] == 'prog'), None)
    if pi is None:
        continue  # the loader and runtime start-up
    total += 1
    first = next(c for c in chain if c[3] != 'std')
    if chain[0][3] == 'std' and (fr[0][0] is None or first[3] == 'prog'):
        cls, name = 'std', chain[0][0]
    else:
        cls, name = first[3], first[0]
    pf = chain[pi]
    at = loft_at.get(pf[2]) if pf[2] else f'{pf[0]} (merged)'
    at = at or '?'
    by_class[cls] += 1
    leaf[(cls, name)] += 1
    if cls == 'prog':
        prog_line[at] += 1
        fn_own[pf[0]] += 1
    else:
        # the runtime ENTRY: the outermost runtime frame below the program frame
        entry = next((c[0] for c in reversed(chain[:pi]) if c[3] == 'rt'),
                     chain[pi - 1][0] if pi else name)
        if cls != 'rt':
            entry = cls
        line_tot[at] += 1
        line_entry[(at, entry)] += 1
        entry_tot[entry] += 1
        fn_rt[pf[0]] += 1
        fam_fn[(family(entry), pf[0])] += 1
    for f in {c[0] for c in chain if c[3] == 'prog'}:
        fn_incl[f] += 1


def pct(n):
    return f'{100.0 * n / total:5.1f}%'


def top(counter, key, n):
    rows = sorted(((m, k[1]) for k, m in counter.items() if k[0] == key), reverse=True)[:n]
    return ', '.join(f'{k} {100.0 * m / total:.1f}' for m, k in rows)


print(f'{total} samples in the program ({len(samples) - total} outside it)')
print('\n== where the cycles are: the innermost frame that is not the standard library ==')
for c, n in by_class.most_common():
    print(f'{pct(n)}  {c}')
print('\n== runtime time by the FAMILY of the entry the program called ==')
fam_tot = collections.Counter()
for e, n in entry_tot.items():
    fam_tot[family(e)] += n
for fam, n in fam_tot.most_common(25):
    print(f'{pct(n)}  {fam:24} {top(fam_fn, fam, 6)}')
print('\n== runtime time by ENTRY (top 30) ==')
for e, n in entry_tot.most_common(30):
    print(f'{pct(n)}  {e}')
print('\n== loft function: inclusive | its own code | runtime charged to its lines ==')
for f, n in fn_incl.most_common(30):
    print(f'{pct(n)} | {pct(fn_own[f])} | {pct(fn_rt[f])}  {f}')
print('\n== runtime time charged to a loft LINE (top 30) ==')
for at, n in line_tot.most_common(30):
    print(f'{pct(n)}  {at:28} {top(line_entry, at, 4)}')
print('\n== leaf self time (top 30) ==')
for (c, name), n in leaf.most_common(30):
    print(f'{pct(n)}  {c:5} {name}')
