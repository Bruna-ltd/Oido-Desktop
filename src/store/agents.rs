//! Agents with their team members, chats and messages.

use std::collections::HashMap;

use super::{Result, Store};
use crate::agents::Agent;
use crate::model::{AgentStatus, Message, Session, chat};
use crate::models::ModelRef;

impl Store {
    /// Every agent with its team, chats and messages, in their saved order. Which agents are
    /// running is not stored, so all load idle.
    pub fn load_agents(&self) -> Result<Vec<Agent>> {
        let mut messages: HashMap<String, Vec<Message>> = HashMap::new();
        {
            let mut stmt = self.conn.prepare_cached(self.sql("ListMessages")?)?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (session, body) = row?;
                messages
                    .entry(session)
                    .or_default()
                    .push(serde_json::from_str(&body)?);
            }
        }
        let mut sessions: HashMap<String, Vec<Session>> = HashMap::new();
        {
            let mut stmt = self.conn.prepare_cached(self.sql("ListSessions")?)?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (agent, id, title) = row?;
                let msgs = messages.remove(&id).unwrap_or_default();
                sessions.entry(agent).or_default().push(Session {
                    id,
                    title,
                    messages: msgs,
                });
            }
        }
        let mut top: Vec<Agent> = Vec::new();
        let mut members: Vec<(String, Agent)> = Vec::new();
        {
            let mut stmt = self.conn.prepare_cached(self.sql("ListAgents")?)?;
            let rows = stmt.query_map([], |r| {
                let id: String = r.get(0)?;
                let parent: Option<String> = r.get(1)?;
                let skills: String = r.get(7)?;
                let tools: String = r.get(8)?;
                let active: i64 = r.get(9)?;
                Ok((
                    parent,
                    id,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get::<_, i64>(6)? != 0,
                    skills,
                    tools,
                    active,
                    r.get::<_, Option<String>>(10)?,
                ))
            })?;
            for row in rows {
                let (parent, id, name, prompt, model, hue, pinned, skills, tools, active, provider) =
                    row?;
                let mut chats = sessions.remove(&id).unwrap_or_default();
                if chats.is_empty() {
                    chats.push(chat("New chat", Vec::new()));
                }
                let agent = Agent {
                    active: (active.max(0) as usize).min(chats.len() - 1),
                    sessions: chats,
                    id,
                    name,
                    prompt,
                    // A model is a provider and an id together; half of one is no model.
                    model: provider
                        .zip(model)
                        .map(|(provider, id)| ModelRef { provider, id }),
                    hue,
                    pinned,
                    status: AgentStatus::Idle,
                    skills: serde_json::from_str(&skills)?,
                    tools: serde_json::from_str(&tools)?,
                    subagents: Vec::new(),
                };
                match parent {
                    None => top.push(agent),
                    Some(p) => members.push((p, agent)),
                }
            }
        }
        for (parent, member) in members {
            if let Some(p) = top.iter_mut().find(|a| a.id == parent) {
                p.subagents.push(member);
            }
        }
        Ok(top)
    }

    /// The agent's own row (not its chats). `position` is its place among its siblings.
    pub fn save_agent(&self, a: &Agent, parent: Option<&str>, position: usize) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("UpsertAgent")?)?
            .execute((
                &a.id,
                parent,
                &a.name,
                &a.prompt,
                a.model.as_ref().map(|m| &m.id),
                a.hue,
                i64::from(a.pinned),
                serde_json::to_string(&a.skills)?,
                serde_json::to_string(&a.tools)?,
                a.active as i64,
                position as i64,
                a.model.as_ref().map(|m| &m.provider),
            ))?;
        Ok(())
    }

    /// An agent, its chats and their messages, and its team the same way, in one step.
    pub fn save_agent_tree(&self, a: &Agent, parent: Option<&str>, position: usize) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.write_tree(a, parent, position)?;
        tx.commit()?;
        Ok(())
    }

    /// Writes every agent: for the first launch.
    pub fn save_roster(&self, list: &[Agent]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (i, a) in list.iter().enumerate() {
            self.write_tree(a, None, i)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The tree's rows, inside whatever transaction the caller opened (SQLite has no nesting).
    fn write_tree(&self, a: &Agent, parent: Option<&str>, position: usize) -> Result<()> {
        self.save_agent(a, parent, position)?;
        for (i, s) in a.sessions.iter().enumerate() {
            self.save_session(&a.id, s, i)?;
            for (seq, m) in s.messages.iter().enumerate() {
                self.put_message(&s.id, seq, m)?;
            }
        }
        for (i, member) in a.subagents.iter().enumerate() {
            self.write_tree(member, Some(&a.id), i)?;
        }
        Ok(())
    }

    /// Removes an agent; its team, chats and messages go with it.
    pub fn delete_agent(&self, id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteAgent")?)?
            .execute([id])?;
        Ok(())
    }

    pub fn save_session(&self, agent_id: &str, s: &Session, position: usize) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("UpsertSession")?)?
            .execute((&s.id, agent_id, &s.title, position as i64))?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteSession")?)?
            .execute([id])?;
        Ok(())
    }

    /// Adds the message at `seq`, or replaces it (an approval decided in place).
    pub fn put_message(&self, session_id: &str, seq: usize, m: &Message) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("PutMessage")?)?
            .execute((session_id, seq as i64, serde_json::to_string(m)?))?;
        Ok(())
    }
}
