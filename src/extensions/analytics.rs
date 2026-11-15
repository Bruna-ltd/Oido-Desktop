//! Google Analytics (GA4): find the person's properties, and read their reports, past and the last
//! half hour. Read only: it changes nothing in Analytics.
//!
//! Reports come from the Analytics Data API and the list of properties from the Analytics Admin
//! API; both must be turned on in the person's Google Cloud project.

use serde_json::{Value, json};

use super::google::{Api, Failure};
use super::report::{arg, limit, names, recent, required};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

const READONLY: &str = "https://www.googleapis.com/auth/analytics.readonly";
const DATA: &str = "https://analyticsdata.googleapis.com/v1beta";
const SUMMARIES: &str = "https://analyticsadmin.googleapis.com/v1beta/accountSummaries";
/// What a report shows when no metrics are asked for.
const DEFAULT_METRICS: &str =
    "activeUsers,newUsers,sessions,screenPageViews,engagementRate,averageSessionDuration";
const DEFAULT_ROWS: u64 = 50;
const MAX_ROWS: u64 = 1000;
/// How a filter may match a dimension's value, as the Data API names it.
const MATCHES: &[(&str, &str)] = &[
    ("exact", "EXACT"),
    ("contains", "CONTAINS"),
    ("begins_with", "BEGINS_WITH"),
    ("ends_with", "ENDS_WITH"),
    ("regex", "FULL_REGEXP"),
];

pub struct Analytics;

impl Extension for Analytics {
    fn title(&self) -> &'static str {
        "Google Analytics"
    }

    fn logo(&self) -> Logo {
        super::marks::ANALYTICS
    }

    fn blurb(&self) -> &'static str {
        "Read the person's Google Analytics (GA4): visitors, pages, traffic sources, events and who is on the site now."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[READONLY]
    }

    fn tools(&self) -> Vec<ToolDef> {
        let property = json!({"type": "string", "description": "The GA4 property's id, the number list_properties gives (like 123456789)."});
        let start = json!({"type": "string", "description": "YYYY-MM-DD. Default: 27 days before end_date."});
        let end = json!({"type": "string", "description": "YYYY-MM-DD. Default: today."});
        let rows = |default: u64, max: u64| json!({"type": "integer", "description": format!("How many rows, 1 to {max}. Default {default}.")});
        vec![
            ToolDef {
                name: "list_properties",
                description: "The Google Analytics accounts and GA4 properties the person can see, with each property's id. Needs the Google Analytics Admin API turned on in the Google Cloud project.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {}}),
            },
            ToolDef {
                name: "run_report",
                description: "A GA4 report for a property over a period. Pick metrics (activeUsers, newUsers, totalUsers, sessions, engagedSessions, engagementRate, bounceRate, averageSessionDuration, screenPageViews, eventCount, keyEvents, totalRevenue, ...) and optionally dimensions to split by (date, pagePath, pageTitle, landingPage, country, city, deviceCategory, browser, sessionSource, sessionMedium, sessionDefaultChannelGroup, eventName, ...). Not every mix is allowed; Google says which when one is not. No dates means the last 28 days. Data for today is not final.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "property": property,
                        "start_date": start,
                        "end_date": end,
                        "metrics": {"type": "string", "description": "Comma-separated, at most 10. Default: activeUsers, newUsers, sessions, screenPageViews, engagementRate, averageSessionDuration."},
                        "dimensions": {"type": "string", "description": "Comma-separated, at most 9. Empty gives one row of totals."},
                        "filters": filters_schema(),
                        "sort": {"type": "string", "description": "A metric or dimension in the report, with - in front for biggest first, like -sessions. Default: biggest first by the first metric."},
                        "max_results": rows(DEFAULT_ROWS, MAX_ROWS)
                    },
                    "required": ["property"]
                }),
            },
            ToolDef {
                name: "top_pages",
                description: "A property's most viewed pages over a period, with views, users and total engagement time in seconds.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {"property": property, "start_date": start, "end_date": end, "max_results": rows(20, 200)},
                    "required": ["property"]
                }),
            },
            ToolDef {
                name: "traffic_sources",
                description: "Where a property's visitors came from over a period: by channel (Organic Search, Direct, Referral, ...) or by source and medium (google / organic), with sessions, users and engagement.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "property": property,
                        "start_date": start,
                        "end_date": end,
                        "by": {"type": "string", "enum": ["channel", "source"], "description": "Default channel."},
                        "max_results": rows(20, 200)
                    },
                    "required": ["property"]
                }),
            },
            ToolDef {
                name: "realtime_report",
                description: "Who is on a property in the last 30 minutes: active users, optionally split by a dimension (country, city, deviceCategory, unifiedScreenName (the page), eventName, platform).",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "property": property,
                        "metrics": {"type": "string", "description": "Comma-separated. Default activeUsers. Others: screenPageViews, eventCount, keyEvents."},
                        "dimensions": {"type": "string", "description": "Comma-separated. Empty gives the total."},
                        "max_results": rows(DEFAULT_ROWS, MAX_ROWS)
                    },
                    "required": ["property"]
                }),
            },
        ]
    }

    fn call(&self, tool: &str, args: &Value, api: &Api) -> Result<String, Failure> {
        match tool {
            "list_properties" => list_properties(api),
            "run_report" => {
                let metrics = names(args, "metrics");
                let metrics = if metrics.is_empty() {
                    DEFAULT_METRICS.split(',').map(str::to_owned).collect()
                } else {
                    metrics
                };
                let dimensions = names(args, "dimensions");
                let mut body = report_body(args, &metrics, &dimensions, DEFAULT_ROWS, MAX_ROWS)?;
                if let Some(filter) = filter_of(args)? {
                    body["dimensionFilter"] = filter;
                }
                body["orderBys"] = json!([order_of(arg(args, "sort"), &metrics, &dimensions)?]);
                let got = api.post(&format!("{}:runReport", property_url(args)?), &[], &body)?;
                Ok(render(&got))
            }
            "top_pages" => {
                let metrics =
                    ["screenPageViews", "activeUsers", "userEngagementDuration"].map(str::to_owned);
                let dimensions = ["pagePath", "pageTitle"].map(str::to_owned);
                let mut body = report_body(args, &metrics, &dimensions, 20, 200)?;
                body["orderBys"] =
                    json!([{"metric": {"metricName": "screenPageViews"}, "desc": true}]);
                let got = api.post(&format!("{}:runReport", property_url(args)?), &[], &body)?;
                Ok(render(&got))
            }
            "traffic_sources" => {
                let dimensions: Vec<String> = match arg(args, "by") {
                    None | Some("channel") => vec!["sessionDefaultChannelGroup".into()],
                    Some("source") => vec!["sessionSource".into(), "sessionMedium".into()],
                    Some(other) => {
                        return Err(Failure::other(format!(
                            "`by` must be channel or source, not {other}"
                        )));
                    }
                };
                let metrics =
                    ["sessions", "activeUsers", "engagementRate", "keyEvents"].map(str::to_owned);
                let mut body = report_body(args, &metrics, &dimensions, 20, 200)?;
                body["orderBys"] = json!([{"metric": {"metricName": "sessions"}, "desc": true}]);
                let got = api.post(&format!("{}:runReport", property_url(args)?), &[], &body)?;
                Ok(render(&got))
            }
            "realtime_report" => {
                let metrics = names(args, "metrics");
                let metrics = if metrics.is_empty() {
                    vec!["activeUsers".to_owned()]
                } else {
                    metrics
                };
                let dimensions = names(args, "dimensions");
                let body = json!({
                    "metrics": metrics.iter().map(|m| json!({"name": m})).collect::<Vec<_>>(),
                    "dimensions": dimensions.iter().map(|d| json!({"name": d})).collect::<Vec<_>>(),
                    "limit": limit(args, DEFAULT_ROWS, MAX_ROWS),
                });
                let got = api.post(
                    &format!("{}:runRealtimeReport", property_url(args)?),
                    &[],
                    &body,
                )?;
                Ok(render(&got))
            }
            other => Err(Failure::other(format!(
                "Google Analytics has no tool called {other}"
            ))),
        }
    }
}

fn filters_schema() -> Value {
    json!({
        "type": "array",
        "description": "Keep only rows whose dimensions match all of these.",
        "items": {
            "type": "object",
            "properties": {
                "dimension": {"type": "string", "description": "Like pagePath or country."},
                "match": {"type": "string", "enum": MATCHES.iter().map(|(m, _)| *m).collect::<Vec<_>>(), "description": "Default exact."},
                "value": {"type": "string"},
                "exclude": {"type": "boolean", "description": "Keep the rows that do not match instead."}
            },
            "required": ["dimension", "value"]
        }
    })
}

/// The address of the property `args` names. A property is a number, given bare or as
/// `properties/123`; anything else is refused, so it cannot reach another address.
fn property_url(args: &Value) -> Result<String, Failure> {
    let given = required(args, "property")?.trim();
    let id = given.strip_prefix("properties/").unwrap_or(given);
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Failure::other(format!(
            "`property` must be a GA4 property id, a number like 123456789, not {given:?}. list_properties gives them."
        )));
    }
    Ok(format!("{DATA}/properties/{id}"))
}

/// The parts of a report request every past-dates tool shares.
fn report_body(
    args: &Value,
    metrics: &[String],
    dimensions: &[String],
    default_rows: u64,
    max_rows: u64,
) -> Result<Value, Failure> {
    let (start, end) = recent(args)?;
    Ok(json!({
        "dateRanges": [{"startDate": start, "endDate": end}],
        "metrics": metrics.iter().map(|m| json!({"name": m})).collect::<Vec<_>>(),
        "dimensions": dimensions.iter().map(|d| json!({"name": d})).collect::<Vec<_>>(),
        "limit": limit(args, default_rows, max_rows),
    }))
}

/// The `filters` argument as a Data API filter: every one must hold.
fn filter_of(args: &Value) -> Result<Option<Value>, Failure> {
    let Some(list) = args["filters"].as_array().filter(|l| !l.is_empty()) else {
        return Ok(None);
    };
    let mut all = Vec::with_capacity(list.len());
    for f in list {
        let dimension = required(f, "dimension")?;
        let value = f["value"]
            .as_str()
            .ok_or_else(|| Failure::other("a filter needs a `value`"))?;
        let wanted = arg(f, "match").unwrap_or("exact");
        let (_, kind) = MATCHES
            .iter()
            .find(|(m, _)| *m == wanted)
            .ok_or_else(|| {
                Failure::other(format!(
                    "a filter's `match` must be exact, contains, begins_with, ends_with or regex, not {wanted}"
                ))
            })?;
        let one = json!({"filter": {
            "fieldName": dimension,
            "stringFilter": {"matchType": kind, "value": value, "caseSensitive": false}
        }});
        all.push(if f["exclude"].as_bool() == Some(true) {
            json!({"notExpression": one})
        } else {
            one
        });
    }
    Ok(Some(if all.len() == 1 {
        all.remove(0)
    } else {
        json!({"andGroup": {"expressions": all}})
    }))
}

/// How the rows are ordered: by `sort` if given (a metric or dimension of the report), else
/// biggest first by the first metric.
fn order_of(
    sort: Option<&str>,
    metrics: &[String],
    dimensions: &[String],
) -> Result<Value, Failure> {
    let Some(sort) = sort.map(str::trim) else {
        return Ok(json!({"metric": {"metricName": metrics[0]}, "desc": true}));
    };
    let (name, desc) = match sort.strip_prefix('-') {
        Some(n) => (n, true),
        None => (sort, false),
    };
    if metrics.iter().any(|m| m == name) {
        Ok(json!({"metric": {"metricName": name}, "desc": desc}))
    } else if dimensions.iter().any(|d| d == name) {
        Ok(json!({"dimension": {"dimensionName": name}, "desc": desc}))
    } else {
        Err(Failure::other(format!(
            "`sort` must name a metric or dimension in the report, not {name}"
        )))
    }
}

fn list_properties(api: &Api) -> Result<String, Failure> {
    let mut lines = Vec::new();
    let mut token = String::new();
    // A person sees few accounts; five pages of 200 is more than anyone has.
    for _ in 0..5 {
        let mut query = vec![("pageSize", "200")];
        if !token.is_empty() {
            query.push(("pageToken", token.as_str()));
        }
        let page = api.get(SUMMARIES, &query)?;
        lines.extend(summary_lines(&page));
        match page["nextPageToken"].as_str().filter(|t| !t.is_empty()) {
            Some(next) => token = next.to_owned(),
            None => break,
        }
    }
    if lines.is_empty() {
        return Ok("This Google account sees no Google Analytics properties.".to_owned());
    }
    Ok(lines.join("\n"))
}

/// An account per line, its properties under it with their ids.
fn summary_lines(page: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    for account in page["accountSummaries"].as_array().into_iter().flatten() {
        lines.push(format!(
            "Account: {}",
            account["displayName"].as_str().unwrap_or("?")
        ));
        for p in account["propertySummaries"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let id = p["property"].as_str().unwrap_or("?");
            lines.push(format!(
                "- {} · property {}",
                p["displayName"].as_str().unwrap_or("?"),
                id.strip_prefix("properties/").unwrap_or(id)
            ));
        }
    }
    lines
}

/// A report as a table: the column names, then a row per line, cells separated by tabs.
fn render(report: &Value) -> String {
    let headers = |key: &str| -> Vec<String> {
        report[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| h["name"].as_str().map(str::to_owned))
            .collect()
    };
    let rows = report["rows"].as_array().map_or(&[][..], Vec::as_slice);
    if rows.is_empty() {
        return "No data for that period. A new property, a filter that matches nothing, or a site without the GA4 tag gives none.".to_owned();
    }
    let mut out = [headers("dimensionHeaders"), headers("metricHeaders")]
        .concat()
        .join("\t");
    for row in rows {
        let values = |key: &str| {
            row[key]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| v["value"].as_str().unwrap_or(""))
                .collect::<Vec<_>>()
        };
        let mut cells: Vec<String> = values("dimensionValues")
            .into_iter()
            .map(|v| v.replace(['\t', '\n'], " "))
            .collect();
        cells.extend(values("metricValues").into_iter().map(number));
        out.push('\n');
        out.push_str(&cells.join("\t"));
    }
    if let Some(total) = report["rowCount"].as_u64()
        && total > rows.len() as u64
    {
        out.push_str(&format!(
            "\n[{} of {total} rows. Ask for more with max_results, or narrow with filters.]",
            rows.len()
        ));
    }
    out
}

/// A metric's value, which the API sends as text: whole numbers stay, others go to two places.
fn number(v: &str) -> String {
    match v.parse::<f64>() {
        Ok(f) if v.contains('.') || v.contains('e') => super::report::decimal(f),
        _ => v.to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_property_is_a_number_bare_or_prefixed() {
        assert_eq!(
            property_url(&json!({"property": "123"})).unwrap(),
            format!("{DATA}/properties/123")
        );
        assert_eq!(
            property_url(&json!({"property": "properties/45"})).unwrap(),
            format!("{DATA}/properties/45")
        );
        for bad in ["", "12a", "1/../2", "properties/", "G-ABC123"] {
            assert!(property_url(&json!({"property": bad})).is_err(), "{bad}");
        }
    }

    #[test]
    fn filters_become_one_filter_or_an_and_group() {
        assert_eq!(filter_of(&json!({})).unwrap(), None);
        let one = filter_of(&json!({"filters": [{"dimension": "country", "value": "Cyprus"}]}))
            .unwrap()
            .unwrap();
        assert_eq!(one["filter"]["fieldName"], "country");
        assert_eq!(one["filter"]["stringFilter"]["matchType"], "EXACT");
        let two = filter_of(&json!({"filters": [
            {"dimension": "pagePath", "match": "begins_with", "value": "/blog"},
            {"dimension": "country", "value": "US", "exclude": true}
        ]}))
        .unwrap()
        .unwrap();
        let all = two["andGroup"]["expressions"].as_array().unwrap();
        assert_eq!(all[0]["filter"]["stringFilter"]["matchType"], "BEGINS_WITH");
        assert_eq!(all[1]["notExpression"]["filter"]["fieldName"], "country");
        assert!(
            filter_of(&json!({"filters": [{"dimension": "x", "match": "like", "value": "y"}]}))
                .is_err()
        );
        assert!(filter_of(&json!({"filters": [{"dimension": "x"}]})).is_err());
    }

    #[test]
    fn sorting_names_a_column_of_the_report() {
        let m = ["sessions".to_owned(), "activeUsers".to_owned()];
        let d = ["country".to_owned()];
        assert_eq!(
            order_of(None, &m, &d).unwrap(),
            json!({"metric": {"metricName": "sessions"}, "desc": true})
        );
        assert_eq!(
            order_of(Some("-activeUsers"), &m, &d).unwrap(),
            json!({"metric": {"metricName": "activeUsers"}, "desc": true})
        );
        assert_eq!(
            order_of(Some("country"), &m, &d).unwrap(),
            json!({"dimension": {"dimensionName": "country"}, "desc": false})
        );
        assert!(order_of(Some("views"), &m, &d).is_err());
    }

    #[test]
    fn a_report_is_a_table_and_says_when_rows_were_left_out() {
        let report = json!({
            "dimensionHeaders": [{"name": "country"}],
            "metricHeaders": [{"name": "sessions"}, {"name": "engagementRate"}],
            "rows": [
                {"dimensionValues": [{"value": "Cyprus"}], "metricValues": [{"value": "120"}, {"value": "0.6123"}]},
                {"dimensionValues": [{"value": "Greece"}], "metricValues": [{"value": "80"}, {"value": "0.5"}]}
            ],
            "rowCount": 9
        });
        assert_eq!(
            render(&report),
            "country\tsessions\tengagementRate\nCyprus\t120\t0.61\nGreece\t80\t0.5\n[2 of 9 rows. Ask for more with max_results, or narrow with filters.]"
        );
        assert!(render(&json!({})).starts_with("No data for that period"));
    }

    #[test]
    fn properties_are_listed_under_their_accounts_with_bare_ids() {
        let page = json!({"accountSummaries": [{"displayName": "Oido", "propertySummaries": [
            {"property": "properties/111", "displayName": "oido.app"}
        ]}]});
        assert_eq!(
            summary_lines(&page),
            ["Account: Oido", "- oido.app · property 111"]
        );
    }

    #[test]
    fn every_tool_reads_and_is_described() {
        for t in Analytics.tools() {
            assert!(
                !t.description.is_empty()
                    && t.schema["type"] == "object"
                    && t.access == Access::Reads,
                "{}",
                t.name
            );
        }
    }
}
