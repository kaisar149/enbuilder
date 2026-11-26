# ARCHITECTURE_9: Part 9 additions (app core, API, snapshots)

Companion to `ARCHITECTURE.md` (which is NOT edited; Part 8 was skipped by decision). Part 9 notes accumulate here and in `UI_NOTES.md`.

## 1. Workspace
Root `Cargo.toml` gets `[workspace] members = [".", "wb-app"]`. `wb-app` (edition 2021) depends on `enbuilder` (path), `serde`, `serde_json`. The simulation crate stays std-only.

Simulation-crate change in Part 9a (rule 6.4): `src/scheduler.rs` gained the read-only `Scheduler::module_params(module) -> Option<Vec<(String, f64)>>`, used for the schema export. Nothing else.

## 2. wb-app modules
| Module | Role |
|---|---|
| `api.rs` | `Command`, `Query`, `Event`, `Ack`, `ApiError`, `QueryResult`, `API_VERSION = 1` |
| `params.rs` | 11 parameters and 5 presets as DATA; each parameter has a `Scope` (Climate < Elevation < Mesh) |
| `world.rs` | `build_domain`, `build_elevation` (noise, exact area ocean fraction), `pipeline`, `solve` (spin-up years x 12 seasons, biomes, annual-mean `temperature` K and `precipitation` kg m-2 s-1), `mesh_id` |
| `schema.rs` | `schema()`; field display metadata table (`FIELD_META`); module list read from the real scheduler |
| `session.rs` | `Session`: execute / query / poll_events / wait_job / solve_blocking; undo and redo of parameter groups; stale tracking; solve worker thread with cancel |
| `png.rs` | stored-deflate PNG encoder, decoder for tests, crc32/adler32/fnv1a |
| `raster.rs` | colormaps, projections, pixel to cell lookup, legend, `render_field` |
| `bin/wb-snap.rs` | CLI |

## 3. Commands (all return `Ack { api_version, command, invalidated[], job }`)
| JSON `type` | Fields | Effect |
|---|---|---|
| `load_preset` | `name` | Defaults plus preset overrides; ONE undo step; error `unknown_preset` |
| `set_param` | `name`, `value` | Validated against min/max (`bad_param`); one undo step; preset label becomes `custom` |
| `solve` | none | Starts a background job, `ack.job` = job id; `job_running` if one is active |
| `cancel` | none | Flags the running job |
| `undo`, `redo` | none | `nothing_to_undo` / `nothing_to_redo` |

Invalidation by parameter scope: Climate = solved fields become stale (names listed in `invalidated`); Elevation = `elevation` regenerated and listed first, solved fields stale; Mesh = new `mesh_id`, `world_loaded` event, solved fields dropped. Changing a parameter while a job runs cancels and joins the job first.

## 4. Queries
`get_schema`, `get_world_info`, `get_mesh`, `get_field {name, layer=0}`, `get_cell {cell}`. Each returns JSON plus optional little-endian binary.
- `get_mesh` binary layout (header `layout[]` has name, dtype, count, offset): `centroid` f32x3, `area` f32, `adj_offsets` u32, `adj` u32 (CSR). Header carries `mesh_id`, `kind`, `n_cells`.
- `get_field` binary: `count` f32 values for one layer; header = schema field entry plus `mesh_id`, `version`, `stale`, `layer`, `layers`, `min`, `max`, `dtype`, `count`. `elevation` is always current; other fields need a solve (`not_solved`).
- `get_cell`: values of every field at the cell, `biome_name`, `neighbors`, `centroid`, `area_m2`, and `lat_deg`/`lon_deg` on spheres.
- Error codes so far: `unknown_preset`, `bad_param`, `not_solved`, `unknown_field`, `bad_layer`, `bad_cell`, `job_running`, `nothing_to_undo`, `nothing_to_redo`, `bad_shape`, `sim_error`.

## 5. Events (pulled with `poll_events`)
`job_progress {job, fraction, stage}`, `job_done {job, status: completed|cancelled|failed}`, `field_changed {names, version}`, `world_loaded {mesh_id, n_cells}`, `error {message}`. A cancelled solve publishes no fields.

## 6. Parameters (generated into the schema)
world: `shape` (0 sphere, 1 cube, 2 plane), `resolution` (1-5, default 3), `radius_km`, `seed`, `ocean_fraction`; laws: `surface_gravity`, `day_length_h` (0 = no rotation), `year_length_days` (0 = no seasons), `axial_tilt_deg`, `solar_constant`; solver: `spinup_years`.
Presets: `earthlike`, `desert`, `waterworld`, `tidally_locked`, `ice_age`.

## 7. Schema format (`get_schema`)
`{api_version, params[], presets[], modules[], loops[], fields[], biomes[], colormaps[]}`.
- `params[]`: name, group, unit, min, max, default, widget hint, scope, description, options.
- `modules[]`: name, inputs, outputs, anomaly_hooks, params `[{name, default}]` (from `Scheduler::module_params`).
- `fields[]`: name, kind (scalar, vector, mask, category), unit, `display {scale, offset, unit}`, colormap, `range` (or null = auto), description, `produced_by[]`.
- `biomes[]`: id, name, color.
Colormaps: `terrain`, `temperature`, `rain`, `speed`, `viridis`, `mask`, `category`.

## 8. Rasters and wb-snap
- Sphere: equirectangular (width x width/2). Cube: unfolded cross, middle row `-x -y +x +y`, `+z` above `-y`, `-z` below `-y` (continuity is tested). Plane: direct, aspect from the mesh extent.
- Pixel to cell: greedy descent on the mesh adjacency from the previous pixel's cell, brute force after jumps; tested equal to brute force.
- Legend strip (44 px): gradient with min/max in display units (tiny 3x5 digit font), or biome swatches with ids.
- CLI: `wb-snap --preset earthlike --field temperature --out t.png [--param k=v]... [--width N] [--layer N] [--no-legend] [--list]`. Prints size, colour range and a PNG hash. Earth-like resolution 3 (642 cells) solves in about 0.8 s (release).

## 9. Tests (`cargo test -p wb-app`, `tests/part9a.rs`, 13 tests)
Command-driven session, typed errors, JSON wire format, golden `schema.json` and `world_info.json`, schema from real modules, undo/redo with invalidation, cancel leaves a valid session, parameter change cancels a job, PNG round trip plus pinned hash file `snapshot_hashes.json`, planet-looking map, all three shapes (lookup vs brute force), cube seam continuity, biome palette. Regenerate goldens on purpose: `UPDATE_GOLDEN=1 cargo test -p wb-app`. If the pinned PNG hashes differ on another machine, the cause is floating-point differences in the simulation, not the encoder.

## 10. Open decisions
- Only 5 commands and 5 queries exist; `paint_stroke`, `add_anomaly`, `set_lock`, `save_world`, `load_world`, `explain` come in later Part 9 sessions.
- `get_field` sends layer 0 unless asked; vector fields (wind) are not drawn by `wb-snap` yet.
- Events are pulled; the adapter (Part 9b) will push them.
- Elevation changes (seed, ocean fraction) rebuild the world on the calling thread.

## 11. Part 9b: server, client, 2D map viewer
- **wb-server** (9b-1): std-only HTTP + WebSocket adapter, one session actor thread, localhost only, serves `ui-web/dist` (`--static DIR` overrides). 13 Rust tests.
- **wb-client** (9b-1): see UI_NOTES. 15 vitest tests against the real binary.
- **ui-web** (9b-2): React + Vite shell, panel registry, schema-generated layer dropdown, `MapViewport2D`, legend, solve button with progress, cell inspector, preset switcher. 19 vitest tests (`npm run test:ui`).
- **No simulation code changed in 9b-2.** wb-app changed in 9b-1 only (`featured`, `palette` additive keys).
- **Parity guarantee:** `ui-web/test/server.test.ts` renders elevation, temperature and biome (sphere) and elevation (cube, plane) both with `wb-snap` and with the browser pipeline and requires under 1% differing pixels; measured 0 differing pixels; a deliberately wrong colormap differs on 100% (negative control).
- **Colormap duplication:** the stops exist in `wb-app/src/raster.rs` and `ui-web/src/viewport/colormaps.ts`. The parity test catches drift; a later step may export the stops through the schema.
