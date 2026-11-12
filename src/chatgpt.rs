//! Signing in with a ChatGPT plan (Plus, Pro, Business, Edu or Enterprise), and what a request made
//! with that sign-in carries. It is how Codex, OpenAI's coding agent, is paid for: the plan's usage
//! covers the calls instead of an API key's billing.
//!
//! The sign-in is Codex's own OAuth client, the one OpenAI lets spend a plan. That client is only
//! registered with a redirect to port 1455 of this computer, so the browser comes back there (see
//! [`crate::loopback`]) and nothing has to be pasted.
//!
//! The plan is spent at ChatGPT's Codex backend, not at `api.openai.com`; neither OpenAI's API nor
//! a key is involved. That backend is not documented by OpenAI, so what is sent here follows what
//! the Codex CLI sends.

use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

use crate::credentials::{self, Pkce, SignIn, TokenHost, Tokens, encode};
use crate::loopback;
use crate::theme::Colors;

/// The Codex CLI's OAuth client.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE: &str = "https://auth.openai.com/oauth/authorize";
/// The only port the client's redirect is registered for.
const PORT: u16 = 1455;
const CALLBACK: &str = "/auth/callback";
const REDIRECT: &str = "http://localhost:1455/auth/callback";
const SCOPES: &str = "openid profile email offline_access";
const OPENAI: TokenHost = TokenHost {
    who: "OpenAI",
    urls: &["https://auth.openai.com/oauth/token"],
    user_agent: "oido-desktop",
};

/// Where the Codex backend answers, for the model list and for chat.
pub const BASE: &str = "https://chatgpt.com/backend-api/codex";
/// What the backend is told the client is: it only serves clients it knows, and answers the model
/// list with what that client can run.
const ORIGINATOR: &str = "codex_cli_rs";
/// The Codex CLI version the model list is asked for; the backend hides models a client that old
/// cannot run.
pub const CLIENT_VERSION: &str = "0.167.2";

/// Starts a sign-in: the port the browser will come back to, and the address to approve it at. The
/// port is taken now, so a port that is in use is said so before the browser opens.
pub fn begin() -> Result<(SignIn, TcpListener), String> {
    let listener = bind(PORT)?;
    let pkce = Pkce::new()?;
    let url = authorize_url(&pkce);
    Ok((
        SignIn {
            verifier: pkce.verifier,
            state: pkce.state,
            url,
        },
        listener,
    ))
}

/// Waits for the browser to come back with the code, then trades it for tokens. Blocks, so call it
/// from a worker thread; `cancel` ends the wait.
pub fn finish(
    listener: &TcpListener,
    sign: &SignIn,
    cancel: &AtomicBool,
    colors: &Colors,
) -> Result<Tokens, String> {
    let code = loopback::wait_for_code(listener, CALLBACK, &sign.state, cancel, colors)?;
    trade(&OPENAI, &code, &sign.verifier)
}

/// Replaces an expiring access token. The refresh token rotates: the new pair must be saved.
pub fn refresh(tokens: &Tokens) -> Result<Tokens, String> {
    refresh_at(&OPENAI, tokens)
}

fn refresh_at(host: &TokenHost, tokens: &Tokens) -> Result<Tokens, String> {
    let body = json!({
        "client_id": CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": tokens.refresh_token,
    });
    credentials::renew(host, tokens, &body.to_string(), "application/json")
}

/// What a request to the Codex backend carries: the access token, and the account it is the plan
/// of. A token that does not name its account is sent without it, and the backend refuses it.
pub fn headers(token: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("Authorization", format!("Bearer {token}")),
        ("OpenAI-Beta", "responses=experimental".to_owned()),
        ("originator", ORIGINATOR.to_owned()),
    ];
    if let Some(account) = account_id(token) {
        headers.push(("chatgpt-account-id", account));
    }
    headers
}

/// Whether `token` says which ChatGPT account it belongs to, as the backend needs.
pub fn names_an_account(token: &str) -> bool {
    account_id(token).is_some()
}

fn bind(port: u16) -> Result<TcpListener, String> {
    // A sign-in that was just cancelled gives its port back within a moment.
    let mut last = None;
    for _ in 0..4 {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok(listener),
            Err(e) => last = Some(e),
        }
        thread::sleep(Duration::from_millis(100));
    }
    let why = last.map(|e| e.to_string()).unwrap_or_default();
    Err(format!(
        "port {port} of this computer is in use, and the ChatGPT sign-in can only come back there ({why}). Close whatever is using it (a Codex CLI sign-in waiting in a terminal, for one) and try again."
    ))
}

fn authorize_url(pkce: &Pkce) -> String {
    format!(
        "{AUTHORIZE}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator={ORIGINATOR}",
        encode(CLIENT_ID),
        encode(REDIRECT),
        encode(SCOPES),
        pkce.challenge,
        pkce.state,
    )
}

/// Trades the code the browser came back with for tokens at `host`.
fn trade(host: &TokenHost, code: &str, verifier: &str) -> Result<Tokens, String> {
    let body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        encode(code),
        encode(REDIRECT),
        encode(CLIENT_ID),
        encode(verifier),
    );
    let tokens = credentials::post(host, &body, "application/x-www-form-urlencoded", "sign-in")?;
    if !names_an_account(&tokens.access_token) {
        return Err(
            "OpenAI did not say which ChatGPT account this is, so it cannot be used. Try signing in again."
                .to_owned(),
        );
    }
    Ok(tokens)
}

/// The ChatGPT account an access token is for: a claim in the token, which is a JWT.
fn account_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let json = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: Value = serde_json::from_slice(&json).ok()?;
    claims
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing::{access_token, serve};

    #[test]
    fn the_address_to_approve_asks_as_codex_does_and_carries_this_sign_ins_own_challenge() {
        let (first, second) = (Pkce::new().unwrap(), Pkce::new().unwrap());
        let url = authorize_url(&first);
        assert!(
            url.starts_with("https://auth.openai.com/oauth/authorize?"),
            "{url}"
        );
        for part in [
            "client_id=app_EMoamEEZ73f0CkXaXp7hrann",
            "redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback",
            "scope=openid%20profile%20email%20offline_access",
            "code_challenge_method=S256",
            "codex_cli_simplified_flow=true",
            "id_token_add_organizations=true",
        ] {
            assert!(url.contains(part), "{part} in {url}");
        }
        assert!(url.contains(&format!("code_challenge={}", first.challenge)));
        assert!(url.contains(&format!("state={}", first.state)));
        assert_ne!(first.state, second.state, "a fresh state each time");
    }

    #[test]
    fn a_taken_port_is_said_so_with_what_to_do() {
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        let why = bind(port).unwrap_err();
        assert!(
            why.contains(&port.to_string()) && why.contains("in use"),
            "{why}"
        );
        drop(held);
        assert!(bind(port).is_ok(), "free again once it is let go");
    }

    #[test]
    fn a_request_names_the_account_the_token_belongs_to() {
        let token = access_token(Some("acct-1"));
        let headers = headers(&token);
        let get = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(
            get("Authorization"),
            Some(format!("Bearer {token}").as_str())
        );
        assert_eq!(get("chatgpt-account-id"), Some("acct-1"));
        assert_eq!(get("OpenAI-Beta"), Some("responses=experimental"));

        for opaque in [
            access_token(None),
            "not-a-jwt".to_owned(),
            "a.!!.c".to_owned(),
        ] {
            assert!(!names_an_account(&opaque), "{opaque}");
            assert!(headers_without_account(&opaque), "{opaque}");
        }
    }

    fn headers_without_account(token: &str) -> bool {
        headers(token)
            .iter()
            .all(|(n, _)| *n != "chatgpt-account-id")
    }

    #[test]
    fn the_code_is_traded_for_tokens_and_a_token_with_no_account_is_not_kept() {
        let good = Box::leak(
            json!({"access_token": access_token(Some("acct-1")), "refresh_token": "r-1", "expires_in": 600})
                .to_string()
                .into_boxed_str(),
        );
        let opaque = Box::leak(
            json!({"access_token": access_token(None), "refresh_token": "r-1"})
                .to_string()
                .into_boxed_str(),
        );
        let (base, seen) = serve(vec![(200, good), (200, opaque)]);
        let urls: &'static [&'static str] = Box::leak(
            vec![&*Box::leak(format!("{base}/oauth/token").into_boxed_str())].into_boxed_slice(),
        );
        let host = TokenHost {
            who: "OpenAI",
            urls,
            user_agent: "test",
        };

        let tokens = trade(&host, "the code/1", "the-verifier").unwrap();
        assert_eq!(tokens.refresh_token, "r-1");
        assert!(tokens.expires_at > crate::app::now());
        let sent = seen.recv().unwrap();
        assert!(sent.contains("grant_type=authorization_code"), "{sent}");
        assert!(sent.contains("code=the%20code%2f1"), "{sent}");
        assert!(sent.contains("code_verifier=the-verifier"), "{sent}");
        assert!(sent.contains("application/x-www-form-urlencoded"), "{sent}");

        let why = trade(&host, "c", "v").unwrap_err();
        assert!(why.contains("which ChatGPT account"), "{why}");
    }

    #[test]
    fn a_refresh_is_json_and_keeps_the_old_refresh_token_when_none_comes_back() {
        let fresh = Box::leak(
            json!({"access_token": access_token(Some("acct-1")), "expires_in": 600})
                .to_string()
                .into_boxed_str(),
        );
        let (base, seen) = serve(vec![(200, fresh)]);
        let urls: &'static [&'static str] = Box::leak(
            vec![&*Box::leak(format!("{base}/oauth/token").into_boxed_str())].into_boxed_slice(),
        );
        let host = TokenHost {
            who: "OpenAI",
            urls,
            user_agent: "test",
        };
        let old = Tokens {
            access_token: "old".into(),
            refresh_token: "r-old".into(),
            expires_at: 0,
        };
        let next = refresh_at(&host, &old).unwrap();
        assert_eq!(next.refresh_token, "r-old");
        let sent = seen.recv().unwrap();
        assert!(
            sent.contains("application/json")
                && sent.contains("\"grant_type\":\"refresh_token\"")
                && sent.contains("\"refresh_token\":\"r-old\""),
            "{sent}"
        );
    }
}
