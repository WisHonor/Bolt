//! The old code of the modules being rewritten, kept until each rewrite is checked against it.
#![allow(dead_code)]

#[cfg(test)]
mod compare_git_config;
#[cfg(test)]
mod compare_ignore;
#[cfg(test)]
mod compare_small;
pub(crate) mod git_config;
pub(crate) mod git_path;
pub(crate) mod ignore;
pub(crate) mod lfs;
pub(crate) mod lookalike;
pub(crate) mod person;
