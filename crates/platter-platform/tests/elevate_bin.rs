//! The root helper as `pkexec` starts it: what it answers to arguments it must refuse. Nothing here runs
//! a cleanup command, so the tests that would reach one are skipped as root.

#![cfg(target_os = "linux")]

use std::process::{Command, Output};

const HELPER: &str = env!("CARGO_BIN_EXE_platter-elevate");

fn run(args: &[&str]) -> Output {
    Command::new(HELPER)
        .args(args)
        .env_clear()
        .output()
        .expect("the helper starts")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn no_argument_a_repeat_an_option_or_an_unknown_id_exits_with_a_usage_error() {
    for args in [
        &[][..],
        &[""],
        &["apt-clean", "apt-clean"],
        &["apt-clean", "nope"],
        &["apt-clean", "--extra"],
        &["-apt-clean"],
        &[
            "paccache-keep-1",
            "paccache-keep-2",
            "paccache-keep-3",
            "apt-clean",
            "dnf-clean",
            "journal-vacuum",
            "coredumps",
            "apt-clean",
        ],
        &["apt-"],
        &["Apt-Clean"],
        &["../apt-clean"],
        &["journal-vacuum --rotate"],
        &["--help"],
    ] {
        let output = run(args);

        assert_eq!(output.status.code(), Some(64), "{args:?}");
        assert!(stderr(&output).starts_with("platter-elevate: "), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn a_known_id_is_refused_when_not_root() {
    if rustix::process::geteuid().is_root() {
        return;
    }

    for args in [&["apt-clean"][..], &["apt-clean", "journal-vacuum", "coredumps"]] {
        let output = run(args);

        assert_eq!(output.status.code(), Some(77), "{args:?}");
        assert!(stderr(&output).starts_with("platter-elevate: "), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn a_message_never_looks_like_pkexecs_refusal() {
    for args in [&[][..], &["nope"], &["apt-clean"], &["apt-clean", "dnf-clean"]] {
        let output = run(args);

        assert!(!stderr(&output).contains("Error executing command as another user"));
        assert!(!matches!(output.status.code(), Some(126 | 127)));
    }
}
