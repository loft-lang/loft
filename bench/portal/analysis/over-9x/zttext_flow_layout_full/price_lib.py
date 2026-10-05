"""Shared line-edit helper: each price script rewrites WHOLE LINES of flow.rs by number (the
flow-only emission at 29d68fde2); every line edited is asserted to carry its marker first."""
import sys
def edit(path, out, edits):
    L = open(path).read().split('\n')
    for ln, marker, new in edits:
        assert marker in L[ln-1], (ln, marker, L[ln-1][:120])
        L[ln-1] = new
    open(out, 'w').write('\n'.join(L))
