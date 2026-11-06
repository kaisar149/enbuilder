use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    InvalidParam(String),
    InvalidMesh(String),
    /// Modules involved in (or between) undeclared dependency cycles.
    Cycle(Vec<String>),
    MultipleProducers { field: String, a: String, b: String },
    UnknownModule(String),
    UnknownField(String),
    DuplicateModule(String),
    MissingInput { module: String, field: String },
    MissingOutput { module: String, field: String },
    NonFinite(String),
    GroupState(String),
    /// No usable GPU adapter (or the `gpu` feature is not compiled in).
    NoGpu(String),
    HistoryTruncated { requested: u64, oldest_available: u64 },
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;
        match self {
            InvalidParam(s) => write!(f, "invalid parameter: {s}"),
            InvalidMesh(s) => write!(f, "invalid mesh: {s}"),
            Cycle(m) => write!(f, "undeclared dependency cycle involving modules: {}", m.join(", ")),
            MultipleProducers { field, a, b } => write!(f, "field '{field}' is produced by both '{a}' and '{b}'"),
            UnknownModule(s) => write!(f, "unknown module '{s}'"),
            UnknownField(s) => write!(f, "unknown field '{s}'"),
            DuplicateModule(s) => write!(f, "module '{s}' registered twice"),
            MissingInput { module, field } => write!(f, "module '{module}' needs input field '{field}' which does not exist"),
            MissingOutput { module, field } => write!(f, "module '{module}' declared output '{field}' but did not produce it"),
            NonFinite(s) => write!(f, "non-finite value in field '{s}'"),
            GroupState(s) => write!(f, "edit group error: {s}"),
            NoGpu(s) => write!(f, "no GPU available: {s}"),
            HistoryTruncated { requested, oldest_available } => {
                write!(f, "history truncated: asked for version {requested}, oldest available is {oldest_available}")
            }
        }
    }
}

impl std::error::Error for Error {}
