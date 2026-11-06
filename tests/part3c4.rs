//! Part 3c-4 tests: f32 projection kernels (CG with device-side scalars and reductions) vs the CPU f64 projection.
//! The kernels run emulated on the CPU (always) and on a real adapter (feature `gpu`, skipped without one).
use enbuilder::backend::PackedMesh;
use enbuilder::generators::*;
use enbuilder::projection::*;
use enbuilder::transport::{edge_flux, flux_divergence, project_flux_counted};
use enbuilder::*;

fn sphere_wind_flux() -> (Domain, Vec<f64>) {
    let d = generate_sphere(&SphereParams { radius: 6.371e6, subdivision: 4, surface_gravity: 9.81, seed: 7 }).unwrap();
    let mut f = FieldStore::new(d.n_cells());
    let tsl: Vec<f64> = d.mesh.up.iter().map(|u| 300.0 - 40.0 * u[2] * u[2]).collect();
    f.put("temperature_sl", 1, tsl.clone()).unwrap();
    f.put("temperature", 1, tsl).unwrap();
    f.put("elevation", 1, d.mesh.up.iter().map(|u| if u[2] > 0.5 { 500.0 } else { -3000.0 }).collect()).unwrap();
    let mut s = Scheduler::new();
    add_atmosphere_stage_stepped(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let w = f.data("wind").unwrap().to_vec();
    let u: Vec<[f64; 3]> = (0..d.n_cells()).map(|i| [w[3 * i], w[3 * i + 1], w[3 * i + 2]]).collect();
    let fl = edge_flux(&d, &u);
    (d, fl)
}

fn synthetic_flux(d: &Domain) -> Vec<f64> {
    let u: Vec<[f64; 3]> = (0..d.n_cells()).map(|i| {
        let (c, up) = (d.mesh.centroid[i], d.mesh.up[i]);
        // u_x depends on x and u_y on y, so the field has real divergence (on the torus too)
        let v = [15.0 * (c[0] / 9.0e5).sin(), 15.0 * (c[1] / 1.1e6).cos(), 8.0 * (c[2] / 7.0e5).sin()];
        let r = v[0] * up[0] + v[1] * up[1] + v[2] * up[2];
        [v[0] - r * up[0], v[1] - r * up[1], v[2] - r * up[2]]
    }).collect();
    edge_flux(d, &u)
}

fn cases() -> Vec<(&'static str, Domain, Vec<f64>)> {
    let (ds, fs) = sphere_wind_flux();
    let cube = generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap();
    let plane = generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: 64, ny: 64 }, cell_size: 5.0e4, boundary: BoundaryKind::Wrap, surface_gravity: 9.81, seed: 1 }).unwrap();
    let (fc, fp) = (synthetic_flux(&cube), synthetic_flux(&plane));
    vec![("sphere L4 (stepped wind)", ds, fs), ("cube 14/edge", cube, fc), ("plane 64x64 wrap", plane, fp)]
}

fn fmax(f: &[f64]) -> f64 { f.iter().fold(0.0f64, |a, v| a.max(v.abs())) }
fn max_diff(a: &[f64], b: &[f64]) -> f64 { a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max) }

/// `solve` runs the f32 kernels (emulated or GPU).
fn check(label: &str, solve: &dyn Fn(&PackedMesh, &[f64], &ProjConfig) -> Result<ProjResult>) {
    for (name, d, f) in cases() {
        let pm = PackedMesh::from_domain(&d);
        let cfg = ProjConfig::default();
        let (_, fc_ref, it_ref) = project_flux_counted(&d, &f, 1e-12, 20000);
        let (_, _, it_same_tol) = project_flux_counted(&d, &f, cfg.tol, 20000);
        let r = solve(&pm, &f, &cfg).unwrap();
        let flux_err = max_diff(&r.fc, &fc_ref) / fmax(&f);
        let res_raw = divergence_residual_f64(&d, &f, &r.fc);
        let fc64 = correct_f64(&d, &f, &r.phi);
        let res_f64 = divergence_residual_f64(&d, &f, &fc64);
        let res_before = divergence_residual_f64(&d, &f, &f);
        let res_cpu = divergence_residual_f64(&d, &f, &fc_ref);
        let ceil = it_same_tol.div_ceil(cfg.interval) * cfg.interval;
        println!(
            "[{label}] {name}: flux err {flux_err:.2e} of max|F|; div residual: before {res_before:.2e}, f32 F' {res_raw:.2e}, f64 F' from f32 phi {res_f64:.2e}, cpu f64 {res_cpu:.2e}; \\
             iters gpu {} / cpu@1e-6 {it_same_tol} (rounded to interval {ceil}) / cpu@1e-12 {it_ref}; device rel residual {:.2e}",
            r.iterations, r.rel_residual
        );
        assert!(flux_err < 1e-4, "{name}: flux parity {flux_err}");
        assert!(res_raw < 1e-5, "{name}: f32 flux residual {res_raw}");
        assert!(res_f64 < 1e-5, "{name}: f64-corrected residual {res_f64}");
        assert!(res_raw < 0.01 * res_before.max(1e-30), "{name}: projection must remove nearly all divergence");
        assert!(r.fc.iter().all(|v| v.is_finite()));
        assert!(r.iterations as f64 <= 1.2 * ceil as f64 && r.iterations as f64 >= 0.8 * it_same_tol as f64, "{name}: iterations {} vs cpu {it_same_tol}", r.iterations);
    }
}

#[test]
fn emulated_f32_projection_matches_cpu_on_sphere_cube_and_plane() {
    check("emulated", &|pm, f, cfg| project_f32_emulated(pm, f, cfg));
}

#[test]
fn corrected_flux_is_exactly_antisymmetric_in_f32() {
    let (d, f) = sphere_wind_flux();
    let pm = PackedMesh::from_domain(&d);
    let r = project_f32_emulated(&pm, &f, &ProjConfig::default()).unwrap();
    let m = &d.mesh;
    for i in 0..m.n_cells {
        for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
            let j = m.adj[k] as usize;
            let (a, b) = (m.adj_offsets[j] as usize, m.adj_offsets[j + 1] as usize);
            let rk = a + m.adj[a..b].binary_search(&(i as u32)).unwrap();
            assert_eq!(r.fc[k], -r.fc[rk]);
        }
    }
    let div = flux_divergence(m, &r.fc);
    assert!(div.iter().all(|v| v.is_finite()));
}

#[test]
fn zero_divergence_input_and_parameter_guards() {
    let (d, _) = sphere_wind_flux();
    let pm = PackedMesh::from_domain(&d);
    let zero = vec![0.0; pm.adj.len()];
    let r = project_f32_emulated(&pm, &zero, &ProjConfig::default()).unwrap();
    assert_eq!(r.iterations, 0);
    assert!(r.fc.iter().all(|&v| v == 0.0));
    let (_, f) = sphere_wind_flux();
    assert!(project_f32_emulated(&pm, &f, &ProjConfig { max_iters: 16, ..Default::default() }).is_err());
    assert!(project_f32_emulated(&pm, &f, &ProjConfig { interval: 0, ..Default::default() }).is_err());
}

#[test]
fn two_pass_dot_matches_f64_and_odd_sizes() {
    for n in [1usize, 63, 64, 65, 4097, 10242] {
        let a: Vec<f32> = (0..n).map(|i| ((i * 37 % 101) as f32) * 0.01 - 0.3).collect();
        let b: Vec<f32> = (0..n).map(|i| ((i * 13 % 97) as f32) * 0.02 - 0.5).collect();
        let exact: f64 = a.iter().zip(&b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let got = k_dot(&a, &b) as f64;
        let scale: f64 = a.iter().zip(&b).map(|(x, y)| (*x as f64 * *y as f64).abs()).sum();
        assert!((got - exact).abs() <= 1e-5 * scale.max(1e-9), "n {n}: {got} vs {exact}");
    }
}

#[cfg(feature = "gpu")]
mod gpu_tests {
    use super::*;
    #[test]
    fn gpu_projection_matches_cpu() {
        match enbuilder::gpu::GpuContext::new() {
            Ok(c) => {
                println!("GPU adapter: {} ({:?})", c.adapter_info.name, c.adapter_info.backend);
                check("gpu", &|pm, f, cfg| project_gpu(&c, pm, f, cfg));
            }
            Err(e) => println!("SKIPPED (no GPU): {e}"),
        }
    }
}
