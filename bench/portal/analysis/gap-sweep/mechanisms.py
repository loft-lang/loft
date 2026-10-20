#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The inlined runtime of a gap sweep by the function it came from, summed across routines in twin units."""
import re,glob,os,collections,sys
S=sys.argv[1] if len(sys.argv)>1 else "/tmp/gap-sweep"
agg=collections.Counter(); cnt=collections.Counter(); where=collections.defaultdict(list); n=0
for f in glob.glob(S+'/*.txt'):
    t=open(f).read()
    m=re.search(r"twin ([\d,]+) ·",t)
    if not m or 'by the function it came from' not in t: continue
    n+=1; tw=int(m[1].replace(',',''))
    hb=t.split('by the function it came from (the mechanism):')[1].split('runtime CALLED')[0]
    for h in re.finditer(r"^\s*([\d,]+)\s+(.+)$",hb,re.M):
        v=int(h[1].replace(',',''))/tw; agg[h[2].strip()]+=v; cnt[h[2].strip()]+=1; where[h[2].strip()].append((v,os.path.basename(f)[:-4]))
print(n,"reports; mean twin-units per routine, routines where it is in the top 12, the three heaviest")
for k,v in agg.most_common(int(sys.argv[2]) if len(sys.argv)>2 else 30):
    top=', '.join(f"{w} {x:.2f}" for x,w in sorted(where[k],reverse=True)[:3])
    print(f"{v/n:6.3f} {cnt[k]:3d}  {k:52} {top}")
