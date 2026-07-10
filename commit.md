# commit.md — Progress Ledger

This is the **living** counterpart to the static `CLAUDE.md`. It answers, at a glance: what is
done, what was measured, and how the project got here.

**Update ritual (per CLAUDE.md §1.8):** a phase's checkbox is ticked only after its full Exit Gate
has run clean in this working tree. Every tick is accompanied, in the same commit, by a Commit
History entry below, any ledger tables the phase requires, and a Deviations note if the
implementation departed from the plan in any way.

---

## Phase Checklist

- [x] **P0** — Repository scaffold & harness
- [x] **P1** — `ArrayContainer` + `Container` enum
- [x] **P2** — `BitmapContainer` + array↔bitmap conversion
- [x] **P3** — `RunContainer` + smallest-of-three `optimize`
- [x] **P4** — `RoaringBitmap` top level + differential testing
- [x] **P5** — Set operations (`and` / `or`)
- [x] **P6** — Sequential baseline benchmarks (Baseline B recorded)
- [x] **P7** — `ConcurrentRoaringBitmap` (sharded `RwLock`) + scaling harness + tax (Baseline A)
- [x] **P8a** — `SnapshotRoaringBitmap` (`arc-swap` lock-free reads)
- [ ] **P8b** — `EpochRoaringBitmap` (`crossbeam-epoch` lock-free reads)
- [ ] **P9** — Comparative writeup, graphs, resume bullets

---

## Benchmark Ledger

Numbers land here as phases complete. Every table states which baseline (A: concurrency tax,
B: absolute reference) it addresses. Include the machine spec line once, above the first table.

**Machine:** Apple M5 · 10 physical / 10 logical cores · 24 GiB · macOS 26.5.1 (arm64) · rustc 1.97.0 (2d8144b78 2026-07-07). Criterion 0.5, `--release`, median of 100 samples (10 for `build/sparse` per criterion's estimate).

### P6 — Sequential baseline (Baseline B: ours vs `roaring` crate)

Ratio = ours ÷ RefBitmap median; **<1 means we are faster**, >1 means slower.

| Benchmark | Dataset | Ours | RefBitmap | Ratio | Notes |
|---|---|---|---|---|---|
| build | dense | 2.948 ms | 3.837 ms | 0.77× | ours faster — dense keys become bitmap containers we fill by direct bit-set |
| build | sparse | 700.9 ms | 529.5 ms | 1.32× | our worst case — 1M random values ⇒ ~65k array containers, each insert is a `Vec::insert` shift |
| build | clustered | 12.58 ms | 14.98 ms | 0.84× | ours faster |
| contains | dense | 9.219 ms | 7.737 ms | 1.19× | slightly slower — optimize() makes dense a single-run RunContainer; run `partition_point` vs a raw bit test |
| contains | sparse | 46.25 ms | 47.91 ms | 0.97× | parity |
| contains | clustered | 18.96 ms | 19.85 ms | 0.95× | parity |
| remove | clustered | 7.871 ms | 7.293 ms | 1.08× | parity |
| and | dense×sparse | 554.1 ns | 93.01 µs | 0.006× | tiny intersection (only keys 0..15 overlap); array·bitmap kernel probes ~15 values/key |
| and | clustered×clustered | 16.54 µs | 602.6 µs | 0.027× | self-∩ of optimized runs ⇒ run·run two-pointer over few runs |
| and | sparse×sparse | 2.069 ms | 2.207 ms | 0.94× | parity |
| or | dense×sparse | 1.472 ms | 1.556 ms | 0.95× | parity |
| or | clustered×clustered | 17.02 µs | 835.6 µs | 0.020× | self-∪ of optimized runs ⇒ run·run interval merge over few runs |
| or | sparse×sparse | 2.257 ms | 2.339 ms | 0.96× | parity |

**T3 check.** No *unfavorable* ratio exceeds 2× — our worst is `build/sparse` at 1.32×, caused by
sorted-array insertion cost: a duplicate-free sparse load creates ~65k `ArrayContainer`s and every
distinct insert is an O(card) `Vec::insert` element shift, whereas the `roaring` crate uses the same
array representation but a tighter insert path. It is within explainable distance and well under the
2× gate.

Three ratios are *dramatically favorable* (0.006×–0.027×): `and`/`or` on `clustered×clustered` and
`and` on `dense×sparse`. These are honest and structural, not a measurement artifact — the operands
are `optimize()`d on our side, so `clustered×clustered` becomes run·run kernels iterating a handful
of runs, and `dense×sparse` intersects only the ~16 overlapping keys with a short array·bitmap probe.
The reference crate (no run containers in 0.10) does more per-word work. All results pass
`assert_invariants` via the P5 differential tests, so the outputs are verified correct, not empty.

### P7 — Concurrency tax (Baseline A) & sharded scaling

Tax (single-threaded, vs sequential `RoaringBitmap`; overhead = sharded ÷ sequential − 1, criterion median):

| Benchmark | Sequential | Sharded (1 thread) | Overhead % | T1 (≤10%)? |
|---|---|---|---|---|
| build/clustered | 11.77 ms | 8.441 ms | **−28.3%** | ✅ (faster) |
| contains/clustered | 22.46 ms | 16.74 ms | **−25.5%** | ✅ (faster) |

The tax is *negative* — the concurrent structure is single-threaded **faster** than the sequential
one. This is honest, not a measurement error: `ConcurrentRoaringBitmap` is 64 shards, each a whole
`RoaringBitmap` holding only the keys with `key & 63 == shard`. Sharding therefore *partitions the
data structure*: each shard's `Vec<(u16, Container)>` is ~1/64 the length, so a `contains` binary
search is shorter and a build-path `Vec::insert` of a new key shifts ~64× fewer elements. On the
clustered workload that structural win dominates the uncontended `parking_lot::RwLock` acquire cost
(a single atomic on an uncontended lock is a handful of nanoseconds). T1 (≤10% overhead) is met with
margin — the machinery costs less than nothing here because it also shrinks the work.

Scaling (`bench-results/scaling.csv`, Mops/s; 16t clamped out — the M5 reports 10 logical cores):

| Workload | 1t | 2t | 4t | 8t | 16t | 8t/1t | T2 (≥4×)? |
|---|---|---|---|---|---|---|---|
| read95 | 22.87 | 30.71 | 50.30 | 50.67 | n/a | 2.22× | ❌ |
| mixed50 | 15.09 | 23.28 | 37.67 | 36.59 | n/a | 2.42× | ❌ |
| write95 | 12.79 | 20.73 | 33.82 | 28.67 | n/a | 2.24× | ❌ |

**T2 cause analysis (goal missed — §0.2 requires the cause, not a phase failure).** Read-heavy
throughput rises monotonically through 8 threads but reaches only **2.22×** its 1-thread number, short
of the ≥4× goal; the write-heavier mixes even *regress* past 4 threads (mixed50 37.67→36.59,
write95 33.82→28.67). Two causes, both anticipated by the plan and both the explicit motivation for
P8:

1. **Core topology.** The benchmark box is an Apple M5: 10 logical cores but a heterogeneous
   performance/efficiency split (~4 P-cores). Every workload's throughput knee is exactly at 4
   threads — past the P-cores, threads land on much slower E-cores, so 4t→8t adds little and, once
   write-lock contention rises, goes backwards. This caps *all* structures on this box and is a
   property of the hardware, not the algorithm.
2. **The `RwLock` read path is not free of shared writes.** Even a *reader* mutates the lock's atomic
   word to register itself; two readers on the same shard bounce that cache line. With 64 shards the
   collision rate is low but non-zero, and the 5% writers take exclusive per-shard locks that stall
   every reader on that shard for the duration of an O(shard-size) `Vec::insert`. This is precisely
   the cost P8a/P8b remove by making reads load an immutable snapshot pointer with no shared write —
   the comparative P8 table will show whether lock-free reads recover the scaling the `RwLock` leaves
   on the table.

### P8 — Full comparative matrix

_Filled incrementally: the sharded and snapshot rows below are from the P8a run (all internally
consistent — one `cargo bench -- tax/` invocation, one `scaling` run). The `epoch` rows and the
final written reading land with P8b, which reruns the full matrix fresh into the CSV._

Tax (single-threaded, all structures vs sequential; overhead = variant ÷ sequential − 1, criterion
median. Same-run sequential baselines: build/clustered 12.523 ms, contains/clustered 23.034 ms):

| Structure | build overhead % | contains overhead % |
|---|---|---|
| sharded | **−32.1%** (8.501 ms) | **−27.3%** (16.736 ms) |
| snapshot | **+4074%** (522.7 ms, ≈41.7×) | **−20.2%** (18.387 ms) |
| epoch | _(P8b)_ | _(P8b)_ |

Scaling at read95 (Mops/s; 16t clamped — M5 reports 10 logical cores):

| Structure | 1t | 2t | 4t | 8t | 16t |
|---|---|---|---|---|---|
| sharded | 21.48 | 29.94 | 49.95 | 50.29 | n/a |
| snapshot | 1.93 | 1.85 | 2.60 | 3.46 | n/a |
| epoch | _(P8b)_ | | | | |

Scaling at write95 (Mops/s):

| Structure | 1t | 2t | 4t | 8t | 16t |
|---|---|---|---|---|---|
| sharded | 12.20 | 20.40 | 33.23 | 26.32 | n/a |
| snapshot | 0.045 | 0.068 | 0.093 | 0.099 | n/a |
| epoch | _(P8b)_ | | | | |

**Partial reading of the P8a results (finalized at P8b with the epoch row).** The snapshot type is a
*read*-optimized structure, and the numbers say so with unusual clarity:

- **Reads are genuinely fast.** The pure-read `contains` tax is **−20.2%** — lock-free `ArcSwap::load`
  plus shard-partitioned (shorter) per-shard vectors beat the sequential map. It loses to the sharded
  `RwLock` read (16.74 ms) by ~10% because `arc-swap`'s load-and-debt bookkeeping is slightly heavier
  than a `parking_lot` reader acquire, but both beat sequential.
- **Writes pay a full-shard clone, and it is brutal on write-heavy loads.** The `build` tax is
  **+4074% (≈42×)**: build is an all-insert workload, and every insert clones the entire shard's
  `RoaringBitmap` before mutating (single-writer RCU), making incremental build effectively O(N²)
  per shard. This is not a defect — it is the deliberate tradeoff being measured (§P8 preamble). T1
  (≤10% tax) is therefore met for reads and **intentionally missed** for the write path; the cause is
  structural, not a regression.
- **In the scaling harness every workload contains writes, so throughput is clone-bound, not
  read-bound.** Even `read95` (5% writes) sits at 1.9–3.5 Mops because those 5% of ops each clone a
  large clustered shard; `write95` collapses to ~0.05–0.10 Mops. Reads *do* scale with threads
  (read95 1.93→3.46, ≈1.8× at 8t — the lock-free path adds no shared write), but the serialized
  per-shard clones cap it far below the sharded structure on any write-containing mix.
- **Lever for the write cost:** clone size is per-shard, so `with_shard_count(256)` (vs the default 64)
  shrinks each cloned unit ~4× and would lift the write-heavy numbers proportionally — the plan flags
  this exact mitigation. Not run here; noted for the P9 tradeoff analysis.

The takeaway the P8b epoch row will sharpen: `SnapshotRoaringBitmap` trades write throughput for a
lock-free read path that is faster than both the sequential map and (marginally slower than) the
`RwLock` reader — a win only on read-dominated-to-read-only workloads, exactly the regime it targets.

### OPT — Post-P8a optimization pass (user-directed, between P8a and P8b)

Four changes, measured incrementally (deviations recorded below): **(1)** fat-LTO / single-CGU
release profile; **(2)** `RoaringBitmap` re-laid out as parallel `keys: Vec<u16>` +
`containers: Vec<Container>` (SoA) instead of `Vec<(u16, Container)>`; **(3)** shards in both
concurrent types padded to one 128-byte cache line each; **(4)** presized `and`/`or` outputs +
direct-push `BitmapContainer::to_array`. Same machine/rustc as above; the old scaling matrix is
preserved at `bench-results/scaling-pre-optimization.csv`.

**Baseline B re-run (ours vs `roaring` crate).** "Before" is the P6/P8a ledger value on this box.

| Benchmark | Ours before | Ours after | Δ ours | Ref after | Ratio after (was) |
|---|---|---|---|---|---|
| build/dense | 2.948 ms | 2.427 ms | −17.7% | 3.999 ms | 0.61× (0.77×) |
| build/sparse | 700.9 ms | 542.9 ms | −22.5% | 534.3 ms | **1.02× (1.32×)** |
| build/clustered | 12.58 ms | 9.647 ms | −23.3% | 15.08 ms | 0.64× (0.84×) |
| contains/dense | 9.219 ms | 6.915 ms | −25.0% | 7.387 ms | **0.94× (1.19×)** |
| contains/sparse | 46.25 ms | 26.77 ms | **−42.1%** | 49.81 ms | 0.54× (0.97×) |
| contains/clustered | 18.96 ms | 13.43 ms | −29.2% | 20.00 ms | 0.67× (0.95×) |
| remove/clustered | 7.871 ms | 7.190 ms | −8.7% | 7.502 ms | 0.96× (1.08×) |
| and/dense×sparse | 554.1 ns | 418.1 ns | −24.5% | 94.86 µs | 0.004× |
| and/clustered×clustered | 16.54 µs | 14.05 µs | −15.0% | 505.6 µs | 0.028× |
| and/sparse×sparse | 2.069 ms | 1.908 ms | −7.8% | 1.943 ms | 0.98× (0.94×) |
| or/dense×sparse | 1.472 ms | 1.380 ms | −6.3% | 1.511 ms | 0.91× (0.95×) |
| or/clustered×clustered | 17.02 µs | 14.18 µs | −16.7% | 546.1 µs | 0.026× (0.020×) |
| or/sparse×sparse | 2.257 ms | 1.797 ms | −20.4% | 1.985 ms | 0.91× (0.96×) |

**T3 after the pass: every ratio ≤ 1.02×.** The one former >1.1× unfavorable gap, `build/sparse`
(1.32×), is now statistical parity: the SoA layout cut the top-level cost both ways — the key
binary search walks a dense 2-byte-stride vec (≤128 KiB fully populated, cache-resident) instead of
striding 48-byte tuples, and a new-key `Vec::insert` shifts 42 B/entry instead of 48. LTO alone
moved `build/sparse` +1% (it is memmove-bound, as the P6 analysis said); the layout change was the
fix. `contains/sparse` (−42%) is the purest read of the same effect. Isolated LTO-only deltas
(round 1): build/dense −16.5%, contains/{dense,sparse,clustered} −4.1/−3.6/−1.9%, others ~noise;
the reference crate moved ±0–4% on build/contains and −11…−34% on set-ops in the same binary, and
the after-ratios above absorb that (e.g. and/sparse×sparse 0.94×→0.98× because *their* kernels
LTO'd better than ours — both sides' absolute times improved).

**Tax re-run (Baseline A; same-run sequential references: build 9.706 ms, contains 17.317 ms):**

| Structure | build overhead % | contains overhead % |
|---|---|---|
| sharded | **−20.6%** (7.710 ms) | **−19.9%** (13.872 ms) |
| snapshot | +5343% (528.3 ms, ≈54×) | **−13.0%** (15.071 ms) |

T1 unchanged in verdict: met for sharded and for snapshot reads; snapshot build is intentionally
missed (clone-per-write is the measured tradeoff — its absolute time is unchanged at ~528 ms, and
the ratio grew ≈42×→≈54× only because the *sequential baseline* got 24.7% faster).

**Scaling re-run (before → after, Mops/s):**

| Structure/workload | 1t | 2t | 4t | 8t | 8t/1t |
|---|---|---|---|---|---|
| sharded read95 | 21.48→29.56 | 29.94→39.55 | 49.95→64.79 | 50.29→**75.97** | 2.34×→2.57× |
| sharded mixed50 | 14.51→20.70 | 23.03→32.06 | 29.85→51.71 | 29.00→50.47 | 2.00×→2.44× |
| sharded write95 | 12.20→17.06 | 20.40→28.25 | 33.23→44.91 | 26.32→34.82 | 2.16×→2.04× |
| snapshot read95 | 1.93→2.26 | 1.85→1.99 | 2.60→2.75 | 3.46→3.80 | 1.79×→1.68× |
| sequential (1t only) read95/mixed50/write95 | 4.12→5.35 / 2.66→3.50 / 2.55→3.37 | | | | |

**T2 reading.** Absolute read-heavy throughput at 8 threads rose **+51%** (50.3→76.0 Mops), and the
4t→8t segment that was dead flat before padding (+0.7%) now gains +17% — that flatline was the
predicted false sharing: unpadded, four 32-byte `RwLock` shards share one 128-byte M5 cache line, so
even *readers* of unrelated shards bounced lock-word lines. read95 stays monotonic through 8t ✓.
The self-relative 8t/1t ratio improves only 2.34×→2.57× (< the 4× goal) because the 1-thread
number itself got 38% faster — the ratio's denominator rose with the same optimizations. The P7
cause analysis stands: past the M5's 4 P-cores, added threads land on E-cores (4t is already 2.19×
of 1t), and the 5% write mix still takes exclusive per-shard locks. write95's ratio dipped
(2.16×→2.04×) for the same denominator reason; its absolute 8t throughput is +32%.

---

## Deviations from Plan

**OPT · 2026-07-09** — User-directed optimization pass (not a phase). Four departures/additions,
all behaviour-preserving (full gate suite + differential proptests + release-mode stress suite green):
1. **`RoaringBitmap` layout** deviates from §2.5's pinned `Vec<(u16, Container)>`: keys and
   containers now live in parallel vecs (`keys: Vec<u16>`, `containers: Vec<Container>`, index-
   paired). Why: every op starts with a key binary search, and the tuple layout strides 48 bytes
   per probe (the enum + tag is 40 B) while the SoA key vec strides 2 — the whole key set is
   ≤128 KiB and cache-resident — and a new-key insert shifts 42 B/entry instead of 48. This closed
   the worst Baseline-B gap (build/sparse 1.32×→1.02×) and cut contains/sparse 42%. Semantics,
   invariants, and the P7 "shard = partition by key" property are unchanged.
2. **Shard padding** (`#[repr(align(128))]`) in `ConcurrentRoaringBitmap` and
   `SnapshotRoaringBitmap`. The plan doesn't specify shard memory layout; unpadded, four 32-byte
   `RwLock` shards (eight 16-byte ArcSwap shards) shared one 128-byte M5 cache line and reader
   lock-word RMWs false-shared across shards — the read95 4t→8t flatline. 8t read95: 50.3→76.0 Mops.
3. **Release profile** `lto = "fat"`, `codegen-units = 1` in `Cargo.toml`. Applies to ours and the
   `roaring` reference inside the same bench binary, so Baseline-B stays fair; isolated effect was
   measured before the code changes (round-1 numbers in the OPT ledger section).
4. **`BitmapContainer::to_array`** now pushes extracted bits into a plain `Vec` (sorted by
   construction) instead of routing each value through `ArrayContainer::insert` — the plan's own
   wording ("pushing in order"); the old form paid a useless per-value binary search. `and`/`or`
   output vecs are additionally presized (implementation detail, no plan text involved).

**P8a · 2026-07-09** — `insert` and `remove` share one private RCU helper `update(x, present: bool)`
rather than two mirrored bodies. The plan describes remove as "the mirror" of insert with an
identical no-op-short-circuit + clone + store shape, so this is a faithful single-source
implementation of the prescribed logic (behaviour is byte-identical: `present` is the membership the
op targets, the short-circuit is `cur.contains(x) == present`), not a semantic departure. Noted only
so a reader diffing against the plan's two-signature sketch finds the one function.

**P7 · 2026-07-09** — The prescribed bench-local tax trait was to carry both
`insert(&mut self)` *and* `contains(&self)`. In practice `contains` in the trait is dead code: both
`RoaringBitmap` and `ConcurrentRoaringBitmap` have an inherent `contains`, so `x.contains(v)` always
resolves to the inherent method and the trait method is never dispatched — which fails
`clippy -D warnings` (`dead_code`). The trait therefore carries only `insert` (the one method whose
signature genuinely differs, `&mut self` vs `&self`, and which the shared generic build loop needs);
the two `contains` benches call each type's inherent `contains` directly. Functionally identical to
the plan; the numbers measure the real methods.

Also, the scaling harness runs the `sequential` structure on a plain `&mut RoaringBitmap` in a single
thread with no lock wrapper (rather than routing it through the concurrent-bench trait behind a
`Mutex`). This keeps the sequential row a true lock-free reference — wrapping it in a `Mutex` purely
to fit a `&self` trait would tax the baseline with lock overhead it should not carry. Within the plan
("`sequential` (single-thread only)").

**P5 · 2026-07-09** — The plan's `setops_match_roaring_crate` test says to "optimize one of them
to force Run participation." The intent is to exercise *our* Run kernels, so only our operand `a`
is `optimize()`d; the reference operand is left as-is. This is necessary because the `roaring` 0.10
crate has no run containers and exposes no run-optimize method (confirmed against the vendored
source), so there is nothing to call on the reference — and it needs nothing: it is only the set
oracle, and optimizing our side alone already forces every bitmap·run / run·run kernel path. (The
P6 plan anticipates exactly this: "call the ref crate's run-optimize equivalent if it exposes one …
if it doesn't, note that in the ledger.")

---

**P6 · 2026-07-09** — Two plan-adjacent additions, neither a semantic departure:
1. `rand` moved from `[dev-dependencies]` to `[dependencies]`. The P6 `datasets` module lives in
   library code (`src/bitmap.rs` per §2.1) and the P7 `src/bin/scaling.rs` binary will also consume
   it; neither the library nor a `src/bin` target can see dev-dependencies, so the generators cannot
   compile unless `rand` is a normal dependency. §1.7 says dependencies are added in the phase that
   first needs them — P6 is that phase for `rand`-in-the-library.
2. `contains`/`and`/`or` benches `optimize()` **only our** structures; the `roaring` 0.10 crate
   exposes no run-optimize / run-compression method on `RoaringBitmap` (grep of the vendored 0.10.12
   source found none — same finding as the P5 deviation). The P6 plan text explicitly anticipates
   this ("call the ref crate's run-optimize equivalent if it exposes one … if it doesn't, note that
   in the ledger").

Also: two dataset seeds are written regrouped to 4-hex-digit blocks to satisfy
`clippy::unusual_byte_groupings` while preserving the exact pinned values — `0xC0FF_EE` → `0x00C0_FFEE`
(clustered) and `0xBADC_0DE` → `0x0BAD_C0DE` (remove-sample). Same numeric seeds, no data change.

## Commit History

Entry template (append newest at the bottom; one entry per phase, plus entries for any
significant fix commits):

```
### P<n> — <title> (<YYYY-MM-DD>)
Commit: <short hash>
Done: <what capability now exists, 1–3 lines>
Measured: <numbers recorded, if any — else "n/a">
Deviations: none | <pointer to Deviations section>
Next: P<n+1>
```

### P0 — Repository scaffold & harness (2026-07-09)
Commit: 8719b50
Done: `cargo init --lib` (crate `concurrent_roaring`, edition 2021); §2.1 sequential-subset
skeleton (`lib.rs`, `bitmap.rs`, `container/{mod,array,bitmap,run}.rs`, `benches/sequential.rs`,
`tests/smoke.rs`), each module a `//!` doc stub; dev-deps criterion/proptest/roaring/rand;
`[[bench]] harness=false`; criterion placeholder bench; `.gitignore` (target/, bench-results/).
Measured: n/a
Deviations: none
Next: P1

### P1 — `ArrayContainer` + `Container` enum (2026-07-09)
Commit: c050b1d
Done: `ArrayContainer` (sorted `Vec<u16>`) with `new`/`cardinality`/`is_empty`/`contains`/
`insert`/`remove`/`num_runs`/`as_slice` per §2.4 array formula; `Container` enum introduced with
only the `Array` variant, dispatching all six ops. Unit tests (0/65535 boundaries, dup/absent,
interleaved sortedness, num_runs), proptest `array_matches_btreeset` (vs `BTreeSet<u16>`) and a
strictly-increasing invariant proptest.
Measured: n/a
Deviations: `as_slice` carries a site-local `#[allow(dead_code)]` (why-comment): it is a listed
P1 deliverable but first consumed by P2/P5, so the lib-only build sees it unused. Not a plan
deviation — the plan prescribes the method.
Next: P2

### P2 — `BitmapContainer` + array↔bitmap conversion (2026-07-09)
Commit: d40118e
Done: `BitmapContainer` (`Box<[u64; 1024]>` + cached `u32` cardinality) with
`new`/`from_array`/`to_array`/`contains`/`insert`/`remove`/`cardinality`/`is_empty`/`num_runs`
(bit-trick fold with word-boundary correction) and `pub(crate) words()`. `Container` gained the
`Bitmap` variant; §2.4 conversion policy lives in `Container::insert` (array→bitmap pre-convert on
the 4097th distinct value) and `Container::remove` (bitmap→array at cardinality exactly 4096).
Tests: cross-representation agreement proptest, `to_array∘from_array` round-trip, `num_runs`
word-boundary units (incl. the −1 correction), and threshold-through-`Container` (unit + proptest).
Measured: n/a
Deviations: none
Next: P3

### P3 — `RunContainer` + smallest-of-three `optimize` (2026-07-09)
Commit: dc7a259
Done: `RunContainer` (`Vec<Run>` + cached `u32` cardinality; `Run{start,len}`, len=count−1) with
`contains`/`insert` (extend/merge/isolated) / `remove` (shrink/split) / `cardinality`/`is_empty`/
`num_runs`/`from_array`/`from_bitmap`/`to_array`/`to_bitmap` and `pub(crate) runs()`; all boundary
math in `u32`. `Container` gained the `Run` variant + dispatch; run-arm mutations demote to Bitmap
when `4×num_runs > 8192` (`demote_run_if_bloated`); `Container::optimize` implements the strict
smallest-of-three (ties keep current) via a private `Repr` target enum. Tests: tri-representation
agreement + round-trips, run mutation vs `BTreeSet` with invariant checks (sorted/non-overlapping/
non-adjacent/cached-card), `optimize` shrink+idempotent proptest, and unit tests exercising both
insert- and remove-driven run→bitmap demotion.
Measured: n/a
Deviations: none
Next: P4

### P4 — `RoaringBitmap` top level + differential testing (2026-07-09)
Commit: 2a4fa35
Done: `split`/`join` value-model helpers (`bitmap.rs`) with boundary units; top-level
`RoaringBitmap` (`Vec<(u16, Container)>` sorted-unique-by-key) with `new`/`insert`/`remove`
(drops emptied containers per the never-empty invariant) / `contains` / `len` (no cached global
count) / `is_empty` / `optimize` / `#[doc(hidden)] assert_invariants` (keys sorted+unique, no
empty container, per-container structural checks with recomputed cached cardinalities via a new
`Container::assert_invariants`). Added `Container::single(v)` for the new-key path and re-exported
`RoaringBitmap` from the crate root. Differential tests (`tests/differential.rs`) vs
`roaring::RoaringBitmap`: `matches_roaring_crate` (≤3000-op streams, every return value + final
len + sampled membership match), `optimize_preserves_semantics` (optimize interleaved, membership/
len unchanged), and boundary units at `0`/`u32::MAX`/`0xFFFF`/`0x1_0000`.
Measured: n/a
Deviations: none — `Container::single` and `Container::assert_invariants` are helper methods the
plan's prescribed logic requires (single-value-array construction for `insert`'s `Err` arm; the
per-container half of `RoaringBitmap::assert_invariants`); both live in the container module where
the leaf accessors are, keeping representation knowledge out of `bitmap.rs`.
Next: P5

### P5 — Set operations (`and` / `or`) (2026-07-09)
Commit: de2f0fe
Done: Top-level `RoaringBitmap::and`/`or` (two-pointer merge-join over the sorted key vecs — `and`
intersects shared keys and drops empty results; `or` carries single-side containers over cloned and
kernel-merges shared keys) plus `BitAnd`/`BitOr`/`BitAndAssign`/`BitOrAssign` operator delegations.
Container-level `and`/`or` dispatch with all six mirrored kernel pairs (array·array, array·bitmap,
array·run, bitmap·bitmap, bitmap·run, run·run) per the pinned CRoaring algorithms; every kernel
result passes through a private `normalize` enforcing §2.4 legality (bitmap ≤4096 → array; run with
`4×num_runs > 8192` → bitmap). Added `pub(crate)` builders `ArrayContainer::from_sorted_vec`,
`BitmapContainer::from_words`, `RunContainer::from_runs`, and a shared `for_range_words` word-mask
helper for the bitmap·run kernels; dropped the now-obsolete `#[allow(dead_code)]` on the leaf
accessors. Tests (`tests/differential.rs`): `setops_match_roaring_crate` (and/or vs the crate via
the subset+equal-cardinality equality trick, our operand optimized to force Run kernels),
`setops_algebraic` (∩⊆ operands, ∪⊇ operands, commutativity in cardinality), and an operator-form
unit test; all results run `assert_invariants`.
Measured: n/a
Deviations: **P5 · 2026-07-09** — reference operand not run-optimized (roaring 0.10 has no run
containers); see Deviations section.
Next: P6

### P6 — Sequential baseline benchmarks (2026-07-09)
Commit: 90e5de6
Done: `#[doc(hidden)] pub mod datasets` in `src/bitmap.rs` (deterministic pinned-seed dense/sparse/
clustered/probes generators); `benches/sequential.rs` rewritten from the P0 placeholder into four
groups (`build`, `contains`, `remove`, `and`/`or`) each measuring ours vs `roaring::RoaringBitmap`
side by side on identical inputs. `rand` promoted to a normal dependency (library + P7 binary need
the generators). Full `cargo bench` ran end-to-end; Baseline-B table filled.
Measured: Baseline B (see P6 table). Ours faster on `build/{dense,clustered}` (0.77×/0.84×), all
`or`/`and` (0.006×–0.96×), parity on `contains`/`remove`; worst unfavorable ratio `build/sparse`
1.32× (sorted-array insert cost) — no gap exceeds the 2× T3 gate.
Deviations: **P6 · 2026-07-09** — `rand` dev→normal dependency; ref crate has no run-optimize; two
seeds regrouped for clippy (values unchanged). See Deviations section.
Next: P7

---

### P7 — `ConcurrentRoaringBitmap` (sharded `RwLock`) + scaling + tax (2026-07-09)
Commit: 0c09507
Done: `ConcurrentRoaringBitmap` (`Box<[parking_lot::RwLock<RoaringBitmap>]>`, power-of-two shards,
default 64, `key & mask` low-bit sharding) with `new`/`with_shard_count`/`insert`/`remove`/`contains`
/`len`/`is_empty`/`snapshot`/`and`/`or`/`optimize`; per-shard-atomic (non-linearizable) cross-shard
ops commented at each site. `snapshot` clones each shard under a brief read lock and reassembles via
a new `pub(crate) RoaringBitmap::from_shards` (key-disjoint concat + sort, no kernel merge).
`parking_lot = "0.12"` added. Stress tests (`tests/concurrent_stress.rs`): disjoint-partition
lost-update detector (8 threads, residue classes) and a 2-second 4-writer/4-reader contended smoke,
both asserting `snapshot().assert_invariants()` — green under `--release`. Scaling harness
(`src/bin/scaling.rs`): `sequential`+`sharded` × read95/mixed50/write95 × {1,2,4,8} threads (16
clamped), barrier-synchronized, writes `bench-results/scaling.csv`. Tax group added to
`benches/sequential.rs` (bench-local trait) comparing both structures single-threaded.
Measured: Baseline A tax is **negative** (sharded faster: build −28.3%, contains −25.5% — sharding
shrinks per-shard vectors) ⇒ T1 met. Scaling reaches 2.2–2.4× at 8t (T2 ≥4× **missed** — 4-P-core M5
topology + `RwLock` reader-atomic/writer-exclusive contention; cause analysis in the P7 ledger,
motivates P8). See P7 tables.
Deviations: **P7 · 2026-07-09** — tax trait omits dead `contains`; scaling `sequential` runs lock-free.
See Deviations section.
Next: P8a

---

### P8a — `SnapshotRoaringBitmap` (`arc-swap` lock-free reads) (2026-07-09)
Commit: 1d0807d
Done: `SnapshotRoaringBitmap` (`Box<[Shard]>` where each `Shard` = `arc_swap::ArcSwap<RoaringBitmap>`
+ per-shard writer `Mutex<()>`; power-of-two shards, default 64, `key & mask` low-bit sharding).
Reads (`contains`/`len`/`is_empty`/`snapshot`) take no lock — they `ArcSwap::load` an immutable
snapshot pointer. Writes (`insert`/`remove`/`optimize`) are single-writer RCU: lock the shard mutex,
clone the current snapshot, mutate the clone, `store` it; `insert`/`remove` share one `update(x,
present)` helper with a no-op short-circuit that skips the clone when membership already matches.
Reclamation is `Arc` refcounting (readers holding a guard keep the old snapshot alive). `arc-swap =
"1"` added. Wired into `lib.rs`, the `stress_suite!` macro (both P7 patterns now stamped for sharded
+ snapshot), the scaling harness (`snapshot` structure), and the tax bench (`snapshot` arm).
Measured: Baseline A tax — reads **−20.2%** (lock-free load + shard partition beats sequential),
build **+4074% (≈42×)** (clone-per-insert ⇒ O(N²) build; the deliberate read-optimized tradeoff, T1
intentionally missed on the write path with cause). Scaling: read95 1.9→3.5 Mops (reads scale ~1.8×
at 8t but every workload's writes are clone-bound); write95 collapses to ~0.05–0.10 Mops. See the P8
comparative tables. Stress suite (both patterns) green for sharded + snapshot under `--release`.
Deviations: **P8a · 2026-07-09** — `insert`/`remove` share one `update()` helper (faithful to the
plan's "remove is the mirror"). See Deviations section.
Next: P8b

---

### OPT — Post-P8a optimization pass (2026-07-09)
Commit: 2d40098
Done: User-directed performance pass over everything built through P8a — SoA key/container layout
for `RoaringBitmap`; 128-byte cache-line-padded shards in both concurrent types; fat-LTO/1-CGU
release profile; presized `and`/`or` outputs; direct-push `to_array`. Full criterion suite, tax
group, and scaling matrix re-measured (prior matrix kept as
`bench-results/scaling-pre-optimization.csv`); gates + release-mode stress suite green.
Measured: Baseline B now ≤1.02× on every benchmark (worst was build/sparse 1.32× → 1.02× parity);
contains/sparse −42%, contains/clustered −29%, build/{sparse,clustered} −22/−23%. Tax: sharded
−20.6%/−19.9%, snapshot reads −13.0% (write path unchanged by design). Scaling: sharded read95 8t
50.3→76.0 Mops (+51%, 4t→8t flatline removed), mixed50 4t/8t +73%/+74%. T3 ✓ with margin, T1 ✓
(reads), T2 ratio 2.57× — cause analysis updated in the OPT ledger section.
Deviations: **OPT · 2026-07-09** (SoA layout, shard padding, release profile, to_array push form).
See Deviations section.
Next: P8b

---

## Resume Bullets

_Filled at P9 with real measured numbers — see CLAUDE.md P9 deliverables._
