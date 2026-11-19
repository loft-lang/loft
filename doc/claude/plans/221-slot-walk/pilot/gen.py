#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# The pilot of @PLN221: writes one .loft cell per slot x filler x kind into the current
# directory, and expect.txt with the counts each must print.  Run it in a scratch directory.
import itertools
PRE='''struct P { k: integer, t: text }
struct Ctr { calls: integer = 0 }
struct W { n: integer, t: text, items: KIND }
struct Hold { ws: vector<W>, v: vector<integer> }
fn mk(c: Ctr) -> KIND { c.calls += 1; [P { k: 1, t: "a" }, P { k: 2, t: "b" }, P { k: 3, t: "c" }] }
fn box(c: Ctr, h: Hold) -> W { c.calls += 1; h.ws[0] }
fn at(c: Ctr, i: integer) -> integer { c.calls += 1; i }
fn tx(c: Ctr) -> text { c.calls += 1; "abc" }
fn yes(c: Ctr, b: boolean) -> boolean { c.calls += 1; b }
fn hold() -> Hold { Hold { ws: [W { n: 1, t: "x", items: [P { k: 1, t: "a" }, P { k: 2, t: "b" }, P { k: 3, t: "c" }] }, W { n: 2, t: "y", items: [P { k: 4, t: "d" }] }], v: [10, 20, 30, 40] } }
fn main() {
  c = Ctr {}; d = Ctr {}; h = hold(); s = 0; o = "";
BODY
  println("NAME c={c.calls} d={d.calls} s={s} o={o} ");
}
'''
kinds={'vec':'vector<P>','sor':'sorted<P[k]>','idx':'index<P[k]>','has':'hash<P[k]>'}
cf={'call':'mk(c)','fld':'box(c, h).items','look':'(h.ws[at(c, 0)].items ?? [])','lookq':'h.ws[at(c, 0)].items?'}
cs={'for':'for p in E { s += p.k; }','forif':'for p in E if p.k > 1 { s += p.k; }','len':'s = len(E);','compr':'q = [for p in E { p.k }]; s = len(q);','get':'s = E[2].k;'}
cells={}
for (kn,k),(fn,f),(sn,sl) in itertools.product(kinds.items(),cf.items(),cs.items()):
    if kn=='has' and sn in('for','forif','compr'): continue
    cells[f'{sn}-{fn}-{kn}']=(k,sl.replace('E',f),'c=1')
V='vector<P>'
one={
 'cadd-fld-call':('box(c, h).n += at(d, 5); s = h.ws[0].n;','c=1 d=1 s=6'),
 'cadd-look':('h.ws[at(c, 1)].n += at(d, 5); s = h.ws[1].n;','c=1 d=1 s=7'),
 'tadd-look':('h.ws[at(c, 1)].t += tx(d); o = h.ws[1].t;','c=1 d=1 o=yabc'),
 'vadd-look':('h.ws[at(c, 1)].items += [P { k: at(d, 9), t: "z" }]; s = len(h.ws[1].items);','c=1 d=1 s=2'),
 'vadd2-look':('h.ws[at(c, 1)].items += [P { k: 8, t: "y" }, P { k: at(d, 9), t: "z" }]; s = len(h.ws[1].items);','c=1 d=1 s=3'),
 'elem-set':('h.v[at(c, 1)] = at(d, 7); s = h.v[1];','c=1 d=1 s=7'),
 'elem-cadd':('h.v[at(c, 1)] += at(d, 7); s = h.v[1];','c=1 d=1 s=27'),
 'elem-cmul':('h.v[at(c, 1)] *= at(d, 2); s = h.v[1];','c=1 d=1 s=40'),
 'range':('for i in at(c, 0)..at(d, 3) { s += i; }','c=1 d=1 s=3'),
 'range-incl':('for i in at(c, 0)..=at(d, 3) { s += i; }','c=1 d=1 s=6'),
 'slice':('q = h.v[at(c, 1)..at(d, 3)]; s = len(q);','c=1 d=1 s=2'),
 'forslice':('for x in h.v[at(c, 1)..at(d, 3)] { s += x; }','c=1 d=1 s=50'),
 'fortext':('for ch in tx(c) { s += 1; }','c=1 s=3'),
 'fmt':('o = "{at(c, 5)}-{at(c, 6):3}";','c=2'),
 'fmt-fld':('o = "{box(c, h).n}{box(c, h).t}";','c=2 o=1x'),
 'and-short':('if yes(c, false) && yes(d, true) { s = 1; }','c=1 d=0'),
 'or-short':('if yes(c, true) || yes(d, true) { s = 1; }','c=1 d=0 s=1'),
 'coal-present':('s = h.v[at(c, 1)] ?? at(d, 9);','c=1 d=0 s=20'),
 'coal-absent':('s = h.v[at(c, 11)] ?? at(d, 9);','c=1 d=1 s=9'),
 'if-expr':('s = if yes(c, true) { at(d, 1) } else { at(d, 2) };','c=1 d=1 s=1'),
 'minmax':('s = min(at(c, 4), at(d, 3));','c=1 d=1 s=3'),
 'cmp':('if at(c, 1) < at(d, 2) { s = 1; }','c=1 d=1 s=1'),
 'meth-temp':('s = tx(c).len();','c=1 s=3'),
 'txt-idx':('o = "{tx(c)[at(d, 1)]}";','c=1 d=1 o=b'),
 'txt-slice':('o = tx(c)[at(d, 1)..];','c=1 d=1 o=bc'),
 'neg-idx':('s = h.v[-at(c, 1)];','c=1 s=40'),
 'nested-look':('s = h.ws[at(c, 0)].items[at(d, 1)].k;','c=1 d=1 s=2'),
 'for-look-fld':('for p in h.ws[at(c, 0)].items ?? [] { s += h.v[at(d, 0)] ?? 0; }','c=1 d=3 s=30'),
 'while':('i = 0; while i < at(c, 2) { i += 1; } s = i;','c=3 s=2'),
 'elem-self':('h.v[at(c, 1)] = h.v[at(c, 2)] + h.v[at(d, 0)]; s = h.v[1];','c=2 d=1 s=40'),
 'look-set-fld':('h.ws[at(c, 1)].n = at(d, 7); s = h.ws[1].n;','c=1 d=1 s=7'),
 'look-set-txt':('h.ws[at(c, 1)].t = tx(d); o = h.ws[1].t;','c=1 d=1 o=abc'),
 'look-set-items':('h.ws[at(c, 1)].items = [P { k: at(d, 7), t: "q" }]; s = len(h.ws[1].items);','c=1 d=1 s=1'),
 'push-v':('h.v += [at(c, 1), at(c, 2)]; s = len(h.v);','c=2 s=6'),
 'vadd-box':('box(c, h).items += [P { k: at(d, 9), t: "z" }]; s = len(h.ws[0].items);','c=1 d=1 s=4'),
 'tadd-box':('box(c, h).t += tx(d); o = h.ws[0].t;','c=1 d=1 o=xabc'),
 'fmt-coll':('o = "{mk(c)}"; o = "";','c=1'),
 'if-stmt':('if at(c, 2) > 1 { s = at(d, 4); } else { s = at(d, 5); }','c=1 d=1 s=4'),
 'coal-fld':('s = h.ws[at(c, 5)].n ?? at(d, 9);','c=1 d=1 s=9'),
 'q-fld':('s = h.ws[at(c, 1)].n?;','c=1 s=2'),
 'coal-call-coll':('for p in h.ws[at(c, 5)].items ?? mk(d) { s += p.k; }','c=1 d=1 s=6'),
 'look-rm':('h.v[at(c, 1)] = null; s = len(h.v);','c=1'),
 'fmt-look':('o = "{h.ws[at(c, 1)].t}";','c=1 o=y'),
 'arg-look':('s = at(d, h.v[at(c, 1)] ?? 0);','c=1 d=1 s=20'),
 'tadd-fmt':('h.ws[at(c, 1)].t += "{at(d, 4)}"; o = h.ws[1].t ?? "";','c=1 d=1 o=y4'),
 'match-int':('match at(c, 2) { 1 => { s = 1; }, 2 => { s = at(d, 2); }, _ => { s = 9; } }','c=1 d=1 s=2'),
}
for n,(b,e) in one.items(): cells[n]=(V,b,e)
exp=open('expect.txt','w')
for n,(k,b,e) in cells.items():
    open(n+'.loft','w').write(PRE.replace('KIND',k).replace('BODY','  '+b).replace('NAME',n)); exp.write(f'{n}\t{e}\n')
print(len(cells))
