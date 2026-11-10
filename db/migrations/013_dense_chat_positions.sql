-- An agent's chats sit at positions 0..n in the order they show. Tabs come back by position and
-- the open one is remembered by its index, so the gaps a closed chat used to leave (and the ties a
-- later chat made with them) reordered tabs and opened the wrong one. Close them, keeping the
-- order they load in.
UPDATE sessions SET position = (
    SELECT o.n FROM (
        SELECT id, ROW_NUMBER() OVER (PARTITION BY agent_id ORDER BY position, rowid) - 1 AS n
        FROM sessions
    ) AS o
    WHERE o.id = sessions.id
);
