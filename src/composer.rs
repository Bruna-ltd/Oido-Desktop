//! The message box's state and what it does: the draft and what is attached to it, the `/` command
//! and `@` file lists that open as you type, walking back through what you sent with the arrow
//! keys, and the message waiting to go while the agent is still answering. `ui::composer` draws it.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rune::{Key, KeyEvent, TextEdit};

use crate::app::{App, Effort};
use crate::model::{Message, Outcome};
use crate::picker::score;
use crate::prompt::{self, Attachment, Image, Prompt};
use crate::providers::ModelStatus;

/// The most rows a list shows.
const ROWS: usize = 50;
/// The most files `@` looks through in a working folder; past it, deeper files are not offered.
const MAX_FILES: usize = 10_000;
/// How long a folder's file list is trusted before `@` looks again.
const FRESH: Duration = Duration::from_secs(30);
/// Folders `@` never looks in: build output and dependencies, which are big and rarely meant.
const SKIPPED: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    "vendor",
    "__pycache__",
    "venv",
];

/// The message being written in one chat.
#[derive(Debug)]
pub struct Composer {
    pub edit: TextEdit,
    pub attachments: Vec<Attachment>,
    /// The highlighted row of the open list, and the text it was highlighted for: typing moves
    /// the highlight back to the top.
    active: (String, usize),
    /// The text when Esc closed the list. It stays closed until the text changes.
    dismissed: Option<String>,
    /// Walking back through your messages, when you are.
    recall: Option<Recall>,
    /// Why the last file could not be attached, or the message could not be sent.
    pub notice: Option<String>,
}

/// Which of your messages the box shows (counted back from the newest) and the draft it replaced.
#[derive(Debug)]
struct Recall {
    back: usize,
    draft: TextEdit,
}

impl Default for Composer {
    fn default() -> Self {
        Self::new()
    }
}

impl Composer {
    pub fn new() -> Composer {
        Composer {
            edit: TextEdit::new(""),
            attachments: Vec::new(),
            active: (String::new(), 0),
            dismissed: None,
            recall: None,
            notice: None,
        }
    }

    /// An empty box with the caret in it.
    pub fn focused() -> Composer {
        let mut c = Composer::new();
        c.edit.focused = true;
        c
    }

    /// Nothing typed, nothing attached.
    pub fn is_blank(&self) -> bool {
        self.edit.text.trim().is_empty() && self.attachments.is_empty()
    }

    /// What the box holds, set aside while another chat is open: the list and the walk through
    /// history end, and so does any notice.
    pub fn parked(mut self) -> Composer {
        if let Some(r) = self.recall.take()
            && self.edit.text.trim().is_empty()
        {
            self.edit = r.draft;
        }
        self.dismissed = None;
        self.notice = None;
        self.edit.focused = false;
        self
    }

    /// The `/` or `@` the caret is in, if a list should open for it.
    pub fn trigger(&self) -> Option<Trigger> {
        let text = &self.edit.text;
        if self.dismissed.as_ref() == Some(text) {
            return None;
        }
        let (a, caret) = self.edit.range();
        if a != caret {
            return None;
        }
        if let Some(query) = text.strip_prefix('/')
            && caret == text.len()
            && !text.contains('\n')
        {
            return Some(Trigger::Command(query.to_lowercase()));
        }
        let before = text.get(..caret)?;
        let start = before
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let query = before[start..].strip_prefix('@')?;
        // A pill's label starts with `@` too; the caret right after one is not a new mention.
        if self
            .edit
            .pills()
            .any(|(r, _)| r.start < caret && start < r.end)
        {
            return None;
        }
        Some(Trigger::File {
            range: start..caret,
            query: query.to_lowercase(),
        })
    }

    /// The highlighted row of a list of `rows`.
    pub fn active(&self, rows: usize) -> usize {
        if self.active.0 == self.edit.text {
            self.active.1.min(rows.saturating_sub(1))
        } else {
            0
        }
    }

    fn set_active(&mut self, row: usize) {
        self.active = (self.edit.text.clone(), row);
    }

    /// Closes the open list until the text changes.
    pub fn dismiss(&mut self) {
        self.dismissed = Some(self.edit.text.clone());
    }

    /// Puts `text` in the box with the caret at its end.
    fn show(&mut self, text: &str) {
        self.edit = TextEdit::new(text);
        self.edit.set_caret(text.len());
        self.edit.focused = true;
    }

    /// Shows your message before the one shown, or the newest when none is. `history` is your
    /// messages, oldest first. False when there is none further back.
    fn recall_older(&mut self, history: &[&str]) -> bool {
        let back = self.recall.as_ref().map_or(0, |r| r.back + 1);
        let Some(text) = history.len().checked_sub(back + 1).map(|i| history[i]) else {
            return false;
        };
        if self.recall.is_none() {
            let draft = std::mem::replace(&mut self.edit, TextEdit::new(""));
            self.recall = Some(Recall { back, draft });
        } else if let Some(r) = &mut self.recall {
            r.back = back;
        }
        self.show(text);
        true
    }

    /// Shows your message after the one shown, or the draft back when the newest was. False
    /// when you were not walking through history.
    fn recall_newer(&mut self, history: &[&str]) -> bool {
        let Some(r) = self.recall.take() else {
            return false;
        };
        match r.back.checked_sub(1) {
            Some(back) => {
                let text = history[history.len() - 1 - back];
                self.recall = Some(Recall {
                    back,
                    draft: r.draft,
                });
                self.show(text);
            }
            None => {
                self.edit = r.draft;
                self.edit.focused = true;
            }
        }
        true
    }

    /// Adds what was attached, or says why it could not be.
    fn attached(&mut self, results: Vec<Result<Attachment, String>>) {
        let mut why = Vec::new();
        for r in results {
            match r {
                Ok(a) => prompt::add(&mut self.attachments, a),
                Err(e) => why.push(e),
            }
        }
        self.notice = (!why.is_empty()).then(|| why.join(" "));
    }

    /// The draft as a message: its words, what was attached, and each file mentioned with `@`,
    /// read now from `root`, the agent's working folder.
    ///
    /// # Errors
    ///
    /// A mentioned file cannot be read (gone, outside the folder, too big), or a file is mentioned
    /// and the agent has no working folder; the message says which.
    fn prompt(&self, root: Option<&Path>) -> Result<Prompt, String> {
        let mut attachments = self.attachments.clone();
        for (_, path) in self.edit.pills() {
            let root = root.ok_or_else(|| {
                format!("@{path} cannot be read: this agent has no working folder.")
            })?;
            let full = crate::files::resolve(root, path)?;
            prompt::add(&mut attachments, Attachment::read(&full, path)?);
        }
        Ok(Prompt {
            text: self.edit.text.trim().to_owned(),
            attachments,
        })
    }
}

/// What the caret is in that opens a list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Trigger {
    /// A `/` at the start of the box, and what follows it (lowercase).
    Command(String),
    /// An `@` word: where it is, to be replaced by the file chosen, and what follows the `@`.
    File { range: Range<usize>, query: String },
}

/// Something the `/` list does.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Command {
    New,
    Clear,
    Retry,
    Teach,
    Model,
    Think(Effort),
    /// Asks the agent to run one of its workflows.
    Run(String),
}

impl Command {
    /// What you type for it.
    pub fn label(&self) -> String {
        match self {
            Command::New => "/new".to_owned(),
            Command::Clear => "/clear".to_owned(),
            Command::Retry => "/retry".to_owned(),
            Command::Teach => "/teach".to_owned(),
            Command::Model => "/model".to_owned(),
            Command::Think(e) => format!("/think {}", e.name()),
            Command::Run(name) => format!("/run {name}"),
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Command::New => "Start a new chat in a new tab",
            Command::Clear => "The agent forgets the chat; it stays on screen",
            Command::Retry => "Ask again for the reply that failed or was stopped",
            Command::Teach => "Learn a workflow from what this chat did",
            Command::Model => "Choose the agent's model",
            Command::Think(Effort::Off) => "Answer without thinking first",
            Command::Think(_) => "Think this hard before answering",
            Command::Run(_) => "Ask the agent to run this workflow",
        }
    }
}

/// A row of the open list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Suggestion {
    Command(Command),
    /// A file in the working folder, by its path inside it.
    File(String),
}

impl Suggestion {
    /// Its id in the list, unique among the rows.
    pub fn id(&self) -> String {
        match self {
            Suggestion::Command(c) => c.label(),
            Suggestion::File(path) => format!("@{path}"),
        }
    }
}

/// The open list: what it is for and its rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Suggestions {
    pub trigger: Trigger,
    pub rows: Vec<Suggestion>,
    /// Where `@`'s files stand; always `Ready` for commands.
    pub files: Listing,
}

/// Where the `@` list's files stand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Listing {
    Ready,
    /// Still looking through the working folder.
    Looking,
    /// The agent has no working folder, so there are no files to offer.
    NoFolder,
}

/// The files of one working folder, for `@`.
#[derive(Debug)]
pub struct FileIndex {
    root: PathBuf,
    /// Paths inside `root`, with `/` between parts; `None` while they are being found.
    files: Option<Vec<String>>,
    found: Instant,
}

/// Every file under `root`, shallow ones first, up to [`MAX_FILES`]. Hidden files and folders,
/// [`SKIPPED`] folders and links are left out.
pub fn walk(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut queue = std::collections::VecDeque::from([PathBuf::new()]);
    while let Some(rel) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(root.join(&rel)) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(kind) = e.file_type() else { continue };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let path = rel.join(&name);
            if kind.is_dir() {
                if !SKIPPED.contains(&name.as_str()) {
                    queue.push_back(path);
                }
            } else if kind.is_file() {
                let parts: Vec<_> = path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect();
                files.push(parts.join("/"));
                if files.len() >= MAX_FILES {
                    return files;
                }
            }
        }
    }
    files
}

/// The commands that apply to `app`'s open chat right now.
fn commands(app: &App) -> Vec<Command> {
    let Some(agent) = app.current() else {
        return Vec::new();
    };
    let mut out = vec![Command::New, Command::Clear];
    if matches!(
        agent.chat().messages.last(),
        Some(Message::Assistant {
            outcome: Outcome::Stopped | Outcome::Failed(_),
            ..
        })
    ) {
        out.push(Command::Retry);
    }
    if app.can_teach() {
        out.push(Command::Teach);
    }
    out.push(Command::Model);
    out.extend(
        [Effort::Off, Effort::Low, Effort::Medium, Effort::High]
            .into_iter()
            .filter(|e| *e != app.effort)
            .map(Command::Think),
    );
    out.extend(
        app.workflows
            .iter()
            .filter(|w| w.agent_id == agent.id)
            .map(|w| Command::Run(w.name.clone())),
    );
    out
}

/// The best `ROWS` of `items` for `query`, best first; all of them, in order, for no query.
fn ranked<T>(items: impl Iterator<Item = T>, query: &str, text: impl Fn(&T) -> &str) -> Vec<T> {
    if query.is_empty() {
        return items.take(ROWS).collect();
    }
    let mut hits: Vec<(i32, usize, T)> = items
        .filter_map(|i| {
            let t = text(&i);
            let hit = score(query, t).map(|s| (s, t.len()));
            hit.map(|(s, len)| (s, len, i))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().take(ROWS).map(|(_, _, i)| i).collect()
}

impl App {
    /// The list the composer shows, if one is open.
    pub fn suggestions(&self) -> Option<Suggestions> {
        let trigger = self.composer.trigger()?;
        match &trigger {
            Trigger::Command(query) => {
                let labelled = commands(self).into_iter().map(|c| (c.label(), c));
                // Matched without the slash, so "th" finds "/think".
                let rows: Vec<Suggestion> = ranked(labelled, query, |(l, _)| &l[1..])
                    .into_iter()
                    .map(|(_, c)| Suggestion::Command(c))
                    .collect();
                // A `/` that matches nothing is just text.
                (!rows.is_empty()).then_some(Suggestions {
                    trigger,
                    rows,
                    files: Listing::Ready,
                })
            }
            Trigger::File { query, .. } => {
                let Some(root) = self.composer_root() else {
                    return Some(Suggestions {
                        trigger,
                        rows: Vec::new(),
                        files: Listing::NoFolder,
                    });
                };
                let index = self.file_index.as_ref().filter(|i| i.root == root);
                let files = index.and_then(|i| i.files.as_deref());
                let rows = files
                    .map(|f| ranked(f.iter(), query, |p| p.as_str()))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| Suggestion::File(p.clone()))
                    .collect();
                Some(Suggestions {
                    trigger,
                    rows,
                    files: if files.is_some() {
                        Listing::Ready
                    } else {
                        Listing::Looking
                    },
                })
            }
        }
    }

    /// The folder chosen for the open chat's agent: `@` offers only its files. Unlike
    /// [`App::workdir_of`] there is no fallback; an agent with no folder chosen has no files.
    fn composer_root(&self) -> Option<PathBuf> {
        self.current()?.workdir.as_deref().map(PathBuf::from)
    }

    /// Your messages in the open chat, oldest first, for the arrow keys.
    fn history(&self) -> Vec<String> {
        self.current()
            .map(|a| {
                a.chat()
                    .messages
                    .iter()
                    .filter_map(|m| match m {
                        Message::User(p) if !p.text.trim().is_empty() => Some(p.text.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A key in the composer, before the field sees it. True when it was taken.
    pub fn composer_key(&mut self, key: &KeyEvent) -> bool {
        let m = key.mods;
        let plain = !(m.shift || m.cmd || m.ctrl || m.alt);
        let list = self.suggestions();
        if list.is_some() && key.key == Key::Escape {
            self.composer.dismiss();
            return true;
        }
        if let Some(list) = list.filter(|l| !l.rows.is_empty()) {
            let n = list.rows.len();
            let at = self.composer.active(n);
            match key.key {
                Key::Up if plain => {
                    self.composer.set_active((at + n - 1) % n);
                    return true;
                }
                Key::Down if plain => {
                    self.composer.set_active((at + 1) % n);
                    return true;
                }
                Key::Enter | Key::Tab if plain => {
                    self.choose_suggestion(&list.rows[at].id());
                    return true;
                }
                _ => {}
            }
        }
        let text = &self.composer.edit.text;
        let (_, caret) = self.composer.edit.range();
        match &key.key {
            Key::Text(t) if t == "@" => {
                self.index_files();
                false
            }
            // Up on the first line walks back through what you sent; Down on the last comes forward.
            Key::Up if plain && (text.trim().is_empty() || self.composer.recall.is_some()) => {
                if text.get(..caret).is_some_and(|b| b.contains('\n')) {
                    return false;
                }
                let history = self.history();
                let history: Vec<&str> = history.iter().map(String::as_str).collect();
                self.composer.recall_older(&history)
            }
            Key::Down if plain && self.composer.recall.is_some() => {
                if text.get(caret..).is_some_and(|a| a.contains('\n')) {
                    return false;
                }
                let history = self.history();
                let history: Vec<&str> = history.iter().map(String::as_str).collect();
                self.composer.recall_newer(&history)
            }
            Key::Escape if self.busy() => {
                self.stop();
                true
            }
            _ => false,
        }
    }

    /// A row of the open list was chosen, by its id.
    pub fn choose_suggestion(&mut self, id: &str) {
        let Some(list) = self.suggestions() else {
            return;
        };
        let Some(row) = list.rows.into_iter().find(|r| r.id() == id) else {
            return;
        };
        match (row, list.trigger) {
            (Suggestion::Command(c), _) => {
                self.composer = Composer::focused();
                self.run_command(c);
            }
            (Suggestion::File(path), Trigger::File { range, .. }) => {
                let edit = &mut self.composer.edit;
                edit.replace_with_pill(range, path.clone(), &format!("@{path}"));
                edit.insert(" ");
                edit.focused = true;
            }
            (Suggestion::File(_), Trigger::Command(_)) => {}
        }
    }

    /// Does what a `/` command says.
    fn run_command(&mut self, c: Command) {
        match c {
            Command::New => self.new_tab(),
            Command::Clear => self.start_fresh(),
            Command::Retry => self.retry(),
            Command::Teach => self.teach_open(),
            Command::Model => {
                let current = self.current().and_then(|a| a.model.clone());
                self.model_picker.show(current.as_ref());
                self.models_opened();
            }
            Command::Think(e) => self.effort = e,
            Command::Run(name) => {
                self.composer.edit = TextEdit::new(format!("Run the workflow \"{name}\"."));
                self.send();
            }
        }
    }

    /// The command the whole draft names, typed out in full (`/clear`), if it names one.
    fn typed_command(&self) -> Option<Command> {
        let typed = self.composer.edit.text.trim();
        if !typed.starts_with('/') || !self.composer.attachments.is_empty() {
            return None;
        }
        commands(self).into_iter().find(|c| c.label() == typed)
    }

    /// Starts finding the open chat's working-folder files for `@`, unless a fresh list is kept.
    pub fn index_files(&mut self) {
        let Some(root) = self.composer_root() else {
            return;
        };
        if self
            .file_index
            .as_ref()
            .is_some_and(|i| i.root == root && (i.files.is_none() || i.found.elapsed() < FRESH))
        {
            return;
        }
        self.file_index = Some(FileIndex {
            root: root.clone(),
            files: None,
            found: Instant::now(),
        });
        let looked = root.clone();
        self.spawn(
            move || walk(&looked),
            move |app, files| {
                if let Some(i) = app.file_index.as_mut().filter(|i| i.root == root) {
                    i.files = Some(files);
                    i.found = Instant::now();
                }
            },
        );
    }

    /// Attaches files to the draft: dropped on the chat, pasted, or chosen. Read off the UI
    /// thread, since a big picture takes a moment to shrink.
    pub fn attach_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let sid = self.current().map(|a| a.chat().id.clone());
        self.spawn(
            move || {
                paths
                    .iter()
                    .map(|p| {
                        let name = p.file_name().map_or_else(
                            || p.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        );
                        Attachment::read(p, &name)
                    })
                    .collect::<Vec<_>>()
            },
            move |app, results| app.attach_results(sid.as_deref(), results),
        );
    }

    /// Attaches a picture pasted from the clipboard.
    pub fn attach_png(&mut self, png: Vec<u8>) {
        let taken: HashSet<String> = self
            .composer
            .attachments
            .iter()
            .map(|a| a.name().to_owned())
            .collect();
        let name = (1..)
            .map(|n| format!("Pasted image {n}.png"))
            .find(|n| !taken.contains(n))
            .unwrap_or_default();
        let sid = self.current().map(|a| a.chat().id.clone());
        self.spawn(
            move || vec![Image::fit(&name, &png).map(Attachment::Image)],
            move |app, results| app.attach_results(sid.as_deref(), results),
        );
    }

    /// Files read for the draft of chat `sid`: they join it, whether it is open or parked.
    fn attach_results(&mut self, sid: Option<&str>, results: Vec<Result<Attachment, String>>) {
        let open = self.current().map(|a| a.chat().id.as_str()) == sid;
        match sid {
            Some(id) if !open => self
                .parked
                .entry(id.to_owned())
                .or_default()
                .attached(results),
            _ => self.composer.attached(results),
        }
    }

    pub fn detach(&mut self, name: &str) {
        self.composer.attachments.retain(|a| a.name() != name);
    }

    /// Whether the open agent's model can look at pictures. Unknown counts as yes: only a model
    /// known to be blind is refused them.
    pub fn sees_images(&self) -> bool {
        let Some(agent) = self.current() else {
            return false;
        };
        match self.providers.status(agent.model.as_ref()) {
            ModelStatus::Ready(m) => self
                .providers
                .model_info(&m)
                .is_none_or(|i| i.caps.vision != Some(false)),
            _ => true,
        }
    }

    /// Sends the draft: it joins the chat and the agent starts answering. A whole command
    /// (`/clear`) runs instead; while the agent is still answering, the message waits and goes
    /// when the reply ends.
    pub fn send(&mut self) {
        if !self.can_answer() || self.awaiting_answer() {
            return;
        }
        if let Some(c) = self.typed_command() {
            self.composer = Composer::focused();
            self.run_command(c);
            return;
        }
        let root = self.composer_root();
        let prompt = match self.composer.prompt(root.as_deref()) {
            Ok(p) if p.is_empty() => return,
            Ok(p) => p,
            Err(why) => {
                self.composer.notice = Some(why);
                return;
            }
        };
        if prompt.images().next().is_some() && !self.sees_images() {
            self.composer.notice = Some(
                "This model cannot look at pictures. Remove them, or choose a model that can."
                    .to_owned(),
            );
            return;
        }
        let Some(sid) = self.current().map(|a| a.chat().id.clone()) else {
            return;
        };
        self.composer = Composer::focused();
        if self.busy() {
            match self.queued.get_mut(&sid) {
                Some(waiting) => waiting.append(prompt),
                None => {
                    self.queued.insert(sid, prompt);
                }
            }
            return;
        }
        self.post(&sid, prompt);
    }

    /// Sends the message waiting in chat `sid`, if there is one and the chat is free for it.
    pub(crate) fn send_queued(&mut self, sid: &str) {
        if !self.queued.contains_key(sid) || self.busy_in(sid) || self.awaiting_in(sid) {
            return;
        }
        let ready = self.roster.owner_of(sid).is_some_and(|a| {
            matches!(
                self.providers.status(a.model.as_ref()),
                ModelStatus::Ready(_)
            )
        });
        if let (true, Some(prompt)) = (ready, self.queued.remove(sid)) {
            self.post(sid, prompt);
        }
    }

    /// Takes the waiting message of the open chat back into the box, to change or drop it.
    pub fn unqueue(&mut self) {
        let Some(sid) = self.current().map(|a| a.chat().id.clone()) else {
            return;
        };
        let Some(waiting) = self.queued.remove(&sid) else {
            return;
        };
        let typed = std::mem::take(&mut self.composer.edit.text);
        let mut back = Prompt::from(waiting.text);
        back.append(Prompt::from(typed));
        self.composer.show(&back.text);
        for a in waiting.attachments {
            prompt::add(&mut self.composer.attachments, a);
        }
    }

    /// The open chat's waiting message, if one waits.
    pub fn queued_here(&self) -> Option<&Prompt> {
        self.current().and_then(|a| self.queued.get(&a.chat().id))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Composer {
        let mut c = Composer::new();
        c.show(text);
        c
    }

    #[test]
    fn a_slash_at_the_start_opens_commands_and_an_at_word_opens_files() {
        assert_eq!(typed("/th").trigger(), Some(Trigger::Command("th".into())));
        assert_eq!(typed("hi /th").trigger(), None, "a slash mid-line is text");
        assert_eq!(
            typed("look at @Src/ma").trigger(),
            Some(Trigger::File {
                range: 8..15,
                query: "src/ma".into()
            })
        );
        assert_eq!(
            typed("mail me@").trigger(),
            None,
            "an @ inside a word is not a mention"
        );
        assert_eq!(
            typed("@").trigger(),
            Some(Trigger::File {
                range: 0..1,
                query: String::new()
            })
        );
    }

    #[test]
    fn a_chosen_file_is_a_pill_and_does_not_reopen_the_list() {
        let mut c = typed("see @ma");
        c.edit
            .replace_with_pill(4..7, "src/main.rs", "@src/main.rs");
        assert_eq!(c.trigger(), None);
        assert_eq!(
            c.edit.pills().map(|(_, id)| id).collect::<Vec<_>>(),
            ["src/main.rs"]
        );
    }

    #[test]
    fn escape_closes_the_list_until_the_text_changes() {
        let mut c = typed("/");
        c.dismiss();
        assert_eq!(c.trigger(), None);
        c.show("/c");
        assert!(c.trigger().is_some());
    }

    #[test]
    fn the_arrows_walk_back_through_what_you_sent_and_return_to_the_draft() {
        let history = ["first", "second"];
        let mut c = typed("");
        assert!(c.recall_older(&history));
        assert_eq!(c.edit.text, "second");
        assert!(c.recall_older(&history));
        assert_eq!(c.edit.text, "first");
        assert!(!c.recall_older(&history), "nothing older");
        assert!(c.recall_newer(&history));
        assert_eq!(c.edit.text, "second");
        assert!(c.recall_newer(&history));
        assert_eq!(c.edit.text, "", "back to the draft");
        assert!(!c.recall_newer(&history));
    }

    #[test]
    fn the_highlight_goes_back_to_the_top_when_the_text_changes() {
        let mut c = typed("/");
        c.set_active(2);
        assert_eq!(c.active(5), 2);
        assert_eq!(c.active(2), 1, "kept inside the list");
        c.show("/n");
        assert_eq!(c.active(5), 0);
    }

    #[test]
    fn ranking_puts_the_closest_match_first_and_keeps_order_with_no_query() {
        let files = ["src/ui/composer.rs", "src/composer.rs", "README.md"];
        assert_eq!(ranked(files.iter(), "", |f| f)[0], &"src/ui/composer.rs");
        let hits = ranked(files.iter(), "composer", |f| f);
        assert_eq!(hits, [&"src/composer.rs", &"src/ui/composer.rs"]);
    }

    #[test]
    fn at_offers_only_files_in_the_folder_chosen_for_the_agent_and_none_without_one() {
        let dir = std::env::temp_dir().join(format!("oido-at-{}", crate::model::new_id()));
        std::fs::create_dir_all(dir.join("mine")).unwrap();
        std::fs::write(dir.join("mine/a.txt"), "").unwrap();
        std::fs::write(dir.join("outside.txt"), "").unwrap();
        let mut app = crate::testing::samples();
        app.roster.current_mut().unwrap().workdir = None;
        app.composer.show("@");
        app.index_files();
        let list = app.suggestions().unwrap();
        assert_eq!(
            list.files,
            Listing::NoFolder,
            "no fallback to the home folder"
        );
        assert!(list.rows.is_empty());

        // Chosen from the box's folder pill.
        app.set_workdir(Some(dir.join("mine").to_string_lossy().into_owned()));
        app.index_files();
        let list = app.suggestions().unwrap();
        assert_eq!(list.files, Listing::Ready);
        assert_eq!(list.rows, [Suggestion::File("a.txt".into())]);

        app.set_workdir(None);
        assert_eq!(app.current().unwrap().workdir, None);
        assert_eq!(app.suggestions().unwrap().files, Listing::NoFolder);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_mention_is_read_only_from_inside_the_agents_folder() {
        let dir = std::env::temp_dir().join(format!("oido-mention-{}", crate::model::new_id()));
        std::fs::create_dir_all(dir.join("mine")).unwrap();
        std::fs::write(dir.join("mine/a.txt"), "inside").unwrap();
        std::fs::write(dir.join("secret.txt"), "outside").unwrap();
        let root = dir.join("mine");
        let mention = |path: &str| {
            let mut c = typed("see ");
            c.edit.insert_pill(path, &format!("@{path}"));
            c
        };
        assert!(
            mention("a.txt")
                .prompt(None)
                .unwrap_err()
                .contains("no working folder"),
            "no folder, no files"
        );
        let p = mention("a.txt").prompt(Some(&root)).unwrap();
        assert!(matches!(&p.attachments[..], [Attachment::File { text, .. }] if text == "inside"));
        assert!(mention("../secret.txt").prompt(Some(&root)).is_err());
        assert!(
            mention(&dir.join("secret.txt").to_string_lossy())
                .prompt(Some(&root))
                .is_err(),
            "nor by its full path"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn walking_a_folder_skips_hidden_and_build_folders_and_lists_shallow_files_first() {
        let dir = std::env::temp_dir().join(format!("oido-walk-{}", crate::model::new_id()));
        for d in ["src/deep", ".git", "target", "node_modules"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in [
            "src/deep/x.rs",
            "src/a.rs",
            "top.md",
            ".git/HEAD",
            "target/out",
            ".env",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        assert_eq!(walk(&dir), ["top.md", "src/a.rs", "src/deep/x.rs"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
