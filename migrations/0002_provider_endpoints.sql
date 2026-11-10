ALTER TABLE providers ADD COLUMN protocol TEXT;
ALTER TABLE providers ADD COLUMN url TEXT;
ALTER TABLE providers ADD COLUMN token TEXT;

UPDATE providers
SET protocol = CASE
        WHEN kind IN ('anthropic', 'antropic') THEN 'anthropic'
        ELSE 'openai'
    END,
    url = base_url
WHERE protocol IS NULL;

ALTER TABLE providers ALTER COLUMN protocol SET NOT NULL;
ALTER TABLE providers ALTER COLUMN url SET NOT NULL;
ALTER TABLE providers DROP COLUMN kind;
ALTER TABLE providers DROP COLUMN base_url;
ALTER TABLE providers DROP COLUMN api_key_env;
