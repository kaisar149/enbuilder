//! Part 9c: "generate N variations". The same parameters with N derived seeds, solved in parallel. Pure functions (no session or JSON types),
//! so they are testable alone. A variation is reproducible: its seed fully identifies it (`set_param seed=S` then `solve` gives the same
//! world as long as the solver settings match; `coarse` only shortens the spin-up).
use crate::png::fnv1a;
use crate::world::{self, Params, SolveError};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Upper bound on one batch.
pub const MAX_VARIATIONS: usize = 12;
/// Seeds stay below the `seed` parameter's maximum so that every variation can be opened with `set_param`.
pub const SEED_LIMIT: u64 = 1_000_000_000;
/// Fields kept per variation (enough for a thumbnail of any of them).
pub const KEPT_FIELDS: [&str; 4] = ["elevation", "temperature", "precipitation", "biome"];

/// `count` distinct seeds derived from `base`; none equals `base`. Deterministic.
pub fn variation_seeds(base: u64, count: usize) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::with_capacity(count);
    for i in 0..count {
        let mut attempt = 0u64;
        loop {
            let s = fnv1a(format!("variation|{base}|{i}|{attempt}").as_bytes()) % SEED_LIMIT;
            if s != base && !out.contains(&s) {
                out.push(s);
                break;
            }
            attempt += 1;
        }
    }
    out
}

/// The parameters a variation is solved with.
pub fn params_for(base: &Params, seed: u64, coarse: bool) -> Params {
    let mut p = base.clone();
    p.insert("seed".into(), seed as f64);
    if coarse {
        let years = p.get("spinup_years").copied().unwrap_or(1.0).min(1.0);
        p.insert("spinup_years".into(), years);
    }
    p
}

pub struct Variation {
    pub index: usize,
    pub seed: u64,
    /// (name, one value per cell)
    pub fields: Vec<(String, Vec<f64>)>,
}

pub struct Batch {
    pub base_seed: u64,
    pub coarse: bool,
    pub items: Vec<Variation>,
}

fn solve_one(base: &Params, index: usize, seed: u64, coarse: bool, cancel: &AtomicBool) -> Result<Variation, SolveError> {
    let p = params_for(base, seed, coarse);
    let solved = world::solve(&p, cancel, &mut |_, _| {})?;
    let mut fields = Vec::new();
    for name in KEPT_FIELDS {
        let data = solved.fields.data(name)?;
        fields.push((name.to_string(), data.to_vec()));
    }
    Ok(Variation { index, seed, fields })
}

/// Solves `count` variations on up to 4 worker threads. `progress(fraction, stage)` is called after each finished variation.
/// Order and content do not depend on thread timing: variation `i` always uses seed `variation_seeds(base, count)[i]`.
pub fn run(base: &Params, count: usize, coarse: bool, cancel: &AtomicBool, progress: &(dyn Fn(f64, &str) + Sync)) -> Result<Batch, SolveError> {
    let base_seed = base.get("seed").copied().unwrap_or(0.0) as u64;
    let seeds = variation_seeds(base_seed, count);
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 4).min(count.max(1));
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<Variation>>> = Mutex::new((0..count).map(|_| None).collect());
    let failure: Mutex<Option<SolveError>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= count || failure.lock().map(|f| f.is_some()).unwrap_or(true) {
                    break;
                }
                match solve_one(base, i, seeds[i], coarse, cancel) {
                    Ok(v) => {
                        if let Ok(mut s) = slots.lock() {
                            s[i] = Some(v);
                        }
                        let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                        progress(n as f64 / count as f64, &format!("variation {n} of {count}"));
                    }
                    Err(e) => {
                        if let Ok(mut f) = failure.lock() {
                            if f.is_none() {
                                *f = Some(e);
                            }
                        }
                        break;
                    }
                }
            });
        }
    });
    if let Some(e) = failure.into_inner().unwrap_or(None) {
        return Err(e);
    }
    let items: Vec<Variation> = slots.into_inner().unwrap_or_default().into_iter().flatten().collect();
    if items.len() != count {
        return Err(SolveError::Failed("a variation worker stopped early".into()));
    }
    Ok(Batch { base_seed, coarse, items })
}
