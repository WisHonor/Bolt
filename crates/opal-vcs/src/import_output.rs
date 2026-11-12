//! What `opalro import` prints: its summary for people, and its `--json` document.

use opal_core::import::{ImportReport, MAX_LISTED};
use opal_core::text::{bytes, escape, escape_all, thousands};
use opal_core::warning::Warning;
use serde_json::json;

use crate::output::{self, OPALRO_JSON};
use crate::style::{self, Role};

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

/// One detail, added to `text`: indented 4, and on a terminal wrapped there.
fn detail(text: &mut String, line: &str) {
    *text += &style::out().detail(None, line);
    text.push('\n');
}

/// `heading` and `items` after it, added to `text` as a detail: one on the same line when it
/// fits there, more on a row each under it. An item is the repository's text, so it is never
/// broken, and a long one runs long.
fn list(text: &mut String, heading: &str, items: &[String]) {
    match items {
        [] => {}
        [one] => detail(text, &format!("{heading}: {}", style::whole(one))),
        _ => {
            detail(text, &format!("{heading}:"));
            for item in items {
                *text += &format!("        {item}\n");
            }
        }
    }
}

/// What `opalro import` tells people it did: the outcome line with what came in, then what was
/// renamed or left out, then where they stand.
pub fn import_text(report: &ImportReport, standing: &str) -> String {
    let outcome = style::out().paint(Role::Ok, "Imported");
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
    let mut text = String::new();
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
                "{kind}{} {} {}",
                escape(&r.git, false),
                crate::style::out().sym(crate::style::Sym::Arrow),
                escape(&r.opalro, false)
            )
        })
        .collect();
    let count = report.renamed.len() as u64;
    list(
        &mut text,
        &format!(
            "Renamed {} that Opalro's names don't allow",
            counted(count, "name", "names")
        ),
        &and_the_rest(renamed, count),
    );
    let refs = report.skipped_refs.iter().map(|r| escape(r, false));
    let heading = format!(
        "Didn't import {} (only branches, server branches and tags come in)",
        counted(report.skipped_ref_count, "ref", "refs")
    );
    list(
        &mut text,
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
    list(&mut text, &heading, &and_the_rest(tags, count));
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
        "Left out 1 entry whose name Opalro never writes to disk (such as .git and .opalro), with \
         everything in it"
            .to_owned()
    } else {
        format!(
            "Left out {} entries whose names Opalro never writes to disk (such as .git and .opalro), \
             with everything in them",
            thousands(report.skipped_entry_count)
        )
    };
    list(&mut text, &heading, &entries);
    match report.inexact_commits {
        0 => {}
        1 => detail(
            &mut text,
            "1 commit can't be recreated exactly by a later export: it holds a folder in a form \
             Git no longer writes, or an entry Opalro left out.",
        ),
        n => detail(
            &mut text,
            &format!(
                "{} commits can't be recreated exactly by a later export: they hold folders in a \
                 form Git no longer writes, or entries Opalro left out.",
                thousands(n)
            ),
        ),
    }
    lfs_text(&mut text, report);
    let details = if text.is_empty() {
        text
    } else {
        format!("\n{text}")
    };
    format!(
        "{outcome} {commits} from Git: {}.\n{details}\n{standing}\n",
        parts.join(", ")
    )
}

/// What the summary says of Git LFS files, added to `text`: how many came in, the commits that
/// hold them, and the pointers a later export can't give back as they were.
fn lfs_text(text: &mut String, report: &ImportReport) {
    if report.lfs_files > 0 {
        let files = counted(report.lfs_files, "Git LFS file", "Git LFS files");
        let size = bytes(report.lfs_bytes);
        detail(
            text,
            &format!("Brought in {files} ({size}) from .git/lfs/objects."),
        );
    }
    match report.lfs_commits {
        0 => {}
        1 => detail(
            text,
            "1 commit holds Git LFS files, so a later export can recreate its Git ID only by \
             turning them back into pointers.",
        ),
        n => detail(
            text,
            &format!(
                "{} commits hold Git LFS files, so a later export can recreate their Git IDs only \
                 by turning those files back into pointers.",
                thousands(n)
            ),
        ),
    }
    match report.inexact_lfs_pointers {
        0 => {}
        1 => detail(
            text,
            "1 Git LFS pointer isn't in the exact form Git LFS writes, so a later export can't \
             recreate it as it was.",
        ),
        n => detail(
            text,
            &format!(
                "{} Git LFS pointers aren't in the exact form Git LFS writes, so a later export \
                 can't recreate them as they were.",
                thousands(n)
            ),
        ),
    }
}

/// The `--json` document of `opalro import` (docs/json.md).
pub fn import_json(report: &ImportReport, warnings: &[Warning]) -> serde_json::Value {
    let position = crate::output::position_json(&report.position);
    let renamed: Vec<serde_json::Value> = report
        .renamed
        .iter()
        .map(|r| json!({"kind": r.kind, "git": r.git, "opalro": r.opalro}))
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
        "opalro_json": OPALRO_JSON,
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
        "lfs_files": {"count": report.lfs_files, "bytes": report.lfs_bytes},
        "lfs_commits": report.lfs_commits,
        "inexact_lfs_pointers": report.inexact_lfs_pointers,
        "warnings": output::warnings_json(warnings),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use opal_core::git::GitError;
    use opal_core::import::ImportError;

    use crate::output::{Failure, failure_lines};
    use crate::style::Screen;

    /// The import's stop past its work allowance can't be reached from a test's repository in
    /// reasonable time, so its error is made here and laid out as `opalro import` lays it out.
    #[test]
    fn the_work_stop_is_laid_out_like_every_other_error() {
        let what = "rebuilding objects from Git's deltas would take more than the 4 GiB of work \
                    Opalro allows: the repository holds a long history of small edits to a big \
                    file, or it was built to make importing slow";
        let way_out = "If you trust this repository, have Git store its files over 1 MiB whole \
                       and make its other deltas anew, with `git -c core.bigFileThreshold=1m \
                       repack -a -d -f`, then run opalro import again. Git's packs stay bigger \
                       until its next `git gc`.";
        let stop = || ImportError::Git(GitError::TooMuchWork { what: what.into() });
        // Before the import began its .opalro folder, and after.
        let after = ImportError::Unfinished {
            stop: Box::new(stop()),
            committed: false,
        };
        for (stop, hint) in [
            (stop(), format!("Nothing was changed. {way_out}")),
            (
                after,
                format!(
                    "{way_out}\nNothing was committed. Opalro kept the .opalro folder it began here: \
                     opalro import stops the same way until the cause is fixed, then finishes from \
                     what it stored. Or delete that folder."
                ),
            ),
        ] {
            let failure = Failure::from(stop);
            assert_eq!(failure.hint.as_deref(), Some(&hint[..]));
            let error = format!("Opalro can't import this Git repository: {what}.");
            let plain = Screen {
                color: false,
                symbols: false,
                width: None,
            };
            assert_eq!(
                failure_lines(plain, &failure).join("\n"),
                format!("error: {error}\n\nhint: {hint}")
            );
            for width in [60, 80, 120] {
                let screen = Screen {
                    width: Some(width),
                    ..plain
                };
                let lines = failure_lines(screen, &failure);
                let blank = lines.iter().position(String::is_empty).unwrap();
                let (said, hinted) = (&lines[..blank], &lines[blank + 1..]);
                assert!(said[0].starts_with("error: Opalro can't"), "{lines:?}");
                assert!(hinted[0].starts_with("hint: "), "{lines:?}");
                for line in &lines {
                    assert!(line.chars().count() <= width.min(100), "{width}: {line:?}");
                }
                for line in &said[1..] {
                    assert!(line.starts_with("       ") && !line.starts_with("        "));
                }
                for line in &hinted[1..] {
                    assert!(line.starts_with("      ") && !line.starts_with("       "));
                }
                let words = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
                assert_eq!(words(&said.join(" ")), words(&format!("error: {error}")));
                assert_eq!(words(&hinted.join(" ")), words(&format!("hint: {hint}")));
            }
        }
    }
}
