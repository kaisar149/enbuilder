//! Part 3c-5: full climate pipeline (level 4 Earth-like sphere), CPU f64 wind vs f32-kernel wind (and GPU with
//! `--features gpu`): global mean precipitation, zonal precipitation, ridge rain shadow, water-budget residual.
//!   cargo run --release --example compare_backends [-- <level>]
use enbuilder::testworld::*;
use enbuilder::*;

fn main() {
    let lvl: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    let seasons = [0.0, 0.25, 0.75];
    let mut backends = vec![("cpu-f64", Backend::Cpu), ("cpu-f32-emul", Backend::CpuF32)];
    #[cfg(feature = "gpu")]
    backends.push(("gpu", Backend::Gpu));
    let mut results = Vec::new();
    for (name, b) in &backends {
        let (d, mut f) = earth_like_world(lvl).unwrap();
        match run_pipeline(&d, &mut f, *b, &seasons) {
            Ok((r, s)) => {
                println!("{name}: wind solve {:.3} s, copy-back {:.2e} s (last season)",
                    s.module_param("wind", "last_solve_s").unwrap(), s.module_param("wind", "last_copy_back_s").unwrap());
                results.push((*name, r));
            }
            Err(Error::NoGpu(m)) => println!("{name}: SKIPPED (no GPU): {m}"),
            Err(e) => println!("{name}: FAILED {e:?}"),
        }
    }
    let Some((_, base)) = results.first() else { return };
    for (k, season) in seasons.iter().enumerate() {
        println!("\n== season {season} ==");
        println!("{:<13} {:>8} {:>8} {:>8} {:>9} {:>9} {:>10}", "backend", "mean P", "tropics", "subtrop", "windward", "leeward", "residual");
        for (name, r) in &results {
            let x = &r[k];
            println!("{name:<13} {:>8.3} {:>8.2} {:>8.2} {:>9.2} {:>9.2} {:>10.2e}  (mean P diff vs cpu {:+.2}%)",
                x.mean_precip, x.tropics, x.subtropics, x.windward, x.leeward, x.residual_rel,
                100.0 * (x.mean_precip - base[k].mean_precip) / base[k].mean_precip);
        }
        println!("zonal P (mm/day), 15-degree bands from -90:");
        for (name, r) in &results {
            println!("  {name:<13} {}", r[k].zonal.iter().map(|v| format!("{v:5.2}")).collect::<Vec<_>>().join(" "));
        }
    }
}
