use std::io::{self, Write};
use std::time::Instant;

use platter_core::dto::{SuggestionsDto, Verdict};
use platter_platform::Platform;
use platter_platform::root::resolve_root;
use platter_rules::{RuleSet, evaluate};

use super::{CliResult, print_line, scan_path};
use crate::args::SuggestArgs;
use crate::format;

pub fn run(args: &SuggestArgs) -> CliResult<()> {
    // Home rules compare paths, so the root has to be the canonical one a scan of the same folder
    // started from the app would have. A path that cannot be resolved is left for the scan to report.
    let root = resolve_root(&args.path).map_or_else(|_| args.path.clone(), |resolved| resolved.path);

    let tree = scan_path(&root, args.one_file_system, args.max_entries)?;

    let started = Instant::now();
    let suggestions = evaluate(
        &tree,
        &Platform::current().environment_for(tree.root_path()),
        &RuleSet::builtin(),
    );
    let evaluated = started.elapsed();

    if args.stats {
        let _ = writeln!(
            io::stderr(),
            "platter-cli: {} entries scanned in {:.3} s, rules evaluated in {:.1} ms",
            tree.len(),
            tree.elapsed().as_secs_f64(),
            evaluated.as_secs_f64() * 1000.0
        );
    }

    if args.json {
        print_line(&format::to_json(&suggestions)?)?;
    } else {
        print_line(&render_text(&suggestions))?;
    }
    Ok(())
}

fn render_text(suggestions: &SuggestionsDto) -> String {
    if suggestions.suggestions.is_empty() {
        return "Nothing found that the rules know how to label.".to_owned();
    }

    let rows: Vec<Vec<String>> = suggestions
        .suggestions
        .iter()
        .map(|suggestion| {
            vec![
                verdict_name(suggestion.verdict).to_owned(),
                format::display(&suggestion.title),
                format::display(&suggestion.path),
                suggestion.items.to_string(),
                format::bytes(suggestion.allocated),
            ]
        })
        .collect();

    let totals = suggestions.totals;
    let mut lines = vec![format::table(
        &["VERDICT", "TITLE", "PATH", "ITEMS", "SIZE"],
        &rows,
    )];
    lines.push(String::new());
    for (verdict, bytes) in [
        (Verdict::Safe, totals.safe),
        (Verdict::Review, totals.review),
        (Verdict::Keep, totals.keep),
    ] {
        lines.push(format!("{:<7}  {}", verdict_name(verdict), format::bytes(bytes)));
    }

    lines.join("\n")
}

fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Safe => "safe",
        Verdict::Review => "review",
        Verdict::Keep => "keep",
    }
}
