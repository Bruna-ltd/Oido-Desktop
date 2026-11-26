-- Skills from runs (docs/skills-from-runs.md, ADR-6): TypeSafe's verdict on whether a finished run
-- holds something worth making a skill of. NULL: not scored; 1: worth it; 0: not.
ALTER TABLE runs ADD COLUMN worth INTEGER;
