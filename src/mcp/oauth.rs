//! Signing in to a remote MCP server that asks for it (OAuth 2.1, as the MCP specification lays it
//! out): the server says where to sign in, this app registers itself there, opens the person's
//! browser, and receives the answer on a port of this computer that only this computer can reach.
//! The tokens that come back are sealed with the server's other secrets and renewed when due.
//!
//! The steps, all blocking, so run them off the UI thread:
//! 1. Discovery: the server (or the service it names) publishes its sign-in endpoints.
//! 2. Registration: this app is a new client there, with a redirect to its own loopback port.
//! 3. Authorization with PKCE: the browser goes to the server's consent page; the answer comes
//!    back to the loopback port with a code, and a `state` that must match what was sent.
//! 4. The code is traded for tokens.

use std::fmt;
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::loopback;
use crate::theme::Colors;

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
/// A token this close to running out is renewed before it is used.
const SLACK_SECS: i64 = 60;

/// What a sign-in gives, and what is needed to renew it later without asking again.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds since the epoch, when the access token runs out.
    #[serde(default)]
    pub expires_at: Option<i64>,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    pub token_endpoint: String,
}

// Tokens are secrets: never printed.
impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Tokens(..)")
    }
}

impl Tokens {
    pub fn expiring(&self) -> bool {
        self.expires_at
            .is_some_and(|t| t - SLACK_SECS <= crate::app::now())
    }
}

/// Where a server signs people in.
#[derive(Debug, PartialEq)]
struct Endpoints {
    authorization: String,
    token: String,
    registration: Option<String>,
}

/// Signs in to the server at `server_url`: opens the browser and waits for the person to finish.
/// The page the browser comes back to is drawn in `colors`, the app's own.
pub fn sign_in(server_url: &str, cancel: &AtomicBool, colors: Colors) -> Result<Tokens, String> {
    sign_in_with(server_url, cancel, colors, crate::browser::open)
}

/// [`sign_in`], with how the consent page is shown to the person chosen by the caller.
fn sign_in_with(
    server_url: &str,
    cancel: &AtomicBool,
    colors: Colors,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Tokens, String> {
    let ep = discover(server_url)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("could not listen for the sign-in to come back: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("could not listen for the sign-in to come back: {e}"))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let registration = ep.registration.as_deref().ok_or(
        "this server does not let apps register themselves, which is the only way this app signs in. Use a key instead, if the server gives them out.",
    )?;
    let (client_id, client_secret) = register(registration, &redirect)?;
    let verifier = random_token()?;
    let state = random_token()?;
    let url = format!(
        "{}{}{}",
        ep.authorization,
        if ep.authorization.contains('?') {
            '&'
        } else {
            '?'
        },
        form(&[
            ("response_type", "code"),
            ("client_id", &client_id),
            ("redirect_uri", &redirect),
            ("code_challenge", &challenge(&verifier)),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("resource", server_url),
        ])
    );
    open(&url)?;
    let code = loopback::wait_for_code(&listener, "/callback", &state, cancel, &colors)?;
    let mut pairs = vec![
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("client_id", client_id.as_str()),
        ("code_verifier", verifier.as_str()),
        ("resource", server_url),
    ];
    if let Some(secret) = &client_secret {
        pairs.push(("client_secret", secret));
    }
    let granted = post_form(&ep.token, &pairs)?;
    tokens_from(&granted, &client_id, client_secret, &ep.token, None)
}

/// Trades the refresh token for a new access token.
pub fn refresh(old: &Tokens) -> Result<Tokens, String> {
    let refresh = old
        .refresh_token
        .as_deref()
        .ok_or("there is no way to renew the sign-in")?;
    let mut pairs = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", old.client_id.as_str()),
    ];
    if let Some(secret) = &old.client_secret {
        pairs.push(("client_secret", secret));
    }
    let granted = post_form(&old.token_endpoint, &pairs)?;
    tokens_from(
        &granted,
        &old.client_id,
        old.client_secret.clone(),
        &old.token_endpoint,
        old.refresh_token.clone(),
    )
}

fn tokens_from(
    granted: &Value,
    client_id: &str,
    client_secret: Option<String>,
    token_endpoint: &str,
    keep_refresh: Option<String>,
) -> Result<Tokens, String> {
    let access_token = granted["access_token"]
        .as_str()
        .ok_or("the server's answer had no access token")?
        .to_owned();
    Ok(Tokens {
        access_token,
        refresh_token: granted["refresh_token"]
            .as_str()
            .map(str::to_owned)
            .or(keep_refresh),
        expires_at: granted["expires_in"]
            .as_i64()
            .map(|s| crate::app::now() + s),
        client_id: client_id.to_owned(),
        client_secret,
        token_endpoint: token_endpoint.to_owned(),
    })
}

// ---- discovery and registration -----------------------------------------------------------

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into()
}

fn get_json(url: &str) -> Option<Value> {
    let mut resp = agent()
        .get(url)
        .header("Accept", "application/json")
        .call()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    serde_json::from_str(&resp.body_mut().read_to_string().ok()?).ok()
}

/// `https://host:port` and the path after it, without a query.
fn split_url(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (host, path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
    (!host.is_empty()).then(|| {
        (
            format!("{scheme}://{host}"),
            path.trim_end_matches('/').to_owned(),
        )
    })
}

fn discover(server_url: &str) -> Result<Endpoints, String> {
    let (origin, path) = split_url(server_url).ok_or("the server's address is not a URL")?;
    // The server says which service signs people in; with no word, it is the server itself.
    let resource = [
        format!("{origin}/.well-known/oauth-protected-resource{path}"),
        format!("{origin}/.well-known/oauth-protected-resource"),
    ]
    .iter()
    .find_map(|u| get_json(u));
    let issuer = resource
        .as_ref()
        .and_then(|r| r["authorization_servers"][0].as_str())
        .unwrap_or(&origin)
        .to_owned();
    let (issuer_origin, issuer_path) =
        split_url(&issuer).ok_or("the sign-in service's address is not a URL")?;
    let mut candidates = Vec::new();
    for well_known in ["oauth-authorization-server", "openid-configuration"] {
        if !issuer_path.is_empty() {
            candidates.push(format!(
                "{issuer_origin}/.well-known/{well_known}{issuer_path}"
            ));
        }
        candidates.push(format!("{issuer_origin}/.well-known/{well_known}"));
    }
    candidates
        .iter()
        .filter_map(|u| get_json(u))
        .find_map(|meta| {
            Some(Endpoints {
                authorization: meta["authorization_endpoint"].as_str()?.to_owned(),
                token: meta["token_endpoint"].as_str()?.to_owned(),
                registration: meta["registration_endpoint"].as_str().map(str::to_owned),
            })
        })
        .ok_or_else(|| "the server does not say how to sign in to it.".to_owned())
}

fn register(endpoint: &str, redirect: &str) -> Result<(String, Option<String>), String> {
    let body = json!({
        "client_name": "Oido Desktop",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let mut resp = agent()
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .send(body.to_string().as_str())
        .map_err(|e| format!("the sign-in service could not be reached: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    match (status, v["client_id"].as_str()) {
        (200..=299, Some(id)) => Ok((
            id.to_owned(),
            v["client_secret"].as_str().map(str::to_owned),
        )),
        _ => Err(format!(
            "the sign-in service did not register this app{}",
            why(&v)
        )),
    }
}

/// `: <what the service said>`, or nothing.
fn why(v: &Value) -> String {
    v["error_description"]
        .as_str()
        .or_else(|| v["error"].as_str())
        .map_or_else(String::new, |s| format!(": {s}"))
}

fn post_form(url: &str, pairs: &[(&str, &str)]) -> Result<Value, String> {
    let mut resp = agent()
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .send(form(pairs).as_str())
        .map_err(|e| format!("the sign-in service could not be reached: {e}"))?;
    let ok = resp.status().is_success();
    let v: Value = serde_json::from_str(&resp.body_mut().read_to_string().unwrap_or_default())
        .unwrap_or(Value::Null);
    if ok {
        Ok(v)
    } else {
        Err(format!("the sign-in service refused{}", why(&v)))
    }
}

// ---- PKCE, encoding and randomness --------------------------------------------------------

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness for signing in: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Everything but the unreserved characters (RFC 3986) is escaped: names such as `response_type`
/// must reach the server as written, since not every server decodes them.
const ESCAPED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, ESCAPED).to_string()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    use super::*;
    use crate::loopback::decode;

    #[test]
    fn the_challenge_is_the_one_rfc_7636_gives_for_its_verifier() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn an_address_splits_into_its_origin_and_path() {
        assert_eq!(
            split_url("https://mcp.example.com:8443/a/mcp?x=1"),
            Some(("https://mcp.example.com:8443".into(), "/a/mcp".into()))
        );
        assert_eq!(
            split_url("https://mcp.example.com/"),
            Some(("https://mcp.example.com".into(), String::new()))
        );
        assert_eq!(split_url("not a url"), None);
    }

    #[test]
    fn a_form_is_encoded_and_a_query_decoded() {
        assert_eq!(form(&[("a b", "c&d")]), "a%20b=c%26d");
        assert_eq!(
            form(&[("response_type", "a-b.c_d~e")]),
            "response_type=a-b.c_d~e"
        );
    }

    #[test]
    fn signing_in_registers_asks_the_browser_trades_the_code_and_a_stale_token_is_renewed() {
        use super::super::Secrets;
        use super::super::Transport;
        use super::super::client::{
            Target, list_tools,
            tests::{fake_http, mcp_handler},
        };

        // One server plays the MCP server and its sign-in service.
        let me = std::sync::Arc::new(std::sync::OnceLock::<String>::new());
        let at = std::sync::Arc::clone(&me);
        let mcp = mcp_handler("at2");
        let base = fake_http(Box::new(move |req| {
            let line = req.lines().next().unwrap_or_default();
            let body = req.split("\r\n\r\n").nth(1).unwrap_or_default();
            let json = |v: Value| {
                (
                    200,
                    vec![("Content-Type", "application/json".to_owned())],
                    v.to_string(),
                )
            };
            if line.starts_with("GET /.well-known/oauth-authorization-server") {
                let me = at.get().unwrap();
                json(
                    json!({"authorization_endpoint": format!("{me}/authorize"), "token_endpoint": format!("{me}/token"), "registration_endpoint": format!("{me}/register")}),
                )
            } else if line.starts_with("POST /register") {
                assert!(
                    body.contains("redirect_uris") && body.contains("127.0.0.1"),
                    "{body}"
                );
                json(json!({"client_id": "cid"}))
            } else if line.starts_with("POST /token") {
                if body.contains("grant_type=authorization_code") {
                    assert!(
                        body.contains("code=CODE") && body.contains("code_verifier="),
                        "{body}"
                    );
                    json(json!({"access_token": "at1", "refresh_token": "rt1", "expires_in": 3600}))
                } else {
                    assert!(
                        body.contains("grant_type=refresh_token")
                            && body.contains("refresh_token=rt1"),
                        "{body}"
                    );
                    json(json!({"access_token": "at2", "expires_in": 3600}))
                }
            } else if line.starts_with("POST /mcp") {
                mcp(req)
            } else {
                (404, vec![], String::new())
            }
        }));
        me.set(base.clone()).unwrap();
        let url = format!("{base}/mcp");

        // The "browser" reads the consent address it is sent to, and comes back with a code.
        let tokens = sign_in_with(&url, &AtomicBool::new(false), Colors::oido(), |auth| {
            assert!(
                auth.contains("code_challenge_method=S256") && auth.contains("client_id=cid"),
                "{auth}"
            );
            let get = |key: &str| {
                auth.split(['?', '&'])
                    .find_map(|p| p.strip_prefix(&format!("{key}=")))
                    .map(decode)
                    .unwrap()
            };
            let (redirect, state) = (get("redirect_uri"), get("state"));
            let target = redirect.strip_prefix("http://").unwrap().to_owned();
            std::thread::spawn(move || {
                let (host, path) = target.split_once('/').unwrap();
                let mut s = TcpStream::connect(host).unwrap();
                write!(
                    s,
                    "GET /{path}?code=CODE&state={} HTTP/1.1\r\n\r\n",
                    enc(&state)
                )
                .unwrap();
                let mut said = String::new();
                let _ = s.read_to_string(&mut said);
            });
            Ok(())
        })
        .unwrap();
        assert_eq!(
            (
                tokens.access_token.as_str(),
                tokens.refresh_token.as_deref()
            ),
            ("at1", Some("rt1"))
        );

        // The saved sign-in has run out: the call renews it first and hands the new tokens back.
        let stale = Tokens {
            expires_at: Some(crate::app::now() - 10),
            ..tokens
        };
        let outcome = list_tools(
            Target {
                id: "t-oauth".into(),
                transport: Transport::Http { url },
                secrets: Secrets {
                    oauth: Some(stale),
                    ..Secrets::default()
                },
            },
            &AtomicBool::new(false),
        );
        assert_eq!(outcome.result.unwrap().len(), 1);
        let renewed = outcome.tokens.unwrap();
        assert_eq!(
            (
                renewed.access_token.as_str(),
                renewed.refresh_token.as_deref()
            ),
            ("at2", Some("rt1")),
            "the refresh token is kept when none is sent"
        );
    }

    #[test]
    fn tokens_expire_a_little_early_and_are_never_printed() {
        let t = |expires_at| Tokens {
            access_token: "secret".into(),
            refresh_token: None,
            expires_at,
            client_id: "c".into(),
            client_secret: None,
            token_endpoint: "https://x/token".into(),
        };
        let now = crate::app::now();
        assert!(t(Some(now + 10)).expiring());
        assert!(!t(Some(now + 3600)).expiring());
        assert!(!t(None).expiring());
        assert!(!format!("{:?}", t(None)).contains("secret"));
    }
}
