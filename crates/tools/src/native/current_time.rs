use chrono::Utc;
use serde_json::{Value, json};

use crate::error::ToolError;

pub const CURRENT_TIME_SLUG: &str = "current_time";

pub fn call(arguments: &str) -> Result<String, ToolError> {
    if !arguments.trim().is_empty() {
        let value: Value = serde_json::from_str(arguments)
            .map_err(|err| ToolError::invalid(format!("arguments must be json: {err}")))?;
        if value.as_object().is_some_and(|object| !object.is_empty()) {
            return Err(ToolError::invalid("current_time does not accept arguments"));
        }
    }
    Ok(json!({
        "utc": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_utc_timestamp() {
        let out = call("{}").unwrap();
        let value: Value = serde_json::from_str(&out).unwrap();
        let utc = value["utc"].as_str().unwrap();
        assert!(utc.contains('T'));
        assert!(utc.ends_with('Z'));
    }
}
