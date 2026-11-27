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

# Part 9d additions: 3D viewer (globe, cube, flat)

## Structure (renderer swap in one file)
- `viewport/BaseViewport3D.ts`: everything but drawing (canvas, pointer/wheel/key input, orbit camera + focus animation, `pick`, hover/pick events, colour per cell, relief, selection, frame stats). Renderers implement 8 hooks (`onGeometry`, `onPositions`, `onColors`, `onView`, `onSelection`, `onResize`, `draw`, `onDispose`).
- `viewport/MeshViewport3D.ts`: Three.js renderer, the ONLY file importing `three`. `viewport/StubViewport.ts` draws nothing (swap proof, no-WebGL fallback). `viewport/factory.ts` chooses and lazy-loads.
- Pure modules (no DOM, no three): `geometry3d.ts` (mesh -> polygons, relief), `orbit.ts` (camera maths), `pick3d.ts` (ray pick), `sun.ts` (sun direction), `view3d.ts` (options from world params, tooltip rows).

## Camera state format (`OrbitCamera`)
`{ kind: 'orbit', yaw, pitch, distance, target: [x,y,z] }`. Lengths are multiples of the world's bounding radius R. Z up; yaw from +X towards +Y, pitch from the horizon towards +Z. Camera position = `target*R + R*distance*(cos p cos y, cos p sin y, sin p)`, looking at `target*R`. Vertical FOV 40 deg. Per-shape limits in `orbit.ts`. 2D `CameraState` and orbit cameras are ignored by the other family. UI-only, never sent to the core.

## Mesh to triangles
One flat-coloured polygon per cell. Corners are circumcentres of (cell, neighbour k, neighbour k+1) in angular order (Voronoi hexagon/pentagon on the sphere, square on cube and plane; plane edge mirrored; cube folds unfolded into the cell's face plane). Triangle fan per cell; corners are owned by one cell, so colour and shading normal are per cell. Relief lifts a corner by the mean relief of the cells meeting there (no cracks), along the radial direction (sphere, cube) or +Z (plane). Relief height = max(elevation, 0) x exaggeration x unitsPerMetre; sea stays flat. unitsPerMetre = mesh radius / (radius_km x 1000).

## Colours and lighting
Same `cellColors` as the 2D map; renderer uses linear output with colour management off, so a cell has identical bytes in 2D and 3D. Flat mode = unlit. Sun mode = Lambert + directional light along `sunDirection(...)` (same geometry as the core insolation; follows the Season slider and a time-of-day slider); day/night on = near-black night side. Lights are scaled by PI (three r155+ physical units).

## UI state (never reaches the core)
`ui.view` ('2d'|'3d'), `ui.view3d` {exaggeration, lighting, dayNight, timeOfDay}, hover tooltip (`get_cell`, 150 ms delay, cached per cell and world version), renderer name, frame stats, benchmark. The relief field name is data (`viewport/view3d.config.json`); open decision: a schema flag instead.

## Panel and toolbar
Registry id `view3d` (left dock): relief, lighting, day/night, time of day, Reset view, Measure frame rate. Map toolbar: 2D map / 3D world toggle, Reset view. The toggle sends no command; layer, season, selected cell and legend are shared.

## Measured numbers (this sandbox: software WebGL, NOT a GPU)
Real-renderer colour parity: 360/360 sampled cells equal the 2D colour (sphere, cube, plane, relief 0 and 40). Geometry build (`setMesh`) in Chromium: plane 500k cells 0.67 s, cube 505k 0.69 s, sphere 164k 0.8 s, sphere 655k 3.4 s. Software-GL frame rate 1.2-2.5 fps at 160k-655k cells (CPU rasteriser). The interactive-frame-rate criterion on a mid-range GPU is NOT yet measured: press "Measure frame rate" or run `node test/gl/run.mjs <outdir>` on a machine with a GPU (remove the swiftshader flags in run.mjs) and record it here: ______ fps at ______ cells on ______.

## Tests
`test/part9d_geometry.test.ts` (5), `part9d_pick.test.ts` (8), `part9d_viewport.test.ts` (7, stub swap, picking, parity), `part9d_ui.test.tsx` (3, real wb-server). `test/gl/` is a manual Chromium harness (not in `vitest run`).

## Demo script
Start wb-server and the UI, Solve, click "3D world", drag to orbit, wheel to zoom, double-click a cell to centre it, raise Relief exaggeration, switch Lighting to Sun, hover a cell for temperature and rainfall, click "2D map" and see the same layer and selection.


# Part 9e additions: vector overlays, layer stack, cell inspector

## Layer kinds (data, in the schema: `layer_kinds`)
`scalar` (the base layer, colormap), plus overlays declared in `schema.overlays`:
`{ id, label, kind, token, field?, source?, weight_field?, threshold?, mode?, animated? }`
- `vector`: reads `field` (3 layers = x, y, z in mesh space). Arrow glyphs, length by magnitude (relative to the 95th percentile), thinned by zoom; optional animated streaks when `animated`.
- `lines`: `source` names a core geometry query (`rivers`); `weight_field` (`discharge`) scales line width (log scale against the largest river).
- `mask`: cells with `field > threshold`; `mode` = `fill` (triangles of the cells, e.g. lakes) or `outline` (core query `get_outline`, e.g. coast, glaciers).
Declared overlays: wind, currents, rivers, lakes, coast, glaciers. `token` -> theme colour `--overlay-<token>`. Viewers know kinds, never field names.

## Geometry queries (in wb-app, topology only: cell ids, no coordinates)
- `get_rivers`: header `{mesh_id, kind:"rivers", version, stale, n_lines, n_points, layout}`; binary `offsets` u32 (n_lines+1), `cells` u32, `discharge` f32 per vertex. Line k = `cells[offsets[k]..offsets[k+1]]`; consecutive vertices are neighbours; lines start at sources/confluences and end where the water goes. Error `not_solved` before a solve.
- `get_outline {field, threshold=0.5}`: header `{mesh_id, field, threshold, version, stale, n_edges}`; binary `pairs` u32 (inside cell, outside neighbour). Works for any scalar/mask field.
- `get_cell` now also returns `seasonal` (12 values for temperature, precipitation, soil moisture).
- Each viewport turns cell ids into its own space (`overlays/geometry.ts` mesh space, `viewport/overlay2d.ts` map space via `makeForward`, `viewport/overlay3d.ts` for the globe/cube).

## Layer stack model (`layers/stack.ts`, UI-only, never sent to the core)
`LayerStack { entries[] (overlays, bottom to top), base {visible, opacity}, solo, preset }`; entry = `{ id, visible, opacity, blend: normal|multiply|screen }`. The scalar base is always drawn first and cannot be reordered, so overlays (rivers) always draw above elevation. Reordering (`moveTo`/`moveBy`) changes order only. `stackFromPreset` builds a stack in one step; `matchesPreset` says whether it is still unmodified. `layers/items.ts` turns stack + fetched data into `OverlayItem[]`; `dataNeeds` says what the controller must fetch (only for drawn overlays).

## Presets (data, `schema.layer_presets`)
`{ id, name, description, base, overlays[] }`: terrain, climate, hydrology, ocean. One click applies base layer + overlay stack. Adding a preset in Rust adds a button.

## Inspector format (`layers/inspector.ts`)
`inspectorModel(schema, cellInfo, pinnedInfo?)` -> `{ title, subtitle (biome), stale, groups[{id,title,rows[]}], pinnedTitle }`. Group = first module in `produced_by` (pipeline order; unknown fields under "Other"). Row = `{ name, label, unit, raw, text, other?{raw,text,delta}, seasons?{raw[12],text[12],min,max} }`. `raw` is exactly the number `get_cell` returned; only the schema display scale/offset is applied for text. Nothing is recomputed. Pinning a cell adds a second column with the difference.

## Client cache (wb-client store)
`ensureRivers()`, `ensureOutline(field, threshold)`: fetched once, cached in `state.rivers` / `state.outlines`; concurrent calls share one request; dropped and refetched (only what is watched) on `field_changed` of their inputs or `world_loaded`.

## Panels and controller
Registry ids `stack` (LayerStackPanel: toggle, reorder, opacity, blend, solo, legend, presets, streak toggle) and `cell` (InspectorPanel). Controller: `stack`, `animate`, `applyPreset`, `updateStack`, `pinCell`. Viewport interface: `setOverlays(items, opts)`; StubViewport logs `overlayList`.

## Fixes made on the way
`pick(null)` no longer resets view/season; tooltip rows skip vector/mask fields and list curated fields first.

## Tests
wb-client `part9e.test.ts` (6, real server: schema data, connected rivers, cache without refetch, outline edges, refetch on change, seasons). ui-web `part9e.test.ts` (21, headless: stack, arrows vs field on plane/sphere/cube, thinning, rivers connected and above base, inspector = get_cell, pinned compare, particles). Rust `wb-app/tests/part9e.rs`. Totals now: wb-client 34, ui-web 79, wb-app 13+6+5+6.

## NOT verified
No browser/GPU check of the overlay drawing this session (only the pure geometry and stub paths are tested). Streak performance and the Hydrology/Climate demo on a real renderer still need a look: run the UI, Solve, click presets.

## Demo script
Start wb-server and the UI, Solve. Layer stack panel: click Hydrology (relief + rivers, lakes, coast), then Climate (rainfall + wind arrows; turn on animated streaks). Click a cell behind a mountain range: the Cell inspector lists every field by module with 12-season values. Press Pin, click a wet coastal cell, read the side-by-side difference.
