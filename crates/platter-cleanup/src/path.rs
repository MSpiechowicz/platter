//! Paths as exact bytes, compared component by component, in the syntax of the machine's OS.
//!
//! The style is chosen at run time from `Environment.os` (never from the OS the code was built for), so
//! the tests on Linux cover the Windows syntax too.
//!
//! - **Posix**: `/a/b`. The top is `/`, which has no components; a name is any bytes but `/` and NUL.
//!   Names are compared as exact bytes.
//! - **Mac**: `/a/b` as Posix, but names are compared the way Windows does (see below): APFS and HFS+ are
//!   case-insensitive by default, so `/users/u/library/keychains` is the Keychains folder. A volume that
//!   is case-sensitive makes this err toward over-protecting, which is what a protected place wants. Names
//!   that differ only in Unicode normalization are not told apart here: the guard also compares identities.
//! - **Windows**: `X:\a\b`, drive-absolute only. The first component is the drive root (`C:\`, with its
//!   backslash), then the names. The separator is `\` and `/` is not one: a name that holds a `/` is
//!   refused, so a path that mixes them never passes as something it is not. A UNC share (`\\host\share`),
//!   a verbatim path (`\\?\C:\`), a drive-relative path (`C:a`) and a path with no drive are refused.
//!   Names are compared the way NTFS does, by each letter's Unicode uppercase when that is one character
//!   (see [`platter_rules::same_windows_name`]: the volume's own table is newer or older than the
//!   standard library's, which errs toward over-protecting, and a protected place must win). A name
//!   Windows would read as something else is refused too: one that ends in a dot or a space (stripped by
//!   the Win32 layer), holds a character that is not allowed in a name (`<>:"|?*`, controls), is a DOS
//!   device name (`CON`, `NUL`, `COM1`, `COM¹`, `CONIN$`...), or ends in a `.{GUID}` (the shell turns
//!   such a folder into a special one, a junction to a namespace). Short names (`PROGRA~1`) are not told
//!   apart here: the guard also compares identities.

use platter_core::Os;
use platter_rules::{fold_windows_name, same_windows_name};

/// The syntax of an absolute path on one OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathStyle {
    Posix,
    /// Posix syntax, names compared without regard to case.
    Mac,
    Windows,
}

/// DOS device names: a file by one of these, with or without an extension, is the device.
const DEVICE_NAMES: &[&str] = &[
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "CONIN$",
    "CONOUT$",
    "COM0",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT0",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

impl Default for PathStyle {
    fn default() -> Self {
        Self::host()
    }
}

impl PathStyle {
    /// The style of paths on `os`: Windows on Windows, Mac on macOS, Posix everywhere else.
    pub fn for_os(os: Os) -> Self {
        match os {
            Os::Windows => Self::Windows,
            Os::MacOs => Self::Mac,
            Os::Linux | Os::Other => Self::Posix,
        }
    }

    /// The style of the OS this program was built for, for the callers that have no `Environment`.
    pub fn host() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Posix
        }
    }

    pub(crate) fn separator(self) -> u8 {
        match self {
            Self::Posix | Self::Mac => b'/',
            Self::Windows => b'\\',
        }
    }

    /// The components of an absolute path. `None` for anything that is not plainly absolute in this
    /// style: a relative path, or one with `.`, `..` or an empty component inside (a trailing separator
    /// is allowed), and on Windows anything described in the module's text.
    pub(crate) fn components(self, path: &[u8]) -> Option<Vec<&[u8]>> {
        match self {
            Self::Posix | Self::Mac => posix_components(path),
            Self::Windows => windows_components(path),
        }
    }

    /// Owned components, for paths that are kept.
    pub(crate) fn owned(self, path: &[u8]) -> Option<Vec<Vec<u8>>> {
        self.components(path)
            .map(|parts| parts.into_iter().map(<[u8]>::to_vec).collect())
    }

    /// Whether `inner` is `outer` or lies below it.
    pub(crate) fn is_within<A: AsRef<[u8]>, B: AsRef<[u8]>>(self, inner: &[A], outer: &[B]) -> bool {
        inner.len() >= outer.len()
            && inner
                .iter()
                .zip(outer)
                .all(|(a, b)| self.same_name(a.as_ref(), b.as_ref()))
    }

    /// Whether two components are the same name.
    pub(crate) fn same_name(self, a: &[u8], b: &[u8]) -> bool {
        match self {
            Self::Posix => a == b,
            Self::Mac | Self::Windows => same_windows_name(a, b),
        }
    }

    /// The components as they are compared: Windows and macOS fold case (see [`same_windows_name`]), so
    /// that sorting them puts a folder before what is inside it whatever the spelling.
    pub(crate) fn folded(self, parts: &[Vec<u8>]) -> Vec<Vec<u8>> {
        match self {
            Self::Posix => parts.to_vec(),
            Self::Mac | Self::Windows => parts.iter().map(|part| fold_windows_name(part)).collect(),
        }
    }

    /// The path of `parts` (the inverse of [`PathStyle::owned`]).
    pub(crate) fn join(self, parts: &[Vec<u8>]) -> Vec<u8> {
        match self {
            Self::Posix | Self::Mac => {
                let mut path = Vec::new();
                for part in parts {
                    path.push(b'/');
                    path.extend_from_slice(part);
                }
                path
            }
            Self::Windows => {
                let mut path = Vec::new();
                for (position, part) in parts.iter().enumerate() {
                    if position > 1 {
                        path.push(b'\\');
                    }
                    path.extend_from_slice(part);
                }
                path
            }
        }
    }

    /// `path` with the name `name` below it.
    pub(crate) fn child(self, path: &[u8], name: &[u8]) -> Vec<u8> {
        let mut joined = path.to_vec();
        if joined.last() != Some(&self.separator()) {
            joined.push(self.separator());
        }
        joined.extend_from_slice(name);

        joined
    }

    /// The folder holding the entry at `path`: everything before the last separator (for a path directly
    /// below a drive root, the drive root with its backslash, for one directly below `/` the empty path).
    pub(crate) fn parent(self, path: &[u8]) -> &[u8] {
        let last = path.iter().rposition(|&byte| byte == self.separator());

        match (self, last) {
            (_, None) => &[],
            (Self::Posix | Self::Mac, Some(at)) => &path[..at],
            // `C:\name`: the parent is `C:\`, which keeps its backslash.
            (Self::Windows, Some(at)) if at <= 2 => &path[..=at],
            (Self::Windows, Some(at)) => &path[..at],
        }
    }

    /// The last name of `path`.
    pub(crate) fn file_name(self, path: &[u8]) -> &[u8] {
        let separator = self.separator();

        path.rsplit(|&byte| byte == separator).next().unwrap_or(path)
    }

    /// A path for people to read: the parts, as the style writes them.
    pub(crate) fn show(self, parts: &[Vec<u8>]) -> String {
        match self {
            // The top has no components: `/`.
            Self::Posix | Self::Mac if parts.is_empty() => "/".to_owned(),
            _ => String::from_utf8_lossy(&self.join(parts)).into_owned(),
        }
    }
}

fn posix_components(path: &[u8]) -> Option<Vec<&[u8]>> {
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

fn windows_components(path: &[u8]) -> Option<Vec<&[u8]>> {
    // `X:\`, a drive letter and its root. Nothing else is absolute: not `C:a`, not `\\host\share`.
    let [letter, b':', b'\\', ..] = path else {
        return None;
    };
    if !letter.is_ascii_alphabetic() {
        return None;
    }

    let mut parts: Vec<&[u8]> = vec![&path[..3]];
    let rest = &path[3..];
    let rest = rest.strip_suffix(b"\\").unwrap_or(rest);
    if rest.is_empty() {
        return Some(parts);
    }

    for name in rest.split(|&byte| byte == b'\\') {
        if !windows_name_is_plain(name) {
            return None;
        }
        parts.push(name);
    }

    Some(parts)
}

/// Whether `name` is a name that means what it says on Windows (see the module's text).
fn windows_name_is_plain(name: &[u8]) -> bool {
    if name.is_empty() || name == b"." || name == b".." {
        return false;
    }
    if name.ends_with(b".") || name.ends_with(b" ") {
        return false;
    }
    if name
        .iter()
        .any(|&byte| byte < 0x20 || matches!(byte, b'/' | b':' | b'<' | b'>' | b'"' | b'|' | b'?' | b'*'))
    {
        return false;
    }

    // `NUL.txt` is the device too: the part before the first dot decides.
    let stem = name.split(|&byte| byte == b'.').next().unwrap_or(name);
    let stem = String::from_utf8_lossy(stem);
    let stem = stem.trim_end_matches(' ');

    if DEVICE_NAMES
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
    {
        return false;
    }

    !ends_in_guid(name)
}

/// Whether `name` ends in `.{` hex digits and dashes `}`: the shell reads such a folder as the namespace
/// the GUID names (a junction to it), whatever is inside.
fn ends_in_guid(name: &[u8]) -> bool {
    let Some(inner) = name.strip_suffix(b"}") else {
        return false;
    };
    let Some(open) = inner.windows(2).rposition(|pair| pair == b".{") else {
        return false;
    };
    let guid = &inner[open + 2..];

    !guid.is_empty() && guid.iter().all(|byte| byte.is_ascii_hexdigit() || *byte == b'-')
}

/// A path for people to read (lossy for names that are not valid UTF-8).
pub(crate) fn display(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSIX: PathStyle = PathStyle::Posix;
    const WINDOWS: PathStyle = PathStyle::Windows;

    #[test]
    fn posix_components_are_exact_and_refuse_anything_that_could_point_elsewhere() {
        assert_eq!(POSIX.components(b"/").unwrap(), Vec::<&[u8]>::new());
        assert_eq!(POSIX.components(b"/a/b/").unwrap(), [&b"a"[..], b"b"]);
        assert_eq!(POSIX.components(b"/a/\xff").unwrap(), [&b"a"[..], b"\xff"]);
        for bad in [&b"a/b"[..], b"", b"/a/../b", b"/a/./b", b"/a//b", b"C:\\a"] {
            assert!(POSIX.components(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn windows_components_start_with_the_drive_root() {
        assert_eq!(WINDOWS.components(b"C:\\").unwrap(), [&b"C:\\"[..]]);
        assert_eq!(
            WINDOWS.components(b"C:\\Users\\me\\").unwrap(),
            [&b"C:\\"[..], b"Users", b"me"]
        );
        assert_eq!(
            WINDOWS.components(b"d:\\a b\\c.d").unwrap(),
            [&b"d:\\"[..], b"a b", b"c.d"]
        );
        assert_eq!(
            WINDOWS.components("C:\\Users\\zażółć".as_bytes()).unwrap().len(),
            3
        );
    }

    #[test]
    fn windows_refuses_everything_that_is_not_a_plain_drive_absolute_path() {
        for bad in [
            &b""[..],
            b"/",
            b"/a/b",
            b"C:",
            b"C:a",
            b"C:/a",
            b"C:\\a/b",
            b"C:\\a\\\\b",
            b"C:\\a\\.\\b",
            b"C:\\a\\..\\b",
            b"\\\\host\\share\\a",
            b"\\\\?\\C:\\a",
            b"\\\\.\\C:\\a",
            b"\\a",
            b"1:\\a",
            b"CC:\\a",
            b"C:\\a.",
            b"C:\\a \\b",
            b"C:\\a:stream",
            b"C:\\a?",
            b"C:\\a*",
            b"C:\\a\x01",
            b"C:\\NUL",
            b"C:\\con.txt",
            b"C:\\a\\COM1",
            b"C:\\a\\lpt9.log",
            b"C:\\a\\conin$",
            b"C:\\a\\CONOUT$.txt",
            "C:\\a\\COM\u{b9}".as_bytes(),
            "C:\\a\\lpt\u{b2}.log".as_bytes(),
            "C:\\a\\LPT\u{b3}".as_bytes(),
            b"C:\\Folder.{645FF040-5081-101B-9F08-00AA002F954E}",
            b"C:\\a\\x.{00000000-0000-0000-0000-000000000000}",
        ] {
            assert!(
                WINDOWS.components(bad).is_none(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn names_that_only_look_like_devices_are_fine() {
        for good in [
            &b"C:\\console"[..],
            b"C:\\nullable",
            b"C:\\COM10",
            b"C:\\a\\LPT",
            b"C:\\a\\conin",
            b"C:\\a\\x.{note}",
            b"C:\\a\\{645FF040-5081-101B-9F08-00AA002F954E}.x",
        ] {
            assert!(
                WINDOWS.components(good).is_some(),
                "{:?}",
                String::from_utf8_lossy(good)
            );
        }
    }

    #[test]
    fn within_compares_whole_components() {
        let home = POSIX.components(b"/home/u").unwrap();

        assert!(POSIX.is_within(&POSIX.components(b"/home/u").unwrap(), &home));
        assert!(POSIX.is_within(&POSIX.components(b"/home/u/x").unwrap(), &home));
        assert!(!POSIX.is_within(&POSIX.components(b"/home/user").unwrap(), &home));
        assert!(!POSIX.is_within(&POSIX.components(b"/home").unwrap(), &home));
        assert!(!POSIX.is_within(&POSIX.components(b"/HOME/u").unwrap(), &home));
    }

    #[test]
    fn windows_within_folds_case_the_way_ntfs_does_and_still_compares_whole_components() {
        let home = WINDOWS.components(b"C:\\Users\\Me").unwrap();

        // Letters beyond ASCII fold too, and the long s reaches an `S`.
        let polish = WINDOWS.components("C:\\Users\\Żółć".as_bytes()).unwrap();
        assert!(WINDOWS.is_within(
            &WINDOWS.components("c:\\USERS\\żÓŁĆ\\x".as_bytes()).unwrap(),
            &polish
        ));
        assert!(WINDOWS.is_within(&WINDOWS.components("C:\\Users\\Me\\x".as_bytes()).unwrap(), &home));
        assert!(WINDOWS.is_within(
            &WINDOWS.components("C:\\U\u{17f}ers\\Me".as_bytes()).unwrap(),
            &home
        ));

        assert!(WINDOWS.is_within(&WINDOWS.components(b"c:\\users\\ME\\x").unwrap(), &home));
        assert!(!WINDOWS.is_within(&WINDOWS.components(b"C:\\Users\\Mem").unwrap(), &home));
        assert!(!WINDOWS.is_within(&WINDOWS.components(b"D:\\Users\\Me").unwrap(), &home));
        assert!(!WINDOWS.is_within(&WINDOWS.components(b"C:\\Users").unwrap(), &home));
    }

    #[test]
    fn join_is_the_inverse_of_owned() {
        for (style, path) in [(POSIX, &b"/a/b"[..]), (WINDOWS, b"C:\\a\\b"), (WINDOWS, b"C:\\")] {
            let parts = style.owned(path).unwrap();

            assert_eq!(style.join(&parts), path);
        }
    }

    #[test]
    fn parent_and_name_follow_the_style() {
        assert_eq!(POSIX.parent(b"/a/b"), b"/a");
        assert_eq!(POSIX.parent(b"/a"), b"");
        assert_eq!(WINDOWS.parent(b"C:\\a\\b"), b"C:\\a");
        assert_eq!(WINDOWS.parent(b"C:\\a"), b"C:\\");
        assert_eq!(POSIX.file_name(b"/a/b"), b"b");
        assert_eq!(WINDOWS.file_name(b"C:\\a\\b.txt"), b"b.txt");
        assert_eq!(WINDOWS.child(b"C:\\", b"a"), b"C:\\a");
        assert_eq!(WINDOWS.child(b"C:\\a", b"b"), b"C:\\a\\b");
        assert_eq!(POSIX.child(b"/", b"a"), b"/a");
        assert_eq!(POSIX.child(b"/a", b"b"), b"/a/b");
    }

    #[test]
    fn the_style_comes_from_the_os() {
        assert_eq!(PathStyle::for_os(Os::Windows), PathStyle::Windows);
        assert_eq!(PathStyle::for_os(Os::MacOs), PathStyle::Mac);
        for os in [Os::Linux, Os::Other] {
            assert_eq!(PathStyle::for_os(os), PathStyle::Posix);
        }
    }

    #[test]
    fn mac_paths_have_the_syntax_of_posix_and_compare_names_without_regard_to_case() {
        let mac = PathStyle::Mac;
        let users = mac.components(b"/Users/Me").unwrap();

        assert_eq!(mac.components(b"/a/b/").unwrap(), [&b"a"[..], b"b"]);
        for bad in [&b"a/b"[..], b"", b"/a/../b", b"/a/./b", b"/a//b", b"C:\\a"] {
            assert!(mac.components(bad).is_none(), "{bad:?}");
        }

        assert!(mac.is_within(&mac.components(b"/users/me/Library").unwrap(), &users));
        assert!(mac.is_within(&mac.components("/USERS/ME".as_bytes()).unwrap(), &users));
        assert!(!mac.is_within(&mac.components(b"/Users/Mem").unwrap(), &users));
        assert!(!mac.is_within(&mac.components(b"/Users").unwrap(), &users));
        // Linux stays exact.
        assert!(!POSIX.is_within(&POSIX.components(b"/users/me").unwrap(), &users));
    }

    #[test]
    fn mac_paths_join_split_and_show_like_posix() {
        let mac = PathStyle::Mac;
        let parts = mac.owned(b"/Users/me/Library").unwrap();

        assert_eq!(mac.join(&parts), b"/Users/me/Library");
        assert_eq!(mac.show(&[]), "/");
        assert_eq!(mac.parent(b"/Users/me"), b"/Users");
        assert_eq!(mac.parent(b"/Users"), b"");
        assert_eq!(mac.file_name(b"/Users/me"), b"me");
        assert_eq!(mac.child(b"/", b"a"), b"/a");
        assert_eq!(mac.child(b"/a", b"b"), b"/a/b");
        assert_eq!(
            mac.folded(&mac.owned(b"/Users/ME").unwrap()),
            mac.folded(&mac.owned(b"/users/me").unwrap())
        );
    }
}
