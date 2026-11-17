use std::path::Path;

use platter_core::NodeId;
use platter_core::dto::ScanId;
use platter_platform::paths::node_path;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Reveals a scanned node in the system file manager.
///
/// The path is rebuilt from the tree, never taken from the UI, so the webview cannot make the backend
/// open an arbitrary location. It is the exact path, so a name that is not valid UTF-8 still resolves.
#[tauri::command]
pub async fn open_in_file_manager(
    state: State<'_, AppState>,
    scan: ScanId,
    node: NodeId,
) -> CommandResult<()> {
    let tree = state.scans.tree(scan)?;
    if !tree.contains(node) {
        return Err(CommandError::not_found(format!("node {node} does not exist")));
    }

    let path = node_path(&tree, node);
    tauri::async_runtime::spawn_blocking(move || tauri_plugin_opener::reveal_item_in_dir(&path))
        .await?
        .map_err(|error| CommandError::io(format!("cannot show the item in the file manager: {error}")))
}

/// Opens the system folder dialog. `None` when the user dismisses it.
///
/// The wire type is a JSON string, so a folder whose path is not valid Unicode (a Unix name of raw
/// bytes) cannot be handed to the UI and back to `start_scan`. It is refused with a clear message
/// instead of being scanned under a mangled name or lossily converted to a path that does not exist.
/// Remembering the `PathBuf` in backend state under an opaque token would lift this, but it adds a
/// second way to name a scan root to the contract (`ScanOptions.root` would be a path or a token), which
/// is not worth it for a rare case; such a folder can still be reached by picking its parent.
#[tauri::command]
pub async fn pick_folder(app: AppHandle, state: State<'_, AppState>) -> CommandResult<Option<String>> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a folder to scan")
            .blocking_pick_folder()
    })
    .await?;

    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|error| {
        CommandError::invalid(format!("the chosen location is not a local folder: {error}"))
    })?;

    let picked = wire_path(&path)?;
    // The folder the user chose in the backend's own dialog may now be scanned, as the folder it is
    // now: its identity is pinned, so swapping it for a link before the scan is refused. Reading it can
    // stall on a hung mount, so off the async worker; without an identity the pick is refused.
    let identity = tauri::async_runtime::spawn_blocking(move || {
        platter_platform::root::resolve_root(&path)
            .ok()
            .and_then(|resolved| resolved.identity)
    })
    .await?
    .ok_or_else(|| CommandError::invalid("the chosen folder cannot be read, so choose another one"))?;
    state.allow_picked(picked.clone(), Some(identity));
    Ok(Some(picked))
}

/// The path as the UI receives it, or an error when it has no exact text form.
fn wire_path(path: &Path) -> CommandResult<String> {
    match path.to_str() {
        Some(text) => Ok(text.to_owned()),
        None => Err(CommandError::invalid(format!(
            "the folder {} has a name that is not valid text, so it cannot be scanned from the \
             folder picker. Choose its parent folder instead",
            path.display()
        ))),
    }
}

/// The fixed project pages the UI may ask the backend to open. The wire form is the contract's
/// `ExternalLink`: `'source' | 'issues' | 'license'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalLink {
    Source,
    Issues,
    License,
}

impl ExternalLink {
    pub const fn url(self) -> &'static str {
        match self {
            Self::Source => "https://github.com/MSpiechowicz/platter",
            Self::Issues => "https://github.com/MSpiechowicz/platter/issues",
            Self::License => "https://www.gnu.org/licenses/gpl-3.0.html",
        }
    }
}

/// Opens one of the fixed project pages in the default browser.
///
/// The URL is a constant on the Rust side; the webview names a page and can never pass a URL. The opener
/// plugin is called through its Rust API only, so the webview is granted no opener permission.
#[tauri::command]
pub async fn open_external(link: ExternalLink) -> CommandResult<()> {
    tauri::async_runtime::spawn_blocking(move || tauri_plugin_opener::open_url(link.url(), None::<&str>))
        .await?
        .map_err(|error| CommandError::io(format!("cannot open the page in the browser: {error}")))
}

#[cfg(test)]
mod tests {
    use platter_core::dto::ApiErrorCode;
    use platter_platform::testing::non_utf8_path;

    use super::*;

    #[test]
    fn a_picked_folder_with_a_valid_text_path_is_returned_unchanged() {
        assert_eq!(
            wire_path(Path::new("/home/me/Zdjęcia")).unwrap(),
            "/home/me/Zdjęcia"
        );
    }

    #[test]
    fn a_picked_folder_whose_name_is_not_valid_text_is_refused_with_a_clear_message() {
        let error = wire_path(&non_utf8_path(Path::new("/home/me"))).unwrap_err();

        assert_eq!(error.code(), ApiErrorCode::Invalid);
        assert!(
            format!("{error:?}").contains("Choose its parent folder"),
            "{error:?}"
        );
    }

    #[test]
    fn the_wire_format_is_the_contracts_lowercase_names() {
        for (name, link) in [
            ("source", ExternalLink::Source),
            ("issues", ExternalLink::Issues),
            ("license", ExternalLink::License),
        ] {
            let wire = serde_json::json!(name);

            assert_eq!(
                serde_json::from_value::<ExternalLink>(wire.clone()).unwrap(),
                link
            );
            assert_eq!(serde_json::to_value(link).unwrap(), wire);
        }
    }

    #[test]
    fn anything_but_a_known_name_is_rejected() {
        for wire in ["https://example.com", "Source", "", "sources"] {
            assert!(
                serde_json::from_value::<ExternalLink>(serde_json::json!(wire)).is_err(),
                "{wire:?} must not be accepted"
            );
        }
    }

    #[test]
    fn each_link_maps_to_its_project_page() {
        assert_eq!(
            ExternalLink::Source.url(),
            "https://github.com/MSpiechowicz/platter"
        );
        assert_eq!(
            ExternalLink::Issues.url(),
            "https://github.com/MSpiechowicz/platter/issues"
        );
        assert_eq!(
            ExternalLink::License.url(),
            "https://www.gnu.org/licenses/gpl-3.0.html"
        );
    }
}
