//! Part 3c-4: the mass-conserving projection (Jacobi-preconditioned CG on the graph Laplacian) as f32 kernels.
//!
//! Same maths as `transport::project_flux`: div = sum of outgoing edge flux, solve `A phi = div - mean(div)` with
//! `A x_i = sum_k w_ik (x_i - x_k)`, `w = edge_len/edge_dist`, then `F'_k = F_k - w_k (phi_i - phi_j)`.
//!
//! Kernels (WGSL in `gpu_impl`, CPU twins `k_*` below; the CPU twins run the same f32 arithmetic in the same
//! reduction order, so the algorithm, buffer layout and host loop are testable without a GPU):
//!  `flux_div`, `cg_init`, `cg_matvec`, `dot1`/`dot2` (two-pass workgroup reduction), `cg_alpha`, `cg_update_xr`,
//!  `cg_beta`, `cg_update_p`, `flux_correct`.
//! All CG scalars (rz, pAp, alpha, beta, rr) live in a device buffer `sc`; the iteration loop has no readback.
//! The host reads `rr` once per `interval` (16) iterations. The constant mode is removed from the right-hand side
//! once, at the start, as on the CPU (the Laplacian maps to mean-free vectors, so r stays mean-free up to f32
//! rounding).
//!
//! The CPU f64 path (`transport::project_flux`) stays the reference for conservation. `divergence_residual_f64`
//! re-checks any corrected flux in f64 with the module's definition of `divergence_residual`.
use crate::backend::PackedMesh;
use crate::domain::Domain;
use crate::error::{Error, Result};
use crate::transport::flux_divergence;

pub const WG: usize = 64;
// scalar slots in `sc`
pub const SUM: usize = 0;
pub const PAP: usize = 1;
pub const RZ: usize = 2;
pub const RZNEW: usize = 3;
pub const RR: usize = 4;
pub const ALPHA: usize = 5;
pub const BETA: usize = 6;
pub const BAD: usize = 7;

#[derive(Debug, Clone)]
pub struct ProjConfig {
    /// Relative residual tolerance on the device (1e-6 default: the f32 floor).
    pub tol: f64,
    pub max_iters: usize,
    /// Iterations between residual readbacks.
    pub interval: usize,
}
impl Default for ProjConfig {
    fn default() -> Self { ProjConfig { tol: 1e-6, max_iters: 20_000, interval: 16 } }
}

#[derive(Debug, Clone)]
pub struct ProjResult {
    pub phi: Vec<f64>,
    /// Corrected flux from the `flux_correct` kernel (f32 converted to f64), parallel to `mesh.adj`.
    pub fc: Vec<f64>,
    pub iterations: usize,
    /// sqrt(rr)/sqrt(rr0) as seen by the device at the last check.
    pub rel_residual: f64,
}

fn validate(cfg: &ProjConfig) -> Result<()> {
    if cfg.interval == 0 || cfg.max_iters == 0 || !(cfg.tol > 0.0 && cfg.tol < 1.0) {
        return Err(Error::InvalidParam("projection: tol in (0,1), max_iters >= 1, interval >= 1".into()));
    }
    Ok(())
}

/// Jacobi diagonal in f32 (1 where a cell has no edges).
pub fn diag_f32(pm: &PackedMesh) -> Vec<f32> {
    (0..pm.n_cells)
        .map(|i| {
            let d: f32 = (pm.offsets[i] as usize..pm.offsets[i + 1] as usize).map(|k| pm.coef[k]).sum();
            if d > 0.0 { d } else { 1.0 }
        })
        .collect()
}

// ----------------------------------------------------------------------------------- emulated kernels

pub fn k_flux_div(pm: &PackedMesh, f: &[f32]) -> Vec<f32> {
    (0..pm.n_cells).map(|i| (pm.offsets[i] as usize..pm.offsets[i + 1] as usize).map(|k| f[k]).fold(0.0f32, |a, v| a + v)).collect()
}

fn tree(v: &mut [f32; WG]) -> f32 {
    let mut stride = WG / 2;
    while stride > 0 {
        for l in 0..stride {
            v[l] += v[l + stride];
        }
        stride /= 2;
    }
    v[0]
}

/// Two-pass dot product, same summation order as `dot1` + `dot2`.
pub fn k_dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len();
    let nwg = n.div_ceil(WG);
    let mut partial = vec![0.0f32; nwg];
    for g in 0..nwg {
        let mut v = [0.0f32; WG];
        for l in 0..WG {
            let i = g * WG + l;
            if i < n { v[l] = a[i] * b[i]; }
        }
        partial[g] = tree(&mut v);
    }
    let mut v = [0.0f32; WG];
    for l in 0..WG {
        let mut g = l;
        let mut acc = 0.0f32;
        while g < nwg {
            acc += partial[g];
            g += WG;
        }
        v[l] = acc;
    }
    tree(&mut v)
}

/// A x with A x_i = sum_k w_ik (x_i - x_k).
pub fn k_matvec(pm: &PackedMesh, x: &[f32]) -> Vec<f32> {
    (0..pm.n_cells)
        .map(|i| {
            let mut s = 0.0f32;
            for k in pm.offsets[i] as usize..pm.offsets[i + 1] as usize {
                s += pm.coef[k] * (x[i] - x[pm.adj[k] as usize]);
            }
            s
        })
        .collect()
}

pub fn k_flux_correct(pm: &PackedMesh, f: &[f32], phi: &[f32]) -> Vec<f32> {
    let mut fc = f.to_vec();
    for i in 0..pm.n_cells {
        for k in pm.offsets[i] as usize..pm.offsets[i + 1] as usize {
            fc[k] = f[k] - pm.coef[k] * (phi[i] - phi[pm.adj[k] as usize]);
        }
    }
    fc
}

/// Whole solve on the emulated kernels (same host loop as the GPU path).
pub fn project_f32_emulated(pm: &PackedMesh, f: &[f64], cfg: &ProjConfig) -> Result<ProjResult> {
    validate(cfg)?;
    let n = pm.n_cells;
    let f32f: Vec<f32> = crate::backend::to_f32(f)?;
    let diag = diag_f32(pm);
    let div = k_flux_div(pm, &f32f);
    let ones = vec![1.0f32; n];
    let mean = k_dot(&div, &ones) / n as f32;
    let mut x = vec![0.0f32; n];
    let mut r: Vec<f32> = div.iter().map(|v| v - mean).collect();
    let mut z: Vec<f32> = (0..n).map(|i| r[i] / diag[i]).collect();
    let mut p = z.clone();
    let mut sc = [0.0f32; 8];
    sc[RZ] = k_dot(&r, &z);
    sc[RR] = k_dot(&r, &r);
    let bn = (sc[RR] as f64).sqrt();
    let mut iterations = 0usize;
    let mut rel = 0.0f64;
    if bn > 0.0 {
        loop {
            for _ in 0..cfg.interval {
                let ap = k_matvec(pm, &p);
                sc[PAP] = k_dot(&p, &ap);
                if sc[PAP] > 0.0 { sc[ALPHA] = sc[RZ] / sc[PAP]; } else { sc[ALPHA] = 0.0; sc[BAD] = 1.0; }
                for i in 0..n {
                    x[i] += sc[ALPHA] * p[i];
                    r[i] -= sc[ALPHA] * ap[i];
                    z[i] = r[i] / diag[i];
                }
                sc[RZNEW] = k_dot(&r, &z);
                sc[RR] = k_dot(&r, &r);
                sc[BETA] = if sc[RZ] != 0.0 { sc[RZNEW] / sc[RZ] } else { 0.0 };
                sc[RZ] = sc[RZNEW];
                for i in 0..n {
                    p[i] = z[i] + sc[BETA] * p[i];
                }
            }
            iterations += cfg.interval;
            rel = (sc[RR].max(0.0) as f64).sqrt() / bn;
            if !rel.is_finite() { return Err(Error::NonFinite("projection (f32)".into())); }
            if rel <= cfg.tol || sc[BAD] != 0.0 { break; }
            if iterations >= cfg.max_iters {
                return Err(Error::InvalidParam(format!("projection (f32): not converged after {iterations} iterations (rel residual {rel:.2e})")));
            }
        }
    }
    let fc = k_flux_correct(pm, &f32f, &x);
    Ok(ProjResult { phi: crate::backend::to_f64(&x), fc: crate::backend::to_f64(&fc), iterations, rel_residual: rel })
}

// ----------------------------------------------------------------------------------- f64 checks

/// The module's `divergence_residual` definition: max_i |div F'_i| / max_i sum_k |F_ik| (f64, on the CPU).
pub fn divergence_residual_f64(domain: &Domain, f: &[f64], fc: &[f64]) -> f64 {
    let m = &domain.mesh;
    let d = flux_divergence(m, fc);
    let fscale = (0..m.n_cells).map(|i| f[m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize].iter().map(|v| v.abs()).sum::<f64>()).fold(0.0, f64::max);
    if fscale > 0.0 { d.iter().fold(0.0f64, |a, v| a.max(v.abs())) / fscale } else { 0.0 }
}

/// F' = F - w (phi_i - phi_j) in f64 from a (GPU) potential: removes the f32 rounding of the correction itself.
pub fn correct_f64(domain: &Domain, f: &[f64], phi: &[f64]) -> Vec<f64> {
    let m = &domain.mesh;
    let mut fc = f.to_vec();
    for i in 0..m.n_cells {
        for k in m.adj_offsets[i] as usize..m.adj_offsets[i + 1] as usize {
            fc[k] -= m.edge_len[k] / m.edge_dist[k] * (phi[i] - phi[m.adj[k] as usize]);
        }
    }
    fc
}

// ----------------------------------------------------------------------------------- GPU (feature)

#[cfg(feature = "gpu")]
pub use gpu_impl::project_gpu;

#[cfg(feature = "gpu")]
mod gpu_impl {
    use super::*;
    use crate::gpu::GpuContext;
    use std::borrow::Cow;

    pub const WGSL_FLUX_DIV: &str = r#"
@group(0) @binding(0) var<storage, read> offsets: array<u32>;
@group(0) @binding(1) var<storage, read> f: array<f32>;
@group(0) @binding(2) var<storage, read_write> div: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = arrayLength(&offsets) - 1u;
    let i = gid.x;
    if (i >= n) { return; }
    var s: f32 = 0.0;
    for (var k = offsets[i]; k < offsets[i + 1u]; k = k + 1u) { s = s + f[k]; }
    div[i] = s;
}
"#;

    pub const WGSL_CG_INIT: &str = r#"
@group(0) @binding(0) var<storage, read> div: array<f32>;
@group(0) @binding(1) var<storage, read> diag: array<f32>;
@group(0) @binding(2) var<storage, read> mean: array<f32>;
@group(0) @binding(3) var<storage, read_write> x: array<f32>;
@group(0) @binding(4) var<storage, read_write> r: array<f32>;
@group(0) @binding(5) var<storage, read_write> z: array<f32>;
@group(0) @binding(6) var<storage, read_write> p: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&div)) { return; }
    x[i] = 0.0;
    let ri = div[i] - mean[0];
    r[i] = ri;
    z[i] = ri / diag[i];
    p[i] = ri / diag[i];
}
"#;

    pub const WGSL_MATVEC: &str = r#"
@group(0) @binding(0) var<storage, read> offsets: array<u32>;
@group(0) @binding(1) var<storage, read> adj: array<u32>;
@group(0) @binding(2) var<storage, read> coef: array<f32>;
@group(0) @binding(3) var<storage, read> x: array<f32>;
@group(0) @binding(4) var<storage, read_write> y: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = arrayLength(&offsets) - 1u;
    let i = gid.x;
    if (i >= n) { return; }
    var s: f32 = 0.0;
    for (var k = offsets[i]; k < offsets[i + 1u]; k = k + 1u) { s = s + coef[k] * (x[i] - x[adj[k]]); }
    y[i] = s;
}
"#;

    pub const WGSL_DOT1: &str = r#"
@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> partial: array<f32>;
var<workgroup> sh: array<f32, 64>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let i = gid.x;
    var v: f32 = 0.0;
    if (i < arrayLength(&a)) { v = a[i] * b[i]; }
    sh[lid.x] = v;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride >> 1u) {
        if (lid.x < stride) { sh[lid.x] = sh[lid.x] + sh[lid.x + stride]; }
        workgroupBarrier();
    }
    if (lid.x == 0u) { partial[wid.x] = sh[0]; }
}
"#;

    /// Writes the total into sc[slot[0]].
    pub const WGSL_DOT2: &str = r#"
@group(0) @binding(0) var<storage, read> partial: array<f32>;
@group(0) @binding(1) var<storage, read_write> sc: array<f32>;
@group(0) @binding(2) var<storage, read> slot: array<u32>;
var<workgroup> sh: array<f32, 64>;
@compute @workgroup_size(64)
fn main(@builtin(local_invocation_id) lid: vec3<u32>) {
    let nwg = arrayLength(&partial);
    var acc: f32 = 0.0;
    for (var g = lid.x; g < nwg; g = g + 64u) { acc = acc + partial[g]; }
    sh[lid.x] = acc;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride >> 1u) {
        if (lid.x < stride) { sh[lid.x] = sh[lid.x] + sh[lid.x + stride]; }
        workgroupBarrier();
    }
    if (lid.x == 0u) { sc[slot[0]] = sh[0]; }
}
"#;

    pub const WGSL_ALPHA: &str = r#"
@group(0) @binding(0) var<storage, read_write> sc: array<f32>;
@compute @workgroup_size(1)
fn main() {
    let pap = sc[1];
    if (pap > 0.0) { sc[5] = sc[2] / pap; } else { sc[5] = 0.0; sc[7] = 1.0; }
}
"#;

    pub const WGSL_UPDATE_XR: &str = r#"
@group(0) @binding(0) var<storage, read> diag: array<f32>;
@group(0) @binding(1) var<storage, read> p: array<f32>;
@group(0) @binding(2) var<storage, read> ap: array<f32>;
@group(0) @binding(3) var<storage, read> sc: array<f32>;
@group(0) @binding(4) var<storage, read_write> x: array<f32>;
@group(0) @binding(5) var<storage, read_write> r: array<f32>;
@group(0) @binding(6) var<storage, read_write> z: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&x)) { return; }
    let alpha = sc[5];
    x[i] = x[i] + alpha * p[i];
    let ri = r[i] - alpha * ap[i];
    r[i] = ri;
    z[i] = ri / diag[i];
}
"#;

    pub const WGSL_BETA: &str = r#"
@group(0) @binding(0) var<storage, read_write> sc: array<f32>;
@compute @workgroup_size(1)
fn main() {
    let rz = sc[2];
    let rzn = sc[3];
    if (rz != 0.0) { sc[6] = rzn / rz; } else { sc[6] = 0.0; }
    sc[2] = rzn;
}
"#;

    pub const WGSL_UPDATE_P: &str = r#"
@group(0) @binding(0) var<storage, read> z: array<f32>;
@group(0) @binding(1) var<storage, read> sc: array<f32>;
@group(0) @binding(2) var<storage, read_write> p: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&p)) { return; }
    p[i] = z[i] + sc[6] * p[i];
}
"#;

    pub const WGSL_FLUX_CORRECT: &str = r#"
@group(0) @binding(0) var<storage, read> offsets: array<u32>;
@group(0) @binding(1) var<storage, read> adj: array<u32>;
@group(0) @binding(2) var<storage, read> coef: array<f32>;
@group(0) @binding(3) var<storage, read> f: array<f32>;
@group(0) @binding(4) var<storage, read> phi: array<f32>;
@group(0) @binding(5) var<storage, read_write> fc: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = arrayLength(&offsets) - 1u;
    let i = gid.x;
    if (i >= n) { return; }
    for (var k = offsets[i]; k < offsets[i + 1u]; k = k + 1u) { fc[k] = f[k] - coef[k] * (phi[i] - phi[adj[k]]); }
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

    /// Same host loop as `project_f32_emulated`, on the device. Readbacks: 2 at start, 1 per `interval`, 2 at the end.
    pub fn project_gpu(ctx: &GpuContext, pm: &PackedMesh, f: &[f64], cfg: &ProjConfig) -> Result<ProjResult> {
        validate(cfg)?;
        let n = pm.n_cells;
        let ne = pm.adj.len();
        let nwg = n.div_ceil(WG);
        let g = |m: usize| m.div_ceil(WG) as u32;
        let f32f = crate::backend::to_f32(f)?;

        let (p_div, p_init, p_mv, p_d1, p_d2, p_alpha, p_uxr, p_beta, p_up, p_fc) = (
            pipeline(ctx, "flux_div", WGSL_FLUX_DIV), pipeline(ctx, "cg_init", WGSL_CG_INIT), pipeline(ctx, "cg_matvec", WGSL_MATVEC),
            pipeline(ctx, "dot1", WGSL_DOT1), pipeline(ctx, "dot2", WGSL_DOT2), pipeline(ctx, "cg_alpha", WGSL_ALPHA),
            pipeline(ctx, "cg_update_xr", WGSL_UPDATE_XR), pipeline(ctx, "cg_beta", WGSL_BETA), pipeline(ctx, "cg_update_p", WGSL_UPDATE_P),
            pipeline(ctx, "flux_correct", WGSL_FLUX_CORRECT),
        );
        let offsets = ctx.upload_u32(&pm.offsets);
        let adj = ctx.upload_u32(&pm.adj);
        let coef = ctx.upload_f32(&pm.coef);
        let diag = ctx.upload_f32(&diag_f32(pm));
        let fb = ctx.upload_f32(&f32f);
        let div = ctx.empty_f32(n);
        let (x, r, z, p, ap) = (ctx.empty_f32(n), ctx.empty_f32(n), ctx.empty_f32(n), ctx.empty_f32(n), ctx.empty_f32(n));
        let ones = ctx.upload_f32(&vec![1.0f32; n]);
        let partial = ctx.empty_f32(nwg);
        let sc = ctx.empty_f32(8);
        let fcb = ctx.empty_f32(ne);
        let slot = |s: usize| ctx.upload_u32(&[s as u32]);
        let (s_sum, s_pap, s_rz, s_rznew, s_rr) = (slot(SUM), slot(PAP), slot(RZ), slot(RZNEW), slot(RR));

        let bg_div = bind(ctx, &p_div, &[&offsets, &fb, &div]);
        let bg_mv = bind(ctx, &p_mv, &[&offsets, &adj, &coef, &p, &ap]);
        let bg_d1 = |a: &wgpu::Buffer, b: &wgpu::Buffer| bind(ctx, &p_d1, &[a, b, &partial]);
        let bg_d2 = |s: &wgpu::Buffer| bind(ctx, &p_d2, &[&partial, &sc, s]);
        let (d1_sum, d1_pap, d1_rz, d1_rr) = (bg_d1(&div, &ones), bg_d1(&p, &ap), bg_d1(&r, &z), bg_d1(&r, &r));
        let (d2_sum, d2_pap, d2_rz, d2_rznew, d2_rr) = (bg_d2(&s_sum), bg_d2(&s_pap), bg_d2(&s_rz), bg_d2(&s_rznew), bg_d2(&s_rr));
        let bg_alpha = bind(ctx, &p_alpha, &[&sc]);
        let bg_uxr = bind(ctx, &p_uxr, &[&diag, &p, &ap, &sc, &x, &r, &z]);
        let bg_beta = bind(ctx, &p_beta, &[&sc]);
        let bg_up = bind(ctx, &p_up, &[&z, &sc, &p]);
        let bg_fc = bind(ctx, &p_fc, &[&offsets, &adj, &coef, &fb, &x, &fcb]);

        let submit = |build: &dyn Fn(&mut wgpu::ComputePass)| {
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                build(&mut pass);
            }
            ctx.queue.submit(Some(enc.finish()));
        };
        let dispatch = |pass: &mut wgpu::ComputePass, pl: &wgpu::ComputePipeline, bg: &wgpu::BindGroup, groups: u32| {
            pass.set_pipeline(pl);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
        };

        // start: div and its sum -> mean
        submit(&|ps| {
            dispatch(ps, &p_div, &bg_div, g(n));
            dispatch(ps, &p_d1, &d1_sum, g(n));
            dispatch(ps, &p_d2, &d2_sum, 1);
        });
        let mean = ctx.download_f32(&sc, 8)?[SUM] / n as f32;
        let mean_buf = ctx.upload_f32(&[mean]);
        let bg_init = bind(ctx, &p_init, &[&div, &diag, &mean_buf, &x, &r, &z, &p]);
        submit(&|ps| {
            dispatch(ps, &p_init, &bg_init, g(n));
            dispatch(ps, &p_d1, &d1_rz, g(n));
            dispatch(ps, &p_d2, &d2_rz, 1);
            dispatch(ps, &p_d1, &d1_rr, g(n));
            dispatch(ps, &p_d2, &d2_rr, 1);
        });
        let s0 = ctx.download_f32(&sc, 8)?;
        let bn = (s0[RR] as f64).sqrt();
        let (mut iterations, mut rel) = (0usize, 0.0f64);
        if bn > 0.0 {
            loop {
                submit(&|ps| {
                    for _ in 0..cfg.interval {
                        dispatch(ps, &p_mv, &bg_mv, g(n));
                        dispatch(ps, &p_d1, &d1_pap, g(n));
                        dispatch(ps, &p_d2, &d2_pap, 1);
                        dispatch(ps, &p_alpha, &bg_alpha, 1);
                        dispatch(ps, &p_uxr, &bg_uxr, g(n));
                        dispatch(ps, &p_d1, &d1_rz, g(n)); // r.z
                        dispatch(ps, &p_d2, &d2_rznew, 1);
                        dispatch(ps, &p_d1, &d1_rr, g(n));
                        dispatch(ps, &p_d2, &d2_rr, 1);
                        dispatch(ps, &p_beta, &bg_beta, 1);
                        dispatch(ps, &p_up, &bg_up, g(n));
                    }
                });
                iterations += cfg.interval;
                let s = ctx.download_f32(&sc, 8)?;
                rel = (s[RR].max(0.0) as f64).sqrt() / bn;
                if !rel.is_finite() { return Err(Error::NonFinite("projection (gpu)".into())); }
                if rel <= cfg.tol || s[BAD] != 0.0 { break; }
                if iterations >= cfg.max_iters {
                    return Err(Error::InvalidParam(format!("projection (gpu): not converged after {iterations} iterations (rel residual {rel:.2e})")));
                }
            }
        }
        submit(&|ps| dispatch(ps, &p_fc, &bg_fc, g(n)));
        let phi = ctx.download_f32(&x, n)?;
        let fc = ctx.download_f32(&fcb, ne)?;
        Ok(ProjResult { phi: crate::backend::to_f64(&phi), fc: crate::backend::to_f64(&fc), iterations, rel_residual: rel })
    }
}
