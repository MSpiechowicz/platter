//! Live progress line on stderr, shown only when stderr is a terminal.

use std::io::{IsTerminal, Write};

use platter_core::Progress;

use crate::format;

pub fn enabled() -> bool {
    std::io::stderr().is_terminal()
}

pub fn show(progress: &Progress) {
    let mut path = format::display(&progress.current_path);
    if path.chars().count() > 60 {
        let tail: String = path
            .chars()
            .rev()
            .take(59)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        path = format!("…{tail}");
    }

    let mut err = std::io::stderr().lock();
    let _ = write!(
        err,
        "\r\x1b[K{} files, {} dirs, {}, {} errors  {path}",
        progress.files,
        progress.dirs,
        format::bytes(progress.bytes),
        progress.errors
    );
    let _ = err.flush();
}

pub fn clear() {
    let mut err = std::io::stderr().lock();
    let _ = write!(err, "\r\x1b[K");
    let _ = err.flush();
}
