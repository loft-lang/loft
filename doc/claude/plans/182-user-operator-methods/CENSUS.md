<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN182 P0 — the name census

Every candidate name, and the METHODS (first parameter `self`) that already carry it in the stdlib
(`default/*.loft`) or in any library's public API, published or on `origin/main` (`make
libcatalogue`).  Regenerate: `python3 doc/claude/plans/182-user-operator-methods/census.py` from the
repo root after `make libcatalogue`.  A name with a use is either COMPATIBLE (the method already
means the operator) or TAKEN (it means something else, and adopting it would hand that type an
operator it never asked for).

```
compare        0  free
Ordering       0  free
then           0  free
cmp            0  free
order          1  stage: order(self: const Stage) -> vector<integer>`
equals         0  free
eq             0  free
same           0  free
less           0  free
less_than      0  free
lt             0  free
before         0  free
after          0  free
greater        0  free
plus           0  free
minus          1  time: minus(self: DateTime, span: Duration) -> DateTime`
times          0  free
divided_by     0  free
over           0  free
remainder      0  free
modulo         0  free
mod            0  free
rem            0  free
negate         1  time: negate(self: Duration) -> Duration`
negated        0  free
neg            0  free
opposite       0  free
add            2  tween: add(self: Track, tw: Tween) ; web: add(self: WsGroup, h: WsHandler) 
sub            0  free
subtract       0  free
multiply       0  free
mul            0  free
divide         0  free
div            0  free
scale          0  free
scaled         0  free
at             0  free
get            2  arguments: get(self: Args, name: text) -> text?`; random: get(self: RandStream, lo: integer, hi: integer) -> integer?`
set            0  free
element        0  free
set_element    0  free
item           1  stdlib:06_json.loft: item(self: const JsonValue, index: integer) -> JsonValue[self];
lookup         0  free
find           1  stdlib:03_text.loft: find(self: text, value: text) -> integer?;
slice          0  free
range          0  free
key_range      0  free
span           0  free
between        0  free
index          0  free
put            0  free
insert         1  stdlib:01_code.loft: insert(self: vector<T>, index: integer, elem: T) 
next           2  server: next(self: Server) -> Request?`; server: next(self: WebSocket) -> text?`
to_text        8  stdlib:01_code.loft: to_text(self: boolean) -> text; stdlib:01_code.loft: to_text(self: character) -> text; stdlib:01_code.loft: to_text(self: float) -> text; stdlib:01_code.loft: to_text(self: integer) -> text; stdlib:01_
set_at         0  free
power          0  free
bit_and        0  free
bit_or         0  free
bit_xor        0  free
bit_not        0  free
shift_left     0  free
shift_right    0  free
```

**Reading.**  `add` is TAKEN twice (`tween` `Track.add(Tween)` and `web` `WsGroup.add(WsHandler)` mean
"insert") and `get` twice (`arguments`, `random`), so neither may back an operator: `plus` and
`at` do.  `minus` and `negate` are COMPATIBLE — `time` already defines them with the operator's
meaning, so adopting them changes nothing `time` wrote.  `item` (the stdlib JSON element read) and
`find` / `insert` / `next` / `to_text` are not candidates.  Every chosen name — `compare`, `plus`,
`minus`, `negate`, `times`, `divided_by`, `remainder`, `at`, `set_at`, `slice`, `key_range`, and
the reserved `power` / `bit_*` / `shift_*` — is free or compatible.  `Ordering`, `Less`, `Greater`
and `then` are free in the stdlib and the catalogue.

The census reads public APIs only: a library's private method of a backing name is unaffected by
(Op-Home) only if it sits on a type of that library, which it does by construction; a CONSUMER
application (moros, dryopea, crawler) is not in the catalogue and is re-checked in P5 by the
revalidation run.
