//! The environment of the programs the launcher starts, when the launcher runs as an AppImage.
//!
//! On Linux the launcher is shipped as an AppImage, which carries the libraries of the system it
//! was built on (Ubuntu 22.04) and points the launcher at them through its environment: the
//! AppImage's `AppRun` puts `$APPDIR/usr/lib` first in `LD_LIBRARY_PATH`, `$APPDIR/usr/bin` first
//! in `PATH`, and its GTK hook sets `GTK_PATH`, `GIO_MODULE_DIR`, `GTK_THEME` and others. `APPDIR`
//! is the folder the AppImage is mounted on while it runs.
//!
//! A program started from the launcher inherits that environment. For the game it means loading
//! the AppImage's libraries instead of its own and the system's: `LD_LIBRARY_PATH` is searched
//! before the runtime's `RUNPATH` (`$ORIGIN`), so the copy of `librexruntime` that linuxdeploy put
//! in `$APPDIR/usr/lib` wins over the one next to `azura`, and the libraries SDL opens at run time
//! (`libxkbcommon`, `libdbus`, the X11 and Wayland helpers) come from 22.04 whatever the player's
//! system is. Every launcher update replaces those copies.
//!
//! `use_host_env` gives a program the environment it would have outside the AppImage: it drops
//! from every variable the paths inside `$APPDIR` (removing the variable when nothing is left),
//! and the variables the AppImage sets that are not paths. Paths the player set themselves, outside
//! `$APPDIR`, are kept. Outside an AppImage (`APPDIR` unset) nothing changes.
//!
//! On Windows it also keeps console programs (the compiler, the code generator) from opening a
//! console window each: the launcher is a windowed app, and every command it runs would otherwise
//! flash one.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

/// The variable holding the AppImage's mount folder, set by the AppImage runtime.
const APPDIR: &str = "APPDIR";

/// Variables the AppImage sets whose values are not paths into `$APPDIR`: the theme the GTK hook
/// forces on the launcher's window, and what the AppImage runtime records about itself.
const APPIMAGE_ONLY_VARIABLES: [&str; 4] = ["GTK_THEME", "APPIMAGE", "ARGV0", "OWD"];

/// Changes `command`'s environment to the one outside the AppImage, when the launcher runs in one.
pub fn use_host_env(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console for a console program; a windowed one (the game) is not
        // affected.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    apply(command, host_changes(std::env::vars_os()))
}

/// Sets or removes on `command` each variable in `changes` (see `host_changes`).
fn apply(command: &mut Command, changes: Vec<(OsString, Option<OsString>)>) -> &mut Command {
    for (name, value) in changes {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    command
}

/// The changes that turn the environment `vars` into the one outside the AppImage: each variable
/// to change, with its new value or `None` to remove it. Empty when `vars` has no `APPDIR`.
fn host_changes(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, Option<OsString>)> {
    let vars: Vec<(OsString, OsString)> = vars.into_iter().collect();
    let Some(appdir) = vars.iter().find(|(name, _)| name == APPDIR).map(|(_, value)| value.clone()) else {
        return Vec::new();
    };
    // An empty APPDIR would make every absolute path look like it is inside it.
    if appdir.is_empty() {
        return Vec::new();
    }
    let appdir = Path::new(&appdir);
    let mut changes = Vec::new();
    for (name, value) in &vars {
        if APPIMAGE_ONLY_VARIABLES.iter().any(|only| name == *only) {
            changes.push((name.clone(), None));
        } else if let Some(kept) = without_appdir_paths(value, appdir) {
            changes.push((name.clone(), kept));
        }
    }
    changes
}

/// `value` read as a list of paths, without the ones inside `appdir` and the empty entries the
/// AppImage leaves when it prepends to an unset variable (an empty entry in `LD_LIBRARY_PATH` or
/// `PATH` means the current folder). `None` when `value` has no path inside `appdir` and stays as
/// it is; `Some(None)` when nothing is left and the variable goes.
fn without_appdir_paths(value: &OsStr, appdir: &Path) -> Option<Option<OsString>> {
    // Path::starts_with compares whole components, so "$APPDIR//usr/lib" (the GTK hook doubles
    // the slash) is inside, and a sibling folder such as "$APPDIR-old" is not.
    let inside = |path: &Path| path.starts_with(appdir);
    if !std::env::split_paths(value).any(|path| inside(&path)) {
        return None;
    }
    let kept: Vec<_> =
        std::env::split_paths(value).filter(|path| !path.as_os_str().is_empty() && !inside(path)).collect();
    if kept.is_empty() {
        return Some(None);
    }
    // The entries came from splitting the value, so none holds the separator and joining works.
    Some(std::env::join_paths(kept).ok())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A test environment from name and value pairs.
    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs.iter().map(|(name, value)| (OsString::from(name), OsString::from(value))).collect()
    }

    /// The change for `name` in `changes`: `None` when untouched, `Some(None)` when removed.
    fn change<'a>(changes: &'a [(OsString, Option<OsString>)], name: &str) -> Option<Option<&'a str>> {
        changes
            .iter()
            .find(|(changed, _)| changed == name)
            .map(|(_, value)| value.as_ref().map(|value| value.to_str().unwrap()))
    }

    const APPDIR_VALUE: &str = "/tmp/.mount_AzuraX";

    #[test]
    fn changes_nothing_outside_an_appimage() {
        let vars = env(&[("LD_LIBRARY_PATH", "/opt/lib"), ("GTK_THEME", "Adwaita:dark"), ("PATH", "/usr/bin")]);
        assert!(host_changes(vars).is_empty());
    }

    #[test]
    fn changes_nothing_with_an_empty_appdir() {
        let vars = env(&[("APPDIR", ""), ("LD_LIBRARY_PATH", "/opt/lib")]);
        assert!(host_changes(vars).is_empty());
    }

    #[test]
    fn drops_the_appimage_paths_and_keeps_the_players() {
        // As AppRun leaves them: its own folders first, then what the variable held before.
        let vars = env(&[
            ("APPDIR", APPDIR_VALUE),
            ("LD_LIBRARY_PATH", "/tmp/.mount_AzuraX/usr/lib/:/tmp/.mount_AzuraX/lib64/:/opt/mylibs"),
            ("PATH", "/tmp/.mount_AzuraX/usr/bin/:/tmp/.mount_AzuraX/bin/:/usr/local/bin:/usr/bin"),
            ("XDG_DATA_DIRS", "/tmp/.mount_AzuraX/usr/share/:/tmp/.mount_AzuraX/usr/share:/usr/share:/usr/local/share"),
        ]);
        let changes = host_changes(vars);
        assert_eq!(change(&changes, "LD_LIBRARY_PATH"), Some(Some("/opt/mylibs")));
        assert_eq!(change(&changes, "PATH"), Some(Some("/usr/local/bin:/usr/bin")));
        assert_eq!(change(&changes, "XDG_DATA_DIRS"), Some(Some("/usr/share:/usr/local/share")));
    }

    #[test]
    fn removes_variables_left_with_nothing() {
        // An unset LD_LIBRARY_PATH ends in an empty entry after AppRun prepends to it, which would
        // mean the current folder if it were kept.
        let vars = env(&[
            ("APPDIR", APPDIR_VALUE),
            ("LD_LIBRARY_PATH", "/tmp/.mount_AzuraX/usr/lib/:/tmp/.mount_AzuraX/lib/:"),
            ("GTK_PATH", "/tmp/.mount_AzuraX//usr/lib/gtk-3.0"),
            ("GDK_PIXBUF_MODULE_FILE", "/tmp/.mount_AzuraX//usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"),
        ]);
        let changes = host_changes(vars);
        assert_eq!(change(&changes, "LD_LIBRARY_PATH"), Some(None));
        assert_eq!(change(&changes, "GTK_PATH"), Some(None));
        assert_eq!(change(&changes, "GDK_PIXBUF_MODULE_FILE"), Some(None));
        assert_eq!(change(&changes, "APPDIR"), Some(None));
    }

    #[test]
    fn removes_the_appimage_variables_that_are_not_paths() {
        let vars = env(&[
            ("APPDIR", APPDIR_VALUE),
            ("GTK_THEME", "Adwaita:dark"),
            ("APPIMAGE", "/home/player/Azura-Launcher_0.3.0_amd64.AppImage"),
            ("ARGV0", "./Azura-Launcher_0.3.0_amd64.AppImage"),
            ("OWD", "/home/player"),
        ]);
        let changes = host_changes(vars);
        for name in APPIMAGE_ONLY_VARIABLES {
            assert_eq!(change(&changes, name), Some(None), "{name}");
        }
    }

    #[test]
    fn leaves_unrelated_variables_alone() {
        let vars = env(&[
            ("APPDIR", APPDIR_VALUE),
            ("HOME", "/home/player"),
            ("DISPLAY", ":0"),
            ("VK_DRIVER_FILES", "/usr/share/vulkan/icd.d/nvidia_icd.json"),
            // Starts like the mount folder but is another folder.
            ("SOME_DIR", "/tmp/.mount_AzuraX-old/data"),
        ]);
        let changes = host_changes(vars);
        assert_eq!(changes.len(), 1, "only APPDIR itself goes: {changes:?}");
        assert_eq!(change(&changes, "APPDIR"), Some(None));
    }

    #[test]
    fn applies_the_changes_to_a_command() {
        let mut command = Command::new("true");
        let changes = host_changes(env(&[
            ("APPDIR", APPDIR_VALUE),
            ("LD_LIBRARY_PATH", "/tmp/.mount_AzuraX/usr/lib/:/opt/mylibs"),
            ("GTK_THEME", "Adwaita:dark"),
        ]));
        apply(&mut command, changes);
        let envs: Vec<_> = command.get_envs().map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned))).collect();
        assert!(envs.contains(&("LD_LIBRARY_PATH".into(), Some("/opt/mylibs".into()))), "{envs:?}");
        assert!(envs.contains(&("GTK_THEME".into(), None)), "{envs:?}");
    }
}
