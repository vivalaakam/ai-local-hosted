async fn messages(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let parsed = match parse_anthropic_message(&body) {
        Ok(parsed) => parsed,
        Err(err) => return ApiError::invalid(err.to_string()).anthropic().response(),
    };
    let requested = apply_provider_header(&parsed.client_model, header_str(&headers, "x-provider"));
    let route = match resolve_route(&state, user, &requested).await {
        Ok(route) => route,
        Err(err) => return err.anthropic().response(),
    };
    let mut request = parsed.request;
    if let Err(err) = prepend_server_prompts(&state, user, &mut request).await {
        return err.anthropic().response();
    }
    request.model = route.upstream_model.clone();
    let meta = log_meta(user, "agent", "anthropic_messages", &route, None, None);
    let mut stream = route.provider.stream(request);
    if !parsed.stream {
        return match read_until_done(stream).await {
            Ok(accum) => {
                let body = replay_anthropic(&route.client_model, &accum);
                write_log(&state.store, &meta, "done", None, accum.usage).await;
                Json(body).into_response()
            }
            Err(err) => {
                write_log(
                    &state.store,
                    &meta,
                    "error",
                    Some(provider_code(&err)),
                    None,
                )
                .await;
                ApiError::from_provider(err).anthropic().response()
            }
        };
    }
    let first = match next_event(&mut stream).await {
        Ok(first) => first,
        Err(err) => {
            write_log(
                &state.store,
                &meta,
                "error",
                Some(provider_code(&err)),
                None,
            )
            .await;
            return ApiError::from_provider(err).anthropic().response();
        }
    };
    let (tx, rx) = channel();
    let store = state.store.clone();
    let model = route.client_model.clone();
    tokio::spawn(async move {
        let mut encoder = AnthropicEncoder::new(new_id("msg"), model);
        let outcome = drive(first, stream, &tx, |event| encoder.push(event)).await;
        finish_encoder(&tx, &outcome, |event| encoder.push(event)).await;
        let status = status_of(&outcome);
        write_log(
            &store,
            &meta,
            status,
            outcome.error_code.as_deref(),
            outcome.accum.usage,
        )
        .await;
    });
    Sse::new(rx).into_response()
}

