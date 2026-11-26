//! Skills from runs (see `docs/skills-from-runs.md`): the agent's model drafts a `SKILL.md` from a
//! finished run's steps, you edit it on the Runs screen, and saving writes it to the skills folder.
//! Only the form writes a skill; no model tool can.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use rune::TextEdit;
use serde_json::Value;

use crate::app::App;
use crate::completion::{self, Completion, Error, Request, Tokens, Turn};
use crate::guard::secrets;
use crate::runs::{Reply, Run, Status, Step, StepKind, Trace, clip};

/// Each step goes to the model cut to this many bytes.
const STEP_BYTES: usize = 1_200;
/// A run with more steps keeps this many from its start and as many from its end: a long run
/// resolves at its end, where the fix and the outcome are.
const HALF: usize = 40;
/// The Agent Skills limits on a skill's name and description.
const MAX_NAME: usize = 64;
const MAX_DESCRIPTION: usize = 1_024;
/// What the draft's run step is called.
pub const STEP_NAME: &str = "skill draft";

const PROMPT: &str = r#"You turn one finished agent run into a reusable skill: instructions a future agent reads before doing the same kind of task.

You are given the run's steps: what the person asked, what the agent said, each tool it called with its arguments and result, and errors. Find what is worth doing the same way next time: the approach that worked, the fix for a failure, a gotcha to avoid.

Answer with JSON only, no other text:
{"title": "...", "use_when": "...", "body": "..."}

- title: what the skill does, in one short sentence.
- use_when: the situation that should make an agent read this skill, starting with "when".
- body: Markdown instructions. Say what to do and what to avoid, and name the commands, files and settings that mattered. Teach the method; do not retell the run, and leave out step numbers, times and ids.

Leave any placeholder such as ⟨secret-1⟩ out of what you write. If the run holds nothing reusable (it failed with no lesson, or was small talk), answer {"none": "<why, in one sentence>"} instead."#;

/// What the model drafted.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Draft {
    pub title: String,
    pub use_when: String,
    pub body: String,
}

/// The model's answer: a draft, or why the run holds nothing to make one from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Parsed {
    Draft(Draft),
    Nothing(String),
}

/// The run's steps as the model is shown them: setup and guardrail notes left out, each step cut
/// to [`STEP_BYTES`], and only the first and last [`HALF`] of a longer run.
pub fn transcript(steps: &[Step]) -> String {
    let lines: Vec<String> = steps.iter().filter_map(line).collect();
    let mut out: Vec<String> = Vec::new();
    if lines.len() > 2 * HALF {
        out.extend_from_slice(&lines[..HALF]);
        out.push(format!("[{} steps left out here]", lines.len() - 2 * HALF));
        out.extend_from_slice(&lines[lines.len() - HALF..]);
    } else {
        out = lines;
    }
    out.join("\n\n")
}

fn line(s: &Step) -> Option<String> {
    let cut = |t: &Option<String>| {
        let (text, was) = clip(t.as_deref().unwrap_or_default().trim(), STEP_BYTES);
        if was { format!("{text}…") } else { text }
    };
    match s.kind {
        StepKind::User => Some(format!("[person] {}", cut(&s.output))),
        StepKind::Llm => Some(format!("[agent] {}", cut(&s.output))),
        StepKind::Tool => Some(format!(
            "[tool {}] {}\n→ {}",
            s.name,
            cut(&s.input),
            cut(&s.output)
        )),
        StepKind::Error => Some(format!("[error] {}", cut(&s.output))),
        _ => None,
    }
}

/// The one-off call that drafts a skill from `steps`, on `model`, with no tools.
pub fn request(model: &str, steps: &[Step]) -> Request {
    Request {
        model: model.to_owned(),
        system: Some(PROMPT.to_owned()),
        context: None,
        turns: vec![Turn::user(format!("<run>\n{}\n</run>", transcript(steps)))],
        think: None,
        tools: Vec::new(),
        cache_key: None,
    }
}

/// Reads the model's answer. JSON, bare or in a code fence, gives a draft or a reason there is
/// none; anything else is taken whole as the body.
pub fn parse(reply: &str) -> Parsed {
    let text = reply.trim();
    let json = text
        .find('{')
        .zip(text.rfind('}'))
        .filter(|(a, b)| a < b)
        .and_then(|(a, b)| serde_json::from_str::<Value>(&text[a..=b]).ok());
    let field = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    match json {
        Some(v) if v.get("none").is_some() => Parsed::Nothing(field(&v, "none")),
        Some(v) if v.get("body").is_some() => Parsed::Draft(Draft {
            title: field(&v, "title"),
            use_when: field(&v, "use_when"),
            body: field(&v, "body"),
        }),
        _ => Parsed::Draft(Draft {
            body: text.to_owned(),
            ..Draft::default()
        }),
    }
}

/// A skill's name and folder: the title's words in lowercase joined by `-`, at most
/// [`MAX_NAME`] bytes. Only `a-z`, `0-9` and `-`, so it can never leave the skills folder.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            if out.len() == MAX_NAME {
                break;
            }
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            if out.len() + 1 >= MAX_NAME {
                break;
            }
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// What the `skill` tool lists: the title's first sentence, then when to use it, cut to
/// [`MAX_DESCRIPTION`] characters.
pub fn description(title: &str, use_when: &str) -> String {
    let first = title.split(['.', '\n']).next().unwrap_or_default().trim();
    let mut out = format!("{first}.");
    let when = use_when.split_whitespace().collect::<Vec<_>>().join(" ");
    let when = when.trim_end_matches('.');
    if !when.is_empty() {
        let when = when
            .strip_prefix("Use ")
            .or_else(|| when.strip_prefix("use "))
            .unwrap_or(when);
        out.push_str(&format!(" Use {when}."));
    }
    if out.chars().count() > MAX_DESCRIPTION {
        out = out.chars().take(MAX_DESCRIPTION - 1).collect::<String>();
        out.truncate(out.trim_end().len());
        out.push('…');
    }
    out
}

/// Where a skill came from, kept under `metadata`.
pub struct Provenance<'a> {
    pub run: &'a str,
    pub agent: &'a str,
    /// `YYYY-MM-DD`.
    pub made: &'a str,
}

/// The `SKILL.md` for `draft`.
pub fn render(name: &str, draft: &Draft, from: &Provenance) -> String {
    let title = draft.title.trim();
    let mut out = format!(
        "---\nname: {name}\ndescription: {}\nmetadata:\n  oido_run: {}\n  oido_agent: {}\n  oido_made: {}\n---\n\n# {title}\n\n",
        quote(&description(title, &draft.use_when)),
        quote(from.run),
        quote(from.agent),
        quote(from.made),
    );
    let when = draft
        .use_when
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let when = when.trim_end_matches('.');
    if !when.is_empty() {
        out.push_str(&format!("Use this skill {when}.\n\n"));
    }
    out.push_str(draft.body.trim());
    out.push('\n');
    out
}

/// A double-quoted YAML scalar on one line, as `skills::field` reads it back.
fn quote(v: &str) -> String {
    let one = v.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("\"{}\"", one.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Why `text` must not be saved: each secret, personal detail or placeholder in it, with its
/// line. Empty when it may be.
pub fn problems(text: &str) -> Vec<String> {
    let line_of = |value: &str| {
        text.lines()
            .position(|l| l.contains(value))
            .map_or(0, |i| i + 1)
    };
    let mut out: Vec<String> = secrets::find(text)
        .into_iter()
        .chain(secrets::find_personal(text))
        .map(|f| format!("a {} on line {}", f.kind, line_of(&f.value)))
        .collect();
    if let Some(p) = secrets::first_placeholder(text) {
        out.push(format!(
            "the placeholder {p} on line {}, where a hidden secret was",
            line_of(p)
        ));
    }
    out
}

/// Where the skill called `name` goes in `dir`.
pub fn path(dir: &Path, name: &str) -> PathBuf {
    dir.join(name).join("SKILL.md")
}

/// Writes `text` as the skill `name` in `dir`. An existing skill is only replaced when `replace`
/// says so; the file is written beside it and renamed, so it is never left half-written.
pub fn install(dir: &Path, name: &str, text: &str, replace: bool) -> Result<PathBuf, String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(format!("\"{name}\" cannot be a skill's name."));
    }
    let file = path(dir, name);
    if file.exists() && !replace {
        return Err(format!("A skill called {name} is already there."));
    }
    let folder = dir.join(name);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Could not make {}: {e}", folder.display()))?;
    let tmp = folder.join("SKILL.md.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Could not save {}: {e}", file.display())
    })?;
    Ok(file)
}

/// Where the form is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The model is writing the draft.
    Drafting,
    Editing,
    /// Saved, as the skill named.
    Saved(String),
}

/// The draft form on the Runs screen, for one run.
#[derive(Debug)]
pub struct Form {
    pub run_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub phase: Phase,
    /// Why the draft is empty, or what stops saving.
    pub note: Option<String>,
    pub title: TextEdit,
    pub use_when: TextEdit,
    pub body: TextEdit,
    /// The skill already there under the name being saved, and its text: shown before Replace.
    pub existing: Option<(String, String)>,
    cancel: Arc<AtomicBool>,
}

impl Form {
    pub(crate) fn new(run: &Run) -> Form {
        Form {
            run_id: run.id.clone(),
            agent_id: run.agent_id.clone(),
            agent_name: run.agent_name.clone(),
            ..Form::blank()
        }
    }

    /// A form for no run, so a field's editor always has something to edit.
    fn blank() -> Form {
        Form {
            run_id: String::new(),
            agent_id: String::new(),
            agent_name: String::new(),
            phase: Phase::Drafting,
            note: None,
            title: TextEdit::new(""),
            use_when: TextEdit::new(""),
            body: TextEdit::new(""),
            existing: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn fill(&mut self, d: &Draft) {
        self.title = TextEdit::new(d.title.clone());
        self.use_when = TextEdit::new(d.use_when.clone());
        self.body = TextEdit::new(d.body.clone());
    }

    /// The draft as it stands in the fields.
    pub fn draft(&self) -> Draft {
        Draft {
            title: self.title.text.trim().to_owned(),
            use_when: self.use_when.text.trim().to_owned(),
            body: self.body.text.trim().to_owned(),
        }
    }

    /// The name it would be saved under.
    pub fn name(&self) -> String {
        slug(&self.title.text)
    }

    pub fn ready(&self) -> bool {
        self.phase == Phase::Editing && !self.name().is_empty() && !self.body.text.trim().is_empty()
    }
}

impl App {
    /// Opens the draft form for the open run and asks its agent's model to fill it. With no way
    /// to ask the model, the form opens empty, saying why.
    pub fn make_skill(&mut self) {
        let Some(run) = self.runs.get(self.run).cloned() else {
            return;
        };
        if let Some(f) = &self.skill_draft {
            f.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let mut form = Form::new(&run);
        let target = self.target_for(&run.model.provider);
        let (endpoint, credential) = match target {
            Ok(t) => t,
            Err(why) => {
                form.phase = Phase::Editing;
                form.note = Some(format!("No draft: {why} Write the skill yourself."));
                self.skill_draft = Some(form);
                return;
            }
        };
        let ask = request(&run.model.id, &self.steps);
        // Runs record secrets as placeholders already; the guard hides anything else and takes
        // out invisible characters. The answer is never revealed: a placeholder left in it
        // blocks saving.
        let ask = self
            .guard
            .request(ask, &mut crate::guard::Hidden::default());
        let cancel = Arc::clone(&form.cancel);
        let who = self.providers.label_of(&run.model.provider);
        let sent = crate::run::transcript(&ask.request().turns);
        let began = Instant::now();
        self.skill_draft = Some(form);
        self.spawn(
            move || completion::complete(&endpoint, credential.as_ref(), &ask, &cancel),
            move |app, got| app.skill_drafted(&run, &sent, began, &who, got),
        );
    }

    /// The model answered: the form is filled, and the call is kept as a run of its own, so its
    /// cost shows with the agent's.
    fn skill_drafted(
        &mut self,
        from: &Run,
        sent: &str,
        began: Instant,
        who: &str,
        got: Result<Completion, Error>,
    ) {
        let (text, tokens, failure) = match got {
            Ok(c) => (c.text, c.tokens, None),
            Err(e) => (String::new(), Tokens::default(), Some(e.describe(who))),
        };
        self.record_draft(from, sent, &text, tokens, began, failure.as_deref());
        let Some(form) = self
            .skill_draft
            .as_mut()
            .filter(|f| f.run_id == from.id && f.phase == Phase::Drafting)
        else {
            return;
        };
        form.phase = Phase::Editing;
        match (failure, parse(&text)) {
            (Some(why), _) => {
                form.note = Some(format!(
                    "No draft: {why} Write the skill yourself, or try again."
                ));
            }
            (None, Parsed::Nothing(why)) => {
                form.note = Some(format!(
                    "{who} found nothing reusable in this run: {why} You can still write one."
                ));
            }
            (None, Parsed::Draft(d)) if d.body.is_empty() => {
                form.note = Some(format!("{who} sent back an empty draft."));
            }
            (None, Parsed::Draft(d)) => form.fill(&d),
        }
    }

    fn record_draft(
        &mut self,
        from: &Run,
        sent: &str,
        text: &str,
        tokens: Tokens,
        began: Instant,
        failure: Option<&str>,
    ) {
        let mut run = Run::new(
            &crate::model::new_id(),
            &from.agent_id,
            &from.agent_name,
            &from.session_id,
            from.model.clone(),
        );
        run.title = format!("Skill draft: {}", from.title);
        let mut trace = Trace::new(run);
        let mut steps = trace.reply(&Reply {
            sent,
            text,
            reasoning: "",
            calls: &[],
            failure,
            status: if failure.is_some() {
                Status::Error
            } else {
                Status::Ok
            },
            tokens,
            began,
            first: None,
        });
        if let Some(call) = steps.first_mut() {
            call.name = STEP_NAME.to_owned();
        }
        let open = self.runs.get(self.run).map(|r| r.id.clone());
        self.save("the run", |db| db.start_run(&trace.run));
        self.save_run(&trace.run, &steps);
        if self.route == crate::app::Route::Runs {
            self.reload_runs();
            // Stay on the run the form is for.
            if let Some(i) = open.and_then(|id| self.runs.iter().position(|r| r.id == id)) {
                self.select_run(i);
            }
        }
    }

    /// Saves the form as a skill. A secret or personal detail in it stops that, saying where; a
    /// skill already there under that name is shown first, and replaced on the next press.
    pub fn save_skill_draft(&mut self) {
        let Some(dir) = self.skills_dir.clone() else {
            return;
        };
        let Some(form) = self.skill_draft.as_mut().filter(|f| f.ready()) else {
            return;
        };
        let (name, draft) = (form.name(), form.draft());
        let made = chrono::Local::now().format("%Y-%m-%d").to_string();
        let text = render(
            &name,
            &draft,
            &Provenance {
                run: &form.run_id,
                agent: &form.agent_name,
                made: &made,
            },
        );
        let found = problems(&text);
        if !found.is_empty() {
            form.note = Some(format!(
                "Not saved: take out {} first. A skill is kept and sent to models, so it must hold none.",
                found.join(", ")
            ));
            return;
        }
        let file = path(&dir, &name);
        let confirmed = form.existing.as_ref().is_some_and(|(n, _)| *n == name);
        if file.exists() && !confirmed {
            let old = std::fs::read_to_string(&file).unwrap_or_default();
            form.existing = Some((name, old));
            form.note = None;
            return;
        }
        match install(&dir, &name, &text, confirmed) {
            Ok(_) => {
                form.phase = Phase::Saved(name);
                form.note = None;
                form.existing = None;
                self.reload_skills();
            }
            Err(why) => form.note = Some(why),
        }
    }

    /// Gives the saved skill to the agent whose run it came from.
    pub fn give_skill_draft(&mut self) {
        let Some((agent_id, Phase::Saved(name))) = self
            .skill_draft
            .as_ref()
            .map(|f| (f.agent_id.clone(), f.phase.clone()))
        else {
            return;
        };
        let Some(target) = self.target_of_agent(&agent_id) else {
            return;
        };
        if let Some(a) = self.roster.find_mut(&target)
            && !a.skills.contains(&name)
        {
            a.skills.push(name);
            self.save_agent_row(&target);
        }
    }

    /// Whether the saved skill can still be given to its run's agent: it is there and lacks it.
    pub fn can_give_skill_draft(&self) -> bool {
        let Some(f) = &self.skill_draft else {
            return false;
        };
        let Phase::Saved(name) = &f.phase else {
            return false;
        };
        self.roster
            .agent_ref(&f.agent_id)
            .is_some_and(|a| !a.skills.contains(name))
    }

    /// The open draft form, for its fields' editors.
    pub fn skill_draft_mut(&mut self) -> &mut Form {
        self.skill_draft.get_or_insert_with(Form::blank)
    }

    pub fn close_skill_draft(&mut self) {
        if let Some(f) = self.skill_draft.take() {
            f.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn target_of_agent(&self, id: &str) -> Option<crate::agents::Target> {
        self.roster.list.iter().find_map(|a| {
            if a.id == id {
                Some(crate::agents::Target::agent(a.id.clone()))
            } else {
                a.subagents
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| crate::agents::Target::member(a.id.clone(), m.id.clone()))
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn step(kind: StepKind, name: &str, input: &str, output: &str) -> Step {
        let mut s = Step::new(1, kind, 0);
        s.name = name.into();
        s.input = Some(input.into()).filter(|t: &String| !t.is_empty());
        s.output = Some(output.into());
        s
    }

    #[test]
    fn the_transcript_leaves_out_setup_and_keeps_the_start_and_end_of_a_long_run() {
        let mut steps = vec![
            step(StepKind::System, "", "", "the brief"),
            step(StepKind::User, "", "", "fix the test"),
            step(StepKind::Guard, "hid", "", "1 secret"),
            step(
                StepKind::Tool,
                "shell",
                "{\"command\":\"cargo test\"}",
                "1 failed",
            ),
            step(StepKind::Llm, "", "everything sent", "Fixed it."),
        ];
        let t = transcript(&steps);
        assert_eq!(
            t,
            "[person] fix the test\n\n[tool shell] {\"command\":\"cargo test\"}\n→ 1 failed\n\n[agent] Fixed it."
        );
        assert!(
            !t.contains("everything sent"),
            "what a call was sent repeats the chat"
        );

        steps = (0..100)
            .map(|i| step(StepKind::Llm, "", "", &format!("reply {i}")))
            .collect();
        let t = transcript(&steps);
        assert!(
            t.contains("reply 39\n\n[20 steps left out here]\n\n[agent] reply 60"),
            "{t}"
        );
        assert!(t.starts_with("[agent] reply 0") && t.ends_with("reply 99"));

        let long = step(StepKind::Llm, "", "", &"x".repeat(5_000));
        assert_eq!(
            transcript(&[long]).len(),
            "[agent] ".len() + STEP_BYTES + "…".len()
        );
    }

    #[test]
    fn an_answer_is_read_as_a_draft_a_refusal_or_a_bare_body() {
        let d = parse(
            "```json\n{\"title\": \"Retry flaky tests\", \"use_when\": \"when a test fails once\", \"body\": \"Run it twice.\"}\n```",
        );
        assert_eq!(
            d,
            Parsed::Draft(Draft {
                title: "Retry flaky tests".into(),
                use_when: "when a test fails once".into(),
                body: "Run it twice.".into(),
            })
        );
        assert_eq!(
            parse("{\"none\": \"It was small talk.\"}"),
            Parsed::Nothing("It was small talk.".into())
        );
        assert_eq!(
            parse("Just do X."),
            Parsed::Draft(Draft {
                body: "Just do X.".into(),
                ..Draft::default()
            })
        );
    }

    #[test]
    fn a_name_is_lowercase_words_joined_by_dashes_within_64_bytes() {
        assert_eq!(
            slug("Retry a flaky `cargo test`!"),
            "retry-a-flaky-cargo-test"
        );
        assert_eq!(slug("  ../../etc  "), "etc");
        assert_eq!(slug("Ünïcode only ☃"), "n-code-only");
        assert_eq!(slug("…"), "");
        let long = slug(&"word ".repeat(40));
        assert!(long.len() <= MAX_NAME && !long.ends_with('-'), "{long}");
    }

    #[test]
    fn the_description_says_what_and_when_within_the_limit() {
        assert_eq!(
            description("Retry flaky tests. More text.", "when a test fails once."),
            "Retry flaky tests. Use when a test fails once."
        );
        assert_eq!(
            description("Retry", "Use when\n  flaky"),
            "Retry. Use when flaky."
        );
        assert_eq!(description("Retry", ""), "Retry.");
        let long = description("T", &"w ".repeat(1_000));
        assert!(long.chars().count() <= MAX_DESCRIPTION);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn a_rendered_skill_reads_back_with_its_name_and_description() {
        let draft = Draft {
            title: "Say \"hi\" first".into(),
            use_when: "when greeting".into(),
            body: "# Steps\nSay hi.".into(),
        };
        let text = render(
            "say-hi-first",
            &draft,
            &Provenance {
                run: "r1",
                agent: "Scout",
                made: "2026-11-26",
            },
        );
        let dir = std::env::temp_dir().join(format!("oido-draft-{}", uuid::Uuid::new_v4()));
        install(&dir, "say-hi-first", &text, false).unwrap();
        let found = crate::skills::discover(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "say-hi-first");
        assert_eq!(found[0].description, "Say \"hi\" first. Use when greeting.");
        let body = crate::skills::body(&dir, "say-hi-first").unwrap();
        assert!(
            body.starts_with("# Say \"hi\" first\n\nUse this skill when greeting.\n\n# Steps"),
            "{body}"
        );
        assert!(text.contains("  oido_run: \"r1\"\n"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_skill_is_only_replaced_when_asked_and_never_outside_the_folder() {
        let dir = std::env::temp_dir().join(format!("oido-draft-{}", uuid::Uuid::new_v4()));
        install(&dir, "a", "one", false).unwrap();
        assert!(
            install(&dir, "a", "two", false)
                .unwrap_err()
                .contains("already there")
        );
        assert_eq!(std::fs::read_to_string(path(&dir, "a")).unwrap(), "one");
        install(&dir, "a", "two", true).unwrap();
        assert_eq!(std::fs::read_to_string(path(&dir, "a")).unwrap(), "two");
        assert!(!dir.join("a").join("SKILL.md.tmp").exists());
        for bad in ["", "../x", "a/b", ".hidden"] {
            assert!(install(&dir, bad, "x", true).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn secrets_personal_details_and_placeholders_stop_a_save() {
        assert!(problems("Run cargo test twice.").is_empty());
        let found = problems("ok\nkey sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCD");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].ends_with("on line 2"), "{found:?}");
        assert!(problems("Mail me@example.com")[0].contains("email address"));
        let marked = {
            let mut h = crate::guard::Hidden::default();
            h.learn("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCD");
            h.hide("use sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCD")
        };
        assert!(
            problems(&marked).iter().any(|p| p.contains("placeholder")),
            "{marked}"
        );
    }

    #[test]
    fn a_run_is_drafted_into_a_skill_saved_and_given_to_its_agent() {
        use crate::testing::{EVENTS, TempDb, app_with_server, text_stream};
        let db = TempDb::new();
        let draft = r#"{"title": "Check the log first", "use_when": "when a service is down", "body": "Read the last 50 lines of the log before restarting."}"#;
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, EVENTS, text_stream("The log said the disk was full.")),
                (200, EVENTS, text_stream(draft)),
            ],
        );
        let dir = std::env::temp_dir().join(format!("oido-draft-{}", uuid::Uuid::new_v4()));
        app.skills_dir = Some(dir.clone());
        app.composer.edit = TextEdit::new("why is the service down?");
        app.send();
        let _ = seen.recv().unwrap();
        app.go(crate::app::Route::Runs);
        app.reload_runs();
        app.select_run(0);
        let source = app.runs[0].id.clone();

        app.make_skill();
        let sent = seen.recv().unwrap();
        assert!(
            sent.contains("the disk was full"),
            "the draft is made from the run's steps"
        );
        let f = app.skill_draft.as_ref().unwrap();
        assert_eq!(f.phase, Phase::Editing);
        assert_eq!(f.run_id, source, "the form stays on the run it is for");
        assert_eq!(f.name(), "check-the-log-first");
        assert_eq!(app.runs[app.run].id, source);
        assert!(
            app.runs
                .iter()
                .any(|r| r.title.starts_with("Skill draft: ") && r.calls == 1),
            "the draft call is kept as a run, so its cost is counted"
        );

        app.save_skill_draft();
        let f = app.skill_draft.as_ref().unwrap();
        assert_eq!(
            f.phase,
            Phase::Saved("check-the-log-first".into()),
            "{:?}",
            f.note
        );
        assert!(app.skills.iter().any(|s| s.name == "check-the-log-first"));
        assert!(app.can_give_skill_draft());
        app.give_skill_draft();
        assert!(!app.can_give_skill_draft());
        let agent_id = app.skill_draft.as_ref().unwrap().agent_id.clone();
        let agent = app.roster.agent_ref(&agent_id).unwrap();
        assert!(agent.skills.contains(&"check-the-log-first".to_owned()));

        // Drafting the same name again shows the skill there before replacing it.
        let mut again = Form::new(&app.runs[app.run]);
        again.phase = Phase::Editing;
        let Parsed::Draft(d) = parse(draft) else {
            panic!("a draft")
        };
        again.fill(&d);
        app.skill_draft = Some(again);
        app.save_skill_draft();
        let f = app.skill_draft.as_ref().unwrap();
        assert!(matches!(f.phase, Phase::Editing) && f.existing.is_some());
        app.save_skill_draft();
        assert!(matches!(
            app.skill_draft.as_ref().unwrap().phase,
            Phase::Saved(_)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
