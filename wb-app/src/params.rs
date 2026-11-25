//! World parameters and presets as DATA. The schema export and every command read these tables; there is no second list.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a parameter change invalidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Only solved climate fields go stale (light, rotation, gravity, spin-up length).
    Climate,
    /// Elevation is regenerated too (seed, ocean fraction).
    Elevation,
    /// A different mesh (shape, resolution, size): new `mesh_id`, everything regenerated.
    Mesh,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamDef {
    pub name: String,
    pub group: String,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// "slider", "integer" or "select".
    pub widget: String,
    pub description: String,
    pub scope: Scope,
    /// Labels for a "select" widget (value = index).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

fn p(name: &str, group: &str, unit: &str, min: f64, max: f64, default: f64, widget: &str, scope: Scope, description: &str) -> ParamDef {
    ParamDef { name: name.into(), group: group.into(), unit: unit.into(), min, max, default, widget: widget.into(), description: description.into(), scope, options: vec![] }
}

pub fn param_defs() -> Vec<ParamDef> {
    let mut shape = p("shape", "world", "", 0.0, 2.0, 0.0, "select", Scope::Mesh, "World shape: sphere, cube or flat plane");
    shape.options = vec!["sphere".into(), "cube".into(), "plane".into()];
    vec![
        shape,
        p("resolution", "world", "", 1.0, 5.0, 3.0, "integer", Scope::Mesh, "Mesh resolution level (sphere: icosahedron subdivisions; cube: 4 x level cells per edge; plane: 16 x 8 x level cells)"),
        p("radius_km", "world", "km", 500.0, 20000.0, 6371.0, "slider", Scope::Mesh, "Planet radius (cube: half side; plane: width = circumference)"),
        p("seed", "world", "", 0.0, 1.0e9, 7.0, "integer", Scope::Elevation, "Seed for the terrain; same seed and inputs give the same world"),
        p("ocean_fraction", "world", "", 0.0, 1.0, 0.7, "slider", Scope::Elevation, "Share of the surface (by area) that lies below sea level"),
        p("surface_gravity", "laws", "m/s^2", 1.0, 30.0, 9.81, "slider", Scope::Climate, "Surface gravity"),
        p("day_length_h", "laws", "h", 0.0, 1000.0, 24.0, "slider", Scope::Climate, "Rotation period (0 = no rotation: a static sun, e.g. tidally locked)"),
        p("year_length_days", "laws", "day", 0.0, 5000.0, 365.25, "slider", Scope::Climate, "Orbital period (0 = no orbit: no seasons)"),
        p("axial_tilt_deg", "laws", "deg", 0.0, 90.0, 23.4, "slider", Scope::Climate, "Axial tilt"),
        p("solar_constant", "laws", "W/m^2", 200.0, 3000.0, 1361.0, "slider", Scope::Climate, "Star irradiance at the planet (luminosity)"),
        p("spinup_years", "solver", "yr", 1.0, 6.0, 2.0, "integer", Scope::Climate, "Years of 12 seasons run before the climate normals are taken (the last one is the result)"),
    ]
}

pub fn defaults() -> BTreeMap<String, f64> { param_defs().into_iter().map(|d| (d.name, d.default)).collect() }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub description: String,
    /// Overrides on top of the defaults.
    pub params: BTreeMap<String, f64>,
}

fn preset(name: &str, description: &str, kv: &[(&str, f64)]) -> Preset {
    Preset { name: name.into(), description: description.into(), params: kv.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
}

pub fn presets() -> Vec<Preset> {
    vec![
        preset("earthlike", "Earth-like: 70% ocean, 24 h day, 365 day year, 23.4 degree tilt", &[]),
        preset("desert", "Desert world: little ocean, a stronger sun", &[("ocean_fraction", 0.15), ("solar_constant", 1500.0)]),
        preset("waterworld", "Waterworld: almost all ocean", &[("ocean_fraction", 0.97)]),
        preset("tidally_locked", "Tidally locked: static sun, no seasons, no tilt", &[("day_length_h", 0.0), ("year_length_days", 0.0), ("axial_tilt_deg", 0.0), ("solar_constant", 1000.0)]),
        preset("ice_age", "Ice age: a dimmer sun, more tilt", &[("solar_constant", 1150.0), ("axial_tilt_deg", 28.0)]),
    ]
}

pub fn find_preset(name: &str) -> Option<Preset> { presets().into_iter().find(|p| p.name == name) }

pub fn validate(name: &str, value: f64) -> Result<ParamDef, String> {
    let d = param_defs().into_iter().find(|d| d.name == name).ok_or_else(|| format!("unknown parameter '{name}'"))?;
    if !value.is_finite() || value < d.min || value > d.max {
        return Err(format!("parameter '{name}' must be in [{}, {}], got {value}", d.min, d.max));
    }
    if (d.widget == "integer" || d.widget == "select") && value.fract() != 0.0 {
        return Err(format!("parameter '{name}' must be a whole number, got {value}"));
    }
    Ok(d)
}
