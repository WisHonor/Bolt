//! Git's ignore rules, so that Bolt keeps out exactly the files Git keeps out.

pub(crate) use crate::legacy::ignore::InForce;
pub use crate::legacy::ignore::{Decision, Matcher, Rules, Verdict, wildmatch};

#[cfg(test)]
mod tests;
