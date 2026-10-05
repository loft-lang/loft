# A team of eager juniors who google a lot

Building a project with AI agents is not so different from running a project staffed by a lot
of eager but inexperienced programmers who google a lot.

They are eager: they never tire, they work through the night, and they will happily take on
anything you point them at. They are inexperienced: they do not take the time to find where a
problem really comes from, so they fix exactly the problem in front of them and miss the fix
that was actually needed. And they google a lot: they know a great deal about programming in
general and nothing about *your* project, unless your project tells them.

None of that is new. Every team lead has managed people like this. What is new is having a
whole team of them, at once, for the price of a subscription. This post is about what that has
meant for loft, the programming language I am building, and the games and libraries built on
top of it.

## Loft is not a new language

Loft had been in development for eight years before any AI was involved. The agents did not
invent it; they inherited it, with its own ideas about memory, data and types, and they work
inside those ideas.

What changed is not only the speed. The language has become much more *complete*. An agent
can take another language — Julia, OCaml, Python — and turn its idioms into small loft
programs. Every program that does not work is a question: should loft be able to say this?
Some answers became features, some became decisions not to, and some turned out to be bugs.
I would never have had the patience to do that by hand, language after language. An eager
team does not mind.

## What I actually do

I write very little of the code myself now. My work is steering: guiding the agents around
problems, in many different ways, and devising ways of working that make the most of what
they are good at while covering for what they cannot see.

Their strengths are easy to list: they are fast, they do not tire, they know a lot, and they
follow a written method faithfully once they have read it. Their blind spots are just as
clear: they remember nothing between sessions, they rush to a fix, they trust their own
explanation, and they apply what is true in general where it is not true here.

So most of my effort goes into how the project works rather than into its code. Each blind
spot gets a habit, a document or a tool that makes it harmless. And every correction I find
myself making twice gets written into the project, so the next session starts with it.

## Nobody remembers anything

An agent starts every session with no memory. Everything that needs to be remembered has to be
in the project itself: the bugs, the features, the rules, the design decisions, each tagged so
it can be found and tested again. The documentation is not a nice extra; it is the team's
memory.

That memory goes stale, just like a person's. A number written in a document is true on the
day it is written, and quietly wrong a month later. So the habit is to write down the command
that produces a number, not the number.

None of this is unique to AI. A team of people needs the same discipline. People just hide the
problem for a while by remembering things. Agents cannot, so the gaps show up immediately.

## The exact problem is fixed, the real one is not

This is where inexperience shows most. An agent finds a bug, finds a change that makes the
failing case pass, and stops. The exact problem is fixed, but the place it came from is not,
and the same mistake keeps living elsewhere. Over time you get many slightly different copies
of the same logic, each patched for its own bug, with no design holding them together.

Optimisation shows it even more clearly. The profiler points at a slow line, and the tempting
fix is a special shortcut for exactly that line. It works, and it helps nothing else. Looking
for the source usually finds something different, and fixes every program that has the same
shape. That is why loft treats a slow library routine as work for the compiler, never as a
place for a hand-written shortcut.

Two things help. The first is making it cheaper to look than not to: before fixing anything
non-trivial, the agent maps out the cases around the bug to see where the real boundary is.
Our working rule says it bluntly: *the urge to fix is the signal you have not earned it yet.*

The second is a set of formal rules: precise statements of how the language must behave. The
rules come first, and code that disagrees with them is what gets fixed. When a bug comes in,
the rules usually already say what the right answer is, so there is much less guessing. But
the rules only help if they are checked again and again. A rule that "has no open problems" is
a claim, and a claim has to be measured.

## Tests have problems of their own

Agents write a lot of tests. That is good, but the tests become a system of their own that
needs managing: how long they take, how much memory they use, how much disk space they fill
when several agents run them at once.

And a test that cannot fail is worse than no test, because it makes you feel safe. So each new
test is checked against the old, broken version of the code, to prove it would have caught the
bug it was written for. Eager programmers write a lot of tests; someone has to check that the
tests are testing something.

## The upside

With all those challenges, why do it? Because the result is worth it.

- **Work happens while I sleep.** Several agents work through the night on different parts of
  the language.
- **Bugs are found by checking rules, not by waiting for users.** Most bugs now show up when
  the rules are validated or loft is compared with other languages, before anyone runs into
  them in a real program.
- **The turnaround is fast.** A bug is often fixed within the hour, with a test that proves it.
- **The cost is low.** I pay about 200 euro a month. Before, getting the base language into
  good shape meant taking half a year of sabbatical.

## What I would tell someone starting

Treat the agents like a team of eager, inexperienced programmers who google a lot, and treat
your own job as designing how that team works. Do not expect them to remember, so make the
project remember for them. Do not expect them to look for the source of a problem, so make
looking cheaper than skipping it. Do not trust their explanations, so give them tools that can
prove an explanation wrong. And do not trust your own documentation either: check it again.

That is not really advice about AI. It is advice about leading a team. The AI just makes it
impossible to skip.

*Loft is open source: [github.com/loft-lang/loft](https://github.com/loft-lang/loft). The rules,
the working methods and the record behind everything in this post are in the repository.*
