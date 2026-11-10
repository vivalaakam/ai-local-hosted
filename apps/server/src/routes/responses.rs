async fn responses(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let previous_id = body
        .get("previous_response_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let previous = if let Some(id) = &previous_id {
        match state.store.get_response_state(user, id).await {
            Ok(Some(stored)) => Some(stored.messages),
            Ok(None) => return ApiError::not_found("unknown previous_response_id").response(),
            Err(err) => return ApiError::database(err).response(),
        }
    } else {
        None
    };
    let parsed = match parse_responses(&body, previous.as_deref()) {
        Ok(parsed) => parsed,
        Err(err) => return ApiError::invalid(err.to_string()).response(),
    };
    let requested = apply_provider_header(
        &parsed.incoming.client_model,
        header_str(&headers, "x-provider"),
    );
    let route = match resolve_route(&state, user, &requested).await {
        Ok(route) => route,
        Err(err) => return err.response(),
    };
    let mut request = parsed.incoming.request;
    let transcript = request.messages.clone();
    if let Err(err) = prepend_server_prompts(&state, user, &mut request).await {
        return err.response();
    }
    request.model = route.upstream_model.clone();
    let response_id = new_id("resp");
    let meta = log_meta(user, "agent", "responses", &route, None, None);
    let mut stream = route.provider.stream(request);
    if !parsed.incoming.stream {
        return match read_until_done(stream).await {
            Ok(accum) => {
                let body = replay_responses(&response_id, &route.client_model, &accum);
                save_response(
                    &state.store,
                    &response_id,
                    previous_id,
                    &meta,
                    transcript,
                    &accum,
                    body.get("output").cloned().unwrap_or(Value::Null),
                )
                .await;
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
                ApiError::from_provider(err).response()
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
            return ApiError::from_provider(err).response();
        }
    };
    let (tx, rx) = channel();
    let store = state.store.clone();
    let model = route.client_model.clone();
    let id = response_id.clone();
    tokio::spawn(async move {
        let mut encoder = ResponsesEncoder::new(id.clone(), model);
        if !forward(&tx, vec![encoder.created_frame()]).await {
            save_response(
                &store,
                &id,
                previous_id,
                &meta,
                transcript,
                &alh_core::CompletionAccum::default(),
                Value::Null,
            )
            .await;
            write_log(&store, &meta, "cancelled", None, None).await;
            return;
        }
        let outcome = drive(first, stream, &tx, |event| encoder.push(event)).await;
        finish_encoder(&tx, &outcome, |event| encoder.push(event)).await;
        let output = encoder.output_items();
        let status = status_of(&outcome);
        save_response(
            &store,
            &id,
            previous_id,
            &meta,
            transcript,
            &outcome.accum,
            output,
        )
        .await;
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

