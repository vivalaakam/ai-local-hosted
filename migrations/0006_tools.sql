CREATE TABLE tools (
    id UUID PRIMARY KEY,
    owner_user_id UUID REFERENCES users (id) ON DELETE CASCADE,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    parameters JSONB NOT NULL DEFAULT '{}'::jsonb,
    kind TEXT NOT NULL CHECK (kind IN ('native', 'rhai')),
    source TEXT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (
        (kind = 'native' AND source IS NULL)
        OR (kind = 'rhai')
    ),
    CHECK (
        (kind = 'native' AND owner_user_id IS NULL)
        OR (kind = 'rhai')
    )
);

CREATE UNIQUE INDEX tools_system_slug_uidx
    ON tools (slug)
    WHERE owner_user_id IS NULL;

CREATE UNIQUE INDEX tools_user_slug_uidx
    ON tools (owner_user_id, slug)
    WHERE owner_user_id IS NOT NULL;

CREATE INDEX tools_owner_idx ON tools (owner_user_id);

INSERT INTO tools (id, owner_user_id, slug, name, description, parameters, kind, source, enabled)
VALUES
    (
        '00000000-0000-0000-0000-000000001001',
        NULL,
        'current_time',
        'Current time',
        'Returns the current UTC time as an ISO-8601 string.',
        '{"type":"object","properties":{},"additionalProperties":false}'::jsonb,
        'native',
        NULL,
        TRUE
    ),
    (
        '00000000-0000-0000-0000-000000001002',
        NULL,
        'get_weather',
        'Get weather',
        'Fetches current weather for a place name via Open-Meteo.',
        '{"type":"object","properties":{"location":{"type":"string","description":"City or place name"}},"required":["location"],"additionalProperties":false}'::jsonb,
        'native',
        NULL,
        TRUE
    );
