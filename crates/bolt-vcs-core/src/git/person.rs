//! An author, committer or tagger line, read as Git reads one.

pub use crate::legacy::person::parse_person;

#[cfg(test)]
mod tests;
