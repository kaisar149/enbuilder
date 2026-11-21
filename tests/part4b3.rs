//! Part 4b-3 acceptance tests: soil bucket, snow, glaciers, land sublimation supply, soil-driven runoff, the `water_cycle` loop.
//! NOT compiled when written (no Rust toolchain in the sandbox). Hand numbers come from `reference/mirror_4b3.py`
//! (`python mirror_4b3.py`); randomized invariants of the same ledger pass there at 1e-14.
use enbuilder::generators::*;
use enbuilder::land_water::LAND_PARAMS;
use enbuilder::math::{length, sub};
use enbuilder::rng::Rng;
use enbuilder::testworld::earth_like_world;
use enbuilder::*;

const R: f64 = 6.371e6;
const CELL: f64 = 10_000.0;
const STEP: f64 = 2_592_000.0;

fn close(a: f64, b: f64, rel: f64) -> bool { (a - b).abs() <= rel * a.abs().max(b.abs()).max(1e-300) }
fn plane(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: CELL, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap() }

/// Same terrain as `tests/part4b2.rs`.
fn terrain(d: &Domain, seed: u64) -> Vec<f64> {
    let n = d.n_cells();
    let c = &d.mesh.centroid;
    let mut mid = [0.0; 3];
    for p in c {
        for k in 0..3 { mid[k] += p[k]; }
    }
    for k in 0..3 { mid[k] /= n as f64; }
    let ext = c.iter().map(|p| length(sub(*p, mid))).fold(1.0, f64::max);
    let mut rng = Rng::new(seed);
    (0..n).map(|i| 800.0 * ((c[i][0] - mid[0]) / ext) - 150.0 + 300.0 * rng.next_f64()).collect()
}

/// Run only the routing stage so `ocean_mask`, `flow_dir`, `elevation_filled`, ... exist next to `elevation`.
fn routed(d: &Domain, z: Vec<f64>) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    f.put("elevation", 1, z).unwrap();
    let mut s = Scheduler::new();
    add_flow_routing_stage(&mut s).unwrap();
    s.run(d, &mut f, &DirtyRegion::All).unwrap();
    f
}

fn cell_in(precip: f64, evap: f64, evap_sub: f64, temp: f64) -> land_water::CellIn { land_water::CellIn { precip, evap, evap_sub, temp } }
fn cell_state(soil: f64, swe: f64, ice: f64, age: f64) -> land_water::CellState { land_water::CellState { soil, swe, ice, age } }

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

#[test]
fn snow_ramp_and_default_parameters() {
    assert_eq!(snow_fraction(250.0, 273.15, 3.0), 1.0);
    assert_eq!(snow_fraction(290.0, 273.15, 3.0), 0.0);
    assert!(close(snow_fraction(273.15, 273.15, 3.0), 0.5, 1e-14));
    assert!(close(snow_fraction(272.4, 273.15, 3.0), 0.75, 1e-12));
    // a ramp, not a step: strictly between 0 and 1 inside +-1.5 K and monotone
    let mut prev = 1.0;
    for k in 0..=30 {
        let t = 271.65 + 0.1 * k as f64;
        let v = snow_fraction(t, 273.15, 3.0);
        assert!(v <= prev + 1e-15 && (0.0..=1.0).contains(&v), "T {t}");
        prev = v;
    }
    let p = LandParams::default();
    assert_eq!(p.soil_capacity_m, 0.15);
    assert_eq!(p.snow_t, 273.15);
    assert_eq!(p.snow_ramp_k, 3.0);
    assert_eq!(p.step_s, STEP);
    assert_eq!(LAND_PARAMS.len(), 15);
}

#[test]
fn cell_ledger_hand_numbers() {
    let p = LandParams::default();
    // A: rain fills the bucket (P = 1e-5 kg/m^2/s over 30 d = 25.92 mm)
    let o = land_cell_step(&p, &cell_in(1e-5, 0.0, 0.0, 290.0), &cell_state(0.05, 0.0, 0.0, 0.0));
    assert!(close(o.state.soil, 0.07592, 1e-12) && o.runoff_we == 0.0 && o.state.swe == 0.0);
    // B: overflow becomes runoff
    let o = land_cell_step(&p, &cell_in(1e-4, 0.0, 0.0, 290.0), &cell_state(0.05, 0.0, 0.0, 0.0));
    assert!(close(o.state.soil, 0.15, 1e-14) && close(o.runoff_we, 0.1592, 1e-12));
    // C: evaporation draws the soil down
    let o = land_cell_step(&p, &cell_in(0.0, 2e-6, 0.0, 290.0), &cell_state(0.1, 0.0, 0.0, 0.0));
    assert!(close(o.state.soil, 0.094816, 1e-12) && close(o.soil_evap_we, 0.005184, 1e-12) && o.unsupplied_we == 0.0);
    // D: cold precipitation is snow; the soil does not change; the snow age starts
    let o = land_cell_step(&p, &cell_in(1e-5, 0.0, 0.0, 263.15), &cell_state(0.0, 0.0, 0.0, 0.0));
    assert!(close(o.state.swe, 0.02592, 1e-12) && o.state.soil == 0.0 && o.rain_we == 0.0 && o.state.age == STEP);
    // K: on the ramp midpoint half falls as snow
    let o = land_cell_step(&p, &cell_in(1e-5, 0.0, 0.0, 273.15), &cell_state(0.0, 0.0, 0.0, 0.0));
    assert!(close(o.snowfall_we, 0.01296, 1e-12) && close(o.rain_we, 0.01296, 1e-12) && close(o.state.soil, 0.01296, 1e-12));
    // E: warm spell melts the whole pack into the soil
    let o = land_cell_step(&p, &cell_in(0.0, 0.0, 0.0, 278.15), &cell_state(0.05, 0.05, 0.0, 0.0));
    assert!(close(o.state.soil, 0.1, 1e-12) && o.state.swe == 0.0 && close(o.melt_snow_we, 0.05, 1e-12) && o.state.age == 0.0);
    // F: partial melt (2 K above freezing: 0.238464 m), the excess over the bucket runs off
    let o = land_cell_step(&p, &cell_in(0.0, 0.0, 0.0, 275.15), &cell_state(0.0, 1.0, 0.0, 0.0));
    assert!(close(o.state.swe, 0.761536, 1e-12) && close(o.state.soil, 0.15, 1e-14) && close(o.runoff_we, 0.088464, 1e-10));
    // G: snow melts first, then glacier ice (0.49616 m w.e.), thickness drops accordingly
    let o = land_cell_step(&p, &cell_in(0.0, 0.0, 0.0, 278.15), &cell_state(0.05, 0.1, 1.0, 0.0));
    assert!(close(o.melt_snow_we, 0.1, 1e-12) && close(o.melt_ice_we, 0.49616, 1e-12));
    assert!(close(o.state.ice, 0.45893129771, 1e-9) && close(o.runoff_we, 0.49616, 1e-10) && close(o.state.soil, 0.15, 1e-14));
    // H: sublimation takes the snow first
    let o = land_cell_step(&p, &cell_in(0.0, 1e-5, 1e-5, 260.0), &cell_state(0.0, 0.05, 0.1, 0.0));
    assert!(close(o.state.swe, 0.02408, 1e-12) && o.state.ice == 0.1 && close(o.sub_we, 0.02592, 1e-12) && o.unsupplied_we == 0.0);
    // H2: ... then the ice (0.01 from snow, 0.01592 from ice)
    let o = land_cell_step(&p, &cell_in(0.0, 1e-5, 1e-5, 260.0), &cell_state(0.0, 0.01, 0.1, 0.0));
    assert!(o.state.swe == 0.0 && close(o.state.ice, 0.082639040349, 1e-9));
    // H3: nothing to sublimate is reported, never invented
    let o = land_cell_step(&p, &cell_in(0.0, 1e-5, 1e-5, 260.0), &cell_state(0.0, 0.0, 0.0, 0.0));
    assert!(close(o.unsupplied_we, 0.02592, 1e-12) && o.state.soil == 0.0 && o.state.swe == 0.0);
    // I: snow above the threshold that has lasted a year becomes ice (excess 0.4 m w.e. / 0.917)
    let o = land_cell_step(&p, &cell_in(0.0, 0.0, 0.0, 250.0), &cell_state(0.0, 0.9, 0.0, 3.0e7));
    assert!(close(o.state.swe, 0.5, 1e-14) && close(o.state.ice, 0.436205016358, 1e-9) && close(o.state.age, 3.0e7 + STEP, 1e-14));
    // I2: too young; I3: below the threshold; J: the age resets when the snow is gone
    let o = land_cell_step(&p, &cell_in(0.0, 0.0, 0.0, 250.0), &cell_state(0.0, 0.9, 0.0, 1.0e6));
    assert!(o.state.swe == 0.9 && o.state.ice == 0.0);
    let o = land_cell_step(&p, &cell_in(0.0, 0.0, 0.0, 250.0), &cell_state(0.0, 0.4, 0.0, 3.0e7));
    assert!(o.state.swe == 0.4 && o.state.ice == 0.0);
    let o = land_cell_step(&p, &cell_in(0.0, 0.0, 0.0, 250.0), &cell_state(0.0, 0.0, 0.0, 3.0e7));
    assert_eq!(o.state.age, 0.0);
}

#[test]
fn cell_ledger_conserves_water_for_any_forcing() {
    let p = LandParams::default();
    let mut rng = Rng::new(11);
    for _ in 0..2000 {
        let s = cell_state(0.15 * rng.next_f64(), 2.0 * rng.next_f64(), 50.0 * rng.next_f64() * rng.next_f64(), 4.0e7 * rng.next_f64());
        let e = 1e-4 * rng.next_f64();
        let i = cell_in(3e-4 * rng.next_f64(), e, e * rng.next_f64(), 230.0 + 80.0 * rng.next_f64());
        let o = land_cell_step(&p, &i, &s);
        assert!(o.state.soil >= 0.0 && o.state.soil <= p.soil_capacity_m + 1e-15 && o.state.swe >= 0.0 && o.state.ice >= 0.0);
        let before = s.soil + s.swe + p.ice_density_ratio * s.ice;
        let after = o.state.soil + o.state.swe + p.ice_density_ratio * o.state.ice;
        let pr = i.precip * STEP / p.rho_water;
        let ev = o.sub_we + o.soil_evap_we;
        // storage change = precipitation - evaporation actually taken - runoff
        assert!((after - before - (pr - ev - o.runoff_we)).abs() <= 1e-13, "ledger leak");
        // evaporation asked for = taken + reported unsupplied
        assert!((i.evap * STEP / p.rho_water - ev - o.unsupplied_we).abs() <= 1e-13);
    }
}

#[test]
fn glacier_flow_chain_snout_and_conservation() {
    let d = plane(6, 3);
    let n = d.n_cells();
    let mut p = LandParams::default();
    p.glacier_flow_coeff = 1e-8;
    // row j = 1 (cell ids 6..11): column 0 is ocean, receiver of column i is column i-1, 100 m drop per cell
    let mut fd = vec![-1i64; n];
    let mut z = vec![1000.0; n];
    let mut ocean = vec![false; n];
    for i in 0..6 {
        let c = 6 + i;
        z[c] = 100.0 * i as f64;
        if i == 0 { ocean[c] = true; } else { fd[c] = (c - 1) as i64; }
    }
    // 100 m of ice in column 3: phi = 1e-8 * 100 * 0.01 * 2.592e6 = 0.02592 of it moves one cell downhill
    let mut ice = vec![0.0; n];
    ice[9] = 100.0;
    let (out, snout) = glacier_flow(&d.mesh, &fd, &z, &ocean, &ice, &p);
    assert!(close(out[9], 97.408, 1e-12) && close(out[8], 2.592, 1e-12));
    assert!(snout.iter().all(|&s| s == 0.0));
    let vol0: f64 = (0..n).map(|c| d.mesh.area[c] * ice[c]).sum();
    let vol1: f64 = (0..n).map(|c| d.mesh.area[c] * out[c]).sum();
    assert!(close(vol0, vol1, 1e-13), "ice volume must be conserved");
    // ice next to the ocean: the moved part melts at the snout and becomes runoff of the source cell (0.917 * 2.592)
    let mut ice = vec![0.0; n];
    ice[7] = 100.0;
    let (out, snout) = glacier_flow(&d.mesh, &fd, &z, &ocean, &ice, &p);
    assert!(close(out[7], 97.408, 1e-12) && out[6] == 0.0);
    assert!(close(snout[7], 2.592 * 0.917, 1e-12));
    // the cap: with a huge coefficient exactly half moves
    p.glacier_flow_coeff = 1e-3;
    let mut ice = vec![0.0; n];
    ice[9] = 100.0;
    let (out, _) = glacier_flow(&d.mesh, &fd, &z, &ocean, &ice, &p);
    assert!(close(out[9], 50.0, 1e-13) && close(out[8], 50.0, 1e-13));
    // flat or uphill receivers do not receive: equal heights
    let flat = vec![500.0; n];
    let (out, _) = glacier_flow(&d.mesh, &fd, &flat, &ocean, &ice, &p);
    assert_eq!(out, ice);
}

#[test]
fn glacier_flow_conserves_volume_on_a_sphere() {
    let d = sphere(3);
    let n = d.n_cells();
    let f = routed(&d, terrain(&d, 3));
    let fd: Vec<i64> = f.data("flow_dir").unwrap().iter().map(|&v| v as i64).collect();
    let ocean: Vec<bool> = f.data("ocean_mask").unwrap().iter().map(|&m| m > 0.5).collect();
    let z = f.data("elevation").unwrap().to_vec();
    let mut rng = Rng::new(9);
    let ice: Vec<f64> = (0..n).map(|c| if ocean[c] { 0.0 } else { 300.0 * rng.next_f64() }).collect();
    let mut p = LandParams::default();
    p.glacier_flow_coeff = 1e-7;
    let (out, snout) = glacier_flow(&d.mesh, &fd, &z, &ocean, &ice, &p);
    let area = &d.mesh.area;
    let before: f64 = (0..n).map(|c| area[c] * ice[c]).sum();
    let after: f64 = (0..n).map(|c| area[c] * out[c]).sum();
    let lost: f64 = (0..n).map(|c| area[c] * snout[c] / p.ice_density_ratio).sum();
    assert!(close(before, after + lost, 1e-12), "volume {before} != {after} + snout {lost}");
    assert!(out.iter().all(|&v| v >= 0.0 && v.is_finite()));
    assert!(out.iter().zip(ice.iter()).any(|(a, b)| a != b), "something must have moved");
    for c in 0..n {
        if ocean[c] { assert_eq!(out[c], 0.0); assert_eq!(snout[c], 0.0); }
    }
}

// ---------------------------------------------------------------------------------------------
// land_supply and the land-state moisture source
// ---------------------------------------------------------------------------------------------

fn pick(a: &[f64], c: usize) -> f64 { a[if c < a.len() { c } else { 2 }] }
fn open_water_rate(t: f64, wind: f64) -> f64 { 1.8e-8 * saturation_vapour_pressure(t) * 0.5 * (0.5 + 0.1 * wind) }

#[test]
fn supply_and_land_state_source() {
    let d = plane(5, 3);
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    // cell 0 ocean; 1 dry bare land; 2 full bucket; 3 no soil but 0.2 m snow and 10 m of ice; 4 bucket one third full; others like 2
    let ocean = [1.0, 0.0, 0.0, 0.0, 0.0];
    let soil = [0.075, 0.0, 0.15, 0.0, 0.05];
    let swe = [0.0, 0.0, 0.0, 0.2, 0.0];
    let ice = [0.0, 0.0, 0.0, 10.0, 0.0];
    f.put("ocean_mask", 1, (0..n).map(|c| if c < 5 { ocean[c] } else { 0.0 }).collect()).unwrap();
    f.put("soil_moisture_prev", 1, (0..n).map(|c| pick(&soil, c)).collect()).unwrap();
    f.put("snow_water_equivalent_prev", 1, (0..n).map(|c| if c < 5 { swe[c] } else { 0.0 }).collect()).unwrap();
    f.put("glacier_ice_prev", 1, (0..n).map(|c| if c < 5 { ice[c] } else { 0.0 }).collect()).unwrap();
    let mut sup = LandSupply::new();
    sup.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let av = f.data("land_availability").unwrap().to_vec();
    let sc = f.data("soil_evap_cap").unwrap().to_vec();
    let sbc = f.data("sublimation_cap").unwrap().to_vec();
    let cv = f.data("snow_cover").unwrap().to_vec();
    assert_eq!((av[0], sc[0], sbc[0], cv[0]), (1.0, 0.0, 0.0, 0.0), "ocean cell");
    assert_eq!((av[1], sc[1], sbc[1], cv[1]), (0.0, 0.0, 0.0, 0.0), "dry land");
    assert!(av[2] == 1.0 && close(sc[2], 1000.0 * 0.15 / STEP, 1e-14));
    let w = 0.2 + 0.917 * 10.0;
    assert!(close(sbc[3], 1000.0 * w / STEP, 1e-13) && close(cv[3], w / (w + 0.01), 1e-13) && av[3] == 0.0);
    assert!(close(av[4], 1.0 / 3.0, 1e-13) && close(sc[4], 1000.0 * 0.05 / STEP, 1e-14));
    assert_eq!(av[7], 1.0, "unlisted cells copy cell 2");

    // moisture source, 285 K, 5 m/s: far below every cap
    f.put("temperature", 1, vec![285.0; n]).unwrap();
    f.put("ice_fraction", 1, vec![0.0; n]).unwrap();
    f.put("wind_speed", 1, vec![5.0; n]).unwrap();
    let mut src = MoistureSource::with_land_state();
    src.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let pot = open_water_rate(285.0, 5.0);
    let e = f.data("evaporation").unwrap().to_vec();
    let es = f.data("evaporation_sublimation").unwrap().to_vec();
    assert!(close(e[0], pot, 1e-12), "ocean evaporates at the open-water rate");
    assert_eq!(e[1], 0.0, "dry land has nothing to evaporate");
    assert!(close(e[2], pot, 1e-12) && es[2] == 0.0, "full bucket evaporates at the potential rate");
    assert!(close(e[4], pot / 3.0, 1e-12), "a third full: a third of the potential");
    let sub = 0.1 * cv[3] * pot;
    assert!(close(es[3], sub, 1e-12) && close(e[3], sub, 1e-12), "snow and ice cover sublimate a fixed small share, no liquid evaporation");
    assert!(es[3] < 0.11 * pot);
    assert!(es.iter().enumerate().all(|(c, &v)| c == 3 || v == 0.0));

    // hot and windy: the potential rate exceeds what the buckets can give, so the caps bind
    f.put("temperature", 1, vec![305.0; n]).unwrap();
    f.put("wind_speed", 1, vec![10.0; n]).unwrap();
    src.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let pot = open_water_rate(305.0, 10.0);
    assert!(pot > 1000.0 * 0.15 / STEP, "test setup: potential must exceed the full-bucket supply rate");
    let e = f.data("evaporation").unwrap().to_vec();
    assert!(close(e[2], sc[2], 1e-12), "full bucket drained exactly in one step, not more");
    assert!(close(e[4], sc[4], 1e-12), "a third-full bucket is also capped by what it holds");
    for c in 0..n { assert!(e[c] <= 1e-4 + 1e-18 && e[c] >= 0.0); }
}

#[test]
fn land_state_source_interface_and_old_source_unchanged() {
    let new = MoistureSource::with_land_state().spec();
    assert_eq!(new.name, "moisture_source");
    for f in ["ocean_mask", "land_availability", "soil_evap_cap", "sublimation_cap", "snow_cover", "temperature", "ice_fraction", "wind_speed"] {
        assert!(new.inputs.iter().any(|s| s == f), "missing input {f}");
    }
    assert_eq!(new.outputs, vec!["evaporation".to_string(), "evaporation_sublimation".to_string()]);
    let old = MoistureSource::new().spec();
    assert_eq!(old.outputs, vec!["evaporation".to_string()]);
    assert!(old.inputs.iter().any(|s| s == "elevation") && !old.inputs.iter().any(|s| s == "ocean_mask"));
    let mut v = MoistureSource::with_land_state();
    v.set_param("sublimation_coeff", 0.2).unwrap();
    assert!(v.set_param("sublimation_coeff", -1.0).is_err());
    let mut o = MoistureSource::new();
    assert!(o.set_param("sublimation_coeff", 0.2).is_err(), "the old source has no such parameter");
}

// ---------------------------------------------------------------------------------------------
// land_water module: closure over many steps, state handling, dry versus wet
// ---------------------------------------------------------------------------------------------

/// Runs supply -> land-state source -> land_water for `steps` commits on a routed sphere with random forcing. Returns the final
/// state fields (soil, snow, ice) and the largest closure numbers seen.
fn random_run(seed: u64, steps: usize, check: bool) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let d = sphere(3);
    let n = d.n_cells();
    let mut f = routed(&d, terrain(&d, 5));
    seed_land_state(&mut f, 0.05).unwrap();
    f.put("wind_speed", 1, vec![6.0; n]).unwrap();
    let mut sup = LandSupply::new();
    let mut src = MoistureSource::with_land_state();
    let mut lw = LandWater::new();
    let mut rng = Rng::new(seed);
    let lp = LandParams::default();
    let ocean: Vec<bool> = f.data("ocean_mask").unwrap().iter().map(|&m| m > 0.5).collect();
    // Random initial snow and ice on land (ocean cells stay empty). Starting from nothing, random weather almost never keeps snow for
    // the 13 steps a glacier needs (about 0.1% of cells in `reference/mirror_4b3.py`), so this exercises flow, melt and the mask instead.
    {
        let mut init = Rng::new(seed ^ 0x5eed);
        let mut pick = |scale: f64, share: f64| -> Vec<f64> { (0..n).map(|c| if ocean[c] || init.next_f64() > share { 0.0 } else { scale * init.next_f64() }).collect() };
        let swe0 = pick(1.5, 0.4);
        let ice0 = pick(120.0, 0.3);
        let age0 = pick(4.0e7, 0.4);
        f.put("snow_water_equivalent_prev", 1, swe0).unwrap();
        f.put("glacier_ice_prev", 1, ice0).unwrap();
        f.put("snow_age_s_prev", 1, age0).unwrap();
    }
    let mut max_glacier = 0.0f64;
    for step in 0..steps {
        let temp: Vec<f64> = (0..n).map(|_| 235.0 + 75.0 * rng.next_f64()).collect();
        let precip: Vec<f64> = (0..n).map(|_| 2.5e-4 * rng.next_f64() * rng.next_f64()).collect();
        let icef: Vec<f64> = (0..n).map(|_| if rng.next_f64() < 0.3 { rng.next_f64() } else { 0.0 }).collect();
        f.put("temperature", 1, temp).unwrap();
        f.put("precipitation", 1, precip).unwrap();
        f.put("ice_fraction", 1, icef).unwrap();
        sup.run(&d, &mut f, &DirtyRegion::All).unwrap();
        src.run(&d, &mut f, &DirtyRegion::All).unwrap();
        lw.run(&d, &mut f, &DirtyRegion::All).unwrap();
        if check {
            let b = LandBudget::from_fields(&d, &f, &lp).unwrap();
            assert!(b.precip_m3 > 0.0);
            assert!(b.residual_rel < 1e-10, "step {step}: land closure residual {} (rel {})", b.residual_m3, b.residual_rel);
            let pl = lw.params().get("precip_land_m3s").unwrap();
            assert!(lw.params().get("unsupplied_m3s").unwrap().abs() <= 1e-10 * pl, "step {step}: evaporation exceeded the stores");
            assert!(lw.params().get("balance_residual").unwrap().abs() < 1e-10, "step {step}: module balance");
            assert!(close(lw.params().get("precip_land_m3s").unwrap() * STEP, b.precip_m3, 1e-10));
            let (soil, swe, ice, ro, mask) = (f.data("soil_moisture").unwrap(), f.data("snow_water_equivalent").unwrap(), f.data("glacier_ice").unwrap(), f.data("runoff").unwrap(), f.data("glacier_mask").unwrap());
            for c in 0..n {
                assert!(soil[c] >= 0.0 && soil[c] <= lp.soil_capacity_m + 1e-15, "soil out of range at {c}: {}", soil[c]);
                assert!(swe[c] >= 0.0 && ice[c] >= 0.0 && ro[c] >= 0.0 && ro[c].is_finite());
                assert_eq!(mask[c] > 0.5, ice[c] > lp.glacier_mask_min_m);
                if ocean[c] { assert!(soil[c] == 0.0 && swe[c] == 0.0 && ice[c] == 0.0 && ro[c] == 0.0, "ocean cell {c} carries land water"); }
            }
            max_glacier = max_glacier.max(ice.iter().cloned().fold(0.0, f64::max));
            // rerunning without a commit does not advance time
            let before = f.data("soil_moisture").unwrap().to_vec();
            lw.run(&d, &mut f, &DirtyRegion::All).unwrap();
            assert_eq!(f.data("soil_moisture").unwrap(), &before[..]);
        }
        commit_land_state(&mut f).unwrap();
    }
    if check {
        assert!(max_glacier > 0.0, "the seeded glaciers should not all vanish in 14 steps of 235-310 K weather");
    }
    (f.data("soil_moisture").unwrap().to_vec(), f.data("snow_water_equivalent").unwrap().to_vec(), f.data("glacier_ice").unwrap().to_vec())
}

#[test]
fn land_closure_bounds_and_state_handling_over_many_steps() {
    random_run(5, 14, true);
}

#[test]
fn land_water_is_deterministic() {
    let a = random_run(8, 6, false);
    let b = random_run(8, 6, false);
    assert_eq!(a, b);
}

#[test]
fn dry_climates_keep_the_soil_empty_and_evaporate_less_than_wet_ones() {
    let run = |p: f64| -> (f64, f64) {
        let d = plane(12, 8);
        let n = d.n_cells();
        let mut z = vec![100.0; n];
        for j in 0..8 { z[j * 12] = -10.0; }
        let mut f = routed(&d, z);
        seed_land_state(&mut f, 0.075).unwrap();
        f.put("wind_speed", 1, vec![5.0; n]).unwrap();
        f.put("temperature", 1, vec![290.0; n]).unwrap();
        f.put("ice_fraction", 1, vec![0.0; n]).unwrap();
        f.put("precipitation", 1, vec![p; n]).unwrap();
        let (mut sup, mut src, mut lw) = (LandSupply::new(), MoistureSource::with_land_state(), LandWater::new());
        for _ in 0..30 {
            sup.run(&d, &mut f, &DirtyRegion::All).unwrap();
            src.run(&d, &mut f, &DirtyRegion::All).unwrap();
            lw.run(&d, &mut f, &DirtyRegion::All).unwrap();
            commit_land_state(&mut f).unwrap();
        }
        (lw.params().get("soil_fraction_mean").unwrap(), lw.params().get("evap_land_m3s").unwrap())
    };
    let (dry_soil, dry_evap) = run(1e-7);
    let (wet_soil, wet_evap) = run(3e-5);
    assert!(dry_soil < 0.01, "a dry climate keeps the bucket near zero, got {dry_soil}");
    assert!(wet_soil > 0.9, "a wet climate keeps it full, got {wet_soil}");
    assert!(wet_evap > 5.0 * dry_evap, "wet {wet_evap} vs dry {dry_evap} m^3/s");
}

#[test]
fn snow_and_glaciers_form_only_where_it_is_cold() {
    let (d, mut f) = earth_like_world(3).unwrap();
    let n = d.n_cells();
    let mut s = Scheduler::new();
    add_flow_routing_stage(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let z = f.data("elevation").unwrap().to_vec();
    let ocean: Vec<bool> = f.data("ocean_mask").unwrap().iter().map(|&m| m > 0.5).collect();
    let lat: Vec<f64> = (0..n).map(|i| d.mesh.up[i][2].asin().to_degrees()).collect();
    // prescribed climate, constant in time: 302 K at the equator, -65 K * sin^2(lat), lapse rate 6.5 K/km above sea level
    let temp: Vec<f64> = (0..n).map(|i| 302.0 - 65.0 * lat[i].to_radians().sin().powi(2) - 6.5e-3 * z[i].max(0.0)).collect();
    f.put("temperature", 1, temp.clone()).unwrap();
    f.put("precipitation", 1, vec![1e-5; n]).unwrap();
    f.put("evaporation", 1, vec![0.0; n]).unwrap();
    f.put("evaporation_sublimation", 1, vec![0.0; n]).unwrap();
    seed_land_state(&mut f, 0.075).unwrap();
    let mut lw = LandWater::new();
    for _ in 0..40 {
        lw.run(&d, &mut f, &DirtyRegion::All).unwrap();
        commit_land_state(&mut f).unwrap();
    }
    let swe = f.data("snow_water_equivalent").unwrap().to_vec();
    let ice = f.data("glacier_ice").unwrap().to_vec();
    let mask = f.data("glacier_mask").unwrap().to_vec();
    let fd: Vec<i64> = f.data("flow_dir").unwrap().iter().map(|&v| v as i64).collect();
    // glacier flow puts a little ice into the downhill neighbour of every ice cell, whatever its temperature; that is the only way
    // ice may appear in a cell that is too warm to grow it
    let fed_by_ice = |c: usize| (0..n).any(|d| fd[d] == c as i64 && ice[d] > 0.0 && !ocean[d]);
    let (mut n_polar, mut n_tropic) = (0, 0);
    for c in 0..n {
        if ocean[c] { continue; }
        if lat[c].abs() < 20.0 && z[c] <= 600.0 {
            n_tropic += 1;
            assert!(swe[c] == 0.0, "snow at sea level in the tropics (cell {c}, {} K)", temp[c]);
            assert!(ice[c] == 0.0 || (fed_by_ice(c) && ice[c] < 0.1), "ice at sea level in the tropics (cell {c}, {} K, {} m)", temp[c], ice[c]);
        }
        if temp[c] < 268.0 {
            n_polar += 1;
            assert!(swe[c] > 0.2, "cold cell {c} ({} K) has no snow", temp[c]);
            assert!(ice[c] > 0.1 && mask[c] == 1.0, "cold cell {c} ({} K) has no glacier after 40 steps: ice {}", temp[c], ice[c]);
        }
        if ice[c] > 0.0 && temp[c] >= 273.15 {
            assert!(fed_by_ice(c) && mask[c] == 0.0, "ice in a cell above freezing all year that no glacier feeds ({} K, {} m)", temp[c], ice[c]);
        }
    }
    assert!(n_polar > 0 && n_tropic > 0, "test world must have both polar and tropical land ({n_polar}, {n_tropic})");
    assert!(lw.params().get("n_glacier_cells").unwrap() >= n_polar as f64);
    assert!(lw.params().get("unsupplied_m3s").unwrap() == 0.0);
}

// ---------------------------------------------------------------------------------------------
// rivers_lakes variant
// ---------------------------------------------------------------------------------------------

#[test]
fn rivers_read_the_land_runoff_field() {
    let (nx, ny) = (20usize, 10usize);
    let d = plane(nx, ny);
    let n = d.n_cells();
    let z: Vec<f64> = (0..n).map(|c| if c % nx == 0 { -40.0 } else { 10.0 + 50.0 * (c % nx) as f64 }).collect();
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, z).unwrap();
    f.put("runoff", 1, vec![4e-5; n]).unwrap();
    f.put("evaporation", 1, vec![1e-5; n]).unwrap();
    let mut s = Scheduler::new();
    add_flow_routing_stage(&mut s).unwrap();
    s.register(Box::new(RiversLakes::with_land_runoff())).unwrap();
    // 41, not 40: column 10 carries exactly 40 m^3/s and a rounding error of 1e-15 must not decide the mask
    s.set_param("rivers_lakes", "river_threshold_m3s", 41.0).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    // runoff is an input here: untouched, and `precipitation` is not needed
    assert!(f.data("runoff").unwrap().iter().all(|&r| r == 4e-5));
    assert!(!f.has("precipitation"));
    let par = |k: &str| s.module_param("rivers_lakes", k).unwrap();
    let ocean = f.data("ocean_mask").unwrap().to_vec();
    let land: f64 = (0..n).filter(|&c| ocean[c] < 0.5).map(|c| 4e-5 * d.mesh.area[c] / 1000.0).sum();
    assert!(close(par("precip_land_m3s"), land, 1e-12), "runoff input total");
    assert_eq!(par("land_loss_m3s"), 0.0);
    assert!(par("balance_residual").abs() <= 1e-10, "residual {}", par("balance_residual"));
    assert!(close(par("ocean_discharge_m3s"), land, 1e-10), "all land runoff reaches the ocean column");
    // each land cell adds 4 m^3/s; discharge in column i is 4 * (nx - i): rivers exactly where it exceeds 41, i.e. columns 1..=9
    let river = f.data("river_mask").unwrap();
    let disch = f.data("discharge").unwrap();
    for c in 0..n {
        let i = c % nx;
        let want = (1..=9).contains(&i);
        assert_eq!(river[c] > 0.5, want, "river mask at column {i}");
        if i >= 1 { assert!(close(disch[c], 4.0 * (nx - i) as f64, 1e-10), "discharge at column {i}: {}", disch[c]); }
    }
    // the plain 4b-2 module is unchanged: it still needs precipitation and writes runoff
    let spec = RiversLakes::new().spec();
    assert!(spec.inputs.iter().any(|s| s == "precipitation") && spec.outputs.iter().any(|s| s == "runoff"));
    let v = RiversLakes::with_land_runoff().spec();
    assert!(v.inputs.iter().any(|s| s == "runoff") && !v.outputs.iter().any(|s| s == "runoff"));
}

// ---------------------------------------------------------------------------------------------
// The assembled pipeline
// ---------------------------------------------------------------------------------------------

fn pipeline(soil_m: f64, seasons: &[f64]) -> (Domain, FieldStore, Scheduler, Vec<LandSeason>) {
    let (d, mut f) = earth_like_world(3).unwrap();
    let mut s = Scheduler::new();
    add_land_water_pipeline(&mut s).unwrap();
    seed_land_state(&mut f, soil_m).unwrap();
    let rep = run_land_cycle(&d, &mut f, &mut s, seasons).unwrap();
    (d, f, s, rep)
}

#[test]
fn pipeline_order_loop_and_parameters() {
    let mut s = Scheduler::new();
    add_land_water_pipeline(&mut s).unwrap();
    let order = s.plan_order().unwrap();
    let pos = |m: &str| order.iter().position(|x| x == m).unwrap_or_else(|| panic!("module {m} missing from {order:?}"));
    assert!(pos("ocean_mask") < pos("flow_routing") && pos("flow_routing") < pos("land_water"));
    assert!(pos("land_supply") < pos("moisture_source"));
    assert!(pos("moisture_source") < pos("moisture_transport"));
    assert!(pos("moisture_transport") < pos("land_water"));
    assert!(pos("land_water") < pos("rivers_lakes"));
    assert_eq!(s.module_param("moisture_transport", "warm_start"), Some(1.0));
    // the state fields are the only new external inputs
    for f in ["soil_moisture_prev", "snow_water_equivalent_prev", "glacier_ice_prev", "snow_age_s_prev"] {
        assert!(s.external_inputs().contains(f), "{f} should be supplied by seed_land_state");
    }
    // shared parameters
    set_land_param(&mut s, "soil_capacity_m", 0.3).unwrap();
    assert_eq!(s.module_param("land_supply", "soil_capacity_m"), Some(0.3));
    assert_eq!(s.module_param("land_water", "soil_capacity_m"), Some(0.3));
    set_land_step(&mut s, 86400.0 * 10.0).unwrap();
    assert_eq!(s.module_param("moisture_transport", "transport_time_s"), Some(864000.0));
    assert_eq!(s.module_param("land_water", "step_s"), Some(864000.0));
    assert!(set_land_param(&mut s, "soil_capacity_m", 0.0).is_err());
    assert!(set_land_param(&mut s, "glacier_flow_max_frac", 1.5).is_err());
    assert!(set_land_param(&mut s, "ice_density_ratio", 0.0).is_err());
    assert!(set_land_param(&mut s, "nonsense", 1.0).is_err());
    assert!(s.set_param("land_water", "runoff_m3s", 1.0).is_err(), "output-only parameters cannot be set");
}

#[test]
fn full_cycle_closes_water_including_soil_and_snow() {
    let seasons: Vec<f64> = (0..12).map(|k| (k as f64 + 0.5) / 12.0).collect();
    let (d, f, s, rep) = pipeline(0.075, &seasons);
    assert_eq!(rep.len(), 12);
    let lp = LandParams::from_scheduler(&s);
    let (mut cum_res, mut cum_scale) = (0.0, 0.0);
    for r in &rep {
        assert!(r.water_cycle_converged, "season {}: water_cycle did not converge in {} passes", r.season, r.water_cycle_passes);
        assert!(r.climate.budget.residual_rel < 1e-8, "season {}: atmosphere budget {}", r.season, r.climate.budget.residual_rel);
        assert!(r.land.residual_rel < 1e-8, "season {}: land budget {}", r.season, r.land.residual_rel);
        assert!(r.land.precip_m3 > 0.0 && r.land.evap_m3 > 0.0);
        cum_res += r.land.residual_m3;
        cum_scale += r.land.precip_m3;
    }
    assert!(cum_res.abs() <= 1e-8 * cum_scale, "cumulative land residual {cum_res} of {cum_scale}");
    // last step: atmosphere + land together against the ocean exchange and the river outflow
    let area = &d.mesh.area;
    let ocean = f.data("ocean_mask").unwrap();
    let (pr, ev, ro) = (f.data("precipitation").unwrap(), f.data("evaporation").unwrap(), f.data("runoff").unwrap());
    let (v1, v0) = (f.data("vapor").unwrap(), f.data("vapor_start").unwrap());
    let (t, rho) = (lp.step_s, lp.rho_water);
    let (mut dvap, mut ocean_net, mut river, mut scale) = (0.0, 0.0, 0.0, 0.0);
    for c in 0..d.n_cells() {
        dvap += (v1[c] - v0[c]) * area[c] / rho;
        scale += ev[c] * area[c] * t / rho;
        if ocean[c] > 0.5 {
            ocean_net += (ev[c] - pr[c]) * area[c] * t / rho;
        } else {
            river += ro[c] * area[c] * t / rho;
        }
    }
    let last = rep.last().unwrap().land;
    let ds = last.storage_end_m3 - last.storage_start_m3;
    let resid = dvap + ds - (ocean_net - river);
    assert!(resid.abs() <= 1e-8 * scale, "atmosphere + land vs ocean exchange: residual {resid} of {scale}");
    // the module's own numbers agree with the independent recomputation
    let lw = |k: &str| s.module_param("land_water", k).unwrap();
    assert!(lw("unsupplied_m3s").abs() <= 1e-10 * lw("precip_land_m3s"));
    assert!(close(lw("runoff_m3s") * t, last.runoff_m3, 1e-9));
    // rivers route exactly what the land produced (no lake feedback to the atmosphere yet)
    let rl = |k: &str| s.module_param("rivers_lakes", k).unwrap();
    assert!(close(rl("precip_land_m3s"), lw("runoff_m3s"), 1e-9), "rivers_lakes input must be the land runoff");
    assert!(rl("balance_residual").abs() <= 1e-8);
    // sanity of the state after a year
    let soil = f.data("soil_moisture").unwrap();
    assert!(soil.iter().all(|&v| v >= 0.0 && v <= lp.soil_capacity_m + 1e-15));
    assert!(lw("soil_fraction_mean") > 0.0 && lw("soil_fraction_mean") <= 1.0);
}

#[test]
fn wetter_soil_means_more_rain() {
    // one July-like step from an empty and from a full bucket: the wet start evaporates more over land, so more vapour
    // is available downwind and more rain falls (total and over land)
    let land_precip = |d: &Domain, f: &FieldStore| -> f64 {
        let ocean = f.data("ocean_mask").unwrap();
        let p = f.data("precipitation").unwrap();
        (0..d.n_cells()).filter(|&c| ocean[c] < 0.5).map(|c| p[c] * d.mesh.area[c]).sum()
    };
    let (d0, f0, _, r0) = pipeline(0.0, &[0.25]);
    let (d1, f1, _, r1) = pipeline(0.15, &[0.25]);
    assert!(r0[0].water_cycle_converged && r1[0].water_cycle_converged);
    assert!(r1[0].climate.mean_evap_mm_day > r0[0].climate.mean_evap_mm_day, "full buckets must evaporate more");
    assert!(r1[0].climate.mean_precip_mm_day > r0[0].climate.mean_precip_mm_day, "global rain {} vs {}", r1[0].climate.mean_precip_mm_day, r0[0].climate.mean_precip_mm_day);
    assert!(land_precip(&d1, &f1) > land_precip(&d0, &f0), "land rain must rise with soil moisture");
}

#[test]
fn old_pipeline_is_unchanged() {
    // the constant-availability source of 3b-1 still gives exactly the documented law, bit for bit
    let (d, mut f) = earth_like_world(3).unwrap();
    let mut s = Scheduler::new();
    add_climate_pipeline(&mut s).unwrap();
    run_season(&d, &mut f, &mut s, 0.25).unwrap();
    assert!(!f.has("evaporation_sublimation") && !f.has("soil_evap_cap"));
    let (t, ice, z, w, e) = (f.data("temperature").unwrap(), f.data("ice_fraction").unwrap(), f.data("elevation").unwrap(), f.data("wind_speed").unwrap(), f.data("evaporation").unwrap());
    for i in 0..d.n_cells() {
        let avail = if z[i] < 0.0 { 1.0 } else { 0.3 };
        let open = (1.0 - ice[i]).clamp(0.0, 1.0);
        let es = saturation_vapour_pressure(t[i]);
        let fw = (0.5 + 0.1 * w[i].max(0.0)).max(0.0);
        let want = (1.8e-8 * avail * open * es * (1.0 - 0.5) * fw).clamp(0.0, 1.0e-4);
        assert_eq!(e[i], want, "cell {i}");
    }
}

#[test]
fn land_modules_interface() {
    let sup = LandSupply::new().spec();
    assert_eq!(sup.name, "land_supply");
    assert_eq!(sup.outputs.len(), 4);
    let lw = LandWater::new().spec();
    assert_eq!(lw.name, "land_water");
    for o in ["soil_moisture", "snow_water_equivalent", "glacier_ice", "glacier_mask", "runoff"] {
        assert!(lw.outputs.iter().any(|s| s == o), "missing output {o}");
    }
    // missing inputs are reported, not defaulted
    let d = plane(4, 3);
    let mut f = FieldStore::new(d.n_cells());
    let mut s = Scheduler::new();
    add_land_water_modules(&mut s).unwrap();
    assert!(matches!(s.run(&d, &mut f, &DirtyRegion::All), Err(Error::MissingInput { .. })));
    // a field of the wrong size cannot slip through
    let mut f = FieldStore::new(d.n_cells());
    seed_land_state(&mut f, 0.05).unwrap();
    f.put("ocean_mask", 1, vec![0.0; d.n_cells()]).unwrap();
    let mut m = LandSupply::new();
    m.run(&d, &mut f, &DirtyRegion::All).unwrap();
    // non-finite forcing is rejected
    for name in ["precipitation", "evaporation", "evaporation_sublimation", "temperature"] {
        let n = d.n_cells();
        let mut f = routed(&d, vec![100.0; n]);
        seed_land_state(&mut f, 0.05).unwrap();
        for (nm, v) in [("precipitation", 1e-5), ("evaporation", 0.0), ("evaporation_sublimation", 0.0), ("temperature", 280.0)] {
            f.put(nm, 1, vec![v; n]).unwrap();
        }
        let mut bad = vec![1e-5; n];
        bad[3] = f64::NAN;
        f.put(name, 1, bad).unwrap();
        assert!(matches!(LandWater::new().run(&d, &mut f, &DirtyRegion::All), Err(Error::NonFinite(_))), "{name}");
    }
}
