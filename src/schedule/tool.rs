//! The `schedule` tool: what an agent may ask, and reading its arguments. What the app does with
//! a call (ownership, drafts, approval) is in `crate::schedules`.

use chrono::{NaiveDateTime, TimeZone};
use serde_json::{Map, Value, json};

use super::Timing;
use super::timing::{AtTime, Days};
use crate::completion::ToolSpec;

pub const NAME: &str = "schedule";

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: NAME.into(),
        description: "Manage your own schedules: prompts sent to you, or your saved workflows run, at set times while the app is open. Each time a schedule fires it opens a new chat with you; nobody is there to answer questions. `create` and `update` show the person a draft to approve, and take effect only once they do; say so. `pause`, `resume` and `delete` apply at once. Use `list` to see your schedules and their ids. Times are the person's local time. Give exactly one of `once`, `every_minutes`, `at` or `cron` when setting when it fires.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "create", "update", "pause", "resume", "delete"]},
                "id": {"type": "string", "description": "The schedule, for update, pause, resume and delete."},
                "name": {"type": "string", "description": "A short name for it."},
                "prompt": {"type": "string", "description": "What to tell yourself each time it fires."},
                "workflow": {"type": "string", "description": "Instead of a prompt: the name of one of your saved workflows to run."},
                "inputs": {"type": "object", "description": "The workflow's inputs, by name."},
                "once": {"type": "string", "description": "Fire one time, at \"YYYY-MM-DD HH:MM\"."},
                "every_minutes": {"type": "integer", "description": "Fire every this many minutes (60 for hourly)."},
                "start": {"type": "string", "description": "With every_minutes: the first time, \"YYYY-MM-DD HH:MM\". Default: one interval from now."},
                "at": {"type": "string", "description": "Fire at this time of day, \"HH:MM\"."},
                "days": {"type": "array", "items": {"type": "string", "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]}, "description": "With at: which weekdays. Default: every day."},
                "cron": {"type": "string", "description": "A 5-field cron expression: minute hour day-of-month month day-of-week."}
            },
            "required": ["action"]
        }),
    }
}

/// A line saying what a call does, for lists: "create: Digest".
pub fn summary(args: &str) -> String {
    let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let part = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    match (part("action"), part("name").or_else(|| part("id"))) {
        (Some(action), Some(what)) => format!("{action}: {what}"),
        (Some(action), None) => action.to_owned(),
        _ => args.to_owned(),
    }
}

/// What a call asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    List,
    Create(Fields),
    Update { id: String, fields: Fields },
    Pause(String),
    Resume(String),
    Delete(String),
}

/// The parts of a schedule a call gives; `None` is "not given" (for an update, "keep it").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields {
    pub name: Option<String>,
    pub prompt: Option<String>,
    pub workflow: Option<String>,
    pub inputs: Option<Map<String, Value>>,
    pub timing: Option<Timing>,
}

/// Reads a call's arguments, with `now` and `tz` for the times in them. Errs, in words the model
/// can act on.
pub fn parse<Tz: TimeZone>(args: &str, now: i64, tz: &Tz) -> Result<Call, String> {
    let v: Value = serde_json::from_str(args).map_err(|_| "the arguments are not JSON")?;
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let id = || text("id").ok_or_else(|| "give the schedule's `id` (see `list`)".to_owned());
    let fields = || -> Result<Fields, String> {
        let (prompt, workflow) = (text("prompt"), text("workflow"));
        if prompt.is_some() && workflow.is_some() {
            return Err("give a `prompt` or a `workflow`, not both".to_owned());
        }
        Ok(Fields {
            name: text("name"),
            prompt,
            workflow,
            inputs: v.get("inputs").and_then(Value::as_object).cloned(),
            timing: timing(&v, now, tz)?,
        })
    };
    match text("action").as_deref() {
        Some("list") => Ok(Call::List),
        Some("create") => {
            let f = fields()?;
            if f.name.is_none() {
                return Err("give the schedule a `name`".to_owned());
            }
            if f.prompt.is_none() && f.workflow.is_none() {
                return Err("give a `prompt` or a `workflow` to run".to_owned());
            }
            if f.timing.is_none() {
                return Err("say when it fires: `once`, `every_minutes`, `at` or `cron`".to_owned());
            }
            Ok(Call::Create(f))
        }
        Some("update") => Ok(Call::Update {
            id: id()?,
            fields: fields()?,
        }),
        Some("pause") => Ok(Call::Pause(id()?)),
        Some("resume") => Ok(Call::Resume(id()?)),
        Some("delete") => Ok(Call::Delete(id()?)),
        Some(other) => Err(format!("there is no action {other:?}")),
        None => Err("give an `action`".to_owned()),
    }
}

/// The timing the arguments give, if any: exactly one of `once`, `every_minutes`, `at`, `cron`.
fn timing<Tz: TimeZone>(v: &Value, now: i64, tz: &Tz) -> Result<Option<Timing>, String> {
    let given: Vec<&str> = ["once", "every_minutes", "at", "cron"]
        .into_iter()
        .filter(|k| v.get(*k).is_some_and(|x| !x.is_null()))
        .collect();
    let field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
    match given[..] {
        [] => Ok(None),
        ["once"] => Ok(Some(Timing::Once(local_time(field("once"), tz)?))),
        ["every_minutes"] => {
            let minutes = v
                .get("every_minutes")
                .and_then(Value::as_i64)
                .ok_or("`every_minutes` must be a whole number")?;
            let start = match v.get("start").and_then(Value::as_str) {
                Some(s) => local_time(s, tz)?,
                None => now,
            };
            Timing::every(start, minutes.saturating_mul(60)).map(Some)
        }
        ["at"] => {
            let (hour, minute) = clock(field("at"))?;
            let days = match v.get("days").and_then(Value::as_array) {
                Some(list) => weekdays(list)?,
                None => Days::EVERY,
            };
            Timing::at(AtTime { hour, minute, days }).map(Some)
        }
        ["cron"] => Timing::cron(field("cron")).map(Some),
        _ => Err(format!("give only one of {}", given.join(", "))),
    }
}

/// "YYYY-MM-DD HH:MM" (or with a `T`) in the local zone, as seconds since the epoch.
pub(crate) fn local_time<Tz: TimeZone>(s: &str, tz: &Tz) -> Result<i64, String> {
    let s = s.trim().replace('T', " ");
    let naive = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M")
        .map_err(|_| format!("{s:?} is not a time like \"2026-11-10 09:30\""))?;
    tz.from_local_datetime(&naive)
        .earliest()
        .map(|d| d.timestamp())
        .ok_or_else(|| format!("{s:?} does not exist in the local time zone"))
}

/// "HH:MM".
pub(crate) fn clock(s: &str) -> Result<(u8, u8), String> {
    let bad = || format!("{s:?} is not a time of day like \"09:30\"");
    let (h, m) = s.trim().split_once(':').ok_or_else(bad)?;
    let (h, m): (u8, u8) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h < 24 && m < 60 {
        Ok((h, m))
    } else {
        Err(bad())
    }
}

fn weekdays(list: &[Value]) -> Result<Days, String> {
    let mut days = Days::NONE;
    for d in list {
        let name = d.as_str().unwrap_or_default().trim().to_lowercase();
        let short: String = name.chars().take(3).collect();
        let day = Days::week()
            .find(|(_, n)| n.eq_ignore_ascii_case(&short))
            .map(|(d, _)| d)
            .ok_or_else(|| format!("{name:?} is not a weekday (mon, tue, ... sun)"))?;
        if !days.contains(day) {
            days.toggle(day);
        }
    }
    if days.is_empty() {
        return Err("`days` is empty".to_owned());
    }
    Ok(days)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use chrono::Utc;

    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn p(args: Value) -> Result<Call, String> {
        parse(&args.to_string(), NOW, &Utc)
    }

    #[test]
    fn a_create_needs_a_name_a_target_and_a_time() {
        let ok = p(
            json!({"action": "create", "name": "Digest", "prompt": "Sum up", "at": "09:00", "days": ["mon", "Friday"]}),
        );
        let Ok(Call::Create(f)) = ok else {
            panic!("{ok:?}")
        };
        assert_eq!(
            (f.name.as_deref(), f.prompt.as_deref()),
            (Some("Digest"), Some("Sum up"))
        );
        let at = f.timing.unwrap().preset().unwrap();
        assert_eq!((at.hour, at.minute), (9, 0));
        assert!(at.days.contains(1) && at.days.contains(5) && !at.days.contains(2));

        for (args, says) in [
            (
                json!({"action": "create", "prompt": "x", "at": "09:00"}),
                "name",
            ),
            (
                json!({"action": "create", "name": "n", "at": "09:00"}),
                "prompt",
            ),
            (
                json!({"action": "create", "name": "n", "prompt": "x"}),
                "when",
            ),
            (
                json!({"action": "create", "name": "n", "prompt": "x", "workflow": "w", "at": "09:00"}),
                "not both",
            ),
            (
                json!({"action": "create", "name": "n", "prompt": "x", "at": "09:00", "cron": "* * * * *"}),
                "only one",
            ),
        ] {
            let err = p(args).unwrap_err();
            assert!(err.contains(says), "{err}");
        }
    }

    #[test]
    fn times_are_read_in_the_given_zone() {
        let Call::Create(f) =
            p(json!({"action": "create", "name": "n", "prompt": "x", "once": "2027-01-15 08:30"}))
                .unwrap()
        else {
            panic!()
        };
        let want = NaiveDateTime::parse_from_str("2027-01-15 08:30", "%Y-%m-%d %H:%M")
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(f.timing, Some(Timing::Once(want)));

        let Call::Create(f) =
            p(json!({"action": "create", "name": "n", "prompt": "x", "every_minutes": 15}))
                .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            f.timing.unwrap().next_after(NOW, &Utc),
            Some(NOW + 900),
            "starts one interval from now"
        );

        assert!(
            p(json!({"action": "create", "name": "n", "prompt": "x", "once": "tomorrow"})).is_err()
        );
        assert!(p(json!({"action": "create", "name": "n", "prompt": "x", "at": "25:00"})).is_err());
        assert!(p(json!({"action": "create", "name": "n", "prompt": "x", "at": "09:00", "days": ["someday"]})).is_err());
        assert!(
            p(json!({"action": "create", "name": "n", "prompt": "x", "every_minutes": 0})).is_err()
        );
    }

    #[test]
    fn other_actions_need_an_id_and_an_update_may_change_any_part() {
        assert_eq!(p(json!({"action": "list"})), Ok(Call::List));
        assert_eq!(
            p(json!({"action": "pause", "id": "s1"})),
            Ok(Call::Pause("s1".into()))
        );
        assert_eq!(
            p(json!({"action": "resume", "id": "s1"})),
            Ok(Call::Resume("s1".into()))
        );
        assert_eq!(
            p(json!({"action": "delete", "id": "s1"})),
            Ok(Call::Delete("s1".into()))
        );
        assert!(p(json!({"action": "delete"})).unwrap_err().contains("id"));
        assert_eq!(
            p(json!({"action": "update", "id": "s1", "name": "New"})),
            Ok(Call::Update {
                id: "s1".into(),
                fields: Fields {
                    name: Some("New".into()),
                    ..Fields::default()
                }
            })
        );
        assert!(p(json!({"action": "explode"})).is_err());
        assert!(parse("nope", NOW, &Utc).is_err());
    }
}
