-- Workflows of other agents that an agent may run too (docs/workflow-builder.md, ADR-11): a JSON
-- list of workflow ids. Its own workflows are not listed; a workflow still belongs to one agent.
ALTER TABLE agents ADD COLUMN workflows TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(workflows));
