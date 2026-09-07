---
render_with_liquid: false
---
# What's new in loft

A short, friendly log of what has changed in each release.  Read top-to-bottom
for a tour of how the language has grown.

Looking for the deep technical history (opcode renames, slot allocator
invariants, internal phase numbers)?  See
[doc/claude/CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md).

---

## 2026-09

**A `?` on a tuple type now says why it cannot be one.**  `t: (integer, integer)? = …` reported
*"Expect token ;"* and left you looking at the semicolon.  A tuple has nowhere to keep "absent" —
it is just its members' bytes — so the type is refused, and the message now says that and names
the two things you can do instead: make the MEMBERS nullable (`(integer?, text?)`), or wrap the
tuple in a `struct`, which can be `?`.

**Reading a tuple out of a vector at a position that may not exist now behaves like every other
type.**  `v[i]` where `i` might be past the end gives "a tuple or nothing", and `v[i]?` is how
you ask for the tuple with its defaults — but it was answering `null` in both members while the
same program written with a struct answered `0`.  It now gives the members' defaults.  Reading
`.0` off the undischarged value, or destructuring it, is refused with a message that names the
discharge instead of complaining about a field name or claiming the value is not a tuple.

**Pattern matching now works over a vector whose elements may be absent.**  `match v { [Id { x }]
=> x, … }` over a `vector<Tok?>` was refused with an error that pointed at a comma and explained
nothing, and the shorter spelling `[Id]` was worse: it quietly matched EVERY element — a
different variant, and an absent one — because the pattern had turned into a plain binding named
`Id`.  Both spellings now ask the same question the dense `vector<Tok>` asks, and an absent
element matches no variant, so it simply falls to the next arm.  The same fix covers a nullable
field inside a pattern (`A { t: Id { x } }`) and a cursor whose source holds absences.

**An element you bind out of a vector keeps that vector alive.**  `match v { [a, ..] => a }`
handed back the first element of a `vector<Tok?>` without recording that the value points INTO
the vector, so the vector's memory was released when the function returned and the caller read
whatever was stored there next — a wrong value, quietly, on both backends.  The dense
`vector<Tok>` was right all along; the nullable one now says what it borrows.

**A slice pattern over the wrong kind of struct now says which field is wrong.**  `match c {
[Id { x }] => … }` needs a cursor — a struct with a `vector<…>` to read from and an integer
`pos`.  Given a struct that is neither, loft used to report *"Expect token }"* three times and
stop.  It now says *"a slice pattern `[ … ]` matches a vector or a cursor; `Cur` is neither — its
`pos` field is `integer?`, and a cursor's position must be an integer"*, once, and keeps parsing.

**A `match` arm now has to answer in the type its siblings answer in.**  Every arm was parsed
without knowing what type the `match` as a whole was expected to produce, so an arm of another
type was neither converted nor refused: a `float` destination with an integer arm read the
integer's BITS as a float, an `integer` destination with a `2.5` arm read the float's bits as
an integer, and `250 + 10` landed in a `u8` local holding 260.  All of it silent, on both
backends, and for every kind of subject you can match on.  Arms now convert to the type their
siblings answer in, exactly as an `else` block converts to its `if`'s — so the integer arm
becomes a real `2.0`, and the ones that cannot convert are reported and name the arm.

**Removing an item from a collection inside an optional record now updates its index.**  A
struct held inside a `vector<Room?>` has collections of its own; removing an item from one of
them left the removed item findable through the sibling lookup, so a search still returned
something that was no longer there.  The same shape one level up, and the same shape without
the `?`, always worked.

**Writing past the end of a vector no longer stops the program.**  `items[9] = thing` on a
shorter vector does nothing, which is what it has always meant; on compiled and interpreted
programs alike it had begun ending the run instead.  A negative index still counts from the
end and writes a real element.

**A value that turns out to be absent no longer costs you memory.**  Assigning a
may-be-missing value over one a variable already held (`x = maybe`) quietly kept the old
record alive when `maybe` turned out to be nothing — once per time it happened, on compiled
programs only.  A long-running loop that reassigned such a variable grew for as long as it
ran.

**A `match` used for its value now says so when an arm gives none.**
`v = match k { 1 => { 5 }, _ => { println("one") } }` quietly set `v` to null when the second
arm ran, and on compiled programs it failed with an error from the Rust compiler instead. It
is now refused where you wrote it, with a message saying to give the arm a value or end the
`match` with `;` to make it a statement.

**Writing an `if` statement the other way round now works.**
`if k == 1 { println("one") } else { 5 };` compiled, while the mirror
`if k == 1 { 5 } else { println("one") };` was rejected — the same statement with its arms
swapped. Both are statements, both throw the value away, and both now compile.

**A function that returns one of two things now hands back a value of its own.**
`fn pick(p, q, first) -> Node? { if first { p } else { q } }` gave the caller the *second*
argument itself, so writing to the result changed the caller's own variable — while the first
argument was correctly copied. Both are copies now, and so is the plain `-> Node` version,
which had the same problem despite being the documented way around it, and so is the generic
`fn pick<T>(p: T, q: T, …) -> T?`.
which had the same problem despite being the documented way around it.

**A generic function used at two integer widths now works in either order.**  Calling the
same generic with a `u8` and then a `u16` was rejected — *"cannot implicitly narrow u16 to
u8"*, pointing at the second call — while writing the two calls the other way round compiled
fine.  The two widths were being treated as one specialisation. They are now two, and the
order you write them in no longer matters.

**A linked list or a tree no longer crashes the compiler when a generic touches it.**
`struct Node { value: integer, next: reference<Node>? }` passed to any generic function killed
the process outright — no error message, no output. The compiler was walking the struct's own
`next` edge round and round while measuring it. It now recognises that a field pointing back
at its own struct is a link rather than something stored inside it.

**A `&` reference can link a value that may be absent.**  `fn bump(p: &integer?)` and
`q = &x` where `x: integer?` were refused outright — you had to link the non-null value and
carry the absence beside it.  Both now work, read and write, on both backends: reading gives
the source's current value (`p ?? 0` and the rest), writing reaches the source, and writing
`null` clears it.

**…and so does a `&` link to a field or a list element.**  `pi = &o.i; pi = S { n: 2 }`
left `o.i` alone, while `pi.n = 2` through the same link worked — so the link looked
correct right up to the moment you replaced the whole value.  Both now write the thing the
link names.

**A `&` link to a text or a vector now behaves like the `&` you already know.**  Writing a
`&` variable is supposed to write the thing it links — `a = 3; b = &a; b = 4` leaves `a` at
4 — and that held for numbers, and for a `&` PARAMETER of any kind.  A `&` to a text or a
vector LOCAL was quietly different: `pc = &c; pc = "z"` left `c` alone (the `&` was dropped
and you got a copy), and `pe = &e; pe = [2, 2]` gave `pe` a new list instead of replacing
`e`'s contents.  Writing the link with the `&` on the type instead of the value
(`pc: &text = c`) could crash outright.  All of these now do what the plain-language rule
says, on both backends, and a `&` to a struct no longer leaks the value it replaces.

The **say-what-you-do** release. Two threads, and they turned out to be one: every page of
the language reference was read against the compiler that ships, and most of what came back
was not a wrong sentence but a promise nothing was keeping.

**A value chosen by an `if` keeps what it was given.**  `x = if k > 0 { h.inner } else
{ mk(0) }` used to start reading the NEW `h` as soon as `h` was replaced — on both backends
when `x` was fresh, and on the compiled one even when it was not.  It now keeps the value it
was handed, like every other binding.

**A view taken inside an `if` or `match` arm is copied when its container is replaced in the
same arm.**  `got = match sh { Holder{inner} => { sh = Empty{…}; inner.a }, … }` read the new
subject's bytes; the same two lines written outside a branch have been copied and reported for
a month.  A plain struct view in that position was wrong on both backends, not just one.

**A view of an enum's payload is copied when its subject is replaced, and the compiler says
so.**  `x = sh.inner` followed by `sh = Empty{…}` used to read the NEW subject's bytes through
the old view — `x.a` answered `0` where the payload said `1` — while the same code written
against a plain struct has copied and warned for a month.  Both now behave the same way and
both tell you.  The same blindness was making the compiler warn that a write through such a
view was "lost" when it was not, which gated library builds; that is gone too.

**A value that may be null is reported wherever it lands in a slot that cannot hold one.**
`t: (integer, integer) = (v[i], 1)`, `H { f: w[i] }` with a vector field, and `s[at + 1..]`
after a `find` — a tuple member, a vector field and an index or key — said nothing when the
value was null, while the same value into an element, an argument or a return already
warned.  All of them now warn, with the discharge to write (`?? d`, `?`, `match`); a program
that compiled yesterday still compiles.  The check moved to the one place every such value
passes, so a position nobody had listed cannot be silent again.

**A declared local takes a nullable value with a warning, like every other slot.**
`x: integer = v[i]` used to stop compilation with *cannot change type from integer to
integer?* — the one slot the compiler refused where a field, an element, an argument or a
return warned.  It now warns like they do, names the local, and the program runs with the
null in the slot (a narrow `u8` local still refuses: it has no bit pattern left for null).
An inferred local written a nullable value widens to `integer?` instead of being refused —
`a = 2; a = v[i]` — as the reference always said, and the next non-null slot it reaches is
where the warning lands.  A write-back `&integer` parameter, which used to take the null in
silence, warns with the rest.

**A nullable struct read out of a field or an element is one value, whichever way you
reach it.**  `x = y; x = o.opt` — a local first bound to a nullable parameter and then to a
nullable field — used to read `y` as if it carried the field's tag byte and free its record
underneath the caller; `x = o.opt ?? y` was refused with an internal name in the message;
`x = o.opt; if c { x = null }` was refused; and `d: S = o.opt` read a record of zeroes when the
field was absent.  A field's or an element's nullable struct is now read as the plain `S?` it
is the moment it leaves its slot, so every such local, `??` and `?` behaves as it does for a
plain `S?` value.  One thing to know: such a local views the field's record, so clearing the
field afterwards is not visible through the local, exactly as with a pointer field.

**A `&` reference to a nullable value is refused with a message, not silently copied.**
`q = &x` with `x: integer?` used to bind a plain copy — `q = 7` left `x` unchanged and nothing
said so — and a `&integer?` parameter could be neither read nor written in its body.  Both
now say *a `&` reference cannot link a nullable `integer?` yet* and name the way round; the
link itself is still owed (loft#1372).

**`loft introspect` shows the program as it is parsed.**  When a program had already run
once, the startup cache answered the next `introspect` from its bundle, and the dump then
named every variable `65535` with a `-` where its slot number and span belong — two runs of
the same command on the same file did not print the same thing.  `introspect` now always
parses; ordinary runs keep the cache.

**`if x is Variant { field }` reads the same value on both backends.**  A payload bound by
`is` was copied by the native compiler and shared by the interpreter, so the same program
could answer differently depending on how it was run — and the native copy was then never
released.  `match` has bound it correctly for a year; `is` does now too.  Writing through the
binding still reaches the value it came from.  (loft#1398)

**Replacing an enum inside a struct while you are still reading its payload now warns.**
`match w.st { Holder{inner} => { w.st = Empty{z: 0}; inner.a }, … }` reads `Empty`'s field
through `inner`, because writing into a place does not move what points at it — that is what
the language documents, and both backends agree. What was missing is that nothing said so, at
the one spelling the equivalent warning deliberately skips: a per-arm binding was taken to be
proof that the variant could not change underneath it. It can. The value is unchanged; you are
told, and told to copy the payload out first.  (loft#1397)

**A list read out of a chosen branch keeps the value it was given.**  `b = if … {
d.tiles.proto } else { [] }` followed by a new `d` used to read the NEW `d` when the program
was interpreted, while the compiled build read the old one — the same program answering two
different things depending on how you ran it.  Both give the old value now, which is the one
the branch chose.  Writing through such a list still does not reach the struct it came from:
copying is what a plain list assignment does, and the branch spelling follows it.  (loft#1399)

**A value kept from one loop pass to the next is the value from that pass.**  Reading a list
out of a returned struct in two steps — `t = dv.tiles; prev = t.proto;` — left `prev` reading
the CURRENT pass's data on the next turn, so a loop comparing consecutive steps answered
*nothing changed* every time.  Written as one expression (`prev = dv.tiles.proto`) it was
already right and said so; both spellings now copy, and both say so.  A number read the same
way, or a whole struct one step down, are unchanged.  (loft#1393, reported by the planet
generator)

**A list built from what it replaces reads the old value, at the last two places it did
not.**  `xs[0].items = [xs[0].items[1]?, xs[0].items[0]?]` — reversing a list that lives in a
struct inside a list — answered zeros, and so did the same line inside a closure over the
list.  Both read what the destination held when the statement began now, as a plain local, a
field and a parameter already did.  Reading a NEIGHBOURING field, or another element's, is
untouched: those are different places and are read as they stand.  (loft#1391)

**A `&` link to a vector keeps up with its source.**  `q = &v; v = [7, 8, 9]` left `q`
reading the two elements `v` had when the link was taken, while `v` read three — on both
backends, with nothing said.  The link now follows the source, as the `&` to a struct, a text
or a struct field always has.  Writing through either side still reaches the other.
(loft#1392)

**A local a closure captured, and then assigned again, no longer leaks the value it ends
up with.**  `s = S{…}; h = |i| { s.a + i }; s = build(h)` kept the store `s` ends up holding,
and so did the inline form `s = build(|i| { s.a + i })` and the same shapes over a vector —
one per reassignment, on both backends, with the right answer printed and only the exit
warning to say so.  The closure record owns the value it captured; the frame owns whatever the
local is given afterwards, and now frees it.  A capture that is never reassigned is unchanged:
that value still belongs to the closure.  A stored closure called twice, either spelling
inside a loop, and a closure capturing a list inside a loop are all clean.  (loft#1388)

**A `match` or `if` arm may hand back the enum it is choosing over.**  `e = match e {
Circle{r} => Circle{r: r + 1}, _ => e }` — replace one variant, keep everything else — was
refused with *"expected Circle, got Sh"*, as was its `if` spelling, where the same value
returned through a function typed as the enum was accepted.  A variant and its enum join to
the enum, and now they do.  The same change closes the other half of it: a `match` whose arms
answer in different variants used to keep the FIRST arm's variant, so `v: Circle = match e {
… Square{…} }` was accepted and read a `Square`'s bytes through `Circle`'s fields with nothing
said.  It is refused now, as the `if` spelling always was.  (loft#1390)

**A struct-enum local declared with its enum no longer leaks when you replace it.**  `e: Sh =
Circle{r: 1}` followed by `e = if … { circle(2) } else { e }` kept the record it replaced —
one per pass of a loop, on both backends, with the right answer printed and only the exit
warning to say so.  Writing the same local from a call, or leaving the annotation off, was
always clean.  (loft#1389)

**A vector literal that reads the vector it replaces reads what that vector held.**  `v =
[v[1], v[0]]` now reverses `v`, `v += [len(v), len(v)]` appends the length twice, and a struct
element, a struct field, a parameter or a loop reads the value from before the statement —
where every one of them used to read the empty (or half-built) result the literal was
filling, and answer zeros or defaults with nothing said.  Comprehensions were given this
promise in August; the literal is the same build without the loop and now shares its cure.
Two destinations still read the result being built — a field reached through an element,
and a collection a closure captured — and are on the list (loft#1391).

**Replacing a struct "when …" no longer leaks natively, and its `match` spelling compiles.**
`s = if s.a > 5 { mk(7) } else { s }` released the store it replaced on the interpreter and
kept it on `--native`, one per pass of a loop; and `s = match s.a { 9 => mk(7), _ => s }` was
refused by rustc outright.  Both backends now free the same store, and the `match` runs.

**A generic function works at a self-referential struct.**  Calling `fn id<T>(v: T) -> T?`
with a `Node` whose `next` is a `reference<Node>?` used to kill the compiler outright — no
message, just a crash — because sizing a `vector<Node>` element walked into `next` forever.
It now asks the store for the size, like every other vector does — and a generic that builds
a `vector<u8>` or `vector<i16>` now writes and reads its elements at their own width, where it
used to hand back a neighbour's bytes (`200 0` for two bytes `200, 201`).  (loft#1378)

**An `if` you write is yours, and an `else if` arm is an `else` arm.**  Storing an
if-expression into a `u8` (or any narrow type) used to treat it as a `??` fallback: the then
arm could read `null` in a slot that has no null, and the first thing compared in your
CONDITION was silently range-checked too, so `c: u8 = if k == 1000 { a } else { b }` took the
else arm.  Both now behave like every other spelling — a value that may not fit is refused at
compile time with the message `c: u8 = a + b` gets, and your condition is left alone.  And an
`else if` arm is now held to the first arm's type: `x: integer = if a { 1 } else if b { 2.5 }
else { 3 }` is refused instead of printing the float's bits, and `else if b { 2 }` behind a
`1.5` becomes `2.0`.  (loft#1379, loft#1380)

**A generic function returns a value of its own, like a plain function does.**  `fn same<T>(x: T)
-> T { x }` used to hand back the argument itself when `T` was a struct, a vector or a keyed
collection, so `r = same(v); r[0] = 99` changed `v`, and `fn pair<T>(x: T) -> (T, integer)`
did the same through the tuple; a plain function written the same way copied.  Every generic
now returns what its plain twin returns.  Two nullable tuple members were wrong for plain
functions as well: `(x, 1)` with `x: S?` read back garbage, and `(null, 2)` with a
`vector<T>?` member read back as an empty vector instead of `null`.

**A tuple built inside a generic copies what it holds.**  `t = (s, 1)` inside a
`fn f<T: …>(…)` now behaves the same as the version with the concrete type written in
place — mutating `s` afterwards no longer shows through `t.0` when `T` is a struct.  The
generic and the non-generic spelling of the same function had been giving different
answers, which is the one thing a type variable is supposed never to do.  A `T` bound to a
vector or a keyed collection is still shared rather than copied (loft#1365).
**A record set cannot be held two ways at once, and says so.**  Declaring a list of `E` and a
list of `E?` beside a lookup over the same records used to leave the first list quietly out of
the group — writes through it reached nothing else. That declaration is now refused, because
the two lists store their elements differently and one set cannot be read both ways; the error
names the two fixes. Lists that agree — all plain, or all nullable — are unaffected.

**A value read out of one field of a record is not disturbed by another field growing.**  The
copy-on-growth rule now asks WHICH list grew: reading from `w.a` and then appending to `w.a`
gives you your own copy, while appending to `w.b` leaves your view of `w.a` exactly as it was.

**A list you read out of a list stays yours when the outer list grows.**  The same fix as for
a record read, now for a list: `b = w[0]` followed by appends to `w` used to leave `b` empty
once the outer list outgrew its allocation.  `b` is now your own copy, and you are told so.

**Two lists and a lookup over the same records are all one collection again.**  A struct with
two plain lists beside a keyed one — `{ a: vector<E>, b: vector<E>, h: hash<E[k]> }` — behaved
as though the lookup were a hub: adding through it filled both lists, but adding through either
list reached only the lookup, so each list held part of the data and nothing said so.  All the
routes now reach all the members, whichever order you declare them in.  Two plain lists with no
keyed member beside them stay independent, as before.

**An `if` used as a statement no longer trips the native compiler.**  `if c { println("x") }
else { 5 };` ran fine interpreted and failed to build with `--native`, reporting a Rust type
error for loft code you wrote.  A statement's value is thrown away — that is what a statement
is — and both backends now agree.

**A value you read out of a list stays yours when the list grows.**  `d = v[0]` followed by
appends used to read whatever was at the old address once the list outgrew its allocation —
right for a few appends, garbage for many, with nothing said either way.  Now the value is
copied for you at the point you bound it, and a note tells you the copy happened and that
writes through it no longer reach the list.  Taking a `&` reference INTO a list that then
grows is refused instead of quietly breaking; a `&` to the list itself is unaffected.

**A lookup that finds nothing is `null` everywhere it goes.**  `v[i]` past the end, `h[k]` for
a key that is not there, and the same read on a `sorted` or an `index`, used to answer a value
that only some null tests could see: bound to a `S?` local it read as present, passed to a
`S?` parameter its `!= null` passed and its fields read `null`, returned from a `-> S?`
function it came back present, and `b = find(v, 9); b.n` read garbage.  Every one of those
now sees `null`, on both backends.  Two neighbours fixed on the way: a `vector<S?>` element
read by a variable index (`v[i]` rather than `v[1]`) no longer refuses to compile, and a
field read or a method call on an absent `vector<S?>` element (`v[i].n`, `v[i].area()`) is
`null` rather than a record of zeroes.

**A list chosen by an `if`, a `match` or a `??` is your own copy.**  Assigning a vector from a
branch — `x = if c { a } else { b }`, a `match`, `x = s.items ?? fallback` — now copies the chosen
branch the way `x = a` does, so writing through `x` no longer changes `a`; that holds on the first
assignment and on a later one, for a nullable `x`, and inside a loop.  Assigning another vector
to a vector PARAMETER now rebinds the parameter locally instead of overwriting the caller's
vector in place, as the language reference already promised.

**Replacing, nulling or removing one element of a list keeps its lookups in step.**  A struct
with a `vector<E>` and a `hash<E[k]>` over the same records is one record set with two routes,
and `w.es += [e]` has kept `w.by_k` current for a long time.  `w.es[0] = E { k: 11 }`,
`w.es[0] = null` and `w.es.remove(0)` did not: the lookup went on answering for the old key,
`len(w.by_k)` counted a record that was gone, and re-adding a removed key counted it twice —
on both backends, and one struct deeper (`w.rooms[0].items[0] = …`).  Each of those now
updates every keyed sibling, and `v.remove(i)` answers `true` or `false` as documented.

**A value kept in a `struct?` behaves the same whether it came from a variable, a call, or a
branch.**  A few store-sharing corners are gone: a nullable local set from a function that hands
back one of its arguments now gets its own copy (writing through it no longer changes the
caller's value); a function returning `struct?` no longer disturbs an argument on the path where
it answers `null`; and reassigning a record local from an `if`/`match` that yields a value copies
the chosen branch, as a first assignment already did.  A separate, invisible fix: a program run a
second time from the same directory (served from loft's on-disk cache) now behaves exactly like
its first run for these cases.

**A struct-enum value binds like a struct.**  `c = e` on an enum-with-fields value now gives
`c` its own copy on the interpreter, as it already did on `--native` — at a first bind, a
rebind, from a parameter, through a nullable spelling, from a variant into its enum
(`c: Shape = circle`), and on each arm of an `if` (where both backends had shared it).  A
nullable one no longer leaves its copy behind.  And a field every variant declares is
reachable through a nullable receiver (`e: Shape?; e.n = 9`, `x = e.n`) instead of being
refused as a type change on the write and an unknown field on the read — exactly as `s.v`
on an `s: S?` always was.  Found by walking the bind-copies rule across every position a
struct's bind is measured at; the same walk found that a tuple holding a vector or struct
member is shared, not copied, by a whole-tuple bind or a destructure (loft#1361, open, with
a verified workaround).

**A reassigned droppable is released.**  `s = S { h: open(a) }; s = S { h: open(b) }` never
closed `a`: the hook ran at scope end only, and the first record was rebuilt over or freed
without it.  It now runs at the reassignment, after the new value is computed, on both
backends — for a rebind from a literal, a call, to `null`, and inside a loop of a local
declared outside it.  A field or element overwritten in place is unchanged (the documented
boundary), and a literal handed into a nested field or an element is released once by its
container instead of twice.

**A copied droppable is released once.**  `t = s` on a record whose type defines `OpDrop`
— or holds a field that does — ran the hook for both records, so a copied file handle was
closed twice; a copy returned from a function ran it three times, the first two before the
caller had read what it was handed.  The copy now owns the resource and the source stops
dropping, the same move a struct field or a collection element already made, on both
backends.  A copy taken from a parameter leaves the caller as the owner.  Two copies of
one value are reported by the `double-move` warning, as two containers built from it
already were.

**A nullable local that views someone else's record no longer frees it.**  `d: In? =
q.inner; d = …` on a value with a nullable field of a parameter freed the caller's nested
record when `d` was reassigned — invisible until a later allocation reused the slot, at
which point the read returned the wrong value.  A view of a local's field or a vector
element failed the same way.  A projection is a view and owns nothing, so it is left alone;
a nullable local that actually mints a record of its own still releases it.

**A nullable local behaves like its dense twin inside a branch and a loop.**  `y: S? = S {
n: 3 }` inside a loop body — or a struct-enum literal, or the literal in an `if`/`match`
arm — was minted in a buffer the loop's next pass reused in place after another record had
taken it, so the second iteration's literal overwrote that record on both backends and
nothing said so.  A `S?`, `vector<T>?` or `text?` local first assigned inside one arm of an
`if` freed a stale word on the other arm (a refused free, or on the second call the free of
a live record), and one first assigned inside a loop body could not be read after the loop
(a use-after-free on the interpreter, a rustc error natively) where the same dense local
could.  And a keyed local bound from a `match` leaked the arm's collection where the
`if … else if …` spelling did not.  Each now does what the dense or the `if` spelling always
did.  One shape stays open: a nullable local assigned both a field like `o.opt` and a plain
`S?` value keeps only the last assignment's representation (loft#1367, being folded into
the null model's one bind junction; use a separate local per source until then).

**Every text buffer a frame mints is released.**  A handful of shapes answered the right
value on both backends while the interpreter left one text buffer behind per call: a
lambda returning a captured text through `??`, a nullable text local returned, a
generic's early return of a loop variable and a generic forwarding a nested generic, a
`par` loop discarding text elements, a function whose result reads its own return
buffer, a `??` consumed by a number (`len(s.name ?? "")`) or by an `if` whose arm
returns, and a `parallel` arm passing a formatted string.  A loop over any of them grew
without bound.  Each now frees, or hands its text to the caller's buffer, the way the
plain spelling always did.  The release's valgrind sweep is the instrument that saw it,
and it now runs nightly.

**A value chosen by `if`, `match` or `??` now binds like any other value.**
`b = if c { a } else { [0, 0] }` used to alias `a` — a later write to `a` showed through
`b` — and a branch mixing a fresh value with an existing one could leak the fresh one on
every pass of a loop, or copy on one backend and alias on the other.  Every arm now binds
the way a plain `b = a` or `b = f(x)` would, on both backends: a variable copies, a call's
result is owned, a view stays a view.  The same rule reaches `x = f(v) ?? d`, a local
re-bound inside a loop from a closure that may hand its argument back, and a local bound at
two places from two sources.  A closure answering a view of a keyed field is no longer freed
under its caller — a regression this cycle had introduced.

**A function that returns text from an early `return` no longer leaks a buffer per call.**
`if !w { return lo(n) ?? "" }`, `return t[0][0]`, `return s.name` inside a loop or a nested
arm — each answered the right text and quietly left one String behind on the interpreter,
so a loop grew without bound while every value stayed correct.  Every `return` now delivers
through the same caller-owned buffer the function's tail already used.  The same census
found that on the compiled backend a function with a `&text` parameter of your own could
write its returned text INTO that parameter; it no longer can.

**A closure that answers a keyed collection, a `?` value or a tuple now carries the right
borrow fact to its caller.**  The bridge that turns a callee's parameter numbers into the
caller's variables handled four shapes and passed the rest through untranslated, so an `if`
choosing between such a call and a local could read a stale number as one of your
variables.  The bridge now covers every shape; the nightly invariant gate that had been
red on it is green.

**Four returns now do what the rules say.**  A `match` on a boolean that spells out `true`
and `false` no longer warns that its result might be null.  A function returning `vector<T>?`
that answers a field of its argument hands back a copy, so a later write to that field no
longer shows through the result.  A nullable record chosen by an `if`, or reassigned from a
call, is copied on the interpreter as it always was on the compiled backend.  And a lambda
declared `-> vector<T>?` or `-> S?` accepts a non-null tail, as a named function always did.

**`--lib` tells you when it lost.**  A project's own `lib/` is searched before a `--lib`
directory, so an override passed on the command line from inside such a project was
ignored without a word.  It still is — that order may well be right — but loft now says so,
naming the file that answered and the one the flag would have used.

**Tuples with text or collections behave the same from a lambda as from a named function.**
A lambda returning `(vector<T>, text)` used to hand its vector out as a view of the caller's
field; it now returns a copy, as a named function always did.  And an `if` that chooses
between such a function's result and a tuple you wrote inline compiles now, whichever arm is
which.  A nullable record answered by a lambda and chosen by an `if`, or reassigned, is
copied on the interpreter as it was on the compiled backend.

**The reference is now read end to end — 40 chapters of 40.** The Standard Library section
was the last and the worst: its generator read three of the seven `default/*.loft` files, so
the entire JSON and reflection API — `json_parse`, `to_json`, `json_errors`, `reflect_type`,
`stack_trace` — was absent from the published reference while the JSON chapter and the
feature catalogue documented it. Forty-three more entries shipped as a bare signature,
`sin`, `sqrt`, `floor` and `split` among them, because a blank line between a doc comment
and its declaration silently orphaned the doc. All 214 public functions are published now,
every one with its documentation, in four new sections.

Elsewhere the pass found a first-run page telling new users to pretend `.loft` is Rust while
the repository ships a VS Code extension and a language server; a roadmap calling a release
from four months earlier "current"; two comparison pages that told Rust and Python
programmers loft lacks features it has; and, on the pages that DO run their examples, cells
that could not fail — a debugger transcript whose expected output was already in the prompt
above it, and an `--explain` demonstration on a program with no diagnostics.

**The other thread is what a function may do to the values you hand it.** A whole-value
replacement of a parameter — `p = [...]` — is local to the callee; growing or writing
through it reaches the caller. That rule held for one spelling out of six, and several of
the rest answered one thing interpreted and the opposite thing compiled — a divergence the
ownership rules say cannot happen. They agree now: a heap parameter's rebind is local
however it is written, a keyed parameter's too, a `&` write-back reaches every heap kind
rather than the two it was written for, and a rebind written inside a CLOSURE — which could
reach past two frames and silently replace a caller's collection — is refused, because a
capture has no route back to the binding it would have to rebind.

### Writing `null` into a record or a collection now says so, like a number already did

```loft
struct Row { v: integer }
fn find(k: integer) -> Row { if k > 0 { return null; } Row { v: 1 } }
```

> warning: `null` is stored into the return value of the non-null type `Row` — the slot
> holds null; declare it `Row?` to make that explicit

The reference has always said you cannot store a `null` into a plain `integer`, `text` **or
`Row`**, and for the scalars the compiler said so. For a record, a collection or an enum it
said nothing — at a field, a return, a vector element and a call argument alike — so a
function could hand back a value whose type promised it was there, and the null travelled on
with nothing to read. It is the same notice the scalars get, and like theirs it is a warning:
your program still runs, and the fix is the `?` the message names.

### Copying a value no longer depends on whether it can be empty

```loft
a: vector<integer>? = [41, 42];
b = a;          // b is its own vector
a[1] = 99;      // b[1] is still 42
```

Giving one variable the value of another COPIES it — that is what `b = a` has always meant,
and writing to `a` afterwards does not reach `b`. Unless `a` could be `null`: then `b` was
the *same* vector, and every later write to either one showed up in the other. The same for a
record. Keyed collections (`hash`, `sorted`, `index`) were unaffected, which is what made it
hard to see — the rule held everywhere you were likely to check it.

Absence survives the copy too, which is the other half: if `a` is `null`, `b` is `null`, not
an empty vector. Those are different values, and a copy that quietly turned one into the
other would have been its own bug.

If you relied on the old behaviour to share a value, `&` is how you say so — `b = &a` links
the two, and always did.

### A linked list can say its last link is empty

```loft
struct Node { value: integer, next: reference<Node>? }   // now compiles
```

`reference<T>` is loft's shared pointer between records, and until now it could not be
written `reference<T>?`. On a type that points back at itself — a list, a tree, anything with
a terminator — that spelling did not compile at all, so the last link had to be a `null` in a
field whose type said it could not be one. Every linked structure in the language was written
that way because there was nothing else to write.

The `?` had been quietly turning the pointer into a *copy*. `reference<Leaf>?` and `Leaf?`
were laid out identically, so on a type where it did compile, writing the `?` gave you a
private copy of the record instead of a link to it — the same program printed `11` where a
pointer prints `22`, and nothing said the sharing was gone. A `?` on a pointer now means only
what it says: the pointer may be absent. It keeps its own bytes, `&pool[i]` still binds it, and
a write through the record is still seen through the field.

Two smaller things fell out. `&` in a struct literal works wherever the field's type asks for
it, not only when that field is written last — `Trail { link: &pool[0], id: 7 }` was rejected
for the comma. And the null-into-a-record warning above now reaches the linked-list terminator
too, naming `reference<Node>?` — the field's own type, where it used to name `Node?`, which is a
different type and, on a self-referencing struct, not one you can write.

### A `for` loop over your own iterator yields every kind of item

```loft
fn next(self: Reader) -> Line? { ... }    // Line, text, vector<T>, an enum — any type
for line in reader { use(line); }
```

A custom iterator whose `next` answered anything heap-carried — a struct, `text`, a
collection, a struct-enum — aborted the compiler outright, and the `server` package's own
documented `for req in srv` was one of them. Where it did compile, two item types ended the
loop before its first turn and exited cleanly with no output: the loop asked whether the
item was *falsy* rather than whether it was *null*, so a `boolean` iterator stopped on its
first `false`. It asks one question now, for every item type — the same one `x == null`
asks — so a `0`, an `""` and a `false` are ordinary elements and the loop hands them to you.

### A JSON field that is not a boolean answers null

```loft
cfg = json_parse(raw);
on = cfg.field("enabled").as_bool();   // null when it is absent, a number, or "true"
if on ?? false { start(); }
```

`as_bool` is `boolean?` now. It answered `false` for every mismatching kind — an absent
field, a number, the string `"true"` — which cannot be told from a field that really says
`false`. Its three siblings already answered null; a two-state boolean simply had nowhere to
put one until the return type could carry it.

### Two generic functions may both call their type variable `T`

```loft
fn largest<T: Ordered>(a: T, b: T) -> T { if a > b { a } else { b } }
fn total<T: Addable>(a: T, b: T) -> T { a + b }
```

Both compile. A header introduces its own type variable; before, one placeholder stood for
every `T` in the program and the second header's calls were checked against a parameter list
its author never wrote — order-dependently, so which one broke depended on which came first.

### A bound gives exactly the operators it declares

`a - b` under `<T: Numeric>` compiled and computed `-a`, discarding the second operand on
both backends with no diagnostic: `-` is one name at two arities, and the bound's unary
negation answered for the binary spelling. Bound satisfaction compares the whole signature
now, so the binary form is refused and you write the subtraction at a concrete type.

### Piping loft into `head` or `less` no longer aborts

A closed pipe is the normal end of `prog | head`. It used to panic on `EPIPE`, and when
stderr shared the pipe the panic printer failed too and the process aborted with a crash
report naming an interpreter opcode and a stdlib line — a false trail for the most ordinary
shell idiom there is.

### Smaller things you may notice

- A tuple with a heap member is a value: `u = t` copies the member, `(s, 5)` copies a struct
  member in, and `a = t.0` / `(a, b) = t` copy a collection member out of a tuple you own —
  on both backends.  A struct member read out is still a view, as a struct field is; and the
  `lost-write` warning no longer fires on a write through that view.
- A struct or vector yielded from a generator's LOOP body compiles on `--native`, and the
  consumer reads the value as it was at the yield; the statements after the loop run too
  (they were silently dropped).
- `yield from` a record generator inside an `if` no longer reports a wrong free on the
  interpreter.
- A capturing closure in a collection is refused with the struct-field route named, not a
  plan that never landed: a collection takes non-capturing lambdas, a struct field holds a
  capturing one.
- `loft check` prints `ok`, not an absolute path and an internal cache entry.
- `yield from` passes its arguments, and a parameterised sub-generator compiles on
  `--native`.
- An `i32` that overflows reads as `null`, like a plain `integer`; the reference now states
  which widths can and which answer their type's default, and why.
- A format hole holding an escaped quote — `"{shout("a\"b")}"` — no longer ends the
  enclosing item early.
- A `par` worker over nested vectors is handed its own row rather than every second one.
- A walker over a self-referencing record — `cur: Node? = a; while cur != null { cur =
  cur.next }` — no longer leaks the copy it starts from, and copying a value into a variable
  that currently views another record no longer writes into that record on `--native`.
- A value handed back through a nullable return (`-> S?`) that is a view of a local is
  copied before it leaves the function; it used to hand back a record the function had
  already released.
- On Windows, a `log.conf` `[levels]` key ending in `/` raises a file's level as documented,
  and `loft check`'s machine-readable line names the source in the spelling the live host
  compares against.
- Every public type in the distribution now has a description on its API page — a third of
  the structs had none, `Stage`, `Canvas` and `Server` among them, each named in dozens of
  signatures with nothing saying what it was. And the reference pages no longer end a
  function's description with a bookkeeping tag (`Example: @RND-001`, `@PLN110`, `loft#…`):
  those were notes to the maintainers, and a reader could open none of them.
- The reference had been warning you off four things that work. The parser library reads
  index expressions, `??` and format literals; production mode logs and continues on the
  compiled backend too, not only interpreted; a text element of a value tuple parameter can
  be written on `--native`; and a generic's result may be used inline without leaking a
  record. Each was a limitation that had since been fixed, with the warning left behind.
- Appending a payload-less variant of a struct-enum to a vector — `xs += [V.Null]` where
  `V` also has variants with fields — no longer leaks a record per site. Spelled `[Null]`
  it never did; the two spellings were built by two copies of one routine.
- A tuple carrying text can be handed to an `if` binding and the original still read
  afterwards on `--native` — `t = if c { pair } else { (0, "z") }; pair.1` used to refuse
  to compile, because the arm had moved the value.
- `scripts/install.sh` — the `curl | sh` path — installs the whole bundle now: the README,
  the examples and the reference PDF beside `bin/` and `default/`, which is also what
  `loft self-update` installs. It used to copy the runtime only and then hand
  `loft verify-self` the manifest of the full bundle, so every installation it made ended
  in *the installation does not verify*. A test now runs the script against a bundle built
  the way a release builds one.

---

## 2026-08

The **heap-correctness** release. Almost everything here is one theme seen from a
dozen angles: a value that outlived the storage it pointed into, or storage that
outlived the value. Closures that build structs, elements taken out of what a
function just returned, keyed collections that drop entries, captures that changed
meaning with declaration order — all of them read correctly once and wrongly later,
which is the shape that cannot be found by reading the code. They are fixed, and the
detectors that would have caught them earlier are now part of the nightly gate.

Alongside that: a store can give its file back (`store_reclaim`, plus automatic
compaction at load), `reserve(v, n)` for vectors you know the size of, a crash report
that survives being piped somewhere, and a `u32` local and field that finally agree
(the top of the range stays reserved as the null sentinel, by design).

Late in the cycle, three more: **`loft install` no longer lets a package's own manifest
decide where on disk it lands** (a name like `../../escaped` wrote outside `~/.loft/lib`
entirely); an **`i32` local now stays inside `i32`** when `+=` pushes it past the top, the
way `u8` and `i16` already did; and **generics work inside tuples** — a `T?` element, a
defaulted `T? = null` reaching a tuple, and a plain `-> (text?, integer)` return all
compiled to the wrong thing or refused to compile at all, on one backend or both.

### Appending to a keyed field you never constructed works

```loft
struct N { h: hash<E[k]>? }

n = N { };          // the field is absent, not empty
n.h += rows;        // crashed the program
```

An absent collection field and an empty one are the same thing to an append — there are no
records either way, and the append builds the collection. The vector side had always read it
that way; the keyed side followed the "no collection here" marker as if it were a record, and
the first lookup took the program down. `hash`, `sorted` and `index` all did it, on both
backends. Constructing the field empty (`N { h: [] }`) was the workaround and is no longer
needed.

### `x ?? d += e` now says what is wrong with it

`?` and `??` build the same thing internally and mean different things on the left of an
assignment. `x?` names one place and says what to read when it is null; `x ?? d` names two
values and no place at all, so there is nothing to write through — on the null path it hands
back a fresh default that the write lands in and nothing can read back.

Only the first was handled. The second reached the machinery below it and each type answered
its own way, none of them right:

```loft
g.data ?? [] += [Ec { k: 7 }];   // the field appended to ITSELF — one record became four,
                                 // and the keyed sibling of its group never saw the record
b.d ?? [] += [Ec { k: 9 }];      // null field: the write simply disappeared
b.n ?? 0 += 1;                   // "Not implemented operation + for type integer"
b.t ?? "" += "cd";               // internal compiler error
```

All four are now one message that names the two spellings that do work — `x += …`, or
`x? += …` when the read needs discharging.

### `const` holds through a `?` inside the place

A `const` parameter could be modified after all, if the write went through a nullable field on
the way:

```loft
fn touch(h: const Holder) {
  h.inner?.x = 99;           // compiled, ran, and the CALLER saw 99
  h.inner.x  = 99;           // refused correctly — the same write, dense
}
```

The promise `const` makes is about the write's root, and a `?` does not change which binding a
write reaches — it changes what a read answers. The same gap made an ordinary write
unreachable from the other side: `a.i?.nm = "cd"` on a `text` member reported that *a
file-scope `NAME: text = …` is a CONSTANT*, about code nobody had written, while the `integer`
member beside it wrote through fine. Both are one missing case, on both backends.
### A `?` on a value no longer buys it past a rule the plain value obeys

Appending one element to a vector needs brackets — `v += [x]`, not `v += x` — because
`vector<vector<T>> += vector<T>` would otherwise be both "push one element" and "concatenate".
That rule applies to every element type. It was not applied to a nullable one:

```loft
struct D { c: vector<integer> }

i: integer = 9
d.c += i          // error: vector `+= elem` is ambiguous; use `+= [elem]`
n: integer? = 9
d.c += n          // …accepted
```

So the `?` spelling of a statement was *more* permissive than the plain one, which is backwards
for a marker whose whole job is to make you handle a value more carefully. Both now ask for the
brackets, and `d.c += [n]` does what you meant.

Nullability and bracketing stay two separate questions about the same line, and both still reach
you: the brackets are this message, and *"a nullable `integer?` is stored into … the non-null
type"* is the other.

### Every `+=` on a collection now either appends or says why not

`c += v` had a list of routes and no `else`, so a source matching none of them went one of two
ways, both quiet.

A **vector** had a catch-all — a route that writes the value as one element without ever
comparing its type — so an unrelated source was written raw:

```loft
struct D { c: vector<integer> }

d = D { c: [] }
f: float = 2.5
d.c += f          // len 1, and the element reads 4612811918334230528
```

That is the IEEE-754 bit pattern of `2.5` read back as an integer. A `boolean` stored `8705`, a
`text` took the process down inside the allocator, and appending a struct — or an integer to a
`vector<text>` — ended in a segfault. `--native` refused to compile any of them, so the two
backends disagreed about the same program.

A **keyed** collection had no catch-all, so the same source reached nothing at all and the
append simply vanished with `len` reading 0.

Both now report:

```
error: cannot append `float` to `vector<integer>` — a `+=` source must be one `integer`
       element written `[…]`, or a `vector<integer>` of them
```

### Appending a record to a keyed field no longer needs a literal

The same list of routes was incomplete in the other direction: a source the collection *can*
hold sometimes reached no route either, and was dropped in silence.

```loft
struct E { k: integer, n: integer }
struct D { h: hash<E[k]> }

d = D { h: [] }
e = E { k: 2, n: 8 }
d.h += e          // used to add nothing; len read 0
```

A keyed field took `d.h += E { k: 2, n: 8 }` and `d.h += [e]`, and a keyed *local* took the bare
`h += e` — three spellings of one question, and only one of them was wrong. All five keyed kinds
were affected.

**If you wrote that statement, check what it was doing for you.** Two collections over the same
element type are a linked *group* — two routes to a single record set — so appending to one
already puts the record in the other:

```loft
struct Counter { ordered: vector<Word>, by_text: hash<Word[text]> }

c.ordered += [w]      // this alone puts the record in BOTH; `c.by_text["apple"]` finds it
c.by_text += w        // a no-op before this release; now a SECOND append
```

Code that filled both members was relying on the second statement doing nothing, and it now adds
every record twice. The vector member is the only one that shows it — a hash keeps one entry per
key either way — so a test that checks the lookup will not notice while the vector's length has
doubled. The fix is to delete the redundant append; `examples/collections.loft` was written this
way and is corrected in this release. Two more of the same shape are fixed with it: appending a variant to a vector over
its enum (`b.items += Named { … }`) grew the vector by three instead of one and now asks for the
usual brackets, and appending a whole keyed collection to another of the same type wrote nothing
and now says so.

### `x? += …` accumulates from the type's zero

`?` on the left of an assignment says *which value to read when this place is null*; the write
still lands in the place. That is the accumulate-from-nothing idiom:

```loft
hits: integer? = null;
hits? += 1;                  // 1     — the `?` read the zero
misses: integer? = null;
misses += 1;                 // null  — no `?`, so the null propagates through `+`
```

It did not work. On a vector field the appended record was built into the destination and the
destination was then appended to itself, so a one-element field grew to four; on a null place
the write disappeared and the field stayed null; a keyed sibling of the vector was never
re-indexed; a `text` place crashed the compiler; and a scalar place was refused with a message
about the operator. All of it silent, and identical on both backends.

For a **collection** the `?` was always redundant — `b.d += [r]` on a null field already builds
the empty collection first — and that spelling was correct throughout. `(a ?? d) += e` stays
refused: it names two values and no place.

### A nullable inside a collection literal says so too

```loft
n: integer? = null;
v: vector<integer> = [n];    // was: silent — and `v[0]` read back null
```

Now it warns, the way `x: integer = n` and `d.c += n` already did. The gap mattered because it
was the cure the language names: `d.c += n` is refused with *"use `+= [n]`"*, and `+= [n]` was
the spelling nothing checked — so following our own advice took you from warned to silent. The
same check now covers a field assignment (`d.c = [n]`), a constructor field (`D { c: [n] }`) and
nested literals.

It is a warning and never an error, including for narrow element types like `u8` where a null
does not really fit: the store already compiled and ran, and refusing it now would break code
that works today. Write `[n?]` or `[n ?? 0]` to say what you mean.

### `c += null` now compiles with `--native`

Appending a bare `null` to a collection worked in the interpreter and had never compiled
natively — `rustc` rejected the generated code for `integer`, `float`, `single`, `character`,
narrow-int and struct element types. A published package (`arguments`) relied on it, so it built
on one backend only. Both backends now agree at every element type.

### Appending a nullable value to a collection says so, instead of crashing

```loft
struct D { c: vector<integer> }
s: vector<integer>? = [1, 2];
d.c += s;                    // was: interpreter panic; `--native` would not compile
```

Now it warns — *"a nullable `vector<integer>?` is stored into … the non-null type
`vector<integer>`"* — and appends, which is what the same value does when you write
`d.c = s`. A keyed field took the value silently before and now warns too. The warning names
the two cures it always had: `d.c += s?` or `d.c += s ?? []`. Appending a value that really is
null appends nothing.

### Appending to a nullable collection works

```loft
struct Bag { rows: vector<Row>?, by_id: hash<Row[id]>? }
b.rows += more;              // was: "No matching operator 'Add' on 'vector<Row>?'"
b.by_id += more;             // was: added nothing at all, and said nothing
```

A `vector<τ>?` or `hash<τ[k]>?` field takes the same append its dense twin does. The two
halves failed differently and both are fixed: the vector was refused outright, and the keyed
one was **silent** — the records vanished and `len` read 0 with no diagnostic anywhere. Only
a *non-literal* source was affected, so `b.rows += [r]` was correct all along, which is what
made the silent half easy to miss. Appending to a field that is actually absent builds the
empty collection first, as it always did for a dense one.

### Appending to a nullable text field works

```loft
struct Row { note: text? }
r.note += "cd";              // was: internal compiler error
```

One append to a `text?` struct field took the compiler down, on both backends. The dense
`text` field beside it and the `text?` local were always fine. Appending to a field that is
*actually* null still leaves it null — `+=` propagates — and `--native` now agrees with the
interpreter about that, which it did not while the crash was hiding the shape.

### A vector rebuilt from itself keeps its contents

The ordinary "drop the last element" idiom quietly produced an **empty** vector:

```loft
st: vector<integer> = [1, 2, 3, 4, 5];
st = [for q in 0..(st.len() - 1) { st[q] ?? 0 }];   // was []; now [1,2,3,4]
```

A comprehension builds a fresh vector and hands it over, so everything inside it — the
source, the range bound, the `if` guard, the body — should read what the variable held
when the line started.  It was reading the empty result being built instead.

The source did not even have to be the vector being assigned.  This kept the right
length and got every value wrong, which is the version that survives a test suite:

```loft
a = [7, 8, 9];
b = [1, 2, 3];
a = [for x in b { x + (a[0] ?? 0) }];   // was [1,3,4]; now [8,9,10]
```

It was found in a breadth-first search whose worklist used the pop idiom: the search
stopped after one expansion, so sets that really were connected were reported as
disconnected, and nothing anywhere threw.  A worklist holding a single item gives the
same answer either way, which is why small cases looked fine.

The same defect had two more faces, and both are fixed with it.  Assigned to a struct
**field** it reads, the comprehension emptied the field the same way.  Appended with
**`+=`** to a vector whose length it measured, it never finished at all — the loop's own
appends grew the length it was testing, so the program hung (and `--native` overflowed),
climbing in memory the whole time:

```loft
s.v = [for i in 0..s.v.len() { s.v[i] ?? 0 }];   // was []; now the elements
a  += [for i in 0..a.len()   { a[i]  ?? 0 }];    // hung; now appends a copy
```

`.map` and `.filter` onto their own receiver were correct all along, on all three —
`s.v = s.v.map(…)` and `a += a.map(…)` both did the right thing.  That is what the
comprehension now does too, so the two spellings of one operation finally agree.  The
temporary (`t = [for …]; a = t;`) was the workaround and still works.
### A width or an alignment now works on every kind of value

`"{name:>10}"` has always padded text and numbers.  On a **character**, a **vector**, a
**struct** or an enum that carries fields, the width was thrown away without a word:

```
c = 'x';
println("[{c:>5}]");     // printed [x]      — now prints [    x]

v = [1, 2];
println("[{v:>12}]");    // printed [[1,2]]  — now prints [       [1,2]]
```

The value was rendered correctly and then simply not padded, so a column of output that
looked aligned in a small test drifted as soon as the values differed in length.  A spec
now applies to whatever the value renders as, for every type.

Two details worth knowing.  Flags that choose *how* a value renders — `#`, and `:j` for
JSON — are unchanged, and combine with a width as you would expect.  And a **null
character renders as nothing**, so `"{c:>3}"` on one gives three spaces: a width pads
whatever the value rendered as, and nothing is still something to pad.

If you had worked around this by rendering to a text first (`t = "{v}"; "{t:>12}"`), that
still does exactly the same thing — you can keep it or drop it.

### A slice with a calculation in it can be wrapped in brackets

`(s[i + 1..])` was refused, with a message naming `OpAddInt` — an internal name — at a
caret pointing past the slice:

```
error: missing argument for parameter 'v2' of `OpAddInt`
```

Nothing in that said *slice*, *index*, or *brackets*, so it read as a problem with
whatever came after.  The same slice without the round brackets always worked, and so did
`(s[i..])` and `(s[2..])` — it needed brackets, a slice, and a **number** written right
before the `..` all at once, which is why it could sit in a file for a while looking like
a puzzle.

It compiles now.  If you hoisted the slice into its own variable to get around it, that is
still perfectly good code.

### A failed `assert` names the line it is on

If a function above it took a `const` (or a `&`) parameter it never modifies, every
message after that function pointed at the wrong line — always by the same amount, so
nothing looked odd:

```
error: assertion failed: repeat call, two params
  --> game.loft:177:1
177 |   assert(inner == 13, "by-value: callee sees its own writes");
```

The message belongs to line 184; line 177 is a different assertion, which is what made
the report so convincing.  The caret of a warning, the file and line in a runtime error,
and the line a failed `assert` prints all come from the same place, so all three moved
together.  They are right now, and a test file whose nineteen assertions had all been
reporting seven lines early reads correctly again.

### A function that picks between a fresh value and a local no longer piles up records

This shape is everywhere, and it was leaking one record on every call:

```loft
fn pick(c: boolean) -> Wind {
  w = prevailing(…);
  if c { Wind { … } } else { w }     // `w` on the arm that is NOT taken
}
```

The answer was always right, so nothing looked wrong — until the calls added up.  A
climate model calling it once per tile per season retained about 16,000 records per
planet, and a program holding four planets at once died with *"store table exhausted"*.
Now the record that loses the branch is released, and the count stays flat however
many times you call it.

Two neighbours of the same shape were answering **wrongly**, quietly, and are fixed with
it: a choice between *two* locals gave you a blank record instead of the first one
(`if c { u } else { w }` handed back an empty `u`), and writing the choice down before
returning it (`r = if c { … } else { w }; r`) blanked the fresh side the same way.  Both
gave the same wrong answer whether you ran interpreted or native, which is why they had
gone unnoticed.

If you had worked around any of this by making both arms build a fresh value, that code
is still correct — you can now simply return the local.

### A browser page can bring its own assets

A `--html` page has no disk, so a program that reads a pack could only get it over the
network — and that costs a gallery page the one thing it is for: being a single file you
can open.  Now the page carries what it reads:

```toml
[[embed]]
path = "assets/game.pack"
```

`loft --html` puts the file in the page, under the name the program reads it by.  The
same line of loft works either way:

```loft
ok = store_load(q, "assets/game.pack")   // from disk, and from inside the page
```

Add `source = "build/game.pack"` when the file is generated somewhere else — `path`
stays what the program passes.  Both are read from beside the **program**, the same
place loft looks when the program itself opens the file, so the two really are the same
file.  A library can declare its own files, and a consumer's page brings them along.

This is for what a page needs before its first fetch, and for a page that has to be one
self-contained file.  Bigger asset sets are still better served over HTTP, where only
the bytes a lookup touches cross the wire — and the page grows by about a third more
than the file, so the size `--html` reports is worth a look.

If a declared file is not there, or is named in a way the program could never ask for
(an absolute path, or `./assets/x` where the program says `assets/x`), the build stops
and says which — before it spends a minute compiling a page that would have drawn
nothing.

### A browser game can bring its own font

A game that draws text in the browser used to get whatever family the browser guessed
from the file name, and there was no way to say otherwise without hand-writing CSS.  Now
the font is a line in `loft.toml`:

```toml
[[font]]
family = "PressStart2P"
native = "fonts/PressStart2P.ttf"     # the file every other target uses
url    = "fonts/PressStart2P.woff2"   # what the page fetches
```

`--html` puts the `@font-face` in the page (or a `<link>`, if you name a provider's
stylesheet instead), and waits for the font to arrive before your program starts — a
webfont that turns up after the first frame is drawn is a fallback nobody was told about.
Declare only `family` and the page brings nothing: that is the case where the browser
already has the font.

The name has to match the path your program passes to `gl_load_font`, and the build now
**says so** instead of quietly drawing in the wrong face.  A font that still cannot be
found reports itself once in the browser console rather than silently substituting.

### A range-read no longer refuses a collection because one field is an enum

Reading one key out of a big store over HTTP range copies that record's fields into
your own store, and the check for *"can this field be moved?"* knew about structs and
about struct-enum variants but not about a plain `enum`. So a single `kind:` field made
the whole collection unreadable a key at a time — and the refusal blamed a
`vector<text>`, which the type did not have, sending you looking for the wrong thing.

An enum's value is a tag byte stored in the record, so there was never anything to
relocate. It reads now, variant and payload alike.

The refusal message also **names the field** it is actually about, rather than always
naming the same two types. A refusal you cannot act on is barely better than a silent
one.

### `store_load` cannot hang any more

Loading a store image compacts it, and compaction rebuilt the image into a fresh
store whose root block was recorded one word shorter than it actually owned. That
stray word became a block owning nothing, and the allocator walks a block chain by
stepping over each block's own size — so it stepped over nothing, for ever. The
symptom was a `store_load` that never came back: no error, no output, no bound of
its own, on a call the program had only asked to READ a file.

It needed a container of several keyed collections — an asset pack's shape — holding
values big enough for a later allocation to reach the linear scan, which is why it
had not been seen before.

Two things changed. The rebuild now records the block at the size it was actually
given, and the allocator's walk can no longer fail to advance: a malformed chain
says so on stderr and the store grows past it, so the worst case is a few leaked
words with a message rather than a program that stops responding.

### ⚠ Asking `git` a question outside a repository now FAILS instead of answering nothing

**This changes behaviour**, so it belongs here rather than being discovered: if you called
`lib/git` from a directory that is not a git repository, every query used to answer *empty* —
no commits, no changed files, no diff. It now halts with

```
git: not a git repository: /path/you/asked/about
```

The old answer was indistinguishable from a real one. "This repository has no commits" and
"this is not a repository" arrived as the same empty vector, so a program that branched on
*"nothing changed, nothing to do"* looked correct while asking a question that could not be
answered at all. That is the failure the language calls **silent-wrong**, and it is the one
kind of bug a green test run cannot see.

**If you relied on the old behaviour**, check for a repository before asking, rather than
reading an empty answer as one.

⚠ An *empty repository* is unchanged and still answers empty — that is a real answer to a
real question. Only "cannot be asked at all" became a fault.

Why now: adding a fault is a **one-way door**. loft's compatibility promise lets the error
surface only ever *shrink* after the contract freeze, so a place where loft is too permissive
— silently accepting something dubious, or handing back a plausible-wrong value where it
should refuse — is a last chance to add the error. Afterwards it could only be loosened, never
tightened. See [COMPATIBILITY.md § The error surface is one-directional](doc/claude/COMPATIBILITY.md).

### A failed `assert` reads the same way whichever backend ran it

```
$ loft p.loft            # the default backend, before
thread '<unnamed>' (2466378) panicked at /tmp/loft_native_2466316.rs:966:18:
p.loft:1 plain assert
```

That names a Rust file in the temporary directory that loft generated and you have never
seen. The same program under `--interpret` named *your* source. Now both print the loft
diagnostic — and both print the functions the fault happened inside, which neither did
before:

```
error: assertion failed: n was 9
  --> game.loft:12:1
  |
12|     assert(n < 5, "n was {n}");
  | ^
  in fn inner() ← called from
        fn middle()
        fn main()
```

Inside a `par` worker the frames are the worker's own, and the halt is reported once
however many workers hit it at the same moment.

### A `par` worker no longer ignores what you wrote as its first argument

The first argument of a `par` worker is the loop element — the loop hands each element to
the worker itself. Writing anything else there used to be accepted and quietly replaced:

```loft
for a in rows par(b = takes_int(a.n), 2) { println("b={b}"); }
```

That ran `takes_int(a)` — the whole record, reinterpreted as an `integer` — so a
`Sq { tag, n }` element answered `tag * 100` and never read `n`. Move `n` to the front of
the struct and the same program answered differently. `f(5)` and `f(other)` were replaced
the same way.

All of these are now compile errors that say what to write instead, including a worker
whose first parameter cannot take the element at all — the check the ordinary
`b = takes_int(a)` has always made. The documented form is unchanged: pass the element
first, read what you need inside the worker, and pass extra context after it.

### Runaway recursion stops at the same place on both backends

A program that recurses without end is stopped at 10 000 stack frames. Which frame that
is no longer depends on which backend ran it: the default backend allowed one call fewer
than `--interpret`, so a program that recursed almost that deep could print its answer
under one and die under the other.

The report reads the same way now too — the loft diagnostic, naming the function that was
running when the stack filled up:

```
error: call stack overflow — exceeded 10000 stack frames
  --> game.loft:3:1
  |
3 | fn walk(room: Room) -> integer {
  | ^
  in fn walk() ← called from
        fn walk()
        … (9995 more frames)
```

The default backend used to print its own shorter line with no source position, no caret
and a different frame layout.

### Less memory for a variable reassigned in several branches

```loft
fn classify(k: integer) -> integer {
  total = 0;
  z = [k, k];
  if k == 0 { z = [1, k]; total += z[0]; }
  else if k == 1 { z = [3, k]; total += z[0]; }
  else { z = [k, k]; total += z[1]; }
  total
}
```

Each branch built its own storage, and all of it stayed alive until the function returned —
even though only one branch ever runs. The cost grew with the number of branches you wrote,
not with the work done: sixteen branches held twenty stores for one taken arm. Now each
branch releases its own, and the count stays flat however many you add.

Nothing about your program changes except how much memory it holds while running. A
variable still read *after* the branches keeps the old behaviour, deliberately — releasing
early there would hand back the wrong value on the branch that did not run.

### An enum may be called `T`

```loft
fn main() { d = T.N; print("{match d { N => 1, S => 2 }}\n"); }
enum T { N, S }
```

This reported `Expect token ;`, pointing at a semicolon whose syntax is fine. `T` is the
name the standard library uses for a generic type parameter, and that internal placeholder
was sitting in the same namespace your own types live in — so the name was quietly taken.
It is now invisible outside the library that declares it, which is what it always claimed
to be.

Your own generics are unaffected: `fn pick<T>(a: T, b: T) -> T` still works, in a file that
also declares an `enum T` or in one that does not. Writing both in the SAME file is still
refused, because loft has one namespace for names — `pick a different name` says so.

### Enums and their variants work before they are declared

`loft` files are meant to read in whatever order suits them — that is what the two-pass
parser is for. Enums did not cooperate. All three of these were refused, and all three
work now:

```loft
fn main() {
  p = Priority.High;
  r = match p { High => 10, Low => 20 };   // "cannot change type from void to integer"

  s = Circle { r: 2 };                     // "unknown type 'Circle'"

  d = D.N;                                 // "Unknown variable 'D'"
}
enum Priority { High, Low }
enum Shape { Circle { r: integer }, Square { w: integer } }
enum D { N, S }
```

Each had a different cause and the same shape: the first pass had to guess something the
second pass already knew. `match` gave up and called the result `void`. A variant literal
left a placeholder that the enum declaration never claimed, though a plain `struct` in the
same position always worked. And a one-letter name was assumed to be a mistyped constant,
because the rule that keeps `N` reporting as an unknown *variable* looks for a lowercase
letter in the name — so `enum D` had none, while `enum Dx` was fine.

Naming an enum `T` still does not work, and that one is not about order: `T` is the name
the standard library uses for a generic type parameter. Pick another name.

### Arithmetic on a function declared further down the file

A function may be used above where it is written — that is what the two-pass parser
is for.  But mixing one with a plain number did not work:

```loft
fn run() -> float {
  a = f() - 1;        // was: Variable 'a' cannot change type from integer to float
  a
}
fn f() -> float { 4.5 }
```

The first pass saw `unknown - 1`, decided from the `1` that this was integer
arithmetic, and wrote that down.  The second pass found out `f()` returns a float,
and the assignment was refused.  Moving `f` above `run` fixed it, which is the tell:
declaration order was deciding a type.  Writing the type down yourself did not help
either — `a: float = f() - 1` was refused too, with the message reversed.

Now the type comes from the operand that is really there, whichever side it is on
and whichever operator you use.  A genuine mismatch is still reported, and still
names the real type: `f() < true` says *"No matching operator '<' on 'float' and
'boolean'"*.

### Leaving out an argument that defaults to `null` gives you the type's zero

```loft
fn f(a: integer? = null) -> integer { a? }
f();                  // was 65535 on the interpreter — now 0
```

`float?` was similarly wrong (a denormal), and a `boolean?` parameter of this shape stopped
`--native` compiling the program at all. Passing the argument explicitly, and the same thing
written as a local, were always correct — it was only the omitted-default path.

`character?` had two problems of its own, now fixed with it — see below.

### An omitted `character?` argument, and a `character?` discharge on `--native`

`character`'s null is codepoint 0 — the same `'\0'` its default is. Three places in loft agreed
on that and two did not, so `a == null` on an omitted `character? = null` answered `true` on the
interpreter and `false` on `--native`:

```loft
fn f(a: character? = null) -> boolean { a == null }
f();                  // was false on --native — now true on both
```

Separately, discharging one with `?` would not compile on `--native` at all — in any form,
including inside a comparison:

```loft
fn g(a: character? = null) -> boolean { (a?) == '\0' }
```

Note the one thing that has NOT changed: a `character` holding `'\0'` reads as null, because
that codepoint IS the reserved sentinel. `'\0' as integer` therefore answers `null` rather than
`0`, on both backends — the documented cost of storing the null inside the value.

### A generic `T? = null` parameter you leave out gives you `T`'s zero

The same shape as the first entry, once the type is a type variable:

```loft
pub fn g<T>(v: vector<T>, a: T? = null) -> T { a? }
g([1]);               // was 34359738369 — now 0
g([1.5]);             // was a denormal  — now 0.0
g(["q"]);             // was a crash     — now ""
```

A record `T` now gets its own field defaults, where before it got an empty record of the type
VARIABLE — a value of no type at all, which also leaked.

### Returning a generic's discharged `T?` is safe, and stops growing the heap

```loft
pub fn g<T>(x: T, a: T?) -> T { a? }
g("q", "z");          // was a crash on a poisoned arena, and leaked 8 bytes every call
```

At `T = text` this answered correctly and then handed the caller memory it had already
written off — invisible on an ordinary run, a crash under the arena poison detector. It also
kept the text it returned, once per call, so a loop over it grew without bound.

Both came from the same place: a generic instantiated at a concrete type was compiled by a
different route than the identical function written out by hand. It now takes the same one,
so it returns its text through the caller's buffer like every other text-returning function.
`T = integer` and a struct `T` were never affected.

### A generic function can be a parallel worker

```loft
pub fn idf<T>(x: T) -> T { x }
for e in [1,2,3] par(r = idf(e), 1) { … }   // was "'idf' is not a function"
```

The name resolved everywhere else in the same file; only the `par(...)` position refused
it, because that path looked the function up and never instantiated it. Workers over
integers, floats and booleans all work, and one generic worker can serve several element
types in the same program.

Still out of reach for now, and it says so instead of miscompiling: a generic worker
called from inside a *generic function*.

### A tuple with a nullable element can be a declared local

```loft
c: (text?, integer) = ("c0", 3);   // was refused — now works
d: (text?, integer) = (null, 3);   // and the null element really is null
```

The same tuple type was already accepted as a function's return type, so this only ever
affected the annotated-local spelling. A `null` element now becomes that element's own
null, where before it quietly became the empty text and would not compile at all with
`--native`.

### A `limit(...)` range is stored the way it is declared, and reads back what you wrote

```loft
v: vector<integer limit(10,255)> = [12, 200];
v[0];                       // was 22 — the element read back exactly `lo` too high
"{v}";                      // was a row of huge negative numbers — now [12,200]
```

A range-annotated integer stores in the smallest width that holds its range, and a
collection's element stride is that width — but only the `u8`-style spelling was narrowing
the STORAGE, while both spellings narrowed the READ. So a `vector<integer limit(10,255)>`
kept 8-byte elements that were read one byte at a time, and every element came back `lo`
too high. It disappeared at `lo == 0`, which is why the common spellings looked fine, and a
struct field of the identical type was always correct.

The two spellings are now one layout, pinned by the golden layout test — which could only
see the `u8` spelling before, and that is what let them drift apart.

### A bound too wide to store is refused instead of quietly shrinking

```loft
c: integer limit(0,5000000000) = 4999999995;   // was 0 — now a compile error
```

`limit(0, 5000000000)` accepted the declaration and then could not hold a value inside its
own declared range: the bound was silently truncated to 705032704, and every value above
that became the range's low end. `0` is an ordinary value of the type, and it is also what
a genuine out-of-range write produces, so nothing at the read could tell you which had
happened. Bounds that fit are untouched, and the error names the widest one that does.

### A null in a narrow slot prints as null

```loft
v: vector<u16?> = [300, null];
"{v}";                      // was [300,-2147483648] — now [300,null]
n: vector<i16?> = [-300];
"{n}";                      // was [-301] — a PRESENT value, one too low
```

Reading one element answered `null`; rendering the whole collection printed the slot's raw
sentinel as a plausible number. And a nullable SIGNED narrow slot sacrifices its bottom
value to make room for that sentinel, so its present values printed one too low — as a
field and as an element, on both backends. Both halves came from the same place: the render
re-derived "is this null?" instead of asking, and the schema was registered with a
different offset than the one the reads and writes use.

### Concatenating two vectors that store their elements differently is refused

```loft
u: vector<u8> = [1, 250];
w: vector<integer> = [7, 8];
u + w;                      // was [1,250,7,0] — now a compile error naming the cure
```

A concatenation copies element BYTES, so it cannot re-encode: `u8` and `integer` are both
"integer" to the type checker, and the copy put 8-byte values into 1-byte slots. Same-width
mixes were wrong too — `vector<u8> + vector<i8>` turned `[-5, 5]` into `[123, 133]`, because
the two encodings count from different places. Append element by element instead, which
converts each value; a narrowing step takes the checked cast loft asks for everywhere else.

### A qualified enum variant works as a value

```loft
f = std::Format.NotExists;  // was "Unknown variable 'std'"
```

`std::abs(-5)`, `lib::CONST`, `lib::Struct { … }` and `f: std::Format = NotExists` all
resolved; only the VALUE spelling of a variant did not, and the error named the library
rather than the enum. It bit hardest under `use lib as a;`, where the qualifier is the
whole point.

### A `text?` crosses into a generator, and out of a tuple, on `--native`

```loft
fn h(a: text?) -> iterator<text> { yield "x"; }   // did not compile natively
f: (text?, integer) = (null, 3);
f.0 == null; f.0 ?? "N";                          // did not compile natively
```

Both backends now build both. A nullable text is stored exactly like a text — the absence
is a sentinel in the same bytes — but a dozen places in the native backend asked "is this
text?" without looking through the `?`, so a `text?` was treated as a scalar: a generator's
captured parameter was filled with the wrong Rust type and moved out of itself, and reading
a tuple element CONSUMED it, so a null test followed by a read would not compile. Each
group of sites now shares one answer.

### A range you declare is enforced however you spell it

```loft
l: integer limit(0,255) = 250;  l += 10;   // was 260 — now 0, like the u8 spelling
w.f = 5;  w.f -= 10;            // f: u32     was 4294967291 — now 0, like the local
```

`u8` and `integer limit(0,255)` name the same range, so they now bound a `+=` the same way;
before, only the `u8` spelling did. And a `u32` or `i32` **field** wrapped around where the
local beside it stopped at the edge — the two now agree, and so does a vector element.

Four-byte ranges were the gap in both directions, so `integer limit(0,70000)` was wrong
everywhere and is fixed too. Plain `integer` is unchanged: it declares no range and still
counts past four bytes.

### A generator can be called before it is written, and a generic can return one

```loft
fn main() { for y in count() { … } }      // was a crash — now runs
fn count() -> iterator<integer> { yield 1; yield 2; }

fn each<T>(v: vector<T>) -> iterator<T> { for e in v { yield e; } }
for y in each([1, 2, 3]) { … }            // was a crash; y was unusable — now an integer
```

Three separate things, all reached from one report.

A generator **called above its own declaration** crashed the interpreter. The call site
records where to write the function's address once the body has been compiled, and the
arithmetic that found that spot assumed the instruction was one byte wide. Generator calls
use one of the wide instructions, so the address landed a byte early, on top of the frame
size — which then read as tens of kilobytes, and the interpreter subtracted it from a much
smaller number. Declaration order is not supposed to matter, and now it does not.

A generator **given a list literal** — `each([1, 2, 3])` — did not compile with `--native`.
Neither needed a generic to go wrong.

And a **generic returning `iterator<T>`** never learned what `T` was, so its loop variable
stayed abstract: it could not be added up or put in a message, `--native` refused the
program, and one generic iterating another's generator corrupted the heap. `vector<T>`,
`(T, T)` and `T?` returns already worked; `iterator<T>` now joins them.

Yielding a struct or a vector from inside a generator's loop is still `--native`'s one
remaining gap here, and says so.

### An accessor that sometimes borrows and sometimes builds is safe on `--native`

```loft
fn view_at(self: const Stage, i: integer) -> View {
    if i < 0 or i >= len(self.views) { return View { … }; }   // builds one
    self.views[i] ?? View { … }                                // hands one back
}
```

Written as a METHOD and called in a loop, every read after the first out-of-range one answered
zeros on `--native` — the receiver's records had been freed and the slot reused. Writing the
same body as a plain function was always correct, which is what hid it. Both spellings agree now.

### Leaving out a nullable record argument no longer grows the heap

```loft
fn f(a: P? = null) -> P { a? }
for i in 0..1000 { b = f(); … }     // leaked one record per call
```

Passing the argument, and passing a variable that happens to be null, were both always fine — it
was only the omitted spelling.

### `sum(v)` — the identity is optional now

```loft
sum([10, 20, 12])      // 42   — the element type's own zero starts it
sum([10, 20, 12], 0)   // 42   — unchanged
sum([1.5, 2.5])        // 4.0  — and for float that zero is 0.0
```

This is also what makes `loft fix` able to apply the one rewrite it offers on `sum_of`:
`sum_of(v)` becomes `sum(v)`, and the fix now verifies instead of being rejected for a missing
argument. Every existing `sum(v, init)` call is unchanged.

### A `??` default that calls a function no longer leaks

```loft
m = v[i] ?? mk();     // one leaked record per index MISS, unbounded in a loop
```

A struct **literal** default was always fine, and the compiler's refusal of a struct-valued
constant points you at the function spelling — so the leaking form was the recommended one.
Fixed for both backends; the hit path and the literal default are unchanged.
`character?` still has this problem and is tracked separately.

### `loft fix` can repair a text slice that stops short

`s[i..len(s)]` looks like "to the end" and is not — `len` counts characters while a slice bound
is a byte offset, so it truncates any text with an accented character. loft warned about it;
now `loft fix` can also repair it, by taking the bound off entirely:

```loft
t = s[0..len(s)];     // "héllo" -> "héll"
t = s[0..];           // what loft fix writes -> "héllo"
```

Works for `s.len()` as well as `len(s)`.

### Naming a `both` method where a value goes now tells you what is wrong

Handing a method to `map`, `filter` or a `fn(...)` parameter does not work — a method is not a
function value — and loft says so. It said so only for `self` methods. A `both` one was silent:

```loft
fn tripled(both: P) -> integer { both.n * 3 }
x = tripled;                      // was: bound null, no message at all
apply(tripled, p);                // was: "expected fn(P) -> integer, got null"
```

Both spellings now give the same message, naming the receiver type and the two cures (wrap it
in a lambda, or declare it with a plain first-parameter name).

### `loft verify-self` tells you when it could not check anything

On an install built from source there is no release manifest to compare against. The command
said so — and exited **0**, which is the same answer it gives when everything verified. Anyone
who wired it into a script (`loft verify-self && deploy`) got a green light from a check that
never ran.

It now exits `2` for "could not verify", keeping `0` for verified-and-intact and `1` for
verified-and-wrong.

### Proximity queries on a `spatial` collection cover every direction

`xs[(x,y)..]` and `xs[(x,y)..:n]` are documented as walking **outward** from a point. They
walked *onward* instead — along the collection's internal order — so half the neighbourhood
was unreachable, and what you got depended on where your query happened to sit:

```loft
for m in s[(20, 20)..:3] { … }    // asked for 3, got 1
for m in s[(99, 99)..:3] { … }    // asked for 3, got nothing at all
```

Worse, the nearest thing could be invisible: from `(12, 11)` a mob two steps away was never
returned while one twelve steps away was. Both forms now walk outward, so `..:n` answers `n`
records from any query point and the nearest ones come first. The walk is approximate — it
orders by the collection's space-filling curve, so a very close point can occasionally arrive
a place late; `xs[(x1,y1)..(x2,y2)]` is still the exact form when you need a guarantee.

### `filter` no longer breaks the collection it read

Filtering a vector whose elements are themselves vectors quietly damaged the source. It kept
its length and its contents still read back, but any later loop over it ran zero times:

```loft
nv = [[1], [2]];
f = filter(nv, |x| { true });
c = [for x in nv { 1 }];          // was: 0 elements, not 2
```

### `loft test` runs your tests, not every function in the file

A file that names any `test_*` function now runs exactly those. A `setup` helper no longer
runs (and can no longer fail your suite from an `assert` inside it), and a `main` no longer
executes during a test run with whatever it prints or writes. A file with no `test_*` at all
is unchanged — every zero-argument function still runs, which is what makes `--tests` usable
on a plain script.

If you had given a function an unused parameter just to keep it out of the run, you can drop
that now.

**And on `--native`, tests in a file with a `main` were being reported as passing without
being run** — a failing test could show green there and red on the interpreter. They run.

### `loft fix` can act on warnings, not just errors

`loft fix` only ever applied fixes attached to errors. Four more now carry a real edit it can
apply and verify: dropping a needless `&` or `const` on a parameter, replacing an empty `{}`
with `[]`, and deleting a deprecated `not null`. The `&` and `const` notices also point at the
token itself now, instead of at a spot inside the function body.

### Passing a tuple containing text now compiles with `--native`

```loft
fn take(p: (integer, text)) -> integer { p.0 }
a: (integer, text) = (3, "three");
take(a);                          // was: --native refused the whole program
```

The same tuple written directly at the call site always worked, which made this an easy one to
trip over. Nested tuples (`((integer, text), text)`) are fixed too.

### `map`, `filter` and friends work on a `value struct` vector

Every vector builtin that walks to the end — `map`, `filter`, `reduce`, `all`, `count_if`,
and the `[for x in v { … }]` comprehension — **hung forever** when the element was a
`value struct`. `any` did stop, but by reading one element past the end and answering from
it, so it could report `true` where the answer is `false`.

```loft
value struct V { x: integer }
vs = [V{x:1}, V{x:2}];
m = map(vs, |v| { v.x * 10 });    // never returned
```

The same code with a plain `struct` was always fine, and so was a `for` statement over the
same vector — which was the workaround. All of them now agree: these loops end after
exactly `len(v)` elements, whatever the elements are.

### `break` naming the wrong variable now tells you, instead of crashing the compiler

`x#break` and `x#continue` leave a loop by naming the variable that loop binds. Naming
something else that happens to be a declared local — easy to do, since it looks like any
other name — crashed the compiler with an internal error and a nonsense index:

```loft
k = 0;
for j in 1..=3 { if j == 2 { k#break } }   // was: internal compiler error
```

It now says `k` is not a loop variable, and lists what you can write instead — a plain
`break`, or the enclosing loops by name, innermost first.

### A library can import itself again

A package whose own test file is named after the package — `tests/hex_world.loft` saying
`use hex_world;`, which is how essentially every loft library is laid out — stopped
seeing its own functions in this cycle. The import bound the test file rather than the
package, so the library's entry never loaded and every call read *"Unknown function"*.

`use <name>` inside the package called `<name>` means the package. Nothing you write needs
to change; if you hit this, it is fixed.

### A type with `OpIndex` can be subscripted with more than one index

Giving your type `OpIndex` lets it be subscripted like a built-in collection — and the
documented motivating case is a matrix, which wants `m[row, col]`. That spelling was a
parse error (`Expect token ]`, pointing at the comma), even though the two-index method
was accepted and `OpIndex(m, row, col)` worked. So you could write the method the feature
is for and never reach it with brackets.

```loft
fn OpIndex(self: Mat, r: integer, c: integer) -> integer { … }
m[1, 2]        // was: error: Expect token ]
```

However many indices your `OpIndex` declares, that is what the brackets take. Getting the
count wrong now says so against your own signature, naming the method as you wrote it.

A slice, `x[a..b]`, still does not work on your own type — there is no range value in loft
for a method to receive, so it needs more than a parser change. It now says that, and what
to write instead, rather than `Expect token ]`.

### A built-in with no native implementation now fails the build instead of doing nothing

`loft --native` already refused to compile a program calling a built-in that has no native
implementation, naming the function — but only when that function returned something. One
returning nothing was emitted as an empty body: it compiled, it was called, and it did
nothing at all, on `--native` only. That is how a parallel loop with an empty body came to
run no workers for as long as it did.

Both now fail the same way. Nothing you can write in loft changes behaviour here — every
built-in that looked implemented still is; the one deliberate do-nothing on this target,
`yield_frame`, says so in its own declaration and keeps working.

### Reflection reports what a field was declared, for every field type

`FieldInfo.nullable` answers *"was this field declared `T?` rather than `T`?"* — the fact
a generated `CREATE TABLE` needs for `NOT NULL`. It was right for the scalar types and a
constant `true` for the other four: a struct, an enum, a vector, and a keyed collection.
So a generic serialiser or ORM emitted every one of those columns as nullable, dropping a
`NOT NULL` the declaration had asked for.

The declaration really does decide — a `Thing` field cannot hold null and a `Thing?` one
can — so this was a fact being lost, not two things that are really one. It now follows
the declaration for every field type, whether the type is declared above the struct or
below it.

One field type is deliberately exempt: `reference<T>`, which is a pointer. A pointer field
can be cleared with `null` and starts as `null` when you leave it out, so it reports
nullable whether or not you write the `?` — the `?` is not what decides it there.

One side effect worth knowing: the redundant-null-check warning can now see these fields
too, so comparing a non-nullable struct, enum, vector or collection field against `null`
is reported the way the scalar ones already were. The comparison was always pointless;
nothing said so before.

### A lazy store that is not a store now says so

`store_bind_lazy` reported every failure to *reach* a source and none to *read* it. A
missing file, an HTTP 404 and a refused connection each raised a fault with a reason. An
empty file, a truncated download, a directory, or a URL answering `200` with an HTML
error page raised nothing at all — `store_lazy_faults` stayed `0`, `store_lazy_error`
stayed empty, `store_verify` said true, and every key came back `null`. That is exactly
what a valid store with no such key looks like, so a program could not tell "the dataset
is empty" from "the dataset never loaded".

The URL case is the one that bites: a stale CDN path, a bucket serving its own 404
document, or a half-finished upload binds successfully and answers `null` forever, with
the health channel reporting fine.

A store image begins with a four-byte marker, and binding now checks it. Any source that
is reachable but is not an image raises a fault with a reason, the same way a missing
file always did.


One thing changed for a *damaged* image: a file whose first four bytes are gone is now
refused rather than read. It used to answer correctly if the pages your keys needed
happened to be intact, which was luck rather than a promise. A merely truncated image is
unaffected — those pages are still read.

### A `match` at the end of a function no longer frees its locals too early

A `match` written as the **last statement** of a function, with a `return` in **any** arm,
released that function's locals *before* the arms ran:

```loft
fn use_it() {
    x = Box { id: 1, items: [7] };
    match 0 {
        0 => { println("items[0] = {x.items[0]}") },   // read a released value
        _ => { return }
    }
}
```

What you saw depended on what the local held. A number came back as `null(oob)` with
`--native` and correctly under the interpreter — the same program, two answers, no
diagnostic. A text came back **empty** on both. A value with a drop was released once
before the arm and again at the `return`, which is a use-after-free: the interpreter ran
the drop twice, and `--native` stopped with an out-of-bounds index.

`break` and `continue` in the same position were always fine, and so was a single
statement after the `match` — which was the workaround. All of it now behaves the same:
one release, after the arm that runs.

### A multi-line block with a value in it is now indented like one without

A backtick block loses the indentation you wrote it at, so the text can sit where it
belongs in your source without that showing up in the output. Unless it contained a
`{…}` — then it was not dedented at all, and kept its trailing blank line too. One value
anywhere in the block, before or after the affected lines, was enough:

```loft
page = `
    <h1>hello {name}</h1>
    <p>bye</p>
    `;
```

That printed four spaces in front of every line and a line of spaces at the end. The
same block without `{name}` printed flush. So the feature served the block with nothing
in it and quietly stopped serving the template, which is what it is for.

The base indentation is now taken from the block's **first content line** instead of
from the closing backtick, which is what lets a block with values in it be measured at
all. In the ordinary layout — content indented to one level, closing backtick a level
out — that is the same number as before and nothing moves. Two things do move, both
only in blocks laid out unusually:

- a block whose closing backtick sits at a *different* column than its own lines now
  follows the lines. This kept four spaces in front of `select 1` and is now flush:

  ```loft
  sql = `
          select 1
      `;
  ```

- a line indented *less* than the base comes out flush rather than keeping its own
  indentation, so it can no longer end up further right than lines that were indented
  past it.

A blank line does not set the base (a template may open with one), and a tab-indented
block is still left alone — a tab is not a space, so there is nothing to count.

### A stray `{` in a message now tells you what to write

The two ways of getting a literal brace wrong got very different answers. A lone `}`
named itself, gave the fix, and stopped there. A lone `{` said `Formatter error` and
then five more errors, the last of which blamed the closing brace of the function:

```loft
println("a lone open { here");
```

`{` opens a value slot, and a slot has to close on the line it opened. When nothing on
that line can close it, that is now a single error pointing at the `{`, naming the cure
(`{{`), and the rest of the file parses as usual.

### A parallel loop with an empty body works with `--native`

```loft
for a in rows par(b = work(a), 4) { }
```

Running the workers purely for their effect and ignoring what they return compiled fine
in the interpreter and failed `--native` with a raw compiler error out of `rustc`. Giving
the body something to do was the workaround.

It compiles now. Two quieter problems came out with it, both in the interpreter and both
invisible for the same reason — a loop that discards every result has nothing to compare:
the row reached the worker in the wrong shape, so a worker taking an `integer` read
nonsense, and a worker returning a `text` could crash the process outright. Every return
type now behaves the same as it does in a loop whose body uses the result.

### A parallel worker declared below its loop no longer mistypes the result

A `par` worker returning a `float` typed the result variable as `integer` when the
worker was declared *after* the loop that used it — so a running total refused to add:

```loft
fn main() { t = 0.0; for a in 1..=4 par(b = half(a), 4) { t += b; } }
fn half(v: integer) -> float { v * 0.5 }
```

*"Variable 't' cannot change type from float to integer"*, on a line where nothing is an
integer. Moving the worker above the loop fixed it, and so did writing `t = t + b`
instead of `t += b`, neither of which is a hint about the cause. Declaration order no
longer decides the type.

### A package keeps its own files, whoever else is in the build

If two libraries both had a file called `skin.loft`, only one of them got to be `skin`.
The other one's file never loaded, so the functions in it were simply missing — reported
against a line inside a library whose author had never seen the problem, in a program they
did not write. Which one lost depended on the order the *consumer* happened to list them
in, and swapping those two lines moved the breakage to the other library.

A library now always gets its own files. `use skin;` inside a package means *this
package's* `skin.loft`, so two libraries can both have one and both work, in any order.
Naming a dependency still wins over a file of the same name, so nothing can shadow what it
depends on.

One thing to know: if both files happen to export the *same* name and you call it without
saying which you mean, you now get an error naming both, instead of one of them being
picked for you. Give one an alias (`use skin as s;`) or import the name you want directly.

### Reading a field the value's variant doesn't have now answers null instead of nonsense

Enum variants carry named fields you read directly:

```loft
enum Node { Named { label: text, n: integer }, Anon { k: integer } }
```

Reading `a.n` when `a` happens to be an `Anon` answered a number — `Anon`'s `k`, handed
back as if it were `Named`'s `n` — and writing `a.label` stored the text in the `Anon`
value, which went on calling itself an `Anon` ever after. Nothing said a word.

Reading fields straight off an enum is still the way loft works, and a field that *every*
variant declares is unaffected. What is new is that reading one only *some* of them have
now **checks which variant you actually have**: the read answers `null` — the same answer
you get from a missing key or an index past the end — and the write is ignored instead of
landing in the wrong field. The value's own fields are left alone, and it goes on being
the variant it was.

You are still told about it, with the variants that have the field and the `match` or `is`
form that binds it for the one you hold, because a write that quietly does nothing is
rarely what anyone meant. Set `LOFT_NO_VARIANT_FIELD=1` to turn the message off; the check
itself stays either way.

One case is left unchecked, and it says so: when the thing you read the field *from* is
itself a call (`shape_of(x).radius`), checking the tag would mean calling it twice. Bind it
to a local first.

Related, and fixed with it: adding to a list through something that isn't there —
`s.v += [1]` where `s` is null — stopped the program with an internal message instead of
doing nothing. Setting a plain field that way was already ignored; adding to a list now is
too.

### Writing `Thing { }` works wherever the struct is declared

An empty struct literal — the way you ask for a value with every field at its default —
was a parse error if the struct happened to be declared *below* the function using it:

```loft
fn main() { a = Cfg { }; }
struct Cfg { port: integer }        // declared after — used to break the line above
```

Naming a single field (`Cfg { port: 0 }`) worked, and moving the struct above worked, so
the error pointed at a line that had nothing wrong with it. Both spellings now work in
either order.

### Tests now see the same warnings a program does

A handful of checks — the lost-write warning, the double-move warning, and the
`#superseded` signpost check — only ran when you ran your code as a *program*. Under
`loft test` they were silent, which is the wrong way round: a library is checked by its
tests, so exactly the code most in need of them was the code that never got them. A
library could publish a signpost pointing at a function that does not exist, or a helper
whose writes all land in a copy, with a completely green suite.

They now run on both paths, once per file, and `--deny-warnings` fails on them as you
would expect.

### Dividing a float by zero gives one answer everywhere

`1.0 / 0.0` answered two different things depending on where the result went. Written
straight into a message it was `inf`; assigned to a variable, or handed back from a
function, it was `null` — the same expression, both backends, nothing said a word. So a
check that looked unnecessary on one line was load-bearing on the next.

There is one answer now, and it is the one the rest of the language already used: `NaN`
is the float null, so `0.0 / 0.0` is null, and `1.0 / 0.0` is `inf` — a real value, which
is why `?? 0.0` does not replace it. That was already true of float *overflow*
(`1.0e308 * 10.0` has always been `inf` wherever it went), so division now agrees with its
own sibling. Dividing by zero without a guard still prints its warning and still keeps
going.

If you were relying on `?? 0.0` to catch a divide by zero, note that it only catches
`0.0 / 0.0`. Test the divisor (`if d != 0.0`) when any numerator is possible.

### A value that doesn't fit a limited field no longer becomes a different one

For a field declared with a range —

```loft
struct Pixel { r: integer limit(0, 255) }
```

— writing something outside it did one of three things, and never said so: `256` came back
as `0`, `260` left the old value untouched, and on a two-byte field `70000` came back as
`4464`. A number you never wrote, sitting in a field that looks fine.

Now anything outside the declared range stores the field's **default** — the lowest value
in that range, or `null` if it is nullable. Nothing is wrapped, aliased, or quietly
dropped, and you get a warning saying so. Ranges that don't start at zero work the same:
`limit(300, 400)` and `limit(-200, 0)` still pack into a single byte, still hold every
value they declare, and now refuse the ones they don't — `500` into a `limit(300, 400)`
gives you `300`, not `500`.

The same is true of a **variable**, which used to ignore its range entirely:

```loft
count: integer limit(10, 20) = 12;
count = 99;   // 10 — the bottom of its range, and a warning
```

In-range code is untouched and costs nothing: the check is only emitted where the value
might actually fall outside.

### A function that sometimes hands back what you gave it, and sometimes something new

A helper with two ways out — one returning what it was passed, the other building a fresh
value —

```loft
fn rotate(st: Stencil, seen: boolean) -> Stencil {
  if !seen { return st; }      // hand back what came in
  Stencil { cells: turned }    // or build a new one
}
```

— never released the value it built. Every call left one behind, so a loop that rotated a
stencil a few thousand times quietly grew by a few thousand stencils. Nothing was wrong
with the answers, nothing was printed, and the program exited 0; it just used more and
more memory the longer it ran. The same happened to the far more common lookup-with-a-
fallback shape:

```loft
fn get(b: Bag, k: text) -> Item { b.items[k] ?? Item { name: "missing", limbs: [] } }
```

where every miss left its fallback record behind.

The trouble was that a function got to give one answer, once, to "who cleans this up?" —
and these functions need two, because which one is right depends on which way the call
actually went. That is now decided per call instead: a value the function built is cleaned
up by the caller, and one that was already yours is left alone. Writing the same thing in
the caller was the workaround, and it is no longer needed.

One shape is still left out and still leaks: a helper whose *collection parameter itself*
is the thing looked in (`fn get(items: hash<Item[name]>, k: text)` rather than a struct
holding it). Passing the struct, or binding the lookup in the caller, avoids it.

### An `if` that answers a new value on one side and an existing one on the other

Choosing between building something and pointing at something already stored —

```loft
it = if fresh { Item { name: "fresh", limbs: [] } } else { b.items["one"]? };
```

— quietly threw away the record `it` was pointing at, once the function returned.  The
first read was correct; the next allocation anywhere in the program took over the freed
space, and every read after that answered out of whatever landed there.  The entry could
even stop being found in the collection it was still filed under.

The giveaway was that swapping the two sides around fixed it, which is nobody's idea of a
difference that should matter.  It doesn't now.  A value that could have come from either
side of an `if`, an `else if`, or a `match` is treated as possibly pointing at whatever
either side points at — so nothing it might still be using is released.  Building on both
sides is unaffected: those really are new values, and they are still cleaned up.

### Filling in a list inside an enum variant works

A struct-enum whose variant carries a collection —

```loft
enum Shape { Circle { limbs: vector<float> }, Square { s: float } }
```

— could be built with its contents (`Circle { limbs: [1.0, 2.0] }`), but adding to that
list afterwards, or replacing it, stopped the program with an internal message and no
line number. It made no difference how the list was reached: a local, an element of a
vector of shapes, a function parameter, a field of some other struct. Reading was fine,
which is what made building the value up in steps look like a mistake in your own code.

Both work now, for every kind of collection a variant can hold.

### A helper that looks something up no longer breaks what it looked in

Reading a record out of a collection through your own small helper —

```loft
fn part(ps: PartSet, name: text) -> Part? { ps.parts[name] }
```

— handed back the right record and then, on the way out of the caller, released the
storage it had borrowed from `ps`. Nothing said so: the first read was correct, and the
next allocation anywhere in the program took over the freed space, so later reads
answered out of whatever landed there. In dryopea a tower drew 96 triangles the first
time and nothing the second. Writing the same lookup inline at the use site was correct,
which is what made it look like a data problem rather than a language one.

The signature now says what the helper hands back: a view into its argument, which the
caller borrows and does not free. Both backends, and the same for a helper that binds a
local first, reaches through two fields, takes the collection itself as the parameter, or
is discharged with `?` or `??` at the call site.

### A one-file script takes the newest release, and stops writing files at you

`use arguments;` in a script with no `loft.toml` already resolved the newest release —
and then left a `loft.lock` in whatever directory you happened to be standing in. That
file pinned the script forever, from then on: the "latest" it took once became the
version it took always, decided by something the run itself produced. Run the same
script from a second directory and it re-resolved and dropped a second lock there.

Now nothing is written by running. A script with no declaration means *the newest
release, re-decided every run*; where you stand is not part of the answer; and if the
registry cannot be reached, the newest copy already in your cache answers instead of a
"library not found" in a directory holding five copies of it.

That covers a project too, which is the shape that hurt most: a project with a
`loft.toml` but no `loft.lock` yet used to resolve through whatever `loft.lock` happened
to sit in the directory you ran from. Same project, same manifest, three directories,
three library versions — and the error you got when the wrong one loaded said the
function was missing, never that a directory had chosen the version.

Where you DO have a declaration — a `loft.toml` in the project, or a
`<script>.loft.lock` from `loft pin` — nothing changes: it governs, exactly as before.
`loft install cbor@0.1.2` in a directory that is not a package now writes the small
`loft.toml` that makes that pin stick, and says so.

### A pinned version is installed, not just loaded

`loft pin my_script.loft` writes the versions that script runs against, and that held as
long as those versions were already in your package cache. On a machine where they were
not — a colleague's laptop, a fresh CI runner — the run installed the *newest* release
instead and said nothing, so the same pinned script ran different code on two boxes. It
now installs exactly what the pin names. The same applies to a project whose
`loft.lock` names a version the box has not downloaded yet.

If you have edited `loft.toml` since (say `^0.1` to `^0.2`) and not re-installed, the
manifest still wins — a lockfile is the resolved form of what the manifest asks for, so
it cannot outrank it.

### Your build says once when a pin has fallen behind

A lockfile has no expiry, so a pinned version holds forever — including through a
release that fixes something you would want. `cbor` 0.1.3 turned one encoder from
O(n³) to O(n²) (a few hundred entries went from "effectively hung" to milliseconds)
with no API change at all: a project pinned at 0.1.2 keeps hanging, and nothing it can
read explains why.

When a pin governs and the registry index you already have says there is a newer
release, the build mentions it once:

```
[registry] cbor 0.1.2 is pinned; 0.1.3 is the newest release — run: loft install cbor
```

It never fetches anything to say this, never speaks for a library's own dependencies,
stays quiet when you are current or offline, and can be turned off with
`LOFT_NO_UPGRADE_NOTICE=1`.
### A `par` loop no longer depends on where its worker is declared

`for a in v par(b = work(a), 4) { … }` compiled with `work` declared above it and
failed with `Expect token ;` on the next line with `work` declared below it. The loop
was being read as a value rather than a statement: when the worker cannot be resolved
yet — the ordinary state of a forward reference on the first parse pass — the recovery
path left no statement behind, so the parser demanded a semicolon after the closing
brace. Four such exits now all leave one, on both backends.

The same change stops a malformed `par` clause from swallowing its own error. `par(b =
x, 4)`, where a worker call belongs, used to report `Expect token ;` pointing at the
following line; it now says what is wrong, where it is wrong:

```
error: Expect '.' after 'x' in parallel clause (use a.method() or func(a))
  --> p.loft:4:24
4 |   for x in v par(b = x, 4) {
  |                        ^
```

### `x.method(name: value)` — named arguments reach the method spelling

loft has had named arguments for a while, and they only worked on the free spelling of a
call. `render(cfg, dry: true)` compiled; `cfg.render(dry: true)` was a parse error — the
same function, the same argument, the same default. Both work now, in any order, mixed
with positional arguments, on both backends.

It mattered most where loft sends you to use it. When a function ends in two boolean
parameters, loft advises *"give them defaults so callers pass only what they change"* —
but on a method, only a named argument can change a flag that is not the first one. Take
the advice and the only spelling left was `f(false, true)`: the very shape the advice was
complaining about, now with a default in front of it.

Two smaller things fell out of the same seam, both of which made a legal program depend on
where its types were DECLARED rather than on what it said:

* `grid.cells[1, 2]` — a lookup in a collection with a compound key — parsed only when the
  types were declared above the caller, and reported `Expect token ]` below it.
* An unresolved method called with a named argument cascaded into five errors that never
  mentioned the real one. `s.nosuch(width: 3)` now reports `Unknown field S.nosuch`, once.

### A default that reads an earlier parameter is a read

`fn window(rows: integer, height: integer = rows * 10)` was reported as *"Parameter rows is
never read"*, offering to drop `rows` and its callers' argument — which deletes what the
default reads. A default is an expression evaluated at the call, and reading a parameter
there counts. A parameter genuinely nobody reads still warns.

### A closure capture is no longer reported as a dead assignment

`s = 10; show = fn() -> integer { s }; s = 20;` hands `show` the 10 — and loft used to
call that first line a dead assignment and offer to delete it. The offer was wrong: the
capture reads the value where it is written, which is what makes a snapshot a snapshot,
and taking the advice changes what the program answers. The warning now stays silent on
any variable a closure captures. A variable no closure touches is unaffected.

### Adding a file no longer changes a library's answer in silence

Two `.loft` files in different packages can share a basename, but only one of them can be
the module of that name — so if your project adds `src/catalogue.loft` and a library you
depend on already had one, the library's own file never loads and *its* `use catalogue;`
picks up yours. The library then computes with your data: in the reported case it answered
100 where, on its own, it answers 42. Nothing in the library changed, and its own tests
still pass.

loft has named that collision for a while, but as `advice` — and advice is routinely
filtered out of build logs, which is exactly what happened. When the file that wins is
*your project's* and the one that loses belongs to a *dependency*, it is now a **warning**:

```
warning[module-name-shadowed]: this project's '…/src/catalogue.loft' captured the
  module name 'catalogue', which dependency module '…/dep/src/catalogue.loft' was
  already using … The dependency now answers differently than it does on its own.
  Rename this project's 'catalogue.loft'; the dependency's author can end it for
  every consumer by writing `use self::catalogue`
```

The other direction — your own module losing its name to a file elsewhere in the graph —
stays advice, because published libraries legitimately overlap that way today and the cure
there (`use self::<module>`) is yours to apply.

### loft tells you when a library is not in your `loft.toml`

`use hex_grid;` works even when your `loft.toml` never mentions `hex_grid`, as long as the
package is installed on the machine. That is deliberate — it is what makes a one-file
script Just Work — but in a project it meant nothing recorded whether you *depend* on a
library or merely *have* it. You could delete a line from `[dependencies]` and every test
still passed, so "is this dependency still load-bearing?" was a question no check could
ask.

Now it says so, once, and keeps running:

```
advice[undeclared-dependency]: `hex_grid` resolved from the registry, but
  `…/loft.toml` does not declare it — so nothing here says whether the project
  depends on `hex_grid` or merely runs on a box that has it installed
  fix  run `loft install hex_grid` to record it under `[dependencies]`
```

Worth doing: an undeclared library is not pinned either. It resolves to the newest version
present, so two machines can quietly build against two different versions of it.

Single-file scripts hear nothing — there is no manifest to declare into — and neither do
you about a library's own dependencies, which are its author's to record. Silence it with
`LOFT_NO_UNDECLARED_DEP=1`.

### `loft install` installs what your project depends on

Typed on its own in a project directory, `loft install` now reads your `loft.toml` and
resolves every dependency it declares — the same thing `npm install` and `cargo fetch`
do, and the thing `loft api` has always told you to run when it reports a dependency as
missing.

It used to install **your own project** into `~/.loft/lib/`, and do nothing about the
dependencies. So the one hint the tool gave was for the one case the command did not
handle — and the copy it left behind could quietly take priority over the published
version of a package with the same name.

If you wanted the old behaviour, it has always had its own spelling:

```bash
loft install          # resolve what loft.toml declares
loft install cbor     # install one package from the registry
loft install .        # install THIS package into ~/.loft/lib for global use
```

A dependency declared as `{ path = "../somewhere" }` needs no install at all — it is
read from the path it names — so `loft install` mentions one only when that path leads
nowhere. And an install is now filed under the name in your `[package] name`, not under
whatever your checkout directory happens to be called.

### A constant list with a negative number in it works

```loft
const OFFSETS: vector<integer> = [10, -5, 9];
```

That read back **empty** — `len()` gave 0, every index gave null, and a `for` over it ran
zero times. No error, no warning, on both backends. One negative number anywhere in the
list was enough; all-positive lists of any length were fine, and the same list written
inside a function was fine.

Worth knowing even if you never hit it: a loop over an empty list runs its body zero
times, so *every check inside it passes*. A test built on such a constant is green
because it is measuring nothing.

### `map` can change the element type

`map` is documented as `fn(T) -> U` answering a `vector<U>`, and now it is:

```loft
xs = [1, 2];
v = map(xs, |x| { "n{x}" });     // vector<text> — used to be refused
```

Before, every `U` other than `T` was rejected, and the error pointed *inside your own lambda*
("expected integer, got text") or at the assignment ("cannot change type from vector<integer>
to vector<text>"). Naming the destination did not help. The only shape that worked was a
transform back to the same type.

The same fix removes two crashes that had nothing to do with changing the type:
`xs.map(|s| { "{s}!" })` on a list of text, and `map(xs, some_text_function)`, both of which
used to report an internal compiler error.

### `reduce` can build text

```loft
joined = words.reduce("", |a, w| { "{a}{w}" });   // one string from a list of them
```

This used to crash the compiler. Once it compiled, it was worse: the fold reused a single
buffer across the turns, so it kept only the *last* step and quietly answered `"c"` for
`"abc"`.

Folding into a **list** is still refused — with a message pointing at the loop to write
instead, rather than the internal compiler error it used to be.

### A `?` on a list or map field now works

You could always write it, and it never did anything:

```loft
struct Config { tags: vector<text>?, }

c = Config { tags: null };
c.tags == null            // said false
```

A collection field stores a record number, and zero already meant "empty" — so `null` and `[]`
were written identically and the check could never come out true. loft warned you about this
at the declaration and told you to drop the `?`.

Now the two are different things, and the warning is gone:

```loft
Config { tags: null }.tags == null      // true  — absent
Config { tags: [] }.tags   == null      // false — present, and empty
```

`len()` still answers 0 for both, which is usually what you want; `== null` is there for when
the difference matters. Works for every collection kind — `vector`, `hash`, `sorted`, `index`,
`spatial`, `trie` — on both backends, and nothing about how your data is stored changed.

### Checking a collection field against `null` no longer damages the record

Comparing a `vector` field with `null` freed the storage of the struct it was read out of. The
next read of that field then returned whatever had since been put in its place:

```loft
h = Holder { vec: [71, 82, 93] };
x = h.vec == null;
filler: vector<integer> = [11, 22, 33, 44, 55, 66, 77, 88];
len(h.vec ?? [])          // said 8 — the filler's length — instead of 3
```

No warning, no crash, on both backends: one variable quietly reading another's data. Fixed.

### Using a type inside a tuple before you declare it

loft lets you use a struct before the line that declares it — except inside a tuple, where
it did not work at all:

```loft
fn main() { t: (integer, Player) = (1, Player { id: 7 }); }
struct Player { id: integer }
```

That reported `cannot change type from (integer, unknown) to (integer, unknown)` — the same
type printed twice — and some spellings reported an internal compiler error instead. It now
works, in every place a tuple can appear: a local, a vector element (with or without writing
the type), a struct field, a nested tuple, a function parameter.

One case is still not supported, and now says so plainly instead of crashing: a function that
*returns* a tuple containing a type declared further down. Move that declaration above the
function and it works — the error message tells you exactly that.

### Lists of tuples you did not have to write the type of

Writing the type out worked; letting loft work it out did not.

```loft
v = [(7, 8), (9, 10)];      // "cannot build this record — its type never resolved"
```

Every tuple shape was refused this way, including one with no structs in it at all, and so was
the version that puts the tuple in a variable first (`t = (7, 8); v = [t]`). The declared
spelling — `v: vector<(integer, integer)> = [(7, 8)]` — was fine, so the fix is not a new
feature: it is the same list, written the shorter way.

A tuple with a struct in it had a second problem, and this one did not need a list at all:

```loft
t = (Player { id: 1 }, 50);   // "internal compiler error"
```

Both are fixed. Tuples of any shape now work whether you write the type or not, nested tuples
included, and a struct can sit in any position.

One related case is still open ([#944](https://github.com/loft-lang/loft/issues/944)): a
struct used in a tuple *above* the line that declares it. Declare it first, and everything
here works.

### A vector of tuples that start with a struct

Pairing a record with a number and keeping a list of them is an ordinary thing to want:

```loft
scores: vector<(Player, integer)> = [(Player { id: 1, name: "ana" }, 50)];
```

Written that way round it did not work, and it went wrong differently depending on what you
did with it — reading one back crashed, two or more in the same literal refused to compile,
and building the list with `+=` gave you back a player whose fields were all zero, with no
message at all. Writing the number first (`vector<(integer, Player)>`) worked fine, which made
the whole thing look like several unrelated bugs.

It was one: the first thing inside the brackets was being built directly into the list's own
slot, which is right for `[Player { … }]` but not for a tuple, where each part has its own
place in the element. Both orders now work, and so do three-part tuples and tuples of two
structs.

### Taking a collection out of a tuple return, in a loop

A function can hand back several things at once, and the natural way to use one is to
destructure it straight back into the variable you passed in:

```loft
fn find_or_add(keys: vector<integer>, k: integer) -> (vector<integer>, integer) { … }

for item in items {
  (keys, at) = find_or_add(keys, item.id);
}
```

From the second turn of that loop, `keys` was empty. Not an error — just empty, and
`--native` gave the right answer while the interpreter gave the wrong one. If the function
appended to the vector instead of only reading it, the run crashed rather than lying.

The value a tuple return is built in belongs to the call, and the next turn of the loop
reclaimed it while the variable was still pointing at it. Destructuring now takes a copy the
variable owns, so it keeps its value for as long as the variable does. The same fix covers a
struct taken out of a tuple, which was quietly losing its fields the same way.

### Two `for` loops in one function can share a name

`for i in names { … }` followed by `for i in 0..3 { … }` used to be a compile error —
the second loop was handed the first loop's variable, so the name could only ever hold
one type per function. Consumers worked around it by prefixing every loop variable with
something per-function (`wt_i`, `tslr_w`), which carries no meaning and is what a reader
meets first in every loop.

Each `for` now binds its own variable, so two loops can spell the name the same way at any
element types. Reading the variable after the loop still works and still gives the last
loop's value, so nothing that relied on that changes.

Nested loops are the exception — `for i { for i { } }` is still rejected, because the inner
binding would take over `i` for the rest of the outer body — and so is a loop variable that
lands on a plain local you already have, which the compiler names and tells you how to fix.

### A struct field you declared with `?` can now actually be empty

Writing `?` on a struct-typed field is how you say "this may be absent". Until now it did
not do that. A field declared `maybe: Inner?` was stored exactly like a field without the
`?`, so there was nowhere to record that nothing was there:

```loft
struct Inner { z: integer }
struct H { maybe: Inner?, tag: integer }

h = H { maybe: null, tag: 0 };
println("{(h.maybe ?? Inner{z:-1}).z}");   // was 0  — now -1
println("{h.maybe == null}");              // was false — now true

h.maybe = Inner { z: 9 };
h.maybe = null;                            // was ignored — now clears
println("{h.maybe == null}");              // was false — now true
```

All three readings of that declaration disagreed with it: `??` never reached its default,
`== null` was always false, and assigning `null` kept the value that was already there.
A program that stored `null` to let go of something optional quietly held on to it, and a
program that checked for `null` before using a field took the "it's there" branch every
time. Both on both backends, and neither said anything.

The field now carries a small marker saying whether it holds a value, which is the same
representation `vector<Inner?>` elements have used for a while — so `??`, `== null`, plain
reads and assignment all agree with each other and with what you wrote. A field with **no**
`?` is unchanged: it cannot be absent, so it stores exactly what it did before and pays
nothing for the marker.

Two smaller things came with it. A struct literal that simply left such a field out did not
compile at all under `--native`; it does now. And a `?` field costs 8 bytes more than it
did — `sizeof` on a struct containing one has grown, which matters only if you were
depending on the exact number.

### A struct literal that leaves a field out will mention it

Leaving a field out of a struct literal gives it that type's zero. That is documented and
unchanged — but nothing distinguished it from someone writing the zero on purpose, and it
goes wrong exactly where zero is a real value:

```
advice[omitted-field-zero]: `EditorInput` literal omits the field `palette_index`, which
takes the type's zero — nothing in the declaration chose that value
```

The cure already existed and was simply hard to find: give the field a default where you
declare it (`palette_index: integer = -1`). Adding one is additive, so callers that already
pass the field keep working.

This is advice, never an error, and it stays quiet where the code already says what it
means: a field **with** a declared default, a nullable field, a collection or text field
(whose zero is "empty", which is the only default you could declare anyway), and a bare
`Thing {}` — that asks for the whole default record, and reads that way. It only speaks for
the partial literal, where some fields were singled out and a reader cannot tell whether the
rest were considered. `LOFT_NO_OMITTED_FIELD=1` turns it off.

### A mistyped key field says so, instead of crashing the compiler

Naming a key a collection's element type does not have — a typo, or the `hash<key, Value>`
spelling most other languages use — used to crash the compiler with an internal error and
point at a line that was fine. It now says what is wrong, where you wrote it:

```
error: Field `idx`: `ca_kye` is not a field of `At`, so it cannot be a key — did you mean
`ca_key`? A keyed collection names its keys as FIELDS OF ITS ELEMENT — write
`hash<Element[key_field]>`, not `hash<key, Element>`
```

All five keyed kinds are covered: `hash`, `index`, `sorted`, `spatial` and `trie`.

### `loft test` stops recompiling your library once per test file

A suite of test files that all `use` the same library used to compile that library
again for every one of them — twice for each, in fact, since the parser reads a file
twice. Nothing was shared between the files, so the cost was the *product* of how many
test files you have and how big your library is: a new module slowed down every test
file, and a new test file re-paid for every module.

```
20 test files over a 25-module library     1.32 s  ->  0.43 s
20 test files over a 50-module library     2.44 s  ->  0.81 s
dryopea's real suite (81 files, 1161 tests)  238 s  ->   209 s
```

Test files that open with the same `use` lines now share one parse of those libraries.
Nothing to change in your code, and nothing about what a run reports changes — each
file is still compiled on its own, still sees only the libraries it named, and still
raises exactly the diagnostics it did before. Running a *single* file is untouched:
sharing needs a second file to share with, so `loft test one_file.loft` costs what it
always did.

### Reading a vector in a loop is about twice as fast

`v[i]` has to work out three things before it can read anything: which store the vector
lives in, which record holds its elements, and how long it is. In a loop that only *reads*,
none of those can change — but every one of them was worked out again for every single
element, and the Rust compiler could not lift them out for us.

Now the compiled backend works them out once, before the loop:

```loft
for j in 0..n {
  ax = qx - (sx[j] ?? 0.0f);      // ~19 ns per iteration before
  ay = qy - (sy[j] ?? 0.0f);      // ~8.5 ns after
  az = qz - (sz[j] ?? 0.0f);
}
```

Nothing to change in your code, and nothing to be careful about: the moment anything in the
loop can write to a collection — appending, removing, clearing, assigning, or calling
something that does — the loop goes back to working it out per element, because then it
genuinely can change. The interpreter is unaffected.

### Filling a vector is as fast written the obvious way

`[for _ in 0..n { -1 }]` — a vector of `n` copies of one value — ran the full
element-by-element build protocol, once per element, for a value that never changed.
The other spelling, `[-1; n]`, has always claimed one element and copied it, and was
about five times faster.

Now the obvious spelling compiles to the fast one. Building a million-element vector
went from 74.6 ms to 14.4 ms on this machine; `[-1; n]` takes 14.7 ms, so the two are
the same code. Nothing to learn and nothing to rewrite — if you already use `[x; n]`,
it is unchanged.

This only applies when the body really is the same value every time. A body that reads
the loop variable, or calls anything at all, still runs once per element:

```loft
[for _ in 0..n { -1 }]        // one value, copied n times
[for i in 0..n { i * 2 }]     // n different values — unchanged
[for _ in 0..n { next() }]    // n calls — unchanged
```

### A program can ask its environment a question it might not answer

`host_input()` reads until whoever is writing hangs up. That is right for a compute
program reading a file on its input, but it makes one question unaskable: *is anyone
out there?* A program that does

```loft
host_output("MODE?");
mode = host_input();
```

got an answer in a web page whose JavaScript replies — and waited forever anywhere
else, because nothing that is absent ever hangs up.

`host_input` now takes an optional wait:

```loft
mode = host_input(200);        // "" => nobody is listening, carry on locally
```

`host_input(0)` takes whatever has already arrived and returns straight away, a
positive number waits that many milliseconds for the first byte, and plain
`host_input()` still reads the whole stream exactly as before. So a request and its
reply are now a conversation you can have outside the browser too — ask, wait a
moment, and treat silence as an answer instead of a hang.

Characters are never torn in half by the wait: a read that arrives mid-character
hands over the part that is whole and keeps the rest for the next read.

### A library can say "my own module", so a consumer cannot change its answer

If your library has `src/catalogue.loft` and says `use catalogue;`, it did not
necessarily get *your* file. Module names are shared across the whole dependency graph,
and building a package reads every file under `src/` — so a program that uses your
library and happens to add its own `src/catalogue.loft` takes the name, and **your**
code starts calling **their** function:

```loft
// your library                        // their program
pub fn part_list() -> integer { 41 }   pub fn part_list() -> integer { 99 }

dep_answer()   // 42 on its own … and 100 in their tree
```

Nothing in your library changed. Nothing in their program imported `catalogue`. Write
`use self::catalogue;` and that cannot happen — it always means your file:

```loft
use self::catalogue;             // this package's own src/catalogue.loft
use self::catalogue as cat;      // …with a qualifier: cat::part_list()
```

Two packages can now both have a `catalogue` module and both work, which is the part
that could not be fixed just by preferring the nearer file.

Bare `use catalogue;` still behaves exactly as before, so nothing you have written
changes — add `self::` where you want the guarantee. The advice that already warns you
about a shared module name is the signpost for where.

### A browser page's crash tells you which of your functions crashed

When an `--html` page traps, the browser hands over a full backtrace, and until now
none of it could be read:

```
[exception] RuntimeError: unreachable
    at wasm://wasm/0168beca:wasm-function[1073]:0x56a035
    at wasm://wasm/0168beca:wasm-function[1054]:0x567983
```

Those numbers name the failing function and everything that called it, but a page
carried nothing to turn a number into a name — so the only way forward was moving a
`println` through your source, rebuilding, and reloading the browser, over and over.

Build with `--names` and they resolve:

```
loft --html --names game.loft
```

The page grows by roughly 10–15 %, which is why you ask for it rather than always
getting it. Reach for it the moment a page traps, and drop it when you ship.

One thing to know: `--names` also keeps your functions from being folded into their
callers, which is what leaves a frame to put a name on. That makes it a slightly
different build — so if a trap happens without `--names` and stops happening with it,
that is worth knowing rather than worth ignoring.

### A test that names a helper the way its library does

A package whose test file defined `fn defaulted(…)` while its library had a private
helper of the same name ran fine on the interpreter and would not compile with
`--native`: `cannot find function n_defaulted in this scope`, on eight tests at once.

Two functions can share a name when they come from different files, and the generated
Rust renames one of them to keep them apart — but one place that writes a CALL spelled
the original name. Both spellings work now, and neither needs renaming to avoid the
other.

### `[x; n]` gives you n elements — of whatever `x` is

Writing `[7; 3]` built **four** elements, and the fourth held whatever was in memory — a
wrong length and unpredictable contents, on both backends, with nothing said about it.

`[7; 0]` was worse: it crashed the process inside the system allocator, because the copy
count wrapped around and wrote past the end of the vector. A count that came out
**negative** — `[7; a - b]` where `b` is the larger — did the same thing.

And `["abc"; 4]` gave you `"abc"` once, then junk: only the first element was really the
text you asked for. The same went for a struct or a nested vector, and for a text field
inside a repeated struct. The length was right and the first element was right, so it read
as correct until something looked past it.

All fixed: `[x; n]` now holds exactly `n` copies of `x` — for any `n`, including zero and
negative, whether the count is written in the source or worked out while the program runs,
and whatever kind of value `x` is.

Worth knowing if you build big arrays: `[x; n]` is currently about **five times faster**
than the equivalent `[for _ in 0..n { x }]`, so it is the spelling to reach for when you
are filling a vector with one repeated value.

### Storing the wrong type into a struct field now says so

Assigning a value of the wrong type to a **field** was accepted and then did nothing. The
old value stayed, no diagnostic appeared, and the program carried on:

```loft
st.view = graphics::mat4_look_at(eye, target, up);   // `view` is a vector<float>,
                                                     // mat4_look_at returns a Mat4
```

That compiled clean and stored nothing, so the page drew every frame through whatever the
field held before — a picture that looks like a camera bug. The identical assignment to a
**local** was refused, which is what made it look like a hole rather than a decision.

It was also not only a lost write. A `text` field given an integer carried that number
into the text machinery as if it were a handle and crashed the process; a `+=` in the same
shape wrote into read-only memory and panicked.

All of it is now one error, naming the field's type and the cast that would make it
deliberate:

```
error: Cannot assign Wrap to a field of type vector<float> — use 'as vector<float>' to
cast explicitly
```

Correct stores are untouched, including the ones where the two types genuinely differ:
building a `hash` or `sorted` field from a vector of its elements, storing `null` into a
nullable field, an integer into a `float` field, and integer-spelled elements in a
`vector<float>` literal.

One thing to know if you read binary files: a sized `f#read` answers a raw byte buffer, so
reading it back into a typed vector field needs the `as` that turns those bytes into
elements — `b.data = f#read(n * sizeof(single)) as vector<single>`. Without it the field
was silently left EMPTY. This is what the documentation always said; now the compiler says
it too.

### A write through a struct a function returned no longer disappears in silence

The same element, reached three ways — and only two of them were a mutation you could see:

```loft
hurt(first(s), 10.0);             // 0  — the write went nowhere
hurt(s.es[0] ?? E {}, 10.0);      // 10 — landed
for e in s.es { hurt(e, 10.0); }  // 20 — landed
```

Returning a struct hands back a **copy**, and that copy is released at the end of the
statement — so the write lands in something nobody can read. Nothing at the call site
distinguished the three: same types, no warning, no error. Found while giving enemies HP in
a game, where six tests failed at once and every one of them read as a bug in the thing
being mutated rather than in the one-line accessor.

The behaviour is unchanged — value semantics for a returned struct is the rule — but the
silence is gone:

```
warning[lost-write]: `hurt` writes to `e`, but the argument here is a value RETURNED by a
call — a temporary that is freed at the end of this statement, so the write is LOST.
```

It stays quiet where nothing is lost: a value the function built from scratch, the
write-it-and-return-it builder idiom, and a result you bind to a variable first (that copy
is still yours to read).

### `=` on a hash, sorted or index now replaces it, instead of adding to it

Assigning a list to a keyed collection added to whatever it already held:

```loft
h: hash<Entry[k]> = [];
h = [Entry{k:1}, Entry{k:2}];
h = [Entry{k:5}, Entry{k:6}];
println("{len(h)}");     // was 4 — all four entries, keys 1, 2, 5 and 6
```

`=` now means what it says: the collection holds exactly what you assigned. `+=` still
adds, unchanged. This applies to `hash`, `sorted` and `index`, as a local or as a struct
field — a plain `vector` always replaced correctly, which is part of why this went
unnoticed for so long. So did a single assignment onto a fresh `[]`, which is what most
code does; you only saw it if you assigned the same collection twice.

The loudest version was a struct holding **two** keyed collections of the same element
type. Those two fields are deliberately two views of one set of records, so filling either
one fills both — and the second assignment then added to a collection you thought you had
just replaced, giving a length of 4 for two elements. That case is fixed too, in the same
release; see below.

### Two keyed collections over one element type no longer destroy each other's records

A struct can hold several keyed collections over the same element type, and they are
deliberately several routes to ONE set of records — filling either fills both. Emptying
either one, however, used to free the shared records, so the other was left reporting its
old length over memory that had already been given back:

```loft
struct H { keyed: hash<E[k]>, ordered: sorted<E[k]> }
h.keyed = [E{k:1,n:"alpha"}, E{k:2,n:"beta"}];
h.ordered = [];                     // empty the other view
for e in h.keyed { … }              // was 4294967296:null — freed memory
```

The records now have an owner. The first-declared collection holds them; every later one
over the same element type is a view of them, and only the owner's records are ever freed.

Assigning to any of them replaces the whole set, so they always agree — which is the same
rule adding already followed: `h.by_k += [e]` has always added to every collection in the
group, not just the one you named. So `h.by_k = []` empties the set, and
`h.by_k = [e]` makes the set exactly `[e]`. The alternative — letting you empty one view
on its own — cannot work for a non-empty list, because the elements still go into the
group: you would be left with an index that does not index most of the records it is over,
and nothing to rebuild it with.

This is also what the documented `vector<T>` + `hash<T[k]>` pairing needed — there the
vector is the owner — and it holds for three or more collections, and for a group nested
inside another struct.

### …and removing one entry takes it out of every collection in the group

Removing a single entry used to be wrong in both directions. Through the secondary
collection it freed the record the primary still held, so the entry was still there and
its text read back `null`:

```loft
struct H { v: vector<E>, by_k: hash<E[k]> }
h.by_k[1] = null;                   // remove through the index
for e in h.v { … }                  // was 1:null — the record was freed underneath
```

And through the primary it never reached the secondary, which went on reporting an entry
over a record that was gone.

The entry now leaves every collection in the group and its record is freed once, whichever
one you spell the removal through — the same rule adding and clearing already follow. The
alternative, dropping one index entry and leaving the record in the primary, has no
sensible next step: `h.by_k[1] = null` followed by `h.by_k[1] = E{k:1,…}` would remove one
entry and then add to the whole group, leaving the primary with two records under one key
and nothing able to repair it.

Two smaller things had to be right for this to work, and are fixed with it: removing an
entry from the `vector` half of a group always removed the FIRST one regardless of which
you asked for, and removing through the `hash` half leaked the record.

### …and `e#remove` in a loop removes one element, not two

`#remove` inside a `for` loop takes the element out of the group too, and it removes
exactly one:

```loft
struct A { v: vector<E> }
struct B { by_k: hash<E[k]> }       // anywhere in the program
for e in a.v { if e.k == 2 { e#remove; } }
for e in a.v { … }                  // was 1:alpha — it took gamma with it
```

That one is worth reading twice: `struct B` is a different struct, and deleting it made
the same loop correct. A `vector<E>` is stored differently once any keyed collection over
`E` exists, and `#remove` was measuring the elements in the wrong unit for that layout —
so whether the loop worked depended on a declaration somewhere else entirely. The removed
element's record is now freed as well, so a long-lived collection that is filled and
drained no longer grows.

Three more things `#remove` now gets right: it takes the element out of every collection
in the group, the way `coll[key] = null` already did; walking backwards with
`for e in rev(v)` no longer skips the next element (or visits one twice, on a `sorted`);
and on a `sorted` collection that shares its records, `--native` used to remove nothing at
all while the interpreter removed correctly. `v.remove(i)` had the same
wrong-unit problem and is fixed with it.

### Walking a `sorted` collection backwards, or over a range, when it shares its records

Three ways of walking a `sorted<T[k]>` did not work once a keyed collection over the same
element type existed anywhere in the program — which changes how the collection is stored:

```loft
for e in rev(s.a)   { … }       // walked FORWARD, silently
for e in s.a[2..4]  { … }       // visited every element, all values zero
for e in rev(s.a[2..4]) { … }   // crashed
```

All three now answer what the same loop answers on a collection that does *not* share its
records — which is the point: how a collection is stored is not something you asked for,
so it must not change what your loop means. `index` collections were unaffected and are
unchanged.

### Two `index` collections over one element type now say so, instead of crashing later

Declaring two `index` fields with the same key over the same element type in one struct
was accepted, filled fine, and then panicked deep inside the compiler on the first
removal. An index keeps its tree links inside the element's record, so the two fields were
never two indexes — they were one, reached two ways. It is now refused where you write it:

```
error: 'also_by_k' and 'by_k' are both 'index<E[k]>' in the same structure — two indexes
cannot share records, because an index keeps its tree links in a field of the record and
one field of links cannot hold two trees. Give the second route a different kind ('hash'
or 'sorted' over the same records), or index a different key
```

Both cures work today. Two indexes on **different** keys were never affected — that is a
genuinely useful pair (two orders over one record set) and it fills and removes correctly.

### A `sorted` collection emptied entry-by-entry accepts new entries again

Emptying a `sorted<T[k]>` with `coll[key] = null` and then adding to it gave back the entry
you last removed — with its text already freed — and dropped the one you added:

```loft
s.a = [E{k:1,n:"alpha"}, E{k:2,n:"beta"}];
s.a[1] = null; s.a[2] = null;       // now empty
s.a += [E{k:9,n:"zz"}];
for e in s.a { … }                  // was 2:null, not 9:zz
```

Emptying it with `s.a = []` was always fine, and so were `hash` and `index`. Only the
by-removal route reached it, because that is the one that leaves the collection empty while
it still holds its allocation.

### …and the second route is actually filled, for every pair of kinds

Filling either collection is supposed to fill both. For three pairings the second one
never got the elements, and nothing said so:

```loft
struct HI { a: hash<E[k]>, b: index<E[k]> }
z.a = [E{k:1,n:"alpha"}, E{k:2,n:"beta"}];
len(z.b)                            // was 1 — the index kept the first, dropped the rest
```

`sorted` + `sorted` and `vector` + `sorted` stayed empty entirely, and `hash` + `hash`
built the right *number* of entries with every one of them naming the first record — so a
length check passed and every lookup but one missed. A secondary index that silently does
not contain your records is worse than one that fails to build: every lookup through it
answers "not found" for records that are demonstrably there, and a smoke test with a
single element passes.

All of it came from one fact. Every collection in a group finds its elements by record
number, so each element needs a record of its own — and two shapes did not give it one: a
`hash` normally packs its entries together for speed, and a `sorted` stores its elements
inline. Both now switch to one record per element when the collection is part of a group,
which is what the `vector<T>` + `hash<T[k]>` pairing already did. A collection that is
*not* part of a group is unaffected and keeps the faster layout.

One consequence worth knowing if you ever compared behaviour across files: whether a
`sorted<T[k]>` was record-backed used to depend on whether an `index<T[..]>` over the same
element type was declared *anywhere else in the program*, so the same two lines behaved
differently in two files. That is gone — group membership alone decides it.

### …and building the pair with a `{…}` literal fills it too, whichever field you write first

Everything above is about the collections once they exist. Building them in one literal
had its own hole, and which field you happened to write first decided whether you hit it:

```loft
struct S { data: vector<E>, lookup: hash<E[k]> }

a = S { data: [E{k:1,v:10}, E{k:2,v:20}] };              // len(a.lookup) was 0
b = S { data: [E{k:1,v:10}, E{k:2,v:20}], lookup: [] };  // len(b.lookup) was 0
c = S { lookup: [], data: [E{k:1,v:10}, E{k:2,v:20}] };  // len(c.lookup) was 2
```

The records were all there — `a.data[0].k` read `1` — but `a.lookup[1]` answered null,
which is exactly what a key that was never inserted answers. The two spellings of "not
found" are indistinguishable to the caller, so the fault read as missing DATA and sent you
to the insert.

A collection field is a small header, and each one used to be cleared at its own position
in the literal. Since putting a record in through one member also files it under the
others, a member written *after* the one holding the records wiped the index it had just
been given — and a member you left out was cleared later still. The whole group is now
cleared once, up front, before any of it is filled. All three lines above read `2`.

One thing follows from this that is worth knowing. If you fill **two** members in one
literal, you now add to the group **twice**:

```loft
struct HS { by_k: hash<E[k]>, by_v: sorted<E[v]> }
s = HS { by_k: [E{k:1,v:10}], by_v: [E{k:2,v:20}] };
len(s.by_k)                          // 2 — one record set, holding both
```

That is the same thing `s.by_k += …; s.by_v += …` has always done, and it is what "two
routes to one set of records" means. If you wanted two independent collections, give them
different element types — sharing one element type is what makes them a pair.

### A vector of keyed collections says so at the declaration, instead of failing later

`vector<hash<E[k]>>` used to parse. It could not do anything else: putting an element in
by literal was a type error, putting one in from a variable crashed the compiler, and
compiling the program natively failed with an error from `rustc`. Only the "declare it and
never fill it" path worked, and that one silently reported length 0.

It is now refused where you write it, with the shape that does work:

```
a `hash` cannot be a vector ELEMENT — a keyed collection has no element form anything
can write, so `vector<hash<…>>` could only ever be declared and stay empty. Hold it in
a struct and make a vector of THAT: the extra record is what the element would have
been anyway.
```

```loft
struct Box { by_k: hash<E[k]> }
boxes: vector<Box> = [];
boxes += [Box { by_k: [E{k:1, v:10}] }];
boxes[0].by_k[1].v                        // 10
```

`sorted`, `index`, `spatial` and `trie` all say the same thing. Nothing changes for
`vector<vector<T>>`, which was never affected.

### Reading a file straight into a struct field no longer leaks

```loft
b.data = f#read(8) as vector<single>;
```

held on to one store for the rest of the run. Storing any freshly-allocated value into a
vector field did — the assignment builds a hidden temporary to hold the right-hand side,
and that temporary was described as borrowing the struct rather than owning its own
storage, so it was never released. Passing the same value through a variable first was
fine, which is why this looked like a problem with the `as` cast.

### Reading a file works the same whether or not you name the result

```loft
println("{len(f#read(8) as vector<single>)}");     // was 1 — there are two
```

Using a `f#read(…) as vector<T>` on the spot — straight into `len(…)`, a `for … in` loop, a
call argument, or a struct literal — used to disagree with the same read bound to a
variable first. On the interpreter it answered a length one short and started one element
in, so iterating those eight bytes yielded `2.5` alone. With `--native` the same line did
not compile at all, reporting Rust errors against generated code. Both are fixed; a read
now behaves identically bound or unbound, and no longer holds on to a store either way.

The wrong value depended on the rest of the file, which is what made it so unpleasant:
declaring a `vector<single>` anywhere — even on a later line, even for something entirely
unrelated — made the read correct again, and deleting that line made it wrong. So a working
program could start returning wrong numbers because a variable somewhere else was removed.

### Taking an element out of a keyed collection you just built

`return lookup(n)[key]` — reading one record straight out of a `hash`, `index`, `sorted` or
`trie` that a function just returned — handed back a pointer into storage that same
function released on its way out. The record you got was whatever landed there next.

It usually looked fine, because released storage normally still holds its old contents, and
`--native` happened to make a defensive copy that hid it entirely. So the same program was
correct compiled and quietly wrong interpreted.

Every keyed collection is fixed, for every number of key fields, with or without a `??`
fallback — and so are the three shapes around it, which were separate faults with the same
shape:

- Reading through a **field** of what a function returned (`make_bag().items[k]`). The
  record lives in the bag's storage, and nothing named the bag, so nothing copied the
  record out before the bag was released. This one also caught the plain `vector` field on
  both backends.
- Binding what a function returned to a **local** first and reading that. Two things
  released the same storage — the collection's move and the temporary holding it — and the
  second release stole whichever storage had been handed that slot in between. Returning a
  record allocated in exactly that window, which is why the shape looked so narrow.
- Binding an element to a local before returning it (`e = make()[key] ?? d; e`) answered an
  empty record.

`e = make_bag().rows[i]` and `return make().rows` are both correct on both backends now.

### A `sorted` collection that quietly kept nothing

`s[key] = value` on a `sorted<T[k]>` inserted **nothing** — `len(s)` stayed 0 and every
lookup answered its fallback — if any struct anywhere in the program declared an
`index<T[…]>` field over the same element type. The struct did not have to be used, or
even constructed; declaring it was enough, and the collection that broke could be in a
different file.

Declaring that pair switches `sorted` to a different internal representation, and the
insert path did not know about it, so every write went to a lookup that missed. Removal
(`s[key] = null`) already knew; only the insert beside it did not.

### A native library that forgot to wire up a function now says so

If a library's native code exports a function but never registers the glue loft calls it
through, that function is dead — and you only found out when something called it, deep in a
program, possibly long after the library shipped.

loft now reports it when the library loads, naming the library and each affected function,
and tells the author how to generate the wiring so it cannot drift out of step with the
declarations again.

### A field's default now applies when you read JSON into it

A field declared with a default — `height: float = 1.5` — got that default from a struct
literal and not from a `text as Struct` cast, which wrote `0` instead. The same field had
two different "absent" values depending on how the record was made, and a key the document
simply omits is exactly the question a default is there to answer.

A default that is a plain value — `= 1.5`, `= 7`, `= "hi"`, `= true` — is now part of the
type, so it answers a missing key, an explicit `null`, and a struct literal alike. A key
the document actually carries still wins.

A default that has to be *computed* — `= 1 + 2`, `= mk()`, `= [1, 2]` — still applies
only where the record is constructed, because reading JSON does not run your code. If you
need one of those after a cast, put the field in the JSON or assign it afterwards.

### A function returning `T?` no longer leaks what it built

A function whose answer is an optional struct or vector — `fn pick(n) -> Cell?` — leaked
the record it allocated whenever the result was not bound to a variable. Writing the
call inline was enough: as an argument, inside a `??`, in an interpolation, or as a
statement on its own whose answer you did not need. One record per call, so a loop grew
the heap for as long as it ran.

Nothing pointed at it. The run was correct, the value was right, and the only signal was
a store-count warning at exit that a long-running or embedded program never reaches.

Binding the result first was always clean, and that is exactly what the compiler now
writes for you.

### "Native function not loaded" when the library was there all along

A program that calls into a native library could fail with

```
native function not loaded: its library's native cdylib is missing or stale
```

when nothing was missing and nothing was stale — and then succeed on the next run.

It only happened where one process runs more than one program: a debugger or REPL
session that loads a second file, an embedder, a test suite. Each compile recorded which
native functions it had stubbed, but that record was kept per PROCESS rather than per
program, so a second compile replaced the first one's. The first program then wired
nothing, kept its placeholder, and the placeholder is what raised — long after the
compile that caused it, and blaming the library.

The record now belongs to the program it describes. The message stays as it is: when a
cdylib really is missing or stale, that is still what it says.

### An early `return` no longer leaks what that path built

A function that answers a vector, whose LAST expression hands back one of its arguments —
or a field of one — leaked a store for every early `return` that built something fresh:

```loft
fn pick(n: integer, other: vector<integer>) -> vector<integer> {
  if n <= 0 { tmp: vector<integer> = [7, 8, 9]; return tmp; }
  other
}
```

Every answer was correct, which is what made it quiet. The only signal was a store-count
warning at exit, and one per early return — so a caller in a loop leaked once an
iteration, and a long-running or embedded program never sees that warning at all.

A function that answers a vector fills a buffer its caller owns. When the last expression
borrows an argument, only that last expression was being filled in; the early returns
still handed back a vector of their own, which the caller never adopts and nobody frees.
Every `return` in such a function now delivers into the same buffer. The same function
written to end in a call, or in a fresh local, was always clean — which is why this hid
for so long.

### Taking an element out of what a function just returned

A function whose answer is an index into a call — `make(n)[0] ?? Cell {}`, whether it is
the last expression or an explicit `return` — read the fallback for every input, and
crashed with an out-of-bounds index when compiled with `--native`.

The vector the inner call built was mistaken for the buffer the CALLER had allocated for
the result, so the callee cleared a single record as though it were a vector and built
into it; the index that followed then found nothing. A `??` fallback is exactly where a
wrong answer is designed to look plausible, and in a tower-defence dogfood this landed as
every enemy stepping onto the hex it was already standing on, from a three-line function
that reads correctly.

Binding the call to a local first was always right and still is — it just is no longer
the difference between a correct program and a silent one.

### A function that builds a vector and hands it back

`fn cells() -> vector<Cell> { c = [...]; c }` could deliver an EMPTY vector when its
result was used to fill a struct field, while the same function called directly in the
same run answered all 1024 elements. The write that followed then landed out of bounds
without saying so, which showed up as content disappearing from a neighbouring layer
while every write reported success.

The function's own return buffer and a scratch buffer inside its body could end up as the
same slot, and the second use silently retyped the first. Both spellings of building the
vector — a comprehension and an append loop — were affected, and what actually decided it
was how the value was handed back.

### Printing a struct that has a `hash` field

Interpolating a record with a `hash<…>` field — `println("{field}")`, or the message of a
failing assertion — segfaulted the interpreter and exited without a word on `--native`.
So did `to_json()` on the same record, which meant such a record could not be serialised
at all.

The formatter walked the hash with an out-of-date picture of how its entries are stored.
It now renders in key order, like every other collection, and round-trips through JSON:

```
{cells: [{q: 1, r: 2, v: 7}, {q: 3, r: 4, v: 8}], n: 2}
```

The cost of this one was mostly in finding it: an assertion message is evaluated only when
the assertion fails, so a test that should have said what went wrong crashed instead.

### Reading JSON into a struct no longer puts `null` in a field that cannot hold it

A field written plainly — `height: float`, not `height: float?` — is not allowed to be
null, and the compiler will tell you so if you guard it. But parsing JSON into such a
struct put a null there anyway, whenever the JSON said `"height": null`, whenever it
left the key out, and whenever the parse failed outright. The result was a value that
compared `<= x` as true and `> x` as false, so a check like `if height > climb` read a
wall as walkable — the right answer for the wrong reason. Write `?? 0.0` to defend it
and the compiler told you the guard was redundant.

Such a field now reads as its type's zero, and only a field you wrote `float?` /
`integer?` can be null. Ranged fields (`u8`, `u16`, `i32`) always behaved this way,
which is what made it look like a quirk of `float` and `integer`.

When a parse fails, `#errors` is what says so; the value is no longer a second, quieter
signal for the same thing.

One visible consequence: printing a struct skips its null fields, so fields that were
wrongly null used to be missing from the output and now appear as `0`. Printing a parsed
struct therefore fills it in rather than echoing what you fed it — and what it prints
reads back as itself.

Text fields follow the same rule now: a plain `text` the JSON leaves out reads as the
empty string, not as the one-character null. Write `text?` when "the document did not say"
has to be tellable from "the document said nothing much". The suite's own assertions were
the demonstration — `assert(!user.name, "missing name is null")` passed only because of
this, on a line where the compiler said it never could.

### A vector of narrow integers can be built by a comprehension

`[for i in 0..n { i as i32 }]` returned instantly at twelve elements and never returned
at thirteen. Each element was written eight bytes wide into a four-byte slot, so it
overwrote its neighbour, and past the initial allocation the write reached the vector's
own length — after which the append never finished.

Every narrow width was affected (`i8`, `u8`, `i16`, `u16`, `i32`, `u32`), just at
different sizes. Two things hid it: `vector<integer>` is genuinely eight bytes wide, so
it was always fine, and the `+=` append loop already wrote the right width, so the
obvious workaround worked and the comprehension looked like the odd one out.

### Parsing JSON into a struct or a vector works wherever you write it

`file(path).content() as vector<Row>` as a function's last expression answered an EMPTY
vector when compiled with `--native` — no error, just nothing, which every caller read
as "the file was empty". The same cast into a plain struct crashed the interpreter
outright, and refused to compile at all on `--native`.

The cast builds a new value out of the text it reads, but it was recorded as if it were
a *view into* that text. Anything borrowed has to be handled specially on the way out of
a function, and that handling is what lost — or corrupted — the result. What you got
depended only on where the text came from, which is why passing a filename worked and
passing the file's contents did not.

### An unknown function says its own name again

Calling a function that does not exist, and then reading a tuple out of the result, used
to report this and nothing else:

```
error: Expect token ;
  --> app.loft:3:18
  |
3 |     first = pair.0;
  |                   ^
```

The line it points at is correct as written; the mistake is on the line above. Across a
library boundary — one missing `use` — every file in the package went red this way and
nothing named the import. Now the call names itself, with a spelling suggestion where
there is one.

The same gap also rejected code that was always valid: a function declared *later* in
the file returning a tuple could not have its result tuple-accessed at all.

### Redefining a stdlib function says so once, and says where

Naming your own function after one the standard library already provides is refused —
but for some names the refusal came with a second error that was not real:

```
error: Cannot redefine 'sum'
error: Syntax error: unexpected '->'      ← there is nothing wrong with the `->`
```

Now there is one message, and it points at the function you collided with:

```
error: Cannot redefine 'sum' (already defined at default/01_code.loft:1657:53)
```

### Reading a tuple out of a vector is now as cheap as reading a struct

`v[i]` on a `vector<(float, float, float)>` was about **fourteen times** slower than
the same read on a vector of structs — enough that rewriting a mesh generator to use
tuples made it *slower* overall, even though the arithmetic got much faster. Every such
read was quietly allocating a scratch record, copying the element into it, and throwing
it away again. It now reads the element where it already lives: **379 ms → 12 ms** on
the reporter's benchmark, against 11 ms for the struct version.

The same mistake had a sharper edge. If you passed a `vector<(…)>` to a function, read
an element from it, and called that function twice, the first call could free the
vector's storage out from under the caller — after which the second call appended into
whatever had taken its place. On the interpreter that ended in a crash naming a record
of "-99 words"; compiled with `--native` it did not complain at all. Both are fixed.

### When a value might be null, the error now tells you something that works

`guess = (guess + x / guess) / 2.0` was refused, correctly: a variable divisor can be
zero, so the division might be null and `guess` cannot hold null. But the error
suggested casting with `as` — and the cast is refused for exactly the same reason, and
casting to `float?` instead came straight back to the first error. Following the
compiler's advice went in a circle.

It now names the cures that work:

```loft
guess = (guess + (x / guess)?) / 2.0;   // `?` — the type's default when null
guess = (guess + (x / guess ?? 0.0)) / 2.0;   // or your own fallback
```

### A big generated file compiles in a second, not a quarter of an hour

A program holding one long vector literal took time proportional to the *square* of
its length. A generated terrain file — a single 86 400-element `vector<integer>` —
took **over 13 minutes**, at 99 % CPU, printing nothing. Nothing was wrong with it;
it compiles correctly if you wait. But nothing says *"still working"* either, so it
reads as a hang, and five build targets that imported it simply stopped being run.

It now takes **under a second**, and the time grows with the length of the literal
rather than its square.

If you split a generated file into chunks to work around this, you can stop: the
cost was never per-literal, so chunking one function into several literals did not
help anyway.

### Every run starts in half the time

Running a loft file has a fixed cost before your program does anything, and you pay it
on every single run — which is the whole story for the edit-rerun loop, or a script you
invoke in a sweep a few hundred times.

Most of that cost was work with no result. The compiler consults a handful of small
tables while it checks your program, and each is decided entirely by the program's own
definitions, so each is the same answer every time. They were being recomputed for
every question asked — thousands of times per run, each one re-reading every definition
there is. A `println`-sized program rebuilt them **9 000 times**.

They are now worked out once. A run that hits the startup cache went from **18 ms to
9 ms**; one that does not, from **54 ms to 30 ms**. Nothing about your program changes —
the compiled result is byte-for-byte what it was.

### A library adding a function no longer takes a word away from you

If a library you use gained a `pub fn turn`, then `turn = 0` stopped compiling
anywhere in your program — a break that arrived on someone else's release, that
nothing announced, and that you could not prepare for. One package doing this took a
consumer's whole test gate red across 109 rows, on a commit that consumer never made.
Every short verb a library exports — `turn`, `step`, `run`, `wait`, `next`, `open`,
`send` — was a word its users could not name a variable.

A local may now carry a function's name, and the function is still there to call:

```loft
chr = 65;                  // a local, not an error
println("{chr(chr)}");     // …and this still calls the function → "A"
```

The parentheses are what pick between the two. This was already true of a parameter, a
`for` variable and a struct field — `fn go(chr: integer) { chr(chr + 1) }` has always
compiled — so what changed is that plain assignment, the typed local `chr: integer = 65`
and a tuple-destructuring element now agree with them instead of refusing.

Shadowing a name you actually rely on is still worth avoiding. It is now your call
rather than a library's.

### A page can save its work

A loft program exported with `--html` could draw, play audio, talk over a
WebSocket — and could not write a file. Every file call compiled and every one of
them quietly answered as if the file were not there, so a drawing editor in a page
looked like it saved and stored nothing. Finding that out meant grepping the
emitted page for `fs_`.

A page now has a filesystem, and it answers exactly what the interpreter and
`--native` answer for the same program:

```loft
fn main() {
  w = file("world.hxw");
  w += render_world();
  println("saved {file("world.hxw")#size} bytes");
}
```

`file(p)` with `#size` / `#next` / `#read(n)` / `+=`, `read_bytes` /
`write_bytes`, `delete` / `move` / `mkdir` / `mkdir_all` / `list_dir` / `is_dir`
/ `is_file` / `exists` — the whole surface.

It is the *page's* filesystem, not the visitor's disk. A browser cannot read
`/home/you/data.csv`, and nothing here pretends otherwise. What the page gets is
an immutable **base tree** you supply, plus every write it makes, kept in
`localStorage`:

```html
<script>
  loftBaseFS = { "/data/parts/tree.obj": "...", "/data/parts/rock.obj": "..." };
</script>
```

Reads take your writes first and fall back to the base tree, so closing the tab
keeps the user's work and `resetToBase()` throws it away. Set
`loftFSPersist = false` if you would rather it lasted only as long as the tab.

A program that only stores still gets the small engine-less page — the
filesystem does not drag a WebGL2 shim in with it.

### A vocabulary you ship, not one you download

A `trie<T[k]>` can already be read a page at a time — `store_load_prefix(local,
"vocab.store", "kerk", 20)` fetches what the prefix walk touches instead of the
whole file. But the records it returns sat wherever they were INSERTED, so
answering 20 of them meant 20 scattered reads, and most of the saving went back.

`store_persist_copy` writes the image with each record in its collection's own
order — key order for a trie — so one prefix is one run. On a 74,692-word
vocabulary, one 20-record query:

| | requests | fetched |
|---|---|---|
| a bound image | 19.9 | 1.28 MB |
| `store_persist_copy` | **4.9** | **0.32 MB** |
| downloading it whole | 1 | 5.17 MB |

```loft
words: trie<Word[w]> = []
for w in vocabulary() { words += [Word { w: w }] }
store_persist_copy(words, "vocab.store")   // ship this file
```

It is a second call rather than a change to `store_persist_bind` because binding
promises your references stay valid, and in a store a record's number *is* its
position — so the promise and the layout are the same thing. This writes a
rebuilt copy and leaves your collection untouched: nothing moves, every reference
still reads, and the file is not bound, so later writes do not reach it. Use it
when the data is final; keep `store_persist_bind` for a store you go on writing.

### A generator's memory follows what it is writing, not what it has written

Binding a store to a file first already keeps a big build small — the file is the
arena, and file-backed pages can be reclaimed where ordinary memory cannot. But the
kernel only works out which pages are finished by evicting some wrong ones first, and
a generator streaming a country's worth of data pays for every wrong guess.

`store_release(collection)` says it outright: *everything I have written so far is
finished*. It starts writing that out to the file and stops holding it in memory.

```loft
tiles: hash<TTile[tkey]> = []
store_persist_bind(tiles, "tiles.store")   // bind FIRST — the file is the arena
for cell in cells {
  // …fill the tile…
  store_release(tiles)                     // this one is done
}
```

On a 20 000-record build, calling it after every record: peak memory **44.3 MB → 2.2
MB**, and the wall clock does not move. Nothing about your data changes — no record
moves, nothing is freed, and a reference you were already holding still reads the same
value. Reading a released record just fetches it back from the file. So it is a hint:
call it too often and you lose a little speed, never an answer.

It pays when you write **in key order** and do not go back. A build that keeps many
records open at once leaves the store scattered with gaps that the allocator keeps
re-reading, and the same call then gives back nothing at all. If your generator streams
in cell order this is close to free — and if it does not, this is a good reason to sort
it first.

It is not `store_reclaim`: that hands back the file's unused tail and changes its size,
while this changes only what is held in memory.

### BLAS, LAPACK and any Fortran routine now bind through `#c`

A `vector` reaching C used to be a pointer **and** a count, always. That is right for
a C library written for loft, and wrong for every numeric library there is: Fortran
passes each argument by reference, so a BLAS routine takes a list of bare pointers and
learns the length from a separate `n`. Neither way of writing the declaration worked —
the honest one was refused for arity, and the one loft accepted handed each count to
the callee where it expected the next pointer.

**The C signature now decides.** Write the signature the header shows you and the count
appears exactly where the header puts one:

```loft
pub fn dgemm(transa: text, transb: text, m: vector<integer>, n: vector<integer>,
             k: vector<integer>, alpha: vector<float>, a: vector<float>,
             lda: vector<integer>, b: vector<float>, ldb: vector<integer>,
             beta: vector<float>, c: vector<float>, ldc: vector<integer>);
#c "dgemm_" "void(const char*, const char*, const int64_t*, const int64_t*, const int64_t*, const double*, const double*, const int64_t*, const double*, const int64_t*, const double*, double*, const int64_t*)"
```

Nothing is copied at the boundary — C writes its result straight into your vector — and
a Fortran argument list costs one slot per argument, so even LAPACK's largest drivers
fit without a shim. Existing declarations are unaffected: a signature that names a count
still gets one.

This is measured against **real OpenBLAS** — `daxpy_`, `dgemm_`, `dgesv_`, `ddot_` and
`dnrm2_` as the library exports them — on both backends, against a C program computing
the same answers.

**A routine that returns a `double` binds too.** The level-1 BLAS functions answer by
value (`ddot_`, `dnrm2_`, `dasum_`, and LAPACK's `dlange_`), and they used to be refused
with advice to write an ANSI-C shim for each one:

```loft
pub fn ddot(n: vector<i32>, x: vector<float>, incx: vector<i32>,
            y: vector<float>, incy: vector<i32>) -> float;
#c "ddot_" "double(const int*, const double*, const int*, const double*, const int*)"
```

A `double` comes back as `float` and a C `float` as `single`. Passing a float *into* C
by value is still refused, and still wants the 1-element-vector idiom — Fortran passes
everything by reference, so numeric bindings never need it.

**And the element type now has to match the C header.** A `vector` reaches C as a pointer
into loft's own element bytes, so `vector<integer>` (8-byte elements) against a
`const int *` was C reading every element from the wrong offset — and where C writes,
straight past the end of your vector. Both used to run to completion with wrong numbers.
The declaration is now refused, naming both widths:

```
parameter 5 is `vector<integer>` — 8-byte integer elements — but C parameter 5 is
`int *`, striding 4 bytes, so C reads every element after the first from the wrong
offset — and where C writes, past the end of the vector.
```

Write the element type the C header spells (`vector<i32>` for `int *`, `vector<float>`
for `double *`), or `void *` in C if the bytes really are opaque. **Note for BLAS
specifically:** the usual Linux build is LP64, so Fortran `INTEGER` is `int`, not
`int64_t` — and that is one thing loft cannot check for you, because both builds export
the same symbol names.

**Libraries that keep your buffer** — FFTW's plan/execute split, zlib's `z_stream`,
`sqlite3_bind_text(…, SQLITE_STATIC)` — bind by letting C own the memory. Allocate on
the C side, hold the pointer as an `integer`, and copy in and out with `memcpy` (libc,
so no shim and no `[c] libs` entry):

```loft
inp = fftw_malloc(n * 16);
p = plan_dft_1d(n, inp, outp, -1, 64);
load(inp, src, n * 16);      // #c "memcpy" "void*(void*, const void*, size_t)"
execute(p);
```

Handing such a library a loft `vector` instead is a use-after-free — loft frees the
vector at its last use, which is the call that handed the pointer over, and C reads
whatever took its place. For FFTW the C-owned form is what its own documentation
recommends anyway, since `fftw_malloc` is where the SIMD alignment comes from.

### Reflection can now read a value, not only a type

`type_of` tells you a record has a `text` at byte 16. `field_value` reads what is
actually there:

```loft
t = type_of(row);
for f in t.fields {
  v = field_value(row, f.position);
  if v.is_null { println("{f.name} is null") }
  else if v.kind == TextKind { println("{f.name}={v.t}") }
  else { println("{f.name}={v.i}") }
}
```

That is the half a generic serialiser or an ORM was missing — walking a value
without naming a single field. A field inside an inline struct is reached with a
path, `field_value(doc, [8, 0])` for `doc.origin.x`, because a nested record
reports its fields relative to itself. The offsets are walked one step at a time
and never added up: each step has to BEGIN a field of the type it is read
against, which is what keeps this a field read rather than a pointer.

Three answers stay apart, because code that confused them would write the wrong
row: nothing begins at that position, something begins there with no single value
to read (a vector, a keyed collection, a nested record), and the scalar holds
`null` — which is not `""`, `0` or `false`. Inside a generic body it is a compile
error rather than an empty answer, since an empty answer there is a row with no
columns.

### A hash costs a third less memory, and fills faster

`hash<T[key]>` used to give every entry a store record of its own — a header word each,
plus the allocator's rounding. Its entries now sit packed at a fixed stride in an arena
the collection owns:

| 1M integer keys | before | after |
|---|---|---|
| filling it (pre-sized) | 330 ms | **258 ms** |
| bytes per entry | 27.7 | **18.6** |
| store records for 2000 entries | ~2000 | **9** |

Lookups are unchanged. That is worth saying plainly, because the change was designed
expecting them to get faster: a lookup reads two random places — the bucket, then the
entry — and packing the entries moves where the second one lands without making it any
less of a cache miss. Density pays when you read things near each other, and a hash
lookup never does.

Nothing about your code changes. A store file written by an older loft is **refused**
rather than misread, because entries live somewhere new inside it; re-write it with this
version to load it again.

Building it also turned up an older bug worth naming, because it could lose data
silently: after a `for` loop over a keyed collection finished, the little scratch it
had used could be handed back twice, and the second time it might read whatever had
since taken its place — occasionally deciding to throw away the whole store the
collection lived in. A long-running program that iterated a collection and then kept
using it could find it empty, with nothing reported. That is fixed.

### Reading a binary file as text says so now

`file(p).content()` returns **null** when there is no text to read — the file is
missing, the path is a directory, or the bytes are not valid UTF-8:

```loft
write_bytes("logo.png", bytes);
c = file("logo.png").content();     // null — those bytes are not text
d = file("empty.txt").content();    // "" — that file really is empty
```

It used to answer `""` for all of them, which is the same thing an empty file
says. That does not just lose information, it inverts tests: a check of the shape
*"write bytes, read them back, compare"* **passed** on binary data, because both
sides were `""`. Reading the file was never the problem — asking for it as text
was. `read_bytes(path)` reads it exactly and round-trips with `write_bytes`.

Add `?? ""` where the distinction does not matter. The stderr warning that names
both readers now appears under `--native` too; it used to be printed only by the
interpreter, so the compiled build read binary in silence.

### git, as a library you call rather than a command you run

loft does not have `run(cmd, args)` and is not getting one: it hands back bytes
and an exit status, and every caller then re-parses text loft already knows how
to type. So the command lives inside a library instead:

```loft
use git;
for c in log(20) { println("{c.sha} {c.date} {c.subject}"); }
for f in changed("main") { println("{f.status} {f.path}"); }
```

`lib/git` answers `vector<Commit>`, `vector<Change>` and `vector<Stat>` — typed
values, not lines to split. It runs in a worker process, so the one library in
the tree that starts an external program is contained. And nothing composes a
command line: the library names a question and loft builds the command, so a
branch name or a path cannot turn into an option. Reading a repository needs the
`git#read` capability, separately from reading files.

The first thing it replaced is `tools/viewer/refresh.sh` — 135 lines of bash that
existed only because loft could not call git, and the review dashboard's
dependency on `jq` along with it. It also fixed a bug on the way: the bash split
`git log` output on tabs, and a commit subject may contain one.

The second was the tracker indexer, which used to walk a hard-coded list of
source directories minus a hand-written list of names that mean "ignored"
(`target`, `node_modules`, `pkg`…). It asks git now. That list had drifted both
ways: four tracked source trees were never indexed at all, and a leftover test
scratch directory was.

### A library can run in its own process, and you cannot tell from the code

A library adds one line to its own `loft.toml`:

```toml
[library]
placement = "process"
```

and its consumers do not change — the same `use`, the same typed calls, the same
values back. What changes is containment: a crash inside the library ends the
call as a loft error instead of taking the program's data with it.

Structs and vectors cross too, in both directions and at any depth — a `text`
field, a struct inside a struct, `vector<text>`, `vector<vector<T>>`. They are
not encoded into some second format on the way; they cross as themselves, in a
store both processes map. That is why a bigger value is not proportionally more
expensive to pass: a sixteen-element vector costs a fifth of a microsecond more
than a two-field struct, and a four-thousand-element one adds nothing you can
measure.

Passing by reference keeps meaning what it means. A library function that writes
to a struct parameter, or appends to a vector one, changes the caller's
value — placed or not. Passing the same value twice stays one value. And where
you have written `const` on a parameter, the crossing knows the library cannot
have changed it and skips carrying it home — about a tenth off a call taking a
twenty-thousand-element vector, for a word you were probably writing anyway.

If the library crashes, the call ends as an ordinary loft error naming the
library, and your own data is checked before you are told — not left to the
argument that it must be fine.

Or on another machine — `placement = "remote"`, with
`loft --lib-server <host:port> <library>` where it should run and
`LOFT_REMOTE_<NAME>=<host:port>` where it is called from. Still the same source,
still the same values; a `vector<Order>` goes on the socket as its own bytes
rather than as an encoding of itself. A library with nowhere to run refuses and
says which variable to set, rather than quietly running here on the wrong data.

A call that leaves the process costs around a microsecond, and one that leaves
the machine around 25 — almost all of it the round trip, not what you passed. So
this is for libraries you call to do real work, not for a getter inside a loop.
It is Linux only, and it does not apply under `--native`, which compiles the
library into your binary; in both cases the library simply runs in-process, which
is the same program without the isolation. Set `LOFT_REQUIRE_PLACEMENT=1` if you
would rather be told than quietly lose it.

`--lib-server` serves exactly the library you name and nothing else, but it is
not authenticated and not a sandbox — bind it where only what should reach it
can.

The engine host went through this first: its sockets, clients and event queue now
run in a worker if you ask them to, and a browser on the other end cannot tell.
Two things came out of doing it that apply to any library you want to place —
make the native private and let the public name be a wrapper over it, and give
the surface a call that answers a whole value instead of a cursor you step. Both
are better in-process too, which is usually how you know.

### A library whose native build cannot be used runs interpreted

`use <lib>` compiles a library to a native cdylib behind your back, and the deal
has always been that anything it cannot compile simply interprets. One case broke
the deal: a cdylib that **built** but that this run could not dispatch through —
linked against a different loft build, missing a system library, or replaced by
another `loft` running at the same moment — took the program down at the first
call to it, with the loft version of the function sitting right there in memory.

It now checks that it can actually reach each function before routing calls to
it, so anything it cannot reach interprets, with the same results. Running
several `loft` programs at once is no longer a way to lose one of them. You get
one line per library saying what fell back and that it costs only speed;
`LOFT_REQUIRE_NATIVE=1` turns that into a refusal instead, and
`LOFT_NO_NATIVE_LIBS=1` (now in `--help`) opts out of the whole mechanism.

The same runs turned up a second way to lose a library: loft keeps only the
eight most recent build artifacts per package, and it was counting the small
C shim a `[c] shim = "…"` package builds beside them. That shim is built once
and never again, so it was always the oldest file — and the first deleted, which
took every `#c` function in the package with it. Housekeeping now only tidies up
after itself.

### `loft update` reads loft.toml, not just loft.lock

Adding a dependency to `[dependencies]` and running `loft update` now locks it.
Before, `update` walked the lockfile alone, so a package the manifest had gained
was never looked up — and the summary counted lock entries, so it announced `all
1 packages up-to-date` with two declared. The lock silently kept lagging the
manifest, which is precisely what a lockfile exists to prevent.

`loft update --check` fails when the lock does not describe the manifest, so CI
catches the gap; a declared package that cannot be resolved at all is named
rather than skipped; and with no lockfile yet, `loft update` writes one.

### Words, and the prefix you actually wanted

`trie<T[k]>` keys a collection on one **text** field and answers what no other
kind could:

```loft
words: trie<Word[w]> = [];
words += [Word { w: "kerk" }, Word { w: "kerkweg" }, Word { w: "lonneker" }];

for x in words["kerk"..] { … }      // kerk, kerkweg — in key order
for x in words["kerk"..:20] { … }   // the first 20 of them
```

The prefix IS the query. Doing this on a `sorted` means inventing a successor
string — `words["kerk".."kerl"]` — which you have to construct, which is easy to
get wrong at a byte boundary, and which asks for a key *interval* rather than a
prefix. So loft refuses `t[a..b]` on a trie and tells you `sorted` is the kind
that answers an interval.

It also shares everything you already know: `+=`, `for` iteration (key order, no
sort), `.len()`, and `t["kerk"]` for the one record — `null` when absent, never a
neighbour.

And it does not have to be in memory. A persisted trie is read **a page at a
time**, so a phone typing one letter reads a few kilobytes instead of the whole
vocabulary:

```loft
words: trie<Word[w]> = [];
store_load_prefix(words, "https://…/vocab.store", "kerk", 20);   // ~4 pages
```

`store_load_key_text` answers one key the same way, and `store_bind_lazy` accepts
a trie image, so a lookup that misses simply fetches. The count is what makes it
worth having: the pages a query touches depend entirely on where the tree's nodes
sit in the file, so persisting now writes them in a cache-oblivious order — which
took one prefix query from 27 pages to under 3. The limit caps the *walk*, not
just the answer: asking for 20 of 459 matches reads 20 records' worth of pages.

And the mistake that prompted it now gets caught. `spatial<Word[w]>` on a text
key used to compile, count correctly, and then answer `null` for a key you had
just inserted — indistinguishable from "not found" wherever you called it. It is
refused at the declaration now, and the message points at `trie`. The mirror too:
a numeric key under `trie` points back at `spatial`, or at `sorted` / `index` for
an order on a number.

### An `i8`, `i16` or `limit(...)` key finds its record too

The last of the key widths that could be stored but not looked up. A collection
keyed on `i8`, `i16`, or `integer limit(min, max)` with a non-zero minimum
accepted every insert and counted them all, and then found none of them again:

```loft
prices: hash<Price[at]> = [];
prices += Price { at: 120, name: "a" };   // at: integer limit(100, 300)
prices[120]                               // was null — now the record you stored
```

Those widths store a value as `val - min` to save space, and the key was being
read back without adding the `min` again, so the two sides differed by exactly
that amount and never matched.

Two more things travelled with it. A `u16` key of 32768 or more came back
negative, so it too was unfindable — the same read, sign-extended. And a
collection keyed on any of these could be built and counted but never walked:
under `--native` a `sorted` iterated in the wrong order and a ranged scan came
back empty, while the interpreter stopped with `Unknown key type`.

As before, nothing about the stored data was wrong, only the reading of the key,
so an existing collection starts answering correctly as soon as you re-run.

### A variant satisfies the interface its enum satisfies

Passing a struct-enum variant to a function with an interface bound accepted the
call and then returned an empty value — `""` from a text method, `0` from an
integer one — with nothing said. Under `--native` it stopped with
`not yet implemented`.

```loft
x = AsA { a: A { nm: "ada" } };   // x is the VARIANT AsA, not the enum Any
c1(x)                             // was "" — now "ada"
```

Calling the method directly, `x.one()`, always worked and reached the method
declared on the enum. Now the generic reaches the same one. A variant that
declares its own version of the method still gets its own — the enum's is only
the fallback, exactly as on the direct call.

The workaround, annotating the binding with the enum type
(`x: Any = AsA { … }`), keeps working and is still worth writing where you mean
it; it is no longer required.

### A `float` key finds the record you stored under it

A `hash`, `sorted` or `index` keyed on a `float` or `single` field accepted every
insert, counted them all, and then found almost none of them again — and a `sorted`
kept only the last one. The key was being read one byte at a time, so `0.5`, `1.5`
and `2.0` were all the same key:

```loft
prices: sorted<Price[at]> = [];
prices += Price { at: 0.5, name: "a" };
prices += Price { at: 1.5, name: "b" };
prices += Price { at: 2.5, name: "c" };
len(prices)        // was 1 — now 3
prices[1.5]        // was null — now the record you stored
```

Nothing about the stored data was wrong; only the reading of the key. So an
existing collection starts answering correctly as soon as you re-run — there is
nothing to rebuild.

### A `u8` or `i32` key works under `--native` too

The same story on one backend only: a collection keyed on a sized integer
(`u8`, `u16`, `i32`, `u32`) looked up correctly under the interpreter and missed
every record under `--native`, where a `hash` answered "not found" and a `sorted`
answered a record whose fields all read `null`. Both backends now build the
lookup key the same way.

The remaining shifted widths — `i8`, `i16`, and `integer limit(min, max)` with a
non-zero minimum — are fixed too; see above.

### Inserting into a keyed collection is a little under twice as fast

`collection += Item { … }` used to build the item in a scratch record, then copy
that record into the collection and free the scratch — one allocation and one
deep copy per insert, for a value that was freshly written and had nowhere else
to be. It is now written straight into the collection's own slot. A million
inserts of a two-field record went from 933 ms to 505 ms.

Nothing changes about what you write. `collection += existing_item` still copies,
because there the item genuinely exists elsewhere.

Looking a key up got about a fifth faster too, on both backends — the work of
deciding *how* to compare a key is now done once per lookup instead of once per
record examined.

### `reserve(h, n)` for a hash you know the size of

`reserve` already gave a vector room for `n` elements. It now takes a `hash` as
well, where it sizes the bucket table:

```loft
cache: hash<Entry[key]> = [];
reserve(cache, expected_rows);
for row in rows { cache += Entry { key: row.id, value: row.value }; }
```

A hash rebuilds its whole table each time it outgrows one — filling a
million-entry hash rebuilds it seventeen times, re-placing every entry it already
holds. Saying the size up front skips all of that: a million inserts went from
618 ms to 352 ms, and the finished table came out **half the size** (10.2 MB →
5.3 MB), because growing doubles past what it needed while reserving asks for
exactly what you said.

Same promise as the vector form: it changes capacity and nothing else — not
`len(h)`, not the records, not which keys are found. Guessing low just means
growth resumes from there, and reserving a hash that already holds entries is
safe. `sorted`, `index`, `spatial` and `trie` have no capacity to set, and say so
if you ask.

### A collection can fetch what it is asked for

Bind a collection to a source and stop writing a loading step. A lookup that misses
fetches exactly that one record and keeps it, so the next lookup for the same key is
an ordinary read:

```loft
persons: hash<Person[id]> = [];
store_bind_lazy(persons, "sqlite:people.db");

p = persons[42];        // one SELECT, one row
q = persons[42];        // never leaves the process
```

There is no cache to manage, because the collection *is* the cache. The source can
be a `.store` image, an `http(s)://` URL served with Range, or a real SQLite
database — and for the database nothing has to be written down: the table, the
columns and the `WHERE` are all derived from the collection's own type. loft binds
to tables that already exist, reads only, and refuses a binding it cannot turn into
a query rather than serving it wrongly.

Traversal is the point. Reaching the same person by a key lookup, by
`store_lazy_query(persons, "name LIKE 'Ada%'")`, and by walking from their employer
gives you the **same record** — identity falls out of the collection, so there is no
identity map to keep in step.

Two things stay honest on purpose. `len` is the count of what you have fetched, not
of what the table holds, and iteration walks the same. And a fetch that could not
reach the source never answers `null`: `store_lazy_error` says why, and
`store_lazy_faults` keeps counting until you acknowledge it — so a traversal that
lost data cannot report itself healthy.

That last promise had a hole, and a refused binding fell through it. A `.store`
image is read a page at a time, which only a `hash` supports — so binding a
`trie`, `sorted`, `index` or `spatial` to one could never work. It used to answer
`true` anyway, then `null` at every lookup, with `store_lazy_error` empty (whose
documented meaning is *"reachable, genuinely no such key"*) and zero faults. A
search box bound to a source it could not page showed "no results" forever, in
perfect health.

```loft
if !store_bind_lazy(words, "vocab.store") {
  store_load(words, "vocab.store");     // whole-image, and it carries every kind
}
```

`store_bind_lazy` now answers `false` for a kind it can never serve — at the call
that is wrong, not at some later lookup, because the kind is known without reading
anything. The refusals it can only learn while fetching (a foreign layout, an entry
holding a `vector<text>`) were equally silent and now reach `store_lazy_error` too.
An unreachable source still reports its own connection error, and a binding that
works still says nothing at all.

**A database loft has no built-in driver for is served by a driver you write**, in
loft — and a program may now have one per collection type:

```loft
fn lazy_fetch(coll: hash<Person[id]>, source: text,
              key_int: integer, key_text: text) -> integer { … }
fn lazy_fetch_orders(coll: hash<Order[id]>, source: text,
                     key_int: integer, key_text: text) -> integer { … }
```

The name after `lazy_fetch_` is yours; what a driver serves is read from its
collection parameter. That matters beyond convenience: with one driver per
program, a *second* lazily-bound collection was filled by the first one's driver
whatever it was written for, so a `Person` landed in a `hash<Order[id]>` and came
back out as an `Order` — a record that looks like data and is not. A collection
whose type has no driver now says so, naming the type, and no driver runs for it.

Helpers may share the prefix: past the exact name `lazy_fetch`, a function only
counts as a driver if it takes a keyed collection first — so `lazy_fetch_row(n)`
is just a function.

### An enum works above the line that declares it

Order stopped mattering for enums, the way it already did not matter for functions
and structs:

```loft
fn probe() -> text { c = Colour.Green; return "{c}"; }
enum Colour { Red, Green }
```

That used to be `Unknown variable`, and across two files in a package it was
`Unknown type null — did you mean 'JNull'?` — naming a type and a suggestion the
author never wrote, because an unresolved name was being handed on as a resolved
type of `null` instead of as "not known yet".

Underneath it was something worse, and quieter. An enum a module names before the
importer declares it is reached through a shared definition, and that definition was
never getting a runtime type — so every variant of it rendered as `unknown`. The
same gap gave the enum zero width at layout time, which meant a struct field
declared *after* an enum-typed field lost its position entirely:

```loft
struct Sess { s_a: integer, s_c: Colour, s_b: integer }   // s_b had no position
```

Both are fixed, and the enum keeps its identity: it compares, matches and renders as
the variant you wrote, on both backends.

### Two libraries can no longer both answer a bare name

If `use hex_world;` and `use hex_voxel;` each export a `Chunk`, writing bare
`Chunk` used to bind to whichever was imported **first** — so swapping the two
`use` lines, and changing nothing else, changed what the source meant. A struct
gave itself away eventually (`Unknown field Chunk.ck_cells`, naming the struct you
did not mean); a shared function or constant did not, because both orders compiled
and ran and simply answered differently.

Now it is an error that names both:

```
error: `Chunk` is declared by more than one package here —
       write hex_voxel::Chunk or hex_world::Chunk to say which
```

It is reported where you write the bare name, not at the `use` line, so two
libraries sharing a name your program never writes bare keeps working. Qualifying
always works, and your own definition still shadows both.

### Advice that no longer sends you to an import you already wrote

Calling a function loft cannot find used to suggest the package that publishes it
— even when your build had already resolved a *different* package of the same
name, from `--lib`. The advice was to add an import that was on line two, and
following it changed nothing. Now that case says what is actually wrong: the
resolved package does not have the function and the published one does, so these
are two different packages sharing a name.

### Using a type no longer depends on having written its name down

Inside a package, a module could name a type its entry declares later only where the
name appeared in a *declaration*. The same type in an expression was rejected — so
`r: Roofs = Roofs { ... }` compiled and `r = Roofs { ... }` did not, and adding the
annotation was the fix for an error that never mentioned it. Constructions, vector
literals, `for` loops over them, `sizeof(T)` and `type` aliases all work now.

And one mistake gives one error again. An error inside a module used to abandon the
file that was waiting on it, so the types *that* file declared quietly vanished and
the run added a second error saying one of them was undefined — pointing at a line
where the declaration was plainly visible.

### A field can name a type from a module that loads later

Inside a package, a struct field whose type is declared in a module the package
loads *after* the one that names it was left out of that struct's storage. The
field could be written, and the write landed outside the record — in whatever
record happened to sit next to it. That is why it read as random: the same program
gave a crash on one run, a runaway allocation on the next, and a clean pass on the
third, and the failure never pointed at the field.

Only the load order decided it, so the same code was correct or corrupt depending
on which module `use`d which. It is now correct either way — including when the
field is a `vector<T>`, a `T?`, or a struct that itself holds one.

### A C binding says what it does on wasm

A library bound straight to a C library (`#c`) cannot work in a browser or in a
wasm module: there is no way to open a shared library there. That used to be
discovered late and badly — one build linked against the wasm sysroot's own libc
and then crashed at the call, another printed a page of Rust errors naming loft's
internals. Now both wasm targets say it once, in a sentence:

```
error: loft: `client_info` is bound to the C symbol 'mysql_get_client_info' with #c
       (package `mariadb`), and the wasm (wasip2) target has no C ABI to reach it —
       a wasm module cannot open a shared library. …
```

Only a call is refused, so a library may declare `#c` bindings and still build for
wasm as long as the wasm program does not reach one. And
`c_library_available("…")` — the question to ask before calling into an optional
backend — now compiles there too, answering `false`.

### A sandboxed script cannot call C on a capability alone

Granting a sandboxed script a capability such as `db#read` used to let it through
to a `#c` binding, and from there into arbitrary C. A capability describes what
data a script may touch; it cannot describe "and any machine code may run here".
That second question has always had its own answer — `native_ffi` — and C bindings
are now gated by it, exactly like Rust ones. Allow-listing a whole library still
admits it: that is you vetting the library.

### `store_verify` on a collection inside a struct

`store_verify(firm.people)` reported a corruption that was not there — it read the
collection's root as if it were the wrapping struct. Verifying the struct itself
was always right, which is what made the false alarm convincing. Both now report
what is true.

### Reflection knows which fields can be null

`type_of` and `type_named` now report `nullable` on every field, so a generated
schema can carry the constraint:

```loft
for f in t.fields {
  col = "{f.name} {sql_type(f.kind)}";
  if !f.nullable { col += " NOT NULL" }
}
```

It is not something the stored bytes could tell you — a `text?` occupies exactly
the same space as a `text` — so it reaches you only because the compiler records
it. Whether a field is `const` is deliberately not reported: that constrains loft
code rather than the data.

### `type_named("Row")` when you only have the name

`type_of` needs a value. When the type name arrives from a config file, a
database catalogue or a command line, there is nothing to pass it:

```loft
t = type_named(wanted);
if t == null { println("no such type") } else { println("{t.name} has {len(t.fields)} fields") }
```

`TypeInfo?` — a name this program has no type for answers null rather than an
empty-looking shape, so a typo cannot read as a type with no fields.

### `type_of(x)` tells you the shape of a type

A program can now ask what a type IS — its fields, their types, their byte
offsets, an enum's variants — without a JSON round-trip and without writing the
shape down a second time:

```loft
t = type_of(row);
println("{t.name} ({t.size} bytes)");
for f in t.fields { println("  {f.name}: {f.type_name} @{f.position}") }
```

`t.kind` says which of the rest to read: a record has `fields`, an enum has
`variants`, a vector has an `element`. Empty is the honest answer for a kind that
has no such thing.

The argument is read for its **type** and is not evaluated — the same contract
C's `sizeof` has — so pass a variable, a field or a parameter rather than an
expression with a side effect. It also does not work inside a generic, where the
type parameter has no concrete type yet.

This is what a generic serialiser, an ORM mapping or a schema check needs, and
until now only a JavaScript reader of a loft value could see it.

### `{x:j}` produces JSON for two shapes that used to break it

A struct holding an **enum field** wrote the variant name bare:

```
{"kind":Circle {"r":2}}      // not JSON — `Circle` is an unquoted token
```

so `json_parse` rejected the whole document and *no* field of that struct could
be read back, not just the enum. It now nests the same object a bare enum
always produced:

```
{"kind":{"Circle":{"r":2}}}
```

And a **`text?` holding null** came back as a one-character string containing a
NUL rather than as JSON `null` — an absent value arriving as a present, corrupt
one. It is `null` now, so absent and empty stay the two different answers the
type exists to keep apart:

```loft
"{ Note { note: null, n: 4 } :j}"   // {"note":null,"n":4}
"{ Note { note: "",   n: 4 } :j}"   // {"note":"","n":4}
```

Both round-trip through `.parse` in both directions. `{x}` — the debug form —
is unchanged.

### A format string can hand a value of your own type to the type it builds

A format string whose target type implements the interpolation contract already
handed over its parts — the bytes you wrote go to `lit`, an interpolated value to
`hole_<kind>`. Holes had to be scalars. Now a hole can be a value of your own
struct or enum, and its method is named after the type:

```loft
fn hole_sql_ident(self: SqlText, v: SqlIdent?)    // SqlIdent -> hole_sql_ident
```

That is what lets a builder treat one hole differently from the rest. A SQL table
name cannot be a bound parameter — no placeholder stands for it — so a safe query
builder has to put it in the statement itself. Making it a *type* is what keeps
that safe: nothing constructs a `SqlIdent` but a constructor that refuses anything
which is not a name, so there is one place to check rather than a rule to remember.

```loft
tbl = ident("orders");                              // null if it is not a name
q: SqlText = "SELECT id FROM {tbl} WHERE name = {n}";
```

`tbl` becomes syntax; `n` becomes a bound value; and a refused `tbl` leaves the
statement with no text to send at all.

A hole kind your type does not accept is still a compile error naming the method
to add, and never a quiet fall back to rendering the value into the text.

### `chr(65)` gives you `"A"`

There was no way to turn a code point back into text — `ch as integer` took a
character apart, and nothing put one together. `chr` does:

```loft
chr(65)       // "A"
chr(20013)    // "中"
chr(128512)   // "😀"
```

Use it to decode an escape (`\u{...}`, an HTML entity) or to rebuild text a
character at a time. A number that names no character gives you `""` rather than
an error — a surrogate, anything past `U+10FFFF`, a negative number, and `0`.

Its byte-level partner `text_from_bytes` has been there for two releases; it was
listed on the wrong reference page, which is why it reads as new. Both are now
under **Text**, together with `byte_at`.

### `for` over text no longer stops at a NUL

A text can hold a NUL — `text_from_bytes([65, 0, 66])` builds one, and `len`, `size`,
`byte_at`, `find` and slicing all read straight past it. Only the loop stopped there:

```loft
s = text_from_bytes([65, 0, 66]);
for c in s { n += 1; }          // n was 1, while len(s) said 3
```

Everything after the first NUL was dropped, silently, and any check written in terms
of the length agreed the data was there. It bit exactly the code `text_from_bytes`
exists for — a decoder that assembles bytes and then walks the result.

A loop over text now yields `len(s)` characters however the text is spelled, the way
a loop over a vector yields `len(v)` elements whatever the elements are. The NUL
position reads as `null`, which is what `s[1]` has always answered there.

Do keep in mind that a NUL survives a round trip through *bytes*, not through
*characters*: `character`'s null is code point 0, so the two cannot be told apart.
To preserve NULs, walk `for i in 0..size(s) { s.byte_at(i) }` and keep the data as
`vector<u8>`.

### A builder function can return the string it builds

A type that builds itself from a format string could be reached by assigning to it
or by passing it to a function, but not by returning it — which is the shape such a
type most wants:

```loft
fn where_name(name: text) -> Query { "SELECT * FROM t WHERE name = {name}" }
// was: expected Query, got text on return from block
```

Every builder had to route through a local first, for no reason an author could
see. It now works from a block tail, from an explicit `return`, and from an `if`
tail whose branches each build one. What a value can be built from is unchanged —
a string written inside a `{…}` hole is still a value handed to the type, not a
second thing being built.

### A tuple pattern the compiler cannot accept now says so

Writing a `..` rest in a tuple pattern used to stop the compiler dead:

```loft
match t { (first, ..) => "got {first}", _ => "no" }
```

No error, no output — the parser looped on that token forever. loft puts no time
bound on itself by default, so an editor calling the compiler simply stopped
answering. Tuple arity is fixed by design, so there is nothing a rest could stand
for; the point is that unsupported syntax has to be *refused*, and this said
nothing at all.

Both that and a pattern with more elements than the tuple has (`(a, b, c, d)` on a
three-element tuple) now come back as one error naming what to write instead. The
arm loop also carries the rule it was missing — every arm must consume something —
so a shape nobody has thought of yet ends in a message rather than a stuck build.

### A destructured name may be a stdlib name

`trim = 7` has always made an ordinary local — `trim` being a stdlib method does not
stop you. The same name in a destructuring did stop you:

```loft
(chq, chr) = px_to_hex(cx, cz);      // "requires plain variable names"
```

The message was the puzzle: `chr` *is* a plain variable name as far as the author
knows, none of the three errors named `chr`, and the fix was a rename that ordinary
assignment would never have needed. Both forms now agree on what a name may be — and
where one is genuinely refused (a global function like `chr` or `print`), you get one
error that says so by name.

### `return x ?? "fallback"` gives you the fallback

A function that returned a fallback text straight from a `??` handed back an empty
string instead — silently, exit 0 — while writing the same thing through a local
first was right:

```loft
fn label(row: Row) -> text {
  got = row.name();
  return got ?? "<unnamed>";     // gave back "", not "<unnamed>"
}
```

`--native` did not compile the same program at all, which is the only reason it
was not worse: an empty string where a fallback belongs is a wrong ANSWER, and
`<unnamed>` never appearing looks like the data was fine. The same fault reached
any `if` or `match` in return position whose arms build text, `return if x { "a-{n}" } else { "b" }`
included.

Alongside it, `text?` is now one type however you obtained it. A `match` whose
arms call the same method on different types could be refused with a message that
quoted the same name twice —

```
error: cannot unify: text? and text?
```

— which is now what it looks like: two identical types, and they unify.

### A function ending in `v[i].field` gives you the field

A function whose body ended in a value read out of a collection — no `return`, the
expression on its own as the last line — handed back an EMPTY vector when compiled
with `--native`, while the same program run with `--interpret` was right:

```loft
fn bytes_of(w: World, tag: integer) -> vector<u8> {
  at = section_at(w, tag);
  if at < 0 { return []; }
  w.sections[at].bytes        // gave back nothing, on --native only
}
```

Writing `return` in front of the same expression worked, which is what made it so
hard to see — the fault was in a four-line getter, and it showed up as the data
being absent somewhere else entirely. Records and struct-enums had the same hole.

### A `vector<integer>` is no longer accepted as a `vector<u8>`

Handing a `vector<integer>` to something that asked for a `vector<u8>` compiled
without a word, and then read each 8-byte number as eight separate bytes:

```loft
b: vector<integer> = [72, 105];
print(text_from_bytes(b));      // "H " — silently, exit 0
```

`len()` agreed the whole time (the count is stored), so any check written in terms
of the length passed while only the bytes were wrong. Writing the same mistake as
a literal was already refused, so the two spellings disagreed. Now both are
refused, and you convert the elements yourself — which is what the machine would
have had to do anyway.

### A `&File` parameter works

`fn write_all(f: &File, …)` was accepted and then nothing you can do with a file
worked through it — `f#read`, `f#exists`, `f#format = …` and `f += value` each
failed, and one of them complained about a loop you had not written. `File` was
the only type whose `&` form was special. All four work now. (`&` on a parameter
is still only worth writing when you REASSIGN the whole binding — loft will say so
if you do not.)

### A generator's store file is the size of what it holds

A collection bound with `store_persist_bind` keeps its file as its working
storage, and that storage grows in jumps and never shrinks by itself. So the file
left at the end was whichever jump the build happened to stop on — up to 57% bigger
than the data, and two builds could differ by 133% holding identical records:

```
40,000 features   ->  7,196,280 bytes
60,000 features   ->  7,196,280 bytes     // half as much data again, same file
```

That is fixed: letting go of the collection gives the spare room back, so the file
a program leaves behind measures its content. Nothing to change — if you were
calling `store_reclaim` at the end of a build, it now has nothing left to find and
you can drop it. It is still the right call in the MIDDLE of a run, when something
has been unloaded for good.

### The compiler produces the same bytecode every time

Compiling one file twice with the same binary could produce different bytecode and
different stack slots. Every run computed the right answer, so nothing was visibly
wrong — but a compiled build was not reproducible, and neither was any before/after
comparison of generated code.

### A format string can build a value, not just text

`"… {x} …"` normally joins everything into one `text`. When the type it is
assigned to says so, it now **builds that type instead**, and the type is told
which bytes you wrote and which came from a value:

```loft
name = "ada";
q: Query = "SELECT * FROM t WHERE name = {name}";
// calls q.lit("SELECT * FROM t WHERE name = ") then q.hole_text(name)
```

A type opts in by defining `lit` plus a `hole_…` method per value kind it takes.
There is no new syntax — the type you assign to (or a parameter, or a return
type) is what decides — and plain `text` behaves exactly as before.

This is what lets a library build a SQL statement, a shell command, an HTML
fragment or a file path in which **an interpolated value can never become
syntax**: the value has no route into the text, because the only path in is
`lit`, and `lit` only ever receives bytes from your source file. Once a value has
been rendered into text it is indistinguishable from text you wrote, so the fix
is not to render it.

The database clients use it, so a query is written the way you would write it
anyway, with no placeholders to count:

```loft
q: SqlText = "SELECT id FROM users WHERE name = {name}";
db.db_rows(q)
```

See [LOFT.md § Building a value instead of text](doc/claude/LOFT.md).

### Removing from a container you hold a reference into no longer loses your write

Handing a function an element of a vector *and* the vector, where the function
removes from it, threw the write away:

```loft
fn shift_then_write(target: &Box, all: &vector<Box>) {
  all.remove(0);        // the element `target` names moves down one
  target.n = 99;        // went nowhere
}
shift_then_write(boxes[2], boxes);
```

Removing renumbers the remaining elements, so `target` kept pointing at the slot
the element used to occupy. Nothing said so — no warning, no crash, just a value
that never changed. The same thing happened without a call, with the removal and
the reference in one function.

loft now **refuses the program**:

```
error: cannot remove from `v` while `c` references an element of it — a removal
  renumbers the remaining elements, so a write through `c` would no longer reach
  the element it names. Move the removal after the last use of `c`, or bind
  without `&` to work on a copy
```

The same applies to the other two things that can end the place a reference names:
**writing a key field** through it (which would leave the element reachable by no key)
and **replacing the container** it points into. Both used to hand you a copy and a
warning; both are now errors.

It only fires while the reference is still in use — finish with it before the
removal and nothing changes:

```loft
c = &v[0];  c.n = 99;  v.remove(2);   // fine — `c` is done before `v` changes
c = &v[0];  v.remove(2);  c.n = 99;   // refused
```

For the call version, pass the **index** instead and read the element again after
the removal:

```loft
fn shift_then_write(idx: integer, all: &vector<Box>) {
  all.remove(0);
  all[idx - 1].n = 99;
}
```

Plain (non-`&`) bindings are unaffected: `c = v[0]` still gets its own copy when
the container changes underneath it, and still tells you it did. The difference is
what you asked for — a plain bind already meant "give me a value", so a copy is
consistent with it; `&` meant "give me a live link", and quietly handing back a copy
would make that a lie.

The error is also the choice that keeps the door open. loft can always *drop* an error
later, so if it ever gains the machinery to follow a reference through these changes,
every program that compiles today keeps compiling and the refused ones start working.
Shipping the silent copy instead would have made that copy permanent.

### An element taken out of a returned value stays valid

Binding an element out of a struct a function just returned gave you a reference
into storage that had already been handed back:

```loft
plan = roof_plans().items[0] ?? RoofPlan {};
```

It read correctly at first and turned to zeroes once other allocations reused
that memory. In the program that reported it, the same binding answered three
questions correctly and the fourth wrongly, in one function with nothing between
them but ordinary library calls — a test asserted a roof's ridge height and got
its eave's. A value that is right the first time and wrong the fourth cannot be
found by reading the code, which is the worst shape this kind of bug takes.

The same expression **inside a loop** was a second, separate fault with the same
cause and a different trigger. There the damage depended on what else the loop
did: with an allocation between the bind and the read, the element was read out
of memory that had already been reused, so

```loft
for i in 0..50 {
  e = make().items[0] ?? P {};
  o = other(i);              // claims the memory `e` points into
  sum += e.a;                // read 0, 1, 2 … instead of e's real value
}
```

summed the loop counter instead of the element. Without such an allocation it
returned the right answer while still reading freed memory, so it was invisible
until the arena poison detector was pointed at it.

Both are fixed. The workaround the issues documented — binding the returned value
to a local first, then indexing it — is no longer needed:

```loft
held = roof_plans();                          // no longer necessary
plan = held.items[0] ?? RoofPlan {};
```

### A closure that builds a struct no longer crashes

A closure that captured something and called a function returning a struct
crashed the interpreter outright:

```loft
k = 4.0;
pt = fn(i: integer) -> Point { make_point(k) };
p = pt(0);                       // SIGSEGV
```

Worse, the compiled build got it right, so the same program behaved differently
depending on how you ran it. Both halves were needed to trigger it — a closure
that captures nothing was fine, and so was one that builds the struct inline
instead of calling for it — which is why it survived so long and then hit a real
program doing something perfectly ordinary.

It is fixed, and the two backends agree again.

Using such a result *inline* also leaked one record per call — the buffer holding
the returned struct had no variable to hang its free on, so nothing released it.
That is fixed as well, and neither form leaks now:

```loft
total += pt(r).a;            // no longer leaks
p = pt(r); total += p.a;     // and neither does binding it first
```

### Removing from a keyed collection, without the crashes

Three ways of removing from a keyed collection went wrong, all now fixed:

- Removing entries from an `index<T[..]>` whose records own a `text` could
  crash while the loop was still running.
- Simply *declaring* a struct with both a `sorted<T[..]>` and an `index<T[..]>`
  over the same element type was enough to break that type everywhere — the
  interpreter hung and the compiled build produced wrong code. No removal
  needed; the declaration did it.
- Removing by key from a `spatial` collection is covered in its own entry below.

### `spatial` collections answer to a point

A `spatial<Mob[x, y]>` could be appended to, iterated, counted and range-sliced,
but the plain point subscript did not work — in three different ways, which is
why it read as one small bug:

```loft
mobs: spatial<Mob[x, y]> = [];
m = mobs[3, 6];                          // crashed
mobs[3, 6] = Mob { x: 3, y: 6, hp: 10 }; // could destroy the collection
mobs[3, 6] = null;                       // corrupted the store
```

Reading a point crashed the interpreter with an index-out-of-bounds while the
compiled backend answered correctly, so the same program behaved differently
depending on how you ran it. Assigning at a point that held nothing did not
insert — it wrote over the collection itself, and four elements read back as
one. Removing was the case that got reported, and it either corrupted the store
or refused to compile.

All three now work, and behave as they do on a `hash`: `xs[x, y]` reads (`null`
when the point is empty), `xs[x, y] = value` inserts or replaces, and
`xs[x, y] = null` removes. Note the coordinates are separate subscripts here —
`xs[3, 6]` — where the range forms parenthesise them, `xs[(3,6)..(9,9)]`.

### A crash report you can still read afterwards

When loft dies of a segfault it prints what it was doing: the last opcode, where
it was in the program, and which function. That went to stderr only — so a build
that filters stderr threw it away, and the one run that could explain the crash
was also the one run you cannot repeat.

The report is now written to a file as well: `.loft/loft-crash-<pid>.txt` next to
the package, or your temp directory when there is no `.loft/`. The report on
stderr names the file it wrote. Set `LOFT_CRASH_FILE` to put it somewhere
specific, or set it to empty for the old stderr-only behaviour. A run that does
not crash writes nothing.

### Removing from a collection now gives the memory back

Removing an element unlinked it and stopped there. The element's storage — and
anything it owned, a `text` or a nested `vector` — was never released, so a
program that keeps a collection at a steady size still grew forever:

```
cycle 0: 300 records live, 0.10 MB claimed
cycle 5: 300 records live, 0.56 MB claimed     // same 300 records
```

Emptying a collection and refilling it grew the store rather than reusing the
space. Every collection kind was affected and every element shape; records
holding only numbers leaked least, which is why this could sit unnoticed.

Now `c[key] = null` and `e#remove` release what the element owned, the space
returns to the free list, and refilling reuses it — a long-running program that
adds and removes stays flat instead of climbing. Nothing to change in your code.

If you were working around it by reusing element objects instead of removing
them, you can stop.

### Reading a collection no longer costs it anything

`for r in collection` over a `hash`, `spatial` or `index` walks a key-ordered
snapshot, and that snapshot was left behind in the collection's own storage
afterwards — about 4 bytes per element, on every pass. A program that reads a
collection inside a loop grew for as long as it ran, without adding anything:

```
built:            2,000 records, 0.29 MB
after 50 reads:   2,000 records, 0.67 MB     // same 2,000 records
```

For a collection bound to a file with `store_persist_bind` it was worse, because
the storage is the file and the file outlives the program. Each run started from
what the last one left, so a program that only ever READ a 4,000-record
collection took its file from 566 KB to 1.3 MB over sixteen runs.

Reading is free now, in memory and on disk. Nothing to change in your code — and
if you had a rule about not iterating a bound collection, or about stat-ing the
file before touching it, you can drop it.

### `store_reclaim` leaves a little room

`store_reclaim` trimmed a store to its exact contents, which sounds like the
point of it and was not: the store is still in use, and it grows by 7/3 when it
runs out, so the very next thing the program did made it 2.33× bigger. On a bound
collection that meant the call gave back 40% of a file and the next read took
more than that back.

It now keeps an eighth of the content as room to grow — the same margin a freshly
bound file gets. So the number it reports is a little smaller, a store that is
already the right size reports `0` instead of shrinking into the cliff, and
calling it no longer makes a file bigger than never calling it.

### `reserve(v, n)`, for when many vectors grow at once

Appending to a vector doubles it when it runs out, which is the right default and
almost never something you think about. It becomes something you think about when
*many* vectors grow together — a generator reading a stream and appending each
item to whichever collection owns it. Every vector is then sitting immediately
before another one, so no growth can extend in place, and each one copies itself
to a new block roughly log N times on the way up.

```loft
for tile in tiles { reserve(tile.points, expected_count(tile)); }
for feature in stream { tiles[feature.tile].points += [feature.point]; }
```

`reserve` only ever changes how much room a vector has. It cannot change its
length, its contents, or anything holding a reference to it, and asking for less
room than it already has does nothing.

The saving shows up on disk too, because a persisted store carries each vector's
claimed capacity rather than its length: on a 125-record × 2312-coordinate store,
reserving took the file from 3,816,152 to 2,609,736 bytes for byte-identical
data.

### `directory("sub")` now appends the subpath it always advertised

`directory`, `user_directory` and `program_directory` each take an optional
subpath, and each quietly ignored it:

```
directory("sub")            // was: /home/you/project      now: /home/you/project/sub
program_directory("assets") // was: /usr/local/bin/loft    now: /usr/local/bin/assets
```

The argument doubles as the buffer the answer comes back in, and it was cleared
before being read. Nothing reported this — you got a valid directory, just not
the one you asked for.

`program_directory()` also returns what its name says now: the directory
**containing** the executable, not the executable's own path. Appending to the
old value produced `/usr/local/bin/loft/assets`, a path that can never exist,
and the browser build already answered with a directory — so the two targets
disagreed. If you were compensating for this by trimming the binary name
yourself, drop that step.

### A file outside your project is now equally out of reach for writing

`file("../notes.txt")` reported the file absent, as paths outside the project
always have — but `f.write(…)`, `f += …` and `write_bytes(…)` went ahead and
created it. The documented way to append,

```
f#next = f.size;   // seek to the end
f += "more";
```

then read `f.size` as 0 and overwrote the file from the start, silently. Reads
and writes now agree: a path outside the project cannot be written either, and
the attempt is visible — `f.write(s).ok()` is `false`, `write_bytes` returns
`false`. Move the file inside your project, or use an absolute path (an
absolute path is not restricted; build one with `directory()` or
`user_directory()`).

### A browser build no longer refuses calls it cannot serve

Naming a builtin the browser has no handler for — `gl_screenshot`, say — failed
the `--html` build outright, so one source had to become two entry points
differing only in which calls they were allowed to mention. It builds now: the
call returns its usual failure value (`false`) and reports itself once in the
browser console, which is what a caller checking the result already handles. The
build still tells you which calls those are.

### Declaration order no longer changes what a closure sees

Loft lets you use a struct before you declare it, and that is meant to be invisible.
Inside a lambda it was not:

```
fn take(w: World) -> float {
  chunk = w.chunks[1];                                   // World declared further down
  f = fn(x: float) -> float { chunk.cells[2].v * x };
  f(2.0)
}
```

That failed with `Unknown field text.cells` — a complaint about `text`, in a program with
no `text` anywhere. Moving the `struct World` above the function fixed it, which is a
confusing thing to have to discover.

The capture now resolves exactly as it does when the declaration comes first, whatever it
was projected out of — a field, a vector element, a whole vector, or a scalar read out of
one. Declaration order is invisible again.

### The last corner of mutable text captures

Making mutated parameter captures work (below) left exactly one combination refused with
a message: mutating a captured `text` **parameter** inside a function that itself returns
`text`. That works now too, so the rule for closures is finally uniform — a mutated
capture behaves the same whether it started as a local or a parameter, and whatever the
enclosing function returns.

What was behind it: a text value the function *returns* is already handled specially (the
caller supplies the buffer), and the compiler had been keying off "does this function
return text?" rather than "is this the value being returned?". The first question is the
wrong one — a function can return one text while a closure mutates another, and only the
second question tells them apart.

### A closure can now count with one of your parameters

The accumulator closure is an old friend:

```
total = 0;
add = fn(n: integer) { total = total + n; };
```

It worked over a local, and crashed the moment `total` was a **parameter** of the
enclosing function — with a garbage function id, a segfault, or, on the compiled
backend, generated code that would not build. Every scalar type was affected, and you
did not even have to call the closure: creating it was enough.

Now a mutated parameter behaves like a mutated local, which is what it looks like. The
closure's writes are visible for the rest of the function, and your **caller's value is
untouched** — a scalar parameter is still passed by value. Mutating a `const` parameter
through a closure is refused, as it is anywhere else.

### A lambda that captures your value no longer eats it

Handing a lambda a value from the surrounding scope is the ordinary way to give a
library a function it can call:

```
sampler = fn(x: float, z: float) -> float { terrain_y(x, z, world) };
rest = ground_axle(sampler, cx, cz, yaw, half, radius);
```

If `world` was a **parameter** of the enclosing function, that closure destroyed the
caller's `world`. Nothing said so at the time — the value stayed readable for a
while — and the failure landed later, in some unrelated function that happened to
touch the same value next, with a crash pointing hundreds of lines away from the
lambda. Capturing a value that only *views* into another (`chunk = world.chunks[1]`,
or a `for` loop's element) did the same thing.

The rule is now the one you would expect, and it needs nothing from you: capturing a
value the function **owns** hands it to the closure, which is what lets a factory
return a closure over its own local; capturing a **parameter** or a view **borrows**,
because the real owner outlives the closure either way. So passing a value into a
function that captures it in a lambda cannot damage your copy, and a returned closure
never reads something already freed.

Nothing you can write got stricter — code that avoided closures over store-backed
values by hand keeps working, and can now stop avoiding them.

### Changing a value you matched on now sticks

Destructuring in a `match` gives you a **view** of what you matched, so writing
through it changes the value:

```
match e {
    Holder { items } => { items += "y"; }     // `e`'s payload really is longer now
    _ => { }
}
```

That is what it always looked like it did, and for a vector payload it is what it did.
For a `text` payload the write was silently thrown away — same syntax, same shape, no
warning, and nothing in the code to tell you which one you had written. It now behaves
the same whatever the payload type, including through nested patterns like
`Wrap { inner: Holder { items } }`, where the write used to vanish for every type.

Two crashes in the same corner are gone with it. A function that returns an enum,
calls itself, and edits the payload before returning it used to segfault the
interpreter while the compiled backend quietly got the right answer — a
particularly unpleasant pair, because the browser runs the interpreter. And three
enums in one file that happen to share a variant name (`Nil`, say) used to abort
the compiler with an internal error, on perfectly ordinary code.

If you had worked around any of these by rebuilding the value instead of editing it
in place, that code keeps working — nothing you can write got slower or stricter.

### A big function no longer breaks its own loops

A function whose body grew past about 32 KB started behaving impossibly: a
`while true` would run its body **once** and then simply carry on past the loop,
and the program would exit reporting success. No error, no warning, no hang. Every
`println` inside the body still ran exactly once, so a log ended mid-story with
nothing wrong in it — there was nothing to follow.

The cause was that the interpreter recorded "how far to jump" in a 16-bit number.
Past 32 KB that number no longer reached, and the jump landed somewhere arbitrary.
It affected **every** kind of jump — `while`, `for`, `break`, and skipping over an
`if` or `else` body — because they all recorded the distance the same way. The
compiled backend (`--native`) was never affected.

Jump distances are now 32-bit, which covers any program loft can hold, so this
cannot come back at a larger size. Nothing to change in your code: functions that
were too big simply work now. (Reported from moros, whose editor server sat right
on the edge — adding two `println` lines made it exit before any client could
connect.)

### A `vector<vector<u8>>` finally reads back what you put in

Nesting a **narrow** number inside a vector — `vector<vector<u8>>`, `<u16>`, `<i16>`,
`<i32>`, `<u32>` — used to be misread the moment you did anything with the whole
collection. Printing one packed two 2-byte values into a single 8-byte slot
(`[[1001,2002],…]` came back as `[[131204073,0],…]`); with a 1-byte inner it crashed
outright. Slicing gave the right number of rows with the contents emptied. Reading one
element at a time was always correct, which is what made this so easy to miss: a length
check passes, a spot-check of `v[0][0]` passes, and only a full comparison shows the
damage. If your byte vectors carry real data — a file, a hash, a ciphertext — that is a
silent corruption, and it is what the consumer who reported this was carrying.

The cause was that loft's type table could not actually *say* "vector of vector of
`u16`": a nested element collapsed to its inner scalar, so a declared
`vector<vector<u16>>` registered as `vector<vector<integer>>` and nothing downstream
could know the real width. Seven different places had each worked out the element's
storage width for themselves, and they agreed only when that element happened to be 8
bytes wide — exactly the boundary where the bug started. They now all read one answer,
so a reader can no longer stride differently from the writer. Nesting depth 3+, every
construction path (literal, typed local, `+=`, function return), slicing, printing,
concatenating and binding into a struct field are all covered by the new guard.

Two related fixes came with it. `((v[i] ?? 0) & 255) as u8` — pulling a byte out of a
`vector<u8>`, masking it, and casting — now compiles on `--native`; it used to fail with
a raw `rustc` type error unless you bound the value to a local first, and that workaround
can go. And reading an **unsigned** narrow integer from a binary file on `--native` now
zero-extends: a `u16` of `0xBEEF` read back as `-16657` in some contexts.

One deliberate change comes with this: `size(v)` on a nested vector now reports each row
as its 4-byte reference rather than the inner scalar's width — so
`size(vector<vector<integer>>)` with two rows is 8, not 16. That is what `size`'s
contract already promised ("a heap element counts as its record pointer, never the
target's content"); the number had been following the stride bug. `len` is unaffected,
and no on-disk layout changes.

### `u32` finally holds every `u32`

Three fixes to narrow-width integers, all found by the crawler consumer building a
collision grid, all affecting the interpreter and `--native` alike.

**`x as u32?` used to return null for every value.** The checked-cast range guard was
built at 32-bit width, and `u32`'s maximum wraps there — so the test became "is the value
at most -2?", which nothing satisfies. This was silent rather than loud: the cast handed
back null, the idiomatic `?? 0` supplied a plausible-looking zero, and a grid of zeroes
reads as *not filled in yet* rather than *corrupt*. A constant like `70000 as u32?` worked,
which is why it only ever showed up through a helper function.

**`v[i] = x` now works on a `vector<u16>` and a `vector<i16>`.** It used to fail to
compile outright ("Cannot assign to attribute on type 'OpGetShortRaw'"), even though the
same write to a `u16` *struct field* was fine.

**A `u32` above 2 147 483 647 now round-trips.** Storage for 4-byte integers was
signed-only, so `4000000000` read back as `-294967296` — in vectors and in struct fields.
Worse, `2147483648` collided with the internal null marker and read back as `null`. Both
are fixed: `u32` now covers its full documented range, and the reserved code sits at the
top (`4294967295`), where no legal `u32` value can reach it. The stored bytes are a plain
native `u32`, so binary formats see exactly what they expect. `i32` is untouched.

If you worked around any of this by using `i32` where you wanted `u32`, or `integer` where
you wanted `u16`, you can now use the narrow type — and get the smaller footprint with it.

## 2026-07

The **stability and type-safety** release. Two things anchor it: parsing text into a
number is now *honestly fallible* (it hands you a nullable instead of quietly
inventing a `0`), and a long-standing class of heap / memory bugs — leaks and
use-after-free corruption around returns, reassignment, and `match` — has been
retired wholesale and is now guarded on every night's CI. The registry, the sandbox,
and reference binding all move forward too.

### Patch `2026.7.2` — the `len`/`size` text flip (breaking, pre-1)

**Breaking (contract 0 — before the 1.0 promise):** `len(text)` now returns the
**character** count and `size(text)` the **byte** count — swapped from earlier
`2026.7.x`, where `len(text)` was bytes. This aligns `len` with every mainstream
language (a text's length is how many characters it holds) and gives byte-level work
an explicit, honest home in `size`. A program that used `len(text)` to mean a **byte**
length must move those sites to `size(text)`; a default-on lint flags the common
`for i in 0..len(s) { s[i] }` shape, where a character count was driving a byte index.

Because this changes observable behaviour, published libraries that depend on the new
meaning now declare `loft = ">=2026.7.2"`, so an **older** loft cleanly refuses to load
a too-new library at load time rather than silently misreading multi-byte text.

This point release also carries the accumulated stability and type-safety work landed
since `2026.7.1`: the compatibility-contract groundwork (@PLN102), PEG `match` patterns
(@PLN35), the state-of-the-distribution catalogue (@PLN112), layout-aware zero-copy FFI
(@PLN105), and a wide store-lifetime / leak / use-after-free sweep now guarded on every
night's CI. Full technical history:
[doc/claude/CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md).

### Patch `2026.7.1` — the downloadable binaries

`2026.7.0` shipped without its pre-built binaries: a release-pipeline bug published
the release before the platform builds could attach, and the published release was
immutable. `2026.7.1` is that same release with the four platform bundles actually
attached — Linux (`x86_64-musl`), macOS (Intel + Apple Silicon), and Windows — plus
the pipeline fix (binaries are now built into a draft *before* publish) so it cannot
recur. The loft binary itself is unchanged from `2026.7.0`.

### Parsing a number can fail — `text as integer` now gives you a nullable

This is the one change most existing code will need to look at.

- `"42" as integer` used to *always* hand back an integer, silently producing `0`
  when the text wasn't actually a number — a wrong answer that looked like a real
  one. Casting text to a number is now an honest **fallible parse**: `text as
  integer`, `text as float`, and `text as single` return a **nullable** (`integer?`,
  `float?`, `single?`), and yield `null` when the text isn't a valid number.
- `"42" as integer` is `42`; `"oops" as integer` is `null`.
- Handle the `null` at the cast site, most often with `??`:
  `count = field as integer ?? 0`, or keep the `integer?` and test it
  (`n = field as integer; if n { … }`).

> **Upgrading:** anywhere you wrote `x = some_text as integer` (or `as float` /
> `as single`) and then used `x` as a plain number, add a default —
> `x = some_text as integer ?? 0` — or give `x` a nullable type. The parse silently
> returning `0` on bad input is exactly the bug this closes, so the compiler now
> makes you say what should happen. `null as integer?` is how you write an explicit
> typed null.

### Float math that can't answer now hands back a nullable

The same honesty reaches floating-point arithmetic. A handful of operations have no
real answer for some inputs — `/` and `%` when the divisor is `0`, and `sqrt`, `ln`,
`log2`, `log10`, `asin`, `acos`, and `pow` outside their domain. Those now produce a
**nullable** (`float?` / `single?`) instead of a silent `NaN`, and the nullability
**propagates**: a value computed from a `float?` stays `float?` until you settle it.

- `sqrt(x)` and `a / b` are `float?`. Storing one into a spot that must hold a real
  number — a `-> float` return, a `not null` field — is where you discharge it:
  `sqrt(dx*dx + dy*dy) ?? 0.0`, or keep the `float?` and test it.
- A **literal operand known to be in range keeps the result non-null** — `x / 2.0`,
  `pow(x, 2.0)`, and `sqrt(2.0)` stay plain `float`, so most everyday arithmetic is
  untouched. `+`, `-`, and `*` on non-null floats are always `float`.
- This is a **warning, not an error**. Existing programs keep compiling and keep
  their old runtime behaviour; the compiler just marks each place a possibly-null
  float lands somewhere that expects a definite number.

> **Upgrading:** if a `-> float` function ends in a variable division or a
> `sqrt`/`pow`/`ln`/… of a variable, add a default at the boundary — `… ?? 0.0` — or
> widen the type to `float?`.

### A written compatibility contract — and a command to check it

loft now states its **compatibility contract** in writing
([COMPATIBILITY.md](doc/claude/COMPATIBILITY.md)): at contract level 1, a program
that runs today keeps running — the language, its error behaviour, and its libraries
all included. Both changes above fit inside it, which is exactly why the
nullable-parse and float-null shifts surface as **warnings** rather than breakage.

A new `loft api-surface` command makes the contract checkable. `loft api-surface
<file>` prints the public surface of a program or library — its `pub` functions,
types, sizes, and signatures — and `loft api-surface --diff <base> <new>` compares
two versions and reports a plain verdict (drop-in, or a break), exiting non-zero on a
break so a CI check can guard it.

### Error messages now point at the problem

Every error — parser, type, or runtime — now shows the file, line, and column,
the offending source line, and a caret under the exact token:

```
error: expected integer, got text on argument 2 of call to add
  --> game.loft:3:14
  |
3 |   x = add(1, "two");
  |              ^
```

- **Did-you-mean suggestions.** A misspelled variable, function, field, method,
  type, or enum variant suggests the near match — `unknown variant Color::Bleu —
  did you mean 'Blue'?`.
- **Concrete type mismatches** name both sides, the operation, and (for calls)
  the argument index — no more bare "type mismatch".
- **A mistyped `match` pattern** that can never match its subject (a text arm on
  an integer subject) is now a clear error instead of a silently-dead arm.
- Prefer the old single-line format? Set `LOFT_ERRORS=compact` (or pass
  `--errors=compact`).

Runtime faults (divide-by-zero, index out of bounds, null dereference,
narrowing-cast overflow) keep loft's rule of never aborting a running program:
they yield loft's usual sentinel (`null`, `0`, …) so a game or server keeps
running, and the fault is recorded with its source position instead of vanishing
silently.

### A whole class of memory bugs is gone

loft stores structs, vectors, and keyed collections on a managed heap. That heap had
a long tail of hard-to-see faults — a store leaked once per loop iteration, or a
value was freed while something still pointed at it (use-after-free) — clustered
around returning a value, reassigning one, and binding records out of `match` arms.
This release **retires that class**: whole-value binds copy and projections stay
views under one consistent rule, and the lifetime checker that decides when a store
is freed now reads a single source of truth instead of re-deriving it per site.

It stays fixed because three independent guards run continuously: a
poison-allocator test suite (every freed store is scribbled over, so any later read
is caught), an `Arbitrary`-driven program fuzzer, and a **nightly differential
oracle** that runs a growing corpus through *both* the interpreter and the `--native`
compiler and fails CI on any divergence in output, exit, or leak.

### Dense vectors and predictable copies

Vectors now default to dense storage, and the copy-versus-view model is spelled out
and enforced: a whole-value bind (`b = a`) **copies**, while a projection (`a.field`,
`v[i]` of a struct) stays a **view** onto the original. Narrowing casts are checked
rather than silently truncating.

### `&` — references you can write back through

A `&`-annotated binding creates a reference: pass `f(&x)` and the function can write
back to your `x`, or bind `a = &b` to alias a value for in-place update. It works on
scalars, heap values, and parameters, on both backends.

**Now including whole vectors.** A plain `d = v` (or `d = self.data`) *copies* the
vector — bind it, and the copy is independent. Write `d = &v` (or `d = &self.data`) and
you get a live, writable window instead: `d[i] = x` and `d += […]` write **through** to
the source. This is the ergonomic primitive for vector-heavy code — grab a sub-vector,
mutate it in place, no copy:

```loft
fn bump(self: Grid) {
  row = &self.cells;                 // a writable view of the field, not a copy
  for i in 0..len(row) { row[i] = row[i] + 1; }
}
```

The rule is one sentence now: **a plain bind copies; a `&` bind aliases** — the same for
scalars and vectors. (Reading a struct-typed field or element is still a view without
`&`, since it names an interior place, not a whole value.)

### Running untrusted code — the sandbox subset

A new compile-time **sandbox** lets you run untrusted loft with capability limits —
what a restricted caller may call, which parameters and fields it may touch, and what
it may mutate — enforced as a compile-time admission check, with an adversarial
escape suite proving the boundary holds.

### Games and the browser

`--html` gains **engine-less web modules** (a plain loft program compiles to a
self-contained WASM page), a `host_input()` primitive for feeding browser input in,
and asyncify resume that keeps running in headless / hidden tabs. A WebSocket WASM
bridge brings networked (and zero-trust crypto) programs to the browser.

### A pile of fixes

Among them: a keyed range or partial-key slice used as a value (`x = idx[lo..hi]`)
is now a clear compile error instead of a crash; `.map` on a literal receiver, a
nested-vector element-stride mismatch, and a native miscompile that returned an empty
vector from a struct field are all fixed. The four utility libraries touched by the
parse change — `arguments`, `random`, `regex`, and `cbor` — are migrated to the new
nullable-parse contract and republished.

## 2026-06

**New versioning.** Starting here, loft moves to a **monthly,
calendar-versioned cadence**: releases are named for their month — this one
is **`2026-06`** — which `Cargo.toml` spells `2026.6.0` (year.month.patch;
the patch digit is reserved for in-month security fixes). A deliberate step
up from the old `0.8.x` line.

This release rounds out loft's **library system** — toolchain-free native
libraries, signature-verified installs, and (with the namespace change below)
per-library namespaces instead of one shared flat space.

### Use a native library without a Rust toolchain

Native libraries (like `graphics` or `imaging`) used to compile their Rust
`cdylib` from source the first time you `use` them — needing `rustc`, `cargo`,
and the right system dev headers. loft can now fetch a **prebuilt** cdylib for
your platform and load it directly: no toolchain, no ~90-second first-use
compile. Building from source stays the automatic fallback when no prebuilt is
published for your platform.

### Registry installs are now signature-verified

`loft install` verifies the registry's index against a **trust root** embedded
in the loft binary before trusting any of it — every install is
cryptographically signed end to end, and a tampered index is refused. The trust
root is three independent keys, so a lost signing device can be retired without
disrupting anyone. Maintainers sign with a review-then-sign tool that shows
exactly what's going into a signature — re-downloading each library tarball to
confirm its checksum — before the key is ever used.

### Names that don't fight each other — enum-scoped variants, shadowing, import aliases

Naming got a lot less cramped (`@PLN22`):

- **Two enums can share a variant name.**  `enum Color { Red, Green }` and
  `enum Light { Red, Amber }` now coexist — a bare `Red` resolves from its
  context (the match subject, the declared type, a comparison, a function
  argument), and you can always qualify it as `Color.Red` when there's no
  context.  Defining a *new untyped variable* straight from a bare variant
  (`x = Red`) is a deliberate error — qualify it or give `x` a type — so adding a
  second enum with that variant can never silently re-point existing code.
- **Your names can shadow the standard library.**  `enum E`, `struct File`,
  `pub PI = 3` are all legal even though the stdlib already has `E` / `File` /
  `PI`; your definition wins bare lookup, and `std::E` still reaches the original.
  (The built-in *type* keywords — `integer`, `vector`, `iterator`, … — stay
  reserved.)
- **Import aliases.**  Rename a whole library or individual names on import:
  `use lib as m;` (qualifier `m::fn`), `use lib::Name as Alias;`, and grouped
  `use lib::(a as x, b, c);`.  Multiple names from one library must be
  parenthesised — `use lib::a, b;` is no longer accepted.

### Small integer types hold their full range — and won't silently null

The fixed-width integer types `u8`, `i8`, `u16`, `i16` pin a field to one or two
bytes; this release makes their ranges predictable and their edges safe.

- **`not null` gives the full native range.**  A `not null u16` now holds the
  whole `0..=65535` (before, `65535` read back as `null`); a `not null i8` holds
  `-128..=127` — exactly what the name promises.
- **A nullable field keeps one value aside for `null`**: `u8` is `0..=254` and
  `u16` `0..=65534` (the top trimmed), `i8` is `-127..=127` and `i16`
  `-32767..=32767` (kept symmetric).  Storing that one reserved value used to turn
  into `null` silently — now it's caught.  A literal is a compile error that tells
  you the fix (*"255 is reserved as the null sentinel of a nullable u8 (usable
  0..=254); declare the field `not null` for the full range, or cast with `as
  u8`"*); a value computed at run time gets a rate-limited warning that points you
  at the field while developing and stays quiet in a shipped game.
- **Narrow-element vectors match the fields.**  `vector<u16>` now holds `65535`
  and `vector<i16>` holds `32767`, just as `vector<u8>` already held `255`.

> Upgrading: if existing code stored the reserved edge value into a *nullable*
> narrow field (e.g. `255` into a `u8`), it will now flag instead of silently
> becoming `null` — declare the field `not null` for the full range, or cast.

### Windowed games without a server — `engine_host::run_local`

The games kernel gains a third way to run, next to the server (`run`) and the
network client (`run_client`): `run_local(tick_interval_us, on_event, on_tick)`
drives a **local windowed game** — steady ticks (one tick = one frame), the
kernel resting the CPU when nothing happens, and live build swaps — with no
server and no socket.  Close the window, call `client_stop()`, and the loop
returns.  When your game goes online later, you swap that one line for
`run_client` and keep your handlers exactly as they are.

### Window input as game events — `engine_host::post`

Post a local event from anywhere in your game — `engine_host::post("K:left")`
— and it arrives in your `on_event` handler like any network message
(`ev.cid == -1` tells you it came from this machine).  Key presses stop
slipping between frames, and your handlers no longer care whether input is
local or remote.  Servers with a window got their exit too: call
`engine_host::stop()` and `run` returns when the window closes.

### The debugger now tells you when a breakpoint can't work

Setting a breakpoint over `loft debug --rpc` answers with `verified` per
breakpoint: `false` means that line can never fire (no code on it, or a file
your program doesn't use) — so you find out immediately instead of waiting on
a stop that never comes.  Tracepoints also got friendlier: `"log": "expr"`
now works as a single expression (before, only the `["expr"]` array form did).

### An interactive prompt — `loft repl`

Run `loft` with no file (or `loft repl`) to get an interactive prompt where you
type loft one line at a time and see the result immediately:

```
loft> x = 40 + 2
loft> x
42
loft> fn dbl(n: integer) -> integer { n + n }
loft> dbl(x)
84
```

Names you bind stay available, functions and structs you define persist for the
session, multi-line input is supported, and a typo or run-time error doesn't end
the session.  Built-in commands inspect what you've defined — `:fns`, `:vars`
(each variable with its current value), `:bytecode`, `:rust`, `:slots` — and
`:help` lists them.

The prompt has **arrow-key history, in-line editing, and Tab completion** (of
function names, types, your variables, and `:`-commands), and it **remembers
your session**: the next time you start it, the variables and definitions from
last time are already there.  Start clean with `loft repl --fresh`.  See
[doc/claude/REPL.md](doc/claude/REPL.md).

### Look inside a program — `loft introspect`

`loft introspect <file>` prints a program's bytecode, the Rust loft generates
for it, per-function variable slot tables, and inferred types — side by side, in
one command.  Sub-flags pick one view (`--show-bytecode`, `--show-rust`, …) or a
single function (`--fn`).  This replaces hunting through `LOFT_LOG=…` dumps for
everyday inspection.

---

## 0.8.5 — 2026-06-07 — Language Maturity

This release is about the language itself getting solid.  Closures finally
work the way you'd expect, bounded generics carry types correctly through
methods and tuples, the native backend ships as production, and a
browser-based **branch review viewer** lets you read your in-flight code
from any device with `make view` + an SSH port-forward.

### Closures that capture what they should

The biggest user-visible fix.  Closures now hold a **live reference** to
captured variables, not a snapshot — they see the latest value, mutations
through one closure are visible to another, and a closure that captures a
struct field reads the field as it currently is.

```loft
counter = make_counter()  // returns a closure pair (inc, get)
counter.inc(); counter.inc(); counter.inc()
println(counter.get())    // 3 — was 1 before this release (snapshot bug)
```

- Closures returned from functions keep their captured environment alive.
- Multiple closures sharing the same captured cell see each other's
  writes.
- Closure-captured vector / struct / nested-struct fields read live, not
  stale.
- Validation matrix in `tests/closure_matrix.rs` cross-checks 30+ shapes
  on interpreter + `--native`.

### Bounded generics + interfaces

Write generic functions with type constraints; the compiler picks the
right per-type implementation at the call site.

```loft
fn show_pair<T: Printable>(a: T, b: T) -> text {
    "{a.to_text()} & {b.to_text()}"
}
println(show_pair(3, 7))           // 3 & 7   (built-in to_text)
println(show_pair("hi", "ho"))     // hi & ho (text passes through)
```

- `<T: Bound>` constraints — `Ordered`, `Equatable`, `Addable`, `Numeric`,
  `Scalable`, `Printable`, plus user-defined interfaces.
- Bound-typed values round-trip through tuples, vectors, struct fields,
  and `for` loops — the compiler now substitutes T's concrete type
  everywhere it appears.
- Generic functions returning `(T, T)` work with text, references, and
  user types — not just primitives.
- Format-string interpolation `"{x}"` where `x: T` routes through the
  bound's `to_text` method automatically.

### Tuples cross-validated end-to-end

Tuples now ship as a fully validated value type.  40 cross-mode test cells
cover 5 element types (scalars, text, nested tuples, closures, struct
references) across 3 storage destinations (local, direct stack, struct
field) — interpreter and `--native` produce byte-identical output.

```loft
fn split_message(s: text) -> (text, text) {
    n = s.len() / 2
    (s[0..n], s[n..s.len()])
}
left, right = split_message("hello world")
```

### Branch review viewer (`make view`)

`make view` launches a browser-accessible doc + code review surface for
the current loft branch.  Dashboard shows files changed vs `main`,
recent commits, uncommitted state — all with status badges.  Click any
file for a rendered view (`.md` rendered via the new `lib/markdown`
library, others as line-numbered code), toggle between
`Rendered ¦ Diff vs main`, click any commit for the per-file diff,
click any tracker tag (`@P-id` / `@PLAN-id`) for cross-doc references.
SSH-port-forward 8765 from the host.  Built entirely in loft (web
server, markdown rendering, JSON parsing, file walking) + a small bash
wrapper for `git` calls; no Python, no external markdown library, no
syntax-highlighter dependency.  See
[doc/claude/DEBUG.md § Branch review viewer](doc/claude/DEBUG.md#branch-review-viewer-make-view).

A `/welcome` landing page surfaces project status at a glance: open
problems, recently closed bugs (last 30 days), active and recently
finished plans, future plans by category — all built from a live tracker
index that updates on every commit.

### Tracker index (`make index`)

A small file-based index of every `@P<id>` / `@PLAN<id>(-segment)*`
reference across the project, queryable from the command line.  The
viewer surfaces the same data; CI uses it to catch broken tracker
references at commit time.

```bash
make index                            # rebuild index/tags.json
./scripts/idx tag:@P259               # all references to a P-issue
./scripts/idx prefix:@PLAN37          # all PLAN37-* phase refs
./scripts/idx incoming:doc/claude/PROBLEMS.md   # backlinks to a doc
./scripts/idx broken                  # broken @-references
./scripts/idx broken-links            # broken markdown links
```

A loft-native scanner port (`make index-loft`) reproduces the bash
scanner's output via the loft language itself — exercises long-running
file-walking + JSON emission shapes that no other loft program touches.

### Native compilation goes production

The `--native` backend (loft → Rust → rustc → standalone binary) is now
the default.  108 / 108 native tests pass; closures, generics, tuples,
JSON, and the viewer all compile + run identically under `--native` and
`--interpret`.  Use `--interpret` only when bisecting a native-only
regression.

Eight previously-tracked native codegen bugs closed (use-after-free in
heap-typed tail returns, text-concat type-dispatch, generic vector
struct returns, closure-tuple-field layout, parallel-queue native
runtime, and four more).

### `lib/markdown` — markdown renderer in loft

A standalone library: headings, bold, italic, inline code, fenced code,
links with anchor support, tables (with alignment), lists (ordered +
unordered + nested), images, autolinks for tracker tags, autolink
prefix configuration, image-URL rewriting.  Pure loft — no external
parser.  Used by the branch-review viewer and any future loft
documentation tool.

```loft
use markdown
html = markdown::render(source, "/tag/", "/img/", "")
```

### Smaller language wins

- **`@P274`** — `text + integer` concat now correctly converts the
  integer (was emitting `OpAppendText` with a raw i64; SIGSEGV in
  interp / E0614 in native).
- **`@P275`** — module-scope `const vector<T>` works under the
  default `--native` path (was only initialised under
  `--native-release`; default emit panicked at
  `stores.const_refs[NNN]`).  Side-fix: nested `OpConstRef` calls
  no longer accumulate `stor` prefixes in their substituted form
  (a substring-of-its-own-output bug in the codegen template
  rewriter).
- **`@P276`** — `(s[i] ?? '<c>') == '<c>'` now type-checks under
  `--native` (was rustc E0308: the pre-evaluated block holding
  the character lifted as `i32`, then the outer
  `OpConvIntFromCharacter` template compared it against `char`).
  Bind-then-compare (`c = s[i] ?? '*'; if c == 'b'`), else-if
  chains, and ordering compares (`<`/`>`) all work too.
- **`@P283`** — format-string interpolation of a self-slice-
  reassigned text PARAMETER no longer crashes either backend.
  Pattern: `fn f(rb: text, id: text) -> text { …; rb = rb[a..b];
  "[{id}] {rb}" }` was SIGSEGVing the interpreter and rejecting
  with rustc E0368 in native.  The work-buffer parameter
  promoted by `text_return` is `RefVar(Text)` (`&mut String`),
  but the codegen for `OpAppendText` / `OpClearText` /
  `OpFormatText` / `OpFormat{Int,Float,Single,Database}` /
  `OpAppendCharacter` on these targets emitted the local-String
  variants — interp treated the refvar slot as a `String` →
  SIGSEGV; native emitted `var += &*(…)` on `&mut String` →
  E0368.  Fix dispatches to the matching `Stack` variant for
  RefVar(Text) targets on both backends (mirrors the existing
  B7 OpAppendCharacter dispatch).
- **`@P259`-`@P261`** — closure / store-allocation / vector-field
  fixes (the closure-cell trio).
- **UTF-8** — `json_parse` now decodes 2/3/4-byte UTF-8 codepoints
  correctly (was widening byte-by-byte; `→` became `âââ`).
- **WebSocket binary frames** — `lib/server` exercises the binary
  path in production; multi-client games use it.
- Eight new P-issues filed from dogfood discovery (native codegen +
  parser quirks surfaced by writing real loft consumers); fixes
  scheduled across the next few releases.

### Workflow + project-management

- New `## Open work` sections in reference docs catalog
  enhancement opportunities discovered while building real consumers.
- DEVELOPMENT.md documents the "fix-on-the-spot vs canonical-home"
  workflow for handling discovered language gaps mid-feature work.
- Plan documentation reorganized: `plans/` for core-language work
  (capped at 2-3 active), `lib_plans/` for library work, `ROADMAP.md`
  as the prioritization view.
- Every PROBLEMS.md row now self-tags with `**@P<n>**` so the
  index unambiguously links each row to its references.

### Relative file paths are now program-relative — portable "program + assets" bundles

A relative file path — `file("assets/font.ttf")`, `read_file("data.bin")`,
`delete("out.tmp")` — now resolves against **the program's own directory** (the
source dir under `--interpret`, the executable's dir under `--native`), not the
process working directory.  An asset addressed relative to your program loads no
matter where the program is launched from:

```loft
f = file("assets/level1.dat");   // beside the program, wherever it runs from
```

This is what #255 needed: a bundled font worked from the source tree but vanished
under `--native` (which runs from a temp dir), because the path resolved against
the cwd.  **Absolute paths are never rewritten.**  Resolution is uniform across
`file()`, `exists()`, `read_file`/`write_file`, the `File` methods,
`delete`/`move`/`mkdir`, and image loads.

**CLI tools opt back into cwd** with a one-line file-top directive — a
*user-supplied* relative path then resolves against the working directory:

```loft
#cwd
fn main(args: vector<text>) { data = read_file(args[1]); }
```

Per-invocation, `LOFT_PATHS=program` / `LOFT_PATHS=cwd` overrides both.
`source_dir()` returns the anchor and now works under `--native` (was empty
before).

**Breaking change** — a program that read or wrote a relative path expecting the
*working directory* now needs `#cwd` at the top.  The in-tree corpus that did so
(13 file-I/O tests) was migrated in this release.

### Faster startup, automatically — the program cache is on by default

Running the same program again is now **~3× faster to start**: the first
run caches the fully-parsed program (the standard library, every library
you `use`, and your script) next to your other caches, and later runs of
the unchanged program skip parsing entirely.  It just works — no flag to
set.  If anything the program reads changes, the cache notices and
re-parses, so you never get a stale result.

- **Turn it off** with `LOFT_NO_CACHE=1` (e.g. for one-shot batch jobs
  where the first-run save isn't worth it).
- **Cap its size** with `LOFT_CACHE_MAX_MB` (default 512 MiB); the oldest
  bundles are evicted past the limit.
- It automatically stays **off inside `cargo run` / `cargo test`**, so
  building the compiler never serves a stale parse.

### File `+=` is now append-only — and `file.sync()` lets you flush

`f += value` now **appends** to the end of the file, matching how
`vector += [elem]` and `text += "more"` work on the other collection
types.  Earlier writes are preserved when you re-open the file:

```loft
{f = file("log.txt"); f += "first\n";  f.sync(); }
{f = file("log.txt"); f += "second\n"; f.sync(); }
{f = file("log.txt"); f += "third\n";  f.sync(); }
// Result: 19 bytes — "first\nsecond\nthird\n", not just "third\n".
```

Use `f.sync()` between log records or block boundaries to guarantee
the buffered bytes have landed on disk before the next write is
issued.  Returns `true` on success; on `Directory` / `NotExists` the
call short-circuits to `false`.

**Breaking change** — code that relied on `f += …` truncating the file
on first re-open now needs to call `f.set_file_size(0)` (or
`f#size = 0`) explicitly before the first write.  Updated call sites
in this release: `tools/audience-demo/single_port_server.loft`,
`lib/world/src/world.loft`, `lib/graphics/src/glb.loft`,
`scripts/build-playground-examples.loft`.  Explicit offsets via
`f#next = N` still overwrite at offset `N`, so the snapshot idiom
(fixed-slot headers, overwrite-in-place) keeps working.

### Interpreter no longer corrupts memory on deep recursion

The interpreter's value stack now grows on demand.  Previously it was
a fixed 8 KB buffer that never expanded, so a program that nested
function calls deeply enough (roughly 40+ frames carrying a handful of
locals) would silently write past the buffer and corrupt the heap —
usually surfacing as a confusing "double free or corruption" abort
*after* the program had finished printing its output.  Deeply
recursive interpreted programs now run correctly (the `--native`
backend was never affected, as it uses the real machine stack).

## 0.8.4 — 2026-04-24 — Awesome Brick Buster

This release focuses on **the web**: your loft programs can now fetch
URLs, serve HTTP, parse JSON, and even run entirely inside a browser tab.
The headline is **Brick Buster** — a full arcade game, paddle + ball +
powerups + music + levels + high score, that you can share with a friend
via a single URL.

### JSON — read and write structured data

```loft
v = json_parse("{\"name\":\"Alice\",\"age\":30}")
println(v.field("name").as_text())   // Alice
println(v.to_json_pretty())          // formatted output
```

- `json_parse(text)` turns JSON into a value you can explore.
- Bad input returns a null value instead of crashing.  Ask
  `json_errors()` what went wrong.
- Build JSON from code with `json_number`, `json_string`,
  `json_array`, `json_object`, ...
- Read it back with `field("key")`, `item(index)`, `len()`, `keys()`.
- `MyStruct.parse(json_value)` fills a struct from JSON in one line.

### HTTP — talk to the web

```loft
use web
r = http_get("https://example.com")
if r.ok() { println(r.body) }
```

- `http_get`, `http_post`, `http_put`, `http_delete` — straightforward
  blocking calls that return an `HttpResponse` with `.status`, `.body`,
  and `.ok()`.
- `..._h` variants accept custom headers: `http_get_h(url, ["Accept: application/json"])`.
- A simple HTTP **server** is also available: `for req in listen(8080) { respond(req, ...) }`.

### Lighting that actually lights

The 3D renderer's PBR shader now uses the light colours and intensity
you pass in.  Previously the `Light` struct was accepted by the
scene-graph but the shader ignored `color_r/g/b`, `intensity`, and all
point lights — every scene looked as if lit by a single neutral-white
directional.

- A directional light's `intensity` scales its contribution.
- A scene's first **point light** is now rendered (quadratic
  attenuation; no shadow yet).
- Goldens for five of the graphics examples are checked in as
  regression guards — a shader tweak that breaks lighting is caught by
  a pixel-diff test.

### Games in the browser

- **Brick Buster** — a complete arcade game (paddle, ball, powerups,
  music, levels, high score) that runs in your browser and on the
  desktop.  Try it at
  <https://loft-lang.org/loft/brick-buster.html>.
- **Graphics gallery** — 24 WebGL demos, from hello-triangle to
  physically-based rendering.
- `loft --html program.loft` produces a single folder you can drop on
  any static web host.

### Easier code, clearer errors

- `parallel { }` really runs in parallel now (one OS thread per arm).
- `x ?? return err` — one line instead of a two-line null check.
- `type Handler = fn(Request) -> Response` — name function and tuple
  types.
- Any type with `fn next(self) -> Item?` can be used in `for x in val`.
- When the interpreter hits a fatal error, it now tells you *which
  function and line* triggered it before exiting.

### A gentler language

- `integer` is now 64-bit everywhere.  Big numbers like
  `9_876_543_210` just work — no suffix required.
- The old `long` type and `33l` literal suffix are gone; use `integer`
  and `33`.
- Three crashes involving `match` on complex types are fixed —
  character interpolation, uneven match arms, and chained native calls
  no longer leak memory.

### Native editor & tooling

- **Native Moros editor** — a full OpenGL editor ships as a standalone
  app you can distribute without installing loft.
- `loft --dump file.loft` — show the compiled bytecode without running
  the program.  Handy when something compiles oddly.
- New test runner: `scripts/find_problems.sh --bg` runs the whole suite
  in the background; check in with `--peek` or `--wait`.

---

### Closures you can return

Functions that return a closure now work correctly, including when the
closure captures variables from the enclosing scope:

```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    |name| { "{prefix}, {name}!" }
}
hello = make_greeter("Hi")
println(hello("Ada"))   // Hi, Ada!
```

Capturing closures also work with `map` and `filter`:

```loft
factor = 10
big = map(nums, |x| { x * factor })
```

### Quality-of-life fixes

- **Typos stop compilation.**  `y = unknown_thing` now fails with a
  clear error instead of silently creating a garbage value.
- **`rev(vector)`** — you can now iterate a plain vector in reverse.
- **Format strings** — `"{n:<5}"` (left-align), `"{n:^5}"` (centre) and
  `"{f:.0}"` (zero decimals) all behave the way you'd expect.
- **File reading** — `file.lines()` now returns text after the last
  newline, not just full lines.
- **Sorted collections** — descending primary-key ranges return
  correct results in every mode.
- **Windows paths** — native compilation correctly escapes `\` in file
  paths.

### Faster programs

- The compiler does arithmetic at compile time where it can, so
  `[for i in 0..100 { i * 2 }]` becomes a ready-made vector instead of
  a loop.
- `par(...)` automatically picks a lighter, faster worker when your
  work doesn't need its own scratch memory — no syntax change.

### Better docs

- New pages on **pattern matching** and **format strings**.
- Expanded chapters on images, threading, and generics.
- 137-page PDF reference regenerated.

---

## 0.8.3 — 2026-03-27 — WebAssembly!

Loft now runs in the browser.  The playground at
<https://loft-lang.org/loft/playground.html> compiles and executes
loft programs entirely in your browser tab — no server involved.

Behind the scenes:

- A virtual in-memory filesystem for browser tests.
- Captured `println` output for the playground.
- A stable plugin protocol so native extensions (imaging, random, web)
  can be loaded at runtime.
- String-heavy programs are faster thanks to format-string
  pre-allocation.

---

## 0.8.2 — 2026-03-24

### Lambdas

Write throw-away functions inline:

```loft
doubled = map([1, 2, 3], |x| { x * 2 })
```

The short form `|x| { ... }` infers types from where you use it.  Use
the long form `fn(x: integer) -> integer { ... }` when you want them
explicit.

### Named arguments and defaults

```loft
fn connect(host: text, port: integer = 80, tls: boolean = true) { ... }

connect("localhost")                       // uses both defaults
connect("localhost", tls: false)           // skips port by name
```

### Native compilation

Ship your loft program as a real native binary:

- `loft --native file.loft` — compile and run via `rustc`.
- `loft --native-emit out.rs` — save the generated Rust source.
- `loft --native-wasm out.wasm` — compile to WebAssembly.

### JSON, computed fields, field constraints

- `"{value:j}"` serialises any struct to JSON.
- `Type.parse(json_text)` parses JSON back into a struct.
- `computed(expr)` fields are recalculated on every read, no storage
  needed: `area: float computed(PI * $.r * $.r)`.
- `assert(...)` clauses on struct fields validate every write.

### Small but welcome

- Workers started with `par(...)` can now return `text` and enum
  values, not just numbers.
- `fn` prefix dropped on function references: write `apply(double, 7)`,
  not `apply(fn double, 7)`.
- `pub` is now required to expose a definition to other files — this
  keeps your module boundaries tidy.

### Clearer errors

- Using `string` as a type suggests `text` instead of a generic error.
- Six common mistakes now come with a fix suggestion.
- Several crashes on unusual input have become proper error messages.

### Bug fixes

- `c + d` on two characters now produces text, not a crash.
- Empty vector literal `[]` as an argument no longer crashes.
- `v += other_vec` on text-bearing vectors no longer corrupts data.
- `map`, `filter`, and `reduce` no longer trip over their own internal
  slots.

---

## 0.8.0 — 2026-03-17

### Match expressions

Pattern-match enums, structs, and scalars:

```loft
match shape {
    Circle { r } => PI * pow(r, 2.0),
    Rect { w, h } => w * h,
}
```

- The compiler checks that you cover every case.
- Supports `North | South =>` (or-patterns), `if r > 0.0` (guards),
  `1..=9` (ranges), null patterns, character patterns, and full
  `{ ... }` block bodies.

### Formatter

- `loft --format file.loft` — format in place.
- `loft --format-check file.loft` — fails if not formatted; useful in
  CI.

### Imports

- `use mylib::*` — bring in everything.
- `use mylib::Point, add` — pick out just what you need.
- Local definitions always win over imported ones.

### Higher-order helpers

```loft
doubles = map(numbers, fn double)
evens   = filter(numbers, fn is_even)
total   = reduce(numbers, fn add, 0)
```

### Testing made easier

- `loft --tests file.loft::test_name` — run a single test.
- `loft --tests 'file.loft::{a,b}'` — run a selection.
- `loft --tests --native` — compile tests to a native binary first.

### New standard-library helpers

- `now()` — milliseconds since 1970.
- `ticks()` — microseconds since program start, monotonic.
- `mkdir(path)` / `mkdir_all(path)` — make directories.
- `vector.clear()` — empty a vector.

### Clearer warnings

- Division or modulo by a constant zero.
- Unused loop variables (silence with `for _i in ...`).
- Unreachable code after `return`, `break`, or `continue`.
- Redundant null checks on `not null` fields.

### Bug fixes

- `x << 0` and `x >> 0` now return `x` instead of null.
- `NaN != x` now returns `true` (it was wrongly `false`).
- `??` works correctly with floats.
- Using `if` as an expression without `else` is now a compile error
  rather than silently returning null.
- Assigning `null` to a struct field no longer crashes.
- `sorted[key] = null` and `hash[key] = null` remove the entry, as
  documented.

---

## 0.1.0 — 2026-03-15 — First release

The core language, in one place.

### Types and values

- **Static types with inference** — no type annotations on locals; the
  compiler figures out the type from the first assignment.
- **Null safety** — every value may be null unless declared `not
  null`; null propagates through arithmetic; `?? default` supplies a
  fallback.
- **Primitives** — `boolean`, `integer`, `long`, `float`, `single`,
  `character`, `text`.
- **Structs** — named records: `Point { x: 1.0, y: 2.0 }`.
- **Enums** — both plain enums and struct-enums (variants with fields
  and per-variant methods).

### Control flow

- `if`/`else`, `for`/`in`, `break`, `continue`, `return`.
- For-loop extras — inline filter (`for x in v if x > 0`), loop
  attributes (`x#first`, `x#count`, `x#index`), in-loop removal
  (`v#remove`).

### Working with collections

- `[for x in v { expr }]` — vector comprehensions.
- `vector<T>` (dynamic array), `sorted<T>` (ordered tree),
  `index<T>` (multi-key tree), `hash<T>` (hash table).

### Text and formatting

- `"Hello {name}, score: {score:.2}"` — string interpolation with
  format specifiers.

### Other

- **Parallel execution** — `for a in items par(b=worker(a), 4) { ... }`
  spreads the work across CPU cores.
- **File I/O** — read, write, seek, directory listing, PNG images.
- **Logging** — `log_info`, `log_warn`, `log_error` with source
  location and rate limiting.
- **Libraries** — `use mylib;` imports from `.loft` files.

---

## Version comparison links

- [Unreleased vs 2026-06](https://github.com/loft-lang/loft/compare/v2026.6.0...main)
- [2026-06 (2026.6.0)](https://github.com/loft-lang/loft/compare/v0.8.5...v2026.6.0)
- [0.8.5](https://github.com/loft-lang/loft/compare/v0.8.4...v0.8.5)
- [0.8.4](https://github.com/loft-lang/loft/compare/v0.8.3...v0.8.4)
- [0.8.3](https://github.com/loft-lang/loft/compare/v0.8.2...v0.8.3)
- [0.8.2](https://github.com/loft-lang/loft/compare/v0.8.0...v0.8.2)
- [0.8.0](https://github.com/loft-lang/loft/releases/tag/v0.8.0) — the first tagged release
- 0.1.0 — pre-dates tagging (no release tag exists)
