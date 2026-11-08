//! Settings: any JSON value under a name.

use rusqlite::OptionalExtension;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Result, Store};

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

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteSetting")?)?
            .execute([key])?;
        Ok(())
    }
}
