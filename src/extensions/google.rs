//! The Google account the extensions share: the OAuth client the person made in Google Cloud, the
//! sign-in it gave, and how a call is made with it. One sign-in covers every Google extension;
//! what it may do is the union of their scopes, asked for again when another is turned on.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Extension;
use crate::mcp::client::Outcome;
use crate::mcp::oauth::{self, Tokens};

/// What the account is stored under.
pub const PROVIDER: &str = "google";
/// Asked for with every sign-in, so the page can say whose account it is.
pub const EMAIL_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.email";
const USERINFO: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const REVOKE: &str = "https://oauth2.googleapis.com/revoke";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// A tool's output is cut to this many bytes before the model sees it.
const OUTPUT_LIMIT: usize = 32 * 1024;
const SIGN_IN_AGAIN: &str =
    "Google no longer accepts the sign-in. Ask the person to sign in again on the Extensions page.";

/// Everything sealed about the account.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Account {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub tokens: Option<Tokens>,
    #[serde(default)]
    pub email: Option<String>,
    /// What Google said the sign-in allows.
    #[serde(default)]
    pub scopes: Vec<String>,
}

// An account is a secret: never printed.
impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Account(..)")
    }
}

impl Account {
    pub fn has_client(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }

    /// Forgets the sign-in but keeps the client, so signing in again needs nothing pasted.
    pub fn sign_out(&mut self) {
        self.tokens = None;
        self.email = None;
        self.scopes.clear();
    }
}

/// Why a request did not give an answer.
#[derive(Debug, PartialEq)]
pub enum Failure {
    /// Google does not accept the access token.
    Unauthorized,
    Other(String),
}

impl Failure {
    pub fn other(why: impl Into<String>) -> Failure {
        Failure::Other(why.into())
    }
}

/// One signed-in way to ask Google things.
pub struct Api<'a> {
    token: &'a str,
    cancel: &'a AtomicBool,
}

impl Api<'_> {
    pub fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<Value, Failure> {
        self.json(Method::Get, url, query, None)
    }

    /// A GET whose answer is a file's own bytes read as text, not JSON.
    pub fn get_text(&self, url: &str, query: &[(&str, &str)]) -> Result<String, Failure> {
        self.raw(Method::Get, url, query, None)
    }

    pub fn post(&self, url: &str, query: &[(&str, &str)], body: &Value) -> Result<Value, Failure> {
        self.json(Method::Post, url, query, Some(body))
    }

    /// A POST of `body` as `content_type` (a multipart upload), answered in JSON.
    pub fn post_raw(
        &self,
        url: &str,
        query: &[(&str, &str)],
        content_type: &str,
        body: String,
    ) -> Result<Value, Failure> {
        let text = self.raw(Method::Post, url, query, Some((content_type, body)))?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub fn patch(&self, url: &str, query: &[(&str, &str)], body: &Value) -> Result<Value, Failure> {
        self.json(Method::Patch, url, query, Some(body))
    }

    /// A PUT of `body` as JSON, which replaces what is there.
    pub fn put(&self, url: &str, query: &[(&str, &str)], body: &Value) -> Result<Value, Failure> {
        self.json(Method::Put, url, query, Some(body))
    }

    /// A PATCH of `body` as `content_type` (new content for a file), answered in JSON.
    pub fn patch_raw(
        &self,
        url: &str,
        query: &[(&str, &str)],
        content_type: &str,
        body: String,
    ) -> Result<Value, Failure> {
        let text = self.raw(Method::Patch, url, query, Some((content_type, body)))?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub fn delete(&self, url: &str, query: &[(&str, &str)]) -> Result<Value, Failure> {
        self.json(Method::Delete, url, query, None)
    }

    fn json(
        &self,
        method: Method,
        url: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value, Failure> {
        let body = body.map(|b| ("application/json", b.to_string()));
        let text = self.raw(method, url, query, body)?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Sends one request and gives the answer's body. A failed read is tried once more after a
    /// pause when Google is busy; a write is not, since a second send could do it twice.
    fn raw(
        &self,
        method: Method,
        url: &str,
        query: &[(&str, &str)],
        body: Option<(&str, String)>,
    ) -> Result<String, Failure> {
        let bearer = format!("Bearer {}", self.token);
        for attempt in 0..2 {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(Failure::other("stopped"));
            }
            let ask = agent();
            let sent = match (method, &body) {
                (Method::Get, _) => ask
                    .get(url)
                    .query_pairs(query.iter().copied())
                    .header("Authorization", &bearer)
                    .call(),
                (Method::Delete, _) => ask
                    .delete(url)
                    .query_pairs(query.iter().copied())
                    .header("Authorization", &bearer)
                    .call(),
                (Method::Post, Some((kind, text))) => ask
                    .post(url)
                    .query_pairs(query.iter().copied())
                    .header("Authorization", &bearer)
                    .header("Content-Type", *kind)
                    .send(text.as_str()),
                (Method::Put, Some((kind, text))) => ask
                    .put(url)
                    .query_pairs(query.iter().copied())
                    .header("Authorization", &bearer)
                    .header("Content-Type", *kind)
                    .send(text.as_str()),
                (Method::Patch, Some((kind, text))) => ask
                    .patch(url)
                    .query_pairs(query.iter().copied())
                    .header("Authorization", &bearer)
                    .header("Content-Type", *kind)
                    .send(text.as_str()),
                (Method::Post | Method::Put | Method::Patch, None) => {
                    return Err(Failure::other("a write was sent with nothing in it"));
                }
            };
            let mut resp =
                sent.map_err(|e| Failure::other(format!("Google could not be reached: {e}")))?;
            let status = resp.status().as_u16();
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| Failure::other(format!("Google's answer could not be read: {e}")))?;
            match status {
                200..=299 => return Ok(text),
                401 => return Err(Failure::Unauthorized),
                429 | 500..=503 if attempt == 0 && method == Method::Get => {
                    std::thread::sleep(Duration::from_secs(1));
                }
                _ => {
                    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                    return Err(Failure::other(refusal(status, &v)));
                }
            }
        }
        Err(Failure::other("Google did not answer."))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// `s` as one piece of an address path: everything but unreserved characters is escaped, so an
/// id from the model cannot reach another address.
pub fn segment(s: &str) -> String {
    const ESCAPED: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(s, ESCAPED).to_string()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into()
}

/// What Google refused with, in words.
fn refusal(status: u16, v: &Value) -> String {
    let said = v["error"]["message"].as_str().unwrap_or_default();
    match status {
        403 if v["error"]["errors"][0]["reason"].as_str() == Some("insufficientPermissions")
            || said.contains("insufficient authentication scopes") =>
        {
            "this sign-in does not allow that. Ask the person to sign in again on the Extensions page and allow it.".to_owned()
        }
        429 => "Google's rate limit was reached. Try again in a minute.".to_owned(),
        404 => format!("Google could not find it ({said})"),
        _ if said.is_empty() => format!("Google refused (status {status})"),
        _ => format!("Google refused: {said}"),
    }
}

/// Runs one tool of `ext` with `account`'s sign-in, renewing it when it is due or refused. A
/// renewed sign-in comes back to be saved.
pub fn call(
    ext: &dyn Extension,
    tool: &str,
    args: &Value,
    mut account: Account,
    cancel: &AtomicBool,
) -> Outcome<String> {
    let mut renewed = None;
    let mut force = false;
    for attempt in 0..2 {
        if let Some(old) = account.tokens.clone()
            && (force || old.expiring())
            && old.refresh_token.is_some()
        {
            // If it cannot be renewed the old token may still be good; if not, the 401 says so.
            if let Ok(fresh) = oauth::refresh(&old) {
                account.tokens = Some(fresh.clone());
                renewed = Some(fresh);
            }
        }
        let Some(tokens) = &account.tokens else {
            return Outcome {
                result: Err(SIGN_IN_AGAIN.to_owned()),
                tokens: renewed,
            };
        };
        let api = Api {
            token: &tokens.access_token,
            cancel,
        };
        match ext.call(tool, args, &api) {
            Err(Failure::Unauthorized) if attempt == 0 && tokens.refresh_token.is_some() => {
                force = true;
            }
            Err(Failure::Unauthorized) => {
                return Outcome {
                    result: Err(SIGN_IN_AGAIN.to_owned()),
                    tokens: renewed,
                };
            }
            Err(Failure::Other(why)) => {
                return Outcome {
                    result: Err(why),
                    tokens: renewed,
                };
            }
            Ok(text) => {
                return Outcome {
                    result: Ok(cut(text)),
                    tokens: renewed,
                };
            }
        }
    }
    Outcome {
        result: Err(SIGN_IN_AGAIN.to_owned()),
        tokens: renewed,
    }
}

/// `text` cut to what the model is given, and told so.
fn cut(mut text: String) -> String {
    if text.len() > OUTPUT_LIMIT {
        let mut end = OUTPUT_LIMIT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(
            "\n[Cut here: the rest was too long. Ask for less, or for a narrower search.]",
        );
    }
    text
}

/// The address a fresh sign-in belongs to, if Google says.
pub fn email_of(tokens: &Tokens) -> Option<String> {
    let mut resp = agent()
        .get(USERINFO)
        .header("Authorization", format!("Bearer {}", tokens.access_token))
        .call()
        .ok()?;
    let v: Value = serde_json::from_str(&resp.body_mut().read_to_string().ok()?).ok()?;
    v["email"].as_str().map(str::to_owned)
}

/// Tells Google to forget the sign-in. Best effort: the account is signed out here either way.
pub fn revoke(tokens: &Tokens) {
    let token = tokens
        .refresh_token
        .as_deref()
        .unwrap_or(&tokens.access_token);
    let _ = agent().post(REVOKE).query("token", token).send_empty();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_output_is_cut_on_a_character_and_says_so() {
        let text = "é".repeat(OUTPUT_LIMIT);
        let out = cut(text);
        assert!(out.len() < OUTPUT_LIMIT + 200);
        assert!(out.ends_with("narrower search.]"));
        assert_eq!(cut("short".into()), "short");
    }

    #[test]
    fn signing_out_keeps_the_client_and_forgets_the_sign_in() {
        let mut a = Account {
            client_id: "id".into(),
            client_secret: "secret".into(),
            ..Account::default()
        };
        assert!(a.has_client());
        a.tokens = Some(Tokens {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: None,
            client_id: "id".into(),
            client_secret: None,
            token_endpoint: String::new(),
        });
        a.scopes = vec!["a".into(), "b".into()];
        a.sign_out();
        assert!(a.has_client() && a.tokens.is_none() && a.scopes.is_empty());
    }

    #[test]
    fn an_account_never_prints_its_secrets() {
        let a = Account {
            client_secret: "hunter2".into(),
            ..Account::default()
        };
        assert!(!format!("{a:?}").contains("hunter2"));
    }

    #[test]
    fn a_path_piece_cannot_leave_its_place() {
        assert_eq!(segment("me@x.org"), "me%40x.org");
        assert_eq!(segment("a/../b?c#d"), "a%2F..%2Fb%3Fc%23d");
        assert_eq!(
            segment("en.usa#holiday@group.v.calendar.google.com"),
            "en.usa%23holiday%40group.v.calendar.google.com"
        );
    }

    #[test]
    fn refusals_are_said_in_words() {
        let missing = serde_json::json!({"error": {"message": "Requested entity was not found."}});
        assert!(refusal(404, &missing).contains("could not find it"));
        let scope = serde_json::json!({"error": {"message": "Request had insufficient authentication scopes."}});
        assert!(refusal(403, &scope).contains("sign in again"));
        assert!(refusal(429, &Value::Null).contains("rate limit"));
        assert_eq!(refusal(500, &Value::Null), "Google refused (status 500)");
    }
}
