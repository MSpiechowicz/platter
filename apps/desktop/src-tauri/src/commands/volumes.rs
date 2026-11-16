use platter_core::VolumeProvider;
use platter_core::dto::VolumeDto;
use tauri::State;

use crate::error::CommandResult;
use crate::state::AppState;

/// Lists mounted volumes. Reading mount tables and free space can stall on a hung network mount, so it
/// runs on the blocking pool instead of an async worker.
#[tauri::command]
pub async fn list_volumes(state: State<'_, AppState>) -> CommandResult<Vec<VolumeDto>> {
    let platform = state.platform.clone();

    let volumes = tauri::async_runtime::spawn_blocking(move || platform.volumes.list()).await?;
    let volumes: Vec<VolumeDto> = volumes.iter().map(VolumeDto::from).collect();

    // These mount points may now be scanned.
    state.allow_roots(volumes.iter().map(|volume| volume.mount_point.clone()));
    Ok(volumes)
}
