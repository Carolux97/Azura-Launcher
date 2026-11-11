//! Where the game keeps its saves, and moving them when the player picks another folder.
//!
//! The runtime stores everything it writes for the player (campaign checkpoints, player storage,
//! profiles) under its user data folder (`user_data_root`), laid out as on the console:
//! `<XUID>/<title ID>/...` for saved content and `<title ID>/profile/...` for profiles. The
//! launcher chooses that folder and passes it to the runtime on start-up.
//!
//! Before the launcher managed it, the runtime used `~/.local/share/azura`. Its saves are moved to
//! the chosen folder the first time the game starts from the launcher. The shader cache that
//! folder also holds is not a save: it goes to the launcher's own cache folder instead.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Folder for saves when the player has not chosen one, relative to the home folder. The
/// development launcher has its own, so it never touches the published launcher's saves or its
/// Alboria identity (which lives in the saves folder).
const DEFAULT_SAVES: &str = if cfg!(feature = "dev") {
    "Documents/Games Saves/Gears3-Azura (DEV)"
} else {
    "Documents/Games Saves/Gears3-Azura"
};

/// Where the runtime kept its data before the launcher chose the folder, relative to home.
const LEGACY_USER_DATA: &str = ".local/share/azura";

/// Entry of the runtime's user data folder that is a cache, not a save.
const CACHE_ENTRY: &str = "cache";

/// Why saves could not be moved. Serialized as `{"kind": "...", "detail": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum SavesError {
    /// The destination already holds saves; moving needs the player's permission to replace them.
    DestinationHasSaves(String),
    /// The destination is inside the folder being moved (or the other way round).
    NestedFolders,
    /// The backup asked for does not exist.
    BackupMissing(String),
    /// A file operation failed.
    Io(String),
}

impl From<io::Error> for SavesError {
    fn from(error: io::Error) -> Self {
        SavesError::Io(error.to_string())
    }
}

/// The user's home folder: `HOME`, or `USERPROFILE` on Windows, which has no `HOME`.
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Folder saves go to when the player has not chosen one.
pub fn default_folder() -> PathBuf {
    home().join(DEFAULT_SAVES)
}

/// Where the runtime kept its data before the launcher managed it.
pub fn legacy_folder() -> PathBuf {
    home().join(LEGACY_USER_DATA)
}

/// Entries of a user data folder that are saves (everything except the cache and hidden files).
fn save_entries(folder: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(folder) else {
        return Vec::new();
    };
    read.filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name != CACHE_ENTRY && !name.starts_with('.')
        })
        .map(|entry| entry.path())
        .collect()
}

/// Whether `folder` holds any saves.
pub fn has_saves(folder: &Path) -> bool {
    !save_entries(folder).is_empty()
}

/// Moves `from` to `to`, also across disks (where a rename is not possible) by copying and then
/// removing the original.
fn move_entry(from: &Path, to: &Path) -> io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_entry(from, to)?;
    if fs::symlink_metadata(from)?.is_dir() {
        fs::remove_dir_all(from)
    } else {
        fs::remove_file(from)
    }
}

fn copy_entry(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(from)?.is_dir() {
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

fn remove_entry(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    }
}

/// `path` with links resolved, also when it does not exist yet: its nearest existing ancestor is
/// resolved and the rest appended (on macOS `/tmp` is a link to `/private/tmp`, so comparing an
/// unresolved path with a resolved one would miss that one folder is inside the other).
fn canonical(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(resolved) = fs::canonicalize(&current) {
            return missing.iter().rev().fold(resolved, |acc, part| acc.join(part));
        }
        match (current.file_name().map(|n| n.to_os_string()), current.parent()) {
            (Some(name), Some(parent)) => {
                missing.push(name);
                current = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Moves the saves of `from` into `to`. When `to` already holds saves it stops with
/// `DestinationHasSaves` unless `replace` is set, in which case entries with the same name in `to`
/// are replaced. Moving a folder into itself does nothing.
pub fn move_saves(from: &Path, to: &Path, replace: bool) -> Result<usize, SavesError> {
    let (from_c, to_c) = (canonical(from), canonical(to));
    if from_c == to_c {
        return Ok(0);
    }
    if to_c.starts_with(&from_c) || from_c.starts_with(&to_c) {
        return Err(SavesError::NestedFolders);
    }
    let entries = save_entries(from);
    if entries.is_empty() {
        return Ok(0);
    }
    if has_saves(to) && !replace {
        return Err(SavesError::DestinationHasSaves(to.to_string_lossy().into_owned()));
    }
    fs::create_dir_all(to).map_err(|e| SavesError::Io(format!("{}: {e}", to.display())))?;
    for entry in &entries {
        let target = to.join(entry.file_name().unwrap_or_default());
        remove_entry(&target).map_err(|e| SavesError::Io(format!("{}: {e}", target.display())))?;
        move_entry(entry, &target).map_err(|e| SavesError::Io(format!("{}: {e}", entry.display())))?;
    }
    Ok(entries.len())
}

/// Backs up the saves a move from `from` to `to` would touch, before it touches them: those in
/// `from` always, and those already in `to` when `replace` lets the move overwrite them. A move
/// that would do nothing or that `move_saves` refuses (same or nested folders, nothing to move, a
/// destination with saves and no `replace`) makes no backup. A backup that fails stops the move.
pub fn backup_before_move(app: &AppHandle, from: &Path, to: &Path, replace: bool) -> Result<Vec<BackupInfo>, SavesError> {
    backups_before_move(&backups_dir(app)?, from, to, replace)
}

fn backups_before_move(backups: &Path, from: &Path, to: &Path, replace: bool) -> Result<Vec<BackupInfo>, SavesError> {
    let (from_c, to_c) = (canonical(from), canonical(to));
    let refused = from_c == to_c || to_c.starts_with(&from_c) || from_c.starts_with(&to_c);
    if refused || !has_saves(from) || (has_saves(to) && !replace) {
        return Ok(Vec::new());
    }
    let mut made = vec![create_backup(backups, &find_gears3_content_dir(from), BackupKind::BeforeMove)?];
    if has_saves(to) {
        made.push(create_backup(backups, &find_gears3_content_dir(to), BackupKind::BeforeReplace)?);
    }
    Ok(made)
}

/// Gets `saves` ready before the game starts: creates it and, the first time, brings over the
/// saves the runtime wrote to its old folder and moves that folder's shader cache to `cache`.
/// Nothing is replaced: when both folders hold saves, the old ones stay where they are.
pub fn prepare(saves: &Path, cache: &Path) -> Result<(), SavesError> {
    let legacy = legacy_folder();
    // The runtime's old folder belongs to the published launcher: the development one leaves it.
    if !cfg!(feature = "dev") && has_saves(&legacy) && !has_saves(saves) {
        move_saves(&legacy, saves, false)?;
    }
    let legacy_cache = legacy.join(CACHE_ENTRY);
    if legacy_cache.is_dir() && !cache.exists() {
        if let Some(parent) = cache.parent() {
            fs::create_dir_all(parent).map_err(|e| SavesError::Io(e.to_string()))?;
        }
        move_entry(&legacy_cache, cache).map_err(|e| SavesError::Io(e.to_string()))?;
    }
    fs::create_dir_all(saves).map_err(|e| SavesError::Io(format!("{}: {e}", saves.display())))?;
    fs::create_dir_all(cache).map_err(|e| SavesError::Io(format!("{}: {e}", cache.display())))
}

/// Backups kept: making one removes the oldest beyond this many.
pub const MAX_BACKUPS: usize = 10;

/// Why a backup was made: by the player, or by the launcher before it changed the saves. Its tag
/// ends the backup's folder name (`backup_<seconds>_<tag>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupKind {
    Manual,
    BeforeRepair,
    BeforeReset,
    /// The saves being moved to another folder, made before they leave.
    BeforeMove,
    /// The saves already in the folder a move replaces, made before they are overwritten.
    BeforeReplace,
}

impl BackupKind {
    fn tag(self) -> &'static str {
        match self {
            BackupKind::Manual => "manual",
            BackupKind::BeforeRepair => "auto_before_repair",
            BackupKind::BeforeReset => "auto_before_reset",
            BackupKind::BeforeMove => "auto_before_move",
            BackupKind::BeforeReplace => "auto_before_replace",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        [BackupKind::Manual, BackupKind::BeforeRepair, BackupKind::BeforeReset, BackupKind::BeforeMove, BackupKind::BeforeReplace]
            .into_iter()
            .find(|kind| kind.tag() == tag)
    }
}

/// A save backup, sent to the interface as JSON; the interface words and formats it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    /// Its folder name, which restoring it takes.
    pub id: String,
    /// When it was made, in milliseconds since 1970 (UTC), from its folder name.
    pub created_ms: Option<u64>,
    /// Why it was made; `None` for a folder name the launcher does not know.
    pub kind: Option<BackupKind>,
    pub size_bytes: u64,
}

/// The state of the game's saves, sent to the interface as JSON; the interface words it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveStatus {
    pub has_saves: bool,
    pub checkpoint_count: usize,
    pub is_corrupted: bool,
    /// "ok", "corrupted" or "empty".
    pub status: String,
    pub save_path: String,
    /// When a checkpoint was last written, in milliseconds since 1970 (UTC).
    pub last_saved_ms: Option<u64>,
    pub backups: Vec<BackupInfo>,
}

/// Resolves where Gears of War 3 content packages and headers are kept inside `saves_dir`.
pub fn find_gears3_content_dir(saves_dir: &Path) -> PathBuf {
    let p1 = saves_dir.join("B13EBABEBABEBABE").join("4D5308AB");
    if p1.exists() {
        return p1;
    }
    let p2 = saves_dir.join("4D5308AB");
    if p2.exists() {
        return p2;
    }
    p1
}

/// The launcher's backups folder, in its app data folder.
fn backups_dir(app: &AppHandle) -> Result<PathBuf, SavesError> {
    let dir = app.path().app_data_dir().map_err(|e| SavesError::Io(e.to_string()))?.join("backups");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn millis(time: std::time::SystemTime) -> Option<u64> {
    time.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

/// What the saves folder holds: whether there is anything at all, the checkpoint files with data,
/// whether a save is broken, and when a checkpoint was last written.
struct SavesScan {
    has_saves: bool,
    checkpoint_count: usize,
    is_corrupted: bool,
    latest_time: Option<std::time::SystemTime>,
}

/// Looks through the content packages and headers in `content_dir`.
///
/// The game makes a profile's checkpoint slots as empty (0-byte) files when the profile is first
/// used and fills them when it saves, so an empty file is a free slot, not damage. A save is
/// broken only when a header has no package with files behind it, which is what keeps the game
/// from loading it and what repairing removes (see orphaned_headers).
fn scan_saves(content_dir: &Path) -> SavesScan {
    let headers_dir = content_dir.join("Headers").join("00000001");
    let packages_dir = content_dir.join("00000001");
    let mut scan = SavesScan { has_saves: false, checkpoint_count: 0, is_corrupted: false, latest_time: None };

    if let Ok(entries) = fs::read_dir(&headers_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            scan.has_saves = true;
            if let Ok(mtime) = path.metadata().and_then(|meta| meta.modified()) {
                scan.latest_time = Some(scan.latest_time.map_or(mtime, |prev| prev.max(mtime)));
            }
        }
    }
    scan.is_corrupted = !orphaned_headers(&headers_dir, &packages_dir).is_empty();

    if let Ok(entries) = fs::read_dir(&packages_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            scan.has_saves = true;
            if let Ok(files) = fs::read_dir(&path) {
                scan.checkpoint_count += files
                    .flatten()
                    .filter(|file| file.metadata().map(|meta| meta.len() > 0).unwrap_or(false))
                    .count();
            }
        }
    }
    scan
}

/// Inspects the current state of saved games.
pub fn inspect_saves(app: &AppHandle, saves_dir: &Path) -> SaveStatus {
    let content_dir = find_gears3_content_dir(saves_dir);
    let SavesScan { has_saves, checkpoint_count, is_corrupted, latest_time } = scan_saves(&content_dir);

    let status = if is_corrupted {
        "corrupted"
    } else if has_saves && checkpoint_count > 0 {
        "ok"
    } else {
        "empty"
    };

    SaveStatus {
        has_saves,
        checkpoint_count,
        is_corrupted,
        status: status.to_string(),
        save_path: content_dir.to_string_lossy().to_string(),
        last_saved_ms: latest_time.and_then(millis),
        backups: backups_dir(app).map(|dir| list_backups(&dir)).unwrap_or_default(),
    }
}

/// Headers in `headers_dir` whose content package in `packages_dir` is missing or empty, each with
/// the package folder (which may not exist).
fn orphaned_headers(headers_dir: &Path, packages_dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut orphans = Vec::new();
    let Ok(entries) = fs::read_dir(headers_dir) else {
        return orphans;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let pkg_dir = packages_dir.join(file_stem);
        let is_orphan = if !pkg_dir.exists() {
            true
        } else if pkg_dir.is_dir() {
            fs::read_dir(&pkg_dir).map(|e| e.count()).unwrap_or(0) == 0
        } else {
            false
        };
        if is_orphan {
            orphans.push((path, pkg_dir));
        }
    }
    orphans
}

/// Removes orphaned headers and their empty package folders, and returns how many. Only runs when
/// the player asks for it, and backs the saves up first: nothing is deleted if the backup fails.
pub fn repair_saves_internal(app: &AppHandle, saves_dir: &Path) -> Result<usize, SavesError> {
    let content_dir = find_gears3_content_dir(saves_dir);
    let headers_dir = content_dir.join("Headers").join("00000001");
    let packages_dir = content_dir.join("00000001");
    let orphans = orphaned_headers(&headers_dir, &packages_dir);
    if !orphans.is_empty() {
        create_backup(&backups_dir(app)?, &content_dir, BackupKind::BeforeRepair)?;
    }
    let removed = orphans.len();
    for (header, pkg_dir) in orphans {
        let _ = fs::remove_file(&header);
        if pkg_dir.exists() {
            let _ = fs::remove_dir_all(&pkg_dir);
        }
    }
    Ok(removed)
}

/// Performs a complete reset of save data, backing up first.
pub fn reset_saves_internal(app: &AppHandle, saves_dir: &Path) -> Result<(), SavesError> {
    let content_dir = find_gears3_content_dir(saves_dir);
    // Nothing is deleted if the backup fails.
    create_backup(&backups_dir(app)?, &content_dir, BackupKind::BeforeReset)?;

    let headers_dir = content_dir.join("Headers");
    let packages_dir = content_dir.join("00000001");
    if headers_dir.exists() {
        let _ = fs::remove_dir_all(&headers_dir);
    }
    if packages_dir.exists() {
        let _ = fs::remove_dir_all(&packages_dir);
    }
    let _ = fs::create_dir_all(content_dir.join("Headers").join("00000001"));
    let _ = fs::create_dir_all(content_dir.join("00000001"));
    Ok(())
}

/// Backs the player's saves up, at their request.
pub fn create_backup_internal(app: &AppHandle, saves_dir: &Path) -> Result<BackupInfo, SavesError> {
    create_backup(&backups_dir(app)?, &find_gears3_content_dir(saves_dir), BackupKind::Manual)
}

/// Copies `content_dir` into a new backup in `backups`, then keeps only the MAX_BACKUPS newest.
fn create_backup(backups: &Path, content_dir: &Path, kind: BackupKind) -> Result<BackupInfo, SavesError> {
    let now = std::time::SystemTime::now();
    let seconds = now.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let id = format!("backup_{}_{}", seconds, kind.tag());
    let target = backups.join(&id);
    if content_dir.exists() {
        copy_entry(content_dir, &target)?;
    } else {
        fs::create_dir_all(&target)?;
    }
    prune_backups(backups, MAX_BACKUPS);
    Ok(backup_info(&target).unwrap_or(BackupInfo { id, created_ms: millis(now), kind: Some(kind), size_bytes: 0 }))
}

/// What a backup folder's name and contents say about it: `backup_<seconds>_<tag>`.
fn backup_info(path: &Path) -> Option<BackupInfo> {
    let id = path.file_name()?.to_str()?.to_owned();
    let (seconds, tag) = id.strip_prefix("backup_").and_then(|rest| rest.split_once('_')).unzip();
    Some(BackupInfo {
        created_ms: seconds.and_then(|s| s.parse::<u64>().ok()).map(|s| s * 1000),
        kind: tag.and_then(BackupKind::from_tag),
        size_bytes: dir_size(path),
        id,
    })
}

/// The backups in `backups`, newest first.
fn list_backups(backups: &Path) -> Vec<BackupInfo> {
    let mut result: Vec<BackupInfo> = fs::read_dir(backups)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|entry| backup_info(&entry.path()))
                .collect()
        })
        .unwrap_or_default();
    result.sort_by(|a, b| b.created_ms.cmp(&a.created_ms).then_with(|| b.id.cmp(&a.id)));
    result
}

/// Removes the oldest backups in `backups` beyond the `keep` newest.
fn prune_backups(backups: &Path, keep: usize) {
    for old in list_backups(backups).into_iter().skip(keep) {
        let _ = fs::remove_dir_all(backups.join(&old.id));
    }
}

/// Restores a backup over the current saves.
pub fn restore_backup_internal(app: &AppHandle, saves_dir: &Path, backup_id: &str) -> Result<(), SavesError> {
    let b_dir = backups_dir(app)?;
    // Only a backup folder of the backups folder: an id with a path in it names none.
    let src = b_dir.join(backup_id);
    if backup_id.contains(['/', '\\']) || backup_id.starts_with('.') || !src.is_dir() {
        return Err(SavesError::BackupMissing(backup_id.to_owned()));
    }

    // The backup is copied beside the current saves first, and only swapped in once the copy is
    // complete, so a failed copy leaves the current saves as they were.
    let content_dir = find_gears3_content_dir(saves_dir);
    let restoring = content_dir.with_extension("restoring");
    let previous = content_dir.with_extension("previous");
    let _ = fs::remove_dir_all(&restoring);
    let _ = fs::remove_dir_all(&previous);
    if let Err(error) = copy_entry(&src, &restoring) {
        let _ = fs::remove_dir_all(&restoring);
        return Err(error.into());
    }
    if content_dir.exists() {
        fs::rename(&content_dir, &previous)?;
    }
    if let Err(error) = fs::rename(&restoring, &content_dir) {
        let _ = fs::rename(&previous, &content_dir);
        return Err(error.into());
    }
    let _ = fs::remove_dir_all(&previous);
    Ok(())
}

/// Opens the saves directory in the system file manager.
pub fn open_saves_folder_internal(saves_dir: &Path) -> Result<(), String> {
    let dir = find_gears3_content_dir(saves_dir);
    let _ = fs::create_dir_all(&dir);

    #[cfg(target_os = "linux")]
    {
        crate::host_env::use_host_env(&mut std::process::Command::new("xdg-open"))
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    /// A user data folder as the runtime writes it: a checkpoint, a profile and a shader cache.
    fn fake_user_data(dir: &Path) {
        fs::create_dir_all(dir.join("B13EBABEBABEBABE/4D5308AB/00000001/GearsCheckpoint")).unwrap();
        fs::write(dir.join("B13EBABEBABEBABE/4D5308AB/00000001/PlayerStorage.dat"), b"save").unwrap();
        fs::create_dir_all(dir.join("4D5308AB/profile/Marcus")).unwrap();
        fs::create_dir_all(dir.join("cache/shaders")).unwrap();
    }

    #[test]
    fn empty_checkpoint_slots_are_not_broken_saves() {
        let dir = TempFolder::new("saves-scan");
        let content = dir.join("content");
        // A profile that never saved the campaign: its header, its package, and the three slots the
        // game makes empty.
        fs::create_dir_all(content.join("Headers/00000001")).unwrap();
        fs::write(content.join("Headers/00000001/GearsCheckpoint.header"), b"header").unwrap();
        fs::create_dir_all(content.join("00000001/GearsCheckpoint")).unwrap();
        for slot in 0..3 {
            fs::write(content.join(format!("00000001/GearsCheckpoint/GearsCheckpoint{slot}.sav")), b"").unwrap();
        }
        let scan = scan_saves(&content);
        assert!(scan.has_saves && !scan.is_corrupted);
        assert_eq!(scan.checkpoint_count, 0);

        // Once the game saves, the slot counts as a checkpoint.
        fs::write(content.join("00000001/GearsCheckpoint/GearsCheckpoint0.sav"), b"save").unwrap();
        let scan = scan_saves(&content);
        assert!(!scan.is_corrupted);
        assert_eq!(scan.checkpoint_count, 1);

        // A header whose package is gone is broken, as repairing sees it.
        fs::write(content.join("Headers/00000001/PlayerStorage.dat.header"), b"header").unwrap();
        assert!(scan_saves(&content).is_corrupted);
    }

    #[test]
    fn moves_saves_but_not_the_cache() {
        let from = TempFolder::new("saves-from");
        let to = TempFolder::new("saves-to");
        fake_user_data(&from);

        assert_eq!(move_saves(&from, &to.join("Saves"), false).unwrap(), 2);
        assert!(to.join("Saves/B13EBABEBABEBABE/4D5308AB/00000001/PlayerStorage.dat").is_file());
        assert!(to.join("Saves/4D5308AB/profile/Marcus").is_dir());
        assert!(!to.join("Saves/cache").exists());
        assert!(from.join("cache/shaders").is_dir());
        assert!(!has_saves(&from));
    }

    #[test]
    fn asks_before_replacing_saves() {
        let from = TempFolder::new("replace-from");
        let to = TempFolder::new("replace-to");
        fake_user_data(&from);
        fs::create_dir_all(to.join("4D5308AB")).unwrap();

        assert!(matches!(move_saves(&from, &to, false), Err(SavesError::DestinationHasSaves(_))));
        assert!(has_saves(&from));
        assert_eq!(move_saves(&from, &to, true).unwrap(), 2);
        assert!(to.join("4D5308AB/profile/Marcus").is_dir());
    }

    #[test]
    fn refuses_a_folder_inside_the_other() {
        let from = TempFolder::new("saves-nested");
        fake_user_data(&from);
        assert_eq!(move_saves(&from, &from.join("inner"), false), Err(SavesError::NestedFolders));
        assert_eq!(move_saves(&from, &from, false).unwrap(), 0);
    }

    #[test]
    fn a_backup_says_when_and_why_it_was_made() {
        let dir = TempFolder::new("backup-info");
        let backups = dir.join("backups");
        let content = dir.join("content");
        fs::create_dir_all(content.join("00000001")).unwrap();
        fs::write(content.join("00000001/checkpoint"), b"save").unwrap();
        let made = create_backup(&backups, &content, BackupKind::BeforeRepair).unwrap();
        assert_eq!(made.kind, Some(BackupKind::BeforeRepair));
        assert!(made.created_ms.is_some());
        assert_eq!(made.size_bytes, 4);
        // Folders made by older launchers read the same way; an unknown one still lists.
        fs::create_dir_all(backups.join("backup_1729000000_manual")).unwrap();
        fs::create_dir_all(backups.join("something_else")).unwrap();
        let listed = list_backups(&backups);
        let old = listed.iter().find(|b| b.id == "backup_1729000000_manual").unwrap();
        assert_eq!((old.created_ms, old.kind), (Some(1_729_000_000_000), Some(BackupKind::Manual)));
        let unknown = listed.iter().find(|b| b.id == "something_else").unwrap();
        assert_eq!((unknown.created_ms, unknown.kind), (None, None));
        // Newest first; a folder without a date goes last.
        assert_eq!(listed[0].id, made.id);
        assert_eq!(listed.last().unwrap().id, "something_else");
    }

    #[test]
    fn moving_backs_up_the_saves_that_leave() {
        let dir = TempFolder::new("backup-move");
        let (backups, from, to) = (dir.join("backups"), dir.join("from"), dir.join("to"));
        fake_user_data(&from);
        let made = backups_before_move(&backups, &from, &to, false).unwrap();
        assert_eq!(made.iter().map(|b| b.kind).collect::<Vec<_>>(), vec![Some(BackupKind::BeforeMove)]);
        assert!(backups.join(&made[0].id).join("00000001/PlayerStorage.dat").is_file());
        // The backup leaves the saves where they are: moving them is still move_saves's job.
        assert!(has_saves(&from));
    }

    #[test]
    fn replacing_also_backs_up_the_saves_it_overwrites() {
        let dir = TempFolder::new("backup-replace");
        let (backups, from, to) = (dir.join("backups"), dir.join("from"), dir.join("to"));
        fake_user_data(&from);
        fake_user_data(&to);
        fs::write(to.join("B13EBABEBABEBABE/4D5308AB/00000001/PlayerStorage.dat"), b"older save").unwrap();
        let made = backups_before_move(&backups, &from, &to, true).unwrap();
        let kinds: Vec<_> = made.iter().map(|b| b.kind).collect();
        assert_eq!(kinds, vec![Some(BackupKind::BeforeMove), Some(BackupKind::BeforeReplace)]);
        let replaced = backups.join(&made[1].id).join("00000001/PlayerStorage.dat");
        assert_eq!(fs::read(replaced).unwrap(), b"older save");
    }

    #[test]
    fn a_move_that_does_nothing_or_is_refused_makes_no_backup() {
        let dir = TempFolder::new("backup-none");
        let (backups, from, to, empty) = (dir.join("backups"), dir.join("from"), dir.join("to"), dir.join("empty"));
        fake_user_data(&from);
        fake_user_data(&to);
        // A destination with saves and no replace: move_saves will ask first.
        assert!(backups_before_move(&backups, &from, &to, false).unwrap().is_empty());
        // The same folder, nested folders and nothing to move.
        assert!(backups_before_move(&backups, &from, &from, true).unwrap().is_empty());
        assert!(backups_before_move(&backups, &from, &from.join("inner"), true).unwrap().is_empty());
        assert!(backups_before_move(&backups, &empty, &to, true).unwrap().is_empty());
        assert!(!backups.exists() || fs::read_dir(&backups).unwrap().next().is_none());
    }

    #[test]
    fn a_failed_backup_stops_before_anything_moves() {
        let dir = TempFolder::new("backup-fails");
        let (from, to) = (dir.join("from"), dir.join("to"));
        fake_user_data(&from);
        // A file where the backups folder should be: the backup cannot be made.
        let backups = dir.join("backups");
        fs::write(&backups, b"not a folder").unwrap();
        assert!(backups_before_move(&backups, &from, &to, false).is_err());
        assert!(has_saves(&from));
        assert!(!to.exists());
    }

    #[test]
    fn only_the_newest_backups_are_kept() {
        let dir = TempFolder::new("backup-prune");
        let backups = dir.join("backups");
        for seconds in 1..=12u64 {
            fs::create_dir_all(backups.join(format!("backup_{}_manual", 1_700_000_000 + seconds))).unwrap();
        }
        prune_backups(&backups, MAX_BACKUPS);
        let kept: Vec<String> = list_backups(&backups).into_iter().map(|b| b.id).collect();
        assert_eq!(kept.len(), 10);
        assert_eq!(kept.first().unwrap(), "backup_1700000012_manual");
        assert_eq!(kept.last().unwrap(), "backup_1700000003_manual");
    }
}
