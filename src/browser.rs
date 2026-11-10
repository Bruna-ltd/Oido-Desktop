//! Opening web pages in the person's browser.

use std::process::{Command, Stdio};

/// Opens `url` in the default browser, through `xdg-open`.
///
/// # Errors
///
/// When `xdg-open` cannot be started, in words.
pub fn open(url: &str) -> Result<(), String> {
    let mut child = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("the browser could not be opened ({e}): is xdg-open installed?"))?;
    // Reaped on the side, so it does not linger as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens a link a model wrote, if it is a web page or an email address. Anything else
/// (`file:`, `javascript:`, a custom scheme that starts an app) is ignored: the text is not ours.
pub fn open_link(url: &str) {
    if !is_web_link(url) {
        return;
    }
    if let Err(e) = open(url) {
        eprintln!("oido: could not open {url}: {e}");
    }
}

fn is_web_link(url: &str) -> bool {
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("http" | "https" | "mailto"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_open() {
        for url in ["https://example.com", "HTTP://x.y/z", "mailto:a@b.c"] {
            assert!(is_web_link(url), "{url}");
        }
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "steam://run/1",
            "/tmp/x",
            "x",
        ] {
            assert!(!is_web_link(url), "{url}");
        }
    }
}
