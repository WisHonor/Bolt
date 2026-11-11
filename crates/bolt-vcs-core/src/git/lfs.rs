//! A Git LFS pointer, read as Git LFS reads one.

pub(crate) use crate::legacy::lfs::{LFS_SPEC, LFS_SPECS};
pub use crate::legacy::lfs::{LfsPointer, MAX_LFS_POINTER_BYTES, lfs_pointer};

#[cfg(test)]
mod tests;
