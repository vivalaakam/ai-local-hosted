CREATE TABLE system_prompts (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    scope TEXT NOT NULL CHECK (scope IN ('global', 'assigned', 'self')),
    content TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, scope),
    CHECK (
        (user_id = '00000000-0000-0000-0000-000000000001' AND scope = 'global')
        OR (user_id <> '00000000-0000-0000-0000-000000000001' AND scope IN ('assigned', 'self'))
    )
);
