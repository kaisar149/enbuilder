//! Part 7a: the anomaly system (magic and non-real physics).
//!
//! An anomaly = shape + falloff + effects (+ rules). It is expressed in the same language as real physics:
//! it never touches results, it only *modifies inputs the physics already reads*.
//!
//! ## Model
//! * `Shape` gives a distance `d >= 0` from the anomaly's core (0 inside). `Falloff` maps `d` to a weight `w` in `[0,1]`
//!   with `w = 1` in the core; `L` (`falloff_m`, scaled per effect by `reach`) is the decay length.
//! * An `Effect` targets a **hook** and applies an operation with weight `w` (times the rule factor):
//!   `Add: v += w m`, `Multiply: v *= 1 + w (m - 1)`, `Override: v = (1 - w) v + w m`. Every op is affine,
//!   and a composition of affine maps is affine, so each hook compiles to two per-cell arrays, `a` and `b`,
//!   with `v' = a v + b` (identity `a = 1, b = 0`). **Source** hooks (base value 0: `temperature_source`,
//!   `wind_forcing`, `charge_potential`, `custom:*`) only keep `b`.
//! * Compiled hooks are written by the module `anomalies` as fields `anom_<hook>_a` / `anom_<hook>_b`
//!   (`:` becomes `_`). Core modules read them *optionally* (`fields.has(..)`, same pattern as `veg_albedo`), so with no
//!   anomaly stage, or with every anomaly muted, the pipeline is bit-identical to the baseline.
//! * `gravity` is special only in WHERE it is applied: modules read gravity from the `Domain`, so
//!   `Anomalies::effective_domain(&base)` returns a copy of the domain whose per-cell gravity magnitude is modified
//!   (direction kept). Nothing else is special-cased: pressure, relief, erosion, vapour capacity all read local `|g|`.
//! * Rules scale the weight over time or by condition (`Periodic`, `When`, `Script`). Rhai scripting is NOT linked
//!   (std-only crate); `Rule::Script` is the seam where a Rhai or WASM engine plugs in.

use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::math::{dot, length, normalize, scale, sub, Vec3};
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::{Arc, Mutex};

// ------------------------------------------------------------------------------------------------ hooks

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// Base value comes from the consumer; compiled to `v' = a v + b`.
    Modifier,
    /// Base value is 0; only `b` is stored (`anom_<hook>_b`).
    Source,
}

/// (name, kind, one-line meaning and unit). Custom scalars are `custom:<name>` (always `Source`).
pub const HOOKS: [(&str, HookKind, &str); 15] = [
    ("gravity", HookKind::Modifier, "|g| per cell, m/s^2 (applied through Anomalies::effective_domain)"),
    ("temperature_source", HookKind::Source, "extra absorbed heat, W/m^2 (energy_balance)"),
    ("albedo", HookKind::Modifier, "surface albedo, clamped to [0,1] (albedo)"),
    ("pressure", HookKind::Modifier, "sea-level pressure, Pa (pressure; wind follows)"),
    ("wind_forcing", HookKind::Source, "pressure-equivalent forcing for the wind only, Pa (wind, wind_stepped)"),
    ("roughness", HookKind::Modifier, "canopy / roughness height, m (wind friction, biomes)"),
    ("evaporation", HookKind::Modifier, "evaporation rate, kg/m^2/s (moisture_source)"),
    ("elevation", HookKind::Modifier, "terrain elevation, m (terrain output)"),
    ("rock_strength", HookKind::Modifier, "relative rock strength (tectonics output)"),
    ("sea_level", HookKind::Modifier, "sea level, m (sea_level module)"),
    ("life_growth", HookKind::Modifier, "growth multiplier on canopy height (biomes) and nutrients"),
    ("time_flow_rate", HookKind::Modifier, "rate multiplier of erosion and sediment steps (1 = normal)"),
    ("light_intensity", HookKind::Modifier, "insolation multiplier (insolation)"),
    ("charge_potential", HookKind::Source, "atmospheric charge potential, normalised (1 = strong storm field); read by `storm`"),
    ("magnetic_field", HookKind::Source, "magnetic field strength, normalised (carried as a field; no module reads it yet)"),
];

pub fn is_custom(hook: &str) -> bool { hook.starts_with("custom:") && hook.len() > 7 }

pub fn hook_kind(hook: &str) -> Option<HookKind> {
    if is_custom(hook) {
        return Some(HookKind::Source);
    }
    HOOKS.iter().find(|h| h.0 == hook).map(|h| h.1)
}

fn key(hook: &str) -> String { hook.replace(':', "_") }
pub fn field_a(hook: &str) -> String { format!("anom_{}_a", key(hook)) }
pub fn field_b(hook: &str) -> String { format!("anom_{}_b", key(hook)) }

/// Apply a compiled Modifier hook to `v` in place (`v = a v + b`). Returns whether the hook was present.
/// Used by core modules; absent fields mean "no anomaly" and `v` is untouched (bit-identical baseline).
pub fn apply_modifier(fields: &FieldStore, hook: &str, v: &mut [f64]) -> Result<bool> {
    let (fa, fb) = (field_a(hook), field_b(hook));
    if !(fields.has(&fa) && fields.has(&fb)) {
        return Ok(false);
    }
    let (a, b) = (fields.data(&fa)?, fields.data(&fb)?);
    if a.len() != v.len() || b.len() != v.len() {
        return Err(Error::InvalidParam(format!("anomaly hook '{hook}': field length differs from target")));
    }
    for i in 0..v.len() {
        v[i] = a[i] * v[i] + b[i];
    }
    Ok(true)
}

/// Effective canopy / roughness height (m) used by wind friction: the vegetation field `veg_roughness_m` (if present with `n` entries)
/// passed through the `roughness` hook (if present). `None` when neither exists, so callers keep their old code path exactly.
pub fn canopy_roughness(fields: &FieldStore, n: usize) -> Result<Option<Vec<f64>>> {
    let mut base: Option<Vec<f64>> = if fields.has("veg_roughness_m") {
        let c = fields.data("veg_roughness_m")?;
        if c.len() == n { Some(c.to_vec()) } else { None }
    } else {
        None
    };
    if fields.has(&field_a("roughness")) && fields.has(&field_b("roughness")) {
        let mut v = base.take().unwrap_or_else(|| vec![0.0; n]);
        apply_modifier(fields, "roughness", &mut v)?;
        for x in v.iter_mut() {
            *x = x.max(0.0);
        }
        base = Some(v);
    }
    Ok(base)
}

/// Compiled Source hook (`anom_<hook>_b`), if present.
pub fn source_of(fields: &FieldStore, hook: &str) -> Result<Option<Vec<f64>>> {
    let fb = field_b(hook);
    if fields.has(&fb) { Ok(Some(fields.data(&fb)?.to_vec())) } else { Ok(None) }
}

// ------------------------------------------------------------------------------------------------ shape

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// 3D ball (world coordinates, cell centroids). `d` = Euclidean distance to the ball.
    Sphere { center: Vec3, radius_m: f64 },
    /// Infinite cylinder around the axis line through `center`. `d` = perpendicular distance minus radius.
    Column { center: Vec3, axis: Vec3, radius_m: f64 },
    /// Segment a-b thickened by `half_width_m`.
    Line { a: Vec3, b: Vec3, half_width_m: f64 },
    /// Axis-aligned box (3D volume; works for sky islands and for shells on a sphere).
    Volume { min: Vec3, max: Vec3 },
    /// Painted cells; `d` = SURFACE distance (Dijkstra on centre-to-centre edge distances) to the painted set.
    Region { cells: Vec<u32> },
    /// Everywhere, `d = 0`.
    Global,
}

impl Shape {
    pub fn kind(&self) -> &'static str {
        match self {
            Shape::Sphere { .. } => "sphere",
            Shape::Column { .. } => "column",
            Shape::Line { .. } => "line",
            Shape::Volume { .. } => "volume",
            Shape::Region { .. } => "region",
            Shape::Global => "global",
        }
    }

    fn validate(&self, n: usize) -> Result<()> {
        let fin = |v: Vec3| v.iter().all(|x| x.is_finite());
        let ok = match self {
            Shape::Sphere { center, radius_m } => fin(*center) && *radius_m >= 0.0 && radius_m.is_finite(),
            Shape::Column { center, axis, radius_m } => fin(*center) && fin(*axis) && length(*axis) > 0.0 && *radius_m >= 0.0 && radius_m.is_finite(),
            Shape::Line { a, b, half_width_m } => fin(*a) && fin(*b) && *half_width_m >= 0.0 && half_width_m.is_finite(),
            Shape::Volume { min, max } => fin(*min) && fin(*max) && (0..3).all(|k| min[k] <= max[k]),
            Shape::Region { cells } => !cells.is_empty() && cells.iter().all(|&c| (c as usize) < n),
            Shape::Global => true,
        };
        if ok { Ok(()) } else { Err(Error::InvalidParam(format!("anomaly shape '{}' is invalid", self.kind()))) }
    }

    /// Distance from every cell to the shape's core (0 inside).
    pub fn distances(&self, domain: &Domain) -> Result<Vec<f64>> {
        let mesh = &domain.mesh;
        let n = mesh.n_cells;
        self.validate(n)?;
        Ok(match self {
            Shape::Global => vec![0.0; n],
            Shape::Sphere { center, radius_m } => (0..n).map(|i| (length(sub(mesh.centroid[i], *center)) - radius_m).max(0.0)).collect(),
            Shape::Column { center, axis, radius_m } => {
                let ax = normalize(*axis);
                (0..n)
                    .map(|i| {
                        let r = sub(mesh.centroid[i], *center);
                        let perp = sub(r, scale(ax, dot(r, ax)));
                        (length(perp) - radius_m).max(0.0)
                    })
                    .collect()
            }
            Shape::Line { a, b, half_width_m } => {
                let ab = sub(*b, *a);
                let l2 = dot(ab, ab);
                (0..n)
                    .map(|i| {
                        let ap = sub(mesh.centroid[i], *a);
                        let t = if l2 > 0.0 { (dot(ap, ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
                        let q = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
                        (length(sub(mesh.centroid[i], q)) - half_width_m).max(0.0)
                    })
                    .collect()
            }
            Shape::Volume { min, max } => (0..n)
                .map(|i| {
                    let c = mesh.centroid[i];
                    let mut s = 0.0;
                    for k in 0..3 {
                        let o = (min[k] - c[k]).max(c[k] - max[k]).max(0.0);
                        s += o * o;
                    }
                    s.sqrt()
                })
                .collect(),
            Shape::Region { cells } => {
                let mut d = vec![f64::INFINITY; n];
                let mut heap: BinaryHeap<Reverse<(u64, usize)>> = BinaryHeap::new();
                for &c in cells {
                    d[c as usize] = 0.0;
                    heap.push(Reverse((0f64.to_bits(), c as usize)));
                }
                while let Some(Reverse((bits, i))) = heap.pop() {
                    let di = f64::from_bits(bits);
                    if di > d[i] {
                        continue;
                    }
                    let (a, b) = (mesh.adj_offsets[i] as usize, mesh.adj_offsets[i + 1] as usize);
                    for k in a..b {
                        let j = mesh.adj[k] as usize;
                        let nd = di + mesh.edge_dist[k];
                        if nd < d[j] {
                            d[j] = nd;
                            heap.push(Reverse((nd.to_bits(), j)));
                        }
                    }
                }
                d
            }
        })
    }
}

// ------------------------------------------------------------------------------------------------ falloff

#[derive(Debug, Clone, PartialEq)]
pub enum Falloff {
    /// 1 in the core, 0 outside.
    Hard,
    /// `1 - d/L`, 0 beyond `L`.
    Linear,
    /// `exp(-(d/L)^2)`, truncated to 0 beyond `3 L` (finite support, so dirty regions stay finite).
    Gaussian,
    /// `1 / (1 + (d/L)^2)`, truncated to 0 beyond `10 L`.
    InverseSquare,
    /// Piecewise-linear curve over `x = d/L`: sorted points `(x, w)`; `w` is held flat before the first point and is 0 after the last.
    Custom(Vec<(f64, f64)>),
}

pub const GAUSSIAN_CUTOFF: f64 = 3.0;
pub const INVERSE_SQUARE_CUTOFF: f64 = 10.0;

impl Falloff {
    pub fn name(&self) -> &'static str {
        match self {
            Falloff::Hard => "hard",
            Falloff::Linear => "linear",
            Falloff::Gaussian => "gaussian",
            Falloff::InverseSquare => "inverse_square",
            Falloff::Custom(_) => "custom",
        }
    }

    pub fn validate(&self) -> Result<()> {
        if let Falloff::Custom(p) = self {
            let ok = !p.is_empty()
                && p.iter().all(|&(x, w)| x.is_finite() && w.is_finite() && x >= 0.0 && (0.0..=1.0).contains(&w))
                && p.windows(2).all(|q| q[0].0 < q[1].0);
            if !ok {
                return Err(Error::InvalidParam("custom falloff: points must have increasing x >= 0 and w in [0,1]".into()));
            }
        }
        Ok(())
    }

    /// Weight at distance `d` outside the core, for decay length `l` (> 0; ignored by `Hard`).
    pub fn weight(&self, d: f64, l: f64) -> f64 {
        if d <= 0.0 {
            return match self {
                Falloff::Custom(p) => p.first().map_or(1.0, |q| if q.0 <= 0.0 { q.1 } else { 1.0 }),
                _ => 1.0,
            };
        }
        let x = d / l;
        match self {
            Falloff::Hard => 0.0,
            Falloff::Linear => (1.0 - x).max(0.0),
            Falloff::Gaussian => if x > GAUSSIAN_CUTOFF { 0.0 } else { (-x * x).exp() },
            Falloff::InverseSquare => if x > INVERSE_SQUARE_CUTOFF { 0.0 } else { 1.0 / (1.0 + x * x) },
            Falloff::Custom(p) => {
                if x <= p[0].0 {
                    return p[0].1;
                }
                for q in p.windows(2) {
                    if x <= q[1].0 {
                        let t = (x - q[0].0) / (q[1].0 - q[0].0);
                        return q[0].1 + t * (q[1].1 - q[0].1);
                    }
                }
                0.0
            }
        }
    }

    /// Largest `d / L` at which the weight can be non-zero (`inf` is never returned: all curves have finite support).
    pub fn support(&self) -> f64 {
        match self {
            Falloff::Hard => 0.0,
            Falloff::Linear => 1.0,
            Falloff::Gaussian => GAUSSIAN_CUTOFF,
            Falloff::InverseSquare => INVERSE_SQUARE_CUTOFF,
            Falloff::Custom(p) => p.last().map_or(0.0, |q| q.0),
        }
    }
}

// ------------------------------------------------------------------------------------------------ effects and rules

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Add,
    Multiply,
    Override,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    /// A name from `HOOKS` or `custom:<name>`.
    pub hook: String,
    pub op: Op,
    pub magnitude: f64,
    /// Multiplies the anomaly's `falloff_m` for this effect (heat can reach farther than gravity).
    pub reach: f64,
}

impl Effect {
    pub fn new(hook: &str, op: Op, magnitude: f64) -> Self { Effect { hook: hook.to_string(), op, magnitude, reach: 1.0 } }
    pub fn add(hook: &str, m: f64) -> Self { Self::new(hook, Op::Add, m) }
    pub fn multiply(hook: &str, m: f64) -> Self { Self::new(hook, Op::Multiply, m) }
    pub fn set(hook: &str, m: f64) -> Self { Self::new(hook, Op::Override, m) }
    pub fn with_reach(mut self, r: f64) -> Self { self.reach = r; self }
}

/// What a rule can look at when it is evaluated.
pub struct RuleContext<'a> {
    pub time_s: f64,
    pub fields: Option<&'a FieldStore>,
    /// Area-weighted mean of the anomaly's weight is not needed; rules are global (one factor per anomaly).
    pub anomaly: &'a str,
}

#[derive(Clone)]
pub struct ScriptRule(pub Arc<dyn Fn(&RuleContext) -> f64 + Send + Sync>);

impl std::fmt::Debug for ScriptRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "ScriptRule(..)") }
}
impl PartialEq for ScriptRule {
    fn eq(&self, o: &Self) -> bool { Arc::ptr_eq(&self.0, &o.0) }
}

/// A rule returns a factor in `[0, 1]` that multiplies the anomaly's weight. All rules of an anomaly multiply.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// Pulses: on for `duty` of every `period_s` (shifted by `phase`, in periods), `low` factor while off.
    Periodic { period_s: f64, duty: f64, phase: f64, low: f64 },
    /// Conditional: `factor_on` while the mean of `field` (first layer, area weighted) is `>= at_least`, else `factor_off`.
    /// Needs the field to exist when the anomaly stage runs (it reads the previous pass / warm start);
    /// without it (or in `effective_domain`) `factor_off` applies.
    When { field: String, at_least: f64, factor_on: f64, factor_off: f64 },
    /// Host-supplied code (the seam for Rhai / WASM).
    Script(ScriptRule),
}

impl Rule {
    pub fn validate(&self) -> Result<()> {
        let ok = match self {
            Rule::Periodic { period_s, duty, phase, low } => *period_s > 0.0 && period_s.is_finite() && (0.0..=1.0).contains(duty) && phase.is_finite() && (0.0..=1.0).contains(low),
            Rule::When { at_least, factor_on, factor_off, .. } => at_least.is_finite() && (0.0..=1.0).contains(factor_on) && (0.0..=1.0).contains(factor_off),
            Rule::Script(_) => true,
        };
        if ok { Ok(()) } else { Err(Error::InvalidParam("anomaly rule has out-of-range values".into())) }
    }

    pub fn factor(&self, ctx: &RuleContext, domain: &Domain) -> f64 {
        match self {
            Rule::Periodic { period_s, duty, phase, low } => {
                let x = (ctx.time_s / period_s + phase).rem_euclid(1.0);
                if x < *duty { 1.0 } else { *low }
            }
            Rule::When { field, at_least, factor_on, factor_off } => {
                let Some(f) = ctx.fields else { return *factor_off };
                let Ok(d) = f.data(field) else { return *factor_off };
                let n = domain.n_cells();
                if d.is_empty() || d.len() % n != 0 {
                    return *factor_off;
                }
                let l = d.len() / n;
                let mut s = 0.0;
                let mut a = 0.0;
                for i in 0..n {
                    s += d[i * l] * domain.mesh.area[i];
                    a += domain.mesh.area[i];
                }
                if s / a >= *at_least { *factor_on } else { *factor_off }
            }
            Rule::Script(s) => (s.0)(ctx).clamp(0.0, 1.0),
        }
    }
}

// ------------------------------------------------------------------------------------------------ anomaly and set

#[derive(Debug, Clone, PartialEq)]
pub struct Anomaly {
    pub name: String,
    pub shape: Shape,
    pub falloff: Falloff,
    /// Decay length `L` in metres (scaled by each effect's `reach`). Must be > 0 unless the falloff is `Hard`.
    pub falloff_m: f64,
    pub effects: Vec<Effect>,
    pub rules: Vec<Rule>,
    pub muted: bool,
}

impl Anomaly {
    pub fn new(name: &str, shape: Shape, falloff: Falloff, falloff_m: f64) -> Self {
        Anomaly { name: name.to_string(), shape, falloff, falloff_m, effects: vec![], rules: vec![], muted: false }
    }
    pub fn with_effect(mut self, e: Effect) -> Self { self.effects.push(e); self }
    pub fn with_rule(mut self, r: Rule) -> Self { self.rules.push(r); self }
    pub fn muted(mut self, m: bool) -> Self { self.muted = m; self }

    pub fn validate(&self, n_cells: usize) -> Result<()> {
        if self.name.is_empty() {
            return Err(Error::InvalidParam("anomaly needs a name".into()));
        }
        self.shape.validate(n_cells)?;
        self.falloff.validate()?;
        if !self.falloff_m.is_finite() || self.falloff_m < 0.0 || (self.falloff != Falloff::Hard && self.falloff_m <= 0.0) {
            return Err(Error::InvalidParam(format!("anomaly '{}': falloff_m must be > 0", self.name)));
        }
        for e in &self.effects {
            if hook_kind(&e.hook).is_none() {
                return Err(Error::InvalidParam(format!("anomaly '{}': unknown hook '{}'", self.name, e.hook)));
            }
            if !e.magnitude.is_finite() || !(e.reach.is_finite() && e.reach > 0.0) {
                return Err(Error::InvalidParam(format!("anomaly '{}': bad effect on '{}'", self.name, e.hook)));
            }
        }
        for r in &self.rules {
            r.validate()?;
        }
        Ok(())
    }

    /// Weight of this anomaly for one effect reach, from precomputed core distances.
    pub fn weights(&self, dist: &[f64], reach: f64) -> Vec<f64> {
        let l = (self.falloff_m * reach).max(f64::MIN_POSITIVE);
        dist.iter().map(|&d| self.falloff.weight(d, l)).collect()
    }
}

/// Per-hook compiled modifier.
#[derive(Debug, Clone, PartialEq)]
pub struct HookMod {
    pub kind: HookKind,
    pub a: Vec<f64>,
    pub b: Vec<f64>,
}

impl HookMod {
    fn identity(kind: HookKind, n: usize) -> Self { HookMod { kind, a: vec![1.0; n], b: vec![0.0; n] } }
    pub fn is_identity(&self) -> bool { self.a.iter().all(|&a| a == 1.0) && self.b.iter().all(|&b| b == 0.0) }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Compiled {
    pub hooks: BTreeMap<String, HookMod>,
}

impl Compiled {
    /// Write `anom_<hook>_a/_b` (Source hooks: `_b` only).
    pub fn write(&self, fields: &mut FieldStore) -> Result<()> {
        for (h, m) in &self.hooks {
            if m.kind == HookKind::Modifier {
                fields.put(&field_a(h), 1, m.a.clone())?;
            }
            fields.put(&field_b(h), 1, m.b.clone())?;
        }
        Ok(())
    }
    pub fn output_fields(hooks: &[String]) -> Vec<String> {
        let mut v = vec![];
        for h in hooks {
            if hook_kind(h) == Some(HookKind::Modifier) {
                v.push(field_a(h));
            }
            v.push(field_b(h));
        }
        v
    }
}

#[derive(Debug, Clone, Default)]
pub struct AnomalySet {
    pub anomalies: Vec<Anomaly>,
    pub time_s: f64,
}

impl AnomalySet {
    pub fn new() -> Self { Self::default() }

    pub fn add(&mut self, a: Anomaly) -> Result<()> {
        if self.anomalies.iter().any(|x| x.name == a.name) {
            return Err(Error::InvalidParam(format!("anomaly '{}' already exists", a.name)));
        }
        self.anomalies.push(a);
        Ok(())
    }
    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.anomalies.len();
        self.anomalies.retain(|a| a.name != name);
        self.anomalies.len() != n
    }
    pub fn get(&self, name: &str) -> Option<&Anomaly> { self.anomalies.iter().find(|a| a.name == name) }
    pub fn set_muted(&mut self, name: &str, muted: bool) -> Result<()> {
        match self.anomalies.iter_mut().find(|a| a.name == name) {
            Some(a) => {
                a.muted = muted;
                Ok(())
            }
            None => Err(Error::InvalidParam(format!("no anomaly named '{name}'"))),
        }
    }

    /// Every hook mentioned by any anomaly, muted or not (sorted).
    pub fn hooks(&self) -> Vec<String> {
        let mut v: Vec<String> = self.anomalies.iter().flat_map(|a| a.effects.iter().map(|e| e.hook.clone())).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Compile the unmuted anomalies, in insertion order, into per-hook `a`/`b` arrays.
    /// `fields` is only used by `Rule::When`.
    pub fn compile(&self, domain: &Domain, fields: Option<&FieldStore>) -> Result<Compiled> {
        let n = domain.n_cells();
        let mut out = Compiled::default();
        for h in self.hooks() {
            out.hooks.insert(h.clone(), HookMod::identity(hook_kind(&h).unwrap_or(HookKind::Source), n));
        }
        for a in &self.anomalies {
            a.validate(n)?;
        }
        for a in self.anomalies.iter().filter(|a| !a.muted) {
            let dist = a.shape.distances(domain)?;
            let ctx = RuleContext { time_s: self.time_s, fields, anomaly: &a.name };
            let rule: f64 = a.rules.iter().map(|r| r.factor(&ctx, domain)).product();
            for e in &a.effects {
                let w = a.weights(&dist, e.reach);
                let m = out.hooks.get_mut(&e.hook).expect("hook table built from the same set");
                for i in 0..n {
                    let wi = w[i] * rule;
                    if wi == 0.0 {
                        continue;
                    }
                    let (ca, cb) = match e.op {
                        Op::Add => (1.0, wi * e.magnitude),
                        Op::Multiply => (1.0 + wi * (e.magnitude - 1.0), 0.0),
                        Op::Override => (1.0 - wi, wi * e.magnitude),
                    };
                    // new = ca * (a v + b) + cb
                    m.a[i] *= ca;
                    m.b[i] = ca * m.b[i] + cb;
                }
            }
        }
        Ok(out)
    }

    /// Copy of `base` whose gravity magnitude is modified by the `gravity` hook (direction kept).
    /// Returns an exact clone when no unmuted anomaly touches gravity.
    pub fn effective_domain(&self, base: &Domain) -> Result<Domain> {
        let mut d = base.clone();
        if !self.anomalies.iter().any(|a| !a.muted && a.effects.iter().any(|e| e.hook == "gravity")) {
            return Ok(d);
        }
        let c = self.compile(base, None)?;
        let Some(m) = c.hooks.get("gravity") else { return Ok(d) };
        for i in 0..d.n_cells() {
            let g = base.gravity[i];
            let gm = length(g);
            if m.a[i] == 1.0 && m.b[i] == 0.0 {
                continue;
            }
            let g2 = m.a[i] * gm + m.b[i];
            if !(g2 > 0.0) || !g2.is_finite() {
                return Err(Error::InvalidParam(format!("gravity anomaly makes |g| non-positive at cell {i}")));
            }
            d.gravity[i] = if gm > 0.0 { scale(g, g2 / gm) } else { g };
        }
        Ok(d)
    }
}

// ------------------------------------------------------------------------------------------------ shared handle and module

/// Shared, thread-safe handle so a UI (inspector) can mute or edit anomalies between solver runs.
#[derive(Debug, Clone, Default)]
pub struct Anomalies {
    inner: Arc<Mutex<AnomalySet>>,
}

impl Anomalies {
    pub fn new() -> Self { Self::default() }
    pub fn from_set(s: AnomalySet) -> Self { Anomalies { inner: Arc::new(Mutex::new(s)) } }
    fn lock(&self) -> std::sync::MutexGuard<'_, AnomalySet> { self.inner.lock().unwrap_or_else(|e| e.into_inner()) }
    pub fn snapshot(&self) -> AnomalySet { self.lock().clone() }
    pub fn add(&self, a: Anomaly) -> Result<()> { self.lock().add(a) }
    pub fn remove(&self, name: &str) -> bool { self.lock().remove(name) }
    pub fn set_muted(&self, name: &str, muted: bool) -> Result<()> { self.lock().set_muted(name, muted) }
    pub fn set_time(&self, t: f64) { self.lock().time_s = t; }
    pub fn hooks(&self) -> Vec<String> { self.lock().hooks() }
    pub fn effective_domain(&self, base: &Domain) -> Result<Domain> { self.lock().effective_domain(base) }
}

/// Module `anomalies`: no inputs; writes the compiled hook fields. Register it FIRST so it runs first
/// (modules read the hook fields optionally, so the scheduler does not see that dependency).
pub struct AnomalyStage {
    handle: Anomalies,
    declared: Vec<String>,
    params: ParamSet,
}

impl AnomalyStage {
    /// Declares the hooks currently mentioned by the set; later anomalies may only use these hooks
    /// (the module's outputs are part of its static spec). Use `with_hooks` to pre-declare more.
    pub fn new(handle: Anomalies) -> Self {
        let declared = handle.hooks();
        AnomalyStage { handle, declared, params: ParamSet::new().with("n_active", 0.0).with("n_muted", 0.0) }
    }
    pub fn with_hooks(mut self, extra: &[&str]) -> Self {
        for h in extra {
            if !self.declared.iter().any(|d| d == h) {
                self.declared.push(h.to_string());
            }
        }
        self.declared.sort();
        self
    }
}

impl Module for AnomalyStage {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec { name: "anomalies".into(), inputs: vec![], outputs: Compiled::output_fields(&self.declared), anomaly_hooks: vec![] }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, _v: f64) -> Result<()> { Err(Error::InvalidParam(format!("anomalies: parameter '{name}' is output-only or unknown"))) }
    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let set = self.handle.snapshot();
        for h in set.hooks() {
            if !self.declared.contains(&h) {
                return Err(Error::InvalidParam(format!("anomalies: hook '{h}' was not declared when the stage was registered (use with_hooks)")));
            }
        }
        let mut c = set.compile(domain, Some(fields))?;
        // declared-but-unused hooks still need their outputs (identity)
        for h in &self.declared {
            c.hooks.entry(h.clone()).or_insert_with(|| HookMod::identity(hook_kind(h).unwrap_or(HookKind::Source), domain.n_cells()));
        }
        c.write(fields)?;
        let muted = set.anomalies.iter().filter(|a| a.muted).count();
        self.params.set("n_active", (set.anomalies.len() - muted) as f64);
        self.params.set("n_muted", muted as f64);
        Ok(())
    }
}

/// Register the anomaly stage. Call BEFORE the climate stages.
pub fn add_anomaly_stage(s: &mut crate::scheduler::Scheduler, handle: &Anomalies) -> Result<()> { s.register(Box::new(AnomalyStage::new(handle.clone()))) }

// ------------------------------------------------------------------------------------------------ inspector data

#[derive(Debug, Clone, PartialEq)]
pub struct AnomalyInfo {
    pub name: String,
    pub muted: bool,
    pub shape: &'static str,
    pub falloff: &'static str,
    pub falloff_m: f64,
    pub hooks: Vec<String>,
    pub rules: usize,
    /// Cells where the anomaly's weight exceeds `1e-3` (for the widest effect), and their area (m^2).
    pub cells_influenced: usize,
    pub area_influenced_m2: f64,
    /// Farthest core distance with non-zero weight (m), widest effect.
    pub reach_m: f64,
}

/// The anomaly list for the inspector (name, mute state, what it touches, how far it reaches).
pub fn inspect(domain: &Domain, set: &AnomalySet) -> Result<Vec<AnomalyInfo>> {
    let mut out = vec![];
    for a in &set.anomalies {
        a.validate(domain.n_cells())?;
        let dist = a.shape.distances(domain)?;
        let reach = a.effects.iter().map(|e| e.reach).fold(1.0f64, f64::max);
        let w = a.weights(&dist, reach);
        let mut hooks: Vec<String> = a.effects.iter().map(|e| e.hook.clone()).collect();
        hooks.sort();
        hooks.dedup();
        let (mut cells, mut area, mut far) = (0usize, 0.0, 0.0f64);
        for i in 0..w.len() {
            if w[i] > 1e-3 {
                cells += 1;
                area += domain.mesh.area[i];
                far = far.max(dist[i]);
            }
        }
        out.push(AnomalyInfo { name: a.name.clone(), muted: a.muted, shape: a.shape.kind(), falloff: a.falloff.name(), falloff_m: a.falloff_m, hooks, rules: a.rules.len(), cells_influenced: cells, area_influenced_m2: area, reach_m: far });
    }
    Ok(out)
}

/// Direct impact of ONE anomaly on every hook it targets, per cell, ignoring mute state and rules:
/// `Add` -> `w m`; `Multiply` -> `w (m - 1)` (relative change); `Override` -> `w` (blend fraction toward `m`).
/// Several effects on one hook are summed. This is the "impact map per hook"; `field_impact` gives the impact
/// on any downstream field by re-solving with the anomaly muted.
pub fn impact_map(domain: &Domain, anomaly: &Anomaly) -> Result<BTreeMap<String, Vec<f64>>> {
    anomaly.validate(domain.n_cells())?;
    let dist = anomaly.shape.distances(domain)?;
    let mut out: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for e in &anomaly.effects {
        let w = anomaly.weights(&dist, e.reach);
        let v = out.entry(e.hook.clone()).or_insert_with(|| vec![0.0; w.len()]);
        for i in 0..w.len() {
            v[i] += match e.op {
                Op::Add => w[i] * e.magnitude,
                Op::Multiply => w[i] * (e.magnitude - 1.0),
                Op::Override => w[i],
            };
        }
    }
    Ok(out)
}

/// Impact of one anomaly on a downstream field: `solve` is the caller's pipeline (build effective domain, run the
/// scheduler to convergence, return the field). It is run twice on fresh stores: with the anomaly and with it muted.
/// Returns `with - without` per cell (layer 0 if the field has layers). The anomaly's mute state is restored.
pub fn field_impact<F>(handle: &Anomalies, name: &str, field: &str, mut solve: F) -> Result<Vec<f64>>
where
    F: FnMut(&Anomalies) -> Result<FieldStore>,
{
    let was = handle.snapshot().get(name).map(|a| a.muted).ok_or_else(|| Error::InvalidParam(format!("no anomaly named '{name}'")))?;
    handle.set_muted(name, false)?;
    let with = solve(handle);
    handle.set_muted(name, true)?;
    let without = solve(handle);
    handle.set_muted(name, was)?;
    let (with, without) = (with?, without?);
    let l = with.layers(field)?;
    let (a, b) = (with.data(field)?, without.data(field)?);
    if a.len() != b.len() {
        return Err(Error::InvalidParam("field_impact: field sizes differ".into()));
    }
    Ok((0..a.len() / l).map(|i| a[i * l] - b[i * l]).collect())
}
