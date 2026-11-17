use platter_core::dto::{NodeDto, SortKey};
use platter_core::{Totals, Tree, query};
use serde::Serialize;

use super::{CliResult, print_line, scan_path};
use crate::args::ScanArgs;
use crate::format;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    entries: usize,
    elapsed_ms: u64,
    entries_per_sec: u64,
    memory_bytes: usize,
    bytes_per_entry: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    root: String,
    totals: Totals,
    children: Vec<NodeDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stats: Option<Stats>,
}

pub fn run(args: &ScanArgs) -> CliResult<()> {
    let tree = scan_path(&args.path, args.one_file_system, args.max_entries, &args.excluded)?;
    let children = query::children(&tree, tree.root(), 0, args.top, SortKey::SizeDesc)?;

    let report = Report {
        root: tree.root_path().display().to_string(),
        totals: tree.totals(),
        children: children.items,
        stats: args.stats.then(|| stats_of(&tree)),
    };

    if args.json {
        print_line(&format::to_json(&report)?)?;
    } else {
        print_line(&render_text(&report))?;
    }
    Ok(())
}

fn stats_of(tree: &Tree) -> Stats {
    let entries = tree.len();
    let elapsed = tree.elapsed().as_secs_f64();

    Stats {
        entries,
        elapsed_ms: tree.elapsed().as_millis() as u64,
        entries_per_sec: if elapsed > 0.0 {
            (entries as f64 / elapsed) as u64
        } else {
            0
        },
        memory_bytes: tree.memory_bytes(),
        bytes_per_entry: tree.memory_bytes() / entries.max(1),
    }
}

fn render_text(report: &Report) -> String {
    let totals = &report.totals;
    let mut lines = vec![format::display(&report.root)];

    lines.push(format!(
        "  allocated  {} ({} B)",
        format::bytes(totals.allocated),
        totals.allocated
    ));
    lines.push(format!(
        "  apparent   {} ({} B)",
        format::bytes(totals.apparent),
        totals.apparent
    ));
    lines.push(format!(
        "  files {}, folders {}, errors {}",
        totals.files, totals.dirs, totals.errors
    ));

    if !report.children.is_empty() {
        lines.push("\nLargest items".to_owned());
        let rows: Vec<Vec<String>> = report
            .children
            .iter()
            .map(|child| {
                vec![
                    format::bytes(child.allocated),
                    format!("{:.1}%", format::percent(child.allocated, totals.allocated)),
                    format!("{:?}", child.kind).to_lowercase(),
                    format::display(&child.name),
                ]
            })
            .collect();
        lines.push(format::table(&["SIZE", "SHARE", "KIND", "NAME"], &rows));
    }

    if let Some(stats) = &report.stats {
        lines.push("\nStats".to_owned());
        lines.push(format!("  entries         {}", stats.entries));
        lines.push(format!(
            "  elapsed         {:.3} s",
            stats.elapsed_ms as f64 / 1000.0
        ));
        lines.push(format!("  entries/sec     {}", stats.entries_per_sec));
        lines.push(format!(
            "  memory          {} (~{} B per entry)",
            format::bytes(stats.memory_bytes as u64),
            stats.bytes_per_entry
        ));
    }
    lines.join("\n")
}
