//! Multithread scaling harness. For each (structure, workload, thread-count) cell it pre-populates
//! a fresh structure with the clustered dataset, releases N threads from a barrier to each run a
//! fixed op count under the workload's read/write mix, and records wall-clock throughput.

use concurrent_roaring::bitmap::datasets;
use concurrent_roaring::{
    ConcurrentRoaringBitmap, EpochRoaringBitmap, RoaringBitmap, SnapshotRoaringBitmap,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fs::{self, OpenOptions};
use std::hint::black_box;
use std::io::Write;
use std::path::Path;
use std::sync::Barrier;
use std::thread;
use std::time::Instant;

/// Ops each thread performs in a timed cell.
const OPS: usize = 2_000_000;
/// Per-thread RNG seed base, XORed with the thread index.
const SEED_BASE: u64 = 0x5CA1_AB1E;

/// A structure exposing the two hot-path ops the workloads drive, both through `&self` so many
/// threads share one instance. (`sequential` runs single-threaded and does not use this.)
trait ConcurrentBench: Sync {
    fn insert(&self, x: u32) -> bool;
    fn contains(&self, x: u32) -> bool;
}

impl ConcurrentBench for ConcurrentRoaringBitmap {
    fn insert(&self, x: u32) -> bool {
        ConcurrentRoaringBitmap::insert(self, x)
    }
    fn contains(&self, x: u32) -> bool {
        ConcurrentRoaringBitmap::contains(self, x)
    }
}

impl ConcurrentBench for SnapshotRoaringBitmap {
    fn insert(&self, x: u32) -> bool {
        SnapshotRoaringBitmap::insert(self, x)
    }
    fn contains(&self, x: u32) -> bool {
        SnapshotRoaringBitmap::contains(self, x)
    }
}

impl ConcurrentBench for EpochRoaringBitmap {
    fn insert(&self, x: u32) -> bool {
        EpochRoaringBitmap::insert(self, x)
    }
    fn contains(&self, x: u32) -> bool {
        EpochRoaringBitmap::contains(self, x)
    }
}

/// One thread's workload: `read_pct`% membership probes (drawn from the pre-populated dataset) and
/// the rest uniform-random inserts.
fn worker<B: ConcurrentBench>(bench: &B, data: &[u32], read_pct: u32, seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    for _ in 0..OPS {
        if rng.random_range(0..100) < read_pct {
            let idx = rng.random_range(0..data.len());
            black_box(bench.contains(data[idx]));
        } else {
            black_box(bench.insert(rng.random::<u32>()));
        }
    }
}

/// Sequential baseline: single thread, no locks, no barrier. Runs the identical op mix on a plain
/// `&mut RoaringBitmap` so its number is a true concurrency-machinery-free reference.
fn run_sequential(data: &[u32], read_pct: u32) -> (usize, f64) {
    let mut map = RoaringBitmap::new();
    for &x in data {
        map.insert(x);
    }
    let mut rng = StdRng::seed_from_u64(SEED_BASE);
    let start = Instant::now();
    for _ in 0..OPS {
        if rng.random_range(0..100) < read_pct {
            let idx = rng.random_range(0..data.len());
            black_box(map.contains(data[idx]));
        } else {
            black_box(map.insert(rng.random::<u32>()));
        }
    }
    (OPS, start.elapsed().as_secs_f64())
}

/// Run `threads` workers concurrently against one shared structure. The barrier is sized
/// `threads + 1`: the main thread waits on it too, so the moment it is released every worker is
/// already in its timed loop — the clock starts at true barrier-release, and elapsed is measured
/// after the last join.
fn run_concurrent<B: ConcurrentBench>(
    bench: &B,
    data: &[u32],
    read_pct: u32,
    threads: usize,
) -> (usize, f64) {
    let barrier = Barrier::new(threads + 1);
    let barrier = &barrier;
    let secs = thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    barrier.wait();
                    worker(bench, data, read_pct, SEED_BASE ^ t as u64);
                })
            })
            .collect();
        barrier.wait();
        let start = Instant::now();
        for h in handles {
            h.join().unwrap();
        }
        start.elapsed().as_secs_f64()
    });
    (OPS * threads, secs)
}

struct Row {
    structure: &'static str,
    workload: &'static str,
    threads: usize,
    total_ops: usize,
    seconds: f64,
    mops: f64,
}

fn write_csv(rows: &[Row]) {
    fs::create_dir_all("bench-results").expect("create bench-results dir");
    let path = "bench-results/scaling.csv";
    let had_header = Path::new(path).exists();
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open scaling.csv");
    if !had_header {
        writeln!(f, "structure,workload,threads,total_ops,seconds,mops").unwrap();
    }
    for r in rows {
        writeln!(
            f,
            "{},{},{},{},{:.6},{:.3}",
            r.structure, r.workload, r.threads, r.total_ops, r.seconds, r.mops
        )
        .unwrap();
    }
}

fn print_table(rows: &[Row]) {
    println!(
        "{:<12} {:<9} {:>7} {:>12} {:>9} {:>9}",
        "structure", "workload", "threads", "total_ops", "seconds", "mops"
    );
    for r in rows {
        println!(
            "{:<12} {:<9} {:>7} {:>12} {:>9.3} {:>9.3}",
            r.structure, r.workload, r.threads, r.total_ops, r.seconds, r.mops
        );
    }
}

fn main() {
    let max_threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    // {1,2,4,8,16} clamped to the machine's parallelism.
    let thread_counts: Vec<usize> = [1, 2, 4, 8, 16]
        .into_iter()
        .filter(|&n| n <= max_threads)
        .collect();

    let workloads: [(&str, u32); 3] = [("read95", 95), ("mixed50", 50), ("write95", 5)];
    let structures = ["sequential", "sharded", "snapshot", "epoch"];

    let data = datasets::clustered();
    let mut rows: Vec<Row> = Vec::new();

    for structure in structures {
        for &(workload, read_pct) in &workloads {
            // `sequential` is single-thread-only; every other structure runs the full sweep.
            let counts: &[usize] = if structure == "sequential" {
                &[1]
            } else {
                &thread_counts
            };
            for &threads in counts {
                let (total_ops, seconds) = match structure {
                    "sequential" => run_sequential(&data, read_pct),
                    "sharded" => {
                        let bench = ConcurrentRoaringBitmap::new();
                        for &x in &data {
                            bench.insert(x);
                        }
                        run_concurrent(&bench, &data, read_pct, threads)
                    }
                    "snapshot" => {
                        let bench = SnapshotRoaringBitmap::new();
                        for &x in &data {
                            bench.insert(x);
                        }
                        run_concurrent(&bench, &data, read_pct, threads)
                    }
                    "epoch" => {
                        let bench = EpochRoaringBitmap::new();
                        for &x in &data {
                            bench.insert(x);
                        }
                        run_concurrent(&bench, &data, read_pct, threads)
                    }
                    _ => unreachable!(),
                };
                let mops = total_ops as f64 / seconds / 1e6;
                rows.push(Row {
                    structure,
                    workload,
                    threads,
                    total_ops,
                    seconds,
                    mops,
                });
            }
        }
    }

    write_csv(&rows);
    print_table(&rows);
}
