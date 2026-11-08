-- A provider's API key, sealed by the app (src/vault.rs): never readable text in this file, and the
-- key that unlocks it is not here either (it lives in the OS keyring).
ALTER TABLE providers ADD COLUMN api_key_enc BLOB;

-- A connected built-in provider has a key. Before this migration none could, so any marked
-- connected was sample data, not a real connection: clear it, and what pointed at it.
UPDATE providers SET connected = 0, key_hint = NULL WHERE custom = 0 AND connected = 1;
DELETE FROM providers WHERE custom = 1 AND base_url = 'https://llama.example.net/v1';
UPDATE settings SET value = 'null' WHERE key = 'providers.active';
