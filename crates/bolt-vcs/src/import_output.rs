//! What `bolt import` prints: its summary for people, and its `--json` document.

use bolt_core::diary::Position;
use bolt_core::import::{ImportReport, MAX_LISTED};
use bolt_core::text::{escape, escape_all, thousands};
use bolt_core::warning::Warning;
use serde_json::json;

use crate::output::{self, BOLT_JSON};

/// "1 tag" or "2 tags", with the count as `thousands` writes it.
fn counted(count: u64, one: &str, many: &str) -> String {
    let noun = if count == 1 { one } else { many };
    format!("{} {noun}", thousands(count))
}

/// `listed`, the first of `count` things, and a last item counting the rest, if there are more.
fn and_the_rest(mut listed: Vec<String>, count: u64) -> Vec<String> {
    let more = count.saturating_sub(listed.len() as u64);
    if more > 0 {
        listed.push(format!("and {} more", thousands(more)));
    }
    listed
}

/// Prints `heading` and `items` after it: one on the same line, more on a line each.
fn print_list(heading: &str, items: &[String]) {
    match items {
        [] => {}
        [one] => println!("{heading}: {one}"),
        _ => {
            println!("{heading}:");
            for item in items {
                println!("  {item}");
            }
        }
    }
}

/// What `bolt import` tells people it did: what came in, where they stand, and what was renamed
/// or left out.
pub fn print_import(report: &ImportReport, standing: &str) {
    let mut parts = vec![counted(report.lines, "line", "lines")];
    if report.servers > 0 {
        let lines = counted(report.server_lines, "server line", "server lines");
        parts.push(format!(
            "{lines} from {}",
            counted(report.servers, "server", "servers")
        ));
    }
    parts.push(counted(report.tags, "tag", "tags"));
    let commits = counted(report.commits, "commit", "commits");
    println!("Imported {commits} from Git: {}.", parts.join(", "));
    println!("{standing}");
    let renamed: Vec<String> = report
        .renamed
        .iter()
        .take(MAX_LISTED)
        .map(|r| {
            let kind = match r.kind {
                "server" => "server ",
                "tag" => "tag ",
                _ => "",
            };
            format!(
                "{kind}{} → {}",
                escape(&r.git, false),
                escape(&r.bolt, false)
            )
        })
        .collect();
    let count = report.renamed.len() as u64;
    print_list(
        &format!(
            "Renamed {} that Bolt's names don't allow",
            counted(count, "name", "names")
        ),
        &and_the_rest(renamed, count),
    );
    let refs = report.skipped_refs.iter().map(|r| escape(r, false));
    let heading = format!(
        "Didn't import {} (only branches, server branches and tags come in)",
        counted(report.skipped_ref_count, "ref", "refs")
    );
    print_list(
        &heading,
        &and_the_rest(refs.collect(), report.skipped_ref_count),
    );
    let tags: Vec<String> = report
        .skipped_tags
        .iter()
        .take(MAX_LISTED)
        .map(|(name, why)| format!("{}, as {why}", escape(name, false)))
        .collect();
    let count = report.skipped_tags.len() as u64;
    let heading = format!("Didn't import {}", counted(count, "tag", "tags"));
    print_list(&heading, &and_the_rest(tags, count));
    let entries: Vec<String> = report
        .skipped_entries
        .iter()
        .map(|(commit, path)| {
            // A name can be empty, and an empty one at the top has no path to show. Every hidden
            // character shows, so that a name left out can't pass for another.
            let path = if path.is_empty() {
                "an empty name".to_owned()
            } else {
                format!("`{}`", escape_all(path))
            };
            let commit = commit.get(..12).unwrap_or(commit);
            format!("{path} in commit {}", escape(commit, false))
        })
        .collect();
    let entries = and_the_rest(entries, report.skipped_entry_count);
    let heading = if report.skipped_entry_count == 1 {
        "Left out 1 entry whose name Bolt never writes to disk (such as .git and .bolt), with \
         everything in it"
            .to_owned()
    } else {
        format!(
            "Left out {} entries whose names Bolt never writes to disk (such as .git and .bolt), \
             with everything in them",
            thousands(report.skipped_entry_count)
        )
    };
    print_list(&heading, &entries);
    match report.inexact_commits {
        0 => {}
        1 => println!(
            "1 commit can't be recreated exactly by a later export: it holds a folder in a form \
             Git no longer writes, or an entry Bolt left out."
        ),
        n => println!(
            "{} commits can't be recreated exactly by a later export: they hold folders in a form \
             Git no longer writes, or entries Bolt left out.",
            thousands(n)
        ),
    }
}

/// The `--json` document of `bolt import` (docs/json.md).
pub fn import_json(report: &ImportReport, warnings: &[Warning]) -> serde_json::Value {
    let position = match &report.position {
        Position::OnLine(line) => json!({ "line": line }),
        Position::AtSave(id) => json!({ "save": id.to_hex() }),
    };
    let renamed: Vec<serde_json::Value> = report
        .renamed
        .iter()
        .map(|r| json!({"kind": r.kind, "git": r.git, "bolt": r.bolt}))
        .collect();
    let skipped_tags: Vec<serde_json::Value> = report
        .skipped_tags
        .iter()
        .map(|(git, reason)| json!({"git": git, "reason": reason}))
        .collect();
    let first: Vec<serde_json::Value> = report
        .skipped_entries
        .iter()
        .map(|(commit, path)| json!({"commit": commit, "path": path}))
        .collect();
    json!({
        "bolt_json": BOLT_JSON,
        "commits": report.commits,
        "lines": report.lines,
        "servers": report.servers,
        "server_lines": report.server_lines,
        "tags": report.tags,
        "position": position,
        "renamed": renamed,
        "skipped_refs": {"count": report.skipped_ref_count, "first": report.skipped_refs},
        "skipped_tags": skipped_tags,
        "skipped_entries": {"count": report.skipped_entry_count, "first": first},
        "inexact_commits": report.inexact_commits,
        "warnings": output::warnings_json(warnings),
    })
}
