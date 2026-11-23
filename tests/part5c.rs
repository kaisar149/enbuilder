//! Part 5c acceptance tests: derived land maps (soil fertility, habitability, travel difficulty).
//! NOT compiled when written. Hand numbers come from `reference/mirror_5c.py`.
use enbuilder::derived::*;
use enbuilder::generators::*;
use enbuilder::*;

fn plane(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn near(a: f64, b: f64, tol: f64) -> bool { (a - b).abs() <= tol }

#[test]
fn ramp_clamps_and_nan_is_zero() {
    assert_eq!(ramp(5.0, 0.0, 10.0), 0.5);
    assert_eq!(ramp(-1.0, 0.0, 10.0), 0.0);
    assert_eq!(ramp(11.0, 0.0, 10.0), 1.0);
    assert_eq!(ramp(f64::NAN, 0.0, 1.0), 0.0);
}

#[test]
fn the_three_scores_match_the_hand_numbers() {
    let p = DerivedParams::default();
    assert!(near(soil_fertility(&p, 1.5, 3000.0, 0.1), 1.0, 1e-12));
    assert!(near(soil_fertility(&p, 0.55, 1400.0, 0.05), 0.1875, 1e-12));
    assert_eq!(soil_fertility(&p, 0.05, 3000.0, 0.1), 0.0, "too dry");
    assert_eq!(soil_fertility(&p, 1.5, 100.0, 0.1), 0.0, "too cold to grow");
    assert!(near(habitability(&p, 1.0, 10.0, 25.0, 1.0), 1.0, 1e-12));
    assert!(near(habitability(&p, 0.5, -20.0, 40.0, 0.2275), 0.136538, 1e-6));
    assert_eq!(habitability(&p, 0.0, -40.0, 5.0, 1.0), 0.0, "arctic cold");
    assert_eq!(habitability(&p, 1.0, 20.0, 47.0, 1.0), 0.0, "lethal heat");
    assert!(near(travel_difficulty(&p, 0.1, 15.0, -20.0, 40.0, false), 2.65, 1e-12));
    assert!(near(travel_difficulty(&p, 0.0, 0.0, -30.0, 2.0, true), 4.0, 1e-12));
    assert_eq!(travel_difficulty(&p, 0.0, 0.0, 5.0, 25.0, false), 1.0);
    assert_eq!(travel_difficulty(&p, 2.0, 0.0, 5.0, 25.0, false), 5.0, "slope term capped at 4");
}

fn fields_for(d: &Domain, z: Vec<f64>) -> FieldStore {
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    for (k, v) in [("clim_t_cold_c", 10.0), ("clim_t_warm_c", 25.0), ("clim_gdd", 3000.0), ("clim_aridity", 1.5), ("clim_soil_m", 0.1), ("biome_roughness_m", 0.0)] {
        f.put(k, 1, vec![v; n]).unwrap();
    }
    f.put("ocean_mask", 1, (0..n).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect()).unwrap();
    f.put("glacier_mask", 1, (0..n).map(|i| if i == 1 { 1.0 } else { 0.0 }).collect()).unwrap();
    f.put("elevation", 1, z).unwrap();
    f
}

#[test]
fn the_module_zeroes_ocean_and_ice_and_scores_land() {
    let d = plane(6, 4);
    let n = d.n_cells();
    let mut f = fields_for(&d, vec![100.0; n]);
    let mut s = Scheduler::new();
    add_derived_stage(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let (fe, ha, tr) = (f.data("soil_fertility").unwrap().to_vec(), f.data("habitability").unwrap().to_vec(), f.data("travel_difficulty").unwrap().to_vec());
    assert!(fe[0] == 0.0 && ha[0] == 0.0 && tr[0] == 0.0, "ocean cell");
    assert!(fe[1] == 0.0 && ha[1] == 0.0 && tr[1] == 3.0, "glacier cell: flat, 2 for ice and 0 else; clim_t_cold 10 adds nothing: 1 + 2");
    for i in 2..n {
        assert!(near(fe[i], 1.0, 1e-12) && near(ha[i], 1.0, 1e-12) && near(tr[i], 1.0, 1e-12), "cell {i}");
    }
}

#[test]
fn travel_difficulty_follows_the_slope_of_the_mesh() {
    let d = plane(6, 4);
    let n = d.n_cells();
    let z: Vec<f64> = (0..n).map(|i| 400.0 * (i % 6) as f64).collect();
    let slope = mean_slope(&d, &z, 0.0);
    // independent check of the first land cell from the mesh's own adjacency
    let i = 8;
    let (nb, dist) = (d.mesh.neighbors(i), d.mesh.edge_dists(i));
    let expect: f64 = nb.iter().zip(dist).map(|(&j, &dd)| (z[j as usize] - z[i]).abs() / dd).sum::<f64>() / nb.len() as f64;
    assert!(near(slope[i], expect, 1e-12) && slope[i] > 0.0);
    let mut flat = fields_for(&d, vec![100.0; n]);
    let mut hill = fields_for(&d, z);
    for f in [&mut flat, &mut hill] {
        let mut s = Scheduler::new();
        add_derived_stage(&mut s).unwrap();
        s.run(&d, f, &DirtyRegion::All).unwrap();
    }
    let (tf, th) = (flat.data("travel_difficulty").unwrap()[i], hill.data("travel_difficulty").unwrap()[i]);
    assert!(near(th - tf, (4.0 * slope[i]).min(4.0), 1e-12), "{th} {tf}");
    // the sea floor does not count as relief: everything below sea level is clipped to it
    assert!(mean_slope(&d, &vec![-500.0; n], 0.0).iter().all(|&s| s == 0.0));
}

#[test]
fn parameters_are_validated_and_change_the_output() {
    let mut m = DerivedMaps::new();
    assert!(m.set_param("nonsense", 1.0).is_err());
    assert!(m.set_param("gdd_lo", f64::NAN).is_err());
    assert!(m.set_param("aridity_lo", 2.0).is_err(), "lo must stay below hi");
    assert!(m.set_param("canopy_ref_m", 0.0).is_err());
    assert!(m.set_param("slope_gain", -1.0).is_err());
    assert_eq!(m.params().get("aridity_lo"), Some(0.1), "a refused value leaves the parameter alone");
    assert!(m.set_param("aridity_hi", 2.0).is_ok());
    let d = plane(6, 4);
    let n = d.n_cells();
    let run = |hi: f64| {
        let mut f = fields_for(&d, vec![100.0; n]);
        f.put("clim_aridity", 1, vec![0.55; n]).unwrap();
        let mut s = Scheduler::new();
        add_derived_stage(&mut s).unwrap();
        s.set_param("derived_maps", "aridity_hi", hi).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        f.data("soil_fertility").unwrap()[5]
    };
    assert!(near(run(1.0), 0.5, 1e-12) && near(run(0.55), 1.0, 1e-12), "a lower wet limit makes the same climate more fertile");
}

#[test]
fn it_chains_behind_the_biome_stage_and_a_missing_input_is_an_error() {
    let d = plane(4, 4);
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("season_temperature", 4, vec![290.0; n * 4]).unwrap();
    f.put("season_precipitation", 4, vec![4e-5; n * 4]).unwrap();
    f.put("season_soil_moisture", 4, vec![0.1; n * 4]).unwrap();
    f.put("ocean_mask", 1, vec![0.0; n]).unwrap();
    f.put("glacier_mask", 1, vec![0.0; n]).unwrap();
    // without elevation the scheduler refuses
    let mut s = Scheduler::new();
    add_biome_stage(&mut s, &BiomeTable::earth_like()).unwrap();
    add_derived_stage(&mut s).unwrap();
    assert!(matches!(s.run(&d, &mut f, &DirtyRegion::All), Err(Error::MissingInput { .. })));
    f.put("elevation", 1, vec![50.0; n]).unwrap();
    let mut s = Scheduler::new();
    add_biome_stage(&mut s, &BiomeTable::earth_like()).unwrap();
    add_derived_stage(&mut s).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    // 17 C, 1260 mm/yr: wet and mild -> fertile, habitable, easy travel
    assert!(f.data("soil_fertility").unwrap().iter().all(|&v| v > 0.3 && v <= 1.0));
    assert!(f.data("habitability").unwrap().iter().all(|&v| v > 0.3 && v <= 1.0));
    assert!(f.data("travel_difficulty").unwrap().iter().all(|&v| v >= 1.0));
}
