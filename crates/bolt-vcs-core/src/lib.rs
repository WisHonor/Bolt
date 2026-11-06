//! Bolt's engine library: objects, storage and history. No `unsafe` code is allowed here.
#![forbid(unsafe_code)]

pub mod changeids;
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
pub mod keyed;
pub mod link;
pub mod listing;
pub mod merge;
pub mod names;
pub mod pack;
pub mod repo;
pub mod safe_fs;
pub mod save;
pub mod snapshot;
pub mod store;
pub mod text;
pub mod verify;
pub mod warning;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
