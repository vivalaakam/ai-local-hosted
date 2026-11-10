CREATE TABLE users (
    id UUID PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_tokens (
    token_hash TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX oauth_tokens_user_idx ON oauth_tokens (user_id);

INSERT INTO users (id, username, password_hash)
VALUES ('00000000-0000-0000-0000-000000000001', 'local', '!')
ON CONFLICT (id) DO NOTHING;

ALTER TABLE providers ADD COLUMN user_id UUID REFERENCES users (id);
ALTER TABLE conversations ADD COLUMN user_id UUID REFERENCES users (id);
ALTER TABLE models ADD COLUMN user_id UUID REFERENCES users (id);
ALTER TABLE request_log ADD COLUMN user_id UUID REFERENCES users (id);
ALTER TABLE response_states ADD COLUMN user_id UUID REFERENCES users (id);

UPDATE providers
SET user_id = '00000000-0000-0000-0000-000000000001'
WHERE user_id IS NULL;

UPDATE conversations AS conversation
SET user_id = provider.user_id
FROM providers AS provider
WHERE conversation.provider_id = provider.id
  AND conversation.user_id IS NULL;

UPDATE models AS model
SET user_id = provider.user_id
FROM providers AS provider
WHERE model.provider_id = provider.id
  AND model.user_id IS NULL;

UPDATE request_log AS entry
SET user_id = provider.user_id
FROM providers AS provider
WHERE entry.provider_id = provider.id
  AND entry.user_id IS NULL;

UPDATE response_states AS state
SET user_id = provider.user_id
FROM providers AS provider
WHERE state.provider_id = provider.id
  AND state.user_id IS NULL;

DELETE FROM models WHERE user_id IS NULL;
DELETE FROM request_log WHERE user_id IS NULL;
DELETE FROM response_states WHERE user_id IS NULL;

ALTER TABLE providers ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE conversations ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE models ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE request_log ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE response_states ALTER COLUMN user_id SET NOT NULL;

ALTER TABLE models DROP CONSTRAINT IF EXISTS models_provider_id_fkey;
ALTER TABLE conversations DROP CONSTRAINT IF EXISTS conversations_provider_id_fkey;
ALTER TABLE providers DROP CONSTRAINT IF EXISTS providers_pkey;
DROP INDEX IF EXISTS providers_one_default_idx;

ALTER TABLE providers ADD PRIMARY KEY (user_id, id);
ALTER TABLE models DROP CONSTRAINT models_pkey;
ALTER TABLE models ADD PRIMARY KEY (user_id, provider_id, name);

ALTER TABLE models
    ADD CONSTRAINT models_provider_fk
    FOREIGN KEY (user_id, provider_id)
    REFERENCES providers (user_id, id)
    ON DELETE CASCADE;

ALTER TABLE conversations
    ADD CONSTRAINT conversations_provider_fk
    FOREIGN KEY (user_id, provider_id)
    REFERENCES providers (user_id, id);

ALTER TABLE conversations
    ADD CONSTRAINT conversations_user_fk
    FOREIGN KEY (user_id) REFERENCES users (id);

CREATE UNIQUE INDEX providers_one_default_per_user_idx
    ON providers (user_id) WHERE is_default;

CREATE INDEX conversations_user_updated_idx
    ON conversations (user_id, updated_at DESC);
