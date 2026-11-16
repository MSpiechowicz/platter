//! The user's own rules ("always keep this folder", "this folder is safe to clean") and the file format
//! they are stored in (docs/architecture.md, "Files on disk"). Pure data and checks: reading and writing
//! the file belongs to the platform crate.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The largest rules file that is read, in bytes.
pub const MAX_RULES_FILE_BYTES: usize = 256 * 1024;
/// The most rules a file holds.
pub const MAX_USER_RULES: usize = 256;
/// The longest folder path of a rule, in bytes.
pub const MAX_RULE_PATH_BYTES: usize = 4096;
/// The only version of the file format.
pub const RULES_FILE_VERSION: u32 = 1;

/// What the user said about a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserVerdict {
    /// Always keep: nothing in it is ever suggested or cleaned.
    Keep,
    /// Safe to clean: the folder may be suggested (and is moved to the trash, never deleted).
    Safe,
}

/// One rule: an exact folder and what to do with it. There are no patterns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserRule {
    pub id: u32,
    pub verdict: UserVerdict,
    pub folder: PathBuf,
}

/// Why a rules file or a rule is not acceptable. The text is for people and is shown as it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RulesFileError {
    #[error("The rules file is larger than {} KiB.", MAX_RULES_FILE_BYTES / 1024)]
    TooLarge,
    #[error("The rules file is not valid: {0}")]
    Syntax(String),
    #[error("The rules file has version {0}; this Platter reads only version {RULES_FILE_VERSION}.")]
    Version(u64),
    #[error("The rules file lists more than {MAX_USER_RULES} rules.")]
    TooMany,
    #[error("The rules file uses the rule number {0} twice.")]
    DuplicateId(u32),
    #[error("The rules file's next number ({next}) is not above the rule number {id}.")]
    NextId { next: u32, id: u32 },
    #[error("The rules file names the folder {0} twice.")]
    DuplicateFolder(String),
    #[error("A folder in the rules file is not acceptable: {0}")]
    BadFolder(String),
    #[error("The rules file would be larger than {} KiB; remove a rule first.", MAX_RULES_FILE_BYTES / 1024)]
    WouldBeTooLarge,
    #[error("All rule numbers are used.")]
    OutOfIds,
}

/// The rules file: `{ "version": 1, "next_id": N, "rules": [...] }`. Every way of getting one checks it
/// (see [`parse`] and [`UserRulesFile::set`]), so a value of this type always meets the limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserRulesFile {
    pub version: u32,
    /// The id the next new rule gets; above every id in use, so a removed rule's id is not reused.
    pub next_id: u32,
    pub rules: Vec<UserRule>,
}

impl Default for UserRulesFile {
    fn default() -> Self {
        Self {
            version: RULES_FILE_VERSION,
            next_id: 1,
            rules: Vec::new(),
        }
    }
}

impl UserRulesFile {
    /// Says `verdict` about `folder`: a folder that already has a rule gets the new verdict (and keeps its
    /// id), otherwise a rule is added. Returns the rule's id. Refuses a folder that is not acceptable and a
    /// full list, and a change that would make the rendered file larger than 256 KiB even in its compact
    /// form (a path with many control characters renders at six bytes each); nothing changes then, so what
    /// is written can always be read back.
    pub fn set(&mut self, folder: &Path, verdict: UserVerdict) -> Result<u32, RulesFileError> {
        check_folder(folder)?;

        let mut changed = self.clone();
        let id = changed.apply(folder, verdict)?;
        if render(&changed).len() > MAX_RULES_FILE_BYTES {
            return Err(RulesFileError::WouldBeTooLarge);
        }

        *self = changed;
        Ok(id)
    }

    fn apply(&mut self, folder: &Path, verdict: UserVerdict) -> Result<u32, RulesFileError> {
        if let Some(rule) = self.rules.iter_mut().find(|rule| rule.folder == folder) {
            rule.verdict = verdict;
            return Ok(rule.id);
        }

        if self.rules.len() >= MAX_USER_RULES {
            return Err(RulesFileError::TooMany);
        }

        let id = self.next_id;
        let next_id = id.checked_add(1).ok_or(RulesFileError::OutOfIds)?;
        self.rules.push(UserRule {
            id,
            verdict,
            folder: folder.to_path_buf(),
        });
        self.next_id = next_id;

        Ok(id)
    }

    /// Removes the rule with `id`; false when there is none.
    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.rules.len();
        self.rules.retain(|rule| rule.id != id);

        self.rules.len() != before
    }
}

/// Checks and reads the bytes of a rules file: at most 256 KiB, version 1, at most 256 rules, unique ids
/// below `next_id`, every folder acceptable (see [`check_folder`]) and no folder named twice. A key that
/// appears twice in an object, or a field the format does not know, is refused: the file is read as the
/// typed value itself, so what was checked is what is used.
pub fn parse(bytes: &[u8]) -> Result<UserRulesFile, RulesFileError> {
    if bytes.len() > MAX_RULES_FILE_BYTES {
        return Err(RulesFileError::TooLarge);
    }

    // A file of another version is reported as such, whatever else differs in it.
    let header: Header =
        serde_json::from_slice(bytes).map_err(|error| RulesFileError::Syntax(error.to_string()))?;
    match header.version {
        Some(version) if version != u64::from(RULES_FILE_VERSION) => {
            return Err(RulesFileError::Version(version));
        }
        Some(_) => {}
        None => return Err(RulesFileError::Syntax("the version is missing".to_owned())),
    }

    let file: UserRulesFile =
        serde_json::from_slice(bytes).map_err(|error| RulesFileError::Syntax(error.to_string()))?;
    check(&file)?;

    Ok(file)
}

/// Only the version of a file, to tell a file of another version from a broken one.
#[derive(Deserialize)]
struct Header {
    version: Option<u64>,
}

/// The bytes of the rules file: pretty JSON ending in a newline, for a person who opens it. A file the
/// pretty form would push over 256 KiB is written compact instead (the newline too is left off when even
/// that is what would not fit), so a file that is read, being at most 256 KiB, can always be changed
/// whenever the changed rules fit in that size at all.
pub fn render(file: &UserRulesFile) -> Vec<u8> {
    // Serialising plain data with string keys cannot fail; the folders were checked as UTF-8.
    let mut bytes = serde_json::to_vec_pretty(file).unwrap_or_default();
    bytes.push(b'\n');
    if bytes.len() <= MAX_RULES_FILE_BYTES {
        return bytes;
    }

    let mut compact = serde_json::to_vec(file).unwrap_or_default();
    if compact.len() < MAX_RULES_FILE_BYTES {
        compact.push(b'\n');
    }

    compact
}

fn check(file: &UserRulesFile) -> Result<(), RulesFileError> {
    if file.rules.len() > MAX_USER_RULES {
        return Err(RulesFileError::TooMany);
    }

    let mut seen: Vec<u32> = Vec::with_capacity(file.rules.len());
    for rule in &file.rules {
        if seen.contains(&rule.id) {
            return Err(RulesFileError::DuplicateId(rule.id));
        }
        seen.push(rule.id);

        if rule.id >= file.next_id {
            return Err(RulesFileError::NextId {
                next: file.next_id,
                id: rule.id,
            });
        }

        check_folder(&rule.folder)?;
    }

    let mut folders: Vec<&Path> = file.rules.iter().map(|rule| rule.folder.as_path()).collect();
    folders.sort_unstable();
    if let Some(pair) = folders.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(RulesFileError::DuplicateFolder(
            pair[0].to_string_lossy().into_owned(),
        ));
    }

    Ok(())
}

/// Whether `folder` may be the folder of a rule: an absolute, normalised path of valid UTF-8, at most
/// 4096 bytes, with no NUL, `.`, `..` or empty part, and not `/` itself.
pub fn check_folder(folder: &Path) -> Result<(), RulesFileError> {
    let bad = |why: &str| RulesFileError::BadFolder(format!("{} ({why})", folder.to_string_lossy()));

    let Some(text) = folder.to_str() else {
        return Err(bad("not valid text"));
    };
    if text.len() > MAX_RULE_PATH_BYTES {
        return Err(bad("too long"));
    }
    if text.contains('\0') {
        return Err(bad("contains a NUL"));
    }
    if !folder.is_absolute() {
        return Err(bad("not absolute"));
    }

    let mut rebuilt = PathBuf::new();
    let mut names = 0usize;
    for component in folder.components() {
        match component {
            Component::CurDir | Component::ParentDir => return Err(bad("contains . or ..")),
            Component::Normal(_) => names += 1,
            Component::Prefix(_) | Component::RootDir => {}
        }
        rebuilt.push(component);
    }

    if names == 0 {
        return Err(bad("the root folder"));
    }
    // Doubled and trailing separators and `.` parts disappear when the path is rebuilt from its parts.
    if rebuilt.to_str() != Some(text) {
        return Err(bad("not normalised"));
    }

    Ok(())
}

#[cfg(test)]
mod tests;
