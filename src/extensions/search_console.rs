//! Google Search Console: the person's sites, how they do in Google Search (queries, pages,
//! clicks, position), whether Google has indexed a page, and their sitemaps, which it can also
//! submit.
//!
//! Search analytics and sitemaps come from the Search Console API (`webmasters/v3`), the URL
//! inspection from its newer `searchconsole/v1`; one API in Google Cloud covers both.

use serde_json::{Value, json};

use super::google::{Api, Failure, segment};
use super::report::{arg, cell, decimal, limit, names, recent, required};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

/// Read and change: submitting a sitemap needs more than the read-only scope.
const WEBMASTERS: &str = "https://www.googleapis.com/auth/webmasters";
const SITES: &str = "https://www.googleapis.com/webmasters/v3/sites";
const INSPECT: &str = "https://searchconsole.googleapis.com/v1/urlInspection/index:inspect";
const DIMENSIONS: &[&str] = &[
    "query",
    "page",
    "country",
    "device",
    "date",
    "searchAppearance",
];
const TYPES: &[&str] = &["web", "image", "video", "news", "discover", "googleNews"];
/// How a filter may match, as the API names it.
const OPERATORS: &[&str] = &[
    "contains",
    "equals",
    "notContains",
    "notEquals",
    "includingRegex",
    "excludingRegex",
];
const DEFAULT_ROWS: u64 = 50;
const MAX_ROWS: u64 = 1000;

pub struct SearchConsole;

impl Extension for SearchConsole {
    fn title(&self) -> &'static str {
        "Google Search Console"
    }

    fn logo(&self) -> Logo {
        super::marks::SEARCH_CONSOLE
    }

    fn blurb(&self) -> &'static str {
        "See how the person's sites do in Google Search: queries, pages, clicks and position, whether a page is indexed, and their sitemaps."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[WEBMASTERS]
    }

    fn tools(&self) -> Vec<ToolDef> {
        let site = json!({"type": "string", "description": "The site as list_sites names it: https://example.com/ for a URL-prefix site, or sc-domain:example.com for a domain site."});
        vec![
            ToolDef {
                name: "list_sites",
                description: "The sites in the person's Search Console, with their permission level.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {}}),
            },
            ToolDef {
                name: "search_analytics",
                description: "How a site did in Google Search over a period: clicks, impressions, CTR and average position, split by query, page, country, device, date or searchAppearance. Rows come biggest first by clicks. No dates means the last 28 days; the last two or three days are usually not in yet. Data goes back 16 months.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "site": site,
                        "start_date": {"type": "string", "description": "YYYY-MM-DD. Default: 27 days before end_date."},
                        "end_date": {"type": "string", "description": "YYYY-MM-DD. Default: today."},
                        "dimensions": {"type": "string", "description": "Comma-separated, from query, page, country, device, date, searchAppearance. Empty gives one row of totals."},
                        "type": {"type": "string", "enum": TYPES, "description": "Which results. Default web."},
                        "filters": {
                            "type": "array",
                            "description": "Keep only rows that match all of these.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "dimension": {"type": "string", "enum": ["query", "page", "country", "device", "searchAppearance"]},
                                    "operator": {"type": "string", "enum": OPERATORS, "description": "Default contains. country takes a three-letter code (cyp), device DESKTOP, MOBILE or TABLET."},
                                    "expression": {"type": "string"}
                                },
                                "required": ["dimension", "expression"]
                            }
                        },
                        "max_results": {"type": "integer", "description": format!("How many rows, 1 to {MAX_ROWS}. Default {DEFAULT_ROWS}.")},
                        "start_row": {"type": "integer", "description": "Skip this many rows, to page on. Default 0."}
                    },
                    "required": ["site"]
                }),
            },
            ToolDef {
                name: "inspect_url",
                description: "What Google knows about one page of a site: indexed or not and why, last crawl, canonical, robots.txt, mobile usability and rich results. At most 2,000 a day per site.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "site": site,
                        "url": {"type": "string", "description": "The full address of the page, inside the site."}
                    },
                    "required": ["site", "url"]
                }),
            },
            ToolDef {
                name: "list_sitemaps",
                description: "A site's sitemaps: when each was submitted and read, errors and warnings, and how many pages it lists.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {"site": site}, "required": ["site"]}),
            },
            ToolDef {
                name: "submit_sitemap",
                description: "Submit a sitemap for a site, or submit one again so Google reads it soon.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "site": site,
                        "sitemap": {"type": "string", "description": "The sitemap's full address, like https://example.com/sitemap.xml."}
                    },
                    "required": ["site", "sitemap"]
                }),
            },
        ]
    }

    fn call(&self, tool: &str, args: &Value, api: &Api) -> Result<String, Failure> {
        match tool {
            "list_sites" => {
                let got = api.get(SITES, &[])?;
                Ok(render_sites(&got))
            }
            "search_analytics" => {
                let body = query_body(args)?;
                let url = format!("{}/searchAnalytics/query", site_url(args)?);
                Ok(render_rows(&api.post(&url, &[], &body)?, &body))
            }
            "inspect_url" => {
                let body = json!({
                    "inspectionUrl": required(args, "url")?,
                    "siteUrl": required(args, "site")?,
                });
                Ok(render_inspection(&api.post(INSPECT, &[], &body)?))
            }
            "list_sitemaps" => {
                let got = api.get(&format!("{}/sitemaps", site_url(args)?), &[])?;
                Ok(render_sitemaps(&got))
            }
            "submit_sitemap" => {
                let sitemap = required(args, "sitemap")?;
                let url = format!("{}/sitemaps/{}", site_url(args)?, segment(sitemap));
                api.put(&url, &[], &json!({}))?;
                Ok(format!(
                    "Sitemap {sitemap} submitted. Google reads it in its own time; list_sitemaps shows when."
                ))
            }
            other => Err(Failure::other(format!(
                "Google Search Console has no tool called {other}"
            ))),
        }
    }
}

/// The address of the site `args` names, escaped into one piece of the path.
fn site_url(args: &Value) -> Result<String, Failure> {
    Ok(format!("{SITES}/{}", segment(required(args, "site")?)))
}

/// A search analytics request from the tool's arguments, each checked against what the API takes.
fn query_body(args: &Value) -> Result<Value, Failure> {
    let (start, end) = recent(args)?;
    let dimensions = names(args, "dimensions");
    if let Some(bad) = dimensions
        .iter()
        .find(|d| !DIMENSIONS.contains(&d.as_str()))
    {
        return Err(Failure::other(format!(
            "`dimensions` takes {}, not {bad}",
            DIMENSIONS.join(", ")
        )));
    }
    let kind = arg(args, "type").unwrap_or("web");
    if !TYPES.contains(&kind) {
        return Err(Failure::other(format!(
            "`type` must be one of {}, not {kind}",
            TYPES.join(", ")
        )));
    }
    let mut body = json!({
        "startDate": start,
        "endDate": end,
        "dimensions": dimensions,
        "type": kind,
        "rowLimit": limit(args, DEFAULT_ROWS, MAX_ROWS),
        "startRow": args["start_row"].as_u64().unwrap_or(0),
    });
    let filters = filters_of(args)?;
    if !filters.is_empty() {
        body["dimensionFilterGroups"] = json!([{"groupType": "and", "filters": filters}]);
    }
    Ok(body)
}

fn filters_of(args: &Value) -> Result<Vec<Value>, Failure> {
    let mut out = Vec::new();
    for f in args["filters"].as_array().into_iter().flatten() {
        let dimension = required(f, "dimension")?;
        if dimension == "date" || !DIMENSIONS.contains(&dimension) {
            return Err(Failure::other(format!(
                "a filter's `dimension` must be query, page, country, device or searchAppearance, not {dimension}"
            )));
        }
        let operator = arg(f, "operator").unwrap_or("contains");
        if !OPERATORS.contains(&operator) {
            return Err(Failure::other(format!(
                "a filter's `operator` must be one of {}, not {operator}",
                OPERATORS.join(", ")
            )));
        }
        out.push(json!({
            "dimension": dimension,
            "operator": operator,
            "expression": required(f, "expression")?,
        }));
    }
    Ok(out)
}

fn render_sites(got: &Value) -> String {
    let lines: Vec<String> = got["siteEntry"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            format!(
                "{} · {}",
                s["siteUrl"].as_str().unwrap_or("?"),
                s["permissionLevel"].as_str().unwrap_or("?")
            )
        })
        .collect();
    if lines.is_empty() {
        return "This Google account has no sites in Search Console.".to_owned();
    }
    lines.join("\n")
}

/// The rows as a table: the asked dimensions, then clicks, impressions, CTR (as a percentage)
/// and average position.
fn render_rows(got: &Value, asked: &Value) -> String {
    let rows = got["rows"].as_array().map_or(&[][..], Vec::as_slice);
    if rows.is_empty() {
        return "No data for that period and filter. The last two or three days are usually not in yet.".to_owned();
    }
    let mut columns: Vec<&str> = asked["dimensions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    columns.extend(["clicks", "impressions", "ctr%", "position"]);
    let mut out = columns.join("\t");
    for row in rows {
        let mut cells: Vec<String> = row["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|k| cell(k).replace(['\t', '\n'], " "))
            .collect();
        cells.push(cell(&row["clicks"]));
        cells.push(cell(&row["impressions"]));
        cells.push(decimal(row["ctr"].as_f64().unwrap_or(0.0) * 100.0));
        cells.push(decimal(row["position"].as_f64().unwrap_or(0.0)));
        out.push('\n');
        out.push_str(&cells.join("\t"));
    }
    let asked_rows = asked["rowLimit"].as_u64().unwrap_or(0);
    if rows.len() as u64 == asked_rows {
        out.push_str(&format!(
            "\n[There may be more: ask with start_row {}.]",
            asked["startRow"].as_u64().unwrap_or(0) + asked_rows
        ));
    }
    out
}

fn render_inspection(got: &Value) -> String {
    let r = &got["inspectionResult"];
    let index = &r["indexStatusResult"];
    let text = |v: &Value| v.as_str().unwrap_or("-").to_owned();
    let mut lines = vec![
        format!("Verdict: {}", text(&index["verdict"])),
        format!("Coverage: {}", text(&index["coverageState"])),
        format!("Indexing allowed: {}", text(&index["indexingState"])),
        format!("robots.txt: {}", text(&index["robotsTxtState"])),
        format!("Page fetch: {}", text(&index["pageFetchState"])),
        format!("Last crawl: {}", text(&index["lastCrawlTime"])),
        format!("Crawled as: {}", text(&index["crawledAs"])),
        format!("Google's canonical: {}", text(&index["googleCanonical"])),
        format!("Declared canonical: {}", text(&index["userCanonical"])),
    ];
    let listed = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    for (what, key) in [("In sitemaps", "sitemap"), ("Linked from", "referringUrls")] {
        let found = listed(&index[key]);
        if !found.is_empty() {
            lines.push(format!("{what}: {}", found.join(", ")));
        }
    }
    if let Some(verdict) = r["mobileUsabilityResult"]["verdict"].as_str() {
        lines.push(format!("Mobile usability: {verdict}"));
    }
    if let Some(verdict) = r["richResultsResult"]["verdict"].as_str() {
        let kinds: Vec<&str> = r["richResultsResult"]["detectedItems"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["richResultType"].as_str())
            .collect();
        lines.push(format!("Rich results: {verdict} ({})", kinds.join(", ")));
    }
    if let Some(link) = r["inspectionResultLink"].as_str() {
        lines.push(format!("In Search Console: {link}"));
    }
    lines.join("\n")
}

fn render_sitemaps(got: &Value) -> String {
    let lines: Vec<String> = got["sitemap"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            let pages: Vec<String> = s["contents"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    format!(
                        "{} {}",
                        cell(&c["submitted"]),
                        c["type"].as_str().unwrap_or("items")
                    )
                })
                .collect();
            format!(
                "{} · submitted {} · read {} · {} errors, {} warnings{}{}",
                s["path"].as_str().unwrap_or("?"),
                day(&s["lastSubmitted"]),
                day(&s["lastDownloaded"]),
                cell(&s["errors"]),
                cell(&s["warnings"]),
                if s["isPending"].as_bool() == Some(true) {
                    " · waiting to be read"
                } else {
                    ""
                },
                if pages.is_empty() {
                    String::new()
                } else {
                    format!(" · lists {}", pages.join(", "))
                }
            )
        })
        .collect();
    if lines.is_empty() {
        return "No sitemaps submitted for this site.".to_owned();
    }
    lines.join("\n")
}

/// The day of a timestamp, or "never".
fn day(v: &Value) -> &str {
    v.as_str().and_then(|t| t.get(..10)).unwrap_or("never")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_one_escaped_piece_of_the_address() {
        assert_eq!(
            site_url(&json!({"site": "https://oido.app/"})).unwrap(),
            format!("{SITES}/https%3A%2F%2Foido.app%2F")
        );
        assert_eq!(
            site_url(&json!({"site": "sc-domain:oido.app"})).unwrap(),
            format!("{SITES}/sc-domain%3Aoido.app")
        );
        assert!(site_url(&json!({})).is_err());
    }

    #[test]
    fn a_query_takes_only_what_the_api_knows() {
        let ok = query_body(&json!({
            "start_date": "2025-01-01", "end_date": "2025-01-31",
            "dimensions": "query, page",
            "filters": [{"dimension": "page", "expression": "/blog/"}],
            "max_results": 5000
        }))
        .unwrap();
        assert_eq!(ok["dimensions"], json!(["query", "page"]));
        assert_eq!(ok["rowLimit"], 1000);
        assert_eq!(ok["type"], "web");
        assert_eq!(
            ok["dimensionFilterGroups"][0]["filters"][0],
            json!({"dimension": "page", "operator": "contains", "expression": "/blog/"})
        );
        for bad in [
            json!({"dimensions": "keyword"}),
            json!({"type": "shopping"}),
            json!({"filters": [{"dimension": "date", "expression": "x"}]}),
            json!({"filters": [{"dimension": "query", "operator": "like", "expression": "x"}]}),
            json!({"filters": [{"dimension": "query"}]}),
        ] {
            assert!(query_body(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rows_show_ctr_as_a_percentage_and_say_when_more_may_exist() {
        let asked = json!({"dimensions": ["query"], "rowLimit": 2, "startRow": 0});
        let got = json!({"rows": [
            {"keys": ["oido"], "clicks": 40, "impressions": 400, "ctr": 0.1, "position": 1.234},
            {"keys": ["voice\tapp"], "clicks": 1, "impressions": 90, "ctr": 0.0111, "position": 12.0}
        ]});
        assert_eq!(
            render_rows(&got, &asked),
            "query\tclicks\timpressions\tctr%\tposition\noido\t40\t400\t10\t1.23\nvoice app\t1\t90\t1.11\t12\n[There may be more: ask with start_row 2.]"
        );
        assert!(render_rows(&json!({}), &asked).starts_with("No data"));
    }

    #[test]
    fn an_inspection_says_whether_the_page_is_indexed() {
        let got = json!({"inspectionResult": {
            "inspectionResultLink": "https://search.google.com/x",
            "indexStatusResult": {"verdict": "PASS", "coverageState": "Submitted and indexed",
                "lastCrawlTime": "2025-03-01T10:00:00Z", "sitemap": ["https://oido.app/sitemap.xml"]},
            "richResultsResult": {"verdict": "PASS", "detectedItems": [{"richResultType": "FAQ"}]}
        }});
        let out = render_inspection(&got);
        assert!(out.starts_with("Verdict: PASS\nCoverage: Submitted and indexed"));
        assert!(out.contains("In sitemaps: https://oido.app/sitemap.xml"));
        assert!(out.contains("Rich results: PASS (FAQ)"));
        assert!(out.contains("Declared canonical: -"));
    }

    #[test]
    fn sitemaps_show_when_they_were_read_and_what_they_list() {
        let got = json!({"sitemap": [{"path": "https://oido.app/sitemap.xml",
            "lastSubmitted": "2025-03-01T10:00:00Z", "isPending": true, "errors": "0", "warnings": "2",
            "contents": [{"type": "web", "submitted": "120"}]}]});
        assert_eq!(
            render_sitemaps(&got),
            "https://oido.app/sitemap.xml · submitted 2025-03-01 · read never · 0 errors, 2 warnings · waiting to be read · lists 120 web"
        );
    }

    #[test]
    fn only_submitting_a_sitemap_changes_things() {
        let tools = SearchConsole.tools();
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
        assert_eq!(changing, ["submit_sitemap"]);
    }
}
