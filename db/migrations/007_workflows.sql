-- Workflows: something an agent did once, saved as steps that can be run again. A workflow belongs
-- to its agent and goes with it. A draft is one just learned from a chat and not saved yet; there
-- is at most one per chat. Runs are kept with their workflow.
CREATE TABLE workflows (
    id         TEXT PRIMARY KEY,
    agent_id   TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    name       TEXT NOT NULL CHECK (length(trim(name)) > 0),
    goal       TEXT NOT NULL DEFAULT '',
    definition TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX workflows_by_agent ON workflows (agent_id);

CREATE TABLE workflow_drafts (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    agent_id   TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    goal       TEXT NOT NULL DEFAULT '',
    definition TEXT NOT NULL
) STRICT;

CREATE TABLE workflow_runs (
    id            TEXT PRIMARY KEY,
    workflow_id   TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    started_by    TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'stopped')),
    inputs        TEXT NOT NULL DEFAULT '{}',
    output        TEXT NOT NULL DEFAULT '',
    failed_step   TEXT NOT NULL DEFAULT '',
    error         TEXT NOT NULL DEFAULT '',
    steps         TEXT NOT NULL DEFAULT '[]',
    input_tokens  INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    started_at    INTEGER NOT NULL,
    finished_at   INTEGER
) STRICT;

CREATE INDEX workflow_runs_by_workflow ON workflow_runs (workflow_id, started_at DESC);
