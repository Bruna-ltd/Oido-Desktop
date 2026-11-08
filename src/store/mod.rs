//! The app's local SQLite store: settings, providers, agents, chats and their messages.
//!
//! The SQL lives in `db/`, laid out as `oido-core`'s sqlc project is: `db/migrations` for the
//! schema and `db/queries` for named statements (`-- name: GetAgent :one`). Rust has no sqlc, so
//! the typed functions here are hand-written over those files, and a test prepares every
//! statement against the migrated schema, so a query that no longer matches the tables fails
//! the build's tests rather than a user's launch.

use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::vault::{self, Vault};

mod agents;
mod models;
mod providers;
mod settings;

pub use providers::Verified;

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
    /// The vault could not seal or open a key.
    Vault(vault::Error),
    /// A key was set for a provider that is not saved.
    NoSuchProvider(String),
    /// A statement the code asks for is not in `db/queries`.
    MissingQuery(&'static str),
    /// The file was made by a newer version of the app than this one.
    NewerSchema {
        found: i64,
        known: i64,
    },
    NoDataDir,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "database error: {e}"),
            Self::Json(e) => write!(f, "stored data is not valid: {e}"),
            Self::Io(e) => write!(f, "could not prepare the data folder: {e}"),
            Self::Vault(e) => write!(f, "{e}"),
            Self::NoSuchProvider(id) => write!(f, "no saved provider {id} to keep a key for"),
            Self::MissingQuery(n) => write!(f, "no query named {n} in db/queries"),
            Self::NewerSchema { found, known } => {
                write!(
                    f,
                    "this data was made by a newer version (schema {found}, this app knows {known})"
                )
            }
            Self::NoDataDir => write!(f, "no data folder on this system"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<vault::Error> for Error {
    fn from(e: vault::Error) -> Self {
        Self::Vault(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Schema steps, in order. The file's `user_version` says how many have run.
const MIGRATIONS: [&str; 3] = [
    include_str!("../../db/migrations/001_init.sql"),
    include_str!("../../db/migrations/002_provider_keys.sql"),
    include_str!("../../db/migrations/003_models.sql"),
];

const QUERY_FILES: [&str; 4] = [
    include_str!("../../db/queries/settings.sql"),
    include_str!("../../db/queries/providers.sql"),
    include_str!("../../db/queries/agents.sql"),
    include_str!("../../db/queries/sessions.sql"),
];

pub struct Store {
    conn: Connection,
    queries: Vec<(&'static str, &'static str)>,
    /// Seals what must not be readable in the file: provider keys.
    vault: Vault,
}

impl Store {
    /// The user's database: `$OIDO_DB` if set (`:memory:` for a throwaway one), else
    /// `oido.db` in the app's data folder.
    pub fn open_default() -> Result<Store> {
        match std::env::var("OIDO_DB") {
            Ok(p) if p == ":memory:" => Store::in_memory(Vault::os()),
            Ok(p) => Store::open(Path::new(&p), Vault::os()),
            Err(_) => Store::open(&default_path()?, Vault::os()),
        }
    }

    pub fn open(path: &Path, vault: Vault) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Store::init(Connection::open(path)?, vault)
    }

    pub fn in_memory(vault: Vault) -> Result<Store> {
        Store::init(Connection::open_in_memory()?, vault)
    }

    fn init(conn: Connection, vault: Vault) -> Result<Store> {
        // Foreign keys are off by default in SQLite, and the cascades depend on them.
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; \
             PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 3000;",
        )?;
        migrate(&conn)?;
        Ok(Store {
            conn,
            queries: parse_queries(),
            vault,
        })
    }

    fn sql(&self, name: &'static str) -> Result<&'static str> {
        self.queries
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, sql)| *sql)
            .ok_or(Error::MissingQuery(name))
    }
}

fn default_path() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .ok_or(Error::NoDataDir)?
        .join("oido-desktop")
        .join("oido.db"))
}

fn migrate(conn: &Connection) -> Result<()> {
    let found: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let known = MIGRATIONS.len() as i64;
    if found > known {
        return Err(Error::NewerSchema { found, known });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        // A step and its version bump land together or not at all.
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// Splits the query files on their `-- name: Name :kind` lines. Comment lines left at the end of
/// a statement are harmless to SQLite.
fn parse_queries() -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    for file in QUERY_FILES {
        for chunk in file.split("-- name: ").skip(1) {
            let (head, body) = chunk.split_once('\n').unwrap_or((chunk, ""));
            if let Some(name) = head.split_whitespace().next() {
                out.push((name, body.trim()));
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::agents::{Agent, seed};
    use crate::model::{AgentStatus, chat};
    use crate::providers;

    /// A throwaway store whose vault has a fixed key: tests never touch a real keyring.
    pub(crate) fn mem() -> Store {
        Store::in_memory(Vault::fixed([1; 32])).unwrap()
    }

    pub(crate) fn idle(mut a: Agent) -> Agent {
        a.status = AgentStatus::Idle;
        a.subagents = a.subagents.into_iter().map(idle).collect();
        a
    }

    #[test]
    fn every_query_prepares_against_the_migrated_schema() {
        let s = mem();
        assert!(
            s.queries.len() >= 14,
            "the query files were parsed: {}",
            s.queries.len()
        );
        for (name, sql) in &s.queries {
            s.conn
                .prepare(sql)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn migrations_run_once_and_a_reopened_file_keeps_its_data() {
        let dir = std::env::temp_dir().join(format!("oido-store-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("nested").join("oido.db");
        {
            let s = Store::open(&path, Vault::fixed([1; 32])).unwrap();
            s.set_setting("theme", &"dark").unwrap();
        }
        let s = Store::open(&path, Vault::fixed([1; 32])).unwrap();
        let version: i64 = s
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        assert_eq!(
            s.setting::<String>("theme").unwrap().as_deref(),
            Some("dark")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_database_from_a_newer_app_is_refused_not_misread() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            Store::init(conn, Vault::fixed([1; 32])),
            Err(Error::NewerSchema { found: 99, .. })
        ));
    }

    #[test]
    fn settings_round_trip_any_json_and_can_be_removed() {
        let s = mem();
        assert_eq!(s.setting::<Vec<u32>>("n").unwrap(), None);
        s.set_setting("n", &vec![1, 2, 3]).unwrap();
        s.set_setting("n", &vec![4]).unwrap();
        assert_eq!(s.setting::<Vec<u32>>("n").unwrap(), Some(vec![4]));
        s.delete_setting("n").unwrap();
        assert_eq!(s.setting::<Vec<u32>>("n").unwrap(), None);
    }

    #[test]
    fn providers_round_trip_in_order() {
        let s = mem();
        let mut p = providers::init();
        p.list[0].connected = true;
        p.list[0].key_masked = Some("sk-…7890".into());
        s.save_providers(&p.list).unwrap();
        let back = s.load_providers().unwrap();
        assert_eq!(back.len(), p.list.len());
        for (a, b) in p.list.iter().zip(&back) {
            assert_eq!(
                (
                    &a.id,
                    &a.label,
                    a.connected,
                    &a.key_masked,
                    &a.base_url,
                    a.custom
                ),
                (
                    &b.id,
                    &b.label,
                    b.connected,
                    &b.key_masked,
                    &b.base_url,
                    b.custom
                )
            );
            assert_eq!(a.color, b.color, "the brand colour comes back from the id");
        }
        let kept: Vec<_> = p
            .list
            .iter()
            .filter(|x| x.id != "openai")
            .cloned()
            .collect();
        s.replace_providers(&kept, None).unwrap();
        assert_eq!(s.load_providers().unwrap().len(), p.list.len() - 1);
    }

    #[test]
    fn the_whole_roster_round_trips_with_teams_chats_and_messages() {
        let s = mem();
        let before: Vec<Agent> = seed().into_iter().map(idle).collect();
        s.save_roster(&before).unwrap();
        let after = s.load_agents().unwrap();
        assert_eq!(after, before);
        assert_eq!(
            after[1].subagents.len(),
            1,
            "the team came back under its agent"
        );
    }

    #[test]
    fn a_team_member_cannot_have_a_team_and_an_agent_with_one_cannot_join_another() {
        let s = mem();
        s.save_roster(&seed().into_iter().map(idle).collect::<Vec<_>>())
            .unwrap();
        let mut deep = idle(seed().remove(2));
        deep.id = "deep".into();
        assert!(
            s.save_agent(&deep, Some("reviewer"), 0).is_err(),
            "a third level is refused"
        );
        let mut forge = idle(seed().remove(1));
        forge.id = "forge".into();
        assert!(
            s.save_agent(&forge, Some("atlas"), 0).is_err(),
            "an agent with a team cannot become a member"
        );
    }

    #[test]
    fn deleting_an_agent_takes_its_team_chats_and_messages_with_it() {
        let s = mem();
        s.save_roster(&seed().into_iter().map(idle).collect::<Vec<_>>())
            .unwrap();
        s.delete_agent("forge").unwrap();
        let left = s.load_agents().unwrap();
        assert!(
            left.iter()
                .all(|a| a.id != "forge" && a.subagents.iter().all(|m| m.id != "reviewer"))
        );
        let rows: i64 = s
            .conn
            .query_row(
                "SELECT count(*) FROM sessions WHERE agent_id = 'forge'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn messages_are_appended_and_a_changed_one_is_replaced_in_place() {
        use crate::model::Message;
        let s = mem();
        let mut a = idle(seed().remove(2));
        a.sessions = vec![chat("One", vec![])];
        let id = a.sessions[0].id.clone();
        s.save_roster(&[a.clone()]).unwrap();
        s.put_message(&id, 0, &Message::User("hi".into())).unwrap();
        s.put_message(&id, 1, &Message::Divider("Context cleared".into()))
            .unwrap();
        s.put_message(&id, 0, &Message::User("hi there".into()))
            .unwrap();
        let back = &s.load_agents().unwrap()[0];
        assert_eq!(
            back.sessions[0].messages,
            vec![
                Message::User("hi there".into()),
                Message::Divider("Context cleared".into())
            ]
        );
    }

    fn with_providers(s: &Store) {
        s.save_providers(&providers::init().list).unwrap();
    }

    /// The sample providers with `ids` connected.
    fn connected_list(ids: &[&str]) -> Vec<providers::Provider> {
        let mut list = providers::init().list;
        for p in &mut list {
            p.connected = ids.contains(&p.id.as_str());
        }
        list
    }

    /// Saves the sample providers, connects each of `keys`, and stores its sealed key, as the app does.
    fn keyed(s: &Store, keys: &[(&str, &str)]) {
        let ids: Vec<&str> = keys.iter().map(|(id, _)| *id).collect();
        let list = connected_list(&ids);
        s.save_providers(&list).unwrap();
        for (id, key) in keys {
            let sealed = s.seal_provider_key(id, key).unwrap();
            let verified = Verified {
                id,
                sealed: Some(&sealed),
                models: &[],
                fetched_at: 0,
            };
            s.replace_providers(&list, Some(verified)).unwrap();
        }
    }

    /// Every byte of the database and its write-ahead log.
    fn bytes_on_disk(path: &Path) -> Vec<u8> {
        let mut all = std::fs::read(path).unwrap();
        for suffix in ["-wal", "-shm"] {
            if let Ok(more) = std::fs::read(format!("{}{suffix}", path.display())) {
                all.extend(more);
            }
        }
        all
    }

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
    }

    #[test]
    fn a_provider_key_is_not_readable_anywhere_in_the_file_and_opens_again_after_a_restart() {
        let dir = std::env::temp_dir().join(format!("oido-key-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("oido.db");
        let key = "sk-live-1234567890-abcdef";
        {
            let s = Store::open(&path, Vault::fixed([1; 32])).unwrap();
            keyed(&s, &[("openai", key)]);
            assert_eq!(&*s.provider_key("openai").unwrap().unwrap(), key);
        }
        let on_disk = bytes_on_disk(&path);
        assert!(
            !contains(&on_disk, key)
                && !contains(&on_disk, "abcdef")
                && !contains(&on_disk, "sk-live"),
            "no part of the key is in the file"
        );

        let again = Store::open(&path, Vault::fixed([1; 32])).unwrap();
        assert_eq!(
            &*again.provider_key("openai").unwrap().unwrap(),
            key,
            "it opens after a restart"
        );
        assert!(
            again.provider_key("anthropic").unwrap().is_none(),
            "a provider with no key has none"
        );
        drop(again);

        let other = Store::open(&path, Vault::fixed([2; 32])).unwrap();
        assert!(
            matches!(
                other.provider_key("openai"),
                Err(Error::Vault(vault::Error::Unreadable))
            ),
            "another master key cannot open it"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_key_needs_a_saved_provider_and_an_unreachable_keyring_stores_nothing() {
        let s = mem();
        assert!(matches!(
            s.replace_providers(
                &[],
                Some(Verified {
                    id: "openai",
                    sealed: Some(b"x"),
                    models: &[],
                    fetched_at: 0
                })
            ),
            Err(Error::NoSuchProvider(_))
        ));
        let down = Store::in_memory(Vault::unavailable()).unwrap();
        with_providers(&down);
        assert!(matches!(
            down.seal_provider_key("openai", "sk-1"),
            Err(Error::Vault(vault::Error::Unavailable(_)))
        ));
        assert!(
            down.provider_key("openai").unwrap().is_none(),
            "nothing was stored"
        );
    }

    #[test]
    fn a_key_moved_to_another_providers_row_does_not_open() {
        let s = mem();
        keyed(&s, &[("openai", "sk-1")]);
        s.conn
            .execute(
                "UPDATE providers SET api_key_enc = (SELECT api_key_enc FROM providers WHERE id = 'openai') WHERE id = 'anthropic'",
                [],
            )
            .unwrap();
        assert!(matches!(
            s.provider_key("anthropic"),
            Err(Error::Vault(vault::Error::Unreadable))
        ));
    }

    #[test]
    fn a_key_stays_with_a_connected_provider_and_goes_when_it_is_removed_or_disconnected() {
        let s = mem();
        keyed(
            &s,
            &[("openai", "sk-1"), ("gemini", "sk-2"), ("kimi", "sk-3")],
        );
        // gemini is removed, kimi is signed out, openai stays connected
        let mut list = connected_list(&["openai", "kimi"]);
        list.retain(|p| p.id != "gemini");
        list.iter_mut().find(|p| p.id == "kimi").unwrap().connected = false;
        s.replace_providers(&list, None).unwrap();
        assert_eq!(
            &*s.provider_key("openai").unwrap().unwrap(),
            "sk-1",
            "a connected provider keeps its key"
        );
        assert!(
            s.provider_key("gemini").unwrap().is_none(),
            "a removed provider took its key with it"
        );
        assert!(
            s.provider_key("kimi").unwrap().is_none(),
            "a signed-out provider holds no key"
        );
    }

    #[test]
    fn a_provider_and_its_key_are_written_together_or_not_at_all() {
        let s = mem();
        with_providers(&s);
        let list = connected_list(&["openai"]);
        // the key's provider is not in the list, so the whole write is refused and none of it lands
        let sealed = s.seal_provider_key("nope", "sk-1").unwrap();
        assert!(
            s.replace_providers(
                &list,
                Some(Verified {
                    id: "nope",
                    sealed: Some(&sealed),
                    models: &[],
                    fetched_at: 0
                })
            )
            .is_err()
        );
        assert!(
            s.load_providers().unwrap().iter().all(|p| !p.connected),
            "the provider was not saved as connected"
        );
    }

    #[test]
    fn a_connect_that_was_cut_short_is_found() {
        let s = mem();
        s.save_providers(&connected_list(&["openrouter"])).unwrap();
        assert_eq!(
            s.connected_without_key().unwrap(),
            vec!["openrouter".to_owned()]
        );
        keyed(&s, &[("openrouter", "sk-1")]);
        assert!(s.connected_without_key().unwrap().is_empty());
    }

    #[test]
    fn migrating_an_older_database_clears_sample_providers_that_had_no_key() {
        let dir = std::env::temp_dir().join(format!("oido-mig-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("oido.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, label, custom, connected, key_hint) VALUES ('anthropic', 'Anthropic', 0, 1, 'sk-…a91f');
                 INSERT INTO providers (id, label, custom, connected, base_url) VALUES ('llama-server:runpod', 'RunPod Llama 70B', 1, 1, 'https://llama.example.net/v1');
                 INSERT INTO providers (id, label, custom, connected, base_url) VALUES ('llama-server:mine', 'Mine', 1, 1, 'http://localhost:8080/v1');
                 ",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }
        let s = Store::open(&path, Vault::fixed([1; 32])).unwrap();
        let list = s.load_providers().unwrap();
        assert!(
            !list.iter().find(|p| p.id == "anthropic").unwrap().connected,
            "a connection with no key was not real"
        );
        assert!(
            list.iter().all(|p| p.id != "llama-server:runpod"),
            "the sample server is gone"
        );
        assert!(
            list.iter().any(|p| p.id == "llama-server:mine"),
            "a server someone really added stays"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The whole real stack: a database file, the real keyring, and a restart. Run with
    /// `cargo test -- --ignored`; it uses its own keyring entry and removes it.
    #[test]
    #[ignore = "needs a running Secret Service (gnome-keyring or kwallet)"]
    fn a_key_survives_a_restart_through_the_real_keyring() {
        let account = format!("test-store-key-{}", uuid::Uuid::new_v4());
        let dir = std::env::temp_dir().join(format!("oido-real-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("oido.db");
        {
            let s = Store::open(&path, Vault::os_account(&account)).unwrap();
            keyed(&s, &[("openai", "sk-live-real-keyring-1234")]);
        }
        assert!(!contains(&bytes_on_disk(&path), "real-keyring"));
        let s = Store::open(&path, Vault::os_account(&account)).unwrap();
        assert_eq!(
            &*s.provider_key("openai").unwrap().unwrap(),
            "sk-live-real-keyring-1234"
        );
        Vault::forget_os_account(&account);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
