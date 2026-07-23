//! Pure-read concurrency scaling after a balanced two-million-value preload outside the timer.

use concurrent_roaring::{ConcurrentRoaringBitmap, EpochRoaringBitmap, SnapshotRoaringBitmap};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fs::{self, File};
use std::hint::black_box;
use std::io::{BufWriter, Write};
use std::sync::Barrier;
use std::thread;
use std::time::Instant;

/// Large enough to leave every default shard with a substantial immutable bitmap container.
const PRELOAD_VALUES: usize = 2_000_000;
/// Matches the main scaling harness so its per-thread sampling depth stays comparable.
const OPS: usize = 2_000_000;
/// Medians suppress scheduler noise from the M5's heterogeneous cores.
const SAMPLES: usize = 5;
/// The public types' default, used so this isolates concurrency strategy rather than configuration.
const SHARDS: u32 = 64;
/// Matches the main scaling harness's pinned per-thread RNG sequence.
const SEED_BASE: u64 = 0x5CA1_AB1E;
/// A short untimed pass faults the maps into cache before the first sample.
const WARMUP_STRIDE: usize = 20;

trait ConcurrentBench: Sync {
    fn insert(&self, value: u32) -> bool;
    fn contains(&self, value: u32) -> bool;
}

impl ConcurrentBench for ConcurrentRoaringBitmap {
    fn insert(&self, value: u32) -> bool {
        ConcurrentRoaringBitmap::insert(self, value)
    }

    fn contains(&self, value: u32) -> bool {
        ConcurrentRoaringBitmap::contains(self, value)
    }
}

impl ConcurrentBench for SnapshotRoaringBitmap {
    fn insert(&self, value: u32) -> bool {
        SnapshotRoaringBitmap::insert(self, value)
    }

    fn contains(&self, value: u32) -> bool {
        SnapshotRoaringBitmap::contains(self, value)
    }
}

impl ConcurrentBench for EpochRoaringBitmap {
    fn insert(&self, value: u32) -> bool {
        EpochRoaringBitmap::insert(self, value)
    }

    fn contains(&self, value: u32) -> bool {
        EpochRoaringBitmap::contains(self, value)
    }
}

fn run<B: ConcurrentBench>(bench: &B, data: &[u32], threads: usize) -> (f64, f64) {
    let barrier = Barrier::new(threads + 1);
    let barrier = &barrier;
    let seconds = thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|thread_index| {
                scope.spawn(move || {
                    let mut rng = StdRng::seed_from_u64(SEED_BASE ^ thread_index as u64);
                    barrier.wait();
                    for _ in 0..OPS {
                        let index = rng.random_range(0..data.len());
                        black_box(bench.contains(data[index]));
                    }
                })
            })
            .collect();
        barrier.wait();
        let start = Instant::now();
        for handle in handles {
            handle.join().unwrap();
        }
        start.elapsed().as_secs_f64()
    });
    let mops = OPS as f64 * threads as f64 / seconds / 1e6;
    (seconds, mops)
}

fn populate<B: ConcurrentBench>(bench: &B, data: &[u32]) {
    for &value in data {
        assert!(bench.insert(value));
    }
}

fn main() {
    let max_threads = thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1);
    let thread_counts: Vec<usize> = [1, 2, 4, 8, 16]
        .into_iter()
        .filter(|threads| *threads <= max_threads)
        .collect();

    // Each key owns a dense low-value range, keeping all 64 shards equally populated while every
    // generated u32 remains distinct.
    let data: Vec<u32> = (0..PRELOAD_VALUES as u32)
        .map(|index| ((index % SHARDS) << 16) | (index / SHARDS))
        .collect();

    let sharded = ConcurrentRoaringBitmap::new();
    let snapshot = SnapshotRoaringBitmap::new();
    let epoch = EpochRoaringBitmap::new();
    populate(&sharded, &data);
    populate(&snapshot, &data);
    populate(&epoch, &data);

    for index in (0..data.len()).step_by(WARMUP_STRIDE) {
        black_box(sharded.contains(data[index]));
        black_box(snapshot.contains(data[index]));
        black_box(epoch.contains(data[index]));
    }

    fs::create_dir_all("bench-results").unwrap();
    let file = File::create("bench-results/read100-preloaded-2m.csv").unwrap();
    let mut output = BufWriter::new(file);
    writeln!(
        output,
        "sample,structure,workload,preloaded,threads,total_ops,seconds,mops"
    )
    .unwrap();
    println!(
        "{:<6} {:<10} {:>7} {:>12} {:>9} {:>9}",
        "sample", "structure", "threads", "total_ops", "seconds", "mops"
    );

    for sample in 1..=SAMPLES {
        for &threads in &thread_counts {
            for (structure, seconds, mops) in [
                {
                    let (seconds, mops) = run(&sharded, &data, threads);
                    ("sharded", seconds, mops)
                },
                {
                    let (seconds, mops) = run(&snapshot, &data, threads);
                    ("snapshot", seconds, mops)
                },
                {
                    let (seconds, mops) = run(&epoch, &data, threads);
                    ("epoch", seconds, mops)
                },
            ] {
                let total_ops = OPS * threads;
                println!(
                    "{sample:<6} {structure:<10} {threads:>7} {total_ops:>12} {seconds:>9.3} {mops:>9.3}"
                );
                writeln!(
                    output,
                    "{sample},{structure},read100,{PRELOAD_VALUES},{threads},{total_ops},{seconds:.6},{mops:.6}"
                )
                .unwrap();
            }
        }
    }
}
