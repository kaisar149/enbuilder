//! Part 7a: the high-gravity example. `cargo run --release --example gravity_probe -- <level>`
//! One anomaly (gravity x2.5, Gaussian) on the Earth-like world; prints what the UNMODIFIED engine does with it.
use enbuilder::anomaly::*;
use enbuilder::budget::{add_climate_pipeline, run_season};
use enbuilder::testworld::earth_like_world;
use enbuilder::{DirtyRegion, FieldStore, Scheduler, Tectonics, TerrainShaping};

fn main() {
    let level: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let (base, _f0) = earth_like_world(level).unwrap();
    let ax = [0.3f64, 0.5, 0.8];
    let al = (ax[0] * ax[0] + ax[1] * ax[1] + ax[2] * ax[2]).sqrt();
    let c = [ax[0] / al * 6.371e6, ax[1] / al * 6.371e6, ax[2] / al * 6.371e6];
    let make = |mute: bool| {
        let mut s = AnomalySet::new();
        s.add(Anomaly::new("heavy", Shape::Sphere { center: c, radius_m: 900_000.0 }, Falloff::Gaussian, std::env::args().nth(4).and_then(|s| s.parse().ok()).unwrap_or(700_000.0)).with_effect(Effect::multiply("gravity", std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(2.5))).muted(mute)).unwrap();
        Anomalies::from_set(s)
    };
    let dist = Shape::Sphere { center: c, radius_m: 900_000.0 }.distances(&base).unwrap();
    let inside: Vec<usize> = (0..base.n_cells()).filter(|&i| dist[i] == 0.0).collect();
    let ring: Vec<usize> = (0..base.n_cells()).filter(|&i| dist[i] > 0.0 && dist[i] < 1_000_000.0).collect();
    println!("cells {}  inside {}  ring {}", base.n_cells(), inside.len(), ring.len());
    let mean = |v: &[f64], idx: &[usize]| idx.iter().map(|&i| v[i]).sum::<f64>() / idx.len() as f64;

    for (label, mute) in [("baseline", true), ("heavy", false)] {
        let h = make(mute);
        let dom = h.effective_domain(&base).unwrap();
        let (_, mut f) = earth_like_world(level).unwrap();
        let mut s = Scheduler::new();
        add_climate_pipeline(&mut s).unwrap();
        let pe: f64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(0.25);
        s.set_param("pressure", "gravity_pressure_exp", pe).unwrap();
        run_season(&dom, &mut f, &mut s, 0.0).unwrap();
        let psl = f.data("pressure_sl").unwrap();
        let ps = f.data("pressure").unwrap();
        let div = f.data("wind_divergence").unwrap();
        let ws = f.data("wind_speed").unwrap();
        let pr = f.data("precipitation").unwrap();
        println!("[{label}] p_sl in {:.0} ring {:.0} | p_surf in {:.0} | wind in {:.2} ring {:.2} m/s | div in {:.3e} ring {:.3e} | precip in {:.3e} ring {:.3e}",
            mean(psl, &inside), mean(psl, &ring), mean(ps, &inside), mean(ws, &inside), mean(ws, &ring), mean(div, &inside), mean(div, &ring), mean(pr, &inside), mean(pr, &ring));
    }
    // relief: tectonics + terrain
    for (label, mute) in [("baseline", true), ("heavy", false)] {
        let h = make(mute);
        let dom = h.effective_domain(&base).unwrap();
        let mut f = FieldStore::new(base.n_cells());
        let mut s = Scheduler::new();
        s.register(Box::new(Tectonics::new())).unwrap();
        s.register(Box::new(TerrainShaping::new())).unwrap();
        s.run(&dom, &mut f, &DirtyRegion::All).unwrap();
        let z = f.data("elevation").unwrap();
        let slope = f.data("slope").unwrap();
        let zmax = inside.iter().map(|&i| z[i]).fold(f64::MIN, f64::max);
        let rel = { let m = mean(z, &inside); (inside.iter().map(|&i| (z[i] - m).powi(2)).sum::<f64>() / inside.len() as f64).sqrt() };
        println!("[{label}] terrain inside: max {:.0} m  std {:.0} m  mean slope {:.4}", zmax, rel, mean(slope, &inside));
    }
}
