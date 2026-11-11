use serde_json::{Value, json};

use crate::error::ToolError;
use crate::http::HttpClient;

pub const GET_WEATHER_SLUG: &str = "get_weather";

#[derive(Debug, Clone)]
pub struct WeatherEndpoints {
    pub geocoding_base: String,
    pub forecast_base: String,
}

impl Default for WeatherEndpoints {
    fn default() -> Self {
        Self {
            geocoding_base: "https://geocoding-api.open-meteo.com".into(),
            forecast_base: "https://api.open-meteo.com".into(),
        }
    }
}

pub async fn call(
    arguments: &str,
    http: &HttpClient,
    endpoints: &WeatherEndpoints,
) -> Result<String, ToolError> {
    let value: Value = if arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(arguments)
            .map_err(|err| ToolError::invalid(format!("arguments must be json: {err}")))?
    };
    let location = value
        .get("location")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|location| !location.is_empty())
        .ok_or_else(|| ToolError::invalid("location is required"))?;

    let geo_url = format!(
        "{}/v1/search?name={}&count=1",
        endpoints.geocoding_base.trim_end_matches('/'),
        urlencoding_encode(location)
    );
    let geo = http.request("GET", &geo_url, None, None).await?;
    let status = geo["status"].as_u64().unwrap_or(0);
    if status != 200 {
        return Err(ToolError::http(format!(
            "geocoding failed with status {status}"
        )));
    }
    let body: Value = serde_json::from_str(geo["body"].as_str().unwrap_or(""))
        .map_err(|err| ToolError::http(format!("geocoding json: {err}")))?;
    let place = body
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| results.first())
        .ok_or_else(|| ToolError::not_found(format!("place not found: {location}")))?;
    let latitude = place
        .get("latitude")
        .and_then(Value::as_f64)
        .ok_or_else(|| ToolError::http("geocoding missing latitude"))?;
    let longitude = place
        .get("longitude")
        .and_then(Value::as_f64)
        .ok_or_else(|| ToolError::http("geocoding missing longitude"))?;
    let name = place
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(location)
        .to_string();
    let country = place
        .get("country")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let forecast_url = format!(
        "{}/v1/forecast?latitude={latitude}&longitude={longitude}&current_weather=true",
        endpoints.forecast_base.trim_end_matches('/')
    );
    let forecast = http.request("GET", &forecast_url, None, None).await?;
    let status = forecast["status"].as_u64().unwrap_or(0);
    if status != 200 {
        return Err(ToolError::http(format!(
            "forecast failed with status {status}"
        )));
    }
    let body: Value = serde_json::from_str(forecast["body"].as_str().unwrap_or(""))
        .map_err(|err| ToolError::http(format!("forecast json: {err}")))?;
    let current = body.get("current_weather").cloned().unwrap_or(Value::Null);

    Ok(json!({
        "location": name,
        "country": country,
        "latitude": latitude,
        "longitude": longitude,
        "current_weather": current,
    })
    .to_string())
}

fn urlencoding_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(nibble(byte >> 4));
                out.push(nibble(byte & 0x0f));
            }
        }
    }
    out
}

fn nibble(value: u8) -> char {
    char::from(match value {
        0..=9 => b'0' + value,
        _ => b'A' + (value - 10),
    })
}
