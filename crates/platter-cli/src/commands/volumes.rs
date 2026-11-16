use platter_core::VolumeProvider;
use platter_core::dto::VolumeDto;
use platter_platform::Platform;

use super::{CliResult, print_line};
use crate::args::VolumesArgs;
use crate::format;

pub fn run(args: &VolumesArgs) -> CliResult<()> {
    let volumes = Platform::current().volumes.list();

    if args.json {
        let dtos: Vec<VolumeDto> = volumes.iter().map(VolumeDto::from).collect();
        print_line(&format::to_json(&dtos)?)?;
        return Ok(());
    }

    let rows: Vec<Vec<String>> = volumes
        .iter()
        .map(|volume| {
            let used = volume.total.saturating_sub(volume.available);
            vec![
                format::display(&volume.mount_point.to_string_lossy()),
                format::display(&volume.label),
                format::display(&volume.device),
                format::display(&volume.fs_type),
                format::bytes(volume.total),
                format::bytes(volume.available),
                format!("{:.0}%", format::percent(used, volume.total)),
                if volume.removable { "yes" } else { "" }.to_owned(),
            ]
        })
        .collect();

    print_line(&format::table(
        &[
            "MOUNT",
            "LABEL",
            "DEVICE",
            "TYPE",
            "SIZE",
            "FREE",
            "USED",
            "REMOVABLE",
        ],
        &rows,
    ))?;
    Ok(())
}
