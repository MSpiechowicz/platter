//! The folders the user keeps, as the guard needs to know them.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use platter_core::{EntryProbe, Environment, UserRule, UserVerdict};
use platter_rules::RuleSet;

use crate::look::{KeepLook, Looked, look_up_keeps};
use crate::path::PathStyle;
use crate::protect::Protected;

/// How long learning the identities of the kept folders may take in all, for one plan request.
///
/// Each look at the disk can wait up to the probe's own limit on a mount that does not answer, so
/// without one overall limit a long list of keeps on a hung mount would hold planning for as many
/// limits as there are keeps.
pub const KEPT_LEARNING_BUDGET: Duration = Duration::from_secs(3);

/// One folder the user keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Keep {
    /// Components of the folder as the rule says it.
    path: Vec<Vec<u8>>,
    /// Components of where it really is, when it is reached through a link.
    real: Option<Vec<Vec<u8>>>,
    /// Device and inode of the folder (of where it really is), when it could be looked up in time.
    identity: Option<(u64, u64)>,
    /// Whether where the folder really is could not be settled in time although it is, or may be, reached
    /// through a link (see [`KeptFolders::unresolved`]).
    unresolved: bool,
}

/// Every folder the user keeps, by path and, as far as the disk could tell in time, by identity.
///
/// The rules match a kept folder by its path. This lets the guard also refuse another path to the same
/// folder (a bind mount, a btrfs top level mounted elsewhere), and any target that is, holds or lies in a
/// kept folder by path, whatever kind of node the scan saw there (a skipped mount, say).
///
/// Learn it once for a plan request and hand it to every plan made for that request (see
/// [`plan_cleanup_with`](crate::plan_cleanup_with) and [`plan_labelled_with`](crate::plan_labelled_with)).
/// It also holds what the first of those plans learned about the protected places, so the later ones do
/// not look them up again.
#[derive(Debug, Clone, Default)]
pub struct KeptFolders {
    /// The syntax the folders are written in (see [`PathStyle`]); plans of another style refuse everything.
    style: PathStyle,
    folders: Vec<Keep>,
    /// The protected places of the environment the plans of this request are made for, looked up.
    protected: OnceLock<(Environment, Option<Protected>)>,
}

/// The learned folders are what counts; what a plan has remembered beside them is not part of it.
impl PartialEq for KeptFolders {
    fn eq(&self, other: &Self) -> bool {
        self.folders == other.folders
    }
}

impl Eq for KeptFolders {}

impl KeptFolders {
    /// Learns the kept folders of `rules` through `probe`, within [`KEPT_LEARNING_BUDGET`], for paths in
    /// the syntax of the OS this program was built for.
    pub fn learn(rules: &RuleSet, probe: &impl EntryProbe) -> Self {
        Self::learn_within(rules, probe, KEPT_LEARNING_BUDGET)
    }

    /// [`KeptFolders::learn`] for paths of `style` (the style of the environment the plans are made for).
    pub fn learn_for(style: PathStyle, rules: &RuleSet, probe: &impl EntryProbe) -> Self {
        Self::learn_within_for(style, rules, probe, KEPT_LEARNING_BUDGET)
    }

    /// [`KeptFolders::learn`] with an explicit time limit.
    ///
    /// The paths are always all known. First every folder's own path is read link by link, by text alone
    /// (`read_link` of each component, a link's text followed up to a small number of links, nothing the
    /// links lead to is touched), many at a time and before any other look, so that where a link keep
    /// leads is known even when mounts ahead of it in the file do not answer. Then, for each folder, one
    /// resolve (where a link leads, when the text could not tell) and one probe (its device and inode) are
    /// made, a few folders at a time: a folder on a mount that does not answer holds one place only, and the
    /// folders behind it are still looked at. Once `budget` has run out no look is started: the folders
    /// not looked at are left to their paths alone (another path to one of them is then not recognised),
    /// and a folder whose resolve was done but not its probe is known by where it really is, not by
    /// identity. Planning can run over the limit by at most one look, which the probe itself bounds.
    ///
    /// A folder that is, or lies behind, a link whose destination could not be worked out within the
    /// limit, or that the limit ran out before it was even read (it could be such a link), makes the whole
    /// set [`unresolved`](KeptFolders::unresolved): no plan is made from it.
    ///
    /// A keep folder cannot be skipped for being unrelated to the scan by path: it is exactly another
    /// path (a bind mount below the scan root of a folder outside it) that the identity is for. A folder
    /// with no components (`/`) holds everything and is not a folder anyone keeps: it is left out, as is
    /// a link that leads to `/`.
    pub fn learn_within(rules: &RuleSet, probe: &impl EntryProbe, budget: Duration) -> Self {
        Self::learn_within_for(PathStyle::host(), rules, probe, budget)
    }

    /// [`KeptFolders::learn_within`] for paths of `style`. A folder that is not an absolute path of that
    /// style is left out, as any folder that is not plainly absolute is.
    pub fn learn_within_for(
        style: PathStyle,
        rules: &RuleSet,
        probe: &impl EntryProbe,
        budget: Duration,
    ) -> Self {
        let mut keeps: Vec<(Vec<Vec<u8>>, Vec<u8>)> = Vec::new();
        for rule in rules
            .user()
            .iter()
            .filter(|rule| rule.verdict == UserVerdict::Keep)
        {
            let bytes = rule.folder.as_os_str().as_encoded_bytes();
            let Some(path) = style.owned(bytes).filter(|path| !path.is_empty()) else {
                continue;
            };
            if keeps.iter().all(|(known, _)| *known != path) {
                keeps.push((path, bytes.to_vec()));
            }
        }

        let paths: Vec<Vec<u8>> = keeps.iter().map(|(_, bytes)| bytes.clone()).collect();
        let looked = look_up_keeps(probe, style, &paths, budget);

        let mut kept = Self {
            style,
            ..Self::default()
        };
        for ((path, _), KeepLook { looked, settled }) in keeps.into_iter().zip(looked) {
            let Looked { real, identity } = looked;
            kept.folders.push(Keep {
                path,
                real,
                identity,
                unresolved: !settled,
            });
        }

        kept
    }

    /// These folders for a user whose home is `home`: where a link leads (its path and identity) is
    /// left out when that is `/`, the home folder or a folder above it, because a keep there would hold
    /// every plan and the rule engine never makes such a place a target. The link itself stays kept, so a
    /// keep that is a link to the home folder protects the link and not the whole home.
    ///
    /// Where a link leads is kept when it is, holds or lies below the scan root: a scan that starts
    /// there is a scan of a kept folder (see [`KeptFolders::holds_scan`]).
    pub fn anchored(&self, home: Option<&Path>) -> Self {
        let style = self.style;
        let home = home.and_then(|home| style.owned(home.as_os_str().as_encoded_bytes()));
        // The top and anything above the home folder hold every plan. (A drive root of another drive does
        // not: keeping a whole data drive is a choice a user can make.)
        let too_wide = |real: &[Vec<u8>]| {
            real.is_empty() || home.as_deref().is_some_and(|home| style.is_within(home, real))
        };

        let mut anchored = self.clone();
        anchored.protected = OnceLock::new();
        for keep in &mut anchored.folders {
            if keep.real.as_deref().is_some_and(too_wide) {
                keep.real = None;
                keep.identity = None;
            }
        }

        anchored
    }

    /// Whether a folder the user keeps may be somewhere that is not known: it is, or lies behind, a link
    /// that could not be followed within the learning limit (a disk may not be answering), or the limit
    /// ran out before the folder was reached. What it protects is then not known, so a plan made from
    /// these folders would be a plan that might clean a kept folder through its real path: planning
    /// refuses every target instead (see [`Refusal::KeptUnresolved`](crate::Refusal::KeptUnresolved)).
    pub fn unresolved(&self) -> bool {
        self.folders.iter().any(|keep| keep.unresolved)
    }

    /// Whether the scan of `root` starts in a folder these keeps protect where a link leads: the real
    /// folder of a keep is the scan root or holds it. Like a scan inside a literal keep, nothing in such
    /// a scan may be cleaned or suggested.
    pub fn holds_scan(&self, root: &Path) -> bool {
        let Some(root) = self.style.owned(root.as_os_str().as_encoded_bytes()) else {
            return false;
        };

        self.folders
            .iter()
            .filter_map(|keep| keep.real.as_deref())
            .any(|real| self.style.is_within(&root, real))
    }

    /// Where a kept folder really is, for every one that is reached through a link (those whose real path
    /// is not valid UTF-8 are left out).
    pub fn resolved_folders(&self) -> Vec<PathBuf> {
        self.folders
            .iter()
            .filter_map(|keep| keep.real.as_deref())
            .filter_map(|real| String::from_utf8(self.style.join(real)).ok())
            .map(PathBuf::from)
            .collect()
    }

    /// The protected places of `env` as they are on the disk, looked up once for the request.
    pub(crate) fn protected_places(&self, env: &Environment, probe: &impl EntryProbe) -> Option<Protected> {
        let (made_for, protected) = self.protected.get_or_init(|| {
            let protected = Protected::on_disk(env, probe);

            (env.clone(), protected)
        });
        if made_for == env {
            return protected.clone();
        }

        Protected::on_disk(env, probe)
    }

    /// Whether `identity` is that of a kept folder.
    pub(crate) fn has_identity(&self, identity: &(u64, u64)) -> bool {
        self.folders
            .iter()
            .any(|keep| keep.identity.as_ref() == Some(identity))
    }

    /// The syntax of the paths these folders are written in.
    pub(crate) fn style(&self) -> PathStyle {
        self.style
    }

    /// Whether the folder at `parts` is, holds or lies inside a kept folder, by path alone.
    pub(crate) fn touches<A: AsRef<[u8]>>(&self, parts: &[A]) -> bool {
        self.folders.iter().any(|keep| {
            let real = keep.real.as_deref();

            std::iter::once(keep.path.as_slice())
                .chain(real)
                .any(|kept| self.style.is_within(parts, kept) || self.style.is_within(kept, parts))
        })
    }
}

/// `rules` with each folder the user keeps that is reached through a link also kept where it really is,
/// for a user whose home is `home` (see [`KeptFolders::anchored`]). A scan that starts in such a real
/// folder gets no suggestions: the folder is kept like a literal keep that holds the scan.
///
/// The rules match a folder by its path, and a plan refuses what lies in the real folder as well (see
/// [`KeptFolders`]), so a suggestion made without this would list a folder the plan then refuses. Where
/// a link leads is looked up once, within [`KEPT_LEARNING_BUDGET`]; a folder that could not be looked up
/// in time stays kept by its own path alone.
pub fn with_real_keeps(rules: RuleSet, home: Option<&Path>, probe: &impl EntryProbe) -> RuleSet {
    with_real_keeps_for(PathStyle::host(), rules, home, probe)
}

/// [`with_real_keeps`] for paths of `style`.
pub fn with_real_keeps_for(
    style: PathStyle,
    rules: RuleSet,
    home: Option<&Path>,
    probe: &impl EntryProbe,
) -> RuleSet {
    let kept = KeptFolders::learn_for(style, &rules, probe).anchored(home);

    let mut user = rules.user().to_vec();
    let mut next_id = user.iter().map(|rule| rule.id).max().unwrap_or(0);
    for folder in kept.resolved_folders() {
        let known = user
            .iter()
            .any(|rule| rule.verdict == UserVerdict::Keep && rule.folder == folder);
        if known {
            continue;
        }

        next_id = next_id.saturating_add(1);
        user.push(UserRule {
            id: next_id,
            verdict: UserVerdict::Keep,
            folder,
        });
    }

    rules.with_user(user)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io;

    use platter_core::{EntryKind, EntryMeta, ProbedEntry};

    use super::*;

    const WINDOWS: PathStyle = PathStyle::Windows;

    /// A disk with a table of paths (what each component is) and of canonical paths.
    #[derive(Default)]
    struct Disk {
        ways: HashMap<Vec<u8>, Vec<(u64, u64)>>,
        real: HashMap<Vec<u8>, Vec<u8>>,
    }

    impl EntryProbe for Disk {
        fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
            let way = self.ways.get(path).ok_or(io::ErrorKind::NotFound)?;

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

        fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
            self.real
                .get(path)
                .cloned()
                .ok_or_else(|| io::ErrorKind::Unsupported.into())
        }
    }

    fn keeping(folders: &[&str]) -> RuleSet {
        RuleSet::builtin().with_user(
            folders
                .iter()
                .enumerate()
                .map(|(id, folder)| UserRule {
                    id: id as u32 + 1,
                    verdict: UserVerdict::Keep,
                    folder: PathBuf::from(folder),
                })
                .collect(),
        )
    }

    fn parts(path: &str) -> Vec<Vec<u8>> {
        WINDOWS.owned(path.as_bytes()).unwrap()
    }

    #[test]
    fn a_kept_folder_is_matched_without_regard_to_case_on_windows() {
        let kept = KeptFolders::learn_for(WINDOWS, &keeping(&["c:\\users\\me\\Keep"]), &Disk::default());

        assert!(kept.touches(&parts("C:\\Users\\Me\\keep\\x")));
        assert!(kept.touches(&parts("C:\\Users\\Me\\KEEP")));
        assert!(
            kept.touches(&parts("C:\\Users\\Me")),
            "what holds it is touched too"
        );
        assert!(!kept.touches(&parts("C:\\Users\\Me\\keeper")));
        assert!(!kept.touches(&parts("D:\\Users\\Me\\keep")));
        assert!(!kept.unresolved());
    }

    #[test]
    fn a_kept_folder_behind_a_junction_is_kept_where_it_really_is_and_by_identity() {
        let disk = Disk {
            ways: HashMap::from([(b"D:\\real\\sub".to_vec(), vec![(2, 1), (2, 10), (2, 11)])]),
            real: HashMap::from([(b"C:\\link\\sub".to_vec(), b"D:\\real\\sub".to_vec())]),
        };

        let kept = KeptFolders::learn_for(WINDOWS, &keeping(&["C:\\link\\sub"]), &disk)
            .anchored(Some(Path::new("C:\\Users\\Me")));

        assert!(kept.touches(&parts("D:\\real\\sub\\file")), "where it really is");
        assert!(
            kept.touches(&parts("C:\\link\\sub\\file")),
            "and where it was written"
        );
        assert!(kept.has_identity(&(2, 11)));
        assert_eq!(kept.resolved_folders(), [PathBuf::from("D:\\real\\sub")]);
        assert!(kept.holds_scan(Path::new("d:\\REAL\\sub\\deeper")));
    }

    #[test]
    fn the_canonical_spelling_of_a_folder_is_not_another_place() {
        let disk = Disk {
            ways: HashMap::new(),
            real: HashMap::from([(b"c:\\users\\me\\keep".to_vec(), b"C:\\Users\\Me\\Keep".to_vec())]),
        };

        let kept = KeptFolders::learn_for(WINDOWS, &keeping(&["c:\\users\\me\\keep"]), &disk);

        assert_eq!(kept.resolved_folders(), Vec::<PathBuf>::new());
    }

    #[test]
    fn a_folder_that_is_not_a_path_of_the_style_is_left_out() {
        let kept = KeptFolders::learn_for(
            WINDOWS,
            &keeping(&["/home/me/keep", "relative", "C:\\ok"]),
            &Disk::default(),
        );

        assert!(!kept.touches(&parts("C:\\other")));
        assert!(kept.touches(&parts("C:\\ok\\x")));
        assert_eq!(kept.style(), WINDOWS);
    }

    #[test]
    fn keeping_a_whole_data_drive_is_allowed() {
        let kept = KeptFolders::learn_for(WINDOWS, &keeping(&["D:\\"]), &Disk::default());

        assert!(kept.touches(&parts("D:\\anything\\at\\all")));
        assert!(!kept.touches(&parts("C:\\anything")));
    }
}
