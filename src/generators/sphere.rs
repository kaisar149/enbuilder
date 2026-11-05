//! Geodesic hex-pent mesh: subdivide an icosahedron, then take the dual.
//! Cells = 10*4^L + 2 (12 pentagons, rest hexagons). Cell boundaries are
//! great-circle arcs between projected triangle centroids, so the cells tile
//! the sphere exactly and the area sum equals 4*pi*R^2 to float precision.
use crate::domain::{BoundaryKind, Domain, DomainKind};
use crate::error::{Error, Result};
use crate::math::*;
use crate::mesh::MeshBuilder;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SphereParams {
    pub radius: f64,
    /// Number of 4x subdivisions of the icosahedron (0..=9).
    pub subdivision: u32,
    pub surface_gravity: f64,
    pub seed: u64,
}

impl Default for SphereParams {
    fn default() -> Self {
        SphereParams { radius: 6.371e6, subdivision: 5, surface_gravity: 9.81, seed: 0 }
    }
}

fn icosahedron() -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let t = (1.0 + 5.0f64.sqrt()) / 2.0;
    let raw: [Vec3; 12] = [
        [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
        [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
        [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
    ];
    let faces: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    (raw.iter().map(|&v| normalize(v)).collect(), faces)
}

fn midpoint(verts: &mut Vec<Vec3>, cache: &mut HashMap<(u32, u32), u32>, a: u32, b: u32) -> u32 {
    let key = if a < b { (a, b) } else { (b, a) };
    if let Some(&i) = cache.get(&key) {
        return i;
    }
    let m = normalize(add(verts[a as usize], verts[b as usize]));
    verts.push(m);
    let i = (verts.len() - 1) as u32;
    cache.insert(key, i);
    i
}

fn tangent_basis(p: Vec3) -> (Vec3, Vec3) {
    let h = if p[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let e1 = normalize(cross(p, h));
    (e1, cross(p, e1))
}

fn shared_neighbor(f0: &[u32; 3], f1: &[u32; 3], v: u32) -> Option<u32> {
    f0.iter().copied().find(|&w| w != v && f1.contains(&w))
}

pub fn generate_sphere(p: &SphereParams) -> Result<Domain> {
    if !(p.radius > 0.0 && p.radius.is_finite()) {
        return Err(Error::InvalidParam("radius must be positive".into()));
    }
    if p.subdivision > 9 {
        return Err(Error::InvalidParam("subdivision > 9 would exceed ~2.6M cells".into()));
    }
    let (mut verts, mut faces) = icosahedron();
    for _ in 0..p.subdivision {
        let mut cache: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next: Vec<[u32; 3]> = Vec::with_capacity(faces.len() * 4);
        for f in faces.iter() {
            let (a, b, c) = (f[0], f[1], f[2]);
            let ab = midpoint(&mut verts, &mut cache, a, b);
            let bc = midpoint(&mut verts, &mut cache, b, c);
            let ca = midpoint(&mut verts, &mut cache, c, a);
            next.push([a, ab, ca]);
            next.push([b, bc, ab]);
            next.push([c, ca, bc]);
            next.push([ab, bc, ca]);
        }
        faces = next;
    }

    let n = verts.len();
    let tri_cen: Vec<Vec3> = faces
        .iter()
        .map(|f| normalize(add(add(verts[f[0] as usize], verts[f[1] as usize]), verts[f[2] as usize])))
        .collect();
    let mut incident: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (ti, f) in faces.iter().enumerate() {
        for &v in f.iter() {
            incident[v as usize].push(ti as u32);
        }
    }

    let r = p.radius;
    let mut mb = MeshBuilder::new(n);
    for v in 0..n {
        let pv = verts[v];
        let (e1, e2) = tangent_basis(pv);
        let mut order: Vec<(f64, u32)> = incident[v]
            .iter()
            .map(|&t| {
                let c = tri_cen[t as usize];
                (dot(c, e2).atan2(dot(c, e1)), t)
            })
            .collect();
        order.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        let m = order.len();
        let mut area = 0.0;
        for k in 0..m {
            let t0 = order[k].1 as usize;
            let t1 = order[(k + 1) % m].1 as usize;
            let w = shared_neighbor(&faces[t0], &faces[t1], v as u32)
                .ok_or_else(|| Error::InvalidMesh("dual construction: triangles share no edge".into()))?;
            let (c0, c1) = (tri_cen[t0], tri_cen[t1]);
            area += spherical_triangle_area(pv, c0, c1);
            mb.add_neighbor(v, w as usize, arc_angle(c0, c1) * r, arc_angle(pv, verts[w as usize]) * r);
        }
        mb.set_cell(v, area * r * r, scale(pv, r), pv);
    }
    let mesh = mb.build();
    let gravity = mesh.up.iter().map(|&u| scale(u, -p.surface_gravity)).collect();
    Domain::new(DomainKind::Sphere, mesh, gravity, BoundaryKind::Closed, [0.0, 0.0, 1.0], p.seed)
}
