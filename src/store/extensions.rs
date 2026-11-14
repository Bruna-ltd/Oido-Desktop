//! Extensions: which are on, which of their tools are off, and the sealed Google account.

use std::collections::BTreeSet;

use rusqlite::OptionalExtension;

use super::{Result, Store};
use crate::extensions::google::Account;

/// What the person chose about one extension.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Setting {
    pub enabled: bool,
    pub off_tools: BTreeSet<String>,
}

impl Store {
    pub fn load_extensions(&self) -> Result<Vec<(String, Setting)>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListExtensions")?)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut all = Vec::new();
        for row in rows {
            let (id, enabled, off) = row?;
            all.push((
                id,
                Setting {
                    enabled: enabled != 0,
                    off_tools: serde_json::from_str(&off)?,
                },
            ));
        }
        Ok(all)
    }

    pub fn save_extension(&self, id: &str, setting: &Setting) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("SaveExtension")?)?
            .execute((
                id,
                i64::from(setting.enabled),
                serde_json::to_string(&setting.off_tools)?,
            ))?;
        Ok(())
    }

    /// The sealed account of `provider`, opened, or `None` when nothing was ever saved.
    pub fn extension_account(&self, provider: &str) -> Result<Option<Account>> {
        let sealed: Option<Vec<u8>> = self
            .conn
            .prepare_cached(self.sql("GetExtensionAccount")?)?
            .query_row([provider], |r| r.get(0))
            .optional()?;
        sealed
            .map(|blob| {
                let plain = self.vault.open(&account_context(provider), &blob)?;
                Ok(serde_json::from_str(&plain)?)
            })
            .transpose()
    }

    pub fn save_extension_account(&self, provider: &str, account: &Account) -> Result<()> {
        let json = zeroize::Zeroizing::new(serde_json::to_string(account)?);
        let sealed = self.vault.seal(&account_context(provider), &json)?;
        self.conn
            .prepare_cached(self.sql("SaveExtensionAccount")?)?
            .execute((provider, sealed))?;
        Ok(())
    }
}

/// What a sealed account is bound to, so one provider's cannot be moved to another's row.
fn account_context(provider: &str) -> String {
    format!("extension-account:{provider}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::tests::mem;
    use super::*;

    #[test]
    fn settings_round_trip_and_replace() {
        let s = mem();
        assert!(s.load_extensions().unwrap().is_empty());
        let mut setting = Setting {
            enabled: true,
            off_tools: BTreeSet::from(["list_labels".to_owned()]),
        };
        s.save_extension("gmail", &setting).unwrap();
        setting.enabled = false;
        s.save_extension("gmail", &setting).unwrap();
        assert_eq!(
            s.load_extensions().unwrap(),
            [("gmail".to_owned(), setting)]
        );
    }

    #[test]
    fn the_account_is_sealed_and_comes_back() {
        let s = mem();
        assert!(s.extension_account("google").unwrap().is_none());
        let account = Account {
            client_id: "cid".into(),
            client_secret: "hunter2-secret".into(),
            email: Some("me@x.org".into()),
            ..Account::default()
        };
        s.save_extension_account("google", &account).unwrap();
        let blob: Vec<u8> = s
            .conn
            .query_row(
                "SELECT secrets_enc FROM extension_accounts WHERE provider = 'google'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!String::from_utf8_lossy(&blob).contains("hunter2"));
        let back = s.extension_account("google").unwrap().unwrap();
        assert_eq!(back.client_secret, "hunter2-secret");
        assert_eq!(back.email.as_deref(), Some("me@x.org"));
        // Saving again replaces it.
        s.save_extension_account("google", &Account::default())
            .unwrap();
        assert!(!s.extension_account("google").unwrap().unwrap().has_client());
    }

    #[test]
    fn an_account_moved_to_another_provider_does_not_open() {
        let s = mem();
        s.save_extension_account("google", &Account::default())
            .unwrap();
        s.conn
            .execute(
                "UPDATE extension_accounts SET provider = 'other' WHERE provider = 'google'",
                [],
            )
            .unwrap();
        assert!(s.extension_account("other").is_err());
    }
}
