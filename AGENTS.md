# AGENTS.md — `concurrent_roaring`: Static Technical Plan

> **This document is STATIC.** It is the plan of record, written once, before implementation.
> Executing agents must NOT edit this file. If reality forces a deviation, implement the
> deviation, then record it (what + why) in `commit.md → Deviations from Plan`.
> This file lives at the repo root so Codex auto-loads it into every session.

---

## 0. Mission

Build a **concurrent Roaring bitmap for `u32` values in Rust, without performance degradation**,
as a resume project for big-tech internship applications. The project's value is not "implemented
a bitmap" — it is **measured engineering judgment**: multiple concurrency strategies, benchmarked
against explicit baselines, with honest tradeoff analysis.

### 0.1 The two-baseline degradation model (normative — never conflate these)

| Baseline | Question it answers | Comparison |
|---|---|---|
| **A. Concurrency tax** | "What did the concurrency machinery cost when there is no contention?" | `ConcurrentRoaringBitmap` (and P8 variants) used **single-threaded** vs. **our own** sequential `RoaringBitmap`, identical workloads. |
| **B. Absolute reference** | "Is our sequential implementation any good in absolute terms?" | Our sequential `RoaringBitmap` vs. the published **`roaring` crate**, identical workloads. |

Every benchmark table in `commit.md` must state which baseline it addresses. A concurrent variant
is never compared directly against the `roaring` crate as its primary claim — that muddies the story.

### 0.2 Numeric targets

These are **goals, not hard gates**. Exit conditions require numbers to be *measured and recorded*;
missing a target requires a written cause analysis in the ledger, not a phase failure.

- **T1 (tax):** each concurrent structure, single-threaded, within **10%** of sequential `RoaringBitmap` on the P6 suite.
- **T2 (scaling):** read-heavy (95/5) throughput increases monotonically to 8 threads and reaches **≥4×** its own 1-thread throughput at 8 threads.
- **T3 (absolute):** sequential impl within explainable distance of the `roaring` crate; any gap **>2×** on any benchmark gets a written cause in the ledger.

### 0.3 Scope

**In scope:** a set of `u32` values. Operations: `insert`, `remove`, `contains`, `len`, `is_empty`,
`optimize`, `and`/`or` (+ `BitAnd`/`BitOr` operator impls). Three concurrent structures (P7, P8a, P8b).
Benchmark suite, scaling harness, comparative writeup.

**Out of scope — do NOT build, even if it seems easy:** serialization, iterators, `xor`/`and_not`,
`rank`/`select`, SIMD, `no_std`, shrink-to-fit policies, custom allocators, `loom` model checking,
cached global cardinality. Scope creep is a plan deviation.

---

## 1. Standing Conventions — read this section fully at the start of EVERY session

**1.1 API mirrors std collections.** Method names, signatures, and return semantics follow
`HashSet`/`BTreeSet` conventions:

| Method | Returns | Semantics |
|---|---|---|
| `insert(x) -> bool` | `true` iff value was newly added |
| `remove(x) -> bool` | `true` iff value was present and removed |
| `contains(x) -> bool` | membership |
| `len() -> u64` | cardinality |
| `is_empty() -> bool` | `len() == 0` |

**1.2 No `Result` on total operations.** Every public op above is total over all `u32` inputs.
No panics on any input value (an internal index panic is a bug, not a contract).

**1.3 Two-tier comment policy.** Never comment *what* the code does. A brief *why* comment is
**mandatory** at: every atomic memory `Ordering` (state the invariant the ordering protects);
every magic number (`4096`, `8192`, `len = count − 1`, shard mask, default shard count); every
deliberately non-obvious choice (e.g., clone-under-read-lock-merge-outside). Everything else: no comment.

**1.4 Essential vs. accidental complexity.** Essential complexity (roaring container algorithms,
conversion heuristics, RCU pattern, epoch reclamation, memory orderings) must **not** be simplified
away — it is the point of the project. Accidental complexity is banned:
- No trait objects. `Container` is an enum (rationale in §2.3).
- No trait abstractions until ≥2 concrete implementations force one. (Exception: a bench-local helper trait inside a benchmark file is permitted.)
- No generics over shard strategy, no speculative config, no feature flags.
- No `unsafe`, with one carve-out: where a dependency's API requires it (`crossbeam-epoch` `Shared::deref` / `defer_destroy`). Every `unsafe` block carries a why-comment stating the invariant that makes it sound.
- Prefer the 3-line fix over the 40-line mechanism. When in doubt, write the simplest thing that passes the test.

**1.5 Quality gates — required for every phase's exit:**
```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
`#[allow(...)]` only at the specific site, with a why-comment.

**1.6 Testing doctrine.** Every new operation lands **in the same phase** with: (a) unit tests for
edge cases, (b) proptest invariants, (c) a differential test against the `roaring` crate where the
operation exists there (import alias: `use roaring::RoaringBitmap as RefBitmap;`). The sequential
`RoaringBitmap` exposes `#[doc(hidden)] pub fn assert_invariants(&self)` (checks the invariant
table in §2.3/§2.5) for use from integration tests.

**1.7 Toolchain & dependencies.** Stable Rust, edition 2021, no MSRV commitment. Dependencies are
added **in the phase that first needs them**, with caret requirements; versions listed per phase are
known-good minimums — use the latest compatible release at implementation time.

**1.8 Session protocol (one phase per session):**
1. Read this file fully.
2. Open `commit.md`; find the **first unchecked phase**. Never start P(k+1) while P(k) is unchecked.
3. Verify the phase's Preconditions (run the listed commands).
4. Implement exactly what the phase prescribes. Nothing more.
5. Run the phase's Exit Gate. All commands must pass.
6. In `commit.md`: tick the checkbox, append a Commit History entry (template is in that file), fill any required ledger table, record deviations if any.
7. `git commit` with message `P<n>: <imperative summary>` (e.g., `P2: bitmap container + array<->bitmap conversion`).

---

## 2. Architecture (normative)

### 2.1 Final repository layout

```
Cargo.toml
AGENTS.md            (this file — static)
commit.md            (living ledger)
README.md            (P9)
.gitignore           (target/, bench-results/)
src/
  lib.rs
  bitmap.rs          (RoaringBitmap + split/join helpers + datasets module)
  container/
    mod.rs           (Container enum, dispatch, conversion policy, normalize, kernels)
    array.rs
    bitmap.rs
    run.rs
  concurrent/
    mod.rs           (P7)
    sharded.rs       (P7: ConcurrentRoaringBitmap)
    snapshot.rs      (P8a: SnapshotRoaringBitmap)
    epoch.rs         (P8b: EpochRoaringBitmap)
  bin/
    scaling.rs       (P7: multithread scaling harness)
benches/
  sequential.rs      (criterion)
tests/
  smoke.rs           (P0)
  differential.rs    (P4+)
  concurrent_stress.rs (P7+)
scripts/
  plot.py            (P9)
bench-results/       (gitignored CSV output)
docs/graphs/         (P9 PNGs, committed)
```

### 2.2 Value model

A `u32` value `x` splits into a container **key** and a **low** part:
`fn split(x: u32) -> (u16, u16)` returns `((x >> 16) as u16, x as u16)`;
`fn join(key: u16, low: u16) -> u32` inverts it. Both live in `bitmap.rs` with unit tests
covering `0`, `u32::MAX`, and the `0x0001_0000` boundary.

### 2.3 The `Container` enum

```rust
pub enum Container {
    Array(ArrayContainer),   // sorted Vec<u16>, cardinality ≤ 4096
    Bitmap(BitmapContainer), // Box<[u64; 1024]>, cardinality > 4096 (up to 65536)
    Run(RunContainer),       // sorted, non-overlapping, non-adjacent runs
}
```

Design decisions (all final):
- **Enum, not trait object.** Containers convert between types constantly; enum conversion is a
  reassignment, dispatch is a jump table with no vtable indirection or boxing per element.
- **Bitmap words are `Box<[u64; 1024]>`** — an inline 8 KiB array would make *every* `Container`
  8 KiB. Boxed, the enum stays pointer-sized-ish for all variants.
- **Cardinality is `u32` at container level** — a full container holds 65 536 values, which does
  not fit in `u16`. (Mandatory why-comment at the field.)
- **All containers and `Container` derive `Clone`** from day one — the P8 RCU write path clones
  shards; retrofitting `Clone` later is churn.
- **Conversion decisions live ONLY in `Container`'s methods** (`insert`/`remove`/`optimize`), never
  inside `ArrayContainer`/`BitmapContainer`/`RunContainer`. Leaf types know their own representation;
  the enum owns representation *policy*.
- `Container` dispatches: `insert(u16) -> bool`, `remove(u16) -> bool`, `contains(u16) -> bool`,
  `cardinality() -> u32`, `is_empty() -> bool`, `num_runs() -> u32`, `optimize(&mut self)`,
  and (P5) `and(&self, &Container) -> Container`, `or(&self, &Container) -> Container`.

**Invariant table** (enforced by `assert_invariants`, exercised by proptests):

| Structure | Invariants |
|---|---|
| ArrayContainer | `values` strictly increasing (sorted, no duplicates); `len ≤ 4096` when stored inside a `RoaringBitmap` |
| BitmapContainer | cached `cardinality` equals popcount of words; `4096 < cardinality ≤ 65536` when stored |
| RunContainer | runs sorted by `start`; non-overlapping; **non-adjacent** (next.start > prev_end + 1, else they'd be one run); `len` field means `count − 1`; cached cardinality = Σ(len + 1) |
| Container in a RoaringBitmap | never empty |

### 2.4 Representation & conversion rules (normative)

| Trigger | Rule | Why |
|---|---|---|
| `Array` insert when `cardinality == 4096` and value absent | Convert to `Bitmap` **first**, then insert | 4096 × 2 B = 8192 B = bitmap size; array wins strictly below, bitmap at/above. Pre-converting avoids growing the Vec to 4097 then copying. |
| `Bitmap` remove succeeds and `cardinality == 4096` | Convert to `Array` | Crosses the threshold by exactly 1, so `==` is the correct check. |
| Any `Run` mutation | Mutate in place (see P3); afterwards, if `4 × num_runs > 8192`, convert to `Bitmap` | A run list bigger than the bitmap has lost its reason to exist. |
| Run creation | **Only** via `optimize()` — never during `insert` | Per-op run detection is O(container) and would poison insert benchmarks. |
| New key in `RoaringBitmap` | Always create an `ArrayContainer` | Every container starts life small. |

**`Container::optimize` — the pinned CRoaring smallest-of-three heuristic.**
Compute the byte size of each valid representation of the current contents:
- array: `2 × cardinality` (valid only if `cardinality ≤ 4096`, else excluded)
- bitmap: `8192`
- run: `4 × num_runs()`

Convert **iff** the smallest candidate is *strictly smaller* than the current representation's
size; ties keep the current representation (deterministic, avoids thrashing).

**`num_runs` per representation:**
- Array: `1 + count of i where values[i+1] > values[i] + 1` (0 if empty).
- Run: `runs.len()`.
- Bitmap: fold over words with `prev` = previous word (0 for the first):
  `runs += popcount(w & !(w << 1)); if (w & 1) == 1 && (prev >> 63) == 1 { runs -= 1 }`.
  Why it works (one-line comment required): `w & !(w << 1)` marks each bit that is set whose
  lower neighbor is clear — i.e., each run start within the word — and the correction removes
  the double-count when a run spans a word boundary.

### 2.5 `RoaringBitmap` (sequential, the P0–P6 deliverable)

```rust
#[derive(Clone, Default)]
pub struct RoaringBitmap {
    containers: Vec<(u16, Container)>, // sorted by key, keys unique
}
```

- Key lookup: `binary_search_by_key(&key, |(k, _)| *k)`; on `Err(idx)`, insert new entry at `idx`.
- `remove`: if the container's remove returns `true` and the container `is_empty()`, remove the
  `(key, container)` entry — the "never empty" invariant.
- `len() -> u64`: sum of container cardinalities. O(#containers), each O(1).
  **There is NO cached global count** — mandatory why-comment: a global counter would become the
  single cross-shard contention point in every concurrent variant (P7/P8).
- `optimize(&mut self)`: call `optimize` on every container.
- This exact `Vec<(u16, Container)>` shape is what P7 shards — sharding is "partition by key,"
  not a redesign.

### 2.6 Concurrency lineup (built in P7/P8; overview here, details in phases)

| Type | Reads | Writes | Reclamation |
|---|---|---|---|
| `ConcurrentRoaringBitmap` (P7) | per-shard `RwLock` read | per-shard `RwLock` write | n/a |
| `SnapshotRoaringBitmap` (P8a) | lock-free `ArcSwap::load` | per-shard writer `Mutex` + clone-and-swap | `Arc` refcount |
| `EpochRoaringBitmap` (P8b) | lock-free `Atomic` load under epoch pin | per-shard writer `Mutex` + clone-and-swap | `crossbeam_epoch::defer_destroy` |

Shared shard scheme (all three types):
- Shard count is a **power of two**, default **64**; constructor `with_shard_count(n)` asserts `n.is_power_of_two()`.
- `shard_index = (key as usize) & (num_shards - 1)` — **low bits of the key**, mandatory
  why-comment: real-world integer sets are typically clustered, so consecutive keys must
  round-robin across shards; taking *high* bits would pile a clustered dataset into shard 0.
- Shard payload is a whole sequential `RoaringBitmap` (maximal code reuse; each shard simply sees
  only its subset of keys).
- Cross-shard operations (`len`, `snapshot`, `and`, `or`) are **per-shard-atomic, not globally
  linearizable** — a documented semantic with a mandatory comment at each site.

---

## 3. Phases

Every phase below states: **Preconditions** (verify before touching code), **Deliverables**,
**Prescribed design** (signatures + logic in plan language — precise enough to implement without
interpretation), **Tests**, and an **Exit Gate** (mechanical; all commands must pass, all artifacts
must exist, `commit.md` must be updated per §1.8 before the phase counts as done).

---

### P0 — Repository scaffold & harness

**Preconditions:** empty directory, `git init` done, `AGENTS.md` + `commit.md` present at root.

**Deliverables:**
- `cargo init --lib`, crate name `concurrent_roaring`, edition 2021.
- Directory/file skeleton per §2.1 (sequential subset only: `src/lib.rs`, `src/bitmap.rs`,
  `src/container/{mod,array,bitmap,run}.rs`, `benches/sequential.rs`, `tests/smoke.rs`).
  Module files may be empty except for a one-line `//!` doc comment; `lib.rs` declares
  `pub mod container;` and `pub mod bitmap;`; `container/mod.rs` declares its three submodules.
- `Cargo.toml` dev-dependencies: `criterion = "0.5"`, `proptest = "1"`, `roaring = "0.10"`,
  `rand = "0.9"` (known-good minimums; use latest compatible). Plus:
  ```toml
  [[bench]]
  name = "sequential"
  harness = false
  ```
- `benches/sequential.rs`: criterion boilerplate with a single placeholder
  `bench_function("placeholder", |b| b.iter(|| std::hint::black_box(1 + 1)))` — replaced in P6.
- `tests/smoke.rs`: one test that links the crate and passes.
- `.gitignore`: `target/`, `bench-results/`.

**Exit Gate:** §1.5 gates + `cargo bench --no-run` compiles. Tree matches the skeleton. `commit.md` updated.

---

### P1 — `ArrayContainer` (+ `Container` enum introduced)

**Preconditions:** P0 checked; §1.5 gates green on `main`.

**Prescribed design** — `src/container/array.rs`:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArrayContainer { values: Vec<u16> }
```

| Signature | Logic (plan language) |
|---|---|
| `pub fn new() -> Self` | empty Vec |
| `pub fn cardinality(&self) -> u32` | `values.len() as u32` — no cached field; `Vec::len` is already O(1) |
| `pub fn is_empty(&self) -> bool` | delegate |
| `pub fn contains(&self, v: u16) -> bool` | `values.binary_search(&v).is_ok()` |
| `pub fn insert(&mut self, v: u16) -> bool` | `binary_search`; on `Err(idx)` → `values.insert(idx, v)`, return `true`; on `Ok` → `false` |
| `pub fn remove(&mut self, v: u16) -> bool` | `binary_search`; on `Ok(idx)` → `values.remove(idx)`, return `true`; on `Err` → `false` |
| `pub fn num_runs(&self) -> u32` | per §2.4 array formula |
| `pub(crate) fn as_slice(&self) -> &[u16]` | for conversion/kernel code in later phases |

`src/container/mod.rs`: define `pub enum Container` with **only** the `Array` variant for now
(variants are added in P2/P3 — growing a private-crate enum is a non-breaking internal change).
Implement dispatching `insert`/`remove`/`contains`/`cardinality`/`is_empty`/`num_runs` by matching.

**Tests:**
- Unit: insert/remove/contains at `0` and `65535`; duplicate insert returns `false`; remove of
  absent returns `false`; interleaved sequence keeps sortedness.
- Proptest `array_matches_btreeset`: model = `BTreeSet<u16>`; random sequence of ≤1024 ops
  (insert/remove/contains over full `u16` domain) applied to both; assert every op's return value
  matches the model, and final cardinality + membership of all model elements + 64 random probes match.
- Proptest invariant: after any op sequence, `values` is strictly increasing.

**Exit Gate:** §1.5 gates; the named proptests exist and pass. `commit.md` updated.

---

### P2 — `BitmapContainer` + array↔bitmap conversion

**Preconditions:** P1 checked; gates green.

**Prescribed design** — `src/container/bitmap.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapContainer {
    words: Box<[u64; 1024]>, // why boxed: see §2.3
    cardinality: u32,        // why u32: full container = 65536 > u16::MAX
}
```

| Signature | Logic |
|---|---|
| `pub fn new() -> Self` | zeroed words, cardinality 0 |
| `pub fn from_array(a: &ArrayContainer) -> Self` | set bit for each value; cardinality = a.cardinality() |
| `pub fn to_array(&self) -> ArrayContainer` | `debug_assert!(cardinality <= 4096)`; for each word, drain set bits via `trailing_zeros` loop (`while w != 0 { bit = w.trailing_zeros(); push(word_idx*64 + bit); w &= w - 1; }`), pushing in order — result is sorted by construction |
| `pub fn contains(&self, v: u16) -> bool` | `i = (v >> 6) as usize; mask = 1u64 << (v & 63);` test |
| `pub fn insert(&mut self, v: u16) -> bool` | read old word, OR mask in; `added = old & mask == 0`; `cardinality += added as u32`; return added |
| `pub fn remove(&mut self, v: u16) -> bool` | mirror of insert with AND-NOT |
| `pub fn cardinality(&self) -> u32` / `is_empty` | cached field |
| `pub fn num_runs(&self) -> u32` | §2.4 bitmap bit-trick, with the one-line why-comment |
| `pub(crate) fn words(&self) -> &[u64; 1024]` | for P5 kernels |

`Container` grows the `Bitmap` variant. **Conversion policy lands here, in `Container::insert` /
`Container::remove`** per §2.4 rows 1–2:
- `Container::insert`, `Array` arm: if `cardinality() == 4096 && !contains(v)` → replace self with
  `Bitmap(BitmapContainer::from_array(..))`, then insert into the bitmap.
- `Container::remove`, `Bitmap` arm: if remove returned `true` and `cardinality() == 4096` →
  replace self with `Array(bitmap.to_array())`.

**Tests:**
- Proptest cross-representation agreement: random `BTreeSet<u16>` (size 0..=8192); build a
  `BitmapContainer` (always) and an `ArrayContainer` (when ≤4096); assert `contains` agrees on all
  members + 256 random probes, cardinalities agree; `to_array(from_array(a)) == a`.
- Proptest thresholds through `Container`: insert 5000 distinct values → variant is `Bitmap`,
  cardinality 5000; remove down to 4096 → variant is `Array`; membership preserved across both conversions.
- Unit: `num_runs` on hand-built words including a run spanning a word boundary (the `−1` correction path).

**Exit Gate:** §1.5 gates; named proptests pass. `commit.md` updated.

---

### P3 — `RunContainer` + the smallest-of-three `optimize`

**Preconditions:** P2 checked; gates green.

**Prescribed design** — `src/container/run.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run { pub start: u16, pub len: u16 } // len = count - 1: a full 65536-value run must be representable

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunContainer { runs: Vec<Run>, cardinality: u32 }
```

**All boundary arithmetic in `u32`** (`start as u32 + len as u32`) — mandatory why-comment:
`start + len` can equal 65535 and intermediate `+1` comparisons overflow `u16`.

| Signature | Logic |
|---|---|
| `pub fn contains(&self, v: u16) -> bool` | `idx = runs.partition_point(|r| r.start <= v)`; if `idx == 0` → false; else check `v as u32 <= runs[idx-1].start as u32 + runs[idx-1].len as u32` |
| `pub fn insert(&mut self, v: u16) -> bool` | locate candidate as above. Cases: (1) already inside a run → `false`. (2) `v == prev_end + 1`: extend prev's `len`; if now adjacent to next run (`next.start == v as u32 + 1`), merge next into prev (add `next.len + 1` to len, remove next). (3) `v + 1 == next.start`: decrement next's start, increment len. (4) otherwise insert `Run { start: v, len: 0 }` at the partition point. Increment cardinality on any true path; return `true`. |
| `pub fn remove(&mut self, v: u16) -> bool` | locate containing run or return `false`. Cases: (1) run has `len == 0` → remove the run. (2) `v == start` → `start += 1; len -= 1`. (3) `v == end` → `len -= 1`. (4) interior → split: truncate current run to `[start, v-1]`, insert `[v+1, old_end]` after it. Decrement cardinality; return `true`. |
| `pub fn cardinality(&self) -> u32` / `is_empty` | cached field (why cached: Σ(len+1) is O(runs), convention demands O(1)) |
| `pub fn num_runs(&self) -> u32` | `runs.len() as u32` |
| `pub fn from_array(&ArrayContainer) -> Self` | single pass over sorted values, extending/starting runs |
| `pub fn from_bitmap(&BitmapContainer) -> Self` | scan words for run starts/ends (any correct linear scan) |
| `pub fn to_array(&self) -> ArrayContainer` / `to_bitmap(&self) -> BitmapContainer` | expand runs in order |
| `pub(crate) fn runs(&self) -> &[Run]` | for P5 kernels |

`Container` grows the `Run` variant; dispatch extended. Two policy additions in `Container` per §2.4:
- After any `Run`-arm mutation: if `4 * num_runs > 8192` → convert to `Bitmap`.
- `pub fn optimize(&mut self)`: compute candidate sizes exactly per §2.4; convert iff strictly smaller; ties keep current.

**Tests:**
- Proptest tri-representation agreement: random set (generated as a union of 0..=64 random ranges,
  so runs actually occur); build array (when ≤4096) / bitmap / run from it; `contains` on all members
  + 256 probes and cardinality agree across every valid representation; all `to_*`/`from_*`
  round-trips are identity.
- Proptest run mutation vs `BTreeSet<u16>` model, values drawn from a union of small ranges (forces
  extend/merge/split paths); return values and final state match; invariants (sorted, non-overlapping,
  non-adjacent, cached cardinality correct) hold after every op.
- Proptest `optimize`: idempotent (second call changes nothing); never increases representation size;
  membership and cardinality unchanged.

**Exit Gate:** §1.5 gates; named proptests pass. `commit.md` updated.

---

### P4 — `RoaringBitmap` (top level) + differential testing

**Preconditions:** P3 checked; gates green.

**Prescribed design** — `src/bitmap.rs`, exactly per §2.5 plus:

| Signature | Logic |
|---|---|
| `pub fn new() -> Self` (+ `Default`) | empty vec |
| `pub fn insert(&mut self, x: u32) -> bool` | `(key, low) = split(x)`; binary search; `Ok(i)` → `containers[i].1.insert(low)`; `Err(i)` → insert `(key, Container::Array(single-value array))` at `i`, return `true` |
| `pub fn remove(&mut self, x: u32) -> bool` | search; `Err` → `false`; `Ok(i)` → `removed = containers[i].1.remove(low)`; if removed and container `is_empty()` → `containers.remove(i)`; return removed |
| `pub fn contains(&self, x: u32) -> bool` | search; `Err` → `false`; else delegate |
| `pub fn len(&self) -> u64` | Σ cardinality (no cached global count — why-comment per §2.5) |
| `pub fn is_empty(&self) -> bool` | `containers.is_empty()` (valid because containers are never empty) |
| `pub fn optimize(&mut self)` | for-each container optimize |
| `#[doc(hidden)] pub fn assert_invariants(&self)` | full §2.3/§2.5 invariant table: keys sorted+unique; no empty container; per-container invariants incl. cached cardinalities recomputed and compared |

**Tests** — `tests/differential.rs`:
- `use roaring::RoaringBitmap as RefBitmap;`
- Proptest `matches_roaring_crate`: op sequence ≤3000 of `{Insert, Remove, Contains}` with value
  strategy `prop_oneof![0u32..500_000, any::<u32>()]` (the narrow arm forces dense keys →
  container conversions; the wide arm exercises sparse keys). Apply to ours and `RefBitmap`
  simultaneously; assert **every op's return value matches**; finally assert `len()` matches and
  every 64th inserted value's membership matches; run `assert_invariants` at the end.
- Same proptest variant with `optimize()` called mid-sequence and at the end — membership/len unchanged.
- Unit: `0`, `u32::MAX`, key-boundary values `0xFFFF`, `0x1_0000`.

**Exit Gate:** §1.5 gates; `matches_roaring_crate` passes. `commit.md` updated.

---

### P5 — Set operations (`and` / `or`)

**Preconditions:** P4 checked; gates green.

**Prescribed design.** Top level (`bitmap.rs`):
`pub fn and(&self, other: &Self) -> Self` and `pub fn or(&self, other: &Self) -> Self` do a
two-pointer merge-join over the two sorted key vecs: `or` emits a (cloned) container for keys in
exactly one side and the kernel result for keys in both; `and` emits kernel results only for keys
in both, **dropping empty results** (disjoint containers). Then implement
`impl BitAnd<&RoaringBitmap> for &RoaringBitmap` / `BitOr` (+`BitAndAssign`/`BitOrAssign` if
trivial) as one-line delegations.

Container level (`container/mod.rs`): `pub(crate) fn and(&self, other: &Container) -> Container`,
`or` likewise. Every kernel result passes through a private
`fn normalize(c: Container) -> Container` that applies §2.4 policy: `Bitmap` with cardinality
≤4096 → `Array`; `Run` with `4×num_runs > 8192` → `Bitmap`. Implement 6 kernels per op; the
mirrored pairs delegate with arguments swapped.

**Kernel matrix (pinned algorithms):**

| Pair | AND | OR |
|---|---|---|
| Array·Array | two-pointer intersection → Array (result ≤ min card, always legal) | if `ca+cb ≤ 4096`: two-pointer merge-dedup → Array; else: bitmap from a, insert b's values, normalize (dedup may drop it back ≤4096) |
| Array·Bitmap | keep each array value present in bitmap → Array (result ≤ array card) | clone bitmap, set each array value with cardinality maintenance → Bitmap (card ≥ bitmap's > 4096, stays legal) |
| Array·Run | keep each array value where `run.contains` → Array | clone run, run-insert each array value, normalize |
| Bitmap·Bitmap | zip words with `&`, summing popcounts → normalize | zip words with `\|`, summing popcounts → Bitmap |
| Bitmap·Run | zeroed result words; for each run, copy source words masked to `[start, end]` (full words copied whole; edge words get start/end masks), popcount during copy → normalize | clone bitmap; for each run, set the word range to 1s via the same edge-mask scheme; recount cardinality with one popcount pass at the end → Bitmap |
| Run·Run | two-pointer over both run lists: `lo = max(starts)`, `hi = min(ends)`; if `lo ≤ hi` emit `Run{lo, hi-lo}`; advance the list whose run ends first; accumulate cardinality → normalize | two-pointer interval merge; merge when overlapping **or adjacent** (`next.start ≤ cur_end + 1`) → normalize |

(All run boundary math in `u32`, per P3's rule.)

**Tests** — extend `tests/differential.rs`:
- Proptest `setops_match_roaring_crate`: generate two random bitmaps with the P4 strategy
  (optimize one of them to force Run participation); compute `ours_and = a.and(&b)` vs
  `ref_and = &ra & &rb`, same for `or`. **Equality check (pinned trick):**
  `ours.len() == ref.len()` **and** every element of `ref.iter()` is contained in ours —
  subset + equal cardinality ⟹ set equality, no iterator needed on our side.
- Algebraic proptests on sampled probes: `a.and(b) ⊆ a`; `a ⊆ a.or(b)`; `and`/`or` commutative in `len`.
- `assert_invariants` on every result.

**Exit Gate:** §1.5 gates; `setops_match_roaring_crate` passes. `commit.md` updated.

---

### P6 — Sequential baseline benchmarks (Baseline B, and the reference point for Baseline A)

**Preconditions:** P5 checked; gates green.

**Prescribed design.**

Datasets — `#[doc(hidden)] pub mod datasets` inside `src/bitmap.rs` (hidden public: shared by the
criterion bench, the P7 scaling binary, and tests; benches cannot import from `tests/`). All
generators are deterministic with **pinned seeds** so every phase and every agent measures the
same data:

| Generator | Definition |
|---|---|
| `pub fn dense() -> Vec<u32>` | `(0..1_000_000).collect()` — long runs / full bitmap containers |
| `pub fn sparse() -> Vec<u32>` | 1,000,000 draws of `rng.random::<u32>()`, `StdRng::seed_from_u64(0xDEAD_BEEF)`; duplicates permitted (identical input to every impl is what matters) — ~15 values per key → array containers |
| `pub fn clustered() -> Vec<u32>` | `StdRng::seed_from_u64(0xC0FF_EE)`; 1,000 bases uniform in `0..=u32::MAX - 1_000`; each contributes `base..base + 1_000` — mixture of run/array containers |
| `pub fn probes(data: &[u32]) -> Vec<u32>` | 500,000 hits sampled (with replacement) from `data` + 500,000 uniform random `u32`, concatenated then shuffled, `StdRng::seed_from_u64(0xFEED_BEEF)` |

`benches/sequential.rs` replaces the P0 placeholder. Every group benchmarks **ours and `RefBitmap`
side by side on identical inputs**:

- `build/{dense,sparse,clustered}`: insert every value into a fresh structure.
- `contains/{dense,sparse,clustered}`: pre-built structure, `optimize()`d (call the ref crate's
  run-optimize equivalent if it exposes one — verify on docs.rs; if it doesn't, note that in the
  ledger), then iterate the probe set with `black_box`.
- `remove/clustered`: `iter_batched` with a cloned pre-built structure per batch; remove 100,000
  values sampled from the dataset (seed `0xBADC_0DE`).
- `and/{dense×sparse, clustered×clustered, sparse×sparse}` and the same three for `or`:
  pre-built, optimized pairs.

**Exit Gate:** §1.5 gates; `cargo bench` completes end-to-end; **Benchmark Ledger — P6 table in
`commit.md` filled with the measured numbers** (this table is Baseline B and the sequential
reference for Baseline A); any >2× gap vs `RefBitmap` gets a one-paragraph cause analysis (T3).
`commit.md` updated.

---

### P7 — `ConcurrentRoaringBitmap`: sharded `RwLock` (Wave 1)

**Preconditions:** P6 checked; ledger has the sequential baseline table.

**Dependencies added now:** `parking_lot = "0.12"` (why over std: no lock poisoning — no
`unwrap()` noise on every access — and a faster uncontended path, which is exactly what the tax
measurement stresses).

**Prescribed design** — `src/concurrent/sharded.rs` (+ `concurrent/mod.rs`, re-export from `lib.rs`):

```rust
pub struct ConcurrentRoaringBitmap {
    shards: Box<[parking_lot::RwLock<RoaringBitmap>]>,
    mask: usize, // num_shards - 1; power of two per §2.6
}
```

| Signature | Logic |
|---|---|
| `pub fn new() -> Self` | 64 shards (default per §2.6) |
| `pub fn with_shard_count(n: usize) -> Self` | `assert!(n.is_power_of_two())`; n shards |
| `fn shard(&self, key: u16) -> &RwLock<RoaringBitmap>` | `&self.shards[(key as usize) & self.mask]` (why low bits: §2.6 comment) |
| `pub fn insert(&self, x: u32) -> bool` | split → shard → `write()` → delegate to inner `RoaringBitmap::insert(x)` (inner sees full u32; it only ever holds this shard's keys) |
| `pub fn remove(&self, x: u32) -> bool` / `contains` | same shape; `contains` takes `read()` |
| `pub fn len(&self) -> u64` | fold over shards, read-locking **one at a time** (why: never hold all locks; consequence: not linearizable across shards — mandatory comment) |
| `pub fn is_empty(&self) -> bool` | all shards empty, same one-at-a-time discipline |
| `pub fn snapshot(&self) -> RoaringBitmap` | for each shard: clone under `read()`, drop lock, merge outside via a cheap key-vec concatenation (shards partition the key space by `key & mask`, so containers can be collected and sorted by key once at the end — no kernel merging needed). Comment the per-shard-atomic semantics. |
| `pub fn and(&self, other: &Self) -> RoaringBitmap` / `or` | `self.snapshot()` op `other.snapshot()` — deliberately simple; no cross-object lock ordering exists, so no deadlock is possible (mandatory why-comment) |
| `pub fn optimize(&self)` | per shard: `write()` → inner optimize |

**Tests** — `tests/concurrent_stress.rs`:
1. **Disjoint-partition (lost-update detector):** 8 threads; thread `t` inserts every value `v` of
   the clustered dataset where `v % 8 == t`; join; assert `len()` equals the dataset's unique count
   (compute once via a `BTreeSet`) and 10,000 sampled values are all present; `snapshot().assert_invariants()`.
2. **Contended smoke:** 4 writer threads (seeded random insert/remove over `0..2_000_000`) + 4
   reader threads (contains over the same range), all spinning until a 2-second deadline
   (`std::time::Instant`); no panics; final snapshot passes `assert_invariants`.

`loom` is explicitly out of scope (§0.3): the cfg plumbing it requires is accidental complexity;
risk is covered by the simplicity of the lock pattern plus these stress tests.

**Scaling harness** — `src/bin/scaling.rs` (this binary is extended, not rewritten, in P8):
- Registers structures behind a match on name: `sequential` (single-thread only) and `sharded`.
- Workloads: `read95` (95% contains / 5% insert), `mixed50`, `write95` (5% contains / 95% insert).
- Thread counts: `{1, 2, 4, 8, 16}` clamped to `std::thread::available_parallelism()`.
- Protocol per (structure, workload, threads) cell: pre-populate with `clustered()`; spawn threads;
  each thread owns `StdRng::seed_from_u64(0x5CA1_AB1E ^ thread_index as u64)`; all threads wait on a
  `std::sync::Barrier`; each performs 2,000,000 ops (contains draws a random element of the dataset
  by index; insert draws a uniform random `u32`); wall time from barrier-release to last join.
- Output: append rows `structure,workload,threads,total_ops,seconds,mops` to
  `bench-results/scaling.csv` (create dir/file with header if absent) and print an aligned table.

**Concurrency-tax bench (Baseline A):** add a `tax` group to `benches/sequential.rs` comparing
`RoaringBitmap` vs `ConcurrentRoaringBitmap` **single-threaded** on `build/clustered` and
`contains/clustered`. The two APIs differ (`&mut self` vs `&self`), so define a tiny **bench-local**
trait inside the bench file (permitted by §1.4) with `insert(&mut self, u32) -> bool` /
`contains(&self, u32) -> bool`, implemented for both.

**Exit Gate:** §1.5 gates; both stress tests pass under `--release`; `cargo run --release --bin
scaling` produces `bench-results/scaling.csv` covering `sequential`(threads=1) + `sharded`(all
thread counts) × all workloads; **Ledger — P7 filled** with the tax table and scaling table; if tax
>10% (T1), a written cause analysis accompanies it. `commit.md` updated.

---

### P8 — Lock-free reads (Wave 2): P8a `arc-swap`, then P8b `crossbeam-epoch`

**Preconditions:** P7 checked; scaling CSV exists.

Both types share one pattern — **single-writer RCU per shard**: readers never take a lock and load
an immutable snapshot pointer; writers serialize on a per-shard mutex, clone the current snapshot,
mutate the clone, then atomically publish it. Mandatory why-comment at each writer mutex: two
concurrent read-copy-update writers on one shard would each clone the same base and the second
publish would silently discard the first's update (lost update); the mutex serializes
read-modify-write, and readers are unaffected by it.

The clone-per-write cost is the deliberate tradeoff being measured (read-optimized structures);
higher shard counts shrink the cloned unit — mention `with_shard_count(256)` in the ledger analysis
if write-heavy numbers look pathological.

#### P8a — `SnapshotRoaringBitmap` (`src/concurrent/snapshot.rs`)

**Dependency added now:** `arc-swap = "1"`.

```rust
struct Shard {
    current: arc_swap::ArcSwap<RoaringBitmap>,
    write: parking_lot::Mutex<()>, // single-writer RCU serialization (comment per above)
}
pub struct SnapshotRoaringBitmap { shards: Box<[Shard]>, mask: usize }
```

- Constructors as in P7. Each shard starts as `ArcSwap::from_pointee(RoaringBitmap::new())`.
- `contains(&self, x)`: split → shard → `let g = shard.current.load();` → `g.contains(x)`.
  `load()` returns a cheap `Guard` (no full `Arc` clone on the hot path) — why-comment.
- `insert(&self, x) -> bool` (remove is the mirror): take `write.lock()`; `let cur =
  shard.current.load_full();` if the op would be a no-op (`cur.contains(x)` already true for
  insert / false for remove) return early **without cloning** (why-comment: skip the O(shard)
  clone when nothing changes — this is what keeps duplicate-heavy workloads sane); else
  `let mut next = (*cur).clone();` apply the op; `shard.current.store(Arc::new(next));` return `true`.
- `len` / `is_empty` / `snapshot` / `and` / `or` / `optimize`: same shapes as P7 (`snapshot` just
  loads each shard's `Arc` — no locks at all; `optimize` goes through the write path).
- Reclamation: `Arc` refcounting — readers holding a `Guard`/`Arc` keep the old snapshot alive; it
  frees when the last reference drops. No further machinery needed (why-comment).

#### P8b — `EpochRoaringBitmap` (`src/concurrent/epoch.rs`)

**Dependency added now:** `crossbeam-epoch = "0.9"`.

```rust
struct Shard {
    current: crossbeam_epoch::Atomic<RoaringBitmap>,
    write: parking_lot::Mutex<()>,
}
pub struct EpochRoaringBitmap { shards: Box<[Shard]>, mask: usize }
```

- Read path (`contains`): `let guard = crossbeam_epoch::pin();` →
  `let shared = shard.current.load(Ordering::Acquire, &guard);` →
  `let map = unsafe { shared.deref() };` → `map.contains(x)`.
  Required comments: **Acquire** pairs with the writer's Release store so a reader that sees the
  new pointer also sees the fully-built clone behind it; the **unsafe deref** is sound because the
  pointer is never null after construction and epoch pinning guarantees no `defer_destroy`ed
  snapshot is freed while this guard is live (the §1.4 unsafe carve-out — comment at every site).
- Write path (`insert`/`remove`, same no-op short-circuit as P8a): lock writer mutex; pin;
  `load(Acquire)`; deref; if no-op → return; clone; mutate; `let old = shard.current.swap(
  Owned::new(next), Ordering::Release, &guard);` → `unsafe { guard.defer_destroy(old) }`.
  Required comments: **Release** publishes the completed clone before the pointer becomes visible;
  `defer_destroy` is sound because after the swap no new reader can load `old`, and epoch GC waits
  out every reader pinned before the swap.
- `impl Drop for EpochRoaringBitmap`: for each shard, swap in a null `Shared` and
  `unsafe { drop(old.into_owned()) }` if non-null (sound: `&mut self` in `Drop` proves no
  concurrent readers exist — comment).
- Remaining API mirrors P8a.

**Tests:** extend `tests/concurrent_stress.rs` — both P7 stress patterns must run against all
three concurrent types. Use a small local `macro_rules!` to stamp the suite per type (permitted:
it removes triplicated test bodies, which would be accidental complexity in the other direction).

**Scaling harness:** register `snapshot` and `epoch` in `scaling.rs`; rerun the **full matrix**
(all four structures × three workloads × all thread counts) into a fresh CSV.

**Exit Gate:** §1.5 gates; stress suite green for all three concurrent types under `--release`;
full-matrix `bench-results/scaling.csv` regenerated; **Ledger — P8 filled**: single-thread tax for
all three types (Baseline A) + the comparative scaling table + a short written reading of the
results (where lock-free reads beat `RwLock`, what write-heavy costs, per T1/T2). Tick P8a only
when P8a's structure passes everything; tick P8b likewise. `commit.md` updated.

---

### P9 — Comparative writeup, graphs, resume bullets

**Preconditions:** P8a and P8b checked; full-matrix CSV exists.

**Deliverables:**
- `scripts/plot.py` (matplotlib, stdlib `csv`): reads `bench-results/scaling.csv`; emits at
  minimum `docs/graphs/read_scaling.png` (mops vs. threads, one line per structure, `read95`) and
  `docs/graphs/write_impact.png` (same for `write95`). PNGs are committed.
- `README.md` with exactly these sections:
  1. **Overview** — what it is, one paragraph.
  2. **Design** — container model, thresholds, shard scheme, the three concurrency strategies (may reference this plan).
  3. **The degradation question** — the §0.1 two-baseline model, stated for a reader who hasn't seen this plan.
  4. **Methodology** — datasets, seeds, workloads, machine specs (CPU model, core count, RAM, OS, rustc version — record the actual box).
  5. **Results** — the graphs + ledger tables, with the T1/T2/T3 verdicts stated plainly.
  6. **Tradeoff analysis** — per structure: where it wins, where it loses, why (this is the interview section).
  7. **Limitations** — must include at minimum: cross-shard ops are per-shard-atomic, not linearizable; clone-per-write cost under write-heavy load; no serialization/iterators; single-machine benchmarks.
  8. **Future work** — honest next steps (e.g., run-aware kernels everywhere, adaptive shard counts, true CAS-based container mutation).
- **Resume bullets** — a `## Resume bullets` section at the bottom of `commit.md` with three
  filled-in variants (numbers from the ledger, not placeholders), e.g.:
  - "Built a concurrent Roaring bitmap in Rust; benchmarked sharded `RwLock` vs. RCU/epoch-based lock-free reads across read/write mixes, sustaining __× read-throughput scaling at 8 threads with __% single-threaded overhead vs. the sequential baseline."
  - One variant emphasizing the memory-reclamation work (epoch GC, memory orderings), one emphasizing measurement methodology.

**Exit Gate:** §1.5 gates; `python3 scripts/plot.py` runs clean; README complete with all eight
sections; ≥2 PNGs committed; resume bullets contain real measured numbers; **every checkbox in
`commit.md` ticked.**

---

## 4. Appendix — quick reference card

- **Thresholds:** array→bitmap at cardinality 4096 (pre-convert on insert of a 4097th distinct value); bitmap→array when remove lands cardinality exactly on 4096; run→bitmap when `4×num_runs > 8192`; run creation only via `optimize()` (smallest-of-three, strict, ties keep current).
- **Sizes:** array `2×card` B; bitmap `8192` B; run `4×num_runs` B.
- **Split:** `key = (x >> 16) as u16`, `low = x as u16`. **Shard:** `(key as usize) & (num_shards − 1)`, default 64 shards.
- **Bitmap num_runs fold:** `runs += popcount(w & !(w << 1)); if (w & 1) & (prev >> 63) == 1 { runs -= 1 }`.
- **Run semantics:** `Run { start, len }` covers `start ..= start + len` (count = len + 1); boundary math in `u32`.
- **Gates:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- **Seeds:** sparse `0xDEAD_BEEF` · clustered `0xC0FF_EE` · probes `0xFEED_BEEF` · remove-sample `0xBADC_0DE` · scaling per-thread `0x5CA1_AB1E ^ thread_idx`.
