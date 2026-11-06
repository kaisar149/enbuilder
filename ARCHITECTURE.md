# ARCHITECTURE.md (after PART 2: Terrain and energy)

Crate: `enbuilder` (Rust edition 2024, std only, no dependencies). Part 1 compiled and passed its tests (reported by the project owner). Part 2 is **written but not yet compiled** (see "Verification status").

Changes to Part 1 code in Part 2: `Domain` gained `spin_axis: Vec3` (unit rotation axis, from `north_ref`); `Light` gained `reference_distance_m` and the constructors `Light::directional`, `Light::point`, `.with_orbit(orbital_period_s, day_length_s, axial_tilt_rad)`. Nothing else in Part 1 changed.

## Module map
| File | Contents |
|---|---|
| `math.rs` | `Vec3 = [f64;3]`, add/sub/scale/dot/cross/normalize/length, `arc_angle`, `spherical_triangle_area` |
| `mesh.rs` | `Mesh` (CSR), `MeshBuilder`, `Mesh::validate()` |
| `domain.rs` | `Domain`, `DomainKind`, `BoundaryKind`, `SurfaceFrame`, `Light`, `LightKind`, `Media`, `Fluid`, `VerticalStructure`, `compute_frames` |
| `generators/{sphere,cube,plane}.rs` | `generate_sphere`, `generate_cube`, `generate_plane` + `*Params` |
| `fields.rs` | `FieldStore`, `Field`, `Diff` (grouped sparse undo/redo/diff) |
| `module.rs` | `Module` trait, `ModuleSpec`, `ParamSet`, `DirtyRegion` |
| `scheduler.rs` | `Scheduler`, `FeedbackLoop`, `RunReport`, `LoopReport` |
| `testmods.rs` | throwaway: `ConstantSource`, `Diffusion`, `Thermostat`, `Noop` |
| `consts.rs` | `SIGMA`, `G_EARTH`, `SOLAR_CONSTANT` |
| `rng.rs` | `Rng` (SplitMix64): `next_u64/next_f64/range/unit_vec3/choose_distinct` |
| `noise.rs` | `value_noise3`, `fbm3` on 3D positions (topology independent) |
| `tectonics.rs` | `Tectonics`, `PaintedPlates`, `PlateDef` |
| `terrain.rs` | `TerrainShaping`, `soft_cap` |
| `insolation.rs` | `Insolation` |
| `energy.rs` | `AlbedoModel`, `EnergyBalance`, `seed_temperature`, `add_energy_stage[_with]` |
| `error.rs` | `Error`, `Result` |
| `tests/acceptance.rs` | Part 1 acceptance tests |
| `tests/part2.rs` | Part 2 acceptance tests (15) |

## Mesh (exact)
```rust
pub struct Mesh {
    pub n_cells: usize,
    pub area: Vec<f64>,            // m^2 per cell
    pub centroid: Vec<Vec3>,       // world position (sphere: on surface, radius R)
    pub up: Vec<Vec3>,             // unit vector, opposite gravity
    pub adj_offsets: Vec<u32>,     // len n+1
    pub adj: Vec<u32>,             // neighbours, sorted ascending per cell
    pub edge_len: Vec<f64>,        // parallel to adj: shared edge length (m)
    pub edge_dist: Vec<f64>,       // parallel to adj: centre-to-centre distance (m)
    pub boundary_len: Vec<f64>,    // per cell: edge length facing outside the domain
}
// accessors: neighbors(i), edge_lens(i), edge_dists(i), degree(i), total_area(), validate()
```
Flux coefficient across an edge = `edge_len / edge_dist`. Conservative schemes must use it so sums stay exact.

## Domain (exact)
```rust
pub struct Domain {
    pub kind: DomainKind,                 // Sphere | Cube | FlatRect | FlatDisc | Custom(String)
    pub mesh: Mesh,
    pub frames: Vec<SurfaceFrame>,        // {up, north, east}; east = north x up
    pub gravity: Vec<Vec3>,               // acceleration vector per cell (m/s^2)
    pub lights: Vec<Light>,               // used by Insolation; push Light::directional/point
    pub media: Media,                     // default Media::earth_like()
    pub boundary: BoundaryKind,           // Closed | Wrap | Wall | Void | OpenSky | SeaFloor
    pub layers: VerticalStructure,        // {atmosphere_layers: 4, ocean_layers: 3} defaults, unused yet
    pub spin_axis: Vec3,                  // unit rotation axis (= north_ref)
    pub seed: u64,
}
// Domain::new(kind, mesh, gravity, boundary, north_ref, seed) validates the mesh and builds frames.
// Module-facing API: n_cells(), neighbors(i), area(i), up(i), gravity_at(i), gravity_magnitude(i)
```
Generators:
- `generate_sphere(SphereParams{radius, subdivision 0..=9, surface_gravity, seed})`: icosahedron subdivided then dualled; cells = 10*4^L+2 (12 pentagons). Cell edges are great-circle arcs between triangle centroids, so areas tile the sphere exactly. Gravity toward centre. "North" = +z projected. No oblateness yet.
- `generate_cube(CubeParams{half_side, cells_per_edge, surface_gravity, seed})`: 6*N*N square cells, index `face*N*N + j*N + i`, faces +x,-x,+y,-y,+z,-z. Cross-edge adjacency by folding the off-face point around the edge. Every cell has degree 4 (corner cells get their 4th neighbour across the edge; the 3-way corner diagonal is not adjacent). `up` is radial (tilts toward edges/corners), gravity toward the centre, so potential = g0*|centroid| (derive when needed).
- `generate_plane(PlaneParams{shape: Rect{nx,ny}|Disc{diameter_cells}, cell_size, boundary: Wrap|Wall|Void, surface_gravity, seed})`: square grid, 4-neighbour. Wrap only for rectangles with both sides >= 3. Outside edges recorded in `boundary_len`. Gravity (0,0,-g), north = +y.

## FieldStore (exact)
- Storage: `BTreeMap<String, Field{layers, data: Vec<f64>, version}>`, layout `data[cell*layers + layer]`, all f64.
- All mutation goes through **edit groups**: `begin_group(label)`, `end_group() -> Option<commit id>`. Calls made outside a group auto-wrap in a one-call group.
- API: `create(name, layers, init)`, `put(name, layers, Vec<f64>)` (create or replace), `write(name, |&mut [f64]| ...)`, `data(name)`, `get(name)`, `layers(name)`, `has`, `names`, `undo()`, `redo()`, `current_version()`, `diff(name, since_version) -> Diff{entries:(idx,old,new), created}`, `with_max_history(n)` (default 64).
- Commits store sparse deltas (or whole data for creation). Ids are monotonic and never reused. `diff` against a trimmed version returns `Error::HistoryTruncated`.

## Module and scheduler (exact)
```rust
pub trait Module: Send {
    fn spec(&self) -> ModuleSpec;                 // name, inputs[], outputs[], anomaly_hooks[]
    fn params(&self) -> &ParamSet;                // BTreeMap<String,f64> wrapper
    fn set_param(&mut self, name: &str, value: f64) -> Result<()>;
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, dirty: &DirtyRegion) -> Result<()>;
}
pub enum DirtyRegion { All, Cells(Vec<u32>) }   // sorted+dedup via DirtyRegion::cells(..)
pub struct FeedbackLoop { name, members: Vec<String>, watch: Vec<String>, relaxation: f64, tolerance: f64, max_iters: usize }
```
Scheduler rules:
1. Edge P->C when C reads a field P writes. A module that reads and writes the same field is an in-place update (no self edge). Two producers of one field: `MultipleProducers`.
2. Declared `FeedbackLoop` members collapse into one node, run in declared order each pass. Any other cycle: `Error::Cycle(names)`. A module may be in at most one loop.
3. Topological order is Kahn with lowest-registration-index tie-break, so order is deterministic.
4. Loop iteration: snapshot watched fields, run members, relax `x = old + w*(new-old)` (skipped when w=1), residual = max over watched fields of `max|dx| / max(max|x|, 1e-12)`, stop at `<= tolerance` or `max_iters`. Empty `watch` means all member outputs. Non-finite values abort with `NonFinite`. Default relaxation 0.5, tolerance 1e-6, max_iters 200.
5. Inputs must exist when a module runs (`MissingInput`); declared outputs must exist afterwards (`MissingOutput`). Fields no module produces are listed by `external_inputs()`.
6. One `run` = one undo group; a failed run is rolled back.

## Part 2: modules, fields, parameters
Pipeline wiring (scheduler derives the order): `tectonics -> terrain` (stage A); `insolation -> [albedo <-> energy_balance]` (stage B). `elevation` from `terrain` feeds `albedo`/`energy_balance`/optional shadowing. If no terrain is run, supply `elevation` yourself (it then appears in `external_inputs()`).

| Module | Inputs | Outputs | Parameters (default) |
|---|---|---|---|
| `tectonics` | none | `plate_id`, `crust_continental` (0/1), `boundary_type` (0 none, 1 collision, 2 subduction, 3 rift, 4 transform), `tectonic_elevation` (m), `rock_strength` (rel., granite~1, clamped 0.3..2), `volcanism` (0..1) | n_plates 12, continental_fraction 0.4, plate_speed 1, boundary_width_m 0 (auto = 3x mean centre distance), hotspots 3, seed_offset 0, base_continental_m 400, base_oceanic_m -3800, max_uplift_m 8000, max_trench_m 6000, rift_depth_m 1500, ridge_height_m 2000, hotspot_uplift_m 1200, transform_threshold 0.15 |
| `terrain` | `tectonic_elevation`, `rock_strength` | `elevation` (m, sea level 0), `slope` (max neighbour gradient) | noise_amp_m 1000, noise_wavelength_m 0 (auto = extent/5), noise_octaves 5, max_slope 0.05, thermal_iters 8, thermal_rate 0.15, peak_ref_m 9000, depth_ref_m 11000, cap_knee 0.6, seed_offset 0 |
| `insolation` | `elevation` only if `.with_shadowing(true)` | `insolation` (W/m^2, day mean at `season`), `insolation_annual` if `.with_annual(n)` | season 0, diurnal_samples 24, shadow_hops 3, shadow_azimuth_deg 30 |
| `albedo` | `temperature`, `elevation` | `albedo`, `ice_fraction` | sea_level 0, albedo_land 0.22, albedo_water 0.06, albedo_ice 0.60, cloud_albedo 0.20 (placeholder), ice_t_lo 263.15, ice_t_hi 273.15 |
| `energy_balance` | insolation field (default `insolation`, change with `.with_insolation_field`), `albedo`, `elevation` | `temperature_sl` (sea-level equivalent, K), `temperature` (surface, K) | transport_coeff 0.55, greenhouse 0.39, t_init 288, newton_iters 8, cg_tol 1e-8, cg_max_iters 3000, lapse_fraction 0.65, sea_level 0, max_step_k 40 |

Ice-albedo loop: `add_energy_stage(&mut scheduler)` registers insolation + albedo + energy_balance and declares loop `ice_albedo` (members `[albedo, energy_balance]`, watch `[temperature_sl, temperature]`, relaxation 0.5, tolerance 2e-3 (~0.6 K), max 50 passes). Call `seed_temperature(&mut fields, 288.0)` first: the loop needs `temperature` to exist before pass 1, and any existing `temperature_sl` is used as a warm start (the mechanism for later incremental re-solves).

Geometry rules worth knowing:
- Light: directional `direction` = star direction at the equinox. With `orbital_period_s > 0` the star circles an orbit normal tilted by `axial_tilt_rad` from `Domain::spin_axis` (declination = asin(sin(tilt) sin(2 pi season))); with `day_length_s > 0` the result is the mean over `diurnal_samples` rotation phases; `day_length_s = 0` gives a static light (flat worlds, tidally locked). Point lights are static with inverse-square falloff from `reference_distance_m`. No latitude is used anywhere; only `up`, centroids and light vectors.
- Energy balance: per cell `A[(1-alpha)Q - (1-greenhouse) sigma T^4] + K sum (l/d)(Tj - Ti) = 0`, `K = transport_coeff * total_area/(4 pi)`. Newton + Jacobi-preconditioned CG. Lapse = `lapse_fraction * |g| / cp_atmosphere`; no atmosphere means no greenhouse and no lapse.
- Gravity dependence in terrain: noise amplitude ~ g_ref/g; relief cap `rock_strength * peak_ref_m * g_ref/g` (trench depth cap `depth_ref_m * g_ref/g`); slope limit `max_slope * g_ref/g * strength`. All use per-cell `|g|`, so gravity anomalies will work unchanged.
- Plates: grown by multi-source Dijkstra on mesh adjacency; boundary effects spread within each plate with exp(-d/L) (max-product). Painted plates via `Tectonics::with_painted(PaintedPlates{plate_of_cell, plates})`.

## Parameters from Part 1

Generators: radius, subdivision, half_side, cells_per_edge, cell_size, shape, boundary, surface_gravity, seed. Test modules: `diffusivity`, `steps`, `target`, `gain`, `value`. No physical field names are reserved yet.

## Decisions made
- (Part 2) `tectonics` writes `tectonic_elevation`; `terrain` is the only writer of `elevation` (the scheduler forbids two producers).
- (Part 2) Energy balance is equilibrium per season: no thermal inertia, so seasonal swings are exaggerated (e.g. high-latitude swing of tens of K).
- (Part 2) Albedo defaults give Earth-like results on a 70% ocean test world: global mean ~286 K, equator ~299 K, poles ~259 K, ~14% ice. An all-land world with the same defaults falls into the snowball state (bistability is real, not a bug).
- (Part 2) Loop tolerance 2e-3 chosen so every tested season converges in <= 17 passes (tolerance 1e-4 needs ~25).
- (Part 2) `max_slope` is per mesh edge, so it is resolution dependent.
- f64 fields (conservation tests); revisit f32 at the GPU stage.
- `Vec3` is a plain array; swap for `glam` when SIMD/GPU interop matters.
- Cube `up` is radial, not the face normal (matches "gravity toward centre").
- Wall and Void behave identically (zero flux) in Part 1; Void semantics (loss/waterfall) belong to the fluid and hydrology parts.
- Module named `generators` (not `gen`, a reserved word in edition 2024).

## Open decisions for Part 3+
0. Thermal inertia / seasonal lag (blend of seasonal and annual insolation, or a heat-capacity state). Ocean vs land heat transport differences. Sea level as a global parameter instead of `sea_level` per module.
0b. Resolution-independent slope limit; hotspots tied to plate motion; `rock_type` field (spec lists it) not yet produced.
0c. Shadowing ignores planetary curvature and point lights; horizon test is a 3-hop, 30 degree azimuth approximation.
1. Do modules need an explicit layer-aware Domain query API, or is `layers` + `FieldStore` enough?
2. Per-field influence radius for `DirtyRegion` expansion: where is it declared (ModuleSpec vs Domain)?
3. Provenance recording hook (Part 9): per-write attribution needs a module-id parameter on `FieldStore` writes.
4. Oblate sphere and cube-sphere meshes (spec lists both); only the geodesic sphere is built.
5. Sky-island/3D domains need a mesh type with volumes and per-island gravity; current `Mesh` is surface-only.
6. Whether `boundary_len` should also carry boundary kind per edge (needed once wrap and wall mix on one mesh).

## Verification status
- Part 1: compiled and all tests passing (reported by the project owner).
- Part 2 compiled: **no** (no Rust toolchain in the build sandbox). Static scan of the new files found no unbalanced delimiters or overflowing literals.
- Part 2 numerics validated in NumPy mirrors (`reference/energy_mirror.py`, `geo_mirror.py`, `run_*.py`; the mirror builds the same sphere/cube/plane meshes): Earth-like sphere eq-pole 40 K; seasons +/-45 K at poles and exactly zero seasonal change at zero tilt; plane with overhead point sun cools 58 K centre to rim, monotonic; cube belt-cap 32 K with cross-edge neighbour differences 1.26x within-face; ice-albedo loop 9 to 17 passes; tectonics collision/rift/subduction/transform behave as asserted; gravity 3/9.81/25 gives peaks 9.4/8.6/4.6 km; thermal erosion conserves volume to 1e-16. Random-seed numbers are not bit-identical between Python and Rust (different RNG), so thresholds were chosen with wide margins.
- Not validated: type/borrow errors in the Rust, and exact iteration counts in Rust (expected to match the mirror because the algorithm is identical).
