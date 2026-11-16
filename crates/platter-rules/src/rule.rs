//! What a rule is: plain data. Adding a rule is one entry in `table.rs`; nothing here does anything.

use platter_core::dto::Verdict;
use platter_core::{Category, Os, UserDir, UserRule, UserVerdict};

const SECONDS_PER_DAY: i64 = 86_400;

/// A condition on how long ago something was last modified (a directory's age is its newest content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Age {
    /// Not modified for at least this many days.
    Idle(u32),
    /// Modified less than this many days ago. The opposite of [`Age::Idle`] with the same number.
    Recent(u32),
}

impl Age {
    /// The number of days in the condition.
    pub const fn days(self) -> u32 {
        match self {
            Age::Idle(days) | Age::Recent(days) => days,
        }
    }

    /// Whether something last modified at `mtime` satisfies the condition at time `now` (unix seconds).
    pub fn holds(self, mtime: i64, now: i64) -> bool {
        let age = now.saturating_sub(mtime);
        let limit = i64::from(self.days()) * SECONDS_PER_DAY;

        match self {
            Age::Idle(_) => age >= limit,
            Age::Recent(_) => age < limit,
        }
    }
}

/// A directory with one of these names, anywhere in the scan, optionally with conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Named {
    pub names: &'static [&'static str],
    /// The directory's parent must contain a file with this name (`Cargo.toml` next to `target`).
    pub sibling: Option<&'static str>,
    /// The parent must also hold a file with one of these names (a lockfile next to `node_modules`).
    pub sibling_any: &'static [&'static str],
    /// No folder on the way down to the directory, scan root included, has one of these names: tools
    /// install their own copies there (`~/.vscode/extensions/*/node_modules`).
    pub not_inside: &'static [&'static str],
    /// The directory is not below one of these absolute paths (`/usr`, `/opt`).
    pub not_below: &'static [&'static str],
    pub age: Option<Age>,
}

impl Named {
    pub const fn new(names: &'static [&'static str]) -> Self {
        Self {
            names,
            sibling: None,
            sibling_any: &[],
            not_inside: &[],
            not_below: &[],
            age: None,
        }
    }

    /// Only when the parent directory also holds a file with one of these names.
    pub const fn next_to_any(mut self, markers: &'static [&'static str]) -> Self {
        self.sibling_any = markers;
        self
    }

    /// Never inside a folder with one of these names.
    pub const fn not_inside(mut self, names: &'static [&'static str]) -> Self {
        self.not_inside = names;
        self
    }

    /// Never below one of these absolute paths.
    pub const fn not_below(mut self, paths: &'static [&'static str]) -> Self {
        self.not_below = paths;
        self
    }

    /// Only when the parent directory holds a file called `marker`.
    pub const fn next_to(mut self, marker: &'static str) -> Self {
        self.sibling = Some(marker);
        self
    }

    /// Only when nothing inside was modified for `days` days.
    pub const fn idle(mut self, days: u32) -> Self {
        self.age = Some(Age::Idle(days));
        self
    }

    /// Only when something inside was modified less than `days` days ago.
    pub const fn recent(mut self, days: u32) -> Self {
        self.age = Some(Age::Recent(days));
        self
    }
}

/// Individual files below a folder of the home directory, selected by what they are. The suggestion
/// covers only the bytes of the files that qualify, not the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Files {
    /// Folder relative to the home directory; empty means the whole home directory.
    pub home_relative: &'static str,
    pub age: Option<Age>,
    pub category: Option<Category>,
    /// Smallest allocated size of a qualifying file.
    pub min_bytes: u64,
}

impl Files {
    pub const fn in_home(home_relative: &'static str) -> Self {
        Self {
            home_relative,
            age: None,
            category: None,
            min_bytes: 0,
        }
    }

    pub const fn idle(mut self, days: u32) -> Self {
        self.age = Some(Age::Idle(days));
        self
    }

    pub const fn of(mut self, category: Category) -> Self {
        self.category = Some(category);
        self
    }

    pub const fn at_least(mut self, bytes: u64) -> Self {
        self.min_bytes = bytes;
        self
    }
}

/// What a rule looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matcher {
    /// The directory at this absolute path.
    Fixed(&'static str),
    /// The directory at this path below the home directory (`.cache`, `.local/share/Trash`).
    Home(&'static str),
    /// One of the user's personal folders, wherever the desktop put it and whatever it is called.
    UserDir(UserDir),
    /// A directory by name, anywhere.
    Named(Named),
    /// Files by age, category and size below a home folder.
    Files(Files),
    /// The folders of the user's own rules with this verdict, matched by exact path and never by name.
    User(UserVerdict),
}

impl Matcher {
    /// The age threshold in days, for the reason text.
    pub const fn age_days(&self) -> Option<u32> {
        let age = match self {
            Matcher::Named(named) => named.age,
            Matcher::Files(files) => files.age,
            Matcher::Fixed(_) | Matcher::Home(_) | Matcher::UserDir(_) | Matcher::User(_) => None,
        };

        match age {
            Some(age) => Some(age.days()),
            None => None,
        }
    }

    /// The size threshold in bytes, for the reason text.
    pub const fn min_bytes(&self) -> Option<u64> {
        match self {
            Matcher::Files(files) if files.min_bytes > 0 => Some(files.min_bytes),
            _ => None,
        }
    }
}

/// One cleanup rule.
///
/// `reason` is a template: `{n:project}` is the number of matches with a noun that gets an `s` unless
/// `{n:copy|copies}` gives both forms, `{days}` the age threshold of the matcher and `{size}` its size
/// threshold. Numbers in the text therefore always come from the real matches or the rule's own data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    /// Stable identifier, unique in a [`RuleSet`](crate::RuleSet).
    pub id: &'static str,
    pub title: &'static str,
    pub verdict: Verdict,
    pub reason: &'static str,
    pub matcher: Matcher,
    /// The only operating system the rule applies to; `None` for all.
    pub os: Option<Os>,
}

impl Rule {
    pub const fn new(
        id: &'static str,
        title: &'static str,
        verdict: Verdict,
        matcher: Matcher,
        reason: &'static str,
    ) -> Self {
        Self {
            id,
            title,
            verdict,
            reason,
            matcher,
            os: None,
        }
    }

    /// Restricts the rule to one operating system.
    pub const fn only_on(mut self, os: Os) -> Self {
        self.os = Some(os);
        self
    }
}

/// An ordered collection of rules, and the user's own rules they are matched against. When two rules match
/// the same folder, the earlier one wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSet {
    rules: Vec<Rule>,
    user: Vec<UserRule>,
}

impl RuleSet {
    /// A custom set of rules, with no user rules.
    pub fn new(rules: Vec<Rule>) -> Self {
        Self {
            rules,
            user: Vec::new(),
        }
    }

    /// The rules Platter ships with.
    pub fn builtin() -> Self {
        Self::new(crate::table::RULES.to_vec())
    }

    /// This set with the user's own rules: two more entries, `user-keep` before every other rule and
    /// `user-safe` after them, each matching exactly the folders of the user rules with that verdict. So a
    /// `keep` of any source wins, then the built-in rules, then the user's `safe`. A set that already had
    /// user rules gets these ones instead.
    pub fn with_user(mut self, user: Vec<UserRule>) -> Self {
        self.rules
            .retain(|rule| !matches!(rule.matcher, Matcher::User(_)));
        self.rules.insert(0, crate::table::USER_KEEP);
        self.rules.push(crate::table::USER_SAFE);
        self.user = user;

        self
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// The user's own rules this set was given.
    pub fn user(&self) -> &[UserRule] {
        &self.user
    }
}
