pub mod cube;
pub mod plane;
pub mod sphere;

pub use cube::{generate_cube, CubeParams};
pub use plane::{generate_plane, PlaneParams, PlaneShape};
pub use sphere::{generate_sphere, SphereParams};
