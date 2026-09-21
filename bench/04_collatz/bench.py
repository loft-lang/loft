"""Benchmark 4: Collatz sequence lengths (starts below 200,000).

The timed loop runs `--n` ops, each on an input that differs by the repetition number in
VALUE, never in the amount of work, and folds every result into `sink`.  The `hash`
column is one canonical op's result; every lane must print the same one.
"""
import sys
import time

def collatz_len(start):
    cur = start
    steps = 1
    while cur != 1:
        cur = cur // 2 if cur % 2 == 0 else 3 * cur + 1
        steps += 1
    return steps


def longest(lo, limit):
    max_steps = 0
    max_n = 0
    for i in range(lo, limit):
        s = collatz_len(i)
        if s > max_steps:
            max_steps = s
            max_n = i
    return max_n * 1000 + max_steps

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
    limit = 200_000
    us, sink = timed(n, lambda r: longest(1 + (r & 1), limit))
    row("collatz", n, us, limit, longest(1, limit), sink)
