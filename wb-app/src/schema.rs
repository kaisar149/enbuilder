//! `get_schema`: parameters, presets, modules, fields and biomes, generated from the REAL module specs and the parameter tables.
//! Only field display metadata (unit, colormap, range) is a table here, because the simulation does not carry it.
use crate::api::API_VERSION;
use crate::params::{param_defs, presets};
use crate::world::{biome_table, pipeline};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// (name, kind, unit, display scale, display offset, display unit, colormap, fixed range or None = auto, description)
type Meta = (&'static str, &'static str, &'static str, f64, f64, &'static str, &'static str, Option<(f64, f64)>, &'static str);

const FIELD_META: &[Meta] = &[
    ("elevation", "scalar", "m", 1.0, 0.0, "m", "terrain", Some((-5000.0, 5000.0)), "Surface elevation relative to sea level"),
    ("temperature", "scalar", "K", 1.0, -273.15, "degC", "temperature", Some((220.0, 320.0)), "Annual mean surface temperature (after a solve)"),
    ("precipitation", "scalar", "kg m-2 s-1", 86400.0, 0.0, "mm/day", "rain", Some((0.0, 1.0e-4)), "Annual mean precipitation (after a solve)"),
    ("biome", "category", "", 1.0, 0.0, "", "category", None, "Biome id (palette in the schema `biomes`)"),
    ("koppen", "category", "", 1.0, 0.0, "", "category", None, "Koppen class id"),
    ("wind_speed", "scalar", "m s-1", 1.0, 0.0, "m/s", "speed", None, "Surface wind speed"),
    ("wind", "vector", "m s-1", 1.0, 0.0, "m/s", "speed", None, "Surface wind vector (3 layers)"),
    ("pressure_sl", "scalar", "Pa", 0.01, 0.0, "hPa", "viridis", None, "Sea-level pressure"),
    ("evaporation", "scalar", "kg m-2 s-1", 86400.0, 0.0, "mm/day", "rain", None, "Evaporation"),
    ("vapor", "scalar", "kg m-2", 1.0, 0.0, "kg/m2", "rain", None, "Column water vapour"),
    ("insolation", "scalar", "W m-2", 1.0, 0.0, "W/m2", "viridis", None, "Absorbed solar input"),
    ("snow_cover", "scalar", "", 1.0, 0.0, "", "rain", Some((0.0, 1.0)), "Snow cover fraction"),
    ("glacier_mask", "mask", "", 1.0, 0.0, "", "mask", Some((0.0, 1.0)), "Glacier ice present"),
    ("ocean_mask", "mask", "", 1.0, 0.0, "", "mask", Some((0.0, 1.0)), "Ocean cells"),
    ("river_mask", "mask", "", 1.0, 0.0, "", "mask", Some((0.0, 1.0)), "River cells"),
    ("discharge", "scalar", "m3 s-1", 1.0, 0.0, "m3/s", "rain", None, "River discharge"),
    ("runoff", "scalar", "kg m-2 s-1", 86400.0, 0.0, "mm/day", "rain", None, "Runoff"),
    ("soil_moisture", "scalar", "", 1.0, 0.0, "", "rain", None, "Soil moisture"),
    ("lake_depth", "scalar", "m", 1.0, 0.0, "m", "rain", None, "Lake depth"),
    ("drainage_area", "scalar", "m2", 1e-6, 0.0, "km2", "viridis", None, "Upstream drainage area"),
    ("coast_distance", "scalar", "m", 1e-3, 0.0, "km", "viridis", None, "Distance to the coast"),
    ("soil_fertility", "scalar", "", 1.0, 0.0, "", "viridis", Some((0.0, 1.0)), "Soil fertility"),
    ("habitability", "scalar", "", 1.0, 0.0, "", "viridis", Some((0.0, 1.0)), "Habitability"),
    ("travel_difficulty", "scalar", "", 1.0, 0.0, "", "viridis", Some((0.0, 1.0)), "Travel difficulty"),
];

pub fn meta_of(name: &str) -> Option<&'static Meta> { FIELD_META.iter().find(|m| m.0 == name) }

pub fn field_json(name: &str) -> Value {
    match meta_of(name) {
        Some(m) => json!({
            "name": name, "kind": m.1, "unit": m.2,
            "display": { "scale": m.3, "offset": m.4, "unit": m.5 },
            "colormap": m.6, "range": m.7.map(|r| json!([r.0, r.1])).unwrap_or(Value::Null), "description": m.8,
        }),
        None => json!({
            "name": name, "kind": "scalar", "unit": "", "display": { "scale": 1.0, "offset": 0.0, "unit": "" },
            "colormap": "viridis", "range": Value::Null, "description": "",
        }),
    }
}

/// The scheduler the schema is read from: the solve pipeline plus the stage-E modules.
fn schema_scheduler() -> enbuilder::Result<enbuilder::Scheduler> {
    let mut s = pipeline()?;
    enbuilder::add_biome_stage(&mut s, &biome_table())?;
    enbuilder::add_derived_stage(&mut s)?;
    Ok(s)
}

pub fn schema() -> crate::api::ApiResult<Value> {
    let s = schema_scheduler()?;
    let mut producers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut modules = vec![];
    for spec in s.specs() {
        for o in &spec.outputs {
            producers.entry(o.clone()).or_default().push(spec.name.clone());
        }
        let params: Vec<Value> = s.module_params(&spec.name).unwrap_or_default().into_iter().map(|(k, v)| json!({ "name": k, "default": v })).collect();
        modules.push(json!({ "name": spec.name, "inputs": spec.inputs, "outputs": spec.outputs, "anomaly_hooks": spec.anomaly_hooks, "params": params }));
    }
    producers.entry("elevation".into()).or_default().push("world".into());
    producers.entry("temperature".into()).or_default().push("solve".into());
    producers.entry("precipitation".into()).or_default().push("solve".into());
    let fields: Vec<Value> = producers
        .iter()
        .map(|(name, by)| {
            let mut j = field_json(name);
            j["produced_by"] = json!(by);
            j
        })
        .collect();
    let loops: Vec<Value> = s.feedback_loops().iter().map(|l| json!({ "name": l.name, "members": l.members })).collect();
    let table = biome_table();
    let biomes: Vec<Value> = table.biomes.iter().enumerate().map(|(i, b)| json!({ "id": i, "name": b.name, "color": b.color })).collect();
    Ok(json!({
        "api_version": API_VERSION,
        "params": param_defs(),
        "presets": presets(),
        "modules": modules,
        "loops": loops,
        "fields": fields,
        "biomes": biomes,
        "colormaps": ["terrain", "temperature", "rain", "speed", "viridis", "mask", "category"],
    }))
}
