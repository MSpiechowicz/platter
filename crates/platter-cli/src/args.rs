use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "platter-cli", version, about = "Local disk-space analyzer, headless")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List mounted volumes.
    Volumes(VolumesArgs),
    /// Scan a directory and print totals and its largest children.
    Scan(ScanArgs),
    /// Scan a directory and list what could be cleaned (read-only: nothing is changed).
    Suggest(SuggestArgs),
    /// Clean what rules find in a directory: `--dry-run` shows the plan, `--yes` carries it out.
    Clean(CleanArgs),
    /// Scan a directory and write a JSON snapshot of it: totals, a level-of-detail tree and a summary.
    ExportFixture(ExportArgs),
}

#[derive(Debug, Args)]
pub struct VolumesArgs {
    /// Print JSON (the `VolumeDto[]` wire format) instead of a table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    pub path: PathBuf,

    /// Stay on the scan root's disk: skip other filesystems (on Linux, subvolumes of the same btrfs
    /// filesystem count as the same disk and are scanned).
    #[arg(long)]
    pub one_file_system: bool,

    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,

    /// How many of the largest children to list.
    #[arg(long, default_value_t = 10)]
    pub top: usize,

    /// Also print entry count, elapsed time, throughput and memory per entry.
    #[arg(long)]
    pub stats: bool,

    /// Leave this folder out of the scan (repeatable). It is listed but never entered, counted or
    /// cleaned. A folder that does not exist is ignored. Relative paths are taken from the current
    /// folder.
    #[arg(long = "exclude", value_name = "PATH")]
    pub excluded: Vec<PathBuf>,

    /// Most entries to hold in memory before giving up (default: no cap of its own; a scan may use at most
    /// half of the free memory in any case).
    #[arg(long, value_name = "N")]
    pub max_entries: Option<u64>,
}

#[derive(Debug, Args)]
pub struct SuggestArgs {
    pub path: PathBuf,

    /// Stay on the scan root's disk (see `scan --help`).
    #[arg(long)]
    pub one_file_system: bool,

    /// Print JSON (the `SuggestionsDto` wire format) instead of a table.
    #[arg(long)]
    pub json: bool,

    /// Also print how long the scan and the rules took (to stderr).
    #[arg(long)]
    pub stats: bool,

    /// Leave this folder out of the scan (repeatable). It is listed but never entered, counted or
    /// cleaned. A folder that does not exist is ignored. Relative paths are taken from the current
    /// folder.
    #[arg(long = "exclude", value_name = "PATH")]
    pub excluded: Vec<PathBuf>,

    /// Most entries to hold in memory before giving up (see `scan --help`).
    #[arg(long, value_name = "N")]
    pub max_entries: Option<u64>,
}

#[derive(Debug, Args)]
pub struct CleanArgs {
    pub path: PathBuf,

    /// Clean what this rule finds (repeatable; see `suggest` for the ids).
    #[arg(long = "rule", value_name = "ID")]
    pub rules: Vec<String>,

    /// Clean what every `safe` rule finds.
    #[arg(long)]
    pub safe: bool,

    /// Show what would be cleaned and change nothing.
    #[arg(long, conflicts_with = "yes")]
    pub dry_run: bool,

    /// Clean for real: delete, empty and move to the trash what the plan lists. Without it (and without
    /// `--dry-run`) nothing is done.
    #[arg(long)]
    pub yes: bool,

    /// List every target, not only the totals per rule.
    #[arg(long)]
    pub verbose: bool,

    /// Stay on the scan root's disk (see `scan --help`).
    #[arg(long)]
    pub one_file_system: bool,

    /// Leave this folder out of the scan (repeatable). It is listed but never entered, counted or
    /// cleaned. A folder that does not exist is ignored. Relative paths are taken from the current
    /// folder.
    #[arg(long = "exclude", value_name = "PATH")]
    pub excluded: Vec<PathBuf>,

    /// Most entries to hold in memory before giving up (see `scan --help`).
    #[arg(long, value_name = "N")]
    pub max_entries: Option<u64>,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    pub path: PathBuf,

    /// File to write.
    #[arg(long)]
    pub out: PathBuf,

    /// Upper bound on nodes in the exported tree.
    #[arg(long, default_value_t = 1500)]
    pub max_nodes: usize,

    /// Upper bound on depth of the exported tree.
    #[arg(long, default_value_t = 10)]
    pub max_depth: usize,

    /// Stay on the scan root's disk (see `scan --help`).
    #[arg(long)]
    pub one_file_system: bool,

    /// Leave this folder out of the scan (repeatable). It is listed but never entered, counted or
    /// cleaned. A folder that does not exist is ignored. Relative paths are taken from the current
    /// folder.
    #[arg(long = "exclude", value_name = "PATH")]
    pub excluded: Vec<PathBuf>,

    /// Most entries to hold in memory before giving up (see `scan --help`).
    #[arg(long, value_name = "N")]
    pub max_entries: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Command {
        let mut argv = vec!["platter-cli"];
        argv.extend_from_slice(args);

        Cli::try_parse_from(argv).expect("the arguments parse").command
    }

    fn excluded(command: Command) -> Vec<PathBuf> {
        match command {
            Command::Scan(args) => args.excluded,
            Command::Suggest(args) => args.excluded,
            Command::Clean(args) => args.excluded,
            Command::ExportFixture(args) => args.excluded,
            Command::Volumes(_) => panic!("volumes takes no folders to leave out"),
        }
    }

    #[test]
    fn exclude_is_repeatable_and_keeps_the_order_given() {
        let command = parse(&["scan", "/data", "--exclude", "/data/a", "--exclude", "/data/b"]);

        assert_eq!(
            excluded(command),
            [PathBuf::from("/data/a"), PathBuf::from("/data/b")]
        );
    }

    #[test]
    fn no_exclude_means_an_empty_list() {
        assert!(excluded(parse(&["scan", "/data"])).is_empty());
    }

    #[test]
    fn every_command_that_scans_takes_exclude() {
        for args in [
            &["suggest", "/data", "--exclude", "/x"][..],
            &["clean", "/data", "--dry-run", "--exclude", "/x"][..],
            &["export-fixture", "/data", "--out", "f.json", "--exclude", "/x"][..],
        ] {
            assert_eq!(excluded(parse(args)), [PathBuf::from("/x")], "{args:?}");
        }
    }

    #[test]
    fn exclude_without_a_path_is_refused() {
        assert!(Cli::try_parse_from(["platter-cli", "scan", "/data", "--exclude"]).is_err());
    }

    #[test]
    fn volumes_does_not_take_exclude() {
        assert!(Cli::try_parse_from(["platter-cli", "volumes", "--exclude", "/x"]).is_err());
    }
}
