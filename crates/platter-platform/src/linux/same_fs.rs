//! "Stay on one filesystem" means "stay on one disk", not "stop at every mount point", and "enter every
//! mount" must not mean "count the same data twice".
//!
//! Many Linux systems mount several subvolumes of ONE btrfs filesystem at `/`, `/home`, `/var/log`, and
//! so on. Each is a mount root, yet scanning `/` must include `/home`: it is the same disk. Bind mounts
//! and repeated mounts of one subvolume, on the other hand, show the same directory at a second place,
//! and a walk that enters both counts it twice. This module decides, from the mount table read when the
//! scan starts, which mounts below the scan root the walk enters. The answer is a set of mount ids; the
//! reader looks entries up in it while walking, so the decision is deterministic and needs no lock.
//! (`platter-core` knows nothing of this: the reader just withholds the child handle of every mount
//! root that is not in the set.)
//!
//! # What "the same filesystem" means
//!
//! The superblock device and the filesystem type, and nothing else. The mount source (the text after the
//! type) is how the mount was spelled, not what it is: one partition mounted twice as `/dev/sdb1` and as
//! `/dev/disk/by-uuid/...` has two different sources and one `major:minor`, and a test that required
//! equal sources would enter both mounts and count the data twice.
//!
//! Dropping the source cannot make two different filesystems look equal. A superblock has exactly one
//! device number and one type, so two mounts with the same number show the same superblock, which is the
//! same filesystem by definition. A block-backed filesystem has its disk's `major:minor`. Every other
//! filesystem (FUSE, tmpfs, network, the pseudo ones, and btrfs, which has no single disk) gets an
//! anonymous number, major 0 and a minor the kernel hands out once per superblock
//! (`get_anon_bdev`), so a FUSE filesystem cannot take the `major:minor` of a block device, nor of another
//! mounted filesystem, whatever it calls its source. (`fuseblk` is the one FUSE variant that shows its
//! block device's number; its type, `fuseblk`, differs from the type of a kernel driver on the same disk,
//! which is why the type stays in the test.) The one thing a mount of untrusted origin controls is the
//! source text, and that is no longer looked at.
//!
//! # The rules
//!
//! A mount `M` below the scan root is entered when all of these hold:
//!
//! 1. **Candidate.** With `one_file_system`, `M` is on the same filesystem as the mount the scan root
//!    lives on: the same superblock device (the `major:minor` field of mountinfo, identical for all
//!    subvolume mounts of one btrfs filesystem, unlike `st_dev`) and the same filesystem type. Another
//!    disk, tmpfs, a network or pseudo filesystem fails this. Without `one_file_system` every
//!    filesystem is a candidate and the rules below apply per filesystem. A mount the [`ExclusionRules`]
//!    exclude is never a candidate.
//! 2. **Reachable.** The mount it is mounted on is the scan root's mount or an entered mount. A mount
//!    inside a skipped one is never met by the walk.
//! 3. **No double counting.** The `root` field of mountinfo is the directory of the filesystem that
//!    the mount shows (`/` for a whole filesystem, `/@home` for a subvolume or a bind mount of that
//!    directory). The same filesystem directory is counted twice exactly when two of those directories
//!    contain one another and the data is reachable at two places in the walk. So `M` is refused when its
//!    `root` is equal to, an ancestor of, or inside the `root` of an area already covered *on the same
//!    filesystem*, with one exception: a mount that sits exactly where its own `root` lives. Covered
//!    areas start with the scan root itself (the `root` of its mount plus the path from the mount point
//!    to the scan root) and grow with every accepted mount.
//!
//! # A mount that replaces its own location
//!
//! Take scan root `/` on subvolume `/@`, with the nested subvolume `/@/home` mounted at `/home`. The
//! mount's `root` lies inside the covered area `/@`, yet nothing is counted twice: the directory `/home`
//! of `/@` is exactly what the mount shows, so entering it replaces that directory instead of adding a
//! second copy. In general, with `P` the mount `M` is mounted on, `M` replaces its own location when
//! `M.root == P.root + (M.mount_point - P.mount_point)`. Such a mount is entered, and its overlap with
//! the areas of `P` and of `P`'s own ancestors is not held against it. The same holds when the scan root
//! is the top-level subvolume `/` and `/home` is mounted at `/home`, or when the scan root is deeper
//! (`/var/lib` with `/@/var/lib/machines` at `/var/lib/machines`). It still overlaps every other area:
//! `/@/var/lib/machines` mounted a second time at `/mnt/m` is not at its own location, and is refused.
//!
//! What is refused, then, are the roots that equal, contain or lie inside a covered area and are
//! reachable at a different path as well: the same subvolume mounted twice; the top-level subvolume `/`
//! mounted under `/mnt/btrfs-root` (it contains everything); a bind mount of a sub-directory of covered
//! data. A bind mount of a directory that is *outside* the scan root, say `/@home/me/docs` bound into
//! `/@srv` while scanning `/srv`, is not covered by anything and is counted, once.
//!
//! # Order
//!
//! The choice does not depend on the order of the mount table or on which mounts happen to become
//! reachable first. Candidates are ranked once, over all of them: the shallowest `root` first (the
//! largest area, so a bind mount of a sub-directory can never shadow the subvolume that contains it,
//! whatever the mount points are called), then the shortest mount point, then the lexically smaller one,
//! then the lower mount id. In that order each candidate is accepted together with the mounts it needs
//! to be reachable (its chain of parent mounts); if the candidate or one of those conflicts with what is
//! covered (or with each other), none of them is added and the candidate is refused. A mount that is
//! only reachable through a conflicting one is therefore refused too: with `/@home/me/data` at `/mnt/a`
//! and `/@home` mounted at `/mnt/a/h` on top of it, entering the larger area would first need the
//! smaller one, which is part of it, so only `/mnt/a` is entered.
//!
//! # Limits
//!
//! Mounts stacked on one mount point hide each other; only the visible one is met by the walk, and if
//! it is refused as a duplicate of the hidden one the visible one stays out of the scan. Rare enough to
//! accept. If the scan root's mount or path cannot be determined, nothing is entered with
//! `one_file_system`, and the exclusion rules alone decide without it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::exclusions::ExclusionRules;
use super::mountinfo::MountEntry;

/// What the table says about the mounts below the scan root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decision {
    /// Ids of the mounts the walk may enter.
    pub enter: HashSet<u64>,
    /// Mounts of a filesystem the scan covers already (refused by rule 3 or unreachable behind a refused
    /// mount).
    pub duplicate: HashSet<u64>,
    /// With `one_file_system`: mounts of another filesystem.
    pub other_disk: HashSet<u64>,
}

/// Which mounts below `scan_root` the walk may enter, given the scan root's own mount (`scan_mount_id`,
/// its `stx_mnt_id`), and why it refuses the others. `scan_root` must be the canonical path as the
/// process sees it, like the mount points in the table.
///
/// `None` means the table cannot say (no `one_file_system`, and the scan root's mount or path is
/// unknown): the exclusion rules alone decide then. With `one_file_system` that case enters nothing and
/// gives no reason for the refusal.
pub fn enterable_mounts(
    mounts: &[MountEntry],
    rules: &ExclusionRules,
    scan_mount_id: u64,
    scan_root: &Path,
    one_file_system: bool,
) -> Option<Decision> {
    let base = mounts.iter().find(|mount| mount.mount_id == scan_mount_id);
    let inside = base.and_then(|base| scan_root.strip_prefix(&base.mount_point).ok());

    let (Some(base), Some(inside)) = (base, inside) else {
        return one_file_system.then(Decision::default);
    };
    Some(decide(mounts, rules, base, inside, scan_root, one_file_system))
}

/// A directory of some filesystem that the walk covers.
struct Area<'a> {
    mount: &'a MountEntry,
    /// The directory the walk covers: the mount's `root`, or for the scan root's own mount the root plus
    /// the path to the scan root.
    root: PathBuf,
}

fn decide(
    mounts: &[MountEntry],
    rules: &ExclusionRules,
    base: &MountEntry,
    inside: &Path,
    scan_root: &Path,
    one_file_system: bool,
) -> Decision {
    let other_disk = mounts
        .iter()
        .filter(|mount| one_file_system && !is_same_filesystem(mount, base))
        .filter(|mount| !rules.excludes(&mount.mount_point, &mount.fs_type))
        .map(|mount| mount.mount_id)
        .collect();

    let mut candidates: Vec<&MountEntry> = mounts
        .iter()
        .filter(|mount| mount.mount_id != base.mount_id)
        .filter(|mount| !one_file_system || is_same_filesystem(mount, base))
        .filter(|mount| !rules.excludes(&mount.mount_point, &mount.fs_type))
        .filter(|mount| is_strictly_below(&mount.mount_point, scan_root))
        .collect();
    candidates.sort_by_key(|mount| preference(mount));

    let by_id: HashMap<u64, &MountEntry> = candidates.iter().map(|mount| (mount.mount_id, *mount)).collect();

    let mut covered: HashMap<u64, Area<'_>> = HashMap::from([(
        base.mount_id,
        Area {
            mount: base,
            root: base.root.join(inside),
        },
    )]);
    let mut refused = HashSet::new();

    for candidate in &candidates {
        if covered.contains_key(&candidate.mount_id) || refused.contains(&candidate.mount_id) {
            continue;
        }

        let Some(chain) = reachability_chain(candidate, &by_id, &covered, &refused) else {
            refused.insert(candidate.mount_id);
            continue;
        };

        let mut added = Vec::new();
        for mount in chain {
            if conflicts(mount, &covered, &by_id) {
                break;
            }

            covered.insert(
                mount.mount_id,
                Area {
                    mount,
                    root: mount.root.clone(),
                },
            );
            added.push(mount.mount_id);
        }

        if !covered.contains_key(&candidate.mount_id) {
            for id in added {
                covered.remove(&id);
            }
            refused.insert(candidate.mount_id);
        }
    }

    covered.remove(&base.mount_id);
    Decision {
        enter: covered.into_keys().collect(),
        duplicate: refused,
        other_disk,
    }
}

/// The candidate and the candidates it is mounted on, outermost first, up to a mount that is already
/// covered. `None` when some mount on the way is not a candidate (another filesystem, excluded, outside
/// the scan) or was refused: the candidate cannot be reached.
fn reachability_chain<'a>(
    candidate: &'a MountEntry,
    by_id: &HashMap<u64, &'a MountEntry>,
    covered: &HashMap<u64, Area<'_>>,
    refused: &HashSet<u64>,
) -> Option<Vec<&'a MountEntry>> {
    let mut chain = vec![candidate];
    let mut parent = candidate.parent_id;

    while !covered.contains_key(&parent) {
        if refused.contains(&parent) || chain.len() > by_id.len() {
            return None;
        }

        let mount = *by_id.get(&parent)?;
        chain.push(mount);
        parent = mount.parent_id;
    }

    chain.reverse();
    Some(chain)
}

/// Whether adding `mount` would count a directory twice. Its parent mount is covered (or was just added
/// as an earlier link of the chain).
fn conflicts(
    mount: &MountEntry,
    covered: &HashMap<u64, Area<'_>>,
    by_id: &HashMap<u64, &MountEntry>,
) -> bool {
    let Some(parent) = covered.get(&mount.parent_id) else {
        return true;
    };

    // The areas of the parent and of its ancestors are what this mount replaces a part of, when it sits
    // at its own location; overlapping them is then the point, not a duplicate.
    let own_places = if replaces_its_own_location(mount, parent.mount) {
        ancestry(mount.parent_id, covered, by_id)
    } else {
        HashSet::new()
    };

    covered.iter().any(|(id, area)| {
        is_same_filesystem(area.mount, mount) && overlaps(&area.root, &mount.root) && !own_places.contains(id)
    })
}

/// `parent_id` and the mounts it is (transitively) mounted on, as far as they are covered.
fn ancestry(
    parent_id: u64,
    covered: &HashMap<u64, Area<'_>>,
    by_id: &HashMap<u64, &MountEntry>,
) -> HashSet<u64> {
    let mut ids = HashSet::new();
    let mut current = parent_id;

    while covered.contains_key(&current) && ids.insert(current) {
        // The scan root's own mount is covered but not a candidate: the end of the chain.
        let Some(mount) = by_id.get(&current) else {
            break;
        };
        current = mount.parent_id;
    }
    ids
}

/// Whether `mount` shows, at its mount point, the very directory that is at that place in `parent` (the
/// mount it is mounted on): `root == parent.root + (mount_point - parent.mount_point)`.
fn replaces_its_own_location(mount: &MountEntry, parent: &MountEntry) -> bool {
    if !is_same_filesystem(mount, parent) {
        return false;
    }

    match mount.mount_point.strip_prefix(&parent.mount_point) {
        Ok(relative) => !relative.as_os_str().is_empty() && parent.root.join(relative) == mount.root,
        Err(_) => false,
    }
}

/// Two mounts show the same filesystem when they have the same superblock device and type; the mount
/// source is deliberately not compared (see "What the same filesystem means").
fn is_same_filesystem(mount: &MountEntry, other: &MountEntry) -> bool {
    mount.device == other.device && mount.fs_type == other.fs_type
}

fn is_strictly_below(path: &Path, ancestor: &Path) -> bool {
    path != ancestor && path.starts_with(ancestor)
}

/// Two directories of one filesystem overlap when one is the other or contains it.
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Sort key of the acceptance order: lower is accepted first.
fn preference(mount: &MountEntry) -> (usize, usize, &[u8], u64) {
    let point = mount.mount_point.as_os_str().as_encoded_bytes();

    (
        mount.root.components().count(),
        point.len(),
        point,
        mount.mount_id,
    )
}

/// The directory behind an open scan root descriptor, as `/proc/self/fd` reports it.
pub(super) fn path_of_fd(fd: &impl std::os::fd::AsRawFd) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::mountinfo::parse;

    /// The layout of a CachyOS machine: subvolumes of one btrfs filesystem (0:32) mounted in several
    /// places, next to a vfat partition, tmpfs and pseudo filesystems.
    const BTRFS_MACHINE: &str = "\
67 1 0:32 /@ / rw,noatime - btrfs /dev/nvme0n1p2 rw,subvolid=256,subvol=/@
36 67 0:6 / /dev rw,nosuid - devtmpfs devtmpfs rw
46 67 0:23 / /proc rw,nosuid - proc proc rw
47 67 0:27 / /run rw,nosuid - tmpfs tmpfs rw
53 67 0:32 /@srv /srv rw,noatime - btrfs /dev/nvme0n1p2 rw,subvolid=259,subvol=/@srv
55 67 0:32 /@root /root rw,noatime - btrfs /dev/nvme0n1p2 rw,subvolid=258,subvol=/@root
54 67 0:32 /@home /home rw,noatime - btrfs /dev/nvme0n1p2 rw,subvolid=257,subvol=/@home
69 67 0:52 / /tmp rw,noatime - tmpfs tmpfs rw
162 67 0:32 /@cache /var/cache rw,noatime - btrfs /dev/nvme0n1p2 rw,subvolid=260,subvol=/@cache
209 67 0:32 /@log /var/log rw,noatime - btrfs /dev/nvme0n1p2 rw,subvolid=262,subvol=/@log
214 67 0:32 /@tmp /var/tmp rw,noatime - btrfs /dev/nvme0n1p2 rw,subvolid=261,subvol=/@tmp
219 67 259:1 / /boot rw,relatime - vfat /dev/nvme0n1p1 rw,fmask=0077
";

    fn decide(text: &str, scan_mount_id: u64, scan_root: &str) -> Vec<u64> {
        sorted(enterable_mounts(
            &parse(text.as_bytes()),
            &ExclusionRules::DEFAULT,
            scan_mount_id,
            Path::new(scan_root),
            true,
        ))
    }

    /// The same, without `one_file_system`.
    fn decide_everywhere(text: &str, scan_mount_id: u64, scan_root: &str) -> Vec<u64> {
        sorted(enterable_mounts(
            &parse(text.as_bytes()),
            &ExclusionRules::DEFAULT,
            scan_mount_id,
            Path::new(scan_root),
            false,
        ))
    }

    fn sorted(decision: Option<Decision>) -> Vec<u64> {
        let decision = decision.expect("the scan root is placed");
        let mut ids: Vec<u64> = decision.enter.into_iter().collect();
        ids.sort_unstable();
        ids
    }

    fn with(extra: &str) -> String {
        format!("{BTRFS_MACHINE}{extra}")
    }

    #[test]
    fn scanning_the_root_of_a_btrfs_machine_enters_every_subvolume_and_nothing_else() {
        // /srv /root /home /var/cache /var/log /var/tmp; not /boot (vfat), /tmp, /run (tmpfs), /dev, /proc.
        assert_eq!(decide(BTRFS_MACHINE, 67, "/"), [53, 54, 55, 162, 209, 214]);
    }

    /// One partition (8:17) mounted twice, once under its device name and once under its UUID link: two
    /// source strings, one superblock.
    const SAME_PARTITION_TWICE: &str = "\
1 0 8:17 / / rw - ext4 /dev/sdb1 rw
2 1 8:17 / /mnt/again rw - ext4 /dev/disk/by-uuid/0e3d-aa rw
3 1 8:1 / /boot rw - ext4 /dev/sda1 rw
";

    #[test]
    fn one_partition_mounted_under_two_device_names_is_one_filesystem_with_one_file_system() {
        // `/mnt/again` shows the whole of the filesystem the scan root lives on: entering it counts every
        // file twice. The source strings differ, the device does not.
        assert_eq!(decide(SAME_PARTITION_TWICE, 1, "/"), Vec::<u64>::new());

        let table = parse(SAME_PARTITION_TWICE.as_bytes());
        let decision = enterable_mounts(&table, &ExclusionRules::DEFAULT, 1, Path::new("/"), true).unwrap();
        assert!(
            decision.duplicate.contains(&2),
            "refused as a duplicate, not as another disk"
        );
        assert!(decision.other_disk.contains(&3), "/boot is another disk");
        assert!(!decision.other_disk.contains(&2));
    }

    #[test]
    fn one_partition_mounted_under_two_device_names_is_entered_once_without_one_file_system() {
        let text = "\
1 0 8:1 / / rw - ext4 /dev/sda1 rw
2 1 8:17 / /mnt/data rw - ext4 /dev/sdb1 rw
3 1 8:17 / /mnt/data-uuid rw - ext4 /dev/disk/by-uuid/0e3d-aa rw
4 1 8:17 / /a rw - ext4 /dev/disk/by-label/data rw
";

        // Shortest mount point first: `/a`; the other two are the same filesystem.
        assert_eq!(decide_everywhere(text, 1, "/"), [4]);
    }

    #[test]
    fn the_same_device_with_another_filesystem_type_is_another_filesystem() {
        // `fuseblk` shows the block device's number like the kernel driver does; the type tells them
        // apart (and a kernel driver and a FUSE driver cannot both have the disk mounted at once).
        let text = "\
1 0 8:17 / / rw - ext4 /dev/sdb1 rw
2 1 8:17 / /mnt/ntfs rw - fuseblk /dev/sdb1 rw
";

        let table = parse(text.as_bytes());
        let decision = enterable_mounts(&table, &ExclusionRules::DEFAULT, 1, Path::new("/"), true).unwrap();
        assert!(decision.other_disk.contains(&2));
        assert!(decision.duplicate.is_empty());
    }

    #[test]
    fn a_mount_source_is_not_part_of_what_identifies_a_filesystem() {
        let mount = |source: &str| {
            parse(format!("1 0 8:17 / / rw - ext4 {source} rw\n").as_bytes())
                .pop()
                .unwrap()
        };

        assert!(is_same_filesystem(
            &mount("/dev/sdb1"),
            &mount("/dev/disk/by-uuid/1234")
        ));
        assert!(is_same_filesystem(
            &mount("/dev/sdb1"),
            &mount("anything the mounter wrote")
        ));
    }

    #[test]
    fn an_anonymous_device_is_never_taken_for_a_block_device() {
        // A FUSE filesystem has a major 0 device of its own; it differs from every block device and
        // from every other anonymous one, whatever source it names.
        let text = "\
1 0 8:17 / / rw - ext4 /dev/sdb1 rw
2 1 0:80 / /mnt/fake rw - fuse.sshfs /dev/sdb1 rw
3 1 0:81 / /mnt/fake2 rw - ext4 /dev/sdb1 rw
";

        let table = parse(text.as_bytes());
        let decision = enterable_mounts(&table, &ExclusionRules::NONE, 1, Path::new("/"), true).unwrap();

        assert!(decision.enter.is_empty());
        assert!(decision.other_disk.contains(&2) && decision.other_disk.contains(&3));
    }

    #[test]
    fn the_top_level_subvolume_mounted_a_second_time_is_not_entered() {
        // Its root `/` contains `/@home`, `/@srv`, ... again: everything would be counted twice.
        let text = with("300 67 0:32 / /mnt/btrfs-root rw - btrfs /dev/nvme0n1p2 rw,subvolid=5,subvol=/\n");

        assert_eq!(decide(&text, 67, "/"), [53, 54, 55, 162, 209, 214]);
    }

    #[test]
    fn mounts_below_a_skipped_mount_are_never_entered() {
        let text = with(
            "300 67 0:32 / /mnt/btrfs-root rw - btrfs /dev/nvme0n1p2 rw\n\
             301 300 0:32 /@games /mnt/btrfs-root/games rw - btrfs /dev/nvme0n1p2 rw\n\
             302 219 0:32 /@games /boot/games rw - btrfs /dev/nvme0n1p2 rw\n",
        );

        let entered = decide(&text, 67, "/");

        assert!(
            !entered.contains(&301),
            "inside the duplicate of the top-level subvolume"
        );
        assert!(!entered.contains(&302), "inside the vfat partition");
    }

    #[test]
    fn a_scan_that_starts_on_the_top_level_subvolume_already_contains_its_subvolumes() {
        let text = "\
1 0 0:32 / / rw - btrfs /dev/sda1 rw,subvolid=5
2 1 0:32 /@home /home rw - btrfs /dev/sda1 rw,subvolid=257
3 1 0:32 /@ /mnt/system rw - btrfs /dev/sda1 rw,subvolid=256
";

        assert_eq!(decide(text, 1, "/"), Vec::<u64>::new());
    }

    #[test]
    fn the_same_subvolume_mounted_twice_is_entered_once_at_the_shortest_mount_point() {
        let text = with(
            "310 67 0:32 /@games /mnt/games-again rw - btrfs /dev/nvme0n1p2 rw\n\
             311 67 0:32 /@games /games rw - btrfs /dev/nvme0n1p2 rw\n\
             312 67 0:32 /@home /mnt/home-again rw - btrfs /dev/nvme0n1p2 rw\n",
        );

        let entered = decide(&text, 67, "/");

        assert!(entered.contains(&311), "/games is shorter than /mnt/games-again");
        assert!(!entered.contains(&310));
        assert!(
            entered.contains(&54) && !entered.contains(&312),
            "/home beats /mnt/home-again"
        );
    }

    #[test]
    fn two_mounts_of_one_directory_with_equally_long_mount_points_resolve_lexically() {
        let text = with(
            "320 67 0:32 /@games /gb rw - btrfs /dev/nvme0n1p2 rw\n\
             321 67 0:32 /@games /ga rw - btrfs /dev/nvme0n1p2 rw\n",
        );

        let entered = decide(&text, 67, "/");

        assert!(entered.contains(&321) && !entered.contains(&320), "/ga < /gb");
    }

    #[test]
    fn a_bind_mount_of_a_directory_inside_covered_data_is_not_entered() {
        // 330 is inside the scan root's own subvolume `/@`, 331 inside `/@home`, which is entered.
        let text = with(
            "330 67 0:32 /@/etc /mnt/etc-copy rw - btrfs /dev/nvme0n1p2 rw\n\
             331 67 0:32 /@home/me/data /var/data rw - btrfs /dev/nvme0n1p2 rw\n",
        );

        let entered = decide(&text, 67, "/");

        assert!(!entered.contains(&330));
        assert!(!entered.contains(&331));
        assert!(entered.contains(&54));
    }

    #[test]
    fn a_short_named_bind_mount_cannot_shadow_the_subvolume_that_contains_it() {
        // `/data` sorts before `/home` and is no longer, yet `/home` must still win.
        let text = with("340 67 0:32 /@home/me/data /data rw - btrfs /dev/nvme0n1p2 rw\n");

        let entered = decide(&text, 67, "/");

        assert!(entered.contains(&54), "/home is entered");
        assert!(
            !entered.contains(&340),
            "the bind mount of a folder inside it is not"
        );
    }

    #[test]
    fn a_nested_subvolume_mounted_at_its_own_location_is_entered() {
        // `/@/var/lib/machines` is a directory of `/@`; mounting it at `/var/lib/machines` shows exactly
        // that directory, so it replaces its own location and nothing is counted twice.
        let text = with("350 67 0:32 /@/var/lib/machines /var/lib/machines rw - btrfs /dev/nvme0n1p2 rw\n");

        assert!(decide(&text, 67, "/").contains(&350));
    }

    #[test]
    fn a_nested_subvolume_also_mounted_at_another_place_is_counted_once_at_its_own() {
        // 351 binds the same directory at /mnt/m (shorter, so ranked first): it is reachable at a
        // different path than where it lives and is refused; 350 at its own location is entered.
        let text = with(
            "350 67 0:32 /@/var/lib/machines /var/lib/machines rw - btrfs /dev/nvme0n1p2 rw\n\
             351 67 0:32 /@/var/lib/machines /mnt/m rw - btrfs /dev/nvme0n1p2 rw\n",
        );

        let entered = decide(&text, 67, "/");

        assert!(entered.contains(&350));
        assert!(!entered.contains(&351));
    }

    const NESTED_SUBVOLUMES: &str = "\
1 0 0:30 /@ / rw - btrfs /dev/sda2 rw,subvolid=256,subvol=/@
2 1 0:30 /@/home /home rw - btrfs /dev/sda2 rw,subvolid=257,subvol=/@/home
3 1 0:30 /@/var /var rw - btrfs /dev/sda2 rw,subvolid=258,subvol=/@/var
4 1 0:30 /@/.snapshots /.snapshots rw - btrfs /dev/sda2 rw,subvolid=259,subvol=/@/.snapshots
";

    #[test]
    fn nested_subvolumes_of_the_root_subvolume_each_mounted_at_their_own_place_are_all_entered() {
        assert_eq!(decide(NESTED_SUBVOLUMES, 1, "/"), [2, 3, 4]);
    }

    #[test]
    fn a_top_level_subvolume_as_root_enters_a_directory_mounted_at_its_own_place() {
        let text = "\
1 0 0:30 / / rw - btrfs /dev/sda2 rw,subvolid=5
2 1 0:30 /home /home rw - btrfs /dev/sda2 rw,subvolid=257
3 1 0:30 /@srv /srv rw - btrfs /dev/sda2 rw,subvolid=258
";

        // `/home` is the directory `/home` of the top-level subvolume: entered. `/@srv` is a directory
        // of it too, but not the one at `/srv`: the walk reaches it as `/@srv` already.
        assert_eq!(decide(text, 1, "/"), [2]);
    }

    #[test]
    fn nested_mounts_inside_a_mount_at_its_own_place_are_entered_too() {
        let text = format!(
            "{NESTED_SUBVOLUMES}\
5 2 0:30 /@/home/me /home/me rw - btrfs /dev/sda2 rw\n\
6 3 0:30 /@/var/log /var/log rw - btrfs /dev/sda2 rw\n\
7 3 0:30 /@/var/cache /var/cache2 rw - btrfs /dev/sda2 rw\n"
        );

        let entered = decide(&text, 1, "/");

        assert_eq!(
            entered,
            [2, 3, 4, 5, 6],
            "7 is `var/cache` shown at another name: a duplicate"
        );
    }

    #[test]
    fn a_mount_hidden_under_another_at_its_own_place_is_not_entered() {
        // 6 is mounted on the base at /var/log, then 3 was mounted over /var (hiding it): its parent
        // is the base but the data is reachable through 3, a covered area that is not 6's own mount.
        let text = format!("{NESTED_SUBVOLUMES}6 1 0:30 /@/var/log /var/log rw - btrfs /dev/sda2 rw\n");

        assert_eq!(decide(&text, 1, "/"), [2, 3, 4]);
    }

    #[test]
    fn a_deep_scan_root_enters_a_nested_subvolume_mounted_at_its_own_place_inside_it() {
        let text = "\
1 0 0:30 /@ / rw - btrfs /dev/sda2 rw
2 1 0:30 /@/var/lib/machines /var/lib/machines rw - btrfs /dev/sda2 rw
3 1 0:30 /@/var/lib/docker /var/lib/docker rw - btrfs /dev/sda2 rw
4 1 0:30 /@/var/lib/docker /mnt/docker-again rw - btrfs /dev/sda2 rw
5 1 0:30 /@/var/log /var/log rw - btrfs /dev/sda2 rw
6 1 0:30 /@/home /home rw - btrfs /dev/sda2 rw
";

        // 5 and 6 are not below /var/lib, 4 is the same directory as 3 at a place it does not live.
        assert_eq!(decide(text, 1, "/var/lib"), [2, 3]);
    }

    #[test]
    fn the_choice_does_not_depend_on_the_order_of_the_table_or_on_what_is_reachable_first() {
        // 3 (`/@home`, the larger area) is reachable only through 4 (a bind of a long path that ranks
        // late), while 2 (a folder inside `/@home`) is reachable at once and ranks before 4. Choosing
        // among the mounts that are reachable *now* would take 2 first and lose the whole of `/@home`.
        let lines = [
            "1 0 0:30 /@ / rw - btrfs /dev/sda2 rw",
            "2 1 0:30 /@home/me/data /mnt/a rw - btrfs /dev/sda2 rw",
            "3 4 0:30 /@home /x/h rw - btrfs /dev/sda2 rw",
            "4 1 0:30 /@x/y/z/w /x rw - btrfs /dev/sda2 rw",
        ];
        let table = |order: &[usize]| {
            order
                .iter()
                .map(|&i| format!("{}\n", lines[i]))
                .collect::<String>()
        };

        for order in [[0, 1, 2, 3], [0, 3, 2, 1], [3, 2, 1, 0], [2, 0, 3, 1]] {
            assert_eq!(decide(&table(&order), 1, "/"), [3, 4], "order {order:?}");
        }
    }

    #[test]
    fn a_larger_area_that_is_only_reachable_through_a_part_of_itself_cannot_be_entered() {
        // `/@home` is mounted on top of the mount of `/@home/me/data`: to get to it the walk must enter
        // the smaller area, which the larger one contains, so the two would count `me/data` twice.
        let text = "\
1 0 0:30 /@ / rw - btrfs /dev/sda2 rw
2 1 0:30 /@home/me/data /mnt/a rw - btrfs /dev/sda2 rw
3 2 0:30 /@home /mnt/a/h rw - btrfs /dev/sda2 rw
";

        assert_eq!(decide(text, 1, "/"), [2]);
    }

    #[test]
    fn the_same_table_in_any_order_gives_the_same_answer() {
        let lines: Vec<&str> = BTRFS_MACHINE.lines().collect();
        let extra = [
            "310 67 0:32 /@games /mnt/games-again rw - btrfs /dev/nvme0n1p2 rw",
            "311 67 0:32 /@games /games rw - btrfs /dev/nvme0n1p2 rw",
            "330 67 0:32 /@/etc /mnt/etc-copy rw - btrfs /dev/nvme0n1p2 rw",
            "350 67 0:32 /@/var/lib/machines /var/lib/machines rw - btrfs /dev/nvme0n1p2 rw",
        ];
        let all: Vec<&str> = lines.iter().copied().chain(extra).collect();
        let expected = decide(&all.join("\n"), 67, "/");

        for step in 1..all.len() {
            let mut rotated = all.clone();
            rotated.rotate_left(step);
            assert_eq!(
                decide(&rotated.join("\n"), 67, "/"),
                expected,
                "rotated by {step}"
            );

            rotated.reverse();
            assert_eq!(
                decide(&rotated.join("\n"), 67, "/"),
                expected,
                "reversed after {step}"
            );
        }
        assert!(expected.contains(&350) && expected.contains(&311) && !expected.contains(&310));
    }

    #[test]
    fn other_disks_tmpfs_network_and_pseudo_filesystems_are_not_entered() {
        let text = with(
            "360 67 0:60 / /data rw - btrfs /dev/sdb1 rw\n\
             361 67 0:61 / /mnt/ext4 rw - ext4 /dev/sdc1 rw\n\
             362 67 0:62 / /var/lib/scratch rw - tmpfs tmpfs rw\n\
             363 67 0:63 / /mnt/nas rw - nfs4 nas:/export rw\n\
             364 67 0:64 /@srv /mnt/other-source rw - btrfs /dev/mapper/other rw\n\
             365 67 0:65 / /sys rw - sysfs sysfs rw\n",
        );

        assert_eq!(decide(&text, 67, "/"), [53, 54, 55, 162, 209, 214]);
    }

    #[test]
    fn a_mount_of_the_same_filesystem_is_not_entered_when_the_rules_exclude_its_mount_point() {
        let text = with("370 67 0:32 /@games /games rw - btrfs /dev/nvme0n1p2 rw\n");
        let rules = ExclusionRules {
            paths: &["/games"],
            ..ExclusionRules::NONE
        };

        let table = parse(text.as_bytes());
        let enter = |rules: &ExclusionRules| {
            enterable_mounts(&table, rules, 67, Path::new("/"), true)
                .unwrap()
                .enter
        };

        assert!(!enter(&rules).contains(&370));
        assert!(enter(&ExclusionRules::NONE).contains(&370));
    }

    #[test]
    fn a_scan_root_that_is_a_subvolume_mount_enters_the_nested_mounts_of_its_filesystem() {
        let text = with(
            "400 54 0:32 /@games /home/me/games rw - btrfs /dev/nvme0n1p2 rw\n\
             401 400 0:32 /@mods /home/me/games/mods rw - btrfs /dev/nvme0n1p2 rw\n\
             402 54 0:70 / /home/me/usb rw - ext4 /dev/sdd1 rw\n\
             403 402 0:32 /@mods2 /home/me/usb/mods rw - btrfs /dev/nvme0n1p2 rw\n\
             404 54 0:32 /@home /home/me/loop rw - btrfs /dev/nvme0n1p2 rw\n",
        );

        let entered = decide(&text, 54, "/home");

        // Not /var/cache and the other subvolumes of the system: they are not below /home. Not 403
        // (inside the ext4 mount), not 404 (the scan root's own subvolume again).
        assert_eq!(entered, [400, 401]);
    }

    #[test]
    fn a_scan_root_inside_a_mount_only_counts_what_it_actually_contains() {
        // 410 binds a folder outside the scan root (not covered: entered), 411 one inside it (covered),
        // 412 is mounted outside the scan root (the walk never meets it).
        let text = with(
            "410 54 0:32 /@home/me/Documents /home/me/Projects/docs rw - btrfs /dev/nvme0n1p2 rw\n\
             411 54 0:32 /@home/me/Projects/app /home/me/Projects/app-copy rw - btrfs /dev/nvme0n1p2 rw\n\
             412 54 0:32 /@games /home/me/games rw - btrfs /dev/nvme0n1p2 rw\n",
        );

        assert_eq!(decide(&text, 54, "/home/me/Projects"), [410]);
    }

    #[test]
    fn nothing_is_entered_when_the_scan_root_cannot_be_placed_in_the_table() {
        assert!(decide(BTRFS_MACHINE, 9999, "/").is_empty(), "unknown mount id");
        assert!(
            decide(BTRFS_MACHINE, 54, "/srv").is_empty(),
            "path outside the mount point"
        );
        assert!(decide("", 67, "/").is_empty(), "no table");

        // Without `one_file_system` the table has no say, the exclusion rules decide: `None`.
        let table = parse(BTRFS_MACHINE.as_bytes());
        let unplaced =
            |id, root| enterable_mounts(&table, &ExclusionRules::DEFAULT, id, Path::new(root), false);
        assert_eq!(unplaced(9999, "/"), None);
        assert_eq!(unplaced(54, "/srv"), None);
    }

    // Without `one_file_system`: other filesystems are entered, but nothing is counted twice.

    const TWO_DISKS: &str = "\
1 0 0:30 /@ / rw - btrfs /dev/sda2 rw
2 1 0:30 /@home /home rw - btrfs /dev/sda2 rw
3 1 8:17 / /mnt/data rw - ext4 /dev/sdb1 rw
4 1 8:17 /photos /mnt/photos rw - ext4 /dev/sdb1 rw
5 1 8:17 / /mnt/data-again rw - ext4 /dev/sdb1 rw
6 1 0:30 /@home /mnt/home-again rw - btrfs /dev/sda2 rw
7 1 0:30 /@/etc /mnt/etc-copy rw - btrfs /dev/sda2 rw
8 1 0:60 / /mnt/scratch rw - tmpfs tmpfs rw
9 1 0:61 / /proc rw - proc proc rw
";

    #[test]
    fn without_one_file_system_other_filesystems_are_entered_but_repeated_mounts_are_not() {
        // 2 (/@home) and 3 (the whole ext4 disk) are entered once; 4 (a folder of 3), 5 (3 again), 6 (2
        // again) and 7 (a folder of the covered `/@`) show data that is reachable elsewhere. 8 is another
        // filesystem (tmpfs): `tmpfs` is excluded by the rules, `/proc` too.
        assert_eq!(decide_everywhere(TWO_DISKS, 1, "/"), [2, 3]);
    }

    #[test]
    fn without_one_file_system_the_mounts_below_a_refused_duplicate_are_not_entered() {
        let text = format!("{TWO_DISKS}20 5 0:30 /@games /mnt/data-again/games rw - btrfs /dev/sda2 rw\n");

        assert!(!decide_everywhere(&text, 1, "/").contains(&20));
    }

    #[test]
    fn without_one_file_system_a_nested_subvolume_at_its_own_place_is_entered_and_an_other_disk_too() {
        let text = format!("{NESTED_SUBVOLUMES}9 3 8:17 / /var/usb rw - ext4 /dev/sdb1 rw\n");

        assert_eq!(decide_everywhere(&text, 1, "/"), [2, 3, 4, 9]);
    }

    #[test]
    fn without_one_file_system_a_filesystem_mounted_twice_is_entered_once_at_the_shortest_mount_point() {
        let text = "\
1 0 0:30 /@ / rw - btrfs /dev/sda2 rw
2 1 8:17 / /mnt/backup-disk rw - ext4 /dev/sdb1 rw
3 1 8:17 / /b rw - ext4 /dev/sdb1 rw
";

        assert_eq!(decide_everywhere(text, 1, "/"), [3]);
    }
}
