//! Part 3c-5 benchmark: steady-state stepped wind, CPU f64 vs the f32 kernels (emulated on the CPU) vs GPU.
//!   cargo run --release --example bench_wind                     (CPU f64 + f32 emulation)
//!   cargo run --release --features gpu --example bench_wind      (adds the GPU rows)
//!   cargo run --release --example bench_wind -- 5 6 7            (levels; default 5 6 7)
//! Each backend is run twice on the same scheduler: "first" includes one-time costs (adapter, pipeline
//! creation), "repeat" is the steady state a season loop sees. Columns: wall time of the whole `pressure + wind`
//! scheduler run, time inside the wind solve, the final device->host copy of the wind (the once-per-season
//! readback; 0 for CPU backends), sub-steps, passes (steps / steps_per_pass; CPU f64 has none), max residual
//! (max |du| per step at the stop, m/s) and the max relative difference of the wind vs the CPU f64 result.
use enbuilder::backend::max_rel_error;
use enbuilder::generators::*;
use enbuilder::*;
use std::time::Instant;

fn earth_t(u: [f64; 3]) -> f64 { 300.0 - 40.0 * u[2] * u[2] }
fn ridge(u: [f64; 3]) -> f64 { -3000.0 + 6000.0 * (-(u[1] / 0.12).powi(2)).exp() }

struct Row { wall: f64, solve: f64, copy: f64, steps: f64, resid: f64, wind: Vec<f64> }

fn once(d: &Domain, s: &mut Scheduler) -> Result<Row> {
    let mut f = FieldStore::new(d.n_cells());
    let t: Vec<f64> = d.mesh.up.iter().map(|&u| earth_t(u)).collect();
    f.put("temperature_sl", 1, t.clone())?;
    f.put("temperature", 1, t)?;
    f.put("elevation", 1, d.mesh.up.iter().map(|&u| ridge(u)).collect())?;
    let t0 = Instant::now();
    s.run(d, &mut f, &DirtyRegion::All)?;
    let wall = t0.elapsed().as_secs_f64();
    let g = |k: &str| s.module_param("wind", k).unwrap_or(f64::NAN);
    Ok(Row { wall, solve: g("last_solve_s"), copy: g("last_copy_back_s"), steps: g("last_steps"), resid: g("last_residual"), wind: f.data("wind")?.to_vec() })
}

fn main() {
    let levels: Vec<u32> = {
        let v: Vec<u32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
        if v.is_empty() { vec![5, 6, 7] } else { v }
    };
    let mut backends = vec![("cpu-f64", Backend::Cpu), ("cpu-f32-emul", Backend::CpuF32)];
    #[cfg(feature = "gpu")]
    backends.push(("gpu", Backend::Gpu));
    println!("{:>5} {:>8} {:<13} {:<7} {:>9} {:>9} {:>10} {:>7} {:>7} {:>10} {:>10}",
        "level", "cells", "backend", "run", "wall_s", "solve_s", "copy_back", "steps", "passes", "residual", "vs_cpu");
    for &lvl in &levels {
        let d = generate_sphere(&SphereParams { radius: 6.371e6, subdivision: lvl, surface_gravity: 9.81, seed: 7 }).unwrap();
        let mut reference: Option<Vec<f64>> = None;
        for (name, b) in &backends {
            let mut s = Scheduler::new();
            add_atmosphere_stage_stepped_with(&mut s, *b).unwrap();
            let spp = s.module_param("wind", "steps_per_pass").unwrap();
            for run in ["first", "repeat"] {
                match once(&d, &mut s) {
                    Ok(r) => {
                        if matches!(b, Backend::Cpu) && reference.is_none() { reference = Some(r.wind.clone()); }
                        let diff = reference.as_ref().map(|w| max_rel_error(&r.wind, w));
                        let passes = if matches!(b, Backend::Cpu) { "-".to_string() } else { format!("{}", (r.steps / spp) as u64) };
                        println!("{:>5} {:>8} {:<13} {:<7} {:>9.4} {:>9.4} {:>10.2e} {:>7} {:>7} {:>10.2e} {:>10}",
                            lvl, d.n_cells(), name, run, r.wall, r.solve, r.copy, r.steps as u64, passes, r.resid,
                            diff.map_or("-".into(), |x| format!("{x:.1e}")));
                    }
                    Err(Error::NoGpu(m)) => { println!("{lvl:>5} {:>8} {name:<13} SKIPPED (no GPU): {m}", d.n_cells()); break; }
                    Err(e) => { println!("{lvl:>5} {:>8} {name:<13} {run:<7} FAILED: {e:?}", d.n_cells()); break; }
                }
            }
        }
    }
}
