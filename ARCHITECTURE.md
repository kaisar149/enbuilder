# ARCHITECTURE.md (after PART 1: Foundations)

Crate: `worldgen-core` (Rust 2021, std only, no dependencies). Status: **written but not yet compiled** (see "Verification status").

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
| `error.rs` | `Error`, `Result` |
| `tests/acceptance.rs` | the four acceptance tests plus extras |

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
    pub lights: Vec<Light>,               // stub, empty until Part 3
    pub media: Media,                     // default Media::earth_like()
    pub boundary: BoundaryKind,           // Closed | Wrap | Wall | Void | OpenSky | SeaFloor
    pub layers: VerticalStructure,        // {atmosphere_layers: 4, ocean_layers: 3} defaults, unused yet
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

## Parameters present so far
Generators: radius, subdivision, half_side, cells_per_edge, cell_size, shape, boundary, surface_gravity, seed. Test modules: `diffusivity`, `steps`, `target`, `gain`, `value`. No physical field names are reserved yet.

## Decisions made
- f64 fields (conservation tests); revisit f32 at the GPU stage.
- `Vec3` is a plain array; swap for `glam` when SIMD/GPU interop matters.
- Cube `up` is radial, not the face normal (matches "gravity toward centre").
- Wall and Void behave identically (zero flux) in Part 1; Void semantics (loss/waterfall) belong to the fluid and hydrology parts.
- Module named `generators` (not `gen`, a reserved word in edition 2024).

## Open decisions for Part 2+
1. Do modules need an explicit layer-aware Domain query API, or is `layers` + `FieldStore` enough?
2. Per-field influence radius for `DirtyRegion` expansion: where is it declared (ModuleSpec vs Domain)?
3. Provenance recording hook (Part 9): per-write attribution needs a module-id parameter on `FieldStore` writes.
4. Oblate sphere and cube-sphere meshes (spec lists both); only the geodesic sphere is built.
5. Sky-island/3D domains need a mesh type with volumes and per-island gravity; current `Mesh` is surface-only.
6. Whether `boundary_len` should also carry boundary kind per edge (needed once wrap and wall mix on one mesh).

## Verification status
- Compiled: **no** (the build sandbox had no Rust toolchain and no network).
- Validated: `reference/mirror_validation.py` is a line-for-line NumPy mirror of the sphere dual, cube folding, plane/disc grids and flux-form diffusion. Results: sphere/cube areas exact, adjacency symmetric on all shapes, diffusion conserved to 1e-16 and uniform to 1e-14, disc area error 0.59% at 20 cells and 0.077% at 100 cells, loop iteration counts well under the caps used in tests.
- Not validated: borrow-checker/type errors in the Rust itself. Run `cargo test` first; expect at most small compile fixes.
