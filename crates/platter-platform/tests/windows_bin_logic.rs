//! The Recycle Bin pre-check's rules and parsers, and the shell time limit, run on every system: the files
//! are plain `std` code that `src/windows` uses on Windows, and are compiled here as well so that their own
//! tests (captured `reg.exe` and `mountvol.exe` output, the defaults, a call that never returns) run in
//! the Linux CI too.

#![allow(dead_code)]

#[path = "../src/windows/bin_text.rs"]
mod bin_text;

#[path = "../src/windows/time_limit.rs"]
mod time_limit;
