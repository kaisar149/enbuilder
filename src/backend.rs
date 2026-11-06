//! Part 3c-2: compute backends (CPU f64 reference, optional GPU f32), f32 packing of the mesh.
//!
//! Precision decision (recorded in ARCHITECTURE.md): the crate stays f64 on the CPU; the GPU path is f32.
//! Conservation-critical transport (`moisture_transport`) stays on the CPU in f64. GPU/CPU parity tests use a
//! relative tolerance of 1e-4 (measured against the largest magnitude of the field, see `max_rel_error`),
//! not the 1e-8 used by the f64 conservation tests.
//!
//! `PackedMesh` is exactly what is uploaded to the GPU (CSR offsets, neighbours, coef = edge_len/edge_dist
//! as f32). `laplacian_f32_emulated` runs the same arithmetic as the WGSL kernel on the CPU in f32, so the
//! packing and the algorithm are testable without a GPU.
use crate::domain::Domain;
use crate::error::{Error, Result};

/// Weighted graph Laplacian `out_i = sum_k w_ik (x_k - x_i)`, `w = edge_len / edge_dist`
/// (flux coefficient of the mesh; conservative: sum_i area-free sum of out_i is exactly 0 in f64).
pub trait ComputeBackend: Send + Sync {
    fn name(&self) -> String;
    /// Precision of the arithmetic ("f64" or "f32").
    fn precision(&self) -> &'static str;
    fn laplacian(&self, domain: &Domain, x: &[f64]) -> Result<Vec<f64>>;
}

/// f64 reference backend (the default).
#[derive(Debug, Clone, Copy, Default)]
pub struct Cpu;

impl ComputeBackend for Cpu {
    fn name(&self) -> String { "cpu".into() }
    fn precision(&self) -> &'static str { "f64" }
    fn laplacian(&self, domain: &Domain, x: &[f64]) -> Result<Vec<f64>> { laplacian_f64(domain, x) }
}

/// Runtime selection. `Gpu` needs the `gpu` feature and an adapter, otherwise `Error::NoGpu`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendKind { #[default] Cpu, Gpu }

/// Creates the backend handle modules will hold (default `Cpu`).
pub fn make_backend(kind: BackendKind) -> Result<Box<dyn ComputeBackend>> {
    match kind {
        BackendKind::Cpu => Ok(Box::new(Cpu)),
        #[cfg(feature = "gpu")]
        BackendKind::Gpu => Ok(Box::new(crate::gpu::Gpu::new()?)),
        #[cfg(not(feature = "gpu"))]
        BackendKind::Gpu => Err(Error::NoGpu("built without the `gpu` cargo feature".into())),
    }
}

fn check_len(domain: &Domain, x: &[f64]) -> Result<()> {
    if x.len() != domain.n_cells() {
        return Err(Error::InvalidParam(format!("field has {} values, mesh has {} cells", x.len(), domain.n_cells())));
    }
    Ok(())
}

/// f64 reference Laplacian.
pub fn laplacian_f64(domain: &Domain, x: &[f64]) -> Result<Vec<f64>> {
    check_len(domain, x)?;
    let m = &domain.mesh;
    Ok((0..m.n_cells)
        .map(|i| {
            let (a, b) = (m.adj_offsets[i] as usize, m.adj_offsets[i + 1] as usize);
            (a..b).map(|k| (m.edge_len[k] / m.edge_dist[k]) * (x[m.adj[k] as usize] - x[i])).sum()
        })
        .collect())
}

/// Mesh data as uploaded to the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct PackedMesh {
    pub n_cells: usize,
    pub offsets: Vec<u32>,
    pub adj: Vec<u32>,
    /// `edge_len / edge_dist`, rounded to f32, parallel to `adj`.
    pub coef: Vec<f32>,
}

impl PackedMesh {
    pub fn from_domain(domain: &Domain) -> PackedMesh {
        let m = &domain.mesh;
        PackedMesh {
            n_cells: m.n_cells,
            offsets: m.adj_offsets.clone(),
            adj: m.adj.clone(),
            coef: (0..m.adj.len()).map(|k| (m.edge_len[k] / m.edge_dist[k]) as f32).collect(),
        }
    }
}

/// `&[f64]` -> f32 (rounded to nearest). Errors on values outside the f32 range.
pub fn to_f32(x: &[f64]) -> Result<Vec<f32>> {
    x.iter()
        .map(|&v| {
            let s = v as f32;
            if v.is_finite() && !s.is_finite() { Err(Error::NonFinite("f32 conversion overflow".into())) } else { Ok(s) }
        })
        .collect()
}

pub fn to_f64(x: &[f32]) -> Vec<f64> { x.iter().map(|&v| v as f64).collect() }

/// Max |a - b| divided by max(max |b|, tiny): the metric used by all parity tests.
pub fn max_rel_error(a: &[f64], b: &[f64]) -> f64 {
    let scale = b.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-30);
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max) / scale
}

/// The WGSL kernel's arithmetic (f32, same loop order) run on the CPU.
pub fn laplacian_f32_emulated(pm: &PackedMesh, x: &[f32]) -> Vec<f32> {
    (0..pm.n_cells)
        .map(|i| {
            let mut acc = 0.0f32;
            for k in pm.offsets[i] as usize..pm.offsets[i + 1] as usize {
                acc += pm.coef[k] * (x[pm.adj[k] as usize] - x[i]);
            }
            acc
        })
        .collect()
}

/// f32 emulation of the GPU kernel as a backend (always available; lets modules and tests exercise the
/// f32 precision path without hardware).
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuF32;

impl ComputeBackend for CpuF32 {
    fn name(&self) -> String { "cpu-f32-emulation".into() }
    fn precision(&self) -> &'static str { "f32" }
    fn laplacian(&self, domain: &Domain, x: &[f64]) -> Result<Vec<f64>> {
        check_len(domain, x)?;
        Ok(to_f64(&laplacian_f32_emulated(&PackedMesh::from_domain(domain), &to_f32(x)?)))
    }
}
