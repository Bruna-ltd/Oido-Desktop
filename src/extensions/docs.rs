//! Google Docs: read a document as text, make one, add to it and replace words in it.
//! Finding a document is Google Drive's `search_files`.

use serde_json::{Value, json};

use super::google::{Api, Failure, segment};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

const DOCUMENTS: &str = "https://www.googleapis.com/auth/documents";
const BASE: &str = "https://docs.googleapis.com/v1/documents";

pub struct Docs;

impl Extension for Docs {
    fn title(&self) -> &'static str {
        "Google Docs"
    }

    fn logo(&self) -> Logo {
        super::marks::DOCS
    }

    fn blurb(&self) -> &'static str {
        "Read the person's documents, make new ones, add text and replace words."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[DOCUMENTS]
    }

    fn tools(&self) -> Vec<ToolDef> {
        let id = json!({"type": "string", "description": "The document's id: the long part of its address between /d/ and /edit, or the id search_files gives."});
        vec![
            ToolDef {
                name: "read_doc",
                description: "Read a Google Doc as text: its title, then its paragraphs, with headings marked by # and list items by -, and tables as tab-separated rows.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {"id": id}, "required": ["id"]}),
            },
            ToolDef {
                name: "create_doc",
                description: "Make a new Google Doc, with some text in it when given. It goes at the top of the person's Drive; move_file in Google Drive puts it in a folder.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "content": {"type": "string", "description": "The text to start it with."}
                    },
                    "required": ["title"]
                }),
            },
            ToolDef {
                name: "append_text",
                description: "Add text at the end of a document, on a new line unless new_line is false.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "id": id,
                        "text": {"type": "string"},
                        "new_line": {"type": "boolean", "description": "Start on a new line. Default true."}
                    },
                    "required": ["id", "text"]
                }),
            },
            ToolDef {
                name: "replace_text",
                description: "Replace every place a piece of text appears in a document with other text, and say how many were changed.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "id": id,
                        "find": {"type": "string"},
                        "replace": {"type": "string", "description": "What to put there. Empty removes it."},
                        "match_case": {"type": "boolean", "description": "Default false."}
                    },
                    "required": ["id", "find", "replace"]
                }),
            },
        ]
    }

    fn call(&self, tool: &str, args: &Value, api: &Api) -> Result<String, Failure> {
        match tool {
            "read_doc" => {
                let doc = api.get(&doc_url(args)?, &[])?;
                Ok(render_doc(&doc))
            }
            "create_doc" => {
                let title = args["title"]
                    .as_str()
                    .filter(|t| !t.trim().is_empty())
                    .ok_or_else(|| Failure::other("`title` is required"))?;
                let made = api.post(BASE, &[], &json!({"title": title}))?;
                let id = made["documentId"].as_str().unwrap_or_default();
                if let Some(text) = args["content"].as_str().filter(|t| !t.is_empty()) {
                    batch(
                        api,
                        id,
                        &json!([{"insertText": {"location": {"index": 1}, "text": text}}]),
                    )?;
                }
                Ok(format!(
                    "Document made. Id {id}. Link: https://docs.google.com/document/d/{id}/edit"
                ))
            }
            "append_text" => {
                let url = doc_url(args)?;
                let text = args["text"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| Failure::other("`text` is required"))?;
                // A document is never empty: it ends with one newline. Past that, a new line needs
                // its own newline in front.
                let size = api.get(&url, &[("fields", "body.content.endIndex")])?;
                let end = last_end(&size);
                let lead = if args["new_line"].as_bool() != Some(false) && end > 2 {
                    "\n"
                } else {
                    ""
                };
                batch(
                    api,
                    id_of(args)?,
                    &json!([{"insertText": {"endOfSegmentLocation": {}, "text": format!("{lead}{text}")}}]),
                )?;
                Ok("Added at the end.".to_owned())
            }
            "replace_text" => {
                let find = args["find"]
                    .as_str()
                    .filter(|f| !f.is_empty())
                    .ok_or_else(|| Failure::other("`find` is required"))?;
                let replace = args["replace"]
                    .as_str()
                    .ok_or_else(|| Failure::other("`replace` is required"))?;
                let done = batch(
                    api,
                    id_of(args)?,
                    &json!([{"replaceAllText": {
                        "containsText": {"text": find, "matchCase": args["match_case"].as_bool() == Some(true)},
                        "replaceText": replace
                    }}]),
                )?;
                let n = done["replies"][0]["replaceAllText"]["occurrencesChanged"]
                    .as_u64()
                    .unwrap_or(0);
                Ok(format!(
                    "Replaced {n} place{}.",
                    if n == 1 { "" } else { "s" }
                ))
            }
            other => Err(Failure::other(format!(
                "Google Docs has no tool called {other}"
            ))),
        }
    }
}

fn id_of(args: &Value) -> Result<&str, Failure> {
    args["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Failure::other("`id` is required"))
}

fn doc_url(args: &Value) -> Result<String, Failure> {
    Ok(format!("{BASE}/{}", segment(id_of(args)?)))
}

fn batch(api: &Api, id: &str, requests: &Value) -> Result<Value, Failure> {
    api.post(
        &format!("{BASE}/{}:batchUpdate", segment(id)),
        &[],
        &json!({"requests": requests}),
    )
}

/// Where the body ends.
fn last_end(doc: &Value) -> u64 {
    doc["body"]["content"]
        .as_array()
        .and_then(|c| c.last())
        .and_then(|e| e["endIndex"].as_u64())
        .unwrap_or(0)
}

fn render_doc(doc: &Value) -> String {
    let mut out = format!(
        "Document: {}\n\n",
        doc["title"].as_str().unwrap_or("(untitled)")
    );
    out.push_str(&elements(&doc["body"]["content"]));
    out.trim_end().to_owned()
}

/// The text of a run of structural elements: paragraphs, tables, and what they hold.
fn elements(content: &Value) -> String {
    let mut out = String::new();
    for e in content.as_array().into_iter().flatten() {
        if let Some(p) = e.get("paragraph") {
            out.push_str(&paragraph(p));
        } else if let Some(t) = e.get("table") {
            for row in t["tableRows"].as_array().into_iter().flatten() {
                let cells: Vec<String> = row["tableCells"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| elements(&c["content"]).trim().replace('\n', " "))
                    .collect();
                out.push_str(&cells.join("\t"));
                out.push('\n');
            }
        }
    }
    out
}

fn paragraph(p: &Value) -> String {
    let text: String = p["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["textRun"]["content"].as_str())
        .collect();
    let text = text.trim_end_matches('\n');
    if text.is_empty() {
        return "\n".to_owned();
    }
    let level = match p["paragraphStyle"]["namedStyleType"].as_str() {
        Some("TITLE") => 1,
        Some(h) if h.starts_with("HEADING_") => h[8..].parse().unwrap_or(1),
        _ => 0,
    };
    let prefix = if level > 0 {
        format!("{} ", "#".repeat(level))
    } else if p.get("bullet").is_some() {
        "- ".to_owned()
    } else {
        String::new()
    };
    format!("{prefix}{text}\n")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn run(text: &str) -> Value {
        json!({"textRun": {"content": text}})
    }

    #[test]
    fn a_document_reads_with_headings_lists_and_tables() {
        let doc = json!({
            "title": "Plan",
            "body": {"content": [
                {"sectionBreak": {}},
                {"paragraph": {"paragraphStyle": {"namedStyleType": "HEADING_2"}, "elements": [run("Goals\n")]}},
                {"paragraph": {"elements": [run("Ship "), run("it.\n")]}},
                {"paragraph": {"bullet": {}, "elements": [run("One\n")]}},
                {"paragraph": {"elements": [run("\n")]}},
                {"table": {"tableRows": [
                    {"tableCells": [{"content": [{"paragraph": {"elements": [run("a\n")]}}]}, {"content": [{"paragraph": {"elements": [run("b\n")]}}]}]}
                ]}}
            ]}
        });
        assert_eq!(
            render_doc(&doc),
            "Document: Plan\n\n## Goals\nShip it.\n- One\n\na\tb"
        );
    }

    #[test]
    fn the_end_of_the_body_is_the_last_elements_end() {
        assert_eq!(
            last_end(&json!({"body": {"content": [{"endIndex": 1}, {"endIndex": 42}]}})),
            42
        );
        assert_eq!(last_end(&json!({})), 0);
    }

    #[test]
    fn an_id_is_required_and_goes_into_the_address_escaped() {
        assert_eq!(
            doc_url(&json!({"id": "a/b"})).unwrap(),
            format!("{BASE}/a%2Fb")
        );
        assert!(doc_url(&json!({})).is_err());
    }

    #[test]
    fn the_tools_that_change_things_are_marked() {
        let tools = Docs.tools();
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
        assert_eq!(changing, ["create_doc", "append_text", "replace_text"]);
    }
}
