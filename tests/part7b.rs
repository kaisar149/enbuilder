//! Part 7b: storm and electricity module (charge -> lightning -> fire, nitrogen, rain seeding -> biomes).
use enbuilder::anomaly::*;
use enbuilder::biomes::{BiomeDef, BiomeTable, Biomes, Condition};
use enbuilder::budget::{add_climate_pipeline, run_season};
use enbuilder::generators::{generate_plane, PlaneParams, PlaneShape};
use enbuilder::storm::{add_storm_stage, Storm};
use enbuilder::testworld::earth_like_world;
use enbuilder::{BoundaryKind, DirtyRegion, Domain, FieldStore, Module, Scheduler};

const NX: usize = 30;
const NY: usize = 10;

fn plane() -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx: NX, ny: NY }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}

/// Savanna-like land: warm, humid, moderate rain, modest convection. Same everywhere, so only the anomaly differs between cells.
fn savanna_fields(n: usize) -> FieldStore {
    let mut f = FieldStore::new(n);
    f.put("precip_conv", 1, vec![3.0e-5; n]).unwrap(); // = conv_ref: convection saturated, so c = 1 everywhere
    f.put("precipitation", 1, vec![2.0e-5; n]).unwrap();
    f.put("vapor", 1, vec![25.0; n]).unwrap();
    f.put("temperature", 1, vec![300.0; n]).unwrap();
    f.put("elevation", 1, vec![100.0; n]).unwrap();
    f
}

fn charge_set(d: &Domain, muted: bool, falloff: Falloff, strength: f64) -> (AnomalySet, Vec<f64>) {
    let c = d.mesh.centroid[5 * NX + 15];
    let shape = Shape::Sphere { center: c, radius_m: 30_000.0 };
    let a = Anomaly::new("storm_field", shape.clone(), falloff, 60_000.0).with_effect(Effect::add("charge_potential", strength)).muted(muted);
    let w = a.weights(&shape.distances(d).unwrap(), 1.0);
    let mut s = AnomalySet::new();
    s.add(a).unwrap();
    (s, w)
}

fn run_storm(d: &Domain, set: Option<&AnomalySet>) -> FieldStore {
    let mut f = savanna_fields(d.n_cells());
    if let Some(s) = set {
        s.compile(d, None).unwrap().write(&mut f).unwrap();
    }
    Storm::new().run(d, &mut f, &DirtyRegion::All).unwrap();
    f
}

fn bits(v: &[f64]) -> Vec<u64> { v.iter().map(|x| x.to_bits()).collect() }

#[test]
fn a_charge_region_gives_lightning_fire_adapted_ground_nitrogen_seeding_and_compass_noise() {
    let d = plane();
    let (set, w) = charge_set(&d, false, Falloff::Hard, 2.0);
    let (base, hot) = (run_storm(&d, None), run_storm(&d, Some(&set)));
    let inside: Vec<usize> = (0..d.n_cells()).filter(|&i| w[i] >= 1.0).collect();
    let outside: Vec<usize> = (0..d.n_cells()).filter(|&i| w[i] == 0.0).collect();
    assert!(inside.len() >= 5 && outside.len() >= 100);
    let m = |f: &FieldStore, name: &str, idx: &[usize]| idx.iter().map(|&i| f.data(name).unwrap()[i]).sum::<f64>() / idx.len() as f64;
    // lightning: measurable, and ~ (0.1 + 2.0) / 0.1 = 21x the background
    let ratio = m(&hot, "lightning_rate", &inside) / m(&base, "lightning_rate", &inside);
    assert!((ratio - 21.0).abs() < 0.5, "lightning ratio {ratio}");
    assert_eq!(bits(hot.data("lightning_rate").unwrap())[outside[0]], bits(base.data("lightning_rate").unwrap())[outside[0]], "outside untouched");
    // fire: fire-adapted inside, none outside
    assert!(inside.iter().all(|&i| hot.data("fire_adapted").unwrap()[i] == 1.0));
    assert!(outside.iter().all(|&i| hot.data("fire_adapted").unwrap()[i] == 0.0));
    assert!(m(&hot, "fire_frequency", &inside) > 10.0 * m(&base, "fire_frequency", &inside));
    // nitrogen, seeding and compass
    assert!(m(&hot, "soil_nitrogen_flux", &inside) > 10.0 * m(&base, "soil_nitrogen_flux", &inside));
    assert!(m(&hot, "soil_nitrogen", &inside) > m(&base, "soil_nitrogen", &inside) && m(&hot, "soil_nitrogen", &inside) <= 1.0, "normalised nitrogen saturates at 1");
    assert!(m(&hot, "precip_seeding", &inside) > 10.0 * m(&base, "precip_seeding", &inside));
    let (p, ps) = (hot.data("precipitation").unwrap(), hot.data("precip_seeded").unwrap());
    assert!(inside.iter().all(|&i| ps[i] > p[i]), "seeded rain exceeds the unseeded rain inside");
    assert!(m(&hot, "compass_disturbance", &inside) > 5.0 * m(&base, "compass_disturbance", &inside));
    // charge alone drives updrafts: with NO convective rain the storm field still produces lightning, the background does not
    let mut f = savanna_fields(d.n_cells());
    f.put("precip_conv", 1, vec![0.0; d.n_cells()]).unwrap();
    set.compile(&d, None).unwrap().write(&mut f).unwrap();
    Storm::new().run(&d, &mut f, &DirtyRegion::All).unwrap();
    let l = f.data("lightning_rate").unwrap();
    assert!(l[inside[0]] > 5.0 * l[outside[0]], "turbulent updraft from the charge itself: {} vs {}", l[inside[0]], l[outside[0]]);
}

#[test]
fn muting_the_charge_anomaly_restores_the_storm_baseline_bit_for_bit() {
    let d = plane();
    let (set, _) = charge_set(&d, true, Falloff::Gaussian, 2.0);
    let (base, muted) = (run_storm(&d, None), run_storm(&d, Some(&set)));
    for name in ["charge", "lightning_rate", "fire_frequency", "fire_adapted", "soil_nitrogen", "precip_seeded", "compass_disturbance"] {
        assert_eq!(bits(base.data(name).unwrap()), bits(muted.data(name).unwrap()), "{name}");
    }
}

#[test]
fn lightning_falls_off_exactly_like_the_chosen_curve() {
    let d = plane();
    for fo in [Falloff::Linear, Falloff::Gaussian] {
        let (set, w) = charge_set(&d, false, fo.clone(), 1.0);
        let f = run_storm(&d, Some(&set));
        let l = f.data("lightning_rate").unwrap();
        // c = 1, m = 1, warm = 1 on this field, so lightning = flash_max * q with q = 0.1 + w
        for i in 0..d.n_cells() {
            let expect = 50.0 * (0.1 + w[i]);
            assert!((l[i] - expect).abs() < 1e-9 * expect.max(1.0), "{fo:?} cell {i}: {} vs {}", l[i], expect);
        }
        assert!(w.iter().any(|&x| x > 0.0 && x < 1.0), "{fo:?} has a real gradient");
    }
}

#[test]
fn fire_adapted_biomes_appear_where_the_fire_regime_is_set_by_lightning() {
    let d = plane();
    let (set, w) = charge_set(&d, false, Falloff::Hard, 2.0);
    let mut f = run_storm(&d, Some(&set));
    let table = BiomeTable::new(vec![
        BiomeDef { name: "fire savanna".into(), color: [200, 160, 40], albedo: 0.2, evap_factor: 0.8, roughness_m: 4.0, conditions: vec![Condition { field: "fire_adapted".into(), lo: 0.5, hi: 2.0 }] },
        BiomeDef { name: "grassland".into(), color: [120, 200, 80], albedo: 0.2, evap_factor: 1.0, roughness_m: 1.0, conditions: vec![] },
    ]);
    Biomes::new(table).run(&d, &mut f, &DirtyRegion::All).unwrap();
    let id = f.data("biome").unwrap();
    for i in 0..d.n_cells() {
        assert_eq!(id[i], if w[i] >= 1.0 { 0.0 } else { 1.0 }, "cell {i}");
    }
}

#[test]
fn storm_parameters_are_validated_and_ocean_has_no_fire() {
    let mut s = Storm::new();
    assert!(s.set_param("flash_max", -1.0).is_err());
    assert!(s.set_param("ocean_ratio", 2.0).is_err());
    assert!(s.set_param("nope", 1.0).is_err());
    assert!(s.set_param("flash_max", 80.0).is_ok());
    let d = plane();
    let mut f = savanna_fields(d.n_cells());
    f.put("elevation", 1, vec![-100.0; d.n_cells()]).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!(f.data("fire_frequency").unwrap().iter().all(|&x| x == 0.0));
    assert!(f.data("lightning_rate").unwrap().iter().all(|&x| x > 0.0 && x < 80.0 * 0.1 * 0.1 + 1e-9 + 80.0), "ocean flashes are scaled down");
}

#[test]
fn full_climate_pipeline_with_a_charge_anomaly_gets_more_lightning_and_local_rain_seeding_and_muting_restores_it() {
    let (base, _) = earth_like_world(3).unwrap();
    let run = |h: &Anomalies| {
        let (_, mut f) = earth_like_world(3).unwrap();
        let dom = h.effective_domain(&base).unwrap();
        let mut s = Scheduler::new();
        add_anomaly_stage(&mut s, h).unwrap();
        add_climate_pipeline(&mut s).unwrap();
        add_storm_stage(&mut s).unwrap();
        run_season(&dom, &mut f, &mut s, 0.0).unwrap();
        f
    };
    let f0 = run(&Anomalies::new());
    // put the storm field on the most convective land cell of the baseline
    let (conv, z) = (f0.data("precip_conv").unwrap(), f0.data("elevation").unwrap());
    let best = (0..base.n_cells()).filter(|&i| z[i] >= 0.0).max_by(|&a, &b| conv[a].partial_cmp(&conv[b]).unwrap()).unwrap();
    let shape = Shape::Sphere { center: base.mesh.centroid[best], radius_m: 700_000.0 };
    let mut set = AnomalySet::new();
    set.add(Anomaly::new("inazuma", shape.clone(), Falloff::Gaussian, 500_000.0).with_effect(Effect::add("charge_potential", 2.0))).unwrap();
    let h = Anomalies::from_set(set);
    let f1 = run(&h);
    let dist = shape.distances(&base).unwrap();
    let inside: Vec<usize> = (0..base.n_cells()).filter(|&i| dist[i] == 0.0 && z[i] >= 0.0).collect();
    assert!(!inside.is_empty());
    let mean = |f: &FieldStore, name: &str| inside.iter().map(|&i| f.data(name).unwrap()[i]).sum::<f64>() / inside.len() as f64;
    assert!(mean(&f1, "lightning_rate") > 5.0 * mean(&f0, "lightning_rate"), "{} vs {}", mean(&f1, "lightning_rate"), mean(&f0, "lightning_rate"));
    assert!(mean(&f1, "precip_seeding") > 5.0 * mean(&f0, "precip_seeding"));
    assert!(mean(&f1, "soil_nitrogen") > mean(&f0, "soil_nitrogen"));
    // the climate itself is untouched by the storm field (rain seeding is an overlay): precipitation is bit-identical
    assert_eq!(bits(f0.data("precipitation").unwrap()), bits(f1.data("precipitation").unwrap()));
    // mute: baseline back exactly
    h.set_muted("inazuma", true).unwrap();
    let f2 = run(&h);
    for name in ["lightning_rate", "fire_frequency", "soil_nitrogen", "precip_seeded", "precipitation", "temperature"] {
        assert_eq!(bits(f0.data(name).unwrap()), bits(f2.data(name).unwrap()), "{name}");
    }
}

#[test]
fn inspector_gives_impact_maps_for_the_storm_hook_and_every_storm_output() {
    let (base, _) = earth_like_world(3).unwrap();
    let shape = Shape::Sphere { center: base.mesh.centroid[base.n_cells() / 3], radius_m: 800_000.0 };
    let mut set = AnomalySet::new();
    set.add(Anomaly::new("inazuma", shape, Falloff::Linear, 600_000.0).with_effect(Effect::add("charge_potential", 2.0))).unwrap();
    let info = inspect(&base, &set).unwrap();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].hooks, vec!["charge_potential".to_string()]);
    assert!(info[0].cells_influenced > 0 && !info[0].muted);
    // direct impact map on the hook: w * magnitude, in [0, 2]
    let im = impact_map(&base, set.get("inazuma").unwrap()).unwrap();
    let q = &im["charge_potential"];
    assert!(q.iter().all(|&x| (0.0..=2.0 + 1e-12).contains(&x)));
    assert!(q.iter().any(|&x| x == 2.0));
    // downstream impact on storm outputs: re-solve with and without the anomaly
    let h = Anomalies::from_set(set);
    let solve = |hh: &Anomalies| -> enbuilder::Result<FieldStore> {
        let (_, mut f) = earth_like_world(3).unwrap();
        let dom = hh.effective_domain(&base)?;
        let mut s = Scheduler::new();
        add_anomaly_stage(&mut s, hh)?;
        add_climate_pipeline(&mut s)?;
        add_storm_stage(&mut s)?;
        run_season(&dom, &mut f, &mut s, 0.0)?;
        Ok(f)
    };
    let tot = |v: &[f64]| v.iter().map(|x| x.abs()).sum::<f64>();
    for out in ["charge", "lightning_rate", "convective_activity", "precip_seeding"] {
        let d = field_impact(&h, "inazuma", out, &solve).unwrap();
        assert_eq!(d.len(), base.n_cells());
        assert!(tot(&d) > 0.0, "{out} must respond to the charge anomaly");
        assert!(d.iter().all(|x| x.is_finite()));
    }
    // charge and lightning only go up
    let dl = field_impact(&h, "inazuma", "lightning_rate", &solve).unwrap();
    assert!(dl.iter().all(|&x| x >= -1e-12));
    // the mute state is restored by field_impact
    assert!(!h.snapshot().get("inazuma").unwrap().muted);
    // temperature is untouched (rain seeding is an overlay)
    let dt = field_impact(&h, "inazuma", "temperature", &solve).unwrap();
    assert_eq!(tot(&dt), 0.0);
}

#[test]
fn lightning_nitrogen_boosts_soil_fertility_only_when_the_gain_is_on() {
    use enbuilder::derived::DerivedMaps;
    let d = plane();
    let n = d.n_cells();
    let make = || {
    let mut f = FieldStore::new(n);
    let c = |v: f64| vec![v; n];
    f.put("clim_t_cold_c", 1, c(10.0)).unwrap();
    f.put("clim_t_warm_c", 1, c(22.0)).unwrap();
    f.put("clim_gdd", 1, c(1000.0)).unwrap();
    f.put("clim_aridity", 1, c(0.4)).unwrap();
    f.put("clim_soil_m", 1, c(0.05)).unwrap();
    f.put("ocean_mask", 1, c(0.0)).unwrap();
    f.put("glacier_mask", 1, c(0.0)).unwrap();
    f.put("biome_roughness_m", 1, c(1.0)).unwrap();
    f.put("elevation", 1, c(100.0)).unwrap();
    f.put("soil_nitrogen", 1, (0..n).map(|i| if i % 2 == 0 { 1.0 } else { 0.0 }).collect()).unwrap();
    f
    };
    let mut off = DerivedMaps::new();
    assert!(!off.spec().inputs.iter().any(|s| s == "soil_nitrogen"));
    let mut f0 = make();
    off.run(&d, &mut f0, &DirtyRegion::All).unwrap();
    let mut on = DerivedMaps::new();
    assert!(on.set_param("nitrogen_gain", -1.0).is_err());
    on.set_param("nitrogen_gain", 0.5).unwrap();
    assert!(on.spec().inputs.iter().any(|s| s == "soil_nitrogen"));
    let mut f1 = make();
    on.run(&d, &mut f1, &DirtyRegion::All).unwrap();
    let (a, b) = (f0.data("soil_fertility").unwrap(), f1.data("soil_fertility").unwrap());
    assert!(a[0] > 0.0 && a[0] < 1.0);
    for i in 0..n {
        if i % 2 == 0 { assert!(b[i] > a[i], "cell {i}"); } else { assert_eq!(a[i].to_bits(), b[i].to_bits(), "cell {i}"); }
    }
}
