-- The models each connected provider offers, as last fetched, so the picker opens at once and
-- works offline. A provider's rows go with it when it is removed.
CREATE TABLE models (
    provider_id TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
    model_id    TEXT NOT NULL,
    name        TEXT NOT NULL,
    vision      INTEGER CHECK (vision IN (0, 1)),
    reasoning   INTEGER CHECK (reasoning IN (0, 1)),
    tools       INTEGER CHECK (tools IN (0, 1)),
    position    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (provider_id, model_id)
) STRICT, WITHOUT ROWID;

-- Seconds since the epoch of the last successful fetch.
ALTER TABLE providers ADD COLUMN models_fetched_at INTEGER;

-- A model is a provider and an id together. Before this an agent's model was a bare name with
-- no provider, which cannot be resolved to one, so it is cleared: the agent asks for a model.
ALTER TABLE agents ADD COLUMN model_provider TEXT;
UPDATE agents SET model = NULL;

-- "The provider agents run on" is gone: each agent has a model, and there is a default model.
DELETE FROM settings WHERE key = 'providers.active';
