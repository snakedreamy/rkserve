/// Target hardware platform identifier. Used for manifest validation and
/// API responses. Update this constant to support a different platform.
pub const PLATFORM: &str = "rk3576";

pub mod api;
pub mod auth;
pub mod domain;
pub mod event_log;
pub mod jobs;
pub mod registry;
pub mod scheduler;
pub mod state_store;
pub mod supervisor;
pub mod telemetry;
