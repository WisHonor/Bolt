//! Bolt's engine library: objects, storage and history. No `unsafe` code is allowed here.
#![forbid(unsafe_code)]

pub mod encoding;
pub mod fsio;
pub mod id;
pub mod listing;
pub mod names;
pub mod pack;
pub mod save;
pub mod store;
pub mod verify;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
