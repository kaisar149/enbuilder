//! Part 3b-2 acceptance tests: divergence projection and conservative advection, on sphere, cube and plane.
use enbuilder::generators::*;
use enbuilder::transport::*;
use enbuilder::*;

const R: f64 = 6.371e6;

fn sphere() -> Domain { generate_sphere(&SphereParams { radius: R, subdivision: 4, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn cube() -> Domain { generate_cube(&CubeParams { half_side: 3.0e6, cells_per_edge: 14, surface_gravity: 9.81, seed: 1 }).unwrap() }
fn plane_wrap(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wrap, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn plane_wall() -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: 40, ny: 24 }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}

/// A smooth wind with a strong divergent part, tangent to every cell.
fn test_wind(d: &Domain) -> Vec<f64> {
    let mut w = Vec::new();
    for i in 0..d.n_cells() {
        let c = d.mesh.centroid[i];
        let up = d.mesh.up[i];
        let a = [3.0 + c[1] * 1e-6, -2.0 + c[2] * 1e-6, 4.0 - c[0] * 1e-6];
        let t = sub(a, scale(up, dot(a, up)));
        let s = 10.0 / length(t).max(1e-9);
        w.extend_from_slice(&scale(t, s));
    }
    w
}
use enbuilder::math::{dot, length, scale, sub};

fn blobby(d: &Domain) -> Vec<f64> { (0..d.n_cells()).map(|i| 1.0 + ((i * 37) % 11) as f64).collect() }
fn mass(d: &Domain, v: &[f64]) -> f64 { v.iter().zip(&d.mesh.area).map(|(a, b)| a * b).sum() }

fn make_fields(d: &Domain, wind: Vec<f64>, evap: Vec<f64>, vapor: Vec<f64>) -> FieldStore {
    let mut f = FieldStore::new(d.n_cells());
    f.put("wind", 3, wind).unwrap();
    f.put("evaporation", 1, evap).unwrap();
    f.put("vapor", 1, vapor).unwrap();
    f
}

fn run_transport(d: &Domain, f: &mut FieldStore, params: &[(&str, f64)]) {
    let mut s = Scheduler::new();
    add_moisture_transport_stage(&mut s).unwrap();
    for (k, v) in params {
        s.set_param("moisture_transport", k, *v).unwrap();
    }
    s.run(d, f, &DirtyRegion::All).unwrap();
}

fn check_domain(d: &Domain, name: &str) {
    let n = d.n_cells();
    let v0 = blobby(d);
    let mut f = make_fields(d, test_wind(d), vec![0.0; n], v0.clone());
    run_transport(d, &mut f, &[("warm_start", 1.0), ("transport_time_s", 3.0e6)]);
    let v = f.data("vapor").unwrap();
    // conservation with no sources
    let (m0, m1) = (mass(d, &v0), mass(d, v));
    assert!(((m1 - m0) / m0).abs() < 1e-10, "{name}: mass drift {}", (m1 - m0) / m0);
    // positivity
    assert!(v.iter().all(|&x| x >= 0.0 && x.is_finite()), "{name}: negative vapour");
    // projection residual
    let r = f.data("divergence_residual").unwrap();
    let rmax = r.iter().fold(0.0f64, |a, b| a.max(b.abs()));
    assert!(rmax < 1e-6, "{name}: residual {rmax}");
    // the raw wind really was divergent, and the projection is not a no-op disaster
    let raw = flux_divergence(&d.mesh, &{
        let w = f.data("wind").unwrap();
        let u: Vec<[f64; 3]> = (0..n).map(|i| [w[3 * i], w[3 * i + 1], w[3 * i + 2]]).collect();
        edge_flux(d, &u)
    });
    let scale_f = raw.iter().fold(0.0f64, |a, b| a.max(b.abs()));
    assert!(scale_f > 0.0, "{name}: test wind has no divergence");
    assert!(rmax * 1e3 < scale_f / (d.mesh.edge_len.iter().cloned().fold(0.0, f64::max) * 10.0 * 6.0), "{name}: not reduced enough");
    // the field actually moved
    assert!(v.iter().zip(&v0).any(|(a, b)| (a - b).abs() > 1e-3), "{name}: nothing advected");
}

#[test] fn conserves_and_projects_on_sphere() { check_domain(&sphere(), "sphere"); }
#[test] fn conserves_and_projects_on_cube() { check_domain(&cube(), "cube"); }
#[test] fn conserves_and_projects_on_wrapped_plane() { check_domain(&plane_wrap(48, 24), "plane_wrap"); }
#[test] fn conserves_and_projects_on_walled_plane() { check_domain(&plane_wall(), "plane_wall"); }

#[test]
fn flux_is_antisymmetric() {
    for d in [sphere(), cube(), plane_wrap(12, 6)] {
        let w = test_wind(&d);
        let u: Vec<[f64; 3]> = (0..d.n_cells()).map(|i| [w[3 * i], w[3 * i + 1], w[3 * i + 2]]).collect();
        let f = edge_flux(&d, &u);
        let m = &d.mesh;
        for i in 0..m.n_cells {
            for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
                let j = m.adj[k] as usize;
                let b = m.adj_offsets[j] as usize;
                let kk = b + m.neighbors(j).iter().position(|&x| x as usize == i).unwrap();
                assert_eq!(f[k], -f[kk]);
            }
        }
    }
}

#[test]
fn constant_vapour_stays_constant_under_projected_flow() {
    let d = sphere();
    let n = d.n_cells();
    let mut f = make_fields(&d, test_wind(&d), vec![0.0; n], vec![20.0; n]);
    run_transport(&d, &mut f, &[("warm_start", 1.0), ("transport_time_s", 2.0e6)]);
    let v = f.data("vapor").unwrap();
    let dev = v.iter().fold(0.0f64, |a, b| a.max((b - 20.0).abs()));
    assert!(dev < 1e-6, "constant field drifted by {dev}");
}

#[test]
fn blob_moves_at_wind_speed() {
    let d = plane_wrap(80, 16);
    let n = d.n_cells();
    let (u0, t) = (10.0f64, 2.0e4f64); // 200 km = 20 cells
    let mut w = vec![0.0; 3 * n];
    for i in 0..n {
        w[3 * i] = u0;
    }
    let v0: Vec<f64> = (0..n).map(|i| {
        let x = d.mesh.centroid[i][0] / 1e4;
        let y = d.mesh.centroid[i][1] / 1e4;
        (-((x + 15.0).powi(2) + (y - 0.0).powi(2)) / (2.0 * 9.0)).exp()
    }).collect();
    let cx = |v: &[f64]| (0..n).map(|i| v[i] * d.mesh.centroid[i][0]).sum::<f64>() / v.iter().sum::<f64>();
    let mut f = make_fields(&d, w, vec![0.0; n], v0.clone());
    run_transport(&d, &mut f, &[("warm_start", 1.0), ("transport_time_s", t)]);
    let v = f.data("vapor").unwrap();
    let moved = cx(v) - cx(&v0);
    assert!((moved - u0 * t).abs() / (u0 * t) < 0.02, "moved {moved} vs {}", u0 * t);
    // numerical diffusion: peak decays but stays within a factor of 2
    let peak = v.iter().cloned().fold(0.0, f64::max);
    assert!(peak > 0.3 && peak <= 1.0 + 1e-12, "peak {peak}");
    assert!(v.iter().all(|&x| x >= 0.0));
}

#[test]
fn sources_add_exactly_their_mass() {
    let d = sphere();
    let n = d.n_cells();
    let evap: Vec<f64> = (0..n).map(|i| 1e-5 * (1.0 + (i % 5) as f64)).collect();
    let v0 = blobby(&d);
    let mut f = make_fields(&d, test_wind(&d), evap.clone(), v0.clone());
    let t = 4.0e5;
    run_transport(&d, &mut f, &[("warm_start", 1.0), ("transport_time_s", t)]);
    let gain: f64 = evap.iter().zip(&d.mesh.area).map(|(e, a)| e * a).sum::<f64>() * t;
    let m1 = mass(&d, f.data("vapor").unwrap());
    assert!(((m1 - mass(&d, &v0) - gain) / gain).abs() < 1e-10);
}

#[test]
fn no_atmosphere_and_calm_wind_and_params() {
    let mut d = sphere();
    let n = d.n_cells();
    let mut f = make_fields(&d, test_wind(&d), vec![1e-5; n], vec![1.0; n]);
    d.media.atmosphere = None;
    run_transport(&d, &mut f, &[]);
    assert!(f.data("vapor").unwrap().iter().all(|&x| x == 0.0));
    // calm wind: vapour only accumulates from the source
    let d = sphere();
    let mut f = make_fields(&d, vec![0.0; 3 * n], vec![1e-5; n], vec![0.0; n]);
    run_transport(&d, &mut f, &[("init_residence_s", 0.0), ("transport_time_s", 1000.0)]);
    assert!(f.data("vapor").unwrap().iter().all(|&x| (x - 1e-2).abs() < 1e-12));
    let mut m = MoistureTransport::new();
    assert!(m.set_param("cfl", 1.5).is_err());
    assert!(m.set_param("warm_start", 0.5).is_err());
    assert!(m.set_param("zzz", 1.0).is_err());
}

#[test]
fn cold_start_uses_evaporation_and_is_deterministic() {
    let d = sphere();
    let n = d.n_cells();
    let mk = || {
        let mut f = FieldStore::new(n);
        f.put("wind", 3, test_wind(&d)).unwrap();
        f.put("evaporation", 1, vec![4e-5; n]).unwrap();
        f
    };
    let (mut a, mut b) = (mk(), mk());
    run_transport(&d, &mut a, &[]);
    run_transport(&d, &mut b, &[]);
    assert_eq!(a.data("vapor").unwrap(), b.data("vapor").unwrap());
    assert!(a.data("vapor").unwrap().iter().all(|&x| x > 0.0));
}
