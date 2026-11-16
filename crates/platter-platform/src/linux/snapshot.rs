//! The mount table as it was when the scan started, and the decision "may the walk enter this mount?".

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use platter_core::SkipReason;
use rustix::fs::{major, minor};

use super::exclusions::ExclusionRules;
use super::mountinfo::MountEntry;
use super::same_fs::Decision;

/// Which mounts exist, which the rules exclude and, when the scan stays on one disk (or has to avoid
/// counting a filesystem twice), which of them the walk may enter (see [`super::same_fs`]).
///
/// A mount id that is not in the table appeared after the snapshot (an autofs or
/// `x-systemd.automount` point that mounted itself, something mounted while scanning). It was never
/// classified, so it is not entered.
///
/// # Reused ids
///
/// The kernel hands a mount id out again after the mount is gone. A mount that appears during the scan
/// could therefore carry the id of a mount that was classified as fine, and would inherit that verdict.
/// Each id is bound to the mount point it had in the table: a mount root met by the walk is entered only
/// when it sits at exactly that path. (The kernel's unique 64-bit mount id, `STATX_MNT_ID_UNIQUE` since
/// 6.8, would be stronger, but `/proc/self/mountinfo` does not list it and rustix has no `statmount`;
/// so the path is the check.) What remains is a different mount that appears at the very same path with
/// a reused id, which this cannot tell from the original.
#[derive(Debug, Clone, Default)]
pub struct MountSnapshot {
    /// Mount id to the mount point it had when the table was read.
    points: HashMap<u64, PathBuf>,
    excluded: HashSet<u64>,
    /// `Some` when only some mounts may be entered: what `same_fs` decided. `None` when every mount the
    /// rules allow may be (the mount table said nothing about double counting).
    decision: Option<Decision>,
    /// For kernels without mount ids: `major:minor` of every mount in the table, with the ids that have it.
    by_device: HashMap<String, Vec<u64>>,
}

impl MountSnapshot {
    /// `decision` is what `same_fs::enterable_mounts` decided; `None` leaves the choice to the exclusion
    /// rules alone.
    pub fn new(rules: &ExclusionRules, mounts: &[MountEntry], decision: Option<Decision>) -> Self {
        let mut by_device: HashMap<String, Vec<u64>> = HashMap::new();
        for mount in mounts {
            by_device
                .entry(mount.device.clone())
                .or_default()
                .push(mount.mount_id);
        }

        Self {
            points: mounts
                .iter()
                .map(|mount| (mount.mount_id, mount.mount_point.clone()))
                .collect(),
            excluded: rules.excluded_mount_ids(mounts),
            decision,
            by_device,
        }
    }

    /// Whether the walk may enter the mount root with kernel mount id `mount_id` that it met at `path`
    /// (the scan root's path followed by the names walked, as the process sees it).
    pub fn may_enter(&self, mount_id: u64, path: &Path) -> bool {
        self.verdict(mount_id, path).is_ok()
    }

    /// The same decision with the reason when the answer is no.
    ///
    /// - `Unknown`: the mount id is not in the table, or it is there for another mount point (see
    ///   "Reused ids"); nothing is known about this mount.
    /// - `System`: the rules exclude the mount (pseudo, virtual, network, FUSE).
    /// - `Duplicate`: a mount of a filesystem the scan covers already (see [`super::same_fs`]).
    /// - `OtherDisk`: another filesystem, with `one_file_system`.
    pub fn verdict(&self, mount_id: u64, path: &Path) -> Result<(), SkipReason> {
        if self.points.get(&mount_id).is_none_or(|point| point != path) {
            return Err(SkipReason::Unknown);
        }
        if self.excluded.contains(&mount_id) {
            return Err(SkipReason::System);
        }

        let Some(decision) = &self.decision else {
            return Ok(());
        };
        if decision.enter.contains(&mount_id) {
            Ok(())
        } else if decision.duplicate.contains(&mount_id) {
            Err(SkipReason::Duplicate)
        } else if decision.other_disk.contains(&mount_id) {
            Err(SkipReason::OtherDisk)
        } else {
            Err(SkipReason::Unknown)
        }
    }

    /// The same decision for a kernel that reports no mount ids (before 5.8): the mount root is known
    /// only by the `st_dev` of its files, as `major:minor` (see [`device_text`]). It may be entered when
    /// a mount of the table with that very device, at that very path, may be.
    pub fn may_enter_by_device(&self, device: &str, path: &Path) -> bool {
        self.verdict_by_device(device, path).is_ok()
    }

    /// [`MountSnapshot::verdict`] for a mount known by its device: the first of the mounts with that
    /// device and mount point that may be entered decides, else the reason of the first.
    pub fn verdict_by_device(&self, device: &str, path: &Path) -> Result<(), SkipReason> {
        let mut refused = SkipReason::Unknown;

        for &mount_id in self.by_device.get(device).into_iter().flatten() {
            match self.verdict(mount_id, path) {
                Ok(()) => return Ok(()),
                Err(SkipReason::Unknown) => {}
                Err(reason) => refused = reason,
            }
        }
        Err(refused)
    }
}

/// `st_dev` as the `major:minor` text mountinfo uses.
pub fn device_text(dev: u64) -> String {
    format!("{}:{}", major(dev), minor(dev))
}

#[cfg(test)]
mod tests {
    use rustix::fs::makedev;

    use super::*;
    use crate::linux::mountinfo::parse;

    const TABLE: &str = "\
1 0 0:1 / / rw - ext4 /dev/sda1 rw
2 1 0:2 / /proc rw - proc proc rw
3 1 8:17 / /data rw - ext4 /dev/sdb1 rw
";

    fn snapshot(rules: &ExclusionRules, enterable: Option<HashSet<u64>>) -> MountSnapshot {
        let decision = enterable.map(|enter| Decision {
            enter,
            ..Decision::default()
        });
        MountSnapshot::new(rules, &parse(TABLE.as_bytes()), decision)
    }

    #[test]
    fn a_mount_missing_from_the_snapshot_is_not_entered_with_or_without_one_file_system() {
        for rules in [ExclusionRules::DEFAULT, ExclusionRules::NONE] {
            for enterable in [None, Some(HashSet::from([3]))] {
                let snapshot = snapshot(&rules, enterable);

                assert!(
                    snapshot.may_enter(3, Path::new("/data")),
                    "a known, ordinary mount is entered"
                );
                assert!(
                    !snapshot.may_enter(99, Path::new("/data")),
                    "a mount that appeared later is not"
                );
            }
        }
        assert!(!snapshot(&ExclusionRules::DEFAULT, None).may_enter(2, Path::new("/proc")));
        assert!(
            !MountSnapshot::default().may_enter(3, Path::new("/data")),
            "with no table at all nothing is known"
        );
    }

    #[test]
    fn a_refused_mount_says_why() {
        let table = "\
1 0 0:1 / / rw - ext4 /dev/sda1 rw
2 1 0:2 / /proc rw - proc proc rw
3 1 8:17 / /data rw - ext4 /dev/sdb1 rw
4 1 8:1 / /again rw - ext4 /dev/disk/by-uuid/x rw
5 1 0:5 / /snap rw - btrfs /dev/sdc1 rw
";
        let decision = Decision {
            enter: HashSet::new(),
            duplicate: HashSet::from([4]),
            other_disk: HashSet::from([3, 5]),
        };
        let snapshot = MountSnapshot::new(&ExclusionRules::DEFAULT, &parse(table.as_bytes()), Some(decision));

        assert_eq!(snapshot.verdict(2, Path::new("/proc")), Err(SkipReason::System));
        assert_eq!(
            snapshot.verdict(3, Path::new("/data")),
            Err(SkipReason::OtherDisk)
        );
        assert_eq!(
            snapshot.verdict(4, Path::new("/again")),
            Err(SkipReason::Duplicate)
        );
        assert_eq!(
            snapshot.verdict(99, Path::new("/new")),
            Err(SkipReason::Unknown),
            "not in the table"
        );
        assert_eq!(
            snapshot.verdict(3, Path::new("/elsewhere")),
            Err(SkipReason::Unknown),
            "a reused id"
        );
        assert_eq!(
            MountSnapshot::default().verdict(3, Path::new("/data")),
            Err(SkipReason::Unknown),
            "no table at all"
        );
    }

    #[test]
    fn a_system_mount_stays_system_whatever_the_decision_says() {
        // The rules exclude `/proc` before the mount table's idea of disks is looked at.
        let table = "1 0 0:1 / / rw - ext4 /dev/sda1 rw\n2 1 0:2 / /proc rw - proc proc rw\n";
        let decision = Decision {
            other_disk: HashSet::from([2]),
            ..Decision::default()
        };

        let snapshot = MountSnapshot::new(&ExclusionRules::DEFAULT, &parse(table.as_bytes()), Some(decision));

        assert_eq!(snapshot.verdict(2, Path::new("/proc")), Err(SkipReason::System));
    }

    #[test]
    fn without_mount_ids_the_reason_comes_from_the_mounts_with_that_device_and_path() {
        let snapshot = snapshot(&ExclusionRules::DEFAULT, Some(HashSet::new()));

        assert_eq!(
            snapshot.verdict_by_device("0:2", Path::new("/proc")),
            Err(SkipReason::System)
        );
        assert_eq!(
            snapshot.verdict_by_device("8:33", Path::new("/data")),
            Err(SkipReason::Unknown),
            "a device the table does not know"
        );
        assert_eq!(
            snapshot.verdict_by_device("8:17", Path::new("/data")),
            Err(SkipReason::Unknown),
            "known, but the decision names no reason"
        );
    }

    #[test]
    fn a_reused_mount_id_met_at_another_path_inherits_nothing() {
        // Mount 3 was `/data` when the table was read. Later it was unmounted and the kernel gave id 3
        // to a mount somewhere else (here: under /proc/..., or at another folder of the scan).
        let snapshot = snapshot(&ExclusionRules::DEFAULT, Some(HashSet::from([3])));

        assert!(snapshot.may_enter(3, Path::new("/data")));
        assert!(!snapshot.may_enter(3, Path::new("/data/nested")), "below, not at");
        assert!(!snapshot.may_enter(3, Path::new("/mnt/usb")), "an unrelated path");
        assert!(!snapshot.may_enter(3, Path::new("/")), "the root");
        assert!(
            snapshot.may_enter(3, Path::new("/data/")),
            "the spelling of the path is not what matters"
        );
    }

    #[test]
    fn a_snapshot_that_only_enters_some_mounts_enters_exactly_those() {
        let free = snapshot(&ExclusionRules::DEFAULT, None);
        let staying = snapshot(&ExclusionRules::DEFAULT, Some(HashSet::new()));

        assert!(free.may_enter(3, Path::new("/data")));
        assert!(
            !staying.may_enter(3, Path::new("/data")),
            "another disk stays out"
        );
    }

    #[test]
    fn without_mount_ids_a_mount_is_entered_only_through_a_device_the_table_allows() {
        let rules = ExclusionRules::DEFAULT;
        let free = snapshot(&rules, None);
        let staying = snapshot(&rules, Some(HashSet::new()));
        let one_disk = snapshot(&rules, Some(HashSet::from([3])));

        assert_eq!(device_text(makedev(8, 17)), "8:17");

        assert!(free.may_enter_by_device("8:17", Path::new("/data")));
        assert!(
            !free.may_enter_by_device("8:17", Path::new("/elsewhere")),
            "right device, wrong place"
        );
        assert!(
            !free.may_enter_by_device("8:33", Path::new("/data")),
            "a device the table does not know"
        );
        assert!(
            !free.may_enter_by_device("0:2", Path::new("/proc")),
            "an excluded filesystem is never entered"
        );

        assert!(
            !staying.may_enter_by_device("8:17", Path::new("/data")),
            "one-file-system keeps it out"
        );
        assert!(one_disk.may_enter_by_device("8:17", Path::new("/data")));
    }
}
