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
/// window title or clipboard, and a bidi override could make a name read differently than it is.
/// On Unix a literal backslash is written `\\`, so a name that merely contains the text `\u{1b}` cannot
/// be mistaken for one that contains the escape character: the output reads back unambiguously. On
/// Windows the backslash is the path separator and cannot be part of a name, so it is printed as it is
/// and `C:\Users\me` stays readable and copyable.
/// Used for every human-readable line; `--json` goes through [`json_safe`].
pub fn display(text: &str) -> String {
    escape(text, cfg!(not(windows)))
}

/// [`display`] with the backslash handling chosen by the caller.
fn escape(text: &str, double_backslash: bool) -> String {
    let mut safe = String::with_capacity(text.len());

    for ch in text.chars() {
        if ch == '\\' && double_backslash {
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

/// The share of `part` in `whole` as a fraction, `0.0` when `whole` is zero.
pub fn fraction(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// A share as the app writes it (`percentText` in `apps/desktop/src/lib/format.ts`, without its fine mode).
/// It never reads `100%` unless the share is exactly 1 or more, so `0.996` reads `99%`, and never `0%` for
/// a share that is not nothing: it reads `<1%`. Nothing, or a value that is not a number, reads `0%`.
pub fn percent_text(fraction: f64) -> String {
    if fraction.is_nan() || fraction <= 0.0 {
        return "0%".to_owned();
    }

    if fraction >= 1.0 {
        return format!("{}%", (fraction * 100.0).round());
    }

    let percent = fraction * 100.0;
    if percent < 1.0 {
        return "<1%".to_owned();
    }

    format!("{}%", percent.round().min(99.0))
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
    fn a_literal_backslash_is_written_twice_so_the_output_is_unambiguous() {
        assert_eq!(escape("C:\\dir\\file", true), "C:\\\\dir\\\\file");

        // The text `\u{1b}` and the escape character used to print the same.
        let literal = escape("\\u{1b}", true);
        let escaped = escape("\u{1b}", true);
        assert_eq!(literal, "\\\\u{1b}");
        assert_eq!(escaped, "\\u{1b}");
        assert_ne!(literal, escaped);

        assert_eq!(
            escape("\\\x1b", true),
            "\\\\\\u{1b}",
            "a backslash before an escape character"
        );
    }

    #[test]
    fn a_windows_path_keeps_its_single_backslashes_while_controls_stay_escaped() {
        assert_eq!(
            escape("C:\\Users\\me\\a\x1b[2Jb", false),
            "C:\\Users\\me\\a\\u{1b}[2Jb"
        );
    }

    #[test]
    fn display_follows_the_host_for_the_backslash() {
        let expected = if cfg!(windows) { "C:\\dir" } else { "C:\\\\dir" };

        assert_eq!(display("C:\\dir"), expected);
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
    fn a_share_of_nothing_reads_zero() {
        assert_eq!(percent_text(0.0), "0%");
        assert_eq!(percent_text(-0.5), "0%");
        assert_eq!(percent_text(f64::NAN), "0%");
        assert_eq!(percent_text(fraction(5, 0)), "0%");
    }

    #[test]
    fn a_share_under_the_smallest_step_reads_less_than_one() {
        assert_eq!(percent_text(fraction(1, 10_000)), "<1%");
        assert_eq!(percent_text(0.0099), "<1%");
    }

    #[test]
    fn a_share_reads_its_rounded_value() {
        assert_eq!(percent_text(0.01), "1%");
        assert_eq!(percent_text(fraction(1, 4)), "25%");
        assert_eq!(percent_text(0.495), "50%");
        assert_eq!(percent_text(0.984), "98%");
    }

    #[test]
    fn a_share_just_under_the_whole_never_reads_a_hundred() {
        assert_eq!(percent_text(fraction(9996, 10_000)), "99%");
        assert_eq!(percent_text(0.9951), "99%");
        assert_eq!(percent_text(fraction(1, 1)), "100%");
        assert_eq!(percent_text(1.004), "100%");
    }

    #[test]
    fn table_aligns_columns() {
        let text = table(&["A", "BB"], &[vec!["xxx".into(), "y".into()]]);
        assert_eq!(text, "A    BB\nxxx  y");
    }
}
