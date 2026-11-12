//! Channels: each one's connected bot (what it signs in with, sealed), the people paired with it,
//! and the chat each carries on with each agent. The rows name their channel.

use rusqlite::OptionalExtension;
use zeroize::Zeroizing;

use super::{Result, Store};
use crate::channels::{Binding, Kind, Paired};

/// A connected bot as saved: what it signs in with, opened (as the channel writes it), its
/// handle and whether it listens.
pub struct SavedBot {
    pub secret: Zeroizing<String>,
    pub name: String,
    pub enabled: bool,
}

impl Store {
    /// `kind`'s bot, what it signs in with opened. `None` when none is connected.
    pub fn channel_bot(&self, kind: Kind) -> Result<Option<SavedBot>> {
        let row: Option<(Vec<u8>, String, i64)> = self
            .conn
            .prepare_cached(self.sql("GetChannelBot")?)?
            .query_row([kind.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((blob, name, enabled)) = row else {
            return Ok(None);
        };
        Ok(Some(SavedBot {
            secret: self.vault.open(&token_context(kind), &blob)?,
            name,
            enabled: enabled != 0,
        }))
    }

    /// Saves `kind`'s bot with `secret` sealed. Nothing is written if it cannot be sealed.
    pub fn save_channel_bot(
        &self,
        kind: Kind,
        secret: &str,
        name: &str,
        enabled: bool,
    ) -> Result<()> {
        let sealed = self.vault.seal(&token_context(kind), secret)?;
        self.conn
            .prepare_cached(self.sql("UpsertChannelBot")?)?
            .execute((kind.as_str(), sealed, name, i64::from(enabled)))?;
        Ok(())
    }

    pub fn set_channel_enabled(&self, kind: Kind, enabled: bool) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("SetChannelBotEnabled")?)?
            .execute((kind.as_str(), i64::from(enabled)))?;
        Ok(())
    }

    /// Forgets `kind`'s bot and what it signs in with. The people paired with it stay paired.
    pub fn delete_channel_bot(&self, kind: Kind) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteChannelBot")?)?
            .execute([kind.as_str()])?;
        Ok(())
    }

    pub fn channel_users(&self, kind: Kind) -> Result<Vec<Paired>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListChannelUsers")?)?;
        let rows = stmt.query_map([kind.as_str()], |r| {
            Ok(Paired {
                id: r.get(0)?,
                name: r.get(1)?,
                handle: r.get(2)?,
                agent: r.get(3)?,
                trusted: r.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn save_channel_user(&self, kind: Kind, p: &Paired) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("UpsertChannelUser")?)?
            .execute((
                kind.as_str(),
                &p.id,
                &p.name,
                &p.handle,
                &p.agent,
                i64::from(p.trusted),
            ))?;
        Ok(())
    }

    /// Unpairs a person; their bindings go with them, their chats stay.
    pub fn delete_channel_user(&self, kind: Kind, id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteChannelUser")?)?
            .execute((kind.as_str(), id))?;
        Ok(())
    }

    pub fn channel_chats(&self, kind: Kind) -> Result<Vec<Binding>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListChannelChats")?)?;
        let rows = stmt.query_map([kind.as_str()], |r| {
            Ok(Binding {
                user: r.get(0)?,
                agent: r.get(1)?,
                session: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn save_channel_chat(&self, kind: Kind, b: &Binding) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("UpsertChannelChat")?)?
            .execute((kind.as_str(), &b.user, &b.agent, &b.session))?;
        Ok(())
    }

    /// The agents people on `kind` may pick, by id.
    pub fn channel_agents(&self, kind: Kind) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListChannelAgents")?)?;
        let rows = stmt.query_map([kind.as_str()], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Lets people on `kind` pick agent `agent`, or no longer (`on` false).
    pub fn set_channel_agent(&self, kind: Kind, agent: &str, on: bool) -> Result<()> {
        let name = if on {
            "AddChannelAgent"
        } else {
            "RemoveChannelAgent"
        };
        self.conn
            .prepare_cached(self.sql(name)?)?
            .execute((kind.as_str(), agent))?;
        Ok(())
    }
}

/// What a sealed token is bound to, so it cannot be moved to another row.
fn token_context(kind: Kind) -> String {
    format!("channel:{}", kind.as_str())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::tests::{idle, mem};
    use super::*;
    use crate::demo::sample_agents;

    const T: Kind = Kind::Telegram;

    #[test]
    fn a_bot_token_is_sealed_in_the_file_and_opens_back() {
        let s = mem();
        assert!(s.channel_bot(T).unwrap().is_none());
        s.save_channel_bot(T, "123:secret-token", "oido_bot", true)
            .unwrap();
        let raw: Vec<u8> = s
            .conn
            .query_row("SELECT token_enc FROM channel_bots", [], |r| r.get(0))
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&raw).contains("secret-token"),
            "the token is not readable in the file"
        );
        let bot = s.channel_bot(T).unwrap().unwrap();
        assert_eq!(
            (bot.secret.as_str(), bot.name.as_str()),
            ("123:secret-token", "oido_bot")
        );
        s.set_channel_enabled(T, false).unwrap();
        assert!(!s.channel_bot(T).unwrap().unwrap().enabled);
        s.delete_channel_bot(T).unwrap();
        assert!(s.channel_bot(T).unwrap().is_none());
    }

    #[test]
    fn unpairing_ends_bindings_and_a_deleted_chat_ends_its_own() {
        let s = mem();
        let agents: Vec<_> = sample_agents().into_iter().map(idle).collect();
        s.save_roster(&agents).unwrap();
        let (agent, chat) = (&agents[0].id, &agents[0].sessions[0].id);
        let p = Paired {
            id: "42".into(),
            name: "Ada".into(),
            handle: Some("ada".into()),
            agent: Some(agent.clone()),
            trusted: false,
        };
        s.save_channel_user(T, &p).unwrap();
        let b = Binding {
            user: "42".into(),
            agent: agent.clone(),
            session: chat.clone(),
        };
        s.save_channel_chat(T, &b).unwrap();
        assert_eq!(s.channel_users(T).unwrap(), vec![p.clone()]);
        assert_eq!(s.channel_chats(T).unwrap(), vec![b.clone()]);
        s.delete_channel_user(T, "42").unwrap();
        assert!(s.channel_chats(T).unwrap().is_empty());

        s.save_channel_user(T, &p).unwrap();
        s.save_channel_chat(T, &b).unwrap();
        s.conn
            .execute("DELETE FROM sessions WHERE id = ?1", [chat])
            .unwrap();
        assert!(s.channel_chats(T).unwrap().is_empty());
        assert_eq!(
            s.channel_users(T).unwrap().len(),
            1,
            "the person stays paired"
        );
    }

    #[test]
    fn telegrams_rows_survive_the_move_to_any_channel_name() {
        use rusqlite::Connection;
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for sql in &super::super::MIGRATIONS[..15] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO agents (id, name) VALUES ('a', 'Atlas');
             INSERT INTO sessions (id, agent_id) VALUES ('s', 'a');
             INSERT INTO channel_bots (channel, token_enc, name) VALUES ('telegram', x'00', 'bot');
             INSERT INTO channel_users (channel, user_id, name, agent_id) VALUES ('telegram', '42', 'Ada', 'a');
             INSERT INTO channel_chats (channel, user_id, agent_id, session_id) VALUES ('telegram', '42', 'a', 's');",
        )
        .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute_batch(super::super::MIGRATIONS[15]).unwrap();
        tx.commit().unwrap();
        let count = |t: &str| -> i64 {
            conn.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(
            (
                count("channel_bots"),
                count("channel_users"),
                count("channel_chats")
            ),
            (1, 1, 1),
            "nothing lost, the chat's binding included"
        );
        conn.execute(
            "INSERT INTO channel_users (channel, user_id, name) VALUES ('slack', 'U1', 'Ada')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM channel_users WHERE user_id = '42'", [])
            .unwrap();
        assert_eq!(
            count("channel_chats"),
            0,
            "unpairing still ends the binding"
        );
    }

    #[test]
    fn agents_switched_on_for_channels_are_chosen_on_each_channel_with_a_bot() {
        use rusqlite::Connection;
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for sql in &super::super::MIGRATIONS[..16] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO agents (id, name, on_channels) VALUES ('a', 'Atlas', 1), ('b', 'Bea', 0);
             INSERT INTO agents (id, parent_id, name, on_channels) VALUES ('m', 'a', 'Member', 1);
             INSERT INTO channel_bots (channel, token_enc, name) VALUES ('telegram', x'00', 'bot');",
        )
        .unwrap();
        conn.execute_batch(super::super::MIGRATIONS[16]).unwrap();
        let rows: Vec<(String, String)> = conn
            .prepare("SELECT channel, agent_id FROM channel_agents")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(rows, [("telegram".to_owned(), "a".to_owned())]);
    }
}
