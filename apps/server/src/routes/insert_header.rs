fn insert_header(response: &mut Response, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response
            .headers_mut()
            .insert(axum::http::header::HeaderName::from_static(name), value);
    }
}

