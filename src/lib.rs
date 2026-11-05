//! worldgen-core, Part 1: foundations. No physics here, only the skeleton
//! every later module plugs into.
pub mod domain;
pub mod error;
pub mod fields;
pub mod generators;
pub mod math;
pub mod mesh;
pub mod module;
pub mod scheduler;
pub mod testmods;

pub use domain::{BoundaryKind, Domain, DomainKind, Fluid, Light, LightKind, Media, SurfaceFrame, VerticalStructure};
pub use error::{Error, Result};
pub use fields::{Diff, Field, FieldStore};
pub use mesh::{Mesh, MeshBuilder};
pub use module::{DirtyRegion, Module, ModuleSpec, ParamSet};
pub use scheduler::{FeedbackLoop, LoopReport, RunReport, Scheduler};
