//! The way back from a browser sign-in: a port of this computer that only this computer can reach,
//! where the browser lands with a code once the person has approved, and the page it is answered
//! with. MCP servers and ChatGPT both sign in this way.

mod page;

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use percent_encoding::percent_decode_str;

use crate::theme::Colors;

/// How long the person has to finish signing in.
const WAIT: Duration = Duration::from_secs(300);

/// Reads a query string's value: `+` is a space and `%XX` is a byte.
pub fn decode(s: &str) -> String {
    percent_decode_str(&s.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

/// Waits for the browser to come back to `path` on the loopback port with the code, up to [`WAIT`].
/// Only a request that carries `state` counts; anything else is answered and the wait goes on.
pub fn wait_for_code(
    listener: &TcpListener,
    path: &str,
    state: &str,
    cancel: &AtomicBool,
    colors: &Colors,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("could not wait for the sign-in: {e}"))?;
    let deadline = Instant::now() + WAIT;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("sign-in was cancelled".to_owned());
        }
        if Instant::now() > deadline {
            return Err("sign-in was not finished in time".to_owned());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = answer(stream, path, state, colors) {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("could not wait for the sign-in: {e}")),
        }
    }
}

/// Reads one request to the loopback port. `None` when it is not the way back (a favicon, a
/// stray request, a wrong `state`): the wait goes on.
fn answer(
    mut stream: TcpStream,
    path: &str,
    state: &str,
    colors: &Colors,
) -> Option<Result<String, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while head.len() < 8192 && !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let head = String::from_utf8_lossy(&head);
    let target = head.lines().next()?.split_whitespace().nth(1)?;
    let (at, query) = target.split_once('?').unwrap_or((target, ""));
    let get = |key: &str| {
        query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| decode(v))
    };
    if at != path {
        page::NOT_FOUND.send(&mut stream, colors);
        return None;
    }
    if get("state").as_deref() != Some(state) {
        page::NOT_OURS.send(&mut stream, colors);
        return None;
    }
    if let Some(error) = get("error") {
        page::REFUSED.send(&mut stream, colors);
        return Some(Err(format!("the sign-in was refused: {error}")));
    }
    match get("code") {
        Some(code) => {
            page::SIGNED_IN.send(&mut stream, colors);
            Some(Ok(code))
        }
        None => {
            page::NO_CODE.send(&mut stream, colors);
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn a_query_is_decoded() {
        assert_eq!(decode("a%20b+c"), "a b c");
    }

    /// Sends `target` to the listener as a browser would, and gives what the wait made of it.
    fn come_back(target: &str, state: &str) -> Result<String, String> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let target = target.to_owned();
        let browser = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut said = String::new();
            let _ = s.read_to_string(&mut said);
            said
        });
        let cancel = AtomicBool::new(false);
        let got = std::thread::scope(|scope| {
            // A wrong-state request must not end the wait; a cancel does.
            let waiting = scope
                .spawn(|| wait_for_code(&listener, "/callback", state, &cancel, &Colors::oido()));
            std::thread::sleep(Duration::from_millis(600));
            cancel.store(true, Ordering::Relaxed);
            waiting.join().unwrap()
        });
        let said = browser.join().unwrap();
        assert!(said.starts_with("HTTP/1.1"), "{said}");
        // Whatever the answer, the page is Oido's.
        assert!(said.contains(&Colors::oido().bg.hex()), "{said}");
        got
    }

    #[test]
    fn the_way_back_gives_the_code_only_with_the_state_that_was_sent() {
        assert_eq!(
            come_back("/callback?code=abc%2F1&state=s1", "s1").unwrap(),
            "abc/1"
        );
        let forged = come_back("/callback?code=evil&state=other", "s1").unwrap_err();
        assert!(forged.contains("cancelled"), "{forged}");
        let refused = come_back("/callback?error=access_denied&state=s1", "s1").unwrap_err();
        assert!(refused.contains("access_denied"), "{refused}");
        let elsewhere = come_back("/other?code=abc&state=s1", "s1").unwrap_err();
        assert!(
            elsewhere.contains("cancelled"),
            "only the callback path counts"
        );
    }
}
