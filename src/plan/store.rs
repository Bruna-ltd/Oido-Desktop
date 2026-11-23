//! A folder's plans on disk, under `.oido/` in the chat's working folder, so any tab or model
//! opened on that folder sees the same plans:
//!
//! ```text
//! .oido/session.json     which plan is active, which are parked
//! .oido/plans/<id>.json  one plan
//! ```
//!
//! Every write goes to a temporary file and is renamed over the real one, so a crash leaves the old
//! file or the new one, never half of either. Locking against a second writer comes with the
//! router, the first thing that has two tabs changing one folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Plan, State};

/// The format of `session.json`. A file from a newer build is refused rather than misread.
pub const VERSION: u32 = 1;

const DIR: &str = ".oido";
const STATE_FILE: &str = "session.json";
const PLANS: &str = "plans";

pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store for a working folder. Nothing is touched until something is saved.
    pub fn open(folder: &Path) -> Store {
        Store {
            root: folder.join(DIR),
        }
    }

    /// What the folder remembers; empty when nothing was saved yet.
    pub fn state(&self) -> io::Result<State> {
        let Some(state) = read::<State>(&self.root.join(STATE_FILE))? else {
            return Ok(State::default());
        };
        if state.version > VERSION {
            return Err(invalid(format!(
                "session.json is version {}, newer than this build reads ({VERSION})",
                state.version
            )));
        }
        Ok(state)
    }

    pub fn save_state(&self, state: &State) -> io::Result<()> {
        write(&self.root.join(STATE_FILE), state)
    }

    pub fn plan(&self, id: &str) -> io::Result<Option<Plan>> {
        read(&self.plan_path(id)?)
    }

    pub fn save_plan(&self, plan: &Plan) -> io::Result<()> {
        write(&self.plan_path(&plan.id)?, plan)
    }

    /// An id comes back out of `session.json`, which anyone can edit, so it is never trusted to be
    /// a file name: only what [`crate::model::new_id`] makes is allowed.
    fn plan_path(&self, id: &str) -> io::Result<PathBuf> {
        let safe = !id.is_empty()
            && id.len() <= 64
            && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !safe {
            return Err(invalid(format!("{id:?} is not a plan id")));
        }
        Ok(self.root.join(PLANS).join(format!("{id}.json")))
    }
}

fn invalid(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

fn read<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| invalid(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn write<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(value).map_err(|e| invalid(e.to_string()))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.part");
    fs::write(&temp, json)?;
    fs::rename(&temp, path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::tests::spec;
    use super::super::{Plan, StepStatus};
    use super::*;

    fn folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oido-plan-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_empty_folder_has_the_default_state_and_no_plans() {
        let dir = folder();
        let store = Store::open(&dir);
        assert_eq!(store.state().unwrap(), State::default());
        assert_eq!(store.plan("nope").unwrap(), None);
        assert!(!dir.join(DIR).exists(), "reading must not create .oido");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn state_and_plans_round_trip_through_a_second_store() {
        let dir = folder();
        let mut plan = Plan::new("trip", vec![spec("a", true), spec("b", false)], 2).unwrap();
        plan.steps[0].status = StepStatus::Done;
        plan.steps[0].result = Some("booked".into());
        let state = State {
            active_plan_id: Some(plan.id.clone()),
            ..State::default()
        };

        let first = Store::open(&dir);
        first.save_plan(&plan).unwrap();
        first.save_state(&state).unwrap();

        // Another tab, or another model, opening the same folder.
        let second = Store::open(&dir);
        assert_eq!(second.state().unwrap(), state);
        assert_eq!(second.plan(&plan.id).unwrap(), Some(plan));
        assert!(
            !dir.join(DIR)
                .join(PLANS)
                .read_dir()
                .unwrap()
                .any(|f| { f.unwrap().file_name().to_string_lossy().ends_with(".part") })
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_id_that_is_not_a_plain_id_never_becomes_a_path() {
        let store = Store::open(&folder());
        for id in ["", "../session", "a/b", "a.b", "..", &"x".repeat(65)] {
            assert!(store.plan(id).is_err(), "{id:?} was allowed");
        }
    }

    #[test]
    fn a_newer_or_damaged_session_file_is_refused() {
        let dir = folder();
        let store = Store::open(&dir);
        store
            .save_state(&State {
                version: VERSION + 1,
                ..State::default()
            })
            .unwrap();
        assert!(store.state().is_err());

        fs::write(dir.join(DIR).join(STATE_FILE), "{ not json").unwrap();
        assert!(store.state().is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
