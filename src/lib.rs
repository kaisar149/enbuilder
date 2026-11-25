//! enbuilder: physics-driven world generator core.
//! Part 1 = foundations (domain, meshes, fields, scheduler); Part 2 = terrain and energy (stages A, B);
//! Part 3a = diagnostic pressure and wind (stage C, air side); Part 3b-1 = evaporation sources; Part 3b-2 = conservative moisture transport; Part 3b-3 = lift and precipitation; Part 3b-4 = budget and seasons; Part 3c-1 = stepped wind (CPU); Part 4a-1 = sea level, ocean mask, wind stress; Part 4a-2b = wind-stress curl and beta; Part 4a-2c = gyres and current field (beta off); Part 4a-2d = beta term and BiCGSTAB; Part 4a-2e = wall and waterworld rules, tuned defaults, validation helpers (`ocean_validation`); Part 4a-3 = ocean heat transport and SST feedback (`ocean_heat`); Part 4a-4 = upwelling, thermohaline, sea ice, nutrients (`ocean_secondary`); Part 3c-5 = backend switch (`add_atmosphere_stage_stepped_with`, `add_climate_pipeline_with`); Part 4b-1 = depression handling, flow direction, drainage area (`flow_routing`); Part 4b-2 = runoff, discharge, rivers, fill-and-spill lakes (`rivers_lakes`); Part 4b-3 = soil moisture, snow, glaciers, land sublimation and soil-driven runoff (`land_water`); Part 4c-1 = stream power erosion (`erosion`, `TerrainShaping::with_erosion`); Part 4c-2 = sediment transport, deposition and thermal erosion (`sediment`, `TerrainShaping::with_net_erosion`).
pub mod anomaly;
pub mod backend;
pub mod biomes;
pub mod budget;
pub mod consts;
pub mod derived;
pub mod diff;
pub mod dirty;
pub mod domain;
pub mod energy;
pub mod erosion;
pub mod error;
pub mod fields;
pub mod flow_routing;
pub mod generators;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod gradient;
pub mod hydraulics;
pub mod storm;
pub mod incremental;
pub mod insolation;
pub mod land_water;
pub mod landscape;
pub mod math;
pub mod mesh;
pub mod module;
pub mod moisture;
pub mod noise;
pub mod ocean;
pub mod ocean_currents;
pub mod ocean_forcing;
pub mod ocean_heat;
pub mod ocean_secondary;
pub mod ocean_validation;
pub mod precipitation;
pub mod provenance;
pub mod projection;
pub mod pressure;
pub mod rivers_lakes;
pub mod rng;
pub mod scheduler;
pub mod sediment;
pub mod tectonics;
pub mod terrain;
pub mod transport;
pub mod vegetation;
pub mod testmods;
pub mod toyclimate;
pub mod testworld;
pub mod wind;
pub mod wind_gpu;
pub mod wind_stepped;

pub use domain::{BoundaryKind, Domain, DomainKind, Fluid, Light, LightKind, Media, SurfaceFrame, VerticalStructure};
pub use error::{Error, Result};
pub use fields::{Diff, Field, FieldStore};
pub use mesh::{Mesh, MeshBuilder};
pub use module::{DirtyRegion, Module, ModuleSpec, ParamSet};
pub use energy::{add_energy_stage, add_energy_stage_with, seed_temperature, AlbedoModel, EnergyBalance};
pub use insolation::Insolation;
pub use tectonics::{PaintedPlates, PlateDef, Tectonics};
pub use terrain::TerrainShaping;
pub use scheduler::{FeedbackLoop, LoopReport, ModuleEvent, RunReport, Scheduler, Trace, TraceCfg};
pub use budget::{add_climate_pipeline, mean_evap_mm_day, mean_precip_mm_day, run_season, SeasonReport, WaterBudget};
pub use gradient::GradientOp;
pub use pressure::{add_atmosphere_stage, PressureModel};
pub use backend::{make_backend, BackendKind, ComputeBackend, Cpu, CpuF32, PackedMesh};
pub use wind::DiagnosticWind;
pub use wind_stepped::{add_atmosphere_stage_stepped, add_atmosphere_stage_stepped_with, Backend, SteppedWind};
pub use transport::{add_moisture_precip_stage, add_moisture_transport_stage, MoistureTransport};
pub use moisture::{add_moisture_source_stage, saturation_vapour_pressure, MoistureSource};
pub use ocean::{add_ocean_mask_stage, add_ocean_stage, CgConfig, ComponentSolve, OceanAdvection, OceanGraph, OceanMask, OceanSolution, SeaLevel, WindStress};
pub use ocean_forcing::{add_gyre_forcing_stage, beta_of, curl_of, curl_of_masked, unpack_vec3, BetaTerm, CurlOp, WindStressCurl};
pub use ocean_currents::{add_ocean_currents_stage, apply_coast_slip, boundary_layer_report, boundary_layer_warning, clamp_speed, BoundaryLayerReport, DEFAULT_R_FRICTION, DEFAULT_H_MIXED_M, effective_ocean, effective_ocean_with, EffectiveOcean, coast_outflux, coast_vectors, current_from_psi, OceanCurrents};
pub use ocean_heat::{add_atmosphere_ocean_pipeline, add_ocean_climate_loop, run_ocean_climate, seed_ocean_heat_flux, OceanClimateReport, OceanHeat};
pub use energy::add_energy_stage_ocean_flux;
pub use ocean_secondary::{add_atmosphere_ocean_pipeline_full, add_ocean_climate_loop_full, add_ocean_secondary_stage, Nutrients, SeaIce, Thermohaline, Upwelling};
pub use energy::add_energy_stage_ocean_flux_with;
pub use flow_routing::{add_flow_routing_module, add_flow_routing_stage, FlowRouting};
pub use rivers_lakes::{add_rivers_lakes_module, add_rivers_lakes_stage, lake_units, solve_lake, LakeSolve, RiversLakes};
pub use moisture::add_moisture_source_land_stage;
pub use land_water::{
    add_land_water_modules, add_land_water_pipeline, commit_land_state, glacier_flow, land_cell_step, run_land_cycle, seed_land_state, set_land_param, set_land_step, snow_fraction,
    LandBudget, LandParams, LandSeason, LandSupply, LandWater,
};
pub use erosion::{add_erosion_module, commit_erosion_state, incise, run_erosion_steps, seed_erosion_state, Erosion, ErosionStep, EROSION_PARAMS};
pub use sediment::{
    add_sediment_module, add_surface_stage, commit_sediment_state, fill_basin, route_sediment, run_surface_steps, seed_sediment_state, seed_surface_state, set_surface_years,
    thermal_relax, transport_capacity, RouteInput, RouteParams, Routed, Sediment, SedimentBudget, SurfaceStep, ThermalReport, SEDIMENT_PARAMS,
};
pub use landscape::{
    add_geology_stage, landscape_summary, run_geology, run_geology_prescribed, seed_landscape, seed_landscape_with, ClimateRun, GeoStep, GeologyParams, GeologyReport, LandscapeMask,
    LandscapeSummary, LANDSCAPE_MAX_ITERS, LANDSCAPE_TOL,
};
pub use biomes::{
    add_biome_stage, biome_summary, capture_season, classify_biomes, koppen_class, koppen_group, pet_mm_day, run_climate_seasons, season_normals, warm_half, BiomeDef, BiomeSummary, BiomeTable,
    Biomes, ClimateNormals, Condition, Koppen, Normals, NormalsParams, KOPPEN_CODES, NORMAL_FIELDS,
};
pub use vegetation::{
    clear_vegetation, land_surface_means, run_vegetation_loop, seasonal_climate, seed_vegetation, VegIteration, VegetationParams, VegetationReport, VEG_FIELDS,
};
pub use derived::{add_derived_stage, habitability, mean_slope, ramp, soil_fertility, travel_difficulty, DerivedMaps, DerivedParams, DERIVED_INPUTS};
pub use dirty::{downstream_fields, extend_downwind, grow_region, plan_dirty, DirtyPlan, InfluenceSpec, Region};
pub use incremental::{pin_cells, resolve_edit, solve_full, Edit, FieldReport, IncrementalConfig, IncrementalReport};
pub use diff::{field_diff, ChangeSet, DiffOptions, DiffSummary, FieldChange, FieldDiff};
pub use provenance::{explain, ExplainOptions, Explanation, Hop, Link, Provenance, MAX_CONTRIBUTORS};
