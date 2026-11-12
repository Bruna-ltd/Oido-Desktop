-- How a provider is asked: with its own API key, or with a Claude Pro/Max subscription. Everything
-- saved before this was a key, which is the default.
ALTER TABLE providers ADD COLUMN auth TEXT NOT NULL DEFAULT 'api_key';
