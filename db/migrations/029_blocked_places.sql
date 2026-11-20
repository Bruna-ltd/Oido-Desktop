-- Blocked places (docs/guardrails.md): folders and files an agent's tools may never reach, as a
-- JSON list of {path, keep}, beside the list that holds for every agent (the `guard.blocked` setting).
ALTER TABLE agents ADD COLUMN blocked TEXT NOT NULL DEFAULT '[]';
