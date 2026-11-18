//! Where a trashed path goes on macOS (package M2a), and the rules around it that need no system call:
//! `~/.Trash` on the home volume, and `<mount>/.Trashes/<uid>` on any other. Also the names an item gets in
//! the trash when its own is taken, which volumes a path crosses, and which places `restore` takes from.
//!
//! Compiled on every OS so that the tests run on Linux. Only `macos::{probe, remove, trash}` call them.

/// The longest name APFS and HFS+ take (in bytes of UTF-8 for APFS, in UTF-16 units for HFS+: the bytes
/// are the stricter count for text with accents, and the same for ASCII).
pub const MAX_NAME_BYTES: usize = 255;

/// How many names (`name`, `name 2`, ...) are tried before giving up on a trash folder that has them all.
pub const MAX_NAMES: u32 = 1000;

/// An extension longer than this (the dot and the letters) is not treated as one when a number is put in
/// front of it.
const MAX_EXTENSION_BYTES: usize = 16;

/// The names of an absolute path. `None` for a path that is not absolute, or that has an empty component
/// (`//`), a `.` or a `..`. A trailing `/` is allowed. The root has no names.
pub fn components(path: &[u8]) -> Option<Vec<&[u8]>> {
    let rest = path.strip_prefix(b"/")?;
    let rest = rest.strip_suffix(b"/").unwrap_or(rest);
    if rest.is_empty() {
        return Some(Vec::new());
    }

    let parts: Vec<&[u8]> = rest.split(|&byte| byte == b'/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == b"." || *part == b"..")
    {
        return None;
    }

    Some(parts)
}

/// The trash folder for a target. `home_trash` is the user's `~/.Trash`; `on_home_volume` says the target is
/// on the volume that holds it; `mount_point` is the mount point of the target's volume. `None` when no
/// trash can take it: a `home_trash` or `mount_point` that is not a clean absolute path.
pub fn trash_dir(home_trash: &[u8], uid: u32, on_home_volume: bool, mount_point: &[u8]) -> Option<Vec<u8>> {
    if on_home_volume {
        let parts = components(home_trash)?;
        // The home trash is a folder of the home folder, never the root itself.
        if parts.is_empty() {
            return None;
        }

        return Some(join(&parts));
    }

    let mut parts = components(mount_point)?;
    let uid = uid.to_string();
    parts.push(b".Trashes");
    parts.push(uid.as_bytes());

    Some(join(&parts))
}

/// Whether a step from a folder on `parent_dev` into one on `dev` starts another mount. A change of device
/// does, except the one every Mac has: the system volume (`root_dev`, the device of `/`) holds firmlinks
/// (`/Users`, `/Applications`, `/Library`, `/opt`, ...) into the data volume (`data_dev`, the device of
/// `/System/Volumes/Data`), and stepping through one of them leaves nothing behind.
pub fn starts_mount(dev: u64, parent_dev: u64, root_dev: u64, data_dev: Option<u64>) -> bool {
    if dev == parent_dev {
        return false;
    }

    !(parent_dev == root_dev && data_dev == Some(dev))
}

/// The mount point of the volume that holds a path: `ancestors` are the folders above the target from the
/// first below `/` on, each with its name and whether it starts a mount. The deepest one that does is the
/// mount point, and `/` when none does.
pub fn mount_point(ancestors: &[(&[u8], bool)]) -> Vec<u8> {
    let Some(last) = ancestors.iter().rposition(|(_, starts)| *starts) else {
        return b"/".to_vec();
    };

    let names: Vec<&[u8]> = ancestors[..=last].iter().map(|(name, _)| *name).collect();
    join(&names)
}

/// The name an item gets in the trash on its `number`th try, the way Finder numbers them: the first try is
/// the name itself, the next ones are `name 2.ext`, `name 3.ext`, with the number before the extension of a
/// file, and `name 2` after the name of a folder. The result is never longer than [`MAX_NAME_BYTES`]: a long
/// name loses its end (not the middle of a letter, when it is text) before the number is added.
pub fn numbered_name(name: &[u8], number: u32, is_dir: bool) -> Vec<u8> {
    if number <= 1 {
        return shorten(name, MAX_NAME_BYTES).to_vec();
    }

    let (stem, extension) = if is_dir {
        (name, &name[name.len()..])
    } else {
        split_extension(name)
    };
    let suffix = format!(" {number}");
    let room = MAX_NAME_BYTES.saturating_sub(suffix.len() + extension.len());

    let mut numbered = shorten(stem, room).to_vec();
    numbered.extend_from_slice(suffix.as_bytes());
    numbered.extend_from_slice(extension);
    numbered
}

/// The name without its extension, and the extension with its dot (empty when there is none). A dot at the
/// start is not an extension (`.zshrc`), and neither is anything but a short run of letters and digits
/// after the last dot (`Mr. Smith`).
fn split_extension(name: &[u8]) -> (&[u8], &[u8]) {
    let Some(dot) = name.iter().rposition(|&byte| byte == b'.') else {
        return (name, &name[name.len()..]);
    };

    let extension = &name[dot..];
    let letters = &extension[1..];
    let plain = dot > 0
        && !letters.is_empty()
        && extension.len() <= MAX_EXTENSION_BYTES
        && letters.iter().all(u8::is_ascii_alphanumeric);
    if plain {
        (&name[..dot], extension)
    } else {
        (name, &name[name.len()..])
    }
}

/// The first `max` bytes of `name`, cut so that a UTF-8 name stays valid UTF-8.
fn shorten(name: &[u8], max: usize) -> &[u8] {
    if name.len() <= max {
        return name;
    }

    let mut end = max;
    if std::str::from_utf8(name).is_ok() {
        while end > 0 && (name[end] & 0b1100_0000) == 0b1000_0000 {
            end -= 1;
        }
    }

    &name[..end]
}

/// Which trash folder an entry given to `restore` lies in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashKind {
    /// `<home>/.Trash`.
    Home,
    /// `<mount>/.Trashes/<uid>`: closed to others, so it must be checked as such.
    Volume,
}

/// The trash folder Platter makes that `trashed` is a direct child of: exactly `home_trash` (the user's
/// `~/.Trash`, as the environment names it; `None` when there is no home, and then no home trash takes
/// anything) or `<mount>/.Trashes/<uid>` of this user. `restore` takes an entry from nowhere else,
/// whatever it is told: an entry is put back only from the trash it was put in.
pub fn trash_entry_kind(trashed: &[u8], uid: u32, home_trash: Option<&[u8]>) -> Option<TrashKind> {
    let parts = components(trashed)?;
    let (_, folder) = parts.split_last()?;

    if let Some(home) = home_trash.and_then(components)
        && !home.is_empty()
        && home.as_slice() == folder
    {
        return Some(TrashKind::Home);
    }

    let uid = uid.to_string();
    match folder {
        [.., shared, user] if *shared == b".Trashes" && *user == uid.as_bytes() => Some(TrashKind::Volume),
        _ => None,
    }
}

/// Whether a trash folder that is held open is one `restore` may take from: it belongs to the user
/// (`owner` is the owner of the open descriptor), and a volume's folder (`kind`) is closed to others (`mode`
/// has no group or other bit). A folder Platter does not own, or one open to others, is where another
/// user could have put an entry of their own.
pub fn trash_folder_is_ours(kind: TrashKind, owner: u32, mode: u32, uid: u32) -> bool {
    if owner != uid {
        return false;
    }

    match kind {
        TrashKind::Home => true,
        TrashKind::Volume => mode & 0o077 == 0,
    }
}

/// The names joined into an absolute path (`/` for none).
fn join(names: &[&[u8]]) -> Vec<u8> {
    if names.is_empty() {
        return b"/".to_vec();
    }

    let mut path = Vec::new();
    for name in names {
        path.push(b'/');
        path.extend_from_slice(name);
    }

    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_clean_names_or_nothing() {
        assert_eq!(components(b"/").unwrap(), Vec::<&[u8]>::new());
        assert_eq!(components(b"/Users/me/").unwrap(), [&b"Users"[..], b"me"]);
        for odd in [&b"relative"[..], b"/a/../b", b"/a//b", b"/a/./b", b""] {
            assert!(components(odd).is_none(), "{odd:?}");
        }
    }

    #[test]
    fn the_home_volume_uses_the_home_trash_and_another_volume_its_own_folder_for_the_user() {
        assert_eq!(
            trash_dir(b"/Users/me/.Trash", 501, true, b"/Volumes/Disk").unwrap(),
            b"/Users/me/.Trash"
        );
        assert_eq!(
            trash_dir(b"/Users/me/.Trash/", 501, true, b"/").unwrap(),
            b"/Users/me/.Trash"
        );
        assert_eq!(
            trash_dir(b"/Users/me/.Trash", 501, false, b"/Volumes/Disk").unwrap(),
            b"/Volumes/Disk/.Trashes/501"
        );
        assert_eq!(
            trash_dir(b"/Users/me/.Trash", 502, false, b"/").unwrap(),
            b"/.Trashes/502"
        );
    }

    #[test]
    fn no_trash_is_made_from_a_path_that_is_not_clean() {
        assert_eq!(trash_dir(b".Trash", 501, true, b"/"), None);
        assert_eq!(trash_dir(b"/", 501, true, b"/"), None);
        assert_eq!(trash_dir(b"/Users/../.Trash", 501, true, b"/"), None);
        assert_eq!(trash_dir(b"/Users/me/.Trash", 501, false, b"Volumes"), None);
        assert_eq!(trash_dir(b"/Users/me/.Trash", 501, false, b"/Volumes/../x"), None);
    }

    #[test]
    fn a_firmlink_into_the_data_volume_is_not_a_mount_and_any_other_change_of_device_is() {
        let (root, data, other) = (1, 2, 3);

        assert!(!starts_mount(root, root, root, Some(data)), "same device");
        assert!(
            !starts_mount(data, root, root, Some(data)),
            "/Users on the data volume"
        );
        assert!(starts_mount(data, root, root, None), "no data volume known");
        assert!(starts_mount(other, root, root, Some(data)), "/Volumes/Disk");
        assert!(
            starts_mount(root, data, root, Some(data)),
            "back out is not a firmlink"
        );
        assert!(
            starts_mount(other, data, root, Some(data)),
            "a disk below the data volume"
        );
    }

    #[test]
    fn the_mount_point_is_the_deepest_folder_that_starts_one() {
        let disk: [(&[u8], bool); 3] = [(b"Volumes", false), (b"Disk", true), (b"photos", false)];
        assert_eq!(mount_point(&disk), b"/Volumes/Disk");

        let nested: [(&[u8], bool); 3] = [(b"Volumes", false), (b"Disk", true), (b"inner", true)];
        assert_eq!(mount_point(&nested), b"/Volumes/Disk/inner");

        let none: [(&[u8], bool); 2] = [(b"Users", false), (b"me", false)];
        assert_eq!(mount_point(&none), b"/");
        assert_eq!(mount_point(&[]), b"/");
    }

    #[test]
    fn a_taken_name_gets_a_number_before_the_extension_like_finders() {
        assert_eq!(numbered_name(b"report.pdf", 1, false), b"report.pdf");
        assert_eq!(numbered_name(b"report.pdf", 2, false), b"report 2.pdf");
        assert_eq!(numbered_name(b"report.pdf", 13, false), b"report 13.pdf");
        assert_eq!(numbered_name(b"archive.tar.gz", 2, false), b"archive.tar 2.gz");
        assert_eq!(numbered_name(b"README", 2, false), b"README 2");
        assert_eq!(numbered_name(b".zshrc", 2, false), b".zshrc 2");
        assert_eq!(numbered_name(b"Mr. Smith", 3, false), b"Mr. Smith 3");
        assert_eq!(numbered_name(b"name.", 2, false), b"name. 2");
    }

    #[test]
    fn a_folder_gets_its_number_at_the_end() {
        assert_eq!(numbered_name(b"build", 2, true), b"build 2");
        assert_eq!(numbered_name(b"v1.2", 2, true), b"v1.2 2");
    }

    #[test]
    fn a_long_name_is_shortened_to_fit_with_its_number_and_extension() {
        let long = "ż".repeat(200);
        let name = format!("{long}.pdf");

        let first = numbered_name(name.as_bytes(), 1, false);
        assert!(first.len() <= MAX_NAME_BYTES);
        assert!(std::str::from_utf8(&first).is_ok());

        let second = numbered_name(name.as_bytes(), 2, false);
        assert!(second.len() <= MAX_NAME_BYTES);
        assert!(second.ends_with(b" 2.pdf"));
        assert!(std::str::from_utf8(&second).is_ok(), "no letter is cut in two");

        let wide = vec![b'x'; 400];
        assert_eq!(numbered_name(&wide, 2, true).len(), MAX_NAME_BYTES);
        assert_eq!(numbered_name(&wide, 1000, false).len(), MAX_NAME_BYTES);
    }

    #[test]
    fn restore_takes_only_from_the_trash_folders_platter_makes() {
        let home = Some(&b"/Users/me/.Trash"[..]);
        let kind = |path: &[u8], home| trash_entry_kind(path, 501, home);

        assert_eq!(kind(b"/Users/me/.Trash/report.pdf", home), Some(TrashKind::Home));
        assert_eq!(
            kind(b"/Volumes/Disk/.Trashes/501/report 2.pdf", home),
            Some(TrashKind::Volume)
        );
        assert_eq!(kind(b"/.Trashes/501/x", None), Some(TrashKind::Volume));

        assert_eq!(
            kind(b"/Volumes/Disk/.Trashes/502/x", home),
            None,
            "someone else's folder"
        );
        assert_eq!(
            kind(b"/Users/me/.Trash/dir/inner", home),
            None,
            "not a direct child"
        );
        assert_eq!(kind(b"/Users/me/Documents/x", home), None);
        assert_eq!(kind(b"/Users/me/.Trash", home), None, "the folder itself");
        assert_eq!(kind(b"/Users/me/.Trash/../x", home), None);
        assert_eq!(kind(b"relative/.Trash/x", home), None);
        assert_eq!(kind(b"/", home), None);
    }

    #[test]
    fn the_home_trash_is_the_one_of_this_home_and_no_other_dot_trash() {
        let home = Some(&b"/Users/me/.Trash"[..]);
        let kind = |path: &[u8], home| trash_entry_kind(path, 501, home);

        assert_eq!(kind(b"/Users/other/.Trash/x", home), None, "another user's home");
        assert_eq!(kind(b"/tmp/.Trash/x", home), None, "any folder named .Trash");
        assert_eq!(kind(b"/Users/me/work/.Trash/x", home), None);
        assert_eq!(kind(b"/Users/me/.Trash/x", None), None, "no home, no home trash");
        assert_eq!(kind(b"/.Trash/x", Some(b"/.Trash")), Some(TrashKind::Home));
        assert_eq!(kind(b"/x", Some(b"/")), None, "the root is no trash folder");
        assert_eq!(
            kind(b"/Users/me/.Trash/x", Some(b"relative/.Trash")),
            None,
            "an environment path that is not plain names nothing"
        );
    }

    #[test]
    fn a_trash_folder_must_be_the_users_and_a_volumes_folder_closed_to_others() {
        assert!(trash_folder_is_ours(TrashKind::Home, 501, 0o755, 501));
        assert!(trash_folder_is_ours(TrashKind::Volume, 501, 0o700, 501));

        assert!(!trash_folder_is_ours(TrashKind::Home, 502, 0o700, 501));
        assert!(!trash_folder_is_ours(TrashKind::Volume, 502, 0o700, 501));
        assert!(!trash_folder_is_ours(TrashKind::Volume, 501, 0o750, 501));
        assert!(!trash_folder_is_ours(TrashKind::Volume, 501, 0o705, 501));
        assert!(!trash_folder_is_ours(TrashKind::Volume, 501, 0o777, 501));
    }
}
