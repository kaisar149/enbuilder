//! Part 3c-3: the stepped-wind momentum update as compute kernels (f32), plus a CPU emulation of the exact
//! same kernels over the exact same packed buffers (`solve_f32_emulated`), so buffer layout, double
//! buffering, the two-stage reduction and the host loop are testable without a GPU.
//!
//! Kernels (one thread per cell, workgroup 64; WGSL source in `WGSL_*`, emulated by `k_*`):
//!  * `pressure_gradient`: a_i = -(sum_k coef_k (pd_j - pd_i)) / rho_i with the CPU `GradientOp` weights
//!    (3 f32 per adjacency entry) and pd = p_sl - mean (keeps f32 differences exact enough). Once per solve.
//!  * `momentum_step`: reads only `u_in`, writes only `u_out` (+ per-cell change `chg`): explicit pressure
//!    force, implicit drag + Coriolis, implicit up-slope blocking, hard clamp. Same maths as `SteppedWind`.
//!  * `residual_reduce` (2 passes): per-workgroup max of (chg, |u|) into `partial`, then one workgroup
//!    reduces `partial` to `result = [max change, max speed]`.
//!
//! Host loop: passes of `steps_per_pass` (even) sub-steps with dt fixed per pass from the max speed read
//! at the previous pass end (CFL rule), double buffer u_a/u_b (sub-step s reads buffer s%2; even passes end
//! in u_a), residual read once per pass = max change over the last `hold` sub-steps of the pass; stop when it
//! is <= steady_tol, error when `max_steps` is exhausted.
//!
//! Static data (f32, packed once): `stat`, stride 12 per cell:
//!   [up.x up.y up.z, r, f, rho, hat.x hat.y hat.z, rise, 0, 0]
use crate::backend::PackedMesh;
use crate::error::{Error, Result};
use crate::math::Vec3;

pub const STAT_STRIDE: usize = 12;
pub const WG: usize = 64;

/// Static per-cell inputs in f64 (computed on the host once per solve).
pub struct StepStatic {
    pub up: Vec<Vec3>,
    pub r: Vec<f64>,
    pub f: Vec<f64>,
    pub rho: Vec<f64>,
    pub hat: Vec<Vec3>,
    pub rise: Vec<f64>,
    /// pressure_sl minus its mean (Pa).
    pub p_dev: Vec<f64>,
    /// `GradientOp` coefficients, parallel to the adjacency.
    pub grad_coef: Vec<Vec3>,
}

#[derive(Debug, Clone)]
pub struct StepConfig {
    pub dt_max: f64,
    pub cfl: f64,
    pub tol: f64,
    pub max_steps: usize,
    pub steps_per_pass: usize,
    pub hold: usize,
    pub cq: f64,
    pub stab: f64,
    pub kb_max: f64,
    pub gain: f64,
    pub max_wind: f64,
    pub min_dx: f64,
}

#[derive(Debug, Clone)]
pub struct StepResult {
    /// Uncapped state (f64 copy of the f32 result).
    pub u: Vec<Vec3>,
    pub steps: usize,
    pub passes: usize,
    pub residual: f64,
    /// Wall time (s) of the final full-field device -> host copy of the wind (0 for the emulated backend).
    /// This is the once-per-solve (once per season) copy of 3c-5; the per-pass 2-float reads are not included.
    pub copy_back_s: f64,
}

pub fn pack_static(st: &StepStatic) -> Vec<f32> {
    let n = st.r.len();
    let mut v = vec![0.0f32; n * STAT_STRIDE];
    for i in 0..n {
        let s = &mut v[i * STAT_STRIDE..(i + 1) * STAT_STRIDE];
        for c in 0..3 {
            s[c] = st.up[i][c] as f32;
            s[6 + c] = st.hat[i][c] as f32;
        }
        s[3] = st.r[i] as f32;
        s[4] = st.f[i] as f32;
        s[5] = st.rho[i] as f32;
        s[9] = st.rise[i] as f32;
    }
    v
}

pub fn pack_vec3(v: &[Vec3]) -> Vec<f32> { v.iter().flat_map(|a| a.iter().map(|&x| x as f32)).collect() }

fn sub_step_params(cfg: &StepConfig, dt: f64) -> [f32; 8] {
    [dt as f32, cfg.cq as f32, cfg.stab as f32, cfg.kb_max as f32, cfg.gain as f32, (10.0 * cfg.max_wind) as f32, 0.0, 0.0]
}

fn validate(cfg: &StepConfig) -> Result<()> {
    if cfg.steps_per_pass < cfg.hold || cfg.steps_per_pass % 2 != 0 {
        return Err(Error::InvalidParam(format!("steps_per_pass must be even and >= {} (got {})", cfg.hold, cfg.steps_per_pass)));
    }
    Ok(())
}

// ----------------------------------------------------------------------------------- emulated kernels

/// Emulates `pressure_gradient`.
pub fn k_pressure_gradient(pm: &PackedMesh, coef3: &[f32], pd: &[f32], stat: &[f32]) -> Vec<f32> {
    let mut a = vec![0.0f32; 3 * pm.n_cells];
    for i in 0..pm.n_cells {
        let (mut gx, mut gy, mut gz) = (0.0f32, 0.0f32, 0.0f32);
        for k in pm.offsets[i] as usize..pm.offsets[i + 1] as usize {
            let d = pd[pm.adj[k] as usize] - pd[i];
            gx += coef3[3 * k] * d;
            gy += coef3[3 * k + 1] * d;
            gz += coef3[3 * k + 2] * d;
        }
        let inv = -1.0f32 / stat[i * STAT_STRIDE + 5].max(1e-9);
        a[3 * i] = gx * inv;
        a[3 * i + 1] = gy * inv;
        a[3 * i + 2] = gz * inv;
    }
    a
}

/// Emulates `momentum_step` for one sub-step. `reset` = start a new change window, else keep the running max.
pub fn k_momentum_step(stat: &[f32], a: &[f32], p: &[f32; 8], reset: bool, u_in: &[f32], u_out: &mut [f32], chg: &mut [f32]) {
    let (dt, cq, stab, kbmax, gain, hard) = (p[0], p[1], p[2], p[3], p[4], p[5]);
    let n = stat.len() / STAT_STRIDE;
    for i in 0..n {
        let s = &stat[i * STAT_STRIDE..(i + 1) * STAT_STRIDE];
        let up = [s[0], s[1], s[2]];
        let (r, f, hat, rise) = (s[3], s[4], [s[6], s[7], s[8]], s[9]);
        let old = [u_in[3 * i], u_in[3 * i + 1], u_in[3 * i + 2]];
        let sp_old = (old[0] * old[0] + old[1] * old[1] + old[2] * old[2]).sqrt();
        let rho_i = (r + cq * sp_old) * dt;
        let phi = f * dt;
        let us = [old[0] + dt * a[3 * i], old[1] + dt * a[3 * i + 1], old[2] + dt * a[3 * i + 2]];
        let kxu = [up[1] * us[2] - up[2] * us[1], up[2] * us[0] - up[0] * us[2], up[0] * us[1] - up[1] * us[0]];
        let den = (1.0 + rho_i) * (1.0 + rho_i) + phi * phi;
        let mut un = [0.0f32; 3];
        for c in 0..3 {
            un[c] = (us[c] * (1.0 + rho_i) - kxu[c] * phi) / den;
        }
        if gain > 0.0 && rise > 0.0 {
            let w = un[0] * hat[0] + un[1] * hat[1] + un[2] * hat[2];
            if w > 0.0 {
                let sp = (un[0] * un[0] + un[1] * un[1] + un[2] * un[2]).sqrt();
                let b = (gain * rise / (rise + sp / stab + 1e-12)).min(1.0);
                let kb = (r * b / (1.0 - b).max(0.02)).min(kbmax);
                let damp = kb * dt / (1.0 + kb * dt);
                for c in 0..3 {
                    un[c] -= hat[c] * damp * w;
                }
            }
        }
        let sp = (un[0] * un[0] + un[1] * un[1] + un[2] * un[2]).sqrt();
        if sp > hard {
            for c in 0..3 {
                un[c] *= hard / sp;
            }
        }
        let d = [un[0] - old[0], un[1] - old[1], un[2] - old[2]];
        let ch = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        chg[i] = if reset { ch } else { chg[i].max(ch) };
        u_out[3 * i..3 * i + 3].copy_from_slice(&un);
    }
}

/// Emulates the two-stage `residual_reduce`: returns [max change, max speed].
pub fn k_residual_reduce(chg: &[f32], u: &[f32]) -> [f32; 2] {
    let n = chg.len();
    let nwg = n.div_ceil(WG);
    let mut partial = vec![0.0f32; 2 * nwg];
    for g in 0..nwg {
        let (mut mc, mut ms) = (0.0f32, 0.0f32);
        for i in g * WG..((g + 1) * WG).min(n) {
            mc = mc.max(chg[i]);
            ms = ms.max((u[3 * i] * u[3 * i] + u[3 * i + 1] * u[3 * i + 1] + u[3 * i + 2] * u[3 * i + 2]).sqrt());
        }
        partial[2 * g] = mc;
        partial[2 * g + 1] = ms;
    }
    let (mut mc, mut ms) = (0.0f32, 0.0f32);
    for g in 0..nwg {
        mc = mc.max(partial[2 * g]);
        ms = ms.max(partial[2 * g + 1]);
    }
    [mc, ms]
}

fn dt_for_pass(cfg: &StepConfig, umax: f64) -> f64 {
    cfg.dt_max.min(cfg.cfl * cfg.min_dx / umax.clamp(1e-3, cfg.max_wind))
}

/// The whole solve through the emulated kernels (same host loop as the GPU path).
pub fn solve_f32_emulated(pm: &PackedMesh, st: &StepStatic, cfg: &StepConfig, u0: &[Vec3]) -> Result<StepResult> {
    validate(cfg)?;
    let n = pm.n_cells;
    let stat = pack_static(st);
    let a = k_pressure_gradient(pm, &pack_vec3(&st.grad_coef), &st.p_dev.iter().map(|&v| v as f32).collect::<Vec<_>>(), &stat);
    let mut ua = pack_vec3(u0);
    let mut ub = vec![0.0f32; 3 * n];
    let mut chg = vec![0.0f32; n];
    let mut umax = ua.chunks(3).map(|c| (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() as f64).fold(0.0, f64::max);
    let (mut steps, mut passes) = (0usize, 0usize);
    loop {
        if steps >= cfg.max_steps {
            return Err(Error::InvalidParam(format!("wind (stepped, f32): not converged after max_steps = {}", cfg.max_steps)));
        }
        let p = sub_step_params(cfg, dt_for_pass(cfg, umax));
        for s in 0..cfg.steps_per_pass {
            let reset = s == cfg.steps_per_pass - cfg.hold;
            if s % 2 == 0 { k_momentum_step(&stat, &a, &p, reset, &ua, &mut ub, &mut chg) } else { k_momentum_step(&stat, &a, &p, reset, &ub, &mut ua, &mut chg) }
        }
        steps += cfg.steps_per_pass;
        passes += 1;
        let [res, sp] = k_residual_reduce(&chg, &ua);
        if !(res.is_finite() && sp.is_finite()) {
            return Err(Error::NonFinite("wind (stepped, f32)".into()));
        }
        umax = sp as f64;
        if res as f64 <= cfg.tol {
            let u = ua.chunks(3).map(|c| [c[0] as f64, c[1] as f64, c[2] as f64]).collect();
            return Ok(StepResult { u, steps, passes, residual: res as f64, copy_back_s: 0.0 });
        }
    }
}

// ----------------------------------------------------------------------------------- GPU (feature)

#[cfg(feature = "gpu")]
pub use gpu_impl::solve_gpu;

#[cfg(feature = "gpu")]
mod gpu_impl {
    use super::*;
    use crate::gpu::GpuContext;
    use std::borrow::Cow;

    pub const WGSL_PRESSURE_GRADIENT: &str = r#"
@group(0) @binding(0) var<storage, read> offsets: array<u32>;
@group(0) @binding(1) var<storage, read> adj: array<u32>;
@group(0) @binding(2) var<storage, read> coef3: array<f32>;
@group(0) @binding(3) var<storage, read> pd: array<f32>;
@group(0) @binding(4) var<storage, read> stat: array<f32>;
@group(0) @binding(5) var<storage, read_write> a: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = arrayLength(&offsets) - 1u;
    let i = gid.x;
    if (i >= n) { return; }
    var g = vec3<f32>(0.0, 0.0, 0.0);
    for (var k = offsets[i]; k < offsets[i + 1u]; k = k + 1u) {
        let d = pd[adj[k]] - pd[i];
        g = g + vec3<f32>(coef3[3u * k], coef3[3u * k + 1u], coef3[3u * k + 2u]) * d;
    }
    let inv = -1.0 / max(stat[i * 12u + 5u], 1e-9);
    a[3u * i] = g.x * inv;
    a[3u * i + 1u] = g.y * inv;
    a[3u * i + 2u] = g.z * inv;
}
"#;

    pub const WGSL_MOMENTUM_STEP: &str = r#"
struct P { dt: f32, cq: f32, stab: f32, kbmax: f32, gain: f32, hard: f32, reset: f32, pad: f32 }
@group(0) @binding(0) var<storage, read> stat: array<f32>;
@group(0) @binding(1) var<storage, read> a: array<f32>;
@group(0) @binding(2) var<storage, read> u_in: array<f32>;
@group(0) @binding(3) var<storage, read_write> u_out: array<f32>;
@group(0) @binding(4) var<storage, read_write> chg: array<f32>;
@group(0) @binding(5) var<storage, read> p: P;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = arrayLength(&chg);
    let i = gid.x;
    if (i >= n) { return; }
    let s = i * 12u;
    let up = vec3<f32>(stat[s], stat[s + 1u], stat[s + 2u]);
    let r = stat[s + 3u];
    let f = stat[s + 4u];
    let hat = vec3<f32>(stat[s + 6u], stat[s + 7u], stat[s + 8u]);
    let rise = stat[s + 9u];
    let old = vec3<f32>(u_in[3u * i], u_in[3u * i + 1u], u_in[3u * i + 2u]);
    let acc = vec3<f32>(a[3u * i], a[3u * i + 1u], a[3u * i + 2u]);
    let rho_i = (r + p.cq * length(old)) * p.dt;
    let phi = f * p.dt;
    let us = old + p.dt * acc;
    let kxu = cross(up, us);
    let den = (1.0 + rho_i) * (1.0 + rho_i) + phi * phi;
    var un = (us * (1.0 + rho_i) - kxu * phi) / den;
    if (p.gain > 0.0 && rise > 0.0) {
        let w = dot(un, hat);
        if (w > 0.0) {
            let b = min(p.gain * rise / (rise + length(un) / p.stab + 1e-12), 1.0);
            let kb = min(r * b / max(1.0 - b, 0.02), p.kbmax);
            let damp = kb * p.dt / (1.0 + kb * p.dt);
            un = un - hat * (damp * w);
        }
    }
    let sp = length(un);
    if (sp > p.hard) { un = un * (p.hard / sp); }
    let ch = length(un - old);
    if (p.reset > 0.5) { chg[i] = ch; } else { chg[i] = max(chg[i], ch); }
    u_out[3u * i] = un.x;
    u_out[3u * i + 1u] = un.y;
    u_out[3u * i + 2u] = un.z;
}
"#;

    pub const WGSL_REDUCE_1: &str = r#"
@group(0) @binding(0) var<storage, read> chg: array<f32>;
@group(0) @binding(1) var<storage, read> u: array<f32>;
@group(0) @binding(2) var<storage, read_write> partial: array<f32>;
var<workgroup> sc: array<f32, 64>;
var<workgroup> ss: array<f32, 64>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let n = arrayLength(&chg);
    let i = gid.x;
    var c = 0.0;
    var s = 0.0;
    if (i < n) {
        c = chg[i];
        s = length(vec3<f32>(u[3u * i], u[3u * i + 1u], u[3u * i + 2u]));
    }
    sc[lid.x] = c;
    ss[lid.x] = s;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride >> 1u) {
        if (lid.x < stride) {
            sc[lid.x] = max(sc[lid.x], sc[lid.x + stride]);
            ss[lid.x] = max(ss[lid.x], ss[lid.x + stride]);
        }
        workgroupBarrier();
    }
    if (lid.x == 0u) {
        partial[2u * wid.x] = sc[0];
        partial[2u * wid.x + 1u] = ss[0];
    }
}
"#;

    pub const WGSL_REDUCE_2: &str = r#"
@group(0) @binding(0) var<storage, read> partial: array<f32>;
@group(0) @binding(1) var<storage, read_write> result: array<f32>;
var<workgroup> sc: array<f32, 64>;
var<workgroup> ss: array<f32, 64>;

@compute @workgroup_size(64)
fn main(@builtin(local_invocation_id) lid: vec3<u32>) {
    let nwg = arrayLength(&partial) / 2u;
    var c = 0.0;
    var s = 0.0;
    for (var g = lid.x; g < nwg; g = g + 64u) {
        c = max(c, partial[2u * g]);
        s = max(s, partial[2u * g + 1u]);
    }
    sc[lid.x] = c;
    ss[lid.x] = s;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride >> 1u) {
        if (lid.x < stride) {
            sc[lid.x] = max(sc[lid.x], sc[lid.x + stride]);
            ss[lid.x] = max(ss[lid.x], ss[lid.x + stride]);
        }
        workgroupBarrier();
    }
    if (lid.x == 0u) {
        result[0] = sc[0];
        result[1] = ss[0];
    }
}
"#;

    fn pipeline(ctx: &GpuContext, label: &str, src: &str) -> wgpu::ComputePipeline {
        let module = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(src)),
        });
        ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: Default::default(),
            cache: None,
        })
    }

    fn bind(ctx: &GpuContext, pl: &wgpu::ComputePipeline, bufs: &[&wgpu::Buffer]) -> wgpu::BindGroup {
        let layout = pl.get_bind_group_layout(0);
        let entries: Vec<wgpu::BindGroupEntry> =
            bufs.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect();
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &entries })
    }

    /// Same host loop as `solve_f32_emulated`, on the device. One `queue.submit` per pass, one readback per pass.
    pub fn solve_gpu(ctx: &GpuContext, pm: &PackedMesh, st: &StepStatic, cfg: &StepConfig, u0: &[Vec3]) -> Result<StepResult> {
        validate(cfg)?;
        let n = pm.n_cells;
        let nwg = n.div_ceil(WG);
        let groups = |m: usize| m.div_ceil(WG) as u32;
        let (p_grad, p_mom, p_r1, p_r2) = (
            pipeline(ctx, "pressure_gradient", WGSL_PRESSURE_GRADIENT),
            pipeline(ctx, "momentum_step", WGSL_MOMENTUM_STEP),
            pipeline(ctx, "residual_reduce_1", WGSL_REDUCE_1),
            pipeline(ctx, "residual_reduce_2", WGSL_REDUCE_2),
        );
        let (offsets, adj) = (ctx.upload_u32(&pm.offsets), ctx.upload_u32(&pm.adj));
        let coef3 = ctx.upload_f32(&pack_vec3(&st.grad_coef));
        let pd = ctx.upload_f32(&st.p_dev.iter().map(|&v| v as f32).collect::<Vec<_>>());
        let stat = ctx.upload_f32(&pack_static(st));
        let a = ctx.empty_f32(3 * n);
        let u_a = ctx.upload_f32(&pack_vec3(u0));
        let u_b = ctx.empty_f32(3 * n);
        let chg = ctx.empty_f32(n);
        let partial = ctx.empty_f32(2 * nwg);
        let result = ctx.empty_f32(2);

        // static: pressure gradient, once
        {
            let bg = bind(ctx, &p_grad, &[&offsets, &adj, &coef3, &pd, &stat, &a]);
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&p_grad);
                pass.set_bind_group(0, &bg, &[]);
                pass.dispatch_workgroups(groups(n), 1, 1);
            }
            ctx.queue.submit(Some(enc.finish()));
        }

        let bg_r1 = bind(ctx, &p_r1, &[&chg, &u_a, &partial]);
        let bg_r2 = bind(ctx, &p_r2, &[&partial, &result]);
        let mut umax = u0.iter().map(|v| crate::math::length(*v)).fold(0.0, f64::max);
        let (mut steps, mut passes) = (0usize, 0usize);
        loop {
            if steps >= cfg.max_steps {
                return Err(Error::InvalidParam(format!("wind (stepped, gpu): not converged after max_steps = {}", cfg.max_steps)));
            }
            let dt = dt_for_pass(cfg, umax);
            // one small parameter buffer and bind group per sub-step (reset flag differs, dt differs per pass)
            let mut groups_bg = Vec::with_capacity(cfg.steps_per_pass);
            for s in 0..cfg.steps_per_pass {
                let mut p = sub_step_params(cfg, dt);
                p[6] = if s == cfg.steps_per_pass - cfg.hold { 1.0 } else { 0.0 };
                let pb = ctx.upload_f32(&p);
                let (src, dst) = if s % 2 == 0 { (&u_a, &u_b) } else { (&u_b, &u_a) };
                groups_bg.push(bind(ctx, &p_mom, &[&stat, &a, src, dst, &chg, &pb]));
            }
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&p_mom);
                for bg in &groups_bg {
                    pass.set_bind_group(0, bg, &[]);
                    pass.dispatch_workgroups(groups(n), 1, 1);
                }
            }
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&p_r1);
                pass.set_bind_group(0, &bg_r1, &[]);
                pass.dispatch_workgroups(groups(n), 1, 1);
                pass.set_pipeline(&p_r2);
                pass.set_bind_group(0, &bg_r2, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            ctx.queue.submit(Some(enc.finish()));
            steps += cfg.steps_per_pass;
            passes += 1;
            let r = ctx.download_f32(&result, 2)?;
            if !(r[0].is_finite() && r[1].is_finite()) {
                return Err(Error::NonFinite("wind (stepped, gpu)".into()));
            }
            umax = r[1] as f64;
            if r[0] as f64 <= cfg.tol {
                let t_copy = std::time::Instant::now();
                let u = ctx.download_f32(&u_a, 3 * n)?;
                let copy_back_s = t_copy.elapsed().as_secs_f64();
                let u = u.chunks(3).map(|c| [c[0] as f64, c[1] as f64, c[2] as f64]).collect();
                return Ok(StepResult { u, steps, passes, residual: r[0] as f64, copy_back_s });
            }
        }
    }
}
