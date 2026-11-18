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
    Downloads,
}

impl UserDir {
    /// The folders that hold the user's own work and that the cleanup never touches. `Downloads` is not
    /// one of them: old downloads are what the user may want to clean, so it is not on this list (the
    /// cleanup deny-list is built from it).
    pub const ALL: [UserDir; 5] = [
        UserDir::Desktop,
        UserDir::Documents,
        UserDir::Music,
        UserDir::Pictures,
        UserDir::Videos,
    ];

    /// Every personal folder there is, `Downloads` included: for finding where each one is, never for
    /// deciding what is protected.
    pub const EVERY: [UserDir; 6] = [
        UserDir::Desktop,
        UserDir::Documents,
        UserDir::Music,
        UserDir::Pictures,
        UserDir::Videos,
        UserDir::Downloads,
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
            (UserDir::Downloads, _) => "Downloads",
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
    /// configured them. A folder that is not listed is assumed under its default name in `home`. An entry
    /// with an empty path says the folder does not exist on this machine (see
    /// [`Environment::set_downloads_missing`]). Only `Downloads` is ever marked so: the cleanup
    /// deny-list ignores an empty path listed for a folder of [`UserDir::ALL`].
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
    /// home directory. `None` when neither is known, and when the folder was marked missing.
    pub fn user_dir(&self, dir: UserDir) -> Option<PathBuf> {
        let configured = self.user_dirs.iter().find(|(kind, _)| *kind == dir);

        if let Some((_, path)) = configured {
            return (!path.as_os_str().is_empty()).then(|| path.clone());
        }

        self.home
            .as_ref()
            .map(|home| home.join(dir.default_name(self.os)))
    }

    /// Says that Downloads does not exist here: [`Environment::user_dir`] returns `None` for it instead of
    /// the default name, so no rule looks for it. Takes no folder, so a personal folder of
    /// [`UserDir::ALL`] can never be unprotected this way: the cleanup deny-list is built from `ALL`.
    pub fn set_downloads_missing(&mut self) {
        self.user_dirs.retain(|(kind, _)| *kind != UserDir::Downloads);
        self.user_dirs.push((UserDir::Downloads, PathBuf::new()));
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
            assert_eq!(UserDir::Downloads.default_name(os), "Downloads");
        }
    }

    #[test]
    fn every_folder_lists_all_of_them_and_all_leaves_downloads_out() {
        assert!(!UserDir::ALL.contains(&UserDir::Downloads));
        assert!(UserDir::EVERY.contains(&UserDir::Downloads));
        assert!(UserDir::ALL.iter().all(|dir| UserDir::EVERY.contains(dir)));
    }

    #[test]
    fn downloads_follows_its_configured_place_and_defaults_below_the_home() {
        let mut linux = environment(Os::Linux, "/home/u");
        assert_eq!(
            linux.user_dir(UserDir::Downloads),
            Some(PathBuf::from("/home/u/Downloads"))
        );

        linux
            .user_dirs
            .push((UserDir::Downloads, PathBuf::from("/home/u/Pobrane")));
        assert_eq!(
            linux.user_dir(UserDir::Downloads),
            Some(PathBuf::from("/home/u/Pobrane"))
        );
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

    #[test]
    fn a_folder_marked_missing_has_no_place_even_with_a_known_home() {
        let mut linux = environment(Os::Linux, "/home/u");
        linux
            .user_dirs
            .push((UserDir::Downloads, PathBuf::from("/home/u/Pobrane")));

        linux.set_downloads_missing();

        assert_eq!(linux.user_dir(UserDir::Downloads), None);
        assert_eq!(
            linux.user_dir(UserDir::Documents),
            Some(PathBuf::from("/home/u/Documents"))
        );
    }
}
