//! Server-side tool catalog runtime: native handlers and Rhai scripts.

mod error;
mod execute;
mod http;
mod names;
mod native;
mod rhai_runtime;

pub use error::{ToolError, ToolErrorKind};
pub use execute::{
    ToolExecutor, ToolKind, ToolSpec, catalog_prompt, definitions_for, seeded_native_specs,
};
pub use names::{ToolOwner, USER_PREFIX, model_name, parse_model_name};
pub use native::{
    CURRENT_TIME_SLUG, CURRENT_TIME_UUID, GET_WEATHER_SLUG, GET_WEATHER_UUID, NativeManifest,
    WeatherEndpoints, native_manifests,
};
