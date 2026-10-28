// Benchmark 20: a bytecode interpreter written in loft (consumer: @PLN91's self-host, `selfhost/src/run.loft`)
//
// The Rust twin of bench.loft: the same run loop over the same ops — a `match` on a plain enum,
// each op an `Op { code, a, b }` read out of a vector, a value stack of `Option<i64>` pushed
// through `put` (overwrite below the length, append at it), the call frames in three `Vec<i64>`
// — and loft's integer semantics on it: a null operand, an overflow or a division by zero is
// null.  The `hash` column must equal the loft lanes'.
use std::hint::black_box;
use std::time::Instant;

// Every code the self-host's lowering emits, used by these programs or not.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
enum Code {
    PushInt,
    Load,
    Store,
    Pop,
    Jump,
    JumpIfFalse,
    Call,
    Ret,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    BitAnd,
    BitOr,
    BitXor,
    Not,
    NonNull,
    PushNull,
}
use Code::*;

#[derive(Clone, Copy)]
struct Op {
    code: Code,
    a: i64,
    b: i64,
}

struct Prog {
    ops: Vec<Op>,
    slots: i64,
    #[allow(dead_code)]
    params: i64,
}

fn put(stack: &mut Vec<Option<i64>>, sp: i64, v: Option<i64>) {
    if (sp as usize) < stack.len() {
        stack[sp as usize] = v;
    } else {
        stack.push(v);
    }
}

fn arith(code: Code, x: Option<i64>, y: Option<i64>) -> Option<i64> {
    let b = |c: bool| Some(i64::from(c));
    match code {
        Eq => b(x == y),
        Ne => b(x != y),
        _ => {
            let (x, y) = (x?, y?);
            match code {
                Add => x.checked_add(y),
                Sub => x.checked_sub(y),
                Mul => x.checked_mul(y),
                Div => x.checked_div(y),
                Rem => x.checked_rem(y),
                BitAnd => Some(x & y),
                BitOr => Some(x | y),
                BitXor => Some(x ^ y),
                Lt => b(x < y),
                Le => b(x <= y),
                Gt => b(x > y),
                Ge => b(x >= y),
                _ => Some(0),
            }
            .filter(|r| *r != i64::MIN)
        }
    }
}

fn run(progs: &[Prog], entry: usize, args: &[i64]) -> Option<i64> {
    let first = &progs[entry];
    let mut stack: Vec<Option<i64>> = Vec::new();
    let mut sp: i64 = 0;
    for &a in args {
        put(&mut stack, sp, Some(a));
        sp += 1;
    }
    for _ in args.len() as i64..first.slots {
        put(&mut stack, sp, Some(0));
        sp += 1;
    }
    let mut f_def: Vec<i64> = Vec::new();
    let mut f_pc: Vec<i64> = Vec::new();
    let mut f_base: Vec<i64> = Vec::new();
    let mut fp: i64 = 0;
    let mut def = entry as i64;
    let mut pc: i64 = 0;
    let mut base: i64 = 0;
    let mut ops: &[Op] = &first.ops;
    loop {
        let op = ops[pc as usize];
        pc += 1;
        match op.code {
            PushInt => {
                put(&mut stack, sp, Some(op.a));
                sp += 1;
            }
            Load => {
                let v = stack[(base + op.a) as usize];
                put(&mut stack, sp, v);
                sp += 1;
            }
            Store => {
                sp -= 1;
                stack[(base + op.a) as usize] = stack[sp as usize];
            }
            Pop => sp -= 1,
            Jump => pc = op.a,
            JumpIfFalse => {
                sp -= 1;
                if stack[sp as usize] == Some(0) {
                    pc = op.a;
                }
            }
            Call => {
                let callee = op.a;
                let callee_prog = &progs[callee as usize];
                if (fp as usize) < f_def.len() {
                    f_def[fp as usize] = def;
                    f_pc[fp as usize] = pc;
                    f_base[fp as usize] = base;
                } else {
                    f_def.push(def);
                    f_pc.push(pc);
                    f_base.push(base);
                }
                fp += 1;
                def = callee;
                pc = 0;
                base = sp - op.b;
                for _ in op.b..callee_prog.slots {
                    put(&mut stack, sp, Some(0));
                    sp += 1;
                }
                ops = &callee_prog.ops;
            }
            Ret => {
                sp -= 1;
                let v = stack[sp as usize];
                if fp == 0 {
                    return v;
                }
                sp = base;
                fp -= 1;
                def = f_def[fp as usize];
                pc = f_pc[fp as usize];
                base = f_base[fp as usize];
                ops = &progs[def as usize].ops;
                put(&mut stack, sp, v);
                sp += 1;
            }
            PushNull => {
                put(&mut stack, sp, None);
                sp += 1;
            }
            Not => {
                let t = (sp - 1) as usize;
                stack[t] = Some(if stack[t] == Some(1) { 0 } else { 1 });
            }
            NonNull => {
                let t = (sp - 1) as usize;
                stack[t] = Some(i64::from(stack[t].is_some()));
            }
            _ => {
                sp -= 1;
                let y = stack[sp as usize];
                sp -= 1;
                let x = stack[sp as usize];
                put(&mut stack, sp, arith(op.code, x, y));
                sp += 1;
            }
        }
    }
}

fn op(code: Code, a: i64, b: i64) -> Op {
    Op { code, a, b }
}

fn programs() -> Vec<Prog> {
    let fib = vec![
        op(Load, 0, 0), op(PushInt, 1, 0), op(Le, 0, 0), op(JumpIfFalse, 6, 0),
        op(Load, 0, 0), op(Ret, 0, 0),
        op(Load, 0, 0), op(PushInt, 1, 0), op(Sub, 0, 0), op(Call, 0, 1),
        op(Load, 0, 0), op(PushInt, 2, 0), op(Sub, 0, 0), op(Call, 0, 1),
        op(Add, 0, 0), op(Ret, 0, 0),
    ];
    let mix = vec![
        op(Load, 1, 0), op(Store, 2, 0), op(PushInt, 0, 0), op(Store, 3, 0),
        op(Load, 3, 0), op(Load, 0, 0), op(Lt, 0, 0), op(JumpIfFalse, 23, 0),
        op(Load, 2, 0), op(Load, 3, 0), op(BitXor, 0, 0),
        op(Load, 3, 0), op(PushInt, 7, 0), op(BitAnd, 0, 0),
        op(Add, 0, 0), op(PushInt, 16777215, 0), op(BitAnd, 0, 0), op(Store, 2, 0),
        op(Load, 3, 0), op(PushInt, 1, 0), op(Add, 0, 0), op(Store, 3, 0), op(Jump, 4, 0),
        op(Load, 2, 0), op(Ret, 0, 0),
    ];
    let collatz = vec![
        op(Load, 0, 0), op(Store, 1, 0), op(PushInt, 1, 0), op(Store, 2, 0),
        op(PushInt, 0, 0), op(Store, 3, 0),
        op(Load, 3, 0), op(PushInt, 10000000, 0), op(Lt, 0, 0), op(JumpIfFalse, 41, 0),
        op(Load, 1, 0), op(PushInt, 1, 0), op(Eq, 0, 0), op(JumpIfFalse, 15, 0), op(Jump, 41, 0),
        op(Load, 1, 0), op(PushInt, 2, 0), op(Rem, 0, 0), op(PushInt, 0, 0), op(Eq, 0, 0),
        op(JumpIfFalse, 26, 0),
        op(Load, 1, 0), op(PushInt, 2, 0), op(Div, 0, 0), op(Store, 1, 0), op(Jump, 32, 0),
        op(PushInt, 3, 0), op(Load, 1, 0), op(Mul, 0, 0), op(PushInt, 1, 0), op(Add, 0, 0),
        op(Store, 1, 0),
        op(Load, 2, 0), op(PushInt, 1, 0), op(Add, 0, 0), op(Store, 2, 0),
        op(Load, 3, 0), op(PushInt, 1, 0), op(Add, 0, 0), op(Store, 3, 0), op(Jump, 6, 0),
        op(Load, 2, 0), op(Ret, 0, 0),
    ];
    let longest = vec![
        op(PushInt, 0, 0), op(Store, 2, 0), op(PushInt, 0, 0), op(Store, 3, 0),
        op(Load, 0, 0), op(Store, 4, 0),
        op(Load, 4, 0), op(Load, 1, 0), op(Lt, 0, 0), op(JumpIfFalse, 26, 0),
        op(Load, 4, 0), op(Call, 2, 1), op(Store, 5, 0),
        op(Load, 5, 0), op(Load, 2, 0), op(Gt, 0, 0), op(JumpIfFalse, 21, 0),
        op(Load, 5, 0), op(Store, 2, 0), op(Load, 4, 0), op(Store, 3, 0),
        op(Load, 4, 0), op(PushInt, 1, 0), op(Add, 0, 0), op(Store, 4, 0), op(Jump, 6, 0),
        op(Load, 3, 0), op(PushInt, 1000, 0), op(Mul, 0, 0), op(Load, 2, 0), op(Add, 0, 0),
        op(Ret, 0, 0),
    ];
    vec![
        Prog { ops: fib, slots: 1, params: 1 },
        Prog { ops: mix, slots: 4, params: 2 },
        Prog { ops: collatz, slots: 4, params: 1 },
        Prog { ops: longest, slots: 6, params: 2 },
    ]
}

fn interp_fib(progs: &[Prog], salt: i64) -> i64 {
    run(progs, 0, &[black_box(20)]).unwrap_or(0) + salt
}

fn interp_mix(progs: &[Prog], salt: i64) -> i64 {
    run(progs, 1, &[black_box(50000), salt]).unwrap_or(0)
}

fn interp_collatz(progs: &[Prog], salt: i64) -> i64 {
    run(progs, 3, &[black_box(1), black_box(300)]).unwrap_or(0) + salt
}

/// `--n N`: how many ops the timed loop runs (the harness calibrates it per lane).
fn arg_n(dflt: i64) -> i64 {
    let args: Vec<String> = std::env::args().collect();
    let mut n = dflt;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--n" && i + 1 < args.len() {
            n = args[i + 1].parse().unwrap_or(dflt);
            i += 1;
        }
        i += 1;
    }
    n.max(2)
}

/// One measured routine, in the row format every lane prints (bench/README.md).
fn row(name: &str, iters: i64, us: i64, items: i64, result: i64) {
    let ns = us * 1000 / iters;
    let per = (us * 1000) as f64 / (iters * items) as f64;
    println!("{name}\t{iters}\t{us}\t{ns}\t{items}\t{per:.3}\t{result:x}");
}

fn main() {
    let n = arg_n(20);
    let progs = programs();
    println!("routine\titers\tus\tns_op\tpx\tns_px\thash");
    let mut sink: i64 = 0;
    let t0 = Instant::now();
    for r in 0..n {
        sink = sink.wrapping_add(interp_fib(&progs, black_box(r & 1)));
    }
    row("interp_fib", n, t0.elapsed().as_micros() as i64, 21891, interp_fib(&progs, 0));
    let t0 = Instant::now();
    for r in 0..n {
        sink = sink.wrapping_add(interp_mix(&progs, black_box(r)));
    }
    row("interp_mix", n, t0.elapsed().as_micros() as i64, 50000, interp_mix(&progs, 1));
    let t0 = Instant::now();
    for r in 0..n {
        sink = sink.wrapping_add(interp_collatz(&progs, black_box(r & 1)));
    }
    row("interp_collatz", n, t0.elapsed().as_micros() as i64, 299, interp_collatz(&progs, 0));
    println!("time: sink={}", black_box(sink));
}
