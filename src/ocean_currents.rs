//! Parts 4a-2c, 4a-2d and 4a-2e: gyres and the current field from the Stommel (friction + beta) vorticity equation.
//!
//! 4a-2e rules (see ARCHITECTURE.md): (1) WALL RULE: on a `Wall`/`Closed` domain an ocean cell with `boundary_len > 0` is pinned like land
//! (`psi = 0`, no current; `wall_pinned_cells`). (2) WATERWORLD RULE: an ocean component with no land-adjacent cell (after the wall rule) has
//! no boundary condition, the problem is singular, so it gets ZERO flow and a warning (`unanchored_cells`). One land cell anywhere in a basin
//! is enough to anchor it. Both rules are computed by [`effective_ocean`].
//!
//! Module and fields
//!   ocean_currents : stress_curl, beta, ocean_mask, ocean_component
//!                    -> psi (m^2/s), current (3 layers, m/s), current_speed (m/s), boundary_layer_m (m)
//!
//! Equation (standard stream-function convention, `u = k x grad(psi)`, relative vorticity `zeta = laplacian(psi)`):
//!     r * laplacian(psi) + beta * d(psi)/d(east) = + curl(tau) / (rho_w * H)     psi = 0 on every non-ocean cell (coast)
//! (the beta term is present when `beta_on = 1`, the default since 4a-2d; `beta_on = 0` is exactly the 4a-2c equation and CG path).
//! With beta on, the system `(-L + N) psi = -curl/(rho H r)` has `N` = upwinded advection of `psi` with velocity `-(beta/r) east`
//! (information travels WEST, so the boundary layer sits at the western coast, width `r/beta`) and is solved with BiCGSTAB.
//! 4a-2c solved as `(-L) psi = -curl(tau) / (rho_w * H * r)` with the 4a-2a CG (`sigma = 0`; the Dirichlet coast makes it
//! positive definite). NOTE the sign differs from the PART text, see ARCHITECTURE.md "Decisions (4a-2c)": with
//! `u = k x grad(psi)` the text's `-curl` would make a positive wind-stress curl drive anticyclonic flow.
//!
//! Velocity: `current_i = up_i x grad(psi)_i` (`GradientOp` on the scalar `psi`, the gradient re-projected onto the
//! tangent plane), then (1) optional coast slip: the component of the current along the net outward land-edge vector of
//! a coast cell is removed, so the net flux through that cell's land edges is exactly 0; (2) the speed is clamped to
//! `max_current` (direction kept) and the number of clamped cells is reported in the output-only parameter
//! `clamped_cells`. Land and lake cells have `psi = 0` and `current = 0`.
use crate::domain::{BoundaryKind, Domain};
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::gradient::GradientOp;
use crate::math::{add, cross, dot, length, scale, sub, Vec3};
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::ocean::{label_components, CgConfig, OceanGraph, OceanSolution};
use crate::scheduler::Scheduler;

/// Default Rayleigh friction (1/s); see ARCHITECTURE.md (4a-2d) for why it is larger than the 4a-2c value of 5e-8.
pub const DEFAULT_R_FRICTION: f64 = 1.0e-5;
/// Default mixed-layer depth (m), tuned in 4a-2e: the wind-driven (Ekman) layer, not the 100 m thermal mixed layer. With the 4a-2d value
/// of 100 m the Earth-like boundary currents peaked near 0.17 m/s; speeds scale exactly as 1/H, so 20 m gives about 0.87 m/s.
pub const DEFAULT_H_MIXED_M: f64 = 20.0;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

/// `u_i = up_i x grad(psi)_i` on ocean cells, exactly 0 elsewhere. `psi` is a GLOBAL field that is 0 on non-ocean cells
/// (so a land neighbour contributes the Dirichlet value 0 to the gradient). The gradient is re-projected onto the
/// tangent plane before the rotation, so the result is tangent to the surface.
pub fn current_from_psi(domain: &Domain, grad: &GradientOp, psi: &[f64], ocean: &[bool]) -> Vec<Vec3> {
    let n = domain.n_cells();
    assert_eq!(psi.len(), n, "current_from_psi: psi length mismatch");
    assert_eq!(ocean.len(), n, "current_from_psi: mask length mismatch");
    let g = grad.gradient(domain, psi);
    (0..n)
        .map(|i| {
            if !ocean[i] {
                return [0.0; 3];
            }
            let up = domain.frames[i].up;
            let gt = sub(g[i], scale(up, dot(g[i], up)));
            cross(up, gt)
        })
        .collect()
}

/// Per cell: `(S_i, L_i)` with `S_i = sum over non-ocean neighbours k of len_ik * n_ik` (the net outward land-edge
/// vector, `n_ik` the unit in-plane direction from i to k, wrapped edges handled as in `GradientOp`) and
/// `L_i = sum len_ik` over the same edges. `(0, 0)` for non-ocean cells and for ocean cells without a land neighbour.
pub fn coast_vectors(domain: &Domain, ocean: &[bool]) -> Vec<(Vec3, f64)> {
    let m = &domain.mesh;
    assert_eq!(ocean.len(), m.n_cells, "coast_vectors: mask length mismatch");
    (0..m.n_cells)
        .map(|i| {
            let mut s: Vec3 = [0.0; 3];
            let mut ls = 0.0;
            if ocean[i] {
                let up = domain.frames[i].up;
                let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
                for k in a..b {
                    let j = m.adj[k] as usize;
                    if ocean[j] {
                        continue;
                    }
                    let dist = m.edge_dist[k];
                    let mut v = sub(m.centroid[j], m.centroid[i]);
                    if length(v) > 1.5 * dist {
                        v = scale(v, -1.0); // wrapped edge
                    }
                    v = sub(v, scale(up, dot(v, up)));
                    let l = length(v);
                    if !(l > 1e-12 * dist) {
                        continue;
                    }
                    s = add(s, scale(v, m.edge_len[k] / l));
                    ls += m.edge_len[k];
                }
            }
            (s, ls)
        })
        .collect()
}

/// Net flux out of each cell through its land (non-ocean) edges, `F_i = sum_k len_ik (u_i . n_ik) = u_i . S_i`
/// (m^2/s; 0 for cells without a land neighbour).
pub fn coast_outflux(domain: &Domain, ocean: &[bool], u: &[Vec3]) -> Vec<f64> {
    assert_eq!(u.len(), domain.n_cells(), "coast_outflux: length mismatch");
    coast_vectors(domain, ocean).iter().zip(u).map(|((s, _), v)| dot(*v, *s)).collect()
}

/// Coast slip: remove from each coast cell's current the component along `S_i / |S_i|`, so that `u_i . S_i = 0`, i.e. no
/// net flow through the cell's land edges. Cells with `|S_i| <= 1e-6 L_i` (land on opposite sides, no defined outward
/// direction) are left alone. Returns the number of cells changed.
pub fn apply_coast_slip(domain: &Domain, ocean: &[bool], u: &mut [Vec3]) -> usize {
    assert_eq!(u.len(), domain.n_cells(), "apply_coast_slip: length mismatch");
    let cv = coast_vectors(domain, ocean);
    let mut changed = 0usize;
    for i in 0..u.len() {
        let (s, ls) = cv[i];
        let sl = length(s);
        if ls > 0.0 && sl > 1e-6 * ls {
            let sh = scale(s, 1.0 / sl);
            u[i] = sub(u[i], scale(sh, dot(u[i], sh)));
            changed += 1;
        }
    }
    changed
}

/// Scale every vector longer than `max_speed` down to exactly that length (direction kept). Returns the number clamped.
pub fn clamp_speed(u: &mut [Vec3], max_speed: f64) -> usize {
    let mut count = 0usize;
    for v in u.iter_mut() {
        let sp = length(*v);
        if sp > max_speed {
            *v = scale(*v, max_speed / sp);
            count += 1;
        }
    }
    count
}

/// The ocean the solver actually sees after the 4a-2e rules.
#[derive(Debug, Clone)]
pub struct EffectiveOcean {
    /// Effective ocean flag per cell: the input mask minus wall-pinned cells minus unanchored components.
    pub ocean: Vec<bool>,
    /// Component id per cell (-1 off-ocean). Equal to the input ids when nothing was pinned.
    pub component: Vec<i64>,
    /// The graph of the effective ocean (`graph.n == 0` for a waterworld).
    pub graph: OceanGraph,
    /// Ocean cells turned into land-like cells because they touch a wall (Wall/Closed domains only).
    pub wall_pinned: usize,
    /// Ocean cells that belong to a basin without any land-adjacent cell (zero flow).
    pub unanchored: usize,
}

/// Apply the 4a-2e wall and waterworld rules (wall pinning always on).
pub fn effective_ocean(domain: &Domain, ocean: &[bool], component: &[i64]) -> Result<EffectiveOcean> { effective_ocean_with(domain, ocean, component, true) }

/// Same with the wall rule switchable (`wall_pin = false` keeps wall edges as no-flux edges, the 4a-2c/2d behaviour).
pub fn effective_ocean_with(domain: &Domain, ocean: &[bool], component: &[i64], wall_pin: bool) -> Result<EffectiveOcean> {
    let n = domain.n_cells();
    if ocean.len() != n || component.len() != n {
        return Err(Error::InvalidParam("effective_ocean: mask/component length does not match the mesh".into()));
    }
    let mut eff = ocean.to_vec();
    let mut comp = component.to_vec();
    let mut pinned = 0usize;
    if wall_pin && matches!(domain.boundary, BoundaryKind::Wall | BoundaryKind::Closed) {
        for i in 0..n {
            if eff[i] && domain.mesh.boundary_len[i] > 0.0 {
                eff[i] = false;
                pinned += 1;
            }
        }
        if pinned > 0 {
            // Pinning can split a basin: relabel (ids ordered by lowest cell index, as `ocean_cells` does).
            let (c, _) = label_components(domain, &eff);
            comp = c;
        }
    }
    for i in 0..n {
        if !eff[i] {
            comp[i] = -1;
        }
    }
    let mut graph = OceanGraph::new(domain, &eff, &comp)?;
    let mut unanchored = 0usize;
    for cells in &graph.comp_cells {
        if !cells.iter().any(|&l| graph.coast_adjacent[l as usize]) {
            for &l in cells {
                eff[graph.cells[l as usize] as usize] = false;
                unanchored += 1;
            }
        }
    }
    if unanchored > 0 {
        for i in 0..n {
            if !eff[i] {
                comp[i] = -1;
            }
        }
        graph = OceanGraph::new(domain, &eff, &comp)?;
    }
    Ok(EffectiveOcean { ocean: eff, component: comp, graph, wall_pinned: pinned, unanchored })
}

/// Western boundary-layer (Stommel) width `r/beta` against the mesh cell size.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryLayerReport {
    /// `r / |beta_i|` (m) per cell; 0 off-ocean and where `|beta| <= 1e-30` (no layer defined).
    pub width_m: Vec<f64>,
    /// Number of ocean cells with a defined layer.
    pub n_cells: usize,
    /// Width in cells (`width / sqrt(area)`): smallest, median over those cells (0 if none).
    pub cells_min: f64,
    pub cells_median: f64,
    /// Fraction of those cells whose layer is thinner than one cell.
    pub under_resolved_fraction: f64,
    /// True when the MEDIAN layer is thinner than one cell (most of the ocean is under-resolved).
    pub under_resolved: bool,
}

/// `r/beta` against `sqrt(area)` on every ocean cell with `|beta| > 1e-30`.
pub fn boundary_layer_report(domain: &Domain, beta: &[f64], ocean: &[bool], r: f64) -> BoundaryLayerReport {
    let n = domain.n_cells();
    assert_eq!(beta.len(), n, "boundary_layer_report: beta length mismatch");
    assert_eq!(ocean.len(), n, "boundary_layer_report: mask length mismatch");
    let mut width = vec![0.0; n];
    let mut ratios: Vec<f64> = Vec::new();
    for i in 0..n {
        if ocean[i] && beta[i].abs() > 1e-30 {
            width[i] = r / beta[i].abs();
            ratios.push(width[i] / domain.mesh.area[i].sqrt());
        }
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (min, med) = if ratios.is_empty() { (0.0, 0.0) } else { (ratios[0], ratios[ratios.len() / 2]) };
    let frac = if ratios.is_empty() { 0.0 } else { ratios.iter().filter(|&&q| q < 1.0).count() as f64 / ratios.len() as f64 };
    BoundaryLayerReport { width_m: width, n_cells: ratios.len(), cells_min: min, cells_median: med, under_resolved_fraction: frac, under_resolved: !ratios.is_empty() && med < 1.0 }
}

/// Human-readable warning (None when the layer is at least one cell wide at the median ocean cell).
pub fn boundary_layer_warning(rep: &BoundaryLayerReport, r: f64) -> Option<String> {
    if !rep.under_resolved {
        return None;
    }
    Some(format!(
        "ocean_currents: the western boundary layer r/beta is thinner than one mesh cell (median {:.2} cells, min {:.2}; {:.0}% of ocean cells under-resolved, r_friction = {:.2e} 1/s). \
         Western intensification will be smeared over about one cell by the upwind scheme: raise r_friction or refine the mesh.",
        rep.cells_median,
        rep.cells_min,
        100.0 * rep.under_resolved_fraction,
        r
    ))
}

// ---------------------------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------------------------

/// Defaults: `H_mixed_m` 20 (m, tuned in 4a-2e), `r_friction` 1e-5 (1/s), `rho_water` 1025, `max_current` 3 (m/s), `beta_on` 1 (0 or 1; 0 is the
/// exact 4a-2c equation), `coast_slip` 1. Output-only (set by `run`, rejected by `set_param`): `clamped_cells`,
/// `max_speed_raw` (m/s before the clamp), `cg_iterations` (iterations of whichever solver ran), `slipped_cells`,
/// `solver_kind` (0 = CG, 1 = BiCGSTAB), `solver_restarts`, `unanchored_cells`, `wall_pinned_cells` (4a-2e), and the boundary-layer report `layer_cells_min`, `layer_cells_median`,
/// `under_resolved_fraction`, `under_resolved` (0/1; text from [`OceanCurrents::warning`]). `wall_pin` (0 or 1, default 1) switches the wall rule off.
pub struct OceanCurrents { params: ParamSet, warning: Option<String> }

impl Default for OceanCurrents {
    fn default() -> Self { Self::new() }
}

impl OceanCurrents {
    pub fn new() -> Self {
        OceanCurrents {
            params: ParamSet::new()
                .with("H_mixed_m", DEFAULT_H_MIXED_M)
                .with("r_friction", DEFAULT_R_FRICTION)
                .with("rho_water", 1025.0)
                .with("max_current", 3.0)
                .with("beta_on", 1.0)
                .with("coast_slip", 1.0)
                .with("wall_pin", 1.0)
                .with("unanchored_cells", 0.0)
                .with("wall_pinned_cells", 0.0)
                .with("clamped_cells", 0.0)
                .with("max_speed_raw", 0.0)
                .with("cg_iterations", 0.0)
                .with("slipped_cells", 0.0)
                .with("solver_kind", 0.0)
                .with("solver_restarts", 0.0)
                .with("layer_cells_min", 0.0)
                .with("layer_cells_median", 0.0)
                .with("under_resolved_fraction", 0.0)
                .with("under_resolved", 0.0),
            warning: None,
        }
    }
    /// Resolution warning from the last `run` (None before the first run, with `beta_on = 0`, or when the layer is resolved).
    pub fn warning(&self) -> Option<&str> { self.warning.as_deref() }
}

impl Module for OceanCurrents {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "ocean_currents".into(),
            inputs: names(&["stress_curl", "beta", "ocean_mask", "ocean_component"]),
            outputs: names(&["psi", "current", "current_speed", "boundary_layer_m"]),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        match name {
            "H_mixed_m" | "r_friction" | "rho_water" | "max_current" => {
                if !value.is_finite() || value <= 0.0 {
                    return Err(Error::InvalidParam(format!("ocean_currents.{name} must be finite and > 0 (got {value})")));
                }
            }
            "beta_on" => {
                if value != 0.0 && value != 1.0 {
                    return Err(Error::InvalidParam(format!("ocean_currents.beta_on must be 0 or 1 (got {value})")));
                }
            }
            "coast_slip" | "wall_pin" => {
                if value != 0.0 && value != 1.0 {
                    return Err(Error::InvalidParam(format!("ocean_currents.{name} must be 0 or 1 (got {value})")));
                }
            }
            _ => return Err(Error::InvalidParam(format!("ocean_currents has no settable parameter '{name}'"))),
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let curl = fields.data("stress_curl")?;
        if curl.len() != n {
            return Err(Error::InvalidParam("ocean_currents: 'stress_curl' must have 1 layer".into()));
        }
        if curl.iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFinite("stress_curl".into()));
        }
        let curl = curl.to_vec();
        let ocean_in: Vec<bool> = fields.data("ocean_mask")?.iter().map(|&m| m > 0.5).collect();
        let comp_in: Vec<i64> = fields.data("ocean_component")?.iter().map(|&v| v.round() as i64).collect();
        // 4a-2e: wall rule and waterworld rule. Everything below works on the EFFECTIVE ocean (pinned and unanchored cells are land-like).
        let wall_pin = self.params.get_or("wall_pin", 1.0) > 0.5;
        let eo = effective_ocean_with(domain, &ocean_in, &comp_in, wall_pin)?;
        let graph = &eo.graph;
        let ocean = &eo.ocean;

        let h = self.params.get_or("H_mixed_m", DEFAULT_H_MIXED_M);
        let r = self.params.get_or("r_friction", DEFAULT_R_FRICTION);
        let rho = self.params.get_or("rho_water", 1025.0);
        let vmax = self.params.get_or("max_current", 3.0);
        let slip = self.params.get_or("coast_slip", 1.0) > 0.5;
        let beta_on = self.params.get_or("beta_on", 1.0) > 0.5;

        // (-L) psi = -curl / (rho H r)   (psi = 0 on land)
        let denom = rho * h * r;
        let b: Vec<f64> = graph.cells.iter().map(|&c| -curl[c as usize] / denom).collect();
        let cfg = CgConfig::default();
        let (mut kind, mut restarts) = (0.0, 0usize);
        let mut layer = BoundaryLayerReport { width_m: vec![0.0; n], n_cells: 0, cells_min: 0.0, cells_median: 0.0, under_resolved_fraction: 0.0, under_resolved: false };
        let beta_field: Option<Vec<f64>> = if beta_on {
            let beta = fields.data("beta")?;
            if beta.len() != n {
                return Err(Error::InvalidParam("ocean_currents: 'beta' must have 1 layer".into()));
            }
            if beta.iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFinite("beta".into()));
            }
            Some(beta.to_vec())
        } else {
            None
        };
        let sol = if graph.n == 0 {
            // Empty effective ocean (no ocean, or a waterworld): nothing to solve, zero flow.
            OceanSolution { psi: Vec::new(), components: Vec::new(), total_iterations: 0 }
        } else if let Some(beta) = &beta_field {
            // Upwinded advection with velocity -(beta/r) east: the system is (-L + N) psi = -curl/(rho H r).
            let c: Vec<Vec3> = (0..n).map(|i| if ocean[i] { scale(domain.frames[i].east, -beta[i] / r) } else { [0.0; 3] }).collect();
            layer = boundary_layer_report(domain, beta, ocean, r);
            let adv = graph.advection(domain, &c)?;
            if adv.max_abs_flux() > 0.0 {
                let s = graph.solve_advective(0.0, &adv, &b, &cfg)?;
                kind = 1.0;
                restarts = s.components.iter().map(|c| c.restarts).sum();
                s
            } else {
                graph.solve(0.0, &b, &cfg)? // beta = 0 everywhere (f-plane): the plain 4a-2c solve, bit-identical
            }
        } else {
            graph.solve(0.0, &b, &cfg)?
        };
        let psi = graph.to_global(&sol.psi);

        let grad = GradientOp::new(domain);
        let mut u = current_from_psi(domain, &grad, &psi, ocean);
        let slipped = if slip { apply_coast_slip(domain, ocean, &mut u) } else { 0 };
        let raw_max = u.iter().fold(0.0f64, |m, v| m.max(length(*v)));
        let clamped = clamp_speed(&mut u, vmax);
        let speed: Vec<f64> = u.iter().map(|v| length(*v)).collect();
        let flat: Vec<f64> = u.iter().flat_map(|v| v.to_vec()).collect();
        if flat.iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFinite("current".into()));
        }

        fields.put("psi", 1, psi)?;
        fields.put("current", 3, flat)?;
        fields.put("current_speed", 1, speed)?;
        fields.put("boundary_layer_m", 1, layer.width_m.clone())?;
        let mut warn: Vec<String> = Vec::new();
        if eo.unanchored > 0 {
            warn.push(format!(
                "ocean_currents: {} ocean cells belong to a basin with no land coast (a waterworld or a wrapped all-ocean domain). With no boundary condition the stream-function problem is singular, so these cells get zero flow.",
                eo.unanchored
            ));
        }
        if eo.wall_pinned > 0 {
            warn.push(format!("ocean_currents: {} ocean cells touch a domain wall and are pinned like land (psi = 0, no current).", eo.wall_pinned));
        }
        if beta_on {
            if let Some(t) = boundary_layer_warning(&layer, r) {
                warn.push(t);
            }
        }
        self.warning = if warn.is_empty() { None } else { Some(warn.join(" ")) };
        self.params.set("unanchored_cells", eo.unanchored as f64);
        self.params.set("wall_pinned_cells", eo.wall_pinned as f64);
        self.params.set("solver_kind", kind);
        self.params.set("solver_restarts", restarts as f64);
        self.params.set("layer_cells_min", layer.cells_min);
        self.params.set("layer_cells_median", layer.cells_median);
        self.params.set("under_resolved_fraction", layer.under_resolved_fraction);
        self.params.set("under_resolved", if layer.under_resolved { 1.0 } else { 0.0 });
        self.params.set("clamped_cells", clamped as f64);
        self.params.set("max_speed_raw", raw_max);
        self.params.set("cg_iterations", sol.total_iterations as f64);
        self.params.set("slipped_cells", slipped as f64);
        Ok(())
    }
}

/// Registers `ocean_currents` (needs `stress_curl`, `beta`, `ocean_mask`, `ocean_component`; see `add_gyre_forcing_stage` and
/// `add_ocean_stage`).
pub fn add_ocean_currents_stage(s: &mut Scheduler) -> Result<()> { s.register(Box::new(OceanCurrents::new())) }
