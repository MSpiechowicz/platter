//! Human-readable formatting helpers.

const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

/// `1536` becomes `1.5 KiB`.
pub fn bytes(value: u64) -> String {
    let mut scaled = value as f64;
    let mut unit = 0;

    while scaled >= 1024.0 && unit + 1 < UNITS.len() {
        scaled /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{scaled:.1} {}", UNITS[unit])
    }
}

/// Makes untrusted text (file names, paths, volume labels) safe to print on a terminal: control
/// characters (C0, DEL, C1) and Unicode bidirectional controls are replaced by a visible `\u{..}`
/// escape. Otherwise a file named `\x1b[2J` could clear the screen, an OSC sequence could rewrite the
/// window title or clipboard, and a bidi override could make a name read differently than it is. A
/// literal backslash is written `\\`, so a name that merely contains the text `\u{1b}` cannot be
/// mistaken for one that contains the escape character: the output reads back unambiguously.
/// Used for every human-readable line; `--json` goes through [`json_safe`].
pub fn display(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());

    for ch in text.chars() {
        if ch == '\\' {
            safe.push_str("\\\\");
        } else if ch.is_control() || is_bidi_control(ch) {
            safe.push_str(&format!("\\u{{{:x}}}", u32::from(ch)));
        } else {
            safe.push(ch);
        }
    }
    safe
}

/// Serializes `value` as pretty JSON that is safe to print on a terminal. `serde_json` escapes only the
/// characters below U+0020, so DEL, the C1 controls (U+0080 to U+009F, where U+009B is a one-character
/// escape-sequence introducer on some terminals) and the bidi controls would reach the terminal raw.
/// They are written as `\uXXXX` escapes instead: the document stays valid and parses back to the exact
/// original text.
pub fn to_json<T: serde::Serialize>(value: &T) -> serde_json::Result<String> {
    Ok(json_safe(&serde_json::to_string_pretty(value)?))
}

/// Rewrites every character of `json` that is unsafe on a terminal and not escaped by the serializer as
/// a `\uXXXX` escape. Such characters can only occur inside JSON strings (everything outside them is
/// ASCII), where `\uXXXX` is an ordinary escape.
pub fn json_safe(json: &str) -> String {
    let mut safe = String::with_capacity(json.len());

    for ch in json.chars() {
        if ch == '\u{7f}' || ('\u{80}'..='\u{9f}').contains(&ch) || is_bidi_control(ch) {
            safe.push_str(&format!("\\u{:04x}", u32::from(ch)));
        } else {
            safe.push(ch);
        }
    }
    safe
}

fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// Percentage of `part` in `whole`, `0.0` when `whole` is zero.
pub fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

/// Left-aligned text table with a header row.
pub fn table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }

    let render = |cells: &[String]| -> String {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        padded.join("  ").trim_end().to_owned()
    };

    let header_cells: Vec<String> = header.iter().map(|h| (*h).to_owned()).collect();
    let mut lines = vec![render(&header_cells)];
    lines.extend(rows.iter().map(|row| render(row)));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_in_binary_units() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn display_escapes_terminal_and_bidi_control_characters() {
        let hostile = "a\x1b[2Jb\x1b]0;pwned\x07c\u{202e}gpj.exe\u{2066}\n\u{7f}\u{9b}d";

        let safe = display(hostile);

        assert_eq!(
            safe,
            "a\\u{1b}[2Jb\\u{1b}]0;pwned\\u{7}c\\u{202e}gpj.exe\\u{2066}\\u{a}\\u{7f}\\u{9b}d"
        );
        assert!(!safe.chars().any(|ch| ch.is_control() || is_bidi_control(ch)));
        assert_eq!(display("plain ünïcode 日本語.txt"), "plain ünïcode 日本語.txt");
    }

    #[test]
    fn display_writes_a_literal_backslash_twice_so_the_output_is_unambiguous() {
        assert_eq!(display("C:\\dir\\file"), "C:\\\\dir\\\\file");

        // The text `\u{1b}` and the escape character used to print the same.
        let literal = display("\\u{1b}");
        let escape = display("\u{1b}");
        assert_eq!(literal, "\\\\u{1b}");
        assert_eq!(escape, "\\u{1b}");
        assert_ne!(literal, escape);

        assert_eq!(
            display("\\\x1b"),
            "\\\\\\u{1b}",
            "a backslash before an escape character"
        );
    }

    #[test]
    fn json_output_escapes_del_c1_and_bidi_controls_and_reads_back_exactly() {
        let name = "a\u{9b}b\u{9d}c\u{7f}d\u{202e}e\u{85}f\u{2066}g\u{1b}h\"q\\ünï日本";

        let json = to_json(&serde_json::json!({ "name": name })).unwrap();

        for ch in [
            '\u{9b}', '\u{9d}', '\u{7f}', '\u{202e}', '\u{85}', '\u{2066}', '\u{1b}',
        ] {
            assert!(
                !json.contains(ch),
                "U+{:04X} is written as an escape",
                u32::from(ch)
            );
        }
        assert!(json.contains("\\u009b") && json.contains("\\u009d") && json.contains("\\u007f"));
        assert!(json.contains("\\u202e"));
        assert!(json.contains("ünï日本"), "ordinary non-ASCII text is left alone");

        let back: serde_json::Value = serde_json::from_str(&json).expect("still valid JSON");
        assert_eq!(back["name"], name, "the name round-trips exactly");
    }

    #[test]
    fn percent_handles_empty_totals() {
        assert_eq!(percent(1, 0), 0.0);
        assert_eq!(percent(1, 4), 25.0);
    }

    #[test]
    fn table_aligns_columns() {
        let text = table(&["A", "BB"], &[vec!["xxx".into(), "y".into()]]);
        assert_eq!(text, "A    BB\nxxx  y");
    }
}
