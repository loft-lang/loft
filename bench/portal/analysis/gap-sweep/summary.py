#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Each routine of a gap sweep in TWIN UNITS: every row of part 1 divided by the twin's whole routine."""
import re,glob,os,collections,sys
S=sys.argv[1] if len(sys.argv)>1 else "/tmp/gap-sweep"
short={"the emitted lines' own work":"own","runtime INLINED":"inlined","keyed collections":"keyed","store claim / free / grow":"store","vector runtime":"vector","block copy / fill":"copy/fill","allocation":"alloc","text runtime":"text","other loft runtime":"other-loft","libstd / other":"libstd","the library's own #native Rust":"lib-native","the bridge to it":"bridge","checked arithmetic":"ops"}
def key(label):
    for k,v in short.items():
        if label.startswith(k): return v
    return label[:10]
rows=[]; tot=collections.Counter(); lead=collections.Counter()
for f in sorted(glob.glob(S+'/*.txt')):
    t=open(f).read(); name=os.path.basename(f)[:-4]
    m=re.search(r"loft ([\d,]+) instructions (per \w+) · twin ([\d,]+) · ([\d.]+)×",t)
    if not m: continue
    lt=int(m[1].replace(',','')); tw=int(m[3].replace(',',''))
    part=t.split('1. WHERE')[1].split('the twin\'s whole routine')[0]
    r={}
    for c in re.finditer(r"^\s*([\d,]+)\s+[\d.]+ %\s+(.+?)(?::|$)",part,re.M):
        k=key(c[2].strip()); r[k]=r.get(k,0)+int(c[1].replace(',',''))/tw
    rows.append((lt/tw,name,r))
    for k,v in r.items(): tot[k]+=v
    gap=sorted(((v,k) for k,v in r.items()),reverse=True)
    lead[gap[0][1]]+=1
rows.sort(key=lambda x:-x[0])
cols=['own','inlined','keyed','store','vector','copy/fill','alloc','other-loft','libstd','lib-native','bridge','text']
print(f"{'routine (instructions, twin = 1.00)':34} {'×':>5} "+' '.join(f"{c[:7]:>7}" for c in cols))
for x,n,r in rows:
    print(f"{n:34} {x:5.2f} "+' '.join(f"{r.get(c,0):7.2f}" if r.get(c,0)>=0.05 else f"{'·':>7}" for c in cols))
n=len(rows)
print(f"{'MEAN over '+str(n):34} {sum(x for x,_,_ in rows)/n:5.2f} "+' '.join(f"{tot[c]/n:7.2f}" for c in cols))
print("\nthe largest row per routine:", dict(lead.most_common()))
