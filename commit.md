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
- [ ] **P7** — `ConcurrentRoaringBitmap` (sharded `RwLock`) + scaling harness + tax (Baseline A)
- [ ] **P8a** — `SnapshotRoaringBitmap` (`arc-swap` lock-free reads)
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

Tax (single-threaded, vs sequential `RoaringBitmap`):

| Benchmark | Sequential | Sharded (1 thread) | Overhead % | T1 (≤10%)? |
|---|---|---|---|---|
| build/clustered | | | | |
| contains/clustered | | | | |

Scaling (`bench-results/scaling.csv` summary, Mops/s):

| Workload | 1t | 2t | 4t | 8t | 16t | 8t/1t | T2 (≥4×)? |
|---|---|---|---|---|---|---|---|
| read95 | | | | | | | |
| mixed50 | | | | | | | |
| write95 | | | | | | | |

### P8 — Full comparative matrix

Tax (single-threaded, all structures vs sequential):

| Structure | build overhead % | contains overhead % |
|---|---|---|
| sharded | | |
| snapshot | | |
| epoch | | |

Scaling at read95 (Mops/s):

| Structure | 1t | 2t | 4t | 8t | 16t |
|---|---|---|---|---|---|
| sharded | | | | | |
| snapshot | | | | | |
| epoch | | | | | |

Scaling at write95 (Mops/s):

| Structure | 1t | 2t | 4t | 8t | 16t |
|---|---|---|---|---|---|
| sharded | | | | | |
| snapshot | | | | | |
| epoch | | | | | |

_Written reading of results (required by P8 exit): …_

---

## Deviations from Plan

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

## Resume Bullets

_Filled at P9 with real measured numbers — see CLAUDE.md P9 deliverables._
