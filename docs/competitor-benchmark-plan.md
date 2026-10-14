# External benchmark plan

Implementation and results: [October comparison](external-benchmarks.md). The subsequent user request also authorized production optimizations; that scope change is recorded in `commit.md`.

**Goal:** produce a reproducible, narrow resume claim about this project's `u32` Roaring bitmaps. The screenshot is a model for a *measured claim*, not a target speedup. Keep the existing historical results in `commit.md`; this is a post-P9 comparison and does not change `AGENTS.md` or the project's implementation scope.

## 1. Comparison groups

| Group | Candidates | Question answered |
|---|---|---|
| Sequential Roaring | This repo's `RoaringBitmap`; current [`roaring`](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html); current [`croaring::Bitmap`](https://docs.rs/croaring/latest/croaring/bitmap/struct.Bitmap.html) (Rust binding to CRoaring) | How fast is the bitmap implementation itself? |
| Concurrent Roaring | This repo's sharded `RwLock`, snapshot, and epoch types; a **64-shard `parking_lot::RwLock` wrapper** around each external bitmap | How does the complete thread-safe structure perform under the same sharding rule? |
| Context controls | `HashSet<u32>` and `BTreeSet<u32>` for sequential build/lookup/memory; [`dashmap::DashSet<u32>`](https://docs.rs/dashmap/latest/dashmap/struct.DashSet.html) for concurrent build/lookup/memory | Is compression useful compared with ordinary sets? These are not Roaring competitors; omit them from `and`/`or` rankings. |

The `roaring` and `croaring` bitmaps are sequential APIs. CRoaring explicitly says it has no built-in thread support, so a synchronized wrapper is necessary for a mutable concurrent comparison. Describe results as “our sharded bitmap vs. identically sharded `roaring`/`croaring` wrappers,” not as a win over a native concurrent product. [CRoaring thread-safety documentation](https://github.com/RoaringBitmap/CRoaring#thread-safety).

Pin exact versions in `Cargo.lock` and the report. The repo currently benchmarks `roaring = "0.10"`; the current `roaring` API has `optimize()` and run containers, unlike that historical 0.10 baseline. Rerun **all** sequential candidates on the same checkout before citing a new comparison, and give all three Roaring libraries their run-optimization pass outside timed lookup/set-operation cells. Also report an unoptimized row where it helps explain a result. [`roaring` current API](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html); [`croaring` API](https://docs.rs/croaring/latest/croaring/bitmap/struct.Bitmap.html).

## 2. Fair benchmark contract

1. Use one Rust benchmark binary, one machine, one toolchain, one release profile, and identical input values and operation traces. Record CPU model, core topology, OS, compiler, crate versions, commit hash, build flags, and date. Repeat on a second machine only if available; label hardware-specific conclusions.
2. Construct each candidate from an empty bitmap with the same **per-value** `insert` loop. Use `croaring`'s checked add/remove methods when comparing return-value semantics; if a faster unchecked operation is also interesting, label it as a separate API comparison. Do not compare batch-add/range-add against per-value insert.
3. Build/preload, optimization, RNG, trace generation, and verification happen outside timed lookup and set-operation regions. Time build and optimize separately. `black_box` the observed result. Use the same hit/miss sequence, including duplicate rates, for every candidate.
4. For set operations, time production of a fresh result. Use pairs with controlled **0%, 50%, and 100% overlap**, and both balanced and skewed cardinalities. The existing `clustered × clustered` case uses the *same object* twice; retain it as a diagnostic, but exclude it from the headline comparison.
5. For concurrency, use the same 64-shard rule (`(x >> 16) & 63`), lock type, padding, and one-shard-at-a-time read/write discipline in the external wrappers. Build each wrapper outside the timed cell. Keep the repo's three strategies as separate rows. Add a global-`RwLock` external wrapper only as a separately labeled integration baseline.
6. Keep the existing Baseline A (one thread vs. this repo's sequential type) and Baseline B (sequential vs. published Roaring) labels. Call the new wrapper comparison **Baseline C: end-to-end concurrent implementations**. Do not derive a concurrency-tax claim by comparing against a different library.

## 3. Workload matrix

| Axis | Required cases | Reason |
|---|---|---|
| Shape | Existing pinned-seed dense, sparse, clustered datasets; plus the existing balanced 2-million-value/64-shard preload | Exposes array, bitmap, run, and shard-distribution behavior. |
| Sequential | Build; 0/50/100% hit `contains`; successful and unsuccessful `remove`; `and`/`or` at controlled overlap; `optimize` cost | Separates bulk construction, read, mutation, and set kernels. |
| Concurrent | `contains` only; 99.5/0.5, 95/5, 50/50 read/write mixes; 1, 2, 4, 8 threads | Captures the read advantage and clone-per-write penalty already visible in the repo. |
| Contention | Evenly distributed keys and a deliberately hot subset of shards | Prevents a uniform-only result from hiding lock contention. |
| Mutation type | Duplicate/no-op inserts and successful inserts/removes, in separate cells | RCU writers skip the clone on a no-op; the success rate materially changes results. |

Precompute per-thread traces. Use the **same total operation count** at each thread count within a workload (strong scaling), reset to the same preload before each write-containing trial, and record actual successful writes. Choose the count in a pilot so even the fastest cell has enough timed work and the slowest finishes reasonably; freeze that count before the scored run. Each cell gets at least five independent trials after warmup. Rotate candidate order across trials, report median throughput and spread (for example, min/max or a bootstrap 95% interval), and keep every raw trial. Include a trace-only control to estimate loop/dispatch overhead; if it is a material fraction of a measured cell, increase work or report the limitation.

## 4. Correctness and memory gates

- Before timing, replay each sequential trace against a `BTreeSet<u32>` oracle. Compare each insert/remove return value, final cardinality, and exact membership. Check `and`/`or` results element-for-element via the reference iterators and cardinality; run this repo's `assert_invariants()`.
- For concurrent tests, use thread-disjoint mutation values for the deterministic correctness pass, then verify final cardinality and membership after joining. Keep a separate contended stress case; compare its final state only where the operation schedule makes it deterministic. Do not publish a speed result from a failing correctness cell.
- Measure **retained process RSS** after an untimed preload and **peak RSS** during build in fresh processes; subtract an empty-process control and repeat. Process RSS includes Rust, C, allocator, and wrapper overhead, so label it “process memory,” not exact bitmap bytes. Measure per candidate/dataset at the same cardinality and include wrapper memory for concurrent rows. Do not claim “X× less memory” from serialized size, or from a single noisy RSS sample.

## 5. Execution and artifacts

1. Add `croaring`, `dashmap`, and the current `roaring` as benchmark-only dependencies; keep the historical 0.10 result and its explanation intact. Implement small benchmark-local adapters and external sharded wrappers; keep library code unchanged.
2. Add one sequential Criterion suite and one concurrent trace-driven harness. Write raw per-trial CSV under `bench-results/` (already ignored), with candidate, version, workload, dataset, seed, optimization state, thread count, operation count, successful-write count, elapsed time, throughput, and memory fields where applicable.
3. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`; then run the full release benchmarks. Archive the exact command, `Cargo.lock`, machine spec, commit hash, raw CSV, and generated plots/table. A README summary may use a small representative table and link to the full report.
4. Review each apparent win: check interval overlap, trace overhead, equal result size, optimization state, shard balance, and whether any special feature of a candidate was left disabled. Investigate every large reversal or outlier; rerun only cells with a concrete measurement concern.

## 6. Resume-claim rule

Choose the claim **after** reading the complete table. Lead with one defensible scenario, name the competitors and workload, and include the hardware and link to the report. For example:

> Built a concurrent Roaring bitmap in Rust; on [machine], its [strategy] delivered [X] M lookups/s at 8 threads on [dataset], [Y]× the same 64-shard `roaring` wrapper and [Z]× the `croaring` wrapper, with [memory metric] at [cardinality].

Use “fastest” only if it wins the stated metric across **every named candidate** under the stated conditions, with uncertainty small enough to distinguish the result. If speed and memory winners differ, give two scoped facts. If no external win survives the checks, the measured three-strategy read/write tradeoff and the reproducible comparison are still strong resume material; report the actual numbers.
