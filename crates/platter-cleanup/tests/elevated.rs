//! The commands that run as the administrator are exactly the closed list the root helper knows.

use platter_cleanup::{CleanupAction, CleanupOptions, KeepVersions, action_for};
use platter_core::{ElevatedTool, Os};
use platter_rules::RuleSet;

/// Every elevated tool any Linux rule can give, under every number of kept versions.
fn linux_elevated_tools() -> Vec<(&'static str, platter_cleanup::Tool)> {
    let mut found = Vec::new();

    for rule in RuleSet::builtin().rules() {
        for keep in [KeepVersions::One, KeepVersions::Two, KeepVersions::Three] {
            let options = CleanupOptions {
                keep_package_versions: keep,
                ask_for_admin: true,
            };

            if let Some(CleanupAction::RunTool(tool)) = action_for(rule.id, &options, Os::Linux)
                && tool.needs_elevation()
            {
                found.push((rule.id, tool));
            }
        }
    }

    found
}

#[test]
fn every_elevated_tool_of_a_linux_rule_is_on_the_helpers_list() {
    let found = linux_elevated_tools();

    assert!(!found.is_empty());
    for (rule, tool) in found {
        assert!(
            ElevatedTool::find(tool.program(), tool.args()).is_some(),
            "{rule}: {} {:?}",
            tool.program(),
            tool.args()
        );
    }
}

#[test]
fn every_command_on_the_helpers_list_is_used_by_a_rule() {
    let found = linux_elevated_tools();

    for wanted in ElevatedTool::ALL {
        assert!(
            found
                .iter()
                .any(|(_, tool)| ElevatedTool::find(tool.program(), tool.args()) == Some(wanted)),
            "{}",
            wanted.id()
        );
    }
}

#[test]
fn the_pacman_cache_of_each_keep_count_is_its_own_command() {
    let command = |keep| {
        let options = CleanupOptions {
            keep_package_versions: keep,
            ask_for_admin: true,
        };
        match action_for("pacman-cache", &options, Os::Linux) {
            Some(CleanupAction::RunTool(tool)) => ElevatedTool::find(tool.program(), tool.args()),
            other => panic!("not a tool: {other:?}"),
        }
    };

    assert_eq!(command(KeepVersions::One), Some(ElevatedTool::Paccache1));
    assert_eq!(command(KeepVersions::Two), Some(ElevatedTool::Paccache2));
    assert_eq!(command(KeepVersions::Three), Some(ElevatedTool::Paccache3));
}

#[test]
fn docker_is_not_elevated_and_not_on_the_list() {
    let options = CleanupOptions::default();

    match action_for("docker-data", &options, Os::Linux) {
        Some(CleanupAction::RunTool(tool)) => {
            assert!(!tool.needs_elevation());
            assert_eq!(ElevatedTool::find(tool.program(), tool.args()), None);
        }
        other => panic!("not a tool: {other:?}"),
    }
}
