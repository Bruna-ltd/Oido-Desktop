-- An agent's own approvals (docs/guardrails.md): tools it may use only with your go-ahead, and
-- where that is asked for, as JSON {on, to, tools}.
ALTER TABLE agents ADD COLUMN approvals TEXT NOT NULL DEFAULT '{}';
