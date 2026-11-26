//! Part 9c: seed variations (pure functions, no session). Run with `cargo test -p wb-app --test part9c_variations`.
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use wb_app::params::defaults;
use wb_app::variations::{params_for, run, variation_seeds, MAX_VARIATIONS, SEED_LIMIT};
use wb_app::world::{self, Params};

fn small() -> Params {
    let mut p = defaults();
    p.insert("resolution".into(), 2.0);
    p.insert("spinup_years".into(), 1.0);
    p
}

#[test]
fn seeds_are_deterministic_distinct_and_valid() {
    let a = variation_seeds(7, 6);
    assert_eq!(a, variation_seeds(7, 6), "same base, same seeds");
    assert_ne!(a, variation_seeds(8, 6), "another base gives other seeds");
    assert_eq!(a.len(), 6);
    let mut sorted = a.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 6, "all distinct");
    assert!(a.iter().all(|&s| s != 7 && s < SEED_LIMIT), "never the base, always a valid seed parameter");
    assert_eq!(variation_seeds(7, 3), a[..3].to_vec(), "asking for fewer gives a prefix");
    assert!(MAX_VARIATIONS >= 6);
}

#[test]
fn six_variations_differ_and_are_reproducible() {
    let base = small();
    let cancel = AtomicBool::new(false);
    let log = Mutex::new(Vec::new());
    let progress = |f: f64, _: &str| log.lock().unwrap().push(f);
    let b1 = run(&base, 6, true, &cancel, &progress).unwrap();
    assert_eq!(b1.items.len(), 6);
    assert_eq!(log.lock().unwrap().len(), 6, "progress after every variation");
    assert!((log.lock().unwrap().iter().cloned().fold(0.0, f64::max) - 1.0).abs() < 1e-12);
    // they differ pairwise (elevation comes from the seed)
    let elev = |b: &wb_app::variations::Batch, i: usize| b.items[i].fields.iter().find(|(n, _)| n == "elevation").unwrap().1.clone();
    for i in 0..6 {
        for j in (i + 1)..6 {
            assert_ne!(elev(&b1, i), elev(&b1, j), "variations {i} and {j} differ");
        }
    }
    // a second run (different thread timing) gives bit-identical results
    let b2 = run(&base, 6, true, &cancel, &|_, _| {}).unwrap();
    for i in 0..6 {
        assert_eq!(b1.items[i].seed, b2.items[i].seed);
        assert_eq!(b1.items[i].fields, b2.items[i].fields, "variation {i} is reproducible");
    }
}

#[test]
fn a_variation_equals_a_plain_solve_with_its_seed() {
    let base = small();
    let cancel = AtomicBool::new(false);
    let batch = run(&base, 2, true, &cancel, &|_, _| {}).unwrap();
    let v = &batch.items[1];
    let mut p = base.clone();
    p.insert("seed".into(), v.seed as f64);
    let solved = world::solve(&p, &cancel, &mut |_, _| {}).unwrap();
    for (name, values) in &v.fields {
        assert_eq!(&solved.fields.data(name).unwrap().to_vec(), values, "{name}");
    }
}

#[test]
fn coarse_only_shortens_spinup() {
    let mut base = defaults();
    base.insert("spinup_years".into(), 4.0);
    assert_eq!(params_for(&base, 5, true)["spinup_years"], 1.0);
    assert_eq!(params_for(&base, 5, false)["spinup_years"], 4.0);
    assert_eq!(params_for(&base, 5, true)["seed"], 5.0);
}

#[test]
fn cancel_stops_the_batch() {
    let base = small();
    let cancel = AtomicBool::new(true);
    assert!(run(&base, 4, true, &cancel, &|_, _| {}).is_err());
}
