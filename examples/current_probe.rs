//! Part 4a-2e probe: run the REAL climate pipeline (Earth-like test world) into the ocean current stage and print the numbers
//! `tests/part4.rs` deliberately does not assert (the wind stress of the real atmosphere is not covered by the numpy mirror).
//!   cargo run --release --example current_probe -- [level=4]
use enbuilder::budget::{add_climate_pipeline, run_season};
use enbuilder::ocean_validation::{coast_hops, seam_spikes, pentagon_cells, speed_report};
use enbuilder::*;

fn main() {
    let level: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(4);
    let (d, mut f) = enbuilder::testworld::earth_like_world(level).unwrap();
    let mut s = Scheduler::new();
    add_climate_pipeline(&mut s).unwrap();
    add_ocean_stage(&mut s).unwrap();
    add_gyre_forcing_stage(&mut s).unwrap();
    add_ocean_currents_stage(&mut s).unwrap();
    run_season(&d, &mut f, &mut s, 0.0).unwrap();
    let n = d.n_cells();
    let ocean: Vec<bool> = f.data("ocean_mask").unwrap().iter().map(|&m| m > 0.5).collect();
    let sp = f.data("current_speed").unwrap();
    let rep = speed_report(&d, &ocean, sp, 10.0, 60.0);
    println!("level {level}: {n} cells, ocean fraction {:.3}", (0..n).filter(|&i| ocean[i]).map(|i| d.mesh.area[i]).sum::<f64>() / d.mesh.total_area());
    println!("boundary peak {:.3} m/s (n {}), interior median {:.3} m/s (n {})", rep.boundary_peak, rep.n_boundary, rep.interior_median, rep.n_interior);
    let hops = coast_hops(&d, &ocean);
    println!("deepest cell is {} hops from the coast", hops.iter().filter(|&&h| h != u32::MAX).max().unwrap_or(&0));
    let sr = seam_spikes(&d, &ocean, sp, &pentagon_cells(&d), 0.2, 0.1);
    println!("pentagons: worst spike {:.3}, evaluable {}/{}", sr.worst, sr.evaluable, sr.candidates);
    for name in ["unanchored_cells", "wall_pinned_cells", "clamped_cells", "max_speed_raw", "solver_kind", "cg_iterations", "under_resolved"] {
        println!("  ocean_currents.{name} = {:?}", s.module_param("ocean_currents", name));
    }
}
