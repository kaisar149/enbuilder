//! Part 4a-2e: pure diagnostics used by `tests/part4.rs` (and by `examples/current_probe.rs`) to judge the ocean current stage on every
//! world shape. Nothing here changes a field; every function takes plain slices so it can be used on any `Domain`.
//!
//!   coast_hops        : graph distance (in cells) from the nearest land-adjacent ocean cell
//!   speed_report      : boundary-current peak and interior median speed in a latitude band
//!   seam_spikes       : does the speed jump at a mesh seam (cube edge, icosahedron pentagon)?
//!   pentagon_cells    : the 12 five-neighbour cells of a geodesic sphere
//!   cross_face_cells  : cube cells that touch another face
//!   nonzonal_fraction : share of a field's second moment that is not explained by its zonal (latitude-band) mean
//!   vorticity_balance : residual of `r zeta + beta v = curl(tau)/(rho H)` for a finished current
use crate::domain::Domain;
use crate::math::{dot, Vec3};
use crate::ocean_forcing::curl_of_masked;
use std::collections::VecDeque;

/// Hops (mesh adjacency steps through ocean cells) from the nearest ocean cell that touches a non-ocean cell.
/// Coast cells get 0, the next ring 1, and so on. Non-ocean cells and ocean cells with no path to a coast get `u32::MAX`.
pub fn coast_hops(domain: &Domain, ocean: &[bool]) -> Vec<u32> {
    let m = &domain.mesh;
    let n = m.n_cells;
    assert_eq!(ocean.len(), n, "coast_hops: mask length mismatch");
    let mut hops = vec![u32::MAX; n];
    let mut queue: VecDeque<usize> = VecDeque::new();
    for i in 0..n {
        if ocean[i] && m.neighbors(i).iter().any(|&j| !ocean[j as usize]) {
            hops[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        for &j in m.neighbors(i) {
            let j = j as usize;
            if ocean[j] && hops[j] == u32::MAX {
                hops[j] = hops[i] + 1;
                queue.push_back(j);
            }
        }
    }
    hops
}

/// Speed statistics inside the latitude band `lat_lo_deg <= |lat| <= lat_hi_deg` (ocean cells only).
#[derive(Debug, Clone, PartialEq)]
pub struct SpeedReport {
    /// Largest speed among BOUNDARY cells (`coast_hops <= 2`, the western boundary layer and its neighbours).
    pub boundary_peak: f64,
    /// Median speed among INTERIOR cells (`coast_hops >= 3`).
    pub interior_median: f64,
    pub n_boundary: usize,
    pub n_interior: usize,
}

/// See [`SpeedReport`]. The latitude of a cell is `asin(up.z)` (spin axis +z).
pub fn speed_report(domain: &Domain, ocean: &[bool], speed: &[f64], lat_lo_deg: f64, lat_hi_deg: f64) -> SpeedReport {
    let n = domain.n_cells();
    assert_eq!(ocean.len(), n, "speed_report: mask length mismatch");
    assert_eq!(speed.len(), n, "speed_report: speed length mismatch");
    let hops = coast_hops(domain, ocean);
    let (mut peak, mut nb) = (0.0f64, 0usize);
    let mut interior: Vec<f64> = Vec::new();
    for i in 0..n {
        if !ocean[i] || hops[i] == u32::MAX {
            continue;
        }
        let lat = domain.mesh.up[i][2].clamp(-1.0, 1.0).asin().to_degrees().abs();
        if lat < lat_lo_deg || lat > lat_hi_deg {
            continue;
        }
        if hops[i] <= 2 {
            peak = peak.max(speed[i]);
            nb += 1;
        } else {
            interior.push(speed[i]);
        }
    }
    interior.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = if interior.is_empty() { 0.0 } else { interior[interior.len() / 2] };
    SpeedReport { boundary_peak: peak, interior_median: med, n_boundary: nb, n_interior: interior.len() }
}

/// Result of [`seam_spikes`].
#[derive(Debug, Clone, PartialEq)]
pub struct SeamReport {
    /// Largest `|s_i - mean(s_neighbours)| / mean(s_neighbours)` over the evaluable candidates (0 if none).
    pub worst: f64,
    /// Candidates that could be judged.
    pub evaluable: usize,
    /// Candidates handed in.
    pub candidates: usize,
    /// Evaluable candidates whose spike exceeds `limit`.
    pub n_over: usize,
}

/// Seam detector. A candidate cell is EVALUABLE when it is ocean, at least 2 hops from any coast (the coast slip legitimately changes the
/// current there), has only ocean neighbours, and its neighbours' mean speed is at least `floor` (m/s; a relative spike on a near-zero
/// speed is meaningless). Its spike is `|s_i - m| / m`, `m` the plain mean of the neighbours' speeds. `limit` only fills `n_over`.
pub fn seam_spikes(domain: &Domain, ocean: &[bool], speed: &[f64], cells: &[usize], limit: f64, floor: f64) -> SeamReport {
    let n = domain.n_cells();
    assert_eq!(ocean.len(), n, "seam_spikes: mask length mismatch");
    assert_eq!(speed.len(), n, "seam_spikes: speed length mismatch");
    let hops = coast_hops(domain, ocean);
    let (mut worst, mut evaluable, mut over) = (0.0f64, 0usize, 0usize);
    for &i in cells {
        if !ocean[i] || hops[i] == u32::MAX || hops[i] < 2 {
            continue;
        }
        let nb = domain.mesh.neighbors(i);
        if nb.is_empty() || nb.iter().any(|&j| !ocean[j as usize]) {
            continue;
        }
        let mean = nb.iter().map(|&j| speed[j as usize]).sum::<f64>() / nb.len() as f64;
        if !(mean >= floor) {
            continue;
        }
        let spike = (speed[i] - mean).abs() / mean;
        evaluable += 1;
        worst = worst.max(spike);
        if spike > limit {
            over += 1;
        }
    }
    SeamReport { worst, evaluable, candidates: cells.len(), n_over: over }
}

/// The cells with exactly five neighbours (the 12 pentagons of a geodesic sphere).
pub fn pentagon_cells(domain: &Domain) -> Vec<usize> { (0..domain.n_cells()).filter(|&i| domain.mesh.degree(i) == 5).collect() }

/// Cube cells (face-major numbering, `cells_per_edge` per face edge) that have a neighbour on another face.
pub fn cross_face_cells(domain: &Domain, cells_per_edge: usize) -> Vec<usize> {
    let per_face = cells_per_edge * cells_per_edge;
    (0..domain.n_cells()).filter(|&i| domain.mesh.neighbors(i).iter().any(|&j| j as usize / per_face != i / per_face)).collect()
}

/// `sum area (f - zonal_mean)^2 / sum area f^2` over the cells of `mask`, where the zonal mean is the area-weighted mean of `f` in the
/// latitude band (width `band_deg`) of the cell. 0 for a purely zonal field, towards 1 for a field with no zonal structure.
pub fn nonzonal_fraction(domain: &Domain, mask: &[bool], f: &[f64], band_deg: f64) -> f64 {
    let n = domain.n_cells();
    assert_eq!(mask.len(), n, "nonzonal_fraction: mask length mismatch");
    assert_eq!(f.len(), n, "nonzonal_fraction: field length mismatch");
    assert!(band_deg > 0.0, "nonzonal_fraction: band_deg must be > 0");
    let nb = (180.0 / band_deg).ceil() as usize + 1;
    let bin = |i: usize| -> usize { (((domain.mesh.up[i][2].clamp(-1.0, 1.0).asin().to_degrees() + 90.0) / band_deg).floor() as usize).min(nb - 1) };
    let (mut sa, mut sf) = (vec![0.0; nb], vec![0.0; nb]);
    for i in 0..n {
        if mask[i] {
            let b = bin(i);
            sa[b] += domain.mesh.area[i];
            sf[b] += domain.mesh.area[i] * f[i];
        }
    }
    let (mut num, mut den) = (0.0, 0.0);
    for i in 0..n {
        if mask[i] {
            let b = bin(i);
            let zm = sf[b] / sa[b];
            num += domain.mesh.area[i] * (f[i] - zm).powi(2);
            den += domain.mesh.area[i] * f[i] * f[i];
        }
    }
    if den > 0.0 { num / den } else { 0.0 }
}

/// Relative residual `||r zeta + beta v - curl/(rho H)|| / ||curl/(rho H)||` (2-norm over cells) on ocean cells at least `min_hops`
/// from the coast, with `zeta = curl_of_masked(u)` and `v = u . north`. Returns `(residual, number of cells used)`.
pub fn vorticity_balance(domain: &Domain, ocean: &[bool], u: &[Vec3], beta: &[f64], curl_tau: &[f64], r: f64, rho: f64, h: f64, min_hops: u32) -> (f64, usize) {
    let n = domain.n_cells();
    assert!(u.len() == n && beta.len() == n && curl_tau.len() == n && ocean.len() == n, "vorticity_balance: length mismatch");
    let zeta = curl_of_masked(domain, u, ocean);
    let hops = coast_hops(domain, ocean);
    let (mut num, mut den, mut cnt) = (0.0, 0.0, 0usize);
    for i in 0..n {
        if !ocean[i] || hops[i] == u32::MAX || hops[i] < min_hops {
            continue;
        }
        let forcing = curl_tau[i] / (rho * h);
        let lhs = r * zeta[i] + beta[i] * dot(u[i], domain.frames[i].north);
        num += (lhs - forcing).powi(2);
        den += forcing * forcing;
        cnt += 1;
    }
    (if den > 0.0 { (num / den).sqrt() } else { 0.0 }, cnt)
}
