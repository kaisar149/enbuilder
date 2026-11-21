//! Part 4c-3 diagnostic: WHERE does the `landscape` loop fail to converge?
//! `geo.run` without a commit just continues the same fixed-point iteration from the current elevation, so calling it repeatedly shows how the
//! residual evolves and which cells carry it. Prints, per call: loop passes/residual, the largest |change of elevation| per pass-block, its cell and
//! class, and how many cells above 1e-4 * max|elevation| are depression cells, lake cells, changed receiver, river cells.
//!
//!   cargo run --release --example landscape_diag -- [cube|plane|wrap] [k_calls=6]
use enbuilder::generators::*;
use enbuilder::math::{length, sub};
use enbuilder::rng::Rng;
use enbuilder::*;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let which = a.get(1).map(String::as_str).unwrap_or("cube").to_string();
    let calls: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
    let (nx, ny) = (24usize, 12usize);
    let (d, z) = if which == "cube" {
        let d = generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap();
        let n = d.n_cells();
        let c = &d.mesh.centroid;
        let mut mid = [0.0; 3];
        for p in c { for k in 0..3 { mid[k] += p[k]; } }
        for k in 0..3 { mid[k] /= n as f64; }
        let ext = c.iter().map(|p| length(sub(*p, mid))).fold(1.0, f64::max);
        let mut rng = Rng::new(11);
        let z: Vec<f64> = (0..n).map(|i| 800.0 * ((c[i][0] - mid[0]) / ext) - 150.0 + 300.0 * rng.next_f64()).collect();
        (d, z)
    } else {
        let b = if which == "wrap" { BoundaryKind::Wrap } else { BoundaryKind::Wall };
        let d = generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: b, surface_gravity: 9.81, seed: 1 }).unwrap();
        let mut rng = Rng::new(3);
        let mut z = vec![0.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let x = 2.0 * std::f64::consts::PI * i as f64 / nx as f64;
                z[j * nx + i] = 600.0 * x.cos() - 200.0 + 250.0 * rng.next_f64() + 20.0 * j as f64;
            }
        }
        (d, z)
    };
    let n = d.n_cells();
    let runoff = if which == "cube" { 5.0e-5 } else { 5.0e-5 };
    let mut f = FieldStore::new(n);
    f.put("tectonic_elevation", 1, z).unwrap();
    f.put("rock_strength", 1, vec![1.0; n]).unwrap();
    f.put("runoff", 1, vec![runoff; n]).unwrap();
    f.put("evaporation", 1, vec![0.5 * runoff; n]).unwrap();
    seed_landscape(&d, &mut f).unwrap();
    let mut s = Scheduler::new();
    add_geology_stage(&mut s).unwrap();
    s.set_param("erosion", "K", 2.0e-6).unwrap();
    set_surface_years_pub(&mut s);

    let mut prev_z = f.data("elevation").unwrap().to_vec();
    let mut prev_fd: Vec<f64> = vec![-2.0; n];
    for call in 0..calls {
        let rep = s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        let lp = &rep.loops[0];
        let z = f.data("elevation").unwrap().to_vec();
        let fd = f.data("flow_dir").unwrap().to_vec();
        let dep = f.data("depression_depth").unwrap().to_vec();
        let band = f.data("coast_band").unwrap().to_vec();
        let ocean = f.data("ocean_mask").unwrap().to_vec();
        let river = f.data("river_mask").unwrap().to_vec();
        let maxabs = z.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let thr = 1.0e-4 * maxabs;
        let (mut worst, mut wc) = (0.0f64, 0usize);
        let (mut n_over, mut n_dep, mut n_fd, mut n_riv, mut n_band, mut n_ocean) = (0, 0, 0, 0, 0, 0);
        for c in 0..n {
            let dz = (z[c] - prev_z[c]).abs();
            if band[c] > 0.5 { n_band += 1; }
            if ocean[c] > 0.5 { n_ocean += 1; }
            if band[c] > 0.5 { continue; }
            if dz > worst { worst = dz; wc = c; }
            if dz > thr {
                n_over += 1;
                if dep[c] > 0.0 { n_dep += 1; }
                if fd[c] != prev_fd[c] { n_fd += 1; }
                if river[c] > 0.5 { n_riv += 1; }
            }
        }
        println!(
            "call {call}: passes {} residual {:.2e} converged {} | cells over thr ({thr:.3} m): {n_over} [depression {n_dep}, receiver changed {n_fd}, river {n_riv}] | band {n_band}, ocean {n_ocean} | worst |dz| {worst:.3} m at cell {wc} (z {:.1}, dep {:.2}, fd {} -> {}, river {})",
            lp.iterations, lp.residual, lp.converged, z[wc], dep[wc], prev_fd[wc], fd[wc], river[wc]
        );
        prev_z = z;
        prev_fd = fd;
    }
}

fn set_surface_years_pub(s: &mut Scheduler) { set_surface_years(s, 2.0e4).unwrap(); }
