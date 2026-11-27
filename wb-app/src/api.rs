//! The API contract (UI_ARCHITECTURE section 2): commands, queries, events. JSON for structure, raw bytes for arrays.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bump on any breaking change to the JSON shapes below. A golden-file test pins them.
pub const API_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    LoadPreset { name: String },
    SetParam { name: String, value: f64 },
    /// Starts a background job (returns at once; progress arrives as events).
    Solve,
    /// Cancels the running job, if any.
    Cancel,
    Undo,
    Redo,
    /// Part 9c: starts a background job that solves `count` alternative seeds in parallel (`coarse`: one year of spin-up instead of the
    /// current setting). Results are read with `get_variations` / `get_variation`. Does not change any parameter.
    Variations {
        count: usize,
        #[serde(default = "default_coarse")]
        coarse: bool,
    },
    /// Part 9f: one brush stroke (batched by the client) on a paint layer (`elevation`, `temperature_bias`, `rainfall_bias`, `biome_override`).
    /// `weights` are per-cell brush weights in [0, 1] (radius and falloff are applied by the client). One undo step.
    PaintStroke {
        layer: String,
        op: crate::paint::Op,
        cells: Vec<u32>,
        weights: Vec<f64>,
        #[serde(default)]
        value: Option<f64>,
        #[serde(default = "default_strength")]
        strength: f64,
    },
    /// Part 9f: lock (`locked: true`, at the current value) or unlock cells of an output field (`elevation`, `temperature`, `precipitation`, `biome`).
    SetLock { field: String, cells: Vec<u32>, locked: bool },
    /// Part 9f: realism slider (0..100) for one output field, or `all`. Instant: the view is re-blended, nothing is re-solved.
    SetRealism { field: String, value: f64 },
    /// Part 9f: background re-solve after edits. `preview` (warm start, 1-2 seasons), `refined` (warm start, 12 seasons) or `full` (= solve).
    Refresh { mode: String },
}

fn default_strength() -> f64 { 1.0 }

fn default_coarse() -> bool { true }

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Command::LoadPreset { .. } => "load_preset",
            Command::SetParam { .. } => "set_param",
            Command::Solve => "solve",
            Command::Cancel => "cancel",
            Command::Undo => "undo",
            Command::Redo => "redo",
            Command::Variations { .. } => "variations",
            Command::PaintStroke { .. } => "paint_stroke",
            Command::SetLock { .. } => "set_lock",
            Command::SetRealism { .. } => "set_realism",
            Command::Refresh { .. } => "refresh",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Query {
    GetSchema,
    GetWorldInfo,
    GetMesh,
    GetField { name: String, #[serde(default)] layer: usize },
    GetCell { cell: usize },
    /// Part 9c: the seeds of the last variation batch (JSON only).
    GetVariations,
    /// Part 9c: one field of one variation (`elevation`, `temperature`, `precipitation` or `biome`), same binary layout as `get_field`.
    GetVariation { index: usize, name: String },
    /// Part 9e: river polylines as cell paths (topology only), binary. Needs a solve.
    GetRivers,
    /// Part 9e: edges between cells inside a mask (`field > threshold`, default 0.5) and their outside neighbours, binary pairs of cell ids.
    GetOutline { field: String, #[serde(default = "default_threshold")] threshold: f64 },
    /// Part 9f: an output field in one of three views: `blended` (what `get_field` returns, default), `computed` (physics only) or `painted` (realism 0).
    GetFieldView { name: String, #[serde(default)] layer: usize, #[serde(default = "default_view")] view: String },
    /// Part 9f: a paint layer (`value` then `mask`, f32 each).
    GetPaint { layer: String },
    /// Part 9f: locked cells per output field (u32 arrays).
    GetLocks,
}

fn default_view() -> String { "blended".into() }

fn default_threshold() -> f64 { 0.5 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    JobProgress { job: u64, fraction: f64, stage: String },
    /// `status`: "completed", "cancelled" or "failed".
    JobDone { job: u64, status: String },
    FieldChanged { names: Vec<String>, version: u64 },
    WorldLoaded { mesh_id: String, n_cells: usize },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self { ApiError { code: code.into(), message: message.into() } }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}: {}", self.code, self.message) }
}

impl std::error::Error for ApiError {}

impl From<enbuilder::Error> for ApiError {
    fn from(e: enbuilder::Error) -> Self { ApiError::new("sim_error", e.to_string()) }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// What a command returns at once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ack {
    pub api_version: u32,
    pub command: String,
    /// Solved fields that are now stale (also pushed as a `field_changed` event).
    pub invalidated: Vec<String>,
    /// Set by `solve`.
    pub job: Option<u64>,
    /// Part 9f: extra command result (stroke: `applied`, `skipped_locked`). Absent for older commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// A query answer: a JSON header plus optional raw little-endian bytes (f32 / u32 arrays, layout described in the header).
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub json: Value,
    pub binary: Vec<u8>,
}
