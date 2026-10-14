// The shared adapters also serve the raw concurrent/RSS runner, which uses additional items.
#[allow(dead_code)]
mod support;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use std::time::Duration;
use support::*;

fn suite<B: Sequential>(c: &mut Criterion, name: &str, dataset: &str, values: &[u32]) {
    let mut g = c.benchmark_group(format!("external/{dataset}/{name}"));
    g.sample_size(10)
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(500));
    g.bench_function("build", |b| b.iter(|| black_box(build::<B>(values, false))));
    let raw = build::<B>(values, false);
    g.bench_function("optimize", |b| {
        b.iter_batched(
            || raw.clone(),
            |mut x| {
                x.optimize();
                x
            },
            BatchSize::SmallInput,
        )
    });
    let optimized = build::<B>(values, true);
    for hits in [0, 50, 100] {
        let p = probes(values, hits, 1_000_000);
        g.bench_function(format!("contains/{hits}"), |b| {
            b.iter(|| {
                for &x in &p {
                    black_box(optimized.contains(x));
                }
            })
        });
    }
    for (label, victims) in [
        (
            "hit",
            values
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .take(10_000)
                .collect::<Vec<_>>(),
        ),
        ("miss", probes(values, 0, 10_000)),
    ] {
        g.bench_function(format!("remove/{label}"), |b| {
            b.iter_batched(
                || optimized.clone(),
                |mut x| {
                    for &v in &victims {
                        black_box(x.remove(v));
                    }
                    x
                },
                BatchSize::SmallInput,
            )
        });
    }
    for overlap in [0, 50, 100] {
        for skew in [1, 8] {
            let (a, b) = pair(values, overlap, skew);
            let a = build::<B>(&a, true);
            let b = build::<B>(&b, true);
            g.bench_function(format!("and/{overlap}/{skew}"), |it| {
                it.iter(|| black_box(a.and(&b)))
            });
            g.bench_function(format!("or/{overlap}/{skew}"), |it| {
                it.iter(|| black_box(a.or(&b)))
            });
        }
    }
    g.finish();
}
fn all(c: &mut Criterion) {
    for d in DATASETS {
        let values = data(d);
        suite::<concurrent_roaring::RoaringBitmap>(c, "ours", d, &values);
        suite::<roaring::RoaringBitmap>(c, "roaring", d, &values);
        suite::<croaring::Bitmap>(c, "croaring", d, &values);
    }
}
criterion_group!(benches, all);
criterion_main!(benches);
