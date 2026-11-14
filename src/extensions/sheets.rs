//! Google Sheets: read cells, write them, add rows, and make spreadsheets and tabs.
//! Finding a spreadsheet is Google Drive's `search_files`.

use serde_json::{Value, json};

use super::google::{Api, Failure, segment};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

const SPREADSHEETS: &str = "https://www.googleapis.com/auth/spreadsheets";
const BASE: &str = "https://sheets.googleapis.com/v4/spreadsheets";
/// The most cells one write may carry.
const MAX_CELLS: usize = 5000;

pub struct Sheets;

impl Extension for Sheets {
    fn title(&self) -> &'static str {
        "Google Sheets"
    }

    fn logo(&self) -> Logo {
        super::marks::SHEETS
    }

    fn blurb(&self) -> &'static str {
        "Read and write the person's spreadsheets, add rows, and make new ones."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[SPREADSHEETS]
    }

    fn tools(&self) -> Vec<ToolDef> {
        let id = json!({"type": "string", "description": "The spreadsheet's id: the long part of its address between /d/ and /edit, or the id search_files gives."});
        let range = |what: &str| json!({"type": "string", "description": format!("{what} in A1 notation, like Sheet1!A1:D20, or just a tab's name like Sheet1.")});
        let values = json!({
            "type": "array",
            "items": {"type": "array", "items": {}},
            "description": "Rows of cells: [[\"Name\", \"Age\"], [\"Ann\", 31]]. A cell is text, a number, true or false. Text starting with = is a formula."
        });
        vec![
            ToolDef {
                name: "get_spreadsheet",
                description: "A spreadsheet's title and its tabs, with each tab's size.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {"id": id}, "required": ["id"]}),
            },
            ToolDef {
                name: "read_range",
                description: "Read cells as the person sees them, one row per line with the cells separated by tabs.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {"id": id, "range": range("The cells")},
                    "required": ["id", "range"]
                }),
            },
            ToolDef {
                name: "update_range",
                description: "Write cells starting at the top left of a range, replacing what is there. Typed as the person would type them: 1/2 may become a date, =SUM(A1:A3) is a formula.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"id": id, "range": range("Where to start"), "values": values},
                    "required": ["id", "range", "values"]
                }),
            },
            ToolDef {
                name: "append_rows",
                description: "Add rows after the last row that has data in a tab or range, without touching what is above.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"id": id, "range": range("The tab or table to add to"), "values": values},
                    "required": ["id", "range", "values"]
                }),
            },
            ToolDef {
                name: "clear_range",
                description: "Empty cells (their formatting stays). The values are gone, though the person can undo from the sheet's version history.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"id": id, "range": range("The cells")},
                    "required": ["id", "range"]
                }),
            },
            ToolDef {
                name: "create_spreadsheet",
                description: "Make a new spreadsheet, with tabs of the names given (default one tab). It goes at the top of the person's Drive.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "tabs": {"type": "array", "items": {"type": "string"}, "description": "Tab names."}
                    },
                    "required": ["title"]
                }),
            },
            ToolDef {
                name: "add_tab",
                description: "Add a tab to a spreadsheet.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"id": id, "title": {"type": "string"}},
                    "required": ["id", "title"]
                }),
            },
        ]
    }

    fn call(&self, tool: &str, args: &Value, api: &Api) -> Result<String, Failure> {
        match tool {
            "get_spreadsheet" => {
                let sheet = api.get(
                    &sheet_url(args)?,
                    &[("fields", "properties.title,sheets.properties")],
                )?;
                Ok(render_spreadsheet(&sheet))
            }
            "read_range" => {
                let url = format!("{}/values/{}", sheet_url(args)?, segment(range_of(args)?));
                let got = api.get(&url, &[("valueRenderOption", "FORMATTED_VALUE")])?;
                Ok(render_values(&got))
            }
            "update_range" => {
                let range = range_of(args)?;
                let rows = rows_of(args)?;
                let url = format!("{}/values/{}", sheet_url(args)?, segment(range));
                let done = api.put(
                    &url,
                    &[("valueInputOption", "USER_ENTERED")],
                    &json!({"range": range, "majorDimension": "ROWS", "values": rows}),
                )?;
                Ok(format!(
                    "Wrote {} cells to {}.",
                    done["updatedCells"].as_u64().unwrap_or(0),
                    done["updatedRange"].as_str().unwrap_or(range)
                ))
            }
            "append_rows" => {
                let range = range_of(args)?;
                let rows = rows_of(args)?;
                let url = format!("{}/values/{}:append", sheet_url(args)?, segment(range));
                let done = api.post(
                    &url,
                    &[
                        ("valueInputOption", "USER_ENTERED"),
                        ("insertDataOption", "INSERT_ROWS"),
                    ],
                    &json!({"majorDimension": "ROWS", "values": rows}),
                )?;
                Ok(format!(
                    "Added {} rows at {}.",
                    done["updates"]["updatedRows"].as_u64().unwrap_or(0),
                    done["updates"]["updatedRange"].as_str().unwrap_or(range)
                ))
            }
            "clear_range" => {
                let url = format!(
                    "{}/values/{}:clear",
                    sheet_url(args)?,
                    segment(range_of(args)?)
                );
                let done = api.post(&url, &[], &json!({}))?;
                Ok(format!(
                    "Cleared {}.",
                    done["clearedRange"].as_str().unwrap_or("the range")
                ))
            }
            "create_spreadsheet" => {
                let title = args["title"]
                    .as_str()
                    .filter(|t| !t.trim().is_empty())
                    .ok_or_else(|| Failure::other("`title` is required"))?;
                let tabs: Vec<&str> = args["tabs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .collect();
                let mut body = json!({"properties": {"title": title}});
                if !tabs.is_empty() {
                    body["sheets"] = tabs
                        .iter()
                        .map(|t| json!({"properties": {"title": t}}))
                        .collect();
                }
                let made = api.post(BASE, &[], &body)?;
                Ok(format!(
                    "Spreadsheet made. Id {}. Link: {}",
                    made["spreadsheetId"].as_str().unwrap_or("?"),
                    made["spreadsheetUrl"].as_str().unwrap_or("?")
                ))
            }
            "add_tab" => {
                let title = args["title"]
                    .as_str()
                    .filter(|t| !t.trim().is_empty())
                    .ok_or_else(|| Failure::other("`title` is required"))?;
                let url = format!("{}:batchUpdate", sheet_url(args)?);
                let done = api.post(
                    &url,
                    &[],
                    &json!({"requests": [{"addSheet": {"properties": {"title": title}}}]}),
                )?;
                Ok(format!(
                    "Tab added: {}.",
                    done["replies"][0]["addSheet"]["properties"]["title"]
                        .as_str()
                        .unwrap_or(title)
                ))
            }
            other => Err(Failure::other(format!(
                "Google Sheets has no tool called {other}"
            ))),
        }
    }
}

fn sheet_url(args: &Value) -> Result<String, Failure> {
    let id = args["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Failure::other("`id` is required"))?;
    Ok(format!("{BASE}/{}", segment(id)))
}

fn range_of(args: &Value) -> Result<&str, Failure> {
    args["range"]
        .as_str()
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(|| Failure::other("`range` is required"))
}

/// The rows to write: lists of cells that are text, numbers, true/false or empty, and not too many.
fn rows_of(args: &Value) -> Result<Value, Failure> {
    let rows = args["values"]
        .as_array()
        .filter(|r| !r.is_empty())
        .ok_or_else(|| Failure::other("`values` must be a list of rows, each a list of cells"))?;
    let mut cells = 0;
    for row in rows {
        let row = row.as_array().ok_or_else(|| {
            Failure::other("`values` must be a list of rows, each a list of cells")
        })?;
        cells += row.len();
        if row.iter().any(|c| c.is_array() || c.is_object()) {
            return Err(Failure::other(
                "a cell must be text, a number, true or false",
            ));
        }
    }
    if cells > MAX_CELLS {
        return Err(Failure::other(format!(
            "that is {cells} cells; write at most {MAX_CELLS} at a time"
        )));
    }
    Ok(Value::Array(rows.clone()))
}

fn render_spreadsheet(sheet: &Value) -> String {
    let mut out = format!(
        "Spreadsheet: {}\nTabs:",
        sheet["properties"]["title"].as_str().unwrap_or("?")
    );
    for t in sheet["sheets"].as_array().into_iter().flatten() {
        let p = &t["properties"];
        out.push_str(&format!(
            "\n- {} ({} rows × {} columns)",
            p["title"].as_str().unwrap_or("?"),
            p["gridProperties"]["rowCount"].as_u64().unwrap_or(0),
            p["gridProperties"]["columnCount"].as_u64().unwrap_or(0)
        ));
    }
    out
}

fn render_values(got: &Value) -> String {
    let rows = got["values"].as_array().map_or(&[][..], Vec::as_slice);
    if rows.is_empty() {
        return format!("{} is empty.", got["range"].as_str().unwrap_or("The range"));
    }
    let mut out = format!("{}\n", got["range"].as_str().unwrap_or(""));
    for row in rows {
        let cells: Vec<String> = row
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                c.as_str()
                    .map_or_else(|| c.to_string(), |s| s.replace(['\t', '\n'], " "))
            })
            .collect();
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn cells_read_one_row_per_line_with_tabs_and_blank_ends_trimmed_away_by_google() {
        let got =
            json!({"range": "Sheet1!A1:B2", "values": [["Name", "Age"], ["Ann", "31\n2"], ["Bo"]]});
        assert_eq!(
            render_values(&got),
            "Sheet1!A1:B2\nName\tAge\nAnn\t31 2\nBo"
        );
        assert_eq!(
            render_values(&json!({"range": "Sheet1!A1"})),
            "Sheet1!A1 is empty."
        );
    }

    #[test]
    fn what_is_written_is_a_list_of_rows_of_plain_cells_and_not_too_many() {
        assert!(rows_of(&json!({"values": [["a", 1, true, null], ["=SUM(A1:A2)"]]})).is_ok());
        for bad in [
            json!({}),
            json!({"values": []}),
            json!({"values": ["a"]}),
            json!({"values": [[["x"]]]}),
            json!({"values": [[{"a": 1}]]}),
        ] {
            assert!(rows_of(&bad).is_err(), "{bad}");
        }
        let big = json!({"values": vec![vec![1; 100]; 51]});
        assert!(
            rows_of(&big).unwrap_err()
                == Failure::other("that is 5100 cells; write at most 5000 at a time")
        );
    }

    #[test]
    fn tabs_are_listed_with_their_size() {
        let sheet = json!({"properties": {"title": "Budget"}, "sheets": [
            {"properties": {"title": "2025", "gridProperties": {"rowCount": 1000, "columnCount": 26}}}
        ]});
        assert_eq!(
            render_spreadsheet(&sheet),
            "Spreadsheet: Budget\nTabs:\n- 2025 (1000 rows × 26 columns)"
        );
    }

    #[test]
    fn ids_and_ranges_are_required_and_escaped_into_the_address() {
        assert_eq!(
            sheet_url(&json!({"id": "a/b"})).unwrap(),
            format!("{BASE}/a%2Fb")
        );
        assert!(sheet_url(&json!({})).is_err() && range_of(&json!({"range": " "})).is_err());
        assert_eq!(
            range_of(&json!({"range": "Sheet1!A1:B2"})).unwrap(),
            "Sheet1!A1:B2"
        );
    }

    #[test]
    fn the_tools_that_change_things_are_marked() {
        let tools = Sheets.tools();
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
                "update_range",
                "append_rows",
                "clear_range",
                "create_spreadsheet",
                "add_tab"
            ]
        );
    }
}
