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
            trust_workdir: true,
            channel_topics: Vec::new(),
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
                            thinking: "The brief needs last week's churn. The usage export has it: read `exports/usage-week-44.csv`, then check the wiki for how churn is defined so the number means what the team expects.".into(),
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
                                context: Some(18_400),
                                trimmed: None,
                            }),
 run: None,
 outcome: Outcome::Done,
                        },
                        Message::Assistant {
                            text: "## Monday brief\n\n- **Churn:** 2.1%, down from 2.8%\n- **Active orgs:** 412 (+9)\n- **Watch:** three orgs idle for 25+ days\n\nReady to post this to `#leadership`.".into(),
                            thinking: String::new(),
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
            trust_workdir: true,
            channel_topics: Vec::new(),
            description: String::new(),
            active: 0,
            sessions: vec![chat(
                "Review PR 482",
                vec![
                    Message::User("Review PR 482 and run the test suite.".into()),
                    Message::Assistant {
                        text: "Reviewing the diff and running `cargo test` now.".into(),
                        thinking: String::new(),
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
            trust_workdir: true,
            channel_topics: Vec::new(),
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
            trust_workdir: true,
            channel_topics: Vec::new(),
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
/// Sample schedules for Atlas and Scout, the first with a few fires.
fn demo_schedules(app: &mut App) {
    use crate::schedule::timing::{AtTime, Days};
    use crate::schedule::{Fire, FireStatus, Schedule, Target, Timing};
    let now = app::now();
    let make = |id: &str, agent: &str, name: &str, prompt: &str, timing: Timing| {
        Schedule::new(
            agent,
            name,
            Target::Prompt(prompt.into()),
            timing,
            now,
            &chrono::Local,
        )
        .map(|mut s| {
            s.id = id.into();
            s
        })
    };
    let weekdays = Timing::at(AtTime {
        hour: 8,
        minute: 30,
        days: Days::WEEKDAYS,
    });
    let list = [
        weekdays.and_then(|t| {
            make(
                "s1",
                "atlas",
                "Morning briefing",
                "Brief me on overnight news about our competitors.",
                t,
            )
        }),
        Timing::every(now - 600, 3600).and_then(|t| {
            make(
                "s2",
                "scout",
                "Inbox sweep",
                "Sort new mail and flag anything urgent.",
                t,
            )
        }),
        Timing::cron("0 17 * * 5").and_then(|t| {
            make(
                "s3",
                "forge",
                "Weekly PR review",
                "Review the open PRs and summarise what is blocked.",
                t,
            )
        }),
    ];
    app.schedules = list.into_iter().filter_map(Result::ok).collect();
    if let Some(s) = app.schedules.get_mut(1) {
        s.enabled = false;
    }
    let fire = |id: &str, ago: i64, status: FireStatus, note: &str| Fire {
        id: id.into(),
        schedule_id: "s1".into(),
        due_at: now - ago,
        started_at: now - ago,
        finished_at: Some(now - ago + 40),
        status,
        note: note.into(),
        session_id: None,
        workflow_run_id: None,
    };
    app.schedule_page.open = Some("s1".into());
    app.schedule_page.fires = vec![
        fire("f3", 3 * 3600, FireStatus::Succeeded, ""),
        fire(
            "f2",
            27 * 3600,
            FireStatus::Failed,
            "Anthropic is limiting requests. Wait a moment and retry.",
        ),
        fire(
            "f1",
            51 * 3600,
            FireStatus::Skipped,
            "The previous fire was still running.",
        ),
    ];
}

/// Thirty days of usage across three models (one with no price), three agents and every source.
fn demo_usage(app: &mut App) {
    use crate::usage::{Price, Row, Source, Totals, day_name, today};
    demo_models(app);
    let price = |input, output, cache_read| Price {
        input,
        output,
        cache_read: Some(cache_read),
        cache_write: None,
    };
    let sonnet = ModelRef::new("anthropic", "claude-sonnet-4-6");
    let mini = ModelRef::new("openai", "gpt-4o-mini");
    let local = ModelRef::new("llama-server:box", "qwen3-coder-30b");
    app.prices
        .catalog
        .insert(sonnet.clone(), price(3.0, 15.0, 0.3));
    app.prices
        .catalog
        .insert(mini.clone(), price(0.15, 0.6, 0.075));
    let mut rows = Vec::new();
    for back in 0..30u64 {
        let day = day_name(today() - chrono::Days::new(back));
        // A working week: quieter weekends, a busy stretch ten days ago.
        let load = match back % 7 {
            5 | 6 => 1,
            _ => 3,
        } + u64::from((8..13).contains(&back)) * 4;
        let mut add = |model: &ModelRef, agent: &str, name: &str, source, calls: u64| {
            let input = calls * 6_000;
            rows.push(Row {
                day: day.clone(),
                model: model.clone(),
                agent_id: agent.into(),
                agent_name: name.into(),
                source,
                totals: Totals {
                    calls,
                    errors: u64::from(calls > 12),
                    input,
                    output: calls * 450,
                    cache_read: input * 3 / 5,
                    cache_write: input / 20,
                    first_ms_total: calls * 700,
                    first_ms_count: calls,
                },
            });
        };
        add(&sonnet, "atlas", "Atlas", Source::Chat, load * 4);
        add(
            &sonnet,
            "fact-checker",
            "Fact checker",
            Source::Member,
            load,
        );
        add(&mini, "scout", "Scout", Source::Schedule, 6);
        if back % 3 == 0 {
            add(&mini, "scout", "Scout", Source::Workflow, 2);
            add(&local, "forge", "Forge", Source::Chat, load * 2);
        }
    }
    rows.sort_by(|a, b| a.day.cmp(&b.day));
    app.usage.rows = rows;
}

type Scene = (&'static str, fn(&mut App));

/// The picture in [`RICH_REPLY`], a line drawn as an SVG `data:` URL.
const TREND: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='240' height='60'%3E%3Cpolyline fill='none' stroke='%2333aaff' stroke-width='3' points='0,50 60,40 120,44 180,20 240,10'/%3E%3C/svg%3E";

/// A reply that uses what the chat draws beyond plain text.
const RICH_REPLY: &str = r##"Churn last week, by plan. The rate is $r = \frac{c}{n}$, and over $k$ weeks it compounds:

$$
R_k = 1 - (1 - r)^k \approx k \cdot r \quad \text{for small } r
$$

The plans cost $5 and $10 a seat.

| Plan | Orgs | Churned | Rate |
|:-----|-----:|--------:|-----:|
| Free | 812 | 41 | 5.0% |
| Team | 361 | 9 | 2.5% |
| Business | 111 | 1 | 0.9% |

```mermaid
pie title Churned orgs by plan
    "Free" : 41
    "Team" : 9
    "Business" : 1
```

```mermaid
flowchart LR
    A[Org] --> B{Run in 30 days?}
    B -->|yes| C[Active]
    B -->|no| D[Churned]
```

![trend](<{trend}>)

A web picture waits for you:

![Weekly churn chart](https://example.com/chart.png)

The definition is on the [wiki](https://example.com/wiki/churn)."##;

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
    // An update installed, waiting for a restart.
    ("settings-updates", |s| {
        if let Ok(v) = "0.2.0".parse() {
            s.updates.status = crate::update::Status::Ready(v);
        }
        s.route = Route::Settings;
    }),
    // Telegram connected, one person asking to pair and one paired; then before connecting, a
    // token that did not work.
    ("channels", |s| {
        load_samples(s);
        let t = &mut s.channels.telegram;
        t.bot = Some("oido_home_bot".into());
        t.agents = vec!["atlas".into()];
        t.enabled = true;
        t.status = crate::channels::Status::Listening;
        t.pending.push(crate::channels::Pairing {
            user: "7".into(),
            name: "Grace Hopper".into(),
            handle: Some("grace".into()),
            code: "482913".into(),
            expires: i64::MAX,
        });
        t.users.push(crate::channels::Paired {
            id: "42".into(),
            name: "Ada Lovelace".into(),
            handle: Some("ada".into()),
            agent: Some("atlas".into()),
            trusted: false,
        });
        s.route = Route::Channels;
    }),
    ("channels-connect", |s| {
        s.channels.telegram_token = rune::TextEdit::new("123456:AAH-abcdefghijklmnopqrstuvw");
        s.channels.telegram.status = crate::channels::Status::Failed(
            "Could not connect: Telegram does not accept this token.".into(),
        );
        s.route = Route::Channels;
    }),
    // A WhatsApp number being linked: the code to scan.
    ("channels-whatsapp-link", |s| {
        s.channels.whatsapp.status = crate::channels::Status::Linking(
            "2@3Fh8sAe0x1wE5vQk9yZb7nP2dLmR4tUjc,0cE1b2A3d4F5g6H7i8J9k0L1m2N3o4P5q6R7s8T9u0=,AbCdEfGhIjKlMnOpQrStUvWxYz0123456789+/=".into(),
        );
        s.route = Route::Channels;
    }),
    // A WhatsApp number linked, an invite on show and Ada paired.
    ("channels-whatsapp", |s| {
        load_samples(s);
        let w = &mut s.channels.whatsapp;
        w.bot = Some("+35799123456".into());
        w.agents = vec!["atlas".into()];
        w.enabled = true;
        w.status = crate::channels::Status::Listening;
        w.invite = Some(crate::channels::Invite {
            token: "3f9a0c7e21b4d58a6c0f".into(),
            expires: crate::app::now() + 9 * 60 + 30,
        });
        w.users.push(crate::channels::Paired {
            id: "123456789@lid".into(),
            name: "Ada Lovelace".into(),
            handle: None,
            agent: Some("atlas".into()),
            trusted: false,
        });
        s.route = Route::Channels;
    }),
    // A chat Ada carries on from Telegram, open beside the agent's own.
    ("chat-telegram", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        if let Some(a) = s.roster.current_mut() {
            a.sessions.push(chat(
                "Telegram · Ada",
                vec![
                    Message::User("What changed in the release notes?".into()),
                    Message::reply(
                        "Two things need action: the API key rotation and the new rate limits.",
                        None,
                        Outcome::Done,
                    ),
                ],
            ));
            a.active = a.sessions.len() - 1;
            let sid = a.chat().id.clone();
            s.channels.telegram.chats.push(crate::channels::Binding {
                user: "42".into(),
                agent: "scout".into(),
                session: sid,
            });
        }
    }),
    // The Guardrails screen, with risky actions switched off.
    ("guardrails", |s| {
        s.guard.set(crate::guard::Pack::Actions, false);
        s.guard
            .set_words(vec!["Project Falcon".into(), "Acme".into()]);
        s.guard_words = TextEdit::new("Project Falcon, Acme");
        s.guard.set_limits(crate::guard::Limits {
            length: 2000,
            per_hour: 30,
        });
        s.route = Route::Guardrails;
    }),
    // A reply whose command waits for your go-ahead, after one a guardrail refused.
    ("chat-guard", |s| {
        use crate::guard::{Trip, TripState};
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        let call = |id: &str, command: &str, trip: Trip, output: Option<&str>| ToolCall {
            name: crate::tools::SHELL.into(),
            summary: command.into(),
            output: output.map(str::to_owned),
            id: id.into(),
            args: serde_json::json!({"command": command, "reason": "Clear space"}).to_string(),
            trip: Some(trip),
            outside: None,
        };
        let refused = Trip {
            rule: "shell.disk".into(),
            why: "It would erase a disk.".into(),
            state: TripState::Blocked,
            asked_at: None,
        };
        let held = Trip {
            rule: "shell.delete".into(),
            why:
                "It deletes /home/ada/Downloads/old, outside the working folder /home/ada/project."
                    .into(),
            state: TripState::Asking,
            asked_at: Some(crate::app::now() - 90),
        };
        if let Some(a) = s.roster.current_mut() {
            let mut reply = Message::reply(
                "I'll clear the old downloads to free some space.",
                None,
                Outcome::Done,
            );
            if let Message::Assistant { tools, .. } = &mut reply {
                let told = refused.told();
                tools.push(call("c1", "mkfs.ext4 /dev/sdb1", refused, Some(&told)));
                tools.push(call("c2", "rm -rf ~/Downloads/old", held, None));
            }
            a.sessions.push(chat(
                "Free up space",
                vec![
                    Message::User("My disk is full, free some space.".into()),
                    reply,
                ],
            ));
            a.active = a.sessions.len() - 1;
            a.status = AgentStatus::Waiting;
            // A team member's command, waiting too: nobody watches its work.
            let sid = a.chat().id.clone();
            s.waiting.push(crate::guard::ask::Waiting::demo(
                crate::guard::ask::Where::Chat(sid),
                "cleaner",
                crate::tools::SHELL,
                "docker system prune -af",
                Trip {
                    rule: "taint.outside".into(),
                    why: "What it was reading holds a web page, which could be steering it. Check this is what you want.".into(),
                    state: TripState::Asking,
                    asked_at: Some(crate::app::now()),
                },
            ));
        }
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
    ("schedules", |s| {
        load_samples(s);
        demo_schedules(s);
        s.route = Route::Schedules;
    }),
    ("schedule-form", |s| {
        load_samples(s);
        demo_schedules(s);
        if let Some(x) = s.schedules.first().cloned() {
            let source = crate::schedules::form::Source::Edit(x.id.clone());
            s.schedule_page.form = Some(crate::schedules::form::Form::of(
                &x,
                source,
                &s.workflows,
                &chrono::Local,
            ));
        }
        s.route = Route::Schedules;
    }),
    ("schedule-draft", |s| {
        use crate::schedule::{Proposal, Schedule, Target as Does, Timing};
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        let timing = Timing::every(app::now(), 30 * 60);
        let draft = timing.and_then(|t| {
            Schedule::new(
                "scout",
                "Inbox sweep",
                Does::Prompt("Sort new mail and flag anything urgent.".into()),
                t,
                app::now(),
                &chrono::Local,
            )
        });
        if let (Some(a), Ok(x)) = (s.roster.current_mut(), draft) {
            let sid = a.chat().id.clone();
            a.chat_mut().messages = vec![
                Message::User("Check my inbox every half hour and flag anything urgent.".into()),
                Message::Assistant {
                    text: "I've drafted a schedule for that. It starts once you approve it.".into(),
                    thinking: String::new(),
                    thought_secs: None,
                    tools: vec![ToolCall {
                        name: "schedule".into(),
                        summary: "create: Inbox sweep".into(),
                        output: Some("The person now sees a draft.".into()),
                        id: "c1".into(),
                        args: "{}".into(),
                        trip: None,
                        outside: None,
                    }],
                    approval: None,
                    stats: None,
                    run: None,
                    outcome: Outcome::Done,
                },
            ];
            s.proposals = vec![Proposal {
                call_id: "c1".into(),
                session_id: sid,
                updates: false,
                schedule: x,
            }];
        }
    }),
    ("usage", |s| {
        demo_usage(s);
        s.route = Route::Usage;
    }),
    // A model with no price, its price form open and a mistake in it.
    ("usage-price", |s| {
        demo_usage(s);
        s.route = Route::Usage;
        s.usage.filters.agent = Some("forge".into());
        s.toggle_price_form(&ModelRef::new("llama-server:box", "qwen3-coder-30b"));
        let f = s.usage.price_mut();
        f.input = TextEdit::new("0.2");
        f.output = TextEdit::new("abc");
        s.save_price();
    }),
    // Cost where it also shows: Atlas's chat header, its form, and Settings asking before clearing.
    ("usage-chat", |s| {
        load_samples(s);
        demo_usage(s);
        s.roster.select(Target::agent("atlas"));
        if let Some(sid) = s.roster.current().map(|a| a.chat().id.clone()) {
            let t = crate::usage::Totals {
                input: 48_000,
                output: 3_100,
                cache_read: 30_000,
                ..crate::usage::Totals::default()
            };
            s.chat_usage.insert(
                sid,
                vec![(ModelRef::new("anthropic", "claude-sonnet-4-6"), t)],
            );
        }
    }),
    ("usage-agent", |s| {
        load_samples(s);
        demo_usage(s);
        s.edit_agent(&Target::agent("atlas"));
        s.agent_spend = s
            .usage
            .rows
            .iter()
            .filter(|r| r.agent_id == "atlas")
            .cloned()
            .collect();
    }),
    ("settings-usage", |s| {
        s.usage.confirm_clear = true;
        s.route = Route::Settings;
    }),
    ("runs", |s| {
        demo_runs(s);
        s.route = Route::Runs;
    }),
    ("context", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        let model = s
            .roster
            .current()
            .and_then(|a| match s.providers.status(a.model.as_ref()) {
                crate::providers::ModelStatus::Ready(m) => Some(m),
                _ => None,
            });
        if let Some(m) = model {
            s.limits.catalog.insert(m, 200_000);
        }
        let mut answered = Message::reply(
            "The three changes that matter: the new sync engine, the dropped Python 3.9 support, and the config file moving to `~/.config/app`.",
            Some("r1".into()),
            Outcome::Done,
        );
        if let Message::Assistant { stats, .. } = &mut answered {
            *stats = Some(Stats {
                model: "GPT-5".into(),
                tokens: 412,
                seconds: 4.8,
                context: Some(171_300),
                trimmed: Some(22_000),
            });
        }
        if let Some(a) = s.roster.current_mut() {
            let sid = a.chat().id.clone();
            a.chat_mut().messages = vec![
                Message::User("Read every release note since 2.0.".into()),
                Message::reply("Done: 41 notes read.", Some("r0".into()), Outcome::Done),
                Message::Summary {
                    text: "You asked for the release notes since 2.0. All 41 were read with the shell tool from `docs/releases/`.\n\n- **Decided:** report only changes that need action.\n- **Open:** a one-page summary for the team.".into(),
                    keeps: 0,
                    outside: None,
                },
                Message::User("Which ones need action?".into()),
                answered,
            ];
            s.summaries_open.insert((sid, 2));
        }
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
                    thinking: String::new(),
                    thought_secs: None,
                    tools: vec![ToolCall {
                        name: "ask_user_question".into(),
                        summary: "Who is it for?".into(),
                        output: None,
                        id: "c1".into(),
                        args: r#"{"question":"Who is the announcement for?","options":["Customers","Investors","The team"],"multiple":true}"#.into(),
                        trip: None,
                        outside: None,
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
                trip: None,
                outside: None,
            };
            let reply = |tools| Message::Assistant {
                text: "Let me look.".into(),
                thinking: String::new(),
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
                    thinking: String::new(),
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
    // A reply with a table, a diagram, a chart, a picture and a link, its thinking opened.
    ("reply-rich", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        if let Some(a) = s.roster.current_mut() {
            let mut reply = Message::reply(
                RICH_REPLY.replace("{trend}", TREND),
                Some("r1".into()),
                Outcome::Done,
            );
            if let Message::Assistant {
                thinking,
                thought_secs,
                ..
            } = &mut reply
            {
                *thinking = "Churn by plan is in the export. A **table** for the numbers, a pie for the split, and a flowchart of how a run is counted.".into();
                *thought_secs = Some(2.8);
            }
            let sid = a.chat().id.clone();
            a.chat_mut().messages = vec![Message::User("Show me churn by plan.".into()), reply];
            s.thinking_open.insert((sid.clone(), 1), true);
            // Decoded before the picture is taken, as the window would redraw once it is.
            let folder = s.workdir_of(&sid);
            crate::ui::wait_for_reply_picture(TREND, &folder);
            crate::ui::wait_for_reply_picture("https://example.com/chart.png", &folder);
        }
    }),
    // The picture in that reply, pressed: opened large over the chat.
    ("picture-open", |s| {
        (SCENES
            .iter()
            .find(|(name, _)| *name == "reply-rich")
            .map(|(_, f)| *f)
            .unwrap_or(|_| {}))(s);
        s.viewing = Some((TREND.into(), "trend".into()));
    }),
    // Thinking out loud, before the first word of the reply.
    ("reply-thinking", |s| {
        load_samples(s);
        s.roster.select(Target::agent("scout"));
        if let Some(a) = s.roster.current_mut() {
            let mut reply = Message::reply("", Some("r1".into()), Outcome::Working);
            if let Message::Assistant { thinking, .. } = &mut reply {
                *thinking = "The notes are in `docs/releases/`. Read each one, keep only the changes that need action, then group them by area.".into();
            }
            a.chat_mut().messages =
                vec![Message::User("Summarise the release notes.".into()), reply];
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
            trip: None,
            outside: None,
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
                thinking: String::new(),
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
                    thinking: String::new(),
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
    // The composer's folder pill with its browser open.
    ("composer-folder", |s| {
        load_samples(s);
        demo_models(s);
        s.set_model(Some(ModelRef::new("anthropic", "claude-opus-4-7")));
        s.set_workdir(Some("/home".into()));
        s.workdir_browser.show(Some(std::path::Path::new("/")));
    }),
    // The composer's `/` list, as it opens on a slash.
    ("composer-commands", |s| {
        load_samples(s);
        demo_models(s);
        s.set_model(Some(ModelRef::new("anthropic", "claude-opus-4-7")));
        s.composer.edit = rune::TextEdit::new("/");
        s.composer.edit.set_caret(1);
        s.composer.edit.focused = true;
    }),
    // A draft with a mentioned file, a text file and a picture attached, and a message waiting
    // for the reply being written.
    ("composer-attached", |s| {
        use crate::prompt::{Attachment, Image};
        load_samples(s);
        demo_models(s);
        s.set_model(Some(ModelRef::new("anthropic", "claude-opus-4-7")));
        let edit = &mut s.composer.edit;
        *edit = rune::TextEdit::new("Compare ");
        edit.set_caret(8);
        edit.insert_pill("src/main.rs", "@src/main.rs");
        edit.insert(" with the screenshot and the notes.");
        edit.focused = true;
        s.composer.attachments.push(Attachment::File {
            name: "notes.md".into(),
            text: "# Notes\n- one\n- two\n".into(),
        });
        // A picture with something in it, so the thumbnail shows it was drawn.
        let picture = |name: &str, w: u32, h: u32| {
            let img = image::RgbImage::from_fn(w, h, |x, y| {
                image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, 180])
            });
            let mut png = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png, image::ImageFormat::Png)
                .ok()
                .and_then(|_| Image::fit(name, png.get_ref()).ok())
        };
        let shot = picture("Screenshot.png", 1280, 800);
        if let Some(img) = &shot {
            s.composer.attachments.push(Attachment::Image(img.clone()));
        }
        let sid = s.roster.current_mut().map(|a| {
            let sent = crate::prompt::Prompt {
                text: "What changed on this page?".into(),
                attachments: shot.iter().cloned().map(Attachment::Image).collect(),
            };
            a.chat_mut().messages = vec![
                Message::User(sent),
                Message::reply("", Some("r1".into()), Outcome::Working),
            ];
            a.chat().id.clone()
        });
        if let Some(img) = &shot {
            crate::ui::wait_for_pictures(&[img]);
        }
        s.scrolled_up = true;
        if let Some(sid) = sid {
            s.queued
                .insert(sid.clone(), "Then draft the announcement.".into());
            s.live.insert(sid, crate::run::Live::demo());
        }
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
    let run = |id: &str, name: &str, title: &str, status, ago: i64| {
        let mut r = Run::new(
            id,
            "a",
            name,
            "s",
            ModelRef::new("anthropic", "claude-sonnet-4-5"),
        );
        r.title = title.into();
        r.status = status;
        r.error = (status == Status::Error).then(|| "Anthropic is limiting requests.".to_owned());
        (r.input_tokens, r.output_tokens) = (Some(3120), Some(412));
        (r.duration_ms, r.calls, r.tools) = (Some(9400), 2, 2);
        r.started_at = app::now() - ago;
        r
    };
    app.runs = vec![
        run(
            "r1",
            "Atlas",
            "What changed in the release notes?",
            Status::Ok,
            120,
        ),
        run("r2", "Forge", "Fix the failing build", Status::Error, 4000),
        run(
            "r3",
            "Atlas",
            "Draft the announcement",
            Status::Cancelled,
            90000,
        ),
    ];
    let mut seq = 0;
    let mut step = |kind, name: &str, out: &str| {
        seq += 1;
        let mut s = Step::new(seq, kind, 0);
        s.name = name.into();
        s.output = Some(out.to_owned());
        s
    };
    let timed = |mut s: Step, at, ms| {
        (s.at_ms, s.duration_ms) = (Some(at), Some(ms));
        s
    };
    let mut first = step(
        StepKind::Llm,
        "",
        "I'll read the notes first.\n\n[tool call] read_file {\"path\":\"NOTES.md\"}",
    );
    first.input = Some("[user]\nWhat changed in the release notes?".into());
    (first.input_tokens, first.output_tokens, first.first_ms) = (Some(1480), Some(96), Some(620));
    first.reasoning = Some("The user wants a summary. The notes are in NOTES.md.".into());
    let mut read = step(
        StepKind::Tool,
        "read_file",
        "## 0.4.0\n- Runs record every tool\n- Skills",
    );
    read.input = Some(r#"{"path":"NOTES.md"}"#.into());
    let mut grep = step(
        StepKind::Tool,
        "shell",
        "Error: grep: CHANGELOG: No such file",
    );
    grep.input = Some(r#"{"command":"grep -n fix CHANGELOG","reason":"Find fixes"}"#.into());
    grep.status = Status::Error;
    let mut last = step(
        StepKind::Llm,
        "",
        "Three things changed:\n\n1. **Runs** record every model call and tool.\n2. Skills show in the setup.\n3. Errors are marked.",
    );
    last.input = Some("[user]\nWhat changed?\n\n[tool result]\n## 0.4.0 …".into());
    (last.input_tokens, last.output_tokens, last.first_ms) = (Some(1640), Some(316), Some(540));
    let brief = step(
        StepKind::System,
        "",
        "You plan work. Be brief, and say what you assumed.",
    );
    let skill = step(
        StepKind::Skill,
        "release-notes",
        "# Release notes\nRead NOTES.md, group changes by area, lead with what users notice.",
    );
    let context = step(
        StepKind::Context,
        "",
        "## Context\nIt is Tuesday, 10 November 2026, 14:02 (UTC+02:00).",
    );
    let tools = step(
        StepKind::Tools,
        "2",
        "read_file(path, lines?)\nReads a text file in the working folder.\n\nshell(command, reason)\nRuns a command.",
    );
    let user = step(StepKind::User, "", "What changed in the release notes?");
    app.steps = vec![
        brief,
        skill,
        context,
        tools,
        timed(user, 0, 0),
        timed(first, 10, 3100),
        timed(read, 3120, 40),
        timed(grep, 3120, 880),
        timed(last, 4020, 5380),
    ];
    app.record = 5;
}
