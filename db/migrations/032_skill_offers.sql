-- Skills from runs (docs/skills-from-runs.md, ADR-7): a run judged worth a skill is offered in its
-- chat until you answer the offer (Not now, or saving a skill from it).
ALTER TABLE runs ADD COLUMN offer_closed INTEGER NOT NULL DEFAULT 0;
