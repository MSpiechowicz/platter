use std::path::PathBuf;

/// The operating system family a scan is judged on. Decided by the caller (the platform crate), never
/// guessed by the domain code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
    Other,
}

/// A personal folder of the user that has a well-known role, whatever it is called on this machine
/// (`~/Documents` is `~/Dokumenty` on a Polish desktop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserDir {
    Desktop,
    Documents,
    Music,
    Pictures,
    Videos,
}

impl UserDir {
    pub const ALL: [UserDir; 5] = [
        UserDir::Desktop,
        UserDir::Documents,
        UserDir::Music,
        UserDir::Pictures,
        UserDir::Videos,
    ];

    /// The folder's name below the home directory when the machine does not say otherwise.
    pub const fn default_name(self) -> &'static str {
        match self {
            UserDir::Desktop => "Desktop",
            UserDir::Documents => "Documents",
            UserDir::Music => "Music",
            UserDir::Pictures => "Pictures",
            UserDir::Videos => "Videos",
        }
    }
}

/// Facts about the machine that a scan result has to be judged against, supplied by the caller so the
/// code that interprets a finished [`Tree`](super::Tree) never has to read the system itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// The user's home directory (absolute, canonical when known); `None` when it is not known.
    pub home: Option<PathBuf>,
    /// Where the user's personal folders really are (absolute, canonical when known), as the desktop
    /// configured them. A folder that is not listed is assumed under its default name in `home`.
    pub user_dirs: Vec<(UserDir, PathBuf)>,
    /// The current time in unix seconds; ages are measured against it.
    pub now: i64,
    pub os: Os,
}

impl Environment {
    /// Where the personal folder `dir` is: the configured place, otherwise its default name below the
    /// home directory. `None` when neither is known.
    pub fn user_dir(&self, dir: UserDir) -> Option<PathBuf> {
        self.user_dirs
            .iter()
            .find(|(kind, _)| *kind == dir)
            .map(|(_, path)| path.clone())
            .or_else(|| self.home.as_ref().map(|home| home.join(dir.default_name())))
    }
}
