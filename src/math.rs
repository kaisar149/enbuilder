//! Tiny dependency-free vector helpers (swap for `glam` later if desired).
pub type Vec3 = [f64; 3];

pub fn add(a: Vec3, b: Vec3) -> Vec3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
pub fn sub(a: Vec3, b: Vec3) -> Vec3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
pub fn scale(a: Vec3, k: f64) -> Vec3 { [a[0] * k, a[1] * k, a[2] * k] }
pub fn dot(a: Vec3, b: Vec3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn length(a: Vec3) -> f64 { dot(a, a).sqrt() }
pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

/// Angle between two vectors; stable for tiny and near-antipodal angles.
pub fn arc_angle(a: Vec3, b: Vec3) -> f64 { length(cross(a, b)).atan2(dot(a, b)) }

/// Area of the spherical triangle on the unit sphere (Van Oosterom-Strackee).
pub fn spherical_triangle_area(a: Vec3, b: Vec3, c: Vec3) -> f64 {
    let triple = dot(a, cross(b, c)).abs();
    let denom = 1.0 + dot(a, b) + dot(b, c) + dot(c, a);
    2.0 * triple.atan2(denom)
}
