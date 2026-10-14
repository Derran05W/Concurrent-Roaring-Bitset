# Comparing current Roaring implementations

This experiment compares the Rust-facing APIs of `concurrent_roaring`, `roaring` 0.11.5, and `croaring` 2.8.0 (bundling CRoaring 5.2.2). Both published implementations support run compression; every optimized comparison enables it on every Roaring candidate. Historical July results against `roaring` 0.10.12 remain in `commit.md` and are not the current baseline.

The [results table](external-results.md) and [complete per-cell summary](benchmark-results/summary.csv) are generated from individual trials. `HashSet`, `BTreeSet`, and `DashSet` are context controls, not additional Roaring implementations. No result establishes an unqualified “fastest bitmap” claim.

## Machine and build

- Apple M5 MacBook Pro: 10 cores (4 performance / 6 efficiency), 24 GB RAM; macOS 26.6.2.
- Rust 1.97.0, aarch64-apple-darwin; Apple clang 21.0.0 for the C dependency.
- Rust release profile: fat LTO, one codegen unit. Default dependency features and C build flags; no CPU features deliberately disabled on a competitor. The FFI boundary is part of the `croaring` Rust API cost.
- Benchmarks run sequentially, without another benchmark or compiler job running alongside them. Thread placement is managed by macOS; these results include heterogeneous-core scheduling and are not a homogeneous eight-core scaling result.
- Each raw-run metadata JSON records the binary SHA-256, source hashes, lockfile, parent Git revision, build environment flags, date, and invocation. The Git parent can precede the benchmark source changes; use the recorded source hashes and committed source for reproduction.

## What changed in the bitmap

`contains` recognizes a consecutive span of high 16-bit keys. Since keys are sorted and unique, `last - first + 1 == number_of_keys` proves that every key in that span exists. A bounds-checked offset can then select its container directly. Gapped key vectors still use binary search. The span is computed from existing metadata, so the optimization adds no per-bitmap or per-value storage and needs no invalidation on mutation.

This benefits dense key spaces, including sparse values spread across every high key. The seeded million-draw sparse dataset fills all 65,536 high keys even though it contains only 999,872 of the possible `u32` values. One missing interior high key disables direct indexing for that map; this optimization is deliberately conditional, not a general constant-time lookup guarantee. It also adds a span check to gapped maps. The comparison archives retain exploratory measurements of the original implementation and the first speculative-index variant; they are not used as the headline external comparison. Exhaustive key-boundary tests cover empty maps, arbitrary starting keys, gaps, all 65,536 high keys, deletion of an interior key, restoration, and optimized containers. Existing differential property tests continue to cover insertion, removal, lookup, and set operations.

## Sequential protocol — Baseline B

`examples/compare.rs` produces raw trials; `benches/competitors.rs` provides the Criterion counterpart. The driver randomizes candidate order within each of five independent process trials. Datasets reuse the pinned project seeds: one million dense values, one million random `u32` draws, one thousand clusters of one thousand consecutive values, and two million values balanced over 64 keys. Duplicate draws are retained in construction inputs and excluded from final set cardinality: dense 1,000,000; sparse 999,872; clustered 999,086; balanced 2,000,000.

All candidates build with per-value insertion. CRoaring uses `add_checked` and `remove_checked`, preserving Boolean success semantics. Construction, optimization, probe generation, warmup, and result verification are outside lookup timing. Each lookup cell processes four million queries: four repetitions of the same one-million-query trace, with exactly 0%, 50%, or 100% hits. Uniform misses are rejected against the full set before timing. The trace-only row measures traversal/black-box overhead; it is reported, not subtracted. In the primary 50%-hit cells it reaches 4.35–4.47 billion queries/s, so traversal accounts for roughly 1.6–6.9% of our measured lookup time. These are warm in-memory lookups.

Raw build timing excludes destruction of the finished map. Raw set-operation timing includes creating and dropping a fresh result. Removal uses a fresh clone outside timing and tests 10,000 distinct present values or 10,000 absent values. Optimize timing excludes cloning the unoptimized input. All candidates use the same lifecycle within a cell. Criterion's default build lifecycle can include destruction, so its build times should not be substituted into the raw build table.

Set operations use independently constructed operands with 0%, 50%, or 100% overlap relative to the smaller operand. Cardinality ratios are 1:1 and 8:1. Values are drawn from sorted slices of each dataset, so these are range-correlated operands. Equal operands are a diagnostic case, not the headline result. Every result is checked against the reference set using equal cardinality plus membership of every expected value; our structural invariants are also checked.

## Concurrent protocol — Baseline C

Compare the three production strategies with two external wrappers using exactly the same 64-shard mapping, `parking_lot::RwLock`, and 128-byte shard padding. `DashSet` uses its native default hashing/sharding and is a separate concurrent-set control. The comparison covers point operations; it makes no claim of globally atomic cross-shard snapshots or set operations.

All maps are preloaded and optimized before timing. Each worker receives a precomputed trace with randomly arranged read/write slots in blocks of 200. CSV names encode the number of reads per block: `read200` is 100% reads, `read199` is 99.5%, `read190` is 95%, and `read100` is 50%. Pure reads contain four million total operations; mixed cells contain 160,000 total operations, held constant across 1, 2, 4, and 8 threads and all implementations.

Exactly half the read operations hit stable values; the other half are verified misses. Successful writes alternate remove/insert on values owned by one worker, restoring the preload after each trace. This maintains constant occupancy and a deterministic final state. Duplicate insert cells separately measure no-op writes. Hot cells route reads, misses, and writes to shard zero; uniform cells use the dataset's natural shard distribution (dense data occupies fewer than 64 shards). Writers can contend on a shard even though they own different values.

There is an untimed warmup trace before each measurement. The main timer starts before releasing the start barrier and ends after all workers join. Every trial checks successful mutation counts, read-hit counts, final cardinality, and restored mutation targets; full membership is checked after the matrix. This tests stable-read membership throughput during updates, not reads that race with writes to the same value. Separate stress tests exercise overlapping activity.

Very short mixed cells include a material fraction of barrier/join overhead. Treat those as diagnostics and inspect their raw duration and trace-control row before using them in a claim. The primary resume candidate is the independently measured sequential lookup result.

## Memory

The memory runner holds fully constructed optimized maps alive while a parent samples process RSS, then obtains peak RSS with macOS `/usr/bin/time -l`. Each candidate runs in a fresh process. A matching empty-map process generates and releases the same input dataset; both raw RSS values and their difference are retained. The `copies` column records the number of live maps, and cardinality is recorded per map.

RSS includes allocator retention, library/runtime code, input construction effects, and concurrent wrappers. Epoch rows can include retired snapshots awaiting reclamation; the memory runner does not force a separate garbage-collection cycle. Peak RSS includes startup and construction transients. Neither measure is serialized size or exact live heap bytes. Differences near the variation between trials cannot support a memory-saving claim. A lookup speedup does not imply a memory reduction.

## Reproduce

Requires stable Rust, a C compiler for CRoaring, Python 3, and Matplotlib for plots. RSS collection below uses macOS tools.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --release --test external --test concurrent_stress --example compare
cargo build --release --example compare
python3 scripts/compare.py seq --output bench-results/external --samples 5
python3 scripts/compare.py concurrent --output bench-results/external --samples 5
python3 scripts/compare.py memory --output bench-results/external --samples 5 --copies 1 --candidates ours roaring croaring hashset btreeset sharded snapshot epoch sharded_roaring sharded_croaring dashset
cargo bench --bench competitors -- --quick
cargo bench --bench sequential -- tax/ --quick
MPLCONFIGDIR=/tmp/roaring-matplotlib python3 scripts/report_comparison.py bench-results/external
```

The raw suite supplies the five-trial headline measurements; the Criterion quick run checks the same operations through a second timing harness. Remove `--quick` for longer Criterion sampling. Memory sampling needs permission to read the benchmark process with `ps` and `time` on macOS. Larger memory differences can be investigated with `--copies 8`; compare equal copy counts.

## Sources

- [`roaring` API](https://docs.rs/roaring/0.11.5/roaring/bitmap/struct.RoaringBitmap.html), including `optimize` and run compression.
- [`croaring` API](https://docs.rs/croaring/latest/croaring/bitmap/struct.Bitmap.html), including checked mutations and `run_optimize`.
- [CRoaring thread safety](https://github.com/RoaringBitmap/CRoaring#thread-safety): mutable concurrent access requires synchronization.
- [`DashSet` API](https://docs.rs/dashmap/latest/dashmap/struct.DashSet.html).

## Findings and tradeoffs

Across five fresh-process trials, optimized 50%-hit sequential lookup reached 291.38, 149.12, 69.47, and 304.76 million operations/s on dense, sparse, clustered, and balanced inputs respectively. The speedups over `roaring` were 2.62×, 6.79×, 1.23×, and 3.61×; over `croaring` they were 1.94×, 7.69×, 1.71×, and 3.04×. Each candidate's observed trial range is available in the summary CSV. These are ratios of medians, not confidence bounds. Summaries recompute throughput from operation counts and elapsed seconds to avoid rounding the small optimization-operation rates.

The sequential lookup improvement does not imply the same improvement inside a shard: low-bit sharding creates gaps between the high keys held by each shard, so many shards still use binary search. Baseline C compares mutable concurrent point-operation contracts; it does not compare against sharing an immutable published bitmap with unsynchronized reads.

### Operations where ours is more than twice as slow (Baseline B)

The complete matrix is retained, including these unfavorable results. These explanations follow the implementation paths; they are not CPU-profile attribution.

- **Optimization:** dense inputs take about 169× longer than `roaring` and 161× longer than `croaring`; balanced inputs about 86× and 75× longer. Our bitmap-to-run conversion visits every set bit after separately counting runs. Long dense runs therefore pay work proportional to cardinality, even though the output contains very few runs. Clustered optimization is also slower (3.0× / 4.5×). Preprocessing is explicitly outside lookup timing and remains a cost users must pay.
- **Disjoint intersection:** sparse, range-separated operands take 127–169× longer than CRoaring, depending on cardinality ratio; clustered operands take about 3.4× longer. Our top-level intersection advances one key at a time, while CRoaring's `roaring_bitmap_and` uses `ra_advance_until` to skip unmatched key ranges. These synthetic sorted-slice operands strongly favor that skip algorithm. They are disclosed as range-correlated workloads, not general random-overlap behavior.
- **Sparse construction and successful removal:** CRoaring is about 2.3× and 2.7× faster respectively. Our insertion shifts a vector of enum containers when a new high key appears; deletion also shifts array contents and can remove container entries. The C implementation has different metadata layout and tuned container kernels. Dedicated profiling would be needed to apportion the gap between those costs.

### Suggested resume bullet

> Optimized a Rust Roaring bitmap to sustain **149M warm membership queries/s**, **6.8× faster than roaring 0.11.5 and 7.7× faster than croaring 2.8.0** on one million seeded random `u32` inputs with 50% hit queries; validated against both libraries with differential tests and five independent trials on Apple M5.

Link this bullet to the methodology and raw results. It describes sequential membership on a defined workload, not universal superiority or a memory-saving result.

### Concurrent interpretation (Baseline C)

At eight threads, snapshot pure-read throughput reaches 1,059.50 Mops/s on balanced data, versus 95.16 and 98.77 Mops/s for the identically sharded published wrappers. This is a comparison of different synchronization strategies under a mutable API contract; it does not establish a win over immutable, unsynchronized library reads. Sharded payload comparisons are much closer: on clustered pure reads, our lock-based wrapper is slightly slower than the CRoaring wrapper.

Successful writes change the ranking. On sparse data at eight threads, the 95%-read mix reaches 91.59 Mops/s for sharded, 1.89 for snapshot, and 2.42 for epoch. The clone-per-write cost is substantial. No-op insert results are reported separately and cannot stand in for successful updates.

**T2 is missed in this new successful-write matrix:** none of the three production structures reaches 4× its own one-thread throughput at eight threads on any dataset. Only the two sparse RCU series are monotonic. The original phase results used a different workload and remain historical.

| Dataset | Strategy | 1t Mops/s | 2t | 4t | 8t | 8t / 1t |
|---|---|---:|---:|---:|---:|---:|
| dense | sharded | 260.50 | 66.28 | 74.07 | 63.91 | 0.25× |
| dense | snapshot | 32.45 | 20.19 | 26.12 | 26.23 | 0.81× |
| dense | epoch | 113.67 | 66.28 | 130.52 | 115.72 | 1.02× |
| sparse | sharded | 39.71 | 62.46 | 97.40 | 91.59 | 2.31× |
| sparse | snapshot | 0.71 | 0.90 | 1.40 | 1.89 | 2.65× |
| sparse | epoch | 0.74 | 0.85 | 1.52 | 2.42 | 3.26× |
| clustered | sharded | 123.85 | 82.48 | 98.80 | 94.34 | 0.76× |
| clustered | snapshot | 22.58 | 18.08 | 26.64 | 25.49 | 1.13× |
| clustered | epoch | 38.05 | 37.95 | 54.99 | 57.45 | 1.51× |
| balanced | sharded | 329.84 | 77.25 | 92.30 | 89.17 | 0.27× |
| balanced | snapshot | 35.46 | 20.92 | 31.84 | 32.32 | 0.91× |
| balanced | epoch | 128.90 | 65.40 | 141.25 | 129.65 | 1.01× |

This matrix holds total work fixed. The large one-to-two-thread drop on dense/balanced sharded maps is consistent with shared lock-word cache traffic becoming expensive relative to a very cheap uncontended lookup. RCU writes additionally serialize and clone; heterogeneous cores and barrier/join overhead affect short cells. These are possible mechanisms, not measured causal attribution. The raw durations and trace controls are retained.

### Criterion cross-check and Baseline A

The [Criterion archive](benchmark-results/criterion.json) contains all 228 external cells and eight
single-thread tax cells, with a [compact table](benchmark-results/criterion-summary.csv). The quick
run corroborates the lookup win on all four datasets. It uses two quick samples per cell, so its
within-run confidence intervals should not substitute for the five independent raw trials.

The original clustered tax harness uses unoptimized preloads. Median time per million reads is
18.810 ms sequential, 16.668 ms sharded, 17.626 ms snapshot, and 18.661 ms epoch: read taxes of
−11.39%, −6.29%, and −0.79%. Construction takes 9.844, 8.999, 533.665, and 548.356 ms respectively.
RCU construction misses the 10% tax goal because each successful write clones its shard. These
are Baseline A comparisons with our own sequential type, not external-library speedups.

### Memory interpretation

All eleven candidates were measured on all four datasets in five fresh-process trials, with one
live optimized map and a paired empty-process control. This does not establish a broad memory win
against the published Roaring libraries. Sparse median total RSS is 12.16 MiB for ours, 13.30 MiB
for `roaring`, and 12.48 MiB for `croaring`; the dense and balanced comparisons include cases where
`roaring` retains less process memory. Small differences must be read alongside baseline variation.

On sparse data, our sharded, snapshot, and epoch variants retain 13.48, 23.56, and 95.58 MiB
respectively. The latter includes allocator retention and potentially unreclaimed retired snapshots
after construction. These are process measurements of the actual preload/optimize path, not a
claim about minimum attainable live bitmap storage. The resume bullet therefore claims lookup
speed only.

### Validation and artifact coverage

The final `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass
(38 tests). Release-mode external tests, concurrent stress tests, and trace-contract tests pass
(13 tests, including the benchmark example's test). The benchmark gates additionally validate
membership, cardinality, hit counts, and mutation counts in the timed-run harness.

Archived coverage: 1,600 sequential trial rows, 7,840 concurrent rows including trace controls,
440 memory rows including empty controls, and 236 Criterion cells. The scored source hashes match
across all three raw suites. Graphs show medians and observed trial ranges; the narrower Criterion
quick intervals are not used as cross-trial uncertainty estimates.
