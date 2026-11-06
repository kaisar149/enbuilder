//! Stage C.7c: lift, condensation and precipitation types (pure physics; driven by `MoistureTransport`).
//!
//! All terms act on column vapour V (kg/m^2) over a sub-step dt and return a mass per area (kg/m^2) removed:
//!  * Column capacity: W_sat(T, g) = vapor_scale_ratio * 0.622 * e_sat(T) / |g|  (kg/m^2). Uniform-q column
//!    holds 0.622 e/g; the ratio (default 0.25) accounts for vapour being confined to the lower atmosphere.
//!  * Lift (m/s) = lift_gain * max(u_c . grad z_c, 0), z_c = max(elevation, sea_level), u_c = projected wind.
//!    Air crosses a cell in tau = sqrt(area)/|u_c| and rises `rise = lift * tau` metres (leeward slopes: 0).
//!  * Orographic: parcel cools dry-adiabatically dT = (|g|/cp) * dry_adiabatic_scale * rise; condensate is the
//!    excess C = max(V - W_sat(T - dT, g), 0) and the amount removed in dt is C (1 - exp(-dt/tau)).
//!  * Convective: V (1 - exp(-conv_rate dt)) * ramp(T) * hum * dyn, dyn = clamp(1 - div/conv_div_ref, conv_dyn_min, conv_dyn_max)
//!    (low-level convergence boosts convection, subsidence suppresses it), ramp = clamp((T - conv_threshold_T)/conv_ramp_k, 0, 1).
//!  * Frontal: V (1 - exp(-front_gain * conv * dt)) * hum, conv = max(-wind_divergence, 0) (1/s) of the RAW wind
//!    (the projected wind has no divergence by design, see 3b-2).
//!  * hum = clamp((RH - rh_trigger)/(1 - rh_trigger), 0, 1), RH = V / W_sat(T, g).
//!  * Saturation adjustment (large-scale condensation): max(V - sat_rh W_sat, 0) (1 - exp(-dt/sat_tau_s)). Without it,
//!    mid/high latitudes with no lift or convergence have no rain sink and vapour piles up unphysically.
//! Each term is multiplied by `removal_fraction` and its own on/off switch. If the four terms together would
//! remove more than V they are scaled down proportionally (only then are they coupled).
use crate::domain::Domain;
use crate::moisture::saturation_vapour_pressure;
use crate::module::ParamSet;

pub const PRECIP_PARAMS: [(&str, f64); 19] = [
    ("lift_gain", 3.0),
    ("dry_adiabatic_scale", 1.0),
    ("vapor_scale_ratio", 0.25),
    ("sea_level", 0.0),
    ("oro_on", 1.0),
    ("conv_on", 1.0),
    ("front_on", 1.0),
    ("conv_threshold_T", 298.0),
    ("conv_ramp_k", 5.0),
    ("conv_rate", 4.6e-5), // 1/s (~6 h)
    ("rh_trigger", 0.5),
    ("front_gain", 0.3),
    ("removal_fraction", 1.0),
    ("conv_div_ref", 5.0e-6), // 1/s; convection scales as clamp(1 - div/ref, min, max)
    ("conv_dyn_min", 0.1),
    ("conv_dyn_max", 3.0),
    ("sat_on", 1.0),
    ("sat_rh", 1.0),        // RH above which the column is supersaturated and condenses
    ("sat_tau_s", 21600.0), // 6 h relaxation time of saturation adjustment
];

/// Dry-adiabatic lapse rate (K/m) = |g| / cp.
pub fn dry_adiabatic_lapse(g: f64, cp: f64) -> f64 { g / cp }

#[derive(Debug, Clone)]
pub struct PrecipParams {
    pub dry_adiabatic_scale: f64,
    pub vapor_scale_ratio: f64,
    pub oro_on: bool,
    pub conv_on: bool,
    pub front_on: bool,
    pub conv_threshold_t: f64,
    pub conv_ramp_k: f64,
    pub conv_rate: f64,
    pub rh_trigger: f64,
    pub front_gain: f64,
    pub removal_fraction: f64,
    pub conv_div_ref: f64,
    pub conv_dyn_min: f64,
    pub conv_dyn_max: f64,
    pub sat_on: bool,
    pub sat_rh: f64,
    pub sat_tau_s: f64,
}

impl PrecipParams {
    pub fn from_params(p: &ParamSet) -> Self {
        let g = |k: &str| p.get_or(k, PRECIP_PARAMS.iter().find(|(n, _)| *n == k).map(|x| x.1).unwrap_or(0.0));
        PrecipParams {
            dry_adiabatic_scale: g("dry_adiabatic_scale"),
            vapor_scale_ratio: g("vapor_scale_ratio"),
            oro_on: g("oro_on") != 0.0,
            conv_on: g("conv_on") != 0.0,
            front_on: g("front_on") != 0.0,
            conv_threshold_t: g("conv_threshold_T"),
            conv_ramp_k: g("conv_ramp_k"),
            conv_rate: g("conv_rate"),
            rh_trigger: g("rh_trigger"),
            front_gain: g("front_gain"),
            removal_fraction: g("removal_fraction"),
            conv_div_ref: g("conv_div_ref"),
            conv_dyn_min: g("conv_dyn_min"),
            conv_dyn_max: g("conv_dyn_max"),
            sat_on: g("sat_on") != 0.0,
            sat_rh: g("sat_rh"),
            sat_tau_s: g("sat_tau_s"),
        }
    }
}

/// Static per-cell inputs of the precipitation terms.
#[derive(Debug, Clone)]
pub struct PrecipState {
    pub lift: Vec<f64>,   // m/s
    pub rise: Vec<f64>,   // m crossed per cell
    pub tau: Vec<f64>,    // s to cross a cell
    pub lapse: Vec<f64>,  // K/m (|g|/cp * scale)
    pub g: Vec<f64>,      // |g| m/s^2
    pub temp: Vec<f64>,   // K
    pub conv: Vec<f64>,   // 1/s, max(-raw divergence, 0)
    pub div: Vec<f64>,    // 1/s, signed raw wind divergence
}

impl PrecipState {
    /// `uc` projected wind, `grad_z` gradient of max(elevation, sea_level), `div` raw wind divergence (1/s).
    pub fn build(domain: &Domain, uc: &[[f64; 3]], grad_z: &[[f64; 3]], temp: &[f64], div: &[f64], lift_gain: f64, par: &PrecipParams) -> Self {
        let n = domain.n_cells();
        let cp = domain.media.atmosphere.as_ref().map(|a| a.heat_capacity).unwrap_or(1005.0);
        let (mut lift, mut rise, mut tau, mut lapse, mut g) = (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            let s = (uc[i][0] * uc[i][0] + uc[i][1] * uc[i][1] + uc[i][2] * uc[i][2]).sqrt();
            let along = uc[i][0] * grad_z[i][0] + uc[i][1] * grad_z[i][1] + uc[i][2] * grad_z[i][2];
            lift[i] = lift_gain * along.max(0.0);
            tau[i] = domain.mesh.area[i].sqrt() / s.max(1e-3);
            rise[i] = lift[i] * tau[i];
            g[i] = domain.gravity_magnitude(i).max(1e-6);
            lapse[i] = dry_adiabatic_lapse(g[i], cp) * par.dry_adiabatic_scale;
        }
        PrecipState { lift, rise, tau, lapse, g, temp: temp.to_vec(), conv: div.iter().map(|d| (-d).max(0.0)).collect(), div: div.to_vec() }
    }
}

/// Column saturation capacity (kg/m^2).
pub fn column_capacity(t: f64, g: f64, ratio: f64) -> f64 { ratio * 0.622 * saturation_vapour_pressure(t) / g }

/// Mass per area (kg/m^2) removed from cell `i` in `dt` seconds by (orographic, convective, frontal, saturation-adjustment) processes,
/// given the current column vapour `v`.
pub fn terms(par: &PrecipParams, st: &PrecipState, i: usize, v: f64, dt: f64) -> (f64, f64, f64, f64) {
    if !(v > 0.0) || !(dt > 0.0) {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let w = column_capacity(st.temp[i], st.g[i], par.vapor_scale_ratio);
    let rh = v / w;
    let hum = ((rh - par.rh_trigger) / (1.0 - par.rh_trigger).max(1e-9)).clamp(0.0, 1.0);
    let mut oro = 0.0;
    if par.oro_on && st.rise[i] > 0.0 {
        let cooled = st.temp[i] - st.lapse[i] * st.rise[i];
        let c = (v - column_capacity(cooled, st.g[i], par.vapor_scale_ratio)).max(0.0);
        oro = c * (1.0 - (-dt / st.tau[i]).exp());
    }
    let mut conv = 0.0;
    if par.conv_on {
        let ramp = ((st.temp[i] - par.conv_threshold_t) / par.conv_ramp_k.max(1e-9)).clamp(0.0, 1.0);
        let dynamic = (1.0 - st.div[i] / par.conv_div_ref.max(1e-12)).clamp(par.conv_dyn_min, par.conv_dyn_max);
        conv = v * (1.0 - (-par.conv_rate * dt).exp()) * ramp * hum * dynamic;
    }
    let mut front = 0.0;
    if par.front_on {
        front = v * (1.0 - (-par.front_gain * st.conv[i] * dt).exp()) * hum;
    }
    let mut sat = 0.0;
    if par.sat_on {
        sat = (v - par.sat_rh * w).max(0.0) * (1.0 - (-dt / par.sat_tau_s.max(1e-9)).exp());
    }
    let f = par.removal_fraction;
    let (mut a, mut b, mut c, mut d) = (oro * f, conv * f, front * f, sat * f);
    let tot = a + b + c + d;
    if tot > v {
        let s = v / tot;
        a *= s;
        b *= s;
        c *= s;
        d *= s;
    }
    (a, b, c, d)
}
