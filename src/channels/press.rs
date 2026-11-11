//! Buttons under the bot's messages that act: answering the agent's question, and approving or
//! rejecting what it drafted (an agent, a schedule, a workflow). Each button belongs to the person
//! it was sent to and works once: pressing one takes its message's buttons away. They live while
//! the app runs; after a restart a press is turned away, and a question can still be answered by
//! writing. MCP server drafts are not offered here: they wait in the app, as a local server runs a
//! program on this computer.

use chrono::Local;

use super::{Button, Channel, Kind, Out, Place};
use crate::app::App;
use crate::model::Message;
use crate::tools::Question;

/// The longest label a button shows, in characters.
const LABEL: usize = 60;
/// How much of a drafted agent's instructions is shown.
const BRIEF: usize = 400;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Press {
    user: String,
    /// The buttons sent together: one pressed, all go.
    group: u64,
    action: Action,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Action {
    Answer {
        sid: String,
        call: String,
        text: String,
    },
    AgentDraft {
        call: String,
        approve: bool,
    },
    Schedule {
        call: String,
        approve: bool,
    },
    Workflow {
        sid: String,
        save: bool,
    },
}

impl Channel {
    /// Buttons for `user`, one per choice, that work once between them.
    fn buttons(&mut self, user: &str, choices: Vec<(String, Action)>) -> Vec<Button> {
        self.next_group += 1;
        let group = self.next_group;
        choices
            .into_iter()
            .enumerate()
            .map(|(i, (label, action))| {
                let data = format!("b:{group}:{i}");
                self.presses.insert(
                    data.clone(),
                    Press {
                        user: user.to_owned(),
                        group,
                        action,
                    },
                );
                Button {
                    label: clip(&label, LABEL),
                    data,
                }
            })
            .collect()
    }

    /// Forgets the buttons that answer chat `sid`'s question: it was answered another way.
    pub(super) fn forget_answers(&mut self, sid: &str) {
        self.presses
            .retain(|_, p| !matches!(&p.action, Action::Answer { sid: s, .. } if s == sid));
    }

    /// The action of the button `data`, if it is `user`'s and still works; it and the buttons sent
    /// with it stop working.
    fn take(&mut self, user: &str, data: &str) -> Option<Action> {
        let p = self.presses.get(data).filter(|p| p.user == user)?;
        let (group, action) = (p.group, p.action.clone());
        self.presses.retain(|_, p| p.group != group);
        Some(action)
    }
}

/// `text` cut to `max` characters, with an ellipsis if it was cut.
fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}

/// The options picked by number ("2", "1, 3"), as the app words picks; `None` when `text` is not
/// numbers of options, or names several for a question that takes one.
pub(super) fn picked(q: &Question, text: &str) -> Option<String> {
    let numbers: Vec<&str> = text
        .split([',', ';', ' '])
        .filter(|w| !w.is_empty() && *w != "and")
        .collect();
    if numbers.is_empty() || (!q.multiple && numbers.len() > 1) {
        return None;
    }
    let picks = numbers
        .iter()
        .map(|n| {
            n.parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| q.options.get(i).cloned())
        })
        .collect::<Option<Vec<String>>>()?;
    Some(crate::tools::answer(&picks, ""))
}

impl App {
    /// A press on one of the buttons [`Channel::buttons`] made. Returns a short note for the
    /// press's spinner.
    pub(super) fn press_button(
        &mut self,
        kind: Kind,
        user: &str,
        data: &str,
        message: Option<(Place, String)>,
    ) -> Option<String> {
        let c = self.channels.get_mut(kind);
        let Some(action) = c.take(user, data) else {
            return Some("This button no longer works.".to_owned());
        };
        if let Some((to, message)) = message {
            c.send(Out::Unbutton { to, message });
        }
        match action {
            Action::Answer { sid, call, text } => {
                let waiting = self.pending_call_in(&sid, crate::tools::ASK);
                if self.busy_in(&sid) || waiting.as_deref() != Some(call.as_str()) {
                    return Some("That question was already answered.".to_owned());
                }
                self.channels.get_mut(kind).forget_answers(&sid);
                self.resolve(&sid, &call, text.clone(), true);
                Some(clip(&text, 180))
            }
            Action::AgentDraft { call, approve } => {
                self.press_agent_draft(kind, user, &call, approve);
                None
            }
            Action::Schedule { call, approve } => {
                self.press_schedule(kind, user, &call, approve);
                None
            }
            Action::Workflow { sid, save } => {
                self.press_workflow(kind, user, &sid, save);
                None
            }
        }
    }

    fn press_agent_draft(&mut self, kind: Kind, user: &str, call: &str, approve: bool) {
        let Some(draft) = self.agent_drafts.iter().find(|d| d.call_id == call) else {
            self.channels.get_mut(kind).say(user, "That draft is gone.");
            return;
        };
        let (sid, title) = (draft.session_id.clone(), self.agent_draft_words(draft).0);
        if !approve {
            self.reject_agent_draft(call);
            self.channels
                .get_mut(kind)
                .say(user, &format!("Rejected: {title}."));
            return;
        }
        self.approve_agent_draft(call);
        match self.agent_draft_notice.clone() {
            Some((c, why)) if c == call => {
                self.channels
                    .get_mut(kind)
                    .say(user, &format!("⚠️ Not done: {why}"));
                self.offer_again(&sid, call);
            }
            _ => self
                .channels
                .get_mut(kind)
                .say(user, &format!("Done: {title}.")),
        }
    }

    fn press_schedule(&mut self, kind: Kind, user: &str, call: &str, approve: bool) {
        let Some(p) = self.proposals.iter().find(|p| p.call_id == call) else {
            self.channels.get_mut(kind).say(user, "That draft is gone.");
            return;
        };
        let (sid, name) = (p.session_id.clone(), p.schedule.name.clone());
        if !approve {
            self.reject_proposal(call);
            self.channels
                .get_mut(kind)
                .say(user, &format!("Rejected the schedule {name}."));
            return;
        }
        match self.approve_proposal(call) {
            Ok(()) => self.channels.get_mut(kind).say(
                user,
                &format!("Saved the schedule **{name}**. It runs on its own from now on."),
            ),
            Err(why) => {
                self.channels
                    .get_mut(kind)
                    .say(user, &format!("⚠️ Not saved: {why}"));
                self.offer_again(&sid, call);
            }
        }
    }

    fn press_workflow(&mut self, kind: Kind, user: &str, sid: &str, save: bool) {
        if !self.drafts.contains_key(sid) {
            self.channels.get_mut(kind).say(user, "That draft is gone.");
            return;
        }
        if !save {
            self.discard_draft_in(sid);
            self.channels
                .get_mut(kind)
                .say(user, "Discarded the workflow draft.");
            return;
        }
        if let Some(name) = self.save_draft_in(sid) {
            self.channels
                .get_mut(kind)
                .say(user, &format!("Saved the workflow **{name}**."));
        }
    }

    /// Offers draft `key` of chat `sid` again: approving it failed and it is still there.
    fn offer_again(&mut self, sid: &str, key: &str) {
        if let Some(kind) = self.channels.kind_of(sid) {
            self.channels.get_mut(kind).offered.remove(key);
            self.offer_drafts(sid);
        }
    }

    /// Sends the question the agent is waiting on, with a button per option when one is picked.
    pub(super) fn offer_question(
        &mut self,
        kind: Kind,
        user: &str,
        sid: &str,
        call: &str,
        q: &Question,
    ) {
        let mut said = format!("❓ {}", q.text);
        if !q.options.is_empty() {
            let options: Vec<String> = q
                .options
                .iter()
                .enumerate()
                .map(|(i, o)| format!("{}. {o}", i + 1))
                .collect();
            said.push_str(&format!("\n\n{}", options.join("\n")));
            said.push_str(if q.multiple {
                "\n\nReply with the numbers of your choices (like 1, 3), or write your own answer."
            } else {
                "\n\nTap one, or write your own answer."
            });
        }
        let buttons = if q.multiple {
            Vec::new()
        } else {
            let choices = q
                .options
                .iter()
                .map(|o| {
                    let action = Action::Answer {
                        sid: sid.to_owned(),
                        call: call.to_owned(),
                        text: o.clone(),
                    };
                    (o.clone(), action)
                })
                .collect();
            self.channels.get_mut(kind).buttons(user, choices)
        };
        self.channels
            .get_mut(kind)
            .say_with(&Place::private(user), &said, buttons);
    }

    /// Sends chat `sid`'s drafts not offered yet, each with its buttons, to the person who
    /// carries the chat on.
    pub(super) fn offer_drafts(&mut self, sid: &str) {
        let Some(kind) = self.channels.kind_of(sid) else {
            return;
        };
        let Some(user) = self.channels.get(kind).user_of(sid) else {
            return;
        };
        for (said, buttons) in self.draft_offers(kind, sid, &user) {
            self.channels
                .get_mut(kind)
                .say_with(&Place::private(&user), &said, buttons);
        }
    }

    /// Chat `sid`'s drafts not offered yet: what each says and its buttons, for `user`. They count
    /// as offered from now on.
    pub(super) fn draft_offers(
        &mut self,
        kind: Kind,
        sid: &str,
        user: &str,
    ) -> Vec<(String, Vec<Button>)> {
        let mut out = Vec::new();
        let agent_drafts: Vec<_> = self
            .agent_drafts
            .iter()
            .filter(|d| {
                d.session_id == sid && !self.channels.get(kind).offered.contains(&d.call_id)
            })
            .map(|d| (d.call_id.clone(), self.agent_draft_words(d)))
            .collect();
        for (call, (title, body, approve, reject)) in agent_drafts {
            let buttons = self.channels.get_mut(kind).buttons(
                user,
                vec![
                    (
                        approve.to_owned(),
                        Action::AgentDraft {
                            call: call.clone(),
                            approve: true,
                        },
                    ),
                    (
                        reject.to_owned(),
                        Action::AgentDraft {
                            call: call.clone(),
                            approve: false,
                        },
                    ),
                ],
            );
            self.channels.get_mut(kind).offered.insert(call);
            out.push((format!("**{title}**\n\n{body}"), buttons));
        }

        let schedules: Vec<_> = self
            .proposals
            .iter()
            .filter(|p| {
                p.session_id == sid && !self.channels.get(kind).offered.contains(&p.call_id)
            })
            .map(|p| {
                let x = &p.schedule;
                let title = if p.updates {
                    format!("Change to schedule: {}", x.name)
                } else {
                    format!("Schedule draft: {}", x.name)
                };
                let mut lines = vec![
                    crate::ui::schedules::target_words(self, &x.target),
                    format!("When: {}", x.timing.describe(&Local)),
                ];
                let next = self.upcoming(x, 3);
                if !next.is_empty() {
                    lines.push(format!("Next: {}", next.join(" · ")));
                }
                lines.push(
                    "It runs on its own, with this agent's tools, once you approve it.".into(),
                );
                (
                    p.call_id.clone(),
                    format!("**{title}**\n\n{}", lines.join("\n\n")),
                )
            })
            .collect();
        for (call, said) in schedules {
            let buttons = self.channels.get_mut(kind).buttons(
                user,
                vec![
                    (
                        "Approve".to_owned(),
                        Action::Schedule {
                            call: call.clone(),
                            approve: true,
                        },
                    ),
                    (
                        "Reject".to_owned(),
                        Action::Schedule {
                            call: call.clone(),
                            approve: false,
                        },
                    ),
                ],
            );
            self.channels.get_mut(kind).offered.insert(call);
            out.push((said, buttons));
        }

        let key = workflow_key(sid);
        if !self.channels.get(kind).offered.contains(&key)
            && let Some(d) = self.drafts.get(sid)
        {
            let steps = d.definition.steps.len();
            let said = format!(
                "**Workflow draft: {}**\n\n{}\n\n{steps} step{}. Saved, it can be run again without planning.",
                d.name,
                d.goal,
                if steps == 1 { "" } else { "s" }
            );
            let buttons = self.channels.get_mut(kind).buttons(
                user,
                vec![
                    (
                        "Save".to_owned(),
                        Action::Workflow {
                            sid: sid.to_owned(),
                            save: true,
                        },
                    ),
                    (
                        "Discard".to_owned(),
                        Action::Workflow {
                            sid: sid.to_owned(),
                            save: false,
                        },
                    ),
                ],
            );
            self.channels.get_mut(kind).offered.insert(key);
            out.push((said, buttons));
        }

        let mcp: Vec<String> = self
            .mcp
            .drafts
            .iter()
            .filter(|d| {
                d.session_id == sid && !self.channels.get(kind).offered.contains(&d.call_id)
            })
            .map(|d| d.call_id.clone())
            .collect();
        if !mcp.is_empty() {
            out.push((
                "An MCP server change waits for your approval in Oido: those are approved there, as they can run programs on that computer.".to_owned(),
                Vec::new(),
            ));
            self.channels.get_mut(kind).offered.extend(mcp);
        }
        out
    }

    /// A drafted agent change in words: its title, what it says, and its buttons' labels.
    fn agent_draft_words(
        &self,
        d: &crate::agent_tool::Draft,
    ) -> (String, String, &'static str, &'static str) {
        use crate::agent_tool::Proposal;
        let parts = |a: &crate::agents::Agent| {
            let list = |l: &[String], none: &str| {
                if l.is_empty() {
                    none.to_owned()
                } else {
                    l.join(", ")
                }
            };
            let model = a.model.as_ref().map_or_else(
                || "the default".to_owned(),
                |m| self.providers.model_name(m),
            );
            let mut lines = vec![
                format!("Model: {model}"),
                format!("Tools: {}", list(&a.tools, "the built-in ones")),
                format!("Skills: {}", list(&a.skills, "none")),
                format!("Folder: {}", a.workdir.as_deref().unwrap_or("home")),
            ];
            if !a.prompt.trim().is_empty() {
                lines.push(clip(&a.prompt, BRIEF));
            }
            lines.join("\n")
        };
        match &d.proposal {
            Proposal::Create { agent, lead: None } => (
                format!("New agent: {}", agent.name),
                parts(agent),
                "Create",
                "Reject",
            ),
            Proposal::Create {
                agent,
                lead: Some(lead),
            } => {
                let lead = self.roster.agent_ref(lead).map_or("?", |a| a.name.as_str());
                (
                    format!("New team member of {lead}: {}", agent.name),
                    parts(agent),
                    "Add to team",
                    "Reject",
                )
            }
            Proposal::Update { agent, changed, .. } => (
                format!("Change agent: {}", agent.name),
                format!("Changes: {}\n\n{}", changed.join(", "), parts(agent)),
                "Save changes",
                "Reject",
            ),
            Proposal::Delete { target, name } => {
                let team = self.roster.find(target).map_or(0, |a| a.subagents.len());
                let also = match team {
                    0 => String::new(),
                    1 => ", and its team member".to_owned(),
                    n => format!(", and its {n} team members"),
                };
                (
                    format!("Delete agent: {name}"),
                    format!("Deletes {name} with all its chats{also}. This cannot be undone."),
                    "Delete",
                    "Keep it",
                )
            }
        }
    }

    /// Chat `sid`'s workflow draft is ready, or could not be learned: the person who carries the
    /// chat on is offered it, or told why.
    pub(crate) fn channel_workflow_ready(&mut self, sid: &str, failed: Option<&str>) {
        let Some(kind) = self.channels.kind_of(sid) else {
            return;
        };
        let Some(user) = self.channels.get(kind).user_of(sid) else {
            return;
        };
        match failed {
            Some(why) => self.channels.get_mut(kind).say(
                &user,
                &format!("⚠️ The workflow could not be learned: {why}"),
            ),
            None => self.offer_again(sid, &workflow_key(sid)),
        }
    }

    /// The question the agent waits on in chat `sid` under call `call`.
    pub(super) fn question_of(&self, sid: &str, call: &str) -> Option<Question> {
        let chat = self
            .roster
            .owner_of(sid)?
            .sessions
            .iter()
            .find(|s| s.id == sid)?;
        match chat.messages.last()? {
            Message::Assistant { tools, .. } => tools
                .iter()
                .find(|t| t.id == call)
                .map(|t| crate::tools::question(&t.args)),
            _ => None,
        }
    }
}

fn workflow_key(sid: &str) -> String {
    format!("workflow:{sid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(multiple: bool) -> Question {
        Question {
            text: "Which?".into(),
            options: vec!["Red".into(), "Blue".into(), "Green".into()],
            multiple,
        }
    }

    #[test]
    fn numbers_written_back_pick_options() {
        assert_eq!(picked(&q(false), "2").as_deref(), Some("Blue"));
        assert_eq!(picked(&q(true), "1, 3").as_deref(), Some("Red; Green"));
        assert_eq!(picked(&q(true), "1 and 2").as_deref(), Some("Red; Blue"));
        assert_eq!(picked(&q(false), "1, 3"), None, "one only");
        assert_eq!(picked(&q(false), "4"), None);
        assert_eq!(picked(&q(false), "0"), None);
        assert_eq!(picked(&q(false), "blue please"), None);
    }

    #[test]
    fn labels_are_cut_to_fit() {
        assert_eq!(clip(" short ", 10), "short");
        assert_eq!(clip("abcdefghijkl", 10), "abcdefghij…");
    }
}
