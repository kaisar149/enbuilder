//! Feature `gpu`: wgpu compute foundation (device, buffers, CSR upload, Laplacian kernel).
//! Written against wgpu 23 (see Cargo.toml). Not hardware-verified in the build sandbox (see ARCHITECTURE.md).
use crate::backend::{to_f32, to_f64, ComputeBackend, PackedMesh};
use crate::domain::Domain;
use crate::error::{Error, Result};
use std::borrow::Cow;

pub const WORKGROUP_SIZE: u32 = 64;

/// One thread per cell: out_i = sum_k coef_k (x[adj_k] - x[i]). n = arrayLength(offsets) - 1.
pub const LAPLACIAN_WGSL: &str = r#"
@group(0) @binding(0) var<storage, read> offsets: array<u32>;
@group(0) @binding(1) var<storage, read> adj: array<u32>;
@group(0) @binding(2) var<storage, read> coef: array<f32>;
@group(0) @binding(3) var<storage, read> x: array<f32>;
@group(0) @binding(4) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = arrayLength(&offsets) - 1u;
    let i = gid.x;
    if (i >= n) { return; }
    var acc: f32 = 0.0;
    let xi = x[i];
    for (var k = offsets[i]; k < offsets[i + 1u]; k = k + 1u) {
        acc = acc + coef[k] * (x[adj[k]] - xi);
    }
    out[i] = acc;
}
"#;

pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
}

impl GpuContext {
    /// High-performance adapter first, then the fallback (software) adapter; `Error::NoGpu` if neither exists.
    pub fn new() -> Result<GpuContext> {
        let instance = wgpu::Instance::default();
        let mut adapter = None;
        for fallback in [false, true] {
            adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: fallback,
                compatible_surface: None,
            }));
            if adapter.is_some() { break; }
        }
        let adapter = adapter.ok_or_else(|| Error::NoGpu("no wgpu adapter found".into()))?;
        let adapter_info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .map_err(|e| Error::NoGpu(format!("device request failed: {e}")))?;
        Ok(GpuContext { device, queue, adapter_info })
    }

    pub fn upload_u32(&self, data: &[u32]) -> wgpu::Buffer { self.upload_bytes(&bytes_u32(data)) }
    pub fn upload_f32(&self, data: &[f32]) -> wgpu::Buffer { self.upload_bytes(&bytes_f32(data)) }

    fn upload_bytes(&self, bytes: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        // storage buffers must not be empty
        let padded: Vec<u8> = if bytes.is_empty() { vec![0; 4] } else { bytes.to_vec() };
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &padded,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        })
    }

    pub fn empty_f32(&self, n: usize) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (4 * n.max(1)) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Copies `n` f32 values back through a staging buffer.
    pub fn download_f32(&self, buf: &wgpu::Buffer, n: usize) -> Result<Vec<f32>> {
        let size = (4 * n.max(1)) as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_buffer_to_buffer(buf, 0, &staging, 0, size);
        self.queue.submit(Some(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|_| Error::NoGpu("readback channel closed".into()))?
            .map_err(|e| Error::NoGpu(format!("buffer map failed: {e:?}")))?;
        let data = slice.get_mapped_range();
        let out: Vec<f32> = data.chunks_exact(4).take(n).map(|c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
        drop(data);
        staging.unmap();
        Ok(out)
    }

    /// Upload then download (conversion round-trip test): f64 -> f32 -> GPU -> f32 -> f64.
    pub fn round_trip(&self, x: &[f64]) -> Result<Vec<f64>> {
        let buf = self.upload_f32(&to_f32(x)?);
        Ok(to_f64(&self.download_f32(&buf, x.len())?))
    }
}

fn bytes_u32(d: &[u32]) -> Vec<u8> { d.iter().flat_map(|v| v.to_ne_bytes()).collect() }
fn bytes_f32(d: &[f32]) -> Vec<u8> { d.iter().flat_map(|v| v.to_ne_bytes()).collect() }

/// The mesh on the GPU plus the compiled Laplacian pipeline.
pub struct GpuLaplacian {
    n: usize,
    pipeline: wgpu::ComputePipeline,
    offsets: wgpu::Buffer,
    adj: wgpu::Buffer,
    coef: wgpu::Buffer,
}

impl GpuLaplacian {
    pub fn new(ctx: &GpuContext, pm: &PackedMesh) -> GpuLaplacian {
        let module = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("laplacian"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(LAPLACIAN_WGSL)),
        });
        let pipeline = ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("laplacian"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: Default::default(),
            cache: None,
        });
        GpuLaplacian {
            n: pm.n_cells,
            pipeline,
            offsets: ctx.upload_u32(&pm.offsets),
            adj: ctx.upload_u32(&pm.adj),
            coef: ctx.upload_f32(&pm.coef),
        }
    }

    pub fn run(&self, ctx: &GpuContext, x: &[f32]) -> Result<Vec<f32>> {
        if x.len() != self.n {
            return Err(Error::InvalidParam(format!("field has {} values, mesh has {} cells", x.len(), self.n)));
        }
        let xb = ctx.upload_f32(x);
        let ob = ctx.empty_f32(self.n);
        let layout = self.pipeline.get_bind_group_layout(0);
        let bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.offsets.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.adj.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.coef.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: xb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: ob.as_entire_binding() },
            ],
        });
        let mut enc = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups((self.n as u32).div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        ctx.queue.submit(Some(enc.finish()));
        ctx.download_f32(&ob, self.n)
    }
}

/// `ComputeBackend` on the GPU (f32). Re-uploads the mesh per call; cache a `GpuLaplacian` for hot loops.
pub struct Gpu { pub ctx: GpuContext }

impl Gpu {
    pub fn new() -> Result<Gpu> { Ok(Gpu { ctx: GpuContext::new()? }) }
}

impl ComputeBackend for Gpu {
    fn name(&self) -> String { format!("gpu: {} ({:?})", self.ctx.adapter_info.name, self.ctx.adapter_info.backend) }
    fn precision(&self) -> &'static str { "f32" }
    fn laplacian(&self, domain: &Domain, x: &[f64]) -> Result<Vec<f64>> {
        let k = GpuLaplacian::new(&self.ctx, &PackedMesh::from_domain(domain));
        Ok(to_f64(&k.run(&self.ctx, &to_f32(x)?)?))
    }
}
