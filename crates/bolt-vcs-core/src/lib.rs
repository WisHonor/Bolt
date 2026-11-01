//! Bolt's engine library: objects, storage and history. No `unsafe` code is allowed here.
#![forbid(unsafe_code)]

pub mod encoding;
pub mod id;
pub mod listing;
pub mod names;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
