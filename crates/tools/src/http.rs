use std::collections::HashMap;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::error::ToolError;

pub const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
pub const HTTP_MAX_BODY: usize = 1024 * 1024;

#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Result<Self, ToolError> {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|err| ToolError::internal(format!("http client: {err}")))?;
        Ok(Self { client })
    }

    pub async fn request(
        &self,
        method: &str,
        url: &str,
        headers: Option<&Value>,
        body: Option<&str>,
    ) -> Result<Value, ToolError> {
        let method = method.trim().to_ascii_uppercase();
        if method.is_empty() {
            return Err(ToolError::invalid("http method is required"));
        }
        let url = url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ToolError::invalid("only http and https urls are allowed"));
        }
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| ToolError::invalid(format!("unsupported http method {method}")))?;
        let mut builder = self.client.request(method, url);
        if let Some(headers) = headers {
            builder = builder.headers(parse_headers(headers)?);
        }
        if let Some(body) = body {
            builder = builder.body(body.to_string());
        }
        let response = builder
            .send()
            .await
            .map_err(|err| ToolError::http(err.to_string()))?;
        let status = response.status().as_u16();
        let mut response_headers = HashMap::new();
        for (name, value) in response.headers() {
            if let Ok(text) = value.to_str() {
                response_headers.insert(name.as_str().to_string(), text.to_string());
            }
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|err| ToolError::http(err.to_string()))?;
        if bytes.len() > HTTP_MAX_BODY {
            return Err(ToolError::http(format!(
                "response body exceeds {HTTP_MAX_BODY} bytes"
            )));
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        Ok(json!({
            "status": status,
            "headers": response_headers,
            "body": body,
        }))
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new().expect("default http client")
    }
}

fn parse_headers(value: &Value) -> Result<HeaderMap, ToolError> {
    let Some(object) = value.as_object() else {
        return Err(ToolError::invalid("headers must be an object"));
    };
    let mut headers = HeaderMap::new();
    for (key, value) in object {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| ToolError::invalid(format!("invalid header name {key}")))?;
        let Some(text) = value.as_str() else {
            return Err(ToolError::invalid(format!(
                "header {key} value must be a string"
            )));
        };
        let value = HeaderValue::from_str(text)
            .map_err(|_| ToolError::invalid(format!("invalid header value for {key}")))?;
        headers.insert(name, value);
    }
    Ok(headers)
}
