//! Installs the game runtime into the launcher's data folder.
//!
//! The runtime (`azura` and the libraries, Vulkan loader and assets beside it) is built from the
//! project with `scripts/build.sh`. Installing copies that build into `<app data>/runtime/`, next
//! to the `azura.toml` the launcher writes: the runtime reads its settings from beside its own
//! executable, so it must be a real copy there, not a link to the build folder.
//!
//! The build is found, in order, in:
//! - the folder named by the `AZURA_RUNTIME_BUILD` environment variable;
//! - `runtime/out/build/mac-arm64-sdk-source/<config>/` of the project the launcher was built in,
//!   found by walking up from the launcher executable;
//! - for the development launcher, Azura (DEV), the same folders of the Azura runtime project
//!   (`AZURA_PROJECT` when it was compiled, or the `Azura` checkout beside this repository): once
//!   installed in /Applications, walking up from its executable never reaches the project.
//!
//! A build newer than the installed copy is reported as an update, so after rebuilding the runtime
//! the launcher offers to install it again.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::runtime::{executable_path, RUNTIME_EXECUTABLE};

/// Environment variable that points at a runtime build folder.
const BUILD_ENV: &str = "AZURA_RUNTIME_BUILD";

/// Build folders of the project, relative to its root, in order of preference.
/// scripts/build.sh builds from the SDK source on every platform (presets `<platform>-sdk-source`).
const PROJECT_BUILDS: [&str; 8] = [
    "runtime/out/build/mac-arm64-sdk-source/RelWithDebInfo",
    "runtime/out/build/mac-arm64-sdk-source/Release",
    "runtime/out/build/linux-amd64-sdk-source/RelWithDebInfo",
    "runtime/out/build/linux-amd64-sdk-source/Release",
    "runtime/out/build/linux-arm64-sdk-source/RelWithDebInfo",
    "runtime/out/build/linux-arm64-sdk-source/Release",
    "runtime/out/build/win-amd64-sdk-source/RelWithDebInfo",
    "runtime/out/build/win-amd64-sdk-source/Release",
];

/// Entries of a build folder that make up the runtime. Everything else there (logs, the
/// developer's own `azura.toml`) stays behind.
const RUNTIME_ENTRIES: [&str; 6] = [
    RUNTIME_EXECUTABLE,
    "azura.default.toml",
    "gamecontrollerdb.txt",
    "assets",
    "vulkan",
    // Libraries are matched by extension; this marks where they go in the copy order.
    "libraries",
];

/// State of the runtime, sent to the interface as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    /// The runtime executable is in the runtime folder.
    pub installed: bool,
    /// A build is available to install from.
    pub build_found: bool,
    /// The available build is newer than the installed copy (or nothing is installed).
    pub update_available: bool,
    /// Path of the installed executable.
    pub path: String,
    /// Where the runtime comes from: "build" (a project build, copied), "kit" (built here from the
    /// player kit and the player's default.xex) or "none".
    pub source: &'static str,
    /// For a runtime built from the kit: what an update takes, "link" (seconds) or "build" (the
    /// generated code again, minutes); `None` when it is up to date or not built from the kit.
    pub update_kind: Option<&'static str>,
}

/// Finds the runtime build to install from, if any.
pub fn find_build() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(BUILD_ENV).map(PathBuf::from) {
        return executable_path(&dir).is_file().then_some(dir);
    }
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent();
    while let Some(current) = dir {
        for build in PROJECT_BUILDS {
            let candidate = current.join(build);
            if executable_path(&candidate).is_file() {
                return Some(candidate);
            }
        }
        dir = current.parent();
    }
    if cfg!(feature = "dev") {
        return project_build(&built_in_project());
    }
    None
}

/// The Azura runtime project a development launcher installs builds from: the folder in
/// `AZURA_PROJECT` when the launcher was compiled, or else a checkout named `Azura` beside this
/// repository.
fn built_in_project() -> PathBuf {
    match option_env!("AZURA_PROJECT") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Azura"),
    }
}

/// The first runtime build of `project` that holds the executable, in PROJECT_BUILDS's order.
fn project_build(project: &Path) -> Option<PathBuf> {
    PROJECT_BUILDS.iter().map(|build| project.join(build)).find(|candidate| executable_path(candidate).is_file())
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Reports whether the runtime is installed in `runtime_dir` and whether `build` is newer.
pub fn status(runtime_dir: &Path, build: Option<&Path>) -> RuntimeStatus {
    let installed_exe = executable_path(runtime_dir);
    // A link to a build (how the runtime was placed by hand before the launcher installed it)
    // does not count as installed: the runtime would read the build folder's settings.
    let installed = fs::symlink_metadata(&installed_exe).map(|m| m.is_file()).unwrap_or(false);
    let update_available = match build {
        None => false,
        Some(build) => {
            !installed
                || match (modified(&executable_path(build)), modified(&installed_exe)) {
                    (Some(built), Some(copied)) => built > copied,
                    _ => true,
                }
        }
    };
    RuntimeStatus {
        installed,
        build_found: build.is_some(),
        update_available,
        path: installed_exe.to_string_lossy().into_owned(),
        source: if build.is_some() { "build" } else { "none" },
        update_kind: None,
    }
}

/// Copies `from` to `to`: a file, or a folder with everything in it. Links are followed, so the
/// copy holds real files.
fn copy_entry(from: &Path, to: &Path) -> io::Result<()> {
    if fs::metadata(from)?.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_entry(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

/// Removes a file, a link or a folder; a missing entry is not an error.
fn remove_entry(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    }
}

/// Names of the runtime entries present in `build`.
fn runtime_entries(build: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for name in RUNTIME_ENTRIES {
        if name == "libraries" {
            for entry in fs::read_dir(build)? {
                let file = entry?.file_name().to_string_lossy().into_owned();
                if file.ends_with(".dylib") || file.ends_with(".so") || file.contains(".so.") || file.ends_with(".dll") {
                    names.push(file);
                }
            }
        } else if build.join(name).exists() {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

/// Installs the runtime from `build` into `runtime_dir`, replacing the previous copy (or links)
/// but keeping `azura.toml` and anything else that is not part of the runtime. The executable is
/// copied last, so an interrupted install never looks complete.
pub fn install(build: &Path, runtime_dir: &Path) -> Result<(), String> {
    let entries = runtime_entries(build).map_err(|e| format!("{}: {e}", build.display()))?;
    if !entries.iter().any(|name| name == RUNTIME_EXECUTABLE) {
        return Err(format!("{} holds no runtime build", build.display()));
    }
    fs::create_dir_all(runtime_dir).map_err(|e| e.to_string())?;
    remove_entry(&executable_path(runtime_dir)).map_err(|e| e.to_string())?;
    for name in entries.iter().filter(|name| name.as_str() != RUNTIME_EXECUTABLE) {
        let target = runtime_dir.join(name);
        remove_entry(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        copy_entry(&build.join(name), &target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    copy_entry(&executable_path(build), &executable_path(runtime_dir)).map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    set_executable_icon(runtime_dir);

    Ok(())
}

/// Gives the installed executable the Azura icon, so macOS shows it instead of a generic one (the
/// runtime is a bare executable, not an app bundle). Done at install time only: setting a file's
/// icon also updates its modification time, which `status` compares with the build's. Failing to
/// set it changes nothing else, so errors are ignored.
#[cfg(target_os = "macos")]
fn set_executable_icon(runtime_dir: &Path) {
    let icon_path = runtime_dir.join("assets").join("azura-icon.png");
    let exe_path = executable_path(runtime_dir);
    if !icon_path.is_file() || !exe_path.is_file() {
        return;
    }
    // The paths go in as arguments, so no quoting of them is needed in the script.
    const SCRIPT: &str = r#"function run(argv) {
  ObjC.import("AppKit");
  var image = $.NSImage.alloc.initWithContentsOfFile(argv[0]);
  if (image) $.NSWorkspace.sharedWorkspace.setIconForFileOptions(image, argv[1], 0);
}"#;
    let _ = std::process::Command::new("osascript")
        .args(["-l", "JavaScript", "-e", SCRIPT])
        .arg(&icon_path)
        .arg(&exe_path)
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    /// A fake build folder with an executable, a library, a data folder and files that must not
    /// be installed.
    fn fake_build(dir: &Path) {
        fs::write(dir.join(RUNTIME_EXECUTABLE), b"runtime").unwrap();
        fs::write(dir.join("librex.dylib"), b"lib").unwrap();
        fs::create_dir_all(dir.join("vulkan/icd.d")).unwrap();
        fs::write(dir.join("vulkan/icd.d/MoltenVK_icd.json"), b"{}").unwrap();
        fs::write(dir.join("azura.toml"), b"fullscreen = true").unwrap();
        fs::create_dir_all(dir.join("logs")).unwrap();
    }

    #[test]
    fn finds_a_project_build_by_the_project_folder() {
        let project = TempFolder::new("project");
        assert_eq!(project_build(&project), None);
        // RelWithDebInfo comes before Release when both exist.
        for config in ["RelWithDebInfo", "Release"] {
            let dir = project.join("runtime/out/build/mac-arm64-sdk-source").join(config);
            fs::create_dir_all(&dir).unwrap();
            fake_build(&dir);
        }
        assert_eq!(project_build(&project), Some(project.join(PROJECT_BUILDS[0])));
        // Without AZURA_PROJECT, the runtime project is the Azura checkout beside this repository.
        if option_env!("AZURA_PROJECT").is_none() {
            assert_eq!(built_in_project(), Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Azura"));
        }
    }

    #[test]
    fn installs_a_real_copy_and_keeps_the_settings() {
        let build = TempFolder::new("build");
        let runtime = TempFolder::new("runtime");
        fake_build(&build);
        fs::write(runtime.join("azura.toml"), b"user_name = \"Marcus\"").unwrap();

        assert!(!status(&runtime, Some(&build)).installed);
        install(&build, &runtime).unwrap();

        assert!(fs::symlink_metadata(executable_path(&runtime)).unwrap().is_file());
        assert!(runtime.join("librex.dylib").is_file());
        assert!(runtime.join("vulkan/icd.d/MoltenVK_icd.json").is_file());
        assert!(!runtime.join("logs").exists());
        assert_eq!(fs::read_to_string(runtime.join("azura.toml")).unwrap(), "user_name = \"Marcus\"");
        let after = status(&runtime, Some(&build));
        assert!(after.installed);
        assert!(!after.update_available);
    }

    // A link to a file needs Developer Mode on Windows.
    #[cfg(unix)]
    #[test]
    fn a_link_to_the_build_is_not_an_installed_runtime() {
        let build = TempFolder::new("build-link");
        let runtime = TempFolder::new("runtime-link");
        fake_build(&build);
        std::os::unix::fs::symlink(executable_path(&build), executable_path(&runtime)).unwrap();

        let before = status(&runtime, Some(&build));
        assert!(!before.installed);
        assert!(before.update_available);
        install(&build, &runtime).unwrap();
        assert!(status(&runtime, Some(&build)).installed);
    }

    #[test]
    fn without_a_build_nothing_is_offered() {
        let runtime = TempFolder::new("runtime-none");
        let found = status(&runtime, None);
        assert!(!found.build_found);
        assert!(!found.update_available);
    }
}
