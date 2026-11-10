CREATE TABLE providers (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key_env TEXT,
    default_model TEXT,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE UNIQUE INDEX providers_one_default_idx ON providers (is_default) WHERE is_default;

CREATE TABLE models (
    provider_id TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    context_window INTEGER,
    supports_tools BOOLEAN NOT NULL DEFAULT FALSE,
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider_id, name)
);

CREATE TABLE conversations (
    id UUID PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    provider_id TEXT NOT NULL REFERENCES providers (id),
    model TEXT NOT NULL DEFAULT '',
    system_prompt TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at TIMESTAMPTZ
);

CREATE INDEX conversations_updated_at_idx ON conversations (updated_at DESC);

CREATE TABLE messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    parts JSONB,
    tool_calls JSONB,
    tool_call_id TEXT,
    thinking TEXT,
    status TEXT NOT NULL,
    error_code TEXT,
    usage JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    UNIQUE (conversation_id, position)
);

CREATE INDEX messages_conversation_position_idx ON messages (conversation_id, position);

CREATE TABLE request_log (
    id UUID PRIMARY KEY,
    source TEXT NOT NULL,
    protocol TEXT NOT NULL,
    conversation_id UUID,
    message_id UUID,
    provider_id TEXT NOT NULL,
    client_model TEXT NOT NULL,
    model TEXT NOT NULL,
    status TEXT NOT NULL,
    error_code TEXT,
    latency_ms INTEGER,
    prompt_tokens INTEGER,
    completion_tokens INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX request_log_created_at_idx ON request_log (created_at DESC);

CREATE TABLE response_states (
    id TEXT PRIMARY KEY,
    previous_id TEXT,
    client_model TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    upstream_model TEXT NOT NULL,
    messages JSONB NOT NULL,
    output JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX response_states_created_at_idx ON response_states (created_at);
