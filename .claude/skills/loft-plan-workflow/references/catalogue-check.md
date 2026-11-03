# Checking the catalogue against what shipped

*Read this when work **adds, changes, renames, or removes a feature**, in a tree where
a tracker issue is the canonical description of that feature and the docs are
generated from it. Not only at close — the likeliest thing to drift is behaviour that
moved DURING the plan.*

Where a tracker issue is the **canonical** description of a feature — and the
reference docs and example tests are GENERATED from it — shipping the code is only
half the change.  **The issue *is* the documentation.**  Code that lands without its
issue being updated publishes a wrong description, and the generated shadow gives
that wrong description the authority of a checked-in file.

Verify three things, in this order:

1. **Existence** — every feature the work shipped has an issue.  Something built with
   no catalogue entry is invisible to everyone who reads the catalogue instead of the
   code.  Something *removed* still has one, and shouldn't.
2. **Accuracy** — the prose and the example describe what the code does **now**:
   the real name, signature, defaults, and behaviour, not what was proposed.  **Run
   the example** — do not eyeball it.  If the generator turns an example into a test,
   a stale example is a test asserting the old contract.
3. **Tags** — the labels match reality.  A label is a query surface: the wrong one
   hides the feature from everyone who filters by it, which is indistinguishable from
   the feature not existing.  Check the kind/subject partition and any status or tier
   the catalogue sorts on.

Then **regenerate the shadow and run the drift guard**.  Never hand-edit a generated
file — edit the issue and regenerate.  A drift guard that fails on hand-edits is
protecting the one-home rule, not obstructing you.

**A green coverage gate is not evidence the catalogue is complete.**  Know what yours
actually measures before trusting it.  Coverage gates typically attribute *source
regions* to catalogue entries — so a capability implemented inside a region that is
already attributed to some other entry never trips them.  New file, no entry → caught;
new feature in an old file → invisible.  That is the normal case for a mature codebase,
which makes this section a **human** step the gate cannot replace.

**Two traps worth naming.**  A generator that promotes the FIRST fenced example into
a runnable test means a teaching snippet placed above the real example silently
becomes the test — put the runnable example first.  And a feature issue closed or
labelled by hand skips whatever automation the merge path would have run, so its tags
drift exactly like a hand-closed plan's do.

## In one line

Shipping a feature is not done until its catalogue entry describes the built thing.
Stale prose, a stale example, or a wrong label ships as authoritative documentation —
verify existence → accuracy → tags, run the example, regenerate, let the drift guard
confirm.

