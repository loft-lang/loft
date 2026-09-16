# Reference review — validating what we promise

The language reference is the promise. It ships inside **all four release bundles**
and on the docs site, and a reader treats it as the definition of the language — so a
sentence in it that is no longer true is not a documentation bug, it is a promise we
are breaking, quietly, to everyone who reads it offline.

**This pass is by hand, and that is not a gap to be closed.** `make release-checklist`
already answers everything a program can about the reference:

| check | what it establishes |
|---|---|
| `A-pdf` | built after every input that decides its content moved |
| `A-pdf-version` | the artifact says it is *this* release, read from its own bytes |
| `A-pdf-content` | every chapter is present, the stdlib chapter is not empty, no placeholder shipped |

All three can be green on a reference that describes behaviour the language stopped
having two releases ago. They check that the document is **whole and current**; not one
of them reads a sentence. What is left is the part that matters most to a user, and it
needs a person.

## Do it early — the watermark

Left to tag day this becomes a day of reading under time pressure, which is how a
review turns into a skim. So it is **continuous**: each chapter records the commit it
was last read through, and it only comes back on the list once its own source has moved
past that. Read a chapter the week its topic changes and the tag-day list is short
because the work already happened.

```bash
make reference-review              # the worklist: never-reviewed, and moved-since
make reference-review ARGS=--verbose   # + the commits behind each moved chapter
```

The aid reports three things and judges none of them: chapters with **no watermark row**
(never read), chapters that have **MOVED** since the commit their row records (with the
commit count, so "three commits, all typo fixes" is dismissed in seconds), and **stale
rows** — a watermark naming something that is no longer a chapter, which would otherwise
read as coverage we do not have.

## What to actually check in a chapter

The examples are **already gated** — every code example in the reference is a
`tests/docs/*.loft` file that runs in the suite, which is what lets page 1 claim it. Do
not re-verify them by hand; that budget belongs to the things no test covers.

⚠ **Read "example" narrowly: it means the code the file EXECUTES.** A chapter is mostly
comments, and three kinds of claim inside them run under nothing at all — which is where
every defect the first passes found was living:

| claim in a comment | what checks it |
|---|---|
| a code snippet shown but not executed (`fn f(v: &vector<T>) …`) | nothing |
| a table of results (`1.0 / 0.0 is inf`) | nothing |
| a quoted diagnostic ("a text parse `as integer` may fail…") | nothing — and an `assert` never can, because the program does not compile |

So the first move on a chapter is to separate the sentences the suite is holding up from
the sentences it is not, and spend the read on the second set.

**Where the proof goes once you have it.** Not into the chapter. A chapter is read by
someone meeting the subject for the first time, so proving a boundary exhaustively on the
page costs them more than it gives: keep the one or two cells that carry the LESSON and
move completeness to a guard in `tests/scripts/`. The two written for the Float and
Functions passes are the model —
`the-reference-float-boundary-is-where-it-says.loft` and
`the-reference-quotes-its-refusals-word-for-word.loft` — the second using `@EXPECT_ERROR`
cells, which one file can hold beside a running cell since loft#1242. Such a guard is a
LOCK rather than a regression test: it has no build on which it fails, so it records
`@falsified-at: none` and must be checked by hand in both directions, because a guard made
of expected failures passes most easily when it is proving nothing.

⚠ **Check a multi-cell `@EXPECT_ERROR` guard ONE CELL AT A TIME, in its own file.** Every
declaration after the first firing one is credited without matching (loft#1261), so the
whole-file run and `make falsify` both report `n/n` on a build that produces one of them —
measured on a guard whose four interesting cells all failed correctly when run alone.
Until that is closed, the by-hand check is: split the cells out, run each against the
build the guard was written for AND against this one, and record the two answers per cell
in `@falsified-at`. `const-binds-through-every-append-route.loft` carries that table.

⚠ **Read the chapter's MODEL before its claims.** Chapter 17 said a library name needs
its `libname::` prefix, and every section after that inherited it: the struct section
called the bare form a parse error, the free-function section called the prefix required,
and the import section built a distinction between `use lib;` and `use lib::*` that does
not exist. Reading claim by claim catches none of that — each sentence agrees with the
others, and the page only comes apart against the compiler. **One measurement did it: a
bare call answering.** So before the sentence-level read, take the chapter's central
rule, write the smallest program that would violate it, and run that.

⚠ **Check the LIBRARY CATALOGUE for the chapter's own subject.** Chapter 22 opened "Loft
provides two time functions" and told the reader to use `now()` for "date calculations" —
while a published `time` package does proleptic-Gregorian arithmetic on exactly the
millisecond integer `now()` returns (`format_iso(now())`, `weekday_name(now())`,
`add_days(now(), 30)`). The sentence is true about the standard library and reads as true
about loft, and the reader it fails is the one who believes it and writes calendar maths by
hand. CLAUDE.md already says to check `make libcatalogue` before building something; a
chapter is where that check is owed to somebody else. **Run it for every chapter whose
subject a package could plausibly cover**, and say plainly which side of the line the
chapter is on.

⚠ **A chapter whose subject is CONFIGURED needs a configured run, and that cannot live on
the page.** Chapter 20 documented a log file format, four config sections, per-file level
overrides, rate limiting and a production mode — and its executable cells called the four
log functions with NO config present, where the documented behaviour is to do nothing. So
the chapter asserted that logging is off, and every claim about what logging DOES ran under
nothing. Three were wrong, including the config spelling that `loft --generate-log-config`
prints into its own template. A `log.conf` cannot be added beside the chapter either — it
would switch logging on for every other chapter in the suite. **The home for a configured
chapter's proof is a Rust test that writes a private directory per case and runs the binary
in it**; `tests/logging_config_is_what_the_chapter_says.rs` is the shape.

⚠ **A chapter that states a rule does not say who KEEPS it, and the reader assumes the
compiler does.** Chapter 19 listed three worker rules in one bulleted list: two the compiler
enforces with precise refusals, and *"must not use global state or I/O (no println, no file
access)"*, which it does not enforce at all. `(C-Impure)` makes that one the author's
contract by design, so the language is right and the page was wrong to shelve it beside the
other two. Reading alone cannot separate them — running the violating program can, and the
cost of not knowing is a worker that appends three lines to a file and leaves one, a
different one per run. **For every "must" in a chapter, write the program that disobeys it
and find out which of the three answers you get: a refusal, a defined behaviour, or
silence.**

⚠ **And run the violating program even when you expect it to confirm the chapter** — the
chapter is not the only thing it can falsify. Chapter 18 taught a single-axis `const`
(the language has had two axes since @PLN40), which is a page defect like chapter 17's.
But the same little programs, run across the shapes the chapter does NOT mention, found
that `p: & const vector<T>` and `p: const hash<R[k]>` both accepted an append that reached
the CALLER — a `const` promise silently not holding, on both backends, which no amount of
reading could have produced. **The reference pass is a language probe that happens to
start from the prose**; the chapter's own subject is the axis nobody has swept.

⚠ **For every "cannot", "is an error" and "does not" in a chapter, write the program that
does it anyway.** Chapter 23 is a catalogue of traps, and its wrong claims CLUSTER: **five
of them are a stated NEGATIVE the language does not have**. Hashes "cannot be iterated"
(`for e in h` has walked one in ascending key order since C60, four and a half months
before the read), slicing inside a multi-byte character "is an error" (it rounds outward to
whole characters), `lines()` "crashes" on invalid UTF-8 (it warns, answers null, and hands
back an empty vector — the dangerous direction, because a reader defending against a crash
writes no defence at all against a silently empty read), a non-`&` vector parameter's
append "is local" (it reaches the caller — the exact claim loft#1251 corrected in LOFT.md
and the Vector chapter, still standing here because nobody had read this page), and
`e#remove` "inside a filtered loop" (it works in a plain one too). That is not a
coincidence about this chapter: **a negative is the one claim an example cannot carry**, so
the sentences a chapter's own cells hold up are exactly the positive ones, and everything a
page says the language will NOT do runs under nothing at all. The reference names the same
feature in more than one chapter, so this is also where a corrected claim goes to survive:
grep the whole reference for a sentence you fix, not just the chapter that owns the subject.

⚠ **A trap catalogue must say what it does not cover.** Chapter 23 opened by claiming it
"catalogues every known trap", which the shipped lints falsify by themselves — nothing on
the page about `omitted-field-zero`, `variant-field-unchecked` or the copy-on-bind lost
write, all of which the compiler now diagnoses. A closed-list promise ages into a wrong one
the first time the language grows a corner, and a reader who believes it stops looking.

⚠ **A CAVEAT outlives the gap it describes, and it costs more than a stale feature
claim.** Chapter 24 told the reader that `Type.parse(text)` "DROPS diagnostics — malformed
input and schema mismatches leave the struct at its defaults with `json_errors()` empty",
and sent them to a two-step spelling for error reporting. That gap (Q1) closed on
2026-08-20; the warning outlived it in THREE documents — the chapter, CAVEATS.md and a
ROADMAP row still asserting both halves — while QUALITY.md recorded the close. A stale
caveat steers a reader away from the correct, shorter spelling permanently, and nothing they
can run contradicts it: a caveat is a claim about what does NOT happen. So when a gap is
closed, grep for the WARNING as well as for the behaviour, and when you review a chapter,
treat each of its caveats as a claim to re-measure rather than as context.

⚠ **When two sentences in one chapter disagree, run BOTH — never pick the plausible one.**
Chapter 24 said schema mismatches "land as the loft null sentinel in the struct" and, forty
lines later, "never by putting a null in a slot the declared type says cannot hold one". It
also carried the caveat above beside a cell comment saying that same form reports. Both
disagreements were the tell that behaviour had moved and one half of the page had been
updated — the later, more specific sentence was right each time, but that is a pattern and
not a rule, and the only way to know is to run them. A chapter that contradicts itself is
the cheapest review lead there is: it has already told you where to look.

⚠ **A chapter's table may be a copy of a DESIGN doc rather than of the code.** Chapter 25
listed six built-in interfaces and what each one lets you write, and three rows were wrong
in the permissive direction — `Addable` "addition and subtraction" (`-` is refused),
`Numeric` "all four scalar operators" (`+` and `/` are refused), `Scalable` "multiplication
by a float factor" (a `scale` METHOD taking an INTEGER, answering `integer`, and satisfied
by no built-in type at all). Every row was a faithful copy of INTERFACES.md's
*"Phase 1 defines these interfaces in `default/01_code.loft`"* block — which is the design
as written, not the file as shipped. So the chapter was not careless; it trusted a sibling
doc that made a claim ABOUT A FILE without being checked against it. **When a chapter's
table names a code artefact, diff it against that artefact, and fix the doc the table came
from too** — otherwise the next chapter copies it again.

⚠ **A cell that only asks "does this compile" passes on a wrong answer.** Writing the
completeness guard for that table, the cell for `a - b` under `Numeric` was built from a
probe that had reported "compiled" — and asserting its VALUE showed it answers `-a`, with
the second operand discarded, on both backends (loft#1274). A bound is a promise about what
compiles, so a compile check feels like the whole of it; it is half. **Assert the value of
every operation a bound is said to permit**, because an operator that binds to the wrong
overload compiles perfectly. (loft#1274 is fixed — the spelling is refused now, and the
guard's cell that pinned the wrong answer had to go with it, which is the other half of the
same lesson: a cell written to record a defect is a cell that fails the day it is cured.)

⚠ **A chapter's general sentence is only tested at the types its examples use.** Chapter
26 said closures capture "at the moment the lambda is written … if a variable changes after
the lambda is written, the lambda still sees the original value", with no type on it. That
is true of every cell on the page — all of them scalars and text — and false of every other
type the sentence covers: a captured vector, hash, sorted, index or struct is SHARED, so a
later append reaches the closure and a write inside it reaches the enclosing scope. Nothing
on the page was wrong; the sentence generalising from it was. The chapter also said nothing
about the third regime, a scalar the closure WRITES to, which is shared rather than copied
and is how an accumulator is written. **So for any claim of the form "a variable …", list
the types the language offers in that slot and run one cell per type** — the claim's truth
is a function of the type, and a chapter's examples reliably pick one of them.

⚠ **And re-check the two COMPARISON pages, where the same claim is stated more strongly.**
`00-vs-rust.html` and `00-vs-python.html` exist to draw a contrast, so they restate the
reference's claims in their sharpest form — and they had the same wrong generalisation with
the hedges removed: *"Capture is always by value"*, and *"later mutations to the original
variable do not affect the lambda **(and vice versa)**. Python's `lambda` … closes over
variables by reference, so mutations are shared."* That draws the loft/Python line on exactly
the axis where the two agree, and it is aimed at the reader most likely to write an
accumulator closure. The vs-Rust page also listed "closures can be stored in structs" as a
Rust advantage, which loft has (a struct FIELD holds a capturing closure; only a COLLECTION
refuses one). These pages are their own review rows, but a claim corrected in a chapter is
owed a grep across them the same day.

⚠ **A section TITLE is a claim, and it is not held up by the cells under it.** Chapter 26's
"Closures with higher-order functions" contained no higher-order function — its cell defined
a capturing closure and called it directly. The capability is real (`map(nums, scale)` with
`scale` capturing works on both backends), so nothing failed and nothing said the section was
not showing its own subject. Read the titles as a list, on their own, and ask of each one
what cell would falsify it.

⚠ **A chapter demonstrating a feature ONCE pins the shape that works, and the suite then
guards the gap.** Chapter 27 shows `yield from` with a single sub-generator, `inner_vals()`,
which takes no arguments. Give the sub-generator an argument and `--native` emits Rust that
does not compile (loft#1277) while the interpreter runs it — so the chapter, the doc suite and
the native gate were all green on a feature broken for every parameterised delegation. The
single cell was not wrong; it was unrepresentative, and being executable made that invisible.
**For a feature the chapter demonstrates once, list the argument it varies and run the other
values** — here: does the sub-generator take arguments, is it nested, does it yield nothing.

⚠ **When the claim is about a NEGATIVE resource fact, first ask which instrument could see
it.** Chapter 27 promised "the abandoned generator frame is freed automatically", and the
store-leak gate is structurally blind to it: coroutine frames live in a side-table on `State`,
outside the store system on purpose, so a clean run proves nothing about the claim. The witness
had to be built — give the generator a store-backed local, so a retained frame retains a store
— and the property is an INVARIANCE, not a threshold: `peak` must not move with the number of
abandoned generators while `allocs` scales with it. A threshold alone passes on a pooled leak,
and an exit-time leak count cannot see growth at all. `tests/coroutine_matrix.rs
::an_abandoned_generator_frame_does_not_accumulate` is the shape, including the measurement
that proves the channel moves (three frames live at once reads `peak=5` against `3`).

⚠ **An internals doc can carry an example that does not COMPILE, and nothing will say so.**
COROUTINE.md § "Exhausting a generator early" documented ending a generator with `return;`,
with a worked example — while `formal/coroutines.md` (G-Return) says a generator has no
`return` and the compiler refuses all three spellings (`return;`, `return e`, and a body whose
tail is a value) with precise messages. The formal rule and the code agreed; the design doc had
simply not been re-read since the rule landed. Reference chapters are gated because they RUN;
`doc/claude/*.md` snippets are not, so **when a chapter's subject has a companion internals doc,
paste its examples into a file and run them** — and when the two docs disagree, the formal rules
settle it.

⚠ **The blind spot repeats one chapter later in a different variable, so name the variable
each time.** Chapter 26's general sentence was only tested at the TYPES its examples used;
chapter 28's "possibly different types" was the same defect with every cell all-`integer`, and
its "a value parameter gets its own copy" section called no function at all. Writing the missing
cell found loft#1278 — assigning to a `text` element of a by-value tuple parameter does not
compile on `--native`, while the interpreter gives the documented answer. `formal/tuples.md`
had already written the lesson down about ITSELF: its `OPEN: 0` read clean while loft#1004 and
loft#1005 were live because its oracle is all-`(integer, integer)` and carries no `text`, and
its deviations section says *"a conformance entry that names one member of a family is a claim
about that member, not the family."* **So: for each general sentence, name the variable it
quantifies over — type, arity, shape, position — and run one cell per value.**

⚠ **And that applies to the reviewer's own new cells.** Chapter 27 gained a section proving a
generator body is suspended, with a step counter — written with the loop shape that is lazy on
both backends. COROUTINE.md's CL-9 table says four other shapes still run the whole loop eagerly
on `--native`; measured, a generator with a statement AFTER the yield does 1000 body steps where
the interpreter does 5, values agreeing throughout. The new section was over-general in exactly
the way the chapter-26 entry above warns about, one chapter later, and had to be corrected after
it was committed. **A cell written to fix an over-general sentence is itself one example of one
shape** — check the caveat table for the subject before claiming the general case.

⚠ **A chapter where every section HAS a cell is not a covered chapter — read the prose that
sits between them.** Chapter 29 is the best-celled page in the reference: all eleven sections
run. Every defect was in a sentence no cell touched. Its wildcard section said *"without it,
the compiler will reject the match if any value could fall through without matching"*, which is
true of an ENUM subject and false of every scalar one — `formal/matching.md` (M-Total) draws the
line by DECIDABILITY, so an unmatched integer, character or text answers **null** instead. That
is the dangerous direction: a reader who trusts the sentence omits the `_` and gets a null
travelling through the program in place of the compile error they were promised. Its guard
section claimed *"the guard can reference variables bound by the pattern"* while its cell
guarded on an OUTER local, so the specific claim ran under nothing (it is true — measured). A
fully-celled chapter moves the whole budget onto the prose; it does not shrink it.

⚠ **When a chapter states a rule, check what the COMPILER says to someone who breaks it.** The
Match chapter tells the reader to put `_` last. Doing otherwise reported `Expect token }` — the
right caret with the wrong reason — and on the scalar path cascaded into four more errors about
the rest of the line, none naming the wildcard. Both match paths `break` out of the arm loop at
a total `_`, so the next arm met the closing-brace expectation. Fixed at both sites, with the
carve-out that a GUARDED `_ if cond` is not total and must still admit the arms after it. **The
chapter is where a reader learns the rule; the diagnostic is where they meet it when they get it
wrong, so the pass owns both** — and a refusal's message is a claim the suite does not check
unless someone writes the cell.

⚠ **A probe bounded by TIME reads an unbounded ALLOCATION as a hang — and the difference is
one `ulimit` away.** The Formatting pass met `println("{1:8.2}")`, which `LOFT_TIMEOUT` cut off
after sixty seconds, and it went into the notes as a hang with a guess about the instruction
stream. It is an allocator: the dotted spec leaves an `f64` in a slot the opcode reads as an
i64 width, which is a pad count of ~4.6e18, and the process grows until the kernel OOM killer
takes it — along with every other process in the session's cgroup, which is how three sessions
died before anyone read the journal. CLAUDE.md already says a time bound does not bound memory;
what this pass adds is that **the review's own probes are where you meet it first**, because a
review deliberately runs the spellings nobody has run. Run them under `ulimit -v`, and read
`anon-rss` in the OOM record before reaching for the page-cache explanation. The 2 GiB test
ceiling does not cover this — it is a STORE budget, and a Rust `String` is outside it.

⚠ **A refusal spelled as a LIST OF TYPES cannot say what it left out.** The compiler had one
line — *"a specifier that can never have any effect on the value type is always a bug"* — and
asked it of `text` and `boolean`. Every other type dropped a radix in silence, and a precision
was dropped or, on the dotted spelling, fatal. Nothing in the code looked incomplete, because a
list of two types reads as a decision rather than as a sample. The cure is to state the
question the renderer can answer — *which radixes does THIS type have an arm for* — which every
type must then answer, including the ones added later. **When a chapter documents a set (bases,
flags, precisions), check the code's refusal against the set, not against the examples**: the
examples are drawn from the same two types that already worked.

⚠ **Split a guard by the CHANNEL its control fails through.** The first draft of the pad guard
put assertion cells beside two cells that are a *compile error* on the control. Run against the
build it was written to catch, the file stopped at the parse error and not one assertion ran —
so it scored as caught while proving nothing about the four defects it was written for. The
existing sibling guard had already recorded this exact hazard, one file over. **`make falsify`
is what says so**, and only because it reports which channel moved: exit, asserts and refusals
are separate columns for this reason.

⚠ **A chapter can USE a construct it never explains.** Three cells in the Formatting chapter
compared against `"{{x:1,y:2}}"` — the doubled braces that mean one literal brace — and no
sentence anywhere on the page said what `{{` is. It is invisible to every check the suite has,
because the cells pass: the escape works. Read the chapter's own EXPECTED VALUES as a list and
ask which of them a first-time reader could not have written.

⚠ **A chapter can be a REGRESSION TEST that was published as documentation.** Chapter 31 was
33 lines: a `use p144_entry` against a `--lib tests/lib` fixture the reader cannot see, prose
whose middle sentence was *"This is P144"*, and assert messages carrying that tracker id into
all four release bundles. It never said what `&` means, when a parameter needs one, or what a
plain parameter already does — while `formal/calls.md` holds the one sentence a reader most
needs (`&` buys exactly one observable thing, whole-value REPLACEMENT; a plain parameter
already shares, appends and clears). Nothing was WRONG on the page, which is why no check
caught it: every assertion passed. **The tell is a chapter whose examples are named after an
issue** — and the fix is not to delete the regression but to notice it already lives in
`tests/issues.rs`, so the chapter is free to become a chapter.

⚠ **Run the chapter and READ ITS STDERR — a chapter that teaches an idiom the compiler
advises against is telling the reader two things.** The rewritten chapter emitted three
`advice[slow-reference-parameter]` notices on its own teaching examples. Two were the
compiler's bug (loft#1286: the lint could not see that a `&` passed ON to another `&`
parameter is load-bearing, so it advised dropping the one thing carrying the write-back —
and fired only on the CORRECT spelling). The third was right, and the chapter was wrong to
use `&` on a function that only appends: the page was demonstrating the opposite of the rule
it teaches, one section below teaching it. **Both directions come from the same read**, and
neither shows up in a chapter's exit status.

⚠ **When you suppress a diagnostic, count how many it still fires.** The fix for loft#1286
could have silenced the lint everywhere and every test would still have passed — the
chapter, the suites, `make ci`. The measurement that says otherwise costs one loop: 31
firings across `tests/scripts` + `tests/docs` before, 23 after, and each of the eight
suppressed is a function its own fixture calls a forwarder. A guard for a diagnostic that
must NOT fire also needs the true-positive control in the same file, because
`make falsify` has no channel for an absence.

⚠ **Read the RENDERED page, not only the chapter source.** Chapter 33's source opens with
an ordinary header; `doc/33-features.html`, `doc/print.html` and the Typst source for the
PDF opened with *"GENERATED by tools/features/gen.loft — DO NOT EDIT. @NAME: Feature
catalogue @TITLE: …"* — a maintainer's instruction and the page's own title, read back to
the reader as the first paragraph of all four release bundles. The renderer drops a header
line only while it believes it is still IN the header, and that block ends at the first
line its five-word vocabulary does not recognise; the generated chapter's third line ended
it and pushed the two directives after it into the prose. Nothing could report this: the
chapter runs, the directives are still consumed, and the leak exists only in the artefact
nobody re-reads. **Open the built page for every chapter you review.**

⚠ **A GENERATED chapter's prose is hand-written somewhere else, and it is the least
checked text in the reference.** Chapter 33 is written by `tools/features/gen.loft`, and
its drift guard proved the 117-entry LIST matched the issue tracker while every sentence
above the list was a hand-written string literal in the generator, checked by nothing —
including the sentence about the guard. Measured: hand-edit `@F45`'s line to *"REMOVED IN
1.4, DO NOT USE"*, and `make features-check` exits 0 printing "features shadow in sync",
because its scope named `doc/features tests/docs/features` and not the published chapter.
**A guard's scope must name every file its generator writes** — and when a chapter says a
guard covers it, run the guard against a broken copy rather than reading the sentence.

⚠ **A count the generator already prints is a sentence the generator should write.** The
same intro promised each of the 117 entries *"what it is, how it aids you, and a runnable
example"*; 35 are infra entries with a different set of headings and no example by design,
13 more are exempt with a written reason, and `features-gen` prints "67 runnable examples"
one line after writing the promise — `make features-review` prints the whole breakdown,
ending "35 infra entries (no example expected)". The fix is not a better-hedged sentence
but a generated one: the page now says "67 of the 82", from the counters the generator
already keeps. **Where a chapter states a quantity its own build knows, generate it.**

⚠ **A chapter that ENUMERATES is a closed-list promise, and the rest of the reference is
what falsifies it.** *"Everything loft can do, in one list"* is checkable against a second
enumeration the repo already has — its own chapters. Four of them describe a subject no
catalogue entry names: the `lexer` and `parser` LIBRARIES (chapters 15 and 16), `#lock`
(18) and `now()`/`ticks()` (22), while `@F39` "Math & trigonometry library" sets the
precedent that a stdlib group belongs there. Two of the four are worse than absent —
`@I57 Lexer` and `@I58 Parser` exist and describe `src/lexer.rs` and `src/parser/`, so a
reader looking for the library finds a page with the right name about something else, and
**the gap does not read as a gap**. Filed as loft#1288; the closed-list promise is gone
from the page, which now says an absent feature is a gap in the TRACKER rather than in
loft. **When a chapter lists, list the same thing from a second source and diff.**

⚠ **A guard that checks a SUBSTRING cannot see extra output — which is the direction a
tool drifts.** Chapter 34 is almost entirely shell transcripts, and they are not unchecked:
`tests/doc_commands.rs` runs every indented `$ ` line in a copy of `tests/docs/cli/` and
requires each line under it to appear in what the command printed. Three transcripts were
still wrong, because `contains` passes on a fragment. The page showed `ok`, and
`loft check hello.loft` printed `ok <absolute source> <absolute cache entry>` — a superset,
so green. It showed `5` for a piped REPL line, where a terminal shows a banner, a prompt
and `5` — the shown line is stdout and the chrome is stderr, which the page did not say.
**Before spending the read, find out what the existing guard's PREDICATE is**: this one
pins presence, never absence or equality, and the review's job is the half it cannot hold.

⚠ **And read the guard's opt-OUT, because that is where the unchecked blocks are.** The
same test treats an indented line WITHOUT `$ ` as illustration and never runs it — a
deliberate escape hatch, so a page can show an interactive session that no script can
drive. Chapter 34's two hand-typed REPL blocks lived there, which is why nothing noticed
that one of them claimed a resume the mode above it does not do. Rewriting them as `$ loft`
made 16 transcripts fail at once, and the failure list is the map: it names exactly which
lines were being asserted for the first time.

⚠ **When the tool and the chapter disagree, ask WHO the output is addressed to before
deciding which is wrong.** The two extra fields on the check line are a machine protocol
(@PLN18 08-S4): `live_dispatch` parses `ok <src> <artifact>` to find the build it just
asked for. One `println!` served both audiences, so a person typing the reference's own
command got a path they had just typed plus an internal content-addressed cache entry.
That is the REACH axis loft#1260 already draws for diagnostics, in a second channel: the
machine now asks for the machine form (`LOFT_CHECK_ARTIFACT`, set by the host that spawns
the driver) and the default answers the person. **A drifted output surface is not
automatically the doc's error** — the chapter had the right answer and the code had left it.

⚠ **A protocol with one consumer and no test is guarded by nothing — run the control
before trusting a green suite.** Deleting the env var that asks for the artifact form left
`tests/engine_host_reload.rs` — four live-reload tests, end to end — entirely green, so the
field's only consumer never exercised it and the change was unguarded in both directions.
`tests/check_line_audiences.rs` now carries a cell per audience, each falsified against the
build that breaks it: the person's cell fails on the pre-fix line, the host's cell fails
when the machine form is dropped.

⚠ **A demonstration in one MODE cannot carry a claim about another.** The chapter taught
the REPL through a PIPED transcript and then said *"Your session is remembered, so you can
close it and come back later"*. Auto-resume is interactive-only — REPL.md says so in its
own words, and measuring both ways confirms it: piped, the binding is gone on the next run;
on a pty (`script -qec 'loft repl' /dev/null`), `restored 1 statement(s) from last session`
and `x + 1` answers 42. The sentence was true of a mode the page never showed. **Name the
mode a transcript is in, then re-read every neighbouring claim against that mode** — and
when a probe cannot observe a claim, say so instead of scoring it false: the first piped
run looked like a broken promise and was a blind instrument.

⚠ **A flag demonstrated on input where it does nothing teaches nothing.** `--explain` was
shown on `hello.loft`, which has no diagnostics, so the whole transcript was `hello,
world!` and the fix line the section exists to describe never appeared. Chapter 20 was this
defect with a log config; this is its transcript form, and the tell is the same — the shown
output does not contain the thing the section is about.

⚠ **A transcript ends at the command; a person adds a pipe.** Running chapter 34's own
commands the ordinary way — through `head`, to read output that scrolls — aborted the
interpreter with SIGABRT and wrote `.loft/loft-crash-<pid>.txt` blaming `OpPrint` and a
stdlib line (loft#1289: `print!` panics on `EPIPE`, and when stderr shares the closed pipe
the panic printer fails too and the process aborts). Both backends panic; only the
interpreter aborts. Nothing in the chapter, the doc suite or `make ci` runs a pipeline, so
the whole class is invisible to them. **Run at least one of a chapter's commands into a
pager or a `head`.**

⚠ **Read the chapter's MODEL first, and the model of a testing chapter is which functions
RUN.** Chapter 35 said it three times — in its `@TITLE`, in "Write a test", and in "Run your
tests" — that a test is *"a function whose name starts with test"*. The runner asks for
`test_`. `testify`, and a function called exactly `test`, are not tests, the run reports
`ok` having never called them, and nothing anywhere says so. TESTING.md stated the same
rule twice, and the @F89 catalogue entry stated its OPPOSITE — *"Not the `test_` prefix —
**arity**"* — with a transcript (`(2 fns: setup, test_one)`) that no longer reproduces. One
`if` in `src/test_runner.rs`, four documents, three different answers. The real rule has two
cases and the second is what makes the reference itself runnable: **a file naming no
`test_*` keeps arity**, so every zero-parameter function is an entry point.

⚠ **A guard that names its own blind spot has told you when to come back.**
`tests/scripts/1010-test-runner-discovery.loft` is @F89's worked example, and its header
said: *"If the runner's rule ever becomes name-based, this file keeps passing … the runner
prints `(3 fns: …)` … and a name-based rule would print one."* The rule became name-based.
The file kept passing, and `loft --tests` on it prints `(1 fn: …)`. A carve-out comment is
a dated prediction, so **when a chapter's subject has a guard, read the guard's own caveat
before its assertions** — it names the measurement to re-take. The count it said could not
be asserted from inside now lives in `tests/test_discovery_rule.rs`, out where the runner's
output can be read.

⚠ **When one rule is written down in several places, collect all of them before fixing
one.** Grepping the corrected sentence found chapter 35, TESTING.md (twice) and the @F89
page, and the four had drifted in different directions rather than all lagging together —
so fixing the chapter alone would have left the catalogue page contradicting it, and that
page is the one chapter 33 sends readers to. The catalogue entry is an ISSUE, so the fix
goes there and `make features-fetch && make features-gen` brings the shadow along; expect
the fetch to carry other people's edits too, and say which in the commit.

⚠ **A negative control is what to do with a cell that has stopped running — and writing one
is how you find the runner you did not count.** Two cells of the witness above had gone dead:
one to the name rule, one to the wrap harness's P147 filter, which excludes value-returning
entry points and which the file claimed it did not have. Deleting them loses the coverage;
leaving them is an assertion nobody runs. So both became deliberately-FALSE assertions — and
one of them **fired in CI**. `tests/native.rs::prepare_native_test` has neither filter, so it
runs a set the interpreter half of the same corpus does not: 165 zero-parameter
value-returning functions across 66 main-less files, executed on the native pass alone
(loft#1293). Three runners had been checked by hand and the fourth was found by the control,
which is the whole argument for writing one. **A claim that "no runner executes this" is a
census, and a census is a measurement** — the cheapest way to take it is to assert the
opposite and let the suite answer.

⚠ **A checked transcript can still be a cell that cannot fail — delete the command and run
it again.** Chapter 36's transcripts all carry `$ `, so `tests/doc_commands.rs` runs every
one; two of them proved nothing. "Look at a value" typed `total` and expected `0`, and the
debugger's own paused line reads `⏸ paused in main | total = 0, i = 1` — so the substring
`0` is there whether or not the expression is ever evaluated. The `:step` cell expected `1`,
which `i = 1` supplies. Measured by deleting the command from the pipe: both cells still
pass. They now ask for `total + i * 111` and get `111`, a value nothing else on the line can
produce — which also demonstrates *"any expression works, not just a name"*, a claim the
page made and never showed. **The control for a transcript is the same command with the
interesting part removed**, and it is one line of shell.

⚠ **A command whose meaning is a DISTINCTION needs a program where the distinction exists.**
The chapter listed four movement commands — `:step` goes INTO a call, `:next` goes OVER one,
`:finish` runs until the current function returns — and demonstrated them on `count.loft`,
which contains no function call at all. All three descriptions are correct (measured), and
not one of them was reachable from the program the page used, so `:next` and `:finish` ran
under nothing and the single `:step` cell showed no stepping-into. A second fixture with a
call now carries three cells whose paused lines differ by function name. **When a section's
subject is the difference between two commands, check that the example can tell them apart.**

⚠ **`:help` is the chapter's own completeness check, and it is one command away.** The page
said `:help` "lists every command"; running it lists `:watch <expr>`, `:undo`, `:redo`,
`:vars` and the short forms `:s :n :o :c`, none of which the chapter mentioned. `:watch` is
the sharpest omission: the page tells the reader to watch a value change by re-reading the
paused line each time round the loop, and `:watch total` stops the run when it changes and
says `0 → 1`. **Where a tool can enumerate its own surface, diff that list against the
chapter** — the same move as diffing a table against the artefact it copies.

⚠ **Read the part of the output the chapter TRIMMED.** The paused line ends
`(+2 compiler temp(s) — \`:vars all\`)`, and the page quoted everything before it while
claiming the line shows "what every local variable holds". The suffix is the tool saying it
does not, and naming the command that does. A transcript shortened to what the author was
explaining is where a contradiction hides, because the guard only checks that the shown part
appears.

⚠ **A model corrected in one chapter survives in the next one that restates it.** Chapter
17's review found that a library name does NOT need its `libname::` prefix and rewrote the
page around it; chapter 37 still said *"Write `use greeter;` instead and you call it as
`greeter::greet(...)`"*, twenty chapters later and in the beginner's words. Both spellings of
`use` and both spellings of the call were measured — all four work. The review doc already
says to grep the whole reference for a sentence you fix; **the grep has to be for the MODEL,
not the sentence**, because the second statement of it shares no wording with the first.

⚠ **"Private" is a claim about what CANNOT be reached, so reach for it.** The same chapter
said *"Without `pub` a function is private to the package"*. It is not: `greeter::shout(...)`
calls a non-`pub` function from outside, and only the bare `shout(...)` is refused. `pub`
decides which names arrive, not what exists — chapter 17 had this right too. A negative about
visibility is one probe, and the probe is the same shape as every other stated "cannot".

⚠ **A rule that names two directories may be a rule about one of them.** *"'src' is where
your code lives and 'tests' is where your tests live. The names matter — loft looks in
exactly those places."* Half true: a test file outside `tests/` is not found, and an entry
moved to `lib/` with the manifest updated works fine — `src/<name>.loft` is the default when
nothing says otherwise, which chapter 17 states. **When a sentence asserts a rule about a
list, check each member**; the two halves of this one had different answers.

⚠ **A feature demonstrated in its EMPTY state teaches nothing, and the page usually says so
itself.** The coverage section promised *"It is a list"* and showed
`coverage: all 1 functions were entered by these tests` — one sentence, no list, because the
fixture had nothing uncovered. The list form exists and names file, line and function. This
is the `--explain`-on-a-clean-program defect (chapter 34) and the logging-with-no-config
defect (chapter 20) a third time, so it is worth stating as a rule: **for any reporting
feature, ask what it prints when it has something to report, and show that.**

⚠ **A caveat that names an ISSUE has a state you can look up in one command, and it may
have shaped the whole page.** Chapter 38 closed with *"What the panel cannot do yet"* — a
text- or vector-valued expression answers `<unavailable>` (loft#1187) — and then drew the
consequence: *"which is why every function on this page hands back one of the shapes that
works"*. `gh issue view 1187` says CLOSED, and `src/wasm_debug.rs`'s own test now asserts
`shout("hi")` → `"HI"` and `evens(4)` → `[0,2,4,6]`. So the section was wrong, and the page
had been BUILT around it: an interactive page about calling functions had no function that
answers text. **When a caveat cites an issue, check the issue before reading further, and
then ask what the page gave up because of it.**

⚠ **A stale caveat leaves copies in the code it described.** The same `<unavailable>` story
was written twice more inside `src/wasm_debug.rs` — the `eval_expr` doc comment still named
"a `text` local" as the unavailable case, sixty lines above the comment explaining how
loft#1187 fixed exactly that, and a test comment still said the issue "tracks making it
evaluate". Both are the fix's own file. **A closed gap is a grep for its DESCRIPTION, not
only for its issue number**; the number was already updated in one place and not the others.

⚠ **Check the premise of a page that claims to be different.** Chapter 38 opened *"Every
other page here shows you loft. This one hands it to you."* Every numbered chapter carries
the same `<section id="loft-panel">`, and `doc/loft-panel.js`'s `boot()` unhides it wherever
a source is present — so every other page hands it to you too. What is actually special is
the CONTENT: functions chosen to be worth calling. The distinction was real and the sentence
named the wrong half of it.

⚠ **The comparison pages have no cells at all, and it shows.** `00-vs-rust.html` is static
HTML: its fourteen loft blocks execute nowhere, and six of its statements were wrong, every
one of them in the direction that flatters Rust. It told a Rust reader that trait-bounded
generics "are not supported" and to write a version per type (`<T: Ordered>` and
`<T: Addable>` both compile, and the section's OWN downside paragraph discusses interface
bounds as existing — the heading and the prose beneath it disagreed); that a field is
nullable unless marked `not null` (the compiler now reports `not null` as deprecated and
inert — a type is non-null by default, and the page's own code asserted `p.label == null`
on a field that is not); that `&` is what makes an append reach the caller (a plain
collection parameter is already shared); and it called the length builtin `length`, twice,
which is not a function. **A page with no runnable cell accumulates exactly the defects a
chapter's cells would have caught, and it is read by the person least able to check it.**

⚠ **A pass that CHANGES behaviour owes the comparison pages a grep the same day — this is
the measurement of what happens when it does not.** The Formatting review (6686e0d9) made
zero-padding reach floats and added `{3.125:08.2}` → `00003.12` to its own chapter. Both
comparison pages still said the flag is "dropped for floats", four chapters later, and the
vs-Python page said it in Python's own vocabulary. The instruction was already written down
here; what it lacked was a guard, and
`tests/scripts/the-comparison-pages-draw-the-line-where-it-is.loft` is now it.

⚠ **Read why a control is RED before believing its verdict.** `make falsify` reported that
guard as falsified on all four rows — a clean result, and wrong: the control had failed with
`cannot load default library`, because the harness runs a guard from the checkout it is in
and an absolute path finds no `default/`. Re-run inside the control tree, exactly ONE cell
of six fails, which is the honest answer and a far more useful one: five cells are LOCKS on
a language that never moved, and the sixth dates the change to this branch.

⚠ **The two comparison pages carry the SAME defects, so review them together.** Everything
found on `00-vs-rust.html` was on `00-vs-python.html` too — the inverted null model, the
`length` builtin that does not exist, a code block telling the reader that operations on `T`
need a per-type version while its own downside paragraph names the interface bounds. Only
the framing differed. Two more were the Python page's own: it drained a queue with
`queue#remove` inside a `while` loop, where `#remove` is valid on a loop ITERATION variable
only (so the block does not compile), and it told a Python reader — as loft's defining
limitation against Python's ecosystem — that *"there is no package manager or external
dependency system"*, while `loft install` resolves `[dependencies]` from a signed registry
of 49 packages and `loft new` / `publish` / `yank` are subcommands. **A claim about what a
language does NOT have is the one to check hardest on a page whose job is to say so.**

⚠ **A promise of a REFUSAL is worse than a promise of a value.** The Python page's null
block ended `u.age = null;  // compile error: age is not null`. It is a warning; the write
lands, and `u.age` then reads `null` out of a field typed `integer`. A reader who was
promised a refusal writes no check at all, so this is the direction that costs — the same
shape as chapter 23's five stated negatives the language does not have.

⚠ **The install page is where a stale "planned" costs the most, because the reader has no
way to know.** `install.html` said *"Loft syntax highlighting extensions are planned. In the
meantime: VS Code — use Rust syntax highlighting as a close approximation"*, and told Vim
users to `set filetype=rust`. The repository ships `editors/vscode/` — a language id, a
TextMate grammar, snippets, Run buttons on F5 and Ctrl+F5 — and two binaries, `loft-lsp` and
`loft-dap`. Driven over stdio, `loft-lsp` advertises diagnostics, completion, definition,
references, hover, document symbols, semantic tokens, inlay hints, rename and formatting.
The extension's OWN README repeated the stale sentence one level down. **A first-run page's
"planned" is a claim with a date on it; check it against `src/bin/` and `Cargo.toml` before
believing it**, and when you fix it, fix the copy the tool ships with.

⚠ **A version number in a prerequisite is checkable from the manifest.** The page asked for
"Rust 1.82 or later" to build, while `Cargo.toml` says `edition = "2024"`, which needs 1.85 —
so a reader who follows the instruction gets an edition error on their first command. The
page's own later line, "Native compilation requires Rust 1.85 or later", had the right number
attached to the wrong step. **Two version numbers on one page that disagree is a lead, and
the manifest settles it.**

⚠ **A page written before a default changed keeps teaching the old one.** *"The interpreter
runs programs immediately, but for compute-heavy work you can compile to a native binary
instead"* — `loft prog.loft` already compiles through `rustc` when it can find it, which is
what `--help` says and what chapter 34 teaches. The flag table listed `--native` and never
`--interpret`, so the page had no name for the thing it said you were already getting.

⚠ **A roadmap page dates faster than any other, and its "planned" list is where shipped
features go to hide.** `roadmap.html` called 0.8.4 the *current* release (shipped
2026-04-24) and 0.8.5 *next* (shipped 2026-06-07), under a semver scheme the project left in
2026-06 for calendar versions — `loft --version` prints `2026.8.0`. Its 0.9.0 section listed
as future work eight things that ship today: error recovery (a two-error program reports
both in one pass), the REPL, the lint set, the TextMate grammar, the VS Code extension, the
`examples/` directory, `loft.lock`, and CI over package tests. **Check a roadmap's PAST
before its future** — the version the binary reports settles where the page should start,
and every bullet is a claim you can run.

⚠ **Re-measure a caveat list rather than trusting the issue state, and re-measure the ISSUE
rather than trusting the changelog.** `CAVEATS.md` presented #1030–#1034 as live with
`silent-wrong` / `sev:high` labels; all five are closed. Running them anyway is what found
loft#1296: a width type's overflowing `+=` no longer keeps `260` — the fix made every
spelling answer **`0`**, an in-range value, while `formal/types.md` says an overflow yields
`null` "never a wrapped / saturated / out-of-range value" and plain `integer` and a nullable
`u8?` both do. The 2026-08 changelog also claimed "`u32` finally holding every `u32`"; `u32`
is declared `integer limit(0, 4294967294)` with the top value reserved as the null sentinel,
so it never held every `u32` and is not meant to. **A closed issue is a claim that something
changed, not a claim about what it changed to.**

⚠ **The Standard Library's defects were STRUCTURAL, and none of them is a wrong sentence.**
Every other row in this table was read claim by claim; the stdlib's 251 entries are generated
from `default/*.loft`, and what was wrong was which of them reached the page at all. Three
faults, each invisible from either side — the source looked right and the page looked
plausible:
1. `gendoc` read a hard-coded list of three `default/*.loft`. Four more had joined the
   directory since, so `04_stacktrace`, `05_coroutine`, `06_json` and `07_reflect`
   contributed nothing: the entire JSON and reflection API was absent from the published
   Standard Library while the JSON chapter and @F42 documented it. 187 of 214 `pub fn`
   reached a page.
2. A blank line between a doc comment and its declaration orphaned the doc. The same
   authoring shape is written both ways across `default/` — `// --- min / max / clamp ---`
   puts its doc against `pub fn min`, `// --- Text ---` leaves a blank before
   `pub fn split` — and only the second lost it. **43 entries shipped as a bare signature**,
   `sin`, `sqrt`, `floor`, `round` and `split` among them.
3. Section names and descriptions came from the maintainer's side of the file: pages titled
   "System directories (#635)" and "Vector operations (T2-8, T2-5)" — tracker tags in the
   published URL — and a Text section whose description was `OpVarText`'s comment, *"Read
   the value of a variable and put a reference to it on the stack"*.

**For a GENERATED reference section, the first question is not whether a sentence is true but
whether the generator can see everything it claims to cover.** Counting the source's public
declarations against the page's entries takes one script and answers it.

⚠ **A doc that promises `null` is checkable against the declared type, and then against the
code.** Cross-checking every `pub fn` whose documentation says "null" against its return type
turned up 21 candidates; running them found two that never answer null at all —
`as_bool` (every JSON kind mismatch answers `false`, where its three siblings all answer null:
a matrix of four extractors × seven kinds says so) and `env_variable` (an unset variable
answers `""`, so the `== null` test its own doc invites never fires). Filed as loft#1302.
`as_text`'s doc also said "returns `null` (empty text)"; it returns null, and the two are
distinguishable. **The type is not the obstacle — `as_text` is equally non-null and answers
null through the sentinel — so a doc and a type agreeing still leaves the code to check.**

⚠ **And check the documentation you write yourself the same way.** Writing a doc comment for
`store_load_key`, which had none, I named the collection kinds it accepts by mirroring its
text-keyed sibling's wording. I had not measured them. The claim came back out; what the
implementation supports (`load_key` is `load_keys(…) > 0`) is what the comment says now. A
reviewer's own sentence is a claim with no more standing than the one being replaced.

1. **Is every claim still true?** Not "does the example run" but "does the prose
   describe what the language does now". A behaviour that changed under a chapter that
   did not is the failure this pass exists for.
2. **Does it promise something we do not deliver?** A capability described in the
   future tense that never landed, a flag that was renamed, an idiom that now emits a
   diagnostic telling the reader not to write it.
3. **Is anything missing that a reader would look here for?** A feature shipped this
   cycle whose chapter was never extended is invisible to everyone who reads the
   reference rather than the changelog.
4. **Is the recommended idiom still the one we would recommend?** Advice ages faster
   than syntax. If a chapter teaches a pattern that a lint now flags, the chapter is
   wrong even though every sentence in it is accurate.

Record the read below. One row per chapter source, and the commit is the one you read
**through** — normally `HEAD` at the time.

⚠ **A caveat that cites an issue is a claim with an expiry date, and the tag was the only
thing marking it.** The 2026-09 chapter pass found five caveats promising limitations the
language no longer has — the parser "does NOT accept" `v[0]`/`??`/format literals
(loft#1259, all three parse), production mode "interpreter only" (loft#1263, native logs
FATAL and continues), `yield from sub(arg)` "will not compile natively" (loft#1277), a text
element write on a value tuple parameter (loft#1278), and an inline generic result that
"retains one record per call" (loft#1273, no leak either backend). Every one had survived
loft#1348's tracker-tag pass with its tag stripped and its sentence kept, which is the worst
of both: the reader still sees the limitation and nothing now says it was ever provisional.
One (the `[levels]` prefix rule) was reworded AROUND the stale claim, making it wrong in a
new way. Each was settled by running it on both backends, never by reading the issue's
close — *a closed issue is a claim that something changed, not what it changed to*. The rule
that falls out: when a chapter's caveat loses its citation, the caveat is re-run or removed,
not kept. `tests/docs/26-closures.loft`'s `&`-capture caveat is the control — its refusal
stands and now names the chapter's cure.

## Watermarks

The one home for "reviewed through". `scripts/reference-review.py` parses this table;
there is deliberately no second machine-readable copy, because it would drift the moment
someone updated only the prose.

⚠ **A watermark names a COMMIT, and no rebase or cherry-pick preserves one.** Joining the
two checkouts rewrote the three rows below onto their twins, and until that was done the
worklist reported all three chapters as owing a re-read — each was being measured against a
commit unreachable from this branch, so its OWN review commit read as a change since the
review. Re-point the rows whenever commits are replayed; the tell is a chapter whose only
"commit since" is the one whose subject names that chapter.

⚠ **A GENERATED chapter moves when its inputs move, so it can owe a re-read after a commit
that never touched it.** Chapter 33 is written by `tools/features/gen.loft` from the issue
tracker, so the chapter-35 pass — which corrected the @F89 entry and refreshed the snapshot
— rewrote one number on it and reopened its row. Read the diff before re-pointing: a
generated chapter whose only change is a generated COUNT has not lost its review, while one
whose entry list changed has.

⚠ **A re-read is not a formality: three chapters moved, and all three had lost something.**
The 40/40 pass finished and then four commits landed under three chapters — a bound fix, a
generated-catalogue addition, and three cherry-picks into `default/`. `make reference-review`
reopened the rows and every one of them paid: a bound the chapter never taught, a self-count
that had gone stale, and a section header a pick silently dropped. The rate is the finding.
A chapter's review expires the moment its source moves, and "the commit was small" is not
evidence, because none of these three commits SET OUT to touch documentation.

⚠ **A table of "the built-in X" is a claim about the SOURCE, and counting is the whole check.**
The Generics chapter taught six built-in interfaces in two tables. `default/01_code.loft`
declares seven `pub interface`: `Walkable` was never in either table, and appears in no
chapter of the reference at all. It is not obscure — the stdlib's own `tree_walk` is bounded
by it, so a user type that defines `children()` gets a breadth-first walk for free, and a
reader with a tree writes their own walk instead. `grep -c '^pub interface'` against the
chapter's row count is a one-line check that nothing in the review was doing, and it is the
same check every "here are the built-ins" list in the reference owes.

⚠ **A GENERATED chapter can be committed with a STALE generated number, and only the gate
sees it.** Chapter 33 said "66 of the 82 carry a runnable example" while the catalogue held
86 with 70 — the four entries added a day earlier reached the LIST and not the SENTENCE
counting it. `make features-check` was red for the whole day and nothing else was. The
neighbouring lesson says a generated chapter whose only change is a count has not lost its
review; the inverse is the sharper half — a generated chapter whose count DID NOT change
when its list did has lost exactly that. Run the chapter's own drift gate before re-pointing
its watermark, and cross-check the number against the index rather than the generator that
wrote both (86 features, 70 with `fn main` plus 3 fragments = the 73 fenced examples in
`index/features.json`).

⚠ **A cherry-pick takes the other branch's version of the REGION, not of the change.**
`d34b375f` is the sibling's `as_bool` fix. Taking it also deleted `// ---  JSON  ---` from
`default/06_json.loft` — a section marker added by the Standard Library review one commit
earlier, sitting a few lines above the doc comment they edited. Nothing in the commit's
subject, message or diffstat mentions sections. The guard added in that same review is what
made it visible: gendoc printed *"Json declares a public item before any `// --- Section ---`
marker"* on every run since, and the published section fell from "JSON" to "Json". Read a
pick's diff against the region you changed, not against its stated subject — and this is the
second half of the lesson about a clean prose merge keeping both halves: it can also keep
neither.

⚠ **A doc comment corrected in the same commit as a real fix is the least-reviewed prose
there is.** `d34b375f` fixed `as_bool` (loft#1302) and, in passing, wrote "returns `null`
(empty text)" onto its sibling `as_text`. Measured on both backends: a kind mismatch does
NOT compare equal to `""` (`false`), and `len` reads **1**, not 0, while a field that really
holds `""` compares equal and measures 0. So the parenthetical named the one test that
cannot see a mismatch, in a doc whose entire subject is telling a mismatch apart — the exact
defect `as_bool` had just been fixed for, re-introduced next door as a sentence. It rode in
under a commit whose message is about something else and whose fix was correct.
`tests/scripts/json-extractors-say-how-a-mismatch-is-told-apart.loft` pins all four
extractors; its `as_text` half is deliberately un-falsifiable, because no build answers
differently and only a cell could ever have caught the claim.

| chapter source | reviewed through | commit |
|---|---|---|
| `default` | 2026-09-16 | `ad62fbac1` |
| `doc/00-vs-python.html` | 2026-09-03 | `b1ccf0e9` |
| `doc/00-vs-rust.html` | 2026-09-03 | `b1ccf0e9` |
| `doc/install.html` | 2026-09-03 | `b1ccf0e9` |
| `doc/roadmap.html` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/01-keywords.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/02-text.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/03-integer.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/04-boolean.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/05-float.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/06-function.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/07-vector.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/08-struct.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/09-enum.loft` | 2026-09-16 | `e5dcf2deb` |
| `tests/docs/10-sorted.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/11-index.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/12-hash.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/13-file.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/15-lexer.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/16-parser.loft` | 2026-09-05 | `b10ef6d6` |
| `tests/docs/17-libraries.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/18-locks.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/19-threading.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/20-logging.loft` | 2026-09-05 | `b10ef6d6` |
| `tests/docs/22-time.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/23-safety.loft` | 2026-09-05 | `b10ef6d6` |
| `tests/docs/24-json.loft` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/25-generics.loft` | 2026-09-05 | `b10ef6d6` |
| `tests/docs/26-closures.loft` | 2026-09-05 | `b10ef6d6` |
| `tests/docs/27-coroutines.loft` | 2026-09-05 | `b10ef6d6` |
| `tests/docs/28-tuples.loft` | 2026-09-05 | `b10ef6d6` |
| `tests/docs/29-match.loft` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/30-formatting.loft` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/31-ref-forward.loft` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/33-features.loft` | 2026-09-16 | `e5dcf2deb` |
| `tests/docs/34-running.loft` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/35-testing.loft` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/36-debugging.loft` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/37-projects.loft` | 2026-09-03 | `b1ccf0e9` |
| `tests/docs/38-call-it-yourself.loft` | 2026-09-03 | `b1ccf0e9` |

## See also

- [RELEASE.md](RELEASE.md) § the per-release checklist — where this pass is reported
- [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) — the same watermark idea over the
  libraries, and the pass this one is modelled on
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the prose should read once it is true
