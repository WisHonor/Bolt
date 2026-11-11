//! Settings: who the user is (from the environment or the user's own `config.toml`) and the
//! `safe-folders` they trust. A project's own `.bolt/config.toml` is never asked: a shared zip
//! carries it, so a stranger could make the user's saves claim to be someone else (decision 025).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use bolt_platform::dir::Dir;
use serde::Deserialize;

use crate::git::{
    UserSetting, read_bool, read_count, read_user_settings, read_user_variable,
    shown as shown_bytes,
};
use crate::repo::Repository;
use crate::safe_fs;
use crate::save::Person;
use crate::text::escape;

mod git_path;
use git_path::tidy_path;

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
    /// A `safe-folders` or `undo-window-days` line below a `[section]` belongs to that section, so
    /// Bolt never sees it.
    #[error(
        "{key} must be at the top of {}, before any [section]",
        escape(&path.display().to_string(), false)
    )]
    NotAtTop { key: &'static str, path: PathBuf },
    /// The user's own Git settings, read for their ignore list (decision 047) and for
    /// `lfs.storage`, can't be read.
    #[error(
        "Bolt can't read your Git settings in {}: {}",
        escape(&path.display().to_string(), false),
        escape(detail, false)
    )]
    GitSettings { path: PathBuf, detail: String },
    /// A variable of the environment that places the user's own Git settings, or gives one, as
    /// `GIT_CONFIG_COUNT` does, can't be read.
    #[error(
        "Bolt can't read your Git settings in {}: {}",
        escape(name, false),
        escape(detail, false)
    )]
    GitVariable { name: String, detail: String },
}

const SNIPPET: &str = "[user]\nname = \"Your Name\"\nemail = \"you@example.com\"";

/// What the hints for the user's own Git settings add.
const GIT_SETTINGS_READ: &str = "Bolt reads your Git settings only for core.excludesFile, your \
                                 own list of files Git ignores, and for lfs.storage; `git config \
                                 --list --show-origin` shows what Git makes of them.";

impl ConfigError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            ConfigError::MissingIdentity { path: Some(path) } => Some(format!(
                "Add this to {} (make the file, and its folder, if they aren't there):\n{SNIPPET}",
                escape(&path.display().to_string(), false)
            )),
            ConfigError::MissingIdentity { path: None } => Some(format!(
                "Set BOLT_CONFIG_DIR to a folder and add this to config.toml there:\n{SNIPPET}"
            )),
            ConfigError::Invalid { .. } => None,
            ConfigError::NotAtTop { .. } => {
                Some("Move that line above [user] and every other [section].".to_owned())
            }
            ConfigError::GitSettings { .. } => Some(format!("Fix that file. {GIT_SETTINGS_READ}")),
            ConfigError::GitVariable { .. } => Some(format!(
                "Fix that variable, or unset it. {GIT_SETTINGS_READ}"
            )),
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
    /// How many days of steps `tidy` keeps what undo needs for; checked where it is read.
    #[serde(default, rename = "undo-window-days")]
    undo_window_days: Option<toml::Value>,
    /// Set after reading: the keys of `AT_TOP` also written below a `[section]`, where they belong
    /// to that section.
    #[serde(skip)]
    misplaced: Vec<&'static str>,
}

/// The keys that only count at the top of the user's own file.
const AT_TOP: [&str; 2] = ["safe-folders", "undo-window-days"];
/// `undo-window-days` when it isn't set, and the most it may be.
const UNDO_WINDOW_DAYS: u32 = 30;
const MAX_UNDO_WINDOW_DAYS: u32 = 36_500;

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

/// Whether a table in `value`, however deep, has the key `key`. The toml parser stops at 80
/// levels, so this does too.
fn holds_key(value: &toml::Value, key: &str) -> bool {
    match value {
        toml::Value::Table(table) => {
            table.contains_key(key) || table.values().any(|v| holds_key(v, key))
        }
        toml::Value::Array(items) => items.iter().any(|v| holds_key(v, key)),
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

/// The `config.toml` at `path`, or `None` if there is none. `read` reads a project's file through
/// its held `.bolt` (never following a link) and the user's with
/// `safe_fs::read_capped_following_links`.
fn read_file(
    path: &Path,
    read: &dyn Fn(&Path, u64) -> std::io::Result<Vec<u8>>,
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
    // The top level is where these keys belong, so only what is below it counts.
    let below = |key: &&str| {
        parsed
            .as_table()
            .is_some_and(|top| top.values().any(|v| holds_key(v, key)))
    };
    let misplaced = AT_TOP.into_iter().filter(below).collect();
    let mut file: ConfigFile = parsed
        .try_into()
        .map_err(|e| invalid(e.message().to_owned()))?;
    file.misplaced = misplaced;
    Ok(Some(file))
}

/// The identity in a `config.toml`, if it exists and names both: the `[[project]]` entry for
/// `project_root` (the last complete one, so a line added at the end takes effect), else `[user]`.
/// A project's own file is read with `None` for `project_root`: it has no say in who the user is.
fn from_file(
    path: &Path,
    read: &dyn Fn(&Path, u64) -> std::io::Result<Vec<u8>>,
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

/// The most of the user's own Git settings Bolt reads, as for a repository's `.git/config`.
const MAX_GIT_SETTINGS_BYTES: u64 = 1 << 20;

/// The user's own Git ignore list (decision 047): the file `core.excludesFile` names in the
/// user's own Git settings, else `$XDG_CONFIG_HOME/git/ignore`. `None` when the setting is empty
/// or names no file (`/dev/null`, and `nul` on Windows), or with neither a home folder nor
/// `XDG_CONFIG_HOME` to find the default list in.
pub fn git_excludes_file(project_root: &Path) -> Result<Option<PathBuf>, ConfigError> {
    git_excludes_file_in(&UserGit::from_env(project_root))
}

/// What the user's own Git settings set that `bolt import` in `top` reads: `lfs.storage`, only to
/// tell whether Git LFS keeps its files elsewhere, and `core.sparseCheckout`, which it refuses.
pub fn git_import_settings(top: &Path) -> Result<UserImportSettings, ConfigError> {
    git_import_settings_in(&UserGit::from_env(top))
}

/// The user's own Git settings an import reads. The last `lfs.storage` in the user's own Git
/// settings files, which the repository's own setting overrides, and the last the environment
/// gives through `GIT_CONFIG_COUNT`, which overrides that in turn, as in Git: each with where it is
/// set, and empty for one with no value. The last `core.sparseCheckout` in each comes alike.
#[derive(Debug, Default)]
pub struct UserImportSettings {
    pub files: Option<(Vec<u8>, String)>,
    pub variables: Option<(Vec<u8>, String)>,
    pub sparse_files: Option<bool>,
    pub sparse_variables: Option<bool>,
}

/// Where Git finds the user's own settings: the home folder and `XDG_CONFIG_HOME`'s folder, if
/// there are any, the environment's variables, read through `var` (`std::env::var_os` but in
/// tests), and the project's top, where Git runs, so a relative path is read from there.
struct UserGit<'a> {
    home: Option<PathBuf>,
    xdg: Option<PathBuf>,
    top: &'a Path,
    var: &'a dyn Fn(&str) -> Option<OsString>,
}

impl<'a> UserGit<'a> {
    /// The user's own Git settings as this process's environment places them.
    fn from_env(top: &'a Path) -> UserGit<'a> {
        fn var(key: &str) -> Option<OsString> {
            std::env::var_os(key)
        }
        UserGit::new(top, &var, cfg!(windows), &Path::is_dir)
    }

    /// The user's own Git settings as the environment `var` places them, on Windows or not, with
    /// `is_dir` telling which paths are folders.
    fn new(
        top: &'a Path,
        var: &'a dyn Fn(&str) -> Option<OsString>,
        windows: bool,
        is_dir: &dyn Fn(&Path) -> bool,
    ) -> UserGit<'a> {
        let home = git_home(var, windows, is_dir).map(|home| {
            // Git puts `~/x` at the home folder's text and `/x`, so an empty one is `/`.
            if home.is_empty() {
                PathBuf::from("/")
            } else {
                PathBuf::from(home)
            }
        });
        // As Git finds it: the variable when it is set to something, else `~/.config`.
        let xdg = var("XDG_CONFIG_HOME")
            .filter(|xdg| !xdg.is_empty())
            .map(PathBuf::from);
        UserGit {
            xdg: xdg.or_else(|| Some(home.as_ref()?.join(".config"))),
            home,
            top,
            var,
        }
    }

    /// The user's own Git settings files, in Git's order, each with whether it is the user's own,
    /// which Git skips when it may not read it: the file `GIT_CONFIG_SYSTEM` names, unless
    /// `GIT_CONFIG_NOSYSTEM` is true, then the file `GIT_CONFIG_GLOBAL` names, or when that isn't
    /// set `$XDG_CONFIG_HOME/git/config` and `~/.gitconfig`. Set to nothing, or to a name for no
    /// file, either variable names no file, as in Git.
    fn files(&self) -> Result<Vec<(PathBuf, bool)>, ConfigError> {
        let mut files = Vec::new();
        if !self.no_system()? {
            files.extend(self.system_file()?.map(|path| (path, false)));
        }
        let global = (self.var)("GIT_CONFIG_GLOBAL").map(|path| {
            let none = path.is_empty() || names_no_file(path.as_encoded_bytes(), cfg!(windows));
            (!none).then(|| self.top.join(path))
        });
        let own = match global {
            Some(global) => Vec::from_iter(global),
            // Without a home folder, there is no `~/.gitconfig`, as in Git.
            None => [
                self.xdg.as_ref().map(|xdg| xdg.join("git").join("config")),
                self.home.as_ref().map(|home| home.join(".gitconfig")),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        files.extend(own.into_iter().map(|path| (path, true)));
        Ok(files)
    }

    /// The file `GIT_CONFIG_SYSTEM` names, tidied first as Git tidies it, if it names one.
    fn system_file(&self) -> Result<Option<PathBuf>, ConfigError> {
        let name = "GIT_CONFIG_SYSTEM";
        let Some(value) = (self.var)(name) else {
            return Ok(None);
        };
        let refused = |detail: String| ConfigError::GitVariable {
            name: name.to_owned(),
            detail,
        };
        let absolute = Path::new(&value).has_root();
        let value = value.as_encoded_bytes();
        let tidied = system_file_named(value, cfg!(windows)).map_err(|()| {
            let fix = if absolute {
                "write it without the `..`"
            } else {
                "write the full path instead"
            };
            refused(format!(
                "`{}` goes above its start with `..`, where Git's tidying of the path fails; {fix}",
                shown_bytes(value)
            ))
        })?;
        let Some(tidied) = tidied else {
            return Ok(None);
        };
        let path = os_path(tidied).ok_or_else(|| refused("it isn't valid Unicode".to_owned()))?;
        Ok(Some(self.top.join(path)))
    }

    /// Gives `each` every setting Bolt reads in the user's own Git settings files, in Git's order.
    fn read_files(&self, each: &mut dyn FnMut(UserSetting, &Place)) -> Result<(), ConfigError> {
        for (path, own) in self.files()? {
            read_git_settings(&path, self.home.as_deref(), 0, own, each)?;
        }
        Ok(())
    }

    /// Whether `GIT_CONFIG_NOSYSTEM` is true, read as Git reads it.
    fn no_system(&self) -> Result<bool, ConfigError> {
        let name = "GIT_CONFIG_NOSYSTEM";
        let Some(value) = (self.var)(name) else {
            return Ok(false);
        };
        read_bool(value.as_encoded_bytes()).ok_or_else(|| ConfigError::GitVariable {
            name: name.to_owned(),
            detail: format!(
                "`{}` isn't true or false to Git, which refuses it too",
                shown_bytes(value.as_encoded_bytes())
            ),
        })
    }

    /// Gives `each` every setting Bolt reads among those the environment gives through
    /// `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>` and `GIT_CONFIG_VALUE_<n>`, which Git reads after
    /// every file, in their order. What Git refuses there stops Bolt too.
    fn read_variables(&self, each: &mut dyn FnMut(UserSetting, &Place)) -> Result<(), ConfigError> {
        let refused = |name: &str, detail: String| ConfigError::GitVariable {
            name: name.to_owned(),
            detail,
        };
        let Some(count) = (self.var)("GIT_CONFIG_COUNT") else {
            return Ok(());
        };
        let count = read_count(count.as_encoded_bytes()).map_err(|why| {
            let value = shown_bytes(count.as_encoded_bytes());
            refused(
                "GIT_CONFIG_COUNT",
                format!("`{value}` {why}, and Git refuses it too"),
            )
        })?;
        for n in 0..count {
            let (key_name, value_name) = (
                format!("GIT_CONFIG_KEY_{n}"),
                format!("GIT_CONFIG_VALUE_{n}"),
            );
            let read = |name: &str| {
                (self.var)(name).ok_or_else(|| {
                    let detail = format!("it isn't set, though GIT_CONFIG_COUNT is {count}");
                    refused(name, format!("{detail}, and Git refuses that too"))
                })
            };
            let (key, value) = (read(&key_name)?, read(&value_name)?);
            let setting = read_user_variable(key.as_encoded_bytes(), value.as_encoded_bytes())
                .map_err(|()| {
                    let key = shown_bytes(key.as_encoded_bytes());
                    let detail = format!("`{key}` isn't a setting's name to Git (section.name)");
                    refused(&key_name, format!("{detail}, which refuses it too"))
                })?;
            let place = Place::Variable(value_name);
            match setting {
                // Git takes a relative include only from a file, from that file's folder.
                Some(UserSetting::Include(value)) => {
                    let value = utf8(value, "include.path", &place)?;
                    let path =
                        git_path(&value, "include.path", self.home.as_deref(), None, &place)?;
                    if let Some(included) = path {
                        read_git_settings(&included, self.home.as_deref(), 1, false, each)?;
                    }
                }
                Some(other) => each(other, &place),
                None => {}
            }
        }
        Ok(())
    }
}

/// `git_excludes_file` with the places given, so tests can choose them. The repository's own
/// config is never read for this: it could name any file on the computer.
fn git_excludes_file_in(git: &UserGit) -> Result<Option<PathBuf>, ConfigError> {
    let (mut named, mut stop) = (None, None);
    let mut take = |setting: UserSetting, place: &Place| {
        if let UserSetting::ExcludesFile(value) = setting {
            // Git expands the values in order once all are loaded, so with no home folder the
            // first `~/` one stops it, even where a later one overrides it.
            if git.home.is_none() && stop.is_none() && from_home(optional(&value).0) {
                stop = Some((value.clone(), place.clone()));
            }
            named = Some((value, place.clone()));
        }
    };
    // Git loads every file and variable before it expands a value, so what it can't load
    // stops it first.
    git.read_files(&mut take)?;
    git.read_variables(&mut take)?;
    let unset = || git.xdg.as_ref().map(|xdg| xdg.join("git").join("ignore"));
    let Some((value, place)) = stop.or(named) else {
        return Ok(unset());
    };
    let (value, optional) = optional(&value);
    let value = utf8(value.to_vec(), "core.excludesFile", &place)?;
    // Git reads no list from an empty setting, which is how one turns it off.
    if value.is_empty() {
        return Ok(if optional { unset() } else { None });
    }
    let path = git_path(
        &value,
        "core.excludesFile",
        git.home.as_deref(),
        Some(git.top),
        &place,
    )?;
    // An optional file that isn't there counts as no setting at all, as in Git.
    let missing = |p: &PathBuf| matches!(std::fs::metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
    if optional && path.as_ref().is_some_and(missing) {
        return Ok(unset());
    }
    Ok(path)
}

/// `value` without Git's `:(optional)` in front, and whether it had one.
fn optional(value: &[u8]) -> (&[u8], bool) {
    match value.strip_prefix(b":(optional)".as_slice()) {
        Some(rest) => (rest, true),
        None => (value, false),
    }
}

/// `git_import_settings` with the places given, so tests can choose them.
fn git_import_settings_in(git: &UserGit) -> Result<UserImportSettings, ConfigError> {
    let mut found = UserImportSettings::default();
    let mut refused = None;
    let (files, sparse) = (&mut found.files, &mut found.sparse_files);
    git.read_files(&mut |setting, place| last(files, sparse, &mut refused, setting, place))?;
    let (variables, sparse) = (&mut found.variables, &mut found.sparse_variables);
    git.read_variables(&mut |setting, place| {
        last(variables, sparse, &mut refused, setting, place)
    })?;
    refused.map_or(Ok(found), Err)
}

/// Keeps `setting`, set at `place`, in `storage` if it is an `lfs.storage`, or in `sparse` if it
/// is a `core.sparseCheckout`; the first of those Git refuses goes in `refused`.
fn last(
    storage: &mut Option<(Vec<u8>, String)>,
    sparse: &mut Option<bool>,
    refused: &mut Option<ConfigError>,
    setting: UserSetting,
    place: &Place,
) {
    match setting {
        UserSetting::LfsStorage(value) => *storage = Some((value, place.to_string())),
        UserSetting::SparseCheckout(Some(on)) => *sparse = Some(on),
        UserSetting::SparseCheckout(None) => {
            refused.get_or_insert_with(|| {
                let detail =
                    "its core.sparseCheckout isn't true or false to Git, which refuses it too";
                place.error(detail.to_owned())
            });
        }
        _ => {}
    }
}

/// Where a setting of the user's own Git settings is set: a file, or the variable of the
/// environment that holds its value.
#[derive(Debug, Clone)]
enum Place {
    File(PathBuf),
    Variable(String),
}

impl Place {
    /// The error for a setting here that `detail` says is wrong.
    fn error(&self, detail: String) -> ConfigError {
        match self {
            Place::File(path) => ConfigError::GitSettings {
                path: path.clone(),
                detail,
            },
            Place::Variable(name) => ConfigError::GitVariable {
                name: name.clone(),
                detail,
            },
        }
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Place::File(path) => path.display().fmt(f),
            Place::Variable(name) => name.fmt(f),
        }
    }
}

/// `value`, the path `setting` is set to at `place`, as text.
fn utf8(value: Vec<u8>, setting: &str, place: &Place) -> Result<String, ConfigError> {
    String::from_utf8(value).map_err(|_| place.error(format!("its {setting} isn't valid UTF-8")))
}

/// How many includes deep Git reads settings, its `MAX_INCLUDE_DEPTH`.
const MAX_INCLUDE_DEPTH: usize = 10;

/// Reads `path`, a file of the user's own Git settings `depth` includes down, giving `each` every
/// setting Bolt reads in it but an include, with the file that holds it. A plain include is read
/// where it stands, as Git reads it (decision 060). A file Bolt can't read is skipped where Git
/// skips it (`skipped`), the user's `own` files apart; anything else stops Bolt, naming it.
fn read_git_settings(
    path: &Path,
    home: Option<&Path>,
    depth: usize,
    own: bool,
    each: &mut dyn FnMut(UserSetting, &Place),
) -> Result<(), ConfigError> {
    let place = Place::File(path.to_owned());
    let text = match safe_fs::read_capped_following_links(path, MAX_GIT_SETTINGS_BYTES) {
        Ok(text) => text,
        Err(e) if skipped(e.kind(), e.raw_os_error(), own, cfg!(windows)) => return Ok(()),
        Err(e) => return Err(place.error(e.to_string())),
    };
    if depth > MAX_INCLUDE_DEPTH {
        return Err(place.error(format!(
            "it is included more than {MAX_INCLUDE_DEPTH} deep, where Git stops too; perhaps an \
             include leads back to it"
        )));
    }
    let settings = read_user_settings(&text)
        .map_err(|line| place.error(format!("line {line} can't be read as Git reads it")))?;
    for setting in settings {
        match setting {
            UserSetting::Include(value) => {
                // A relative include is from the including file's folder.
                let folder = path.parent().unwrap_or(Path::new(""));
                let value = utf8(value, "include.path", &place)?;
                if let Some(included) =
                    git_path(&value, "include.path", home, Some(folder), &place)?
                {
                    read_git_settings(&included, home, depth + 1, false, each)?;
                }
            }
            other => each(other, &place),
        }
    }
    Ok(())
}

/// Whether Git skips a settings file it couldn't read for `kind` (the system's error `raw`), as
/// Bolt does too: one that isn't there, or has a file where a folder on its path should be, and
/// one of the user's `own` that it may not read, but not on Windows (`windows`).
fn skipped(kind: std::io::ErrorKind, raw: Option<i32>, own: bool, windows: bool) -> bool {
    use std::io::ErrorKind::{NotADirectory, NotFound, PermissionDenied};
    // Git skips EACCES, 13 on Linux and macOS, but not EPERM, which Rust reports alike.
    const EACCES: i32 = 13;
    match kind {
        NotFound | NotADirectory => true,
        // Git for Windows' access check doesn't see permissions, so its read fails and Git stops.
        PermissionDenied => own && !windows && raw == Some(EACCES),
        _ => false,
    }
}

/// `GIT_CONFIG_SYSTEM`'s value as Git takes it: tidied as `tidy_path` does, then none when it is
/// nothing or names no file. `Err` where the tidying fails.
fn system_file_named(value: &[u8], windows: bool) -> Result<Option<Vec<u8>>, ()> {
    let tidied = tidy_path(value, windows).ok_or(())?;
    if tidied.is_empty() || names_no_file(&tidied, windows) {
        return Ok(None);
    }
    Ok(Some(tidied))
}

/// `bytes`, a path the environment gave once tidied, as the system's path again: only ASCII was
/// dropped or changed, so any bytes do on Unix, and valid Unicode stays so on Windows.
fn os_path(bytes: Vec<u8>) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes).ok().map(PathBuf::from)
    }
}

/// Whether `value`, a path a Git setting or variable gives, names no file, as Git reads it: the
/// exact text `/dev/null`, and on Windows (`windows`) `nul` in any case, which Git opens as the
/// null device. It is judged on the value as written, before `~/` is expanded or it is joined to
/// a folder, which on Windows would make `/dev/null` a file on the project's drive.
fn names_no_file(value: &[u8], windows: bool) -> bool {
    value == b"/dev/null" || (windows && value.eq_ignore_ascii_case(b"nul"))
}

/// `value`, the path `setting` is set to at `place`, as Git expands it: none for a name for no
/// file; `~` is the `home` folder, and stops where there is none, as Git stops; an absolute path
/// stays as it is; and a relative one is from `folder`. With no folder, as for a variable's
/// include, Git refuses a relative one.
fn git_path(
    value: &str,
    setting: &str,
    home: Option<&Path>,
    folder: Option<&Path>,
    place: &Place,
) -> Result<Option<PathBuf>, ConfigError> {
    if names_no_file(value.as_bytes(), cfg!(windows)) {
        return Ok(None);
    }
    // Git puts the folder it is installed in there.
    if value.starts_with("%(prefix)/") {
        return Err(place.error(format!(
            "Bolt can't expand its {setting}, {value}: %(prefix)/ stands for the folder Git is \
             installed in, which Bolt can't tell; write the full path instead"
        )));
    }
    let Some(rest) = value.strip_prefix('~') else {
        return match folder {
            Some(folder) => Ok(Some(folder.join(value))),
            None if Path::new(value).has_root() => Ok(Some(PathBuf::from(value))),
            None => Err(place.error(format!(
                "its {setting}, `{value}`, is a relative path, which Git takes only from a file \
                 and refuses here too"
            ))),
        };
    };
    // Someone writing `~\x` on Windows most likely meant their own home folder.
    if let Some(after) = rest.strip_prefix('\\') {
        return Err(place.error(format!(
            "Bolt can't expand its {setting}, {value}: only `~/` names your home folder, and Git \
             stops at this too; write `~/{after}` or the whole path instead"
        )));
    }
    // Git looks `~user/` up in the system's accounts, which Bolt doesn't read.
    if !from_home(value.as_bytes()) {
        return Err(place.error(format!(
            "Bolt can't expand its {setting}, {value}, which names a home folder by its user \
             (~user/); write the full path instead"
        )));
    }
    // Git dies here: "failed to expand user dir".
    let Some(home) = home else {
        return Err(place.error(format!(
            "Bolt can't expand its {setting}, {value}: there is no home folder for `~` (HOME \
             isn't set), and Git stops there too; set HOME, or write the full path instead"
        )));
    };
    // Git puts what follows `~` after the home folder, so `~//x` is the home folder's `x`.
    Ok(Some(
        home.join(rest.trim_start_matches(std::path::is_separator)),
    ))
}

/// Whether Git puts the home folder at the start of `value`: `~` alone or before a `/`. Git takes
/// `~\x` as a user's name even on Windows, and stops there.
fn from_home(value: &[u8]) -> bool {
    value.first() == Some(&b'~') && value.get(1).is_none_or(|&c| c == b'/')
}

/// The home folder Git v2.56.0 takes `~` and its own files from: `HOME`, whatever it is set to;
/// on Windows (`windows`), where Git for Windows sets `HOME` when it isn't set
/// (`setup_windows_environment`), `HOMEDRIVE` followed by `HOMEPATH` when that names a folder
/// (`is_dir`), else `USERPROFILE`. `None` where there is none: Git doesn't ask the system.
fn git_home(
    var: &dyn Fn(&str) -> Option<OsString>,
    windows: bool,
    is_dir: &dyn Fn(&Path) -> bool,
) -> Option<OsString> {
    if let Some(home) = var("HOME") {
        return Some(home);
    }
    if !windows {
        return None;
    }
    if let (Some(mut home), Some(path)) = (var("HOMEDRIVE"), var("HOMEPATH")) {
        home.push(path);
        if is_dir(Path::new(&home)) {
            return Some(home);
        }
    }
    var("USERPROFILE")
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
        && let Some(identity) =
            from_file(path, &safe_fs::read_capped_following_links, project_root)?
    {
        return Ok(identity);
    }
    Err(ConfigError::MissingIdentity { path: user_config })
}

/// The identity the project's own `.bolt/config.toml` names, if Bolt did not use it (it differs
/// from the one `load_identity` gives, which is none if that fails). Read like every file in
/// `.bolt`: through the `.bolt` the project holds open, never through a link, at most 1 MiB; one
/// that can't be read names nobody.
#[must_use]
pub fn ignored_project_identity(repo: &Repository) -> Option<Identity> {
    let used = load_identity(Some(repo.root())).ok();
    let bolt = repo.root().join(".bolt");
    ignored_given(repo.bolt_folder(), &bolt, used.as_ref())
}

/// `ignored_project_identity`, given the project's `.bolt` held open (at `bolt`) and the identity
/// in use.
fn ignored_given(held: &Dir, bolt: &Path, used: Option<&Identity>) -> Option<Identity> {
    let name = OsStr::new("config.toml");
    let read = |shown: &Path, max| safe_fs::read_capped_in(held, name, shown, max);
    let named = from_file(&bolt.join(name), &read, None).ok().flatten()?;
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
    let Some(file) = users_file(path, "safe-folders")? else {
        return Ok(Vec::new());
    };
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

/// The user's own `config.toml` at `path`, for `key` of `AT_TOP`: refused if `key` is also
/// written below a `[section]`.
fn users_file(path: &Path, key: &'static str) -> Result<Option<ConfigFile>, ConfigError> {
    let file = read_file(path, &safe_fs::read_capped_following_links)?;
    if file.as_ref().is_some_and(|f| f.misplaced.contains(&key)) {
        return Err(ConfigError::NotAtTop {
            key,
            path: path.to_owned(),
        });
    }
    Ok(file)
}

/// How many days of steps `tidy` keeps what undo and redo need for: `undo-window-days` at the
/// top of the user's own `config.toml`, a whole number from 1 to 36,500, and 30 when it isn't set.
pub fn undo_window_days() -> Result<u32, ConfigError> {
    user_config_path().map_or(Ok(UNDO_WINDOW_DAYS), |path| undo_window_days_in(&path))
}

/// `undo_window_days`, from the user's `config.toml` at `path`.
fn undo_window_days_in(path: &Path) -> Result<u32, ConfigError> {
    let Some(value) = users_file(path, "undo-window-days")?.and_then(|f| f.undo_window_days) else {
        return Ok(UNDO_WINDOW_DAYS);
    };
    let days = match &value {
        toml::Value::Integer(n) => u32::try_from(*n).ok(),
        _ => None,
    };
    days.filter(|d| (1..=MAX_UNDO_WINDOW_DAYS).contains(d))
        .ok_or_else(|| ConfigError::Invalid {
            path: path.to_owned(),
            detail: format!(
                "undo-window-days must be a whole number of days from 1 to \
                 {MAX_UNDO_WINDOW_DAYS}, not {value}"
            ),
        })
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
pub(crate) fn others_can_write(folder: &Path) -> bool {
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
        let read = |path: &Path| from_file(path, &safe_fs::read_capped, None);
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
    fn the_users_own_git_ignore_list_is_found_as_git_finds_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let xdg = home.join("xdg");
        let root = home.join("project");
        let find = || excludes(home, &xdg, &root, &[]).unwrap();
        // Nothing set: git/ignore in XDG_CONFIG_HOME.
        assert_eq!(find(), Some(xdg.join("git").join("ignore")));
        // A relative path is from the project's top, as in Git.
        std::fs::create_dir_all(xdg.join("git")).unwrap();
        std::fs::write(
            xdg.join("git").join("config"),
            "[core]\n\texcludesFile = list\n",
        )
        .unwrap();
        assert_eq!(find(), Some(root.join("list")));
        // ~/.gitconfig is read after it, so it wins, and `~/` is the home folder.
        std::fs::write(home.join(".gitconfig"), "[core]\n\texcludesFile = ~/mine\n").unwrap();
        assert_eq!(find(), Some(home.join("mine")));
        // Set to nothing, it names no list, as in Git.
        std::fs::write(home.join(".gitconfig"), "[core]\n\texcludesFile =\n").unwrap();
        assert_eq!(find(), None);
    }

    #[test]
    fn git_settings_bolt_cant_read_stop_it_and_are_named() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::write(home.join(".gitconfig"), "[user]\n[core\n").unwrap();
        let e = excludes(home, &home.join("xdg"), home, &[]).unwrap_err();
        assert!(matches!(e, ConfigError::GitSettings { .. }), "{e}");
        let said = e.to_string();
        assert!(
            said.contains(".gitconfig") && said.contains("line 2"),
            "{said}"
        );
        assert!(e.hint().unwrap().contains("core.excludesFile"));
    }

    /// Plain includes are read where they stand, as Git reads them (decision 060).
    #[test]
    fn includes_in_the_users_git_settings_are_followed() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let xdg = home.join("xdg");
        let find = || excludes(home, &xdg, &home.join("project"), &[]).unwrap();
        let set = |path: &Path, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        let gitconfig = home.join(".gitconfig");
        set(
            &home.join(".gitconfig.local"),
            "[core]\n\texcludesFile = ~/mine\n",
        );
        set(&gitconfig, "[include]\n\tpath = ~/.gitconfig.local\n");
        assert_eq!(find(), Some(home.join("mine")));
        // What an include sets counts at its place: a later line wins over it, and it wins over
        // an earlier one.
        let after = "[include]\n\tpath = ~/.gitconfig.local\n[core]\n\texcludesFile = ~/after\n";
        set(&gitconfig, after);
        assert_eq!(find(), Some(home.join("after")));
        let before = "[core]\n\texcludesFile = ~/before\n[include]\n\tpath = ~/.gitconfig.local\n";
        set(&gitconfig, before);
        assert_eq!(find(), Some(home.join("mine")));
        // A relative path is from the including file's folder.
        set(&gitconfig, "");
        set(
            &xdg.join("git").join("config"),
            "[include]\n\tpath = more\n",
        );
        set(
            &xdg.join("git").join("more"),
            "[core]\n\texcludesFile = ~/xdg\n",
        );
        set(&home.join("more"), "[core]\n\texcludesFile = ~/wrong\n");
        assert_eq!(find(), Some(home.join("xdg")));
        // A file that isn't there is skipped, as Git skips it.
        set(
            &xdg.join("git").join("config"),
            "[include]\n\tpath = gone\n",
        );
        assert_eq!(find(), Some(xdg.join("git").join("ignore")));
        // An `[includeIf]` needs the repository to decide, so it isn't followed.
        let only_if = "[includeIf \"gitdir:~/\"]\n\tpath = more\n";
        set(&xdg.join("git").join("config"), only_if);
        assert_eq!(find(), Some(xdg.join("git").join("ignore")));
    }

    #[test]
    fn includes_stop_ten_deep_as_in_git() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let find = || excludes(home, &home.join("xdg"), home, &[]);
        // `.gitconfig` includes `1`, which includes `2`, and so on; the last one sets the list.
        let chain = |deepest: u32| {
            std::fs::write(home.join(".gitconfig"), "[include]\n\tpath = 1\n").unwrap();
            for n in 1..deepest {
                let next = format!("[include]\n\tpath = {}\n", n + 1);
                std::fs::write(home.join(n.to_string()), next).unwrap();
            }
            let last = "[core]\n\texcludesFile = ~/list\n";
            std::fs::write(home.join(deepest.to_string()), last).unwrap();
        };
        chain(10);
        assert_eq!(find().unwrap(), Some(home.join("list")));
        chain(11);
        match find() {
            Err(ConfigError::GitSettings { path, .. }) => assert_eq!(path, home.join("11")),
            other => panic!("{other:?}"),
        }
        // A loop goes as deep as Git allows, then stops.
        std::fs::write(home.join(".gitconfig"), "[include]\n\tpath = .gitconfig\n").unwrap();
        let e = find().unwrap_err();
        assert!(matches!(e, ConfigError::GitSettings { .. }), "{e}");
        assert!(e.to_string().contains("10"), "{e}");
    }

    #[test]
    fn a_home_folder_path_is_expanded_as_git_expands_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let find = |text: &str| {
            std::fs::write(home.join(".gitconfig"), text).unwrap();
            excludes(home, &home.join("xdg"), &home.join("project"), &[])
        };
        let list = |value: &str| find(&format!("[core]\n\texcludesFile = {value}\n"));
        // Git puts what follows `~` after the home folder, so more slashes change nothing.
        assert_eq!(list("~//x").unwrap(), Some(home.join("x")));
        assert_eq!(list("~").unwrap(), Some(home.to_owned()));
        // Git looks other users up in the system's accounts; Bolt asks for the full path.
        for text in [
            "[core]\n\texcludesFile = ~alice/x\n",
            "[include]\n\tpath = ~alice/x\n",
        ] {
            let e = find(text).unwrap_err();
            assert!(matches!(e, ConfigError::GitSettings { .. }), "{e}");
            let said = e.to_string();
            assert!(
                said.contains("~alice/x") && said.contains("full path"),
                "{said}"
            );
        }
    }

    /// Reads the user's own Git settings with `read`, as Git finds them with this home folder,
    /// `XDG_CONFIG_HOME` and project top, and only `vars` set in the environment, so no test
    /// reads the user's own.
    fn as_git_finds<T>(
        (home, xdg, top): (&Path, &Path, &Path),
        vars: &[(&str, &str)],
        read: fn(&UserGit) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let places = (Some(home), Some(xdg), top);
        as_git_finds_where(places, vars, read)
    }

    /// `as_git_finds`, where there may be no home folder, or no `XDG_CONFIG_HOME` folder.
    fn as_git_finds_where<T>(
        (home, xdg, top): (Option<&Path>, Option<&Path>, &Path),
        vars: &[(&str, &str)],
        read: fn(&UserGit) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let var = |key: &str| {
            let (_, value) = vars.iter().find(|(name, _)| *name == key)?;
            Some(OsString::from(value))
        };
        read(&UserGit {
            home: home.map(Path::to_owned),
            xdg: xdg.map(Path::to_owned),
            top,
            var: &var,
        })
    }

    fn excludes(
        home: &Path,
        xdg: &Path,
        top: &Path,
        vars: &[(&str, &str)],
    ) -> Result<Option<PathBuf>, ConfigError> {
        as_git_finds((home, xdg, top), vars, git_excludes_file_in)
    }

    /// A file of Git settings at `path` whose `core.excludesFile` is `list`.
    fn excludes_file_set(path: &Path, list: &str) -> String {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("[core]\n\texcludesFile = {list}\n")).unwrap();
        path.display().to_string()
    }

    /// The variable an error names, which must be one of the environment's.
    fn variable_named(e: ConfigError) -> String {
        assert!(e.hint().unwrap().contains("unset it"), "{e}");
        match e {
            ConfigError::GitVariable { name, .. } => name,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_system_file_comes_first_unless_git_config_nosystem_is_true() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (xdg, top) = (home.join("xdg"), home.join("project"));
        let system = excludes_file_set(&home.join("etc/gitconfig"), "~/system");
        let find = |vars: &[(&str, &str)]| excludes(home, &xdg, &top, vars);
        let unset = Some(xdg.join("git").join("ignore"));
        // Only the file `GIT_CONFIG_SYSTEM` names: Git's own place depends on how it was built.
        assert_eq!(find(&[]).unwrap(), unset);
        let named = [("GIT_CONFIG_SYSTEM", system.as_str())];
        assert_eq!(find(&named).unwrap(), Some(home.join("system")));
        // A relative one is from the project's top, where Git runs; set to nothing, it names no
        // file, as in Git.
        excludes_file_set(&top.join("etc/gitconfig"), "~/relative");
        let relative = [("GIT_CONFIG_SYSTEM", "etc/gitconfig")];
        assert_eq!(find(&relative).unwrap(), Some(home.join("relative")));
        assert_eq!(find(&[("GIT_CONFIG_SYSTEM", "")]).unwrap(), unset);
        // `GIT_CONFIG_NOSYSTEM` is read as Git reads a true or false value.
        for (value, skipped) in [
            ("1", true),
            ("true", true),
            ("YES", true),
            ("on", true),
            ("2", true),
            ("", false),
            ("0", false),
            ("false", false),
            ("Off", false),
        ] {
            let vars = [named[0], ("GIT_CONFIG_NOSYSTEM", value)];
            let want = if skipped {
                unset.clone()
            } else {
                Some(home.join("system"))
            };
            assert_eq!(find(&vars).unwrap(), want, "{value:?}");
        }
        // Git stops at a value it can't read, whether or not `GIT_CONFIG_SYSTEM` is set.
        let e = find(&[("GIT_CONFIG_NOSYSTEM", "maybe")]).unwrap_err();
        assert!(e.to_string().contains("`maybe`"), "{e}");
        assert_eq!(variable_named(e), "GIT_CONFIG_NOSYSTEM");
        // The user's own files come after it, so they win.
        excludes_file_set(&home.join(".gitconfig"), "~/mine");
        assert_eq!(find(&named).unwrap(), Some(home.join("mine")));
    }

    #[test]
    fn git_config_global_takes_the_place_of_the_home_files() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (xdg, top) = (home.join("xdg"), home.join("project"));
        excludes_file_set(&xdg.join("git").join("config"), "~/xdg");
        excludes_file_set(&home.join(".gitconfig"), "~/home");
        let find = |vars: &[(&str, &str)]| excludes(home, &xdg, &top, vars);
        assert_eq!(find(&[]).unwrap(), Some(home.join("home")));
        let global = excludes_file_set(&home.join("global"), "~/global");
        assert_eq!(
            find(&[("GIT_CONFIG_GLOBAL", global.as_str())]).unwrap(),
            Some(home.join("global"))
        );
        // Neither home file is read, even when the file named sets nothing, isn't there, is
        // nothing at all or is `/dev/null`, as Git's own documentation suggests.
        std::fs::write(home.join("empty"), "[user]\n\tname = x\n").unwrap();
        let empty = home.join("empty").display().to_string();
        let gone = home.join("gone").display().to_string();
        for named in [empty.as_str(), gone.as_str(), "", "/dev/null"] {
            let vars = [("GIT_CONFIG_GLOBAL", named)];
            let want = Some(xdg.join("git").join("ignore"));
            assert_eq!(find(&vars).unwrap(), want, "{named:?}");
        }
        // The system's file comes before it.
        let system = excludes_file_set(&home.join("system"), "~/system");
        let both = |global: &str| {
            find(&[
                ("GIT_CONFIG_SYSTEM", system.as_str()),
                ("GIT_CONFIG_GLOBAL", global),
            ])
            .unwrap()
        };
        assert_eq!(both(&empty), Some(home.join("system")));
        assert_eq!(both(&global), Some(home.join("global")));
        let dev_null = [
            ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ];
        assert_eq!(
            find(&dev_null).unwrap(),
            Some(xdg.join("git").join("ignore"))
        );
    }

    #[test]
    fn git_config_count_gives_settings_after_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (xdg, top) = (home.join("xdg"), home.join("project"));
        let system = excludes_file_set(&home.join("system"), "~/system");
        let global = excludes_file_set(&home.join("global"), "~/global");
        let find = |given: &[(&str, &str)]| {
            let mut vars = vec![("GIT_CONFIG_SYSTEM", system.as_str())];
            vars.extend([("GIT_CONFIG_GLOBAL", global.as_str())]);
            vars.extend(given);
            excludes(home, &xdg, &top, &vars)
        };
        assert_eq!(find(&[]).unwrap(), Some(home.join("global")));
        // In their order, the last winning, whatever other settings are given; the section and
        // the key are read in any letter case, as in Git, but a subsection is another setting.
        let given = [
            ("GIT_CONFIG_COUNT", "3"),
            ("GIT_CONFIG_KEY_0", "core.excludesFile"),
            ("GIT_CONFIG_VALUE_0", "~/first"),
            ("GIT_CONFIG_KEY_1", "CORE.EXCLUDESFILE"),
            ("GIT_CONFIG_VALUE_1", "~/second"),
            ("GIT_CONFIG_KEY_2", "core.Sub.excludesFile"),
            ("GIT_CONFIG_VALUE_2", "~/third"),
        ];
        assert_eq!(find(&given).unwrap(), Some(home.join("second")));
        // Only as many as the count says; nothing set is none, a relative path is from the
        // project's top, and set to nothing, it names no list.
        let count = |count: &'static str, value: &'static str| {
            find(&[
                ("GIT_CONFIG_COUNT", count),
                ("GIT_CONFIG_KEY_0", "core.excludesFile"),
                ("GIT_CONFIG_VALUE_0", value),
            ])
            .unwrap()
        };
        assert_eq!(count("0", "~/first"), Some(home.join("global")));
        assert_eq!(count("", "~/first"), Some(home.join("global")));
        assert_eq!(count("-0", "~/first"), Some(home.join("global")));
        assert_eq!(count(" \t+1", "list"), Some(top.join("list")));
        assert_eq!(count("1", ""), None);
        // An include there is followed, from a full path only, as Git follows it.
        let included = excludes_file_set(&home.join("included"), "~/included");
        let include = |path: &str| {
            let vars = [
                ("GIT_CONFIG_COUNT", "1"),
                ("GIT_CONFIG_KEY_0", "include.path"),
                ("GIT_CONFIG_VALUE_0", path),
            ];
            find(&vars)
        };
        assert_eq!(include(&included).unwrap(), Some(home.join("included")));
        assert_eq!(include("~/included").unwrap(), Some(home.join("included")));
        let e = include("included").unwrap_err();
        assert!(e.to_string().contains("relative path"), "{e}");
        assert_eq!(variable_named(e), "GIT_CONFIG_VALUE_0");
    }

    #[test]
    fn what_git_refuses_in_git_config_count_stops_bolt_and_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let find = |vars: &[(&str, &str)]| excludes(home, &home.join("xdg"), home, vars);
        for count in [
            "x",
            "1 ",
            "0x1",
            " ",
            "+",
            "1k",
            "-1",
            "2147483648",
            "99999999999999999999",
        ] {
            let e = find(&[("GIT_CONFIG_COUNT", count)]).unwrap_err();
            assert!(e.to_string().contains(&format!("`{count}`")), "{e}");
            assert_eq!(variable_named(e), "GIT_CONFIG_COUNT", "{count:?}");
        }
        let count = |n| ("GIT_CONFIG_COUNT", n);
        let (key, value) = (("GIT_CONFIG_KEY_0", "a.b"), ("GIT_CONFIG_VALUE_0", "x"));
        // As many keys and values as the count says must be set.
        for (vars, missing) in [
            (vec![count("2")], "GIT_CONFIG_KEY_0"),
            (vec![count("1"), key], "GIT_CONFIG_VALUE_0"),
            (vec![count("2"), key, value], "GIT_CONFIG_KEY_1"),
        ] {
            let e = find(&vars).unwrap_err();
            assert!(e.to_string().contains("isn't set"), "{e}");
            assert_eq!(variable_named(e), missing);
        }
        // Each key must be one Git takes, whatever setting it names.
        for name in [
            "",
            "core",
            ".x",
            "core.",
            "core.1x",
            "co_re.x",
            "a.b\nc.d",
            "user.na me",
        ] {
            let e = find(&[count("1"), ("GIT_CONFIG_KEY_0", name), value]).unwrap_err();
            assert_eq!(variable_named(e), "GIT_CONFIG_KEY_0", "{name:?}");
        }
        for name in ["-.x", ".a.b", "1.x", "a.b c\t.d"] {
            let vars = [count("1"), ("GIT_CONFIG_KEY_0", name), value];
            assert!(find(&vars).is_ok(), "{name:?}");
        }
    }

    #[test]
    fn lfs_storage_is_found_in_the_files_and_in_the_variables_apart() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let set = |path: &Path, value: &str| {
            std::fs::write(path, format!("[lfs]\n\tstorage = {value}\n")).unwrap();
            path.display().to_string()
        };
        let xdg = home.join("xdg");
        let find = |vars: &[(&str, &str)]| {
            as_git_finds((home, &xdg, home), vars, git_import_settings_in).unwrap()
        };
        let system = set(&home.join("system"), "/system");
        let gitconfig = set(&home.join(".gitconfig"), "/home");
        let found = find(&[("GIT_CONFIG_SYSTEM", system.as_str())]);
        assert_eq!(found.files, Some((b"/home".to_vec(), gitconfig)));
        assert_eq!(found.variables, None);
        let found = find(&[
            ("GIT_CONFIG_SYSTEM", system.as_str()),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "lfs.storage"),
            ("GIT_CONFIG_VALUE_0", ""),
        ]);
        // Named as Git tidies it: on Windows with `/`.
        let tidied = if cfg!(windows) {
            system.replace('\\', "/")
        } else {
            system
        };
        assert_eq!(found.files, Some((b"/system".to_vec(), tidied)));
        let variable = "GIT_CONFIG_VALUE_0".to_owned();
        assert_eq!(found.variables, Some((Vec::new(), variable)));
    }

    /// `/dev/null`, and `nul` in any case on Windows, names no file wherever a Git setting or
    /// variable names one, judged as written: joined to a folder on Windows, `/dev/null` would be
    /// `C:\dev\null`, which anyone may make.
    #[test]
    fn a_name_for_no_file_is_judged_before_it_is_joined_to_a_folder() {
        for (value, on_windows, elsewhere) in [
            ("/dev/null", true, true),
            ("nul", true, false),
            ("NUL", true, false),
            ("Nul", true, false),
            ("C:\\dev\\null", false, false),
            ("C:/dev/null", false, false),
            ("\\dev\\null", false, false),
            ("//dev/null", false, false),
            ("/dev/null/", false, false),
            ("/DEV/NULL", false, false),
            ("nul.txt", false, false),
            ("C:\\x\\nul", false, false),
            ("", false, false),
        ] {
            assert_eq!(
                names_no_file(value.as_bytes(), true),
                on_windows,
                "{value:?}"
            );
            assert_eq!(
                names_no_file(value.as_bytes(), false),
                elsewhere,
                "{value:?}"
            );
        }
        // Where the project's top is on a drive, no name for no file becomes a path on it.
        let top = Path::new("C:\\project");
        let files = |name: &'static str| {
            let vars = [("GIT_CONFIG_SYSTEM", name), ("GIT_CONFIG_GLOBAL", name)];
            let places = (Path::new("home"), Path::new("xdg"), top);
            as_git_finds(places, &vars, |git| git.files()).unwrap()
        };
        let place = Place::Variable("GIT_CONFIG_VALUE_0".into());
        let path = |value| {
            git_path(
                value,
                "include.path",
                Some(Path::new("home")),
                Some(top),
                &place,
            )
        };
        assert_eq!(files("/dev/null"), []);
        assert_eq!(path("/dev/null").unwrap(), None);
        for name in ["nul", "NUL"] {
            let want_files = if cfg!(windows) {
                vec![]
            } else {
                vec![(top.join(name), false), (top.join(name), true)]
            };
            assert_eq!(files(name), want_files, "{name}");
            let want_path = (!cfg!(windows)).then(|| top.join(name));
            assert_eq!(path(name).unwrap(), want_path, "{name}");
        }
        // So `core.excludesFile = /dev/null` is no list, and an include of it no include.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let find = |text: &str| {
            std::fs::write(home.join(".gitconfig"), text).unwrap();
            excludes(home, &home.join("xdg"), &home.join("project"), &[])
        };
        assert_eq!(find("[core]\n\texcludesFile = /dev/null\n").unwrap(), None);
        let included = "[core]\n\texcludesFile = ~/mine\n[include]\n\tpath = /dev/null\n";
        assert_eq!(find(included).unwrap(), Some(home.join("mine")));
        let named = if cfg!(windows) {
            None
        } else {
            Some(home.join("project").join("NUL"))
        };
        assert_eq!(find("[core]\n\texcludesFile = NUL\n").unwrap(), named);
    }

    /// As in Git, a settings file is skipped when a file stands where a folder on its path should
    /// be, and the user's own when Bolt may not read it; anything else that keeps one from being
    /// read stops Bolt, a folder or a device included.
    #[test]
    fn settings_files_git_skips_are_skipped_and_others_stop() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let xdg = home.join("xdg");
        std::fs::write(home.join("plain"), "not a folder").unwrap();
        let under_a_file = home.join("plain").join("config").display().to_string();
        let find = |vars: &[(&str, &str)]| excludes(home, &xdg, home, vars);
        let unset = Some(xdg.join("git").join("ignore"));
        for name in ["GIT_CONFIG_SYSTEM", "GIT_CONFIG_GLOBAL"] {
            assert_eq!(
                find(&[(name, under_a_file.as_str())]).unwrap(),
                unset,
                "{name}"
            );
        }
        let include = format!(
            "[include]\n\tpath = {}\n",
            home.join("plain").join("x").display()
        );
        std::fs::write(home.join("included"), include.replace('\\', "/")).unwrap();
        let included = home.join("included").display().to_string();
        assert_eq!(
            find(&[("GIT_CONFIG_GLOBAL", included.as_str())]).unwrap(),
            unset
        );
        let folder = home.display().to_string();
        let e = find(&[("GIT_CONFIG_GLOBAL", folder.as_str())]).unwrap_err();
        assert!(matches!(e, ConfigError::GitSettings { .. }), "{e}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for device in ["/dev/zero", "//dev/null"] {
                let e = find(&[("GIT_CONFIG_GLOBAL", device)]).unwrap_err();
                assert!(
                    matches!(e, ConfigError::GitSettings { .. }),
                    "{device}: {e}"
                );
            }
            let locked = home.join("locked");
            std::fs::write(&locked, "[core]\n\texcludesFile = ~/locked-list\n").unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            // Whoever may read it anyway (root) can't show what Git skips.
            if std::fs::read(&locked).is_err() {
                let locked = locked.display().to_string();
                assert_eq!(
                    find(&[("GIT_CONFIG_GLOBAL", locked.as_str())]).unwrap(),
                    unset
                );
                let e = find(&[("GIT_CONFIG_SYSTEM", locked.as_str())]).unwrap_err();
                assert!(matches!(e, ConfigError::GitSettings { .. }), "{e}");
                let include = format!("[include]\n\tpath = {locked}\n");
                std::fs::write(home.join(".gitconfig"), include).unwrap();
                let e = find(&[]).unwrap_err();
                assert!(matches!(e, ConfigError::GitSettings { .. }), "{e}");
            }
        }
    }

    /// Git skips the user's own settings file it may not read, but Git for Windows can't tell that
    /// before it reads one, and stops there; Bolt does as each does, so it never applies other
    /// settings than Git.
    #[test]
    fn a_settings_file_is_skipped_where_git_skips_it() {
        use std::io::ErrorKind::{InvalidInput, NotADirectory, NotFound, PermissionDenied};
        // EACCES and EPERM, as Linux and macOS number them.
        let (eacces, eperm) = (Some(13), Some(1));
        for (kind, raw, own, windows, want) in [
            (NotFound, None, false, false, true),
            (NotFound, Some(2), true, true, true),
            (NotADirectory, Some(20), false, true, true),
            (NotADirectory, None, true, false, true),
            (PermissionDenied, eacces, true, false, true),
            (PermissionDenied, eacces, true, true, false),
            (PermissionDenied, eacces, false, false, false),
            (PermissionDenied, eacces, false, true, false),
            // Git skips EACCES only; Rust reports EPERM as the same kind.
            (PermissionDenied, eperm, true, false, false),
            (PermissionDenied, None, true, false, false),
            (InvalidInput, None, true, false, false),
            (InvalidInput, None, false, true, false),
        ] {
            let said = format!("{kind:?} ({raw:?}), own: {own}, on Windows: {windows}");
            assert_eq!(skipped(kind, raw, own, windows), want, "{said}");
        }
    }

    /// `GIT_CONFIG_SYSTEM` names no file where Git opens none.
    #[test]
    fn git_config_system_names_no_file_where_git_reads_none() {
        // So these name no file on Windows, as they do to Git there.
        let named = |text: &str, windows| system_file_named(text.as_bytes(), windows);
        for text in ["\\dev\\null", "/dev//null", "/dev/./null", "/x/../dev/null"] {
            assert_eq!(named(text, true), Ok(None), "{text:?}");
        }
        assert_eq!(named("/dev//null", false), Ok(None));
        assert_eq!(named("dir/..", true), Ok(None));
        let kept = named("C:\\Git\\etc\\gitconfig", true);
        assert_eq!(kept, Ok(Some(b"C:/Git/etc/gitconfig".to_vec())));
        assert_eq!(named("../gitconfig", false), Err(()));
        // Bytes that aren't UTF-8 pass as they are, where the system takes them.
        let latin =
            named("/opt/caf\u{e9}/gitconfig", false).map(|path| path.map(String::from_utf8));
        assert_eq!(latin, Ok(Some(Ok("/opt/caf\u{e9}/gitconfig".to_owned()))));
        assert_eq!(
            system_file_named(b"/opt/\xe9/x", false),
            Ok(Some(b"/opt/\xe9/x".to_vec()))
        );
    }

    /// The tidied file is the one read; where the tidying fails, the variable is named.
    #[test]
    fn git_config_system_names_what_git_reads() {
        let top = Path::new("C:\\project");
        let files = |vars: &[(&str, &str)]| {
            let places = (Path::new("home"), Path::new("xdg"), top);
            as_git_finds(places, vars, |git| git.files())
        };
        let system = |name| files(&[("GIT_CONFIG_SYSTEM", name), ("GIT_CONFIG_GLOBAL", "")]);
        assert_eq!(system("/dev//null").unwrap(), []);
        assert_eq!(
            system("etc/./gitconfig").unwrap(),
            [(top.join("etc/gitconfig"), false)]
        );
        // `\` separates only on Windows; elsewhere it is part of a name.
        let want = if cfg!(windows) {
            vec![]
        } else {
            vec![(PathBuf::from("C:\\project/\\dev\\null"), false)]
        };
        assert_eq!(system("\\dev\\null").unwrap(), want);
        // A relative one needs a full path; an absolute one, only to lose its `..`.
        for (value, fix) in [
            ("../gitconfig", "write the full path instead"),
            ("/../gitconfig", "write it without the `..`"),
        ] {
            let e = system(value).unwrap_err();
            let said = e.to_string();
            let goes = format!("`{value}` goes above its start with `..`");
            assert!(said.contains(&goes) && said.ends_with(fix), "{said}");
            assert_eq!(variable_named(e), "GIT_CONFIG_SYSTEM");
        }
        // Git reads no system file under GIT_CONFIG_NOSYSTEM, however it is spelled.
        let unread = [
            ("GIT_CONFIG_SYSTEM", "../gitconfig"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", ""),
        ];
        assert_eq!(files(&unread).unwrap(), []);
    }

    /// The environment `vars` gives, as `std::env::var_os` would read it.
    fn lookup<'v>(vars: &'v [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'v {
        |key: &str| {
            let (_, value) = vars.iter().find(|(name, _)| *name == key)?;
            Some(OsString::from(value))
        }
    }

    /// The home folder is `HOME`, whatever it holds; on Unix nothing else, and on Windows, where
    /// Git for Windows fills it in, `HOMEDRIVE` and `HOMEPATH` when they name a folder, else
    /// `USERPROFILE`. Git never asks the system for one.
    #[test]
    fn the_home_folder_is_found_as_git_finds_it() {
        let find = |vars: &[(&str, &str)], windows: bool, folders: &[&str]| {
            let is_dir = |path: &Path| folders.iter().any(|folder| Path::new(folder) == path);
            git_home(&lookup(vars), windows, &is_dir)
        };
        let home = |text: &str| Some(OsString::from(text));
        let (drive, path) = (("HOMEDRIVE", "H:"), ("HOMEPATH", "\\me"));
        let profile = ("USERPROFILE", "C:\\Users\\me");
        let all = [("HOME", "/h"), drive, path, profile];
        for windows in [false, true] {
            assert_eq!(find(&all, windows, &["H:\\me"]), home("/h"), "{windows}");
            assert_eq!(find(&[("HOME", "")], windows, &[]), home(""), "{windows}");
        }
        let empty_first = [("HOME", ""), drive, path, profile];
        assert_eq!(find(&empty_first, true, &["H:\\me"]), home(""));
        // On Unix, `HOME` alone.
        assert_eq!(find(&all[1..], false, &["H:\\me"]), None);
        assert_eq!(find(&[], false, &[]), None);
        // On Windows, the home drive's folder when it is one, else the profile.
        assert_eq!(find(&all[1..], true, &["H:\\me"]), home("H:\\me"));
        assert_eq!(find(&all[1..], true, &[]), home("C:\\Users\\me"));
        assert_eq!(
            find(&[drive, profile], true, &["H:"]),
            home("C:\\Users\\me")
        );
        assert_eq!(
            find(&[path, profile], true, &["\\me"]),
            home("C:\\Users\\me")
        );
        assert_eq!(find(&[drive, path], true, &[]), None);
        assert_eq!(find(&[], true, &[]), None);
    }

    /// Without a home folder Git reads no `~/.gitconfig` and no `~/.config/git` file, but still
    /// reads `XDG_CONFIG_HOME`'s and what the variables give, and stops at a `~/` path.
    #[test]
    fn without_a_home_folder_only_what_git_reads_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        let xdg = top.join("xdg");
        let no_folder = |_: &Path| false;
        let places = |vars: &[(&str, &str)], windows: bool| {
            let var = lookup(vars);
            let git = UserGit::new(top, &var, windows, &no_folder);
            (git.home.clone(), git.xdg.clone(), git.files().unwrap())
        };
        // Neither: no file at all, on either system.
        for windows in [false, true] {
            assert_eq!(places(&[], windows), (None, None, vec![]), "{windows}");
        }
        let xdg_text = xdg.display().to_string();
        let only_xdg = [("XDG_CONFIG_HOME", xdg_text.as_str())];
        let own = vec![(xdg.join("git").join("config"), true)];
        assert_eq!(places(&only_xdg, false), (None, Some(xdg.clone()), own));
        // An empty HOME is Git's `/`: `~/.gitconfig` is `/.gitconfig`.
        let root = PathBuf::from("/");
        let (home, xdg_folder, files) = places(&[("HOME", "")], false);
        assert_eq!(
            (home, xdg_folder),
            (Some(root.clone()), Some(root.join(".config")))
        );
        let want = [root.join(".config/git/config"), root.join(".gitconfig")];
        assert_eq!(files, want.map(|path| (path, true)));
        // The list: `XDG_CONFIG_HOME`'s default, a full path, and `~/`, which stops.
        let find = |vars: &[(&str, &str)]| {
            as_git_finds_where((None, Some(&xdg), top), vars, git_excludes_file_in)
        };
        assert_eq!(find(&[]).unwrap(), Some(xdg.join("git").join("ignore")));
        let full = top.join("mine").display().to_string().replace('\\', "/");
        let list = excludes_file_set(&top.join("list-settings"), &full);
        let named = [("GIT_CONFIG_GLOBAL", list.as_str())];
        assert_eq!(find(&named).unwrap(), Some(PathBuf::from(&full)));
        let config = excludes_file_set(&xdg.join("git").join("config"), "~/old.ignore");
        // Git stops at the first `~/` value, even with a full path set after it...
        let set_later = [
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "core.excludesFile"),
            ("GIT_CONFIG_VALUE_0", full.as_str()),
        ];
        for later in [&[][..], &set_later[..]] {
            let said = find(later).unwrap_err().to_string();
            assert!(
                said.contains(&config) && said.contains("~/old.ignore: there is no home folder"),
                "{said}"
            );
        }
        // ...but only once everything has loaded: a variable it can't load stops it first.
        assert_eq!(
            variable_named(find(&set_later[..1]).unwrap_err()),
            "GIT_CONFIG_KEY_0"
        );
        let given = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "core.excludesFile"),
            ("GIT_CONFIG_VALUE_0", "~/mine"),
        ];
        assert_eq!(
            variable_named(find(&given).unwrap_err()),
            "GIT_CONFIG_VALUE_0"
        );
        let no_list = as_git_finds_where((None, None, top), &[], git_excludes_file_in);
        assert_eq!(no_list.unwrap(), None);
    }

    /// Git puts the folder it is installed in for `%(prefix)/`, which Bolt can't tell.
    #[test]
    fn an_include_from_where_git_is_installed_stops_and_asks_for_the_full_path() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let vars = [
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "include.path"),
            ("GIT_CONFIG_VALUE_0", "%(prefix)/etc/gitconfig"),
        ];
        let e = excludes(home, &home.join("xdg"), home, &vars).unwrap_err();
        let said = e.to_string();
        assert!(
            said.contains(
                "include.path, %(prefix)/etc/gitconfig: %(prefix)/ stands for the folder"
            ) && said.contains("installed in, which Bolt can't tell; write the full path"),
            "{said}"
        );
        assert!(!said.contains("refuses"), "{said}");
        assert_eq!(variable_named(e), "GIT_CONFIG_VALUE_0");
    }

    #[test]
    fn a_prefix_path_stops_wherever_it_appears() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let gitconfig = home.join(".gitconfig");
        for list in ["%(prefix)/ignore", ":(optional)%(prefix)/ignore"] {
            excludes_file_set(&gitconfig, list);
            let said = excludes(home, &home.join("xdg"), home, &[])
                .unwrap_err()
                .to_string();
            assert!(said.contains("%(prefix)/ stands for the folder"), "{said}");
        }
        std::fs::write(&gitconfig, "[include]\n\tpath = %(prefix)/etc/gitconfig\n").unwrap();
        let said = excludes(home, &home.join("xdg"), home, &[])
            .unwrap_err()
            .to_string();
        assert!(said.contains("%(prefix)/ stands for the folder"), "{said}");
    }

    /// Only `~/` is the home folder: Git for Windows 2.50.1 says "failed to expand user dir in:
    /// '~\ign'", as it says for `~user/`.
    #[test]
    fn tilde_backslash_stops_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let gitconfig = home.join(".gitconfig");
        excludes_file_set(&gitconfig, "~\\\\ign");
        let said = excludes(home, &home.join("xdg"), home, &[])
            .unwrap_err()
            .to_string();
        assert!(
            said.contains("~\\ign: only `~/` names your home folder")
                && said.contains("write `~/ign` or the whole path"),
            "{said}"
        );
        std::fs::write(&gitconfig, "[include]\n\tpath = ~\\\\inc\n").unwrap();
        let said = excludes(home, &home.join("xdg"), home, &[])
            .unwrap_err()
            .to_string();
        assert!(
            said.contains("~\\inc: only `~/` names your home folder")
                && said.contains("write `~/inc` or the whole path"),
            "{said}"
        );
    }

    #[test]
    fn an_optional_excludes_file_that_is_not_there_counts_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let xdg = home.join("xdg");
        let system = excludes_file_set(&home.join("etc/gitconfig"), "~/system");
        excludes_file_set(&home.join(".gitconfig"), ":(optional)~/nope");
        // It undoes the earlier setting too, so Git's own default list is read.
        let vars = [("GIT_CONFIG_SYSTEM", system.as_str())];
        let found = excludes(home, &xdg, home, &vars).unwrap();
        assert_eq!(found, Some(xdg.join("git").join("ignore")));
    }

    #[test]
    fn an_optional_excludes_file_that_is_there_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::write(home.join("list"), "*.log\n").unwrap();
        excludes_file_set(&home.join(".gitconfig"), ":(optional)~/list");
        let found = excludes(home, &home.join("xdg"), home, &[]).unwrap();
        assert_eq!(found, Some(home.join("list")));
    }

    #[test]
    fn missing_identity_shows_the_snippet() {
        let e = ConfigError::MissingIdentity {
            path: Some(PathBuf::from("cfg.toml")),
        };
        let hint = e.hint().unwrap();
        assert!(
            hint.contains("cfg.toml") && hint.contains("make the file") && hint.contains(SNIPPET),
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
        let misplaced = ConfigError::NotAtTop {
            key: "safe-folders",
            path: PathBuf::from(HOSTILE),
        };
        let shown = misplaced.to_string();
        assert_hostile_text_escaped(&shown);
        assert_eq!(shown.matches("b\\u{202e}c").count(), 1, "{shown}");
        // A variable's name and what it holds.
        let variable = ConfigError::GitVariable {
            name: HOSTILE.into(),
            detail: HOSTILE.into(),
        };
        let shown = variable.to_string();
        assert_hostile_text_escaped(&shown);
        assert_eq!(shown.matches("b\\u{202e}c").count(), 2, "{shown}");
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

    /// `ignored_given` for the project at `root`, its `.bolt` held open as a project holds it.
    fn named_by(root: &Path, used: Option<&Identity>) -> Option<Identity> {
        let bolt = root.join(".bolt");
        std::fs::create_dir_all(&bolt).unwrap();
        ignored_given(&Dir::open_top(&bolt).unwrap(), &bolt, used)
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
        assert_eq!(named_by(&project, None), None, "no settings at all");
        project_settings(&project, MALLORY);
        let ada = person("Ada", "ada@example.com");
        assert_eq!(named_by(&project, Some(&ada)), Some(mallory.clone()));
        // Nobody could be found for the user: the project's is still not used.
        assert_eq!(named_by(&project, None), Some(mallory.clone()));
        // The same as the identity in use: nothing was ignored.
        assert_eq!(named_by(&project, Some(&mallory)), None);
        // Not an identity: half of one, or none.
        for text in ["[user]\nname = \"Mallory\"\n", "[user]\n", "", "x = 1\n"] {
            project_settings(&project, text);
            assert_eq!(named_by(&project, Some(&ada)), None, "{text:?}");
        }
    }

    /// The project's settings are read through its `.bolt` as the project holds it open: a `.bolt`
    /// swapped for a link since can't bring settings from elsewhere.
    #[test]
    fn project_settings_are_read_through_the_held_bolt_folder() {
        let dir = tempfile::tempdir().unwrap();
        let project = folder(dir.path(), "project");
        project_settings(&project, "");
        let bolt = project.join(".bolt");
        let held = Dir::open_top(&bolt).unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("config.toml"), MALLORY).unwrap();
        std::fs::rename(&bolt, project.join("checked")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(elsewhere.path(), &bolt).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&bolt, elsewhere.path());
        assert_eq!(ignored_given(&held, &bolt, None), None);
    }

    #[test]
    fn project_settings_that_cannot_be_read_name_nobody() {
        let dir = tempfile::tempdir().unwrap();
        let project = folder(dir.path(), "project");
        let ada = person("Ada", "ada@example.com");
        let path = project.join(".bolt").join("config.toml");
        let unreadable = |what: &str| assert_eq!(named_by(&project, Some(&ada)), None, "{what}");

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
                let _ = tx.send(named_by(&project, None));
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
    fn undo_window_days_is_read_from_the_users_settings_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // No settings file, or one without the key: 30 days.
        assert_eq!(undo_window_days_in(&path).unwrap(), 30);
        std::fs::write(&path, "[user]\nname = \"Ada\"\n").unwrap();
        assert_eq!(undo_window_days_in(&path).unwrap(), 30);
        for (text, days) in [
            ("undo-window-days = 1\n", 1),
            ("undo-window-days = 36500\n[user]\nname = \"Ada\"\n", 36_500),
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(undo_window_days_in(&path).unwrap(), days, "{text}");
        }
        for value in ["0", "36501", "-1", "\"30\"", "30.5", "[30]"] {
            std::fs::write(&path, format!("undo-window-days = {value}\n")).unwrap();
            match undo_window_days_in(&path) {
                Err(e @ ConfigError::Invalid { .. }) => {
                    let said = e.to_string();
                    assert!(said.starts_with(&path.display().to_string()), "{said}");
                    let want = format!(
                        "undo-window-days must be a whole number of days from 1 to 36500, not \
                         {value}"
                    );
                    assert!(said.ends_with(&want), "{said}");
                }
                other => panic!("{value}: {other:?}"),
            }
        }
        // Below a [section] it would belong to that section, where Bolt never looks.
        std::fs::write(&path, "[user]\nname = \"Ada\"\nundo-window-days = 7\n").unwrap();
        let err = undo_window_days_in(&path).unwrap_err();
        let want = format!(
            "undo-window-days must be at the top of {}, before any [section]",
            path.display()
        );
        assert_eq!(err.to_string(), want);
        assert!(err.hint().unwrap().contains("above"), "{err:?}");
        // Each key is judged on its own.
        assert!(safe_folders_in(&path).unwrap().is_empty());
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
