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
