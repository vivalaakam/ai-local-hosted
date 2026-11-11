use alh_core::ToolCall;
use alh_tools::{
    CURRENT_TIME_SLUG, GET_WEATHER_SLUG, ToolExecutor, ToolKind, ToolSpec, WeatherEndpoints,
};
use axum::Router;
use axum::extract::Query;
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
async fn current_time_native_tool_returns_utc() {
    let executor = ToolExecutor::new().unwrap();
    let specs = vec![ToolSpec {
        id: Uuid::nil(),
        owner_user_id: None,
        slug: CURRENT_TIME_SLUG.into(),
        name: "Current time".into(),
        description: "now".into(),
        parameters: json!({"type": "object", "properties": {}}),
        kind: ToolKind::Native,
        source: None,
        enabled: true,
    }];
    let out = executor
        .execute_call(
            &specs,
            &ToolCall {
                index: 0,
                id: "call_1".into(),
                name: CURRENT_TIME_SLUG.into(),
                arguments: "{}".into(),
            },
        )
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&out).unwrap();
    assert!(value["utc"].as_str().unwrap().ends_with('Z'));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_weather_uses_http_endpoints() {
    let stub = weather_stub().await;
    let executor = ToolExecutor::new()
        .unwrap()
        .with_weather_endpoints(WeatherEndpoints {
            geocoding_base: stub.clone(),
            forecast_base: stub,
        });
    let specs = vec![ToolSpec {
        id: Uuid::nil(),
        owner_user_id: None,
        slug: GET_WEATHER_SLUG.into(),
        name: "Get weather".into(),
        description: "weather".into(),
        parameters: json!({"type":"object","properties":{"location":{"type":"string"}}}),
        kind: ToolKind::Native,
        source: None,
        enabled: true,
    }];
    let out = executor
        .execute_call(
            &specs,
            &ToolCall {
                index: 0,
                id: "call_1".into(),
                name: GET_WEATHER_SLUG.into(),
                arguments: r#"{"location":"Berlin"}"#.into(),
            },
        )
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["location"], "Berlin");
    assert_eq!(value["current_weather"]["temperature"], 21.5);
}

#[tokio::test(flavor = "multi_thread")]
async fn rhai_script_can_http_request() {
    let stub = echo_stub().await;
    let executor = ToolExecutor::new().unwrap();
    let source = format!(
        r#"
        fn run(args) {{
            let response = http_request("GET", "{stub}/ping");
            json_stringify(#{{
                "status": response.status,
                "body": response.body,
                "echo": args.msg
            }})
        }}
        "#
    );
    let specs = vec![ToolSpec {
        id: Uuid::new_v4(),
        owner_user_id: Some(Uuid::new_v4()),
        slug: "ping".into(),
        name: "Ping".into(),
        description: "ping".into(),
        parameters: json!({"type":"object","properties":{"msg":{"type":"string"}}}),
        kind: ToolKind::Rhai,
        source: Some(source),
        enabled: true,
    }];
    let out = executor
        .execute_call(
            &specs,
            &ToolCall {
                index: 0,
                id: "call_1".into(),
                name: "user__ping".into(),
                arguments: r#"{"msg":"hi"}"#.into(),
            },
        )
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["status"], 200);
    assert_eq!(value["body"], "pong");
    assert_eq!(value["echo"], "hi");
}

async fn weather_stub() -> String {
    let app = Router::new()
        .route("/v1/search", get(geocode))
        .route("/v1/forecast", get(forecast));
    bind(app).await
}

async fn echo_stub() -> String {
    let app = Router::new().route("/ping", get(|| async { "pong" }));
    bind(app).await
}

async fn bind(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}")
}

#[derive(Deserialize)]
struct GeoQuery {
    name: String,
}

async fn geocode(Query(query): Query<GeoQuery>) -> axum::Json<Value> {
    axum::Json(json!({
        "results": [{
            "name": query.name,
            "country": "DE",
            "latitude": 52.52,
            "longitude": 13.41
        }]
    }))
}

#[derive(Deserialize)]
struct ForecastQuery {
    latitude: f64,
    longitude: f64,
}

async fn forecast(Query(query): Query<ForecastQuery>) -> impl IntoResponse {
    assert!((query.latitude - 52.52).abs() < 0.01);
    assert!((query.longitude - 13.41).abs() < 0.01);
    axum::Json(json!({
        "current_weather": {"temperature": 21.5, "windspeed": 3.2}
    }))
}
