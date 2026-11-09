-- Delegation. A team member says when it should be used, so its lead can choose it. A run an agent
-- handed to a team member points at the run that handed it over.
ALTER TABLE agents ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE runs ADD COLUMN parent_run_id TEXT;
CREATE INDEX runs_by_parent ON runs (parent_run_id);
