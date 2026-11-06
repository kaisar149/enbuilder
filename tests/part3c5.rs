//! Part 3c-5 tests: backend switch and full-pipeline comparison.
//! `Backend::Cpu` (f64) is the reference; `Backend::CpuF32` runs the GPU kernels' arithmetic on the CPU and is
//! always available; `Backend::Gpu` needs `--features gpu` and an adapter (prints SKIPPED and passes without one).
use enbuilder::budget::add_climate_pipeline;
use enbuilder::testworld::*;
use enbuilder::*;

const SEASONS: [f64; 3] = [0.0, 0.25, 0.75];

fn run(sub: u32, backend: Backend, seasons: &[f64]) -> (Vec<ClimateSummary>, Scheduler) {
    let (d, mut f) = earth_like_world(sub).unwrap();
    run_pipeline(&d, &mut f, backend, seasons).unwrap()
}

/// Equinox (season 0): tropics wetter than subtropics. The ITCZ shifts at the solstices, so this is not
/// asserted there.
fn check_equinox(tag: &str, s: &ClimateSummary) {
    assert!(s.tropics > 1.5 * s.subtropics, "{tag}: tropics {:.2} vs subtropics {:.2} mm/day", s.tropics, s.subtropics);
    check_common(tag, s);
}

/// Solstice (season 0.25): rain shadow behind the ridge, where 3b-4 tuned it. At the equinox the mid-latitude
/// ridge is dry and windward/leeward are only compared between backends (see `compare`).
fn check_solstice(tag: &str, s: &ClimateSummary) {
    assert!(s.windward > s.leeward, "{tag}: rain shadow missing, windward {:.2} vs leeward {:.2}", s.windward, s.leeward);
    check_common(tag, s);
}

fn check_common(tag: &str, s: &ClimateSummary) {
    assert!(s.residual_rel < 1e-8, "{tag}: water budget residual {:.2e}", s.residual_rel);
    assert!(s.mean_precip > 0.5 && s.mean_precip < 8.0, "{tag}: implausible mean precipitation {:.2}", s.mean_precip);
}

fn compare(tag: &str, a: &[ClimateSummary], b: &[ClimateSummary]) {
    for (x, y) in a.iter().zip(b) {
        let rel = (x.mean_precip - y.mean_precip).abs() / x.mean_precip;
        println!("{tag} season {}: mean P {:.3} vs {:.3} mm/day ({:.2}%)", x.season, x.mean_precip, y.mean_precip, 100.0 * rel);
        assert_eq!(x.windward > x.leeward, y.windward > y.leeward, "{tag}: backends disagree on the rain shadow");
        assert!(rel < 0.05, "{tag}: global mean precipitation differs by {:.2}% (limit 5%)", 100.0 * rel);
    }
}

#[test]
fn backend_cpu_equals_plain_stepped_stage() {
    // the new constructor with Backend::Cpu must be exactly the old one
    let (d, mut f1) = earth_like_world(3).unwrap();
    let (_, mut f2) = earth_like_world(3).unwrap();
    let mut s1 = Scheduler::new();
    add_atmosphere_stage_stepped(&mut s1).unwrap();
    let mut s2 = Scheduler::new();
    add_atmosphere_stage_stepped_with(&mut s2, Backend::Cpu).unwrap();
    s1.run(&d, &mut f1, &DirtyRegion::All).unwrap();
    s2.run(&d, &mut f2, &DirtyRegion::All).unwrap();
    assert_eq!(f1.data("wind").unwrap(), f2.data("wind").unwrap());
}

#[test]
fn backend_param_is_set_by_constructor() {
    let mut s = Scheduler::new();
    add_atmosphere_stage_stepped_with(&mut s, Backend::CpuF32).unwrap();
    assert_eq!(s.module_param("wind", "backend"), Some(1.0));
    let mut s = Scheduler::new();
    add_atmosphere_stage_stepped_with(&mut s, Backend::Gpu).unwrap();
    assert_eq!(s.module_param("wind", "backend"), Some(2.0));
}

#[cfg(not(feature = "gpu"))]
#[test]
fn gpu_backend_without_feature_is_no_gpu() {
    let (d, mut f) = earth_like_world(2).unwrap();
    let err = run_pipeline(&d, &mut f, Backend::Gpu, &[0.0]).err().expect("must fail");
    assert!(matches!(err, Error::NoGpu(_)), "got {err:?}");
}

#[test]
fn default_pipeline_still_uses_diagnostic_wind() {
    let mut s = Scheduler::new();
    add_climate_pipeline(&mut s).unwrap();
    // the diagnostic wind has no `backend` parameter; the stepped one does
    assert_eq!(s.module_param("wind", "backend"), None);
}

#[test]
fn cpu_pipeline_is_qualitatively_earth_like_and_conserves_water() {
    let (res, s) = run(4, Backend::Cpu, &SEASONS);
    check_equinox("cpu season 0", &res[0]);
    check_solstice("cpu season 0.25", &res[1]);
    for r in &res {
        assert!(r.residual_rel < 1e-8, "cpu residual {:.2e}", r.residual_rel);
    }
    assert!(s.module_param("wind", "last_solve_s").unwrap() > 0.0);
}

#[test]
fn f32_kernel_pipeline_matches_cpu_within_5_percent() {
    let (cpu, _) = run(4, Backend::Cpu, &SEASONS);
    let (f32r, s) = run(4, Backend::CpuF32, &SEASONS);
    check_equinox("f32 season 0", &f32r[0]);
    check_solstice("f32 season 0.25", &f32r[1]);
    compare("f32-emulated", &cpu, &f32r);
    // conservation lives in the f64 transport, not in the wind: unchanged by the f32 wind
    for r in &f32r {
        assert!(r.residual_rel < 1e-8, "f32 residual {:.2e}", r.residual_rel);
    }
    assert_eq!(s.module_param("wind", "last_copy_back_s"), Some(0.0)); // no device, no copy
}

#[test]
fn f32_pipeline_is_deterministic() {
    let (a, _) = run(3, Backend::CpuF32, &[0.0]);
    let (b, _) = run(3, Backend::CpuF32, &[0.0]);
    assert_eq!(a[0].mean_precip, b[0].mean_precip);
    assert_eq!(a[0].zonal, b[0].zonal);
}

#[cfg(feature = "gpu")]
#[test]
fn gpu_pipeline_matches_cpu() {
    let (d, mut f) = earth_like_world(4).unwrap();
    let gpu = match run_pipeline(&d, &mut f, Backend::Gpu, &SEASONS) {
        Ok(r) => r,
        Err(Error::NoGpu(m)) => {
            println!("SKIPPED (no GPU): {m}");
            return;
        }
        Err(e) => panic!("{e:?}"),
    };
    let (cpu, _) = run(4, Backend::Cpu, &SEASONS);
    check_equinox("gpu season 0", &gpu.0[0]);
    check_solstice("gpu season 0.25", &gpu.0[1]);
    compare("gpu", &cpu, &gpu.0);
    for r in &gpu.0 {
        assert!(r.residual_rel < 1e-8, "gpu residual {:.2e}", r.residual_rel);
    }
    println!("GPU copy-back {:.3e} s, solve {:.3e} s",
        gpu.1.module_param("wind", "last_copy_back_s").unwrap(), gpu.1.module_param("wind", "last_solve_s").unwrap());
}
