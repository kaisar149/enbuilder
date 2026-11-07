//! Part 4a-3: ocean heat transport and the SST feedback to the energy balance.
//! Worlds: (1) wrapped plane with a uniform current and a sinusoidal atmosphere (analytic steady state), (2) the Earth-like four-cap sphere
//! with the analytic zonal stress (same world as `tests/part4.rs`), (3) the real climate pipeline at L3 for the outer coupling.
use enbuilder::generators::*;
use enbuilder::math::{dot, normalize, scale, Vec3};
use enbuilder::ocean::*;
use enbuilder::ocean_currents::*;
use enbuilder::ocean_forcing::*;
use enbuilder::ocean_heat::*;
use enbuilder::testworld::{DAY_S, YEAR_S};
use enbuilder::*;

const R: f64 = 6.371e6;
const OMEGA: f64 = 7.292e-5;

const EARTH_CAPS: [([f64; 3], f64); 4] = [([1.0, 0.2, 0.3], 42.0), ([-0.8, 0.5, -0.1], 36.0), ([0.1, -0.9, -0.4], 30.0), ([0.0, 0.0, -1.0], 20.0)];

fn lat(d: &Domain, i: usize) -> f64 { d.mesh.up[i][2].asin() }

/// Earth-like caps world with a light, `elevation`, the ocean fields and `current` from the real current module under the analytic stress.
fn caps_world(level: u32) -> (Domain, FieldStore, Vec<bool>) {
    let mut d = generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap();
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR_S, DAY_S, 23.4f64.to_radians()));
    let n = d.n_cells();
    let land: Vec<bool> = (0..n).map(|i| EARTH_CAPS.iter().any(|(ax, th)| dot(d.mesh.up[i], normalize(*ax)) > th.to_radians().cos())).collect();
    let z: Vec<f64> = (0..n).map(|i| if land[i] { 100.0 } else { -3000.0 }).collect();
    let (mask, comp) = ocean_cells(&d, &z, &vec![0.0; n], 1.0);
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].east, -0.1 * (3.0 * lat(&d, i)).cos())).collect();
    let curl = curl_of_masked(&d, &tau, &mask);
    let f2: Vec<f64> = (0..n).map(|i| 2.0 * OMEGA * d.mesh.up[i][2]).collect();
    let beta = beta_of(&d, &f2);
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, z).unwrap();
    f.put("ocean_mask", 1, mask.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect()).unwrap();
    f.put("ocean_component", 1, comp.iter().map(|&c| c as f64).collect()).unwrap();
    f.put("stress_curl", 1, curl).unwrap();
    f.put("beta", 1, beta).unwrap();
    let mut oc = OceanCurrents::new();
    oc.run(&d, &mut f, &DirtyRegion::All).unwrap();
    (d, f, mask)
}

fn copy_fields(f: &FieldStore) -> FieldStore {
    let mut g = FieldStore::new(f.n_cells());
    for name in f.names() {
        g.put(&name, f.layers(&name).unwrap(), f.data(&name).unwrap().to_vec()).unwrap();
    }
    g
}

fn t_zonal(d: &Domain) -> Vec<f64> { (0..d.n_cells()).map(|i| 300.0 - 40.0 * lat(d, i).sin().powi(2)).collect() }

fn run_heat(d: &Domain, f: &mut FieldStore, params: &[(&str, f64)]) -> OceanHeat {
    let mut m = OceanHeat::new();
    for &(k, v) in params {
        m.set_param(k, v).unwrap();
    }
    m.run(d, f, &DirtyRegion::All).unwrap();
    m
}

fn area_sum(d: &Domain, a: &[f64]) -> f64 { (0..d.n_cells()).map(|i| d.mesh.area[i] * a[i]).sum() }

fn plane_wrap(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wrap, surface_gravity: 9.81, seed: 1 }).unwrap()
}

// ------------------------------------------------------------------------------------------------ analytic plane
#[test]
fn uniform_current_over_a_sinusoidal_atmosphere_matches_the_analytic_lag_and_damping() {
    // u dT/dx = (Tatm - T)/tau with Tatm = T0 + a sin(kx): T = T0 + a/sqrt(1+s^2) sin(kx - atan s), s = u k tau. kappa = 0 isolates the advection.
    let (nx, ny) = (100usize, 4usize);
    let d = plane_wrap(nx, ny);
    let n = d.n_cells();
    let lx = nx as f64 * 10_000.0;
    let k = 2.0 * std::f64::consts::PI / lx;
    let (u, tau, a) = (0.01, 5.0e6, 5.0);
    let x: Vec<f64> = (0..n).map(|i| d.mesh.centroid[i][0]).collect();
    let t_atm: Vec<f64> = x.iter().map(|&xi| 285.0 + a * (k * xi).sin()).collect();
    let mut f = FieldStore::new(n);
    f.put("temperature_sl", 1, t_atm.clone()).unwrap();
    f.put("ocean_mask", 1, vec![1.0; n]).unwrap();
    f.put("current", 3, (0..n).flat_map(|_| [u, 0.0, 0.0]).collect()).unwrap();
    let m = run_heat(&d, &mut f, &[("diffusivity", 0.0), ("project", 1.0), ("steady_tol_k", 1e-9)]);
    let s = u * k * tau;
    let amp = a / (1.0 + s * s).sqrt();
    let phase = s.atan();
    let sst = f.data("sst").unwrap();
    let mut worst = 0.0f64;
    for i in 0..n {
        let exact = 285.0 + amp * (k * x[i] - phase).sin();
        worst = worst.max((sst[i] - exact).abs());
    }
    println!("plane analytic: s {s:.3}, amplitude {amp:.3} K, worst error {worst:.4} K, steps {}", m.params().get("last_steps").unwrap());
    // First-order upwind adds numerical diffusion u dx/2 = 50 m^2/s, so a few percent of the amplitude is expected.
    assert!(worst < 0.06 * a, "worst error {worst}");
    // The flux equals C u dT/dx = C (T - Tatm)/tau at steady state.
    let q = f.data("ocean_heat_flux").unwrap();
    let c = DEFAULT_MIXED_LAYER_HEAT_CAPACITY;
    let mut qerr = 0.0f64;
    for i in 0..n {
        qerr = qerr.max((q[i] - c * (sst[i] - t_atm[i]) / tau).abs());
    }
    assert!(qerr < 0.02 * c * a / tau * 1.0 + 1e-9, "air-sea mismatch {qerr}");
    assert!(m.params().get("air_sea_mismatch").unwrap() < 0.02);
    assert!(area_sum(&d, q).abs() <= 1e-9 * area_sum(&d, &q.iter().map(|v| v.abs()).collect::<Vec<_>>()));
}

#[test]
fn a_uniform_atmosphere_and_a_still_ocean_give_no_flux() {
    let (d, mut f, mask) = caps_world(3);
    let n = d.n_cells();
    // Uniform atmosphere with the real gyre current: a uniform tracer must stay uniform (the flux is divergence-free after projection).
    f.put("temperature_sl", 1, vec![285.0; n]).unwrap();
    let m = run_heat(&d, &mut f, &[]);
    let (sst, q) = (f.data("sst").unwrap(), f.data("ocean_heat_flux").unwrap());
    for i in 0..n {
        if mask[i] {
            assert!((sst[i] - 285.0).abs() < 1e-6, "cell {i}: {}", sst[i]);
            assert!(q[i].abs() < 1e-3, "cell {i}: {}", q[i]);
        } else {
            assert_eq!((sst[i], q[i]), (0.0, 0.0));
        }
    }
    println!("raw divergence of the masked current flux (relative): {:.3e}", m.params().get("div_raw_rel").unwrap());
    // Still ocean: SST equals the atmosphere exactly, flux zero.
    let mut g = FieldStore::new(n);
    g.put("temperature_sl", 1, t_zonal(&d)).unwrap();
    g.put("ocean_mask", 1, f.data("ocean_mask").unwrap().to_vec()).unwrap();
    g.put("current", 3, vec![0.0; 3 * n]).unwrap();
    run_heat(&d, &mut g, &[("diffusivity", 0.0), ("t_freeze", 100.0)]);
    let t = t_zonal(&d);
    for i in 0..n {
        if mask[i] {
            assert!((g.data("sst").unwrap()[i] - t[i]).abs() < 1e-3, "{i}");
            assert!(g.data("ocean_heat_flux").unwrap()[i].abs() < 1e-3);
        }
    }
}

// ------------------------------------------------------------------------------------------------ sphere
#[test]
fn ocean_heat_flux_integrates_to_zero_and_the_sst_is_steady() {
    let (d, mut f, mask) = caps_world(4);
    f.put("temperature_sl", 1, t_zonal(&d)).unwrap();
    let m = run_heat(&d, &mut f, &[]);
    let q = f.data("ocean_heat_flux").unwrap();
    let tot = area_sum(&d, q);
    let abs = area_sum(&d, &q.iter().map(|v| v.abs()).collect::<Vec<_>>());
    let qmax = q.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    println!("L4 gyre world: sum(A Q) = {tot:.3e} W, sum(A |Q|) = {abs:.3e} W, max |Q| {qmax:.1} W/m^2, steps {}, mismatch {:.2e}", m.params().get("last_steps").unwrap(), m.params().get("air_sea_mismatch").unwrap());
    assert!(qmax > 1.0, "the gyres must move heat: max |Q| {qmax}");
    assert!(tot.abs() <= 1e-9 * abs, "heat created: {tot} of {abs}");
    assert!(m.params().get("flux_closure").unwrap() <= 1e-9);
    assert!(m.params().get("air_sea_mismatch").unwrap() < 0.01, "SST not steady");
    for i in 0..d.n_cells() {
        if !mask[i] {
            assert_eq!(q[i], 0.0);
        }
    }
    // Q is exactly linear in the heat capacity (the steady SST does not depend on it).
    let (d2, mut f2, _) = caps_world(4);
    f2.put("temperature_sl", 1, t_zonal(&d2)).unwrap();
    run_heat(&d2, &mut f2, &[("mixed_layer_heat_capacity", 2.0 * DEFAULT_MIXED_LAYER_HEAT_CAPACITY)]);
    for i in 0..d.n_cells() {
        assert!((f2.data("ocean_heat_flux").unwrap()[i] - 2.0 * q[i]).abs() <= 1e-9 * (1.0 + q[i].abs()), "{i}");
        assert!((f2.data("sst").unwrap()[i] - f.data("sst").unwrap()[i]).abs() < 1e-9);
    }
}

#[test]
fn warm_western_boundary_currents_carry_heat_poleward() {
    // Subtropical gyres flow poleward along western boundaries in both hemispheres, so at the same latitude the western ocean is warmer.
    let (d, mut f, mask) = caps_world(5);
    let t = t_zonal(&d);
    f.put("temperature_sl", 1, t.clone()).unwrap();
    run_heat(&d, &mut f, &[]);
    let sst = f.data("sst").unwrap().to_vec();
    let cur = f.data("current").unwrap();
    let n = d.n_cells();
    // West/east coast ocean cells: a land neighbour to the west (east) of the cell.
    let (mut w, mut e) = (vec![], vec![]);
    for i in 0..n {
        if !mask[i] {
            continue;
        }
        let l = lat(&d, i).to_degrees().abs();
        if !(25.0..=55.0).contains(&l) {
            continue;
        }
        let (mut west, mut east) = (false, false);
        for k in d.mesh.adj_offsets[i] as usize..d.mesh.adj_offsets[i + 1] as usize {
            let j = d.mesh.adj[k] as usize;
            if !mask[j] {
                let dv = [d.mesh.centroid[j][0] - d.mesh.centroid[i][0], d.mesh.centroid[j][1] - d.mesh.centroid[i][1], d.mesh.centroid[j][2] - d.mesh.centroid[i][2]];
                let ew = dot(dv, d.frames[i].east);
                if ew < -0.5 * dot(dv, dv).sqrt() { west = true; }
                if ew > 0.5 * dot(dv, dv).sqrt() { east = true; }
            }
        }
        let poleward = dot([cur[3 * i], cur[3 * i + 1], cur[3 * i + 2]], d.frames[i].north) * d.mesh.up[i][2].signum();
        if west && !east { w.push((i, sst[i] - t[i], poleward)); }
        if east && !west { e.push((i, sst[i] - t[i], poleward)); }
    }
    let mean = |v: &[(usize, f64, f64)], c: usize| v.iter().map(|x| if c == 0 { x.1 } else { x.2 }).sum::<f64>() / v.len() as f64;
    println!("L5: west-coast cells {} (SST anomaly {:+.2} K, poleward speed {:+.3} m/s); east-coast cells {} ({:+.2} K, {:+.3} m/s)", w.len(), mean(&w, 0), mean(&w, 1), e.len(), mean(&e, 0), mean(&e, 1));
    assert!(w.len() > 20 && e.len() > 20);
    assert!(mean(&w, 1) > 0.0 && mean(&e, 1) < 0.0, "current sense");
    assert!(mean(&w, 0) > mean(&e, 0) + 0.5, "west {} east {}", mean(&w, 0), mean(&e, 0));
}

#[test]
fn the_ocean_flux_variant_of_the_energy_balance_is_bit_identical_for_a_zero_flux() {
    let (d, f0, _) = caps_world(3);
    let run = |variant: bool| {
        let mut f = copy_fields(&f0);
        enbuilder::energy::seed_temperature(&mut f, 288.0).unwrap();
        let mut s = Scheduler::new();
        if variant {
            seed_ocean_heat_flux(&mut f).unwrap();
            add_energy_stage_ocean_flux(&mut s).unwrap();
        } else {
            add_energy_stage(&mut s).unwrap();
        }
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        (f.data("temperature_sl").unwrap().to_vec(), f.data("temperature").unwrap().to_vec(), f.data("albedo").unwrap().to_vec())
    };
    assert_eq!(run(false), run(true));
}

fn coupled_land_response(d: &Domain, f0: &FieldStore, mask: &[bool]) -> (f64, f64, f64, usize, usize, Vec<f64>, Vec<f64>) {
    let n = d.n_cells();
    // Baseline: no ocean transport.
    let mut fb = copy_fields(f0);
    enbuilder::energy::seed_temperature(&mut fb, 288.0).unwrap();
    let mut sb = Scheduler::new();
    add_energy_stage(&mut sb).unwrap();
    sb.run(d, &mut fb, &DirtyRegion::All).unwrap();
    // Coupled loop `ocean_climate`.
    let mut fc = copy_fields(f0);
    enbuilder::energy::seed_temperature(&mut fc, 288.0).unwrap();
    seed_ocean_heat_flux(&mut fc).unwrap();
    let mut sc = Scheduler::new();
    add_ocean_climate_loop(&mut sc).unwrap();
    let rep = sc.run(d, &mut fc, &DirtyRegion::All).unwrap();
    let l = rep.loops.iter().find(|l| l.name == "ocean_climate").unwrap();
    println!("ocean_climate: {} passes, residual {:.2e}, converged {}", l.iterations, l.residual, l.converged);
    assert!(l.converged && l.iterations <= 30);
    let (tb, tc) = (fb.data("temperature_sl").unwrap(), fc.data("temperature_sl").unwrap());
    let (mut we, mut wn, mut ee, mut en) = (0.0, 0usize, 0.0, 0usize);
    for i in 0..n {
        let l = lat(d, i).to_degrees().abs();
        if mask[i] || !(20.0..=55.0).contains(&l) {
            continue;
        }
        let (mut ocean_west, mut ocean_east) = (false, false);
        for k in d.mesh.adj_offsets[i] as usize..d.mesh.adj_offsets[i + 1] as usize {
            let j = d.mesh.adj[k] as usize;
            if mask[j] {
                let dv = [d.mesh.centroid[j][0] - d.mesh.centroid[i][0], d.mesh.centroid[j][1] - d.mesh.centroid[i][1], d.mesh.centroid[j][2] - d.mesh.centroid[i][2]];
                let ew = dot(dv, d.frames[i].east);
                let len = dot(dv, dv).sqrt();
                if ew < -0.5 * len { ocean_west = true; }
                if ew > 0.5 * len { ocean_east = true; }
            }
        }
        // Ocean to the west of the land cell = the land is on the EASTERN side of the basin (cold equatorward current).
        if ocean_west && !ocean_east { ee += tc[i] - tb[i]; en += 1; }
        if ocean_east && !ocean_west { we += tc[i] - tb[i]; wn += 1; }
    }
    // Reference: all land in the same latitude range, so the response of the whole band (convexity of emission, ice) can be subtracted.
    let (mut ra, mut rn) = (0.0, 0usize);
    for i in 0..n {
        let l = lat(d, i).to_degrees().abs();
        if !mask[i] && (20.0..=55.0).contains(&l) {
            ra += tc[i] - tb[i];
            rn += 1;
        }
    }
    println!("land 20..55 band mean dT {:+.2} K (n {rn})", ra / rn.max(1) as f64);
    (we / wn.max(1) as f64, ee / en.max(1) as f64, ra / rn.max(1) as f64, wn, en, tb.to_vec(), tc.to_vec())
}

#[test]
fn a_cold_coast_current_cools_the_adjacent_land_and_a_warm_one_warms_it() {
    let (d, f0, mask) = caps_world(4);
    let (warm, cold, band, wn, en, tb, tc) = coupled_land_response(&d, &f0, &mask);
    println!("land next to a western-boundary (warm) current: dT {warm:+.2} K (n {wn}); next to an eastern-boundary (cold) current: dT {cold:+.2} K (n {en})");
    assert!(wn > 10 && en > 10);
    // Heat moved poleward warms the whole mid-latitude land band (about +1 K here: equalising T lowers emission at a fixed mean,
    // so the mean rises, plus less ice). The coast effect is the departure from that band response.
    assert!(band > 0.0, "band response {band}");
    assert!(cold - band < -0.2, "cold-current coast relative to the band: {}", cold - band);
    assert!(warm - band > 0.2, "warm-current coast relative to the band: {}", warm - band);
    assert!(warm > cold + 0.5, "warm {warm} cold {cold}");
    // The global mean is not shifted much (heat is moved): bounded by the nonlinearity of emission only.
    let n = d.n_cells();
    let tot = d.mesh.total_area();
    let (mb, mc) = ((0..n).map(|i| d.mesh.area[i] * tb[i]).sum::<f64>() / tot, (0..n).map(|i| d.mesh.area[i] * tc[i]).sum::<f64>() / tot);
    println!("global mean temperature_sl: without {mb:.2} K, with ocean transport {mc:.2} K");
    assert!((mb - mc).abs() < 1.5);
}

#[test]
fn the_ocean_climate_loop_converges_within_thirty_passes_on_the_l4_sphere_and_is_deterministic() {
    let (d, f0, mask) = caps_world(4);
    let a = coupled_land_response(&d, &f0, &mask);
    let b = coupled_land_response(&d, &f0, &mask);
    assert_eq!(a.6, b.6);
}

// ------------------------------------------------------------------------------------------------ interface
#[test]
fn parameters_spec_and_errors() {
    let mut m = OceanHeat::new();
    let p = m.params();
    assert_eq!(p.get("mixed_layer_heat_capacity"), Some(DEFAULT_MIXED_LAYER_HEAT_CAPACITY));
    assert_eq!((p.get("sst_relax_s"), p.get("diffusivity")), (Some(5.0e6), Some(1000.0)));
    for ro in ["last_steps", "last_residual_k", "flux_closure", "div_raw_rel", "air_sea_mismatch", "ocean_cells"] {
        assert!(m.set_param(ro, 1.0).is_err(), "{ro}");
    }
    assert!(m.set_param("cfl", 1.5).is_err() && m.set_param("diffusivity", -1.0).is_err() && m.set_param("nope", 1.0).is_err() && m.set_param("project", 0.5).is_err());
    let s = m.spec();
    assert_eq!(s.name, "ocean_heat");
    assert_eq!(s.outputs, vec!["sst".to_string(), "ocean_heat_flux".to_string()]);
    assert!(s.inputs.contains(&"current".to_string()) && s.inputs.contains(&"temperature_sl".to_string()));
    assert!(EnergyBalance::with_ocean_heat().spec().inputs.contains(&"ocean_heat_flux".to_string()));
    assert!(!EnergyBalance::new().spec().inputs.contains(&"ocean_heat_flux".to_string()));
    // Missing input through the scheduler.
    let (d, mut f, _) = caps_world(2);
    let mut sc = Scheduler::new();
    sc.register(Box::new(OceanHeat::new())).unwrap();
    assert!(sc.run(&d, &mut f, &DirtyRegion::All).is_err()); // no temperature_sl
    // No ocean at all: runs, all zero.
    let n = d.n_cells();
    let mut g = FieldStore::new(n);
    g.put("temperature_sl", 1, vec![280.0; n]).unwrap();
    g.put("ocean_mask", 1, vec![0.0; n]).unwrap();
    g.put("current", 3, vec![0.0; 3 * n]).unwrap();
    OceanHeat::new().run(&d, &mut g, &DirtyRegion::All).unwrap();
    assert!(g.data("sst").unwrap().iter().all(|&v| v == 0.0) && g.data("ocean_heat_flux").unwrap().iter().all(|&v| v == 0.0));
}

// ------------------------------------------------------------------------------------------------ real pipeline, outer coupling
#[test]
fn the_outer_coupling_runs_through_the_real_pipeline() {
    let (d, mut f) = enbuilder::testworld::earth_like_world(3).unwrap();
    let mut a = Scheduler::new();
    add_atmosphere_ocean_pipeline(&mut a).unwrap();
    let mut b = Scheduler::new();
    add_ocean_climate_loop(&mut b).unwrap();
    let rep = run_ocean_climate(&d, &mut f, &mut a, &mut b, 0.0, 2).unwrap();
    println!("{rep:?}");
    assert!(rep.heat_loop_converged.iter().all(|&c| c));
    assert!(rep.heat_loop_passes.iter().all(|&p| p >= 1 && p <= 30));
    assert!(rep.sst_change_k[1] < 20.0);
    for name in ["sst", "ocean_heat_flux", "current", "temperature_sl", "temperature", "precipitation"] {
        assert!(f.data(name).unwrap().iter().all(|v| v.is_finite()), "{name}");
    }
    let q = f.data("ocean_heat_flux").unwrap();
    let tot = (0..d.n_cells()).map(|i| d.mesh.area[i] * q[i]).sum::<f64>();
    let abs = (0..d.n_cells()).map(|i| d.mesh.area[i] * q[i].abs()).sum::<f64>();
    assert!(tot.abs() <= 1e-6 * abs.max(1.0), "{tot} {abs}");
    assert!(run_ocean_climate(&d, &mut f, &mut a, &mut b, 0.0, 0).is_err());
}
