//! `get_schema`: parameters, presets, modules, fields and biomes, generated from the REAL module specs and the parameter tables.
//! Only field display metadata (unit, colormap, range) is a table here, because the simulation does not carry it.
use crate::api::API_VERSION;
use crate::params::{group_defs, param_defs, presets};
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
    ("current", "vector", "m s-1", 1.0, 0.0, "m/s", "speed", None, "Ocean surface current vector (3 layers)"),
    ("current_speed", "scalar", "m s-1", 1.0, 0.0, "m/s", "speed", None, "Ocean surface current speed"),
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

/// How a category field is coloured: `"biomes"` = the schema `biomes[]` table, `"hash"` = deterministic colour per id.
/// `None` for non-category fields. (Added in 9b so clients do not hard-code field names.)
pub fn palette_of(name: &str) -> Option<&'static str> {
    match meta_of(name) {
        Some(m) if m.1 == "category" => Some(if name == "biome" { "biomes" } else { "hash" }),
        _ => None,
    }
}

pub fn field_json(name: &str) -> Value {
    let mut j = match meta_of(name) {
        Some(m) => json!({
            "name": name, "kind": m.1, "unit": m.2,
            "display": { "scale": m.3, "offset": m.4, "unit": m.5 },
            "colormap": m.6, "range": m.7.map(|r| json!([r.0, r.1])).unwrap_or(Value::Null), "description": m.8,
            "featured": true,
        }),
        None => json!({
            "name": name, "kind": "scalar", "unit": "", "display": { "scale": 1.0, "offset": 0.0, "unit": "" },
            "colormap": "viridis", "range": Value::Null, "description": "", "featured": false,
        }),
    };
    if let Some(p) = palette_of(name) {
        j["palette"] = json!(p);
    }
    j
}

/// The scheduler the schema is read from: the solve pipeline plus the stage-E modules.
fn schema_scheduler() -> enbuilder::Result<enbuilder::Scheduler> {
    let mut s = pipeline()?;
    enbuilder::add_biome_stage(&mut s, &biome_table())?;
    enbuilder::add_derived_stage(&mut s)?;
    Ok(s)
}

/// Part 9c: the seasonal (12-layer) twin of each annual field. `solve` stores them (`capture_season`); a client shows season `k` by asking
/// `get_field` for the twin with `layer = k`. Season `k` is the orbit fraction `(k + 0.5) / count`.
pub fn seasons_json() -> Value {
    let variants: serde_json::Map<String, Value> = season_variants().iter().map(|(b, t)| (b.to_string(), json!(t))).collect();
    json!({ "count": crate::world::SEASONS, "variants": variants })
}

/// (base field, seasonal twin) pairs; `get_cell` also returns the twelve seasonal values of each (Part 9e).
pub fn season_variants() -> [(&'static str, &'static str); 3] {
    [("temperature", "season_temperature"), ("precipitation", "season_precipitation"), ("soil_moisture", "season_soil_moisture")]
}

/// Part 9e: overlay declarations, as data. `kind`: `vector` (arrows / streaks from a 3-layer vector field), `lines` (polylines from a
/// core query, `source`), `mask` (cells where `field > threshold`, drawn as `mode` = `fill` or `outline`). `token` names a theme colour
/// (`--overlay-<token>`); `weight_field` scales line width. The renderer handles kinds and never a specific field.
pub fn overlays_json() -> Value {
    json!([
        { "id": "wind", "label": "Wind", "kind": "vector", "field": "wind", "token": "wind", "animated": true, "unit_hint": "m/s" },
        { "id": "currents", "label": "Ocean currents", "kind": "vector", "field": "current", "token": "current", "animated": true, "unit_hint": "m/s" },
        { "id": "rivers", "label": "Rivers", "kind": "lines", "source": "rivers", "weight_field": "discharge", "token": "river" },
        { "id": "lakes", "label": "Lakes", "kind": "mask", "field": "lake_depth", "threshold": 0.0, "mode": "fill", "token": "lake" },
        { "id": "coast", "label": "Coastline", "kind": "mask", "field": "ocean_mask", "threshold": 0.5, "mode": "outline", "token": "coast" },
        { "id": "glaciers", "label": "Glaciers", "kind": "mask", "field": "glacier_mask", "threshold": 0.5, "mode": "outline", "token": "glacier" },
    ])
}

/// Part 9e: layer presets, as data. `base` is the scalar/mask/category field shown underneath, `overlays` are overlay ids from the list above
/// in draw order (first = bottom). Applying a preset replaces the whole stack in one step.
pub fn layer_presets_json() -> Value {
    json!([
        { "id": "terrain", "name": "Terrain", "description": "Relief and coastline", "base": "elevation", "overlays": ["coast", "glaciers"] },
        { "id": "climate", "name": "Climate", "description": "Wind arrows over rainfall", "base": "precipitation", "overlays": ["coast", "wind"] },
        { "id": "hydrology", "name": "Hydrology", "description": "Rivers, lakes and relief", "base": "elevation", "overlays": ["glaciers", "lakes", "coast", "rivers"] },
        { "id": "ocean", "name": "Ocean", "description": "Currents over sea temperature", "base": "temperature", "overlays": ["coast", "currents"] },
    ])
}

pub fn schema() -> crate::api::ApiResult<Value> {
    let s = schema_scheduler()?;
    let mut producers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut modules = vec![];
    // Part 9e: the diagnostic ocean-current stages that `solve` also runs (wind stress, gyre forcing, currents)
    let mut extra = enbuilder::Scheduler::new();
    enbuilder::add_ocean_stage(&mut extra)?;
    enbuilder::add_gyre_forcing_stage(&mut extra)?;
    enbuilder::add_ocean_currents_stage(&mut extra)?;
    let known: Vec<String> = s.specs().iter().map(|x| x.name.clone()).collect();
    let all_specs: Vec<_> = s.specs().iter().cloned().chain(extra.specs().iter().filter(|x| !known.contains(&x.name)).cloned()).collect();
    for spec in &all_specs {
        for o in &spec.outputs {
            producers.entry(o.clone()).or_default().push(spec.name.clone());
        }
        let params: Vec<Value> = s.module_params(&spec.name).or_else(|| extra.module_params(&spec.name)).unwrap_or_default().into_iter().map(|(k, v)| json!({ "name": k, "default": v })).collect();
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
        "groups": group_defs(),
        "seasons": seasons_json(),
        "overlays": overlays_json(),
        "layer_presets": layer_presets_json(),
        "presets": presets(),
        "modules": modules,
        "loops": loops,
        "fields": fields,
        "biomes": biomes,
        "colormaps": ["terrain", "temperature", "rain", "speed", "viridis", "mask", "category"],
        "layer_kinds": ["scalar", "mask", "category", "vector", "lines"],
    }))
}
