//! enbuilder: physics-driven world generator core.
//! Part 1 = foundations (domain, meshes, fields, scheduler); Part 2 = terrain and energy (stages A, B).
pub mod consts;
pub mod domain;
pub mod energy;
pub mod error;
pub mod fields;
pub mod generators;
pub mod insolation;
pub mod math;
pub mod mesh;
pub mod module;
pub mod noise;
pub mod rng;
pub mod scheduler;
pub mod tectonics;
pub mod terrain;
pub mod testmods;

pub use domain::{BoundaryKind, Domain, DomainKind, Fluid, Light, LightKind, Media, SurfaceFrame, VerticalStructure};
pub use error::{Error, Result};
pub use fields::{Diff, Field, FieldStore};
pub use mesh::{Mesh, MeshBuilder};
pub use module::{DirtyRegion, Module, ModuleSpec, ParamSet};
pub use energy::{add_energy_stage, add_energy_stage_with, seed_temperature, AlbedoModel, EnergyBalance};
pub use insolation::Insolation;
pub use tectonics::{PaintedPlates, PlateDef, Tectonics};
pub use terrain::TerrainShaping;
pub use scheduler::{FeedbackLoop, LoopReport, RunReport, Scheduler};
