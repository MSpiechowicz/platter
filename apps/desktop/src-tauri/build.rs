use std::path::PathBuf;

fn main() {
    // Build scripts run on the host, so the target comes from Cargo's cfg variables.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_manifest_in_every_binary();
    } else {
        tauri_build::build();
    }
}

/// Tauri's default manifest only reaches the app binary through a resource file. Test binaries
/// lack it, and without Common Controls v6 they fail to load (STATUS_ENTRYPOINT_NOT_FOUND from
/// the TaskDialogIndirect import). The same manifest is therefore linked into every binary,
/// the app included, instead of being added as a resource.
fn embed_manifest_in_every_binary() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("windows-app-manifest.xml");

    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run the Tauri build script");

    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
