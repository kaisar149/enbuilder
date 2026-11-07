# ARCHITECTURE.md (after PART 4a-2d: beta term, BiCGSTAB, western intensification; 4a-2b, 4a-2c and 4a-2d all compiled and passing)

**Part 4a-2d (latest session, VERIFIED)** adds to `ocean.rs`: `OceanAdvection`, `OceanGraph::advection`, `OceanGraph::solve_advective` (Jacobi-preconditioned BiCGSTAB), `ComponentSolve.restarts`; to `ocean_currents.rs`: the beta term (`beta_on` default 1), `boundary_layer_report`, `boundary_layer_warning`, `BoundaryLayerReport`, `DEFAULT_R_FRICTION` (now 1e-5), new input `beta`, new output `boundary_layer_m`; `tests/part4a2d.rs` (16 tests), `examples/gyre_probe.rs`. This session had cargo 1.97 offline (`gpu` feature stripped from `Cargo.toml` for the sandbox build only; the delivered `Cargo.toml` is unchanged). **Whole crate: every suite passes, zero warnings from the new code.** This also confirms that the previously UNVERIFIED 4a-2b (10 tests) and 4a-2c (12 tests) compile and pass as written (4a-2c needed only test edits for the new defaults, see "Part 4a-2d"). See "Part 4a-2d".

**Part 4a-2a (latest session, VERIFIED)** adds `OceanGraph`, `CgConfig`, `ComponentSolve`, `OceanSolution` to `ocean.rs`, `Error::NoConvergence`, `tests/part4a2a.rs` (11 tests). This session had a Rust toolchain (cargo 1.97, offline, `gpu` feature stripped from `Cargo.toml` only for the sandbox build; the delivered `Cargo.toml` is unchanged). **Whole crate: all suites pass, zero warnings** (this also confirms `part4a1` (12/12; the lake-fixture fix works) and `part3c5` (7) compile and pass; the 3c-5 *benchmark* tables remain PENDING because no benchmark was run). No physics added; nothing in the pipeline reads `OceanGraph` yet.

Crate: `enbuilder` (Rust edition 2024, std only, no dependencies). Part 1 compiled and passed its tests (reported by the project owner). Parts 2 and 3a are **written but not yet compiled** (no Rust toolchain in the build sandbox; see "Verification status").

**Part 3c-5 (latest session, UNVERIFIED)** adds the backend switch `add_atmosphere_stage_stepped_with(&mut Scheduler, Backend)` and `add_climate_pipeline_with`, the `Backend` enum, output-only timing params on `wind`, `testworld.rs`, `tests/part3c5.rs`, `examples/bench_wind.rs`, `examples/compare_backends.rs`. **This session's sandbox had no Rust toolchain and no network, so nothing was compiled, tested or benchmarked.** Every number in the 3c-5 tables below is marked PENDING; run the commands in "Part 3c" and paste the output. Expect small compile errors on first build; please send them back.

**Part 3c-4 (previous session)** adds `projection.rs`, `tests/part3c4.rs` (4 always-on + 1 `gpu` test) and `transport::project_flux_counted` (iteration count; `project_flux` unchanged in behaviour). Default build: 98 tests, zero warnings. `moisture_transport` is NOT changed and still projects on the CPU in f64. As before, the `gpu` feature (WGSL + wgpu calls) was never compiled or run (no crates.io, no adapter in the sandbox); the algorithm is verified through CPU emulation of the identical kernels.

**Part 3c-3 (previous session)** adds `wind_gpu.rs`, `tests/part3c3.rs` (7 always-on + 2 `gpu` tests), `GradientOp::coef()`, and `SteppedWind` params `backend`, `steps_per_pass`. Default build: 94 tests, zero warnings. As in 3c-2a the `gpu` feature (wgpu code and WGSL) was NOT compiled or run (no crates.io, no GPU in the sandbox); the kernel algorithm, buffer layout, double buffering, reduction and host loop ARE verified through a CPU emulation of the identical kernels.

**Part 3c-2a (previous session)** adds `backend.rs`, `gpu/mod.rs` (feature `gpu`), `tests/part3c2.rs`, `Error::NoGpu`. See section "Part 3c-2a". Default build: 87 tests pass, zero warnings. **The `gpu` feature was NOT compiled or run** (build sandbox has no crates.io access and no GPU).

**Part 3c-1 (previous session)** adds `wind_stepped.rs`, `tests/part3c1.rs` (10 tests) and a small refactor of `wind.rs` (`surface_friction`, `balance`, `soft_cap`, `PARAMS` made `pub(crate)`; behaviour unchanged). Whole crate: 82 tests pass, no warnings (cargo 1.97, release and default profile). Only 3c-2 onward (GPU port) remains in Part 3.

Part 3a added `gradient.rs`, `pressure.rs`, `wind.rs`, `tests/part3.rs` and the `lib.rs` exports; nothing in Parts 1-2 changed. Part 3 is split: **3a diagnostic pressure + wind (done, tests pass)**, **3b-1 evaporation sources (done, this session)**, 3b-2 moisture transport (done), 3b-3 lift + precipitation (done), **3b-4 budget, seasons, tuning (done, this session)**, 3c GPU stepped solver (the only remaining Part 3 piece). 3b-1 adds `moisture.rs`, `tests/part3b1.rs`, `reference/evap_mirror.py`; nothing else changed. Parts 1, 2, 3a and 3b-1 all compile and pass `cargo test` (verified in the build sandbox, 47 integration + unit tests).

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
| `gradient.rs` | `GradientOp::new(&Domain)`, `.gradient(&Domain, &[f64]) -> Vec<Vec3>`: topology-agnostic least-squares tangent gradient |
| `pressure.rs` | `PressureModel`, `add_atmosphere_stage(&mut Scheduler)` |
| `wind.rs` | `DiagnosticWind` (+ shared `pub(crate)` helpers) |
| `backend.rs` | `ComputeBackend`, `Cpu`, `CpuF32`, `BackendKind`, `make_backend`, `PackedMesh`, `laplacian_f64`, `laplacian_f32_emulated`, `to_f32/to_f64`, `max_rel_error` |
| `gpu/mod.rs` (feature `gpu`) | `GpuContext`, `GpuLaplacian`, `Gpu`, `LAPLACIAN_WGSL` |
| `ocean.rs` | `SeaLevel`, `OceanMask`, `WindStress`, `add_ocean_mask_stage`, `add_ocean_stage`, pure fns `label_components`, `ocean_cells`, `coast_distance`, `stress_vector` (Part 4a-1); `OceanGraph`, `CgConfig`, `ComponentSolve`, `OceanSolution` (Part 4a-2a); `OceanAdvection`, `OceanGraph::advection`, `OceanGraph::solve_advective` (Part 4a-2d) |
| `projection.rs` | `ProjConfig`, `ProjResult`, emulated kernels `k_flux_div/k_dot/k_matvec/k_flux_correct`, `project_f32_emulated`, `divergence_residual_f64`, `correct_f64`; feature `gpu`: WGSL sources + `project_gpu` |
| `wind_gpu.rs` | `StepStatic`, `StepConfig`, `StepResult`, packing, emulated kernels `k_pressure_gradient/k_momentum_step/k_residual_reduce`, `solve_f32_emulated`; feature `gpu`: WGSL sources + `solve_gpu` |
| `wind_stepped.rs` | `SteppedWind`, `add_atmosphere_stage_stepped(&mut Scheduler)` |
| `budget.rs` | `WaterBudget`, `SeasonReport`, `run_season`, `add_climate_pipeline`, `mean_precip_mm_day`, `mean_evap_mm_day` |
| `precipitation.rs` | `PrecipParams`, `PrecipState`, `terms()`, `column_capacity`, `dry_adiabatic_lapse`, `PRECIP_PARAMS` (pure physics, no module) |
| `transport.rs` | `MoistureTransport`, `edge_flux`, `flux_divergence`, `project_flux`, `stable_dt`, `advect`, `advect_with_sink`, `MoistureTransport::{new, with_precipitation}`, `add_moisture_transport_stage`, `add_moisture_precip_stage` |
| `moisture.rs` | `MoistureSource`, `saturation_vapour_pressure(T_K) -> Pa`, `add_moisture_source_stage(&mut Scheduler)` |
| `energy.rs` | `AlbedoModel`, `EnergyBalance`, `seed_temperature`, `add_energy_stage[_with]` |
| `error.rs` | `Error`, `Result` |
| `tests/acceptance.rs` | Part 1 acceptance tests |
| `tests/part2.rs` | Part 2 acceptance tests (15) |
| `tests/part3.rs` | Part 3a acceptance tests (10) |
| `tests/part3b1.rs` | Part 3b-1 acceptance tests (7) |
| `tests/part3b2.rs` | Part 3b-2 acceptance tests (10) |
| `tests/part3b3.rs` | Part 3b-3 acceptance tests (8) |
| `tests/part4a1.rs` | Part 4a-1: ocean mask, coast distance, wind stress (12 tests, pass) |
| `tests/part4a2a.rs` | Part 4a-2a: ocean operator and CG on synthetic problems (11 tests, pass) |
| `ocean_forcing.rs` | `CurlOp`, `curl_of`, `curl_of_masked`, `beta_of`, `unpack_vec3`, `WindStressCurl`, `BetaTerm`, `add_gyre_forcing_stage` (Part 4a-2b, compiled and passing) |
| `tests/part4a2b.rs` | Part 4a-2b: curl and beta analytic tests (10 tests, pass) |
| `ocean_currents.rs` | `OceanCurrents` (module `ocean_currents`), `add_ocean_currents_stage`, `current_from_psi`, `coast_vectors`, `coast_outflux`, `apply_coast_slip`, `clamp_speed` (Part 4a-2c); beta term, `boundary_layer_report`, `boundary_layer_warning`, `BoundaryLayerReport`, `DEFAULT_R_FRICTION` (Part 4a-2d); all compiled and passing |
| `tests/part4a2c.rs` | Part 4a-2c: gyres, linearity, coast flux, divergence, clamp (12 tests, pass; module pinned to beta off and r 5e-8 by `run_with`) |
| `tests/part4a2d.rs` | Part 4a-2d: BiCGSTAB on synthetic non-symmetric problems, beta term, western intensification, resolution warning (16 tests, pass) |
| `examples/gyre_probe.rs` | `cargo run --release --example gyre_probe -- <level> <r1> <r2> ...`: west/east peak speeds with beta off and on for the two-strip gyre world |
| `tests/part3c4.rs` | Part 3c-4: f32 projection vs CPU f64 |
| `tests/part3c3.rs` | Part 3c-3: f32 kernels vs CPU f64 stepped wind |
| `tests/part3c2.rs` | Part 3c-2a: 5 always-on tests + 2 `gpu` tests (skip without adapter) |
| `tests/part3c1.rs` | Part 3c-1 stepped wind tests (10) |
| `tests/part3b4.rs` | Part 3b-4 budget / seasons / Earth-like tests (7) (the PART file calls it `part3b.rs`) |
| `examples/tune.rs` | `cargo run --release --example tune <level> module.param=value ...`: prints budgets, zonal P/E/V/div per season, ridge band |

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

## Part 3a: pressure and wind (diagnostic)

Wiring: `add_atmosphere_stage(&mut scheduler)` registers `pressure` and `wind`; the scheduler orders `pressure -> wind` from their fields. Needs `temperature_sl`, `temperature`, `elevation` (from Part 2 or supplied). Both modules write only new fields, so they sit after the ice-albedo loop and do not feed back yet (sea-temperature/wind and vegetation loops come later).

| Module | Inputs | Outputs | Parameters (default) |
|---|---|---|---|
| `pressure` | `temperature_sl`, `temperature`, `elevation` | `pressure_sl` (Pa, sea-level reduced), `pressure` (Pa, surface), `pressure_upper` (Pa, ~500 hPa analogue), `air_density` (kg/m^3, surface), `coriolis_f` (1/s), `band_coord` (distance from dynamical equator / L_H, -1 = undefined) | rotation_period_s 86164 (0 = no rotation), coriolis_scale 1, coriolis_const 0 (nonzero = f-plane), sea_level_pressure_pa 101325, gas_constant 287.05, thermal_coeff 0.04, band_coeff 0.05, hadley_scale 2.5, contrast_k 3.36, upper_ratio 0.7, sea_level 0 |
| `wind` | `pressure_sl`, `pressure_upper`, `air_density`, `coriolis_f`, `elevation` | `wind` (3 layers: world-space tangent vector, m/s), `wind_upper` (3 layers), `wind_speed`, `wind_east`, `wind_north` (surface) | drag_ocean 1.5e-5, drag_land 3e-5, drag_rough_ref_m 500, drag_upper 8e-6, upper_density_ratio 0.5, stability_n 0.01, blocking_gain 1 (0 = off), max_wind 80, sea_level 0 |

Model (reduced order, all from mesh adjacency, `up`, spin axis and `|g|`; no latitude anywhere):
- Coriolis `f = coriolis_scale * 2 Omega (spin_axis . up)`; `Omega = 2 pi / rotation_period_s`. On a plane the spin axis lies in the surface so `f = 0` unless `coriolis_const` is set. Tidally locked: pass a long `rotation_period_s` (or 0) and give the light `day_length_s = 0`.
- `pressure_sl = p0 + thermal + band`, area mean forced to `p0` (`p0 = sea_level_pressure_pa * atmosphere.density / 1.2`). Thermal: warm = low. Band: `-A cos(pi d / L_H)` with `d` = mesh-graph distance from the contour where `f` changes sign, `L_H = sqrt(5/3 * hadley_scale * R * dT) / Omega_eff` (Held-Hou scaling), `dT = contrast_k * std(T_sl)`. Gives ~3 cells per hemisphere at Earth rotation (L_H ~ 3000 km), 1 at a quarter of the rate, ~9 at triple. This is a diagnostic stand-in for emergent bands; deliverable 3c replaces it.
- Surface pressure hydrostatic with local `|g|` (so gravity anomalies change it). `pressure_upper = 0.5 p0 - upper_ratio * anomaly` (reversed aloft).
- Wind: `u = (r a - f k x a) / (r^2 + f^2)`, `a = -grad(p)/rho`; friction by water/land and roughness (mean `|dz|` to neighbours, sea floor ignored); up-slope component removed by `b = gain * rise / (rise + |u|/N)` (mountain deflection); `tanh` soft cap at `max_wind`. Upper level uses `pressure_upper`, density `upper_density_ratio * air_density`, weak drag, no blocking.
- No atmosphere (`media.atmosphere = None`): all pressure, density and wind fields are zero.
- Reference numbers (NumPy mirror, sphere level 4, T = 300 - 40 sin^2 lat, all ocean): L_H 3002 km; pressure extrema in the zonal mean 0 / 3 / 9 at 0.25x / 1x / 3x rotation; zonal surface wind 0-30 deg -9.3, 30-60 deg +2.5, 60-90 deg -2.9 m/s; mean speed 7.5, max 23 m/s; tidally locked: surface +3 m/s toward the substellar point, upper -7 m/s.

Test-time facts: `GradientOp` treats an edge as wrapped when the centroid gap exceeds 1.5x `edge_dist` and flips the direction.

## Part 3b-1: evaporation sources (no transport yet)

Wiring: `add_moisture_source_stage(&mut scheduler)` registers `moisture_source`; the scheduler orders it after the ice-albedo loop and `wind`. Needs `temperature`, `ice_fraction`, `elevation`, `wind_speed`. Writes only the new field `evaporation`; nothing reads it yet.

| Module | Inputs | Outputs | Parameters (default) |
|---|---|---|---|
| `moisture_source` | `temperature` (K), `ice_fraction`, `elevation`, `wind_speed` | `evaporation` (kg/m^2/s per cell, >= 0; 1 kg/m^2 = 1 mm of water) | k_ocean 1.8e-8 (kg/m^2/s/Pa), land_availability 0.3, relative_humidity_ref 0.5, wind_base 0.5, wind_coeff 0.1, sea_level 0, max_evap 1e-4 |

Law: `E = k_ocean * avail * (1 - ice_fraction) * e_sat(T) * (1 - RH_ref) * (wind_base + wind_coeff * wind_speed)`, clamped to `[0, max_evap]`. `avail` = 1 where `elevation < sea_level`, else `land_availability`. `e_sat` = Magnus, `611.2 exp(17.62 Tc / (243.12 + Tc))` Pa (0 C 611.2, 20 C 2333, 30 C 4234; all within 0.3% of tables), T clamped to >= 150 K. No atmosphere: field is all zero. Calibration: 300 K ocean, 7 m/s -> ~3.3 mm/day; 288 K -> ~1.6; 260 K -> ~0.2. Params validated (`RH` and `land_availability` in 0..1, rates >= 0).

Notes for 3b-2:
- `evaporation` is a flux density; multiply by `mesh.area` for kg/s into the air column. Total-water conservation is tested in 3b-2 (sources here, sinks in precipitation).
- `max_evap` can break exact linearity in `k_ocean` if hit; tests raise it. Hot test worlds (> ~315 K) will hit the cap.
- `albedo` is not an input (the spec listed it but no use exists yet). Add it when net-radiation limiting of evaporation is introduced.

### Decisions (3b-1)
- Sublimation over ice is ignored: `(1 - ice_fraction)` removes evaporation proportionally, so fully iced cells give exactly 0 and partially iced cells scale linearly. Open: add a small sublimation term (cold, dry katabatic air) later.
- Ocean vs land is decided by `elevation < sea_level` (lakes later by hydrology). `land_availability` is a global constant until soil moisture / biomes exist.
- Air humidity is a fixed fraction of saturation here; in 3b-2 `e_air_ref` should come from the transported humidity, making `E` self-limiting.
- Added parameter `wind_base` (0.5) so the spec's `f = 0.5 + 0.1 u` is data, not code.

## Part 3b-2: conservative moisture transport

**Decision: Option A.** The 3a wind is projected to a divergence-free *edge-flux* field; vapour is advected with the corrected flux. Done in flux space (not on the vector wind), so conservation is exact by construction.

Wiring: `add_moisture_transport_stage(&mut scheduler)`; needs `wind` (3a) and `evaporation` (3b-1). Full recompute (ignores `DirtyRegion`).

Definitions
- Edge flux `F_k = 0.5 (u_i.n + u_j.n) * edge_len_k` (m^2/s, outward from cell i across adjacency entry k, parallel to `mesh.adj`). `n` = unit tangent direction i->j at the edge midpoint (centroid difference with the midpoint-up component removed; sign flipped for wrapped edges, same rule as `GradientOp`). Computed once for i<j and mirrored, so `F_ji = -F_ij` exactly. Wall/void edges carry no flux.
- Projection: `A phi = div(F)`, `A = sum_j w_ij (phi_i - phi_j)`, `w = edge_len/edge_dist`; Jacobi-CG on the graph Laplacian (constant mode removed from the rhs). `F' = F - w (phi_i - phi_j)`, so `sum_k F'_k = 0` per cell. Own CG in `transport.rs` (the energy CG has a different operator). phi is in m^2/s.
- Advection: `A_i dV_i/dt = sum_k[-max(F'_k,0) V_i + max(-F'_k,0) V_j] + A_i E_i`, forward Euler. Positive for CFL <= 1; a `max(0)` guard remains for rounding.
- Time step: `dt_max = cfl * min_i A_i / sum_k max(F'_k,0)`; the run uses `nsub = ceil(T/dt_max)` equal sub-steps; more than `max_substeps` is an `InvalidParam` error (never silently unstable).

Outputs: `vapor` (kg/m^2); `wind_flux`, `wind_flux_corrected` (m^2/s, layers = max cell degree, layer q = q-th neighbour in adjacency order, padded 0); `wind_corrected` (3 layers, `wind - GradientOp(phi)`, a least-squares reconstruction for diagnostics, not exactly divergence-free); `wind_divergence` (raw, 1/s, kept for convective rain in 3b-3); `divergence_residual` (corrected divergence / max_i sum_k |F_ik|, dimensionless).

| Module | Inputs | Outputs | Parameters (default) |
|---|---|---|---|
| `moisture_transport` | `wind`, `evaporation` | `vapor`, `wind_flux`, `wind_flux_corrected`, `wind_corrected`, `wind_divergence`, `divergence_residual` | transport_time_s 2592000 (30 d, was 5 d), init_residence_s 864000 (10 d, was 4 d), warm_start 0, cfl 0.5, max_substeps 100000, projection_tol 1e-12, projection_max_iters 20000 |

Initial state: if `warm_start = 1` and `vapor` exists, start from it; otherwise `V0 = evaporation * init_residence_s`. Default is cold start because there is no rain sink yet: warm starting repeatedly would accumulate water without bound. 3b-3 must turn warm start on inside its feedback loop.

Tolerances: CG relative residual 1e-12; tests require `divergence_residual < 1e-6` and mass drift < 1e-10 (sphere L4, cube 14/edge, wrapped and walled planes, plus a source test: mass gain equals `sum(E*A)*T` to 1e-10).

### Decisions and caveats (3b-2)
- Projecting removes large-scale convergence (ITCZ, monsoon lows) from the transport wind. Closed domains force zero net flow through any cross-section, so a uniform wind on a walled plane is (correctly) damped; use a wrapped plane to test uniform flow. Convective/frontal rain in 3b-3 should use `wind_divergence` and `wind_upper`, not the corrected flux.
- First-order upwind is diffusive (blob peak decays); accepted for a "dumb but working" version. Better advection (limited second order) is a later option.
- No saturation cap yet: `vapor` can exceed what air could hold. Condensation (3b-3) is the sink.
- Mountain lift is not modelled here; `elevation` gradient with `wind_flux` is for 3b-3.
- Cube corners: the 3-way diagonal is not adjacent (Part 1), so no flux passes there.
- Not run: stability test at extreme wind (`max_wind` 80 m/s on fine meshes needs many sub-steps; `max_substeps` guards).

## Part 3b-3: lift, condensation and precipitation

**Structure decision:** precipitation is an operator-split sink *inside* `moisture_transport`, not a separate module: the scheduler allows one producer per field and splitting is what creates rain shadows. Each sub-step is: upwind advection + evaporation, then rain removal from the new `vapor`. Two constructors: `MoistureTransport::new()` / `add_moisture_transport_stage` (no rain, unchanged 3b-2 behaviour, inputs `wind`, `evaporation`) and `MoistureTransport::with_precipitation()` / `add_moisture_precip_stage` (module name stays `moisture_transport`; adds inputs `temperature`, `elevation`; adds outputs below and the precipitation parameters). Register one or the other. Spec is fixed at construction, so scheduler `set_param` cannot toggle coupling; use the `*_on` switches instead.

Extra outputs (3b-4 also adds `precip_sat` and, on every transport module, `vapor_start`; kg/m^2/s are run-mean rates = accumulated rain / `transport_time_s`): `precipitation`, `precip_oro`, `precip_conv`, `precip_front`, `lift` (m/s, static).

Definitions (all in `precipitation.rs`)
- Column capacity `W_sat(T,g) = vapor_scale_ratio * 0.622 e_sat(T) / |g|` (kg/m^2). Uses local `|g|`, so gravity anomalies flow through. RH = V / W_sat.
- **Lift** `= lift_gain * max(u_c . grad z_c, 0)`, `u_c = wind_corrected`, `z_c = max(elevation, sea_level)` (sea floor has no lift), gradient from `GradientOp`. Cell crossing time `tau = sqrt(area)/|u_c|`, rise `= lift * tau` (m).
- **Orographic**: cooling `dT = (|g|/cp) * dry_adiabatic_scale * rise`; condensate `C = max(V - W_sat(T - dT), 0)`; removed in dt: `C (1 - exp(-dt/tau))`. Leeward slopes: lift 0, so no orographic term (downslope warming/drying beyond "no new rain" is not modelled).
- **Convective**: `V (1 - exp(-conv_rate dt)) * ramp * hum`, `ramp = clamp((T - conv_threshold_T)/conv_ramp_k, 0, 1)`.
- **Frontal**: `V (1 - exp(-front_gain * conv * dt)) * hum`, `conv = max(-wind_divergence, 0)`. Uses the RAW divergence (`wind_divergence`), not `wind_corrected`, because the projected wind is divergence-free by construction (deviation from the PART file text, as flagged in 3b-2).
- `hum = clamp((RH - rh_trigger)/(1 - rh_trigger), 0, 1)`.
- **Removal rule**: each term is multiplied by `removal_fraction` and its switch; if the three together exceed `V` they are scaled down proportionally (the only coupling between terms); `vapor -= P dt`, accumulated into the outputs. Total water: `vapor_final + sum(P*area)*T = vapor_0 + sum(E*area)*T` (tested to 1e-8, holds to ~1e-12).

| Parameter | Default | Meaning |
|---|---|---|
| lift_gain | 3 (was 1 in 3b-3; tuned in 3b-4) | scales lift (and rise) |
| dry_adiabatic_scale | 1 | multiplies `|g|/cp` (PART file called it `dry_adiabatic_g_over_cp`) |
| vapor_scale_ratio | 0.25 | share of the column air mass that carries vapour |
| sea_level | 0 | |
| oro_on, conv_on, front_on | 1 | process switches (0/1) |
| conv_threshold_T / conv_ramp_k / conv_rate | 298 K / 5 K / 4.6e-5 1/s | convective trigger and rate |
| rh_trigger | 0.5 | RH at which condensation terms start |
| front_gain | 0.3 | fraction of moisture convergence rained out |
| removal_fraction | 1 | scales all terms |
| conv_div_ref / conv_dyn_min / conv_dyn_max | 5e-6 / 0.1 / 3 | convergence gating of convection (3b-4) |
| sat_on / sat_rh / sat_tau_s | 1 / 1 / 21600 | saturation adjustment, 4th term `precip_sat` (3b-4) |

Verified: north-south Gaussian ridge under a westerly gives windward `precip_oro` > 0 and leeward ~0 plus drier lee vapour, on a wrapped plane and on the sphere; zero vapour gives zero rain; switching one term off leaves the other two bit-identical (at fixed vapour); full chain `moisture_source -> moisture_transport` runs through the scheduler.

### Decisions and caveats (3b-3)
- **Gravity sanity test:** the PART file says doubling gravity "reduces" dry-adiabatic cooling per metre. Physically `g/cp` doubles, so the test asserts the lapse rate doubles (and that capacity per column falls with g). Revisit if you really want the opposite.
- Magnitudes are untuned. Dry air (RH << 1) produces no orographic rain (needs enough rise to saturate); that is intended. Tune `vapor_scale_ratio`, `rh_trigger`, `conv_rate`, `front_gain` against a reference Earth-like run before 3c.
- No re-evaporation of rain, no snow/rain split, no soil moisture return (evaporation over land still uses constant `land_availability`). Precipitation is not yet read by hydrology (Part 4).
- `evaporation` is not made self-limiting by humidity yet (open since 3b-1).
- Coupling `elevation` to wind (3a blocking) and rain shadow share terrain but do not feed back; seasonal runs (two seasons) remain a driver-level task: run the pipeline per `season`.
- Full recompute only (`DirtyRegion` ignored). Time-averaged rain rates assume `transport_time_s` > 0.

## Part 3b-4: budget closure, seasons and tuning (Part 3b complete)

**Pipeline** (`add_climate_pipeline`): `insolation -> [albedo <-> energy_balance]` then `pressure -> wind`, `moisture_source`, `moisture_transport` (coupled with precipitation). `run_season(domain, fields, scheduler, season)` sets the insolation `season`, runs the scheduler once and returns a `SeasonReport {season, budget, mean_precip_mm_day, mean_evap_mm_day, passes_energy}`. Call `seed_temperature` first; later seasons warm-start the energy balance from the previous `temperature_sl`. The moisture stage is not in a feedback loop and needs exactly one pass per season. `Scheduler::module_param(module, name)` was added.

**`WaterBudget`** (`WaterBudget::from_fields`, uses new output `vapor_start`): all terms are area-integrated kg: `vapor_start`, `vapor_end`, `evaporation` (rate * T), `precipitation` (rate * T), `residual = vapor_end + rain - vapor_start - evaporation`, `residual_rel`, and `storage_fraction = (vapor_end - vapor_start)/evaporation` (0 = steady state). Closure is at rounding level (1e-15 to 1e-16 observed; tests require 1e-8) on sphere, cube, wrapped plane and a disc, over two seasons, cold and warm start.

**Season note:** `season` 0 and 0.5 are both equinoxes (declination = asin(sin(tilt) sin(2 pi season))), so they give identical climates. The two-season tests use 0.25 and 0.75 (solstices). `Season 0` is used for the tropics vs subtropics check.

### Changes made while tuning (all defaults; record of reasoning)
| Change | Why |
|---|---|
| New 4th rain term `precip_sat` (saturation adjustment: `max(V - sat_rh*W_sat, 0)(1 - exp(-dt/sat_tau_s))`, `sat_on` 1, `sat_rh` 1, `sat_tau_s` 21600) and output `precip_sat`; `precipitation = oro + conv + front + sat` | With only the three PART-file types, mid/high latitudes (no lift, no convergence, too cold for convection) had no rain sink and vapour piled up to 75-90 kg/m^2. Large-scale condensation is also what the PART file's "condensed = excess above saturation" implies. `terms()` now returns 4 values. |
| Convective dynamics factor `dyn = clamp(1 - div/conv_div_ref, conv_dyn_min, conv_dyn_max)` with `conv_div_ref` 5e-6 1/s, min 0.1, max 3 | Without it, rain followed evaporation (peak at 15 deg trades) instead of the ITCZ. Raw 3a divergence is convergent at the equator (-6e-6) and divergent at 15-30 deg (+4e-6), so convection is boosted at the ITCZ and suppressed under subsidence. |
| `transport_time_s` 5 d -> 30 d; `init_residence_s` 4 d -> 10 d | At 5 d precipitation was ~0.4 of evaporation (vapour still spinning up). At 30 d `P/E` = 0.96-1.05 (`storage_fraction` < 0.15) and one pass per season suffices. Runs of 100 d add under 10% more rain. |
| `lift_gain` 1 -> 3 | A 3000 m ridge averaged over ~400 km cells (sphere level 4) understates slopes, so lift is scaled up; with 3 the windward ridge band rains 2.08 vs 1.37 mm/day on the lee side at the summer solstice, and `precip_oro` is exactly 0 leeward. |
| `k_ocean` 1.8e-8 and `removal_fraction` 1 unchanged | Earth-like sphere (30% land cap + 3000 m ridge, level 4): global E = 2.10 / 1.68 / 3.59 mm/day (equinox / summer / winter solstice). |

**Tuned Earth-like results** (sphere level 4, tilt 23.4 deg): global mean precipitation 2.02 (equinox), 1.70 (season 0.25), 3.78 (season 0.75), mean 2.5 mm/day (target 2-4); equinox zonal precipitation about 4-6 mm/day within 15 deg of the equator vs about 1.4 at 20-35 deg; budget residuals 1e-15; energy loop converges in 15-22 passes.

### Open issues (carry into later parts)
1. **No thermal inertia** (Part 2): solstice runs are extreme (summer pole near 300 K, E ~ 8 mm/day, P up to 9 mm/day at 30 deg of the summer hemisphere; the global mean swings 1.7 vs 3.8). Seasonal moisture realism depends on adding a heat-capacity state or blending annual and seasonal insolation.
2. **Sublimation** over ice is still zero; **land heat capacity** and **land-sea monsoons** from `temperature_sl` contrast only.
3. **Moisture feedback to the energy balance:** no latent heat, no cloud albedo, no greenhouse from vapour. Evaporation is not humidity-limited (`e_air_ref` is a fixed fraction), so rain recycles locally; `land_availability` is a constant (soil moisture comes with hydrology).
4. **Convective trigger realism:** crude temperature ramp + RH threshold gated by raw divergence; no CAPE, no diurnal cycle. Tune `conv_*`, `front_gain`, `rh_trigger` against better references.
5. **Transport:** first-order upwind is diffusive; the projected wind has no ITCZ convergence (raw `wind_divergence` is used for rain instead); closed domains force zero net cross-section flow.
6. **Orographic rain** depends on resolution (`lift_gain` compensates); no downslope drying/foehn warming; no re-evaporation of falling rain; no snow/rain split.
7. **Gravity:** the PART-file claim that doubling gravity reduces dry-adiabatic cooling per metre is physically backwards; tests assert the lapse rate scales with `|g|`.
8. Not yet covered: seasonal inertia of vapour (each season cold-starts unless `warm_start` is set), `DirtyRegion` incremental solves, provenance of rain terms, GPU port (3c).

## Part 3c-1: stepped shallow-layer wind (CPU reference)

Module `SteppedWind` (file `wind_stepped.rs`), registered with `add_atmosphere_stage_stepped(&mut s)` (= `pressure` + stepped wind). **Module name is still `wind`**, same inputs and outputs as `DiagnosticWind` (`wind`, `wind_upper`, `wind_speed`, `wind_east`, `wind_north`), so downstream modules and `set_param("wind", ..)` are unchanged. Register this or `add_atmosphere_stage`, never both.

Equation per cell: `du/dt = a - f k x u - (r + drag_quad |u|) u + F_block`, `a = -grad(p_sl)/rho` (constant, from `GradientOp`), `r` = the 3a friction (water/land, roughness; shared function `surface_friction`).
- **Time step rule:** `dt = min(dt_max_s, cfl_wind * min(edge_dist) / max(|u|))`, |u| floored at 1e-3 and capped at `max_wind`. At the default 600 s the CFL term never binds on Earth-size meshes (it would at max wind only for cells < 120 km at 80 m/s).
- **Integrator (deviation from "explicit forward step"):** pressure force explicit; Coriolis + drag implicit per cell (2x2 solve `u = ((1+r dt) u* - f dt k x u*) / ((1+r dt)^2 + (f dt)^2)`). Explicit Euler grows by sqrt(1+(f dt)^2) per step and fails on fast-rotating worlds. Fixed point is exactly the 3a balance; tested independent of dt (150/600/3000 s agree within 0.3 m/s).
- **Blocking as a force:** up-slope component damped implicitly at rate `k_b = r b/(1-b)` (b = 3a blocking fraction; `1-b` floored at 0.02; capped by `block_rate_max`), which reproduces 3a's `(1-b)` removal for a scalar balance. Not bit-equal to 3a with Coriolis or varying b.
- **Convergence:** stop when max |du| over one step <= `steady_tol` for 5 consecutive steps. Reaching `max_steps` returns `Error::InvalidParam` (never an unconverged field). Outcome readable through `Scheduler::module_param("wind", "last_steps" | "last_residual")` (these two are output-only; `set_param` rejects them).
- **Cap:** the state is uncapped; the output gets the 3a tanh soft cap (capping every step acts as extra drag and moves the steady state). Hard clamp of the state at 10x `max_wind` as a runaway guard. `|u| <= max_wind` always holds in the output.
- **Initial state:** rest by default; `spin_up_steps > 0` starts from the diagnostic balance (no blocking); `warm_start = 1` starts from an existing 3-layer `wind` field. No atmosphere: all outputs zero.
- **Upper level:** unchanged 3a diagnostic (open decision 0d).

| Parameter | Default |
|---|---|
| all 9 from `DiagnosticWind` | as 3a |
| dt_max_s | 600 |
| cfl_wind | 0.5 |
| steady_tol | 1e-3 m/s per step |
| max_steps | 20000 |
| spin_up_steps | 0 |
| warm_start | 0 |
| drag_quad (C_d/h, 1/m) | 0 (linear drag only, matches 3a) |
| block_rate_max | 1e-2 1/s |

**Measured results** (earth_t = 300 - 40 sin^2 lat, all ocean, blocking off, Earth rotation):
| | L3 | L4 |
|---|---|---|
| steps to converge (cold start) | 650 | 667 |
| last max abs du per step | 9.6e-4 | 9.6e-4 m/s |
| zonal mean east wind 0-30 / 30-60 / 60-90 deg, diagnostic | -8.554 / +1.996 / -2.628 | -9.273 / +2.489 / -2.894 |
| same, stepped | -8.554 / +1.996 / -2.628 | -9.274 / +2.489 / -2.894 |
| max per-cell speed difference | 0.046 | 0.047 m/s |

Agreement is within 0.1% in the zonal means (requirement was 20%); sign pattern of the three cells identical. Other measurements: tidally locked mean surface flow toward the substellar point diag/stepped 2.97/2.93 m/s (rotation 0) and 2.87/2.85 (30 d); wind-band count (sign changes of zonal-mean east wind) 0 / 3 / 9 at 0.25x / 1x / 3x rotation, identical to the diagnostic; spin-up start converges in 5 steps vs 650 cold; quadratic drag `drag_quad = 2e-6` lowers the L3 mean speed 6.60 -> 5.08 m/s; ridge test (plane, 3000 m): windward-slope wind free 17.5, blocked 12.0 (diagnostic blocked 13.5), far-field 26.7, lee unchanged; stepped wind driving moisture transport and rain closes the water budget to 1.2e-15 (mean P 1.85 mm/day, L3). Bit-identical on repeat runs (sphere, cube, torus).

Caveats: the stepped wind is cold-started per pass (no state carried unless `warm_start`); it still contains no advection term (`u.grad u`), so it is a damped balance in time, not yet a full shallow-water solver; the pressure field it relaxes to is still the 3a parametric one (band term), so 3c-1 does not by itself make bands emergent; that needs advection plus a dynamically evolving pressure/height field (3c-2/3c-3 decision). Cost: ~650 steps x n cells, about 0.05 s at L4 in release.

### Decisions (3c-1)
- Module keeps the name `wind` and the same outputs; stepped and diagnostic are interchangeable at registration time.
- Implicit Coriolis/drag instead of explicit Euler (stability); steady state unchanged.
- Blocking is a damping force, not a per-step projection (a per-step projection would make the steady state depend on dt).
- Open for 3c-2: should the state (`wind`) become a persistent, warm-started field across passes and seasons? Should a height/pressure tendency and momentum advection be added so bands emerge instead of being parametric? WGSL port must keep the same implicit update.

## Part 3c-2a: GPU foundation (code written, GPU path unverified)

**Verification status (read first):** the sandbox has no crates.io access (`index.crates.io` does not resolve) and no GPU adapter, so `wgpu`/`pollster` could not be downloaded and `src/gpu/mod.rs` has **never been compiled**. Verified here: the default build (zero warnings, 87 tests) and the f32 arithmetic of the kernel via CPU emulation. Please run `cargo test --features gpu -- --nocapture` and send back compiler errors and the "GPU adapter:" / "SKIPPED" lines. wgpu's API changes between majors; code targets **wgpu 23** (`request_device(&desc, None)`, `entry_point: "main"`, `cache: None`, `Maintain::Wait`). If you use another major, expect small edits there. The default build is unaffected. Because optional dependencies are still resolved for `Cargo.lock`, the first `cargo build` needs network access once (I removed the stale `Cargo.lock`; cargo regenerates it).

**Decisions**
- Precision: crate stays f64; GPU path is f32; `moisture_transport` (conservation-critical) stays on the CPU in f64. GPU parity tolerance is **relative 1e-4** (`max_rel_error` = max|a-b| / max|b|), not 1e-8. Measured f32 kernel arithmetic vs f64: sphere L4 8.8e-6, cube 14/edge 9.8e-6, plane 64x64 4.1e-6 (10x margin under 1e-4).
- Feature `gpu` (`default = []`) enables optional deps `wgpu = "23"`, `pollster = "0.4"`.
- `trait ComputeBackend { name(), precision(), laplacian(&Domain, &[f64]) }` with `Cpu` (f64, default), `CpuF32` (CPU emulation of the kernel, always available) and `Gpu` (feature). `make_backend(BackendKind)` selects at runtime; `Gpu` without the feature or adapter returns `Error::NoGpu`. Only the Laplacian is on the trait for now; no module takes a backend handle yet (no physics changes in this part). Add operations to the trait as kernels are ported (3c-3).
- Kernel: `out_i = sum_k (edge_len/edge_dist)_k (x[adj_k] - x_i)`, WGSL, workgroup 64, one thread per cell, n = `arrayLength(offsets) - 1`, 5 storage bindings (offsets, adj, coef, x, out). `PackedMesh` = CSR offsets/adj (u32) + f32 coef. Mesh is re-uploaded per `Gpu::laplacian` call; hot loops should hold a `GpuLaplacian`.
- Buffer layer: `GpuContext::{upload_f32, upload_u32, empty_f32, download_f32 (staging buffer), round_trip}`; f64->f32 overflow is an error (`to_f32`).
- Adapter: high-performance first, then fallback adapter, else `NoGpu`.
- Round-trip error: max 1.5e-5 on temperature-like fields (limit f32 eps x magnitude, about 3.7e-5).

**Tests (`tests/part3c2.rs`):** round-trip, packing, CPU conservation, f32-emulated parity on all three generators, NoGpu without the feature; with `--features gpu`: hardware round-trip and Laplacian parity (print "SKIPPED (no GPU)" and pass when no adapter).

## Part 3c-3: stepped-wind kernels (f32), parity with the CPU solver

**Selection:** `SteppedWind` param `backend` (0 = CPU f64 as 3c-1, default; 1 = the f32 kernels emulated on the CPU; 2 = GPU, needs feature `gpu`, else `Error::NoGpu`) and `steps_per_pass` (even, >= 6, default 20). Same outputs and module name; `last_steps`/`last_residual` are reported by all backends.

**Kernels** (one thread per cell, workgroup 64; WGSL constants `WGSL_PRESSURE_GRADIENT`, `WGSL_MOMENTUM_STEP`, `WGSL_REDUCE_1`, `WGSL_REDUCE_2`; CPU twins `k_pressure_gradient`, `k_momentum_step`, `k_residual_reduce`):
1. `pressure_gradient` (once per solve): `a_i = -(sum_k coef_k (pd_j - pd_i)) / rho_i`, `coef` = the CPU `GradientOp` weights (3 f32 per adjacency entry), `pd = p_sl - mean(p_sl)` (keeps f32 differences accurate).
2. `momentum_step` (per sub-step): explicit pressure force, implicit drag + Coriolis, implicit up-slope blocking, hard clamp at 10x max_wind; identical maths to the CPU `SteppedWind`. Reads only `u_in`, writes only `u_out` and `chg`.
3. `residual_reduce`: pass 1 reduces `chg` and `|u|` per workgroup (shared-memory tree) into `partial[2*wg]`; pass 2 (one workgroup, strided) writes `result = [max change, max speed]`.

**Buffers** (all f32 storage): `offsets`, `adj` (u32 CSR), `coef3` (3/adj entry), `pd` (n), `stat` (12/cell: up xyz, r, f, rho, hat xyz, rise, 2 pad), `a` (3n), `u_a`, `u_b` (3n each, world-space tangent vectors), `chg` (n), `partial` (2*ceil(n/64)), `result` (2), plus one 8-float parameter buffer and bind group per sub-step (`dt, cq, stab, kb_max, gain, hard, reset, pad`).

**Double buffering:** sub-step s reads `u_a` when s is even and `u_b` when odd, writes the other. `steps_per_pass` is even, so every pass starts and ends in `u_a`; no read/write hazards, no in-place updates.

**Host loop:** per pass, one submit of `steps_per_pass` momentum dispatches + the 2 reduction dispatches; ONE 2-float readback per pass. `dt` is fixed within a pass: `min(dt_max_s, cfl_wind * min_dx / clamp(max|u| from the previous readback, 1e-3, max_wind))`. The change window is the last `hold` = 5 sub-steps of the pass (the `reset` flag restarts `chg` at sub-step `steps_per_pass - 5`), so the stop test is the same "5 consecutive steps <= steady_tol" as on the CPU, evaluated at pass ends; steps are therefore rounded up to a multiple of `steps_per_pass`. `max_steps` exhausted -> `InvalidParam`. All static data (gradient weights, `f`, drag, `rho`, blocking geometry, pressure) is computed and uploaded once per solve, none per sub-step. The soft cap, `wind_*` outputs and the upper level stay on the host in f64.

**Measured parity** (emulated kernels vs CPU f64, sphere L4, default `steady_tol` 1e-3; metric = max|du| / max|u| over all wind components):
| case | parity | steps cpu / f32 | 
|---|---|---|
| all-ocean, blocking off | 5.4e-4 | 667 / 680 (ratio 1.019) |
| 3000 m ridge, blocking on | 5.4e-4 (ridge region only: 5.6e-4; blocking changes the wind there by 14%, so it is a real test) | 667 / 680 |
| ridge, `steady_tol` 1e-4 | 1.5e-5 | 920 / 920 |
Requirement was 1e-3 and +/-10% steps. The 5e-4 at the default tolerance is the different stopping point of the two loops (cpu stops at step 667, f32 at the pass boundary 680) plus f32 rounding; at tol 1e-4 the two agree to 1.5e-5, i.e. f32 rounding itself is about 1e-5. Tidally locked flow toward the substellar point: 2.93 (f32) vs 2.93 (cpu) at rotation 0, 2.85 vs 2.85 at 30 d. Bit-identical on repeat runs (emulation). The reduction kernel equals brute force for n = 1, 63, 64, 65, 1000, 2562.

**NOT verified:** the WGSL text, wgpu 23 calls and `solve_gpu` have never been compiled or executed. Run `cargo test --features gpu -- --nocapture`; the GPU tests (`gpu_matches_cpu_steady_state`, `gpu_tidal_lock_and_determinism`) print the adapter or "SKIPPED (no GPU)". Likely first-compile issues: wgpu major-version API drift, WGSL validation messages. Determinism on a real adapter is only expected on the same adapter/driver.

### Decisions (3c-3)
- Kernel logic is mirrored by CPU emulation over identical packed buffers so that any later GPU/CPU mismatch points at WGSL or wgpu, not at the algorithm.
- Per-sub-step parameter buffers (not push constants) for portability; `dt` is held per pass, which is exact whenever the CFL term does not bind (default).
- Pressure is uploaded as a deviation from its mean (f32 conditioning).
- Open: persistent device state across passes/seasons (warm start on device); moving `wind_upper` and the cap on device; batching the 4 pipelines into one cached `GpuWindSolver` object instead of rebuilding per solve; projection and moisture transport stay on CPU f64 (3c-4+).

## Part 3c-4: GPU projection (Jacobi-CG with device-side scalars)

**Status:** library only; `moisture_transport` is untouched and still uses the CPU f64 `project_flux`, which **remains the reference for conservation and for the 1e-12 CG tolerance**. Entry points: `project_f32_emulated(&PackedMesh, &flux_f64, &ProjConfig)` and (feature `gpu`) `project_gpu(&GpuContext, ...)`, both returning `ProjResult { phi, fc, iterations, rel_residual }`. Input flux is the CPU `edge_flux` layout (parallel to `mesh.adj`, antisymmetric); output `fc` has the same layout as `project_flux`'s corrected flux (the module's padded `wind_flux_corrected` is a trivial repack).

**Maths:** `div_i = sum_k F_k`; solve `A phi = div - mean(div)`, `A x_i = sum_k w_ik (x_i - x_k)`, `w = edge_len/edge_dist` (f32), Jacobi diagonal `sum_k w`; `F'_k = F_k - w_k (phi_i - phi_j)`. Because both ends of an edge compute the same f32 `w` and `phi_i - phi_j` is exactly antisymmetric, `F'` is exactly antisymmetric in f32 (tested).

**Kernels** (workgroup 64, WGSL constants in `projection::gpu_impl`; CPU twins `k_*`): `flux_div`, `cg_init`, `cg_matvec`, `dot1` + `dot2` (two-pass reduction: per-workgroup tree sum, then one workgroup strided sum + tree; the CPU twin uses the same order), `cg_alpha`, `cg_update_xr` (x += a p; r -= a Ap; z = r/diag fused), `cg_beta`, `cg_update_p`, `flux_correct`. Scalars live in the device buffer `sc[8]` = `[sum, pAp, rz, rz_new, rr, alpha, beta, bad]`; the dot kernels write to `sc[slot]` (slot from a 1-element u32 buffer), so an iteration is 11 dispatches with **no readback** and static bind groups (no per-iteration allocation).

**Host loop / readbacks:** one readback after `div` (its sum gives the mean; constant mode removed from the right-hand side once at the start, as on the CPU, not every iteration: the operator maps to mean-free vectors, f32 drift was not observed to matter), one after init (`rr0` -> `bn`), then **one 8-float readback per `interval` = 16 iterations** (checks `sqrt(rr) <= tol * bn`, and the `bad` flag set when pAp <= 0), plus two readbacks at the end (phi, F'). `max_iters` exhausted -> `InvalidParam`. Iteration count is therefore a multiple of 16.

**Tolerances:** device `tol` = 1e-6 relative (recurrence residual, f32 floor), `max_iters` 20000. This is a **documented relaxation** from the CPU's 1e-12.

**Measured** (emulated kernels vs CPU f64; flux field from the stepped wind on sphere L4, synthetic divergent winds on cube 14/edge and wrapped plane 64x64):
| case | flux error / max abs F | divergence_residual before | after, f32 F' | after, f64 F' from f32 phi | CPU f64 | iters f32 / CPU@1e-6 / CPU@1e-12 |
|---|---|---|---|---|---|---|
| sphere L4 | 1.2e-6 | 3.1e-1 | 1.05e-6 | 1.04e-6 | 2.0e-13 | 80 / 78 / 116 |
| cube 14 | 2.3e-6 | 2.5e-1 | 1.9e-6 | 1.9e-6 | 2.3e-13 | 64 / 59 / 81 |
| plane 64x64 | 1.0e-5 | 2.6e-1 | 7.6e-6 | 7.6e-6 | 3.4e-14 | 48 / 40 / 46 |
Requirements: flux within 1e-4 (met, worst 1.0e-5); `divergence_residual` < 1e-5 on the CPU-side f64 check (met, worst 7.6e-6, **looser than the CPU 1e-6 target and about 7 orders looser than the CPU's own 1e-13**, so the GPU projection does not meet the module's conservation tests, which must keep using the CPU); iterations within 20% (GPU count = CPU count at the same 1e-6 tolerance rounded up to the 16-iteration interval: 80/78, 64/59, 48/40 = +20%, at the limit on the small plane; versus the CPU run to 1e-12 the counts are 80/116, 64/81, 48/46). The residual is dominated by f32 rounding in `F'` and in `phi`, not by the CG stopping point: recomputing `F'` in f64 from the f32 `phi` (`correct_f64`) changes it by under 3%.

**Checks on the CPU after the solve (f64):** `divergence_residual_f64` (the module's definition: max |div F'| / max sum |F|) and `correct_f64`. Use them whenever a GPU projection result is accepted.

**NOT verified:** WGSL text, wgpu 23 calls and `project_gpu` were never compiled or run. `cargo test --features gpu -- --nocapture` runs `gpu_projection_matches_cpu` (prints the adapter or "SKIPPED (no GPU)"). On real hardware the dot-product order is fixed by the kernels, so results should match the emulation to rounding (fma contraction can differ).

### Decisions (3c-4)
- Everything the CG loop needs is on the device (scalars included), so the readback cost is 1 per 16 iterations independent of mesh size.
- Corrected flux is produced in f32 on the device to match the spec; the f64 re-correction from `phi` is offered for the final check but does not change the order of magnitude.
- Open for 3c-5: wiring `projection_backend` into `moisture_transport` (CPU upwind advection would still need `F'` on the host, so decide whether advection also moves to the device); GPU edge-flux construction from the wind (`edge_flux` is still CPU f64); better preconditioning (multigrid) if iteration counts grow with resolution; reducing the f32 floor (compensated sums in the dots, or f64 emulation) if 1e-6 on the plane proves too loose.

## Part 3c: backend switch, precision decisions, benchmarks (Part 3 close-out)

**Status: written, not compiled, not run.** (No `cargo` in the 3c-5 sandbox.) The code paths touched are small and reuse the verified 3c-3 emulated kernels, but treat every claim below that is not marked "by construction" as unconfirmed.

### Backend switch
- `enum Backend { Cpu (default), CpuF32, Gpu }` (in `wind_stepped.rs`, re-exported). `Backend::param_value()` = the `backend` param of `wind` (0, 1, 2). `CpuF32` = the GPU kernels' f32 arithmetic on the CPU (no hardware). `Gpu` without feature `gpu` or without an adapter returns `Error::NoGpu`.
- `add_atmosphere_stage_stepped_with(&mut s, Backend)` = `pressure` + stepped `wind`; with `Backend::Cpu` it is identical to `add_atmosphere_stage_stepped` (test `backend_cpu_equals_plain_stepped_stage`, bit-equal wind).
- `add_climate_pipeline_with(&mut s, Backend)` = energy loop + stepped wind + moisture source + moisture/precipitation. **`add_climate_pipeline` is unchanged and keeps the diagnostic wind as the default.**
- `SteppedWind::with_backend(Backend)`; the module keeps the name `wind`.
- New output-only params (read with `Scheduler::module_param`, rejected by `set_param`): `last_solve_s` (wall time of the backend solve, includes pack/upload and, for the first GPU run, adapter and pipeline creation), `last_copy_back_s` (the final full-field device -> host copy of the wind; 0 for CPU backends). `StepResult` gained `copy_back_s`.
- The `GpuContext` is now cached inside `SteppedWind`, so a season loop requests the adapter once. WGSL pipelines are still rebuilt per solve (open issue).
- `testworld.rs`: `earth_like_world(level)` (the `tune.rs` world), `run_pipeline(domain, fields, backend, seasons)`, `summarize` -> `ClimateSummary {mean_precip, zonal (12 bands), tropics (|lat|<10), subtropics (20-35), windward, leeward, residual_rel}`.

### Data flow and the readback
Per season: CPU packs static data -> device solves (one 2-float read per pass of `steps_per_pass`, which also waits for the GPU) -> **one** 3n-float copy of the wind back to the CPU -> soft cap, `wind_*` outputs, `wind_upper` on the CPU in f64 -> moisture transport, projection and precipitation on the CPU in f64. The copy happens once per solve (once per season), never per step. Its cost is `last_copy_back_s`. The per-pass 2-float reads are not separable from GPU compute time (they block until the work finishes) and are inside `last_solve_s`.

### Precision and tolerance decisions
- Wind may be f32 (relative parity tolerance 1e-4 for kernels; end-to-end the criterion is the 5% on global mean precipitation). Conservation never depends on the wind precision: transport projects the wind on the CPU in f64, so the budget residual must stay < 1e-8 on every backend (tests assert this for `Cpu` and `CpuF32`, and `Gpu` under the feature).
- GPU projection (3c-4) is still NOT wired into `moisture_transport` (its 7.6e-6 divergence residual cannot meet the module's 1e-8 water closure).

### Tests (`tests/part3c5.rs`)
Always on: `backend_cpu_equals_plain_stepped_stage`, `backend_param_is_set_by_constructor`, `default_pipeline_still_uses_diagnostic_wind`, `cpu_pipeline_is_qualitatively_earth_like_and_conserves_water`, `f32_kernel_pipeline_matches_cpu_within_5_percent` (3 seasons, level 4), `f32_pipeline_is_deterministic`, `gpu_backend_without_feature_is_no_gpu` (default build only). Feature `gpu`: `gpu_pipeline_matches_cpu` (prints SKIPPED and passes without an adapter). Thresholds: tropics > 1.5x subtropics, windward > leeward, residual < 1e-8, global mean P within 5%. The tropics/subtropics factor is deliberately loose (3b-4 measured about 3x with the diagnostic wind; the stepped wind has not been measured in this pipeline); tighten after the first run.

### Benchmark table: PENDING (run on your machine)
```
cargo run --release --example bench_wind                       # CPU f64 + f32 emulation
cargo run --release --features gpu --example bench_wind        # adds GPU rows
cargo run --release [--features gpu] --example compare_backends
cargo test --release [--features gpu] --test part3c5 -- --nocapture
```
Wind time per season (seconds, "repeat" run, `solve_s`; sphere levels 5/6/7 = 10242 / 40962 / 163842 cells):
| level | cpu-f64 | cpu-f32-emul | gpu | gpu copy_back | gpu steps / passes | verdict |
|---|---|---|---|---|---|---|
| 5 | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| 6 | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| 7 | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |

Backend comparison, level 4, global mean precipitation (mm/day) and water residual: PENDING.

Acceptance rule for the table: GPU must beat CPU f64 at level 6 or 7; if not, record that and the bottleneck. Likely suspects, in order: (1) per-solve pipeline creation and buffer upload (not cached), (2) one blocking readback per 20-step pass (about 35 passes at the default tolerance, each a full GPU sync), (3) the final copy (3n f32; negligible next to 1 and 2 at level 7 on a discrete GPU), (4) host work that stays in f64 (gradient operator, `wind_upper`, soft cap, `pack_static`). `cpu-f32-emul` is a correctness twin, expected to be slower than `cpu-f64`; do not read it as a performance result.

### Open issues (carry into Part 4 and later)
1. All benchmark and GPU numbers are unmeasured; wgpu 23 code and WGSL (3c-2 to 3c-4) have still never been compiled.
2. Upper-level wind (`wind_upper`) is still the diagnostic balance, on the host.
3. Moisture transport, edge flux, projection and precipitation are CPU f64; the GPU CG projection needs a tighter tolerance path before it can feed conservation-critical transport.
4. Readback cost: raise `steps_per_pass` or overlap passes (double-buffered readback) if the per-pass sync dominates; cache the four pipelines and static buffers across seasons (persistent device state / warm start on device).
5. Stepped wind still has no momentum advection, so bands are still parametric (3c-1 caveat).
6. Seasonal inertia of vapour and heat (3b-4 issues 1 and 8) are unchanged.

Part 3 is functionally closed once the PENDING items above are confirmed on real hardware; Part 4 (ocean and hydrology) can start from the CPU path, which does not depend on them.

## Part 4a-1: ocean mask, shared sea level, wind stress (UNVERIFIED: written without a Rust toolchain)

Wiring: `add_ocean_mask_stage(&mut s)` registers `sea_level` + `ocean_mask` (needs only `elevation`, works with no atmosphere); `add_ocean_stage(&mut s)` adds `wind_stress` (needs `wind`, `ice_fraction`, so register it with the atmosphere stage and the ice-albedo loop). The scheduler orders `sea_level -> ocean_mask -> wind_stress`. Nothing feeds back yet.

| Module | Inputs | Outputs | Parameters (default) |
|---|---|---|---|
| `sea_level` | none | `sea_level` (m, per cell) | sea_level 0 |
| `ocean_mask` | `elevation`, `sea_level` | `ocean_mask` (0/1), `ocean_component` (id from 0, -1 off-ocean), `coast_distance` (m), `depth` (m, >= 0, 0 off-ocean) | min_ocean_area_m2 0 (auto = 1% of total area) |
| `wind_stress` | `wind`, `ocean_mask`, `ice_fraction` | `wind_stress` (3 layers, world-space tangent vector, N/m^2) | rho_air 1.2, cd_stress 1.3e-3 |

### Decisions (4a-1)
- **Shared sea level (resolves open decision 0 for the ocean stage):** one module `sea_level` writes the per-cell field `sea_level`. Per cell, not a scalar, so a later sea-level anomaly can edit it locally. Ocean reads it. **Not yet migrated:** `albedo`, `energy_balance`, `pressure`, `wind`, `moisture_source`, `precipitation` still carry their own `sea_level` parameter (default 0). Until they read the field, changing the shared level will not move their ocean/land decision. Migration (add `sea_level` input, drop the param) is a small separate task I left alone because those modules are compiled and tested and I could not recompile.
- **Below sea level** = `elevation < sea_level` (strict).
- **Ocean vs lake:** connected component (mesh adjacency) of below-sea-level cells with area >= `min_ocean_area_m2` is ocean; smaller is a lake (4b owns lakes; no lake field written here). Ids are ordered by lowest cell index, dense from 0.
- **Coast:** ocean cell touching a non-ocean cell, or a wall edge (`boundary_len > 0` on `Wall`/`Closed`). Void edges are not coasts. Wrap has no edge.
- **coast_distance:** multi-source Dijkstra through ocean cells only, weight `edge_dist`; 0 on coast and on non-ocean cells; a coastless ocean (all-ocean sphere, wrapped plane) gets the finite sentinel `sqrt(total_area)`. Land continentality is not provided here.
- **Stress:** `rho_air * cd * |u| u * (1 - ice_fraction)`, ice clamped 0..1, wind re-projected onto the tangent plane, exactly 0 on land. Surface layer of `wind` only.
- Full recompute (ignores `DirtyRegion`). Fields stay f64.

### Tests (`tests/part4a1.rs`, 12; 11 passed on first run, the lake test had a bad fixture and is fixed, unrerun)
Mask equals an independent BFS reference on sphere, cube, walled and wrapped plane; raising sea level adds ocean and depth is relative to it; small pit is a lake and becomes ocean with a lower threshold; coast distance 0 at coasts, positive inland, 1-Lipschitz and each interior cell has a closer neighbour; wall vs wrap coast; no-ocean world all zero; all-ocean sphere one component; determinism; stress parallel, magnitude, zero on land; ice scaling and zero under full ice; parameter validation; scheduler order.

### Verification status (4a-1)
No Rust toolchain, no network: **not compiled, tests not run**. Only a delimiter-balance check was done. Expect small compile errors; run `cargo test --test part4a1` and send output back. Existing suites untouched (only `lib.rs` gained `pub mod ocean;` and one `pub use`).

### Hand-off to 4a-2
Read `ocean_mask`, `depth`, `coast_distance`, `wind_stress`, `sea_level`. Ocean components are independent basins for gyres. Open: migrate other modules to the shared `sea_level` field.

## Part 4a-2a: ocean subgraph, Dirichlet operator, linear solver (no physics)

**API** (all in `ocean.rs`, re-exported from the crate root)
```rust
pub struct CgConfig { pub tol: f64 /*1e-10, relative*/, pub max_iters: usize /*20_000*/ }
pub struct OceanGraph {
    pub n: usize,                    // ocean cells
    pub cells: Vec<u32>,             // local -> global (ascending)
    pub local_of: Vec<i64>,          // global -> local, -1 off-ocean
    pub offsets: Vec<u32>, pub adj: Vec<u32>,   // CSR over local indices, ocean neighbours only
    pub coef: Vec<f64>,              // edge_len/edge_dist parallel to adj
    pub area: Vec<f64>,              // per local cell
    pub land_coef: Vec<f64>,         // sum of edge_len/edge_dist over NON-ocean neighbours (Dirichlet diagonal)
    pub coast_adjacent: Vec<bool>,   // >= 1 land neighbour (wall/void edges do NOT count)
    pub component: Vec<i64>, pub comp_ids: Vec<i64>, pub comp_cells: Vec<Vec<u32>>, // from ocean_component
}
OceanGraph::new(&Domain, ocean:&[bool], component:&[i64]) -> Result<Self>   // errors if adjacent ocean cells differ in component
OceanGraph::from_fields(&Domain, &FieldStore) -> Result<Self>                // reads ocean_mask, ocean_component
.n_components() .neighbors(i) .weight(i,q) // w_ik = coef/area_i
.apply_l(&[f64]) -> Vec<f64>                 // L psi
.apply_shifted(sigma,&[f64]) -> Vec<f64>     // (-L + sigma I) psi
.dot_area(x,y)                               // sum area_i x_i y_i
.to_global(&local) / .to_local(&global)      // global off-ocean = 0
.split_by_component(&b) / .merge_by_component(&parts)   // rhs helper; empty ocean -> [], one cell -> [[b0]]
.solve(sigma, &b_local, &CgConfig) -> Result<OceanSolution{ psi (local), components: Vec<ComponentSolve{id,n_cells,iterations,rel_residual,singular}>, total_iterations }>
```
`Error::NoConvergence { solver, iterations, residual }` is new (iteration guard hit, or non-positive curvature).

**Operator:** `(L psi)_i = sum_k w_ik (psi_k - psi_i)`, `w_ik = (edge_len/edge_dist)_k / area_i`, over all mesh neighbours; a land neighbour has `psi = 0`, so it adds `w_ik (0 - psi_i)`. The solver sees `(-L + sigma I) psi = b`, `sigma >= 0`.

### Decisions (4a-2a)
- **Symmetry is area-weighted (deviation from the PART text).** Because of the `/area_i`, `L = -A^-1 C` with `C` symmetric, so `<x, Ly> = <Lx, y>` holds for `<x,y>_A = sum area_i x_i y_i`, not the plain dot product, unless all areas are equal (true on the plane, and tested there in both forms). On sphere and cube (areas differ) the plain-dot form is NOT symmetric; tests check the area-weighted form to 1e-12.
- **Solver runs on the Euclidean-SPD equivalent** `(C + sigma A) psi = A b` (same solution), Jacobi preconditioner = its diagonal. Stopping rule: `||r|| <= tol ||A b||` in 2-norm of that system, start from `psi = 0` (deterministic, bit-identical on repeat). Own PCG, no reuse of the energy CG types.
- **All coasts share `psi = 0`; islands carry no separate circulation** (acknowledged limit, as specified).
- **Per-component solves:** one independent CG per `ocean_component` id; a component's result is bit-identical to solving that ocean alone, and changing the rhs in one ocean leaves the others unchanged (tested).
- **Walls/void/wrap edges carry no operator term** (zero flux, natural boundary), consistent with Part 1. A wall is not `coast_adjacent`. This differs from 4a-1's `coast_distance`, which counts a wall as a coast. **Open for 4a-2b:** physically a wall should also pin `psi` (no through-flow); if wanted, add a Dirichlet term `boundary_len / (0.5 sqrt(area))` and a flag.
- **Singular components handled instead of erroring:** a component with no `coast_adjacent` cell and `sigma == 0` (all-ocean sphere, wrapped plane) has a constant null space. The rhs gets its area-weighted mean removed and the solution is normalised to area-weighted mean zero; `ComponentSolve.singular = true`. With `sigma > 0` it is an ordinary SPD solve.
- Zero rhs returns zero with 0 iterations; empty ocean returns empty `OceanSolution`; invalid `sigma`/`tol`/lengths give `InvalidParam`, NaN rhs gives `NonFinite`.

### Measured (tests/part4a2a.rs; default tol 1e-10)
| problem | sigma | iterations | max error / max abs(psi_true) |
|---|---|---|---|
| plane 64x64, land border (62x62 ocean) | 0 / 1e-12 / 1e-10 | 203 / 203 / 173 | 8.5e-10 / 7.9e-10 / 9.4e-10 |
| sphere level 4, land cap (z > 0.6 R) | 0 / 1e-12 | 163 / 58 | 3.1e-10 / 1.6e-10 |
| cube 14/edge, one land face | 0 / 1e-12 | 127 / 69 | 1.2e-10 / 1.2e-10 |
Requirement was 1e-8 (about 10x margin). The manufactured solution is smooth + 20% white noise. Note `sigma = 1e-12` is tiny against the Laplacian's smallest eigenvalue scale only because `w` has units 1/m^2 (about 1e-8 per cell); choose `sigma` in those units when 4a-2b adds Rayleigh friction.

### Tests (11): manufactured plane/sphere/cube; symmetry (area-weighted all three, plain on plane) + negative semi-definite + `-L+sigma I` positive; Dirichlet term on a one-cell ocean; two disjoint oceans (joint == alone, bit-identical, rhs-independence); empty and one-cell ocean; `coast_adjacent` against an independent check on three topologies; coastless singular case; iteration count, determinism, `NoConvergence` at `max_iters = 3`, bad-input errors; `from_fields` through the real `ocean_mask` stage and the split/merge/global/local round trips.

### Hand-off to 4a-2b
Build `OceanGraph::from_fields` after `ocean_mask`; right-hand side = curl of `wind_stress` (local vector, per cell); call `solve(sigma, &b, &CgConfig::default())`; `to_global` for a `psi` field. Decide the wall question above and the units of `sigma` (Rayleigh/Stommel friction). Nothing is wired into the scheduler yet.

## Part 4a-2b: wind-stress curl and beta term (no solver, no physics beyond two diagnostics)

**API** (`src/ocean_forcing.rs`, re-exported from the crate root)
```rust
pub struct CurlOp { /* coef: Vec<Vec3> parallel to mesh.adj */ }
CurlOp::new(&Domain) -> CurlOp
.coef() -> &[Vec3]
.apply(&Domain, &[Vec3]) -> Vec<f64>                          // curl_i = sum_k coef_k . (v_j - v_i), all neighbours
.apply_masked(&Domain, &[Vec3], ocean:&[bool]) -> Vec<f64>    // non-ocean neighbours contribute nothing; non-ocean cells = 0
curl_of(&Domain, &[Vec3]) -> Vec<f64>
curl_of_masked(&Domain, &[Vec3], &[bool]) -> Vec<f64>
beta_of(&Domain, coriolis_f:&[f64]) -> Vec<f64>                // grad(f) . north via GradientOp
unpack_vec3(&[f64]) -> Vec<Vec3>                               // 3-layer field -> vectors
WindStressCurl  (module "wind_stress_curl": wind_stress, ocean_mask -> stress_curl, N/m^3, 0 off-ocean)
BetaTerm        (module "beta_term": coriolis_f -> beta, 1/(m s), every cell)
add_gyre_forcing_stage(&mut Scheduler)                         // registers both; no parameters
```
Fields: `stress_curl` (1 layer, ocean cells, 0 on land and lakes), `beta` (1 layer). No new parameters; `set_param` returns `InvalidParam`.

### Decisions (4a-2b)
- **Curl = discrete Stokes circulation** `curl_i = (1/area_i) sum_k tau_edge . t_ik len_ik`, `t_ik = up_i x n_ik`, `n_ik` = unit direction from centroid i to j projected into i's tangent plane (negated for wrapped edges, same test as `GradientOp`). No east/north component of the stress is differentiated.
- **Evaluated relative to the cell's own value:** `tau_edge = tau_i + (h/d)(tau_j - tau_i)`; the `tau_i` term is the circulation of a constant over a closed loop (0) and is dropped. A uniform field therefore gives exactly 0 on every cell and topology (tested bitwise to 1e-30), independent of cell shape.
- **Land / lake / wall rule (as specified):** the cell's own value is used across such an edge, which means *no term*. Wall/void edges are not in the adjacency, so they also carry no term. Values stored on non-ocean cells are never read (tested with 1e9 garbage).
- **Deviation 1, edge geometry from area closure.** The mesh has no centroid-to-edge distance `h`. Measured with the naive `h = d/2` (weights `len t /(2 area)`), solid-body curl was 12% off on the hex-pent sphere at all levels (cells are not Voronoi). Now `h_ik = (s_i/2) d_ik`, `s_i = 4 area_i / sum(len d)`; weights `2 len t / sum(len d)`. Solid-body error fell to 0.9% (L3), 0.4% (L4), 0.2% (L5).
- **Deviation 2, first-order consistency correction.** Even then `curl(grad phi)` did not shrink (14%, 18%, 20% at L3-5, measured in a numpy mirror) because the stencil is not exact for linear fields on distorted cells. A minimal weighted-norm correction (the same 2x2 least squares as `GradientOp`, weight `1/d^2`, eps `1e-10`) makes the stencil exact for every linear tangent field. Regular cells (square grid, regular hexagon) get a correction of exactly 0. Mirror result: `max|curl grad phi| / (|grad phi|/R)` = 1.7e-2, 9.0e-3, 4.6e-3 at L3, L4, L5 (halves per level). Boundary cells with missing wall edges are also exact for linear fields (3-neighbour and corner cells, tested to 1e-6 on the plane).
- **Deviation 3, PART text typo.** Rotation about the polar axis has radial vorticity `2 omega sin(lat)` = `2 omega (axis . up)`, not `2 omega cos(lat)` (that is the colatitude form). Tests use `axis . up`.
- **Masked curl near land is first-order inconsistent by design** (the zero-gradient rule discards the land edge and the weights are not re-corrected). That is the recorded boundary behaviour, not a bug.
- **Beta** = `grad(f) . north_i`, `GradientOp` on the scalar `coriolis_f`. Does not need the spin axis or latitude; works with `coriolis_const` (f-plane gives exactly 0) and with a linear-in-position `f` on a plane (beta recovered to 2e-6 relative: `GradientOp`'s `eps = 1e-6 (sxx+syy)` regularisation shrinks every gradient by about 2e-6; test tolerance 1e-5). At a pole cell `north` is an arbitrary tangent (Part 1 rule), so beta there is meaningless but finite.
- Orientation independence is tested by rotating the whole icosahedral mesh (centroids and up) by 0.7 rad about (1,2,3) while keeping the spin axis fixed; frames are rebuilt by `Domain::new`.

### Numpy mirror results (a stand-in for running the tests; `/reference` style, same geometry as `generate_sphere`)
| quantity | L3 | L4 | L5 |
|---|---|---|---|
| solid-body curl, max err / (2 omega) | 8.7e-3 | 4.3e-3 | 2.1e-3 |
| curl of grad phi, max / (grad scale) | 1.7e-2 | 9.0e-3 | 4.6e-3 |
| beta, max rel err (abs lat < 60) | 2.2e-2 | 1.2e-2 | 6.0e-3 |
Rotated mesh: solid-body 9.0e-3 / 4.5e-3 / 2.2e-3; curl grad 1.7e-2 / 9.1e-3 / 4.7e-3; beta 3.2e-2 / 1.5e-2 / 7.4e-3. Plane (40x30 wall), all cells including walls and corners: 2.5e-11.

### Tests (`tests/part4a2b.rs`, 10): plane solid-body (wall and void, interior and wall cells, 1e-6); sphere solid-body `2 omega sin(lat)` (L3, L4, shrinking, < 2%); curl of gradient L3-L5 shrinking (< 0.7x per level, < 1% at L5); uniform field exactly 0 on sphere, cube, walled and wrapped plane, plus finiteness; masked rule (garbage on land, hand-summed stencil next to land); beta on the sphere (< 5%, positive); beta orientation independence (per 10-degree band means within 2%); f-plane = 0 and beta-plane within 1e-5; modules write fields, land curl is exactly 0, results equal the pure functions; module input errors.

### Verification status (4a-2b)
**UPDATE (4a-2d session): compiled and all 10 tests pass as written.** Original note: **Not compiled, tests not run** (no Rust toolchain and no network in this session; cargo is absent). Only a delimiter-balance check plus the numpy mirrors above. The test tolerances were chosen from the mirror with at least 2x margin. Please run `cargo test --test part4a2b` and then `cargo test`; expect small compile slips.

### Hand-off to 4a-2c (done, see "Part 4a-2c")
RHS of the gyre equation = `stress_curl / (rho_ocean * H)` (units 1/s^2 after dividing by density and depth; `depth` field exists). The Stommel/Munk equation `beta d(psi)/dx = curl(tau)/(rho H) - r lap(psi)` needs a *directional* beta term (`beta` is a scalar along local north; the advective term is `beta * (north . grad psi)` rotated by 90 degrees), which is not symmetric, so CG will not apply directly to it; decide between a Rayleigh-friction-only streamfunction (`-L + sigma I` with `b = curl/(rho H)`, solvable now with 4a-2a) and a non-symmetric solver (BiCGSTAB or GMRES). Still open: the wall Dirichlet question from 4a-2a.

## Part 4a-2c: symmetric gyres and the current field (beta off)

**Wiring:** `add_ocean_stage` (or `add_ocean_mask_stage`) + `add_gyre_forcing_stage` + `add_ocean_currents_stage`. Order: `sea_level -> ocean_mask -> wind_stress_curl -> ocean_currents`. Nothing reads `psi`/`current` yet and nothing feeds back (SST coupling is a later part).

| Module | Inputs | Outputs | Parameters (default) |
|---|---|---|---|
| `ocean_currents` | `stress_curl`, `ocean_mask`, `ocean_component` | `psi` (1 layer, m^2/s, 0 off-ocean), `current` (3 layers, world-space tangent vector, m/s, 0 off-ocean), `current_speed` (m/s) | `H_mixed_m` 100, `r_friction` 5e-8 (1/s), `rho_water` 1025, `max_current` 3, `beta_on` 0 (any other value is `InvalidParam` until 4a-2d), `coast_slip` 1 (0 or 1) |

Output-only parameters (written by `run`, `set_param` rejects them, read with `Scheduler::module_param`): `clamped_cells`, `max_speed_raw` (m/s before the clamp, after the slip), `cg_iterations`, `slipped_cells`. `ocean_component` is a third input (the PART text lists two): `OceanGraph::from_fields` needs it.

**Equation (as implemented, standard convention `u = k x grad(psi)`, `zeta = lap(psi)`):**
`r * lap(psi) = + curl(tau) / (rho_w * H)`, `psi = 0` on every non-ocean cell. Solved as `(-L) psi = -curl(tau) / (rho_w H r)` by `OceanGraph::solve(0.0, ...)` (4a-2a CG). `rhs = -stress_curl / (rho_w * H_mixed_m * r_friction)` is computed with that operand order, so doubling `r_friction` or `H_mixed_m` scales the rhs by an exact power of two and `psi` halves bit-for-bit.
**Velocity:** `current_i = up_i x (grad psi)_i`, `GradientOp` on the global scalar `psi` (land neighbours carry `psi = 0`), gradient re-projected onto the tangent plane first. Then, in this order: coast slip, clamp.
**Coast slip (new, on by default):** for a coast cell with land edges `k`, `S_i = sum_k len_ik n_ik`, `L_i = sum_k len_ik`; if `|S_i| > 1e-6 L_i` the component of `current_i` along `S_i/|S_i|` is removed, so `F_i = sum_k len_ik (u_i . n_ik) = u_i . S_i = 0` (net flux through that cell's land edges, any topology, wrapped edges handled as in `GradientOp`). `coast_slip = 0` returns the raw stream-function current.
**Clamp rule:** after slip, any `|current| > max_current` is scaled to exactly `max_current` (direction kept); the number of such cells is `clamped_cells`. The clamp changes only `current`, never `psi`.

### Decisions and deviations (4a-2c)
- **Deviation 1, sign of the equation.** The PART text writes `r lap(psi) = -curl(tau)/(rho H)` together with `current = k x grad(psi)`. With that pair a positive wind-stress curl drives anticyclonic flow (opposite to the physics: vorticity balance `r zeta = curl(tau)/(rho H)` with `zeta = lap(psi)` for `u = k x grad psi`). Implemented with the physical sign. Check in the mirror: northern subtropics (10..40 N, trades below, westerlies above) have negative curl, negative vorticity (clockwise), a positive-`psi` hill; the southern hemisphere is the exact mirror image. The test asserts the vorticity of the current equals `curl/(rho H r)` within 10% (mirror 0.985 at L4). 4a-2d will add `beta * d(psi)/dx` to the same convention (`r lap(psi) + beta d(psi)/dx = curl(tau)/(rho H)`).
- **Deviation 2, "psi = 0 on every coast-adjacent cell" is read as `psi = 0` on every non-ocean cell** (the Dirichlet value lives on the land neighbour, as fixed in 4a-2a). On the coast-adjacent OCEAN cells `psi` is the first interior value; it is small compared with the deep ocean (mean ratio 0.41, 0.25, 0.14 at L3, L4, L5 in the mirror, halving per level) and is not zero.
- **Deviation 3, no-normal-flow needed an explicit rule.** The raw `k x grad(psi)` current at a cell-centred coast cell is NOT parallel to the coast: the net flux through land edges was 0.63, 0.58, 0.45 of `L * max|u|` at L3-L5 (mirror), shrinking only slowly. Per-edge flux cannot be zero on a zig-zag coast (an oblique edge is crossed even by coast-parallel flow), so the criterion is the net flux per coast cell, which the slip rule makes zero to roundoff (1e-16). Walls, voids and wrap edges are not touched (see Open).
- **Deviation 4, divergence.** The PART asks for `div(k x grad psi) < 1e-6` relative. A cell-centred gradient on a hex-pent mesh is non-divergent only in the continuum. Measured with the `GradientOp`-consistent divergence `sum coef.(u_j - u_i)` on cells two hops from any coast: **plane 1.3e-16** relative to `max|u|/dx` (central differences commute: exact to roundoff, test < 1e-9); **sphere** max 0.013, 0.026, 0.027 and rms 0.017, 0.012, 0.011 of the vorticity scale at L3-L5 (does NOT shrink in the max, pentagon neighbourhoods dominate; tests: max < 0.08, rms < 0.04 at L4).
- **Default friction is far too small for ocean-like speeds.** With the PART defaults (`r_friction` 5e-8) a Rayleigh-friction-only gyre has `zeta = curl/(rho H r)`: speeds of 19 m/s at L4 (max), **1398 of 2316 ocean cells above 10 m/s and 2202 above 3 m/s**, so the 3 m/s clamp is active almost everywhere (`clamped_cells` reports it). `r_friction = 4e-6` gives 0.24 m/s with nothing clamped (mirror). Defaults left as specified; **please decide** whether to change the default now or when beta arrives (Stommel friction is typically 1e-6..1e-5 1/s).
- Singular components (no coast anchor: all-ocean sphere) use the 4a-2a mean-zero rule; `slipped_cells = 0` there. An empty ocean gives all-zero fields.
- Full recompute (ignores `DirtyRegion`), f64 fields, deterministic (CG starts from zero).

### Numpy mirror results (`reference/mirror_gyres.py`, `mirror_gyres2.py`; same sphere geometry, `CurlOp`, `GradientOp`, `OceanGraph` as the Rust code, scipy CG to 1e-12)
World: sphere, two meridional land strips (|y| < 0.13 R, |lat| < 70 deg), one ocean joined over the poles, stress `-0.1 cos(3 lat)` eastward.
| quantity | L3 | L4 | L5 |
|---|---|---|---|
| ocean cells | 592 | 2316 | 9228 |
| slipped coast cells (all coast cells) | 84 | 172 | 328 |
| raw net coast flux / (L max u) | 0.63 | 0.58 | 0.45 |
| after slip | 1e-16 | 1e-16 | 1e-16 |
| band vorticity / (curl/(rho H r)), 10..40 N and S | 0.955 | 0.985 | 0.994 |
| N/S vorticity ratio | -1.000 | -1.000 | -1.000 |
| ring/deep mean abs psi | 0.41 | 0.25 | 0.14 |
| LS div / vorticity, max (rms) | 0.013 (0.017) | 0.026 (0.012) | 0.027 (0.011) |
Plane 40x30, land border, `tau_x = -0.1 cos(2 pi y/Ly)`: LS divergence 1.3e-16, raw coast flux up to 3.6 m/s per unit edge length (absolute) and 8e-17 after slip; uniform wind: curl exactly 0.

### Tests (`tests/part4a2c.rs`, 12)
psi zero on land + ring psi small + net coast flux zero with slip + nonzero without (so the rule does work); subtropical gyres opposite senses, vorticity = curl/(rho H r) within 10%, psi hill signs; opposite forcing reverses psi and current (1e-12), doubling `r_friction` and `H_mixed_m` halves psi and current; flat plane + uniform wind through the real scheduler gives exactly zero psi/current and 0 CG iterations; plane divergence at roundoff; sphere divergence small; clamp count equals an independent count at 3 and 10 m/s, direction kept, raw max > 10 at default friction, `r_friction = 4e-6` gives 0.1..0.6 m/s with 0 clamped; finite, tangent, zero on land, bit-deterministic; cube smoke (one land face: psi zero on land, zero net coast flux, CG ran); all-ocean sphere (singular, mean-zero psi); no-ocean world; wiring (spec, defaults, errors, `beta_on = 1` rejected, output-only params read-only, scheduler order, full run, input errors).

### Verification status (4a-2c)
**UPDATE (4a-2d session): compiled and all 12 tests pass as written (after the 4a-2d test edits above).** Original note: **Not compiled, tests not run** (no Rust toolchain, no network, cargo absent). Done: delimiter balance on all new files and the numpy mirror above (every threshold in `tests/part4a2c.rs` was taken from it with >= 1.5x margin; the cube, all-ocean, no-ocean, wiring and error tests make no numeric claims, so they are unverified in a different way: logic only). Please run `cargo test --test part4a2c` then `cargo test` and send back compile errors. Existing files touched: `lib.rs` only (`pub mod`, `pub use`, doc line).

### Open (carry into 4a-2d)
- **(Done in 4a-2d)** **Beta term:** `beta_on != 0` was rejected. 4a-2d needed the non-symmetric operator `r lap(psi) + beta d(psi)/dx` (directional `beta` via `north`/`east`), so BiCGSTAB or GMRES, or a symmetric split. The slip/clamp/velocity rules above carry over unchanged.
- **Walls:** a `Wall`/`Closed` domain edge still imposes no `psi` condition and no slip (a walled plane without land lets the current pass the wall); wall normals are not stored in `Mesh`. The Dirichlet-wall option from 4a-2a is still undecided.
- **(Default `r_friction` settled in 4a-2d: 1e-5)** Still open: whether `H_mixed_m` should come from the `depth` field.
- Islands carry no separate circulation (all coasts share `psi = 0`), as specified.

## Part 4a-2d: beta term, western intensification, BiCGSTAB

**Wiring:** unchanged: `add_ocean_stage` + `add_gyre_forcing_stage` + `add_ocean_currents_stage`. Order `sea_level -> ocean_mask -> wind_stress_curl, beta_term -> ocean_currents`. `ocean_currents` now ALSO needs `beta` (always declared, even with `beta_on = 0`, which just does not read it; `add_gyre_forcing_stage` provides it from `coriolis_f`). Nothing feeds back yet.

| Module | Inputs | Outputs | Parameters (default) |
|---|---|---|---|
| `ocean_currents` | `stress_curl`, `beta`, `ocean_mask`, `ocean_component` | `psi`, `current` (3 layers), `current_speed`, `boundary_layer_m` (m, `r/|beta|`, 0 off-ocean and where beta = 0) | `H_mixed_m` 100, **`r_friction` 1e-5** (was 5e-8), `rho_water` 1025, `max_current` 3, **`beta_on` 1** (0 or 1; was 0 only), `coast_slip` 1 |

Output-only (read with `Scheduler::module_param`, `set_param` rejects): `clamped_cells`, `max_speed_raw`, `cg_iterations` (iterations of whichever solver ran; BiCGSTAB counts full iterations = 2 matrix products), `slipped_cells`, new: `solver_kind` (0 CG, 1 BiCGSTAB), `solver_restarts`, `layer_cells_min`, `layer_cells_median`, `under_resolved_fraction`, `under_resolved` (0/1). `OceanCurrents::warning()` returns the warning text of the last run.

### Decisions (4a-2d)
- **Option A chosen** (upwinded advection + BiCGSTAB, in `ocean.rs`, no dependency). B and C not implemented.
- **Equation:** `r lap(psi) + beta d(psi)/d(east) = curl(tau)/(rho H)` with `u = k x grad(psi)` (same convention as 4a-2c, the sign of the beta term follows from `beta v = curl/(rho H) - r zeta`, `v = d psi/d east`). Solved as `(-L + N) psi = -curl/(rho H r)`, `N psi = -(beta/r) d psi/d east`: advection with velocity `c = -(beta/r) east` (1/m), i.e. information travels WEST, so the boundary layer is at the west coast (verified: negating beta moves it to the east coast).
- **Upwind scheme (deviation from the PART text, deliberate):** per edge `v_ik = (c_e . n_ik) len_ik`, `c_e = (c_i + c_j)/2`, `n_ik` = unit chord i->j projected on the mean tangent plane (wrap handled as in `GradientOp`). `v_ik = -v_ki` exactly. Upwind by sign of `v`: outflow edges contribute nothing, inflow edges (`v < 0`) contribute `v (psi_k - psi_i)/area_i`. This is the ADVECTIVE (difference) form, not the flux form `v psi_up`: the flux form carries a spurious `psi div(c)` source because the discrete `div(beta east)` is not zero on the geodesic mesh, whereas the advective form annihilates constants exactly. Land neighbour: upstream value is the Dirichlet `psi = 0` (diagonal term `land_inflow`); walls/voids carry no term. Matrix is an M-matrix (positive diagonal, non-positive off-diagonals), so the scheme is monotone and stable for any `r`. First-order upwind adds numerical diffusion `~ dx/(2 delta)` relative to the physical one (`delta = r/beta`); the layer is smeared to about one cell when `delta < dx`.
- **Solver:** Jacobi-preconditioned BiCGSTAB (van der Vorst) on the area-multiplied system `(C + A N + sigma A) psi = A b`, one per ocean component, tol 1e-10 (relative true residual), `max_iters` 20000 (`CgConfig`). Breakdown detection (`rho`, `rhat.v`, `t.t`, `omega`) and recurrence-vs-true-residual drift trigger a restart (fresh shadow residual); more than 20 restarts, a non-finite residual or `max_iters` returns `Error::NoConvergence { solver: "ocean_bicgstab (...)" }`. The restart/breakdown paths were never triggered in any test (restarts = 0 everywhere), so they are exercised only by reading, not by a test. With `beta == 0` everywhere (f-plane) the module uses the plain CG path, so the result is bit-identical to `beta_on = 0`.
- **Singular component** (no coast anchor, e.g. all-ocean sphere): rhs mean removed, first cell pinned to 0, result normalised to area-weighted mean zero. With advection the left null vector is not the area measure, so the pinned system is only approximately consistent; the dropped equation absorbs the residual. Tested finite and deterministic only; not physically meaningful (no coast, no western boundary).
- **Default `r_friction` changed 5e-8 -> 1e-5.** Not needed for solver stability (the M-matrix is stable for any r; BiCGSTAB converges at 5e-8 too). Reasons: (1) 5e-8 gives non-physical 19 m/s friction-only speeds (see 4a-2c); (2) the layer `r/beta` must be about one cell or more: at `r = 4e-6` the median layer is 0.44 cells at L4 (warns), at 1e-5 it is 1.11 cells at L4 and 2.2 at L5 (0.56 at L3, warns). Peak speeds at 1e-5 with the clamp off: 0.085 / 0.117 / 0.174 / 0.229 m/s at L3 / L4 / L5 / L6, nothing near the 3 m/s clamp. Mesh-resolution dependence of the peak is real (the layer carries a fixed transport through a thinner layer as the mesh refines); that is expected, not a bug.
- **Boundary-layer report:** `delta_i = r/|beta_i|` against `sqrt(area_i)`; `under_resolved` = median ratio < 1 (most of the ocean thinner than one cell), fraction and min reported too; polar cells with beta ~ 0 have a huge `delta` and count as resolved. The warning is returned by `OceanCurrents::warning()` and flagged in `under_resolved`; no logging facility exists in the scheduler.
- **4a-2c tests** (`tests/part4a2c.rs`): `run_with` now pins `beta_on = 0` and `r_friction = 5e-8` (the 4a-2c configuration) before applying a test's own overrides; the flat-plane scheduler test registers `add_gyre_forcing_stage` with a constant `coriolis_f`; the wiring test asserts the new inputs/outputs/defaults and that `beta_on` accepts 0 and 1 and rejects other values. No other change.

### Measured (tests/part4a2d.rs, default parameters; `max_current` raised to 1e9 only where a test measures peaks)
Gyre world = two meridional land strips, stress `-0.1 cos(3 lat)`; peak `current_speed` within 15 degrees of the west / east coast, lat 10..40, per hemisphere and basin (all four gyres give the same numbers on this mirror-symmetric world):
| level | west peak (m/s) | east peak (m/s) | ratio | BiCGSTAB iterations (CG beta off) | median layer (cells) |
|---|---|---|---|---|---|
| 3 | 0.068 | 0.020 | 3.4 | 28 | 0.56 |
| 4 | 0.117 | 0.022 | 5.4 | 56 | 1.11 |
| 5 | 0.174 | 0.023 | 7.4 | 109 | 2.22 |
Beta off: west/east = 1.000000 at every level. Beta negated: east 0.117 vs west 0.022 at L4 (intensification on the east coast). Cube 14/edge (one land face): 94 iterations, finite. Mean `psi` in 10..40 N / S: +7.97e4 / -7.97e4 m^2/s (L4); correlation of `psi` (beta on) with `psi` (beta off) 0.91. Interior meridional flow is equatorward in both subtropical bands. Plane 40x30 f-plane: x-asymmetry of psi 2e-16, CG path; plane beta-plane (r 7e-7, layer 3 cells): west peak 1.65, east 0.65 (2.6x). 1-D strip: BiCGSTAB equals the exact discrete (Thomas) solution to 4e-11 relative; manufactured non-symmetric problems (plane at cell Peclet ~5, sphere L3/L4) recovered to 2e-10 / 2e-11 / 2e-10 absolute (psi scale ~1). Zero advection: BiCGSTAB vs CG 1.8e-10 relative. Divergence of the current: beta-plane 1.5e-16 (roundoff, relative to `umax/dx`), sphere L4 max `|div|/max|zeta|` on deep cells 0.025 (same discretisation limit as 4a-2c).

### Tests (`tests/part4a2d.rs`, 16)
Solver (the 4a-2d-i content): agreement with CG at zero advection (with and without shift); manufactured non-symmetric solutions on plane and sphere; edge-flux antisymmetry, exactness for linear fields on the plane, constants annihilated; 1-D Stommel layer against Thomas, west position, mirror under reversed advection; errors (`sigma`, `tol`, lengths, NaN, foreign advection) and `NoConvergence` at `max_iters = 3` (reports exactly 3 iterations), zero rhs, determinism. Physics (the 4a-2d-ii content): beta off equals an independent 4a-2c pipeline bit-for-bit (garbage `beta` ignored); `beta = 0` with `beta_on = 1` bit-identical to off; west > 2x east for all four gyres at L3-L5 (and exactly 1 with beta off); reversed beta flips it; gyre senses and N/S antisymmetry; convergence on sphere L3-L5 and cube 14/edge; f-plane symmetric vs beta-plane asymmetric; no NaN and divergence; boundary-layer report (r = width x beta exactly, equator 437 km, width in cells doubles per level), warning on/off; scheduler wiring, defaults, read-only outputs, missing and NaN `beta`; all-ocean sphere.

### Open (carry forward)
- **Walls** still impose no `psi` condition and no slip (unchanged from 4a-2c; a walled plane with no land lets the current pass the wall).
- **First-order upwind** smears the layer to about one cell; a second-order (limited) upwind or a finer mesh near western coasts would sharpen it. Only worth doing if the layer width matters to a later module (upwelling at western coasts).
- **`H_mixed_m` from the `depth` field** and **islands** (separate circulation) still open. Beta-driven gyres are not yet coupled to SST (a later part).
- Tropical equatorial dynamics (beta -> 2 Omega/R near the equator, where geostrophy fails) are not represented; the model is the Stommel gyre, not an equatorial-wave model.
- Full recompute only (ignores `DirtyRegion`).
- Breakdown/restart paths in the BiCGSTAB are not exercised by any test (no case triggered them).

### Verification status (4a-2d)
Compiled and run in the sandbox (cargo 1.97, offline, `gpu` feature stripped from a scratch copy of `Cargo.toml`; the delivered `Cargo.toml` is the original). `cargo test` passes every suite including `part4a2b` (10), `part4a2c` (12), `part4a2d` (16). No numpy mirror was written for 4a-2d: thresholds come from the Rust probe (`examples/gyre_probe.rs`) with >= 1.3x margin (the tightest: beta-plane west/east 2.6x against a 2x threshold; sphere L3 3.4x against 2x). The 3c-5 benchmark/GPU numbers remain PENDING as before.

## Parameters from Part 1

Generators: radius, subdivision, half_side, cells_per_edge, cell_size, shape, boundary, surface_gravity, seed. Test modules: `diffusivity`, `steps`, `target`, `gain`, `value`. No physical field names are reserved yet.

## Decisions made
- (Part 2) `tectonics` writes `tectonic_elevation`; `terrain` is the only writer of `elevation` (the scheduler forbids two producers).
- (Part 2) Energy balance is equilibrium per season: no thermal inertia, so seasonal swings are exaggerated (e.g. high-latitude swing of tens of K).
- (Part 2) Albedo defaults give Earth-like results on a 70% ocean test world: global mean ~286 K, equator ~299 K, poles ~259 K, ~14% ice. An all-land world with the same defaults falls into the snowball state (bistability is real, not a bug).
- (Part 2) Loop tolerance 2e-3 chosen so every tested season converges in <= 17 passes (tolerance 1e-4 needs ~25).
- (Part 2) `max_slope` is per mesh edge, so it is resolution dependent.
- (Part 3a) Pressure and wind are separate modules; `coriolis_f` is a field written by `pressure`, so rotation is a single parameter there and `wind` just reads it.
- (Part 3a) Wind is a steady balance (drag + Coriolis + pressure gradient), not mass-conserving: no divergence constraint yet. Vertical motion for orographic lift is therefore left to the moisture module (3b), which should use `wind`, `elevation` gradient and `wind_upper`.
- (Part 3a) Surface wind is the only low-level wind; `wind_upper` is a return-flow level, not a full vertical structure (`Domain.layers` still unused).
- f64 fields (conservation tests); revisit f32 at the GPU stage.
- `Vec3` is a plain array; swap for `glam` when SIMD/GPU interop matters.
- Cube `up` is radial, not the face normal (matches "gravity toward centre").
- Wall and Void behave identically (zero flux) in Part 1; Void semantics (loss/waterfall) belong to the fluid and hydrology parts.
- Module named `generators` (not `gen`, a reserved word in edition 2024).

## Open decisions for Part 3+
0c. (3a) Band term is parametric. Land-sea monsoons only come from `temperature_sl` contrasts (no land heat capacity yet). Cube worlds use `elevation` as height; the geopotential difference between face centres and edges is not yet added to hydrostatic pressure.
0d. (3a) Where thresholds for `blocking` should live once real vertical structure exists; whether wind should become a 2-layer field instead of two named fields.
0. Thermal inertia / seasonal lag (blend of seasonal and annual insolation, or a heat-capacity state). Ocean vs land heat transport differences. Sea level as a global parameter instead of `sea_level` per module (4a-1: shared `sea_level` field exists; other modules not yet migrated).
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
- Part 2 compiled and its 15 tests pass (confirmed in sandbox this session).
- Part 3a compiled and its 10 tests pass. Static check: balanced delimiters in `gradient.rs`, `pressure.rs`, `wind.rs`, `tests/part3.rs`. Expect to fix small type/borrow errors on first `cargo test`; please send the compiler output back.
- Part 3a numerics validated in NumPy: `reference/wind_mirror.py` + `reference/run_wind.py` mirror `gradient.rs`, `pressure.rs`, `wind.rs` formula for formula; test thresholds in `tests/part3.rs` were set from that mirror with wide margins (band counts 0/3/9, zonal winds, tidal flow, barrier slowdown ~0.77x). Not validated: Rust compilation, exact Rust numbers on the cube and wrapped plane (smoke test only checks finiteness, `<= max_wind`, determinism).
- Parts 1-3b and 3c-1 all compiled and pass in the sandbox (cargo 1.97, offline; 82 tests total, no warnings). Water conservation including rain is tested in 3b-3 and 3b-4.
- Part 4a-1 and 3c-5 (later compiled in the 4a-2a session): `cargo test` passes everything, zero warnings; the 3c-5 benchmark/GPU numbers are still PENDING. Original note: Part 3c-5 was written without a Rust toolchain; **not compiled, tests not run, benchmarks not run**. Static check only (balanced delimiters). Please run `cargo test` (expect 98 existing + 6 new tests, zero warnings) and the examples, then fill in the PENDING tables.
