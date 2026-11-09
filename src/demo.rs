//! Sample data and named screens for looking at the app without real data: the headless renders
//! (`--scene NAME`, debug builds only) and the tests use them. A fresh install has none of this:
//! it starts with no agents, and nothing here is in a release build.

use rune::TextEdit;

use crate::agents::{Agent, Roster, Target};
use crate::app::{self, App, Route};
use crate::listing;
use crate::model::{AgentStatus, Approval, Message, Outcome, Stats, ToolCall, chat};
use crate::models::{ModelRef, RawModel};
use crate::providers::NEW_SERVER;

fn member(id: &str, name: &str, hue: f32, prompt: &str) -> Agent {
    Agent {
        name: name.into(),
        prompt: prompt.into(),
        hue: Some(hue),
        ..Agent::blank(id.into())
    }
}

pub fn sample_agents() -> Vec<Agent> {
    vec![
        Agent {
            id: "atlas".into(),
            name: "Atlas".into(),
            prompt: "You research a topic thoroughly and write short briefings. Cite where each number came from, and say so when you are unsure.".into(),
            model: None,
            hue: Some(150.0),
            pinned: true,
            status: AgentStatus::Waiting,
            skills: vec!["web-research".into(), "summarize".into(), "wiki".into()],
            tools: vec!["storage.read".into(), "wiki.search".into(), "slack.post".into()],
            subagents: vec![member("fact-checker", "fact-checker", 60.0, "You check every number in a draft against its source before it goes out.")],
            workdir: None,
            description: String::new(),
            active: 0,
            sessions: vec![
                chat(
                    "Monday brief",
                    vec![
                        Message::User(
                            "Pull last week's churn numbers and draft the Monday brief.".into(),
                        ),
                        Message::Assistant {
                            text: "I'll read the usage export, then draft the brief. One step needs your sign-off before it posts.".into(),
                            thought_secs: Some(3.4),
                            tools: vec![
                                ToolCall {
                                    name: "storage.read".into(),
                                    summary: "exports/usage-week-44.csv".into(),
                                    output: Some("1,284 rows · 9 columns".into()),
                                    ..Default::default()
                                },
                                ToolCall {
                                    name: "wiki.search".into(),
                                    summary: "\"churn definition\"".into(),
                                    output: Some("Churn = paid orgs with no run in 30 days.".into()),
                                    ..Default::default()
                                },
                            ],
                            approval: None,
                            stats: Some(Stats {
                                model: "claude-sonnet-5-5".into(),
                                tokens: 2140,
                                seconds: 6.2,
                            }),
 run: None,
 outcome: Outcome::Done,
                        },
                        Message::Assistant {
                            text: "## Monday brief\n\n- **Churn:** 2.1%, down from 2.8%\n- **Active orgs:** 412 (+9)\n- **Watch:** three orgs idle for 25+ days\n\nReady to post this to `#leadership`.".into(),
                            thought_secs: None,
                            tools: vec![ToolCall {
                                name: "slack.post".into(),
                                summary: "#leadership".into(),
                                output: None,
                                ..Default::default()
                            }],
                            approval: Some(Approval {
                                action: "Post the Monday brief to #leadership".into(),
                                decision: None,
                            }),
                            stats: None,
 run: None,
 outcome: Outcome::Done,
                        },
                    ],
                ),
                chat("Churn deep-dive", vec![]),
            ],
        },
        Agent {
            id: "forge".into(),
            name: "Forge".into(),
            prompt: "You review pull requests and run the test suite. Be specific about what to change and why.".into(),
            model: None,
            hue: Some(250.0),
            pinned: true,
            status: AgentStatus::Running,
            skills: vec!["github".into(), "code-review".into(), "tests".into()],
            tools: vec!["github.diff".into(), "shell.run".into()],
            subagents: vec![member("reviewer", "reviewer", 200.0, "You read a diff for unsafe code and missing tests, and report only what matters.")],
            workdir: None,
            description: String::new(),
            active: 0,
            sessions: vec![chat(
                "Review PR 482",
                vec![
                    Message::User("Review PR 482 and run the test suite.".into()),
                    Message::Assistant {
                        text: "Reviewing the diff and running `cargo test` now.".into(),
                        thought_secs: None,
                        tools: vec![
                            ToolCall {
                                name: "github.diff".into(),
                                summary: "PR 482".into(),
                                output: Some("6 files · +212 −48".into()),
                                ..Default::default()
                            },
                            ToolCall {
                                name: "shell.run".into(),
                                summary: "cargo test".into(),
                                output: None,
                                ..Default::default()
                            },
                        ],
                        approval: None,
                        stats: None,
 run: None,
 outcome: Outcome::Done,
                    },
                ],
            )],
        },
        Agent {
            id: "scout".into(),
            name: "Scout".into(),
            prompt: "You triage the inbox: flag what needs a reply today and summarise the rest.".into(),
            model: None,
            hue: Some(30.0),
            pinned: false,
            status: AgentStatus::Idle,
            skills: vec!["gmail".into(), "calendar".into()],
            tools: vec!["gmail.list".into()],
            subagents: vec![],
            workdir: None,
            description: String::new(),
            active: 0,
            sessions: vec![chat("New chat", vec![])],
        },
        Agent {
            id: "ledger".into(),
            name: "Ledger".into(),
            prompt: "You reconcile invoices against payments and report anything that does not match.".into(),
            model: None,
            hue: Some(330.0),
            pinned: false,
            status: AgentStatus::Idle,
            skills: vec!["stripe".into(), "sheets".into()],
            tools: vec![],
            subagents: vec![],
            workdir: None,
            description: String::new(),
            active: 0,
            sessions: vec![chat("New chat", vec![])],
        },
    ]
}

/// Replaces the agents with the samples (Atlas, Forge, Scout, Ledger and two team members).
pub fn load_samples(app: &mut App) {
    app.roster = Roster::from_saved(sample_agents(), None);
}

/// Connects a few providers with made-up model lists, as if they had been checked.
pub fn demo_models(app: &mut App) {
    let list = |ids: &[&str]| {
        listing::finish_all(
            ids.iter()
                .map(|id| RawModel {
                    id: (*id).into(),
                    ..RawModel::default()
                })
                .collect(),
        )
    };
    for (id, ids) in [
        (
            "anthropic",
            vec![
                "claude-opus-4-7",
                "claude-sonnet-4-6",
                "claude-haiku-4-5-20251001",
                "claude-3-5-sonnet-20241022",
            ],
        ),
        (
            "openai",
            vec![
                "gpt-4o",
                "gpt-4o-mini",
                "text-embedding-3-large",
                "whisper-1",
            ],
        ),
        (
            "deepseek",
            vec!["deepseek-v4-pro", "deepseek-reasoner", "deepseek-v3.2"],
        ),
    ] {
        app.providers.connect(id);
        app.providers.set_models(id, list(&ids), app::now());
        app.providers.set_hint(id, "sk-demo-1234567890");
    }
    app.providers.default_model = Some(ModelRef::new("anthropic", "claude-sonnet-4-6"));
}

/// `--scene NAME`: the app in a state worth looking at. With no scene it opens as a new install does.
/// A scene: its name and how it sets the app up.
type Scene = (&'static str, fn(&mut App));

pub static SCENES: &[Scene] = &[
    // The Settings page as this computer's Omarchy themes make it: following the desktop, then in
    // a light theme.
    ("settings", |s| {
        s.look.appearance = crate::theme::Appearance::System;
        s.theme_open = true;
        s.route = Route::Settings;
    }),
    ("settings-light", |s| {
        s.look.appearance = crate::theme::Appearance::Omarchy("flexoki-light".into());
        s.theme_open = true;
        s.route = Route::Settings;
    }),
    ("mcp-search", |s| {
        s.mcp.search = rune::TextEdit::new("cloud work");
        s.route = Route::Mcp;
        crate::ui::wait_for_logos(&["https://docs.mcp.cloudflare.com/mcp"]);
    }),
    ("mcp", |s| {
        use crate::mcp::{Auth, Server, Status, Tool, Transport};
        let tool = |n: &str, d: &str| Tool {
            name: n.into(),
            description: d.into(),
            schema: serde_json::json!({"type": "object"}),
        };
        let server = |name: &str, transport, auth, status, detail: &str, tools| Server {
            id: name.to_lowercase().replace(' ', "-"),
            name: name.into(),
            transport,
            auth,
            enabled: true,
            status,
            detail: detail.into(),
            tools,
        };
        s.mcp.servers = vec![
            server(
                "Context7",
                Transport::Http {
                    url: "https://mcp.context7.com/mcp".into(),
                },
                Auth::None,
                Status::Reachable,
                "",
                vec![
                    tool("resolve-library-id", "Find the library a name refers to."),
                    tool("get-library-docs", "Fetch current docs for a library."),
                ],
            ),
            server(
                "GitHub",
                Transport::Http {
                    url: "https://api.githubcopilot.com/mcp/".into(),
                },
                Auth::OAuth,
                Status::Unreachable,
                "the server did not accept the sign-in or key. Sign in or add the key again on the MCP page.",
                vec![],
            ),
            server(
                "Everything",
                Transport::Stdio {
                    command: "npx".into(),
                    args: vec![
                        "-y".into(),
                        "@modelcontextprotocol/server-everything".into(),
                    ],
                },
                Auth::None,
                Status::Reachable,
                "",
                vec![tool("echo", "Echoes the message back.")],
            ),
        ];
        crate::ui::wait_for_logos(&[
            "https://mcp.context7.com/mcp",
            "https://api.githubcopilot.com/mcp/",
        ]);
        s.mcp.open = Some("context7".into());
        s.mcp.adding = true;
        s.route = Route::Mcp;
    }),
    ("empty-chat", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
    }),
    ("forge", |s| {
        load_samples(s);
        s.roster.select(Target::agent("forge"));
    }),
    ("runs", |s| {
        demo_runs(s);
        s.route = Route::Runs;
    }),
    ("reply-failed", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        if let Some(a) = s.roster.current_mut() {
            a.chat_mut().messages = vec![
                Message::User("Summarise the release notes.".into()),
                Message::reply(
                    "The notes cover three",
                    Some("r1".into()),
                    Outcome::Failed("OpenAI is limiting requests. Wait a moment and retry.".into()),
                ),
            ];
        }
    }),
    ("question", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        if let Some(a) = s.roster.current_mut() {
            a.chat_mut().messages = vec![
                Message::User("Write the launch announcement.".into()),
                Message::Assistant {
                    text: "Before I draft it, I need one detail.".into(),
                    thought_secs: None,
                    tools: vec![ToolCall {
                        name: "ask_user_question".into(),
                        summary: "Who is it for?".into(),
                        output: None,
                        id: "c1".into(),
                        args: r#"{"question":"Who is the announcement for?","options":["Customers","Investors","The team"],"multiple":true}"#.into(),
                    }],
                    approval: None,
                    stats: None,
                    run: Some("r1".into()),
                    outcome: Outcome::Done,
                },
            ];
        }
        s.picks = vec!["Customers".into()];
    }),
    ("command", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        if let Some(a) = s.roster.current_mut() {
            let call = |id: &str, cmd: &str, out: Option<&str>| ToolCall {
                name: "shell".into(),
                summary: cmd.into(),
                output: out.map(str::to_owned),
                id: id.into(),
                args: format!(r#"{{"command":"{cmd}","reason":"See what is taking the space."}}"#),
            };
            let reply = |tools| Message::Assistant {
                text: "Let me look.".into(),
                thought_secs: None,
                tools,
                approval: None,
                stats: None,
                run: Some("r1".into()),
                outcome: Outcome::Done,
            };
            a.chat_mut().messages = vec![
                Message::User("Why is my disk full?".into()),
                reply(vec![call(
                    "a",
                    "du -sh ~/* | sort -h | tail -3",
                    Some("exit code: 0\nstdout:\n2.1G\t/home/me/Videos\n9.4G\t/home/me/Projects"),
                )]),
                Message::Assistant {
                    text: "Projects is the biggest. I want to check its build folders.".into(),
                    thought_secs: None,
                    tools: vec![call(
                        "b",
                        "du -sh ~/Projects/*/target | sort -h | tail -3",
                        None,
                    )],
                    approval: None,
                    stats: None,
                    run: Some("r2".into()),
                    outcome: Outcome::Done,
                },
            ];
        }
    }),
    ("reply-working", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        if let Some(a) = s.roster.current_mut() {
            a.chat_mut().messages = vec![
                Message::User("Summarise the release notes.".into()),
                Message::reply("", Some("r1".into()), Outcome::Working),
            ];
        }
        s.live.insert("demo".into(), crate::run::Live::demo());
    }),
    ("agent-new", |s| s.new_agent()),
    ("agent-new-models", |s| {
        demo_models(s);
        s.new_agent();
    }),
    ("agent-new-template", |s| {
        s.new_agent();
        s.roster.pick_template(0);
    }),
    ("agent-edit", |s| {
        load_samples(s);
        s.edit_agent(&Target::agent("atlas"));
        s.roster.form.advanced = true;
    }),
    ("tools", |s| {
        load_samples(s);
        if let Some(a) = s.roster.agent_mut("atlas") {
            a.tools = vec![crate::tools::ASK.into()];
        }
        s.route = Route::Tools;
    }),
    ("tools-open", |s| {
        load_samples(s);
        s.edit_agent(&Target::agent("atlas"));
        s.roster.form.advanced = true;
        s.roster.form.tools = vec![crate::tools::ASK.into()];
        s.roster.form.tool_picker.show();
    }),
    ("delegation", |s| {
        use crate::delegate::{Delegated, Stage};
        use crate::model::{Message, Outcome, ToolCall};
        use std::time::{Duration, Instant};
        load_samples(s);
        let ago = |secs: u64| {
            Instant::now()
                .checked_sub(Duration::from_secs(secs))
                .unwrap_or_else(Instant::now)
        };
        let stage = |label: &str, detail: &str, from: u64, to: Option<u64>, ok: bool| Stage {
            label: label.into(),
            detail: detail.into(),
            began: ago(from),
            ended: to.map(ago),
            ok,
        };
        let call = |id: &str, who: &str, task: &str, out: Option<&str>| ToolCall {
            name: crate::tools::DELEGATE.into(),
            summary: format!("{who}: {task}"),
            output: out.map(str::to_owned),
            id: id.into(),
            args: String::new(),
        };
        let card = |stages: Vec<Stage>,
                    text: &str,
                    began: u64,
                    ended: Option<u64>,
                    input: u32,
                    output: u32| Delegated {
            sid: "demo".into(),
            member_id: "fact-checker".into(),
            began: ago(began),
            ended: ended.map(ago),
            text: text.into(),
            stages,
            input,
            output,
            cancel: Default::default(),
        };
        s.delegated.insert(
            "d1".into(),
            card(
                vec![
                    stage("Thinking", "", 14, Some(12), true),
                    stage("shell", "grep -c churn export.csv", 12, Some(11), true),
                    stage("Thinking", "", 11, Some(5), true),
                    stage("shell", "cat sources.md", 5, Some(4), false),
                    stage("Thinking", "", 4, None, true),
                ],
                "Two of the three figures match their source. The active-orgs number is 412 in the brief but 409 in",
                14,
                None,
                2300,
                410,
            ),
        );
        s.delegated
            .insert("d2".into(), card(vec![], "", 31, Some(3), 1800, 220));
        if let Some(a) = s.roster.agent_mut("atlas") {
            a.chat_mut().messages.push(Message::User(
                "Check the Monday brief before it goes out.".into(),
            ));
            a.chat_mut().messages.push(Message::Assistant {
                text: String::new(),
                thought_secs: None,
                tools: vec![
                    call(
                        "d1",
                        "fact-checker",
                        "check every number in the draft against its source",
                        None,
                    ),
                    call(
                        "d2",
                        "fact-checker",
                        "verify the headline churn figure",
                        Some("Confirmed: 2.1% matches the usage export."),
                    ),
                    call(
                        "d3",
                        "fact-checker",
                        "check the pricing table",
                        Some("Error: fact-checker has no model that can answer."),
                    ),
                ],
                approval: None,
                stats: None,
                run: None,
                outcome: Outcome::Working,
            });
            a.status = crate::model::AgentStatus::Running;
        }
        if let Some(m) = s.roster.agent_mut("fact-checker") {
            m.status = crate::model::AgentStatus::Running;
        }
    }),
    ("workdir-open", |s| {
        load_samples(s);
        s.edit_agent(&Target::agent("atlas"));
        s.roster.form.advanced = true;
        s.roster.form.workdir = Some("/home".into());
        s.roster.form.folders.show(Some(std::path::Path::new("/")));
    }),
    ("workflows", |s| {
        use crate::workflow::definition::{Definition, Input, Step};
        use crate::workflow::engine::StepLog;
        use crate::workflow::{RunRecord, RunStatus, Saved};
        load_samples(s);
        let step = |id: &str, kind: &str| Step {
            id: id.into(),
            kind: kind.into(),
            ..Step::default()
        };
        let mut list = Step {
            tool: Some("shell".into()),
            args: serde_json::json!({"command": "git log --since={{inputs.since}} --oneline"})
                .as_object()
                .cloned(),
            bind: Some("log".into()),
            ..step("log", "tool")
        };
        list.bind = Some("log".into());
        let write = Step {
            instruction: Some("Write a short changelog from {{log.stdout}}".into()),
            ..step("write", "generate")
        };
        let def = Definition {
            inputs: [(
                "since".to_owned(),
                Input {
                    kind: "string".into(),
                    description: "How far back".into(),
                    default: Some("1 week ago".into()),
                },
            )]
            .into(),
            steps: vec![list, write],
            success: "A changelog was written.".into(),
        };
        let w = Saved {
            id: "w1".into(),
            agent_id: "atlas".into(),
            name: "Weekly changelog".into(),
            goal: "Turns the week's commits into a changelog.".into(),
            definition: def,
            created_at: app::now(),
        };
        let other = Saved {
            id: "w2".into(),
            name: "Tidy downloads".into(),
            goal: String::new(),
            ..w.clone()
        };
        s.workflows = vec![w, other];
        s.wf_open = Some("w1".into());
        s.wf_inputs = vec![("since".into(), TextEdit::new("1 week ago"))];
        let log = |what: &str, out: &str, err: Option<&str>| StepLog {
            id: "x".into(),
            kind: "tool".into(),
            what: what.into(),
            output: out.into(),
            error: err.map(str::to_owned),
            ms: 120,
        };
        s.wf_runs = vec![
            RunRecord {
                id: "r1".into(),
                workflow_id: "w1".into(),
                started_by: "you".into(),
                status: RunStatus::Succeeded,
                inputs: "{}".into(),
                output: "Added runs.".into(),
                failed_step: String::new(),
                error: String::new(),
                steps: vec![
                    log(
                        "shell: `git log --since=1 week ago --oneline`",
                        "a1b2c3 Add runs screen",
                        None,
                    ),
                    log(
                        "Write: Write a short changelog from {{log.stdout}}",
                        "Added the runs screen.",
                        None,
                    ),
                ],
                input_tokens: 420,
                output_tokens: 60,
                started_at: app::now() - 300,
                finished_at: None,
            },
            RunRecord {
                id: "r2".into(),
                workflow_id: "w1".into(),
                started_by: "agent".into(),
                status: RunStatus::Failed,
                inputs: "{}".into(),
                output: String::new(),
                failed_step: "log".into(),
                error: "exit code: 128".into(),
                steps: vec![log("shell: `git log`", "", Some("exit code: 128"))],
                input_tokens: 0,
                output_tokens: 0,
                started_at: app::now() - 90000,
                finished_at: None,
            },
        ];
        s.route = Route::Workflows;
    }),
    ("draft", |s| {
        use crate::workflow::Draft;
        use crate::workflow::definition::{Definition, Step};
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        let sid = s
            .roster
            .current()
            .map(|a| a.chat().id.clone())
            .unwrap_or_default();
        if let Some(a) = s.roster.current_mut() {
            a.chat_mut().messages = vec![
                Message::User("Summarise my notes folder.".into()),
                Message::Assistant {
                    text: "Done: three notes, all short.".into(),
                    thought_secs: None,
                    tools: vec![],
                    approval: None,
                    stats: None,
                    run: None,
                    outcome: Outcome::Done,
                },
            ];
        }
        let list = Step {
            id: "ls".into(),
            kind: "tool".into(),
            tool: Some("shell".into()),
            args: serde_json::json!({"command": "ls notes"})
                .as_object()
                .cloned(),
            bind: Some("files".into()),
            ..Step::default()
        };
        let say = Step {
            id: "say".into(),
            kind: "generate".into(),
            instruction: Some("Summarise {{files.stdout}}".into()),
            ..Step::default()
        };
        s.drafts.insert(
            sid.clone(),
            Draft {
                session_id: sid,
                agent_id: "scout".into(),
                name: "Summarise notes".into(),
                goal: "Lists the notes folder and summarises it.".into(),
                definition: Definition {
                    steps: vec![list, say],
                    success: "A summary was written.".into(),
                    ..Definition::default()
                },
            },
        );
    }),
    ("skills-open", |s| {
        load_samples(s);
        s.edit_agent(&Target::agent("atlas"));
        s.roster.form.advanced = true;
        s.roster.form.skills = vec![
            "ads".into(),
            "analytics".into(),
            "coding-guidelines".into(),
            "cost-audit".into(),
            "cc-skills-golang".into(),
            "marketing-plan".into(),
        ];
        s.roster.form.skill_picker.close();
        s.roster.form.skill_picker.search = TextEdit::new("seo");
    }),
    ("member-new", |s| {
        load_samples(s);
        s.new_member("forge");
    }),
    ("menu", |s| {
        load_samples(s);
        s.roster.open_menu(Target::agent("forge"), None);
    }),
    ("menu-member", |s| {
        load_samples(s);
        s.roster
            .open_menu(Target::member("forge", "reviewer"), None);
    }),
    ("confirm-delete", |s| {
        load_samples(s);
        s.roster.confirm = Some(Target::agent("forge"));
    }),
    ("providers", |s| s.route = Route::Providers),
    ("provider-form", |s| {
        s.route = Route::Providers;
        s.providers.toggle("openai");
    }),
    ("provider-error", |s| {
        s.route = Route::Providers;
        s.providers.toggle("openai");
        s.providers.fail(
            "openai",
            "the system keyring is not available (no secret service), so the key was not saved"
                .into(),
        );
    }),
    ("server-form", |s| {
        s.route = Route::Providers;
        s.providers.toggle(NEW_SERVER);
    }),
    ("models", |s| {
        demo_models(s);
        s.route = Route::Providers;
    }),
    ("picker", |s| {
        load_samples(s);
        demo_models(s);
        s.model_picker.show(None);
    }),
    ("picker-search", |s| {
        load_samples(s);
        demo_models(s);
        s.model_picker.show(None);
        s.model_picker.search = TextEdit::new("son");
    }),
    ("picker-empty", |s| {
        load_samples(s);
        s.model_picker.show(None);
    }),
    ("form-model", |s| {
        load_samples(s);
        demo_models(s);
        s.edit_agent(&Target::agent("atlas"));
        s.roster.form.model = Some(ModelRef::new("anthropic", "claude-sonnet-4-6"));
        s.roster
            .form
            .model_picker
            .show(s.roster.form.model.clone().as_ref());
    }),
    ("model-ready", |s| {
        load_samples(s);
        demo_models(s);
        s.set_model(Some(ModelRef::new("anthropic", "claude-opus-4-7")));
    }),
    ("model-gone", |s| {
        load_samples(s);
        demo_models(s);
        s.set_model(Some(ModelRef::new("gemini", "gemini-2.5-pro")));
    }),
];

/// Runs for the Runs screen, as the database would give them.
fn demo_runs(app: &mut App) {
    use crate::runs::{Run, Status, Step, StepKind};
    let run = |id: &str, name: &str, status, ago: i64| Run {
        id: id.into(),
        agent_id: "a".into(),
        agent_name: name.into(),
        session_id: "s".into(),
        model: ModelRef::new("anthropic", "claude-sonnet-4-5"),
        status,
        error: (status == Status::Error).then(|| "Anthropic is limiting requests.".to_owned()),
        input_tokens: Some(812),
        output_tokens: Some(240),
        duration_ms: Some(3400),
        started_at: app::now() - ago,
        ended_at: None,
        parent: None,
    };
    app.runs = vec![
        run("r1", "Atlas", Status::Ok, 120),
        run("r2", "Forge", Status::Error, 4000),
        run("r3", "Atlas", Status::Cancelled, 90000),
    ];
    let step = |seq, kind, out: &str| {
        let mut s = Step::new(seq, kind, 0);
        s.output = Some(out.to_owned());
        s.duration_ms = (kind == StepKind::Llm).then_some(3400);
        s
    };
    let mut call = step(
        3,
        StepKind::Llm,
        "Three things changed: the runs screen is real now.",
    );
    call.input = Some("[system]\nYou plan work.\n\n[user]\nWhat changed?".into());
    call.input_tokens = Some(812);
    call.output_tokens = Some(240);
    call.reasoning = Some("The user wants a summary.".into());
    app.steps = vec![
        step(1, StepKind::System, "You plan work."),
        step(2, StepKind::User, "What changed?"),
        call,
    ];
}
