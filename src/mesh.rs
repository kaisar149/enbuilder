use crate::error::{Error, Result};
use crate::math::Vec3;

/// Unstructured cell mesh with CSR adjacency. Modules see only this.
///
/// For cell `i`, its neighbours are `adj[adj_offsets[i]..adj_offsets[i+1]]`
/// (sorted ascending). `edge_len[k]` is the length of the shared face/edge
/// (2D: metres; the same index `k` addresses `adj`, `edge_len`, `edge_dist`).
/// `edge_dist[k]` is the centre-to-centre distance across that edge, so a
/// flux coefficient is `edge_len / edge_dist`.
/// `boundary_len[i]` is the length of cell edges that face *outside* the
/// domain (wall/void); wrapped or interior edges never count here.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub n_cells: usize,
    pub area: Vec<f64>,
    pub centroid: Vec<Vec3>,
    pub up: Vec<Vec3>,
    pub adj_offsets: Vec<u32>,
    pub adj: Vec<u32>,
    pub edge_len: Vec<f64>,
    pub edge_dist: Vec<f64>,
    pub boundary_len: Vec<f64>,
}

impl Mesh {
    fn range(&self, i: usize) -> std::ops::Range<usize> {
        self.adj_offsets[i] as usize..self.adj_offsets[i + 1] as usize
    }
    pub fn neighbors(&self, i: usize) -> &[u32] { &self.adj[self.range(i)] }
    pub fn edge_lens(&self, i: usize) -> &[f64] { &self.edge_len[self.range(i)] }
    pub fn edge_dists(&self, i: usize) -> &[f64] { &self.edge_dist[self.range(i)] }
    pub fn degree(&self, i: usize) -> usize { self.range(i).len() }
    pub fn total_area(&self) -> f64 { self.area.iter().sum() }

    /// Structural checks: sizes, sorted/unique/in-range neighbours, no
    /// self-loops, symmetric adjacency with matching edge length and distance.
    pub fn validate(&self) -> Result<()> {
        let n = self.n_cells;
        let bad = |s: String| Err(Error::InvalidMesh(s));
        if self.area.len() != n || self.centroid.len() != n || self.up.len() != n || self.boundary_len.len() != n {
            return bad("per-cell array length mismatch".into());
        }
        if self.adj_offsets.len() != n + 1 || self.adj.len() != self.edge_len.len() || self.adj.len() != self.edge_dist.len() {
            return bad("CSR array length mismatch".into());
        }
        for i in 0..n {
            if !(self.area[i].is_finite() && self.area[i] > 0.0) {
                return bad(format!("cell {i} has non-positive area"));
            }
            let nb = self.neighbors(i);
            for w in nb.windows(2) {
                if w[0] >= w[1] {
                    return bad(format!("cell {i} neighbours unsorted or duplicated"));
                }
            }
            for (q, &j) in nb.iter().enumerate() {
                let ju = j as usize;
                if ju >= n || ju == i {
                    return bad(format!("cell {i} has invalid neighbour {j}"));
                }
                match self.neighbors(ju).binary_search(&(i as u32)) {
                    Ok(p) => {
                        let (l1, l2) = (self.edge_lens(i)[q], self.edge_lens(ju)[p]);
                        let (d1, d2) = (self.edge_dists(i)[q], self.edge_dists(ju)[p]);
                        if (l1 - l2).abs() > 1e-9 * l1.max(l2) || (d1 - d2).abs() > 1e-9 * d1.max(d2) {
                            return bad(format!("edge {i}<->{ju} has asymmetric length/distance"));
                        }
                        if !(l1 > 0.0 && d1 > 0.0) {
                            return bad(format!("edge {i}<->{ju} has non-positive length/distance"));
                        }
                    }
                    Err(_) => return bad(format!("adjacency {i}->{ju} is not symmetric")),
                }
            }
        }
        Ok(())
    }
}

/// Collects per-cell data and neighbour lists, then freezes into CSR.
pub struct MeshBuilder {
    n: usize,
    area: Vec<f64>,
    centroid: Vec<Vec3>,
    up: Vec<Vec3>,
    nbrs: Vec<Vec<(u32, f64, f64)>>,
    boundary_len: Vec<f64>,
}

impl MeshBuilder {
    pub fn new(n: usize) -> Self {
        MeshBuilder {
            n,
            area: vec![0.0; n],
            centroid: vec![[0.0; 3]; n],
            up: vec![[0.0, 0.0, 1.0]; n],
            nbrs: vec![Vec::new(); n],
            boundary_len: vec![0.0; n],
        }
    }
    pub fn set_cell(&mut self, i: usize, area: f64, centroid: Vec3, up: Vec3) {
        self.area[i] = area;
        self.centroid[i] = centroid;
        self.up[i] = up;
    }
    /// Directed entry i -> j. Call for both directions (generators do).
    pub fn add_neighbor(&mut self, i: usize, j: usize, edge_len: f64, edge_dist: f64) {
        self.nbrs[i].push((j as u32, edge_len, edge_dist));
    }
    pub fn add_boundary(&mut self, i: usize, len: f64) { self.boundary_len[i] += len; }

    pub fn build(mut self) -> Mesh {
        let mut adj_offsets = Vec::with_capacity(self.n + 1);
        let mut adj = Vec::new();
        let mut edge_len = Vec::new();
        let mut edge_dist = Vec::new();
        adj_offsets.push(0u32);
        for list in self.nbrs.iter_mut() {
            list.sort_by_key(|e| e.0);
            for &(j, l, d) in list.iter() {
                adj.push(j);
                edge_len.push(l);
                edge_dist.push(d);
            }
            adj_offsets.push(adj.len() as u32);
        }
        Mesh {
            n_cells: self.n,
            area: self.area,
            centroid: self.centroid,
            up: self.up,
            adj_offsets,
            adj,
            edge_len,
            edge_dist,
            boundary_len: self.boundary_len,
        }
    }
}
