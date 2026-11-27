//! Part 9f: paint layers, locks and the realism blend. Pure functions and plain data: no session, no jobs, no UI types.
//!
//! A paint layer stores, per cell, a painted `value` and a `mask` (coverage in [0, 1]), separately from every computed field.
//! Layers: `elevation` (absolute metres), `temperature_bias` (K delta), `rainfall_bias` (fractional delta of precipitation),
//! `biome_override` (biome id). Blend rule, per field realism `r = slider / 100`:
//!   effective = computed + (1 - r) * mask * (painted - computed)        (bias layers: painted - computed = bias)
//!   category (biome): painted wins iff (1 - r) * mask >= 0.5
//! so r = 0 gives the painted value as given, r = 1 ignores the paint, and values in between blend linearly.
use crate::api::{ApiError, ApiResult};
use std::collections::BTreeMap;

/// Output fields that can be locked and that have a realism slider.
pub const OUTPUT_FIELDS: [&str; 4] = ["elevation", "temperature", "precipitation", "biome"];
/// Paint layers, in the same order as `OUTPUT_FIELDS`.
pub const PAINT_LAYERS: [&str; 4] = ["elevation", "temperature_bias", "rainfall_bias", "biome_override"];

/// The output field a paint layer edits.
pub fn output_of(layer: &str) -> Option<&'static str> {
    PAINT_LAYERS.iter().position(|l| *l == layer).map(|i| OUTPUT_FIELDS[i])
}

/// Value range a layer is clamped to after a stroke.
pub fn clamp_range(layer: &str) -> (f64, f64) {
    match layer {
        "elevation" => (-12000.0, 12000.0),
        "temperature_bias" => (-60.0, 60.0),
        "rainfall_bias" => (-1.0, 5.0),
        _ => (0.0, f64::MAX),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub value: Vec<f64>,
    pub mask: Vec<f64>,
}

impl Layer {
    pub fn new(n: usize) -> Layer { Layer { value: vec![0.0; n], mask: vec![0.0; n] } }
    pub fn painted_cells(&self) -> usize { self.mask.iter().filter(|m| **m > 0.0).count() }
}

/// All paint layers of one world (created lazily on first use).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paint {
    pub layers: BTreeMap<String, Layer>,
}

impl Paint {
    pub fn layer(&self, name: &str) -> Option<&Layer> { self.layers.get(name) }
    pub fn layer_mut(&mut self, name: &str, n: usize) -> &mut Layer { self.layers.entry(name.to_string()).or_insert_with(|| Layer::new(n)) }
}

/// Everything the user has edited on one mesh: paint, locks (field -> cell -> locked value) and realism (0..100 per output field).
#[derive(Debug, Clone, PartialEq)]
pub struct Edits {
    pub paint: Paint,
    pub locks: BTreeMap<String, BTreeMap<u32, f64>>,
    pub realism: BTreeMap<String, f64>,
}

impl Default for Edits {
    fn default() -> Self { Edits { paint: Paint::default(), locks: BTreeMap::new(), realism: OUTPUT_FIELDS.iter().map(|f| (f.to_string(), DEFAULT_REALISM)).collect() } }
}

/// Default realism: 0 = "what I paint is what I get" (the paint is visible at once; the solver still reacts to it).
pub const DEFAULT_REALISM: f64 = 0.0;

impl Edits {
    pub fn r(&self, field: &str) -> f64 { (self.realism.get(field).copied().unwrap_or(DEFAULT_REALISM) / 100.0).clamp(0.0, 1.0) }
    pub fn is_locked(&self, field: &str, cell: usize) -> bool { self.locks.get(field).map(|l| l.contains_key(&(cell as u32))).unwrap_or(false) }
    pub fn lock_count(&self, field: &str) -> usize { self.locks.get(field).map(|l| l.len()).unwrap_or(0) }
}

// ------------------------------------------------------------------------------------------------ blending

/// Blends one continuous field. `base` is the computed value for elevation and ignored (0) for bias layers.
pub fn blend_elevation(computed: &[f64], layer: Option<&Layer>, r: f64) -> Vec<f64> {
    match layer {
        None => computed.to_vec(),
        Some(l) => (0..computed.len()).map(|i| computed[i] + (1.0 - r) * l.mask[i] * (l.value[i] - computed[i])).collect(),
    }
}

/// temperature + (1 - r) * mask * bias
pub fn blend_temperature(computed: &[f64], layer: Option<&Layer>, r: f64) -> Vec<f64> {
    match layer {
        None => computed.to_vec(),
        Some(l) => (0..computed.len()).map(|i| computed[i] + (1.0 - r) * l.mask[i] * l.value[i]).collect(),
    }
}

/// precipitation * (1 + (1 - r) * mask * bias), never below zero.
pub fn blend_precipitation(computed: &[f64], layer: Option<&Layer>, r: f64) -> Vec<f64> {
    match layer {
        None => computed.to_vec(),
        Some(l) => (0..computed.len()).map(|i| (computed[i] * (1.0 + (1.0 - r) * l.mask[i] * l.value[i])).max(0.0)).collect(),
    }
}

/// Category field: the painted id wins where `(1 - r) * mask >= 0.5`.
pub fn blend_biome(computed: &[f64], layer: Option<&Layer>, r: f64) -> Vec<f64> {
    match layer {
        None => computed.to_vec(),
        Some(l) => (0..computed.len()).map(|i| if (1.0 - r) * l.mask[i] >= 0.5 { l.value[i] } else { computed[i] }).collect(),
    }
}

/// Applies locks: locked cells show their locked value whatever the blend says.
pub fn apply_locks(values: &mut [f64], locks: Option<&BTreeMap<u32, f64>>) {
    if let Some(l) = locks {
        for (&c, &v) in l {
            if let Some(x) = values.get_mut(c as usize) {
                *x = v;
            }
        }
    }
}

/// The painted view (r = 0): what the user painted, shown regardless of the slider.
pub fn painted_view(layer_name: &str, computed: &[f64], layer: Option<&Layer>) -> Vec<f64> {
    match layer_name {
        "elevation" => blend_elevation(computed, layer, 0.0),
        "temperature_bias" => blend_temperature(computed, layer, 0.0),
        "rainfall_bias" => blend_precipitation(computed, layer, 0.0),
        _ => blend_biome(computed, layer, 0.0),
    }
}

// ------------------------------------------------------------------------------------------------ strokes

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// Continuous layers: view += amount * w (raise / lower with the sign of `amount`).
    Add,
    /// view += w * strength * (value - view). Biome layer: paints id `value` with coverage w.
    Set,
    /// view += w * strength * (mean of neighbours - view)
    Smooth,
    /// view += w * strength * (target - view), target = `value` or the weighted mean over the stroke
    Flatten,
    /// Returns cells to the computed field: mask *= (1 - w * strength)
    Erase,
}

#[derive(Debug, Clone)]
pub struct StrokeSpec {
    pub op: Op,
    pub cells: Vec<u32>,
    pub weights: Vec<f64>,
    /// Add: delta at weight 1 (field units). Set / Flatten: target. Biome Set: biome id.
    pub value: Option<f64>,
    /// 0..1 strength for Set, Smooth, Flatten, Erase (default 1).
    pub strength: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StrokeEffect {
    /// Cells whose layer entry changed, ascending.
    pub cells: Vec<u32>,
    pub old: Vec<(f64, f64)>,
    pub new: Vec<(f64, f64)>,
    /// Cells of the stroke that were skipped because the output field is locked there.
    pub skipped_locked: usize,
}

/// Applies a stroke to one layer. `base` is the computed field the layer is relative to (elevation) or `None` (bias / biome layers).
/// `adj_offsets`/`adj` give the mesh adjacency for smoothing. Cells for which `locked` is true are skipped and counted.
pub fn apply_stroke(
    layer_name: &str,
    layer: &mut Layer,
    base: Option<&[f64]>,
    adj_offsets: &[u32],
    adj: &[u32],
    spec: &StrokeSpec,
    locked: &dyn Fn(usize) -> bool,
) -> ApiResult<StrokeEffect> {
    let n = layer.value.len();
    if spec.cells.len() != spec.weights.len() {
        return Err(ApiError::new("bad_stroke", format!("{} cells but {} weights", spec.cells.len(), spec.weights.len())));
    }
    let is_biome = layer_name == "biome_override";
    match (is_biome, spec.op) {
        (true, Op::Add | Op::Smooth | Op::Flatten) => return Err(ApiError::new("bad_op", "the biome layer supports only `set` and `erase`")),
        (_, Op::Add) if spec.value.is_none() => return Err(ApiError::new("bad_stroke", "`add` needs `value` (the change at full weight)")),
        (_, Op::Set) if spec.value.is_none() => return Err(ApiError::new("bad_stroke", "`set` needs `value`")),
        _ => {}
    }
    if !(0.0..=1.0).contains(&spec.strength) || !spec.strength.is_finite() {
        return Err(ApiError::new("bad_stroke", "strength must be in [0, 1]"));
    }
    if let Some(v) = spec.value {
        if !v.is_finite() {
            return Err(ApiError::new("bad_stroke", "value must be finite"));
        }
    }
    // dedupe (max weight per cell), validate, ascending order => deterministic
    let mut w: BTreeMap<u32, f64> = BTreeMap::new();
    for (k, &c) in spec.cells.iter().enumerate() {
        if c as usize >= n {
            return Err(ApiError::new("bad_cell", format!("cell {c} out of range (n_cells = {n})")));
        }
        let x = spec.weights[k];
        if !x.is_finite() {
            return Err(ApiError::new("bad_stroke", "weights must be finite"));
        }
        let x = x.clamp(0.0, 1.0);
        let e = w.entry(c).or_insert(0.0);
        *e = e.max(x);
    }
    let mut skipped = 0;
    let active: Vec<(u32, f64)> = w
        .into_iter()
        .filter(|&(c, x)| {
            if x <= 0.0 {
                return false;
            }
            if locked(c as usize) {
                skipped += 1;
                return false;
            }
            true
        })
        .collect();
    let view0 = |l: &Layer, i: usize| -> f64 {
        match base {
            Some(b) => b[i] + l.mask[i] * (l.value[i] - b[i]),
            None => l.mask[i] * l.value[i],
        }
    };
    let (lo, hi) = clamp_range(layer_name);
    let mut effect = StrokeEffect { cells: vec![], old: vec![], new: vec![], skipped_locked: skipped };
    if active.is_empty() {
        return Ok(effect);
    }
    // targets are computed from the PRE-stroke view so the result does not depend on cell order
    let before: Vec<f64> = if is_biome { vec![] } else { (0..n).map(|i| view0(layer, i)).collect() };
    let flat_target = match spec.op {
        Op::Flatten => Some(spec.value.unwrap_or_else(|| {
            let (mut s, mut sw) = (0.0, 0.0);
            for &(c, x) in &active {
                s += x * before[c as usize];
                sw += x;
            }
            s / sw.max(1e-12)
        })),
        _ => None,
    };
    let mut updates: Vec<(u32, (f64, f64), (f64, f64))> = vec![];
    for &(c, x) in &active {
        let i = c as usize;
        let old = (layer.value[i], layer.mask[i]);
        let new = if is_biome {
            match spec.op {
                Op::Set => {
                    let id = spec.value.unwrap().round().max(0.0);
                    if x >= old.1 { (id, x) } else { old }
                }
                _ => (old.0, old.1 * (1.0 - x * spec.strength)),
            }
        } else if spec.op == Op::Erase {
            (old.0, old.1 * (1.0 - x * spec.strength))
        } else {
            let v = before[i];
            let nv = match spec.op {
                Op::Add => v + spec.value.unwrap() * x,
                Op::Set => v + x * spec.strength * (spec.value.unwrap() - v),
                Op::Smooth => {
                    let (a, b) = (adj_offsets[i] as usize, adj_offsets[i + 1] as usize);
                    let nb = &adj[a..b];
                    let mean = if nb.is_empty() { v } else { nb.iter().map(|&j| before[j as usize]).sum::<f64>() / nb.len() as f64 };
                    v + x * spec.strength * (mean - v)
                }
                Op::Flatten => v + x * spec.strength * (flat_target.unwrap() - v),
                Op::Erase => unreachable!(),
            };
            (nv.clamp(lo, hi), 1.0)
        };
        if new != old {
            updates.push((c, old, new));
        }
    }
    for (c, old, new) in updates {
        layer.value[c as usize] = new.0;
        layer.mask[c as usize] = new.1;
        effect.cells.push(c);
        effect.old.push(old);
        effect.new.push(new);
    }
    Ok(effect)
}

/// Writes `(value, mask)` pairs back (undo / redo of a stroke).
pub fn restore(layer: &mut Layer, cells: &[u32], vals: &[(f64, f64)]) {
    for (k, &c) in cells.iter().enumerate() {
        layer.value[c as usize] = vals[k].0;
        layer.mask[c as usize] = vals[k].1;
    }
}

/// FNV-1a over the exact bytes of an f64 slice: the content hash used for "field versions equal".
pub fn hash_f64(v: &[f64]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for x in v {
        for b in x.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_endpoints_and_midpoint() {
        let c = vec![100.0, 200.0];
        let l = Layer { value: vec![1000.0, 0.0], mask: vec![1.0, 0.0] };
        assert_eq!(blend_elevation(&c, Some(&l), 0.0), vec![1000.0, 200.0]);
        assert_eq!(blend_elevation(&c, Some(&l), 1.0), c);
        assert_eq!(blend_elevation(&c, Some(&l), 0.5), vec![550.0, 200.0]);
        assert_eq!(blend_biome(&[1.0, 1.0], Some(&Layer { value: vec![5.0, 5.0], mask: vec![1.0, 0.4] }), 0.0), vec![5.0, 1.0]);
        assert_eq!(blend_biome(&[1.0], Some(&Layer { value: vec![5.0], mask: vec![1.0] }), 0.5), vec![5.0]);
        assert_eq!(blend_biome(&[1.0], Some(&Layer { value: vec![5.0], mask: vec![1.0] }), 0.6), vec![1.0]);
    }
}
