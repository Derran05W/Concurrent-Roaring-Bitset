//! Sequential baseline benchmarks (Baseline B: ours vs the `roaring` crate; also the sequential
//! reference point for Baseline A in P7). Every group measures our `RoaringBitmap` and the
//! reference `roaring::RoaringBitmap` side by side on identical, pinned-seed inputs.

use concurrent_roaring::bitmap::datasets;
use concurrent_roaring::{ConcurrentRoaringBitmap, RoaringBitmap};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use roaring::RoaringBitmap as RefBitmap;
use std::hint::black_box;

fn build_ours(data: &[u32]) -> RoaringBitmap {
    let mut b = RoaringBitmap::new();
    for &x in data {
        b.insert(x);
    }
    b
}

fn build_ref(data: &[u32]) -> RefBitmap {
    let mut b = RefBitmap::new();
    for &x in data {
        b.insert(x);
    }
    b
}

/// build/{dense,sparse,clustered}: insert every value into a fresh structure.
fn bench_build(c: &mut Criterion) {
    for (name, data) in [
        ("dense", datasets::dense()),
        ("sparse", datasets::sparse()),
        ("clustered", datasets::clustered()),
    ] {
        let mut g = c.benchmark_group(format!("build/{name}"));
        g.bench_function("ours", |b| b.iter(|| black_box(build_ours(&data))));
        g.bench_function("roaring", |b| b.iter(|| black_box(build_ref(&data))));
        g.finish();
    }
}

/// contains/{dense,sparse,clustered}: pre-built structure, optimize()d on our side (the `roaring`
/// 0.10 crate exposes no run-optimize equivalent — see the P6 ledger note), then iterate the probe
/// stream.
fn bench_contains(c: &mut Criterion) {
    for (name, data) in [
        ("dense", datasets::dense()),
        ("sparse", datasets::sparse()),
        ("clustered", datasets::clustered()),
    ] {
        let mut ours = build_ours(&data);
        ours.optimize();
        let refb = build_ref(&data);
        let probes = datasets::probes(&data);

        let mut g = c.benchmark_group(format!("contains/{name}"));
        g.bench_function("ours", |b| {
            b.iter(|| {
                for &x in &probes {
                    black_box(ours.contains(x));
                }
            })
        });
        g.bench_function("roaring", |b| {
            b.iter(|| {
                for &x in &probes {
                    black_box(refb.contains(x));
                }
            })
        });
        g.finish();
    }
}

/// remove/clustered: a fresh clone of the pre-built structure per batch, then remove 100,000
/// values sampled from the dataset.
fn bench_remove(c: &mut Criterion) {
    let data = datasets::clustered();
    let ours = build_ours(&data);
    let refb = build_ref(&data);

    // Pinned seed for the remove sample (§ appendix seed table): 0xBADC0DE, regrouped to 4-digit
    // blocks for clippy::unusual_byte_groupings (same value).
    let mut rng = StdRng::seed_from_u64(0x0BAD_C0DE);
    let victims: Vec<u32> = (0..100_000)
        .map(|_| data[rng.random_range(0..data.len())])
        .collect();

    let mut g = c.benchmark_group("remove/clustered");
    g.bench_function("ours", |b| {
        b.iter_batched(
            || ours.clone(),
            |mut m| {
                for &x in &victims {
                    black_box(m.remove(x));
                }
                m
            },
            BatchSize::SmallInput,
        )
    });
    g.bench_function("roaring", |b| {
        b.iter_batched(
            || refb.clone(),
            |mut m| {
                for &x in &victims {
                    black_box(m.remove(x));
                }
                m
            },
            BatchSize::SmallInput,
        )
    });
    g.finish();
}

/// and/or over the three prescribed dataset pairs; both operands pre-built and (on our side)
/// optimized so Run kernels participate.
fn bench_setops(c: &mut Criterion) {
    let dense = datasets::dense();
    let sparse = datasets::sparse();
    let clustered = datasets::clustered();

    let pairs: [(&str, &[u32], &[u32]); 3] = [
        ("dense_x_sparse", &dense, &sparse),
        ("clustered_x_clustered", &clustered, &clustered),
        ("sparse_x_sparse", &sparse, &sparse),
    ];

    for (name, a, b_data) in pairs {
        let mut oa = build_ours(a);
        let mut ob = build_ours(b_data);
        oa.optimize();
        ob.optimize();
        let ra = build_ref(a);
        let rb = build_ref(b_data);

        let mut g = c.benchmark_group(format!("and/{name}"));
        g.bench_function("ours", |bch| bch.iter(|| black_box(oa.and(&ob))));
        g.bench_function("roaring", |bch| bch.iter(|| black_box(&ra & &rb)));
        g.finish();

        let mut g = c.benchmark_group(format!("or/{name}"));
        g.bench_function("ours", |bch| bch.iter(|| black_box(oa.or(&ob))));
        g.bench_function("roaring", |bch| bch.iter(|| black_box(&ra | &rb)));
        g.finish();
    }
}

/// Bench-local abstraction over the two structures (§1.4 permits a trait inside a bench file). Only
/// the build path needs normalizing: `RoaringBitmap::insert` takes `&mut self` while
/// `ConcurrentRoaringBitmap::insert` takes `&self`. The contains benches call each type's inherent
/// method directly, so `contains` is not part of the trait.
trait TaxBench: Default {
    fn insert(&mut self, x: u32) -> bool;
}

impl TaxBench for RoaringBitmap {
    fn insert(&mut self, x: u32) -> bool {
        RoaringBitmap::insert(self, x)
    }
}

impl TaxBench for ConcurrentRoaringBitmap {
    fn insert(&mut self, x: u32) -> bool {
        // &self op; the &mut here just satisfies the shared trait signature.
        ConcurrentRoaringBitmap::insert(self, x)
    }
}

fn build_tax<T: TaxBench>(data: &[u32]) -> T {
    let mut b = T::default();
    for &x in data {
        b.insert(x);
    }
    b
}

/// Baseline A (concurrency tax): sequential `RoaringBitmap` vs `ConcurrentRoaringBitmap` run
/// single-threaded on clustered build + contains. The gap is the cost of the sharding/locking
/// machinery when there is no contention.
fn bench_tax(c: &mut Criterion) {
    let data = datasets::clustered();
    let probes = datasets::probes(&data);

    let mut g = c.benchmark_group("tax/build_clustered");
    g.bench_function("sequential", |b| {
        b.iter(|| black_box(build_tax::<RoaringBitmap>(&data)))
    });
    g.bench_function("sharded", |b| {
        b.iter(|| black_box(build_tax::<ConcurrentRoaringBitmap>(&data)))
    });
    g.finish();

    let seq = build_tax::<RoaringBitmap>(&data);
    let shard = build_tax::<ConcurrentRoaringBitmap>(&data);
    let mut g = c.benchmark_group("tax/contains_clustered");
    g.bench_function("sequential", |b| {
        b.iter(|| {
            for &x in &probes {
                black_box(seq.contains(x));
            }
        })
    });
    g.bench_function("sharded", |b| {
        b.iter(|| {
            for &x in &probes {
                black_box(shard.contains(x));
            }
        })
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_build,
    bench_contains,
    bench_remove,
    bench_setops,
    bench_tax
);
criterion_main!(benches);
