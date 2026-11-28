# HANDOFF: Part 9g (diff view, propagation animation, "why" explainer)  -- LIVE FILE, updated at every checkpoint
STATUS: cp4 = ALL PLANNED DELIVERABLES DONE. wb-app 54 (+9 part9g), wb-server 13, wb-client 56, ui-web 110 (+8 part9g_ui), typecheck + vite build green. Not run in a real browser/GPU.

## How to resume
Extract the latest `part9g_cpN.tar.gz` over the project root (whole source tree minus target/ and node_modules/).
Toolchain (this sandbox had none): conda-forge `rust-1.100.0` (linux-64) + `rust-std-x86_64-unknown-linux-gnu-1.100.0` (noarch) `.conda` files;
unzip, decompress `pkg-*.tar.zst` with python `zstandard` into /tmp/rustenv; `export PATH=/tmp/rustenv/bin:$PATH`.
Then `npm install`, `cargo build --release -p wb-app -p wb-server`, `cargo test --release -p wb-app --test part9g`, `npm run test:client`, `npm run test:ui`.

## Design decisions (9g)
1. STORED VERSIONS (`wb-app/src/diffq.rs::Snap`): every installed solve/refresh and every stroke-undo that swaps a solved state is stored (max 8, oldest dropped):
   solved fields (Arc), the 4 output views as clients saw them (physics + paint + locks), pins used, label (`quality: last history label`). Key = the solved
   state's `version` (monotonic). Query `get_versions` lists them (current mesh only).
2. DIFF = core `ChangeSet` built from two stored states (all common fields with equal layer count; the 4 output fields from the stored VIEWS, so locks hold
   and the diff equals what the user saw), then core `field_diff`. Multi-layer fields (season_*): the layer with the largest |change| per cell.
3. NOISE: re-running a solve with NO edit drifts (measured res 3: temperature peak 0.015 K, rain peak 1.6e-6 = ~4% of a real edit's peak, wind 1e-12).
   So `min_rel` (default 0.01, query param) hides changes smaller than min_rel x the field's largest change AND 1e-4 x the field's value range. Header reports
   `hidden_cells`, `changed_cells_exact`. `min_rel = 0` = exact. Locked cells: identical in both views, so exactly 0 whatever the threshold.
4. FRONT: no scheduler `Trace` exists for the app's land-cycle solve, so the core's fallback order is used: (surface distance from the edit seeds, cell id).
   Seeds = cells whose elevation view differs or whose pin differs between the versions (`front_kind: "edit"`); with none (parameter change, no-edit refresh)
   seeds = the cell with the largest change of that field (`front_kind: "peak"`). Wire `front` = REVEAL TIME in [0,1] (rank/(changed-1)) for changed cells, -1 unchanged.
   Playhead t shows exactly the cells with 0 <= front <= t. `dist_km` also sent (-1 = unreachable).
5. EXPLAIN = core `explain` over the significance-filtered ChangeSet + `Provenance::from_producers` + marks `pin:painted elevation` (elevation seed cells) and
   `pin:locked <field>` (pin diffs) as ORIGINS. Chain returned as data (`steps[]`), `text` = core `Explanation::text()` unchanged. A step's `source_cells` = next link's cell + equally
   ranked alternatives (origin step: cells carrying the origin contributor, max 2000).
6. Honest limit seen in the demo: a basin dug into the driest land cell (res 3) changed 90 fields, reach ~19000 km (global pressure/vapour coupling); the chain for a far
   rain cell is `precipitation <- moisture_transport GLOBAL coupling <- elevation [origin: pin:painted elevation]`. The explainer never invents intermediate "wind -> humidity" hops: it shows what the core measured.

## API additions (additive, api_version stays 1)
Queries: `get_versions`; `get_diff {field, from_version, to_version, min_rel=0.01}` -> JSON header + binary 5 x f32[n]: delta, before, after, front, dist_km;
`get_diff_summary {from_version, to_version, min_rel}` (JSON: per-field rows, totals, reach_km, edit_cells); `explain {cell, field, from_version?, to_version?, min_rel}` (JSON).
Errors: `unknown_version`, `mesh_mismatch`, `unknown_field`, `bad_cell`, `not_solved`. No core simulation code changed. Goldens unchanged (no existing JSON changed).

## Status
- [x] cp1 Rust: diffq.rs, api.rs variants, session.rs (snaps, diff_cache, record_snapshot, default_pair), `wb-app/tests/part9g.rs` (9 tests):
      self-diff zero; front nondecreasing + ends with exactly the changed cells; before+delta=after = visible field; locked cells never change/never revealed;
      explain chain = core text, first link = measured change; unknown version/field errors + summary; undo stores the restored version (diff vs first solve = 0);
      no-edit re-solve hides round-off; basin demo (prints chain).
- [x] cp2 wb-client: types (VersionsData, DiffData, DiffSummary, Explanation, ExplainStep), client `getVersions/getDiff/getDiffSummary/explain`, `decodeDiff` (layout offsets),
      `diffview.ts` (pure: frameDelta/revealedCount at playhead t, symmetricRange (slider), advancePlayhead, sweepCoord + compareValues for swipe/flip, stepLine/coreLinkLines/explainRows),
      `diff.ts` DiffController (versions list follows world field versions; pair is AUTO = (previous, latest) until `select()`, `followLatest()` resets; per pair/field/min_rel cache; no view state in it).
      Tests `wb-client/test/part9g.test.ts` (9, real server). Layout of get_diff now carries `offset` (fixed in diffq.rs).
- [x] cp3 ui-web (state/diffLayer.ts, state/diffctl.ts, panels/DiffPanel.tsx, panels/ExplainPanel.tsx, MapView Diff button + highlight overlay, registry ids `diff` `explain`, colormap `diverging` in TS + Rust; test/part9g_ui.test.tsx). Original plan: diverging colormap (TS + Rust raster), DiffPanel (version pickers, Diff toggle, range slider, play/time slider, before/after swipe + flip, summary), ExplainPanel (click cell -> chain -> step click highlights source cells), playhead kept across 2D/3D
- [x] cp4 docs (UI_NOTES 9g section, ARCHITECTURE_9 s16, USER_GUIDE_9G.md). Original plan: UI_NOTES (9g), ARCHITECTURE_9 s16, USER_GUIDE_9G.md

## Bugs found and fixed on the way
- `get_diff` header named the field `name` (from `field_json`), so `DiffData.field` was undefined; header now has `field`, decoder falls back to `name`.
- DiffController pair was frozen at (v, v) when the first version arrived; pair is now automatic until the user selects.
- Fields outside the curated schema list crashed the compare layer (no schema meta); the diff answer's own metadata is the fallback.

## Remaining (optional)
- Real-browser/GPU check of the demo script (nothing drawn by a real renderer yet); screen-space swipe wipe.
- Traced solve (`Trace`) for true pass-order fronts; `field_diff` already takes a trace.
- Persist versions in world files; ARCHITECTURE.md (main) one-line summary for 9g; Rust/TS colormap parity test for `diverging` (stops are copied by hand).
- Possible 9h: explanation of preview-vs-refined error separately from the edit.
