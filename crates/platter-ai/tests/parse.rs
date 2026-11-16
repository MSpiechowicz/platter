//! Reading the model's answer: the verdict is clamped, the reason is made safe to show, and anything
//! that is not exactly the expected JSON is an error (the folder is skipped, never defaulted).

use platter_ai::{ParseError, parse};
use platter_core::ModelVerdict;

fn answer(verdict: &str, reason: &str) -> String {
    serde_json::json!({ "verdict": verdict, "reason": reason }).to_string()
}

#[test]
fn only_keep_is_keep_and_everything_else_is_review() {
    let verdict = |word: &str| parse(&answer(word, "ok")).unwrap().verdict;

    assert_eq!(verdict("keep"), ModelVerdict::Keep);
    assert_eq!(verdict("review"), ModelVerdict::Review);
    assert_eq!(
        verdict("safe"),
        ModelVerdict::Review,
        "the model can never say safe"
    );
    assert_eq!(verdict("delete"), ModelVerdict::Review);
    assert_eq!(verdict("Keep"), ModelVerdict::Review);
    assert_eq!(verdict(" keep"), ModelVerdict::Review);
    assert_eq!(verdict(""), ModelVerdict::Review);
}

#[test]
fn the_answer_may_be_wrapped_in_whitespace() {
    let judgement = parse("\n {\"verdict\":\"keep\",\"reason\":\"Photos.\"} \n").unwrap();

    assert_eq!(judgement.verdict, ModelVerdict::Keep);
    assert_eq!(judgement.reason, "Photos.");
}

#[test]
fn a_long_reason_is_cut_to_200_characters() {
    let long = "é".repeat(300);

    let judgement = parse(&answer("review", &long)).unwrap();

    assert_eq!(judgement.reason.chars().count(), 200);
    assert_eq!(
        parse(&answer("review", &"a".repeat(200))).unwrap().reason.len(),
        200
    );
}

#[test]
fn a_reason_is_trimmed_and_loses_control_bidi_and_separator_characters() {
    let reason =
        "  Cache\u{202E}\u{2066}\u{200F} of\nthe\u{2028}system\u{2029}\u{7}\u{0} rebuilds\u{200B}.  ";

    let judgement = parse(&answer("review", reason)).unwrap();

    assert_eq!(judgement.reason, "Cache ofthesystem rebuilds.");
}

#[test]
fn a_reason_is_cut_after_it_is_cleaned_and_does_not_end_in_a_space() {
    let reason = format!("{}\u{202E}{}", "a".repeat(199), " tail");

    let judgement = parse(&answer("review", &reason)).unwrap();

    assert_eq!(judgement.reason, "a".repeat(199));
}

#[test]
fn a_reason_with_nothing_left_is_an_error() {
    assert_eq!(parse(&answer("review", "")), Err(ParseError::EmptyReason));
    assert_eq!(
        parse(&answer("review", " \u{202E}\n")),
        Err(ParseError::EmptyReason)
    );
}

#[test]
fn anything_but_exactly_the_two_fields_is_an_error() {
    let bad = [
        "",
        "review",
        "not json at all",
        r#"{"verdict":"review","reason":"cut short"#,
        r#"{"verdict":"review","reason":"x"#,
        r#"{"verdict":"review""#,
        r#"{"verdict":"review"}"#,
        r#"{"reason":"x"}"#,
        r#"{"verdict":"review","reason":"x","extra":1}"#,
        r#"{"verdict":"review","reason":"x","verdict":"keep"}"#,
        r#"{"verdict":"safe","reason":"x","confidence":0.9}"#,
        r#"{"verdict":1,"reason":"x"}"#,
        r#"{"verdict":"review","reason":null}"#,
        r#"["review","x"]"#,
        r#"{"verdict":"review","reason":"x"} trailing"#,
        r#"{"verdict":"review","reason":"x"}{"verdict":"keep","reason":"y"}"#,
    ];

    for text in bad {
        assert!(
            matches!(parse(text), Err(ParseError::Malformed(_))),
            "{text:?} was accepted: {:?}",
            parse(text)
        );
    }
}
