//! YouTube: search, watch details, channels, playlists, subscriptions and comments. Read only.
//!
//! The YouTube Data API has a daily quota (10,000 units by default): a search costs 100 units and
//! almost everything else 1, so the search description says to prefer the cheaper tools.

use serde_json::{Value, json};

use super::google::{Api, Failure};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

const READ: &str = "https://www.googleapis.com/auth/youtube.readonly";
const BASE: &str = "https://www.googleapis.com/youtube/v3";
const DEFAULT_RESULTS: u64 = 10;
const MAX_RESULTS: u64 = 50;
/// A description or comment is cut to this many characters.
const TEXT_LIMIT: usize = 1500;

pub struct YouTube;

impl Extension for YouTube {
    fn title(&self) -> &'static str {
        "YouTube"
    }

    fn logo(&self) -> Logo {
        super::marks::YOUTUBE
    }

    fn blurb(&self) -> &'static str {
        "Search videos, read their details and comments, and see the person's channel, playlists and subscriptions."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[READ]
    }

    fn tools(&self) -> Vec<ToolDef> {
        let max = json!({"type": "integer", "description": "How many, 1 to 50. Default 10."});
        let page = json!({"type": "string", "description": "From the end of an earlier listing, for the next page."});
        vec![
            ToolDef {
                name: "search",
                description: "Search YouTube for videos, channels or playlists. Gives one line per result: its id, title, channel and date. Costs 100 units of the person's daily quota (10,000 by default), so use get_video, list_playlist_items or get_channel when you already know what you want.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "What to search for."},
                        "type": {"type": "string", "enum": ["video", "channel", "playlist"], "description": "What to find. Default video."},
                        "order": {"type": "string", "enum": ["relevance", "date", "rating", "viewCount"], "description": "Default relevance."},
                        "channel_id": {"type": "string", "description": "Only results from this channel."},
                        "published_after": {"type": "string", "description": "Only results from after this date and time (RFC 3339, like 2025-01-01T00:00:00Z)."},
                        "max_results": max,
                        "page_token": page
                    },
                    "required": ["query"]
                }),
            },
            ToolDef {
                name: "get_video",
                description: "Details of up to 10 videos: title, channel, when it went up, length, views, likes, comment count, description and link.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {"ids": {"type": "string", "description": "Video ids, separated by commas. A video's id is the v= part of its address."}},
                    "required": ["ids"]
                }),
            },
            ToolDef {
                name: "get_channel",
                description: "A channel's details: name, handle, subscribers, videos, views, description, and the id of its playlist of uploads (list_playlist_items on it gives the newest videos). With nothing given, the person's own channel.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "channel_id": {"type": "string", "description": "A channel id, like UC..."},
                        "handle": {"type": "string", "description": "A channel handle, like @name."}
                    }
                }),
            },
            ToolDef {
                name: "list_playlists",
                description: "A channel's playlists, each with its id and how many videos it has. With no channel given, the person's own.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {"channel_id": {"type": "string"}, "max_results": max, "page_token": page}
                }),
            },
            ToolDef {
                name: "list_playlist_items",
                description: "The videos in a playlist, in order: position, video id, title and channel.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "playlist_id": {"type": "string", "description": "The playlist's id."},
                        "max_results": max,
                        "page_token": page
                    },
                    "required": ["playlist_id"]
                }),
            },
            ToolDef {
                name: "list_subscriptions",
                description: "The channels the person subscribes to, with each one's id.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {"max_results": max, "page_token": page}}),
            },
            ToolDef {
                name: "get_comments",
                description: "The top comments on a video: who, how many likes, how many replies, and the text.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "video_id": {"type": "string"},
                        "order": {"type": "string", "enum": ["relevance", "time"], "description": "Default relevance."},
                        "max_results": max,
                        "page_token": page
                    },
                    "required": ["video_id"]
                }),
            },
        ]
    }

    fn call(&self, tool: &str, args: &Value, api: &Api) -> Result<String, Failure> {
        match tool {
            "search" => search(args, api),
            "get_video" => {
                let ids = required(args, "ids")?;
                let found = list(
                    api,
                    "videos",
                    &[("part", "snippet,contentDetails,statistics"), ("id", ids)],
                )?;
                Ok(items(&found)
                    .iter()
                    .map(render_video)
                    .collect::<Vec<_>>()
                    .join("\n\n---\n\n"))
                .map(|t| {
                    if t.is_empty() {
                        "No such video.".to_owned()
                    } else {
                        t
                    }
                })
            }
            "get_channel" => {
                let mut query = vec![("part", "snippet,statistics,contentDetails")];
                match (arg(args, "channel_id"), arg(args, "handle")) {
                    (Some(id), _) => query.push(("id", id)),
                    (None, Some(handle)) => query.push(("forHandle", handle)),
                    (None, None) => query.push(("mine", "true")),
                }
                let found = list(api, "channels", &query)?;
                Ok(items(&found)
                    .first()
                    .map_or_else(|| "No such channel.".to_owned(), render_channel))
            }
            "list_playlists" => {
                let mut query = vec![("part", "snippet,contentDetails")];
                match arg(args, "channel_id") {
                    Some(id) => query.push(("channelId", id)),
                    None => query.push(("mine", "true")),
                }
                paged(
                    api,
                    "playlists",
                    args,
                    query,
                    playlist_line,
                    "No playlists.",
                )
            }
            "list_playlist_items" => {
                let id = required(args, "playlist_id")?;
                let query = vec![("part", "snippet"), ("playlistId", id)];
                paged(
                    api,
                    "playlistItems",
                    args,
                    query,
                    playlist_item_line,
                    "The playlist is empty.",
                )
            }
            "list_subscriptions" => {
                let query = vec![("part", "snippet"), ("mine", "true")];
                paged(
                    api,
                    "subscriptions",
                    args,
                    query,
                    subscription_line,
                    "No subscriptions.",
                )
            }
            "get_comments" => {
                let id = required(args, "video_id")?;
                let order = arg(args, "order").unwrap_or("relevance");
                let query = vec![
                    ("part", "snippet"),
                    ("videoId", id),
                    ("order", order),
                    ("textFormat", "plainText"),
                ];
                paged(
                    api,
                    "commentThreads",
                    args,
                    query,
                    comment_line,
                    "No comments.",
                )
            }
            other => Err(Failure::other(format!(
                "YouTube has no tool called {other}"
            ))),
        }
    }
}

fn arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args[name].as_str().filter(|v| !v.trim().is_empty())
}

fn required<'a>(args: &'a Value, name: &str) -> Result<&'a str, Failure> {
    arg(args, name).ok_or_else(|| Failure::other(format!("`{name}` is required")))
}

fn items(found: &Value) -> &[Value] {
    found["items"].as_array().map_or(&[][..], Vec::as_slice)
}

fn list(api: &Api, what: &str, query: &[(&str, &str)]) -> Result<Value, Failure> {
    api.get(&format!("{BASE}/{what}"), query)
}

fn max_results(args: &Value) -> String {
    args["max_results"]
        .as_u64()
        .unwrap_or(DEFAULT_RESULTS)
        .clamp(1, MAX_RESULTS)
        .to_string()
}

/// A listing with a page size and token from `args`, one line per item, and the next page's token.
fn paged(
    api: &Api,
    what: &str,
    args: &Value,
    query: Vec<(&str, &str)>,
    line: fn(&Value) -> String,
    none: &str,
) -> Result<String, Failure> {
    let max = max_results(args);
    let mut query = query;
    query.push(("maxResults", max.as_str()));
    if let Some(t) = arg(args, "page_token") {
        query.push(("pageToken", t));
    }
    let found = list(api, what, &query)?;
    let rows = items(&found);
    if rows.is_empty() {
        return Ok(none.to_owned());
    }
    let mut lines: Vec<String> = rows.iter().map(line).collect();
    if let Some(next) = found["nextPageToken"].as_str() {
        lines.push(format!("More: ask again with page_token={next}"));
    }
    Ok(lines.join("\n"))
}

fn search(args: &Value, api: &Api) -> Result<String, Failure> {
    let q = required(args, "query")?;
    let kind = match arg(args, "type") {
        None => "video",
        Some(k @ ("video" | "channel" | "playlist")) => k,
        Some(other) => {
            return Err(Failure::other(format!(
                "type must be video, channel or playlist, not {other}"
            )));
        }
    };
    let max = max_results(args);
    let mut query = vec![
        ("part", "snippet"),
        ("q", q),
        ("type", kind),
        ("maxResults", max.as_str()),
    ];
    for (name, key) in [
        ("order", "order"),
        ("channel_id", "channelId"),
        ("published_after", "publishedAfter"),
        ("page_token", "pageToken"),
    ] {
        if let Some(v) = arg(args, name) {
            query.push((key, v));
        }
    }
    let found = list(api, "search", &query)?;
    let rows = items(&found);
    if rows.is_empty() {
        return Ok("No results.".to_owned());
    }
    let mut lines: Vec<String> = rows.iter().map(search_line).collect();
    if let Some(next) = found["nextPageToken"].as_str() {
        lines.push(format!("More: search again with page_token={next}"));
    }
    Ok(lines.join("\n"))
}

fn search_line(r: &Value) -> String {
    let id = ["videoId", "channelId", "playlistId"]
        .iter()
        .find_map(|k| r["id"][k].as_str())
        .unwrap_or("?");
    format!(
        "{id} · {} · {} · {}",
        r["snippet"]["title"].as_str().unwrap_or("?"),
        r["snippet"]["channelTitle"].as_str().unwrap_or("?"),
        day(r["snippet"]["publishedAt"].as_str())
    )
}

/// The date part of a timestamp.
fn day(t: Option<&str>) -> &str {
    t.map_or("?", |t| t.split('T').next().unwrap_or(t))
}

/// `PT1H2M3S` as `1:02:03`, `PT45S` as `0:45`.
fn duration(iso: &str) -> String {
    let rest = iso.strip_prefix('P').unwrap_or(iso);
    let (days, time) = rest.split_once('T').unwrap_or((rest, ""));
    let number = |s: &str, unit: char| -> u64 {
        s.split_inclusive(|c: char| c.is_ascii_alphabetic())
            .find(|part| part.ends_with(unit))
            .and_then(|part| part.trim_end_matches(unit).parse().ok())
            .unwrap_or(0)
    };
    let (d, h, m, s) = (
        number(days, 'D'),
        number(time, 'H'),
        number(time, 'M'),
        number(time, 'S'),
    );
    let hours = d * 24 + h;
    if hours > 0 {
        format!("{hours}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn count(v: &Value) -> String {
    v.as_str().unwrap_or("?").to_owned()
}

fn cut(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(TEXT_LIMIT) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

fn render_video(v: &Value) -> String {
    let id = v["id"].as_str().unwrap_or("?");
    let s = &v["snippet"];
    let stats = &v["statistics"];
    let mut out = format!(
        "Video {id}\nTitle: {}\nChannel: {} ({})\nPublished: {}\nLength: {}\nViews: {} · Likes: {} · Comments: {}\nLink: https://www.youtube.com/watch?v={id}\n",
        s["title"].as_str().unwrap_or("?"),
        s["channelTitle"].as_str().unwrap_or("?"),
        s["channelId"].as_str().unwrap_or("?"),
        day(s["publishedAt"].as_str()),
        v["contentDetails"]["duration"]
            .as_str()
            .map_or_else(|| "?".to_owned(), duration),
        count(&stats["viewCount"]),
        count(&stats["likeCount"]),
        count(&stats["commentCount"]),
    );
    if let Some(d) = s["description"].as_str().filter(|d| !d.trim().is_empty()) {
        out.push_str(&format!("\n{}", cut(d)));
    }
    out.trim_end().to_owned()
}

fn render_channel(c: &Value) -> String {
    let s = &c["snippet"];
    let stats = &c["statistics"];
    let subscribers = if stats["hiddenSubscriberCount"].as_bool() == Some(true) {
        "hidden".to_owned()
    } else {
        count(&stats["subscriberCount"])
    };
    let mut out = format!(
        "Channel {}\nName: {}\nHandle: {}\nSubscribers: {subscribers} · Videos: {} · Views: {}\nUploads playlist: {}\n",
        c["id"].as_str().unwrap_or("?"),
        s["title"].as_str().unwrap_or("?"),
        s["customUrl"].as_str().unwrap_or("-"),
        count(&stats["videoCount"]),
        count(&stats["viewCount"]),
        c["contentDetails"]["relatedPlaylists"]["uploads"]
            .as_str()
            .unwrap_or("-"),
    );
    if let Some(d) = s["description"].as_str().filter(|d| !d.trim().is_empty()) {
        out.push_str(&format!("\n{}", cut(d)));
    }
    out.trim_end().to_owned()
}

fn playlist_line(p: &Value) -> String {
    format!(
        "{} · {} · {} videos",
        p["id"].as_str().unwrap_or("?"),
        p["snippet"]["title"].as_str().unwrap_or("?"),
        p["contentDetails"]["itemCount"].as_u64().unwrap_or(0)
    )
}

fn playlist_item_line(i: &Value) -> String {
    let s = &i["snippet"];
    format!(
        "{} · {} · {} · {}",
        s["position"]
            .as_u64()
            .map_or_else(|| "?".to_owned(), |p| (p + 1).to_string()),
        s["resourceId"]["videoId"].as_str().unwrap_or("?"),
        s["title"].as_str().unwrap_or("?"),
        s["videoOwnerChannelTitle"].as_str().unwrap_or("?")
    )
}

fn subscription_line(s: &Value) -> String {
    format!(
        "{} · {}",
        s["snippet"]["resourceId"]["channelId"]
            .as_str()
            .unwrap_or("?"),
        s["snippet"]["title"].as_str().unwrap_or("?")
    )
}

fn comment_line(t: &Value) -> String {
    let top = &t["snippet"]["topLevelComment"]["snippet"];
    format!(
        "{} ({} likes, {} replies): {}",
        top["authorDisplayName"].as_str().unwrap_or("?"),
        top["likeCount"].as_u64().unwrap_or(0),
        t["snippet"]["totalReplyCount"].as_u64().unwrap_or(0),
        cut(top["textDisplay"].as_str().unwrap_or("")).replace('\n', " ")
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn lengths_are_read_as_clock_times() {
        for (iso, shown) in [
            ("PT45S", "0:45"),
            ("PT3M5S", "3:05"),
            ("PT1H2M3S", "1:02:03"),
            ("PT1H", "1:00:00"),
            ("P1DT1H", "25:00:00"),
            ("PT0S", "0:00"),
        ] {
            assert_eq!(duration(iso), shown, "{iso}");
        }
    }

    #[test]
    fn a_search_result_names_its_id_whatever_it_is() {
        let video = json!({"id": {"videoId": "v1"}, "snippet": {"title": "T", "channelTitle": "C", "publishedAt": "2025-03-01T10:00:00Z"}});
        assert_eq!(search_line(&video), "v1 · T · C · 2025-03-01");
        let channel =
            json!({"id": {"channelId": "UC1"}, "snippet": {"title": "Ch", "channelTitle": "Ch"}});
        assert!(search_line(&channel).starts_with("UC1 · Ch"));
    }

    #[test]
    fn a_video_shows_its_numbers_link_and_a_cut_description() {
        let v = json!({
            "id": "abc", "snippet": {"title": "Hi", "channelTitle": "Me", "channelId": "UC9", "publishedAt": "2025-01-02T00:00:00Z", "description": "x".repeat(2000)},
            "contentDetails": {"duration": "PT2M"}, "statistics": {"viewCount": "10", "likeCount": "3", "commentCount": "1"}
        });
        let out = render_video(&v);
        assert!(out.contains("Length: 2:00") && out.contains("Views: 10 · Likes: 3 · Comments: 1"));
        assert!(out.contains("Link: https://www.youtube.com/watch?v=abc"));
        assert!(out.ends_with('…') && out.len() < 1900);
    }

    #[test]
    fn a_channel_says_when_its_subscribers_are_hidden() {
        let c = json!({"id": "UC1", "snippet": {"title": "N", "customUrl": "@n"}, "statistics": {"hiddenSubscriberCount": true, "videoCount": "5", "viewCount": "9"},
            "contentDetails": {"relatedPlaylists": {"uploads": "UU1"}}});
        let out = render_channel(&c);
        assert!(
            out.contains("Subscribers: hidden")
                && out.contains("Uploads playlist: UU1")
                && out.contains("Handle: @n")
        );
    }

    #[test]
    fn list_lines_count_from_one_and_comments_stay_on_one_line() {
        let item = json!({"snippet": {"position": 0, "resourceId": {"videoId": "v"}, "title": "T", "videoOwnerChannelTitle": "C"}});
        assert_eq!(playlist_item_line(&item), "1 · v · T · C");
        let thread = json!({"snippet": {"totalReplyCount": 2, "topLevelComment": {"snippet": {"authorDisplayName": "Ann", "likeCount": 7, "textDisplay": "a\nb"}}}});
        assert_eq!(comment_line(&thread), "Ann (7 likes, 2 replies): a b");
        assert_eq!(
            playlist_line(
                &json!({"id": "P", "snippet": {"title": "Mix"}, "contentDetails": {"itemCount": 4}})
            ),
            "P · Mix · 4 videos"
        );
    }

    #[test]
    fn a_page_is_between_one_and_fifty() {
        assert_eq!(max_results(&json!({})), "10");
        assert_eq!(max_results(&json!({"max_results": 0})), "1");
        assert_eq!(max_results(&json!({"max_results": 999})), "50");
    }

    #[test]
    fn every_tool_only_reads_and_is_described() {
        for t in YouTube.tools() {
            assert!(
                !t.description.is_empty()
                    && t.schema["type"] == "object"
                    && t.access == Access::Reads,
                "{}",
                t.name
            );
        }
        assert!(required(&json!({}), "ids").is_err());
    }
}
