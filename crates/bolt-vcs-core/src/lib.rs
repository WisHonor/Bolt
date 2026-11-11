//! Bolt's engine library: objects, storage and history. No `unsafe` code is allowed here.
#![forbid(unsafe_code)]

pub mod changeids;
pub mod changes;
pub mod chunks;
pub mod config;
pub mod diary;
pub mod diff;
pub mod drive;
pub mod encoding;
pub mod filecache;
pub mod fsio;
pub mod git;
pub mod history;
pub mod id;
pub mod ignore;
pub mod import;
pub(crate) mod ingest;
pub mod journal;
pub(crate) mod kept;
pub mod keyed;
pub mod lines;
pub mod link;
pub mod listing;
pub mod merge;
pub mod names;
pub mod otherlines;
pub mod pack;
pub mod presets;
pub mod repo;
pub mod rules;
pub mod safe_fs;
pub mod save;
pub(crate) mod scope;
pub mod setup;
pub mod shelf;
pub mod snapshot;
pub mod status;
pub mod store;
pub mod switch;
pub mod text;
pub mod tidy;
pub(crate) mod timeline;
pub mod undo;
pub mod verify;
pub mod warning;
pub mod workers;
pub mod writeout;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
