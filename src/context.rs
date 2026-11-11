//! What the agent is told about the moment it is in, beside its brief: the date and time, and the
//! folder it works in. It goes after the brief and skills (and outside the part a provider
//! caches), since it changes with every message while they do not.

use std::path::Path;

use chrono::{DateTime, FixedOffset, Local};

/// The context block for `now` and the working folder `folder`, as lines under a heading. The
/// folder is given in full because tools that do not run in it, such as an MCP server's, need
/// absolute paths.
pub fn text(now: DateTime<FixedOffset>, folder: &Path) -> String {
    let when = now.format("%A, %-d %B %Y, %H:%M (UTC%:z)");
    format!(
        "## Context\n- Current date and time: {when}\n- Working folder: {}",
        folder.display()
    )
}

/// The context block for this moment, in this computer's time zone.
pub fn now_text(folder: &Path) -> String {
    text(Local::now().fixed_offset(), folder)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_date_and_time_read_in_words_with_the_zone() {
        let now = DateTime::parse_from_rfc3339("2026-11-09T14:32:05+02:00").unwrap();
        assert_eq!(
            text(now, Path::new("/home/me/work")),
            "## Context\n- Current date and time: Monday, 9 November 2026, 14:32 (UTC+02:00)\n- Working folder: /home/me/work"
        );
        let west = DateTime::parse_from_rfc3339("2027-01-01T00:05:00-05:00").unwrap();
        assert!(text(west, Path::new("/")).contains("Friday, 1 January 2027, 00:05 (UTC-05:00)"));
    }
}
