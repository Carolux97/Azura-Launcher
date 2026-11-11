//! What this launcher version brings, for the "What's new" panel.
//!
//! The notes come from `CHANGELOG.md`, built into the launcher, so a launcher always shows
//! its own version's notes. The release scripts publish the same section as the update's notes
//! (scripts/release-common.sh), so the file is the only place they are written.

use serde::Serialize;

/// The changelog, as the launcher was built with it.
const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

/// One version's notes, sent to the interface as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseNotes {
    pub version: String,
    /// The section's list items, without their `- `.
    pub items: Vec<String>,
}

/// The list items of the `## <version>` section of `changelog`, or `None` when it has no such
/// section or the section has no items. An item may continue on indented lines.
pub fn notes_for(changelog: &str, version: &str) -> Option<ReleaseNotes> {
    let heading = format!("## {version}");
    let mut lines = changelog.lines().skip_while(|line| line.trim_end() != heading).skip(1);
    let mut items: Vec<String> = Vec::new();
    for line in lines.by_ref() {
        if line.starts_with("## ") {
            break;
        }
        if let Some(item) = line.strip_prefix("- ") {
            items.push(item.trim().to_owned());
        } else if line.starts_with("  ") && !line.trim().is_empty() {
            if let Some(last) = items.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
        }
    }
    (!items.is_empty()).then(|| ReleaseNotes { version: version.to_owned(), items })
}

/// The notes of the running launcher's version (`version`), from the built-in changelog.
pub fn current(version: &str) -> Option<ReleaseNotes> {
    notes_for(CHANGELOG, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog\n\nIntro.\n\n## 0.2.0\n\n- Controllers drive the launcher.\n- Disc images\n  are read in place.\n\n## 0.1.0\n\n- First release.\n";

    #[test]
    fn reads_the_items_of_a_version() {
        let notes = notes_for(SAMPLE, "0.2.0").unwrap();
        assert_eq!(notes.items, ["Controllers drive the launcher.", "Disc images are read in place."]);
        assert_eq!(notes_for(SAMPLE, "0.1.0").unwrap().items, ["First release."]);
    }

    #[test]
    fn a_version_without_a_section_has_no_notes() {
        assert_eq!(notes_for(SAMPLE, "0.3.0"), None);
        // "0.1" is not "0.1.0".
        assert_eq!(notes_for(SAMPLE, "0.1"), None);
    }

    #[test]
    fn the_launchers_own_version_has_notes() {
        // A version bump adds its section; the release scripts refuse a version without one.
        assert!(current(env!("CARGO_PKG_VERSION")).is_some());
    }
}
