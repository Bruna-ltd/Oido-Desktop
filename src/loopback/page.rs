//! The pages the browser lands on after signing in to a server or a service: what the loopback port
//! answers with. They wear the app's look, so the person comes back to something that is plainly
//! Oido and not a bare browser tab.
//!
//! Nothing the browser sent is ever put in a page: the words are fixed, so there is nothing to
//! escape.

use std::io::Write;
use std::net::TcpStream;

use crate::theme::Colors;

/// One answer to the browser: its status line and the two lines it shows.
pub(super) struct Notice {
    status: u16,
    /// Whether the sign-in went well, which picks the colour of the mark before the title.
    good: bool,
    title: &'static str,
    detail: &'static str,
}

pub(super) const SIGNED_IN: Notice = Notice {
    status: 200,
    good: true,
    title: "Signed in",
    detail: "You can close this tab and go back to Oido.",
};
pub(super) const REFUSED: Notice = Notice {
    status: 200,
    good: false,
    title: "Sign-in was refused",
    detail: "You can close this tab and go back to Oido.",
};
pub(super) const NOT_OURS: Notice = Notice {
    status: 400,
    good: false,
    title: "Not started by Oido",
    detail: "This sign-in was not started by Oido. You can close this tab.",
};
pub(super) const NO_CODE: Notice = Notice {
    status: 400,
    good: false,
    title: "No code came back",
    detail: "The server sent nothing to sign in with. You can close this tab.",
};
pub(super) const NOT_FOUND: Notice = Notice {
    status: 404,
    good: false,
    title: "Not found",
    detail: "There is nothing here. You can close this tab.",
};

/// Oido's mark, drawn in the page's text colour (`assets/icon/oido-desktop.svg` without its tile).
const LOGO: &str = r#"<svg viewBox="0 0 32 32" width="40" height="40" fill="none" stroke="currentColor" stroke-width="2.9" aria-hidden="true"><path d="M1.45 1.45L16 1.45A14.55 14.55 0 0 1 30.55 16A14.55 14.55 0 0 1 16 30.55A14.55 14.55 0 0 1 1.45 16Z"/><circle cx="16" cy="16" r="6.6"/></svg>"#;

const STYLE: &str = "\
body{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif}\
main{box-sizing:border-box;width:min(420px,100% - 32px);padding:32px;border:1px solid var(--line);border-radius:14px;background:var(--card)}\
svg{display:block;margin-bottom:24px}\
h1{margin:0 0 4px;font-size:20px;font-weight:600}\
h1::before{content:'';display:inline-block;width:10px;height:10px;margin-right:10px;border-radius:50%;background:var(--mark)}\
p{margin:0;color:var(--muted)}";

impl Notice {
    /// The page, as HTML.
    fn html(&self, c: &Colors) -> String {
        let mark = if self.good { c.green } else { c.amber };
        format!(
            "<!doctype html><html lang=en><meta charset=utf-8>\
             <meta name=viewport content=\"width=device-width,initial-scale=1\">\
             <title>Oido</title>\
             <style>:root{{--bg:{};--fg:{};--muted:{};--line:{};--card:{};--mark:{}}}{STYLE}</style>\
             <main>{LOGO}<h1>{}</h1><p>{}</p></main>",
            c.bg.hex(),
            c.fg.hex(),
            c.fg.alpha(0.6).hex(),
            c.fg.alpha(0.12).hex(),
            c.fg.alpha(0.04).hex(),
            mark.hex(),
            self.title,
            self.detail,
        )
    }

    /// Writes the whole response. The page loads nothing from anywhere, and says so.
    pub(super) fn send(&self, stream: &mut TcpStream, colors: &Colors) {
        let body = self.html(colors);
        let _ = write!(
            stream,
            "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
             Cache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.status,
            if self.status == 200 { "OK" } else { "Error" },
            body.len()
        );
        let _ = stream.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_wears_the_colours_it_is_given_and_says_its_piece() {
        let c = Colors::oido();
        let page = SIGNED_IN.html(&c);
        assert!(
            page.contains(&c.bg.hex()) && page.contains(&c.fg.hex()),
            "{page}"
        );
        assert!(
            page.contains("--mark:") && page.contains(&c.green.hex()),
            "{page}"
        );
        assert!(page.contains("go back to Oido"), "{page}");
        // A refusal is marked in the warning colour, not the success one.
        let page = REFUSED.html(&c);
        assert!(
            page.contains(&format!("--mark:{}", c.amber.hex())),
            "{page}"
        );
    }
}
