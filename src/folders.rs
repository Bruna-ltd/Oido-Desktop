//! Choosing a folder without a system dialog: a small browser over the folders on disk. It holds
//! where you are and what is inside; the screen draws it.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct Browser {
    pub open: bool,
    /// The folder being looked at.
    pub at: PathBuf,
    /// Its subfolders, by name.
    pub entries: Vec<String>,
    /// Why it could not be read, when it could not.
    pub error: Option<String>,
    pub hidden: bool,
}

impl Browser {
    /// Opens at `start` if it is a folder, else at the home folder.
    pub fn show(&mut self, start: Option<&Path>) {
        self.open = true;
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let at = start.filter(|p| p.is_dir()).map_or(home, Path::to_path_buf);
        self.go(at);
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn go(&mut self, path: PathBuf) {
        match list(&path, self.hidden) {
            Ok(entries) => {
                (self.at, self.entries, self.error) = (path, entries, None);
            }
            // Stay where we were, and say why.
            Err(why) => self.error = Some(format!("Cannot open {}: {why}", path.display())),
        }
    }

    pub fn up(&mut self) {
        if let Some(parent) = self.at.parent() {
            self.go(parent.to_path_buf());
        }
    }

    pub fn enter(&mut self, name: &str) {
        self.go(self.at.join(name));
    }

    pub fn toggle_hidden(&mut self) {
        self.hidden ^= true;
        self.go(self.at.clone());
    }
}

/// The folders inside `path` (links to folders count), by name, ignoring case (then by case). Names that start
/// with a dot are left out unless `hidden`.
pub fn list(path: &Path, hidden: bool) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = std::fs::read_dir(path)
        .map_err(|e| e.kind().to_string())?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| hidden || !n.starts_with('.'))
        .collect();
    names.sort_by_key(|n| (n.to_lowercase(), n.clone()));
    Ok(names)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!("oido-folders-{}", uuid::Uuid::new_v4()));
        for d in ["beta", "Alpha", ".secret", "alpha/inner"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("file.txt"), "x").unwrap();
        root
    }

    #[test]
    fn only_folders_are_listed_by_name_with_hidden_ones_on_request() {
        let root = tree();
        assert_eq!(list(&root, false).unwrap(), ["Alpha", "alpha", "beta"]);
        assert_eq!(
            list(&root, true).unwrap(),
            [".secret", "Alpha", "alpha", "beta"]
        );
        assert!(list(&root.join("nowhere"), false).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_browser_goes_in_and_up_and_stays_put_when_a_folder_cannot_be_opened() {
        let root = tree();
        let mut b = Browser::default();
        b.show(Some(&root));
        assert!(b.open && b.at == root);
        b.enter("alpha");
        assert_eq!(b.entries, ["inner"]);
        b.up();
        assert_eq!(b.at, root);
        b.enter("nowhere");
        assert_eq!(b.at, root, "it did not move");
        assert!(b.error.as_deref().unwrap().contains("Cannot open"));
        b.toggle_hidden();
        assert!(b.entries.contains(&".secret".to_owned()) && b.error.is_none());
        let mut from_file = Browser::default();
        from_file.show(Some(&root.join("file.txt")));
        assert_eq!(
            Some(from_file.at),
            dirs::home_dir(),
            "a start that is not a folder falls back to home"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
