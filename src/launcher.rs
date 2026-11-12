//! Oido in the desktop's app launcher: its `.desktop` entry and icon under `~/.local/share`, named
//! after the app id its window reports, so launchers, the bar and window lists show its icon.
//! Release builds write them at each launch, as they update themselves in place: the entry runs
//! the binary that last launched.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The id the window reports to the desktop (Wayland `app_id`, X11 `WM_CLASS`), which names the
/// entry and the icon.
pub const APP_ID: &str = "oido-desktop";
/// The icon, drawn from Oido's logo (`ui::icons::LOGO`) on a tile. `assets/icon/oido-desktop.png`
/// is it rendered at 256 px: `rsvg-convert -w 256 -h 256 oido-desktop.svg -o oido-desktop.png`.
pub const SVG: &[u8] = include_bytes!("../assets/icon/oido-desktop.svg");
pub const PNG: &[u8] = include_bytes!("../assets/icon/oido-desktop.png");

/// Writes the entry and icon where launchers look, if they are not there already; says so on
/// stderr when it cannot.
pub fn install() {
    if let Err(e) = register() {
        eprintln!("oido: could not add Oido to the app launcher: {e}");
    }
}

/// `install`, for `--register`, which exits with the error rather than going on without it.
pub fn register() -> io::Result<()> {
    let Some(data) = dirs::data_dir() else {
        return Ok(());
    };
    let exe = installed(std::env::current_exe()?);
    let entry = entry(&exe).ok_or_else(|| {
        io::Error::other(format!("{} is not a path an entry can hold", exe.display()))
    })?;
    let icons = data.join("icons/hicolor");
    // The icon first, so a launcher that sees the entry finds it.
    write_if_changed(&icons.join(format!("scalable/apps/{APP_ID}.svg")), SVG)?;
    write_if_changed(&icons.join(format!("256x256/apps/{APP_ID}.png")), PNG)?;
    write_if_changed(
        &data.join(format!("applications/{APP_ID}.desktop")),
        entry.as_bytes(),
    )
}

/// The binary's path once an update has renamed a new one over it: Linux then names the running
/// one `<path> (deleted)`.
fn installed(exe: PathBuf) -> PathBuf {
    match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(path) => PathBuf::from(path),
        None => exe,
    }
}

/// The desktop entry that runs `exe`; `None` for a path it cannot hold (not UTF-8, or with a
/// control character).
fn entry(exe: &Path) -> Option<String> {
    let exe = exe.to_str()?;
    if exe.chars().any(char::is_control) {
        return None;
    }
    // Quoted, as the spec asks of a path that may hold spaces: `"`, `` ` ``, `$` and `\` are
    // escaped inside the quotes, then `\` again as the file's string escape, and `%` as `%%`.
    let mut quoted = String::from('"');
    for c in exe.chars() {
        match c {
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '%' => quoted.push_str("%%"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    Some(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Oido\n\
         Comment=Run your agents\n\
         Exec={quoted}\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Development;Utility;\n\
         StartupWMClass={APP_ID}\n\
         StartupNotify=true\n"
    ))
}

/// Writes `bytes` to `path` unless it already holds them, through a file beside it, so a launcher
/// never reads half an entry.
fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    fs::write(&part, bytes)?;
    fs::rename(&part, path)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_runs_this_binary_and_shows_the_icon() {
        let entry = entry(Path::new("/home/me/.local/bin/oido-desktop")).expect("entry");
        assert!(entry.contains("\nExec=\"/home/me/.local/bin/oido-desktop\"\n"));
        assert!(entry.contains("\nIcon=oido-desktop\n"));
        assert!(entry.contains("\nStartupWMClass=oido-desktop\n"));
    }

    #[test]
    fn odd_paths_are_escaped_or_refused() {
        let entry = entry(Path::new("/opt/my apps/100%/$x")).expect("entry");
        assert!(
            entry.contains("Exec=\"/opt/my apps/100%%/\\\\$x\"\n"),
            "{entry}"
        );
        assert_eq!(super::entry(Path::new("/opt/a\nExec=evil")), None);
    }

    #[test]
    fn a_replaced_binary_keeps_its_path() {
        let exe = PathBuf::from("/home/me/.local/bin/oido-desktop");
        assert_eq!(
            installed(PathBuf::from("/home/me/.local/bin/oido-desktop (deleted)")),
            exe
        );
        assert_eq!(installed(exe.clone()), exe);
    }

    #[test]
    fn files_are_written_once() {
        let dir = std::env::temp_dir().join(format!("oido-launcher-{}", uuid::Uuid::new_v4()));
        let path = dir.join("applications/oido-desktop.desktop");
        write_if_changed(&path, b"one").expect("write");
        assert_eq!(fs::read(&path).expect("read"), b"one");
        let before = fs::metadata(&path)
            .and_then(|m| m.modified())
            .expect("mtime");
        write_if_changed(&path, b"one").expect("same");
        let after = fs::metadata(&path)
            .and_then(|m| m.modified())
            .expect("mtime");
        assert_eq!(before, after, "unchanged content is not rewritten");
        write_if_changed(&path, b"two").expect("rewrite");
        assert_eq!(fs::read(&path).expect("read"), b"two");
        let _ = fs::remove_dir_all(dir);
    }
}
