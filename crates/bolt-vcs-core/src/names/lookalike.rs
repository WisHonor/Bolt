//! Whether Windows or macOS would take a name for `.git`, `.gitmodules` or `.bolt`.

pub use crate::legacy::lookalike::{is_bolt_alias, is_git_alias};
pub(crate) use crate::legacy::lookalike::{is_hashed_short_name, is_plain_short_name};

#[cfg(test)]
mod tests;
