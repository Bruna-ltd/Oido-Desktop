-- Skills from runs (docs/skills-from-runs.md, ADR-10): TypeSafe's answers about a finished run, as a
-- JSON list of probabilities in the order asked, so the Runs screen can show why it was or was not
-- offered. NULL: not judged.
ALTER TABLE runs ADD COLUMN scores TEXT;
