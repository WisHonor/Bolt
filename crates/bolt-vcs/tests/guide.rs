//! The user guide, the README and SECURITY.md against the program: every command the guide shows
//! runs, every link points somewhere, and nothing that ships points into the private repository.
//! And the manifests: no crate can be published, and each names the license.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

mod support;
use support::{Env, text};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_doc(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The lines of every fenced block marked `lang`, in order, each with its line number.
fn blocks(doc: &str, lang: &str) -> Vec<Vec<(usize, String)>> {
    let mut found = Vec::new();
    let mut open: Option<Vec<(usize, String)>> = None;
    for (n, line) in doc.lines().enumerate() {
        match &mut open {
            Some(block) if line.trim_start().starts_with("```") => {
                found.push(std::mem::take(block));
                open = None;
            }
            Some(block) => block.push((n + 1, line.to_owned())),
            None if line.trim() == format!("```{lang}") => open = Some(Vec::new()),
            None => {}
        }
    }
    assert!(open.is_none(), "a ```{lang} block is never closed");
    found
}

/// Fails unless every fenced block is marked `sh`, `text` or `toml`, so a block marked anything
/// else can't be passed over unrun.
fn assert_fences_known(doc: &str) {
    let mut open = false;
    for (n, line) in doc.lines().enumerate() {
        let Some(lang) = line.trim_start().strip_prefix("```") else {
            continue;
        };
        if !open {
            assert!(
                ["sh", "text", "toml"].contains(&lang.trim()),
                "guide.md line {}: a fence marked {lang:?}; use sh, text or toml",
                n + 1
            );
        }
        open = !open;
    }
}

/// A command line's words: split at spaces, with double quotes keeping a word together.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut started) = (false, false);
    for c in line.chars() {
        match c {
            '"' => (quoted, started) = (!quoted, true),
            ' ' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                }
                started = false;
            }
            _ => {
                word.push(c);
                started = true;
            }
        }
    }
    assert!(!quoted, "an unclosed quote in {line:?}");
    if started {
        words.push(word);
    }
    words
}

/// `echo "text" > path` or `>>`: the text, the path, and whether it appends.
fn echo(line: &str) -> Option<(String, String, bool)> {
    let rest = line.strip_prefix("echo \"")?;
    let (said, rest) = rest.split_once('"')?;
    let (append, path) = match rest.strip_prefix(" >> ") {
        Some(path) => (true, path),
        None => (false, rest.strip_prefix(" > ")?),
    };
    (!path.is_empty() && !path.contains(' ')).then(|| (said.to_owned(), path.to_owned(), append))
}

/// The commands `bolt help` lists, with those named beside another (`(and: redo)`).
fn commands(env: &Env) -> BTreeSet<String> {
    let out = env.bolt().arg("help").output().unwrap();
    assert!(out.status.success());
    let mut names = BTreeSet::new();
    for line in text(&out.stdout).lines() {
        let Some((first, rest)) = line.split_once("  ") else {
            continue;
        };
        if !first.is_empty() && first.bytes().all(|b| b.is_ascii_lowercase()) {
            names.insert(first.to_owned());
            if let Some((_, beside)) = rest.split_once("(and: ") {
                names.insert(beside.trim_end_matches(')').to_owned());
            }
        }
    }
    names
}

#[test]
fn every_command_in_the_guide_runs() {
    let guide = read_doc("docs/guide.md");
    let env = Env::empty();
    assert_fences_known(&guide);

    // The guide's first step tells Bolt who you are in the settings file; this run's own one.
    let settings = blocks(&guide, "toml");
    let who: Vec<&str> = settings
        .first()
        .expect("the guide has no ```toml block saying who you are")
        .iter()
        .map(|(_, l)| l.as_str())
        .collect();
    assert!(
        who.contains(&"[user]"),
        "the guide's first toml block isn't [user]: {who:?}"
    );
    std::fs::write(env.config().join("config.toml"), who.join("\n") + "\n").unwrap();

    let mut ran = 0;
    let mut used = BTreeSet::new();
    for (n, line) in blocks(&guide, "sh").into_iter().flatten() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(args) = line.strip_prefix("bolt ") {
            let args = words(args);
            let out = env
                .bolt()
                .env_remove("BOLT_USER_NAME")
                .env_remove("BOLT_USER_EMAIL")
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(0),
                "guide.md line {n}: `{line}` failed\n--- stdout\n{}--- stderr\n{}",
                text(&out.stdout),
                text(&out.stderr)
            );
            used.insert(args[0].clone());
        } else if let Some((said, path, append)) = echo(line) {
            let at = env.root().join(&path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            let mut bytes = if append {
                std::fs::read(&at).unwrap_or_default()
            } else {
                Vec::new()
            };
            bytes.extend_from_slice(format!("{said}\n").as_bytes());
            std::fs::write(&at, bytes).unwrap();
        } else {
            panic!(
                "guide.md line {n}: `{line}` can't run in a test: an ```sh block holds only bolt \
                 commands and `echo \"text\" > path` or `>> path`, so this line belongs in a \
                 ```text block"
            );
        }
        ran += 1;
    }
    assert!(
        ran >= 30,
        "the guide's ```sh blocks hold only {ran} runnable lines"
    );

    // Every command but import, which needs a Git repository, is shown running.
    let known = commands(&env);
    assert!(
        known.len() >= 17,
        "bolt help lists only {known:?}, so this check would check little"
    );
    let missing: Vec<String> = known
        .into_iter()
        .filter(|c| c != "import" && !used.contains(c))
        .collect();
    assert!(missing.is_empty(), "the guide never runs: {missing:?}");
}

/// The anchor GitHub gives each heading of `doc`: lower case, punctuation dropped, spaces as
/// hyphens, and `-1`, `-2` after a repeat.
fn anchors(doc: &str) -> BTreeSet<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut found = BTreeSet::new();
    let mut fenced = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let Some(title) = line.strip_prefix('#').filter(|_| !fenced) else {
            continue;
        };
        let slug: String = title
            .trim_start_matches('#')
            .trim()
            .to_lowercase()
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .collect();
        let count = seen.entry(slug.clone()).or_default();
        found.insert(if *count == 0 {
            slug.clone()
        } else {
            format!("{slug}-{count}")
        });
        *count += 1;
    }
    found
}

/// Each `](target)` in `doc`.
fn link_targets(doc: &str) -> Vec<String> {
    doc.split("](")
        .skip(1)
        .filter_map(|rest| rest.split_once(')').map(|(t, _)| t.to_owned()))
        .collect()
}

#[test]
fn every_link_in_the_guide_the_readme_and_security_md_leads_somewhere() {
    for rel in ["docs/guide.md", "README.md", "SECURITY.md"] {
        let doc = read_doc(rel);
        let folder = repo_root().join(rel).parent().unwrap().to_owned();
        for target in link_targets(&doc) {
            if target.starts_with("https://") || target.starts_with("http://") {
                continue;
            }
            let (file, anchor) = target.split_once('#').unwrap_or((&target, ""));
            let linked = if file.is_empty() {
                doc.clone()
            } else {
                let at = folder.join(file);
                assert!(at.exists(), "{rel}: the link ({target}) leads to no file");
                if !file.ends_with(".md") {
                    continue;
                }
                std::fs::read_to_string(at).unwrap()
            };
            assert!(
                anchor.is_empty() || anchors(&linked).contains(anchor),
                "{rel}: the link ({target}) leads to no heading"
            );
        }
    }
}

/// `v1.2.3.md`, a version's notes.
fn is_notes(name: &str) -> bool {
    let Some(version) = name.strip_prefix('v').and_then(|n| n.strip_suffix(".md")) else {
        return false;
    };
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

#[test]
fn nothing_that_ships_points_into_the_repository() {
    let mut shipped: Vec<String> = [
        "README.md",
        "docs/guide.md",
        "SECURITY.md",
        "USER-AGREEMENT.md",
        "docs/releases/download-page.md",
        "docs/releases/welcome.md",
        "docs/releases/reporting-a-problem.md",
        "docs/releases/known-problems.md",
    ]
    .map(String::from)
    .to_vec();
    for entry in std::fs::read_dir(repo_root().join("docs/releases")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if is_notes(&name) {
            shipped.push(format!("docs/releases/{name}"));
        }
    }

    let mut wrong = Vec::new();
    for rel in &shipped {
        let doc =
            std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let lower = doc.to_lowercase();
        // Also catches git@github.com:WisHonor/Bolt and raw.githubusercontent.com/WisHonor/Bolt.
        if lower.contains("github.com/wishonor") || lower.contains("wishonor/bolt") {
            wrong.push(format!("{rel}: holds the repository's address"));
        }
        // The README ships beside the guide and the agreement; the others link only to themselves.
        let beside: &[&str] = if rel == "README.md" {
            &["docs/guide.md", "USER-AGREEMENT.md"]
        } else {
            &[]
        };
        for target in link_targets(&doc) {
            let file = target.split('#').next().unwrap_or_default();
            if target.starts_with("https://") || file.is_empty() || beside.contains(&file) {
                continue;
            }
            wrong.push(format!(
                "{rel}: links ({target}), which doesn't ship beside it"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The trimmed lines of the table `[name]` in `toml`, up to the next table.
fn table<'a>(toml: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("[{name}]");
    toml.lines()
        .map(str::trim)
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .collect()
}

#[test]
fn no_crate_can_be_published_and_each_names_the_license() {
    assert!(read_doc("LICENSE").contains("All rights reserved."));
    let holds = |rel: &str, name: &str, want: &[&str]| {
        let doc = read_doc(rel);
        let lines = table(&doc, name);
        for line in want {
            assert!(lines.contains(line), "{rel}: [{name}] lacks `{line}`");
        }
    };
    holds(
        "Cargo.toml",
        "workspace.package",
        &["publish = false", r#"license-file = "LICENSE""#],
    );
    for entry in std::fs::read_dir(repo_root().join("crates")).unwrap() {
        let dir = entry.unwrap().file_name();
        holds(
            &format!("crates/{}/Cargo.toml", dir.to_string_lossy()),
            "package",
            &["publish.workspace = true", "license-file.workspace = true"],
        );
    }
    // fuzz/ is a workspace of its own, so it can't inherit either key.
    holds(
        "fuzz/Cargo.toml",
        "package",
        &["publish = false", r#"license-file = "../LICENSE""#],
    );
}
