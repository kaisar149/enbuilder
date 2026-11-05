//! Flat plane (rectangle or disc) on a square grid. Constant downward gravity.
//! Boundary: Wrap (torus; rectangle only), Wall, or Void. Edges that face
//! outside are recorded in `mesh.boundary_len`; Part 1 diffusion treats Wall
//! and Void identically (zero flux). The disc area is a staircase
//! approximation (~0.6% off at 20 cells across, <0.1% at 100).
use crate::domain::{BoundaryKind, Domain, DomainKind};
use crate::error::{Error, Result};
use crate::mesh::MeshBuilder;

#[derive(Debug, Clone)]
pub enum PlaneShape { Rect { nx: usize, ny: usize }, Disc { diameter_cells: usize } }

#[derive(Debug, Clone)]
pub struct PlaneParams {
    pub shape: PlaneShape,
    pub cell_size: f64,
    pub boundary: BoundaryKind,
    pub surface_gravity: f64,
    pub seed: u64,
}

impl Default for PlaneParams {
    fn default() -> Self {
        PlaneParams {
            shape: PlaneShape::Rect { nx: 128, ny: 64 },
            cell_size: 10_000.0,
            boundary: BoundaryKind::Wall,
            surface_gravity: 9.81,
            seed: 0,
        }
    }
}

pub fn generate_plane(p: &PlaneParams) -> Result<Domain> {
    if !(p.cell_size > 0.0 && p.cell_size.is_finite()) {
        return Err(Error::InvalidParam("cell_size must be positive".into()));
    }
    match p.boundary {
        BoundaryKind::Wrap | BoundaryKind::Wall | BoundaryKind::Void => {}
        _ => return Err(Error::InvalidParam("plane boundary must be Wrap, Wall or Void".into())),
    }
    let (gx, gy, kind) = match p.shape {
        PlaneShape::Rect { nx, ny } => (nx, ny, DomainKind::FlatRect),
        PlaneShape::Disc { diameter_cells } => (diameter_cells, diameter_cells, DomainKind::FlatDisc),
    };
    if gx == 0 || gy == 0 || gx * gy > 64_000_000 {
        return Err(Error::InvalidParam("grid size out of range".into()));
    }
    let is_disc = matches!(p.shape, PlaneShape::Disc { .. });
    if p.boundary == BoundaryKind::Wrap {
        if is_disc {
            return Err(Error::InvalidParam("a disc cannot wrap".into()));
        }
        if gx < 3 || gy < 3 {
            return Err(Error::InvalidParam("wrapped rectangle needs at least 3 cells per side".into()));
        }
    }
    let dx = p.cell_size;
    let radius = gx as f64 * dx / 2.0;
    let cx = gx as f64 * dx / 2.0;
    let cy = gy as f64 * dx / 2.0;

    let mut index = vec![u32::MAX; gx * gy];
    let mut cells: Vec<(usize, usize)> = Vec::new();
    for j in 0..gy {
        for i in 0..gx {
            let x = (i as f64 + 0.5) * dx - cx;
            let y = (j as f64 + 0.5) * dx - cy;
            if !is_disc || x * x + y * y <= radius * radius {
                index[j * gx + i] = cells.len() as u32;
                cells.push((i, j));
            }
        }
    }
    let n = cells.len();
    if n == 0 {
        return Err(Error::InvalidParam("no cells generated".into()));
    }
    let mut mb = MeshBuilder::new(n);
    let wrap = p.boundary == BoundaryKind::Wrap;
    for (k, &(i, j)) in cells.iter().enumerate() {
        let x = (i as f64 + 0.5) * dx - cx;
        let y = (j as f64 + 0.5) * dx - cy;
        mb.set_cell(k, dx * dx, [x, y, 0.0], [0.0, 0.0, 1.0]);
        for &(di, dj) in &[(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let mut a = i as i64 + di;
            let mut b = j as i64 + dj;
            if wrap {
                a = a.rem_euclid(gx as i64);
                b = b.rem_euclid(gy as i64);
            }
            let inside = a >= 0 && b >= 0 && (a as usize) < gx && (b as usize) < gy;
            let nb = if inside { index[b as usize * gx + a as usize] } else { u32::MAX };
            if nb != u32::MAX {
                mb.add_neighbor(k, nb as usize, dx, dx);
            } else {
                mb.add_boundary(k, dx);
            }
        }
    }
    let mesh = mb.build();
    let gravity = vec![[0.0, 0.0, -p.surface_gravity]; n];
    // "North" = +y on a plane.
    Domain::new(kind, mesh, gravity, p.boundary, [0.0, 1.0, 0.0], p.seed)
}
