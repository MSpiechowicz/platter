//! What Platter reads of the Recycle Bin's settings, and the rules it applies to them (W2). Text and
//! numbers only, no system calls: `bin_guard` runs `reg.exe` and `mountvol.exe` and hands their output here.
//! This file uses nothing but `std`, so the tests in it run on every system (`tests/windows_bin_logic.rs`
//! includes it).
//!
//! The shell may ask the user (or, without a window, wait for an answer nobody gives) whether to delete
//! an item for good when the bin will not take it. Platter never lets it come to that: an item is only
//! given to the shell when every rule below says the bin takes it.
//!
//! The rules (`C:` is the drive of the item, `{guid}` its volume as `mountvol` names it):
//!
//! - `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{guid}`, value `NukeOnDelete`:
//!   not 0 means the bin deletes at once. Refused.
//! - `HKCU` and `HKLM` `Software\Microsoft\Windows\CurrentVersion\Policies\Explorer`, value `NoRecycleFiles`:
//!   not 0 means "do not move deleted files to the Recycle Bin". Refused.
//! - The same keys, value `RecycleBinSize` (a policy that sets the bin's size): present with any value is
//!   refused, because what it does to the size is not something Platter reads reliably.
//! - `MaxCapacity` of the volume's key is the size of the bin in MB. An item is refused when its size is
//!   not below it. When there is no `MaxCapacity` the default is taken to be [`DEFAULT_PERCENT`] of the
//!   volume's total size. That is the low end of what Windows documents (Windows has used 10% of the first
//!   40 GB and 5% of the rest, and 5% for the whole volume), so the default can only refuse more.
//! - A size is counted as whole [`CLUSTER`]-byte blocks per file, because the shell may count what the
//!   file takes on the disk, not its length. [`CLUSTER`] is 64 KiB, the largest cluster NTFS and ReFS
//!   use, so a volume formatted with big clusters is never undercounted (over-counting only refuses more).
//!   Reading the real cluster size needs a system call that has no safe API in the project.
//! - A folder moved to the bin is renamed to `<drive>:\$Recycle.Bin\<SID>\$Rxxxxxx`, and everything below
//!   it keeps its relative path, so a path that is short where it is can be longer than `MAX_PATH` (260)
//!   in the bin, and then the shell asks whether to delete it for good. [`fits_bin_path`] refuses an item
//!   when [`BIN_PREFIX_UNITS`] plus its name plus the longest path below it reaches [`MAX_PATH_UNITS`].
//!
//! What cannot be read is not guessed: a value that is not a `REG_DWORD` is an error.

use std::fmt::Write as _;

/// The share of the volume's size the bin is taken to hold when the volume's key has no `MaxCapacity`.
pub const DEFAULT_PERCENT: u64 = 5;

/// A file is counted in whole blocks of this many bytes: the largest cluster size of NTFS and ReFS.
pub const CLUSTER: u64 = 64 * 1024;

/// The UTF-16 units of `MAX_PATH`, the terminating NUL included: a path in the bin has to be shorter.
pub const MAX_PATH_UNITS: usize = 260;

/// An upper bound for what comes before the name of an item in the bin, `C:\$Recycle.Bin\S-1-5-21-a-b-c-d\`
/// and the `\$Rxxxxxx` of the item: 3 + 12 + 1 + at most 61 for a SID (`S-1-5-21-` and four numbers of at
/// most 10 digits) + 1 + 8 = 86, and some room.
pub const BIN_PREFIX_UNITS: usize = 100;

/// Whether an item whose name is `name_units` UTF-16 units long, and whose longest path below it
/// (each name with the separator before it, `\a\b`) is `longest_below_units` long, keeps a path shorter
/// than `MAX_PATH` in the bin. The name is counted in whole (the bin names it `$R` and six characters
/// plus the extension, which is not longer).
pub fn fits_bin_path(name_units: usize, longest_below_units: usize) -> Result<(), String> {
    let longest = BIN_PREFIX_UNITS
        .saturating_add(name_units)
        .saturating_add(longest_below_units);
    if longest < MAX_PATH_UNITS {
        return Ok(());
    }

    Err(format!(
        "its path in the Recycle Bin would reach {longest} characters (Windows allows {}), so Windows would ask to delete it for good",
        MAX_PATH_UNITS - 1
    ))
}

/// One value that `reg query` printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegValue {
    pub name: String,
    /// `REG_DWORD`, `REG_SZ` and so on, as `reg` prints it.
    pub kind: String,
    pub data: String,
}

/// The values `reg query <key>` printed, in order. A line that is not a value (the key's own name, a
/// subkey, an empty line) is skipped. A value's line is its name, its type (`REG_...`) and its data,
/// separated by whitespace, and starts with whitespace.
pub fn parse_reg_values(output: &str) -> Vec<RegValue> {
    let mut values = Vec::new();

    for line in output.lines() {
        if !line.starts_with(char::is_whitespace) {
            continue;
        }

        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(at) = words.iter().position(|word| word.starts_with("REG_")) else {
            continue;
        };
        if at == 0 {
            continue;
        }

        values.push(RegValue {
            name: words[..at].join(" "),
            kind: words[at].to_owned(),
            data: words[at + 1..].join(" "),
        });
    }

    values
}

/// The `REG_DWORD` called `name` among `values`: `Ok(None)` when there is none (a name is not case
/// sensitive), and an error when there is one that is not a number as `reg` prints it (`0x1f`).
pub fn dword(values: &[RegValue], name: &str) -> Result<Option<u32>, String> {
    let Some(value) = values.iter().find(|value| value.name.eq_ignore_ascii_case(name)) else {
        return Ok(None);
    };

    let digits = value
        .data
        .strip_prefix("0x")
        .or_else(|| value.data.strip_prefix("0X"));
    let number = match digits {
        Some(digits) if value.kind == "REG_DWORD" => u32::from_str_radix(digits, 16).ok(),
        _ => None,
    };

    match number {
        Some(number) => Ok(Some(number)),
        None => Err(format!(
            "the setting {name} is not a number Platter can read ({} {})",
            value.kind, value.data
        )),
    }
}

/// Whether `values` has a value called `name`, of any type.
pub fn has(values: &[RegValue], name: &str) -> bool {
    values.iter().any(|value| value.name.eq_ignore_ascii_case(name))
}

/// The `{guid}` (braces included) of the volume that `mountvol X:\ /L` printed: `\\?\Volume{guid}\`.
/// `None` for anything that does not look like that, so a message is never mistaken for a volume.
pub fn parse_volume_guid(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("\\\\?\\Volume") else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('{') else {
            continue;
        };
        let Some((guid, after)) = rest.split_once('}') else {
            continue;
        };
        if is_guid(guid) && after.trim_matches('\\').is_empty() {
            return Some(format!("{{{guid}}}"));
        }
    }

    None
}

/// `8-4-4-4-12` hexadecimal digits.
fn is_guid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();

    groups.len() == 5
        && groups.iter().zip([8usize, 4, 4, 4, 12]).all(|(group, length)| {
            group.len() == length && group.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

/// What was read of the settings that decide whether the bin takes an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    /// `NukeOnDelete` of the volume's key.
    pub nuke_on_delete: Option<u32>,
    /// `MaxCapacity` of the volume's key, in MB.
    pub max_capacity_mb: Option<u32>,
    /// `NoRecycleFiles` of the policy keys: the larger of what `HKCU` and `HKLM` say.
    pub no_recycle_files: Option<u32>,
    /// Whether either policy key has `RecycleBinSize`.
    pub size_policy: bool,
}

/// What the bin of one volume takes: made by [`Rules::new`] only when it takes anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    /// Bytes: an item has to be smaller than this.
    pub capacity: u64,
    /// Whether `capacity` is the default and not what the volume's key says.
    pub default_capacity: bool,
}

impl Rules {
    /// The rules for a volume of `volume_total` bytes, or why the bin takes nothing there.
    pub fn new(settings: &Settings, volume_total: u64) -> Result<Self, String> {
        if settings.nuke_on_delete.is_some_and(|value| value != 0) {
            return Err(
                "the Recycle Bin of this drive is set to delete files at once, not to keep them".to_owned(),
            );
        }
        if settings.no_recycle_files.is_some_and(|value| value != 0) {
            return Err("a policy says that deleted files do not go to the Recycle Bin".to_owned());
        }
        if settings.size_policy {
            return Err(
                "a policy sets the size of the Recycle Bin, and Platter does not read what it allows"
                    .to_owned(),
            );
        }

        Ok(match settings.max_capacity_mb {
            Some(megabytes) => Self {
                capacity: u64::from(megabytes).saturating_mul(1024 * 1024),
                default_capacity: false,
            },
            None => Self {
                capacity: volume_total / 100 * DEFAULT_PERCENT,
                default_capacity: true,
            },
        })
    }

    /// Whether the bin takes `size` bytes, counted as [`charged`] says: only when it is below the capacity.
    pub fn takes(&self, size: u64) -> Result<(), String> {
        if size < self.capacity {
            return Ok(());
        }

        let mut why = format!(
            "it takes at least {} and the Recycle Bin of this drive holds {}",
            megabytes(size),
            megabytes(self.capacity)
        );
        if self.default_capacity {
            let _ = write!(
                why,
                " (the size is not set, so {DEFAULT_PERCENT}% of the drive is assumed), so Windows would ask to delete it for good"
            );
        } else {
            why.push_str(", so Windows would ask to delete it for good");
        }

        Err(why)
    }
}

/// What a file of `length` bytes counts for: whole [`CLUSTER`] blocks.
pub fn charged(length: u64) -> u64 {
    length.div_ceil(CLUSTER).saturating_mul(CLUSTER)
}

fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1024 * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOLUME_KEY: &str = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BitBucket\\Volume\\{0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0}\r\n    MaxCapacity    REG_DWORD    0x400\r\n    NukeOnDelete    REG_DWORD    0x0\r\n\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BitBucket\\Volume\\{0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0}\\Sub\r\n";

    #[test]
    fn values_are_read_from_what_reg_query_prints() {
        let values = parse_reg_values(VOLUME_KEY);

        assert_eq!(values.len(), 2);
        assert_eq!(dword(&values, "MaxCapacity"), Ok(Some(1024)));
        assert_eq!(dword(&values, "nukeondelete"), Ok(Some(0)));
        assert_eq!(dword(&values, "NoRecycleFiles"), Ok(None));
    }

    #[test]
    fn a_value_with_spaces_in_its_name_and_data_is_kept_whole() {
        let values = parse_reg_values("HKEY_X\n    A Name    REG_SZ    some data here\n");

        assert_eq!(
            values,
            [RegValue {
                name: "A Name".to_owned(),
                kind: "REG_SZ".to_owned(),
                data: "some data here".to_owned()
            }]
        );
        assert!(has(&values, "a name"));
    }

    #[test]
    fn an_error_message_and_an_empty_output_have_no_values() {
        assert!(parse_reg_values("").is_empty());
        assert!(
            parse_reg_values("ERROR: The system was unable to find the specified registry key or value.\r\n")
                .is_empty()
        );
        assert!(parse_reg_values("HKEY_X\\REG_Key\r\n").is_empty());
    }

    #[test]
    fn a_setting_that_is_not_a_number_is_an_error_not_a_zero() {
        let text = parse_reg_values("HKEY_X\n    NukeOnDelete    REG_SZ    1\n");
        let wide = parse_reg_values("HKEY_X\n    NukeOnDelete    REG_DWORD    0xzz\n");
        let large = parse_reg_values("HKEY_X\n    NukeOnDelete    REG_DWORD    0x1ffffffff\n");
        let decimal = parse_reg_values("HKEY_X\n    NukeOnDelete    REG_DWORD    1\n");

        for values in [text, wide, large, decimal] {
            assert!(dword(&values, "NukeOnDelete").is_err(), "{values:?}");
        }
        assert_eq!(
            dword(
                &parse_reg_values("HKEY_X\n    MaxCapacity    REG_DWORD    0xFFFFFFFF\n"),
                "MaxCapacity"
            ),
            Ok(Some(u32::MAX))
        );
    }

    #[test]
    fn the_volume_is_the_guid_mountvol_prints() {
        assert_eq!(
            parse_volume_guid("    \\\\?\\Volume{0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0}\\\r\n").as_deref(),
            Some("{0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0}")
        );
        assert_eq!(
            parse_volume_guid("\\\\?\\Volume{0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0}\\").as_deref(),
            Some("{0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0}")
        );
    }

    #[test]
    fn anything_else_from_mountvol_is_not_a_volume() {
        for text in [
            "",
            "The volume name is {0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0}",
            "\\\\?\\Volume{not-a-guid}\\",
            "\\\\?\\Volume{0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0}\\more",
            "\\\\?\\Volume{0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f}\\",
            "Possible values for VolumeName along with current mount points are:",
        ] {
            assert_eq!(parse_volume_guid(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_bin_that_deletes_at_once_or_is_forbidden_takes_nothing() {
        let total = 100 * 1024 * 1024 * 1024;

        for settings in [
            Settings {
                nuke_on_delete: Some(1),
                ..Settings::default()
            },
            Settings {
                nuke_on_delete: Some(7),
                ..Settings::default()
            },
            Settings {
                no_recycle_files: Some(1),
                ..Settings::default()
            },
            Settings {
                size_policy: true,
                ..Settings::default()
            },
        ] {
            assert!(Rules::new(&settings, total).is_err(), "{settings:?}");
        }
        assert!(
            Rules::new(
                &Settings {
                    nuke_on_delete: Some(0),
                    no_recycle_files: Some(0),
                    ..Settings::default()
                },
                total
            )
            .is_ok()
        );
    }

    #[test]
    fn the_capacity_is_the_volumes_or_five_percent_of_it() {
        let total = 200 * 1024 * 1024 * 1024u64;
        let set = Rules::new(
            &Settings {
                max_capacity_mb: Some(1),
                ..Settings::default()
            },
            total,
        )
        .unwrap();
        let default = Rules::new(&Settings::default(), total).unwrap();

        assert_eq!((set.capacity, set.default_capacity), (1024 * 1024, false));
        assert_eq!(
            (default.capacity, default.default_capacity),
            (total / 100 * 5, true)
        );
    }

    #[test]
    fn an_item_is_taken_only_when_it_is_below_the_capacity() {
        let rules = Rules::new(
            &Settings {
                max_capacity_mb: Some(1),
                ..Settings::default()
            },
            1 << 40,
        )
        .unwrap();

        assert!(rules.takes(0).is_ok());
        assert!(rules.takes(1024 * 1024 - 1).is_ok());
        assert!(rules.takes(1024 * 1024).is_err());
        assert!(rules.takes(3 * 1024 * 1024).unwrap_err().contains("3 MB"));
        let none = Rules::new(
            &Settings {
                max_capacity_mb: Some(0),
                ..Settings::default()
            },
            1 << 40,
        )
        .unwrap();
        assert!(none.takes(0).is_err(), "a bin of no size takes nothing");
    }

    #[test]
    fn a_default_capacity_says_so_in_its_refusal() {
        let rules = Rules::new(&Settings::default(), 1024 * 1024 * 1024).unwrap();

        assert!(
            rules
                .takes(rules.capacity)
                .unwrap_err()
                .contains("5% of the drive is assumed")
        );
    }

    #[test]
    fn a_file_counts_in_whole_blocks() {
        assert_eq!(charged(0), 0);
        assert_eq!(charged(1), 65536);
        assert_eq!(charged(65536), 65536);
        assert_eq!(charged(65537), 131072);
        assert_eq!(charged(u64::MAX), u64::MAX, "it saturates");
    }

    #[test]
    fn a_path_that_would_pass_max_path_in_the_bin_is_refused() {
        // 100 + name + below must stay under 260.
        assert!(fits_bin_path(20, 0).is_ok());
        assert!(fits_bin_path(20, 139).is_ok(), "159 + 100 = 259");
        assert!(fits_bin_path(20, 140).is_err(), "260 is too long");
        assert!(fits_bin_path(160, 0).is_err(), "a file with a long name alone");
        assert!(fits_bin_path(159, 0).is_ok());
        assert!(fits_bin_path(usize::MAX, usize::MAX).is_err(), "it saturates");
        assert!(fits_bin_path(160, 0).unwrap_err().contains("259"));
    }
}
