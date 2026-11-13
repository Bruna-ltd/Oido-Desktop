//! Processes the agent starts to keep running after its call returns: a dev server, a watcher, a
//! long build. The `shell` tool waits for a command and ends it after a time limit; with
//! `background` set it starts the command here instead and answers at once, and the `process` tool
//! lists, reads and stops what is running.
//!
//! Each process belongs to the chat that started it: another chat cannot see or end it. All of
//! them end when the chat is stopped, closed or deleted, and when the app quits, because nothing
//! else is following them. A process runs through `sh -c` in its own process group (as in
//! [`crate::command`]), so ending it ends what it started. It should stay in the foreground of
//! that shell, with no `&` or `nohup`: once the shell itself has exited the process counts as
//! ended and what it left behind is no longer followed.

use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

use crate::command;

/// Most processes one chat keeps running at once.
pub const MAX_RUNNING: usize = 8;
/// Ended processes one chat keeps, to read their output.
const KEEP_ENDED: usize = 8;
/// A process's output is kept from the end, up to this many bytes.
const TAIL: usize = 16 * 1024;
/// What one read gives back, at most.
const SHOWN: usize = 8 * 1024;
/// Longest command shown in a list.
const NAME_LEN: usize = 100;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum State {
    #[default]
    Running,
    /// It ended by itself; `None` when a signal ended it.
    Exited(Option<i32>),
    /// It was ended on purpose.
    Stopped,
}

/// What a process has printed and how it stands, shared with the threads that follow it.
#[derive(Default)]
struct Inner {
    tail: Vec<u8>,
    /// Everything it printed, of which `tail` is the end.
    total: usize,
    state: State,
}

impl Inner {
    fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len();
        self.tail.extend_from_slice(bytes);
        if let Some(over) = self.tail.len().checked_sub(TAIL) {
            self.tail.drain(..over);
        }
    }
}

struct Process {
    id: String,
    sid: String,
    command: String,
    pid: u32,
    began: Instant,
    inner: Arc<Mutex<Inner>>,
}

impl Process {
    fn state(&self) -> State {
        self.inner.lock().map_or(State::Stopped, |i| i.state)
    }

    /// Ends it if it is running, with what it started. Whether it was.
    fn end(&self) -> bool {
        let was_running = self.inner.lock().is_ok_and(|mut i| {
            let running = i.state == State::Running;
            if running {
                i.state = State::Stopped;
            }
            running
        });
        if was_running {
            command::end_group(self.pid);
        }
        was_running
    }

    /// How it stands and the end of what it printed, in words for the model.
    fn report(&self) -> String {
        let Ok(inner) = self.inner.lock() else {
            return format!("{}: its record was left broken.", self.id);
        };
        let state = match inner.state {
            State::Running => format!("running for {}s", self.began.elapsed().as_secs()),
            State::Exited(Some(code)) => format!("exited with code {code}"),
            State::Exited(None) => "ended by a signal".to_owned(),
            State::Stopped => "stopped".to_owned(),
        };
        let mut text = format!("{} `{}`: {state}", self.id, name(&self.command));
        let shown = &inner.tail[inner.tail.len().saturating_sub(SHOWN)..];
        if shown.is_empty() {
            text.push_str("\nIt has printed nothing yet.");
        } else {
            text.push_str(&format!(
                "\nOutput, the last {} of {} bytes:\n{}",
                shown.len(),
                inner.total,
                String::from_utf8_lossy(shown).trim_end()
            ));
        }
        text
    }
}

/// The command on one line, cut short, for a list.
fn name(command: &str) -> String {
    let line = command.lines().next().unwrap_or_default();
    if line.chars().count() > NAME_LEN || command.contains('\n') {
        format!("{}…", line.chars().take(NAME_LEN).collect::<String>())
    } else {
        line.to_owned()
    }
}

fn field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
}

/// Reads `source` to its end on a thread, into the process's output.
fn follow(mut source: impl Read + Send + 'static, inner: Arc<Mutex<Inner>>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = source.read(&mut buf) {
            if n == 0 {
                break;
            }
            if let Ok(mut i) = inner.lock() {
                i.push(&buf[..n]);
            }
        }
    });
}

/// Every background process of every chat.
#[derive(Default)]
pub struct Processes {
    list: Vec<Process>,
    /// How many were started, for the next one's name.
    started: u32,
}

impl Processes {
    /// Starts `command` in `dir` for chat `sid` and gives its name, without waiting for it.
    pub fn start(&mut self, sid: &str, command: &str, dir: &Path) -> Result<String, String> {
        if self.running_in(sid) >= MAX_RUNNING {
            return Err(format!(
                "{MAX_RUNNING} background processes are already running in this chat. Stop one with the process tool first."
            ));
        }
        let mut child = command::shell(command, dir)
            .spawn()
            .map_err(|e| format!("The command could not be started: {e}"))?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            command::end_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            return Err("The command could not be started: no output pipes.".to_owned());
        };
        let inner = Arc::new(Mutex::new(Inner::default()));
        follow(stdout, Arc::clone(&inner));
        follow(stderr, Arc::clone(&inner));
        let pid = child.id();
        let watched = Arc::clone(&inner);
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|status| status.code());
            if let Ok(mut i) = watched.lock()
                && i.state == State::Running
            {
                i.state = State::Exited(code);
            }
            // Nothing else tells the screen it ended, so its "running" marks would stay.
            if let Some(handle) = crate::app::handle() {
                handle.update(|_, _| {});
            }
        });
        self.forget_old_ended(sid);
        self.started += 1;
        let id = format!("bg{}", self.started);
        self.list.push(Process {
            id: id.clone(),
            sid: sid.to_owned(),
            command: command.to_owned(),
            pid,
            began: Instant::now(),
            inner,
        });
        Ok(id)
    }

    /// How many of chat `sid`'s are running.
    pub fn running_in(&self, sid: &str) -> usize {
        self.list
            .iter()
            .filter(|p| p.sid == sid && p.state() == State::Running)
            .count()
    }

    /// Ends every process chat `sid` started.
    pub fn stop_chat(&self, sid: &str) {
        for p in self.list.iter().filter(|p| p.sid == sid) {
            p.end();
        }
    }

    /// Ends every process there is.
    pub fn stop_all(&self) {
        for p in &self.list {
            p.end();
        }
    }

    /// Answers a call of the `process` tool from chat `sid`, in words for the model.
    pub fn call(&self, sid: &str, args: &str) -> String {
        let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
        let (action, id) = (field(&v, "action"), field(&v, "id"));
        if action == "list" {
            return self.list_of(sid);
        }
        if !matches!(action, "read" | "stop") {
            return "Error: `action` is one of list, read and stop.".to_owned();
        }
        let Some(p) = self.list.iter().find(|p| p.sid == sid && p.id == id) else {
            return format!(
                "Error: this chat has no background process called {id:?}. {}",
                self.list_of(sid)
            );
        };
        if action == "read" {
            return p.report();
        }
        let was_running = p.end();
        let report = p.report();
        if was_running {
            format!("Stopped.\n{report}")
        } else {
            format!("It had already ended.\n{report}")
        }
    }

    fn list_of(&self, sid: &str) -> String {
        let lines: Vec<String> = self
            .list
            .iter()
            .filter(|p| p.sid == sid)
            .map(|p| {
                let state = match p.state() {
                    State::Running => "running".to_owned(),
                    State::Exited(Some(code)) => format!("exited with code {code}"),
                    State::Exited(None) => "ended by a signal".to_owned(),
                    State::Stopped => "stopped".to_owned(),
                };
                format!("{} {state}: {}", p.id, name(&p.command))
            })
            .collect();
        if lines.is_empty() {
            "This chat has no background processes.".to_owned()
        } else {
            lines.join("\n")
        }
    }

    /// Makes room for one more of chat `sid`'s: its oldest ended ones beyond [`KEEP_ENDED`] go.
    fn forget_old_ended(&mut self, sid: &str) {
        let ended = self
            .list
            .iter()
            .filter(|p| p.sid == sid && p.state() != State::Running)
            .count();
        let mut over = ended.saturating_sub(KEEP_ENDED - 1);
        self.list.retain(|p| {
            let drop = over > 0 && p.sid == sid && p.state() != State::Running;
            over -= usize::from(drop);
            !drop
        });
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    const DIR: &str = "/tmp";

    fn started(p: &mut Processes, sid: &str, command: &str) -> String {
        p.start(sid, command, Path::new(DIR)).unwrap()
    }

    /// Calls `check` until it holds, for up to three seconds.
    fn within_a_moment(what: &str, mut check: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(3);
        while !check() {
            assert!(Instant::now() < until, "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn read(p: &Processes, sid: &str, id: &str) -> String {
        p.call(sid, &format!(r#"{{"action":"read","id":"{id}"}}"#))
    }

    #[test]
    fn a_process_runs_on_while_its_output_is_read_and_stopping_ends_it_with_what_it_started() {
        let pids = std::env::temp_dir().join(format!("oido-bg-{}", uuid::Uuid::new_v4()));
        let mut p = Processes::default();
        let id = started(
            &mut p,
            "s1",
            &format!("echo up; sleep 60 & echo $! > {}; wait", pids.display()),
        );
        within_a_moment("it printed", || read(&p, "s1", &id).contains("up"));
        assert!(read(&p, "s1", &id).contains("running for"));
        assert_eq!(p.running_in("s1"), 1);
        let child: u32 = within_a_moment_value(|| {
            std::fs::read_to_string(&pids)
                .ok()
                .and_then(|t| t.trim().parse().ok())
        });

        let said = p.call("s1", &format!(r#"{{"action":"stop","id":"{id}"}}"#));
        assert!(
            said.starts_with("Stopped.") && said.contains("stopped"),
            "{said}"
        );
        assert_eq!(p.running_in("s1"), 0);
        within_a_moment("what it started ended too", || gone(child));
        let again = p.call("s1", &format!(r#"{{"action":"stop","id":"{id}"}}"#));
        assert!(again.starts_with("It had already ended."), "{again}");
        let _ = std::fs::remove_file(pids);
    }

    fn within_a_moment_value<T>(mut get: impl FnMut() -> Option<T>) -> T {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(v) = get() {
                return v;
            }
            assert!(Instant::now() < until, "it never came");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Whether process `pid` is gone (or only waiting to be collected).
    fn gone(pid: u32) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(_) => true,
            Ok(s) => s
                .rsplit(") ")
                .next()
                .is_some_and(|rest| rest.starts_with('Z')),
        }
    }

    #[test]
    fn one_that_ends_by_itself_says_how() {
        let mut p = Processes::default();
        let id = started(&mut p, "s1", "echo hi; exit 3");
        within_a_moment("it ended", || {
            read(&p, "s1", &id).contains("exited with code 3")
        });
        let said = read(&p, "s1", &id);
        assert!(said.contains("hi"), "{said}");
        assert_eq!(p.running_in("s1"), 0);
        assert!(p.list_of("s1").contains("exited with code 3"));
    }

    #[test]
    fn another_chat_can_neither_see_nor_end_it() {
        let mut p = Processes::default();
        let id = started(&mut p, "s1", "sleep 60");
        let said = p.call("s2", &format!(r#"{{"action":"stop","id":"{id}"}}"#));
        assert!(
            said.starts_with("Error:") && said.contains("no background processes"),
            "{said}"
        );
        assert_eq!(p.running_in("s1"), 1, "still running");
        p.stop_chat("s2");
        assert_eq!(p.running_in("s1"), 1);
        p.stop_chat("s1");
        assert_eq!(p.running_in("s1"), 0);
    }

    #[test]
    fn a_chat_may_run_only_so_many_and_stop_all_ends_every_chats() {
        let mut p = Processes::default();
        for _ in 0..MAX_RUNNING {
            started(&mut p, "s1", "sleep 60");
        }
        let refused = p.start("s1", "sleep 60", Path::new(DIR)).unwrap_err();
        assert!(refused.contains("already running"), "{refused}");
        started(&mut p, "s2", "sleep 60");
        assert_eq!((p.running_in("s1"), p.running_in("s2")), (MAX_RUNNING, 1));
        p.stop_all();
        assert_eq!((p.running_in("s1"), p.running_in("s2")), (0, 0));
    }

    #[test]
    fn long_output_keeps_its_end_and_says_how_much_there_was() {
        let mut p = Processes::default();
        let id = started(&mut p, "s1", "yes x | head -c 100000");
        within_a_moment("it ended", || read(&p, "s1", &id).contains("exited"));
        let said = read(&p, "s1", &id);
        assert!(
            said.contains(&format!("the last {SHOWN} of 100000 bytes")),
            "{}",
            said.len()
        );
        assert!(said.len() < SHOWN + 200);
    }

    #[test]
    fn only_so_many_ended_ones_are_kept_and_the_oldest_go_first() {
        let mut p = Processes::default();
        let mut ids = Vec::new();
        for _ in 0..KEEP_ENDED + 3 {
            let id = started(&mut p, "s1", "true");
            within_a_moment("it ended", || p.running_in("s1") == 0);
            ids.push(id);
        }
        let list = p.list_of("s1");
        assert_eq!(list.lines().count(), KEEP_ENDED, "{list}");
        assert!(!list.contains(&format!("{} ", ids[0])), "{list}");
        assert!(list.contains(ids.last().unwrap()), "{list}");
    }

    #[test]
    fn a_call_with_a_wrong_action_or_name_says_what_is_allowed() {
        let mut p = Processes::default();
        assert_eq!(
            p.call("s1", r#"{"action":"list"}"#),
            "This chat has no background processes."
        );
        assert!(p.call("s1", "not json").contains("list, read and stop"));
        assert!(
            p.call("s1", r#"{"action":"read","id":"bg9"}"#)
                .contains("no background process called")
        );
        let id = started(&mut p, "s1", "sleep 60");
        assert!(
            p.call("s1", r#"{"action":"list"}"#)
                .contains(&format!("{id} running: sleep 60"))
        );
        p.stop_all();
    }
}
