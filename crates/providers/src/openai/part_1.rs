#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub id: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub header_timeout: Duration,
    pub idle_timeout: Duration,
}

#[derive(Clone)]
pub struct OpenAiProvider {
    id: String,
    base_url: String,
    api_key: Option<String>,
    header_timeout: Duration,
    idle_timeout: Duration,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let api_key = config.api_key.and_then(|key| {
            let key = key.trim().to_string();
            if key.is_empty() { None } else { Some(key) }
        });
        Self {
            id: config.id,
            base_url: config.base_url.trim().trim_end_matches('/').to_string(),
            api_key,
            header_timeout: config.header_timeout,
            idle_timeout: config.idle_timeout,
            client: shared_client(),
        }
    }

    pub async fn probe(&self, timeout: Duration) -> Result<(), ProviderError> {
        let response = self
            .send_with_timeout(self.client.get(self.models_url()), timeout)
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(status_error(response).await)
        }
    }

    fn models_url(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn authorized(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.api_key.as_deref() {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        }
    }

    async fn send_with_timeout(
        &self,
        builder: reqwest::RequestBuilder,
        timeout: Duration,
    ) -> Result<reqwest::Response, ProviderError> {
        let request = self.authorized(builder);
        match tokio::time::timeout(timeout, request.send()).await {
            Err(_) => Err(ProviderError::new(ProviderErrorKind::Timeout)),
            Ok(Err(err)) => Err(map_transport(err)),
            Ok(Ok(response)) => Ok(response),
        }
    }

    async fn list_models_inner(&self) -> Result<Vec<ModelDescriptor>, ProviderError> {
        let response = self
            .send_with_timeout(self.client.get(self.models_url()), self.header_timeout)
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        let value: Value = match tokio::time::timeout(self.header_timeout, response.json()).await {
            Err(_) => return Err(ProviderError::new(ProviderErrorKind::Timeout)),
            Ok(Err(err)) => {
                return Err(ProviderError::with_detail(
                    ProviderErrorKind::Decode,
                    err.to_string(),
                ));
            }
            Ok(Ok(value)) => value,
        };
        let data = value.get("data").and_then(Value::as_array).ok_or_else(|| {
            ProviderError::with_detail(ProviderErrorKind::Decode, "models response has no data")
        })?;
        let mut models = Vec::new();
        for item in data {
            let Some(name) = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let context = item
                .get("context_window")
                .or_else(|| item.get("context_length"))
                .or_else(|| item.get("max_model_len"))
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok());
            models.push(ModelDescriptor {
                name: name.to_string(),
                context_window: context,
                supports_tools: false,
            });
        }
        Ok(models)
    }
}

impl ChatProvider for OpenAiProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_models(&self) -> BoxFuture<Result<Vec<ModelDescriptor>, ProviderError>> {
        let provider = self.clone();
        Box::pin(async move { provider.list_models_inner().await })
    }

    fn stream(&self, request: CompletionRequest) -> CompletionStream {
        let provider = self.clone();
        Box::pin(futures::stream::try_unfold(
            Phase::Opening { provider, request },
            step,
        ))
    }
}

enum Phase {
    Opening {
        provider: OpenAiProvider,
        request: CompletionRequest,
    },
    Reading {
        parser: SseParser,
        body: Pin<Box<dyn futures::Stream<Item = Result<Vec<u8>, ProviderError>> + Send>>,
        idle: Duration,
        queue: VecDeque<CompletionEvent>,
    },
}

async fn step(phase: Phase) -> Result<Option<(CompletionEvent, Phase)>, ProviderError> {
    let mut phase = phase;
    loop {
        match phase {
            Phase::Opening { provider, request } => {
                let response = provider
                    .send_with_timeout(
                        provider
                            .client
                            .post(provider.completions_url())
                            .json(&openai_body(&request)),
                        provider.header_timeout,
                    )
                    .await?;
                if !response.status().is_success() {
                    return Err(status_error(response).await);
                }
                let idle = provider.idle_timeout;
                let body = response
                    .bytes_stream()
                    .map(|chunk| chunk.map(|bytes| bytes.to_vec()).map_err(map_transport));
                phase = Phase::Reading {
                    parser: SseParser::default(),
                    body: Box::pin(body),
                    idle,
                    queue: VecDeque::new(),
                };
            }
            Phase::Reading {
                mut parser,
                mut body,
                idle,
                mut queue,
            } => {
                if let Some(event) = queue.pop_front() {
                    return Ok(Some((
                        event,
                        Phase::Reading {
                            parser,
                            body,
                            idle,
                            queue,
                        },
                    )));
                }
                if parser.emitted_done {
                    return Ok(None);
                }
                match tokio::time::timeout(idle, body.next()).await {
                    Err(_) => return Err(ProviderError::new(ProviderErrorKind::Timeout)),
                    Ok(None) => {
                        queue.extend(parser.finish());
                        if queue.is_empty() {
                            return Ok(None);
                        }
                        phase = Phase::Reading {
                            parser,
                            body,
                            idle,
                            queue,
                        };
                    }
                    Ok(Some(Err(err))) => return Err(err),
                    Ok(Some(Ok(bytes))) => {
                        queue.extend(parser.push(&bytes)?);
                        phase = Phase::Reading {
                            parser,
                            body,
                            idle,
                            queue,
                        };
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
    pending_finish: Option<FinishReason>,
    usage: Option<alh_core::TokenUsage>,
    emitted_done: bool,
}

