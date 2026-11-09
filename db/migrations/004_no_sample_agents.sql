-- Earlier builds put six made-up agents (with made-up chats) in every new database. A new install
-- now starts with none, and these are removed from existing ones. They have fixed ids; agents you
-- made have random ones and are left alone. Their chats and messages go with them (cascade).
DELETE FROM settings
 WHERE key = 'agents.selected'
   AND json_extract(value, '$.id') IN ('atlas', 'forge', 'scout', 'ledger', 'fact-checker', 'reviewer');

DELETE FROM agents
 WHERE id IN ('atlas', 'forge', 'scout', 'ledger', 'fact-checker', 'reviewer');
