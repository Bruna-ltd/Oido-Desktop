//! Settings: any JSON value under a name.

use rusqlite::OptionalExtension;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Result, Store};

/// Where the connector's token is kept, and what it is sealed against.
const CONNECTOR_TOKEN: &str = "connector.token";

impl Store {
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .conn
            .prepare_cached(self.sql("GetSetting")?)?
            .query_row([key], |r| r.get(0))
            .optional()?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value)?;
        self.conn
            .prepare_cached(self.sql("SetSetting")?)?
            .execute((key, json))?;
        Ok(())
    }

    /// The token the Oido Browser Connector sends, sealed in the file like a key.
    pub fn connector_token(&self) -> Result<Option<String>> {
        match self.setting::<Vec<u8>>(CONNECTOR_TOKEN)? {
            Some(sealed) => Ok(Some(self.vault.open(CONNECTOR_TOKEN, &sealed)?.to_string())),
            None => Ok(None),
        }
    }

    pub fn set_connector_token(&self, token: &str) -> Result<()> {
        let sealed = self.vault.seal(CONNECTOR_TOKEN, token)?;
        self.set_setting(CONNECTOR_TOKEN, &sealed)
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteSetting")?)?
            .execute([key])?;
        Ok(())
    }
}
