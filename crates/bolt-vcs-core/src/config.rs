//! Settings: who the user is (from the environment or the user's own `config.toml`) and the
//! `safe-folders` they trust. A project's own `.bolt/config.toml` is never asked: a shared zip
//! carries it, so a stranger could make the user's saves claim to be someone else (decision 025).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::safe_fs;
use crate::save::Person;
use crate::text::escape;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

/// A settings file's path and what is wrong with it can hold text from outside Bolt (a project's
/// `.bolt/config.toml` sits in a folder with any name), so they are shown escaped.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Bolt needs your name and email to record who made each save.")]
    MissingIdentity { path: Option<PathBuf> },
    #[error(
        "{} is not a valid Bolt settings file: {}",
        escape(&path.display().to_string(), false),
        escape(detail, false)
    )]
    Invalid { path: PathBuf, detail: String },
    /// A `safe-folders` line below a `[section]` belongs to that section, so Bolt never sees it.
    #[error(
        "safe-folders must be at the top of {}, before any [section]",
        escape(&path.display().to_string(), false)
    )]
    SafeFoldersNotAtTop { path: PathBuf },
}

const SNIPPET: &str = "[user]\nname = \"Your Name\"\nemail = \"you@example.com\"";

impl ConfigError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            ConfigError::MissingIdentity { path: Some(path) } => Some(format!(
                "Add this to {}:\n{SNIPPET}",
                escape(&path.display().to_string(), false)
            )),
            ConfigError::MissingIdentity { path: None } => Some(format!(
                "Set BOLT_CONFIG_DIR to a folder and add this to config.toml there:\n{SNIPPET}"
            )),
            ConfigError::Invalid { .. } => None,
            ConfigError::SafeFoldersNotAtTop { .. } => {
                Some("Move that line above [user] and every other [section].".to_owned())
            }
        }
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    user: Option<User>,
    /// An identity for one folder; only the user's own file is asked for these.
    #[serde(default)]
    project: Vec<ProjectEntry>,
    /// Folders to use although another account owns them; only the user's own file is asked.
    #[serde(default, rename = "safe-folders")]
    safe_folders: Vec<String>,
    /// Set after reading: `safe-folders` was also written below a `[section]`, where it belongs to
    /// that section and is no list.
    #[serde(skip)]
    misplaced_safe_folders: bool,
}

#[derive(Deserialize)]
struct User {
    name: Option<String>,
    email: Option<String>,
}

#[derive(Deserialize)]
struct ProjectEntry {
    folder: Option<String>,
    name: Option<String>,
    email: Option<String>,
}

/// Whether a table in `value`, however deep, has a `safe-folders` key. The toml parser stops at 80
/// levels, so this does too.
fn holds_safe_folders(value: &toml::Value) -> bool {
    match value {
        toml::Value::Table(table) => {
            table.contains_key("safe-folders") || table.values().any(holds_safe_folders)
        }
        toml::Value::Array(items) => items.iter().any(holds_safe_folders),
        _ => false,
    }
}

fn both(name: Option<String>, email: Option<String>) -> Option<Identity> {
    match (name, email) {
        (Some(name), Some(email)) if !name.is_empty() && !email.is_empty() => {
            Some(Identity { name, email })
        }
        _ => None,
    }
}

/// Whether `a` and `b` are one folder: each is resolved (links, `..`, short names) and the results
/// must be exactly equal. Windows spells resolved names as stored, so `C:\WORK` matches `C:\Work`;
/// nothing else is folded (a KELVIN SIGN is not a `K`). An unresolvable folder never matches.
pub(crate) fn same_folder(a: &Path, b: &Path) -> bool {
    let (Ok(a), Ok(b)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) else {
        return false;
    };
    a == b
}

/// The `config.toml` at `path`, or `None` if there is none. `read` is `safe_fs::read_capped` for a
/// project's file (never follows a link) and `safe_fs::read_capped_following_links` for the user's.
fn read_file(
    path: &Path,
    read: fn(&Path, u64) -> std::io::Result<Vec<u8>>,
) -> Result<Option<ConfigFile>, ConfigError> {
    let invalid = |detail: String| ConfigError::Invalid {
        path: path.to_owned(),
        detail,
    };
    let bytes = match read(path, safe_fs::MAX_CONFIG_BYTES) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            // `safe_fs` starts its messages with the path, which the error names already.
            let said = e.to_string();
            let reason = match said.strip_prefix(&path.display().to_string()) {
                Some(rest) => format!("it{rest}"),
                None => said,
            };
            return Err(invalid(reason));
        }
    };
    let parsed: toml::Value =
        toml::from_slice(&bytes).map_err(|e| invalid(e.message().to_owned()))?;
    // The top level is where the list belongs, so only what is below it counts.
    let misplaced = parsed
        .as_table()
        .is_some_and(|top| top.values().any(holds_safe_folders));
    let mut file: ConfigFile = parsed
        .try_into()
        .map_err(|e| invalid(e.message().to_owned()))?;
    file.misplaced_safe_folders = misplaced;
    Ok(Some(file))
}

/// The identity in a `config.toml`, if it exists and names both: the `[[project]]` entry for
/// `project_root` (the last complete one, so a line added at the end takes effect), else `[user]`.
/// A project's own file is read with `None` for `project_root`: it has no say in who the user is.
fn from_file(
    path: &Path,
    read: fn(&Path, u64) -> std::io::Result<Vec<u8>>,
    project_root: Option<&Path>,
) -> Result<Option<Identity>, ConfigError> {
    let Some(file) = read_file(path, read)? else {
        return Ok(None);
    };
    // As in safe-folders: a relative folder would name a different one wherever a command runs.
    if project_root.is_some()
        && let Some(folder) = file
            .project
            .iter()
            .filter_map(|entry| entry.folder.as_deref())
            .find(|folder| !Path::new(folder).is_absolute())
    {
        return Err(ConfigError::Invalid {
            path: path.to_owned(),
            detail: format!("[[project]] folder must be a full path, not \"{folder}\""),
        });
    }
    let for_folder = project_root.and_then(|root| {
        file.project.into_iter().rev().find_map(|entry| {
            let folder = entry.folder?;
            same_folder(Path::new(&folder), root)
                .then(|| both(entry.name, entry.email))
                .flatten()
        })
    });
    Ok(for_folder.or_else(|| file.user.and_then(|u| both(u.name, u.email))))
}

/// `config.toml` in `BOLT_CONFIG_DIR` if set, else in the system's settings folder for Bolt.
#[must_use]
pub fn user_config_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("BOLT_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => directories::ProjectDirs::from("", "", "bolt")?
            .config_dir()
            .to_owned(),
    };
    Some(dir.join("config.toml"))
}

/// The user's name and email, from the first source that names both (decision 025): the environment
/// (`BOLT_USER_NAME`, `BOLT_USER_EMAIL`), the user's own `config.toml` (the `[[project]]` entry for
/// `project_root`, then `[user]`). The project's own `.bolt/config.toml` is never used.
pub fn load_identity(project_root: Option<&Path>) -> Result<Identity, ConfigError> {
    identity_from(identity_from_env(), user_config_path(), project_root)
}

/// The identity the environment names: `BOLT_USER_NAME` and `BOLT_USER_EMAIL`, both set. One alone
/// names nobody.
fn identity_from_env() -> Option<Identity> {
    let env = |key| std::env::var(key).ok();
    both(env("BOLT_USER_NAME"), env("BOLT_USER_EMAIL"))
}

/// `load_identity` with the places it looks in given, so that tests can choose them: the
/// environment's identity, and the path of the user's own `config.toml`.
fn identity_from(
    from_env: Option<Identity>,
    user_config: Option<PathBuf>,
    project_root: Option<&Path>,
) -> Result<Identity, ConfigError> {
    if let Some(identity) = from_env {
        return Ok(identity);
    }
    if let Some(path) = &user_config
        && let Some(identity) = from_file(path, safe_fs::read_capped_following_links, project_root)?
    {
        return Ok(identity);
    }
    Err(ConfigError::MissingIdentity { path: user_config })
}

/// The identity the project's own `.bolt/config.toml` names, if Bolt did not use it (it differs
/// from the one `load_identity` gives, which is none if that fails). Read like every project file:
/// never through a link, at most 1 MiB; one that can't be read names nobody.
#[must_use]
pub fn ignored_project_identity(project_root: &Path) -> Option<Identity> {
    ignored_given(
        project_root,
        load_identity(Some(project_root)).ok().as_ref(),
    )
}

/// `ignored_project_identity`, given the identity that is in use.
fn ignored_given(project_root: &Path, used: Option<&Identity>) -> Option<Identity> {
    let path = project_root.join(".bolt").join("config.toml");
    let named = from_file(&path, safe_fs::read_capped, None)
        .ok()
        .flatten()?;
    (used != Some(&named)).then_some(named)
}

/// What `save` adds when a project's settings name an identity Bolt ignored: what was ignored and
/// the lines that would make it the user's choice here (decision 025). Project text is escaped.
#[must_use]
pub fn ignored_identity_note(ignored: &Identity, project_root: &Path) -> String {
    note_for(
        ignored,
        project_root,
        user_config_path().as_deref(),
        identity_from_env().is_some(),
    )
}

/// `text` as a TOML string for a line the user is shown to add. The TOML writer quotes it and
/// escaping comes first, so no folder, name or email can break out of its line or start a table.
fn toml_string(text: &str) -> String {
    toml::Value::String(escape(text, false)).to_string()
}

/// The most characters of a name or email a note shows.
const MAX_SHOWN: usize = 256;

/// `text` escaped and, past `MAX_SHOWN` characters, cut there and ended with `…`; and whether it
/// was cut.
fn shown(text: &str) -> (String, bool) {
    match text.char_indices().nth(MAX_SHOWN) {
        Some((end, _)) => (format!("{}…", escape(&text[..end], false)), true),
        None => (escape(text, false), false),
    }
}

/// `ignored_identity_note`, given the path of the user's `config.toml` and whether the environment
/// names the user, which comes before the settings.
fn note_for(
    ignored: &Identity,
    project_root: &Path,
    config: Option<&Path>,
    env_first: bool,
) -> String {
    let ((name, name_cut), (email, email_cut)) = (shown(&ignored.name), shown(&ignored.email));
    let said = format!(
        "note: This project's settings name {name} <{email}>; \
         Bolt ignores identities from project settings."
    );
    if name_cut || email_cut {
        // Lines with the name cut short would save as someone else.
        return format!(
            "{said}\nhint: That name or email is longer than the {MAX_SHOWN} characters Bolt \
             shows, so it offers no lines to save as them. If you mean to, add them to your own \
             settings yourself."
        );
    }
    let snippet = format!(
        "[[project]]\nfolder = {}\nname = {}\nemail = {}",
        toml_string(&project_root.to_string_lossy()),
        toml_string(&ignored.name),
        toml_string(&ignored.email),
    );
    let place = match config {
        Some(path) => format!(
            "add this to {}:",
            escape(&path.display().to_string(), false)
        ),
        None => "set BOLT_CONFIG_DIR to a folder and add this to config.toml there:".to_owned(),
    };
    let first = if env_first {
        "BOLT_USER_NAME and BOLT_USER_EMAIL are set, and they come before your settings, so the \
         lines below take no effect until you unset them. "
    } else {
        ""
    };
    format!(
        "{said}\nhint: {first}To save as them in this project, {place}\n{}",
        // The lines are Bolt's own, so their newlines stay.
        escape(&snippet, true)
    )
}

/// The folders whose projects the user trusts although another account owns the folder or its
/// `.bolt`, or Bolt can't tell who does (decisions 025 to 027, T23): the `safe-folders` list in the
/// user's own `config.toml`, never a project's. Entries must be full paths, since a relative one
/// names a different folder wherever a command runs.
pub fn safe_folders() -> Result<Vec<PathBuf>, ConfigError> {
    user_config_path().map_or(Ok(Vec::new()), |path| safe_folders_in(&path))
}

/// `safe_folders`, from the user's `config.toml` at `path`.
fn safe_folders_in(path: &Path) -> Result<Vec<PathBuf>, ConfigError> {
    let Some(file) = read_file(path, safe_fs::read_capped_following_links)? else {
        return Ok(Vec::new());
    };
    if file.misplaced_safe_folders {
        return Err(ConfigError::SafeFoldersNotAtTop {
            path: path.to_owned(),
        });
    }
    file.safe_folders
        .into_iter()
        .map(|entry| {
            let folder = PathBuf::from(&entry);
            if folder.is_absolute() {
                Ok(folder)
            } else {
                Err(ConfigError::Invalid {
                    path: path.to_owned(),
                    detail: format!("safe-folders must list full paths, not \"{entry}\""),
                })
            }
        })
        .collect()
}

/// What the user can do about a project Bolt may not use (another account owns the folder or its
/// `.bolt`, or Bolt can't tell who does): the line that trusts its folder `root`, and where it goes
/// (decisions 025 to 027). For a folder other accounts can write to, why Bolt offers none.
#[must_use]
pub fn trust_hint(root: &Path) -> String {
    trust_hint_for(root, user_config_path().as_deref())
}

/// Whether other accounts can put a project in `folder`: on Unix, when anyone may write to it (the
/// sticky bit of `/tmp` only stops them removing each other's files); on Windows, when it is a
/// drive's root, where any account may make folders.
fn others_can_write(folder: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(folder).is_ok_and(|meta| meta.permissions().mode() & 0o002 != 0)
    }
    #[cfg(windows)]
    {
        // Also a link that leads to a root.
        folder.parent().is_none()
            || std::fs::canonicalize(folder).is_ok_and(|resolved| resolved.parent().is_none())
    }
}

/// `trust_hint`, given the path of the user's `config.toml`.
fn trust_hint_for(root: &Path, config: Option<&Path>) -> String {
    // Anyone could have planted a project there (CVE-2022-24765), so no line is offered.
    if others_can_write(root) {
        return format!(
            "Bolt doesn't suggest adding {} to safe-folders: other accounts on this computer \
             can write there, so anyone could put a project in it that Bolt would then trust. \
             Move your project into a folder of your own.",
            escape(&root.display().to_string(), false)
        );
    }
    let place = config.map_or_else(
        || "your Bolt settings file".to_owned(),
        |path| escape(&path.display().to_string(), false),
    );
    // "At the top": a key written below a `[user]` line would belong to that section instead.
    format!(
        "If you trust it, add its folder to safe-folders at the top of {place}:\n\
         safe-folders = [{}]",
        toml_string(&root.to_string_lossy())
    )
}

/// `identity`, now: the time in milliseconds and the local offset from UTC in minutes.
#[must_use]
pub fn now_person(identity: &Identity) -> Person {
    let now = jiff::Zoned::now();
    Person {
        name: identity.name.clone(),
        email: identity.email.clone(),
        time_ms: now.timestamp().as_millisecond(),
        tz_offset_min: i16::try_from(now.offset().seconds() / 60).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn reads_the_user_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let read = |path: &Path| from_file(path, safe_fs::read_capped, None);
        assert_eq!(read(&path).unwrap(), None);
        std::fs::write(
            &path,
            "[user]\nname = \"Ada\"\nemail = \"ada@example.com\"\n",
        )
        .unwrap();
        let ada = read(&path).unwrap().unwrap();
        assert_eq!(
            (ada.name.as_str(), ada.email.as_str()),
            ("Ada", "ada@example.com")
        );
        std::fs::write(&path, "[user]\nname = \"Ada\"\n").unwrap();
        assert_eq!(read(&path).unwrap(), None);
        std::fs::write(&path, "[user\n").unwrap();
        assert!(matches!(read(&path), Err(ConfigError::Invalid { .. })));
    }

    #[test]
    fn missing_identity_shows_the_snippet() {
        let e = ConfigError::MissingIdentity {
            path: Some(PathBuf::from("cfg.toml")),
        };
        let hint = e.hint().unwrap();
        assert!(
            hint.contains("cfg.toml") && hint.contains(SNIPPET),
            "{hint}"
        );
    }

    #[test]
    fn untrusted_text_in_errors_is_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        // The path (a project's folder is in it) and what the file said, both shown.
        let invalid = ConfigError::Invalid {
            path: PathBuf::from(HOSTILE),
            detail: HOSTILE.into(),
        };
        let shown = invalid.to_string();
        assert_hostile_text_escaped(&shown);
        assert_eq!(shown.matches("b\\u{202e}c").count(), 2, "{shown}");
        let misplaced = ConfigError::SafeFoldersNotAtTop {
            path: PathBuf::from(HOSTILE),
        };
        let shown = misplaced.to_string();
        assert_hostile_text_escaped(&shown);
        assert_eq!(shown.matches("b\\u{202e}c").count(), 1, "{shown}");
        let missing = ConfigError::MissingIdentity {
            path: Some(PathBuf::from(HOSTILE)),
        };
        let hint = missing.hint().unwrap();
        assert_hostile_text_escaped(&hint);
        // Bolt's own line breaks in the hint stay.
        assert!(hint.contains(":\n[user]\nname"), "{hint}");
    }

    #[test]
    fn now_person_has_a_plausible_time() {
        let p = now_person(&Identity {
            name: "A".into(),
            email: "a@b".into(),
        });
        assert!(p.time_ms > 1_700_000_000_000);
        assert!(p.tz_offset_min.abs() <= 26 * 60);
    }

    // Whose identity it is (RT-9, decision 025)

    fn person(name: &str, email: &str) -> Identity {
        Identity {
            name: name.into(),
            email: email.into(),
        }
    }

    /// `text` as a TOML string, however awkward it is.
    fn quoted(text: &str) -> String {
        toml::Value::String(text.into()).to_string()
    }

    /// The user's own settings: `[user]` Ada, and then these `[[project]]` entries.
    fn user_config(dir: &Path, entries: &[(&Path, &str, &str)]) -> PathBuf {
        let mut text = "[user]\nname = \"Ada\"\nemail = \"ada@example.com\"\n".to_owned();
        for (folder, name, email) in entries {
            let folder = quoted(&folder.display().to_string());
            text += &format!(
                "\n[[project]]\nfolder = {folder}\nname = {}\nemail = {}\n",
                quoted(name),
                quoted(email)
            );
        }
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        path
    }

    fn folder(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// The `.bolt/config.toml` a shared project can bring.
    fn project_settings(root: &Path, text: &str) {
        std::fs::create_dir_all(root.join(".bolt")).unwrap();
        std::fs::write(root.join(".bolt").join("config.toml"), text).unwrap();
    }

    const MALLORY: &str = "[user]\nname = \"Mallory\"\nemail = \"mallory@evil.example\"\n";

    #[test]
    fn project_entry_in_user_config_wins_for_that_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (work, play) = (folder(dir.path(), "work"), folder(dir.path(), "play"));
        let config = user_config(dir.path(), &[(&work, "Work Name", "me@work.example")]);
        let identity = |root: Option<&Path>| identity_from(None, Some(config.clone()), root);
        assert_eq!(
            identity(Some(&work)).unwrap(),
            person("Work Name", "me@work.example")
        );
        // The same folder written another way: through a `..`, and with a closing separator.
        let roundabout = play.join("..").join("work").join("");
        assert_eq!(
            identity(Some(&roundabout)).unwrap(),
            person("Work Name", "me@work.example")
        );
        // Every other folder, and a project that is not started yet, get the user's own.
        assert_eq!(
            identity(Some(&play)).unwrap(),
            person("Ada", "ada@example.com")
        );
        assert_eq!(identity(None).unwrap(), person("Ada", "ada@example.com"));
        // The environment still comes first.
        let env = person("Env", "env@example.com");
        let first = identity_from(Some(env.clone()), Some(config.clone()), Some(&work));
        assert_eq!(first.unwrap(), env);
    }

    #[test]
    fn project_entry_for_another_folder_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let project = folder(dir.path(), "project");
        let (other, below) = (folder(dir.path(), "other"), folder(&project, "below"));
        let missing = dir.path().join("missing");
        // An entry for another folder, for a folder inside this one, for one that doesn't exist
        // (which matches nothing, not even itself), and one for this folder with no email.
        let config = user_config(
            dir.path(),
            &[
                (&other, "Other", "other@example.com"),
                (&below, "Below", "below@example.com"),
                (&missing, "Missing", "missing@example.com"),
                (&project, "No Email", ""),
            ],
        );
        for root in [&project, &missing] {
            let got = identity_from(None, Some(config.clone()), Some(root));
            assert_eq!(got.unwrap(), person("Ada", "ada@example.com"), "{root:?}");
        }
    }

    #[test]
    fn project_entries_must_name_full_paths() {
        let dir = tempfile::tempdir().unwrap();
        let project = folder(dir.path(), "project");
        for entry in ["project", "../project", "", r"\project", "C:project"] {
            let config = user_config(dir.path(), &[(Path::new(entry), "Rel", "rel@example.com")]);
            match identity_from(None, Some(config), Some(&project)) {
                Err(ConfigError::Invalid { detail, .. }) => {
                    assert!(detail.contains("full path"), "{entry:?}: {detail}");
                }
                other => panic!("{entry:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn the_last_complete_entry_for_a_folder_wins() {
        let dir = tempfile::tempdir().unwrap();
        let project = folder(dir.path(), "project");
        let config = user_config(
            dir.path(),
            &[
                (&project, "First", "first@example.com"),
                (&project, "Second", "second@example.com"),
                (&project, "Third", ""),
            ],
        );
        let got = identity_from(None, Some(config), Some(&project));
        assert_eq!(got.unwrap(), person("Second", "second@example.com"));
    }

    #[test]
    fn bolt_config_identity_is_never_used() {
        let dir = tempfile::tempdir().unwrap();
        let project = folder(dir.path(), "project");
        // Whatever a project says about who the user is, in either shape.
        let planted = format!(
            "{MALLORY}\n[[project]]\nfolder = {}\nname = \"Mallory\"\nemail = \"mallory@evil.example\"\n",
            quoted(&project.display().to_string())
        );
        project_settings(&project, &planted);
        let config = user_config(dir.path(), &[]);
        let got = identity_from(None, Some(config), Some(&project));
        assert_eq!(got.unwrap(), person("Ada", "ada@example.com"));
        // With nobody else to be, the user is told to say who they are, not given Mallory.
        let none = dir.path().join("no-such-config.toml");
        let got = identity_from(None, Some(none), Some(&project));
        assert!(matches!(got, Err(ConfigError::MissingIdentity { .. })));
        let got = identity_from(None, None, Some(&project));
        assert!(matches!(
            got,
            Err(ConfigError::MissingIdentity { path: None })
        ));
    }

    #[test]
    fn same_folder_resolves_both_sides() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (folder(dir.path(), "a"), folder(dir.path(), "b"));
        assert!(same_folder(&a, &a));
        assert!(same_folder(&a, &b.join("..").join("a")));
        assert!(!same_folder(&a, &b));
        // A folder that isn't there is the same as nothing, itself included.
        let missing = dir.path().join("missing");
        assert!(!same_folder(&missing, &missing));
        assert!(!same_folder(&a, &missing) && !same_folder(&missing, &a));
        let link = dir.path().join("link");
        if crate::test_support::symlink(&a, &link) {
            assert!(same_folder(&link, &a) && same_folder(&a, &link));
            assert!(!same_folder(&link, &b));
        }
    }

    #[cfg(windows)]
    #[test]
    fn same_folder_ignores_case_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let work = folder(dir.path(), "Work");
        // Resolving gives back each name as stored on disk, so no folding is needed for these.
        let shouted = work.to_string_lossy().to_uppercase();
        let whispered = work.to_string_lossy().to_lowercase();
        assert!(same_folder(&work, Path::new(&shouted)));
        assert!(same_folder(Path::new(&whispered), &work));
    }

    #[test]
    fn what_the_project_names_is_ignored_unless_it_was_used() {
        let dir = tempfile::tempdir().unwrap();
        let project = folder(dir.path(), "project");
        let mallory = person("Mallory", "mallory@evil.example");
        assert_eq!(ignored_given(&project, None), None, "no settings at all");
        project_settings(&project, MALLORY);
        let ada = person("Ada", "ada@example.com");
        assert_eq!(ignored_given(&project, Some(&ada)), Some(mallory.clone()));
        // Nobody could be found for the user: the project's is still not used.
        assert_eq!(ignored_given(&project, None), Some(mallory.clone()));
        // The same as the identity in use: nothing was ignored.
        assert_eq!(ignored_given(&project, Some(&mallory)), None);
        // Not an identity: half of one, or none.
        for text in ["[user]\nname = \"Mallory\"\n", "[user]\n", "", "x = 1\n"] {
            project_settings(&project, text);
            assert_eq!(ignored_given(&project, Some(&ada)), None, "{text:?}");
        }
    }

    #[test]
    fn project_settings_that_cannot_be_read_name_nobody() {
        let dir = tempfile::tempdir().unwrap();
        let project = folder(dir.path(), "project");
        let ada = person("Ada", "ada@example.com");
        let path = project.join(".bolt").join("config.toml");
        let unreadable =
            |what: &str| assert_eq!(ignored_given(&project, Some(&ada)), None, "{what}");

        project_settings(&project, "[user\n");
        unreadable("not TOML");
        let big = format!("{MALLORY}# {}\n", "x".repeat(1 << 20));
        project_settings(&project, &big);
        unreadable("over 1 MiB");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        unreadable("a folder");
        std::fs::remove_dir(&path).unwrap();
        unreadable("missing");

        // A link is never followed, and a pipe is never waited on (the way into RT-3 and RT-4).
        let target = dir.path().join("target.toml");
        std::fs::write(&target, MALLORY).unwrap();
        if crate::test_support::symlink(&target, &path) {
            unreadable("a link to a settings file");
        }
        #[cfg(unix)]
        {
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
            unreadable("a link to /dev/zero");
            std::fs::remove_file(&path).unwrap();
            let made = std::process::Command::new("mkfifo").arg(&path).status();
            assert!(made.unwrap().success());
            // A hang fails this test instead of stopping the whole run.
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(ignored_given(&project, None));
            });
            let got = rx.recv_timeout(std::time::Duration::from_secs(10));
            assert_eq!(got.expect("reading a named pipe hung"), None);
        }
    }

    const MALLORY_NOTE: &str = "note: This project's settings name Mallory \
                                <mallory@evil.example>; Bolt ignores identities from project settings.";

    #[test]
    fn the_note_says_what_was_ignored_and_offers_the_lines() {
        let mallory = person("Mallory", "mallory@evil.example");
        let cfg = Path::new("/home/me/.config/bolt/config.toml");
        let note = note_for(&mallory, Path::new("/home/me/Work"), Some(cfg), false);
        let expected = [
            MALLORY_NOTE,
            "hint: To save as them in this project, add this to /home/me/.config/bolt/config.toml:",
            "[[project]]",
            "folder = \"/home/me/Work\"",
            "name = \"Mallory\"",
            "email = \"mallory@evil.example\"",
        ];
        assert_eq!(note, expected.join("\n"));

        // A Windows folder keeps its backslashes, in the quoting TOML gives them
        // (decision 025's example).
        let note = note_for(
            &mallory,
            Path::new(r"C:\Users\me\Projects\Work"),
            None,
            false,
        );
        assert!(
            note.contains("\nfolder = 'C:\\Users\\me\\Projects\\Work'\n"),
            "{note}"
        );
        // With nowhere known to put them, the way to get one is said instead.
        let hint = "hint: To save as them in this project, set BOLT_CONFIG_DIR to a folder \
                    and add this to config.toml there:\n[[project]]\n";
        assert!(note.contains(hint), "{note}");
    }

    /// The part of a note after its two lines of words.
    fn snippet(note: &str) -> &str {
        note.splitn(3, '\n').nth(2).unwrap()
    }

    #[test]
    fn the_snippet_stays_valid_toml_for_awkward_folders_and_names() {
        let cases = [
            (r"C:\Users\me\Projects\Work", "Work Name", "me@work.example"),
            (
                r"C:\Users\o'brien\Work",
                r#"Bob "Bobby" O'Neil"#,
                "bob@example.com",
            ),
            ("/home/it's \"mine\"", r"A\B", "a'b@example.com"),
            ("/tmp/''' and \"\"\"", "x\"\"\"y'''z", "'''@\"\"\""),
            (
                "/tmp/ends with a quote'",
                "ends with a backslash\\",
                "e@x.example",
            ),
            ("/tmp/café 日本語", "Zoë 日本語", "zoe@example.com"),
        ];
        for (folder, name, email) in cases {
            let note = note_for(&person(name, email), Path::new(folder), None, false);
            let parsed: toml::Table = toml::from_str(snippet(&note)).unwrap_or_else(|e| {
                panic!("{folder:?}, {name:?}: {e}\n{note}");
            });
            let entry = &parsed["project"][0];
            assert_eq!(
                parsed["project"].as_array().map(Vec::len),
                Some(1),
                "{note}"
            );
            assert_eq!(entry["folder"].as_str(), Some(folder), "{note}");
            assert_eq!(entry["name"].as_str(), Some(name), "{note}");
            assert_eq!(entry["email"].as_str(), Some(email), "{note}");
            assert_eq!(snippet(&note).lines().count(), 4, "{note}");
        }
    }

    // Folders the user trusts although another account owns them (T23, decision 025)

    #[test]
    fn safe_folders_come_from_the_users_own_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // No settings file, or one with no list, trusts nothing.
        assert!(safe_folders_in(&path).unwrap().is_empty());
        std::fs::write(&path, "[user]\nname = \"Ada\"\n").unwrap();
        assert!(safe_folders_in(&path).unwrap().is_empty());
        // Full paths, as decision 025 writes them, at the top of the file.
        let (work, play) = (dir.path().join("work"), dir.path().join("play"));
        let text = format!(
            "safe-folders = [{}, {}]\n[user]\nname = \"Ada\"\n",
            quoted(&work.display().to_string()),
            quoted(&play.display().to_string())
        );
        std::fs::write(&path, text).unwrap();
        assert_eq!(safe_folders_in(&path).unwrap(), [work, play]);
    }

    #[test]
    fn safe_folders_must_be_full_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // A relative entry would name a different folder wherever a command runs. The last two
        // are relative on Windows too: no drive, and a drive with no root.
        for entry in ["work", "../work", "", r"\work", "C:work"] {
            std::fs::write(&path, format!("safe-folders = [{}]\n", quoted(entry))).unwrap();
            match safe_folders_in(&path) {
                Err(ConfigError::Invalid { detail, .. }) => {
                    assert!(detail.contains("full paths"), "{entry:?}: {detail}");
                }
                other => panic!("{entry:?}: {other:?}"),
            }
        }
        // Not a list of text.
        for text in ["safe-folders = '/work'\n", "safe-folders = [1]\n"] {
            std::fs::write(&path, text).unwrap();
            let got = safe_folders_in(&path);
            assert!(matches!(got, Err(ConfigError::Invalid { .. })), "{text}");
        }
    }

    #[test]
    fn the_trust_hint_offers_the_line_to_add() {
        let cfg = Path::new("/home/me/.config/bolt/config.toml");
        assert_eq!(
            trust_hint_for(Path::new("/srv/shared/work"), Some(cfg)),
            "If you trust it, add its folder to safe-folders at the top of \
             /home/me/.config/bolt/config.toml:\nsafe-folders = [\"/srv/shared/work\"]"
        );
        // Decision 025's form for a Windows folder; and with no known settings file, the words.
        let hint = trust_hint_for(Path::new(r"C:\Shared\Work"), None);
        assert_eq!(
            hint,
            "If you trust it, add its folder to safe-folders at the top of your Bolt settings \
             file:\nsafe-folders = ['C:\\Shared\\Work']"
        );
        // The line stays valid TOML that names the folder exactly, however it is spelt.
        for folder in [
            r"C:\Users\o'brien\Work",
            "/tmp/''' and \"\"\"",
            "/tmp/ends with a quote'",
            "/tmp/café 日本語",
        ] {
            let hint = trust_hint_for(Path::new(folder), None);
            let line = hint.lines().nth(1).unwrap();
            let parsed: toml::Table = toml::from_str(line).unwrap();
            assert_eq!(parsed["safe-folders"][0].as_str(), Some(folder), "{hint}");
            assert_eq!(hint.lines().count(), 2, "{hint}");
        }
        // Nothing in the folder's name or the settings path can drive the terminal or add a line.
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let hint = trust_hint_for(Path::new(HOSTILE), Some(Path::new(HOSTILE)));
        assert_hostile_text_escaped(&hint);
        assert_eq!(hint.lines().count(), 2, "{hint}");
        let sneaky = trust_hint_for(Path::new("/x']\n[user]\nname = 'Mallory"), None);
        assert_eq!(sneaky.lines().count(), 2, "{sneaky}");
    }

    #[test]
    fn the_note_and_snippet_cannot_drive_the_terminal() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let planted = person(HOSTILE, &format!("{HOSTILE}@evil.example"));
        let cfg = PathBuf::from(HOSTILE);
        let note = note_for(&planted, Path::new(HOSTILE), Some(&cfg), false);
        assert_hostile_text_escaped(&note);
        // Two lines of words and four of snippet, whatever the project put in its name: no
        // line break of its own, and no text that would pass for the lines Bolt writes.
        assert_eq!(note.lines().count(), 6, "{note}");
        let parsed: toml::Table = toml::from_str(snippet(&note)).unwrap();
        assert_eq!(
            parsed["project"].as_array().map(Vec::len),
            Some(1),
            "{note}"
        );
        // A name that is a whole TOML table, or a line of the note.
        let sneaky = "x\"\n[user]\nname = \"Mallory\"\nhint: do something";
        let note = note_for(
            &person(sneaky, "m@evil.example"),
            Path::new("/p"),
            None,
            false,
        );
        assert_eq!(note.lines().count(), 6, "{note}");
        let parsed: toml::Table = toml::from_str(snippet(&note)).unwrap();
        assert!(parsed.get("user").is_none(), "{note}");
    }

    #[test]
    fn the_identity_note_is_short_and_honest() {
        let root = Path::new("/home/me/Work");
        let cfg = Path::new("/home/me/.config/bolt/config.toml");
        // At most 256 characters of a name or an email, cut between characters (these take two
        // bytes each), then `…`. Exactly 256 are shown whole.
        let (long, whole) = ("é".repeat(300), "x".repeat(256));
        let note = note_for(&person(&long, &whole), root, Some(cfg), false);
        let said = format!(
            "note: This project's settings name {}… <{whole}>; \
             Bolt ignores identities from project settings.",
            "é".repeat(256)
        );
        assert_eq!(note.lines().next(), Some(said.as_str()), "{note}");
        // Lines that save as a name cut short would save as someone else, so none are offered.
        assert!(!note.contains("[[project]]"), "{note}");
        assert!(note.len() < 2_000, "{note}");
        let note = note_for(&person("Ada", &"y".repeat(257)), root, Some(cfg), false);
        assert!(note.contains(&format!("<{}…>", "y".repeat(256))), "{note}");
        assert!(!note.contains("[[project]]"), "{note}");
        let fits = note_for(&person(&whole, &whole), root, Some(cfg), false);
        assert!(
            fits.contains("[[project]]") && !fits.contains('…'),
            "{fits}"
        );
        // The cut comes before the escaping, so what is shown stays safe to print.
        let hidden = "\u{202e}".repeat(300);
        let note = note_for(&person(&hidden, "m@evil.example"), root, Some(cfg), false);
        assert!(!note.contains('\u{202e}'), "{note}");
        assert_eq!(note.matches("\\u{202e}").count(), 256, "{note}");

        // While BOLT_USER_NAME and BOLT_USER_EMAIL are set they come before the user's settings,
        // so the lines the hint offers would change nothing.
        let mallory = person("Mallory", "mallory@evil.example");
        let plain = note_for(&mallory, root, Some(cfg), false);
        let first = note_for(&mallory, root, Some(cfg), true);
        assert!(!plain.contains("BOLT_USER"), "{plain}");
        let hint = first.lines().nth(1).unwrap();
        assert!(
            hint.starts_with("hint: BOLT_USER_NAME and BOLT_USER_EMAIL are set")
                && hint.contains("take no effect until you unset them. To save as them"),
            "{hint}"
        );
        assert!(hint.ends_with("add this to /home/me/.config/bolt/config.toml:"));
        assert_eq!(snippet(&first), snippet(&plain));
    }

    // The settings file's own mistakes

    #[test]
    fn safe_folders_in_a_table_is_warned_about() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let work = quoted(&dir.path().join("work").display().to_string());
        let user = "[user]\nname = \"Ada\"\nemail = \"ada@example.com\"\n";
        let project = format!("[[project]]\nfolder = {work}\nname = \"Ada\"\nemail = \"a@b\"\n");
        // The line the trust hint offers, added at the bottom of a file that has tables: it then
        // belongs to the last one, where Bolt never looks for it.
        for text in [
            format!("{user}safe-folders = [{work}]\n"),
            format!("{project}safe-folders = [{work}]\n"),
            format!("user.safe-folders = [{work}]\n"),
            format!("[editor]\nsafe-folders = [{work}]\n"),
            format!("{user}[user.extra]\nsafe-folders = [{work}]\n"),
            format!("safe-folders = [{work}]\n{user}safe-folders = [{work}]\n"),
        ] {
            std::fs::write(&path, &text).unwrap();
            let err = safe_folders_in(&path).unwrap_err();
            let want = format!(
                "safe-folders must be at the top of {}, before any [section]",
                path.display()
            );
            assert_eq!(err.to_string(), want, "{text}");
            assert!(err.hint().unwrap().contains("above"), "{err:?}");
        }
        // At the top it is the list, whatever tables follow.
        std::fs::write(&path, format!("safe-folders = [{work}]\n{user}{project}")).unwrap();
        assert_eq!(safe_folders_in(&path).unwrap().len(), 1);
    }

    #[test]
    fn a_settings_file_that_cant_be_read_is_named_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let shown = path.display().to_string();
        std::fs::create_dir(&path).unwrap();
        let folder = safe_folders_in(&path).unwrap_err().to_string();
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, vec![b'#'; (1 << 20) + 1]).unwrap();
        let big = safe_folders_in(&path).unwrap_err().to_string();
        for said in [&folder, &big] {
            assert_eq!(said.matches(&shown).count(), 1, "{said}");
            assert!(
                said.starts_with(&format!("{shown} is not a valid Bolt")),
                "{said}"
            );
        }
        assert!(
            folder.ends_with(": it is not a regular file (it is a link, folder or special file)"),
            "{folder}"
        );
        assert!(big.ends_with(": it is larger than 1048576 bytes"), "{big}");
    }

    /// The search for a misplaced `safe-folders` walks the parsed file, which is only safe while
    /// the parser stops a file nested too deep.
    #[test]
    fn a_settings_file_nested_too_deep_is_refused_not_walked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let deep = 100_000;
        for text in [
            format!("a = {}{}\n", "[".repeat(deep), "]".repeat(deep)),
            format!("a = {}1{}\n", "{a=".repeat(deep), "}".repeat(deep)),
            format!("[{}]\n", "a.".repeat(deep) + "a"),
        ] {
            std::fs::write(&path, &text).unwrap();
            let got = safe_folders_in(&path);
            assert!(matches!(got, Err(ConfigError::Invalid { .. })), "{got:?}");
        }
    }

    // A folder anyone can put a project in is never offered for trust (CVE-2022-24765)

    #[cfg(unix)]
    #[test]
    fn the_trust_hint_never_offers_tmp() {
        use std::os::unix::fs::PermissionsExt;
        let offers = |folder: &Path| trust_hint_for(folder, None).contains("safe-folders = [");
        // The sticky bit only stops removing other accounts' files; anyone can still add a project.
        let hint = trust_hint_for(Path::new("/tmp"), None);
        assert!(!hint.contains("safe-folders = ["), "{hint}");
        assert!(
            hint.starts_with("Bolt doesn't suggest adding /tmp to safe-folders: "),
            "{hint}"
        );
        assert!(hint.contains("other accounts on this computer can write there"));
        assert!(
            hint.ends_with("Move your project into a folder of your own."),
            "{hint}"
        );
        let dir = tempfile::tempdir().unwrap();
        for (mode, offered) in [
            (0o1777, false),
            (0o777, false),
            (0o757, false),
            (0o770, true),
            (0o755, true),
            (0o700, true),
        ] {
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(offers(dir.path()), offered, "mode {mode:o}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_trust_hint_never_offers_a_drive_root() {
        // Any account may make a folder at the top of a drive, so a project planted there is
        // anyone's.
        for root in [r"C:\", r"D:\", r"\\server\share\", r"\\?\C:\"] {
            let hint = trust_hint_for(Path::new(root), None);
            assert!(!hint.contains("safe-folders = ["), "{root}: {hint}");
            assert!(
                hint.starts_with(&format!(
                    "Bolt doesn't suggest adding {root} to safe-folders: "
                )),
                "{root}: {hint}"
            );
            assert!(
                hint.ends_with("Move your project into a folder of your own."),
                "{hint}"
            );
        }
        // A folder on the drive is a different matter.
        let hint = trust_hint_for(Path::new(r"C:\Shared"), None);
        assert!(hint.ends_with("safe-folders = ['C:\\Shared']"), "{hint}");
        // A link that leads to a drive's root is that root, which is what safe-folders would trust.
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("to-the-root");
        crate::test_support::junction(&link, Path::new(r"C:\"));
        let hint = trust_hint_for(&link, None);
        assert!(!hint.contains("safe-folders = ["), "{hint}");
    }
}
