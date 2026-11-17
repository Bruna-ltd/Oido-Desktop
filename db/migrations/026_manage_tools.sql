-- The tools that manage the app (schedule, mcp_servers, agents) stopped being every agent's: an
-- agent now has them only when they are chosen. Agents made before keep them, so nothing they did stops
-- working; they can be taken off in the agent's settings. Team members never had them.
UPDATE agents SET tools = json_insert(tools, '$[#]', 'schedule')
WHERE parent_id IS NULL AND NOT EXISTS (SELECT 1 FROM json_each(agents.tools) WHERE value = 'schedule');
UPDATE agents SET tools = json_insert(tools, '$[#]', 'mcp_servers')
WHERE parent_id IS NULL AND NOT EXISTS (SELECT 1 FROM json_each(agents.tools) WHERE value = 'mcp_servers');
UPDATE agents SET tools = json_insert(tools, '$[#]', 'agents')
WHERE parent_id IS NULL AND NOT EXISTS (SELECT 1 FROM json_each(agents.tools) WHERE value = 'agents');
