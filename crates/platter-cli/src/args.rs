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
    /// Scan a directory and write a JSON fixture for the frontend mock.
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

    /// Most entries to hold in memory before giving up (see `scan --help`).
    #[arg(long, value_name = "N")]
    pub max_entries: Option<u64>,
}
