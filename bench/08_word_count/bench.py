"""Benchmark 8: Word frequency count (300,000 hash operations).

The timed loop runs `--n` ops, each on an input that differs by the repetition number in
VALUE, never in the amount of work, and folds every result into `sink`.  The `hash`
column is one canonical op's result; every lane must print the same one.
"""
import sys
import time

WORDS = ["the", "quick", "brown", "fox", "jumps", "over", "the", "lazy", "dog",
         "the", "fox", "and", "the", "dog", "are", "friends", "the", "end"]


def tally(ops, salt):
    freq = {}
    for i in range(ops):
        w = WORDS[(i + salt) % 18]
        freq[w] = freq.get(w, 0) + 1
    return freq["the"] * 100 + freq["end"]

def arg_n(dflt):
    """`--n N`: how many ops the timed loop runs (the harness calibrates it per lane)."""
    n = dflt
    for i, a in enumerate(sys.argv):
        if a == "--n" and i + 1 < len(sys.argv):
            n = int(sys.argv[i + 1])
    return max(n, 2)


def row(name, iters, us, items, result, sink):
    """One measured routine, in the row format every lane prints (bench/README.md)."""
    print("routine\titers\tus\tns_op\tpx\tns_px\thash")
    print(f"{name}\t{iters}\t{us}\t{us * 1000 // iters}\t{items}\t{us * 1000 / (iters * items):.3f}\t{result:x}")
    print(f"time: {us // 1000}ms sink={sink}")


def timed(n, op):
    t0 = time.perf_counter_ns()
    sink = 0
    for r in range(n):
        sink += op(r)
    return (time.perf_counter_ns() - t0) // 1000, sink

if __name__ == "__main__":
    n = arg_n(2)
    ops = 300_000
    us, sink = timed(n, lambda r: tally(ops, r & 1))
    row("word_count", n, us, ops, tally(ops, 0), sink)
