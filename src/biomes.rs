//! Part 5a (stage E): climate normals, Koppen classes, the biome table as DATA, and the biome map.
//!
//! Written without a Rust toolchain (not compiled): `reference/mirror_5a.py` mirrors every formula here and produced the
//! hand numbers of `tests/part5.rs`.
//!
//! Pipeline (all after the climate; no feedback yet, that is 5b):
//!   `season_temperature` (K), `season_precipitation` (kg/m^2/s), `season_soil_moisture` (m), each with one layer per season
//!   -> `climate_normals` -> `clim_*` fields (deg C, mm/yr, ...)
//!   -> `koppen` (class code per cell, from the seasonal fields directly)
//!   -> `biomes` (first matching row of a user-editable table) -> `biome`, `biome_albedo`, `biome_evap_factor`,
//!      `biome_roughness_m`, `biome_color`.
//!
//! The seasonal fields come from `run_climate_seasons` (real pipeline) or are supplied directly (tests, painted climates).
//! Seasons are equal-length blocks of the year in orbit order. With 4 seasons a "month count" is quantised to 0, 3, 6, 9, 12.
//! The climate has no thermal inertia (Part 2), so seasonal swings, and with them D-type classes, are exaggerated.
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::land_water::{run_land_cycle, LandSeason};
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::scheduler::Scheduler;
use std::collections::BTreeSet;

pub const KELVIN: f64 = 273.15;
/// Seconds per year used to turn kg/m^2/s into mm/yr (365.25 days).
pub const SEASON_YEAR_S: f64 = 3.15576e7;
const DAYS_PER_YEAR: f64 = 365.25;

fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

/// The built-in table (data file `data/earth_biomes.txt`, parsed by `BiomeTable::earth_like`).
pub const EARTH_BIOMES_TEXT: &str = include_str!("../data/earth_biomes.txt");

// ---------------------------------------------------------------------------------------------
// Pure: seasonal normals
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalsParams {
    /// Potential evapotranspiration `pet_slope * max(T_C, 0)^pet_exp` in mm/day (about 4.5 mm/day at 26 C, 1.4 at 10 C).
    pub pet_slope: f64,
    pub pet_exp: f64,
    pub year_s: f64,
}

impl Default for NormalsParams {
    fn default() -> Self { NormalsParams { pet_slope: 0.09, pet_exp: 1.2, year_s: SEASON_YEAR_S } }
}

/// Annual statistics of one cell. Temperatures in deg C, precipitation in mm/yr (a season's rate expressed per year).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Normals {
    pub t_mean_c: f64,
    pub t_cold_c: f64,
    pub t_warm_c: f64,
    pub t_range_k: f64,
    pub p_annual: f64,
    /// Driest and wettest season, each as an annual-equivalent rate (mm/yr).
    pub p_dry: f64,
    pub p_wet: f64,
    /// Share of the annual precipitation that falls in the warm half of the seasons (0.5 = even).
    pub p_summer_frac: f64,
    /// 12 * (seasons with T > 10 C) / n.
    pub months_warm: f64,
    /// Growing degree days above 5 C (K day / yr).
    pub gdd: f64,
    pub pet: f64,
    /// UNEP index `p_annual / pet` (pet floored at 1 mm/yr).
    pub aridity: f64,
}

pub fn pet_mm_day(t_c: f64, p: &NormalsParams) -> f64 { p.pet_slope * t_c.max(0.0).powf(p.pet_exp) }

/// Marks the warm half of the seasons: the `ceil(n/2)` warmest (ties to the lower index). Hemisphere independent.
pub fn warm_half(tc: &[f64]) -> Vec<bool> {
    let n = tc.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| tc[b].partial_cmp(&tc[a]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
    let mut w = vec![false; n];
    for &k in idx.iter().take((n + 1) / 2) {
        w[k] = true;
    }
    w
}

/// `t_k` and `p_rate` hold one value per season (same length, >= 1). Negative rates count as 0.
pub fn season_normals(t_k: &[f64], p_rate: &[f64], p: &NormalsParams) -> Normals {
    let n = t_k.len();
    let nf = n as f64;
    let tc: Vec<f64> = t_k.iter().map(|&t| t - KELVIN).collect();
    let pyr: Vec<f64> = p_rate.iter().map(|&r| r.max(0.0) * p.year_s).collect();
    let p_ann = pyr.iter().sum::<f64>() / nf;
    let warm = warm_half(&tc);
    let sum_warm: f64 = (0..n).filter(|&k| warm[k]).map(|k| pyr[k]).sum();
    let sum_all: f64 = pyr.iter().sum();
    let summer_frac = if sum_all > 0.0 { sum_warm / sum_all } else { 0.5 };
    let months_warm = 12.0 * tc.iter().filter(|&&x| x > 10.0).count() as f64 / nf;
    let gdd = tc.iter().map(|&x| (x - 5.0).max(0.0)).sum::<f64>() / nf * DAYS_PER_YEAR;
    let pet = tc.iter().map(|&x| pet_mm_day(x, p)).sum::<f64>() / nf * DAYS_PER_YEAR;
    let t_cold = tc.iter().cloned().fold(f64::INFINITY, f64::min);
    let t_warm = tc.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    Normals {
        t_mean_c: tc.iter().sum::<f64>() / nf,
        t_cold_c: t_cold,
        t_warm_c: t_warm,
        t_range_k: t_warm - t_cold,
        p_annual: p_ann,
        p_dry: pyr.iter().cloned().fold(f64::INFINITY, f64::min),
        p_wet: pyr.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        p_summer_frac: summer_frac,
        months_warm,
        gdd,
        pet,
        aridity: p_ann / pet.max(1.0),
    }
}

// ---------------------------------------------------------------------------------------------
// Pure: Koppen-Geiger (Peel et al. 2007) on seasons
// ---------------------------------------------------------------------------------------------

/// Code 0 is the ocean (no class); land classes are 1.. in this order. `koppen` stores the index as f64.
pub const KOPPEN_CODES: [&str; 31] = [
    "Ocean", "Af", "Am", "Aw", "BWh", "BWk", "BSh", "BSk", "Csa", "Csb", "Csc", "Cwa", "Cwb", "Cwc", "Cfa", "Cfb", "Cfc", "Dsa", "Dsb", "Dsc", "Dsd", "Dwa", "Dwb",
    "Dwc", "Dwd", "Dfa", "Dfb", "Dfc", "Dfd", "ET", "EF",
];

/// Class index (into `KOPPEN_CODES`, always >= 1) from per-season temperature (K) and precipitation rate (kg/m^2/s).
/// Each season is treated as a block of equal months; monthly precipitation = rate * year_s / 12. Thresholds follow Peel et al.:
/// E when the warmest season is under 10 C (EF when under 0 C); B when annual precipitation is below `10 Pth` with
/// `Pth = 2 T_ann (+28 when 70% falls in the warm half, +0 when 70% in the cold half, else +14)` (W below `5 Pth`, h when
/// `T_ann >= 18`); A when the coldest season is at least 18 C (Af driest month >= 60 mm, Am when `P_ann >= 25 (100 - P_min)`, else Aw);
/// C when the coldest season is above 0 C, else D; second letter s / w / f from the driest and wettest warm and cold seasons; third letter
/// a (warmest >= 22 C), b (>= 4 months above 10 C), d (coldest < -38 C), else c.
pub fn koppen_class(t_k: &[f64], p_rate: &[f64], year_s: f64) -> usize {
    let n = t_k.len();
    let nf = n as f64;
    let tc: Vec<f64> = t_k.iter().map(|&t| t - KELVIN).collect();
    let pm: Vec<f64> = p_rate.iter().map(|&r| r.max(0.0) * year_s / 12.0).collect();
    let t_ann = tc.iter().sum::<f64>() / nf;
    let t_cold = tc.iter().cloned().fold(f64::INFINITY, f64::min);
    let t_warm = tc.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let p_ann = pm.iter().sum::<f64>() * 12.0 / nf;
    let months_warm = 12.0 * tc.iter().filter(|&&x| x > 10.0).count() as f64 / nf;
    let idx = |code: &str| -> usize { KOPPEN_CODES.iter().position(|&c| c == code).unwrap_or(0) };
    if t_warm < 10.0 {
        return idx(if t_warm > 0.0 { "ET" } else { "EF" });
    }
    let warm = warm_half(&tc);
    let warm_ids: Vec<usize> = (0..n).filter(|&k| warm[k]).collect();
    let mut cold_ids: Vec<usize> = (0..n).filter(|&k| !warm[k]).collect();
    if cold_ids.is_empty() {
        cold_ids = (0..n).collect();
    }
    let tot: f64 = pm.iter().sum();
    let frac_s = if n == 1 || tot <= 0.0 { 0.5 } else { warm_ids.iter().map(|&k| pm[k]).sum::<f64>() / tot };
    let add = if frac_s >= 0.7 { 28.0 } else if 1.0 - frac_s >= 0.7 { 0.0 } else { 14.0 };
    let pth = (2.0 * t_ann + add).max(0.0);
    if p_ann < 10.0 * pth {
        let a = if p_ann < 5.0 * pth { "BW" } else { "BS" };
        let b = if t_ann >= 18.0 { "h" } else { "k" };
        return idx(&format!("{a}{b}"));
    }
    let p_min = pm.iter().cloned().fold(f64::INFINITY, f64::min);
    if t_cold >= 18.0 {
        if p_min >= 60.0 {
            return idx("Af");
        }
        return idx(if p_ann >= 25.0 * (100.0 - p_min) { "Am" } else { "Aw" });
    }
    let first = if t_cold > 0.0 { "C" } else { "D" };
    let sd = warm_ids.iter().map(|&k| pm[k]).fold(f64::INFINITY, f64::min);
    let sw = warm_ids.iter().map(|&k| pm[k]).fold(f64::NEG_INFINITY, f64::max);
    let wd = cold_ids.iter().map(|&k| pm[k]).fold(f64::INFINITY, f64::min);
    let ww = cold_ids.iter().map(|&k| pm[k]).fold(f64::NEG_INFINITY, f64::max);
    let s2 = if sd < 40.0 && sd < ww / 3.0 {
        "s"
    } else if wd < sw / 10.0 {
        "w"
    } else {
        "f"
    };
    let s3 = if t_warm >= 22.0 {
        "a"
    } else if months_warm >= 4.0 {
        "b"
    } else if t_cold < -38.0 {
        "d"
    } else {
        "c"
    };
    idx(&format!("{first}{s2}{s3}"))
}

/// 0 ocean, 1 A, 2 B, 3 C, 4 D, 5 E.
pub fn koppen_group(code: usize) -> usize {
    match KOPPEN_CODES.get(code).and_then(|c| c.chars().next()) {
        Some('A') => 1,
        Some('B') => 2,
        Some('C') => 3,
        Some('D') => 4,
        Some('E') => 5,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------------------------
// The biome table (data)
// ---------------------------------------------------------------------------------------------

/// `lo <= value < hi` on the first layer of any field in the store. Open ends are +-infinity.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub field: String,
    pub lo: f64,
    pub hi: f64,
}

impl Condition {
    pub fn holds(&self, v: f64) -> bool { v >= self.lo && v < self.hi }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BiomeDef {
    pub name: String,
    pub color: [u8; 3],
    /// Surface albedo of land under this biome (the 5b albedo variant reads it).
    pub albedo: f64,
    /// Multiplier on the land availability of the moisture source (vegetation transpiration; 5b).
    pub evap_factor: f64,
    /// Canopy height in metres (adds to the surface roughness of the wind; 5b).
    pub roughness_m: f64,
    /// All must hold. An empty list matches every cell, so it acts as a catch-all when placed last.
    pub conditions: Vec<Condition>,
}

/// An ordered table: the first biome whose conditions all hold wins. A cell that matches nothing gets id `len()` ("unclassified").
///
/// Text format (one item per line; `#` starts a comment when it begins a word; names have no spaces):
/// ```text
/// biome desert color=#e6c36a albedo=0.35 evap=0.2 rough=0.1
///   clim_aridity < 0.2
///   clim_t_mean_c >= 18
///   clim_p_annual in 0..250
/// ```
/// Operators: `>=` (lower bound), `<` (upper bound), `in a..b` (`a <= x < b`; `inf` and `-inf` allowed).
/// Keys: `color` (#rrggbb, default #808080), `albedo` (0..1, default 0.22), `evap` (>= 0, default 1), `rough` (>= 0, default 0).
#[derive(Debug, Clone, PartialEq)]
pub struct BiomeTable {
    pub biomes: Vec<BiomeDef>,
}

fn table_err(line: usize, msg: &str) -> Error { Error::InvalidParam(format!("biome table line {line}: {msg}")) }

fn parse_num(line: usize, s: &str) -> Result<f64> {
    s.parse::<f64>().map_err(|_| table_err(line, &format!("'{s}' is not a number")))
}

impl BiomeTable {
    pub fn new(biomes: Vec<BiomeDef>) -> Self { BiomeTable { biomes } }

    /// The default Earth-like table (the file `data/earth_biomes.txt`).
    pub fn earth_like() -> Self { BiomeTable::parse(EARTH_BIOMES_TEXT).expect("built-in biome table must parse") }

    pub fn parse(text: &str) -> Result<Self> {
        let mut biomes: Vec<BiomeDef> = Vec::new();
        for (k, raw) in text.lines().enumerate() {
            let ln = k + 1;
            let mut tok: Vec<&str> = Vec::new();
            for t in raw.split_whitespace() {
                if t.starts_with('#') {
                    break;
                }
                tok.push(t);
            }
            if tok.is_empty() {
                continue;
            }
            if tok[0] == "biome" {
                if tok.len() < 2 {
                    return Err(table_err(ln, "biome needs a name"));
                }
                let name = tok[1].to_string();
                if biomes.iter().any(|b| b.name == name) {
                    return Err(table_err(ln, &format!("duplicate biome name '{name}'")));
                }
                let mut b = BiomeDef { name, color: [128, 128, 128], albedo: 0.22, evap_factor: 1.0, roughness_m: 0.0, conditions: Vec::new() };
                for kv in &tok[2..] {
                    let (key, val) = kv.split_once('=').ok_or_else(|| table_err(ln, &format!("'{kv}' is not key=value")))?;
                    match key {
                        "color" => {
                            let h = val.strip_prefix('#').ok_or_else(|| table_err(ln, "color must be #rrggbb"))?;
                            if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                                return Err(table_err(ln, "color must be #rrggbb"));
                            }
                            let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0);
                            b.color = [byte(0), byte(2), byte(4)];
                        }
                        "albedo" => b.albedo = parse_num(ln, val)?,
                        "evap" | "evap_factor" => b.evap_factor = parse_num(ln, val)?,
                        "rough" | "roughness_m" => b.roughness_m = parse_num(ln, val)?,
                        other => return Err(table_err(ln, &format!("unknown key '{other}'"))),
                    }
                }
                if !(b.albedo >= 0.0 && b.albedo <= 1.0) {
                    return Err(table_err(ln, "albedo must be in 0..1"));
                }
                if !(b.evap_factor >= 0.0 && b.evap_factor.is_finite()) || !(b.roughness_m >= 0.0 && b.roughness_m.is_finite()) {
                    return Err(table_err(ln, "evap and rough must be finite and >= 0"));
                }
                biomes.push(b);
            } else {
                let b = biomes.last_mut().ok_or_else(|| table_err(ln, "condition before any biome"))?;
                if tok.len() < 3 {
                    return Err(table_err(ln, "condition needs: field op value"));
                }
                let field = tok[0];
                if !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err(table_err(ln, &format!("bad field name '{field}'")));
                }
                let (lo, hi) = match tok[1] {
                    ">=" => (parse_num(ln, tok[2])?, f64::INFINITY),
                    "<" => (f64::NEG_INFINITY, parse_num(ln, tok[2])?),
                    "in" => {
                        let (a, c) = tok[2].split_once("..").ok_or_else(|| table_err(ln, "'in' needs a..b"))?;
                        (parse_num(ln, a)?, parse_num(ln, c)?)
                    }
                    other => return Err(table_err(ln, &format!("operator '{other}' is not one of >=, <, in"))),
                };
                if lo.is_nan() || hi.is_nan() || !(lo < hi) {
                    return Err(table_err(ln, "empty or invalid range"));
                }
                b.conditions.push(Condition { field: field.to_string(), lo, hi });
            }
        }
        Ok(BiomeTable { biomes })
    }

    /// Text that `parse` reads back to an equal table.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for b in &self.biomes {
            s.push_str(&format!(
                "biome {} color=#{:02x}{:02x}{:02x} albedo={} evap={} rough={}\n",
                b.name, b.color[0], b.color[1], b.color[2], b.albedo, b.evap_factor, b.roughness_m
            ));
            for c in &b.conditions {
                if c.lo.is_finite() && c.hi.is_finite() {
                    s.push_str(&format!("  {} in {}..{}\n", c.field, c.lo, c.hi));
                } else if c.lo.is_finite() {
                    s.push_str(&format!("  {} >= {}\n", c.field, c.lo));
                } else if c.hi.is_finite() {
                    s.push_str(&format!("  {} < {}\n", c.field, c.hi));
                } else {
                    s.push_str(&format!("  {} in -inf..inf\n", c.field));
                }
            }
        }
        s
    }

    pub fn len(&self) -> usize { self.biomes.len() }
    pub fn is_empty(&self) -> bool { self.biomes.is_empty() }

    /// Name of a biome id; `len()` is "unclassified".
    pub fn name(&self, id: usize) -> &str { self.biomes.get(id).map(|b| b.name.as_str()).unwrap_or("unclassified") }
    pub fn id_of(&self, name: &str) -> Option<usize> { self.biomes.iter().position(|b| b.name == name) }

    /// Every field name some condition reads (sorted, unique).
    pub fn fields(&self) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        for b in &self.biomes {
            for c in &b.conditions {
                s.insert(c.field.clone());
            }
        }
        s
    }

    /// Id of the first matching biome, `len()` when none matches. `get` returns the value of a field for the cell in question.
    pub fn classify(&self, get: &dyn Fn(&str) -> f64) -> usize {
        for (i, b) in self.biomes.iter().enumerate() {
            if b.conditions.iter().all(|c| c.holds(get(&c.field))) {
                return i;
            }
        }
        self.biomes.len()
    }
}

// ---------------------------------------------------------------------------------------------
// Module: climate_normals
// ---------------------------------------------------------------------------------------------

pub const NORMAL_FIELDS: [&str; 13] = [
    "clim_t_mean_c",
    "clim_t_cold_c",
    "clim_t_warm_c",
    "clim_t_range_k",
    "clim_p_annual",
    "clim_p_dry",
    "clim_p_wet",
    "clim_p_summer_frac",
    "clim_months_warm",
    "clim_gdd",
    "clim_pet",
    "clim_aridity",
    "clim_soil_m",
];

/// Annual statistics of the seasonal fields. Inputs `season_temperature` (K), `season_precipitation` (kg/m^2/s) and
/// `season_soil_moisture` (m), all with the same number of layers (one per season). Outputs `NORMAL_FIELDS` (one layer each).
/// Parameters: `pet_slope` 0.09, `pet_exp` 1.2, `year_s` 3.15576e7.
pub struct ClimateNormals {
    params: ParamSet,
}

impl Default for ClimateNormals {
    fn default() -> Self { Self::new() }
}

impl ClimateNormals {
    pub fn new() -> Self {
        let d = NormalsParams::default();
        ClimateNormals { params: ParamSet::new().with("pet_slope", d.pet_slope).with("pet_exp", d.pet_exp).with("year_s", d.year_s) }
    }
    fn normals_params(&self) -> NormalsParams {
        let d = NormalsParams::default();
        NormalsParams {
            pet_slope: self.params.get_or("pet_slope", d.pet_slope),
            pet_exp: self.params.get_or("pet_exp", d.pet_exp),
            year_s: self.params.get_or("year_s", d.year_s),
        }
    }
}

fn season_layers(fields: &FieldStore, module: &str, name: &str, n_cells: usize) -> Result<(Vec<f64>, usize)> {
    let l = fields.layers(name)?;
    let d = fields.data(name)?;
    if l == 0 || d.len() != n_cells * l {
        return Err(Error::InvalidParam(format!("{module}: field '{name}' must have one value per cell and layer")));
    }
    Ok((d.to_vec(), l))
}

impl Module for ClimateNormals {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "climate_normals".into(),
            inputs: names(&["season_temperature", "season_precipitation", "season_soil_moisture"]),
            outputs: names(&NORMAL_FIELDS),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        match name {
            "pet_slope" | "pet_exp" => {
                if !(value >= 0.0 && value.is_finite()) {
                    return Err(Error::InvalidParam(format!("climate_normals.{name} must be finite and >= 0")));
                }
            }
            "year_s" => {
                if !(value > 0.0 && value.is_finite()) {
                    return Err(Error::InvalidParam("climate_normals.year_s must be > 0".into()));
                }
            }
            _ => return Err(Error::InvalidParam(format!("climate_normals has no parameter '{name}'"))),
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let (t, lt) = season_layers(fields, "climate_normals", "season_temperature", n)?;
        let (p, lp) = season_layers(fields, "climate_normals", "season_precipitation", n)?;
        let (s, ls) = season_layers(fields, "climate_normals", "season_soil_moisture", n)?;
        if lt != lp || lt != ls {
            return Err(Error::InvalidParam("climate_normals: the three seasonal fields must have the same number of layers".into()));
        }
        let np = self.normals_params();
        let mut out: Vec<Vec<f64>> = (0..NORMAL_FIELDS.len()).map(|_| vec![0.0; n]).collect();
        for i in 0..n {
            let r = season_normals(&t[i * lt..(i + 1) * lt], &p[i * lt..(i + 1) * lt], &np);
            let soil = s[i * lt..(i + 1) * lt].iter().sum::<f64>() / lt as f64;
            let vals = [
                r.t_mean_c,
                r.t_cold_c,
                r.t_warm_c,
                r.t_range_k,
                r.p_annual,
                r.p_dry,
                r.p_wet,
                r.p_summer_frac,
                r.months_warm,
                r.gdd,
                r.pet,
                r.aridity,
                soil,
            ];
            for (k, v) in vals.iter().enumerate() {
                out[k][i] = *v;
            }
        }
        for (k, name) in NORMAL_FIELDS.iter().enumerate() {
            fields.put(name, 1, std::mem::take(&mut out[k]))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Module: koppen
// ---------------------------------------------------------------------------------------------

/// Koppen-Geiger class per cell: `koppen` (index into `KOPPEN_CODES`, 0 over the ocean) and `koppen_group` (0 ocean, 1 A .. 5 E).
/// Inputs `season_temperature`, `season_precipitation`, `ocean_mask`. Parameter `year_s` 3.15576e7.
pub struct Koppen {
    params: ParamSet,
}

impl Default for Koppen {
    fn default() -> Self { Self::new() }
}

impl Koppen {
    pub fn new() -> Self { Koppen { params: ParamSet::new().with("year_s", SEASON_YEAR_S) } }
}

impl Module for Koppen {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "koppen".into(),
            inputs: names(&["season_temperature", "season_precipitation", "ocean_mask"]),
            outputs: names(&["koppen", "koppen_group"]),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if name != "year_s" {
            return Err(Error::InvalidParam(format!("koppen has no parameter '{name}'")));
        }
        if !(value > 0.0 && value.is_finite()) {
            return Err(Error::InvalidParam("koppen.year_s must be > 0".into()));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let (t, lt) = season_layers(fields, "koppen", "season_temperature", n)?;
        let (p, lp) = season_layers(fields, "koppen", "season_precipitation", n)?;
        if lt != lp {
            return Err(Error::InvalidParam("koppen: temperature and precipitation need the same number of layers".into()));
        }
        let ocean = fields.data("ocean_mask")?.to_vec();
        if ocean.len() != n {
            return Err(Error::InvalidParam("koppen: ocean_mask must have one value per cell".into()));
        }
        let year_s = self.params.get_or("year_s", SEASON_YEAR_S);
        let mut code = vec![0.0; n];
        let mut group = vec![0.0; n];
        for i in 0..n {
            if ocean[i] > 0.5 {
                continue;
            }
            let c = koppen_class(&t[i * lt..(i + 1) * lt], &p[i * lt..(i + 1) * lt], year_s);
            code[i] = c as f64;
            group[i] = koppen_group(c) as f64;
        }
        fields.put("koppen", 1, code)?;
        fields.put("koppen_group", 1, group)
    }
}

// ---------------------------------------------------------------------------------------------
// Module: biomes
// ---------------------------------------------------------------------------------------------

const BIOME_OUTPUT_ONLY: [&str; 2] = ["n_unclassified", "n_biomes_present"];
const BIOME_SETTABLE: [&str; 3] = ["fallback_albedo", "fallback_evap_factor", "fallback_roughness_m"];

/// The biome map. Inputs: exactly the fields the table's conditions read (so the scheduler orders it after whatever produces them).
/// Outputs `biome` (id as f64, `table.len()` = unclassified), `biome_albedo`, `biome_evap_factor`, `biome_roughness_m`, and
/// `biome_color` (3 layers, 0..1). Parameters: `fallback_albedo` 0.22, `fallback_evap_factor` 1, `fallback_roughness_m` 0 (properties of
/// unclassified cells). Output-only: `n_unclassified`, `n_biomes_present`.
/// The input list is fixed at construction; to change the table, build a new `Biomes` (and a new scheduler).
pub struct Biomes {
    table: BiomeTable,
    params: ParamSet,
}

impl Biomes {
    pub fn new(table: BiomeTable) -> Self {
        let params = ParamSet::new()
            .with("fallback_albedo", 0.22)
            .with("fallback_evap_factor", 1.0)
            .with("fallback_roughness_m", 0.0)
            .with("n_unclassified", 0.0)
            .with("n_biomes_present", 0.0);
        Biomes { table, params }
    }
    pub fn earth_like() -> Self { Biomes::new(BiomeTable::earth_like()) }
    pub fn table(&self) -> &BiomeTable { &self.table }
}

impl Module for Biomes {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "biomes".into(),
            inputs: self.table.fields().into_iter().collect(),
            outputs: names(&["biome", "biome_albedo", "biome_evap_factor", "biome_roughness_m", "biome_color"]),
            anomaly_hooks: vec![],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        if BIOME_OUTPUT_ONLY.contains(&name) {
            return Err(Error::InvalidParam(format!("biomes.{name} is output-only")));
        }
        if !BIOME_SETTABLE.contains(&name) {
            return Err(Error::InvalidParam(format!("biomes has no parameter '{name}'")));
        }
        let ok = if name == "fallback_albedo" { (0.0..=1.0).contains(&value) } else { value >= 0.0 && value.is_finite() };
        if !ok {
            return Err(Error::InvalidParam(format!("biomes.{name} out of range")));
        }
        self.params.set(name, value);
        Ok(())
    }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let n = domain.n_cells();
        let needed: Vec<String> = self.table.fields().into_iter().collect();
        let mut cols: Vec<(Vec<f64>, usize)> = Vec::with_capacity(needed.len());
        for f in &needed {
            let l = fields.layers(f)?;
            let d = fields.data(f)?;
            if l == 0 || d.len() != n * l {
                return Err(Error::InvalidParam(format!("biomes: field '{f}' must have one value per cell and layer")));
            }
            cols.push((d.to_vec(), l));
        }
        // conditions compiled to (column, lo, hi)
        let compiled: Vec<Vec<(usize, f64, f64)>> = self
            .table
            .biomes
            .iter()
            .map(|b| b.conditions.iter().map(|c| (needed.binary_search(&c.field).unwrap_or(0), c.lo, c.hi)).collect())
            .collect();
        let fb_albedo = self.params.get_or("fallback_albedo", 0.22);
        let fb_evap = self.params.get_or("fallback_evap_factor", 1.0);
        let fb_rough = self.params.get_or("fallback_roughness_m", 0.0);
        let nb = self.table.len();
        let mut id = vec![nb as f64; n];
        let mut albedo = vec![fb_albedo; n];
        let mut evap = vec![fb_evap; n];
        let mut rough = vec![fb_rough; n];
        let mut color = vec![0.5; n * 3];
        let mut present = vec![false; nb + 1];
        for i in 0..n {
            let mut hit = nb;
            for (b, conds) in compiled.iter().enumerate() {
                let all = conds.iter().all(|&(col, lo, hi)| {
                    let (d, l) = &cols[col];
                    let v = d[i * l];
                    v >= lo && v < hi
                });
                if all {
                    hit = b;
                    break;
                }
            }
            present[hit] = true;
            id[i] = hit as f64;
            if hit < nb {
                let b = &self.table.biomes[hit];
                albedo[i] = b.albedo;
                evap[i] = b.evap_factor;
                rough[i] = b.roughness_m;
                for k in 0..3 {
                    color[i * 3 + k] = b.color[k] as f64 / 255.0;
                }
            }
        }
        let n_unclassified = (0..n).filter(|&i| id[i] as usize == nb).count();
        self.params.set("n_unclassified", n_unclassified as f64);
        self.params.set("n_biomes_present", present.iter().filter(|&&p| p).count() as f64);
        fields.put("biome", 1, id)?;
        fields.put("biome_albedo", 1, albedo)?;
        fields.put("biome_evap_factor", 1, evap)?;
        fields.put("biome_roughness_m", 1, rough)?;
        fields.put("biome_color", 3, color)
    }
}

// ---------------------------------------------------------------------------------------------
// Wiring, summary, seasonal driver
// ---------------------------------------------------------------------------------------------

/// Registers `climate_normals`, `koppen` and `biomes` (with `table`). External inputs the caller supplies: `season_temperature`,
/// `season_precipitation`, `season_soil_moisture`, `ocean_mask`, and whatever else the table reads (the default table needs `glacier_mask`).
pub fn add_biome_stage(s: &mut Scheduler, table: &BiomeTable) -> Result<()> {
    s.register(Box::new(ClimateNormals::new()))?;
    s.register(Box::new(Koppen::new()))?;
    s.register(Box::new(Biomes::new(table.clone())))
}

/// Area per biome over LAND cells (`ocean_mask < 0.5`). `names` has `table.len() + 1` entries, the last is "unclassified".
#[derive(Debug, Clone, PartialEq)]
pub struct BiomeSummary {
    pub names: Vec<String>,
    pub cells: Vec<usize>,
    pub area_m2: Vec<f64>,
    pub land_area_m2: f64,
    pub ocean_area_m2: f64,
}

impl BiomeSummary {
    /// Share of the land area (0 when there is no land or the name is unknown).
    pub fn land_share(&self, name: &str) -> f64 {
        match self.names.iter().position(|n| n == name) {
            Some(k) if self.land_area_m2 > 0.0 => self.area_m2[k] / self.land_area_m2,
            _ => 0.0,
        }
    }
}

pub fn biome_summary(domain: &Domain, fields: &FieldStore, table: &BiomeTable) -> Result<BiomeSummary> {
    let n = domain.n_cells();
    let id = fields.data("biome")?;
    let ocean = fields.data("ocean_mask")?;
    if id.len() != n || ocean.len() != n {
        return Err(Error::InvalidParam("biome_summary: fields must have one value per cell".into()));
    }
    let nb = table.len();
    let mut cells = vec![0usize; nb + 1];
    let mut area = vec![0.0; nb + 1];
    let (mut land, mut sea) = (0.0, 0.0);
    for i in 0..n {
        let a = domain.mesh.area[i];
        if ocean[i] > 0.5 {
            sea += a;
            continue;
        }
        let k = (id[i].max(0.0) as usize).min(nb);
        cells[k] += 1;
        area[k] += a;
        land += a;
    }
    let mut nm: Vec<String> = table.biomes.iter().map(|b| b.name.clone()).collect();
    nm.push("unclassified".into());
    Ok(BiomeSummary { names: nm, cells, area_m2: area, land_area_m2: land, ocean_area_m2: sea })
}

/// Classify the current seasonal fields with `table` on a fresh scheduler and summarise the land.
pub fn classify_biomes(domain: &Domain, fields: &mut FieldStore, table: &BiomeTable) -> Result<BiomeSummary> {
    let mut s = Scheduler::new();
    add_biome_stage(&mut s, table)?;
    s.run(domain, fields, &DirtyRegion::All)?;
    biome_summary(domain, fields, table)
}

fn capture_layer(fields: &mut FieldStore, src: &str, dst: &str, k: usize, n: usize) -> Result<()> {
    let cells = fields.n_cells();
    let s = fields.data(src)?.to_vec();
    if s.len() != cells {
        return Err(Error::InvalidParam(format!("capture_season: '{src}' must have one layer")));
    }
    let mut d = if fields.has(dst) && fields.layers(dst)? == n { fields.data(dst)?.to_vec() } else { vec![0.0; cells * n] };
    for i in 0..cells {
        d[i * n + k] = s[i];
    }
    fields.put(dst, n, d)
}

/// Copy the current `temperature`, `precipitation` and `soil_moisture` into layer `k` of `season_temperature`,
/// `season_precipitation`, `season_soil_moisture` (created with `n` layers when absent or of another size).
pub fn capture_season(fields: &mut FieldStore, k: usize, n: usize) -> Result<()> {
    if n == 0 || k >= n {
        return Err(Error::InvalidParam("capture_season: need k < n".into()));
    }
    capture_layer(fields, "temperature", "season_temperature", k, n)?;
    capture_layer(fields, "precipitation", "season_precipitation", k, n)?;
    capture_layer(fields, "soil_moisture", "season_soil_moisture", k, n)
}

/// Run the land-water pipeline (`add_land_water_pipeline` scheduler) once per orbit fraction in `seasons` and capture each result
/// into the seasonal fields. The land state (soil, snow, ice) carries over between seasons, so run a few years first (call this
/// repeatedly) before trusting the normals. Each season lasts `step_s`, so 4 seasons of 30 days do NOT cover a year: use
/// `set_land_step(s, year / n)` first for a full-year cycle, or use 12 seasons.
pub fn run_climate_seasons(domain: &Domain, fields: &mut FieldStore, scheduler: &mut Scheduler, seasons: &[f64]) -> Result<Vec<LandSeason>> {
    if seasons.is_empty() {
        return Err(Error::InvalidParam("run_climate_seasons: need at least one season".into()));
    }
    let n = seasons.len();
    let mut out = Vec::with_capacity(n);
    for (k, &season) in seasons.iter().enumerate() {
        let r = run_land_cycle(domain, fields, scheduler, &[season])?;
        capture_season(fields, k, n)?;
        out.extend(r);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn koppen_codes_are_unique_and_start_with_ocean() {
        assert_eq!(KOPPEN_CODES[0], "Ocean");
        for (i, a) in KOPPEN_CODES.iter().enumerate() {
            for b in KOPPEN_CODES.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn earth_table_parses() {
        let t = BiomeTable::earth_like();
        assert!(t.len() > 10);
        assert_eq!(t.name(0), "ocean");
    }
}
