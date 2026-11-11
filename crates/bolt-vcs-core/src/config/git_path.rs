//! A path one of Git's settings gives, tidied as Git tidies it.

pub(crate) use crate::legacy::git_path::tidy_path;

#[cfg(test)]
mod tests;
