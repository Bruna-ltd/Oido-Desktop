-- Schedules: an agent's prompt or one of its workflows, fired on a timing while the app is open.
-- A schedule goes with its agent, and with its workflow. Each occurrence is a fire; a fire keeps
-- the chat it opened (and the workflow run it made) until retention deletes it. A draft is a
-- schedule an agent proposed with its `schedule` tool, waiting in its chat for approval.
CREATE TABLE schedules (
    id          TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    -- NULL: the schedule sends `prompt`; otherwise it runs this workflow with `inputs`.
    workflow_id TEXT REFERENCES workflows(id) ON DELETE CASCADE,
    name        TEXT NOT NULL CHECK (length(trim(name)) > 0),
    prompt      TEXT NOT NULL DEFAULT '',
    inputs      TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(inputs)),
    -- {"once": <unix>} | {"every": {"start": <unix>, "secs": <n>}} | {"cron": "<expr>"}
    timing      TEXT NOT NULL CHECK (json_valid(timing)),
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    -- The next fire; NULL once a "once" schedule has fired.
    next_at     INTEGER,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    CHECK ((workflow_id IS NULL) = (length(trim(prompt)) > 0))
) STRICT;

CREATE INDEX schedules_by_agent ON schedules (agent_id);
CREATE INDEX schedules_by_workflow ON schedules (workflow_id);

CREATE TABLE schedule_fires (
    id              TEXT PRIMARY KEY,
    schedule_id     TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    due_at          INTEGER NOT NULL,
    started_at      INTEGER NOT NULL,
    finished_at     INTEGER,
    status          TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'skipped')),
    note            TEXT NOT NULL DEFAULT '',
    session_id      TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    workflow_run_id TEXT REFERENCES workflow_runs(id) ON DELETE SET NULL
) STRICT;

CREATE INDEX schedule_fires_by_schedule ON schedule_fires (schedule_id, started_at DESC);
CREATE INDEX schedule_fires_by_session ON schedule_fires (session_id);

-- `schedule_id` is set when the draft changes a saved schedule; `body` is the whole schedule.
CREATE TABLE schedule_drafts (
    call_id     TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    schedule_id TEXT REFERENCES schedules(id) ON DELETE CASCADE,
    body        TEXT NOT NULL CHECK (json_valid(body))
) STRICT;

CREATE INDEX schedule_drafts_by_session ON schedule_drafts (session_id);
