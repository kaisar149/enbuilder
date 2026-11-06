//! Stage C.5c-1: time-stepped shallow-layer surface wind (CPU reference).
//!
//! Same module name (`wind`), inputs and outputs as `DiagnosticWind`, so downstream modules are unchanged.
//! Register one or the other, never both (`add_atmosphere_stage` vs `add_atmosphere_stage_stepped`).
//!
//! Momentum equation per surface cell (k = local up, u tangent):
//!     du/dt = a - f k x u - (r + c_q |u|) u + F_block,      a = -grad(p_sl)/rho
//!  * r is the same friction rate as the diagnostic wind (water/land, roughness), so the linear part of the
//!    drag is the existing `drag_*` parameters. `drag_quad` = C_d/h (1/m) adds the quadratic term of the PART
//!    file (default 0, which keeps the steady state identical to the diagnostic balance).
//!  * F_block is the 3a up-slope removal expressed as a force: the up-slope component of u is damped at rate
//!    k_b = r b / (1 - b) (capped by `block_rate_max`), with b the 3a blocking fraction. For a scalar balance
//!    this reproduces the 3a result (1 - b) of the free flow.
//!
//! Time stepping (deviation from "explicit forward step", on purpose): the pressure force is explicit, but
//! Coriolis and drag are treated implicitly per cell (a 2x2 solve). Explicit Euler is unstable for rotation
//! (growth sqrt(1 + (f dt)^2) per step), which would break fast-rotation worlds at dt = 600 s. The implicit
//! solve is unconditionally stable and its fixed point is exactly the 3a balance `0 = a - r u - f k x u`.
//!     dt = min(dt_max_s, cfl_wind * min(edge_dist) / max(|u|)),   steps until max|du| per step <= steady_tol
//! for `HOLD` consecutive steps. If `max_steps` is reached first the run fails with `InvalidParam`
//! (never silently returns an unconverged field). Module parameters `last_steps` / `last_residual` hold the
//! outcome (read with `Scheduler::module_param`).
//!
//! The state is kept uncapped; the output goes through the same tanh soft cap as the diagnostic wind (applying
//! the cap every step would act as extra drag and shift the steady state). A hard clamp at 10x `max_wind`
//! guards the state against runaway forcing.
//!
//! Upper level (`wind_upper`) is still the 3a diagnostic balance (open decision 0d).
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::fields::FieldStore;
use crate::gradient::GradientOp;
use crate::math::{cross, dot, length, scale, sub, Vec3};
use crate::module::{DirtyRegion, Module, ModuleSpec, ParamSet};
use crate::backend::PackedMesh;
use crate::wind_gpu::{solve_f32_emulated, StepConfig, StepStatic};
#[cfg(feature = "gpu")]
use crate::wind_gpu::solve_gpu;
use crate::wind::{balance, soft_cap, surface_friction, PARAMS};

const EXTRA: [(&str, f64); 10] = [
    ("dt_max_s", 600.0),
    ("cfl_wind", 0.5),
    ("steady_tol", 1e-3),   // m/s, max change of u over one step
    ("max_steps", 20000.0),
    ("spin_up_steps", 0.0), // > 0: start from the diagnostic balance (no blocking) instead of rest
    ("warm_start", 0.0),    // 1: start from the existing `wind` field if present
    ("drag_quad", 0.0),     // C_d / h, 1/m
    ("block_rate_max", 1e-2), // 1/s cap on the blocking damping rate
    ("backend", 0.0),       // 0 = CPU f64, 1 = f32 kernels emulated on the CPU, 2 = GPU (feature `gpu`)
    ("steps_per_pass", 20.0), // backends 1 and 2: even sub-steps between residual reads
];
/// Consecutive steps below `steady_tol` required to stop.
const HOLD: usize = 5;

/// Which implementation advances the stepped wind (Part 3c-5). Maps onto the `backend` parameter of `wind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// f64 CPU reference (default).
    #[default]
    Cpu,
    /// The f32 kernels emulated on the CPU (no hardware needed; same arithmetic as the GPU).
    CpuF32,
    /// wgpu compute (feature `gpu`; without it the run fails with `Error::NoGpu`).
    Gpu,
}

impl Backend {
    pub fn param_value(self) -> f64 {
        match self { Backend::Cpu => 0.0, Backend::CpuF32 => 1.0, Backend::Gpu => 2.0 }
    }
}

pub struct SteppedWind {
    params: ParamSet,
    /// Device handle kept between runs (seasons) so the adapter is requested once, not per solve.
    #[cfg(feature = "gpu")]
    gpu_ctx: Option<crate::gpu::GpuContext>,
}

impl Default for SteppedWind {
    fn default() -> Self { Self::new() }
}

impl SteppedWind {
    pub fn new() -> Self {
        let mut params = ParamSet::new();
        for (k, v) in PARAMS.iter().chain(EXTRA.iter()) {
            params.set(k, *v);
        }
        params.set("last_steps", 0.0);
        params.set("last_residual", 0.0);
        // output-only timings of the last run (seconds); rejected by `set_param`
        params.set("last_solve_s", 0.0);
        params.set("last_copy_back_s", 0.0);
        SteppedWind {
            params,
            #[cfg(feature = "gpu")]
            gpu_ctx: None,
        }
    }

    /// Same module with the backend preselected.
    pub fn with_backend(backend: Backend) -> Self {
        let mut w = Self::new();
        w.params.set("backend", backend.param_value());
        w
    }
}

/// Part 3c-5: like `add_atmosphere_stage_stepped`, but picks the wind backend with one argument.
/// `Backend::Cpu` is identical to `add_atmosphere_stage_stepped`.
pub fn add_atmosphere_stage_stepped_with(s: &mut crate::scheduler::Scheduler, backend: Backend) -> Result<()> {
    s.register(Box::new(crate::pressure::PressureModel::new()))?;
    s.register(Box::new(SteppedWind::with_backend(backend)))
}

/// Registers `pressure` and the stepped `wind` (alternative to `add_atmosphere_stage`).
pub fn add_atmosphere_stage_stepped(s: &mut crate::scheduler::Scheduler) -> Result<()> {
    s.register(Box::new(crate::pressure::PressureModel::new()))?;
    s.register(Box::new(SteppedWind::new()))
}

impl Module for SteppedWind {
    fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: "wind".into(),
            inputs: ["pressure_sl", "pressure_upper", "air_density", "coriolis_f", "elevation"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            outputs: ["wind", "wind_upper", "wind_speed", "wind_east", "wind_north"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            anomaly_hooks: vec!["wind_forcing".into(), "roughness".into(), "gravity".into()],
        }
    }
    fn params(&self) -> &ParamSet { &self.params }
    fn set_param(&mut self, name: &str, value: f64) -> Result<()> {
        let known = PARAMS.iter().chain(EXTRA.iter()).any(|(k, _)| *k == name);
        if !known || !value.is_finite() {
            return Err(Error::InvalidParam(format!("wind: bad parameter '{name}'")));
        }
        let positive = matches!(
            name,
            "drag_ocean" | "drag_land" | "drag_rough_ref_m" | "drag_upper" | "upper_density_ratio" | "stability_n"
                | "max_wind" | "dt_max_s" | "cfl_wind" | "steady_tol" | "max_steps" | "block_rate_max"
        );
        if (name == "backend" && ![0.0, 1.0, 2.0].contains(&value))
            || (name == "steps_per_pass" && (value < 6.0 || value.fract() != 0.0 || (value as u64) % 2 != 0))
        {
            return Err(Error::InvalidParam(format!("wind: '{name}' out of range")));
        }
        let nonneg = matches!(name, "blocking_gain" | "spin_up_steps" | "drag_quad");
        if (positive && value <= 0.0) || (nonneg && value < 0.0) || (name == "warm_start" && value != 0.0 && value != 1.0) {
            return Err(Error::InvalidParam(format!("wind: '{name}' out of range")));
        }
        self.params.set(name, value);
        Ok(())
    }

    fn run(&mut self, domain: &Domain, fields: &mut FieldStore, _d: &DirtyRegion) -> Result<()> {
        let pset = self.params.clone();
        let p = |k: &str| pset.get_or(k, 0.0);
        let n = domain.n_cells();
        let mesh = &domain.mesh;

        if domain.media.atmosphere.is_none() {
            fields.put("wind", 3, vec![0.0; 3 * n])?;
            fields.put("wind_upper", 3, vec![0.0; 3 * n])?;
            for name in ["wind_speed", "wind_east", "wind_north"] {
                fields.put(name, 1, vec![0.0; n])?;
            }
            self.params.set("last_steps", 0.0);
            self.params.set("last_residual", 0.0);
            self.params.set("last_solve_s", 0.0);
            self.params.set("last_copy_back_s", 0.0);
            return Ok(());
        }

        let psl = fields.data("pressure_sl")?.to_vec();
        let pup = fields.data("pressure_upper")?.to_vec();
        let rho = fields.data("air_density")?.to_vec();
        let f = fields.data("coriolis_f")?.to_vec();
        let z = fields.data("elevation")?.to_vec();

        let op = GradientOp::new(domain);
        let sea = p("sea_level");
        let zc: Vec<f64> = z.iter().map(|&v| v.max(sea)).collect();
        let r = surface_friction(domain, &z, sea, p("drag_ocean"), p("drag_land"), p("drag_rough_ref_m"));

        // constant forcing a = -grad(p)/rho
        let gp = op.gradient(domain, &psl);
        let a: Vec<Vec3> = (0..n).map(|i| scale(gp[i], -1.0 / rho[i].max(1e-9))).collect();

        // blocking geometry (static): unit up-slope direction and rise across one cell
        let gain = p("blocking_gain");
        let mut hat: Vec<Vec3> = vec![[0.0; 3]; n];
        let mut rise = vec![0.0; n];
        if gain > 0.0 {
            let gz = op.gradient(domain, &zc);
            for i in 0..n {
                let s = length(gz[i]);
                if s > 1e-9 {
                    hat[i] = scale(gz[i], 1.0 / s);
                    rise[i] = s * mesh.area[i].sqrt();
                }
            }
        }

        let max_wind = p("max_wind");
        let hard = 10.0 * max_wind;
        let cq = p("drag_quad");
        let stab = p("stability_n");
        let kb_max = p("block_rate_max");
        let tol = p("steady_tol");
        let max_steps = p("max_steps") as usize;
        let dt_max = p("dt_max_s");
        let cfl = p("cfl_wind");
        let min_dx = mesh.edge_dist.iter().cloned().fold(f64::INFINITY, f64::min);

        // initial state
        let mut u: Vec<Vec3> = vec![[0.0; 3]; n];
        if p("warm_start") == 1.0 && fields.has("wind") && fields.layers("wind")? == 3 {
            let w = fields.data("wind")?;
            for i in 0..n {
                u[i] = [w[3 * i], w[3 * i + 1], w[3 * i + 2]];
            }
        } else if p("spin_up_steps") > 0.0 {
            u = balance(domain, &gp, &rho, &r, &f);
        }

        let backend = p("backend") as u32;
        let t_solve = std::time::Instant::now();
        let mut copy_back_s = 0.0f64;
        let (steps, resid) = if backend != 0 {
            let pmean = psl.iter().sum::<f64>() / n as f64;
            let st = StepStatic {
                up: mesh.up.clone(),
                r: r.clone(),
                f: f.clone(),
                rho: rho.clone(),
                hat: hat.clone(),
                rise: rise.clone(),
                p_dev: psl.iter().map(|v| v - pmean).collect(),
                grad_coef: op.coef().to_vec(),
            };
            let cfg = StepConfig {
                dt_max, cfl, tol, max_steps, steps_per_pass: p("steps_per_pass") as usize, hold: HOLD,
                cq, stab, kb_max, gain, max_wind, min_dx,
            };
            let pm = PackedMesh::from_domain(domain);
            let res = if backend == 1 {
                solve_f32_emulated(&pm, &st, &cfg, &u)?
            } else {
                #[cfg(feature = "gpu")]
                {
                    if self.gpu_ctx.is_none() {
                        self.gpu_ctx = Some(crate::gpu::GpuContext::new()?);
                    }
                    solve_gpu(self.gpu_ctx.as_ref().expect("context just created"), &pm, &st, &cfg, &u)?
                }
                #[cfg(not(feature = "gpu"))]
                { return Err(Error::NoGpu("built without the `gpu` cargo feature".into())); }
            };
            u = res.u;
            copy_back_s = res.copy_back_s;
            (res.steps, res.residual)
        } else {
        let mut steps = 0usize;
        let mut resid = f64::INFINITY;
        let mut hold = 0usize;
        while hold < HOLD {
            if steps >= max_steps {
                return Err(Error::InvalidParam(format!(
                    "wind (stepped): not converged after max_steps = {max_steps} (max |du| per step {resid:.3e} m/s > steady_tol {tol:.1e}); raise max_steps or steady_tol"
                )));
            }
            let umax = u.iter().map(|v| length(*v)).fold(0.0, f64::max).min(max_wind).max(1e-3);
            let dt = dt_max.min(cfl * min_dx / umax);
            let mut change = 0.0f64;
            for i in 0..n {
                let old = u[i];
                let rr = r[i] + cq * length(old);
                let rho_i = rr * dt;
                let phi = f[i] * dt;
                // explicit pressure force, then implicit drag + Coriolis
                let us = [old[0] + dt * a[i][0], old[1] + dt * a[i][1], old[2] + dt * a[i][2]];
                let kxu = cross(domain.mesh.up[i], us);
                let den = (1.0 + rho_i) * (1.0 + rho_i) + phi * phi;
                let mut un = scale(sub(scale(us, 1.0 + rho_i), scale(kxu, phi)), 1.0 / den);
                // mountain blocking: implicit damping of the up-slope component
                if gain > 0.0 && rise[i] > 0.0 {
                    let w = dot(un, hat[i]);
                    if w > 0.0 {
                        let b = (gain * rise[i] / (rise[i] + length(un) / stab + 1e-12)).min(1.0);
                        let kb = (r[i] * b / (1.0 - b).max(0.02)).min(kb_max);
                        let damp = kb * dt / (1.0 + kb * dt);
                        un = sub(un, scale(hat[i], damp * w));
                    }
                }
                let sp = length(un);
                if sp > hard {
                    un = scale(un, hard / sp);
                }
                if !(un[0].is_finite() && un[1].is_finite() && un[2].is_finite()) {
                    return Err(Error::NonFinite("wind (stepped)".into()));
                }
                change = change.max(length(sub(un, old)));
                u[i] = un;
            }
            steps += 1;
            resid = change;
            if change <= tol { hold += 1 } else { hold = 0 }
        }
        (steps, resid)
        };
        self.params.set("last_steps", steps as f64);
        self.params.set("last_residual", resid);
        self.params.set("last_solve_s", t_solve.elapsed().as_secs_f64());
        self.params.set("last_copy_back_s", copy_back_s);

        soft_cap(&mut u, max_wind);

        // upper level: 3a diagnostic balance
        let rho_up: Vec<f64> = rho.iter().map(|d| d * p("upper_density_ratio")).collect();
        let r_up = vec![p("drag_upper"); n];
        let mut uu = balance(domain, &op.gradient(domain, &pup), &rho_up, &r_up, &f);
        soft_cap(&mut uu, max_wind);

        let mut w3 = Vec::with_capacity(3 * n);
        let mut u3 = Vec::with_capacity(3 * n);
        let (mut spd, mut we, mut wn) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            w3.extend_from_slice(&u[i]);
            u3.extend_from_slice(&uu[i]);
            spd[i] = length(u[i]);
            we[i] = dot(u[i], domain.frames[i].east);
            wn[i] = dot(u[i], domain.frames[i].north);
        }
        fields.put("wind", 3, w3)?;
        fields.put("wind_upper", 3, u3)?;
        fields.put("wind_speed", 1, spd)?;
        fields.put("wind_east", 1, we)?;
        fields.put("wind_north", 1, wn)?;
        Ok(())
    }
}
