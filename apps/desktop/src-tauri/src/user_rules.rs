//! The user's rules as the backend holds them: the rules file's content, a generation that counts every
//! change, and the only code that changes the file (docs/architecture.md, "Suggestions" and "Files on disk").
//!
//! The file is read again on every look, so a hand edit (or a file that broke) is seen at once and counts
//! as a change like any other. Whatever uses the rules (suggestions, plans) remembers the generation it
//! worked from, and is made again, or refused, when the generation has moved on.
//!
//! A file that is broken is never overwritten here: every change is refused until it was moved aside.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use platter_core::{RulesFileError, UserRule, UserRulesFile, UserVerdict};
use platter_platform::user_rules::{Loaded, load, move_aside, save};
use platter_rules::RuleSet;

use crate::error::{CommandError, CommandResult};

/// The user's rules at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRulesView {
    /// Counts the changes of the rules and of the file; starts at 0.
    pub generation: u64,
    /// The rules; none while the file is broken.
    pub rules: Vec<UserRule>,
    /// Why the file cannot be used, when it cannot.
    pub problem: Option<String>,
}

impl UserRulesView {
    /// The built-in rules with these user rules.
    pub fn rule_set(&self) -> RuleSet {
        RuleSet::builtin().with_user(self.rules.clone())
    }

    /// `invalid` while the rules file is broken: with rules that cannot be read, nothing says which
    /// folders the user keeps, so nothing may be planned.
    pub fn usable(&self) -> CommandResult<()> {
        match &self.problem {
            None => Ok(()),
            Some(problem) => Err(CommandError::invalid(format!(
                "Your rules file cannot be used, so Platter makes no cleanup plan: {problem} \
                 Fix the file, or start over in the Advisor view."
            ))),
        }
    }
}

struct Current {
    generation: u64,
    loaded: Loaded,
}

pub struct UserRulesStore {
    /// `None` when the settings folder is not known: no rules, and none can be added.
    path: Option<PathBuf>,
    current: Mutex<Current>,
}

impl UserRulesStore {
    /// Loads the rules file at `path` (a file that does not exist is an empty list).
    pub fn new(path: Option<PathBuf>) -> Self {
        let loaded = match &path {
            Some(path) => load(path),
            None => Loaded::Rules(UserRulesFile::default()),
        };

        Self {
            path,
            current: Mutex::new(Current {
                generation: 0,
                loaded,
            }),
        }
    }

    /// The rules file of this user (`$XDG_CONFIG_HOME/platter/rules.json`).
    pub fn production() -> Self {
        Self::new(platter_platform::user_rules::rules_path())
    }

    fn lock(&self) -> MutexGuard<'_, Current> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reads the file again; a difference from what was held counts as a change.
    fn refresh(&self, current: &mut Current) {
        let Some(path) = &self.path else {
            return;
        };

        let now = load(path);
        if now != current.loaded {
            current.loaded = now;
            current.generation += 1;
        }
    }

    fn view_of(current: &Current) -> UserRulesView {
        match &current.loaded {
            Loaded::Rules(file) => UserRulesView {
                generation: current.generation,
                rules: file.rules.clone(),
                problem: None,
            },
            Loaded::Broken(problem) => UserRulesView {
                generation: current.generation,
                rules: Vec::new(),
                problem: Some(problem.clone()),
            },
        }
    }

    /// The rules now. Reads the file, so call it off the async runtime.
    pub fn view(&self) -> UserRulesView {
        let mut current = self.lock();
        self.refresh(&mut current);

        Self::view_of(&current)
    }

    /// The generation now (see [`UserRulesStore::view`]).
    pub fn generation(&self) -> u64 {
        self.view().generation
    }

    /// Says `verdict` about `folder`, replacing an earlier verdict about the same folder. `invalid` for a
    /// folder a rule cannot name, a full list or a broken file; `io` when the file cannot be written.
    pub fn set(&self, folder: &Path, verdict: UserVerdict) -> CommandResult<UserRulesView> {
        self.change(|file| file.set(folder, verdict).map(|_| ()))
    }

    /// Removes the rule `id`. An unknown id changes nothing.
    pub fn remove(&self, id: u32) -> CommandResult<UserRulesView> {
        self.change(|file| {
            file.remove(id);
            Ok(())
        })
    }

    fn change(
        &self,
        apply: impl FnOnce(&mut UserRulesFile) -> Result<(), RulesFileError>,
    ) -> CommandResult<UserRulesView> {
        let mut current = self.lock();
        self.refresh(&mut current);
        Self::view_of(&current).usable()?;

        let Some(path) = &self.path else {
            return Err(CommandError::io(
                "Platter does not know where to keep your rules: the settings folder is not known.",
            ));
        };
        let Loaded::Rules(held) = &current.loaded else {
            return Err(CommandError::invalid("Your rules file cannot be used."));
        };

        let mut file = held.clone();
        apply(&mut file).map_err(|error| CommandError::invalid(error.to_string()))?;
        if file == *held {
            return Ok(Self::view_of(&current));
        }

        save(path, &file)
            .map_err(|error| CommandError::io(format!("Your rules could not be saved: {error}")))?;
        current.loaded = Loaded::Rules(file);
        current.generation += 1;

        Ok(Self::view_of(&current))
    }

    /// Moves a broken rules file to `rules.json.broken` and starts with no rules. `invalid` when the file
    /// is not broken.
    pub fn reset(&self) -> CommandResult<UserRulesView> {
        let mut current = self.lock();
        self.refresh(&mut current);
        if !matches!(current.loaded, Loaded::Broken(_)) {
            return Err(CommandError::invalid("The rules file is not broken."));
        }

        let Some(path) = &self.path else {
            return Err(CommandError::invalid("The rules file is not broken."));
        };
        move_aside(path)
            .map_err(|error| CommandError::io(format!("The rules file could not be moved aside: {error}")))?;

        current.loaded = Loaded::Rules(UserRulesFile::default());
        current.generation += 1;

        Ok(Self::view_of(&current))
    }
}

/// The rules now, read off the async runtime (the file is read each time).
pub async fn current(store: &Arc<UserRulesStore>) -> CommandResult<UserRulesView> {
    let store = Arc::clone(store);

    Ok(tauri::async_runtime::spawn_blocking(move || store.view()).await?)
}

#[cfg(test)]
mod tests;
