//! Lines, and what `goto` can be pointed at (decision 055).

use crate::history::{ResolveError, line_named, resolve};
use crate::id::Id;
use crate::repo::Repository;

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
