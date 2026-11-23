//! Part 5b acceptance tests: the vegetation feedback loop and the three hooks it drives.
//! NOT compiled when written (no Rust toolchain in the sandbox). Hand numbers come from `reference/mirror_5b.py`.
use enbuilder::biomes::{KELVIN, SEASON_YEAR_S};
use enbuilder::generators::*;
use enbuilder::testworld::earth_like_world;
use enbuilder::vegetation::*;
use enbuilder::wind::add_canopy_friction;
use enbuilder::*;

fn plane(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn near(a: f64, b: f64, tol: f64) -> bool { (a - b).abs() <= tol }

// ---------------------------------------------------------------------------------------------
// Toy climate: cell-local, depends on the surface state, so the loop can be checked by hand.
//   T_season = [24, 28, 24, 28] - k_alb (veg_albedo - 0.22) + 273.15      (a brighter surface is cooler)
//   P_annual = (100 + 60 i) (1 + g_evap (veg_evap_factor - 1))            (transpiration recycles rain)
// Cells with i % 8 == 7 are ocean. Same formulas as `toy_climate` in reference/mirror_5b.py.
// ---------------------------------------------------------------------------------------------
fn toy(k_alb: f64, g_evap: f64) -> impl FnMut(&Domain, &mut FieldStore) -> Result<()> {
    move |d: &Domain, f: &mut FieldStore| {
        let n = d.n_cells();
        let alb = f.data("veg_albedo")?.to_vec();
        let ev = f.data("veg_evap_factor")?.to_vec();
        let (mut t, mut p) = (vec![0.0; n * 4], vec![0.0; n * 4]);
        for i in 0..n {
            let pann = (100.0 + 60.0 * i as f64) * (1.0 + g_evap * (ev[i] - 1.0));
            for (k, b) in [24.0, 28.0, 24.0, 28.0].iter().enumerate() {
                t[i * 4 + k] = b - k_alb * (alb[i] - 0.22) + KELVIN;
                p[i * 4 + k] = pann / SEASON_YEAR_S;
            }
        }
        f.put("season_temperature", 4, t)?;
        f.put("season_precipitation", 4, p)?;
        f.put("season_soil_moisture", 4, vec![0.05; n * 4])?;
        f.put("ocean_mask", 1, (0..n).map(|i| if i % 8 == 7 { 1.0 } else { 0.0 }).collect())?;
        f.put("glacier_mask", 1, vec![0.0; n])
    }
}

fn run_toy(k_alb: f64, g_evap: f64, p: &VegetationParams) -> (Domain, FieldStore, VegetationReport) {
    let d = plane(8, 4);
    let mut f = FieldStore::new(d.n_cells());
    let table = BiomeTable::earth_like();
    let mut c = toy(k_alb, g_evap);
    let rep = run_vegetation_loop(&d, &mut f, &table, p, &mut c).unwrap();
    (d, f, rep)
}

fn count(f: &FieldStore, table: &BiomeTable, name: &str) -> usize {
    let id = table.id_of(name).unwrap() as f64;
    f.data("biome").unwrap().iter().filter(|&&v| v == id).count()
}

#[test]
fn parameters_validate_and_relaxation_decays_geometrically() {
    let p = VegetationParams::default();
    assert!(p.validate().is_ok());
    assert_eq!((p.relax, p.relax_decay, p.max_iters), (0.5, 0.7, 12));
    assert!(near(p.relax_at(0), 0.5, 1e-15) && near(p.relax_at(1), 0.35, 1e-15) && near(p.relax_at(2), 0.245, 1e-15));
    for bad in [
        VegetationParams { relax: 0.0, ..p },
        VegetationParams { relax: 1.2, ..p },
        VegetationParams { relax_decay: 0.0, ..p },
        VegetationParams { max_iters: 0, ..p },
        VegetationParams { tol_albedo: 0.0, ..p },
        VegetationParams { baseline_albedo: 1.5, ..p },
        VegetationParams { baseline_evap: -1.0, ..p },
    ] {
        assert!(matches!(bad.validate(), Err(Error::InvalidParam(_))));
    }
}

#[test]
fn seeding_creates_the_baseline_and_keeps_a_warm_start() {
    let mut f = FieldStore::new(6);
    let p = VegetationParams::default();
    seed_vegetation(&mut f, &p).unwrap();
    assert!(f.data("veg_albedo").unwrap().iter().all(|&v| v == 0.22));
    assert!(f.data("veg_evap_factor").unwrap().iter().all(|&v| v == 1.0));
    assert!(f.data("veg_roughness_m").unwrap().iter().all(|&v| v == 0.0));
    f.put("veg_albedo", 1, vec![0.4; 6]).unwrap();
    seed_vegetation(&mut f, &p).unwrap();
    assert!(f.data("veg_albedo").unwrap().iter().all(|&v| v == 0.4), "an existing field is a warm start");
    clear_vegetation(&mut f, &p).unwrap();
    assert!(f.data("veg_albedo").unwrap().iter().all(|&v| v == 0.22));
    assert!(land_surface_means(&plane(2, 3), &FieldStore::new(6)).unwrap().is_none());
}

#[test]
fn the_loop_converges_on_the_toy_climate_with_the_hand_numbers() {
    let p = VegetationParams::default();
    let (_d, f, rep) = run_toy(20.0, 0.5, &p);
    assert!(rep.converged, "vegetation feedback loop must converge");
    assert_eq!(rep.passes(), 8, "mirror: 8 passes");
    let it = &rep.iterations;
    // pass 0: bare-land climate -> every land cell relaxes half way to its biome
    assert_eq!(it[0].changed_frac, 1.0);
    assert!(near(it[0].d_albedo_max, 0.065, 1e-9), "{}", it[0].d_albedo_max); // 0.5 * (0.35 - 0.22)
    assert!(near(it[0].d_evap_max, 0.45, 1e-9)); // 0.5 * (1.0 - 0.1)
    assert!(near(it[0].d_rough_max, 15.0, 1e-9)); // 0.5 * 30 m rainforest canopy
    assert!(near(it[1].changed_frac, 2.0 / 28.0, 1e-12), "two of the 28 land cells change biome in pass 1");
    for (k, x) in it.iter().enumerate() {
        assert!(near(x.relax, 0.5 * 0.7f64.powi(k as i32), 1e-12));
    }
    let last = rep.last();
    assert!(last.d_albedo_max <= p.tol_albedo && last.d_evap_max <= p.tol_evap && last.d_rough_max <= p.tol_rough_m);
    assert!(near(last.d_albedo_max, 0.00297, 5e-4) && near(last.d_rough_max, 0.3224, 0.05));
    // the step sizes shrink overall (pass 6 is allowed to bump up: one cell flipped biome)
    assert!(it[7].d_albedo_max < it[0].d_albedo_max / 10.0);
    // final state of the driest land cell (cell 0, a hot desert)
    let (a, e, r) = (f.data("veg_albedo").unwrap()[0], f.data("veg_evap_factor").unwrap()[0], f.data("veg_roughness_m").unwrap()[0]);
    assert!(near(a, 0.3308, 3e-3) && near(e, 0.2331, 5e-3) && near(r, 0.0426, 0.02), "{a} {e} {r}");
    assert!(near(last.land_albedo_mean, 0.21163, 2e-3), "{}", last.land_albedo_mean);
    assert!(rep.summary.land_area_m2 > 0.0);
}

#[test]
fn ocean_cells_keep_the_baseline_and_land_cells_leave_it() {
    let p = VegetationParams::default();
    let (d, f, _rep) = run_toy(20.0, 0.5, &p);
    let (a, e, r) = (f.data("veg_albedo").unwrap(), f.data("veg_evap_factor").unwrap(), f.data("veg_roughness_m").unwrap());
    for i in 0..d.n_cells() {
        if i % 8 == 7 {
            assert!(a[i] == 0.22 && e[i] == 1.0 && r[i] == 0.0, "ocean cell {i} must not be touched");
        } else {
            assert!(a[i] != 0.22 || e[i] != 1.0 || r[i] != 0.0, "land cell {i} should have moved");
        }
        assert!(a[i].is_finite() && (0.0..=1.0).contains(&a[i]) && e[i] >= 0.0 && r[i] >= 0.0);
    }
}

#[test]
fn the_feedback_changes_the_map_and_is_off_when_the_coupling_is_off() {
    let table = BiomeTable::earth_like();
    let p = VegetationParams::default();
    let (_d, f_on, rep_on) = run_toy(20.0, 0.5, &p);
    let (_d2, f_off, rep_off) = run_toy(0.0, 0.0, &p);
    assert!(rep_on.converged && rep_off.converged);
    assert_eq!(rep_off.passes(), 6, "mirror: 6 passes with no coupling");
    // a desert loses its transpiration, so it gets drier and the desert grows: 6 cells with the feedback, 4 without
    assert_eq!(count(&f_on, &table, "hot_desert"), 6);
    assert_eq!(count(&f_off, &table, "hot_desert"), 4);
    // the driest cell: bare 100 mm/yr, vegetated about 62 mm/yr, and about 2.2 K cooler from the brighter surface
    let ev = f_on.data("veg_evap_factor").unwrap()[0];
    let al = f_on.data("veg_albedo").unwrap()[0];
    assert!(near(100.0 * (1.0 + 0.5 * (ev - 1.0)), 61.7, 0.7));
    assert!(near(20.0 * (al - 0.22), 2.22, 0.1));
}

#[test]
fn running_out_of_passes_is_reported_not_an_error() {
    let p = VegetationParams { max_iters: 2, ..VegetationParams::default() };
    let (_d, f, rep) = run_toy(20.0, 0.5, &p);
    assert!(!rep.converged);
    assert_eq!(rep.passes(), 2);
    assert!(near(rep.iterations[1].d_albedo_max, 0.035, 1e-9));
    assert!(f.data("veg_albedo").unwrap().iter().all(|v| v.is_finite()));
}

#[test]
fn a_first_pass_fixed_point_converges_in_one_pass() {
    // every biome effect equals the baseline, so nothing moves
    let table = BiomeTable::parse("biome everything color=#808080 albedo=0.22 evap=1 rough=0\n").unwrap();
    let d = plane(4, 4);
    let mut f = FieldStore::new(d.n_cells());
    let mut c = |dom: &Domain, fs: &mut FieldStore| -> Result<()> {
        let n = dom.n_cells();
        fs.put("season_temperature", 2, vec![290.0; n * 2])?;
        fs.put("season_precipitation", 2, vec![3e-5; n * 2])?;
        fs.put("season_soil_moisture", 2, vec![0.05; n * 2])?;
        fs.put("ocean_mask", 1, vec![0.0; n])?;
        fs.put("glacier_mask", 1, vec![0.0; n])
    };
    let rep = run_vegetation_loop(&d, &mut f, &table, &VegetationParams::default(), &mut c).unwrap();
    assert!(rep.converged && rep.passes() == 1);
    assert!(rep.last().d_albedo_max == 0.0 && rep.last().d_evap_max == 0.0 && rep.last().d_rough_max == 0.0);
}

#[test]
fn errors_from_the_climate_and_a_missing_input_propagate() {
    let d = plane(4, 4);
    let table = BiomeTable::earth_like();
    let mut f = FieldStore::new(d.n_cells());
    let mut boom = |_: &Domain, _: &mut FieldStore| -> Result<()> { Err(Error::InvalidParam("boom".into())) };
    match run_vegetation_loop(&d, &mut f, &table, &VegetationParams::default(), &mut boom) {
        Err(Error::InvalidParam(m)) => assert_eq!(m, "boom"),
        other => panic!("expected the climate error, got {other:?}"),
    }
    // a climate that forgets the seasonal fields: the scheduler reports the missing input
    let mut nothing = |_: &Domain, _: &mut FieldStore| -> Result<()> { Ok(()) };
    assert!(matches!(run_vegetation_loop(&d, &mut f, &table, &VegetationParams::default(), &mut nothing), Err(Error::MissingInput { .. })));
    // bad parameters are refused before anything runs
    let bad = VegetationParams { relax: 0.0, ..VegetationParams::default() };
    assert!(run_vegetation_loop(&d, &mut f, &table, &bad, &mut nothing).is_err());
}

#[test]
fn the_loop_is_deterministic() {
    let run = || {
        let (_d, f, rep) = run_toy(20.0, 0.5, &VegetationParams::default());
        (f.data("veg_albedo").unwrap().to_vec(), f.data("biome").unwrap().to_vec(), rep.iterations.len())
    };
    assert_eq!(run(), run());
}

#[test]
fn editing_the_biome_table_changes_the_vegetated_climate_with_no_code_change() {
    let d = plane(8, 4);
    let base = BiomeTable::earth_like();
    let run = |t: &BiomeTable| {
        let mut f = FieldStore::new(d.n_cells());
        let mut c = toy(20.0, 0.5);
        run_vegetation_loop(&d, &mut f, t, &VegetationParams::default(), &mut c).unwrap();
        f.data("veg_albedo").unwrap().to_vec()
    };
    let a0 = run(&base);
    let dark = BiomeTable::parse(&base.to_text().replace("albedo=0.35", "albedo=0.12")).unwrap();
    let a1 = run(&dark);
    assert_ne!(a0, a1);
    assert!(a1[0] < 0.22, "a dark desert lowers the surface albedo of cell 0: {}", a1[0]);
}

// ---------------------------------------------------------------------------------------------
// The three hooks
// ---------------------------------------------------------------------------------------------

#[test]
fn albedo_reads_veg_albedo_on_land_only_and_is_unchanged_without_it() {
    let d = plane(4, 4);
    let n = d.n_cells();
    let elev: Vec<f64> = (0..n).map(|i| if i < 8 { -100.0 } else { 100.0 }).collect();
    let mk = || {
        let mut f = FieldStore::new(n);
        f.put("temperature", 1, vec![300.0; n]).unwrap();
        f.put("elevation", 1, elev.clone()).unwrap();
        f
    };
    let run = |f: &mut FieldStore| {
        let mut s = Scheduler::new();
        s.register(Box::new(AlbedoModel::new())).unwrap();
        s.run(&d, f, &DirtyRegion::All).unwrap();
        f.data("albedo").unwrap().to_vec()
    };
    let mut f0 = mk();
    let a0 = run(&mut f0);
    // 0.22 land and 0.06 water, then the 0.2 cloud layer: a + (1 - a) 0.2
    assert!(near(a0[10], 0.22 + 0.78 * 0.2, 1e-12) && near(a0[0], 0.06 + 0.94 * 0.2, 1e-12));
    let mut f1 = mk();
    f1.put("veg_albedo", 1, vec![0.5; n]).unwrap();
    let a1 = run(&mut f1);
    assert!(near(a1[10], 0.5 + 0.5 * 0.2, 1e-12), "land takes the vegetation albedo");
    assert!(near(a1[0], a0[0], 1e-15), "water ignores it");
    // a field with the wrong layer count is an error, not a guess
    let mut f2 = mk();
    f2.put("veg_albedo", 2, vec![0.5; 2 * n]).unwrap();
    let mut s = Scheduler::new();
    s.register(Box::new(AlbedoModel::new())).unwrap();
    assert!(matches!(s.run(&d, &mut f2, &DirtyRegion::All), Err(Error::InvalidParam(_))));
}

#[test]
fn land_supply_scales_the_availability_and_leaves_the_stores_alone() {
    let d = plane(4, 4);
    let n = d.n_cells();
    let mk = |veg: Option<Vec<f64>>| {
        let mut f = FieldStore::new(n);
        f.put("ocean_mask", 1, (0..n).map(|i| if i < 4 { 1.0 } else { 0.0 }).collect()).unwrap();
        f.put("soil_moisture_prev", 1, vec![0.075; n]).unwrap(); // half of the 0.15 m capacity
        f.put("snow_water_equivalent_prev", 1, vec![0.0; n]).unwrap();
        f.put("glacier_ice_prev", 1, vec![0.0; n]).unwrap();
        if let Some(v) = veg {
            f.put("veg_evap_factor", 1, v).unwrap();
        }
        let mut s = Scheduler::new();
        s.register(Box::new(LandSupply::new())).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        (f.data("land_availability").unwrap().to_vec(), f.data("soil_evap_cap").unwrap().to_vec())
    };
    let (a0, c0) = mk(None);
    assert!(a0[8] == 0.5 && a0[0] == 1.0);
    let factors: Vec<f64> = (0..n).map(|i| match i % 4 { 0 => 1.5, 1 => 0.1, 2 => 4.0, _ => 1.0 }).collect();
    let (a1, c1) = mk(Some(factors));
    assert!(near(a1[8], 0.75, 1e-12), "1.5 x 0.5"); // i = 8, i % 4 = 0
    assert!(near(a1[9], 0.05, 1e-12), "0.1 x 0.5");
    assert_eq!(a1[10], 1.0, "clamped at 1");
    assert_eq!(a1[11], 0.5);
    assert_eq!(a1[0], 1.0, "ocean stays 1");
    assert_eq!(c0, c1, "the soil store cap is not scaled, so water stays conserved");
}

#[test]
fn canopy_friction_raises_land_friction_only() {
    let z = [-10.0, 5.0, 5.0, 5.0];
    let mut r = vec![1e-4, 3e-5, 3e-5, 3e-5];
    add_canopy_friction(&mut r, &z, 0.0, &[30.0, 30.0, 0.0, -4.0], 3e-5, 500.0);
    assert_eq!(r[0], 1e-4, "ocean untouched");
    assert!(near(r[1], 3e-5 * (1.0 + 30.0 / 500.0), 1e-18), "a 30 m forest adds 6%");
    assert_eq!(r[2], 3e-5);
    assert_eq!(r[3], 3e-5, "negative canopy counts as 0");
}

// ---------------------------------------------------------------------------------------------
// Real pipeline smoke test (thresholds loose; numbers printed for the hand-off)
// ---------------------------------------------------------------------------------------------

#[test]
fn the_real_pipeline_runs_the_vegetation_loop() {
    let (d, mut f) = earth_like_world(3).unwrap();
    let mut s = Scheduler::new();
    add_land_water_pipeline(&mut s).unwrap();
    seed_land_state(&mut f, 0.075).unwrap();
    let seasons: Vec<f64> = (0..12).map(|k| (k as f64 + 0.5) / 12.0).collect();
    let p = VegetationParams { max_iters: 5, ..VegetationParams::default() };
    let table = BiomeTable::earth_like();
    let rep = {
        let mut climate = seasonal_climate(&mut s, &seasons, 1);
        run_vegetation_loop(&d, &mut f, &table, &p, &mut climate).unwrap()
    };
    assert!(rep.passes() >= 2);
    assert!(f.data("season_temperature").unwrap().iter().all(|&t| t.is_finite() && t > 150.0 && t < 400.0));
    for name in VEG_FIELDS {
        assert!(f.data(name).unwrap().iter().all(|v| v.is_finite() && *v >= 0.0), "{name}");
    }
    assert!(f.data("veg_albedo").unwrap().iter().all(|&a| a <= 1.0));
    let (alb, evap, rough) = land_surface_means(&d, &f).unwrap().unwrap();
    assert!(alb != 0.22 && alb > 0.05 && alb < 0.7, "land albedo mean moved off the baseline: {alb}");
    eprintln!("5b smoke (L3, 12 seasons, {} passes, converged {}): land mean albedo {alb:.4} evap factor {evap:.3} canopy {rough:.2} m", rep.passes(), rep.converged);
    for x in &rep.iterations {
        eprintln!("  pass {} relax {:.3} changed {:.3} d_alb {:.4} d_evap {:.4} d_rough {:.3}", x.iter, x.relax, x.changed_frac, x.d_albedo_max, x.d_evap_max, x.d_rough_max);
    }
}
