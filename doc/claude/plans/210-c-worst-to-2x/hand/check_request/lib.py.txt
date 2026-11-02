import re
def fn_span(src, sig):
    """(start, end) char offsets of the function whose definition line starts with sig."""
    i = src.index(sig)
    while not src[i:].split('\n',1)[0].rstrip().endswith('{'):
        i = src.index(sig, i + 1)
    j = src.index('\n}\n', i) + 3
    return i, j
def edit_fn(src, sig, f):
    i, j = fn_span(src, sig)
    return src[:i] + f(src[i:j]) + src[j:]
def sub1(s, old, new, count=1):
    n = s.count(old)
    assert n == count, (n, old[:80])
    return s.replace(old, new)
