//! Asking whether the Recycle Bin will take an item, before the shell is asked to move it (W2).
//!
//! The shell may stop to ask whether to delete an item for good when the bin will not take it (a bin set
//! to delete at once, an item larger than the bin), and with no window nobody answers. So nothing is given
//! to the shell until the settings that decide it have been read and say yes: see [`bin_text`] for the
//! rules and the defaults. Everything here fails closed: a setting that cannot be read, a tool that does
//! not answer in time, a folder too large to measure is a refusal, and the item is not touched.
//!
//! The settings are read with `reg.exe` and `mountvol.exe`, the programs of `System32` found through the
//! known folders (never `PATH`), with a cleared environment, no window, no input and a time limit each,
//! because the project has no safe registry API. A value that is missing is read as missing only when a
//! key that always exists answers: a `reg` that fails for another reason is an error, not "no setting".

use std::io::{self, Read};
use std::os::windows::fs::MetadataExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use known_folders::{KnownFolder, get_known_folder_path};

use super::bin_text::{RegValue, Rules, Settings, charged, dword, has, parse_reg_values, parse_volume_guid};

/// `CREATE_NO_WINDOW`.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `FILE_ATTRIBUTE_REPARSE_POINT`: a link of any kind, which is never entered.
const REPARSE_POINT: u32 = 0x400;
/// How long one of the tools may take.
const TOOL_LIMIT: Duration = Duration::from_secs(15);
/// How much of a tool's output is read.
const TOOL_OUTPUT_BYTES: u64 = 1024 * 1024;

/// How many entries of a folder are looked at to measure it, and for how long.
const MEASURE_ENTRIES: usize = 200_000;
const MEASURE_LIMIT: Duration = Duration::from_secs(30);

const HKCU_BASE: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion";
const HKLM_BASE: &str = "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion";

/// What the Recycle Bin of the drive `drive_root` (`C:\`, `volume_total` bytes) takes, or why it takes
/// nothing. `Unsupported` when the settings say it will not take what the shell would be given, `Other`
/// when the settings could not be read: in both the shell is not called.
pub(super) fn rules_for(drive_root: &str, volume_total: u64) -> io::Result<Rules> {
    let settings = read_settings(drive_root).map_err(|error| {
        io::Error::other(format!(
            "the settings of the Recycle Bin could not be read, so nothing is moved there ({error})"
        ))
    })?;

    Rules::new(&settings, volume_total).map_err(|why| refused(&why))
}

/// The refusal of an item the bin would not take, for a message.
pub(super) fn refusal_text(why: &str) -> String {
    format!("the Recycle Bin would not keep it, so it is left where it is: {why}")
}

/// The same as an error (`Unsupported`): nothing was done to the item.
pub(super) fn refused(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, refusal_text(why))
}

/// Whether the bin takes the item at `path`: a file of `file_length` bytes, or a folder, which is measured
/// (links are not entered, a file counts in whole blocks). A folder with more than [`MEASURE_ENTRIES`]
/// entries, or one that takes more than [`MEASURE_LIMIT`] to look at, is refused: it cannot be known to fit.
pub(super) fn takes(rules: &Rules, path: &Path, is_dir: bool, file_length: u64) -> Result<(), String> {
    if !is_dir {
        return rules.takes(charged(file_length));
    }

    let started = Instant::now();
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut pending = vec![path.to_path_buf()];

    while let Some(folder) = pending.pop() {
        let unreadable = |error: io::Error| format!("it could not be measured ({error})");
        for entry in std::fs::read_dir(&folder).map_err(unreadable)? {
            let entry = entry.map_err(unreadable)?;

            seen += 1;
            if seen > MEASURE_ENTRIES || started.elapsed() > MEASURE_LIMIT {
                return Err(format!(
                    "it holds too much to measure (over {MEASURE_ENTRIES} items, or more than {} s)",
                    MEASURE_LIMIT.as_secs()
                ));
            }

            let meta = entry.metadata().map_err(unreadable)?;
            if meta.file_attributes() & REPARSE_POINT != 0 {
                continue;
            }
            if meta.is_dir() {
                pending.push(entry.path());
                continue;
            }

            total = total.saturating_add(charged(meta.len()));
            rules.takes(total)?;
        }
    }

    rules.takes(total)
}

/// The settings of the drive's bin and the policies. An error is a failure to read, not a refusal.
fn read_settings(drive_root: &str) -> io::Result<Settings> {
    let mountvol = system_tool("mountvol.exe")?;
    let reg = system_tool("reg.exe")?;

    let (code, output) = run(&mountvol, &[drive_root, "/L"])?;
    let guid = match (code, parse_volume_guid(&output)) {
        (0, Some(guid)) => guid,
        _ => {
            return Err(io::Error::other(format!(
                "mountvol did not name the volume of {drive_root}"
            )));
        }
    };

    let volume_key = format!("{HKCU_BASE}\\Explorer\\BitBucket\\Volume\\{guid}");
    let volume = query_key(&reg, &volume_key, HKCU_BASE)?;
    let user_policy = query_key(&reg, &format!("{HKCU_BASE}\\Policies\\Explorer"), HKCU_BASE)?;
    let machine_policy = query_key(&reg, &format!("{HKLM_BASE}\\Policies\\Explorer"), HKLM_BASE)?;

    let readable = |error: String| io::Error::other(error);
    let mut settings = Settings {
        nuke_on_delete: dword(&volume, "NukeOnDelete").map_err(readable)?,
        max_capacity_mb: dword(&volume, "MaxCapacity").map_err(readable)?,
        no_recycle_files: None,
        size_policy: false,
    };
    for policy in [&user_policy, &machine_policy] {
        let no_recycle = dword(policy, "NoRecycleFiles").map_err(readable)?;
        settings.no_recycle_files = settings.no_recycle_files.max(no_recycle);
        settings.size_policy |= has(policy, "RecycleBinSize");
    }

    Ok(settings)
}

/// The values of `key`, none when the key is not there. Exit code 1 is how `reg` says "not found", and
/// also how it says most other things, so it is taken as "not found" only when `exists`, a key that is
/// always there, answers.
fn query_key(reg: &Path, key: &str, exists: &str) -> io::Result<Vec<RegValue>> {
    let (code, output) = run(reg, &["query", key])?;
    match code {
        0 => Ok(parse_reg_values(&output)),
        1 => match run(reg, &["query", exists])? {
            (0, _) => Ok(Vec::new()),
            (other, _) => Err(io::Error::other(format!(
                "reg failed ({other}) on a key that always exists"
            ))),
        },
        other => Err(io::Error::other(format!("reg query {key} ended with {other}"))),
    }
}

/// A program of `System32`, found through the known folders, a regular file.
fn system_tool(name: &str) -> io::Result<PathBuf> {
    let path = get_known_folder_path(KnownFolder::System)
        .ok_or_else(|| io::Error::other("the System32 folder is not known"))?
        .join(name);
    if !std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name} is not in System32"),
        ));
    }

    Ok(path)
}

/// Runs `program` with `args`: its exit code and what it printed to its output (lossy text), within
/// [`TOOL_LIMIT`]. Killed and an error when it takes longer.
fn run(program: &Path, args: &[&str]) -> io::Result<(i32, String)> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    for name in ["SystemRoot", "windir"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }

    let mut child = command.spawn()?;
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);

    let deadline = Instant::now() + TOOL_LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{} did not answer in {} s",
                    program.display(),
                    TOOL_LIMIT.as_secs()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let text = stdout
        .map(|reader| reader.join().unwrap_or_default())
        .unwrap_or_default();
    let _ = stderr.map(|reader| reader.join());
    let code = status
        .code()
        .ok_or_else(|| io::Error::other(format!("{} ended without an exit code", program.display())))?;

    Ok((code, String::from_utf8_lossy(&text).into_owned()))
}

/// Reads a pipe in its own thread to its end (keeping [`TOOL_OUTPUT_BYTES`] of it), so a program that
/// writes a lot never waits for the reader.
fn drain<R: Read + Send + 'static>(mut pipe: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let _ = (&mut pipe).take(TOOL_OUTPUT_BYTES).read_to_end(&mut kept);
        let _ = io::copy(&mut pipe, &mut io::sink());

        kept
    })
}
