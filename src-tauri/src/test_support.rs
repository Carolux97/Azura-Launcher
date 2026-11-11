//! Helpers shared by the unit tests.

use std::ops::Deref;
use std::path::{Path, PathBuf};

/// An empty folder under the system temp directory, removed with its contents when dropped, so
/// tests leave nothing behind even when they fail.
pub struct TempFolder(PathBuf);

use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(1);

impl TempFolder {
    /// Creates the folder; `name` keeps it unique among the tests of one run.
    pub fn new(name: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("azura-test-{}-{count}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create a temp folder");
        Self(dir)
    }
}

impl Deref for TempFolder {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
