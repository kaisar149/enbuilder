# HANDOFF: Part 9b (checkpoint 3: 9b-2 code complete, needs a look in a real browser)

## Status
- [x] 9b-1 `wb-server` (Rust) and `wb-client` (TypeScript). Unchanged in this checkpoint.
- [x] 9b-2 `ui-web` (Vite + React): shell, panel registry, schema-generated layer dropdown, `MapViewport2D`, legend, solve button + progress, cell inspector, preset switcher, theme tokens.
- [x] docs: `UI_NOTES.md` (9b section appended), `ARCHITECTURE_9.md` section 11.
- [ ] NOT DONE / NOT VERIFIED: nobody has looked at the UI in a real browser yet (no browser in the sandbox). Layout, canvas sizing, pan/zoom feel and dark mode are untested by eye.
- [ ] Not started (belongs to 9c): parameter sliders, undo/redo buttons, paint tools, vector (wind) and lines (rivers) layers.

## Verified in the sandbox (real binaries, no mocks)
- `npm run test:client`: 15 pass. `npm run test:ui`: 19 pass (server.test 9, static 3, viewport 4, panels 3). `tsc --noEmit` clean. `vite build` OK (173 kB JS, 57 kB gzip).
- Pixel parity with `wb-snap --no-legend` at width 720: 0 differing pixels for sphere elevation, temperature, biome, cube elevation, plane elevation. Negative control (wrong colormap) differs on 100%.
- Layer switching sends only `get_field` (zero commands; zero requests when cached). Click sends one `get_cell`.
- `wb-server` serves `ui-web/dist` (index and assets answer 200).
- Cargo 1.75 was installed in the sandbox (apt works); `cargo build --release -p wb-server -p wb-app` takes about 70 s. I did NOT run `cargo test` this round (no Rust code changed).

## Decisions made in 9b-2
1. Raster core is pure (`viewport/raster.ts`, `projections.ts`, `colormaps.ts`): no DOM, so Node tests can compare it with the Rust PNG. `MapViewport2D` only wraps it.
2. Pixel to cell table built once per mesh; layer switch = repaint from table; pan/zoom = redraw cached bitmap.
3. Unsolved layers are shown but disabled ("solve first"). After a solve they unlock; `field_changed` refetches only the watched layer.
4. UI-only state lives in `MapController.ui` (layer, hover, picked cell); camera lives in the viewport. Nothing UI-only is sent to the core.
5. Colormap stops are duplicated in Rust and TS; the parity test guards drift.
6. Design direction: "survey sheet" (cool grey-green paper, ink, one indigo accent; serif panel titles, system sans body). All values are variables in `theme.css`; dark mode via `prefers-color-scheme`.
7. Static test exempts `colormaps.ts` from the "no field names" scan because colormap names (`terrain`, `temperature`, `rain`, `speed`) overlap field names but are a different vocabulary.

## Files missing from the zip you gave me (check your real copies)
`Cargo.toml` (root), `package.json` (root), `pack.sh`. I wrote a sandbox-only root `Cargo.toml` (edition 2021, no `gpu` feature) and a new root `package.json` (workspaces `wb-client`, `ui-web`; scripts `build:server`, `build:ui`, `dev:ui`, `test:client`, `test:ui`, `typecheck`). Keep your own Cargo.toml and `pack.sh`; merge only the package.json scripts you want. Nothing else outside `ui-web/` and the docs was modified (`core/Cargo.toml` was touched temporarily and restored).

## Verify on your machine
```
cargo build --release -p wb-server -p wb-app
npm install
npm run test:client && npm run test:ui
npm run build:ui
./target/release/wb-server --preset earthlike      # open the printed URL
```
Look for: map fills the centre; dropdown lists schema layers; Solve fills the bar and unlocks layers; legend matches; click shows cell values; try `--param shape=1` and `shape=2`.

## Next step suggestions
1. Eyeball the browser and send me anything off (screenshots help). 
2. 9c: schema-generated parameter panel (`set_param`), undo/redo, stale-result banner.
