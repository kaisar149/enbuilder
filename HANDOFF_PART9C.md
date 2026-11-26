# HANDOFF: Part 9c (checkpoint 4: Rust + wb-client + ui-web panels DONE and tested; docs next)

## How to resume
Extract `part9c_changes.tar.gz` over your project root (it holds only files that differ from your zip; `CHANGED_FILES.txt` lists them).
If this session is interrupted, tell the next session: "continue Part 9c from HANDOFF_PART9C.md".

## Sandbox limits (important)
- No network: crates.io and npm are unreachable, so `serde`, `serde_json`, `vitest`, `vite`, `jsdom` were NOT available.
- Therefore `wb-app` (needs serde) was NOT compiled here. What WAS compiled and run for real: the new pure module `variations.rs` together with
  `world.rs`/`png.rs` against the real simulation crate (5 tests pass, see `wb-app/tests/part9c_variations.rs`).
- NOT compiled (please run on your machine): `session.rs`, `api.rs`, `schema.rs`, `params.rs` edits and `wb-app/tests/part9c.rs`.
  Run: `cargo test -p wb-app` (expect golden files to match; if they differ only by formatting run `UPDATE_GOLDEN=1 cargo test -p wb-app` and diff).
  The two golden JSON files were edited BY HAND.

## Done (Rust, wb-app)
- `params.rs`: `ParamDef.identity` (true for shape, resolution, radius_km, seed), `GroupDef` + `group_defs()` (laws, world, solver[advanced]).
- `schema.rs`: new top-level keys `groups` and `seasons {count, variants{temperature, precipitation, soil_moisture -> season_* twin}}`.
- `api.rs`: command `variations {count, coarse=true}`; queries `get_variations`, `get_variation {index, name}`.
- `variations.rs` (new): deterministic distinct seeds, parallel solves (max 4 threads), reproducible, cancelable.
- `session.rs`: variations as a background job (`job_progress` stage "variation n of N"); results kept per mesh; `world_info.history {undo[], redo[]}`
  with `{label, preset, changes[{name, old, new}]}`; a mesh change drops the batch.
- `world.rs`: `#[derive(Debug)]` on `SolveError` only.
- No simulation (`enbuilder`) code changed.

## Still to do
- wb-client: types, `variations`/`getVariations`/`getVariation`, selectors (preset match, groups, seasons), debounce helper.
- ui-web: ParameterPanel, preset picker + dirty indicator, seed + 6 variations, season slider, undo/redo + history, validation messages.
- Tests (Node `node:test` is used here because vitest is missing), UI_NOTES.md, ARCHITECTURE_9.md section 12, user guide.

## Decisions
1. Variations use the SAME mesh and parameters with new seeds; `coarse` only shortens spin-up to 1 year. Opening one = `set_param seed` + `solve`.
2. Dirty-vs-preset is computed by the client from `get_schema` (`presets[].params`) and `world_info.params`, ignoring `identity` params.
3. Season slider asks `get_field(<twin>, layer=k)`; no new solver work.
4. Greenhouse, evaporation and erosion parameters from the 9c brief do not exist in the schema yet (not wired to the simulation); they will appear by themselves once added to `param_defs()`.

## Checkpoint 2 update
- Network is available this session (npm, crates via conda Rust). `cargo test -p wb-app` now PASSES (13 + 6 + 5 tests, golden files match as hand-edited).
- Rust toolchain in sandbox: micromamba env at /opt/rustenv (PATH=/opt/rustenv/bin:$PATH). `npm install` at repo root works; vitest available.
- Order of remaining work: wb-client (types, variations API, selectors, ParamController with debounce) -> tests vs real wb-server -> ui-web panels -> docs.
- Error codes the UI must show next to controls: `bad_param` (out of range), `unknown_param`, `job_running`.

## Checkpoint 3 update: wb-client done (13 new tests, 28 total pass against the real wb-server)
New in wb-client: `debounce.ts`, `params.ts` (`ParamController`: edit/commit/flush/reset/resetGroup/applyPreset/undo/redo/randomSeed/solve/openVariation),
selectors (`paramGroups` with titles+advanced, `presetStatus`, `presetTarget`, `randomSeed`, `seasonsAvailable/seasonVariant/seasonLabel`, `historyUndo/Redo`, `fmtParam`),
`WbApp.generateVariations`, `ensureVariation`, store keys `variations` and `variationFields`, `app.command(cmd, {quiet})`.
Facts: a preset resets every parameter it does not mention (incl. seed); world.preset becomes "custom" after set_param, so the UI keeps `basePreset` (ParamController.ui).
Test run: `cargo build -p wb-server && cd wb-client && npx vitest run`.
Next: ui-web ParameterPanel, PresetPicker, SeedPanel+VariationsPanel, SeasonSlider, HistoryPanel; panels tests; UI_NOTES.md, ARCHITECTURE_9.md section 12, USER_GUIDE_9C.md.

## Checkpoint 4 update: ui-web done
New panels (registry ids): `preset` (PresetPanel), `params` (ParameterPanel + ParamControl), `variations` (VariationsPanel, thumbnails via `viewport/thumbnail.ts`),
`season` (SeasonPanel), `history` (HistoryPanel). `world` panel lost its preset select (moved to PresetPanel). SolvePanel now calls `ctl.params.solve()` (flushes pending edits).
`MapController` gained `params: ParamController` and UI-only `season` (+ `setSeason`); `viewLayers` swaps in the season layer, keeping the base field's colour range.
`static.test.ts` forced the thumbnail field to come from data (`variationThumbField`), never a literal name.
Tests: `cd ui-web && npx vitest run test/part9c.test.tsx test/server9c.test.tsx` (15 pass; server9c needs target/debug or release wb-server).
`server.test.ts` needs `cargo build --release -p wb-app` (wb-snap); building that in the sandbox now.
Remaining: run the full suites once release binaries exist, UI_NOTES.md (9c section), ARCHITECTURE_9.md section 12, USER_GUIDE_9C.md, final CHANGED_FILES list + tarball.
