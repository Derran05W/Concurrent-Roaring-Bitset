use criterion::{criterion_group, criterion_main, Criterion};

fn bench(c: &mut Criterion) {
    c.bench_function("placeholder", |b| b.iter(|| std::hint::black_box(1 + 1)));
}

criterion_group!(benches, bench);
criterion_main!(benches);
