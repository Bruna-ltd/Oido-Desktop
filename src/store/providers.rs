//! Providers. The key itself is not stored here (the OS keyring holds it); only the masked hint.

use rusqlite::OptionalExtension;
use zeroize::Zeroizing;

use super::{Error, Result, Store};
use crate::credentials::{Credential, Tokens};
use crate::models::ModelInfo;
use crate::providers::{Provider, ProviderAuth, color_of};

/// A provider whose key just proved itself by listing models: what is written with it, together.
pub struct Verified<'a> {
    pub id: &'a str,
    /// The sealed key to store, or `None` to keep the one it has.
    pub sealed: Option<&'a [u8]>,
    pub models: &'a [ModelInfo],
    pub fetched_at: i64,
}

impl Store {
    pub fn load_providers(&self) -> Result<Vec<Provider>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListProviders")?)?;
        let rows = stmt.query_map([], |r| {
            let id: String = r.get(0)?;
            let custom = r.get::<_, i64>(2)? != 0;
            Ok(Provider {
                color: color_of(&id, custom),
                label: r.get(1)?,
                custom,
                connected: r.get::<_, i64>(3)? != 0,
                key_masked: r.get(4)?,
                base_url: r.get(5)?,
                auth: auth_of(&r.get::<_, String>(6)?),
                id,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn save_provider(&self, p: &Provider, position: usize) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("UpsertProvider")?)?
            .execute((
                &p.id,
                &p.label,
                i64::from(p.custom),
                i64::from(p.connected),
                &p.key_masked,
                &p.base_url,
                auth_str(p.auth),
                position as i64,
            ))?;
        Ok(())
    }

    /// Makes the stored list exactly `list`, and, if given, stores
    /// the verified key (sealed by [`Store::seal_provider_key`]) and its models: all in one step, so a
    /// provider is never saved as connected without its key. Providers that stay keep their keys;
    /// removed or disconnected ones lose theirs.
    pub fn replace_providers(&self, list: &[Provider], verified: Option<Verified>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for stored in self.load_providers()? {
            if !list.iter().any(|p| p.id == stored.id) {
                self.conn
                    .prepare_cached(self.sql("DeleteProvider")?)?
                    .execute([&stored.id])?;
            }
        }
        for (i, p) in list.iter().enumerate() {
            self.save_provider(p, i)?;
        }
        // A provider that is not connected has no models to offer: its saved list goes too.
        for p in list.iter().filter(|p| !p.connected) {
            self.conn
                .prepare_cached(self.sql("DeleteModels")?)?
                .execute([&p.id])?;
            self.conn
                .prepare_cached(self.sql("SetModelsFetchedAt")?)?
                .execute((&p.id, None::<i64>))?;
        }
        if let Some(v) = verified {
            if let Some(sealed) = v.sealed {
                let changed = self
                    .conn
                    .prepare_cached(self.sql("SetProviderKey")?)?
                    .execute((v.id, sealed))?;
                if changed == 0 {
                    return Err(Error::NoSuchProvider(v.id.to_owned()));
                }
            }
            self.write_models(v.id, v.models, v.fetched_at)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Seals `key` for `id`, to be stored by [`Store::replace_providers`]. Nothing is written, so
    /// if the vault cannot be reached nothing has changed.
    pub fn seal_provider_key(&self, id: &str, key: &str) -> Result<Vec<u8>> {
        Ok(self.vault.seal(&key_context(id), key)?)
    }

    /// Built-in providers marked connected that have no key, from a connect that was cut short.
    pub fn connected_without_key(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached(self.sql("ListConnectedWithoutKey")?)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The provider's credential, opened, or `None` when it has none. A key is wiped from memory
    /// when dropped, so hold it only as long as a request needs it.
    pub fn provider_credential(&self, id: &str) -> Result<Option<Credential>> {
        let Some((stored, auth)) = self.stored(id)? else {
            return Ok(None);
        };
        Ok(Some(match auth {
            ProviderAuth::ApiKey => Credential::Key(stored),
            ProviderAuth::Subscription => {
                Credential::Subscription(Zeroizing::new(self.tokens_of(id, &stored)?.access_token))
            }
        }))
    }

    /// The provider's own API key, opened. `None` when it has none, or signs in with a subscription.
    pub fn provider_key(&self, id: &str) -> Result<Option<Zeroizing<String>>> {
        Ok(match self.provider_credential(id)? {
            Some(Credential::Key(key)) => Some(key),
            _ => None,
        })
    }

    /// A subscription's tokens, refresh token included, for keeping them current. `None` for a
    /// provider that is not signed in with a subscription.
    pub fn subscription_tokens(&self, id: &str) -> Result<Option<Tokens>> {
        let Some((stored, auth)) = self.stored(id)? else {
            return Ok(None);
        };
        match auth {
            ProviderAuth::Subscription => Ok(Some(self.tokens_of(id, &stored)?)),
            ProviderAuth::ApiKey => Ok(None),
        }
    }

    /// Writes a refreshed or newly signed-in subscription's tokens, sealed like a key.
    pub fn save_subscription_tokens(&self, id: &str, tokens: &Tokens) -> Result<()> {
        let sealed = self.seal_provider_key(
            id,
            &tokens
                .json()
                .map_err(|_| Error::BadCredential(id.to_owned()))?,
        )?;
        let changed = self
            .conn
            .prepare_cached(self.sql("SetProviderSubscription")?)?
            .execute((id, sealed))?;
        if changed == 0 {
            return Err(Error::NoSuchProvider(id.to_owned()));
        }
        Ok(())
    }

    /// Writes the whole list and the active one in one step: for the first launch.
    pub fn save_providers(&self, list: &[Provider]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (i, p) in list.iter().enumerate() {
            self.save_provider(p, i)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The sealed credential and how it is used, opened, or `None` when the provider has neither.
    fn stored(&self, id: &str) -> Result<Option<(Zeroizing<String>, ProviderAuth)>> {
        let row: Option<(Option<Vec<u8>>, String)> = self
            .conn
            .prepare_cached(self.sql("GetProviderCredential")?)?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        match row {
            Some((Some(blob), auth)) => Ok(Some((
                self.vault.open(&key_context(id), &blob)?,
                auth_of(&auth),
            ))),
            _ => Ok(None),
        }
    }

    /// The tokens in a subscription's sealed value.
    fn tokens_of(&self, id: &str, stored: &str) -> Result<Tokens> {
        Tokens::parse(stored).map_err(|_| Error::BadCredential(id.to_owned()))
    }
}

/// What a sealed credential is bound to, so one provider's key cannot be moved to another's row.
fn key_context(id: &str) -> String {
    format!("provider-key:{id}")
}

fn auth_of(stored: &str) -> ProviderAuth {
    match stored {
        "subscription" => ProviderAuth::Subscription,
        _ => ProviderAuth::ApiKey,
    }
}

fn auth_str(auth: ProviderAuth) -> &'static str {
    match auth {
        ProviderAuth::ApiKey => "api_key",
        ProviderAuth::Subscription => "subscription",
    }
}
