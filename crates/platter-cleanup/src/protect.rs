//! Places cleanup never touches, whatever a rule says.

use std::time::Duration;

use platter_core::{EntryProbe, Environment, UserDir};

use crate::look::{Looked, look_up};
use crate::path::PathStyle;

/// How long looking the protected places up on the disk may take in all, for one plan request: the
/// places share it as the kept folders share [`KEPT_LEARNING_BUDGET`](crate::KEPT_LEARNING_BUDGET).
pub const PROTECTED_LEARNING_BUDGET: Duration = Duration::from_secs(3);

/// System folders that may be neither a target nor have a target above them.
const SYSTEM_FOLDERS: &[&str] = &[
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/home",
    "/lib",
    "/lib32",
    "/lib64",
    "/mnt",
    "/media",
    "/opt",
    "/proc",
    "/root",
    "/run",
    "/sbin",
    "/srv",
    "/sys",
    "/tmp",
    "/usr",
    "/var",
    "/var/cache",
    "/var/lib",
    "/var/log",
];

/// System folders nothing inside of may be a target.
const SYSTEM_INSIDE: &[&str] = &[
    "/bin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/opt", "/proc", "/root", "/run", "/sbin",
    "/srv", "/sys", "/usr",
];

/// Folders of the home directory nothing inside of may be a target: keys, passwords, settings and
/// browser and mail profiles.
const HOME_INSIDE: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".password-store",
    ".pki",
    ".config",
    ".local/share/keyrings",
    ".mozilla",
    ".thunderbird",
    ".local/share/kwalletd",
    ".var",
    "snap",
    ".aws",
    ".kube",
    ".netrc",
    ".git-credentials",
];

/// The system folders of macOS, besides the ones above (`/usr`, `/bin`, `/sbin`, `/opt`, `/etc`, `/var`,
/// `/tmp`, which macOS has too): neither these nor anything above them may be a target. `/private` holds
/// the real `/var`, `/tmp` and `/etc`, which are links to it, so both spellings are listed.
const MAC_SYSTEM_FOLDERS: &[&str] = &[
    "/System",
    "/Library",
    "/Applications",
    "/Users",
    "/Volumes",
    "/cores",
    "/private",
    "/private/etc",
    "/private/tmp",
    "/private/var",
];

/// System folders of macOS nothing inside of may be a target (besides the ones above): the system, the
/// folders shared by all users, and the system's own data below `/private/var` (by both spellings).
const MAC_SYSTEM_INSIDE: &[&str] = &[
    "/System",
    "/Library",
    "/Applications",
    "/cores",
    "/private/etc",
    "/private/var/db",
    "/private/var/protected",
    "/private/var/root",
    "/private/var/vm",
    "/var/db",
    "/var/protected",
    "/var/root",
    "/var/vm",
];

/// Folders of a macOS home (below the home folder, written with `/`) nothing inside of may be a target:
/// the Trash itself, keys and credentials (`Library/Keychains`, `Library/Cookies`), mail and messages,
/// what iCloud and the apps' sandboxes keep (`Mobile Documents`, `CloudStorage`, `Containers`,
/// `Group Containers`), settings, accounts and contacts, the photo library, and the profiles of the
/// browsers and mail programs. The ones of [`HOME_INSIDE`] hold on macOS too.
const MAC_HOME_INSIDE: &[&str] = &[
    ".Trash",
    "Library/Keychains",
    "Library/Mail",
    "Library/Messages",
    "Library/Cookies",
    "Library/Mobile Documents",
    "Library/CloudStorage",
    "Library/Containers",
    "Library/Group Containers",
    "Library/Preferences",
    "Library/Accounts",
    "Library/Calendars",
    "Library/Safari",
    "Library/Application Support/AddressBook",
    "Library/Application Support/MobileSync/Backup",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Chromium",
    "Library/Application Support/Firefox",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Arc",
    "Library/Application Support/Vivaldi",
    "Library/Application Support/com.operasoftware.Opera",
    "Library/Application Support/Thunderbird",
    "Pictures/Photos Library.photoslibrary",
];

/// Folders of a Windows profile (below the profile folder, written with `/`) nothing inside of may be a
/// target: keys and credentials, the settings that follow the user (`AppData\Roaming`), the per-user
/// registry hive, and the profiles of the browsers and mail programs.
const WINDOWS_HOME_INSIDE: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".config",
    ".netrc",
    "_netrc",
    ".git-credentials",
    "AppData/Roaming",
    "AppData/Local/Microsoft/Credentials",
    "AppData/Local/Microsoft/Vault",
    "AppData/Local/Microsoft/Windows",
    "AppData/Local/Google/Chrome/User Data",
    "AppData/Local/Microsoft/Edge/User Data",
    "AppData/Local/BraveSoftware",
    "AppData/Local/Mozilla",
    "AppData/Local/Thunderbird",
];

/// Folders at the root of the system drive that may be neither a target nor have a target above them.
const WINDOWS_ROOT_FOLDERS: &[&str] = &[
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    "Users",
    "Recovery",
    "PerfLogs",
    "Documents and Settings",
];

/// Folders at the root of a drive nothing inside of may be a target (besides the folders above).
const WINDOWS_DRIVE_INSIDE: &[&str] = &["$Recycle.Bin", "System Volume Information", "Recovery"];

/// The folder below `%SystemRoot%` that only an owning tool may clean (`DISM`), by name.
const WINDOWS_TOOL_FOLDER: &str = "WinSxS";

/// The protected places of one machine, as components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protected {
    style: PathStyle,
    /// Neither these nor anything above them may be a target.
    not_or_above: Vec<Vec<Vec<u8>>>,
    /// Nothing at or below these may be a target.
    not_inside: Vec<Vec<Vec<u8>>>,
    /// Folders inside a place that is otherwise closed, which the tool that owns them may clean, and
    /// only that tool (never a plan of Platter's own, never anything inside them). The folders of
    /// `shelters` are what they lie in.
    tool_folders: Vec<Vec<Vec<u8>>>,
    shelters: Vec<Vec<Vec<u8>>>,
    /// The same places by identity (device, inode), so another path to them (a bind mount of the home,
    /// a btrfs top level mounted elsewhere) is recognised too. `inside`: nothing at or below it; else only
    /// the place itself.
    identities: Vec<Identity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    place: String,
    inside: bool,
    /// The place is one the tool folders lie in (see [`Protected`]).
    sheltering: bool,
}

impl Protected {
    /// The protected places for `env`. `None` without a known home directory: then personal folders
    /// cannot be told apart, and cleanup refuses everything.
    pub fn for_environment(env: &Environment) -> Option<Self> {
        let style = PathStyle::for_os(env.os);
        let home = env.home.as_ref()?;
        let home_parts = style.owned(home.as_os_str().as_encoded_bytes())?;

        // The top (`/`) is no home folder, and neither is a drive root.
        let least = if style == PathStyle::Windows { 2 } else { 1 };
        if home_parts.len() < least {
            return None;
        }

        let mut protected = Self {
            style,
            not_or_above: vec![home_parts.clone()],
            not_inside: Vec::new(),
            tool_folders: Vec::new(),
            shelters: Vec::new(),
            identities: Vec::new(),
        };
        match style {
            PathStyle::Posix => protected.add_posix_places(&home_parts),
            PathStyle::Mac => protected.add_mac_places(&home_parts),
            PathStyle::Windows => protected.add_windows_places(env, &home_parts),
        }

        for dir in UserDir::ALL {
            if let Some(parts) = user_dir_parts(env, style, &home_parts, dir) {
                protected.not_or_above.push(parts.clone());
                protected.not_inside.push(parts);
            }
        }

        // The folders above the home and the personal folders, so that they are known by identity too
        // (a second path to `/data` must not reach the `/data/users/u` above a home).
        let mut above: Vec<Vec<Vec<u8>>> = protected
            .not_or_above
            .iter()
            .flat_map(|place| (1..place.len()).map(|length| place[..length].to_vec()))
            .collect();
        // On Windows and macOS the folders above a place that is closed inside (`AppData`,
        // `AppData\Local\Microsoft`, `...\Google\Chrome`; `~/Library`, `~/Library/Application Support`...)
        // may not be targets either: moving one whole takes the closed place with it. On Linux they are
        // left as they are (`~/.local` is no target of a rule, and a home folder's own list stays what it
        // was).
        if style != PathStyle::Posix {
            above.extend(
                protected
                    .not_inside
                    .iter()
                    .flat_map(|place| (1..place.len()).map(|length| place[..length].to_vec())),
            );
        }
        protected.not_or_above.extend(above);
        protected.not_or_above.sort();
        protected.not_or_above.dedup();

        Some(protected)
    }

    fn add_posix_places(&mut self, home_parts: &[Vec<u8>]) {
        // The top itself: `/` has no components.
        self.not_or_above.push(Vec::new());

        for folder in SYSTEM_FOLDERS {
            self.not_or_above.extend(self.style.owned(folder.as_bytes()));
        }
        for folder in SYSTEM_INSIDE {
            self.not_inside.extend(self.style.owned(folder.as_bytes()));
        }
        for folder in HOME_INSIDE {
            self.not_inside.push(below(home_parts, folder, '/'));
        }
    }

    /// The macOS places: everything protected on Linux (the shared system folders are the same: `/usr`,
    /// `/bin`, `/etc`... and the dot folders of a home), the system folders of macOS, and what in the
    /// Library of the user is not the user's to clean. Names are compared without regard to case.
    fn add_mac_places(&mut self, home_parts: &[Vec<u8>]) {
        self.add_posix_places(home_parts);

        for folder in MAC_SYSTEM_FOLDERS {
            self.not_or_above.extend(self.style.owned(folder.as_bytes()));
        }
        for folder in MAC_SYSTEM_INSIDE {
            self.not_inside.extend(self.style.owned(folder.as_bytes()));
        }
        for folder in MAC_HOME_INSIDE {
            self.not_inside.push(below(home_parts, folder, '/'));
        }
    }

    /// The Windows places: the system folder, the program folders, the data folders of programs and
    /// users, the profile and what in it is the user's own, and the special folders of every drive.
    fn add_windows_places(&mut self, env: &Environment, home_parts: &[Vec<u8>]) {
        let style = self.style;
        let drive_of_home = home_parts[0].clone();

        // `%SystemRoot%` as the environment knows it. Without it the usual place on the home drive is
        // protected, which errs toward protecting.
        let system_root = env
            .system_root
            .as_ref()
            .and_then(|root| style.owned(root.as_os_str().as_encoded_bytes()))
            .filter(|root| root.len() >= 2)
            .unwrap_or_else(|| vec![drive_of_home.clone(), b"Windows".to_vec()]);
        let system_drive = system_root[0].clone();

        self.not_or_above.push(system_root.clone());
        self.not_inside.push(system_root.clone());
        self.shelters.push(system_root.clone());
        let mut tool = system_root.clone();
        tool.push(WINDOWS_TOOL_FOLDER.as_bytes().to_vec());
        self.tool_folders.push(tool);

        for folder in WINDOWS_ROOT_FOLDERS {
            self.not_or_above
                .push(below(std::slice::from_ref(&system_drive), folder, '/'));
        }
        for folder in ["Program Files", "Program Files (x86)", "ProgramData"] {
            self.not_inside
                .push(below(std::slice::from_ref(&system_drive), folder, '/'));
        }
        for drive in [&system_drive, &drive_of_home] {
            for folder in WINDOWS_DRIVE_INSIDE {
                self.not_inside
                    .push(below(std::slice::from_ref(drive), folder, '/'));
            }
            self.not_or_above
                .push(below(std::slice::from_ref(drive), "$Recycle.Bin", '/'));
            self.not_or_above.push(below(
                std::slice::from_ref(drive),
                "System Volume Information",
                '/',
            ));
        }

        for folder in WINDOWS_HOME_INSIDE {
            self.not_inside.push(below(home_parts, folder, '/'));
        }
    }

    /// [`Protected::for_environment`] with every place also looked up on the disk, so that another path to
    /// one (a bind mount of the home) is recognised by identity: see [`Protected::refuses_on_disk`]. The
    /// looking up takes [`PROTECTED_LEARNING_BUDGET`] at most.
    pub fn on_disk(env: &Environment, probe: &impl EntryProbe) -> Option<Self> {
        Self::on_disk_within(env, probe, PROTECTED_LEARNING_BUDGET)
    }

    /// [`Protected::on_disk`] with an explicit time limit.
    pub fn on_disk_within(env: &Environment, probe: &impl EntryProbe, budget: Duration) -> Option<Self> {
        let mut protected = Self::for_environment(env)?;
        protected.learn_identities(probe, budget);

        Some(protected)
    }

    /// Looks every protected place up on the disk, so it is also recognised by identity. A place that
    /// does not exist is skipped.
    ///
    /// A place reached through a link (`~/.config -> dotfiles/config`) is protected where it really is:
    /// its resolved path joins the lists, and the identity recorded is that of the folder, not the link.
    ///
    /// All places share `budget` (see [`look_up`]: a few are looked at at a time, and none is started once
    /// it has run out), so a mount that does not answer cannot hold planning for one probe limit per
    /// place. A place not looked at in time is protected by its path alone.
    pub(crate) fn learn_identities(&mut self, probe: &impl EntryProbe, budget: Duration) {
        let style = self.style;
        let places: Vec<(Vec<Vec<u8>>, bool)> = self
            .not_inside
            .iter()
            .map(|place| (place.clone(), true))
            .chain(self.not_or_above.iter().map(|place| (place.clone(), false)))
            .filter(|(place, _)| !place.is_empty())
            .collect();
        let paths: Vec<Vec<u8>> = places.iter().map(|(place, _)| style.join(place)).collect();

        let looked = look_up(probe, style, &paths, budget);

        let mut identities = Vec::new();
        for (((place, inside), path), looked) in places.iter().zip(&paths).zip(looked) {
            let Looked { real, identity } = looked.unwrap_or_default();

            if let Some(real) = real {
                if *inside {
                    // On Windows and macOS the folders above a closed place may not be targets (see
                    // `for_environment`). That holds where the place really is too: `AppData\Roaming`
                    // behind a junction to `D:\Sync\Roaming` makes `D:\Sync` a folder that holds it.
                    if style != PathStyle::Posix {
                        self.not_or_above
                            .extend((1..real.len()).map(|length| real[..length].to_vec()));
                    }
                    self.not_inside.push(real);
                } else {
                    self.not_or_above.push(real);
                }
            }
            if let Some((dev, ino)) = identity {
                identities.push(Identity {
                    dev,
                    ino,
                    place: String::from_utf8_lossy(path).into_owned(),
                    inside: *inside,
                    sheltering: *inside
                        && self
                            .shelters
                            .iter()
                            .any(|shelter| self.same_place(shelter, place)),
                });
            }
        }
        self.identities = identities;
    }

    /// The protected place met on the way to a target, by identity: `way` is the identity of every
    /// component from the top down, the target last. `by_tool_folder`: the target is a folder only its
    /// owning tool may clean and is acted on by that tool (see [`Protected::is_tool_folder`]), so the
    /// places it lies in do not count.
    pub(crate) fn hit_by_identity(&self, way: &[(u64, u64)], by_tool_folder: bool) -> Option<String> {
        let target = way.last()?;
        self.identities
            .iter()
            .filter(|known| !(by_tool_folder && known.sheltering))
            .find(|known| {
                let id = (known.dev, known.ino);
                if known.inside {
                    way.contains(&id)
                } else {
                    *target == id
                }
            })
            .map(|known| known.place.clone())
    }

    /// Whether `target` is a folder that is closed to every plan but one by the tool that owns it (the
    /// component store of Windows, cleaned by `DISM`). Only the folder itself: nothing inside it.
    pub(crate) fn is_tool_folder<A: AsRef<[u8]>>(&self, target: &[A]) -> bool {
        self.tool_folders
            .iter()
            .any(|folder| self.same_place(folder, target))
    }

    fn same_place<A: AsRef<[u8]>, B: AsRef<[u8]>>(&self, a: &[A], b: &[B]) -> bool {
        a.len() == b.len() && self.style.is_within(a, b)
    }

    /// The protected place the exact path `path_bytes` is, holds or lies inside, as a readable path; `None`
    /// when no protected place is hit. This looks at the path only; planning also compares identities.
    /// A path that is not plainly absolute (relative, or with `.`, `..` or an empty part) is refused too.
    pub fn refuses(&self, path_bytes: &[u8]) -> Option<String> {
        match self.style.components(path_bytes) {
            Some(parts) => self.hit(&parts),
            None => Some(crate::path::display(path_bytes)),
        }
    }

    /// [`Protected::refuses`], and also the places the folder at `path_bytes` is, or lies inside, by
    /// identity (device and inode, read through `probe` now): a second path to the home folder or to a
    /// place inside it is refused like the first. Only meaningful for a value made by
    /// [`Protected::on_disk`]. A path that cannot be looked up adds nothing to the answer of `refuses`.
    pub fn refuses_on_disk(&self, probe: &impl EntryProbe, path_bytes: &[u8]) -> Option<String> {
        if let Some(place) = self.refuses(path_bytes) {
            return Some(place);
        }

        let way: Vec<(u64, u64)> = probe
            .probe(path_bytes)
            .ok()?
            .iter()
            .map(|entry| (entry.meta.dev, entry.meta.ino))
            .collect();

        self.hit_by_identity(&way, false)
    }

    /// The protected place `target` hits, if any, as a readable path.
    pub(crate) fn hit<A: AsRef<[u8]>>(&self, target: &[A]) -> Option<String> {
        self.hit_for(target, false)
    }

    /// [`Protected::hit`] for a target that its owning tool cleans (`by_tool`): a folder that only a tool
    /// may clean is let through then (see [`Protected::is_tool_folder`]).
    pub(crate) fn hit_for<A: AsRef<[u8]>>(&self, target: &[A], by_tool: bool) -> Option<String> {
        // A drive root, or something above one, is never a target.
        if self.style == PathStyle::Windows && target.len() <= 1 {
            let parts: Vec<Vec<u8>> = target.iter().map(|part| part.as_ref().to_vec()).collect();

            return Some(self.style.show(&parts));
        }

        // The bin and the volume information of any drive, whichever drives this machine has.
        if self.style == PathStyle::Windows
            && let Some(name) = target.get(1)
            && WINDOWS_DRIVE_INSIDE
                .iter()
                .any(|special| self.style.same_name(name.as_ref(), special.as_bytes()))
        {
            let parts: Vec<Vec<u8>> = target[..2].iter().map(|part| part.as_ref().to_vec()).collect();

            return Some(self.style.show(&parts));
        }

        let sheltered = by_tool && self.is_tool_folder(target);
        let above = self
            .not_or_above
            .iter()
            .find(|place| self.style.is_within(place, target));
        let inside = self.not_inside.iter().find(|place| {
            self.style.is_within(target, place)
                && !(sheltered
                    && self
                        .shelters
                        .iter()
                        .any(|shelter| self.same_place(shelter, place)))
        });

        above.or(inside).map(|place| self.style.show(place))
    }
}

/// `names` (parts of a path, joined with `separator`) below the folder `base`.
fn below(base: &[Vec<u8>], names: &str, separator: char) -> Vec<Vec<u8>> {
    let mut parts = base.to_vec();
    parts.extend(names.split(separator).map(|part| part.as_bytes().to_vec()));

    parts
}

/// Where the personal folder `dir` is: as the environment says, else under its default name below the
/// home folder. An entry with an empty path is the "missing" mark of Downloads and says nothing about
/// where a protected folder is, so it counts as no entry. Joined by parts, so a Windows environment is
/// read right on any OS.
fn user_dir_parts(
    env: &Environment,
    style: PathStyle,
    home_parts: &[Vec<u8>],
    dir: UserDir,
) -> Option<Vec<Vec<u8>>> {
    let listed = env
        .user_dirs
        .iter()
        .find(|(kind, path)| *kind == dir && !path.as_os_str().is_empty());

    if let Some((_, path)) = listed {
        return style.owned(path.as_os_str().as_encoded_bytes());
    }

    let mut parts = home_parts.to_vec();
    parts.push(dir.default_name(env.os).as_bytes().to_vec());

    Some(parts)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io;
    use std::path::PathBuf;

    use platter_core::{EntryKind, EntryMeta, Os, ProbedEntry};

    use super::*;

    const STYLE: PathStyle = PathStyle::Windows;

    fn environment() -> Environment {
        Environment {
            home: Some(PathBuf::from("C:\\Users\\Me")),
            user_dirs: Vec::new(),
            now: 0,
            os: Os::Windows,
            system_root: Some(PathBuf::from("C:\\Windows")),
        }
    }

    fn parts(path: &str) -> Vec<Vec<u8>> {
        STYLE.owned(path.as_bytes()).unwrap()
    }

    #[test]
    fn only_the_tool_that_owns_the_component_store_may_clean_it_and_nothing_inside() {
        let protected = Protected::for_environment(&environment()).unwrap();

        let store = parts("C:\\Windows\\WinSxS");
        assert!(protected.hit_for(&store, false).is_some());
        assert!(protected.hit_for(&store, true).is_none());
        assert!(protected.hit_for(&parts("c:\\WINDOWS\\winsxs"), true).is_none());

        for other in [
            "C:\\Windows\\WinSxS\\amd64_x",
            "C:\\Windows\\System32",
            "C:\\Windows",
            "C:\\Windows\\Installer",
            "C:\\Program Files",
        ] {
            assert!(protected.hit_for(&parts(other), true).is_some(), "{other}");
        }
    }

    #[test]
    fn an_empty_path_listed_for_a_personal_folder_does_not_unprotect_it() {
        let mut env = environment();
        env.user_dirs.push((UserDir::Documents, PathBuf::new()));

        let protected = Protected::for_environment(&env).unwrap();

        for target in ["C:\\Users\\Me\\Documents", "C:\\Users\\Me\\Documents\\work"] {
            assert!(protected.hit(&parts(target)).is_some(), "{target}");
        }
    }

    /// A disk that is a table of paths and what each component is: (device, inode).
    struct Disk(HashMap<Vec<u8>, Vec<(u64, u64)>>);

    impl EntryProbe for Disk {
        fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
            let way = self.0.get(path).ok_or(io::ErrorKind::NotFound)?;

            Ok(way
                .iter()
                .map(|(dev, ino)| ProbedEntry {
                    meta: EntryMeta {
                        kind: EntryKind::Dir,
                        allocated: 0,
                        apparent: 0,
                        mtime: 0,
                        dev: *dev,
                        ino: *ino,
                        nlink: 1,
                        is_mount_root: false,
                    },
                    mount_id: None,
                    birth: None,
                })
                .collect())
        }
    }

    #[test]
    fn a_short_name_for_a_protected_folder_is_caught_by_its_identity() {
        let disk = Disk(HashMap::from([
            (b"C:\\Program Files".to_vec(), vec![(1, 1), (1, 50)]),
            (b"C:\\PROGRA~1\\App".to_vec(), vec![(1, 1), (1, 50), (1, 60)]),
            (
                b"C:\\Users\\Me\\code".to_vec(),
                vec![(1, 1), (1, 2), (1, 3), (1, 70)],
            ),
        ]));
        let protected = Protected::on_disk(&environment(), &disk).unwrap();

        // The spelling gives nothing away, the folder it leads through does.
        assert_eq!(protected.refuses(b"C:\\PROGRA~1\\App"), None);
        assert_eq!(
            protected.refuses_on_disk(&disk, b"C:\\PROGRA~1\\App").as_deref(),
            Some("C:\\Program Files")
        );
        assert_eq!(protected.refuses_on_disk(&disk, b"C:\\Users\\Me\\code"), None);
    }

    /// A disk on which some places are reached through a junction: `resolve` gives where they really are.
    struct Linked(Disk, HashMap<Vec<u8>, Vec<u8>>);

    impl EntryProbe for Linked {
        fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
            self.0.probe(path)
        }

        fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
            Ok(self.1.get(path).cloned().unwrap_or_else(|| path.to_vec()))
        }
    }

    #[test]
    fn the_folders_above_a_closed_place_behind_a_junction_are_refused_where_it_really_is() {
        let roaming = b"C:\\Users\\Me\\AppData\\Roaming".to_vec();
        let disk = Linked(
            Disk(HashMap::from([(
                roaming.clone(),
                vec![(1, 1), (1, 2), (1, 3), (1, 4), (2, 90)],
            )])),
            HashMap::from([(roaming, b"D:\\Sync\\Roaming".to_vec())]),
        );
        let protected = Protected::on_disk(&environment(), &disk).unwrap();

        assert!(protected.refuses(b"D:\\Sync").is_some());
        assert!(protected.refuses(b"D:\\Sync\\Roaming\\x").is_some());
        assert_eq!(protected.refuses(b"D:\\Other"), None);
    }

    fn mac_environment() -> Environment {
        Environment {
            home: Some(PathBuf::from("/Users/me")),
            os: Os::MacOs,
            system_root: None,
            ..environment()
        }
    }

    #[test]
    fn on_a_mac_the_folders_above_a_closed_place_are_refused_where_it_really_is() {
        let keychains = b"/Users/me/Library/Keychains".to_vec();
        let disk = Linked(
            Disk(HashMap::from([(
                keychains.clone(),
                vec![(1, 1), (1, 2), (1, 3), (1, 4), (2, 90)],
            )])),
            HashMap::from([(keychains, b"/Volumes/Sync/Keys".to_vec())]),
        );
        let protected = Protected::on_disk(&mac_environment(), &disk).unwrap();

        assert!(protected.refuses(b"/Volumes/Sync").is_some());
        assert!(protected.refuses(b"/Volumes/Sync/Keys/x").is_some());
        assert_eq!(protected.refuses(b"/Volumes/Other"), None);
    }

    #[test]
    fn on_a_mac_another_path_to_the_library_is_refused_by_its_identity() {
        let disk = Disk(HashMap::from([
            (
                b"/Users/me/Library".to_vec(),
                vec![(1, 1), (1, 2), (1, 3), (1, 30)],
            ),
            (b"/Volumes/Alias/Library".to_vec(), vec![(1, 1), (1, 9), (1, 30)]),
        ]));
        let protected = Protected::on_disk(&mac_environment(), &disk).unwrap();

        assert_eq!(protected.refuses(b"/Volumes/Alias/Library"), None);
        assert!(
            protected
                .refuses_on_disk(&disk, b"/Volumes/Alias/Library")
                .is_some()
        );
    }

    #[test]
    fn a_folder_that_only_a_tool_may_clean_is_not_caught_by_the_identity_of_the_folder_it_lies_in() {
        let disk = Disk(HashMap::from([
            (b"C:\\Windows".to_vec(), vec![(1, 1), (1, 40)]),
            (b"C:\\Windows\\WinSxS".to_vec(), vec![(1, 1), (1, 40), (1, 41)]),
        ]));
        let protected = Protected::on_disk(&environment(), &disk).unwrap();
        let way = [(1, 1), (1, 40), (1, 41)];

        assert!(protected.hit_by_identity(&way, false).is_some());
        assert!(protected.hit_by_identity(&way, true).is_none());
    }
}
