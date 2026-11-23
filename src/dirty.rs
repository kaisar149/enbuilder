//! Part 6, step 1: dirty-region tracking.
//!
//! An edit marks seed cells. Each FIELD has an influence radius (metres of surface distance along mesh edges); the region of a field is
//! every cell within that radius of a seed. Fields that are carried by the wind additionally get a DOWNWIND extension: starting from the
//! region, the front keeps going along edges that point (within a cone) the way the local wind blows, for `downwind_m` more metres.
//! Everything here uses only the mesh adjacency, edge distances and centroids, so it works on sphere, cube, plane and any custom mesh.
use crate::domain::Domain;
use crate::fields::FieldStore;
use crate::math::{dot, normalize, sub};
use crate::module::ModuleSpec;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

/// How far each field's change is allowed to reach (metres), plus optional downwind carry.
#[derive(Debug, Clone)]
pub struct InfluenceSpec {
    /// Radius for any field without its own entry.
    pub default_m: f64,
    pub per_field: BTreeMap<String, f64>,
    /// Extra metres a field's change is carried downwind beyond its radius (moisture, heat advected by air, ...).
    pub downwind_m: BTreeMap<String, f64>,
    /// 3-layer vector field holding the carrying wind (cell-major x, y, z). Missing field = no downwind extension.
    pub wind_field: String,
    /// An edge is "downwind" when cos(angle between edge direction and wind) >= this.
    pub cone_cos: f64,
    /// Winds slower than this (m/s) do not carry anything.
    pub min_wind: f64,
}

impl InfluenceSpec {
    /// One radius for every field, no downwind carry.
    pub fn uniform(radius_m: f64) -> Self {
        InfluenceSpec { default_m: radius_m, per_field: BTreeMap::new(), downwind_m: BTreeMap::new(), wind_field: "wind".into(), cone_cos: 0.5, min_wind: 0.1 }
    }
    /// Planet-scale defaults (the spec's examples: a lake changes temperature and humidity over tens of km, moisture far downwind).
    /// Radii are starting guesses that the incremental solver widens when the border still moves; they are not tuned physics.
    pub fn earth_like() -> Self {
        let mut s = Self::uniform(300e3);
        for (f, r) in [
            ("albedo", 50e3),
            ("ice_fraction", 100e3),
            ("temperature", 600e3),
            ("temperature_sl", 600e3),
            ("sst", 1500e3),
            ("pressure", 800e3),
            ("wind", 800e3),
            ("wind_speed", 800e3),
            ("vapor", 200e3),
            ("evaporation", 50e3),
            ("precipitation", 200e3),
            ("runoff", 200e3),
            ("discharge", 200e3),
            ("elevation", 0.0),
        ] {
            s.per_field.insert(f.into(), r);
        }
        for (f, m) in [("vapor", 3000e3), ("precipitation", 2000e3), ("runoff", 1000e3)] {
            s.downwind_m.insert(f.into(), m);
        }
        s
    }
    pub fn radius_for(&self, field: &str) -> f64 { self.per_field.get(field).copied().unwrap_or(self.default_m) }
    pub fn downwind_for(&self, field: &str) -> f64 { self.downwind_m.get(field).copied().unwrap_or(0.0) }
    pub fn with_field(mut self, field: &str, radius_m: f64) -> Self { self.per_field.insert(field.into(), radius_m); self }
    pub fn with_downwind(mut self, field: &str, extra_m: f64) -> Self { self.downwind_m.insert(field.into(), extra_m); self }
    pub fn with_wind_field(mut self, name: &str) -> Self { self.wind_field = name.into(); self }
}

/// A set of cells with the surface distance of each from the nearest seed (`INFINITY` outside).
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// Sorted cell indices.
    pub cells: Vec<u32>,
    /// Length n_cells; `INFINITY` for cells outside the region.
    pub dist: Vec<f64>,
}

impl Region {
    pub fn empty(n: usize) -> Self { Region { cells: Vec::new(), dist: vec![f64::INFINITY; n] } }
    pub fn len(&self) -> usize { self.cells.len() }
    pub fn is_empty(&self) -> bool { self.cells.is_empty() }
    pub fn contains(&self, c: usize) -> bool { self.dist[c].is_finite() }
    pub fn is_all(&self) -> bool { self.cells.len() == self.dist.len() }
    pub fn mask(&self) -> Vec<bool> { self.dist.iter().map(|d| d.is_finite()).collect() }
    pub fn max_dist(&self) -> f64 { self.cells.iter().map(|&c| self.dist[c as usize]).fold(0.0, f64::max) }
    /// Ring index of a cell: 0 = seeds, k = (k-1)*w < dist <= k*w. `None` outside the region.
    pub fn ring_of(&self, c: usize, ring_width_m: f64) -> Option<u32> {
        let d = self.dist[c];
        if !d.is_finite() {
            None
        } else if d <= 0.0 {
            Some(0)
        } else {
            Some((d / ring_width_m).ceil() as u32)
        }
    }
    pub fn to_dirty(&self) -> crate::module::DirtyRegion { crate::module::DirtyRegion::Cells(self.cells.clone()) }
    /// Cells of the region that have a neighbour outside it (domain boundaries do not count). Empty when the region is everything.
    pub fn border(&self, domain: &Domain) -> Vec<u32> {
        let m = &domain.mesh;
        self.cells.iter().copied().filter(|&c| m.neighbors(c as usize).iter().any(|&j| !self.dist[j as usize].is_finite())).collect()
    }
    pub fn union(&self, other: &Region) -> Region {
        let dist: Vec<f64> = self.dist.iter().zip(&other.dist).map(|(a, b)| a.min(*b)).collect();
        let cells = (0..dist.len() as u32).filter(|&c| dist[c as usize].is_finite()).collect();
        Region { cells, dist }
    }
}

#[derive(PartialEq)]
struct Node(f64, u32);
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) }
}
impl Ord for Node {
    // min-heap on distance, deterministic tie-break on cell index
    fn cmp(&self, o: &Self) -> Ordering { o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal).then_with(|| o.1.cmp(&self.1)) }
}

fn dijkstra<F: Fn(usize, usize, usize) -> bool>(domain: &Domain, seeds: &[(u32, f64)], radius_m: f64, allow: F) -> Region {
    let m = &domain.mesh;
    let n = m.n_cells;
    let mut dist = vec![f64::INFINITY; n];
    let mut heap = BinaryHeap::new();
    for &(c, d0) in seeds {
        if (c as usize) < n && d0 < dist[c as usize] {
            dist[c as usize] = d0;
            heap.push(Node(d0, c));
        }
    }
    while let Some(Node(d, c)) = heap.pop() {
        if d > dist[c as usize] {
            continue;
        }
        let (nb, ed) = (m.neighbors(c as usize), m.edge_dists(c as usize));
        for q in 0..nb.len() {
            let j = nb[q] as usize;
            let nd = d + ed[q];
            if nd <= radius_m && nd < dist[j] && allow(c as usize, j, q) {
                dist[j] = nd;
                heap.push(Node(nd, j as u32));
            }
        }
    }
    // Seed distances beyond the radius cannot occur (seeds start at their own offset); keep them anyway.
    let cells = (0..n as u32).filter(|&c| dist[c as usize].is_finite()).collect();
    Region { cells, dist }
}

/// Every cell within `radius_m` (sum of centre-to-centre edge distances along the shortest mesh path) of any seed. `radius_m = 0` = the seeds.
pub fn grow_region(domain: &Domain, seeds: &[u32], radius_m: f64) -> Region {
    let s: Vec<(u32, f64)> = seeds.iter().map(|&c| (c, 0.0)).collect();
    dijkstra(domain, &s, radius_m.max(0.0), |_, _, _| true)
}

/// Carry a region downwind. From EVERY cell of `region` the front continues along edges whose direction lies within the wind cone of
/// the wind at the cell it leaves, for `along_m` more metres. `wind` is cell-major xyz (`3 * n_cells`). Returns `region` plus the plume.
pub fn extend_downwind(domain: &Domain, region: &Region, wind: &[f64], along_m: f64, cone_cos: f64, min_wind: f64) -> Region {
    if along_m <= 0.0 || region.is_empty() || wind.len() != 3 * domain.n_cells() {
        return region.clone();
    }
    let m = &domain.mesh;
    let seeds: Vec<(u32, f64)> = region.cells.iter().map(|&c| (c, region.dist[c as usize])).collect();
    let base = region.max_dist();
    // Distance accounting: plume distance is measured as region distance of the source + metres travelled along the wind. The cap is
    // `base + along_m` so a seed on the far rim can still carry `along_m`. A per-cell budget would be tighter but needs a second array.
    let carried = dijkstra(domain, &seeds, base + along_m, |a, b, _| {
        let w = [wind[3 * a], wind[3 * a + 1], wind[3 * a + 2]];
        let speed = dot(w, w).sqrt();
        if speed < min_wind {
            return false;
        }
        let e = normalize(sub(m.centroid[b], m.centroid[a]));
        dot(e, [w[0] / speed, w[1] / speed, w[2] / speed]) >= cone_cos
    });
    region.union(&carried)
}

/// Fields downstream of `edited` through the module dependency graph (including `edited` itself). A module is downstream when it reads
/// any field already in the set; all its outputs join. Feedback loops are handled naturally (the set only grows).
pub fn downstream_fields(specs: &[ModuleSpec], edited: &[&str]) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = edited.iter().map(|s| s.to_string()).collect();
    loop {
        let before = set.len();
        for s in specs {
            if s.inputs.iter().any(|i| set.contains(i)) {
                for o in &s.outputs {
                    set.insert(o.clone());
                }
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

/// Per-field regions for one edit.
#[derive(Debug, Clone)]
pub struct DirtyPlan {
    pub seeds: Vec<u32>,
    /// Multiplier applied to every radius and downwind length (the widening factor, 1.0 at the start).
    pub scale: f64,
    pub per_field: BTreeMap<String, Region>,
    /// Union over fields (what a module that only understands one region would receive).
    pub union: Region,
}

/// Build the dirty plan: for each field in the downstream closure of `edited`, grow its radius (times `scale`) around `seeds`, then carry
/// it downwind when the spec says so and `fields` has the wind. The edited fields themselves get exactly the seeds.
pub fn plan_dirty(domain: &Domain, fields: &FieldStore, specs: &[ModuleSpec], edited: &[&str], seeds: &[u32], spec: &InfluenceSpec, scale: f64) -> DirtyPlan {
    let n = domain.n_cells();
    let affected = downstream_fields(specs, edited);
    let wind: Option<&[f64]> = fields.get(&spec.wind_field).filter(|f| f.layers == 3).map(|f| f.data.as_slice());
    let mut per_field = BTreeMap::new();
    let mut union = Region::empty(n);
    let mut cache: BTreeMap<u64, Region> = BTreeMap::new();
    for f in affected {
        let is_edit = edited.contains(&f.as_str());
        let r = if is_edit { 0.0 } else { spec.radius_for(&f) * scale };
        let key = r.to_bits();
        let base = cache.entry(key).or_insert_with(|| grow_region(domain, seeds, r)).clone();
        let reg = match wind {
            Some(w) if !is_edit => extend_downwind(domain, &base, w, spec.downwind_for(&f) * scale, spec.cone_cos, spec.min_wind),
            _ => base,
        };
        union = union.union(&reg);
        per_field.insert(f, reg);
    }
    DirtyPlan { seeds: seeds.to_vec(), scale, per_field, union }
}
