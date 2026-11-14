//! Google Calendar: see calendars and events, and make, change and delete events.

use serde_json::{Map, Value, json};

use super::google::{Api, Failure, segment};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

const EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";
const READ: &str = "https://www.googleapis.com/auth/calendar.readonly";
const BASE: &str = "https://www.googleapis.com/calendar/v3";
const DEFAULT_RESULTS: u64 = 20;
const MAX_RESULTS: u64 = 50;

pub struct Calendar;

impl Extension for Calendar {
    fn title(&self) -> &'static str {
        "Google Calendar"
    }

    fn logo(&self) -> Logo {
        super::marks::CALENDAR
    }

    fn blurb(&self) -> &'static str {
        "See the person's calendars and events, and make, change and delete events."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[EVENTS, READ]
    }

    fn tools(&self) -> Vec<ToolDef> {
        let calendar = json!({"type": "string", "description": "Which calendar: its id from list_calendars. Default: the person's own (primary)."});
        let event_id = json!({"type": "string", "description": "The event's id from list_events."});
        let when = |what: &str| json!({"type": "string", "description": format!("{what}: a date and time like 2025-03-01T14:00:00+02:00 (or 2025-03-01T14:00:00 with time_zone), or a date like 2025-03-01 for an all-day event.")});
        let time_zone = json!({"type": "string", "description": "An IANA time zone like Europe/Nicosia, for times written without an offset."});
        let notify = json!({"type": "boolean", "description": "Email the attendees about this. Default false."});
        vec![
            ToolDef {
                name: "list_calendars",
                description: "List the calendars the person can see: their own, shared and subscribed ones, with the id each is known by.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {}}),
            },
            ToolDef {
                name: "list_events",
                description: "List events in a calendar, soonest first, with repeating events spelled out one by one. Gives one line per event: its id, when, title and place. Read one in full with get_event.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "calendar_id": calendar,
                        "time_min": {"type": "string", "description": "Only events ending after this date and time (RFC 3339, like 2025-03-01T00:00:00Z). Default: now."},
                        "time_max": {"type": "string", "description": "Only events starting before this (RFC 3339). Default: no limit."},
                        "query": {"type": "string", "description": "Only events whose title, description, place or attendees contain this."},
                        "max_results": {"type": "integer", "description": "How many events, 1 to 50. Default 20."},
                        "page_token": {"type": "string", "description": "From the end of an earlier listing, for the next page."}
                    }
                }),
            },
            ToolDef {
                name: "get_event",
                description: "Read one event in full: when, where, description, who is invited and whether they answered, and its video-call link.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {"calendar_id": calendar, "event_id": event_id},
                    "required": ["event_id"]
                }),
            },
            ToolDef {
                name: "create_event",
                description: "Make an event. Attendees are only emailed if notify_attendees is true.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "calendar_id": calendar,
                        "summary": {"type": "string", "description": "The title."},
                        "start": when("When it starts"),
                        "end": when("When it ends"),
                        "time_zone": time_zone,
                        "description": {"type": "string"},
                        "location": {"type": "string"},
                        "attendees": {"type": "array", "items": {"type": "string"}, "description": "Email addresses to invite."},
                        "notify_attendees": notify
                    },
                    "required": ["summary", "start", "end"]
                }),
            },
            ToolDef {
                name: "update_event",
                description: "Change an event: only what is given changes. Giving attendees replaces the whole list of invited people.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "calendar_id": calendar,
                        "event_id": event_id,
                        "summary": {"type": "string"},
                        "start": when("When it starts"),
                        "end": when("When it ends"),
                        "time_zone": time_zone,
                        "description": {"type": "string"},
                        "location": {"type": "string"},
                        "attendees": {"type": "array", "items": {"type": "string"}, "description": "Email addresses; replaces everyone invited."},
                        "notify_attendees": notify
                    },
                    "required": ["event_id"]
                }),
            },
            ToolDef {
                name: "delete_event",
                description: "Delete an event. This cannot be undone from here.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"calendar_id": calendar, "event_id": event_id, "notify_attendees": notify},
                    "required": ["event_id"]
                }),
            },
        ]
    }

    fn call(&self, tool: &str, args: &Value, api: &Api) -> Result<String, Failure> {
        match tool {
            "list_calendars" => {
                let list = api.get(&format!("{BASE}/users/me/calendarList"), &[])?;
                Ok(render_calendars(&list))
            }
            "list_events" => list_events(args, api),
            "get_event" => {
                let url = event_url(args)?;
                Ok(render_event(&api.get(&url, &[])?))
            }
            "create_event" => {
                let body = event_body(args, true)?;
                let url = format!("{BASE}/calendars/{}/events", calendar_of(args));
                let made = api.post(&url, &[("sendUpdates", send_updates(args))], &body)?;
                Ok(format!("Created.\n{}", render_event(&made)))
            }
            "update_event" => {
                let body = event_body(args, false)?;
                if body.as_object().is_some_and(Map::is_empty) {
                    return Err(Failure::other("nothing to change was given"));
                }
                let changed = api.patch(
                    &event_url(args)?,
                    &[("sendUpdates", send_updates(args))],
                    &body,
                )?;
                Ok(format!("Updated.\n{}", render_event(&changed)))
            }
            "delete_event" => {
                api.delete(&event_url(args)?, &[("sendUpdates", send_updates(args))])?;
                Ok("Deleted.".to_owned())
            }
            other => Err(Failure::other(format!(
                "Google Calendar has no tool called {other}"
            ))),
        }
    }
}

/// The calendar an argument names, ready for an address: the person's own when none is named.
fn calendar_of(args: &Value) -> String {
    segment(
        args["calendar_id"]
            .as_str()
            .filter(|c| !c.is_empty())
            .unwrap_or("primary"),
    )
}

fn event_url(args: &Value) -> Result<String, Failure> {
    let id = args["event_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Failure::other("`event_id` is required"))?;
    Ok(format!(
        "{BASE}/calendars/{}/events/{}",
        calendar_of(args),
        segment(id)
    ))
}

fn send_updates(args: &Value) -> &'static str {
    if args["notify_attendees"].as_bool() == Some(true) {
        "all"
    } else {
        "none"
    }
}

fn list_events(args: &Value, api: &Api) -> Result<String, Failure> {
    let max = args["max_results"]
        .as_u64()
        .unwrap_or(DEFAULT_RESULTS)
        .clamp(1, MAX_RESULTS)
        .to_string();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let given = |name: &str| args[name].as_str().filter(|v| !v.is_empty());
    let mut query = vec![
        ("singleEvents", "true"),
        ("orderBy", "startTime"),
        ("maxResults", max.as_str()),
        ("timeMin", given("time_min").unwrap_or(now.as_str())),
    ];
    for (name, key) in [
        ("time_max", "timeMax"),
        ("query", "q"),
        ("page_token", "pageToken"),
    ] {
        if let Some(v) = given(name) {
            query.push((key, v));
        }
    }
    let url = format!("{BASE}/calendars/{}/events", calendar_of(args));
    let found = api.get(&url, &query)?;
    let events = found["items"].as_array().map_or(&[][..], Vec::as_slice);
    if events.is_empty() {
        return Ok("No events found.".to_owned());
    }
    let mut lines: Vec<String> = events.iter().map(summary_line).collect();
    if let Some(next) = found["nextPageToken"].as_str() {
        lines.push(format!("More events: list again with page_token={next}"));
    }
    Ok(lines.join("\n"))
}

/// What the person asked to set on an event. A new event needs a title and both times; a change
/// needs only what changes.
fn event_body(args: &Value, creating: bool) -> Result<Value, Failure> {
    let mut body = Map::new();
    let text = |name: &str| args[name].as_str();
    for name in ["summary", "description", "location"] {
        if let Some(v) = text(name) {
            body.insert(name.to_owned(), json!(v));
        }
    }
    let zone = text("time_zone").filter(|z| !z.is_empty());
    for name in ["start", "end"] {
        if let Some(v) = text(name) {
            body.insert(name.to_owned(), moment(v, zone)?);
        }
    }
    if let Some(list) = args["attendees"].as_array() {
        let mut people = Vec::new();
        for who in list {
            let email = who
                .as_str()
                .filter(|e| e.contains('@') && !e.contains(char::is_whitespace))
                .ok_or_else(|| Failure::other(format!("{who} is not an email address")))?;
            people.push(json!({"email": email}));
        }
        body.insert("attendees".to_owned(), Value::Array(people));
    }
    if creating {
        for need in ["summary", "start", "end"] {
            if !body.contains_key(need) {
                return Err(Failure::other(format!("`{need}` is required")));
            }
        }
    }
    Ok(Value::Object(body))
}

/// A date, or a date and time, as Google wants it written.
fn moment(v: &str, zone: Option<&str>) -> Result<Value, Failure> {
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    let is_date =
        v.len() == 10 && v.split('-').map(str::len).eq([4, 2, 2]) && v.split('-').all(digits);
    if is_date {
        Ok(json!({"date": v}))
    } else if v.len() >= 16 && v.as_bytes()[10] == b'T' {
        Ok(match zone {
            Some(z) => json!({"dateTime": v, "timeZone": z}),
            None => json!({"dateTime": v}),
        })
    } else {
        Err(Failure::other(format!(
            "{v:?} is not a date (2025-03-01) or a date and time (2025-03-01T14:00:00+02:00)"
        )))
    }
}

/// When an event starts or ends, as it was written.
fn when(t: &Value) -> &str {
    t["dateTime"]
        .as_str()
        .or_else(|| t["date"].as_str())
        .unwrap_or("?")
}

fn summary_line(e: &Value) -> String {
    let mut line = format!(
        "{} · {} → {} · {}",
        e["id"].as_str().unwrap_or("?"),
        when(&e["start"]),
        when(&e["end"]),
        e["summary"].as_str().unwrap_or("(no title)")
    );
    if let Some(place) = e["location"].as_str().filter(|p| !p.is_empty()) {
        line.push_str(&format!(" · {place}"));
    }
    line
}

fn render_event(e: &Value) -> String {
    let mut out = format!(
        "Event {}\nTitle: {}\nStarts: {}\nEnds: {}\n",
        e["id"].as_str().unwrap_or("?"),
        e["summary"].as_str().unwrap_or("(no title)"),
        when(&e["start"]),
        when(&e["end"]),
    );
    let mut field = |name: &str, value: Option<&str>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            out.push_str(&format!("{name}: {v}\n"));
        }
    };
    field("Where", e["location"].as_str());
    field("Organizer", e["organizer"]["email"].as_str());
    field("Video call", e["hangoutLink"].as_str());
    field(
        "Repeats",
        e["recurringEventId"]
            .as_str()
            .map(|_| "yes, part of a series"),
    );
    field("Link", e["htmlLink"].as_str());
    let people: Vec<String> = e["attendees"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            format!(
                "{} ({})",
                a["email"].as_str().unwrap_or("?"),
                a["responseStatus"].as_str().unwrap_or("no answer")
            )
        })
        .collect();
    if !people.is_empty() {
        out.push_str(&format!("Invited: {}\n", people.join(", ")));
    }
    if let Some(d) = e["description"].as_str().filter(|d| !d.trim().is_empty()) {
        out.push_str(&format!("\n{}", d.trim()));
    }
    out.trim_end().to_owned()
}

fn render_calendars(list: &Value) -> String {
    let lines: Vec<String> = list["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            format!(
                "{} · {}{} · {}",
                c["id"].as_str().unwrap_or("?"),
                c["summary"].as_str().unwrap_or("?"),
                if c["primary"].as_bool() == Some(true) {
                    " (primary)"
                } else {
                    ""
                },
                c["accessRole"].as_str().unwrap_or("?")
            )
        })
        .collect();
    if lines.is_empty() {
        "No calendars.".to_owned()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_new_event_needs_a_title_and_both_times_and_keeps_zones_with_times() {
        let body = event_body(
            &json!({
                "summary": "Lunch", "start": "2025-03-01T12:00:00", "end": "2025-03-01T13:00:00",
                "time_zone": "Europe/Nicosia", "attendees": ["a@b.c"], "location": "Cafe"
            }),
            true,
        )
        .unwrap();
        assert_eq!(body["summary"], "Lunch");
        assert_eq!(
            body["start"],
            json!({"dateTime": "2025-03-01T12:00:00", "timeZone": "Europe/Nicosia"})
        );
        assert_eq!(body["attendees"], json!([{"email": "a@b.c"}]));
        let err = event_body(&json!({"summary": "x", "start": "2025-03-01"}), true).unwrap_err();
        assert_eq!(err, Failure::other("`end` is required"));
    }

    #[test]
    fn a_change_carries_only_what_is_given() {
        let body = event_body(&json!({"event_id": "e", "location": "Home"}), false).unwrap();
        assert_eq!(body, json!({"location": "Home"}));
        assert_eq!(
            event_body(&json!({"event_id": "e"}), false).unwrap(),
            json!({})
        );
    }

    #[test]
    fn dates_and_times_are_told_apart_and_nonsense_is_refused() {
        assert_eq!(
            moment("2025-03-01", Some("UTC")).unwrap(),
            json!({"date": "2025-03-01"})
        );
        assert_eq!(
            moment("2025-03-01T09:30:00Z", None).unwrap(),
            json!({"dateTime": "2025-03-01T09:30:00Z"})
        );
        for bad in ["tomorrow", "2025-3-1", "2025-03-01 09:30", ""] {
            assert!(moment(bad, None).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_attendee_that_is_not_an_address_is_refused() {
        for bad in [json!(["nobody"]), json!(["a b@c.d"]), json!([1])] {
            assert!(
                event_body(&json!({"attendees": bad}), false).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn attendees_hear_of_it_only_when_asked() {
        assert_eq!(send_updates(&json!({})), "none");
        assert_eq!(send_updates(&json!({"notify_attendees": false})), "none");
        assert_eq!(send_updates(&json!({"notify_attendees": true})), "all");
    }

    #[test]
    fn ids_go_into_addresses_escaped() {
        assert_eq!(calendar_of(&json!({})), "primary");
        assert_eq!(
            calendar_of(&json!({"calendar_id": "x@group.calendar.google.com"})),
            "x%40group.calendar.google.com"
        );
        assert_eq!(
            event_url(&json!({"event_id": "../x"})).unwrap(),
            format!("{BASE}/calendars/primary/events/..%2Fx")
        );
        assert!(event_url(&json!({})).is_err());
    }

    #[test]
    fn events_read_on_a_line_and_in_full() {
        let e = json!({
            "id": "e1", "summary": "Standup", "location": "Room 2",
            "start": {"dateTime": "2025-03-01T09:00:00+02:00"}, "end": {"dateTime": "2025-03-01T09:15:00+02:00"},
            "organizer": {"email": "boss@x.org"}, "hangoutLink": "https://meet/x",
            "attendees": [{"email": "a@b.c", "responseStatus": "accepted"}, {"email": "d@e.f"}],
            "description": " Bring notes \n"
        });
        assert_eq!(
            summary_line(&e),
            "e1 · 2025-03-01T09:00:00+02:00 → 2025-03-01T09:15:00+02:00 · Standup · Room 2"
        );
        let full = render_event(&e);
        assert!(full.contains("Where: Room 2") && full.contains("Video call: https://meet/x"));
        assert!(full.contains("Invited: a@b.c (accepted), d@e.f (no answer)"));
        assert!(full.ends_with("\nBring notes"));
        let all_day =
            json!({"id": "e2", "start": {"date": "2025-03-02"}, "end": {"date": "2025-03-03"}});
        assert_eq!(
            summary_line(&all_day),
            "e2 · 2025-03-02 → 2025-03-03 · (no title)"
        );
    }

    #[test]
    fn calendars_show_their_id_name_and_role() {
        let list = json!({"items": [{"id": "me@x.org", "summary": "Me", "primary": true, "accessRole": "owner"}]});
        assert_eq!(render_calendars(&list), "me@x.org · Me (primary) · owner");
        assert_eq!(render_calendars(&json!({})), "No calendars.");
    }

    #[test]
    fn tools_that_change_things_say_so_and_every_tool_is_described() {
        let tools = Calendar.tools();
        for t in &tools {
            assert!(
                !t.description.is_empty() && t.schema["type"] == "object",
                "{}",
                t.name
            );
        }
        let changing: Vec<&str> = tools
            .iter()
            .filter(|t| t.access == Access::Changes)
            .map(|t| t.name)
            .collect();
        assert_eq!(changing, ["create_event", "update_event", "delete_event"]);
    }
}
