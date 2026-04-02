use std::{
    fmt,
    path::{Path, PathBuf},
};

/// Canonical path for an element in the config tree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ElementPath {
    segments: Vec<String>,
}

impl ElementPath {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn parse(path: &str) -> Self {
        if path.is_empty() {
            return Self::root();
        }

        Self {
            segments: path
                .split('.')
                .filter(|segment| !segment.is_empty())
                .map(str::to_string)
                .collect(),
        }
    }

    pub fn from_path(path: &Path) -> Self {
        Self {
            segments: path
                .iter()
                .map(|segment| segment.to_string_lossy().into_owned())
                .collect(),
        }
    }

    pub fn child(&self, segment: impl Into<String>) -> Self {
        let mut next = self.segments.clone();
        next.push(segment.into());
        Self { segments: next }
    }

    pub fn parent(&self) -> Option<Self> {
        if self.segments.is_empty() {
            return None;
        }

        let mut next = self.segments.clone();
        next.pop();
        Some(Self { segments: next })
    }

    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn as_key(&self) -> String {
        self.segments.join(".")
    }

    pub fn display(&self) -> String {
        if self.is_root() {
            "/".to_string()
        } else {
            self.as_key()
        }
    }

    pub fn to_path_buf(&self) -> PathBuf {
        self.segments.iter().collect()
    }
}

impl fmt::Display for ElementPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

impl From<&str> for ElementPath {
    fn from(value: &str) -> Self {
        Self::parse(value)
    }
}

impl From<String> for ElementPath {
    fn from(value: String) -> Self {
        Self::parse(&value)
    }
}

impl From<&String> for ElementPath {
    fn from(value: &String) -> Self {
        Self::parse(value)
    }
}

impl From<&Path> for ElementPath {
    fn from(value: &Path) -> Self {
        Self::from_path(value)
    }
}

impl From<PathBuf> for ElementPath {
    fn from(value: PathBuf) -> Self {
        Self::from_path(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::ElementPath;

    #[test]
    fn parse_root_path() {
        assert_eq!(ElementPath::parse(""), ElementPath::root());
        assert_eq!(ElementPath::root().display(), "/");
    }

    #[test]
    fn parse_nested_path() {
        let path = ElementPath::parse("system.features");
        assert_eq!(
            path.segments(),
            &["system".to_string(), "features".to_string()]
        );
        assert_eq!(path.as_key(), "system.features");
        assert_eq!(path.parent().unwrap().as_key(), "system");
    }
}
