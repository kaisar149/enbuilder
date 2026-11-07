use enbuilder::generators::*;
use enbuilder::math::{scale, Vec3};
use enbuilder::ocean::*;
use enbuilder::ocean_currents::*;
use enbuilder::ocean_forcing::*;
use enbuilder::*;
const R: f64 = 6.371e6;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let level: u32 = args[1].parse().unwrap();
    let rs: Vec<f64> = args[2..].iter().map(|s| s.parse().unwrap()).collect();
    let d = generate_sphere(&SphereParams { radius: R, subdivision: level, surface_gravity: 9.81, seed: 1 }).unwrap();
    let n = d.n_cells();
    let lat = |i: usize| d.mesh.up[i][2].asin();
    let lon = |i: usize| d.mesh.up[i][1].atan2(d.mesh.up[i][0]);
    let land: Vec<bool> = (0..n).map(|i| d.mesh.centroid[i][1].abs() < 0.13 * R && lat(i).abs() < 70f64.to_radians()).collect();
    let z: Vec<f64> = (0..n).map(|i| if land[i] { 100.0 } else { -1000.0 }).collect();
    let (ocean, comp) = ocean_cells(&d, &z, &vec![0.0; n], 1.0);
    let tau: Vec<Vec3> = (0..n).map(|i| scale(d.frames[i].east, -0.1 * (3.0 * lat(i)).cos())).collect();
    let curl = curl_of_masked(&d, &tau, &ocean);
    let om = 7.292e-5;
    let f: Vec<f64> = (0..n).map(|i| 2.0 * om * d.mesh.up[i][2]).collect();
    let beta = beta_of(&d, &f);
    for &r in &rs {
        for beta_on in [0.0, 1.0] {
            let mut fs = FieldStore::new(n);
            fs.put("ocean_mask", 1, ocean.iter().map(|&o| o as i32 as f64).collect()).unwrap();
            fs.put("ocean_component", 1, comp.iter().map(|&c| c as f64).collect()).unwrap();
            fs.put("stress_curl", 1, curl.clone()).unwrap();
            fs.put("beta", 1, beta.clone()).unwrap();
            let mut m = OceanCurrents::new();
            m.set_param("r_friction", r).unwrap();
            m.set_param("beta_on", beta_on).unwrap();
            m.set_param("max_current", 1e9).unwrap();
            let t = std::time::Instant::now();
            m.run(&d, &mut fs, &DirtyRegion::All).unwrap();
            let sp = fs.data("current_speed").unwrap().to_vec();
            let mut out = String::new();
            for (hemi, (lo, hi)) in [("N", (10.0, 40.0)), ("S", (-40.0, -10.0))] {
                for basin in 0..2 {
                    let (mut w, mut e) = (0.0f64, 0.0f64);
                    for i in 0..n {
                        if !ocean[i] { continue; }
                        let l = lat(i).to_degrees();
                        if l < lo || l > hi { continue; }
                        let mut lam = lon(i).to_degrees();
                        if basin == 1 { lam += 180.0; }
                        if lam < 0.0 { lam += 360.0; }
                        if lam > 180.0 { lam -= 360.0; if basin==1 {} }
                        // basin A: lam in (0,180)
                        if !(lam > 0.0 && lam < 180.0) { continue; }
                        let edge = (0.13 / lat(i).cos()).asin().to_degrees();
                        if lam < edge + 15.0 { w = w.max(sp[i]); }
                        if lam > 180.0 - edge - 15.0 { e = e.max(sp[i]); }
                    }
                    out += &format!("{hemi}{basin}: W {w:.3} E {e:.3} ({:.2}x)  ", w / e);
                }
            }
            println!("L{level} r={r:.1e} beta_on={beta_on} kind={} it={} restarts={} maxraw={:.3} layer_med={:.2} {:?}ms\n   {out}", m.params().get("solver_kind").unwrap(), m.params().get("cg_iterations").unwrap(), m.params().get("solver_restarts").unwrap(), m.params().get("max_speed_raw").unwrap(), m.params().get("layer_cells_median").unwrap(), t.elapsed().as_millis());
        }
    }
}
