//! On macOS, links clang's runtime library (`libclang_rt.osx.a`) into every binary that has llama.cpp.
//!
//! llama.cpp's Metal code asks at run time whether the system is new enough for some features
//! (`@available`). When the binary is built for an older macOS than the one it is built on (the release
//! targets macOS 13, `MACOSX_DEPLOYMENT_TARGET`), that question becomes a call to
//! `__isPlatformVersionAtLeast`, which lives in clang's runtime library. Rust links with
//! `-nodefaultlibs`, so that library is left out and the link fails. Linking instructions given here
//! reach every binary that depends on this crate: the app, the CLI and the tests.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

    // Build scripts run on the host, so the target comes from Cargo's cfg variables.
    let for_macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    let with_llama = std::env::var_os("CARGO_FEATURE_LLAMA").is_some();
    if !(for_macos && with_llama) {
        return;
    }

    match clang_runtime_folder() {
        Some(folder) => {
            println!("cargo:rustc-link-search=native={}", folder.display());
            println!("cargo:rustc-link-lib=static=clang_rt.osx");
        }
        None => println!(
            "cargo:warning=clang's runtime library (libclang_rt.osx.a) was not found; a build for an older \
             macOS than this one may fail to link (__isPlatformVersionAtLeast)"
        ),
    }
}

/// The folder of `libclang_rt.osx.a`: `<clang's resource folder>/lib/darwin`.
fn clang_runtime_folder() -> Option<PathBuf> {
    let output = Command::new("xcrun")
        .args(["clang", "--print-resource-dir"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let resources = String::from_utf8(output.stdout).ok()?;
    let folder = PathBuf::from(resources.trim()).join("lib").join("darwin");

    folder.join("libclang_rt.osx.a").is_file().then_some(folder)
}
