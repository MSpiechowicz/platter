use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use platter_core::dto::{LodNode, SummaryDto, VolumeDto};
use platter_core::{Totals, VolumeProvider, query};
use platter_platform::Platform;
use serde::Serialize;

use super::{CliResult, scan_path};
use crate::args::ExportArgs;
use crate::format;

/// The file the frontend mock loads: `{ volume?, totals, root, summary }`.
#[derive(Serialize)]
struct Fixture {
    #[serde(skip_serializing_if = "Option::is_none")]
    volume: Option<VolumeDto>,
    totals: Totals,
    root: LodNode,
    summary: SummaryDto,
}

pub fn run(args: &ExportArgs) -> CliResult<()> {
    let tree = scan_path(&args.path, args.one_file_system, args.max_entries, &args.excluded)?;

    let fixture = Fixture {
        volume: volume_of(&args.path),
        totals: tree.totals(),
        root: query::lod(&tree, tree.root(), args.max_nodes, args.max_depth)?,
        summary: query::summary(&tree, tree.root())?,
    };

    if let Some(parent) = args.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let json = format::json_safe(&serde_json::to_string(&fixture)?);
    BufWriter::new(File::create(&args.out)?).write_all(json.as_bytes())?;

    eprintln!("wrote {}", format::display(&args.out.to_string_lossy()));
    Ok(())
}

/// The volume whose mount point is the longest prefix of `path`, when it can be determined. The path is
/// resolved as a scan resolves its root, so on Windows it is `C:\...` as a volume's mount point is, not a
/// verbatim `\\?\C:\...`.
fn volume_of(path: &Path) -> Option<VolumeDto> {
    let canonical = platter_platform::root::resolve_root(path).ok()?.path;

    Platform::current()
        .volumes
        .list()
        .iter()
        .filter(|volume| canonical.starts_with(&volume.mount_point))
        .max_by_key(|volume| volume.mount_point.components().count())
        .map(VolumeDto::from)
}
