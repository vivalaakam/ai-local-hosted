ALTER TABLE models DROP CONSTRAINT IF EXISTS models_provider_fk;
ALTER TABLE conversations DROP CONSTRAINT IF EXISTS conversations_provider_fk;

ALTER TABLE providers ADD COLUMN id_uuid UUID;
UPDATE providers SET id_uuid = gen_random_uuid() WHERE id_uuid IS NULL;
ALTER TABLE providers ALTER COLUMN id_uuid SET NOT NULL;

ALTER TABLE models ADD COLUMN provider_uuid UUID;
UPDATE models AS model
SET provider_uuid = provider.id_uuid
FROM providers AS provider
WHERE model.user_id = provider.user_id
  AND model.provider_id = provider.id;

ALTER TABLE conversations ADD COLUMN provider_uuid UUID;
UPDATE conversations AS conversation
SET provider_uuid = provider.id_uuid
FROM providers AS provider
WHERE conversation.user_id = provider.user_id
  AND conversation.provider_id = provider.id;

ALTER TABLE request_log ADD COLUMN provider_uuid UUID;
UPDATE request_log AS entry
SET provider_uuid = provider.id_uuid
FROM providers AS provider
WHERE entry.user_id = provider.user_id
  AND entry.provider_id = provider.id;

ALTER TABLE response_states ADD COLUMN provider_uuid UUID;
UPDATE response_states AS state
SET provider_uuid = provider.id_uuid
FROM providers AS provider
WHERE state.user_id = provider.user_id
  AND state.provider_id = provider.id;

DELETE FROM models WHERE provider_uuid IS NULL;
DELETE FROM conversations WHERE provider_uuid IS NULL;
DELETE FROM request_log WHERE provider_uuid IS NULL;
DELETE FROM response_states WHERE provider_uuid IS NULL;

ALTER TABLE models DROP CONSTRAINT IF EXISTS models_pkey;
ALTER TABLE models DROP COLUMN provider_id;
ALTER TABLE models RENAME COLUMN provider_uuid TO provider_id;
ALTER TABLE models ALTER COLUMN provider_id SET NOT NULL;
ALTER TABLE models ADD PRIMARY KEY (user_id, provider_id, name);

ALTER TABLE conversations DROP COLUMN provider_id;
ALTER TABLE conversations RENAME COLUMN provider_uuid TO provider_id;
ALTER TABLE conversations ALTER COLUMN provider_id SET NOT NULL;

ALTER TABLE request_log DROP COLUMN provider_id;
ALTER TABLE request_log RENAME COLUMN provider_uuid TO provider_id;
ALTER TABLE request_log ALTER COLUMN provider_id SET NOT NULL;

ALTER TABLE response_states DROP COLUMN provider_id;
ALTER TABLE response_states RENAME COLUMN provider_uuid TO provider_id;
ALTER TABLE response_states ALTER COLUMN provider_id SET NOT NULL;

ALTER TABLE providers DROP CONSTRAINT IF EXISTS providers_pkey;
ALTER TABLE providers DROP COLUMN id;
ALTER TABLE providers RENAME COLUMN id_uuid TO id;
ALTER TABLE providers ADD PRIMARY KEY (user_id, id);

ALTER TABLE models
    ADD CONSTRAINT models_provider_fk
    FOREIGN KEY (user_id, provider_id)
    REFERENCES providers (user_id, id)
    ON DELETE CASCADE;

ALTER TABLE conversations
    ADD CONSTRAINT conversations_provider_fk
    FOREIGN KEY (user_id, provider_id)
    REFERENCES providers (user_id, id);
