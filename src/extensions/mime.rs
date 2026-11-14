//! A plain-text email as Gmail's API takes it: the whole message, RFC 822 style, in URL-safe
//! base64. Everything in a header comes from a model, so none of it may break out of its line.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};

use super::google::Failure;

/// What goes into a message.
#[derive(Default, Debug)]
pub struct Outgoing {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// The `Message-ID` of what this answers, and what that one already answered.
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

impl Outgoing {
    /// The message, ready to send or keep as a draft.
    pub fn raw(&self) -> Result<String, Failure> {
        Ok(URL_SAFE_NO_PAD.encode(self.text()?))
    }

    fn text(&self) -> Result<String, Failure> {
        if self.to.is_empty() {
            return Err(Failure::other("`to` needs at least one email address"));
        }
        let mut out = String::from("MIME-Version: 1.0\r\n");
        for (name, list) in [("To", &self.to), ("Cc", &self.cc), ("Bcc", &self.bcc)] {
            if !list.is_empty() {
                let addresses: Result<Vec<&str>, Failure> =
                    list.iter().map(|a| address(a)).collect();
                out.push_str(&format!("{name}: {}\r\n", addresses?.join(", ")));
            }
        }
        out.push_str(&format!("Subject: {}\r\n", subject(&self.subject)?));
        for (name, value) in [
            ("In-Reply-To", &self.in_reply_to),
            ("References", &self.references),
        ] {
            if let Some(v) = value {
                out.push_str(&format!("{name}: {}\r\n", line(v)?));
            }
        }
        out.push_str(
            "Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n",
        );
        out.push_str(&wrap(
            &STANDARD.encode(self.body.replace("\r\n", "\n").replace('\n', "\r\n")),
        ));
        Ok(out)
    }
}

/// `value` if it is one line, which a header must be.
fn line(value: &str) -> Result<&str, Failure> {
    if value.contains(['\r', '\n']) {
        Err(Failure::other("a header cannot hold a line break"))
    } else {
        Ok(value.trim())
    }
}

/// A plain email address: no names, no brackets, nothing that could start another header.
fn address(a: &str) -> Result<&str, Failure> {
    let a = line(a)?;
    let ok = a.is_ascii()
        && a.matches('@').count() == 1
        && !a.starts_with('@')
        && !a.ends_with('@')
        && !a.contains(|c: char| {
            c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';' | '"' | '(' | ')')
        });
    if ok {
        Ok(a)
    } else {
        Err(Failure::other(format!(
            "{a:?} is not a plain email address (write name@example.com, with no display name)"
        )))
    }
}

/// A subject, as encoded words when it has anything but ASCII.
fn subject(s: &str) -> Result<String, Failure> {
    let s = line(s)?;
    if s.is_ascii() {
        return Ok(s.to_owned());
    }
    // Each encoded word holds at most 45 bytes (75 characters once encoded), cut on a character.
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in s.chars() {
        if chunk.len() + c.len_utf8() > 45 {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    words.push(chunk);
    Ok(words
        .iter()
        .map(|w| format!("=?UTF-8?B?{}?=", STANDARD.encode(w)))
        .collect::<Vec<_>>()
        .join("\r\n "))
}

/// Base64 in lines of 76 characters.
fn wrap(encoded: &str) -> String {
    encoded
        .as_bytes()
        .chunks(76)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("\r\n")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn mail() -> Outgoing {
        Outgoing {
            to: vec!["a@x.org".into()],
            subject: "Hi".into(),
            body: "Line one\nLine two".into(),
            ..Outgoing::default()
        }
    }

    fn decoded(m: &Outgoing) -> String {
        String::from_utf8(URL_SAFE_NO_PAD.decode(m.raw().unwrap()).unwrap()).unwrap()
    }

    #[test]
    fn a_message_has_its_headers_and_a_base64_body_with_crlf_lines() {
        let mut m = mail();
        m.cc = vec!["b@y.org".into()];
        m.in_reply_to = Some("<id1@x>".into());
        let text = decoded(&m);
        assert!(text.starts_with("MIME-Version: 1.0\r\nTo: a@x.org\r\nCc: b@y.org\r\nSubject: Hi\r\nIn-Reply-To: <id1@x>\r\n"));
        let (_, body) = text.split_once("\r\n\r\n").unwrap();
        let back = String::from_utf8(STANDARD.decode(body.replace("\r\n", "")).unwrap()).unwrap();
        assert_eq!(back, "Line one\r\nLine two");
    }

    #[test]
    fn nothing_from_the_model_can_start_another_header() {
        let mut m = mail();
        m.subject = "Hi\r\nBcc: evil@x.org".into();
        assert!(m.raw().is_err());
        for bad in [
            "a@x.org\r\nBcc: e@x.org",
            "Ann <a@x.org>",
            "a@x.org, b@x.org",
            "no-at-sign",
            "a@@x",
            "é@x.org",
            "a b@x.org",
        ] {
            let mut m = mail();
            m.to = vec![bad.into()];
            assert!(m.raw().is_err(), "{bad:?}");
        }
        let mut m = mail();
        m.in_reply_to = Some("<a>\r\nBcc: e@x".into());
        assert!(m.raw().is_err());
        let mut m = mail();
        m.to.clear();
        assert!(m.raw().is_err());
    }

    #[test]
    fn a_subject_with_more_than_ascii_is_sent_as_encoded_words_cut_on_characters() {
        let s = subject("Réunion à Nicosie — ordre du jour très détaillé pour demain").unwrap();
        assert!(s.starts_with("=?UTF-8?B?") && s.contains("\r\n =?UTF-8?B?"));
        let decoded: String = s
            .split("\r\n ")
            .map(|w| {
                let b64 = w.trim_start_matches("=?UTF-8?B?").trim_end_matches("?=");
                String::from_utf8(STANDARD.decode(b64).unwrap()).unwrap()
            })
            .collect();
        assert_eq!(
            decoded,
            "Réunion à Nicosie — ordre du jour très détaillé pour demain"
        );
        assert_eq!(subject("Plain").unwrap(), "Plain");
    }

    #[test]
    fn a_long_body_wraps_at_76_characters() {
        let mut m = mail();
        m.body = "x".repeat(500);
        let text = decoded(&m);
        let (_, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(body.split("\r\n").all(|l| l.len() <= 76));
    }
}
