"""Benchmark 11: par equivalent — 100,000 elements of 50-step Newton's sqrt, 4 processes.

The timed loop runs `--n` ops, each on an input that differs by the repetition number in
VALUE, never in the amount of work, and folds every result into `sink`.  The `hash`
column is one canonical op's result; every lane must print the same one.
"""
import sys
import time

import math
from multiprocessing import Pool


def newton_sqrt(x):
    guess = x / 2.0
    for _ in range(50):
        guess = (guess + x / guess) / 2.0
    return guess


def chunk(bounds):
    lo, hi = bounds
    return [newton_sqrt(float(i + 1)) for i in range(lo, hi)]


def one_pass(pool, count):
    part = count // 4
    bounds = [(t * part, count if t == 3 else (t + 1) * part) for t in range(4)]
    total = 0.0
    for values in pool.map(chunk, bounds):  # summed in item order, as loft's `par` delivers
        for v in values:
            total += v
    return int(math.floor(total + 0.5))

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
    count = 100_000
    with Pool(4) as pool:
        warm = one_pass(pool, count)
        us, sink = timed(n, lambda r: one_pass(pool, count))
    row("par", n, us, count, warm, sink)
