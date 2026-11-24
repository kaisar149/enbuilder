# HANDOFF: Part 6 (edit propagation, diff, explainer). Living file, updated at each milestone.

## How to continue after an interruption
1. Unzip the latest `part6_snapshot_*.zip` over your `enbuilder` crate root (paths inside are relative to the crate root: `src/...`, `tests/...`).
2. Build note: the project `Cargo.toml` lists optional `wgpu`/`pollster`; offline sandboxes need a scratch Cargo.toml without `[dependencies]` and `gpu = []`.
3. `cargo test --test part6a` (dirty regions, incremental, locks), later `--test part6b` (diff, provenance, explainer), then `cargo test`.
4. Paste `00_SHARED_CONTEXT.md`, `ARCHITECTURE.md`, `PART_06_edit_propagation.md` (+ this file) into a new session and say "continue Part 6 from HANDOFF_PART6.md".

## Status
| Item (PART_06 "Build") | State |
|---|---|
| 1 Dirty-region tracking | DONE `src/dirty.rs` |
| 2 Incremental re-solve | DONE `src/incremental.rs` |
| 3 Diff engine + propagation front | DONE `src/diff.rs` (6b) |
| 4 Provenance + explainer | DONE `src/provenance.rs` (6b) |
| 5 Locks and pins | DONE `FieldStore::lock_cells/set_cells/fork` |
| ARCHITECTURE.md update | DONE ("Part 6" section appended) |
| Tests | part6a 23, part6b 13, full `cargo test` green in the AI sandbox (rustc 1.75, edition 2021 scratch manifest) |

**PART 6 IS COMPLETE.** Remaining: run `cargo test` on your machine (edition 2024 / wgpu feature were not exercised in the sandbox) and optionally the follow-ups below.

## What was added (files)
- `src/fields.rs`: locks (`lock_cells`, `unlock_cells`, `unlock_field`, `clear_locks`, `is_locked`, `locked_cells`, `locked_fields`), `set_cells` (user edit, bypasses locks, optional pin), `fork()` (independent copy with locks, empty history). Every `write`/`put` restores locked cells.
- `src/scheduler.rs`: `specs()`, `feedback_loops()`, `set_trace(Some(TraceCfg))`, `take_trace()`, types `TraceCfg`, `Trace` (ticks, per-field `front`, `touched` per (module, field), `events`, baseline), `ModuleEvent`. Tracing is off by default; behaviour with it off is unchanged.
- `src/dirty.rs`: `InfluenceSpec` (`uniform`, `earth_like`, per-field radius, downwind metres, wind field, cone), `Region` (cells, dist, border, union, ring_of), `grow_region` (Dijkstra on edge distances), `extend_downwind`, `downstream_fields`, `plan_dirty`, `DirtyPlan`.
- `src/incremental.rs`: `Edit`, `IncrementalConfig`, `resolve_edit` (one undo group; rolls back on error), `solve_full` (reference), `IncrementalReport`/`FieldReport`.
- `src/toyclimate.rs`: throwaway toy pipeline with exactly known causal structure (tests only).
- `tests/part6a.rs`: 23 tests.

## Key design decisions (record)
- **Honest status of "incremental":** modules still recompute every cell (they ignore `DirtyRegion`). The solve runs warm on a scratch fork of the store, then each field is committed ONLY inside its dirty region. Real benefit today: warm start (real energy stage: loop passes 24 cold -> 5-6 warm) and exact locality (outside the regions every value is bit-identical to before). `modules_honor_dirty` in the report is false; when modules learn to honour `DirtyRegion` the same API gets faster.
- **Widening:** per field, start at `radius x scale`, widen x1.5 until border change <= max(border_tol x peak, solver noise), up to `max_widen`.
- **Solver noise** = `loop_noise_factor x loop tolerance x max|field|` for fields a feedback loop watches (energy stage: ~0.63 K). A warm re-run of an UNCHANGED world moves temperature by ~0.49 K, so smaller differences are not signal.
- **Far-field guard (found by the real-physics test):** the global energy balance shifts ~2 K everywhere when one lake changes the planet's outgoing radiation. Radius alone cannot see that. Because the scratch solve is global we know every cell's change, so cells outside the region whose change exceeds the threshold are ADDED (`far_field_added`). This makes "incremental matches full within `max(border_tol x peak, noise)`" a guarantee, at the cost of locality when physics is global.
- **Edits of module-produced fields:** an UNPINNED edit is only a warm initial condition (the producer's answer wins). Use `Edit::pinned()` for an override (e.g. a frozen patch: pin `albedo`).
- **Locks** are weak Dirichlet conditions: enforced after every module write; a module can read `FieldStore::is_locked` to hold cells fixed inside its own iteration (the toy `heat` module does, and the test shows the cold lock cooling its neighbours).
- `undo` of an edit restores values but does NOT remove pins (locks are not in the history). Decision to revisit.

## 6b additions (files)
- `src/diff.rs` (ChangeSet, FieldDiff, frames), `src/provenance.rs` (Provenance, explain), `src/lib.rs` (mods and re-exports), `src/scheduler.rs` (trace tolerance floor, see ARCHITECTURE), `tests/part6b.rs`.
- Findings: temperature in the energy stage has NO propagation front (global balance, all cells at tick 3); the ice-albedo front does creep (albedo ticks 2, 4, 8). The toy `heat` module moves every cell by > 0.01 K (weak global mode), so the dynamic provenance marks it everywhere; sharp ground truth is `surface_albedo` marking exactly the patch.
- Explainer default `local_radius_m` is 6 x mean neighbour distance (12x was too large on the coarse level-3 sphere: nothing was ever Global).

## Follow-ups (not required for Part 6)
1. Let modules honour `DirtyRegion` (real speed-up with the same API).
2. Explainer: use trace ticks to require cause-before-effect; weight inputs by sensitivity instead of relative change.
3. Save `Provenance` (and pins) in the world file; make undo remove pins.

## Original open items (now done, kept for the record)
1. `src/diff.rs`: `ChangeSet` from a store version (`FieldStore::diff`), per-field summaries, propagation front (`Trace.front` ticks + distance from the edit), before/after frames for animation.
2. `src/provenance.rs`: persistent per cell-field contributor bitmasks (modules, edits, anomalies) from `Trace.touched`, and `explain(cell, field)` causal-chain builder; three test edits with known ground truth (toy chain; real energy stage lake; pinned albedo "ice patch in a desert").
3. Update `ARCHITECTURE.md`: dirty-region API, diff format, provenance format.
