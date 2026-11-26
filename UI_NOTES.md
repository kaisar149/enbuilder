# UI_NOTES: what a client needs to know (Part 9a)

Read with `UI_ARCHITECTURE.md`. Everything below is the contract; `ARCHITECTURE_9.md` has the detail.

## API version
`api_version = 1`. Breaking JSON changes bump it; golden tests in `wb-app/tests/golden/` pin the shapes.

## Commands (JSON, `type` tag)
`load_preset {name}`, `set_param {name, value}`, `solve`, `cancel`, `undo`, `redo`. Each answers `{api_version, command, invalidated[], job}`.

## Queries
`get_schema`, `get_world_info`, `get_mesh`, `get_field {name, layer}`, `get_cell {cell}`. Binary parts are little-endian f32/u32; the JSON header describes offsets and counts.

## Events
`job_progress`, `job_done`, `field_changed`, `world_loaded`, `error`.

## Generate the UI from the schema
- Parameter panels: group by `params[].group`; widget from `widget` (`slider`, `integer`, `select` with `options`); show `unit`, `min`, `max`, `default`.
- Presets: `presets[]` is name, description, overrides. Send `load_preset`.
- Layer list: one entry per `fields[]` item whose `kind` is scalar, mask or category. Layer declaration: `{id, field, kind, colormap, range, opacity}`; use `range` if not null, otherwise the `min`/`max` from `get_field`.
- Legend labels: `value * display.scale + display.offset`, unit `display.unit` (temperature shows degC, precipitation mm/day).
- Biome colours: `biomes[].color`; a `biome` or `koppen` field holds ids.
- Stale marker: `get_world_info.stale` and `field.stale`; after `field_changed` refetch the named fields.
- Mesh: fetch `get_mesh` once per `mesh_id` (`world_loaded` announces a new one).
- Undo UI: `get_world_info.can_undo` / `can_redo`.

## Layer types so far
`scalar` (colormap + range), `mask` (two-colour), `category` (palette from schema). `vector` (wind) and `lines` (rivers) are declared in the schema but no renderer exists yet.

## Demo script
```
cargo build --release -p wb-app
./target/release/wb-snap --preset earthlike --field temperature   --out t.png
./target/release/wb-snap --preset earthlike --field precipitation --out rain.png
./target/release/wb-snap --preset earthlike --field biome         --out biome.png
./target/release/wb-snap --preset earthlike --param shape=1 --field elevation --out cube.png --width 640
```
Expect warm equator and cold poles in `t.png`, a biome map with a swatch legend, and the cube as an unfolded cross.

---

# Part 9b additions: wire protocol, client, viewport, panel registry

## Wire protocol v1 (`wb-server/src/protocol.rs`)
WebSocket at `/ws` on `127.0.0.1` only. `Host` must be localhost; `Origin`, when sent, must be local.
- Client to server (text, JSON): `{"id": <any>, "command": {...}}` or `{"id": <any>, "query": {...}}`.
- Server to client: one `hello` text frame (`{"hello": {wire_version, api_version, server}}`), then per request `{"id","ok":true,"kind","result"}` or `{"id","ok":false,"error":{code,message}}`, plus pushed `{"event": {...}}` frames.
- A response is written BEFORE the events its command caused.
- Queries with arrays (`get_mesh`, `get_field`) answer with ONE binary frame: `"WBF1"` | u32 LE json_len | JSON (the same response envelope) | zero padding so the payload starts on a multiple of 4 | little-endian payload. Offsets and counts come from the JSON `layout` (mesh) or `count` (field); never hard-code them.

## wb-client (framework-free)
`WbApp.connect(url)` loads schema, world info and mesh. `app.store` (`connection`, `schema`, `world`, `mesh`, `fields`, `jobs`, `lastError`). `app.ensureField(name)` is the only fetch path: one `get_field`, or nothing when the cached version is current. Events keep the store in sync; `app.client.sent` records every request (tests use it to prove what the UI sent).

## ui-web (React + Vite) layout
```
src/viewport/   types.ts (Viewport interface), MapViewport2D.ts (canvas), projections.ts, raster.ts, colormaps.ts   <- no React
src/state/      controller.ts (UI-only store + MapController, no React), hooks.ts (useStore)
src/panels/     registry.ts (layout data), components.tsx (name -> component), one file per panel
src/theme.css   design tokens (CSS variables) ONLY; app.css uses them
```

### Viewport interface (`src/viewport/types.ts`)
`setMesh(mesh)`, `setLayers(layers, fields, ctx?)`, `setCamera({x,y,zoom})`, `getCamera()`, `pick(x, y) -> cell | null`, `resize()`, `dispose()`, plus `onPick`, `onHover`, `onCameraChange` (each returns an unsubscribe). `ctx = { biomes }` carries palette data that layer declarations point at. For the 2D map `x,y` are the map point at the screen centre in 0..1 and `zoom 1` shows the whole map.

`MapViewport2D`: the pixel to cell table is built once per mesh (greedy descent on the CSR adjacency, brute-force on jumps, same as Rust `pixel_cells`); a layer switch repaints from that table; pan and zoom only redraw the cached bitmap. Projection strategy comes from `mesh.kind`: `sphere` equirectangular (w x w/2), `cube` unfolded cross (4s x 3s, see `CUBE_CROSS` in raster.rs), `plane` bounding box. Raster width defaults to 720 like `wb-snap`.
Input: drag pans, wheel zooms at the cursor, double click or `0` resets, arrow keys pan, `+`/`-` zoom, click picks.

### Panel registry (`src/panels/registry.ts`)
`{ id, title, component, default_dock: 'left'|'right'|'bottom'|'hidden' }[]`. `resolveLayout(defs, overrides)` takes `{panelId: dock}` overrides (the hook for saved layouts). `components.tsx` maps `component` names to React components receiving `{ app, ctl }`. Panels so far: `solve`, `layers`, `world`, `cell` (right), `legend` (bottom).

### Theme
Every colour, font, size and spacing is a CSS variable in `theme.css` (light, and dark through `prefers-color-scheme`). `static.test.ts` fails if a hex colour appears anywhere else.

### UI state that never reaches the core
Selected layer, hover cell, picked cell and its info (`MapController.ui`), camera (inside the viewport), theme.

## Demo script
```
cargo build --release -p wb-server -p wb-app
npm install
npm run build:ui                                  # builds ui-web/dist
./target/release/wb-server --preset earthlike     # prints http://127.0.0.1:PORT
```
Open the URL. Expect: the elevation map; "Show on map" lists every schema layer with unsolved ones disabled; press Solve, the bar fills and the dropdown unlocks; pick Temperature or Biome and the legend changes; wheel zooms, drag pans, click shows the cell. Change `--preset desert` or `--param shape=1` (cube) / `shape=2` (plane) to see other projections. Dev mode with hot reload: `wb-server --port 8787` in one terminal, `npm run dev:ui` in another, open http://localhost:5173.

---

# Part 9c additions: parameter panels, presets, seeds, variations, seasons, history

## Schema additions (additive; `api_version` stays 1)
- `params[].identity` (bool): true for `shape`, `resolution`, `radius_km`, `seed`. They define WHICH world it is, so the dirty-vs-preset check ignores them.
- `groups[]`: `{name, title, advanced}` in display order (`laws`, `world`, `solver[advanced]`). A parameter whose group is missing here still renders (group named after it, not advanced).
- `seasons`: `{count: 12, variants: {temperature: season_temperature, precipitation: season_precipitation, soil_moisture: season_soil_moisture}}`. Each twin is a field with `count` layers, read with `get_field {name: <twin>, layer: k}`. No new solver work; the twins appear in `world_info.fields` after a solve.
- `presets[].params` holds the overrides. **A preset resets every parameter it does not mention to its default (the seed too).**
- Greenhouse, evaporation and erosion parameters from the 9c brief are NOT in the schema yet (not wired to the simulation). They will get controls by themselves the day they are added to `param_defs()`; nothing in the UI names a parameter.

## Widget hints (what `ParamControl` does with `params[].widget`)
| widget | control |
|---|---|
| `slider` | range input + number input, unit label, `default` text, reset button |
| `integer` | number input (step 1); a range input too when `max - min <= 20` |
| `select` | `<select>` from `options[]`, value = index |
All show `description` as tooltip. Errors from the core appear under the control with `role="alert"`, `aria-invalid` on the input.

## Commands and queries added
- Command `variations {count, coarse=true}` (count 1..12, else `bad_param`): background job, answers `{job}`, progress stage `variation n of N`, `job_done` at the end. Changes no parameter and creates no undo step. `coarse` shortens spin-up to 1 year; the mesh and all other parameters are the current ones. A parameter change cancels the job; a mesh change (resolution, shape, radius) drops the stored batch.
- Query `get_variations`: `{mesh_id, base_seed, coarse, fields[], items: [{index, seed}]}`; `items` is empty when there is no batch for the current mesh.
- Query `get_variation {index, name}`: one kept field (`elevation`, `temperature`, `precipitation`, `biome`) in the same binary layout as `get_field`, header plus `variation` and `seed`. Errors: `no_variations`, `bad_variation`, `unknown_field`.
- **Opening a variation = `set_param seed` + `solve`.** The thumbnail is exactly the world you get (a test proves equality of the biome map).

## world_info addition: history
`world_info.history = { undo: [..oldest first..], redo: [..next step to re-apply first..] }`, each `{label, preset, changes: [{name, old, new}]}`. Labels are `name: old -> new` or `preset NAME`. A new change clears `redo`. `can_undo`/`can_redo` are unchanged.

## Validation
`set_param` outside min/max (or a non-integer for `integer`/`select`) fails with `bad_param`, message `parameter 'x' must be in [lo, hi], got v`. The core never clamps; the UI keeps what the user typed and shows the message. A non-number typed locally is flagged locally and never sent.

## wb-client additions
- `ParamController` (`params.ts`): `edit` (draft, debounced 250 ms), `commit`, `flush`, `reset`, `resetGroup`, `applyPreset`, `undo`, `redo`, `randomSeed`, `solve` (flushes first), `openVariation`. State `ui = {drafts, errors, sending, basePreset}`. Commits are serialised, so order is preserved.
- Selectors: `paramGroups` (titles and advanced flag), `presetStatus` / `presetTarget` (dirty indicator), `randomSeed`, `seasonVariant`, `seasonsAvailable`, `seasonLabel`, `variationThumbField`, `historyUndo` / `historyRedo`, `fmtParam`.
- `WbApp`: `generateVariations(count, coarse)`, `ensureVariation(index, name)`, `command(cmd, {quiet})`; store keys `variations` and `variationFields`; `jobs.current.kind` is `solve` or `variations`.
- `debounce(fn, ms, timers)` with `cancel`, `flush`, `pending`.

## Dirty-vs-preset rule
Target = every schema default, overridden by the preset's `params`. The world is dirty when a non-identity parameter differs. The base preset is `world.preset` when it is a real preset, else the preset the user last picked (`basePreset`, UI state), else the closest preset. (`world.preset` becomes `custom` after any `set_param`.)

## Panels added (registry ids)
`preset`, `params`, `variations` (left); `season`, `history`, `world` (right). `world` no longer has the preset select. Seasons are UI state (`MapController.ui.season`, `setSeason(k|null)`); the map swaps in `get_field(<twin>, layer=k)` and keeps the base field's colour range, so no command is sent.

## Demo script
```
cargo build --release -p wb-server -p wb-app && npm install && npm run build:ui
./target/release/wb-server --preset earthlike       # open the printed URL
```
1. "Preset" shows Earth like, "Matches Earth like". 2. In "World laws" drag axial tilt from 23.4 to 60: nothing is sent until you stop (one `set_param`), the preset line turns red and lists `axial tilt deg: 23.4 to 60 deg`. 3. Solve. 4. Select Temperature, open "Season", drag the slider: the map changes between seasons with no solve; "Yearly average" returns. 5. Type 120 in tilt: the core's message appears under the control and the value stays 120. 6. Pick Waterworld in the preset list. 7. "Seed and variations": Generate 6 variations, six thumbnails appear, click one: its seed is set and a solve starts. 8. "History" lists every change; Undo/Redo walk through them.

## Tests
`cargo test -p wb-app` (13 + 6 + 5), `npm run test:client` (28), `npm run test:ui` (jsdom panel tests, real-server demo test, plus the 9b suites).
