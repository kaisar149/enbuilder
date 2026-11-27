//! Part 9e: vector overlay geometry, built in the app core and returned by queries (UI_ARCHITECTURE: the UI never builds it).
//!
//! The answers are TOPOLOGY ONLY (cell ids), no coordinates, so every viewport (2D map, globe, cube, a future one) resolves them to its own
//! space. Two query shapes:
//!   * `get_rivers`  : polylines of cells following `flow_dir`, with `discharge` per vertex (line width).
//!   * `get_outline` : edges between a cell inside a mask and a neighbour outside it (coast = `ocean_mask`, glacier outline = `glacier_mask`,
//!                     lake shore = `lake_depth` with threshold 0, ...). Works for ANY scalar or mask field; nothing names a field.
use crate::api::{ApiError, ApiResult};

/// River polylines. `offsets` has `n_lines + 1` entries; line `k` is `cells[offsets[k]..offsets[k+1]]`, `values` is discharge (m3/s) per vertex.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rivers {
    pub cells: Vec<u32>,
    pub offsets: Vec<u32>,
    pub values: Vec<f32>,
}

impl Rivers {
    pub fn n_lines(&self) -> usize { self.offsets.len().saturating_sub(1) }
    pub fn line(&self, k: usize) -> &[u32] { &self.cells[self.offsets[k] as usize..self.offsets[k + 1] as usize] }
}

/// Splits the river forest into polylines. A line starts at every river cell that is a source or a confluence (upstream river count != 1)
/// and runs downstream until it reaches another confluence (that cell is the last vertex, and the first of the next line) or leaves the
/// river network. The receiver it leaves to (ocean, lake, sink edge) is appended as the final vertex, so every line ends where the water goes.
/// Every river cell is in at least one line; consecutive vertices are always neighbours (`flow_dir` receivers are).
pub fn rivers(flow_dir: &[f64], river: &[f64], discharge: &[f64]) -> ApiResult<Rivers> {
    let n = flow_dir.len();
    if river.len() != n || discharge.len() != n {
        return Err(ApiError::new("sim_error", "rivers: field length mismatch"));
    }
    let is_river = |c: usize| river[c] > 0.5;
    let recv = |c: usize| -> Option<usize> {
        let r = flow_dir[c];
        if r >= 0.0 && (r as usize) < n { Some(r as usize) } else { None }
    };
    let mut up = vec![0u32; n];
    for c in 0..n {
        if is_river(c) {
            if let Some(r) = recv(c) {
                if is_river(r) {
                    up[r] += 1;
                }
            }
        }
    }
    let mut out = Rivers { cells: vec![], offsets: vec![0], values: vec![] };
    for head in 0..n {
        if !is_river(head) || up[head] == 1 {
            continue;
        }
        let mut c = head;
        let mut guard = 0usize;
        loop {
            out.cells.push(c as u32);
            out.values.push(discharge[c].max(0.0) as f32);
            guard += 1;
            if guard > n {
                return Err(ApiError::new("sim_error", "rivers: flow_dir contains a cycle"));
            }
            match recv(c) {
                None => break,
                Some(r) if is_river(r) => {
                    if up[r] != 1 {
                        // confluence: the line ends here, the next one starts here
                        out.cells.push(r as u32);
                        out.values.push(discharge[r].max(0.0) as f32);
                        break;
                    }
                    c = r;
                }
                Some(r) => {
                    // leaves the network: ocean, lake, or the edge of the domain
                    out.cells.push(r as u32);
                    out.values.push(discharge[r].max(0.0).max(discharge[c].max(0.0)) as f32);
                    break;
                }
            }
        }
        out.offsets.push(out.cells.len() as u32);
    }
    Ok(out)
}

/// Edge list: `pairs[2k]` is inside the mask, `pairs[2k+1]` the neighbour outside it. Sorted by inside cell, then neighbour order, so the
/// answer is deterministic.
pub fn outline(adj_offsets: &[u32], adj: &[u32], values: &[f64], threshold: f64) -> Vec<u32> {
    let n = values.len();
    let inside = |c: usize| values[c].is_finite() && values[c] > threshold;
    let mut pairs = vec![];
    for c in 0..n {
        if !inside(c) {
            continue;
        }
        for &nb in &adj[adj_offsets[c] as usize..adj_offsets[c + 1] as usize] {
            if !inside(nb as usize) {
                pairs.push(c as u32);
                pairs.push(nb);
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    // a 1 x 6 strip: 0-1-2-3-4-5, river cells 0..=3 flowing right into ocean cell 4; a tributary cell 5 is not used here
    fn strip_adj(n: usize) -> (Vec<u32>, Vec<u32>) {
        let mut off = vec![0u32];
        let mut adj = vec![];
        for i in 0..n {
            if i > 0 { adj.push((i - 1) as u32); }
            if i + 1 < n { adj.push((i + 1) as u32); }
            off.push(adj.len() as u32);
        }
        (off, adj)
    }

    #[test]
    fn a_single_river_is_one_line_ending_at_the_outlet() {
        let fd = [1.0, 2.0, 3.0, 4.0, -1.0];
        let river = [1.0, 1.0, 1.0, 1.0, 0.0];
        let q = [1.0, 2.0, 3.0, 4.0, 0.0];
        let r = rivers(&fd, &river, &q).unwrap();
        assert_eq!(r.n_lines(), 1);
        assert_eq!(r.line(0), &[0, 1, 2, 3, 4]);
        assert_eq!(r.values, vec![1.0, 2.0, 3.0, 4.0, 4.0]);
    }

    #[test]
    fn a_confluence_ends_one_line_and_starts_the_next() {
        // 0 -> 2, 1 -> 2, 2 -> 3, 3 -> ocean 4
        let fd = [2.0, 2.0, 3.0, 4.0, -1.0];
        let river = [1.0, 1.0, 1.0, 1.0, 0.0];
        let q = [1.0, 1.0, 2.0, 2.0, 0.0];
        let r = rivers(&fd, &river, &q).unwrap();
        assert_eq!(r.n_lines(), 3);
        let lines: Vec<Vec<u32>> = (0..3).map(|k| r.line(k).to_vec()).collect();
        assert!(lines.contains(&vec![0, 2]) && lines.contains(&vec![1, 2]) && lines.contains(&vec![2, 3, 4]));
    }

    #[test]
    fn outline_lists_inside_outside_neighbour_pairs() {
        let (off, adj) = strip_adj(5);
        let v = [1.0, 1.0, 0.0, 0.0, 1.0];
        assert_eq!(outline(&off, &adj, &v, 0.5), vec![1, 2, 4, 3]);
        assert!(outline(&off, &adj, &[1.0; 5], 0.5).is_empty());
    }
}
