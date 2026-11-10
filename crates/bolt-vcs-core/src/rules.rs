//! The project's rules in `bolt.toml` (docs/storage-format.md section 12): what Bolt reads from
//! it, and what it passes over. Nothing in the file ever stops a command.

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use bolt_platform::dir::{Dir, EntryType};
use toml::{Table, Value};

use crate::chunks::{CHUNK_FROM, CHUNK_UP_TO};
use crate::safe_fs;
use crate::text::escape;
use crate::warning::{self, Warning};

/// The longest `bolt.toml` Bolt reads.
pub const MAX_RULES_FILE_BYTES: u64 = 1 << 20;
/// The longest `preset` label, in bytes.
const MAX_PRESET_BYTES: usize = 64;
/// How much of a name or value from the file a warning shows, in characters.
const SHOWN_CHARS: usize = 64;
/// How many passed-over names one warning lists.
const NAMES_LISTED: usize = 20;

/// The units of a size, the 1,024 family first, each family largest first.
const UNITS: [(&str, u64); 9] = [
    ("TiB", 1 << 40),
    ("GiB", 1 << 30),
    ("MiB", 1 << 20),
    ("KiB", 1 << 10),
    ("TB", 1_000_000_000_000),
    ("GB", 1_000_000_000),
    ("MB", 1_000_000),
    ("KB", 1_000),
    ("B", 1),
];

const UNKNOWN: &str = "Bolt doesn't know this key; a newer Bolt may have written it";

/// Settings for the files a rule's patterns match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub files: Vec<String>,
    pub lockable: Option<bool>,
    pub conflict_markers: Option<bool>,
}

/// What `bolt.toml` says, with Bolt's own value for each setting it doesn't give.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    pub preset: Option<String>,
    /// A file of this size or more (up to 128 GiB) is stored in chunks.
    pub chunk_files_over: u64,
    /// A save that adds more new bytes than this, or more new files than the other, asks first.
    pub ask_over_bytes: u64,
    pub ask_over_files: u64,
    pub rules: Vec<Rule>,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            preset: None,
            chunk_files_over: CHUNK_FROM,
            ask_over_bytes: 100_000_000,
            ask_over_files: 10_000,
            rules: vec![],
        }
    }
}

/// A size as `bolt.toml` writes it, in bytes: digits, any number of spaces, then a unit. `None`
/// for anything else, or a size that doesn't fit 64 bits.
#[must_use]
pub fn parse_size(text: &str) -> Option<u64> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let (number, unit) = text.split_at_checked(digits)?;
    let unit = unit.trim_start_matches(' ');
    let (_, times) = UNITS.iter().find(|(name, _)| *name == unit)?;
    // Digits only, so a sign or an empty number fails here.
    number.parse::<u64>().ok()?.checked_mul(*times)
}

/// A setting's size as `bolt.toml` would write it: the largest unit that gives it exactly, the
/// 1,024 family first.
#[must_use]
pub fn size_text(bytes: u64) -> String {
    let (name, times) = UNITS
        .iter()
        .find(|(_, times)| bytes.checked_rem(*times) == Some(0) && (bytes > 0 || *times == 1))
        .unwrap_or(&("B", 1));
    format!("{} {name}", bytes.checked_div(*times).unwrap_or(bytes))
}

/// What a `bolt.toml`'s text means, and each name passed over with why. `Err` says why the text
/// isn't TOML.
pub fn parse(text: &[u8]) -> Result<(Rules, Vec<(String, String)>), String> {
    let text = std::str::from_utf8(text).map_err(|e| {
        format!(
            "it isn't TOML: it isn't UTF-8 text at byte {}",
            e.valid_up_to()
        )
    })?;
    // The parser bounds how deep arrays and tables nest, so a hostile file can't overflow the
    // stack.
    let table: Table = text.parse().map_err(|e| not_toml(text, &e))?;
    let mut rules = Rules::default();
    let mut ignored = Vec::new();
    let mut listed: &[Value] = &[];
    for (key, value) in &table {
        let why = match key.as_str() {
            "preset" => match value.as_str() {
                Some(label) if label.len() <= MAX_PRESET_BYTES => {
                    rules.preset = Some(label.to_owned());
                    continue;
                }
                Some(_) => "it is longer than 64 bytes".to_owned(),
                None => "it must be text".to_owned(),
            },
            "chunk-files-over" => match size(value) {
                Ok(bytes) if (CHUNK_FROM..=CHUNK_UP_TO).contains(&bytes) => {
                    rules.chunk_files_over = bytes;
                    continue;
                }
                Ok(bytes) => format!("{} is outside 1 MiB to 128 GiB", size_text(bytes)),
                Err(why) => why,
            },
            "ask-before-saving-over" => match size(value) {
                Ok(bytes) => {
                    rules.ask_over_bytes = bytes;
                    continue;
                }
                Err(why) => why,
            },
            "ask-before-saving-files-over" => {
                match value.as_integer().and_then(|n| u64::try_from(n).ok()) {
                    Some(files) => {
                        rules.ask_over_files = files;
                        continue;
                    }
                    None => "it must be a whole number, 0 or more".to_owned(),
                }
            }
            "rule" => match value.as_array() {
                Some(list) => {
                    listed = list;
                    continue;
                }
                None => "it must be a list of rules".to_owned(),
            },
            _ => UNKNOWN.to_owned(),
        };
        ignored.push((key.clone(), why));
    }
    // Top-level names in byte order, whatever order the table keeps.
    ignored.sort();
    for (n, item) in (1u64..).zip(listed) {
        match rule(item) {
            Ok((kept, bad)) => {
                rules.rules.push(kept);
                ignored.extend(
                    bad.into_iter()
                        .map(|(k, why)| (format!("rule[{n}].{k}"), why)),
                );
            }
            Err(why) => ignored.push((format!("rule[{n}]"), why)),
        }
    }
    Ok((rules, ignored))
}

/// One `[[rule]]`, and its keys passed over in byte order; `Err` passes it over whole.
fn rule(item: &Value) -> Result<(Rule, Vec<(String, String)>), String> {
    let table = item.as_table().ok_or("it must be a table")?;
    let files = table
        .get("files")
        .and_then(Value::as_array)
        .and_then(|list| {
            let patterns = list.iter().map(|p| p.as_str().filter(|p| !p.is_empty()));
            patterns
                .map(|p| p.map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .filter(|files| !files.is_empty())
        .ok_or("it has no usable `files`: a list of one or more patterns, none of them empty")?;
    let mut rule = Rule {
        files,
        lockable: None,
        conflict_markers: None,
    };
    let mut bad = Vec::new();
    for (key, value) in table {
        let setting = match key.as_str() {
            "files" => continue,
            "lockable" => &mut rule.lockable,
            "conflict-markers" => &mut rule.conflict_markers,
            _ => {
                bad.push((key.clone(), UNKNOWN.to_owned()));
                continue;
            }
        };
        match value.as_bool() {
            Some(on) => *setting = Some(on),
            None => bad.push((key.clone(), "it must be true or false".to_owned())),
        }
    }
    bad.sort();
    Ok((rule, bad))
}

/// A size's bytes, or why `value` isn't one.
fn size(value: &Value) -> Result<u64, String> {
    let Some(text) = value.as_str() else {
        return Err("it must be a size in quotes, such as \"1 MiB\"".to_owned());
    };
    parse_size(text).ok_or_else(|| {
        format!(
            "\"{}\" isn't a size: digits, then B, KB, MB, GB, TB, KiB, MiB, GiB or TiB",
            shown(text)
        )
    })
}

/// Why `text` isn't TOML, as the parser says, at its line. The parser may quote the file, so
/// what it says is escaped and cut.
fn not_toml(text: &str, e: &toml::de::Error) -> String {
    let said = shown(e.message().trim());
    let line = e
        .span()
        .and_then(|span| text.as_bytes().get(..span.start))
        .map(|before| {
            before
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                .saturating_add(1)
        });
    match line {
        Some(line) => format!("it isn't TOML: line {line}: {said}"),
        None => format!("it isn't TOML: {said}"),
    }
}

/// Text from the file as a warning shows it: escaped, and cut so that it can't flood the terminal.
fn shown(text: &str) -> String {
    let mut cut: String = text.chars().take(SHOWN_CHARS).collect();
    if cut.len() < text.len() {
        cut.push_str("...");
    }
    escape(&cut, false)
}

/// The rules as read from the folder.
#[derive(Clone, Debug, Default)]
pub struct RulesRead {
    pub rules: Rules,
    /// Whether a `bolt.toml` was read and used.
    pub file: bool,
    /// Why the whole file was passed over.
    pub skipped: Option<String>,
    /// Each name passed over, with why.
    pub ignored: Vec<(String, String)>,
}

/// Reads `bolt.toml` in `top`. Never fails: what can't be used is in `skipped` or `ignored`.
#[must_use]
pub fn read(top: &Dir) -> RulesRead {
    let name = OsStr::new("bolt.toml");
    let skipped = |why| RulesRead {
        skipped: Some(why),
        ..RulesRead::default()
    };
    let shown = Path::new("bolt.toml");
    let bytes = match safe_fs::read_capped_in(top, name, shown, MAX_RULES_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return RulesRead::default(),
        Err(e) => return skipped(unusable(top, name, &e)),
    };
    match parse(&bytes) {
        Ok((rules, ignored)) => RulesRead {
            rules,
            file: true,
            skipped: None,
            ignored,
        },
        Err(why) => skipped(why),
    }
}

/// Why the file `name` couldn't be read, judged by what it is.
fn unusable(top: &Dir, name: &OsStr, e: &io::Error) -> String {
    match top.metadata(name).map(|m| (m.kind, m.size)) {
        Ok((EntryType::Symlink, _)) => "it is a link, which Bolt doesn't follow here".to_owned(),
        Ok((EntryType::Dir | EntryType::Other, _)) => {
            "it is a folder or special file, not a regular file".to_owned()
        }
        Ok((EntryType::File, size)) if size > MAX_RULES_FILE_BYTES => {
            "it is larger than 1 MiB".to_owned()
        }
        _ => format!("Bolt couldn't read it: {}", escape(&e.to_string(), false)),
    }
}

impl RulesRead {
    /// `rules_file_skipped` or `rules_ignored`, if anything was passed over.
    #[must_use]
    pub fn warnings(&self) -> Vec<Warning> {
        if let Some(why) = &self.skipped {
            let own = Rules::default();
            let message = format!(
                "Bolt passed over bolt.toml: {why}. Its own values apply instead: files of {} or \
                 more are stored in chunks, and a save asks first over {} or {} new files. Fix or \
                 remove the file to use its rules.",
                size_text(own.chunk_files_over),
                size_text(own.ask_over_bytes),
                own.ask_over_files
            );
            return vec![Warning::new(warning::RULES_FILE_SKIPPED, message)];
        }
        if self.ignored.is_empty() {
            return vec![];
        }
        let mut message = "Bolt passed over these parts of bolt.toml, which it can't use, and \
                           used its own values for them; the rest of the file counts:"
            .to_owned();
        for (name, why) in self.ignored.iter().take(NAMES_LISTED) {
            message.push_str(&format!("\n  {}: {why}", shown(name)));
        }
        let more = self.ignored.len().saturating_sub(NAMES_LISTED);
        if more > 0 {
            message.push_str(&format!("\n  and {more} more"));
        }
        message.push_str("\nFix or remove them, or update Bolt if a newer one wrote them.");
        vec![Warning::new(warning::RULES_IGNORED, message)]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::test_support::{HOSTILE, assert_hostile_text_escaped, symlink};
    use crate::warning;
    use std::fs;
    use std::path::Path;

    fn read_in(dir: &Path) -> RulesRead {
        read(&Dir::open_top(dir).unwrap())
    }

    fn kinds(read: &RulesRead) -> Vec<&'static str> {
        read.warnings().iter().map(|w| w.kind).collect()
    }

    #[test]
    fn sizes_read_in_both_families() {
        for (text, bytes) in [
            ("0 B", 0),
            ("7B", 7),
            ("3 KB", 3_000),
            ("3 KiB", 3_072),
            ("100 MB", 100_000_000),
            ("1 MiB", 1 << 20),
            ("5   GB", 5_000_000_000),
            ("1GiB", 1 << 30),
            ("2 TB", 2_000_000_000_000),
            ("2 TiB", 2 << 40),
            ("0010 KiB", 10 << 10),
        ] {
            assert_eq!(parse_size(text), Some(bytes), "{text}");
        }
        for text in [
            "",
            "MiB",
            "1",
            "1 ",
            "1 mib",
            "1 KIB",
            "1.5 MiB",
            "-1 B",
            "+1 B",
            " 1 B",
            "1 B ",
            "1\tB",
            "1 B B",
            "1 PiB",
            "\u{661} B",
        ] {
            assert_eq!(parse_size(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_size_that_does_not_fit_64_bits_is_no_size() {
        assert_eq!(parse_size("18446744073709551615 B"), Some(u64::MAX));
        assert_eq!(parse_size("18446744073709551616 B"), None);
        assert_eq!(parse_size("16777215 TiB"), Some(16_777_215 << 40));
        assert_eq!(parse_size("16777216 TiB"), None);
        assert_eq!(parse_size(&format!("{} B", "9".repeat(10_000))), None);
    }

    #[test]
    fn a_settings_size_is_written_in_the_largest_exact_unit_and_reads_back() {
        for (bytes, text) in [
            (1 << 20, "1 MiB"),
            (100_000_000, "100 MB"),
            (1 << 30, "1 GiB"),
            (5_000_000_000, "5 GB"),
            (1500, "1500 B"),
            (0, "0 B"),
            (1_024_000, "1000 KiB"),
            (137_438_953_472, "128 GiB"),
            (u64::MAX, "18446744073709551615 B"),
        ] {
            assert_eq!(size_text(bytes), text, "{bytes}");
            assert_eq!(parse_size(&size_text(bytes)), Some(bytes), "{bytes}");
        }
    }

    #[test]
    fn a_file_of_nested_brackets_is_passed_over_without_overflowing_the_stack() {
        for text in [
            "[".repeat(100_000),
            format!("a = {}", "[".repeat(100_000)),
            format!("a = {}1{}", "[".repeat(100_000), "]".repeat(100_000)),
            format!("a = {}", "{ b = ".repeat(100_000)),
            format!("[{}]", vec!["a"; 100_000].join(".")),
        ] {
            assert!(parse(text.as_bytes()).is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bolt.toml"), "[".repeat(100_000)).unwrap();
        let read = read_in(dir.path());
        assert!(read.skipped.is_some());
        assert_eq!(read.rules, Rules::default());
        assert_eq!(kinds(&read), [warning::RULES_FILE_SKIPPED]);
    }

    #[test]
    fn no_file_means_bolts_own_values_without_a_word() {
        let dir = tempfile::tempdir().unwrap();
        let read = read_in(dir.path());
        assert!(!read.file);
        assert_eq!(read.rules, Rules::default());
        assert_eq!(read.rules.chunk_files_over, 1 << 20);
        assert_eq!(read.rules.ask_over_bytes, 100_000_000);
        assert_eq!(read.rules.ask_over_files, 10_000);
        assert!(read.warnings().is_empty());
    }

    #[test]
    fn a_file_of_1_mib_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut text = b"chunk-files-over = \"2 MiB\"\n#".to_vec();
        text.resize(1 << 20, b'x');
        fs::write(dir.path().join("bolt.toml"), &text).unwrap();
        let read = read_in(dir.path());
        assert!(read.file);
        assert_eq!(read.rules.chunk_files_over, 2 << 20);
        assert!(read.warnings().is_empty());
    }

    #[test]
    fn a_link_a_folder_and_a_file_over_1_mib_are_each_passed_over() {
        let over = tempfile::tempdir().unwrap();
        let mut text = b"chunk-files-over = \"2 MiB\"\n#".to_vec();
        text.resize((1 << 20) + 1, b'x');
        fs::write(over.path().join("bolt.toml"), &text).unwrap();
        let folder = tempfile::tempdir().unwrap();
        fs::create_dir(folder.path().join("bolt.toml")).unwrap();
        let mut cases = vec![(over, "larger than 1 MiB"), (folder, "not a regular file")];
        let link = tempfile::tempdir().unwrap();
        fs::write(
            link.path().join("real.toml"),
            "chunk-files-over = \"2 MiB\"\n",
        )
        .unwrap();
        if symlink(
            &link.path().join("real.toml"),
            &link.path().join("bolt.toml"),
        ) {
            cases.push((link, "a link"));
        }
        for (dir, why) in &cases {
            let read = read_in(dir.path());
            assert!(!read.file, "{why}");
            assert_eq!(read.rules, Rules::default(), "{why}");
            let warnings = read.warnings();
            assert_eq!(warnings.len(), 1, "{why}");
            assert_eq!(warnings[0].kind, warning::RULES_FILE_SKIPPED);
            assert!(warnings[0].message.contains(why), "{}", warnings[0].message);
        }
    }

    #[test]
    fn the_file_is_never_read_through_a_link() {
        let outside = tempfile::tempdir().unwrap();
        let secret = "preset = \"the-secret-text\"\nzebra-secret = 1\n";
        fs::write(outside.path().join("secret.toml"), secret).unwrap();
        let dir = tempfile::tempdir().unwrap();
        if !symlink(
            &outside.path().join("secret.toml"),
            &dir.path().join("bolt.toml"),
        ) {
            return;
        }
        let read = read_in(dir.path());
        assert_eq!(read.rules, Rules::default());
        assert!(read.ignored.is_empty());
        let warnings = read.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(
            !warnings[0].message.contains("secret"),
            "{}",
            warnings[0].message
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_is_passed_over_without_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(dir.path().join("bolt.toml"))
            .status()
            .unwrap();
        assert!(made.success());
        let read = read_in(dir.path());
        assert_eq!(kinds(&read), [warning::RULES_FILE_SKIPPED]);
    }

    #[test]
    fn warnings_escape_what_the_file_says() {
        let hostile = HOSTILE
            .replace('\u{1b}', "\\u001b")
            .replace('\u{202e}', "\\u202e")
            .replace('\u{2028}', "\\u2028");
        let text = format!("\"{hostile}\" = 1\nchunk-files-over = \"{hostile}\"\n");
        let (rules, ignored) = parse(text.as_bytes()).unwrap();
        assert_eq!(rules, Rules::default());
        assert_eq!(ignored.len(), 2);
        let read = RulesRead {
            rules,
            file: true,
            skipped: None,
            ignored,
        };
        let warnings = read.warnings();
        assert_eq!(kinds(&read), [warning::RULES_IGNORED]);
        let message = &warnings[0].message;
        assert_hostile_text_escaped(message);
        assert_eq!(message.matches("a\\x1b[2Jb").count(), 2, "{message}");
        // What the parser says about a file that isn't TOML can hold the file's text too.
        let bad = format!("\"{hostile}\" = = 1\n");
        let why = parse(bad.as_bytes()).unwrap_err();
        let read = RulesRead {
            skipped: Some(why),
            ..RulesRead::default()
        };
        assert!(!read.warnings()[0].message.contains('\u{1b}'));
    }

    #[test]
    fn a_long_value_or_name_is_cut_and_many_names_are_not_all_listed() {
        let long = "x".repeat(10_000);
        let mut text = format!("chunk-files-over = \"{long}\"\n{long} = 1\n");
        for n in 0..1_000 {
            text.push_str(&format!("key{n} = 1\n"));
        }
        let (rules, ignored) = parse(text.as_bytes()).unwrap();
        assert_eq!(ignored.len(), 1_002);
        let read = RulesRead {
            rules,
            file: true,
            skipped: None,
            ignored,
        };
        let message = &read.warnings()[0].message;
        assert!(message.len() < 4_000, "{}", message.len());
        assert!(!message.contains(&"x".repeat(65)), "{message}");
        assert!(message.contains("and 982 more"), "{message}");
    }

    proptest::proptest! {
        #[test]
        fn what_is_kept_keeps_to_the_limits(
            text in "((chunk-files-over|ask-before-saving-over|preset|rule|files|lockable|x) \
                     ?= ?(\"[0-9 ]{0,12}[KMGT]?i?B?\"|-?[0-9]{1,20}|true|\\[\\]|\\{\\}|\\[\\{ ?files = \
                     \\[\"[a-z*.]{0,3}\"\\] ?\\}\\])\n|\\[\\[rule\\]\\]\n){0,8}"
        ) {
            if let Ok((rules, _)) = parse(text.as_bytes()) {
                let usable = |r: &Rule| !r.files.is_empty() && r.files.iter().all(|p| !p.is_empty());
                proptest::prop_assert!((CHUNK_FROM..=CHUNK_UP_TO).contains(&rules.chunk_files_over));
                proptest::prop_assert!(rules.rules.iter().all(usable));
            }
        }
    }

    /// The rules as saves use them.
    mod saves {
        use super::*;
        use crate::id::{Id, hash_content};
        use crate::pack::{ObjectKind, PackReader};
        use crate::repo::Repository;
        use crate::save::Person;
        use crate::snapshot::{LargeSave, SaveError, SaveOutcome, SaveRequest, save_folder};
        use crate::test_support::noise;
        use std::collections::HashSet;

        fn project() -> (tempfile::TempDir, Repository) {
            let dir = tempfile::tempdir().unwrap();
            let who = Person {
                name: "Ada".into(),
                email: "ada@example.com".into(),
                time_ms: 1_700_000_000_000,
                tz_offset_min: 0,
            };
            let repo = Repository::init(dir.path(), &who).unwrap();
            (dir, repo)
        }

        /// A save whose question about adding much is answered `answer`, and what it asked.
        fn save(repo: &mut Repository, answer: bool) -> (Result<SaveOutcome, SaveError>, bool) {
            let who = Person {
                name: "Ada".into(),
                email: "ada@example.com".into(),
                time_ms: 1_700_000_000_000,
                tz_offset_min: 0,
            };
            let req = SaveRequest {
                message: Some("Save"),
                who: &who,
                command: "save",
                accept_crlf: false,
                git_excludes: None,
                new_line: None,
                only: None,
                amend: false,
            };
            let mut asked = false;
            let result = save_folder(repo, &req, &mut |_| {}, &mut |_: &LargeSave| {
                asked = true;
                answer
            });
            (result, asked)
        }

        /// The files the store holds as chunk lists.
        fn chunk_lists(repo: &Repository) -> HashSet<Id> {
            let packs = fs::read_dir(repo.packs_dir()).unwrap();
            let paths = packs.map(|p| p.unwrap().path());
            let paths = paths.filter(|p| p.extension().is_some_and(|e| e == "pack"));
            let mut found = HashSet::new();
            for path in paths {
                for entry in PackReader::open(&path).unwrap().entries() {
                    let entry = entry.unwrap();
                    if entry.kind == ObjectKind::ChunkList {
                        found.insert(entry.id);
                    }
                }
            }
            found
        }

        /// The kinds of a save's warnings about `bolt.toml`.
        fn rules_warnings(outcome: &SaveOutcome) -> Vec<&'static str> {
            let (SaveOutcome::Saved { warnings, .. }
            | SaveOutcome::NothingChanged { warnings, .. }) = outcome;
            let about = [warning::RULES_FILE_SKIPPED, warning::RULES_IGNORED];
            let kinds = warnings.iter().map(|w| w.kind);
            kinds.filter(|k| about.contains(k)).collect()
        }

        #[test]
        fn chunk_files_over_decides_which_files_are_cut() {
            let (two, four, five) = (noise(1, 2 << 20), noise(2, 4 << 20), noise(3, 5 << 20));
            for (rules, two_cut) in [
                (Some("chunk-files-over = \"4 MiB\"\n"), false),
                (None, true),
            ] {
                let (dir, mut repo) = project();
                if let Some(rules) = rules {
                    fs::write(dir.path().join("bolt.toml"), rules).unwrap();
                }
                for (name, bytes) in [("two", &two), ("four", &four), ("five", &five)] {
                    fs::write(dir.path().join(name), bytes).unwrap();
                }
                let (saved, _) = save(&mut repo, true);
                let SaveOutcome::Saved { save, warnings, .. } = saved.unwrap() else {
                    panic!("nothing was saved");
                };
                assert!(warnings.is_empty(), "{warnings:?}");
                let lists = chunk_lists(&repo);
                assert_eq!(lists.contains(&hash_content(&two)), two_cut, "{rules:?}");
                // From that size on, both ends counted.
                assert!(lists.contains(&hash_content(&four)), "{rules:?}");
                assert!(lists.contains(&hash_content(&five)), "{rules:?}");
                // How a file is stored never changes its ID.
                let root = repo.store().listing(&save.root).unwrap();
                for (name, bytes) in [("two", &two), ("four", &four), ("five", &five)] {
                    assert_eq!(root.get(name).unwrap().id, hash_content(bytes), "{name}");
                }
            }
        }

        #[test]
        fn the_question_uses_the_files_numbers() {
            let (dir, mut repo) = project();
            let rules = "ask-before-saving-over = \"1 KiB\"\n";
            fs::write(dir.path().join("bolt.toml"), rules).unwrap();
            fs::write(dir.path().join("a"), [7; 2048]).unwrap();
            let (result, asked) = save(&mut repo, false);
            assert!(asked);
            assert!(matches!(result, Err(SaveError::LargeSave(_))), "{result:?}");

            let (dir, mut repo) = project();
            let rules = "ask-before-saving-files-over = 0\n";
            fs::write(dir.path().join("bolt.toml"), rules).unwrap();
            let (result, asked) = save(&mut repo, false);
            assert!(asked, "the rules file is a new file");
            assert!(matches!(result, Err(SaveError::LargeSave(_))), "{result:?}");

            // Over Bolt's own 10,000 files, which these rules raise.
            let (dir, mut repo) = project();
            let rules =
                "ask-before-saving-over = \"1 TiB\"\nask-before-saving-files-over = 1000000\n";
            fs::write(dir.path().join("bolt.toml"), rules).unwrap();
            for i in 0..10_001 {
                let folder = dir.path().join(format!("many/{}", i % 10));
                fs::create_dir_all(&folder).unwrap();
                fs::write(folder.join(format!("{i}.txt")), b"x").unwrap();
            }
            let (result, asked) = save(&mut repo, false);
            assert!(!asked);
            assert!(
                matches!(result, Ok(SaveOutcome::Saved { .. })),
                "{result:?}"
            );
        }

        #[test]
        fn a_rules_file_bolt_cannot_use_never_stops_a_save() {
            let mut over = b"preset = \"big\"\n#".to_vec();
            over.resize((1 << 20) + 1, b'x');
            let cases: [(&str, Option<&[u8]>, &str); 5] = [
                (
                    "not TOML",
                    Some(b"this is not = = toml\n"),
                    warning::RULES_FILE_SKIPPED,
                ),
                ("over 1 MiB", Some(&over), warning::RULES_FILE_SKIPPED),
                ("a folder", None, warning::RULES_FILE_SKIPPED),
                (
                    "not UTF-8",
                    Some(b"preset = \"\xff\"\n"),
                    warning::RULES_FILE_SKIPPED,
                ),
                (
                    "bad keys",
                    Some(b"chunk-files-over = \"1 KiB\"\nask-before-saving-over = 0\nzebra = 1\n"),
                    warning::RULES_IGNORED,
                ),
            ];
            for (case, text, kind) in cases {
                let (dir, mut repo) = project();
                match text {
                    Some(text) => fs::write(dir.path().join("bolt.toml"), text).unwrap(),
                    None => fs::create_dir(dir.path().join("bolt.toml")).unwrap(),
                }
                fs::write(dir.path().join("a.txt"), "a").unwrap();
                let (result, asked) = save(&mut repo, false);
                assert!(!asked, "{case}");
                let outcome = result.unwrap_or_else(|e| panic!("{case}: {e}"));
                assert!(matches!(outcome, SaveOutcome::Saved { .. }), "{case}");
                assert_eq!(rules_warnings(&outcome), [kind], "{case}");
                assert_eq!(repo.rules().rules.chunk_files_over, 1 << 20, "{case}");
            }
            let (dir, mut repo) = project();
            fs::write(dir.path().join("real.toml"), "zebra = 1\n").unwrap();
            if symlink(&dir.path().join("real.toml"), &dir.path().join("bolt.toml")) {
                let (result, _) = save(&mut repo, false);
                let outcome = result.unwrap();
                assert_eq!(rules_warnings(&outcome), [warning::RULES_FILE_SKIPPED]);
            }
        }

        #[test]
        fn the_rules_are_read_from_the_folder_not_the_save() {
            let (dir, mut repo) = project();
            let rules = dir.path().join("bolt.toml");
            fs::write(&rules, "chunk-files-over = \"4 MiB\"\n").unwrap();
            assert_eq!(repo.rules().rules, Rules::default(), "nothing read yet");
            assert_eq!(repo.read_rules().rules.chunk_files_over, 4 << 20);
            save(&mut repo, true).0.unwrap();
            assert_eq!(repo.rules().rules.chunk_files_over, 4 << 20);
            // Changed, not saved: the folder's copy is the one that counts.
            fs::write(&rules, "chunk-files-over = \"2 MiB\"\n").unwrap();
            let three = noise(4, 3 << 20);
            fs::write(dir.path().join("three"), &three).unwrap();
            save(&mut repo, true).0.unwrap();
            assert!(chunk_lists(&repo).contains(&hash_content(&three)));
            // Gone from the folder, it no longer counts, though the last save holds it.
            fs::remove_file(&rules).unwrap();
            let one_and_a_half = noise(5, 3 << 19);
            fs::write(dir.path().join("one-and-a-half"), &one_and_a_half).unwrap();
            let (result, _) = save(&mut repo, true);
            assert!(rules_warnings(&result.unwrap()).is_empty());
            assert_eq!(repo.rules().rules, Rules::default());
            assert!(chunk_lists(&repo).contains(&hash_content(&one_and_a_half)));
        }
    }
}
