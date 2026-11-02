# @PLN91 1b-frame — history

What [1b-frame.md](1b-frame.md) no longer carries because it is past.

## PF4 before loft#2310 (2026-10-31 → 2026-11-02)

A text loop's peak memory grew with its round count — 15 → 84 MB from 20 000 to 200 000 rounds
before F-1, 14 → 68 MB after F-1b.  The cause was not the frame: `put_text` writes the text stack
with a replace on one branch and an append on the other, and `@FR-H-TextReplace` counted its
`text` parameter by mention, so it declined the release and every text the run evaluated stayed
in the stack vector's store.  Read by path (loft#2310), the release is admitted and the peak is
flat.
