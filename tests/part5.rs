//! Part 5a acceptance tests: seasonal normals, Koppen classes, the biome table as data, the biome map.
//! NOT compiled when written (no Rust toolchain in the sandbox). Every hand number and band threshold comes from
//! `reference/mirror_5a.py` and `reference/cases_5a.py` (`python mirror_5a.py`, `python cases_5a.py`).
use enbuilder::biomes::{KELVIN, SEASON_YEAR_S};
use enbuilder::generators::*;
use enbuilder::testworld::earth_like_world;
use enbuilder::*;

fn close(a: f64, b: f64, rel: f64) -> bool { (a - b).abs() <= rel * a.abs().max(b.abs()).max(1e-300) }

fn plane(nx: usize, ny: usize) -> Domain {
    generate_plane(&PlaneParams { shape: PlaneShape::Rect { nx, ny }, cell_size: 10_000.0, boundary: BoundaryKind::Wall, surface_gravity: 9.81, seed: 1 }).unwrap()
}
fn sphere(level: u32) -> Domain { generate_sphere(&SphereParams { radius: 6.371e6, subdivision: level, surface_gravity: 9.81, seed: 7 }).unwrap() }

/// Four seasons: temperatures in deg C and precipitation in mm per month -> the field units the modules read.
fn kc(tc: [f64; 4], pm: [f64; 4]) -> &'static str {
    let t: Vec<f64> = tc.iter().map(|x| x + KELVIN).collect();
    let p: Vec<f64> = pm.iter().map(|m| m * 12.0 / SEASON_YEAR_S).collect();
    KOPPEN_CODES[koppen_class(&t, &p, SEASON_YEAR_S)]
}

// ---------------------------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------------------------

#[test]
fn warm_half_picks_the_warmest_seasons_in_either_hemisphere() {
    assert_eq!(warm_half(&[1.0, 3.0, 2.0, 0.0]), vec![false, true, true, false]);
    assert_eq!(warm_half(&[3.0, 0.0, 1.0, 2.0]), vec![true, false, false, true]); // southern order
    assert_eq!(warm_half(&[5.0, 5.0, 5.0, 5.0]), vec![true, true, false, false]); // ties: lower index
    assert_eq!(warm_half(&[7.0]), vec![true]);
    assert_eq!(warm_half(&[1.0, 9.0, 4.0]), vec![false, true, true]); // ceil(3/2) = 2
}

#[test]
fn normals_hand_numbers() {
    let p = NormalsParams::default();
    // T = 0, 10, 20, 10 C; 2 mm/day all year
    let t: Vec<f64> = [0.0, 10.0, 20.0, 10.0].iter().map(|x| x + KELVIN).collect();
    let r = season_normals(&t, &[2.0 / 86400.0; 4], &p);
    assert!(close(r.t_mean_c, 10.0, 1e-12) && close(r.t_warm_c, 20.0, 1e-12) && close(r.t_range_k, 20.0, 1e-9));
    assert!(r.t_cold_c.abs() < 1e-9);
    assert!(close(r.p_annual, 730.5, 1e-9) && close(r.p_dry, 730.5, 1e-9) && close(r.p_wet, 730.5, 1e-9));
    assert!(close(r.p_summer_frac, 0.5, 1e-12));
    assert_eq!(r.months_warm, 3.0); // only the 20 C season is above 10 C (10 C itself is not)
    assert!(close(r.gdd, 2282.8125, 1e-9));
    assert!(close(r.pet, 559.7295, 1e-6) && close(r.aridity, 1.3051, 1e-3));
    // T = -5, 15, 25, 5 C; 1, 2, 6, 3 mm/day: the warm half is seasons 1 and 2
    let t: Vec<f64> = [-5.0, 15.0, 25.0, 5.0].iter().map(|x| x + KELVIN).collect();
    let pr: Vec<f64> = [1.0, 2.0, 6.0, 3.0].iter().map(|x| x / 86400.0).collect();
    let r = season_normals(&t, &pr, &p);
    assert!(close(r.p_annual, 1095.75, 1e-9) && close(r.p_dry, 365.25, 1e-9) && close(r.p_wet, 2191.5, 1e-9));
    assert!(close(r.p_summer_frac, 8.0 / 12.0, 1e-12));
    assert_eq!(r.months_warm, 6.0);
    assert!(close(r.gdd, 2739.375, 1e-9) && close(r.pet, 659.6818, 1e-6));
    // negative rates count as zero, a dry cell has summer fraction 0.5 and aridity 0
    let r = season_normals(&t, &[-1.0; 4], &p);
    assert_eq!(r.p_annual, 0.0);
    assert_eq!(r.p_summer_frac, 0.5);
    assert_eq!(r.aridity, 0.0);
}

#[test]
fn koppen_hand_cases() {
    assert_eq!(kc([26.0, 27.0, 26.0, 27.0], [250.0, 200.0, 220.0, 240.0]), "Af");
    assert_eq!(kc([26.0, 27.0, 26.0, 27.0], [400.0, 20.0, 60.0, 420.0]), "Am");
    assert_eq!(kc([25.0, 27.0, 25.0, 27.0], [5.0, 200.0, 5.0, 200.0]), "Aw");
    assert_eq!(kc([20.0, 32.0, 20.0, 32.0], [5.0, 10.0, 5.0, 10.0]), "BWh");
    assert_eq!(kc([20.0, 30.0, 20.0, 30.0], [20.0, 50.0, 20.0, 50.0]), "BSh");
    assert_eq!(kc([-5.0, 20.0, -5.0, 20.0], [8.0, 8.0, 8.0, 8.0]), "BWk");
    assert_eq!(kc([-2.0, 22.0, -2.0, 22.0], [15.0, 25.0, 15.0, 25.0]), "BSk");
    assert_eq!(kc([5.0, 15.0, 6.0, 16.0], [60.0, 70.0, 60.0, 70.0]), "Cfb");
    assert_eq!(kc([10.0, 25.0, 12.0, 24.0], [90.0, 5.0, 95.0, 5.0]), "Csa");
    assert_eq!(kc([8.0, 26.0, 9.0, 25.0], [5.0, 200.0, 6.0, 180.0]), "Cwa");
    assert_eq!(kc([-12.0, 14.0, -10.0, 15.0], [40.0, 70.0, 40.0, 70.0]), "Dfb");
    assert_eq!(kc([-20.0, 12.0, -18.0, 2.0], [30.0, 50.0, 30.0, 50.0]), "Dfc");
    assert_eq!(kc([-15.0, 24.0, -14.0, 23.0], [5.0, 150.0, 6.0, 140.0]), "Dwa");
    assert_eq!(kc([-15.0, 6.0, -14.0, 5.0], [30.0, 30.0, 30.0, 30.0]), "ET");
    assert_eq!(kc([-30.0, -5.0, -28.0, -6.0], [10.0, 10.0, 10.0, 10.0]), "EF");
}

#[test]
fn koppen_works_for_one_season_and_for_twelve() {
    let y = SEASON_YEAR_S;
    assert_eq!(KOPPEN_CODES[koppen_class(&[27.0 + KELVIN], &[2400.0 / y], y)], "Af");
    assert_eq!(KOPPEN_CODES[koppen_class(&[30.0 + KELVIN], &[100.0 / y], y)], "BWh");
    let t = vec![27.0 + KELVIN; 12];
    let p = vec![200.0 * 12.0 / y; 12];
    assert_eq!(KOPPEN_CODES[koppen_class(&t, &p, y)], "Af");
    // groups
    assert_eq!(koppen_group(0), 0);
    assert_eq!(koppen_group(KOPPEN_CODES.iter().position(|&c| c == "Csa").unwrap()), 3);
    assert_eq!(koppen_group(KOPPEN_CODES.iter().position(|&c| c == "EF").unwrap()), 5);
    assert_eq!(KOPPEN_CODES.len(), 31);
}

#[test]
fn koppen_is_hemisphere_independent() {
    // the same year shifted by two seasons (south) gives the same class
    let t = [10.0, 25.0, 12.0, 24.0];
    let p = [90.0, 5.0, 95.0, 5.0];
    let rot = |a: [f64; 4]| [a[2], a[3], a[0], a[1]];
    assert_eq!(kc(t, p), kc(rot(t), rot(p)));
}

// ---------------------------------------------------------------------------------------------
// The table as data
// ---------------------------------------------------------------------------------------------

#[test]
fn earth_table_parses_round_trips_and_starts_with_the_ocean() {
    let t = BiomeTable::earth_like();
    assert_eq!(t.name(0), "ocean");
    assert_eq!(t.name(t.len()), "unclassified");
    assert!(t.len() >= 14);
    let again = BiomeTable::parse(&t.to_text()).unwrap();
    assert_eq!(again, t);
    let fields = t.fields();
    for f in ["ocean_mask", "glacier_mask", "clim_aridity", "clim_t_mean_c", "clim_t_cold_c", "clim_t_warm_c", "clim_p_summer_frac"] {
        assert!(fields.contains(f), "default table should read {f}");
    }
    // every field the default table reads is either produced by climate_normals or supplied by the pipeline
    for f in &fields {
        assert!(NORMAL_FIELDS.contains(&f.as_str()) || f == "ocean_mask" || f == "glacier_mask", "unexpected field {f}");
    }
}

#[test]
fn table_parser_rejects_bad_input_with_line_numbers() {
    let bad = [
        "  x < 1\n",
        "biome a\nbiome a\n",
        "biome a colour=#ffffff\n",
        "biome a color=ffffff\n",
        "biome a color=#fffff\n",
        "biome a albedo=1.5\n",
        "biome a evap=-1\n",
        "biome a rough=abc\n",
        "biome a\n  x > 1\n",
        "biome a\n  x <= 1\n",
        "biome a\n  x in 5..3\n",
        "biome a\n  x in 5\n",
        "biome a\n  x >=\n",
        "biome a\n  x>=1\n",
        "biome a\n  x-y < 1\n",
        "biome\n",
    ];
    for text in bad {
        match BiomeTable::parse(text) {
            Err(Error::InvalidParam(m)) => assert!(m.contains("line"), "message should name the line: {m}"),
            other => panic!("expected an error for {text:?}, got {other:?}"),
        }
    }
    // comments, blank lines and the '#' of a color are handled
    let ok = BiomeTable::parse("# header\n\nbiome a color=#102030 albedo=0.3 # trailing note\n  x in -inf..2 # more\n").unwrap();
    assert_eq!(ok.biomes[0].color, [0x10, 0x20, 0x30]);
    assert_eq!(ok.biomes[0].conditions, vec![Condition { field: "x".into(), lo: f64::NEG_INFINITY, hi: 2.0 }]);
    assert_eq!(BiomeTable::parse("").unwrap().len(), 0);
}

#[test]
fn first_match_wins_and_no_match_is_unclassified() {
    let t = BiomeTable::parse("biome low\n  a < 1\nbiome mid\n  a in 1..2\nbiome mid_but_never\n  a in 1..2\n").unwrap();
    let get = |v: f64| move |_: &str| v;
    assert_eq!(t.classify(&get(0.5)), 0);
    assert_eq!(t.classify(&get(1.0)), 1, "lower bound is inclusive");
    assert_eq!(t.classify(&get(2.0)), 3, "upper bound is exclusive, nothing matches -> len()");
    assert_eq!(t.classify(&get(f64::NAN)), 3, "NaN matches no condition");
    assert_eq!(t.id_of("mid"), Some(1));
    // an empty condition list is a catch-all
    let c = BiomeTable::parse("biome low\n  a < 1\nbiome rest\n").unwrap();
    assert_eq!(c.classify(&get(50.0)), 1);
}

// ---------------------------------------------------------------------------------------------
// Modules on hand-built fields
// ---------------------------------------------------------------------------------------------

const CUSTOM: &str = "\
biome low   color=#ff0000 albedo=0.1 evap=0.5 rough=2
  x < 1
biome mid   color=#00ff00 albedo=0.3 evap=1.5 rough=4
  x in 1..2
  y >= 0
biome high  color=#0000ff
  x >= 2
";

fn custom_fields(d: &Domain) -> FieldStore {
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("x", 1, (0..n).map(|i| (i % 4) as f64 * 0.75).collect()).unwrap();
    f.put("y", 1, (0..n).map(|i| if i < 12 { 1.0 } else { -1.0 }).collect()).unwrap();
    f
}

#[test]
fn biome_module_applies_the_table_and_the_fallbacks() {
    let d = plane(6, 4);
    let mut f = custom_fields(&d);
    let table = BiomeTable::parse(CUSTOM).unwrap();
    let mut s = Scheduler::new();
    s.register(Box::new(Biomes::new(table.clone()))).unwrap();
    s.set_param("biomes", "fallback_albedo", 0.4).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    let id = f.data("biome").unwrap().to_vec();
    let (al, ev, ro) = (f.data("biome_albedo").unwrap().to_vec(), f.data("biome_evap_factor").unwrap().to_vec(), f.data("biome_roughness_m").unwrap().to_vec());
    assert_eq!(f.layers("biome_color").unwrap(), 3);
    let col = f.data("biome_color").unwrap().to_vec();
    for i in 0..d.n_cells() {
        let x = (i % 4) as f64 * 0.75;
        let expect = if x < 1.0 {
            0
        } else if x < 2.0 && i < 12 {
            1
        } else if x >= 2.0 {
            2
        } else {
            3
        };
        assert_eq!(id[i] as usize, expect, "cell {i}");
        match expect {
            0 => assert!(al[i] == 0.1 && ev[i] == 0.5 && ro[i] == 2.0 && col[i * 3] == 1.0 && col[i * 3 + 1] == 0.0),
            1 => assert!(al[i] == 0.3 && ev[i] == 1.5 && ro[i] == 4.0 && col[i * 3 + 1] == 1.0),
            2 => assert!(al[i] == 0.22 && ev[i] == 1.0 && ro[i] == 0.0 && col[i * 3 + 2] == 1.0), // defaults of a bare biome line
            _ => assert!(al[i] == 0.4 && ev[i] == 1.0 && ro[i] == 0.0 && col[i * 3] == 0.5, "unclassified cells take the fallbacks"),
        }
    }
    // x = 1.5 (i % 4 == 2) with y < 0 (i >= 12) matches nothing: cells 14, 18, 22
    assert_eq!(s.module_param("biomes", "n_unclassified"), Some(3.0));
    assert_eq!(s.module_param("biomes", "n_biomes_present"), Some(4.0));
}

#[test]
fn only_the_first_layer_of_a_layered_field_is_read() {
    let d = plane(3, 3);
    let mut f = FieldStore::new(d.n_cells());
    f.put("z", 2, (0..d.n_cells() * 2).map(|k| if k % 2 == 0 { 1.0 } else { 99.0 }).collect()).unwrap();
    let table = BiomeTable::parse("biome small\n  z < 5\n").unwrap();
    let mut s = Scheduler::new();
    s.register(Box::new(Biomes::new(table))).unwrap();
    s.run(&d, &mut f, &DirtyRegion::All).unwrap();
    assert!(f.data("biome").unwrap().iter().all(|&v| v == 0.0));
}

#[test]
fn editing_the_table_changes_the_map_with_no_code_change() {
    let d = plane(6, 4);
    let base = BiomeTable::parse(CUSTOM).unwrap();
    let run = |t: &BiomeTable| -> (Vec<f64>, Vec<f64>) {
        let mut f = custom_fields(&d);
        let mut s = Scheduler::new();
        s.register(Box::new(Biomes::new(t.clone()))).unwrap();
        s.run(&d, &mut f, &DirtyRegion::All).unwrap();
        (f.data("biome").unwrap().to_vec(), f.data("biome_albedo").unwrap().to_vec())
    };
    let (id0, al0) = run(&base);
    // move the threshold: x < 1  becomes  x < 0.5 (x = 0.75 moves from "low" to "mid"/unclassified)
    let edited = BiomeTable::parse(&base.to_text().replace("x < 1\n", "x < 0.5\n")).unwrap();
    let (id1, _) = run(&edited);
    assert_ne!(id0, id1);
    // change an effect only
    let bright = BiomeTable::parse(&base.to_text().replace("albedo=0.1 ", "albedo=0.5 ")).unwrap();
    let (id2, al2) = run(&bright);
    assert_eq!(id0, id2);
    assert_ne!(al0, al2);
    assert!(al2.iter().zip(&id2).all(|(a, i)| *i != 0.0 || *a == 0.5));
    // a catch-all first row takes everything
    let all = BiomeTable::parse(&format!("biome everything\n{}", base.to_text())).unwrap();
    let (id3, _) = run(&all);
    assert!(id3.iter().all(|&v| v == 0.0));
}

#[test]
fn module_interfaces_and_parameters() {
    let b = Biomes::new(BiomeTable::parse(CUSTOM).unwrap());
    assert_eq!(b.spec().inputs, vec!["x".to_string(), "y".to_string()]);
    let mut b = Biomes::earth_like();
    assert!(b.set_param("n_unclassified", 1.0).is_err(), "output-only");
    assert!(b.set_param("nonsense", 1.0).is_err());
    assert!(b.set_param("fallback_albedo", 1.5).is_err());
    assert!(b.set_param("fallback_evap_factor", -1.0).is_err());
    assert!(b.set_param("fallback_roughness_m", 3.0).is_ok());
    let mut n = ClimateNormals::new();
    assert!(n.set_param("pet_slope", -1.0).is_err() && n.set_param("year_s", 0.0).is_err() && n.set_param("bogus", 1.0).is_err());
    assert!(n.set_param("pet_exp", 1.0).is_ok());
    let mut k = Koppen::new();
    assert!(k.set_param("year_s", 3.0e7).is_ok() && k.set_param("year_s", -1.0).is_err() && k.set_param("other", 1.0).is_err());
    assert_eq!(ClimateNormals::new().spec().outputs.len(), 13);
    assert_eq!(NormalsParams::default(), NormalsParams { pet_slope: 0.09, pet_exp: 1.2, year_s: 3.15576e7 });
}

#[test]
fn a_missing_input_or_mismatched_layers_is_an_error_not_a_guess() {
    let d = plane(4, 4);
    let n = d.n_cells();
    // default table needs glacier_mask (and the normals): nothing supplied -> MissingInput from the scheduler
    let mut f = FieldStore::new(n);
    f.put("season_temperature", 4, vec![290.0; n * 4]).unwrap();
    f.put("season_precipitation", 4, vec![3e-5; n * 4]).unwrap();
    f.put("season_soil_moisture", 4, vec![0.05; n * 4]).unwrap();
    f.put("ocean_mask", 1, vec![0.0; n]).unwrap();
    let mut s = Scheduler::new();
    add_biome_stage(&mut s, &BiomeTable::earth_like()).unwrap();
    assert!(matches!(s.run(&d, &mut f, &DirtyRegion::All), Err(Error::MissingInput { .. })));
    // layer counts that disagree
    f.put("glacier_mask", 1, vec![0.0; n]).unwrap();
    // `put` refuses to change a field's layer count, so rebuild the store with a 3-layer soil field
    let mut f = FieldStore::new(n);
    f.put("season_temperature", 4, vec![290.0; n * 4]).unwrap();
    f.put("season_precipitation", 4, vec![3e-5; n * 4]).unwrap();
    f.put("season_soil_moisture", 3, vec![0.05; n * 3]).unwrap();
    f.put("ocean_mask", 1, vec![0.0; n]).unwrap();
    f.put("glacier_mask", 1, vec![0.0; n]).unwrap();
    let mut s = Scheduler::new();
    add_biome_stage(&mut s, &BiomeTable::earth_like()).unwrap();
    assert!(matches!(s.run(&d, &mut f, &DirtyRegion::All), Err(Error::InvalidParam(_))));
}

#[test]
fn capture_season_fills_one_layer_at_a_time() {
    let d = plane(3, 3);
    let n = d.n_cells();
    let mut f = FieldStore::new(n);
    f.put("temperature", 1, vec![280.0; n]).unwrap();
    f.put("precipitation", 1, (0..n).map(|i| i as f64).collect()).unwrap();
    f.put("soil_moisture", 1, vec![0.1; n]).unwrap();
    capture_season(&mut f, 0, 3).unwrap();
    f.write("temperature", |t| t.iter_mut().for_each(|v| *v = 290.0)).unwrap();
    capture_season(&mut f, 2, 3).unwrap();
    assert_eq!(f.layers("season_temperature").unwrap(), 3);
    let t = f.data("season_temperature").unwrap();
    for i in 0..n {
        assert_eq!((t[i * 3], t[i * 3 + 1], t[i * 3 + 2]), (280.0, 0.0, 290.0));
    }
    assert_eq!(f.data("season_precipitation").unwrap()[4 * 3], 4.0);
    assert!(capture_season(&mut f, 3, 3).is_err() && capture_season(&mut f, 0, 0).is_err());
}

// ---------------------------------------------------------------------------------------------
// An Earth-like analytic climate on a sphere: the acceptance check of 5a
// ---------------------------------------------------------------------------------------------

const LAT_T: [(f64, f64); 7] = [(0.0, 27.0), (15.0, 26.0), (30.0, 19.0), (45.0, 9.0), (60.0, 1.0), (75.0, -12.0), (90.0, -23.0)];
const LAT_A: [(f64, f64); 7] = [(0.0, 1.5), (15.0, 3.0), (30.0, 8.0), (45.0, 13.0), (60.0, 17.0), (75.0, 19.0), (90.0, 20.0)];
const LAT_P: [(f64, f64); 9] = [(0.0, 2400.0), (10.0, 2200.0), (20.0, 800.0), (30.0, 300.0), (40.0, 650.0), (50.0, 800.0), (60.0, 600.0), (75.0, 250.0), (90.0, 150.0)];
const SEASONS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];

fn interp(tab: &[(f64, f64)], x: f64) -> f64 {
    if x <= tab[0].0 {
        return tab[0].1;
    }
    for w in tab.windows(2) {
        if x <= w[1].0 {
            let f = (x - w[0].0) / (w[1].0 - w[0].0);
            return w[0].1 + f * (w[1].1 - w[0].1);
        }
    }
    tab[tab.len() - 1].1
}

/// Per-season temperature (K) and precipitation (kg/m^2/s) of a cell at latitude `lat_deg`, longitude `lon` (radians).
/// Same formulas as `analytic_cell` in `reference/mirror_5a.py`.
fn analytic_cell(lat_deg: f64, lon: f64) -> ([f64; 4], [f64; 4]) {
    let al = lat_deg.abs();
    let sg = if lat_deg == 0.0 { 0.0 } else { lat_deg.signum() };
    let tm = interp(&LAT_T, al);
    let amp = interp(&LAT_A, al) * (1.0 + 0.4 * (2.0 * lon).sin());
    let pann = interp(&LAT_P, al) * (1.0 + 0.6 * (3.0 * lon).sin());
    let mut t = [0.0; 4];
    let mut p = [0.0; 4];
    for (k, &s) in SEASONS.iter().enumerate() {
        let ph = (2.0 * std::f64::consts::PI * s).sin();
        t[k] = tm + amp * ph * sg + KELVIN;
        let w = if al < 25.0 {
            1.0 + 0.8 * sg * ph * (1.0 - al / 25.0)
        } else if al >= 30.0 && al < 45.0 {
            1.0 - 0.7 * sg * ph
        } else {
            1.0
        };
        p[k] = (pann * w).max(0.0) / SEASON_YEAR_S;
    }
    let tot = p.iter().sum::<f64>() / 4.0;
    for v in p.iter_mut() {
        *v *= (pann / SEASON_YEAR_S) / tot;
    }
    (t, p)
}

fn is_land(lat: f64, lon: f64) -> bool { (lon + 0.3).sin() + 0.5 * (3.0 * lon).sin() + 0.2 * (2.0 * lat).cos() > 0.1 }

/// Returns the domain, the seasonal fields, `ocean_mask`, `glacier_mask`, plus per-cell latitude in degrees.
fn analytic_world(level: u32) -> (Domain, FieldStore, Vec<f64>) {
    let d = sphere(level);
    let n = d.n_cells();
    let mut t = vec![0.0; n * 4];
    let mut p = vec![0.0; n * 4];
    let (mut ocean, mut glacier, mut lats) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    for i in 0..n {
        let u = d.mesh.up[i];
        let lat = u[2].clamp(-1.0, 1.0).asin();
        let lon = u[1].atan2(u[0]);
        let (ti, pi) = analytic_cell(lat.to_degrees(), lon);
        for k in 0..4 {
            t[i * 4 + k] = ti[k];
            p[i * 4 + k] = pi[k];
        }
        let land = is_land(lat, lon);
        ocean[i] = if land { 0.0 } else { 1.0 };
        glacier[i] = if land && lat.to_degrees() > 80.0 { 1.0 } else { 0.0 };
        lats[i] = lat.to_degrees();
    }
    let mut f = FieldStore::new(n);
    f.put("season_temperature", 4, t).unwrap();
    f.put("season_precipitation", 4, p).unwrap();
    f.put("season_soil_moisture", 4, vec![0.05; n * 4]).unwrap();
    f.put("ocean_mask", 1, ocean).unwrap();
    f.put("glacier_mask", 1, glacier).unwrap();
    (d, f, lats)
}

fn share<F: Fn(usize) -> bool>(cells: &[usize], pred: F) -> f64 { cells.iter().filter(|&&i| pred(i)).count() as f64 / cells.len().max(1) as f64 }

#[test]
fn an_earth_like_climate_gives_deserts_rainforests_tundra_and_taiga_in_plausible_places() {
    let (d, mut f, lat) = analytic_world(4);
    let table = BiomeTable::earth_like();
    let sum = classify_biomes(&d, &mut f, &table).unwrap();
    let id = f.data("biome").unwrap().to_vec();
    let ocean = f.data("ocean_mask").unwrap().to_vec();
    let kop = f.data("koppen").unwrap().to_vec();
    let grp = f.data("koppen_group").unwrap().to_vec();
    let aridity = f.data("clim_aridity").unwrap().to_vec();
    let tmean = f.data("clim_t_mean_c").unwrap().to_vec();
    let tcold = f.data("clim_t_cold_c").unwrap().to_vec();
    let name = |i: usize| table.name(id[i] as usize).to_string();
    let land: Vec<usize> = (0..d.n_cells()).filter(|&i| ocean[i] < 0.5).collect();
    assert!(land.len() > 500);
    assert_eq!(sum.cells[table.len()], 0, "no unclassified cell with the default table");
    assert!(((sum.land_area_m2 + sum.ocean_area_m2) - d.mesh.total_area()).abs() < 1e-6 * d.mesh.total_area());
    // ocean cells: biome ocean, Koppen code 0
    for i in 0..d.n_cells() {
        if ocean[i] > 0.5 {
            assert_eq!(name(i), "ocean");
            assert_eq!(kop[i], 0.0);
            assert_eq!(grp[i], 0.0);
        } else {
            assert!(kop[i] >= 1.0 && grp[i] >= 1.0);
        }
    }
    // bands
    let eq: Vec<usize> = land.iter().cloned().filter(|&i| lat[i].abs() < 10.0).collect();
    let sub: Vec<usize> = land.iter().cloned().filter(|&i| lat[i].abs() >= 20.0 && lat[i].abs() < 35.0).collect();
    let pol: Vec<usize> = land.iter().cloned().filter(|&i| lat[i].abs() > 70.0).collect();
    let bor: Vec<usize> = land.iter().cloned().filter(|&i| lat[i].abs() >= 50.0 && lat[i].abs() < 65.0).collect();
    for (nm, b) in [("equatorial", &eq), ("subtropical", &sub), ("polar", &pol), ("boreal", &bor)] {
        assert!(b.len() > 30, "{nm} band has only {} land cells", b.len());
    }
    assert!(share(&eq, |i| matches!(name(i).as_str(), "tropical_rainforest" | "tropical_seasonal_forest")) >= 0.7, "tropics are forest");
    assert!(share(&eq, |i| grp[i] == 1.0) >= 0.95, "tropics are Koppen A");
    assert!(share(&sub, |i| matches!(name(i).as_str(), "hot_desert" | "cold_desert" | "thorn_scrub" | "steppe")) >= 0.2, "subtropics are dry");
    assert!(share(&sub, |i| name(i) == "hot_desert") >= 0.03, "deserts exist in the subtropics");
    assert!(share(&sub, |i| name(i) == "tropical_rainforest") < 0.02, "no rainforest in the subtropical dry belt");
    assert!(share(&pol, |i| matches!(name(i).as_str(), "tundra" | "polar_ice" | "glacier")) >= 0.9, "poles are tundra and ice");
    assert!(share(&pol, |i| grp[i] == 5.0) >= 0.9, "poles are Koppen E");
    assert!(share(&bor, |i| name(i) == "taiga") >= 0.55, "boreal band is taiga");
    assert!(share(&bor, |i| grp[i] == 4.0) >= 0.65, "boreal band is Koppen D");
    // every named biome obeys its own definition
    for &i in &land {
        match name(i).as_str() {
            "hot_desert" => assert!(aridity[i] < 0.2 && tmean[i] >= 18.0 && lat[i].abs() > 15.0 && lat[i].abs() < 40.0),
            "tropical_rainforest" => assert!(aridity[i] >= 1.1 && tcold[i] >= 18.0),
            "glacier" => assert!(lat[i] > 80.0),
            _ => {}
        }
    }
    // a minimum menu of biomes exists
    for b in ["tropical_rainforest", "hot_desert", "tundra", "taiga", "temperate_forest", "savanna"] {
        assert!(sum.land_share(b) > 0.0, "{b} missing");
    }
    let total: f64 = sum.names.iter().map(|n| sum.land_share(n)).sum();
    assert!(close(total, 1.0, 1e-12));
}

#[test]
fn koppen_and_normals_modules_agree_with_the_pure_functions() {
    let (d, mut f, _lat) = analytic_world(3);
    classify_biomes(&d, &mut f, &BiomeTable::earth_like()).unwrap();
    let (t, p) = (f.data("season_temperature").unwrap().to_vec(), f.data("season_precipitation").unwrap().to_vec());
    let ocean = f.data("ocean_mask").unwrap().to_vec();
    let kop = f.data("koppen").unwrap().to_vec();
    let (ar, pa, soil) = (f.data("clim_aridity").unwrap().to_vec(), f.data("clim_p_annual").unwrap().to_vec(), f.data("clim_soil_m").unwrap().to_vec());
    for i in 0..d.n_cells() {
        let (ti, pi) = (&t[i * 4..i * 4 + 4], &p[i * 4..i * 4 + 4]);
        let r = season_normals(ti, pi, &NormalsParams::default());
        assert_eq!(ar[i], r.aridity);
        assert_eq!(pa[i], r.p_annual);
        assert!(close(soil[i], 0.05, 1e-12));
        let expect = if ocean[i] > 0.5 { 0.0 } else { koppen_class(ti, pi, SEASON_YEAR_S) as f64 };
        assert_eq!(kop[i], expect);
    }
}

#[test]
fn editing_the_default_table_text_moves_the_desert_boundary() {
    let (d, mut f, _lat) = analytic_world(4);
    let base = BiomeTable::earth_like();
    let s0 = classify_biomes(&d, &mut f, &base).unwrap();
    // to_text output is generated by the crate, so the replaced lines are exact on every platform
    let wetter_limit = BiomeTable::parse(&base.to_text().replace("clim_aridity < 0.2\n", "clim_aridity < 0.4\n")).unwrap();
    let s1 = classify_biomes(&d, &mut f, &wetter_limit).unwrap();
    assert!(s1.land_share("hot_desert") > s0.land_share("hot_desert"), "a higher aridity limit gives more desert");
    assert!(s1.land_share("thorn_scrub") < s0.land_share("thorn_scrub"));
    let dark = BiomeTable::parse(&base.to_text().replace("albedo=0.35", "albedo=0.5")).unwrap();
    classify_biomes(&d, &mut f, &dark).unwrap();
    let id = f.data("biome").unwrap().to_vec();
    let al = f.data("biome_albedo").unwrap().to_vec();
    let hd = base.id_of("hot_desert").unwrap() as f64;
    assert!((0..d.n_cells()).filter(|&i| id[i] == hd).all(|i| al[i] == 0.5));
}

#[test]
fn classification_is_deterministic() {
    let run = || {
        let (d, mut f, _) = analytic_world(3);
        classify_biomes(&d, &mut f, &BiomeTable::earth_like()).unwrap();
        (f.data("biome").unwrap().to_vec(), f.data("koppen").unwrap().to_vec(), f.data("biome_albedo").unwrap().to_vec())
    };
    assert_eq!(run(), run());
}

// ---------------------------------------------------------------------------------------------
// The real pipeline feeds the stage (smoke test; thresholds are deliberately loose, numbers are printed)
// ---------------------------------------------------------------------------------------------

#[test]
fn the_land_water_pipeline_feeds_the_biome_stage() {
    let (d, mut f) = earth_like_world(3).unwrap();
    let mut s = Scheduler::new();
    add_land_water_pipeline(&mut s).unwrap();
    seed_land_state(&mut f, 0.075).unwrap();
    let seasons: Vec<f64> = (0..12).map(|k| (k as f64 + 0.5) / 12.0).collect();
    // year 1 spins the land state up, year 2 is the one the normals are taken from
    run_climate_seasons(&d, &mut f, &mut s, &seasons).unwrap();
    let rep = run_climate_seasons(&d, &mut f, &mut s, &seasons).unwrap();
    assert_eq!(rep.len(), 12);
    assert_eq!(f.layers("season_temperature").unwrap(), 12);
    assert_eq!(f.layers("season_precipitation").unwrap(), 12);
    assert!(f.data("season_temperature").unwrap().iter().all(|&t| t.is_finite() && t > 150.0 && t < 400.0));
    let table = BiomeTable::earth_like();
    let sum = classify_biomes(&d, &mut f, &table).unwrap();
    let ocean = f.data("ocean_mask").unwrap().to_vec();
    let id = f.data("biome").unwrap().to_vec();
    assert!((0..d.n_cells()).all(|i| ocean[i] < 0.5 || id[i] == 0.0), "ocean cells are the ocean biome");
    assert!(sum.land_area_m2 > 0.0);
    let present = (0..table.len()).filter(|&k| sum.cells[k] > 0).count();
    assert!(present >= 3, "expected at least three biomes on the land of the Earth-like world, got {present}");
    eprintln!("5a smoke (L3, 12 seasons): land share per biome");
    for (k, nm) in sum.names.iter().enumerate() {
        if sum.cells[k] > 0 {
            eprintln!("  {:<28} {:>5} cells  {:>6.3}", nm, sum.cells[k], sum.land_share(nm));
        }
    }
}
