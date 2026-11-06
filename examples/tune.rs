//! Earth-like sphere exploration for Part 3b-4 tuning: prints budgets and zonal precipitation per season.
use enbuilder::generators::*;
use enbuilder::*;

const YEAR: f64 = 3.15576e7;
const DAY: f64 = 86400.0;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sub: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    let mut d = generate_sphere(&SphereParams { radius: 6.371e6, subdivision: sub, surface_gravity: 9.81, seed: 7 }).unwrap();
    d.lights.push(Light::directional([1.0, 0.0, 0.0], 1361.0).with_orbit(YEAR, DAY, 23.4f64.to_radians()));
    let n = d.n_cells();
    let a = [0.3_f64, 0.5, 0.8];
    let al = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let z: Vec<f64> = d.mesh.up.iter().map(|u| {
        let c = (u[0] * a[0] + u[1] * a[1] + u[2] * a[2]) / al;
        if c > 0.4 { 100.0 + 3000.0 * (-((c - 0.7).powi(2)) / (2.0 * 0.05f64.powi(2))).exp() } else { -3000.0 }
    }).collect();
    let mut f = FieldStore::new(n);
    f.put("elevation", 1, z).unwrap();
    seed_temperature(&mut f, 288.0).unwrap();
    let mut s = Scheduler::new();
    add_climate_pipeline(&mut s).unwrap();
    for arg in args.iter().skip(2) {
        let (k, v) = arg.split_once('=').unwrap();
        let (m, p) = k.split_once('.').unwrap();
        s.set_param(m, p, v.parse().unwrap()).unwrap();
    }
    for season in [0.0, 0.25, 0.75] {
        let r = run_season(&d, &mut f, &mut s, season).unwrap();
        let b = r.budget;
        println!("season {season}: P {:.2} mm/d  E {:.2} mm/d  vapour mean {:.1} kg/m2  residual_rel {:.2e}  energy passes {}",
            r.mean_precip_mm_day, r.mean_evap_mm_day, b.vapor_end_kg / d.mesh.total_area(), b.residual_rel, r.passes_energy);
        let p = f.data("precipitation").unwrap();
        let e = f.data("evaporation").unwrap();
        let v = f.data("vapor").unwrap();
        let mut row = String::new();
        for lo in (-90..90).step_by(15) {
            let (mut sp, mut se, mut sv, mut sa) = (0.0, 0.0, 0.0, 0.0);
            for i in 0..n {
                let lat = d.mesh.up[i][2].asin().to_degrees();
                if lat >= lo as f64 && lat < (lo + 15) as f64 { sp += p[i] * d.mesh.area[i]; se += e[i] * d.mesh.area[i]; sv += v[i] * d.mesh.area[i]; sa += d.mesh.area[i]; }
            }
            let dv = f.data("wind_divergence").unwrap();
            let sd: f64 = (0..n).filter(|&i| { let lat = d.mesh.up[i][2].asin().to_degrees(); lat >= lo as f64 && lat < (lo + 15) as f64 }).map(|i| dv[i] * d.mesh.area[i]).sum();
            row += &format!("  {:>4}: P{:5.2} E{:5.2} V{:5.1} div{:7.2}e-6\n", lo, sp / sa * DAY, se / sa * DAY, sv / sa, sd / sa * 1e6);
        }
        println!("{row}");
        let (po, pc, pf) = (f.data("precip_oro").unwrap(), f.data("precip_conv").unwrap(), f.data("precip_front").unwrap());
        let tot = |x: &[f64]| x.iter().zip(&d.mesh.area).map(|(a, b)| a * b).sum::<f64>() / d.mesh.total_area() * DAY;
        let lift = f.data("lift").unwrap();
        let zz = f.data("elevation").unwrap();
        let near: Vec<usize> = (0..n).filter(|&i| { let c = (d.mesh.up[i][0]*a[0]+d.mesh.up[i][1]*a[1]+d.mesh.up[i][2]*a[2])/al; c > 0.55 && c < 0.85 && zz[i] > 0.0 }).collect();
        let m = |idx: Vec<usize>, x: &[f64]| if idx.is_empty() { f64::NAN } else { idx.iter().map(|&i| x[i]).sum::<f64>() / idx.len() as f64 * DAY };
        let (wi, le): (Vec<usize>, Vec<usize>) = near.iter().partition(|&&i| lift[i] > 0.0);
        println!("   ridge band: windward n={} P_oro {:.3} P {:.2} | leeward n={} P_oro {:.3} P {:.2} mm/d  | V wind {:.1} lee {:.1}", wi.len(), m(wi.clone(), po), m(wi.clone(), p), le.len(), m(le.clone(), po), m(le.clone(), p), m(wi.clone(), v)/DAY, m(le.clone(), v)/DAY);
        let ps = f.data("precip_sat").unwrap();
        println!("   oro {:.3} conv {:.3} front {:.3} sat {:.3} mm/d", tot(po), tot(pc), tot(pf), tot(ps));
    }
}
