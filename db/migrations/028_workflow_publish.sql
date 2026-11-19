-- A workflow's `definition` is its draft: what the editor changes and a test run uses. `published`
-- is the copy that is live: it is what an agent, a sub-flow call or a schedule runs. NULL means
-- never published (or unpublished): nothing runs it for real. Workflows saved before this was
-- added go live as they are, so nothing an agent could run stops working.
ALTER TABLE workflows ADD COLUMN published TEXT CHECK (published IS NULL OR json_valid(published));
ALTER TABLE workflows ADD COLUMN published_at INTEGER;
UPDATE workflows SET published = definition, published_at = unixepoch();

-- A schedule a workflow's trigger made when it was published. It goes when the workflow is
-- unpublished, and is only changed by publishing again.
ALTER TABLE schedules ADD COLUMN from_trigger INTEGER NOT NULL DEFAULT 0 CHECK (from_trigger IN (0, 1));
