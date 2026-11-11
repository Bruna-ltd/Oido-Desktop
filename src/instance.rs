//! One running app per database. Opening Oido again while it runs (its window closed, its icon in
//! the tray) shows the running app's window instead of starting a second one, which would fire
//! every schedule twice.
//!
//! The running app holds an exclusive lock on `oido-desktop-<id>.lock` in the user's runtime
//! folder for as long as it lives (the system lets go when the process ends, crashed or not), and
//! listens on the socket beside it. A later launch finds the lock taken, sends its activation
//! token down the socket, so the compositor lets the window take focus, and exits. `<id>` comes
//! from the database's path, so apps on different databases (a copy on `$OIDO_DB`) run side by side.

use std::fs::{self, File, TryLockError};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// How long a later launch keeps trying to reach a running app that is still starting up.
const PATIENCE: Duration = Duration::from_secs(3);
const RETRY: Duration = Duration::from_millis(100);
/// The longest token a launch may send; real ones are well under 100 bytes.
const MAX_TOKEN: u64 = 1024;

/// What this launch should do.
#[derive(Debug)]
pub enum Launch {
    /// Run the app. With a claim, it is the database's one app: [`listen`] for later launches.
    /// Without one (no runtime folder, no database file), nothing can tell, so it runs anyway.
    Run(Option<Claim>),
    /// Another process runs the app on this database and was asked to show its window: exit.
    Exit,
}

/// This process's hold on its database, kept for as long as the app runs.
#[derive(Debug)]
pub struct Claim {
    _lock: File,
    listener: UnixListener,
}

/// Decides whether this launch runs the app or hands over to the one already running.
pub fn claim() -> Launch {
    let db = match crate::store::default_location() {
        Ok(Some(db)) => db,
        Ok(None) => return Launch::Run(None),
        Err(e) => {
            eprintln!("oido: cannot find the database to check for a running app: {e}");
            return Launch::Run(None);
        }
    };
    let Some(dir) = dirs::runtime_dir() else {
        return Launch::Run(None);
    };
    claim_in(&dir, &db, &activation_token())
}

/// [`claim`], with the lock and socket in `dir`, for the database at `db`; a later launch sends
/// `token`.
fn claim_in(dir: &Path, db: &Path, token: &str) -> Launch {
    let (lock_path, socket) = paths(dir, db);
    let lock = match File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(lock) => lock,
        Err(e) => {
            eprintln!(
                "oido: cannot check for a running app ({}): {e}",
                lock_path.display()
            );
            return Launch::Run(None);
        }
    };
    match lock.try_lock() {
        Ok(()) => match bind(&socket) {
            Ok(listener) => Launch::Run(Some(Claim {
                _lock: lock,
                listener,
            })),
            Err(e) => {
                // Still the only app on the database (the lock says so); later launches just
                // cannot reach it, and say so rather than open a second one.
                eprintln!(
                    "oido: cannot listen for later launches ({}): {e}",
                    socket.display()
                );
                Launch::Run(None)
            }
        },
        Err(TryLockError::WouldBlock) => {
            if let Err(e) = tell(&socket, token) {
                eprintln!("oido: Oido is already running but did not answer: {e}");
            }
            Launch::Exit
        }
        Err(TryLockError::Error(e)) => {
            eprintln!(
                "oido: cannot check for a running app ({}): {e}",
                lock_path.display()
            );
            Launch::Run(None)
        }
    }
}

/// The lock file and socket for the database at `db`, in `dir`.
fn paths(dir: &Path, db: &Path) -> (PathBuf, PathBuf) {
    let db = std::path::absolute(db).unwrap_or_else(|_| db.to_path_buf());
    let hash = Sha256::digest(db.as_os_str().as_encoded_bytes());
    let id: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    (
        dir.join(format!("oido-desktop-{id}.lock")),
        dir.join(format!("oido-desktop-{id}.sock")),
    )
}

/// Listens at `socket`, replacing one a crashed app left behind: only the lock's holder calls this.
fn bind(socket: &Path) -> io::Result<UnixListener> {
    match fs::remove_file(socket) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    UnixListener::bind(socket)
}

/// Asks the app listening at `socket` to show its window, waiting a moment for one still starting
/// (it holds the lock before it listens).
fn tell(socket: &Path, token: &str) -> io::Result<()> {
    let mut waited = Duration::ZERO;
    let mut stream = loop {
        match UnixStream::connect(socket) {
            Ok(stream) => break stream,
            Err(e) if waited >= PATIENCE => return Err(e),
            Err(_) => {
                thread::sleep(RETRY);
                waited += RETRY;
            }
        }
    };
    stream.write_all(format!("{token}\n").as_bytes())
}

/// The token the launcher gave this launch, so the running app's window may take focus.
fn activation_token() -> String {
    ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|t| !t.is_empty()))
        .unwrap_or_default()
}

/// Runs `show` with each later launch's activation token (empty when it had none), on a thread of
/// its own, for as long as the app runs.
pub fn listen(claim: Claim, show: impl Fn(String) + Send + 'static) {
    thread::spawn(move || {
        let Claim { _lock, listener } = claim;
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => show(read_token(stream)),
                Err(e) => eprintln!("oido: a later launch could not reach this one: {e}"),
            }
        }
    });
}

/// A launch's token: one line of printable ASCII, else none. Only this user's processes can reach
/// the socket (the runtime folder is theirs alone), but a slow or odd one must not stall it.
fn read_token(stream: UnixStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    if BufReader::new(stream.take(MAX_TOKEN))
        .read_line(&mut line)
        .is_err()
    {
        return String::new();
    }
    let token = line.trim_end_matches('\n');
    if token.bytes().all(|b| b.is_ascii_graphic()) {
        token.to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oido-instance-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_second_launch_shows_the_first_and_exits() {
        let dir = dir();
        let db = dir.join("oido.db");
        let Launch::Run(Some(claim)) = claim_in(&dir, &db, "") else {
            panic!("the first launch runs the app");
        };
        let (tx, rx) = mpsc::channel();
        listen(claim, move |token| {
            tx.send(token).expect("test is listening")
        });

        assert!(matches!(claim_in(&dir, &db, "token-123"), Launch::Exit));
        let got = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the first app hears");
        assert_eq!(got, "token-123");

        assert!(matches!(claim_in(&dir, &db, ""), Launch::Exit));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).expect("again"), "");
    }

    #[test]
    fn apps_on_different_databases_run_side_by_side() {
        let dir = dir();
        let first = claim_in(&dir, &dir.join("a.db"), "");
        let second = claim_in(&dir, &dir.join("b.db"), "");
        assert!(matches!(first, Launch::Run(Some(_))));
        assert!(matches!(second, Launch::Run(Some(_))));
    }

    #[test]
    fn a_socket_left_by_a_crashed_app_is_replaced() {
        let dir = dir();
        let db = dir.join("oido.db");
        let (_, socket) = paths(&dir, &db);
        drop(UnixListener::bind(&socket).expect("stale socket"));
        assert!(socket.exists());
        assert!(matches!(claim_in(&dir, &db, ""), Launch::Run(Some(_))));
    }

    #[test]
    fn odd_tokens_are_dropped() {
        let (mut a, b) = UnixStream::pair().expect("pair");
        a.write_all(b"bad token\n").expect("write");
        assert_eq!(read_token(b), "");
    }
}
