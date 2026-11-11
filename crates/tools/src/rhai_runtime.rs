use std::sync::Arc;

use rhai::{Dynamic, Engine, Map, Scope};
use serde_json::Value;
use tokio::runtime::Handle;

use crate::error::ToolError;
use crate::http::HttpClient;

pub fn run_rhai(source: &str, arguments_json: &str, http: HttpClient) -> Result<String, ToolError> {
    let args_value: Value = if arguments_json.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(arguments_json)
            .map_err(|err| ToolError::invalid(format!("arguments must be json: {err}")))?
    };
    let handle = Handle::current();
    let http = Arc::new(http);

    let mut engine = Engine::new();
    engine.set_max_expr_depths(64, 64);
    engine.set_max_operations(100_000);
    engine.set_max_string_size(1_048_576);
    engine.set_max_array_size(10_000);
    engine.set_max_map_size(10_000);

    register_http(&mut engine, Arc::clone(&http), handle.clone());
    engine.register_fn(
        "json_parse",
        |text: &str| -> Result<Dynamic, Box<rhai::EvalAltResult>> {
            let value: Value =
                serde_json::from_str(text).map_err(|err| eval_err(format!("json_parse: {err}")))?;
            Ok(json_to_dynamic(&value))
        },
    );
    engine.register_fn(
        "json_stringify",
        |value: Dynamic| -> Result<String, Box<rhai::EvalAltResult>> {
            let json = dynamic_to_json(&value).map_err(eval_err)?;
            Ok(json.to_string())
        },
    );

    let ast = engine
        .compile(source)
        .map_err(|err| ToolError::script(format!("compile: {err}")))?;
    let mut scope = Scope::new();
    let args = json_to_dynamic(&args_value);

    let result = engine
        .call_fn::<Dynamic>(&mut scope, &ast, "run", (args,))
        .map_err(|err| ToolError::script(format!("runtime: {err}")))?;

    let json = dynamic_to_json(&result).map_err(ToolError::script)?;
    Ok(match json {
        Value::String(text) => text,
        other => other.to_string(),
    })
}

fn register_http(engine: &mut Engine, http: Arc<HttpClient>, handle: Handle) {
    {
        let http = Arc::clone(&http);
        let handle = handle.clone();
        engine.register_fn(
            "http_request",
            move |method: &str, url: &str| -> Result<Dynamic, Box<rhai::EvalAltResult>> {
                let http = Arc::clone(&http);
                let method = method.to_string();
                let url = url.to_string();
                handle
                    .block_on(async move { http.request(&method, &url, None, None).await })
                    .map(|value| json_to_dynamic(&value))
                    .map_err(|err| eval_err(err.to_string()))
            },
        );
    }
    {
        let http = Arc::clone(&http);
        let handle = handle.clone();
        engine.register_fn(
            "http_request",
            move |method: &str,
                  url: &str,
                  headers: Dynamic,
                  body: Dynamic|
                  -> Result<Dynamic, Box<rhai::EvalAltResult>> {
                let headers_json = dynamic_to_json(&headers).map_err(eval_err)?;
                let body_text = if body.is_unit() {
                    None
                } else if body.is_string() {
                    let text = body.to_string();
                    if text.is_empty() { None } else { Some(text) }
                } else {
                    Some(dynamic_to_json(&body).map_err(eval_err)?.to_string())
                };
                let headers_ref = if headers_json.is_null()
                    || headers_json
                        .as_object()
                        .is_some_and(serde_json::Map::is_empty)
                {
                    None
                } else {
                    Some(headers_json)
                };
                let http = Arc::clone(&http);
                let method = method.to_string();
                let url = url.to_string();
                handle
                    .block_on(async move {
                        http.request(&method, &url, headers_ref.as_ref(), body_text.as_deref())
                            .await
                    })
                    .map(|value| json_to_dynamic(&value))
                    .map_err(|err| eval_err(err.to_string()))
            },
        );
    }
}

fn eval_err(message: impl Into<String>) -> Box<rhai::EvalAltResult> {
    Box::new(rhai::EvalAltResult::ErrorRuntime(
        message.into().into(),
        rhai::Position::NONE,
    ))
}

fn json_to_dynamic(value: &Value) -> Dynamic {
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(flag) => Dynamic::from(*flag),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Dynamic::from(value)
            } else if let Some(value) = number.as_u64() {
                Dynamic::from(value as i64)
            } else if let Some(value) = number.as_f64() {
                Dynamic::from(value)
            } else {
                Dynamic::from(number.to_string())
            }
        }
        Value::String(text) => Dynamic::from(text.clone()),
        Value::Array(items) => {
            let list: rhai::Array = items.iter().map(json_to_dynamic).collect();
            Dynamic::from(list)
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                out.insert(key.clone().into(), json_to_dynamic(value));
            }
            Dynamic::from(out)
        }
    }
}

fn dynamic_to_json(value: &Dynamic) -> Result<Value, String> {
    if value.is_unit() {
        return Ok(Value::Null);
    }
    if let Some(flag) = value.clone().try_cast::<bool>() {
        return Ok(Value::Bool(flag));
    }
    if let Some(number) = value.clone().try_cast::<i64>() {
        return Ok(Value::Number(number.into()));
    }
    if let Some(number) = value.clone().try_cast::<f64>() {
        return serde_json::Number::from_f64(number)
            .map(Value::Number)
            .ok_or_else(|| "non-finite number".to_string());
    }
    if let Some(text) = value.clone().try_cast::<String>() {
        return Ok(Value::String(text));
    }
    if let Some(list) = value.clone().try_cast::<rhai::Array>() {
        let items = list
            .iter()
            .map(dynamic_to_json)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Value::Array(items));
    }
    if let Some(map) = value.clone().try_cast::<Map>() {
        let mut object = serde_json::Map::new();
        for (key, item) in map {
            object.insert(key.to_string(), dynamic_to_json(&item)?);
        }
        return Ok(Value::Object(object));
    }
    Ok(Value::String(value.to_string()))
}
