//! Settings: who the user is (name and email), from the environment or a `config.toml` file.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::save::Person;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Bolt needs your name and email to record who made each save.")]
    MissingIdentity { path: Option<PathBuf> },
    #[error("{} is not a valid Bolt settings file: {detail}", path.display())]
    Invalid { path: PathBuf, detail: String },
}

const SNIPPET: &str = "[user]\nname = \"Your Name\"\nemail = \"you@example.com\"";

impl ConfigError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            ConfigError::MissingIdentity { path: Some(path) } => {
                Some(format!("Add this to {}:\n{SNIPPET}", path.display()))
            }
            ConfigError::MissingIdentity { path: None } => Some(format!(
                "Set BOLT_CONFIG_DIR to a folder and add this to config.toml there:\n{SNIPPET}"
            )),
            ConfigError::Invalid { .. } => None,
        }
    }
}

#[derive(Deserialize)]
struct ConfigFile {
    user: Option<User>,
}

#[derive(Deserialize)]
struct User {
    name: Option<String>,
    email: Option<String>,
}

fn both(name: Option<String>, email: Option<String>) -> Option<Identity> {
    match (name, email) {
        (Some(name), Some(email)) if !name.is_empty() && !email.is_empty() => {
            Some(Identity { name, email })
        }
        _ => None,
    }
}

/// The identity in a `config.toml` file, if it exists and names both.
fn from_file(path: &Path) -> Result<Option<Identity>, ConfigError> {
    let invalid = |detail: String| ConfigError::Invalid {
        path: path.to_owned(),
        detail,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(invalid(e.to_string())),
    };
    let file: ConfigFile = toml::from_str(&text).map_err(|e| invalid(e.message().to_owned()))?;
    Ok(file.user.and_then(|u| both(u.name, u.email)))
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

/// The user's name and email: from `BOLT_USER_NAME` and `BOLT_USER_EMAIL`, else the project's
/// `.bolt/config.toml`, else the user's `config.toml`.
pub fn load_identity(project_root: Option<&Path>) -> Result<Identity, ConfigError> {
    let env = |key| std::env::var(key).ok();
    if let Some(identity) = both(env("BOLT_USER_NAME"), env("BOLT_USER_EMAIL")) {
        return Ok(identity);
    }
    if let Some(root) = project_root
        && let Some(identity) = from_file(&root.join(".bolt").join("config.toml"))?
    {
        return Ok(identity);
    }
    let path = user_config_path();
    if let Some(path) = &path
        && let Some(identity) = from_file(path)?
    {
        return Ok(identity);
    }
    Err(ConfigError::MissingIdentity { path })
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
        assert_eq!(from_file(&path).unwrap(), None);
        std::fs::write(
            &path,
            "[user]\nname = \"Ada\"\nemail = \"ada@example.com\"\n",
        )
        .unwrap();
        let ada = from_file(&path).unwrap().unwrap();
        assert_eq!(
            (ada.name.as_str(), ada.email.as_str()),
            ("Ada", "ada@example.com")
        );
        std::fs::write(&path, "[user]\nname = \"Ada\"\n").unwrap();
        assert_eq!(from_file(&path).unwrap(), None);
        std::fs::write(&path, "[user\n").unwrap();
        assert!(matches!(from_file(&path), Err(ConfigError::Invalid { .. })));
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
    fn now_person_has_a_plausible_time() {
        let p = now_person(&Identity {
            name: "A".into(),
            email: "a@b".into(),
        });
        assert!(p.time_ms > 1_700_000_000_000);
        assert!(p.tz_offset_min.abs() <= 26 * 60);
    }
}
