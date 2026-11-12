//! What a provider is asked with: its own API key, or a Claude Pro/Max subscription's OAuth token,
//! and the sign-in and refresh that keep a subscription current.
//!
//! The subscription signs in as Claude Code's OAuth client, because Anthropic routes that client to
//! subscription inference. The sign-in is a PKCE flow with no local listener: the browser lands on
//! Anthropic's callback page, which shows a code to paste back, so the app needs no port.

use std::fmt;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Claude Code's OAuth client: the one Anthropic lets spend a Claude Pro/Max subscription.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE: &str = "https://claude.ai/oauth/authorize";
const REDIRECT: &str = "https://console.anthropic.com/oauth/code/callback";
const SCOPES: &str = "org:create_api_key user:profile user:inference";
/// The live token host, then the one that still answers if it moves.
const TOKEN_URLS: [&str; 2] = [
    "https://platform.claude.com/v1/oauth/token",
    "https://console.anthropic.com/v1/oauth/token",
];
/// Anthropic 429s token requests that introduce themselves as `claude-code/`; Claude Code's own
/// token client sends bare axios.
const TOKEN_USER_AGENT: &str = "axios/1.7.9";
/// OAuth inference must introduce itself as Claude Code, and say it is the CLI.
pub const CLAUDE_CODE_USER_AGENT: &str = "claude-code/2.1.74 (external, cli)";
/// The betas an OAuth request needs; without them Anthropic refuses the call.
pub const OAUTH_BETA: &str = "interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14,claude-code-20250219,oauth-2025-04-20";
/// The first system line an OAuth request carries; Anthropic intermittently fails without it.
pub const CLAUDE_CODE_SYSTEM: &str = "You are Claude Code, Anthropic's official CLI for Claude.";
/// On a subscription Claude Code only runs tools under this namespace, so every tool name is sent
/// with it and the prefix is stripped again on the way back.
pub const TOOL_PREFIX: &str = "mcp__";

const TIMEOUT: Duration = Duration::from_secs(20);

/// How a provider is asked: with its own key, or with a subscription's access token.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    /// A provider's own API key, sent the way that provider expects it.
    Key(Zeroizing<String>),
    /// A Claude Pro/Max subscription's OAuth access token.
    Subscription(Zeroizing<String>),
}

impl Credential {
    /// Shorthand for tests: a provider key from a borrowed string.
    #[cfg(test)]
    pub fn key(token: impl Into<String>) -> Credential {
        Self::Key(Zeroizing::new(token.into()))
    }

    pub fn subscription(token: impl Into<String>) -> Credential {
        Self::Subscription(Zeroizing::new(token.into()))
    }

    pub fn token(&self) -> &str {
        match self {
            Self::Key(token) | Self::Subscription(token) => token.as_str(),
        }
    }

    pub fn is_subscription(&self) -> bool {
        matches!(self, Self::Subscription(_))
    }
}

/// Never print a token, not even in a panic message.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Key(_) => "Key(…)",
            Self::Subscription(_) => "Subscription(…)",
        })
    }
}

/// What the token endpoints hand back, and what a refresh keeps for next time.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
}

/// Never print a token, not even in a panic message.
impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"…")
            .field("refresh_token", &"…")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Tokens {
    /// The credential a request is made with: only the access token leaves the store.
    pub fn credential(&self) -> Credential {
        Credential::subscription(self.access_token.clone())
    }

    pub fn json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|e| format!("the sign-in could not be stored ({e}); try signing in again."))
    }

    pub fn parse(text: &str) -> Result<Tokens, String> {
        serde_json::from_str(text)
            .map_err(|e| format!("the saved sign-in could not be read ({e}); sign in again."))
    }

    /// True once the access token is close enough to expiry that a request may see it run out.
    pub fn is_stale(&self, now: i64) -> bool {
        self.expires_at - 60 <= now
    }
}

/// A sign-in in progress: what the browser is asked to approve, and what proves the code that comes
/// back belongs to this request.
pub struct SignIn {
    pub verifier: String,
    pub state: String,
    pub url: String,
}

/// Starts a subscription sign-in: a PKCE verifier and the address to approve it at.
pub fn sign_in() -> Result<SignIn, String> {
    let verifier = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = URL_SAFE_NO_PAD.encode(random::<32>()?);
    let url = format!(
        "{AUTHORIZE}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        encode(CLIENT_ID),
        encode(REDIRECT),
        encode(SCOPES),
    );
    Ok(SignIn {
        verifier,
        state,
        url,
    })
}

/// Trades the code the browser showed for tokens. `state` must be the one [`sign_in`] made.
pub fn exchange(code: &str, state: &str, verifier: &str) -> Result<Tokens, String> {
    let body = json!({
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "code": code,
        "state": state,
        "redirect_uri": REDIRECT,
        "code_verifier": verifier,
    });
    post(&body.to_string(), "application/json", "sign-in")
}

/// Replaces an expiring access token. The refresh token may rotate; the new pair must be saved, or
/// the next refresh spends a token that is already gone.
pub fn refresh(tokens: &Tokens) -> Result<Tokens, String> {
    if tokens.refresh_token.is_empty() {
        return Err("the sign-in has no refresh token; sign in again".to_owned());
    }
    // As Claude Code asks: form-encoded, unlike the sign-in's JSON.
    let body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={CLIENT_ID}",
        encode(&tokens.refresh_token),
    );
    let mut fresh = post(&body, "application/x-www-form-urlencoded", "refresh")?;
    if fresh.refresh_token.is_empty() {
        fresh.refresh_token = tokens.refresh_token.clone();
    }
    Ok(fresh)
}

/// Posts to the token hosts in order. A dead grant is not replayed at the second host.
fn post(body: &str, content_type: &str, what: &str) -> Result<Tokens, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut last = String::new();
    for url in TOKEN_URLS {
        let sent = agent
            .post(url)
            .header("Content-Type", content_type)
            .header("User-Agent", TOKEN_USER_AGENT)
            .send(body);
        let mut resp = match sent {
            Ok(resp) => resp,
            Err(_) => {
                last = "could not reach Anthropic (check the connection and try again)".to_owned();
                continue;
            }
        };
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        if (200..300).contains(&status) {
            return tokens_from(&text, what);
        }
        last = refused(status, &text, what);
        if status == 400 || status == 401 {
            break;
        }
    }
    Err(last)
}

/// The token endpoint's answer, as tokens. `expires_in` is taken as seconds from now.
fn tokens_from(text: &str, what: &str) -> Result<Tokens, String> {
    let v: Value = serde_json::from_str(text)
        .map_err(|_| format!("Anthropic's {what} answer was not understood; try again."))?;
    let access = v
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if access.is_empty() {
        return Err(format!(
            "Anthropic's {what} answer had no access token; try again."
        ));
    }
    let refresh = v
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let secs = v
        .get("expires_in")
        .and_then(Value::as_i64)
        .unwrap_or(3600)
        .max(60);
    Ok(Tokens {
        access_token: access.to_owned(),
        refresh_token: refresh.to_owned(),
        expires_at: chrono::Utc::now().timestamp() + secs,
    })
}

/// Why a token request was refused, in Anthropic's own words where it gave them.
fn refused(status: u16, body: &str, what: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v.get("error").and_then(Value::as_str).unwrap_or_default();
    let detail = v
        .get("error_description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let said = [code, detail]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(": ");
    if said.is_empty() {
        format!("Anthropic refused the {what} (HTTP {status}); try again.")
    } else {
        format!("Anthropic refused the {what} (HTTP {status}): {said}")
    }
}

/// RFC 3986's unreserved marks stay as they are, so ids and scopes read as they were written.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode(s: &str) -> String {
    utf8_percent_encode(s, QUERY).to_string()
}

fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness for the sign-in ({e})"))?;
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_in_asks_for_claude_codes_scope_and_carries_its_own_challenge() {
        let first = sign_in().unwrap();
        let second = sign_in().unwrap();
        assert!(first.url.starts_with(AUTHORIZE));
        assert!(first.url.contains("code_challenge_method=S256"));
        assert!(
            first
                .url
                .contains("client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e")
        );
        assert!(first.url.contains("state="));
        assert_ne!(first.state, second.state, "a fresh state each time");
        assert_ne!(first.verifier, second.verifier);
        assert!(!first.verifier.contains('='), "unpadded, as PKCE wants");
    }

    #[test]
    fn tokens_round_trip_through_json_and_read_as_a_subscription() {
        let tokens = Tokens {
            access_token: "sk-ant-oat-1".into(),
            refresh_token: "refresh-1".into(),
            expires_at: 1_700_000_000,
        };
        let read = Tokens::parse(&tokens.json().unwrap()).unwrap();
        assert_eq!(read, tokens);
        assert_eq!(read.credential().token(), "sk-ant-oat-1");
        assert!(read.credential().is_subscription());
    }

    #[test]
    fn a_token_is_stale_a_minute_before_it_expires() {
        let mut tokens = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 1_000,
        };
        assert!(tokens.is_stale(999));
        assert!(tokens.is_stale(940));
        assert!(!tokens.is_stale(939));
        tokens.expires_at = 0;
        assert!(tokens.is_stale(0));
    }

    #[test]
    fn a_refusal_keeps_anthropics_words() {
        let why = refused(
            400,
            r#"{"error":"invalid_grant","error_description":"code expired"}"#,
            "sign-in",
        );
        assert!(
            why.contains("invalid_grant") && why.contains("code expired"),
            "{why}"
        );
    }

    #[test]
    fn debug_never_prints_a_credential() {
        let shown = format!("{:?}", Credential::key("sk-secret-value"));
        assert!(!shown.contains("sk-secret-value"), "{shown}");
    }
}
