//! The presets: what Code and Game write in `opal.toml` and `.opalignore`
//! (docs/storage-format.md section 12), and the look at a folder that suggests one.

use std::ffi::OsString;
use std::io;
use std::ops::Range;

use opal_platform::dir::{Dir, DirEntry, EntryType};

/// A starter set of project rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Code,
    Game,
    Custom,
}

impl Preset {
    /// The name `--preset` takes and `opal.toml` records.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Preset::Code => "code",
            Preset::Game => "game",
            Preset::Custom => "custom",
        }
    }

    /// The preset with this exact name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Preset> {
        [Preset::Code, Preset::Game, Preset::Custom]
            .into_iter()
            .find(|p| p.name() == name)
    }
}

/// A game engine whose project files were found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Engine {
    Unity,
    Unreal,
    Godot,
}

impl Engine {
    pub const ALL: [Engine; 3] = [Engine::Unity, Engine::Unreal, Engine::Godot];

    /// The name the `.opalignore` block's first line gives it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Engine::Unity => "unity",
            Engine::Unreal => "unreal",
            Engine::Godot => "godot",
        }
    }

    fn rules(self) -> &'static str {
        match self {
            Engine::Unity => include_str!("presets/unity.toml"),
            Engine::Unreal => include_str!("presets/unreal.toml"),
            Engine::Godot => include_str!("presets/godot.toml"),
        }
    }

    fn ignore_lines(self) -> &'static str {
        match self {
            Engine::Unity => include_str!("presets/unity.txt"),
            Engine::Unreal => include_str!("presets/unreal.txt"),
            Engine::Godot => include_str!("presets/godot.txt"),
        }
    }

    /// The folders at the top where it keeps what it makes again, which sizes leave out.
    fn caches(self) -> &'static [&'static str] {
        match self {
            Engine::Unity => &[
                "Library",
                "Temp",
                "Logs",
                "Obj",
                "Build",
                "Builds",
                "UserSettings",
            ],
            Engine::Unreal => &["Binaries", "Intermediate", "Saved", "DerivedDataCache"],
            Engine::Godot => &[".godot", ".import"],
        }
    }
}

/// A programming language whose project file was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Language {
    Node,
    Python,
    Rust,
    Go,
    Dart,
    Dotnet,
    Maven,
    Gradle,
}

impl Language {
    pub const ALL: [Language; 8] = [
        Language::Node,
        Language::Python,
        Language::Rust,
        Language::Go,
        Language::Dart,
        Language::Dotnet,
        Language::Maven,
        Language::Gradle,
    ];

    /// The name the `.opalignore` block's first line gives it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Language::Node => "node",
            Language::Python => "python",
            Language::Rust => "rust",
            Language::Go => "go",
            Language::Dart => "dart",
            Language::Dotnet => "dotnet",
            Language::Maven => "maven",
            Language::Gradle => "gradle",
        }
    }

    fn ignore_lines(self) -> &'static str {
        match self {
            Language::Node => include_str!("presets/node.txt"),
            Language::Python => include_str!("presets/python.txt"),
            Language::Rust => include_str!("presets/rust.txt"),
            Language::Go => include_str!("presets/go.txt"),
            Language::Dart => include_str!("presets/dart.txt"),
            Language::Dotnet => include_str!("presets/dotnet.txt"),
            Language::Maven => include_str!("presets/maven.txt"),
            Language::Gradle => include_str!("presets/gradle.txt"),
        }
    }
}

/// What a folder's top-level names show: each engine and language once, in the enums' order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub engines: Vec<Engine>,
    pub languages: Vec<Language>,
}

/// How many entries a look meets before it stops counting.
pub const LOOK_AT_MOST: u64 = 20_000;
/// How deep below the top a look goes.
const DEEPEST: usize = 128;

/// What a look at a folder found.
#[derive(Clone, Debug)]
pub struct Looked {
    pub found: Found,
    /// The size of the files below the top, leaving out `.git`, `.opal` and the engines' caches.
    pub bytes: u64,
    /// False when the look stopped early or passed over a folder, so `bytes` is a floor.
    pub counted_all: bool,
    /// Whether the top folder holds an entry named `opal.toml`, of any kind.
    pub has_rules_file: bool,
}

/// Names in the top folder, and the sizes of the files below it. Reads no file's contents and
/// never goes through a link. Project files' names are compared exactly as written, whatever the
/// drive; an engine's cache folders without regard to ASCII letter case.
pub fn look(top: &Dir) -> io::Result<Looked> {
    let entries = top.entries()?;
    let found = found_in(top, &entries);
    let has_rules_file = entries.iter().any(|e| e.name == "opal.toml");
    let left_out = |e: &DirEntry| {
        let name = e.name.as_encoded_bytes();
        name == b".git"
            || name == b".opal"
            || (e.kind == EntryType::Dir
                && found.engines.iter().any(|engine| {
                    engine
                        .caches()
                        .iter()
                        .any(|c| name.eq_ignore_ascii_case(c.as_bytes()))
                }))
    };
    let mut tally = Tally {
        met: 0,
        bytes: 0,
        counted_all: true,
    };
    let mut top_folders = tally.add(top, &entries, left_out).unwrap_or_default();
    // Each open folder with the names of its folders still to go into: one handle a level.
    let mut stack: Vec<(Dir, Vec<OsString>)> = Vec::new();
    loop {
        let depth = stack.len() + 1;
        let (parent, waiting) = match stack.last_mut() {
            Some((dir, names)) => (&*dir, names),
            None => (top, &mut top_folders),
        };
        let Some(name) = waiting.pop() else {
            if stack.pop().is_none() {
                break;
            }
            continue;
        };
        if depth > DEEPEST {
            tally.counted_all = false;
            continue;
        }
        // A folder that can't be opened or listed is passed over, never followed if a link.
        let Some((dir, entries)) = parent
            .open_dir(&name)
            .ok()
            .and_then(|dir| dir.entries().ok().map(|entries| (dir, entries)))
        else {
            tally.counted_all = false;
            continue;
        };
        match tally.add(&dir, &entries, |_| false) {
            Some(folders) => stack.push((dir, folders)),
            None => break,
        }
    }
    Ok(Looked {
        found,
        bytes: tally.bytes,
        counted_all: tally.counted_all,
        has_rules_file,
    })
}

/// The engines and languages the top folder's names show.
fn found_in(top: &Dir, entries: &[DirEntry]) -> Found {
    let is = |name: &str, kind: EntryType| entries.iter().any(|e| e.name == name && e.kind == kind);
    let mut found = Found::default();
    let unity_version = || {
        top.open_dir("ProjectSettings".as_ref())
            .and_then(|d| d.metadata("ProjectVersion.txt".as_ref()))
            .is_ok_and(|m| m.kind == EntryType::File)
    };
    if is("Assets", EntryType::Dir) && is("ProjectSettings", EntryType::Dir) && unity_version() {
        found.engines.push(Engine::Unity);
    }
    for e in entries.iter().filter(|e| e.kind == EntryType::File) {
        let name = e.name.as_encoded_bytes();
        if name.ends_with(b".uproject") {
            found.engines.push(Engine::Unreal);
        }
        if name == b"project.godot" {
            found.engines.push(Engine::Godot);
        }
        if let Some(language) = language_of(name) {
            found.languages.push(language);
        }
    }
    found.engines.sort_unstable();
    found.engines.dedup();
    found.languages.sort_unstable();
    found.languages.dedup();
    found
}

fn language_of(name: &[u8]) -> Option<Language> {
    Some(match name {
        b"package.json" => Language::Node,
        b"pyproject.toml" | b"requirements.txt" | b"setup.py" => Language::Python,
        b"Cargo.toml" => Language::Rust,
        b"go.mod" => Language::Go,
        b"pubspec.yaml" => Language::Dart,
        b"pom.xml" => Language::Maven,
        b"build.gradle" | b"build.gradle.kts" => Language::Gradle,
        _ if name.ends_with(b".sln") || name.ends_with(b".csproj") => Language::Dotnet,
        _ => return None,
    })
}

struct Tally {
    met: u64,
    bytes: u64,
    counted_all: bool,
}

impl Tally {
    /// Counts the entries of `dir` and adds up its files' sizes, and gives the names of its
    /// folders to go into; `None` once the look has met all it may.
    fn add(
        &mut self,
        dir: &Dir,
        entries: &[DirEntry],
        left_out: impl Fn(&DirEntry) -> bool,
    ) -> Option<Vec<OsString>> {
        let mut folders = Vec::new();
        for e in entries {
            if self.met == LOOK_AT_MOST {
                self.counted_all = false;
                return None;
            }
            self.met += 1;
            if left_out(e) {
                continue;
            }
            match e.kind {
                EntryType::Dir => folders.push(e.name.clone()),
                EntryType::File => match dir.metadata(&e.name) {
                    Ok(meta) if meta.kind == EntryType::File => {
                        self.bytes = self.bytes.saturating_add(meta.size);
                    }
                    Ok(_) => {}
                    Err(_) => self.counted_all = false,
                },
                EntryType::Symlink | EntryType::Other => {}
            }
        }
        Some(folders)
    }
}

/// Game when an engine was found, else Code when a language was, else Custom.
#[must_use]
pub fn suggested(found: &Found) -> Preset {
    if !found.engines.is_empty() {
        Preset::Game
    } else if !found.languages.is_empty() {
        Preset::Code
    } else {
        Preset::Custom
    }
}

/// The `opal.toml` a preset writes; `None` for Custom, which writes none.
#[must_use]
pub fn opal_toml(preset: Preset, found: &Found) -> Option<String> {
    match preset {
        Preset::Code => Some(include_str!("presets/code.toml").to_owned()),
        Preset::Game => {
            let mut text = include_str!("presets/game.toml").to_owned();
            for engine in Engine::ALL
                .into_iter()
                .filter(|e| found.engines.contains(e))
            {
                text.push_str(engine.rules());
            }
            Some(text)
        }
        Preset::Custom => None,
    }
}

const START_MARK: &[u8] = b"# >>> Opalro preset";
const END_MARK: &[u8] = b"# <<< Opalro preset";
const BOM: &[u8] = b"\xef\xbb\xbf";

/// The block of lines a preset writes in `.opalignore`, marks included; `None` for Custom.
#[must_use]
pub fn opalignore_block(preset: Preset, found: &Found) -> Option<String> {
    let (names, lines): (Vec<&str>, String) = match preset {
        Preset::Code => {
            let languages = Language::ALL
                .into_iter()
                .filter(|l| found.languages.contains(l));
            let names = languages.clone().map(Language::name).collect();
            let mut lines: String = languages.map(Language::ignore_lines).collect();
            lines.push_str(include_str!("presets/secrets.txt"));
            (names, lines)
        }
        Preset::Game => {
            let engines = Engine::ALL
                .into_iter()
                .filter(|e| found.engines.contains(e));
            (
                engines.clone().map(Engine::name).collect(),
                engines.map(Engine::ignore_lines).collect(),
            )
        }
        Preset::Custom => return None,
    };
    let found = if names.is_empty() {
        String::new()
    } else {
        format!(" ({})", names.join(", "))
    };
    Some(format!(
        "# >>> Opalro preset: {}{found}, version 1. `opal setup` rewrites the lines down to the end \
         mark.\n{lines}# <<< Opalro preset\n",
        preset.name()
    ))
}

/// Where the first block is: its bytes, from the start of its start line to the end of its end
/// line, and how many lines lie between the two.
struct Block {
    bytes: Range<usize>,
    lines: usize,
}

/// The first block in `text`, a `.opalignore` without its byte order mark. A start line with no
/// end line after it is an ordinary comment.
fn first_block(text: &[u8]) -> Option<Block> {
    let mut start = None;
    let mut lines = 0;
    let mut at = 0;
    for line in text.split_inclusive(|&b| b == b'\n') {
        let next = at + line.len();
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        match start {
            None if line.starts_with(START_MARK) => start = Some(at),
            None => {}
            Some(from) if line == END_MARK => {
                return Some(Block {
                    bytes: from..next,
                    lines,
                });
            }
            Some(_) => lines += 1,
        }
        at = next;
    }
    None
}

/// A `.opalignore` with the first block replaced by `block`, added at the top, or removed. `None`:
/// no file is left.
#[must_use]
pub fn edit_opalignore(before: Option<&[u8]>, block: Option<&str>) -> Option<Vec<u8>> {
    let Some(before) = before else {
        return block.map(|b| b.as_bytes().to_vec());
    };
    // A byte order mark always stays first.
    let bom = if before.starts_with(BOM) {
        BOM.len()
    } else {
        0
    };
    let (head, text) = before.split_at(bom);
    let found = first_block(text);
    let mut after = head.to_vec();
    match (found, block) {
        (Some(found), block) => {
            after.extend_from_slice(&text[..found.bytes.start]);
            after.extend_from_slice(block.unwrap_or_default().as_bytes());
            after.extend_from_slice(&text[found.bytes.end..]);
            let blank = |line: &[u8]| line.is_empty() || line == b"\r";
            if block.is_none() && after[bom..].split(|&b| b == b'\n').all(blank) {
                return None;
            }
        }
        (None, Some(block)) => {
            after.extend_from_slice(block.as_bytes());
            if !text.is_empty() {
                after.push(b'\n');
            }
            after.extend_from_slice(text);
        }
        (None, None) => after.extend_from_slice(text),
    }
    Some(after)
}

/// How many lines the first block holds between its marks; `None` when there is no block.
#[must_use]
pub fn block_lines(opalignore: &[u8]) -> Option<usize> {
    let text = opalignore.strip_prefix(BOM).unwrap_or(opalignore);
    first_block(text).map(|b| b.lines)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::drive::Drive;
    use crate::ignore::{Decision, Matcher, Rules};
    use crate::rules::{self, Rule};
    use crate::test_support::symlink;
    use std::fs;
    use std::path::Path;

    fn look_in(dir: &Path) -> Looked {
        look(&Dir::open_top(dir).unwrap()).unwrap()
    }

    fn every_found() -> Vec<Found> {
        let mut all = vec![
            Found::default(),
            Found {
                engines: Engine::ALL.to_vec(),
                languages: Language::ALL.to_vec(),
            },
        ];
        all.extend(Engine::ALL.map(|e| Found {
            engines: vec![e],
            languages: vec![],
        }));
        all.extend(Language::ALL.map(|l| Found {
            engines: vec![],
            languages: vec![l],
        }));
        all
    }

    fn rule(files: &[&str], lockable: Option<bool>, conflict_markers: Option<bool>) -> Rule {
        Rule {
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            lockable,
            conflict_markers,
        }
    }

    #[test]
    fn what_a_preset_writes_reads_back_as_its_rules() {
        assert_eq!(opal_toml(Preset::Custom, &every_found()[1]), None);
        for found in every_found() {
            let (code, ignored) =
                rules::parse(opal_toml(Preset::Code, &found).unwrap().as_bytes()).unwrap();
            assert!(ignored.is_empty(), "{ignored:?}");
            assert_eq!(code.preset.as_deref(), Some("code"));
            assert_eq!(code.chunk_files_over, 1 << 20);
            assert_eq!(code.ask_over_bytes, 100_000_000);
            assert_eq!(code.ask_over_files, 10_000);
            assert!(code.rules.is_empty());

            let (game, ignored) =
                rules::parse(opal_toml(Preset::Game, &found).unwrap().as_bytes()).unwrap();
            assert!(ignored.is_empty(), "{ignored:?}");
            assert_eq!(game.preset.as_deref(), Some("game"));
            assert_eq!(game.chunk_files_over, 1 << 20);
            assert_eq!(game.ask_over_bytes, 1 << 30);
            assert_eq!(game.ask_over_files, 10_000);
            let mut expected = vec![rule(
                &[
                    "*.psd", "*.png", "*.tga", "*.tif", "*.tiff", "*.exr", "*.jpg", "*.jpeg",
                    "*.fbx", "*.blend", "*.max", "*.mb", "*.wav", "*.ogg", "*.mp3", "*.mp4",
                    "*.mov",
                ],
                Some(true),
                None,
            )];
            for engine in &found.engines {
                match engine {
                    Engine::Unity => expected.extend([
                        rule(&["*.unity", "*.prefab"], Some(true), Some(false)),
                        rule(&["*.meta"], None, Some(false)),
                    ]),
                    Engine::Unreal => {
                        expected.push(rule(&["*.uasset", "*.umap"], Some(true), None))
                    }
                    Engine::Godot => expected.push(rule(&["*.scn", "*.res"], Some(true), None)),
                }
            }
            assert_eq!(game.rules, expected, "{found:?}");
        }
    }

    /// A path the pattern matches, made by filling in its wildcards, and whether it is a folder.
    fn example(pattern: &str) -> (String, bool) {
        let pattern = pattern.strip_prefix('!').unwrap_or(pattern);
        let folder = pattern.ends_with('/');
        let pattern = pattern.trim_end_matches('/').trim_start_matches('/');
        let mut path = String::new();
        let mut chars = pattern
            .replace("**", "a")
            .chars()
            .collect::<Vec<_>>()
            .into_iter();
        while let Some(c) = chars.next() {
            match c {
                '*' | '?' => path.push('x'),
                '[' => {
                    path.push(chars.next().unwrap());
                    chars.by_ref().find(|&c| c == ']').unwrap();
                }
                c => path.push(c),
            }
        }
        (path, folder)
    }

    fn lines_of(block: &str) -> impl Iterator<Item = &str> {
        block.lines().filter(|l| !l.starts_with('#'))
    }

    #[test]
    fn every_ignore_line_is_one_gits_matcher_can_use() {
        let drive = Drive {
            ignores_case: false,
            composes_unicode: false,
        };
        let full = &every_found()[1];
        for preset in [Preset::Code, Preset::Game] {
            let block = opalignore_block(preset, full).unwrap();
            assert!(lines_of(&block).count() > 0);
            for line in lines_of(&block) {
                let mut m = Matcher::new(drive);
                m.enter("", vec![Rules::parse(line.as_bytes(), "", false)]);
                let (path, folder) = example(line);
                let wanted = if line.starts_with('!') {
                    Decision::Kept
                } else {
                    Decision::Ignored
                };
                assert_eq!(m.decide(&path, folder), wanted, "{line} on {path}");
            }
        }
    }

    #[test]
    fn no_preset_line_keeps_out_an_asset_by_its_extension() {
        let full = &every_found()[1];
        for ignores_case in [false, true] {
            let drive = Drive {
                ignores_case,
                composes_unicode: false,
            };
            for preset in [Preset::Code, Preset::Game] {
                let block = opalignore_block(preset, full).unwrap();
                let mut m = Matcher::new(drive);
                m.enter("", vec![Rules::parse(block.as_bytes(), "", false)]);
                for ext in ["obj", "exe", "dll", "png", "log", "tmp"] {
                    for path in [format!("hero.{ext}"), format!("Art/Models/hero.{ext}")] {
                        assert_eq!(m.decide(&path, false), Decision::NoRule, "{path}");
                    }
                }
            }
        }
    }

    #[test]
    fn unity_unreal_and_godot_are_each_found_by_their_files() {
        let unity = |dir: &Path| {
            fs::create_dir(dir.join("Assets")).unwrap();
            fs::create_dir(dir.join("ProjectSettings")).unwrap();
            fs::write(
                dir.join("ProjectSettings/ProjectVersion.txt"),
                "m_EditorVersion",
            )
            .unwrap();
        };
        let dir = tempfile::tempdir().unwrap();
        unity(dir.path());
        assert_eq!(look_in(dir.path()).found.engines, [Engine::Unity]);
        // Without its Assets folder, it isn't Unity's.
        fs::remove_dir(dir.path().join("Assets")).unwrap();
        assert_eq!(look_in(dir.path()).found, Found::default());

        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Shooter.uproject"), "{}").unwrap();
        assert_eq!(look_in(dir.path()).found.engines, [Engine::Unreal]);

        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("project.godot"), "").unwrap();
        assert_eq!(look_in(dir.path()).found.engines, [Engine::Godot]);

        // Listed in any order, each comes once, in the enum's order.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("project.godot"), "").unwrap();
        fs::write(dir.path().join("A.uproject"), "").unwrap();
        fs::write(dir.path().join("B.uproject"), "").unwrap();
        unity(dir.path());
        assert_eq!(look_in(dir.path()).found.engines, Engine::ALL);
    }

    #[test]
    fn each_language_is_found_by_its_manifest() {
        use Language::*;
        let manifests = [
            ("package.json", Node),
            ("pyproject.toml", Python),
            ("requirements.txt", Python),
            ("setup.py", Python),
            ("Cargo.toml", Rust),
            ("go.mod", Go),
            ("pubspec.yaml", Dart),
            ("App.sln", Dotnet),
            ("App.csproj", Dotnet),
            ("pom.xml", Maven),
            ("build.gradle", Gradle),
            ("build.gradle.kts", Gradle),
        ];
        for (name, language) in manifests {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join(name), "").unwrap();
            let looked = look_in(dir.path());
            assert_eq!(looked.found.languages, [language], "{name}");
            assert!(looked.found.engines.is_empty());
        }
        let dir = tempfile::tempdir().unwrap();
        for (name, _) in manifests.iter().rev() {
            fs::write(dir.path().join(name), "").unwrap();
        }
        assert_eq!(look_in(dir.path()).found.languages, Language::ALL);
        // Names are compared exactly, and only at the top.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("PACKAGE.JSON"), "").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/Cargo.toml"), "").unwrap();
        assert_eq!(look_in(dir.path()).found, Found::default());
    }

    #[test]
    fn a_folder_named_like_a_project_file_is_not_one() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["project.godot", "Game.uproject", "package.json", "App.sln"] {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        // Unity's version file must be a file too.
        fs::create_dir(dir.path().join("Assets")).unwrap();
        fs::create_dir_all(dir.path().join("ProjectSettings/ProjectVersion.txt")).unwrap();
        assert_eq!(look_in(dir.path()).found, Found::default());
    }

    #[test]
    fn sizes_leave_out_git_opal_and_the_engines_caches() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        let write = |path: &str, len: usize| {
            let path = top.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, vec![b'x'; len]).unwrap();
        };
        write("ProjectSettings/ProjectVersion.txt", 3);
        write("Assets/Hero.png", 10);
        write("Assets/Deep/er/still.wav", 20);
        write(".git/objects/pack", 1000);
        write(".opal/packs/pack", 1000);
        write("Library/cache", 1000);
        write("temp/cache", 1000);
        write(".godot/cache", 400);
        assert!(!look_in(top).has_rules_file);
        let looked = look_in(top);
        assert_eq!(looked.found.engines, [Engine::Unity]);
        assert_eq!(looked.bytes, 3 + 10 + 20 + 400);
        assert!(looked.counted_all);

        // Without the engine, its folders are ordinary ones.
        fs::remove_file(top.join("ProjectSettings/ProjectVersion.txt")).unwrap();
        assert_eq!(look_in(top).bytes, 10 + 20 + 2000 + 400);

        // An opal.toml of any kind is seen.
        fs::create_dir(top.join("opal.toml")).unwrap();
        assert!(look_in(top).has_rules_file);
    }

    #[test]
    fn looking_stops_after_20_000_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("many")).unwrap();
        for i in 1..LOOK_AT_MOST {
            fs::write(dir.path().join(format!("many/{i}")), "").unwrap();
        }
        // The folder and the files in it make exactly the limit.
        assert!(look_in(dir.path()).counted_all);
        fs::write(dir.path().join("one-more"), "x").unwrap();
        assert!(!look_in(dir.path()).counted_all);
    }

    #[test]
    fn looking_never_goes_through_a_link() {
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(outside.path().join("big")).unwrap();
        fs::write(outside.path().join("big/file"), vec![0; 5000]).unwrap();
        fs::write(outside.path().join("project.godot"), "").unwrap();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("own"), "abc").unwrap();
        if !symlink(&outside.path().join("big"), &dir.path().join("big"))
            || !symlink(
                &outside.path().join("big/file"),
                &dir.path().join("file-link"),
            )
            || !symlink(
                &outside.path().join("project.godot"),
                &dir.path().join("project.godot"),
            )
        {
            return;
        }
        let looked = look_in(dir.path());
        assert_eq!(looked.bytes, 3);
        assert!(looked.counted_all);
        assert_eq!(looked.found, Found::default());
    }

    #[test]
    fn the_suggestion_is_game_then_code_then_custom() {
        let both = &every_found()[1];
        assert_eq!(suggested(both), Preset::Game);
        let code = Found {
            engines: vec![],
            languages: vec![Language::Go],
        };
        assert_eq!(suggested(&code), Preset::Code);
        assert_eq!(suggested(&Found::default()), Preset::Custom);
        for p in [Preset::Code, Preset::Game, Preset::Custom] {
            assert_eq!(Preset::from_name(p.name()), Some(p));
        }
        assert_eq!(Preset::from_name("Code"), None);
    }

    #[test]
    fn block_lines_counts_between_the_marks() {
        let full = &every_found()[1];
        let block = opalignore_block(Preset::Game, full).unwrap();
        let file = format!("mine/\n{block}more/\n");
        assert_eq!(
            block_lines(file.as_bytes()),
            Some(block.lines().count() - 2)
        );
        assert_eq!(
            block_lines(b"# >>> Opalro preset: code, version 1.\nx\n"),
            None
        );
        assert_eq!(
            block_lines(b"\xef\xbb\xbf# >>> Opalro preset\r\n# <<< Opalro preset\r\n"),
            Some(0)
        );
    }
}
