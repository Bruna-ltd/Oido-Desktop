//! Versions and updates: which version this is, asking GitHub for the newest release, and putting
//! its binary in place of this one so the next launch runs it.
//!
//! A release is a GitHub release tagged `vX.Y.Z` with two files for each machine:
//! `oido-desktop-<arch>-linux` and `oido-desktop-<arch>-linux.sha256` (`sha256sum`'s output).
//! `scripts/release.sh` makes one. The binary is replaced by a rename in its own folder, so a
//! running app keeps its old file and a failed download never leaves half a binary behind.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::app::App;

/// This build's version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The GitHub repository whose releases are this app's updates.
const REPO: &str = "Bruna-ltd/Oido-Desktop";
/// How often automatic updates look for a new release, the first time at launch.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Asking for the newest release; downloading it may take longer.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// A download larger than this is refused (a release binary is about 50 MB).
const MAX_BINARY: u64 = 400 * 1024 * 1024;

/// A release version, `major.minor.patch`, ordered as numbers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl FromStr for Version {
    type Err = String;

    /// Reads `1.2.3` or a tag's `v1.2.3`. Anything else, a pre-release among them, is an error.
    fn from_str(s: &str) -> Result<Version, String> {
        let bad = || format!("not a version: {s:?}");
        let mut parts = s.strip_prefix('v').unwrap_or(s).split('.');
        let mut next =
            || -> Result<u64, String> { parts.next().ok_or_else(bad)?.parse().map_err(|_| bad()) };
        let v = Version {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        match parts.next() {
            None => Ok(v),
            Some(_) => Err(bad()),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The newest release, and where its binary for this machine and that binary's checksum are.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Release {
    pub version: Version,
    binary: String,
    checksum: String,
}

/// Where automatic and asked-for updates stand.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Installing(Version),
    /// Installed: the next launch runs it.
    Ready(Version),
    Failed(String),
}

/// Updates: whether they happen on their own, where they stand, and the binary they replace.
#[derive(Debug)]
pub struct Updates {
    pub auto: bool,
    pub status: Status,
    /// This app's binary, as found at launch: once replaced, Linux reports the running one's
    /// path with " (deleted)" after it. `None` in development builds, which never replace
    /// themselves (that would overwrite `target/debug`), and when it could not be found.
    exe: Option<PathBuf>,
}

impl Updates {
    pub fn new(auto: bool) -> Updates {
        let exe = if cfg!(debug_assertions) {
            None
        } else {
            std::env::current_exe().ok()
        };
        Updates {
            auto,
            status: Status::Idle,
            exe,
        }
    }

    /// Whether this build can update itself.
    pub fn available(&self) -> bool {
        self.exe.is_some()
    }

    /// A check or an install is under way.
    pub fn busy(&self) -> bool {
        matches!(self.status, Status::Checking | Status::Installing(_))
    }
}

/// What this machine's release files are called.
fn asset_name() -> String {
    format!("oido-desktop-{}-linux", std::env::consts::ARCH)
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(format!("oido-desktop/{VERSION}"))
        .build()
        .into()
}

/// The newest release that has a binary for this machine. `None` when none does yet.
///
/// # Errors
///
/// The network, GitHub's answer, or a tag that is not a version, in words.
pub fn latest() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = agent(CHECK_TIMEOUT)
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| format!("could not reach GitHub: {e}"))?;
    match resp.status().as_u16() {
        // No release published yet.
        404 => return Ok(None),
        200 => {}
        code => return Err(format!("GitHub answered {code}")),
    }
    let body = resp
        .into_body()
        .read_to_string()
        .map_err(|e| format!("could not read GitHub's answer: {e}"))?;
    parse_release(&body, &asset_name())
}

/// The newest release, when it is newer than this build.
fn newer() -> Result<Option<Release>, String> {
    let current: Version = VERSION.parse()?;
    Ok(latest()?.filter(|r| r.version > current))
}

/// The release in GitHub's `releases/latest` answer, if it has `asset` and its checksum.
fn parse_release(json: &str, asset: &str) -> Result<Option<Release>, String> {
    #[derive(Deserialize)]
    struct Answer {
        tag_name: String,
        assets: Vec<Asset>,
    }
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    let answer: Answer =
        serde_json::from_str(json).map_err(|e| format!("unexpected answer from GitHub: {e}"))?;
    let version = answer.tag_name.parse()?;
    let url = |name: &str| {
        answer
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
    };
    let sum = format!("{asset}.sha256");
    Ok(url(asset).zip(url(&sum)).map(|(binary, checksum)| Release {
        version,
        binary,
        checksum,
    }))
}

/// Downloads `release`, checks it against its checksum, and puts it in place of `exe`.
///
/// # Errors
///
/// The download, a checksum that does not match, or a binary that cannot be replaced (one
/// installed by a package manager, say), in words.
fn install(release: &Release, exe: &Path) -> Result<(), String> {
    let agent = agent(DOWNLOAD_TIMEOUT);
    let fetch = |url: &str, limit: u64| -> Result<Vec<u8>, String> {
        agent
            .get(url)
            .call()
            .map_err(|e| format!("could not download the update: {e}"))?
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| format!("could not download the update: {e}"))
    };
    let checksum = String::from_utf8(fetch(&release.checksum, 4096)?)
        .map_err(|_| "the update's checksum is not text".to_owned())?;
    let binary = fetch(&release.binary, MAX_BINARY)?;
    verify(&binary, &checksum)?;
    replace(exe, &binary).map_err(|e| format!("could not replace {}: {e}", exe.display()))
}

/// Whether `bytes` match `checksum`, a SHA-256 in hex (`sha256sum`'s line: the hash, then a name).
fn verify(bytes: &[u8], checksum: &str) -> Result<(), String> {
    let want = checksum
        .split_whitespace()
        .next()
        .ok_or("the update's checksum is empty")?
        .to_ascii_lowercase();
    let got: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got == want {
        Ok(())
    } else {
        Err("the download does not match its checksum".to_owned())
    }
}

/// Writes `bytes` next to `exe` and renames them over it: either the whole new binary is in
/// place, or the old one still is.
fn replace(exe: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let name = exe
        .file_name()
        .ok_or_else(|| io::Error::other("the binary has no file name"))?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(".update");
    let temp = exe.with_file_name(temp_name);
    let written = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.set_permissions(fs::Permissions::from_mode(0o755))?;
        file.sync_all()?;
        fs::rename(&temp, exe)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

/// What a check found: an install to start, or where things stand.
enum Found {
    Newer(Release),
    Nothing(Status),
}

impl App {
    /// Looks for a newer release and installs it. Does nothing while one is already under way, in
    /// a build that cannot update itself, or once an update waits for a restart.
    pub fn check_for_updates(&mut self) {
        if !self.updates.available()
            || self.updates.busy()
            || matches!(self.updates.status, Status::Ready(_))
        {
            return;
        }
        self.updates.status = Status::Checking;
        self.spawn(
            || match newer() {
                Ok(Some(r)) => Found::Newer(r),
                Ok(None) => Found::Nothing(Status::UpToDate),
                Err(e) => Found::Nothing(Status::Failed(e)),
            },
            |s, found| match found {
                Found::Newer(release) => s.install_update(release),
                Found::Nothing(status) => s.updates.status = status,
            },
        );
    }

    fn install_update(&mut self, release: Release) {
        let Some(exe) = self.updates.exe.clone() else {
            return;
        };
        let version = release.version;
        self.updates.status = Status::Installing(version);
        self.spawn(
            move || install(&release, &exe),
            move |s, result| {
                s.updates.status = match result {
                    Ok(()) => Status::Ready(version),
                    Err(e) => Status::Failed(e),
                }
            },
        );
    }

    /// Starts the installed update and quits this app.
    pub fn restart_to_update(&mut self, tx: &mut rune::Tx) {
        let Some(exe) = &self.updates.exe else {
            return;
        };
        match std::process::Command::new(exe).spawn() {
            Ok(_) => tx.quit(),
            Err(e) => self.updates.status = Status::Failed(format!("could not restart: {e}")),
        }
    }

    /// Turns automatic updates on or off, and remembers it. Turning them on checks at once.
    pub fn set_auto_update(&mut self, on: bool) {
        self.updates.auto = on;
        self.save_auto_update();
        if on {
            self.check_for_updates();
        }
    }
}

/// Calls `check` at launch and every [`CHECK_EVERY`] after, from a thread of its own that lives
/// as long as the app.
pub fn every_so_often(check: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        loop {
            check();
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    #[test]
    fn versions_read_with_or_without_a_v_and_order_as_numbers() {
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert_eq!(v("0.10.0").to_string(), "0.10.0");
        assert!(v("0.10.0") > v("0.9.9"), "numbers, not text");
        assert!(v("1.0.0") > v("0.99.99"));
        for bad in ["", "1.2", "1.2.3.4", "1.2.x", "1.2.3-beta", "v"] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?} is not a version");
        }
    }

    #[test]
    fn this_builds_version_is_a_version() {
        assert_eq!(v(VERSION).to_string(), VERSION);
    }

    const ANSWER: &str = r#"{
        "tag_name": "v0.2.0",
        "assets": [
            {"name": "oido-desktop-x86_64-linux", "browser_download_url": "https://x/bin"},
            {"name": "oido-desktop-x86_64-linux.sha256", "browser_download_url": "https://x/sum"},
            {"name": "oido-desktop-aarch64-linux", "browser_download_url": "https://x/arm"}
        ]
    }"#;

    #[test]
    fn a_release_has_this_machines_binary_and_its_checksum_or_is_not_one() {
        let release = parse_release(ANSWER, "oido-desktop-x86_64-linux")
            .unwrap()
            .unwrap();
        assert_eq!(
            release,
            Release {
                version: v("0.2.0"),
                binary: "https://x/bin".to_owned(),
                checksum: "https://x/sum".to_owned(),
            }
        );
        let no_checksum = parse_release(ANSWER, "oido-desktop-aarch64-linux").unwrap();
        assert_eq!(
            no_checksum, None,
            "a binary without its checksum is never offered"
        );
        assert!(parse_release(r#"{"tag_name":"nightly","assets":[]}"#, "x").is_err());
    }

    #[test]
    fn a_download_is_checked_against_sha256sums_line() {
        // sha256 of "hello\n", as `sha256sum` prints it.
        let line = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03  oido\n";
        assert_eq!(verify(b"hello\n", line), Ok(()));
        assert_eq!(
            verify(b"hello\n", &line.to_uppercase()),
            Ok(()),
            "hex in any case"
        );
        assert!(verify(b"tampered\n", line).is_err());
        assert!(verify(b"hello\n", "").is_err());
    }

    #[test]
    fn replacing_swaps_the_whole_binary_in_and_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("oido-update-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let exe = dir.join("oido-desktop");
        fs::write(&exe, b"old").unwrap();
        replace(&exe, b"new").unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(&exe).unwrap().permissions().mode()
        };
        assert_eq!(mode & 0o777, 0o755, "executable");
        let left: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file left over");
        fs::remove_dir_all(&dir).unwrap();
    }
}
