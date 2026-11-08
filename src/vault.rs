//! Encrypting what must not sit in the database as readable text: provider API keys.
//!
//! A key is sealed with XChaCha20-Poly1305 under a 256-bit master key. The master key is made
//! once, on first use, and kept in the OS keyring (the Secret Service on Linux), never in the
//! database file, so a copy of the database alone, such as a backup, reveals nothing. If the
//! keyring is unavailable the vault says so and nothing is stored: it never falls back to a key
//! kept beside the data, which would only look like protection.
//!
//! A sealed value is `[version][24-byte nonce][ciphertext and tag]`. The provider's id is bound
//! in as associated data, so a value moved to another row fails to open.

use std::cell::RefCell;
use std::fmt;
use std::sync::OnceLock;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, Zeroizing};

const SERVICE: &str = "oido-desktop";
const ACCOUNT: &str = "database-master-key";
const VERSION: u8 = 1;
const NONCE: usize = 24;

#[derive(Debug)]
pub enum Error {
    /// The OS keyring cannot be reached, so the master key cannot be read or made.
    Unavailable(String),
    /// A sealed value or the master key is malformed, or does not match: the keyring was reset,
    /// the data was changed, or it belongs to another row.
    Unreadable,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(why) => write!(
                f,
                "the system keyring is not available ({why}), so the key was not saved"
            ),
            Self::Unreadable => write!(
                f,
                "the saved key can no longer be read, connect again to replace it"
            ),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

struct MasterKey(Zeroizing<[u8; 32]>);

/// Seals and opens values. The master key is fetched on first use, not at startup, so people who
/// never store a key are never asked to unlock their keyring.
pub struct Vault {
    source: Box<dyn Fn() -> Result<MasterKey>>,
    key: RefCell<Option<MasterKey>>,
}

impl Vault {
    /// The user's vault: the master key is in the OS keyring.
    pub fn os() -> Vault {
        Vault::with_source(|| os_master_key(ACCOUNT))
    }

    fn with_source(source: impl Fn() -> Result<MasterKey> + 'static) -> Vault {
        Vault {
            source: Box::new(source),
            key: RefCell::new(None),
        }
    }

    /// A vault with a fixed key, so tests never touch a real keyring.
    #[cfg(test)]
    pub fn fixed(bytes: [u8; 32]) -> Vault {
        Vault::with_source(move || Ok(MasterKey(Zeroizing::new(bytes))))
    }

    /// The real keyring under a test's own entry name, so the app's master key is never touched.
    #[cfg(test)]
    pub fn os_account(account: &str) -> Vault {
        let account = account.to_owned();
        Vault::with_source(move || os_master_key(&account))
    }

    /// Removes a test's keyring entry.
    #[cfg(test)]
    pub fn forget_os_account(account: &str) {
        if let Ok(entry) = keyring_core::Entry::new(SERVICE, account) {
            let _ = entry.delete_credential();
        }
    }

    /// A vault whose keyring is unreachable.
    #[cfg(test)]
    pub fn unavailable() -> Vault {
        Vault::with_source(|| Err(Error::Unavailable("test".into())))
    }

    fn cipher(&self) -> Result<XChaCha20Poly1305> {
        let mut slot = self.key.borrow_mut();
        if slot.is_none() {
            *slot = Some((self.source)()?);
        }
        let key = slot.as_ref().ok_or(Error::Unreadable)?;
        Ok(XChaCha20Poly1305::new(&Key::from(*key.0)))
    }

    pub fn seal(&self, aad: &str, plaintext: &str) -> Result<Vec<u8>> {
        let cipher = self.cipher()?;
        let nonce: [u8; NONCE] = random()?;
        let sealed = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::Unreadable)?;
        let mut out = Vec::with_capacity(1 + NONCE + sealed.len());
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    pub fn open(&self, aad: &str, blob: &[u8]) -> Result<Zeroizing<String>> {
        let (version, rest) = blob.split_first().ok_or(Error::Unreadable)?;
        if *version != VERSION || rest.len() <= NONCE {
            return Err(Error::Unreadable);
        }
        let (nonce, sealed) = rest.split_at(NONCE);
        let nonce: [u8; NONCE] = nonce.try_into().map_err(|_| Error::Unreadable)?;
        let plain = self
            .cipher()?
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: sealed,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::Unreadable)?;
        let plain = Zeroizing::new(plain);
        String::from_utf8(plain.to_vec())
            .map(Zeroizing::new)
            .map_err(|_| Error::Unreadable)
    }
}

/// Reads the master key from the keyring, making and storing a new one the first time.
fn os_master_key(account: &str) -> Result<MasterKey> {
    ensure_keyring()?;
    let entry = keyring_core::Entry::new(SERVICE, account).map_err(unavailable)?;
    match entry.get_secret() {
        Ok(mut bytes) => {
            let key = <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| Error::Unreadable);
            bytes.zeroize();
            key.map(|k| MasterKey(Zeroizing::new(k)))
        }
        Err(keyring_core::Error::NoEntry) => {
            let raw = Zeroizing::new(random::<32>()?);
            entry.set_secret(raw.as_ref()).map_err(unavailable)?;
            Ok(MasterKey(raw))
        }
        Err(e) => Err(unavailable(e)),
    }
}

/// `N` bytes from the operating system's random source.
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| Error::Unavailable(format!("no randomness: {e}")))?;
    Ok(bytes)
}

fn unavailable(e: keyring_core::Error) -> Error {
    Error::Unavailable(e.to_string())
}

/// Registers the Secret Service as the keyring, once per process.
fn ensure_keyring() -> Result<()> {
    static READY: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    READY
        .get_or_init(|| {
            let store =
                zbus_secret_service_keyring_store::Store::new().map_err(|e| e.to_string())?;
            keyring_core::set_default_store(store);
            Ok(())
        })
        .clone()
        .map_err(Error::Unavailable)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn vault() -> Vault {
        Vault::fixed([7; 32])
    }

    #[test]
    fn a_sealed_key_opens_to_the_same_text_and_is_not_readable_in_the_blob() {
        let v = vault();
        let blob = v.seal("openai", "sk-live-secret-1234567890").unwrap();
        assert_eq!(
            &*v.open("openai", &blob).unwrap(),
            "sk-live-secret-1234567890"
        );
        let haystack = String::from_utf8_lossy(&blob);
        assert!(!haystack.contains("secret") && !haystack.contains("sk-live"));
        assert_eq!(blob[0], VERSION);
    }

    #[test]
    fn sealing_twice_gives_different_bytes_because_each_gets_a_fresh_nonce() {
        let v = vault();
        assert_ne!(v.seal("a", "same").unwrap(), v.seal("a", "same").unwrap());
    }

    #[test]
    fn a_value_moved_to_another_provider_does_not_open() {
        let v = vault();
        let blob = v.seal("openai", "sk-1").unwrap();
        assert!(matches!(v.open("anthropic", &blob), Err(Error::Unreadable)));
    }

    #[test]
    fn a_changed_or_truncated_value_does_not_open() {
        let v = vault();
        let mut blob = v.seal("openai", "sk-1").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(matches!(v.open("openai", &blob), Err(Error::Unreadable)));
        assert!(matches!(
            v.open("openai", &blob[..10]),
            Err(Error::Unreadable)
        ));
        assert!(matches!(v.open("openai", &[]), Err(Error::Unreadable)));
    }

    #[test]
    fn another_master_key_cannot_open_it() {
        let blob = vault().seal("openai", "sk-1").unwrap();
        assert!(matches!(
            Vault::fixed([8; 32]).open("openai", &blob),
            Err(Error::Unreadable)
        ));
    }

    #[test]
    fn an_unreachable_keyring_stores_nothing_and_says_why() {
        let err = Vault::unavailable().seal("openai", "sk-1").unwrap_err();
        assert!(err.to_string().contains("keyring is not available"));
    }

    /// Against the real keyring. Run with `cargo test -- --ignored`; it uses its own entry and
    /// removes it, so it never touches the app's master key.
    #[test]
    #[ignore = "needs a running Secret Service (gnome-keyring or kwallet)"]
    fn the_os_keyring_makes_the_master_key_once_and_returns_the_same_one_after() {
        let account = format!("test-master-key-{}", uuid::Uuid::new_v4());
        let first = Vault::with_source({
            let a = account.clone();
            move || os_master_key(&a)
        });
        let blob = first.seal("p", "sk-real-test").unwrap();
        let second = Vault::with_source({
            let a = account.clone();
            move || os_master_key(&a)
        });
        assert_eq!(
            &*second.open("p", &blob).unwrap(),
            "sk-real-test",
            "a second read gets the stored key"
        );
        keyring_core::Entry::new(SERVICE, &account)
            .unwrap()
            .delete_credential()
            .unwrap();
    }
}
