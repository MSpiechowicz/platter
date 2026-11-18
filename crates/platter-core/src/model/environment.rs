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

    /// The folder's name below the home directory on `os` when the machine does not say otherwise. The
    /// videos folder is `Movies` on macOS.
    pub const fn default_name(self, os: Os) -> &'static str {
        match (self, os) {
            (UserDir::Desktop, _) => "Desktop",
            (UserDir::Documents, _) => "Documents",
            (UserDir::Music, _) => "Music",
            (UserDir::Pictures, _) => "Pictures",
            (UserDir::Videos, Os::MacOs) => "Movies",
            (UserDir::Videos, _) => "Videos",
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
    /// Windows only: the system folder (`%SystemRoot%`, `C:\Windows`), absolute; `None` on every other
    /// OS and when it is not known. Rules and the cleanup guard protect it and what is inside.
    pub system_root: Option<PathBuf>,
}

impl Environment {
    /// Where the personal folder `dir` is: the configured place, otherwise its default name below the
    /// home directory. `None` when neither is known.
    pub fn user_dir(&self, dir: UserDir) -> Option<PathBuf> {
        self.user_dirs
            .iter()
            .find(|(kind, _)| *kind == dir)
            .map(|(_, path)| path.clone())
            .or_else(|| {
                self.home
                    .as_ref()
                    .map(|home| home.join(dir.default_name(self.os)))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(os: Os, home: &str) -> Environment {
        Environment {
            home: Some(PathBuf::from(home)),
            user_dirs: Vec::new(),
            now: 0,
            os,
            system_root: None,
        }
    }

    #[test]
    fn the_videos_folder_is_called_movies_on_macos_and_videos_elsewhere() {
        assert_eq!(UserDir::Videos.default_name(Os::MacOs), "Movies");
        for os in [Os::Linux, Os::Windows, Os::Other] {
            assert_eq!(UserDir::Videos.default_name(os), "Videos");
        }
    }

    #[test]
    fn the_other_personal_folders_keep_their_names_on_every_os() {
        for os in [Os::Linux, Os::MacOs, Os::Windows, Os::Other] {
            assert_eq!(UserDir::Desktop.default_name(os), "Desktop");
            assert_eq!(UserDir::Documents.default_name(os), "Documents");
            assert_eq!(UserDir::Music.default_name(os), "Music");
            assert_eq!(UserDir::Pictures.default_name(os), "Pictures");
        }
    }

    #[test]
    fn a_personal_folder_without_a_configured_place_is_below_the_home_under_the_name_of_the_os() {
        let mac = environment(Os::MacOs, "/Users/u");
        let linux = environment(Os::Linux, "/home/u");

        assert_eq!(
            mac.user_dir(UserDir::Videos),
            Some(PathBuf::from("/Users/u/Movies"))
        );
        assert_eq!(
            linux.user_dir(UserDir::Videos),
            Some(PathBuf::from("/home/u/Videos"))
        );
    }

    #[test]
    fn a_configured_place_wins_over_the_default_name() {
        let mut mac = environment(Os::MacOs, "/Users/u");
        mac.user_dirs
            .push((UserDir::Videos, PathBuf::from("/Volumes/Media/Films")));

        assert_eq!(
            mac.user_dir(UserDir::Videos),
            Some(PathBuf::from("/Volumes/Media/Films"))
        );
    }
}
