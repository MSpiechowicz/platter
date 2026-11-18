//! The Linux packages, the polkit policy and the runner agree on one helper path and one prompt per run.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};

use platter_platform::linux::tools::HELPER;
use serde_json::Value;

const POLICY_FILE: &str = "io.github.mspiechowicz.platter.policy";

fn src_tauri(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(file)
}

fn policy() -> String {
    std::fs::read_to_string(src_tauri(&format!("linux/{POLICY_FILE}"))).expect("the policy file exists")
}

/// The text between `<tag ...>` and `</tag>`, the first time it appears.
fn element<'a>(xml: &'a str, tag: &str) -> &'a str {
    let open = format!("<{tag}");
    let start = xml.find(&open).unwrap_or_else(|| panic!("no <{tag}>"));
    let body = start + xml[start..].find('>').expect("the tag closes") + 1;
    let end = body
        + xml[body..]
            .find(&format!("</{tag}>"))
            .expect("the element closes");

    xml[body..end].trim()
}

fn linux_config() -> Value {
    let text = std::fs::read_to_string(src_tauri("tauri.linux.conf.json")).expect("the Linux config exists");

    serde_json::from_str(&text).expect("the Linux config is JSON")
}

fn package(config: &Value, kind: &str) -> Value {
    config["bundle"]["linux"][kind].clone()
}

#[test]
fn the_policy_runs_the_helper_the_runner_asks_for() {
    let policy = policy();

    assert_eq!(element(&policy, "annotate"), HELPER);
    assert!(policy.contains(r#"key="org.freedesktop.policykit.exec.path""#));
}

#[test]
fn the_policy_asks_for_a_password_every_time_and_remembers_none() {
    let policy = policy();

    assert_eq!(element(&policy, "allow_any"), "auth_admin");
    assert_eq!(element(&policy, "allow_inactive"), "auth_admin");
    assert_eq!(element(&policy, "allow_active"), "auth_admin");
    assert!(!policy.contains("_keep"));
}

#[test]
fn the_policy_action_id_is_the_app_identifier_and_its_message_has_no_semicolon() {
    let policy = policy();
    let tauri: Value =
        serde_json::from_str(&std::fs::read_to_string(src_tauri("tauri.conf.json")).unwrap()).unwrap();
    let identifier = tauri["identifier"].as_str().unwrap();

    assert!(policy.contains(&format!(r#"<action id="{identifier}.clean-system-caches">"#)));
    assert_eq!(POLICY_FILE, format!("{identifier}.policy"));
    assert!(!element(&policy, "message").contains(';'));
    assert!(!element(&policy, "description").contains(';'));
}

#[test]
fn the_deb_and_the_rpm_install_the_helper_and_the_policy() {
    let config = linux_config();
    let policy_path = format!("/usr/share/polkit-1/actions/{POLICY_FILE}");

    for kind in ["deb", "rpm"] {
        let files = &package(&config, kind)["files"];

        assert!(files[HELPER].is_string(), "{kind}: {HELPER}");
        assert_eq!(files[&policy_path], format!("linux/{POLICY_FILE}"), "{kind}");
        assert!(src_tauri(&format!("linux/{POLICY_FILE}")).is_file());
    }
}

#[test]
fn the_packages_need_polkit_and_keep_the_vulkan_loader() {
    let config = linux_config();
    let depends = |kind: &str| -> Vec<String> {
        package(&config, kind)["depends"]
            .as_array()
            .expect("depends is a list")
            .iter()
            .map(|dependency| dependency.as_str().unwrap().to_string())
            .collect()
    };

    assert_eq!(depends("deb"), ["libvulkan1", "pkexec | policykit-1"]);
    assert_eq!(depends("rpm"), ["vulkan-loader", "polkit"]);
}

#[test]
fn the_helper_is_built_before_the_bundle() {
    let config = linux_config();
    let command = config["build"]["beforeBundleCommand"].as_str().unwrap();

    assert!(command.contains("--bin platter-elevate"));
    assert!(command.contains("--release"));
}
