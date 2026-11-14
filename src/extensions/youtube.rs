//! YouTube: search, watch details, channels, playlists, subscriptions and comments, and making
//! playlists, filling them, rating videos and commenting, and the channel's YouTube Studio
//! analytics (views, watch time, subscribers, where viewers come from).
//!
//! The YouTube Data API has a daily quota (10,000 units by default): a search costs 100 units,
//! a write 50, and a read 1, so the search description says to prefer the cheaper tools.

use serde_json::{Value, json};

use super::google::{Api, Failure};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

/// Manage the account over SSL: read, and everything below. (Posting comments needs this one.)
const MANAGE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
/// The channel's analytics (not its revenue, which has a scope of its own).
const ANALYTICS: &str = "https://www.googleapis.com/auth/yt-analytics.readonly";
const REPORTS: &str = "https://youtubeanalytics.googleapis.com/v2/reports";
/// What a report shows when no metrics are asked for.
const DEFAULT_METRICS: &str = "views,estimatedMinutesWatched,averageViewDuration,likes,comments,shares,subscribersGained,subscribersLost";
/// A report covers this many days up to today when no dates are given.
const DEFAULT_DAYS: i64 = 28;
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
        "Search videos, read their details and comments, see the person's channel, playlists and subscriptions, make playlists, rate videos and comment, and read the channel's analytics."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[MANAGE, ANALYTICS]
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
            ToolDef {
                name: "create_playlist",
                description: "Make a new playlist on the person's channel. Private unless told otherwise. Costs 50 units of quota.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "description": {"type": "string"},
                        "privacy": {"type": "string", "enum": ["private", "unlisted", "public"], "description": "Who can see it. Default private."}
                    },
                    "required": ["title"]
                }),
            },
            ToolDef {
                name: "add_to_playlist",
                description: "Add a video to one of the person's playlists. Costs 50 units of quota.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "playlist_id": {"type": "string"},
                        "video_id": {"type": "string"},
                        "position": {"type": "integer", "description": "Where in the playlist, counting from 1. Default: the end."}
                    },
                    "required": ["playlist_id", "video_id"]
                }),
            },
            ToolDef {
                name: "remove_from_playlist",
                description: "Take a video out of a playlist. It needs the playlist item's id that list_playlist_items shows after each video, not the video's id.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"playlist_item_id": {"type": "string"}},
                    "required": ["playlist_item_id"]
                }),
            },
            ToolDef {
                name: "rate_video",
                description: "Like or dislike a video as the person, or take the rating back.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "video_id": {"type": "string"},
                        "rating": {"type": "string", "enum": ["like", "dislike", "none"]}
                    },
                    "required": ["video_id", "rating"]
                }),
            },
            ToolDef {
                name: "post_comment",
                description: "Post a new comment on a video as the person. It is public at once. Costs 50 units of quota.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"video_id": {"type": "string"}, "text": {"type": "string"}},
                    "required": ["video_id", "text"]
                }),
            },
            ToolDef {
                name: "reply_to_comment",
                description: "Reply to a comment as the person, with the id get_comments shows in brackets before it. It is public at once. Costs 50 units of quota.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"comment_id": {"type": "string"}, "text": {"type": "string"}},
                    "required": ["comment_id", "text"]
                }),
            },
            ToolDef {
                name: "analytics_report",
                description: "A YouTube Studio analytics report for the person's own channel. Pick metrics (views, estimatedMinutesWatched, averageViewDuration, averageViewPercentage, likes, dislikes, comments, shares, subscribersGained, subscribersLost, annotationClickThroughRate, cardClickRate ...) and optionally a dimension to split by (day, month, video, country, deviceType, operatingSystem, insightTrafficSourceType, subscribedStatus, ageGroup, gender ...). Not every mix is allowed; Google says which when one is not. No dates means the last 28 days. Revenue is not available. Data runs a day or two behind. Needs the YouTube Analytics API turned on in the Google Cloud project.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "start_date": {"type": "string", "description": "YYYY-MM-DD. Default: 27 days before end_date."},
                        "end_date": {"type": "string", "description": "YYYY-MM-DD. Default: today."},
                        "metrics": {"type": "string", "description": "Comma-separated. Default: views, estimatedMinutesWatched, averageViewDuration, likes, comments, shares, subscribersGained, subscribersLost."},
                        "dimensions": {"type": "string", "description": "Comma-separated, to split the numbers by. Empty gives one row of totals."},
                        "filters": {"type": "string", "description": "Like video==VIDEO_ID, or country==US;deviceType==MOBILE (semicolons mean and)."},
                        "sort": {"type": "string", "description": "A metric or dimension, with - in front for biggest first, like -views."},
                        "max_results": {"type": "integer", "description": "How many rows, 1 to 200. Default 50."}
                    }
                }),
            },
            ToolDef {
                name: "top_videos",
                description: "The channel's best videos over a period, with their titles: views, watch time, average view duration, likes, comments and subscribers gained for each.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "start_date": {"type": "string", "description": "YYYY-MM-DD. Default: 27 days before end_date."},
                        "end_date": {"type": "string", "description": "YYYY-MM-DD. Default: today."},
                        "sort_by": {"type": "string", "enum": ["views", "estimatedMinutesWatched", "likes", "comments", "shares", "subscribersGained"], "description": "What makes a video the best. Default views."},
                        "max_results": {"type": "integer", "description": "How many videos, 1 to 50. Default 10."}
                    }
                }),
            },
            ToolDef {
                name: "video_analytics",
                description: "One video's analytics over a period: totals, or split by day, country, device or where viewers found it (insightTrafficSourceType).",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "video_id": {"type": "string"},
                        "start_date": {"type": "string", "description": "YYYY-MM-DD. Default: 27 days before end_date."},
                        "end_date": {"type": "string", "description": "YYYY-MM-DD. Default: today."},
                        "by": {"type": "string", "enum": ["day", "country", "deviceType", "insightTrafficSourceType"], "description": "Split the numbers by this. Default: one row of totals."}
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
            "create_playlist" => {
                let title = required(args, "title")?;
                let privacy = match arg(args, "privacy") {
                    None => "private",
                    Some(p @ ("private" | "unlisted" | "public")) => p,
                    Some(other) => {
                        return Err(Failure::other(format!(
                            "privacy must be private, unlisted or public, not {other}"
                        )));
                    }
                };
                let made = api.post(
                    &format!("{BASE}/playlists"),
                    &[("part", "snippet,status")],
                    &json!({
                        "snippet": {"title": title, "description": args["description"].as_str().unwrap_or_default()},
                        "status": {"privacyStatus": privacy}
                    }),
                )?;
                Ok(format!(
                    "Playlist made ({privacy}). Id {}.",
                    made["id"].as_str().unwrap_or("?")
                ))
            }
            "add_to_playlist" => {
                let (playlist, video) =
                    (required(args, "playlist_id")?, required(args, "video_id")?);
                let mut snippet = json!({"playlistId": playlist, "resourceId": {"kind": "youtube#video", "videoId": video}});
                if let Some(p) = args["position"].as_u64().filter(|p| *p >= 1) {
                    snippet["position"] = json!(p - 1);
                }
                let added = api.post(
                    &format!("{BASE}/playlistItems"),
                    &[("part", "snippet")],
                    &json!({"snippet": snippet}),
                )?;
                Ok(format!(
                    "Added. Playlist item id {}.",
                    added["id"].as_str().unwrap_or("?")
                ))
            }
            "remove_from_playlist" => {
                let id = required(args, "playlist_item_id")?;
                api.delete(&format!("{BASE}/playlistItems"), &[("id", id)])?;
                Ok("Removed from the playlist.".to_owned())
            }
            "rate_video" => {
                let id = required(args, "video_id")?;
                let rating = match arg(args, "rating") {
                    Some(r @ ("like" | "dislike" | "none")) => r,
                    _ => return Err(Failure::other("rating must be like, dislike or none")),
                };
                api.post(
                    &format!("{BASE}/videos/rate"),
                    &[("id", id), ("rating", rating)],
                    &json!({}),
                )?;
                Ok(format!("Rating set to {rating}."))
            }
            "post_comment" => {
                let (video, text) = (required(args, "video_id")?, required(args, "text")?);
                let made = api.post(
                    &format!("{BASE}/commentThreads"),
                    &[("part", "snippet")],
                    &json!({"snippet": {"videoId": video, "topLevelComment": {"snippet": {"textOriginal": text}}}}),
                )?;
                Ok(format!(
                    "Comment posted. Id {}.",
                    made["id"].as_str().unwrap_or("?")
                ))
            }
            "reply_to_comment" => {
                let (parent, text) = (required(args, "comment_id")?, required(args, "text")?);
                let made = api.post(
                    &format!("{BASE}/comments"),
                    &[("part", "snippet")],
                    &json!({"snippet": {"parentId": parent, "textOriginal": text}}),
                )?;
                Ok(format!(
                    "Reply posted. Id {}.",
                    made["id"].as_str().unwrap_or("?")
                ))
            }
            "analytics_report" => {
                let (start, end) = period(args, chrono::Utc::now().date_naive())?;
                let metrics =
                    tokens(args, "metrics")?.unwrap_or_else(|| DEFAULT_METRICS.to_owned());
                let max = args["max_results"]
                    .as_u64()
                    .unwrap_or(50)
                    .clamp(1, 200)
                    .to_string();
                let mut query = vec![
                    ("ids", "channel==MINE"),
                    ("startDate", start.as_str()),
                    ("endDate", end.as_str()),
                    ("metrics", metrics.as_str()),
                    ("maxResults", max.as_str()),
                ];
                let (dimensions, filters, sort) = (
                    tokens(args, "dimensions")?,
                    tokens(args, "filters")?,
                    tokens(args, "sort")?,
                );
                for (key, value) in [
                    ("dimensions", &dimensions),
                    ("filters", &filters),
                    ("sort", &sort),
                ] {
                    if let Some(v) = value {
                        query.push((key, v.as_str()));
                    }
                }
                Ok(render_report(&api.get(REPORTS, &query)?))
            }
            "top_videos" => top_videos(args, api),
            "video_analytics" => {
                let video = required(args, "video_id")?;
                let (start, end) = period(args, chrono::Utc::now().date_naive())?;
                let filter = format!("video=={video}");
                let mut query = vec![
                    ("ids", "channel==MINE"),
                    ("startDate", start.as_str()),
                    ("endDate", end.as_str()),
                    ("metrics", DEFAULT_METRICS),
                    ("filters", filter.as_str()),
                ];
                match arg(args, "by") {
                    None => {}
                    Some(by @ ("day" | "country" | "deviceType" | "insightTrafficSourceType")) => {
                        query.push(("dimensions", by));
                        query.push(("sort", if by == "day" { "day" } else { "-views" }));
                    }
                    Some(other) => {
                        return Err(Failure::other(format!(
                            "`by` must be day, country, deviceType or insightTrafficSourceType, not {other}"
                        )));
                    }
                }
                Ok(render_report(&api.get(REPORTS, &query)?))
            }
            other => Err(Failure::other(format!(
                "YouTube has no tool called {other}"
            ))),
        }
    }
}

/// The dates a report covers: `start_date` to `end_date`, as asked, else the 28 days up to `today`.
fn period(args: &Value, today: chrono::NaiveDate) -> Result<(String, String), Failure> {
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

/// A list of metric, dimension, filter or sort names: letters, digits and the few marks they use.
fn tokens(args: &Value, name: &str) -> Result<Option<String>, Failure> {
    match arg(args, name).map(str::trim) {
        None => Ok(None),
        Some(v)
            if v.chars()
                .all(|c| c.is_ascii_alphanumeric() || ",;=-_".contains(c)) =>
        {
            Ok(Some(v.to_owned()))
        }
        Some(v) => Err(Failure::other(format!(
            "`{name}` holds something that is not a name: {v:?}"
        ))),
    }
}

/// A report as a table: its column names, then a row per line, cells separated by tabs.
fn render_report(report: &Value) -> String {
    let columns: Vec<&str> = report["columnHeaders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["name"].as_str())
        .collect();
    let rows = report["rows"].as_array().map_or(&[][..], Vec::as_slice);
    if rows.is_empty() {
        return "No data for that period. Analytics run a day or two behind, and a new or small channel may have none.".to_owned();
    }
    let mut out = columns.join("\t");
    for row in rows {
        out.push('\n');
        let cells: Vec<String> = row.as_array().into_iter().flatten().map(cell).collect();
        out.push_str(&cells.join("\t"));
    }
    out
}

fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_i64().map_or_else(
            || {
                let f = n.as_f64().unwrap_or(0.0);
                format!("{f:.2}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_owned()
            },
            |i| i.to_string(),
        ),
        other => other.to_string(),
    }
}

fn top_videos(args: &Value, api: &Api) -> Result<String, Failure> {
    let (start, end) = period(args, chrono::Utc::now().date_naive())?;
    let by = match arg(args, "sort_by") {
        None => "views",
        Some(
            m @ ("views"
            | "estimatedMinutesWatched"
            | "likes"
            | "comments"
            | "shares"
            | "subscribersGained"),
        ) => m,
        Some(other) => {
            return Err(Failure::other(format!(
                "`sort_by` must be views, estimatedMinutesWatched, likes, comments, shares or subscribersGained, not {other}"
            )));
        }
    };
    let sort = format!("-{by}");
    let max = args["max_results"]
        .as_u64()
        .unwrap_or(10)
        .clamp(1, 50)
        .to_string();
    let report = api.get(
        REPORTS,
        &[
            ("ids", "channel==MINE"),
            ("startDate", start.as_str()),
            ("endDate", end.as_str()),
            ("dimensions", "video"),
            ("metrics", "views,estimatedMinutesWatched,averageViewDuration,likes,comments,subscribersGained"),
            ("sort", sort.as_str()),
            ("maxResults", max.as_str()),
        ],
    )?;
    let ids: Vec<&str> = report["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r[0].as_str())
        .collect();
    if ids.is_empty() {
        return Ok(render_report(&report));
    }
    let joined = ids.join(",");
    let titles = api.get(
        &format!("{BASE}/videos"),
        &[("part", "snippet"), ("id", joined.as_str())],
    )?;
    Ok(render_top(&report, &titles))
}

/// The best videos, each with its title: a report whose first column is the video's id.
fn render_top(report: &Value, titles: &Value) -> String {
    let title_of = |id: &str| {
        items(titles)
            .iter()
            .find(|v| v["id"].as_str() == Some(id))
            .and_then(|v| v["snippet"]["title"].as_str())
            .unwrap_or("(title not found)")
            .to_owned()
    };
    let names: Vec<&str> = report["columnHeaders"]
        .as_array()
        .into_iter()
        .flatten()
        .skip(1)
        .filter_map(|c| c["name"].as_str())
        .collect();
    let mut lines = Vec::new();
    for (i, row) in report["rows"].as_array().into_iter().flatten().enumerate() {
        let cells = row.as_array().map_or(&[][..], Vec::as_slice);
        let id = cells.first().and_then(Value::as_str).unwrap_or("?");
        let numbers: Vec<String> = names
            .iter()
            .zip(cells.iter().skip(1))
            .map(|(n, v)| format!("{n} {}", cell(v)))
            .collect();
        lines.push(format!(
            "{}. {} ({id}): {}",
            i + 1,
            title_of(id),
            numbers.join(", ")
        ));
    }
    lines.join("\n")
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
        "{} · {} · {} · {} · item {}",
        s["position"]
            .as_u64()
            .map_or_else(|| "?".to_owned(), |p| (p + 1).to_string()),
        s["resourceId"]["videoId"].as_str().unwrap_or("?"),
        s["title"].as_str().unwrap_or("?"),
        s["videoOwnerChannelTitle"].as_str().unwrap_or("?"),
        i["id"].as_str().unwrap_or("?")
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
        "[{}] {} ({} likes, {} replies): {}",
        t["id"].as_str().unwrap_or("?"),
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
        let item = json!({"id": "PI1", "snippet": {"position": 0, "resourceId": {"videoId": "v"}, "title": "T", "videoOwnerChannelTitle": "C"}});
        assert_eq!(playlist_item_line(&item), "1 · v · T · C · item PI1");
        let thread = json!({"id": "CT1", "snippet": {"totalReplyCount": 2, "topLevelComment": {"snippet": {"authorDisplayName": "Ann", "likeCount": 7, "textDisplay": "a\nb"}}}});
        assert_eq!(comment_line(&thread), "[CT1] Ann (7 likes, 2 replies): a b");
        assert_eq!(
            playlist_line(
                &json!({"id": "P", "snippet": {"title": "Mix"}, "contentDetails": {"itemCount": 4}})
            ),
            "P · Mix · 4 videos"
        );
    }

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
    fn names_in_a_report_request_are_plain() {
        assert_eq!(
            tokens(&json!({"filters": "video==abc;country==US"}), "filters")
                .unwrap()
                .as_deref(),
            Some("video==abc;country==US")
        );
        assert_eq!(tokens(&json!({}), "metrics").unwrap(), None);
        assert!(tokens(&json!({"metrics": "views&ids=channel==OTHER"}), "metrics").is_err());
        assert!(tokens(&json!({"sort": "-views "}), "sort").is_ok());
    }

    #[test]
    fn a_report_is_a_table_with_whole_numbers_and_short_decimals() {
        let report = json!({
            "columnHeaders": [{"name": "day"}, {"name": "views"}, {"name": "averageViewDuration"}],
            "rows": [["2025-03-01", 120, 61.5], ["2025-03-02", 80, 59.123456]]
        });
        assert_eq!(
            render_report(&report),
            "day\tviews\taverageViewDuration\n2025-03-01\t120\t61.5\n2025-03-02\t80\t59.12"
        );
        assert!(
            render_report(&json!({"columnHeaders": []})).starts_with("No data for that period")
        );
    }

    #[test]
    fn the_best_videos_get_their_titles_and_numbers() {
        let report = json!({
            "columnHeaders": [{"name": "video"}, {"name": "views"}, {"name": "likes"}],
            "rows": [["v1", 900, 40], ["v2", 5, 1]]
        });
        let titles = json!({"items": [{"id": "v1", "snippet": {"title": "Big one"}}]});
        assert_eq!(
            render_top(&report, &titles),
            "1. Big one (v1): views 900, likes 40\n2. (title not found) (v2): views 5, likes 1"
        );
    }

    #[test]
    fn a_page_is_between_one_and_fifty() {
        assert_eq!(max_results(&json!({})), "10");
        assert_eq!(max_results(&json!({"max_results": 0})), "1");
        assert_eq!(max_results(&json!({"max_results": 999})), "50");
    }

    #[test]
    fn every_tool_is_described_and_the_writes_are_marked() {
        let tools = YouTube.tools();
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
        assert_eq!(
            changing,
            [
                "create_playlist",
                "add_to_playlist",
                "remove_from_playlist",
                "rate_video",
                "post_comment",
                "reply_to_comment"
            ]
        );
        assert!(required(&json!({}), "ids").is_err());
    }
}
