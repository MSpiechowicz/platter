//! The grammar that forces the model's answer into the one shape the parser accepts.

/// GBNF, root `root`: `{"verdict":"review"|"keep","reason":"<1 to 180 plain characters>"}`. The reason
/// cannot contain a quote, a backslash or a control character, so the answer is always valid JSON.
pub const VERDICT_GRAMMAR: &str = r#"root    ::= "{\"verdict\":\"" verdict "\",\"reason\":\"" reason "\"}"
verdict ::= "review" | "keep"
reason  ::= rchar{1,180}
rchar   ::= [^"\\\x00-\x1F\x7F]
"#;

/// The name of the grammar's start rule.
pub const GRAMMAR_ROOT: &str = "root";

/// GBNF, root `root`: the question, read into `{"intent":…,"scope":"<0 to 120 plain characters>","kind":…,
/// "limit":1 to 10}`. The scope cannot contain a quote, a backslash, a control character or `<`, so the
/// answer is always valid JSON and never writes a chat marker.
pub const QUESTION_GRAMMAR: &str = r#"root   ::= "{\"intent\":\"" intent "\",\"scope\":\"" scope "\",\"kind\":\"" kind "\",\"limit\":" limit "}"
intent ::= "largest" | "cleanable" | "labelled" | "explain" | "find" | "general" | "offtopic"
scope  ::= schar{0,120}
schar  ::= [^"\\\x00-\x1F\x7F<]
kind   ::= "any" | "cache" | "build" | "media"
limit  ::= [1-9] | "10"
"#;
