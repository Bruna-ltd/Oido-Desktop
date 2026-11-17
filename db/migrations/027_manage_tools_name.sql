-- A development build of 026 gave agents the MCP-managing tool as 'mcp', but the tool is called
-- 'mcp_servers'. 'mcp' names no tool (an MCP server's tools are 'mcp__server__tool'), so it
-- becomes 'mcp_servers', once. A database 026 ran on correctly has no 'mcp' and is unchanged.
UPDATE agents SET tools = (
    SELECT json_group_array(value) FROM (
        SELECT CASE WHEN value = 'mcp' THEN 'mcp_servers' ELSE value END AS value
        FROM json_each(agents.tools) ORDER BY key
    )
)
WHERE EXISTS (SELECT 1 FROM json_each(agents.tools) WHERE value = 'mcp')
  AND NOT EXISTS (SELECT 1 FROM json_each(agents.tools) WHERE value = 'mcp_servers');
UPDATE agents SET tools = (
    SELECT json_group_array(value) FROM (
        SELECT value FROM json_each(agents.tools) WHERE value <> 'mcp' ORDER BY key
    )
)
WHERE EXISTS (SELECT 1 FROM json_each(agents.tools) WHERE value = 'mcp');
