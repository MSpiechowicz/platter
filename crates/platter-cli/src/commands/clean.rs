use platter_cleanup::{
    CleanupAction, CleanupOptions, CleanupPlan, CleanupReport, Disk, TargetOutcome, execute, plan_cleanup,
};
use platter_core::dto::Verdict;
use platter_core::{CancelToken, UserRule};
use platter_platform::Platform;
use platter_platform::environment::home_trash;

use super::{CliResult, print_line, scan_path, user_rules};
use crate::args::CleanArgs;
use crate::format;

pub fn run(args: &CleanArgs) -> CliResult<()> {
    if !args.dry_run && !args.yes {
        return Err("this deletes files: add --dry-run to see the plan, or --yes to clean".into());
    }
    if args.rules.is_empty() && !args.safe {
        return Err("choose what to clean: --safe, or --rule ID (see `platter-cli suggest`)".into());
    }

    // With rules that cannot be read nothing says which folders the user keeps: plan nothing.
    let user = usable_rules()?;
    let rules = platter_rules::RuleSet::builtin().with_user(user.clone());
    let mut selected: Vec<&str> = args.rules.iter().map(String::as_str).collect();
    if args.safe {
        selected.extend(
            rules
                .rules()
                .iter()
                .filter(|rule| rule.verdict == Verdict::Safe)
                .map(|rule| rule.id),
        );
    }
    selected.sort_unstable();
    selected.dedup();

    let tree = scan_path(&args.path, args.one_file_system, args.max_entries, &args.excluded)?;
    // A scan can take long: the rules are read again, and the plan is made from the ones that were read
    // first only when they are still what the file says.
    unchanged_rules(&user, user_rules())?;
    let platform = Platform::current();
    let env = platform.environment_for(tree.root_path());
    // No flags for the options yet: the defaults (keep 2 package versions, ask for admin) apply.
    let options = CleanupOptions::default();
    let plan = plan_cleanup(&tree, &env, &rules, &selected, &platform.entry_probe, &options);

    // Only rules asked for by name are worth a "found nothing"; `--safe` asks for all of them.
    let asked: Vec<String> = plan
        .not_found
        .iter()
        .filter(|id| args.rules.contains(id))
        .cloned()
        .collect();
    print_line(&render(&plan, &asked, args.verbose, args.dry_run))?;
    if args.dry_run {
        return Ok(());
    }
    if plan.groups().is_empty() {
        // Asked to clean, and nothing could be: that is not a success.
        if !plan.refused.is_empty() || !plan.left_out.is_empty() || !asked.is_empty() {
            return Err("nothing was cleaned (see above)".into());
        }
        return Ok(());
    }

    let trash = home_trash(env.home.as_deref());
    let disk = Disk {
        probe: &platform.entry_probe,
        remover: &platform.entry_remover,
        trash_dir: trash.as_deref(),
        tools: Some(&platform.tool_runner),
    };
    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});
    print_line(&render_report(&report))?;

    if report.results.iter().any(|result| !is_clean(&result.outcome)) {
        return Err("some items could not be cleaned (listed above)".into());
    }
    if !asked.is_empty() {
        return Err("some rules found nothing to clean (listed above)".into());
    }
    Ok(())
}

/// The rules of the file, or why nothing may be planned from it.
fn usable_rules() -> CliResult<Vec<UserRule>> {
    user_rules().map_err(|why| unusable(&why).into())
}

fn unusable(why: &str) -> String {
    format!(
        "your rules file cannot be used, so nothing is planned: {} Fix the file, or move it away \
         (the app's Advisor view does that with \"Start over\").",
        format::display(why)
    )
}

/// Refuses when the rules file broke, or says something else, since `before` was read.
fn unchanged_rules(before: &[UserRule], now: Result<Vec<UserRule>, String>) -> CliResult<()> {
    match now {
        Err(why) => Err(unusable(&why).into()),
        Ok(now) if now != before => Err(
            "your rules file changed while the folder was scanned, so nothing is planned. Run the command again"
                .into(),
        ),
        Ok(_) => Ok(()),
    }
}

fn is_clean(outcome: &TargetOutcome) -> bool {
    match outcome {
        TargetOutcome::Removed(removed) => !removed.has_errors() && removed.skipped_mounts == 0,
        TargetOutcome::Trashed(_) | TargetOutcome::ByTool(_) => true,
        TargetOutcome::Skipped(_) | TargetOutcome::Failed(_) => false,
    }
}

fn render(plan: &CleanupPlan, not_found: &[String], verbose: bool, dry_run: bool) -> String {
    let first = if dry_run {
        "Dry run: nothing was changed."
    } else {
        "Plan:"
    };
    let mut lines = vec![first.to_owned(), String::new()];

    if plan.groups().is_empty() {
        lines.push("Nothing to clean.".to_owned());
    } else {
        let rows: Vec<Vec<String>> = plan
            .groups()
            .iter()
            .map(|group| {
                vec![
                    group.rule_id().to_owned(),
                    action_name(&group.action()),
                    group.targets().len().to_string(),
                    format::bytes(group.allocated()),
                ]
            })
            .collect();
        lines.push(format::table(&["RULE", "ACTION", "ITEMS", "SIZE"], &rows));
        lines.push(String::new());
        lines.push(format!(
            "{} {} (as scanned, though hard links and shared blocks can make the space freed differ).",
            if dry_run { "Would clean" } else { "Cleaning" },
            format::bytes(plan.allocated)
        ));

        if verbose {
            lines.push(String::new());
            for group in plan.groups() {
                for target in group.targets() {
                    lines.push(format!(
                        "  {}  {}",
                        group.rule_id(),
                        format::display(&target.display_path())
                    ));
                }
            }
        }
    }

    if !plan.refused.is_empty() || !plan.left_out.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Left out ({}):",
            plan.refused.len() + plan.left_out.len()
        ));
        for refused in &plan.refused {
            lines.push(format!(
                "  {}  {}: {}",
                refused.rule_id,
                format::display(&refused.path),
                format::display(&refused.reason.to_string())
            ));
        }
        for left_out in &plan.left_out {
            lines.push(format!(
                "  {}  {}: {}",
                left_out.rule_id,
                format::display(&left_out.path),
                left_out.reason
            ));
        }
    }

    if !not_found.is_empty() {
        lines.push(String::new());
        let ids: Vec<String> = not_found.iter().map(|id| format::display(id)).collect();
        lines.push(format!("Found nothing for: {}", ids.join(", ")));
    }

    lines.join("\n")
}

fn render_report(report: &CleanupReport) -> String {
    let mut lines = vec![String::new()];
    let mut problems = Vec::new();

    for result in &report.results {
        let path = format::display(&result.path);
        match &result.outcome {
            TargetOutcome::Removed(removed) => {
                for (inner, error) in &removed.errors {
                    problems.push(format!(
                        "  {}: {}",
                        format::display(&String::from_utf8_lossy(inner)),
                        format::display(error)
                    ));
                }
                if removed.more_errors > 0 {
                    problems.push(format!(
                        "  {path}: and {} more problems inside",
                        removed.more_errors
                    ));
                }
                if removed.skipped_mounts > 0 {
                    problems.push(format!(
                        "  {path}: {} mounted folders inside were left alone",
                        removed.skipped_mounts
                    ));
                }
            }
            TargetOutcome::Trashed(_) | TargetOutcome::ByTool(_) => {}
            TargetOutcome::Skipped(why) | TargetOutcome::Failed(why) => {
                problems.push(format!("  {path}: {}", format::display(why)));
            }
        }
    }

    let cleaned = report.results.iter().filter(|r| is_clean(&r.outcome)).count();
    let with_problems = report.results.len() - cleaned;
    let mut summary = format!("Cleaned {cleaned} of {} items.", report.results.len());
    if with_problems > 0 {
        summary.push_str(&format!(" {with_problems} with problems (listed below)."));
    }
    lines.push(summary);

    // Moving to the trash frees nothing until the trash is emptied.
    let only_trashed = report
        .results
        .iter()
        .filter(|r| is_clean(&r.outcome))
        .all(|r| matches!(r.outcome, TargetOutcome::Trashed(_)));
    match report.freed {
        Some(_) if cleaned > 0 && only_trashed => {
            lines.push("Moved to the trash: the space is freed when the trash is emptied.".to_owned())
        }
        Some(0) if cleaned > 0 => lines.push(
            "Free space has not changed yet: the filesystem may release it a little later, or snapshots and \
             other copies still hold those blocks."
                .to_owned(),
        ),
        Some(freed) if freed >= 0 => lines.push(format!(
            "Free space grew by {}.",
            format::bytes(u64::try_from(freed).unwrap_or(u64::MAX))
        )),
        Some(_) => lines.push(
            "Free space did not grow (other programs wrote meanwhile, or the space is shared).".to_owned(),
        ),
        None => lines.push("Free space could not be measured.".to_owned()),
    }
    if !report.tools_not_run.is_empty() {
        lines.push(format!(
            "Not run yet (needs the owning tool): {}",
            report.tools_not_run.join(", ")
        ));
    }
    if report.cancelled {
        lines.push("Stopped early: cancelled.".to_owned());
    }
    if !problems.is_empty() {
        lines.push(String::new());
        lines.push("Problems:".to_owned());
        lines.extend(problems);
    }

    lines.join("\n")
}

fn action_name(action: &CleanupAction) -> String {
    match action {
        CleanupAction::Delete => "delete".to_owned(),
        CleanupAction::EmptyFolder => "empty folder".to_owned(),
        CleanupAction::Trash => "move to trash".to_owned(),
        CleanupAction::EmptyTrash => "empty the trash".to_owned(),
        CleanupAction::RunTool(tool) => {
            let elevated = if tool.needs_elevation() {
                " (as administrator)"
            } else {
                ""
            };
            format!("run {} {}{elevated}", tool.program(), tool.args().join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use platter_core::UserVerdict;

    use super::*;

    fn rule(id: u32, verdict: UserVerdict, folder: &str) -> UserRule {
        UserRule {
            id,
            verdict,
            folder: folder.into(),
        }
    }

    #[test]
    fn rules_that_changed_or_broke_while_scanning_plan_nothing() {
        let before = vec![rule(1, UserVerdict::Safe, "/home/u/a")];

        assert!(unchanged_rules(&before, Ok(before.clone())).is_ok());

        let kept_now = vec![rule(1, UserVerdict::Keep, "/home/u/a")];
        let changed = unchanged_rules(&before, Ok(kept_now)).unwrap_err().to_string();
        assert!(
            changed.contains("changed while the folder was scanned"),
            "{changed}"
        );

        let removed = unchanged_rules(&before, Ok(Vec::new()));
        assert!(removed.is_err());

        let broken = unchanged_rules(&before, Err("The rules file is not valid.".to_owned()))
            .unwrap_err()
            .to_string();
        assert!(
            broken.contains("cannot be used") && broken.contains("not valid"),
            "{broken}"
        );
    }
}
