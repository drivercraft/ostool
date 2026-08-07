//! Verified prebuilt OVMF firmware cache.

use std::path::PathBuf;

pub use crate::run::ovmf_prebuilt::{Arch, Error, FileType, Prebuilt, Source};

/// Returns the shared OVMF cache directory.
///
/// All ostool consumers use `$TMPDIR/ostool/ovmf`, where `$TMPDIR` is the
/// platform temporary directory selected by [`std::env::temp_dir`].
pub fn default_cache_dir() -> PathBuf {
    std::env::temp_dir().join("ostool").join("ovmf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_cache_is_owned_by_ostool() {
        assert_eq!(
            default_cache_dir(),
            std::env::temp_dir().join("ostool").join("ovmf")
        );
    }
}
