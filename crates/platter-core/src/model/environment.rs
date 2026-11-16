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

/// Facts about the machine that a scan result has to be judged against, supplied by the caller so the
/// code that interprets a finished [`Tree`](super::Tree) never has to read the system itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// The user's home directory (absolute, canonical when known); `None` when it is not known.
    pub home: Option<PathBuf>,
    /// The current time in unix seconds; ages are measured against it.
    pub now: i64,
    pub os: Os,
}
