-- A workflow run waiting for you (or for a long delay) keeps going after the app closes. The run
-- itself stays 'running' in workflow_runs, so no table has to be rebuilt; the run is waiting if it
-- has a row here. The row holds where it stopped (the snapshot), what it asked, when a delay is
-- over, and the steps it was running, so editing the workflow meanwhile does not change this run.
CREATE TABLE workflow_waits (
    run_id     TEXT PRIMARY KEY REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step       TEXT NOT NULL,
    prompt     TEXT NOT NULL,
    until      INTEGER,
    snapshot   TEXT NOT NULL,
    definition TEXT NOT NULL
) STRICT;
