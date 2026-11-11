//! The player's achievements, for the Achievements section.
//!
//! Two sources make up what the section shows:
//!
//! - The game's own data: names, descriptions, gamerscore, flags and artwork of every achievement,
//!   in every language, are in the XDBF resource of `default.xex`. The executable is encrypted and
//!   compressed, so the runtime reads it: `azura --export_achievements=<folder>` loads it without a
//!   window or a GPU, writes `achievements.json` and `icons/<imageId>.png` there, and exits
//!   (`runtime/src/achievement_export.h`). The launcher runs it once per game build and keeps the
//!   result in its app data folder (`achievements/<build>/`). It is read from the player's own
//!   copy of the game and never leaves this computer.
//! - The unlocks: the runtime records each achievement the game unlocks, with the time, in
//!   `<saves folder>/achievements/<title ID>.toml` (`[unlocked.<id>]` tables with a `filetime`, a
//!   Windows FILETIME: hundreds of nanoseconds since 1601).
//!
//! Downloadable content brings its own achievements in its package (`spa.bin`, the same database
//! format as the executable's resource). With RAAM's Shadow installed, the launcher exports that
//! file too (`--export_achievements_from=<spa.bin>`, kept beside the game's export) and adds the
//! add-on's achievements (`dlc::RAAMS_SHADOW_ACHIEVEMENTS`) to the game's.
//!
//! An achievement the original game hid (its `flags` lack `SHOW_UNACHIEVED_FLAG`) is sent without
//! its name, description, gamerscore or artwork until it is unlocked, so the interface cannot
//! reveal it by mistake.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};

/// File the runtime writes last, so its presence means the export is complete.
pub const EXPORT_FILE: &str = "achievements.json";

/// Folder of the artwork inside an export.
const ICONS_DIR: &str = "icons";

/// Bit of an achievement's flags that shows it before it is unlocked. In Gears of War 3 every
/// achievement has it except the secret one (its locked description is "Secret.").
pub const SHOW_UNACHIEVED_FLAG: u32 = 0x8;

/// Language of the texts when neither the game's nor the launcher's is in the game data.
const FALLBACK_LANGUAGE: &str = "en";

/// How long the runtime may take to export before it is stopped.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(30);

/// SDL hint (read from the environment) that keeps a macOS process out of the Dock.
pub const MAC_BACKGROUND_APP_HINT: &str = "SDL_MAC_BACKGROUND_APP";

/// Difference between the FILETIME epoch (1601) and the Unix epoch (1970), in FILETIME units.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// One achievement as the runtime exports it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedAchievement {
    pub id: u32,
    pub gamerscore: u32,
    pub flags: u32,
    /// Its artwork in `icons/<imageId>.png`; 0 when the game has none.
    pub image_id: u32,
    /// Texts by language code ("en", "es", "zh-Hant", ...).
    pub label: HashMap<String, String>,
    pub description: HashMap<String, String>,
    pub unachieved: HashMap<String, String>,
}

/// What `achievements.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Export {
    pub title_id: String,
    pub languages: Vec<String>,
    pub achievements: Vec<ExportedAchievement>,
}

/// One achievement as the interface shows it. A hidden achievement that is still locked has no
/// `name`, `description`, `gamerscore` or `icon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Achievement {
    pub id: u32,
    pub name: Option<String>,
    pub description: Option<String>,
    pub gamerscore: Option<u32>,
    /// Hidden in the original game (shown as "Hidden achievement" until unlocked).
    pub hidden: bool,
    pub unlocked: bool,
    /// When it was unlocked, in milliseconds since 1970.
    pub unlocked_at: Option<i64>,
    /// Its artwork as a `data:image/png;base64,...` URL; `None` for the generic medal.
    pub icon: Option<String>,
}

/// The section's content, sent to the interface as JSON (`{"status": "ok", ...}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum AchievementsState {
    /// The achievements, with the totals the progress card shows.
    #[serde(rename_all = "camelCase")]
    Ok {
        achievements: Vec<Achievement>,
        unlocked_count: usize,
        earned_gamerscore: u32,
        total_gamerscore: u32,
        /// When the last achievement was unlocked, once every one is (the Embry Star).
        completed_at: Option<i64>,
    },
    /// No valid game folder chosen yet.
    NoGame,
    /// The game folder is valid but the runtime that reads its data is not installed yet.
    NoRuntime,
    /// The runtime could not read the achievements.
    Unavailable { detail: String },
}

/// Whether the original game hides this achievement until it is unlocked.
pub fn is_hidden(flags: u32) -> bool {
    flags & SHOW_UNACHIEVED_FLAG == 0
}

/// Language code of the achievement texts for an engine language code ("ESN" → "es"). Czech and
/// Hungarian have no texts in the game data.
pub fn text_language(engine_code: &str) -> Option<&'static str> {
    Some(match engine_code {
        "INT" => "en",
        "JPN" => "ja",
        "DEU" => "de",
        "FRA" => "fr",
        "ESN" | "ESM" => "es",
        "ITA" => "it",
        "KOR" => "ko",
        "CHN" => "zh-Hant",
        "PTB" => "pt",
        "POL" => "pl",
        "RUS" => "ru",
        _ => return None,
    })
}

/// The text in the first of `languages` that has one, else in English, else in any language.
fn pick_text(texts: &HashMap<String, String>, languages: &[&str]) -> Option<String> {
    languages
        .iter()
        .chain(std::iter::once(&FALLBACK_LANGUAGE))
        .find_map(|language| texts.get(*language).filter(|text| !text.is_empty()))
        .or_else(|| texts.values().find(|text| !text.is_empty()))
        .cloned()
}

/// Milliseconds since 1970 for a FILETIME; `None` before 1970 (a corrupt value).
pub fn filetime_to_unix_ms(filetime: u64) -> Option<i64> {
    let since_1970 = filetime.checked_sub(FILETIME_UNIX_EPOCH)?;
    i64::try_from(since_1970 / 10_000).ok()
}

/// Reads the unlock file: achievement ID → FILETIME. A missing or unreadable file reads as no
/// unlocks, as the runtime does.
pub fn read_unlocks(path: &Path) -> HashMap<u32, u64> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(document) = text.parse::<toml::Table>() else {
        return HashMap::new();
    };
    let Some(unlocked) = document.get("unlocked").and_then(toml::Value::as_table) else {
        return HashMap::new();
    };
    unlocked
        .iter()
        .filter_map(|(id, entry)| {
            let id = id.parse::<u32>().ok()?;
            let filetime = entry.get("filetime").and_then(toml::Value::as_integer).unwrap_or(0);
            Some((id, u64::try_from(filetime).unwrap_or(0)))
        })
        .collect()
}

/// Path of the unlock file in a saves folder.
pub fn unlocks_path(saves: &Path, title_id: &str) -> PathBuf {
    saves.join("achievements").join(format!("{}.toml", title_id.to_ascii_uppercase()))
}

/// The artwork of an export as a data URL, or `None` when it is missing.
fn icon_data_url(export_dir: &Path, image_id: u32) -> Option<String> {
    if image_id == 0 {
        return None;
    }
    let bytes = fs::read(export_dir.join(ICONS_DIR).join(format!("{image_id}.png"))).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Builds what the section shows from exports (each with its folder, for the artwork: the game's,
/// then those of its downloadable content), the unlocks and the languages to show the texts in
/// (most preferred first).
pub fn build_state(sources: &[(&Export, &Path)], unlocks: &HashMap<u32, u64>, languages: &[&str]) -> AchievementsState {
    let exported_all: Vec<(&ExportedAchievement, &Path)> = sources
        .iter()
        .flat_map(|(export, dir)| export.achievements.iter().map(move |exported| (exported, *dir)))
        .collect();
    let achievements: Vec<Achievement> = exported_all
        .iter()
        .map(|&(exported, export_dir)| {
            let unlock = unlocks.get(&exported.id).copied();
            let hidden = is_hidden(exported.flags);
            let revealed = unlock.is_some() || !hidden;
            // A locked achievement shows what to do; an unlocked one, what was done.
            let description = if unlock.is_some() { &exported.description } else { &exported.unachieved };
            Achievement {
                id: exported.id,
                name: revealed.then(|| pick_text(&exported.label, languages)).flatten(),
                description: revealed.then(|| pick_text(description, languages)).flatten(),
                gamerscore: revealed.then_some(exported.gamerscore),
                hidden,
                unlocked: unlock.is_some(),
                unlocked_at: unlock.and_then(filetime_to_unix_ms),
                icon: revealed.then(|| icon_data_url(export_dir, exported.image_id)).flatten(),
            }
        })
        .collect();

    let unlocked_count = achievements.iter().filter(|achievement| achievement.unlocked).count();
    let earned_gamerscore = exported_all
        .iter()
        .filter(|(exported, _)| unlocks.contains_key(&exported.id))
        .map(|(exported, _)| exported.gamerscore)
        .sum();
    let total_gamerscore = exported_all.iter().map(|(exported, _)| exported.gamerscore).sum();
    let completed = !achievements.is_empty() && unlocked_count == achievements.len();
    let completed_at = completed
        .then(|| achievements.iter().filter_map(|achievement| achievement.unlocked_at).max())
        .flatten();

    AchievementsState::Ok { achievements, unlocked_count, earned_gamerscore, total_gamerscore, completed_at }
}

/// Reads a finished export; `None` when it is missing or unreadable (then it is made again).
pub fn read_export(export_dir: &Path) -> Option<Export> {
    let text = fs::read_to_string(export_dir.join(EXPORT_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Runs the runtime in `runtime_dir` to export the achievements of the game in `game_folder` (or,
/// with `from`, of that achievement database) to `export_dir`. It writes into a sibling folder
/// first and renames it, so an interrupted export never looks finished.
pub fn export(runtime_dir: &Path, game_folder: &Path, export_dir: &Path, from: Option<&Path>) -> Result<Export, String> {
    let executable = crate::runtime::executable_path(runtime_dir);
    if !executable.is_file() {
        return Err("the runtime is not installed".into());
    }
    let staging = export_dir.with_extension("tmp");
    let _ = fs::remove_dir_all(&staging);
    let mut child = crate::host_env::use_host_env(&mut Command::new(&executable))
        .arg(format!("--game_data_root={}", game_folder.display()))
        .arg(format!("--export_achievements={}", staging.display()))
        .args(from.map(|from| format!("--export_achievements_from={}", from.display())))
        .current_dir(runtime_dir)
        // The runtime starts SDL's video before it can tell it will only export, and on macOS that
        // makes it an app with a Dock icon for the second the export takes. SDL reads this hint
        // from the environment and keeps it a background process instead; elsewhere it is ignored.
        .env(MAC_BACKGROUND_APP_HINT, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", executable.display()))?;
    // The export takes about a second; a runtime still running long after has gone wrong (or is
    // one that started the game), so it is stopped rather than left to block the section.
    let deadline = Instant::now() + EXPORT_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("the runtime did not finish the export in {} s", EXPORT_TIMEOUT.as_secs()));
        }
        thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        let _ = fs::remove_dir_all(&staging);
        let mut stdout = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            let _ = pipe.read_to_string(&mut stdout);
        }
        let reason = stdout.lines().rev().find(|line| line.starts_with('{')).unwrap_or("").trim();
        return Err(format!("the runtime could not export the achievements (exit {:?}) {reason}", status.code()));
    }
    let export = read_export(&staging).ok_or("the runtime wrote no achievements")?;
    let _ = fs::remove_dir_all(export_dir);
    fs::rename(&staging, export_dir).map_err(|e| e.to_string())?;
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    fn texts(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(language, text)| (language.to_string(), text.to_string())).collect()
    }

    fn exported(id: u32, gamerscore: u32, flags: u32) -> ExportedAchievement {
        ExportedAchievement {
            id,
            gamerscore,
            flags,
            image_id: id,
            label: texts(&[("en", &format!("Name {id}")), ("es", &format!("Nombre {id}"))]),
            description: texts(&[("en", "Done"), ("es", "Hecho")]),
            unachieved: texts(&[("en", "To do"), ("es", "Por hacer")]),
        }
    }

    fn sample_export() -> Export {
        Export {
            title_id: "4D5308AB".into(),
            languages: vec!["en".into(), "es".into()],
            achievements: vec![exported(1, 5, 14), exported(19, 5, 1), exported(20, 50, 9)],
        }
    }

    /// A FILETIME for a Unix time in milliseconds.
    fn filetime(unix_ms: u64) -> u64 {
        FILETIME_UNIX_EPOCH + unix_ms * 10_000
    }

    fn ok(state: AchievementsState) -> (Vec<Achievement>, usize, u32, u32, Option<i64>) {
        match state {
            AchievementsState::Ok { achievements, unlocked_count, earned_gamerscore, total_gamerscore, completed_at } => {
                (achievements, unlocked_count, earned_gamerscore, total_gamerscore, completed_at)
            }
            other => panic!("not ok: {other:?}"),
        }
    }

    #[test]
    fn only_achievements_without_the_show_flag_are_hidden() {
        // The flags of Gears of War 3: 14, 9, 10, 12 and 15 are shown; 1 (the secret one) is not.
        for flags in [14, 9, 10, 12, 15] {
            assert!(!is_hidden(flags), "{flags}");
        }
        assert!(is_hidden(1));
    }

    #[test]
    fn filetimes_become_unix_milliseconds() {
        // 2026-10-23 22:11:21 UTC, as the runtime wrote it for a real unlock.
        assert_eq!(filetime_to_unix_ms(134_372_670_816_713_290), Some(1_792_793_481_671));
        assert_eq!(filetime_to_unix_ms(0), None);
    }

    #[test]
    fn reads_the_runtime_unlock_file() {
        let dir = TempFolder::new("unlocks");
        let path = unlocks_path(&dir, "4d5308ab");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "# Achievement unlock state - managed by ReXGlue runtime\n\n\
             [unlocked.3]\nfiletime = 134372670816713290\n\n[unlocked.1]\nfiletime = 134370909655994810\n",
        )
        .unwrap();
        let unlocks = read_unlocks(&path);
        assert_eq!(unlocks.len(), 2);
        assert_eq!(unlocks[&3], 134_372_670_816_713_290);
        assert!(path.ends_with("achievements/4D5308AB.toml"));
    }

    #[test]
    fn a_missing_or_broken_unlock_file_reads_as_no_unlocks() {
        let dir = TempFolder::new("unlocks-broken");
        assert!(read_unlocks(&dir.join("missing.toml")).is_empty());
        fs::write(dir.join("broken.toml"), "not = = toml").unwrap();
        assert!(read_unlocks(&dir.join("broken.toml")).is_empty());
    }

    #[test]
    fn texts_follow_the_preferred_languages_then_english() {
        let label = texts(&[("en", "Marcus, It's Your Father"), ("es", "Marcus, es tu padre")]);
        assert_eq!(pick_text(&label, &["es", "en"]).as_deref(), Some("Marcus, es tu padre"));
        assert_eq!(pick_text(&label, &["cs"]).as_deref(), Some("Marcus, It's Your Father"));
        assert_eq!(pick_text(&texts(&[("de", "Vater")]), &["fr"]).as_deref(), Some("Vater"));
    }

    #[test]
    fn engine_codes_map_to_text_languages() {
        assert_eq!(text_language("ESM"), Some("es"));
        assert_eq!(text_language("CHN"), Some("zh-Hant"));
        assert_eq!(text_language("CZE"), None);
    }

    #[test]
    fn a_locked_hidden_achievement_reveals_nothing() {
        let dir = TempFolder::new("hidden");
        let (achievements, ..) = ok(build_state(&[(&sample_export(), &dir)], &HashMap::new(), &["es"]));
        let secret = &achievements[1];
        assert!(secret.hidden && !secret.unlocked);
        assert_eq!((&secret.name, &secret.description, secret.gamerscore, &secret.icon), (&None, &None, None, &None));
        // A locked achievement that is not hidden shows what to do.
        assert_eq!(achievements[0].name.as_deref(), Some("Nombre 1"));
        assert_eq!(achievements[0].description.as_deref(), Some("Por hacer"));
    }

    #[test]
    fn an_unlocked_achievement_shows_everything_and_its_date() {
        let dir = TempFolder::new("unlocked");
        fs::create_dir_all(dir.join(ICONS_DIR)).unwrap();
        fs::write(dir.join(ICONS_DIR).join("19.png"), [0x89, b'P', b'N', b'G']).unwrap();
        let unlocks = HashMap::from([(19, filetime(1_000))]);
        let (achievements, unlocked_count, earned, total, completed_at) =
            ok(build_state(&[(&sample_export(), &dir)], &unlocks, &["es"]));
        let secret = &achievements[1];
        assert_eq!(secret.name.as_deref(), Some("Nombre 19"));
        assert_eq!(secret.description.as_deref(), Some("Hecho"));
        assert_eq!(secret.unlocked_at, Some(1_000));
        assert_eq!(secret.icon.as_deref(), Some("data:image/png;base64,iVBORw=="));
        // Without its file, the artwork falls back to the generic medal.
        assert_eq!(achievements[0].icon, None);
        assert_eq!((unlocked_count, earned, total, completed_at), (1, 5, 60, None));
    }

    #[test]
    fn unlocking_every_achievement_completes_at_the_last_unlock() {
        let dir = TempFolder::new("complete");
        let unlocks = HashMap::from([(1, filetime(3_000)), (19, filetime(9_000)), (20, filetime(5_000))]);
        let (_, unlocked_count, earned, total, completed_at) =
            ok(build_state(&[(&sample_export(), &dir)], &unlocks, &["en"]));
        assert_eq!((unlocked_count, earned, total, completed_at), (3, 60, 60, Some(9_000)));
    }

    #[test]
    fn downloadable_content_adds_its_achievements_with_its_own_artwork() {
        let game_dir = TempFolder::new("achievements-game");
        let dlc_dir = TempFolder::new("achievements-dlc");
        fs::create_dir_all(dlc_dir.join(ICONS_DIR)).unwrap();
        fs::write(dlc_dir.join(ICONS_DIR).join("56.png"), b"png").unwrap();
        let dlc = Export { title_id: String::new(), languages: vec!["en".into()], achievements: vec![exported(56, 25, 9)] };
        let unlocks = HashMap::from([(56, filetime(1_000))]);
        let (achievements, unlocked, earned, total, _) =
            ok(build_state(&[(&sample_export(), &game_dir), (&dlc, &dlc_dir)], &unlocks, &["en"]));
        assert_eq!(achievements.len(), 4);
        assert_eq!((unlocked, earned, total), (1, 25, 85));
        let added = achievements.iter().find(|achievement| achievement.id == 56).unwrap();
        assert_eq!(added.name.as_deref(), Some("Name 56"));
        assert!(added.icon.as_deref().is_some_and(|icon| icon.starts_with("data:image/png;base64,")));
    }

    #[test]
    fn reads_the_runtime_export() {
        let dir = TempFolder::new("export");
        fs::write(
            dir.join(EXPORT_FILE),
            r#"{"titleId":"4D5308AB","languages":["en","es"],"achievements":[{"id":19,"gamerscore":5,
               "flags":1,"imageId":19,"label":{"en":"Respect the Dead","es":"Respeto a los muertos"},
               "description":{"en":"x","es":"y"},"unachieved":{"en":"Secret.","es":"Secreto."}}]}"#,
        )
        .unwrap();
        let export = read_export(&dir).unwrap();
        assert_eq!(export.achievements[0].unachieved["es"], "Secreto.");
        assert!(read_export(&dir.join("missing")).is_none());
    }
}
