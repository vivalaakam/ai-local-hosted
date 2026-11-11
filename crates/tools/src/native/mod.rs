mod current_time;
mod get_weather;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::ToolError;
use crate::http::HttpClient;

pub use current_time::CURRENT_TIME_SLUG;
pub use get_weather::{GET_WEATHER_SLUG, WeatherEndpoints};

/// Stable seed ids for native tool manifests (must match migrations/0006_tools.sql).
pub const CURRENT_TIME_UUID: Uuid = Uuid::from_u128(0x1001);
pub const GET_WEATHER_UUID: Uuid = Uuid::from_u128(0x1002);

#[derive(Debug, Clone)]
pub struct NativeManifest {
    pub id: Uuid,
    pub slug: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

pub fn native_manifests() -> Vec<NativeManifest> {
    vec![
        NativeManifest {
            id: CURRENT_TIME_UUID,
            slug: CURRENT_TIME_SLUG,
            name: "Current time",
            description: "Returns the current UTC time as an ISO-8601 string.",
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        NativeManifest {
            id: GET_WEATHER_UUID,
            slug: GET_WEATHER_SLUG,
            name: "Get weather",
            description: "Fetches current weather for a place name via Open-Meteo.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "City or place name"
                    }
                },
                "required": ["location"],
                "additionalProperties": false
            }),
        },
    ]
}

pub async fn call_native(
    slug: &str,
    arguments: &str,
    http: &HttpClient,
    weather: &WeatherEndpoints,
) -> Result<String, ToolError> {
    match slug {
        CURRENT_TIME_SLUG => current_time::call(arguments),
        GET_WEATHER_SLUG => get_weather::call(arguments, http, weather).await,
        other => Err(ToolError::not_found(format!("unknown native tool {other}"))),
    }
}
