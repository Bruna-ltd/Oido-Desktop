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
        // Chats made up for agents that had none saved: saved below, so a message sent in one has a
        // row to belong to.
        let mut made_up: Vec<(String, Session)> = Vec::new();
        {
            let mut stmt = self.conn.prepare_cached(self.sql("ListAgents")?)?;
            // The row as an agent, but for what needs more than a column read: its skills and
            // tools (JSON), its chats, and which one is open.
            let rows = stmt.query_map([], |r| {
                let provider: Option<String> = r.get(10)?;
                let model: Option<String> = r.get(4)?;
                let agent = Agent {
                    id: r.get(0)?,
                    name: r.get(2)?,
                    prompt: r.get(3)?,
                    // A model is a provider and an id together; half of one is no model.
                    model: provider
                        .zip(model)
                        .map(|(provider, id)| ModelRef { provider, id }),
                    hue: r.get(5)?,
                    pinned: r.get::<_, i64>(6)? != 0,
                    status: AgentStatus::Idle,
                    skills: Vec::new(),
                    tools: Vec::new(),
                    subagents: Vec::new(),
                    workdir: r.get(11)?,
                    description: r.get(12)?,
                    on_channels: r.get::<_, i64>(13)? != 0,
                    active: 0,
                    sessions: Vec::new(),
                };
                Ok((
                    r.get::<_, Option<String>>(1)?,
                    agent,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, i64>(9)?,
                ))
            })?;
            for row in rows {
                let (parent, mut agent, skills, tools, active) = row?;
                agent.skills = serde_json::from_str(&skills)?;
                agent.tools = serde_json::from_str(&tools)?;
                let mut chats = sessions.remove(&agent.id).unwrap_or_default();
                if chats.is_empty() {
                    let first = chat("New chat", Vec::new());
                    made_up.push((agent.id.clone(), first.clone()));
                    chats.push(first);
                }
                agent.active = (active.max(0) as usize).min(chats.len() - 1);
                agent.sessions = chats;
                match parent {
                    None => top.push(agent),
                    Some(p) => members.push((p, agent)),
                }
            }
        }
        for (agent, session) in &made_up {
            self.save_session(agent, session, 0)?;
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
                &a.workdir,
                &a.description,
                i64::from(a.on_channels),
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

    /// Removes chat `id` and renumbers the agent's chats left, `rest`, in one step. Chats keep
    /// positions 0..n: a new one is saved at the end by its index, and the open one is
    /// remembered by its index, so a gap would reorder the tabs.
    pub fn remove_session(&self, id: &str, agent_id: &str, rest: &[Session]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.delete_session(id)?;
        for (i, s) in rest.iter().enumerate() {
            self.save_session(agent_id, s, i)?;
        }
        tx.commit()?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::agents::Agent;
    use crate::store::Store;
    use crate::vault::Vault;

    #[test]
    fn an_agent_saved_without_a_chat_gets_one_that_is_saved_so_its_messages_can_be() {
        use crate::model::Message;
        let db = Store::in_memory(Vault::fixed([1; 32])).unwrap();
        let mut lone = Agent::blank("lone".into());
        lone.name = "Lone".into();
        db.save_agent(&lone, None, 0).unwrap();
        let loaded = db.load_agents().unwrap();
        let chat = loaded[0].chat().id.clone();
        // Before this, saving the first message failed: FOREIGN KEY constraint failed.
        db.put_message(&chat, 0, &Message::User("hello".into()))
            .unwrap();
        let again = db.load_agents().unwrap();
        assert_eq!(again[0].sessions.len(), 1, "no second chat is made up");
        assert_eq!(again[0].chat().id, chat, "the same chat comes back");
        assert_eq!(again[0].chat().messages.len(), 1);
    }

    #[test]
    fn an_agents_working_folder_is_saved_and_none_stays_none() {
        let db = Store::in_memory(Vault::fixed([1; 32])).unwrap();
        let (mut a, mut b) = (Agent::blank("a".into()), Agent::blank("b".into()));
        (a.name, b.name) = ("A".into(), "B".into());
        a.workdir = Some("/srv/project".into());
        db.save_roster(&[a, b]).unwrap();
        let loaded = db.load_agents().unwrap();
        assert_eq!(loaded[0].workdir.as_deref(), Some("/srv/project"));
        assert_eq!(loaded[1].workdir, None);
    }
}
