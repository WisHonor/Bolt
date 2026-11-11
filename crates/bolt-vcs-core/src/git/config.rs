//! `.git/config`, read as Git reads it, keeping only what decides how to read the repository,
//! and the user's own Git settings that Bolt reads.

pub(super) use crate::legacy::git_config::{Format, read_format, read_worktree};
pub(crate) use crate::legacy::git_config::{
    UserSetting, read_bool, read_count, read_user_settings, read_user_variable,
};

#[cfg(test)]
mod tests;
