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
