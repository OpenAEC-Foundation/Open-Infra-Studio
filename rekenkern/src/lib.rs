//! Open Infra's calculation core: pure functions on plain data (metres,
//! seconds, m³/s), without Open CAD Studio, so they are tested here on
//! analytical cases and used by the plugins' commands. Every design value
//! carries its source in the doc comments.

/// A point in plan (x, y) in metres.
pub type P2 = [f64; 2];
/// A point in space (x, y, z) in metres.
pub type P3 = [f64; 3];

pub mod balans;
pub mod berging;
pub mod bui;
pub mod buien;
pub mod druknet;
pub mod ids;
pub mod kabel;
pub mod markering;
pub mod mesh3d;
pub mod polygoon;
pub mod riool;
pub mod step;
pub mod waterloop;
pub mod zonnepark;
