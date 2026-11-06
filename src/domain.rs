use crate::error::{Error, Result};
use crate::math::{cross, dot, length, normalize, scale, sub, Vec3};
use crate::mesh::Mesh;

#[derive(Debug, Clone, PartialEq)]
pub enum DomainKind { Sphere, Cube, FlatRect, FlatDisc, Custom(String) }

/// Boundary condition of the domain as a whole. Closed = no boundary at all
/// (sphere, cube). Part 1 only builds Closed/Wrap/Wall/Void; OpenSky and
/// SeaFloor are reserved for vertical boundaries in later parts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoundaryKind { Closed, Wrap, Wall, Void, OpenSky, SeaFloor }

/// Local frame. `east = north x up`, so (east, north, up) is right-handed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceFrame { pub up: Vec3, pub north: Vec3, pub east: Vec3 }

#[derive(Debug, Clone, PartialEq)]
pub enum LightKind { Point { position: Vec3 }, Directional { direction: Vec3 } }

/// Stub for Part 3 (insolation): data only, no behaviour yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    /// Directional: irradiance at the world (W/m^2, e.g. 1361). Point: irradiance
    /// at `reference_distance_m` from the light.
    pub intensity: f64,
    /// 0 = no orbital motion (season is ignored).
    pub orbital_period_s: f64,
    /// 0 = no rotation (static light, e.g. tidally locked or flat worlds).
    pub day_length_s: f64,
    pub axial_tilt_rad: f64,
    /// Point lights only: inverse-square reference distance (m).
    pub reference_distance_m: f64,
}

impl Light {
    /// `direction` points from the world toward the star at the equinox (season 0).
    pub fn directional(direction: Vec3, intensity: f64) -> Light {
        Light {
            kind: LightKind::Directional { direction },
            intensity,
            orbital_period_s: 0.0,
            day_length_s: 0.0,
            axial_tilt_rad: 0.0,
            reference_distance_m: 1.0,
        }
    }
    pub fn point(position: Vec3, intensity: f64, reference_distance_m: f64) -> Light {
        Light {
            kind: LightKind::Point { position },
            intensity,
            orbital_period_s: 0.0,
            day_length_s: 0.0,
            axial_tilt_rad: 0.0,
            reference_distance_m,
        }
    }
    pub fn with_orbit(mut self, orbital_period_s: f64, day_length_s: f64, axial_tilt_rad: f64) -> Light {
        self.orbital_period_s = orbital_period_s;
        self.day_length_s = day_length_s;
        self.axial_tilt_rad = axial_tilt_rad;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fluid { pub density: f64, pub heat_capacity: f64, pub viscosity: f64 }

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Media { pub atmosphere: Option<Fluid>, pub ocean: Option<Fluid> }

impl Media {
    pub fn earth_like() -> Self {
        Media {
            atmosphere: Some(Fluid { density: 1.2, heat_capacity: 1005.0, viscosity: 1.8e-5 }),
            ocean: Some(Fluid { density: 1025.0, heat_capacity: 3990.0, viscosity: 1.1e-3 }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerticalStructure { pub atmosphere_layers: usize, pub ocean_layers: usize }

#[derive(Debug, Clone)]
pub struct Domain {
    pub kind: DomainKind,
    pub mesh: Mesh,
    pub frames: Vec<SurfaceFrame>,
    /// Gravity acceleration vector per cell (m/s^2); points "down".
    pub gravity: Vec<Vec3>,
    pub lights: Vec<Light>,
    pub media: Media,
    pub boundary: BoundaryKind,
    pub layers: VerticalStructure,
    /// Unit rotation axis in world coordinates (the `north_ref` given at construction).
    pub spin_axis: Vec3,
    pub seed: u64,
}

impl Domain {
    /// `north_ref` is the rotation-axis-like direction used to define "north"
    /// (projected into each cell's tangent plane).
    pub fn new(
        kind: DomainKind,
        mesh: Mesh,
        gravity: Vec<Vec3>,
        boundary: BoundaryKind,
        north_ref: Vec3,
        seed: u64,
    ) -> Result<Domain> {
        mesh.validate()?;
        if gravity.len() != mesh.n_cells {
            return Err(Error::InvalidParam("gravity length != n_cells".into()));
        }
        let frames = compute_frames(&mesh, north_ref);
        Ok(Domain {
            kind,
            mesh,
            frames,
            gravity,
            lights: Vec::new(),
            media: Media::earth_like(),
            boundary,
            layers: VerticalStructure { atmosphere_layers: 4, ocean_layers: 3 },
            spin_axis: normalize(north_ref),
            seed,
        })
    }

    // The only things modules should ask of a Domain:
    pub fn n_cells(&self) -> usize { self.mesh.n_cells }
    pub fn neighbors(&self, i: usize) -> &[u32] { self.mesh.neighbors(i) }
    pub fn area(&self, i: usize) -> f64 { self.mesh.area[i] }
    pub fn up(&self, i: usize) -> Vec3 { self.mesh.up[i] }
    pub fn gravity_at(&self, i: usize) -> Vec3 { self.gravity[i] }
    pub fn gravity_magnitude(&self, i: usize) -> f64 { length(self.gravity[i]) }
}

pub fn compute_frames(mesh: &Mesh, north_ref: Vec3) -> Vec<SurfaceFrame> {
    (0..mesh.n_cells)
        .map(|i| {
            let up = normalize(mesh.up[i]);
            let mut north = sub(north_ref, scale(up, dot(north_ref, up)));
            if length(north) < 1e-9 {
                // At a pole: pick any tangent direction deterministically.
                let alt = if up[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
                north = sub(alt, scale(up, dot(alt, up)));
            }
            let north = normalize(north);
            SurfaceFrame { up, north, east: cross(north, up) }
        })
        .collect()
}
