//! Renders a rule's reason template with the facts of the matches.

use platter_core::units::{Units, format_size};

/// The numbers a reason may mention, and the units a size is written in.
pub(crate) struct Facts {
    pub count: u64,
    pub days: Option<u32>,
    pub min_bytes: Option<u64>,
    pub units: Units,
}

/// Fills `{n:noun}`, `{n:one|many}`, `{days}` and `{size}`. Anything else in braces is kept as written.
pub(crate) fn render(template: &str, facts: &Facts) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;

    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];

        let Some(close) = after.find('}') else {
            out.push_str(after);
            return out;
        };

        match expand(&after[1..close], facts) {
            Some(text) => out.push_str(&text),
            None => out.push_str(&after[..=close]),
        }
        rest = &after[close + 1..];
    }

    out.push_str(rest);
    out
}

fn expand(token: &str, facts: &Facts) -> Option<String> {
    if token == "days" {
        return facts.days.map(|days| days.to_string());
    }
    if token == "size" {
        return facts.min_bytes.map(|bytes| format_size(bytes, facts.units));
    }

    let nouns = token.strip_prefix("n:")?;
    let (singular, plural) = match nouns.split_once('|') {
        Some((singular, plural)) => (singular.to_owned(), plural.to_owned()),
        None => (nouns.to_owned(), format!("{nouns}s")),
    };

    let noun = if facts.count == 1 { singular } else { plural };
    Some(format!("{} {noun}", facts.count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(count: u64) -> Facts {
        Facts {
            count,
            days: Some(30),
            min_bytes: Some(1 << 30),
            units: Units::Binary,
        }
    }

    #[test]
    fn counts_agree_with_their_noun() {
        let text = "{n:project} not touched for {days} days";

        assert_eq!(render(text, &facts(14)), "14 projects not touched for 30 days");
        assert_eq!(render(text, &facts(1)), "1 project not touched for 30 days");
        assert_eq!(render("{n:copy|copies}", &facts(2)), "2 copies");
        assert_eq!(render("{n:copy|copies}", &facts(1)), "1 copy");
    }

    #[test]
    fn sizes_are_written_in_the_units_asked_for() {
        let binary = facts(1);
        let decimal = Facts {
            units: Units::Decimal,
            ..facts(1)
        };

        assert_eq!(render("over {size}", &binary), "over 1.0 GiB");
        assert_eq!(render("over {size}", &decimal), "over 1.1 GB");
    }

    #[test]
    fn unknown_or_unavailable_tokens_are_left_as_written() {
        let none = Facts {
            count: 3,
            days: None,
            min_bytes: None,
            units: Units::Binary,
        };

        assert_eq!(
            render("{days} {size} {other} {", &none),
            "{days} {size} {other} {"
        );
    }
}
