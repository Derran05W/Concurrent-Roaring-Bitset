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
- [ ] **P6** — Sequential baseline benchmarks (Baseline B recorded)
- [ ] **P7** — `ConcurrentRoaringBitmap` (sharded `RwLock`) + scaling harness + tax (Baseline A)
- [ ] **P8a** — `SnapshotRoaringBitmap` (`arc-swap` lock-free reads)
- [ ] **P8b** — `EpochRoaringBitmap` (`crossbeam-epoch` lock-free reads)
- [ ] **P9** — Comparative writeup, graphs, resume bullets

---

## Benchmark Ledger

Numbers land here as phases complete. Every table states which baseline (A: concurrency tax,
B: absolute reference) it addresses. Include the machine spec line once, above the first table.

**Machine:** _(CPU model · cores/threads · RAM · OS · rustc version — fill at P6)_

### P6 — Sequential baseline (Baseline B: ours vs `roaring` crate)

| Benchmark | Dataset | Ours | RefBitmap | Ratio | Notes |
|---|---|---|---|---|---|
| build | dense | | | | |
| build | sparse | | | | |
| build | clustered | | | | |
| contains | dense | | | | |
| contains | sparse | | | | |
| contains | clustered | | | | |
| remove | clustered | | | | |
| and | dense×sparse | | | | |
| and | clustered×clustered | | | | |
| and | sparse×sparse | | | | |
| or | dense×sparse | | | | |
| or | clustered×clustered | | | | |
| or | sparse×sparse | | | | |

_T3 check: any ratio >2× requires a cause paragraph here._

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

---

## Resume Bullets

_Filled at P9 with real measured numbers — see CLAUDE.md P9 deliverables._
