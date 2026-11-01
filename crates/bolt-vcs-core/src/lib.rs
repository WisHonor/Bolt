//! Bolt's engine library: objects, storage and history. No `unsafe` code is allowed here.
#![forbid(unsafe_code)]

pub mod id;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
