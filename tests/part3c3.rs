//! Part 3c-3 tests: stepped wind kernels (f32) vs the CPU f64 solver of 3c-1.
//! Backend 1 = the kernels emulated on the CPU (always runs); backend 2 = real GPU (feature `gpu`,
//! skips without an adapter).
use enbuilder::backend::max_rel_error;
use enbuilder::generators::*;
use enbuilder::wind_gpu::*;
use enbuilder::*;

const DAY: f64 = 86400.0;

fn sphere(sub: u32) -> Domain {
    generate_sphere(&SphereParams { radius: 6.371e6, subdivision: sub, surface_gravity: 9.81, seed: 7 }).unwrap()
}
fn earth_t(u: [f64; 3]) -> f64 { 300.0 - 40.0 * u[2] * u[2] }
fn ocean(_u: [f64; 3]) -> f64 { -3000.0 }
/// A 3000 m ridge along the x-z great circle on an ocean world.
fn ridge(u: [f64; 3]) -> f64 { -3000.0 + 6000.0 * (-(u[1] / 0.12).powi(2)).exp() }

fn fields(d: &Domain, t: impl Fn([f64; 3]) -> f64, z: impl Fn([f64; 3]) -> f64) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    let tsl: Vec<f64> = d.mesh.up.iter().map(|&u| t(u)).collect();
    f.put("temperature_sl", 1, tsl.clone()).unwrap();
    f.put("temperature", 1, tsl).unwrap();
    f.put("elevation", 1, d.mesh.up.iter().map(|&u| z(u)).collect()).unwrap();
    f
}

#[derive(Clone, Copy)]
enum Case { Ocean, Ridge, Tidal }
fn mk(d: &Domain, c: Case) -> FieldStore {
    match c {
        Case::Ocean => fields(d, earth_t, ocean),
        Case::Ridge => fields(d, earth_t, ridge),
        Case::Tidal => fields(d, |u| 240.0 + 60.0 * u[0].max(0.0), ocean),
    }
}

struct Out { wind: Vec<f64>, steps: f64, resid: f64 }

/// backend: 0 CPU f64, 1 emulated f32 kernels, 2 GPU.
fn run(d: &Domain, c: Case, backend: f64, extra: &[(&str, &str, f64)]) -> enbuilder::Result<Out> {
    let mut f = mk(d, c);
    let mut s = Scheduler::new();
    add_atmosphere_stage_stepped(&mut s)?;
    s.set_param("wind", "backend", backend)?;
    for (m, k, v) in extra {
        s.set_param(m, k, *v)?;
    }
    s.run(d, &mut f, &DirtyRegion::All)?;
    Ok(Out {
        wind: f.data("wind")?.to_vec(),
        steps: s.module_param("wind", "last_steps").unwrap(),
        resid: s.module_param("wind", "last_residual").unwrap(),
    })
}

fn rel(a: &Out, b: &Out) -> f64 { max_rel_error(&a.wind, &b.wind) }

fn mean_toward_substellar(d: &Domain, w: &[f64]) -> f64 {
    let (mut s, mut c) = (0.0, 0usize);
    for i in 0..d.n_cells() {
        let up = d.mesh.up[i];
        let cs = up[0];
        if cs.abs() > 0.99 { continue; }
        let dir = [1.0 - up[0] * cs, -up[1] * cs, -up[2] * cs];
        let l = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        s += (w[3 * i] * dir[0] + w[3 * i + 1] * dir[1] + w[3 * i + 2] * dir[2]) / l;
        c += 1;
    }
    s / c as f64
}

fn parity_cases() -> Vec<(&'static str, Domain, Case, Vec<(&'static str, &'static str, f64)>)> {
    vec![
        ("L4 ocean, no blocking", sphere(4), Case::Ocean, vec![("wind", "blocking_gain", 0.0)]),
        ("L4 ridge, blocking on", sphere(4), Case::Ridge, vec![]),
    ]
}

fn check_parity(backend: f64) {
    for (name, d, f, params) in parity_cases() {
        let cpu = run(&d, f, 0.0, &params).unwrap();
        let alt = run(&d, f, backend, &params).unwrap();
        let e = rel(&alt, &cpu);
        let ratio = alt.steps / cpu.steps;
        println!("[backend {backend}] {name}: max |du|/max|u| = {e:.3e}; steps cpu {} alt {} (ratio {ratio:.3})", cpu.steps, alt.steps);
        assert!(e < 1e-3, "{name}: parity {e}");
        assert!((ratio - 1.0).abs() < 0.10, "{name}: step ratio {ratio}");
        assert!(alt.wind.iter().all(|v| v.is_finite()));
        assert!(alt.resid <= 1e-3);
    }
}

#[test]
fn emulated_f32_kernels_match_cpu_steady_state_on_ocean_and_ridge_spheres() { check_parity(1.0); }

#[test]
fn parity_holds_in_the_blocked_ridge_region_itself() {
    let d = sphere(4);
    let (cpu, emu) = (run(&d, Case::Ridge, 0.0, &[]).unwrap(), run(&d, Case::Ridge, 1.0, &[]).unwrap());
    let free = run(&d, Case::Ridge, 0.0, &[("wind", "blocking_gain", 0.0)]).unwrap();
    let near: Vec<usize> = (0..d.n_cells()).filter(|&i| d.mesh.up[i][1].abs() < 0.2).collect();
    let pick = |w: &[f64]| -> Vec<f64> { near.iter().flat_map(|&i| [w[3 * i], w[3 * i + 1], w[3 * i + 2]]).collect() };
    let e = max_rel_error(&pick(&emu.wind), &pick(&cpu.wind));
    let effect = max_rel_error(&pick(&cpu.wind), &pick(&free.wind));
    println!("ridge region ({} cells): emulated-vs-cpu {e:.3e}; blocking changes the wind by {effect:.3e} (so parity here is non-trivial)", near.len());
    assert!(effect > 0.05, "blocking must matter in this region");
    assert!(e < 1e-3);
}

#[test]
fn tighter_tolerance_gives_the_same_steady_state() {
    let d = sphere(4);
    let f = Case::Ridge;
    let p = [("wind", "steady_tol", 1e-4)];
    let (cpu, emu) = (run(&d, f, 0.0, &p).unwrap(), run(&d, f, 1.0, &p).unwrap());
    let e = rel(&emu, &cpu);
    println!("tol 1e-4 ridge: parity {e:.3e}, steps cpu {} emulated {}", cpu.steps, emu.steps);
    assert!(e < 1e-3);
}

#[test]
fn tidally_locked_flow_direction_matches() {
    let d = sphere(4);
    let f = Case::Tidal;
    for period in [0.0, 30.0 * DAY] {
        let p = [("pressure", "rotation_period_s", period)];
        let (cpu, emu) = (run(&d, f, 0.0, &p).unwrap(), run(&d, f, 1.0, &p).unwrap());
        let (a, b) = (mean_toward_substellar(&d, &cpu.wind), mean_toward_substellar(&d, &emu.wind));
        println!("period {period}: toward substellar cpu {a:.3} emulated {b:.3}");
        assert!(b > 1.0 && (b / a - 1.0).abs() < 0.01);
    }
}

#[test]
fn deterministic_finite_and_bounded() {
    let d = sphere(4);
    let f = Case::Ridge;
    let (a, b) = (run(&d, f, 1.0, &[]).unwrap(), run(&d, f, 1.0, &[]).unwrap());
    assert_eq!(a.wind, b.wind, "bit-identical");
    assert!(a.wind.iter().all(|v| v.is_finite()));
    // soft-capped output
    let mut s = Scheduler::new();
    add_atmosphere_stage_stepped(&mut s).unwrap();
    s.set_param("wind", "backend", 1.0).unwrap();
    let mut ff = mk(&d, f);
    s.run(&d, &mut ff, &DirtyRegion::All).unwrap();
    assert!(ff.data("wind_speed").unwrap().iter().all(|&v| v <= 80.0));
}

#[test]
fn guards_and_parameter_validation() {
    let d = sphere(3);
    let f = Case::Ocean;
    assert!(matches!(run(&d, f, 1.0, &[("wind", "max_steps", 30.0)]), Err(Error::InvalidParam(_))));
    let mut s = Scheduler::new();
    add_atmosphere_stage_stepped(&mut s).unwrap();
    assert!(s.set_param("wind", "backend", 3.0).is_err());
    assert!(s.set_param("wind", "steps_per_pass", 7.0).is_err());
    assert!(s.set_param("wind", "steps_per_pass", 4.0).is_err());
    #[cfg(not(feature = "gpu"))]
    assert!(matches!(run(&d, f, 2.0, &[]), Err(Error::NoGpu(_))));
}

#[test]
fn reduce_kernel_matches_brute_force_and_handles_ragged_sizes() {
    for n in [1usize, 63, 64, 65, 1000, 2562] {
        let chg: Vec<f32> = (0..n).map(|i| ((i * 37 % 101) as f32) * 0.01).collect();
        let u: Vec<f32> = (0..3 * n).map(|i| ((i * 13 % 97) as f32) * 0.1).collect();
        let [c, s] = k_residual_reduce(&chg, &u);
        let bc = chg.iter().cloned().fold(0.0, f32::max);
        let bs = (0..n).map(|i| (u[3 * i].powi(2) + u[3 * i + 1].powi(2) + u[3 * i + 2].powi(2)).sqrt()).fold(0.0, f32::max);
        assert_eq!((c, s), (bc, bs), "n = {n}");
    }
}

#[cfg(feature = "gpu")]
mod gpu_tests {
    use super::*;
    fn has_gpu() -> bool {
        match enbuilder::gpu::GpuContext::new() {
            Ok(c) => { println!("GPU adapter: {} ({:?})", c.adapter_info.name, c.adapter_info.backend); true }
            Err(e) => { println!("SKIPPED (no GPU): {e}"); false }
        }
    }
    #[test]
    fn gpu_matches_cpu_steady_state() { if has_gpu() { check_parity(2.0); } }
    #[test]
    fn gpu_tidal_lock_and_determinism() {
        if !has_gpu() { return; }
        let d = sphere(4);
        let f = Case::Tidal;
        let (a, b) = (run(&d, f, 2.0, &[]).unwrap(), run(&d, f, 2.0, &[]).unwrap());
        assert!(mean_toward_substellar(&d, &a.wind) > 1.0);
        assert_eq!(a.wind, b.wind, "same adapter, same bits");
        assert!(a.wind.iter().all(|v| v.is_finite()));
    }
}
