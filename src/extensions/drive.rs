//! Google Drive: find and read files, and make or trash files this app made.
//!
//! The sign-in reads everything in Drive but may only change what this app created
//! (`drive.file`), so a tool that trashes a file only works on one Oido made.

use serde_json::{Value, json};

use super::google::{Api, Failure, segment};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

const READ: &str = "https://www.googleapis.com/auth/drive.readonly";
const OWN: &str = "https://www.googleapis.com/auth/drive.file";
const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const DEFAULT_RESULTS: u64 = 20;
const MAX_RESULTS: u64 = 50;
/// What a listing asks Drive for about each file.
const LISTED: &str =
    "nextPageToken,files(id,name,mimeType,modifiedTime,size,owners(emailAddress),webViewLink)";
const DETAILED: &str = "id,name,mimeType,modifiedTime,createdTime,size,description,owners(emailAddress),parents,webViewLink,trashed";
const DOC: &str = "application/vnd.google-apps.document";
const SHEET: &str = "application/vnd.google-apps.spreadsheet";
const SLIDES: &str = "application/vnd.google-apps.presentation";
const FOLDER: &str = "application/vnd.google-apps.folder";

pub struct Drive;

impl Extension for Drive {
    fn title(&self) -> &'static str {
        "Google Drive"
    }

    fn logo(&self) -> Logo {
        super::marks::DRIVE
    }

    fn blurb(&self) -> &'static str {
        "Find and read the person's files, and make text files and Google Docs."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[READ, OWN]
    }

    fn tools(&self) -> Vec<ToolDef> {
        let id = json!({"type": "string", "description": "The file's id from search_files."});
        vec![
            ToolDef {
                name: "search_files",
                description: "Search the person's Drive with Drive's query syntax, like: name contains 'budget', fullText contains 'invoice', mimeType = 'application/pdf', 'FOLDER_ID' in parents, modifiedTime > '2025-01-01T00:00:00'. Combine with and / or. Trashed files are left out. Gives one line per file: id, name, type, when changed.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "A Drive query. Empty lists the most recently changed files."},
                        "max_results": {"type": "integer", "description": "How many files, 1 to 50. Default 20."},
                        "page_token": {"type": "string", "description": "From the end of an earlier search, for the next page."}
                    }
                }),
            },
            ToolDef {
                name: "get_file",
                description: "Details of one file or folder: its type, size, owner, when it changed, the folders it is in and its link.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {"id": id}, "required": ["id"]}),
            },
            ToolDef {
                name: "read_file",
                description: "Read a file as text: a Google Doc or Slides deck as plain text, a Google Sheet as CSV (its first sheet), or a text, Markdown, CSV, JSON or code file as it is. Other files (PDFs, images, Office files) cannot be read: get_file gives their link.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {"id": id}, "required": ["id"]}),
            },
            ToolDef {
                name: "create_file",
                description: "Make a new file from text: a plain text file, or a Google Doc when as_google_doc is true. It goes in the folder given, or the top of the person's Drive.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "The file's name."},
                        "content": {"type": "string", "description": "What is in it."},
                        "as_google_doc": {"type": "boolean", "description": "Make a Google Doc instead of a text file. Default false."},
                        "folder_id": {"type": "string", "description": "The folder to put it in."}
                    },
                    "required": ["name", "content"]
                }),
            },
            ToolDef {
                name: "trash_file",
                description: "Move a file to the trash, where the person can still restore it. Only works on files Oido made with create_file.",
                access: Access::Changes,
                schema: json!({"type": "object", "properties": {"id": id}, "required": ["id"]}),
            },
        ]
    }

    fn call(&self, tool: &str, args: &Value, api: &Api) -> Result<String, Failure> {
        match tool {
            "search_files" => search(args, api),
            "get_file" => {
                let file = api.get(
                    &file_url(args)?,
                    &[("fields", DETAILED), ("supportsAllDrives", "true")],
                )?;
                Ok(render_file(&file))
            }
            "read_file" => read(args, api),
            "create_file" => create(args, api),
            "trash_file" => {
                let file = api.patch(
                    &file_url(args)?,
                    &[("fields", "id,name,trashed"), ("supportsAllDrives", "true")],
                    &json!({"trashed": true}),
                )?;
                Ok(format!(
                    "Moved to the trash: {} ({}).",
                    file["name"].as_str().unwrap_or("?"),
                    file["id"].as_str().unwrap_or("?")
                ))
            }
            other => Err(Failure::other(format!(
                "Google Drive has no tool called {other}"
            ))),
        }
    }
}

fn file_url(args: &Value) -> Result<String, Failure> {
    let id = args["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Failure::other("`id` is required"))?;
    Ok(format!("{FILES}/{}", segment(id)))
}

/// The query as asked, with trashed files left out unless the query says something about them.
fn with_trash_hidden(query: &str) -> String {
    let query = query.trim();
    if query.contains("trashed") {
        query.to_owned()
    } else if query.is_empty() {
        "trashed = false".to_owned()
    } else {
        format!("({query}) and trashed = false")
    }
}

fn search(args: &Value, api: &Api) -> Result<String, Failure> {
    let max = args["max_results"]
        .as_u64()
        .unwrap_or(DEFAULT_RESULTS)
        .clamp(1, MAX_RESULTS)
        .to_string();
    let q = with_trash_hidden(args["query"].as_str().unwrap_or_default());
    let mut query = vec![
        ("q", q.as_str()),
        ("pageSize", max.as_str()),
        ("orderBy", "modifiedTime desc"),
        ("fields", LISTED),
        ("supportsAllDrives", "true"),
        ("includeItemsFromAllDrives", "true"),
    ];
    if let Some(t) = args["page_token"].as_str().filter(|t| !t.is_empty()) {
        query.push(("pageToken", t));
    }
    let found = api.get(FILES, &query)?;
    let files = found["files"].as_array().map_or(&[][..], Vec::as_slice);
    if files.is_empty() {
        return Ok("No files found.".to_owned());
    }
    let mut lines: Vec<String> = files.iter().map(summary_line).collect();
    if let Some(next) = found["nextPageToken"].as_str() {
        lines.push(format!("More files: search again with page_token={next}"));
    }
    Ok(lines.join("\n"))
}

/// How a Drive file is turned into text: asked to be exported as this type, downloaded as it is,
/// or not readable.
#[derive(Debug, PartialEq)]
enum Way {
    Export(&'static str),
    Download,
    Unreadable,
}

fn way_to_read(mime: &str) -> Way {
    match mime {
        DOC | SLIDES => Way::Export("text/plain"),
        SHEET => Way::Export("text/csv"),
        FOLDER => Way::Unreadable,
        m if m.starts_with("text/")
            || matches!(
                m,
                "application/json"
                    | "application/xml"
                    | "application/x-yaml"
                    | "application/javascript"
            ) =>
        {
            Way::Download
        }
        _ => Way::Unreadable,
    }
}

fn read(args: &Value, api: &Api) -> Result<String, Failure> {
    let url = file_url(args)?;
    let meta = api.get(
        &url,
        &[
            ("fields", "id,name,mimeType,size"),
            ("supportsAllDrives", "true"),
        ],
    )?;
    let (name, mime) = (
        meta["name"].as_str().unwrap_or("?"),
        meta["mimeType"].as_str().unwrap_or(""),
    );
    let text = match way_to_read(mime) {
        Way::Export(to) => api.get_text(&format!("{url}/export"), &[("mimeType", to)])?,
        Way::Download => api.get_text(&url, &[("alt", "media"), ("supportsAllDrives", "true")])?,
        Way::Unreadable => {
            return Err(Failure::other(format!(
                "{name} is a {mime} file, which cannot be read as text. get_file gives its link"
            )));
        }
    };
    Ok(format!("File: {name} ({mime})\n\n{text}"))
}

fn create(args: &Value, api: &Api) -> Result<String, Failure> {
    let (name, content) = match (args["name"].as_str(), args["content"].as_str()) {
        (Some(n), Some(c)) if !n.trim().is_empty() => (n, c),
        _ => return Err(Failure::other("`name` and `content` are required")),
    };
    let mut meta = json!({"name": name});
    if args["as_google_doc"].as_bool() == Some(true) {
        meta["mimeType"] = json!(DOC);
    }
    if let Some(folder) = args["folder_id"].as_str().filter(|f| !f.is_empty()) {
        meta["parents"] = json!([folder]);
    }
    let boundary = format!("oido{}", uuid::Uuid::new_v4().simple());
    let made = api.post_raw(
        UPLOAD,
        &[
            ("uploadType", "multipart"),
            ("fields", DETAILED),
            ("supportsAllDrives", "true"),
        ],
        &format!("multipart/related; boundary={boundary}"),
        multipart(&boundary, &meta, content),
    )?;
    Ok(format!("Created.\n{}", render_file(&made)))
}

/// A multipart upload body: the file's details as JSON, then its text.
fn multipart(boundary: &str, meta: &Value, content: &str) -> String {
    format!(
        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{content}\r\n--{boundary}--"
    )
}

fn kind_of(mime: &str) -> &str {
    match mime {
        DOC => "Google Doc",
        SHEET => "Google Sheet",
        SLIDES => "Google Slides",
        FOLDER => "folder",
        other => other,
    }
}

fn summary_line(f: &Value) -> String {
    format!(
        "{} · {} · {} · {}",
        f["id"].as_str().unwrap_or("?"),
        f["name"].as_str().unwrap_or("?"),
        kind_of(f["mimeType"].as_str().unwrap_or("?")),
        f["modifiedTime"].as_str().unwrap_or("?")
    )
}

fn render_file(f: &Value) -> String {
    let mut out = format!(
        "File {}\nName: {}\nType: {}\n",
        f["id"].as_str().unwrap_or("?"),
        f["name"].as_str().unwrap_or("?"),
        kind_of(f["mimeType"].as_str().unwrap_or("?"))
    );
    let mut field = |name: &str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            out.push_str(&format!("{name}: {v}\n"));
        }
    };
    field("Size", f["size"].as_str().map(|s| format!("{s} bytes")));
    field("Created", f["createdTime"].as_str().map(str::to_owned));
    field("Changed", f["modifiedTime"].as_str().map(str::to_owned));
    field(
        "Owner",
        f["owners"][0]["emailAddress"].as_str().map(str::to_owned),
    );
    field("In folder", f["parents"][0].as_str().map(str::to_owned));
    field(
        "Trashed",
        (f["trashed"].as_bool() == Some(true)).then(|| "yes".to_owned()),
    );
    field("Description", f["description"].as_str().map(str::to_owned));
    field("Link", f["webViewLink"].as_str().map(str::to_owned));
    out.trim_end().to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn trashed_files_stay_out_unless_the_query_speaks_of_them() {
        assert_eq!(with_trash_hidden(""), "trashed = false");
        assert_eq!(
            with_trash_hidden(" name contains 'a' or name contains 'b' "),
            "(name contains 'a' or name contains 'b') and trashed = false"
        );
        assert_eq!(with_trash_hidden("trashed = true"), "trashed = true");
    }

    #[test]
    fn each_kind_of_file_is_exported_downloaded_or_refused() {
        assert_eq!(way_to_read(DOC), Way::Export("text/plain"));
        assert_eq!(way_to_read(SLIDES), Way::Export("text/plain"));
        assert_eq!(way_to_read(SHEET), Way::Export("text/csv"));
        assert_eq!(way_to_read("text/markdown"), Way::Download);
        assert_eq!(way_to_read("application/json"), Way::Download);
        for no in [FOLDER, "application/pdf", "image/png", ""] {
            assert_eq!(way_to_read(no), Way::Unreadable, "{no}");
        }
    }

    #[test]
    fn an_upload_is_the_details_then_the_text_between_boundaries() {
        let body = multipart("B", &json!({"name": "n"}), "hello");
        assert_eq!(
            body,
            "--B\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{\"name\":\"n\"}\r\n--B\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nhello\r\n--B--"
        );
    }

    #[test]
    fn files_read_on_a_line_and_in_full() {
        let f = json!({
            "id": "f1", "name": "Plan", "mimeType": DOC, "modifiedTime": "2025-03-01T10:00:00Z",
            "size": "120", "owners": [{"emailAddress": "me@x.org"}], "parents": ["root1"],
            "webViewLink": "https://docs/f1", "description": "Q1"
        });
        assert_eq!(
            summary_line(&f),
            "f1 · Plan · Google Doc · 2025-03-01T10:00:00Z"
        );
        let full = render_file(&f);
        for want in [
            "Type: Google Doc",
            "Size: 120 bytes",
            "Owner: me@x.org",
            "In folder: root1",
            "Link: https://docs/f1",
            "Description: Q1",
        ] {
            assert!(full.contains(want), "{want}");
        }
        assert!(!full.contains("Trashed"));
    }

    #[test]
    fn ids_go_into_addresses_escaped_and_are_required() {
        assert_eq!(
            file_url(&json!({"id": "a/b"})).unwrap(),
            format!("{FILES}/a%2Fb")
        );
        assert!(file_url(&json!({})).is_err());
    }

    #[test]
    fn tools_that_change_things_say_so_and_every_tool_is_described() {
        let tools = Drive.tools();
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
        assert_eq!(changing, ["create_file", "trash_file"]);
    }
}
