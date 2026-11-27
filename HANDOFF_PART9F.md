# HANDOFF: Part 9f (paint tools, locks, realism slider, preview modes)  -- LIVE FILE, updated at every checkpoint

## How to resume
Extract the latest `part9f_cpN.tar.gz` over the project root (it contains the whole source tree minus target/ and node_modules/).
Toolchain (no rustup): conda-forge `rust-1.100.0` (linux-64) + `rust-std-x86_64-unknown-linux-gnu-1.100.0` (noarch) `.conda` files, extract
`pkg-*.tar.zst` inside with python `zstandard` into /tmp/rustenv; `export PATH=/tmp/rustenv/bin:$PATH`. Then `npm install`,
`cargo build --release -p wb-app -p wb-server`, `cargo test --release -p wb-app`, `npm run test:client`, `npm run test:ui`.

## Measured baseline (this sandbox, release, earthlike res 3 = 642 cells)
full solve spinup 1 = 0.65 s, spinup 2 = 1.1 s  => ~37 ms per season; res 2 (162 cells) full year 0.08 s; res 4 (2562 cells) 5.3 s.

## Design decisions (9f)
1. Paint layers live in `wb-app` (`paint.rs`, pure): per field `value[]` + `mask[]`; fields: `elevation` (absolute m), `temperature_bias` (K delta),
   `rainfall_bias` (fractional delta of precipitation), `biome_override` (biome id). Stored SEPARATELY from computed fields.
2. Blend rule (per field realism r in [0,1] = slider/100): `effective = computed + (1-r) * mask * (painted - computed)`
   (bias layers: painted - computed = bias). r=0 -> painted as given, r=1 -> ignored, linear between. Category (biome): painted wins iff (1-r)*mask >= 0.5.
3. Strokes always edit the r=0 "painted view" (what you painted), independent of the slider. One `paint_stroke` command per stroke (batched by the client).
4. Locks: lockable output fields `elevation`, `temperature`, `precipitation`, `biome`. elevation + temperature locks are pinned inside the solver
   (FieldStore locks = boundary conditions); precipitation + biome locks hold the published value. Painting a locked cell is skipped (reported).
5. History is unified: params groups, strokes, locks and realism changes share one undo/redo stack with `kind`. Strokes keep an `Arc` snapshot of the
   solved state on the other side of the step (swap on undo/redo) so undo restores climate exactly. "Field versions equal" is implemented as
   content-hash equality, because versions must stay monotonic for client caches (documented deviation).
6. Solved state stays UNBIASED/computed; a derived "view" (bias, biome override, locks) is rebuilt on every mutation. Realism changes are instant, no re-solve.
7. Preview modes (command `refresh {mode}`, background job): `preview` = warm start from the last solve, 2 representative seasons (1 if too slow), rest of
   the year extrapolated by the mean delta; `refined` = warm start, 12 seasons; `full` = existing `solve` (now honours paint + locks).
8. Client: `tools.ts` (Tool interface + registry, brush geometry via mesh adjacency, falloff, stroke batching) and `EditController` (preview then refined).

## Status
- [ ] Rust core: paint.rs, api, session, world, schema, tests
- [ ] wb-client: types, client methods, tools, EditController, tests
- [ ] ui-web: Tools panel, brush ring, lock hatch overlay, realism, history, status badge, tests
- [ ] Docs: UI_NOTES (9f), ARCHITECTURE_9 s15, USER_GUIDE_9F
