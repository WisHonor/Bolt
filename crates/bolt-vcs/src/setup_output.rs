//! What `bolt start` and `bolt setup` say about a project's rules, for people and as JSON.

use std::io::{self, Write};
use std::time::Duration;

use bolt_core::changes::file_lines;
use bolt_core::diff::{ChangeKind, FileChange};
use bolt_core::listing::EntryKind;
use bolt_core::presets::{Engine, Found, Language, Looked, Preset, suggested};
use bolt_core::rules::{Rules, RulesRead, size_text};
use bolt_core::setup::RuleFileChange;
use bolt_core::text::{bytes, escape, thousands};
use serde_json::{Value, json};

use crate::output::write_lines;
use crate::style::{self, Role, Sym};

/// How long a rule file's lines are compared before they show in bigger pieces.
const DIFF_DEADLINE: Duration = Duration::from_secs(2);

pub fn title(preset: Preset) -> &'static str {
    match preset {
        Preset::Code => "Code",
        Preset::Game => "Game",
        Preset::Custom => "Custom",
    }
}

fn engine_title(engine: Engine) -> &'static str {
    match engine {
        Engine::Unity => "Unity",
        Engine::Unreal => "Unreal",
        Engine::Godot => "Godot",
    }
}

fn language_title(language: Language) -> &'static str {
    match language {
        Language::Node => "Node",
        Language::Python => "Python",
        Language::Rust => "Rust",
        Language::Go => "Go",
        Language::Dart => "Dart",
        Language::Dotnet => ".NET",
        Language::Maven => "Maven",
        Language::Gradle => "Gradle",
    }
}

/// "A", "A and B", "A, B and C".
fn joined(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

pub fn found_json(found: &Found) -> Value {
    json!({
        "engines": found.engines.iter().map(|e| e.name()).collect::<Vec<_>>(),
        "languages": found.languages.iter().map(|l| l.name()).collect::<Vec<_>>(),
    })
}

/// What a look found, in fixed words and numbers only: never a name read from the folder.
pub fn found_words(looked: &Looked) -> String {
    let more = if looked.counted_all { "" } else { "more than " };
    let size = format!("{more}{} of files", bytes(looked.bytes));
    let names: Vec<&str> = if looked.found.engines.is_empty() {
        looked
            .found
            .languages
            .iter()
            .map(|l| language_title(*l))
            .collect()
    } else {
        looked
            .found
            .engines
            .iter()
            .map(|e| engine_title(*e))
            .collect()
    };
    if names.is_empty() {
        format!("found no game engine's or language's project files, and {size}")
    } else {
        format!("found {} files and {size}", joined(&names))
    }
}

/// The line the preset question starts with.
pub fn look_line(looked: &Looked) -> String {
    let found = found_words(looked);
    match suggested(&looked.found) {
        Preset::Game => format!("Looks like a game project: {found}."),
        Preset::Code => format!("Looks like a code project: {found}."),
        Preset::Custom => format!("Bolt can't tell what kind of project this is: it {found}."),
    }
}

/// What `bolt start` says once the project is made: the outcome, the rules in words, and the first
/// save. `preset` is `None` where the folder kept a `bolt.toml` of its own.
pub fn started(
    root: &str,
    preset: Option<Preset>,
    found: &Found,
    rules: &Rules,
    short: Option<&str>,
) -> String {
    let s = style::out();
    let bullet = s.sym(Sym::Bullet);
    let mut text = format!(
        "{} a Bolt project in {}\n\n",
        s.paint(Role::Ok, "Started"),
        s.paint(Role::Path, root)
    );
    let names = |list: Vec<&str>| {
        if list.is_empty() {
            String::new()
        } else {
            format!(" ({})", list.join(", "))
        }
    };
    text += &match preset {
        None => "Using the rules in this folder's bolt.toml, which stay as they are:".to_owned(),
        Some(Preset::Custom) => format!(
            "Using {} settings: Bolt's plain defaults, with no rule files:",
            s.paint(Role::Bold, "Custom")
        ),
        Some(Preset::Game) => format!(
            "Using {} settings{}:",
            s.paint(Role::Bold, "Game"),
            names(found.engines.iter().map(|e| engine_title(*e)).collect())
        ),
        Some(Preset::Code) => format!(
            "Using {} settings{}:",
            s.paint(Role::Bold, "Code"),
            names(found.languages.iter().map(|l| language_title(*l)).collect())
        ),
    };
    text += "\n\n";
    let mut said = vec![format!(
        "Files of {} or more are stored in chunks, so a changed part is stored once",
        size_text(rules.chunk_files_over)
    )];
    match preset {
        Some(Preset::Game) if found.engines.is_empty() => said.push(
            "No game engine was found, so no engine's files are left out yet: run bolt setup \
             --preset game once the engine has made its project"
                .to_owned(),
        ),
        Some(Preset::Game) => said.extend(found.engines.iter().map(|e| left_out(*e).to_owned())),
        Some(Preset::Code) if found.languages.is_empty() => {
            said.push(".env files are left out".to_owned());
        }
        Some(Preset::Code) => said.push(format!(
            "Build output and caches of {} are left out, and so are .env files",
            joined(
                &found
                    .languages
                    .iter()
                    .map(|l| language_title(*l))
                    .collect::<Vec<_>>()
            )
        )),
        _ => {}
    }
    said.push(format!(
        "A save that adds over {} or {} new files asks first",
        size_text(rules.ask_over_bytes),
        thousands(rules.ask_over_files)
    ));
    if preset == Some(Preset::Game) {
        let scenes = if found.engines.is_empty() {
            ""
        } else {
            ", and the engine's scenes and assets,"
        };
        said.push(format!(
            "Art and audio files{scenes} are marked lockable; locking arrives with syncing"
        ));
    }
    for line in said {
        text += &s.detail(Some(bullet), &line);
        text.push('\n');
    }
    let setup = s.command("bolt setup");
    let see = match preset {
        None => format!("See them any time with: {setup}"),
        Some(Preset::Custom) => {
            format!("There are no rule files. See the rules any time with: {setup}")
        }
        Some(_) => {
            format!("The rules are in bolt.toml and .boltignore. See them any time with: {setup}")
        }
    };
    text += &format!("\n{}\n", s.detail(None, &see));
    if let Some(short) = short {
        text += &format!(
            "\nYour first save is {} on line {}.\n",
            s.paint(Role::ChangeId, short),
            s.paint(Role::Line, "main")
        );
    }
    text
}

/// What an engine's lines in `.boltignore` leave out, in words.
fn left_out(engine: Engine) -> &'static str {
    match engine {
        Engine::Unity => "Unity's Library/, Temp/, Logs/ and other generated files are left out",
        Engine::Unreal => {
            "Unreal's Binaries/, Intermediate/, Saved/ and other generated files are left out"
        }
        Engine::Godot => "Godot's .godot/, .import/ and other generated files are left out",
    }
}

/// `bolt setup`'s list: the rules as Bolt reads them, each project text escaped.
pub fn rules_shown(read: &RulesRead, block_lines: Option<usize>) -> String {
    let s = style::out();
    let rules = &read.rules;
    let mut text = match (&read.rules.preset, read.file) {
        _ if read.skipped.is_some() => {
            "bolt.toml couldn't be used, so Bolt's own values apply:".to_owned()
        }
        (_, false) => "No bolt.toml here, so Bolt's own values apply:".to_owned(),
        (Some(label), true) => format!(
            "Using {} settings, from bolt.toml:",
            s.paint(Role::Bold, &escape(label, false))
        ),
        (None, true) => "Using the rules in bolt.toml:".to_owned(),
    };
    text += "\n\n";
    let patterns = |pick: &dyn Fn(&bolt_core::rules::Rule) -> bool| {
        let all: Vec<String> = rules
            .rules
            .iter()
            .filter(|r| pick(r))
            .flat_map(|r| r.files.iter().map(|f| shown_pattern(f)))
            .collect();
        if all.is_empty() {
            "none".to_owned()
        } else {
            all.join(" ")
        }
    };
    let ignore = match block_lines {
        None => "no preset lines in .boltignore".to_owned(),
        Some(1) => "1 line in .boltignore".to_owned(),
        Some(n) => format!("{} lines in .boltignore", thousands(n as u64)),
    };
    let rows = [
        (
            "Files stored in chunks",
            format!("{} or more", size_text(rules.chunk_files_over)),
        ),
        (
            "A save asks first",
            format!(
                "over {} or {} new files",
                size_text(rules.ask_over_bytes),
                thousands(rules.ask_over_files)
            ),
        ),
        ("Lockable", patterns(&|r| r.lockable == Some(true))),
        (
            "No conflict markers",
            patterns(&|r| r.conflict_markers == Some(false)),
        ),
        ("Left out by the preset", ignore),
    ];
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0) + 2;
    for (label, value) in rows {
        text += &format!("    {label:<width$}{value}\n");
    }
    text
}

/// A rule's pattern, escaped; between double quotes, as `bolt.toml` writes it, when it holds a
/// space, so the list reads as its patterns.
fn shown_pattern(pattern: &str) -> String {
    let shown = escape(pattern, false);
    if pattern.contains(' ') {
        format!("\"{shown}\"")
    } else {
        shown
    }
}

pub fn rules_json(read: &RulesRead, block_lines: Option<usize>) -> Value {
    let rules = &read.rules;
    json!({
        "has_rules_file": read.file,
        "preset": rules.preset,
        "chunk_files_over": rules.chunk_files_over,
        "ask_before_saving_over": rules.ask_over_bytes,
        "ask_before_saving_files_over": rules.ask_over_files,
        "rules": rules.rules.iter().map(|r| json!({
            "files": r.files,
            "lockable": r.lockable,
            "conflict_markers": r.conflict_markers,
        })).collect::<Vec<_>>(),
        "ignore_block_lines": block_lines,
    })
}

pub fn changes_json(changes: &[RuleFileChange]) -> Value {
    let text = |b: &Option<Vec<u8>>| b.as_ref().map(|b| String::from_utf8_lossy(b).into_owned());
    changes
        .iter()
        .map(|c| json!({"path": c.path, "before": text(&c.before), "after": text(&c.after)}))
        .collect()
}

/// Each change, line by line, as `bolt changes` shows a file, one empty line between files.
pub fn write_preview(out: &mut dyn Write, changes: &[RuleFileChange]) -> io::Result<()> {
    for (i, c) in changes.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        let size = |b: &Option<Vec<u8>>| b.as_ref().map(|b| b.len() as u64);
        let change = FileChange {
            path: c.path,
            change: match (&c.before, &c.after) {
                (None, _) => ChangeKind::Added,
                (_, None) => ChangeKind::Removed,
                _ => ChangeKind::Modified,
            },
            kind: EntryKind::File,
            old_kind: c.before.as_ref().map(|_| EntryKind::File),
            old_size: size(&c.before),
            new_size: size(&c.after),
            old_id: None,
            new_id: None,
        };
        let empty = Vec::new();
        let lines = file_lines(
            c.before.as_ref().unwrap_or(&empty),
            c.after.as_ref().unwrap_or(&empty),
            DIFF_DEADLINE,
        );
        write_lines(out, &change, &lines)?;
    }
    Ok(())
}

/// What a switch to `preset` says once it is done, after the preview: what it wrote, or that it
/// wrote nothing.
pub fn switched(preset: Preset, changes: &[RuleFileChange], written: &[&str]) -> String {
    let s = style::out();
    if written.is_empty() {
        let again = format!("bolt setup --preset {}", preset.name());
        return format!(
            "\nNothing was written: the rule files changed while Bolt worked. Run {} again.\n",
            s.paint(Role::Command, &again)
        );
    }
    format!(
        "\n{} to {} settings: {}.\n",
        s.paint(Role::Ok, "Switched"),
        title(preset),
        written_words(changes, written)
    )
}

/// What a switch did to each file: "made bolt.toml, changed .boltignore".
fn written_words(changes: &[RuleFileChange], written: &[&str]) -> String {
    let words: Vec<String> = changes
        .iter()
        .filter(|c| written.contains(&c.path))
        .map(|c| {
            let verb = match (&c.before, &c.after) {
                (None, _) => "made",
                (_, None) => "removed",
                _ => "changed",
            };
            format!("{verb} {}", c.path)
        })
        .collect();
    words.join(", ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use bolt_core::rules::Rule;

    #[test]
    fn a_pattern_with_a_space_is_shown_between_quotes() {
        let mut rules = Rules::default();
        rules.rules.push(Rule {
            files: vec!["My Art/*.psd".into(), "*.png".into()],
            lockable: Some(true),
            conflict_markers: None,
        });
        let read = RulesRead {
            rules,
            file: true,
            skipped: None,
            ignored: Vec::new(),
        };
        let shown = rules_shown(&read, None);
        let row = shown.lines().find(|l| l.contains("Lockable")).unwrap();
        assert!(row.ends_with(" \"My Art/*.psd\" *.png"), "{row}");
    }

    #[test]
    fn a_switch_that_wrote_nothing_says_so() {
        let changes = [RuleFileChange {
            path: "bolt.toml",
            before: None,
            after: Some(b"x".to_vec()),
        }];
        let said = switched(Preset::Game, &changes, &[]);
        assert!(!said.contains("Switched"), "{said}");
        assert_eq!(
            said,
            "\nNothing was written: the rule files changed while Bolt worked. Run bolt setup \
             --preset game again.\n"
        );
        let said = switched(Preset::Game, &changes, &["bolt.toml"]);
        assert_eq!(said, "\nSwitched to Game settings: made bolt.toml.\n");
    }
}
