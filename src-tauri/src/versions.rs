//! The registry of known game builds, read from `versions.toml` at the repository root.
//!
//! The file is compiled into the launcher, so the app always knows the builds it was released
//! for. See `versions.toml` for what each field means.

use serde::Deserialize;

/// The registry as it ships with the launcher.
const BUNDLED_REGISTRY: &str = include_str!("../../versions.toml");

/// One known build of the game.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct KnownVersion {
    pub id: String,
    pub name: String,
    pub media_id: String,
    pub xex_sha256: String,
    pub manifest: String,
}

/// Every known build, plus the title ID all of them share.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub title_id: String,
    #[serde(default)]
    pub versions: Vec<KnownVersion>,
}

impl Registry {
    /// Parses a registry from TOML text.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// The registry compiled into the launcher. A parse failure is a packaging bug, caught by the
    /// `bundled_registry_parses` test before any release.
    pub fn bundled() -> Self {
        Self::parse(BUNDLED_REGISTRY).expect("versions.toml bundled with the launcher is invalid")
    }

    /// Finds the build whose executable has this SHA-256 (lowercase hex).
    pub fn find_by_hash(&self, sha256: &str) -> Option<&KnownVersion> {
        self.versions
            .iter()
            .find(|version| version.xex_sha256.eq_ignore_ascii_case(sha256))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_registry_parses() {
        let registry = Registry::bundled();
        assert_eq!(registry.title_id, "4D5308AB");
        assert!(!registry.versions.is_empty());
    }

    #[test]
    fn every_bundled_entry_is_complete() {
        for version in Registry::bundled().versions {
            assert_eq!(version.media_id.len(), 8, "{}: media_id", version.id);
            assert_eq!(version.xex_sha256.len(), 64, "{}: xex_sha256", version.id);
            assert!(version.manifest.ends_with(".toml"), "{}: manifest", version.id);
        }
    }

    #[test]
    fn finds_a_version_by_hash_in_any_case() {
        let registry = Registry::parse(
            r#"
            title_id = "4D5308AB"
            [[versions]]
            id = "world"
            name = "World"
            media_id = "58122AD6"
            xex_sha256 = "abcdef"
            manifest = "runtime/azura_58122AD6.toml"
            "#,
        )
        .unwrap();
        assert_eq!(registry.find_by_hash("ABCDEF").map(|v| v.id.as_str()), Some("world"));
        assert!(registry.find_by_hash("123456").is_none());
    }
}
