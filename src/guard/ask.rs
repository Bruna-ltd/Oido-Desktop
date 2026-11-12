//! Asking for a go-ahead from work nobody is watching: a team member handed a task, or a workflow
//! step. Its call waits on a card where the person will see it (the chat that started the work, or
//! the workflow's page) until they answer, the wait runs out, or the work is stopped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use super::actions::Finding;
use super::{Trip, TripState};
use crate::app::App;

/// Where a card for unwatched work shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Where {
    /// The chat whose agent handed the task over, or ran the workflow.
    Chat(String),
    /// The page of the workflow (run from the Workflows page, or by a schedule).
    Workflow(String),
}

/// A call from unwatched work, waiting for the person's go-ahead.
#[derive(Debug)]
pub struct Waiting {
    pub id: String,
    pub at: Where,
    /// Who made the call: the team member or the workflow, by name.
    pub from: String,
    pub tool: String,
    /// The call in a line (the command, the file).
    pub summary: String,
    pub trip: Trip,
    answer: Sender<TripState>,
    /// The work's own stop: once set, nobody waits for the answer.
    stopped: Arc<AtomicBool>,
}

impl Waiting {
    /// A call waiting with nobody to answer to, for screenshots.
    #[cfg(debug_assertions)]
    pub fn demo(at: Where, from: &str, tool: &str, summary: &str, trip: Trip) -> Waiting {
        Waiting {
            id: crate::model::new_id(),
            at,
            from: from.to_owned(),
            tool: tool.to_owned(),
            summary: summary.to_owned(),
            trip,
            answer: mpsc::channel().0,
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Whether the work that asked was stopped, so the card no longer matters.
    pub fn gone(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

/// Hands a [`Waiting`] call to whatever shows it.
type Post = Arc<dyn Fn(Waiting) + Send + Sync>;

/// What unwatched work asks through: where its card goes, who it is, and how the card is shown.
#[derive(Clone)]
pub struct Asker {
    at: Option<Where>,
    from: String,
    post: Option<Post>,
}

impl std::fmt::Debug for Asker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Asker")
            .field("at", &self.at)
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

impl Asker {
    /// Whether there is anyone to ask.
    pub fn can_ask(&self) -> bool {
        self.post.is_some()
    }

    /// Nobody to ask: a call a rule questions does not run.
    pub fn nobody() -> Asker {
        Asker {
            at: None,
            from: String::new(),
            post: None,
        }
    }

    /// Asks through `post`, which shows the card at `at`; `from` names who asks.
    pub fn new(at: Where, from: &str, post: impl Fn(Waiting) + Send + Sync + 'static) -> Asker {
        Asker {
            at: Some(at),
            from: from.to_owned(),
            post: Some(Arc::new(post)),
        }
    }

    /// Puts the call `tool` (`summary` in a line), which broke the rule in `found`, to the person,
    /// and waits for their answer: allowed, turned down, or unanswered in time. Stopping the work
    /// (`stop`) ends the wait as turned down. With nobody to ask, it is turned down at once.
    pub fn ask(
        &self,
        tool: &str,
        summary: &str,
        found: &Finding,
        stop: &Arc<AtomicBool>,
    ) -> TripState {
        let (Some(at), Some(post)) = (&self.at, &self.post) else {
            return TripState::Denied;
        };
        let (answer, answered) = mpsc::channel();
        post(Waiting {
            id: crate::model::new_id(),
            at: at.clone(),
            from: self.from.clone(),
            tool: tool.to_owned(),
            summary: summary.to_owned(),
            trip: Trip {
                rule: found.rule.to_owned(),
                why: found.why.clone(),
                state: TripState::Asking,
                asked_at: Some(crate::app::now()),
            },
            answer,
            stopped: Arc::clone(stop),
        });
        loop {
            match answered.recv_timeout(Duration::from_millis(250)) {
                Ok(state) => return state,
                Err(RecvTimeoutError::Timeout) if stop.load(Ordering::Relaxed) => {
                    return TripState::Denied;
                }
                Err(RecvTimeoutError::Timeout) => {}
                // Dropped unanswered: the app closed.
                Err(RecvTimeoutError::Disconnected) => return TripState::TimedOut,
            }
        }
    }
}

impl App {
    /// What unwatched work at `at`, by `from`, asks through: a card in the app, or with no app
    /// running (tests, headless renders) nobody, so a questioned call does not run.
    pub(crate) fn asker(&self, at: Where, from: &str) -> Asker {
        let Some(handle) = crate::app::handle() else {
            return Asker::nobody();
        };
        let handle = handle.clone();
        Asker::new(at, from, move |waiting| {
            handle.update(move |app, _| app.unwatched_asks(waiting));
        })
    }

    /// A call from unwatched work waits for you: its card shows, and you are told.
    fn unwatched_asks(&mut self, waiting: Waiting) {
        let chat = match &waiting.at {
            Where::Chat(sid) => Some(sid.clone()),
            Where::Workflow(_) => None,
        };
        crate::notify::post(crate::notify::Note {
            title: waiting.from.clone(),
            body: "Needs your go-ahead to carry on.".to_owned(),
            chat,
        });
        self.waiting.push(waiting);
    }

    /// Your answer on the card of unwatched work's call `id`.
    pub fn decide_waiting(&mut self, id: &str, allow: bool) {
        self.answer_waiting(
            id,
            if allow {
                TripState::Allowed
            } else {
                TripState::Denied
            },
        );
    }

    fn answer_waiting(&mut self, id: &str, state: TripState) {
        if let Some(i) = self.waiting.iter().position(|w| w.id == id) {
            let w = self.waiting.remove(i);
            // The work may have been stopped meanwhile; then nobody listens, which is fine.
            let _ = w.answer.send(state);
        }
    }

    /// Unwatched work's calls that waited longer than `wait_secs` are not run; cards of work that
    /// was stopped go.
    pub(crate) fn expire_waiting(&mut self, now: i64, wait_secs: i64) {
        self.waiting.retain(|w| !w.gone());
        let late: Vec<String> = self
            .waiting
            .iter()
            .filter(|w| w.trip.asked_at.is_some_and(|at| now - at >= wait_secs))
            .map(|w| w.id.clone())
            .collect();
        for id in late {
            self.answer_waiting(&id, TripState::TimedOut);
        }
    }

    /// The calls waiting at `at`, for its cards.
    pub fn waiting_at<'a>(&'a self, at: &'a Where) -> impl Iterator<Item = &'a Waiting> {
        self.waiting
            .iter()
            .filter(move |w| &w.at == at && !w.gone())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::guard::Action;

    fn found() -> Finding {
        Finding {
            rule: "shell.admin",
            why: "It runs as administrator.".into(),
            action: Action::Ask,
        }
    }

    /// An app that keeps what is posted, and an asker posting to it from a worker thread.
    fn asking(at: Where) -> (Arc<Mutex<Vec<Waiting>>>, Asker) {
        let posted: Arc<Mutex<Vec<Waiting>>> = Arc::default();
        let into = Arc::clone(&posted);
        (
            posted,
            Asker::new(at, "checker", move |w| into.lock().unwrap().push(w)),
        )
    }

    fn answered(posted: &Arc<Mutex<Vec<Waiting>>>, app: &mut App) -> String {
        loop {
            if let Some(w) = posted.lock().unwrap().pop() {
                let id = w.id.clone();
                app.waiting.push(w);
                return id;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_worker_waits_for_the_answer_given_on_its_card() {
        let mut app = App::new(None);
        let (posted, asker) = asking(Where::Chat("s1".into()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = std::thread::spawn(move || asker.ask("shell", "sudo ls", &found(), &stop));
        let id = answered(&posted, &mut app);
        assert_eq!(app.waiting_at(&Where::Chat("s1".into())).count(), 1);
        assert_eq!(app.waiting_at(&Where::Chat("s2".into())).count(), 0);
        app.decide_waiting(&id, true);
        assert_eq!(worker.join().unwrap(), TripState::Allowed);
        assert!(app.waiting.is_empty(), "the card goes once answered");
    }

    #[test]
    fn unanswered_in_time_it_does_not_run_and_stopped_work_stops_waiting() {
        let mut app = App::new(None);
        let (posted, asker) = asking(Where::Workflow("w1".into()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = std::thread::spawn({
            let asker = asker.clone();
            let stop = Arc::clone(&stop);
            move || asker.ask("shell", "sudo ls", &found(), &stop)
        });
        answered(&posted, &mut app);
        app.expire_waiting(crate::app::now() + 600, 600);
        assert_eq!(worker.join().unwrap(), TripState::TimedOut);

        let worker = std::thread::spawn({
            let stop = Arc::clone(&stop);
            move || asker.ask("shell", "sudo ls", &found(), &stop)
        });
        answered(&posted, &mut app);
        stop.store(true, Ordering::Relaxed);
        assert_eq!(worker.join().unwrap(), TripState::Denied);
        app.expire_waiting(crate::app::now(), 600);
        assert!(app.waiting.is_empty(), "the stopped work's card went");
    }

    #[test]
    fn with_nobody_to_ask_a_questioned_call_does_not_run() {
        let stop = Arc::new(AtomicBool::new(false));
        assert_eq!(
            Asker::nobody().ask("shell", "sudo ls", &found(), &stop),
            TripState::Denied
        );
    }
}
