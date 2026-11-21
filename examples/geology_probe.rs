//! Part 4c-3 probe: runs the coupled geology (`run_geology`) on the Earth-like test world and prints everything the Part 4 hand-off needs
//! (passes, budgets, discharge, glaciers, rivers, lakes, erosion depth, wall time).
//!
//!   cargo run --release --example geology_probe -- [level=4] [n_steps=50] [years_per_step=2e4] [climate_every=10]
//!
//! Paste the output into the "Earth-like numbers" table of the Part 4 section of ARCHITECTURE.md.
use enbuilder::testworld::earth_like_world;
use enbuilder::*;
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let arg = |i: usize, d: f64| -> f64 { a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d) };
    let level = arg(1, 4.0) as u32;
    let p = GeologyParams { n_steps: arg(2, 50.0) as usize, years_per_step: arg(3, 2.0e4), climate_every: arg(4, 10.0) as usize, ..GeologyParams::default() };

    let (d, mut f) = earth_like_world(level).unwrap();
    let n = d.n_cells();
    let z0 = f.data("elevation").unwrap().to_vec();
    f.put("tectonic_elevation", 1, z0).unwrap();
    f.put("rock_strength", 1, vec![1.0; n]).unwrap();
    seed_landscape(&d, &mut f).unwrap();
    let mut climate = Scheduler::new();
    add_land_water_pipeline(&mut climate).unwrap();
    let mut geo = Scheduler::new();
    add_geology_stage(&mut geo).unwrap();

    let start_max = f.data("elevation").unwrap().iter().cloned().fold(f64::MIN, f64::max);
    let t0 = Instant::now();
    let rep = run_geology(&d, &mut f, &mut climate, &mut geo, &p).unwrap();
    let secs = t0.elapsed().as_secs_f64();
    let sum = landscape_summary(&d, &f, &geo).unwrap();

    println!("level {level}: {n} cells, {} steps x {:.1e} yr = {:.2e} yr, climate every {} steps, wall {secs:.1} s", p.n_steps, p.years_per_step, rep.years, p.climate_every);
    println!("inner loop: all converged {}, max passes {}, per-step passes {:?}", rep.all_loops_converged, rep.max_passes, rep.steps.iter().map(|s| s.passes).collect::<Vec<_>>());
    println!("budgets: sediment residual {:.2e}, water residual {:.2e}, river balance {:.2e}", rep.max_sediment_residual_rel, rep.max_water_residual_rel, rep.max_river_balance_rel);
    println!("max |drift| of the surface identity: {:.3e} m3 (eroded total {:.3e} m3)", rep.steps.iter().map(|s| s.drift_m3.abs()).fold(0.0, f64::max), rep.eroded_total_m3);
    println!("sediment: eroded {:.3e} m3, deposited on land/deltas {:.3e}, exported {:.3e}", rep.eroded_total_m3, rep.deposited_total_m3, rep.exported_total_m3);
    for c in &rep.climate {
        println!("climate run before step {:3}: runoff residual {:.3e}, elevation residual {:.3e}, precip {:.2} mm/day, water residual {:.1e}", c.step, c.runoff_residual, c.elevation_residual, c.mean_precip_mm_day, c.water_residual_rel);
    }
    println!("coupling converged (both climate residuals under tolerance): {}", rep.converged);
    println!("max elevation {start_max:.0} m -> {:.0} m; mean land elevation now {:.0} m", sum.max_elevation_m, sum.mean_land_elevation_m);
    println!("EARTH-LIKE NUMBERS: ocean discharge {:.3e} m3/s = {:.3} Sv, glaciers {:.1}% of land, river cells {}, lakes {}, depression cells {}, land {:.1}%, non-draining land cells {}", sum.ocean_discharge_m3s, sum.ocean_discharge_m3s / 1.0e6, 100.0 * sum.glacier_fraction_of_land, sum.river_cells, sum.n_lakes, sum.n_depression_cells, 100.0 * sum.land_fraction, sum.land_cells_not_draining);
    let z = f.data("elevation").unwrap();
    let tect = f.data("tectonic_elevation").unwrap();
    let ocean = f.data("ocean_mask").unwrap();
    let (mut max_cut, mut max_fill) = (0.0f64, 0.0f64);
    for c in 0..n {
        if ocean[c] < 0.5 {
            max_cut = max_cut.max(tect[c] - z[c]);
            max_fill = max_fill.max(z[c] - tect[c]);
        }
    }
    println!("land lowered by at most {max_cut:.0} m and raised by at most {max_fill:.0} m relative to the tectonic surface (noise and thermal relaxation included)");
}
