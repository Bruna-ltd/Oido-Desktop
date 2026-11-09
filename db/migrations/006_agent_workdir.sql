-- The folder an agent's commands run in. NULL is the home folder.
ALTER TABLE agents ADD COLUMN workdir TEXT;
