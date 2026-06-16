//! Rust object tool lookup helpers.

use std::path::PathBuf;

/// Rust toolchain object tools used by artifact preparation and analysis.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectToolKind {
    Objcopy,
    Objdump,
    Readobj,
    Nm,
}

impl ObjectToolKind {
    fn rust_program(self) -> &'static str {
        match self {
            Self::Objcopy => "rust-objcopy",
            Self::Objdump => "rust-objdump",
            Self::Readobj => "rust-readobj",
            Self::Nm => "rust-nm",
        }
    }
}

/// Default Rust object tools.
#[derive(Clone, Debug, Default)]
pub(crate) struct ObjectTools;

impl ObjectTools {
    pub(crate) fn program(&self, kind: ObjectToolKind) -> PathBuf {
        PathBuf::from(kind.rust_program())
    }

    pub(crate) fn objcopy(&self) -> PathBuf {
        self.program(ObjectToolKind::Objcopy)
    }

    #[allow(dead_code)]
    pub(crate) fn objdump(&self) -> PathBuf {
        self.program(ObjectToolKind::Objdump)
    }

    #[allow(dead_code)]
    pub(crate) fn readobj(&self) -> PathBuf {
        self.program(ObjectToolKind::Readobj)
    }

    #[allow(dead_code)]
    pub(crate) fn nm(&self) -> PathBuf {
        self.program(ObjectToolKind::Nm)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{ObjectToolKind, ObjectTools};

    #[test]
    fn default_object_tools_use_rust_tool_names() {
        let tools = ObjectTools;

        assert_eq!(
            tools.program(ObjectToolKind::Objcopy),
            PathBuf::from("rust-objcopy")
        );
        assert_eq!(
            tools.program(ObjectToolKind::Objdump),
            PathBuf::from("rust-objdump")
        );
        assert_eq!(
            tools.program(ObjectToolKind::Readobj),
            PathBuf::from("rust-readobj")
        );
        assert_eq!(tools.program(ObjectToolKind::Nm), PathBuf::from("rust-nm"));
    }
}
