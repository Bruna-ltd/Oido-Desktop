//! Running a command for the shell tool. This is the only place the app starts a process on the
//! model's say-so. There is no approval step: a command runs as soon as an agent that was given
//! the shell tool asks (see [`crate::run`]), so the limits below, and which agents have the tool,
//! are the only controls.
//!
//! A command runs through `sh -c` in its own process group, with no input, under a time limit.
//! Its output is read as it comes and cut at a limit, so a noisy command can neither fill the pipe
//! and hang nor flood the model.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const DEFAULT_SECS: u64 = 30;
pub const MAX_SECS: u64 = 120;
/// Each of stdout and stderr is kept up to this many bytes.
const STREAM_LIMIT: usize = 8 * 1024;
/// How long to wait for the output to finish after the command ends.
const DRAIN: Duration = Duration::from_millis(500);

/// A command as the model asked for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Spec {
    pub command: String,
    /// Why the model wants it, to show you.
    pub reason: String,
    pub timeout: Duration,
}

/// Reads a call's arguments. `None` when there is no command to run.
pub fn parse(args: &str) -> Option<Spec> {
    let v: Value = serde_json::from_str(args).ok()?;
    let command = v.get("command")?.as_str()?.trim().to_owned();
    if command.is_empty() {
        return None;
    }
    let secs = v
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_SECS)
        .clamp(1, MAX_SECS);
    Some(Spec {
        command,
        reason: v
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        timeout: Duration::from_secs(secs),
    })
}

/// How a command ended, with what it printed in words for the model.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finish {
    Done(String),
    /// You pressed Stop.
    Stopped(String),
}

struct Capture {
    data: Mutex<(Vec<u8>, bool)>,
}

/// Reads `source` to its end on a thread, keeping the first [`STREAM_LIMIT`] bytes.
fn capture(mut source: impl Read + Send + 'static) -> (Arc<Capture>, std::thread::JoinHandle<()>) {
    let cap = Arc::new(Capture {
        data: Mutex::new((Vec::new(), false)),
    });
    let shared = cap.clone();
    let handle = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = source.read(&mut buf) {
            if n == 0 {
                break;
            }
            if let Ok(mut d) = shared.data.lock() {
                let room = STREAM_LIMIT.saturating_sub(d.0.len());
                d.0.extend_from_slice(&buf[..n.min(room)]);
                d.1 |= n > room;
            }
        }
    });
    (cap, handle)
}

fn text_of(cap: &Capture) -> (String, bool) {
    cap.data.lock().map_or_else(
        |_| (String::new(), false),
        |d| (String::from_utf8_lossy(&d.0).into_owned(), d.1),
    )
}

/// Ends the command and anything it started.
fn kill(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{}", child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// What a command did, before it is put into words.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Raw {
    pub end: Ended,
    pub stdout: String,
    pub stderr: String,
    pub stdout_cut: bool,
    pub stderr_cut: bool,
    /// The time limit it was under, for saying so.
    pub limit_secs: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ended {
    /// It finished; `None` when a signal ended it.
    Exit(Option<i32>),
    TimedOut,
    Stopped,
    /// It could not be started or waited on.
    Failed(String),
}

impl Raw {
    /// Whether it ran to the end and reported success.
    pub fn succeeded(&self) -> bool {
        self.end == Ended::Exit(Some(0))
    }

    /// In words, with what it printed, for the model.
    pub fn text(&self) -> String {
        let mut text = match &self.end {
            Ended::Exit(Some(code)) => format!("exit code: {code}"),
            Ended::Exit(None) => "The command was ended by a signal.".to_owned(),
            Ended::TimedOut => format!(
                "The command ran past its {} second limit and was ended.",
                self.limit_secs
            ),
            Ended::Stopped => "The person stopped the command before it finished.".to_owned(),
            Ended::Failed(why) => why.clone(),
        };
        for (label, body, cut) in [
            ("stdout", &self.stdout, self.stdout_cut),
            ("stderr", &self.stderr, self.stderr_cut),
        ] {
            if !body.is_empty() {
                text.push_str(&format!("\n{label}:\n{}", body.trim_end()));
                if cut {
                    text.push_str("\n[output cut here]");
                }
            }
        }
        text
    }
}

/// Runs `spec` in `dir` and waits for it, up to its time limit, giving the result in words.
pub fn run(spec: &Spec, dir: &Path, cancel: &AtomicBool) -> Finish {
    let raw = run_raw(spec, dir, cancel);
    let text = raw.text();
    match raw.end {
        Ended::Stopped => Finish::Stopped(text),
        _ => Finish::Done(text),
    }
}

/// Runs `spec` in `dir` and waits for it, up to its time limit. Blocks, so call it off the UI
/// thread. `cancel` is checked about every 25 ms.
pub fn run_raw(spec: &Spec, dir: &Path, cancel: &AtomicBool) -> Raw {
    let failed = |why: String| Raw {
        end: Ended::Failed(why),
        stdout: String::new(),
        stderr: String::new(),
        stdout_cut: false,
        stderr_cut: false,
        limit_secs: spec.timeout.as_secs(),
    };
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(&spec.command)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return failed(format!("The command could not be started: {e}")),
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        kill(&mut child);
        return failed("The command could not be started: no output pipes.".to_owned());
    };
    let (out, out_thread) = capture(stdout);
    let (err, err_thread) = capture(stderr);

    let deadline = Instant::now() + spec.timeout;
    let end = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ended::Exit(status.code()),
            Ok(None) => {}
            Err(e) => {
                kill(&mut child);
                break Ended::Failed(format!("The command could not be waited on: {e}"));
            }
        }
        if cancel.load(Ordering::Relaxed) {
            kill(&mut child);
            break Ended::Stopped;
        }
        if Instant::now() >= deadline {
            kill(&mut child);
            break Ended::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let _ = child.wait();
    // A process that left the group can hold the pipe open; take what there is after a moment.
    let drained = Instant::now() + DRAIN;
    while Instant::now() < drained && !(out_thread.is_finished() && err_thread.is_finished()) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let (stdout, stdout_cut) = text_of(&out);
    let (stderr, stderr_cut) = text_of(&err);
    Raw {
        end,
        stdout,
        stderr,
        stdout_cut,
        stderr_cut,
        limit_secs: spec.timeout.as_secs(),
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn spec(command: &str, secs: u64) -> Spec {
        Spec {
            command: command.into(),
            reason: String::new(),
            timeout: Duration::from_secs(secs),
        }
    }

    fn run_in(command: &str, secs: u64) -> Finish {
        run(
            &spec(command, secs),
            Path::new("/tmp"),
            &AtomicBool::new(false),
        )
    }

    fn done(f: Finish) -> String {
        match f {
            Finish::Done(t) => t,
            Finish::Stopped(t) => panic!("stopped: {t}"),
        }
    }

    #[test]
    fn a_command_runs_in_its_folder_and_reports_its_exit_and_output() {
        let t = done(run_in("pwd; echo oops >&2; exit 3", 5));
        assert!(t.starts_with("exit code: 3"), "{t}");
        assert!(
            t.contains("stdout:\n/tmp") && t.contains("stderr:\noops"),
            "{t}"
        );
        assert_eq!(done(run_in("true", 5)), "exit code: 0");
    }

    #[test]
    fn a_command_cannot_read_input_so_one_that_waits_for_it_ends() {
        let started = Instant::now();
        assert!(done(run_in("cat", 5)).starts_with("exit code: 0"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn one_that_runs_too_long_is_ended_with_everything_it_started() {
        let started = Instant::now();
        let t = done(run_in("echo before; sleep 30 & sleep 30", 1));
        assert!(
            t.contains("ran past its 1 second limit") && t.contains("before"),
            "{t}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "it was not waited out"
        );
    }

    #[test]
    fn stopping_ends_it_at_once_and_keeps_what_it_printed() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let f = run(
            &spec("echo started; sleep 30", 60),
            Path::new("/tmp"),
            &cancel,
        );
        stopper.join().unwrap();
        let Finish::Stopped(t) = f else {
            panic!("stopped");
        };
        assert!(
            t.contains("stopped the command") && t.contains("started"),
            "{t}"
        );
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn long_output_is_cut_and_says_so() {
        let t = done(run_in("yes x | head -c 100000", 5));
        assert!(t.contains("[output cut here]"), "cut");
        assert!(t.len() < STREAM_LIMIT + 200, "{}", t.len());
    }

    #[test]
    fn arguments_are_read_with_a_clamped_time_limit_and_no_command_means_nothing_to_run() {
        let s = parse(r#"{"command":" ls -la ","reason":"look","timeout_secs":9999}"#).unwrap();
        assert_eq!(
            (s.command.as_str(), s.reason.as_str(), s.timeout),
            ("ls -la", "look", Duration::from_secs(MAX_SECS))
        );
        assert_eq!(
            parse(r#"{"command":"x"}"#).unwrap().timeout,
            Duration::from_secs(DEFAULT_SECS)
        );
        assert!(parse(r#"{"command":"  "}"#).is_none());
        assert!(parse("not json").is_none() && parse("{}").is_none());
    }
}
