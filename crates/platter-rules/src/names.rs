//! Comparing names the way Windows does.
//!
//! NTFS and ReFS compare names by mapping every UTF-16 unit through a fixed upcase table (one unit in, one
//! unit out) and comparing the results. [`char::to_uppercase`] is the nearest thing here: a letter whose
//! uppercase is a single character is compared by it, and one whose uppercase is several (`ß`) stands for
//! itself, as in a one-to-one table. The Unicode data the standard library has is newer than the table
//! of a given volume, which can only make two names equal that the volume tells apart: that errs toward
//! over-protecting, which is what a protected place and a kept folder want.

/// The upcase of one character: its uppercase when that is one character, else itself.
fn upcase(c: char) -> char {
    let mut upper = c.to_uppercase();

    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// Whether two names (UTF-8, as bytes) are the same name on Windows. Text that is not valid UTF-8 is
/// compared with ASCII case folded only.
pub fn same_windows_name(a: &[u8], b: &[u8]) -> bool {
    let (Ok(a_text), Ok(b_text)) = (std::str::from_utf8(a), std::str::from_utf8(b)) else {
        return a.eq_ignore_ascii_case(b);
    };

    a_text.chars().map(upcase).eq(b_text.chars().map(upcase))
}

/// `name` as [`same_windows_name`] compares it: equal names fold to equal bytes, so sorting folded names
/// puts a folder before what is inside it whatever the spelling.
pub fn fold_windows_name(name: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(name) else {
        return name.to_ascii_lowercase();
    };

    text.chars().map(upcase).collect::<String>().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_differ_only_in_case_are_the_same_name() {
        assert!(same_windows_name(b"Program Files", b"PROGRAM files"));
        assert!(same_windows_name("Żółć".as_bytes(), "żÓŁĆ".as_bytes()));
        assert!(same_windows_name("ΣΑΣ".as_bytes(), "σας".as_bytes()));
        assert!(!same_windows_name(b"Program Files", b"Program File"));
        assert!(!same_windows_name("zażółć".as_bytes(), "zazolc".as_bytes()));
    }

    #[test]
    fn letters_whose_uppercase_is_ascii_match_that_ascii() {
        // The long s (`\u{17f}`) and the dotless i (`\u{131}`) uppercase to `S` and `I`: a protected
        // `System32` is also reached as `Sy\u{17f}tem32`.
        assert!(same_windows_name("Sy\u{17f}tem32".as_bytes(), b"System32"));
        assert!(same_windows_name("W\u{131}ndows".as_bytes(), b"WINDOWS"));
    }

    #[test]
    fn a_letter_with_a_several_character_uppercase_is_only_itself() {
        assert!(same_windows_name("straße".as_bytes(), "straße".as_bytes()));
        assert!(!same_windows_name("straße".as_bytes(), b"STRASSE"));
    }

    #[test]
    fn text_that_is_not_utf8_folds_ascii_only() {
        assert!(same_windows_name(b"A\xffb", b"a\xffB"));
        assert!(!same_windows_name(b"A\xff", b"a\xfe"));
    }

    #[test]
    fn folding_is_consistent_with_comparing() {
        for (a, b) in [("Users", "USERS"), ("Żółć", "żółć"), ("a", "b")] {
            assert_eq!(
                fold_windows_name(a.as_bytes()) == fold_windows_name(b.as_bytes()),
                same_windows_name(a.as_bytes(), b.as_bytes())
            );
        }
    }
}
