//! First-run setup: once a provider is connected, a built-in Setup agent interviews the person in
//! the ordinary chat. It is a real agent, so its model, effort and usage work as everyone's do.

use serde_json::json;

use crate::agents::{Agent, Target};
use crate::app::{App, Route};
use crate::completion::ToolSpec;
use crate::guard::Pack;
use crate::model::Message;
use crate::models::ModelRef;

const SETUP_ID: &str = "setup";

/// The Setup agent's own tool: it turns on every guardrail pack.
pub const TIGHTEN: &str = "tighten_guardrails";

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: TIGHTEN.into(),
        description: "Turn on every guardrail: hiding secrets and personal details from models, holding risky actions, asking before acting on outside text, catching hidden instructions, and checking replies to channels. It never turns anything off. Use it only when the person asks for extra care.".into(),
        schema: json!({"type": "object", "properties": {}}),
    }
}

/// Whether `agent` is the Setup agent, the only one given [`spec`].
pub fn is_setup(agent: &Agent) -> bool {
    agent.id == SETUP_ID
}

const SETUP_PROMPT: &str = "You are Oido's setup guide. You help a new person get started, in plain, friendly language with no jargon. Keep every message short.\n\n\
Work through these in order, one question at a time, using `ask_user_question` whenever the answer is a choice:\n\
1. What they do, and what they hope an assistant can take off their plate.\n\
2. How careful you should be. Oido already hides secrets and holds risky actions for a go-ahead. If they want extra care, call `tighten_guardrails`, which turns every protection on. Never offer to switch protections off: they can do that themselves on the Guardrails screen.\n\
3. Suggest one to three agents that fit what they told you, each with a clear name and instructions. Make each with `agents` (action `create`); the person sees a draft and approves it, so say that, and do not claim it exists until they have. Give agents only the app's own tools unless they ask for more.\n\
4. Finish by saying what you set up and that they can find agents in the sidebar and change anything later in Settings and Guardrails.\n\n\
Never ask for passwords or API keys. If they want to connect an account such as Gmail or Telegram, tell them to open Extensions or Channels, since you cannot do that for them.";

const SETUP_OPENER: &str = "Hi! I just installed Oido and want to get set up.";

impl App {
    /// Called when a provider has just been saved. While the first-run screen is up, this ends it:
    /// a model is chosen if none is, and the chat opens on the Setup agent, who speaks first.
    pub(crate) fn provider_connected(&mut self) {
        if self.route != Route::Onboarding {
            return;
        }
        if self.providers.default_model.is_none()
            && let Some(model) = self.first_chat_model()
        {
            self.set_default_model(Some(model));
        }
        let target = self.setup_agent();
        let Some(sid) = self.roster.find(&target).map(|a| a.chat().id.clone()) else {
            return;
        };
        self.roster.select(target);
        self.save_selection();
        self.route = Route::Chat;
        self.post(&sid, SETUP_OPENER.into());
    }

    /// Answers the chat's waiting `tighten_guardrails` calls. Only the Setup agent is given the
    /// tool, so a call from any other agent is refused.
    pub(crate) fn run_setup_calls(&mut self, sid: &str) {
        let Some(owner) = self.roster.owner_of(sid) else {
            return;
        };
        let allowed = is_setup(owner);
        let waiting: Vec<String> = match owner
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .and_then(|s| s.messages.last())
        {
            Some(Message::Assistant { tools, .. }) => tools
                .iter()
                .filter(|t| t.name == TIGHTEN && t.output.is_none())
                .map(|t| t.id.clone())
                .collect(),
            _ => return,
        };
        for id in waiting {
            let told = if allowed {
                self.tighten_guardrails()
            } else {
                "Error: this tool is not available to you.".to_owned()
            };
            self.resolve(sid, &id, told, true);
        }
    }

    /// Turns every pack on, and says which were off.
    fn tighten_guardrails(&mut self) -> String {
        let turned: Vec<&str> = Pack::ALL
            .into_iter()
            .filter(|p| !self.guard.on(*p))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|p| {
                self.set_pack(p, true);
                p.title()
            })
            .collect();
        if turned.is_empty() {
            "Every guardrail was already on.".to_owned()
        } else {
            format!("Turned on: {}.", turned.join("; "))
        }
    }

    /// The first model of the first connected provider that is not known to lack tool use.
    fn first_chat_model(&self) -> Option<ModelRef> {
        self.providers.groups().into_iter().find_map(|g| {
            g.models
                .iter()
                .find(|m| m.caps.tools != Some(false))
                .map(|m| ModelRef::new(&g.provider, &m.id))
        })
    }

    /// The Setup agent, made the first time it is wanted and kept after, so setup can be run again.
    fn setup_agent(&mut self) -> Target {
        let target = Target::agent(SETUP_ID);
        if self.roster.find(&target).is_none() {
            let mut a = Agent::blank(SETUP_ID.to_owned());
            a.name = "Setup".to_owned();
            a.prompt = SETUP_PROMPT.to_owned();
            a.hue = Some(200.0);
            self.roster.list.push(a);
            self.save_agent_tree(&target);
        }
        target
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::Message;

    fn connected() -> App {
        let mut app = App::new(None);
        crate::testing::ready(&mut app);
        app.providers.default_model = None;
        app.route = Route::Onboarding;
        app
    }

    #[test]
    fn connecting_opens_the_setup_agents_chat_with_a_model_chosen() {
        let mut app = connected();
        app.provider_connected();
        assert_eq!(app.route, Route::Chat);
        assert_eq!(
            app.providers.default_model,
            Some(ModelRef::new("anthropic", "claude-test"))
        );
        let setup = app.current().unwrap();
        assert_eq!(setup.id, SETUP_ID);
        assert!(matches!(
            setup.chat().messages.first(),
            Some(Message::User(_))
        ));
    }

    #[test]
    fn connecting_later_does_not_touch_the_open_screen_or_make_the_agent() {
        let mut app = connected();
        app.route = Route::Providers;
        app.provider_connected();
        assert_eq!(app.route, Route::Providers);
        assert!(app.roster.find(&Target::agent(SETUP_ID)).is_none());
    }

    #[test]
    fn the_setup_agent_is_made_once() {
        let mut app = connected();
        app.provider_connected();
        app.route = Route::Onboarding;
        app.provider_connected();
        assert_eq!(
            app.roster.list.iter().filter(|a| a.id == SETUP_ID).count(),
            1
        );
    }

    /// Gives the open agent's chat a reply asking for `tighten_guardrails`, as a model would, runs
    /// it, and returns what came back.
    fn ask_to_tighten(app: &mut App) -> String {
        let call = crate::model::ToolCall {
            name: TIGHTEN.into(),
            summary: String::new(),
            output: None,
            id: "call-1".into(),
            args: "{}".into(),
            trip: None,
            outside: None,
        };
        let mut reply = Message::reply("", None, Default::default());
        if let Message::Assistant { tools, .. } = &mut reply {
            tools.push(call);
        }
        let chat = app.roster.current_mut().unwrap().chat_mut();
        chat.messages.push(reply);
        let sid = chat.id.clone();
        app.run_setup_calls(&sid);
        // Resolving the call starts the next turn, so the answer is in the reply that asked.
        let mut answered = app.roster.current().unwrap().chat().messages.iter().rev();
        answered
            .find_map(|m| match m {
                Message::Assistant { tools, .. } => tools
                    .iter()
                    .find(|t| t.id == "call-1")
                    .and_then(|t| t.output.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn only_the_setup_agent_is_given_the_guardrails_tool() {
        let mut app = connected();
        app.provider_connected();
        let offered = |a: &Agent| {
            crate::delegate::tools_for(a, &[], false, &[])
                .iter()
                .any(|t| t.name == TIGHTEN)
        };
        assert!(offered(app.current().unwrap()));
        assert!(!offered(&Agent::blank("other".into())));
    }

    #[test]
    fn tightening_turns_every_pack_on_and_never_off() {
        let mut app = connected();
        app.provider_connected();
        let told = ask_to_tighten(&mut app);
        for p in Pack::ALL {
            assert!(app.guard.on(p), "{p:?}");
        }
        assert!(told.contains("Hide personal details"), "{told}");
        assert!(ask_to_tighten(&mut app).contains("already on"));
    }

    #[test]
    fn another_agent_cannot_use_the_guardrails_tool() {
        let mut app = connected();
        app.provider_connected();
        let other = Target::agent("other");
        app.roster.list.push(Agent::blank("other".into()));
        app.roster.select(other);
        let told = ask_to_tighten(&mut app);
        assert!(told.starts_with("Error"), "{told}");
        assert!(!app.guard.on(Pack::Personal));
    }
}
