//! The macOS rules that need no system call, compiled on every OS so that their tests run on Linux. They
//! exist from package M0 so the packages that fill them never edit this file.
//!
//! | Module | Filled by | What it is |
//! |---|---|---|
//! | [`same_disk`] | M1 | which devices and volume-root folders a scan may enter |
//! | [`trash_place`] | M2a | where in the trash a path goes: `~/.Trash`, or `<mount>/.Trashes/<uid>` |
//!
//! Only `macos` calls them, so on other OSes they are dead code by design.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

pub mod same_disk;
pub mod trash_place;
