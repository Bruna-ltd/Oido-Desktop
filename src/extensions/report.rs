//! What the tools that read reports share (YouTube, Google Analytics, Search Console): the
//! arguments they take, the dates a report covers, and how its numbers are written.

use serde_json::Value;

use super::google::Failure;

/// A report covers this many days up to today when no dates are given.
const DEFAULT_DAYS: i64 = 28;

/// A text argument, if it was given and is not blank.
pub fn arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args[name].as_str().filter(|v| !v.trim().is_empty())
}

pub fn required<'a>(args: &'a Value, name: &str) -> Result<&'a str, Failure> {
    arg(args, name).ok_or_else(|| Failure::other(format!("`{name}` is required")))
}

/// How many rows to ask for: `max_results` if given, held between 1 and `max`.
pub fn limit(args: &Value, default: u64, max: u64) -> u64 {
    args["max_results"]
        .as_u64()
        .unwrap_or(default)
        .clamp(1, max)
}

/// The dates a report covers: `start_date` to `end_date`, as asked, else the 28 days up to `today`.
pub fn period(args: &Value, today: chrono::NaiveDate) -> Result<(String, String), Failure> {
    let date = |name: &str| -> Result<Option<chrono::NaiveDate>, Failure> {
        arg(args, name)
            .map(|d| {
                chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| {
                    Failure::other(format!(
                        "`{name}` must be a date like 2025-03-01, not {d:?}"
                    ))
                })
            })
            .transpose()
    };
    let end = date("end_date")?.unwrap_or(today);
    let start = date("start_date")?.unwrap_or(end - chrono::Duration::days(DEFAULT_DAYS - 1));
    if start > end {
        return Err(Failure::other("`start_date` is after `end_date`"));
    }
    Ok((start.to_string(), end.to_string()))
}

/// The period up to today, by the clock of this computer.
pub fn recent(args: &Value) -> Result<(String, String), Failure> {
    period(args, chrono::Utc::now().date_naive())
}

/// A comma-separated list of names, trimmed, blanks dropped.
pub fn names(args: &Value, name: &str) -> Vec<String> {
    arg(args, name)
        .into_iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A cell of a report: text as it is, a whole number as one, any other number to two places.
pub fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n
            .as_i64()
            .map_or_else(|| decimal(n.as_f64().unwrap_or(0.0)), |i| i.to_string()),
        other => other.to_string(),
    }
}

/// `f` to at most two places, with no trailing zeros.
pub fn decimal(f: f64) -> String {
    format!("{f:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_report_covers_the_asked_dates_or_the_28_days_up_to_today() {
        let today = day(2025, 3, 31);
        assert_eq!(
            period(&json!({}), today).unwrap(),
            ("2025-03-04".to_owned(), "2025-03-31".to_owned())
        );
        assert_eq!(
            period(&json!({"end_date": "2025-02-10"}), today).unwrap(),
            ("2025-01-14".to_owned(), "2025-02-10".to_owned())
        );
        assert_eq!(
            period(
                &json!({"start_date": "2025-01-01", "end_date": "2025-01-31"}),
                today
            )
            .unwrap(),
            ("2025-01-01".to_owned(), "2025-01-31".to_owned())
        );
        for bad in [
            json!({"start_date": "yesterday"}),
            json!({"start_date": "2025-05-01", "end_date": "2025-04-01"}),
        ] {
            assert!(period(&bad, today).is_err(), "{bad}");
        }
    }

    #[test]
    fn numbers_are_whole_or_two_places() {
        assert_eq!(cell(&json!(120)), "120");
        assert_eq!(cell(&json!(61.5)), "61.5");
        assert_eq!(cell(&json!(59.123456)), "59.12");
        assert_eq!(cell(&json!(2.0)), "2");
        assert_eq!(cell(&json!("US")), "US");
    }

    #[test]
    fn lists_and_limits_are_tidied() {
        assert_eq!(
            names(&json!({"m": " a, b ,,c"}), "m"),
            ["a", "b", "c"].map(String::from)
        );
        assert!(names(&json!({}), "m").is_empty());
        assert_eq!(limit(&json!({}), 10, 50), 10);
        assert_eq!(limit(&json!({"max_results": 0}), 10, 50), 1);
        assert_eq!(limit(&json!({"max_results": 999}), 10, 50), 50);
        assert!(required(&json!({"id": " "}), "id").is_err());
    }
}
