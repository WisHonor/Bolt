//! Lines made, renamed and deleted, and what `goto` can be pointed at (decision 055).

use std::io;

use crate::ancestry::only_in;
use crate::diary::{Action, DiaryEntry, Position, State, line_named_alike, validate_line_name};
use crate::encoding::DecodeError;
use crate::history::{ResolveError, line_named, resolve, tip_of};
use crate::id::Id;
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::save::Person;
use crate::switch::{self, Earlier, SwitchError};
use crate::text::escape;
use crate::warning::Warning;
use crate::writeout::{Applied, Plan};

/// What `line new` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    pub line: String,
    /// The save the new line starts at.
    pub save: Id,
    pub warnings: Vec<Warning>,
}

/// What `line rename` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    pub from: String,
    pub to: String,
    /// False when the line had that name already, and no step was made.
    pub renamed: bool,
    pub warnings: Vec<Warning>,
}

/// What `line delete` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    pub line: String,
    /// The line's newest save.
    pub save: Id,
    /// How many saves only the line held.
    pub only_here: u64,
    pub warnings: Vec<Warning>,
}

/// Why a name can't be a new line's.
#[derive(Debug, thiserror::Error)]
pub enum NameError {
    // `reason` is Opalro's own wording.
    #[error("`{}` can't be a line's name: {reason}.", escape(name, false))]
    Bad { name: String, reason: String },
    #[error(
        "There is already a line {}, and two lines can't have names that differ only by letter \
         case.",
        escape(taken, false)
    )]
    Taken { name: String, taken: String },
    #[error(
        "\"{}\" is a tag's name already, and a line can't share it.",
        escape(name, false)
    )]
    Tag { name: String },
    /// The tags couldn't be read to check the name against.
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl NameError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            NameError::Bad { .. } | NameError::Taken { .. } | NameError::Tag { .. } => {
                Some("Choose another name.".to_owned())
            }
            NameError::Repo(e) => e.hint(),
        }
    }
}

/// Why a line wasn't made, renamed or deleted.
#[derive(Debug, thiserror::Error)]
pub enum LineError {
    #[error(transparent)]
    Name(NameError),
    #[error("There is no line named \"{}\".", escape(.0, false))]
    NoSuchLine(String),
    #[error("You're at a save, not on a line, so there is no current line to rename.")]
    NotOnALine,
    #[error(
        "You're on line {}, and Opalro doesn't delete the line you're on.",
        escape(.0, false)
    )]
    OnIt(String),
    #[error(
        "Line {} holds {} that no other line or tag has, so it wasn't deleted.",
        escape(line, false),
        saves(*only_here)
    )]
    Kept { line: String, only_here: u64 },
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl From<NameError> for LineError {
    fn from(e: NameError) -> Self {
        match e {
            NameError::Repo(e) => LineError::Repo(e),
            e => LineError::Name(e),
        }
    }
}

impl LineError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            LineError::Name(e) => e.hint(),
            LineError::NoSuchLine(_) => Some("See the lines with: opalro line".to_owned()),
            LineError::NotOnALine => {
                Some("Name the line too: opalro line rename <old> <new>".to_owned())
            }
            LineError::OnIt(_) => Some("Go to another line first: opalro goto <line>".to_owned()),
            LineError::Kept { line, .. } => Some(format!(
                "To delete it anyway: opalro line delete {} --yes",
                escape(line, false)
            )),
            LineError::Repo(e) => e.hint(),
        }
    }
}

fn saves(n: u64) -> String {
    match n {
        1 => "1 save".to_owned(),
        n => format!("{} saves", crate::text::thousands(n)),
    }
}

/// Makes line `name` at the save the project is on, and puts the project on it.
pub fn new_line(
    repo: &mut Repository,
    name: &str,
    who: &Person,
    on_wait: &mut dyn FnMut(&str),
) -> Result<Made, LineError> {
    step(repo, "line new", on_wait, |repo, lock, earlier| {
        let (head, entry) = repo.current()?;
        check_new_name(repo, &entry.state, name, None)?;
        let save = tip_of(&entry)?;
        let mut state = entry.state;
        state.lines.insert(name.to_owned(), save);
        state.position = Position::OnLine(name.to_owned());
        record(repo, lock, head, state, who, "line new")?;
        Ok(Made {
            line: name.to_owned(),
            save,
            warnings: told(earlier),
        })
    })
}

/// Renames line `old`, or the line the project is on, to `new`. A project on it stays on it.
pub fn rename_line(
    repo: &mut Repository,
    old: Option<&str>,
    new: &str,
    who: &Person,
    on_wait: &mut dyn FnMut(&str),
) -> Result<Renamed, LineError> {
    step(repo, "line rename", on_wait, |repo, lock, earlier| {
        let (head, entry) = repo.current()?;
        let old = match (old, &entry.state.position) {
            (Some(old), _) => old.to_owned(),
            (None, Position::OnLine(line)) => line.clone(),
            (None, Position::AtSave(_)) => return Err(LineError::NotOnALine),
        };
        let Some(&save) = entry.state.lines.get(&old) else {
            return Err(LineError::NoSuchLine(old));
        };
        let renamed = old != new;
        if renamed {
            check_new_name(repo, &entry.state, new, Some(&old))?;
            let mut state = entry.state;
            state.lines.remove(&old);
            state.lines.insert(new.to_owned(), save);
            if state.position == Position::OnLine(old.clone()) {
                state.position = Position::OnLine(new.to_owned());
            }
            record(repo, lock, head, state, who, "line rename")?;
        }
        Ok(Renamed {
            from: old,
            to: new.to_owned(),
            renamed,
            warnings: told(earlier),
        })
    })
}

/// `confirm(line, only_here)` is called only when no other line, tag or server line holds
/// the line's newest save; a `false` keeps the line.
pub fn delete_line(
    repo: &mut Repository,
    name: &str,
    who: &Person,
    on_wait: &mut dyn FnMut(&str),
    confirm: &mut dyn FnMut(&str, u64) -> bool,
) -> Result<Deleted, LineError> {
    step(repo, "line delete", on_wait, |repo, lock, earlier| {
        let (head, entry) = repo.current()?;
        let Some(&save) = entry.state.lines.get(name) else {
            return Err(LineError::NoSuchLine(name.to_owned()));
        };
        if entry.state.position == Position::OnLine(name.to_owned()) {
            return Err(LineError::OnIt(name.to_owned()));
        }
        // Where the project stands holds nothing: it moves on with the next goto.
        let mut holders: Vec<Id> = entry
            .state
            .lines
            .iter()
            .filter(|(line, _)| *line != name)
            .map(|(_, id)| *id)
            .collect();
        let tags = repo.tag_list(&entry.state)?;
        holders.extend(tags.tags.values().map(|tag| tag.save));
        let servers = repo.server_lines(&entry.state)?;
        holders.extend(servers.servers.values().flat_map(|l| l.values().copied()));
        let only_here = only_in(repo, save, &holders)?;
        if only_here > 0 && !confirm(name, only_here) {
            return Err(LineError::Kept {
                line: name.to_owned(),
                only_here,
            });
        }
        let mut state = entry.state;
        state.lines.remove(name);
        record(repo, lock, head, state, who, "line delete")?;
        Ok(Deleted {
            line: name.to_owned(),
            save,
            only_here,
            warnings: told(earlier),
        })
    })
}

/// The one check of a new line's name. `renaming` is the line being renamed, left out of the
/// "alike" check so a line can take another spelling of its own name.
pub(crate) fn check_new_name(
    repo: &Repository,
    state: &State,
    name: &str,
    renaming: Option<&str>,
) -> Result<(), NameError> {
    validate_line_name(name).map_err(|e| NameError::Bad {
        name: name.to_owned(),
        reason: match e {
            DecodeError::Invalid(reason) => reason.to_owned(),
            _ => "it breaks the rules for line names".to_owned(),
        },
    })?;
    // No two lines are alike, so the one found is the only one.
    if let Some(taken) = line_named_alike(&state.lines, name)
        && Some(taken) != renaming
    {
        return Err(NameError::Taken {
            name: name.to_owned(),
            taken: taken.to_owned(),
        });
    }
    if repo.tag_list(state)?.tags.contains_key(name) {
        return Err(NameError::Tag {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// Runs `f` under the lock for changes once a stopped switch is finished. What finishing left to
/// tell stays in `earlier` until `f` takes it with [`told`], so a refusal still tells it.
fn step<T>(
    repo: &mut Repository,
    command: &str,
    on_wait: &mut dyn FnMut(&str),
    f: impl FnOnce(&mut Repository, &ChangeLock, &mut Earlier) -> Result<T, LineError>,
) -> Result<T, LineError> {
    let done = switch::locked(repo, command, on_wait, |repo, lock, earlier, _| {
        Ok(f(repo, lock, earlier))
    });
    match done {
        Ok(done) => done,
        Err(SwitchError::Repo(e)) => Err(LineError::Repo(e)),
        // Taking the lock and finishing a switch fail only with the repository's errors.
        Err(e) => Err(LineError::Repo(RepoError::Io(io::Error::other(
            e.to_string(),
        )))),
    }
}

/// What finishing a stopped switch left to tell, taken for a step that succeeded.
fn told(earlier: &mut Earlier) -> Vec<Warning> {
    switch::switch_warnings(Plan::default(), Applied::default(), earlier).warnings
}

/// Records a step to `state` that changes no file.
fn record(
    repo: &mut Repository,
    lock: &ChangeLock,
    head: Id,
    state: State,
    who: &Person,
    command: &str,
) -> Result<(), RepoError> {
    let entry = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: command.to_owned(),
        who: who.clone(),
        opal_version: env!("CARGO_PKG_VERSION").into(),
        state,
        folder_change: None,
    };
    let packs = repo.new_packs()?;
    repo.commit(lock, packs, &entry)?;
    Ok(())
}

/// Where `goto` goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Onto a line, at its newest save.
    Line(String),
    /// To a server line's save, which is then the project's position: no line.
    ServerLine {
        server: String,
        line: String,
        save: Id,
    },
    Save(Id),
}

/// Reads `text` as, in this order, a line's name, a server line `line@server`, the start of a
/// change ID, or the start of a fingerprint. A line name never holds `@`, so a server line can't
/// be taken for one; a line name that is also the start of another save's ID is refused.
pub fn resolve_target(repo: &Repository, text: &str) -> Result<Target, ResolveError> {
    let (_, entry) = repo.current()?;
    if line_named(repo, &entry, text)?.is_some() {
        return Ok(Target::Line(text.to_owned()));
    }
    if let Some((line, server)) = text.split_once('@') {
        let servers = repo.server_lines(&entry.state)?;
        let save = servers
            .servers
            .get(server)
            .and_then(|lines| lines.get(line));
        return match save {
            Some(save) => Ok(Target::ServerLine {
                server: server.to_owned(),
                line: line.to_owned(),
                save: *save,
            }),
            None => Err(ResolveError::NotFound(text.to_owned())),
        };
    }
    Ok(Target::Save(resolve(repo, text)?))
}

#[cfg(test)]
mod tests;
