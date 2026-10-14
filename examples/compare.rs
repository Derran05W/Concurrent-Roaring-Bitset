// Shared with Criterion, where some adapters are intentionally unused.
#[allow(dead_code)]
#[path = "../benches/support/mod.rs"]
mod support;
use concurrent_roaring::{
    ConcurrentRoaringBitmap, EpochRoaringBitmap, RoaringBitmap, SnapshotRoaringBitmap,
};
use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};
use std::{
    collections::{BTreeSet, HashSet},
    hint::black_box,
    io::{self, Write},
    sync::Barrier,
    thread,
    time::Instant,
};
use support::*;

// Keeping the CSV fields explicit makes every measurement site auditable.
#[allow(clippy::too_many_arguments)]
fn row(
    name: &str,
    dataset: &str,
    operation: &str,
    optimized: bool,
    threads: usize,
    sample: usize,
    counts: (usize, usize),
    seconds: f64,
) {
    let (ops, changed) = counts;
    println!("{name},{dataset},{operation},{optimized},{threads},{sample},{ops},{changed},{seconds:.9},{:.6}", ops as f64 / seconds / 1e6);
}
fn timed<F: FnMut()>(mut f: F, iterations: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    start.elapsed().as_secs_f64()
}
fn sequential<B: Sequential>(name: &str, d: &str, sample: usize, reads_only: bool) {
    let values = data(d);
    let oracle: roaring::RoaringBitmap = values.iter().copied().collect();
    let start = Instant::now();
    let raw = build::<B>(&values, false);
    let seconds = start.elapsed().as_secs_f64();
    verify(&raw, &oracle);
    row(
        name,
        d,
        "build",
        false,
        1,
        sample,
        (values.len(), oracle.len() as usize),
        seconds,
    );
    let control = name == "hashset" || name == "btreeset";
    for optimized in [false, true] {
        if control && optimized {
            continue;
        }
        let mut b = raw.clone();
        if optimized {
            let seconds = timed(|| b.optimize(), 1);
            row(name, d, "optimize", true, 1, sample, (1, 0), seconds);
        }
        for hits in [0, 50, 100] {
            let p = probes(&values, hits, 1_000_000);
            let observed = p.iter().filter(|&&x| b.contains(x)).count();
            assert_eq!(observed, p.len() * hits / 100);
            // An untimed pass warms the same map and trace for each candidate.
            for &x in &p {
                black_box(b.contains(x));
            }
            let seconds = timed(
                || {
                    for &x in &p {
                        black_box(b.contains(x));
                    }
                },
                4,
            );
            row(
                name,
                d,
                &format!("contains_{hits}"),
                optimized,
                1,
                sample,
                (p.len() * 4, 0),
                seconds,
            );
            if name == "ours" {
                let seconds = timed(
                    || {
                        for &x in &p {
                            black_box(x);
                        }
                    },
                    4,
                );
                row(
                    "trace",
                    d,
                    &format!("contains_{hits}"),
                    optimized,
                    1,
                    sample,
                    (p.len() * 4, 0),
                    seconds,
                );
            }
        }
        if !optimized || control || reads_only {
            continue;
        }
        for hits in [false, true] {
            let victims = if hits {
                oracle.iter().take(10_000).collect()
            } else {
                probes(&values, 0, 10_000)
            };
            let mut copy = b.clone();
            let mut changed = 0;
            let seconds = timed(
                || {
                    for &x in &victims {
                        changed += copy.remove(x) as usize;
                    }
                },
                1,
            );
            assert_eq!(changed, if hits { victims.len() } else { 0 });
            let mut expected = oracle.clone();
            for &v in &victims {
                expected.remove(v);
            }
            verify(&copy, &expected);
            row(
                name,
                d,
                if hits { "remove_hit" } else { "remove_miss" },
                true,
                1,
                sample,
                (victims.len(), changed),
                seconds,
            );
        }
        for overlap in [0, 50, 100] {
            for skew in [1, 8] {
                let (av, bv) = pair(&values, overlap, skew);
                let a = build::<B>(&av, true);
                let b = build::<B>(&bv, true);
                let ra: roaring::RoaringBitmap = av.into_iter().collect();
                let rb: roaring::RoaringBitmap = bv.into_iter().collect();
                for union in [false, true] {
                    let expected = if union { &ra | &rb } else { &ra & &rb };
                    let result = if union { a.or(&b) } else { a.and(&b) };
                    verify(&result, &expected);
                    let mut op = || {
                        black_box(if union { a.or(&b) } else { a.and(&b) });
                    };
                    let pilot = timed(&mut op, 1);
                    // Amortize timer overhead; immutable operations repeat without changing inputs.
                    let iterations = (0.03 / pilot).ceil().clamp(1.0, 100_000.0) as usize;
                    let seconds = timed(op, iterations);
                    row(
                        name,
                        d,
                        &format!("{}_{overlap}_skew{skew}", if union { "or" } else { "and" }),
                        true,
                        1,
                        sample,
                        (iterations, 0),
                        seconds,
                    );
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Op {
    Read(u32),
    Insert(u32),
    Remove(u32),
}
fn traces(
    values: &[u32],
    threads: usize,
    reads: usize,
    hot: bool,
    noop: bool,
    total: usize,
) -> Vec<Vec<Op>> {
    let mut unique = values.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if hot {
        unique.retain(|x| ((x >> 16) as usize & (SHARDS - 1)) == 0);
    }
    assert!(unique.len() > threads * 2);
    let oracle: roaring::RoaringBitmap = values.iter().copied().collect();
    (0..threads)
        .map(|t| {
            let mut rng = StdRng::seed_from_u64(SEED ^ t as u64);
            let mut result = Vec::with_capacity(total / threads);
            let mut writes = 0;
            let mut target = 0;
            let mut seen_reads = 0;
            let mut read_slots = [false; 200];
            read_slots[..reads].fill(true);
            for i in 0..total / threads {
                // A 200-op period represents the 99.5% read mix exactly, with even write counts per worker.
                if i % 200 == 0 {
                    read_slots.shuffle(&mut rng);
                }
                if read_slots[i % 200] {
                    // Stable odd-indexed values cannot be removed by any writer; misses are checked.
                    let x = if seen_reads % 2 == 0 {
                        unique[(rng.random_range(0..unique.len() / 2) * 2) + 1]
                    } else {
                        loop {
                            let mut x = rng.random::<u32>();
                            // Hot misses must contend on the same shard as hot hits and writes.
                            if hot {
                                x &= !(((SHARDS - 1) as u32) << 16);
                            }
                            if !oracle.contains(x) {
                                break x;
                            }
                        }
                    };
                    result.push(Op::Read(x));
                    seen_reads += 1;
                } else {
                    if writes % 2 == 0 {
                        let slots = unique.len() / (threads * 2);
                        target = unique[(rng.random_range(0..slots) * threads + t) * 2];
                    }
                    result.push(if noop || writes % 2 == 1 {
                        Op::Insert(target)
                    } else {
                        Op::Remove(target)
                    });
                    writes += 1;
                }
            }
            assert_eq!(
                writes % 2,
                0,
                "successful mutations must restore the preload"
            );
            result
        })
        .collect()
}
#[derive(Default)]
struct TraceControl;
impl Concurrent for TraceControl {
    fn contains(&self, x: u32) -> bool {
        black_box(x) & 1 == 0
    }
    fn insert(&self, x: u32) -> bool {
        black_box(x);
        true
    }
    fn remove(&self, x: u32) -> bool {
        black_box(x);
        true
    }
    fn len(&self) -> u64 {
        0
    }
    fn optimize(&self) {}
}
fn run<B: Concurrent>(b: &B, traces: &[Vec<Op>]) -> (f64, usize, usize) {
    let ready = Barrier::new(traces.len() + 1);
    let go = Barrier::new(traces.len() + 1);
    thread::scope(|scope| {
        let handles: Vec<_> = traces
            .iter()
            .map(|ops| {
                let ready = &ready;
                let go = &go;
                scope.spawn(move || {
                    ready.wait();
                    go.wait();
                    let start = Instant::now();
                    let mut changed = 0;
                    let mut hits = 0;
                    for &op in ops {
                        match op {
                            Op::Read(x) => {
                                hits += black_box(b.contains(x)) as usize;
                            }
                            Op::Insert(x) => changed += b.insert(x) as usize,
                            Op::Remove(x) => changed += b.remove(x) as usize,
                        }
                    }
                    (start.elapsed().as_secs_f64(), changed, hits)
                })
            })
            .collect();
        ready.wait();
        // Start before release: no worker can execute uncounted operations.
        let start = Instant::now();
        go.wait();
        let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        (
            start.elapsed().as_secs_f64(),
            outcomes.iter().map(|o| o.1).sum(),
            outcomes.iter().map(|o| o.2).sum(),
        )
    })
}
fn concurrent<B: Concurrent>(name: &str, d: &str, samples: usize) {
    let values = data(d);
    let oracle: roaring::RoaringBitmap = values.iter().copied().collect();
    let b = B::default();
    for &x in &values {
        b.insert(x);
    }
    b.optimize();
    for hot in [false, true] {
        for reads in [200, 199, 190, 100] {
            for noop in [false, true] {
                if reads == 200 && noop {
                    continue;
                }
                for threads in [1, 2, 4, 8] {
                    let total = if reads == 200 { 4_000_000 } else { 160_000 };
                    let trace = traces(&values, threads, reads, hot, noop, total);
                    let expected_writes = if noop { 0 } else { total * (200 - reads) / 200 };
                    let expected_hits = trace
                        .iter()
                        .flatten()
                        .filter(|op| matches!(op, Op::Read(x) if oracle.contains(*x)))
                        .count();
                    let (_, changed, hits) = run(&b, &trace);
                    assert_eq!(hits, expected_hits);
                    assert_eq!(changed, expected_writes);
                    for sample in 0..samples {
                        if name == "sharded" {
                            let (seconds, _, _) = run(&TraceControl, &trace);
                            row(
                                "trace_concurrent",
                                d,
                                &format!(
                                    "read{reads}_{}_{}",
                                    if hot { "hot" } else { "uniform" },
                                    if noop { "noop" } else { "change" }
                                ),
                                true,
                                threads,
                                sample,
                                (total, 0),
                                seconds,
                            );
                        }
                        let (seconds, changed, hits) = run(&b, &trace);
                        assert_eq!(hits, expected_hits);
                        assert_eq!(changed, expected_writes);
                        assert_eq!(b.len(), oracle.len());
                        for ops in &trace {
                            for &op in ops {
                                if let Op::Insert(x) = op {
                                    assert!(b.contains(x));
                                }
                            }
                        }
                        row(
                            name,
                            d,
                            &format!(
                                "read{reads}_{}_{}",
                                if hot { "hot" } else { "uniform" },
                                if noop { "noop" } else { "change" }
                            ),
                            true,
                            threads,
                            sample,
                            (total, changed),
                            seconds,
                        );
                    }
                }
            }
        }
    }
    assert!(oracle.iter().all(|x| b.contains(x)));
}
fn memory<B: Sequential>(d: &str, copies: usize) {
    let values = data(d);
    let maps: Vec<B> = (0..copies).map(|_| build::<B>(&values, true)).collect();
    drop(values);
    println!(
        "READY {} {}",
        std::process::id(),
        maps.first().map_or(0, Sequential::len)
    );
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    black_box(maps);
}
fn memory_concurrent<B: Concurrent>(d: &str, copies: usize) {
    let values = data(d);
    let maps: Vec<B> = (0..copies)
        .map(|_| {
            let b = B::default();
            for &v in &values {
                b.insert(v);
            }
            b.optimize();
            b
        })
        .collect();
    drop(values);
    println!(
        "READY {} {}",
        std::process::id(),
        maps.first().map_or(0, Concurrent::len)
    );
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    black_box(maps);
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("seq");
    let name = args.get(2).map(String::as_str).unwrap_or("ours");
    let d = args.get(3).map(String::as_str).unwrap_or("clustered");
    let n = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(5);
    if mode != "memory" {
        println!("candidate,dataset,operation,optimized,threads,sample,ops,changed,seconds,mops");
    }
    macro_rules! seq {
        ($t:ty) => {
            if mode == "memory" {
                memory::<$t>(d, n);
            } else {
                sequential::<$t>(name, d, n, mode == "reads");
            }
        };
    }
    macro_rules! conc {
        ($t:ty) => {
            if mode == "memory" {
                memory_concurrent::<$t>(d, n);
            } else {
                concurrent::<$t>(name, d, n);
            }
        };
    }
    match name {
        "ours" => seq!(RoaringBitmap),
        "roaring" => seq!(roaring::RoaringBitmap),
        "croaring" => seq!(croaring::Bitmap),
        "hashset" => seq!(HashSet<u32>),
        "btreeset" => seq!(BTreeSet<u32>),
        "sharded" => conc!(ConcurrentRoaringBitmap),
        "snapshot" => conc!(SnapshotRoaringBitmap),
        "epoch" => conc!(EpochRoaringBitmap),
        "sharded_roaring" => conc!(ShardedRoaring),
        "sharded_croaring" => conc!(ShardedCroaring),
        "dashset" => conc!(dashmap::DashSet<u32>),
        _ => panic!("unknown candidate"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traces_have_exact_mix_and_restore_preload() {
        let values: Vec<_> = (0..65536u32).map(|i| ((i % 64) << 16) | (i / 64)).collect();
        let base: roaring::RoaringBitmap = values.iter().copied().collect();
        for threads in [1, 2, 4, 8] {
            for reads in [100, 190, 199, 200] {
                for hot in [false, true] {
                    for noop in [false, true] {
                        let trace = traces(&values, threads, reads, hot, noop, 3200);
                        let mut b = base.clone();
                        let mut read_count = 0;
                        let mut hits = 0;
                        for ops in trace {
                            for op in ops {
                                match op {
                                    Op::Read(x) => {
                                        read_count += 1;
                                        hits += b.contains(x) as usize;
                                        if hot {
                                            assert_eq!((x >> 16) & 63, 0);
                                        }
                                    }
                                    Op::Insert(x) => assert_eq!(b.insert(x), !noop),
                                    Op::Remove(x) => assert!(b.remove(x)),
                                }
                            }
                        }
                        assert_eq!(read_count, 3200 * reads / 200);
                        assert_eq!(hits, read_count / 2);
                        assert_eq!(b, base);
                    }
                }
            }
        }
    }
}
