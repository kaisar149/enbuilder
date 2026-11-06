//! Part 3c-2 tests: backend trait, f32 packing, Laplacian parity.
//! Always-on tests use the f32 CPU emulation of the WGSL kernel. GPU tests (feature `gpu`) skip with a
//! message when no adapter exists.
use enbuilder::backend::*;
use enbuilder::generators::*;
use enbuilder::*;

fn domains() -> Vec<(&'static str, Domain)> {
    vec![
        ("sphere L4", generate_sphere(&SphereParams { radius: 6.371e6, subdivision: 4, surface_gravity: 9.81, seed: 1 }).unwrap()),
        ("cube 14/edge", generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap()),
        ("plane 64x64", generate_plane(&PlaneParams {
            shape: PlaneShape::Rect { nx: 64, ny: 64 }, cell_size: 5.0e4, boundary: BoundaryKind::Wrap, surface_gravity: 9.81, seed: 1,
        }).unwrap()),
    ]
}

/// A smooth, realistic-magnitude field (temperature-like, ~220-310 K) plus a rough component.
fn field(d: &Domain) -> Vec<f64> {
    d.mesh.centroid.iter().enumerate().map(|(i, c)| {
        let r = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt().max(1.0);
        265.0 + 40.0 * (c[2] / r) + 7.0 * (c[0] / 1.0e6).sin() + 3.0 * ((i * 7919 % 101) as f64 / 101.0)
    }).collect()
}

#[test]
fn f32_round_trip_error_is_below_epsilon_times_magnitude() {
    for (name, d) in domains() {
        let x = field(&d);
        let back = to_f64(&to_f32(&x).unwrap());
        let m = x.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        let err = x.iter().zip(&back).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        println!("{name}: round-trip max error {err:.3e} (limit {:.3e})", f32::EPSILON as f64 * m);
        assert!(err <= f32::EPSILON as f64 * m);
    }
    assert!(to_f32(&[1e300]).is_err(), "overflow is reported");
}

#[test]
fn packed_mesh_matches_the_mesh() {
    for (_, d) in domains() {
        let pm = PackedMesh::from_domain(&d);
        assert_eq!(pm.offsets, d.mesh.adj_offsets);
        assert_eq!(pm.adj, d.mesh.adj);
        for k in 0..pm.coef.len() {
            let w = d.mesh.edge_len[k] / d.mesh.edge_dist[k];
            assert!(((pm.coef[k] as f64) / w - 1.0).abs() < 1e-6);
        }
    }
}

#[test]
fn cpu_backend_is_the_f64_reference_and_is_conservative() {
    let cpu = make_backend(BackendKind::Cpu).unwrap();
    assert_eq!(cpu.precision(), "f64");
    for (_, d) in domains() {
        let x = field(&d);
        let l = cpu.laplacian(&d, &x).unwrap();
        // symmetric weights: sum_i out_i = 0 up to rounding
        let sum: f64 = l.iter().sum();
        let mag: f64 = l.iter().map(|v| v.abs()).sum();
        assert!(sum.abs() < 1e-9 * mag.max(1.0), "sum {sum}");
        // constant field -> zero
        assert!(cpu.laplacian(&d, &vec![5.0; d.n_cells()]).unwrap().iter().all(|&v| v == 0.0));
        assert!(cpu.laplacian(&d, &x[1..]).is_err());
    }
}

#[test]
fn f32_kernel_arithmetic_matches_cpu_within_1e_4_on_all_generators() {
    let f32b = CpuF32;
    let cpu = Cpu;
    for (name, d) in domains() {
        let x = field(&d);
        let (a, b) = (f32b.laplacian(&d, &x).unwrap(), cpu.laplacian(&d, &x).unwrap());
        let e = max_rel_error(&a, &b);
        println!("{name}: f32-emulated Laplacian rel error {e:.3e}");
        assert!(e < 1e-4, "{name}: {e}");
    }
}

#[test]
fn gpu_without_the_feature_reports_no_gpu() {
    #[cfg(not(feature = "gpu"))]
    assert!(matches!(make_backend(BackendKind::Gpu), Err(Error::NoGpu(_))));
}

#[cfg(feature = "gpu")]
mod gpu_tests {
    use super::*;
    use enbuilder::gpu::*;

    fn ctx() -> Option<GpuContext> {
        match GpuContext::new() {
            Ok(c) => { println!("GPU adapter: {} ({:?})", c.adapter_info.name, c.adapter_info.backend); Some(c) }
            Err(e) => { println!("SKIPPED (no GPU): {e}"); None }
        }
    }

    #[test]
    fn upload_download_round_trip() {
        let Some(c) = ctx() else { return };
        for (name, d) in domains() {
            let x = field(&d);
            let back = c.round_trip(&x).unwrap();
            let m = x.iter().fold(0.0f64, |a, v| a.max(v.abs()));
            let err = x.iter().zip(&back).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(err <= f32::EPSILON as f64 * m, "{name}: {err}");
        }
    }

    #[test]
    fn gpu_laplacian_parity_on_sphere_cube_and_plane() {
        let Some(c) = ctx() else { return };
        for (name, d) in domains() {
            let x = field(&d);
            let k = GpuLaplacian::new(&c, &PackedMesh::from_domain(&d));
            let g = to_f64(&k.run(&c, &to_f32(&x).unwrap()).unwrap());
            let e = max_rel_error(&g, &Cpu.laplacian(&d, &x).unwrap());
            println!("{name}: GPU vs CPU f64 relative error {e:.3e}");
            assert!(e < 1e-4, "{name}: {e}");
        }
    }
}
