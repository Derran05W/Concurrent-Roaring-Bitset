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
- [ ] **P2** — `BitmapContainer` + array↔bitmap conversion
- [ ] **P3** — `RunContainer` + smallest-of-three `optimize`
- [ ] **P4** — `RoaringBitmap` top level + differential testing
- [ ] **P5** — Set operations (`and` / `or`)
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

_None yet. Format: **P<n> · <date>** — what changed vs. CLAUDE.md, and why it was necessary._

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
Commit: _pending_
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

---

## Resume Bullets

_Filled at P9 with real measured numbers — see CLAUDE.md P9 deliverables._
