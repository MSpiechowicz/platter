//! The safety boundary of this crate: it labels, it never touches anything. The crate's sources must not
//! use the filesystem, processes, the environment or the clock, and its only dependency must be
//! `platter-core`. A future cleanup feature belongs in another crate; this test fails if code that acts
//! creeps in here. The workspace root manifest sets `unsafe_code = "forbid"` and edition 2024 for every
//! crate of the workspace, and this test checks that it still does.
//!
//! Three layers hold the line. `lib.rs` forbids (not denies) clippy's `disallowed_*` lints, so no
//! `#[allow]` lifts them. `clippy.toml` lists what they refuse. This test refuses the ways around both:
//! a build script, another manifest section, a source pulled in by path, a rewritten lint level.

// This test is the one place that reads files: it reads the crate's own sources to inspect them.
#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::{Path, PathBuf};

/// Paths and calls that read or change the outside world, and the ways to hide them or to switch the lints off.
const FORBIDDEN: &[&str] = &[
    "std::fs",
    "std::process",
    "std::env",
    "std::net",
    "std::os",
    "std::time::SystemTime",
    "std::time::Instant",
    "SystemTime",
    "Instant::now",
    "use std::fs",
    "fs::",
    "remove_file",
    "remove_dir",
    "rename(",
    "Command::new",
    "unsafe",
    // Code that runs without `unsafe` in the source, or lands in a place the compiler would not look.
    "global_asm",
    "asm!",
    "naked",
    "link_section",
    "export_name",
    "no_mangle",
    "#[used",
    "libc",
    "rustix",
    "tokio",
    // Another file compiled in, or a module moved out of the scanned tree.
    "#[path",
    "include!",
    "include_str!",
    "include_bytes!",
    "cfg_attr",
    // A path or name built or disguised in a way the list above cannot see: a raw identifier (`std::r#fs`)
    // and a macro that pastes identifiers together.
    "r#",
    "macro_rules!",
    // Reaching `std` through a group, a glob, an alias or a leading `::`.
    "extern crate",
    "std::{",
    "std::*",
    "::std::",
    "std as",
    "std;",
    // Switching the lints off.
    "allow(clippy",
    "expect(clippy",
];

/// What `clippy.toml` must keep refusing.
const REQUIRED_DISALLOWED: &[&str] = &[
    "std::fs::File",
    "std::fs::OpenOptions",
    "std::fs::DirBuilder",
    "std::process::Command",
    "std::fs::read_dir",
    "std::fs::remove_file",
    "std::fs::write",
    "std::fs::soft_link",
    "std::os::unix::fs::symlink",
    "std::os::windows::fs::symlink_file",
    "std::os::windows::fs::symlink_dir",
    "std::env::var",
    "std::time::SystemTime::now",
    "std::path::Path::exists",
    "std::path::Path::try_exists",
    "std::path::Path::metadata",
    "std::path::Path::symlink_metadata",
    "std::path::Path::read_dir",
    "std::path::Path::canonicalize",
    "std::path::Path::read_link",
    "std::path::Path::is_dir",
    "std::path::Path::is_file",
    "std::path::Path::is_symlink",
    "std::io::stdin",
    "std::net::TcpStream::connect",
    "std::net::UdpSocket::bind",
    "std::net::TcpListener::bind",
    "std::os::unix::fs::chown",
    "std::os::unix::fs::lchown",
    "std::os::unix::fs::fchown",
    "std::os::unix::fs::chroot",
    "std::env::current_dir",
    "std::env::set_current_dir",
    "std::process::exit",
    "std::process::abort",
];

/// Cargo's automatic targets that would compile code outside `src/` without a line in the manifest.
const FORBIDDEN_ENTRIES: &[&str] = &["build.rs", "benches", "examples"];

/// The only tables the manifest may have.
const ALLOWED_TABLES: &[&str] = &["package", "lints", "dependencies", "dev-dependencies"];

/// Keys of `[package]` that add targets, a build script or other tables.
const FORBIDDEN_PACKAGE_KEYS: &[&str] = &[
    "build",
    "lib",
    "bin",
    "test",
    "bench",
    "example",
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "target",
    "lints",
    "features",
    "patch",
    "replace",
    "workspace",
    "profile",
];

fn squashed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `text` as the scan reads it: without its comments, and the lines where a raw string starts.
struct Stripped {
    code: String,
    /// The 1-based lines of raw string literals (`r"`, `r#"`, `br"`, `cr"`): this crate has none, because
    /// a raw string has no escapes, which is what lets a hostile one desynchronise a scan like this.
    raw_string_lines: Vec<usize>,
}

/// `text` without its comments (`//` to the end of the line, and `/* */`, which nest). A comment becomes one
/// space, so that what it separated stays separate, and its line breaks stay, so that line numbers hold.
/// What lies inside a string or character literal is code, not a comment: `"//"` hides nothing after it.
fn without_comments(text: &str) -> String {
    strip(text).code
}

/// [`without_comments`], and where raw strings start. An `r` that opens a raw string counts after an
/// identifier character too (`br"`, `cr"`): in code that is only ever a string prefix, and treating it as
/// one keeps the scan in step with the compiler. `b"` and `c"` need nothing of the kind: their prefix is
/// copied like any code and the quote opens an ordinary string.
fn strip(text: &str) -> Stripped {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut raw_string_lines = Vec::new();
    let mut line = 1;
    let mut at = 0;

    while at < chars.len() {
        let c = chars[at];
        let next = chars.get(at + 1).copied();
        let start = at;

        if c == '/' && next == Some('/') {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
            out.push(' ');
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while at < chars.len() {
                if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
                    depth += 1;
                    at += 2;
                } else if chars[at] == '*' && chars.get(at + 1) == Some(&'/') {
                    depth -= 1;
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[at] == '\n' {
                        out.push('\n');
                    }
                    at += 1;
                }
            }
            out.push(' ');
        } else if c == '"' {
            at = copy_literal(&chars, at, '"', &mut out);
        } else if c == 'r'
            && let Some(hashes) = raw_string_hashes(&chars, at)
        {
            raw_string_lines.push(line);
            at = copy_raw_string(&chars, at, hashes, &mut out);
        } else if c == '\'' && is_char_literal(&chars, at) {
            at = copy_literal(&chars, at, '\'', &mut out);
        } else {
            out.push(c);
            at += 1;
        }

        line += chars[start..at.min(chars.len())]
            .iter()
            .filter(|c| **c == '\n')
            .count();
    }

    Stripped {
        code: out,
        raw_string_lines,
    }
}

/// How many `#` follow the `r` at `at` when a raw string starts there (`r"`, `r#"`), else `None`.
fn raw_string_hashes(chars: &[char], at: usize) -> Option<usize> {
    let hashes = chars[at + 1..].iter().take_while(|c| **c == '#').count();

    (chars.get(at + 1 + hashes) == Some(&'"')).then_some(hashes)
}

/// Copies the raw string that starts at `at`, up to its closing quote and hashes. Returns where it ends.
fn copy_raw_string(chars: &[char], at: usize, hashes: usize, out: &mut String) -> usize {
    let mut end = at + hashes + 2;

    while end < chars.len() {
        let closes = chars[end] == '"'
            && chars[end + 1..]
                .iter()
                .take(hashes)
                .filter(|c| **c == '#')
                .count()
                == hashes;
        end += 1;
        if closes {
            end += hashes;
            break;
        }
    }

    let end = end.min(chars.len());
    out.extend(&chars[at..end]);

    end
}

/// Whether the `'` at `at` opens a character literal, as opposed to a lifetime or a label.
fn is_char_literal(chars: &[char], at: usize) -> bool {
    match chars.get(at + 1) {
        Some('\\') => true,
        Some(c) if *c != '\'' => chars.get(at + 2) == Some(&'\''),
        _ => false,
    }
}

/// Copies the literal that opens with `quote` at `at`, honouring backslash escapes. Returns where it ends.
fn copy_literal(chars: &[char], at: usize, quote: char, out: &mut String) -> usize {
    let mut end = at + 1;

    while end < chars.len() {
        if chars[end] == '\\' {
            end += 2;
        } else if chars[end] == quote {
            end += 1;
            break;
        } else {
            end += 1;
        }
    }

    let end = end.min(chars.len());
    out.extend(&chars[at..end]);

    end
}

/// What `text`, the source of `name`, uses from `FORBIDDEN`, comments aside. Whitespace between tokens (or
/// a comment) does not hide it.
fn source_violations(name: &str, text: &str) -> Vec<String> {
    let stripped = strip(text);
    let text = &stripped.code;
    let mut violations: Vec<String> = stripped
        .raw_string_lines
        .iter()
        .map(|line| format!("{name}:{line}: a raw string"))
        .collect();
    let mut found_on_a_line = Vec::new();

    for (number, line) in text.lines().enumerate() {
        for word in FORBIDDEN {
            if line.contains(word) {
                found_on_a_line.push(*word);
                violations.push(format!("{name}:{}: {word}: {}", number + 1, line.trim()));
            }
        }
    }

    let text = squashed(text);
    for word in FORBIDDEN {
        if !found_on_a_line.contains(word) && text.contains(&squashed(word)) {
            violations.push(format!("{name}: {word}: split by whitespace"));
        }
    }

    violations
}

fn strip_comment(line: &str) -> &str {
    let mut quoted = false;

    for (index, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..index],
            _ => {}
        }
    }

    line
}

/// What in `manifest` lets this crate use more than `platter-core`, run code at build time, add a
/// target by path or change its lint levels.
fn manifest_problems(manifest: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut table: Option<String> = None;

    for raw in manifest.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("[[") {
            problems.push(format!("array of tables: {line}"));
            table = Some(String::new());
            continue;
        }

        if line.starts_with('[') {
            let name: String = line
                .trim_matches(|c: char| c == '[' || c == ']' || c.is_whitespace())
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
                .collect();

            if !ALLOWED_TABLES.contains(&name.as_str()) {
                problems.push(format!("table not allowed: {line}"));
            }
            table = Some(name);
            continue;
        }

        let key = line.split('=').next().unwrap_or("").trim().trim_matches('"');
        let first_key = key.split('.').next().unwrap_or("").trim().trim_matches('"');

        match table.as_deref() {
            None => problems.push(format!("key before the first table: {line}")),
            Some("package") if FORBIDDEN_PACKAGE_KEYS.contains(&first_key) => {
                problems.push(format!("[package] key not allowed: {line}"));
            }
            Some("lints") if squashed(line) != "workspace=true" => {
                problems.push(format!("[lints] must only be `workspace = true`: {line}"));
            }
            Some("dependencies") if squashed(line) != "platter-core.workspace=true" => {
                problems.push(format!("the only dependency is platter-core: {line}"));
            }
            _ => {}
        }
    }

    problems
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();

        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_sources_use_no_filesystem_process_environment_or_clock_api() {
    let mut files = Vec::new();
    sources(&manifest_dir().join("src"), &mut files);
    assert!(files.len() >= 8, "the sources were found: {files:?}");

    let mut violations = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        violations.extend(source_violations(&file.display().to_string(), &text));
    }

    assert!(
        violations.is_empty(),
        "read-only crate touches the outside world:\n{}",
        violations.join("\n")
    );
}

/// The workspace root manifest: its `[workspace.lints.rust]` is what keeps `unsafe` out of this crate.
fn workspace_manifest() -> String {
    fs::read_to_string(manifest_dir().join("../../Cargo.toml")).unwrap()
}

/// The keys of the table `[name]` of `manifest`, comments removed and whitespace squashed.
fn table_lines(manifest: &str, name: &str) -> Vec<String> {
    let mut in_table = false;
    let mut lines = Vec::new();

    for raw in manifest.lines() {
        let line = strip_comment(raw).trim();

        if line.starts_with('[') {
            in_table = squashed(line) == format!("[{name}]");
        } else if in_table && !line.is_empty() {
            lines.push(squashed(line));
        }
    }

    lines
}

#[test]
fn the_workspace_forbids_unsafe_code_and_uses_edition_2024() {
    let manifest = workspace_manifest();

    assert!(
        table_lines(&manifest, "workspace.lints.rust").contains(&"unsafe_code=\"forbid\"".to_string()),
        "the root Cargo.toml must keep `unsafe_code = \"forbid\"` under [workspace.lints.rust]"
    );
    assert!(
        table_lines(&manifest, "workspace.package").contains(&"edition=\"2024\"".to_string()),
        "the root Cargo.toml must keep `edition = \"2024\"` under [workspace.package]"
    );
}

#[test]
fn the_workspace_manifest_check_reads_only_the_named_table() {
    let manifest = "[workspace.package]\nedition = \"2024\"\n[other]\nunsafe_code = \"forbid\"\n\
        [workspace.lints.rust]\nunsafe_code = \"warn\" # forbid\n";

    assert_eq!(
        table_lines(manifest, "workspace.lints.rust"),
        ["unsafe_code=\"warn\""]
    );
    assert_eq!(table_lines(manifest, "workspace.package"), ["edition=\"2024\""]);
}

#[test]
fn lib_forbids_the_disallowed_lints_so_no_allow_can_lift_them() {
    let lib = squashed(&fs::read_to_string(manifest_dir().join("src/lib.rs")).unwrap());

    assert!(
        lib.contains("#![forbid(clippy::disallowed_methods,clippy::disallowed_types)]"),
        "src/lib.rs must start with #![forbid(clippy::disallowed_methods, clippy::disallowed_types)]"
    );
}

#[test]
fn clippy_toml_still_lists_what_the_crate_must_not_call() {
    let config = fs::read_to_string(manifest_dir().join("clippy.toml")).unwrap();

    let missing: Vec<&&str> = REQUIRED_DISALLOWED
        .iter()
        .filter(|path| !config.contains(&format!("\"{path}\"")))
        .collect();

    assert!(missing.is_empty(), "clippy.toml no longer refuses: {missing:?}");
}

#[test]
fn there_is_no_build_script_and_no_target_outside_src() {
    for name in FORBIDDEN_ENTRIES {
        assert!(
            !manifest_dir().join(name).exists(),
            "{name} would run or compile code outside src/"
        );
    }
}

#[test]
fn the_manifest_has_only_platter_core_and_no_build_script_or_extra_target() {
    let manifest = fs::read_to_string(manifest_dir().join("Cargo.toml")).unwrap();

    assert_eq!(manifest_problems(&manifest), Vec::<String>::new());
}

const GOOD_MANIFEST: &str = r#"
[package]
name = "platter-rules"
version.workspace = true
description = "Read-only # not a comment"

[lints]
workspace = true

[dependencies]
platter-core.workspace = true # the one

[dev-dependencies]
platter-core = { workspace = true, features = ["testing"] }
serde_json.workspace = true
"#;

#[test]
fn the_manifest_check_accepts_the_expected_shape() {
    assert_eq!(manifest_problems(GOOD_MANIFEST), Vec::<String>::new());
}

#[test]
fn the_manifest_check_refuses_every_way_to_add_code_or_dependencies() {
    let bypasses = [
        ("a build-dependencies table", "[build-dependencies]\ncc = \"1\""),
        (
            "a target table",
            "[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"",
        ),
        (
            "a target table in a quoted form",
            "[ \"target\" . 'x86_64'.dependencies ]\nlibc = \"0.2\"",
        ),
        (
            "a dependency as its own table",
            "[dependencies.libc]\nversion = \"0.2\"",
        ),
        ("a lib path", "[lib]\npath = \"../elsewhere/lib.rs\""),
        ("a bin target", "[[bin]]\nname = \"x\"\npath = \"x.rs\""),
        ("a build script", "[package]\nbuild = \"build.rs\""),
        ("a build script with a dotted key", "[package]\nbuild.x = 1"),
        (
            "a package key that adds a table",
            "[package]\nlib.path = \"x.rs\"",
        ),
        (
            "a second dependency",
            "[dependencies]\nplatter-core.workspace = true\nserde.workspace = true",
        ),
        (
            "a renamed dependency",
            "[dependencies]\nplatter-core = { workspace = true, package = \"libc\" }",
        ),
        (
            "a lint level",
            "[lints]\nworkspace = true\nclippy.disallowed_methods = \"allow\"",
        ),
        ("a lints table", "[lints.clippy]\ndisallowed_methods = \"allow\""),
        ("a features table", "[features]\ndefault = []"),
        ("a metadata table", "[package.metadata.x]\na = 1"),
    ];

    for (what, extra) in bypasses {
        let manifest = format!("{GOOD_MANIFEST}\n{extra}\n");
        let problems = manifest_problems(&manifest);

        assert!(!problems.is_empty(), "{what} was accepted:\n{extra}");
    }
}

#[test]
fn the_manifest_check_refuses_a_key_before_the_first_table() {
    let manifest = format!("package.build = \"build.rs\"\n{GOOD_MANIFEST}");

    assert!(!manifest_problems(&manifest).is_empty());
}

#[test]
fn the_manifest_check_names_the_offending_line() {
    let problems = manifest_problems("[package]\nbuild = \"build.rs\"\n[build-dependencies]\n");

    assert_eq!(
        problems,
        [
            "[package] key not allowed: build = \"build.rs\"",
            "table not allowed: [build-dependencies]"
        ]
    );
}

#[test]
fn the_source_check_accepts_plain_code() {
    let source = "use std::collections::HashMap;\n// A comment about files.\nfn f(path: &str) -> usize { path.len() }\n";

    assert_eq!(source_violations("ok.rs", source), Vec::<String>::new());
}

#[test]
fn the_source_check_refuses_ways_to_pull_in_code_or_switch_the_lints_off() {
    let bypasses = [
        ("a module by path", "#[path = \"../../other/x.rs\"]\nmod x;"),
        ("a module by path with spaces", "# [ path = \"x.rs\" ]\nmod x;"),
        ("an include", "include!(\"../x.rs\");"),
        ("an include of text", "const A: &str = include_str!(\"x\");"),
        ("an include of bytes", "const A: &[u8] = include_bytes!(\"x\");"),
        ("a conditional path", "#[cfg_attr(unix, path = \"x.rs\")]\nmod x;"),
        ("an extern crate", "extern crate libc as l;"),
        ("a std group", "use std::{fs, io};"),
        ("a std group over lines", "use std::\n{fs};"),
        ("a leading ::std", "fn f() { ::std::mem::drop(1); }"),
        ("a std alias", "use std as s;"),
        ("a std glob", "use std::*;"),
        (
            "an allow of a lint",
            "#[allow(clippy::disallowed_methods)]\nfn f() {}",
        ),
        (
            "an allow with spaces",
            "#[allow( clippy::disallowed_methods )]\nfn f() {}",
        ),
        (
            "an allow split over lines",
            "#[allow(\n    clippy::disallowed_methods\n)]\nfn f() {}",
        ),
        (
            "an expect of a lint",
            "#[expect(clippy::disallowed_methods)]\nfn f() {}",
        ),
        ("a crate-level allow", "#![allow(clippy::disallowed_types)]"),
        ("a raw identifier in a path", "use std::r#fs as f;"),
        ("a raw identifier anywhere", "fn f() { let r#x = 1; }"),
        (
            "a comment between the parts of a path",
            "fn f() { std::/**/os::unix::fs/**/::symlink(\"a\", \"b\"); }",
        ),
        (
            "a macro that pastes an identifier into a path",
            "macro_rules! m { ($a:ident) => { std::$a::read(\"x\") }; }\nfn f() { m!(fs); }",
        ),
        (
            "code after a string that holds //",
            "fn f() { let a = \"//\"; std::fs::read(a); }",
        ),
        (
            "code after a string that holds /*",
            "fn f() { let a = \"/*\"; std::fs::read(a); }",
        ),
        (
            "code after a character that is a slash",
            "fn f() { let a = '/'; let b = '/'; std::fs::read(a); }",
        ),
        (
            "code after a raw string that holds //",
            "fn f() { let a = r\"//\"; std::fs::read(a); }",
        ),
        (
            "code after a byte raw string that holds a backslash",
            "fn f() { let _a = br\"\\\"; let _b = \"//\"; std::fs::read(_a); }",
        ),
        (
            "code after a C raw string that holds a backslash",
            "fn f() { let _a = cr\"\\\"; let _b = \"//\"; std::fs::read(_a); }",
        ),
        (
            "code after a byte string that holds a quote",
            "fn f() { let _a = b\"\\\"\"; let _b = \"//\"; std::fs::read(_a); }",
        ),
        (
            "code after a C string that holds a quote",
            "fn f() { let _a = c\"\\\"\"; let _b = \"//\"; std::fs::read(_a); }",
        ),
        ("a global assembly block", "core::arch::global_asm!(\"\");"),
        (
            "an inline assembly block",
            "fn f() { core::arch::asm!(\"nop\"); }",
        ),
        ("a naked function", "#[naked]\nfn f() {}"),
        ("a link section", "#[link_section = \".x\"]\nstatic A: u8 = 0;"),
        ("an exported name", "#[export_name = \"x\"]\nfn f() {}"),
        ("an unmangled name", "#[no_mangle]\nfn f() {}"),
        ("a used static", "#[used]\nstatic A: u8 = 0;"),
        ("a raw string", "const A: &str = r\"x\";"),
        ("a raw string with hashes", "const A: &str = r#\"x\"#;"),
        ("a raw byte string", "const A: &[u8] = br\"x\";"),
        ("a raw C string", "const A: &std::ffi::CStr = cr\"x\";"),
        (
            "code after a nested comment",
            "/* a /* b */ std::fs::read */ std::fs::read(\"x\");",
        ),
    ];

    for (what, source) in bypasses {
        assert!(
            !source_violations("bad.rs", source).is_empty(),
            "{what} was accepted:\n{source}"
        );
    }
}

#[test]
fn a_raw_string_is_named_and_does_not_hide_what_follows_it() {
    let source = "fn f() { let _a = br\"\\\"; let _b = \"//\"; std::fs::read(_a); }";

    let violations = source_violations("bad.rs", source);

    assert!(
        violations.iter().any(|v| v.contains("a raw string")),
        "{violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.contains("std::fs")),
        "the code after it was read: {violations:?}"
    );
}

#[test]
fn the_source_check_accepts_strings_that_end_in_an_r_and_byte_strings() {
    let source = "fn f() -> (&'static str, &'static [u8], char) { (\"user\", b\"color\", 'r') }\n";

    assert_eq!(source_violations("ok.rs", source), Vec::<String>::new());
}

#[test]
fn the_raw_string_lines_are_counted_from_one() {
    let stripped = strip("fn f() {\n    let a = 1; /* x\n y */\n    let b = r\"x\";\n}\n");

    assert_eq!(stripped.raw_string_lines, [4]);
}

#[test]
fn the_source_check_ignores_what_only_a_comment_says() {
    let source = "\
// std::fs::read and unsafe in a line comment
/// std::process::Command in a doc comment
/* std::env::var in a block comment /* and a nested one: rustix */ still comment: libc */
fn f<'a>(x: &'a str) -> &'a str { x } // trailing: include!(\"x\")
";

    assert_eq!(source_violations("ok.rs", source), Vec::<String>::new());
}

#[test]
fn the_comment_stripper_keeps_code_and_line_breaks() {
    assert_eq!(
        without_comments("a /* x\ny */ b // c\nd \"//\" 'e' '\\'' f<'a>"),
        "a \n  b  \nd \"//\" 'e' '\\'' f<'a>"
    );
}
